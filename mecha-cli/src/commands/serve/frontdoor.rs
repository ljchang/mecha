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
    // Reconcile, then settle — the terminal verbs' order, and it is
    // load-bearing: a booking triaged before any of this existed sits in
    // `awaiting_me`, which settling refuses to touch, so reconcile has to lift
    // it out first or the migration takes two page loads. This page said it
    // settled "exactly as the terminal verbs do" while doing only half of it.
    //
    // Without settling at all the page reads a store nobody reconciled: a
    // confirmed booking sits in the queue behind an **Extract** button whose
    // child prints `nothing to extract` and exits 0, so the page reports
    // success for work that did not happen.
    //
    // Both best-effort — a store this cannot write to is still worth showing.
    if let Some(outbox) = mecha_core::outbox::OutboxStore::open_existing_default() {
        if let Err(e) = store.reconcile(&outbox) {
            tracing::warn!(error = %e, "reconciling front-door drafts for the page");
        }
    }
    if let Err(e) = store.settle_bookings(&super::super::frontdoor::swept_bookings()) {
        tracing::warn!(error = %e, "settling bookings for the front door page");
    }
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
                // The meeting, when this record is one — facts only. Stamps
                // go over as RFC 3339 and the page renders them in the
                // *viewer's* zone: the phone in your hand knows where it is,
                // and this page is only ever read by its owner.
                //
                // Deliberately no `settled` flag. The page needs to know
                // whether this record owes anybody anything, and that is the
                // **state**, not the policy predicate — `is_settled_booking()`
                // stays true for a booking a person later closed by hand with
                // a reason, which would have filed it under "nothing owed"
                // beside a `closed` chip. `show` gates that same sentence on
                // `state == BOOKED`, and now so does the page.
                "booking": r.booking().map(|b| serde_json::json!({
                    "start": b.start,
                    "end": b.end,
                    "duration_minutes": b.duration_minutes,
                    // No `manage_url`: the page renders it nowhere, and the
                    // read endpoint is where `show` already prints it. It is
                    // a capability URL the owner holds anyway, so shipping it
                    // unread is untidy rather than unsafe — but a field with
                    // no reader is one nobody notices growing a reader.
                })),
                // Prose is never in the list payload — it opens only through
                // the read endpoint, which is a person's explicit act. The
                // requester's address is not prose: it is the value the box
                // proved a stranger controls, and a booking row that cannot
                // say who is coming is the card this change exists to fix.
                // Only where the card reads it — inside the booking branch,
                // which is the one row that says who is coming. Shipping it on
                // every record made it a field with no reader, which is exactly
                // what the `manage_url` note above declines to do.
                "reply_to": r.booking().and(r.reply_to.clone()),
                // Whether the CLI's verbs will refuse this record — a
                // different question from "is it `booked`", and the two
                // disagree for a collided booking (permanently `drained`) and
                // for anything inside the drain→sweep window. The page gates
                // its **Extract** button on this, because gating it on the
                // state left the button live on exactly those records, where
                // the child prints `nothing to extract` and exits 0 and the
                // page reports success for work that did not happen.
                "inert": super::super::frontdoor::inert(r),
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
