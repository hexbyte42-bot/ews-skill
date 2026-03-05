use crate::cache::{CachedEmail, CachedFolder, Repository};
use crate::sync_engine::SyncEngine;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::error;

pub struct EmailService {
    sync_engine: Arc<SyncEngine>,
    repository: Repository,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HealthStatus {
    pub ews_url: String,
    pub auth_ok: bool,
    pub inbox_found: bool,
    pub cached_folders: i64,
    pub cached_emails: i64,
    pub synced_folders: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EmailListOptions {
    pub folder_id: Option<String>,
    pub folder_name: Option<String>,
    pub limit: Option<i32>,
    pub unread_only: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EmailSearchOptions {
    pub query: Option<String>,
    pub subject: Option<String>,
    pub sender: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub folder_name: Option<String>,
    pub limit: Option<i32>,
    pub include_body: Option<bool>,
}

impl EmailService {
    pub fn new(sync_engine: SyncEngine, repository: Repository) -> Self {
        Self {
            sync_engine: Arc::new(sync_engine),
            repository,
        }
    }

    pub fn list_folders(&self) -> Vec<CachedFolder> {
        self.repository.list_folders()
    }

    pub fn get_folder(&self, folder_name: &str) -> Option<CachedFolder> {
        self.repository.get_folder_by_name(folder_name)
    }

    pub fn list_emails(&self, options: EmailListOptions) -> Vec<CachedEmail> {
        let folder_id = if let Some(name) = options.folder_name {
            self.repository.get_folder_by_name(&name).map(|f| f.id)
        } else {
            options.folder_id
        };

        if let Some(folder_id) = folder_id {
            self.repository.list_emails(
                &folder_id,
                options.limit.unwrap_or(50),
                options.unread_only.unwrap_or(false),
            )
        } else {
            Vec::new()
        }
    }

    pub fn get_email(&self, email_id: &str) -> Option<CachedEmail> {
        self.repository.get_email(email_id)
    }

    pub fn get_unread(&self, folder_name: &str, limit: i32) -> Vec<CachedEmail> {
        if let Some(folder) = self.repository.get_folder_by_name(folder_name) {
            self.repository.list_emails(&folder.id, limit, true)
        } else {
            Vec::new()
        }
    }

    pub fn search(&self, options: EmailSearchOptions) -> Result<Vec<CachedEmail>, String> {
        let has_filter = options.query.as_ref().is_some_and(|v| !v.trim().is_empty())
            || options
                .subject
                .as_ref()
                .is_some_and(|v| !v.trim().is_empty())
            || options
                .sender
                .as_ref()
                .is_some_and(|v| !v.trim().is_empty())
            || options
                .date_from
                .as_ref()
                .is_some_and(|v| !v.trim().is_empty())
            || options
                .date_to
                .as_ref()
                .is_some_and(|v| !v.trim().is_empty())
            || options
                .folder_name
                .as_ref()
                .is_some_and(|v| !v.trim().is_empty());

        if !has_filter {
            return Err(
                "at least one search filter is required (query, subject, sender, date_from, date_to, folder_name)".to_string(),
            );
        }

        let folder_id = if let Some(folder_name) = options.folder_name.as_ref() {
            Some(
                self.repository
                    .get_folder_by_name(folder_name)
                    .ok_or_else(|| format!("Folder not found: {}", folder_name))?
                    .id,
            )
        } else {
            None
        };

        Ok(self.repository.search_emails(
            options.query.as_deref(),
            options.subject.as_deref(),
            options.sender.as_deref(),
            options.date_from.as_deref(),
            options.date_to.as_deref(),
            folder_id.as_deref(),
            options.limit.unwrap_or(50).clamp(1, 200),
            options.include_body.unwrap_or(true),
        ))
    }

    pub async fn send_email(&self, to: &str, subject: &str, body: &str) -> Result<String, String> {
        self.sync_engine
            .get_client()
            .send_email(to, subject, body)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn mark_read(&self, email_id: &str, is_read: bool) -> Result<(), String> {
        let resolved_email_id = self
            .repository
            .get_email(email_id)
            .map(|e| e.id)
            .unwrap_or_else(|| email_id.to_string());

        self.repository.mark_read(&resolved_email_id, is_read);

        self.sync_engine
            .get_client()
            .mark_read(&resolved_email_id, is_read)
            .await
            .map_err(|e| {
                self.repository.mark_read(&resolved_email_id, !is_read);
                e.to_string()
            })
    }

    pub async fn move_email(&self, email_id: &str, destination_folder: &str) -> Result<(), String> {
        let folder = self
            .repository
            .get_folder_by_name(destination_folder)
            .ok_or_else(|| format!("Folder not found: {}", destination_folder))?;

        let email = self
            .repository
            .get_email(email_id)
            .ok_or_else(|| format!("Email not found: {}", email_id))?;
        let resolved_email_id = email.id;
        let old_folder = email.folder_id;

        self.repository.move_email(&resolved_email_id, &folder.id);

        match self
            .sync_engine
            .get_client()
            .move_item(&resolved_email_id, &folder.id)
            .await
        {
            Ok(_) => Ok(()),
            Err(e) => {
                self.repository.move_email(&resolved_email_id, &old_folder);
                Err(e.to_string())
            }
        }
    }

    pub async fn delete_email(&self, email_id: &str, skip_trash: bool) -> Result<(), String> {
        let resolved_email_id = self
            .repository
            .get_email(email_id)
            .map(|e| e.id)
            .unwrap_or_else(|| email_id.to_string());

        self.repository.delete_email(&resolved_email_id);

        self.sync_engine
            .get_client()
            .delete_item(&resolved_email_id, skip_trash)
            .await
            .map_err(|e| {
                error!("Failed to delete email from server: {}", e);
                e.to_string()
            })
    }

    pub async fn sync_now(&self) -> Result<(), String> {
        self.sync_engine.sync_all_folders().await
    }

    pub async fn add_folder_to_sync(&self, folder_name: &str) -> Result<(), String> {
        match self.sync_engine.find_and_cache_folder(folder_name).await {
            Ok(Some(folder)) => self.sync_engine.full_sync_folder(&folder.id).await,
            Ok(None) => Err(format!("Folder not found: {}", folder_name)),
            Err(e) => Err(e),
        }
    }

    pub async fn health_check(&self) -> HealthStatus {
        let mut auth_ok = false;
        let mut inbox_found = false;

        if let Ok(folder) = self.sync_engine.find_and_cache_folder("inbox").await {
            auth_ok = true;
            inbox_found = folder.is_some();
        }

        HealthStatus {
            ews_url: self.sync_engine.get_client().ews_url().to_string(),
            auth_ok,
            inbox_found,
            cached_folders: self.repository.count_folders(),
            cached_emails: self.repository.count_emails(),
            synced_folders: self.repository.get_synced_folders().len(),
        }
    }
}

impl Clone for EmailService {
    fn clone(&self) -> Self {
        Self {
            sync_engine: self.sync_engine.clone(),
            repository: self.repository.clone(),
        }
    }
}
