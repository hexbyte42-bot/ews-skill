use parking_lot::Mutex;
use rusqlite::{Connection, Result as SqlResult};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    pub fn new(path: &PathBuf) -> SqlResult<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;

        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };

        db.init_schema()?;
        info!("Database initialized at: {:?}", path);

        Ok(db)
    }

    fn init_schema(&self) -> SqlResult<()> {
        let conn = self.conn.lock();

        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS folders (
                id TEXT PRIMARY KEY,
                change_key TEXT,
                parent_id TEXT,
                display_name TEXT NOT NULL,
                unread_count INTEGER DEFAULT 0,
                total_count INTEGER DEFAULT 0,
                synced_at TEXT DEFAULT CURRENT_TIMESTAMP
            );
            
            CREATE TABLE IF NOT EXISTS emails (
                id TEXT PRIMARY KEY,
                change_key TEXT,
                folder_id TEXT NOT NULL,
                subject TEXT,
                sender_name TEXT,
                sender_email TEXT,
                to_recipients TEXT,
                cc_recipients TEXT,
                body_text TEXT,
                body_html TEXT,
                has_attachments INTEGER DEFAULT 0,
                is_read INTEGER DEFAULT 0,
                importance TEXT DEFAULT 'Normal',
                datetime_received TEXT,
                datetime_sent TEXT,
                cached_at TEXT DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (folder_id) REFERENCES folders(id)
            );
            
            CREATE TABLE IF NOT EXISTS sync_states (
                folder_id TEXT PRIMARY KEY,
                sync_state TEXT NOT NULL,
                last_sync_at TEXT DEFAULT CURRENT_TIMESTAMP
            );
            
            CREATE INDEX IF NOT EXISTS idx_emails_folder ON emails(folder_id);
            CREATE INDEX IF NOT EXISTS idx_emails_date ON emails(datetime_received);
            CREATE INDEX IF NOT EXISTS idx_emails_sender ON emails(sender_email);
            CREATE INDEX IF NOT EXISTS idx_emails_read ON emails(is_read);
            CREATE INDEX IF NOT EXISTS idx_emails_subject ON emails(subject);

            DELETE FROM emails WHERE id = '' OR id IS NULL;
            DELETE FROM folders WHERE id = '' OR id IS NULL;
            DELETE FROM sync_states WHERE folder_id = '' OR folder_id IS NULL;
        "#,
        )?;

        Ok(())
    }

    pub fn connection(&self) -> Arc<Mutex<Connection>> {
        self.conn.clone()
    }
}

impl Clone for Database {
    fn clone(&self) -> Self {
        Self {
            conn: self.conn.clone(),
        }
    }
}
