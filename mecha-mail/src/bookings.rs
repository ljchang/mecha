//! Drained booking records → calendar events. The inbound sibling of
//! `freebusy`: a deterministic verb on the drain path, no model anywhere,
//! run by the same timer that drains.
//!
//! The seam is the record on disk. mecha-factory's side ends at
//! `~/.mecha/requests/<seq>-<type>.json` — data, structurally parseable,
//! client-agnostic — and this handler is *a* consumer of that contract, the
//! one a mecha deployment wires because this crate custodies the calendar
//! credentials. A deployment that is not mecha brings its own handler; the
//! factory never learns what a calendar is.
//!
//! Two decisions worth their comments:
//!
//! - **The invite is the provider's own.** The event is created with the
//!   requester as attendee and notifications on, so the confirmation the
//!   visitor receives is a native calendar invite from the user's real
//!   mailbox — the most deliverable calendar mail that exists, with an
//!   Accept/Decline that RSVPs back to the real event, and a native
//!   retraction when the event is later deleted. The box's SES sends only
//!   account plumbing (magic links); it never speaks for the user. The
//!   cancel capability rides in the event description, which both Gmail
//!   and Outlook render in the invite.
//! - **The ledger is the idempotency.** Records persist in the request
//!   store after handling — they are the archive — so "which bookings
//!   already have events" cannot be derived from the directory. One line
//!   per created event in `bookings.jsonl`, appended after the create
//!   succeeds: a crash between the two re-runs the create, which is the
//!   right side to err on (a duplicate event is visible and deletable; a
//!   ledger row for an event that was never made is a meeting nobody sees).

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What the handler needs from one drained booking record.
#[derive(Debug, Clone, PartialEq)]
pub struct DrainedBooking {
    pub seq: i64,
    pub type_id: String,
    pub booking_id: String,
    /// RFC 3339 UTC, exactly as the record carries them.
    pub start: String,
    pub end: String,
    pub name: String,
    pub email: Option<String>,
    pub purpose: Option<String>,
    pub topic: Option<String>,
    /// The box-minted cancel capability, when the record carries one.
    pub manage_url: Option<String>,
}

/// Parse one record, if it is a booking this handler should act on.
///
/// `None` for anything else — a plain request, an invalid record, a booking
/// missing its machinery — because this scanner walks a directory holding
/// every kind of drained record and "not mine" is the ordinary case, not an
/// error. The one thing never guessed at: a record with a `_booking_id` but
/// unparseable stamps returns `None` rather than a booking with invented
/// times.
pub fn parse_record(record: &Value) -> Option<DrainedBooking> {
    if record["valid"].as_bool() != Some(true) {
        return None;
    }
    let values = record["values"].as_object()?;
    let booking_id = values.get("_booking_id")?.as_str()?.to_string();
    let start = values.get("_slot_start")?.as_str()?.to_string();
    let end = values.get("_slot_end")?.as_str()?.to_string();
    for stamp in [&start, &end] {
        chrono::DateTime::parse_from_rfc3339(stamp).ok()?;
    }
    let text = |key: &str| {
        values
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    Some(DrainedBooking {
        seq: record["seq"].as_i64().unwrap_or(0),
        type_id: record["type_id"].as_str().unwrap_or_default().to_string(),
        booking_id,
        start,
        end,
        name: text("requester_name").unwrap_or_else(|| "someone".into()),
        email: record["reply_to"]
            .as_str()
            .map(str::to_string)
            .or_else(|| text("requester_email")),
        purpose: text("purpose"),
        topic: text("topic"),
        manage_url: text("_manage_url"),
    })
}

/// Every cancellation in the request store, oldest first.
pub fn scan_cancellations(dir: &Path) -> Result<Vec<(i64, String)>> {
    let mut cancellations = Vec::new();
    for record in read_records(dir)? {
        if let Some(cancellation) = parse_cancellation(&record) {
            cancellations.push(cancellation);
        }
    }
    cancellations.sort();
    Ok(cancellations)
}

fn read_records(dir: &Path) -> Result<Vec<Value>> {
    let mut records = Vec::new();
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("reading the request store at {}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(record) = serde_json::from_str::<Value>(&text) {
            records.push(record);
        }
    }
    Ok(records)
}

/// Every booking in the request store, oldest first.
pub fn scan(dir: &Path) -> Result<Vec<DrainedBooking>> {
    let mut bookings = Vec::new();
    for record in read_records(dir)? {
        if let Some(booking) = parse_record(&record) {
            bookings.push(booking);
        }
    }
    bookings.sort_by_key(|b| b.seq);
    Ok(bookings)
}

/// One action taken, remembered.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub booking_id: String,
    pub event_id: String,
    pub account: String,
    pub seq: i64,
    pub created_at: String,
    /// `created` or `cancelled`. Lines written before the field existed are
    /// creations — the only action there was.
    #[serde(default = "default_action")]
    pub action: String,
}

fn default_action() -> String {
    "created".into()
}

/// Every complete ledger line, oldest first.
pub fn entries(path: &Path) -> Vec<LedgerEntry> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str::<LedgerEntry>(line).ok())
        .collect()
}

/// A cancellation record: machinery only, nothing typed by anyone. `None`
/// for everything else, exactly as [`parse_record`] answers for bookings.
pub fn parse_cancellation(record: &Value) -> Option<(i64, String)> {
    if record["valid"].as_bool() != Some(true) {
        return None;
    }
    let values = record["values"].as_object()?;
    if values.get("_cancelled").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    Some((
        record["seq"].as_i64().unwrap_or(0),
        values.get("_booking_id")?.as_str()?.to_string(),
    ))
}

/// `~/.mecha/mail/bookings.jsonl` (beside the account registry).
pub fn ledger_path() -> Result<PathBuf> {
    Ok(crate::accounts::dir()?.join("bookings.jsonl"))
}

/// The booking ids that already have events. A torn trailing line is
/// skipped; every complete line counts — under-reading the ledger recreates
/// an event (visible, deletable), over-reading it silently drops a meeting.
pub fn handled(path: &Path) -> BTreeSet<String> {
    entries(path)
        .into_iter()
        .filter(|e| e.action == "created")
        .map(|e| e.booking_id)
        .collect()
}

/// The booking ids whose cancellation is already processed.
pub fn cancelled(path: &Path) -> BTreeSet<String> {
    entries(path)
        .into_iter()
        .filter(|e| e.action == "cancelled")
        .map(|e| e.booking_id)
        .collect()
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

/// A reminder tier: how long before the meeting it fires, and the ledger
/// action that remembers it fired. Two tiers, the industry's converged
/// default; the manifest's `[policy] reminders` can drive this later.
pub const REMINDER_TIERS: [(i64, &str); 2] =
    [(24 * 60, "reminded_24h"), (60, "reminded_1h")];

/// Which reminders are owed right now. Pure, so every rule is testable
/// without a clock or a mailbox:
///
/// - only bookings whose event was **created** and never **cancelled**;
/// - only future meetings — a reminder after the fact is an apology;
/// - each tier fires once (its ledger action is its memory);
/// - and a tier whose window was already open when the booking was made is
///   **suppressed**, not sent late: someone who booked at 9pm for 10am
///   already has the invite in hand, and a "24 hours to go" at 9:01pm is
///   machinery talking to itself.
pub fn reminders_due(
    bookings: &[DrainedBooking],
    ledger: &[LedgerEntry],
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<(DrainedBooking, &'static str)> {
    use std::collections::BTreeMap;
    let mut created: BTreeMap<&str, &LedgerEntry> = BTreeMap::new();
    let mut done: BTreeSet<(&str, &str)> = BTreeSet::new();
    let mut cancelled: BTreeSet<&str> = BTreeSet::new();
    for entry in ledger {
        match entry.action.as_str() {
            "created" => {
                created.insert(&entry.booking_id, entry);
            }
            "cancelled" => {
                cancelled.insert(&entry.booking_id);
            }
            action => {
                done.insert((entry.booking_id.as_str(), action));
            }
        }
    }

    let mut due = Vec::new();
    for booking in bookings {
        let Some(entry) = created.get(booking.booking_id.as_str()) else {
            continue;
        };
        if cancelled.contains(booking.booking_id.as_str()) {
            continue;
        }
        let (Ok(start), Ok(booked_at)) = (
            chrono::DateTime::parse_from_rfc3339(&booking.start),
            chrono::DateTime::parse_from_rfc3339(&entry.created_at),
        ) else {
            continue;
        };
        for (minutes, action) in REMINDER_TIERS {
            let window = chrono::Duration::minutes(minutes);
            let fires = start.with_timezone(&chrono::Utc) - now;
            let existed_before_window =
                start.with_timezone(&chrono::Utc) - booked_at.with_timezone(&chrono::Utc)
                    > window;
            if fires > chrono::Duration::zero()
                && fires <= window
                && existed_before_window
                && !done.contains(&(booking.booking_id.as_str(), action))
            {
                due.push((booking.clone(), action));
            }
        }
    }
    due
}

/// The event's title and description. The description carries the
/// stranger's own words (name, purpose, notes) — inert text on the user's
/// own calendar, read by a human, with no model and no tool surface
/// anywhere on this path; the same trust level as the mail already in
/// their inbox.
pub fn event_text(booking: &DrainedBooking) -> (String, String) {
    let title = match &booking.purpose {
        Some(purpose) => format!("{} — {purpose}", booking.name),
        None => format!("{} — booked meeting", booking.name),
    };
    let mut description = format!(
        "Booked via the `{}` page (request #{}, booking {}).\n",
        booking.type_id, booking.seq, booking.booking_id
    );
    if let Some(email) = &booking.email {
        description.push_str(&format!("Email: {email}\n"));
    }
    if let Some(topic) = &booking.topic {
        description.push_str(&format!("\nTheir notes:\n{topic}\n"));
    }
    // In the description on purpose: both Gmail and Outlook render the
    // description in the invite mail, so the cancel capability reaches the
    // visitor inside the one message the provider already sends.
    if let Some(url) = &booking.manage_url {
        description.push_str(&format!("\nNeed to change or cancel? {url}\n"));
    }
    (title, description)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record() -> Value {
        json!({
            "seq": 21,
            "type_id": "book",
            "valid": true,
            "reply_to": "priya@example.edu",
            "values": {
                "requester_name": "Priya",
                "requester_email": "priya@example.edu",
                "purpose": "advising",
                "topic": "Reading before we meet",
                "_booking_id": "abc123",
                "_slot_start": "2026-08-25T18:00:00Z",
                "_slot_end": "2026-08-25T18:30:00Z",
                "_duration_minutes": 30
            }
        })
    }

    #[test]
    fn a_booking_record_parses_and_others_answer_none() {
        let booking = parse_record(&record()).unwrap();
        assert_eq!(booking.booking_id, "abc123");
        assert_eq!(booking.start, "2026-08-25T18:00:00Z");
        assert_eq!(booking.email.as_deref(), Some("priya@example.edu"));

        // Not mine: a plain request, an invalid record, invented times.
        let mut plain = record();
        plain["values"].as_object_mut().unwrap().remove("_booking_id");
        assert!(parse_record(&plain).is_none());
        let mut invalid = record();
        invalid["valid"] = json!(false);
        assert!(parse_record(&invalid).is_none(), "an invalid record is nobody's booking");
        let mut torn = record();
        torn["values"]["_slot_start"] = json!("tuesdayish");
        assert!(parse_record(&torn).is_none(), "never invent a meeting time");
    }

    #[test]
    fn the_ledger_makes_reruns_no_ops() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bookings.jsonl");
        assert!(handled(&path).is_empty());
        append(
            &path,
            &LedgerEntry {
                booking_id: "abc123".into(),
                event_id: "ev9".into(),
                account: "dartmouth".into(),
                seq: 21,
                created_at: "2026-08-08T12:00:00Z".into(),
                action: "created".into(),
            },
        )
        .unwrap();
        let done = handled(&path);
        assert!(done.contains("abc123"));
        // A torn trailing line loses only itself.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"booking_id\": \"trun")
            .unwrap();
        assert_eq!(handled(&path).len(), 1);
    }

    #[test]
    fn scan_finds_bookings_and_skips_everything_else() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("0000000021-book.json"),
            record().to_string(),
        )
        .unwrap();
        std::fs::write(dir.path().join("0000000002-meeting.json"), "{\"valid\": true, \"values\": {}}")
            .unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not json").unwrap();
        let found = scan(dir.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].booking_id, "abc123");
    }

    #[test]
    fn event_text_carries_the_stranger_inertly() {
        let mut with_url = record();
        with_url["values"]["_manage_url"] = json!("https://gate.example.org/s/alice/book/m/tok");
        let (title, description) = event_text(&parse_record(&with_url).unwrap());
        assert_eq!(title, "Priya — advising");
        assert!(description.contains("request #21"));
        assert!(description.contains("Reading before we meet"));
        assert!(
            description.contains("cancel? https://gate.example.org/s/alice/book/m/tok"),
            "the cancel capability rides the invite: {description}"
        );
    }

    /// The cancellation loop's data half: a machinery-only record parses as
    /// a cancellation and nothing else does; ledger lines written before
    /// `action` existed still count as creations; and a processed
    /// cancellation is remembered apart from them.
    #[test]
    fn cancellations_parse_and_the_ledger_remembers_both_actions() {
        let cancel = json!({
            "seq": 30, "type_id": "book", "valid": true,
            "values": {"_booking_id": "abc123", "_cancelled": true,
                        "_slot_start": "2026-08-25T18:00:00Z"}
        });
        assert_eq!(parse_cancellation(&cancel), Some((30, "abc123".into())));
        assert!(parse_record(&cancel).is_none(), "a cancellation is not a booking");
        assert!(parse_cancellation(&record()).is_none(), "a booking is not a cancellation");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bookings.jsonl");
        // A line from before `action` existed: a creation, the only action
        // there was.
        std::fs::write(
            &path,
            "{\"booking_id\":\"old1\",\"event_id\":\"ev1\",\"account\":\"a\",\"seq\":1,\"created_at\":\"t\"}\n",
        )
        .unwrap();
        append(
            &path,
            &LedgerEntry {
                booking_id: "old1".into(),
                event_id: "ev1".into(),
                account: "a".into(),
                seq: 30,
                created_at: "t2".into(),
                action: "cancelled".into(),
            },
        )
        .unwrap();
        assert!(handled(&path).contains("old1"), "pre-action lines are creations");
        assert!(cancelled(&path).contains("old1"));
    }

    /// Every reminder rule, against one clock: each tier fires inside its
    /// window and only once, a cancelled booking is silent, the past is
    /// silent, and a booking made inside a tier's window suppresses that
    /// tier — the invite in hand IS that reminder.
    #[test]
    fn reminders_fire_once_per_tier_and_never_absurdly() {
        let t = |s: &str| chrono::DateTime::parse_from_rfc3339(s)
            .unwrap()
            .with_timezone(&chrono::Utc);
        let booking = |id: &str, start: &str| DrainedBooking {
            seq: 1,
            type_id: "book".into(),
            booking_id: id.into(),
            start: start.into(),
            end: start.replace("T18", "T19"),
            name: "Priya".into(),
            email: Some("priya@example.edu".into()),
            purpose: None,
            topic: None,
            manage_url: None,
        };
        let entry = |id: &str, action: &str, at: &str| LedgerEntry {
            booking_id: id.into(),
            event_id: "ev".into(),
            account: "a".into(),
            seq: 1,
            created_at: at.into(),
            action: action.into(),
        };
        let now = t("2026-08-25T12:00:00Z");

        // Booked a week ago, meeting in 6h: the 24h tier is due, the 1h not.
        let b = booking("b1", "2026-08-25T18:00:00Z");
        let ledger = vec![entry("b1", "created", "2026-08-18T12:00:00Z")];
        let due = reminders_due(std::slice::from_ref(&b), &ledger, now);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].1, "reminded_24h");

        // The 24h already sent: quiet until the 1h window opens…
        let ledger2 = [ledger.clone(), vec![entry("b1", "reminded_24h", "t")]].concat();
        assert!(reminders_due(std::slice::from_ref(&b), &ledger2, now).is_empty());
        // …then the 1h fires, once.
        let near = t("2026-08-25T17:30:00Z");
        let due = reminders_due(std::slice::from_ref(&b), &ledger2, near);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].1, "reminded_1h");

        // Cancelled: silent. Past: silent.
        let cancelled = [ledger.clone(), vec![entry("b1", "cancelled", "t")]].concat();
        assert!(reminders_due(std::slice::from_ref(&b), &cancelled, now).is_empty());
        assert!(
            reminders_due(std::slice::from_ref(&b), &ledger, t("2026-08-25T19:00:00Z")).is_empty(),
            "a reminder after the fact is an apology"
        );

        // Booked 30 minutes ago for later today: both windows were already
        // open at booking time, so both tiers are suppressed — the invite
        // in hand is the reminder.
        let fresh = vec![entry("b1", "created", "2026-08-25T11:30:00Z")];
        assert!(reminders_due(&[b], &fresh, now).is_empty());
    }
}