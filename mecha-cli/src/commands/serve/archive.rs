//! The drawer's two ways to put a conversation away: archive (filed, still
//! on the record — `mecha_core::archive`) and delete (gone, with everything
//! derived from it — `mecha_core::forget`).
//!
//! Both take the transcript's id, never a live key: an archived or earlier
//! conversation has no key, and an open one's key is an address this process
//! happens to hold it under. Both let go of an open conversation first
//! (`ChatState::release_recorded`), so it leaves the rail and this process
//! never appends to it again — and both refuse while a run is in flight,
//! because the run owns the conversation until it ends.
//!
//! Behind `owner_guard` like every `/api` route, and no tool reaches either:
//! forgetting is the owner's act on the owner's surface.

use super::chat::chat_state;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use mecha_core::session::Session;

type Web = State<super::WebState>;

// The Err arm carries a whole `Response`, built once per refused request —
// `chat::chat_state`'s reasoning.
#[allow(clippy::result_large_err)]
fn sessions_dir() -> Result<std::path::PathBuf, axum::response::Response> {
    Session::default_dir()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response())
}

/// Release `id` from the process, or answer why not (a 409's reason).
async fn release(state: &super::WebState, id: &str) -> Result<(), &'static str> {
    // No chat subsystem means no open conversations to let go of; archiving
    // and deleting the record still work.
    let Ok(chat) = chat_state(state) else {
        return Ok(());
    };
    chat.release_recorded(id).await.map(|_| ())
}

fn busy(why: &str) -> axum::response::Response {
    (StatusCode::CONFLICT, format!("{why}\n")).into_response()
}

/// POST /api/sessions/{id}/archive — out of the list, still on the record.
pub async fn archive(State(state): Web, Path(id): Path<String>) -> axum::response::Response {
    let dir = match sessions_dir() {
        Ok(d) => d,
        Err(r) => return r,
    };
    if !dir.join(format!("{id}.jsonl")).is_file() {
        return (StatusCode::NOT_FOUND, "no such conversation\n").into_response();
    }
    if let Err(why) = release(&state, &id).await {
        return busy(why);
    }
    match mecha_core::archive::archive(&dir, &id, chrono::Utc::now()) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("{e:#}\n")).into_response(),
    }
}

/// POST /api/sessions/{id}/unarchive — back in the list.
pub async fn unarchive(Path(id): Path<String>) -> axum::response::Response {
    let dir = match sessions_dir() {
        Ok(d) => d,
        Err(r) => return r,
    };
    match mecha_core::archive::unarchive(&dir, &id) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("{e:#}\n")).into_response(),
    }
}

/// DELETE /api/sessions/{id} — forget the conversation everywhere. Answers
/// the report: `complete: false` with the stores that failed, which the page
/// shows rather than pretending — pressing delete again finishes it.
pub async fn delete(State(state): Web, Path(id): Path<String>) -> axum::response::Response {
    let dir = match sessions_dir() {
        Ok(d) => d,
        Err(r) => return r,
    };
    let known = dir.join(format!("{id}.jsonl")).is_file()
        || dir.join(format!("{id}.jsonl.forgetting")).is_file();
    if !known {
        return (StatusCode::NOT_FOUND, "no such conversation\n").into_response();
    }
    if let Err(why) = release(&state, &id).await {
        return busy(why);
    }
    // Where this process's chats actually staged their drafts, when there is
    // a chat subsystem — it resolved `[outbox] dir` at start.
    let outbox = chat_state(&state)
        .ok()
        .map(|c| c.outbox_root().to_path_buf());
    // Files and a child process: off the async workers.
    let forgot = tokio::task::spawn_blocking(move || {
        let mut roots =
            mecha_core::forget::Roots::from_config(&mecha_core::config::Config::load_global()?)?;
        if let Some(outbox) = outbox {
            roots.outbox = outbox;
        }
        mecha_core::forget::forget(&roots, &id, &crate::commands::sessions::GraphCli)
    })
    .await;
    match forgot {
        Ok(Ok(report)) => Json(report).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, format!("{e:#}\n")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}\n")).into_response(),
    }
}
