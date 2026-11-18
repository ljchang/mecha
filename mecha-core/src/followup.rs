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
        if let Ok(f) = self.get(needle) {
            return Ok(f);
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
/// "Follow-up due" reminder.
pub fn workflow_for(
    f: &FollowUp,
    workspace: PathBuf,
    outbox_root: Option<PathBuf>,
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
    w.record("created", format!("Owner accepted follow-up {}", f.id), now);
    w
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
        };
        let w = workflow_for(&f, "/tmp/w".into(), None, n);
        assert_eq!(w.id, "followup-20261118T120000-abcd1234");
        assert_eq!(w.session_id.as_deref(), Some("sess-a"));
        let c = w.commitment.as_ref().unwrap();
        assert!(!c.overdue(n + chrono::Duration::days(30)));
        assert!(!c.follow_up_due(n));
        assert!(c.follow_up_due(n + chrono::Duration::hours(3)));
        assert_eq!(w.section(n + chrono::Duration::days(30)), "waiting");
    }
}
