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
    default_mail: Option<String>,
    default_calendar: Option<String>,
    /// Built once at startup: the `account` enum is baked into the schemas,
    /// so the model sees the real account names instead of guessing.
    definitions: Vec<Value>,
}

impl MailTools {
    /// The configured accounts, for operator commands that work per mailbox
    /// rather than through the tool surface — `mecha-mail corpus` is the one
    /// today. Read-only: nothing outside this module builds an `Account`.
    pub fn accounts(&self) -> &[Account] {
        &self.accounts
    }

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
            definitions: tool_definitions(&names, &file),
            accounts: list,
            default_mail: file.default_mail,
            default_calendar: file.default_calendar,
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
    /// Something new is created: that surface's default account, or
    /// instructions. The surface rides along because the two creates are
    /// separate decisions — a person whose mail goes out from work may keep
    /// their life on a personal calendar, and one `default` covering both
    /// forces a choice that is not one choice.
    Create(Surface),
}

/// Which create is being resolved.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Surface {
    Mail,
    Calendar,
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
        Mode::Create(_) => match default {
            Some(d) => match names.iter().position(|n| n == d) {
                Some(i) => Ok(vec![i]),
                None => Err(format!(
                    "default account `{d}` is not configured ({listed})"
                )),
            },
            None => Err(format!(
                "several accounts are configured ({listed}) and no default is set — \
                 ask the user which account to use, then pass it as `account`. \
                 (They can set a standing default with `mecha-mail default <name>`, \
                 or one for this surface alone with `{verb}`.)",
                verb = match mode {
                    Mode::Create(Surface::Mail) => "mecha-mail default <name> --mail",
                    _ => "mecha-mail default <name> --calendar",
                }
            )),
        },
    }
}

// ---------------------------------------------------------------- tool defs

/// The unified surface. `names` becomes the `account` enum in every schema.
pub fn tool_definitions(names: &[String], file: &crate::accounts::AccountsFile) -> Vec<Value> {
    // The note has to be the default that *this* tool would actually use. A
    // schema saying "the default account is `personal`" on `mail_send` while
    // sends resolve to `dartmouth` is worse than saying nothing: the model
    // omits `account` believing it knows where the message goes.
    //
    // Which is why only a **create** carries one at all. `resolve` consults
    // a default in `Mode::Create` and nowhere else — a read fans out over
    // every account and an item op errors until one is named — so a default
    // note on `mail_search` or `mail_get_thread` describes behaviour that
    // does not exist, and contradicts the sentence beside it ("Omit to
    // search every account").
    //
    // The `default` **key**, not just the prose, and that is the load-bearing
    // half: a caller cannot resolve this — `mecha-core` has no dependency on
    // this crate, by design, so the account map is only ever visible through
    // the schema. The harness materialises a declared default into the call's
    // arguments before staging it, which is how the outbox and the approval
    // card come to show the account a send would leave from
    // (`mecha_core::tool::with_schema_defaults`). Declare it only where
    // omitting the argument really does resolve to it.
    let plain = |rule: &str| -> Value {
        json!({
            "type": "string",
            "enum": names,
            "description": rule,
        })
    };
    let with_default = |rule: &str, default: Option<&str>| -> Value {
        let Some(d) = default else {
            return plain(rule);
        };
        json!({
            "type": "string",
            "enum": names,
            "description": format!("{rule} The default account is `{d}`."),
            "default": d,
        })
    };
    // One account is its own default, configured or not. `resolve` answers
    // `names.len() == 1` *before* it reaches the `Mode::Create` arm, so with a
    // single mailbox the account a create will use is known whether or not
    // anybody ran `mecha-mail default` — and a draft from the only mailbox
    // there is should still say which one it is. The whole point is that a
    // reviewer never has to know how many accounts exist to read a draft.
    let only_account = match names {
        [only] => Some(only.clone()),
        _ => None,
    };
    let mail_default = only_account
        .clone()
        .or_else(|| file.mail_default().map(String::from));
    let calendar_default = only_account.or_else(|| file.calendar_default().map(String::from));
    let account = |rule: &str| plain(rule);
    let mail_account = |rule: &str| with_default(rule, mail_default.as_deref());
    let calendar_account = |rule: &str| with_default(rule, calendar_default.as_deref());

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
                    "account": mail_account("The account to send from."),
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
            "name": "mail_triage",
            "description": "Clear a conversation out of the inbox: archive it, mark it read or unread, report it as spam, or move it to the trash. Acts on the WHOLE thread. Pass the thread_id and its `account` (both are in every search row). Nothing here leaves the mailbox or reaches anyone else — archive just drops the thread out of the inbox, and trash is recoverable. Use archive for anything dealt with; use spam only for genuine junk, because it also trains the provider's filter. To tag a thread for later, do not use this — tags are mecha's own and are set on the triage record.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string"},
                    "action": {
                        "type": "string",
                        "enum": ["archive", "read", "unread", "spam", "trash"]
                    },
                    "account": account("The account the thread lives in; required when several accounts are configured.")
                },
                "required": ["thread_id", "action"]
            },
            // Neither a read nor a send. It mutates the user's own mailbox and
            // reaches no third party, which is the whole reason it is safe to
            // let a triage loop call it without staging — and the whole reason
            // it must be gated by the approver instead. See the capability
            // note on `assert_tool_surface`.
            "annotations": {"destructiveHint": true}
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
            "name": "calendar_freebusy",
            "description": "Busy intervals merged across every account (or one, when `account` is given) — when the user is busy, with no event details. Times are RFC 3339; the answer is in UTC with a local rendering beside it when a zone is configured. Omit both bounds for the next 7 days. Use this for scheduling questions ('when am I free?'); use calendar_list_events when the events themselves matter.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "time_min": {"type": "string"},
                    "time_max": {"type": "string"},
                    "account": account("Omit to merge every account's busy time.")
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
                    "account": calendar_account("The account whose calendar gets the event."),
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

/// What `mail_triage` may do. A **closed enum**, on the reasoning
/// `docs/SLACK-ACTIONS-DESIGN.md` §1 already set out for executable actions:
/// the set is small, every variant is spelled out, and there is deliberately
/// no escape hatch that takes a provider-native label or folder name. A
/// free-form `mail_label(["SPAM"])` would let the model reach `spam` through
/// the argument of a verb that reads as harmless, and would mean every
/// provider difference leaking into the schema.
///
/// Tagging is **not** here. A mecha tag lives on the triage record, costs no
/// OAuth scope, and works identically on both providers; these five are the
/// operations that must reach the provider or they have not happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriageAction {
    /// Out of the inbox, still in the mailbox. The common case by far.
    Archive,
    Read,
    Unread,
    /// Reports to the provider's filter — the one action with an effect
    /// outside this mailbox, which is why it is not a label argument.
    Spam,
    /// Recoverable. Neither scope this crate holds can delete permanently.
    Trash,
}

impl TriageAction {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "archive" => Self::Archive,
            "read" => Self::Read,
            "unread" => Self::Unread,
            "spam" => Self::Spam,
            "trash" => Self::Trash,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Archive => "archive",
            Self::Read => "read",
            Self::Unread => "unread",
            Self::Spam => "spam",
            Self::Trash => "trash",
        }
    }

    /// Past tense, for the line the model reads back.
    fn done(self) -> &'static str {
        match self {
            Self::Archive => "archived",
            Self::Read => "marked read",
            Self::Unread => "marked unread",
            Self::Spam => "reported as spam and removed from the inbox",
            Self::Trash => "moved to the trash",
        }
    }

    pub const ALL: [Self; 5] = [
        Self::Archive,
        Self::Read,
        Self::Unread,
        Self::Spam,
        Self::Trash,
    ];
}

/// Apply one triage action to one thread in one account.
///
/// Gmail returns nothing on success; Graph returns how many messages of the
/// conversation it actually touched, because it has no thread resource and
/// the operation is not atomic (see `graph_mail.rs`). The count is carried
/// rather than discarded so the caller can say "3 of 5" instead of "done".
async fn triage_one(
    a: &Account,
    thread_id: &str,
    action: TriageAction,
) -> Result<Option<usize>, MailError> {
    with_token(&a.manager, |t| async move {
        match a.provider {
            Provider::Google => {
                let g = GmailProvider::new(t);
                match action {
                    TriageAction::Archive => g.archive_thread(thread_id).await,
                    TriageAction::Read => g.set_thread_read(thread_id, true).await,
                    TriageAction::Unread => g.set_thread_read(thread_id, false).await,
                    TriageAction::Spam => g.spam_thread(thread_id).await,
                    TriageAction::Trash => g.trash_thread(thread_id).await,
                }
                .map(|()| None)
            }
            Provider::Outlook => {
                let o = OutlookProvider::new(t);
                match action {
                    TriageAction::Archive => o.archive_thread(thread_id).await,
                    TriageAction::Read => o.set_thread_read(thread_id, true).await,
                    TriageAction::Unread => o.set_thread_read(thread_id, false).await,
                    TriageAction::Spam => o.spam_thread(thread_id).await,
                    TriageAction::Trash => o.trash_thread(thread_id).await,
                }
                .map(Some)
            }
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

/// One account's busy intervals, parsed. A stamp pair that does not parse is
/// an error for the whole account, never a skipped interval — a dropped busy
/// interval reads as free time, and the consumer of this call offers free
/// time to strangers.
async fn freebusy_one(
    a: &Account,
    time_min: &str,
    time_max: &str,
) -> Result<Vec<crate::freebusy::Interval>, MailError> {
    let pairs = with_token(&a.manager, |t| async move {
        match a.provider {
            Provider::Google => {
                gcal::CalendarProvider::new(t)
                    .freebusy(time_min, time_max)
                    .await
            }
            Provider::Outlook => {
                let Some(address) = a.address.as_deref() else {
                    return Err(MailError::InvalidInput(format!(
                        "account `{}` has no stored mailbox address, which Graph's \
                         getSchedule needs — re-run `mecha-mail auth {} --provider outlook`",
                        a.name, a.name
                    )));
                };
                mcal::OutlookCalendarProvider::new(t)
                    .freebusy(address, time_min, time_max)
                    .await
            }
        }
    })
    .await?;

    pairs
        .into_iter()
        .map(|(start, end)| {
            let parse = |raw: &str| {
                crate::freebusy::parse_stamp(raw)
                    .ok_or_else(|| MailError::ParseError(format!("unparseable busy stamp `{raw}`")))
            };
            Ok(crate::freebusy::Interval {
                start: parse(&start)?,
                end: parse(&end)?,
            })
        })
        .collect()
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
                // The deterministic bulk signal, surfaced so the triage
                // pre-filter can dispose of a thread without a model call.
                // A property of the message rather than a judgement about it,
                // which is why it rides on the row rather than being inferred
                // downstream from the sender.
                "bulk": e.list_unsubscribe.is_some(),
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
    /// Merged busy intervals across accounts (or one named account), with
    /// per-account failures reported beside the result. `Err` only when the
    /// account cannot resolve or *every* account failed — the CLI decides
    /// how strict to be about partial failure, because its consumers differ:
    /// a model can reason over "one mailbox was unreadable", a slot pipeline
    /// must refuse to treat it as free time.
    pub async fn freebusy(
        &self,
        time_min: &str,
        time_max: &str,
        account: Option<&str>,
    ) -> Result<(Vec<crate::freebusy::Interval>, Vec<String>), String> {
        let picked = self.pick(account, Mode::Read)?;
        let results = futures::future::join_all(picked.iter().map(|a| async {
            (
                a.name.clone(),
                a.provider,
                freebusy_one(a, time_min, time_max).await,
            )
        }))
        .await;
        let (ok, failures, all_failed) = merge(results);
        if all_failed {
            return Err(failures.join("\n"));
        }
        let intervals = crate::freebusy::merge(ok.into_iter().flat_map(|(_, _, iv)| iv).collect());
        Ok((intervals, failures))
    }

    /// The account a create with this argument lands on, by name — the same
    /// `Mode::Create` resolution `create_event_invite` applies (the named
    /// account, else the default, else instructions), exposed so the booking
    /// sweep can resolve it once before creating anything. Deliberately NOT
    /// a scope for the sweep's freebusy re-verify: that read fans out over
    /// every account, because a slot free on the landing calendar but busy
    /// on another is a collision — a revoked token on one of the others is
    /// classified and skipped there, never used to narrow the read.
    pub fn create_account_name(&self, account: Option<&str>) -> Result<String, String> {
        self.pick(account, Mode::Create(Surface::Calendar))
            .map(|p| p[0].name.clone())
    }

    /// Create one event with typed arguments — the bookings handler's
    /// path, beside the tool's. Same account resolution (the default, or
    /// instructions to ask), same token machinery. With an attendee, the
    /// **provider sends its native invite** (Graph does unconditionally;
    /// Google via `sendUpdates=all`) — which is the design: an invite from
    /// the user's own mailbox is the most deliverable calendar mail that
    /// exists, its Accept/Decline RSVPs back to the real event, and
    /// cancellation later is a native retraction. Returns
    /// `(account, event_id)`.
    pub async fn create_event_invite(
        &self,
        account: Option<&str>,
        title: &str,
        description: &str,
        start: &str,
        end: &str,
        attendee: Option<&str>,
    ) -> Result<(String, String), String> {
        let account = self.pick(account, Mode::Create(Surface::Calendar))?[0];
        let attendees: Vec<String> = attendee.map(str::to_string).into_iter().collect();
        let result = with_token(&account.manager, |t| {
            let (title, description) = (title.to_string(), description.to_string());
            let (start, end) = (start.to_string(), end.to_string());
            let attendees = attendees.clone();
            async move {
                match account.provider {
                    Provider::Google => {
                        let request = gcal::CreateEventRequest {
                            title,
                            description: Some(description),
                            start_time: start,
                            end_time: end,
                            location: None,
                            attendees,
                            all_day: false,
                            timezone: None,
                        };
                        gcal::CalendarProvider::new(t)
                            .create_event("primary", &request)
                            .await
                            .map(|e| e.event_id)
                    }
                    Provider::Outlook => {
                        let request = mcal::CreateEventRequest {
                            title,
                            description: Some(description),
                            start_time: start,
                            end_time: end,
                            location: None,
                            attendees,
                            all_day: false,
                            timezone: None,
                        };
                        mcal::OutlookCalendarProvider::new(t)
                            .create_event("primary", &request)
                            .await
                            .map(|e| e.event_id)
                    }
                }
            }
        })
        .await;
        result
            .map(|event_id| (account.name.clone(), event_id))
            .map_err(|e| format!("account `{}`: {e}", account.name))
    }

    /// Delete one event — the cancellation half of the bookings handler.
    /// The account is named exactly (it came off the ledger), and both
    /// providers mail the attendees their native retraction.
    pub async fn delete_event_quiet(&self, account: &str, event_id: &str) -> Result<(), String> {
        let account = self.pick(Some(account), Mode::Item)?[0];
        let event_id = event_id.to_string();
        with_token(&account.manager, |t| {
            let event_id = event_id.clone();
            async move {
                match account.provider {
                    Provider::Google => {
                        gcal::CalendarProvider::new(t)
                            .delete_event("primary", &event_id)
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
        .await
        .map_err(|e| format!("account `{}`: {e}", account.name))
    }

    /// Send one plain message from a named account — the reminder path.
    /// Deterministic machinery beside `create_event_invite`, never a model's
    /// composition: the body is templated by the caller from typed values.
    pub async fn send_mail_quiet(
        &self,
        account: &str,
        to: &str,
        subject: &str,
        body_markdown: &str,
    ) -> Result<(), String> {
        let account = self.pick(Some(account), Mode::Item)?[0];
        let html = markdown_to_html(body_markdown);
        with_token(&account.manager, |t| {
            let (to, subject, html) = (to.to_string(), subject.to_string(), html.clone());
            async move {
                match account.provider {
                    Provider::Google => GmailProvider::new(t)
                        .send_email(&to, &subject, &html, None, None, None, None)
                        .await
                        .map(|_| ()),
                    Provider::Outlook => {
                        OutlookProvider::new(t)
                            .send_email(&to, &subject, &html, None, None)
                            .await
                    }
                }
            }
        })
        .await
        .map_err(|e| format!("account `{}`: {e}", account.name))
    }

    fn pick(&self, arg: Option<&str>, mode: Mode) -> Result<Vec<&Account>, String> {
        let names: Vec<String> = self.accounts.iter().map(|a| a.name.clone()).collect();
        // Only a create consults a default at all; the surface picks which
        // one, falling back to the general default when that surface has no
        // opinion. Resolved here rather than inside `resolve` so that
        // function stays a pure question about names.
        let default = match mode {
            Mode::Create(Surface::Mail) => self.default_mail.as_deref().or(self.default.as_deref()),
            Mode::Create(Surface::Calendar) => {
                self.default_calendar.as_deref().or(self.default.as_deref())
            }
            _ => self.default.as_deref(),
        };
        resolve(&names, default, arg, mode)
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
            "mail_triage" => {
                let Some(thread_id) = str_arg("thread_id") else {
                    return missing("thread_id");
                };
                let Some(raw) = str_arg("action") else {
                    return missing("action");
                };
                let Some(action) = TriageAction::parse(&raw) else {
                    // Naming the alternatives rather than saying "invalid":
                    // the model can recover from a list, not from a refusal.
                    let all: Vec<&str> = TriageAction::ALL.iter().map(|a| a.name()).collect();
                    return fail(format!(
                        "unknown action `{raw}`; expected one of: {}",
                        all.join(", ")
                    ));
                };
                // Mode::Item, like every other id-carrying call: a thread_id
                // is account-scoped, so this never fans out. Triaging "the
                // same thread" across every account is not a thing that can
                // be meant.
                let account = match self.pick(account_arg.as_deref(), Mode::Item) {
                    Ok(p) => p[0],
                    Err(e) => return fail(e),
                };
                match triage_one(account, &thread_id, action).await {
                    Ok(None) => Some((
                        format!("{}: thread {thread_id} {}", account.name, action.done()),
                        false,
                    )),
                    // Graph's per-message reality, surfaced rather than
                    // rounded up: a conversation that half-moved says so.
                    Ok(Some(n)) => Some((
                        format!(
                            "{}: thread {thread_id} {} ({n} message{})",
                            account.name,
                            action.done(),
                            if n == 1 { "" } else { "s" }
                        ),
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
                let account = match self.pick(account_arg.as_deref(), Mode::Create(Surface::Mail)) {
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
            "calendar_freebusy" => {
                let now = chrono::Utc::now();
                let time_min = str_arg("time_min").unwrap_or_else(|| now.to_rfc3339());
                let time_max = str_arg("time_max")
                    .unwrap_or_else(|| (now + chrono::Duration::days(7)).to_rfc3339());
                let (busy, failures) = match self
                    .freebusy(&time_min, &time_max, account_arg.as_deref())
                    .await
                {
                    Ok(r) => r,
                    Err(e) => return fail(e),
                };
                let tz = crate::time::configured_zone();
                let rows: Vec<Value> = busy
                    .iter()
                    .map(|iv| {
                        let mut row = serde_json::to_value(iv).unwrap_or_else(|_| json!({}));
                        if tz.is_some() {
                            let render = |t: &chrono::DateTime<chrono::Utc>| {
                                crate::time::in_zone(&t.to_rfc3339(), tz)
                            };
                            row["local"] =
                                json!(format!("{} — {}", render(&iv.start), render(&iv.end)));
                        }
                        row
                    })
                    .collect();
                let body = serde_json::to_string_pretty(&json!({
                    "time_min": time_min,
                    "time_max": time_max,
                    "busy": rows,
                }))
                .unwrap_or_else(|_| "{}".into());
                Some((with_notes(body, &failures), false))
            }
            "calendar_create_event" => {
                let (Some(title), Some(start), Some(end)) =
                    (str_arg("title"), str_arg("start_time"), str_arg("end_time"))
                else {
                    return missing("title, start_time, and end_time");
                };
                let account =
                    match self.pick(account_arg.as_deref(), Mode::Create(Surface::Calendar)) {
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
            &tool_definitions(
                &names(&["dartmouth", "personal"]),
                &conf(Some("dartmouth"), None, None),
            ),
            &[
                "mail_search",
                "mail_recent",
                "mail_get_thread",
                "calendar_list",
                "calendar_list_events",
                "calendar_freebusy",
            ],
            &[
                "mail_send",
                "mail_reply",
                "calendar_create_event",
                "calendar_update_event",
                "calendar_delete_event",
            ],
            &["mail_triage"],
        );
    }

    /// The action set is closed, and the closure is what stops `spam` being
    /// reachable through a label argument on a verb that reads as harmless.
    #[test]
    fn the_triage_action_set_is_closed_and_round_trips() {
        for action in TriageAction::ALL {
            assert_eq!(TriageAction::parse(action.name()), Some(action));
            assert!(!action.done().is_empty());
        }
        for bogus in ["delete", "label", "SPAM", "", "archive "] {
            assert_eq!(TriageAction::parse(bogus), None, "{bogus} must not parse");
        }

        // The schema and the parser must name the same set, or the model is
        // offered a verb that fails or denied one that works.
        let defs = tool_definitions(&names(&["a"]), &conf(Some("a"), None, None));
        let tool = defs.iter().find(|t| t["name"] == "mail_triage").unwrap();
        let schema: Vec<&str> = tool["inputSchema"]["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let coded: Vec<&str> = TriageAction::ALL.iter().map(|a| a.name()).collect();
        assert_eq!(schema, coded);
    }

    /// Tagging must never become a provider operation: it costs an OAuth
    /// scope, diverges between Gmail labels and Graph categories, and is
    /// mecha's own concept on the triage record.
    #[test]
    fn the_mail_surface_offers_no_tagging_verb() {
        let defs = tool_definitions(&names(&["a"]), &conf(Some("a"), None, None));
        for tool in &defs {
            let name = tool["name"].as_str().unwrap();
            assert!(
                !name.contains("label") && !name.contains("tag") && !name.contains("categor"),
                "{name} would make a mecha tag a provider write"
            );
        }
    }

    /// The account enum is the point of building schemas at startup: the
    /// model picks from the real names instead of guessing.
    #[test]
    fn every_tool_offers_the_real_account_names() {
        let defs = tool_definitions(&names(&["dartmouth", "personal"]), &conf(None, None, None));
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
        let defs = tool_definitions(&names(&["a", "b"]), &conf(Some("a"), None, None));
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
        for mode in [Mode::Read, Mode::Item, Mode::Create(Surface::Mail)] {
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
        assert_eq!(
            resolve(&n, Some("b"), None, Mode::Create(Surface::Mail)).unwrap(),
            vec![1]
        );
        let err = resolve(&n, None, None, Mode::Create(Surface::Mail)).unwrap_err();
        // The wording is deliberate: "ask the user", never "use your best
        // judgment" — the measured failure of the latter is the model
        // inventing an answer.
        assert!(err.contains("ask the user"), "{err}");
    }

    // ---- booking re-verification scoping ----

    /// An accounts file carrying just the defaults a schema test needs.
    fn conf(
        default: Option<&str>,
        mail: Option<&str>,
        calendar: Option<&str>,
    ) -> crate::accounts::AccountsFile {
        crate::accounts::AccountsFile {
            default: default.map(String::from),
            default_mail: mail.map(String::from),
            default_calendar: calendar.map(String::from),
            accounts: Vec::new(),
        }
    }

    fn tools_over(names: &[&str], default: Option<&str>) -> MailTools {
        let accounts = names
            .iter()
            .map(|n| Account {
                name: n.to_string(),
                provider: Provider::Google,
                address: None,
                manager: TokenManager::with_credentials(
                    std::path::PathBuf::from("/nonexistent/oauth.json"),
                    crate::token::StoredCredentials {
                        client_id: "id".into(),
                        client_secret: String::new(),
                        tenant: None,
                        access_token: "at".into(),
                        refresh_token: "rt".into(),
                        expires_at: 0,
                        account: None,
                        granted_scopes: None,
                        granted_at: None,
                    },
                ),
            })
            .collect();
        MailTools {
            definitions: Vec::new(),
            accounts,
            default: default.map(String::from),
            default_mail: None,
            default_calendar: None,
        }
    }

    #[test]
    fn mail_and_calendar_can_default_to_different_accounts() {
        // The case this exists for, in the owner's words: mail out from the
        // work address, events on the personal calendar. One `default` made
        // that a single choice, so setting either moved both.
        let mut tools = tools_over(&["personal", "dartmouth"], None);
        tools.default_mail = Some("dartmouth".into());
        tools.default_calendar = Some("personal".into());

        let send = tools.pick(None, Mode::Create(Surface::Mail)).unwrap();
        assert_eq!(send[0].name, "dartmouth");
        let event = tools.pick(None, Mode::Create(Surface::Calendar)).unwrap();
        assert_eq!(event[0].name, "personal");

        // An explicit `account` still wins over both, on either surface.
        let named = tools
            .pick(Some("personal"), Mode::Create(Surface::Mail))
            .unwrap();
        assert_eq!(named[0].name, "personal");
    }

    #[test]
    fn a_surface_with_no_opinion_falls_back_to_the_general_default() {
        // The upgrade path: a file that predates the split has only
        // `default`, and both creates must keep resolving exactly as they
        // did. Setting one surface must not orphan the other.
        let mut tools = tools_over(&["personal", "dartmouth"], Some("personal"));
        for mode in [Mode::Create(Surface::Mail), Mode::Create(Surface::Calendar)] {
            assert_eq!(tools.pick(None, mode).unwrap()[0].name, "personal");
        }
        tools.default_mail = Some("dartmouth".into());
        assert_eq!(
            tools.pick(None, Mode::Create(Surface::Calendar)).unwrap()[0].name,
            "personal",
            "naming a mail default must not disturb the calendar"
        );
    }

    #[test]
    fn the_schema_tells_each_create_its_own_default() {
        // The model omits `account` when the schema says it knows where the
        // thing goes, so a note naming the wrong surface's default is worse
        // than no note: it is confidently wrong at the moment of sending.
        let defs = tool_definitions(
            &names(&["personal", "dartmouth"]),
            &conf(None, Some("dartmouth"), Some("personal")),
        );
        let note = |tool: &str| -> String {
            defs.iter().find(|d| d["name"] == tool).unwrap()["inputSchema"]["properties"]["account"]
                ["description"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert!(
            note("mail_send").contains("`dartmouth`"),
            "{}",
            note("mail_send")
        );
        assert!(
            note("calendar_create_event").contains("`personal`"),
            "{}",
            note("calendar_create_event")
        );
    }

    /// The default belongs in the schema's `default` key, not only in its
    /// prose — and only on the tools that actually consult one.
    ///
    /// Two failures, one fix. A reviewer approving a staged send could not see
    /// which mailbox it would leave from, because the account was resolved
    /// here long after the draft was written and no caller can look it up
    /// (`mecha-core` does not depend on this crate). And a *read* or an *item*
    /// op carried the note "The default account is `dartmouth`" while
    /// `resolve` consults a default in `Mode::Create` alone — a promise the
    /// code does not keep, next to a sentence saying the opposite.
    #[test]
    fn only_a_create_declares_a_default_and_it_declares_it_machine_readably() {
        let defs = tool_definitions(
            &names(&["personal", "dartmouth"]),
            &conf(None, Some("dartmouth"), Some("personal")),
        );
        let account = |tool: &str| -> Value {
            defs.iter().find(|d| d["name"] == tool).unwrap()["inputSchema"]["properties"]["account"]
                .clone()
        };
        assert_eq!(account("mail_send")["default"], json!("dartmouth"));
        assert_eq!(
            account("calendar_create_event")["default"],
            json!("personal")
        );

        for tool in [
            "mail_search",
            "mail_recent",
            "mail_get_thread",
            "mail_reply",
            "mail_triage",
        ] {
            let spec = account(tool);
            assert!(spec.get("default").is_none(), "{tool}: {spec}");
            assert!(
                !spec["description"].as_str().unwrap().contains("default"),
                "{tool}: {}",
                spec["description"]
            );
        }
    }

    /// The single-account install still says who a draft is from.
    ///
    /// `resolve` answers `names.len() == 1` before it consults a default at
    /// all, so the account is known even with nothing configured — and a
    /// draft that omits it for want of a config line is the same unsigned
    /// letter, on the install least likely to have run `mecha-mail default`.
    #[test]
    fn one_account_is_its_own_default_without_being_configured() {
        let defs = tool_definitions(&names(&["personal"]), &conf(None, None, None));
        for tool in ["mail_send", "calendar_create_event"] {
            let spec = &defs.iter().find(|d| d["name"] == tool).unwrap()["inputSchema"]
                ["properties"]["account"];
            assert_eq!(spec["default"], json!("personal"), "{tool}: {spec}");
        }
        // And it is still only a create that claims one: omitting `account`
        // on a read means every account, which happens to be the same one.
        let search = &defs.iter().find(|d| d["name"] == "mail_search").unwrap()["inputSchema"]
            ["properties"]["account"];
        assert!(search.get("default").is_none(), "{search}");
    }

    /// With no default configured there is nothing to declare, and inventing
    /// one would be the worst outcome of all: the model omits `account`
    /// believing it knows where the message goes, and the send resolves
    /// somewhere else — or errors, having told the reviewer otherwise.
    #[test]
    fn a_create_with_no_default_declares_none() {
        let defs = tool_definitions(&names(&["personal", "dartmouth"]), &conf(None, None, None));
        let send = defs.iter().find(|d| d["name"] == "mail_send").unwrap();
        let spec = &send["inputSchema"]["properties"]["account"];
        assert!(spec.get("default").is_none(), "{spec}");
        assert!(!spec["description"].as_str().unwrap().contains("default"));
    }

    /// The booking sweep resolves where its events land once, up front —
    /// this is that resolution. It must resolve exactly as the create will
    /// (explicit flag, else default), and refuse with instructions when
    /// neither exists. Note what it no longer scopes: the freebusy
    /// re-verify fans out over every account (see the sweep), because
    /// scoping it here silently dropped cross-account collision detection.
    #[test]
    fn the_booking_event_account_resolves_like_the_create() {
        let tools = tools_over(&["dartmouth", "personal"], Some("dartmouth"));
        assert_eq!(tools.create_account_name(None).unwrap(), "dartmouth");
        assert_eq!(
            tools.create_account_name(Some("personal")).unwrap(),
            "personal"
        );

        let err = tools_over(&["a", "b"], None)
            .create_account_name(None)
            .unwrap_err();
        assert!(err.contains("no default"), "{err}");
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
