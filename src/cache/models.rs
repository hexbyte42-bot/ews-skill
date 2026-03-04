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
        let (body_text, body_html) = split_email_body(ews_email);

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
            body_text,
            body_html,
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

fn split_email_body(email: &crate::ews_client::Email) -> (String, Option<String>) {
    let body = &email.body;
    let text_body = &email.text_body;

    let value = if body.value.trim().is_empty() {
        text_body.value.trim()
    } else {
        body.value.trim()
    };

    if value.is_empty() {
        return (String::new(), None);
    }

    let body_type = if body.body_type.trim().is_empty() {
        text_body.body_type.trim()
    } else {
        body.body_type.trim()
    };

    if body_type.eq_ignore_ascii_case("html") {
        return (html_to_text(value), Some(value.to_string()));
    }

    (value.to_string(), None)
}

fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let bytes = html.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'<' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b'>' {
                j += 1;
            }

            if j >= bytes.len() {
                break;
            }

            let raw_tag = &html[i + 1..j];
            let tag = raw_tag.trim().to_ascii_lowercase();
            if tag.starts_with("br")
                || tag.starts_with("/p")
                || tag.starts_with("/div")
                || tag.starts_with("/li")
                || tag.starts_with("/tr")
                || tag.starts_with("/h")
            {
                out.push('\n');
            }

            i = j + 1;
            continue;
        }

        if bytes[i] == b'&' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b';' && (j - i) <= 12 {
                j += 1;
            }

            if j < bytes.len() && bytes[j] == b';' {
                let entity = &html[i..=j];
                if let Some(decoded) = decode_html_entity(entity) {
                    out.push(decoded);
                    i = j + 1;
                    continue;
                }
            }
        }

        if let Some(ch) = html[i..].chars().next() {
            out.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
    }

    normalize_whitespace(&out)
}

fn decode_html_entity(entity: &str) -> Option<char> {
    match entity {
        "&amp;" => Some('&'),
        "&lt;" => Some('<'),
        "&gt;" => Some('>'),
        "&quot;" => Some('"'),
        "&apos;" | "&#39;" => Some('\''),
        "&nbsp;" => Some(' '),
        _ if entity.starts_with("&#x") && entity.ends_with(';') => {
            u32::from_str_radix(&entity[3..entity.len() - 1], 16)
                .ok()
                .and_then(char::from_u32)
        }
        _ if entity.starts_with("&#") && entity.ends_with(';') => entity[2..entity.len() - 1]
            .parse::<u32>()
            .ok()
            .and_then(char::from_u32),
        _ => None,
    }
}

fn normalize_whitespace(s: &str) -> String {
    let mut out = String::new();
    let mut prev_space = false;
    let mut prev_newline = false;

    for ch in s.chars() {
        if ch == '\n' {
            if !prev_newline {
                out.push('\n');
            }
            prev_newline = true;
            prev_space = false;
            continue;
        }

        if ch.is_whitespace() {
            if !prev_space && !prev_newline {
                out.push(' ');
                prev_space = true;
            }
            continue;
        }

        out.push(ch);
        prev_space = false;
        prev_newline = false;
    }

    out.trim().to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncState {
    pub folder_id: String,
    pub sync_state: String,
    pub last_sync_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::{html_to_text, split_email_body};

    #[test]
    fn split_html_body_into_html_and_plaintext() {
        let email = crate::ews_client::Email {
            body: crate::ews_client::BodyType {
                body_type: "HTML".to_string(),
                value: "<p>Hello&nbsp;<b>World</b></p><div>Line 2</div>".to_string(),
            },
            ..Default::default()
        };

        let (text, html) = split_email_body(&email);
        assert_eq!(
            html.as_deref(),
            Some("<p>Hello&nbsp;<b>World</b></p><div>Line 2</div>")
        );
        assert_eq!(text, "Hello World\nLine 2");
    }

    #[test]
    fn split_email_body_falls_back_to_text_body() {
        let email = crate::ews_client::Email {
            text_body: crate::ews_client::BodyType {
                body_type: "Text".to_string(),
                value: "fallback text".to_string(),
            },
            ..Default::default()
        };

        let (text, html) = split_email_body(&email);
        assert_eq!(text, "fallback text");
        assert_eq!(html, None);
    }

    #[test]
    fn html_to_text_decodes_entities() {
        assert_eq!(html_to_text("A &amp; B &lt; C"), "A & B < C");
    }
}
