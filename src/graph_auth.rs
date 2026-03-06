use chrono::{DateTime, Duration, Utc};
use directories::ProjectDirs;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct GraphAuthConfig {
    pub tenant_id: String,
    pub client_id: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TokenCache {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: i64,
    interval: Option<u64>,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    error: Option<String>,
    error_description: Option<String>,
}

pub fn login_device_code(config: &GraphAuthConfig) -> Result<(), String> {
    let client = Client::new();
    let scope = requested_scopes();
    let dc_url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/devicecode",
        config.tenant_id
    );

    let device_code: DeviceCodeResponse = client
        .post(dc_url)
        .form(&[
            ("client_id", config.client_id.as_str()),
            ("scope", scope.as_str()),
        ])
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;

    if let Some(message) = device_code.message.as_deref() {
        println!("{}", message);
    } else {
        println!(
            "Open {} and enter code {}",
            device_code.verification_uri, device_code.user_code
        );
    }
    println!(
        "If admin consent is required, ask an admin to grant access here: https://login.microsoftonline.com/{}/adminconsent?client_id={}",
        config.tenant_id, config.client_id
    );

    let token_url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
        config.tenant_id
    );
    let interval = device_code.interval.unwrap_or(5).max(2);
    let expires_deadline = Utc::now() + Duration::seconds(device_code.expires_in.max(60));

    loop {
        if Utc::now() >= expires_deadline {
            return Err("device code expired before authorization completed".to_string());
        }

        let token: TokenResponse = client
            .post(&token_url)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", config.client_id.as_str()),
                ("device_code", device_code.device_code.as_str()),
            ])
            .send()
            .map_err(|e| e.to_string())?
            .json()
            .map_err(|e| e.to_string())?;

        if let Some(access_token) = token.access_token {
            let refresh_token = token.refresh_token.unwrap_or_default();
            let expires_in = token.expires_in.unwrap_or(3600).max(60);
            let cache = TokenCache {
                access_token,
                refresh_token,
                expires_at: Utc::now() + Duration::seconds(expires_in),
            };
            write_cache(&cache)?;
            println!("Graph delegated login successful.");
            return Ok(());
        }

        match token.error.as_deref() {
            Some("authorization_pending") => {
                std::thread::sleep(std::time::Duration::from_secs(interval));
            }
            Some("slow_down") => {
                std::thread::sleep(std::time::Duration::from_secs(interval + 3));
            }
            Some(err) => {
                let desc = token.error_description.unwrap_or_default();
                if desc.contains("AADSTS65001")
                    || desc.to_lowercase().contains("consent")
                    || desc.to_lowercase().contains("admin")
                {
                    return Err(format!(
                        "admin consent required: {}. Admin grant URL: https://login.microsoftonline.com/{}/adminconsent?client_id={}",
                        desc, config.tenant_id, config.client_id
                    ));
                }
                return Err(format!("device login failed: {} ({})", err, desc));
            }
            None => {
                return Err("device login failed: unknown token response".to_string());
            }
        }
    }
}

pub fn logout() -> Result<(), String> {
    let path = cache_path();
    if path.exists() {
        fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn get_access_token(config: &GraphAuthConfig) -> Result<String, String> {
    let mut cache = read_cache().ok_or_else(|| "graph delegated token not found".to_string())?;
    if cache.expires_at > Utc::now() + Duration::seconds(90) {
        return Ok(cache.access_token);
    }

    if cache.refresh_token.trim().is_empty() {
        return Err("graph delegated token expired and no refresh token available".to_string());
    }

    let client = Client::new();
    let token_url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
        config.tenant_id
    );

    let refreshed: TokenResponse = client
        .post(token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", config.client_id.as_str()),
            ("refresh_token", cache.refresh_token.as_str()),
            ("scope", requested_scopes().as_str()),
        ])
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;

    if let Some(access_token) = refreshed.access_token {
        let expires_in = refreshed.expires_in.unwrap_or(3600).max(60);
        cache.access_token = access_token;
        if let Some(refresh_token) = refreshed.refresh_token {
            cache.refresh_token = refresh_token;
        }
        cache.expires_at = Utc::now() + Duration::seconds(expires_in);
        write_cache(&cache)?;
        return Ok(cache.access_token);
    }

    Err(format!(
        "graph token refresh failed: {} ({})",
        refreshed
            .error
            .unwrap_or_else(|| "unknown_error".to_string()),
        refreshed.error_description.unwrap_or_default()
    ))
}

pub fn token_state(config: &GraphAuthConfig) -> (bool, Option<String>) {
    match get_access_token(config) {
        Ok(_) => (true, None),
        Err(e) => (false, Some(e)),
    }
}

fn cache_path() -> PathBuf {
    if let Some(proj_dirs) = ProjectDirs::from("com", "ews-skill", "ews-skill") {
        let dir = proj_dirs.data_local_dir();
        fs::create_dir_all(dir).ok();
        return dir.join("graph_token_cache.json");
    }
    PathBuf::from("graph_token_cache.json")
}

fn read_cache() -> Option<TokenCache> {
    let path = cache_path();
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_cache(cache: &TokenCache) -> Result<(), String> {
    let path = cache_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let payload = serde_json::to_string_pretty(cache).map_err(|e| e.to_string())?;
    fs::write(&path, payload).map_err(|e| e.to_string())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path)
            .map_err(|e| e.to_string())?
            .permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&path, perms).map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn requested_scopes() -> String {
    std::env::var("GRAPH_SCOPES")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            "offline_access User.Read Mail.Read Mail.ReadWrite Mail.Send".to_string()
        })
}
