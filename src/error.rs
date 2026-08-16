use axum::{
    body::Body,
    http::{header, HeaderValue, Response, StatusCode},
    response::IntoResponse,
};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config file {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("invalid configuration: {0}")]
    Validation(String),
    #[error("failed to build HTTP client: {0}")]
    HttpClient(#[from] reqwest::Error),
}

#[derive(Debug, Error)]
pub enum RelayError {
    #[error("missing or invalid relay API key in the proxy URL")]
    Unauthorized,
    #[error("provider is not allowed: {0}")]
    UnknownProvider(String),
    #[error("proxy path is missing")]
    InvalidPath,
    #[error("failed to build upstream URL: {0}")]
    InvalidTargetUrl(String),
    #[error("upstream request failed: {0}")]
    Upstream(#[from] reqwest::Error),
    #[error("failed to build response: {0}")]
    ResponseBuild(#[from] axum::http::Error),
    #[error("too many in-flight requests")]
    TooManyRequests { retry_after_seconds: u64 },
}

impl RelayError {
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::UnknownProvider(_) => StatusCode::FORBIDDEN,
            Self::InvalidPath | Self::InvalidTargetUrl(_) => StatusCode::BAD_REQUEST,
            Self::TooManyRequests { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::Upstream(error) if error.is_timeout() => StatusCode::GATEWAY_TIMEOUT,
            Self::Upstream(_) => StatusCode::BAD_GATEWAY,
            Self::ResponseBuild(_) => StatusCode::BAD_GATEWAY,
        }
    }

    fn error_code(&self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::UnknownProvider(_) => "unknown_provider",
            Self::InvalidPath => "invalid_path",
            Self::InvalidTargetUrl(_) => "invalid_target_url",
            Self::Upstream(error) if error.is_timeout() => "upstream_timeout",
            Self::Upstream(_) => "upstream_request_failed",
            Self::ResponseBuild(_) => "response_build_failed",
            Self::TooManyRequests { .. } => "too_many_requests",
        }
    }
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: &'a str,
    message: String,
}

impl IntoResponse for RelayError {
    fn into_response(self) -> Response<Body> {
        let status = self.status_code();
        let mut response = axum::Json(ErrorBody {
            error: self.error_code(),
            message: self.to_string(),
        })
        .into_response();

        *response.status_mut() = status;
        if let Self::TooManyRequests {
            retry_after_seconds,
        } = self
        {
            if let Ok(value) = HeaderValue::from_str(&retry_after_seconds.to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
        }

        response
    }
}
