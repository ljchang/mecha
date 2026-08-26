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

/// GET /api/tasks — the board, straight off `kg_task_list`'s own JSON.
/// The child pays an MCP startup (~a second against the graph server), which
/// is why the timeout is generous and the page shows its own loading state.
pub async fn tasks(State(state): St) -> Response {
    match self_json(&state, &["tasks", "list", "--json"]).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    }
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
