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
    /// Stage a board's release on the factory in the outbox: the page, the
    /// renderer, the checked spec and each loader's columns and digest. The
    /// owner's release is the review (`mecha outbox show`, then `send`);
    /// after it, every refresh pushes the board's datasets.
    Publish {
        id: String,
        /// Readable by anyone with the link. Without it the dashboard is
        /// private: the owner, and whoever they share it with.
        #[arg(long)]
        public: bool,
    },
    /// Refresh datasets: every loader of one board (or of all), now — or,
    /// with --due, only the loaders whose schedule says so, which is what the
    /// timer runs. A published board's datasets are then pushed.
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
        Cmd::Publish { id, public } => publish(&store, &id, public),
        Cmd::Refresh { id, due, json } => refresh(&store, id, due, json),
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
    let refreshed_ids = ids.clone();
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
    let unpushed = push_published(store, &refreshed_ids, json)?;
    if broken + failed + unpushed > 0 {
        bail!(
            "{} board(s) did not load, {failed} loader(s) did not refresh, \
             {unpushed} dataset(s) could not be pushed",
            broken
        );
    }
    Ok(())
}

/// The outbox tool that publishes a bundle: the factory's `bundle_publish`,
/// as `[outbox] publish_tools` routes it. Never a guess at a server's name.
fn publish_tool(cfg: &mecha_core::config::Config) -> Result<String> {
    cfg.outbox
        .publish_tools
        .iter()
        .find(|t| t.ends_with("__bundle_publish"))
        .cloned()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no factory `bundle_publish` is routed through the outbox — \
                 `[outbox] publish_tools` must name it (e.g. \"factory__bundle_publish\")"
            )
        })
}

fn publish(store: &Store, id: &str, public: bool) -> Result<()> {
    use mecha_core::hud::publish;
    let installed = match store.load(id)? {
        Ok(installed) => installed,
        Err(r) => return refused(&r),
    };
    let cfg = mecha_core::config::Config::load_global()?;
    let tool = publish_tool(&cfg)?;
    let assets = match cfg.web.assets.clone() {
        Some(dir) => dir,
        None => mecha_core::work::mecha_home()?.join("web").join("dist"),
    };
    let out = mecha_core::work::producer_dir("hud")?
        .join(format!("{id}-{}", Utc::now().format("%Y%m%dT%H%M%S%3f")));
    publish::assemble(&installed, &assets.join(publish::RENDERER_DIR), &out)?;
    let bundle = publish::bundle_id(id);
    // The store `mecha outbox show` and `send` open, resolved the same way.
    let outbox = crate::commands::outbox::open_store()?;
    let item = outbox.stage_publish_by_harness(
        &tool,
        serde_json::json!({
            "id": bundle,
            "bundle": out.display().to_string(),
            "title": installed.spec().title(),
            "visibility": if public { "public" } else { "private" },
        }),
    )?;
    publish::mark_published(&store.board_path(id)?, &bundle)?;
    println!(
        "staged {:?} for the factory as `{bundle}` ({}) — outbox item {}",
        installed.spec().title(),
        if public { "public" } else { "private" },
        item.id
    );
    println!(
        "  review it:  the web outbox, or `mecha outbox show {}`",
        item.id
    );
    println!("  release it: mecha outbox send {}", item.id);
    println!("  after the release, every refresh pushes its datasets");
    Ok(())
}

/// Push every published board's datasets that changed since the last push.
/// How many could not be pushed — refused, or failed in a way worth retrying
/// next tick. A board not yet released is skipped quietly: its outbox item is
/// still waiting on the owner.
fn push_published(store: &Store, ids: &[String], json: bool) -> Result<usize> {
    use mecha_core::hud::publish::{self, Plan, Pushed};
    let bundles = mecha_core::work::bundles_root()?;
    // `--json`'s stdout is the refresh outcomes and nothing else.
    let say = |line: String| {
        if json {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    };
    let mut unpushed = 0usize;
    for id in ids {
        let Ok(dir) = store.board_path(id) else {
            continue;
        };
        // One board's trouble is counted and said, and the boards after it
        // still push, as the refresh loop above treats a board (review of
        // #651).
        let published = match publish::published(&dir) {
            Ok(Some(published)) => published,
            Ok(None) => continue,
            Err(e) => {
                eprintln!("{id}: its publish record is unreadable: {e:#}");
                unpushed += 1;
                continue;
            }
        };
        let release = match publish::release_at(&bundles, &published.bundle) {
            Ok(Some(release)) => release,
            Ok(None) => continue,
            Err(e) => {
                eprintln!("{id}: what was released could not be read: {e:#}");
                unpushed += 1;
                continue;
            }
        };
        let installed = match store.load(id) {
            Ok(Ok(installed)) => installed,
            Ok(Err(_)) => continue,
            Err(e) => {
                eprintln!("{id}: could not be read to push: {e:#}");
                unpushed += 1;
                continue;
            }
        };
        let mut pushed = match Pushed::read(&dir) {
            Ok(pushed) => pushed,
            Err(e) => {
                eprintln!(
                    "{id}: the record of what was pushed is unreadable, so nothing is pushed \
                     ({e:#}); if it is lost, `factory-publish dataset reset {}` starts over",
                    published.bundle
                );
                unpushed += 1;
                continue;
            }
        };
        for name in installed.loaders().keys() {
            let dataset = match store.dataset(id, name) {
                Ok(Some(dataset)) => dataset,
                Ok(None) => continue,
                Err(e) => {
                    eprintln!("{id}/{name}: could not be read to push: {e:#}");
                    unpushed += 1;
                    continue;
                }
            };
            let body = match publish::plan(
                &installed,
                name,
                &dataset,
                &release,
                pushed.last(release.version, name),
            ) {
                Plan::Push(body) => body,
                Plan::Pushed => continue,
                Plan::Undeclared => {
                    eprintln!(
                        "{id}/{name}: not in release {} — publish the board again to add it",
                        release.version
                    );
                    // Never pushed until republished: the timer says so.
                    unpushed += 1;
                    continue;
                }
                Plan::LoaderChanged => {
                    eprintln!(
                        "{id}/{name}: its loader changed since release {} and is not pushed \
                         — publish the board again (`mecha hud publish {id}`)",
                        release.version
                    );
                    unpushed += 1;
                    continue;
                }
            };
            match push_one(&published.bundle, name, &body) {
                Ok(outcome) => match outcome["outcome"].as_str() {
                    Some("stored") | Some("unchanged") => {
                        pushed.record(release.version, name, dataset.generation);
                        say(format!(
                            "{id}/{name}: pushed, generation {}",
                            dataset.generation
                        ));
                    }
                    // The factory's release moved past the one home recorded
                    // (released from another machine, or the account page):
                    // the box drops the push, and the dashboard would freeze
                    // behind a timer that looked fine, so it counts.
                    Some("stale") => {
                        eprintln!(
                            "{id}/{name}: the factory is past release {} — publish again \
                             (`mecha hud publish {id}`) so home records the current one: {}",
                            release.version,
                            outcome["message"].as_str().unwrap_or_default()
                        );
                        unpushed += 1;
                    }
                    _ => {
                        eprintln!(
                            "{id}/{name}: the factory refused the push ({}): {}",
                            outcome["code"].as_str().unwrap_or("?"),
                            outcome["message"].as_str().unwrap_or_default()
                        );
                        unpushed += 1;
                    }
                },
                Err(e) => {
                    eprintln!("{id}/{name}: not pushed, retried next refresh: {e:#}");
                    unpushed += 1;
                }
            }
        }
        if let Err(e) = pushed.write(&dir) {
            eprintln!("{id}: what was pushed could not be recorded: {e:#}");
            unpushed += 1;
        }
    }
    Ok(unpushed)
}

/// One push, through the factory client: its outcome line, or an error worth
/// retrying (the network, a 5xx, the client missing).
fn push_one(bundle: &str, name: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
    use std::io::Write;
    let mut child = std::process::Command::new("factory-publish")
        .args(["dataset", "push", bundle, name, "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("running factory-publish: {e}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("no stdin to factory-publish"))?
        .write_all(body.to_string().as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!(
            "factory-publish exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let line = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(line.trim())
        .map_err(|e| anyhow::anyhow!("factory-publish answered {line:?}: {e}"))
}
