//! `mecha polls` — the owner's half of a meeting poll's lifecycle.
//!
//! Three verbs on one timer carry a poll from the release to the booking
//! (MEETING-POLL-UX-DESIGN.md §3.4): `factory-publish polls sweep` observes
//! the box and decides, `mecha-mail polls` mails and books, and this one
//! does the two things only `mecha` can, because only `mecha` holds the
//! outbox:
//!
//! - **Stage the pick.** A verdict of `pick` — a tie, an if-needed in the
//!   best slot, someone silent at the deadline — becomes a real
//!   `calendar_create_event` draft for the top-ranked time, with the ranking
//!   and each candidate's reason in its description. Releasing it *is* the
//!   booking, through the same route any calendar draft takes; nothing here
//!   creates an event.
//! - **Reconcile.** When that draft is `sent`, the record learns the slot
//!   and the poll page gets its sentence on the sweep's next tick; when it
//!   is `rejected`, the verdict becomes `no_time` with the reason. Ruling 5:
//!   no mail goes to the participants on a rejection — there is nothing
//!   templated to say, and the owner is right there to say it.
//!
//! `pick <poll> <n>` swaps the draft's slot for the n-th ranked candidate —
//! an `update_args` on `start_time`/`end_time`, and on the description only
//! while it is still the generated one (its `▸` marks the loaded slot), so
//! a title or attendee the owner edited in survives the swap and the draft
//! stays the thing the reviewer read. The `/polls` modal's `p` key is this
//! function. The card is staged by the harness (`OutboxStore::stage_by_harness`),
//! which is what keeps its release out of the writing miner.
//!
//! The record is `factory-publish`'s file, edited through the JSON it came
//! from: this side writes `pick_item`, `book`, `booked`, `verdict` and
//! `resolution`, and never touches a field it does not own.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::GlobalOpts;
use mecha_core::outbox::{OutboxItem, OutboxStore};

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// One line per meeting poll on record: where its lifecycle stands.
    List,
    /// Stage a pick card for every poll that needs one, and fold released or
    /// rejected cards back into their records. The timer's verb; idempotent.
    Sweep,
    /// Load the n-th ranked candidate (1-based) into a poll's pick card.
    Pick { poll_id: String, n: usize },
}

pub async fn run(_global: &GlobalOpts, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::List => {
            let dir = records_dir()?;
            let (records, problems) = scan(&dir)?;
            for problem in &problems {
                eprintln!("unreadable: {problem}");
            }
            if records.is_empty() {
                println!("no meeting polls on record");
            }
            for record in &records {
                println!(
                    "{:<24} {:<28} {}",
                    record.poll_id,
                    clip(&record.title, 28),
                    summary(&record.value["lifecycle"])
                );
            }
            Ok(())
        }
        Cmd::Sweep => {
            let report = sweep()?;
            for line in &report {
                println!("{line}");
            }
            if report.is_empty() {
                println!("nothing to do");
            }
            Ok(())
        }
        Cmd::Pick { poll_id, n } => {
            let loaded = pick(&poll_id, n)?;
            println!("{poll_id}: pick card now holds {loaded}");
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// The record.

pub fn records_dir() -> Result<PathBuf> {
    Ok(mecha_core::work::mecha_home()?
        .join("factory")
        .join("polls"))
}

/// A poll record with a lifecycle, as this side reads it.
pub struct PollRecord {
    pub path: PathBuf,
    pub poll_id: String,
    pub title: String,
    pub value: Value,
    /// The lifecycle keys this process changed — the only ones `save`
    /// writes, into the file as it is then. The other two verbs run on the
    /// same timer and own the rest.
    pub dirty: BTreeSet<&'static str>,
}

impl PollRecord {
    pub fn lifecycle(&self) -> &Value {
        &self.value["lifecycle"]
    }
    fn set(&mut self, key: &'static str, value: Value) {
        self.value["lifecycle"][key] = value;
        self.dirty.insert(key);
    }
    /// The ranked candidates the sweep wrote, when the verdict is a pick.
    pub fn ranked(&self) -> Vec<Value> {
        self.lifecycle()["ranked"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
    /// Every participant's address — and an error, never a shorter list,
    /// when one has none: a person missing from the booking is the finding
    /// this whole surface exists to avoid.
    pub fn participant_emails(&self) -> Result<Vec<String>> {
        self.value["participants"]
            .as_array()
            .context("the record has no participant list")?
            .iter()
            .map(|p| {
                p["email"].as_str().map(str::to_string).with_context(|| {
                    format!(
                        "participant `{}` has no address",
                        p["name"].as_str().unwrap_or("?")
                    )
                })
            })
            .collect()
    }
}

pub fn scan(dir: &Path) -> Result<(Vec<PollRecord>, Vec<String>)> {
    let mut records = Vec::new();
    let mut problems = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        // A machine that has never created a poll has no directory: empty,
        // not broken.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((records, problems)),
        Err(e) => return Err(e).context(format!("reading {}", dir.display())),
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    for path in paths {
        match load(&path) {
            Ok(Some(record)) => records.push(record),
            Ok(None) => {}
            Err(e) => problems.push(format!("{}: {e:#}", path.display())),
        }
    }
    Ok((records, problems))
}

pub fn load(path: &Path) -> Result<Option<PollRecord>> {
    let text = std::fs::read_to_string(path)?;
    let value: Value = serde_json::from_str(&text).context("not JSON")?;
    if !value["lifecycle"].is_object() {
        return Ok(None);
    }
    Ok(Some(PollRecord {
        path: path.to_path_buf(),
        poll_id: value["poll_id"].as_str().unwrap_or_default().to_string(),
        title: value["title"].as_str().unwrap_or("Meeting").to_string(),
        value,
        dirty: BTreeSet::new(),
    }))
}

pub fn load_by_id(poll_id: &str) -> Result<PollRecord> {
    anyhow::ensure!(
        !poll_id.is_empty()
            && poll_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "`{poll_id}` is not a poll id"
    );
    let path = records_dir()?.join(format!("{poll_id}.json"));
    load(&path)?.ok_or_else(|| anyhow::anyhow!("no meeting poll `{poll_id}` on record"))
}

/// Write the keys this process changed into the record as it is now —
/// re-read first, so the other verbs' writes since the load survive.
pub fn save(record: &PollRecord) -> Result<()> {
    if record.dirty.is_empty() {
        return Ok(());
    }
    let mut current = match std::fs::read_to_string(&record.path) {
        Ok(text) => serde_json::from_str::<Value>(&text).with_context(|| {
            format!(
                "{} changed under the sweep and is not JSON",
                record.path.display()
            )
        })?,
        // Gone since the load: write what we have rather than lose the tick.
        // Any other failure to read is not a licence to write a snapshot
        // over the other verbs' keys — the one thing the dirty set prevents.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => record.value.clone(),
        Err(e) => return Err(e).with_context(|| format!("re-reading {}", record.path.display())),
    };
    if !current["lifecycle"].is_object() {
        current["lifecycle"] = json!({});
    }
    for key in &record.dirty {
        current["lifecycle"][*key] = record.lifecycle()[*key].clone();
    }
    // A sibling unique to this process: three binaries write this file,
    // and a shared temp name would let two of them interleave into a torn
    // record that both then report as unreadable. `scan` walks `.json`
    // only, so the sibling is invisible to it either way.
    let tmp = record
        .path
        .with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string_pretty(&current)?)?;
    std::fs::rename(&tmp, &record.path)
        .with_context(|| format!("renaming into {}", record.path.display()))?;
    Ok(())
}

/// The one-line state the monitor and `list` show. Mirrors
/// `factory-publish`'s `Lifecycle::summary`, from the JSON.
pub fn summary(life: &Value) -> String {
    let invites = life["invites"].as_object();
    let sent = invites
        .map(|m| m.values().filter(|v| !v.is_null()).count())
        .unwrap_or(0);
    let total = invites.map(|m| m.len()).unwrap_or(0);
    let booked = !life["booked"].is_null();
    match life["verdict"].as_str() {
        Some("book") if booked => "booked".into(),
        // A collision the mail half wrote down, waiting on the factory
        // sweep to turn it into a pick: parked, and never a quiet "booking".
        Some("book") if !life["conflict"].is_null() => "booking blocked — collision".into(),
        Some("book") => "booking".into(),
        Some("pick") if booked => "booked (your pick)".into(),
        Some("pick") if !life["pick_item"].is_null() => "needs a pick — in the outbox".into(),
        Some("pick") => "needs a pick".into(),
        Some("no_time") => "no time found".into(),
        Some("stalled") if !life["book"].is_null() => "stalled — booking never made".into(),
        Some("stalled") => "stalled — invitations never all sent".into(),
        Some(other) => other.to_string(),
        // No invitations on record is not "all sent": unknown is never done.
        None if total == 0 => "—".into(),
        None if sent < total => format!("invites {sent}/{total}"),
        None => "invites sent".into(),
    }
}

fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        text.to_string()
    } else {
        let cut: String = text.chars().take(width - 1).collect();
        format!("{cut}…")
    }
}

// ---------------------------------------------------------------------------
// The pick card.

/// The routed name of `calendar_create_event`, from `[outbox] tools` and
/// never a guess — the same rule `mecha mail compose` follows. Unrouted, the
/// pick is refused: a draft nothing releases is not a review.
fn create_event_tool(cfg: &mecha_core::config::Config) -> Result<String> {
    cfg.outbox
        .tools
        .iter()
        .find(|t| t.rsplit("__").next() == Some("calendar_create_event"))
        .cloned()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "calendar_create_event is not outbox-routed in [outbox] tools — a pick card \
                 is a calendar draft, and there is no route to release it"
            )
        })
}

/// The config for the routed name, and the store — the latter through
/// `commands::outbox::open_store`, the one home for "which store", so this
/// side can never stage into a directory the review does not read.
fn cfg_and_store() -> Result<(mecha_core::config::Config, OutboxStore)> {
    let cwd = std::env::current_dir().context("cannot determine the working directory")?;
    let cfg = mecha_core::config::Config::load(&cwd)?;
    let store = crate::commands::outbox::open_store()?;
    Ok((cfg, store))
}

/// "Tue 5 Feb, 1:00 PM–2:00 PM EST" for one ranked row, in the poll's zone.
pub fn local_range(row: &Value, timezone: &str) -> String {
    let parse = |k: &str| {
        row[k]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&chrono::Utc))
    };
    let (Some(start), Some(end)) = (parse("start"), parse("end")) else {
        return "?".into();
    };
    match timezone.parse::<chrono_tz::Tz>() {
        Ok(tz) => {
            let (s, e) = (start.with_timezone(&tz), end.with_timezone(&tz));
            format!(
                "{}, {}–{}",
                s.format("%a %-d %b"),
                s.format("%-I:%M %p"),
                e.format("%-I:%M %p %Z")
            )
        }
        Err(_) => format!(
            "{}–{} UTC",
            start.format("%a %-d %b %H:%M"),
            end.format("%H:%M")
        ),
    }
}

/// The draft's arguments for one candidate: the event as the attendees will
/// see it, with the whole ranking in the description so the reviewer picks
/// with the reasons in front of them. Pure, so the card is a unit test.
pub fn pick_args(record: &PollRecord, index: usize) -> Result<Value> {
    let life = record.lifecycle();
    let ranked = record.ranked();
    let row = ranked.get(index).ok_or_else(|| {
        anyhow::anyhow!(
            "the ranking has {} candidate(s), not {}",
            ranked.len(),
            index + 1
        )
    })?;
    let tz = life["timezone"].as_str().unwrap_or("UTC");
    let mut description = String::new();
    if let Some(message) = life["message"].as_str().filter(|m| !m.trim().is_empty()) {
        description.push_str(message.trim());
        description.push_str("\n\n");
    }
    description.push_str(&format!(
        "Scheduled from the poll \"{}\".\n\nThe ranking, as answered:\n",
        record.title
    ));
    for (i, candidate) in ranked.iter().enumerate() {
        description.push_str(&format!(
            "{}{}. {} — {}\n",
            if i == index { "▸ " } else { "  " },
            i + 1,
            local_range(candidate, tz),
            candidate["reason"].as_str().unwrap_or("")
        ));
    }
    if let Some(silent) = life["silent"].as_array().filter(|s| !s.is_empty()) {
        description.push_str(&format!(
            "\nNever answered: {}.\n",
            silent
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(conflict) = life["conflict"].as_str() {
        description.push_str(&format!("\nNote: {conflict}.\n"));
    }
    // The card's own name for its poll — what lets a tick that staged the
    // card but never wrote `pick_item` find it again instead of staging a
    // second one.
    description.push_str(&format!("\n{}\n", poll_marker(&record.poll_id)));
    let mut args = json!({
        "title": record.title,
        "start_time": row["start"],
        "end_time": row["end"],
        "attendees": record.participant_emails()?,
        "description": description,
    });
    if let Some(account) = life["account"].as_str() {
        args["account"] = json!(account);
    }
    Ok(args)
}

/// The draft's arguments with the n-th candidate loaded, keeping every edit
/// the owner made: only the two time fields move, and the description only
/// when it is still the one this side generated for the slot it held.
pub fn repick(record: &PollRecord, current: &Value, index: usize) -> Result<Value> {
    let fresh = pick_args(record, index)?;
    let mut args = current.clone();
    args["start_time"] = fresh["start_time"].clone();
    args["end_time"] = fresh["end_time"].clone();
    let untouched = current["start_time"]
        .as_str()
        .and_then(|s| {
            record
                .ranked()
                .iter()
                .position(|c| c["start"].as_str() == Some(s))
        })
        .and_then(|loaded| pick_args(record, loaded).ok())
        .is_some_and(|generated| generated["description"] == current["description"]);
    if untouched {
        args["description"] = fresh["description"].clone();
    } else if let Some(edited) = current["description"].as_str() {
        // The owner edited the prose: keep every word of it, but the `▸`
        // is generated state and must follow the slot — a card that marks
        // Tuesday and books Thursday is the event body every attendee gets.
        args["description"] = json!(move_marker(edited, index));
    }
    Ok(args)
}

/// Move the `▸` in a card's ranking to the `index`-th row, leaving every
/// other character as the owner left it. Rows are the generated
/// `  N. …` / `▸ N. …` lines; a description with no such rows is returned
/// unchanged.
pub fn move_marker(description: &str, index: usize) -> String {
    let wanted = format!("{}. ", index + 1);
    description
        .lines()
        .map(|line| {
            let (marker, rest) = if let Some(rest) = line.strip_prefix("▸ ") {
                ("▸ ", rest)
            } else if let Some(rest) = line.strip_prefix("  ") {
                ("  ", rest)
            } else {
                return line.to_string();
            };
            let is_row = rest
                .split_once(". ")
                .is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
            if !is_row {
                return format!("{marker}{rest}");
            }
            if rest.starts_with(&wanted) {
                format!("▸ {rest}")
            } else {
                format!("  {rest}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The event id in `calendar_create_event`'s answer — "created in `a`:"
/// followed by the event as JSON. Empty when the answer has no readable id,
/// which the record shows as such rather than inventing one.
pub fn event_id_of(output: &str) -> String {
    let parsed = output
        .find('{')
        .and_then(|i| serde_json::from_str::<Value>(&output[i..]).ok())
        .and_then(|v| v["event_id"].as_str().map(str::to_string));
    // The recorded output is bounded, so the JSON may be cut short of its
    // closing brace; the id is near the top, so read it from the text.
    parsed
        .or_else(|| {
            let (_, rest) = output.split_once("\"event_id\": \"")?;
            let (id, _) = rest.split_once('"')?;
            (!id.is_empty()).then(|| id.to_string())
        })
        .unwrap_or_default()
}

/// The account the release landed on — "created in `work`:" — so both
/// halves write the same `booked` shape; empty when the answer has none.
pub fn account_of(output: &str) -> String {
    output
        .strip_prefix("created in `")
        .and_then(|rest| rest.split_once('`'))
        .map(|(account, _)| account.to_string())
        .unwrap_or_default()
}

/// The line on a pick card that names its poll.
pub fn poll_marker(poll_id: &str) -> String {
    format!("poll: {poll_id}")
}

/// A pending pick card for this poll already in the store — staged by a
/// tick that then failed to write `pick_item` — or nothing. Without this
/// the gap between `stage` and `save` orphaned a releasable calendar draft
/// and the next tick staged a second, which is two events for one poll.
fn adoptable_card(store: &OutboxStore, tool: &str, poll_id: &str) -> Result<Option<OutboxItem>> {
    let marker = poll_marker(poll_id);
    // The strict walk: an item this binary cannot read is an error here,
    // not a shorter list — a shorter list is how the orphan goes unseen and
    // a second card gets staged, the exact outcome this guards against.
    Ok(store.items_strict()?.into_iter().find(|item| {
        item.status == "pending"
            && item.author() == mecha_core::outbox::Author::Harness
            && item.tool == tool
            && item.args["description"]
                .as_str()
                .is_some_and(|d| d.lines().any(|l| l == marker))
    }))
}

/// Which ranked candidate a draft currently holds, by its start.
pub fn loaded_index(record: &PollRecord, item: &OutboxItem) -> Option<usize> {
    let start = item.args["start_time"].as_str()?;
    record
        .ranked()
        .iter()
        .position(|c| c["start"].as_str() == Some(start))
}

/// Load the n-th ranked candidate (1-based) into the poll's pick card.
/// Returns the slot as the reviewer will read it.
pub fn pick(poll_id: &str, n: usize) -> Result<String> {
    let store = crate::commands::outbox::open_store()?;
    let record = load_by_id(poll_id)?;
    anyhow::ensure!(n >= 1, "candidates are numbered from 1");
    // Taken before the read it protects: an `outbox edit` landing between
    // the read and the write would otherwise be rebuilt over. Never waited
    // on — `outbox send` holds this across a human prompt, and the TUI's
    // `p` runs on its event loop.
    let Some(_lock) = store.try_lock()? else {
        bail!("the outbox is locked by a review in progress — try again in a moment");
    };
    let item = pick_card(&store, &record)?;
    let args = repick(&record, &item.args, n - 1)?;
    store.update_args(&item.id, args)?;
    let tz = record.lifecycle()["timezone"].as_str().unwrap_or("UTC");
    Ok(local_range(&record.ranked()[n - 1], tz))
}

/// Advance to the next candidate, wrapping — the `/polls` modal's `p` key.
pub fn pick_next(poll_id: &str) -> Result<String> {
    let store = crate::commands::outbox::open_store()?;
    let record = load_by_id(poll_id)?;
    let count = record.ranked().len();
    anyhow::ensure!(count > 0, "the ranking is empty");
    // The next index is computed under the same lock the write takes, from
    // the same read — one exact-id read of the store, on the TUI's event
    // loop, and no window for an `outbox edit` to land between them.
    let Some(_lock) = store.try_lock()? else {
        bail!("the outbox is locked by a review in progress — try again in a moment");
    };
    let item = pick_card(&store, &record)?;
    let next = loaded_index(&record, &item)
        .map(|i| (i + 1) % count)
        .unwrap_or(0);
    let args = repick(&record, &item.args, next)?;
    store.update_args(&item.id, args)?;
    let tz = record.lifecycle()["timezone"].as_str().unwrap_or("UTC");
    Ok(local_range(&record.ranked()[next], tz))
}

/// The poll's pending pick card — read once, by its exact id (never the
/// whole-store scan `item()` is), and checked to be a card this side staged
/// before anything rewrites it.
fn pick_card(store: &OutboxStore, record: &PollRecord) -> Result<OutboxItem> {
    let Some(item_id) = record.lifecycle()["pick_item"].as_str() else {
        bail!(
            "`{}` has no pick card ({}) — `mecha polls sweep` stages one when the verdict \
             is a pick",
            record.poll_id,
            summary(record.lifecycle())
        );
    };
    let item = store
        .item_exact(item_id)?
        .ok_or_else(|| anyhow::anyhow!("the pick card {item_id} is not in the outbox"))?;
    anyhow::ensure!(
        item.status == "pending",
        "the pick card {item_id} is {}, not pending",
        item.status
    );
    // Belt and braces: `pick_item` is a field on a file three binaries
    // write. Only a card this side staged is this side's to rewrite.
    anyhow::ensure!(
        item.author() == mecha_core::outbox::Author::Harness
            && item.tool.rsplit("__").next() == Some("calendar_create_event"),
        "outbox item {item_id} is not a pick card (`{}`, {})",
        item.tool,
        item.author
    );
    Ok(item)
}

/// One tick: stage what needs a card, reconcile what has been decided.
pub fn sweep() -> Result<Vec<String>> {
    let (cfg, store) = cfg_and_store()?;
    // Held across the tick: adopt-then-stage is a read-modify-write over
    // the store, and two overlapping sweeps — the timer and a hand run —
    // would each see no card and each stage one, the second of which no
    // later tick reconciles. `stage_by_harness` and `update_args` do not
    // take the lock themselves. Never waited on: this is the last verb on
    // the slots timer's line, and `outbox send` holds the lock across a
    // human prompt — a tick that finds it held says so and yields.
    let Some(_lock) = store.try_lock()? else {
        return Ok(vec![
            "the outbox is locked by a review in progress; nothing done this tick".to_string(),
        ]);
    };
    let dir = records_dir()?;
    let (records, problems) = scan(&dir)?;
    let mut lines: Vec<String> = problems
        .iter()
        .map(|p| format!("unreadable: {p}"))
        .collect();
    for mut record in records {
        // One record's failure is that record's line, never the reason a
        // later poll's decision goes unreconciled.
        let outcome = step(&mut record, &store, &cfg).and_then(|line| match line {
            Some(line) => save(&record).map(|()| Some(line)),
            None => Ok(None),
        });
        match outcome {
            Ok(Some(line)) => lines.push(format!("{}: {line}", record.poll_id)),
            Ok(None) => {}
            Err(e) => lines.push(format!("{}: failed — {e:#}", record.poll_id)),
        }
    }
    Ok(lines)
}

/// What one record needs from this side, decided against the outbox.
/// `Some` when the record changed.
fn step(
    record: &mut PollRecord,
    store: &OutboxStore,
    cfg: &mecha_core::config::Config,
) -> Result<Option<String>> {
    let life = record.lifecycle().clone();
    if life["verdict"].as_str() != Some("pick")
        || !life["booked"].is_null()
        || !life["resolution"].is_null()
    {
        return Ok(None);
    }
    match life["pick_item"].as_str() {
        None => {
            if record.ranked().is_empty() {
                // A pick with nothing to pick from: the sweep saw no feasible
                // slot for anyone. The owner closes it from /polls with a
                // sentence; nothing to stage.
                return Ok(None);
            }
            let tool = create_event_tool(cfg)?;
            if let Some(orphan) = adoptable_card(store, &tool, &record.poll_id)? {
                record.set("pick_item", json!(orphan.id));
                return Ok(Some(format!(
                    "adopted pick card {} staged by an earlier tick that could not write the record",
                    orphan.id
                )));
            }
            let args = pick_args(record, 0)?;
            // The harness's own bookkeeping: the owner's records and the
            // box's enum answers, no model, and a release that says nothing
            // about drafting.
            let item = store.stage_by_harness(&tool, args)?;
            record.set("pick_item", json!(item.id));
            Ok(Some(format!(
                "pick card staged as {} — release books the top candidate; `mecha polls pick {} <n>` or `p` in /polls swaps it",
                item.id, record.poll_id
            )))
        }
        Some(item_id) => {
            let item = match store.item_exact(item_id)? {
                Some(item) => item,
                None => {
                    // The card is gone (swept, deleted by hand): stage again
                    // next tick rather than wait on a ghost.
                    record.set("pick_item", Value::Null);
                    return Ok(Some(format!(
                        "pick card {item_id} is gone; will stage a new one"
                    )));
                }
            };
            // As strict as `pick_card`: `pick_item` is a field on a file three
            // binaries write, and acting on a mismatched id — a `sent` mail
            // draft, say — would close the poll as booked with no event and
            // no slot, terminally.
            if item.author() != mecha_core::outbox::Author::Harness
                || item.tool.rsplit("__").next() != Some("calendar_create_event")
            {
                return Ok(Some(format!(
                    "pick_item {} is not a pick card (`{}`, {}); left alone",
                    item.id, item.tool, item.author
                )));
            }
            match item.status.as_str() {
                "sent" => {
                    let (start, end) = (
                        item.args["start_time"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        item.args["end_time"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                    );
                    let minutes = match (
                        chrono::DateTime::parse_from_rfc3339(&start),
                        chrono::DateTime::parse_from_rfc3339(&end),
                    ) {
                        (Ok(s), Ok(e)) => (e - s).num_minutes().max(0) as u64,
                        _ => 0,
                    };
                    record.set(
                        "book",
                        json!({"start": start, "end": end, "duration_minutes": minutes}),
                    );
                    let output = item.output.as_deref().unwrap_or("");
                    record.set(
                        "booked",
                        json!({
                            "event_id": event_id_of(output),
                            "account": match item.args["account"].as_str() {
                                Some(pinned) => pinned.to_string(),
                                None => account_of(output),
                            },
                            "at": item.resolved_at.clone().unwrap_or_default(),
                            "via": format!("outbox:{}", item.id),
                        }),
                    );
                    Ok(Some(format!(
                        "pick released — booked {start}; the sweep closes the poll page next tick"
                    )))
                }
                "rejected" => {
                    record.set("verdict", json!("no_time"));
                    record.set(
                        "resolution",
                        json!(item
                            .reason
                            .as_deref()
                            .filter(|r| !r.trim().is_empty())
                            .unwrap_or("No time found")),
                    );
                    Ok(Some("pick rejected — closing as no time found".to_string()))
                }
                _ => Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::outbox::{Author, OutboxKind};

    /// A fresh directory under the system temp dir, removed on drop. No
    /// `tempfile` in this crate's dev-dependencies, and one helper is
    /// cheaper than a dependency.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir =
                std::env::temp_dir().join(format!("mecha-polls-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn record(life: Value) -> PollRecord {
        PollRecord {
            path: PathBuf::from("/tmp/lab.json"),
            poll_id: "lab-20300128".into(),
            title: "Lab meeting".into(),
            dirty: BTreeSet::new(),
            value: json!({
                "poll_id": "lab-20300128",
                "title": "Lab meeting",
                "participants": [
                    {"name": "Priya", "email": "priya@example.edu", "url": "u1"},
                    {"name": "Tal", "email": "tal@example.edu", "url": "u2"}
                ],
                "lifecycle": life,
            }),
        }
    }

    fn ranked() -> Value {
        json!([
            {"start": "2030-02-05T18:00:00Z", "end": "2030-02-05T19:00:00Z", "duration_minutes": 60, "reason": "Tal if needed"},
            {"start": "2030-02-07T18:00:00Z", "end": "2030-02-07T19:00:00Z", "duration_minutes": 60, "reason": "Priya can't"}
        ])
    }

    /// The card is the event as attendees will see it, the loaded candidate
    /// marked, every reason readable, and the account pinned from the record.
    #[test]
    fn the_pick_card_is_a_calendar_draft_with_the_ranking_in_it() {
        let r = record(json!({
            "verdict": "pick",
            "ranked": ranked(),
            "timezone": "America/New_York",
            "message": "Before the grant deadline.",
            "silent": ["Tal"],
            "account": "work",
        }));
        let args = pick_args(&r, 1).unwrap();
        assert_eq!(args["title"], "Lab meeting");
        assert_eq!(args["start_time"], "2030-02-07T18:00:00Z");
        assert_eq!(args["end_time"], "2030-02-07T19:00:00Z");
        assert_eq!(
            args["attendees"],
            json!(["priya@example.edu", "tal@example.edu"])
        );
        assert_eq!(args["account"], "work");
        let description = args["description"].as_str().unwrap();
        assert!(description.starts_with("Before the grant deadline."));
        assert!(
            description.contains("  1. Tue 5 Feb, 1:00 PM–2:00 PM EST — Tal if needed"),
            "{description}"
        );
        assert!(
            description.contains("▸ 2. Thu 7 Feb, 1:00 PM–2:00 PM EST — Priya can't"),
            "{description}"
        );
        assert!(description.contains("Never answered: Tal."));
        assert!(
            description.lines().any(|l| l == "poll: lab-20300128"),
            "{description}"
        );

        let err = pick_args(&r, 2).unwrap_err().to_string();
        assert!(err.contains("2 candidate(s), not 3"), "{err}");

        // A participant with no address is a finding, not a shorter invite.
        let mut short = r.value.clone();
        short["participants"][1] = json!({"name": "Tal", "url": "u2"});
        let short = PollRecord {
            path: r.path.clone(),
            poll_id: r.poll_id.clone(),
            title: r.title.clone(),
            value: short,
            dirty: BTreeSet::new(),
        };
        let err = pick_args(&short, 0).unwrap_err().to_string();
        assert!(err.contains("`Tal` has no address"), "{err}");
    }

    /// The owner edited the card — a title, an extra attendee — and then
    /// swapped the slot: only the two times move, and the generated
    /// description follows the slot unless the owner rewrote it too.
    #[test]
    fn a_pick_keeps_the_owners_edits_to_the_card() {
        let r = record(json!({"verdict": "pick", "ranked": ranked(), "timezone": "UTC"}));
        let mut edited = pick_args(&r, 0).unwrap();
        edited["title"] = json!("Lab meeting (grant)");
        edited["attendees"]
            .as_array_mut()
            .unwrap()
            .push(json!("assistant@example.edu"));
        let swapped = repick(&r, &edited, 1).unwrap();
        assert_eq!(swapped["title"], "Lab meeting (grant)");
        assert_eq!(swapped["attendees"].as_array().unwrap().len(), 3);
        assert_eq!(swapped["start_time"], "2030-02-07T18:00:00Z");
        assert_eq!(swapped["end_time"], "2030-02-07T19:00:00Z");
        assert!(
            swapped["description"].as_str().unwrap().contains("▸ 2."),
            "the untouched description follows the slot"
        );

        // A rewritten description is the owner's and stays theirs.
        let mut rewritten = pick_args(&r, 0).unwrap();
        rewritten["description"] = json!("Bring the draft.");
        let swapped = repick(&r, &rewritten, 1).unwrap();
        assert_eq!(swapped["description"], "Bring the draft.");
        assert_eq!(swapped["start_time"], "2030-02-07T18:00:00Z");

        // A partial edit — one word fixed — keeps every word, and the marker
        // still follows the slot: the card never names a row it will not book.
        let mut touched = pick_args(&r, 0).unwrap();
        let text = touched["description"]
            .as_str()
            .unwrap()
            .replace("Scheduled from", "Chosen from");
        touched["description"] = json!(text);
        let swapped = repick(&r, &touched, 1).unwrap();
        let description = swapped["description"].as_str().unwrap();
        assert!(description.contains("Chosen from"), "{description}");
        assert!(description.contains("\n  1. "), "{description}");
        assert!(description.contains("\n▸ 2. "), "{description}");
        assert!(!description.contains("▸ 1. "), "{description}");
        assert_eq!(move_marker("no rows here", 1), "no rows here");
    }

    #[test]
    fn the_loaded_candidate_is_found_by_its_start() {
        let r = record(json!({"ranked": ranked()}));
        let mut item = OutboxItem {
            output: None,
            author: Default::default(),
            id: "ob1".into(),
            status: "pending".into(),
            tool: "mail__calendar_create_event".into(),
            kind: OutboxKind::Message,
            args_before: json!({}),
            args: pick_args(&r, 1).unwrap(),
            summary: String::new(),
            session_id: None,
            workspace: None,
            taint: Default::default(),
            created_at: String::new(),
            resolved_at: None,
            reason: None,
            error: None,
            call_id: None,
            filled_defaults: Vec::new(),
        };
        assert_eq!(loaded_index(&r, &item), Some(1));
        item.args["start_time"] = json!("2031-01-01T00:00:00Z");
        assert_eq!(loaded_index(&r, &item), None);
    }

    #[test]
    fn the_summary_reads_the_lifecycle_the_way_the_sweep_wrote_it() {
        assert_eq!(
            summary(&json!({})),
            "—",
            "no invitations on record is not done"
        );
        assert_eq!(summary(&json!({"invites": {}})), "—");
        assert_eq!(
            summary(&json!({"invites": {"a": null, "b": "t"}})),
            "invites 1/2"
        );
        assert_eq!(
            summary(&json!({"invites": {"a": "t", "b": "t"}})),
            "invites sent"
        );
        assert_eq!(summary(&json!({"verdict": "pick"})), "needs a pick");
        assert_eq!(
            summary(&json!({"verdict": "pick", "pick_item": "ob1"})),
            "needs a pick — in the outbox"
        );
        assert_eq!(summary(&json!({"verdict": "book"})), "booking");
        assert_eq!(
            summary(&json!({"verdict": "book", "conflict": "busy"})),
            "booking blocked — collision"
        );
        assert_eq!(summary(&json!({"verdict": "book", "booked": {}})), "booked");
        assert_eq!(summary(&json!({"verdict": "no_time"})), "no time found");
        assert_eq!(
            summary(&json!({"verdict": "stalled"})),
            "stalled — invitations never all sent"
        );
        assert_eq!(
            summary(&json!({"verdict": "stalled", "book": {"start": "x"}})),
            "stalled — booking never made"
        );
        assert_eq!(summary(&json!({"verdict": "closed"})), "closed");
    }

    /// Records without a lifecycle are not this side's; a broken file is a
    /// finding, and a missing directory is an empty machine.
    #[test]
    fn scan_reports_broken_files_and_tolerates_no_directory() {
        let dir = Scratch::new();
        std::fs::write(
            dir.path().join("a.json"),
            json!({"poll_id": "a", "lifecycle": {}}).to_string(),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.json"),
            json!({"poll_id": "b"}).to_string(),
        )
        .unwrap();
        std::fs::write(dir.path().join("c.json"), "{").unwrap();
        let (records, problems) = scan(dir.path()).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(problems.len(), 1);
        let (none, no_problems) = scan(&dir.path().join("absent")).unwrap();
        assert!(none.is_empty() && no_problems.is_empty());
    }

    /// `save` writes the keys this tick changed into the file as it is now:
    /// what the mail half wrote after the load survives.
    #[test]
    fn save_merges_only_the_keys_this_tick_changed() {
        let dir = Scratch::new();
        let path = dir.path().join("lab.json");
        let on_disk = json!({"poll_id": "lab", "title": "Lab", "participants": [], "lifecycle": {"verdict": "pick"}});
        std::fs::write(&path, on_disk.to_string()).unwrap();
        let mut mine = load(&path).unwrap().unwrap();
        save(&mine).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            on_disk.to_string(),
            "nothing dirty"
        );

        let mut theirs = on_disk.clone();
        theirs["lifecycle"]["invites"] = json!({"Priya": "2030-01-28T12:00:00Z"});
        std::fs::write(&path, theirs.to_string()).unwrap();
        mine.set("pick_item", json!("ob1"));
        save(&mine).unwrap();
        let after: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(after["lifecycle"]["pick_item"], "ob1");
        assert_eq!(
            after["lifecycle"]["invites"]["Priya"], "2030-01-28T12:00:00Z",
            "theirs kept"
        );
    }

    /// The tick against a real store: a pick stages one card and remembers
    /// it; a second tick stages nothing; a release books, a rejection
    /// resolves, and each writes only its own fields.
    #[test]
    fn the_sweep_stages_once_and_reconciles_the_decision() {
        let dir = Scratch::new();
        let store = OutboxStore::open(dir.path().join("outbox")).unwrap();
        let mut cfg = mecha_core::config::Config::default();
        cfg.outbox.tools = vec!["mail__calendar_create_event".into()];

        let mut r = record(json!({
            "verdict": "pick",
            "ranked": ranked(),
            "timezone": "UTC",
            "a_field_from_the_future": 1,
        }));
        let first = step(&mut r, &store, &cfg).unwrap().expect("staged");
        assert!(first.contains("pick card staged"), "{first}");
        let item_id = r.lifecycle()["pick_item"].as_str().unwrap().to_string();
        let item = store.item(&item_id).unwrap();
        assert_eq!(item.tool, "mail__calendar_create_event");
        assert_eq!(item.kind, OutboxKind::Message);
        assert_eq!(
            item.author(),
            Author::Harness,
            "nobody's draft: never mined"
        );
        assert_eq!(item.args["start_time"], "2030-02-05T18:00:00Z");
        assert!(
            step(&mut r, &store, &cfg).unwrap().is_none(),
            "pending: nothing to do"
        );

        // Released: the record learns the slot and the event the tool made;
        // the page's sentence is the factory sweep's to write.
        store
            .resolve_with_output(
                &item_id,
                "sent",
                None,
                Some("created in `work`:\n{\n  \"event_id\": \"ev42\",\n  \"title\": \"Lab meeting\"\n}".into()),
            )
            .unwrap();
        let line = step(&mut r, &store, &cfg).unwrap().expect("reconciled");
        assert!(line.contains("booked 2030-02-05T18:00:00Z"), "{line}");
        let life = r.lifecycle();
        assert_eq!(life["book"]["duration_minutes"], 60);
        assert_eq!(life["booked"]["event_id"], "ev42");
        assert_eq!(life["booked"]["via"], format!("outbox:{item_id}"));
        assert_eq!(event_id_of("nothing here"), "", "no id is no id");
        assert_eq!(
            event_id_of(
                "created in `w`:\n{\n  \"event_id\": \"ev9\",\n  \"description\": \"cut sho"
            ),
            "ev9",
            "a truncated answer still yields its id"
        );
        assert_eq!(
            life["booked"]["account"], "work",
            "the account the release landed on"
        );
        assert_eq!(account_of("created in `work`:\n{}"), "work");
        assert_eq!(account_of("something else"), "");
        assert_eq!(life["a_field_from_the_future"], 1);
        assert!(
            step(&mut r, &store, &cfg).unwrap().is_none(),
            "booked: done"
        );

        // Rejected: no time found, with the owner's reason.
        let mut r = record(json!({"verdict": "pick", "ranked": ranked(), "timezone": "UTC"}));
        step(&mut r, &store, &cfg).unwrap();
        let item_id = r.lifecycle()["pick_item"].as_str().unwrap().to_string();
        store
            .resolve(&item_id, "rejected", Some("Let's do it async".into()))
            .unwrap();
        step(&mut r, &store, &cfg).unwrap().expect("resolved");
        assert_eq!(r.lifecycle()["verdict"], "no_time");
        assert_eq!(r.lifecycle()["resolution"], "Let's do it async");

        // A tick that staged and lost the write: the next one adopts the
        // card rather than staging a second.
        let mut r = record(json!({"verdict": "pick", "ranked": ranked(), "timezone": "UTC"}));
        step(&mut r, &store, &cfg).unwrap();
        let staged = r.lifecycle()["pick_item"].as_str().unwrap().to_string();
        let mut lost = record(json!({"verdict": "pick", "ranked": ranked(), "timezone": "UTC"}));
        let line = step(&mut lost, &store, &cfg).unwrap().expect("adopted");
        assert!(line.contains("adopted"), "{line}");
        assert_eq!(lost.lifecycle()["pick_item"], staged);
        let pending = store
            .items()
            .unwrap()
            .into_iter()
            .filter(|i| {
                i.status == "pending"
                    && i.args["description"]
                        .as_str()
                        .unwrap_or("")
                        .contains("poll: lab-20300128")
            })
            .count();
        assert_eq!(pending, 1, "one card for one poll");

        // Unrouted: refused, not staged somewhere nothing releases.
        let mut r = record(json!({"verdict": "pick", "ranked": ranked()}));
        let unrouted = mecha_core::config::Config::default();
        let err = step(&mut r, &store, &unrouted).unwrap_err().to_string();
        assert!(err.contains("not outbox-routed"), "{err}");
        assert!(r.lifecycle()["pick_item"].is_null());
    }
}
