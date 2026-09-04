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
    pub fn participant_emails(&self) -> Vec<String> {
        self.value["participants"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|p| p["email"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
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
        Err(_) => record.value.clone(),
    };
    if !current["lifecycle"].is_object() {
        current["lifecycle"] = json!({});
    }
    for key in &record.dirty {
        current["lifecycle"][*key] = record.lifecycle()[*key].clone();
    }
    let tmp = record.path.with_extension("json.tmp");
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
        Some("book") => "booking".into(),
        Some("pick") if booked => "booked (your pick)".into(),
        Some("pick") if !life["pick_item"].is_null() => "needs a pick — in the outbox".into(),
        Some("pick") => "needs a pick".into(),
        Some("no_time") => "no time found".into(),
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

fn open_store(cfg: &mecha_core::config::Config) -> Result<OutboxStore> {
    let root = match cfg.outbox.dir.clone() {
        Some(dir) => dir,
        None => OutboxStore::default_root()?,
    };
    OutboxStore::open(root)
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
    let mut args = json!({
        "title": record.title,
        "start_time": row["start"],
        "end_time": row["end"],
        "attendees": record.participant_emails(),
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
    }
    Ok(args)
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
    let cwd = std::env::current_dir().context("cannot determine the working directory")?;
    let cfg = mecha_core::config::Config::load(&cwd)?;
    let store = open_store(&cfg)?;
    let record = load_by_id(poll_id)?;
    let Some(item_id) = record.lifecycle()["pick_item"].as_str() else {
        bail!(
            "`{poll_id}` has no pick card ({}) — `mecha polls sweep` stages one when the \
             verdict is a pick",
            summary(record.lifecycle())
        );
    };
    anyhow::ensure!(n >= 1, "candidates are numbered from 1");
    // Taken before the read it protects: an `outbox edit` landing between
    // the read and the write would otherwise be rebuilt over.
    let _lock = store.lock()?;
    let item = store.item(item_id)?;
    anyhow::ensure!(
        item.status == "pending",
        "the pick card {item_id} is {}, not pending",
        item.status
    );
    let args = repick(&record, &item.args, n - 1)?;
    store.update_args(item_id, args)?;
    let tz = record.lifecycle()["timezone"].as_str().unwrap_or("UTC");
    Ok(local_range(&record.ranked()[n - 1], tz))
}

/// Advance to the next candidate, wrapping — the `/polls` modal's `p` key.
pub fn pick_next(poll_id: &str) -> Result<String> {
    let cwd = std::env::current_dir().context("cannot determine the working directory")?;
    let cfg = mecha_core::config::Config::load(&cwd)?;
    let store = open_store(&cfg)?;
    let record = load_by_id(poll_id)?;
    let Some(item_id) = record.lifecycle()["pick_item"].as_str() else {
        bail!("no pick card — {}", summary(record.lifecycle()));
    };
    let item = store.item(item_id)?;
    let count = record.ranked().len();
    anyhow::ensure!(count > 0, "the ranking is empty");
    let next = loaded_index(&record, &item)
        .map(|i| (i + 1) % count)
        .unwrap_or(0);
    pick(poll_id, next + 1)
}

/// One tick: stage what needs a card, reconcile what has been decided.
pub fn sweep() -> Result<Vec<String>> {
    let cwd = std::env::current_dir().context("cannot determine the working directory")?;
    let cfg = mecha_core::config::Config::load(&cwd)?;
    let store = open_store(&cfg)?;
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
                    record.set(
                        "booked",
                        json!({
                            "event_id": "",
                            "account": item.args["account"].as_str().unwrap_or(""),
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

        let err = pick_args(&r, 2).unwrap_err().to_string();
        assert!(err.contains("2 candidate(s), not 3"), "{err}");
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
    }

    #[test]
    fn the_loaded_candidate_is_found_by_its_start() {
        let r = record(json!({"ranked": ranked()}));
        let mut item = OutboxItem {
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
        assert_eq!(summary(&json!({"verdict": "book", "booked": {}})), "booked");
        assert_eq!(summary(&json!({"verdict": "no_time"})), "no time found");
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
        assert_eq!(item.author, Author::Harness, "nobody's draft: never mined");
        assert_eq!(item.args["start_time"], "2030-02-05T18:00:00Z");
        assert!(
            step(&mut r, &store, &cfg).unwrap().is_none(),
            "pending: nothing to do"
        );

        // Released: the record learns the slot; the page's sentence is the
        // factory sweep's to write.
        store.resolve(&item_id, "sent", None).unwrap();
        let line = step(&mut r, &store, &cfg).unwrap().expect("reconciled");
        assert!(line.contains("booked 2030-02-05T18:00:00Z"), "{line}");
        let life = r.lifecycle();
        assert_eq!(life["book"]["duration_minutes"], 60);
        assert_eq!(life["booked"]["via"], format!("outbox:{item_id}"));
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

        // Unrouted: refused, not staged somewhere nothing releases.
        let mut r = record(json!({"verdict": "pick", "ranked": ranked()}));
        let unrouted = mecha_core::config::Config::default();
        let err = step(&mut r, &store, &unrouted).unwrap_err().to_string();
        assert!(err.contains("not outbox-routed"), "{err}");
        assert!(r.lifecycle()["pick_item"].is_null());
    }
}
