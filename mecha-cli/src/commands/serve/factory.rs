//! The Factory view — the owner's instruments on the public surface, worked
//! from the page rather than by editing TOML on the machine.
//!
//! **Reads go through the factory's own programs; the one write is here.**
//! What a draft would offer is `factory-publish slots preview`, and whether a
//! manifest is one the box would take is `factory-publish type check` — the
//! factory judges its own vocabulary, and this side never re-decides it. The
//! write (a booking's hours, set in place in the manifest) lives in this
//! handler and nowhere else, for the charter's reason: a CLI verb that sets
//! availability is a verb a model's shell could run, and the hours are what
//! a stranger's prose would most like to change (`mecha_core::instrument`).
//!
//! **A save pushes.** The manifest is the file the slot timer reads, so a
//! saved edit reaches the page within the timer's two minutes regardless; the
//! push that follows a save makes it immediate, and its failure is reported
//! beside a save that stands — the timer retries, and the page says so.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use std::path::{Path, PathBuf};

use mecha_core::instrument::{self, Hours};

type St = State<super::WebState>;

fn err(status: StatusCode, e: impl std::fmt::Display) -> Response {
    (status, format!("{e:#}\n")).into_response()
}

/// GET /api/factory — every manifest at home, with the ones that cannot be
/// read named rather than dropped.
pub async fn list(State(_state): St) -> Response {
    let listed = tokio::task::spawn_blocking(|| -> anyhow::Result<_> {
        let dir = instrument::types_dir()?;
        Ok((dir.clone(), instrument::list_in(&dir)?))
    })
    .await;
    match listed {
        Ok(Ok((dir, all))) => Json(serde_json::json!({
            "dir": dir,
            "instruments": all,
        }))
        .into_response(),
        Ok(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

#[derive(Deserialize)]
pub struct IdQuery {
    pub id: String,
}

/// GET /api/factory/instrument?id= — one instrument: its hours, what they
/// offer now, the meetings it has booked, and how many requests wait.
pub async fn instrument(State(_state): St, Query(q): Query<IdQuery>) -> Response {
    let found = match tokio::task::spawn_blocking(move || load(&q.id)).await {
        Ok(Ok(found)) => found,
        Ok(Err(e)) => return err(StatusCode::NOT_FOUND, e),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    Json(detail(found).await).into_response()
}

/// An instrument and its file as read now.
struct Loaded {
    inst: instrument::Instrument,
    text: String,
}

fn load(id: &str) -> anyhow::Result<Loaded> {
    let inst = instrument::find_in(&instrument::types_dir()?, id)?;
    let text = std::fs::read_to_string(&inst.path)?;
    Ok(Loaded { inst, text })
}

async fn detail(found: Loaded) -> serde_json::Value {
    let Loaded { inst, text } = found;
    let mut out = serde_json::json!({
        "id": inst.id,
        "kind": inst.kind,
        "title": inst.title,
        "path": inst.path,
        "digest": instrument::digest(&text),
        "pipeline_days": instrument::PIPELINE_DAYS,
        // Whether the slot timer pushes this page; only then may the page
        // promise its retries or blame it for a stale push.
        "timer_pushes": inst.id == instrument::PIPELINE_INSTRUMENT,
        "days": instrument::DAYS,
    });
    let id = inst.id.clone();
    let (bookings, open, skipped) = tokio::task::spawn_blocking(move || meetings(&id))
        .await
        .unwrap_or_else(|e| (Err(anyhow::anyhow!("{e}")), None, 0));
    if skipped > 0 {
        out["requests_skipped"] = skipped.into();
    }
    match bookings {
        Ok(rows) => out["bookings"] = serde_json::Value::Array(rows),
        Err(e) => out["bookings_error"] = format!("{e:#}").into(),
    }
    // Unknown is not zero: a store that could not be read leaves the count
    // absent, and the page says so rather than showing "0 waiting".
    if let Some(open) = open {
        out["requests_open"] = open.into();
    }
    if inst.is_booking() {
        // Refused before the work, not after it: the save would turn an
        // oversized file away, so the editor never opens on one.
        if text.len() > instrument::MAX_MANIFEST_BYTES {
            out["hours_error"] =
                "this manifest is larger than any booking page's; edit it by hand".into();
        } else {
            match instrument::read_hours(&text) {
                Ok(hours) => out["hours"] = serde_json::to_value(hours).unwrap_or_default(),
                Err(e) => out["hours_error"] = format!("{e:#}").into(),
            }
        }
        match preview(&inst.id, &inst.path).await {
            Ok(p) => out["preview"] = p,
            Err(e) => out["preview_error"] = format!("{e:#}").into(),
        }
    }
    out
}

/// Upcoming meetings this instrument booked, from the front-door store, and
/// how many of its requests are open.
///
/// **Reconcile, then settle** — `serve/frontdoor.rs`'s order, and it is
/// load-bearing there for the same reason: settling skips `awaiting_me`, so
/// without the outbox pass first a booking whose reply went out keeps
/// counting as open, and one cancelled while a draft was staged keeps its
/// meeting. Both best-effort; a store that cannot be written is still read.
fn meetings(id: &str) -> (anyhow::Result<Vec<serde_json::Value>>, Option<usize>, usize) {
    use mecha_core::frontdoor::{self, Frontdoor};
    let store = match Frontdoor::open_default() {
        Ok(s) => s,
        Err(e) => return (Err(e), None, 0),
    };
    if let Some(outbox) = mecha_core::outbox::OutboxStore::open_existing_default() {
        if let Err(e) = store.reconcile(&outbox) {
            tracing::warn!(error = %e, "reconciling front-door drafts for the factory page");
        }
    }
    if let Err(e) = store.settle_bookings(&super::super::frontdoor::swept_bookings()) {
        tracing::warn!(error = %e, "settling bookings for the factory page");
    }
    // Counting what it skipped: a count over a store that dropped an
    // unreadable row is a partial denominator, and the page says so beside
    // the number rather than presenting it as the whole.
    let (records, skipped) = match store.records_counting() {
        Ok(r) => r,
        Err(e) => return (Err(e), None, 0),
    };
    let now = chrono::Utc::now();
    // Every booking a withdrawal names, joined here rather than trusted to
    // `state`: "Coming up" is a claim about the calendar, and a cancelled
    // meeting must leave it whatever last wrote the record's state.
    let withdrawn: std::collections::BTreeSet<String> =
        records.iter().filter_map(|r| r.cancellation()).collect();
    let mine: Vec<_> = records.iter().filter(|r| r.type_id == id).collect();
    let open = mine
        .iter()
        .filter(|r| r.booking().is_none() && frontdoor::counts_as_open(&r.state))
        .count();
    let mut rows: Vec<(String, serde_json::Value)> = mine
        .iter()
        .filter(|r| r.state != frontdoor::CLOSED)
        .filter_map(|r| r.booking().map(|b| (r, b)))
        .filter(|(_, b)| !withdrawn.contains(&b.booking_id))
        .filter(|(_, b)| chrono::DateTime::parse_from_rfc3339(&b.end).map_or(true, |end| end > now))
        .map(|(r, b)| {
            (
                b.start.clone(),
                serde_json::json!({
                    "seq": r.seq,
                    "start": b.start,
                    "end": b.end,
                    "duration_minutes": b.duration_minutes,
                    // Who is coming: the address the box proved the booker
                    // holds, the same field the front-door card shows.
                    "reply_to": r.reply_to,
                    "on_calendar": r.state == frontdoor::BOOKED,
                    "state": r.state,
                }),
            )
        })
        .collect();
    // By instant, not by string: the stamps are kept as the box wrote them,
    // and an offset other than `Z` would sort wrong lexically.
    rows.sort_by_key(|(start, _)| {
        chrono::DateTime::parse_from_rfc3339(start)
            .ok()
            .map(|t| t.with_timezone(&chrono::Utc))
    });
    (
        Ok(rows.into_iter().map(|(_, v)| v).collect()),
        Some(open),
        skipped,
    )
}

/// Run a sibling program by its absolute path. A user unit has no login's
/// `PATH`, which is why `find_program` also looks in `~/.cargo/bin`.
fn program(name: &str) -> anyhow::Result<PathBuf> {
    // Under test, a directory of stand-ins and nothing else: the real
    // `factory-publish` on a developer's machine pushes to a real box, so a
    // test that missed its stand-in must fail rather than fall through.
    #[cfg(test)]
    if let Some(dir) = TEST_BIN_DIR
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
    {
        return Ok(dir.join(name));
    }
    mecha_core::onboarding::find_program(name)
        .ok_or_else(|| anyhow::anyhow!("`{name}` is not installed on this machine"))
}

/// Where [`program`] finds its children in a test (`serve::tests`).
#[cfg(test)]
pub(super) static TEST_BIN_DIR: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Whether a child may be killed when its request goes away. A read
/// (`type check`, `slots preview`) may: a client that walked off wants it
/// gone. A publish (`slots push`) may not — `board.rs`'s `graph_verb` keeps
/// the same rule, because a write killed mid-flight leaves the owner told it
/// failed while it may have half-happened.
#[derive(Clone, Copy, PartialEq)]
enum Child {
    Read,
    Publish,
}

async fn run(
    name: &str,
    args: &[&std::ffi::OsStr],
    stdin: Option<Vec<u8>>,
    kind: Child,
) -> anyhow::Result<String> {
    use tokio::io::AsyncWriteExt;
    let mut cmd = tokio::process::Command::new(program(name)?);
    // Every serve child starts where `child_cwd` says, never in the unit's
    // home directory — the rule lives at the spawn helpers, and this is one.
    if let Some(dir) = super::child_cwd() {
        cmd.current_dir(dir);
    }
    cmd.args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(if stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .kill_on_drop(kind == Child::Read);
    let mut child = cmd.spawn()?;
    let pipe = child.stdin.take();
    // The write and the wait run together, inside the one timeout. Written
    // first and outside it, a child that neither reads its stdin nor exits
    // blocked the save for ever once the busy time outgrew the pipe buffer —
    // the timeout never started (review of #821; `hooks.rs` keeps the same
    // rule at the same spot).
    let feed = async move {
        let (Some(mut pipe), Some(bytes)) = (pipe, stdin) else {
            return Ok(());
        };
        // A child that refuses before reading its stdin (`slots push` checks
        // the manifest's id first) closes the pipe under the write. That is
        // the child's answer arriving early, not a failure of ours: carry on
        // and report its exit and its own refusal, which is what the owner
        // needs to read. Reported as "Broken pipe" it said nothing (CI on
        // #816's merge, where the stand-in exited before the write finished).
        let written = match pipe.write_all(&bytes).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
            Err(e) => Err(e),
        };
        // Closed here, the moment the write is done: a child reading to EOF
        // must see it now, not whenever `join!` drops a finished branch —
        // `wait_with_output` cannot close a stdin that was taken out of it.
        drop(pipe);
        written
    };
    let (fed, output) = tokio::time::timeout(std::time::Duration::from_secs(120), async {
        tokio::join!(feed, child.wait_with_output())
    })
    .await
    .map_err(|_| anyhow::anyhow!("`{name}` timed out"))?;
    fed?;
    let output = output?;
    if !output.status.success() {
        anyhow::bail!("{}", refusal(name, &output.stderr, output.status));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// What a child's failure says to the owner: the last line of its stderr
/// (the refusal itself, `Error: …`), or — for a child that died without a
/// word, which the early-exit path makes reachable — its exit status, never
/// a bare "failed".
fn refusal(name: &str, stderr: &[u8], status: std::process::ExitStatus) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    match stderr.lines().rfind(|l| !l.trim().is_empty()) {
        Some(line) => line.trim_start_matches("Error: ").to_string(),
        None => format!("`{name}` {status} and said nothing"),
    }
}

async fn preview(id: &str, manifest: &Path) -> anyhow::Result<serde_json::Value> {
    let out = run(
        "factory-publish",
        &[
            "slots".as_ref(),
            "preview".as_ref(),
            id.as_ref(),
            "--manifest".as_ref(),
            manifest.as_os_str(),
            "--json".as_ref(),
        ],
        None,
        Child::Read,
    )
    .await?;
    Ok(serde_json::from_str(&out)?)
}

/// A draft beside the manifest, removed when dropped: hidden and not
/// `.toml`, so neither the list nor anything globbing the directory sees it.
struct Draft(PathBuf);

impl Draft {
    fn write(beside: &Path, text: &str) -> anyhow::Result<Draft> {
        let dir = beside
            .parent()
            .ok_or_else(|| anyhow::anyhow!("a manifest path has a directory"))?;
        let path = dir.join(format!(".draft.{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&path, text)?;
        Ok(Draft(path))
    }
}

impl Drop for Draft {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The candidate file `hours` would make, checked by the factory as a whole
/// manifest, and the slots it would offer. Nothing on disk changes.
async fn judge(id: &str, path: &Path, candidate: &str) -> anyhow::Result<serde_json::Value> {
    let draft = Draft::write(path, candidate)?;
    run(
        "factory-publish",
        &["type".as_ref(), "check".as_ref(), draft.0.as_os_str()],
        None,
        Child::Read,
    )
    .await
    .map_err(|e| anyhow::anyhow!("the factory would refuse this manifest: {e:#}"))?;
    preview(id, &draft.0).await
}

#[derive(Deserialize, Clone)]
pub struct HoursBody {
    pub id: String,
    /// The digest of the manifest as the page read it.
    pub base: String,
    pub hours: Hours,
}

/// The candidate text, or the refusal as a response.
#[allow(clippy::result_large_err)]
fn candidate(body: &HoursBody) -> Result<(instrument::Instrument, String), Response> {
    let found = load(&body.id).map_err(|e| err(StatusCode::NOT_FOUND, e))?;
    if !found.inst.is_booking() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!("`{}` is not a booking page; it has no hours", body.id),
        ));
    }
    if found.text.len() > instrument::MAX_MANIFEST_BYTES {
        return Err(err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "this manifest is larger than any booking page's; edit it by hand",
        ));
    }
    match instrument::rewrite(&found.text, &body.base, &body.hours) {
        Ok(text) => Ok((found.inst, text)),
        Err(e) if e.downcast_ref::<instrument::Stale>().is_some() => {
            Err(err(StatusCode::CONFLICT, e))
        }
        Err(e) => Err(err(StatusCode::UNPROCESSABLE_ENTITY, e)),
    }
}

/// The most a preview or save body may be. A week of hours is a few
/// hundred bytes; the cap is what keeps one request from asking two child
/// processes to judge a manifest of fifty thousand windows (`validate` also
/// caps the counts, so what is written stays a manifest this page can read
/// back).
pub const MAX_HOURS_BODY: usize = 64 * 1024;

/// [`candidate`] off the executor, as `list` and `instrument` read the store:
/// it reads the directory and the manifest.
#[allow(clippy::result_large_err)]
async fn candidate_blocking(
    body: &HoursBody,
) -> Result<(instrument::Instrument, String), Response> {
    let body = body.clone();
    tokio::task::spawn_blocking(move || candidate(&body))
        .await
        .unwrap_or_else(|e| Err(err(StatusCode::INTERNAL_SERVER_ERROR, e)))
}

/// POST /api/factory/preview — what these hours would offer, without
/// saving them.
pub async fn preview_hours(State(_state): St, Json(body): Json<HoursBody>) -> Response {
    let (inst, text) = match candidate_blocking(&body).await {
        Ok(c) => c,
        Err(refused) => return refused,
    };
    match judge(&inst.id, &inst.path, &text).await {
        Ok(p) => Json(serde_json::json!({ "preview": p })).into_response(),
        Err(e) => err(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

/// POST /api/factory/hours — check, write, then push; each step's outcome
/// reported, and a push failure never undoes a save that stands.
pub async fn save_hours(State(_state): St, Json(body): Json<HoursBody>) -> Response {
    let (inst, text) = match candidate_blocking(&body).await {
        Ok(c) => c,
        Err(refused) => return refused,
    };
    // Judged before it is written: a manifest the factory refuses never
    // reaches the file the timer reads.
    if let Err(e) = judge(&inst.id, &inst.path, &text).await {
        return err(StatusCode::UNPROCESSABLE_ENTITY, e);
    }
    // The `base` guard again, at the write: the judging above spawned two
    // children and took seconds, and a second tab saving in that window
    // must get its 409 rather than have its hours silently replaced. The
    // text written is the one judged; only the file it replaces is re-read.
    let path = inst.path.clone();
    let base = body.base.clone();
    let wrote =
        tokio::task::spawn_blocking(move || instrument::replace_if_unchanged(&path, &base, &text))
            .await;
    match wrote {
        Ok(Ok(())) => {}
        Ok(Err(e)) if e.downcast_ref::<instrument::Stale>().is_some() => {
            return err(StatusCode::CONFLICT, e)
        }
        Ok(Err(e)) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
    tracing::info!(instrument = %inst.id, "hours saved from the factory page");
    let push = push(&inst.id, &inst.path).await;
    // From here the save stands, whatever else fails: a re-read that cannot
    // be made is reported beside it, as a failed push is, never as a failed
    // save — which would send the page back with the pre-write `base` and a
    // 409 for a file that is exactly what the owner asked for.
    let mut out = serde_json::json!({ "saved": true });
    match tokio::task::spawn_blocking(move || load(&body.id)).await {
        Ok(Ok(found)) => out["instrument"] = detail(found).await,
        Ok(Err(e)) => out["reread_error"] = format!("{e:#}").into(),
        Err(e) => out["reread_error"] = format!("{e:#}").into(),
    }
    match push {
        Ok(line) => {
            out["pushed"] = true.into();
            out["push"] = line.trim().into();
        }
        Err(e) => {
            out["pushed"] = false.into();
            out["push_error"] = format!("{e:#}").into();
        }
    }
    Json(out).into_response()
}

/// The slot timer's pipeline, once, now: `mecha-mail freebusy` into
/// `factory-publish slots push --manifest`, as `scripts/mecha-slots.service`
/// runs it.
async fn push(id: &str, manifest: &Path) -> anyhow::Result<String> {
    let days = instrument::PIPELINE_DAYS.to_string();
    let busy = run(
        "mecha-mail",
        &[
            "freebusy".as_ref(),
            "--days".as_ref(),
            days.as_ref(),
            "--json".as_ref(),
        ],
        None,
        Child::Read,
    )
    .await
    .map_err(|e| anyhow::anyhow!("reading your calendars: {e:#}"))?;
    run(
        "factory-publish",
        &[
            "slots".as_ref(),
            "push".as_ref(),
            id.as_ref(),
            "--manifest".as_ref(),
            manifest.as_os_str(),
        ],
        Some(busy.into_bytes()),
        Child::Publish,
    )
    .await
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn a_childs_refusal_is_its_last_line_or_its_exit_never_a_bare_failed() {
        let status = std::process::ExitStatus::from_raw(3 << 8);
        let said = super::refusal(
            "factory-publish",
            b"context\nError: the box is down\n\n",
            status,
        );
        assert_eq!(said, "the box is down");
        // Died without a word: the exit status is the only thing it said.
        let said = super::refusal("factory-publish", b"", status);
        assert!(
            said.contains("exit status: 3") && said.contains("factory-publish"),
            "{said}"
        );
    }
}
