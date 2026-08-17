use std::{
    collections::BTreeMap,
    fs,
    net::IpAddr,
    path::{Path, PathBuf},
};

use reqwest::Url;
use serde::{Deserialize, Deserializer};

use crate::error::ConfigError;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub server: ServerConfig,
    pub runtime: RuntimeConfig,
    pub timeout: TimeoutConfig,
    pub pool: PoolConfig,
    pub security: SecurityConfig,
    pub proxy: Option<ProxyConfig>,
    pub providers: BTreeMap<String, ProviderConfig>,
}

impl Config {
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let path_display = path.display().to_string();
        let raw = fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path_display.clone(),
            source,
        })?;
        let mut config: Self = serde_yaml::from_str(&raw).map_err(|source| ConfigError::Parse {
            path: path_display,
            source,
        })?;
        config.security.api_key_file =
            resolve_config_relative_path(path, config.security.api_key_file);
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.server.port == 0 {
            return Err(ConfigError::Validation(
                "server.port must be greater than 0".into(),
            ));
        }
        if self.server.max_body_size.0 == 0 {
            return Err(ConfigError::Validation(
                "server.max_body_size must be greater than 0".into(),
            ));
        }
        if self.server.max_concurrent_requests == 0 {
            return Err(ConfigError::Validation(
                "server.max_concurrent_requests must be greater than 0".into(),
            ));
        }
        if self.runtime.worker_threads == 0 {
            return Err(ConfigError::Validation(
                "runtime.worker_threads must be greater than 0".into(),
            ));
        }
        if self.timeout.connect == 0 || self.timeout.request == 0 {
            return Err(ConfigError::Validation(
                "timeout.connect and timeout.request must be greater than 0".into(),
            ));
        }
        if self.pool.max_idle_per_host == 0 || self.pool.idle_timeout == 0 {
            return Err(ConfigError::Validation(
                "pool.max_idle_per_host and pool.idle_timeout must be greater than 0".into(),
            ));
        }
        if self.security.api_key_file.as_os_str().is_empty() {
            return Err(ConfigError::Validation(
                "security.api_key_file must not be empty".into(),
            ));
        }
        if self.providers.is_empty() {
            return Err(ConfigError::Validation(
                "at least one provider must be configured".into(),
            ));
        }

        for (name, provider) in &self.providers {
            validate_provider_name(name)?;
            validate_provider_url(name, provider)?;
        }

        if let Some(proxy) = &self.proxy {
            let parsed = Url::parse(&proxy.url).map_err(|error| {
                ConfigError::Validation(format!("proxy.url is invalid: {error}"))
            })?;
            match parsed.scheme() {
                "http" | "https" | "socks5" | "socks5h" => {}
                scheme => {
                    return Err(ConfigError::Validation(format!(
                        "proxy.url scheme {scheme:?} is not supported"
                    )));
                }
            }
        }

        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            runtime: RuntimeConfig::default(),
            timeout: TimeoutConfig::default(),
            pool: PoolConfig::default(),
            security: SecurityConfig::default(),
            proxy: None,
            providers: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub max_body_size: ByteSize,
    pub max_concurrent_requests: usize,
    pub retry_after_seconds: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".into(),
            port: 5017,
            max_body_size: ByteSize(8 * 1024 * 1024),
            max_concurrent_requests: 800,
            retry_after_seconds: 1,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeConfig {
    pub worker_threads: usize,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self { worker_threads: 2 }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TimeoutConfig {
    pub connect: u64,
    pub request: u64,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            connect: 10,
            request: 300,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PoolConfig {
    pub max_idle_per_host: usize,
    pub idle_timeout: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SecurityConfig {
    pub api_key_file: PathBuf,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            api_key_file: PathBuf::from("api_key"),
        }
    }
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_idle_per_host: 8,
            idle_timeout: 30,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyConfig {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderConfig {
    pub base_url: String,
    pub allow_private: bool,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            allow_private: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteSize(pub usize);

impl<'de> Deserialize<'de> for ByteSize {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Number(u64),
            Text(String),
        }

        let value = match Repr::deserialize(deserializer)? {
            Repr::Number(number) => usize::try_from(number)
                .map_err(|_| serde::de::Error::custom("byte size is too large"))?,
            Repr::Text(text) => parse_byte_size(&text).map_err(serde::de::Error::custom)?,
        };

        Ok(Self(value))
    }
}

impl Default for ByteSize {
    fn default() -> Self {
        Self(8 * 1024 * 1024)
    }
}

pub fn parse_byte_size(input: &str) -> Result<usize, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("byte size cannot be empty".into());
    }

    let split_at = input
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(input.len());
    let (number, unit) = input.split_at(split_at);

    if number.is_empty() {
        return Err(format!("byte size {input:?} is missing a number"));
    }

    let number: u128 = number
        .parse()
        .map_err(|_| format!("byte size {input:?} has an invalid number"))?;
    let unit = unit.trim().to_ascii_lowercase();
    let multiplier: u128 = match unit.as_str() {
        "" | "b" => 1,
        "kb" | "kib" | "k" => 1024,
        "mb" | "mib" | "m" => 1024 * 1024,
        "gb" | "gib" | "g" => 1024 * 1024 * 1024,
        _ => return Err(format!("byte size unit {unit:?} is not supported")),
    };

    let bytes = number
        .checked_mul(multiplier)
        .ok_or_else(|| format!("byte size {input:?} is too large"))?;

    usize::try_from(bytes).map_err(|_| format!("byte size {input:?} is too large"))
}

fn validate_provider_name(name: &str) -> Result<(), ConfigError> {
    if name.is_empty() {
        return Err(ConfigError::Validation(
            "provider names cannot be empty".into(),
        ));
    }

    let valid = name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'));
    if !valid {
        return Err(ConfigError::Validation(format!(
            "provider name {name:?} may only contain ASCII letters, numbers, '-' and '_'"
        )));
    }

    Ok(())
}

fn resolve_config_relative_path(config_path: &Path, configured_path: PathBuf) -> PathBuf {
    if configured_path.is_absolute() {
        return configured_path;
    }

    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(configured_path)
}

fn validate_provider_url(name: &str, provider: &ProviderConfig) -> Result<(), ConfigError> {
    let parsed = Url::parse(&provider.base_url).map_err(|error| {
        ConfigError::Validation(format!("providers.{name}.base_url is invalid: {error}"))
    })?;

    match parsed.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(ConfigError::Validation(format!(
                "providers.{name}.base_url scheme {scheme:?} is not supported"
            )));
        }
    }

    if parsed.host_str().is_none() {
        return Err(ConfigError::Validation(format!(
            "providers.{name}.base_url must include a host"
        )));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(ConfigError::Validation(format!(
            "providers.{name}.base_url must not include credentials"
        )));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(ConfigError::Validation(format!(
            "providers.{name}.base_url must not include query or fragment components"
        )));
    }

    if name == "cloudflare"
        && parsed.host_str() == Some("gateway.ai.cloudflare.com")
        && parsed.path().trim_matches('/').is_empty() == false
    {
        return Err(ConfigError::Validation(
            "providers.cloudflare.base_url must be https://gateway.ai.cloudflare.com; put /v1/<account-id>/<gateway-name>/compat/... in the relay request path".into(),
        ));
    }

    if !provider.allow_private && is_private_target(&parsed) {
        return Err(ConfigError::Validation(format!(
            "providers.{name}.base_url points to a private or local target; set allow_private: true only for intentional private upstreams"
        )));
    }

    Ok(())
}

fn is_private_target(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return true;
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();

    if matches!(host.as_str(), "localhost" | "localhost.localdomain") {
        return true;
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(ip) => {
                ip.is_private()
                    || ip.is_loopback()
                    || ip.is_link_local()
                    || ip.is_broadcast()
                    || ip.is_documentation()
                    || ip.is_unspecified()
            }
            IpAddr::V6(ip) => {
                ip.is_loopback()
                    || ip.is_unspecified()
                    || ip.is_unique_local()
                    || ip.is_unicast_link_local()
            }
        };
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_byte_sizes() {
        assert_eq!(parse_byte_size("8MB").unwrap(), 8 * 1024 * 1024);
        assert_eq!(parse_byte_size("1024").unwrap(), 1024);
        assert_eq!(parse_byte_size("2 gib").unwrap(), 2 * 1024 * 1024 * 1024);
    }

    #[test]
    fn loads_example_style_config_with_defaults() {
        let config: Config = serde_yaml::from_str(
            r#"
providers:
  openai:
    base_url: "https://api.openai.com"
"#,
        )
        .unwrap();

        config.validate().unwrap();
        assert_eq!(config.server.port, 5017);
        assert_eq!(config.server.max_body_size.0, 8 * 1024 * 1024);
        assert_eq!(config.runtime.worker_threads, 2);
        assert_eq!(config.security.api_key_file, PathBuf::from("api_key"));
    }

    #[test]
    fn checked_in_config_is_valid() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config/config.yaml");
        let config = Config::load_from_path(path).unwrap();

        assert_eq!(
            config.providers["cloudflare"].base_url,
            "https://gateway.ai.cloudflare.com"
        );
    }

    #[test]
    fn rejects_cloudflare_gateway_base_url_with_embedded_path() {
        let config: Config = serde_yaml::from_str(
            r#"
providers:
  cloudflare:
    base_url: "https://gateway.ai.cloudflare.com/v1/account/gateway/compat"
"#,
        )
        .unwrap();

        let error = config.validate().unwrap_err().to_string();
        assert!(error
            .contains("providers.cloudflare.base_url must be https://gateway.ai.cloudflare.com"));
    }

    #[test]
    fn rejects_private_provider_targets_by_default() {
        let config: Config = serde_yaml::from_str(
            r#"
providers:
  local:
    base_url: "http://127.0.0.1:11434"
"#,
        )
        .unwrap();

        assert!(config.validate().is_err());
    }

    #[test]
    fn allows_private_provider_targets_when_explicit() {
        let config: Config = serde_yaml::from_str(
            r#"
providers:
  local:
    base_url: "http://127.0.0.1:11434"
    allow_private: true
"#,
        )
        .unwrap();

        config.validate().unwrap();
    }
}
