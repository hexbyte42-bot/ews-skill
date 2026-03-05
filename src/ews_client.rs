use base64::{engine::general_purpose, Engine as _};
use curl::easy::{Auth, Easy, List};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use std::cmp::min;
use std::time::Duration;
use thiserror::Error;
use tracing::{error, info, warn};

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    body: String,
    www_authenticate: Option<String>,
}

#[derive(Error, Debug)]
pub enum EwsError {
    #[error("SOAP error: {0}")]
    SoapError(String),
    #[error("HTTP error: {0}")]
    HttpError(String),
    #[error("XML parse error: {0}")]
    XmlError(String),
    #[error("Authentication error: {0}")]
    AuthError(String),
    #[error("Not found: {0}")]
    NotFound(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EwsCredentials {
    pub username: String,
    pub password: String,
    pub email: String,
    pub auth_mode: String,
}

pub struct EwsClient {
    credentials: EwsCredentials,
    ews_url: String,
    http_client: reqwest::Client,
    options: EwsClientOptions,
}

pub fn ntlm_supported() -> bool {
    curl::Version::get().feature_ntlm()
}

#[derive(Debug, Clone)]
pub struct EwsClientOptions {
    pub retry_max_attempts: u32,
    pub retry_base_ms: u64,
    pub retry_max_backoff_ms: u64,
}

impl Default for EwsClientOptions {
    fn default() -> Self {
        Self {
            retry_max_attempts: 5,
            retry_base_ms: 500,
            retry_max_backoff_ms: 10_000,
        }
    }
}

impl Clone for EwsClient {
    fn clone(&self) -> Self {
        Self {
            credentials: self.credentials.clone(),
            ews_url: self.ews_url.clone(),
            http_client: reqwest::Client::new(),
            options: self.options.clone(),
        }
    }
}

impl EwsClient {
    pub fn new(
        credentials: EwsCredentials,
        ews_url: Option<String>,
        options: EwsClientOptions,
    ) -> Self {
        let url = ews_url
            .unwrap_or_else(|| "https://autodiscover.outlook.com/EWS/Exchange.asmx".to_string());

        Self {
            credentials,
            ews_url: url,
            http_client: reqwest::Client::new(),
            options,
        }
    }

    pub async fn discover(&mut self) -> Result<(), EwsError> {
        let email = &self.credentials.email;
        if email.is_empty() {
            return Err(EwsError::AuthError(
                "Email required for autodiscover".to_string(),
            ));
        }

        let domain = email
            .split('@')
            .nth(1)
            .ok_or_else(|| EwsError::AuthError("invalid email address".to_string()))?;

        let autodiscover_body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<Autodiscover xmlns="http://schemas.microsoft.com/exchange/autodiscover/outlook/requestschema/2006">
  <Request>
    <EMailAddress>{}</EMailAddress>
    <AcceptableResponseSchema>http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a</AcceptableResponseSchema>
  </Request>
</Autodiscover>"#,
            escape_xml(email)
        );

        let candidates = [
            format!(
                "https://autodiscover.{}/Autodiscover/Autodiscover.xml",
                domain
            ),
            format!("https://{}/Autodiscover/Autodiscover.xml", domain),
        ];

        let mode = normalized_auth_mode(&self.credentials.auth_mode);
        let auth_header = if mode == "basic" {
            let basic_token = general_purpose::STANDARD.encode(format!(
                "{}:{}",
                self.credentials.username, self.credentials.password
            ));
            Some(authorization_header(&self.credentials, &basic_token)?)
        } else {
            None
        };

        let mut last_error: Option<String> = None;

        for url in candidates {
            info!("Attempting autodiscover at: {}", url);
            let response = self
                .post_xml_with_retry(
                    &url,
                    autodiscover_body.clone(),
                    "application/xml, text/xml",
                    auth_header.clone(),
                )
                .await;

            let response = match response {
                Ok(r) => r,
                Err(e) => {
                    last_error = Some(e.to_string());
                    warn!("autodiscover request failed for {}: {}", url, e);
                    continue;
                }
            };

            if response.status < 200 || response.status >= 300 {
                last_error = Some(format!("HTTP {}", response.status));
                warn!("autodiscover endpoint {} returned {}", url, response.status);
                continue;
            }

            if let Some(found) = extract_ews_url_from_autodiscover(&response.body) {
                self.ews_url = found;
                info!("Discovered EWS URL: {}", self.ews_url);
                return Ok(());
            }

            if let Some(redirect_url) = extract_redirect_url_from_autodiscover(&response.body) {
                let final_url = normalize_ews_url(&redirect_url);
                self.ews_url = final_url;
                info!("Autodiscover redirect URL resolved to: {}", self.ews_url);
                return Ok(());
            }

            last_error = Some("autodiscover response did not contain EwsUrl".to_string());
        }

        Err(EwsError::NotFound(format!(
            "autodiscover failed for domain {}: {}",
            domain,
            last_error.unwrap_or_else(|| "unknown error".to_string())
        )))
    }

    pub fn ews_url(&self) -> &str {
        &self.ews_url
    }

    pub async fn sync_folder_items(
        &self,
        folder_id: &str,
        sync_state: Option<String>,
        max_changes: i32,
    ) -> Result<SyncFolderItemsResponse, EwsError> {
        let safe_folder_id = escape_xml(folder_id);
        let sync_state_xml = sync_state
            .map(|s| format!("<t:SyncState>{}</t:SyncState>", escape_xml(&s)))
            .unwrap_or_default();

        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
            <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"
                          xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
                <soap:Header>
                    <t:RequestServerVersion Version="Exchange2016"/>
                </soap:Header>
                <soap:Body>
                    <SyncFolderItems xmlns="http://schemas.microsoft.com/exchange/services/2006/messages">
                        <ItemShape>
                            <t:BaseShape>AllProperties</t:BaseShape>
                        </ItemShape>
                        <SyncFolderId>
                            <t:FolderId Id="{}"/>
                        </SyncFolderId>
                        {}
                        <MaxChangesReturned>{}</MaxChangesReturned>
                    </SyncFolderItems>
                </soap:Body>
            </soap:Envelope>"#,
            safe_folder_id, sync_state_xml, max_changes
        );

        self.send_request("SyncFolderItems", body).await
    }

    pub async fn get_folder(&self, folder_id: &str) -> Result<Folder, EwsError> {
        let safe_folder_id = escape_xml(folder_id);
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
            <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"
                          xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
                <soap:Header>
                    <t:RequestServerVersion Version="Exchange2016"/>
                </soap:Header>
                <soap:Body>
                    <GetFolder xmlns="http://schemas.microsoft.com/exchange/services/2006/messages">
                        <FolderShape>
                            <t:BaseShape>Default</t:BaseShape>
                        </FolderShape>
                        <FolderIds>
                            <t:FolderId Id="{}"/>
                        </FolderIds>
                    </GetFolder>
                </soap:Body>
            </soap:Envelope>"#,
            safe_folder_id
        );

        let response: GetFolderResponse = self.send_request("GetFolder", body).await?;
        Ok(response.response_messages.get_folder.folders.folder)
    }

    pub async fn get_distinguished_folder(
        &self,
        distinguished_id: &str,
    ) -> Result<Folder, EwsError> {
        let safe_id = escape_xml(distinguished_id);
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
            <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"
                          xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
                <soap:Header>
                    <t:RequestServerVersion Version="Exchange2016"/>
                </soap:Header>
                <soap:Body>
                    <GetFolder xmlns="http://schemas.microsoft.com/exchange/services/2006/messages">
                        <FolderShape>
                            <t:BaseShape>Default</t:BaseShape>
                        </FolderShape>
                        <FolderIds>
                            <t:DistinguishedFolderId Id="{}"/>
                        </FolderIds>
                    </GetFolder>
                </soap:Body>
            </soap:Envelope>"#,
            safe_id
        );

        let response: GetFolderResponse = self.send_request("GetFolder", body).await?;
        Ok(response.response_messages.get_folder.folders.folder)
    }

    pub async fn find_folder(&self, folder_name: &str) -> Result<Option<Folder>, EwsError> {
        if let Some(distinguished_id) = distinguished_folder_id_from_spec(folder_name) {
            return self
                .get_distinguished_folder(distinguished_id)
                .await
                .map(Some);
        }

        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
            <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"
                          xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
                <soap:Header>
                    <t:RequestServerVersion Version="Exchange2016"/>
                </soap:Header>
                <soap:Body>
                    <FindFolder xmlns="http://schemas.microsoft.com/exchange/services/2006/messages" Traversal="Deep">
                        <FolderShape>
                            <t:BaseShape>Default</t:BaseShape>
                        </FolderShape>
                        <ParentFolderIds>
                            <t:DistinguishedFolderId Id="root"/>
                        </ParentFolderIds>
                    </FindFolder>
                </soap:Body>
            </soap:Envelope>"#
        );

        let xml = self.send_request_xml(body).await?;
        Ok(find_folder_in_xml(&xml, folder_name))
    }

    pub async fn get_item(&self, item_id: &str) -> Result<Email, EwsError> {
        let safe_item_id = escape_xml(item_id);
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
            <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"
                          xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
                <soap:Header>
                    <t:RequestServerVersion Version="Exchange2016"/>
                </soap:Header>
                <soap:Body>
                    <GetItem xmlns="http://schemas.microsoft.com/exchange/services/2006/messages">
                        <ItemShape>
                            <t:BaseShape>AllProperties</t:BaseShape>
                            <t:BodyType>Best</t:BodyType>
                            <t:FilterHtmlContent>false</t:FilterHtmlContent>
                            <t:AdditionalProperties>
                                <t:FieldURI FieldURI="item:Body"/>
                                <t:FieldURI FieldURI="item:TextBody"/>
                            </t:AdditionalProperties>
                        </ItemShape>
                        <ItemIds>
                            <t:ItemId Id="{}"/>
                        </ItemIds>
                    </GetItem>
                </soap:Body>
            </soap:Envelope>"#,
            safe_item_id
        );

        let response: GetItemResponse = self.send_request("GetItem", body).await?;
        response
            .response_messages
            .get_item
            .items
            .messages
            .into_iter()
            .next()
            .ok_or_else(|| EwsError::NotFound("item not found in GetItem response".to_string()))
    }

    pub async fn find_item(
        &self,
        folder_id: &str,
        query: &str,
        max_results: i32,
    ) -> Result<Vec<Email>, EwsError> {
        let query_xml = if query.is_empty() {
            String::new()
        } else {
            format!("<t:QueryString>{}</t:QueryString>", escape_xml(query))
        };
        let safe_folder_id = escape_xml(folder_id);

        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
            <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"
                          xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
                <soap:Header>
                    <t:RequestServerVersion Version="Exchange2016"/>
                </soap:Header>
                <soap:Body>
                    <FindItem xmlns="http://schemas.microsoft.com/exchange/services/2006/messages" Traversal="Shallow">
                        <ItemShape>
                            <t:BaseShape>Default</t:BaseShape>
                        </ItemShape>
                        <ParentFolderIds>
                            <t:FolderId Id="{}"/>
                        </ParentFolderIds>
                        {}
                        <MaxItemsReturned>{}</MaxItemsReturned>
                    </FindItem>
                </soap:Body>
            </soap:Envelope>"#,
            safe_folder_id, query_xml, max_results
        );

        let response: FindItemResponse = self.send_request("FindItem", body).await?;
        Ok(response
            .response_messages
            .find_item
            .root_folder
            .items
            .into_vec())
    }

    pub async fn send_email(
        &self,
        to: &str,
        subject: &str,
        body: &str,
    ) -> Result<String, EwsError> {
        let body_xml = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
            <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"
                          xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
                <soap:Header>
                    <t:RequestServerVersion Version="Exchange2016"/>
                </soap:Header>
                <soap:Body>
                    <CreateItem xmlns="http://schemas.microsoft.com/exchange/services/2006/messages" MessageDisposition="SendAndSaveCopy">
                        <SavedItemFolderId>
                            <t:DistinguishedFolderId Id="sentitems"/>
                        </SavedItemFolderId>
                        <Items>
                            <t:Message>
                                <t:Subject>{}</t:Subject>
                                <t:Body BodyType="Text">{}</t:Body>
                                <t:ToRecipients>
                                    <t:Mailbox>
                                        <t:EmailAddress>{}</t:EmailAddress>
                                    </t:Mailbox>
                                </t:ToRecipients>
                            </t:Message>
                        </Items>
                    </CreateItem>
                </soap:Body>
            </soap:Envelope>"#,
            escape_xml(subject),
            escape_xml(body),
            escape_xml(to)
        );

        let xml = self.send_request_xml(body_xml).await?;

        if let Ok(envelope) = quick_xml::de::from_str::<soap::Envelope<CreateItemResponse>>(&xml) {
            if let Some(id) = envelope
                .body
                .response
                .response_messages
                .create_item
                .items
                .messages
                .into_iter()
                .next()
                .map(|m| m.item_id.id)
                .filter(|id| !id.trim().is_empty())
            {
                return Ok(id);
            }
        }

        if let Some(id) = extract_first_item_id_from_xml(&xml) {
            return Ok(id);
        }

        if xml_indicates_success(&xml) {
            warn!("CreateItem succeeded without item id; returning synthetic id");
            return Ok("sent-no-itemid".to_string());
        }

        debug_response_excerpt("CreateItem", &xml);
        Err(EwsError::NotFound(
            "item id not found in CreateItem response".to_string(),
        ))
    }

    pub async fn mark_read(&self, item_id: &str, is_read: bool) -> Result<(), EwsError> {
        let is_read_str = if is_read { "true" } else { "false" };

        let safe_item_id = escape_xml(item_id);
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
            <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"
                          xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
                <soap:Header>
                    <t:RequestServerVersion Version="Exchange2016"/>
                </soap:Header>
                <soap:Body>
                    <UpdateItem xmlns="http://schemas.microsoft.com/exchange/services/2006/messages" ConflictResolution="AlwaysOverwrite">
                        <ItemChanges>
                            <t:ItemChange>
                                <t:ItemId Id="{}"/>
                                <t:Updates>
                                    <t:SetItemField>
                                        <t:FieldURI FieldURI="message:IsRead"/>
                                        <t:Message>
                                            <t:IsRead>{}</t:IsRead>
                                        </t:Message>
                                    </t:SetItemField>
                                </t:Updates>
                            </t:ItemChange>
                        </ItemChanges>
                    </UpdateItem>
                </soap:Body>
            </soap:Envelope>"#,
            safe_item_id, is_read_str
        );

        self.send_request::<()>("UpdateItem", body).await?;
        Ok(())
    }

    pub async fn move_item(
        &self,
        item_id: &str,
        destination_folder: &str,
    ) -> Result<String, EwsError> {
        let safe_destination_folder = escape_xml(destination_folder);
        let safe_item_id = escape_xml(item_id);
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
            <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"
                          xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
                <soap:Header>
                    <t:RequestServerVersion Version="Exchange2016"/>
                </soap:Header>
                <soap:Body>
                    <MoveItem xmlns="http://schemas.microsoft.com/exchange/services/2006/messages">
                        <ToFolderId>
                            <t:FolderId Id="{}"/>
                        </ToFolderId>
                        <ItemIds>
                            <t:ItemId Id="{}"/>
                        </ItemIds>
                    </MoveItem>
                </soap:Body>
            </soap:Envelope>"#,
            safe_destination_folder, safe_item_id
        );

        let xml = self.send_request_xml(body).await?;

        if let Ok(envelope) = quick_xml::de::from_str::<soap::Envelope<MoveItemResponse>>(&xml) {
            if let Some(item_id) = envelope
                .body
                .response
                .response_messages
                .move_item
                .items
                .item_id
            {
                if !item_id.id.trim().is_empty() {
                    return Ok(item_id.id);
                }
            }

            if let Some(first) = envelope
                .body
                .response
                .response_messages
                .move_item
                .items
                .messages
                .into_iter()
                .next()
                .map(|m| m.item_id.id)
                .filter(|id| !id.trim().is_empty())
            {
                return Ok(first);
            }
        }

        if let Some(id) = extract_first_item_id_from_xml(&xml) {
            return Ok(id);
        }

        if xml_indicates_success(&xml) {
            warn!("MoveItem succeeded without returned item id; returning original id");
            return Ok(item_id.to_string());
        }

        debug_response_excerpt("MoveItem", &xml);
        Err(EwsError::NotFound(
            "item id not found in MoveItem response".to_string(),
        ))
    }

    pub async fn delete_item(&self, item_id: &str, skip_trash: bool) -> Result<(), EwsError> {
        let safe_item_id = escape_xml(item_id);
        let delete_type = if skip_trash {
            "SoftDelete"
        } else {
            "MoveToDeletedItems"
        };
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
            <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"
                          xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
                <soap:Header>
                    <t:RequestServerVersion Version="Exchange2016"/>
                </soap:Header>
                <soap:Body>
                    <DeleteItem xmlns="http://schemas.microsoft.com/exchange/services/2006/messages" DeleteType="{}">
                        <ItemIds>
                            <t:ItemId Id="{}"/>
                        </ItemIds>
                    </DeleteItem>
                </soap:Body>
            </soap:Envelope>"#,
            delete_type, safe_item_id
        );

        self.send_request::<()>("DeleteItem", body).await?;
        Ok(())
    }

    async fn send_request<T: for<'de> Deserialize<'de>>(
        &self,
        _action: &str,
        body: String,
    ) -> Result<T, EwsError> {
        let response_body = self.send_request_xml(body).await?;

        let result = quick_xml::de::from_str::<soap::Envelope<T>>(&response_body)
            .map_err(|e| EwsError::XmlError(format!("{}: {}", e, &response_body)))?;

        Ok(result.body.response)
    }

    async fn send_request_xml(&self, body: String) -> Result<String, EwsError> {
        let mode = normalized_auth_mode(&self.credentials.auth_mode);
        let auth = if mode == "basic" {
            let basic_token = general_purpose::STANDARD.encode(format!(
                "{}:{}",
                self.credentials.username, self.credentials.password
            ));
            Some(authorization_header(&self.credentials, &basic_token)?)
        } else {
            None
        };

        let response = self
            .post_xml_with_retry(&self.ews_url, body, "application/xml", auth)
            .await?;

        if response.status < 200 || response.status >= 300 {
            error!(
                "EWS request failed: {} auth={:?} body={} ",
                response.status, response.www_authenticate, response.body
            );
            return Err(EwsError::SoapError(format!(
                "HTTP {} auth={:?}: {}",
                response.status, response.www_authenticate, response.body
            )));
        }

        if response.body.contains("<soap:Fault>") || response.body.contains("<Fault>") {
            let fault_string = extract_fault_string(&response.body);
            return Err(EwsError::SoapError(fault_string));
        }

        Ok(response.body)
    }

    async fn post_xml_with_retry(
        &self,
        url: &str,
        body: String,
        accept: &str,
        basic_auth_header: Option<String>,
    ) -> Result<HttpResponse, EwsError> {
        let attempts = self.options.retry_max_attempts.max(1);
        let mut last_error: Option<EwsError> = None;

        for attempt in 1..=attempts {
            let result = self
                .post_xml_with_auth(url, body.clone(), accept, basic_auth_header.clone())
                .await;

            match result {
                Ok(response) => {
                    if is_retryable_http_status(response.status) && attempt < attempts {
                        let delay = backoff_delay(
                            attempt,
                            self.options.retry_base_ms,
                            self.options.retry_max_backoff_ms,
                        );
                        warn!(
                            "retryable status {} for {} attempt {}/{}, retrying in {:?}",
                            response.status, url, attempt, attempts, delay
                        );
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    return Ok(response);
                }
                Err(err) => {
                    if is_retryable_error(&err) && attempt < attempts {
                        let delay = backoff_delay(
                            attempt,
                            self.options.retry_base_ms,
                            self.options.retry_max_backoff_ms,
                        );
                        warn!(
                            "retryable network error for {} attempt {}/{}, retrying in {:?}: {}",
                            url, attempt, attempts, delay, err
                        );
                        last_error = Some(err);
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    return Err(err);
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| EwsError::HttpError("request failed after retries".to_string())))
    }

    async fn post_xml_with_auth(
        &self,
        url: &str,
        body: String,
        accept: &str,
        basic_auth_header: Option<String>,
    ) -> Result<HttpResponse, EwsError> {
        let mode = normalized_auth_mode(&self.credentials.auth_mode);
        if mode == "ntlm" {
            let url = url.to_string();
            let username = self.credentials.username.clone();
            let password = self.credentials.password.clone();
            let accept = accept.to_string();
            let response = tokio::task::spawn_blocking(move || {
                ntlm_post_blocking(&url, &username, &password, &body, &accept)
            })
            .await
            .map_err(|e| EwsError::HttpError(e.to_string()))??;
            return Ok(response);
        }

        let mut request = self
            .http_client
            .post(url)
            .header("Content-Type", "text/xml; charset=utf-8")
            .header("Accept", accept)
            .body(body);

        if let Some(auth) = basic_auth_header {
            request = request.header("Authorization", auth);
        }

        let response = request
            .send()
            .await
            .map_err(|e| EwsError::HttpError(e.to_string()))?;

        let status = response.status().as_u16();
        let www_authenticate = response
            .headers()
            .get(reqwest::header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned);
        let body = response
            .text()
            .await
            .map_err(|e| EwsError::HttpError(e.to_string()))?;

        Ok(HttpResponse {
            status,
            body,
            www_authenticate,
        })
    }
}

fn ntlm_post_blocking(
    url: &str,
    username: &str,
    password: &str,
    request_body: &str,
    accept: &str,
) -> Result<HttpResponse, EwsError> {
    let mut easy = Easy::new();
    easy.url(url)
        .map_err(|e| EwsError::HttpError(e.to_string()))?;
    easy.post(true)
        .map_err(|e| EwsError::HttpError(e.to_string()))?;
    easy.username(username)
        .map_err(|e| EwsError::AuthError(e.to_string()))?;
    easy.password(password)
        .map_err(|e| EwsError::AuthError(e.to_string()))?;
    easy.http_auth(Auth::new().ntlm(true))
        .map_err(|e| EwsError::AuthError(e.to_string()))?;
    easy.post_fields_copy(request_body.as_bytes())
        .map_err(|e| EwsError::HttpError(e.to_string()))?;

    let mut headers = List::new();
    headers
        .append("Content-Type: text/xml; charset=utf-8")
        .map_err(|e| EwsError::HttpError(e.to_string()))?;
    headers
        .append(&format!("Accept: {}", accept))
        .map_err(|e| EwsError::HttpError(e.to_string()))?;
    easy.http_headers(headers)
        .map_err(|e| EwsError::HttpError(e.to_string()))?;

    let mut response_body = Vec::new();
    let mut www_authenticate: Option<String> = None;
    {
        let mut transfer = easy.transfer();
        transfer
            .write_function(|new_data| {
                response_body.extend_from_slice(new_data);
                Ok(new_data.len())
            })
            .map_err(|e| EwsError::HttpError(e.to_string()))?;

        transfer
            .header_function(|header| {
                if let Ok(header_str) = std::str::from_utf8(header) {
                    if let Some((name, value)) = header_str.split_once(':') {
                        if name.eq_ignore_ascii_case("www-authenticate") {
                            www_authenticate = Some(value.trim().to_string());
                        }
                    }
                }
                true
            })
            .map_err(|e| EwsError::HttpError(e.to_string()))?;

        transfer
            .perform()
            .map_err(|e| EwsError::HttpError(e.to_string()))?;
    }

    let status = easy
        .response_code()
        .map_err(|e| EwsError::HttpError(e.to_string()))? as u16;

    Ok(HttpResponse {
        status,
        body: String::from_utf8_lossy(&response_body).to_string(),
        www_authenticate,
    })
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn extract_fault_string(body: &str) -> String {
    if let Some(start) = body.find("<faultstring>") {
        let start = start + 13;
        if let Some(end) = body[start..].find("</faultstring>") {
            return body[start..start + end].to_string();
        }
    }
    "Unknown fault".to_string()
}

fn is_retryable_http_status(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504)
}

fn is_retryable_error(err: &EwsError) -> bool {
    match err {
        EwsError::HttpError(msg) => {
            let lower = msg.to_ascii_lowercase();
            lower.contains("timed out")
                || lower.contains("timeout")
                || lower.contains("connection reset")
                || lower.contains("connection refused")
                || lower.contains("temporarily unavailable")
                || lower.contains("tls connect error")
                || lower.contains("ssl connect error")
                || lower.contains("unexpected eof")
                || lower.contains("empty reply from server")
                || lower.contains("recv failure")
                || lower.contains("dns")
                || lower.contains("lookup address information")
        }
        _ => false,
    }
}

fn backoff_delay(attempt: u32, base_ms: u64, max_backoff_ms: u64) -> Duration {
    let shift = min(attempt.saturating_sub(1), 10);
    let exp = base_ms.saturating_mul(1u64 << shift);
    let capped = min(exp, max_backoff_ms);
    let jitter = ((attempt as u64).saturating_mul(137)) % 251;
    Duration::from_millis(capped.saturating_add(jitter))
}

fn extract_first_item_id_from_xml(xml: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    reader.trim_text(true);

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == "ItemId" {
                    for attr in e.attributes().flatten() {
                        if local_name(attr.key.as_ref()) == "Id" {
                            let v = String::from_utf8_lossy(attr.value.as_ref()).to_string();
                            if !v.trim().is_empty() {
                                return Some(v);
                            }
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    None
}

fn xml_indicates_success(xml: &str) -> bool {
    (xml.contains("ResponseClass=\"Success\"") || xml.contains("ResponseClass='Success'"))
        && xml.contains("NoError")
}

fn debug_response_excerpt(op: &str, xml: &str) {
    let start = xml
        .find("ResponseMessages")
        .or_else(|| xml.find("ResponseClass"))
        .unwrap_or(0);
    let excerpt: String = xml.chars().skip(start).take(1400).collect();
    warn!("{} response excerpt: {}", op, excerpt);
}

pub fn distinguished_folder_id_from_spec(spec: &str) -> Option<&'static str> {
    let normalized = spec.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "inbox" => Some("inbox"),
        "sent" | "sentitems" | "sent_items" | "sent items" => Some("sentitems"),
        "draft" | "drafts" => Some("drafts"),
        "deleted" | "deleteditems" | "deleted_items" | "deleted items" | "trash" => {
            Some("deleteditems")
        }
        "junk" | "junkemail" | "junk_email" | "junk email" | "spam" => Some("junkemail"),
        "archive" => Some("archive"),
        "outbox" => Some("outbox"),
        "calendar" => Some("calendar"),
        "contacts" => Some("contacts"),
        "tasks" => Some("tasks"),
        _ => None,
    }
}

fn normalized_auth_mode(mode: &str) -> &str {
    if mode.trim().is_empty() {
        "basic"
    } else {
        mode.trim()
    }
}

fn authorization_header(
    credentials: &EwsCredentials,
    basic_token: &str,
) -> Result<String, EwsError> {
    let mode = normalized_auth_mode(&credentials.auth_mode);

    match mode.to_lowercase().as_str() {
        "basic" => Ok(format!("Basic {}", basic_token)),
        "ntlm" => Err(EwsError::AuthError(
            "NTLM authentication is configured but not implemented in this client".to_string(),
        )),
        "oauth" => Err(EwsError::AuthError(
            "OAuth authentication is configured but not implemented in this on-prem client"
                .to_string(),
        )),
        other => Err(EwsError::AuthError(format!(
            "unsupported auth mode: {}",
            other
        ))),
    }
}

fn extract_ews_url_from_autodiscover(xml: &str) -> Option<String> {
    extract_tag_value(xml, &["EwsUrl", "ExternalEwsUrl", "InternalEwsUrl"])
        .map(|v| normalize_ews_url(&v))
}

fn extract_redirect_url_from_autodiscover(xml: &str) -> Option<String> {
    extract_tag_value(xml, &["RedirectUrl"])
}

fn normalize_ews_url(url: &str) -> String {
    if url.ends_with("/EWS/Exchange.asmx") {
        return url.to_string();
    }
    if url.ends_with("/Autodiscover/Autodiscover.xml") {
        return url.replace("/Autodiscover/Autodiscover.xml", "/EWS/Exchange.asmx");
    }
    if url.ends_with('/') {
        return format!("{}EWS/Exchange.asmx", url);
    }
    if url.contains("/EWS/") {
        return url.to_string();
    }
    format!("{}/EWS/Exchange.asmx", url)
}

fn extract_tag_value(xml: &str, candidates: &[&str]) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    reader.trim_text(true);

    let mut buf = Vec::new();
    let mut capture = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = local_name(e.name().as_ref()).to_string();
                capture = candidates.iter().any(|candidate| *candidate == local);
            }
            Ok(Event::Text(e)) if capture => {
                if let Ok(text) = e.unescape() {
                    return Some(text.into_owned());
                }
            }
            Ok(Event::End(_)) => capture = false,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }

        buf.clear();
    }

    None
}

fn local_name(raw_name: &[u8]) -> &str {
    let name = std::str::from_utf8(raw_name).unwrap_or_default();
    match name.rsplit_once(':') {
        Some((_, local)) => local,
        None => name,
    }
}

fn find_folder_in_xml(xml: &str, folder_name: &str) -> Option<Folder> {
    let mut reader = Reader::from_str(xml);
    reader.trim_text(true);

    let mut buf = Vec::new();
    let mut current_tag = String::new();
    let mut current_folder: Option<Folder> = None;
    let mut in_folder_node = false;
    let target = folder_name.to_lowercase();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_string();
                current_tag = name.clone();

                if is_folder_node(&name) {
                    in_folder_node = true;
                    current_folder = Some(Folder::default());
                } else if in_folder_node && name == "FolderId" {
                    if let Some(folder) = current_folder.as_mut() {
                        for attr in e.attributes().flatten() {
                            let key = local_name(attr.key.as_ref());
                            let value = String::from_utf8_lossy(attr.value.as_ref()).to_string();
                            if key == "Id" {
                                folder.folder_id.id = value;
                            } else if key == "ChangeKey" {
                                folder.folder_id.change_key = value;
                            }
                        }
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref()).to_string();
                if in_folder_node && name == "FolderId" {
                    if let Some(folder) = current_folder.as_mut() {
                        for attr in e.attributes().flatten() {
                            let key = local_name(attr.key.as_ref());
                            let value = String::from_utf8_lossy(attr.value.as_ref()).to_string();
                            if key == "Id" {
                                folder.folder_id.id = value;
                            } else if key == "ChangeKey" {
                                folder.folder_id.change_key = value;
                            }
                        }
                    }
                }
            }
            Ok(Event::Text(e)) => {
                if in_folder_node {
                    if let Some(folder) = current_folder.as_mut() {
                        if let Ok(text) = e.unescape() {
                            let value = text.into_owned();
                            match current_tag.as_str() {
                                "DisplayName" => folder.display_name = value,
                                "TotalCount" => {
                                    folder.total_count = value.parse::<i32>().unwrap_or(0)
                                }
                                "UnreadCount" => {
                                    folder.unread_count = value.parse::<i32>().unwrap_or(0)
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            Ok(Event::End(e)) => {
                let name = local_name(e.name().as_ref()).to_string();
                if in_folder_node && is_folder_node(&name) {
                    if let Some(folder) = current_folder.take() {
                        if folder.display_name.to_lowercase() == target {
                            return Some(folder);
                        }
                    }
                    in_folder_node = false;
                }
                current_tag.clear();
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    None
}

fn is_folder_node(name: &str) -> bool {
    matches!(
        name,
        "Folder" | "SearchFolder" | "CalendarFolder" | "ContactsFolder" | "TasksFolder"
    )
}

mod soap {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Deserialize, Serialize)]
    pub struct Envelope<T> {
        #[serde(rename = "Body")]
        pub body: Body<T>,
    }

    #[derive(Debug, Deserialize, Serialize)]
    pub struct Body<T> {
        #[serde(rename = "$value")]
        pub response: T,
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SyncFolderItemsResponse {
    #[serde(rename = "ResponseMessages")]
    pub response_messages: SyncFolderItemsResponseMessages,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SyncFolderItemsResponseMessages {
    #[serde(rename = "SyncFolderItemsResponseMessage")]
    pub sync_folder_items: SyncFolderItemsResponseMessage,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SyncFolderItemsResponseMessage {
    #[serde(rename = "ResponseCode")]
    pub response_code: String,
    #[serde(rename = "SyncState", default)]
    pub sync_state: Option<String>,
    #[serde(rename = "Changes", default)]
    pub changes: ChangesType,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct ChangesType {
    #[serde(rename = "Create", default)]
    pub create: Option<Vec<EmailChange>>,
    #[serde(rename = "Update", default)]
    pub update: Option<Vec<EmailChange>>,
    #[serde(rename = "Delete", default)]
    pub delete: Option<Vec<DeleteType>>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct EmailChange {
    #[serde(rename = "Message", default)]
    pub messages: Vec<Email>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct DeleteType {
    #[serde(rename = "ItemId")]
    pub item_id: ItemId,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GetFolderResponse {
    #[serde(rename = "ResponseMessages")]
    pub response_messages: GetFolderResponseMessages,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GetFolderResponseMessages {
    #[serde(rename = "GetFolderResponseMessage")]
    pub get_folder: GetFolderResponseMessage,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GetFolderResponseMessage {
    #[serde(rename = "Folders")]
    pub folders: FoldersResponse,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct FoldersResponse {
    #[serde(rename = "Folder")]
    pub folder: Folder,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct FindFolderResponse {
    #[serde(rename = "ResponseMessages")]
    pub response_messages: FindFolderResponseMessages,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct FindFolderResponseMessages {
    #[serde(rename = "FindFolderResponseMessage")]
    pub find_folder: FindFolderResponseMessage,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct FindFolderResponseMessage {
    #[serde(rename = "RootFolder")]
    pub root_folder: RootFolder,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RootFolder {
    #[serde(rename = "Folders")]
    pub folders: FolderCollection,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct FolderCollection {
    #[serde(rename = "Folder", default)]
    pub folder: Vec<Folder>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GetItemResponse {
    #[serde(rename = "ResponseMessages")]
    pub response_messages: GetItemResponseMessages,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GetItemResponseMessages {
    #[serde(rename = "GetItemResponseMessage")]
    pub get_item: GetItemResponseMessage,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GetItemResponseMessage {
    #[serde(rename = "Items")]
    pub items: ItemsResponse,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct FindItemResponse {
    #[serde(rename = "ResponseMessages")]
    pub response_messages: FindItemResponseMessages,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct FindItemResponseMessages {
    #[serde(rename = "FindItemResponseMessage")]
    pub find_item: FindItemResponseMessage,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct FindItemResponseMessage {
    #[serde(rename = "RootFolder")]
    pub root_folder: RootFolderData,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RootFolderData {
    #[serde(rename = "Items")]
    pub items: ItemsResponse,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ItemsResponse {
    #[serde(rename = "Message", default)]
    pub messages: Vec<Email>,
    #[serde(rename = "Item", default)]
    pub items: Vec<Email>,
}

impl ItemsResponse {
    pub fn into_vec(self) -> Vec<Email> {
        if self.messages.is_empty() {
            self.items
        } else {
            self.messages
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CreateItemResponse {
    #[serde(rename = "ResponseMessages")]
    pub response_messages: CreateItemResponseMessages,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CreateItemResponseMessages {
    #[serde(rename = "CreateItemResponseMessage")]
    pub create_item: CreateItemResponseMessage,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CreateItemResponseMessage {
    #[serde(rename = "Items")]
    pub items: ItemsResponse,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MoveItemResponse {
    #[serde(rename = "ResponseMessages")]
    pub response_messages: MoveItemResponseMessages,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MoveItemResponseMessages {
    #[serde(rename = "MoveItemResponseMessage")]
    pub move_item: MoveItemResponseMessage,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MoveItemResponseMessage {
    #[serde(rename = "Items")]
    pub items: MovedItems,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct MovedItems {
    #[serde(rename = "Message", default)]
    pub messages: Vec<Email>,
    #[serde(rename = "Item", default)]
    pub items: Vec<Email>,
    #[serde(rename = "ItemId", default)]
    pub item_id: Option<ItemId>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct Folder {
    #[serde(rename = "FolderId")]
    pub folder_id: FolderId,
    #[serde(rename = "DisplayName", default)]
    pub display_name: String,
    #[serde(rename = "TotalCount", default)]
    pub total_count: i32,
    #[serde(rename = "UnreadCount", default)]
    pub unread_count: i32,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct FolderId {
    #[serde(rename = "@Id", default)]
    pub id: String,
    #[serde(rename = "@ChangeKey", default)]
    pub change_key: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Email {
    #[serde(rename = "ItemId", default)]
    pub item_id: ItemId,
    #[serde(rename = "Subject", default)]
    pub subject: String,
    #[serde(rename = "Sender", default)]
    pub sender: MailboxContainer,
    #[serde(rename = "ToRecipients", default)]
    pub to_recipients: RecipientCollection,
    #[serde(rename = "CcRecipients", default)]
    pub cc_recipients: RecipientCollection,
    #[serde(rename = "DateTimeReceived", default)]
    pub datetime_received: String,
    #[serde(rename = "DateTimeSent", default)]
    pub datetime_sent: String,
    #[serde(rename = "Body", default)]
    pub body: BodyType,
    #[serde(rename = "TextBody", default)]
    pub text_body: BodyType,
    #[serde(rename = "HasAttachments", default)]
    pub has_attachments: bool,
    #[serde(rename = "IsRead", default)]
    pub is_read: bool,
    #[serde(rename = "Importance", default)]
    pub importance: String,
    #[serde(rename = "From", default)]
    pub from: MailboxContainer,
}

impl Default for Email {
    fn default() -> Self {
        Self {
            item_id: ItemId::default(),
            subject: String::new(),
            sender: MailboxContainer::default(),
            to_recipients: RecipientCollection::default(),
            cc_recipients: RecipientCollection::default(),
            datetime_received: String::new(),
            datetime_sent: String::new(),
            body: BodyType::default(),
            text_body: BodyType::default(),
            has_attachments: false,
            is_read: false,
            importance: "Normal".to_string(),
            from: MailboxContainer::default(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct MailboxContainer {
    #[serde(rename = "Mailbox", default)]
    pub mailbox: Option<EmailAddressType>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct RecipientCollection {
    #[serde(rename = "Mailbox", default)]
    pub mailbox: Vec<EmailAddressType>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct BodyType {
    #[serde(rename = "@BodyType", default)]
    pub body_type: String,
    #[serde(rename = "$text", default)]
    pub value: String,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct EmailAddressType {
    #[serde(rename = "Name", default)]
    pub name: Option<String>,
    #[serde(rename = "EmailAddress", default)]
    pub email_address: String,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct ItemId {
    #[serde(rename = "@Id", default)]
    pub id: String,
    #[serde(rename = "@ChangeKey", default)]
    pub change_key: String,
}
