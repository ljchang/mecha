//! `mecha model` — what the local model router can serve, and choosing which
//! one it holds (`REMOTE-SURFACE-DESIGN.md` §14, D12).
//!
//! The owner's side of `provider::router`. **Loading a model is the pick** —
//! there is no setting to write, because every default run asks the router
//! what it has loaded. This is reached by the owner only: this command, the
//! TUI's `/model` picker, the web chip through `mecha serve`. There is no
//! tool; a model never chooses the model.

use crate::GlobalOpts;
use anyhow::{bail, Context, Result};
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
    /// and wait until it is resident; every run after this one follows it.
    /// The model it replaces finishes any reply in flight first, unless
    /// `--now`. If the new model fails to come up, the previous one is loaded
    /// back. A model whose preset temperature disagrees with its provider
    /// entry is refused.
    Use {
        name: String,
        /// Give up after this many seconds. A cold load from disk measured
        /// 33–39 s on 2026-09-26; the unit allows 600.
        #[arg(long, default_value_t = 600)]
        wait_secs: u64,
        /// Switch now: stop the resident model even mid-reply, instead of
        /// waiting for it to go idle. The reply in progress fails.
        #[arg(long)]
        now: bool,
    },
}

#[derive(Serialize)]
struct Router {
    base_url: String,
    /// `None` when nothing is loaded — or when `readable` is false.
    resident: Option<String>,
    /// Whether the router's model list is one this build fully reads. When
    /// it is not, `resident: None` means "unknown", not "nothing loaded".
    readable: bool,
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
    /// Entries whose `temperature` disagrees with this model's preset;
    /// `mecha model use` refuses the model while any remain (R4).
    sampling_mismatches: Vec<String>,
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
    router::followed_bases(cfg)
}

async fn survey(cfg: &Config) -> Vec<(String, Option<Router>)> {
    let mut out = Vec::new();
    for base in router_bases(cfg) {
        let Some(list) = router::models(&base).await else {
            out.push((base, None));
            continue;
        };
        let served: Vec<&str> = list.iter().map(|m| m.id.as_str()).collect();
        // Only from a list this reads: an unreadable (say, empty) one would
        // name every entry "not served" under the banner saying it cannot be
        // read — two opposite claims about one answer (found on review).
        let readable = router::readable(&list);
        let unserved = if !readable {
            Vec::new()
        } else {
            cfg.providers
                .iter()
                .filter(|(_, p)| p.base_url.as_deref().map(router::base).as_deref() == Some(&base))
                .filter(|(_, p)| p.model.as_deref().is_some_and(|m| !served.contains(&m)))
                .map(|(n, _)| n.clone())
                .collect()
        };
        let r = Router {
            resident: readable
                .then(|| router::resident(&list).map(str::to_string))
                .flatten(),
            readable,
            models: list
                .iter()
                .map(|m| Model {
                    id: m.id.clone(),
                    status: m.status.value.clone(),
                    providers: router::namers(cfg, &base, &m.id)
                        .map(str::to_string)
                        .collect(),
                    sampling_mismatches: router::sampling_mismatches(cfg, &base, m),
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
        Cmd::Use {
            name,
            wait_secs,
            now,
        } => use_(&cfg, &name, wait_secs, now, args.json).await,
    }
}

async fn list(cfg: &Config, json: bool) -> Result<()> {
    let routers = survey(cfg).await;
    if json {
        // A router that did not answer is listed, not dropped: "nothing
        // loaded" and "server down" need different answers from whoever
        // reads this (the chip), and an unreadable source is a finding, not
        // an empty list (found on review).
        let all: Vec<serde_json::Value> = routers
            .iter()
            .map(|(base, r)| match r {
                Some(r) => {
                    let mut v = serde_json::to_value(r).unwrap_or_default();
                    v["reachable"] = true.into();
                    v
                }
                None => serde_json::json!({ "base_url": base, "reachable": false }),
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "routers": all }))?
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
        if !r.readable {
            println!(
                "  ! this router's model list is one this build cannot fully read (empty, or a \
                 status it does not know) — which model is loaded is unknown"
            );
        }
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
            for w in &m.sampling_mismatches {
                println!("      ! {w}");
            }
        }
        for n in &r.unserved {
            println!("  ! [providers.{n}] names a model this router does not serve");
        }
    }
    Ok(())
}

async fn use_(cfg: &Config, name: &str, wait_secs: u64, now: bool, json: bool) -> Result<()> {
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
    let list = router::models(&base)
        .await
        .with_context(|| format!("{base} is not a llama-server router (or is not up)"))?;
    let previous = router::resident(&list).map(str::to_string);

    // R4: a preset whose temperature the config would override is refused.
    if let Some(m) = list.iter().find(|m| m.id == model) {
        let mismatches = router::sampling_mismatches(cfg, &base, m);
        if !mismatches.is_empty() {
            bail!(
                "refusing to load {model}: its preset's sampling disagrees with config, so every \
                 request would silently re-tune it —\n  {}\nMake the entry's temperature match \
                 the preset in scripts/start-router.sh (or unset it).",
                mismatches.join("\n  ")
            );
        }
    }

    // Already resident is success — after R4, so the model R4 refuses is
    // refused whatever happens to be loaded (found on review).
    if previous.as_deref() == Some(model.as_str()) {
        return report(cfg, &base, &model, 0.0, json);
    }

    // R2: the resident model mid-reply is waited for, or — `--now` — cut off.
    if let Some(prev) = &previous {
        let busy = match mecha_core::brief::read_slots(&base, Some(prev)).await {
            mecha_core::brief::Slots::Read { busy, .. } => Some(busy),
            _ => None,
        };
        if now {
            // `--now` unloads whatever the slot reading says: a sleeping
            // model, or a `/slots` read that bounced, must not quietly turn
            // "now" into a wait (found on review).
            match busy {
                Some(b) if b > 0 => {
                    eprintln!("stopping {prev} now — {b} reply(ies) in progress will fail")
                }
                _ => eprintln!("stopping {prev} now"),
            }
            // Not fatal: the load below evicts an idle model regardless, so
            // a refused unload costs only the "now".
            if let Err(e) = router::unload(&base, prev, Duration::from_secs(wait_secs)).await {
                eprintln!("warning: {e:#}");
            }
        } else if let Some(b) = busy.filter(|b| *b > 0) {
            eprintln!(
                "{prev} is answering {b} request(s); the switch waits for it to go idle \
                 (--now cuts it off)"
            );
        }
    }

    let started = Instant::now();
    match router::load(&base, &model, Duration::from_secs(wait_secs)).await {
        Ok(()) => report(cfg, &base, &model, started.elapsed().as_secs_f64(), json),
        // R1: a model that does not come up is replaced by the one it was
        // meant to replace, rather than leaving the next request to load the
        // default.
        Err(failed) => {
            let Some(prev) = previous else {
                return Err(failed);
            };
            eprintln!("{failed:#}\nloading {prev} back…");
            match router::load(&base, &prev, Duration::from_secs(wait_secs)).await {
                Ok(()) => Err(failed.context(format!(
                    "{model} did not load; {prev} is loaded again, as it was before"
                ))),
                Err(back) => Err(failed.context(format!(
                    "{model} did not load, and loading {prev} back failed too: {back:#}"
                ))),
            }
        }
    }
}

/// What `use` did, for a person or for the chip.
fn report(cfg: &Config, base: &str, model: &str, secs: f64, json: bool) -> Result<()> {
    let providers: Vec<String> = router::namers(cfg, base, model)
        .map(str::to_string)
        .collect();
    // The same rule `observe` warns with, not a re-derivation of it.
    let warning = router::unfollowable(cfg, base, model);
    if json {
        println!(
            "{}",
            serde_json::json!({
                "base_url": base, "model": model, "providers": providers,
                "seconds": secs, "warning": warning,
            })
        );
    } else {
        println!("{model} is loaded at {base} ({secs:.1}s)");
        if let Some(w) = warning {
            println!("warning: {w}");
        }
    }
    Ok(())
}
