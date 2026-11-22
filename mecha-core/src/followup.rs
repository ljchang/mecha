//! Follow-ups a run proposed, kept until the owner accepts or rejects them.
//!
//! **The workflow store's front step, and the reason it is a store of its
//! own.** A follow-up — "check back on this tomorrow at nine" — is a
//! commitment with a date, and `workflow.rs` already holds those, raises
//! their reminders, and shows them on Home and in Tasks. But no model tool
//! mutates the workflow store (its module rule), and
//! `ASSISTANT-FOLLOW-THROUGH-DESIGN.md` decision 6 says the same thing from
//! the other side: **model extractions are proposals until accepted.** So the
//! model writes here, and only the owner's `accept` writes a workflow.
//!
//! The shape is the question store's, one store over, for the same reasons:
//! one pretty JSON per item, temp-sibling-and-rename, an advisory flock for
//! writers, and nothing deleted except by `forget` — a rejected proposal is
//! the record of what the owner declined.
//!
//! - **The session is required.** A follow-up returns to the conversation
//!   that proposed it; one with no recorded conversation has nowhere to go,
//!   so the tool refuses it rather than storing a reminder about nothing.
//! - **The taint is recorded**, fail-closed: a proposal made by a run that
//!   had read third-party text may be an injection's way to schedule itself,
//!   so `show` says so and nothing accepts it on the owner's behalf.
//! - **The words are the model's**, bounded to one line and
//!   [`WHAT_MAX_CHARS`], because they become a workflow's title on surfaces
//!   that lay out one row per item.
//! - **Who accepted is recorded** ([`Acceptance`]): the owner's own hand, or
//!   the owner's standing rule — a web chat's **auto**, the TUI's `/review
//!   auto` (design F7) — applied by the harness when the proposing run
//!   ended, never by a model. The record and its workflow both say which, so
//!   no later reader credits the owner's click with what a standing rule
//!   took.

use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::agent::Taint;

/// `proposed` | `accepted` | `rejected`.
pub const PROPOSED: &str = "proposed";
pub const ACCEPTED: &str = "accepted";
pub const REJECTED: &str = "rejected";

/// How long the words of one follow-up may be.
pub const WHAT_MAX_CHARS: usize = 200;
/// The nearest a follow-up may be scheduled. Anything sooner is the run
/// still working, not a follow-up.
pub const MIN_LEAD: chrono::Duration = chrono::Duration::minutes(10);
/// The furthest. A follow-up past this is a project, which belongs on the
/// board, not in a reminder.
pub const MAX_LEAD: chrono::Duration = chrono::Duration::days(90);

/// How many undecided proposals one conversation may hold. Each is bounded
/// by [`WHAT_MAX_CHARS`]; this bounds their number, so a conversation under
/// an injection cannot fill the owner's queue, and the cap is said to the
/// model rather than enforced by silence.
pub const MAX_OPEN_PER_SESSION: usize = 5;

/// One proposed follow-up.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FollowUp {
    pub id: String,
    pub status: String,
    /// What to check, in the model's words — one line.
    pub what: String,
    /// When to follow up.
    pub due_at: DateTime<Utc>,
    /// The conversation it came out of, and returns to.
    pub session_id: String,
    /// The jail the proposing run was held to.
    #[serde(default)]
    pub workspace: Option<PathBuf>,
    /// The conversation's taint when it was proposed. The tool records an
    /// unstamped taint as full taint; a record without the field — written
    /// by hand, or by a binary before it — is unknown, and [`armed`] reads
    /// unknown as untrusted.
    ///
    /// [`armed`]: FollowUp::armed
    #[serde(default)]
    pub taint: Option<Taint>,
    pub proposed_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<DateTime<Utc>>,
    /// The workflow an accept created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    /// Why the owner rejected it, in their words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// How an accepted proposal was accepted. Absent on one still proposed
    /// or rejected, and on one accepted before the field existed — when the
    /// owner's own `accept` was the only way — which a reader shows as
    /// unrecorded rather than guess.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_by: Option<Acceptance>,
}

/// Who accepted a follow-up (design F7).
///
/// A closed enum on an append-only record, so a wire format: a word this
/// build cannot name loads as [`Acceptance::Unknown`], never as either of
/// the two it knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Acceptance {
    /// The owner's own hand: `mecha followups accept`, which the proposals
    /// pane's Accept drives.
    Owner,
    /// The owner's standing rule, applied by the harness when the proposing
    /// run ended: a web chat's **auto** switched on, or the TUI's `/review
    /// auto`, and the proposal's recorded taint clean
    /// (`review_policy::auto_accepts`). The owner's word, given before the
    /// proposal existed — not a decision on this one.
    Auto,
    #[serde(other)]
    Unknown,
}

impl Acceptance {
    pub fn as_str(self) -> &'static str {
        match self {
            Acceptance::Owner => "owner",
            Acceptance::Auto => "auto",
            Acceptance::Unknown => "unknown",
        }
    }
}

impl FollowUp {
    pub fn is_proposed(&self) -> bool {
        self.status == PROPOSED
    }

    /// Whether the proposing conversation had read anything from outside —
    /// what `show` warns about, and what keeps a proposal from ever being
    /// accepted on the owner's behalf. Unknown is untrusted.
    pub fn armed(&self) -> bool {
        self.taint.is_none_or(|t| t.untrusted)
    }
}

/// Check and normalise what a model asked for: one line, bounded, and a time
/// inside [`MIN_LEAD`]..=[`MAX_LEAD`] from `now`. The error is a sentence the
/// model can act on.
pub fn admit(
    what: &str,
    due_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> std::result::Result<String, String> {
    let what = what.split_whitespace().collect::<Vec<_>>().join(" ");
    // One line, and only characters that render as themselves: `what` is
    // printed at the owner's decision point beside the taint warning, so an
    // escape sequence or a bidi override in it could rewrite the warning it
    // was meant to carry (`title::is_format_char`, `hud::spec`). Refused,
    // not filtered, so the model is told.
    if what
        .chars()
        .any(|c| c.is_control() || crate::title::is_format_char(c))
    {
        return Err("keep `what` to printable text: no control or formatting characters".into());
    }
    if what.is_empty() {
        return Err("say what to follow up on, in one line".into());
    }
    if what.chars().count() > WHAT_MAX_CHARS {
        return Err(format!(
            "keep `what` to {WHAT_MAX_CHARS} characters: it is the reminder's title"
        ));
    }
    if due_at < now + MIN_LEAD {
        return Err(format!(
            "a follow-up must be at least {} minutes from now; sooner is still this conversation",
            MIN_LEAD.num_minutes()
        ));
    }
    if due_at > now + MAX_LEAD {
        return Err(format!(
            "a follow-up must be within {} days; anything later belongs on the task board",
            MAX_LEAD.num_days()
        ));
    }
    Ok(what)
}

pub struct FollowUpStore {
    root: PathBuf,
}

/// Holds the store's writer lock for as long as it lives.
pub struct FollowUpLock {
    _file: std::fs::File,
}

impl FollowUpStore {
    pub fn default_root() -> Result<PathBuf> {
        if let Ok(dir) = std::env::var("MECHA_FOLLOWUPS_DIR") {
            return Ok(PathBuf::from(dir));
        }
        Ok(crate::work::mecha_home()?.join("followups"))
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        crate::create_private_dir(&root).with_context(|| format!("creating {}", root.display()))?;
        Ok(FollowUpStore { root })
    }

    /// A handle on `root` that creates nothing — for a registry that is
    /// never executed (`setup::surface_only_registry`).
    pub fn open_unchecked(root: PathBuf) -> Self {
        FollowUpStore { root }
    }

    pub fn open_default() -> Result<Self> {
        Self::open(Self::default_root()?)
    }

    /// Open at the default location only if it already exists — for read
    /// paths that must not create state as a side effect.
    pub fn open_existing_default() -> Option<Self> {
        let root = Self::default_root().ok()?;
        root.is_dir().then_some(FollowUpStore { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, id: &str) -> Result<PathBuf> {
        ensure!(
            !id.is_empty()
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "invalid follow-up id"
        );
        Ok(self.root.join(format!("{id}.json")))
    }

    /// Take the writer lock. Held across a read-modify-write, never across a
    /// run.
    pub fn lock(&self) -> Result<FollowUpLock> {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::io::AsRawFd;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(self.root.join(".lock"))?;
        // SAFETY: flock on an fd we own, held open by the returned guard.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error()).context("locking the follow-up store");
        }
        Ok(FollowUpLock { _file: file })
    }

    /// Every follow-up, newest first, and how many records could not be
    /// read. An unreadable record is counted, never dropped silently: a
    /// reader that claims the store was read must be able to say it was not
    /// read whole.
    pub fn items_counting(&self) -> Result<(Vec<FollowUp>, usize)> {
        let mut out = Vec::new();
        let mut skipped = 0usize;
        let dir = match std::fs::read_dir(&self.root) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((out, 0)),
            Err(e) => return Err(e).context("reading the follow-up store"),
        };
        for entry in dir {
            let path = match entry {
                Ok(entry) => entry.path(),
                Err(e) => {
                    skipped += 1;
                    tracing::warn!("skipping an unreadable follow-up entry: {e}");
                    continue;
                }
            };
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str::<FollowUp>(&t).ok())
            {
                Some(f) => out.push(f),
                None => {
                    skipped += 1;
                    tracing::warn!(path = %path.display(), "skipping an unreadable follow-up");
                }
            }
        }
        out.sort_by(|a, b| b.proposed_at.cmp(&a.proposed_at).then(b.id.cmp(&a.id)));
        Ok((out, skipped))
    }

    pub fn items(&self) -> Result<Vec<FollowUp>> {
        self.items_counting().map(|(items, _)| items)
    }

    pub fn get(&self, id: &str) -> Result<FollowUp> {
        let text = std::fs::read_to_string(self.path(id)?)
            .with_context(|| format!("no such follow-up: {id}"))?;
        let f: FollowUp =
            serde_json::from_str(&text).with_context(|| format!("unreadable follow-up: {id}"))?;
        ensure!(f.id == id, "follow-up id does not match its file name");
        Ok(f)
    }

    /// Resolve an abbreviated id — head or tail — and refuse an ambiguous one
    /// rather than guessing, as the question store does.
    pub fn find(&self, needle: &str) -> Result<FollowUp> {
        // A record that is there and cannot be read is said as that, never
        // as "no such follow-up": distress is not absence (review of #896).
        match self.get(needle) {
            Ok(f) => return Ok(f),
            Err(e) if self.path(needle).is_ok_and(|p| p.exists()) => return Err(e),
            Err(_) => {}
        }
        let matches: Vec<FollowUp> = self
            .items()?
            .into_iter()
            .filter(|f| f.id.starts_with(needle) || f.id.ends_with(needle))
            .collect();
        match matches.len() {
            0 => anyhow::bail!("no such follow-up: {needle}"),
            1 => Ok(matches.into_iter().next().expect("just checked")),
            n => anyhow::bail!("{needle} matches {n} follow-ups — use more of the id"),
        }
    }

    pub fn put(&self, f: &FollowUp) -> Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let path = self.path(&f.id)?;
        let tmp = path.with_extension("json.tmp");
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(serde_json::to_string_pretty(f)?.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Record a proposal and hand back what was stored. The caller has
    /// already [`admit`]ted the words and the time.
    pub fn propose(
        &self,
        what: String,
        due_at: DateTime<Utc>,
        session_id: &str,
        workspace: Option<PathBuf>,
        taint: Taint,
        now: DateTime<Utc>,
    ) -> Result<FollowUp> {
        ensure!(!session_id.is_empty(), "a follow-up needs its conversation");
        let f = FollowUp {
            id: crate::session::Session::new_id(),
            status: PROPOSED.into(),
            what,
            due_at,
            session_id: session_id.into(),
            workspace,
            taint: Some(taint),
            proposed_at: now,
            decided_at: None,
            workflow_id: None,
            reason: None,
            accepted_by: None,
        };
        let _lock = self.lock()?;
        // Counted under the lock, so two calls in one batch cannot both
        // squeeze under the cap. A store that cannot be read whole refuses
        // with its own reason — unknown is never "fewer than five", and it
        // is not this conversation's to fix (review of #896).
        let (items, skipped) = self.items_counting()?;
        ensure!(
            skipped == 0,
            "the follow-up store could not be read whole ({skipped} unreadable record(s) under {})",
            self.root.display()
        );
        let open = items
            .iter()
            .filter(|o| o.session_id == f.session_id && o.is_proposed())
            .count();
        ensure!(
            open < MAX_OPEN_PER_SESSION,
            "this conversation already has {open} follow-ups waiting on the user's answer \
             (at most {MAX_OPEN_PER_SESSION}); mention it instead of proposing another"
        );
        ensure!(!self.path(&f.id)?.try_exists()?, "follow-up id collision");
        self.put(&f)?;
        Ok(f)
    }

    /// Decide a proposal, under the lock: `f` sees the current record and
    /// may fail, in which case nothing is written. Refuses a follow-up that
    /// is no longer proposed — a decision is made once.
    pub fn decide(
        &self,
        id: &str,
        status: &str,
        now: DateTime<Utc>,
        f: impl FnOnce(&mut FollowUp) -> Result<()>,
    ) -> Result<FollowUp> {
        ensure!(
            status == ACCEPTED || status == REJECTED,
            "a follow-up is accepted or rejected"
        );
        let _lock = self.lock()?;
        let mut item = self.get(id)?;
        ensure!(
            item.is_proposed(),
            "follow-up {} was already {}",
            short(&item.id),
            item.status
        );
        f(&mut item)?;
        item.status = status.into();
        item.decided_at = Some(now);
        self.put(&item)?;
        Ok(item)
    }
}

/// The harness's turn that wakes a conversation for its follow-up. A stem of
/// its own, so `agent::is_harness_voice` keeps it out of correction mining
/// and no reader takes it for the owner's words.
pub const WAKE_STEM: &str = "(From the harness: a follow-up you proposed is due";

/// The turn itself: what the conversation proposed, quoted, and what to do.
/// The quoted words are the model's own from this conversation and carry its
/// taint; the turn adds none and launders none.
pub fn render_wake(what: &str, proposed_at: DateTime<Utc>, tz: chrono_tz::Tz) -> String {
    format!(
        "{WAKE_STEM} — \"{what}\", proposed {}. Check it now and tell the owner what you \
         found. Nothing here is the owner's request; anything you would send is staged for \
         their review.)",
        proposed_at.with_timezone(&tz).format("%a %b %-d, %H:%M %Z")
    )
}

/// The words a wake quotes for `w`: the commitment's expectation (the
/// proposal's own words), or the workflow's title on a record without one.
/// One definition, because the waker writes them and the acted-on measure
/// finds the woken turn by them.
pub fn wake_words(w: &crate::workflow::Workflow) -> &str {
    w.commitment
        .as_ref()
        .and_then(|c| c.expectation.as_deref())
        .unwrap_or(&w.title)
}

/// The proposal an accepted follow-up's workflow came from, by its
/// `commitment.source`. `None` for every other workflow.
pub fn proposal_of(w: &crate::workflow::Workflow) -> Option<&str> {
    w.commitment
        .as_ref()?
        .source
        .strip_prefix(WORKFLOW_SOURCE_PREFIX)
}

/// Whether the web host should wake `w`'s conversation now: a follow-up's
/// workflow, open, its time come, not snoozed, not running, never woken.
pub fn due_to_wake(w: &crate::workflow::Workflow, now: DateTime<Utc>) -> bool {
    proposal_of(w).is_some()
        && w.closed_at.is_none()
        && w.woken_at.is_none()
        && w.state != "running"
        && w.session_id.is_some()
        && !w.snoozed_until.is_some_and(|t| t > now)
        && w.commitment.as_ref().is_some_and(|c| c.follow_up_due(now))
}

/// What an accepted follow-up's workflow names it by, in
/// `commitment.source` — the key `forget` finds that workflow with.
pub const WORKFLOW_SOURCE_PREFIX: &str = "followup:";

/// The distinguishing tail of an id, for display — the question store's
/// reasoning: the head of a `Session::new_id` is a timestamp, and every
/// follow-up proposed on one day shares it.
pub fn short(id: &str) -> &str {
    id.rsplit_once('-').map(|(_, tail)| tail).unwrap_or(id)
}

/// The workflow an accepted follow-up becomes: the conversation it returns
/// to, the jail it ran in, and a commitment dated by the proposal. No
/// deadline: a follow-up is a time to look again, not a promise to a party,
/// so it is never overdue (`Commitment::overdue`) and only raises the
/// "Follow-up due" reminder. `by` is who accepted it, kept on the workflow
/// and said in its `created` event, so the acted-on measure and any later
/// audit can tell a standing rule's acceptance from a click.
pub fn workflow_for(
    f: &FollowUp,
    workspace: PathBuf,
    outbox_root: Option<PathBuf>,
    by: Acceptance,
    now: DateTime<Utc>,
) -> crate::workflow::Workflow {
    let mut w = crate::workflow::Workflow::new(
        format!("followup-{}", f.id),
        format!("Follow-up: {}", f.what),
        workspace,
        now,
    );
    w.session_id = Some(f.session_id.clone());
    w.outbox_root = outbox_root;
    w.commitment = Some(crate::workflow::Commitment {
        party: "owner".into(),
        source: format!("{WORKFLOW_SOURCE_PREFIX}{}", f.id),
        due_at: None,
        follow_up_at: Some(f.due_at),
        expectation: Some(f.what.clone()),
        consequence: None,
    });
    w.accepted_by = Some(by);
    let detail = match by {
        Acceptance::Owner => format!("Owner accepted follow-up {}", f.id),
        Acceptance::Auto => format!(
            "Follow-up {} accepted automatically: auto was on in its conversation (the \
             owner's standing rule) and it had read no outside content",
            f.id
        ),
        Acceptance::Unknown => format!("Follow-up {} accepted by an unrecorded hand", f.id),
    };
    w.record("created", detail, now);
    w
}

// ---------------------------------------------------------------------------
// The acted-on measure (design §Step 4)
// ---------------------------------------------------------------------------
//
// **A sensor: it counts and never judges, and nothing reads it yet** (the
// owner's ruling for proactive messages, 2026-11-17: sensor first). For each
// follow-up whose woken turn was answered, it reads from records only what
// the owner did in the day after: replied in the chat, released or declined
// a draft staged there, or dismissed the reminder. **Whether the owner
// opened the chat is not measured** — nothing server-side records a page
// being looked at, and a measure that inferred it would be inventing the
// one signal it cannot see. Unknown is never none: a source that cannot be
// read, a reply that cannot be placed in time, or a draft resolved by a hand
// that is not provably the owner's leaves the follow-up `unknown`, outside
// both sides of the rate.

/// How long after a woken answer the owner's response is looked for.
pub const ACTED_ON_WINDOW: chrono::Duration = chrono::Duration::hours(24);

/// What the owner did about one woken answer, as the records show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Response {
    /// Their own words in the chat after the woken turn began.
    Replied,
    /// A draft staged in the chat since the woken turn began, released by
    /// the owner's own hand.
    Released,
    /// Such a draft, declined by the owner's own hand — an act, by declining.
    Rejected,
    /// The workflow's reminder dismissed by the owner.
    Dismissed,
    /// None of these, and the window has closed.
    #[serde(rename = "none")]
    NoResponse,
    /// None yet, and the window is still open.
    Pending,
    /// A record that would decide it could not be read or attributed.
    Unknown,
}

impl Response {
    pub fn as_str(self) -> &'static str {
        match self {
            Response::Replied => "replied",
            Response::Released => "released",
            Response::Rejected => "rejected",
            Response::Dismissed => "dismissed",
            Response::NoResponse => "none",
            Response::Pending => "pending",
            Response::Unknown => "unknown",
        }
    }

    /// `Some(true)` for an act, `Some(false)` for none, and `None` for the
    /// two that are not decided — pending and unknown sit on neither side
    /// of the rate.
    pub fn acted_on(self) -> Option<bool> {
        match self {
            Response::Replied | Response::Released | Response::Rejected | Response::Dismissed => {
                Some(true)
            }
            Response::NoResponse => Some(false),
            Response::Pending | Response::Unknown => None,
        }
    }
}

/// One record of the owner acting, found inside the window. A follow-up
/// may carry several; its [`Response`] is the first in this order.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Signal {
    Replied,
    Released { outbox_id: String },
    Rejected { outbox_id: String },
    Dismissed { at: DateTime<Utc> },
}

/// What the measure reads of a transcript: the messages as loaded, and each
/// recorded run's end placed among them — the only clock a transcript keeps
/// (`RunStats::ended_at`), since a message carries no time of its own.
pub struct Timeline<'a> {
    messages: &'a [crate::message::Message],
    /// `(message count when the run's outcome was written, when)`, for every
    /// outcome that has both.
    run_ends: Vec<(usize, DateTime<Utc>)>,
}

impl<'a> Timeline<'a> {
    pub fn new(
        messages: &'a [crate::message::Message],
        run_ends: Vec<(usize, DateTime<Utc>)>,
    ) -> Self {
        Timeline { messages, run_ends }
    }

    /// A transcript read whole. An outcome a summarising compaction
    /// displaced (`outcome_positions` `None`), or one from before
    /// `ended_at`, places nothing and is left out.
    pub fn of(t: &'a crate::session::Transcript) -> Self {
        let run_ends = t
            .outcome_positions
            .iter()
            .zip(&t.outcomes)
            .filter_map(|(pos, o)| Some(((*pos)?, o.ended_at?)))
            .collect();
        Timeline::new(&t.convo.messages, run_ends)
    }

    /// The latest time message `i` is known to come after: the end of the
    /// last run recorded before it was appended.
    fn after(&self, i: usize) -> Option<DateTime<Utc>> {
        self.run_ends
            .iter()
            .filter(|(pos, _)| *pos <= i)
            .map(|(_, at)| *at)
            .max()
    }

    /// The earliest time message `i` is known to come before: the end of
    /// the first run recorded with it in the file, or `read_at` — it was
    /// there when the transcript was read.
    fn before(&self, i: usize, read_at: DateTime<Utc>) -> DateTime<Utc> {
        self.run_ends
            .iter()
            .filter(|(pos, _)| *pos > i)
            .map(|(_, at)| *at)
            .min()
            .unwrap_or(read_at)
            .min(read_at)
    }
}

/// One woken-and-answered follow-up, measured.
#[derive(Debug, Clone, Serialize)]
pub struct Measured {
    /// The proposal's id (`short` for display).
    pub proposal: String,
    pub workflow: String,
    pub session_id: String,
    /// When the woken turn was claimed — the window's start. `None` only on
    /// a record that holds an answer and no wake, which is `unknown`.
    pub woken_at: Option<DateTime<Utc>>,
    pub answered_at: DateTime<Utc>,
    /// `answered_at` + [`ACTED_ON_WINDOW`].
    pub window_closes: DateTime<Utc>,
    pub response: Response,
    pub signals: Vec<Signal>,
    /// Why it could not be decided, when that was so.
    pub unknown: Vec<String>,
    /// Who accepted it ([`Acceptance`]), as its workflow records it, so a
    /// reader can keep the follow-ups the owner clicked apart from those a
    /// standing auto rule took. `None` on a workflow from before the field.
    pub accepted_by: Option<Acceptance>,
}

/// The latest `follow_up_answered` on a follow-up's workflow — the answer
/// the measure is about. `None` for any other workflow.
pub fn answered_at(w: &crate::workflow::Workflow) -> Option<DateTime<Utc>> {
    proposal_of(w)?;
    w.events
        .iter()
        .rev()
        .find(|e| e.kind == crate::workflow::FOLLOW_UP_ANSWERED)
        .map(|e| e.at)
}

/// Measure one workflow from records already read: its transcript (or why
/// it could not be read) and the outbox items of its store (likewise).
/// `None` for a workflow that is not a woken-and-answered follow-up's.
///
/// The window runs from the woken turn's claim to a day after its answer,
/// for every signal alike. A reply is the owner's own words
/// (`agent::owner_text`) in a user message after the wake — a steer typed
/// into the woken run counts, a harness turn or a later wake does not —
/// placed in time by the run ends around it, and counted only when it is
/// provably inside: a reply that cannot be placed either side of the close
/// is `unknown`, never a reply and never none. A draft counts when it was
/// staged from the conversation since the wake and resolved inside the
/// window by [`Actor::Owner`](crate::closure::Actor::Owner); resolved by any
/// other hand, it is `unknown` (`forecast::observe`'s rule). A dismissal
/// counts when [`NOTICE_ACKED`](crate::workflow::NOTICE_ACKED) names the
/// owner.
pub fn measure(
    w: &crate::workflow::Workflow,
    transcript: std::result::Result<&Timeline<'_>, &str>,
    outbox: std::result::Result<&[crate::outbox::OutboxItem], &str>,
    now: DateTime<Utc>,
) -> Option<Measured> {
    use crate::closure::Actor;
    let proposal = proposal_of(w)?.to_string();
    let answered_at = answered_at(w)?;
    let closes = answered_at + ACTED_ON_WINDOW;
    let session_id = w.session_id.clone().unwrap_or_default();
    let woken_at = w.woken_at.or_else(|| {
        w.events
            .iter()
            .rev()
            .find(|e| e.kind == crate::workflow::FOLLOW_UP_WOKEN)
            .map(|e| e.at)
    });
    let mut signals = Vec::new();
    let mut unknown = Vec::new();
    let inside = |t: DateTime<Utc>, from: DateTime<Utc>| t >= from && t <= closes;

    match woken_at {
        None => unknown.push("the workflow records an answer and no wake".to_string()),
        Some(from) => {
            match transcript {
                Err(why) => unknown.push(format!("the transcript could not be read: {why}")),
                Ok(t) => replied(
                    t,
                    wake_words(w),
                    from,
                    closes,
                    now,
                    &mut signals,
                    &mut unknown,
                ),
            }
            match outbox {
                Err(why) => unknown.push(format!("the outbox could not be read: {why}")),
                Ok(items) => {
                    for item in items
                        .iter()
                        .filter(|i| i.session_id.as_deref() == Some(session_id.as_str()))
                    {
                        let Some(staged) = rfc3339(&item.created_at) else {
                            unknown.push(format!("outbox item {} has no readable time", item.id));
                            continue;
                        };
                        if staged < from {
                            continue;
                        }
                        // A release the owner pressed whose delivery did not
                        // complete leaves the draft pending with no stamp:
                        // an attempt inside the window is a release by a hand
                        // nothing recorded — unknown, never none (review of
                        // #903).
                        if item.status == "pending" {
                            // As `forecast::observe` reads it: an error on a
                            // pending draft is a release that did not land —
                            // `release` records it before any attempt when
                            // the tool is missing — and carries no time, so
                            // it is unknown, never none (review of #903).
                            let attempted = item
                                .delivery_attempts
                                .iter()
                                .any(|a| rfc3339(&a.started_at).is_none_or(|at| inside(at, from)));
                            if attempted || item.error.is_some() {
                                unknown.push(format!(
                                    "outbox item {} was released and not delivered",
                                    item.id
                                ));
                            }
                            continue;
                        }
                        if !matches!(item.status.as_str(), "sent" | "rejected") {
                            continue;
                        }
                        let Some(resolved) = item.resolved_at.as_deref().and_then(rfc3339) else {
                            unknown.push(format!(
                                "outbox item {} was {} at no readable time",
                                item.id, item.status
                            ));
                            continue;
                        };
                        if !inside(resolved, from) {
                            continue;
                        }
                        let by = item.resolved_by();
                        if by != Actor::Owner {
                            unknown.push(format!(
                                "outbox item {} was {} by {}, not provably the owner",
                                item.id,
                                item.status,
                                by.as_str()
                            ));
                            continue;
                        }
                        let outbox_id = item.id.clone();
                        signals.push(if item.status == "sent" {
                            Signal::Released { outbox_id }
                        } else {
                            Signal::Rejected { outbox_id }
                        });
                    }
                }
            }
            for e in w
                .events
                .iter()
                .filter(|e| e.kind == crate::workflow::NOTICE_ACKED && inside(e.at, from))
            {
                if e.detail == Actor::Owner.as_str() {
                    signals.push(Signal::Dismissed { at: e.at });
                    break;
                }
                unknown.push(format!(
                    "the reminder was dismissed by {}, not provably the owner",
                    e.detail
                ));
            }
        }
    }

    let has = |f: fn(&Signal) -> bool| signals.iter().any(f);
    let response = if has(|s| matches!(s, Signal::Replied)) {
        Response::Replied
    } else if has(|s| matches!(s, Signal::Released { .. })) {
        Response::Released
    } else if has(|s| matches!(s, Signal::Rejected { .. })) {
        Response::Rejected
    } else if has(|s| matches!(s, Signal::Dismissed { .. })) {
        Response::Dismissed
    } else if !unknown.is_empty() {
        Response::Unknown
    } else if now <= closes {
        Response::Pending
    } else {
        Response::NoResponse
    };
    Some(Measured {
        proposal,
        workflow: w.id.clone(),
        session_id,
        woken_at,
        answered_at,
        window_closes: closes,
        response,
        signals,
        unknown,
        accepted_by: w.accepted_by,
    })
}

/// The reply half of [`measure`]: find the woken turn — the last wake
/// quoting `what` that no run is known to have ended after `from` before —
/// then the first of the owner's own messages after it, and place it.
fn replied(
    t: &Timeline<'_>,
    what: &str,
    from: DateTime<Utc>,
    closes: DateTime<Utc>,
    now: DateTime<Utc>,
    signals: &mut Vec<Signal>,
    unknown: &mut Vec<String>,
) {
    use crate::message::{Block, Role};
    let quoted = format!("\"{what}\"");
    let is_wake = |m: &crate::message::Message| {
        m.role == Role::User
            && m.content.iter().any(|b| {
                matches!(b, Block::Text { text }
                    if text.trim_start().starts_with(WAKE_STEM) && text.contains(&quoted))
            })
    };
    let Some(wake) = (0..t.messages.len())
        .rev()
        .find(|&i| is_wake(&t.messages[i]) && t.after(i).is_none_or(|at| at <= from))
    else {
        unknown.push("the woken turn is not in the transcript".into());
        return;
    };
    // From the wake itself: a steer typed before the woken run's first turn
    // is folded onto the wake message (`agent::fold_steer`), which then holds
    // the owner's words beside the harness's (review of #903).
    for j in wake..t.messages.len() {
        let m = &t.messages[j];
        if m.role != Role::User || crate::agent::owner_text(m).trim().is_empty() {
            continue;
        }
        if t.before(j, now) <= closes {
            signals.push(Signal::Replied);
        } else if t.after(j).is_none_or(|at| at <= closes) {
            // Somewhere around the close, and no run end says which side:
            // a later message is later still, so nothing after it decides.
            unknown.push("a reply could not be placed inside or outside the window".into());
        }
        // The owner's first words after the wake decide it: if they came
        // after the close, every later message did too.
        return;
    }
}

fn rfc3339(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// The readout's totals. The rate is acted-on over decided — replied,
/// released, rejected and dismissed over those and none — and `None` over
/// an empty denominator, never 0; pending and unknown are counted beside it
/// and sit on neither side.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Totals {
    pub replied: usize,
    pub released: usize,
    pub rejected: usize,
    pub dismissed: usize,
    pub none: usize,
    pub pending: usize,
    pub unknown: usize,
    pub acted_on: usize,
    pub decided: usize,
    pub acted_on_rate: Option<f64>,
}

impl Totals {
    pub fn of(rows: &[Measured]) -> Self {
        let mut t = Totals::default();
        for r in rows {
            *match r.response {
                Response::Replied => &mut t.replied,
                Response::Released => &mut t.released,
                Response::Rejected => &mut t.rejected,
                Response::Dismissed => &mut t.dismissed,
                Response::NoResponse => &mut t.none,
                Response::Pending => &mut t.pending,
                Response::Unknown => &mut t.unknown,
            } += 1;
            match r.response.acted_on() {
                Some(true) => {
                    t.acted_on += 1;
                    t.decided += 1;
                }
                Some(false) => t.decided += 1,
                None => {}
            }
        }
        t.acted_on_rate = (t.decided > 0).then(|| t.acted_on as f64 / t.decided as f64);
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        "2026-11-18T12:00:00Z".parse().unwrap()
    }

    /// A fresh store under the temp directory; removed when dropped.
    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store() -> (Scratch, FollowUpStore) {
        let dir = std::env::temp_dir().join(format!("followups-{}", uuid::Uuid::new_v4()));
        let s = FollowUpStore::open(dir.join("followups")).unwrap();
        (Scratch(dir), s)
    }

    #[test]
    fn admit_keeps_one_line_and_a_time_inside_the_window() {
        let n = now();
        assert_eq!(
            admit(
                "  check whether\nthe build\tpassed ",
                n + chrono::Duration::hours(2),
                n
            )
            .unwrap(),
            "check whether the build passed"
        );
        assert!(admit("   ", n + chrono::Duration::hours(2), n).is_err());
        assert!(admit("x", n + chrono::Duration::minutes(5), n)
            .unwrap_err()
            .contains("10 minutes"));
        assert!(admit("x", n + chrono::Duration::days(91), n)
            .unwrap_err()
            .contains("90 days"));
        // Nothing that rewrites the terminal or reorders what is shown.
        for hostile in [
            "ok\u{1b}[2K\u{1b}[Ahidden",
            "ok \u{202e}derever",
            "zero\u{200b}width",
        ] {
            assert!(
                admit(hostile, n + chrono::Duration::hours(2), n).is_err(),
                "{hostile:?} was admitted"
            );
        }
        let long = "a".repeat(WHAT_MAX_CHARS + 1);
        assert!(admit(&long, n + chrono::Duration::hours(2), n).is_err());
        // The bounds themselves are inside.
        assert!(admit("x", n + MIN_LEAD, n).is_ok());
        assert!(admit("x", n + MAX_LEAD, n).is_ok());
    }

    #[test]
    fn a_proposal_round_trips_and_is_decided_once() {
        let (_d, s) = store();
        let n = now();
        let f = s
            .propose(
                "check the build".into(),
                n + chrono::Duration::hours(3),
                "sess-a",
                Some("/tmp/w".into()),
                Taint::default(),
                n,
            )
            .unwrap();
        assert_eq!(s.find(short(&f.id)).unwrap(), f);
        let accepted = s
            .decide(&f.id, ACCEPTED, n, |f| {
                f.workflow_id = Some("followup-x".into());
                Ok(())
            })
            .unwrap();
        assert_eq!(accepted.status, ACCEPTED);
        assert_eq!(accepted.decided_at, Some(n));
        let again = s.decide(&f.id, REJECTED, n, |_| Ok(())).unwrap_err();
        assert!(
            format!("{again:#}").contains("already accepted"),
            "{again:#}"
        );
        assert_eq!(s.get(&f.id).unwrap().status, ACCEPTED);
    }

    #[test]
    fn a_failed_decision_writes_nothing() {
        let (_d, s) = store();
        let n = now();
        let f = s
            .propose(
                "x".into(),
                n + MIN_LEAD,
                "sess-a",
                None,
                Taint::default(),
                n,
            )
            .unwrap();
        assert!(s
            .decide(&f.id, ACCEPTED, n, |_| anyhow::bail!("workflow store full"))
            .is_err());
        assert!(s.get(&f.id).unwrap().is_proposed());
    }

    #[test]
    fn a_record_without_its_taint_reads_untrusted() {
        let f: FollowUp = serde_json::from_value(serde_json::json!({
            "id": "x", "status": "proposed", "what": "x",
            "due_at": "2026-11-19T00:00:00Z", "session_id": "s",
            "proposed_at": "2026-11-18T00:00:00Z"
        }))
        .unwrap();
        assert!(f.taint.is_none() && f.armed());
    }

    #[test]
    fn one_conversation_holds_at_most_a_few_undecided_proposals() {
        let (_d, s) = store();
        let n = now();
        let propose =
            |session: &str| s.propose("x".into(), n + MIN_LEAD, session, None, Taint::default(), n);
        let first = propose("sess-a").unwrap();
        for _ in 1..MAX_OPEN_PER_SESSION {
            propose("sess-a").unwrap();
        }
        assert!(format!("{:#}", propose("sess-a").unwrap_err()).contains("at most"));
        // Another conversation is not this one's cap.
        propose("sess-b").unwrap();
        // A decided one frees its place.
        s.decide(&first.id, REJECTED, n, |_| Ok(())).unwrap();
        propose("sess-a").unwrap();
    }

    #[test]
    fn a_follow_up_wakes_once_when_due_open_and_unsnoozed() {
        let n = now();
        let f = FollowUp {
            id: "20261118T120000-wake1234".into(),
            status: ACCEPTED.into(),
            what: "check the build".into(),
            due_at: n + chrono::Duration::hours(1),
            session_id: "sess-a".into(),
            workspace: None,
            taint: Some(Taint::default()),
            proposed_at: n,
            decided_at: Some(n),
            workflow_id: None,
            reason: None,
            accepted_by: None,
        };
        let mut w = workflow_for(&f, "/tmp/w".into(), None, Acceptance::Owner, n);
        let later = n + chrono::Duration::hours(2);
        assert_eq!(proposal_of(&w), Some(f.id.as_str()));
        assert!(!due_to_wake(&w, n), "not before its time");
        assert!(due_to_wake(&w, later));
        w.snoozed_until = Some(later + chrono::Duration::hours(1));
        assert!(!due_to_wake(&w, later), "a snooze holds it");
        w.snoozed_until = None;
        w.state = "running".into();
        assert!(!due_to_wake(&w, later), "not over a run in flight");
        w.state = "idle".into();
        w.woken_at = Some(later);
        assert!(!due_to_wake(&w, later), "woken once");
        w.woken_at = None;
        w.closed_at = Some(later);
        assert!(!due_to_wake(&w, later), "a closed workflow is done");
        // An owner's own workflow is never a follow-up's.
        let mut own = crate::workflow::Workflow::new("w".into(), "t".into(), "/tmp".into(), n);
        own.commitment = w.commitment.clone().map(|mut c| {
            c.source = "mail:thread-1".into();
            c
        });
        assert!(!due_to_wake(&own, later));
        // The turn is the harness's, quoting the model's words.
        let turn = render_wake(&f.what, n, chrono_tz::UTC);
        assert!(turn.starts_with(WAKE_STEM) && turn.contains("\"check the build\""));
        // And every reader that sees only text knows it is not the owner's.
        assert!(crate::agent::is_harness_voice(&turn));
    }

    #[test]
    fn a_corrupt_record_reads_unreadable_not_missing() {
        let (_d, s) = store();
        std::fs::write(s.root().join("20261118T120000-broken12.json"), "{").unwrap();
        let e = format!("{:#}", s.find("20261118T120000-broken12").unwrap_err());
        assert!(e.contains("unreadable"), "{e}");
        let e = format!("{:#}", s.find("nothing-here").unwrap_err());
        assert!(e.contains("no such follow-up"), "{e}");
    }

    #[test]
    fn a_proposal_needs_its_conversation() {
        let (_d, s) = store();
        let n = now();
        assert!(s
            .propose("x".into(), n + MIN_LEAD, "", None, Taint::default(), n)
            .is_err());
    }

    #[test]
    fn an_unreadable_record_is_counted_not_dropped() {
        let (_d, s) = store();
        let n = now();
        s.propose(
            "x".into(),
            n + MIN_LEAD,
            "sess-a",
            None,
            Taint::default(),
            n,
        )
        .unwrap();
        std::fs::write(s.root().join("broken.json"), "{").unwrap();
        let (items, skipped) = s.items_counting().unwrap();
        assert_eq!((items.len(), skipped), (1, 1));
    }

    // --- the acted-on measure ---

    use crate::closure::Actor;
    use crate::message::{Block, Message};
    use crate::workflow::{Workflow, FOLLOW_UP_ANSWERED, NOTICE_ACKED};

    fn h(n: i64) -> chrono::Duration {
        chrono::Duration::hours(n)
    }

    /// A follow-up's workflow woken at `now()` and answered ten minutes on.
    fn answered_workflow(what: &str) -> Workflow {
        let n = now();
        let f = FollowUp {
            id: "20261118T090000-meas1234".into(),
            status: ACCEPTED.into(),
            what: what.into(),
            due_at: n,
            session_id: "sess-a".into(),
            workspace: None,
            taint: Some(Taint::default()),
            proposed_at: n - h(3),
            decided_at: Some(n - h(3)),
            workflow_id: None,
            reason: None,
            accepted_by: None,
        };
        let mut w = workflow_for(&f, "/tmp/w".into(), None, Acceptance::Owner, n - h(3));
        w.woken_at = Some(n);
        w.record(
            FOLLOW_UP_ANSWERED,
            "sess-a",
            n + chrono::Duration::minutes(10),
        );
        w
    }

    fn harness(text: String) -> Message {
        let mut m = Message::user(text);
        m.harness = true;
        m
    }

    fn said(text: &str) -> Message {
        Message::assistant(vec![Block::text(text)])
    }

    /// The conversation before and through the woken turn: an earlier
    /// exchange (its run ended before the wake), the wake, the answer.
    fn woken_chat(what: &str) -> (Vec<Message>, Vec<(usize, DateTime<Utc>)>) {
        let n = now();
        let messages = vec![
            Message::user("can you watch the nightly build for me"),
            said("I will look again in the morning."),
            harness(render_wake(what, n - h(3), chrono_tz::UTC)),
            said("The build passed overnight."),
        ];
        let run_ends = vec![(2, n - h(3)), (4, n + chrono::Duration::minutes(10))];
        (messages, run_ends)
    }

    #[test]
    fn a_reply_inside_the_day_is_the_owner_acting_and_a_harness_turn_is_not() {
        let what = "check the nightly build";
        let w = answered_workflow(what);
        let n = now();
        let empty: &[crate::outbox::OutboxItem] = &[];
        let measure_at = |messages: &[Message], ends: Vec<_>, at| {
            let t = Timeline::new(messages, ends);
            measure(&w, Ok(&t), Ok(empty), at).unwrap()
        };

        // Only the harness spoke after the answer — a nudge in the harness's
        // voice and a harness-marked turn. Not the owner's words: pending
        // while the day runs, none once it has closed.
        let (mut messages, mut ends) = woken_chat(what);
        messages.push(harness("(From the harness: a note for the run.)".into()));
        messages.push(said("Noted."));
        ends.push((6, n + h(2)));
        assert_eq!(
            measure_at(&messages, ends.clone(), n + h(1)).response,
            Response::Pending
        );
        let closed = measure_at(&messages, ends.clone(), n + h(30));
        assert_eq!(closed.response, Response::NoResponse);
        assert!(closed.signals.is_empty() && closed.unknown.is_empty());

        // The owner's own words, their run ended inside the day: replied.
        messages.push(Message::user("thanks, that is what I needed"));
        messages.push(said("Glad to help."));
        ends.push((8, n + h(5)));
        let replied = measure_at(&messages, ends.clone(), n + h(30));
        assert_eq!(replied.response, Response::Replied);
        assert_eq!(replied.signals, vec![Signal::Replied]);
        assert_eq!(replied.response.acted_on(), Some(true));
    }

    /// A steer the owner typed before the woken run's first turn is folded
    /// onto the wake message, unmarking it: those are the owner's words, and
    /// they are a reply.
    #[test]
    fn a_steer_folded_onto_the_wake_is_a_reply() {
        let what = "check the nightly build";
        let w = answered_workflow(what);
        let n = now();
        let empty: &[crate::outbox::OutboxItem] = &[];
        let (mut messages, ends) = woken_chat(what);
        messages[2].harness = false;
        messages[2]
            .content
            .push(Block::text("and the docs build too"));
        let t = Timeline::new(&messages, ends);
        let m = measure(&w, Ok(&t), Ok(empty), n + h(30)).unwrap();
        assert_eq!(m.response, Response::Replied);
    }

    #[test]
    fn a_reply_is_counted_only_where_the_run_ends_place_it() {
        let what = "check the nightly build";
        let w = answered_workflow(what);
        let n = now();
        let empty: &[crate::outbox::OutboxItem] = &[];
        let (mut messages, mut ends) = woken_chat(what);
        messages.push(Message::user("any news on the release?"));
        messages.push(said("Not yet."));

        // Its run ended two days later, and nothing places it earlier than
        // the answer: it may be inside or past the close — unknown.
        let mut around = ends.clone();
        around.push((6, n + h(48)));
        let t = Timeline::new(&messages, around);
        let m = measure(&w, Ok(&t), Ok(empty), n + h(72)).unwrap();
        assert_eq!(m.response, Response::Unknown, "{m:?}");
        assert_eq!(m.response.acted_on(), None);

        // A run between, ended after the close, places the reply past it:
        // the window closed with nothing in it.
        messages.insert(4, harness("(From the harness: a note.)".into()));
        messages.insert(5, said("ok"));
        ends.push((6, n + h(26)));
        ends.push((8, n + h(48)));
        let t = Timeline::new(&messages, ends);
        let m = measure(&w, Ok(&t), Ok(empty), n + h(72)).unwrap();
        assert_eq!(m.response, Response::NoResponse, "{m:?}");
    }

    #[test]
    fn the_woken_turn_is_the_one_this_follow_up_woke() {
        // The same words woke this conversation twice. The owner replied
        // between the two: that is the first follow-up's reply, and not the
        // second's.
        let what = "check the nightly build";
        let w = answered_workflow(what);
        let n = now();
        let empty: &[crate::outbox::OutboxItem] = &[];
        let (mut messages, mut ends) = woken_chat(what);
        messages.push(Message::user("thanks"));
        messages.push(said("You are welcome."));
        messages.push(harness(render_wake(what, n, chrono_tz::UTC)));
        messages.push(said("Still green."));
        ends.push((6, n + h(1)));
        ends.push((8, n + h(5) + chrono::Duration::minutes(10)));
        let t = Timeline::new(&messages, ends);
        let first = measure(&w, Ok(&t), Ok(empty), n + h(72)).unwrap();
        assert_eq!(first.response, Response::Replied, "{first:?}");
        let mut second = w.clone();
        second.woken_at = Some(n + h(5));
        second.events.retain(|e| e.kind != FOLLOW_UP_ANSWERED);
        second.record(
            FOLLOW_UP_ANSWERED,
            "sess-a",
            n + h(5) + chrono::Duration::minutes(10),
        );
        let m = measure(&second, Ok(&t), Ok(empty), n + h(72)).unwrap();
        assert_eq!(m.response, Response::NoResponse, "{m:?}");
        // A transcript that holds no wake at all cannot say: unknown.
        let bare = [Message::user("hello"), said("hi")];
        let t = Timeline::new(&bare, vec![]);
        assert_eq!(
            measure(&w, Ok(&t), Ok(empty), n + h(72)).unwrap().response,
            Response::Unknown
        );
    }

    #[test]
    fn unreadable_is_unknown_never_none_and_an_act_still_counts_beside_it() {
        let what = "check the nightly build";
        let w = answered_workflow(what);
        let n = now();
        let empty: &[crate::outbox::OutboxItem] = &[];
        let (messages, ends) = woken_chat(what);
        let t = Timeline::new(&messages, ends);
        let late = n + h(30);
        let m = measure(&w, Err("permission denied"), Ok(empty), late).unwrap();
        assert_eq!(m.response, Response::Unknown);
        assert!(m.unknown[0].contains("permission denied"), "{m:?}");
        assert_eq!(
            measure(&w, Ok(&t), Err("2 unreadable records"), late)
                .unwrap()
                .response,
            Response::Unknown
        );
        // Unknown outranks pending: distress is said while the day runs.
        assert_eq!(
            measure(&w, Err("gone"), Ok(empty), n + h(1))
                .unwrap()
                .response,
            Response::Unknown
        );
        // One thing the owner provably did decides it, whatever else could
        // not be read: the dismissal is on the workflow itself.
        let mut w = w;
        w.notice = Some(crate::workflow::Notice {
            at: n,
            reason: "Follow-up answered — open the chat".into(),
            urgent: false,
        });
        assert!(w.acknowledge(Actor::Owner, n + h(2)));
        let m = measure(&w, Err("gone"), Err("gone"), late).unwrap();
        assert_eq!(m.response, Response::Dismissed);
        assert_eq!(m.signals, vec![Signal::Dismissed { at: n + h(2) }]);
    }

    #[test]
    fn a_draft_or_a_dismissal_counts_only_by_the_owner_s_hand_inside_the_window() {
        let what = "check the nightly build";
        let n = now();
        let dir = std::env::temp_dir().join(format!("followup-measure-{}", uuid::Uuid::new_v4()));
        let _scratch = Scratch(dir.clone());
        let store = crate::outbox::OutboxStore::open(&dir).unwrap();
        let stage = |session: &str, staged: DateTime<Utc>| {
            let mut item = store
                .stage(
                    "mail_send",
                    crate::outbox::OutboxKind::Message,
                    serde_json::json!({"to": "ines.okafor@example.org", "body": "It passed."}),
                    Taint::default(),
                    crate::outbox::Provenance {
                        session_id: Some(session.into()),
                        ..Default::default()
                    },
                )
                .unwrap();
            item.created_at = staged.to_rfc3339();
            item
        };
        let resolve = |mut item: crate::outbox::OutboxItem, status: &str, at, by| {
            item.status = status.into();
            item.resolved_at = Some(DateTime::<Utc>::to_rfc3339(&at));
            item.resolved_by = Some(by);
            item
        };
        let w = answered_workflow(what);
        let (messages, ends) = woken_chat(what);
        let t = Timeline::new(&messages, ends);
        let late = n + h(30);
        let staged = n + chrono::Duration::minutes(5);
        let response = |items: &[crate::outbox::OutboxItem], w: &Workflow| {
            measure(w, Ok(&t), Ok(items), late).unwrap()
        };

        // Released by the owner and not delivered: pending, no stamp, an
        // attempt inside the window — unknown, never none.
        let mut failed = stage("sess-a", staged);
        failed
            .delivery_attempts
            .push(crate::outbox::DeliveryAttempt {
                prediction_id: None,
                id: "attempt-1".into(),
                started_at: (n + h(2)).to_rfc3339(),
                outcome: crate::outbox::DeliveryOutcome::NotDelivered,
                resolved_at: None,
                evidence: None,
            });
        let m = response(std::slice::from_ref(&failed), &w);
        assert_eq!(m.response, Response::Unknown, "{m:?}");
        // Refused before any attempt (the tool was missing): an error and no
        // attempt is still unknown.
        let mut refused = stage("sess-a", staged);
        refused.error = Some("the tool is not in the registry".into());
        let m = response(std::slice::from_ref(&refused), &w);
        assert_eq!(m.response, Response::Unknown, "{m:?}");
        // An attempt whose time cannot be read is unknown too.
        let mut unreadable = stage("sess-a", staged);
        unreadable
            .delivery_attempts
            .push(crate::outbox::DeliveryAttempt {
                prediction_id: None,
                id: "attempt-2".into(),
                started_at: "not a time".into(),
                outcome: crate::outbox::DeliveryOutcome::NotDelivered,
                resolved_at: None,
                evidence: None,
            });
        let m = response(std::slice::from_ref(&unreadable), &w);
        assert_eq!(m.response, Response::Unknown, "{m:?}");

        let released = resolve(stage("sess-a", staged), "sent", n + h(3), Actor::Owner);
        let m = response(std::slice::from_ref(&released), &w);
        assert_eq!(m.response, Response::Released);
        assert_eq!(
            m.signals,
            vec![Signal::Released {
                outbox_id: released.id.clone()
            }]
        );
        let declined = resolve(stage("sess-a", staged), "rejected", n + h(3), Actor::Owner);
        assert_eq!(response(&[declined], &w).response, Response::Rejected);

        // Nothing the owner did: another chat's draft, one staged before the
        // wake, one released after the close, one still pending.
        let ignored = [
            resolve(stage("sess-b", staged), "sent", n + h(3), Actor::Owner),
            resolve(stage("sess-a", n - h(1)), "sent", n + h(3), Actor::Owner),
            resolve(stage("sess-a", staged), "sent", n + h(25), Actor::Owner),
            stage("sess-a", staged),
        ];
        let m = response(&ignored, &w);
        assert_eq!(m.response, Response::NoResponse, "{m:?}");

        // Resolved inside the window by a hand that is not provably the
        // owner's: unknown, never the owner's act and never none.
        for by in [Actor::OwnerApproved, Actor::Unknown] {
            let item = resolve(stage("sess-a", staged), "sent", n + h(3), by);
            let m = response(&[item], &w);
            assert_eq!(m.response, Response::Unknown, "{by:?}");
        }

        // A dismissal is the owner's only when it says so.
        let mut dismissed = w.clone();
        dismissed.notice = Some(crate::workflow::Notice {
            at: n,
            reason: "Follow-up answered — open the chat".into(),
            urgent: false,
        });
        let mut by_run = dismissed.clone();
        assert!(by_run.acknowledge(Actor::Unknown, n + h(2)));
        assert_eq!(response(&[], &by_run).response, Response::Unknown);
        assert!(dismissed.acknowledge(Actor::Owner, n + h(2)));
        assert_eq!(response(&[], &dismissed).response, Response::Dismissed);
        // And only inside the window.
        let mut too_late = w.clone();
        too_late.events.push(crate::workflow::Event {
            at: n + h(26),
            kind: NOTICE_ACKED.into(),
            detail: "owner".into(),
        });
        assert_eq!(response(&[], &too_late).response, Response::NoResponse);
    }

    #[test]
    fn only_a_woken_and_answered_follow_up_is_measured() {
        let n = now();
        let empty: &[crate::outbox::OutboxItem] = &[];
        let t = Timeline::new(&[], vec![]);
        let mut w = answered_workflow("x");
        w.events.retain(|e| e.kind != FOLLOW_UP_ANSWERED);
        assert!(measure(&w, Ok(&t), Ok(empty), n).is_none(), "not answered");
        let mut own = Workflow::new("w".into(), "t".into(), "/tmp".into(), n);
        own.record(FOLLOW_UP_ANSWERED, "sess-a", n);
        assert!(
            measure(&own, Ok(&t), Ok(empty), n).is_none(),
            "not a follow-up's"
        );
    }

    #[test]
    fn the_rate_is_acted_on_over_decided_and_none_over_nothing_decided() {
        let w = answered_workflow("x");
        let row = |response| Measured {
            proposal: "p".into(),
            workflow: w.id.clone(),
            session_id: "sess-a".into(),
            woken_at: Some(now()),
            answered_at: now(),
            window_closes: now() + ACTED_ON_WINDOW,
            response,
            signals: vec![],
            unknown: vec![],
            accepted_by: None,
        };
        let undecided = Totals::of(&[row(Response::Pending), row(Response::Unknown)]);
        assert_eq!(
            (undecided.decided, undecided.acted_on_rate),
            (0, None),
            "a dash, never zero"
        );
        assert_eq!((undecided.pending, undecided.unknown), (1, 1));
        assert_eq!(Totals::of(&[]).acted_on_rate, None);
        let t = Totals::of(&[
            row(Response::Replied),
            row(Response::Rejected),
            row(Response::NoResponse),
            row(Response::Unknown),
            row(Response::Pending),
        ]);
        assert_eq!((t.acted_on, t.decided), (2, 3));
        assert!((t.acted_on_rate.unwrap() - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(
            serde_json::to_value(Response::NoResponse).unwrap(),
            serde_json::json!("none")
        );
    }

    #[test]
    fn the_workflow_returns_to_the_conversation_and_is_never_overdue() {
        let n = now();
        let f = FollowUp {
            id: "20261118T120000-abcd1234".into(),
            status: PROPOSED.into(),
            what: "check the build".into(),
            due_at: n + chrono::Duration::hours(3),
            session_id: "sess-a".into(),
            workspace: None,
            taint: Some(Taint::default()),
            proposed_at: n,
            decided_at: None,
            workflow_id: None,
            reason: None,
            accepted_by: None,
        };
        let w = workflow_for(&f, "/tmp/w".into(), None, Acceptance::Owner, n);
        assert_eq!(w.id, "followup-20261118T120000-abcd1234");
        assert_eq!(w.session_id.as_deref(), Some("sess-a"));
        let c = w.commitment.as_ref().unwrap();
        assert!(!c.overdue(n + chrono::Duration::days(30)));
        assert!(!c.follow_up_due(n));
        assert!(c.follow_up_due(n + chrono::Duration::hours(3)));
        assert_eq!(w.section(n + chrono::Duration::days(30)), "waiting");
    }
}
