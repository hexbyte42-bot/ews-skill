pub mod cache;
pub mod config;
pub mod email_service;
pub mod ews_client;
pub mod graph_auth;
pub mod graph_client;
pub mod skill;
pub mod sync_engine;

use cache::{Database, Repository};
use config::Config;
use email_service::EmailService;
use ews_client::{ntlm_supported, EwsClient, EwsClientOptions};
use graph_auth::{token_state, GraphAuthConfig};
use graph_client::{GraphClient, GraphSearchOptions};
use serde_json::Value;
use skill::EmailSkill;
use std::sync::{Arc, Mutex};
use sync_engine::SyncEngine;
use tokio::runtime::Runtime;
use tracing::error;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub struct EwsSkill {
    email_skill: Option<Arc<Mutex<EmailSkill>>>,
    sync_engine: Option<SyncEngine>,
    graph_client: Option<GraphClient>,
    protocol: String,
    graph_auth: Option<GraphAuthConfig>,
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

        if config.mail_protocol.eq_ignore_ascii_case("graph") {
            let db = Database::new(&config.cache.path).map_err(|e| e.to_string())?;
            db.ensure_account_scope(&format!(
                "graph:{}:{}",
                config.graph.tenant_id, config.graph.client_id
            ))
            .map_err(|e| e.to_string())?;
            let graph_auth = GraphAuthConfig {
                tenant_id: config.graph.tenant_id.clone(),
                client_id: config.graph.client_id.clone(),
            };
            let graph_client = GraphClient::new(graph_auth.clone());
            return Ok(Self {
                email_skill: None,
                sync_engine: None,
                graph_client: Some(graph_client),
                protocol: "graph".to_string(),
                graph_auth: Some(graph_auth),
            });
        }

        let db = Database::new(&config.cache.path).map_err(|e| e.to_string())?;
        db.ensure_account_scope(&config.exchange.email)
            .map_err(|e| e.to_string())?;

        let repository = Repository::new(db);

        let credentials = ews_client::EwsCredentials {
            username: config.exchange.username.clone(),
            password: config.exchange.password.clone(),
            email: config.exchange.email.clone(),
            auth_mode: config.exchange.auth_mode.clone(),
        };

        if credentials.auth_mode.eq_ignore_ascii_case("ntlm") && !ntlm_supported() {
            return Err(
                "NTLM authentication requested, but this ews_skilld build does not include NTLM-enabled libcurl"
                    .to_string(),
            );
        }

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
        sync_engine.start_polling(&runtime);

        let init_engine = sync_engine.clone();
        runtime.spawn(async move {
            if let Err(e) = init_engine.initialize().await {
                error!("sync engine initialization failed: {}", e);
            }
        });

        let email_service = EmailService::new(sync_engine.clone(), repository);
        let email_skill = EmailSkill::new(email_service, runtime.clone());

        Ok(Self {
            email_skill: Some(Arc::new(Mutex::new(email_skill))),
            sync_engine: Some(sync_engine),
            graph_client: None,
            protocol: "ews".to_string(),
            graph_auth: None,
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
        if self.protocol == "graph" {
            return match self
                .graph_client
                .as_ref()
                .ok_or_else(|| "graph client not initialized".to_string())
            {
                Ok(client) => match client.list_folders() {
                    Ok(folders) => skill::ToolResult::ok(serde_json::json!({
                        "folders": folders.into_iter().map(|f| serde_json::json!({
                            "id": f.id,
                            "display_name": f.display_name,
                            "unread_count": f.unread_count,
                            "total_count": f.total_count,
                        })).collect::<Vec<_>>()
                    })),
                    Err(e) => skill::ToolResult::err(e),
                },
                Err(e) => skill::ToolResult::err(e),
            };
        }

        match self.email_skill.as_ref().and_then(|s| s.lock().ok()) {
            Some(skill) => skill.list_folders(),
            None => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn list_emails(
        &self,
        folder_name: Option<String>,
        limit: Option<i32>,
        unread_only: Option<bool>,
    ) -> skill::ToolResult {
        if self.protocol == "graph" {
            let client = match self.graph_client.as_ref() {
                Some(c) => c,
                None => return skill::ToolResult::err("graph client not initialized".to_string()),
            };
            let folder = folder_name.unwrap_or_else(|| "inbox".to_string());
            return match client.list_emails(
                &folder,
                limit.unwrap_or(50),
                unread_only.unwrap_or(false),
            ) {
                Ok(emails) => skill::ToolResult::ok(serde_json::json!({
                    "emails": emails.into_iter().map(|e| serde_json::json!({
                        "id": e.id,
                        "subject": e.subject,
                        "sender_name": e.sender_name,
                        "sender_email": e.sender_email,
                        "is_read": e.is_read,
                        "has_attachments": e.has_attachments,
                        "importance": e.importance,
                        "datetime_received": e.datetime_received,
                    })).collect::<Vec<_>>()
                })),
                Err(e) => skill::ToolResult::err(e),
            };
        }

        match self.email_skill.as_ref().and_then(|s| s.lock().ok()) {
            Some(skill) => skill.list_emails(folder_name, limit, unread_only),
            None => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn read_email(&self, email_id: String) -> skill::ToolResult {
        if self.protocol == "graph" {
            let client = match self.graph_client.as_ref() {
                Some(c) => c,
                None => return skill::ToolResult::err("graph client not initialized".to_string()),
            };
            return match client.read_email(&email_id) {
                Ok(email) => skill::ToolResult::ok(serde_json::json!({
                    "id": email.id,
                    "subject": email.subject,
                    "sender_name": email.sender_name,
                    "sender_email": email.sender_email,
                    "to_recipients": email.to_recipients,
                    "cc_recipients": email.cc_recipients,
                    "body_text": email.body_text,
                    "body_html": email.body_html,
                    "is_read": email.is_read,
                    "has_attachments": email.has_attachments,
                    "importance": email.importance,
                    "datetime_received": email.datetime_received,
                    "datetime_sent": email.datetime_sent,
                })),
                Err(e) => skill::ToolResult::err(e),
            };
        }

        match self.email_skill.as_ref().and_then(|s| s.lock().ok()) {
            Some(skill) => skill.email_read(email_id),
            None => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn search(
        &self,
        query: Option<String>,
        subject: Option<String>,
        sender: Option<String>,
        date_from: Option<String>,
        date_to: Option<String>,
        folder_name: Option<String>,
        limit: Option<i32>,
        include_body: Option<bool>,
    ) -> skill::ToolResult {
        if self.protocol == "graph" {
            let client = match self.graph_client.as_ref() {
                Some(c) => c,
                None => return skill::ToolResult::err("graph client not initialized".to_string()),
            };
            let has_filter = query.as_ref().is_some_and(|v| !v.trim().is_empty())
                || subject.as_ref().is_some_and(|v| !v.trim().is_empty())
                || sender.as_ref().is_some_and(|v| !v.trim().is_empty())
                || date_from.as_ref().is_some_and(|v| !v.trim().is_empty())
                || date_to.as_ref().is_some_and(|v| !v.trim().is_empty())
                || folder_name.as_ref().is_some_and(|v| !v.trim().is_empty());
            if !has_filter {
                return skill::ToolResult::err(
                    "at least one search filter is required (query, subject, sender, date_from, date_to, folder_name)".to_string(),
                );
            }

            let date_from_parsed = match date_from.as_deref().map(parse_rfc3339_utc).transpose() {
                Ok(v) => v,
                Err(e) => return skill::ToolResult::err(format!("invalid date_from: {}", e)),
            };
            let date_to_parsed = match date_to.as_deref().map(parse_rfc3339_utc).transpose() {
                Ok(v) => v,
                Err(e) => return skill::ToolResult::err(format!("invalid date_to: {}", e)),
            };

            return match client.search_emails(GraphSearchOptions {
                query,
                subject,
                sender,
                date_from: date_from_parsed,
                date_to: date_to_parsed,
                folder_name,
                limit: limit.unwrap_or(50).clamp(1, 200),
                include_body: include_body.unwrap_or(true),
            }) {
                Ok(results) => skill::ToolResult::ok(serde_json::json!({
                    "results": results.into_iter().map(|e| serde_json::json!({
                        "id": e.id,
                        "subject": e.subject,
                        "sender_name": e.sender_name,
                        "sender_email": e.sender_email,
                        "is_read": e.is_read,
                        "datetime_received": e.datetime_received,
                    })).collect::<Vec<_>>()
                })),
                Err(e) => skill::ToolResult::err(e),
            };
        }

        match self.email_skill.as_ref().and_then(|s| s.lock().ok()) {
            Some(skill) => skill.email_search(
                query,
                subject,
                sender,
                date_from,
                date_to,
                folder_name,
                limit,
                include_body,
            ),
            None => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn get_unread(&self, folder_name: Option<String>, limit: Option<i32>) -> skill::ToolResult {
        if self.protocol == "graph" {
            let client = match self.graph_client.as_ref() {
                Some(c) => c,
                None => return skill::ToolResult::err("graph client not initialized".to_string()),
            };
            let folder = folder_name.unwrap_or_else(|| "inbox".to_string());
            return match client.list_emails(&folder, limit.unwrap_or(20), true) {
                Ok(emails) => skill::ToolResult::ok(serde_json::json!({
                    "emails": emails.into_iter().map(|e| serde_json::json!({
                        "id": e.id,
                        "subject": e.subject,
                        "sender_name": e.sender_name,
                        "sender_email": e.sender_email,
                        "datetime_received": e.datetime_received,
                    })).collect::<Vec<_>>()
                })),
                Err(e) => skill::ToolResult::err(e),
            };
        }

        match self.email_skill.as_ref().and_then(|s| s.lock().ok()) {
            Some(skill) => skill.get_unread(folder_name, limit),
            None => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn mark_read(&self, email_id: String, is_read: bool) -> skill::ToolResult {
        if self.protocol == "graph" {
            let client = match self.graph_client.as_ref() {
                Some(c) => c,
                None => return skill::ToolResult::err("graph client not initialized".to_string()),
            };
            return match client.mark_read(&email_id, is_read) {
                Ok(_) => skill::ToolResult::ok(serde_json::json!({"message": "Email marked"})),
                Err(e) => skill::ToolResult::err(e),
            };
        }
        match self.email_skill.as_ref().and_then(|s| s.lock().ok()) {
            Some(skill) => skill.mark_read(email_id, is_read),
            None => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn send(&self, to: String, subject: String, body: String) -> skill::ToolResult {
        if self.protocol == "graph" {
            let client = match self.graph_client.as_ref() {
                Some(c) => c,
                None => return skill::ToolResult::err("graph client not initialized".to_string()),
            };
            return match client.send_email(&to, &subject, &body) {
                Ok(_) => skill::ToolResult::ok(serde_json::json!({"message": "Email sent"})),
                Err(e) => skill::ToolResult::err(e),
            };
        }
        match self.email_skill.as_ref().and_then(|s| s.lock().ok()) {
            Some(skill) => skill.send_email(to, subject, body),
            None => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn move_email(&self, email_id: String, destination_folder: String) -> skill::ToolResult {
        if self.protocol == "graph" {
            let client = match self.graph_client.as_ref() {
                Some(c) => c,
                None => return skill::ToolResult::err("graph client not initialized".to_string()),
            };
            return match client.move_email(&email_id, &destination_folder) {
                Ok(new_id) => skill::ToolResult::ok(serde_json::json!({"new_id": new_id})),
                Err(e) => skill::ToolResult::err(e),
            };
        }
        match self.email_skill.as_ref().and_then(|s| s.lock().ok()) {
            Some(skill) => skill.move_email(email_id, destination_folder),
            None => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn delete(&self, email_id: String, skip_trash: bool) -> skill::ToolResult {
        if self.protocol == "graph" {
            let client = match self.graph_client.as_ref() {
                Some(c) => c,
                None => return skill::ToolResult::err("graph client not initialized".to_string()),
            };
            return match client.delete_email(&email_id, skip_trash) {
                Ok(_) => skill::ToolResult::ok(serde_json::json!({"message": "Email deleted"})),
                Err(e) => skill::ToolResult::err(e),
            };
        }
        match self.email_skill.as_ref().and_then(|s| s.lock().ok()) {
            Some(skill) => skill.delete_email(email_id, skip_trash),
            None => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn sync(&self) -> skill::ToolResult {
        if self.protocol == "graph" {
            return skill::ToolResult::ok(serde_json::json!({
                "message": "Graph mode uses live API reads; explicit cache sync is not required"
            }));
        }
        match self.email_skill.as_ref().and_then(|s| s.lock().ok()) {
            Some(skill) => skill.sync_now(),
            None => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn add_folder(&self, folder_name: String) -> skill::ToolResult {
        if self.protocol == "graph" {
            return skill::ToolResult::ok(serde_json::json!({
                "message": format!("Graph mode does not require folder enrollment; requested '{}'", folder_name)
            }));
        }
        match self.email_skill.as_ref().and_then(|s| s.lock().ok()) {
            Some(skill) => skill.add_folder(folder_name),
            None => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
        }
    }

    pub fn health(&self) -> skill::ToolResult {
        if self.protocol == "graph" {
            let auth = self
                .graph_auth
                .as_ref()
                .cloned()
                .ok_or_else(|| "graph auth config missing".to_string());
            return match auth {
                Ok(auth) => {
                    let (ok, err) = token_state(&auth);
                    let status = if ok { "ready" } else { "auth_required" };
                    skill::ToolResult::ok(serde_json::json!({
                        "backend": "graph",
                        "status": status,
                        "auth_ok": ok,
                        "inbox_found": ok,
                        "initial_sync_in_progress": false,
                        "progress": "0/0 folders",
                        "synced_folders": 0,
                        "total_folders": 0,
                        "last_sync_at": Value::Null,
                        "error": err,
                    }))
                }
                Err(e) => skill::ToolResult::err(e),
            };
        }

        match self.email_skill.as_ref().and_then(|s| s.lock().ok()) {
            Some(skill) => skill.health(),
            None => skill::ToolResult::err("failed to acquire email skill lock".to_string()),
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
                        return skill::ToolResult::err(
                            "missing required argument: email_id".to_string(),
                        )
                    }
                };
                self.read_email(email_id)
            }
            "email_search" => {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned);
                let subject = args
                    .get("subject")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned);
                let sender = args
                    .get("sender")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned);
                let date_from = args
                    .get("date_from")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned);
                let date_to = args
                    .get("date_to")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned);
                let folder_name = args
                    .get("folder_name")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned);
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_i64())
                    .and_then(|v| i32::try_from(v).ok());
                let include_body = args.get("include_body").and_then(|v| v.as_bool());
                self.search(
                    query,
                    subject,
                    sender,
                    date_from,
                    date_to,
                    folder_name,
                    limit,
                    include_body,
                )
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
                        return skill::ToolResult::err(
                            "missing required argument: email_id".to_string(),
                        )
                    }
                };
                let is_read = match args.get("is_read").and_then(|v| v.as_bool()) {
                    Some(v) => v,
                    None => {
                        return skill::ToolResult::err(
                            "missing required argument: is_read".to_string(),
                        )
                    }
                };
                self.mark_read(email_id, is_read)
            }
            "email_send" => {
                let to = match args.get("to").and_then(|v| v.as_str()) {
                    Some(v) => v.to_string(),
                    None => {
                        return skill::ToolResult::err("missing required argument: to".to_string())
                    }
                };
                let subject = match args.get("subject").and_then(|v| v.as_str()) {
                    Some(v) => v.to_string(),
                    None => {
                        return skill::ToolResult::err(
                            "missing required argument: subject".to_string(),
                        )
                    }
                };
                let body = match args.get("body").and_then(|v| v.as_str()) {
                    Some(v) => v.to_string(),
                    None => {
                        return skill::ToolResult::err(
                            "missing required argument: body".to_string(),
                        )
                    }
                };
                self.send(to, subject, body)
            }
            "email_move" => {
                let email_id = match args.get("email_id").and_then(|v| v.as_str()) {
                    Some(v) => v.to_string(),
                    None => {
                        return skill::ToolResult::err(
                            "missing required argument: email_id".to_string(),
                        )
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
                        return skill::ToolResult::err(
                            "missing required argument: email_id".to_string(),
                        )
                    }
                };
                let skip_trash = args
                    .get("skip_trash")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                self.delete(email_id, skip_trash)
            }
            "email_sync_now" => self.sync(),
            "email_add_folder" => {
                let folder_name = match args.get("folder_name").and_then(|v| v.as_str()) {
                    Some(v) => v.to_string(),
                    None => {
                        return skill::ToolResult::err(
                            "missing required argument: folder_name".to_string(),
                        )
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
        if let Some(sync_engine) = &self.sync_engine {
            sync_engine.stop_polling();
        }
    }
}

fn parse_rfc3339_utc(value: &str) -> Result<chrono::DateTime<chrono::Utc>, String> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|v| v.with_timezone(&chrono::Utc))
        .map_err(|e| e.to_string())
}
