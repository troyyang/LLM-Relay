pub mod config;
pub mod error;
pub mod proxy;
pub mod security;

use std::time::Duration;

use axum::Router;
use config::Config;
use error::ConfigError;
use proxy::AppState;

pub fn build_http_client(config: &Config) -> Result<reqwest::Client, ConfigError> {
    let mut builder = reqwest::Client::builder()
        .pool_max_idle_per_host(config.pool.max_idle_per_host)
        .pool_idle_timeout(Duration::from_secs(config.pool.idle_timeout))
        .connect_timeout(Duration::from_secs(config.timeout.connect))
        .timeout(Duration::from_secs(config.timeout.request))
        .redirect(reqwest::redirect::Policy::none());

    if let Some(proxy) = &config.proxy {
        builder = builder.proxy(reqwest::Proxy::all(&proxy.url)?);
    }

    Ok(builder.build()?)
}

pub fn build_app(config: Config, client: reqwest::Client, api_key: String) -> Router {
    proxy::router(AppState::new(config, client, api_key))
}
