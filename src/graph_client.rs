use crate::cache::CachedEmail;
use crate::graph_auth::{get_access_token, GraphAuthConfig};
use chrono::{DateTime, Utc};
use reqwest::blocking::Client;
use serde::Deserialize;

#[derive(Clone)]
pub struct GraphClient {
    auth: GraphAuthConfig,
    http: Client,
}

#[derive(Debug, Clone)]
pub struct GraphFolder {
    pub id: String,
    pub display_name: String,
    pub unread_count: i32,
    pub total_count: i32,
}

#[derive(Debug, Deserialize)]
struct GraphList<T> {
    value: Vec<T>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphFolderItem {
    id: String,
    display_name: String,
    unread_item_count: i32,
    total_item_count: i32,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct GraphAddress {
    address: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct GraphRecipient {
    email_address: Option<GraphAddress>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphMessage {
    id: String,
    subject: Option<String>,
    is_read: Option<bool>,
    has_attachments: Option<bool>,
    importance: Option<String>,
    received_date_time: Option<String>,
    sent_date_time: Option<String>,
    body_preview: Option<String>,
    body: Option<GraphItemBody>,
    from: Option<GraphRecipient>,
    to_recipients: Option<Vec<GraphRecipient>>,
    cc_recipients: Option<Vec<GraphRecipient>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphItemBody {
    content_type: Option<String>,
    content: Option<String>,
}

impl GraphClient {
    pub fn new(auth: GraphAuthConfig) -> Self {
        Self {
            auth,
            http: Client::new(),
        }
    }

    pub fn list_folders(&self) -> Result<Vec<GraphFolder>, String> {
        let response: GraphList<GraphFolderItem> = self
            .request(
                "GET",
                "https://graph.microsoft.com/v1.0/me/mailFolders?$top=100",
            )?
            .json()
            .map_err(|e| e.to_string())?;

        Ok(response
            .value
            .into_iter()
            .map(|f| GraphFolder {
                id: f.id,
                display_name: f.display_name,
                unread_count: f.unread_item_count,
                total_count: f.total_item_count,
            })
            .collect())
    }

    pub fn list_emails(
        &self,
        folder_name: &str,
        limit: i32,
        unread_only: bool,
    ) -> Result<Vec<CachedEmail>, String> {
        let top = limit.clamp(1, 200);
        let mut url = format!(
            "https://graph.microsoft.com/v1.0/me/mailFolders/{}/messages?$top={}&$orderby=receivedDateTime%20desc",
            folder_name, top
        );
        if unread_only {
            url.push_str("&$filter=isRead%20eq%20false");
        }

        let response: GraphList<GraphMessage> = self
            .request("GET", &url)?
            .json()
            .map_err(|e| e.to_string())?;
        Ok(response
            .value
            .into_iter()
            .map(|m| self.message_to_cached_email(m, folder_name))
            .collect())
    }

    pub fn read_email(&self, id: &str) -> Result<CachedEmail, String> {
        let url = format!(
            "https://graph.microsoft.com/v1.0/me/messages/{}?$select=id,subject,from,toRecipients,ccRecipients,receivedDateTime,sentDateTime,isRead,hasAttachments,importance,body,bodyPreview",
            id
        );
        let msg: GraphMessage = self
            .request("GET", &url)?
            .json()
            .map_err(|e| e.to_string())?;
        Ok(self.message_to_cached_email(msg, "inbox"))
    }

    pub fn search_emails(&self, query: &str, limit: i32) -> Result<Vec<CachedEmail>, String> {
        let top = limit.clamp(1, 200);
        let escaped = query.replace('"', "");
        let url = format!(
            "https://graph.microsoft.com/v1.0/me/messages?$top={}&$search=\"{}\"",
            top, escaped
        );

        let response: GraphList<GraphMessage> = self
            .request_with_header("GET", &url, Some(("ConsistencyLevel", "eventual")))?
            .json()
            .map_err(|e| e.to_string())?;

        Ok(response
            .value
            .into_iter()
            .map(|m| self.message_to_cached_email(m, "inbox"))
            .collect())
    }

    fn request(&self, method: &str, url: &str) -> Result<reqwest::blocking::Response, String> {
        self.request_with_header(method, url, None)
    }

    fn request_with_header(
        &self,
        method: &str,
        url: &str,
        header: Option<(&str, &str)>,
    ) -> Result<reqwest::blocking::Response, String> {
        let token = get_access_token(&self.auth)
            .map_err(|e| format!("graph auth required: {}. run `ews_skillctl login`", e))?;

        let mut req = self
            .http
            .request(
                reqwest::Method::from_bytes(method.as_bytes()).map_err(|e| e.to_string())?,
                url,
            )
            .bearer_auth(token);

        if let Some((k, v)) = header {
            req = req.header(k, v);
        }

        req.send()
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())
    }

    fn message_to_cached_email(&self, m: GraphMessage, folder_id: &str) -> CachedEmail {
        let sender = m
            .from
            .as_ref()
            .and_then(|f| f.email_address.as_ref())
            .cloned();

        let (body_text, body_html) = match m.body.as_ref().and_then(|b| b.content.as_ref()) {
            Some(content) => {
                let html = m
                    .body
                    .as_ref()
                    .and_then(|b| b.content_type.as_deref())
                    .map(|ct| ct.eq_ignore_ascii_case("html"))
                    .unwrap_or(false);
                if html {
                    (
                        m.body_preview.clone().unwrap_or_default(),
                        Some(content.clone()),
                    )
                } else {
                    (content.clone(), None)
                }
            }
            None => (m.body_preview.clone().unwrap_or_default(), None),
        };

        CachedEmail {
            id: m.id,
            change_key: None,
            folder_id: folder_id.to_string(),
            subject: m.subject.unwrap_or_default(),
            sender_name: sender
                .as_ref()
                .and_then(|s| s.name.clone())
                .unwrap_or_default(),
            sender_email: sender
                .as_ref()
                .and_then(|s| s.address.clone())
                .unwrap_or_default(),
            to_recipients: recipients_to_vec(m.to_recipients),
            cc_recipients: recipients_to_vec(m.cc_recipients),
            body_text,
            body_html,
            has_attachments: m.has_attachments.unwrap_or(false),
            is_read: m.is_read.unwrap_or(false),
            importance: m.importance.unwrap_or_else(|| "normal".to_string()),
            datetime_received: parse_graph_datetime(m.received_date_time.as_deref()),
            datetime_sent: parse_graph_datetime(m.sent_date_time.as_deref()),
            cached_at: Utc::now(),
        }
    }
}

fn recipients_to_vec(values: Option<Vec<GraphRecipient>>) -> Vec<String> {
    values
        .unwrap_or_default()
        .into_iter()
        .filter_map(|r| r.email_address.and_then(|a| a.address))
        .collect()
}

fn parse_graph_datetime(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .map(|v| v.with_timezone(&Utc))
}
