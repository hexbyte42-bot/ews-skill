use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedFolder {
    pub id: String,
    pub change_key: Option<String>,
    pub parent_id: Option<String>,
    pub display_name: String,
    pub unread_count: i32,
    pub total_count: i32,
    pub synced_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedEmail {
    pub id: String,
    pub change_key: Option<String>,
    pub folder_id: String,
    pub subject: String,
    pub sender_name: String,
    pub sender_email: String,
    pub to_recipients: Vec<String>,
    pub cc_recipients: Vec<String>,
    pub body_text: String,
    pub body_html: Option<String>,
    pub has_attachments: bool,
    pub is_read: bool,
    pub importance: String,
    pub datetime_received: Option<DateTime<Utc>>,
    pub datetime_sent: Option<DateTime<Utc>>,
    pub cached_at: DateTime<Utc>,
}

impl CachedEmail {
    pub fn from_ews_email(ews_email: &crate::ews_client::Email, folder_id: &str) -> Self {
        let datetime_received = parse_ews_datetime(&ews_email.datetime_received);
        let datetime_sent = parse_ews_datetime(&ews_email.datetime_sent);

        Self {
            id: ews_email.item_id.id.clone(),
            change_key: Some(ews_email.item_id.change_key.clone()),
            folder_id: folder_id.to_string(),
            subject: ews_email.subject.clone(),
            sender_name: ews_email
                .sender
                .mailbox
                .as_ref()
                .and_then(|m| m.name.clone())
                .unwrap_or_default(),
            sender_email: ews_email
                .sender
                .mailbox
                .as_ref()
                .map(|m| m.email_address.clone())
                .unwrap_or_default(),
            to_recipients: ews_email
                .to_recipients
                .mailbox
                .iter()
                .map(|r| r.email_address.clone())
                .collect(),
            cc_recipients: ews_email
                .cc_recipients
                .mailbox
                .iter()
                .map(|r| r.email_address.clone())
                .collect(),
            body_text: ews_email.body.value.clone(),
            body_html: None,
            has_attachments: ews_email.has_attachments,
            is_read: ews_email.is_read,
            importance: ews_email.importance.clone(),
            datetime_received,
            datetime_sent,
            cached_at: Utc::now(),
        }
    }
}

fn parse_ews_datetime(s: &str) -> Option<DateTime<Utc>> {
    if s.is_empty() {
        return None;
    }

    let formats = [
        "%Y-%m-%dT%H:%M:%SZ",
        "%Y-%m-%dT%H:%M:%S%.fZ",
        "%Y-%m-%dT%H:%M:%S%z",
    ];

    for fmt in &formats {
        if let Ok(dt) = DateTime::parse_from_str(s, fmt) {
            return Some(dt.with_timezone(&Utc));
        }
    }

    if let Ok(dt) = s.parse::<DateTime<Utc>>() {
        return Some(dt);
    }

    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncState {
    pub folder_id: String,
    pub sync_state: String,
    pub last_sync_at: DateTime<Utc>,
}
