use super::*;
use crate::system::Category;
use chrono::{Duration, TimeZone};
use rusqlite::{params, Connection};

/// A series with a little of everything, an hour back from `now`.
fn series() -> (std::path::PathBuf, DateTime<Utc>) {
    let dir = std::env::temp_dir().join(format!("mecha-system-read-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("series.sqlite");
    let c = Connection::open(&db).unwrap();
    c.execute_batch(crate::system::SCHEMA).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 10, 12, 12, 0, 0).unwrap();
    let at = |m: i64| {
        (now - Duration::minutes(m))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    };
    for m in [30, 20, 10] {
        for (meas, v) in [
            (Measurement::DiskRootUsed, 500e9),
            (Measurement::VoiceLive, 1.0),
            (Measurement::MemoryOomKills, 1.0),
        ] {
            c.execute(
                "INSERT INTO reading_minute VALUES (?1, ?2, ?3)",
                params![at(m), meas as u16, v],
            )
            .unwrap();
        }
        for cat in [Category::ChatModel, Category::Voice] {
            c.execute(
                "INSERT INTO category_minute VALUES (?1, ?2, 1073741824, 2.0, 1, 0)",
                params![at(m), cat as u8],
            )
            .unwrap();
        }
    }
    c.execute(
        "INSERT INTO model_kinds VALUES (1, 'chat'), (4, 'speech to text'), (5, 'text to speech')",
        [],
    )
    .unwrap();
    for kind in [1u8, 4, 5] {
        c.execute(
            "INSERT INTO model_minute (at, kind, model, model_known, running, mem_bytes, requests)
             VALUES (?1, ?2, 'some-model', 1, 1, 1073741824, 2)",
            params![at(10), kind],
        )
        .unwrap();
    }
    (db, now)
}

fn ask(q: Option<&str>, w: Window, by: bool) -> ToolOutput {
    let (db, now) = series();
    answer(&db, None, None, q, w, by, now).unwrap()
}

/// R7: voice is readable now, never over a window — refused by name, not
/// dropped in silence, and nothing of its history is in the result.
#[test]
fn voice_history_is_refused_by_name() {
    let out = ask(Some("voice"), Window::H1, false);
    assert!(!out.is_error, "{}", out.content);
    assert!(
        out.content.contains("Not shown: voice.live"),
        "{}",
        out.content
    );
    assert!(!out.content.contains("on in"), "{}", out.content);
}

#[test]
fn the_voice_category_is_left_out_of_a_window() {
    let out = ask(Some("category.memory"), Window::H1, true);
    assert!(out.content.contains("chat model:"), "{}", out.content);
    assert!(!out.content.contains("    voice:"), "{}", out.content);
    assert!(
        out.content.contains("Not shown: category.memory for voice"),
        "{}",
        out.content
    );
}

#[test]
fn speech_models_are_now_only() {
    let window = ask(Some("models"), Window::H1, false);
    assert!(
        window
            .content
            .contains("chat: ran 1 of the window's 60 minutes (recorded for 1)"),
        "{}",
        window.content
    );
    assert!(
        !window.content.contains("- speech to text"),
        "{}",
        window.content
    );
    assert!(
        !window.content.contains("- text to speech"),
        "{}",
        window.content
    );
    assert!(window
        .content
        .contains("Not shown: speech to text and text to speech"));
    let now = ask(Some("models"), Window::Now, false);
    assert!(
        now.content
            .contains("- text to speech: running, some-model"),
        "{}",
        now.content
    );
}

/// R7 on the now path (review of #660): a speech kind's speed exists only in
/// a minute it spoke, so "now" must not reach back a day and print when the
/// last call was. It says only whether it is speaking now, with no time.
#[test]
fn a_speech_models_now_never_says_when_the_last_call_was() {
    let (db, now) = series();
    let c = Connection::open(&db).unwrap();
    let at = |m: i64| {
        (now - Duration::minutes(m))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    };
    // Spoke eleven hours ago, silent since.
    c.execute(
        "INSERT INTO model_minute (at, kind, model, model_known, running, mem_bytes, rtf)
         VALUES (?1, 5, 'some-model', 1, 1, 1073741824, 0.24)",
        params![at(660)],
    )
    .unwrap();
    let out = answer(&db, None, None, Some("models"), Window::Now, false, now).unwrap();
    assert!(!out.content.contains("0.24"), "{}", out.content);
    assert!(
        out.content
            .contains("text to speech: running, some-model, 1.0 GiB; not speaking now"),
        "{}",
        out.content
    );
    // Speaking within the last minutes: said, with no time.
    c.execute(
        "UPDATE model_minute SET rtf = 0.31 WHERE kind = 4 AND at = ?1",
        params![at(10)],
    )
    .unwrap();
    c.execute(
        "INSERT INTO model_minute (at, kind, model, model_known, running, mem_bytes, rtf)
         VALUES (?1, 4, 'some-model', 1, 1, 1073741824, 0.31)",
        params![at(1)],
    )
    .unwrap();
    let out = answer(&db, None, None, Some("models"), Window::Now, false, now).unwrap();
    assert!(
        out.content
            .contains("speech to text: running, some-model, 1.0 GiB; speaking now, RTF 0.31\n"),
        "{}",
        out.content
    );
}

/// A level reads as latest/min/mean/max with its stretches; a per-minute
/// count is totalled.
#[test]
fn a_window_reads_as_a_summary_in_its_unit() {
    let disk = ask(Some("disk.root.used"), Window::H1, false);
    assert!(
        disk.content.contains("latest 465.66 GiB"),
        "{}",
        disk.content
    );
    assert!(
        disk.content
            .contains("(3 of the window's 60 minutes known; 57 not recorded)"),
        "{}",
        disk.content
    );
    assert!(disk.content.contains("by stretch:"), "{}", disk.content);
    let oom = ask(Some("memory.oom_kills"), Window::H1, false);
    assert!(
        oom.content.contains("- memory.oom_kills: 3 in all"),
        "{}",
        oom.content
    );
}

#[test]
fn an_unknown_name_lists_the_real_ones() {
    let out = ask(Some("disk.free"), Window::Now, false);
    assert!(out.is_error);
    assert!(out.content.contains("disk.root.used"), "{}", out.content);
}

/// §5, R3b: private, nothing from outside — our own sampler wrote every
/// value — no egress; read-only so a read-only chat can ask.
#[test]
fn it_declares_private_and_nothing_else() {
    let tool = SystemRead::new(std::path::PathBuf::from("/nonexistent"), None);
    let caps = tool.capabilities();
    assert!(caps.private_data);
    assert!(!caps.untrusted_input && !caps.destructive);
    assert_eq!(caps.egress, crate::tool::Egress::None);
    assert!(tool.read_only());
}

/// A per-minute count's stretches are totals, as its headline is: a burst in
/// one minute is not diluted to 0.0 across its stretch (review of #660).
#[test]
fn a_counts_stretches_are_totals() {
    let (db, now) = series();
    let c = Connection::open(&db).unwrap();
    let at = |m: i64| {
        (now - Duration::minutes(m))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    };
    for m in 1..=9 {
        c.execute(
            "INSERT OR REPLACE INTO reading_minute VALUES (?1, ?2, 0.0)",
            params![at(m), Measurement::MemoryOomKillsGlobal as u16],
        )
        .unwrap();
    }
    c.execute(
        "UPDATE reading_minute SET value = 3.0 WHERE at = ?1 AND measurement = ?2",
        params![at(2), Measurement::MemoryOomKillsGlobal as u16],
    )
    .unwrap();
    let out = answer(
        &db,
        None,
        None,
        Some("memory.oom_kills.global"),
        Window::H1,
        false,
        now,
    )
    .unwrap();
    assert!(out.content.contains("3 in all"), "{}", out.content);
    assert!(
        out.content.contains(" 3 ·") || out.content.ends_with(" 3\n"),
        "{}",
        out.content
    );
}

/// A series from before S2a has no `reading_minute`: said, not an error.
#[test]
fn an_older_series_is_said_not_failed() {
    let dir = std::env::temp_dir().join(format!("mecha-system-read-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("series.sqlite");
    Connection::open(&db)
        .unwrap()
        .execute_batch("CREATE TABLE system_minute (at TEXT PRIMARY KEY);")
        .unwrap();
    let out = answer(
        &db,
        None,
        None,
        Some("disk.root.used"),
        Window::H1,
        false,
        Utc::now(),
    )
    .unwrap();
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("older sampler"), "{}", out.content);
    let _ = std::fs::remove_dir_all(&dir);
}
