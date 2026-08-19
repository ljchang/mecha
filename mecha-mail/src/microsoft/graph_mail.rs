//! Outlook mail over Microsoft Graph. Four rules here, each one a bug if
//! undone:
//!
//! 1. **Replies go through `POST /messages/{id}/reply`.** Hand-injecting
//!    `In-Reply-To`/`References` via `internetMessageHeaders` does not work:
//!    those names are on Graph's reserved list and may be silently dropped,
//!    landing replies as new conversations. `/reply` makes Graph set the
//!    headers and `conversationId` itself.
//! 2. **`to` is split on commas**, exactly as cc and bcc are — otherwise a
//!    multi-recipient To becomes one malformed address.
//! 3. **`$search` for free text, `$filter` for structured**, never `$orderby`
//!    alongside an arbitrary `$filter` (that combination 400s with
//!    `InefficientFilter`).
//! 4. **HTML bodies go through [`crate::text`]**, not a char-scan tag stripper
//!    that leaves `<style>` contents in the text.
//!
//! Dropped: the delta/sync machinery (it serves a local cache we do not keep)
//! and the folder-move operations.

use serde_json::{json, Value};

use crate::http::send_with_retry;
use crate::types::{Email, MailError};

const GRAPH: &str = "https://graph.microsoft.com/v1.0";

/// The fields worth asking for. Graph returns everything by default, which is
/// a lot of bytes per message.
/// `internetMessageHeaders` is here for one header — `List-Unsubscribe`, which
/// is the deterministic bulk-mail signal the triage pre-filter keys on. Graph
/// exposes no bulk flag of its own, and without the header selected the field
/// parses as `None` for every message: the pre-filter would then hold for Gmail
/// and silently never fire for Outlook, which is the account the nightly
/// actually runs on. A rule that is inert on the mailbox it was written for is
/// worse than no rule, because the config reads as though it were working.
const SELECT: &str = "id,conversationId,internetMessageId,subject,bodyPreview,body,from,\
toRecipients,ccRecipients,bccRecipients,receivedDateTime,isRead,hasAttachments,flag,\
internetMessageHeaders";

#[derive(Clone)]
pub struct OutlookProvider {
    access_token: String,
    client: reqwest::Client,
}

impl OutlookProvider {
    pub fn new(access_token: String) -> Self {
        Self {
            access_token,
            client: crate::http::client(),
        }
    }

    async fn get_json(&self, url: &str) -> Result<Value, MailError> {
        let resp = send_with_retry(
            self.client
                .get(url)
                .bearer_auth(&self.access_token)
                // Required whenever $search is used; harmless otherwise.
                .header("ConsistencyLevel", "eventual"),
        )
        .await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(MailError::ApiError {
                status,
                message: super::auth::humanize_aadsts(&body),
            });
        }
        resp.json::<Value>().await.map_err(MailError::from)
    }

    /// Free-text search across the mailbox, KQL-ish: bare words, or
    /// `from:alice@x.com`, `subject:invoice`, `received>=2026-08-01`.
    ///
    /// Graph forbids `$orderby` with `$search`, so results come back in
    /// relevance order and are sorted here by date instead.
    pub async fn search(&self, query: &str, max_results: u32) -> Result<Vec<Email>, MailError> {
        let top = max_results.clamp(1, 100);
        let url = format!(
            "{GRAPH}/me/messages?$top={top}&$select={SELECT}&$search=%22{}%22",
            urlencode(query)
        );
        let json = self.get_json(&url).await?;
        let mut emails: Vec<Email> = json["value"]
            .as_array()
            .map(|arr| arr.iter().map(parse_outlook_message).collect())
            .unwrap_or_default();
        emails.sort_by(|a, b| b.date_received.cmp(&a.date_received));
        Ok(emails)
    }

    /// Most recent **inbox** messages, newest first. Scoped to the inbox
    /// folder because `/me/messages` spans every folder — Deleted Items,
    /// Sent, Drafts — and "what just came in" answered with the user's own
    /// sent replies and trashed mail reads as correct while being wrong.
    /// `$orderby` is legal here precisely because there is no `$filter` or
    /// `$search` beside it.
    pub async fn recent(&self, max_results: u32) -> Result<Vec<Email>, MailError> {
        let top = max_results.clamp(1, 100);
        let url = format!(
            "{GRAPH}/me/mailFolders/inbox/messages?$top={top}&$select={SELECT}&$orderby=receivedDateTime%20desc"
        );
        let json = self.get_json(&url).await?;
        Ok(json["value"]
            .as_array()
            .map(|arr| arr.iter().map(parse_outlook_message).collect())
            .unwrap_or_default())
    }

    /// Walk the whole mailbox backwards in time, newest first, stopping once
    /// messages are older than `since` (RFC 3339).
    ///
    /// This is the corpus path, and it is a different shape from
    /// [`Self::search`] for two reasons Graph forces. `search` caps at 100 with
    /// no cursor at all, so it cannot reach a year; and `$search` forbids
    /// `$orderby`, so it cannot walk time in order. What works is `$orderby`
    /// **alone** — no `$filter` beside it, the same constraint `recent`
    /// documents — plus following `@odata.nextLink`, which
    /// `graph_calendar.rs` already does for `calendarView`.
    ///
    /// `/me/messages` rather than `/me/mailFolders/inbox/messages`: a corpus
    /// wants filed and sent mail too. Sent is the load-bearing part — whether
    /// a thread ever got a reply is only answerable if the replies are in the
    /// data, and "which requests did I never answer" is the question this
    /// corpus exists to ask.
    ///
    /// Envelope and preview only, never bodies. A year of full bodies is a
    /// second copy of the mailbox for an analysis that does not need one, and
    /// the bodies of any thread that turns out to matter can be fetched by id
    /// afterwards.
    ///
    /// `on_batch` is called per page so the caller can stream to disk instead
    /// of holding a year of mail in memory.
    pub async fn page_since<F>(&self, since: &str, mut on_batch: F) -> Result<usize, MailError>
    where
        F: FnMut(&[Email]) -> Result<(), MailError>,
    {
        const CORPUS_SELECT: &str = "id,conversationId,subject,bodyPreview,from,toRecipients,\
ccRecipients,receivedDateTime,isRead,hasAttachments,internetMessageHeaders";
        let mut url = format!(
            "{GRAPH}/me/messages?$top=100&$select={CORPUS_SELECT}&$orderby=receivedDateTime%20desc"
        );
        let mut total = 0usize;
        loop {
            let json = self.get_json(&url).await?;
            let page: Vec<Email> = json["value"]
                .as_array()
                .map(|arr| arr.iter().map(parse_outlook_message).collect())
                .unwrap_or_default();
            if page.is_empty() {
                break;
            }
            // Newest first, so the first message older than the cutoff ends
            // the walk — everything after it is older still.
            let keep: Vec<Email> = page
                .iter()
                .take_while(|e| e.date_received.as_str() >= since)
                .cloned()
                .collect();
            let reached_cutoff = keep.len() < page.len();
            if !keep.is_empty() {
                total += keep.len();
                on_batch(&keep)?;
            }
            if reached_cutoff {
                break;
            }
            match json["@odata.nextLink"].as_str() {
                Some(next) => url = next.to_string(),
                None => break,
            }
        }
        Ok(total)
    }

    pub async fn get_message(&self, message_id: &str) -> Result<Email, MailError> {
        let url = format!(
            "{GRAPH}/me/messages/{}?$select={SELECT}",
            urlencode(message_id)
        );
        Ok(parse_outlook_message(&self.get_json(&url).await?))
    }

    /// A conversation. Graph has no thread resource — a "thread" is every
    /// message sharing a `conversationId`, which is a filter query.
    pub async fn get_thread(&self, conversation_id: &str) -> Result<Vec<Email>, MailError> {
        let filter = format!(
            "conversationId eq '{}'",
            conversation_id.replace('\'', "''")
        );
        let url = format!(
            "{GRAPH}/me/messages?$filter={}&$select={SELECT}",
            urlencode(&filter)
        );
        let json = self.get_json(&url).await?;
        let mut emails: Vec<Email> = json["value"]
            .as_array()
            .map(|arr| arr.iter().map(parse_outlook_message).collect())
            .unwrap_or_default();
        // Oldest first, so a thread reads top to bottom.
        emails.sort_by(|a, b| a.date_received.cmp(&b.date_received));
        Ok(emails)
    }

    /// Send a new message. `body` is HTML.
    pub async fn send_email(
        &self,
        to: &str,
        subject: &str,
        body: &str,
        cc: Option<&str>,
        bcc: Option<&str>,
    ) -> Result<(), MailError> {
        let mut message = json!({
            "subject": subject,
            "body": {"contentType": "HTML", "content": body},
            "toRecipients": recipients(to),
        });
        if let Some(cc) = cc.filter(|s| !s.is_empty()) {
            message["ccRecipients"] = json!(recipients(cc));
        }
        if let Some(bcc) = bcc.filter(|s| !s.is_empty()) {
            message["bccRecipients"] = json!(recipients(bcc));
        }

        let resp = send_with_retry(
            self.client
                .post(format!("{GRAPH}/me/sendMail"))
                .bearer_auth(&self.access_token)
                .json(&json!({"message": message, "saveToSentItems": true})),
        )
        .await?;
        self.ok_or_err(resp).await
    }

    /// Reply to a message, in its conversation. Graph sets `In-Reply-To`,
    /// `References`, and `conversationId` itself — which is the whole reason
    /// this is a separate operation from [`Self::send_email`].
    pub async fn reply(
        &self,
        message_id: &str,
        body: &str,
        reply_all: bool,
    ) -> Result<(), MailError> {
        let verb = if reply_all { "replyAll" } else { "reply" };
        let resp = send_with_retry(
            self.client
                .post(format!(
                    "{GRAPH}/me/messages/{}/{verb}",
                    urlencode(message_id)
                ))
                .bearer_auth(&self.access_token)
                .json(&json!({"message": {"body": {"contentType": "HTML", "content": body}}})),
        )
        .await?;
        self.ok_or_err(resp).await
    }

    /// The signed-in account's address.
    ///
    /// `GET /me` needs `User.Read`, which this crate deliberately does not
    /// request — mail and calendar do not need to read your directory
    /// profile, and asking for a scope you do not need is a consent prompt
    /// (and, in a managed tenant, possibly an admin approval) bought for
    /// nothing. So try it, and fall back to reading the From address off a
    /// message in Sent Items, which `Mail.Read` already covers.
    pub async fn profile_address(&self) -> Result<String, MailError> {
        if let Ok(json) = self
            .get_json(&format!("{GRAPH}/me?$select=mail,userPrincipalName"))
            .await
        {
            if let Some(addr) = json["mail"]
                .as_str()
                .or_else(|| json["userPrincipalName"].as_str())
            {
                return Ok(addr.to_string());
            }
        }

        let json = self
            .get_json(&format!(
                "{GRAPH}/me/mailFolders/sentitems/messages?$top=1&$select=from"
            ))
            .await?;
        json["value"][0]["from"]["emailAddress"]["address"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| {
                MailError::ParseError(
                    "could not determine the account address (no User.Read scope, and \
                     Sent Items is empty)"
                        .into(),
                )
            })
    }

    // ── triage ────────────────────────────────────────────────────────────
    //
    // The asymmetry with Gmail lives here and cannot be abstracted away.
    // Gmail applies a label change to a whole thread in one call; **Graph has
    // no thread resource at all** — a conversation is every message sharing a
    // `conversationId`, which is a filter query (see `get_thread`). So every
    // verb below is: resolve the conversation to message ids, then act on
    // each.
    //
    // Two consequences worth stating rather than discovering. It is N+1
    // requests where Gmail is one. And it is **not atomic**: a failure
    // halfway leaves some messages moved and some not. The verbs therefore
    // report how many they touched and fail only if *every* message failed —
    // the same convention a fan-out read already uses, for the same reason.
    // A partly-archived conversation is visibly odd; a silently-half-archived
    // one reported as success is the failure that costs trust.

    /// The message ids in a conversation. Ids only — the triage verbs never
    /// need the bodies, and `$select=id` keeps a fifty-message thread cheap.
    async fn conversation_message_ids(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<String>, MailError> {
        let filter = format!(
            "conversationId eq '{}'",
            conversation_id.replace('\'', "''")
        );
        let url = format!(
            "{GRAPH}/me/messages?$filter={}&$select=id&$top=100",
            urlencode(&filter)
        );
        let json = self.get_json(&url).await?;
        let ids: Vec<String> = json["value"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m["id"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if ids.is_empty() {
            return Err(MailError::ApiError {
                status: 404,
                message: format!("no messages in conversation {conversation_id}"),
            });
        }
        Ok(ids)
    }

    /// Apply an operation to every message in a conversation, tolerating
    /// partial failure. Errors only when nothing succeeded, and then with the
    /// first real error rather than a count.
    async fn for_each_message<'a, F, Fut>(
        &'a self,
        conversation_id: &str,
        op: F,
    ) -> Result<usize, MailError>
    where
        F: Fn(&'a Self, String) -> Fut,
        Fut: std::future::Future<Output = Result<(), MailError>>,
    {
        let ids = self.conversation_message_ids(conversation_id).await?;
        let total = ids.len();
        let mut first_err = None;
        let mut ok = 0usize;
        for id in ids {
            match op(self, id).await {
                Ok(()) => ok += 1,
                Err(e) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                }
            }
        }
        match (ok, first_err) {
            (0, Some(e)) => Err(e),
            _ => {
                if ok < total {
                    tracing::warn!(
                        conversation = conversation_id,
                        touched = ok,
                        total,
                        "triage applied to part of the conversation"
                    );
                }
                Ok(ok)
            }
        }
    }

    /// Move one message to a well-known folder.
    async fn move_message(&self, message_id: &str, folder: &str) -> Result<(), MailError> {
        let resp = send_with_retry(
            self.client
                .post(format!(
                    "{GRAPH}/me/messages/{}/move",
                    urlencode(message_id)
                ))
                .bearer_auth(&self.access_token)
                .json(&json!({ "destinationId": folder })),
        )
        .await?;
        self.ok_or_err(resp).await
    }

    /// Move the conversation to Archive. `archive` is a well-known folder
    /// name Graph resolves per-mailbox, so this works without knowing the
    /// user's folder ids or their language.
    pub async fn archive_thread(&self, conversation_id: &str) -> Result<usize, MailError> {
        self.for_each_message(conversation_id, |s, id| async move {
            s.move_message(&id, "archive").await
        })
        .await
    }

    /// Report the conversation as junk. Graph has no "mark as spam" verb —
    /// moving to the Junk Email folder *is* the report, and is what trains
    /// the filter.
    pub async fn spam_thread(&self, conversation_id: &str) -> Result<usize, MailError> {
        self.for_each_message(conversation_id, |s, id| async move {
            s.move_message(&id, "junkemail").await
        })
        .await
    }

    /// Move the conversation to Deleted Items — recoverable, like Gmail's
    /// trash. Nothing here can permanently delete.
    pub async fn trash_thread(&self, conversation_id: &str) -> Result<usize, MailError> {
        self.for_each_message(conversation_id, |s, id| async move {
            s.move_message(&id, "deleteditems").await
        })
        .await
    }

    /// Set read state across the conversation. A PATCH on the message rather
    /// than a move, so this is the one verb that leaves the message where it
    /// is.
    pub async fn set_thread_read(
        &self,
        conversation_id: &str,
        read: bool,
    ) -> Result<usize, MailError> {
        self.for_each_message(conversation_id, move |s, id| async move {
            let resp = send_with_retry(
                s.client
                    .patch(format!("{GRAPH}/me/messages/{}", urlencode(&id)))
                    .bearer_auth(&s.access_token)
                    .json(&json!({ "isRead": read })),
            )
            .await?;
            s.ok_or_err(resp).await
        })
        .await
    }

    async fn ok_or_err(&self, resp: reqwest::Response) -> Result<(), MailError> {
        if resp.status().is_success() {
            return Ok(());
        }
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        Err(MailError::ApiError {
            status,
            message: super::auth::humanize_aadsts(&body),
        })
    }
}

/// Split a comma-separated address list into Graph recipient objects. Every
/// recipient field goes through this — `to` as much as cc and bcc.
fn recipients(list: &str) -> Vec<Value> {
    list.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|addr| json!({"emailAddress": {"address": addr}}))
        .collect()
}

pub(crate) fn parse_outlook_message(msg: &Value) -> Email {
    let provider_id = msg["id"].as_str().unwrap_or_default().to_string();

    let addr_list = |key: &str| -> Vec<String> {
        msg[key]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|r| r["emailAddress"]["address"].as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    };

    // Graph gives one body with a contentType; the text/HTML split that
    // `crate::text::clean_body` expects is reconstructed here.
    let content = msg["body"]["content"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let is_html = msg["body"]["contentType"]
        .as_str()
        .is_some_and(|t| t.eq_ignore_ascii_case("html"));
    let (body_text, body_html) = if is_html {
        (String::new(), content)
    } else {
        (content, String::new())
    };

    let date_received = msg["receivedDateTime"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| {
            dt.with_timezone(&chrono::Utc)
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string()
        })
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());

    Email {
        id: format!("outlook-{provider_id}"),
        provider: "outlook".to_string(),
        provider_id,
        thread_id: msg["conversationId"].as_str().map(|s| s.to_string()),
        message_id: msg["internetMessageId"].as_str().map(|s| s.to_string()),
        subject: msg["subject"].as_str().unwrap_or_default().to_string(),
        from_address: msg["from"]["emailAddress"]["address"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        from_name: msg["from"]["emailAddress"]["name"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        to_addresses: addr_list("toRecipients"),
        cc_addresses: addr_list("ccRecipients"),
        bcc_addresses: addr_list("bccRecipients"),
        date_received,
        body_text,
        body_html,
        snippet: msg["bodyPreview"].as_str().unwrap_or_default().to_string(),
        // Graph has no labels; folders are a different concept and unused here.
        labels: Vec::new(),
        is_read: msg["isRead"].as_bool().unwrap_or(false),
        is_starred: msg["flag"]["flagStatus"].as_str() == Some("flagged"),
        has_attachments: msg["hasAttachments"].as_bool().unwrap_or(false),
        list_unsubscribe: msg["internetMessageHeaders"].as_array().and_then(|hdrs| {
            hdrs.iter().find_map(|h| {
                let name = h["name"].as_str()?;
                name.eq_ignore_ascii_case("list-unsubscribe")
                    .then(|| h["value"].as_str().map(|s| s.to_string()))?
            })
        }),
    }
}

pub(crate) fn urlencode(s: &str) -> String {
    crate::google::gmail::urlencode(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_recipient_field_splits_on_commas() {
        let r = recipients("a@x.com, b@y.com ,c@z.com");
        assert_eq!(
            r.len(),
            3,
            "`to` must split on commas, not be sent as one address"
        );
        assert_eq!(r[1]["emailAddress"]["address"], "b@y.com");
        assert!(recipients("").is_empty());
    }

    #[test]
    fn a_graph_message_parses_into_the_portable_shape() {
        let msg = json!({
            "id": "AAMk123", "conversationId": "conv-1",
            "internetMessageId": "<mid@dartmouth.edu>",
            "subject": "Grant deadline", "bodyPreview": "quick note",
            "body": {"contentType": "html", "content": "<p>quick note</p>"},
            "from": {"emailAddress": {"name": "Priya Nair", "address": "priya@dartmouth.edu"}},
            "toRecipients": [{"emailAddress": {"address": "luke@dartmouth.edu"}}],
            "receivedDateTime": "2026-08-04T15:30:00Z",
            "isRead": false, "hasAttachments": true,
            "flag": {"flagStatus": "flagged"}
        });
        let e = parse_outlook_message(&msg);
        assert_eq!(e.id, "outlook-AAMk123");
        assert_eq!(e.thread_id.as_deref(), Some("conv-1"));
        assert_eq!(e.from_name, "Priya Nair");
        assert_eq!(e.to_addresses, vec!["luke@dartmouth.edu"]);
        assert_eq!(e.date_received, "2026-08-04T15:30:00Z");
        assert!(!e.is_read && e.is_starred && e.has_attachments);
        // An HTML body must land in body_html so text::clean_body converts it
        // rather than handing the model an empty string.
        assert!(e.body_text.is_empty() && e.body_html.contains("<p>"));
        assert!(!crate::text::clean_body(&e).is_empty());
    }

    #[test]
    fn a_plain_text_body_lands_in_body_text() {
        let msg = json!({
            "id": "x",
            "body": {"contentType": "text", "content": "plain words"}
        });
        let e = parse_outlook_message(&msg);
        assert_eq!(e.body_text, "plain words");
        assert!(e.body_html.is_empty());
    }

    #[test]
    fn a_conversation_id_with_a_quote_cannot_break_the_filter() {
        // OData escapes a single quote by doubling it; without this a crafted
        // conversation id would change the query's meaning.
        let escaped = "AAQk'DROP".replace('\'', "''");
        assert_eq!(escaped, "AAQk''DROP");
    }
}
