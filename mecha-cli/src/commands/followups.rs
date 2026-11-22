//! `mecha followups` — the follow-ups a run proposed, and the owner's answer.
//!
//! A proposal is the model's (`propose_follow_up`); everything here is the
//! owner's. `accept` is the one way a proposal becomes a workflow, whose
//! reminder then comes due in Tasks → workflows and on Home
//! (`mecha_core::followup`, `ASSISTANT-FOLLOW-THROUGH-DESIGN.md` §Model-proposed
//! follow-ups). Shaped for `commands::review::review_source`: `list --json`,
//! `show`, `accept`, `reject --reason`, so the web's proposals pane and the
//! TUI's `/queues` drive it as they drive the other proposal stores.

use anyhow::{ensure, Context, Result};
use chrono::Utc;
use mecha_core::followup::{self, FollowUp, FollowUpStore};
use mecha_core::workflow::WorkflowStore;

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Follow-ups waiting on your answer (`--all` for decided ones too).
    List {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        all: bool,
    },
    /// One follow-up whole: what, when, and the conversation it came from.
    Show { id: String },
    /// Schedule it: creates the workflow whose reminder comes due at its time.
    Accept { id: String },
    /// Decline it, saying why.
    Reject {
        id: String,
        #[arg(long)]
        reason: String,
    },
    /// What you did within a day of each woken follow-up's answer: replied,
    /// released or declined a draft, dismissed the reminder, or none. A
    /// sensor — nothing reads it. Whether you opened the chat is not measured.
    Measure {
        #[arg(long)]
        json: bool,
        /// Only follow-ups answered at or after this time (RFC 3339).
        #[arg(long)]
        since: Option<chrono::DateTime<Utc>>,
    },
}

/// What the readout says it cannot see, on every surface it prints to.
const NOT_MEASURED: &str = "whether you opened the chat: nothing server-side records a page \
                            being looked at, so it is not measured";

/// A transcript read for measuring, refused when any line is not a
/// record. `Session::read` skips a malformed line and returns the rest —
/// right for resuming a chat, wrong here: a torn append of the owner's reply
/// would read as a conversation with no reply in it, and the follow-up as
/// `none`, where it is `unknown` (review of #903). A record from a newer
/// build that this one cannot name is refused the same way.
fn read_whole(path: &std::path::Path) -> Result<mecha_core::session::Transcript> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let torn = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter(|line| serde_json::from_str::<mecha_core::session::Record>(line).is_err())
        .count();
    if torn > 0 {
        anyhow::bail!("{torn} line(s) of {} could not be read", path.display());
    }
    mecha_core::session::Session::parse(path, &text)
}

/// Every woken-and-answered follow-up under `workflows`, measured from the
/// transcripts in `sessions_dir` and the outbox each workflow names (or
/// `default_outbox`), oldest answer first. A store that cannot be read
/// whole makes its follow-ups `unknown`, never `none`; an outbox directory
/// that does not exist held nothing.
pub fn gather(
    workflows: &WorkflowStore,
    sessions_dir: &std::path::Path,
    default_outbox: &std::path::Path,
    since: Option<chrono::DateTime<Utc>>,
    now: chrono::DateTime<Utc>,
) -> Result<Vec<followup::Measured>> {
    use mecha_core::outbox::{OutboxItem, OutboxStore};
    use mecha_core::session::Session;
    use std::collections::BTreeMap;
    let mut outboxes: BTreeMap<std::path::PathBuf, std::result::Result<Vec<OutboxItem>, String>> =
        BTreeMap::new();
    let mut rows = Vec::new();
    for w in workflows.list()? {
        let Some(answered) = followup::answered_at(&w) else {
            continue;
        };
        if since.is_some_and(|s| answered < s) {
            continue;
        }
        let root = w
            .outbox_root
            .clone()
            .unwrap_or_else(|| default_outbox.to_path_buf());
        let items = outboxes.entry(root.clone()).or_insert_with(|| {
            match root.try_exists() {
                Ok(false) => return Ok(vec![]),
                Ok(true) => {}
                Err(e) => return Err(format!("{}: {e}", root.display())),
            }
            let (items, skipped) = OutboxStore::open(&root)
                .and_then(|s| s.items_counting())
                .map_err(|e| format!("{e:#}"))?;
            if skipped > 0 {
                return Err(format!(
                    "{skipped} record(s) under {} could not be read",
                    root.display()
                ));
            }
            Ok(items)
        });
        let transcript = w
            .session_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .context("the workflow names no conversation")
            .and_then(|id| Session::find(sessions_dir, id))
            .and_then(|path| read_whole(&path))
            .map_err(|e| format!("{e:#}"));
        let timeline = transcript
            .as_ref()
            .map(followup::Timeline::of)
            .map_err(String::as_str);
        if let Some(row) = followup::measure(
            &w,
            timeline.as_ref().map_err(|e| *e),
            items.as_deref().map_err(String::as_str),
            now,
        ) {
            rows.push(row);
        }
    }
    rows.sort_by_key(|r| r.answered_at);
    Ok(rows)
}

fn measure_readout(
    rows: &[followup::Measured],
    since: Option<chrono::DateTime<Utc>>,
) -> serde_json::Value {
    let rows_json: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": followup::short(&r.proposal),
                "proposal": r.proposal,
                "workflow": r.workflow,
                "session_id": r.session_id,
                "woken_at": r.woken_at,
                "answered_at": r.answered_at,
                "window_closes": r.window_closes,
                "outcome": r.response,
                "signals": r.signals,
                "unknown": r.unknown,
                "accepted_by": r.accepted_by,
            })
        })
        .collect();
    serde_json::json!({
        "window_hours": followup::ACTED_ON_WINDOW.num_hours(),
        "since": since,
        "rows": rows_json,
        "totals": followup::Totals::of(rows),
        "not_measured": [NOT_MEASURED],
    })
}

fn measure_text(rows: &[followup::Measured], tz: chrono_tz::Tz) -> String {
    let mut out = format!(
        "What you did within {} hours of each woken follow-up's answer.\nNot measured: {NOT_MEASURED}.\n\n",
        followup::ACTED_ON_WINDOW.num_hours()
    );
    if rows.is_empty() {
        out.push_str("No follow-up has been woken and answered yet.\n");
    }
    for r in rows {
        let mut line = format!(
            "  {:<10} answered {}  {}",
            followup::short(&r.proposal),
            r.answered_at.with_timezone(&tz).format("%Y-%m-%d %H:%M %Z"),
            r.response.as_str()
        );
        if r.accepted_by == Some(followup::Acceptance::Auto) {
            line.push_str(" · auto-accepted");
        }
        let said: Vec<String> = r
            .signals
            .iter()
            .map(|s| match s {
                followup::Signal::Replied => "replied".to_string(),
                followup::Signal::Released { outbox_id } => format!("released {outbox_id}"),
                followup::Signal::Rejected { outbox_id } => format!("declined {outbox_id}"),
                followup::Signal::Dismissed { .. } => "dismissed".to_string(),
            })
            .chain(r.unknown.iter().cloned())
            .collect();
        if !said.is_empty() {
            line.push_str(&format!(" ({})", said.join("; ")));
        }
        out.push_str(&line);
        out.push('\n');
    }
    let t = followup::Totals::of(rows);
    out.push_str(&format!(
        "\nreplied {} · released {} · declined {} · dismissed {} · none {} · pending {} · unknown {}\n",
        t.replied, t.released, t.rejected, t.dismissed, t.none, t.pending, t.unknown
    ));
    let rate = t
        .acted_on_rate
        .map(|r| format!("{:.0}%", r * 100.0))
        .unwrap_or_else(|| "—".into());
    out.push_str(&format!(
        "acted on: {} of {} decided ({rate}); pending and unknown are on neither side\n",
        t.acted_on, t.decided
    ));
    out
}

fn store() -> Result<FollowUpStore> {
    FollowUpStore::open_default()
}

/// The list line's detail: when, in the owner's zone, and whether the
/// proposing conversation had read outside content.
fn detail(f: &FollowUp, tz: chrono_tz::Tz) -> String {
    let when = f.due_at.with_timezone(&tz).format("%a %b %-d, %H:%M %Z");
    let mut d = format!("due {when}");
    if f.armed() {
        d.push_str(" · proposed after reading outside content");
    }
    if f.is_proposed() && f.due_at <= Utc::now() {
        d.push_str(" · its time has passed; accepting it makes it due now");
    }
    if !f.is_proposed() {
        d.push_str(&format!(" · {}", f.status));
    }
    if f.accepted_by == Some(followup::Acceptance::Auto) {
        d.push_str(" automatically, by its chat's auto setting");
    }
    d
}

/// Who accepted it, as `show` says it.
fn accepted_by(f: &FollowUp) -> Option<&'static str> {
    use followup::Acceptance;
    match (f.status.as_str(), f.accepted_by) {
        (followup::ACCEPTED, Some(Acceptance::Owner)) => Some("you"),
        (followup::ACCEPTED, Some(Acceptance::Auto)) => {
            Some("automatically — auto was on in its chat, and it had read no outside content")
        }
        (followup::ACCEPTED, _) => Some("not recorded"),
        _ => None,
    }
}

/// Where the owner picks the conversation back up: the web app's link, and
/// the TUI's resume for a terminal, which has no Open-chat button.
fn way_back(f: &FollowUp) -> String {
    format!(
        "Open chat on its workflow in Tasks → Workflows, or `mecha tui --resume {}`",
        f.session_id
    )
}

fn owner_tz() -> chrono_tz::Tz {
    mecha_core::config::Config::load_global()
        .ok()
        .and_then(|c| c.agent.timezone.as_deref().and_then(|t| t.parse().ok()))
        .unwrap_or(chrono_tz::UTC)
}

fn show(f: &FollowUp, tz: chrono_tz::Tz) -> String {
    let mut out = String::new();
    out.push_str(&format!("follow-up   {}\n", f.id));
    out.push_str(&format!("status      {}\n", f.status));
    out.push_str(&format!(
        "due         {}\n",
        f.due_at.with_timezone(&tz).format("%Y-%m-%d %H:%M %Z")
    ));
    out.push_str(&format!("from chat   {}\n", f.session_id));
    out.push_str(&format!("back to it  {}\n", way_back(f)));
    if let Some(by) = accepted_by(f) {
        out.push_str(&format!("accepted by {by}\n"));
    }
    if let Some(w) = &f.workflow_id {
        out.push_str(&format!("workflow    {w}\n"));
    }
    if let Some(r) = &f.reason {
        out.push_str(&format!("reason      {r}\n"));
    }
    if f.armed() {
        out.push_str(
            "\n⚠ The conversation that proposed this had read content from outside (mail, \
             the web, a document). Accept it only if you would have asked for it yourself.\n",
        );
    }
    out.push_str(&format!("\n{}\n", f.what));
    out
}

/// Accept, under the follow-up store's lock: the workflow is written first
/// and the proposal marked only if that succeeded, so a failure leaves it
/// proposed. A workflow already under the id is the one an interrupted
/// accept wrote, and is taken as done rather than refused — the retry
/// finishes the first attempt, never duplicates it.
pub fn accept(
    store: &FollowUpStore,
    workflows: &WorkflowStore,
    id: &str,
    outbox_root: Option<std::path::PathBuf>,
) -> Result<FollowUp> {
    accept_as(
        store,
        workflows,
        id,
        outbox_root,
        followup::Acceptance::Owner,
    )
}

/// [`accept`], saying who accepted: the one writer that turns a proposal
/// into a workflow, for the owner's click and the standing auto rule alike
/// ([`settle_auto`]), so the two can differ only in the record they leave.
pub fn accept_as(
    store: &FollowUpStore,
    workflows: &WorkflowStore,
    id: &str,
    outbox_root: Option<std::path::PathBuf>,
    by: followup::Acceptance,
) -> Result<FollowUp> {
    let now = Utc::now();
    let found = store.find(id)?;
    store.decide(&found.id, followup::ACCEPTED, now, |f| {
        let workspace = f
            .workspace
            .clone()
            .context("this follow-up recorded no workspace, so its workflow has no jail")?;
        let w = followup::workflow_for(f, workspace, outbox_root, by, now);
        match workflows.get(&w.id) {
            Ok(existing) => ensure!(
                existing.session_id.as_deref() == Some(f.session_id.as_str()),
                "workflow {} exists and belongs to another conversation",
                existing.id
            ),
            Err(_) => {
                workflows.create(w.clone())?;
            }
        }
        f.workflow_id = Some(w.id);
        f.accepted_by = Some(by);
        Ok(())
    })
}

/// Every follow-up id in the default store now — the before half of "what
/// did this run propose", taken when a run starts, as the outbox's
/// baseline is (`review_policy::staged_since`). A store that does not exist
/// yet holds nothing, and is not created here; one that cannot be read
/// whole is `None`, and no baseline means nothing is accepted on the
/// owner's behalf: acting on a guess about what a run proposed is worse
/// than a missed convenience.
pub fn baseline() -> Option<std::collections::HashSet<String>> {
    let root = FollowUpStore::default_root().ok()?;
    baseline_at(&root)
}

fn baseline_at(root: &std::path::Path) -> Option<std::collections::HashSet<String>> {
    if !root.try_exists().ok()? {
        return Some(Default::default());
    }
    let (items, skipped) = FollowUpStore::open_unchecked(root.to_path_buf())
        .items_counting()
        .ok()?;
    (skipped == 0).then(|| items.into_iter().map(|f| f.id).collect())
}

/// What the standing auto rule did with one run's proposals.
#[derive(Debug, Default)]
pub struct Settled {
    /// Accepted on the owner's standing word, each now a workflow.
    pub accepted: Vec<FollowUp>,
    /// Left for the owner's decision in the proposals pane, and why.
    pub held: Vec<(FollowUp, Held)>,
}

/// Why a proposal the rule looked at still waits for the owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Held {
    /// Its conversation had read outside content, or its taint is unknown.
    ReadOutside,
    /// The run that proposed it stopped early or failed.
    StoppedEarly,
    /// The accept itself failed; the proposal is still proposed.
    Failed(String),
}

impl Held {
    /// The line a surface prints beside the proposal's words.
    pub fn says(&self) -> String {
        match self {
            Held::ReadOutside => "this conversation had read outside content, so it waits for you \
                                  in Review → Follow-ups"
                .into(),
            Held::StoppedEarly => {
                "the run stopped early, so it waits for you in Review → Follow-ups".into()
            }
            Held::Failed(why) => {
                format!("it could not be accepted ({why}); it waits in Review → Follow-ups")
            }
        }
    }
}

/// The standing auto rule at a run's end (`ASSISTANT-FOLLOW-THROUGH-DESIGN.md`
/// F7): the proposals `session_id` made since `baseline`, each accepted
/// through [`accept_as`] — as [`Acceptance::Auto`](followup::Acceptance::Auto)
/// — when `review_policy::auto_accepts` says so, and the rest left proposed,
/// exactly as a click-only conversation leaves them.
///
/// The scope is an id-diff against the run's start, scoped to its
/// conversation: what was already waiting when the run began — another
/// run's, an earlier turn's — is none of this run's business, whatever the
/// rule says. With `auto` off nothing is read at all.
pub fn settle_auto(
    store: &FollowUpStore,
    workflows: &WorkflowStore,
    session_id: &str,
    baseline: &std::collections::HashSet<String>,
    auto: bool,
    finished_clean: bool,
    outbox_root: Option<std::path::PathBuf>,
) -> Result<Settled> {
    let mut settled = Settled::default();
    if !auto {
        return Ok(settled);
    }
    let mine: Vec<FollowUp> = store
        .items()?
        .into_iter()
        .filter(|f| f.session_id == session_id && f.is_proposed() && !baseline.contains(&f.id))
        .collect();
    for f in mine {
        if !crate::review_policy::auto_accepts(auto, &f, finished_clean) {
            let why = if f.armed() {
                Held::ReadOutside
            } else {
                Held::StoppedEarly
            };
            settled.held.push((f, why));
            continue;
        }
        match accept_as(
            store,
            workflows,
            &f.id,
            outbox_root.clone(),
            followup::Acceptance::Auto,
        ) {
            Ok(done) => settled.accepted.push(done),
            Err(e) => settled.held.push((f, Held::Failed(format!("{e:#}")))),
        }
    }
    Ok(settled)
}

/// The chat line for one proposal the auto rule accepted.
pub fn accepted_line(f: &FollowUp, tz: chrono_tz::Tz) -> String {
    format!(
        "Follow-up accepted automatically (auto is on): \"{}\" — {}. Tasks → Workflows has it.",
        f.what,
        f.due_at
            .with_timezone(&tz)
            .format("due %a %b %-d, %H:%M %Z")
    )
}

/// The chat line for one it left for the owner.
pub fn held_line(f: &FollowUp, why: &Held) -> String {
    format!(
        "Follow-up not accepted automatically: \"{}\" — {}.",
        f.what,
        why.says()
    )
}

/// [`settle_auto`] with auto on, over the default stores, as the lines a
/// surface prints — the one call the web chat's run end and the TUI's
/// `/review auto` share, so neither can hold half the rule. A follow-up
/// store that does not exist yet holds no proposal and is not created; a
/// failure is said, never read as "nothing proposed", and whatever the run
/// proposed stays proposed.
pub fn settle_auto_lines(
    session_id: &str,
    baseline: &std::collections::HashSet<String>,
    finished_clean: bool,
    outbox_root: Option<std::path::PathBuf>,
    tz: chrono_tz::Tz,
) -> Vec<String> {
    let Some(store) = FollowUpStore::open_existing_default() else {
        return Vec::new();
    };
    let settled = WorkflowStore::default_store().and_then(|workflows| {
        settle_auto(
            &store,
            &workflows,
            session_id,
            baseline,
            true,
            finished_clean,
            outbox_root,
        )
    });
    match settled {
        Ok(s) => s
            .accepted
            .iter()
            .map(|f| accepted_line(f, tz))
            .chain(s.held.iter().map(|(f, why)| held_line(f, why)))
            .collect(),
        Err(e) => vec![format!(
            "Follow-ups were not accepted automatically ({e:#}); anything this run \
             proposed waits in Review → Follow-ups."
        )],
    }
}

pub fn reject(store: &FollowUpStore, id: &str, reason: &str) -> Result<FollowUp> {
    let reason = reason.trim();
    ensure!(!reason.is_empty(), "say why (`--reason`)");
    let found = store.find(id)?;
    store.decide(&found.id, followup::REJECTED, Utc::now(), |f| {
        f.reason = Some(reason.to_string());
        Ok(())
    })
}

pub fn run(args: Args) -> Result<()> {
    let tz = owner_tz();
    match args.cmd.unwrap_or(Cmd::List {
        json: false,
        all: false,
    }) {
        Cmd::List { json, all } => {
            // A store that does not exist yet is empty; one that exists and
            // cannot be read whole says so rather than listing a part.
            let (items, skipped) = match FollowUpStore::open_existing_default() {
                Some(s) => s.items_counting()?,
                None => (vec![], 0),
            };
            ensure!(
                skipped == 0,
                "{skipped} follow-up record(s) could not be read; fix or remove them under {}",
                FollowUpStore::default_root()?.display()
            );
            let shown: Vec<&FollowUp> = items.iter().filter(|f| all || f.is_proposed()).collect();
            if json {
                let rows: Vec<serde_json::Value> = shown
                    .iter()
                    .map(|f| {
                        serde_json::json!({
                            "id": f.id,
                            "kind": "follow-up",
                            "title": f.what,
                            "detail": detail(f, tz),
                            "status": f.status,
                            "due_at": f.due_at,
                            "session_id": f.session_id,
                            "taint": f.taint,
                            "accepted_by": f.accepted_by,
                            "proposed_at": f.proposed_at,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else if shown.is_empty() {
                println!("No follow-ups are waiting on you.");
            } else {
                for f in shown {
                    println!(
                        "{}  {}  ({})",
                        followup::short(&f.id),
                        f.what,
                        detail(f, tz)
                    );
                }
            }
        }
        // A read: it must not create the store it reads (`List`'s rule).
        Cmd::Show { id } => {
            let store = FollowUpStore::open_existing_default()
                .with_context(|| format!("no such follow-up: {id}"))?;
            print!("{}", show(&store.find(&id)?, tz));
        }
        Cmd::Accept { id } => {
            let outbox_root = mecha_core::config::Config::load_global()?.outbox.dir;
            let f = accept(
                &store()?,
                &WorkflowStore::default_store()?,
                &id,
                outbox_root,
            )?;
            let when = if f.due_at <= Utc::now() {
                "now — its time had already passed".to_string()
            } else {
                f.due_at
                    .with_timezone(&tz)
                    .format("at %Y-%m-%d %H:%M %Z")
                    .to_string()
            };
            println!(
                "Accepted {}: workflow {} reminds you {when}.\n{}.",
                followup::short(&f.id),
                f.workflow_id.as_deref().unwrap_or("?"),
                way_back(&f)
            );
        }
        Cmd::Reject { id, reason } => {
            let f = reject(&store()?, &id, &reason)?;
            println!("Rejected {}.", followup::short(&f.id));
        }
        // A read of three stores; it creates none of them.
        Cmd::Measure { json, since } => {
            let default_outbox = match mecha_core::config::Config::load_global()?.outbox.dir {
                Some(root) => root,
                None => mecha_core::outbox::OutboxStore::default_root()?,
            };
            let rows = gather(
                &WorkflowStore::default_store()?,
                &mecha_core::session::Session::default_dir()?,
                &default_outbox,
                since,
                Utc::now(),
            )?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&measure_readout(&rows, since))?
                );
            } else {
                print!("{}", measure_text(&rows, tz));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::agent::Taint;

    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn stores() -> (Scratch, FollowUpStore, WorkflowStore) {
        let dir = std::env::temp_dir().join(format!("followups-cli-{}", uuid::Uuid::new_v4()));
        let f = FollowUpStore::open(dir.join("followups")).unwrap();
        let w = WorkflowStore::at(dir.join("workflows"));
        (Scratch(dir), f, w)
    }

    fn proposed(store: &FollowUpStore, workspace: Option<&str>) -> FollowUp {
        let now = Utc::now();
        store
            .propose(
                "check the build".into(),
                now + chrono::Duration::hours(2),
                "sess-a",
                workspace.map(Into::into),
                Taint::default(),
                now,
            )
            .unwrap()
    }

    #[test]
    fn accepting_writes_the_workflow_that_reminds_and_marks_the_proposal() {
        let (_d, store, workflows) = stores();
        let f = proposed(&store, Some("/tmp/w"));
        let done = accept(&store, &workflows, followup::short(&f.id), None).unwrap();
        assert_eq!(done.status, followup::ACCEPTED);
        let w = workflows.get(done.workflow_id.as_deref().unwrap()).unwrap();
        assert_eq!(w.session_id.as_deref(), Some("sess-a"));
        assert_eq!(
            w.commitment.as_ref().unwrap().follow_up_at,
            Some(f.due_at),
            "the reminder comes due at the proposal's time"
        );
        // The owner's click, said so on both records.
        assert_eq!(done.accepted_by, Some(followup::Acceptance::Owner));
        assert_eq!(w.accepted_by, Some(followup::Acceptance::Owner));
        assert!(show(&done, chrono_tz::UTC).contains("accepted by you"));
        // Decided once.
        assert!(accept(&store, &workflows, &f.id, None).is_err());
        assert!(reject(&store, &f.id, "changed my mind").is_err());
    }

    #[test]
    fn an_interrupted_accept_is_finished_by_the_retry_not_duplicated() {
        let (_d, store, workflows) = stores();
        let f = proposed(&store, Some("/tmp/w"));
        // The workflow landed and the proposal's own write did not.
        workflows
            .create(followup::workflow_for(
                &f,
                "/tmp/w".into(),
                None,
                followup::Acceptance::Owner,
                Utc::now(),
            ))
            .unwrap();
        let done = accept(&store, &workflows, &f.id, None).unwrap();
        assert_eq!(done.status, followup::ACCEPTED);
        assert_eq!(workflows.list().unwrap().len(), 1);
    }

    /// A proposal from `session` with the given recorded taint, written as
    /// the tool writes one — or, `None`, as a record with no taint field.
    fn proposed_with(
        store: &FollowUpStore,
        session: &str,
        what: &str,
        taint: Option<Taint>,
    ) -> FollowUp {
        let now = Utc::now();
        let mut f = store
            .propose(
                what.into(),
                now + chrono::Duration::hours(2),
                session,
                Some("/tmp/w".into()),
                Taint::default(),
                now,
            )
            .unwrap();
        f.taint = taint;
        store.put(&f).unwrap();
        f
    }

    /// F7 at a run's end, over real stores: with auto on, the clean
    /// proposal this run made becomes a workflow through the owner's own
    /// accept path, recorded as the standing rule's; a tainted one and one
    /// of unknown taint stay proposed; the one waiting from before the run
    /// and another conversation's are untouched.
    #[test]
    fn auto_accepts_this_runs_clean_proposals_and_leaves_the_rest() {
        let (_d, store, workflows) = stores();
        let earlier = proposed_with(
            &store,
            "sess-a",
            "from an earlier turn",
            Some(Taint::default()),
        );
        let base = baseline_at(store.root()).expect("a readable store has a baseline");
        assert!(base.contains(&earlier.id));
        let clean = proposed_with(&store, "sess-a", "check the build", Some(Taint::default()));
        let tainted = proposed_with(
            &store,
            "sess-a",
            "the link the page asked for",
            Some(Taint {
                private: false,
                untrusted: true,
            }),
        );
        let unknown = proposed_with(&store, "sess-a", "no taint recorded", None);
        let other = proposed_with(&store, "sess-b", "another chat", Some(Taint::default()));

        let settled = settle_auto(&store, &workflows, "sess-a", &base, true, true, None).unwrap();
        assert_eq!(settled.accepted.len(), 1, "{settled:?}");
        assert_eq!(settled.accepted[0].id, clean.id);
        let held: Vec<(&str, &Held)> = settled
            .held
            .iter()
            .map(|(f, why)| (f.id.as_str(), why))
            .collect();
        assert!(held.contains(&(tainted.id.as_str(), &Held::ReadOutside)));
        assert!(held.contains(&(unknown.id.as_str(), &Held::ReadOutside)));
        assert_eq!(held.len(), 2);

        let done = store.get(&clean.id).unwrap();
        assert_eq!(done.status, followup::ACCEPTED);
        assert_eq!(done.accepted_by, Some(followup::Acceptance::Auto));
        let w = workflows.get(done.workflow_id.as_deref().unwrap()).unwrap();
        assert_eq!(w.accepted_by, Some(followup::Acceptance::Auto));
        assert!(
            w.events
                .iter()
                .any(|e| e.kind == "created" && e.detail.contains("accepted automatically")),
            "{:?}",
            w.events
        );
        assert!(show(&done, chrono_tz::UTC).contains("accepted by automatically"));
        for f in [&tainted, &unknown, &earlier, &other] {
            assert!(
                store.get(&f.id).unwrap().is_proposed(),
                "{} was decided",
                f.what
            );
        }
        assert_eq!(workflows.list().unwrap().len(), 1);
    }

    /// Auto off reads nothing and accepts nothing; an early-stopped run's
    /// clean proposal waits, saying why.
    #[test]
    fn auto_off_or_an_early_stop_accepts_nothing() {
        let (_d, store, workflows) = stores();
        let base = baseline_at(store.root()).unwrap();
        let clean = proposed_with(&store, "sess-a", "check the build", Some(Taint::default()));
        let off = settle_auto(&store, &workflows, "sess-a", &base, false, true, None).unwrap();
        assert!(off.accepted.is_empty() && off.held.is_empty());
        let stopped = settle_auto(&store, &workflows, "sess-a", &base, true, false, None).unwrap();
        assert!(stopped.accepted.is_empty());
        assert_eq!(stopped.held.len(), 1);
        assert_eq!(stopped.held[0].1, Held::StoppedEarly);
        assert!(store.get(&clean.id).unwrap().is_proposed());
        assert!(workflows.list().unwrap().is_empty());
    }

    /// No baseline, no acceptance: a store that cannot be read whole at the
    /// run's start yields none, and one that does not exist yet is empty
    /// and is not created by asking.
    #[test]
    fn a_baseline_is_none_for_a_store_not_read_whole() {
        let (d, store, _w) = stores();
        let missing = d.0.join("not-yet");
        assert_eq!(baseline_at(&missing), Some(Default::default()));
        assert!(!missing.exists());
        std::fs::write(store.root().join("torn.json"), "{").unwrap();
        assert_eq!(baseline_at(store.root()), None);
    }

    #[test]
    fn a_failed_accept_leaves_it_proposed() {
        let (_d, store, workflows) = stores();
        let f = proposed(&store, None);
        assert!(accept(&store, &workflows, &f.id, None).is_err());
        assert!(store.get(&f.id).unwrap().is_proposed());
        assert!(workflows.list().unwrap().is_empty());
    }

    #[test]
    fn a_reject_needs_a_reason_and_keeps_it() {
        let (_d, store, _w) = stores();
        let f = proposed(&store, Some("/tmp/w"));
        assert!(reject(&store, &f.id, "  ").is_err());
        let done = reject(&store, &f.id, "not needed").unwrap();
        assert_eq!(done.status, followup::REJECTED);
        assert_eq!(done.reason.as_deref(), Some("not needed"));
    }

    /// The measure over real stores on disk: a transcript read back with its
    /// run ends, an outbox, and a conversation that is gone. Explicit roots
    /// under the temp directory, never the mecha home.
    #[test]
    fn measure_reads_the_stores_and_says_what_it_could_not() {
        use mecha_core::closure::Actor;
        use mecha_core::message::{Block, Message};
        use mecha_core::outbox::{OutboxKind, OutboxStore, Provenance};
        use mecha_core::session::{Record, RunStats, Session, SessionMeta};
        use mecha_core::workflow::FOLLOW_UP_ANSWERED;

        let (dir, _f, workflows) = stores();
        let sessions = dir.0.join("sessions");
        let outbox_root = dir.0.join("outbox");
        let now = Utc::now();
        let woken = now - chrono::Duration::hours(2);
        let answered = woken + chrono::Duration::minutes(10);
        let what = "check the nightly build";

        // A conversation woken, answered, and replied to by the owner.
        let chat = |id: &str, reply: bool| {
            let s = Session::create(
                &sessions,
                SessionMeta {
                    id: id.into(),
                    created_at: woken - chrono::Duration::hours(4),
                    provider: "scripted".into(),
                    model: "test-model".into(),
                    workspace: "/tmp".into(),
                    title: None,
                    kind: None,
                },
            )
            .unwrap();
            let end = |at| {
                Record::Outcome(RunStats {
                    ended_at: Some(at),
                    ..RunStats::default()
                })
            };
            s.append_messages(&[
                Message::user("can you watch the nightly build for me"),
                Message::assistant(vec![Block::text("I will look in the morning.")]),
            ])
            .unwrap();
            s.append(&end(woken - chrono::Duration::hours(3))).unwrap();
            let mut wake = Message::user(followup::render_wake(
                what,
                woken - chrono::Duration::hours(4),
                chrono_tz::UTC,
            ));
            wake.harness = true;
            s.append_messages(&[wake, Message::assistant(vec![Block::text("It passed.")])])
                .unwrap();
            s.append(&end(answered)).unwrap();
            if reply {
                s.append_messages(&[
                    Message::user("thanks, that is what I needed"),
                    Message::assistant(vec![Block::text("Glad to help.")]),
                ])
                .unwrap();
                s.append(&end(answered + chrono::Duration::minutes(30)))
                    .unwrap();
            }
        };
        let follow_up = |tail: &str, session: &str, outbox: Option<std::path::PathBuf>| {
            let f = FollowUp {
                id: format!("20261118T090000-{tail}"),
                status: followup::ACCEPTED.into(),
                what: what.into(),
                due_at: woken,
                session_id: session.into(),
                workspace: None,
                taint: Some(Taint::default()),
                proposed_at: woken - chrono::Duration::hours(4),
                decided_at: Some(woken - chrono::Duration::hours(4)),
                workflow_id: None,
                reason: None,
                accepted_by: Some(followup::Acceptance::Owner),
            };
            let mut w = followup::workflow_for(
                &f,
                "/tmp/w".into(),
                outbox,
                followup::Acceptance::Owner,
                woken,
            );
            w.woken_at = Some(woken);
            w.record(FOLLOW_UP_ANSWERED, session, answered);
            workflows.create(w).unwrap();
        };

        chat("20261118T050000-replied1", true);
        // Its outbox root does not exist: nothing was staged there, which is
        // an empty outbox, not an unreadable one.
        follow_up(
            "repl1234",
            "20261118T050000-replied1",
            Some(dir.0.join("none")),
        );
        chat("20261118T050000-declined", false);
        follow_up("decl5678", "20261118T050000-declined", None);
        let store = OutboxStore::open(&outbox_root).unwrap();
        let draft = store
            .stage(
                "mail_send",
                OutboxKind::Message,
                serde_json::json!({"to": "ines.okafor@example.org", "body": "It passed."}),
                Taint::default(),
                Provenance {
                    session_id: Some("20261118T050000-declined".into()),
                    ..Provenance::default()
                },
            )
            .unwrap();
        store
            .resolve(&draft.id, "rejected", None, Actor::Owner)
            .unwrap();
        // A conversation that has gone.
        follow_up("gone9012", "20261118T050000-vanished", None);
        // One whose owner's reply was torn mid-append: the rest reads, and a
        // skipping reader would call it a conversation with no reply.
        chat("20261118T050000-tornrep1", true);
        let torn = Session::find(&sessions, "20261118T050000-tornrep1").unwrap();
        let text = std::fs::read_to_string(&torn).unwrap();
        let cut: String = text
            .lines()
            .map(|line| {
                if line.contains("thanks, that is what I needed") {
                    line[..line.len() / 2].to_string()
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(&torn, cut).unwrap();
        follow_up("torn3456", "20261118T050000-tornrep1", None);

        let rows = gather(&workflows, &sessions, &outbox_root, None, now).unwrap();
        let by_id = |tail: &str| {
            rows.iter()
                .find(|r| followup::short(&r.proposal) == tail)
                .unwrap_or_else(|| panic!("{tail} not measured: {rows:?}"))
        };
        assert_eq!(by_id("repl1234").response, followup::Response::Replied);
        assert_eq!(by_id("decl5678").response, followup::Response::Rejected);
        let gone = by_id("gone9012");
        assert_eq!(gone.response, followup::Response::Unknown);
        assert!(gone.unknown[0].contains("transcript"), "{gone:?}");
        let torn = by_id("torn3456");
        assert_eq!(torn.response, followup::Response::Unknown, "{torn:?}");
        assert!(torn.unknown[0].contains("could not be read"), "{torn:?}");

        let readout = measure_readout(&rows, None);
        let totals = &readout["totals"];
        assert_eq!(
            (&totals["acted_on"], &totals["decided"], &totals["unknown"]),
            (
                &serde_json::json!(2),
                &serde_json::json!(2),
                &serde_json::json!(2)
            )
        );
        assert_eq!(totals["acted_on_rate"], serde_json::json!(1.0));
        assert!(readout["not_measured"][0]
            .as_str()
            .unwrap()
            .contains("opened the chat"));
        assert!(measure_text(&rows, chrono_tz::UTC).contains("Not measured: whether you opened"));

        // `--since` past every answer: nothing to measure, and a dash.
        let none = gather(&workflows, &sessions, &outbox_root, Some(now), now).unwrap();
        assert!(none.is_empty());
        assert!(measure_readout(&none, Some(now))["totals"]["acted_on_rate"].is_null());
        assert!(measure_text(&none, chrono_tz::UTC).contains("0 of 0 decided (—)"));
    }

    #[test]
    fn show_warns_when_the_proposing_conversation_had_read_outside_content() {
        let (_d, store, _w) = stores();
        let mut f = proposed(&store, Some("/tmp/w"));
        assert!(!show(&f, chrono_tz::UTC).contains('⚠'));
        f.taint = Some(Taint {
            private: false,
            untrusted: true,
        });
        assert!(show(&f, chrono_tz::UTC).contains('⚠'));
        assert!(detail(&f, chrono_tz::UTC).contains("outside content"));
    }
}
