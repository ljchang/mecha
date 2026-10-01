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
fn sessions_dir(state: &super::WebState) -> Result<std::path::PathBuf, axum::response::Response> {
    // The chat door's own directory where there is one — the rail these
    // routes act on lists it, and a test's door keeps its own (#471) — and
    // the configured one where `serve` runs without a chat.
    if let Ok(chat) = chat_state(state) {
        return Ok(chat.sessions_dir().to_path_buf());
    }
    Session::default_dir()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response())
}

/// Release `id` from the process, or answer why not (a 409's reason).
///
/// Two writers to ask, as resume asks them: this process's open chats, and a
/// detached task run in another process — whose conversation this map cannot
/// see, and whose workspace a delete would remove mid-tool-call.
async fn release(state: &super::WebState, id: &str) -> Result<(), String> {
    if let Some(task) = crate::commands::tasks::detached_writer(id).map_err(|e| format!("{e:#}"))? {
        return Err(format!(
            "a run is working {task} in this conversation — stop it first \
             (`mecha tasks stop {task}`)"
        ));
    }
    // No chat subsystem means no open conversations to let go of; archiving
    // and deleting the record still work.
    let Ok(chat) = chat_state(state) else {
        return Ok(());
    };
    chat.release_recorded(id)
        .await
        .map(|_| ())
        .map_err(str::to_string)
}

fn busy(why: &str) -> axum::response::Response {
    (StatusCode::CONFLICT, format!("{why}\n")).into_response()
}

/// POST /api/sessions/{id}/archive — out of the list, still on the record.
pub async fn archive(State(state): Web, Path(id): Path<String>) -> axum::response::Response {
    let dir = match sessions_dir(&state) {
        Ok(d) => d,
        Err(r) => return r,
    };
    if !dir.join(format!("{id}.jsonl")).is_file() {
        return (StatusCode::NOT_FOUND, "no such conversation\n").into_response();
    }
    if let Err(why) = release(&state, &id).await {
        return busy(&why);
    }
    match mecha_core::archive::archive(&dir, &id, chrono::Utc::now()) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("{e:#}\n")).into_response(),
    }
}

/// POST /api/sessions/{id}/unarchive — back in the list.
pub async fn unarchive(State(state): Web, Path(id): Path<String>) -> axum::response::Response {
    let dir = match sessions_dir(&state) {
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
    let dir = match sessions_dir(&state) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let known = dir.join(format!("{id}.jsonl")).is_file()
        || dir.join(format!("{id}.jsonl.forgetting")).is_file();
    if !known {
        return (StatusCode::NOT_FOUND, "no such conversation\n").into_response();
    }
    if let Err(why) = release(&state, &id).await {
        return busy(&why);
    }
    // Where this process's chats actually staged their drafts, when there is
    // a chat subsystem — it resolved `[outbox] dir` at start.
    let outbox = chat_state(&state)
        .ok()
        .map(|c| c.outbox_root().to_path_buf());
    // The home this door's workspaces sit under (`<home>/work/web`):
    // `forget` purges a workspace only under `home/work`, so a door rooted
    // elsewhere would keep its files (review of #471). In `serve` it is
    // `~/.mecha`, as before.
    let door_home = chat_state(&state).ok().and_then(|c| {
        c.work_dir()
            .parent()
            .and_then(|work| work.parent())
            .map(std::path::Path::to_path_buf)
    });
    let forget_dir = dir.clone();
    // Files and a child process: off the async workers.
    let forgot = tokio::task::spawn_blocking(move || {
        let mut roots =
            mecha_core::forget::Roots::from_config(&mecha_core::config::Config::load_global()?)?;
        if let Some(outbox) = outbox {
            roots.outbox = outbox;
        }
        // The transcripts it forgets are the ones this door lists.
        roots.sessions = forget_dir;
        if let Some(home) = door_home {
            roots.home = home;
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
