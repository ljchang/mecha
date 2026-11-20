//! Tasks → Background: every unattended run in one place
//! (`ASSISTANT-FOLLOW-THROUGH-DESIGN.md` §Tasks → Background, ruling B1).
//!
//! **A view, not a verb.** `GET /api/background` reads three things and
//! changes none of them: the trigger ledger with its running markers, the
//! woken follow-ups in the workflow store, and the machine's `mecha-*`
//! systemd timers. Answering stays in Tasks → Waiting, stopping a delegated
//! run on its board card, dismissing on the workflow card — one door per
//! act. The fourth kind of assistant run, a delegated task's, is drawn by
//! the page from the board it already holds (`/api/tasks`' `run` summary),
//! so this adds no second reader of the graph store.
//!
//! **Each section carries its own answer.** `{"rows": […]}`, `{"error": …}`,
//! or — jobs only — `{"unavailable": …}`. One unreadable store never blanks
//! the others, and none of them renders its own failure as an empty list:
//! "nothing ran" and "I could not look" are opposite findings (doctor's
//! dash-never-zero rule). A Mac runs these jobs under launchd, so there the
//! jobs section says it cannot read them, never "no jobs".
//!
//! **Bounded work per request, never per row.** One ledger tail scan, one
//! marker-directory read, one workflow-store listing, and exactly two
//! `systemctl` calls (the timer list, then every service's state at once),
//! each under a timeout.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Path;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Serialize;

use mecha_core::agent::Taint;
use mecha_core::questions::QuestionStore;
use mecha_core::runmarker::RunMarkers;
use mecha_core::trigger::{RunRecord, RunStatus, TriggerStore};
use mecha_core::workflow::{
    Workflow, WorkflowStore, FOLLOW_UP_ANSWERED, FOLLOW_UP_ASKED, FOLLOW_UP_FAILED,
    FOLLOW_UP_NOT_WOKEN, FOLLOW_UP_WOKEN,
};

type St = State<super::WebState>;

/// How far back the view looks. Longer history is `mecha trigger show
/// <name>`'s, and the workflow card's.
pub(super) const WINDOW: chrono::Duration = chrono::Duration::days(7);

/// The most trigger rows one answer carries. A five-minute trigger writes
/// two thousand rows a week; the newest are the ones worth reading, and the
/// section says when it stopped short (`truncated`) rather than implying it
/// showed everything.
pub(super) const TRIGGER_ROWS: usize = 200;

/// How long either `systemctl` call may take before the section says so.
const SYSTEMCTL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// One section's answer: rows, or why there are none to show.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(untagged)]
pub(super) enum Section<T> {
    Rows {
        rows: Vec<T>,
        /// Ledger rows or run markers that could not be read — each of
        /// which may be a run this list does not show.
        #[serde(skip_serializing_if = "is_zero")]
        unreadable: usize,
        /// The list stopped at its cap; older rows exist.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        truncated: bool,
    },
    /// The store exists and could not be read.
    Error { error: String },
    /// There is nothing on this machine to ask (jobs only): no `systemctl`,
    /// or no user session bus.
    Unavailable { unavailable: String },
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl<T> Section<T> {
    fn rows(rows: Vec<T>) -> Self {
        Section::Rows {
            rows,
            unreadable: 0,
            truncated: false,
        }
    }

    fn error(why: impl Into<String>) -> Self {
        Section::Error { error: why.into() }
    }
}

/// Where an assistant run stands. A closed set, and `Unknown` is a real
/// answer: a status this build cannot name, or a record that says too
/// little — never shown as done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RunState {
    Running,
    Waiting,
    Done,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Kind {
    Trigger,
    FollowUp,
}

/// One assistant run.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub(super) struct RunRow {
    pub kind: Kind,
    /// What it is: the trigger's name, or the follow-up's own words.
    pub name: String,
    pub state: RunState,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    /// The session id *Open chat* resumes (`#chat/<id>`) — never a chat key,
    /// which that route would look up as a session and 404 (review of #904).
    pub session: Option<String>,
    /// The parked question, for a `waiting` row's link to Tasks → Waiting.
    pub question: Option<String>,
    /// What had entered the run's conversation. `None` is not recorded,
    /// which the page shows as unknown, never as clean.
    pub taint: Option<Taint>,
    /// A trigger's ledger summary, first line.
    pub summary: Option<String>,
    /// Why it failed, why it is unknown, or what became of its question.
    pub detail: Option<String>,
    /// A follow-up's workflow, for the workflow card.
    pub workflow: Option<String>,
}

impl RunRow {
    fn new(kind: Kind, name: String, state: RunState) -> Self {
        RunRow {
            kind,
            name,
            state,
            started_at: None,
            finished_at: None,
            session: None,
            question: None,
            taint: None,
            summary: None,
            detail: None,
            workflow: None,
        }
    }
}

/// GET /api/background — the triggers, woken follow-ups and scheduled jobs
/// of the last seven days. Read-only.
pub async fn list(State(state): St) -> Response {
    let _ = state;
    let now = Utc::now();
    // Store reads off the executor — a slow disk must not stall the SSE
    // streams and approval cards this process is also holding — and beside
    // the systemctl calls rather than after them.
    let (stores, jobs) = tokio::join!(
        tokio::task::spawn_blocking(move || default_stores(now)),
        jobs(OsStr::new("systemctl"), SYSTEMCTL_TIMEOUT),
    );
    let (triggers, follow_ups) = match stores {
        Ok(pair) => pair,
        Err(e) => (
            Section::error(format!("reading the stores: {e}")),
            Section::error(format!("reading the stores: {e}")),
        ),
    };
    Json(serde_json::json!({
        "triggers": triggers,
        "follow_ups": follow_ups,
        "jobs": jobs,
    }))
    .into_response()
}

/// The two store sections over the owner's own stores, each opened only if
/// it exists, so drawing the page never creates what it examines.
fn default_stores(now: DateTime<Utc>) -> (Section<RunRow>, Section<RunRow>) {
    let questions = QuestionStore::open_existing_default();
    let triggers = match TriggerStore::default_root() {
        Ok(root) => trigger_rows(
            TriggerStore::existing_at(&root).as_ref(),
            questions.as_ref(),
            now,
        ),
        Err(e) => Section::error(format!("{e:#}")),
    };
    let follow_ups = match WorkflowStore::default_store() {
        Ok(store) => {
            let sessions = mecha_core::session::Session::default_dir().ok();
            follow_up_rows(&store, sessions.as_deref(), questions.as_ref(), now)
        }
        Err(e) => Section::error(format!("{e:#}")),
    };
    (triggers, follow_ups)
}

/// What became of a parked question, when the question store can say it
/// was closed: the run's state now, and why. `None` — open, or not
/// readable — leaves the run's own record standing: the ledger's `waiting`
/// is never overruled by a store that could not be read. Only `answered`
/// and `abandoned` reach `done`; a status this build cannot name (the store
/// is append-only, and a newer build may widen it) is `unknown`, never done
/// (review of #906).
fn closed_question(questions: Option<&QuestionStore>, id: &str) -> Option<(RunState, String)> {
    let q = questions?.get(id).ok()?;
    if q.is_open() {
        return None;
    }
    Some(match q.status.as_str() {
        mecha_core::questions::ANSWERED => (
            RunState::Done,
            "answered — it went on as your turn in its chat".into(),
        ),
        mecha_core::questions::ABANDONED => (RunState::Done, "you gave up on its question".into()),
        other => (RunState::Unknown, format!("its question is {other}")),
    })
}

/// The first line of a ledger summary, bounded for a list.
fn first_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    const MAX: usize = 240;
    Some(if line.chars().count() > MAX {
        let mut cut: String = line.chars().take(MAX).collect();
        cut.push('…');
        cut
    } else {
        line.to_string()
    })
}

/// Scheduled triggers: runs in flight from their markers, then the ledger's
/// rows newest first over [`WINDOW`].
///
/// An absent store is no rows; one whose ledger or markers cannot be read
/// is the section's error. The ledger is read from its tail
/// (`scan_runs_rev`) and the scan stops at the first row that ended before
/// the window — rows are appended as runs finish, so that is where the
/// window ends; a torn line is skipped and counted, never fatal.
/// Skipped slots (`skipped-overlap`, `skipped-stale`) are accounting, not
/// runs, and are left out.
pub(super) fn trigger_rows(
    store: Option<&TriggerStore>,
    questions: Option<&QuestionStore>,
    now: DateTime<Utc>,
) -> Section<RunRow> {
    let Some(store) = store else {
        return Section::rows(Vec::new());
    };
    let cutoff = now - WINDOW;
    let markers = RunMarkers::new(TriggerStore::locks_dir_at(store.root()));
    // `live_names` and `peek`, never `running`: `running` clears a dead
    // marker, and a view writes nothing.
    let live = match markers.live_names() {
        Ok(live) => live,
        Err(e) => return Section::error(format!("{e:#}")),
    };
    let mut rows = Vec::new();
    for name in &live.names {
        let Some(marker) = markers.peek(name) else {
            continue;
        };
        let mut row = RunRow::new(Kind::Trigger, name.clone(), RunState::Running);
        row.started_at = Some(marker.started_at);
        row.session = marker.session;
        rows.push(row);
    }
    let mut truncated = false;
    let mut ledger = Vec::new();
    let skipped = store.scan_runs_rev(|r| {
        if r.finished_at.unwrap_or(r.started_at) < cutoff {
            return false;
        }
        if matches!(
            r.status,
            RunStatus::SkippedOverlap | RunStatus::SkippedStale
        ) {
            return true;
        }
        if ledger.len() >= TRIGGER_ROWS {
            truncated = true;
            return false;
        }
        ledger.push(trigger_row(&r, questions));
        true
    });
    let skipped = match skipped {
        Ok(n) => n,
        Err(e) => return Section::error(format!("{e:#}")),
    };
    rows.extend(ledger);
    Section::Rows {
        rows,
        unreadable: skipped + live.unreadable,
        truncated,
    }
}

/// One ledger row as the view shows it.
pub(super) fn trigger_row(r: &RunRecord, questions: Option<&QuestionStore>) -> RunRow {
    let (mut state, mut detail) = match r.status {
        RunStatus::Ok => (
            RunState::Done,
            r.notify_error
                .as_ref()
                .map(|e| format!("ran, and was not delivered: {e}")),
        ),
        RunStatus::Error => (
            RunState::Failed,
            Some(
                r.error
                    .clone()
                    .unwrap_or_else(|| "no reason recorded".into()),
            ),
        ),
        RunStatus::Waiting => (RunState::Waiting, None),
        // The ledger is a wire format: a newer build's status loads as
        // `Unknown`, and nothing reads it as done.
        RunStatus::Unknown | RunStatus::SkippedOverlap | RunStatus::SkippedStale => (
            RunState::Unknown,
            Some("a status this build cannot name".into()),
        ),
    };
    if state == RunState::Waiting {
        match &r.question_id {
            Some(id) => {
                if let Some((now, said)) = closed_question(questions, id) {
                    state = now;
                    detail = Some(said);
                }
            }
            None => detail = Some("waiting, with no question recorded".into()),
        }
    }
    let mut row = RunRow::new(Kind::Trigger, r.trigger.clone(), state);
    row.started_at = Some(r.started_at);
    row.finished_at = r.finished_at;
    row.session = r.session_id.clone();
    row.question = r.question_id.clone();
    row.taint = Some(r.taint);
    row.summary = first_line(&r.summary);
    row.detail = detail;
    row
}

/// Woken follow-ups: workflows a follow-up was accepted into
/// (`followup::proposal_of`) that have been woken, within [`WINDOW`].
///
/// State comes from the workflow's own events after its latest
/// `follow_up_woken`: none yet is `running`, `follow_up_asked` is `waiting`
/// (or done, once the question store says it was answered or given up),
/// `follow_up_answered` is `done`, `follow_up_failed` is `failed`. A
/// follow-up the host decided never to wake (`follow_up_not_woken`) ran
/// nothing, so it is not a row — its workflow card says why. A `woken_at`
/// with neither event is `unknown`: the claim is taken before the turn
/// starts, and a record that stops there says nothing about a run.
pub(super) fn follow_up_rows(
    store: &WorkflowStore,
    sessions: Option<&Path>,
    questions: Option<&QuestionStore>,
    now: DateTime<Utc>,
) -> Section<RunRow> {
    let workflows = match store.list() {
        Ok(w) => w,
        Err(e) => return Section::error(format!("{e:#}")),
    };
    let cutoff = now - WINDOW;
    let rows = workflows
        .iter()
        .filter(|w| mecha_core::followup::proposal_of(w).is_some())
        .filter(|w| w.woken_at.is_some_and(|at| at >= cutoff))
        .filter_map(|w| follow_up_row(w, sessions, questions))
        .collect();
    Section::rows(rows)
}

/// One woken follow-up as the view shows it; `None` when it never ran.
pub(super) fn follow_up_row(
    w: &Workflow,
    sessions: Option<&Path>,
    questions: Option<&QuestionStore>,
) -> Option<RunRow> {
    let name = mecha_core::followup::wake_words(w).to_string();
    let woke = w
        .events
        .iter()
        .rposition(|e| e.kind == FOLLOW_UP_WOKEN || e.kind == FOLLOW_UP_NOT_WOKEN);
    let Some(woke) = woke else {
        let mut row = RunRow::new(Kind::FollowUp, name, RunState::Unknown);
        row.started_at = w.woken_at;
        row.session = w.session_id.clone();
        row.workflow = Some(w.id.clone());
        row.detail = Some("claimed for waking, with no record of the run".into());
        return Some(row);
    };
    if w.events[woke].kind == FOLLOW_UP_NOT_WOKEN {
        return None;
    }
    let woken = &w.events[woke];
    let outcome = w.events[woke + 1..].iter().rev().find(|e| {
        matches!(
            e.kind.as_str(),
            FOLLOW_UP_ASKED | FOLLOW_UP_ANSWERED | FOLLOW_UP_FAILED
        )
    });
    let mut row = RunRow::new(Kind::FollowUp, name, RunState::Running);
    row.started_at = Some(woken.at);
    row.workflow = Some(w.id.clone());
    // The woken event's detail is the session the turn ran in; the
    // workflow's own is the fallback on a record that left it blank.
    row.session = Some(woken.detail.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| w.session_id.clone());
    if let Some(e) = outcome {
        row.finished_at = Some(e.at);
        match e.kind.as_str() {
            FOLLOW_UP_ASKED => {
                row.state = RunState::Waiting;
                row.question = Some(e.detail.clone());
                if let Some((now, said)) = closed_question(questions, &e.detail) {
                    row.state = now;
                    row.detail = Some(said);
                }
            }
            FOLLOW_UP_ANSWERED => row.state = RunState::Done,
            _ => {
                row.state = RunState::Failed;
                row.detail = Some(e.detail.clone());
            }
        }
    }
    row.taint = match (sessions, &row.session) {
        (Some(dir), Some(id)) => transcript_taint(dir, id),
        _ => None,
    };
    Some(row)
}

/// The recorded taint of the transcript `id` in `dir`, resolved by name as
/// the board resolves it — `None` when the id is not a bare file name, the
/// transcript is gone, or it records no checkpoint.
fn transcript_taint(dir: &Path, id: &str) -> Option<Taint> {
    if !super::board::is_bare_path_component(id) {
        return None;
    }
    let path = dir.join(format!("{id}.jsonl"));
    mecha_core::session::Session::last_outcome_and_taint(&path)
        .ok()
        .and_then(|(_, taint)| taint)
}

// ---------------------------------------------------------------------------
// Scheduled jobs: the machine's `mecha-*` systemd user timers
// ---------------------------------------------------------------------------

/// Where a scheduled job stands, from its service's last run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum JobState {
    Running,
    Ok,
    Failed,
    Unknown,
}

/// One scheduled job, one line.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub(super) struct JobRow {
    pub timer: String,
    pub service: Option<String>,
    pub last_run: Option<DateTime<Utc>>,
    pub next_run: Option<DateTime<Utc>>,
    pub state: JobState,
    pub detail: Option<String>,
}

/// A `systemctl` call that did not produce output to read.
#[derive(Debug)]
enum Probe {
    /// No binary at all.
    Missing,
    /// It ran, or tried to, and did not answer.
    Failed(String),
}

/// Run `systemctl` once, killed if it takes longer than `timeout`
/// ([`SYSTEMCTL_TIMEOUT`] in serve).
///
/// The binary is a parameter so a test can hand in a stand-in script
/// (`doctor`'s `mail_rules_timer_with`); a file just written can be busy
/// for the instant another thread's fork holds its descriptor, so that one
/// error is retried briefly, as there.
async fn systemctl_run(
    systemctl: &OsStr,
    args: &[&str],
    timeout: std::time::Duration,
) -> Result<std::process::Output, Probe> {
    let mut tries = 0;
    let child = loop {
        let spawned = tokio::process::Command::new(systemctl)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn();
        match spawned {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 20 => {
                tries += 1;
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Probe::Missing),
            Err(e) => return Err(Probe::Failed(format!("systemctl could not start: {e}"))),
            Ok(child) => break child,
        }
    };
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => Ok(out),
        Ok(Err(e)) => Err(Probe::Failed(format!("reading systemctl: {e}"))),
        Err(_) => Err(Probe::Failed(format!(
            "systemctl did not answer within {:.1}s",
            timeout.as_secs_f32()
        ))),
    }
}

/// Is this `systemctl` failure "there is no user session to ask" — a
/// machine fact, like a missing binary — rather than a broken one?
fn no_user_bus(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    s.contains("failed to connect to bus")
        || s.contains("failed to connect to user scope bus")
        || s.contains("dbus_session_bus_address")
}

fn first_stderr_line(out: &std::process::Output) -> String {
    let text = String::from_utf8_lossy(&out.stderr);
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("exited {}", out.status))
}

/// The scheduled jobs: `list-timers` for last and next, then one `show`
/// over every service for how its last run went.
pub(super) async fn jobs(systemctl: &OsStr, timeout: std::time::Duration) -> Section<JobRow> {
    let out = match systemctl_run(
        systemctl,
        &[
            "--user",
            "list-timers",
            "mecha-*",
            "--all",
            "--no-pager",
            "-o",
            "json",
        ],
        timeout,
    )
    .await
    {
        Ok(out) => out,
        Err(Probe::Missing) => {
            return Section::Unavailable {
                unavailable: "systemctl is not on this machine, so its scheduled jobs cannot be \
                              read here (a Mac runs them under launchd)"
                    .into(),
            }
        }
        Err(Probe::Failed(why)) => return Section::error(why),
    };
    if !out.status.success() {
        let why = first_stderr_line(&out);
        if no_user_bus(&why) {
            return Section::Unavailable {
                unavailable: format!("there is no systemd user session to ask ({why})"),
            };
        }
        return Section::error(format!("systemctl list-timers failed: {why}"));
    }
    let timers = match parse_timers(&out.stdout) {
        Ok(t) => t,
        Err(why) => return Section::error(why),
    };
    let services: Vec<&str> = timers
        .iter()
        .filter_map(|t| t.service.as_deref())
        .filter(|s| !s.starts_with('-'))
        .collect();
    let props: Result<HashMap<String, HashMap<String, String>>, String> = if services.is_empty() {
        Ok(HashMap::new())
    } else {
        let mut args = vec!["--user", "show", "--no-pager"];
        args.extend([
            "--property=Id,LoadState,ActiveState,Result",
            // `--` so a unit name can never read as a flag.
            "--",
        ]);
        args.extend(services.iter().copied());
        match systemctl_run(systemctl, &args, timeout).await {
            Ok(out) if out.status.success() => {
                Ok(parse_show(&String::from_utf8_lossy(&out.stdout)))
            }
            // A missing unit can make `show` exit nonzero while it still
            // prints every other unit's block, as `doctor`'s timer reader
            // found: keep the blocks it printed (a unit without one reads
            // unknown), and fail only when it printed none (review of #906).
            Ok(out) if !parse_show(&String::from_utf8_lossy(&out.stdout)).is_empty() => {
                Ok(parse_show(&String::from_utf8_lossy(&out.stdout)))
            }
            Ok(out) => Err(format!(
                "systemctl show failed: {}",
                first_stderr_line(&out)
            )),
            Err(Probe::Missing) => Err("systemctl went missing between two calls".into()),
            Err(Probe::Failed(why)) => Err(why),
        }
    };
    let rows = timers
        .into_iter()
        .map(|t| {
            let (state, detail) = match &props {
                Ok(props) => job_state(t.last, t.service.as_deref().and_then(|s| props.get(s))),
                Err(why) => (
                    JobState::Unknown,
                    Some(format!("its service's state could not be read: {why}")),
                ),
            };
            JobRow {
                timer: t.unit,
                service: t.service,
                last_run: t.last,
                next_run: t.next,
                state,
                detail,
            }
        })
        .collect();
    Section::rows(rows)
}

/// One entry of `list-timers -o json`.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Timer {
    pub unit: String,
    pub service: Option<String>,
    pub last: Option<DateTime<Utc>>,
    pub next: Option<DateTime<Utc>>,
}

/// `list-timers -o json`: an array of `{unit, activates, last, next}` with
/// times in microseconds since the epoch, `0` or `null` for never. Anything
/// else is garbled, and garbled is an error — never a list of jobs that
/// look fine. A systemd too old to print JSON prints its table, and lands
/// here too.
pub(super) fn parse_timers(stdout: &[u8]) -> Result<Vec<Timer>, String> {
    let value: serde_json::Value = serde_json::from_slice(stdout).map_err(|e| {
        format!("systemctl's timer list did not read as JSON ({e}) — a systemd that cannot list timers as JSON cannot be read here")
    })?;
    let entries = value
        .as_array()
        .ok_or("systemctl's timer list was not a list")?;
    let usec = |v: &serde_json::Value| {
        v.as_u64()
            .filter(|n| *n > 0)
            .and_then(|n| i64::try_from(n).ok())
            .and_then(DateTime::from_timestamp_micros)
    };
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let unit = e["unit"]
                .as_str()
                .filter(|u| !u.is_empty())
                .ok_or_else(|| format!("systemctl's timer list: entry {i} names no unit"))?;
            Ok(Timer {
                unit: unit.to_string(),
                service: e["activates"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
                last: usec(&e["last"]),
                next: usec(&e["next"]),
            })
        })
        .collect()
}

/// `systemctl show`'s `Key=value` blocks, one per unit and separated by a
/// blank line, keyed by each block's `Id`. A block without one is dropped:
/// its job reads as unknown below, never as fine.
pub(super) fn parse_show(text: &str) -> HashMap<String, HashMap<String, String>> {
    let mut out = HashMap::new();
    for block in text.split("\n\n") {
        let props: HashMap<String, String> = block
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect();
        if let Some(id) = props.get("Id").cloned() {
            out.insert(id, props);
        }
    }
    out
}

/// How a job's last run went, from its service's properties.
///
/// `ok` needs the whole story: a loaded service, inactive, whose last
/// `Result` was `success`, and a timer that has fired at least once — a
/// service that never ran is also inactive and successful, and "nothing
/// went wrong" is not "it ran" (`unknown`, *not run yet*).
pub(super) fn job_state(
    last: Option<DateTime<Utc>>,
    props: Option<&HashMap<String, String>>,
) -> (JobState, Option<String>) {
    let Some(p) = props else {
        return (
            JobState::Unknown,
            Some("systemctl said nothing about its service".into()),
        );
    };
    let get = |k: &str| p.get(k).map(String::as_str).filter(|v| !v.is_empty());
    if let Some(load) = get("LoadState").filter(|l| *l != "loaded") {
        return (JobState::Unknown, Some(format!("its service is {load}")));
    }
    match (get("ActiveState"), get("Result")) {
        (Some("active" | "activating" | "deactivating" | "reloading" | "refreshing"), _) => {
            (JobState::Running, None)
        }
        (Some("failed"), result) => (
            JobState::Failed,
            Some(format!("last run: {}", result.unwrap_or("failed"))),
        ),
        (Some("inactive"), Some("success")) if last.is_some() => (JobState::Ok, None),
        (Some("inactive"), Some("success")) => (JobState::Unknown, Some("not run yet".into())),
        (Some("inactive"), Some(result)) => (JobState::Failed, Some(format!("last run: {result}"))),
        (active, result) => (
            JobState::Unknown,
            Some(format!(
                "systemd says {} / {}",
                active.unwrap_or("no state"),
                result.unwrap_or("no result")
            )),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use mecha_core::followup::{self, FollowUp};
    use std::path::PathBuf;

    /// A scratch directory under the temp dir, never the mecha home.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "mecha-background-{tag}-{}",
                mecha_core::session::Session::new_id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn rows<T: Clone + Serialize>(s: &Section<T>) -> Vec<T> {
        match s {
            Section::Rows { rows, .. } => rows.clone(),
            other => panic!(
                "expected rows, got {}",
                serde_json::to_string(other).unwrap()
            ),
        }
    }

    fn run(trigger: &str, status: RunStatus, ended: DateTime<Utc>) -> RunRecord {
        let mut r = RunRecord::started(trigger, None, false);
        r.started_at = ended - Duration::minutes(3);
        r.finished_at = Some(ended);
        r.status = status;
        r.session_id = Some(format!("20261120T090000-{trigger}"));
        r.summary = format!("{trigger} summary line\nsecond line");
        r
    }

    /// Every ledger status lands on its own state, and a status this build
    /// cannot name is `unknown` — never done. Skipped slots are not runs.
    #[test]
    fn trigger_rows_map_each_status_and_never_read_unknown_as_done() {
        let dir = Scratch::new("trigger-states");
        let store = TriggerStore::open(dir.0.join("triggers")).unwrap();
        let questions = QuestionStore::open(dir.0.join("questions")).unwrap();
        let now = Utc::now();
        let open = questions
            .park(
                "Which branch should the nightly build use?",
                vec![],
                "20261120T090000-waiting",
                None,
                None,
                Taint::default(),
                None,
            )
            .unwrap();
        let answered = questions
            .park(
                "Should the report go out today?",
                vec![],
                "20261120T090000-answered",
                None,
                None,
                Taint::default(),
                None,
            )
            .unwrap();
        questions.answer(&answered.id, "yes").unwrap();
        // A status a newer build writes: closed, but not one this build can
        // name, so the run it held is unknown — never done.
        let mut widened = questions
            .park(
                "Which room is the review in?",
                vec![],
                "20261120T090000-widened",
                None,
                None,
                Taint::default(),
                None,
            )
            .unwrap();
        widened.status = "superseded-by-a-later-build".into();
        questions.put(&widened).unwrap();

        store
            .append_run(&run("good", RunStatus::Ok, now - Duration::hours(5)))
            .unwrap();
        let mut bad = run("bad", RunStatus::Error, now - Duration::hours(4));
        bad.error = Some("provider timed out".into());
        store.append_run(&bad).unwrap();
        let mut waiting = run("asks", RunStatus::Waiting, now - Duration::hours(3));
        waiting.question_id = Some(open.id.clone());
        waiting.taint = Taint {
            private: true,
            untrusted: true,
        };
        store.append_run(&waiting).unwrap();
        let mut widened_run = run("widened", RunStatus::Waiting, now - Duration::minutes(150));
        widened_run.question_id = Some(widened.id.clone());
        store.append_run(&widened_run).unwrap();
        let mut moved_on = run("asked", RunStatus::Waiting, now - Duration::hours(2));
        moved_on.question_id = Some(answered.id.clone());
        store.append_run(&moved_on).unwrap();
        store
            .append_run(&run(
                "skipped",
                RunStatus::SkippedOverlap,
                now - Duration::hours(1),
            ))
            .unwrap();
        // A newer build's status, written as that build would write it.
        let mut future = serde_json::to_value(run("future", RunStatus::Ok, now)).unwrap();
        future["status"] = "re-queued-by-a-later-build".into();
        use std::io::Write;
        let mut ledger = std::fs::OpenOptions::new()
            .append(true)
            .open(store.ledger_path())
            .unwrap();
        writeln!(ledger, "{future}").unwrap();

        let section = trigger_rows(Some(&store), Some(&questions), now);
        let got = rows(&section);
        let state = |name: &str| {
            got.iter()
                .find(|r| r.name == name)
                .unwrap_or_else(|| panic!("no row for {name}"))
                .state
        };
        assert_eq!(state("good"), RunState::Done);
        assert_eq!(state("bad"), RunState::Failed);
        assert_eq!(state("asks"), RunState::Waiting);
        assert_eq!(state("future"), RunState::Unknown, "unknown is never done");
        assert_eq!(
            state("widened"),
            RunState::Unknown,
            "a question status this build cannot name is not done"
        );
        assert_eq!(
            state("asked"),
            RunState::Done,
            "a question since answered is not waiting on you"
        );
        assert!(
            got.iter().all(|r| r.name != "skipped"),
            "a skip is not a run"
        );

        let asks = got.iter().find(|r| r.name == "asks").unwrap();
        assert_eq!(asks.question.as_deref(), Some(open.id.as_str()));
        assert_eq!(asks.taint.map(|t| t.untrusted), Some(true));
        assert_eq!(asks.summary.as_deref(), Some("asks summary line"));
        assert_eq!(asks.session.as_deref(), Some("20261120T090000-asks"));
        let bad = got.iter().find(|r| r.name == "bad").unwrap();
        assert_eq!(bad.detail.as_deref(), Some("provider timed out"));
        // Newest first: the ledger's tail is read first.
        assert_eq!(got.first().unwrap().name, "future");
    }

    /// A live marker is a run in flight; rows that ended before the window
    /// are left out; a torn line is counted, not fatal.
    #[test]
    fn a_live_marker_is_running_and_the_window_is_seven_days() {
        let dir = Scratch::new("trigger-window");
        let store = TriggerStore::open(dir.0.join("triggers")).unwrap();
        let now = Utc::now();
        store
            .append_run(&run("ancient", RunStatus::Ok, now - Duration::days(8)))
            .unwrap();
        store
            .append_run(&run("recent", RunStatus::Ok, now - Duration::days(6)))
            .unwrap();
        use std::io::Write;
        let mut ledger = std::fs::OpenOptions::new()
            .append(true)
            .open(store.ledger_path())
            .unwrap();
        writeln!(ledger, "{{\"trigger\": \"torn").unwrap();
        // This process is alive, so its marker is a run in flight.
        store.mark_running("nightly-build", None).unwrap();

        let section = trigger_rows(Some(&store), None, now);
        let got = rows(&section);
        let names: Vec<&str> = got.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["nightly-build", "recent"]);
        assert_eq!(got[0].state, RunState::Running);
        assert!(got[0].started_at.is_some());
        let Section::Rows { unreadable, .. } = section else {
            unreachable!()
        };
        assert_eq!(unreadable, 1, "the torn line is said");
    }

    /// An absent store is no rows; one whose ledger cannot be read is the
    /// section's error, never an empty list.
    #[test]
    fn an_unreadable_ledger_is_the_section_s_error_not_an_empty_list() {
        let dir = Scratch::new("trigger-unreadable");
        assert_eq!(trigger_rows(None, None, Utc::now()), Section::rows(vec![]));
        let store = TriggerStore::open(dir.0.join("triggers")).unwrap();
        std::fs::create_dir(store.ledger_path()).unwrap();
        let section = trigger_rows(Some(&store), None, Utc::now());
        assert!(
            matches!(section, Section::Error { .. }),
            "{}",
            serde_json::to_string(&section).unwrap()
        );
    }

    fn follow_up(tail: &str, what: &str, woken: Option<DateTime<Utc>>) -> Workflow {
        let f = FollowUp {
            id: format!("20261118T090000-{tail}"),
            status: followup::ACCEPTED.into(),
            what: what.into(),
            due_at: Utc::now() - Duration::days(1),
            session_id: format!("20261118T050000-{tail}"),
            workspace: None,
            taint: Some(Taint::default()),
            proposed_at: Utc::now() - Duration::days(2),
            decided_at: Some(Utc::now() - Duration::days(2)),
            workflow_id: None,
            reason: None,
        };
        let mut w = followup::workflow_for(&f, "/tmp/w".into(), None, Utc::now());
        w.woken_at = woken;
        w
    }

    /// Each follow-up event lands on its state; a follow-up never woken, one
    /// the host chose not to wake, an ordinary workflow, and one woken
    /// before the window are not rows.
    #[test]
    fn follow_up_rows_read_their_state_from_the_latest_outcome() {
        let dir = Scratch::new("follow-ups");
        let store = WorkflowStore::at(dir.0.join("workflows"));
        let sessions = dir.0.join("sessions");
        let questions = QuestionStore::open(dir.0.join("questions")).unwrap();
        let now = Utc::now();
        let at = now - Duration::hours(2);

        let mut running = follow_up("run00001", "check the nightly build", Some(at));
        running.record(FOLLOW_UP_WOKEN, "20261118T050000-run00001", at);
        store.create(running).unwrap();

        let q = questions
            .park(
                "Which mirror should the build use?",
                vec![],
                "20261118T050000-ask00001",
                None,
                None,
                Taint::default(),
                None,
            )
            .unwrap();
        let mut asked = follow_up("ask00001", "ask about the mirror", Some(at));
        asked.record(FOLLOW_UP_WOKEN, "20261118T050000-ask00001", at);
        asked.record(FOLLOW_UP_ASKED, q.id.clone(), at + Duration::minutes(2));
        store.create(asked).unwrap();

        let mut done = follow_up("ans00001", "send the example-lab summary", Some(at));
        done.record(FOLLOW_UP_WOKEN, "20261118T050000-ans00001", at);
        done.record(
            FOLLOW_UP_ANSWERED,
            "20261118T050000-ans00001",
            at + Duration::minutes(5),
        );
        // A later bookkeeping event does not hide the outcome.
        done.record(
            "source_changed",
            "a draft was linked",
            at + Duration::minutes(6),
        );
        store.create(done).unwrap();

        let mut failed = follow_up("fai00001", "look at the queue", Some(at));
        failed.record(FOLLOW_UP_WOKEN, "20261118T050000-fai00001", at);
        failed.record(
            FOLLOW_UP_FAILED,
            "provider refused",
            at + Duration::minutes(1),
        );
        store.create(failed).unwrap();

        // `woken_at` claimed, and no record of a run after it.
        store
            .create(follow_up("clm00001", "claimed and silent", Some(at)))
            .unwrap();

        let mut never = follow_up("nev00001", "a memory chat", Some(at));
        never.record(FOLLOW_UP_NOT_WOKEN, "it is a memory chat", at);
        store.create(never).unwrap();
        store
            .create(follow_up("unw00001", "not due yet", None))
            .unwrap();
        let mut old = follow_up("old00001", "last month", Some(now - Duration::days(9)));
        old.record(
            FOLLOW_UP_WOKEN,
            "20261118T050000-old00001",
            now - Duration::days(9),
        );
        store.create(old).unwrap();
        let mut plain = Workflow::new(
            "plain-1".into(),
            "a task's workflow".into(),
            "/tmp/w".into(),
            now,
        );
        plain.woken_at = Some(at);
        plain.record(FOLLOW_UP_WOKEN, "s", at);
        store.create(plain).unwrap();

        // The asked follow-up's transcript read outside content.
        let s = mecha_core::session::Session::create(
            &sessions,
            mecha_core::session::SessionMeta {
                id: "20261118T050000-ask00001".into(),
                created_at: at,
                provider: "scripted".into(),
                model: "test-model".into(),
                workspace: "/tmp".into(),
                title: None,
                kind: None,
            },
        )
        .unwrap();
        s.append(&mecha_core::session::Record::Taint(Taint {
            private: false,
            untrusted: true,
        }))
        .unwrap();

        let got = rows(&follow_up_rows(
            &store,
            Some(&sessions),
            Some(&questions),
            now,
        ));
        let by = |what: &str| got.iter().find(|r| r.name == what).cloned();
        assert_eq!(
            by("check the nightly build").unwrap().state,
            RunState::Running
        );
        let asked = by("ask about the mirror").unwrap();
        assert_eq!(asked.state, RunState::Waiting);
        assert_eq!(asked.question.as_deref(), Some(q.id.as_str()));
        assert_eq!(asked.session.as_deref(), Some("20261118T050000-ask00001"));
        assert_eq!(asked.taint.map(|t| t.untrusted), Some(true));
        let done = by("send the example-lab summary").unwrap();
        assert_eq!(done.state, RunState::Done);
        assert_eq!(done.taint, None, "no transcript: unknown, not clean");
        let failed = by("look at the queue").unwrap();
        assert_eq!(failed.state, RunState::Failed);
        assert_eq!(failed.detail.as_deref(), Some("provider refused"));
        assert_eq!(by("claimed and silent").unwrap().state, RunState::Unknown);
        for gone in [
            "a memory chat",
            "not due yet",
            "last month",
            "a task's workflow",
        ] {
            assert!(by(gone).is_none(), "{gone} is not a woken follow-up's run");
        }
        assert_eq!(got.len(), 5);

        // Answered since: no longer waiting on you.
        questions.answer(&q.id, "the second one").unwrap();
        let got = rows(&follow_up_rows(
            &store,
            Some(&sessions),
            Some(&questions),
            now,
        ));
        let asked = got
            .iter()
            .find(|r| r.name == "ask about the mirror")
            .unwrap();
        assert_eq!(asked.state, RunState::Done);
    }

    /// A workflow store that cannot be read is the section's error.
    #[test]
    fn an_unreadable_workflow_store_is_an_error_not_an_empty_list() {
        let dir = Scratch::new("follow-ups-bad");
        let root = dir.0.join("workflows");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("broken.json"), "{ not json").unwrap();
        let store = WorkflowStore::at(&root);
        let section = follow_up_rows(&store, None, None, Utc::now());
        assert!(matches!(section, Section::Error { .. }));
        // And an absent one is no rows.
        let absent = WorkflowStore::at(dir.0.join("nowhere"));
        assert_eq!(
            follow_up_rows(&absent, None, None, Utc::now()),
            Section::rows(vec![])
        );
    }

    /// A stand-in `systemctl` at a path of the test's own: `list-timers`
    /// answers with `timers`, `show` with `show` (exit status `show_exit`).
    fn fake_systemctl(dir: &Path, timers: &str, show: &str, show_exit: i32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("systemctl");
        std::fs::write(dir.join("timers.out"), timers).unwrap();
        std::fs::write(dir.join("show.out"), show).unwrap();
        let script = format!(
            "#!/bin/sh\ncase \"$2\" in\n  list-timers) cat '{d}/timers.out' ;;\n  show) cat '{d}/show.out'; exit {show_exit} ;;\n  *) echo \"unexpected: $*\" >&2; exit 2 ;;\nesac\n",
            d = dir.display()
        );
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    // Realistic `list-timers -o json` output (systemd 255), microseconds.
    const TIMERS: &str = r#"[{"next":1795194900000000,"left":1795194900000000,"last":1795194600250848,"passed":1052750104210,"unit":"mecha-mail-rules.timer","activates":"mecha-mail-rules.service"},{"next":1795231800000000,"left":1795231800000000,"last":1795145402243213,"passed":1003552096575,"unit":"mecha-ruminate.timer","activates":"mecha-ruminate.service"},{"next":1795261243359751,"left":1795261243359751,"last":1795175367243044,"passed":1033517096410,"unit":"mecha-github-collect.timer","activates":"mecha-github-collect.service"},{"next":1795300000000000,"left":1795300000000000,"last":null,"passed":null,"unit":"mecha-hud-refresh.timer","activates":"mecha-hud-refresh.service"}]"#;

    const SHOW: &str = "Result=success\nId=mecha-mail-rules.service\nLoadState=loaded\nActiveState=inactive\n\n\
                        Result=exit-code\nId=mecha-ruminate.service\nLoadState=loaded\nActiveState=failed\n\n\
                        Result=success\nId=mecha-github-collect.service\nLoadState=loaded\nActiveState=activating\n\n\
                        Result=success\nId=mecha-hud-refresh.service\nLoadState=loaded\nActiveState=inactive\n";

    const QUICK: std::time::Duration = std::time::Duration::from_secs(5);

    #[tokio::test]
    async fn jobs_read_ok_failed_and_running_from_a_realistic_systemctl() {
        let dir = Scratch::new("jobs-ok");
        let bin = fake_systemctl(&dir.0, TIMERS, SHOW, 0);
        let got = rows(&jobs(bin.as_os_str(), QUICK).await);
        let state = |timer: &str| got.iter().find(|j| j.timer == timer).unwrap().state;
        assert_eq!(state("mecha-mail-rules.timer"), JobState::Ok);
        assert_eq!(state("mecha-ruminate.timer"), JobState::Failed);
        assert_eq!(state("mecha-github-collect.timer"), JobState::Running);
        assert_eq!(
            state("mecha-hud-refresh.timer"),
            JobState::Unknown,
            "never run is not ok"
        );
        let mail = got
            .iter()
            .find(|j| j.timer == "mecha-mail-rules.timer")
            .unwrap();
        assert_eq!(mail.service.as_deref(), Some("mecha-mail-rules.service"));
        assert_eq!(
            mail.last_run,
            DateTime::from_timestamp_micros(1_795_194_600_250_848)
        );
        assert_eq!(
            mail.next_run,
            DateTime::from_timestamp_micros(1_795_194_900_000_000)
        );
        let ruminate = got
            .iter()
            .find(|j| j.timer == "mecha-ruminate.timer")
            .unwrap();
        assert_eq!(ruminate.detail.as_deref(), Some("last run: exit-code"));
    }

    /// No binary, and no user bus, are this machine's facts: `unavailable`
    /// with a reason, never an empty list.
    #[tokio::test]
    async fn no_systemctl_or_no_user_bus_is_unavailable_never_empty() {
        let dir = Scratch::new("jobs-missing");
        let missing = jobs(dir.0.join("no-such-systemctl").as_os_str(), QUICK).await;
        assert!(
            matches!(&missing, Section::Unavailable { unavailable } if unavailable.contains("launchd")),
            "{}",
            serde_json::to_string(&missing).unwrap()
        );
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.0.join("systemctl");
        std::fs::write(
            &bin,
            "#!/bin/sh\necho 'Failed to connect to bus: No medium found' >&2\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(matches!(
            jobs(bin.as_os_str(), QUICK).await,
            Section::Unavailable { .. }
        ));
    }

    /// A systemctl that fails, prints something else, or says nothing about
    /// a service never yields an `ok` job.
    #[tokio::test]
    async fn a_failing_or_garbled_systemctl_is_an_error_or_unknown_never_ok() {
        let dir = Scratch::new("jobs-garbled");
        // An older systemd's table instead of JSON.
        let table = "Fri 2026-11-20 17:15:00 UTC 1min left Fri 2026-11-20 17:10:00 UTC 3min ago mecha-mail-rules.timer mecha-mail-rules.service\n";
        let bin = fake_systemctl(&dir.0, table, SHOW, 0);
        assert!(matches!(
            jobs(bin.as_os_str(), QUICK).await,
            Section::Error { .. }
        ));
        // A list that names no unit.
        let bin = fake_systemctl(&dir.0, r#"[{"next":1}]"#, SHOW, 0);
        assert!(matches!(
            jobs(bin.as_os_str(), QUICK).await,
            Section::Error { .. }
        ));
        // `show` exiting nonzero while it still printed the blocks (a
        // missing unit, on some systemd): the printed blocks are read.
        let bin = fake_systemctl(&dir.0, TIMERS, SHOW, 1);
        let got = rows(&jobs(bin.as_os_str(), QUICK).await);
        assert!(
            got.iter().any(|j| j.state == JobState::Ok)
                && got.iter().any(|j| j.state == JobState::Failed),
            "a nonzero show threw away what it printed: {got:?}"
        );
        // `show` failing: every job unknown, with the reason.
        let bin = fake_systemctl(&dir.0, TIMERS, "", 1);
        let got = rows(&jobs(bin.as_os_str(), QUICK).await);
        assert_eq!(got.len(), 4);
        assert!(got.iter().all(|j| j.state == JobState::Unknown), "{got:?}");
        // `show` answering with blocks that carry no `Id`: unknown, not ok.
        let bin = fake_systemctl(&dir.0, TIMERS, "Result=success\nActiveState=inactive\n", 0);
        let got = rows(&jobs(bin.as_os_str(), QUICK).await);
        assert!(got.iter().all(|j| j.state == JobState::Unknown), "{got:?}");
        // A list-timers that exits nonzero for another reason is an error.
        let bin = fake_systemctl(&dir.0, TIMERS, SHOW, 0);
        std::fs::write(&bin, "#!/bin/sh\necho 'Access denied' >&2\nexit 1\n").unwrap();
        assert!(matches!(
            jobs(bin.as_os_str(), QUICK).await,
            Section::Error { .. }
        ));
        // A hung systemctl is cut off and said.
        std::fs::write(&bin, "#!/bin/sh\nexec sleep 5\n").unwrap();
        let hung = jobs(bin.as_os_str(), std::time::Duration::from_millis(200)).await;
        assert!(
            matches!(&hung, Section::Error { error } if error.contains("did not answer")),
            "{}",
            serde_json::to_string(&hung).unwrap()
        );
    }

    /// The real handler, through the router, over a scratch home: the
    /// trigger ledger cannot be read, and the follow-ups section still has
    /// its row — one unreadable store never blanks the others.
    #[tokio::test]
    async fn the_endpoint_answers_each_section_on_its_own() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::util::ServiceExt;
        let home = crate::testenv::HomeGuard::new("serve-background");
        let triggers = TriggerStore::open(TriggerStore::root_under(&home.dir)).unwrap();
        std::fs::create_dir(triggers.ledger_path()).unwrap();
        let workflows = WorkflowStore::at(home.dir.join("workflows"));
        let at = Utc::now() - Duration::hours(1);
        let mut w = follow_up("end00001", "check the nightly build", Some(at));
        w.record(FOLLOW_UP_WOKEN, "20261118T050000-end00001", at);
        workflows.create(w).unwrap();

        let response = super::super::tests::test_router()
            .oneshot(
                Request::builder()
                    .uri("/api/background")
                    .header("Tailscale-User-Login", "owner@example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(v["triggers"]["error"].is_string(), "{v}");
        let follow_ups = v["follow_ups"]["rows"].as_array().unwrap();
        assert_eq!(follow_ups.len(), 1, "{v}");
        assert_eq!(follow_ups[0]["state"], "running");
        assert_eq!(follow_ups[0]["kind"], "follow_up");
        assert_eq!(follow_ups[0]["session"], "20261118T050000-end00001");
        // Jobs read this machine's systemd, whatever it is: one of the three
        // answers, never absent.
        let jobs = v["jobs"].as_object().unwrap();
        assert!(
            jobs.contains_key("rows")
                || jobs.contains_key("error")
                || jobs.contains_key("unavailable"),
            "{v}"
        );
    }
}
