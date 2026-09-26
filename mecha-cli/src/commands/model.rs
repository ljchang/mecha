//! `mecha model` — what the local model router can serve, and choosing which
//! one it holds (`REMOTE-SURFACE-DESIGN.md` §14, D12).
//!
//! The owner's side of `provider::router`. **Loading a model is the pick** —
//! there is no setting to write, because every default run asks the router
//! what it has loaded. This is reached by the owner only: this command, the
//! TUI's `/model` picker, the web chip through `mecha serve`. There is no
//! tool; a model never chooses the model.

use crate::GlobalOpts;
use anyhow::{bail, Result};
use mecha_core::config::Config;
use mecha_core::provider::router;
use serde::Serialize;
use std::time::{Duration, Instant};

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
    /// Machine-readable output.
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Each router's models, which one is loaded, and the provider entry
    /// naming each (default).
    List,
    /// Load a model — by provider entry name or by the router's model name —
    /// and wait until it is resident. The model it replaces finishes any
    /// request in flight first; every run after this one follows it.
    Use {
        name: String,
        /// Give up after this many seconds. A cold load from disk measured
        /// 33–39 s on 2026-09-26; the unit allows 600.
        #[arg(long, default_value_t = 600)]
        wait_secs: u64,
    },
}

#[derive(Serialize)]
struct Router {
    base_url: String,
    /// `None` when nothing is loaded.
    resident: Option<String>,
    models: Vec<Model>,
    /// Entries pointing at this router whose model it does not serve: every
    /// run on one is a 400 (`model '…' not found`).
    unserved: Vec<String>,
}

#[derive(Serialize)]
struct Model {
    id: String,
    status: String,
    /// The provider entries naming this model on this router. One is the
    /// working case; none means a run can never choose it by default.
    providers: Vec<String>,
}

fn load_config(global: &GlobalOpts) -> Result<Config> {
    if global.global_config_only {
        Config::load_global()
    } else {
        Config::load(&std::env::current_dir()?)
    }
}

/// Every base URL a `follow_loaded` entry points at: the routers the owner
/// has said stand for "whatever is loaded".
fn router_bases(cfg: &Config) -> Vec<String> {
    let mut b: Vec<String> = cfg
        .providers
        .values()
        .filter(|p| p.follow_loaded)
        .filter_map(|p| p.base_url.as_deref().map(router::base))
        .collect();
    b.sort();
    b.dedup();
    b
}

async fn survey(cfg: &Config) -> Vec<(String, Option<Router>)> {
    let mut out = Vec::new();
    for base in router_bases(cfg) {
        let Some(list) = router::models(&base).await else {
            out.push((base, None));
            continue;
        };
        let served: Vec<&str> = list.iter().map(|m| m.id.as_str()).collect();
        let unserved = cfg
            .providers
            .iter()
            .filter(|(_, p)| p.base_url.as_deref().map(router::base).as_deref() == Some(&base))
            .filter(|(_, p)| p.model.as_deref().is_some_and(|m| !served.contains(&m)))
            .map(|(n, _)| n.clone())
            .collect();
        let r = Router {
            resident: router::resident(&list).map(str::to_string),
            models: list
                .iter()
                .map(|m| Model {
                    id: m.id.clone(),
                    status: m.status.value.clone(),
                    providers: router::namers(cfg, &base, &m.id)
                        .map(str::to_string)
                        .collect(),
                })
                .collect(),
            unserved,
            base_url: base.clone(),
        };
        out.push((base, Some(r)));
    }
    out
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let cfg = load_config(global)?;
    match args.cmd.unwrap_or(Cmd::List) {
        Cmd::List => list(&cfg, args.json).await,
        Cmd::Use { name, wait_secs } => use_(&cfg, &name, wait_secs, args.json).await,
    }
}

async fn list(cfg: &Config, json: bool) -> Result<()> {
    let routers = survey(cfg).await;
    if json {
        let found: Vec<&Router> = routers.iter().filter_map(|(_, r)| r.as_ref()).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "routers": found }))?
        );
        return Ok(());
    }
    if routers.is_empty() {
        println!(
            "no provider has `follow_loaded = true`, so no router stands for \"whatever is \
             loaded\" — see REMOTE-SURFACE-DESIGN §14 and scripts/start-router.sh"
        );
        return Ok(());
    }
    for (base, r) in routers {
        let Some(r) = r else {
            println!("{base}: not a llama-server router, or not up");
            continue;
        };
        println!("{base}");
        for m in &r.models {
            let here = if r.resident.as_deref() == Some(m.id.as_str()) {
                "●"
            } else {
                " "
            };
            let who = match m.providers.as_slice() {
                [] => "(no provider entry)".to_string(),
                ps => ps.join(", "),
            };
            println!("  {here} {:<30} {:<9} {who}", m.id, m.status);
        }
        for n in &r.unserved {
            println!("  ! [providers.{n}] names a model this router does not serve");
        }
    }
    Ok(())
}

async fn use_(cfg: &Config, name: &str, wait_secs: u64, json: bool) -> Result<()> {
    let bases = router_bases(cfg);
    // A provider entry on one of the routers, then a model id one serves.
    let target = match cfg.providers.get(name) {
        Some(p) => {
            let base = p.base_url.as_deref().map(router::base);
            match (base, p.model.clone()) {
                (Some(b), Some(m)) if bases.contains(&b) => Some((b, m)),
                _ => bail!(
                    "[providers.{name}] is not on a router any `follow_loaded` entry points at, \
                     so loading it would not be followed — name it with `--provider {name}` instead"
                ),
            }
        }
        None => {
            let mut hit = None;
            for b in &bases {
                if router::models(b)
                    .await
                    .is_some_and(|l| l.iter().any(|m| m.id == name))
                {
                    hit = Some((b.clone(), name.to_string()));
                    break;
                }
            }
            hit
        }
    };
    let Some((base, model)) = target else {
        bail!(
            "no provider entry or router model named {name:?} — `mecha model list` shows what \
             can be loaded"
        );
    };
    let started = Instant::now();
    router::load(&base, &model, Duration::from_secs(wait_secs)).await?;
    let secs = started.elapsed().as_secs_f64();
    let providers: Vec<String> = router::namers(cfg, &base, &model)
        .map(str::to_string)
        .collect();
    if json {
        println!(
            "{}",
            serde_json::json!({ "base_url": base, "model": model, "providers": providers, "seconds": secs })
        );
    } else {
        println!("{model} is loaded at {base} ({secs:.1}s)");
        if providers.len() != 1 {
            println!(
                "warning: {} provider entries name it — default runs will keep the default \
                 provider and swap it back out. `mecha model list` shows which.",
                providers.len()
            );
        }
    }
    Ok(())
}
