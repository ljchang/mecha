//! Meeting-poll records → invitations, one nudge, and the booking. The
//! poll's mail-and-calendar half, beside `bookings`: deterministic, no
//! model, run by the same timer, from the owner's own account.
//!
//! The seam is the record on disk. `factory-publish` writes
//! `~/.mecha/factory/polls/<id>.json` — who was invited, with the addresses
//! the box never learns, and a `lifecycle` block its `polls sweep` advances —
//! and this handler is *a* consumer of that contract, the one a mecha
//! deployment wires because this crate custodies the mail and calendar
//! credentials. It reads what is **due** and does it: an invitation still
//! unsent, the nudge the sweep queued, the event for a verdict of `book`.
//! It decides nothing — when a poll closes and what wins are the sweep's
//! (MEETING-POLL-UX-DESIGN.md §3.4).
//!
//! Three decisions worth their comments:
//!
//! - **The record is written back through the JSON it came from**, never
//!   through a struct of this crate's. `lifecycle` carries fields this
//!   handler has no business knowing (`ranked`, `pick_item`, whatever a
//!   newer `factory-publish` adds), and a round trip through a typed struct
//!   would drop them. Only the fields this half owns are set: `invites`,
//!   `nudge_due`/`nudged_at`, `booked`, `conflict`.
//! - **The ledger is the idempotency**, as it is for bookings: one line per
//!   message sent or event made, appended *after* the provider accepted it,
//!   and consulted *before* sending. A crash between the provider's answer
//!   and the record write re-runs nothing — the ledger already says it went
//!   — and the record catches up on the next tick.
//! - **The invitation is a template the owner already reviewed.** The
//!   subject and body ride in the record from the outbox card; this side
//!   substitutes the person's own link and nothing else. No model composes
//!   here, which is what lets a stranger's name pass through unread.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The templates `factory-publish` ships, kept here too so a record whose
/// template is empty still gets a sentence rather than nothing.
pub const DEFAULT_SUBJECT: &str = "When can you meet? — {title}";
pub const DEFAULT_NUDGE: &str = "\
A quick reminder — the poll for \"{title}\" closes {deadline_local}, and I don't have your answer yet:

    {url}

It takes about ten seconds. Thank you!";

/// One person on a poll, as the record knows them and the box does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub email: String,
    pub url: String,
}

/// A record with a lifecycle, loaded. The lifecycle stays a `Value` on
/// purpose — see the module docs.
#[derive(Debug, Clone)]
pub struct PollRecord {
    pub path: PathBuf,
    pub poll_id: String,
    pub title: String,
    pub duration_minutes: u32,
    pub people: Vec<Person>,
    pub value: Value,
    /// The lifecycle keys this process changed — the only ones `save`
    /// writes, into the file as it is *then*. The sweep and `mecha polls`
    /// run on the same timer and own the rest; a snapshot taken before a
    /// round of sends must not write their fields back over theirs.
    pub dirty: BTreeSet<&'static str>,
}

impl PollRecord {
    pub fn lifecycle(&self) -> &Value {
        &self.value["lifecycle"]
    }

    fn lifecycle_mut(&mut self) -> &mut Value {
        &mut self.value["lifecycle"]
    }

    fn set(&mut self, key: &'static str, value: Value) {
        self.lifecycle_mut()[key] = value;
        self.dirty.insert(key);
    }

    /// The account the record names, else the sweep's.
    pub fn account(&self) -> Option<&str> {
        self.lifecycle()["account"].as_str()
    }

    /// Every `{placeholder}` a template may use, for one person.
    pub fn vars(&self, person: &Person) -> Vec<(&'static str, String)> {
        let life = self.lifecycle();
        vec![
            ("{title}", self.title.clone()),
            ("{duration}", self.duration_minutes.to_string()),
            ("{deadline_local}", deadline_local(life)),
            ("{url}", person.url.clone()),
            ("{name}", person.name.clone()),
            (
                "{message}",
                life["message"].as_str().unwrap_or_default().to_string(),
            ),
        ]
    }
}

/// The deadline as the recipient reads it, in the poll's zone.
fn deadline_local(life: &Value) -> String {
    let deadline = life["deadline"]
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc));
    let tz = life["timezone"]
        .as_str()
        .and_then(|z| z.parse::<chrono_tz::Tz>().ok());
    match (deadline, tz) {
        (Some(at), Some(tz)) => at
            .with_timezone(&tz)
            .format("%A %-d %B at %-I:%M %p %Z")
            .to_string(),
        (Some(at), None) => at.format("%Y-%m-%d %H:%M UTC").to_string(),
        (None, _) => "when everyone has answered".to_string(),
    }
}

/// Substitute placeholders. An empty `{message}` takes its blank lines
/// with it, so a recipient never opens a letter that begins with two
/// empty lines.
pub fn render(template: &str, vars: &[(&str, String)]) -> String {
    let mut out = template.to_string();
    for (key, value) in vars {
        if *key == "{message}" && value.trim().is_empty() {
            out = out.replace("{message}\n\n", "").replace("{message}", "");
        } else {
            out = out.replace(key, value);
        }
    }
    out.trim().to_string()
}

/// What this tick owes for one record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    Invite(Person),
    Nudge(Person),
    /// The clean winner: create the event with everyone as attendee.
    Book {
        start: String,
        end: String,
    },
}

/// The jobs a record is owed, in the order they should run. Pure, so every
/// rule is a test with a JSON literal.
///
/// - an invitation for every name with no `invites[name]` while the poll is
///   open — but never after close, because a person invited to a poll that
///   has already decided is a person handed a dead link;
/// - a nudge for every name in `nudge_due`;
/// - the booking when the verdict is `book` and nothing is `booked` yet.
pub fn jobs_due(record: &PollRecord) -> Vec<Job> {
    let life = record.lifecycle();
    let mut jobs = Vec::new();
    let closed = !life["closed_at"].is_null();
    let by_name = |name: &str| record.people.iter().find(|p| p.name == name).cloned();

    if !closed {
        if let Some(invites) = life["invites"].as_object() {
            for (name, sent) in invites {
                if sent.is_null() {
                    if let Some(person) = by_name(name) {
                        jobs.push(Job::Invite(person));
                    }
                }
            }
        }
        if let Some(due) = life["nudge_due"].as_array() {
            for name in due.iter().filter_map(Value::as_str) {
                if let Some(person) = by_name(name) {
                    jobs.push(Job::Nudge(person));
                }
            }
        }
    }
    // A collision recorded on the last attempt waits for the sweep to turn
    // it into the owner's pick; this side never re-tries the slot.
    if life["verdict"].as_str() == Some("book")
        && life["booked"].is_null()
        && life["conflict"].is_null()
    {
        if let (Some(start), Some(end)) =
            (life["book"]["start"].as_str(), life["book"]["end"].as_str())
        {
            jobs.push(Job::Book {
                start: start.to_string(),
                end: end.to_string(),
            });
        }
    }
    jobs
}

/// The event as the attendees will see it. The description names the poll
/// and the owner's own sentence; the attendee list is the participants,
/// silent ones included — the invite is how they learn.
pub fn event_text(record: &PollRecord) -> (String, String) {
    let life = record.lifecycle();
    let mut description = String::new();
    if let Some(message) = life["message"].as_str().filter(|m| !m.trim().is_empty()) {
        description.push_str(message.trim());
        description.push_str("\n\n");
    }
    description.push_str(&format!(
        "Scheduled from the poll \"{}\" — a time everyone could make.\n",
        record.title
    ));
    (record.title.clone(), description)
}

// ---------------------------------------------------------------------------
// The records on disk.

/// Where `factory-publish` keeps the records.
pub fn records_dir() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("cannot determine home directory")?
        .join(".mecha")
        .join("factory")
        .join("polls"))
}

fn person(v: &Value) -> Option<Person> {
    Some(Person {
        name: v["name"].as_str()?.to_string(),
        email: v["email"].as_str()?.to_string(),
        url: v["url"].as_str().unwrap_or_default().to_string(),
    })
}

/// Parse one record. `Ok(None)` for a record with no lifecycle — a general
/// poll, or one from before the lifecycle existed — which this handler has
/// nothing to do for. A record *with* a lifecycle that this cannot make
/// sense of is an error, never `None`: it would otherwise be a poll that
/// silently gets no invitations, no nudge and no booking.
pub fn parse_record(path: &Path, value: Value) -> Result<Option<PollRecord>> {
    if !value["lifecycle"].is_object() {
        return Ok(None);
    }
    let poll_id = value["poll_id"]
        .as_str()
        .context("a lifecycle record with no `poll_id`")?
        .to_string();
    let people = value["participants"]
        .as_array()
        .context("a lifecycle record whose `participants` is not a list")?
        .iter()
        .filter_map(person)
        .collect();
    Ok(Some(PollRecord {
        path: path.to_path_buf(),
        poll_id,
        title: value["title"].as_str().unwrap_or("Meeting").to_string(),
        duration_minutes: value["duration_minutes"].as_u64().unwrap_or(0) as u32,
        people,
        value,
        dirty: BTreeSet::new(),
    }))
}

/// Every record with a lifecycle, and every file that could not be read —
/// reported, never skipped as if absent.
pub fn scan(dir: &Path) -> Result<(Vec<PollRecord>, Vec<String>)> {
    let mut records = Vec::new();
    let mut problems = Vec::new();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    for path in paths {
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()));
        match parsed
            .map_err(anyhow::Error::msg)
            .and_then(|v| parse_record(&path, v))
        {
            Ok(Some(record)) => records.push(record),
            Ok(None) => {}
            Err(e) => problems.push(format!("{}: {e:#}", path.display())),
        }
    }
    Ok((records, problems))
}

/// Write the keys this process changed back into the record as it is now —
/// re-read first, so another verb's writes since the load survive — then
/// temp-sibling-and-rename. Nothing dirty, nothing written.
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
        Err(_) => record.value.clone(),
    };
    if !current["lifecycle"].is_object() {
        current["lifecycle"] = json!({});
    }
    for key in &record.dirty {
        current["lifecycle"][*key] = record.lifecycle()[*key].clone();
    }
    let tmp = record.path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&current)?)
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &record.path)
        .with_context(|| format!("renaming into {}", record.path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// The ledger.

/// One action taken, remembered. `name` is empty for a booking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub poll_id: String,
    #[serde(default)]
    pub name: String,
    /// `invited` | `nudged` | `booked` | `conflict`.
    pub action: String,
    #[serde(default)]
    pub event_id: String,
    pub account: String,
    pub at: String,
}

impl LedgerEntry {
    fn key(&self) -> (String, String, String) {
        (self.poll_id.clone(), self.name.clone(), self.action.clone())
    }
}

pub fn ledger_path() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("cannot determine home directory")?
        .join(".mecha")
        .join("mail")
        .join("polls.jsonl"))
}

pub fn entries(path: &Path) -> Vec<LedgerEntry> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str::<LedgerEntry>(line).ok())
        .collect()
}

/// `(poll_id, name, action)` of everything already done.
pub fn handled(path: &Path) -> BTreeSet<(String, String, String)> {
    entries(path).into_iter().map(|e| e.key()).collect()
}

pub fn append(path: &Path, entry: &LedgerEntry) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    writeln!(file, "{}", serde_json::to_string(entry)?)?;
    Ok(())
}

/// The key a job writes to the ledger, and reads before running.
pub fn job_key(poll_id: &str, job: &Job) -> (String, String, String) {
    match job {
        Job::Invite(p) => (poll_id.to_string(), p.name.clone(), "invited".into()),
        Job::Nudge(p) => (poll_id.to_string(), p.name.clone(), "nudged".into()),
        Job::Book { .. } => (poll_id.to_string(), String::new(), "booked".into()),
    }
}

// ---------------------------------------------------------------------------
// Writing the outcome back into the record — only this half's fields.

pub fn mark_invited(record: &mut PollRecord, name: &str, at: DateTime<Utc>) {
    let mut invites = record.lifecycle()["invites"].clone();
    if !invites.is_object() {
        invites = json!({});
    }
    invites[name] = json!(at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    record.set("invites", invites);
}

/// All nudges for a tick went: clear the queue and stamp it.
pub fn mark_nudged(record: &mut PollRecord, at: DateTime<Utc>) {
    record.set("nudge_due", json!([]));
    record.set(
        "nudged_at",
        json!(at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
    );
}

pub fn mark_booked(record: &mut PollRecord, event_id: &str, account: &str, at: DateTime<Utc>) {
    record.set(
        "booked",
        json!({
            "event_id": event_id,
            "account": account,
            "at": at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        }),
    );
}

/// The clean winner collides with something now on the owner's calendar:
/// no event, and the collision written down — the same fail-closed
/// re-verify the bookings sweep runs. The verdict and the ranking are the
/// factory sweep's; it reads `conflict` and turns the poll into the owner's
/// pick over the full ranking, with the collision named on its row.
pub fn mark_conflict(record: &mut PollRecord, reason: &str) {
    record.set("conflict", json!(reason));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(life: Value) -> PollRecord {
        parse_record(
            Path::new("/tmp/x.json"),
            json!({
                "poll_id": "lab-20300128",
                "title": "Lab meeting",
                "duration_minutes": 60,
                "participants": [
                    {"name": "Priya", "email": "priya@example.edu", "url": "https://g/p/1"},
                    {"name": "Tal", "email": "tal@example.edu", "url": "https://g/p/2"}
                ],
                "lifecycle": life,
            }),
        )
        .unwrap()
        .expect("a record with a lifecycle")
    }

    /// Open poll, one invitation sent: the other is due, nothing else.
    #[test]
    fn an_unsent_invitation_is_due_while_the_poll_is_open() {
        let r = record(json!({
            "invites": {"Priya": "2030-01-28T12:00:00Z", "Tal": null},
        }));
        let jobs = jobs_due(&r);
        assert_eq!(jobs.len(), 1);
        assert!(matches!(&jobs[0], Job::Invite(p) if p.name == "Tal"));

        // Closed: a dead link is not sent.
        let r = record(json!({
            "invites": {"Priya": "2030-01-28T12:00:00Z", "Tal": null},
            "closed_at": "2030-01-30T12:00:00Z",
        }));
        assert!(jobs_due(&r).is_empty());
    }

    /// The sweep queues the nudge by name; this side owes exactly those.
    #[test]
    fn a_queued_nudge_is_due_by_name() {
        let r = record(json!({
            "invites": {"Priya": "2030-01-28T12:00:00Z", "Tal": "2030-01-28T12:00:00Z"},
            "nudge_due": ["Tal"],
        }));
        let jobs = jobs_due(&r);
        assert_eq!(jobs, vec![Job::Nudge(r.people[1].clone())]);
    }

    /// A verdict of `book` with nothing booked is the event; once booked,
    /// nothing — and a `pick` verdict is never this side's to act on.
    #[test]
    fn the_booking_is_due_once_and_only_for_a_book_verdict() {
        let book = json!({"start": "2030-02-05T18:00:00Z", "end": "2030-02-05T19:00:00Z", "duration_minutes": 60});
        let r = record(json!({
            "invites": {"Priya": "x", "Tal": "x"},
            "closed_at": "2030-01-30T12:00:00Z",
            "verdict": "book",
            "book": book,
        }));
        assert_eq!(
            jobs_due(&r),
            vec![Job::Book {
                start: "2030-02-05T18:00:00Z".into(),
                end: "2030-02-05T19:00:00Z".into()
            }]
        );

        let mut booked = r.clone();
        mark_booked(&mut booked, "ev1", "work", Utc::now());
        assert!(jobs_due(&booked).is_empty());

        let r = record(json!({
            "invites": {"Priya": "x", "Tal": "x"},
            "closed_at": "2030-01-30T12:00:00Z",
            "verdict": "pick",
            "book": book,
        }));
        assert!(
            jobs_due(&r).is_empty(),
            "a pick is the owner's, through mecha"
        );
    }

    /// The template the owner reviewed, with the person's own link in it —
    /// and no blank opening when there was no message.
    #[test]
    fn the_invitation_renders_the_owners_template_per_person() {
        let r = record(json!({
            "message": "Before the grant deadline, ideally.",
            "deadline": "2030-01-31T22:00:00Z",
            "timezone": "America/New_York",
            "subject": DEFAULT_SUBJECT,
            "invitation": "{message}\n\nPick a time for {title} ({duration} min): {url}\nBy {deadline_local}.",
        }));
        let body = render(
            r.lifecycle()["invitation"].as_str().unwrap(),
            &r.vars(&r.people[1]),
        );
        assert_eq!(
            body,
            "Before the grant deadline, ideally.\n\nPick a time for Lab meeting (60 min): https://g/p/2\nBy Thursday 31 January at 5:00 PM EST."
        );
        assert_eq!(
            render(DEFAULT_SUBJECT, &r.vars(&r.people[0])),
            "When can you meet? — Lab meeting"
        );

        let quiet = record(json!({"invitation": "{message}\n\nPick a time: {url}"}));
        assert_eq!(
            render(
                quiet.lifecycle()["invitation"].as_str().unwrap(),
                &quiet.vars(&quiet.people[0])
            ),
            "Pick a time: https://g/p/1"
        );
    }

    /// Only this half's fields are written, and everything else in the
    /// lifecycle survives the round trip.
    #[test]
    fn writing_back_touches_only_this_halfs_fields() {
        let mut r = record(json!({
            "invites": {"Priya": null, "Tal": null},
            "nudge_due": ["Tal"],
            "ranked": [{"reason": "from the sweep"}],
            "pick_item": "ob-123",
            "a_field_from_the_future": true,
        }));
        let at = DateTime::parse_from_rfc3339("2030-01-28T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        mark_invited(&mut r, "Priya", at);
        mark_nudged(&mut r, at);
        mark_booked(&mut r, "ev1", "work", at);
        let life = r.lifecycle();
        assert_eq!(life["invites"]["Priya"], "2030-01-28T12:00:00Z");
        assert!(life["invites"]["Tal"].is_null());
        assert_eq!(life["nudge_due"], json!([]));
        assert_eq!(life["nudged_at"], "2030-01-28T12:00:00Z");
        assert_eq!(life["booked"]["event_id"], "ev1");
        assert_eq!(life["ranked"][0]["reason"], "from the sweep");
        assert_eq!(life["pick_item"], "ob-123");
        assert_eq!(life["a_field_from_the_future"], true);
    }

    /// A collision is written down and the slot is never re-tried; the
    /// verdict and the ranking stay the factory sweep's to change.
    #[test]
    fn a_conflict_is_recorded_and_the_booking_is_not_retried() {
        let mut r = record(json!({
            "verdict": "book",
            "book": {"start": "2030-02-05T18:00:00Z", "end": "2030-02-05T19:00:00Z", "duration_minutes": 60},
            "ranked": [{"reason": "the sweep's"}],
        }));
        assert_eq!(jobs_due(&r).len(), 1);
        mark_conflict(&mut r, "your calendar now has something at that time");
        let life = r.lifecycle();
        assert_eq!(
            life["conflict"],
            "your calendar now has something at that time"
        );
        assert_eq!(life["verdict"], "book", "not this half's to change");
        assert_eq!(
            life["ranked"][0]["reason"], "the sweep's",
            "not this half's to change"
        );
        assert_eq!(
            r.dirty.iter().copied().collect::<Vec<_>>(),
            vec!["conflict"]
        );
        assert!(jobs_due(&r).is_empty());
    }

    /// The ledger key is what stops a re-send: one invitation per person per
    /// poll, one booking per poll.
    #[test]
    fn the_ledger_dedups_by_poll_name_and_action() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = dir.path().join("polls.jsonl");
        assert!(handled(&ledger).is_empty());
        append(
            &ledger,
            &LedgerEntry {
                poll_id: "lab".into(),
                name: "Tal".into(),
                action: "invited".into(),
                event_id: String::new(),
                account: "work".into(),
                at: "2030-01-28T12:00:00Z".into(),
            },
        )
        .unwrap();
        let done = handled(&ledger);
        let tal = Person {
            name: "Tal".into(),
            email: "t@e".into(),
            url: String::new(),
        };
        assert!(done.contains(&job_key("lab", &Job::Invite(tal.clone()))));
        assert!(!done.contains(&job_key("lab", &Job::Nudge(tal))));
        assert!(!done.contains(&job_key(
            "lab",
            &Job::Book {
                start: String::new(),
                end: String::new()
            }
        )));
    }

    /// `save` writes the keys this process changed into the file as it is
    /// now: a field the sweep wrote after the load survives, and nothing
    /// dirty means nothing written.
    #[test]
    fn save_merges_only_the_keys_this_process_changed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lab.json");
        let on_disk = json!({
            "poll_id": "lab", "title": "Lab", "duration_minutes": 60,
            "participants": [{"name": "Priya", "email": "p@e", "url": "u"}],
            "lifecycle": {"invites": {"Priya": null}, "verdict": null},
        });
        std::fs::write(&path, on_disk.to_string()).unwrap();
        let mut mine = parse_record(&path, on_disk.clone()).unwrap().unwrap();

        // Nothing changed: the file is not touched.
        save(&mine).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), on_disk.to_string());

        // The sweep closes the poll under us; we send Priya her invitation.
        let mut theirs = on_disk.clone();
        theirs["lifecycle"]["verdict"] = json!("pick");
        theirs["lifecycle"]["closed_at"] = json!("2030-01-30T12:00:00Z");
        std::fs::write(&path, theirs.to_string()).unwrap();
        let at = DateTime::parse_from_rfc3339("2030-01-28T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        mark_invited(&mut mine, "Priya", at);
        save(&mine).unwrap();
        let after: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            after["lifecycle"]["invites"]["Priya"],
            "2030-01-28T12:00:00Z"
        );
        assert_eq!(
            after["lifecycle"]["verdict"], "pick",
            "the sweep's write survives"
        );
        assert_eq!(after["lifecycle"]["closed_at"], "2030-01-30T12:00:00Z");
    }

    /// Records without a lifecycle — general polls, old ones — are not
    /// this handler's, and a broken file is a finding.
    #[test]
    fn scan_keeps_lifecycle_records_and_reports_broken_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.json"),
            json!({"poll_id": "a", "participants": [], "lifecycle": {}}).to_string(),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.json"),
            json!({"poll_id": "b"}).to_string(),
        )
        .unwrap();
        std::fs::write(dir.path().join("c.json"), "{").unwrap();
        // A lifecycle record this cannot read is a finding, not a poll
        // that quietly gets no mail.
        std::fs::write(
            dir.path().join("d.json"),
            json!({"poll_id": "d", "participants": "Priya", "lifecycle": {}}).to_string(),
        )
        .unwrap();
        let (records, problems) = scan(dir.path()).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].poll_id, "a");
        assert_eq!(problems.len(), 2);
        assert!(problems[0].contains("c.json"));
        assert!(
            problems[1].contains("d.json") && problems[1].contains("participants"),
            "{}",
            problems[1]
        );
    }
}
