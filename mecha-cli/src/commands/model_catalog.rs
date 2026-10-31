//! `mecha model catalog`, `add` and `remove` — the owner's side of
//! `mecha_core::catalog` (`docs/MODEL-CATALOG-DESIGN.md`).
//!
//! **Installing is the owner's act (C4).** This command, and the chip through
//! `mecha serve`, which runs `model add --yes` as a child exactly as it runs
//! `model use`. There is no tool: a model never chooses what is fetched onto
//! the machine, any more than it chooses the model.

use crate::GlobalOpts;
use anyhow::{bail, Context, Result};
use mecha_core::catalog::{self, Entry, Offer};
use mecha_core::config::Config;
use mecha_core::provider::router;
use mecha_core::router_unit::{self, Naming};
use serde::Serialize;
use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};

/// The comment line above every provider table `model add` writes, which
/// `remove` takes out with the table. What decides that a table is `add`'s
/// is the caller's check that it names this router and this id.
const TABLE_COMMENT: &str = "Written by `mecha model add` from mecha's catalog.";

/// What every catalog verb reads first: the machine, its hub, the router.
struct Here {
    m: mecha_core::sidecar::Machinery,
    machine: mecha_core::recommend::Machine,
    hub: std::path::PathBuf,
    naming: Naming,
}

impl Here {
    fn read() -> Result<Here> {
        Ok(Here {
            m: mecha_core::sidecar::Machinery::real()?,
            machine: mecha_core::recommend::Machine::read()?,
            hub: mecha_core::fetch::hub_dir()?,
            naming: Naming::shipped(),
        })
    }

    fn offers(&self) -> Result<Vec<Offer>> {
        catalog::offers(&self.m.mecha_home, &self.hub, &self.machine)
    }

    /// Why a catalog model cannot be put on the router here, or `None`.
    /// Only a router mecha installed: one the owner runs is theirs (§3), and
    /// gets the section to paste instead.
    fn router_refusal(&self) -> Result<Option<String>> {
        let home = &self.m.mecha_home;
        if !router_unit::installed_by_mecha(home)? {
            return Ok(Some(
                "mecha did not install the chat router here — `mecha setup chat` installs it; a \
                 router you run yourself takes the preset section by hand"
                    .into(),
            ));
        }
        if router_unit::installed_choice(home)?.is_none() {
            return Ok(Some(
                "the router's presets file is missing — `mecha setup chat` writes it again".into(),
            ));
        }
        Ok(None)
    }
}

/// The catalog as `model list --json` carries it for the chip: every offer,
/// whether a model can be installed here at all, and an install under way.
#[derive(Serialize)]
pub(super) struct Listing {
    offers: Vec<Offer>,
    /// Why nothing can be installed on this machine's router, or `None`.
    refusal: Option<String>,
    installing: Option<catalog::Progress>,
}

/// A catalog that cannot be read is said as the refusal, with no offers —
/// never a missing listing, which the chip would read as a build with no
/// catalog (review of #727).
pub(super) fn listing() -> Listing {
    let unreadable = |e: anyhow::Error| Listing {
        offers: Vec::new(),
        refusal: Some(format!("mecha's catalog could not be read: {e:#}")),
        installing: None,
    };
    let here = match Here::read() {
        Ok(h) => h,
        Err(e) => return unreadable(e),
    };
    match here.offers() {
        Ok(offers) => Listing {
            offers,
            refusal: here
                .router_refusal()
                .unwrap_or_else(|e| Some(format!("{e:#}"))),
            installing: catalog::progress_read(&here.m.mecha_home),
        },
        Err(e) => unreadable(e),
    }
}

fn gib(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / 1_073_741_824.0)
}

/// `mecha model catalog`.
pub(super) async fn show(json: bool) -> Result<()> {
    let here = Here::read()?;
    let offers = here.offers()?;
    let served = served_ids(&here.naming).await;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "offers": offers,
                "served": served,
                "refusal": here.router_refusal()?,
            }))?
        );
        return Ok(());
    }
    let tier = here.machine.tier_gb();
    println!(
        "Chat models in mecha's catalog{}:",
        tier.map(|t| format!(" (this machine: the {t} GB tier)"))
            .unwrap_or_default()
    );
    for o in &offers {
        let serves = served.as_ref().is_some_and(|s| s.iter().any(|x| x == o.id));
        let state = if serves && o.installed {
            "installed, served"
        } else if serves {
            // A preset someone wrote by hand: served, and not in the record —
            // `setup chat` would drop it, and `model add` adopts it.
            "served, set up by hand — `mecha model add` brings it into the catalog's record"
        } else if o.installed && served.is_none() {
            // `served_ids` keeps a router that did not answer apart from one
            // that serves none of these (review of #727).
            "installed; the router is not answering, so whether it serves it is unknown"
        } else if o.installed {
            "installed, not served yet"
        } else if o.unfit.is_some() {
            "not offered here"
        } else {
            "available"
        };
        println!("\n  {}  — {state}", o.id);
        println!("    {}", o.label);
        println!("    {}", o.note);
        match &o.unfit {
            Some(why) => println!("    {why}"),
            None => println!(
                "    {} to download; ~{} in memory (arithmetic), in place of the chat model",
                gib(o.download_bytes),
                o.memory_mb
                    .map(|mb| gib(u64::from(mb) * 1_048_576))
                    .unwrap_or_else(|| "—".into())
            ),
        }
    }
    if let Some(why) = here.router_refusal()? {
        println!("\nNothing can be installed here yet: {why}");
    } else {
        println!("\n`mecha model add <id>` shows what it will do and asks first.");
    }
    Ok(())
}

/// The ids the router at the naming's port serves, or `None` when it did not
/// answer — never an empty list for a router that is down.
async fn served_ids(naming: &Naming) -> Option<Vec<String>> {
    router::models(&naming.base())
        .await
        .map(|l| l.into_iter().map(|m| m.id).collect())
}

/// The provider table a catalog model is served through (C9): from the
/// entry's pins, never from `/props`, which would load the model to answer.
fn provider_settings(e: &Entry, tier: u32, base: &str) -> Result<Vec<(&'static str, String)>> {
    let (ctx, slots, _) = catalog::geometry_at(e, tier)
        .with_context(|| format!("{} is not offered at the {tier} GB tier", e.id))?;
    let q = mecha_core::onboarding::toml_string;
    let mut s = vec![
        ("kind", q("local")),
        ("base_url", q(base)),
        ("model", q(e.id)),
        // Per slot: `-c` is divided across `-np` (CLAUDE.md).
        ("context_window", (ctx / slots).to_string()),
        (
            "vision",
            e.pins
                .iter()
                .any(|p| p.role == catalog::Role::Projector)
                .to_string(),
        ),
    ];
    if let Some(t) = e.temperature {
        s.push(("temperature", format!("{t:?}")));
    }
    Ok(s)
}

/// The default provider's name when it names this router and does not yet
/// follow it — the table `follow_loaded = true` goes on (C8).
fn follow_target(cfg: &Config, port: u16) -> Option<String> {
    let p = cfg.providers.get(&cfg.default_provider)?;
    (router_unit::names_this_router(p, port) && !p.follow_loaded)
        .then(|| cfg.default_provider.clone())
}

/// The project `mecha.toml` that names `[providers.<provider>]`, when the
/// layered config reads one: a table there replaces the global one whole.
fn project_table(global: &GlobalOpts, provider: &str) -> Option<std::path::PathBuf> {
    if global.global_config_only {
        return None;
    }
    let path = std::env::current_dir().ok()?.join(Config::PROJECT_FILE);
    let v: toml::Value = toml::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
    v.get("providers")?.get(provider)?;
    Some(path)
}

/// A table of that name already in the config that is not this entry's.
fn table_conflict(cfg: &Config, e: &Entry, port: u16) -> Option<String> {
    let p = cfg.providers.get(e.provider)?;
    let ours = router_unit::names_this_router(p, port) && p.model.as_deref() == Some(e.id);
    (!ours).then(|| {
        format!(
            "[providers.{}] is already in your config and names something else — rename it, or \
             remove it, and run this again",
            e.provider
        )
    })
}

fn confirm(question: &str) -> Result<bool> {
    print!("\n{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}

/// Clears the progress record however `add` ends.
struct ProgressGuard<'a>(&'a std::path::Path);

impl Drop for ProgressGuard<'_> {
    fn drop(&mut self) {
        catalog::progress_clear(self.0);
    }
}

/// Wait until no run holds the router at `base`, as the plan promised:
/// `router_unit::install` declines rather than waits (the gate's §10.3), and
/// a decline after a 20 GB download is not what the owner approved (review
/// of #727). Ctrl-C ends the process with the files kept and nothing on the
/// router or in the record changed (both are edited after this) — SIGINT is
/// deliberately not captured here, since a handler outliving the wait would
/// leave the restart below unstoppable (the #350 trap). A run that starts in
/// the instant before the restart still meets the gate's decline, which says
/// to run this again.
///
/// Every spelling of the router's address `install` will check
/// (`router_unit::hold_bases`): a run holds under its provider's own
/// `base_url`, and `localhost` is not folded into `127.0.0.1`.
async fn wait_quiet(cfg: &Config, naming: &Naming, say: &mut dyn FnMut(&str)) -> Result<()> {
    let holds = mecha_core::hold::Holds::open_default()?;
    let bases = router_unit::hold_bases(cfg, naming);
    let mut said = String::new();
    loop {
        let live: Vec<_> = bases.iter().flat_map(|b| holds.live(b)).collect();
        if live.is_empty() {
            return Ok(());
        }
        let what: Vec<&str> = live.iter().map(|h| h.what.as_str()).collect();
        let now = format!(
            "waiting for {} run(s) using the router to finish before restarting it: {}",
            live.len(),
            what.join(", ")
        );
        if now != said {
            say(&now);
            said = now;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// `mecha model add <id>`.
pub(super) async fn add(
    global: &GlobalOpts,
    cfg: &Config,
    id: &str,
    yes: bool,
    use_after: bool,
    json: bool,
) -> Result<()> {
    let here = Here::read()?;
    let home = here.m.mecha_home.clone();
    let Some(e) = catalog::entry(id) else {
        bail!(
            "no chat model {id:?} in mecha's catalog — `mecha model catalog` lists them: {}",
            catalog::CATALOG
                .iter()
                .map(|e| e.id)
                .collect::<Vec<_>>()
                .join(", ")
        );
    };
    let offer = here
        .offers()?
        .into_iter()
        .find(|o| o.id == id)
        .context("the catalog entry has no offer")?;
    if let Some(why) = &offer.unfit {
        bail!("{id} is not offered on this machine: {why}");
    }
    let tier = here.machine.tier_gb().context("no memory tier")?;
    let base = here.naming.base();
    if let Some(why) = here.router_refusal()? {
        // The owner's router: what to paste, and nothing changed.
        let paths = catalog::paths(e, &here.hub)?;
        eprintln!(
            "The preset section and provider table for {id}, to add by hand:\n\n{}\n[providers.{}]",
            catalog::section_text(e, tier, &paths)?,
            e.provider
        );
        for (k, v) in provider_settings(e, tier, &base)? {
            eprintln!("{k} = {v}");
        }
        bail!("nothing was installed: {why}");
    }
    if let Some(why) = table_conflict(cfg, e, here.naming.port) {
        bail!("nothing was installed: {why}");
    }
    // A brought GGUF whose stem is this id already holds the name on the
    // router; the catalog's section would be left out (review of #727).
    if let Some(c) = router_unit::installed_choice(&home)? {
        if router_unit::choice_alias(&c) == id {
            bail!(
                "nothing was installed: the router's base model is already served as {id} — a \
                 file you brought with that name — so a catalog model cannot have it too"
            );
        }
    }
    let one_at_a_time = || match catalog::progress_read(&home) {
        Some(p) => bail!(
            "another catalog install is running ({} — {}, pid {}); one at a time",
            p.id,
            p.phase,
            p.pid
        ),
        None => Ok(()),
    };
    one_at_a_time()?;
    let settings = provider_settings(e, tier, &base)?;
    // Written to the global file, so a project `mecha.toml` naming the same
    // table would shadow it whole: said in the plan, not written (review of
    // #727) — `remove`'s mirror case.
    let mut follow = follow_target(cfg, here.naming.port);
    let shadowed = follow
        .as_deref()
        .and_then(|d| project_table(global, d).map(|f| (d.to_string(), f)));
    if shadowed.is_some() {
        follow = None;
    }
    if let Some(f) = project_table(global, e.provider) {
        bail!(
            "nothing was installed: [providers.{}] is in {}, which would shadow the table `add` \
             writes to your global config — remove it there, or run this from another directory",
            e.provider,
            f.display()
        );
    }

    // The plan, in full, before anything is fetched: what the approval is for.
    // On stderr under `--json`, where stdout is the answer.
    let say_line = |s: &str| {
        if json {
            eprintln!("{s}");
        } else {
            println!("{s}");
        }
    };
    say_line(&format!("{id} — {}", e.label));
    say_line(&format!("  {}", e.note));
    say_line(&format!(
        "  downloads {} (pinned, every file checked against its sha256)",
        gib(offer.download_bytes)
    ));
    if let Some(mb) = offer.memory_mb {
        say_line(&format!(
            "  ~{} in memory at this machine's tier (arithmetic), in place of the chat model — \
             the router holds one at a time",
            gib(u64::from(mb) * 1_048_576)
        ));
    }
    say_line(&format!(
        "  writes [providers.{}] for it{}",
        e.provider,
        follow
            .as_deref()
            .map(|d| format!(", and `follow_loaded = true` on [providers.{d}] so default runs follow the model you load"))
            .unwrap_or_default()
    ));
    let followed = router::followed_bases(cfg).contains(&router::base(&base));
    if use_after && follow.is_none() && !followed {
        say_line(
            "  and does not switch to it: no provider entry follows this router \
             (`follow_loaded = true`), so there is nothing a switch would reach — set it on the \
             entry that names this router, then `mecha model use`",
        );
    }
    if let Some((d, f)) = &shadowed {
        say_line(&format!(
            "  [providers.{d}] is in {}, which mecha does not edit — add `follow_loaded = true` \
             there for default runs to follow the model you load",
            f.display()
        ));
    }
    say_line("  then restarts the router — after any run using it has finished");
    if !yes {
        if !std::io::stdin().is_terminal() {
            bail!("nothing was installed: not a terminal to ask at — pass --yes to approve");
        }
        if !confirm(&format!("Install {id}?"))? {
            say_line("Nothing was changed.");
            return Ok(());
        }
        // The prompt can sit for as long as the owner likes; another
        // install may have started meanwhile (review of #727).
        one_at_a_time()?;
    }

    // Fetch and assemble, recording progress for the chip. Nothing below
    // holds the router until the restart (C6).
    let _guard = ProgressGuard(&home);
    let started_at = chrono::Utc::now();
    let pid = std::process::id();
    let mut last = (String::new(), 0u64, Instant::now());
    let mut progress = |phase: &str, done: u64, total: u64| {
        let moved = done.saturating_sub(last.1) >= 64 * 1_048_576
            || last.2.elapsed() >= Duration::from_secs(1);
        if phase != last.0 || moved {
            last = (phase.to_string(), done, Instant::now());
            let _ = catalog::progress_write(
                &home,
                &catalog::Progress {
                    id: id.to_string(),
                    phase: phase.to_string(),
                    done_bytes: done,
                    total_bytes: total,
                    pid,
                    started_at,
                },
            );
        }
    };
    progress("fetching", 0, offer.download_bytes);
    catalog::fetch(
        e,
        &here.hub,
        &mut |s| say_line(&format!("  {s}")),
        &mut progress,
    )
    .await
    .with_context(|| format!("nothing was installed: fetching {id}"))?;

    progress("restarting", 0, 0);
    let previous = router::models(&base)
        .await
        .filter(|l| router::readable(l))
        .and_then(|l| router::resident(&l).map(str::to_string));
    wait_quiet(cfg, &here.naming, &mut |l| say_line(&format!("  {l}"))).await?;
    // Recorded only once the wait is over, and taken back out if the restart
    // fails: a recorded id is rendered by the next router install, whoever
    // runs it, and with no provider table behind it (review of #727).
    let was_recorded = catalog::installed(&home)?.iter().any(|i| i == id);
    let kept = || {
        format!(
            "{id} is downloaded and the router was not restarted — `mecha model add {id}` again \
             finishes, with nothing fetched twice"
        )
    };
    catalog::record_add(&home, id).with_context(kept)?;
    let choice = router_unit::installed_choice(&home)
        .with_context(kept)?
        .context("the router's presets are gone")
        .with_context(kept)?;
    let restarted = router_unit::install(
        &here.m,
        cfg,
        &choice,
        &here.naming,
        &here.machine,
        &here.hub,
        &mut |s| say_line(&format!("  {s}")),
    )
    .await;
    if let Err(err) = restarted {
        let undone = if was_recorded {
            Ok(())
        } else {
            catalog::record_remove(&home, id).map(|_| ())
        };
        let mut msg = format!(
            "{id} is downloaded, and the router was not restarted onto it — `mecha model add \
             {id}` again restarts it, with nothing fetched twice"
        );
        if let Err(u) = undone {
            msg.push_str(&format!(
                "; its id also stayed in {} ({u:#})",
                catalog::record_path(&home).display()
            ));
        }
        return Err(err.context(msg));
    }

    // The router has restarted with the section by now: what is not written
    // below is said as such, not as a bare write error.
    let unwritten = |table: &str| {
        format!(
            "{id} is on the router, and [providers.{table}] was not written — run `mecha model \
             add {id}` again to write it"
        )
    };
    super::setup::write_provider_table(e.provider, TABLE_COMMENT, &settings)
        .with_context(|| unwritten(e.provider))?;
    if let Some(d) = &follow {
        super::setup::write_provider_table(d, TABLE_COMMENT, &[("follow_loaded", "true".into())])
            .with_context(|| unwritten(d))?;
    }
    drop(_guard);

    // The config just written is what the switch below must read.
    let cfg = super::model::load_config(global)?;
    // With no entry following the router a switch has nothing to reach:
    // the install stands, the plan already said it would not switch, and
    // `use_` would only refuse (review of #727).
    let switchable = router::followed_bases(&cfg).contains(&router::base(&base));
    if use_after && switchable {
        let out = if json {
            super::model::Out::Json
        } else {
            super::model::Out::Text
        };
        // Installed either way; a switch that cannot follow says so, rather
        // than a bare refusal that reads as the install failing (review of
        // #727).
        return super::model::use_(&cfg, e.provider, 600, false, out)
            .await
            .with_context(|| {
                format!(
                    "{id} is installed and on the router, and was not switched to — `mecha model \
                     use {}` switches to it",
                    e.provider
                )
            });
    }
    load_back(&cfg, &base, previous, &mut |l| say_line(l)).await;
    let unswitched = (use_after && !switchable).then_some(
        "not switched to: no provider entry follows this router — set `follow_loaded = true` \
         on the entry that names it, then `mecha model use`",
    );
    if json {
        println!(
            "{}",
            // `warning`: the key `serve` records for the chip's outcome line.
            serde_json::json!({ "installed": id, "provider": e.provider, "warning": unswitched })
        );
    } else if let Some(why) = unswitched {
        println!("\n{id} is installed, and {why}.");
    } else {
        println!(
            "\n{id} is installed. `mecha model use {}` (or the chip) loads it.",
            e.provider
        );
    }
    Ok(())
}

/// The router's restart loaded its base model; load back what was resident
/// before — when that was something else, still served, and followed.
async fn load_back(cfg: &Config, base: &str, previous: Option<String>, say: &mut dyn FnMut(&str)) {
    let now = router::models(base)
        .await
        .filter(|l| router::readable(l))
        .and_then(|l| router::resident(&l).map(str::to_string));
    let Some(prev) = previous.filter(|p| {
        Some(p) != now.as_ref() && router::namers(cfg, &router::base(base), p).next().is_some()
    }) else {
        return;
    };
    if let Err(err) = super::model::use_(cfg, &prev, 600, false, super::model::Out::Quiet).await {
        say(&format!("  {prev} was not loaded back: {err:#}"));
    }
}

/// What still names `[providers.<provider>]` — the default, a fallback, a
/// subagent, a trigger (`(name, provider)`) — said as what to change first.
/// Removing a table something names fails every run after, not this one; a
/// trigger's fails unattended (review of #727). An experiment's arm or judge
/// is deliberately not checked: it runs attended, so a missing provider
/// fails in front of whoever started it.
fn still_named(
    cfg: &Config,
    triggers: &[(String, Option<String>)],
    provider: &str,
) -> Option<String> {
    let mut by = Vec::new();
    if cfg.default_provider == provider {
        by.push("`default_provider`".to_string());
    }
    for (name, p) in &cfg.providers {
        if p.fallbacks.iter().any(|f| f == provider) {
            by.push(format!("the fallbacks of [providers.{name}]"));
        }
    }
    for s in &cfg.subagents {
        if s.provider.as_deref() == Some(provider) {
            by.push(format!("the subagent {:?}", s.name));
        }
    }
    for (name, p) in triggers {
        if p.as_deref() == Some(provider) {
            by.push(format!("the trigger {name:?}"));
        }
    }
    (!by.is_empty()).then(|| {
        format!(
            "{} still name{} [providers.{provider}] — point {} at another provider first",
            by.join(", "),
            if by.len() == 1 { "s" } else { "" },
            if by.len() == 1 { "it" } else { "them" }
        )
    })
}

/// `mecha model remove <id>`.
pub(super) async fn remove(global: &GlobalOpts, cfg: &Config, id: &str, yes: bool) -> Result<()> {
    let here = Here::read()?;
    let home = &here.m.mecha_home;
    let e =
        catalog::entry(id).with_context(|| format!("no chat model {id:?} in mecha's catalog"))?;
    if !catalog::installed(home)?.iter().any(|i| i == id) {
        bail!("{id} is not installed from the catalog here");
    }
    if let Some(why) = here.router_refusal()? {
        bail!("nothing was removed: {why}");
    }
    // `add` rewrites the same record; one catalog change at a time.
    if let Some(p) = catalog::progress_read(home) {
        bail!(
            "nothing was removed: a catalog install is running ({} — {}, pid {})",
            p.id,
            p.phase,
            p.pid
        );
    }
    // Read only: a check must not create the trigger store as a side effect.
    let triggers: Vec<(String, Option<String>)> =
        match mecha_core::trigger::TriggerStore::open_existing_default() {
            Some(store) => store
                .list()?
                .0
                .into_iter()
                .map(|t| (t.name, t.provider))
                .collect(),
            None => Vec::new(),
        };
    if let Some(why) = still_named(cfg, &triggers, e.provider) {
        bail!("nothing was removed: {why}");
    }
    let held: u64 = e.pins.iter().map(|p| p.file.bytes).sum();
    println!(
        "This takes {id} off the router and removes [providers.{}], then restarts the router \
         after any run using it has finished. Its files stay where they are — about {} in the \
         Hugging Face cache ({}) — for `mecha model add {id}` to use again.",
        e.provider,
        gib(held),
        here.hub.display()
    );
    if !yes {
        if !std::io::stdin().is_terminal() {
            bail!("nothing was removed: not a terminal to ask at — pass --yes");
        }
        if !confirm(&format!("Remove {id}?"))? {
            println!("Nothing was changed.");
            return Ok(());
        }
    }
    let base = here.naming.base();
    let previous = router::models(&base)
        .await
        .filter(|l| router::readable(l))
        .and_then(|l| router::resident(&l).map(str::to_string))
        .filter(|p| p != id);
    // The record changes only once the wait is over, and is put back if the
    // restart fails: a `remove` stopped or declined here can be run again
    // (review of #727).
    wait_quiet(cfg, &here.naming, &mut |l| println!("  {l}")).await?;
    catalog::record_remove(home, id)?;
    let choice = router_unit::installed_choice(home)?.context("the router's presets are gone")?;
    let restarted = router_unit::install(
        &here.m,
        cfg,
        &choice,
        &here.naming,
        &here.machine,
        &here.hub,
        &mut |s| println!("  {s}"),
    )
    .await;
    if let Err(err) = restarted {
        let mut msg = format!(
            "{id} was not removed: the router was not restarted without it — run this again"
        );
        if let Err(u) = catalog::record_add(home, id) {
            msg.push_str(&format!(
                "; its id could not be put back in {} ({u:#}) — `mecha model add {id}` restores it",
                catalog::record_path(home).display()
            ));
        }
        return Err(err.context(msg));
    }
    if cfg
        .providers
        .get(e.provider)
        .is_some_and(|p| p.model.as_deref() == Some(id))
        && !super::setup::remove_provider_table(e.provider, TABLE_COMMENT)?
    {
        // Moved to a project `mecha.toml` after `add` wrote it globally:
        // the owner's file, so said rather than edited (review of #727).
        println!(
            "[providers.{}] is not in {} — it is in a file mecha did not write it to; \
             remove it there.",
            e.provider,
            mecha_core::config::Config::global_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "the global config".into())
        );
    }
    let cfg = super::model::load_config(global)?;
    load_back(&cfg, &base, previous, &mut |l| println!("{l}")).await;
    println!("{id} is removed.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::config::ProviderConfig;

    fn router_table(model: &str, follow: bool) -> ProviderConfig {
        ProviderConfig {
            kind: "local".into(),
            base_url: Some("http://127.0.0.1:8080".into()),
            model: Some(model.into()),
            follow_loaded: follow,
            ..Default::default()
        }
    }

    /// The pin's fields, per slot, with `temperature` only where the preset
    /// sets one (R4).
    #[test]
    fn a_provider_table_comes_from_the_pin() {
        let gemma = catalog::entry("gemma-4-26b-a4b").unwrap();
        let s = provider_settings(gemma, 128, "http://127.0.0.1:8080").unwrap();
        let get = |k| s.iter().find(|(n, _)| *n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("model"), Some("\"gemma-4-26b-a4b\""));
        assert_eq!(get("context_window"), Some("32768"));
        assert_eq!(get("vision"), Some("true"));
        assert_eq!(get("temperature"), None);

        let u = catalog::entry("qwen3.6-35b-a3b-uncensored").unwrap();
        let s = provider_settings(u, 128, "http://127.0.0.1:8080").unwrap();
        let get = |k| s.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone());
        let g = mecha_core::recommend::chat_geometry(128).unwrap();
        assert_eq!(get("context_window"), Some((g.ctx / g.slots).to_string()));
        assert_eq!(get("temperature").as_deref(), Some("0.6"));
        assert!(provider_settings(u, 16, "x").is_err());
    }

    /// A table the default, a fallback or a subagent still names is not
    /// removed: the next run would find no such provider (review of #727).
    #[test]
    fn a_table_something_still_names_is_not_removed() {
        let mut cfg = Config {
            default_provider: "local".into(),
            ..Default::default()
        };
        cfg.providers
            .insert("qwen38".into(), router_table("qwen3.8-27b", false));
        assert_eq!(still_named(&cfg, &[], "qwen38"), None);
        cfg.default_provider = "qwen38".into();
        assert!(still_named(&cfg, &[], "qwen38")
            .unwrap()
            .contains("`default_provider` still names"));
        cfg.default_provider = "local".into();
        cfg.providers.insert(
            "local".into(),
            ProviderConfig {
                fallbacks: vec!["qwen38".into()],
                ..router_table("qwen3.6-35b-a3b", true)
            },
        );
        assert!(still_named(&cfg, &[], "qwen38")
            .unwrap()
            .contains("the fallbacks of [providers.local]"));
        cfg.providers.get_mut("local").unwrap().fallbacks.clear();
        let nightly = [("nightly".to_string(), Some("qwen38".to_string()))];
        assert!(still_named(&cfg, &nightly, "qwen38")
            .unwrap()
            .contains("the trigger \"nightly\""));
    }

    /// `follow_loaded` goes on the default only when it names this router and
    /// does not follow yet.
    #[test]
    fn follow_is_offered_only_on_a_default_that_names_this_router() {
        let mut cfg = Config {
            default_provider: "local".into(),
            ..Default::default()
        };
        cfg.providers
            .insert("local".into(), router_table("qwen3.6-35b-a3b", false));
        assert_eq!(follow_target(&cfg, 8080).as_deref(), Some("local"));
        cfg.providers.get_mut("local").unwrap().follow_loaded = true;
        assert_eq!(follow_target(&cfg, 8080), None);
        cfg.default_provider = "anthropic".into();
        assert_eq!(follow_target(&cfg, 8080), None);
    }

    /// A table of the entry's name that is not the entry's is the owner's.
    #[test]
    fn an_owner_s_table_of_the_same_name_is_not_written_over() {
        let e = catalog::entry("qwen3.8-27b").unwrap();
        let mut cfg = Config::default();
        assert_eq!(table_conflict(&cfg, e, 8080), None);
        cfg.providers
            .insert(e.provider.into(), router_table("qwen3.8-27b", false));
        assert_eq!(table_conflict(&cfg, e, 8080), None);
        cfg.providers
            .insert(e.provider.into(), router_table("something-else", false));
        assert!(table_conflict(&cfg, e, 8080).is_some());
    }
}
