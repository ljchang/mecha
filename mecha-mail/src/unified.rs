//! The unified mail surface: one set of provider-neutral tools over any
//! number of configured accounts. The model never chooses "gmail vs
//! outlook" — it names an *account* (or none), and the server routes.
//!
//! Resolution policy, which is the design:
//! - **Reads fan out.** No `account` on a search or a calendar window means
//!   every account, merged in time order, each row tagged with the account
//!   it came from — "what's on Thursday" is one call, not one per mailbox.
//! - **Item operations name their account.** Thread and event ids are
//!   account-scoped, so `get_thread`, `reply`, `update`, `delete` require
//!   `account` once more than one is configured. Every row a read returns
//!   carries the account, so the model always has it in hand.
//! - **Creates use the default, or ask.** A new mail or event with no
//!   `account` goes to the configured default; with several accounts and no
//!   default the call fails with instructions to *ask the user* — worded
//!   that way deliberately, because "use your best judgment" is how a model
//!   invents an answer.
//!
//! A failed account never sinks a fan-out: its error is reported beside the
//! other accounts' results, and the call is an error only when every account
//! failed.

use serde_json::{json, Value};

use crate::accounts::{self, Provider};
use crate::google::calendar as gcal;
use crate::google::gmail::GmailProvider;
use crate::google::server::markdown_to_html;
use crate::microsoft::graph_calendar as mcal;
use crate::microsoft::graph_mail::OutlookProvider;
use crate::text::clean_body;
use crate::token::{self, TokenManager};
use crate::types::{Email, MailError};

pub struct Account {
    pub name: String,
    pub provider: Provider,
    /// The mailbox's own address, recorded at auth time. Used to keep the
    /// user off their own reply-all recipient lists.
    pub address: Option<String>,
    pub manager: TokenManager,
}

pub struct MailTools {
    accounts: Vec<Account>,
    default: Option<String>,
    /// Built once at startup: the `account` enum is baked into the schemas,
    /// so the model sees the real account names instead of guessing.
    definitions: Vec<Value>,
}

impl MailTools {
    /// Load every configured account. A configured account whose credentials
    /// cannot load fails startup with the fix named — a server that silently
    /// served fewer mailboxes than configured would read as "no mail there".
    pub fn load() -> anyhow::Result<Self> {
        use anyhow::Context;
        let file = accounts::load()?;
        anyhow::ensure!(
            !file.accounts.is_empty(),
            "no accounts configured — run `mecha-mail auth <name> --provider google|outlook`"
        );
        let mut list = Vec::new();
        for entry in &file.accounts {
            let path = accounts::credentials_path(&entry.name)?;
            let creds = token::load(&path).with_context(|| {
                format!(
                    "account `{}`: run `mecha-mail auth {} --provider {}`",
                    entry.name, entry.name, entry.provider
                )
            })?;
            // One read serves both the address and the manager: a re-auth
            // between two reads could pair one login's address with
            // another's tokens, and the address drives reply addressing.
            let address = creds.account.clone();
            let manager = match entry.provider {
                Provider::Google => TokenManager::with_credentials(path, creds),
                Provider::Outlook => TokenManager::with_credentials_microsoft(path, creds)
                    .with_context(|| {
                        format!(
                            "account `{}`: run `mecha-mail auth {} --provider outlook`",
                            entry.name, entry.name
                        )
                    })?,
            };
            list.push(Account {
                name: entry.name.clone(),
                provider: entry.provider,
                address,
                manager,
            });
        }
        let names: Vec<String> = list.iter().map(|a| a.name.clone()).collect();
        Ok(MailTools {
            definitions: tool_definitions(&names, file.default.as_deref()),
            accounts: list,
            default: file.default,
        })
    }
}

// ---------------------------------------------------------------- resolution

/// How a tool relates to accounts, which decides what "no `account`" means.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    /// Fan out to every account.
    Read,
    /// The call carries an account-scoped id: one account, named.
    Item,
    /// Something new is created: the default account, or instructions.
    Create,
}

/// Pure so it is testable without credentials: pick account indexes.
fn resolve(
    names: &[String],
    default: Option<&str>,
    arg: Option<&str>,
    mode: Mode,
) -> Result<Vec<usize>, String> {
    let listed = names.join(", ");
    if let Some(wanted) = arg {
        return match names.iter().position(|n| n == wanted) {
            Some(i) => Ok(vec![i]),
            None => Err(format!(
                "unknown account `{wanted}`; configured accounts: {listed}"
            )),
        };
    }
    match mode {
        Mode::Read => Ok((0..names.len()).collect()),
        _ if names.len() == 1 => Ok(vec![0]),
        Mode::Item => Err(format!(
            "several accounts are configured ({listed}) and this id is account-scoped — \
             pass `account` (every search and list row carries it)"
        )),
        Mode::Create => match default {
            Some(d) => match names.iter().position(|n| n == d) {
                Some(i) => Ok(vec![i]),
                None => Err(format!(
                    "default account `{d}` is not configured ({listed})"
                )),
            },
            None => Err(format!(
                "several accounts are configured ({listed}) and no default is set — \
                 ask the user which account to use, then pass it as `account`. \
                 (They can set a standing default with `mecha-mail default <name>`.)"
            )),
        },
    }
}

// ---------------------------------------------------------------- tool defs

/// The unified surface. `names` becomes the `account` enum in every schema.
pub fn tool_definitions(names: &[String], default: Option<&str>) -> Vec<Value> {
    let account = |rule: &str| -> Value {
        let default_note = match default {
            Some(d) => format!(" The default account is `{d}`."),
            None => String::new(),
        };
        json!({
            "type": "string",
            "enum": names,
            "description": format!("{rule}{default_note}"),
        })
    };

    json!([
        {
            "name": "mail_search",
            "description": "Search mail. With no `account`, every configured account is searched and each result row is tagged with the account it came from. from:/to:/subject: filters work on all providers; date filters are provider-specific (Gmail: after:YYYY/MM/DD; Outlook: received>=YYYY-MM-DD), so name an account when using one. Returns metadata and snippets; use mail_get_thread to read full messages.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "account": account("Omit to search every account."),
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 50, "default": 10}
                },
                "required": ["query"]
            },
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "mail_recent",
            "description": "The most recent messages, newest first, across every account (or one, when `account` is given). Use when the user asks what just came in rather than for a specific search.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "account": account("Omit to read every account."),
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 50, "default": 10}
                }
            },
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "mail_get_thread",
            "description": "Read a whole conversation by thread_id, oldest first, with clean text bodies. thread_ids are account-scoped: pass the `account` from the search row the id came from.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string"},
                    "account": account("The account the thread_id came from; required when several accounts are configured.")
                },
                "required": ["thread_id"]
            },
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "mail_send",
            "description": "Send a NEW email. body_markdown is converted to HTML. To answer an existing message use mail_reply instead, so it threads. With no `account` the default account is used; if none is set, the call fails and you should ask the user which account to send from.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "to": {"type": "string"},
                    "subject": {"type": "string"},
                    "body_markdown": {"type": "string"},
                    "account": account("The account to send from."),
                    "cc": {"type": "string"},
                    "bcc": {"type": "string"}
                },
                "required": ["to", "subject", "body_markdown"]
            },
            "annotations": {"openWorldHint": true}
        },
        {
            "name": "mail_reply",
            "description": "Reply within an existing conversation so it threads, quoting/threading per provider automatically. Pass the thread_id and its `account` (both are in every search row). Replies to the newest message in the thread unless message_id names one. Set reply_all to include everyone on the original.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string"},
                    "body_markdown": {"type": "string"},
                    "account": account("The account the thread lives in; required when several accounts are configured."),
                    "message_id": {"type": "string", "description": "Reply to this specific message instead of the newest one."},
                    "reply_all": {"type": "boolean", "default": false}
                },
                "required": ["thread_id", "body_markdown"]
            },
            "annotations": {"openWorldHint": true}
        },
        {
            "name": "calendar_list",
            "description": "List the calendars in every configured account (or one, when `account` is given), with write access noted.",
            "inputSchema": {
                "type": "object",
                "properties": {"account": account("Omit to list every account's calendars.")}
            },
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "calendar_list_events",
            "description": "List events in a time window across every account, merged in time order and tagged by account (recurring events arrive expanded). Times are RFC 3339; omit both to get the next 7 days. calendar_id addresses one account's calendar, so it requires `account`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "time_min": {"type": "string"},
                    "time_max": {"type": "string"},
                    "account": account("Omit to read every account's primary calendar."),
                    "calendar_id": {"type": "string", "default": "primary"}
                }
            },
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "calendar_create_event",
            "description": "Create a calendar event; attendees receive invitations. Times are RFC 3339 (or YYYY-MM-DD with all_day). With no `account` the default account's calendar is used; if none is set, the call fails and you should ask the user which calendar.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "start_time": {"type": "string"},
                    "end_time": {"type": "string"},
                    "account": account("The account whose calendar gets the event."),
                    "description": {"type": "string"},
                    "location": {"type": "string"},
                    "attendees": {"type": "array", "items": {"type": "string"}},
                    "all_day": {"type": "boolean", "default": false},
                    "timezone": {"type": "string"},
                    "calendar_id": {"type": "string", "default": "primary"}
                },
                "required": ["title", "start_time", "end_time"]
            },
            "annotations": {"openWorldHint": true}
        },
        {
            "name": "calendar_update_event",
            "description": "Update fields of an existing event by event_id in the `account` it lives in (from the event row). Only the fields provided change; attendees are notified.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "event_id": {"type": "string"},
                    "account": account("The account the event lives in; required when several accounts are configured."),
                    "calendar_id": {"type": "string", "default": "primary"},
                    "title": {"type": "string"},
                    "start_time": {"type": "string"},
                    "end_time": {"type": "string"},
                    "description": {"type": "string"},
                    "location": {"type": "string"},
                    "attendees": {"type": "array", "items": {"type": "string"}},
                    "all_day": {"type": "boolean"},
                    "timezone": {"type": "string"}
                },
                "required": ["event_id"]
            },
            "annotations": {"openWorldHint": true, "destructiveHint": true}
        },
        {
            "name": "calendar_delete_event",
            "description": "Delete an event by event_id in the `account` it lives in. Attendees are notified of the cancellation.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "event_id": {"type": "string"},
                    "account": account("The account the event lives in; required when several accounts are configured."),
                    "calendar_id": {"type": "string", "default": "primary"}
                },
                "required": ["event_id"]
            },
            "annotations": {"openWorldHint": true, "destructiveHint": true}
        }
    ])
    .as_array()
    .cloned()
    .unwrap()
}

// ------------------------------------------------------------ per-account ops

/// Run one provider call with a live token; on a 401, one forced refresh and
/// retry — the same `refreshOrReconnect` shape the per-provider servers use.
async fn with_token<T, F, Fut>(manager: &TokenManager, f: F) -> Result<T, MailError>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<T, MailError>>,
{
    let token = match manager.access_token().await {
        Ok(t) => t,
        Err(e) => return Err(MailError::AuthError(format!("{e:#}"))),
    };
    match f(token).await {
        Err(e) if e.is_auth_expiry() => {
            let token = match manager.force_refresh().await {
                Ok(t) => t,
                Err(e) => return Err(MailError::AuthError(format!("{e:#}"))),
            };
            f(token).await
        }
        other => other,
    }
}

async fn search_one(a: &Account, query: &str, max: u32) -> Result<Vec<Email>, MailError> {
    with_token(&a.manager, |t| async move {
        match a.provider {
            Provider::Google => GmailProvider::new(t).search(query, max).await,
            Provider::Outlook => OutlookProvider::new(t).search(query, max).await,
        }
    })
    .await
}

async fn recent_one(a: &Account, max: u32) -> Result<Vec<Email>, MailError> {
    with_token(&a.manager, |t| async move {
        match a.provider {
            // Gmail has no "recent" endpoint; the inbox newest-first is the
            // same answer.
            Provider::Google => GmailProvider::new(t).search("in:inbox", max).await,
            Provider::Outlook => OutlookProvider::new(t).recent(max).await,
        }
    })
    .await
}

async fn thread_one(a: &Account, thread_id: &str) -> Result<Vec<Email>, MailError> {
    with_token(&a.manager, |t| async move {
        match a.provider {
            Provider::Google => GmailProvider::new(t).get_thread(thread_id).await,
            Provider::Outlook => OutlookProvider::new(t).get_thread(thread_id).await,
        }
    })
    .await
}

async fn calendars_one(a: &Account) -> Result<Value, MailError> {
    with_token(&a.manager, |t| async move {
        match a.provider {
            Provider::Google => gcal::CalendarProvider::new(t)
                .list_calendars()
                .await
                .map(|c| serde_json::to_value(c).unwrap_or_else(|_| json!([]))),
            Provider::Outlook => mcal::OutlookCalendarProvider::new(t)
                .list_calendars()
                .await
                .map(|c| serde_json::to_value(c).unwrap_or_else(|_| json!([]))),
        }
    })
    .await
}

/// Events as raw JSON values, times still in the provider's UTC — sorting
/// happens on the raw stamps, rendering into the user's zone afterwards.
async fn events_one(
    a: &Account,
    calendar_id: &str,
    time_min: &str,
    time_max: &str,
) -> Result<Vec<Value>, MailError> {
    let to_values = |v: Value| -> Vec<Value> { v.as_array().cloned().unwrap_or_default() };
    with_token(&a.manager, |t| async move {
        match a.provider {
            Provider::Google => gcal::CalendarProvider::new(t)
                .list_events(calendar_id, time_min, time_max)
                .await
                .map(|e| to_values(serde_json::to_value(e).unwrap_or_else(|_| json!([])))),
            Provider::Outlook => mcal::OutlookCalendarProvider::new(t)
                .list_events(calendar_id, time_min, time_max)
                .await
                .map(|e| to_values(serde_json::to_value(e).unwrap_or_else(|_| json!([])))),
        }
    })
    .await
}

// -------------------------------------------------------------------- merges

fn date_key(raw: &str) -> i64 {
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(raw.trim()) {
        return d.timestamp();
    }
    // An all-day event's bare date is a day, not an instant; midnight UTC is
    // a stable sort position for it.
    if let Ok(d) = chrono::NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d") {
        if let Some(dt) = d.and_hms_opt(0, 0, 0) {
            return dt.and_utc().timestamp();
        }
    }
    0
}

/// Sort merged events on the raw provider stamps, then render into the
/// user's zone — rendered strings only sort within one zone. All-day events
/// render as bare dates on **both** providers: Google already sends
/// `YYYY-MM-DD`, but Graph sends an all-day event as midnight UTC
/// (`2026-08-10T00:00:00.0000000Z`), which zone conversion would move to the
/// previous evening — a retreat starting Monday announced as Sunday 8pm.
fn finish_events(events: &mut [Value], tz: Option<chrono_tz::Tz>) {
    events.sort_by_key(|e| date_key(e["start_time"].as_str().unwrap_or_default()));
    for e in events {
        let all_day = e["is_all_day"].as_bool().unwrap_or(false);
        for key in ["start_time", "end_time"] {
            if let Some(raw) = e[key].as_str() {
                e[key] = if all_day {
                    // The date as the provider stated it, never zone-shifted:
                    // a day is not an instant.
                    json!(raw.get(..10).unwrap_or(raw))
                } else {
                    json!(crate::time::in_zone(raw, tz))
                };
            }
        }
    }
}

/// The reply-capable id for a message, per provider: Graph replies by
/// message id; Gmail threads by the RFC 5322 Message-ID.
fn reply_id(provider: Provider, e: &Email) -> String {
    match provider {
        Provider::Google => e
            .message_id
            .clone()
            .unwrap_or_else(|| e.provider_id.clone()),
        Provider::Outlook => e.provider_id.clone(),
    }
}

fn render_rows(mut rows: Vec<(Provider, String, Email)>) -> String {
    if rows.is_empty() {
        return "no matching messages".into();
    }
    rows.sort_by_key(|(_, _, e)| std::cmp::Reverse(date_key(&e.date_received)));
    let rows: Vec<Value> = rows
        .into_iter()
        .map(|(provider, account, e)| {
            json!({
                "account": account,
                "thread_id": e.thread_id,
                "message_id": reply_id(provider, &e),
                "from": format!("{} <{}>", e.from_name, e.from_address),
                "subject": e.subject,
                "date": e.date_received,
                "snippet": e.snippet,
                "unread": !e.is_read,
                "has_attachments": e.has_attachments,
            })
        })
        .collect();
    serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".into())
}

fn render_thread(provider: Provider, account: &str, emails: &[Email]) -> String {
    emails
        .iter()
        .map(|e| {
            format!(
                "--- [{}] From: {} <{}> · {}\nSubject: {}\nMessage id (for mail_reply): {}\n\n{}",
                account,
                e.from_name,
                e.from_address,
                e.date_received,
                e.subject,
                reply_id(provider, e),
                clean_body(e)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Merge fan-out results: successes render, failures are named beside them,
/// and the call is an error only when *every* account failed.
fn merge<T>(
    results: Vec<(String, Provider, Result<T, MailError>)>,
) -> (Vec<(String, Provider, T)>, Vec<String>, bool) {
    let mut ok = Vec::new();
    let mut failed = Vec::new();
    for (name, provider, r) in results {
        match r {
            Ok(v) => ok.push((name, provider, v)),
            Err(e) => failed.push(format!("account `{name}`: {e}")),
        }
    }
    let all_failed = ok.is_empty() && !failed.is_empty();
    (ok, failed, all_failed)
}

fn with_notes(body: String, failures: &[String]) -> String {
    if failures.is_empty() {
        body
    } else {
        format!(
            "{body}\n\nnote — some accounts could not be read:\n{}",
            failures.join("\n")
        )
    }
}

// ------------------------------------------------------------ gmail replies

/// Synthesize the addressing of a Gmail reply from the message being
/// answered. Outlook gets this from Graph's reply endpoint; Gmail threads by
/// headers, so the recipient logic has to live here: answer the sender
/// (or, when the target is the user's own message, its recipients), keep
/// everyone on for reply-all, and never put the user's own address on the
/// list. Returns `(to, cc, subject)`.
pub(crate) fn gmail_reply_fields(
    target: &Email,
    self_address: Option<&str>,
    reply_all: bool,
) -> (String, Option<String>, String) {
    let is_self = |addr: &str| {
        self_address
            .map(|s| s.eq_ignore_ascii_case(addr.trim()))
            .unwrap_or(false)
    };
    let own_message = is_self(&target.from_address);

    let mut to: Vec<String> = Vec::new();
    let push = |list: &mut Vec<String>, addr: &str| {
        let addr = addr.trim();
        if !addr.is_empty() && !is_self(addr) && !list.iter().any(|t| t.eq_ignore_ascii_case(addr))
        {
            list.push(addr.to_string());
        }
    };
    if !own_message {
        push(&mut to, &target.from_address);
    }
    if reply_all || own_message {
        for a in &target.to_addresses {
            push(&mut to, a);
        }
    }
    if to.is_empty() {
        // Replying to a note-to-self: the only recipient there is.
        to.push(target.from_address.clone());
    }

    let cc = if reply_all {
        let mut cc: Vec<String> = Vec::new();
        for a in &target.cc_addresses {
            // Someone listed in both To and Cc of the original (moved to To,
            // never removed from Cc) must not be addressed twice.
            if !to.iter().any(|t| t.eq_ignore_ascii_case(a.trim())) {
                push(&mut cc, a);
            }
        }
        (!cc.is_empty()).then(|| cc.join(", "))
    } else {
        None
    };

    let subject = if target
        .subject
        .trim()
        .to_ascii_lowercase()
        .starts_with("re:")
    {
        target.subject.clone()
    } else {
        format!("Re: {}", target.subject)
    };
    (to.join(", "), cc, subject)
}

// ------------------------------------------------------------------ dispatch

impl MailTools {
    fn pick(&self, arg: Option<&str>, mode: Mode) -> Result<Vec<&Account>, String> {
        let names: Vec<String> = self.accounts.iter().map(|a| a.name.clone()).collect();
        resolve(&names, self.default.as_deref(), arg, mode)
            .map(|idx| idx.into_iter().map(|i| &self.accounts[i]).collect())
    }

    async fn dispatch(&self, name: &str, args: &Value) -> Option<(String, bool)> {
        let str_arg = |key: &str| args.get(key).and_then(Value::as_str).map(|s| s.to_string());
        let account_arg = str_arg("account");
        let fail = |msg: String| Some((msg, true));
        let missing = |what: &str| Some((format!("missing required `{what}`"), true));

        match name {
            "mail_search" => {
                let Some(query) = str_arg("query") else {
                    return missing("query");
                };
                let max = args
                    .get("max_results")
                    .and_then(Value::as_u64)
                    .unwrap_or(10) as u32;
                let max = max.clamp(1, 50);
                let picked = match self.pick(account_arg.as_deref(), Mode::Read) {
                    Ok(p) => p,
                    Err(e) => return fail(e),
                };
                let results = futures::future::join_all(picked.iter().map(|a| async {
                    (a.name.clone(), a.provider, search_one(a, &query, max).await)
                }))
                .await;
                let (ok, failures, all_failed) = merge(results);
                if all_failed {
                    return fail(failures.join("\n"));
                }
                let rows = ok
                    .into_iter()
                    .flat_map(|(n, p, emails)| emails.into_iter().map(move |e| (p, n.clone(), e)))
                    .collect();
                Some((with_notes(render_rows(rows), &failures), false))
            }
            "mail_recent" => {
                let max = args
                    .get("max_results")
                    .and_then(Value::as_u64)
                    .unwrap_or(10) as u32;
                let max = max.clamp(1, 50);
                let picked = match self.pick(account_arg.as_deref(), Mode::Read) {
                    Ok(p) => p,
                    Err(e) => return fail(e),
                };
                let results = futures::future::join_all(
                    picked
                        .iter()
                        .map(|a| async { (a.name.clone(), a.provider, recent_one(a, max).await) }),
                )
                .await;
                let (ok, failures, all_failed) = merge(results);
                if all_failed {
                    return fail(failures.join("\n"));
                }
                let rows = ok
                    .into_iter()
                    .flat_map(|(n, p, emails)| emails.into_iter().map(move |e| (p, n.clone(), e)))
                    .collect();
                Some((with_notes(render_rows(rows), &failures), false))
            }
            "mail_get_thread" => {
                let Some(thread_id) = str_arg("thread_id") else {
                    return missing("thread_id");
                };
                let account = match self.pick(account_arg.as_deref(), Mode::Item) {
                    Ok(p) => p[0],
                    Err(e) => return fail(e),
                };
                match thread_one(account, &thread_id).await {
                    Ok(emails) => Some((
                        render_thread(account.provider, &account.name, &emails),
                        false,
                    )),
                    Err(e) => fail(format!("{e}")),
                }
            }
            "mail_send" => {
                let (Some(to), Some(subject), Some(body_md)) =
                    (str_arg("to"), str_arg("subject"), str_arg("body_markdown"))
                else {
                    return missing("to, subject, and body_markdown");
                };
                let account = match self.pick(account_arg.as_deref(), Mode::Create) {
                    Ok(p) => p[0],
                    Err(e) => return fail(e),
                };
                let html = markdown_to_html(&body_md);
                let cc = str_arg("cc");
                let bcc = str_arg("bcc");
                let sent = with_token(&account.manager, |t| {
                    let (to, subject, html) = (to.clone(), subject.clone(), html.clone());
                    let (cc, bcc) = (cc.clone(), bcc.clone());
                    async move {
                        match account.provider {
                            Provider::Google => GmailProvider::new(t)
                                .send_email(
                                    &to,
                                    &subject,
                                    &html,
                                    None,
                                    cc.as_deref(),
                                    bcc.as_deref(),
                                    None,
                                )
                                .await
                                .map(|id| format!("sent (message id {id})")),
                            Provider::Outlook => OutlookProvider::new(t)
                                .send_email(&to, &subject, &html, cc.as_deref(), bcc.as_deref())
                                .await
                                .map(|()| "sent".to_string()),
                        }
                    }
                })
                .await;
                match sent {
                    Ok(msg) => Some((format!("{msg} from `{}` to {to}", account.name), false)),
                    Err(e) => fail(format!("{e}")),
                }
            }
            "mail_reply" => {
                let (Some(thread_id), Some(body_md)) =
                    (str_arg("thread_id"), str_arg("body_markdown"))
                else {
                    return missing("thread_id and body_markdown");
                };
                let account = match self.pick(account_arg.as_deref(), Mode::Item) {
                    Ok(p) => p[0],
                    Err(e) => return fail(e),
                };
                let reply_all = args
                    .get("reply_all")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let wanted = str_arg("message_id");

                // The thread read finds the target message and, for Gmail,
                // the addressing to synthesize the reply from.
                let emails = match thread_one(account, &thread_id).await {
                    Ok(e) => e,
                    Err(e) => return fail(format!("{e}")),
                };
                let target = match &wanted {
                    Some(mid) => emails
                        .iter()
                        .find(|e| e.provider_id == *mid || e.message_id.as_deref() == Some(mid)),
                    None => emails.last(),
                };
                let Some(target) = target else {
                    return fail(match wanted {
                        Some(mid) => format!("no message `{mid}` in thread {thread_id}"),
                        None => format!("thread {thread_id} has no messages"),
                    });
                };

                let html = markdown_to_html(&body_md);
                let result = match account.provider {
                    Provider::Outlook => {
                        let target_id = target.provider_id.clone();
                        with_token(&account.manager, |t| {
                            let (target_id, html) = (target_id.clone(), html.clone());
                            async move {
                                OutlookProvider::new(t)
                                    .reply(&target_id, &html, reply_all)
                                    .await
                            }
                        })
                        .await
                        .map(|()| "replied in the original conversation".to_string())
                    }
                    Provider::Google => {
                        let (to, cc, subject) =
                            gmail_reply_fields(target, account.address.as_deref(), reply_all);
                        let in_reply_to = target.message_id.clone();
                        with_token(&account.manager, |t| {
                            let (to, cc, subject, html) =
                                (to.clone(), cc.clone(), subject.clone(), html.clone());
                            let (thread_id, in_reply_to) = (thread_id.clone(), in_reply_to.clone());
                            async move {
                                GmailProvider::new(t)
                                    .send_email(
                                        &to,
                                        &subject,
                                        &html,
                                        Some(&thread_id),
                                        cc.as_deref(),
                                        None,
                                        in_reply_to.as_deref(),
                                    )
                                    .await
                            }
                        })
                        .await
                        .map(|id| format!("replied in the original conversation (message id {id})"))
                    }
                };
                match result {
                    Ok(msg) => Some((format!("{msg} from `{}`", account.name), false)),
                    Err(e) => fail(format!("{e}")),
                }
            }
            "calendar_list" => {
                let picked = match self.pick(account_arg.as_deref(), Mode::Read) {
                    Ok(p) => p,
                    Err(e) => return fail(e),
                };
                let results = futures::future::join_all(
                    picked
                        .iter()
                        .map(|a| async { (a.name.clone(), a.provider, calendars_one(a).await) }),
                )
                .await;
                let (ok, failures, all_failed) = merge(results);
                if all_failed {
                    return fail(failures.join("\n"));
                }
                let listed: Vec<Value> = ok
                    .into_iter()
                    .map(|(name, _, cals)| json!({"account": name, "calendars": cals}))
                    .collect();
                let body = serde_json::to_string_pretty(&listed).unwrap_or_else(|_| "[]".into());
                Some((with_notes(body, &failures), false))
            }
            "calendar_list_events" => {
                let now = chrono::Utc::now();
                let time_min = str_arg("time_min").unwrap_or_else(|| now.to_rfc3339());
                let time_max = str_arg("time_max")
                    .unwrap_or_else(|| (now + chrono::Duration::days(7)).to_rfc3339());
                let calendar_id = str_arg("calendar_id");
                // A named calendar is account-scoped, so it needs its account.
                let mode = match calendar_id.as_deref() {
                    Some(id) if id != "primary" => Mode::Item,
                    _ => Mode::Read,
                };
                let calendar_id = calendar_id.unwrap_or_else(|| "primary".into());
                let picked = match self.pick(account_arg.as_deref(), mode) {
                    Ok(p) => p,
                    Err(e) => return fail(e),
                };
                let results = futures::future::join_all(picked.iter().map(|a| async {
                    (
                        a.name.clone(),
                        a.provider,
                        events_one(a, &calendar_id, &time_min, &time_max).await,
                    )
                }))
                .await;
                let (ok, failures, all_failed) = merge(results);
                if all_failed {
                    return fail(failures.join("\n"));
                }
                let mut events: Vec<Value> = ok
                    .into_iter()
                    .flat_map(|(name, _, events)| {
                        events.into_iter().map(move |mut e| {
                            e["account"] = json!(name.clone());
                            e
                        })
                    })
                    .collect();
                if events.is_empty() {
                    let body = format!("no events between {time_min} and {time_max}");
                    return Some((with_notes(body, &failures), false));
                }
                finish_events(&mut events, crate::time::configured_zone());
                let body = serde_json::to_string_pretty(&events).unwrap_or_else(|_| "[]".into());
                Some((with_notes(body, &failures), false))
            }
            "calendar_create_event" => {
                let (Some(title), Some(start), Some(end)) =
                    (str_arg("title"), str_arg("start_time"), str_arg("end_time"))
                else {
                    return missing("title, start_time, and end_time");
                };
                let account = match self.pick(account_arg.as_deref(), Mode::Create) {
                    Ok(p) => p[0],
                    Err(e) => return fail(e),
                };
                let calendar_id = str_arg("calendar_id").unwrap_or_else(|| "primary".into());
                let description = str_arg("description");
                let location = str_arg("location");
                let attendees = str_list(args, "attendees");
                let all_day = args
                    .get("all_day")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let timezone = str_arg("timezone");
                let result = with_token(&account.manager, |t| {
                    let (title, start, end) = (title.clone(), start.clone(), end.clone());
                    let (description, location, timezone) =
                        (description.clone(), location.clone(), timezone.clone());
                    let (attendees, calendar_id) = (attendees.clone(), calendar_id.clone());
                    async move {
                        match account.provider {
                            Provider::Google => {
                                let request = gcal::CreateEventRequest {
                                    title,
                                    description,
                                    start_time: start,
                                    end_time: end,
                                    location,
                                    attendees,
                                    all_day,
                                    timezone,
                                };
                                gcal::CalendarProvider::new(t)
                                    .create_event(&calendar_id, &request)
                                    .await
                                    .map(|e| serde_json::to_string_pretty(&e).unwrap_or_default())
                            }
                            Provider::Outlook => {
                                let request = mcal::CreateEventRequest {
                                    title,
                                    description,
                                    start_time: start,
                                    end_time: end,
                                    location,
                                    attendees,
                                    all_day,
                                    timezone,
                                };
                                mcal::OutlookCalendarProvider::new(t)
                                    .create_event(&calendar_id, &request)
                                    .await
                                    .map(|e| serde_json::to_string_pretty(&e).unwrap_or_default())
                            }
                        }
                    }
                })
                .await;
                match result {
                    Ok(body) => Some((format!("created in `{}`:\n{body}", account.name), false)),
                    Err(e) => fail(format!("{e}")),
                }
            }
            "calendar_update_event" => {
                let Some(event_id) = str_arg("event_id") else {
                    return missing("event_id");
                };
                let account = match self.pick(account_arg.as_deref(), Mode::Item) {
                    Ok(p) => p[0],
                    Err(e) => return fail(e),
                };
                let calendar_id = str_arg("calendar_id").unwrap_or_else(|| "primary".into());
                let attendees = args
                    .get("attendees")
                    .and_then(Value::as_array)
                    .map(|_| str_list(args, "attendees"));
                let fields = (
                    str_arg("title"),
                    str_arg("description"),
                    str_arg("start_time"),
                    str_arg("end_time"),
                    str_arg("location"),
                    args.get("all_day").and_then(Value::as_bool),
                    str_arg("timezone"),
                );
                let result = with_token(&account.manager, |t| {
                    let (title, description, start_time, end_time, location, all_day, timezone) =
                        fields.clone();
                    let (event_id, calendar_id, attendees) =
                        (event_id.clone(), calendar_id.clone(), attendees.clone());
                    async move {
                        match account.provider {
                            Provider::Google => {
                                let request = gcal::UpdateEventRequest {
                                    title,
                                    description,
                                    start_time,
                                    end_time,
                                    location,
                                    attendees,
                                    all_day,
                                    timezone,
                                };
                                gcal::CalendarProvider::new(t)
                                    .update_event(&calendar_id, &event_id, &request)
                                    .await
                                    .map(|e| serde_json::to_string_pretty(&e).unwrap_or_default())
                            }
                            Provider::Outlook => {
                                let request = mcal::UpdateEventRequest {
                                    title,
                                    description,
                                    start_time,
                                    end_time,
                                    location,
                                    attendees,
                                    all_day,
                                    timezone,
                                };
                                mcal::OutlookCalendarProvider::new(t)
                                    .update_event(&event_id, &request)
                                    .await
                                    .map(|e| serde_json::to_string_pretty(&e).unwrap_or_default())
                            }
                        }
                    }
                })
                .await;
                match result {
                    Ok(body) => Some((format!("updated in `{}`:\n{body}", account.name), false)),
                    Err(e) => fail(format!("{e}")),
                }
            }
            "calendar_delete_event" => {
                let Some(event_id) = str_arg("event_id") else {
                    return missing("event_id");
                };
                let account = match self.pick(account_arg.as_deref(), Mode::Item) {
                    Ok(p) => p[0],
                    Err(e) => return fail(e),
                };
                let calendar_id = str_arg("calendar_id").unwrap_or_else(|| "primary".into());
                let result = with_token(&account.manager, |t| {
                    let (event_id, calendar_id) = (event_id.clone(), calendar_id.clone());
                    async move {
                        match account.provider {
                            Provider::Google => {
                                gcal::CalendarProvider::new(t)
                                    .delete_event(&calendar_id, &event_id)
                                    .await
                            }
                            Provider::Outlook => {
                                mcal::OutlookCalendarProvider::new(t)
                                    .delete_event(&event_id)
                                    .await
                            }
                        }
                    }
                })
                .await;
                match result {
                    Ok(()) => Some((
                        format!("deleted event {event_id} from `{}`", account.name),
                        false,
                    )),
                    Err(e) => fail(format!("{e}")),
                }
            }
            _ => None,
        }
    }
}

fn str_list(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[async_trait::async_trait]
impl crate::mcp::ToolProvider for MailTools {
    fn server_name(&self) -> &'static str {
        "mecha-mail"
    }

    fn tools(&self) -> Vec<Value> {
        self.definitions.clone()
    }

    async fn call(&self, name: &str, args: &Value) -> Option<(String, bool)> {
        self.dispatch(name, args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_tool_surface_is_labelled_correctly() {
        crate::mcp::assert_tool_surface(
            &tool_definitions(&names(&["dartmouth", "personal"]), Some("dartmouth")),
            &[
                "mail_search",
                "mail_recent",
                "mail_get_thread",
                "calendar_list",
                "calendar_list_events",
            ],
            &[
                "mail_send",
                "mail_reply",
                "calendar_create_event",
                "calendar_update_event",
                "calendar_delete_event",
            ],
        );
    }

    /// The account enum is the point of building schemas at startup: the
    /// model picks from the real names instead of guessing.
    #[test]
    fn every_tool_offers_the_real_account_names() {
        let defs = tool_definitions(&names(&["dartmouth", "personal"]), None);
        for tool in &defs {
            let name = tool["name"].as_str().unwrap();
            let enum_values = &tool["inputSchema"]["properties"]["account"]["enum"];
            assert_eq!(
                enum_values,
                &json!(["dartmouth", "personal"]),
                "{name} must enumerate the accounts"
            );
        }
    }

    #[test]
    fn the_default_account_is_named_in_the_schema() {
        let defs = tool_definitions(&names(&["a", "b"]), Some("a"));
        let send = defs.iter().find(|t| t["name"] == "mail_send").unwrap();
        let desc = send["inputSchema"]["properties"]["account"]["description"]
            .as_str()
            .unwrap();
        assert!(desc.contains("`a`"), "{desc}");
    }

    // ---- resolution ----

    #[test]
    fn reads_fan_out_and_named_accounts_resolve() {
        let n = names(&["a", "b"]);
        assert_eq!(resolve(&n, None, None, Mode::Read).unwrap(), vec![0, 1]);
        assert_eq!(resolve(&n, None, Some("b"), Mode::Read).unwrap(), vec![1]);
        assert_eq!(resolve(&n, None, Some("b"), Mode::Item).unwrap(), vec![1]);
    }

    #[test]
    fn a_single_account_never_needs_naming() {
        let n = names(&["only"]);
        for mode in [Mode::Read, Mode::Item, Mode::Create] {
            assert_eq!(resolve(&n, None, None, mode).unwrap(), vec![0]);
        }
    }

    #[test]
    fn an_unknown_account_error_lists_the_real_ones() {
        let err = resolve(&names(&["a", "b"]), None, Some("work"), Mode::Read).unwrap_err();
        assert!(err.contains("work") && err.contains("a, b"), "{err}");
    }

    #[test]
    fn item_ops_with_several_accounts_demand_the_account() {
        let err = resolve(&names(&["a", "b"]), None, None, Mode::Item).unwrap_err();
        assert!(err.contains("pass `account`"), "{err}");
    }

    #[test]
    fn creates_use_the_default_and_otherwise_say_to_ask_the_user() {
        let n = names(&["a", "b"]);
        assert_eq!(resolve(&n, Some("b"), None, Mode::Create).unwrap(), vec![1]);
        let err = resolve(&n, None, None, Mode::Create).unwrap_err();
        // The wording is deliberate: "ask the user", never "use your best
        // judgment" — the measured failure of the latter is the model
        // inventing an answer.
        assert!(err.contains("ask the user"), "{err}");
    }

    // ---- gmail reply synthesis ----

    fn email(from: &str, to: &[&str], cc: &[&str], subject: &str) -> Email {
        Email {
            id: "gmail-1".into(),
            provider: "gmail".into(),
            provider_id: "1".into(),
            thread_id: Some("t1".into()),
            message_id: Some("<m1@x>".into()),
            subject: subject.into(),
            from_address: from.into(),
            from_name: "Someone".into(),
            to_addresses: to.iter().map(|s| s.to_string()).collect(),
            cc_addresses: cc.iter().map(|s| s.to_string()).collect(),
            bcc_addresses: vec![],
            date_received: "2026-08-05T12:00:00Z".into(),
            body_text: "hi".into(),
            body_html: String::new(),
            snippet: "hi".into(),
            labels: vec![],
            is_read: true,
            is_starred: false,
            has_attachments: false,
            list_unsubscribe: None,
        }
    }

    #[test]
    fn a_plain_reply_answers_the_sender_only() {
        let e = email(
            "priya@x.edu",
            &["me@dartmouth.edu", "bob@y.com"],
            &[],
            "Plans",
        );
        let (to, cc, subject) = gmail_reply_fields(&e, Some("me@dartmouth.edu"), false);
        assert_eq!(to, "priya@x.edu");
        assert_eq!(cc, None);
        assert_eq!(subject, "Re: Plans");
    }

    #[test]
    fn reply_all_keeps_everyone_except_the_user() {
        let e = email(
            "priya@x.edu",
            &["me@dartmouth.edu", "bob@y.com"],
            &["carol@z.org", "ME@dartmouth.edu"],
            "Re: Plans",
        );
        let (to, cc, subject) = gmail_reply_fields(&e, Some("me@dartmouth.edu"), true);
        assert_eq!(to, "priya@x.edu, bob@y.com");
        assert_eq!(cc.as_deref(), Some("carol@z.org"));
        // Already "Re:" — not "Re: Re:".
        assert_eq!(subject, "Re: Plans");
    }

    /// Someone in both To and Cc of the original (moved to To, never removed
    /// from Cc) is addressed once, not twice.
    #[test]
    fn reply_all_never_addresses_anyone_in_both_to_and_cc() {
        let e = email(
            "priya@x.edu",
            &["me@dartmouth.edu", "bob@y.com"],
            &["Bob@y.com", "carol@z.org"],
            "Plans",
        );
        let (to, cc, _) = gmail_reply_fields(&e, Some("me@dartmouth.edu"), true);
        assert_eq!(to, "priya@x.edu, bob@y.com");
        assert_eq!(cc.as_deref(), Some("carol@z.org"));
    }

    /// Replying within a thread whose newest message is the user's own —
    /// the reply goes back to the people they wrote to, not to themselves.
    #[test]
    fn replying_to_your_own_message_addresses_its_recipients() {
        let e = email(
            "me@dartmouth.edu",
            &["priya@x.edu", "bob@y.com"],
            &[],
            "Plans",
        );
        let (to, _, _) = gmail_reply_fields(&e, Some("me@dartmouth.edu"), false);
        assert_eq!(to, "priya@x.edu, bob@y.com");
    }

    #[test]
    fn a_note_to_self_still_has_a_recipient() {
        let e = email("me@dartmouth.edu", &["me@dartmouth.edu"], &[], "todo");
        let (to, _, _) = gmail_reply_fields(&e, Some("me@dartmouth.edu"), false);
        assert_eq!(to, "me@dartmouth.edu");
    }

    // ---- merge order ----

    #[test]
    fn events_sort_on_raw_stamps_including_all_day_dates() {
        // An all-day date, a UTC stamp, and an offset stamp interleave
        // correctly only if sorting happens before zone rendering.
        assert!(date_key("2026-08-10") < date_key("2026-08-10T09:00:00Z"));
        assert!(date_key("2026-08-10T09:00:00Z") < date_key("2026-08-10T06:00:00-05:00"));
        assert!(date_key("not a date") == 0);
    }

    /// The bug this guards: Graph states an all-day event as midnight UTC,
    /// and zone-rendering that instant moved the day — a Monday retreat
    /// announced as Sunday 8pm. All-day events stay bare dates; timed
    /// events still render in the configured zone; order is by raw stamp.
    #[test]
    fn all_day_events_keep_their_day_while_timed_events_render_in_zone() {
        let tz: Option<chrono_tz::Tz> = Some("America/New_York".parse().unwrap());
        let mut events = vec![
            json!({"start_time": "2026-08-10T16:00:00Z", "end_time": "2026-08-10T17:00:00Z",
                   "is_all_day": false}),
            // Graph's all-day shape: midnight UTC with the 7-digit fraction.
            json!({"start_time": "2026-08-10T00:00:00.0000000Z",
                   "end_time": "2026-08-11T00:00:00.0000000Z", "is_all_day": true}),
            // Google's all-day shape passes through untouched either way.
            json!({"start_time": "2026-08-09", "end_time": "2026-08-10", "is_all_day": true}),
        ];
        finish_events(&mut events, tz);
        assert_eq!(events[0]["start_time"], "2026-08-09");
        assert_eq!(events[1]["start_time"], "2026-08-10");
        assert_eq!(events[1]["end_time"], "2026-08-11");
        assert_eq!(events[2]["start_time"], "2026-08-10 12:00 EDT");
    }

    #[test]
    fn merged_mail_renders_newest_first_with_account_tags() {
        let mut older = email("a@x.com", &[], &[], "old");
        older.date_received = "2026-08-01T00:00:00Z".into();
        let mut newer = email("b@y.com", &[], &[], "new");
        newer.date_received = "2026-08-04T00:00:00Z".into();
        let out = render_rows(vec![
            (Provider::Google, "personal".into(), older),
            (Provider::Outlook, "dartmouth".into(), newer),
        ]);
        let rows: Vec<Value> = serde_json::from_str(&out).unwrap();
        assert_eq!(rows[0]["subject"], "new");
        assert_eq!(rows[0]["account"], "dartmouth");
        assert_eq!(rows[1]["account"], "personal");
    }

    #[test]
    fn a_failed_account_is_noted_beside_the_others_results() {
        let (ok, failures, all_failed) = merge(vec![
            ("a".to_string(), Provider::Google, Ok(1)),
            (
                "b".to_string(),
                Provider::Outlook,
                Err(MailError::AuthError("expired".into())),
            ),
        ]);
        assert_eq!(ok.len(), 1);
        assert!(!all_failed);
        assert!(failures[0].contains("`b`"), "{failures:?}");
        let noted = with_notes("results".into(), &failures);
        assert!(noted.contains("could not be read"), "{noted}");
    }

    #[test]
    fn all_accounts_failing_is_an_error() {
        let (_, _, all_failed) = merge::<()>(vec![(
            "a".to_string(),
            Provider::Google,
            Err(MailError::AuthError("expired".into())),
        )]);
        assert!(all_failed);
    }
}
