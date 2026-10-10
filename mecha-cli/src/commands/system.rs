//! `mecha system` — the state of the machine mecha runs on, read in one place
//! (`docs/SYSTEM-STATE-DESIGN.md`). No model runs anywhere in this command.
//!
//! `sample` is what the per-minute timer runs; `probe` reads named
//! measurements — now from the machine, or a rate from the series' newest
//! minute. Slicing the series over time (`read`) arrives in S3.

use anyhow::{bail, Result};
use chrono::Utc;
use mecha_core::system::{self, measure::Unit, Measurement, Probed, Reader, Reading};

use crate::GlobalOpts;

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Record one minute of the host's load, by kind of work only, into
    /// ~/.mecha/system/series.sqlite — what `scripts/mecha-system-sample.timer`
    /// runs each minute. No process, unit or model name is ever written; a
    /// sample that cannot be read is refused and exits non-zero.
    Sample {
        #[arg(long)]
        json: bool,
    },
    /// Read measurements now: one by name (`memory.available`), a group by
    /// prefix (`gpu`, `pressure.memory`), or every one when none is named.
    /// A rate (`disk.read_rate`, `cpu.busy`) comes from the series' newest
    /// minute. Unknown is shown as unknown, with why — never as zero.
    Probe {
        /// A measurement's name or a group prefix.
        name: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(_global: &GlobalOpts, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::Sample { json } => sample(json),
        Cmd::Probe { name, json } => probe(name.as_deref(), json),
    }
}

fn probe(name: Option<&str>, json: bool) -> Result<()> {
    let wanted = match name {
        None => Measurement::ALL.to_vec(),
        Some(q) => {
            let m = Measurement::matching(q);
            if m.is_empty() {
                let names: Vec<&str> = Measurement::ALL.iter().map(|m| m.name()).collect();
                bail!(
                    "no measurement is named or grouped {q:?}; the names are:\n  {}",
                    names.join("\n  ")
                );
            }
            m
        }
    };
    let db = system::db_path()?;
    let reader = Reader::new();
    let now = Utc::now();
    let rows: Vec<(Measurement, Probed)> = wanted
        .into_iter()
        .map(|m| (m, system::probe(&reader, &db, m, now)))
        .collect();
    if json {
        let out: Vec<_> = rows
            .iter()
            .map(|(m, p)| serde_json::json!({ "name": m.name(), "unit": m.unit(), "reading": p }))
            .collect();
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    for (m, p) in &rows {
        match p {
            Probed::One(r) => println!("{:<32} {}", m.name(), show(m.unit(), r)),
            Probed::ByCategory(cats) => {
                println!("{}", m.name());
                for (label, r) in cats {
                    println!("  {label:<30} {}", show(m.unit(), r));
                }
            }
        }
    }
    Ok(())
}

/// A reading for a person: the value in its unit, or what stands in for one.
fn show(unit: Unit, r: &Reading) -> String {
    let v = match r {
        Reading::Observed { value } => *value,
        Reading::Unread { why } => return format!("unknown — {why}"),
        Reading::NotHere => return "not on this machine".to_string(),
    };
    const GIB: f64 = 1073741824.0;
    match unit {
        Unit::Bytes => format!("{:.2} GiB", v / GIB),
        Unit::BytesPerSec if v >= 1048576.0 => format!("{:.1} MiB/s", v / 1048576.0),
        Unit::BytesPerSec => format!("{:.1} KiB/s", v / 1024.0),
        Unit::Percent => format!("{v:.1}%"),
        Unit::Count | Unit::Load => format!("{v}"),
        Unit::Celsius => format!("{v:.1} °C"),
        Unit::Watts => format!("{v:.1} W"),
        Unit::Seconds => format!("{:.1} days", v / 86400.0),
        Unit::Dbm => format!("{v:.0} dBm"),
        Unit::Mbits => format!("{v:.1} Mbit/s"),
        Unit::Flag => (if v != 0.0 { "yes" } else { "no" }).to_string(),
        Unit::Label(labels) => labels
            .get(v as usize)
            .map_or_else(|| format!("unknown label {v}"), |l| l.to_string()),
    }
}

fn sample(json: bool) -> Result<()> {
    let db = system::db_path()?;
    let since = system::last_sample_at(&db);
    let recorded = system::record(&db, &system::collect(Utc::now(), since)?)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&recorded)?);
    } else {
        let cpu = recorded
            .cpu_pct
            .map_or("—".to_string(), |p| format!("{p:.1}%"));
        println!("{}  cpu {cpu}", recorded.at);
        for c in &recorded.by_category {
            let pct = c.cpu_pct.map_or("—".to_string(), |p| format!("{p:.1}%"));
            let mem = c.mem_bytes.map_or("—".to_string(), |m| {
                format!("{:.2} GiB", m as f64 / 1073741824.0)
            });
            println!("  {:<17} {mem:>11}  cpu {pct}", c.category);
        }
    }
    Ok(())
}
