use super::*;
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
        window.content.contains("chat: ran 1 of 1 minutes"),
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
        disk.content.contains("(3 of 3 minutes known)"),
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
