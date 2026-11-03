//! The Gmail client, scoped to what an agent doing on-demand work needs:
//! search, thread reads, send, and the triage verbs — archive, read-state,
//! spam, trash. There is deliberately no history/sync machinery and no local
//! cache; `threads.get` fetches a conversation directly instead.

use base64::{engine::general_purpose::URL_SAFE, Engine};
use serde_json::Value;

use crate::http::send_with_retry;
use crate::types::{authentication_results, first_header, Email, MailError};

#[derive(Clone)]
pub struct GmailProvider {
    access_token: String,
    client: reqwest::Client,
}

impl GmailProvider {
    pub fn new(access_token: String) -> Self {
        Self {
            access_token,
            client: crate::http::client(),
        }
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.access_token)
    }

    async fn get_json(&self, url: &str) -> Result<Value, MailError> {
        let resp = send_with_retry(
            self.client
                .get(url)
                .header("Authorization", self.auth_header()),
        )
        .await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(MailError::ApiError {
                status,
                message: body,
            });
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

    /// Fetch a whole conversation in one round trip, rather than
    /// reassembling it from per-message reads.
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
        let raw_message = build_gmail_raw_message(to, subject, body, cc, bcc, in_reply_to)?;
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
            return Err(MailError::ApiError {
                status,
                message: body,
            });
        }
        let json: Value = resp.json().await?;
        Ok(json["id"].as_str().unwrap_or_default().to_string())
    }

    // ── triage ────────────────────────────────────────────────────────────
    //
    // All of these operate on a **thread**, not a message, because that is
    // the unit the model was handed: every search row carries a `thread_id`,
    // and archiving four of a conversation's five messages is not a state a
    // person ever wants. Gmail's `threads.modify` applies the label change to
    // every message in the thread in one call, which is also one quota unit
    // instead of five.
    //
    // Archive, read-state and spam use label arithmetic; trash and untrash
    // have dedicated endpoints. The labels below are Gmail's system ids.

    /// Add and remove labels across every message in a thread.
    async fn modify_thread(
        &self,
        thread_id: &str,
        add: &[&str],
        remove: &[&str],
    ) -> Result<(), MailError> {
        let resp = send_with_retry(self.modify_thread_request(thread_id, add, remove)).await?;
        Self::ok_or_err(resp).await
    }

    fn modify_thread_request(
        &self,
        thread_id: &str,
        add: &[&str],
        remove: &[&str],
    ) -> reqwest::RequestBuilder {
        self.client
            .post(format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/threads/{}/modify",
                urlencode(thread_id)
            ))
            .header("Authorization", self.auth_header())
            .json(&serde_json::json!({"addLabelIds": add, "removeLabelIds": remove}))
    }

    fn restore_thread_request(&self, thread_id: &str, from_trash: bool) -> reqwest::RequestBuilder {
        if from_trash {
            self.client
                .post(format!(
                    "https://gmail.googleapis.com/gmail/v1/users/me/threads/{}/untrash",
                    urlencode(thread_id)
                ))
                .header("Authorization", self.auth_header())
                .header("Content-Length", "0")
        } else {
            self.modify_thread_request(thread_id, &["INBOX"], &[])
        }
    }

    /// Undo archive by adding INBOX to every message in the thread.
    pub async fn unarchive_thread(&self, thread_id: &str) -> Result<(), MailError> {
        Self::ok_or_err(send_with_retry(self.restore_thread_request(thread_id, false)).await?).await
    }

    /// Remove every message in the thread from trash. Gmail restores its
    /// labels; this does not promise to put previously archived mail in INBOX.
    pub async fn untrash_thread(&self, thread_id: &str) -> Result<(), MailError> {
        Self::ok_or_err(send_with_retry(self.restore_thread_request(thread_id, true)).await?).await
    }

    /// Remove the thread from the inbox. Gmail has no archive folder — being
    /// archived *is* not carrying `INBOX` — so this is a label removal and
    /// nothing is moved or lost.
    pub async fn archive_thread(&self, thread_id: &str) -> Result<(), MailError> {
        self.modify_thread(thread_id, &[], &["INBOX"]).await
    }

    /// Mark every message in the thread read, or unread.
    pub async fn set_thread_read(&self, thread_id: &str, read: bool) -> Result<(), MailError> {
        if read {
            self.modify_thread(thread_id, &[], &["UNREAD"]).await
        } else {
            self.modify_thread(thread_id, &["UNREAD"], &[]).await
        }
    }

    /// Report the thread as spam. Removing `INBOX` alongside is what makes it
    /// leave the inbox rather than sit there labelled — Gmail does not imply
    /// one from the other. This is the one triage verb with an effect outside
    /// the mailbox: it trains the provider's filter, which is why it is its
    /// own verb rather than a label argument someone could pass by accident.
    pub async fn spam_thread(&self, thread_id: &str) -> Result<(), MailError> {
        self.modify_thread(thread_id, &["SPAM"], &["INBOX"]).await
    }

    /// Move the thread to the trash, where the user can retrieve it. Note
    /// what this is not: `gmail.modify` cannot permanently delete, by
    /// deliberate choice of scope, so the destructive verb the model can
    /// reach is the reversible one.
    pub async fn trash_thread(&self, thread_id: &str) -> Result<(), MailError> {
        let resp = send_with_retry(
            self.client
                .post(format!(
                    "https://gmail.googleapis.com/gmail/v1/users/me/threads/{thread_id}/trash"
                ))
                .header("Authorization", self.auth_header())
                .header("Content-Length", "0"),
        )
        .await?;
        Self::ok_or_err(resp).await
    }

    /// Collapse a response into unit-or-error. Triage verbs return nothing
    /// worth reading on success, and the body on failure is the whole point.
    async fn ok_or_err(resp: reqwest::Response) -> Result<(), MailError> {
        if resp.status().is_success() {
            return Ok(());
        }
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        Err(MailError::ApiError {
            status,
            message: body,
        })
    }

    /// The authenticated account's address — doubles as the auth smoke test.
    pub async fn profile_address(&self) -> Result<String, MailError> {
        let json = self
            .get_json("https://gmail.googleapis.com/gmail/v1/users/me/profile")
            .await?;
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

/// Refuse a header value an extra header could be smuggled through. To, Cc,
/// Bcc and Subject are model-drafted — which makes them injection-reachable —
/// and a CR or LF inside one starts a new header line in the raw message
/// (`\r\nBcc: attacker@…`). Same rule as [`valid_reply_message_id`], which
/// guards the one header that comes from *incoming* mail. Refused rather than
/// stripped: silently rewriting an attack hides it, where an error surfaces
/// it to the model and the user.
fn checked_header(name: &str, value: &str) -> Result<(), MailError> {
    if value.chars().any(|c| c == '\r' || c == '\n' || c == '\0') {
        return Err(MailError::InvalidInput(format!(
            "the {name} value contains a line break; mail header values must be a single line"
        )));
    }
    Ok(())
}

pub(crate) fn build_gmail_raw_message(
    to: &str,
    subject: &str,
    body: &str,
    cc: Option<&str>,
    bcc: Option<&str>,
    in_reply_to: Option<&str>,
) -> Result<String, MailError> {
    checked_header("to", to)?;
    checked_header("subject", subject)?;
    let mut headers = format!("To: {to}\r\n");
    if let Some(cc_val) = cc {
        if !cc_val.is_empty() {
            checked_header("cc", cc_val)?;
            headers.push_str(&format!("Cc: {cc_val}\r\n"));
        }
    }
    if let Some(bcc_val) = bcc {
        if !bcc_val.is_empty() {
            checked_header("bcc", bcc_val)?;
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
    Ok(headers)
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
    let date_received = if let Some(internal_date) = msg["internalDate"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
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
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
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
        auto_submitted: first_header(headers.map(Vec::as_slice), "auto-submitted"),
        precedence: first_header(headers.map(Vec::as_slice), "precedence"),
        list_id: first_header(headers.map(Vec::as_slice), "list-id"),
        reply_to: first_header(headers.map(Vec::as_slice), "reply-to"),
        // A separate pass preserves EVERY instance, including duplicates.
        // The assign-every-time loop above must never handle this header.
        authentication_results: authentication_results(headers.map(Vec::as_slice)),
        headers_complete: headers
            .filter(|hs| {
                hs.iter()
                    .all(|h| h["name"].is_string() && h["value"].is_string())
            })
            .map(|_| true),
        is_draft: msg["labelIds"]
            .as_array()
            .map(|labels| labels.iter().any(|label| label == "DRAFT")),
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

    let cleaned = if let Some(pos) = trimmed.rfind('(') {
        trimmed[..pos].trim()
    } else {
        trimmed
    };

    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(cleaned) {
        return dt
            .with_timezone(&chrono::Utc)
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
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
            return dt
                .with_timezone(&chrono::Utc)
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string();
        }
    }

    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(cleaned) {
        return dt
            .with_timezone(&chrono::Utc)
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
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
        assert_eq!(
            extract_message_id(&headers),
            Some("<abc@mail.gmail.com>".to_string())
        );
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
        )
        .unwrap();
        assert!(raw.contains("In-Reply-To: <abc@mail.gmail.com>"));
        assert!(raw.contains("References: <abc@mail.gmail.com>"));
        assert!(raw.contains("To: bob@example.com"));
        // Reply headers belong in the header block, before the body separator.
        assert!(raw.find("In-Reply-To").unwrap() < raw.find("\r\n\r\n").unwrap());

        let raw2 =
            build_gmail_raw_message("bob@example.com", "Hi", "<p>x</p>", None, None, None).unwrap();
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
        )
        .unwrap();
        assert!(
            !raw.contains("attacker@evil.com"),
            "injected header must not survive"
        );
    }

    #[test]
    fn an_injected_addressing_or_subject_header_is_refused() {
        // Every one of these values is model-drafted, so a prompt injection
        // can reach them: a CR/LF inside any of them starts a new header line
        // in the raw message. The reply Message-ID was already guarded; these
        // were not, and body newlines must of course stay legal.
        let smuggle = "x\r\nBcc: attacker@evil.com";
        for (to, subject, cc, bcc) in [
            (smuggle, "Hi", None, None),
            ("bob@example.com", smuggle, None, None),
            ("bob@example.com", "Hi", Some(smuggle), None),
            ("bob@example.com", "Hi", None, Some(smuggle)),
        ] {
            let err = build_gmail_raw_message(to, subject, "<p>x</p>", cc, bcc, None)
                .expect_err("a header value with a line break must be refused");
            assert!(err.to_string().contains("line break"), "{err}");
        }

        let raw = build_gmail_raw_message(
            "bob@example.com",
            "Hi",
            "<p>line one</p>\r\n<p>line two</p>",
            None,
            None,
            None,
        )
        .unwrap();
        assert!(raw.contains("line two"), "the body keeps its newlines");
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
                    {"name": "To", "value": "robin@example.edu, Bob <bob@example.com>"},
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
        assert_eq!(
            email.to_addresses,
            vec!["robin@example.edu", "bob@example.com"]
        );
        assert_eq!(email.body_text, "hello there");
        assert!(!email.is_read);
        assert_eq!(email.message_id.as_deref(), Some("<mid@x>"));
        assert_eq!(email.date_received, "2024-08-04T11:20:00Z");
    }
    fn with_authentication_results(values: &[&str]) -> Email {
        let headers: Vec<_> = values
            .iter()
            .map(|value| {
                json!({
                    "name":"aUtHeNtIcAtIoN-ReSuLtS", "value":value
                })
            })
            .collect();
        parse_gmail_message(
            &json!({"id":"message-1", "threadId":"thread-1", "payload":{"headers":headers}}),
        )
    }

    #[test]
    fn gmail_authentication_results_expose_missing_foreign_duplicate_and_single_claims() {
        assert!(with_authentication_results(&[])
            .authentication_results
            .is_none());
        assert!(parse_gmail_message(&json!({}))
            .authentication_results
            .is_none());
        assert_eq!(parse_gmail_message(&json!({})).headers_complete, None);
        let genuine = "mx.google.com; dmarc=pass header.from=example.edu";
        let forged = "mx.google.com; dmarc=fail header.from=example.com";
        let foreign = "relay.example.com; dmarc=pass header.from=example.edu";
        for (values, claims) in [
            (vec![foreign], 0),
            (vec![genuine, forged], 2),
            (vec![forged, genuine], 2),
            (vec![genuine], 1),
            (vec![foreign, genuine], 1),
        ] {
            let email = with_authentication_results(&values);
            let instances = email.authentication_results.unwrap();
            assert_eq!(instances.len(), values.len());
            assert_eq!(
                instances
                    .iter()
                    .filter(|r| r.authserv_id.as_deref() == Some("mx.google.com"))
                    .count(),
                claims
            );
            // The foreign topmost field must remain topmost; callers cannot
            // skip to a later field just because its id looks familiar.
            assert_eq!(instances.first().unwrap().value, values[0]);
            assert_eq!(
                instances
                    .iter()
                    .map(|r| r.value.as_str())
                    .collect::<Vec<_>>(),
                values
            );
            assert_eq!(email.headers_complete, Some(true));
        }
    }

    #[test]
    fn gmail_exposes_rule_headers_and_provider_ids_without_collapsing_absence() {
        let email = parse_gmail_message(&json!({
            "id":"provider-message", "threadId":"provider-thread", "labelIds":["INBOX"],
            "payload":{"headers":[
                {"name":"Message-ID", "value":"<sender-controlled@example.com>"},
                {"name":"Auto-Submitted", "value":"auto-generated"},
                {"name":"Precedence", "value":"bulk"},
                {"name":"List-Id", "value":"<notices.example.edu>"},
                {"name":"Reply-To", "value":"reply@example.com"}
            ]}
        }));
        assert_eq!(email.provider_id, "provider-message");
        assert_eq!(email.thread_id.as_deref(), Some("provider-thread"));
        assert_eq!(email.auto_submitted.as_deref(), Some("auto-generated"));
        assert_eq!(email.precedence.as_deref(), Some("bulk"));
        assert_eq!(email.list_id.as_deref(), Some("<notices.example.edu>"));
        assert_eq!(email.reply_to.as_deref(), Some("reply@example.com"));
        assert!(email.authentication_results.is_none());
        assert_eq!(email.is_draft, Some(false));
        assert_eq!(
            parse_gmail_message(&json!({"labelIds":["DRAFT"]})).is_draft,
            Some(true)
        );
        let missing = parse_gmail_message(&json!({}));
        assert!(
            missing.auto_submitted.is_none()
                && missing.precedence.is_none()
                && missing.list_id.is_none()
                && missing.reply_to.is_none()
        );
    }

    #[test]
    fn gmail_restore_requests_invert_archive_and_trash() {
        let provider = GmailProvider::new("fixture-token".into());
        let request = provider
            .restore_thread_request("thread/1", false)
            .build()
            .unwrap();
        assert_eq!(request.method(), reqwest::Method::POST);
        assert_eq!(
            request.url().as_str(),
            "https://gmail.googleapis.com/gmail/v1/users/me/threads/thread%2F1/modify"
        );
        let body: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(body, json!({"addLabelIds":["INBOX"], "removeLabelIds":[]}));
        let archive = provider
            .modify_thread_request("thread/1", &[], &["INBOX"])
            .build()
            .unwrap();
        let body: Value =
            serde_json::from_slice(archive.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(body, json!({"addLabelIds":[], "removeLabelIds":["INBOX"]}));
        let request = provider
            .restore_thread_request("thread/1", true)
            .build()
            .unwrap();
        assert_eq!(request.method(), reqwest::Method::POST);
        assert_eq!(
            request.url().as_str(),
            "https://gmail.googleapis.com/gmail/v1/users/me/threads/thread%2F1/untrash"
        );
        assert_eq!(request.headers()["Content-Length"], "0");
        assert!(request.body().is_none());
    }
}
