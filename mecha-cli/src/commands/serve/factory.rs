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

use mecha_core::form_draft::{self, FormEdit};
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
        let served = instrument::list_in(&dir)?;
        let drafts = instrument::list_in(&form_draft::drafts_dir()?)?;
        Ok((dir, served, drafts))
    })
    .await;
    match listed {
        Ok(Ok((dir, served, drafts))) => {
            // Each served form says whether a draft of it waits; a draft of a
            // form never published is listed on its own, as unpublished.
            let mut rows: Vec<serde_json::Value> = served
                .iter()
                .map(|i| {
                    let mut v = serde_json::to_value(i).unwrap_or_default();
                    v["has_draft"] = drafts.iter().any(|d| d.id == i.id).into();
                    v
                })
                .collect();
            for d in drafts
                .iter()
                .filter(|d| !served.iter().any(|i| i.id == d.id))
            {
                let mut v = serde_json::to_value(d).unwrap_or_default();
                v["draft_only"] = true.into();
                v["has_draft"] = true.into();
                rows.push(v);
            }
            Json(serde_json::json!({ "dir": dir, "instruments": rows })).into_response()
        }
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
    if let Some(bytes) = stdin {
        let mut pipe = child.stdin.take().expect("piped");
        // A child that refuses before reading its stdin (`slots push` checks
        // the manifest's id first) closes the pipe under the write. That is
        // the child's answer arriving early, not a failure of ours: carry on
        // and report its exit and its own refusal, which is what the owner
        // needs to read. Reported as "Broken pipe" it said nothing (CI on
        // #816's merge, where the stand-in exited before the write finished).
        match pipe.write_all(&bytes).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
            Err(e) => return Err(e.into()),
        }
        drop(pipe);
    }
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("`{name}` timed out"))??;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // The last line is the refusal itself (`Error: …`), which is what
        // the owner needs to read; the rest is context the journal keeps.
        anyhow::bail!(
            "{}",
            stderr
                .lines()
                .rfind(|l| !l.trim().is_empty())
                .unwrap_or("failed")
                .trim_start_matches("Error: ")
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
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

// ── Forms: drafts, preview, publish ─────────────────────────────────────
//
// The served copy is the factory's record of what strangers see, so the
// owner edits a draft beside it and Publish is `type push` of the draft
// (`mecha_core::form_draft`). Every write here is the owner's, from this
// page only, as the hours are.

/// The most a form save body may be: a manifest is a few kilobytes.
pub const MAX_FORM_BODY: usize = 256 * 1024;

/// A form's working text: its draft if one waits, else the served copy.
struct Working {
    id: String,
    kind: String,
    text: String,
    draft: bool,
    served: Option<String>,
}

fn working(id: &str) -> anyhow::Result<Working> {
    form_draft::valid_id(id)?;
    let draft_path = form_draft::draft_path(&form_draft::drafts_dir()?, id)?;
    let served = instrument::list_in(&instrument::types_dir()?)?
        .into_iter()
        .find(|i| i.id == id)
        .map(|i| std::fs::read_to_string(&i.path))
        .transpose()?;
    let (text, draft) = match std::fs::read_to_string(&draft_path) {
        Ok(t) => (t, true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => match &served {
            Some(t) => (t.clone(), false),
            None => anyhow::bail!("no form `{id}`, served or drafted"),
        },
        Err(e) => return Err(e.into()),
    };
    let kind = if text.contains("kind = \"booking\"") {
        "booking"
    } else {
        "request"
    }
    .to_string();
    Ok(Working {
        id: id.to_string(),
        kind,
        text,
        draft,
        served,
    })
}

fn form_state(w: &Working) -> serde_json::Value {
    let mut out = serde_json::json!({
        "id": w.id,
        "kind": w.kind,
        "draft": w.draft,
        "published": w.served.is_some(),
        "digest": instrument::digest(&w.text),
        "text": w.text,
        "kinds": form_draft::KINDS,
    });
    match form_draft::read_form(&w.text) {
        Ok(f) => out["form"] = serde_json::to_value(f).unwrap_or_default(),
        Err(e) => out["form_error"] = format!("{e:#}").into(),
    }
    out
}

/// GET /api/factory/starters — what a new form can begin as.
pub async fn starters(State(_state): St) -> Response {
    match run(
        "factory-publish",
        &["type".as_ref(), "starters".as_ref()],
        None,
        Child::Read,
    )
    .await
    {
        Ok(out) => {
            let rows: Vec<serde_json::Value> = out
                .lines()
                .filter_map(|l| l.split_once('\t'))
                .map(|(name, title)| serde_json::json!({ "name": name, "title": title }))
                .collect();
            Json(serde_json::json!({ "starters": rows })).into_response()
        }
        Err(e) => err(StatusCode::BAD_GATEWAY, e),
    }
}

/// GET /api/factory/form?id= — the form as the editor shows it.
pub async fn form(State(_state): St, Query(q): Query<IdQuery>) -> Response {
    match tokio::task::spawn_blocking(move || working(&q.id)).await {
        Ok(Ok(w)) => Json(form_state(&w)).into_response(),
        Ok(Err(e)) => err(StatusCode::NOT_FOUND, e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

#[derive(Deserialize, Clone)]
pub struct FormSave {
    pub id: String,
    /// The digest of the text the page read (the draft, or the served copy).
    pub base: String,
    #[serde(default)]
    pub form: Option<FormEdit>,
    /// The whole manifest — the "Edit as TOML" save.
    #[serde(default)]
    pub raw: Option<String>,
}

#[allow(clippy::result_large_err)]
/// POST /api/factory/form/save — write the draft, never the served copy.
/// Checked by the factory before it lands, so a draft that will not parse
/// never sits waiting to be published.
pub async fn form_save(State(_state): St, Json(body): Json<FormSave>) -> Response {
    let b = body.clone();
    let prepared = tokio::task::spawn_blocking(move || -> Result<(Working, String), Response> {
        let w = working(&b.id).map_err(|e| err(StatusCode::NOT_FOUND, e))?;
        if instrument::digest(&w.text) != b.base {
            return Err(err(StatusCode::CONFLICT, instrument::Stale));
        }
        let next = match (b.form, b.raw) {
            (Some(f), None) => form_draft::write_form(&w.text, &f)
                .map_err(|e| err(StatusCode::UNPROCESSABLE_ENTITY, e))?,
            (None, Some(raw)) => raw,
            _ => {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    "a form save carries `form` or `raw`, one of them",
                ))
            }
        };
        if next.len() > MAX_FORM_BODY {
            return Err(err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "a manifest this large is not a form",
            ));
        }
        Ok((w, next))
    })
    .await;
    let (w, next) = match prepared {
        Ok(Ok(p)) => p,
        Ok(Err(refused)) => return refused,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    // The id is the file's name and the form's URL: a raw edit may not move
    // it. Checked before the factory is asked anything.
    let id_line = format!("id = \"{}\"", w.id);
    if !next.lines().any(|l| l.trim() == id_line) {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("the id must stay `{}`: it names the form's URL", w.id),
        );
    }
    let dir = match form_draft::drafts_dir() {
        Ok(d) => d,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, e);
    }
    let draft = match Draft::write(&dir.join("x"), &next) {
        Ok(d) => d,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    match run(
        "factory-publish",
        &["type".as_ref(), "check".as_ref(), draft.0.as_os_str()],
        None,
        Child::Read,
    )
    .await
    {
        Ok(_) => {}
        Err(e) => {
            return err(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("the factory would refuse this form: {e:#}"),
            )
        }
    }
    let path = match form_draft::draft_path(&dir, &w.id) {
        Ok(p) => p,
        Err(e) => return err(StatusCode::BAD_REQUEST, e),
    };
    let base_text = w.text.clone();
    let wrote = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        // Guard at the write, as the hours save does: the page read either
        // the draft or the served copy, and neither may have moved since.
        let now = working(&w.id)?;
        if now.text != base_text {
            anyhow::bail!(instrument::Stale);
        }
        instrument::replace_file(&path, &next)
    })
    .await;
    match wrote {
        Ok(Ok(())) => {}
        Ok(Err(e)) if e.downcast_ref::<instrument::Stale>().is_some() => {
            return err(StatusCode::CONFLICT, e)
        }
        Ok(Err(e)) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
    let id = body.id.clone();
    match tokio::task::spawn_blocking(move || working(&id)).await {
        Ok(Ok(w)) => Json(form_state(&w)).into_response(),
        Ok(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

#[derive(Deserialize)]
pub struct IdBody {
    pub id: String,
    #[serde(default)]
    pub base: Option<String>,
}

/// The page a form serves, made safe to show inside the owner's page: its
/// stylesheet inlined and every script removed, for a `sandbox` frame with
/// no network and no scripts at all.
fn preview_html(html: &str, style: &str) -> String {
    let mut out = String::with_capacity(html.len() + style.len());
    let mut rest = html;
    loop {
        let script = rest.find("<script");
        let link = rest.find("<link");
        let next = match (script, link) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => break,
        };
        out.push_str(&rest[..next]);
        let tail = &rest[next..];
        if tail.starts_with("<script") {
            match tail.find("</script>") {
                Some(end) => rest = &tail[end + "</script>".len()..],
                None => {
                    rest = "";
                    break;
                }
            }
        } else {
            match tail.find('>') {
                Some(end) => rest = &tail[end + 1..],
                None => {
                    rest = "";
                    break;
                }
            }
        }
    }
    out.push_str(rest);
    let styled = format!("<style>{style}</style>");
    match out.find("</head>") {
        Some(at) => format!("{}{styled}{}", &out[..at], &out[at..]),
        None => format!("{styled}{out}"),
    }
}

/// POST /api/factory/form/render — what the working text would serve.
pub async fn form_render(State(_state): St, Json(body): Json<IdBody>) -> Response {
    let w = match tokio::task::spawn_blocking(move || working(&body.id)).await {
        Ok(Ok(w)) => w,
        Ok(Err(e)) => return err(StatusCode::NOT_FOUND, e),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let dir = match form_draft::drafts_dir().and_then(|d| {
        std::fs::create_dir_all(&d)?;
        Ok(d)
    }) {
        Ok(d) => d,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let draft = match Draft::write(&dir.join("x"), &w.text) {
        Ok(d) => d,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    match run("factory-publish", &["type".as_ref(), "render".as_ref(), draft.0.as_os_str()], None, Child::Read).await {
        Ok(out) => match serde_json::from_str::<serde_json::Value>(&out) {
            Ok(page) => Json(serde_json::json!({
                "kind": page["kind"],
                "html": preview_html(page["html"].as_str().unwrap_or(""), page["style"].as_str().unwrap_or("")),
            }))
            .into_response(),
            Err(e) => err(StatusCode::BAD_GATEWAY, e),
        },
        Err(e) => err(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

#[derive(Deserialize)]
pub struct NewForm {
    pub starter: String,
    pub id: String,
}

/// POST /api/factory/form/new — a draft from a starter, under a new id.
pub async fn form_new(State(_state): St, Json(body): Json<NewForm>) -> Response {
    if let Err(e) = form_draft::valid_id(&body.id) {
        return err(StatusCode::BAD_REQUEST, e);
    }
    let id = body.id.clone();
    let taken = tokio::task::spawn_blocking(move || working(&id).is_ok())
        .await
        .unwrap_or(true);
    if taken {
        return err(
            StatusCode::CONFLICT,
            format!("a form called `{}` already exists", body.id),
        );
    }
    let text = match run(
        "factory-publish",
        &[
            "type".as_ref(),
            "new".as_ref(),
            body.starter.as_ref(),
            "--id".as_ref(),
            body.id.as_ref(),
        ],
        None,
        Child::Read,
    )
    .await
    {
        Ok(t) => t,
        Err(e) => return err(StatusCode::UNPROCESSABLE_ENTITY, e),
    };
    let id = body.id.clone();
    let wrote = tokio::task::spawn_blocking(move || -> anyhow::Result<Working> {
        let dir = form_draft::drafts_dir()?;
        std::fs::create_dir_all(&dir)?;
        instrument::replace_file(&form_draft::draft_path(&dir, &id)?, &text)?;
        working(&id)
    })
    .await;
    match wrote {
        Ok(Ok(w)) => Json(form_state(&w)).into_response(),
        Ok(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

/// POST /api/factory/form/publish — `type push` of the draft, which makes
/// it what strangers see. A booking draft takes the served hours first.
pub async fn form_publish(State(_state): St, Json(body): Json<IdBody>) -> Response {
    let id = body.id.clone();
    let w = match tokio::task::spawn_blocking(move || working(&id)).await {
        Ok(Ok(w)) => w,
        Ok(Err(e)) => return err(StatusCode::NOT_FOUND, e),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    if !w.draft {
        return err(
            StatusCode::BAD_REQUEST,
            "there is no draft to publish; this form is as served",
        );
    }
    if body.base.as_deref() != Some(instrument::digest(&w.text).as_str()) {
        return err(StatusCode::CONFLICT, instrument::Stale);
    }
    let text = match &w.served {
        Some(served) => match form_draft::carry_hours(&w.text, served) {
            Ok(t) => t,
            Err(e) => return err(StatusCode::UNPROCESSABLE_ENTITY, e),
        },
        None => w.text.clone(),
    };
    let dir = match form_draft::drafts_dir() {
        Ok(d) => d,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let path = match form_draft::draft_path(&dir, &w.id) {
        Ok(p) => p,
        Err(e) => return err(StatusCode::BAD_REQUEST, e),
    };
    if text != w.text {
        if let Err(e) = instrument::replace_file(&path, &text) {
            return err(StatusCode::INTERNAL_SERVER_ERROR, e);
        }
    }
    match run(
        "factory-publish",
        &["type".as_ref(), "push".as_ref(), path.as_os_str()],
        None,
        Child::Publish,
    )
    .await
    {
        Ok(out) => {
            // Live, and recorded as served by the push itself; the draft has
            // done its job.
            let _ = std::fs::remove_file(&path);
            tracing::info!(form = %w.id, "form published from the factory page");
            Json(serde_json::json!({ "published": true, "output": out.trim() })).into_response()
        }
        Err(e) => err(StatusCode::BAD_GATEWAY, format!("not published: {e:#}")),
    }
}

/// POST /api/factory/form/discard — drop the draft; what is served stays.
pub async fn form_discard(State(_state): St, Json(body): Json<IdBody>) -> Response {
    let done = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let w = working(&body.id)?;
        if !w.draft {
            return Ok(());
        }
        if body.base.as_deref() != Some(instrument::digest(&w.text).as_str()) {
            anyhow::bail!(instrument::Stale);
        }
        std::fs::remove_file(form_draft::draft_path(&form_draft::drafts_dir()?, &w.id)?)?;
        Ok(())
    })
    .await;
    match done {
        Ok(Ok(())) => Json(serde_json::json!({ "discarded": true })).into_response(),
        Ok(Err(e)) if e.downcast_ref::<instrument::Stale>().is_some() => {
            err(StatusCode::CONFLICT, e)
        }
        Ok(Err(e)) => err(StatusCode::NOT_FOUND, e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}
