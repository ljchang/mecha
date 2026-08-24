//! Phase 3 of `mecha serve`: the outbox on the phone.
//!
//! The TUI-modal split, unchanged: **store reads for display, every mutation
//! a `mecha …` child process** — `outbox approve --yes` (the page owns the
//! confirmation, exactly as the TUI modal does before it shells out),
//! `outbox reject --reason`, `outbox edit --body-file`. One implementation
//! per verb; nothing reachable from a browser that a script cannot do.
//!
//! The reviewable-object rules travel with the surface: the detail endpoint
//! returns the whole `DraftView` (headers, prose, everything-else — nothing
//! dropped), the source reads the draft answers (third-party text, marked as
//! such), the taint snapshot, and the exact arguments — because approving
//! without reading is the failure this queue exists to prevent, now on the
//! device where people read least carefully.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use mecha_core::outbox::{DraftView, OutboxItem, OutboxKind, OutboxStore};
use mecha_core::{outbox_source, session::Session};

pub struct ReviewState {
    pub outbox_root: PathBuf,
    pub sessions_dir: Option<PathBuf>,
}

type St = State<super::WebState>;

#[derive(Debug, Clone, Serialize)]
pub struct Row {
    id: String,
    tool: String,
    kind: String,
    summary: String,
    status: String,
    created_at: String,
    tainted: bool,
    edited: bool,
}

fn row(item: &OutboxItem) -> Row {
    Row {
        id: item.id.clone(),
        tool: item.tool.clone(),
        kind: format!("{:?}", item.kind).to_lowercase(),
        summary: item.summary.clone(),
        status: item.status.clone(),
        created_at: item.created_at.clone(),
        tainted: item.taint.trifecta_armed(),
        edited: item.edited(),
    }
}

/// Pending first, newest within each group — the modal's ordering.
fn rows(mut items: Vec<OutboxItem>) -> (Vec<Row>, usize) {
    items.sort_by(|a, b| {
        let rank = |i: &OutboxItem| (i.status != "pending") as u8;
        rank(a).cmp(&rank(b)).then(b.created_at.cmp(&a.created_at))
    });
    let resolved = items.iter().filter(|i| i.status != "pending").count();
    (items.iter().map(row).collect(), resolved)
}

/// GET /api/outbox — pending drafts, with the resolved count so the filter
/// is visible rather than the list silently looking shorter than it is.
pub async fn list(State(state): St) -> Response {
    let store = match OutboxStore::open(&state.review.outbox_root) {
        Ok(s) => s,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response(),
    };
    let items = match store.items() {
        Ok(items) => items,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response(),
    };
    let (all, resolved) = rows(items);
    let pending: Vec<&Row> = all.iter().filter(|r| r.status == "pending").collect();
    Json(serde_json::json!({ "pending": pending, "resolved": resolved })).into_response()
}

/// GET /api/outbox/{id} — the whole reviewable object.
pub async fn detail(State(state): St, UrlPath(id): UrlPath<String>) -> Response {
    let store = match OutboxStore::open(&state.review.outbox_root) {
        Ok(s) => s,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response(),
    };
    let item = match store.item(&id) {
        Ok(item) => item,
        Err(e) => return (StatusCode::NOT_FOUND, format!("{e:#}\n")).into_response(),
    };
    let sources = match (&state.review.sessions_dir, item.kind) {
        // A publish's reviewable object is the rendered page, not a thread.
        (Some(dir), OutboxKind::Message) => outbox_source::for_item(&item, dir),
        _ => Vec::new(),
    };
    Json(detail_json(&item, &sources)).into_response()
}

/// Pure, so the nothing-is-dropped property is a unit test rather than a
/// hope: every argument lands in headers, body, or other — and the exact
/// bytes ride along for the confirm sheet, which is the check, not the read.
fn detail_json(item: &OutboxItem, sources: &[outbox_source::SourceRead]) -> serde_json::Value {
    let view = DraftView::of(&item.args);
    serde_json::json!({
        "id": item.id,
        "tool": item.tool,
        "kind": format!("{:?}", item.kind).to_lowercase(),
        "status": item.status,
        "created_at": item.created_at,
        "summary": item.summary,
        "taint": {
            "private": item.taint.private,
            "untrusted": item.taint.untrusted,
            "armed": item.taint.trifecta_armed(),
        },
        "headers": view.headers,
        "body": view.body,
        "other": view.other,
        "edited": item.edited(),
        "args": item.args,
        "session_id": item.session_id,
        "sources": sources.iter().map(|s| serde_json::json!({
            "tool": s.tool,
            "keys": s.keys,
            "text": s.text,
        })).collect::<Vec<_>>(),
    })
}

#[derive(serde::Deserialize)]
pub struct RejectBody {
    pub reason: String,
}

#[derive(serde::Deserialize)]
pub struct EditBody {
    pub body: String,
}

/// POST /api/outbox/{id}/approve — the page confirmed (tainted drafts with
/// the exact arguments on screen); `--yes` hands that confirmation to the
/// CLI, exactly as the TUI modal does.
pub async fn approve(State(state): St, UrlPath(id): UrlPath<String>) -> Response {
    verb(&state, &["outbox", "approve", &id, "--yes"]).await
}

/// POST /api/outbox/{id}/reject — a reason is required: it returns to the
/// store as the record of the refusal, and the learning miner reads it.
pub async fn reject(
    State(state): St,
    UrlPath(id): UrlPath<String>,
    Json(body): Json<RejectBody>,
) -> Response {
    let reason = body.reason.trim();
    if reason.is_empty() {
        return (StatusCode::BAD_REQUEST, "a reject needs a reason\n").into_response();
    }
    verb(&state, &["outbox", "reject", &id, "--reason", reason]).await
}

/// POST /api/outbox/{id}/edit — the prose through `--body-file`, so the one
/// implementation of "edit" (guards, learning capture, no-prose refusal)
/// stays in the CLI.
pub async fn edit(
    State(state): St,
    UrlPath(id): UrlPath<String>,
    Json(body): Json<EditBody>,
) -> Response {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("mecha-web-edit-{}-{}.md", id, std::process::id()));
    if let Err(e) = std::fs::write(&path, &body.body) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response();
    }
    let path_str = path.to_string_lossy().to_string();
    let result = verb(&state, &["outbox", "edit", &id, "--body-file", &path_str]).await;
    let _ = std::fs::remove_file(&path);
    result
}

/// Run our own binary and relay the outcome: stdout on success, the first
/// stderr line with a 409 on refusal — the CLI's error *is* the API's.
async fn verb(state: &super::WebState, args: &[&str]) -> Response {
    let _ = state; // state carries nothing the child needs; the store is the meeting point
    let output = match tokio::time::timeout(
        std::time::Duration::from_secs(120),
        tokio::process::Command::new(crate::exe::self_exe())
            .args(args)
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("spawning: {e:#}\n"),
            )
                .into_response()
        }
        Err(_) => return (StatusCode::GATEWAY_TIMEOUT, "the verb timed out\n").into_response(),
    };
    if output.status.success() {
        Json(serde_json::json!({
            "ok": true,
            "output": String::from_utf8_lossy(&output.stdout).trim(),
        }))
        .into_response()
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        (
            StatusCode::CONFLICT,
            format!("{}\n", stderr.lines().last().unwrap_or("failed")),
        )
            .into_response()
    }
}

pub fn review_state(config: &mecha_core::config::Config) -> Result<ReviewState> {
    let outbox_root = match config.outbox.dir.clone() {
        Some(dir) => dir,
        None => OutboxStore::default_root().context("resolving the outbox root")?,
    };
    Ok(ReviewState {
        outbox_root,
        sessions_dir: Session::default_dir().ok(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::agent::Taint;
    use serde_json::json;

    fn item(id: &str, status: &str, created: &str) -> OutboxItem {
        OutboxItem {
            id: id.into(),
            status: status.into(),
            tool: "mail__mail_reply".into(),
            kind: OutboxKind::Message,
            args_before: json!({"thread_id": "t", "body_markdown": "hi"}),
            args: json!({"thread_id": "t", "body_markdown": "hi", "account": "a"}),
            summary: "re: something".into(),
            session_id: None,
            workspace: None,
            taint: Taint {
                private: true,
                untrusted: true,
            },
            created_at: created.into(),
            resolved_at: None,
            reason: None,
            error: None,
        }
    }

    #[test]
    fn pending_sorts_before_resolved_and_newest_first() {
        let (rows, resolved) = rows(vec![
            item("a", "sent", "2026-08-24T10:00:00Z"),
            item("b", "pending", "2026-08-20T10:00:00Z"),
            item("c", "pending", "2026-08-23T10:00:00Z"),
        ]);
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["c", "b", "a"]
        );
        assert_eq!(resolved, 1);
    }

    #[test]
    fn detail_drops_nothing_every_arg_key_is_visible() {
        let item = item("x", "pending", "2026-08-24T10:00:00Z");
        let detail = detail_json(&item, &[]);
        let shown: Vec<String> = detail["headers"]
            .as_array()
            .unwrap()
            .iter()
            .chain(detail["other"].as_array().unwrap())
            .map(|pair| pair[0].as_str().unwrap().to_string())
            .collect();
        let mut covered = shown;
        if let Some(field) = DraftView::of(&item.args).body_field {
            covered.push(field);
        }
        for key in item.args.as_object().unwrap().keys() {
            assert!(
                covered.contains(key),
                "argument {key} is invisible to the reviewer"
            );
        }
        assert!(detail["taint"]["armed"].as_bool().unwrap());
        assert_eq!(detail["args"], item.args, "the exact bytes must ride along");
    }
}
