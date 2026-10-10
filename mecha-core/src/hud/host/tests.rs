use std::path::PathBuf;

use chrono::{TimeZone, Utc};
use rusqlite::Connection;

use super::*;

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "mecha-hud-host-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_discriminants_are_pinned_and_an_unknown_one_is_none() {
    let pinned = [
        (Category::Other, 0),
        (Category::ChatModel, 1),
        (Category::Embeddings, 2),
        (Category::Ocr, 3),
        (Category::Voice, 4),
        (Category::ImageGen, 5),
        (Category::Mecha, 6),
    ];
    for (c, n) in pinned {
        assert_eq!(c as u8, n, "{c:?}'s number is a wire format");
        assert_eq!(Category::from_u8(n), Some(c));
    }
    assert_eq!(
        Category::from_u8(7),
        None,
        "unknown is None, never a neighbour"
    );
}

#[test]
fn units_map_to_categories_and_anything_else_is_other() {
    for (unit, cat) in [
        ("llama-local.service", Category::ChatModel),
        ("llama-embed.service", Category::Embeddings),
        ("llama-embed-proxy.service", Category::Embeddings),
        ("llama-ocr.service", Category::Ocr),
        ("comfyui.service", Category::ImageGen),
        ("mecha-comfyui-idle-reset.service", Category::ImageGen),
        ("mecha-breeze-tts.service", Category::Voice),
        ("mecha-voice-worker.service", Category::Voice),
        ("mecha-parakeet.service", Category::Voice),
        ("mecha-serve.service", Category::Mecha),
        ("mecha-hud.service", Category::Mecha),
        ("ollama.service", Category::Other),
        ("tmux-spawn-1234.scope", Category::Other),
        ("", Category::Other),
    ] {
        assert_eq!(category_of(unit), cat, "{unit}");
    }
}

#[test]
fn the_parsers_read_what_the_system_prints() {
    let m = parse_meminfo("MemTotal:  1000 kB\nMemFree: 1 kB\nMemAvailable:  400 kB\nSwapTotal: 10 kB\nSwapFree: 4 kB\n");
    assert_eq!(
        m,
        MemInfo {
            total: 1_024_000,
            available: 409_600,
            swap_total: 10_240,
            swap_free: 4_096
        }
    );

    // user nice system idle iowait irq softirq steal
    assert_eq!(
        parse_proc_stat("cpu  10 0 5 80 5 0 0 0 0 0\ncpu0 3 1 4 1 5 9 2 6\n"),
        Some((15, 100))
    );
    assert_eq!(
        parse_loadavg("2.10 3.39 2.71 2/1719 1003973\n"),
        Some((2.10, 1719))
    );
    assert_eq!(
        parse_cpu_stat("usage_usec 90242915341\nuser_usec 1\n"),
        Some(90_242_915_341)
    );

    assert_eq!(
        parse_gpu("37, 50, 12.05, 24576\n"),
        GpuNow {
            util: Some(37.0),
            temp: Some(50.0),
            power_w: Some(12.05),
            unified: false
        }
    );
    assert_eq!(
        parse_gpu("[N/A], 50, [N/A], [N/A]\n"),
        GpuNow {
            util: None,
            temp: Some(50.0),
            power_w: None,
            unified: true
        },
        "[N/A] is None, never zero"
    );
    assert_eq!(
        parse_gpu_apps("101, 6081\n202, 170\nbad line\n"),
        vec![(101, 6081), (202, 170)]
    );

    assert_eq!(
        unit_of_cgroup(
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/llama-local.service\n"
        ),
        Some("llama-local.service".to_string())
    );
    assert_eq!(
        unit_of_cgroup("0::/user.slice/user-1000.slice/session-3.scope\n"),
        None
    );

    // Two GPUs: hottest wins, power sums, and unified only if both say so.
    let two = parse_gpu("10, 40, 5, [N/A]\n70, 60, 7, 24576\n");
    assert_eq!(
        (two.util, two.temp, two.power_w, two.unified),
        (Some(70.0), Some(60.0), Some(12.0), false)
    );
    assert!(parse_gpu("1, 2, 3, [N/A]\n4, 5, 6, [N/A]\n").unified);
    assert_eq!(parse_gpu(""), GpuNow::default());
}

#[test]
fn folding_keeps_categories_and_drops_names() {
    let units = vec![
        ("llama-local.service".to_string(), 100, 10, 2),
        ("mecha-serve.service".to_string(), 50, 5, 1),
        ("mecha-slack.service".to_string(), 25, 5, 1),
        ("somebody-elses.service".to_string(), 7, 1, 1),
    ];
    let gpu = vec![
        ("llama-local.service".to_string(), 6000),
        (String::new(), 40),
    ];
    let f = fold(&units, Some(&gpu));
    assert_eq!(f[&Category::ChatModel].mem_bytes, 100);
    assert_eq!(f[&Category::Mecha].mem_bytes, 75);
    assert_eq!(f[&Category::Mecha].tasks, 2);
    assert_eq!(f[&Category::Other].mem_bytes, 7);
    assert_eq!(f[&Category::ChatModel].gpu_mib, Some(6000));
    assert_eq!(
        f[&Category::Other].gpu_mib,
        Some(40),
        "an unknown process's GPU memory is Other's"
    );
    assert_eq!(
        f[&Category::Voice].gpu_mib,
        Some(0),
        "the GPU answered and voice held none"
    );

    let none = fold(&units, None);
    assert_eq!(
        none[&Category::Voice].gpu_mib,
        None,
        "no answer from the GPU is None, not zero"
    );
}

fn sample(min: u32, units: &[UnitCounters], busy: u64, total: u64) -> Sample {
    Sample {
        at: Utc.with_ymd_and_hms(2031, 4, 17, 9, min, 0).unwrap(),
        mem: MemInfo {
            total: 1_000_000,
            available: 400_000,
            swap_total: 0,
            swap_free: 0,
        },
        cpu_jiffies: Some((busy, total)),
        load1: Some(1.5),
        tasks_total: Some(100),
        gpu: GpuNow {
            util: Some(20.0),
            temp: Some(50.0),
            power_w: Some(12.0),
            unified: false,
        },
        disk: Some((300, 1000)),
        by_category: fold(units, Some(&[("llama-local.service".to_string(), 6000)])),
    }
}

#[test]
fn a_second_sample_yields_cpu_and_other_is_the_remainder() {
    let s = Scratch::new();
    let db = s.0.join("host.sqlite");
    let u1 = vec![
        ("llama-local.service".to_string(), 300_000, 0, 10),
        ("mecha-serve.service".to_string(), 100_000, 0, 5),
    ];
    let first = record(&db, &sample(0, &u1, 0, 0)).unwrap();
    assert_eq!(first.cpu_pct, None, "no previous sample, no rate");

    // One minute later the chat model used 60 CPU-seconds.
    let u2 = vec![
        ("llama-local.service".to_string(), 300_000, 60_000_000, 10),
        ("mecha-serve.service".to_string(), 100_000, 0, 5),
    ];
    record(&db, &sample(1, &u2, 500, 1000)).unwrap();

    let c = Connection::open(&db).unwrap();
    let ncpu = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
    let chat: f64 = c
        .query_row(
            "SELECT cpu_pct FROM category_minute WHERE at = '2031-04-17 09:01:00' AND category = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!((chat - 100.0 / ncpu).abs() < 1e-6, "{chat}");
    let (other_mem, other_tasks): (i64, i64) = c
        .query_row(
            "SELECT mem_bytes, tasks FROM category_minute WHERE at = '2031-04-17 09:01:00' AND category = 0",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    // used = 1_000_000 - 400_000; named = 400_000; tasks 100 - 15.
    assert_eq!((other_mem, other_tasks), (200_000, 85));
    let labels: Vec<(i64, String)> = c
        .prepare("SELECT id, label FROM categories ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(labels.len(), Category::ALL.len());
    assert_eq!(labels[4], (4, "voice".to_string()));
}

#[test]
fn a_completed_quarter_hour_rolls_up_and_old_rows_are_dropped() {
    let s = Scratch::new();
    let db = s.0.join("host.sqlite");
    let u = vec![("llama-local.service".to_string(), 300_000, 0, 10)];
    for m in [0, 5, 10, 15] {
        record(&db, &sample(m, &u, 0, 0)).unwrap();
    }
    let c = Connection::open(&db).unwrap();
    let n: i64 = c
        .query_row(
            "SELECT count(*) FROM system_15m WHERE at = '2031-04-17 09:00:00'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1, "the 09:00 bucket closed at 09:15 and was rolled up");

    // Eight days later the minute rows from the 17th are gone; the rollup stays.
    let mut late = sample(0, &u, 0, 0);
    late.at = Utc.with_ymd_and_hms(2031, 4, 25, 9, 0, 0).unwrap();
    record(&db, &late).unwrap();
    let old: i64 = c
        .query_row(
            "SELECT count(*) FROM system_minute WHERE at < '2031-04-18'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(old, 0);
    let kept: i64 = c
        .query_row(
            "SELECT count(*) FROM system_15m WHERE at = '2031-04-17 09:00:00'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kept, 1);
}

/// The owner's condition, measured on the artifact: whatever units were
/// running, the database file holds no unit name — not a mapped one, not an
/// unknown one — only numbers and the enum's own labels.
#[test]
fn no_unit_name_ever_reaches_the_database_file() {
    let s = Scratch::new();
    let db = s.0.join("host.sqlite");
    let secret = "zzprivatemodelnamezz";
    let units = vec![
        (format!("{secret}.service"), 5_000, 1, 1),
        ("llama-local.service".to_string(), 300_000, 1, 10),
        ("mecha-voice-worker.service".to_string(), 1_000, 1, 1),
    ];
    let mut s1 = sample(0, &units, 0, 0);
    s1.by_category = fold(&units, Some(&[(format!("{secret}.service"), 99)]));
    record(&db, &s1).unwrap();
    record(&db, &sample(1, &units, 10, 20)).unwrap();

    let mut bytes = std::fs::read(&db).unwrap();
    for side in ["-journal", "-wal"] {
        if let Ok(more) = std::fs::read(s.0.join(format!("host.sqlite{side}"))) {
            bytes.extend(more);
        }
    }
    let hay = String::from_utf8_lossy(&bytes);
    for name in [secret, "llama-local", "mecha-voice-worker", ".service"] {
        assert!(!hay.contains(name), "{name:?} reached the database file");
    }
    assert!(hay.contains("chat model"), "the labels come from the enum");
}

/// The board shipped under `scripts/hud/host/` validates as written, and
/// every one of its loaders runs — confined, as a refresh would — against a
/// database this sampler wrote, and passes its own declared shape.
#[test]
fn the_shipped_host_board_validates_and_its_loaders_read_the_sampler() {
    use crate::hud::runner::{run_sqlite, QUERY_TIMEOUT};
    use crate::hud::Installed;

    let board = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/hud/host");
    let installed = Installed::load_dir(&board, "host")
        .unwrap()
        .unwrap_or_else(|r| panic!("{r}"));
    assert_eq!(installed.loaders().len(), 5);

    let s = Scratch::new();
    let db = s.0.join("host.sqlite");
    let units = vec![
        ("llama-local.service".to_string(), 300_000, 0, 10),
        ("mecha-serve.service".to_string(), 100_000, 0, 5),
    ];
    // Recent samples on five-minute marks, so the 24-hour loaders find them.
    let base = Utc::now().duration_trunc(Duration::minutes(5)).unwrap() - Duration::minutes(30);
    for (i, step) in [0i64, 5, 10, 15, 20].into_iter().enumerate() {
        let mut smp = sample(0, &units, i as u64 * 100, i as u64 * 1000);
        smp.at = base + Duration::minutes(step);
        record(&db, &smp).unwrap();
    }
    // And the current minute, which the `now` loader's freshness bound needs.
    let mut current = sample(0, &units, 600, 6000);
    current.at = Utc::now();
    record(&db, &current).unwrap();
    for (name, loader) in installed.loaders() {
        assert_eq!(loader.source(), "host", "{name}");
        let fetched = run_sqlite(&db, loader.query(), loader.max_rows(), QUERY_TIMEOUT)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let rows = loader
            .shape(&fetched.columns, fetched.rows)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!rows.is_empty(), "{name} read nothing");
    }
}

/// On unified memory the chat model's GPU allocations count as its memory,
/// not as "other" — the bug the first run on the GB10 showed.
#[test]
fn on_unified_memory_a_categorys_gpu_memory_is_its_memory() {
    let s = Scratch::new();
    let db = s.0.join("host.sqlite");
    let units = vec![("llama-local.service".to_string(), 100_000, 0, 10)];
    let mut smp = sample(0, &units, 0, 0);
    smp.gpu.unified = true;
    smp.mem = MemInfo {
        total: 10_000_000_000,
        available: 3_000_000_000,
        swap_total: 0,
        swap_free: 0,
    };
    smp.by_category = fold(&units, Some(&[("llama-local.service".to_string(), 6000)]));
    record(&db, &smp).unwrap();
    let c = Connection::open(&db).unwrap();
    let mem = |cat: u8| -> i64 {
        c.query_row(
            "SELECT mem_bytes FROM category_minute WHERE category = ?1",
            [cat],
            |r| r.get(0),
        )
        .unwrap()
    };
    let gpu_bytes = 6000 * 1024 * 1024;
    assert_eq!(
        mem(1),
        100_000 + gpu_bytes,
        "chat model = cgroup + its GPU memory"
    );
    assert_eq!(
        mem(0),
        7_000_000_000 - (100_000 + gpu_bytes),
        "other is what is left"
    );
}

/// A stopped sampler reads as no headline figure, not as the machine now.
#[test]
fn the_now_loader_refuses_a_stale_sample() {
    use crate::hud::runner::{run_sqlite, QUERY_TIMEOUT};
    use crate::hud::Installed;
    let board = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/hud/host");
    let installed = Installed::load_dir(&board, "host").unwrap().unwrap();
    let now = &installed.loaders()["now"];
    let s = Scratch::new();
    let db = s.0.join("host.sqlite");
    let mut old = sample(0, &[], 0, 0);
    old.at = Utc::now() - Duration::minutes(30);
    record(&db, &old).unwrap();
    let fetched = run_sqlite(&db, now.query(), now.max_rows(), QUERY_TIMEOUT).unwrap();
    assert!(
        fetched.rows.is_empty(),
        "a 30-minute-old sample is not the machine now"
    );
}
