use directories::ProjectDirs;
use serde::{Deserialize, Deserializer};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to load config: {0}")]
    LoadError(String),
    #[error("Missing required field: {0}")]
    MissingField(String),
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct Config {
    #[serde(default)]
    pub exchange: ExchangeConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub sync: SyncConfig,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ExchangeConfig {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub autodiscover: bool,
    #[serde(default)]
    pub ews_url: Option<String>,
    #[serde(default = "default_auth_mode")]
    pub auth_mode: String,
    #[serde(default = "default_retry_max_attempts")]
    pub retry_max_attempts: u32,
    #[serde(default = "default_retry_base_ms")]
    pub retry_base_ms: u64,
    #[serde(default = "default_retry_max_backoff_ms")]
    pub retry_max_backoff_ms: u64,
}

fn default_auth_mode() -> String {
    "basic".to_string()
}

fn default_retry_max_attempts() -> u32 {
    5
}

fn default_retry_base_ms() -> u64 {
    500
}

fn default_retry_max_backoff_ms() -> u64 {
    10_000
}

impl ExchangeConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.username.is_empty() && self.email.is_empty() {
            return Err(ConfigError::MissingField("username or email".to_string()));
        }
        if self.password.is_empty() {
            return Err(ConfigError::MissingField("password".to_string()));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct CacheConfig {
    #[serde(default = "default_cache_path")]
    pub path: PathBuf,
    #[serde(default = "default_true")]
    pub initial_sync: bool,
    #[serde(default = "default_max_cached_emails")]
    pub max_cached_emails: i64,
}

fn default_cache_path() -> PathBuf {
    if let Some(proj_dirs) = ProjectDirs::from("com", "ews-skill", "ews-skill") {
        let data_dir = proj_dirs.data_dir();
        std::fs::create_dir_all(data_dir).ok();
        return data_dir.join("ews_cache.db");
    }
    PathBuf::from("ews_cache.db")
}

fn default_true() -> bool {
    true
}
fn default_max_cached_emails() -> i64 {
    10000
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            path: default_cache_path(),
            initial_sync: true,
            max_cached_emails: 10000,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct SyncConfig {
    #[serde(default = "default_folders", deserialize_with = "deserialize_folders")]
    pub folders: Vec<String>,
    #[serde(default = "default_interval")]
    pub interval_seconds: u64,
    #[serde(default = "default_true")]
    pub initial_sync: bool,
    #[serde(default = "default_lookback_days")]
    pub lookback_days: u32,
}

fn default_folders() -> Vec<String> {
    vec!["inbox".to_string()]
}

fn deserialize_folders<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum FoldersInput {
        One(String),
        Many(Vec<String>),
    }

    match FoldersInput::deserialize(deserializer)? {
        FoldersInput::One(value) => Ok(value
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToOwned::to_owned)
            .collect()),
        FoldersInput::Many(values) => Ok(values),
    }
}

fn default_interval() -> u64 {
    30
}

fn default_lookback_days() -> u32 {
    7
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            folders: default_folders(),
            interval_seconds: default_interval(),
            initial_sync: true,
            lookback_days: default_lookback_days(),
        }
    }
}

impl Config {
    pub fn load(path: &PathBuf) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Config::default());
        }

        let content =
            std::fs::read_to_string(path).map_err(|e| ConfigError::LoadError(e.to_string()))?;

        let config: Config =
            toml::from_str(&content).map_err(|e| ConfigError::LoadError(e.to_string()))?;

        config.exchange.validate()?;
        Ok(config)
    }

    pub fn load_from_env() -> Result<Self, ConfigError> {
        let mut config = Config::default();

        if let Ok(username) = std::env::var("EWS_USERNAME") {
            config.exchange.username = username;
        }
        if let Ok(password) = std::env::var("EWS_PASSWORD") {
            config.exchange.password = password;
        }
        if let Ok(email) = std::env::var("EWS_EMAIL") {
            config.exchange.email = email;
        }
        if let Ok(url) = std::env::var("EWS_URL") {
            config.exchange.ews_url = Some(url);
        }
        if let Ok(autodiscover) = std::env::var("EWS_AUTODISCOVER") {
            config.exchange.autodiscover = autodiscover.eq_ignore_ascii_case("true")
                || autodiscover == "1"
                || autodiscover.eq_ignore_ascii_case("yes");
        }
        if let Ok(auth_mode) = std::env::var("EWS_AUTH_MODE") {
            config.exchange.auth_mode = auth_mode;
        }
        if let Ok(value) = std::env::var("EWS_RETRY_MAX_ATTEMPTS") {
            if let Ok(parsed) = value.parse::<u32>() {
                config.exchange.retry_max_attempts = parsed;
            }
        }
        if let Ok(value) = std::env::var("EWS_RETRY_BASE_MS") {
            if let Ok(parsed) = value.parse::<u64>() {
                config.exchange.retry_base_ms = parsed;
            }
        }
        if let Ok(value) = std::env::var("EWS_RETRY_MAX_BACKOFF_MS") {
            if let Ok(parsed) = value.parse::<u64>() {
                config.exchange.retry_max_backoff_ms = parsed;
            }
        }
        if let Ok(folders) = std::env::var("EWS_SYNC_FOLDERS") {
            config.sync.folders = folders
                .split(',')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToOwned::to_owned)
                .collect();
        }
        if let Ok(interval) = std::env::var("EWS_SYNC_INTERVAL_SECONDS") {
            if let Ok(parsed) = interval.parse::<u64>() {
                config.sync.interval_seconds = parsed;
            }
        }
        if let Ok(days) = std::env::var("EWS_SYNC_LOOKBACK_DAYS") {
            if let Ok(parsed) = days.parse::<u32>() {
                config.sync.lookback_days = parsed;
            }
        }

        config.exchange.validate()?;
        Ok(config)
    }
}
