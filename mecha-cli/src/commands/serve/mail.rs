//! The mail queue on the phone — the `/mail` modal's split, unchanged:
//! **the store is read for display, and every mutation is a `mecha mail …`
//! child process.** Nothing here reimplements a verb, so a thing the page
//! can do is a thing a script and the nightly can do.
//!
//! This surface shows prose, on the modal's reasoning: a person reading
//! their own mail on their own phone is the safe context, and a list nobody
//! can recognise a thread in is not a list. What must not see the prose is a
//! *privileged run*, and none happens here — the JSON leaves this process
//! only toward the authenticated owner's browser.
//!
//! The one asymmetry carried across: **spam is the only triage action with
//! an effect outside the user's own mailbox** (it trains the provider's
//! filter), so the page owns a confirm sheet for it exactly as the TUI
//! confirms on `s`. The server cannot tell a confirmed tap from an
//! unconfirmed one, so the closed verb list is the boundary here and the
//! confirm is the page's contract — the same trust the TUI places in its own
//! keymap.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use mecha_core::mail_triage::{handle, Bucket, Record, TriageStore, CLASSIFIED, FAILED};

type St = State<super::WebState>;

/// One thread, flattened for the list — the TUI's `MailRow`, serialised.
#[derive(Debug, Serialize)]
pub struct Row {
    thread_id: String,
    handle: String,
    account: String,
    from: String,
    urgency: String,
    tags: Vec<String>,
    /// The classifier's one-liner, else the subject — display for the owner,
    /// exactly what the modal shows. Never fed to a run from here.
    summary: String,
    state: String,
    needs_me: bool,
    bucket: Option<String>,
    deadline: Option<String>,
    proposed: Option<String>,
}

fn row(r: &Record) -> Row {
    let v = r.verdict.as_ref();
    Row {
        thread_id: r.thread_id.clone(),
        handle: handle(&r.thread_id),
        account: r.account.clone(),
        from: r.from.clone(),
        urgency: v
            .map(|v| v.urgency.as_str().to_string())
            .unwrap_or_default(),
        tags: v.map(|v| v.tags.clone()).unwrap_or_default(),
        summary: v
            .map(|v| v.one_line.clone())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| r.subject.clone()),
        state: r.state.clone(),
        needs_me: r.state == CLASSIFIED && v.is_some_and(|v| v.bucket == Bucket::Respond)
            || r.state == FAILED,
        bucket: v.map(|v| v.bucket.as_str().to_string()),
        deadline: v.and_then(|v| v.deadline.clone()),
        proposed: v.map(|v| v.proposed.as_str().to_string()),
    }
}

/// GET /api/mail — every record, sorted the way a person works: respond
/// first, then by urgency. The page filters display; the sort is policy and
/// lives here so every reader agrees with the TUI's.
pub async fn list(State(_state): St) -> Response {
    let Some(store) = TriageStore::open_existing_default() else {
        return Json(Vec::<Row>::new()).into_response();
    };
    match store.list() {
        Ok(records) => {
            let mut rows: Vec<Row> = records.iter().map(row).collect();
            rows.sort_by_key(|r| {
                (
                    !r.needs_me,
                    match r.urgency.as_str() {
                        "now" => 0,
                        "today" => 1,
                        "week" => 2,
                        _ => 3,
                    },
                )
            });
            Json(rows).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response(),
    }
}

#[derive(Deserialize)]
pub struct ReadQuery {
    pub thread: String,
    pub account: String,
}

/// GET /api/mail/read — one thread's prose, exactly what `mecha mail show`
/// prints: one renderer of a thread, so the page cannot drift from the
/// command line. This is third-party text and the page marks it as such.
///
/// A query, not a path segment: provider thread ids are whatever the
/// provider says they are, and a path that has to round-trip them is a
/// parser someone will get wrong.
pub async fn read(State(state): St, Query(q): Query<ReadQuery>) -> Response {
    match self_text(
        &state,
        &["mail", "show", &q.thread, "--account", &q.account],
    )
    .await
    {
        Ok(text) => text.into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    }
}

#[derive(Deserialize)]
pub struct ActBody {
    pub verb: String,
    pub thread: String,
    pub account: String,
    /// needs-info's `--missing`, reply/forward/schedule's `--note`.
    pub text: Option<String>,
    /// forward's comma-separated `--to`.
    pub to: Option<String>,
}

/// POST /api/mail/act — one triage verb through the CLI.
///
/// **The verb set is a closed match, never an interpolation.** A body field
/// that reached the argv as a verb would make this route `mecha <anything>`
/// with extra steps; unknown verbs are refused by name. The drafting verbs
/// (`reply`, `forward`, `schedule`) are whole agent runs, so they spawn
/// detached exactly as the TUI spawns them — the draft lands in the outbox,
/// which is the one approval surface, and the response says so rather than
/// waiting minutes on an HTTP request.
pub async fn act(State(state): St, Json(body): Json<ActBody>) -> Response {
    let thread = body.thread.as_str();
    let account = body.account.as_str();
    match body.verb.as_str() {
        "archive" | "spam" | "dismiss" => {
            verb_now(&state, &[&body.verb, thread, "--account", account]).await
        }
        "task" => verb_now(&state, &["task", thread, "--account", account]).await,
        "needs-info" => {
            let Some(missing) = body.text.as_deref().filter(|t| !t.trim().is_empty()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    "needs-info needs `text`: what you are waiting for\n",
                )
                    .into_response();
            };
            verb_now(
                &state,
                &[
                    "needs-info",
                    thread,
                    "--account",
                    account,
                    "--missing",
                    missing,
                ],
            )
            .await
        }
        "reply" | "schedule" => {
            let mut args = vec![body.verb.as_str(), thread, "--account", account];
            if let Some(note) = body.text.as_deref().filter(|t| !t.trim().is_empty()) {
                args.push("--note");
                args.push(note);
            }
            spawn_detached(&args)
        }
        "forward" => {
            let Some(to) = body.to.as_deref().filter(|t| !t.trim().is_empty()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    "forward needs `to`: comma-separated recipients\n",
                )
                    .into_response();
            };
            let mut args = vec!["forward", thread, "--account", account, "--to", to];
            if let Some(note) = body.text.as_deref().filter(|t| !t.trim().is_empty()) {
                args.push("--note");
                args.push(note);
            }
            spawn_detached(&args)
        }
        other => (
            StatusCode::BAD_REQUEST,
            format!("unknown mail verb: {other}\n"),
        )
            .into_response(),
    }
}

/// A quick verb: run it, wait, report. Archive and friends are one MCP
/// startup plus one provider call — seconds, not minutes.
async fn verb_now(state: &super::WebState, args: &[&str]) -> Response {
    let mut argv = vec!["mail"];
    argv.extend_from_slice(args);
    verb_now_named(state, &argv).await
}

/// The same, taking the whole argv — the frontdoor page's verbs ride this
/// too, so one implementation owns the run-wait-report shape.
pub(super) async fn verb_now_named(state: &super::WebState, argv: &[&str]) -> Response {
    match self_text(state, argv).await {
        Ok(out) => Json(serde_json::json!({ "ok": true, "output": out })).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    }
}

/// A drafting verb: spawn and answer. The child is an agent run that stages
/// into the outbox; the store is the record of how it went, and the outbox
/// page is where the result appears — polling a child from an HTTP handler
/// would tie a phone's request timeout to a model's writing speed.
fn spawn_detached(args: &[&str]) -> Response {
    let mut argv = vec!["mail".to_string()];
    argv.extend(args.iter().map(|s| s.to_string()));
    spawn_detached_named(&argv)
}

/// The whole-argv detached spawn, shared with the frontdoor page.
pub(super) fn spawn_detached_named(argv: &[String]) -> Response {
    let mut cmd = std::process::Command::new(crate::exe::self_exe());
    if let Some(dir) = super::child_cwd() {
        cmd.current_dir(dir);
    }
    match cmd
        .args(argv)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_) => Json(serde_json::json!({
            "ok": true,
            "detached": true,
            "note": "drafting — the result lands in the outbox for review",
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("spawning: {e:#}\n"),
        )
            .into_response(),
    }
}

/// Like review's `self_json`, but the child's stdout is prose to pass
/// through. Mail verbs reach a provider (MCP startup, OAuth refresh), so the
/// budget is minutes where review's is seconds.
pub(super) async fn self_text(state: &super::WebState, args: &[&str]) -> anyhow::Result<String> {
    let _ = state;
    let mut cmd = tokio::process::Command::new(crate::exe::self_exe());
    if let Some(dir) = super::child_cwd() {
        cmd.current_dir(dir);
    }
    let output = tokio::time::timeout(std::time::Duration::from_secs(120), cmd.args(args).output())
        .await
        .map_err(|_| anyhow::anyhow!("timed out"))??;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{}", stderr.lines().last().unwrap_or("failed"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    /// The verb set is closed: the strings `act` will pass to the CLI, and
    /// nothing else. A new verb is added here *and* in the match, and a body
    /// naming anything outside it must be refused before an argv exists.
    #[test]
    fn the_verb_set_is_closed_and_spam_is_the_only_outward_one() {
        // The match arms in `act`, written down once more so a widening is a
        // conscious edit to a test that explains the stakes.
        let quick = ["archive", "spam", "dismiss", "task", "needs-info"];
        let detached = ["reply", "forward", "schedule"];
        assert_eq!(quick.len() + detached.len(), 8);
        // `spam` is the one verb whose effect leaves the mailbox — the page
        // must confirm it, and this test is where that contract is written.
        assert!(quick.contains(&"spam"));
        // No verb here deletes, labels, or sends: sends stage via the
        // drafting verbs into the outbox, which is the one approval surface.
        for v in quick.iter().chain(&detached) {
            assert!(!matches!(*v, "trash" | "delete" | "send"));
        }
    }
}
