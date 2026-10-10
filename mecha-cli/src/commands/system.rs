//! `mecha system` — the state of the machine mecha runs on, read in one place
//! (`docs/SYSTEM-STATE-DESIGN.md`). No model runs anywhere in this command.
//!
//! S0 carries the per-minute sampler; reading and probing named measurements
//! arrive with the probe API (S1, S3).

use anyhow::Result;
use chrono::Utc;
use mecha_core::system;

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
}

pub async fn run(_global: &GlobalOpts, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::Sample { json } => sample(json),
    }
}

fn sample(json: bool) -> Result<()> {
    let recorded = system::record(&system::db_path()?, &system::collect(Utc::now())?)?;
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
