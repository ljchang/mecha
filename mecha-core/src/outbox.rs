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

/// One staged outbound action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboxItem {
    pub id: String,
    /// `pending` | `sent` | `rejected`.
    pub status: String,
    /// The tool a release will execute, by registry name (`web__fetch`).
    pub tool: String,
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
    session_id: std::sync::Mutex<Option<String>>,
}

impl OutboxRoute {
    pub fn new(store: OutboxStore, routed: impl IntoIterator<Item = String>) -> Self {
        OutboxRoute {
            store,
            routed: routed.into_iter().collect(),
            session_id: std::sync::Mutex::new(None),
        }
    }

    pub fn routes(&self, tool: &str) -> bool {
        self.routed.contains(tool)
    }

    pub fn routed(&self) -> impl Iterator<Item = &str> {
        self.routed.iter().map(String::as_str)
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
        let home = dirs::home_dir().context("cannot determine home directory")?;
        Ok(home.join(".mecha").join("outbox"))
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)
            .with_context(|| format!("creating {}", root.display()))?;
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
        args: Value,
        taint: Taint,
        session_id: Option<String>,
    ) -> Result<OutboxItem> {
        let item = OutboxItem {
            id: Session::new_id(),
            status: "pending".into(),
            tool: tool.to_string(),
            summary: summarize(tool, &args),
            args_before: args.clone(),
            args,
            session_id,
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
                matches.iter().map(|i| i.id.as_str()).collect::<Vec<_>>().join(", ")
            ),
        }
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

/// A per-line diff of two argument renderings, `- `/`+ ` prefixed. Line-set
/// based: enough to show *what* changed in a draft without a diff crate, and
/// the full before/after always survives on the item itself. Used by both
/// `mecha outbox show` and the reflect pass that mines edits.
pub fn diff_args(before: &Value, after: &Value) -> String {
    let pretty =
        |v: &Value| serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string());
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

/// One line for the list view: the tool plus as much of the compact argument
/// JSON as fits.
fn summarize(tool: &str, args: &Value) -> String {
    let compact = serde_json::to_string(args).unwrap_or_default();
    let mut text = compact;
    if text.len() > 80 {
        // Truncate on a char boundary; arguments can be any UTF-8.
        let cut = (0..=80).rev().find(|&i| text.is_char_boundary(i)).unwrap_or(0);
        text.truncate(cut);
        text.push('…');
    }
    format!("{tool} {text}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("mecha-outbox-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn an_item_round_trips_and_lists_in_id_order() {
        let root = scratch("roundtrip");
        let store = OutboxStore::open(&root).unwrap();

        let a = store
            .stage("web__fetch", json!({"url": "https://a"}), Taint::default(), None)
            .unwrap();
        let b = store
            .stage(
                "email__send",
                json!({"to": "x@y"}),
                Taint { private: true, untrusted: true },
                Some("sess-1".into()),
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
        store.stage("t", json!({}), Taint::default(), None).unwrap();
        store.stage("t", json!({}), Taint::default(), None).unwrap();

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
            .stage("web__fetch", json!({"url": "https://a"}), Taint::default(), None)
            .unwrap();

        let edited = store.update_args(&item.id, json!({"url": "https://b"})).unwrap();
        assert!(edited.edited());
        assert_eq!(edited.args_before, json!({"url": "https://a"}));
        assert_eq!(edited.args, json!({"url": "https://b"}));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolution_rewrites_in_place_and_only_pending_resolves() {
        let root = scratch("resolve");
        let store = OutboxStore::open(&root).unwrap();
        let item = store.stage("t", json!({}), Taint::default(), None).unwrap();

        let sent = store.resolve(&item.id, "sent", None).unwrap();
        assert_eq!(sent.status, "sent");
        assert!(sent.resolved_at.is_some());
        assert_eq!(store.items().unwrap().len(), 1, "resolved in place, not archived");

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
        let item = store.stage("t", json!({}), Taint::default(), None).unwrap();

        store.record_error(&item.id, "server unreachable").unwrap();
        let loaded = store.item(&item.id).unwrap();
        assert_eq!(loaded.status, "pending");
        assert_eq!(loaded.error.as_deref(), Some("server unreachable"));

        // A later successful resolution clears the stale error.
        let sent = store.resolve(&item.id, "sent", None).unwrap();
        assert_eq!(sent.error, None);

        let _ = std::fs::remove_dir_all(&root);
    }
}
