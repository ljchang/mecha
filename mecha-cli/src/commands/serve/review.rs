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
use axum::extract::{Path as UrlPath, Query, State};
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
    /// A verb a person reads ("Reply", "Doc edit"), never the registry name.
    label: String,
    /// The addressing line — subject, title, or recipient — from `DraftView`.
    headline: String,
    /// The prose head, word-safe; for prose-less drafts, the short arguments
    /// rendered readably. Never raw JSON: the store's `summary` field is a
    /// terminal one-liner, and putting it on a card was the wrong bytes for
    /// the surface (found by a person reading the phone, not the code).
    snippet: String,
    status: String,
    created_at: String,
    tainted: bool,
    edited: bool,
}

/// The registry name, made into a verb. Curated for the tools that exist,
/// with a humanized fallback so an unanticipated tool reads as words too.
fn label_for(tool: &str) -> String {
    let suffix = tool.rsplit("__").next().unwrap_or(tool);
    match suffix {
        "mail_reply" => "Reply".into(),
        "mail_send" => "New mail".into(),
        "docs_replace" => "Doc edit".into(),
        "docs_create" => "New doc".into(),
        "sheets_write" => "Sheet write".into(),
        "slides_write" => "Slides edit".into(),
        "calendar_create" | "calendar_respond" => "Calendar".into(),
        other => {
            let words = other.replace('_', " ");
            let mut c = words.chars();
            match c.next() {
                Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                None => tool.to_string(),
            }
        }
    }
}

/// Cut at a word boundary with an ellipsis — never mid-identifier, never a
/// wall of base64-shaped id.
fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    let cut = match cut.rfind(char::is_whitespace) {
        Some(i) if i > max / 2 => &cut[..i],
        _ => cut.as_str(),
    };
    format!("{}…", cut.trim_end())
}

/// An id-shaped value tells a reviewer nothing on a card.
fn id_like(key: &str, value: &str) -> bool {
    key.ends_with("_id") || key == "id" || (value.len() > 24 && !value.contains(' '))
}

fn headline_and_snippet(args: &serde_json::Value) -> (String, String) {
    let view = DraftView::of(args);
    let get = |k: &str| {
        view.headers
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.clone())
    };
    let headline = get("subject")
        .or_else(|| get("title"))
        .or_else(|| get("to").map(|to| format!("To {to}")))
        .or_else(|| get("channel").map(|c| format!("To #{c}")))
        .unwrap_or_default();
    let snippet = match &view.body {
        Some(body) => clip(body, 140),
        None => {
            // No prose: the short, non-id arguments, readably.
            let pairs: Vec<String> = view
                .other
                .iter()
                .filter(|(k, v)| !id_like(k, v))
                .take(2)
                .map(|(k, v)| format!("{k}: {}", clip(v, 60)))
                .collect();
            pairs.join(" · ")
        }
    };
    (headline, snippet)
}

fn row(item: &OutboxItem) -> Row {
    let (headline, snippet) = headline_and_snippet(&item.args);
    Row {
        id: item.id.clone(),
        tool: item.tool.clone(),
        kind: format!("{:?}", item.kind).to_lowercase(),
        label: label_for(&item.tool),
        headline,
        snippet,
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
    let (headline, _) = headline_and_snippet(&item.args);
    serde_json::json!({
        "id": item.id,
        "tool": item.tool,
        "label": label_for(&item.tool),
        "headline": headline,
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

// ---------------------------------------------------------------------------
// The graph queue: proposers rollup and the sample deck. The sampling rules
// are the CLI's (`mecha review sample`): the seed is chosen here and
// *returned*, so the sample a phone verdicts is redrawable and quotable; a
// verdict never resamples (the page drops the card locally, seed unchanged);
// a new draw is an explicit button and a new seed.

/// GET /api/queue — the queue rolled up by proposing mechanism, each row
/// stamped with its evidence tier (see `classes` for why the stamp is
/// computed here and never in page script).
pub async fn queue(State(state): St) -> Response {
    match self_json(&state, &["review", "proposers", "--json"]).await {
        Ok(mut v) => {
            if let Some(rows) = v.as_array_mut() {
                for row in rows {
                    let judged = row["accepted_hist"].as_i64().unwrap_or(0)
                        + row["rejected_hist"].as_i64().unwrap_or(0);
                    row["tier"] = serde_json::json!(crate::tui::queues::Tier::of(judged).as_str());
                }
            }
            Json(v).into_response()
        }
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct ClassQuery {
    pub proposer: Option<String>,
}

/// GET /api/queue/classes — one proposer's pending classes, each stamped
/// with its evidence tier.
///
/// **The tier is computed here, from `tui::queues::Tier::of`** — the single
/// definition behind the TUI's label and filter — and never re-derived in
/// page script. The page shipped with its own `n < 10 ? 'thin' : …` copy of
/// the thresholds, which is exactly the drift the one-definition rule
/// exists to prevent: a filter disagreeing with the word beside it is worse
/// than no filter, because you verdict a class believing it sits in a tier
/// it does not.
pub async fn classes(State(state): St, Query(q): Query<ClassQuery>) -> Response {
    let mut args: Vec<&str> = vec!["review", "list", "--json"];
    if let Some(proposer) = &q.proposer {
        args.push("--proposer");
        args.push(proposer);
    }
    match self_json(&state, &args).await {
        Ok(mut v) => {
            if let Some(rows) = v.as_array_mut() {
                for row in rows {
                    let judged = row["accepted_hist"].as_i64().unwrap_or(0)
                        + row["rejected_hist"].as_i64().unwrap_or(0);
                    row["tier"] = serde_json::json!(crate::tui::queues::Tier::of(judged).as_str());
                }
            }
            Json(v).into_response()
        }
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct GroupsQuery {
    pub proposer: String,
    pub predicate: String,
}

/// GET /api/queue/groups — one class's pending candidates grouped by
/// semantic similarity, largest first: where one verdict fans out furthest.
/// The envelope (`{threshold, groups}`) is the CLI's, passed through — a
/// group's face is a real member statement, never a paraphrase, on the
/// outbox's rule that approving a summary is approving unread. Both class
/// keys are required because a group never crosses a class; the CLI child
/// enforces that too, but a route that *could* ask for cross-class groups
/// would document a thing the system refuses to mean.
pub async fn groups(State(state): St, Query(q): Query<GroupsQuery>) -> Response {
    let args = [
        "review",
        "groups",
        "--proposer",
        &q.proposer,
        "--predicate",
        &q.predicate,
        "--json",
    ];
    match self_json(&state, &args).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct SampleBody {
    pub proposer: Option<String>,
    pub predicate: Option<String>,
    pub seed: Option<u64>,
}

/// POST /api/queue/sample — twelve at random; the seed rides back with them.
pub async fn sample(State(state): St, Json(body): Json<SampleBody>) -> Response {
    let seed = body.seed.unwrap_or_else(|| {
        // Uncorrelated with the content is all the draw needs; the *record*
        // of which seed was drawn is what makes it checkable.
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64 ^ (d.as_secs() << 20))
            .unwrap_or(1)
    });
    let seed_s = seed.to_string();
    let mut args: Vec<&str> = vec!["review", "sample", "-n", "12", "--seed", &seed_s, "--json"];
    if let Some(proposer) = &body.proposer {
        args.push("--proposer");
        args.push(proposer);
    }
    if let Some(predicate) = &body.predicate {
        args.push("--predicate");
        args.push(predicate);
    }
    match self_json(&state, &args).await {
        Ok(items) => Json(serde_json::json!({ "seed": seed, "items": items })).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct VerdictBody {
    pub id: i64,
    pub accept: bool,
    pub reason: Option<String>,
    /// Explicit member ids from a groups listing: the named id is the
    /// owner's verdict, these follow as a labeled machine cascade the
    /// ladder never counts. Always the ids the page *showed*, never a
    /// re-derived similarity — what a person saw is what the verdict covers.
    pub cascade: Option<Vec<i64>>,
}

/// POST /api/queue/verdict — one candidate, one verdict, through the CLI.
/// With `cascade`, one *human* verdict still: the members ride `--cascade`
/// and land labeled `cascade:<seed>`, invisible to the autonomy ladder.
pub async fn verdict(State(state): St, Json(body): Json<VerdictBody>) -> Response {
    let id = body.id.to_string();
    let members = body
        .cascade
        .as_ref()
        .filter(|ids| !ids.is_empty())
        .map(|ids| {
            ids.iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(",")
        });
    let mut args: Vec<&str> = vec!["review"];
    if body.accept {
        args.extend(["accept", &id]);
    } else {
        args.extend(["reject", &id]);
        if let Some(reason) = body.reason.as_deref().filter(|r| !r.trim().is_empty()) {
            args.extend(["--reason", reason]);
        }
    }
    if let Some(members) = &members {
        args.extend(["--cascade", members]);
    }
    verb(&state, &args).await
}

/// Like `verb`, but the child's stdout is JSON to pass through.
pub(super) async fn self_json(
    state: &super::WebState,
    args: &[&str],
) -> anyhow::Result<serde_json::Value> {
    let _ = state;
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        tokio::process::Command::new(crate::exe::self_exe())
            .args(args)
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("timed out"))??;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{}", stderr.lines().last().unwrap_or("failed"));
    }
    Ok(serde_json::from_slice(&output.stdout)?)
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
pub(super) async fn verb(state: &super::WebState, args: &[&str]) -> Response {
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
    fn a_reply_row_reads_as_words_never_json() {
        let mut it = item("x", "pending", "2026-08-24T10:00:00Z");
        it.args = json!({
            "account": "dartmouth",
            "thread_id": "f8a2c1d9e0aa9b",
            "subject": "Re: R01 resubmission",
            "body_markdown": "Dear Dirk,\n\nThank you for reaching out and for your interest in our work on the neural signature of trust and everything after it."
        });
        let row = row(&it);
        assert_eq!(row.label, "Reply");
        assert_eq!(row.headline, "Re: R01 resubmission");
        assert!(row.snippet.starts_with("Dear Dirk,"));
        assert!(row.snippet.chars().count() <= 145);
        assert!(!row.snippet.contains('{'), "raw JSON on a card was the bug");
        assert!(
            !row.snippet.contains("f8a2c1"),
            "ids tell a reviewer nothing"
        );
    }

    #[test]
    fn a_prose_less_draft_shows_short_args_and_hides_id_shaped_ones() {
        let mut it = item("y", "pending", "2026-08-24T10:00:00Z");
        it.tool = "docs__docs_replace".into();
        it.args = json!({
            "file_id": "110RA0YIgljxkaZXZcfnp7hKFLqeZtfnlnJRiBBn",
            "find": "Office hours Tu 2-4",
            "replace": "Office hours Th 1-3"
        });
        let row = row(&it);
        assert_eq!(row.label, "Doc edit");
        assert!(
            !row.snippet.contains("110RA0"),
            "the file id is noise on a card"
        );
        assert!(row.snippet.contains("Office hours"));
    }

    #[test]
    fn an_unknown_tool_still_reads_as_words() {
        let mut it = item("z", "pending", "2026-08-24T10:00:00Z");
        it.tool = "factory__bundle_publish".into();
        assert_eq!(row(&it).label, "Bundle publish");
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
