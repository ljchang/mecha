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
/// The budget for one child. Named rather than spelled inline because the
/// paragraph in [`run`] argues for a number, and an argument beside a literal
/// is two things that can disagree — which they promptly did: the prose said
/// 120 and the call said 60 for a whole commit, and the reviewer trusted the
/// prose, as a reader should.
///
/// Matches `board.rs::graph_verb` and `verb_output` deliberately: `graph_verb`
/// reaches the *same* `mecha-graph` merge from the entity page, and a merge
/// that succeeds from one surface and reports a timeout from the other is the
/// worst answer available on the one store whose verb has no undo.
const VERB_TIMEOUT_SECS: u64 = 120;

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
///
/// Boxed for the same reason [`source_of`] is, and this one is why the rule
/// is written down: an `async fn` hides the `Result` inside a future, so the
/// 1.97 clippy on this box saw nothing while CI's 1.98 failed the build on
/// it. Reach for `cargo +1.98.0 clippy --workspace --all-targets
/// --all-features` before believing a green local lint.
async fn run(src: &ReviewSource, argv: &[String]) -> Result<String, Box<Response>> {
    let bin = if src.graph {
        std::path::PathBuf::from(graph_bin())
    } else {
        crate::exe::self_exe()
    };
    let mut cmd = tokio::process::Command::new(&bin);
    if let Some(dir) = super::child_cwd() {
        cmd.current_dir(dir);
    }
    let out = match tokio::time::timeout(
        std::time::Duration::from_secs(VERB_TIMEOUT_SECS),
        cmd.args(argv).output(),
    )
    .await
    {
        // Dropping `output()` does NOT kill the child — `tokio::process`
        // sets no `kill_on_drop` — so the verb behind this is still
        // running, and for `entities` it can be a merge with no undo.
        // Reporting a bare timeout invites the owner to accept again
        // something that has already applied. Say what is actually known,
        // the way `board.rs::graph_verb` does over the same child. The
        // budget matches it and `verb_output` too: 120s, because the
        // slowest work on this surface lives here.
        Err(_) => {
            return Err(Box::new(
                (
                    StatusCode::GATEWAY_TIMEOUT,
                    "the verb is still running — it was not cancelled, and it may yet \
                         finish. Re-open this queue to see what actually happened before \
                         deciding it again.\n",
                )
                    .into_response(),
            ))
        }
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound && src.graph => {
            let bin = graph_bin();
            return Err(Box::new(
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!(
                        "`{bin}` not found — install mecha-graph, or set MECHA_GRAPH_BIN \
                         to its path. The other stores work without it.\n"
                    ),
                )
                    .into_response(),
            ));
        }
        Ok(Err(e)) => {
            return Err(Box::new(
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("spawning: {e:#}\n"),
                )
                    .into_response(),
            ))
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
    Err(Box::new(
        (StatusCode::CONFLICT, format!("{reason}\n")).into_response(),
    ))
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
        Err(r) => return *r,
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
    // The id is a URL path segment, so it is caller-controlled text, and
    // without the separator `GET /api/proposals/harness/--help` runs
    // `mecha harness show --help`, exits 0, and renders clap's help as the
    // candidate. Every other child spawn on this surface already separates —
    // the "these ids come from the store" assumption stops holding the moment
    // one arrives from a URL instead.
    argv.push("--".into());
    argv.push(id);
    match run(&src, &argv).await {
        Ok(text) => Json(Detail { text }).into_response(),
        Err(r) => *r,
    }
}

/// Did the graph's own report say the item was not decided, on a run that
/// still exited 0?
///
/// `mecha-graph`'s `proposals accept` applies first and decides second, on
/// purpose — "a proposal marked accepted whose repair then failed is a lie
/// the queue keeps telling". But its failure arms `println!` and fall
/// through, so the process exits 0 and every exit-code check calls it a
/// success. The web surface then toasts the child's own line — which reads
/// `#125: NOT applied — …` — as if it were the report of a success, closes
/// the detail, and shows the item still sitting in the reloaded list.
///
/// So the markers are read rather than the status. Coupled to another
/// binary's wording deliberately and narrowly: these two strings are the
/// whole of its error vocabulary here, `graph_failure_markers_match_the_
/// graphs_own_arms` pins them, and a marker that stops matching fails
/// closed the safe way — back to trusting the exit code, which is where
/// every other store already is.
fn graph_item_failed(out: &str) -> Option<&str> {
    out.lines()
        .map(str::trim)
        .find(|l| l.contains("NOT applied") || l.contains("no such proposal"))
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

/// The argv for one decision, as a function so it can be tested — the bug
/// that forced this out of the handler could not be reached by any test that
/// did not spawn a child, and so shipped under a green suite.
///
/// **Order is the whole content.** `--` is clap's escape: everything after it
/// matches *positionals only*, option parsing off for the remainder. Both
/// reject verbs take one positional and `--reason` as a flag, so a `--reason`
/// pushed after the separator arrives as an unexpected second positional and
/// the verb exits non-zero — which made reject fail 100% of the time on the
/// two stores that require a reason, i.e. every reject this handler can
/// perform. The separator still earns its place, because the id arrives from
/// a URL; it just has to come last, with every flag ahead of it.
fn decide_argv(src: &ReviewSource, id: &str, accepting: bool, reason: &str) -> Vec<String> {
    let mut argv = if accepting {
        src.accept.clone()
    } else {
        src.reject.clone()
    };
    if !accepting && !src.graph {
        argv.push("--reason".into());
        argv.push(reason.to_string());
    }
    argv.push("--".into()); // caller-controlled, exactly as in `detail`
    argv.push(id.to_string());
    argv
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
    let argv = decide_argv(&src, id, accepting, reason);
    match run(&src, &argv).await {
        // A zero exit is not the graph's answer — see `graph_item_failed`.
        // Checked before the success shape is built, so a refusal can never
        // be dressed as one.
        Ok(out) => match graph_item_failed(&out).filter(|_| src.graph) {
            Some(line) => (StatusCode::CONFLICT, format!("{line}\n")).into_response(),
            // The child's own first line, never a sentence composed here: an
            // accept can *apply* something — an override layer entry, a merge
            // — and what it did is the child's to report.
            None => Json(serde_json::json!({
                "ok": true,
                "output": out.trim(),
                "said": out.trim().lines().next().unwrap_or("").to_string(),
            }))
            .into_response(),
        },
        Err(r) => *r,
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

    /// The two strings `graph_item_failed` keys on are `mecha-graph`'s own
    /// error arms, quoted from its `ProposalAction::Accept` loop. If it ever
    /// rewords them this test still passes — nothing here can see that repo —
    /// so the value is the opposite direction: it pins OUR side, so a
    /// well-meant tidy of the markers cannot quietly restore the bug where a
    /// failed merge toasted as a success.
    #[test]
    fn graph_failure_markers_match_the_graphs_own_arms() {
        // Verbatim from `mecha-graph`'s accept loop.
        assert_eq!(
            graph_item_failed("#125: NOT applied — node 4021 has no such alias"),
            Some("#125: NOT applied — node 4021 has no such alias"),
        );
        assert_eq!(
            graph_item_failed("#900: no such proposal"),
            Some("#900: no such proposal"),
        );
        // A failure anywhere in a multi-item report is still a failure — the
        // first line being fine is what made this look like a success.
        assert_eq!(
            graph_item_failed("#1: merged 12 facts\n#2: NOT applied — busy"),
            Some("#2: NOT applied — busy"),
        );
        // And the success arms must not read as failures, or every decision
        // 409s and the queue cannot be worked at all.
        assert_eq!(graph_item_failed("#125: merged 12 facts, 3 aliases"), None);
        assert_eq!(graph_item_failed("#125: rejected"), None);
        assert_eq!(graph_item_failed(""), None);
    }

    /// The graph's `proposals list` defaults to `--limit 20` while the depth
    /// beside it counts every pending row, so an unlimited-looking call shows
    /// 20 of 45 and says nothing. Only the graph store needs it: the other
    /// two answer with their whole store.
    #[test]
    fn the_graph_listing_asks_for_more_than_the_default_twenty() {
        let src =
            review_source("graph entities").expect("the graph store must still be reviewable");
        let argv = src.list.join(" ");
        assert!(
            argv.contains("--limit"),
            "the graph listing must name a limit, or it silently takes 20: {argv}"
        );
        // Read back out of the argv rather than compared as a constant:
        // asserting on the const directly is a comparison the compiler folds,
        // which clippy calls out as an assertion with a constant value — and
        // it would also pass while the argv named something else entirely.
        let limit: usize = src
            .list
            .iter()
            .skip_while(|a| *a != "--limit")
            .nth(1)
            .and_then(|v| v.parse().ok())
            .expect("--limit must be followed by a number");
        assert!(
            limit > 20,
            "a limit at or under the verb's own default is ceremony around the same truncation"
        );
        for store in ["harness changes", "rule proposals"] {
            let src = review_source(store).expect(store);
            assert!(
                !src.list.join(" ").contains("--limit"),
                "{store} takes no --limit; passing one would make the verb fail to parse"
            );
        }
    }

    /// Nothing after clap's `--` may look like a flag, or it is read as a
    /// positional and the verb refuses the whole call. This is the test the
    /// second wave did not have: the argv lived inside an async handler, no
    /// test in this module spawns a child, and so `--reason` sat after the
    /// separator through a fully green suite while every reject 400ed at the
    /// child. Asserting on the composed argv is what makes it reachable.
    #[test]
    fn no_flag_is_ever_pushed_after_the_separator() {
        for store in ["harness changes", "rule proposals", "graph entities"] {
            let src = review_source(store).expect(store);
            for accepting in [true, false] {
                let argv = decide_argv(&src, "the-id", accepting, "because");
                let Some(sep) = argv.iter().position(|a| a == "--") else {
                    panic!("{store}: the separator must be present — the id comes from a URL");
                };
                assert_eq!(
                    sep,
                    argv.len() - 2,
                    "{store} (accepting={accepting}): `--` must be second-to-last so only \
                     the id follows it, got {argv:?}"
                );
                assert!(
                    !argv[sep + 1..].iter().any(|a| a.starts_with('-')),
                    "{store} (accepting={accepting}): {argv:?} puts a flag after `--`, which \
                     clap reads as an unexpected positional and refuses"
                );
            }
        }
    }

    /// A reject on the two stores that keep one must actually carry it, and
    /// ahead of the separator. The graph store takes no `--reason` at all.
    #[test]
    fn a_reason_rides_ahead_of_the_separator_and_only_where_it_is_kept() {
        for store in ["harness changes", "rule proposals"] {
            let src = review_source(store).expect(store);
            let argv = decide_argv(&src, "id", false, "the evidence does not support it");
            let at = argv.iter().position(|a| a == "--reason").expect(store);
            assert_eq!(argv[at + 1], "the evidence does not support it");
            assert!(
                at < argv.iter().position(|a| a == "--").unwrap(),
                "{argv:?}"
            );
        }
        let graph = review_source("graph entities").expect("graph entities");
        let argv = decide_argv(&graph, "125", false, "typed but not kept");
        assert!(
            !argv.iter().any(|a| a == "--reason"),
            "mecha-graph takes no --reason; passing one makes the verb refuse: {argv:?}"
        );
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
