//! The outbox: staged outbound actions awaiting the user's review.
//!
//! An outbox-routed tool call is never executed by the agent loop — it is
//! written here as a draft, and nothing leaves the machine until the user
//! reads exactly what would be sent and releases it (`mecha outbox send`).
//! That is "draft-only, never send" made structural: the gate lives in core,
//! so an email or calendar tool — including a third-party MCP server's —
//! needs no knowledge of it to be covered by it.
//!
//! The item keeps the drafted arguments (`args_before`) separate from the
//! arguments the release will execute (`args`), because the difference is a
//! measurement: a user edit before sending is a writing correction, and
//! `mecha reflect` mines `diff(args_before, args)` into the learning store.
//!
//! **Staging is sink-agnostic; reviewing is not.** The outbox generalised to a
//! new kind of outbound action — publishing a bundle to the public surface —
//! without a line changing here, which was the design goal. Its *review*
//! affordances did not: `show` printing arguments, `edit` opening them in
//! `$EDITOR`, and the writing miner reading the diff all assume the staged
//! thing is a message someone wrote. That is why an item carries an
//! [`OutboxKind`], set at staging from `[outbox] publish_tools`, and why the
//! miner keys on it — see [`OutboxKind::Publish`].
//!
//! Storage follows the learning store's rules: one pretty-printed JSON file
//! per item so `$EDITOR` and `git diff` work on it, temp-sibling-and-rename
//! for every rewrite so a reader never sees a half-written file, and an
//! advisory flock for writers — taken *before reading the state acted on*,
//! and never held across an editor invocation. Staging takes no lock at all:
//! a fresh item is a fresh file with a unique id, and the agent loop must
//! never block on a human's review session.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::agent::Taint;
use crate::session::Session;

/// What kind of outbound action a staged item is, which decides how it is
/// *reviewed* rather than how it is staged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutboxKind {
    /// Prose somebody wrote and somebody else will read: an email, a calendar
    /// invitation, a message. The reviewable object *is* the arguments, and an
    /// edit before release is a writing correction worth learning from.
    #[default]
    Message,
    /// A publication to the public surface: a rendered bundle, an alias move,
    /// a request-type push. Three things follow, and each is a bug if undone:
    ///
    /// - The reviewable object is the **rendered page**, not the arguments —
    ///   which are a path and a visibility flag.
    /// - `edit` is refused. Editing the content means editing the source and
    ///   re-rendering, which stages a new item; rewriting the path is not
    ///   editing the draft.
    /// - The writing miner **excludes it**. Feeding `diff(args_before, args)`
    ///   of a changed directory path to a reflector that mines voice rules
    ///   would carry noise into every future run's cached prefix. Same mistake
    ///   as learning from `"Blocked by a hook:"`, in a new costume.
    Publish,
}

impl OutboxKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            OutboxKind::Message => "message",
            OutboxKind::Publish => "publish",
        }
    }
}

/// One staged outbound action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboxItem {
    pub id: String,
    /// `pending` | `sent` | `rejected`.
    pub status: String,
    /// The tool a release will execute, by registry name (`web__fetch`).
    pub tool: String,
    /// How this is reviewed. Defaulted rather than required, so items staged
    /// before the field existed load as the kind they in fact were.
    #[serde(default)]
    pub kind: OutboxKind,
    /// The arguments as the agent drafted them. Never modified — this is the
    /// baseline the learning capture diffs against.
    pub args_before: Value,
    /// The arguments a release will execute. Starts equal to `args_before`;
    /// `mecha outbox edit` rewrites it.
    pub args: Value,
    /// One line for `mecha outbox list`.
    pub summary: String,
    /// The session that drafted this, when the front-end knew it.
    #[serde(default)]
    pub session_id: Option<String>,
    /// The path jail the call was drafted under.
    ///
    /// A staged call is a *deferred* tool call, and a tool call only means
    /// anything relative to the workspace it was made in: `bundle` here is a
    /// directory under the drafting run's jail. Release happens in another
    /// process, minutes or hours later, from whatever directory the reviewer
    /// happens to be standing in — so without this the release resolves the
    /// argument against the wrong root. An absolute path fails loudly; a
    /// relative one is worse, because a same-named directory beside the
    /// reviewer would quietly publish the wrong bytes.
    ///
    /// Recording it also keeps the release inside the jail the *agent* was
    /// held to, rather than the reviewer's, which is the stricter of the two
    /// and the one the interlock reasoned about.
    ///
    /// Defaulted, like `kind`: items staged before the field existed load as
    /// `None` and release exactly as they did before.
    #[serde(default)]
    pub workspace: Option<PathBuf>,
    /// The conversation's taint at the moment of staging. An armed snapshot
    /// means third-party text was in context when this draft was written —
    /// review it as possibly an attacker's words, not the assistant's.
    #[serde(default)]
    pub taint: Taint,
    pub created_at: String,
    #[serde(default)]
    pub resolved_at: Option<String>,
    /// Why it was rejected, when it was.
    #[serde(default)]
    pub reason: Option<String>,
    /// The last release attempt's failure, if any. A failed send stays
    /// `pending` — the draft is still good; the delivery was not.
    #[serde(default)]
    pub error: Option<String>,
}

impl OutboxItem {
    pub fn edited(&self) -> bool {
        self.args != self.args_before
    }

    /// Whether `mecha reflect` may mine this item as a **writing** correction.
    ///
    /// A `writing`-domain reflection can become a consolidated rule, and a rule
    /// rides in every future run's system prompt inside the cached prefix. That
    /// is the longest half-life anything in this project has, so what feeds it
    /// is filtered structurally rather than by a prompt asking the reflector to
    /// use its judgement:
    ///
    /// - **Sent, and edited.** An unedited release is not a correction (that it
    ///   is *positive* evidence is a separate, unread signal); a rejected one
    ///   never went out.
    /// - **A message.** A publish's `diff(args_before, args)` is a changed
    ///   filesystem path or visibility flag. Mining it would teach voice rules
    ///   from bookkeeping — the same mistake as learning from
    ///   `"Blocked by a hook:"`, which is machine policy read as a human
    ///   correction.
    pub fn mineable_as_writing(&self) -> bool {
        self.kind == OutboxKind::Message && self.status == "sent" && self.edited()
    }
}

/// What the agent loop consults per call: the store plus the routed names.
///
/// `session_id` is interior-mutable because the front-end learns it after the
/// agent (and its default [`RunContext`](crate::agent::RunContext)) is built:
/// the session is created at run start, the route at setup. Best-effort —
/// `None` on front-ends that record no session (batch, eval).
pub struct OutboxRoute {
    pub store: OutboxStore,
    routed: std::collections::BTreeSet<String>,
    publishes: std::collections::BTreeSet<String>,
    session_id: std::sync::Mutex<Option<String>>,
}

impl OutboxRoute {
    pub fn new(
        store: OutboxStore,
        routed: impl IntoIterator<Item = String>,
        publishes: impl IntoIterator<Item = String>,
    ) -> Self {
        OutboxRoute {
            store,
            routed: routed.into_iter().collect(),
            publishes: publishes.into_iter().collect(),
            session_id: std::sync::Mutex::new(None),
        }
    }

    pub fn routes(&self, tool: &str) -> bool {
        self.routed.contains(tool)
    }

    pub fn routed(&self) -> impl Iterator<Item = &str> {
        self.routed.iter().map(String::as_str)
    }

    /// A tool's kind, which is config's to declare and never the tool's: the
    /// loop must not learn what a publish is, and a third-party MCP server
    /// cannot be trusted to say. Anything unnamed is a message, which is the
    /// conservative default — it keeps the arguments reviewable and the item
    /// mineable, and the cost of getting it wrong is a voice rule learned from
    /// a path rather than a page nobody could review.
    pub fn kind_of(&self, tool: &str) -> OutboxKind {
        if self.publishes.contains(tool) {
            OutboxKind::Publish
        } else {
            OutboxKind::Message
        }
    }

    /// Names declared as publishes. Used at startup to warn about one that is
    /// not routed at all — it would execute unstaged, which is the
    /// silently-degrading-sandbox shape the routed-name warning already
    /// catches.
    pub fn publishes(&self) -> impl Iterator<Item = &str> {
        self.publishes.iter().map(String::as_str)
    }

    pub fn set_session_id(&self, id: &str) {
        if let Ok(mut slot) = self.session_id.lock() {
            *slot = Some(id.to_string());
        }
    }

    pub fn session_id(&self) -> Option<String> {
        self.session_id.lock().ok().and_then(|s| s.clone())
    }
}

pub struct OutboxStore {
    root: PathBuf,
}

/// Holds the store's writer lock for as long as it lives.
pub struct OutboxLock {
    _file: std::fs::File,
}

impl OutboxStore {
    pub fn default_root() -> Result<PathBuf> {
        if let Ok(dir) = std::env::var("MECHA_OUTBOX_DIR") {
            return Ok(PathBuf::from(dir));
        }
        Ok(crate::work::mecha_home()?.join("outbox"))
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        crate::create_private_dir(&root).with_context(|| format!("creating {}", root.display()))?;
        Ok(OutboxStore { root })
    }

    /// Open at the default location only if it already exists — for read
    /// paths that must not create state as a side effect.
    pub fn open_existing_default() -> Option<Self> {
        let root = Self::default_root().ok()?;
        root.is_dir().then_some(OutboxStore { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Stage a drafted call. No lock: the id is fresh, so there is no state
    /// to race on, and the agent loop must never wait on a review session.
    pub fn stage(
        &self,
        tool: &str,
        kind: OutboxKind,
        args: Value,
        taint: Taint,
        session_id: Option<String>,
        workspace: Option<PathBuf>,
    ) -> Result<OutboxItem> {
        let item = OutboxItem {
            id: Session::new_id(),
            status: "pending".into(),
            tool: tool.to_string(),
            kind,
            summary: summarize(tool, &args),
            args_before: args.clone(),
            args,
            session_id,
            workspace,
            taint,
            created_at: chrono::Utc::now().to_rfc3339(),
            resolved_at: None,
            reason: None,
            error: None,
        };
        self.write_item(&item)?;
        Ok(item)
    }

    /// Every item, oldest first.
    pub fn items(&self) -> Result<Vec<OutboxItem>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&self.root)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match serde_json::from_str(&std::fs::read_to_string(&path)?) {
                Ok(item) => out.push(item),
                Err(e) => {
                    tracing::warn!("skipping unreadable outbox item {}: {e}", path.display())
                }
            }
        }
        out.sort_by(|a: &OutboxItem, b: &OutboxItem| a.id.cmp(&b.id));
        Ok(out)
    }

    /// Find one item by id or unique prefix. Ambiguity is an error rather
    /// than a guess, same as session and proposal lookup.
    pub fn item(&self, id: &str) -> Result<OutboxItem> {
        let all = self.items()?;
        let matches: Vec<&OutboxItem> = all.iter().filter(|i| i.id.starts_with(id)).collect();
        match matches.len() {
            0 => anyhow::bail!("no outbox item matching `{id}`"),
            1 => Ok(matches[0].clone()),
            n => anyhow::bail!(
                "`{id}` matches {n} outbox items: {}",
                matches
                    .iter()
                    .map(|i| i.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    /// One item by its exact store-minted id — a single file read, never a
    /// directory scan. For the hot paths (a button press on an event loop)
    /// that already hold the full id and must not pay `items()`'s
    /// read-and-parse of every draft ever staged. Prefix lookup stays
    /// [`OutboxStore::item`]'s business.
    ///
    /// The id is validated by shape *before* it is joined onto the store
    /// root: ids arrive here from button payloads, and a value shaped like a
    /// path (`../…`) must be refused, not resolved. A hostile shape is an
    /// error; a missing item is `Ok(None)`; a torn file is an error, so a
    /// caller that maps errors to "unreadable" keeps failing closed.
    pub fn item_exact(&self, id: &str) -> Result<Option<OutboxItem>> {
        anyhow::ensure!(
            is_item_id(id),
            "`{id}` is not shaped like an outbox id"
        );
        let path = self.root.join(format!("{id}.json"));
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Ok(Some(serde_json::from_str(&text).with_context(|| {
            format!("parsing {}", path.display())
        })?))
    }

    /// Replace a pending item's release arguments. `args_before` is untouched
    /// — it is the baseline the learning capture diffs against.
    pub fn update_args(&self, id: &str, args: Value) -> Result<OutboxItem> {
        let mut item = self.item(id)?;
        anyhow::ensure!(
            item.status == "pending",
            "outbox item {} is {}, not pending",
            item.id,
            item.status
        );
        item.args = args;
        item.summary = summarize(&item.tool, &item.args);
        self.write_item(&item)?;
        Ok(item)
    }

    /// Resolve a pending item as `sent` or `rejected`, in place — the file is
    /// its own audit record, so nothing moves to an archive.
    pub fn resolve(&self, id: &str, status: &str, reason: Option<String>) -> Result<OutboxItem> {
        let mut item = self.item(id)?;
        anyhow::ensure!(
            item.status == "pending",
            "outbox item {} is {}, not pending",
            item.id,
            item.status
        );
        item.status = status.to_string();
        item.resolved_at = Some(chrono::Utc::now().to_rfc3339());
        item.reason = reason;
        item.error = None;
        self.write_item(&item)?;
        Ok(item)
    }

    /// Record a failed release attempt. The item stays `pending`: the draft
    /// is still good, and the next `send` retries.
    pub fn record_error(&self, id: &str, error: &str) -> Result<()> {
        let mut item = self.item(id)?;
        item.error = Some(error.to_string());
        self.write_item(&item)
    }

    fn write_item(&self, item: &OutboxItem) -> Result<()> {
        let path = self.root.join(format!("{}.json", item.id));
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(item)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Writer lock for read-modify-write paths (edit, send, reject). Taken
    /// before reading the item acted on; never held across `$EDITOR`.
    pub fn lock(&self) -> Result<OutboxLock> {
        use std::os::unix::io::AsRawFd;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.root.join(".lock"))?;
        // SAFETY: flock on an fd we own, held open by the returned guard.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error()).context("locking the outbox");
        }
        Ok(OutboxLock { _file: file })
    }
}

/// The shape of a store-minted id (`Session::new_id`: a timestamp, a hyphen,
/// a uuid fragment). Checked before an id from the outside is joined onto the
/// store root — nothing with a separator or a dot can name a file elsewhere.
fn is_item_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A per-line diff of two argument renderings, `- `/`+ ` prefixed. Line-set
/// based: enough to show *what* changed in a draft without a diff crate, and
/// the full before/after always survives on the item itself. Used by both
/// `mecha outbox show` and the reflect pass that mines edits.
pub fn diff_args(before: &Value, after: &Value) -> String {
    let pretty = |v: &Value| serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string());
    let b = pretty(before);
    let a = pretty(after);
    let b_lines: Vec<&str> = b.lines().collect();
    let a_lines: Vec<&str> = a.lines().collect();
    let mut out = String::new();
    for line in &b_lines {
        if !a_lines.contains(line) {
            out.push_str(&format!("  - {line}\n"));
        }
    }
    for line in &a_lines {
        if !b_lines.contains(line) {
            out.push_str(&format!("  + {line}\n"));
        }
    }
    if out.is_empty() {
        out.push_str("  (no textual change)\n");
    }
    out
}

/// One line for the list view: who and what when the arguments say, the
/// compact JSON when they do not.
///
/// Keyed on well-known argument *names*, never on the tool — the store stays
/// tool-agnostic, but a queue of mail drafts whose rows all lead with
/// `{"body_markdown":…` made every review surface start with the least
/// informative bytes of each item. Anything without the conventional fields
/// falls back to what it always was.
fn summarize(tool: &str, args: &Value) -> String {
    let text = headline(args).unwrap_or_else(|| serde_json::to_string(args).unwrap_or_default());
    format!("{tool} {}", clip(text, 80))
}

/// "to a@x — \"subject\"", when the arguments carry the conventional names.
fn headline(args: &Value) -> Option<String> {
    let map = args.as_object()?;
    let field = |key: &str| {
        map.get(key)
            .and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                // `to` is a list on some surfaces and a string on others.
                Value::Array(a) => Some(
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
                _ => None,
            })
            .filter(|s| !s.trim().is_empty())
    };
    let to = field("to");
    let subject = field("subject").or_else(|| field("title"));
    match (to, subject) {
        (Some(to), Some(subject)) => Some(format!("to {to} — \"{subject}\"")),
        (Some(to), None) => Some(format!("to {to}")),
        (None, Some(subject)) => Some(format!("\"{subject}\"")),
        (None, None) => None,
    }
}

/// Truncate on a char boundary; the text can be any UTF-8.
fn clip(mut text: String, max: usize) -> String {
    if text.len() > max {
        let cut = (0..=max)
            .rev()
            .find(|&i| text.is_char_boundary(i))
            .unwrap_or(0);
        text.truncate(cut);
        text.push('…');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mecha-outbox-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_summary_leads_with_who_and_what_when_the_arguments_say() {
        // The conventional fields, in every combination they arrive in.
        assert_eq!(
            summarize(
                "mail__send",
                &json!({"to": "a@x.org", "subject": "Tuesday?", "body_markdown": "long…"})
            ),
            "mail__send to a@x.org — \"Tuesday?\""
        );
        assert_eq!(
            summarize(
                "mail__send",
                &json!({"to": ["a@x.org", "b@x.org"], "body": "hi"})
            ),
            "mail__send to a@x.org, b@x.org"
        );
        assert_eq!(
            summarize(
                "cal__event_create",
                &json!({"title": "Standup", "start": "…"})
            ),
            "cal__event_create \"Standup\""
        );

        // Without them, the compact JSON it always was — and still bounded.
        let plain = summarize("factory__bundle_publish", &json!({"bundle": "/tmp/x"}));
        assert!(plain.contains("bundle"), "{plain}");
        let long = summarize("t", &json!({"to": "x".repeat(200)}));
        assert!(long.len() < 120, "{}", long.len());
        assert!(long.ends_with('…'), "{long}");

        // An empty `to` is absence, not an addressee.
        assert_eq!(
            summarize("t", &json!({"to": "", "body": "x"})),
            r#"t {"body":"x","to":""}"#
        );
    }

    #[test]
    fn an_item_round_trips_and_lists_in_id_order() {
        let root = scratch("roundtrip");
        let store = OutboxStore::open(&root).unwrap();

        let a = store
            .stage(
                "web__fetch",
                OutboxKind::Message,
                json!({"url": "https://a"}),
                Taint::default(),
                None,
                None,
            )
            .unwrap();
        let b = store
            .stage(
                "email__send",
                OutboxKind::Message,
                json!({"to": "x@y"}),
                Taint {
                    private: true,
                    untrusted: true,
                },
                Some("sess-1".into()),
                None,
            )
            .unwrap();

        let items = store.items().unwrap();
        assert_eq!(items.len(), 2);
        // Ids sort by creation time, so listing order is staging order.
        assert_eq!(items[0].id, a.id.min(b.id.clone()));

        let loaded = store.item(&b.id).unwrap();
        assert_eq!(loaded.tool, "email__send");
        assert!(loaded.taint.trifecta_armed());
        assert_eq!(loaded.session_id.as_deref(), Some("sess-1"));
        assert_eq!(loaded.args, loaded.args_before);
        assert!(!loaded.edited());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_prefix_that_matches_two_items_is_an_error_not_a_guess() {
        let root = scratch("prefix");
        let store = OutboxStore::open(&root).unwrap();
        store
            .stage(
                "t",
                OutboxKind::Message,
                json!({}),
                Taint::default(),
                None,
                None,
            )
            .unwrap();
        store
            .stage(
                "t",
                OutboxKind::Message,
                json!({}),
                Taint::default(),
                None,
                None,
            )
            .unwrap();

        // Both ids share the timestamp prefix of the second they were made in.
        let err = store.item("2").unwrap_err();
        assert!(err.to_string().contains("matches 2"), "{err}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn editing_replaces_args_and_never_touches_the_baseline() {
        let root = scratch("edit");
        let store = OutboxStore::open(&root).unwrap();
        let item = store
            .stage(
                "web__fetch",
                OutboxKind::Message,
                json!({"url": "https://a"}),
                Taint::default(),
                None,
                None,
            )
            .unwrap();

        let edited = store
            .update_args(&item.id, json!({"url": "https://b"}))
            .unwrap();
        assert!(edited.edited());
        assert_eq!(edited.args_before, json!({"url": "https://a"}));
        assert_eq!(edited.args, json!({"url": "https://b"}));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The rule that protects every future run's system prompt. A publish's
    /// edit diff is a changed filesystem path, and a `writing` reflection
    /// becomes a rule in the cached prefix — the same class of mistake as
    /// mining `"Blocked by a hook:"` as if a human had said it. Fails on the
    /// old behaviour, which mined any sent-and-edited item.
    #[test]
    fn the_writing_miner_takes_edited_messages_and_never_publishes() {
        let root = scratch("mineable");
        let store = OutboxStore::open(&root).unwrap();

        let cases = [
            (OutboxKind::Message, "sent", true, true),
            // A publish, edited and sent — the one the old filter accepted.
            (OutboxKind::Publish, "sent", true, false),
            // Unedited is not a correction; rejected never went out.
            (OutboxKind::Message, "sent", false, false),
            (OutboxKind::Message, "rejected", true, false),
            (OutboxKind::Message, "pending", true, false),
        ];
        for (kind, status, edited, expected) in cases {
            let mut item = store
                .stage(
                    "x__send",
                    kind,
                    json!({"path": "/tmp/a"}),
                    Taint::default(),
                    None,
                    None,
                )
                .unwrap();
            item.status = status.into();
            if edited {
                item.args = json!({"path": "/tmp/b"});
            }
            assert_eq!(
                item.mineable_as_writing(),
                expected,
                "{kind:?} / {status} / edited={edited}"
            );
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The kind is config's to declare, and anything unnamed stays a message —
    /// which keeps the arguments reviewable rather than silently hiding them.
    #[test]
    fn a_routes_kind_comes_from_config_and_defaults_to_message() {
        let root = scratch("kindof");
        let store = OutboxStore::open(&root).unwrap();
        let route = OutboxRoute::new(
            store,
            [
                "mail__send".to_string(),
                "factory__bundle_publish".to_string(),
            ],
            ["factory__bundle_publish".to_string()],
        );
        assert_eq!(
            route.kind_of("factory__bundle_publish"),
            OutboxKind::Publish
        );
        assert_eq!(route.kind_of("mail__send"), OutboxKind::Message);
        assert_eq!(route.kind_of("never__heard_of_it"), OutboxKind::Message);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Items written before the field existed must load as what they in fact
    /// were, or an upgrade would reclassify every staged email as unknown.
    #[test]
    fn an_item_recorded_before_kinds_existed_loads_as_a_message() {
        let item: OutboxItem = serde_json::from_value(json!({
            "id": "20260101-000000-abc",
            "status": "sent",
            "tool": "mail__send",
            "args_before": {"body": "a"},
            "args": {"body": "b"},
            "summary": "mail__send",
            "created_at": "2026-01-01T00:00:00Z",
        }))
        .unwrap();
        assert_eq!(item.kind, OutboxKind::Message);
        assert!(item.mineable_as_writing());
        // And the same for the jail it was drafted under: an older item names
        // none, and a release falls back to the reviewer's workspace, which is
        // exactly what it did before the field existed.
        assert_eq!(item.workspace, None);
    }

    /// A staged call is a deferred tool call, and the release happens in
    /// another process from another directory. Without the drafting jail on
    /// the item, `{"bundle": "site"}` resolves against wherever the reviewer
    /// stands — an absolute path fails loudly, and a relative one silently
    /// publishes whatever `./site` happens to be there.
    #[test]
    fn a_staged_call_records_the_jail_it_was_drafted_under() {
        let root = scratch("workspace");
        let store = OutboxStore::open(&root).unwrap();
        let jail = PathBuf::from("/home/someone/.mecha/work/morning");

        let item = store
            .stage(
                "factory__bundle_publish",
                OutboxKind::Publish,
                json!({"bundle": "site", "id": "brief"}),
                Taint::default(),
                None,
                Some(jail.clone()),
            )
            .unwrap();
        assert_eq!(item.workspace.as_ref(), Some(&jail));

        // And it survives the round-trip through the file, which is the only
        // form the reviewing process ever sees.
        let loaded = store.item(&item.id).unwrap();
        assert_eq!(loaded.workspace.as_ref(), Some(&jail));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolution_rewrites_in_place_and_only_pending_resolves() {
        let root = scratch("resolve");
        let store = OutboxStore::open(&root).unwrap();
        let item = store
            .stage(
                "t",
                OutboxKind::Message,
                json!({}),
                Taint::default(),
                None,
                None,
            )
            .unwrap();

        let sent = store.resolve(&item.id, "sent", None).unwrap();
        assert_eq!(sent.status, "sent");
        assert!(sent.resolved_at.is_some());
        assert_eq!(
            store.items().unwrap().len(),
            1,
            "resolved in place, not archived"
        );

        let err = store.resolve(&item.id, "rejected", None).unwrap_err();
        assert!(err.to_string().contains("not pending"), "{err}");
        let err = store.update_args(&item.id, json!({"x": 1})).unwrap_err();
        assert!(err.to_string().contains("not pending"), "{err}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_release_records_the_error_and_stays_pending() {
        let root = scratch("error");
        let store = OutboxStore::open(&root).unwrap();
        let item = store
            .stage(
                "t",
                OutboxKind::Message,
                json!({}),
                Taint::default(),
                None,
                None,
            )
            .unwrap();

        store.record_error(&item.id, "server unreachable").unwrap();
        let loaded = store.item(&item.id).unwrap();
        assert_eq!(loaded.status, "pending");
        assert_eq!(loaded.error.as_deref(), Some("server unreachable"));

        // A later successful resolution clears the stale error.
        let sent = store.resolve(&item.id, "sent", None).unwrap();
        assert_eq!(sent.error, None);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The targeted read behind the hot paths: one file, found or honestly
    /// missing, and a value shaped like a path is refused before it can name
    /// a file outside the store — even one that exists and would parse.
    #[test]
    fn an_exact_lookup_reads_one_file_and_refuses_a_hostile_id() {
        let root = scratch("exact");
        let store = OutboxStore::open(&root).unwrap();
        let staged = store
            .stage(
                "mail__send",
                OutboxKind::Message,
                json!({"to": "a@x.org"}),
                Taint::default(),
                None,
                None,
            )
            .unwrap();

        let found = store.item_exact(&staged.id).unwrap().expect("found");
        assert_eq!(found.id, staged.id);
        assert_eq!(found.tool, "mail__send");

        // Missing is None, not an error: the caller decides what absence means.
        assert!(store
            .item_exact("20990101T000000-deadbeef")
            .unwrap()
            .is_none());

        // A perfectly valid item file sitting *beside* the store, reachable
        // only by traversal — the refusal below is not vacuous.
        let outside = root.parent().unwrap().join("mecha-outbox-evil.json");
        std::fs::write(&outside, serde_json::to_string_pretty(&staged).unwrap()).unwrap();
        for hostile in [
            "../mecha-outbox-evil",
            "a/b",
            "a.b",
            ".",
            "",
            &"x".repeat(200),
        ] {
            assert!(
                store.item_exact(hostile).is_err(),
                "{hostile:?} must be refused, not resolved"
            );
        }
        let _ = std::fs::remove_file(&outside);

        let _ = std::fs::remove_dir_all(&root);
    }
}
