use crate::cache::db::Database;
use crate::cache::models::{CachedEmail, CachedFolder, SyncState};
use chrono::Utc;
use rusqlite::{params, Row};
use tracing::error;

pub struct Repository {
    db: Database,
}

impl Repository {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn get_folder(&self, folder_id: &str) -> Option<CachedFolder> {
        let conn = self.db.connection();
        let conn = conn.lock();

        conn.query_row(
            "SELECT id, change_key, parent_id, display_name, unread_count, total_count, synced_at FROM folders WHERE id = ?1",
            params![folder_id],
            |row| Self::row_to_folder(row),
        ).ok()
    }

    pub fn get_folder_by_name(&self, name: &str) -> Option<CachedFolder> {
        let conn = self.db.connection();
        let conn = conn.lock();

        conn.query_row(
            "SELECT id, change_key, parent_id, display_name, unread_count, total_count, synced_at FROM folders WHERE display_name = ?1",
            params![name],
            |row| Self::row_to_folder(row),
        ).ok()
    }

    pub fn save_folder(&self, folder: &CachedFolder) {
        if folder.id.trim().is_empty() {
            error!("skip saving folder with empty id: {}", folder.display_name);
            return;
        }

        let conn = self.db.connection();
        let conn = conn.lock();

        conn.execute(
            r#"INSERT OR REPLACE INTO folders (id, change_key, parent_id, display_name, unread_count, total_count, synced_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            params![
                folder.id,
                folder.change_key,
                folder.parent_id,
                folder.display_name,
                folder.unread_count,
                folder.total_count,
                folder.synced_at.to_rfc3339(),
            ],
        ).ok();
    }

    pub fn list_folders(&self) -> Vec<CachedFolder> {
        let conn = self.db.connection();
        let conn = conn.lock();

        let mut stmt = match conn.prepare(
            "SELECT id, change_key, parent_id, display_name, unread_count, total_count, synced_at FROM folders ORDER BY display_name"
        ) {
            Ok(stmt) => stmt,
            Err(e) => {
                error!("failed to prepare list_folders query: {}", e);
                return Vec::new();
            }
        };

        let out = match stmt.query_map([], Self::row_to_folder) {
            Ok(rows) => rows
                .filter_map(|r| r.ok())
                .filter(|f| !f.id.trim().is_empty())
                .collect(),
            Err(e) => {
                error!("failed to list folders: {}", e);
                Vec::new()
            }
        };

        out
    }

    pub fn get_email(&self, email_id: &str) -> Option<CachedEmail> {
        let conn = self.db.connection();
        let conn = conn.lock();

        conn.query_row(
            "SELECT id, change_key, folder_id, subject, sender_name, sender_email, to_recipients, cc_recipients, body_text, body_html, has_attachments, is_read, importance, datetime_received, datetime_sent, cached_at FROM emails WHERE id = ?1",
            params![email_id],
            |row| Self::row_to_email(row),
        ).ok()
    }

    pub fn save_email(&self, email: &CachedEmail) {
        if email.id.trim().is_empty() {
            error!(
                "skip saving email with empty id in folder {}",
                email.folder_id
            );
            return;
        }

        let conn = self.db.connection();
        let conn = conn.lock();

        conn.execute(
            r#"INSERT OR REPLACE INTO emails (id, change_key, folder_id, subject, sender_name, sender_email, to_recipients, cc_recipients, body_text, body_html, has_attachments, is_read, importance, datetime_received, datetime_sent, cached_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)"#,
            params![
                email.id,
                email.change_key,
                email.folder_id,
                email.subject,
                email.sender_name,
                email.sender_email,
                serde_json::to_string(&email.to_recipients).unwrap_or_default(),
                serde_json::to_string(&email.cc_recipients).unwrap_or_default(),
                email.body_text,
                email.body_html,
                email.has_attachments as i32,
                email.is_read as i32,
                email.importance,
                email.datetime_received.map(|d| d.to_rfc3339()),
                email.datetime_sent.map(|d| d.to_rfc3339()),
                email.cached_at.to_rfc3339(),
            ],
        ).ok();
    }

    pub fn delete_email(&self, email_id: &str) {
        let conn = self.db.connection();
        let conn = conn.lock();

        conn.execute("DELETE FROM emails WHERE id = ?1", params![email_id])
            .ok();
    }

    pub fn list_emails(&self, folder_id: &str, limit: i32, unread_only: bool) -> Vec<CachedEmail> {
        let conn = self.db.connection();
        let conn = conn.lock();

        let query = if unread_only {
            "SELECT id, change_key, folder_id, subject, sender_name, sender_email, to_recipients, cc_recipients, body_text, body_html, has_attachments, is_read, importance, datetime_received, datetime_sent, cached_at FROM emails WHERE folder_id = ?1 AND is_read = 0 ORDER BY datetime_received DESC LIMIT ?2"
        } else {
            "SELECT id, change_key, folder_id, subject, sender_name, sender_email, to_recipients, cc_recipients, body_text, body_html, has_attachments, is_read, importance, datetime_received, datetime_sent, cached_at FROM emails WHERE folder_id = ?1 ORDER BY datetime_received DESC LIMIT ?2"
        };

        let mut stmt = match conn.prepare(query) {
            Ok(stmt) => stmt,
            Err(e) => {
                error!("failed to prepare list_emails query: {}", e);
                return Vec::new();
            }
        };

        let out = match stmt.query_map(params![folder_id, limit], Self::row_to_email) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(e) => {
                error!("failed to list emails: {}", e);
                Vec::new()
            }
        };

        out
    }

    pub fn search_emails(&self, query: &str, limit: i32) -> Vec<CachedEmail> {
        let conn = self.db.connection();
        let conn = conn.lock();

        let search_pattern = format!("%{}%", query);

        let mut stmt = match conn.prepare(
            "SELECT id, change_key, folder_id, subject, sender_name, sender_email, to_recipients, cc_recipients, body_text, body_html, has_attachments, is_read, importance, datetime_received, datetime_sent, cached_at FROM emails WHERE subject LIKE ?1 OR body_text LIKE ?1 OR sender_email LIKE ?1 ORDER BY datetime_received DESC LIMIT ?2"
        ) {
            Ok(stmt) => stmt,
            Err(e) => {
                error!("failed to prepare search_emails query: {}", e);
                return Vec::new();
            }
        };

        let out = match stmt.query_map(params![search_pattern, limit], Self::row_to_email) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(e) => {
                error!("failed to search emails: {}", e);
                Vec::new()
            }
        };

        out
    }

    pub fn get_unread_count(&self, folder_id: &str) -> i32 {
        let conn = self.db.connection();
        let conn = conn.lock();

        conn.query_row(
            "SELECT COUNT(*) FROM emails WHERE folder_id = ?1 AND is_read = 0",
            params![folder_id],
            |row| row.get(0),
        )
        .unwrap_or(0)
    }

    pub fn mark_read(&self, email_id: &str, is_read: bool) {
        let conn = self.db.connection();
        let conn = conn.lock();

        conn.execute(
            "UPDATE emails SET is_read = ?1 WHERE id = ?2",
            params![is_read as i32, email_id],
        )
        .ok();
    }

    pub fn move_email(&self, email_id: &str, new_folder_id: &str) {
        let conn = self.db.connection();
        let conn = conn.lock();

        conn.execute(
            "UPDATE emails SET folder_id = ?1 WHERE id = ?2",
            params![new_folder_id, email_id],
        )
        .ok();
    }

    pub fn get_sync_state(&self, folder_id: &str) -> Option<SyncState> {
        let conn = self.db.connection();
        let conn = conn.lock();

        conn.query_row(
            "SELECT folder_id, sync_state, last_sync_at FROM sync_states WHERE folder_id = ?1",
            params![folder_id],
            |row| Self::row_to_sync_state(row),
        )
        .ok()
    }

    pub fn save_sync_state(&self, state: &SyncState) {
        let conn = self.db.connection();
        let conn = conn.lock();

        conn.execute(
            r#"INSERT OR REPLACE INTO sync_states (folder_id, sync_state, last_sync_at)
               VALUES (?1, ?2, ?3)"#,
            params![
                state.folder_id,
                state.sync_state,
                state.last_sync_at.to_rfc3339(),
            ],
        )
        .ok();
    }

    pub fn get_synced_folders(&self) -> Vec<String> {
        let conn = self.db.connection();
        let conn = conn.lock();

        let mut stmt = match conn.prepare("SELECT DISTINCT folder_id FROM sync_states") {
            Ok(stmt) => stmt,
            Err(e) => {
                error!("failed to prepare get_synced_folders query: {}", e);
                return Vec::new();
            }
        };

        let out = match stmt.query_map([], |row| row.get(0)) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(e) => {
                error!("failed to get synced folders: {}", e);
                Vec::new()
            }
        };

        out
    }

    pub fn count_emails(&self) -> i64 {
        let conn = self.db.connection();
        let conn = conn.lock();
        conn.query_row("SELECT COUNT(*) FROM emails", [], |row| row.get(0))
            .unwrap_or(0)
    }

    pub fn count_folders(&self) -> i64 {
        let conn = self.db.connection();
        let conn = conn.lock();
        conn.query_row("SELECT COUNT(*) FROM folders", [], |row| row.get(0))
            .unwrap_or(0)
    }

    fn row_to_folder(row: &Row) -> rusqlite::Result<CachedFolder> {
        Ok(CachedFolder {
            id: row.get(0)?,
            change_key: row.get(1)?,
            parent_id: row.get(2)?,
            display_name: row.get(3)?,
            unread_count: row.get(4)?,
            total_count: row.get(5)?,
            synced_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(6)?)
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }

    fn row_to_email(row: &Row) -> rusqlite::Result<CachedEmail> {
        let to_recipients: String = row.get(6)?;
        let cc_recipients: String = row.get(7)?;

        Ok(CachedEmail {
            id: row.get(0)?,
            change_key: row.get(1)?,
            folder_id: row.get(2)?,
            subject: row.get(3)?,
            sender_name: row.get(4)?,
            sender_email: row.get(5)?,
            to_recipients: serde_json::from_str(&to_recipients).unwrap_or_default(),
            cc_recipients: serde_json::from_str(&cc_recipients).unwrap_or_default(),
            body_text: row.get(8)?,
            body_html: row.get(9)?,
            has_attachments: row.get::<_, i32>(10)? != 0,
            is_read: row.get::<_, i32>(11)? != 0,
            importance: row.get(12)?,
            datetime_received: row
                .get::<_, Option<String>>(13)?
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| d.with_timezone(&Utc)),
            datetime_sent: row
                .get::<_, Option<String>>(14)?
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| d.with_timezone(&Utc)),
            cached_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(15)?)
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }

    fn row_to_sync_state(row: &Row) -> rusqlite::Result<SyncState> {
        Ok(SyncState {
            folder_id: row.get(0)?,
            sync_state: row.get(1)?,
            last_sync_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(2)?)
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }
}

impl Clone for Repository {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
        }
    }
}
