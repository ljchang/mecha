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
pub async fn note(State(state): St, Json(body): Json<NoteBody>) -> Response {
    let text = body.text.trim();
    if text.is_empty() {
        return (StatusCode::BAD_REQUEST, "an empty note\n").into_response();
    }
    verb(&state, &["kg", "note", text]).await
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
