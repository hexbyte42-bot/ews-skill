pub mod cache;
pub mod config;
pub mod email_service;
pub mod ews_client;
pub mod skill;
pub mod sync_engine;

use cache::{Database, Repository};
use config::Config;
use email_service::EmailService;
use ews_client::{EwsClient, EwsClientOptions};
use serde_json::Value;
use skill::EmailSkill;
use std::sync::{Arc, Mutex};
use sync_engine::SyncEngine;
use tokio::runtime::Runtime;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub struct EwsSkill {
    email_skill: Arc<Mutex<EmailSkill>>,
    sync_engine: SyncEngine,
}

impl EwsSkill {
    pub fn new(config: Config) -> Result<Self, String> {
        let log_level = std::env::var("EWS_LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level));

        let _ = tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
            .with(filter)
            .try_init();

        let db = Database::new(&config.cache.path).map_err(|e| e.to_string())?;

        let repository = Repository::new(db);

        let credentials = ews_client::EwsCredentials {
            username: config.exchange.username.clone(),
            password: config.exchange.password.clone(),
            email: config.exchange.email.clone(),
            auth_mode: config.exchange.auth_mode.clone(),
        };

        let client_options = EwsClientOptions {
            retry_max_attempts: config.exchange.retry_max_attempts,
            retry_base_ms: config.exchange.retry_base_ms,
            retry_max_backoff_ms: config.exchange.retry_max_backoff_ms,
        };

        let mut ews_client =
            EwsClient::new(credentials, config.exchange.ews_url.clone(), client_options);

        let runtime = Arc::new(Runtime::new().map_err(|e| e.to_string())?);

        if config.exchange.autodiscover {
            runtime
                .block_on(ews_client.discover())
                .map_err(|e| e.to_string())?;
        }

        let sync_engine = SyncEngine::new(ews_client, repository.clone(), config.clone());
        runtime.block_on(sync_engine.initialize())?;

        sync_engine.start_polling(&runtime);

        let email_service = EmailService::new(sync_engine.clone(), repository);
        let email_skill = EmailSkill::new(email_service, runtime.clone());

        Ok(Self {
            email_skill: Arc::new(Mutex::new(email_skill)),
            sync_engine,
        })
    }

    pub fn from_env() -> Result<Self, String> {
        let config = Config::load_from_env().map_err(|e| e.to_string())?;
        Self::new(config)
    }

    pub fn from_config_file(path: &std::path::PathBuf) -> Result<Self, String> {
        let config = Config::load(path).map_err(|e| e.to_string())?;
        Self::new(config)
    }

    pub fn list_folders(&self) -> skill::ToolResult {
        match self.email_skill.lock() {
            Ok(skill) => skill.list_folders(),
            Err(_) => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn list_emails(
        &self,
        folder_name: Option<String>,
        limit: Option<i32>,
        unread_only: Option<bool>,
    ) -> skill::ToolResult {
        match self.email_skill.lock() {
            Ok(skill) => skill.list_emails(folder_name, limit, unread_only),
            Err(_) => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn read_email(&self, email_id: String) -> skill::ToolResult {
        match self.email_skill.lock() {
            Ok(skill) => skill.email_read(email_id),
            Err(_) => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn search(&self, query: String, limit: Option<i32>) -> skill::ToolResult {
        match self.email_skill.lock() {
            Ok(skill) => skill.email_search(query, limit),
            Err(_) => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn get_unread(&self, folder_name: Option<String>, limit: Option<i32>) -> skill::ToolResult {
        match self.email_skill.lock() {
            Ok(skill) => skill.get_unread(folder_name, limit),
            Err(_) => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn mark_read(&self, email_id: String, is_read: bool) -> skill::ToolResult {
        match self.email_skill.lock() {
            Ok(skill) => skill.mark_read(email_id, is_read),
            Err(_) => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn send(&self, to: String, subject: String, body: String) -> skill::ToolResult {
        match self.email_skill.lock() {
            Ok(skill) => skill.send_email(to, subject, body),
            Err(_) => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn move_email(&self, email_id: String, destination_folder: String) -> skill::ToolResult {
        match self.email_skill.lock() {
            Ok(skill) => skill.move_email(email_id, destination_folder),
            Err(_) => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn delete(&self, email_id: String) -> skill::ToolResult {
        match self.email_skill.lock() {
            Ok(skill) => skill.delete_email(email_id),
            Err(_) => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn sync(&self) -> skill::ToolResult {
        match self.email_skill.lock() {
            Ok(skill) => skill.sync_now(),
            Err(_) => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn add_folder(&self, folder_name: String) -> skill::ToolResult {
        match self.email_skill.lock() {
            Ok(skill) => skill.add_folder(folder_name),
            Err(_) => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn health(&self) -> skill::ToolResult {
        match self.email_skill.lock() {
            Ok(skill) => skill.health(),
            Err(_) => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn get_tools() -> Vec<serde_json::Value> {
        EmailSkill::get_tool_definitions()
    }

    pub fn execute_tool(&self, tool_name: &str, args: Value) -> skill::ToolResult {
        match tool_name {
            "email_health" => self.health(),
            "email_list_folders" => self.list_folders(),
            "email_list" => {
                let folder_name = args
                    .get("folder_name")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned);
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_i64())
                    .and_then(|v| i32::try_from(v).ok());
                let unread_only = args.get("unread_only").and_then(|v| v.as_bool());
                self.list_emails(folder_name, limit, unread_only)
            }
            "email_read" => {
                let email_id = match args.get("email_id").and_then(|v| v.as_str()) {
                    Some(v) => v.to_string(),
                    None => {
                        return skill::ToolResult::err("missing required argument: email_id".to_string())
                    }
                };
                self.read_email(email_id)
            }
            "email_search" => {
                let query = match args.get("query").and_then(|v| v.as_str()) {
                    Some(v) => v.to_string(),
                    None => {
                        return skill::ToolResult::err("missing required argument: query".to_string())
                    }
                };
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_i64())
                    .and_then(|v| i32::try_from(v).ok());
                self.search(query, limit)
            }
            "email_get_unread" => {
                let folder_name = args
                    .get("folder_name")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned);
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_i64())
                    .and_then(|v| i32::try_from(v).ok());
                self.get_unread(folder_name, limit)
            }
            "email_mark_read" => {
                let email_id = match args.get("email_id").and_then(|v| v.as_str()) {
                    Some(v) => v.to_string(),
                    None => {
                        return skill::ToolResult::err("missing required argument: email_id".to_string())
                    }
                };
                let is_read = match args.get("is_read").and_then(|v| v.as_bool()) {
                    Some(v) => v,
                    None => {
                        return skill::ToolResult::err("missing required argument: is_read".to_string())
                    }
                };
                self.mark_read(email_id, is_read)
            }
            "email_send" => {
                let to = match args.get("to").and_then(|v| v.as_str()) {
                    Some(v) => v.to_string(),
                    None => return skill::ToolResult::err("missing required argument: to".to_string()),
                };
                let subject = match args.get("subject").and_then(|v| v.as_str()) {
                    Some(v) => v.to_string(),
                    None => {
                        return skill::ToolResult::err("missing required argument: subject".to_string())
                    }
                };
                let body = match args.get("body").and_then(|v| v.as_str()) {
                    Some(v) => v.to_string(),
                    None => {
                        return skill::ToolResult::err("missing required argument: body".to_string())
                    }
                };
                self.send(to, subject, body)
            }
            "email_move" => {
                let email_id = match args.get("email_id").and_then(|v| v.as_str()) {
                    Some(v) => v.to_string(),
                    None => {
                        return skill::ToolResult::err("missing required argument: email_id".to_string())
                    }
                };
                let destination_folder =
                    match args.get("destination_folder").and_then(|v| v.as_str()) {
                        Some(v) => v.to_string(),
                        None => {
                            return skill::ToolResult::err(
                                "missing required argument: destination_folder".to_string(),
                            )
                        }
                    };
                self.move_email(email_id, destination_folder)
            }
            "email_delete" => {
                let email_id = match args.get("email_id").and_then(|v| v.as_str()) {
                    Some(v) => v.to_string(),
                    None => {
                        return skill::ToolResult::err("missing required argument: email_id".to_string())
                    }
                };
                self.delete(email_id)
            }
            "email_sync_now" => self.sync(),
            "email_add_folder" => {
                let folder_name = match args.get("folder_name").and_then(|v| v.as_str()) {
                    Some(v) => v.to_string(),
                    None => {
                        return skill::ToolResult::err("missing required argument: folder_name".to_string())
                    }
                };
                self.add_folder(folder_name)
            }
            _ => skill::ToolResult::err(format!("unknown tool: {}", tool_name)),
        }
    }
}

impl Drop for EwsSkill {
    fn drop(&mut self) {
        self.sync_engine.stop_polling();
    }
}
