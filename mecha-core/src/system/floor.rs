//! The memory floor: admit before allocating (`docs/SYSTEM-STATE-DESIGN.md`
//! §3.7, rulings R5).
//!
//! On this class of machine the GPU and everything else draw from one pool,
//! and an out-of-memory is a discrete event — a model loaded beside another,
//! an oversized build, a picture started beside a resident chat model. So
//! prevention is a **refusal at admission**, before the allocation, never a
//! controller running beside it. Something about to allocate states what it
//! needs; [`admit`] says yes or no, and the caller refuses.
//!
//! - **The floor is `need + reserve`.** The reserve is headroom kept for what
//!   runs uninvited — the voice pipeline mid-call, the page cache, the next
//!   turn's KV growth. R5: 8 GiB to start, tuned from the series.
//! - **Pressure refuses regardless of headroom.** A machine can show free
//!   bytes while it thrashes; work stalled on reclaim for more than the
//!   ceiling's share of the last ten seconds (R5: 10%) is a no.
//! - **Unknown is never a pass.** Available memory that cannot be read
//!   refuses, exactly as the image tool's gate always has. A kernel without
//!   PSI has no pressure to read (`NotHere`), and only the headroom is asked;
//!   pressure that exists and cannot be read refuses.
//! - **It kills nothing, reclaims nothing, and decides nothing itself.**
//!   [`admit`] is a pure function of two readings and a need; the caller
//!   refuses. A refusal names the largest holders by category, so the owner
//!   can see what to stop — never a process name.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{Measurement, Probed, Reader, Reading};

/// `[system]` in the global config. Global-file only: a cloned repository's
/// `mecha.toml` must not be able to lower a safety floor (`merge_file`
/// strips it from project layers).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SystemConfig {
    /// Headroom kept back beyond any stated need, in MB. R5: 8 GiB, about
    /// one voice pipeline plus margin on the GB10.
    pub memory_reserve_mb: u64,
    /// Refuse when `pressure.memory.avg10` is above this percentage, however
    /// much is free. R5: 10.
    pub memory_pressure_ceiling: f64,
}

impl Default for SystemConfig {
    fn default() -> SystemConfig {
        SystemConfig {
            memory_reserve_mb: 8 * 1024,
            memory_pressure_ceiling: 10.0,
        }
    }
}

const MIB: u64 = 1024 * 1024;

/// What the floor said.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "admit", rename_all = "snake_case")]
pub enum Admit {
    Yes {
        available: u64,
        floor: u64,
    },
    /// Not enough headroom for the need plus the reserve.
    Short {
        available: u64,
        need: u64,
        reserve: u64,
        holders: Vec<(&'static str, u64)>,
    },
    /// Enough bytes, but the machine is already stalling on reclaim.
    Pressure {
        pressure: f64,
        ceiling: f64,
        holders: Vec<(&'static str, u64)>,
    },
    /// The gauge could not be read: a refusal, never a pass.
    Unread {
        why: String,
    },
}

impl Admit {
    pub fn admitted(&self) -> bool {
        matches!(self, Admit::Yes { .. })
    }

    /// The refusal as a sentence for a person or a model; `what` is the
    /// thing that was not started ("a generation", "the build").
    pub fn sentence(&self, what: &str) -> Option<String> {
        let gb = |b: u64| b as f64 / (1024.0 * MIB as f64);
        let holding = |holders: &[(&'static str, u64)]| {
            if holders.is_empty() {
                return String::new();
            }
            let list: Vec<String> = holders
                .iter()
                .map(|(c, b)| format!("{c} {:.1} GB", gb(*b)))
                .collect();
            format!(
                " Holding the most as of the last sample: {}.",
                list.join(", ")
            )
        };
        match self {
            Admit::Yes { .. } => None,
            Admit::Short {
                available,
                need,
                reserve,
                holders,
            } => Some(format!(
                "Only {:.1} GB of memory is available, and {what} needs about {:.1} GB plus the \
                 {:.1} GB kept free for everything else, so it was not started — this machine \
                 shares one memory pool between the GPU and everything else.{}",
                gb(*available),
                gb(*need),
                gb(*reserve),
                holding(holders)
            )),
            Admit::Pressure {
                pressure,
                ceiling,
                holders,
            } => Some(format!(
                "This machine is short of memory right now — work stalled waiting on it for \
                 {pressure:.0}% of the last ten seconds, above the {ceiling:.0}% ceiling — so \
                 {what} was not started.{}",
                holding(holders)
            )),
            // Either gauge: `why` names which (available memory, or
            // `memory pressure: ...`).
            Admit::Unread { why } => Some(format!(
                "Could not read this machine's memory ({why}), so {what} was not started: \
                 an unreadable gauge is not an empty tank."
            )),
        }
    }
}

/// The floor's answer for `need` bytes, from a reading of available memory
/// and one of memory pressure. Pure: no I/O, no clock.
pub fn admit(available: &Reading, pressure: &Reading, need: u64, cfg: &SystemConfig) -> Admit {
    let available = match available {
        Reading::Observed { value } => *value as u64,
        Reading::Unread { why } => return Admit::Unread { why: why.clone() },
        Reading::NotHere => {
            return Admit::Unread {
                why: "this machine reports no available memory".into(),
            }
        }
    };
    match pressure {
        Reading::Observed { value } if *value > cfg.memory_pressure_ceiling => {
            return Admit::Pressure {
                pressure: *value,
                ceiling: cfg.memory_pressure_ceiling,
                holders: Vec::new(),
            }
        }
        Reading::Unread { why } => {
            return Admit::Unread {
                why: format!("memory pressure: {why}"),
            }
        }
        _ => {}
    }
    let reserve = cfg.memory_reserve_mb.saturating_mul(MIB);
    let floor = need.saturating_add(reserve);
    if available < floor {
        return Admit::Short {
            available,
            need,
            reserve,
            holders: Vec::new(),
        };
    }
    Admit::Yes { available, floor }
}

/// [`admit`] against the machine now, with a refusal naming the largest
/// holders from the series' newest minute (when the sampler is running).
pub fn admit_now(need: u64, cfg: &SystemConfig, series: Option<&Path>) -> Admit {
    let reader = Reader::new();
    let mut verdict = admit(
        &reader.now(Measurement::MemoryAvailable),
        &reader.now(Measurement::PressureMemoryAvg10),
        need,
        cfg,
    );
    if let Admit::Short { holders, .. } | Admit::Pressure { holders, .. } = &mut verdict {
        if let Some(db) = series {
            *holders = largest_holders(&reader, db, chrono::Utc::now());
        }
    }
    verdict
}

/// The three categories holding the most memory, above a gigabyte, from the
/// series' newest minute. Categories only: the refusal never names a
/// process. `other` is left out: it is the remainder — the kernel, the page
/// cache, whatever no unit owns — and names nothing the owner could stop.
pub(super) fn largest_holders(
    reader: &Reader,
    db: &Path,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<(&'static str, u64)> {
    let Probed::ByCategory(rows) = super::probe(reader, db, Measurement::CategoryMemory, now)
    else {
        return Vec::new();
    };
    let mut held: Vec<(&'static str, u64)> = rows
        .into_iter()
        .filter(|(c, _)| *c != super::Category::Other.label())
        .filter_map(|(c, r)| r.value().map(|v| (c, v as u64)))
        .filter(|(_, b)| *b >= 1024 * MIB)
        .collect();
    held.sort_by_key(|h| std::cmp::Reverse(h.1));
    held.truncate(3);
    held
}

/// A size as a person writes it: `24G`, `512M`, `20480` (MB), `1.5G`.
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, mult) = match s.char_indices().last()? {
        (i, 'G' | 'g') => (&s[..i], 1024 * MIB),
        (i, 'M' | 'm') => (&s[..i], MIB),
        _ => (s, MIB),
    };
    let v: f64 = num.trim().parse().ok()?;
    (v.is_finite() && v >= 0.0).then_some((v * mult as f64) as u64)
}
