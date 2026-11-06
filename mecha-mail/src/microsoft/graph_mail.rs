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
//! (folder moves are the mailbox-only triage verbs below).

use serde_json::{json, Value};

use crate::http::send_with_retry;
use crate::types::{authentication_results, first_header, Email, MailError};

const GRAPH: &str = "https://graph.microsoft.com/v1.0";

/// The fields worth asking for. Graph returns everything by default, which is
/// a lot of bytes per message.
/// `internetMessageHeaders` supplies rule prerequisites and `List-Unsubscribe`, which
/// is the deterministic bulk-mail signal the triage pre-filter keys on. Graph
/// exposes no bulk flag of its own, and without the header selected the field
/// parses as `None` for every message: the pre-filter would then hold for Gmail
/// and silently never fire for Outlook, which is the account the nightly
/// actually runs on. A rule that is inert on the mailbox it was written for is
/// worse than no rule, because the config reads as though it were working.
const SELECT: &str = "id,conversationId,internetMessageId,subject,bodyPreview,body,from,\
toRecipients,ccRecipients,bccRecipients,receivedDateTime,isRead,isDraft,parentFolderId,hasAttachments,flag,\
internetMessageHeaders";

/// Successful and failed message mutations. For read-state verbs, `moved`
/// counts successful patches instead of folder moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MutationOutcome {
    pub moved: usize,
    pub failed: usize,
    pub failure_kind: Option<MutationFailureKind>,
}

/// Failure categories retained across partial mutations. Only an entirely
/// environmental failure set is exempt from the rule failure brake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationFailureKind {
    Environment,
    Action,
    Unknown,
}
impl MutationFailureKind {
    pub(crate) fn of(error: &MailError) -> Self {
        match error {
            MailError::HttpError(_)
            | MailError::AuthError(_)
            | MailError::AuthRevoked(_)
            | MailError::GmailQuota { .. } => Self::Environment,
            MailError::ApiError { status, .. }
                if *status == 401 || *status == 429 || *status >= 500 =>
            {
                Self::Environment
            }
            MailError::InvalidInput(_) | MailError::ApiError { .. } => Self::Action,
            MailError::ParseError(_) => Self::Unknown,
        }
    }
    fn combine(self, other: Self) -> Self {
        if self == Self::Unknown || other == Self::Unknown {
            Self::Unknown
        } else if self == Self::Action || other == Self::Action {
            Self::Action
        } else {
            Self::Environment
        }
    }
}

/// Result of a folder-scoped restore. Messages with an unknown destination
/// remain in the source folder. Failed moves are counted separately because
/// a transport failure does not prove whether the move happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RestoreOutcome {
    pub moved: usize,
    pub left_in_place: usize,
    pub failed: usize,
}

#[cfg(test)]
#[derive(Default)]
struct ScriptedReads {
    urls: Vec<String>,
    pages: std::collections::VecDeque<Result<Value, MailError>>,
}

#[derive(Clone)]
pub struct OutlookProvider {
    access_token: String,
    client: reqwest::Client,
    immutable_ids: bool,
    #[cfg(test)]
    scripted_reads: Option<std::sync::Arc<std::sync::Mutex<ScriptedReads>>>,
    #[cfg(test)]
    scripted_mailbox: Option<std::sync::Arc<std::sync::Mutex<tests::Mailbox>>>,
}

impl OutlookProvider {
    pub fn new(access_token: String) -> Self {
        Self {
            access_token,
            client: crate::http::client(),
            immutable_ids: false,
            #[cfg(test)]
            scripted_reads: None,
            #[cfg(test)]
            scripted_mailbox: None,
        }
    }

    /// Opt this mail client into Graph immutable ids for every request it
    /// makes, including pagination, moves and restores. New clients retain
    /// Graph's default ids. Callers must use the same mode across a ledger's
    /// reads and writes; no unified tool enables it by default.
    pub fn with_immutable_ids(mut self) -> Self {
        self.immutable_ids = true;
        self
    }

    fn request(&self, method: reqwest::Method, url: &str) -> reqwest::RequestBuilder {
        let request = self
            .client
            .request(method, url)
            .bearer_auth(&self.access_token);
        if self.immutable_ids {
            request.header("Prefer", "IdType=\"ImmutableId\"")
        } else {
            request
        }
    }

    async fn get_json(&self, url: &str) -> Result<Value, MailError> {
        #[cfg(test)]
        if let Some(mailbox) = &self.scripted_mailbox {
            return mailbox.lock().unwrap().get(url);
        }
        // Inject only the I/O boundary so tests exercise the real provider
        // methods, including their decision to follow subsequent pages.
        #[cfg(test)]
        if let Some(script) = &self.scripted_reads {
            let mut script = script.lock().unwrap();
            script.urls.push(url.to_owned());
            return script.pages.pop_front().expect("unexpected Graph request");
        }
        let resp = send_with_retry(
            self.request(reqwest::Method::GET, url)
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
        emails.sort_by_key(|e| std::cmp::Reverse(e.received_instant()));
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

    /// Recent inbox conversation IDs only. Filter dates locally so missing
    /// or malformed arrivals remain visible to read_thread's fail-closed check.
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
        let scope = if inbox_only {
            "me/mailFolders/inbox/messages"
        } else {
            "me/messages"
        };
        let mut url = format!("{GRAPH}/{scope}?$top={}&$select=id,conversationId,receivedDateTime&$orderby=receivedDateTime%20desc", max_results.clamp(1, 100));
        let mut pages = std::collections::BTreeSet::new();
        while seen < max_results {
            crate::rules::check_rule_page(pages.len())?;
            validate_page_url(&url, &mut pages)?;
            let page = self.get_json(&url).await?;
            let messages = page["value"].as_array().ok_or_else(|| {
                MailError::ParseError("Graph message page has no value array".into())
            })?;
            let mut all_older = !messages.is_empty();
            for message in messages.iter().take((max_results - seen) as usize) {
                seen += 1;
                let arrival = message["receivedDateTime"]
                    .as_str()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
                if arrival.is_none_or(|at| at >= since) {
                    all_older = false;
                    let id = message["id"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| MailError::ParseError("rule message has no id".into()))?;
                    let thread = message["conversationId"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| {
                            MailError::ParseError("rule message has no conversation id".into())
                        })?;
                    hits.push(crate::rules::RuleMessageHit {
                        id: id.into(),
                        thread_id: thread.into(),
                    });
                }
            }
            let next = next_page(&page)?;
            if all_older {
                break;
            }
            match next {
                Some(next) => url = next,
                None => break,
            }
        }
        Ok(hits)
    }

    /// Inbox identity is account-local; compare it to the fresh message read.
    pub async fn rule_inbox_id(&self) -> Result<String, MailError> {
        self.get_json(&format!("{GRAPH}/me/mailFolders/inbox?$select=id"))
            .await?["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| MailError::ParseError("inbox folder has no id".into()))
    }
    pub async fn rule_test_message(
        &self,
        id: &str,
        inbox: &str,
    ) -> Result<(Email, Option<bool>), MailError> {
        let raw = self
            .get_json(&format!(
                "{GRAPH}/me/messages/{}?$select={SELECT}",
                urlencode(id)
            ))
            .await?;
        let in_inbox = raw["parentFolderId"].as_str().map(|folder| folder == inbox);
        Ok((parse_outlook_message(&raw), in_inbox))
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
    pub async fn page_since<F>(&self, since: &str, on_batch: F) -> Result<usize, MailError>
    where
        F: FnMut(&[Email]) -> Result<(), MailError>,
    {
        corpus_since(
            since,
            on_batch,
            |url| async move { self.get_json(&url).await },
        )
        .await
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
            "{GRAPH}/me/messages?$filter={}&$select={SELECT}&$top=100",
            urlencode(&filter)
        );
        let messages = message_pages(url, |url| async move { self.get_json(&url).await }).await?;
        let mut emails: Vec<Email> = messages.iter().map(parse_outlook_message).collect();
        // Oldest first, so a thread reads top to bottom.
        emails.sort_by_key(Email::received_instant);
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
            self.request(reqwest::Method::POST, &format!("{GRAPH}/me/sendMail"))
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
            self.request(
                reqwest::Method::POST,
                &format!("{GRAPH}/me/messages/{}/{verb}", urlencode(message_id)),
            )
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

    /// The message ids in a conversation, optionally restricted to a source
    /// folder. Ids only — these verbs never need the bodies. An empty source
    /// folder returns zero ids; an absent whole conversation remains a 404.
    async fn conversation_message_ids(
        &self,
        conversation_id: &str,
        source_folder: Option<&str>,
    ) -> Result<Vec<String>, MailError> {
        let filter = format!(
            "conversationId eq '{}'",
            conversation_id.replace('\'', "''")
        );
        let collection = source_folder.map_or_else(
            || "me/messages".to_owned(),
            |folder| format!("me/mailFolders/{folder}/messages"),
        );
        let url = format!(
            "{GRAPH}/{collection}?$filter={}&$select=id&$top=100",
            urlencode(&filter)
        );
        let messages = message_pages(url, |url| async move { self.get_json(&url).await }).await?;
        let ids: Vec<String> = messages
            .iter()
            .map(|m| {
                m["id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| MailError::ParseError("conversation message has no id".into()))
            })
            .collect::<Result<_, _>>()?;
        if ids.is_empty() {
            if source_folder.is_some() {
                // Empty Inbox is a no-op only if the conversation exists.
                // This is an existence probe, never a source of move ids;
                // one result suffices, so do not follow its nextLink.
                let url = format!(
                    "{GRAPH}/me/messages?$filter={}&$select=id&$top=1",
                    urlencode(&filter)
                );
                let page = self.get_json(&url).await?;
                let messages = page["value"].as_array().ok_or_else(|| {
                    MailError::ParseError("Graph message page has no value array".into())
                })?;
                if let Some(message) = messages.first() {
                    message["id"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .ok_or_else(|| {
                            MailError::ParseError("conversation message has no id".into())
                        })?;
                    return Ok(ids);
                }
            }
            return Err(MailError::ApiError {
                status: 404,
                message: format!("no messages in conversation {conversation_id}"),
            });
        }
        Ok(ids)
    }

    /// Apply an operation to the conversation's messages in the given scope,
    /// tolerating partial failure. Errors if every attempted operation failed,
    /// with the first real error; an empty source folder returns zero counts.
    async fn for_each_message<'a, F, Fut>(
        &'a self,
        conversation_id: &str,
        source_folder: Option<&str>,
        op: F,
    ) -> Result<MutationOutcome, MailError>
    where
        F: Fn(&'a Self, String) -> Fut,
        Fut: std::future::Future<Output = Result<(), MailError>>,
    {
        let ids = self
            .conversation_message_ids(conversation_id, source_folder)
            .await?;
        let total = ids.len();
        let mut first_err = None;
        let mut failure_kind: Option<MutationFailureKind> = None;
        let mut ok = 0usize;
        for id in ids {
            match op(self, id).await {
                Ok(()) => ok += 1,
                Err(e) => {
                    let kind = MutationFailureKind::of(&e);
                    let combined = failure_kind.map_or(kind, |previous| previous.combine(kind));
                    if first_err.is_none() || failure_kind != Some(combined) {
                        first_err = Some(e);
                    }
                    failure_kind = Some(combined);
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
                Ok(MutationOutcome {
                    moved: ok,
                    failed: total - ok,
                    failure_kind,
                })
            }
        }
    }

    /// Move one message to a well-known folder.
    pub async fn move_message(&self, message_id: &str, folder: &str) -> Result<(), MailError> {
        self.mutate_message(self.move_message_request(message_id, folder))
            .await
    }

    /// Restore only the exact immutable message still in its source folder.
    /// Like P0's structured restore, nothing left in the source is success.
    pub async fn restore_rule_message(&self, id: &str, source: &str) -> Result<(), MailError> {
        let folder = self
            .get_json(&format!("{GRAPH}/me/mailFolders/{source}?$select=id"))
            .await?;
        let source_id = folder["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| MailError::ParseError("restore source folder has no id".into()))?;
        let message = self
            .get_json(&format!(
                "{GRAPH}/me/messages/{}?$select=id,parentFolderId",
                urlencode(id)
            ))
            .await?;
        if message["id"].as_str() != Some(id) {
            return Err(MailError::ParseError(
                "restore read returned a different message id".into(),
            ));
        }
        let current = message["parentFolderId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| MailError::ParseError("restore read has unknown folder".into()))?;
        if current != source_id {
            return Ok(());
        }
        self.move_message(id, "inbox").await
    }

    /// Patch exactly the authorized message, retaining immutable-ID preference.
    pub async fn set_message_read(&self, id: &str, read: bool) -> Result<(), MailError> {
        self.mutate_message(
            self.request(
                reqwest::Method::PATCH,
                &format!("{GRAPH}/me/messages/{}", urlencode(id)),
            )
            .json(&json!({"isRead":read})),
        )
        .await
    }

    async fn mutate_message(&self, request: reqwest::RequestBuilder) -> Result<(), MailError> {
        #[cfg(test)]
        if let Some(mailbox) = &self.scripted_mailbox {
            return mailbox.lock().unwrap().mutate(request.build()?);
        }
        let resp = send_with_retry(request).await?;
        self.ok_or_err(resp).await
    }

    fn move_message_request(&self, message_id: &str, folder: &str) -> reqwest::RequestBuilder {
        self.request(
            reqwest::Method::POST,
            &format!("{GRAPH}/me/messages/{}/move", urlencode(message_id)),
        )
        .json(&json!({ "destinationId": folder }))
    }

    async fn restore_thread(
        &self,
        conversation_id: &str,
        source_folder: &str,
        own_address: Option<&str>,
    ) -> Result<RestoreOutcome, MailError> {
        restore_conversation(
            conversation_id,
            source_folder,
            own_address,
            |url| async move { self.get_json(&url).await },
            |id, folder| async move { self.move_message(&id, folder).await },
        )
        .await
    }

    /// Restore only this conversation's messages currently in Archive.
    /// Drafts go to Drafts, owner-authored mail to Sent Items, other mail to
    /// Inbox. Without the own address, non-drafts stay in place and are counted.
    pub async fn unarchive_thread(
        &self,
        conversation_id: &str,
        own_address: Option<&str>,
    ) -> Result<RestoreOutcome, MailError> {
        self.restore_thread(conversation_id, "archive", own_address)
            .await
    }

    /// Restore only this conversation's messages currently in Deleted Items,
    /// using the same destination and unknown-address rules as unarchive.
    pub async fn untrash_thread(
        &self,
        conversation_id: &str,
        own_address: Option<&str>,
    ) -> Result<RestoreOutcome, MailError> {
        self.restore_thread(conversation_id, "deleteditems", own_address)
            .await
    }

    /// Move only this conversation's Inbox messages to Archive, matching
    /// Gmail's removal of INBOX. Sent Items, Drafts and other folders stay put.
    /// Well-known folder names work regardless of the mailbox's language.
    pub async fn archive_thread(
        &self,
        conversation_id: &str,
    ) -> Result<MutationOutcome, MailError> {
        self.for_each_message(conversation_id, Some("inbox"), |s, id| async move {
            s.move_message(&id, "archive").await
        })
        .await
    }

    /// Move only this conversation's Inbox mail to Junk Email. Gmail applies
    /// SPAM thread-wide, but Graph moves would also sweep in sent copies and
    /// drafts. Use Inbox as the inbound scope; already-filed mail stays put.
    pub async fn spam_thread(&self, conversation_id: &str) -> Result<MutationOutcome, MailError> {
        self.for_each_message(conversation_id, Some("inbox"), |s, id| async move {
            s.move_message(&id, "junkemail").await
        })
        .await
    }

    /// Move the whole conversation to Deleted Items, including Sent Items
    /// and Drafts, matching Gmail's threads.trash (no draft exclusion).
    /// Recoverable; nothing here can permanently delete.
    pub async fn trash_thread(&self, conversation_id: &str) -> Result<MutationOutcome, MailError> {
        self.for_each_message(conversation_id, None, |s, id| async move {
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
    ) -> Result<MutationOutcome, MailError> {
        self.for_each_message(conversation_id, None, move |s, id| async move {
            s.mutate_message(
                s.request(
                    reqwest::Method::PATCH,
                    &format!("{GRAPH}/me/messages/{}", urlencode(&id)),
                )
                .json(&json!({ "isRead": read })),
            )
            .await
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

/// A missing account address supplies no evidence that any From is foreign.
/// Draft status alone suffices for Drafts; all other ambiguous cases stay put.
fn restore_destination(message: &Value, own_address: Option<&str>) -> Option<&'static str> {
    match message["isDraft"].as_bool()? {
        true => Some("drafts"),
        false => {
            let own = own_address.map(str::trim).filter(|s| !s.is_empty())?;
            let sender = message["from"]["emailAddress"]["address"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())?;
            Some(if sender.eq_ignore_ascii_case(own) {
                "sentitems"
            } else {
                "inbox"
            })
        }
    }
}

/// The injected I/O lets tests exercise the same scoped listing, pagination,
/// routing and accounting as the public restore verbs without a mailbox.
async fn restore_conversation<F, Fetch, M, Move>(
    conversation_id: &str,
    source_folder: &str,
    own_address: Option<&str>,
    fetch: F,
    mut move_message: M,
) -> Result<RestoreOutcome, MailError>
where
    F: FnMut(String) -> Fetch,
    Fetch: std::future::Future<Output = Result<Value, MailError>>,
    M: FnMut(String, &'static str) -> Move,
    Move: std::future::Future<Output = Result<(), MailError>>,
{
    let filter = format!(
        "conversationId eq '{}'",
        conversation_id.replace('\'', "''")
    );
    let url = format!(
        "{GRAPH}/me/mailFolders/{source_folder}/messages?$filter={}&$select=id,isDraft,from&$top=100",
        urlencode(&filter)
    );
    let messages = message_pages(url, fetch).await?;
    // Validate all ids before the first move, so a malformed later message
    // cannot leave an otherwise valid restore half applied.
    let plan: Vec<_> = messages
        .iter()
        .map(|message| {
            let id = message["id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| MailError::ParseError("restore message has no id".into()))?;
            Ok((id.to_owned(), restore_destination(message, own_address)))
        })
        .collect::<Result<_, MailError>>()?;
    let mut outcome = RestoreOutcome::default();
    let mut first_error = None;
    for (id, destination) in plan {
        let Some(destination) = destination else {
            outcome.left_in_place += 1;
            continue;
        };
        match move_message(id, destination).await {
            Ok(()) => outcome.moved += 1,
            Err(error) => {
                outcome.failed += 1;
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    if let Some(error) = first_error {
        if outcome.moved == 0 {
            return Err(error);
        }
        tracing::warn!(conversation = conversation_id, source_folder, moved = outcome.moved,
            left_in_place = outcome.left_in_place, failed = outcome.failed, %error, "restore applied to part of the source folder");
    }
    Ok(outcome)
}

/// Validate before fetching: `get_json` attaches the account's bearer token.
fn validate_page_url(
    url: &str,
    seen: &mut std::collections::BTreeSet<String>,
) -> Result<(), MailError> {
    if !url.starts_with(&format!("{GRAPH}/")) || !seen.insert(url.to_owned()) {
        return Err(MailError::ParseError(
            "invalid or repeated Graph nextLink".into(),
        ));
    }
    Ok(())
}

fn next_page(page: &Value) -> Result<Option<String>, MailError> {
    match page.get("@odata.nextLink") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(next)) => Ok(Some(next.clone())),
        Some(_) => Err(MailError::ParseError("invalid Graph nextLink".into())),
    }
}

/// Stream corpus pages through the same link validation as thread reads.
async fn corpus_since<B, F, Fut>(
    since: &str,
    mut on_batch: B,
    mut fetch: F,
) -> Result<usize, MailError>
where
    B: FnMut(&[Email]) -> Result<(), MailError>,
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<Value, MailError>>,
{
    let cutoff = chrono::DateTime::parse_from_rfc3339(since)
        .map_err(|_| MailError::InvalidInput("invalid corpus cutoff timestamp".into()))?;
    const CORPUS_SELECT: &str = "id,conversationId,subject,bodyPreview,from,toRecipients,\
ccRecipients,receivedDateTime,isRead,isDraft,parentFolderId,hasAttachments,internetMessageHeaders";
    let mut url = format!(
        "{GRAPH}/me/messages?$top=100&$select={CORPUS_SELECT}&$orderby=receivedDateTime%20desc"
    );
    let mut total = 0usize;
    let mut seen = std::collections::BTreeSet::new();
    loop {
        validate_page_url(&url, &mut seen)?;
        let json = fetch(url).await?;
        let page: Vec<Email> = json["value"]
            .as_array()
            .ok_or_else(|| MailError::ParseError("Graph message page has no value array".into()))?
            .iter()
            .map(parse_outlook_message)
            .collect();
        // Newest first, so the first message older than the cutoff ends
        // the walk — everything after it is older still.
        let keep: Vec<Email> = page
            .iter()
            .take_while(|e| e.received_instant().is_some_and(|at| at >= cutoff))
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
        match next_page(&json)? {
            Some(next) => url = next,
            None => break,
        }
    }
    Ok(total)
}

/// Follow every page before returning a thread or acting on it. A failed or
/// malformed later page must not masquerade as a complete, owner-free thread.
async fn message_pages<F, Fut>(mut url: String, mut fetch: F) -> Result<Vec<Value>, MailError>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<Value, MailError>>,
{
    let mut messages = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    loop {
        validate_page_url(&url, &mut seen)?;
        let page = fetch(url).await?;
        let values = page["value"]
            .as_array()
            .ok_or_else(|| MailError::ParseError("Graph message page has no value array".into()))?;
        messages.extend(values.iter().cloned());
        match next_page(&page)? {
            None => break,
            Some(next) => url = next,
        }
    }
    Ok(messages)
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

    let parsed_date = msg["receivedDateTime"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| {
            dt.with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
        });
    let date_known = Some(parsed_date.is_some());
    let date_received =
        parsed_date.unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());

    let headers = msg["internetMessageHeaders"].as_array().map(Vec::as_slice);

    Email {
        id: format!("outlook-{provider_id}"),
        provider: "outlook".to_string(),
        provider_id,
        thread_id: msg["conversationId"].as_str().map(|s| s.to_string()),
        message_id: msg["internetMessageId"].as_str().map(|s| s.to_string()),
        subject: msg["subject"].as_str().unwrap_or_default().to_string(),
        subject_is_raw: false,
        from_address: msg["from"]["emailAddress"]["address"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        sender_known: msg["from"]["emailAddress"]["address"]
            .as_str()
            .map(|s| !s.is_empty()),
        from_name: msg["from"]["emailAddress"]["name"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        to_addresses: addr_list("toRecipients"),
        cc_addresses: addr_list("ccRecipients"),
        bcc_addresses: addr_list("bccRecipients"),
        date_received,
        date_known,
        provider_arrival_at: msg["receivedDateTime"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|at| at.with_timezone(&chrono::Utc)),
        attachment_presence: msg["hasAttachments"].as_bool(),
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
        auto_submitted: first_header(headers, "auto-submitted"),
        precedence: first_header(headers, "precedence"),
        list_id: first_header(headers, "list-id"),
        reply_to: first_header(headers, "reply-to"),
        headers: crate::types::header_values(headers),
        authentication_results: authentication_results(headers),
        // internetMessageHeaders exposes no completeness guarantee or
        // truncation marker we can rely on. A nonempty array is not proof
        // that a second provider-claiming instance was not dropped.
        headers_complete: None,
        // Observed duplicates invalidate the dedicated field too, even when
        // Graph does not guarantee completeness of the raw collection.
        subject_complete: if headers.is_some_and(|headers| {
            headers
                .iter()
                .filter(|h| {
                    h["name"]
                        .as_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case("subject"))
                })
                .count()
                > 1
        }) {
            Some(false)
        } else {
            msg["subject"].as_str().map(|_| true)
        },
        recipients_complete: ["toRecipients", "ccRecipients", "bccRecipients"]
            .iter()
            .try_fold(true, |complete, field| {
                Some(
                    complete
                        && msg[field].as_array()?.iter().all(|recipient| {
                            recipient["emailAddress"]["address"]
                                .as_str()
                                .is_some_and(|address| !address.is_empty())
                        }),
                )
            }),
        is_draft: msg["isDraft"].as_bool(),
        direction_evidence: {
            use crate::types::DirectionEvidence;
            // Graph normally returns opaque folder IDs. Only a recognized
            // well-known name proves a folder here; otherwise use isDraft and
            // the owner's address set, never guess from an opaque ID.
            match msg["parentFolderId"]
                .as_str()
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("drafts") => Some(DirectionEvidence::Draft),
                Some("sentitems") => Some(DirectionEvidence::Sent),
                _ => msg["isDraft"].as_bool().map(|draft| {
                    if draft {
                        DirectionEvidence::Draft
                    } else {
                        DirectionEvidence::NotDraft
                    }
                }),
            }
        },
    }
}

pub(crate) fn urlencode(s: &str) -> String {
    crate::google::gmail::urlencode(s)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn retrospective_listing_uses_all_folders_and_the_shared_page_ceiling() {
        let since = "2026-10-01T00:00:00Z".parse().unwrap();
        let mut provider = OutlookProvider::new("fixture-token".into()).with_immutable_ids();
        provider.scripted_reads = Some(std::sync::Arc::new(std::sync::Mutex::new(ScriptedReads {
            urls: Vec::new(), pages: [Ok(json!({"value":[{"id":"archived", "conversationId":"thread-archived", "receivedDateTime":"2026-10-15T00:00:00Z"}]}))].into()
        })));
        assert_eq!(
            provider.rule_test_hits(since).await.unwrap()[0].id,
            "archived"
        );
        let urls = provider
            .scripted_reads
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .urls
            .clone();
        assert_eq!(
            reqwest::Url::parse(&urls[0]).unwrap().path(),
            "/v1.0/me/messages"
        );
        let request = provider
            .request(reqwest::Method::GET, &urls[0])
            .build()
            .unwrap();
        assert_eq!(request.headers()["Prefer"], "IdType=\"ImmutableId\"");
        let mut pages: Vec<_> = (0..100).map(|n| Ok(json!({"value":[], "@odata.nextLink":format!("{GRAPH}/me/messages?$skiptoken=distinct-{n}")}))).collect();
        pages.push(Ok(json!({"value":[]})));
        provider.scripted_reads = Some(std::sync::Arc::new(std::sync::Mutex::new(ScriptedReads {
            urls: Vec::new(),
            pages: pages.into(),
        })));
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
            let provider = mailbox_provider().with_immutable_ids();
            let message = MessageRef {
                provider_message_id: "incoming".into(),
                ..Default::default()
            };
            crate::rules::outlook_restore(&provider, &message, action)
                .await
                .unwrap();
            let mailbox = provider.scripted_mailbox.as_ref().unwrap().lock().unwrap();
            assert!(
                mailbox.writes.is_empty(),
                "already restored must be a nothing-to-do success"
            );
            mailbox.assert_folder("incoming", "inbox");
        }
    }

    #[tokio::test]
    async fn rule_restore_preserves_immutable_id_and_leaves_thread_siblings_untouched() {
        use mecha_core::mail_rules::{MessageRef, RestoreAction};
        for action in [
            RestoreAction::Unarchive,
            RestoreAction::Untrash,
            RestoreAction::Read,
            RestoreAction::Unread,
        ] {
            let provider = mailbox_provider().with_immutable_ids();
            if matches!(action, RestoreAction::Unarchive | RestoreAction::Untrash) {
                let mut mailbox = provider.scripted_mailbox.as_ref().unwrap().lock().unwrap();
                let original = mailbox
                    .messages
                    .iter_mut()
                    .find(|m| m["id"] == "incoming")
                    .unwrap();
                original["fixtureFolder"] = json!(if action == RestoreAction::Unarchive {
                    "archive"
                } else {
                    "deleteditems"
                });
            }
            let message = MessageRef {
                provider_message_id: "incoming".into(),
                thread_id: "thread'1".into(),
                ..Default::default()
            };
            crate::rules::outlook_restore(&provider, &message, action)
                .await
                .unwrap();
            if matches!(action, RestoreAction::Unarchive | RestoreAction::Untrash) {
                crate::rules::outlook_restore(&provider, &message, action)
                    .await
                    .unwrap();
            }
            let mailbox = provider.scripted_mailbox.as_ref().unwrap().lock().unwrap();
            assert_eq!(mailbox.writes.len(), 1);
            let request = &mailbox.writes[0];
            assert!(request
                .url()
                .path()
                .starts_with("/v1.0/me/messages/incoming"));
            assert_eq!(request.headers()["Prefer"], "IdType=\"ImmutableId\"");
            if matches!(action, RestoreAction::Unarchive | RestoreAction::Untrash) {
                mailbox.assert_folder("incoming", "inbox");
            }
            mailbox.assert_folder("sent", "sentitems");
            mailbox.assert_folder("incoming2", "inbox");
        }
    }

    #[tokio::test]
    async fn distinct_empty_rule_pages_hit_a_counted_ceiling() {
        let mut provider = OutlookProvider::new("fixture-token".into()).with_immutable_ids();
        let mut pages: Vec<_> = (0..100).map(|n| Ok(json!({"value":[], "@odata.nextLink":format!("{GRAPH}/me/messages?$skiptoken=page-{n}")}))).collect();
        pages.push(Ok(json!({"value":[]})));
        provider.scripted_reads = Some(std::sync::Arc::new(std::sync::Mutex::new(ScriptedReads {
            urls: Vec::new(),
            pages: pages.into(),
        })));
        let result = provider
            .rule_thread_ids(500, "2026-11-01T00:00:00Z".parse().unwrap())
            .await;
        assert!(
            matches!(result, Err(MailError::ParseError(ref why)) if why.contains("page ceiling")),
            "unbounded empty enumeration: {result:?}"
        );
        assert!(
            provider
                .scripted_reads
                .as_ref()
                .unwrap()
                .lock()
                .unwrap()
                .urls
                .len()
                <= 100
        );
    }

    #[tokio::test]
    async fn rule_listing_refuses_unsafe_links_before_authenticated_fetch() {
        let bad = "https://example.com/steal-token";
        let mut provider = OutlookProvider::new("fixture-token".into()).with_immutable_ids();
        provider.scripted_reads = Some(std::sync::Arc::new(std::sync::Mutex::new(ScriptedReads {
            urls: Vec::new(),
            pages: [
                Ok(json!({"value":[], "@odata.nextLink":bad})),
                Ok(json!({"value":[]})),
            ]
            .into(),
        })));
        let result = provider
            .rule_thread_ids(500, "2026-11-01T00:00:00Z".parse().unwrap())
            .await;
        let script = provider.scripted_reads.as_ref().unwrap().lock().unwrap();
        assert!(
            !script.urls.iter().any(|url| url == bad),
            "credential-bearing fetch reached untrusted origin"
        );
        assert_eq!(script.urls.len(), 1);
        assert!(matches!(result, Err(MailError::ParseError(_))));
    }

    #[tokio::test]
    async fn rule_listing_rejects_malformed_pages_and_repeated_links() {
        let initial = format!("{GRAPH}/me/mailFolders/inbox/messages?$top=100&$select=id,conversationId,receivedDateTime&$orderby=receivedDateTime%20desc");
        for page in [
            json!({}),
            json!({"value":null}),
            json!({"value":[], "@odata.nextLink":false}),
            json!({"value":[], "@odata.nextLink":initial}),
        ] {
            let mut provider = OutlookProvider::new("fixture-token".into()).with_immutable_ids();
            provider.scripted_reads =
                Some(std::sync::Arc::new(std::sync::Mutex::new(ScriptedReads {
                    urls: Vec::new(),
                    pages: [Ok(page.clone())].into(),
                })));
            let result = provider
                .rule_thread_ids(500, "2026-11-01T00:00:00Z".parse().unwrap())
                .await;
            assert!(
                matches!(result, Err(MailError::ParseError(_))),
                "incomplete enumeration became success: {page}: {result:?}"
            );
            assert_eq!(
                provider
                    .scripted_reads
                    .as_ref()
                    .unwrap()
                    .lock()
                    .unwrap()
                    .urls
                    .len(),
                1
            );
        }
    }

    #[tokio::test]
    async fn corpus_cutoff_compares_fractional_and_offset_instants() {
        for since in ["2026-11-01T12:00:00Z", "2026-11-01T13:00:00+01:00"] {
            let mut kept = Vec::new();
            let total = corpus_since(
                since,
                |batch| {
                    kept.extend(batch.iter().map(|e| e.provider_id.clone()));
                    Ok(())
                },
                |_| async {
                    Ok(json!({"value":[
                        {"id":"fractional", "receivedDateTime":"2026-11-01T12:00:00.123Z"},
                        {"id":"boundary", "receivedDateTime":"2026-11-01T12:00:00Z"},
                        {"id":"old", "receivedDateTime":"2026-11-01T11:59:59Z"}
                    ]}))
                },
            )
            .await
            .unwrap();
            assert_eq!(total, 2);
            assert_eq!(kept, ["fractional", "boundary"]);
        }
    }

    use super::*;
    use serde_json::json;

    // A mailbox at the HTTP boundary: reads select the requested conversation
    // and folder, and actual mutation requests update that same state.
    #[derive(Default)]
    pub(super) struct Mailbox {
        messages: Vec<Value>,
        reads: Vec<String>,
        writes: Vec<reqwest::Request>,
        fail_moves: std::collections::BTreeSet<String>,
        fail_later_page: bool,
        failure_status: Option<u16>,
    }

    impl Mailbox {
        pub(super) fn get(&mut self, url: &str) -> Result<Value, MailError> {
            self.reads.push(url.to_owned());
            let parsed = reqwest::Url::parse(url).unwrap();
            let query: std::collections::BTreeMap<_, _> =
                parsed.query_pairs().into_owned().collect();
            if let Some(folder) = parsed.path().strip_prefix("/v1.0/me/mailFolders/") {
                if !folder.contains('/') {
                    return Ok(json!({"id":folder}));
                }
            }
            if let Some(id) = parsed.path().strip_prefix("/v1.0/me/messages/") {
                let original = self
                    .messages
                    .iter()
                    .find(|message| message["id"] == id)
                    .unwrap();
                return Ok(json!({"id":id, "parentFolderId":original["fixtureFolder"]}));
            }
            assert_eq!(query["$filter"], "conversationId eq 'thread''1'");
            if query["$top"] == "1" {
                assert_eq!(parsed.path(), "/v1.0/me/messages");
                assert_eq!(query["$select"], "id");
                assert!(!query.contains_key("$skiptoken"));
            } else {
                assert_eq!(query["$top"], "100");
            }
            let folder = if parsed.path() == "/v1.0/me/messages" {
                None
            } else {
                Some(
                    parsed
                        .path()
                        .strip_prefix("/v1.0/me/mailFolders/")
                        .unwrap()
                        .strip_suffix("/messages")
                        .unwrap(),
                )
            };
            let offset: usize = query
                .get("$skiptoken")
                .map(|s| s.parse().unwrap())
                .unwrap_or(0);
            if offset > 0 && self.fail_later_page {
                return Err(MailError::InvalidInput("fixture page failed".into()));
            }
            let selected: Vec<_> = self
                .messages
                .iter()
                .filter(|m| {
                    m["conversationId"] == "thread'1"
                        && folder.is_none_or(|f| m["fixtureFolder"] == f)
                })
                .collect();
            let mut page =
                json!({"value": selected.iter().skip(offset).take(1).collect::<Vec<_>>()});
            if offset + 1 < selected.len() {
                let mut next = parsed.clone();
                next.set_query(None);
                next.query_pairs_mut()
                    .extend_pairs(query.iter().filter(|(key, _)| key.as_str() != "$skiptoken"))
                    .append_pair("$skiptoken", &(offset + 1).to_string());
                page["@odata.nextLink"] = json!(next.as_str());
            }
            Ok(page)
        }

        pub(super) fn mutate(&mut self, request: reqwest::Request) -> Result<(), MailError> {
            let path = request
                .url()
                .path()
                .strip_prefix("/v1.0/me/messages/")
                .unwrap();
            let body: Value =
                serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
            let id = path.strip_suffix("/move").unwrap_or(path);
            let message = self.messages.iter_mut().find(|m| m["id"] == id).unwrap();
            let failed = self.fail_moves.contains(id);
            if path.ends_with("/move") {
                assert_eq!(request.method(), reqwest::Method::POST);
                assert_eq!(body.as_object().unwrap().len(), 1);
                if !failed {
                    message["fixtureFolder"] = body["destinationId"].clone();
                }
            } else {
                assert_eq!(request.method(), reqwest::Method::PATCH);
                assert_eq!(body.as_object().unwrap().len(), 1);
                message["isRead"] = body["isRead"].clone();
            }
            self.writes.push(request);
            if failed {
                Err(self.failure_status.map_or_else(
                    || MailError::InvalidInput("fixture move failed".into()),
                    |status| MailError::ApiError {
                        status,
                        message: "fixture outage".into(),
                    },
                ))
            } else {
                Ok(())
            }
        }

        fn assert_reads(&self, source: Option<&str>, select: &str, count: usize) {
            assert_eq!(self.reads.len(), count);
            for url in &self.reads {
                let url = reqwest::Url::parse(url).unwrap();
                assert_eq!(
                    url.path(),
                    source.map_or_else(
                        || "/v1.0/me/messages".to_owned(),
                        |s| format!("/v1.0/me/mailFolders/{s}/messages")
                    )
                );
                assert!(url
                    .query_pairs()
                    .any(|(k, v)| k == "$select" && v == select));
            }
        }

        fn assert_folder(&self, id: &str, folder: &str) {
            assert_eq!(
                self.messages.iter().find(|m| m["id"] == id).unwrap()["fixtureFolder"],
                folder
            );
        }

        fn assert_moves(&self, ids: &[&str], destination: &str) {
            assert_eq!(self.writes.len(), ids.len());
            for (request, id) in self.writes.iter().zip(ids) {
                assert_eq!(request.method(), reqwest::Method::POST);
                assert_eq!(request.url().path(), format!("/v1.0/me/messages/{id}/move"));
                let body: Value =
                    serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
                assert_eq!(body, json!({"destinationId":destination}));
            }
        }
    }

    fn mailbox_provider() -> OutlookProvider {
        let mut messages = vec![
            restore_fixture("incoming", "inbox", "sender@example.edu", false),
            restore_fixture("sent", "sentitems", "owner@example.com", false),
            restore_fixture("draft", "drafts", "owner@example.com", true),
            restore_fixture("filed", "custom", "sender@example.edu", false),
            restore_fixture("incoming2", "inbox", "sender@example.edu", false),
        ];
        for message in &mut messages {
            message["conversationId"] = json!("thread'1");
        }
        let mut unrelated = restore_fixture("unrelated", "inbox", "sender@example.edu", false);
        unrelated["conversationId"] = json!("another-thread");
        messages.push(unrelated);
        let mut provider = OutlookProvider::new("fixture-token".into());
        provider.scripted_mailbox = Some(std::sync::Arc::new(std::sync::Mutex::new(Mailbox {
            messages,
            ..Default::default()
        })));
        provider
    }

    #[tokio::test]
    async fn mail_reply_and_search_order_mixed_timestamp_precision_chronologically() {
        let newer = json!({"id":"newer", "subject":"Meeting agenda", "from":{"emailAddress":{"address":"newer@example.edu"}}, "receivedDateTime":"2026-11-03T12:00:00.100Z"});
        let older = json!({"id":"older", "from":{"emailAddress":{"address":"older@example.edu"}}, "receivedDateTime":"2026-11-03T12:00:00Z"});
        let mut provider = OutlookProvider::new("fixture-token".into());
        provider.scripted_reads = Some(std::sync::Arc::new(std::sync::Mutex::new(ScriptedReads {
            urls: Vec::new(),
            pages: [
                Ok(json!({"value":[newer.clone(), older.clone()]})),
                Ok(json!({"value":[older, newer]})),
            ]
            .into(),
        })));
        let thread = provider.get_thread("thread-alpha").await.unwrap();
        let target = crate::unified::reply_target(&thread, None).unwrap();
        assert_eq!(target.provider_id, "newer");
        assert_eq!(target.from_address, "newer@example.edu");
        let found = provider.search("meeting", 10).await.unwrap();
        assert_eq!(found[0].provider_id, "newer");
    }

    #[tokio::test]
    async fn rule_actions_touch_only_the_authorized_immutable_message() {
        use mecha_core::mail_rules::{ActionVerb, MessageRef};
        for action in [
            ActionVerb::Archive,
            ActionVerb::Trash,
            ActionVerb::Read,
            ActionVerb::Unread,
        ] {
            let provider = mailbox_provider().with_immutable_ids();
            let message = MessageRef {
                thread_id: "thread'1".into(),
                provider_message_id: "incoming".into(),
                ..Default::default()
            };
            crate::rules::outlook_action(&provider, &message, action)
                .await
                .unwrap();
            let mailbox = provider.scripted_mailbox.as_ref().unwrap().lock().unwrap();
            // Older, pre-activation messages and a newly arrived nonmatch
            // share the conversation. Neither is authorized by this candidate.
            assert_eq!(mailbox.writes.len(), 1);
            assert!(mailbox.reads.is_empty());
            let request = &mailbox.writes[0];
            assert!(request
                .url()
                .path()
                .starts_with("/v1.0/me/messages/incoming"));
            assert_eq!(request.headers()["Prefer"], "IdType=\"ImmutableId\"");
            mailbox.assert_folder("sent", "sentitems");
            mailbox.assert_folder("incoming2", "inbox");
        }
    }

    #[tokio::test]
    async fn partial_environment_failures_keep_their_kind() {
        for status in [
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
        ] {
            let provider = mailbox_provider();
            {
                let mut mailbox = provider.scripted_mailbox.as_ref().unwrap().lock().unwrap();
                mailbox.fail_moves.insert("incoming2".into());
                mailbox.failure_status = Some(status.as_u16());
            }
            let outcome = provider.archive_thread("thread'1").await.unwrap();
            assert_eq!(outcome.moved, 1);
            assert_eq!(outcome.failed, 1);
            assert!(matches!(
                crate::rules::triage_result(crate::unified::TriageResult::Count(outcome)),
                mecha_core::mail_rules::engine::ActionResult::Failed {
                    kind: mecha_core::mail_rules::FailureKind::Environment,
                    ..
                }
            ));
        }
    }

    #[tokio::test]
    async fn archive_and_spam_only_move_inbox_mail_across_all_pages() {
        for destination in ["archive", "junkemail"] {
            let provider = mailbox_provider();
            let count = if destination == "archive" {
                provider.archive_thread("thread'1").await
            } else {
                provider.spam_thread("thread'1").await
            }
            .unwrap();
            assert_eq!(
                count,
                MutationOutcome {
                    moved: 2,
                    failed: 0,
                    ..Default::default()
                }
            );
            let mailbox = provider.scripted_mailbox.as_ref().unwrap().lock().unwrap();
            mailbox.assert_reads(Some("inbox"), "id", 2);
            mailbox.assert_moves(&["incoming", "incoming2"], destination);
            mailbox.assert_folder("sent", "sentitems");
            mailbox.assert_folder("draft", "drafts");
            mailbox.assert_folder("filed", "custom");
            mailbox.assert_folder("unrelated", "inbox");
        }
    }

    #[tokio::test]
    async fn archive_then_unarchive_round_trips_only_inbox_messages() {
        let provider = mailbox_provider();
        assert_eq!(
            provider.archive_thread("thread'1").await.unwrap(),
            MutationOutcome {
                moved: 2,
                failed: 0,
                ..Default::default()
            }
        );
        {
            let mut mailbox = provider.scripted_mailbox.as_ref().unwrap().lock().unwrap();
            mailbox.assert_moves(&["incoming", "incoming2"], "archive");
            mailbox.assert_folder("incoming", "archive");
            mailbox.assert_folder("sent", "sentitems");
            mailbox.assert_folder("draft", "drafts");
            mailbox.reads.clear();
            mailbox.writes.clear();
        }
        let outcome = provider
            .unarchive_thread("thread'1", Some("owner@example.com"))
            .await
            .unwrap();
        assert_eq!(
            outcome,
            RestoreOutcome {
                moved: 2,
                ..Default::default()
            }
        );
        let mailbox = provider.scripted_mailbox.as_ref().unwrap().lock().unwrap();
        mailbox.assert_reads(Some("archive"), "id,isDraft,from", 2);
        mailbox.assert_moves(&["incoming", "incoming2"], "inbox");
        mailbox.assert_folder("incoming", "inbox");
        mailbox.assert_folder("sent", "sentitems");
        mailbox.assert_folder("draft", "drafts");
        mailbox.assert_folder("filed", "custom");
        mailbox.assert_folder("unrelated", "inbox");
    }

    #[tokio::test]
    async fn trash_still_moves_the_whole_conversation_and_untrash_restores_roles() {
        let provider = mailbox_provider();
        assert_eq!(
            provider.trash_thread("thread'1").await.unwrap(),
            MutationOutcome {
                moved: 5,
                failed: 0,
                ..Default::default()
            }
        );
        {
            let mut mailbox = provider.scripted_mailbox.as_ref().unwrap().lock().unwrap();
            mailbox.assert_reads(None, "id", 5);
            mailbox.assert_moves(
                &["incoming", "sent", "draft", "filed", "incoming2"],
                "deleteditems",
            );
            mailbox.reads.clear();
            mailbox.writes.clear();
        }
        let outcome = provider
            .untrash_thread("thread'1", Some("owner@example.com"))
            .await
            .unwrap();
        assert_eq!(
            outcome,
            RestoreOutcome {
                moved: 5,
                ..Default::default()
            }
        );
        let mailbox = provider.scripted_mailbox.as_ref().unwrap().lock().unwrap();
        mailbox.assert_reads(Some("deleteditems"), "id,isDraft,from", 5);
        mailbox.assert_folder("incoming", "inbox");
        mailbox.assert_folder("sent", "sentitems");
        mailbox.assert_folder("draft", "drafts");
        mailbox.assert_folder("unrelated", "inbox");
    }

    #[tokio::test]
    async fn read_and_unread_still_patch_the_whole_conversation_without_moves() {
        for read in [true, false] {
            let provider = mailbox_provider();
            assert_eq!(
                provider.set_thread_read("thread'1", read).await.unwrap(),
                MutationOutcome {
                    moved: 5,
                    failed: 0,
                    ..Default::default()
                }
            );
            let mailbox = provider.scripted_mailbox.as_ref().unwrap().lock().unwrap();
            mailbox.assert_reads(None, "id", 5);
            assert_eq!(mailbox.writes.len(), 5);
            for (request, id) in
                mailbox
                    .writes
                    .iter()
                    .zip(["incoming", "sent", "draft", "filed", "incoming2"])
            {
                assert_eq!(request.method(), reqwest::Method::PATCH);
                assert_eq!(request.url().path(), format!("/v1.0/me/messages/{id}"));
                let body: Value =
                    serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
                assert_eq!(body, json!({"isRead":read}));
            }
            mailbox.assert_folder("incoming", "inbox");
            mailbox.assert_folder("sent", "sentitems");
            mailbox.assert_folder("draft", "drafts");
            mailbox.assert_folder("filed", "custom");
            assert!(mailbox.messages.last().unwrap().get("isRead").is_none());
        }
    }

    #[tokio::test]
    async fn empty_inbox_does_not_move_other_conversation_messages() {
        for spam in [false, true] {
            let provider = mailbox_provider();
            provider
                .scripted_mailbox
                .as_ref()
                .unwrap()
                .lock()
                .unwrap()
                .messages
                .retain(|m| m["fixtureFolder"] != "inbox");
            let result = if spam {
                provider.spam_thread("thread'1").await
            } else {
                provider.archive_thread("thread'1").await
            };
            let outcome = result.unwrap();
            assert_eq!(outcome, MutationOutcome::default());
            let action = if spam {
                crate::unified::TriageAction::Spam
            } else {
                crate::unified::TriageAction::Archive
            };
            let reply = crate::unified::triage_reply(
                "fixture",
                "thread'1",
                action,
                crate::unified::TriageResult::Count(outcome),
            );
            assert_eq!(reply.structured_content.unwrap()["status"], "nothing_to_do");
            let mailbox = provider.scripted_mailbox.as_ref().unwrap().lock().unwrap();
            assert_eq!(mailbox.reads.len(), 2);
            let first = reqwest::Url::parse(&mailbox.reads[0]).unwrap();
            assert_eq!(first.path(), "/v1.0/me/mailFolders/inbox/messages");
            let probe = reqwest::Url::parse(&mailbox.reads[1]).unwrap();
            assert_eq!(probe.path(), "/v1.0/me/messages");
            let query: std::collections::BTreeMap<_, _> =
                probe.query_pairs().into_owned().collect();
            assert_eq!(query["$filter"], "conversationId eq 'thread''1'");
            assert_eq!(query["$select"], "id");
            assert_eq!(query["$top"], "1");
            assert!(mailbox.writes.is_empty());
        }
    }

    #[tokio::test]
    async fn archive_and_spam_reject_a_conversation_missing_from_every_folder() {
        for spam in [false, true] {
            let provider = mailbox_provider();
            // Keep unrelated mail: finding any message in the mailbox is
            // insufficient evidence that this particular conversation exists.
            provider
                .scripted_mailbox
                .as_ref()
                .unwrap()
                .lock()
                .unwrap()
                .messages
                .retain(|m| m["conversationId"] != "thread'1");
            let result = if spam {
                provider.spam_thread("thread'1").await
            } else {
                provider.archive_thread("thread'1").await
            };
            assert!(
                matches!(result, Err(MailError::ApiError { status: 404, .. })),
                "{result:?}"
            );
            let mailbox = provider.scripted_mailbox.as_ref().unwrap().lock().unwrap();
            assert_eq!(mailbox.reads.len(), 2);
            assert!(mailbox.writes.is_empty());
        }
    }

    #[tokio::test]
    async fn an_unreadable_existence_probe_does_not_claim_nothing_to_do() {
        for probe in [
            Err(MailError::ApiError {
                status: 503,
                message: "fixture unavailable".into(),
            }),
            Ok(json!({})),
            Ok(json!({"value":[{}]})),
            Ok(json!({"value":[{"id":""}]})),
        ] {
            let mut provider = OutlookProvider::new("fixture-token".into());
            provider.scripted_reads =
                Some(std::sync::Arc::new(std::sync::Mutex::new(ScriptedReads {
                    urls: Vec::new(),
                    pages: [Ok(json!({"value":[]})), probe].into(),
                })));
            assert!(provider.archive_thread("thread'1").await.is_err());
            assert_eq!(
                provider
                    .scripted_reads
                    .as_ref()
                    .unwrap()
                    .lock()
                    .unwrap()
                    .urls
                    .len(),
                2
            );
        }
    }

    #[tokio::test]
    async fn inbox_moves_preserve_partial_failure_counts_and_read_before_writing() {
        for spam in [false, true] {
            for (failed_ids, fail_page) in [
                (vec!["incoming"], false),
                (vec!["incoming", "incoming2"], false),
                (vec![], true),
            ] {
                let provider = mailbox_provider();
                {
                    let mut mailbox = provider.scripted_mailbox.as_ref().unwrap().lock().unwrap();
                    mailbox.fail_moves = failed_ids.iter().map(|id| (*id).to_owned()).collect();
                    mailbox.fail_later_page = fail_page;
                }
                let result = if spam {
                    provider.spam_thread("thread'1").await
                } else {
                    provider.archive_thread("thread'1").await
                };
                if fail_page || failed_ids.len() == 2 {
                    assert!(result.is_err());
                } else {
                    assert_eq!(
                        result.unwrap(),
                        MutationOutcome {
                            moved: 1,
                            failed: 1,
                            failure_kind: Some(MutationFailureKind::Action)
                        }
                    );
                }
                let mailbox = provider.scripted_mailbox.as_ref().unwrap().lock().unwrap();
                if fail_page {
                    assert!(mailbox.writes.is_empty());
                } else {
                    mailbox.assert_moves(
                        &["incoming", "incoming2"],
                        if spam { "junkemail" } else { "archive" },
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn rule_thread_listing_paginates_and_keeps_unknown_arrivals_within_bound() {
        let mut provider = OutlookProvider::new("fixture-token".into()).with_immutable_ids();
        provider.scripted_reads = Some(std::sync::Arc::new(std::sync::Mutex::new(ScriptedReads {
            urls: Vec::new(),
            pages: [Ok(json!({"value":[
                {"id":"boundary", "conversationId":"thread-boundary", "receivedDateTime":"2026-11-01T00:00:00Z"},
                {"id":"old", "conversationId":"thread-old", "receivedDateTime":"2026-10-01T00:00:00Z"}
            ], "@odata.nextLink":format!("{GRAPH}/me/mailFolders/inbox/messages?$skiptoken=next&$select=id,conversationId,receivedDateTime")})),
            Ok(json!({"value":[
                {"id":"unknown", "conversationId":"thread-unknown", "receivedDateTime":"invalid"},
                {"id":"missing", "conversationId":"thread-missing"},
                {"id":"excess", "conversationId":"thread-excess"}
            ], "@odata.nextLink":"never-requested"}))].into(),
        })));
        let ids = provider
            .rule_thread_ids(4, "2026-11-01T00:00:00Z".parse().unwrap())
            .await
            .unwrap();
        assert_eq!(ids, ["thread-boundary", "thread-unknown", "thread-missing"]);
        let script = provider.scripted_reads.as_ref().unwrap().lock().unwrap();
        assert_eq!(script.urls.len(), 2);
        assert!(script.pages.is_empty());
        for url in &script.urls {
            let request = provider.request(reqwest::Method::GET, url).build().unwrap();
            assert_eq!(request.headers()["Prefer"], "IdType=\"ImmutableId\"");
            assert!(request
                .url()
                .query_pairs()
                .any(|(k, v)| k == "$select" && v == "id,conversationId,receivedDateTime"));
        }
    }

    #[tokio::test]
    async fn rule_thread_listing_never_requests_bodies() {
        let mut provider = OutlookProvider::new("fixture-token".into()).with_immutable_ids();
        provider.scripted_reads = Some(std::sync::Arc::new(std::sync::Mutex::new(ScriptedReads {
            urls: Vec::new(),
            pages: [Ok(json!({"value":[
                {"id":"recent", "conversationId":"thread-recent", "receivedDateTime":"2026-11-03T00:00:00Z"},
                {"id":"unknown", "conversationId":"thread-unknown"},
                {"id":"old", "conversationId":"thread-old", "receivedDateTime":"2026-10-01T00:00:00Z"}
            ]}))].into(),
        })));
        let ids = provider
            .rule_thread_ids(500, "2026-11-01T00:00:00Z".parse().unwrap())
            .await
            .unwrap();
        assert_eq!(ids, ["thread-recent", "thread-unknown"]);
        let script = provider.scripted_reads.as_ref().unwrap().lock().unwrap();
        assert_eq!(script.urls.len(), 1);
        let url = reqwest::Url::parse(&script.urls[0]).unwrap();
        assert!(url.path().ends_with("/mailFolders/inbox/messages"));
        assert_eq!(
            url.query_pairs().find(|(k, _)| k == "$select").unwrap().1,
            "id,conversationId,receivedDateTime"
        );
        let request = provider
            .request(reqwest::Method::GET, url.as_str())
            .build()
            .unwrap();
        assert_eq!(request.headers()["Prefer"], "IdType=\"ImmutableId\"");
    }

    fn paginated_provider() -> OutlookProvider {
        let mut provider = OutlookProvider::new("fixture-token".into());
        provider.scripted_reads = Some(std::sync::Arc::new(std::sync::Mutex::new(ScriptedReads {
            urls: Vec::new(),
            pages: [
                Ok(json!({"value":[{
                    "id":"newer", "conversationId":"thread-1", "isDraft":false,
                    "from":{"emailAddress":{"address":"sender@example.edu"}},
                    "receivedDateTime":"2024-02-01T00:00:00Z"
                }], "@odata.nextLink":format!("{GRAPH}/me/messages?$skiptoken=fixture")})),
                Ok(json!({"value":[{
                    "id":"older", "conversationId":"thread-1", "isDraft":false,
                    "from":{"emailAddress":{"address":"owner@example.com"}},
                    "receivedDateTime":"2024-01-01T00:00:00Z"
                }]})),
            ]
            .into(),
        })));
        provider
    }

    fn assert_followed_both_pages(provider: &OutlookProvider) {
        let script = provider.scripted_reads.as_ref().unwrap().lock().unwrap();
        assert!(
            script.pages.is_empty(),
            "provider must consume the later page"
        );
        assert_eq!(script.urls.len(), 2);
        assert_eq!(
            script.urls[1],
            format!("{GRAPH}/me/messages?$skiptoken=fixture")
        );
        let first = reqwest::Url::parse(&script.urls[0]).unwrap();
        assert!(first
            .query_pairs()
            .any(|(key, value)| key == "$top" && value == "100"));
    }

    #[tokio::test]
    async fn get_thread_follows_later_pages_and_requests_one_hundred_messages() {
        let provider = paginated_provider();
        let emails = provider.get_thread("thread-1").await.unwrap();
        assert_eq!(
            emails
                .iter()
                .map(|e| e.provider_id.as_str())
                .collect::<Vec<_>>(),
            ["older", "newer"]
        );
        assert_eq!(
            crate::types::ThreadMetadata::from_emails(&emails, Some("owner@example.com"))
                .owner_participated,
            Some(true)
        );
        assert_followed_both_pages(&provider);
    }

    #[tokio::test]
    async fn conversation_message_ids_follows_later_pages() {
        let provider = paginated_provider();
        assert_eq!(
            provider
                .conversation_message_ids("thread-1", None)
                .await
                .unwrap(),
            ["newer", "older"]
        );
        assert_followed_both_pages(&provider);
    }

    #[tokio::test]
    async fn page_since_follows_later_pages() {
        let provider = paginated_provider();
        let mut batches = Vec::new();
        let total = provider
            .page_since("2024-01-01T00:00:00Z", |batch| {
                batches.push(
                    batch
                        .iter()
                        .map(|e| e.provider_id.clone())
                        .collect::<Vec<_>>(),
                );
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(total, 2);
        assert_eq!(batches, [vec!["newer"], vec!["older"]]);
        assert_followed_both_pages(&provider);
    }

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
            "internetMessageId": "<mid@example.edu>",
            "subject": "Grant deadline", "bodyPreview": "quick note",
            "body": {"contentType": "html", "content": "<p>quick note</p>"},
            "from": {"emailAddress": {"name": "Priya Nair", "address": "priya@example.edu"}},
            "toRecipients": [{"emailAddress": {"address": "robin@example.edu"}}],
            "receivedDateTime": "2026-08-04T15:30:00Z",
            "isRead": false, "hasAttachments": true,
            "flag": {"flagStatus": "flagged"}
        });
        let e = parse_outlook_message(&msg);
        assert_eq!(e.id, "outlook-AAMk123");
        assert_eq!(e.thread_id.as_deref(), Some("conv-1"));
        assert_eq!(e.from_name, "Priya Nair");
        assert_eq!(e.to_addresses, vec!["robin@example.edu"]);
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
    #[test]
    fn graph_unknown_inbound_dates_prevent_newest_selection() {
        use crate::types::ThreadMetadata;
        let mut message = json!({"id":"known", "isDraft":false,
            "from":{"emailAddress":{"address":"sender@example.edu"}},
            "receivedDateTime":"2024-01-01T01:00:00+01:00"});
        let known = parse_outlook_message(&message);
        assert_eq!(known.date_known, Some(true));
        assert_eq!(known.date_received, "2024-01-01T00:00:00Z");
        for date in [None, Some(json!("not a date")), Some(json!(""))] {
            message.as_object_mut().unwrap().remove("receivedDateTime");
            if let Some(date) = date {
                message["receivedDateTime"] = date;
            }
            let unknown = parse_outlook_message(&message);
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
    }

    #[tokio::test]
    async fn graph_corpus_pages_reject_unsafe_repeated_and_nonstring_links() {
        for link in [
            json!(123),
            json!({}),
            json!(""),
            json!("https://example.com/untrusted"),
            json!("https://graph.microsoft.com.example.com/v1.0/me/messages"),
            json!(format!("{GRAPH}/me/messages?next")),
        ] {
            let mut replies = vec![
                json!({"value":[], "@odata.nextLink":format!("{GRAPH}/me/messages?next")}),
                json!({"value":[], "@odata.nextLink":link}),
            ]
            .into_iter();
            let result = corpus_since(
                "2024-01-01T00:00:00Z",
                |_| Ok(()),
                |_| std::future::ready(Ok(replies.next().expect("no fetch after invalid link"))),
            )
            .await;
            assert!(result.is_err());
            assert!(replies.next().is_none());
        }
    }

    #[tokio::test]
    async fn graph_corpus_pages_select_drafts_stream_batches_and_stop_at_cutoff() {
        let mut visits = 0;
        let mut emails = Vec::new();
        let total = corpus_since("2024-01-01T00:00:00Z", |batch| {
            emails.extend_from_slice(batch);
            Ok(())
        }, |url| {
            visits += 1;
            let page = match visits {
                1 => {
                    let url = reqwest::Url::parse(&url).unwrap();
                    let select = url.query_pairs().find(|(k, _)| k == "$select").unwrap().1;
                    assert!(select.split(',').any(|field| field == "isDraft"));
                    json!({"value":[{"id":"draft", "isDraft":true, "receivedDateTime":"2024-02-01T00:00:00Z"}],
                        "@odata.nextLink":format!("{GRAPH}/me/messages?next")})
                }
                2 => {
                    assert_eq!(url, format!("{GRAPH}/me/messages?next"));
                    json!({"value":[
                        {"id":"sent", "isDraft":false, "receivedDateTime":"2024-01-01T00:00:00Z"},
                        {"id":"old", "isDraft":false, "receivedDateTime":"2023-12-01T00:00:00Z"}],
                        "@odata.nextLink":format!("{GRAPH}/me/messages?older")})
                }
                _ => panic!("no fetch beyond cutoff"),
            };
            std::future::ready(Ok(page))
        }).await.unwrap();
        assert_eq!(total, 2);
        assert_eq!(visits, 2);
        assert_eq!(emails[0].is_draft, Some(true));
        assert_eq!(emails[1].is_draft, Some(false));
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
        parse_outlook_message(
            &json!({"id":"message-1", "conversationId":"thread-1", "internetMessageHeaders":headers}),
        )
    }

    #[test]
    fn graph_authentication_results_expose_missing_foreign_duplicate_and_single_claims() {
        assert!(with_authentication_results(&[])
            .authentication_results
            .is_none());
        assert!(parse_outlook_message(&json!({}))
            .authentication_results
            .is_none());
        let eop = "spf=pass; dmarc=pass header.from=example.edu; compauth=pass reason=100";
        let submitted = "relay.example.com; dmarc=pass header.from=example.com; compauth=pass";
        let foreign = "relay.example.com; dmarc=pass header.from=example.edu";
        for (values, claims) in [
            (vec![foreign], 0),
            (vec![eop, submitted], 2),
            (vec![submitted, eop], 2),
            (vec![eop], 1),
            (vec![foreign, eop], 1),
        ] {
            let email = with_authentication_results(&values);
            let instances = email.authentication_results.unwrap();
            assert_eq!(instances.len(), values.len());
            // Count both EOP's id-less claim and a submitted compauth claim
            // with a foreign id. P0 exposes these; a future guard decides.
            assert_eq!(
                instances
                    .iter()
                    .filter(|r| r
                        .value
                        .split(';')
                        .any(|part| part.trim_start().starts_with("compauth=")))
                    .count(),
                claims
            );
            assert_eq!(instances.first().unwrap().value, values[0]);
            assert_eq!(
                instances
                    .iter()
                    .map(|r| r.value.as_str())
                    .collect::<Vec<_>>(),
                values
            );
            for instance in &instances {
                assert_eq!(
                    instance.authserv_id.as_deref(),
                    if instance.value == eop {
                        None
                    } else {
                        Some("relay.example.com")
                    }
                );
            }
            // Even the positive fixture is not evidence that Graph returned
            // the whole collection. A future send guard must still hold.
            assert_eq!(email.headers_complete, None);
        }
    }

    #[test]
    fn graph_exposes_rule_headers_and_never_infers_header_completeness() {
        let email = parse_outlook_message(&json!({
            "id":"provider-message", "conversationId":"provider-thread", "isDraft":false,
            "internetMessageId":"<sender-controlled@example.com>",
            "internetMessageHeaders":[
                {"name":"Auto-Submitted", "value":"auto-generated"},
                {"name":"Precedence", "value":"bulk"},
                {"name":"List-Id", "value":"<notices.example.edu>"},
                {"name":"Reply-To", "value":"reply@example.com"}
            ]
        }));
        assert_eq!(email.provider_id, "provider-message");
        assert_eq!(email.thread_id.as_deref(), Some("provider-thread"));
        assert_eq!(email.auto_submitted.as_deref(), Some("auto-generated"));
        assert_eq!(email.precedence.as_deref(), Some("bulk"));
        assert_eq!(email.list_id.as_deref(), Some("<notices.example.edu>"));
        assert_eq!(email.reply_to.as_deref(), Some("reply@example.com"));
        assert!(email.authentication_results.is_none());
        assert_eq!(email.headers_complete, None);
        assert_eq!(email.is_draft, Some(false));
        assert_eq!(
            parse_outlook_message(&json!({"isDraft":true})).is_draft,
            Some(true)
        );
        let missing = parse_outlook_message(&json!({}));
        assert!(
            missing.auto_submitted.is_none()
                && missing.precedence.is_none()
                && missing.list_id.is_none()
                && missing.reply_to.is_none()
        );
        assert_eq!(missing.is_draft, None);
        assert!(SELECT.split(',').any(|field| field == "isDraft"));
        assert!(SELECT
            .split(',')
            .any(|field| field == "internetMessageHeaders"));
    }

    #[test]
    fn graph_move_requests_keep_the_opt_in_id_mode() {
        let default = OutlookProvider::new("fixture-token".into());
        let immutable = default.clone().with_immutable_ids();
        for (provider, preference) in [
            (&default, None),
            (&immutable, Some("IdType=\"ImmutableId\"")),
        ] {
            for folder in ["archive", "deleteditems", "inbox", "sentitems", "drafts"] {
                let request = provider
                    .move_message_request("message/1", folder)
                    .build()
                    .unwrap();
                assert_eq!(request.method(), reqwest::Method::POST);
                assert_eq!(
                    request.url().as_str(),
                    "https://graph.microsoft.com/v1.0/me/messages/message%2F1/move"
                );
                let body: Value =
                    serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
                assert_eq!(body, json!({"destinationId":folder}));
                assert_eq!(
                    request.headers().get("Prefer").map(|v| v.to_str().unwrap()),
                    preference
                );
            }
            for method in [
                reqwest::Method::GET,
                reqwest::Method::POST,
                reqwest::Method::PATCH,
            ] {
                let request = provider
                    .request(method, &format!("{GRAPH}/me/messages"))
                    .build()
                    .unwrap();
                assert_eq!(
                    request.headers().get("Prefer").map(|v| v.to_str().unwrap()),
                    preference
                );
            }
        }
    }

    #[tokio::test]
    async fn graph_thread_pages_include_later_owner_participation_and_newest_inbound() {
        let first = format!("{GRAPH}/me/messages?$filter=fixture");
        let second = format!("{GRAPH}/me/messages?$skiptoken=next");
        let mut visited = Vec::new();
        let messages = message_pages(first.clone(), |url| {
            visited.push(url.clone());
            let page = if url == first {
                json!({"value":[{
                    "id":"old-inbound", "conversationId":"thread-1", "isDraft":false,
                    "from":{"emailAddress":{"address":"sender@example.edu"}},
                    "receivedDateTime":"2026-01-01T12:00:00Z"
                }], "@odata.nextLink":second})
            } else {
                assert_eq!(url, second);
                json!({"value":[{
                    "id":"new-inbound", "conversationId":"thread-1", "isDraft":false,
                    "from":{"emailAddress":{"address":"sender@example.edu"}},
                    "receivedDateTime":"2026-01-02T12:00:00Z"
                }, {
                    "id":"owner-reply", "conversationId":"thread-1", "isDraft":false,
                    "from":{"emailAddress":{"address":"owner@example.com"}},
                    "receivedDateTime":"2026-01-03T12:00:00Z"
                }]})
            };
            std::future::ready(Ok(page))
        })
        .await
        .unwrap();
        assert_eq!(visited, [first, second]);
        // This same page walker feeds the folder-scoped restore listing.
        assert_eq!(
            messages
                .iter()
                .map(|m| m["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["old-inbound", "new-inbound", "owner-reply"]
        );
        let emails: Vec<_> = messages.iter().map(parse_outlook_message).collect();
        let metadata =
            crate::types::ThreadMetadata::from_emails(&emails, Some("owner@example.com"));
        assert_eq!(metadata.owner_participated, Some(true));
        assert_eq!(
            metadata.newest_inbound_provider_id.as_deref(),
            Some("new-inbound")
        );
    }

    #[tokio::test]
    async fn graph_thread_pages_fail_closed_on_later_errors_and_bad_envelopes() {
        for later in [
            Err(MailError::ApiError {
                status: 503,
                message: "fixture failure".into(),
            }),
            Ok(json!({"error":{"code":"fixture"}})),
            Ok(json!({"value":[], "@odata.nextLink":123})),
            Ok(json!({"value":[], "@odata.nextLink":"https://example.com/untrusted"})),
            Ok(json!({"value":[], "@odata.nextLink":format!("{GRAPH}/me/messages?next")})),
        ] {
            let mut replies = vec![Ok(json!({"value":[{"id":"first"}], "@odata.nextLink":format!("{GRAPH}/me/messages?next")})), later].into_iter();
            let result = message_pages(format!("{GRAPH}/me/messages"), |_| {
                std::future::ready(replies.next().expect("no request past a bad page"))
            })
            .await;
            assert!(result.is_err());
        }
    }
    fn restore_fixture(id: &str, source: &str, sender: &str, draft: bool) -> Value {
        json!({"id":id, "fixtureFolder":source, "isDraft":draft,
            "from":{"emailAddress":{"address":sender}}})
    }

    #[tokio::test]
    async fn restores_are_source_scoped_paginated_and_route_each_message_to_its_role() {
        for source in ["archive", "deleteditems"] {
            let mailbox = [
                restore_fixture("incoming", source, "sender@example.edu", false),
                restore_fixture("outgoing", source, "OWNER@example.com", false),
                restore_fixture("draft", source, "owner@example.com", true),
                restore_fixture("untouched-inbox", "inbox", "sender@example.edu", false),
                restore_fixture("untouched-sent", "sentitems", "owner@example.com", false),
                restore_fixture("untouched-draft", "drafts", "owner@example.com", true),
                restore_fixture(
                    "untouched-other-source",
                    if source == "archive" {
                        "deleteditems"
                    } else {
                        "archive"
                    },
                    "sender@example.edu",
                    false,
                ),
            ];
            let mut reads = Vec::new();
            let mut moves = Vec::new();
            let outcome = restore_conversation("thread'1", source, Some("owner@example.com"), |url| {
                let parsed = reqwest::Url::parse(&url).unwrap();
                assert_eq!(parsed.path(), format!("/v1.0/me/mailFolders/{source}/messages"));
                let query: std::collections::BTreeMap<_, _> = parsed.query_pairs().into_owned().collect();
                assert_eq!(query["$filter"], "conversationId eq 'thread''1'");
                assert_eq!(query["$select"], "id,isDraft,from");
                let offset = if query.contains_key("$skiptoken") { 2 } else { 0 };
                // The fixture server selects by the requested folder. An
                // unscoped /me/messages listing fails the path assertion.
                let selected: Vec<_> = mailbox.iter().filter(|m| m["fixtureFolder"] == source).skip(offset).take(2).cloned().collect();
                let page = if offset == 0 {
                    json!({"value":selected, "@odata.nextLink":format!("{url}&$skiptoken=page2")})
                } else { json!({"value":selected}) };
                reads.push(url);
                std::future::ready(Ok(page))
            }, |id, folder| {
                moves.push((id, folder));
                std::future::ready(Ok(()))
            }).await.unwrap();
            assert_eq!(reads.len(), 2);
            assert_eq!(
                moves,
                [
                    ("incoming".into(), "inbox"),
                    ("outgoing".into(), "sentitems"),
                    ("draft".into(), "drafts")
                ]
            );
            assert_eq!(
                outcome,
                RestoreOutcome {
                    moved: 3,
                    left_in_place: 0,
                    failed: 0,
                }
            );
        }
    }

    #[tokio::test]
    async fn restore_unknown_owner_keeps_ambiguous_mail_and_reports_it() {
        for source in ["archive", "deleteditems"] {
            for address in [None, Some(""), Some("  ")] {
                let mut moves = Vec::new();
                let outcome = restore_conversation(
                    "thread-1",
                    source,
                    address,
                    |_| {
                        std::future::ready(Ok(json!({"value":[
                            restore_fixture("incoming", source, "sender@example.edu", false),
                            restore_fixture("outgoing", source, "owner@example.com", false),
                            restore_fixture("draft", source, "owner@example.com", true)
                        ]})))
                    },
                    |id, folder| {
                        moves.push((id, folder));
                        std::future::ready(Ok(()))
                    },
                )
                .await
                .unwrap();
                assert_eq!(moves, [("draft".into(), "drafts")]);
                assert_eq!(
                    outcome,
                    RestoreOutcome {
                        moved: 1,
                        left_in_place: 2,
                        failed: 0,
                    }
                );
            }
        }
        // No moves is still a reportable result when all destinations are
        // ambiguous; do not claim a successful inbox restore or a 404.
        let outcome = restore_conversation("thread-1", "archive", None, |_| {
            std::future::ready(Ok(json!({"value":[restore_fixture("ambiguous", "archive", "sender@example.edu", false)]})))
        }, |_, _| std::future::ready(Err(MailError::InvalidInput("must not move".into())))).await.unwrap();
        assert_eq!(
            outcome,
            RestoreOutcome {
                moved: 0,
                left_in_place: 1,
                failed: 0,
            }
        );
    }

    #[test]
    fn restore_requires_sender_and_draft_evidence() {
        assert_eq!(
            restore_destination(&json!({"isDraft":false}), Some("owner@example.com")),
            None
        );
        assert_eq!(
            restore_destination(
                &json!({"from":{"emailAddress":{"address":"sender@example.edu"}}}),
                Some("owner@example.com")
            ),
            None
        );
        assert_eq!(
            restore_destination(&json!({"isDraft":true}), None),
            Some("drafts")
        );
    }

    #[tokio::test]
    async fn restore_reads_every_page_before_any_move_and_counts_failed_moves() {
        let mut calls = 0;
        let result = restore_conversation("thread-1", "archive", Some("owner@example.com"), |url| {
            calls += 1;
            std::future::ready(if calls == 1 {
                Ok(json!({"value":[restore_fixture("incoming", "archive", "sender@example.edu", false)], "@odata.nextLink":format!("{url}&$skiptoken=next")}))
            } else { Err(MailError::ApiError { status:503, message:"fixture unavailable".into() }) })
        }, |_, _| {
            panic!("no move before the entire source folder was read");
            #[allow(unreachable_code)]
            std::future::ready(Ok(()))
        }).await;
        assert!(result.is_err());
        let outcome = restore_conversation(
            "thread-1",
            "archive",
            Some("owner@example.com"),
            |_| {
                std::future::ready(Ok(json!({"value":[
                    restore_fixture("moved", "archive", "sender@example.edu", false),
                    restore_fixture("failed", "archive", "owner@example.com", false)
                ]})))
            },
            |id, _| {
                std::future::ready(if id == "failed" {
                    Err(MailError::ApiError {
                        status: 503,
                        message: "fixture unavailable".into(),
                    })
                } else {
                    Ok(())
                })
            },
        )
        .await
        .unwrap();
        assert_eq!(
            outcome,
            RestoreOutcome {
                moved: 1,
                left_in_place: 0,
                failed: 1,
            }
        );
    }
}
