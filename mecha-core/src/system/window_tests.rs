use chrono::{Duration, TimeZone, Utc};
use rusqlite::{params, Connection};

use super::*;

fn scratch() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mecha-window-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("series.sqlite")
}

fn at(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Unknown is never zero: a NULL minute is counted unknown and stays out of
/// min, mean and max, and a stretch with no value is `None`.
#[test]
fn a_window_summarises_known_minutes_and_counts_unknown_ones() {
    let db = scratch();
    let c = Connection::open(&db).unwrap();
    c.execute_batch(super::super::SCHEMA).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 10, 12, 12, 0, 0).unwrap();
    let m = Measurement::MemoryAvailable as u16;
    for (mins_ago, v) in [
        (50, Some(10.0)),
        (40, None),
        (30, Some(30.0)),
        (5, Some(20.0)),
    ] {
        c.execute(
            "INSERT INTO reading_minute VALUES (?1, ?2, ?3)",
            params![at(now - Duration::minutes(mins_ago)), m, v],
        )
        .unwrap();
    }
    // Outside the window: not counted.
    c.execute(
        "INSERT INTO reading_minute VALUES (?1, ?2, 999.0)",
        params![at(now - Duration::hours(2)), m],
    )
    .unwrap();
    let Some(Windowed::One(s)) = read(&db, Measurement::MemoryAvailable, Window::H1, now).unwrap()
    else {
        panic!("one summary")
    };
    assert_eq!((s.minutes, s.unknown), (4, 1));
    assert_eq!((s.min, s.max), (Some(10.0), Some(30.0)));
    assert_eq!(s.mean, Some(20.0));
    assert_eq!(s.latest.as_ref().map(|l| l.1), Some(20.0));
    assert_eq!(s.buckets.len(), 12, "an hour in five-minute stretches");
    assert_eq!(s.buckets[2].mean, Some(10.0), "11:10–11:15");
    assert_eq!(
        s.buckets[4].mean, None,
        "11:20, an unknown minute, is no value"
    );
    assert_eq!(
        s.buckets[0].mean, None,
        "a stretch with no row is no value, not 0"
    );
}

#[test]
fn no_series_is_said_and_now_is_not_a_window() {
    let db = scratch();
    let now = Utc::now();
    assert!(read(&db, Measurement::CpuBusy, Window::H1, now)
        .unwrap()
        .is_none());
    assert!(read(&db, Measurement::CpuBusy, Window::Now, now).is_err());
    assert!(models_now(&db, now).unwrap().is_none());
}

/// By kind of work, one summary per category, read from its named column.
#[test]
fn a_category_window_reads_each_kinds_column() {
    let db = scratch();
    let c = Connection::open(&db).unwrap();
    c.execute_batch(super::super::SCHEMA).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 10, 12, 12, 0, 0).unwrap();
    c.execute(
        "INSERT INTO category_minute VALUES (?1, ?2, ?3, 1.5, 3, 0)",
        params![
            at(now - Duration::minutes(3)),
            Category::ChatModel as u8,
            4096_i64
        ],
    )
    .unwrap();
    let Some(Windowed::ByCategory {
        rows: cats,
        withheld,
    }) = read(&db, Measurement::CategoryMemory, Window::H1, now).unwrap()
    else {
        panic!("by category")
    };
    assert_eq!(
        withheld,
        vec![Category::Voice.label()],
        "R7, in the read itself"
    );
    let chat = cats
        .iter()
        .find(|(l, _)| *l == Category::ChatModel.label())
        .unwrap();
    assert_eq!(chat.1.latest.as_ref().map(|l| l.1), Some(4096.0));
    assert!(
        !cats.iter().any(|(l, _)| *l == Category::Voice.label()),
        "the voice category is never read over a window"
    );
}

/// The model rows: the newest per kind with its last measured speed, and a
/// window that names each model loaded in turn — a switch shows as two.
#[test]
fn model_rows_show_what_is_loaded_and_each_switch() {
    let db = scratch();
    let c = Connection::open(&db).unwrap();
    c.execute_batch(super::super::SCHEMA).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 10, 12, 12, 0, 0).unwrap();
    c.execute(
        "INSERT INTO model_kinds VALUES (1, 'chat'), (5, 'text to speech')",
        [],
    )
    .unwrap();
    let row = |mins: i64, kind: u8, model: &str, gen: Option<f64>, reqs: i64| {
        c.execute(
            "INSERT INTO model_minute (at, kind, model, model_known, running, mem_bytes,
                 requests, gen_tps)
             VALUES (?1, ?2, ?3, 1, 1, 1073741824, ?4, ?5)",
            params![at(now - Duration::minutes(mins)), kind, model, reqs, gen],
        )
        .unwrap();
    };
    row(50, 1, "small-model", Some(90.0), 2);
    row(30, 1, "big-model", None, 0);
    row(10, 1, "big-model", Some(40.0), 3);
    row(10, 5, "voice-model", None, 1);

    let now_rows = models_now(&db, now).unwrap().unwrap();
    let chat = now_rows.iter().find(|r| r.kind_id == 1).unwrap();
    assert_eq!(chat.model.as_deref(), Some("big-model"));
    assert_eq!(chat.speed.as_ref().map(|s| s.1), Some(40.0));

    let w = models_window(&db, Window::H1, now).unwrap().unwrap();
    assert_eq!(w.len(), 1, "a speech kind is not read over a window (R7)");
    assert_eq!(
        w[0].models,
        vec![("small-model".to_string(), 1), ("big-model".to_string(), 2)]
    );
    assert_eq!(w[0].requests, Some(5.0));
    assert_eq!(
        w[0].speed.mean,
        Some(65.0),
        "minutes without requests are left out"
    );
}

/// R7 in the read: `voice.live` over a window is withheld here, whatever
/// formats it; and a measurement with no row while the sampler ran is "not on
/// this machine", not a quiet window (review of #660).
#[test]
fn voice_is_withheld_and_absence_is_not_here() {
    let db = scratch();
    let c = Connection::open(&db).unwrap();
    c.execute_batch(super::super::SCHEMA).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 10, 12, 12, 0, 0).unwrap();
    c.execute(
        "INSERT INTO system_minute (at) VALUES (?1)",
        params![at(now - Duration::minutes(5))],
    )
    .unwrap();
    assert_eq!(
        read(&db, Measurement::VoiceLive, Window::H1, now).unwrap(),
        Some(Windowed::Withheld)
    );
    // Not listed by the sampler that wrote the series: an older sampler's,
    // not this machine's lack.
    assert_eq!(
        read(&db, Measurement::GpuTemperature, Window::H1, now).unwrap(),
        Some(Windowed::OlderSampler)
    );
    // Listed, and still no row while it sampled: not on this machine.
    c.execute(
        "INSERT INTO measurements (id, name, unit) VALUES (?1, 'gpu.temperature', 'celsius')",
        params![Measurement::GpuTemperature as u16],
    )
    .unwrap();
    assert_eq!(
        read(&db, Measurement::GpuTemperature, Window::H1, now).unwrap(),
        Some(Windowed::NotHere)
    );
}
