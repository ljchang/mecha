//! The Gmail client, extracted from flowmail's `email/gmail.rs` and trimmed
//! to what an agent doing on-demand work needs: search, thread reads, and
//! send. The history/sync machinery, spam/trash/archive ops, and the local
//! cache stayed behind — `threads.get` (which flowmail reconstructed from
//! SQL) is the one addition.

use base64::{engine::general_purpose::URL_SAFE, Engine};
use serde_json::Value;

use crate::http::send_with_retry;
use crate::types::{Email, MailError};

#[derive(Clone)]
pub struct GmailProvider {
    access_token: String,
    client: reqwest::Client,
}

impl GmailProvider {
    pub fn new(access_token: String) -> Self {
        Self { access_token, client: crate::http::client() }
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.access_token)
    }

    async fn get_json(&self, url: &str) -> Result<Value, MailError> {
        let resp = send_with_retry(
            self.client.get(url).header("Authorization", self.auth_header()),
        )
        .await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(MailError::ApiError { status, message: body });
        }
        resp.json::<Value>().await.map_err(MailError::from)
    }

    /// Fetch a single Gmail message by id, full format.
    async fn get_message(&self, message_id: &str) -> Result<Value, MailError> {
        self.get_json(&format!(
            "https://gmail.googleapis.com/gmail/v1/users/me/messages/{message_id}?format=full"
        ))
        .await
    }

    /// Fetch full messages for the given ids concurrently (bounded),
    /// preserving input order. Individual failures are logged and skipped.
    /// The bound stays well under Gmail's per-user quota (messages.get costs
    /// 5 units of ~250 units/sec).
    async fn get_messages_concurrently(&self, ids: &[String]) -> Vec<Email> {
        const CONCURRENT_FETCHES: usize = 10;
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(CONCURRENT_FETCHES));
        let mut set = tokio::task::JoinSet::new();
        for (idx, msg_id) in ids.iter().enumerate() {
            let provider = self.clone();
            let msg_id = msg_id.clone();
            let permit = semaphore.clone();
            set.spawn(async move {
                let _permit = permit.acquire().await.expect("semaphore closed");
                match provider.get_message(&msg_id).await {
                    Ok(json) => Some((idx, parse_gmail_message(&json))),
                    Err(e) => {
                        tracing::warn!("failed to fetch Gmail message {msg_id}: {e}");
                        None
                    }
                }
            });
        }

        let mut slots: Vec<Option<Email>> = (0..ids.len()).map(|_| None).collect();
        while let Some(res) = set.join_next().await {
            if let Ok(Some((idx, email))) = res {
                slots[idx] = Some(email);
            }
        }
        slots.into_iter().flatten().collect()
    }

    /// Search: list matching message ids (cheap), then fetch them
    /// concurrently. `query` is Gmail search syntax (`from:`, `after:`, …).
    pub async fn search(&self, query: &str, max_results: u32) -> Result<Vec<Email>, MailError> {
        let page_size = max_results.clamp(1, 100);
        let mut message_ids: Vec<String> = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let mut list_url = format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/messages?maxResults={page_size}&q={}",
                urlencode(query)
            );
            if let Some(ref token) = page_token {
                list_url.push_str(&format!("&pageToken={token}"));
            }

            let list_json = self.get_json(&list_url).await?;
            if let Some(refs) = list_json["messages"].as_array() {
                for msg_ref in refs {
                    if message_ids.len() >= max_results as usize {
                        break;
                    }
                    if let Some(id) = msg_ref["id"].as_str() {
                        if !id.is_empty() {
                            message_ids.push(id.to_string());
                        }
                    }
                }
            }
            match list_json["nextPageToken"].as_str() {
                Some(token) if message_ids.len() < max_results as usize => {
                    page_token = Some(token.to_string());
                }
                _ => break,
            }
        }

        Ok(self.get_messages_concurrently(&message_ids).await)
    }

    /// Fetch a whole conversation in one round trip — the payload flowmail
    /// reconstructed from its SQL cache.
    pub async fn get_thread(&self, thread_id: &str) -> Result<Vec<Email>, MailError> {
        let json = self
            .get_json(&format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/threads/{thread_id}?format=full"
            ))
            .await?;
        let messages = json["messages"]
            .as_array()
            .ok_or_else(|| MailError::ParseError("thread has no messages array".into()))?;
        let mut emails: Vec<Email> = messages.iter().map(parse_gmail_message).collect();
        // Oldest first is a promise callers build on — the unified reply
        // tool answers `last()` as "the newest message" — and the API's
        // array order is observed, not documented. The stamps are
        // fixed-width ISO, so the string sort is chronological.
        emails.sort_by(|a, b| a.date_received.cmp(&b.date_received));
        Ok(emails)
    }

    /// Send via `messages.send`, returning the sent message's id. `body` is
    /// HTML — callers convert markdown before this point.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_email(
        &self,
        to: &str,
        subject: &str,
        body: &str,
        thread_id: Option<&str>,
        cc: Option<&str>,
        bcc: Option<&str>,
        in_reply_to: Option<&str>,
    ) -> Result<String, MailError> {
        let raw_message = build_gmail_raw_message(to, subject, body, cc, bcc, in_reply_to);
        let encoded = URL_SAFE.encode(raw_message.as_bytes());

        let mut payload = serde_json::json!({ "raw": encoded });
        if let Some(tid) = thread_id {
            payload["threadId"] = serde_json::json!(tid);
        }

        let resp = send_with_retry(
            self.client
                .post("https://gmail.googleapis.com/gmail/v1/users/me/messages/send")
                .header("Authorization", self.auth_header())
                .json(&payload),
        )
        .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(MailError::ApiError { status, message: body });
        }
        let json: Value = resp.json().await?;
        Ok(json["id"].as_str().unwrap_or_default().to_string())
    }

    /// The authenticated account's address — doubles as the auth smoke test.
    pub async fn profile_address(&self) -> Result<String, MailError> {
        let json =
            self.get_json("https://gmail.googleapis.com/gmail/v1/users/me/profile").await?;
        json["emailAddress"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| MailError::ParseError("profile has no emailAddress".into()))
    }
}

/// Guard against email header injection: a reply Message-ID comes from an
/// untrusted incoming email, so refuse control chars (CR/LF/NUL).
pub fn valid_reply_message_id(mid: &str) -> bool {
    !mid.is_empty() && !mid.chars().any(|c| c == '\r' || c == '\n' || c == '\0')
}

pub(crate) fn build_gmail_raw_message(
    to: &str,
    subject: &str,
    body: &str,
    cc: Option<&str>,
    bcc: Option<&str>,
    in_reply_to: Option<&str>,
) -> String {
    let mut headers = format!("To: {to}\r\n");
    if let Some(cc_val) = cc {
        if !cc_val.is_empty() {
            headers.push_str(&format!("Cc: {cc_val}\r\n"));
        }
    }
    if let Some(bcc_val) = bcc {
        if !bcc_val.is_empty() {
            headers.push_str(&format!("Bcc: {bcc_val}\r\n"));
        }
    }
    if let Some(mid) = in_reply_to {
        if valid_reply_message_id(mid) {
            headers.push_str(&format!("In-Reply-To: {mid}\r\nReferences: {mid}\r\n"));
        }
    }
    headers.push_str(&format!(
        "Subject: {subject}\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{body}"
    ));
    headers
}

/// Pull the RFC Message-ID out of a Gmail header array (case-insensitive).
pub(crate) fn extract_message_id(headers: &[Value]) -> Option<String> {
    headers.iter().find_map(|h| {
        let name = h.get("name")?.as_str()?;
        if name.eq_ignore_ascii_case("message-id") {
            h.get("value")?.as_str().map(|s| s.trim().to_string())
        } else {
            None
        }
    })
}

/// Parse a Gmail API message JSON into an [`Email`].
pub(crate) fn parse_gmail_message(msg: &Value) -> Email {
    let provider_id = msg["id"].as_str().unwrap_or_default().to_string();
    let thread_id = msg["threadId"].as_str().map(|s| s.to_string());
    let snippet = msg["snippet"].as_str().unwrap_or_default().to_string();

    let headers = msg["payload"]["headers"].as_array();
    let mut subject = String::new();
    let mut from_raw = String::new();
    let mut to_raw = String::new();
    let mut cc_raw = String::new();
    let mut bcc_raw = String::new();
    let mut date_str = String::new();
    let mut list_unsubscribe = None;

    if let Some(hdrs) = headers {
        for h in hdrs {
            let name = h["name"].as_str().unwrap_or_default().to_lowercase();
            let value = h["value"].as_str().unwrap_or_default();
            match name.as_str() {
                "subject" => subject = value.to_string(),
                "from" => from_raw = value.to_string(),
                "to" => to_raw = value.to_string(),
                "cc" => cc_raw = value.to_string(),
                "bcc" => bcc_raw = value.to_string(),
                "date" => date_str = value.to_string(),
                "list-unsubscribe" => list_unsubscribe = Some(value.to_string()),
                _ => {}
            }
        }
    }
    let message_id = headers.and_then(|h| extract_message_id(h));

    let (from_name, from_address) = parse_email_address(&from_raw);

    let parse_addr_list = |raw: &str| -> Vec<String> {
        raw.split(',')
            .map(|s| parse_email_address(s.trim()).1)
            .filter(|s| !s.is_empty())
            .collect()
    };
    let to_addresses = parse_addr_list(&to_raw);
    let cc_addresses = parse_addr_list(&cc_raw);
    let bcc_addresses = parse_addr_list(&bcc_raw);

    // Prefer internalDate (epoch millis) over the header, which lies more.
    let date_received = if let Some(internal_date) =
        msg["internalDate"].as_str().and_then(|s| s.parse::<i64>().ok())
    {
        match chrono::DateTime::from_timestamp(internal_date / 1000, 0) {
            Some(dt) => dt.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            None => parse_date_to_iso(&date_str),
        }
    } else {
        parse_date_to_iso(&date_str)
    };

    // Raw parts; the HTML-only fallback is applied by `text::clean_body`,
    // where the model-facing rendering lives.
    let (body_text, body_html) = extract_body_parts(&msg["payload"]);

    let labels: Vec<String> = msg["labelIds"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();

    let is_read = !labels.contains(&"UNREAD".to_string());
    let is_starred = labels.contains(&"STARRED".to_string());
    let has_attachments = check_has_attachments(&msg["payload"]);

    Email {
        id: format!("gmail-{provider_id}"),
        provider: "gmail".to_string(),
        provider_id,
        thread_id,
        message_id,
        subject,
        from_address,
        from_name,
        to_addresses,
        cc_addresses,
        bcc_addresses,
        date_received,
        body_text,
        body_html,
        snippet,
        labels,
        is_read,
        is_starred,
        has_attachments,
        list_unsubscribe,
    }
}

/// Parse "Name <email@addr.com>" into (name, email).
fn parse_email_address(raw: &str) -> (String, String) {
    let raw = raw.trim();
    if let Some(start) = raw.find('<') {
        if let Some(end) = raw.find('>') {
            let name = raw[..start].trim().trim_matches('"').to_string();
            let email = raw[start + 1..end].trim().to_string();
            return (name, email);
        }
    }
    (String::new(), raw.to_string())
}

/// Extract text and HTML body parts from a Gmail payload, recursively.
fn extract_body_parts(payload: &Value) -> (String, String) {
    let mut body_text = String::new();
    let mut body_html = String::new();

    let mime_type = payload["mimeType"].as_str().unwrap_or_default();
    if let Some(data) = payload["body"]["data"].as_str() {
        if let Ok(decoded) = URL_SAFE.decode(data) {
            if let Ok(text) = String::from_utf8(decoded) {
                match mime_type {
                    "text/plain" => body_text = text,
                    "text/html" => body_html = text,
                    _ => {}
                }
            }
        }
    }

    if let Some(parts) = payload["parts"].as_array() {
        for part in parts {
            let (text, html) = extract_body_parts(part);
            if !text.is_empty() && body_text.is_empty() {
                body_text = text;
            }
            if !html.is_empty() && body_html.is_empty() {
                body_html = html;
            }
        }
    }

    (body_text, body_html)
}

fn check_has_attachments(payload: &Value) -> bool {
    if let Some(filename) = payload["filename"].as_str() {
        if !filename.is_empty() {
            return true;
        }
    }
    if let Some(parts) = payload["parts"].as_array() {
        for part in parts {
            if check_has_attachments(part) {
                return true;
            }
        }
    }
    false
}

/// Parse various date formats into ISO 8601, normalized to UTC.
fn parse_date_to_iso(date_str: &str) -> String {
    let trimmed = date_str.trim();
    if trimmed.is_empty() {
        return chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    }

    let cleaned =
        if let Some(pos) = trimmed.rfind('(') { trimmed[..pos].trim() } else { trimmed };

    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(cleaned) {
        return dt.with_timezone(&chrono::Utc).format("%Y-%m-%dT%H:%M:%SZ").to_string();
    }

    let formats = [
        "%a, %d %b %Y %H:%M:%S %z",
        "%d %b %Y %H:%M:%S %z",
        "%a, %d %b %Y %H:%M:%S %Z",
        "%Y-%m-%dT%H:%M:%S%z",
        "%Y-%m-%dT%H:%M:%S%.f%z",
    ];
    for fmt in &formats {
        if let Ok(dt) = chrono::DateTime::parse_from_str(cleaned, fmt) {
            return dt.with_timezone(&chrono::Utc).format("%Y-%m-%dT%H:%M:%SZ").to_string();
        }
    }

    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(cleaned) {
        return dt.with_timezone(&chrono::Utc).format("%Y-%m-%dT%H:%M:%SZ").to_string();
    }

    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub(crate) fn urlencode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char)
            }
            _ => result.push_str(&format!("%{byte:02X}")),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_message_id_case_insensitively() {
        let headers = vec![
            json!({"name": "Subject", "value": "Hi"}),
            json!({"name": "Message-ID", "value": "<abc@mail.gmail.com>"}),
        ];
        assert_eq!(extract_message_id(&headers), Some("<abc@mail.gmail.com>".to_string()));
        let headers2 = vec![json!({"name": "message-id", "value": "<x@y>"})];
        assert_eq!(extract_message_id(&headers2), Some("<x@y>".to_string()));
        assert_eq!(extract_message_id(&[]), None);
    }

    #[test]
    fn gmail_raw_includes_reply_headers_when_set() {
        let raw = build_gmail_raw_message(
            "bob@example.com",
            "Re: Hi",
            "<p>hello</p>",
            None,
            None,
            Some("<abc@mail.gmail.com>"),
        );
        assert!(raw.contains("In-Reply-To: <abc@mail.gmail.com>"));
        assert!(raw.contains("References: <abc@mail.gmail.com>"));
        assert!(raw.contains("To: bob@example.com"));
        // Reply headers belong in the header block, before the body separator.
        assert!(raw.find("In-Reply-To").unwrap() < raw.find("\r\n\r\n").unwrap());

        let raw2 = build_gmail_raw_message("bob@example.com", "Hi", "<p>x</p>", None, None, None);
        assert!(!raw2.contains("In-Reply-To"));
        assert!(!raw2.contains("References"));
    }

    #[test]
    fn an_injected_message_id_is_refused() {
        assert!(valid_reply_message_id("<abc@mail.gmail.com>"));
        assert!(!valid_reply_message_id(""));
        assert!(!valid_reply_message_id("<a@b>\r\nBcc: attacker@evil.com"));

        let raw = build_gmail_raw_message(
            "bob@example.com",
            "Hi",
            "<p>x</p>",
            None,
            None,
            Some("<a@b>\r\nBcc: attacker@evil.com"),
        );
        assert!(!raw.contains("attacker@evil.com"), "injected header must not survive");
    }

    #[test]
    fn a_gmail_message_parses_into_the_portable_shape() {
        let msg = json!({
            "id": "m1", "threadId": "t1", "snippet": "hello there",
            "internalDate": "1722770400000",
            "labelIds": ["INBOX", "UNREAD"],
            "payload": {
                "mimeType": "multipart/alternative",
                "headers": [
                    {"name": "Subject", "value": "Greetings"},
                    {"name": "From", "value": "Priya Nair <priya@example.edu>"},
                    {"name": "To", "value": "luke@example.edu, Bob <bob@example.com>"},
                    {"name": "Message-ID", "value": "<mid@x>"}
                ],
                "parts": [
                    {"mimeType": "text/plain", "body": {"data": "aGVsbG8gdGhlcmU="}}
                ]
            }
        });
        let email = parse_gmail_message(&msg);
        assert_eq!(email.id, "gmail-m1");
        assert_eq!(email.from_name, "Priya Nair");
        assert_eq!(email.from_address, "priya@example.edu");
        assert_eq!(email.to_addresses, vec!["luke@example.edu", "bob@example.com"]);
        assert_eq!(email.body_text, "hello there");
        assert!(!email.is_read);
        assert_eq!(email.message_id.as_deref(), Some("<mid@x>"));
        assert_eq!(email.date_received, "2024-08-04T11:20:00Z");
    }
}
