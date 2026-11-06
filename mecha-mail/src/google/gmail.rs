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
    #[cfg(test)]
    read_urls: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    access_token: String,
    client: reqwest::Client,
    #[cfg(test)]
    scripted_mutations: Option<std::sync::Arc<std::sync::Mutex<Vec<reqwest::Request>>>>,
    #[cfg(test)]
    scripted_reads: Option<std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<Value>>>>,
}

impl GmailProvider {
    pub fn new(access_token: String) -> Self {
        Self {
            #[cfg(test)]
            read_urls: Default::default(),
            access_token,
            client: crate::http::client(),
            #[cfg(test)]
            scripted_mutations: None,
            #[cfg(test)]
            scripted_reads: None,
        }
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.access_token)
    }

    async fn get_json(&self, url: &str) -> Result<Value, MailError> {
        #[cfg(test)]
        if let Some(reads) = &self.scripted_reads {
            self.read_urls.lock().unwrap().push(url.into());
            return Ok(reads
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected Gmail request"));
        }
        let resp = send_with_retry(
            self.client
                .get(url)
                .header("Authorization", self.auth_header()),
        )
        .await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(MailError::gmail_api_error(status, body));
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
                    Ok(json) => Some((idx, parse_gmail_message(HeaderRead::Full, &json))),
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

    fn next_list_token(
        page: &Value,
        seen: &mut std::collections::BTreeSet<String>,
    ) -> Result<Option<String>, MailError> {
        match page.get("nextPageToken") {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(token)) if !token.is_empty() && seen.insert(token.clone()) => {
                Ok(Some(token.clone()))
            }
            _ => Err(MailError::ParseError(
                "invalid or repeated Gmail page token".into(),
            )),
        }
    }

    fn list_messages(page: &Value) -> Result<&[Value], MailError> {
        match page.get("messages") {
            Some(Value::Array(messages)) => Ok(messages),
            // Gmail omits messages for an empty result set. A continued page
            // or an explicit non-array is not evidence of an empty mailbox.
            None if page.get("nextPageToken").is_none_or(Value::is_null)
                && page
                    .get("resultSizeEstimate")
                    .is_none_or(|value| value.as_u64() == Some(0)) =>
            {
                Ok(&[])
            }
            _ => Err(MailError::ParseError("invalid Gmail messages page".into())),
        }
    }

    /// Search: list matching message ids (cheap), then fetch them
    /// concurrently. `query` is Gmail search syntax (`from:`, `after:`, …).
    pub async fn search(&self, query: &str, max_results: u32) -> Result<Vec<Email>, MailError> {
        let page_size = max_results.clamp(1, 100);
        let mut message_ids: Vec<String> = Vec::new();
        let mut page_token: Option<String> = None;
        let mut tokens = std::collections::BTreeSet::new();

        loop {
            let mut list_url = format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/messages?maxResults={page_size}&q={}",
                urlencode(query)
            );
            if let Some(ref token) = page_token {
                list_url.push_str(&format!("&pageToken={}", urlencode(token)));
            }

            let list_json = self.get_json(&list_url).await?;
            {
                let refs = Self::list_messages(&list_json)?;
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
            match Self::next_list_token(&list_json, &mut tokens)? {
                Some(token) if message_ids.len() < max_results as usize => {
                    page_token = Some(token);
                }
                _ => break,
            }
        }

        Ok(self.get_messages_concurrently(&message_ids).await)
    }

    /// Recent inbox conversations, using the engine's clock reading.
    pub async fn rule_thread_ids(
        &self,
        max_results: u32,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<String>, MailError> {
        Ok(self
            .rule_message_hits(max_results, since, true)
            .await?
            .into_iter()
            .map(|hit| hit.thread_id)
            .collect())
    }

    /// Separate read-only test surface: all folders, never the live inbox scope.
    pub async fn rule_test_hits(
        &self,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<crate::rules::RuleMessageHit>, MailError> {
        self.rule_message_hits(u32::MAX, since, false).await
    }

    async fn rule_message_hits(
        &self,
        max_results: u32,
        since: chrono::DateTime<chrono::Utc>,
        inbox_only: bool,
    ) -> Result<Vec<crate::rules::RuleMessageHit>, MailError> {
        let mut hits = Vec::new();
        let mut seen = 0;
        let mut page_token: Option<String> = None;
        let mut tokens = std::collections::BTreeSet::new();
        let mut pages = 0;
        while seen < max_results {
            crate::rules::check_rule_page(pages)?;
            pages += 1;
            let size = (max_results - seen).min(100);
            let query = format!(
                "{}after:{}",
                if inbox_only {
                    "in:inbox "
                } else {
                    "in:anywhere "
                },
                since.timestamp()
            );
            let mut url = format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/messages?maxResults={size}&q={}",
                urlencode(&query)
            );
            if !inbox_only {
                url.push_str("&includeSpamTrash=true");
            }
            if let Some(token) = &page_token {
                url.push_str(&format!("&pageToken={}", urlencode(token)));
            }
            let page = self.get_json(&url).await?;
            for message in Self::list_messages(&page)?
                .iter()
                .take((max_results - seen) as usize)
            {
                seen += 1;
                let id = message["id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| MailError::ParseError("rule message has no id".into()))?;
                let thread = message["threadId"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| MailError::ParseError("rule message has no thread id".into()))?;
                hits.push(crate::rules::RuleMessageHit {
                    id: id.into(),
                    thread_id: thread.into(),
                });
            }
            match Self::next_list_token(&page, &mut tokens)? {
                Some(token) => page_token = Some(token),
                None => break,
            }
        }
        Ok(hits)
    }

    pub async fn rule_test_message(&self, id: &str) -> Result<(Email, Option<bool>), MailError> {
        let raw = self.get_message(id).await?;
        let in_inbox = raw["labelIds"]
            .as_array()
            .map(|labels| labels.iter().any(|l| l == "INBOX"));
        Ok((parse_gmail_message(HeaderRead::Full, &raw), in_inbox))
    }

    /// Fetch a whole conversation in one round trip, rather than
    /// reassembling it from per-message reads.
    pub async fn get_thread(&self, thread_id: &str) -> Result<Vec<Email>, MailError> {
        let json = self
            .get_json(&format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/threads/{}?format=full",
                urlencode(thread_id)
            ))
            .await?;
        let messages = json["messages"]
            .as_array()
            .ok_or_else(|| MailError::ParseError("thread has no messages array".into()))?;
        let mut emails: Vec<Email> = messages
            .iter()
            .map(|message| parse_gmail_message(HeaderRead::Full, message))
            .collect();
        // Oldest first is a promise callers build on — the unified reply
        // tool answers `last()` as "the newest message" — and the API's
        // array order is observed, not documented. Fractional precision varies.
        emails.sort_by_key(Email::received_instant);
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
            return Err(MailError::gmail_api_error(status, body));
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
        self.mutate(self.modify_thread_request(thread_id, add, remove))
            .await
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

    /// Rule execution always addresses its authorized message, never a thread.
    pub async fn archive_message(&self, id: &str) -> Result<(), MailError> {
        self.mutate(self.modify_message_request(id, &[], &["INBOX"]))
            .await
    }
    pub async fn unarchive_message(&self, id: &str) -> Result<(), MailError> {
        if self
            .rule_message_labels(id)
            .await?
            .iter()
            .any(|label| label == "INBOX")
        {
            return Ok(());
        }
        self.mutate(self.modify_message_request(id, &["INBOX"], &[]))
            .await
    }
    pub async fn untrash_message(&self, id: &str) -> Result<(), MailError> {
        if !self
            .rule_message_labels(id)
            .await?
            .iter()
            .any(|label| label == "TRASH")
        {
            return Ok(());
        }
        self.mutate(
            self.client
                .post(format!(
                    "https://gmail.googleapis.com/gmail/v1/users/me/messages/{}/untrash",
                    urlencode(id)
                ))
                .header("Authorization", self.auth_header())
                .header("Content-Length", "0"),
        )
        .await
    }
    async fn rule_message_labels(&self, id: &str) -> Result<Vec<String>, MailError> {
        let raw = self
            .get_json(&format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/messages/{}?format=minimal",
                urlencode(id)
            ))
            .await?;
        if raw["id"].as_str() != Some(id) {
            return Err(MailError::ParseError(
                "restore read returned a different message id".into(),
            ));
        }
        raw["labelIds"]
            .as_array()
            .ok_or_else(|| MailError::ParseError("restore read has unknown labels".into()))?
            .iter()
            .map(|label| {
                label.as_str().map(str::to_owned).ok_or_else(|| {
                    MailError::ParseError("restore read has malformed labels".into())
                })
            })
            .collect()
    }
    pub async fn set_message_read(&self, id: &str, read: bool) -> Result<(), MailError> {
        let (add, remove): (&[&str], &[&str]) = if read {
            (&[], &["UNREAD"])
        } else {
            (&["UNREAD"], &[])
        };
        self.mutate(self.modify_message_request(id, add, remove))
            .await
    }
    pub async fn trash_message(&self, id: &str) -> Result<(), MailError> {
        self.mutate(
            self.client
                .post(format!(
                    "https://gmail.googleapis.com/gmail/v1/users/me/messages/{}/trash",
                    urlencode(id)
                ))
                .header("Authorization", self.auth_header())
                .header("Content-Length", "0"),
        )
        .await
    }
    fn modify_message_request(
        &self,
        id: &str,
        add: &[&str],
        remove: &[&str],
    ) -> reqwest::RequestBuilder {
        self.client
            .post(format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/messages/{}/modify",
                urlencode(id)
            ))
            .header("Authorization", self.auth_header())
            .json(&serde_json::json!({"addLabelIds":add, "removeLabelIds":remove}))
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
    /// This does not clear TRASH or SPAM: success does not make those messages
    /// visible in Inbox. Use `untrash_thread` for trashed mail.
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
        self.mutate(
            self.client
                .post(format!(
                    "https://gmail.googleapis.com/gmail/v1/users/me/threads/{}/trash",
                    urlencode(thread_id)
                ))
                .header("Authorization", self.auth_header())
                .header("Content-Length", "0"),
        )
        .await
    }
    async fn mutate(&self, request: reqwest::RequestBuilder) -> Result<(), MailError> {
        #[cfg(test)]
        if let Some(requests) = &self.scripted_mutations {
            requests.lock().unwrap().push(request.build()?);
            return Ok(());
        }
        Self::ok_or_err(send_with_retry(request).await?).await
    }

    /// Collapse a response into unit-or-error. Triage verbs return nothing
    /// worth reading on success, and the body on failure is the whole point.
    async fn ok_or_err(resp: reqwest::Response) -> Result<(), MailError> {
        if resp.status().is_success() {
            return Ok(());
        }
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        Err(MailError::gmail_api_error(status, body))
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

/// What the caller requested, not what the returned JSON appears to contain.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum HeaderRead {
    /// `format=full`, without a field mask that restricts headers.
    Full,
    /// Metadata, restricted fields, or unknown request provenance.
    #[default]
    Partial,
}

/// Parse a Gmail API message JSON using the caller's request provenance.
pub(crate) fn parse_gmail_message(header_read: HeaderRead, msg: &Value) -> Email {
    let provider_id = msg["id"].as_str().unwrap_or_default().to_string();
    let thread_id = msg["threadId"].as_str().map(|s| s.to_string());
    let snippet = msg["snippet"].as_str().unwrap_or_default().to_string();

    let headers = msg["payload"]["headers"].as_array();
    let mut subject = String::new();
    let mut from_raw = String::new();
    let mut from_count = 0;
    let mut subject_count = 0;
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
                "subject" => {
                    subject_count += 1;
                    subject = value.to_string();
                }
                "from" => {
                    from_count += 1;
                    from_raw = value.to_string();
                }
                "to" => append_address_header(&mut to_raw, value),
                "cc" => append_address_header(&mut cc_raw, value),
                "bcc" => append_address_header(&mut bcc_raw, value),
                "date" => date_str = value.to_string(),
                "list-unsubscribe" => list_unsubscribe = Some(value.to_string()),
                _ => {}
            }
        }
    }
    let message_id = headers.and_then(|h| extract_message_id(h));

    let (from_name, from_address) = if from_count == 1 {
        parse_email_address(&from_raw)
    } else {
        Default::default()
    };

    let sender_known = Some(!from_address.is_empty());
    let (from_name, from_address) = if sender_known == Some(true) {
        (from_name, from_address)
    } else {
        display_email_address(&from_raw)
    };

    let to = parse_address_list(&to_raw);
    let cc = parse_address_list(&cc_raw);
    let bcc = parse_address_list(&bcc_raw);
    let recipients_parsed = to.is_some() && cc.is_some() && bcc.is_some();
    let display_list = |raw: &str| {
        raw.split(',')
            .map(|part| display_email_address(part).1)
            .filter(|s| !s.is_empty())
            .collect()
    };
    let to_addresses = to.unwrap_or_else(|| display_list(&to_raw));
    let cc_addresses = cc.unwrap_or_else(|| display_list(&cc_raw));
    let bcc_addresses = bcc.unwrap_or_else(|| display_list(&bcc_raw));

    // Prefer internalDate (epoch millis) over the header, which lies more.
    let provider_arrival_at = msg["internalDate"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .and_then(chrono::DateTime::from_timestamp_millis);
    let parsed_date = provider_arrival_at
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true))
        .or_else(|| parse_date_to_iso(&date_str));
    let date_known = Some(parsed_date.is_some());
    let date_received =
        parsed_date.unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());

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

    let headers_complete = headers
        .filter(|hs| {
            header_read == HeaderRead::Full
                && !hs.is_empty()
                && hs
                    .iter()
                    .all(|h| h["name"].is_string() && h["value"].is_string())
        })
        .map(|_| true);

    Email {
        id: format!("gmail-{provider_id}"),
        provider: "gmail".to_string(),
        provider_id,
        thread_id,
        message_id,
        subject,
        subject_is_raw: true,
        from_address,
        sender_known,
        from_name,
        to_addresses,
        cc_addresses,
        bcc_addresses,
        date_received,
        date_known,
        provider_arrival_at,
        attachment_presence: attachment_presence(&msg["payload"]),
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
        headers: crate::types::header_values(headers.map(Vec::as_slice)),
        authentication_results: authentication_results(headers.map(Vec::as_slice)),
        subject_complete: if subject_count > 1 {
            Some(false)
        } else {
            headers_complete
        },
        recipients_complete: if recipients_parsed {
            headers_complete
        } else {
            Some(false)
        },
        headers_complete,
        is_draft: msg["labelIds"]
            .as_array()
            .filter(|labels| labels.iter().all(Value::is_string))
            .map(|labels| labels.iter().any(|label| label == "DRAFT")),
        direction_evidence: msg["labelIds"]
            .as_array()
            .filter(|labels| labels.iter().all(Value::is_string))
            .map(|labels| {
                if labels.iter().any(|label| label == "DRAFT") {
                    crate::types::DirectionEvidence::Draft
                } else if labels.iter().any(|label| label == "SENT") {
                    crate::types::DirectionEvidence::Sent
                } else {
                    crate::types::DirectionEvidence::NotSent
                }
            }),
    }
}

fn append_address_header(target: &mut String, value: &str) {
    if !target.is_empty() {
        target.push_str(", ");
    }
    target.push_str(value);
}

/// A From is exactly one mailbox. A list, group, duplicate field or syntax
/// error is unknown (empty address), never a best-effort identity.
fn parse_email_address(raw: &str) -> (String, String) {
    let Some(address) = mailboxes(raw).and_then(|list| list.extract_single_info()) else {
        return Default::default();
    };
    (address.display_name.unwrap_or_default(), address.addr)
}

/// Legacy display/reply fallback only. These strings are not rule evidence.
fn display_email_address(raw: &str) -> (String, String) {
    let raw = raw.trim();
    if let Some(start) = raw.find('<') {
        if let Some(end) = raw.find('>').filter(|end| *end > start) {
            return (
                raw[..start].trim().trim_matches('"').to_owned(),
                raw[start + 1..end].trim().to_owned(),
            );
        }
    }
    (String::new(), raw.to_owned())
}

fn parse_address_list(raw: &str) -> Option<Vec<String>> {
    Some(
        mailboxes(raw)?
            .into_inner()
            .into_iter()
            .flat_map(|entry| match entry {
                mailparse::MailAddr::Single(address) => vec![address.addr],
                mailparse::MailAddr::Group(group) => group
                    .addrs
                    .into_iter()
                    .map(|address| address.addr)
                    .collect(),
            })
            .collect(),
    )
}

fn mailboxes(raw: &str) -> Option<mailparse::MailAddrList> {
    // mailparse handles mailbox/list grammar and encoded display names. Its
    // comment parser is deliberately permissive; remove balanced RFC comments
    // first, including nesting and quoted pairs, without touching quoted names.
    let raw = without_comments(raw)?;
    if raw.trim().starts_with(',') || raw.trim().ends_with(',') {
        return None;
    }
    let bytes = format!("From: {raw}");
    let (header, consumed) = mailparse::parse_header(bytes.as_bytes()).ok()?;
    if consumed != bytes.len() {
        return None;
    }
    let list = mailparse::addrparse_header(&header).ok()?;
    let valid = |address: &mailparse::SingleInfo| {
        mecha_core::mail_rules::engine::parsed_address(&address.addr).is_some()
    };
    list.iter()
        .all(|entry| match entry {
            mailparse::MailAddr::Single(address) => valid(address),
            mailparse::MailAddr::Group(group) => group.addrs.iter().all(valid),
        })
        .then_some(list)
}

fn without_comments(raw: &str) -> Option<String> {
    let mut result = String::new();
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    let mut group = false;
    for c in raw.chars() {
        if c.is_control() && !matches!(c, '\t' | '\r' | '\n') {
            return None;
        }
        if escaped {
            if depth == 0 {
                result.push(c);
            }
            escaped = false;
        } else if c == '\\' && (quoted || depth > 0) {
            escaped = true;
            if depth == 0 {
                result.push(c);
            }
        } else if depth > 0 {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
        } else if c == '"' {
            quoted = !quoted;
            result.push(c);
        } else if c == '(' && !quoted {
            depth = 1;
            result.push(' ');
        } else if c == ':' && !quoted {
            if group {
                return None;
            }
            group = true;
            result.push(c);
        } else if c == ';' && !quoted {
            if !group {
                return None;
            }
            group = false;
            result.push(c);
        } else if c == ')' && !quoted {
            return None;
        } else {
            result.push(c);
        }
    }
    (depth == 0 && !quoted && !escaped && !group).then_some(result)
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

// Only a complete MIME tree can prove the absence of attachments. A positive
// attachment anywhere is sufficient even if a different part is incomplete.
fn attachment_presence(payload: &Value) -> Option<bool> {
    if check_has_attachments(payload) {
        return Some(true);
    }
    let mime = payload["mimeType"].as_str()?;
    payload["filename"].as_str()?;
    if mime.starts_with("multipart/") {
        let parts = payload["parts"].as_array()?;
        if parts.is_empty() || parts.iter().any(|p| attachment_presence(p).is_none()) {
            return None;
        }
    } else if !payload["body"].is_object() {
        return None;
    }
    Some(false)
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
fn parse_date_to_iso(date_str: &str) -> Option<String> {
    let trimmed = date_str.trim();
    if trimmed.is_empty() {
        return None;
    }

    let cleaned = if let Some(pos) = trimmed.rfind('(') {
        trimmed[..pos].trim()
    } else {
        trimmed
    };

    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(cleaned) {
        return Some(
            dt.with_timezone(&chrono::Utc)
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string(),
        );
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
            return Some(
                dt.with_timezone(&chrono::Utc)
                    .format("%Y-%m-%dT%H:%M:%SZ")
                    .to_string(),
            );
        }
    }

    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(cleaned) {
        return Some(
            dt.with_timezone(&chrono::Utc)
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string(),
        );
    }

    None
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
    #[tokio::test]
    async fn retrospective_listing_uses_all_mail_and_the_shared_page_ceiling() {
        let since = "2026-10-01T00:00:00Z".parse().unwrap();
        let mut provider = GmailProvider::new("fixture-token".into());
        provider.scripted_reads = Some(std::sync::Arc::new(std::sync::Mutex::new([
            json!({"messages":[{"id":"archived", "threadId":"thread-archived"}], "nextPageToken":"next+page"}),
            json!({"messages":[]})].into())));
        assert_eq!(
            provider.rule_test_hits(since).await.unwrap()[0].id,
            "archived"
        );
        let urls = provider.read_urls.lock().unwrap().clone();
        let first = reqwest::Url::parse(&urls[0]).unwrap();
        assert!(first
            .query_pairs()
            .any(|(k, v)| k == "q" && v == format!("in:anywhere after:{}", since.timestamp())));
        assert!(first
            .query_pairs()
            .any(|(k, v)| k == "includeSpamTrash" && v == "true"));
        let mut pages: Vec<_> = (0..100)
            .map(|n| json!({"messages":[], "nextPageToken":format!("distinct-{n}")}))
            .collect();
        pages.push(json!({"messages":[]}));
        provider.scripted_reads = Some(std::sync::Arc::new(std::sync::Mutex::new(pages.into())));
        assert!(provider
            .rule_test_hits(since)
            .await
            .unwrap_err()
            .to_string()
            .contains("page ceiling"));
    }

    #[tokio::test]
    async fn rule_restore_retry_is_a_noop_after_message_was_restored() {
        use mecha_core::mail_rules::{MessageRef, RestoreAction};
        for action in [RestoreAction::Unarchive, RestoreAction::Untrash] {
            let mut provider = GmailProvider::new("fixture-token".into());
            let writes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            provider.scripted_mutations = Some(writes.clone());
            provider.scripted_reads = Some(std::sync::Arc::new(std::sync::Mutex::new(
                [json!({"id":"authorized-message", "labelIds":["INBOX"]})].into(),
            )));
            let message = MessageRef {
                provider_message_id: "authorized-message".into(),
                ..Default::default()
            };
            crate::rules::gmail_restore(&provider, &message, action)
                .await
                .unwrap();
            assert!(
                writes.lock().unwrap().is_empty(),
                "already restored must be a nothing-to-do success"
            );
        }
    }

    #[tokio::test]
    async fn rule_restore_targets_only_the_authorized_message() {
        use mecha_core::mail_rules::{MessageRef, RestoreAction};
        for action in [
            RestoreAction::Unarchive,
            RestoreAction::Untrash,
            RestoreAction::Read,
            RestoreAction::Unread,
        ] {
            let mut provider = GmailProvider::new("fixture-token".into());
            let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            provider.scripted_mutations = Some(requests.clone());
            provider.scripted_reads = Some(std::sync::Arc::new(std::sync::Mutex::new([
                json!({"id":"authorized-message", "labelIds": if action == RestoreAction::Untrash { vec!["TRASH"] } else { vec!["UNREAD"] }}),
                json!({"id":"authorized-message", "labelIds":["INBOX"]})
            ].into())));
            let message = MessageRef {
                provider_message_id: "authorized-message".into(),
                thread_id: "shared-thread".into(),
                ..Default::default()
            };
            crate::rules::gmail_restore(&provider, &message, action)
                .await
                .unwrap();
            if matches!(action, RestoreAction::Unarchive | RestoreAction::Untrash) {
                crate::rules::gmail_restore(&provider, &message, action)
                    .await
                    .unwrap();
            }
            let requests = requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert!(requests[0]
                .url()
                .path()
                .starts_with("/gmail/v1/users/me/messages/authorized-message/"));
            if action == RestoreAction::Untrash {
                assert!(requests[0].url().path().ends_with("/untrash"));
            }
            if action == RestoreAction::Unarchive {
                let body: Value =
                    serde_json::from_slice(requests[0].body().unwrap().as_bytes().unwrap())
                        .unwrap();
                assert_eq!(body["addLabelIds"], json!(["INBOX"]));
            }
        }
    }

    #[tokio::test]
    async fn distinct_empty_rule_pages_hit_a_counted_ceiling() {
        let mut provider = GmailProvider::new("fixture-token".into());
        let mut pages: Vec<_> = (0..100)
            .map(|n| json!({"messages":[], "nextPageToken":format!("page-{n}")}))
            .collect();
        pages.push(json!({"messages":[]}));
        provider.scripted_reads = Some(std::sync::Arc::new(std::sync::Mutex::new(pages.into())));
        let result = provider
            .rule_thread_ids(500, "2026-11-01T00:00:00Z".parse().unwrap())
            .await;
        assert!(
            matches!(result, Err(MailError::ParseError(ref why)) if why.contains("page ceiling")),
            "unbounded empty enumeration: {result:?}"
        );
        assert!(provider.read_urls.lock().unwrap().len() <= 100);
    }

    #[test]
    fn oversized_routing_address_keeps_display_but_is_not_sender_evidence() {
        let address = format!("{}@example.com", "a".repeat(243));
        let email = parse_gmail_message(
            HeaderRead::Full,
            &json!({"payload":{"headers":[{"name":"From", "value":address}]}}),
        );
        assert_eq!(email.from_address, address);
        assert_eq!(email.sender_known, Some(false));
    }

    #[tokio::test]
    async fn rule_listing_rejects_invalid_or_repeated_page_tokens() {
        for pages in [
            vec![serde_json::json!({"messages":[], "nextPageToken":false})],
            vec![serde_json::json!({"messages":[], "nextPageToken":""})],
            vec![serde_json::json!({"messages":{}, "nextPageToken":null})],
            vec![serde_json::json!({"resultSizeEstimate":5})],
            vec![
                serde_json::json!({"messages":[], "nextPageToken":"again"}),
                serde_json::json!({"messages":[], "nextPageToken":"again"}),
            ],
        ] {
            let mut provider = GmailProvider::new("fixture-token".into());
            let count = pages.len();
            provider.scripted_reads =
                Some(std::sync::Arc::new(std::sync::Mutex::new(pages.into())));
            let result = provider
                .rule_thread_ids(500, "2026-11-01T00:00:00Z".parse().unwrap())
                .await;
            assert!(
                matches!(result, Err(MailError::ParseError(_))),
                "incomplete enumeration became success: {result:?}"
            );
            assert_eq!(provider.read_urls.lock().unwrap().len(), count);
        }
    }

    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn rule_actions_address_one_message_never_the_thread() {
        use mecha_core::mail_rules::{ActionVerb, MessageRef};
        for action in [
            ActionVerb::Archive,
            ActionVerb::Trash,
            ActionVerb::Read,
            ActionVerb::Unread,
        ] {
            let mut provider = GmailProvider::new("fixture-token".into());
            let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            provider.scripted_mutations = Some(requests.clone());
            let message = MessageRef {
                thread_id: "conversation-alpha".into(),
                provider_message_id: "authorized-message".into(),
                ..Default::default()
            };
            crate::rules::gmail_action(&provider, &message, action)
                .await
                .unwrap();
            let requests = requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert!(requests[0]
                .url()
                .path()
                .starts_with("/gmail/v1/users/me/messages/authorized-message/"));
        }
    }

    #[tokio::test]
    async fn rule_thread_listing_paginates_and_bounds_message_hits() {
        let mut provider = GmailProvider::new("fixture-token".into());
        provider.scripted_reads = Some(std::sync::Arc::new(std::sync::Mutex::new([
            serde_json::json!({"messages":[{"id":"alpha","threadId":"thread-alpha"}], "nextPageToken":"page+next"}),
            serde_json::json!({"messages":[{"id":"beta","threadId":"thread-alpha"},
                {"id":"gamma","threadId":"thread-gamma"}, {"id":"excess","threadId":"thread-excess"}],
                "nextPageToken":"never-requested"})].into())));
        assert_eq!(
            provider
                .rule_thread_ids(3, "2026-11-01T00:00:00Z".parse().unwrap())
                .await
                .unwrap(),
            ["thread-alpha", "thread-alpha", "thread-gamma"]
        );
        let urls = provider.read_urls.lock().unwrap();
        assert_eq!(urls.len(), 2);
        for url in urls.iter() {
            assert!(reqwest::Url::parse(url)
                .unwrap()
                .path()
                .ends_with("/messages"));
            assert!(!url.contains("format="));
        }
        let second = reqwest::Url::parse(&urls[1]).unwrap();
        assert!(second
            .query_pairs()
            .any(|(k, v)| k == "pageToken" && v == "page+next"));
        assert!(second
            .query_pairs()
            .any(|(k, v)| k == "maxResults" && v == "2"));
    }

    #[tokio::test]
    async fn rule_thread_listing_never_fetches_full_messages() {
        let mut provider = GmailProvider::new("fixture-token".into());
        provider.scripted_reads = Some(std::sync::Arc::new(std::sync::Mutex::new(
            [
                serde_json::json!({"messages":[{"id":"message-alpha","threadId":"thread-alpha"}]}),
                serde_json::json!({"id":"message-alpha","threadId":"thread-alpha"}),
            ]
            .into(),
        )));
        let ids = provider
            .rule_thread_ids(500, "2026-11-01T00:00:00Z".parse().unwrap())
            .await
            .unwrap();
        assert_eq!(ids, ["thread-alpha"]);
        let urls = provider.read_urls.lock().unwrap();
        assert_eq!(
            urls.len(),
            1,
            "listing must not fetch each full message: {urls:?}"
        );
        assert!(!urls[0].contains("format=full"));
        assert!(reqwest::Url::parse(&urls[0])
            .unwrap()
            .query_pairs()
            .any(|(k, v)| k == "q" && v.contains("in:inbox") && v.contains("after:1793491200")));
    }

    #[tokio::test]
    async fn mail_reply_uses_newest_message_with_mixed_timestamp_precision() {
        let whole: chrono::DateTime<chrono::Utc> = "2026-11-03T12:00:00Z".parse().unwrap();
        let fraction = whole + chrono::Duration::milliseconds(100);
        let make = |id: &str, sender: &str, at: chrono::DateTime<chrono::Utc>| serde_json::json!({"id":id, "threadId":"thread-alpha", "internalDate":at.timestamp_millis().to_string(), "payload":{"headers":[{"name":"From","value":sender},{"name":"Message-ID","value":format!("<{id}@example.com>")}]}});
        let mut provider = GmailProvider::new("fixture-token".into());
        provider.scripted_reads = Some(std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::from([
                serde_json::json!({"messages":[make("newer", "newer@example.edu", fraction), make("older", "older@example.edu", whole)]}),
            ]),
        )));
        let emails = provider.get_thread("thread-alpha").await.unwrap();
        let target = crate::unified::reply_target(&emails, None).unwrap();
        assert_eq!(target.provider_id, "newer");
        let (to, _, _) =
            crate::unified::gmail_reply_fields(target, Some("owner@example.com"), false);
        assert_eq!(to, "newer@example.edu");
        assert_eq!(
            crate::unified::reply_target(&emails, Some("older"))
                .unwrap()
                .provider_id,
            "older"
        );
    }

    #[test]
    fn structured_quota_forbidden_errors_are_environment_failures() {
        use mecha_core::mail_rules::{engine::ActionResult, FailureKind};
        for reason in [
            "rateLimitExceeded",
            "userRateLimitExceeded",
            "dailyLimitExceeded",
        ] {
            let body = serde_json::json!({"error":{"code":403,"errors":[{"reason":reason}],"message":"fixture quota"}}).to_string();
            let error = MailError::gmail_api_error(403, body);
            assert!(matches!(
                crate::rules::failure(error),
                ActionResult::Failed {
                    kind: FailureKind::Environment,
                    ..
                }
            ));
        }
        for body in ["rateLimitExceeded".into(), serde_json::json!({"error":{"errors":[{"reason":"domainPolicy"}]}}).to_string(), serde_json::json!({"error":{"errors":[{"reason":"rateLimitExceeded"},{"reason":"forbidden"}]}}).to_string()] {
            assert!(matches!(crate::rules::failure(MailError::gmail_api_error(403, body)), ActionResult::Failed {kind: FailureKind::Action, ..}));
        }
    }

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
        let email = parse_gmail_message(HeaderRead::Full, &msg);
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
    #[test]
    fn gmail_unknown_inbound_dates_prevent_newest_selection() {
        use crate::types::ThreadMetadata;
        let known = parse_gmail_message(
            HeaderRead::Full,
            &json!({
                "id":"known", "internalDate":"1704067200000", "labelIds":["INBOX"],
                "payload":{"headers":[{"name":"From", "value":"sender@example.edu"}]}
            }),
        );
        assert_eq!(known.date_known, Some(true));
        assert_eq!(known.date_received, "2024-01-01T00:00:00Z");
        for date in [
            json!({}),
            json!({"internalDate":"bad"}),
            json!({"internalDate":"9223372036854775807"}),
        ] {
            let mut message = date;
            message["id"] = json!("unknown");
            message["labelIds"] = json!(["INBOX"]);
            message["payload"] = json!({"headers":[
                {"name":"From", "value":"sender@example.edu"},
                {"name":"Date", "value":"not a date"}
            ]});
            for header in [json!("not a date"), json!("")] {
                message["payload"]["headers"][1]["value"] = header;
                let unknown = parse_gmail_message(HeaderRead::Full, &message);
                assert_eq!(unknown.date_known, Some(false));
                assert!(chrono::DateTime::parse_from_rfc3339(&unknown.date_received).is_ok());
                for certainty in [Some(false), None] {
                    let mut unknown = unknown.clone();
                    unknown.date_known = certainty;
                    let metadata = ThreadMetadata::from_emails(
                        &[known.clone(), unknown.clone()],
                        Some("owner@example.com"),
                    );
                    assert_eq!(metadata.newest_inbound_provider_id, None);
                    assert_eq!(metadata.owner_participated, Some(false));
                    unknown.from_address = "owner@example.com".into();
                    let metadata = ThreadMetadata::from_emails(
                        &[known.clone(), unknown],
                        Some("owner@example.com"),
                    );
                    assert_eq!(
                        metadata.newest_inbound_provider_id.as_deref(),
                        Some("known")
                    );
                    assert_eq!(metadata.owner_participated, Some(true));
                }
            }
            message["payload"]["headers"][1]["value"] = json!("Mon, 01 Jan 2024 00:00:00 +0000");
            let fallback = parse_gmail_message(HeaderRead::Full, &message);
            assert_eq!(fallback.date_known, Some(true));
            assert_eq!(fallback.date_received, known.date_received);
        }
        assert_eq!(
            parse_gmail_message(HeaderRead::Full, &json!({})).date_known,
            Some(false)
        );
    }

    #[test]
    fn gmail_header_completeness_requires_a_full_request() {
        // A metadata response can have the same well-formed array as a full
        // response. The request, not the array shape, establishes completeness.
        let message = json!({"payload":{"headers":[
            {"name":"Authentication-Results", "value":"mx.google.com; dmarc=pass header.from=example.edu"}
        ]}});
        let full = parse_gmail_message(HeaderRead::Full, &message);
        let partial = parse_gmail_message(HeaderRead::Partial, &message);
        assert_eq!(full.headers_complete, Some(true));
        assert_eq!(partial.headers_complete, None);
        assert_eq!(partial.authentication_results, full.authentication_results);
    }

    #[test]
    fn gmail_empty_missing_or_malformed_headers_are_not_complete() {
        for message in [
            json!({}),
            json!({"payload":{"headers":[]}}),
            json!({"payload":{"headers":[{"name":"From"}]}}),
        ] {
            assert_eq!(
                parse_gmail_message(HeaderRead::Full, &message).headers_complete,
                None
            );
        }
        assert_eq!(
            parse_gmail_message(
                HeaderRead::Full,
                &json!({"payload":{"headers":[
                    {"name":"From", "value":"sender@example.edu"}
                ]}})
            )
            .headers_complete,
            Some(true)
        );
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
            HeaderRead::Full,
            &json!({"id":"message-1", "threadId":"thread-1", "payload":{"headers":headers}}),
        )
    }

    #[test]
    fn gmail_authentication_results_expose_missing_foreign_duplicate_and_single_claims() {
        assert!(with_authentication_results(&[])
            .authentication_results
            .is_none());
        assert!(parse_gmail_message(HeaderRead::Full, &json!({}))
            .authentication_results
            .is_none());
        assert_eq!(
            parse_gmail_message(HeaderRead::Full, &json!({})).headers_complete,
            None
        );
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
        let email = parse_gmail_message(
            HeaderRead::Full,
            &json!({
                "id":"provider-message", "threadId":"provider-thread", "labelIds":["INBOX"],
                "payload":{"headers":[
                    {"name":"Message-ID", "value":"<sender-controlled@example.com>"},
                    {"name":"Auto-Submitted", "value":"auto-generated"},
                    {"name":"Precedence", "value":"bulk"},
                    {"name":"List-Id", "value":"<notices.example.edu>"},
                    {"name":"Reply-To", "value":"reply@example.com"}
                ]}
            }),
        );
        assert_eq!(email.provider_id, "provider-message");
        assert_eq!(email.thread_id.as_deref(), Some("provider-thread"));
        assert_eq!(email.auto_submitted.as_deref(), Some("auto-generated"));
        assert_eq!(email.precedence.as_deref(), Some("bulk"));
        assert_eq!(email.list_id.as_deref(), Some("<notices.example.edu>"));
        assert_eq!(email.reply_to.as_deref(), Some("reply@example.com"));
        assert!(email.authentication_results.is_none());
        assert_eq!(email.is_draft, Some(false));
        assert_eq!(
            parse_gmail_message(HeaderRead::Full, &json!({"labelIds":["DRAFT"]})).is_draft,
            Some(true)
        );
        let missing = parse_gmail_message(HeaderRead::Full, &json!({}));
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
