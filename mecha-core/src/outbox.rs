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

use anyhow::{bail, Context, Result};
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

/// Where a staged draft came from: the run, the jail, the call — and which of
/// its arguments this harness wrote rather than the model.
///
/// A struct rather than three more parameters, and not only because `stage`
/// had reached clippy's argument ceiling. `session_id` and `call_id` are both
/// `Option<String>` and both optional, so positionally they are
/// interchangeable to the compiler and not at all interchangeable to the
/// store — one names the transcript, the other names a single call inside it,
/// and a swap would compile, store, and only surface as a draft whose source
/// pane is wrong.
///
/// Every field is optional because every field is genuinely absent for some
/// legitimate staging: `mecha mail compose` has no run and no call, a tool
/// built over a fixed directory has no per-run workspace, and an item written
/// before `call_id` existed has none of the third.
#[derive(Debug, Clone, Default)]
pub struct Provenance {
    /// The session whose transcript holds the staging call.
    pub session_id: Option<String>,
    /// The jail the tool would really have executed under. A release happens
    /// in another process from another directory; a staged path means nothing
    /// without the root it was written against.
    pub workspace: Option<PathBuf>,
    /// The `tool_use` id of the call that staged this — see
    /// [`OutboxItem::call_id`].
    pub call_id: Option<String>,
    /// Argument keys the *harness* wrote, not the model — see
    /// [`OutboxItem::filled_defaults`].
    pub filled_defaults: Vec<String>,
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
    ///
    /// **Equal to `args` at stage time, and that is load-bearing rather than
    /// incidental.** `edited()` is `args != args_before`, so anything that
    /// makes them differ before a human has touched the draft reports every
    /// send as a correction: `writing_outcome()` returns `SentEdited` instead
    /// of `SentUnchanged`, which flips the appraisal signal from `+1.0 /
    /// Own` to `-1.0 / Owner`, and `mineable_as_writing()` feeds the
    /// harness's own bookkeeping to a reflector whose rules ride in every
    /// future run's cached prefix. So a default the loop pins into a draft
    /// belongs in *both*, and what locates the staging call in the transcript
    /// is [`OutboxItem::call_id`], not this.
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
    /// The `tool_use` id of the call that staged this, when a call staged it.
    ///
    /// The anchor [`crate::outbox_source`] walks a transcript to find. It used
    /// to match `(tool, args_before)`, which is identity by content and was
    /// true only for as long as nothing between the model's call and the
    /// stored draft touched the arguments — the loop now pins schema defaults
    /// into a staged call, so a `mail_reply` whose `reply_all` was filled no
    /// longer equals its own recorded input, the walk runs past the staging
    /// call, and the draft joins to *itself*.
    ///
    /// `None` for a draft no tool call produced (`mecha mail compose`) and for
    /// every item written before this field existed, which is why the walk
    /// keeps the old argument match as its fallback: defaulted on load, on the
    /// append-only store's rule, so a pending draft staged yesterday still
    /// finds its source today.
    #[serde(default)]
    pub call_id: Option<String>,
    /// The argument keys the loop pinned from the tool's schema, rather than
    /// the model naming them.
    ///
    /// A value the harness wrote is not evidence of what the run was working
    /// from, and [`crate::outbox_source`] joins a draft back to its source on
    /// precisely such arguments: `provider_ids` takes every string argument
    /// that is neither addressing nor prose, so a pinned
    /// `calendar_id: "primary"` became a join key on every calendar draft —
    /// and `Join::Asked` has no entropy floor, because it matches key *and*
    /// value and "a coincidence has to happen twice". That held while both
    /// sides were the model's; one side is now a constant, so the second
    /// coincidence is free, and an unrelated calendar listing gets presented
    /// as the thing the draft was written from.
    ///
    /// Empty for a draft nothing was pinned into, and for every item written
    /// before this field existed — on the append-only store's rule, where the
    /// old value is also the true one, since nothing pinned anything then.
    #[serde(default)]
    pub filled_defaults: Vec<String>,
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
    /// - **Sent, and edited.** An unedited release is not a correction — it is
    ///   *positive* evidence, which is [`WritingOutcome::SentUnchanged`] and no
    ///   longer unread. A rejected one never went out.
    /// - **A message.** A publish's `diff(args_before, args)` is a changed
    ///   filesystem path or visibility flag. Mining it would teach voice rules
    ///   from bookkeeping — the same mistake as learning from
    ///   `"Blocked by a hook:"`, which is machine policy read as a human
    ///   correction.
    pub fn mineable_as_writing(&self) -> bool {
        self.writing_outcome() == Some(WritingOutcome::SentEdited)
    }

    /// What this item says about the drafting, if it says anything.
    ///
    /// **The signed half of the outbox's evidence, and the cheapest signal in
    /// the goal system** (`docs/GOAL-SYSTEM-DESIGN.md` §5.2). Every evaluative
    /// signal mecha had was a cost or a correction: `Trigger` is four ways of
    /// saying a person stepped in, and every `Metric` is phrased so that lower
    /// is better. So a draft could be recorded as *wrong* and never as *right*,
    /// and the `writing` domain learned only from what displeased.
    ///
    /// This needed no new recording. `args_before` has always been kept beside
    /// `args`, so "the owner read a letter written in their name and sent it as
    /// drafted" was already on disk and simply had no reader.
    ///
    /// **It is the owner's judgement, not the agent's**, which is what makes it
    /// immune to the failure that rules out scoring your own work: nothing the
    /// model does can produce a `SentUnchanged` except drafting something a
    /// person then chose to send unaltered.
    ///
    /// `None` for anything that says nothing about drafting — a pending item
    /// (undecided), a rejected one (never went out, and its reason is the
    /// record), or a publish (whose diff is a path and a visibility flag, not
    /// prose).
    pub fn writing_outcome(&self) -> Option<WritingOutcome> {
        if self.kind != OutboxKind::Message || self.status != "sent" {
            return None;
        }
        Some(match self.edited() {
            true => WritingOutcome::SentEdited,
            false => WritingOutcome::SentUnchanged,
        })
    }
}

/// What a released draft says about how it was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritingOutcome {
    /// The owner sent it as drafted. Positive evidence.
    SentUnchanged,
    /// The owner rewrote it before sending. The correction `reflect` mines.
    SentEdited,
}

/// How the drafting has been going, counted over released items.
///
/// Deliberately counts and never judges — the threshold for "well enough"
/// belongs to whoever acts on it, the same division `runlog` keeps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WritingTally {
    pub unchanged: usize,
    pub edited: usize,
}

impl WritingTally {
    pub fn of<'a>(items: impl IntoIterator<Item = &'a OutboxItem>) -> WritingTally {
        let mut tally = WritingTally::default();
        for item in items {
            match item.writing_outcome() {
                Some(WritingOutcome::SentUnchanged) => tally.unchanged += 1,
                Some(WritingOutcome::SentEdited) => tally.edited += 1,
                None => {}
            }
        }
        tally
    }

    pub fn sent(&self) -> usize {
        self.unchanged + self.edited
    }

    /// The share of sent drafts that went out as written.
    ///
    /// `None` over an empty denominator, never zero. "Nothing was edited" and
    /// "nothing has been sent" are opposite findings, and rendering the second
    /// as 0% would report an outbox nobody has used as one whose every draft
    /// was rewritten — the null-run bug, in the one measure here that is
    /// supposed to say something went *well*.
    pub fn unchanged_rate(&self) -> Option<f64> {
        (self.sent() > 0).then(|| self.unchanged as f64 / self.sent() as f64)
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
        from: Provenance,
    ) -> Result<OutboxItem> {
        let Provenance {
            session_id,
            workspace,
            call_id,
            filled_defaults,
        } = from;
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
            call_id,
            filled_defaults,
        };
        self.write_item(&item)?;
        Ok(item)
    }

    /// Every item, oldest first. A file that fails to parse is skipped with
    /// a `tracing::warn!` rather than failing the whole read — right for a
    /// listing, which should show what it can. See [`Self::items_strict`]
    /// for the caller that cannot accept a silent skip.
    pub fn items(&self) -> Result<Vec<OutboxItem>> {
        self.items_counting().map(|(items, _)| items)
    }

    /// [`items`](Self::items), and how many `.json` files it skipped. For a
    /// reader that must not mistake a short list for a small store: the
    /// appraisal's request arm asks "was anything drafted for this?", and
    /// a skew-version draft skipped into a `tracing::warn!` nobody reads
    /// answered "no" for a request the triage had answered (found on
    /// review). `Session::list_counting`'s shape, one store over.
    pub fn items_counting(&self) -> Result<(Vec<OutboxItem>, usize)> {
        let mut skipped = 0usize;
        let items = self.items_impl(false, &mut skipped)?;
        Ok((items, skipped))
    }

    /// Every item, oldest first — but a single unparseable file fails the
    /// whole read instead of being skipped.
    ///
    /// `items()`'s skip-and-warn is right for a listing and wrong for a
    /// caller about to write a permanent record from what it read. **Not**
    /// because of a half-written file mid-save — this module's own header
    /// names the reason that cannot happen: temp-sibling-and-rename means a
    /// reader never sees a partial write, and `items_impl` only looks at the
    /// `.json` extension the rename lands on, so the `.json.tmp` sibling is
    /// invisible to the walk regardless. The realistic cause is a
    /// *persistent* one: a stray file, or an item written by a schema this
    /// binary cannot read (the hand-rolled `Deserialize` on [`OutboxKind`]
    /// and `Proposed` exists for exactly that skew). Either way, `items()`
    /// would pass it through as a silently short result — indistinguishable
    /// from an outbox that simply has fewer drafts. `mecha distill`'s
    /// episode tagging is exactly the caller that cannot accept that: its
    /// own `tracing::warn!` is invisible there anyway (the nightly runs
    /// with no `MECHA_LOG`), and because the cause is persistent rather
    /// than transient, a caller that reacts to this by only asking the
    /// operator to retry will keep failing the same way every night; see
    /// that caller's own handling for how it names the distinction.
    pub fn items_strict(&self) -> Result<Vec<OutboxItem>> {
        let mut skipped = 0usize;
        self.items_impl(true, &mut skipped)
    }

    fn items_impl(&self, strict: bool, skipped: &mut usize) -> Result<Vec<OutboxItem>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&self.root)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match serde_json::from_str(&std::fs::read_to_string(&path)?) {
                Ok(item) => out.push(item),
                Err(e) if strict => {
                    bail!("outbox item {} failed to parse: {e}", path.display())
                }
                Err(e) => {
                    *skipped += 1;
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
        anyhow::ensure!(is_item_id(id), "`{id}` is not shaped like an outbox id");
        let path = self.root.join(format!("{id}.json"));
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Ok(Some(
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?,
        ))
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

/// A staged message, shaped the way a person reads one.
///
/// [`OutboxKind::Publish`]'s lesson generalises: **a message's reviewable
/// object is the message**, not the JSON carrying it. A review surface that
/// prints `{"body_markdown": "Dear Dirk,\n\nThank you…"}` asks the reviewer to
/// decode escape sequences to find out what would be said in their name — and
/// "approve without reading" is the exact failure the outbox exists to
/// prevent, so a draft that is hard to read is a security cost rather than a
/// cosmetic one. It is also what an editor should open: editing prose inside a
/// JSON string literal is where a real newline becomes `\n`, a stray quote
/// becomes a parse error, and the whole edit is refused for a reason that has
/// nothing to do with what the person meant to say.
///
/// Keyed on well-known argument *names*, like [`headline`] and for the same
/// reason: the store stays tool-agnostic, so a tool nobody anticipated is
/// still reviewable — its fields land in `other` rather than vanishing.
///
/// **Nothing is dropped.** Every key of the arguments appears in exactly one
/// of `headers`, `body` or `other`, and there is a test on that, because a
/// field the reviewer cannot see is a field they approved without reading.
/// That is the whole difference between reshaping a draft and summarising it.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DraftView {
    /// Addressing and the other short scalars, in reading order.
    pub headers: Vec<(String, String)>,
    /// The prose, with its real newlines.
    pub body: Option<String>,
    /// Which argument the prose came from, so an edit writes it back to the
    /// same key rather than guessing a second time.
    pub body_field: Option<String>,
    /// Everything else, unshaped — shown after the body, never hidden.
    pub other: Vec<(String, String)>,
}

/// Header-ish arguments, in the order a person reads them rather than the
/// order a map hands them back.
/// Deliberately short. `thread_id` and `message_id` address the *provider*,
/// not a person, and a reviewer answering "would I send this?" needs them the
/// way a letter writer needs the postcode format — which is to say later, and
/// not above the prose. They fall through to `other`, which every surface
/// shows below the body.
/// `start_time`/`end_time` sit here rather than falling through to `other`
/// for one reason: `other` is in map order, which is alphabetical, so an
/// event read *end before start* — nonsense on a page and worse in an ear,
/// where a listener cannot glance back to sort it out.
const HEADER_FIELDS: [&str; 12] = [
    "to",
    "cc",
    "bcc",
    "channel",
    "subject",
    "title",
    "when",
    "start",
    "start_time",
    "end",
    "end_time",
    "account",
];

/// Arguments that carry the prose, most specific first. Exactly one wins; the
/// runners-up are ordinary arguments and are shown as such.
const BODY_FIELDS: [&str; 8] = [
    "body_markdown",
    "body_text",
    "body_html",
    "body",
    "text",
    "markdown",
    "message",
    "content",
];

impl DraftView {
    pub fn of(args: &Value) -> DraftView {
        let mut view = DraftView::default();
        let Some(map) = args.as_object() else {
            // Not an object: there is nothing to shape, and showing it raw is
            // still showing all of it.
            view.other.push(("arguments".into(), args.to_string()));
            return view;
        };
        for key in HEADER_FIELDS {
            if let Some(value) = map.get(key) {
                view.headers.push((key.to_string(), render(value)));
            }
        }
        for key in BODY_FIELDS {
            if let Some(text) = map.get(key).and_then(Value::as_str) {
                view.body = Some(text.to_string());
                view.body_field = Some(key.to_string());
                break;
            }
        }
        for (key, value) in map {
            if HEADER_FIELDS.contains(&key.as_str())
                || view.body_field.as_deref() == Some(key.as_str())
            {
                continue;
            }
            view.other.push((key.clone(), render(value)));
        }
        view
    }
}

/// A draft as it would be **read out loud** — every argument, nothing
/// summarised.
///
/// The reviewable object of a message is the message, and that rule does not
/// change when the reviewer is listening instead of looking. What changes is
/// that a listener cannot skim: they hear it once, in order, at speaking
/// speed, so the only safe spoken offer is one that utters the whole thing.
/// A paraphrase read aloud is not a smaller review, it is a different
/// document — and the field it leaves out (one more address on the `to` line)
/// is exactly the field an injection would add.
///
/// So this is [`DraftView`]'s three buckets spoken in reading order, and it
/// inherits that type's guarantee: **every argument key appears**, with a
/// test on it. The only thing it decides is wording.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SpokenDraft {
    /// The draft, in speakable lines. Concatenate with pauses between.
    pub lines: Vec<String>,
}

impl SpokenDraft {
    /// How much speech this is. Characters rather than words because that is
    /// what a TTS leg is actually handed, and the two are proportional at any
    /// rate worth caring about.
    pub fn chars(&self) -> usize {
        self.lines.iter().map(|l| l.chars().count() + 1).sum()
    }

    pub fn text(&self) -> String {
        self.lines.join(" ")
    }
}

/// An argument name as a person hears it: `body_markdown` → `Body markdown`.
///
/// Deliberately mechanical rather than a lookup table of nice phrasings. A
/// table would cover the tools thought of today and quietly mis-speak the
/// rest, and the store is tool-agnostic on purpose — an unanticipated field
/// must arrive sounding slightly stiff, never sounding like something else.
fn spoken_label(key: &str) -> String {
    let words = key.replace('_', " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => words,
    }
}

/// A value as it should be *heard*.
///
/// One case, and it is the case a calendar draft is made of:
/// `2026-08-28T14:30:00-04:00` read aloud is a run of digits nobody can check
/// a meeting against, and being checkable is the entire purpose of reading a
/// draft back. So a timestamp is spoken as a date and a time.
///
/// **Rendered in the offset the string itself carries, never in local time.**
/// A reviewer must hear the moment the draft actually names; translating it
/// into some other zone would be the wrong-bytes review arriving through the
/// one door built to prevent it. Anything that does not parse is spoken
/// unchanged — a value this does not understand must reach the listener as
/// itself, not as a guess.
fn spoken_value(value: &str) -> String {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
        let on = dt.format("%A %B %-d");
        return if dt.format("%M").to_string() == "00" {
            format!("{on} at {}", dt.format("%-I %p"))
        } else {
            format!("{on} at {}", dt.format("%-I:%M %p"))
        };
    }
    // A local datetime with no offset — which is what a calendar tool sends
    // when the zone rides in a separate `timezone` argument, spoken beside
    // it. Formatted, never *converted*: there is no offset here to convert
    // from, and inventing one would be the harness telling the listener a
    // different hour than the draft says.
    for form in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(value, form) {
            let on = dt.format("%A %B %-d");
            return if dt.format("%M").to_string() == "00" {
                format!("{on} at {}", dt.format("%-I %p"))
            } else {
                format!("{on} at {}", dt.format("%-I:%M %p"))
            };
        }
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return date.format("%A %B %-d").to_string();
    }
    value.to_string()
}

impl DraftView {
    /// This draft, spoken.
    pub fn spoken(&self) -> SpokenDraft {
        let mut lines = Vec::new();
        for (key, value) in &self.headers {
            lines.push(format!("{}: {}.", spoken_label(key), spoken_value(value)));
        }
        if let Some(body) = &self.body {
            // The prose is uttered as prose, with no label in front of it:
            // "Body markdown: Dear Dirk" is a field name a listener has to
            // parse past. Which argument carried it is `body_field`'s to
            // report, and no surface has ever needed to hear it.
            lines.push(body.trim().to_string());
        }
        for (key, value) in &self.other {
            lines.push(format!("{}: {}.", spoken_label(key), spoken_value(value)));
        }
        SpokenDraft { lines }
    }
}

/// One argument as a person should see it: a string as itself, a list of
/// addresses joined, anything else as its JSON. An empty value says so rather
/// than rendering as nothing — a blank `to` is a fact about the draft, and a
/// label with nothing after it reads as a display bug instead.
fn render(value: &Value) -> String {
    let text = match value {
        Value::String(s) => s.clone(),
        Value::Array(a) if a.iter().all(Value::is_string) => a
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        other => other.to_string(),
    };
    if text.trim().is_empty() {
        "(empty)".to_string()
    } else {
        text
    }
}

/// The arguments that address the **provider** rather than a person.
///
/// The leftovers of the classification [`DraftView`] already makes: not
/// addressing, not the prose, and a string. What remains — `thread_id`,
/// `message_id`, a Slack `ts` — names the *object* the call acts on, which is
/// exactly what two calls about the same object have in common. That makes it
/// the join key from a staged draft back to the read that produced it
/// ([`crate::outbox_source`]), and it lives here so the one decision about
/// which argument is which is still made once.
///
/// **Callers must also exclude whatever the harness pinned**
/// ([`OutboxItem::filled_defaults`]). This function sees arguments, not their
/// authors, and a value the loop wrote is a constant rather than a fact about
/// the run — `calendar_id: "primary"` is a string, so it lands here, and it
/// matches every calendar call in the session.
///
/// `account` and the other headers are excluded on purpose, and it is the
/// exclusion that makes the join worth anything: `{"account": "dartmouth"}`
/// is shared by every mail call in the session and would match all of them,
/// which is a filter that filters nothing. Provider ids are high-entropy
/// because they have to be.
pub fn provider_ids(args: &Value) -> Vec<(String, String)> {
    let Some(map) = args.as_object() else {
        return Vec::new();
    };
    let body = DraftView::of(args).body_field;
    map.iter()
        .filter(|(key, _)| !HEADER_FIELDS.contains(&key.as_str()))
        .filter(|(key, _)| body.as_deref() != Some(key.as_str()))
        .filter_map(|(key, value)| {
            let text = value.as_str()?;
            (!text.trim().is_empty()).then(|| (key.clone(), text.to_string()))
        })
        .collect()
}

/// Write an edited body back into the arguments it came from.
///
/// The inverse of [`DraftView::body_field`], and it lives here so the one
/// decision about which key holds the prose is made once. Returns the changed
/// arguments, or `None` when there was no body to replace — a caller must
/// then fall back to editing the arguments themselves rather than silently
/// changing nothing.
pub fn with_body(args: &Value, body: &str) -> Option<Value> {
    let field = DraftView::of(args).body_field?;
    let mut args = args.clone();
    args.as_object_mut()?
        .insert(field, Value::String(body.to_string()));
    Some(args)
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
                Provenance {
                    filled_defaults: Vec::new(),
                    session_id: None,
                    workspace: None,
                    call_id: None,
                },
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
                Provenance {
                    filled_defaults: Vec::new(),
                    session_id: Some("sess-1".into()),
                    workspace: None,
                    call_id: None,
                },
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
    fn items_skips_a_malformed_file_but_items_strict_bails_on_it() {
        let root = scratch("malformed");
        let store = OutboxStore::open(&root).unwrap();
        store
            .stage(
                "t",
                OutboxKind::Message,
                json!({}),
                Taint::default(),
                Provenance {
                    filled_defaults: Vec::new(),
                    session_id: None,
                    workspace: None,
                    call_id: None,
                },
            )
            .unwrap();
        // A stray file, or one written by a schema this binary cannot read —
        // never a half-written save, which temp-sibling-and-rename rules out.
        std::fs::write(root.join("zzz-corrupt.json"), "{not json").unwrap();

        let items = store.items().unwrap();
        assert_eq!(items.len(), 1, "the listing shows what it can");

        let err = store.items_strict().unwrap_err();
        assert!(
            format!("{err:#}").contains("zzz-corrupt.json"),
            "the error names the file that failed: {err:#}"
        );

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
                Provenance {
                    filled_defaults: Vec::new(),
                    session_id: None,
                    workspace: None,
                    call_id: None,
                },
            )
            .unwrap();
        store
            .stage(
                "t",
                OutboxKind::Message,
                json!({}),
                Taint::default(),
                Provenance {
                    filled_defaults: Vec::new(),
                    session_id: None,
                    workspace: None,
                    call_id: None,
                },
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
                Provenance {
                    filled_defaults: Vec::new(),
                    session_id: None,
                    workspace: None,
                    call_id: None,
                },
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
                    Provenance {
                        filled_defaults: Vec::new(),
                        session_id: None,
                        workspace: None,
                        call_id: None,
                    },
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

    /// The signal that had no reader: the owner read a letter written in their
    /// name and sent it as drafted. Positive evidence, and deliberately *not*
    /// a correction — mining it as one would teach voice rules from approval.
    #[test]
    fn a_draft_sent_as_written_is_positive_evidence_and_never_a_correction() {
        let root = scratch("writing-outcome");
        let store = OutboxStore::open(&root).unwrap();
        let stage = |kind| {
            store
                .stage(
                    "x__send",
                    kind,
                    json!({"body": "Dear Dirk,"}),
                    Taint::default(),
                    Provenance {
                        filled_defaults: Vec::new(),
                        session_id: None,
                        workspace: None,
                        call_id: None,
                    },
                )
                .unwrap()
        };

        let mut unchanged = stage(OutboxKind::Message);
        unchanged.status = "sent".into();
        assert_eq!(
            unchanged.writing_outcome(),
            Some(WritingOutcome::SentUnchanged)
        );
        assert!(
            !unchanged.mineable_as_writing(),
            "approval is not a correction"
        );

        let mut edited = stage(OutboxKind::Message);
        edited.status = "sent".into();
        edited.args = json!({"body": "Dear Dr Baumgartner,"});
        assert_eq!(edited.writing_outcome(), Some(WritingOutcome::SentEdited));
        assert!(edited.mineable_as_writing());

        // Says nothing about drafting: undecided, never went out, or not prose.
        let pending = stage(OutboxKind::Message);
        assert_eq!(pending.writing_outcome(), None);

        let mut rejected = stage(OutboxKind::Message);
        rejected.status = "rejected".into();
        assert_eq!(rejected.writing_outcome(), None);

        let mut published = stage(OutboxKind::Publish);
        published.status = "sent".into();
        assert_eq!(published.writing_outcome(), None);

        let tally = WritingTally::of([&unchanged, &edited, &pending, &rejected, &published]);
        assert_eq!(
            tally,
            WritingTally {
                unchanged: 1,
                edited: 1
            }
        );
        assert_eq!(tally.unchanged_rate(), Some(0.5));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// "Nothing was edited" and "nothing has been sent" are opposite findings.
    /// Reporting the second as 0% would describe an outbox nobody has used as
    /// one whose every draft was rewritten — the null-run bug, arriving in the
    /// one measure here whose job is to say something went well.
    #[test]
    fn a_rate_over_nothing_sent_is_absent_rather_than_zero() {
        assert_eq!(WritingTally::default().unchanged_rate(), None);
        assert_eq!(WritingTally::default().sent(), 0);
        assert_eq!(
            WritingTally {
                unchanged: 0,
                edited: 3
            }
            .unchanged_rate(),
            Some(0.0),
            "every draft rewritten is a real zero, and is not the same finding"
        );
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
                Provenance {
                    filled_defaults: Vec::new(),
                    session_id: None,
                    workspace: Some(jail.clone()),
                    call_id: None,
                },
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
                Provenance {
                    filled_defaults: Vec::new(),
                    session_id: None,
                    workspace: None,
                    call_id: None,
                },
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
                Provenance {
                    filled_defaults: Vec::new(),
                    session_id: None,
                    workspace: None,
                    call_id: None,
                },
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
                Provenance {
                    filled_defaults: Vec::new(),
                    session_id: None,
                    workspace: None,
                    call_id: None,
                },
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

    /// The invariant that separates reshaping a draft from summarising one:
    /// every argument reaches the reviewer somewhere. A field that falls
    /// between the three buckets is a field released unread.
    #[test]
    fn a_draft_view_drops_no_argument() {
        let args = json!({
            "to": ["a@x.org", "b@x.org"],
            "subject": "Tuesday?",
            "body_markdown": "Dear A,\n\nHello.\n\nLuke",
            "account": "dartmouth",
            "importance": "high",
            "attachments": [{"name": "f.pdf"}],
        });
        let view = DraftView::of(&args);
        let mut seen: Vec<String> = view
            .headers
            .iter()
            .map(|(k, _)| k.clone())
            .chain(view.body_field.clone())
            .chain(view.other.iter().map(|(k, _)| k.clone()))
            .collect();
        seen.sort();
        let mut keys: Vec<String> = args.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(seen, keys);
        assert_eq!(view.body.as_deref(), Some("Dear A,\n\nHello.\n\nLuke"));
        // Reading order, not map order.
        assert_eq!(
            view.headers
                .iter()
                .map(|(k, _)| k.as_str())
                .collect::<Vec<_>>(),
            ["to", "subject", "account"]
        );
        assert_eq!(view.headers[0].1, "a@x.org, b@x.org");
    }

    /// The spoken form carries the same guarantee, and it is the one that
    /// matters most: a listener cannot skim back over the line where the
    /// extra recipient was. Every argument's value must be *audible* — the
    /// check is on values rather than keys, because the body is spoken with
    /// no label and a key-only check would pass on a draft that read out its
    /// field names and none of its content.
    #[test]
    fn a_spoken_draft_utters_every_argument() {
        let args = json!({
            "to": ["a@x.org", "b@x.org"],
            "subject": "Tuesday?",
            "body_markdown": "Dear A,\n\nHello.\n\nLuke",
            "account": "dartmouth",
            "importance": "high",
        });
        let spoken = DraftView::of(&args).spoken().text();
        for audible in [
            "a@x.org",
            "b@x.org",
            "Tuesday?",
            "Dear A,",
            "Luke",
            "dartmouth",
            "high",
        ] {
            assert!(
                spoken.contains(audible),
                "{audible} was never said: {spoken}"
            );
        }
        // Labels are spoken as words, and the body carries none — "Body
        // markdown:" is a field name a listener has to parse past.
        assert!(spoken.contains("Subject: Tuesday?."), "{spoken}");
        assert!(!spoken.contains("Body markdown"), "{spoken}");
        assert!(spoken.contains("Importance: high."), "{spoken}");
    }

    /// A calendar draft is mostly timestamps, and a timestamp read out as
    /// digits is a draft nobody can check. Rendered in the offset the string
    /// carries — hearing a different moment than the draft names is the
    /// wrong-bytes review arriving through the ear.
    #[test]
    fn a_timestamp_is_spoken_as_a_time_in_its_own_offset() {
        let spoken = DraftView::of(&json!({
            "title": "Walk with Sage",
            "start_time": "2026-08-28T14:00:00-04:00",
            "end_time": "2026-08-28T14:30:00-04:00",
        }))
        .spoken()
        .text();
        assert!(spoken.contains("Friday August 28 at 2 PM"), "{spoken}");
        assert!(spoken.contains("Friday August 28 at 2:30 PM"), "{spoken}");
        assert!(!spoken.contains("T14:00"), "{spoken}");
        // A calendar tool that puts the zone in its own argument sends a
        // *naive* datetime, which is the form this missed on the first pass:
        // it fell through to the fallback and read out "2026-08-28T16:00:00".
        let naive = DraftView::of(&json!({
            "start_time": "2026-08-28T16:00:00",
            "timezone": "America/New_York",
        }))
        .spoken()
        .text();
        assert!(naive.contains("Friday August 28 at 4 PM"), "{naive}");
        // Start before end, whatever order the map hands them back in.
        let start = spoken.find("Start time").expect("start");
        let end = spoken.find("End time").expect("end");
        assert!(start < end, "an event read end-first is nonsense: {spoken}");
    }

    /// A value the renderer does not understand reaches the listener as
    /// itself. Guessing at it would be the one thing a spoken review cannot
    /// afford.
    #[test]
    fn an_unparseable_value_is_spoken_unchanged() {
        let spoken = DraftView::of(&json!({"when": "sometime next week"}))
            .spoken()
            .text();
        assert_eq!(spoken, "When: sometime next week.");
    }

    /// A tool nobody anticipated is still speakable, stiffly and completely.
    /// The alternative — a lookup table of nice phrasings — covers the tools
    /// thought of today and mis-speaks the rest.
    #[test]
    fn an_unanticipated_argument_is_spoken_stiffly_not_silently() {
        let spoken = DraftView::of(&json!({"emoji": "wave", "ts": 17}))
            .spoken()
            .text();
        assert_eq!(spoken, "Emoji: wave. Ts: 17.");
    }

    /// A tool nobody anticipated is still reviewable: no headers, no body, and
    /// every argument shown.
    #[test]
    fn an_unrecognised_draft_shows_everything_as_other() {
        let view = DraftView::of(&json!({"emoji": "wave", "ts": 17}));
        assert!(view.headers.is_empty() && view.body.is_none());
        assert_eq!(
            view.other,
            vec![
                ("emoji".to_string(), "wave".to_string()),
                ("ts".to_string(), "17".to_string())
            ]
        );
    }

    /// A blank value is shown as blank-on-purpose. The alternative is a label
    /// with nothing after it, which reads as a broken display rather than as
    /// an empty recipient list.
    #[test]
    fn an_empty_argument_says_so() {
        let view = DraftView::of(&json!({"to": "", "body": "hi"}));
        assert_eq!(
            view.headers,
            vec![("to".to_string(), "(empty)".to_string())]
        );
    }

    /// An edit writes back to the key the body came from, and to nothing else.
    #[test]
    fn an_edited_body_returns_to_its_own_field() {
        let args = json!({"thread_id": "t1", "body_markdown": "old", "account": "personal"});
        let edited = with_body(&args, "new").unwrap();
        assert_eq!(edited["body_markdown"], "new");
        assert_eq!(edited["thread_id"], "t1");
        assert_eq!(edited["account"], "personal");
        // No prose, no body edit — the caller must fall back rather than
        // silently save nothing.
        assert!(with_body(&json!({"event_id": "e1", "response": "accept"}), "x").is_none());
    }

    #[test]
    fn items_counting_says_how_many_files_the_lenient_read_skipped() {
        let dir = std::env::temp_dir().join(format!("outbox-count-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("20260901T000000-bad.json"), "{not json").unwrap();
        let store = OutboxStore::open(&dir).unwrap();
        let (items, skipped) = store.items_counting().unwrap();
        assert!(items.is_empty());
        assert_eq!(skipped, 1);
        assert!(store.items_strict().is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
