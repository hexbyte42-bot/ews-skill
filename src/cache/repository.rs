use crate::cache::db::Database;
use crate::cache::models::{CachedEmail, CachedFolder, SyncState};
use chrono::Utc;
use rusqlite::{params, params_from_iter, types::Value as SqlValue, Row};
use std::collections::HashSet;
use tracing::{debug, error};

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

        for candidate in email_id_candidates(email_id) {
            if let Ok(email) = conn.query_row(
                "SELECT id, change_key, folder_id, subject, sender_name, sender_email, to_recipients, cc_recipients, body_text, body_html, has_attachments, is_read, importance, datetime_received, datetime_sent, cached_at FROM emails WHERE id = ?1",
                params![candidate],
                Self::row_to_email,
            ) {
                if candidate != email_id {
                    debug!(requested_id = email_id, matched_id = candidate, "resolved email id variant from cache");
                }
                return Some(email);
            }
        }

        None
    }

    pub fn save_email(&self, email: &CachedEmail) {
        if email.id.trim().is_empty() {
            error!(
                "skip saving email with empty id in folder {}",
                email.folder_id
            );
            return;
        }

        let mut email_to_save = email.clone();
        let incoming_body_empty = email_to_save.body_text.trim().is_empty()
            && email_to_save
                .body_html
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty();

        if incoming_body_empty {
            if let Some(existing) = self.get_email(&email_to_save.id) {
                let existing_has_body = !existing.body_text.trim().is_empty()
                    || !existing
                        .body_html
                        .as_deref()
                        .unwrap_or_default()
                        .trim()
                        .is_empty();
                if existing_has_body {
                    debug!(
                        email_id = %email_to_save.id,
                        subject = %email_to_save.subject,
                        "preserving existing non-empty cached body"
                    );
                    email_to_save.body_text = existing.body_text;
                    email_to_save.body_html = existing.body_html;
                }
            }
        }

        let conn = self.db.connection();
        let conn = conn.lock();

        conn.execute(
            r#"INSERT OR REPLACE INTO emails (id, change_key, folder_id, subject, sender_name, sender_email, to_recipients, cc_recipients, body_text, body_html, has_attachments, is_read, importance, datetime_received, datetime_sent, cached_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)"#,
            params![
                email_to_save.id,
                email_to_save.change_key,
                email_to_save.folder_id,
                email_to_save.subject,
                email_to_save.sender_name,
                email_to_save.sender_email,
                serde_json::to_string(&email_to_save.to_recipients).unwrap_or_default(),
                serde_json::to_string(&email_to_save.cc_recipients).unwrap_or_default(),
                email_to_save.body_text,
                email_to_save.body_html,
                email_to_save.has_attachments as i32,
                email_to_save.is_read as i32,
                email_to_save.importance,
                email_to_save.datetime_received.map(|d| d.to_rfc3339()),
                email_to_save.datetime_sent.map(|d| d.to_rfc3339()),
                email_to_save.cached_at.to_rfc3339(),
            ],
        ).ok();
    }

    pub fn delete_email(&self, email_id: &str) {
        let conn = self.db.connection();
        let conn = conn.lock();

        for candidate in email_id_candidates(email_id) {
            conn.execute("DELETE FROM emails WHERE id = ?1", params![candidate])
                .ok();
        }
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

    #[allow(clippy::too_many_arguments)]
    pub fn search_emails(
        &self,
        query: Option<&str>,
        subject: Option<&str>,
        sender: Option<&str>,
        date_from: Option<&str>,
        date_to: Option<&str>,
        folder_id: Option<&str>,
        limit: i32,
        include_body: bool,
    ) -> Vec<CachedEmail> {
        let conn = self.db.connection();
        let conn = conn.lock();

        let mut sql = String::from(
            "SELECT id, change_key, folder_id, subject, sender_name, sender_email, to_recipients, cc_recipients, body_text, body_html, has_attachments, is_read, importance, datetime_received, datetime_sent, cached_at FROM emails WHERE 1=1",
        );
        let mut values: Vec<SqlValue> = Vec::new();

        if let Some(folder_id) = folder_id {
            sql.push_str(" AND folder_id = ?");
            values.push(SqlValue::from(folder_id.to_string()));
        }

        if let Some(subject) = subject.filter(|v| !v.trim().is_empty()) {
            sql.push_str(" AND LOWER(subject) LIKE LOWER(?)");
            values.push(SqlValue::from(format!("%{}%", subject)));
        }

        if let Some(sender) = sender.filter(|v| !v.trim().is_empty()) {
            sql.push_str(
                " AND (LOWER(sender_email) LIKE LOWER(?) OR LOWER(sender_name) LIKE LOWER(?))",
            );
            let pattern = format!("%{}%", sender);
            values.push(SqlValue::from(pattern.clone()));
            values.push(SqlValue::from(pattern));
        }

        if let Some(query) = query.filter(|v| !v.trim().is_empty()) {
            if include_body {
                sql.push_str(" AND (LOWER(subject) LIKE LOWER(?) OR LOWER(sender_email) LIKE LOWER(?) OR LOWER(sender_name) LIKE LOWER(?) OR LOWER(body_text) LIKE LOWER(?))");
                let pattern = format!("%{}%", query);
                values.push(SqlValue::from(pattern.clone()));
                values.push(SqlValue::from(pattern.clone()));
                values.push(SqlValue::from(pattern.clone()));
                values.push(SqlValue::from(pattern));
            } else {
                sql.push_str(" AND (LOWER(subject) LIKE LOWER(?) OR LOWER(sender_email) LIKE LOWER(?) OR LOWER(sender_name) LIKE LOWER(?))");
                let pattern = format!("%{}%", query);
                values.push(SqlValue::from(pattern.clone()));
                values.push(SqlValue::from(pattern.clone()));
                values.push(SqlValue::from(pattern));
            }
        }

        if let Some(date_from) = date_from.filter(|v| !v.trim().is_empty()) {
            sql.push_str(" AND datetime_received IS NOT NULL AND datetime_received >= ?");
            values.push(SqlValue::from(date_from.to_string()));
        }

        if let Some(date_to) = date_to.filter(|v| !v.trim().is_empty()) {
            sql.push_str(" AND datetime_received IS NOT NULL AND datetime_received <= ?");
            values.push(SqlValue::from(date_to.to_string()));
        }

        sql.push_str(" ORDER BY datetime_received DESC LIMIT ?");
        values.push(SqlValue::from(limit.max(1)));

        let mut stmt = match conn.prepare(&sql) {
            Ok(stmt) => stmt,
            Err(e) => {
                error!("failed to prepare search_emails query: {}", e);
                return Vec::new();
            }
        };

        let out = match stmt.query_map(params_from_iter(values), Self::row_to_email) {
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

        for candidate in email_id_candidates(email_id) {
            conn.execute(
                "UPDATE emails SET is_read = ?1 WHERE id = ?2",
                params![is_read as i32, candidate],
            )
            .ok();
        }
    }

    pub fn move_email(&self, email_id: &str, new_folder_id: &str) {
        let conn = self.db.connection();
        let conn = conn.lock();

        for candidate in email_id_candidates(email_id) {
            conn.execute(
                "UPDATE emails SET folder_id = ?1 WHERE id = ?2",
                params![new_folder_id, candidate],
            )
            .ok();
        }
    }

    pub fn replace_folder_snapshot(&self, folder_id: &str, emails: &[CachedEmail]) {
        let conn = self.db.connection();
        let conn = conn.lock();

        if let Err(e) = conn.execute(
            "DELETE FROM emails WHERE folder_id = ?1",
            params![folder_id],
        ) {
            error!("failed to clear folder snapshot {}: {}", folder_id, e);
            return;
        }
        drop(conn);

        for email in emails {
            self.save_email(email);
        }
    }

    pub fn prune_folder_before(&self, folder_id: &str, cutoff_rfc3339: &str) {
        let conn = self.db.connection();
        let conn = conn.lock();
        conn.execute(
            "DELETE FROM emails WHERE folder_id = ?1 AND datetime_received IS NOT NULL AND datetime_received < ?2",
            params![folder_id, cutoff_rfc3339],
        )
        .ok();
    }

    pub fn remove_folder_rows_not_in(&self, folder_id: &str, keep_ids: &HashSet<String>) {
        let conn = self.db.connection();
        let conn = conn.lock();

        let mut stmt = match conn.prepare("SELECT id FROM emails WHERE folder_id = ?1") {
            Ok(stmt) => stmt,
            Err(e) => {
                error!("failed to prepare remove_folder_rows_not_in query: {}", e);
                return;
            }
        };

        let existing: Vec<String> = match stmt.query_map(params![folder_id], |row| row.get(0)) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(e) => {
                error!("failed to list folder rows for reconciliation: {}", e);
                return;
            }
        };
        drop(stmt);

        for id in existing {
            if !keep_ids.contains(&id) {
                conn.execute("DELETE FROM emails WHERE id = ?1", params![id])
                    .ok();
            }
        }
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

fn email_id_candidates(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    push_unique(&mut out, raw.to_string());

    let trimmed = raw.trim();
    push_unique(&mut out, trimmed.to_string());

    let unwrapped = trim_wrappers(trimmed);
    push_unique(&mut out, unwrapped.to_string());

    if trimmed.contains(' ') {
        let plus_normalized = trimmed.replace(' ', "+");
        push_unique(&mut out, plus_normalized.clone());
        push_unique(&mut out, trim_wrappers(&plus_normalized).to_string());
    }

    out
}

fn trim_wrappers(value: &str) -> &str {
    if value.len() < 2 {
        return value;
    }

    let mut chars = value.chars();
    let first = match chars.next() {
        Some(ch) => ch,
        None => return value,
    };
    let last = match value.chars().last() {
        Some(ch) => ch,
        None => return value,
    };

    let wrapped = matches!(
        (first, last),
        ('"', '"') | ('\'', '\'') | ('`', '`') | ('<', '>') | ('(', ')') | ('[', ']')
    );

    if wrapped {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

fn push_unique(out: &mut Vec<String>, value: String) {
    if !value.is_empty() && !out.iter().any(|v| v == &value) {
        out.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::email_id_candidates;

    #[test]
    fn email_id_candidates_include_trimmed_and_unwrapped() {
        let ids = email_id_candidates("  \"AAMkABC==\"  ");
        assert!(ids.iter().any(|v| v == "\"AAMkABC==\""));
        assert!(ids.iter().any(|v| v == "AAMkABC=="));
    }

    #[test]
    fn email_id_candidates_handle_plus_as_space() {
        let ids = email_id_candidates("AAMkA C+DE=");
        assert!(ids.iter().any(|v| v == "AAMkA+C+DE="));
    }
}
