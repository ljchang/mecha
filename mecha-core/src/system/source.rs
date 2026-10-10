//! One parser per source (`docs/SYSTEM-STATE-DESIGN.md` §1.2).
//!
//! Every function here that parses is a pure function of the text a file or
//! a command printed, so every figure `mecha system` reports is testable on a
//! fixture without the machine. The few that touch the system — a bounded
//! command, `statvfs` — are named for it and return what they could not read
//! as `None` or a [`Ran`] variant, never as zero.
//!
//! **Nothing else in mecha parses these sources.** The homeostat, the image
//! tool's memory gate and `recommend`'s machine read all come here; a second
//! parser of `/proc/meminfo` is the duplication this module exists to end.

use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant};

/// `/proc/meminfo`, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MemInfo {
    pub total: u64,
    pub available: u64,
    pub swap_total: u64,
    pub swap_free: u64,
}

/// `None` unless both `MemTotal:` and `MemAvailable:` are there — an
/// unreadable `/proc/meminfo` is not a machine with no memory, and one with
/// no `MemAvailable:` (a kernel before 3.14) is not a machine with nothing
/// left. Swap absent is zero: a kernel without swap has none in use.
pub fn parse_meminfo(text: &str) -> Option<MemInfo> {
    let mut m = MemInfo::default();
    let mut seen_total = false;
    let mut seen_available = false;
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
            "MemTotal:" => {
                m.total = bytes;
                seen_total = true;
            }
            "MemAvailable:" => {
                m.available = bytes;
                seen_available = true;
            }
            "SwapTotal:" => m.swap_total = bytes,
            "SwapFree:" => m.swap_free = bytes,
            _ => {}
        }
    }
    (seen_total && seen_available).then_some(m)
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

/// `/proc/stat`'s per-core lines: the cores the aggregate `cpu` line sums
/// over. Read from the same text as the jiffies on purpose — the sampler's
/// own affinity or `CPUQuota=` narrows `available_parallelism()` but not
/// this, and the two denominators must agree.
pub fn parse_cores(text: &str) -> Option<u64> {
    let n = text
        .lines()
        .filter(|l| {
            l.strip_prefix("cpu")
                .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()))
        })
        .count() as u64;
    (n > 0).then_some(n)
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
    /// `nvidia-smi` answered at all. When it did not, `unified` says nothing
    /// and the store's last known answer is used instead (`record`).
    pub answered: bool,
}

/// `nvidia-smi --query-gpu=utilization.gpu,temperature.gpu,power.draw,memory.total
/// --format=csv,noheader,nounits` — one line per GPU. Utilisation and
/// temperature are the hottest GPU's, power is the sum, and `unified` holds
/// only when *every* GPU reports no memory total: whether GPU memory is
/// folded into a category's memory must not depend on which GPU is listed
/// first.
pub fn parse_gpu(text: &str) -> GpuNow {
    let lines: Vec<Vec<&str>> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split(',').map(str::trim).collect())
        .collect();
    let col = |i: usize| -> Vec<f64> {
        lines
            .iter()
            .filter_map(|f| f.get(i).and_then(|v| v.parse::<f64>().ok()))
            .collect()
    };
    let max = |v: Vec<f64>| v.into_iter().reduce(f64::max);
    let power = col(2);
    GpuNow {
        util: max(col(0)),
        temp: max(col(1)),
        // A total only when every GPU answered: a partial sum is not the
        // machine's draw.
        power_w: (!power.is_empty() && power.len() == lines.len()).then(|| power.iter().sum()),
        unified: !lines.is_empty()
            && lines
                .iter()
                .all(|f| f.get(3).is_some_and(|m| m.contains("N/A"))),
        answered: !lines.is_empty(),
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

// ---- bounded commands ----

/// How long any command `mecha system` runs may take before it is killed.
/// `nvidia-smi` blocks for good on a wedged driver; nothing here waits on it.
pub const FORK_TIMEOUT: Duration = Duration::from_secs(10);

/// What a bounded command produced. A program that is not installed is a
/// different finding from one that failed or hung: the first is a fact
/// about the machine (`NotHere`), the others are incidents (`Unread`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ran {
    Out(String),
    Missing,
    Failed,
    TimedOut,
}

impl Ran {
    pub fn out(self) -> Option<String> {
        match self {
            Ran::Out(s) => Some(s),
            _ => None,
        }
    }
}

/// Run `cmd args`, killed after `timeout`. Its output is drained on a thread
/// while it runs, so a large answer (`tailscale status --json` on a busy
/// tailnet) cannot fill the pipe and stall the child into the timeout.
pub fn run_bounded(cmd: &str, args: &[&str], timeout: Duration) -> Ran {
    // A spent budget runs nothing: starting a command only to kill it would
    // be load for no reading.
    if timeout.is_zero() {
        return Ran::TimedOut;
    }
    let mut child = match std::process::Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ran::Missing,
        Err(_) => return Ran::Failed,
    };
    let mut stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(out) = stdout.as_mut() {
            let _ = out.read_to_string(&mut text);
        }
        text
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break None,
        }
    };
    let text = reader.join().unwrap_or_default();
    match status {
        Some(s) if s.success() => Ran::Out(text),
        Some(_) => Ran::Failed,
        None if started.elapsed() > timeout => Ran::TimedOut,
        None => Ran::Failed,
    }
}

/// One `nvidia-smi --query-<what>` in CSV with no header or units.
pub fn nvidia_smi(query: &str, timeout: Duration) -> Ran {
    run_bounded(
        "nvidia-smi",
        &[query, "--format=csv,noheader,nounits"],
        timeout,
    )
}

/// Used and total bytes of the filesystem holding `path` (`statvfs`). Used
/// counts blocks reserved for root, so it reads a little above `df`'s
/// `Use%`: space this user cannot have is not free to it.
pub fn disk_usage(path: &Path) -> Option<(u64, u64)> {
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

// ---- pressure, OOM, temperature, uptime ----

/// `/proc/pressure/<resource>`'s `some` line: the share of time at least one
/// task was stalled waiting on the resource, over ten and sixty seconds.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pressure {
    pub avg10: f64,
    pub avg60: f64,
}

pub fn parse_pressure(text: &str) -> Option<Pressure> {
    let line = text.lines().find(|l| l.starts_with("some "))?;
    let field = |key: &str| -> Option<f64> {
        line.split_whitespace()
            .find_map(|kv| kv.strip_prefix(key))?
            .parse()
            .ok()
    };
    Some(Pressure {
        avg10: field("avg10=")?,
        avg60: field("avg60=")?,
    })
}

/// `/proc/vmstat`'s `oom_kill`: every OOM kill since boot, the machine's and
/// every capped cgroup's alike. [`parse_oom_kills`] says which was which.
pub fn parse_vmstat_oom_kill(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|l| l.strip_prefix("oom_kill "))?
        .trim()
        .parse()
        .ok()
}

/// One OOM kill as the kernel logged it, with nothing that names a process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OomKill {
    /// `constraint=CONSTRAINT_NONE`: the machine ran out. Anything else is a
    /// cgroup (or cpuset, or mempolicy) hitting the limit it was given.
    pub global: bool,
    /// The killed task's unit, when it was a user service — folded into a
    /// category by the caller and dropped.
    pub unit: Option<String>,
}

/// The kernel log's `oom-kill:` lines (`journalctl -k`). The task's name and
/// pid are on the same line and are never read.
pub fn parse_oom_kills(text: &str) -> Vec<OomKill> {
    text.lines()
        .filter_map(|l| l.split_once("oom-kill:").map(|(_, rest)| rest))
        .map(|rest| {
            let field = |key: &str| {
                rest.split(',')
                    .find_map(|kv| kv.strip_prefix(key))
                    .map(str::to_string)
            };
            let unit = field("task_memcg=").and_then(|path| {
                let last = path.rsplit('/').next()?.to_string();
                last.ends_with(".service").then_some(last)
            });
            OomKill {
                global: field("constraint=").as_deref() == Some("CONSTRAINT_NONE"),
                unit,
            }
        })
        .collect()
}

/// `/proc/uptime`'s first field, in seconds. A fall is a reboot.
pub fn parse_uptime(text: &str) -> Option<f64> {
    text.split_whitespace().next()?.parse().ok()
}

/// A `/sys/class/thermal/thermal_zone*/temp` reading (millidegrees) in °C.
/// A zone that reports nonsense (negative, or hotter than anything a machine
/// survives) is `None`, never folded into the maximum.
pub fn parse_thermal(text: &str) -> Option<f64> {
    let milli: i64 = text.trim().parse().ok()?;
    let c = milli as f64 / 1000.0;
    (0.0..150.0).contains(&c).then_some(c)
}

// ---- storage ----

/// One block device's cumulative counters from `/proc/diskstats`. Sectors
/// there are always 512 bytes, whatever the device's own sector size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskCounters {
    pub read_bytes: u64,
    pub write_bytes: u64,
    /// Milliseconds the device had I/O in flight: its busy time.
    pub io_ms: u64,
}

pub fn parse_diskstats(text: &str, major: u64, minor: u64) -> Option<DiskCounters> {
    text.lines().find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 13 || f[0].parse() != Ok(major) || f[1].parse() != Ok(minor) {
            return None;
        }
        let n = |i: usize| f[i].parse::<u64>().ok();
        Some(DiskCounters {
            read_bytes: n(5)? * 512,
            write_bytes: n(9)? * 512,
            io_ms: n(12)?,
        })
    })
}

// ---- network ----

/// The interface carrying the default route in `/proc/net/route`: up, a
/// zero destination and mask, and the lowest metric when several do.
pub fn parse_default_route(text: &str) -> Option<String> {
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let flags = u32::from_str_radix(f.get(3)?, 16).ok()?;
            let up = flags & 0x1 != 0;
            (up && *f.get(1)? == "00000000" && *f.get(7)? == "00000000")
                .then(|| Some((f.get(6)?.parse::<u64>().ok()?, f[0].to_string())))
                .flatten()
        })
        .min()
        .map(|(_, iface)| iface)
}

/// What `iw dev <if> link` says about the Wi-Fi link: the signal level and
/// the rate the link is transmitting at, which is the link's speed without
/// sending a byte of test traffic.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WifiLink {
    pub signal_dbm: Option<f64>,
    pub bitrate_mbit: Option<f64>,
}

pub fn parse_iw_link(text: &str) -> Option<WifiLink> {
    if text.trim_start().starts_with("Not connected") {
        return None;
    }
    let after = |key: &str| -> Option<f64> {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(key))?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    };
    Some(WifiLink {
        signal_dbm: after("signal:"),
        bitrate_mbit: after("tx bitrate:"),
    })
}

/// `tailscale status --json`: whether the backend is running, and how many
/// peers are online. Peer names are in the same document and never read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tailnet {
    pub up: bool,
    pub peers_online: u64,
}

pub fn parse_tailscale_status(text: &str) -> Option<Tailnet> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let up = v.get("BackendState")?.as_str()? == "Running";
    let peers_online = v
        .get("Peer")
        .and_then(|p| p.as_object())
        .map_or(0, |peers| {
            peers
                .values()
                .filter(|p| p.get("Online").and_then(|o| o.as_bool()) == Some(true))
                .count() as u64
        });
    Some(Tailnet { up, peers_online })
}

// ---- GPU throttling, failed units ----

/// Whether any GPU is held below its clocks by power or heat, from
/// `nvidia-smi --query-gpu=clocks_throttle_reasons.active`. The idle and
/// application-clock bits are not throttling and are masked out; an empty
/// answer is `None`.
pub fn parse_throttle(text: &str) -> Option<bool> {
    // SW power cap, HW slowdown, SW thermal, HW thermal, HW power brake.
    const THROTTLING: u64 = 0x4 | 0x8 | 0x20 | 0x40 | 0x80;
    let masks: Vec<u64> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| u64::from_str_radix(l.trim_start_matches("0x"), 16).ok())
        .collect::<Option<_>>()?;
    (!masks.is_empty()).then(|| masks.iter().any(|m| m & THROTTLING != 0))
}

/// `systemctl --user --failed --no-legend --plain`: one line per failed unit.
pub fn parse_failed_units(text: &str) -> u64 {
    text.lines().filter(|l| !l.trim().is_empty()).count() as u64
}

// ---- macOS ----

/// `vm_stat`'s pages that can be handed to a new allocation without
/// swapping — free, inactive, speculative and purgeable — times its page
/// size, in bytes. The closest macOS analogue of Linux's `MemAvailable`.
pub fn parse_vm_stat(text: &str) -> Option<u64> {
    let page: u64 = text
        .lines()
        .next()?
        .split("page size of ")
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let pages = |key: &str| -> Option<u64> {
        let line = text.lines().find(|l| l.starts_with(key))?;
        line.rsplit(':')
            .next()?
            .trim()
            .trim_end_matches('.')
            .parse()
            .ok()
    };
    let total = pages("Pages free")?
        + pages("Pages inactive")?
        + pages("Pages speculative").unwrap_or(0)
        + pages("Pages purgeable").unwrap_or(0);
    Some(total * page)
}
