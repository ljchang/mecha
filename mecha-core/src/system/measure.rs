//! The named measurements and how one is read now
//! (`docs/SYSTEM-STATE-DESIGN.md` §1.1, §2).
//!
//! [`Measurement`] is a closed list, and its numbers are a wire format: the
//! series stores a reading under its number for seven days, so a number is
//! never reordered or reused, and one this build does not know reads back as
//! `None`. Its dotted name is the same on the CLI, in the series'
//! `measurements` table, and (in S5) in the model tool.
//!
//! A read keeps unknown apart from zero ([`Reading`]): `NotHere` is a fact
//! about the machine (no GPU, no Wi-Fi, no PSI in this kernel), `Unread` is an
//! incident with a reason, and only `Observed` carries a number.
//!
//! Three kinds of measurement ([`How`]):
//! - **`Now`** — read from the machine at once, by [`Reader`];
//! - **`Rate`** — a change over time (CPU busy, disk and network rates, OOM
//!   kills), which only the sampler can compute, from two samples;
//! - **`ByCategory`** — one value per [`Category`], also the sampler's.
//!
//! A probe of the last two reads the series' newest minute (`super::latest`).

use std::cell::OnceCell;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::Serialize;

use super::source::{self, GpuNow, MemInfo, Pressure, Ran, Tailnet, WifiLink};

/// What a reading is measured in. A `Label` measurement stores the index of
/// its label; a `Flag` stores 0 or 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Bytes,
    BytesPerSec,
    Percent,
    Count,
    Load,
    Celsius,
    Watts,
    Seconds,
    Dbm,
    Mbits,
    Flag,
    Label(&'static [&'static str]),
}

/// How a measurement is had: read now, or computed by the sampler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum How {
    Now,
    Rate,
    ByCategory,
}

/// What reading it costs (§1.1). `Fork` reads are bounded by
/// [`source::FORK_TIMEOUT`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cost {
    File,
    Fork,
}

const UPLINK_KINDS: &[&str] = &["wired", "wireless"];

/// What a measurement this build lists but does not read says. A test over
/// the whole list fails on it, so a new variant cannot ship unwired.
pub const NOT_WIRED: &str = "this build has no reader for this measurement";

macro_rules! measurements {
    ($( $(#[$doc:meta])* $v:ident = $n:literal, $name:literal, $unit:expr, $cost:ident, $how:ident; )*) => {
        /// A measurement mecha can make of the machine. Numbers are fixed:
        /// never reorder, never reuse one; a new measurement takes a free
        /// number in its group.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u16)]
        pub enum Measurement {
            $( $(#[$doc])* $v = $n, )*
        }

        impl Measurement {
            /// Every measurement, in number order.
            pub const ALL: &'static [Measurement] = &[$(Measurement::$v),*];

            /// The dotted name: `memory.available`, `gpu.temperature`.
            pub fn name(self) -> &'static str {
                match self { $( Measurement::$v => $name, )* }
            }
            pub fn unit(self) -> Unit {
                match self { $( Measurement::$v => $unit, )* }
            }
            pub fn cost(self) -> Cost {
                match self { $( Measurement::$v => Cost::$cost, )* }
            }
            pub fn how(self) -> How {
                match self { $( Measurement::$v => How::$how, )* }
            }
            /// A stored number back to a measurement; one this build does not
            /// know is `None`, never a neighbour.
            pub fn from_u16(n: u16) -> Option<Measurement> {
                match n { $( $n => Some(Measurement::$v), )* _ => None }
            }
        }
    };
}

measurements! {
    MemoryTotal = 1, "memory.total", Unit::Bytes, File, Now;
    /// What a new allocation can have without swapping: the number the
    /// memory floor compares against.
    MemoryAvailable = 2, "memory.available", Unit::Bytes, File, Now;
    MemorySwapUsed = 3, "memory.swap_used", Unit::Bytes, File, Now;
    /// Every OOM kill since the previous sample, the machine's and every
    /// capped cgroup's alike (`/proc/vmstat`).
    MemoryOomKills = 4, "memory.oom_kills", Unit::Count, File, Rate;
    /// Of those, the kills where the machine itself ran out.
    MemoryOomKillsGlobal = 5, "memory.oom_kills.global", Unit::Count, Fork, Rate;
    /// Of those, the kills where a cgroup hit the cap it was given — a
    /// sandboxed command over its limit is the cap working.
    MemoryOomKillsCapped = 6, "memory.oom_kills.capped", Unit::Count, Fork, Rate;

    CpuBusy = 10, "cpu.busy", Unit::Percent, File, Rate;
    CpuCores = 11, "cpu.cores", Unit::Count, File, Now;
    CpuLoad1m = 12, "cpu.load_1m", Unit::Load, File, Now;
    CpuTasks = 13, "cpu.tasks", Unit::Count, File, Now;
    /// The hottest thermal zone. On a single-die machine like the GB10 the
    /// zones are the package the GPU shares, so this rises with GPU work.
    CpuTemperature = 14, "cpu.temperature", Unit::Celsius, File, Now;

    /// Share of the last ten seconds some task waited on the CPU.
    PressureCpuAvg10 = 20, "pressure.cpu.avg10", Unit::Percent, File, Now;
    PressureCpuAvg60 = 21, "pressure.cpu.avg60", Unit::Percent, File, Now;
    /// Share of the last ten seconds some task stalled on memory reclaim —
    /// the leading sign of an out-of-memory, before anything is killed.
    PressureMemoryAvg10 = 22, "pressure.memory.avg10", Unit::Percent, File, Now;
    PressureMemoryAvg60 = 23, "pressure.memory.avg60", Unit::Percent, File, Now;
    PressureIoAvg10 = 24, "pressure.io.avg10", Unit::Percent, File, Now;
    PressureIoAvg60 = 25, "pressure.io.avg60", Unit::Percent, File, Now;

    DiskRootUsed = 30, "disk.root.used", Unit::Bytes, File, Now;
    DiskRootTotal = 31, "disk.root.total", Unit::Bytes, File, Now;
    DiskReadRate = 32, "disk.read_rate", Unit::BytesPerSec, File, Rate;
    DiskWriteRate = 33, "disk.write_rate", Unit::BytesPerSec, File, Rate;
    DiskBusy = 34, "disk.busy", Unit::Percent, File, Rate;

    GpuUtilization = 40, "gpu.utilization", Unit::Percent, Fork, Now;
    GpuTemperature = 41, "gpu.temperature", Unit::Celsius, Fork, Now;
    GpuPower = 42, "gpu.power", Unit::Watts, Fork, Now;
    /// The GPU allocates from system memory (no memory total of its own).
    GpuUnified = 43, "gpu.unified", Unit::Flag, Fork, Now;
    /// Held below its clocks by power or heat.
    GpuThrottled = 44, "gpu.throttled", Unit::Flag, Fork, Now;

    NetworkUplinkRxRate = 50, "network.uplink.rx_rate", Unit::BytesPerSec, File, Rate;
    NetworkUplinkTxRate = 51, "network.uplink.tx_rate", Unit::BytesPerSec, File, Rate;
    NetworkUplinkKind = 52, "network.uplink.kind", Unit::Label(UPLINK_KINDS), File, Now;
    NetworkWifiSignal = 53, "network.wifi.signal", Unit::Dbm, Fork, Now;
    /// The rate the Wi-Fi link transmits at: its speed, without sending a
    /// byte of test traffic.
    NetworkWifiBitrate = 54, "network.wifi.bitrate", Unit::Mbits, Fork, Now;
    NetworkTailnetRxRate = 55, "network.tailnet.rx_rate", Unit::BytesPerSec, File, Rate;
    NetworkTailnetTxRate = 56, "network.tailnet.tx_rate", Unit::BytesPerSec, File, Rate;
    NetworkTailnetUp = 57, "network.tailnet.up", Unit::Flag, Fork, Now;
    NetworkTailnetPeersOnline = 58, "network.tailnet.peers_online", Unit::Count, Fork, Now;

    /// Seconds since boot; a fall is a reboot.
    SystemUptime = 60, "system.uptime", Unit::Seconds, File, Now;
    /// Failed units in the user manager.
    ServicesFailed = 61, "services.failed", Unit::Count, Fork, Now;

    CategoryMemory = 100, "category.memory", Unit::Bytes, Fork, ByCategory;
    CategoryCpu = 101, "category.cpu", Unit::Percent, File, ByCategory;
    CategoryTasks = 102, "category.tasks", Unit::Count, File, ByCategory;
    CategoryGpuMemory = 103, "category.gpu_memory", Unit::Bytes, Fork, ByCategory;
    /// OOM kills since the previous sample whose victim was this kind of
    /// work, from the kernel log.
    CategoryOomKills = 104, "category.oom_kills", Unit::Count, Fork, ByCategory;
}

impl Measurement {
    /// Exactly this name, or every measurement under a group prefix
    /// (`gpu` → every `gpu.*`). Empty when nothing matches; the caller says
    /// so and lists the names, never prints an empty table.
    pub fn matching(query: &str) -> Vec<Measurement> {
        let query = query.trim_end_matches('.');
        Measurement::ALL
            .iter()
            .copied()
            .filter(|m| {
                let name = m.name();
                name == query
                    || name
                        .strip_prefix(query)
                        .is_some_and(|rest| rest.starts_with('.'))
            })
            .collect()
    }
}

/// One reading. Unknown is never zero.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Reading {
    Observed {
        value: f64,
    },
    /// The source exists here and could not be read now; `why` is for the
    /// doctor and the CLI, never a value.
    Unread {
        why: String,
    },
    /// This machine does not have it.
    NotHere,
}

impl Reading {
    pub fn value(&self) -> Option<f64> {
        match self {
            Reading::Observed { value } => Some(*value),
            _ => None,
        }
    }

    fn unread(why: impl Into<String>) -> Reading {
        Reading::Unread { why: why.into() }
    }

    fn of(value: Option<f64>, why: &str) -> Reading {
        match value {
            Some(value) => Reading::Observed { value },
            None => Reading::unread(why),
        }
    }
}

/// A file read three ways: its text, absent (the machine does not have it),
/// or present and unreadable.
enum File {
    Text(String),
    Absent,
    Unreadable(String),
}

fn file(path: &str) -> File {
    match std::fs::read_to_string(path) {
        Ok(t) => File::Text(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => File::Absent,
        Err(e) => File::Unreadable(format!("{path}: {e}")),
    }
}

/// Reads the machine, each source at most once however many measurements
/// ask for it — one `nvidia-smi` answers all five GPU measurements. Not
/// `Sync`, and not meant to outlive one probe or one sample: a reading is of
/// a moment.
#[derive(Default)]
pub struct Reader {
    /// When every command this reader runs must have finished. A command
    /// gets what is left of it, at most [`source::FORK_TIMEOUT`]; once it is
    /// spent, a reading that needs a command is `Unread`.
    deadline: Option<Instant>,
    meminfo: OnceCell<Result<MemInfo, Reading>>,
    mac_total: OnceCell<Option<u64>>,
    mac_available: OnceCell<Option<u64>>,
    stat: OnceCell<Result<String, Reading>>,
    loadavg: OnceCell<Result<(f64, u64), Reading>>,
    uptime: OnceCell<Reading>,
    thermal: OnceCell<Reading>,
    pressure: [OnceCell<Result<Pressure, Reading>>; 3],
    disk: OnceCell<Option<(u64, u64)>>,
    gpu: OnceCell<Result<GpuNow, Reading>>,
    throttle: OnceCell<Reading>,
    uplink: OnceCell<Option<String>>,
    wifi: OnceCell<Result<WifiLink, Reading>>,
    tailnet: OnceCell<Result<Tailnet, Reading>>,
    failed: OnceCell<Reading>,
}

/// On Linux a missing `/proc` file is an incident; elsewhere there is no
/// `/proc` at all.
fn core_missing(path: &str) -> Reading {
    if cfg!(target_os = "linux") {
        Reading::unread(format!("{path} is missing"))
    } else {
        Reading::NotHere
    }
}

fn ran_reading(ran: &Ran, what: &str) -> Reading {
    match ran {
        Ran::Out(_) => Reading::unread(format!("{what} answered something this build cannot read")),
        Ran::Missing => Reading::NotHere,
        Ran::Failed => Reading::unread(format!("{what} failed")),
        Ran::TimedOut => Reading::unread(format!("{what} did not answer in time")),
    }
}

impl Reader {
    pub fn new() -> Reader {
        Reader::default()
    }

    /// A reader whose commands, together, finish by `deadline` — the sampler
    /// runs several in a row inside its unit's start timeout, and a bound
    /// per command does not add up to a bound per sample.
    pub fn until(deadline: Instant) -> Reader {
        Reader {
            deadline: Some(deadline),
            ..Reader::default()
        }
    }

    /// How long the next command may take.
    pub fn budget(&self) -> Duration {
        match self.deadline {
            None => source::FORK_TIMEOUT,
            Some(d) => d
                .saturating_duration_since(Instant::now())
                .min(source::FORK_TIMEOUT),
        }
    }

    /// macOS's total, from `sysctl`, on its own: a `vm_stat` that cannot be
    /// read must not cost the total.
    fn mac_total(&self) -> Option<u64> {
        *self.mac_total.get_or_init(|| {
            match source::run_bounded("sysctl", &["-n", "hw.memsize"], self.budget()) {
                Ran::Out(t) => t.trim().parse::<u64>().ok(),
                _ => None,
            }
        })
    }

    fn mac_available(&self) -> Option<u64> {
        *self.mac_available.get_or_init(|| {
            match source::run_bounded("vm_stat", &[], self.budget()) {
                Ran::Out(t) => source::parse_vm_stat(&t),
                _ => None,
            }
        })
    }

    /// `/proc/meminfo`, parsed once. On macOS the two figures that matter
    /// come from `sysctl` and `vm_stat` instead; swap is not read there.
    pub fn meminfo(&self) -> Result<MemInfo, Reading> {
        self.meminfo
            .get_or_init(|| {
                if cfg!(target_os = "macos") {
                    return match (self.mac_total(), self.mac_available()) {
                        (Some(total), Some(available)) => Ok(MemInfo {
                            total,
                            available,
                            ..MemInfo::default()
                        }),
                        _ => Err(Reading::unread(
                            "sysctl hw.memsize or vm_stat could not be read",
                        )),
                    };
                }
                match file("/proc/meminfo") {
                    File::Text(t) => source::parse_meminfo(&t).ok_or_else(|| {
                        Reading::unread("/proc/meminfo lacks MemTotal or MemAvailable")
                    }),
                    File::Absent => Err(core_missing("/proc/meminfo")),
                    File::Unreadable(why) => Err(Reading::unread(why)),
                }
            })
            .clone()
    }

    /// `/proc/stat`'s text: the aggregate jiffies and the core lines.
    pub fn stat(&self) -> Result<String, Reading> {
        self.stat
            .get_or_init(|| match file("/proc/stat") {
                File::Text(t) => Ok(t),
                File::Absent => Err(core_missing("/proc/stat")),
                File::Unreadable(why) => Err(Reading::unread(why)),
            })
            .clone()
    }

    pub fn loadavg(&self) -> Result<(f64, u64), Reading> {
        self.loadavg
            .get_or_init(|| match file("/proc/loadavg") {
                File::Text(t) => source::parse_loadavg(&t)
                    .ok_or_else(|| Reading::unread("/proc/loadavg did not parse")),
                File::Absent => Err(core_missing("/proc/loadavg")),
                File::Unreadable(why) => Err(Reading::unread(why)),
            })
            .clone()
    }

    fn pressure(&self, i: usize) -> Result<Pressure, Reading> {
        let path = [
            "/proc/pressure/cpu",
            "/proc/pressure/memory",
            "/proc/pressure/io",
        ][i];
        self.pressure[i]
            .get_or_init(|| match file(path) {
                File::Text(t) => source::parse_pressure(&t)
                    .ok_or_else(|| Reading::unread(format!("{path} did not parse"))),
                // A kernel built without PSI has no /proc/pressure.
                File::Absent => Err(Reading::NotHere),
                File::Unreadable(why) => Err(Reading::unread(why)),
            })
            .clone()
    }

    /// The root filesystem's used and total bytes.
    pub fn disk(&self) -> Option<(u64, u64)> {
        *self.disk.get_or_init(|| source::disk_usage(Path::new("/")))
    }

    /// One `nvidia-smi --query-gpu`, shared by every GPU measurement and by
    /// the sampler. `NotHere` without the program; `answered` false when it
    /// printed nothing.
    pub fn gpu(&self) -> Result<GpuNow, Reading> {
        self.gpu
            .get_or_init(|| {
                let ran = source::nvidia_smi(
                    "--query-gpu=utilization.gpu,temperature.gpu,power.draw,memory.total",
                    self.budget(),
                );
                match &ran {
                    Ran::Out(t) => Ok(source::parse_gpu(t)),
                    other => Err(ran_reading(other, "nvidia-smi")),
                }
            })
            .clone()
    }

    /// The interface carrying the default route. Held in memory only: the
    /// series records roles, never an interface's name.
    pub fn uplink(&self) -> Option<String> {
        self.uplink
            .get_or_init(|| match file("/proc/net/route") {
                File::Text(t) => source::parse_default_route(&t),
                _ => None,
            })
            .clone()
    }

    fn wifi(&self) -> Result<WifiLink, Reading> {
        self.wifi
            .get_or_init(|| {
                let Some(iface) = self.uplink() else {
                    return Err(Reading::unread("no default route"));
                };
                if !Path::new(&format!("/sys/class/net/{iface}/wireless")).exists() {
                    return Err(Reading::NotHere);
                }
                match source::run_bounded("iw", &["dev", &iface, "link"], self.budget()) {
                    Ran::Out(t) => source::parse_iw_link(&t)
                        .ok_or_else(|| Reading::unread("the Wi-Fi link is not connected")),
                    other => Err(ran_reading(&other, "iw")),
                }
            })
            .clone()
    }

    fn tailnet(&self) -> Result<Tailnet, Reading> {
        self.tailnet
            .get_or_init(|| {
                match source::run_bounded("tailscale", &["status", "--json"], self.budget()) {
                    Ran::Out(t) => source::parse_tailscale_status(&t)
                        .ok_or_else(|| Reading::unread("tailscale status did not parse")),
                    // `tailscale status` exits non-zero while logged out or
                    // stopped; the JSON still says which on stdout, but a
                    // failure here is "down", read as an incident.
                    other => Err(ran_reading(&other, "tailscale")),
                }
            })
            .clone()
    }

    /// Read one `Now` measurement. A `Rate` or `ByCategory` measurement
    /// cannot be read in a moment and says so; the series has it.
    pub fn now(&self, m: Measurement) -> Reading {
        use Measurement::*;
        let mem = |f: fn(&MemInfo) -> Option<u64>| match self.meminfo() {
            Ok(mi) => Reading::of(f(&mi).map(|v| v as f64), "not reported on this machine"),
            Err(r) => r,
        };
        let gpu = |f: fn(&GpuNow) -> Option<f64>| match self.gpu() {
            Ok(g) if !g.answered => Reading::unread("nvidia-smi printed no GPU"),
            Ok(g) => match f(&g) {
                Some(value) => Reading::Observed { value },
                None => Reading::NotHere,
            },
            Err(r) => r,
        };
        let pressure = |i: usize, ten: bool| match self.pressure(i) {
            Ok(p) => Reading::Observed {
                value: if ten { p.avg10 } else { p.avg60 },
            },
            Err(r) => r,
        };
        match m {
            // On macOS each figure fails on its own source alone.
            MemoryTotal if cfg!(target_os = "macos") => Reading::of(
                self.mac_total().map(|v| v as f64),
                "sysctl hw.memsize could not be read",
            ),
            MemoryAvailable if cfg!(target_os = "macos") => Reading::of(
                self.mac_available().map(|v| v as f64),
                "vm_stat could not be read",
            ),
            MemoryTotal => mem(|m| Some(m.total)),
            MemoryAvailable => mem(|m| Some(m.available)),
            MemorySwapUsed if !cfg!(target_os = "linux") => Reading::NotHere,
            MemorySwapUsed => mem(|m| Some(m.swap_total.saturating_sub(m.swap_free))),
            CpuCores => match self.stat() {
                Ok(t) => Reading::of(
                    source::parse_cores(&t).map(|n| n as f64),
                    "/proc/stat lists no cores",
                ),
                Err(r) => r,
            },
            CpuLoad1m => match self.loadavg() {
                Ok((l, _)) => Reading::Observed { value: l },
                Err(r) => r,
            },
            CpuTasks => match self.loadavg() {
                Ok((_, n)) => Reading::Observed { value: n as f64 },
                Err(r) => r,
            },
            CpuTemperature => self.thermal.get_or_init(read_thermal).clone(),
            PressureCpuAvg10 => pressure(0, true),
            PressureCpuAvg60 => pressure(0, false),
            PressureMemoryAvg10 => pressure(1, true),
            PressureMemoryAvg60 => pressure(1, false),
            PressureIoAvg10 => pressure(2, true),
            PressureIoAvg60 => pressure(2, false),
            DiskRootUsed => Reading::of(self.disk().map(|d| d.0 as f64), "statvfs(\"/\") failed"),
            DiskRootTotal => Reading::of(self.disk().map(|d| d.1 as f64), "statvfs(\"/\") failed"),
            GpuUtilization => gpu(|g| g.util),
            GpuTemperature => gpu(|g| g.temp),
            GpuPower => gpu(|g| g.power_w),
            GpuUnified => gpu(|g| Some(if g.unified { 1.0 } else { 0.0 })),
            GpuThrottled => self
                .throttle
                .get_or_init(|| {
                    match source::nvidia_smi(
                        "--query-gpu=clocks_throttle_reasons.active",
                        self.budget(),
                    ) {
                        Ran::Out(t) => match source::parse_throttle(&t) {
                            Some(b) => Reading::Observed {
                                value: if b { 1.0 } else { 0.0 },
                            },
                            None => Reading::unread("nvidia-smi gave no throttle reasons"),
                        },
                        other => ran_reading(&other, "nvidia-smi"),
                    }
                })
                .clone(),
            NetworkUplinkKind => match self.uplink() {
                Some(iface) => Reading::Observed {
                    value: if Path::new(&format!("/sys/class/net/{iface}/wireless")).exists() {
                        1.0
                    } else {
                        0.0
                    },
                },
                None => Reading::unread("no default route"),
            },
            NetworkWifiSignal => match self.wifi() {
                Ok(w) => Reading::of(w.signal_dbm, "iw reported no signal"),
                Err(r) => r,
            },
            NetworkWifiBitrate => match self.wifi() {
                Ok(w) => Reading::of(w.bitrate_mbit, "iw reported no bitrate"),
                Err(r) => r,
            },
            NetworkTailnetUp => match self.tailnet() {
                Ok(t) => Reading::Observed {
                    value: if t.up { 1.0 } else { 0.0 },
                },
                Err(r) => r,
            },
            NetworkTailnetPeersOnline => match self.tailnet() {
                Ok(t) => Reading::Observed {
                    value: t.peers_online as f64,
                },
                Err(r) => r,
            },
            SystemUptime => self
                .uptime
                .get_or_init(|| match file("/proc/uptime") {
                    File::Text(t) => {
                        Reading::of(source::parse_uptime(&t), "/proc/uptime did not parse")
                    }
                    File::Absent => core_missing("/proc/uptime"),
                    File::Unreadable(why) => Reading::unread(why),
                })
                .clone(),
            ServicesFailed => self
                .failed
                .get_or_init(|| {
                    match source::run_bounded(
                        "systemctl",
                        &["--user", "--failed", "--no-legend", "--plain"],
                        self.budget(),
                    ) {
                        Ran::Out(t) => Reading::Observed {
                            value: source::parse_failed_units(&t) as f64,
                        },
                        other => ran_reading(&other, "systemctl"),
                    }
                })
                .clone(),
            // Not a catch-all for `Now` measurements: one added without a
            // reader here reads as NOT_WIRED, never as a NULL the series
            // would hold forever, and a test over every `Now` measurement
            // fails on it.
            _ if m.how() != How::Now => {
                Reading::unread("computed by the sampler from two samples; read it from the series")
            }
            _ => Reading::unread(NOT_WIRED),
        }
    }
}

/// The hottest thermal zone. No zones at all is `NotHere`; zones that all
/// fail to read is an incident.
fn read_thermal() -> Reading {
    let Ok(zones) = std::fs::read_dir("/sys/class/thermal") else {
        return Reading::NotHere;
    };
    let mut any = false;
    let mut hottest: Option<f64> = None;
    for z in zones.flatten() {
        if !z.file_name().to_string_lossy().starts_with("thermal_zone") {
            continue;
        }
        any = true;
        if let Some(c) = std::fs::read_to_string(z.path().join("temp"))
            .ok()
            .and_then(|t| source::parse_thermal(&t))
        {
            hottest = Some(hottest.map_or(c, |h: f64| h.max(c)));
        }
    }
    match (any, hottest) {
        (false, _) => Reading::NotHere,
        (true, Some(value)) => Reading::Observed { value },
        (true, None) => Reading::unread("no thermal zone gave a plausible temperature"),
    }
}
