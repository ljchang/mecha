//! `mecha work` — what runs have generated, and removing what is past.
//!
//! The human half of [`mecha_core::work`]. A producer's directory is the
//! workspace its runs are jailed to, so this is also the answer to "where did
//! the briefing go" and "what can that trigger actually see".
//!
//! `clean` is the retention policy made real. It is deliberately loud about
//! what it removed — a sweep that prints nothing is one nobody trusts enough
//! to schedule, and the nightly runs it unattended.

use anyhow::{Context, Result};
use mecha_core::work;

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// What each producer has generated (default).
    List,
    /// Print one producer's directory, creating it if absent — for `cd $(...)`
    /// and for a shell script that wants somewhere to write.
    Path { producer: String },
    /// Remove all but the newest N entries per producer.
    Clean {
        /// How many entries survive per producer. Defaults to `[work] keep`.
        #[arg(long)]
        keep: Option<usize>,
        /// Only this producer.
        #[arg(long)]
        producer: Option<String>,
        /// Say what would go, and remove nothing.
        #[arg(long)]
        dry_run: bool,
    },
}

pub async fn execute(args: Args) -> Result<()> {
    match args.cmd.unwrap_or(Cmd::List) {
        Cmd::List => list(),
        Cmd::Path { producer } => {
            println!("{}", work::ensure(&producer)?.display());
            Ok(())
        }
        Cmd::Clean {
            keep,
            producer,
            dry_run,
        } => clean(keep, producer.as_deref(), dry_run),
    }
}

fn list() -> Result<()> {
    let producers = work::list()?;
    if producers.is_empty() {
        println!(
            "no work directories yet — {} is where a run's generated output goes",
            work::root()?.display()
        );
        return Ok(());
    }
    for p in &producers {
        println!(
            "{:<24} {:>4} entr{}  {:>8}  {}",
            p.name,
            p.entries.len(),
            if p.entries.len() == 1 { "y" } else { "ies" },
            human(p.bytes),
            p.path.display()
        );
        // The newest entry is the one a later run reads back, so name it.
        if let Some(newest) = p.entries.first() {
            println!(
                "  newest  {}",
                newest
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            );
        }
    }
    Ok(())
}

fn clean(keep: Option<usize>, producer: Option<&str>, dry_run: bool) -> Result<()> {
    let keep = match keep {
        Some(k) => k,
        None => {
            let cwd = std::env::current_dir().context("cannot determine the working directory")?;
            mecha_core::config::Config::load(&cwd)?.work.keep
        }
    };
    let report = work::clean(keep, producer, dry_run)?;
    if report.removed.is_empty() && report.protected.is_empty() {
        println!("nothing to remove: every producer is within the last {keep}");
        return Ok(());
    }
    for entry in &report.removed {
        println!(
            "{} {}",
            if dry_run { "would remove" } else { "removed" },
            entry.path.display()
        );
    }
    // Named rather than silent: an entry that survives its own retention
    // window reads as a bug in the sweep unless the reason is on screen.
    for entry in &report.protected {
        println!(
            "kept (a published bundle names it as a source) {}",
            entry.path.display()
        );
    }
    println!(
        "{} {} entr{}, {}",
        if dry_run { "would free" } else { "freed" },
        report.removed.len(),
        if report.removed.len() == 1 {
            "y"
        } else {
            "ies"
        },
        human(report.bytes_removed())
    );
    Ok(())
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
