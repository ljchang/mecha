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
        // Why the last release attempt failed, when one did. The store has
        // carried this since failed sends started surviving as pending, and
        // the page could only ever say "1 of 1 item(s) did not send" — a
        // reviewer looking straight at an item that cannot succeed, with the
        // reason on disk two fields away. A queue that cannot say why it is
        // stuck is a queue that grows.
        "error": item.error,
        "args": item.args,
        "session_id": item.session_id,
        "sources": sources.iter().map(|s| serde_json::json!({
            "tool": s.tool,
            "keys": s.keys,
            // The rendered line and the raw discriminant both: the pane shows
            // the first and can style on the second, and neither is rebuilt
            // from `keys` on the far side of a JSON boundary.
            "heading": s.heading(),
            "join": match s.join {
                outbox_source::Join::Asked => "asked",
                outbox_source::Join::Returned => "returned",
            },
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
    pub proposer: Option<String>,
    pub predicate: Option<String>,
    /// The top layer: group the whole pending queue across classes.
    #[serde(default)]
    pub all: bool,
    /// Cosine floor override — the page's looser/stricter stepper, always
    /// stepping from the threshold the last envelope reported ran.
    pub threshold: Option<f64>,
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
    if q.all {
        // The global layer embeds every pending statement, which is minutes,
        // not seconds — a deliberate button on the page, priced accordingly.
        let mut args: Vec<&str> = vec!["review", "groups", "--all", "--json"];
        if let Some(p) = &q.proposer {
            args.push("--proposer");
            args.push(p);
        }
        let t_s = q.threshold.map(|t| format!("{t:.2}"));
        if let Some(t) = &t_s {
            args.push("--threshold");
            args.push(t);
        }
        return match self_json_within(&state, &args, 360).await {
            Ok(v) => Json(v).into_response(),
            Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
        };
    }
    let (Some(proposer), Some(predicate)) = (&q.proposer, &q.predicate) else {
        return (
            StatusCode::BAD_REQUEST,
            "class groups need proposer and predicate; all=true is the cross-class layer\n",
        )
            .into_response();
    };
    let args = [
        "review",
        "groups",
        "--proposer",
        proposer,
        "--predicate",
        predicate,
        "--json",
    ];
    match self_json(&state, &args).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct ItemsQuery {
    /// Candidate ids, comma-separated, in the order to show them — a
    /// similarity group's members, leader first.
    pub ids: String,
}

/// GET /api/queue/items?ids=… — the named candidates, in full.
///
/// The way into a group without covering it: a group verdict is one
/// keystroke over every member, which is right when they repeat and wrong
/// when they merely *rhyme* — seven near-repeats naming three different
/// children are contradictory facts a single Accept would assert together.
/// So the members are readable one at a time, and each verdict there is a
/// plain verdict on one candidate: no cascade, nothing labeled, one human
/// decision per fact.
///
/// **A named set, re-fetched by id — never a redraw.** The set is what the
/// sitting is about (the TUI's rule at the same depth), so this route takes
/// ids and has deliberately no sampling parameter: a second draw here would
/// answer a question nobody asked and quietly change which items a person
/// believes they have judged.
pub async fn items(State(state): St, Query(q): Query<ItemsQuery>) -> Response {
    let ids = q.ids.trim();
    // An empty list would reach the CLI as `--ids ""` and come back as the
    // whole class head — a listing where the caller asked for a named set.
    if ids.is_empty() || !ids.split(',').all(|p| p.trim().parse::<i64>().is_ok()) {
        return (
            StatusCode::BAD_REQUEST,
            "ids must be a comma-separated list of candidate ids\n",
        )
            .into_response();
    }
    match self_json(&state, &["review", "items", "--ids", ids, "--json"]).await {
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
    /// The cascade came from the cross-class layer (`groups?all=true`), so
    /// its ids may sit in other classes. Meaningless without `cascade`.
    #[serde(default)]
    pub across: bool,
    /// `accept --create-subjects`: a subject the graph does not know becomes
    /// a new topic node instead of the verdict failing. The second way
    /// through `cannot resolve subject`, and the one that does not need a
    /// target to bind to.
    #[serde(default)]
    pub create_subjects: bool,
}

/// POST /api/queue/verdict — one candidate, one verdict, through the CLI.
/// With `cascade`, one *human* verdict still: the members ride `--cascade`
/// and land labeled `cascade:<seed>`, invisible to the autonomy ladder.
///
/// **A verdict that landed on nothing is a failure, whatever the exit code
/// says.** `mecha-graph accept <id>` prints `#id FAILED: …` and exits *zero*
/// — right for a bulk run where one candidate of five hundred cannot
/// resolve, and a lie here, because the page drops the card it just sent and
/// reports success. The child's own report is the only account of what
/// happened, so it is tallied with `review::tally_report` — the same
/// function the TUI reads, so the two surfaces cannot come to different
/// conclusions about one line of output — and a zero tally answers 409 with
/// the reason. Only the *cascade* arm exits non-zero today, which is why the
/// bug was invisible on the group cards and live on the sample deck.
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
        if body.create_subjects {
            args.push("--create-subjects");
        }
    } else {
        args.extend(["reject", &id]);
        if let Some(reason) = body.reason.as_deref().filter(|r| !r.trim().is_empty()) {
            args.extend(["--reason", reason]);
        }
    }
    if let Some(members) = &members {
        args.extend(["--cascade", members]);
        if body.across {
            args.push("--across-classes");
        }
    }
    let report = match verb_output(&state, &args).await {
        Ok(out) => out,
        Err(refusal) => return *refusal,
    };
    let (landed, _failed) = crate::commands::review::tally_report(&report);
    if landed == 0 {
        return (
            StatusCode::CONFLICT,
            format!("{}\n", why_nothing_landed(&report)),
        )
            .into_response();
    }
    let (cascaded, left) = crate::commands::review::cascade_tally(&report).unwrap_or((0, 0));
    Json(serde_json::json!({
        "ok": true,
        "landed": landed,
        "cascaded": cascaded,
        "left_pending": left,
        "output": report.trim(),
    }))
    .into_response()
}

/// The line of a verdict report that says why nothing landed.
///
/// Pure so it can be tested, and deliberately the child's own words: this
/// string is what a person reads before deciding what to do next, and a
/// re-wording here would be a second account of a failure the graph already
/// described exactly once.
pub(super) fn why_nothing_landed(report: &str) -> String {
    report
        .lines()
        .map(str::trim)
        .find(|l| l.contains("FAILED"))
        .map(|l| l.to_string())
        .unwrap_or_else(|| "the verdict landed on nothing, with no reason reported".into())
}

#[derive(serde::Deserialize)]
pub struct BindBody {
    pub id: i64,
    /// Exact display name of the entity to bind to; omitted, the graph takes
    /// its own top suggestion.
    pub to: Option<String>,
}

/// POST /api/queue/bind — rebind a candidate's unresolvable subject.
///
/// The way through `cannot resolve subject 'X'` without leaving the surface
/// that reported it, and the reason this exists on the phone at all: the
/// error was already arriving here, with the two keys that answer it
/// (`b` and `A`) reachable only from the TUI. The old spelling is learned as
/// an alias graph-side, so the fix outlives this one candidate — and on a
/// *group*, binding the leader unblocks the whole cascade, because sharing a
/// subject is most of what made it a group.
///
/// The candidate stays pending: a bound subject is a candidate that can now
/// be accepted, not one that has been.
pub async fn bind(State(state): St, Json(body): Json<BindBody>) -> Response {
    let id = body.id.to_string();
    let mut args: Vec<&str> = vec!["review", "bind", &id];
    if let Some(to) = body.to.as_deref().filter(|t| !t.trim().is_empty()) {
        args.extend(["--to", to]);
    }
    verb(&state, &args).await
}

/// Like `verb`, but the child's stdout is JSON to pass through.
pub(super) async fn self_json(
    state: &super::WebState,
    args: &[&str],
) -> anyhow::Result<serde_json::Value> {
    self_json_within(state, args, 30).await
}

/// `self_json` with the budget stated, for the one child that legitimately
/// runs minutes (the global grouping embeds the whole queue).
pub(super) async fn self_json_within(
    state: &super::WebState,
    args: &[&str],
    secs: u64,
) -> anyhow::Result<serde_json::Value> {
    let _ = state;
    let mut cmd = tokio::process::Command::new(crate::exe::self_exe());
    if let Some(dir) = super::child_cwd() {
        cmd.current_dir(dir);
    }
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(secs),
        cmd.args(args).output(),
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

/// Run our own binary and hand back its stdout, or the response a refusal
/// deserves — the first stderr line with a 409, the CLI's error *being* the
/// API's.
///
/// Split out of [`verb`] rather than duplicated because a caller sometimes
/// has to *read* what the child said (a verdict's own report is the only
/// account of how many candidates it landed on) and every such caller must
/// still refuse a spawn failure, a timeout and a non-zero exit identically.
/// Two spellings of that is two places for the statuses to drift.
/// The refusal is boxed because an axum `Response` is 128 bytes, and a
/// `Result` whose error dwarfs its success value is paid for by every call
/// that never fails — `clippy::result_large_err`, which the CI toolchain
/// enforces and this box's older clippy does not yet know about. Worth the
/// noise at three call sites: the alternative found the repo through a red
/// main rather than through a local run.
pub(super) async fn verb_output(
    state: &super::WebState,
    args: &[&str],
) -> Result<String, Box<Response>> {
    let _ = state; // state carries nothing the child needs; the store is the meeting point
    let mut cmd = tokio::process::Command::new(crate::exe::self_exe());
    if let Some(dir) = super::child_cwd() {
        cmd.current_dir(dir);
    }
    let output =
        match tokio::time::timeout(std::time::Duration::from_secs(120), cmd.args(args).output())
            .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                return Err(Box::new(
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("spawning: {e:#}\n"),
                    )
                        .into_response(),
                ))
            }
            Err(_) => {
                return Err(Box::new(
                    (StatusCode::GATEWAY_TIMEOUT, "the verb timed out\n").into_response(),
                ))
            }
        };
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(Box::new(
            (
                StatusCode::CONFLICT,
                format!("{}\n", stderr.lines().last().unwrap_or("failed")),
            )
                .into_response(),
        ))
    }
}

/// Run our own binary and relay the outcome, for the verbs whose whole
/// answer is "it worked".
pub(super) async fn verb(state: &super::WebState, args: &[&str]) -> Response {
    match verb_output(state, args).await {
        Ok(out) => Json(serde_json::json!({ "ok": true, "output": out.trim() })).into_response(),
        Err(refusal) => *refusal,
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

    /// The bug this endpoint was carrying: `mecha-graph accept <id>` reports
    /// a per-candidate failure on *stdout* and exits zero, so a page that
    /// keyed on the exit code dropped the card it had just sent and told the
    /// owner it had landed. Nothing had; the candidate is still pending.
    #[test]
    fn a_verdict_that_landed_on_nothing_is_not_a_success() {
        // A neutral subject on purpose: this repo is public, and the string
        // a resolve failure carries is a *name out of the owner's graph*.
        // The test is about the reporting, never about whose name it was.
        let report = "#9286 FAILED: cannot resolve subject 'Ada and Grace Fixture'\n";
        let (landed, failed) = crate::commands::review::tally_report(report);
        assert_eq!((landed, failed), (0, 1));
        assert!(
            why_nothing_landed(report).contains("cannot resolve subject"),
            "the reason is the child's own words — it is what the owner acts on"
        );
    }

    /// And the other half: a cascade that worked must not be mistaken for a
    /// failure by the same rule. The seed's line is the human verdict; the
    /// members are summarised on the graph's own `cascade:` line.
    #[test]
    fn a_cascade_that_landed_reports_its_two_numbers() {
        let report = "#601 accepted -> fact 3f2a (your verdict)\n\
                      cascade: 6 accepted, 0 left pending — one human verdict on the ladder\n";
        let (landed, _) = crate::commands::review::tally_report(report);
        assert_eq!(landed, 1, "one human verdict, whatever the fan-out");
        assert_eq!(crate::commands::review::cascade_tally(report), Some((6, 0)));
    }

    /// A report with no `FAILED` line and nothing accepted says *something*
    /// rather than an empty string: a refusal a person cannot read is a
    /// refusal they will retry forever.
    #[test]
    fn a_silent_report_still_names_itself() {
        assert!(!why_nothing_landed("").is_empty());
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

    #[test]
    fn a_failed_send_tells_the_reviewer_why() {
        // A failed release leaves the item pending with the reason recorded,
        // and the page used to receive everything about that item except the
        // reason — so a draft that could never succeed looked identical to
        // one nobody had tried yet, and the only signal was a summary count
        // in a notice. The field is the whole fix; the test is here because
        // dropping one key from a payload is invisible in review.
        let mut item = item("x", "pending", "2026-08-24T10:00:00Z");
        assert_eq!(
            detail_json(&item, &[])["error"],
            serde_json::Value::Null,
            "an untried item has no failure to report"
        );
        item.error = Some("no default account is set".into());
        assert_eq!(
            detail_json(&item, &[])["error"],
            "no default account is set"
        );
    }
}
