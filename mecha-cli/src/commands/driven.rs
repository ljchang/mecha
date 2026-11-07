//! `mecha driven` — the home-side markers a driven task's setup writes
//! (`docs/DRIVER-DESIGN.md` §2, D1b).
//!
//! A marker names one task clone and a deadline, and sits under the mecha
//! home, outside every jail, so the run it covers cannot write it. While one
//! exists, a run **not** marked driven whose workspace is inside its clone is
//! refused, expired or not: that is the case of a driven task whose mark was
//! lost, which would otherwise run with the owner's servers, hooks, grants
//! and mailbox attached. The deadline governs only the driven store's load
//! (D2), where an expired marker loading nothing is the safe direction.
//!
//! Task setup, the harness and not the model, writes one per task and
//! removes it at teardown. Nothing here starts a run.

use anyhow::Result;
use mecha_core::driven::Marker;
use std::path::PathBuf;

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// The markers under this home, and whether each is live (default).
    List,
    /// Write a task's marker, covering its clone until the deadline.
    Mark {
        /// The task's name: lowercase letters, digits, `-` and `_`.
        task: String,
        /// The task clone the marker covers. Must exist, and must not
        /// overlap the mecha home.
        #[arg(long)]
        clone: PathBuf,
        /// How long the marker stays live: `90s`, `30m`, `2h`, `1d`.
        #[arg(long = "for", value_name = "DURATION", default_value = "4h")]
        duration: String,
    },
    /// Remove a task's marker, at teardown.
    Unmark { task: String },
}

pub async fn execute(args: Args) -> Result<()> {
    let home = mecha_core::work::mecha_home()?;
    match args.cmd.unwrap_or(Cmd::List) {
        Cmd::List => {
            let now = chrono::Utc::now();
            let markers = Marker::all(&home)?;
            if markers.is_empty() {
                println!("no driven markers");
            }
            for m in markers {
                println!(
                    "{}  {}  {} {}",
                    m.task,
                    m.clone.display(),
                    if m.live(now) { "live until" } else { "expired" },
                    m.deadline.format("%Y-%m-%dT%H:%M:%SZ")
                );
            }
            Ok(())
        }
        Cmd::Mark {
            task,
            clone,
            duration,
        } => {
            let span = mecha_core::trigger::parse_duration(&duration)
                .map_err(|e| anyhow::anyhow!("--for {duration:?}: {e}"))?;
            let m = Marker::write(&home, &task, &clone, chrono::Utc::now() + span)?;
            println!(
                "marked `{}`: {} until {}",
                m.task,
                m.clone.display(),
                m.deadline.format("%Y-%m-%dT%H:%M:%SZ")
            );
            Ok(())
        }
        Cmd::Unmark { task } => {
            if Marker::remove(&home, &task)? {
                println!("unmarked `{task}`");
            } else {
                println!("no marker for `{task}`");
            }
            Ok(())
        }
    }
}
