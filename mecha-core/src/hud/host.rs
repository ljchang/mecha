//! The host's load, by category: what the machine running mecha is doing —
//! memory, CPU, tasks, GPU memory — broken down by what *kind* of work it is,
//! and nothing finer (design §11, owner ruling R13).
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
use rusqlite::{params, Connection};

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
    /// `None`, never a neighbour.
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

// ---- parsers: pure functions of what the system printed ----

/// `/proc/meminfo`, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MemInfo {
    pub total: u64,
    pub available: u64,
    pub swap_total: u64,
    pub swap_free: u64,
}

pub fn parse_meminfo(text: &str) -> MemInfo {
    let mut m = MemInfo::default();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(key), Some(value)) = (parts.next(), parts.next()) else {
            continue;
        };
        let Ok(kib) = value.parse::<u64>() else {
            continue;
        };
        let bytes = kib * 1024;
        match key {
            "MemTotal:" => m.total = bytes,
            "MemAvailable:" => m.available = bytes,
            "SwapTotal:" => m.swap_total = bytes,
            "SwapFree:" => m.swap_free = bytes,
            _ => {}
        }
    }
    m
}

/// The aggregate `cpu` line of `/proc/stat`: (busy, total) jiffies.
pub fn parse_proc_stat(text: &str) -> Option<(u64, u64)> {
    let line = text.lines().find(|l| l.starts_with("cpu "))?;
    let v: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|x| x.parse().ok())
        .collect();
    if v.len() < 8 {
        return None;
    }
    // user nice system idle iowait irq softirq steal …
    let idle = v[3] + v[4];
    let total: u64 = v[..8].iter().sum();
    Some((total - idle, total))
}

/// `/proc/loadavg`: the one-minute load and the total task count.
pub fn parse_loadavg(text: &str) -> Option<(f64, u64)> {
    let mut parts = text.split_whitespace();
    let load1 = parts.next()?.parse().ok()?;
    let tasks = parts.nth(2)?.split('/').nth(1)?.parse().ok()?;
    Some((load1, tasks))
}

/// `usage_usec` from a cgroup's `cpu.stat`.
pub fn parse_cpu_stat(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|l| l.strip_prefix("usage_usec "))
        .and_then(|v| v.trim().parse().ok())
}

/// The GPU as a whole: utilisation %, temperature °C, power draw W — each
/// `None` when the GB10 answers `[N/A]`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GpuNow {
    pub util: Option<f64>,
    pub temp: Option<f64>,
    pub power_w: Option<f64>,
    /// The GPU reported no memory total of its own — it allocates from the
    /// system pool (the GB10).
    pub unified: bool,
}

/// `nvidia-smi --query-gpu=utilization.gpu,temperature.gpu,power.draw,memory.total
/// --format=csv,noheader,nounits`.
pub fn parse_gpu(text: &str) -> GpuNow {
    let line = text.lines().next().unwrap_or("");
    let raw: Vec<&str> = line.split(',').map(str::trim).collect();
    let num = |i: usize| raw.get(i).and_then(|f| f.parse::<f64>().ok());
    GpuNow {
        util: num(0),
        temp: num(1),
        power_w: num(2),
        unified: raw.get(3).is_some_and(|f| f.contains("N/A")),
    }
}

/// `nvidia-smi --query-compute-apps=pid,used_memory
/// --format=csv,noheader,nounits`: (pid, MiB) per process.
pub fn parse_gpu_apps(text: &str) -> Vec<(u32, u64)> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split(',').map(str::trim);
            Some((f.next()?.parse().ok()?, f.next()?.parse().ok()?))
        })
        .collect()
}

/// The unit a process belongs to, from `/proc/<pid>/cgroup`: the last path
/// segment when it is a `.service`. Used in memory only.
pub fn unit_of_cgroup(text: &str) -> Option<String> {
    let path = text.lines().find_map(|l| l.strip_prefix("0::"))?;
    let last = path.rsplit('/').next()?;
    last.ends_with(".service").then(|| last.to_string())
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
    pub load1: Option<f64>,
    pub tasks_total: Option<u64>,
    pub gpu: GpuNow,
    pub disk: Option<(u64, u64)>,
    pub by_category: BTreeMap<Category, CategoryLoad>,
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
pub fn collect(at: DateTime<Utc>) -> Sample {
    let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
    let mem = parse_meminfo(&read("/proc/meminfo"));
    let cpu_jiffies = parse_proc_stat(&read("/proc/stat"));
    let (load1, tasks_total) = parse_loadavg(&read("/proc/loadavg")).unzip();

    let units = unit_counters(&user_app_slice());
    let gpu = run(
        "nvidia-smi",
        &[
            "--query-gpu=utilization.gpu,temperature.gpu,power.draw,memory.total",
            "--format=csv,noheader,nounits",
        ],
    )
    .map(|t| parse_gpu(&t))
    .unwrap_or_default();
    let gpu_apps = run(
        "nvidia-smi",
        &[
            "--query-compute-apps=pid,used_memory",
            "--format=csv,noheader,nounits",
        ],
    )
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

    Sample {
        at,
        mem,
        cpu_jiffies,
        load1,
        tasks_total,
        gpu,
        disk: disk_usage(Path::new("/")),
        by_category: fold(&units, gpu_apps.as_deref()),
    }
}

/// The user manager's `app.slice`, where systemd puts user services.
fn user_app_slice() -> PathBuf {
    // SAFETY: getuid cannot fail and has no preconditions.
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!(
        "/sys/fs/cgroup/user.slice/user-{uid}.slice/user@{uid}.service/app.slice"
    ))
}

fn unit_counters(slice: &Path) -> Vec<UnitCounters> {
    let Ok(entries) = std::fs::read_dir(slice) else {
        return Vec::new();
    };
    let num = |p: PathBuf| {
        std::fs::read_to_string(p)
            .ok()
            .and_then(|t| t.trim().parse::<u64>().ok())
            .unwrap_or(0)
    };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            if !name.ends_with(".service") {
                return None;
            }
            let dir = e.path();
            let cpu = std::fs::read_to_string(dir.join("cpu.stat"))
                .ok()
                .and_then(|t| parse_cpu_stat(&t))
                .unwrap_or(0);
            Some((
                name,
                num(dir.join("memory.current")),
                cpu,
                num(dir.join("pids.current")),
            ))
        })
        .collect()
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn disk_usage(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: a zeroed statvfs is a valid out-parameter; `c` is a valid path.
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let block = s.f_frsize as u64;
    let total = s.f_blocks as u64 * block;
    let free = s.f_bavail as u64 * block;
    Some((total.saturating_sub(free), total))
}

// ---- the store ----

const MINUTE_KEEP_DAYS: i64 = 7;
const ROLLUP_KEEP_DAYS: i64 = 90;

/// `~/.mecha/hud/host.sqlite`.
pub fn db_path() -> Result<PathBuf> {
    Ok(super::dir()?.join("host.sqlite"))
}

fn ts(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%d %H:%M:%S").to_string()
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS categories (id INTEGER PRIMARY KEY, label TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS category_minute (
  at TEXT NOT NULL, category INTEGER NOT NULL,
  mem_bytes INTEGER NOT NULL, cpu_pct REAL, tasks INTEGER NOT NULL, gpu_mib INTEGER,
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
";

/// What one write recorded, for the CLI to show. Categories and numbers.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Recorded {
    pub at: String,
    pub cpu_pct: Option<f64>,
    pub by_category: Vec<(String, u64, Option<f64>)>,
}

/// Record one sample: the minute rows, the CPU deltas against the previous
/// sample, the last completed fifteen-minute rollup, and retention — in one
/// transaction, so a reader never sees half a minute.
pub fn record(db: &Path, s: &Sample) -> Result<Recorded> {
    if let Some(dir) = db.parent() {
        crate::create_private_dir(dir)?;
    }
    let mut conn = Connection::open(db).with_context(|| format!("opening {}", db.display()))?;
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
    let ncpu = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
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
    // On unified memory a category's GPU allocations are system RAM its
    // cgroup was not charged for (module docs).
    let mem_of = |load: &CategoryLoad| {
        let gpu = if s.gpu.unified {
            load.gpu_mib.unwrap_or(0) * 1024 * 1024
        } else {
            0
        };
        load.mem_bytes + gpu
    };
    let mut named_mem = 0u64;
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
            (secs > 0.0 && delta >= 0).then(|| delta as f64 / (secs * 1e6 * ncpu) * 100.0)
        });
        tx.execute(
            "INSERT OR REPLACE INTO counters (category, cpu_usec, at) VALUES (?1, ?2, ?3)",
            params![*cat as u8, load.cpu_usec as i64, ts(s.at)],
        )?;
        cat_pct.insert(*cat, pct);
        if *cat != Category::Other {
            named_mem += mem_of(load);
            named_cpu += pct.unwrap_or(0.0);
            named_tasks += load.tasks;
        }
    }
    for cat in Category::ALL {
        let load = s.by_category.get(&cat).copied().unwrap_or_default();
        let (mem, pct, tasks) = if cat == Category::Other {
            (
                used.saturating_sub(named_mem),
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
            params![at, cat as u8, mem as i64, pct, tasks as i64, load.gpu_mib.map(|g| g as i64)],
        )?;
        shown.push((cat.label().to_string(), mem, pct));
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

    rollup(&tx, s.at)?;
    let cutoff = |days: i64| ts(s.at - Duration::days(days));
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

/// Average the last *completed* fifteen-minute bucket into the rollups.
/// Idempotent: re-running a minute rewrites the same bucket.
fn rollup(tx: &rusqlite::Transaction<'_>, now: DateTime<Utc>) -> Result<()> {
    let bucket = now.duration_trunc(Duration::minutes(15)).unwrap_or(now) - Duration::minutes(15);
    let (from, to) = (ts(bucket), ts(bucket + Duration::minutes(15)));
    tx.execute(
        "INSERT OR REPLACE INTO category_15m (at, category, mem_bytes, cpu_pct, tasks, gpu_mib)
         SELECT ?1, category, CAST(avg(mem_bytes) AS INTEGER), avg(cpu_pct),
                CAST(avg(tasks) AS INTEGER), CAST(avg(gpu_mib) AS INTEGER)
         FROM category_minute WHERE at >= ?1 AND at < ?2 GROUP BY category",
        params![from, to],
    )?;
    tx.execute(
        "INSERT OR REPLACE INTO system_15m (at, mem_total, mem_available, swap_used, cpu_pct,
           load1, tasks, gpu_util, gpu_temp, gpu_power_w, disk_used, disk_total)
         SELECT ?1, CAST(avg(mem_total) AS INTEGER), CAST(avg(mem_available) AS INTEGER),
                CAST(avg(swap_used) AS INTEGER), avg(cpu_pct), avg(load1),
                CAST(avg(tasks) AS INTEGER), avg(gpu_util), avg(gpu_temp), avg(gpu_power_w),
                CAST(avg(disk_used) AS INTEGER), CAST(avg(disk_total) AS INTEGER)
         FROM system_minute WHERE at >= ?1 AND at < ?2 HAVING count(*) > 0",
        params![from, to],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
