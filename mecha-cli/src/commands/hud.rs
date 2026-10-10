//! `mecha hud` — dashboards over the owner's own data.
//!
//! Installing is the owner's act and refreshing is a timer's: `refresh --due`
//! is the primitive a systemd timer runs, and due-ness is a function of each
//! board's ledger and the clock, so a timer that was asleep reaches the same
//! answer as one that never stopped (`scripts/mecha-hud.timer`). No model runs
//! anywhere in this command. `docs/LIVE-DASHBOARD-DESIGN.md` is the authority.

use std::path::PathBuf;

use anyhow::{bail, Result};
use chrono::Utc;
use mecha_core::hud::store::{Event, Outcome, Store, Which};
use mecha_core::hud::Refusals;

use crate::GlobalOpts;

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Installed boards, and how each loader's last refresh went.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Check a draft directory (hud.json + loaders/) without installing it.
    Validate {
        dir: PathBuf,
        /// The id it would be installed under; defaults to the directory's name.
        #[arg(long)]
        id: Option<String>,
    },
    /// Install a draft as a board. Refuses an id already installed.
    Install {
        dir: PathBuf,
        /// Install under this id instead of the directory's name.
        #[arg(long)]
        id: Option<String>,
    },
    /// Record one sample of the host's load, by kind of work only, into
    /// ~/.mecha/hud/host.sqlite — what `scripts/mecha-hud-sample.timer` runs
    /// each minute. No process, unit or model name is ever written.
    Sample {
        #[arg(long)]
        json: bool,
    },
    /// Refresh datasets: every loader of one board (or of all), now — or,
    /// with --due, only the loaders whose schedule says so, which is what the
    /// timer runs.
    Refresh {
        id: Option<String>,
        #[arg(long)]
        due: bool,
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(_global: &GlobalOpts, args: Args) -> Result<()> {
    let store = Store::open()?;
    match args.cmd {
        Cmd::List { json } => list(&store, json),
        Cmd::Validate { dir, id } => {
            let id = id_for(&dir, id)?;
            match store.validate(&dir, &id)? {
                Ok(installed) => {
                    println!(
                        "ok: {:?} — {} panel(s), {} loader(s)",
                        installed.spec().title(),
                        installed.spec().panels().len(),
                        installed.loaders().len()
                    );
                    Ok(())
                }
                Err(r) => refused(&r),
            }
        }
        Cmd::Install { dir, id } => match store.install(&dir, id.as_deref(), Utc::now())? {
            Ok(installed) => {
                println!(
                    "installed {:?} as {} — run `mecha hud refresh {}` to fill it now",
                    installed.spec().title(),
                    installed.id(),
                    installed.id()
                );
                Ok(())
            }
            Err(r) => refused(&r),
        },
        Cmd::Refresh { id, due, json } => refresh(&store, id, due, json),
        Cmd::Sample { json } => sample(json),
    }
}

fn id_for(dir: &std::path::Path, id: Option<String>) -> Result<String> {
    match id {
        Some(id) => Ok(id),
        None => Ok(dir
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow::anyhow!("the directory needs a name, or pass --id"))?
            .to_string()),
    }
}

fn refused(r: &Refusals) -> Result<()> {
    eprintln!("{r}");
    bail!("refused: {} problem(s)", r.0.len())
}

fn list(store: &Store, json: bool) -> Result<()> {
    let boards = store.status(Utc::now())?;
    if json {
        println!("{}", serde_json::to_string_pretty(&boards)?);
        return Ok(());
    }
    if boards.is_empty() {
        println!("no boards installed — `mecha hud install <dir>`");
        return Ok(());
    }
    for b in boards {
        match &b.title {
            Some(title) => println!("{}  {title}", b.id),
            None => println!("{}  (does not load)", b.id),
        }
        for r in &b.refusals {
            println!("    refused: {r}");
        }
        for l in &b.loaders {
            let last = match l.last.as_ref().map(|e| &e.event) {
                None => "never refreshed".to_string(),
                Some(Event::Refreshed { rows, .. }) => format!("refreshed, {rows} row(s)"),
                Some(Event::Refused { reason, .. }) => format!("REFUSED — {reason}"),
                Some(Event::Failed { reason, .. }) => format!("FAILED — {reason}"),
                Some(Event::Installed) => "installed".to_string(),
            };
            let stale = if l.stale { "  [stale]" } else { "" };
            println!("    {} ({}): {last}{stale}", l.name, l.source);
        }
    }
    Ok(())
}

fn refresh(store: &Store, id: Option<String>, due: bool, json: bool) -> Result<()> {
    let which = if due { Which::Due } else { Which::All };
    let ids = match id {
        Some(id) => vec![id],
        None => store.board_ids()?,
    };
    let now = Utc::now();
    let mut outcomes: Vec<Outcome> = Vec::new();
    let mut broken = 0usize;
    for id in ids {
        // One board's trouble — refused, or an error reading it — is counted
        // and printed, and the boards after it still refresh.
        match store.refresh(&id, now, which) {
            Ok(Ok(mut o)) => outcomes.append(&mut o),
            Ok(Err(r)) => {
                broken += 1;
                eprintln!("board {id} does not load:\n{r}");
            }
            Err(e) => {
                broken += 1;
                eprintln!("board {id} could not be refreshed: {e:#}");
            }
        }
    }
    let failed = outcomes
        .iter()
        .filter(|o| !matches!(o.event, Event::Refreshed { .. }))
        .count();
    if json {
        println!("{}", serde_json::to_string_pretty(&outcomes)?);
    } else {
        for o in &outcomes {
            match &o.event {
                Event::Refreshed {
                    rows, generation, ..
                } => {
                    println!(
                        "{}/{}: {rows} row(s), generation {generation}",
                        o.board, o.loader
                    )
                }
                Event::Refused { reason, .. } => {
                    println!("{}/{}: REFUSED — {reason}", o.board, o.loader)
                }
                Event::Failed { reason, .. } => {
                    println!("{}/{}: FAILED — {reason}", o.board, o.loader)
                }
                Event::Installed => {}
            }
        }
        if outcomes.is_empty() && broken == 0 && due {
            // The timer's common case, said once so a journal reader knows it ran.
            println!("nothing due");
        }
    }
    if broken + failed > 0 {
        bail!(
            "{} board(s) did not load, {failed} loader(s) did not refresh",
            broken
        );
    }
    Ok(())
}

fn sample(json: bool) -> Result<()> {
    use mecha_core::hud::host;
    let recorded = host::record(&host::db_path()?, &host::collect(Utc::now())?)?;
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
