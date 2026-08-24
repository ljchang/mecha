//! The front door on the phone — the last review store without a page.
//!
//! The mail page's split, unchanged: the list is a store read, the prose is
//! `mecha frontdoor show`'s exact text (a person reading a stranger's
//! request on their own phone is the safe context — the same reasoning as
//! the terminal), and every mutation is a closed-verb CLI child. The
//! privileged-run boundary is untouched from here: nothing this page shows
//! ever re-enters a run, and the extraction stays the only representation a
//! run is given.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use mecha_core::frontdoor::Frontdoor;

type St = State<super::WebState>;

/// GET /api/frontdoor — every request, newest first. Typed fields plus the
/// extraction's own summary lines; the raw prose stays behind the read
/// endpoint, where opening it is a person's explicit act.
pub async fn list(State(_state): St) -> Response {
    let store = match Frontdoor::open_default() {
        Ok(s) => s,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response(),
    };
    let mut records = match store.records() {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response(),
    };
    records.sort_by_key(|r| std::cmp::Reverse(r.seq));
    let rows: Vec<serde_json::Value> = records
        .iter()
        .map(|r| {
            serde_json::json!({
                "seq": r.seq,
                "type_id": r.type_id,
                "state": r.state,
                "created_at": r.created_at,
                "valid": r.valid,
                "invalid_reason": r.invalid_reason,
                "topic": r.extraction.as_ref().map(|x| x.topic.clone()),
                "reading": r.extraction.as_ref().map(|x| x.reading.clone()),
                "urgency_claimed": r.extraction.as_ref().map(|x| x.urgency_claimed.clone()),
                "extraction_error": r.extraction_error,
            })
        })
        .collect();
    Json(serde_json::json!({ "requests": rows })).into_response()
}

#[derive(Deserialize)]
pub struct ReadQuery {
    pub seq: i64,
}

/// GET /api/frontdoor/read — one request in full, including the stranger's
/// prose, exactly as `mecha frontdoor show` prints it: one renderer, and
/// the page marks it as third-party text.
pub async fn read(State(state): St, Query(q): Query<ReadQuery>) -> Response {
    let seq = q.seq.to_string();
    match super::mail::self_text(&state, &["frontdoor", "show", &seq]).await {
        Ok(text) => text.into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    }
}

#[derive(Deserialize)]
pub struct ActBody {
    pub verb: String,
    pub seq: i64,
    /// close's `--reason`, needs-info's `--note`.
    pub text: Option<String>,
}

/// POST /api/frontdoor/act — one verb through the CLI, closed-matched.
/// `extract` is the quarantined tool-less pass (seconds); `triage` is a
/// whole agent run and spawns detached — its drafts land in the outbox,
/// which stays the one approval surface.
pub async fn act(State(state): St, Json(body): Json<ActBody>) -> Response {
    let seq = body.seq.to_string();
    match body.verb.as_str() {
        "extract" => {
            super::mail::verb_now_named(&state, &["frontdoor", "extract", "--seq", &seq]).await
        }
        "needs-info" => {
            let mut args = vec!["frontdoor", "needs-info", &seq];
            if let Some(note) = body.text.as_deref().filter(|t| !t.trim().is_empty()) {
                args.push("--note");
                args.push(note);
            }
            super::mail::verb_now_named(&state, &args).await
        }
        "close" => {
            let Some(reason) = body.text.as_deref().filter(|t| !t.trim().is_empty()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    "close needs `text`: the reason is the record\n",
                )
                    .into_response();
            };
            super::mail::verb_now_named(&state, &["frontdoor", "close", &seq, "--reason", reason])
                .await
        }
        "triage" => super::mail::spawn_detached_named(&[
            "frontdoor".to_string(),
            "triage".to_string(),
            "--seq".to_string(),
            seq,
        ]),
        other => (
            StatusCode::BAD_REQUEST,
            format!("unknown frontdoor verb: {other}\n"),
        )
            .into_response(),
    }
}
