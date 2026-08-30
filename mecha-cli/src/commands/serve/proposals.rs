//! The proposal stores on the phone: harness candidates, rule proposals and
//! the graph's entity proposals.
//!
//! Three stores, one surface — `commands::review::review_source` is the table
//! and this file holds no list of its own, on the same argument the TUI's
//! generic review level is built on: four stores that answer `list --json`
//! and take `show` / `accept` / `reject` are the same review with different
//! nouns, and a copy per surface is a copy to keep correct. The web had no
//! copy at all, which is the bug this fixes: `mecha harness ruminate` stages
//! anything outside the closed override set for a person, the doctor says so
//! at 72h, and the only place to answer was a terminal.
//!
//! **Every read and every mutation is a child process** — the `/queues` and
//! `/tasks` rule. Nothing reachable from a browser that a script cannot do,
//! and for the graph's store that is not merely a convention: its verbs live
//! in `mecha-graph`, a different binary entirely.
//!
//! **A decision needs the thing itself in front of it.** `accept` and
//! `reject` take a `read` flag the page sets only from a `show` it actually
//! rendered — the outbox's rule, one queue over. A harness candidate carries
//! a prediction, a rationale and the evidence the diagnostician saw, and
//! accepting a change to your own config off a one-line title is the failure
//! this queue exists to prevent. The page greys the buttons; this refuses
//! them, because a greyed button is a suggestion and the store is where the
//! rule has to hold.

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::commands::review::{review_from_json, review_source, ReviewRow, ReviewSource};

type St = State<super::WebState>;

/// The stores this pane offers, and the queue name each one is keyed by.
///
/// The URL segment is short because it is a hash route a person can type
/// (`#review/harness`); the queue name is the one `collect_queues` emits and
/// the only key `review_source` knows. `graph shadow` is deliberately absent:
/// it has its own pane already, riding the sample deck's endpoints.
const STORES: [(&str, &str); 3] = [
    ("harness", "harness changes"),
    ("rules", "rule proposals"),
    ("entities", "graph entities"),
];

fn queue_of(store: &str) -> Option<&'static str> {
    STORES
        .iter()
        .find(|(k, _)| *k == store)
        .map(|(_, queue)| *queue)
}

/// Resolve a URL segment to the argv behind it, or the refusal a bad one
/// deserves. An unknown store is the caller's error and says so by name —
/// this is a hash route, so it is reachable by typing.
///
/// The refusal is boxed on `review.rs`'s rule: an axum `Response` is 128
/// bytes, and a `Result` whose error dwarfs its success value is paid for by
/// every call that never fails (`clippy::result_large_err`, which the CI
/// toolchain enforces).
fn source_of(store: &str) -> Result<(&'static str, ReviewSource), Box<Response>> {
    let queue = queue_of(store).ok_or_else(|| {
        let known: Vec<&str> = STORES.iter().map(|(k, _)| *k).collect();
        Box::new(
            (
                StatusCode::NOT_FOUND,
                format!(
                    "no proposal store named `{store}` (known: {})\n",
                    known.join(", ")
                ),
            )
                .into_response(),
        )
    })?;
    let src = review_source(queue).ok_or_else(|| {
        // Reachable only if the table and this list disagree — which is what
        // `every_store_the_web_offers_is_a_reviewable_queue` exists to catch
        // at build time. Say which one moved rather than 500ing anonymously.
        Box::new(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("queue `{queue}` is no longer a reviewable-proposal store\n"),
            )
                .into_response(),
        )
    })?;
    Ok((queue, src))
}

/// Where `mecha-graph` lives, on `commands::review`'s rule: `$MECHA_GRAPH_BIN`
/// first, then the name on `PATH`, and never `mecha.toml`.
fn graph_bin() -> String {
    std::env::var("MECHA_GRAPH_BIN").unwrap_or_else(|_| "mecha-graph".into())
}

/// Run the store's own binary and hand back stdout, or the response the
/// failure deserves.
///
/// A missing `mecha-graph` answers **503 with the variable that fixes it**,
/// not a 500 and not an empty list: this pane's whole reason for existing is
/// that a queue grew unnoticed, and a reader that rendered its own inability
/// to look as "nothing waiting" would reproduce exactly that.
async fn run(src: &ReviewSource, argv: &[String]) -> Result<String, Response> {
    let bin = if src.graph {
        std::path::PathBuf::from(graph_bin())
    } else {
        crate::exe::self_exe()
    };
    let mut cmd = tokio::process::Command::new(&bin);
    if let Some(dir) = super::child_cwd() {
        cmd.current_dir(dir);
    }
    let out =
        match tokio::time::timeout(std::time::Duration::from_secs(60), cmd.args(argv).output())
            .await
        {
            Err(_) => {
                return Err((StatusCode::GATEWAY_TIMEOUT, "the verb timed out\n").into_response())
            }
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound && src.graph => {
                let bin = graph_bin();
                return Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!(
                        "`{bin}` not found — install mecha-graph, or set MECHA_GRAPH_BIN \
                     to its path. The other stores work without it.\n"
                    ),
                )
                    .into_response());
            }
            Ok(Err(e)) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("spawning: {e:#}\n"),
                )
                    .into_response())
            }
            Ok(Ok(out)) => out,
        };
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).to_string());
    }
    // The reason may be on either stream — `mecha-graph` reports per-item
    // failures on stdout and exits non-zero, which is how `bind 2951` came to
    // report "failed" while stdout held the whole answer.
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let reason = stderr
        .trim()
        .lines()
        .next()
        .filter(|l| !l.trim().is_empty())
        .or_else(|| stdout.trim().lines().last())
        .unwrap_or("failed")
        .to_string();
    Err((StatusCode::CONFLICT, format!("{reason}\n")).into_response())
}

#[derive(Serialize)]
struct StoreRow {
    store: &'static str,
    /// The store's own word for what it holds ("harness candidates").
    label: String,
    /// `None` when the store could not be read — a dash on the chip, never a
    /// zero. "Nothing waiting" and "could not look" are opposite findings.
    depth: Option<usize>,
    detail: String,
    oldest: Option<String>,
    /// The verb that opens this store in a terminal, kept on the surface so
    /// the phone never becomes the only way to reach it.
    opens: String,
}

/// GET /api/proposals — the three stores and what each is holding, off one
/// `mecha review queues --json`, so the chips carry counts before anything is
/// opened. The depths are the home page's own, from the same reader.
pub async fn stores(State(state): St) -> Response {
    let queues = match super::review::self_json(&state, &["review", "queues", "--json"]).await {
        Ok(v) => v,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response();
        }
    };
    let rows: Vec<StoreRow> = STORES
        .iter()
        .filter_map(|(store, queue)| {
            let src = review_source(queue)?;
            let q = queues
                .as_array()?
                .iter()
                .find(|q| q["queue"].as_str() == Some(queue));
            Some(StoreRow {
                store,
                label: src.label,
                // `as_u64` on a JSON null is None, which is the dash the
                // command already meant by it — a missing row is the same
                // finding as an unreadable store, not an empty one.
                depth: q.and_then(|q| q["depth"].as_u64()).map(|n| n as usize),
                detail: q
                    .and_then(|q| q["detail"].as_str())
                    .unwrap_or("")
                    .to_string(),
                oldest: q.and_then(|q| q["oldest"].as_str()).map(str::to_string),
                opens: q
                    .and_then(|q| q["opens"].as_str())
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect();
    Json(rows).into_response()
}

#[derive(Serialize)]
struct Listing {
    label: String,
    rows: Vec<ReviewRow>,
}

/// GET /api/proposals/{store} — the store's pending items, normalized by the
/// one parser all four stores share.
pub async fn list(State(_state): St, UrlPath(store): UrlPath<String>) -> Response {
    let (_, src) = match source_of(&store) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let raw = match run(&src, &src.list).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    match review_from_json(&raw) {
        Ok(rows) => Json(Listing {
            label: src.label,
            rows,
        })
        .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            format!("reading `{}`: {e:#}\n", src.list.join(" ")),
        )
            .into_response(),
    }
}

#[derive(Serialize)]
struct Detail {
    text: String,
}

/// GET /api/proposals/{store}/{id} — the whole item as its own verb renders
/// it: prediction, rationale, and the evidence the diagnostician saw. Text
/// rather than fields, because `show` is the store's account of itself and a
/// second renderer here would be a second thing to keep true.
pub async fn detail(
    State(_state): St,
    UrlPath((store, id)): UrlPath<(String, String)>,
) -> Response {
    let (_, src) = match source_of(&store) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let mut argv = src.show.clone();
    argv.push(id);
    match run(&src, &argv).await {
        Ok(text) => Json(Detail { text }).into_response(),
        Err(r) => r,
    }
}

#[derive(Deserialize)]
pub struct DecideBody {
    /// Why — required on a reject, recorded on the item for the next reader
    /// and mined by the learning miner. Ignored by the graph's store, which
    /// takes no `--reason`.
    #[serde(default)]
    pub reason: String,
    /// The page rendered this item's `show` before offering the buttons.
    /// Sent explicitly rather than inferred from a prior GET: a session that
    /// merely *could* have read it is not one that did, and the store is
    /// where "read before you decide" has to hold — a greyed button is only
    /// a suggestion.
    #[serde(default)]
    pub read: bool,
}

async fn decide(store: &str, id: &str, accepting: bool, body: DecideBody) -> Response {
    let (_, src) = match source_of(store) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    if !body.read {
        return (
            StatusCode::BAD_REQUEST,
            "read the item before deciding it — GET this id first\n",
        )
            .into_response();
    }
    let reason = body.reason.trim();
    if !accepting && !src.graph && reason.is_empty() {
        return (StatusCode::BAD_REQUEST, "a reject needs a reason\n").into_response();
    }
    let mut argv = if accepting {
        src.accept.clone()
    } else {
        src.reject.clone()
    };
    argv.push(id.to_string());
    if !accepting && !src.graph {
        argv.push("--reason".into());
        argv.push(reason.to_string());
    }
    match run(&src, &argv).await {
        // The child's own first line, never a sentence composed here: an
        // accept can *apply* something — an override layer entry, a merge —
        // and what it did is the child's to report.
        Ok(out) => Json(serde_json::json!({
            "ok": true,
            "output": out.trim(),
            "said": out.trim().lines().next().unwrap_or("").to_string(),
        }))
        .into_response(),
        Err(r) => r,
    }
}

/// POST /api/proposals/{store}/{id}/accept
pub async fn accept(
    State(_state): St,
    UrlPath((store, id)): UrlPath<(String, String)>,
    Json(body): Json<DecideBody>,
) -> Response {
    decide(&store, &id, true, body).await
}

/// POST /api/proposals/{store}/{id}/reject
pub async fn reject(
    State(_state): St,
    UrlPath((store, id)): UrlPath<(String, String)>,
    Json(body): Json<DecideBody>,
) -> Response {
    decide(&store, &id, false, body).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every store this pane offers must still be a queue the backlog reports
    /// *and* a store `review_source` knows the verbs for. A renamed queue
    /// would otherwise leave the pane routing to a 500 — the same silent half
    /// the home page's `queueTargets` guard exists to catch, one layer down.
    #[test]
    fn every_store_the_web_offers_is_a_reviewable_queue() {
        let names: Vec<&str> = include_str!("../review.rs")
            .lines()
            .filter_map(|l| l.trim().strip_prefix("name: \""))
            .filter_map(|l| l.split_once('"'))
            .map(|(n, _)| n)
            .collect();
        assert!(
            names.contains(&"harness changes"),
            "the queue-row extraction found {names:?} — did the rows move?"
        );
        for (store, queue) in STORES {
            assert!(
                names.contains(&queue),
                "the web offers store {store:?} for queue {queue:?}, which \
                 `collect_queues` no longer reports"
            );
            assert!(
                review_source(queue).is_some(),
                "queue {queue:?} has no argv in `review_source` — the pane \
                 would list it and then fail to open it"
            );
        }
    }

    /// The pane's URL segments are what `Home.svelte` routes to and what
    /// `Review.svelte` names as panes, so a store added here without a pane
    /// is a card with a chevron that lands nowhere.
    #[test]
    fn every_store_has_a_pane_and_a_card_that_reaches_it() {
        let review = include_str!("../../../../web/src/lib/Review.svelte");
        let home = include_str!("../../../../web/src/lib/Home.svelte");
        for (store, queue) in STORES {
            assert!(
                review.contains(&format!("'{store}'")),
                "Review.svelte names no pane {store:?} — the hash route \
                 `#review/{store}` falls back to the outbox"
            );
            assert!(
                home.contains(&format!("'{queue}': 'review/{store}'")),
                "Home.svelte's queueTargets does not route {queue:?} to \
                 `review/{store}` — the card stays flat while the pane exists"
            );
        }
    }
}
