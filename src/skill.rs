use crate::email_service::EmailService;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::runtime::Runtime;

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
}

impl ToolResult {
    pub fn ok(data: serde_json::Value) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(error: String) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(error),
        }
    }
}

pub struct EmailSkill {
    service: EmailService,
    runtime: Arc<Runtime>,
}

impl EmailSkill {
    pub fn new(service: EmailService, runtime: Arc<Runtime>) -> Self {
        Self { service, runtime }
    }

    pub fn list_folders(&self) -> ToolResult {
        let folders = self.service.list_folders();
        let data: Vec<_> = folders
            .iter()
            .map(|f| {
                json!({
                    "id": f.id,
                    "display_name": f.display_name,
                    "total_count": f.total_count,
                    "unread_count": f.unread_count,
                })
            })
            .collect();

        ToolResult::ok(json!({ "folders": data }))
    }

    pub fn list_emails(
        &self,
        folder_name: Option<String>,
        limit: Option<i32>,
        unread_only: Option<bool>,
    ) -> ToolResult {
        let emails = self
            .service
            .list_emails(crate::email_service::EmailListOptions {
                folder_id: None,
                folder_name,
                limit,
                unread_only,
            });

        let data: Vec<_> = emails
            .iter()
            .map(|e| {
                json!({
                    "id": e.id,
                    "subject": e.subject,
                    "sender_name": e.sender_name,
                    "sender_email": e.sender_email,
                    "is_read": e.is_read,
                    "has_attachments": e.has_attachments,
                    "importance": e.importance,
                    "datetime_received": e.datetime_received,
                })
            })
            .collect();

        ToolResult::ok(json!({ "emails": data }))
    }

    pub fn email_read(&self, email_id: String) -> ToolResult {
        match self.service.get_email(&email_id) {
            Some(email) => {
                let data = json!({
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
                });
                ToolResult::ok(data)
            }
            None => ToolResult::err("Email not found".to_string()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn email_search(
        &self,
        query: Option<String>,
        subject: Option<String>,
        sender: Option<String>,
        date_from: Option<String>,
        date_to: Option<String>,
        folder_name: Option<String>,
        limit: Option<i32>,
        include_body: Option<bool>,
    ) -> ToolResult {
        let emails = match self
            .service
            .search(crate::email_service::EmailSearchOptions {
                query,
                subject,
                sender,
                date_from,
                date_to,
                folder_name,
                limit,
                include_body,
            }) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(e),
        };

        let data: Vec<_> = emails
            .iter()
            .map(|e| {
                json!({
                    "id": e.id,
                    "subject": e.subject,
                    "sender_name": e.sender_name,
                    "sender_email": e.sender_email,
                    "is_read": e.is_read,
                    "datetime_received": e.datetime_received,
                })
            })
            .collect();

        ToolResult::ok(json!({ "results": data }))
    }

    pub fn get_unread(&self, folder_name: Option<String>, limit: Option<i32>) -> ToolResult {
        let folder = folder_name.unwrap_or_else(|| "inbox".to_string());
        let emails = self.service.get_unread(&folder, limit.unwrap_or(20));

        let data: Vec<_> = emails
            .iter()
            .map(|e| {
                json!({
                    "id": e.id,
                    "subject": e.subject,
                    "sender_name": e.sender_name,
                    "sender_email": e.sender_email,
                    "datetime_received": e.datetime_received,
                })
            })
            .collect();

        ToolResult::ok(json!({ "emails": data }))
    }

    pub fn mark_read(&self, email_id: String, is_read: bool) -> ToolResult {
        match self
            .runtime
            .block_on(self.service.mark_read(&email_id, is_read))
        {
            Ok(_) => ToolResult::ok(json!({ "message": "Email marked as read" })),
            Err(e) => ToolResult::err(e),
        }
    }

    pub fn send_email(&self, to: String, subject: String, body: String) -> ToolResult {
        match self
            .runtime
            .block_on(self.service.send_email(&to, &subject, &body))
        {
            Ok(_) => ToolResult::ok(json!({ "message": "Email sent" })),
            Err(e) => ToolResult::err(e),
        }
    }

    pub fn move_email(&self, email_id: String, destination_folder: String) -> ToolResult {
        match self
            .runtime
            .block_on(self.service.move_email(&email_id, &destination_folder))
        {
            Ok(_) => ToolResult::ok(json!({ "message": "Email moved" })),
            Err(e) => ToolResult::err(e),
        }
    }

    pub fn delete_email(&self, email_id: String, skip_trash: bool) -> ToolResult {
        match self
            .runtime
            .block_on(self.service.delete_email(&email_id, skip_trash))
        {
            Ok(_) => ToolResult::ok(json!({ "message": "Email deleted" })),
            Err(e) => ToolResult::err(e),
        }
    }

    pub fn sync_now(&self) -> ToolResult {
        match self.runtime.block_on(self.service.sync_now()) {
            Ok(_) => ToolResult::ok(json!({ "message": "Sync completed" })),
            Err(e) => ToolResult::err(e),
        }
    }

    pub fn add_folder(&self, folder_name: String) -> ToolResult {
        match self
            .runtime
            .block_on(self.service.add_folder_to_sync(&folder_name))
        {
            Ok(_) => ToolResult::ok(json!({ "message": "Folder added to sync" })),
            Err(e) => ToolResult::err(e),
        }
    }

    pub fn health(&self) -> ToolResult {
        let health = self.runtime.block_on(self.service.health_check());
        ToolResult::ok(json!({
            "ews_url": health.ews_url,
            "status": health.status,
            "initial_sync_in_progress": health.initial_sync_in_progress,
            "progress": health.progress,
            "auth_ok": health.auth_ok,
            "inbox_found": health.inbox_found,
            "cached_folders": health.cached_folders,
            "cached_emails": health.cached_emails,
            "synced_folders": health.synced_folders,
            "total_folders": health.total_folders,
            "last_sync_at": health.last_sync_at,
        }))
    }

    pub fn get_tool_definitions() -> Vec<serde_json::Value> {
        vec![
            json!({
                "name": "email_list_folders",
                "description": "List all available email folders",
                "parameters": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            }),
            json!({
                "name": "email_list",
                "description": "List emails from a folder",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "folder_name": {
                            "type": "string",
                            "description": "Folder name or distinguished id (default: inbox)"
                        },
                        "limit": {
                            "type": "number",
                            "description": "Maximum number of emails to return (default: 50)"
                        },
                        "unread_only": {
                            "type": "boolean",
                            "description": "Only return unread emails"
                        }
                    },
                    "required": []
                }
            }),
            json!({
                "name": "email_read",
                "description": "Get full email content by ID",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "email_id": {
                            "type": "string",
                            "description": "The email ID"
                        }
                    },
                    "required": ["email_id"]
                }
            }),
            json!({
                "name": "email_search",
                "description": "Search emails by combined filters (query, subject, sender, time range, folder)",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Free-text query over subject/sender/body_text"
                        },
                        "subject": {
                            "type": "string",
                            "description": "Match subject text"
                        },
                        "sender": {
                            "type": "string",
                            "description": "Match sender email or name"
                        },
                        "date_from": {
                            "type": "string",
                            "description": "UTC lower bound (RFC3339), e.g. 2026-03-05T00:00:00Z"
                        },
                        "date_to": {
                            "type": "string",
                            "description": "UTC upper bound (RFC3339), e.g. 2026-03-06T00:00:00Z"
                        },
                        "folder_name": {
                            "type": "string",
                            "description": "Restrict search to folder display name"
                        },
                        "limit": {
                            "type": "number",
                            "description": "Maximum results (default: 50)"
                        },
                        "include_body": {
                            "type": "boolean",
                            "description": "Include body_text in query matching (default: true)"
                        }
                    },
                    "required": []
                }
            }),
            json!({
                "name": "email_get_unread",
                "description": "Get unread emails from inbox",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "folder_name": {
                            "type": "string",
                            "description": "Folder name or distinguished id (default: inbox)"
                        },
                        "limit": {
                            "type": "number",
                            "description": "Maximum results (default: 20)"
                        }
                    },
                    "required": []
                }
            }),
            json!({
                "name": "email_mark_read",
                "description": "Mark email as read or unread",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "email_id": {
                            "type": "string",
                            "description": "The email ID"
                        },
                        "is_read": {
                            "type": "boolean",
                            "description": "Mark as read (true) or unread (false)"
                        }
                    },
                    "required": ["email_id", "is_read"]
                }
            }),
            json!({
                "name": "email_send",
                "description": "Send a new email",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "to": {
                            "type": "string",
                            "description": "Recipient email address"
                        },
                        "subject": {
                            "type": "string",
                            "description": "Email subject"
                        },
                        "body": {
                            "type": "string",
                            "description": "Email body"
                        }
                    },
                    "required": ["to", "subject", "body"]
                }
            }),
            json!({
                "name": "email_move",
                "description": "Move email to another folder",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "email_id": {
                            "type": "string",
                            "description": "The email ID"
                        },
                        "destination_folder": {
                            "type": "string",
                            "description": "Destination folder name"
                        }
                    },
                    "required": ["email_id", "destination_folder"]
                }
            }),
            json!({
                "name": "email_delete",
                "description": "Delete an email",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "email_id": {
                            "type": "string",
                            "description": "The email ID"
                        },
                        "skip_trash": {
                            "type": "boolean",
                            "description": "If true, skip Deleted Items and perform soft delete (default: false)"
                        }
                    },
                    "required": ["email_id"]
                }
            }),
            json!({
                "name": "email_sync_now",
                "description": "Force sync all folders now",
                "parameters": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            }),
            json!({
                "name": "email_add_folder",
                "description": "Add a folder to sync list",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "folder_name": {
                            "type": "string",
                            "description": "Folder name to add"
                        }
                    },
                    "required": ["folder_name"]
                }
            }),
            json!({
                "name": "email_health",
                "description": "Check EWS connectivity and cache health",
                "parameters": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            }),
        ]
    }
}
