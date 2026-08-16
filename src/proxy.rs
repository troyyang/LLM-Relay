use std::{
    collections::HashSet,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, Request, Response, Uri},
    routing::{any, get},
    Json, Router,
};
use bytes::Bytes;
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tower_http::limit::RequestBodyLimitLayer;
use tracing::{error, info, warn};

use crate::{
    config::{Config, ProviderConfig},
    error::RelayError,
    security::constant_time_eq,
};

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct AppState {
    config: Arc<Config>,
    client: reqwest::Client,
    limiter: Arc<Semaphore>,
    api_key: Arc<str>,
}

impl AppState {
    pub fn new(config: Config, client: reqwest::Client, api_key: String) -> Self {
        let limiter = Arc::new(Semaphore::new(config.server.max_concurrent_requests));
        Self {
            config: Arc::new(config),
            client,
            limiter,
            api_key: Arc::from(api_key),
        }
    }
}

pub fn router(state: AppState) -> Router {
    let max_body_size = state.config.server.max_body_size.0;

    Router::new()
        .route("/healthz", get(healthz))
        .route("/proxy/{api_key}/{provider}/{*path}", any(proxy_handler))
        .layer(RequestBodyLimitLayer::new(max_body_size))
        .with_state(state)
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

#[derive(Debug, Deserialize)]
struct ProxyPath {
    api_key: String,
    provider: String,
    path: String,
}

async fn proxy_handler(
    State(state): State<AppState>,
    Path(params): Path<ProxyPath>,
    request: Request<Body>,
) -> Result<Response<Body>, RelayError> {
    authorize_request(&params.api_key, &state.api_key)?;

    let permit =
        state
            .limiter
            .clone()
            .try_acquire_owned()
            .map_err(|_| RelayError::TooManyRequests {
                retry_after_seconds: state.config.server.retry_after_seconds,
            })?;

    let request_id = request_id(request.headers());
    let method = request.method().clone();
    let uri = redacted_proxy_uri(&params, request.uri());
    let started = Instant::now();

    let result = forward_request(state, params, request, request_id.clone(), permit).await;
    match &result {
        Ok(response) => {
            info!(
                request_id = %request_id,
                method = %method,
                path = %uri,
                status = response.status().as_u16(),
                latency_ms = started.elapsed().as_millis(),
                "request forwarded"
            );
        }
        Err(error) => {
            let status = error.status_code();
            if status.is_server_error() {
                error!(
                    request_id = %request_id,
                    method = %method,
                    path = %uri,
                    status = status.as_u16(),
                    latency_ms = started.elapsed().as_millis(),
                    error = %error,
                    "request failed"
                );
            } else {
                warn!(
                    request_id = %request_id,
                    method = %method,
                    path = %uri,
                    status = status.as_u16(),
                    latency_ms = started.elapsed().as_millis(),
                    error = %error,
                    "request rejected"
                );
            }
        }
    }

    result
}

async fn forward_request(
    state: AppState,
    params: ProxyPath,
    request: Request<Body>,
    request_id: String,
    permit: OwnedSemaphorePermit,
) -> Result<Response<Body>, RelayError> {
    if params.path.trim().is_empty() {
        return Err(RelayError::InvalidPath);
    }

    let provider = state
        .config
        .providers
        .get(&params.provider)
        .ok_or_else(|| RelayError::UnknownProvider(params.provider.clone()))?;
    let target_url = build_target_url(provider, &params.path, request.uri())?;

    let (parts, body) = request.into_parts();
    let headers = forward_headers(&parts.headers, Some(&request_id));
    let body = reqwest::Body::wrap_stream(body.into_data_stream());

    let upstream = state
        .client
        .request(parts.method, target_url)
        .headers(headers)
        .body(body)
        .send()
        .await?;

    let status = upstream.status();
    let response_headers = forward_headers(upstream.headers(), None);
    let stream = PermitStream::new(upstream.bytes_stream(), permit);
    let body = Body::from_stream(stream);

    let mut builder = Response::builder().status(status);
    if let Some(headers) = builder.headers_mut() {
        for (name, value) in response_headers.iter() {
            headers.append(name, value.clone());
        }
        if let Ok(value) = HeaderValue::from_str(&request_id) {
            headers.insert(HeaderName::from_static("x-request-id"), value);
        }
    }

    Ok(builder.body(body)?)
}

fn build_target_url(
    provider: &ProviderConfig,
    path: &str,
    original_uri: &Uri,
) -> Result<String, RelayError> {
    let base = provider.base_url.trim_end_matches('/');
    let path = path.trim_start_matches('/');

    if path.is_empty() {
        return Err(RelayError::InvalidPath);
    }

    let mut target = format!("{base}/{path}");
    if let Some(query) = original_uri.query() {
        target.push('?');
        target.push_str(query);
    }

    reqwest::Url::parse(&target)
        .map(|_| target)
        .map_err(|error| RelayError::InvalidTargetUrl(error.to_string()))
}

fn request_id(headers: &HeaderMap) -> String {
    let name = HeaderName::from_static("x-request-id");
    if let Some(value) = headers.get(&name).and_then(|value| value.to_str().ok()) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let counter = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("req_{now:x}_{counter:x}")
}

fn forward_headers(headers: &HeaderMap, request_id: Option<&str>) -> HeaderMap {
    let connection_tokens = connection_header_tokens(headers);
    let mut forwarded = HeaderMap::new();

    for (name, value) in headers.iter() {
        if should_drop_header(name, &connection_tokens) {
            continue;
        }
        forwarded.append(name.clone(), value.clone());
    }

    if let Some(request_id) = request_id {
        let name = HeaderName::from_static("x-request-id");
        if !forwarded.contains_key(&name) {
            if let Ok(value) = HeaderValue::from_str(request_id) {
                forwarded.insert(name, value);
            }
        }
    }

    forwarded
}

fn connection_header_tokens(headers: &HeaderMap) -> HashSet<HeaderName> {
    let mut tokens = HashSet::new();

    for value in headers.get_all(header::CONNECTION) {
        let Ok(value) = value.to_str() else {
            continue;
        };

        for token in value.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            if let Ok(name) = HeaderName::from_bytes(token.as_bytes()) {
                tokens.insert(name);
            }
        }
    }

    tokens
}

fn should_drop_header(name: &HeaderName, connection_tokens: &HashSet<HeaderName>) -> bool {
    matches!(
        *name,
        header::CONNECTION
            | header::HOST
            | header::PROXY_AUTHENTICATE
            | header::PROXY_AUTHORIZATION
            | header::TE
            | header::TRAILER
            | header::TRANSFER_ENCODING
            | header::UPGRADE
            | header::CONTENT_LENGTH
    ) || name.as_str().eq_ignore_ascii_case("keep-alive")
        || name.as_str().eq_ignore_ascii_case("x-relay-api-key")
        || name.as_str().eq_ignore_ascii_case("x-api-key")
        || connection_tokens.contains(name)
}

fn authorize_request(provided: &str, expected_api_key: &str) -> Result<(), RelayError> {
    if constant_time_eq(provided.as_bytes(), expected_api_key.as_bytes()) {
        Ok(())
    } else {
        Err(RelayError::Unauthorized)
    }
}

fn redacted_proxy_uri(params: &ProxyPath, uri: &Uri) -> String {
    let mut redacted = format!("/proxy/[redacted]/{}/{}", params.provider, params.path);
    if let Some(query) = uri.query() {
        redacted.push('?');
        redacted.push_str(query);
    }
    redacted
}

struct PermitStream {
    inner: Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>,
    _permit: OwnedSemaphorePermit,
}

impl PermitStream {
    fn new(
        stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
        permit: OwnedSemaphorePermit,
    ) -> Self {
        Self {
            inner: Box::pin(stream),
            _permit: permit,
        }
    }
}

impl Stream for PermitStream {
    type Item = Result<Bytes, reqwest::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::StatusCode;
    use std::collections::BTreeMap;
    use tokio::net::TcpListener;
    use tower::ServiceExt;

    #[tokio::test]
    async fn forwards_request_and_streams_response_body() {
        let upstream_addr = spawn_upstream().await;
        let app = test_app(format!("http://{upstream_addr}"), 4);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/proxy/test-relay-key/openai/v1/chat/completions?trace=1")
                    .header(header::AUTHORIZATION, "Bearer test")
                    .header("x-api-key", "do-not-forward")
                    .header("x-relay-api-key", "do-not-forward")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header("x-request-id", "req-test")
                    .body(Body::from(r#"{"stream":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(response.headers().get("x-seen-method").unwrap(), "POST");
        assert_eq!(
            response.headers().get("x-seen-uri").unwrap(),
            "/v1/chat/completions?trace=1"
        );
        assert_eq!(
            response.headers().get("x-seen-auth").unwrap(),
            "Bearer test"
        );
        assert_eq!(
            response.headers().get("x-seen-request-id").unwrap(),
            "req-test"
        );
        assert_eq!(response.headers().get("x-seen-relay-key").unwrap(), "");
        assert_eq!(response.headers().get("x-seen-api-key").unwrap(), "");
        assert_eq!(response.headers().get("x-request-id").unwrap(), "req-test");

        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], br#"{"stream":true}"#);
    }

    #[tokio::test]
    async fn rejects_unknown_provider() {
        let app = test_app("http://127.0.0.1:1".into(), 4);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/proxy/test-relay-key/missing/v1/chat/completions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn rejects_when_concurrency_limit_is_full() {
        let app = test_app("http://127.0.0.1:1".into(), 0);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/proxy/test-relay-key/openai/v1/chat/completions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers().get(header::RETRY_AFTER).unwrap(), "1");
    }

    #[tokio::test]
    async fn rejects_missing_or_invalid_api_key_before_forwarding() {
        let app = test_app("http://127.0.0.1:1".into(), 4);

        let missing = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/proxy/openai/v1/chat/completions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

        let invalid = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/proxy/wrong/openai/v1/chat/completions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);
    }

    async fn spawn_upstream() -> std::net::SocketAddr {
        let app = Router::new().fallback(upstream_echo);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        addr
    }

    async fn upstream_echo(request: Request<Body>) -> Response<Body> {
        let method = request.method().to_string();
        let uri = request.uri().to_string();
        let auth = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let request_id = request
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let relay_key = request
            .headers()
            .get("x-relay-api-key")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let api_key = request
            .headers()
            .get("x-api-key")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let body = to_bytes(request.into_body(), 1024).await.unwrap();

        Response::builder()
            .status(StatusCode::CREATED)
            .header("content-type", "application/json")
            .header("x-seen-method", method)
            .header("x-seen-uri", uri)
            .header("x-seen-auth", auth)
            .header("x-seen-request-id", request_id)
            .header("x-seen-relay-key", relay_key)
            .header("x-seen-api-key", api_key)
            .body(Body::from(body))
            .unwrap()
    }

    fn test_app(base_url: String, max_concurrent_requests: usize) -> Router {
        let mut providers = BTreeMap::new();
        providers.insert(
            "openai".into(),
            ProviderConfig {
                base_url,
                allow_private: true,
            },
        );

        let mut config = Config::default();
        config.server.max_concurrent_requests = max_concurrent_requests;
        config.providers = providers;

        router(AppState::new(
            config,
            reqwest::Client::new(),
            "test-relay-key".into(),
        ))
    }
}
