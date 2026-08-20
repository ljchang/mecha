//! Which live session owns a named Slack thread, and the store that remembers.
//!
//! Rung 2 of `docs/REMOTE-CONTROL-DESIGN.md`. The TUI process owns the agent,
//! the conversation and the session file; a Slack thread is a second view onto
//! it. This module is the seam between the two, and it is a directory of JSON
//! because that is what every cross-process seam in this project is — the
//! frontdoor, the triggers, the mailbox. A socket would buy latency nothing
//! here needs and cost a second IPC idiom.
//!
//! **The TUI is the writer and the connector is a reader.** `ThreadStore` says
//! "one writer, and it is enforced" and backs it with `connector.lock`, so the
//! attaching process must not write thread records. It writes here instead,
//! and the connector reads this to learn that a thread is spoken for. Two
//! stores, one writer each, and neither has to trust the other's discipline.
//!
//! Three decisions carry it:
//!
//! - **A name is durable and its thread is forever.** `/remote-control lab`
//!   tomorrow posts into the same thread as today, which is what makes a line
//!   of work accumulate in one place instead of scattering across a new thread
//!   per session. That is why **detaching does not delete the record** — the
//!   record is how the thread is found again, and dropping it would silently
//!   start a new thread under the same name. Going cold is a state, not an
//!   absence.
//! - **Liveness is a pid, checked.** The `AgentMarker`/`RunMarker` rule, third
//!   use: `mecha_core::process_alive` rather than a bare `kill`, because the
//!   range test inside it is the whole correctness — `kill(-1, 0)` succeeds and
//!   would report every dead session as alive.
//! - **A name is a path component, so it is validated and not trusted.**
//!   `work::valid_producer` is reused rather than reimplemented: the rule for
//!   what may become a directory under `~/.mecha` should have one statement.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use mecha_slack::{chat, Slack};
use serde::{Deserialize, Serialize};

/// A session is attached and the process is expected to be alive.
pub const STATE_LIVE: &str = "live";
/// The session that held this name is gone. The thread and its history stay.
pub const STATE_COLD: &str = "cold";

/// One name, and what it is attached to.
///
/// `channel_id` and `thread_ts` are `Option` because they are learned a moment
/// after the record exists — the record is written first so that a crash
/// between writing and posting leaves a name that is claimed rather than a
/// thread nobody owns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachRecord {
    pub name: String,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub thread_ts: Option<String>,
    pub session_id: String,
    pub pid: u32,
    pub workspace: PathBuf,
    pub attached_at: DateTime<Utc>,
    /// [`STATE_LIVE`] or [`STATE_COLD`]. A string rather than an enum because
    /// this is an on-disk format read by a second process, and an unknown
    /// variant must degrade rather than fail the record — the `Proposed`
    /// lesson, one store over.
    pub state: String,
    #[serde(default)]
    pub ended_reason: Option<String>,
    pub updated_at: DateTime<Utc>,
}

impl AttachRecord {
    pub fn new(name: &str, session_id: &str, workspace: PathBuf) -> Self {
        let now = Utc::now();
        AttachRecord {
            name: name.to_string(),
            channel_id: None,
            thread_ts: None,
            session_id: session_id.to_string(),
            pid: std::process::id(),
            workspace,
            attached_at: now,
            state: STATE_LIVE.to_string(),
            ended_reason: None,
            updated_at: now,
        }
    }

    /// Whether the process behind this record still exists.
    ///
    /// A record marked live whose pid is gone reads as **not** live, so a hard
    /// kill cannot leave a name claimed forever. Same rule as the trigger
    /// `.running` marker, and for the same reason.
    pub fn is_live(&self) -> bool {
        self.state == STATE_LIVE && mecha_core::process_alive(self.pid)
    }

    /// Mark this attachment ended, keeping everything that finds the thread
    /// again.
    pub fn go_cold(&mut self, reason: &str) {
        self.state = STATE_COLD.to_string();
        self.ended_reason = Some(reason.to_string());
        self.updated_at = Utc::now();
    }
}

/// What claiming a name would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// Nobody has ever used this name.
    Fresh,
    /// The previous holder is gone. Its thread is reused, and the takeover is
    /// announced there so the scrollback never runs two sessions together with
    /// no seam.
    TakenOver {
        previous_session: String,
        thread_ts: Option<String>,
    },
    /// This session already holds it — asking again is not an error.
    AlreadyMine,
    /// Another session holds it and is alive.
    Refused { pid: u32, session_id: String },
}

/// Decide what claiming `name` would do, given whatever is on disk.
///
/// Pure, with liveness injected, so every branch is testable without spawning
/// a process — including the one that matters most, which is a dead holder
/// being taken over rather than blocking the name forever.
pub fn decide(
    existing: Option<&AttachRecord>,
    my_session: &str,
    alive: impl Fn(u32) -> bool,
) -> Claim {
    let Some(rec) = existing else {
        return Claim::Fresh;
    };
    if rec.session_id == my_session {
        return Claim::AlreadyMine;
    }
    if rec.state == STATE_LIVE && alive(rec.pid) {
        return Claim::Refused {
            pid: rec.pid,
            session_id: rec.session_id.clone(),
        };
    }
    Claim::TakenOver {
        previous_session: rec.session_id.clone(),
        thread_ts: rec.thread_ts.clone(),
    }
}

pub struct RemoteStore {
    root: PathBuf,
}

impl RemoteStore {
    pub fn default_root() -> Result<PathBuf> {
        Ok(mecha_core::work::mecha_home()?.join("remote"))
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        mecha_core::create_private_dir(&root)
            .with_context(|| format!("creating {}", root.display()))?;
        Ok(RemoteStore { root })
    }

    pub fn open_default() -> Result<Self> {
        Self::open(Self::default_root()?)
    }

    fn dir(&self, name: &str) -> Result<PathBuf> {
        mecha_core::work::valid_producer(name)
            .with_context(|| format!("`{name}` is not a usable remote-control name"))?;
        Ok(self.root.join(name))
    }

    fn path(&self, name: &str) -> Result<PathBuf> {
        Ok(self.dir(name)?.join("record.json"))
    }

    pub fn get(&self, name: &str) -> Result<Option<AttachRecord>> {
        let path = self.path(name)?;
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Ok(None);
        };
        // A malformed record is reported, never silently treated as absent:
        // "no record" would let a second session claim a name whose thread is
        // still being posted into, and two writers into one thread is the
        // confusion this store exists to prevent.
        let rec =
            serde_json::from_str(&text).with_context(|| format!("reading {}", path.display()))?;
        Ok(Some(rec))
    }

    /// Write a record, atomically. Same rules as the outbox: pretty JSON, a
    /// temp sibling, and a rename — so a reader sees the old record or the new
    /// one and never half of either.
    pub fn put(&self, rec: &AttachRecord) -> Result<()> {
        let dir = self.dir(&rec.name)?;
        mecha_core::create_private_dir(&dir)?;
        let path = dir.join("record.json");
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(rec)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Every name this store has ever held, newest attachment first.
    pub fn list(&self) -> Result<Vec<AttachRecord>> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Ok(out);
        };
        for entry in entries.flatten() {
            let path = entry.path().join("record.json");
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            // Unreadable entries are skipped here where `get` refuses, and the
            // difference is deliberate: a listing must not be taken down by one
            // bad file, but a claim must never proceed on a guess.
            if let Ok(rec) = serde_json::from_str::<AttachRecord>(&text) {
                out.push(rec);
            }
        }
        out.sort_by_key(|r| std::cmp::Reverse(r.attached_at));
        Ok(out)
    }

    /// Mark cold every record whose process is gone, returning what changed.
    ///
    /// The `mecha slack sweep` shape: a thread left showing a live attachment
    /// for a session that died is exactly the confusion this surface exists to
    /// prevent, and it must be answerable without the process that died.
    pub fn sweep(&self) -> Result<Vec<AttachRecord>> {
        let mut swept = Vec::new();
        for mut rec in self.list()? {
            if rec.state == STATE_LIVE && !mecha_core::process_alive(rec.pid) {
                rec.go_cold("the terminal session ended without detaching");
                self.put(&rec)?;
                swept.push(rec);
            }
        }
        Ok(swept)
    }
}

/// Everything the TUI needs to keep mirroring into one thread.
///
/// Held on `App` for the session, not rebuilt per run: a `/model` switch
/// replaces the agent wholesale and must not silently drop the attachment.
#[derive(Clone)]
pub struct Attached {
    pub name: String,
    pub channel_id: String,
    pub thread_ts: String,
    pub slack: Slack,
    /// The stream flush settings, resolved once at attach time.
    ///
    /// Carried here so starting a run never reads config: `submit` runs on the
    /// event loop, and a TOML read per turn is a file access in the one place
    /// that must not block on the filesystem.
    pub flush_chars: usize,
    pub flush_ms: u64,
}

/// The message that opens a thread, or re-opens one under a new session.
///
/// Pure, so the wording is testable without a workspace or a token — and the
/// wording is the feature here. Somebody reading this on a phone has to be
/// able to tell *which* session they are looking at and what it may already
/// be refusing to do, without going back to the terminal to find out.
pub fn header_text(
    name: &str,
    workspace: &Path,
    model: &str,
    taint: (bool, bool),
    prior_messages: usize,
    takeover_of: Option<&str>,
) -> String {
    let mut out = String::new();
    if let Some(previous) = takeover_of {
        out.push_str(&format!(
            "_The session that held `{name}` ended (was `{previous}`). \
             Picking the name up again._\n\n"
        ));
    }
    out.push_str(&format!("*mecha · {name}*\n"));
    out.push_str(&format!("`{}`\n", workspace.display()));
    out.push_str(&format!("model `{model}`\n"));

    // Either half changes what the next turn may do, so it is said up front
    // rather than discovered when an outbound call is refused. Same reasoning
    // as the resume line in the TUI.
    out.push_str(match taint {
        (true, true) => {
            "⚠️ already holds private data *and* third-party content — \
             outbound calls will be refused\n"
        }
        (true, false) => "holds private data\n",
        (false, true) => "holds third-party content\n",
        (false, false) => "clean\n",
    });

    if prior_messages > 0 {
        // Said, not shown. Replaying a conversation that began privately is an
        // egress decision taken retroactively, and the retroactive version
        // cannot be declined.
        out.push_str(&format!(
            "_{prior_messages} earlier message(s) are not repeated here; this thread starts now._\n"
        ));
    }

    // The honest statement of what this rung does not do yet. Without it, a
    // reply here silently starts a *separate* Slack-side run in a different
    // workspace, which looks like the mirror answering and is not.
    // Deliberately does not claim to know whether the connector is running.
    // `flock` cannot be queried without attempting it, and attempting it could
    // make the connector fail to start in that instant — so this says something
    // true in both worlds rather than probing for a fact it cannot cheaply
    // have. The same rule as a rate with no denominator reading `unknown`
    // rather than zero.
    out.push_str(
        "\n_Output only for now. Nothing typed here reaches this session — \
         and if the connector is running it starts a *separate* Slack run instead._",
    );
    out
}

/// Claim a name and open (or re-open) its thread.
///
/// The record is written **before** the header is posted, so a crash between
/// the two leaves a name that is claimed rather than a thread nobody owns —
/// the recoverable direction.
#[allow(clippy::too_many_arguments)]
pub async fn attach(
    name: &str,
    session_id: &str,
    workspace: &Path,
    model: &str,
    taint: (bool, bool),
    prior_messages: usize,
) -> Result<(Attached, String)> {
    mecha_core::work::valid_producer(name)
        .with_context(|| format!("`{name}` is not a usable remote-control name"))?;

    let store = RemoteStore::open_default()?;
    let existing = store.get(name)?;
    let claim = decide(existing.as_ref(), session_id, mecha_core::process_alive);

    let (reuse_thread, takeover_of, notice) = match &claim {
        Claim::Refused { pid, session_id } => bail!(
            "`{name}` is held by a live session ({session_id}, pid {pid}) —              pick another name, or detach it there first"
        ),
        Claim::AlreadyMine => (
            existing.as_ref().and_then(|r| r.thread_ts.clone()),
            None,
            format!("already attached as `{name}`"),
        ),
        Claim::TakenOver {
            previous_session,
            thread_ts,
        } => (
            thread_ts.clone(),
            Some(previous_session.clone()),
            format!("attached as `{name}` — reusing the thread the previous session left"),
        ),
        Claim::Fresh => (None, None, format!("attached as `{name}`")),
    };

    let mut record = match existing {
        // Keep the thread, take the session: that is what a durable name is.
        Some(mut r) => {
            r.session_id = session_id.to_string();
            r.pid = std::process::id();
            r.workspace = workspace.to_path_buf();
            r.state = STATE_LIVE.to_string();
            r.ended_reason = None;
            r.attached_at = Utc::now();
            r.updated_at = Utc::now();
            r
        }
        None => AttachRecord::new(name, session_id, workspace.to_path_buf()),
    };
    store.put(&record)?;

    let cfg = mecha_core::config::Config::load_global()?;
    let slack_store =
        mecha_slack::binding::SlackStore::open(mecha_core::work::mecha_home()?.join("slack"))?;
    let (slack, channel) = crate::slack::send::owner_dm(&slack_store).await?;

    let text = header_text(
        name,
        workspace,
        model,
        taint,
        prior_messages,
        takeover_of.as_deref(),
    );
    // A reused thread gets the header *inside* it, so one name really is one
    // scrollback. A fresh name gets a new parent message, and its `ts` becomes
    // the thread everything else replies into.
    let ts = chat::post_message(&slack, &channel, reuse_thread.as_deref(), &text, None).await?;
    let thread_ts = reuse_thread.unwrap_or_else(|| ts.to_string());

    record.channel_id = Some(channel.clone());
    record.thread_ts = Some(thread_ts.clone());
    record.updated_at = Utc::now();
    store.put(&record)?;

    Ok((
        Attached {
            name: name.to_string(),
            channel_id: channel,
            thread_ts,
            slack,
            flush_chars: cfg.slack.stream_flush_chars,
            flush_ms: cfg.slack.stream_flush_ms,
        },
        notice,
    ))
}

/// End an attachment: mark the record cold, and say so in the thread.
///
/// Best-effort on the Slack half and strict on the store half. A closing line
/// that failed to post is a cosmetic loss; a record left reading `live` for a
/// process that has gone is the confusion this whole surface exists to
/// prevent.
pub async fn detach(attached: &Attached, reason: &str) -> Result<()> {
    let store = RemoteStore::open_default()?;
    if let Some(mut rec) = store.get(&attached.name)? {
        rec.go_cold(reason);
        store.put(&rec)?;
    }
    let _ = chat::post_message(
        &attached.slack,
        &attached.channel_id,
        Some(&attached.thread_ts),
        &format!(
            "_{reason}. `/remote-control {}` picks this thread up again._",
            attached.name
        ),
        None,
    )
    .await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The house convention for a test directory — `std::env::temp_dir()`
    /// with a unique suffix, matching `threads.rs`. A new dev-dependency for
    /// eight tests is a dependency the whole crate then carries.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mecha-remote-{name}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn rec(name: &str, session: &str, pid: u32) -> AttachRecord {
        let mut r = AttachRecord::new(name, session, PathBuf::from("/w"));
        r.pid = pid;
        r
    }

    const DEAD: fn(u32) -> bool = |_| false;
    const ALIVE: fn(u32) -> bool = |_| true;

    fn header(taint: (bool, bool), prior: usize, takeover: Option<&str>) -> String {
        header_text(
            "lab",
            Path::new("/home/u/project"),
            "claude-opus-5",
            taint,
            prior,
            takeover,
        )
    }

    /// Somebody reading this on a phone has to be able to tell which session
    /// it is without going back to the terminal.
    #[test]
    fn the_header_names_the_session_its_workspace_and_its_model() {
        let h = header((false, false), 0, None);
        assert!(h.contains("mecha · lab"), "{h}");
        assert!(h.contains("/home/u/project"), "{h}");
        assert!(h.contains("claude-opus-5"), "{h}");
    }

    /// Both legs armed means outbound calls are already being refused. Saying
    /// so up front is the difference between "the harness is protecting me"
    /// and "the remote control is broken".
    #[test]
    fn a_fully_armed_interlock_is_stated_rather_than_discovered() {
        let h = header((true, true), 0, None);
        assert!(h.contains("refused"), "{h}");
        assert!(h.contains("private data"), "{h}");
        assert!(h.contains("third-party"), "{h}");
    }

    #[test]
    fn each_taint_leg_reads_differently_and_clean_says_clean() {
        assert!(header((true, false), 0, None).contains("holds private data"));
        assert!(header((false, true), 0, None).contains("third-party content"));
        assert!(header((false, false), 0, None).contains("clean"));
    }

    /// Prior turns are counted and never reproduced: replaying a conversation
    /// that began privately is an egress decision taken retroactively, and the
    /// retroactive version cannot be declined.
    #[test]
    fn earlier_messages_are_counted_and_never_repeated() {
        let h = header((false, false), 12, None);
        assert!(h.contains("12 earlier message"), "{h}");
        assert!(h.contains("starts now"), "{h}");
        assert!(!header((false, false), 0, None).contains("earlier message"));
    }

    /// A takeover has to leave a seam, or the scrollback reads as one
    /// continuous session when it is two.
    #[test]
    fn a_takeover_says_whose_thread_this_was() {
        let h = header((false, false), 0, Some("sess-old"));
        assert!(h.contains("sess-old"), "{h}");
        assert!(h.contains("ended"), "{h}");
        assert!(!header((false, false), 0, None).contains("ended"));
    }

    /// The rung's own limitation, said out loud — without it a reply here
    /// silently starts a separate Slack-side run in a different workspace,
    /// which looks like the mirror answering and is not.
    ///
    /// The wording must hold whether or not the connector is running, because
    /// this cannot cheaply find that out: `flock` is not queryable without
    /// attempting it, and attempting it could make the connector fail to
    /// start. Saying something true in both worlds beats probing for a fact
    /// and being wrong about it.
    #[test]
    fn the_header_admits_what_this_rung_cannot_do_yet() {
        let h = header((false, false), 0, None);
        assert!(h.contains("Nothing typed here reaches this session"), "{h}");
        assert!(h.contains("*separate* Slack run"), "{h}");
    }

    #[test]
    fn an_unused_name_is_free() {
        assert_eq!(decide(None, "s1", ALIVE), Claim::Fresh);
    }

    /// The whole point of a durable name: a session that died must not hold it
    /// forever, and the thread it was using is reused rather than abandoned.
    #[test]
    fn a_dead_holder_is_taken_over_and_hands_its_thread_on() {
        let mut old = rec("lab", "s1", 424242);
        old.thread_ts = Some("1755.0001".into());
        assert_eq!(
            decide(Some(&old), "s2", DEAD),
            Claim::TakenOver {
                previous_session: "s1".into(),
                thread_ts: Some("1755.0001".into())
            }
        );
    }

    #[test]
    fn a_live_holder_refuses_and_says_which_process() {
        let old = rec("lab", "s1", 4242);
        assert_eq!(
            decide(Some(&old), "s2", ALIVE),
            Claim::Refused {
                pid: 4242,
                session_id: "s1".into()
            }
        );
    }

    /// Re-running `/remote-control lab` in the session that already holds it
    /// is a question, not a collision.
    #[test]
    fn my_own_name_is_not_a_collision_even_while_live() {
        let mine = rec("lab", "s1", 4242);
        assert_eq!(decide(Some(&mine), "s1", ALIVE), Claim::AlreadyMine);
    }

    /// A cold record whose pid happens to have been reused by an unrelated
    /// process must still be takeable: the state is what was recorded on the
    /// way out, and it outranks a coincidence.
    #[test]
    fn a_cold_record_is_free_even_if_its_pid_was_recycled() {
        let mut old = rec("lab", "s1", 4242);
        old.go_cold("detached");
        assert!(matches!(
            decide(Some(&old), "s2", ALIVE),
            Claim::TakenOver { .. }
        ));
    }

    #[test]
    fn a_live_record_whose_process_is_gone_does_not_read_as_live() {
        let mut r = rec("lab", "s1", 424242);
        assert_eq!(r.state, STATE_LIVE);
        assert!(!r.is_live(), "a dead pid must not read as live");
        r.pid = std::process::id();
        assert!(r.is_live());
    }

    /// Going cold keeps everything that finds the thread again. Deleting the
    /// record would silently start a *new* thread under the same name, which
    /// is the opposite of what a durable name promises.
    #[test]
    fn going_cold_keeps_the_thread_it_was_using() {
        let store = RemoteStore::open(scratch("cold")).unwrap();

        let mut r = AttachRecord::new("lab", "s1", PathBuf::from("/w"));
        r.channel_id = Some("D1".into());
        r.thread_ts = Some("1755.0001".into());
        store.put(&r).unwrap();

        let mut back = store.get("lab").unwrap().unwrap();
        back.go_cold("detached");
        store.put(&back).unwrap();

        let after = store.get("lab").unwrap().unwrap();
        assert_eq!(after.state, STATE_COLD);
        assert_eq!(after.thread_ts.as_deref(), Some("1755.0001"));
        assert_eq!(after.channel_id.as_deref(), Some("D1"));
        assert_eq!(after.ended_reason.as_deref(), Some("detached"));
    }

    /// A name reaches the filesystem as a directory, so it is validated rather
    /// than trusted — the same rule the thread store applies to a Slack key.
    #[test]
    fn a_name_that_is_a_path_is_refused_before_it_becomes_one() {
        let store = RemoteStore::open(scratch("names")).unwrap();
        for bad in ["../escape", "has/slash", "Upper", "with space", ""] {
            assert!(store.get(bad).is_err(), "{bad:?} should be refused");
        }
        assert!(store.get("lab-2").is_ok());
    }

    #[test]
    fn a_sweep_cools_a_dead_attachment_and_leaves_a_live_one() {
        let store = RemoteStore::open(scratch("sweep")).unwrap();

        let mut dead = AttachRecord::new("gone", "s1", PathBuf::from("/w"));
        dead.pid = 424242;
        store.put(&dead).unwrap();
        store
            .put(&AttachRecord::new("here", "s2", PathBuf::from("/w")))
            .unwrap();

        let swept = store.sweep().unwrap();
        assert_eq!(swept.len(), 1);
        assert_eq!(swept[0].name, "gone");
        assert_eq!(store.get("gone").unwrap().unwrap().state, STATE_COLD);
        assert_eq!(store.get("here").unwrap().unwrap().state, STATE_LIVE);
    }
}
