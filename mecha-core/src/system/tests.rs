use std::path::PathBuf;

use chrono::{TimeZone, Utc};
use rusqlite::Connection;

use super::*;

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "mecha-system-{}-{}",
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
        Some(MemInfo {
            total: 1_024_000,
            available: 409_600,
            swap_total: 10_240,
            swap_free: 4_096
        })
    );
    assert_eq!(
        parse_meminfo(""),
        None,
        "an unreadable /proc/meminfo is not a machine with no memory"
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
            unified: false,
            answered: true,
        }
    );
    assert_eq!(
        parse_gpu("[N/A], 50, [N/A], [N/A]\n"),
        GpuNow {
            util: None,
            temp: Some(50.0),
            power_w: None,
            unified: true,
            answered: true,
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
    assert_eq!(
        parse_gpu("1, 2, [N/A], 3\n4, 5, 6, 7\n").power_w,
        None,
        "a partial sum is not the machine's draw"
    );
    assert!(parse_gpu("1, 2, 3, [N/A]\n4, 5, 6, [N/A]\n").unified);
    assert_eq!(parse_gpu(""), GpuNow::default());
    assert!(!parse_gpu("").answered);

    assert_eq!(
        parse_cores("cpu  1 2 3 4 5 6 7 8\ncpu0 1\ncpu1 2\ncpu17 3\nintr 9\nctxt 4\n"),
        Some(3)
    );
    assert_eq!(
        parse_cores(""),
        None,
        "an unread /proc/stat has no cores, not zero"
    );
}

/// "No services ran" and "the services could not be read" are opposite
/// findings; the second refuses the sample instead of recording zeros.
#[test]
fn an_unreadable_cgroup_tree_is_an_error_not_an_idle_machine() {
    let s = Scratch::new();
    assert!(unit_counters(&s.0.join("no-such-slice")).is_err());
    assert_eq!(
        unit_counters(&s.0).unwrap(),
        Vec::new(),
        "an empty slice is empty"
    );
}

fn mem_at(db: &Path, at: &str) -> Vec<Option<i64>> {
    let c = Connection::open(db).unwrap();
    let rows = c
        .prepare("SELECT mem_bytes FROM category_minute WHERE at = ?1 ORDER BY category")
        .unwrap()
        .query_map([at], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    rows
}

/// One failed `nvidia-smi` must not flip what `mem_bytes` means for a
/// minute: unified memory is remembered, and with the per-process query
/// silent too, every category's memory is unknown rather than its cgroup
/// figure alone presented as the whole.
#[test]
fn a_silent_gpu_on_unified_memory_records_memory_as_unknown() {
    let s = Scratch::new();
    let db = s.0.join("series.sqlite");
    let units = vec![("llama-local.service".to_string(), 100_000, 0, 10)];
    let mut first = sample(0, &units, 0, 0);
    first.gpu.unified = true;
    record(&db, &first).unwrap();

    let mut silent = sample(1, &units, 0, 0);
    silent.gpu = GpuNow::default();
    silent.by_category = fold(&units, None);
    let recorded = record(&db, &silent).unwrap();
    assert!(
        recorded.by_category.iter().all(|c| c.mem_bytes.is_none()),
        "{recorded:?}"
    );
    assert_eq!(
        mem_at(&db, "2031-04-17 09:01:00"),
        vec![None; Category::ALL.len()]
    );

    // Answered unified, but the per-process query failed: still unknown.
    let mut half = sample(2, &units, 0, 0);
    half.gpu.unified = true;
    half.by_category = fold(&units, None);
    record(&db, &half).unwrap();
    assert_eq!(
        mem_at(&db, "2031-04-17 09:02:00"),
        vec![None; Category::ALL.len()]
    );

    // A discrete GPU that goes silent keeps measuring: cgroup memory is all
    // of a category's system memory there.
    let s2 = Scratch::new();
    let db2 = s2.0.join("series.sqlite");
    record(&db2, &sample(0, &units, 0, 0)).unwrap();
    let mut quiet = sample(1, &units, 0, 0);
    quiet.gpu = GpuNow::default();
    quiet.by_category = fold(&units, None);
    record(&db2, &quiet).unwrap();
    assert_eq!(mem_at(&db2, "2031-04-17 09:01:00")[1], Some(100_000));
}

/// A category number this build does not label reads as `unknown` in every
/// shipped loader, never as a dropped row.
#[test]
fn the_loaders_keep_a_row_whose_category_has_no_label() {
    use crate::hud::runner::{run_sqlite, QUERY_TIMEOUT};
    use crate::hud::Installed;
    let board = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/hud/host");
    let installed = Installed::load_dir(&board, "host").unwrap().unwrap();
    let s = Scratch::new();
    let db = s.0.join("series.sqlite");
    let mut now = sample(0, &[], 0, 0);
    // The by-category loaders read five-minute steps of the last day.
    now.at = Utc::now().duration_trunc(Duration::minutes(5)).unwrap();
    record(&db, &now).unwrap();
    let c = Connection::open(&db).unwrap();
    c.execute(
        "INSERT INTO category_minute (at, category, mem_bytes, cpu_pct, tasks, gpu_mib)
         SELECT at, 200, 1073741824, 1.0, 1, 1024 FROM system_minute",
        [],
    )
    .unwrap();
    drop(c);
    for name in ["memory_by_category", "cpu_by_category", "gpu_by_category"] {
        let loader = &installed.loaders()[name];
        let fetched = run_sqlite(&db, loader.query(), loader.max_rows(), QUERY_TIMEOUT).unwrap();
        let text = format!("{:?}", fetched.rows);
        assert!(text.contains("unknown"), "{name}: {text}");
    }
}

#[test]
fn last_written_reads_without_creating() {
    let s = Scratch::new();
    let db = s.0.join("series.sqlite");
    assert!(
        last_written(&db).is_err(),
        "no file is an error, not a fresh store"
    );
    assert!(!db.exists(), "the probe created the store");
    record(&db, &sample(7, &[], 0, 0)).unwrap();
    assert_eq!(
        last_written(&db).unwrap(),
        Some(Utc.with_ymd_and_hms(2031, 4, 17, 9, 7, 0).unwrap())
    );
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
        cores: Some(4),
        load1: Some(1.5),
        tasks_total: Some(100),
        gpu: GpuNow {
            util: Some(20.0),
            temp: Some(50.0),
            power_w: Some(12.0),
            unified: false,
            answered: true,
        },
        disk: Some((300, 1000)),
        by_category: fold(units, Some(&[("llama-local.service".to_string(), 6000)])),
        now: Vec::new(),
        counters: Counters::default(),
    }
}

#[test]
fn a_second_sample_yields_cpu_and_other_is_the_remainder() {
    let s = Scratch::new();
    let db = s.0.join("series.sqlite");
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
    // Four cores from /proc/stat, whatever this test process may use.
    let ncpu = 4.0;
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
    let db = s.0.join("series.sqlite");
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
    let db = s.0.join("series.sqlite");
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
        if let Ok(more) = std::fs::read(s.0.join(format!("series.sqlite{side}"))) {
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
    let db = s.0.join("series.sqlite");
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
        assert_eq!(loader.source(), "system", "{name}");
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
    let db = s.0.join("series.sqlite");
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
    let db = s.0.join("series.sqlite");
    let mut old = sample(0, &[], 0, 0);
    old.at = Utc::now() - Duration::minutes(30);
    record(&db, &old).unwrap();
    let fetched = run_sqlite(&db, now.query(), now.max_rows(), QUERY_TIMEOUT).unwrap();
    assert!(
        fetched.rows.is_empty(),
        "a 30-minute-old sample is not the machine now"
    );
}

/// A gap in sampling must not lose the bucket it straddled: minutes 09:00-09:09
/// recorded, a suspend, then 09:50 — the 09:00 bucket is still rolled up.
#[test]
fn a_gap_in_sampling_still_rolls_up_the_bucket_it_straddled() {
    let s = Scratch::new();
    let db = s.0.join("series.sqlite");
    let u = vec![("llama-local.service".to_string(), 300_000, 0, 10)];
    for m in [0, 5, 9] {
        record(&db, &sample(m, &u, 0, 0)).unwrap();
    }
    record(&db, &sample(50, &u, 0, 0)).unwrap();
    let c = Connection::open(&db).unwrap();
    let buckets: Vec<String> = c
        .prepare("SELECT at FROM system_15m ORDER BY at")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(
        buckets.contains(&"2031-04-17 09:00:00".to_string()),
        "the 09:00 bucket held real minutes: {buckets:?}"
    );
    assert!(
        !buckets.contains(&"2031-04-17 09:45:00".to_string()),
        "the current bucket is not closed yet: {buckets:?}"
    );
}

/// Both sides of the database wait for the other: a sample waits out a
/// reader holding the file, and a loader waits out a sample mid-commit.
#[test]
fn the_sampler_and_a_loader_wait_for_each_other() {
    use crate::hud::runner::{run_sqlite, QUERY_TIMEOUT};
    let s = Scratch::new();
    let db = s.0.join("series.sqlite");
    let u = vec![("llama-local.service".to_string(), 300_000, 0, 10)];
    record(&db, &sample(0, &u, 0, 0)).unwrap();

    // A reader holds a shared lock for a moment; the writer waits it out.
    let reader = Connection::open(&db).unwrap();
    reader
        .execute_batch("BEGIN; SELECT count(*) FROM system_minute;")
        .unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        reader.execute_batch("COMMIT").unwrap();
    });
    record(&db, &sample(1, &u, 0, 0)).expect("the sample waits for the reader");
    release.join().unwrap();

    // A writer holds the file exclusively for a moment; the loader waits.
    let writer = Connection::open(&db).unwrap();
    writer.execute_batch("BEGIN EXCLUSIVE;").unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        writer.execute_batch("COMMIT").unwrap();
    });
    let fetched = run_sqlite(&db, "SELECT count(*) FROM system_minute", 10, QUERY_TIMEOUT)
        .expect("the loader waits for the writer");
    release.join().unwrap();
    assert_eq!(fetched.rows[0][0], serde_json::json!(2));
}

// ---- S1: sources, measurements, rates ----

#[test]
fn the_new_parsers_read_what_the_system_prints() {
    use source::*;
    let p = parse_pressure(
        "some avg10=1.50 avg60=0.25 avg300=0.00 total=99\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=1\n",
    )
    .unwrap();
    assert_eq!(
        (p.avg10, p.avg60),
        (1.5, 0.25),
        "the `some` line, not `full`"
    );
    assert_eq!(parse_pressure(""), None);

    assert_eq!(
        parse_vmstat_oom_kill("nr_free_pages 3\noom_kill 7\n"),
        Some(7)
    );
    assert_eq!(parse_vmstat_oom_kill("nr_free_pages 3\n"), None);
    assert_eq!(parse_uptime("1258406.10 23617715.72\n"), Some(1258406.10));
    assert_eq!(parse_thermal("44800\n"), Some(44.8));
    assert_eq!(
        parse_thermal("-273000\n"),
        None,
        "nonsense is not a temperature"
    );
    assert_eq!(parse_thermal("999000\n"), None);

    let disk = parse_diskstats(
        " 259       0 nvme0n1 1 2 100 4 5 6 300 8 0 40 9\n 259       2 nvme0n1p2 1 2 10 4 5 6 30 8 0 7 9\n",
        259,
        2,
    )
    .unwrap();
    assert_eq!(
        (disk.read_bytes, disk.write_bytes, disk.io_ms),
        (10 * 512, 30 * 512, 7),
        "the device asked for, in 512-byte sectors"
    );
    assert_eq!(parse_diskstats("", 259, 2), None);

    let route = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\n\
        eth9\t00000000\t0101A8C0\t0003\t0\t0\t700\t00000000\n\
        wlan9\t00000000\t0132A8C0\t0003\t0\t0\t600\t00000000\n\
        down9\t00000000\t0132A8C0\t0002\t0\t0\t1\t00000000\n\
        br9\t000011AC\t00000000\t0001\t0\t0\t0\t0000FFFF\n";
    assert_eq!(
        parse_default_route(route).as_deref(),
        Some("wlan9"),
        "lowest metric among default routes that are up"
    );

    let link = parse_iw_link(
        "Connected to 00:11:22:33:44:55 (on wlan9)\n\tsignal: -47 dBm\n\ttx bitrate: 286.7 MBit/s HE-MCS 11\n",
    )
    .unwrap();
    assert_eq!(
        (link.signal_dbm, link.bitrate_mbit),
        (Some(-47.0), Some(286.7))
    );
    assert_eq!(parse_iw_link("Not connected.\n"), None);

    let ts = r#"{"BackendState":"Running","Peer":{"k1":{"HostName":"made-up-a","Online":true},"k2":{"HostName":"made-up-b","Online":false},"k3":{"Online":true}}}"#;
    assert_eq!(
        parse_tailscale_status(ts),
        Some(Tailnet {
            up: true,
            peers_online: 2
        })
    );
    assert_eq!(
        parse_tailscale_status(r#"{"BackendState":"Stopped"}"#),
        Some(Tailnet {
            up: false,
            peers_online: 0
        })
    );

    assert_eq!(
        parse_throttle("0x0000000000000001\n"),
        Some(false),
        "idle is not throttled"
    );
    assert_eq!(
        parse_throttle("0x0000000000000000\n0x0000000000000040\n"),
        Some(true)
    );
    assert_eq!(parse_throttle(""), None, "no answer is not 'not throttled'");
    assert_eq!(
        parse_failed_units("a.service loaded failed failed A\nb.service loaded failed failed B\n"),
        2
    );
    assert_eq!(parse_failed_units(""), 0);
}

/// The kernel log names the killed task and its pid on the same line; only
/// the constraint and the cgroup's unit are read, and the unit is folded
/// into a category before anything is stored.
#[test]
fn an_oom_kill_is_read_as_its_kind_and_its_unit_only() {
    let log = "\
oom-kill:constraint=CONSTRAINT_MEMCG,nodemask=(null),cpuset=user.slice,mems_allowed=0,oom_memcg=/user.slice/user-1000.slice/user@1000.service/app.slice/run-rabc.scope,task_memcg=/user.slice/user-1000.slice/user@1000.service/app.slice/run-rabc.scope,task=made-up-tool,pid=11,uid=1000
something else entirely
oom-kill:constraint=CONSTRAINT_NONE,nodemask=(null),cpuset=/,mems_allowed=0,global_oom,task_memcg=/user.slice/user-1000.slice/user@1000.service/app.slice/llama-local.service,task=made-up-server,pid=22,uid=1000
";
    let kills = source::parse_oom_kills(log);
    assert_eq!(
        kills,
        vec![
            source::OomKill {
                global: false,
                unit: None
            },
            source::OomKill {
                global: true,
                unit: Some("llama-local.service".into())
            },
        ]
    );
}

#[test]
fn a_bounded_command_is_killed_and_a_large_answer_does_not_stall_it() {
    use source::{run_bounded, Ran};
    use std::time::Duration;
    assert_eq!(
        run_bounded("mecha-no-such-program", &[], Duration::from_secs(1)),
        Ran::Missing,
        "not installed is a fact about the machine"
    );
    assert_eq!(
        run_bounded("sleep", &["5"], Duration::from_millis(200)),
        Ran::TimedOut
    );
    assert_eq!(
        run_bounded("false", &[], Duration::from_secs(5)),
        Ran::Failed
    );
    // Far beyond a pipe's 64 KiB: drained while running, so it finishes.
    match run_bounded(
        "head",
        &["-c", "300000", "/dev/zero"],
        Duration::from_secs(5),
    ) {
        Ran::Out(t) => assert_eq!(t.len(), 300_000),
        other => panic!("{other:?}"),
    }
}

/// A measurement's number is a wire format: the series stores readings
/// under it for a week.
#[test]
fn measurement_numbers_and_names_are_pinned_and_unique() {
    let pinned: &[(u16, &str)] = &[
        (1, "memory.total"),
        (2, "memory.available"),
        (4, "memory.oom_kills"),
        (5, "memory.oom_kills.global"),
        (6, "memory.oom_kills.capped"),
        (10, "cpu.busy"),
        (22, "pressure.memory.avg10"),
        (32, "disk.read_rate"),
        (43, "gpu.unified"),
        (52, "network.uplink.kind"),
        (61, "services.failed"),
        (104, "category.oom_kills"),
    ];
    for (n, name) in pinned {
        let m = Measurement::from_u16(*n).unwrap_or_else(|| panic!("{n} is gone"));
        assert_eq!(m.name(), *name);
        assert_eq!(m as u16, *n);
    }
    assert_eq!(
        Measurement::from_u16(9999),
        None,
        "unknown is None, never a neighbour"
    );
    let mut names: Vec<&str> = Measurement::ALL.iter().map(|m| m.name()).collect();
    let n = names.len();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), n, "every name is unique");
    for m in Measurement::ALL {
        assert_eq!(Measurement::from_u16(*m as u16), Some(*m));
    }
}

#[test]
fn a_query_matches_a_name_or_a_whole_group_never_a_fragment() {
    assert_eq!(
        Measurement::matching("memory.available"),
        vec![Measurement::MemoryAvailable]
    );
    assert_eq!(Measurement::matching("gpu").len(), 5);
    assert_eq!(
        Measurement::matching("pressure.memory"),
        vec![
            Measurement::PressureMemoryAvg10,
            Measurement::PressureMemoryAvg60
        ]
    );
    assert!(
        Measurement::matching("gp").is_empty(),
        "a fragment is not a group"
    );
    assert!(Measurement::matching("nonsense").is_empty());
}

fn reading_at(db: &Path, at: &str, m: Measurement) -> Option<Option<f64>> {
    let c = Connection::open(db).unwrap();
    c.query_row(
        "SELECT value FROM reading_minute WHERE at = ?1 AND measurement = ?2",
        rusqlite::params![at, m as u16],
        |r| r.get(0),
    )
    .optional()
    .unwrap()
}

/// Observed writes its value, Unread writes NULL, NotHere writes no row.
#[test]
fn a_reading_is_stored_as_a_value_a_null_or_no_row() {
    let s = Scratch::new();
    let db = s.0.join("series.sqlite");
    let mut smp = sample(0, &[], 0, 0);
    smp.now = vec![
        (
            Measurement::MemoryAvailable,
            Reading::Observed { value: 4.0 },
        ),
        (
            Measurement::GpuThrottled,
            Reading::Unread {
                why: "nvidia-smi failed".into(),
            },
        ),
        (Measurement::NetworkWifiSignal, Reading::NotHere),
    ];
    record(&db, &smp).unwrap();
    let at = "2031-04-17 09:00:00";
    assert_eq!(
        reading_at(&db, at, Measurement::MemoryAvailable),
        Some(Some(4.0))
    );
    assert_eq!(reading_at(&db, at, Measurement::GpuThrottled), Some(None));
    assert_eq!(reading_at(&db, at, Measurement::NetworkWifiSignal), None);
}

fn with_counters(min: u32, disk_read: u64, dev: u64, rx: u64, ifindex: u64, oom: u64) -> Sample {
    let mut s = sample(min, &[], 0, 0);
    let at = s.at;
    s.counters = Counters {
        oom_kills: Ok(oom),
        oom_read_at: at,
        oom_log: None,
        disk: Ok((
            dev,
            source::DiskCounters {
                read_bytes: disk_read,
                write_bytes: 0,
                io_ms: 0,
            },
        )),
        uplink: Ok(NetCounters { ifindex, rx, tx: 0 }),
        tailnet: Err(Reading::NotHere),
    };
    s
}

/// A rate is a difference over time — never across a reset or a change of
/// the device or interface holding the role.
#[test]
fn a_rate_is_drawn_between_samples_and_never_across_a_reset_or_a_switch() {
    let s = Scratch::new();
    let db = s.0.join("series.sqlite");
    record(&db, &with_counters(0, 1_000, 7, 500, 3, 0)).unwrap();
    assert_eq!(
        reading_at(&db, "2031-04-17 09:00:00", Measurement::DiskReadRate),
        Some(None),
        "the first sample has nothing to differ from"
    );

    record(&db, &with_counters(1, 61_000, 7, 6_500, 3, 0)).unwrap();
    let at = "2031-04-17 09:01:00";
    assert_eq!(
        reading_at(&db, at, Measurement::DiskReadRate),
        Some(Some(1_000.0))
    );
    assert_eq!(
        reading_at(&db, at, Measurement::NetworkUplinkRxRate),
        Some(Some(100.0))
    );
    assert_eq!(
        reading_at(&db, at, Measurement::NetworkTailnetRxRate),
        None,
        "no tailnet interface writes no row"
    );

    // A reboot: the disk counter fell. Another interface took the route.
    record(&db, &with_counters(2, 5, 7, 9_999_999, 4, 0)).unwrap();
    let at = "2031-04-17 09:02:00";
    assert_eq!(reading_at(&db, at, Measurement::DiskReadRate), Some(None));
    assert_eq!(
        reading_at(&db, at, Measurement::NetworkUplinkRxRate),
        Some(None)
    );

    // And from there it measures again.
    record(&db, &with_counters(3, 60_005, 7, 9_999_999 + 60, 4, 0)).unwrap();
    let at = "2031-04-17 09:03:00";
    assert_eq!(
        reading_at(&db, at, Measurement::DiskReadRate),
        Some(Some(1_000.0))
    );
    assert_eq!(
        reading_at(&db, at, Measurement::NetworkUplinkRxRate),
        Some(Some(1.0))
    );
}

fn category_oom(db: &Path, at: &str, cat: Category) -> Option<f64> {
    let c = Connection::open(db).unwrap();
    c.query_row(
        "SELECT value FROM category_reading_minute WHERE at = ?1 AND measurement = ?2 AND category = ?3",
        rusqlite::params![at, Measurement::CategoryOomKills as u16, cat as u8],
        |r| r.get(0),
    )
    .unwrap()
}

/// The kernel's counter says how many; the log says which kind and whose,
/// and is believed only when it accounts for exactly that many.
#[test]
fn the_oom_split_is_recorded_only_when_the_log_accounts_for_every_kill() {
    let s = Scratch::new();
    let db = s.0.join("series.sqlite");
    record(&db, &with_counters(0, 0, 7, 0, 3, 100)).unwrap();

    // Two kills; the log names both: one capped scope, one global chat model.
    let mut two = with_counters(1, 0, 7, 0, 3, 102);
    two.counters.oom_log = Some(vec![(false, Category::Other), (true, Category::ChatModel)]);
    record(&db, &two).unwrap();
    let at = "2031-04-17 09:01:00";
    assert_eq!(
        reading_at(&db, at, Measurement::MemoryOomKills),
        Some(Some(2.0))
    );
    assert_eq!(
        reading_at(&db, at, Measurement::MemoryOomKillsGlobal),
        Some(Some(1.0))
    );
    assert_eq!(
        reading_at(&db, at, Measurement::MemoryOomKillsCapped),
        Some(Some(1.0))
    );
    assert_eq!(category_oom(&db, at, Category::ChatModel), Some(1.0));
    assert_eq!(category_oom(&db, at, Category::Voice), Some(0.0));

    // Three kills, but the log saw one: the total stands, the split is unknown.
    let mut short = with_counters(2, 0, 7, 0, 3, 105);
    short.counters.oom_log = Some(vec![(true, Category::ChatModel)]);
    record(&db, &short).unwrap();
    let at = "2031-04-17 09:02:00";
    assert_eq!(
        reading_at(&db, at, Measurement::MemoryOomKills),
        Some(Some(3.0))
    );
    assert_eq!(
        reading_at(&db, at, Measurement::MemoryOomKillsGlobal),
        Some(None)
    );
    assert_eq!(category_oom(&db, at, Category::ChatModel), None);

    // No kills: zero of each kind, whatever the log window happened to hold.
    let mut none = with_counters(3, 0, 7, 0, 3, 105);
    none.counters.oom_log = Some(vec![(true, Category::Voice)]);
    record(&db, &none).unwrap();
    let at = "2031-04-17 09:03:00";
    assert_eq!(
        reading_at(&db, at, Measurement::MemoryOomKillsGlobal),
        Some(Some(0.0))
    );
    assert_eq!(category_oom(&db, at, Category::Voice), Some(0.0));
}

/// Without the memory controller every unit would read zero and `Other`
/// would absorb the machine.
#[test]
fn a_slice_whose_units_report_no_memory_is_refused() {
    let s = Scratch::new();
    let unit = s.0.join("made-up.service");
    std::fs::create_dir_all(&unit).unwrap();
    std::fs::write(unit.join("pids.current"), "3\n").unwrap();
    let err = unit_counters(&s.0).unwrap_err();
    assert!(format!("{err:#}").contains("memory.current"), "{err:#}");
    std::fs::write(unit.join("memory.current"), "4096\n").unwrap();
    assert_eq!(unit_counters(&s.0).unwrap()[0].1, 4096);
}

/// A rate is probed from the series' newest minute while it is fresh; a
/// stale series is unknown with a reason, never its last value.
#[test]
fn a_rate_probe_reads_the_fresh_series_and_refuses_a_stale_one() {
    let s = Scratch::new();
    let db = s.0.join("series.sqlite");
    let reader = Reader::new();
    let m = Measurement::DiskReadRate;
    assert!(matches!(
        probe(&reader, &db, m, Utc::now()),
        Probed::One(Reading::Unread { .. })
    ));
    record(&db, &with_counters(0, 1_000, 7, 0, 3, 0)).unwrap();
    record(&db, &with_counters(1, 61_000, 7, 0, 3, 0)).unwrap();
    let then = Utc.with_ymd_and_hms(2031, 4, 17, 9, 2, 0).unwrap();
    assert_eq!(
        probe(&reader, &db, m, then),
        Probed::One(Reading::Observed { value: 1_000.0 })
    );
    let later = Utc.with_ymd_and_hms(2031, 4, 17, 9, 30, 0).unwrap();
    match probe(&reader, &db, m, later) {
        Probed::One(Reading::Unread { why }) => assert!(why.contains("newest minute"), "{why}"),
        other => panic!("{other:?}"),
    }
    match probe(&reader, &db, Measurement::CategoryMemory, then) {
        Probed::ByCategory(rows) => assert_eq!(rows.len(), Category::ALL.len()),
        other => panic!("{other:?}"),
    }
}

/// The sample's commands share one deadline, and it sits inside the unit's
/// start timeout with room left for the store write: a bound per command
/// added up past the timeout, and systemd killed the sample before it wrote.
#[test]
fn the_sample_budget_fits_inside_the_units_start_timeout() {
    let unit = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/mecha-system-sample.service");
    let text = std::fs::read_to_string(&unit).unwrap();
    let secs: u64 = text
        .lines()
        .find_map(|l| l.strip_prefix("TimeoutStartSec="))
        .and_then(|v| v.trim().strip_suffix('s'))
        .and_then(|v| v.parse().ok())
        .expect("the unit states TimeoutStartSec in seconds");
    assert!(
        SAMPLE_BUDGET + std::time::Duration::from_secs(5) <= std::time::Duration::from_secs(secs),
        "budget {SAMPLE_BUDGET:?} leaves under 5 s of the unit's {secs} s"
    );
}

/// Once the budget is spent, a reading that needs a command is unknown, and
/// the command is never started.
#[test]
fn a_spent_budget_runs_nothing_and_reads_as_unknown() {
    let reader = Reader::until(std::time::Instant::now());
    assert_eq!(reader.budget(), std::time::Duration::ZERO);
    let started = std::time::Instant::now();
    match reader.now(Measurement::ServicesFailed) {
        Reading::Unread { why } => assert!(why.contains("in time"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert!(started.elapsed() < std::time::Duration::from_millis(500));
    assert_eq!(
        source::run_bounded("sleep", &["5"], std::time::Duration::ZERO),
        source::Ran::TimedOut
    );
}

#[test]
fn meminfo_without_mem_available_is_unknown_not_empty() {
    assert_eq!(
        parse_meminfo("MemTotal: 1000 kB\nMemFree: 10 kB\n"),
        None,
        "no MemAvailable is not a machine with nothing left"
    );
}

/// Every `Now` measurement has a reader in `Reader::now`: one listed but not
/// wired reads `NOT_WIRED` here instead of a NULL the series would keep.
#[test]
fn every_now_measurement_is_wired_to_a_reader() {
    // A spent budget: no command runs, and every reading still answers.
    let reader = Reader::until(std::time::Instant::now());
    for m in Measurement::ALL
        .iter()
        .filter(|m| m.how() == measure::How::Now)
    {
        if let Reading::Unread { why } = reader.now(*m) {
            assert_ne!(why, measure::NOT_WIRED, "{} has no reader", m.name());
        }
    }
}

/// Every `ByCategory` measurement has a column or table in the series'
/// read-back: one added without it must not come back as a neighbour's
/// values under its own name.
#[test]
fn every_category_measurement_reads_back_its_own_column() {
    let s = Scratch::new();
    let db = s.0.join("series.sqlite");
    record(&db, &with_counters(0, 0, 7, 0, 3, 0)).unwrap();
    let then = Utc.with_ymd_and_hms(2031, 4, 17, 9, 1, 0).unwrap();
    let reader = Reader::new();
    for m in Measurement::ALL
        .iter()
        .filter(|m| m.how() == measure::How::ByCategory)
    {
        match probe(&reader, &db, *m, then) {
            Probed::ByCategory(rows) => assert_eq!(rows.len(), Category::ALL.len()),
            other => panic!("{} has no column: {other:?}", m.name()),
        }
    }
}
