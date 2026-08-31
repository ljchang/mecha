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
    // A class grouping embeds that class, which is seconds on a thousand-item
    // class and more on the big ones — the same kind of work as the global
    // layer, one class wide. It used to go through the default 30-second
    // budget, so a class large enough to be worth grouping was the one that
    // answered `502 timed out` on the phone while the identical `mecha review
    // groups` in a terminal, which has no cap, printed it. Same work, same
    // order of magnitude, so it gets the stated budget too — smaller than the
    // whole queue's, because it is a fraction of the queue.
    match self_json_within(&state, &args, 180).await {
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
            format!("{}\n", crate::commands::review::why_nothing_landed(&report)),
        )
            .into_response();
    }
    // `None` when the report carried no `cascade:` line at all — an older
    // graph binary, a changed line, a cascade that died before printing. It
    // rides out as `null` rather than being flattened to zero, because a
    // reader cannot tell a real zero from a failure to look, and this one
    // acts on the difference: the page marks a fan-out's members judged when
    // none were left pending, and hiding still-pending candidates is the
    // worse of the two ways it can be wrong. A dash is never zero, on the
    // wire as much as in a column.
    let cascade = crate::commands::review::cascade_tally(&report);
    Json(serde_json::json!({
        "ok": true,
        "landed": landed,
        "cascaded": cascade.map(|(c, _)| c),
        "left_pending": cascade.map(|(_, left)| left),
        "output": report.trim(),
    }))
    .into_response()
}

#[derive(serde::Deserialize)]
pub struct ShadowQuery {
    pub limit: Option<usize>,
}

/// GET /api/queue/shadow — the graph's surfaced-verdict queue: live shadow
/// facts that are about to matter, each with the reasons it surfaced.
/// Passed through from `mecha review shadow --json` (which runs the
/// owner's `mecha-graph` binary), so the page shows the graph's own
/// account — counts included: `shadow_live` and `shadow_served` ride the
/// envelope beside `surfaced`.
pub async fn shadow(State(state): St, Query(q): Query<ShadowQuery>) -> Response {
    let limit = q.limit.unwrap_or(10).min(50).to_string();
    match self_json(&state, &["review", "shadow", "--json", "--limit", &limit]).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            format!(
                "{e:#}
"
            ),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct ShadowVerdictBody {
    pub uid: String,
    /// true = confirm (a human stands behind it, tier → reviewed);
    /// false = refute (never true; retracted, and the reason feeds the
    /// graph's rejection memory).
    pub confirm: bool,
    pub reason: Option<String>,
}

/// POST /api/queue/shadow/verdict — decide one surfaced shadow fact.
///
/// This is a human surface: the request rides the owner's authenticated
/// web session and lands in the owner's own `mecha-graph` binary. The
/// verdict verbs are deliberately absent from the MCP tool surface
/// (`kg_shadow_queue` is read-only) — the model can show the queue, but
/// only a hand on an owner surface can settle it.
pub async fn shadow_verdict(State(state): St, Json(body): Json<ShadowVerdictBody>) -> Response {
    let mut args: Vec<&str> = vec!["review", "shadow"];
    if body.confirm {
        args.extend(["--confirm", &body.uid]);
    } else {
        args.extend(["--refute", &body.uid]);
        if let Some(r) = body.reason.as_deref().filter(|r| !r.trim().is_empty()) {
            args.extend(["--reason", r]);
        }
    }
    verb(&state, &args).await
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

    const QUEUE_SVELTE: &str = include_str!("../../../../web/src/lib/Queue.svelte");

    /// The body of a top-level function in the queue pane's script block,
    /// from its opening line to the `\n  }` that closes it at script indent.
    fn queue_fn(name: &str) -> &'static str {
        let after = QUEUE_SVELTE
            .split_once(&format!("function {name}("))
            .unwrap_or_else(|| panic!("Queue.svelte must still define `{name}`"))
            .1;
        after
            .split_once("\n  }")
            .expect("the function must still close at script indent")
            .0
    }

    /// **Leaving a similarity group must cost nothing.**
    ///
    /// The whole-queue layer embeds every pending statement, which the route
    /// below budgets three hundred and sixty seconds for. `closeItems` used
    /// to answer the Back arrow by re-running exactly that query whenever a
    /// verdict had been filed inside the group — so a glance was free and the
    /// actual work was not, and the ordinary loop of a sitting (open a group,
    /// reject a few, step back) paid minutes each time round. It was reported
    /// as the thing that ended the desire to clear the queue, which is the
    /// real cost of a slow surface: not the wait, the abandonment.
    ///
    /// Nothing needs re-deriving. The page knows which ids it judged, and the
    /// TUI has rebuilt the group from its survivors since the level existed.
    ///
    /// Asserted against the source because this pane has no JS test rig, and
    /// checked as an *absence*: any call out of either function — `fetch`, or
    /// an open helper — reintroduces the wait. With the old body in place this
    /// fails on `openGlobal`.
    #[test]
    fn leaving_a_similarity_group_makes_no_request() {
        for name in ["closeItems", "reconcileGroup"] {
            let body = queue_fn(name);
            for forbidden in ["fetch(", "openGlobal(", "openGroups(", "loadGroups("] {
                assert!(
                    !body.contains(forbidden),
                    "{name} must not call `{forbidden}` — leaving a group would re-embed \
                     the queue. Prune the listing from the survivors instead.\n{body}"
                );
            }
        }
        // And the pruning must still happen, or "costs nothing" is bought by
        // leaving the card describing members the reviewer just judged away.
        let reconcile = queue_fn("reconcileGroup");
        assert!(
            reconcile.contains("survivors"),
            "reconcileGroup must rebuild the group from its survivors:\n{reconcile}"
        );
    }

    /// **The listing is brought into step by the verdict, not by the exit.**
    ///
    /// Back is not the only way off the group screen — the Review tabs are one
    /// tap away and unmount the pane, while the cached listing outlives it. So
    /// a prune that only ran on the way out could be walked around: reject
    /// three members, tap Outbox, tap back, reopen the grouping, and the cache
    /// serves a card still offering all seven. Worse than a wrong count if the
    /// leader was among them — a verdict seeded on a candidate that is gone
    /// fails, a fan-out from a failed verdict cascades nothing by the graph's
    /// own rule, and the card cannot be cleared at all, only regrouped.
    ///
    /// So the write-back belongs where the verdict lands, which no navigation
    /// can route around.
    #[test]
    fn a_member_verdict_writes_the_group_back_itself() {
        assert!(
            queue_fn("itemVerdict").contains("reconcileGroup()"),
            "itemVerdict must reconcile the group as the verdict lands"
        );
        assert!(
            queue_fn("openItems").contains("reconcileGroup()"),
            "openItems must reconcile away members judged before it was opened"
        );
    }

    /// One listing, one cache key.
    ///
    /// The cross-class layer can be asked with no threshold, or with the floor
    /// the server would have picked anyway — the same listing under two names.
    /// Filing it under both made the entries alias, and a write-back then had
    /// to find its own siblings; it could not, because a cached open re-keyed
    /// the listing to whichever name it was looked up under. A group emptied
    /// from inside came back offering to accept candidates already verdicted:
    /// the exact staleness the write-back exists to prevent.
    ///
    /// Resolving the name instead of duplicating the entry removes the class,
    /// so `cacheGroups` writes one key and a regroup overwrites the row it
    /// read.
    #[test]
    fn a_grouping_is_cached_under_exactly_one_key() {
        let body = queue_fn("cacheGroups");
        assert_eq!(
            body.matches("groupCache.set").count(),
            1,
            "cacheGroups must write one key, not a key and its aliases:\n{body}"
        );
        assert!(
            QUEUE_SVELTE.contains("defaultGlobalThreshold"),
            "the default cross-class floor must be resolved to the floor it means, \
             so one listing cannot be filed under two names"
        );
    }

    /// **A verdict reaches every cached listing, not just the one on screen.**
    ///
    /// A pending candidate sits in several cache entries at once: the stepper
    /// makes an entry per floor and a pair above the stricter one is in both,
    /// and a within-class near-repeat is in its class listing *and* the global
    /// one. A write-back that only touched `groups.key` left the others
    /// offering a candidate already verdicted — and a whole-group verdict
    /// seeded on one that is gone fails, cascades nothing, and leaves a card
    /// that only a Regroup can clear, which is the wait this branch exists to
    /// remove. Verdicts from the Sample-12 deck reached no listing at all.
    ///
    /// Every entry into the groups screen used to be a fresh fetch, so none of
    /// this was reachable: the cache is what makes it reachable. So the ids
    /// are recorded where verdicts are SENT — one place, no caller able to
    /// miss it — and every listing is filtered on the way out of the cache.
    #[test]
    fn a_verdict_reaches_listings_other_than_the_one_on_screen() {
        assert!(
            queue_fn("sendVerdict").contains("judgedIds.add"),
            "the one place a verdict is sent must record it, or a caller can file \
             a verdict no cached listing hears about"
        );
        // Every way a listing reaches the screen, or the gap reopens on
        // whichever one was left unfiltered. Deliberately not an equality on
        // the call count: the first spelling of this pinned it at two and so
        // would have *blocked* the fix for the third install path — the
        // error-restore — instead of catching that it was missing. A test
        // that forbids a correct change is worse than the one it replaced.
        let load = queue_fn("loadGroups");
        assert!(
            load.matches("withoutJudged(").count() >= 3,
            "every listing that reaches the screen must be filtered — the cache hit, \
             the fresh fetch, and the restore after a failed regroup:\n{load}"
        );
    }

    /// **A number nobody has is not zero.**
    ///
    /// `cascade_tally` answers `None` when the child's report carries no
    /// `cascade:` line — an older graph binary, a changed line, a cascade that
    /// died before printing. Flattening that to `(0, 0)` on the way out told
    /// the page "nothing was left pending" on the strength of a tally that had
    /// not been read, and the page acts on exactly that: it marks a fan-out's
    /// members judged, which hides still-pending candidates for the session.
    ///
    /// So the field is nullable, and the page tests `=== 0` rather than
    /// falsiness. This is the wire half; the reader half is asserted below it.
    #[test]
    fn an_unreadable_cascade_is_null_not_zero() {
        use crate::commands::review::cascade_tally;
        assert_eq!(
            cascade_tally("#12 rejected\ncascade: 4 rejected, 2 left pending"),
            Some((4, 2))
        );
        assert_eq!(
            cascade_tally("#12 rejected\ncascade: 6 rejected"),
            Some((6, 0))
        );
        // The case the flattening erased: a landed seed whose cascade arm said
        // nothing this can parse.
        assert_eq!(
            cascade_tally("#12 rejected\n(cascade arm produced no line)"),
            None
        );

        // And the page must not treat that `null` as "none left pending".
        let send = queue_fn("sendVerdict");
        assert!(
            send.contains("out?.left_pending === 0"),
            "a falsy test cannot tell `null` from a real zero:\n{send}"
        );
    }

    /// A count that has stopped being true is worse than an absent one.
    ///
    /// `classes` renders as per-class chips directly under a kicker reading
    /// "N near-repeats". Carrying it through a reconcile leaves the two
    /// disagreeing on the same card — reject four of seven and the kicker says
    /// three while the chips still sum to seven. Re-deriving it is not the
    /// alternative (the key is the graph's `cluster_key` and this page must
    /// not own a second copy of that rule); dropping it is. The page-level
    /// cross-class caution is not on the chips and survives without them.
    #[test]
    fn a_shrunken_group_stops_showing_its_old_class_spans() {
        assert!(
            queue_fn("reconcileGroup").contains("g.classes = null"),
            "a group that lost members must drop its class chips"
        );
        assert!(
            queue_fn("withoutJudged").contains("classes: null"),
            "a group rebuilt out of the cache must drop its class chips too"
        );
    }

    /// **A partial fan-out must not hide the members it left behind.**
    ///
    /// Between the two ways this page can be wrong about a cached listing —
    /// showing a candidate that is gone, or hiding one that is still there —
    /// the second is much the worse, because it is silent and a Regroup does
    /// not undo it. A first pass recorded every cascade id as judged on the
    /// belief that vetting only drops ids already decided. This route
    /// contradicts that on the same response: members are vetted per-id
    /// against the seed's class, an unresolvable subject fails the same way,
    /// and `left_pending` says how many stayed. Marking those judged removed
    /// them from every cached listing AND from the next fetch, which is
    /// filtered through the same set — invisible for the rest of the session.
    ///
    /// The route reports the number, so the page must read it, and the pane
    /// must say so out loud the way the TUI does.
    #[test]
    fn a_partial_cascade_leaves_its_survivors_visible() {
        let send = queue_fn("sendVerdict");
        assert!(
            send.contains("left_pending"),
            "the cascade may only be recorded as judged when all of it landed:\n{send}"
        );
        assert!(
            queue_fn("groupVerdict").contains("left_pending"),
            "a group verdict that swept less than it offered must say so"
        );
        // The number exists to be read: this is the field the page depends on.
        assert!(
            QUEUE_SVELTE.contains("left_pending"),
            "the page must consume the route's `left_pending`"
        );
    }

    /// The TUI's row has the same two numbers on it as the web card.
    ///
    /// `queues.rs` renders `×{g.size()}` with `spans: {c} ×{n}, …` directly
    /// underneath, so a group that lost members to a verdict reads `×3` above
    /// spans summing to seven — the same disagreement the web pane drops the
    /// chips to avoid. Both surfaces show a partially-judged group the same
    /// way, or the claim that they do is the thing that is wrong.
    #[test]
    fn the_tui_drops_its_spans_when_a_group_shrinks() {
        let src = include_str!("../../tui/mod.rs");
        let arm = src
            .split_once("// Back to the groups, updated LOCALLY")
            .expect("the local group rebuild must still be there")
            .1;
        let arm = &arm[..arm
            .find("modal.level = queues::Level::Groups")
            .unwrap_or(arm.len())];
        assert!(
            arm.contains("g.classes.clear()"),
            "a shrunken group must drop its spans line:\n{arm}"
        );
    }

    /// **A leader with nobody behind it is not a group, on all three paths.**
    ///
    /// A pair is the commonest group size there is, so judging one member is
    /// the ordinary case rather than an edge: it leaves a lone leader, and a
    /// card reading "1 near-repeats" over *Reject all 1* covers exactly one
    /// candidate the item list is already showing. Tapping it sends an empty
    /// cascade, which reaches the child as no `--cascade` at all and comes
    /// back with no `cascade:` line — so the pane announced that the fan-out
    /// could not be measured, about a group with nothing to fan out to.
    ///
    /// Two rebuild paths in this pane and one in the TUI, and they disagreed:
    /// `withoutJudged` dropped at one survivor while `reconcileGroup` dropped
    /// only at zero, and `split_first` kept the case in the modal. Nor did it
    /// self-heal — `reconcileGroup` had already written the emptied members
    /// to the cache, so the cached path saw a count that had not changed.
    #[test]
    fn a_group_of_one_is_dropped_wherever_a_group_is_rebuilt() {
        assert!(
            queue_fn("reconcileGroup").contains("survivors.length < 2"),
            "the live rebuild must drop a group that is down to its leader"
        );
        assert!(
            queue_fn("withoutJudged").contains("members.length === 0"),
            "the cached rebuild must drop a group that is down to its leader"
        );
        let src = include_str!("../../tui/mod.rs");
        let arm = src
            .split_once("// Back to the groups, updated LOCALLY")
            .expect("the local group rebuild must still be there")
            .1;
        assert!(
            arm[..arm
                .find("modal.level = queues::Level::Groups")
                .unwrap_or(arm.len())]
                .contains("!rest.is_empty()"),
            "the modal's rebuild must drop one too, or `both surfaces, one rule` is false"
        );
    }

    /// A notice about one group verdict must not outlive the screen it was
    /// about.
    ///
    /// It renders at the top of the pane at every depth, and the fan-out it
    /// describes belongs to a card that has just been removed. Left standing,
    /// it follows the reviewer into the class list and the sample deck and
    /// survives a Regroup that contradicts it — the failure the header's clock
    /// time is written to avoid, on a different field.
    ///
    /// **Scoped, not swept.** The first version cleared it by hand at each
    /// navigation point, and the one exit it missed was the only exit from the
    /// screen that writes it: the groups back arrow is an inline
    /// `() => { groups = null }` and cleared nothing. Counting clear sites
    /// could not find that — the missing one is a handler, not a function —
    /// and a test that counts would have gone on passing while any future
    /// handler reintroduced it. So the message carries the listing instance it
    /// belongs to and the render decides, which no new handler can get wrong.
    #[test]
    fn a_fan_out_notice_does_not_outlive_its_screen() {
        assert!(
            QUEUE_SVELTE.contains("notice.on === listingInstance"),
            "the notice must be scoped to the listing it describes, not cleared by hand \
             at each navigation point — the exits are handlers, and one was missed"
        );
        // `groups` gone is the back arrow, and it must hide the message too.
        assert!(
            QUEUE_SVELTE.contains("notice && groups && notice.on === listingInstance"),
            "leaving the groups screen must take the notice with it"
        );
        // Every install of a listing is a new instance, or a Regroup would
        // keep a message that contradicts what it just fetched.
        assert_eq!(
            queue_fn("loadGroups")
                .matches("listingInstance += 1")
                .count(),
            3,
            "each of the three ways a listing reaches the screen is a new instance"
        );
    }

    /// One door to the expensive query, so the cache cannot be walked around.
    ///
    /// A grouping is kept for the life of the page (`groupCache`), which is
    /// what makes the Back arrow and a tab switch free rather than a fresh
    /// two minutes. That only holds while every path to a listing goes
    /// through the one loader — a second `fetch` of this route added beside
    /// it would be a listing nobody cached and nobody could regroup.
    #[test]
    fn the_grouping_query_has_exactly_one_caller_in_the_page() {
        assert_eq!(
            QUEUE_SVELTE.matches("/api/queue/groups").count(),
            1,
            "every grouping must go through the cached loader in Queue.svelte"
        );
        assert!(
            QUEUE_SVELTE.contains("const groupCache"),
            "Queue.svelte must keep the listing across an unmount"
        );
    }

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
            crate::commands::review::why_nothing_landed(report).contains("cannot resolve subject"),
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
        assert!(!crate::commands::review::why_nothing_landed("").is_empty());
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
