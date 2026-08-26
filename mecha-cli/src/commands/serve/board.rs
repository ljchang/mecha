//! Phase 3 of `mecha serve`, continued: tasks and notes.
//!
//! Both stores are the knowledge graph's, reached exactly as the TUI reaches
//! them — every read and mutation drives the CLI (`mecha tasks …`,
//! `mecha kg …`), which reaches the graph over its MCP surface. The board's
//! rules ride along for free: nothing here confirms (every status is one tap
//! from where it was, and the tool surface has no delete), and a note is
//! evidence — the graph's extractor mines it, and candidates wait in the
//! owner's review queue, never entering belief directly.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use super::review::{self_json, verb};

type St = State<super::WebState>;

/// GET /api/tasks — the board, straight off `kg_task_list`'s own JSON, with
/// how each delegated run *went* attached (D16).
///
/// The child pays an MCP startup (~a second against the graph server), which
/// is why the timeout is generous and the page shows its own loading state.
///
/// **Attached here rather than fetched per card**, and that is the whole of
/// D16 rather than a performance note. The rule is that no two card states
/// render alike and that `failed` must never render as `idle` — a board that
/// paints every task as idle and then flickers into its real state is that
/// rule broken for the first second, on the surface people glance at. One
/// request, every card correct on first paint.
///
/// Bounded the way `runlog`'s scans are: only sessions the board actually
/// names, and only their outcome records — no message parsing, no
/// `Session::load`. Measured, a task transcript is 9–76 KB and a board names
/// a handful, so this is a few tens of milliseconds beside a child process
/// that pays an MCP startup.
pub async fn tasks(State(state): St) -> Response {
    // **`--closed`, because the page has a view for closed tasks.** The
    // drawer's `done` filter selects `done | dropped`, and without this the
    // list it filters never contains either — so that view has been
    // structurally empty since it shipped, in the way that reads as "you have
    // finished nothing" rather than as a filter that cannot match. No other
    // view widens: the three above it select `next|inbox`, `scheduled` and
    // `waiting`, none of which a closed task can be.
    let mut board = match self_json(&state, &["tasks", "list", "--closed", "--json"]).await {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    };
    if let Ok(dir) = mecha_core::session::Session::default_dir() {
        attach_runs(&mut board, &dir);
    }
    Json(board).into_response()
}

/// Put each task's run beside it, and **only where there was one**.
///
/// A task nobody ever handed to the agent gets no `run` key at all, rather
/// than an empty one. That absence is what the card reads as `idle`, and it
/// is the common case — most of a board is things the owner typed. An
/// object saying "no outcome recorded" on every hand-written task would put
/// every one of them into the state reserved for a delegation that broke.
fn attach_runs(board: &mut serde_json::Value, dir: &std::path::Path) {
    let Some(items) = board["items"].as_array_mut() else {
        return;
    };
    for task in items.iter_mut() {
        let Some(id) = task["session"].as_str().map(str::to_string) else {
            continue;
        };
        task["run"] = run_summary(dir, &id);
    }
}

/// How the run on a session went — or that nobody can say.
///
/// **Three answers, never two.** An outcome record says how the loop stopped;
/// its *absence* on a session that exists says the run did not get as far as
/// recording one — a crash, a kill, or a transcript written before the record
/// existed. That third answer is reported as itself and never folded into
/// either of the others: doctor's rule that a rate over a zero denominator is
/// `None` and never zero, one surface over, where the consequence of getting
/// it wrong is a delegation that died reading exactly like one nobody started
/// (or, in the other direction, every run from before this shipped shouting
/// that it failed).
///
/// The last outcome wins. A session is resumed by answering a question, and
/// what the owner is deciding about is where it stands now.
fn run_summary(dir: &std::path::Path, session_id: &str) -> serde_json::Value {
    let Ok(path) = mecha_core::session::Session::find(dir, session_id) else {
        // The transcript is gone. Not "the run failed" and not "it finished":
        // the record this answer is made of is missing.
        return serde_json::json!({ "recorded": false, "transcript": false });
    };
    let last = mecha_core::session::Session::outcomes(&path)
        .ok()
        .and_then(|mut o| o.pop());
    let Some(stats) = last else {
        return serde_json::json!({ "recorded": false, "transcript": true });
    };
    serde_json::json!({
        "recorded": true,
        "transcript": true,
        // `None` when the loop never named one; the page renders that as
        // unknown rather than as completion, for the reason above.
        "stop_cause": stats.stop_cause,
        // The harness cut it short — as distinct from the model finishing or
        // a person stopping it. One definition, `StopCause`'s own, because
        // there were two once and they disagreed.
        "cut_short": stats.stop_cause.is_some_and(|c| c.is_early()),
        // The silent failure: the model stopped of its own accord with its
        // last call failed and answered as though it had not. An observation
        // rather than an error — a task whose right answer is "that file does
        // not exist" ends this way legitimately — so the card shows it and
        // does not rule on it.
        "ended_on_failed_call": stats.ended_on_failed_call,
        // The evidence `ready for review` is supposed to arrive with.
        "turns": stats.turns,
        "tool_calls": stats.tool_calls,
        // Never `tool_errors` alone: a denial is the approver doing its job,
        // and averaging it into failure is how a read-only run reports the
        // harness working as a harness fault.
        "tool_errors": stats.tool_errors,
        "tool_denied": stats.tool_denied,
        "tool_staged": stats.tool_staged,
    })
}

#[derive(serde::Deserialize)]
pub struct TaskSetBody {
    pub task: String,
    pub status: Option<String>,
    pub due: Option<String>,
    pub defer: Option<String>,
    pub context: Option<String>,
}

/// POST /api/tasks/set — lifecycle and scheduling, through the one driver.
/// Omitted fields stay omitted: the unset-vs-clear distinction is the
/// tool's, and flattening it here would wipe due dates on status changes.
pub async fn task_set(State(state): St, Json(body): Json<TaskSetBody>) -> Response {
    let mut args: Vec<&str> = vec!["tasks", "set", &body.task];
    if let Some(v) = &body.status {
        args.extend(["--status", v]);
    }
    if let Some(v) = &body.due {
        args.extend(["--due", v]);
    }
    if let Some(v) = &body.defer {
        args.extend(["--defer", v]);
    }
    if let Some(v) = &body.context {
        args.extend(["--context", v]);
    }
    if args.len() == 3 {
        return (StatusCode::BAD_REQUEST, "nothing to change\n").into_response();
    }
    verb(&state, &args).await
}

#[derive(serde::Deserialize)]
pub struct TaskWorkBody {
    pub task: String,
    pub note: Option<String>,
}

/// POST /api/tasks/work — hand a task to the agent.
///
/// **Detached and unattended**, which are two decisions rather than one.
/// Detached because this is a whole agent run that can take twenty minutes,
/// and a request holding a connection open for that is a request that times
/// out — the mail page's drafting verbs spawn the same way, and the board is
/// the meeting point rather than the child.
///
/// Unattended because the approver in this process cannot reach a child
/// process: the web approval cards belong to a chat session's `RunContext`,
/// and a spawned `mecha tasks work` builds its own agent. So the run takes
/// the trigger posture — reads run, sends stage, anything needing approval is
/// refused — which is D3's rule exactly: *a run gets more permission by
/// acquiring a human, never by asking for one*. The way to give this run more
/// is to open its conversation, where `ask` mode's cards already work.
///
/// Nothing is returned but an acknowledgement. What happened is on the board:
/// `waiting_on` names the agent while the run is in flight and the owner when
/// it stops, so the page derives the card's state from the store rather than
/// from anything the run says about itself (D5, D16).
pub async fn task_work(State(state): St, Json(body): Json<TaskWorkBody>) -> Response {
    let _ = state; // the board is the meeting point, not this process
    let task = body.task.trim();
    if task.is_empty() {
        return (StatusCode::BAD_REQUEST, "which task?\n").into_response();
    }
    let mut argv: Vec<String> = vec![
        "tasks".into(),
        "work".into(),
        task.into(),
        "--unattended".into(),
    ];
    if let Some(note) = body
        .note
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        argv.push("--note".into());
        argv.push(note.into());
    }
    super::mail::spawn_detached_note(
        &argv,
        "handed to mecha — the board says who has it, and the conversation is on the card",
    )
}

#[derive(serde::Deserialize)]
pub struct TaskPlanBody {
    pub session: String,
}

/// POST /api/tasks/plan — the plan a task run wrote, read from its transcript.
///
/// **Not from the shared agent's todo tool**, and the difference is not an
/// implementation detail. A chat session runs *in this process*, so its plan
/// is in memory keyed by the session's jail. A `tasks work` run is a separate
/// process: nothing of its list is here, and asking the tool would answer
/// with an empty list for every task on the board.
///
/// The transcript is the meeting point, exactly as it is for the board and
/// the outbox — and `TodoTool::from_transcript` already knows how to read a
/// plan back out of one, including the compacted case where the `todo` calls
/// are gone and only the carried block survives. Built for resuming (D15),
/// reused here without change.
pub async fn task_plan(State(state): St, Json(body): Json<TaskPlanBody>) -> Response {
    let _ = state;
    let id = body.session.trim();
    if id.is_empty() {
        return (StatusCode::BAD_REQUEST, "which session?\n").into_response();
    }
    let found = mecha_core::session::Session::default_dir()
        .and_then(|dir| mecha_core::session::Session::find(&dir, id))
        .and_then(|path| mecha_core::session::Session::load(&path));
    match found {
        Ok((_, convo)) => Json(serde_json::json!({
            "todo": mecha_core::tool::todo::TodoTool::from_transcript(&convo.messages)
                .unwrap_or_default(),
        }))
        .into_response(),
        // A session that cannot be read is not a task with no plan. The page
        // shows nothing either way, but saying which is which keeps a broken
        // store from reading as a quiet one.
        Err(e) => (StatusCode::NOT_FOUND, format!("{e:#}\n")).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct TaskSourceBody {
    pub task: String,
}

/// POST /api/tasks/source — what the task was captured from, in full.
///
/// **The pointer is on the board and the bytes come from the source**, which
/// is the same split `/api/outbox/{id}` uses for a draft: ids on the wire,
/// content from the store, because a reviewer reading one thing while acting
/// on another is what these surfaces exist to prevent. The page already holds
/// `captured_from` from `/api/tasks` — enough to decide whether to *offer* a
/// way back — and asks here only when somebody actually opens it, so a board
/// of twenty tasks does not fetch twenty mail threads.
///
/// Through the CLI like every other verb here, so the one reader per kind is
/// `mecha tasks source`'s and the page cannot drift from the terminal. Its
/// budget is `self_text`'s minutes rather than seconds: following a mail
/// pointer pays an MCP startup and may refresh an OAuth token.
///
/// **The answer is third-party text and reaches the page as text**, never as
/// anything a run reads back. Nothing here re-enters a prompt and no taint
/// moves: these bytes were accounted for when the mail was first read.
pub async fn task_source(State(state): St, Json(body): Json<TaskSourceBody>) -> Response {
    let task = body.task.trim();
    if task.is_empty() {
        return (StatusCode::BAD_REQUEST, "which task?\n").into_response();
    }
    match super::mail::self_text(&state, &["tasks", "source", task]).await {
        Ok(text) => text.into_response(),
        // A source that cannot be read is not a task without one. The card
        // says which, because a broken pointer and a hand-typed task look
        // identical from a blank panel.
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct TaskStopBody {
    pub task: String,
}

/// POST /api/tasks/stop — ask the run working a task to stop.
///
/// Synchronous, unlike starting one: writing a sentinel is instant, and the
/// answer worth having is whether there was anything to stop. `tasks stop`
/// says so rather than pretending, and that distinction has to survive the
/// wire — a page told "ok" for a stop that stopped nothing will show the run
/// as ended while it carries on.
///
/// It does not kill anything. The run finishes its current step and keeps the
/// partial turn, exactly as Ctrl-C does in a terminal — Copilot's documented
/// recovery for a stuck session is to unassign and reassign, and there is no
/// excuse for reproducing that.
pub async fn task_stop(State(state): St, Json(body): Json<TaskStopBody>) -> Response {
    let task = body.task.trim();
    if task.is_empty() {
        return (StatusCode::BAD_REQUEST, "which task?\n").into_response();
    }
    verb(&state, &["tasks", "stop", task]).await
}

#[derive(serde::Deserialize)]
pub struct TaskAddBody {
    pub name: String,
    pub due: Option<String>,
    pub context: Option<String>,
}

/// POST /api/tasks/add — capture; lands in `inbox`, committed to nothing.
pub async fn task_add(State(state): St, Json(body): Json<TaskAddBody>) -> Response {
    let name = body.name.trim();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, "a task needs a name\n").into_response();
    }
    let mut args: Vec<&str> = vec!["tasks", "add", name];
    if let Some(v) = &body.due {
        args.extend(["--due", v]);
    }
    if let Some(v) = &body.context {
        args.extend(["--context", v]);
    }
    verb(&state, &args).await
}

#[derive(serde::Deserialize)]
pub struct NoteBody {
    pub text: String,
}

/// POST /api/notes — the owner's own words, staged to the graph as an
/// episode (`kg note`); entities named in it are linked on landing.
///
/// `--` before the text because a note is prose and prose can start with a
/// dash; without it clap reads the owner's first word as a flag it does not
/// have and refuses the capture.
pub async fn note(State(state): St, Json(body): Json<NoteBody>) -> Response {
    let text = body.text.trim();
    if text.is_empty() {
        return (StatusCode::BAD_REQUEST, "an empty note\n").into_response();
    }
    verb(&state, &["kg", "note", "--", text]).await
}

#[derive(serde::Deserialize)]
pub struct NoteEditBody {
    /// The episode key from a `kg_notes` row — not the `uid`, which names
    /// the row but cannot write to it.
    pub source_id: String,
    pub text: String,
}

/// POST /api/notes/edit — rewrite one note in place.
///
/// The whole edit lives in `mecha kg note --edit`, including the part that
/// matters (the note's original `occurred_at` is read back and preserved, so
/// fixing a typo does not restamp when the thing happened). This handler
/// only checks that both halves are present: a note page that could reach a
/// rewrite the terminal cannot is the shape `/tasks` and `/triggers` both
/// refuse.
pub async fn note_edit(State(state): St, Json(body): Json<NoteEditBody>) -> Response {
    let id = body.source_id.trim();
    let text = body.text.trim();
    if id.is_empty() {
        return (StatusCode::BAD_REQUEST, "which note?\n").into_response();
    }
    if text.is_empty() {
        // Emptying a note is not editing it, and there is deliberately no
        // delete here: a note is evidence, and what the graph derived from it
        // is retracted in review, not by blanking the record.
        return (
            StatusCode::BAD_REQUEST,
            "an empty note — clear it in review instead\n",
        )
            .into_response();
    }
    verb(&state, &["kg", "note", "--edit", id, "--", text]).await
}

#[derive(serde::Deserialize)]
pub struct FindQuery {
    pub q: String,
}

/// GET /api/find?q= — `kg search` (the TUI /find verb), for the notes page's search box.
pub async fn find(
    State(state): St,
    axum::extract::Query(query): axum::extract::Query<FindQuery>,
) -> Response {
    let q = query.q.trim();
    if q.is_empty() {
        return Json(serde_json::json!([])).into_response();
    }
    match self_json(&state, &["kg", "search", q, "--json"]).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    }
}

/// GET /api/notes — recent notes, the `kg_notes` envelope verbatim.
pub async fn notes(State(state): St) -> Response {
    match self_json(&state, &["kg", "notes", "--json"]).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::agent::StopCause;
    use mecha_core::session::{Record, RunStats, Session, SessionMeta};

    /// A private directory per test, on this repo's own convention — no
    /// `tempfile` dependency in the binary that reads the owner's stores.
    fn tmpdir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mecha-board-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn session(dir: &std::path::Path, id: &str) -> Session {
        Session::create(
            dir,
            SessionMeta {
                id: id.into(),
                created_at: chrono::Utc::now(),
                provider: "local".into(),
                model: "a-model".into(),
                workspace: dir.to_path_buf(),
                title: Some("task: something".into()),
            },
        )
        .unwrap()
    }

    /// **The third answer, and the reason it exists.** A transcript with no
    /// outcome record is a run that never got as far as saying how it went —
    /// a crash, a kill, or a session written before the record existed. It is
    /// not a failure and it is not a completion, and folding it into either
    /// is how every delegation from before this shipped would shout that it
    /// broke (or how one that really did break would read as idle, which is
    /// the rule D16 states outright).
    #[test]
    fn a_run_that_recorded_no_outcome_says_so_rather_than_guessing() {
        let dir = tmpdir("unrecorded");
        session(&dir, "20260826T090000-aaaaaaaa");
        let v = run_summary(&dir, "20260826T090000-aaaaaaaa");
        assert_eq!(v["transcript"], serde_json::json!(true));
        assert_eq!(v["recorded"], serde_json::json!(false));
        assert!(
            v["stop_cause"].is_null(),
            "and it must not invent one — unknown is the answer"
        );
    }

    /// A session the board names that is not on disk is a third thing again:
    /// the record this answer would be made of is missing.
    #[test]
    fn a_missing_transcript_is_not_a_failed_run() {
        let dir = tmpdir("missing");
        let v = run_summary(&dir, "20260826T090000-nosuch01");
        assert_eq!(v["transcript"], serde_json::json!(false));
        assert_eq!(v["recorded"], serde_json::json!(false));
    }

    /// The state D16 says must never render as `idle`. `NoOutput` is a real
    /// failure the loop names — the run produced nothing and did not recover
    /// — and the card has to be able to tell it from a completion.
    #[test]
    fn a_run_the_harness_cut_short_is_distinguishable_from_one_that_finished() {
        let dir = tmpdir("cutshort");
        let s = session(&dir, "20260826T090000-bbbbbbbb");
        s.append(&Record::Outcome(RunStats {
            stop_cause: Some(StopCause::NoOutput),
            tool_calls: 3,
            ..RunStats::default()
        }))
        .unwrap();
        let v = run_summary(&dir, "20260826T090000-bbbbbbbb");
        assert_eq!(v["recorded"], serde_json::json!(true));
        assert_eq!(v["cut_short"], serde_json::json!(true));

        let s = session(&dir, "20260826T090000-cccccccc");
        s.append(&Record::Outcome(RunStats {
            stop_cause: Some(StopCause::Completed),
            tool_calls: 3,
            tool_staged: 1,
            ..RunStats::default()
        }))
        .unwrap();
        let v = run_summary(&dir, "20260826T090000-cccccccc");
        assert_eq!(v["cut_short"], serde_json::json!(false));
        assert_eq!(v["tool_staged"], serde_json::json!(1));
    }

    /// **A task nobody delegated has no run, not an empty one.** The card
    /// reads the key's absence as `idle`, and most of a board is things the
    /// owner typed — so a summary attached to every row would put every
    /// hand-written task into the state reserved for a delegation that broke.
    #[test]
    fn a_task_that_was_never_delegated_gets_no_run_at_all() {
        let dir = tmpdir("never");
        let mut board = serde_json::json!({
            "items": [
                { "id": "task-1", "session": null },
                { "id": "task-2" },
                { "id": "task-3", "session": "20260826T090000-eeeeeeee" },
            ]
        });
        session(&dir, "20260826T090000-eeeeeeee");
        attach_runs(&mut board, &dir);
        assert!(board["items"][0].get("run").is_none());
        assert!(board["items"][1].get("run").is_none());
        assert_eq!(
            board["items"][2]["run"]["transcript"],
            serde_json::json!(true),
            "and the one that was delegated is answered for"
        );
    }

    /// **The last outcome wins.** A session is resumed by answering a
    /// question, so a task that asked, was answered and then finished must
    /// not still be reported by the run that stopped to ask — which is the
    /// state the owner already dealt with.
    #[test]
    fn a_resumed_session_is_reported_by_where_it_stands_now() {
        let dir = tmpdir("resumed");
        let s = session(&dir, "20260826T090000-dddddddd");
        s.append(&Record::Outcome(RunStats {
            stop_cause: Some(StopCause::Interrupted),
            ..RunStats::default()
        }))
        .unwrap();
        s.append(&Record::Outcome(RunStats {
            stop_cause: Some(StopCause::Completed),
            turns: 9,
            ..RunStats::default()
        }))
        .unwrap();
        let v = run_summary(&dir, "20260826T090000-dddddddd");
        assert_eq!(v["cut_short"], serde_json::json!(false));
        assert_eq!(v["turns"], serde_json::json!(9));
    }
}
