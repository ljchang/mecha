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
        /// Give up on the *load* after this many seconds, and, separately, on
        /// a loaded model that stays busy answering a request for this long
        /// before it (the router will not evict a busy model). A cold load from
        /// disk measured 33–39 s on 2026-09-26; the unit allows 600. The wait
        /// for runs in progress before either has no limit, by the owner's
        /// ruling (D13) — `--now` is the way past both.
        #[arg(long, default_value_t = 600)]
        wait_secs: u64,
        /// Switch now: ask every run holding the model to stop at its next safe
        /// point (as Ctrl-C would), give them 15 s, then switch — instead of
        /// waiting for them to finish. A reply in progress ends early.
        #[arg(long)]
        now: bool,
    },
    /// Withdraw a pending switch — the way out of one whose `mecha model
    /// use` is gone or whose file cannot be read, which every run on the
    /// router would otherwise wait for. A live switch is better stopped with
    /// Ctrl-C where it runs; withdrawn here, it stops at its next check.
    CancelSwitch,
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
    /// The switch waiting on this router, if any, and what it waits for
    /// (D13) — what the chip shows as "switching to X — waiting for: …".
    pending_switch: Option<Pending>,
}

#[derive(Serialize)]
struct Pending {
    to: String,
    from: Option<String>,
    started_at: chrono::DateTime<chrono::Utc>,
    /// The runs holding the router, oldest first, as each described itself.
    waiting_on: Vec<String>,
    /// `false` for a switch file nobody can read (`Holds::pending` reports it
    /// as pending, the fail-closed way). No switcher is there to hurry, so
    /// "switch now" cannot work on it; `cancel-switch` is its way out.
    readable: bool,
    /// The switch has stopped waiting for runs and is unloading or loading:
    /// nothing is left for "switch now" to hurry, or for cancel to stop.
    past_the_wait: bool,
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
    /// Why default runs would not follow this model once it is loaded — no
    /// entry names it, or several do (`router::would_not_follow`:
    /// `unfollowable`'s rule, worded for a model that may not be loaded).
    /// The chip offers only models runs would follow.
    would_not_follow: Option<String>,
}

/// How long `--now` (or a hurried switch) gives the runs it asked to stop.
const NOW_GRACE: Duration = Duration::from_secs(15);

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
    let holds = mecha_core::hold::Holds::open_default().ok();
    for base in router_bases(cfg) {
        let Some(list) = router::models(&base).await else {
            out.push((base, None));
            continue;
        };
        let mut r = router_of(cfg, &base, &list);
        r.pending_switch = holds.as_ref().and_then(|h| {
            h.pending(&base).map(|s| Pending {
                // `pending`'s placeholder for an unreadable file is pid 0 —
                // the same test `request_now` refuses it by.
                readable: s.pid != 0,
                past_the_wait: h.is_past_the_wait(&base, &s),
                to: s.to,
                from: s.from,
                started_at: s.started_at,
                waiting_on: h.live(&base).into_iter().map(|h| h.what).collect(),
            })
        });
        out.push((base.clone(), Some(r)));
    }
    out
}

/// What one router's `/models` answer says, against config — the pure half of
/// `survey`, so the readability rules are tested without a server.
fn router_of(cfg: &Config, base: &str, list: &[router::RouterModel]) -> Router {
    let served: Vec<&str> = list.iter().map(|m| m.id.as_str()).collect();
    // An *empty* list would name every entry "not served" under the banner
    // saying it cannot be read — two opposite claims about one answer. An
    // unknown *status* does not touch this: the ids are complete either
    // way, and an entry the router does not serve 400s on every run
    // (found on review, twice). `resident` below is the claim about
    // statuses, and is gated on `readable`.
    let readable = router::readable(list);
    let resident = readable.then(|| router::resident(list)).flatten();
    let unserved = if list.is_empty() {
        Vec::new()
    } else {
        cfg.providers
            .iter()
            .filter(|(_, p)| p.base_url.as_deref().map(router::base).as_deref() == Some(base))
            .filter(|(_, p)| p.model.as_deref().is_some_and(|m| !served.contains(&m)))
            .map(|(n, _)| n.clone())
            .collect()
    };
    Router {
        resident: resident.map(str::to_string),
        readable,
        models: list
            .iter()
            .map(|m| Model {
                id: m.id.clone(),
                status: m.status.value.clone(),
                providers: router::namers(cfg, base, &m.id)
                    .map(str::to_string)
                    .collect(),
                sampling_mismatches: router::sampling_mismatches(cfg, base, m),
                // The rule in the tense that is true of the row: the loaded
                // model *is* loaded (found on review, pass 3).
                would_not_follow: if resident == Some(m.id.as_str()) {
                    router::unfollowable(cfg, base, &m.id)
                } else {
                    router::would_not_follow(cfg, base, &m.id)
                },
            })
            .collect(),
        unserved,
        pending_switch: None,
        base_url: base.to_string(),
    }
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
        Cmd::CancelSwitch => cancel_switch(),
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
        if r.pending_switch.as_ref().is_some_and(|p| !p.readable) {
            println!(
                "  ! a switch file here cannot be read; every run on this router waits for it — \
                 `mecha model cancel-switch` withdraws it"
            );
        } else if let Some(p) = &r.pending_switch {
            println!(
                "  switching to {} (since {}) — waiting for: {}",
                p.to,
                p.started_at.format("%H:%M:%SZ"),
                if p.past_the_wait || p.waiting_on.is_empty() {
                    "nothing; the load is under way".to_string()
                } else {
                    p.waiting_on.join(", ")
                }
            );
        }
    }
    Ok(())
}

async fn use_(cfg: &Config, name: &str, wait_secs: u64, mut now: bool, json: bool) -> Result<()> {
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
    // What is loaded now, only from a list this reads: on an unreadable one
    // `previous` would be `None`, and both rulings that depend on it — R2's
    // `--now` and R1's rollback — would silently not happen (found on review).
    anyhow::ensure!(
        router::readable(&list),
        "the router at {base} answered /models with a list this build cannot read (empty, or \
         a status it does not know), so what is loaded now is unknown — refusing to switch \
         without it; `mecha model list` shows what it sees"
    );
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

    // D13 (owner's rulings, 2026-09-27): the switch waits until no *run*
    // holds the router — not only no request — so a run is answered by one
    // model start to finish; runs that start meanwhile wait for the switch;
    // no time limit, `--now` is the way out and Ctrl-C withdraws the switch.
    // Held to the end of this function, past the load and R1's rollback, so
    // the runs waiting on it resume on whatever is actually loaded.
    let holds = mecha_core::hold::Holds::open_default()?;
    let _switching = match holds.begin_switch(&base, previous.as_deref(), &model)? {
        Ok(s) => s,
        // `--now` for the switch already waiting: hurry that one rather than
        // refuse — the chip's "switch now" is this, since the switch it shows
        // belongs to another process (the one the page, or a terminal,
        // started). Only for the same model: a different one is a second
        // switch, and still refused.
        Err(other) if now && other.to == model => {
            // `request_now` checks the target again as it marks: between the
            // refusal above and the marker, the switch could have been
            // withdrawn and replaced by one to another model, which must not
            // be hurried in this one's name (found on review).
            return match holds.request_now(&base, &model)? {
                Some(s) => {
                    eprintln!(
                        "asked the waiting switch to {} (pid {}) to go now",
                        s.to, s.pid
                    );
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "base_url": base, "model": model, "hurried": true,
                            })
                        );
                    }
                    Ok(())
                }
                None => bail!(
                    "the switch to {model} on {base} is no longer waiting — `mecha model list` \
                     shows what is loaded"
                ),
            };
        }
        Err(other) => bail!(
            "a switch to {} is already waiting on {base} (pid {}, since {}) — let it finish, \
             stop it with Ctrl-C where it runs, or withdraw it with `mecha model cancel-switch`",
            other.to,
            other.pid,
            other.started_at.format("%H:%M:%SZ")
        ),
    };
    // One interrupt receiver for the whole switch — wait, unload, load and
    // R1's rollback. A `ctrl_c()` future made and dropped per poll left
    // SIGINT captured with nobody draining it once the wait ended, so up to
    // three 600 s router calls could not be stopped at all (review of #350;
    // `interrupt.rs`: an uninterruptible process is worse than a lost one).
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .context("installing the interrupt handler for the switch")?;
    if now {
        let asked = holds.cancel_holders(&base);
        if asked > 0 {
            eprintln!("asking {asked} run(s) to stop now");
        }
        wait_for_runs(&holds, &_switching, &base, Some(NOW_GRACE), &mut interrupt).await?;
    } else if wait_for_runs(&holds, &_switching, &base, None, &mut interrupt).await? {
        // Hurried while it waited: from here it is the `--now` it was asked
        // to become, so R2 below cuts off a reply in flight too.
        now = true;
    }
    // Re-read after the wait, which can last hours: R1's rollback target and
    // "already resident" are about what is loaded now, not when this began.
    let list = router::models(&base)
        .await
        .with_context(|| format!("{base} stopped answering while the switch waited"))?;
    // The first read's guard, again: the wait can last hours, and a router
    // restarted in it can answer with a list this build cannot read — then
    // `previous` is `None`, and `--now`'s unload and R1's rollback both
    // silently do not happen (review of #350).
    anyhow::ensure!(
        router::readable(&list),
        "the router at {base} answered /models with a list this build cannot read after the \
         switch waited, so what is loaded now is unknown — refusing to switch without it"
    );
    let previous = router::resident(&list).map(str::to_string);
    if previous.as_deref() == Some(model.as_str()) {
        return report(cfg, &base, &model, 0.0, json);
    }
    // The last check before anything changes. With nothing holding, the wait
    // returns on its first look, so a `cancel-switch` during the re-read was
    // otherwise never seen: waiting runs were released to resolve a model
    // while this process swapped it under them (review of #350).
    anyhow::ensure!(
        _switching.still_pending(),
        "the switch was withdrawn (`mecha model cancel-switch`); the loaded model stays"
    );

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
            let unloaded = tokio::select! {
                r = router::unload(&base, prev, Duration::from_secs(wait_secs)) => r,
                _ = interrupt.recv() => bail!(
                    "interrupted while {prev} was being stopped; the switch is withdrawn — \
                     `mecha model list` shows what is loaded"
                ),
            };
            if let Err(e) = unloaded {
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
    let mut hooks = SwitchHooks {
        switching: &_switching,
        previous: previous.clone(),
        now,
        cut: false,
        said: false,
    };
    let loaded = tokio::select! {
        r = router::load_with(&base, &model, Duration::from_secs(wait_secs), &mut hooks) => r,
        _ = interrupt.recv() => bail!(
            "interrupted while {model} was loading; the switch is withdrawn, and the router may \
             still finish the load — `mecha model list` shows what is loaded"
        ),
    };
    match loaded {
        Ok(()) => report(cfg, &base, &model, started.elapsed().as_secs_f64(), json),
        // R1: a model that does not come up is replaced by the one it was
        // meant to replace, rather than leaving the next request to load the
        // default.
        Err(failed) => {
            let Some(prev) = previous else {
                return Err(failed);
            };
            eprintln!("{failed:#}\nloading {prev} back…");
            let back = tokio::select! {
                r = router::load(&base, &prev, Duration::from_secs(wait_secs)) => r,
                _ = interrupt.recv() => bail!(
                    "{failed:#}\ninterrupted while loading {prev} back; `mecha model list` shows \
                     what is loaded"
                ),
            };
            match back {
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

/// What a switch does while the router refuses its load because the loaded
/// model is still answering a request. The switch is still only *waiting*
/// then, so it stays hurriable and withdrawable — the chip keeps offering
/// "switch now" and cancel, and both work (found on review: marked past its
/// wait before the load, it offered neither while being refused for
/// minutes).
struct SwitchHooks<'a> {
    switching: &'a mecha_core::hold::Switching,
    previous: Option<String>,
    /// `--now`, or hurried while it waited for runs.
    now: bool,
    /// Whether the busy model has been cut off already: once is the ask.
    cut: bool,
    said: bool,
}

impl router::LoadHooks for SwitchHooks<'_> {
    fn busy(&mut self) -> router::OnBusy {
        if !self.switching.still_pending() {
            return router::OnBusy::Stop(anyhow::anyhow!(
                "the switch was withdrawn (`mecha model cancel-switch`); the loaded model stays"
            ));
        }
        if let Some(prev) = &self.previous {
            if !self.cut && (self.now || self.switching.now_requested()) {
                self.cut = true;
                eprintln!("stopping {prev} now — the reply it is answering will fail");
                return router::OnBusy::CutOff(prev.clone());
            }
            if !std::mem::replace(&mut self.said, true) {
                eprintln!(
                    "{prev} is answering a request; the switch waits for it to finish \
                     (--now, or \"switch now\" on the chip, cuts it off)"
                );
            }
        }
        router::OnBusy::Wait
    }

    /// Past its wait now: "switch now" has nothing left to hurry and the
    /// chip stops offering it (found on review of #364). A hint, not a guard
    /// — a marker that cannot be written costs only that.
    fn accepted(&mut self) {
        if let Err(e) = self.switching.past_the_wait() {
            eprintln!("warning: {e:#}");
        }
    }
}

/// Wait until no run holds the router at `base` (D13), saying what it waits
/// on whenever that changes. `limit` is `--now`'s grace for the runs it asked
/// to stop; without one there is none, by ruling, and Ctrl-C withdraws the
/// switch — the pending file goes with `use_`'s guard, and the model stays.
///
/// A wait without a limit can be hurried from outside
/// ([`Holds::request_now`](mecha_core::hold::Holds::request_now), the chip's
/// "switch now"): it then does what `--now` does from that moment — asks the
/// runs to stop and gives them the same grace — and returns `true`.
async fn wait_for_runs(
    holds: &mecha_core::hold::Holds,
    switching: &mecha_core::hold::Switching,
    base: &str,
    mut limit: Option<Duration>,
    interrupt: &mut tokio::signal::unix::Signal,
) -> Result<bool> {
    let mut started = Instant::now();
    let mut said = String::new();
    let mut hurried = false;
    loop {
        if !switching.still_pending() {
            bail!("the switch was withdrawn (`mecha model cancel-switch`); the loaded model stays");
        }
        if limit.is_none() && switching.now_requested() {
            let asked = holds.cancel_holders(base);
            eprintln!("asked to switch now — asking {asked} run(s) to stop");
            limit = Some(NOW_GRACE);
            started = Instant::now();
            hurried = true;
        }
        let live = holds.live(base);
        if live.is_empty() {
            return Ok(hurried);
        }
        if let Some(l) = limit.filter(|l| started.elapsed() >= *l) {
            eprintln!(
                "{} run(s) did not stop within {}s; switching anyway",
                live.len(),
                l.as_secs()
            );
            return Ok(hurried);
        }
        let what: Vec<&str> = live.iter().map(|h| h.what.as_str()).collect();
        let now_saying = format!(
            "waiting for {} run(s) to finish: {} (--now stops them, Ctrl-C cancels the switch)",
            live.len(),
            what.join(", ")
        );
        if now_saying != said {
            eprintln!("{now_saying}");
            said = now_saying;
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            _ = interrupt.recv() => {
                bail!("switch cancelled; the loaded model stays");
            }
        }
    }
}

/// Withdraw every pending switch — every switch file in `~/.mecha/holds`, not
/// only those on routers this config still follows, so a switch left on one it
/// no longer points at is reachable too (review of #350).
fn cancel_switch() -> Result<()> {
    let holds = mecha_core::hold::Holds::open_default()?;
    let withdrawn = holds.withdraw_all_switches();
    for s in &withdrawn {
        eprintln!(
            "withdrew the switch to {} on {} (pid {}, since {}); runs waiting for it start now",
            s.to,
            s.base_url,
            s.pid,
            s.started_at.format("%H:%M:%SZ")
        );
    }
    if withdrawn.is_empty() {
        eprintln!("no switch is pending");
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::config::ProviderConfig;

    const BASE: &str = "http://127.0.0.1:8080";

    fn cfg() -> Config {
        let mut c = Config {
            default_provider: "local".into(),
            ..Default::default()
        };
        for (name, model) in [("local", "qwen3.6-35b-a3b"), ("stale", "gone-model")] {
            c.providers.insert(
                name.into(),
                ProviderConfig {
                    kind: "local".into(),
                    base_url: Some(BASE.into()),
                    model: Some(model.into()),
                    follow_loaded: name == "local",
                    ..Default::default()
                },
            );
        }
        c
    }

    fn list(json: &str) -> Vec<router::RouterModel> {
        serde_json::from_str::<serde_json::Value>(json)
            .and_then(|v| serde_json::from_value(v["data"].clone()))
            .unwrap()
    }

    #[test]
    fn a_readable_list_names_what_is_loaded_and_what_is_not_served() {
        let r = router_of(
            &cfg(),
            BASE,
            &list(r#"{"data":[{"id":"qwen3.6-35b-a3b","status":{"value":"loaded"}}]}"#),
        );
        assert!(r.readable);
        assert_eq!(r.resident.as_deref(), Some("qwen3.6-35b-a3b"));
        assert_eq!(r.unserved, vec!["stale".to_string()]);
    }

    /// Each row's reason is in the tense true of it: the loaded model's says
    /// it is loaded, an unloaded one's says "once loaded" (review, pass 3).
    #[test]
    fn the_reason_is_in_the_tense_true_of_the_row() {
        let mut c = cfg();
        c.providers.remove("local");
        c.default_provider = "stale".into();
        let r = router_of(
            &c,
            BASE,
            &list(
                r#"{"data":[{"id":"qwen3.6-35b-a3b","status":{"value":"loaded"}},
                            {"id":"orphan","status":{"value":"unloaded"}}]}"#,
            ),
        );
        let why = |id: &str| {
            r.models
                .iter()
                .find(|m| m.id == id)
                .and_then(|m| m.would_not_follow.clone())
                .unwrap()
        };
        assert!(
            why("qwen3.6-35b-a3b").contains("loaded, but"),
            "{}",
            why("qwen3.6-35b-a3b")
        );
        assert!(why("orphan").contains("once loaded"), "{}", why("orphan"));
    }

    /// An unknown status makes "what is loaded" unknown — never "nothing" —
    /// but the ids are still complete, so "not served" still holds.
    #[test]
    fn an_unknown_status_hides_the_resident_but_not_the_unserved() {
        let r = router_of(
            &cfg(),
            BASE,
            &list(r#"{"data":[{"id":"qwen3.6-35b-a3b","status":{"value":"resident"}}]}"#),
        );
        assert!(!r.readable);
        assert_eq!(r.resident, None);
        assert_eq!(r.unserved, vec!["stale".to_string()]);
    }

    /// An empty list says nothing about ids either: no "not served" claims
    /// under a banner saying the list cannot be read.
    #[test]
    fn an_empty_list_claims_nothing() {
        let r = router_of(&cfg(), BASE, &list(r#"{"data":[]}"#));
        assert!(!r.readable);
        assert_eq!(r.resident, None);
        assert!(r.unserved.is_empty());
    }
}

#[cfg(test)]
mod wait_tests {
    use super::*;
    use mecha_core::hold::Holds;

    const ROUTER: &str = "http://127.0.0.1:8080";

    fn interrupt() -> tokio::signal::unix::Signal {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).unwrap()
    }

    /// A withdrawn switch stops waiting, rather than go on to load a model
    /// nobody is waiting for any more.
    #[tokio::test]
    async fn a_withdrawn_switch_stops_waiting() {
        let home = crate::testenv::HomeGuard::new("model-wait-withdrawn");
        let holds = Holds::new(home.dir.join("holds"));
        let switching = holds.begin_switch(ROUTER, Some("a"), "b").unwrap().unwrap();
        // A run it waits for, then the withdrawal the command makes.
        let held = holds.try_hold(ROUTER, "web chat");
        assert!(held.unwrap().is_err(), "a run held under a pending switch");
        let held = {
            assert!(holds.withdraw_switch(ROUTER).is_some());
            holds.try_hold(ROUTER, "web chat").unwrap().unwrap()
        };
        let err = tokio::time::timeout(
            Duration::from_secs(5),
            wait_for_runs(&holds, &switching, ROUTER, None, &mut interrupt()),
        )
        .await
        .expect("the wait never noticed the withdrawal")
        .unwrap_err();
        assert!(err.to_string().contains("withdrawn"), "{err}");
        drop(held);
    }

    /// "Switch now" from outside hurries a wait that has no limit: the runs
    /// are asked to stop, and the wait says it was hurried, so `use_` goes on
    /// as `--now` would.
    #[tokio::test]
    async fn a_hurried_wait_asks_the_runs_to_stop() {
        let home = crate::testenv::HomeGuard::new("model-wait-hurried");
        let holds = std::sync::Arc::new(Holds::new(home.dir.join("holds")));
        let held = holds.try_hold(ROUTER, "web chat").unwrap().unwrap();
        let asked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let asked = std::sync::Arc::clone(&asked);
            held.on_cancel(move || asked.store(true, std::sync::atomic::Ordering::SeqCst));
        }
        let switching = holds.begin_switch(ROUTER, Some("a"), "b").unwrap().unwrap();
        let waiting = {
            let holds = std::sync::Arc::clone(&holds);
            tokio::spawn(async move {
                wait_for_runs(&holds, &switching, ROUTER, None, &mut interrupt()).await
            })
        };
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(
            !asked.load(std::sync::atomic::Ordering::SeqCst),
            "asked before anyone hurried it"
        );
        holds.request_now(ROUTER, "b").unwrap().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !asked.load(std::sync::atomic::Ordering::SeqCst) {
            assert!(
                Instant::now() < deadline,
                "the hurried wait never asked the run to stop"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        drop(held);
        let hurried = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("the wait never ended after the run stopped")
            .unwrap()
            .unwrap();
        assert!(hurried, "the wait did not say it was hurried");
    }

    /// While the load is refused as busy the switch is still only waiting,
    /// so its hooks keep "switch now" and cancel working: it waits, a hurry
    /// cuts the busy model off (once), a withdrawal stops it, and only the
    /// router taking the load marks it past its wait.
    #[test]
    fn a_busy_refusal_still_honours_switch_now_and_cancel() {
        use router::{LoadHooks, OnBusy};
        let home = crate::testenv::HomeGuard::new("model-busy-hooks");
        let holds = Holds::new(home.dir.join("holds"));
        let switching = holds
            .begin_switch(ROUTER, Some("old"), "b")
            .unwrap()
            .unwrap();
        let mut hooks = SwitchHooks {
            switching: &switching,
            previous: Some("old".into()),
            now: false,
            cut: false,
            said: false,
        };
        assert!(matches!(hooks.busy(), OnBusy::Wait));
        holds.request_now(ROUTER, "b").unwrap().unwrap();
        assert!(matches!(hooks.busy(), OnBusy::CutOff(m) if m == "old"));
        assert!(matches!(hooks.busy(), OnBusy::Wait), "cut off twice");
        let pending = holds.pending(ROUTER).unwrap();
        assert!(
            !holds.is_past_the_wait(ROUTER, &pending),
            "past its wait while refused"
        );
        hooks.accepted();
        assert!(holds.is_past_the_wait(ROUTER, &pending));
        holds.withdraw_switch(ROUTER).unwrap();
        assert!(matches!(hooks.busy(), OnBusy::Stop(e) if e.to_string().contains("withdrawn")));
    }

    /// The wait ends when the last hold drops — and not before.
    #[tokio::test]
    async fn the_wait_ends_when_the_last_hold_drops() {
        let home = crate::testenv::HomeGuard::new("model-wait-drops");
        let holds = std::sync::Arc::new(Holds::new(home.dir.join("holds")));
        let held = holds.try_hold(ROUTER, "mecha run").unwrap().unwrap();
        let switching = holds.begin_switch(ROUTER, Some("a"), "b").unwrap().unwrap();
        let waiting = {
            let holds = std::sync::Arc::clone(&holds);
            tokio::spawn(async move {
                wait_for_runs(&holds, &switching, ROUTER, None, &mut interrupt()).await
            })
        };
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(
            !waiting.is_finished(),
            "the switch went ahead under a held run"
        );
        drop(held);
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("the wait never ended after the last hold dropped")
            .unwrap()
            .unwrap();
    }
}
