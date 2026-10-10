//! `mecha system`: the state of the machine mecha runs on, read in one place
//! (`docs/SYSTEM-STATE-DESIGN.md`). This is its first step (S0): the
//! per-minute sampler and the series it writes, moved here from the HUD,
//! which is now one reader of it among several. The probe API and the named
//! measurements arrive in S1.
//!
//! What it records today is the host's load, by category: memory, CPU, tasks
//! and GPU memory, broken down by what *kind* of work it is, and nothing
//! finer (`LIVE-DASHBOARD-DESIGN.md` §11, owner ruling R13).
//!
//! **Privacy by construction.** The owner's condition is absolute: no detail
//! about any process beyond a generic category. So it is not enforced by a
//! loader, a spec or review — it is enforced by what is ever written down. The
//! sampler aggregates at the moment of sampling into rows that hold only
//! numbers and a [`Category`]; a unit name, a process name, a command line, a
//! pid, a path or a model name never reaches the store, so nothing downstream
//! can leak one. A unit this table does not know is [`Category::Other`],
//! summed and never itemised — a new service shows up as a bigger `Other`,
//! which is the right failure. The test that pins it greps the database file
//! itself for a unit name and expects nothing.
//!
//! **Measured from counters, not by inspecting processes.** Memory, CPU and
//! task counts per unit come from the user manager's cgroups
//! (`memory.current`, `cpu.stat`, `pids.current`); GPU memory per process
//! from `nvidia-smi`, whose pid is used in memory to find its unit and then
//! dropped. On the GB10 the GPU has no memory of its own to total — it shares
//! the system pool — so GPU memory is a per-category share and `[N/A]` is
//! `None`, never zero.
//!
//! **On unified memory, a category's memory includes its GPU memory.** A CUDA
//! allocation comes out of system RAM there but is not charged to the unit's
//! cgroup, so counted by cgroup alone the chat model's weights would land in
//! `Other` — measured on this machine: 12 GiB "chat model" beside 73 GiB
//! "other", most of which was the model. When `nvidia-smi` reports no memory
//! total of its own (`[N/A]`, which is how unified memory shows itself), each
//! category's memory is cgroup memory plus GPU memory; on a discrete GPU the
//! two stay apart. It is an approximation either way — a model file's page
//! cache can be counted on both sides — so `Other` is a remainder clamped at
//! zero, never a negative number.
//!
//! **The discriminants are a wire format.** Rollups keep ninety days of a
//! category as its number, so each variant's number is fixed, never reused,
//! and an unknown one reads back as `None`. The `categories` table is
//! rewritten from the enum on every sample, so a loader joins it for a label
//! and no name mapping ever lives in a model-drafted query.
//!
//! Parsing is pure (functions of the text a file or `nvidia-smi` printed), so
//! every number here is testable without the machine; [`collect`] is the only
//! part that reads the system.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, DurationRound, Utc};
use rusqlite::{params, Connection, OptionalExtension};

pub mod measure;
pub mod source;

pub use measure::{Measurement, Reader, Reading};
pub use source::{
    parse_cores, parse_cpu_stat, parse_gpu, parse_gpu_apps, parse_loadavg, parse_meminfo,
    parse_proc_stat, unit_of_cgroup, GpuNow, MemInfo,
};

/// What kind of work a unit does. Numbers are fixed: never reorder, never
/// reuse one; a new category takes the next free number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Category {
    Other = 0,
    ChatModel = 1,
    Embeddings = 2,
    Ocr = 3,
    Voice = 4,
    ImageGen = 5,
    Mecha = 6,
}

impl Category {
    pub const ALL: [Category; 7] = [
        Category::Other,
        Category::ChatModel,
        Category::Embeddings,
        Category::Ocr,
        Category::Voice,
        Category::ImageGen,
        Category::Mecha,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Other => "other",
            Category::ChatModel => "chat model",
            Category::Embeddings => "embeddings",
            Category::Ocr => "ocr",
            Category::Voice => "voice",
            Category::ImageGen => "image generation",
            Category::Mecha => "mecha",
        }
    }

    /// A stored number back to a category; one this build does not know is
    /// `None`, never a neighbour. Loaders read through SQL rather than this
    /// function, so the same rule is kept there by the `categories` table and
    /// a `LEFT JOIN … coalesce(label, 'unknown')` in every shipped loader.
    pub fn from_u8(n: u8) -> Option<Category> {
        Category::ALL.into_iter().find(|c| *c as u8 == n)
    }
}

/// Which category a systemd unit's work belongs to. mecha's own units, by
/// name; everything else is [`Category::Other`].
pub fn category_of(unit: &str) -> Category {
    let name = unit.strip_suffix(".service").unwrap_or(unit);
    match name {
        "llama-local" => Category::ChatModel,
        "llama-embed" | "llama-embed-proxy" => Category::Embeddings,
        "llama-ocr" | "llama-ocr-proxy" => Category::Ocr,
        "comfyui" | "mecha-comfyui-idle-reset" => Category::ImageGen,
        "mecha-parakeet" => Category::Voice,
        n if n.starts_with("mecha-breeze-") || n.starts_with("mecha-voice-") => Category::Voice,
        n if n.starts_with("mecha-") => Category::Mecha,
        _ => Category::Other,
    }
}

// ---- one sample ----

/// Per category, what one sample measured. Numbers only.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CategoryLoad {
    pub mem_bytes: u64,
    /// Cumulative CPU time (cgroup `usage_usec`); a percentage needs the
    /// previous sample, which the store keeps.
    pub cpu_usec: u64,
    pub tasks: u64,
    /// `None` when no process of this category held GPU memory and the GPU
    /// did not answer; `Some(0)` when it answered and none did.
    pub gpu_mib: Option<u64>,
}

/// One sample of the whole machine. Numbers and categories only — this is
/// the type the privacy property lives in.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub at: DateTime<Utc>,
    pub mem: MemInfo,
    pub cpu_jiffies: Option<(u64, u64)>,
    /// The machine's cores, from the same `/proc/stat` (`parse_cores`).
    pub cores: Option<u64>,
    pub load1: Option<f64>,
    pub tasks_total: Option<u64>,
    pub gpu: GpuNow,
    pub disk: Option<(u64, u64)>,
    pub by_category: BTreeMap<Category, CategoryLoad>,
    /// Every `Now` measurement, read once this minute.
    pub now: Vec<(Measurement, Reading)>,
    /// The cumulative counters the `Rate` measurements are differences of.
    pub counters: Counters,
}

/// Cumulative counters, as read this minute. `Err(Reading::NotHere)` is a
/// counter this machine does not have (no tailnet interface), which records
/// no row; any other `Err` records the rate as unknown.
#[derive(Debug, Clone, PartialEq)]
pub struct Counters {
    /// `/proc/vmstat` `oom_kill`, since boot.
    pub oom_kills: Result<u64, Reading>,
    /// The kernel log's OOM kills since the previous sample, already folded:
    /// whether the machine ran out, and the victim's category. `None` when
    /// there was no previous sample to start the window at, or the log could
    /// not be read.
    pub oom_log: Option<Vec<(bool, Category)>>,
    /// The disk holding `/`: its device number and counters.
    pub disk: Result<(u64, source::DiskCounters), Reading>,
    pub uplink: Result<NetCounters, Reading>,
    pub tailnet: Result<NetCounters, Reading>,
}

impl Default for Counters {
    fn default() -> Counters {
        Counters {
            oom_kills: Err(Reading::NotHere),
            oom_log: None,
            disk: Err(Reading::NotHere),
            uplink: Err(Reading::NotHere),
            tailnet: Err(Reading::NotHere),
        }
    }
}

/// An interface's byte counters, keyed by its index rather than its name:
/// if a different interface takes the role, the index changes and no rate
/// is drawn across the switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetCounters {
    pub ifindex: u64,
    pub rx: u64,
    pub tx: u64,
}

/// Each unit's counters, as read from its cgroup: (unit, memory bytes, CPU
/// µs, tasks). The unit name lives only here, in memory, and is folded into
/// a category by [`fold`].
pub type UnitCounters = (String, u64, u64, u64);

/// Fold per-unit counters and per-process GPU memory into per-category
/// load. This is where names stop: nothing returned carries one.
pub fn fold(
    units: &[UnitCounters],
    gpu_apps: Option<&[(String, u64)]>,
) -> BTreeMap<Category, CategoryLoad> {
    let mut out: BTreeMap<Category, CategoryLoad> = Category::ALL
        .into_iter()
        .map(|c| (c, CategoryLoad::default()))
        .collect();
    for (unit, mem, cpu, tasks) in units {
        let load = out.entry(category_of(unit)).or_default();
        load.mem_bytes += mem;
        load.cpu_usec += cpu;
        load.tasks += tasks;
    }
    if let Some(apps) = gpu_apps {
        for load in out.values_mut() {
            load.gpu_mib = Some(0);
        }
        for (unit, mib) in apps {
            let load = out.entry(category_of(unit)).or_default();
            *load.gpu_mib.get_or_insert(0) += mib;
        }
    }
    out
}

/// Read the machine now. The one function here that touches the system.
///
/// **Refuses** rather than records when the two readings everything else is
/// measured against cannot be had: `/proc/meminfo` (every memory figure) and
/// the user manager's cgroups (every category). Recorded as zeros they would
/// draw an idle machine — `Other` is a remainder, so it would absorb the
/// whole box — and nothing would say so. Refused, the sample unit fails, the
/// `now` loader empties, and the doctor reports a sampler that stopped writing.
pub fn collect(at: DateTime<Utc>, since: Option<DateTime<Utc>>) -> Result<Sample> {
    let reader = Reader::new();
    let mem = reader.meminfo().map_err(|r| {
        anyhow::anyhow!(
            "/proc/meminfo could not be read ({}); no sample recorded",
            why(&r)
        )
    })?;
    let stat = reader.stat().unwrap_or_default();
    let cpu_jiffies = parse_proc_stat(&stat);
    let cores = parse_cores(&stat);
    let (load1, tasks_total) = reader.loadavg().ok().unzip();

    let units = unit_counters(&user_app_slice())?;
    let gpu = reader.gpu().unwrap_or_default();
    let gpu_apps = source::nvidia_smi("--query-compute-apps=pid,used_memory")
        .out()
        .map(|t| {
            parse_gpu_apps(&t)
                .into_iter()
                .map(|(pid, mib)| {
                    // The pid finds its unit and is dropped here.
                    let unit = std::fs::read_to_string(format!("/proc/{pid}/cgroup"))
                        .ok()
                        .and_then(|c| unit_of_cgroup(&c))
                        .unwrap_or_default();
                    (unit, mib)
                })
                .collect::<Vec<_>>()
        });

    let now = Measurement::ALL
        .iter()
        .filter(|m| m.how() == measure::How::Now)
        .map(|&m| (m, reader.now(m)))
        .collect();

    Ok(Sample {
        at,
        mem,
        cpu_jiffies,
        cores,
        load1,
        tasks_total,
        gpu,
        disk: reader.disk(),
        by_category: fold(&units, gpu_apps.as_deref()),
        now,
        counters: read_counters(&reader, at, since),
    })
}

fn why(r: &Reading) -> String {
    match r {
        Reading::Unread { why } => why.clone(),
        Reading::NotHere => "not on this machine".into(),
        Reading::Observed { .. } => "read".into(),
    }
}

fn read_counters(reader: &Reader, at: DateTime<Utc>, since: Option<DateTime<Utc>>) -> Counters {
    let text = |path: &str| -> Result<String, Reading> {
        std::fs::read_to_string(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Reading::NotHere,
            _ => Reading::Unread {
                why: format!("{path}: {e}"),
            },
        })
    };
    let unread = |why: &str| Reading::Unread { why: why.into() };
    let oom_kills = text("/proc/vmstat").and_then(|t| {
        source::parse_vmstat_oom_kill(&t).ok_or_else(|| unread("/proc/vmstat has no oom_kill"))
    });
    // The window starts at the previous sample, so each kill is counted
    // once; the first sample has no window, and nothing to compare with.
    let oom_log = since.and_then(|since| {
        let ran = source::run_bounded(
            "journalctl",
            &[
                "-k",
                "-q",
                "--no-pager",
                "-o",
                "cat",
                &format!("--since=@{}", since.timestamp()),
                &format!("--until=@{}", at.timestamp()),
            ],
            source::FORK_TIMEOUT,
        );
        ran.out().map(|t| {
            source::parse_oom_kills(&t)
                .into_iter()
                .map(|k| {
                    (
                        k.global,
                        k.unit.as_deref().map_or(Category::Other, category_of),
                    )
                })
                .collect()
        })
    });
    let disk = root_device().and_then(|(dev, major, minor)| {
        let t = text("/proc/diskstats")?;
        source::parse_diskstats(&t, major, minor)
            .map(|c| (dev, c))
            .ok_or_else(|| unread("the disk holding / is not in /proc/diskstats"))
    });
    let uplink = match reader.uplink() {
        Some(iface) => net_counters(&iface),
        None => Err(unread("no default route")),
    };
    let tailnet = std::fs::read_dir("/sys/class/net")
        .ok()
        .and_then(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .find(|n| n.starts_with("tailscale"))
        })
        .map_or(Err(Reading::NotHere), |iface| net_counters(&iface));
    Counters {
        oom_kills,
        oom_log,
        disk,
        uplink,
        tailnet,
    }
}

/// The device number of the filesystem holding `/`, and its major and minor.
fn root_device() -> Result<(u64, u64, u64), Reading> {
    use std::os::unix::fs::MetadataExt;
    let dev = std::fs::metadata("/")
        .map_err(|e| Reading::Unread {
            why: format!("/: {e}"),
        })?
        .dev();
    let major = ((dev >> 8) & 0xfff) | ((dev >> 32) & !0xfff);
    let minor = (dev & 0xff) | ((dev >> 12) & !0xff);
    // Major 0 is a filesystem with no block device (overlay, tmpfs, btrfs
    // subvolumes): there is no disk to count.
    if major == 0 {
        return Err(Reading::NotHere);
    }
    Ok((dev, major, minor))
}

/// An interface's counters. Its name is used to find them and dropped.
fn net_counters(iface: &str) -> Result<NetCounters, Reading> {
    let num = |file: &str| -> Result<u64, Reading> {
        let path = format!("/sys/class/net/{iface}/{file}");
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| t.trim().parse().ok())
            .ok_or_else(|| Reading::Unread {
                why: format!("an interface counter ({file}) could not be read"),
            })
    };
    Ok(NetCounters {
        ifindex: num("ifindex")?,
        rx: num("statistics/rx_bytes")?,
        tx: num("statistics/tx_bytes")?,
    })
}

/// The user manager's `app.slice`, where systemd puts user services.
fn user_app_slice() -> PathBuf {
    // SAFETY: getuid cannot fail and has no preconditions.
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!(
        "/sys/fs/cgroup/user.slice/user-{uid}.slice/user@{uid}.service/app.slice"
    ))
}

/// Each user service's counters. An unreadable slice is an error, not an
/// empty machine — "no services ran" and "the services could not be read"
/// are opposite findings.
pub(crate) fn unit_counters(slice: &Path) -> Result<Vec<UnitCounters>> {
    let entries = std::fs::read_dir(slice).with_context(|| {
        format!(
            "the user manager's cgroups at {} could not be read; no sample recorded",
            slice.display()
        )
    })?;
    let read = |p: PathBuf| {
        std::fs::read_to_string(p)
            .ok()
            .and_then(|t| t.trim().parse::<u64>().ok())
    };
    let num = |p: PathBuf| read(p).unwrap_or(0);
    let mut services = 0usize;
    let mut with_memory = 0usize;
    let units = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            if !name.ends_with(".service") {
                return None;
            }
            let dir = e.path();
            services += 1;
            let mem = read(dir.join("memory.current"));
            with_memory += usize::from(mem.is_some());
            let cpu = std::fs::read_to_string(dir.join("cpu.stat"))
                .ok()
                .and_then(|t| parse_cpu_stat(&t))
                .unwrap_or(0);
            Some((name, mem.unwrap_or(0), cpu, num(dir.join("pids.current"))))
        })
        .collect::<Vec<_>>();
    // `memory.current` exists only where the memory controller is enabled for
    // the slice. Without it every unit would read zero and `Other` — the
    // remainder — would draw the whole machine: refuse, as for an unreadable
    // slice.
    if services > 0 && with_memory == 0 {
        anyhow::bail!(
            "no user service under {} reports memory.current (is memory accounting \
             enabled for the user manager?); no sample recorded",
            slice.display()
        );
    }
    Ok(units)
}

// ---- the store ----

const MINUTE_KEEP_DAYS: i64 = 7;
const ROLLUP_KEEP_DAYS: i64 = 90;

/// `~/.mecha/system/series.sqlite`.
pub fn db_path() -> Result<PathBuf> {
    Ok(dir()?.join("series.sqlite"))
}

/// `~/.mecha/system/` — the series and, from S1, nothing a model writes.
pub fn dir() -> Result<PathBuf> {
    Ok(crate::work::mecha_home()?.join("system"))
}

/// When the sampler last wrote a minute, read without creating or changing
/// anything — the doctor's question. `Ok(None)`: the store exists and holds
/// no minute yet.
pub fn last_written(path: &Path) -> Result<Option<DateTime<Utc>>> {
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let at: Option<String> =
        conn.query_row("SELECT max(at) FROM system_minute", [], |r| r.get(0))?;
    at.map(|at| {
        chrono::NaiveDateTime::parse_from_str(&at, "%Y-%m-%d %H:%M:%S")
            .map(|t| t.and_utc())
            .with_context(|| format!("system_minute holds an unreadable time {at:?}"))
    })
    .transpose()
}

fn ts(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%d %H:%M:%S").to_string()
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS categories (id INTEGER PRIMARY KEY, label TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS category_minute (
  at TEXT NOT NULL, category INTEGER NOT NULL,
  mem_bytes INTEGER, cpu_pct REAL, tasks INTEGER NOT NULL, gpu_mib INTEGER,
  PRIMARY KEY (at, category));
CREATE TABLE IF NOT EXISTS system_minute (
  at TEXT PRIMARY KEY, mem_total INTEGER, mem_available INTEGER, swap_used INTEGER,
  cpu_pct REAL, load1 REAL, tasks INTEGER,
  gpu_util REAL, gpu_temp REAL, gpu_power_w REAL, disk_used INTEGER, disk_total INTEGER);
CREATE TABLE IF NOT EXISTS category_15m (
  at TEXT NOT NULL, category INTEGER NOT NULL,
  mem_bytes INTEGER, cpu_pct REAL, tasks INTEGER, gpu_mib INTEGER,
  PRIMARY KEY (at, category));
CREATE TABLE IF NOT EXISTS system_15m (
  at TEXT PRIMARY KEY, mem_total INTEGER, mem_available INTEGER, swap_used INTEGER,
  cpu_pct REAL, load1 REAL, tasks INTEGER,
  gpu_util REAL, gpu_temp REAL, gpu_power_w REAL, disk_used INTEGER, disk_total INTEGER);
CREATE TABLE IF NOT EXISTS counters (category INTEGER PRIMARY KEY, cpu_usec INTEGER NOT NULL, at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS machine (id INTEGER PRIMARY KEY CHECK (id = 0), busy INTEGER NOT NULL, total INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS gpu_mode (id INTEGER PRIMARY KEY CHECK (id = 0), unified INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS measurements (id INTEGER PRIMARY KEY, name TEXT NOT NULL, unit TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS reading_minute (
  at TEXT NOT NULL, measurement INTEGER NOT NULL, value REAL,
  PRIMARY KEY (at, measurement));
CREATE TABLE IF NOT EXISTS category_reading_minute (
  at TEXT NOT NULL, measurement INTEGER NOT NULL, category INTEGER NOT NULL, value REAL,
  PRIMARY KEY (at, measurement, category));
CREATE TABLE IF NOT EXISTS counter_state (
  key TEXT PRIMARY KEY, value INTEGER NOT NULL, ident INTEGER NOT NULL, at TEXT NOT NULL);
";

/// What one write recorded, for the CLI to show. Categories and numbers.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Recorded {
    pub at: String,
    pub cpu_pct: Option<f64>,
    pub by_category: Vec<CategoryRecorded>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CategoryRecorded {
    pub category: &'static str,
    /// `None` when it could not be known this minute (module docs).
    pub mem_bytes: Option<u64>,
    pub cpu_pct: Option<f64>,
    pub tasks: u64,
    pub gpu_mib: Option<u64>,
}

/// Record one sample: the minute rows, the CPU deltas against the previous
/// sample, the last completed fifteen-minute rollup, and retention — in one
/// transaction, so a reader never sees half a minute.
pub fn record(db: &Path, s: &Sample) -> Result<Recorded> {
    if let Some(dir) = db.parent() {
        crate::create_private_dir(dir)?;
    }
    let mut conn = Connection::open(db).with_context(|| format!("opening {}", db.display()))?;
    // A refresh reading mid-sample makes the writer wait rather than drop the
    // minute: in rollback-journal mode a reader blocks the writer as much as
    // the reverse. rusqlite already defaults to five seconds; it is set here
    // so the guarantee does not rest on a library default, and a test pins
    // both directions. (Not WAL: the loaders open read-only, and a read-only
    // connection cannot recreate WAL's -shm file.)
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch(SCHEMA)?;
    let tx = conn.transaction()?;
    let at = ts(s.at.duration_trunc(Duration::minutes(1)).unwrap_or(s.at));

    for c in Category::ALL {
        tx.execute(
            "INSERT OR REPLACE INTO categories (id, label) VALUES (?1, ?2)",
            params![c as u8, c.label()],
        )?;
    }

    // Machine CPU from /proc/stat, as a delta against the previous sample.
    // Whole-machine cores, as `machine_pct` counts them — never the
    // sampler's own parallelism (`parse_cores`).
    let ncpu = s.cores.map(|n| n as f64);
    let mut machine_pct = None;
    if let Some((busy, total)) = s.cpu_jiffies {
        let prev: Option<(i64, i64)> = tx
            .query_row("SELECT busy, total FROM machine WHERE id = 0", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .ok();
        if let Some((pb, pt)) = prev {
            let (db_, dt) = (busy as i64 - pb, total as i64 - pt);
            if dt > 0 && db_ >= 0 {
                machine_pct = Some(db_ as f64 / dt as f64 * 100.0);
            }
        }
        tx.execute(
            "INSERT OR REPLACE INTO machine (id, busy, total) VALUES (0, ?1, ?2)",
            params![busy as i64, total as i64],
        )?;
    }

    // Per category; `Other` is the remainder of the machine, never a sum of
    // named units — the units it covers are not written anywhere.
    let mut shown = Vec::new();
    let used = s.mem.total.saturating_sub(s.mem.available);
    // Unified memory is a property of the machine, not of one sample: a probe
    // that failed this minute must not flip what `mem_bytes` means. The last
    // answer the GPU gave is kept and used when it does not answer.
    let unified = if s.gpu.answered {
        tx.execute(
            "INSERT OR REPLACE INTO gpu_mode (id, unified) VALUES (0, ?1)",
            params![s.gpu.unified],
        )?;
        s.gpu.unified
    } else {
        tx.query_row("SELECT unified FROM gpu_mode WHERE id = 0", [], |r| {
            r.get(0)
        })
        .unwrap_or(false)
    };
    // On unified memory a category's GPU allocations are system RAM its
    // cgroup was not charged for (module docs) — so where the per-process GPU
    // query did not answer this minute, its memory is unknown, never the
    // cgroup figure alone presented as the whole.
    let mem_of = |load: &CategoryLoad| -> Option<u64> {
        if !unified {
            return Some(load.mem_bytes);
        }
        load.gpu_mib.map(|g| load.mem_bytes + g * 1024 * 1024)
    };
    let mut named_mem = Some(0u64);
    let mut named_cpu = 0f64;
    let mut named_tasks = 0u64;
    let mut cat_pct: BTreeMap<Category, Option<f64>> = BTreeMap::new();
    for (cat, load) in &s.by_category {
        let prev: Option<(i64, String)> = tx
            .query_row(
                "SELECT cpu_usec, at FROM counters WHERE category = ?1",
                params![*cat as u8],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        let pct = prev.and_then(|(usec, prev_at)| {
            let prev_at = chrono::NaiveDateTime::parse_from_str(&prev_at, "%Y-%m-%d %H:%M:%S")
                .ok()?
                .and_utc();
            let secs = (s.at - prev_at).num_milliseconds() as f64 / 1000.0;
            let delta = load.cpu_usec as i64 - usec;
            let ncpu = ncpu?;
            (secs > 0.0 && delta >= 0).then(|| delta as f64 / (secs * 1e6 * ncpu) * 100.0)
        });
        tx.execute(
            "INSERT OR REPLACE INTO counters (category, cpu_usec, at) VALUES (?1, ?2, ?3)",
            params![*cat as u8, load.cpu_usec as i64, ts(s.at)],
        )?;
        cat_pct.insert(*cat, pct);
        if *cat != Category::Other {
            named_mem = named_mem.zip(mem_of(load)).map(|(a, b)| a + b);
            named_cpu += pct.unwrap_or(0.0);
            named_tasks += load.tasks;
        }
    }
    for cat in Category::ALL {
        let load = s.by_category.get(&cat).copied().unwrap_or_default();
        let (mem, pct, tasks) = if cat == Category::Other {
            (
                named_mem.map(|n| used.saturating_sub(n)),
                machine_pct.map(|m| (m - named_cpu).max(0.0)),
                s.tasks_total
                    .map_or(load.tasks, |t| t.saturating_sub(named_tasks)),
            )
        } else {
            (
                mem_of(&load),
                cat_pct.get(&cat).copied().flatten(),
                load.tasks,
            )
        };
        tx.execute(
            "INSERT OR REPLACE INTO category_minute (at, category, mem_bytes, cpu_pct, tasks, gpu_mib)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                at,
                cat as u8,
                mem.map(|m| m as i64),
                pct,
                tasks as i64,
                load.gpu_mib.map(|g| g as i64)
            ],
        )?;
        shown.push(CategoryRecorded {
            category: cat.label(),
            mem_bytes: mem,
            cpu_pct: pct,
            tasks,
            gpu_mib: load.gpu_mib,
        });
    }

    tx.execute(
        "INSERT OR REPLACE INTO system_minute (at, mem_total, mem_available, swap_used, cpu_pct,
           load1, tasks, gpu_util, gpu_temp, gpu_power_w, disk_used, disk_total)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            at,
            s.mem.total as i64,
            s.mem.available as i64,
            s.mem.swap_total.saturating_sub(s.mem.swap_free) as i64,
            machine_pct,
            s.load1,
            s.tasks_total.map(|t| t as i64),
            s.gpu.util,
            s.gpu.temp,
            s.gpu.power_w,
            s.disk.map(|d| d.0 as i64),
            s.disk.map(|d| d.1 as i64),
        ],
    )?;

    record_readings(&tx, s, &at, machine_pct)?;

    rollup(&tx, s.at)?;
    let cutoff = |days: i64| ts(s.at - Duration::days(days));
    for table in ["reading_minute", "category_reading_minute"] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE at < ?1"),
            params![cutoff(MINUTE_KEEP_DAYS)],
        )?;
    }
    tx.execute(
        "DELETE FROM category_minute WHERE at < ?1",
        params![cutoff(MINUTE_KEEP_DAYS)],
    )?;
    tx.execute(
        "DELETE FROM system_minute WHERE at < ?1",
        params![cutoff(MINUTE_KEEP_DAYS)],
    )?;
    tx.execute(
        "DELETE FROM category_15m WHERE at < ?1",
        params![cutoff(ROLLUP_KEEP_DAYS)],
    )?;
    tx.execute(
        "DELETE FROM system_15m WHERE at < ?1",
        params![cutoff(ROLLUP_KEEP_DAYS)],
    )?;
    tx.commit()?;

    Ok(Recorded {
        at,
        cpu_pct: machine_pct,
        by_category: shown,
    })
}

/// The named measurements' rows for one minute: every `Now` reading as it
/// was read, and every `Rate` as a difference of counters against the
/// previous sample. `Observed` writes its value, `Unread` writes NULL, and
/// `NotHere` writes no row — three findings, kept apart in the store too.
fn record_readings(
    tx: &rusqlite::Transaction<'_>,
    s: &Sample,
    at: &str,
    machine_pct: Option<f64>,
) -> Result<()> {
    use Measurement as M;
    for m in Measurement::ALL {
        let unit = serde_json::to_value(m.unit())?;
        let unit = unit
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| "label".to_string());
        tx.execute(
            "INSERT OR REPLACE INTO measurements (id, name, unit) VALUES (?1, ?2, ?3)",
            params![*m as u16, m.name(), unit],
        )?;
    }
    let put = |m: Measurement, v: Option<f64>| -> Result<()> {
        tx.execute(
            "INSERT OR REPLACE INTO reading_minute (at, measurement, value) VALUES (?1, ?2, ?3)",
            params![at, m as u16, v],
        )?;
        Ok(())
    };
    for (m, reading) in &s.now {
        match reading {
            Reading::Observed { value } => put(*m, Some(*value))?,
            Reading::Unread { .. } => put(*m, None)?,
            Reading::NotHere => {}
        }
    }
    put(M::CpuBusy, machine_pct)?;

    // A counter's rate, or what stands in for one: NotHere writes nothing.
    let rate = |m: Measurement,
                counter: &Result<(u64, u64), Reading>,
                key: &str,
                per: fn(u64, f64) -> f64|
     -> Result<()> {
        match counter {
            Err(Reading::NotHere) => Ok(()),
            Err(_) => put(m, None),
            Ok((value, ident)) => {
                let d = delta(tx, key, *value, *ident, s.at)?;
                put(m, d.map(|(d, secs)| per(d, secs)))
            }
        }
    };
    let per_sec = |d: u64, secs: f64| d as f64 / secs;
    let c = &s.counters;
    rate(
        M::DiskReadRate,
        &c.disk.clone().map(|(dev, d)| (d.read_bytes, dev)),
        "disk.read_bytes",
        per_sec,
    )?;
    rate(
        M::DiskWriteRate,
        &c.disk.clone().map(|(dev, d)| (d.write_bytes, dev)),
        "disk.write_bytes",
        per_sec,
    )?;
    rate(
        M::DiskBusy,
        &c.disk.clone().map(|(dev, d)| (d.io_ms, dev)),
        "disk.io_ms",
        |ms, secs| (ms as f64 / (secs * 10.0)).min(100.0),
    )?;
    for (rx, tx_, counters, key) in [
        (
            M::NetworkUplinkRxRate,
            M::NetworkUplinkTxRate,
            &c.uplink,
            "uplink",
        ),
        (
            M::NetworkTailnetRxRate,
            M::NetworkTailnetTxRate,
            &c.tailnet,
            "tailnet",
        ),
    ] {
        rate(
            rx,
            &counters.clone().map(|n| (n.rx, n.ifindex)),
            &format!("{key}.rx"),
            per_sec,
        )?;
        rate(
            tx_,
            &counters.clone().map(|n| (n.tx, n.ifindex)),
            &format!("{key}.tx"),
            per_sec,
        )?;
    }

    // OOM kills: the kernel's counter is the authority on how many; the
    // log says which kind and whose. The split is recorded only when the log
    // accounts for exactly the counter's kills — a log that could not be
    // read, or a window that missed one, is unknown, never a guess.
    let kills = match &c.oom_kills {
        Err(Reading::NotHere) => None,
        Err(_) => {
            put(M::MemoryOomKills, None)?;
            None
        }
        Ok(v) => {
            let d = delta(tx, "vmstat.oom_kill", *v, 0, s.at)?.map(|(d, _)| d);
            put(M::MemoryOomKills, d.map(|d| d as f64))?;
            d
        }
    };
    let split: Option<Vec<(bool, Category)>> = match (kills, &c.oom_log) {
        (Some(0), _) => Some(Vec::new()),
        (Some(n), Some(log)) if log.len() as u64 == n => Some(log.clone()),
        _ => None,
    };
    if c.oom_kills != Err(Reading::NotHere) {
        let count = |global: bool| {
            split
                .as_ref()
                .map(|k| k.iter().filter(|(g, _)| *g == global).count() as f64)
        };
        put(M::MemoryOomKillsGlobal, count(true))?;
        put(M::MemoryOomKillsCapped, count(false))?;
        for cat in Category::ALL {
            let n = split
                .as_ref()
                .map(|k| k.iter().filter(|(_, c)| *c == cat).count() as f64);
            tx.execute(
                "INSERT OR REPLACE INTO category_reading_minute (at, measurement, category, value)
                 VALUES (?1, ?2, ?3, ?4)",
                params![at, M::CategoryOomKills as u16, cat as u8, n],
            )?;
        }
    }
    Ok(())
}

/// A cumulative counter's change since the previous sample, with the
/// seconds between. `None` on the first sample, when the counter fell (a
/// reboot, a restarted interface), or when `ident` changed (another device
/// or interface now holds the role): a difference across any of those is
/// not a rate. The counter's state is updated either way.
fn delta(
    tx: &rusqlite::Transaction<'_>,
    key: &str,
    value: u64,
    ident: u64,
    at: DateTime<Utc>,
) -> Result<Option<(u64, f64)>> {
    let prev: Option<(i64, i64, String)> = tx
        .query_row(
            "SELECT value, ident, at FROM counter_state WHERE key = ?1",
            params![key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    tx.execute(
        "INSERT OR REPLACE INTO counter_state (key, value, ident, at) VALUES (?1, ?2, ?3, ?4)",
        params![key, value as i64, ident as i64, ts(at)],
    )?;
    Ok(prev.and_then(|(v, i, prev_at)| {
        let prev_at = chrono::NaiveDateTime::parse_from_str(&prev_at, "%Y-%m-%d %H:%M:%S")
            .ok()?
            .and_utc();
        let secs = (at - prev_at).num_milliseconds() as f64 / 1000.0;
        (i == ident as i64 && value as i64 >= v && secs > 0.0)
            .then(|| ((value as i64 - v) as u64, secs))
    }))
}

/// When the previous sample was taken, to the second — where the next
/// sample's kernel-log window starts. Read-only; `None` without a store or
/// a previous sample.
pub fn last_sample_at(db: &Path) -> Option<DateTime<Utc>> {
    let conn = Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let at: String = conn
        .query_row(
            "SELECT at FROM counter_state WHERE key = 'vmstat.oom_kill'",
            [],
            |r| r.get(0),
        )
        .ok()?;
    chrono::NaiveDateTime::parse_from_str(&at, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|t| t.and_utc())
}

/// What a probe found: one reading, or one per category.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(untagged)]
pub enum Probed {
    One(Reading),
    ByCategory(Vec<(&'static str, Reading)>),
}

/// How old the series' newest minute may be for a probe to answer from it.
const FRESH: i64 = 3;

/// Read one measurement: a `Now` one from the machine, anything else from
/// the series' newest minute when it is fresh. Never writes.
pub fn probe(reader: &Reader, db: &Path, m: Measurement, now: DateTime<Utc>) -> Probed {
    if m.how() == measure::How::Now {
        return Probed::One(reader.now(m));
    }
    match latest(db, m, now) {
        Ok(p) => p,
        Err(e) => Probed::One(Reading::Unread {
            why: format!("the series could not be read: {e:#}"),
        }),
    }
}

fn latest(db: &Path, m: Measurement, now: DateTime<Utc>) -> Result<Probed> {
    let stale = |why: String| Probed::One(Reading::Unread { why });
    if !db.exists() {
        return Ok(stale(
            "no series yet; is mecha-system-sample.timer running?".into(),
        ));
    }
    let conn = Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let newest: Option<String> =
        conn.query_row("SELECT max(at) FROM system_minute", [], |r| r.get(0))?;
    let Some(newest) = newest else {
        return Ok(stale("the series holds no minute yet".into()));
    };
    let at = chrono::NaiveDateTime::parse_from_str(&newest, "%Y-%m-%d %H:%M:%S")?.and_utc();
    if now - at > Duration::minutes(FRESH) {
        return Ok(stale(format!(
            "the series' newest minute is {newest}; is mecha-system-sample.timer running?"
        )));
    }
    // A series written by a sampler older than this measurement has no table
    // for it yet; that is not a broken store.
    let table = match m {
        Measurement::CategoryOomKills => "category_reading_minute",
        _ if m.how() == measure::How::Rate => "reading_minute",
        _ => "category_minute",
    };
    let has_table: bool = conn.query_row(
        "SELECT count(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = ?1",
        params![table],
        |r| r.get(0),
    )?;
    if !has_table {
        return Ok(stale(
            "the series was written by an older sampler; it is recorded from the next sample"
                .into(),
        ));
    }
    let cell = |v: Option<Option<f64>>, missing: Reading| match v {
        None => missing,
        Some(None) => Reading::Unread {
            why: "the sampler could not compute it this minute".into(),
        },
        Some(Some(value)) => Reading::Observed { value },
    };
    if m.how() == measure::How::Rate {
        let v: Option<Option<f64>> = conn
            .query_row(
                "SELECT value FROM reading_minute WHERE at = ?1 AND measurement = ?2",
                params![newest, m as u16],
                |r| r.get(0),
            )
            .optional()?;
        return Ok(Probed::One(cell(v, Reading::NotHere)));
    }
    let mut out = Vec::new();
    for cat in Category::ALL {
        let v: Option<Option<f64>> = match m {
            Measurement::CategoryOomKills => conn
                .query_row(
                    "SELECT value FROM category_reading_minute
                     WHERE at = ?1 AND measurement = ?2 AND category = ?3",
                    params![newest, m as u16, cat as u8],
                    |r| r.get(0),
                )
                .optional()?,
            _ => {
                let column = match m {
                    Measurement::CategoryMemory => "CAST(mem_bytes AS REAL)",
                    Measurement::CategoryCpu => "cpu_pct",
                    Measurement::CategoryTasks => "CAST(tasks AS REAL)",
                    _ => "gpu_mib * 1048576.0",
                };
                conn.query_row(
                    &format!(
                        "SELECT {column} FROM category_minute WHERE at = ?1 AND category = ?2"
                    ),
                    params![newest, cat as u8],
                    |r| r.get(0),
                )
                .optional()?
            }
        };
        out.push((cat.label(), cell(v, Reading::NotHere)));
    }
    Ok(Probed::ByCategory(out))
}

/// The fifteen-minute bucket a stored `at` falls in, in SQL.
const BUCKET: &str = "strftime('%Y-%m-%d %H:', at) || \
    printf('%02d', (CAST(strftime('%M', at) AS INTEGER) / 15) * 15) || ':00'";

/// Average every *completed* fifteen-minute bucket not yet rolled up — from
/// the last bucket already in the rollup (re-closed, since it may have been
/// closed early) up to the one before `now`'s. Not only the previous bucket:
/// a gap in sampling (a suspend, a stopped timer) would otherwise leave the
/// bucket it straddled never rolled up, and its minute rows would age out of
/// the seven-day window with the ninety-day history showing nothing where
/// data existed. Idempotent, and bounded by the minute rows still kept.
fn rollup(tx: &rusqlite::Transaction<'_>, now: DateTime<Utc>) -> Result<()> {
    let current = ts(now.duration_trunc(Duration::minutes(15)).unwrap_or(now));
    let from: String = tx
        .query_row("SELECT max(at) FROM system_15m", [], |r| {
            r.get::<_, Option<String>>(0)
        })?
        .unwrap_or_default();
    tx.execute(
        &format!(
            "INSERT OR REPLACE INTO category_15m (at, category, mem_bytes, cpu_pct, tasks, gpu_mib)
             SELECT {BUCKET} AS bucket, category, CAST(avg(mem_bytes) AS INTEGER), avg(cpu_pct),
                    CAST(avg(tasks) AS INTEGER), CAST(avg(gpu_mib) AS INTEGER)
             FROM category_minute WHERE at >= ?1 AND at < ?2 GROUP BY bucket, category"
        ),
        params![from, current],
    )?;
    tx.execute(
        &format!(
            "INSERT OR REPLACE INTO system_15m (at, mem_total, mem_available, swap_used, cpu_pct,
               load1, tasks, gpu_util, gpu_temp, gpu_power_w, disk_used, disk_total)
             SELECT {BUCKET} AS bucket, CAST(avg(mem_total) AS INTEGER),
                    CAST(avg(mem_available) AS INTEGER), CAST(avg(swap_used) AS INTEGER),
                    avg(cpu_pct), avg(load1), CAST(avg(tasks) AS INTEGER), avg(gpu_util),
                    avg(gpu_temp), avg(gpu_power_w), CAST(avg(disk_used) AS INTEGER),
                    CAST(avg(disk_total) AS INTEGER)
             FROM system_minute WHERE at >= ?1 AND at < ?2 GROUP BY bucket"
        ),
        params![from, current],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
