//! `mecha features` — which optional parts of mecha are on, the way to turn
//! on each of the rest, and `enable` / `disable` to answer the switches
//! (`docs/FEATURES-DESIGN.md`).
//!
//! The human half of [`mecha_core::feature`]. **No network for reading**:
//! the listing, `--probe` and `plan` read the config and the disk, never a
//! server, because several servers here start the moment they are asked.
//! `enable` is the one verb that fetches — what a feature needs and this
//! machine lacks, from pinned sources, on a yes (`mecha_core::install`).
//! The switches are written in place
//! into the global `config.toml` (`feature::write_switches`), never through
//! a rewrite that would drop comments or a newer build's key.

use anyhow::{Context, Result};
use mecha_core::config::Config;
use mecha_core::feature::{self, Feature, Row, State};
use mecha_core::recommend::{self, Budget, Floor, Machine, Peak, Sum};
use mecha_core::sidecar;

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
    /// Machine output: one object per feature, `state` naming which of
    /// on / off / blocked / unready / unknown it is.
    #[arg(long)]
    pub json: bool,
    /// Add up the memory of every model the shown features need — any not
    /// off or blocked, so one switched on but not yet set up counts too —
    /// against this machine's (the card's and the host's, if it has a card).
    /// Reads `/proc/meminfo` and `nvidia-smi`; asks no server.
    #[arg(long)]
    pub probe: bool,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Switch features on in the global `config.toml`'s `[features]` table.
    ///
    /// Refuses a part (`ocr`, `dictate`: turn on its parent and its setting)
    /// and a feature whose dependency is off, printing the command that
    /// switches both.
    Enable {
        #[arg(required = true)]
        ids: Vec<String>,
        /// Write the switch only: install nothing, even what the feature
        /// needs and this machine lacks (`mecha features plan <id>` lists it).
        #[arg(long)]
        no_install: bool,
        /// With `image`: also fetch the turbo model (6.8 GiB), which a
        /// persona's picture settings can choose. Without it, the turbo
        /// model is fetched only once a persona chooses it.
        #[arg(long)]
        turbo: bool,
        /// Rewrite and restart the services mecha installed for these
        /// features, as an install would — what `enable` already offers for
        /// a service an older mecha wrote. Nothing current is downloaded.
        #[arg(long)]
        reinstall: bool,
    },
    /// Switch features off in the global `config.toml`'s `[features]` table.
    /// Their settings stay, for when they are switched back on.
    Disable {
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// What a feature runs beside mecha, whether this machine already has
    /// it, and every model file it would download. Read-only: nothing is
    /// installed, and no server is asked (a part plans as its parent).
    Plan {
        id: String,
        /// Machine output: the plan as one object.
        #[arg(long)]
        json: bool,
        /// Hash every model file that is not a blob named by its sha256 —
        /// a hand-placed one — instead of reporting it unverified. Reads
        /// the whole file (seconds for the 22 GB chat model).
        #[arg(long)]
        verify: bool,
    },
    /// Internal: run a program with its output in a log that rotates at a
    /// cap — the journal's part, for every launchd agent mecha installs on
    /// macOS (`mecha_core::log_cap`). Run by launchd, not by hand.
    #[command(hide = true)]
    LogCap {
        #[arg(long)]
        log: std::path::PathBuf,
        #[arg(long, default_value_t = mecha_core::log_cap::MAX_MB)]
        max_mb: u64,
        /// The program and its arguments, after `--`.
        #[arg(last = true, required = true)]
        program: Vec<String>,
    },
    /// Internal: hold a port and start the server behind it on its first
    /// request, stopping it when idle — the program of the embeddings, OCR
    /// and image (ComfyUI) servers' launchd agents on macOS
    /// (`mecha_core::on_demand`). Run by launchd, not by hand.
    #[command(hide = true)]
    OnDemand {
        #[arg(long)]
        listen: std::net::SocketAddr,
        #[arg(long)]
        backend: u16,
        #[arg(long, default_value_t = 600)]
        idle_secs: u64,
        #[arg(long)]
        launcher: std::path::PathBuf,
        /// What answers 2xx once the server is ready.
        #[arg(long, default_value = "/health")]
        ready_path: String,
    },
}

pub async fn execute(args: Args) -> Result<()> {
    // Checked here, not with clap's `args_conflicts_with_subcommands`: that
    // also refuses the global flags (`mecha features --yes enable graph`).
    if args.probe && args.cmd.is_some() {
        anyhow::bail!("`--probe` reads the machine; it does not go with `enable` or `disable`");
    }
    match args.cmd {
        Some(Cmd::Enable {
            ids,
            no_install,
            turbo,
            reinstall,
        }) => return enable(&ids, no_install, turbo, reinstall, false).await,
        Some(Cmd::Disable { ids }) => {
            set(&ids, false)?;
            println!("{RESTART_NOTE}");
            return Ok(());
        }
        Some(Cmd::Plan { id, json, verify }) => return plan(&id, json, verify),
        Some(Cmd::LogCap {
            log,
            max_mb,
            program,
        }) => {
            let code = mecha_core::log_cap::run(mecha_core::log_cap::Spec {
                log,
                max_bytes: max_mb.max(1) << 20,
                program,
            })
            .await?;
            crate::exit_with(code);
        }
        Some(Cmd::OnDemand {
            listen,
            backend,
            idle_secs,
            launcher,
            ready_path,
        }) => {
            return mecha_core::on_demand::run(mecha_core::on_demand::Spec {
                listen,
                backend,
                idle: std::time::Duration::from_secs(idle_secs),
                // `mecha-wait-healthy`'s budget for these servers' start.
                start_timeout: std::time::Duration::from_secs(180),
                launcher,
                // A path, always: `system_stats` alone would build
                // `http://127.0.0.1:18188system_stats`.
                ready_path: if ready_path.starts_with('/') {
                    ready_path
                } else {
                    format!("/{ready_path}")
                },
            })
            .await;
        }
        None => {}
    }
    let home = mecha_core::work::mecha_home()?;
    // The global config only, as `setup` and `doctor` read it: which
    // features an install has is a property of the machine, and `[features]`
    // is stripped from project layers anyway.
    let cfg = Config::load_global()?;
    let facts = feature::Facts::read(&home, &cfg);
    let rows = feature::all(&facts);
    for key in feature::unknown_switches(&cfg) {
        eprintln!("mecha: `[features] {key}` is not a feature this build knows — a newer build's, or a typo; ignored");
    }
    // Said where the owner asks what is on, never on every start: a line an
    // older `enable` wrote that nothing reads now (review of #695 — an
    // `incognito = false` written for a reason is otherwise overridden with
    // no trace).
    for key in feature::RETIRED_SWITCHES
        .iter()
        .filter(|k| cfg.features.get(k).is_some())
    {
        eprintln!(
            "mecha: `[features] {key}` is no longer read — `{key}` is part of `{}` now; the line \
             can go",
            Feature::parse(key).map_or("another feature", |f| f.switch_owner().id())
        );
    }
    if args.probe {
        let shown: Vec<Feature> = rows.iter().filter(|r| r.shown).map(|r| r.id).collect();
        let budget = recommend::budget(recommend::Machine::read()?, &shown);
        if args.json {
            println!("{}", serde_json::to_string_pretty(&budget)?);
        } else {
            print!("{}", render_budget(&budget));
        }
        return Ok(());
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    print!("{}", render(&rows, &feature::announcements(&facts)));
    Ok(())
}

/// `mecha features enable` (F7, FEATURES-DESIGN.md §10.2 item 3): the plan
/// for each feature named, and — when a sidecar it needs is missing here and
/// mecha can install it — the install, on one yes, at a terminal. The switch
/// is written only after every install and its check succeeded; answering no
/// writes nothing. Without a terminal, an enable that would install refuses,
/// naming `--no-install`; one with nothing to install writes the switch as
/// before.
/// `yes` is an answer already given: the guided `mecha setup` lists every
/// install with its size and asks once (ruling F14), so this neither asks
/// again nor refuses for want of a terminal to ask at.
/// `turbo` is `--turbo`: the image server's install fetches the turbo model
/// too, as it does unasked once a persona's picture settings choose it.
/// `reinstall` is `--reinstall`: every installed sidecar with services is
/// offered as one an older mecha wrote would be (`PlannedSidecar::stale`).
pub(super) async fn enable(
    ids: &[String],
    no_install: bool,
    turbo: bool,
    reinstall: bool,
    yes: bool,
) -> Result<()> {
    let on = enable_unwired(ids, no_install, turbo, reinstall, yes).await?;
    // The switches are written whatever the wiring does: the note is said
    // either way (review of #698).
    let wired = mcp_entries(&on);
    if !on.is_empty() {
        println!("{RESTART_NOTE}");
    }
    wired
}

/// [`enable`] up to the `[[mcp]]` wiring, returning the features it switched
/// on: the guided `mecha setup` enables one feature at a time (#631) and
/// wires the server features of every yes together after the last, so one
/// `[outbox]` decision covers mail and docs alike rather than the second
/// finding the first's fresh table (review of #669, pass 4).
pub(super) async fn enable_unwired(
    ids: &[String],
    no_install: bool,
    turbo: bool,
    reinstall: bool,
    yes: bool,
) -> Result<Vec<Feature>> {
    use mecha_core::comfyui_unit as comfy;
    use mecha_core::install;
    use mecha_core::sidecar::SidecarState;
    use std::io::IsTerminal;

    let cfg = Config::load_global()?;
    let features = feature::plan_enable(&cfg, ids).map_err(anyhow::Error::msg)?;
    if turbo && !features.contains(&Feature::Image) {
        anyhow::bail!("`--turbo` is the image model's: `mecha features enable image --turbo`");
    }
    if turbo && no_install {
        anyhow::bail!("`--turbo` fetches a model and `--no-install` installs nothing; drop one");
    }
    if reinstall && no_install {
        anyhow::bail!(
            "`--reinstall` rewrites services and `--no-install` installs nothing; drop one"
        );
    }
    // Refused rather than ignored: past here, a platform with no installer
    // never reaches the block that would say so (review of #650).
    if turbo && !install::installable(comfy::ID) {
        anyhow::bail!(
            "`--turbo` fetches into the image server mecha installs, and mecha installs none on \
             this platform; put {} in your ComfyUI's models/diffusion_models/",
            mecha_core::sampling::turbo_model()
        );
    }
    // The machine is read only when some sidecar these features need is one
    // mecha installs and this machine would run: `enable messages` beside a
    // hosted provider stays a switch write, with no GPU probe to wait on.
    let chat_here = install::chat_runs_here(&cfg);
    let can_install = features.iter().any(|f| install::may_offer(*f, chat_here));
    if !no_install && can_install {
        let m = sidecar::Machinery::real()?;
        let machine = recommend::Machine::read()?;
        let hub = mecha_core::fetch::hub_dir()?;
        // (sidecar id, its label, the feature whose plan names it, its
        // download where known before installing)
        let mut todo: Vec<(&'static str, &'static str, Feature, Option<u64>)> = Vec::new();
        // The rows offered only because their services are an older build's:
        // said as a refresh, not an install.
        let mut refreshing: Vec<&'static str> = Vec::new();
        let mut router_said = false;
        // The driver, read once for the whole command and only if some plan
        // would install the engine.
        let nvidia = std::cell::OnceCell::new();
        let read_nvidia = || *nvidia.get_or_init(mecha_core::engine::read_nvidia);
        // The image server as the plan found it: whether a turbo fetch has
        // an install of mecha's to land in.
        let mut comfy_found: Option<(&'static str, SidecarState)> = None;
        // A server on ComfyUI's port that no evidence names reads as
        // missing, and is left alone here: someone else's all the same.
        let mut comfy_left_alone = false;
        for f in &features {
            let mut p = sidecar::plan(*f, &m, &machine, &hub, false)?;
            install::price(&mut p, chat_here, read_nvidia);
            if reinstall {
                for s in p.sidecars.iter_mut().filter(|s| {
                    matches!(s.state, SidecarState::Installed)
                        && mecha_core::user_service::rev(s.id).is_some()
                }) {
                    s.stale = true;
                    // Never `enable`'s to rewrite (F13): said, not dropped.
                    if s.id == mecha_core::router_unit::ID && chat_here && !router_said {
                        println!(
                            "The chat router is not reinstalled here: `mecha setup chat` \
                             refreshes it."
                        );
                        router_said = true;
                    }
                }
            }
            if let Some(s) = p.sidecars.iter().find(|s| s.id == comfy::ID) {
                comfy_found = Some((s.label, s.state.clone()));
            }
            // A check that could not run is never installed over — and
            // never passed over in silence either.
            for s in &p.sidecars {
                if let SidecarState::Unknown { why } = &s.state {
                    eprintln!(
                        "mecha: {} could not be checked ({why}); nothing is installed over it",
                        s.label
                    );
                }
            }
            for s in install::offered(&p, chat_here) {
                // Someone's own server where the install would put one is
                // used, not installed over — never offered, so the switch
                // is still written (review of #638).
                if let Some(why) = install::left_alone(s.id, &m)? {
                    println!("{}: {why}", s.label);
                    comfy_left_alone |= s.id == comfy::ID;
                    continue;
                }
                if !todo.iter().any(|(id, ..)| *id == s.id) {
                    todo.push((s.id, s.label, p.feature, s.bytes));
                    if s.stale
                        && matches!(s.state, SidecarState::Installed)
                        && !install::model_gone(&p, s.id)
                    {
                        refreshing.push(s.id);
                    }
                }
            }
        }
        // A machine no pinned build fits is told so before it is asked
        // anything: the answer is to fix the driver or install the toolchain
        // a build here needs, not to say yes.
        let built_here = if todo.iter().any(|(id, ..)| *id == "llama") {
            mecha_core::engine::choose_here()
                .map_err(|why| {
                    anyhow::anyhow!(
                        "enabling {} needs the llama.cpp engine, and {why}",
                        ids.join(" ")
                    )
                })?
                .is_built()
        } else {
            false
        };
        // A machine the image server's pinned torch cannot run on is not
        // offered it — and not refused the switch either: images may come
        // from a ComfyUI the owner runs, stopped right now, and the switch
        // is what turned them on before ComfyUI had an installer (review of
        // #645).
        if todo
            .iter()
            .any(|(id, ..)| *id == mecha_core::comfyui_unit::ID)
        {
            if let Err(why) = mecha_core::comfyui_unit::fits(
                std::env::consts::OS,
                std::env::consts::ARCH,
                read_nvidia(),
            ) {
                println!(
                    "ComfyUI is not installed: {why}. Images switch on for one you run yourself."
                );
                todo.retain(|(id, ..)| *id != mecha_core::comfyui_unit::ID);
            }
        }
        // The graph's MCP server is built with cargo: a machine without it is
        // told so, and graph stays switched on for a server installed later
        // — the footnote below says it is not working until then (§13).
        if todo.iter().any(|(id, ..)| *id == mecha_core::graph_mcp::ID) {
            if let Err(why) = mecha_core::graph_mcp::fits() {
                println!(
                    "{} is not installed: {why}.",
                    mecha_core::graph_mcp::PROGRAM
                );
                todo.retain(|(id, ..)| *id != mecha_core::graph_mcp::ID);
            }
        }
        // The mail servers likewise (§15).
        if todo.iter().any(|(id, ..)| *id == mecha_core::mail_mcp::ID) {
            if let Err(why) = mecha_core::mail_mcp::fits() {
                println!("The mail and documents servers are not installed: {why}.");
                todo.retain(|(id, ..)| *id != mecha_core::mail_mcp::ID);
            }
        }
        // Likewise the speech server: voice may be spoken by a server the
        // owner runs — Chatterbox, or Breeze by hand — so a machine that
        // cannot build the engine is not offered it, and keeps the switch.
        if todo
            .iter()
            .any(|(id, ..)| *id == mecha_core::speech_unit::ID)
        {
            if let Err(why) = mecha_core::speech_unit::fits(std::env::consts::OS, read_nvidia()) {
                println!(
                    "The speech server is not installed: {why}. Voice switches on for one you \
                     run yourself."
                );
                todo.retain(|(id, ..)| *id != mecha_core::speech_unit::ID);
            }
        }
        // The turbo model, on demand (the owner, 2026-10-11: until it has
        // been tested more): fetched into an install of mecha's — a fresh
        // one, or one already here, reinstalled to link it — and only said
        // for a ComfyUI someone else runs.
        // (why, what it downloads, whether an installed server is
        // reinstalled — restarted — for it)
        let mut fetch_turbo: Option<(String, u64, bool)> = None;
        if features.contains(&Feature::Image) {
            // `--turbo` fetches the pinned file, which a picture loads only
            // when `[image] turbo_model` names it (review of #650).
            if let Some(own) = cfg
                .image
                .as_ref()
                .map(|i| i.turbo_model.as_str())
                .filter(|t| turbo && *t != mecha_core::sampling::turbo_model())
            {
                println!(
                    "[image] turbo_model is {own}, so a turbo picture loads that, not the {} \
                     this fetches.",
                    mecha_core::sampling::turbo_model()
                );
            }
            let installing = todo.iter().any(|(id, ..)| *id == comfy::ID);
            let wanted = comfy::turbo_wanted(
                &m.mecha_home,
                turbo,
                cfg.image.as_ref().map(|i| i.turbo_model.as_str()),
            );
            // Asked only when turbo is in play, and an unreadable cache is
            // one without it — the full download offered, never an `enable
            // image` failed over a file it does not want (review of #650).
            let cached = wanted.is_some()
                && mecha_core::fetch::cached(
                    &hub,
                    comfy::TURBO_REPO,
                    comfy::TURBO_REVISION,
                    comfy::turbo_file(),
                    false,
                )
                .is_ok_and(|c| c == mecha_core::fetch::Cached::Verified);
            let here = comfy::turbo_here(&m.mecha_home);
            let facts = TurboFacts {
                wanted,
                found: comfy_found.as_ref().map(|(_, state)| state),
                installing,
                cached,
                linked_here: here == comfy::TurboHere::Linked,
                placed_here: here == comfy::TurboHere::Placed,
                left_alone: comfy_left_alone,
                asked: turbo,
                yes,
            };
            let unfit =
                || comfy::fits(std::env::consts::OS, std::env::consts::ARCH, read_nvidia()).err();
            match turbo_step(facts, unfit) {
                TurboStep::Nothing => {}
                TurboStep::Say(line) => println!("{line}"),
                TurboStep::Fetch {
                    why,
                    download,
                    reinstall,
                } => {
                    // Its size rides on its own clause, not the row's total:
                    // a row re-offered for a lost model carries none (review
                    // of #650).
                    match (reinstall, &comfy_found) {
                        (true, Some((label, _))) => {
                            todo.push((comfy::ID, *label, Feature::Image, None));
                            fetch_turbo = Some((why, download, true));
                        }
                        (true, None) => {}
                        (false, _) => fetch_turbo = Some((why, download, false)),
                    }
                }
            }
        }
        // The turbo model is optional: re-offered alone, it never costs the
        // switch (#645; review of #650). Without a terminal its reinstall
        // is left for one, and a no below keeps the switch.
        if let Some((why, _, true)) = &fetch_turbo {
            if !yes && !std::io::stdin().is_terminal() {
                println!(
                    "The turbo model is not fetched ({why}): linking it restarts the image \
                     server, which needs a yes at a terminal — `mecha features enable image \
                     --turbo`."
                );
                todo.retain(|(id, ..)| *id != comfy::ID);
                fetch_turbo = None;
            }
        }
        if reinstall && todo.is_empty() {
            println!("Nothing to reinstall here for {}.", ids.join(" "));
        }
        let turbo_only = matches!(fetch_turbo, Some((.., true))) && todo.len() == 1;
        if !todo.is_empty() {
            let names: Vec<&str> = todo.iter().map(|(_, label, ..)| *label).collect();
            let list = names.join(", ");
            if !yes && !std::io::stdin().is_terminal() {
                anyhow::bail!(
                    "enabling {} would install {list}; run it at a terminal to answer, or pass \
                     --no-install to write the switch only",
                    ids.join(" ")
                );
            }
            let services = if cfg!(target_os = "macos") {
                "launchd agents"
            } else {
                "systemd user units"
            };
            // Every row a refresh: nothing is installed, so the header does
            // not say so (review of #691, pass 5).
            // Asked of what is left to do: a row dropped above (not offered
            // here, someone else's) is no longer counted (review of #691,
            // pass 6).
            if todo.iter().all(|(id, ..)| refreshing.contains(id)) {
                println!(
                    "Enabling {} refreshes what an older mecha installed — its {services}, \
                     rewritten and restarted, and started once to check them:",
                    ids.join(" ")
                );
            } else {
                println!(
                    "Enabling {} installs, from pinned sources — into ~/.mecha/sidecars/, and, \
                     for a server, its {services}, started once to check them:",
                    ids.join(" ")
                );
            }
            let turbo_line = match &fetch_turbo {
                Some((why, download, reinstall)) => format!(
                    ", {} the turbo model ({}; {why})",
                    if *reinstall {
                        "restarted to link"
                    } else {
                        "with"
                    },
                    if *download == 0 {
                        "in the cache".to_string()
                    } else {
                        format!("{} to fetch", bytes_text(*download))
                    }
                ),
                None => String::new(),
            };
            for (id, label, _, bytes) in &todo {
                // A build here is said as one: its download is the source,
                // and the compile is the wait (review of #643).
                let built = if refreshing.contains(id) {
                    // Its build and models are current: only the services
                    // are written again.
                    let why = if reinstall {
                        "reinstalled, as asked"
                    } else {
                        "its service is from an older mecha"
                    };
                    if *id == comfy::ID && !turbo_line.is_empty() {
                        println!("  {label} — {why}: rewritten and restarted{turbo_line}");
                    } else {
                        println!(
                            "  {label} — {why}: rewritten and restarted; nothing current is \
                             downloaded again"
                        );
                    }
                    continue;
                } else if *id == "llama" && built_here {
                    ", then built here — no release fits this machine; minutes for the CPU, \
                     longer for CUDA"
                } else if *id == mecha_core::speech_unit::ID {
                    // Always a build, and the weights' licence is said
                    // before the yes, not only at the fetch (review of #649).
                    if cfg!(target_os = "macos") {
                        ", built here from its pinned commit (under a minute on Apple silicon), \
                         then its model; Breeze TTS 2's weights are licensed for research and \
                         non-commercial use only"
                    } else {
                        ", built here from its pinned commit (about seven minutes on a GB10), \
                         then its model; Breeze TTS 2's weights are licensed for research and \
                         non-commercial use only"
                    }
                } else if *id == comfy::ID {
                    &turbo_line
                } else if *id == mecha_core::graph_mcp::ID {
                    ", built here with cargo from its pinned commit — a few minutes"
                } else if *id == mecha_core::mail_mcp::ID {
                    ", built here with cargo from this mecha's own source — a few minutes"
                } else {
                    ""
                };
                match bytes {
                    Some(b) => println!("  {label} — {}{built}", bytes_text(*b)),
                    None => println!("  {label}{built}"),
                }
            }
            let mut plans: Vec<&str> = todo.iter().map(|(_, _, f, _)| f.id()).collect();
            plans.sort_unstable();
            plans.dedup();
            println!(
                "(`mecha features plan {}` shows each piece, and the models and engine to download.)",
                plans.join("` / `mecha features plan ")
            );
            let answer = if yes {
                "y".to_string()
            } else {
                print!("Install now? [y/N] ");
                use std::io::Write;
                std::io::stdout().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                answer
            };
            let go = matches!(answer.trim(), "y" | "Y" | "yes");
            if !go && turbo_only {
                println!(
                    "The turbo model is not fetched; `mecha features enable image --turbo` \
                     fetches it."
                );
            } else if !go {
                println!(
                    "Nothing written. `mecha features enable {} --no-install` writes the switch without installing.",
                    ids.join(" ")
                );
                return Ok(Vec::new());
            }
            for (id, label, ..) in todo.iter().filter(|_| go) {
                if refreshing.contains(id) {
                    println!("Refreshing {label}…");
                } else {
                    println!("Installing {label}…");
                }
                let turbo = *id == comfy::ID && fetch_turbo.is_some();
                install::install(id, &m, &machine, &hub, turbo, &mut |s| println!("  {s}"))
                    .await
                    .with_context(|| {
                        format!(
                            "installing {label} — nothing was switched on; run the same command again to resume"
                        )
                    })?;
            }
        }
    }
    // Nothing here could be installed at all: `--reinstall` still says it
    // found nothing (review of #691, pass 5).
    if reinstall && !can_install {
        println!("Nothing to reinstall here for {}.", ids.join(" "));
    }
    set(ids, true)?;
    if features.contains(&Feature::Image) {
        image_table()?;
    }
    if features.contains(&Feature::Documents) {
        // Said, never fatal: the switch is already written, and a failure
        // here must not stop the steps after it (graph's `[[mcp]]` entry
        // among them) and leave a half-on install — the state §13 exists to
        // end (review of #683).
        if let Err(e) = documents_table().await {
            println!(
                "No [documents] table written: {e:#}. `mecha features enable documents` again \
                 writes it once that is fixed."
            );
        }
    }
    if features.contains(&Feature::Voice) {
        voices_dir()?;
    }
    Ok(features)
}

/// `[voice] voices_dir` for a speech server already installed: the installer
/// writes it, but an install that has the server never runs the installer
/// again, so `enable voice` writes it too — `image_table`'s two halves
/// (review of #669). Nothing where no speech server is installed.
fn voices_dir() -> Result<()> {
    let path = Config::global_path().context("no mecha home to find config.toml in")?;
    let Some(home) = path.parent() else {
        return Ok(());
    };
    let voices = mecha_core::speech_unit::voices_dir(home);
    if !voices.is_dir() {
        return Ok(());
    }
    let outcome = mecha_core::speech_unit::write_voices_dir(&path, &voices);
    if let Some(line) = mecha_core::speech_unit::voices_dir_said(&outcome, &voices, &path) {
        println!("{line}");
    }
    Ok(())
}

/// A server feature's switch alone leaves it off — `server_state` wants an
/// `[[mcp]]` entry running the program — so `mecha setup`'s yes to mail ended
/// at "add an [[mcp]] entry" instead of a sign-in page (the owner, 2026-10-12).
/// Written here where none runs it, every feature of the command in one write
/// so one `[outbox]` decision covers them all (review of #669), each program
/// found on PATH (or in `~/.cargo/bin`) and recorded by its absolute path, so
/// a unit's short PATH still finds it.
pub(super) fn mcp_entries(features: &[Feature]) -> Result<()> {
    let path = Config::global_path().context("no mecha home to find config.toml in")?;
    let mut items = Vec::new();
    let mut seen = Vec::new();
    for f in features {
        let Some(wiring) = feature::mcp_wiring(*f) else {
            continue;
        };
        // One pass per feature: a dependency two yeses share is wired, or
        // said to be missing, once.
        if seen.contains(f) {
            continue;
        }
        seen.push(*f);
        let found = mcp_command(
            wiring.program,
            mecha_core::onboarding::find_program(wiring.program),
            path.parent(),
        );
        match found {
            Some(command) => items.push((*f, wiring, command)),
            // Where `enable` is the installer, the feature is said to be not
            // working rather than left as a footnote under "enabled" (§13).
            // A copy from another build was passed over: said as that, not
            // as "not installed" — it is installed, and an existing entry may
            // still run it (review of #699).
            None if wiring.install.starts_with("mecha features enable")
                && mecha_core::onboarding::find_program(wiring.program)
                    .is_some_and(|p| mecha_core::sidecar::stale_copy(wiring.program, &p)) =>
            {
                println!(
                    "{} is switched on, but the `{}` on your PATH is another build's and the \
                     matching one is not installed, so no [[mcp]] entry was written and none \
                     was changed — `{}` builds it",
                    f.id(),
                    wiring.program,
                    wiring.install
                )
            }
            None if wiring.install.starts_with("mecha features enable") => println!(
                "{} is switched on, but not working: `{}` is not installed, so no [[mcp]] entry \
                 was written — `{}` installs it",
                f.id(),
                wiring.program,
                wiring.install
            ),
            None => println!(
                "`{}` is not installed, so no [[mcp]] entry was written — `{}`, then `mecha \
                 features enable {}` again writes it",
                wiring.program,
                wiring.install,
                f.id()
            ),
        }
    }
    if items.is_empty() {
        return Ok(());
    }
    let refs: Vec<(&feature::McpWiring, &std::path::Path)> =
        items.iter().map(|(_, w, c)| (w, c.as_path())).collect();
    // An operator rule on a tool this would route, where no `[outbox]` exists
    // yet: no `[outbox]` is written and the conflict is named — the owner's
    // rules, like the owner's tables, are never overridden (review of #669).
    // Every decision counts: an `allow`/`prompt` beside the route refuses
    // every start (`setup::live_rules`), and a `forbid` is never reached
    // while it is on — the call becomes a draft a person can release, so the
    // rule meant as "never sends" would quietly stop meaning it. The routes
    // are matched as the entry serving each feature names them, so the write
    // decides (review of #669, pass 4).
    let cfg = Config::load_global()?;
    let rules: Vec<&mecha_core::policy::RuleConfig> =
        cfg.rules.iter().filter(|r| !r.from_project).collect();
    let done = feature::write_mcp_entries(&path, &refs, &|t| rules.iter().any(|r| r.tool == t))?;
    // One `[outbox]` covers every server of the call, so a rule on one
    // feature's send leaves the others' unstaged too: said for each, never
    // left to a silent absence (review of #669, pass 5).
    let held = done.iter().any(|d| !d.ruled.is_empty());
    for ((f, wiring, command), done) in items.iter().zip(done) {
        // The feature's own line first, what hangs under it after (review of
        // #669, pass 9).
        if done.wrote_entry {
            println!(
                "Wrote [[mcp]] `{}` (running {}, its results read as untrusted) into {}",
                wiring.name,
                command.display(),
                path.display()
            );
        }
        for tool in &done.ruled {
            let why = match rules.iter().find(|r| &r.tool == tool).map(|r| &r.decision) {
                Some(mecha_core::policy::RuleDecision::Forbid) => {
                    "forbids it, and staging would turn that into a draft you can release — it \
                     stays refused, as your rule says"
                }
                _ => {
                    "judges it, and every start refuses an `allow` or `prompt` on a call that \
                     is staged"
                }
            };
            println!(
                "  no [outbox] was written: your [[rule]] on `{tool}` {why}. To have the sends \
                 staged for your review instead, remove the rule, then `mecha features enable \
                 {}` again",
                f.id()
            );
        }
        // The ruled feature's own other sends included: a rule is per tool,
        // the `[outbox]` it held per command (review of #669, pass 6).
        let unstaged: Vec<&str> = done
            .routes
            .iter()
            .filter(|t| !done.ruled.contains(t))
            .map(String::as_str)
            .collect();
        if held && !unstaged.is_empty() {
            println!(
                "  {}'s calls that send or write and no rule names ({}) are not staged \
                 either — one [outbox] covers every server, and the rule above kept it \
                 unwritten",
                wiring.name,
                unstaged.join(", ")
            );
        }
        if let Some(old) = &done.repointed {
            println!(
                "  the [[mcp]] entry for {} ran {old}, another build's — it now runs {}, which \
                 matches this mecha",
                wiring.name,
                command.display()
            );
        }
        if let Some(other) = &done.runs_other {
            println!(
                "  the [[mcp]] entry called `{}` runs `{other}`, not {}, so it is left as it is \
                 and none is added beside it (two entries of one name shadow each other's \
                 tools) — {} reads as set up only with an entry running {}: point yours at it, \
                 or rename it and `mecha features enable {}` again",
                wiring.name,
                wiring.program,
                wiring.name,
                wiring.program,
                f.id()
            );
        }
        if let Some(name) = &done.disabled {
            println!(
                "  the [[mcp]] entry `{name}` running {} is disabled, so {} reads off until it \
                 runs — remove `disabled = true` from [[mcp]] `{name}`",
                wiring.program,
                f.id()
            );
        }
        // Never edited — a store built at another width would break — and
        // said with the lines to add (review of #698).
        if !done.env_missing.is_empty() {
            let lines: Vec<String> = done
                .env_missing
                .iter()
                .map(|(k, v)| format!("{k} = \"{v}\""))
                .collect();
            println!(
                "  the [[mcp]] entry for {} is left as it is — it does not tell the server mecha's \
                 embeddings server, so it assumes another width; unless its store was built with \
                 another embedder, add under [mcp.env]: {}",
                wiring.name,
                lines.join(", ")
            );
        }
        if done.untrusted_missing && done.runs_other.is_none() {
            println!(
                "  the [[mcp]] entry for {} is left as it is — it needs \
                 [mcp.capabilities] untrusted_input = true, or what it reads never arms the \
                 interlock",
                wiring.name
            );
        }
        if done.wrote_outbox {
            println!(
                "  {}'s calls that send or write ({}) are staged in the outbox for you to review \
                 (`mecha outbox`) — held for you rather than refused when a run has read \
                 something untrusted",
                wiring.name,
                done.routes.join(", ")
            );
        }
        if !done.outbox_missing.is_empty() {
            println!(
                "  the [outbox] table is left as it is — for {}'s sends to be staged for review \
                 rather than run, add to its tools: {}",
                wiring.name,
                done.outbox_missing.join(", ")
            );
        }
        if !done.ruled_unrouted.is_empty() {
            println!(
                "  not advised for the [outbox]: {} — your [[rule]] judges it, and a staged call \
                 is never judged by rules (an `allow` or `prompt` beside it stops every start; a \
                 `forbid` would become a draft you can release)",
                done.ruled_unrouted.join(", ")
            );
        }
    }
    Ok(())
}

/// The program an `[[mcp]]` entry runs, by its absolute path: one on the
/// PATH, unless it is one of mecha's own servers from another build (§15);
/// else the build mecha installed into its own tree, which no PATH names.
/// The program a remedy names, as setup runs it: for one of mecha's own
/// servers, the build its `[[mcp]]` entry would run — so a sign-in after
/// `enable` is the matching server's, never a stale copy on the PATH whose
/// `auth` takes other arguments (review of #699). Anything else as named.
/// A stale copy with nothing matching installed is refused, never run as a
/// fallback (review of #699): its `auth` is the one that took other
/// arguments.
pub(crate) fn remedy_program(program: &str) -> Result<std::ffi::OsString, String> {
    let own = mecha_core::mail_mcp::PROGRAMS.contains(&program)
        || program == mecha_core::graph_mcp::PROGRAM;
    if !own {
        return Ok(program.into());
    }
    let home = mecha_core::work::mecha_home().ok();
    let on_path = mecha_core::onboarding::find_program(program);
    if let Some(found) = mcp_command(program, on_path.clone(), home.as_deref()) {
        return Ok(found.into());
    }
    match on_path {
        Some(p) if mecha_core::sidecar::stale_copy(program, &p) => {
            // The way to the matching build, asked rather than assumed: a
            // build that cannot fetch its own source is told what can
            // (review of #699).
            let fits = if program == mecha_core::graph_mcp::PROGRAM {
                mecha_core::graph_mcp::fits().map(|_| ())
            } else {
                mecha_core::mail_mcp::fits().map(|_| ())
            };
            Err(stale_refusal(program, &p, fits.err()))
        }
        _ => Ok(program.into()),
    }
}

/// What `remedy_program` says of a stale copy it will not run: the command
/// that builds the matching one, or why that cannot run here.
fn stale_refusal(program: &str, copy: &std::path::Path, unfit: Option<String>) -> String {
    let feature = if program == mecha_core::graph_mcp::PROGRAM {
        "graph"
    } else if program == "mecha-docs" {
        "docs"
    } else {
        "mail"
    };
    let way = match unfit {
        None => format!("`mecha features enable {feature}` builds it"),
        Some(why) => why,
    };
    format!(
        "the `{program}` on your PATH ({}) is another build's, and the matching one is not \
         installed — {way}",
        copy.display()
    )
}

fn mcp_command(
    program: &str,
    on_path: Option<std::path::PathBuf>,
    mecha_home: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    let own = |home: &std::path::Path| -> Option<std::path::PathBuf> {
        if program == mecha_core::graph_mcp::PROGRAM {
            return mecha_core::graph_mcp::installed(home)
                .then(|| mecha_core::graph_mcp::binary(home));
        }
        (mecha_core::mail_mcp::PROGRAMS.contains(&program) && mecha_core::mail_mcp::installed(home))
            .then(|| mecha_core::mail_mcp::binary(home, program))
    };
    match on_path {
        Some(p) if !mecha_core::sidecar::stale_copy(program, &p) => Some(p),
        // A stale copy is never wired, even with nothing better here: the
        // plan has said it is not used (review of #699).
        _ => mecha_home.and_then(own),
    }
}

/// What `enable` knows when it decides about the turbo model.
struct TurboFacts<'a> {
    /// [`mecha_core::comfyui_unit::turbo_wanted`]'s reason.
    wanted: Option<String>,
    /// The image server as the plan found it.
    found: Option<&'a mecha_core::sidecar::SidecarState>,
    /// The install is offered already, for its own sake.
    installing: bool,
    /// The pinned blob is in the hub and verified.
    cached: bool,
    /// The install links it (perhaps to a blob gone from the hub).
    linked_here: bool,
    /// A file is there that mecha did not link: the owner's.
    placed_here: bool,
    /// A server on ComfyUI's port was left alone as someone else's.
    left_alone: bool,
    /// `--turbo`.
    asked: bool,
    /// The guided setup's one yes, already given (ruling F14).
    yes: bool,
}

/// What `enable` does about the turbo model.
#[derive(Debug, PartialEq)]
enum TurboStep {
    Nothing,
    Say(String),
    /// Fetch and link it; `reinstall` adds an installed server to the
    /// installs, which restarts it.
    Fetch {
        why: String,
        download: u64,
        reinstall: bool,
    },
}

/// The turbo model's decision, apart from the machine so a table can hold
/// it (review of #650). `unfit` is the image server's fit check, asked only
/// where the install would otherwise be offered without it.
fn turbo_step(f: TurboFacts, unfit: impl FnOnce() -> Option<String>) -> TurboStep {
    use mecha_core::sidecar::SidecarState;
    let Some(why) = f.wanted else {
        return TurboStep::Nothing;
    };
    let file = mecha_core::sampling::turbo_model();
    // Someone else's server is told the file, never installed over.
    if let Some(SidecarState::Provided { by }) = f.found {
        return TurboStep::Say(format!(
            "The turbo model is not fetched ({why}): the image server here is {by}, not \
             mecha's — it needs {file} in its models/diffusion_models/."
        ));
    }
    if f.left_alone {
        return TurboStep::Say(format!(
            "The turbo model is not fetched ({why}): the image server on :{} is not mecha's \
             — it needs {file} in its models/diffusion_models/.",
            mecha_core::comfyui_unit::Naming::shipped().port
        ));
    }
    // A file put there by hand is the owner's, never replaced or fetched
    // again (review of #650).
    if f.placed_here {
        return if f.asked {
            TurboStep::Say(
                "The turbo model is in mecha's image server already, put there by hand; it is \
                 left as it is."
                    .into(),
            )
        } else {
            TurboStep::Nothing
        };
    }
    // Linked to a blob still in the hub: a link whose blob is gone is
    // re-offered, or the refusal naming this command would name a command
    // that does nothing.
    let linked = f.cached && f.linked_here;
    let installed = matches!(f.found, Some(SidecarState::Installed));
    if installed && linked && !f.installing {
        return if f.asked {
            TurboStep::Say("The turbo model is installed already.".into())
        } else {
            TurboStep::Nothing
        };
    }
    // A server offered for turbo alone has not been through the fit check,
    // and the install would fail on it — writing no switch, which a machine
    // that does not fit is never refused (#645). One the check dropped is
    // told the reason it was, not another.
    if f.found.is_some() && !f.installing && !linked {
        if let Some(unfit) = unfit() {
            return TurboStep::Say(format!("The turbo model is not fetched ({why}): {unfit}."));
        }
    }
    if f.installing || installed {
        let download = if f.cached {
            0
        } else {
            mecha_core::comfyui_unit::turbo_file().bytes
        };
        // The guided setup listed its downloads and installs before its one
        // yes (ruling F14): nothing it did not list is fetched, and no
        // server it did not list is reinstalled — a restart — to link one.
        if f.yes && !f.asked && (download > 0 || !f.installing) {
            return if download > 0 {
                TurboStep::Say(format!(
                    "The turbo model is not fetched by this setup ({why}); `mecha features \
                     enable image` fetches it ({}).",
                    bytes_text(download)
                ))
            } else {
                TurboStep::Nothing
            };
        }
        return TurboStep::Fetch {
            why,
            download,
            reinstall: !f.installing,
        };
    }
    if !f.asked {
        return TurboStep::Nothing;
    }
    TurboStep::Say(match f.found {
        Some(SidecarState::Unknown { .. }) => format!(
            "The turbo model is not fetched ({why}): the image server could not be checked \
             (said above)."
        ),
        _ => format!(
            "The turbo model is not fetched ({why}): mecha is not installing an image server \
             here."
        ),
    })
}

/// `documents` reads unready with no `[documents]` table, whatever the
/// switch says, so `enable` writes one where there is none (§13): the
/// settings that match what is here — OCR where its server is installed or
/// provided, layout where its environment is — and the parser's confinement
/// as this machine runs it, asked of each backend's own preflight — Seatbelt
/// on a Mac (left as its default, unnamed), bubblewrap then Landlock on
/// Linux. A check that could not run writes nothing (unknown is never a
/// no). A table the owner wrote is never edited.
async fn documents_table() -> Result<()> {
    use mecha_core::sidecar::SidecarState;
    // The file's own answer, as `image_table`'s: `write_documents_table`
    // leaves a table the file has, and the layered config (an override, the
    // environment) is not what this writes into (review of #683).
    let path = Config::global_path().context("no mecha home to find config.toml in")?;
    // Asked of the file before the machine is: a table already there means
    // no plan and no confinement probe.
    let has_table = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| toml::from_str::<toml::Table>(&t).ok())
        .is_some_and(|d| d.contains_key("documents"));
    if has_table {
        return Ok(());
    }
    let m = sidecar::Machinery::real()?;
    let machine = recommend::Machine::read()?;
    let hub = mecha_core::fetch::hub_dir()?;
    let p = sidecar::plan(Feature::Documents, &m, &machine, &hub, false)?;
    let state = |id: &str| p.sidecars.iter().find(|s| s.id == id).map(|s| &s.state);
    // A check that could not run is not a no: written down, `ocr = false`
    // would outlive the reason it was read (review of #683). So nothing is
    // written while one is unknown, and the owner is told what to look at.
    for id in [mecha_core::llama_units::Which::Ocr.sidecar(), "layout"] {
        if let Some(SidecarState::Unknown { why }) = state(id) {
            println!(
                "No [documents] table written: {id} could not be checked ({why}). `mecha \
                 features plan documents` shows it; `mecha features enable documents` again \
                 writes the table once it can be read."
            );
            return Ok(());
        }
    }
    let here = |id: &str| {
        matches!(
            state(id),
            Some(SidecarState::Installed | SidecarState::Provided { .. })
        )
    };
    // An OCR server run by hand on its port is one too: the sidecar's
    // evidence is unit files and agents, and a server started from a
    // terminal has neither (review of #683).
    let ocr_port = mecha_core::llama_units::Which::Ocr.shipped().public;
    let ocr = here(mecha_core::llama_units::Which::Ocr.sidecar())
        || mecha_core::router_unit::port_answers(ocr_port);
    let layout_here = here("layout");
    // Written only once every piece is here. A piece still missing at this
    // point was not installed — `--no-install`, the path `enable` itself
    // names off a terminal — and the table, written once and never edited,
    // would keep `ocr = false` after a later `enable documents` installed the
    // server (review of #683). So nothing is written yet, and the owner is
    // told what is missing.
    let missing: Vec<&str> = [
        ("the OCR server", ocr),
        ("the layout environment", layout_here),
    ]
    .into_iter()
    .filter(|(_, here)| !here)
    .map(|(what, _)| what)
    .collect();
    if !missing.is_empty() {
        println!(
            "No [documents] table written yet: {} {} not installed. `mecha features enable \
             documents` installs {} and writes the table; a [documents] table you write \
             yourself is used as it is.",
            missing.join(" and "),
            if missing.len() == 1 { "is" } else { "are" },
            if missing.len() == 1 { "it" } else { "them" }
        );
        return Ok(());
    }
    // The confinement this machine runs, asked of each of its own backends:
    // Seatbelt on a Mac (written as no `confine`, its default there),
    // bubblewrap then Landlock on Linux (written as the one that worked).
    let macos = cfg!(target_os = "macos");
    // A probe that found no working confinement writes nothing: written,
    // the table would pin the default that just failed, and `enable` never
    // revisits a table that is there (review of #683). The owner is told the
    // fix, and `enable documents` again writes the table once it works.
    let confine = match mecha_core::document::working_confinement().await {
        Ok(b) => {
            if macos {
                None
            } else {
                Some(b)
            }
        }
        Err(why) => {
            println!(
                "No [documents] table written yet: the PDF parser cannot be confined here \
                 ({why}). {} Then `mecha features enable documents` again writes the table.",
                if macos {
                    "`brew install poppler`."
                } else {
                    "Install poppler-utils, and bubblewrap or a kernel with Landlock."
                }
            );
            return Ok(());
        }
    };
    // Both are here by now (the gate above); the table says so.
    let choice = mecha_core::document::TableChoice {
        ocr,
        layout: true,
        confine,
    };
    if mecha_core::document::write_documents_table(&path, &choice)? {
        println!(
            "Wrote a [documents] table into {}: ocr = {}, layout = {}{}",
            path.display(),
            choice.ocr,
            choice.layout,
            choice
                .confine
                .map(|c| format!(", confine = \"{}\"", c.as_str()))
                .unwrap_or_default()
        );
    }
    Ok(())
}

/// `image` reads off with no `[image]` table, whatever the switch says, and
/// only an install writes one — so a server used as it is (`--no-install`,
/// or one already on `:8188` that was left alone) would leave images off.
/// Written here on its defaults where there is none (review of #645).
fn image_table() -> Result<()> {
    let path = Config::global_path().context("no mecha home to find config.toml in")?;
    if mecha_core::comfyui_unit::write_image_table(&path, None)? {
        println!(
            "Wrote an [image] table into {} on its defaults — ComfyUI at 127.0.0.1:8188. An \
             incognito chat also needs its server_temp_dir.",
            path.display()
        );
    }
    Ok(())
}

fn set(ids: &[String], on: bool) -> Result<()> {
    let cfg = Config::load_global()?;
    let path = Config::global_path().context("no mecha home to find config.toml in")?;
    let features: Vec<Feature> = if on {
        feature::plan_enable(&cfg, ids).map_err(anyhow::Error::msg)?
    } else {
        let mut out = Vec::new();
        for id in ids {
            let f = Feature::parse(id).with_context(|| {
                format!("`{id}` is not a feature — `mecha features` lists them")
            })?;
            // The owner who wants incognito off is the one who types this:
            // told there is no longer a separate way (review of #695).
            anyhow::ensure!(
                f != Feature::Incognito,
                "`incognito` is part of `web` and has no switch of its own since F15 — it is \
                 offered wherever the web app is on and the chat model runs on this machine; \
                 `mecha features disable web` turns it off with the web app"
            );
            anyhow::ensure!(f != Feature::World, feature::WORLD_IS_PER_PERSONA);
            anyhow::ensure!(
                f.has_switch(),
                "`{id}` is part of `{}` and has no switch of its own",
                f.part_of().map(|p| p.id()).unwrap_or_default()
            );
            out.push(f);
        }
        out
    };
    let changes: Vec<(Feature, bool)> = features.iter().map(|f| (*f, on)).collect();
    feature::write_switches(&path, &changes)?;
    let names: Vec<String> = features.iter().map(|f| format!("`{}`", f.id())).collect();
    println!(
        "{} {} in {}",
        if on { "enabled" } else { "disabled" },
        names.join(", "),
        path.display()
    );
    if !on {
        for dependent in left_blocked(&cfg, &features) {
            println!("  `{}` needs it, and now reads blocked", dependent.id());
        }
    }
    Ok(())
}

/// Said once per command, after every switch it wrote: the guided setup
/// enables one feature at a time, and printed per feature it read fourteen
/// times (the owner's fresh Mac, 2026-10-14).
pub(super) const RESTART_NOTE: &str =
    "Services that are already running pick this up when they restart.";

/// What disabling `features` leaves blocked, said rather than discovered: each
/// feature or part another switch owns that is on now and would not be after.
/// A part of a disabled feature is that feature, and not listed. Asked through
/// `switched_on`, so a part's need counts as a switch's does — `disable web`
/// names `dictate`, `calls` and `cloning`, voice's parts that live in the web
/// app (review of #695: since incognito became a part, a filter on switches
/// with a `requires` edge matched nothing).
fn left_blocked(cfg: &Config, features: &[Feature]) -> Vec<Feature> {
    let mut after = cfg.clone();
    for f in features {
        feature::set_switch(&mut after, *f, false);
    }
    Feature::ALL
        .iter()
        .copied()
        .filter(|g| !features.contains(&g.switch_owner()))
        .filter(|g| feature::switched_on(cfg, *g) && !feature::switched_on(&after, *g))
        .collect()
}

/// What the global configuration says about `f` right now, if it refuses:
/// the same [`feature::refusal`] the web routes answer with. `None` when
/// `f` is on, not guarded yet, or the configuration cannot be read — only
/// for wording a message ([`graph_tool_absent`]); a guard uses [`require`],
/// which refuses on an unreadable file.
pub fn refusal_now(f: Feature) -> Option<feature::Refusal> {
    let home = mecha_core::work::mecha_home().ok()?;
    let cfg = Config::load_global().ok()?;
    feature::refusal(&feature::Facts::read(&home, &cfg), f)
}

/// The first thing a verb that belongs to `f` does (FEATURES-DESIGN.md §4.2
/// item 5): refuse with the one sentence every surface says — *"Mail and
/// calendar is off (not enabled in [features]) — `mecha features enable
/// mail`"* — read from the global file, never the layered `Config` the verb
/// then runs with, which a project could otherwise answer for the owner.
///
/// **An unreadable configuration refuses** — the guard cannot run, so the
/// verb does not either. `refusal_now`'s "the verb reports it a moment
/// later" did not hold for every caller: `imagelib` never loads a config,
/// so a malformed file let `imagelib remove` through with `image = false`
/// written in it (review of #451).
pub fn require(f: Feature) -> Result<()> {
    let home = mecha_core::work::mecha_home()?;
    let cfg = Config::load_global().with_context(|| {
        format!(
            "cannot tell whether `{}` is switched on — the global configuration did not load",
            f.switch_owner().id()
        )
    })?;
    match feature::refusal(&feature::Facts::read(&home, &cfg), f) {
        Some(r) => anyhow::bail!("{}", r.sentence()),
        None => Ok(()),
    }
}

/// Why a knowledge-graph tool is missing from a run's surface: the switch,
/// when that is the cause — `prepare_tools` does not connect the graph
/// server with `graph` off, and asking about `[[mcp]]` would send the owner
/// to the wrong file — else the missing server entry (the #445 leftover).
pub fn graph_tool_absent(f: Feature, tool: &str) -> String {
    match refusal_now(f) {
        Some(r) => r.sentence(),
        None => format!(
            "no knowledge-graph server in this configuration — `{tool}` is not on the \
             tool surface. Is `[[mcp]]` enabled?"
        ),
    }
}

/// The upgrade notice, printed on stderr when a session or a service
/// starts: one line per feature this install set up whose switch is still
/// unanswered, and one per `[features]` key this build does not know.
///
/// Best-effort by design — a config that does not load is the command's own
/// error to report, a moment later, and a notice must never be the reason a
/// start fails.
pub fn print_notices() {
    let (Ok(home), Ok(cfg)) = (mecha_core::work::mecha_home(), Config::load_global()) else {
        return;
    };
    let facts = feature::Facts::read(&home, &cfg);
    if let Some(line) = notice_line(&feature::announcements(&facts)) {
        eprintln!("mecha: {line}");
    }
    for key in feature::unknown_switches(&cfg) {
        eprintln!("mecha: `[features] {key}` is not a feature this build knows — a newer build's, or a typo; ignored");
    }
}

/// The whole upgrade notice as one line, with one command that answers all of
/// it: nine lines on every start is a notice the owner learns to skip.
fn notice_line(announced: &[feature::Announcement]) -> Option<String> {
    if announced.is_empty() {
        return None;
    }
    let name = |a: &feature::Announcement| match &a.caveat {
        Some(why) => format!("{} ({why})", a.id.id()),
        None => a.id.id().to_string(),
    };
    // Split by what an unanswered switch does *today*: `Feature::gated` says
    // which are already off, and the rest still work until their surfaces
    // are guarded — calling those off would be a readout saying off while
    // the thing is on (found on review of #445).
    let off: Vec<String> = announced
        .iter()
        .filter(|a| a.id.gated())
        .map(name)
        .collect();
    let working: Vec<String> = announced
        .iter()
        .filter(|a| !a.id.gated())
        .map(name)
        .collect();
    let mut ids: Vec<&str> = Vec::new();
    for a in announced {
        for id in a.fix.split_whitespace().skip(3) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    let mut parts = Vec::new();
    if !off.is_empty() {
        parts.push(format!("off until switched on: {}", off.join(", ")));
    }
    if !working.is_empty() {
        parts.push(format!(
            "still working, but will be off once their surfaces follow the switch: {}",
            working.join(", ")
        ));
    }
    Some(format!(
        "set up here but not switched on in [features] — {} — `mecha features enable {}`, or `mecha setup` to answer each",
        parts.join("; "),
        ids.join(" ")
    ))
}

fn plan(id: &str, json: bool, verify: bool) -> Result<()> {
    let f = Feature::parse(id)
        .with_context(|| format!("`{id}` is not a feature — `mecha features` lists them"))?;
    let mut p = sidecar::plan(
        f,
        &sidecar::Machinery::real()?,
        &recommend::Machine::read()?,
        &mecha_core::fetch::hub_dir()?,
        verify,
    )?;
    let cfg = Config::load_global()?;
    let chat_here = mecha_core::install::chat_runs_here(&cfg);
    // Kept, so the note below can say whether the engine is built here —
    // and still read only if `price` reads it (review of #643).
    let mut nvidia = None;
    mecha_core::install::price(&mut p, chat_here, || {
        *nvidia.insert(mecha_core::engine::read_nvidia())
    });
    if json {
        println!("{}", serde_json::to_string_pretty(&p)?);
    } else {
        print!("{}", render_plan(&p, chat_here));
        // The plan prices a build here by its source; the compile is said.
        let builds = nvidia.is_some_and(|n| {
            p.sidecars
                .iter()
                .any(|s| s.id == "llama" && s.bytes.is_some())
                && mecha_core::engine::choose(std::env::consts::OS, std::env::consts::ARCH, n)
                    .is_ok_and(|t| t.is_built())
        });
        if builds {
            println!(
                "No llama.cpp release fits this machine, so its download is the source, built \
                 here: minutes for the CPU, longer for CUDA."
            );
        }
    }
    Ok(())
}

fn bytes_text(b: u64) -> String {
    if b >= 1 << 30 {
        format!("{:.1} GiB", b as f64 / (1u64 << 30) as f64)
    } else {
        format!("{:.0} MiB", b as f64 / (1u64 << 20) as f64)
    }
}

/// `mecha features plan`: each sidecar and each pinned file, what this
/// machine already has, and what an install would fetch.
fn render_plan(p: &sidecar::Plan, chat_here: bool) -> String {
    use sidecar::{FileState, SidecarState};
    let mut out = format!("What `{}` runs beside mecha:\n", p.feature.id());
    for s in &p.sidecars {
        let not_needed = mecha_core::install::not_needed(s.id, p.feature, chat_here);
        let state = match &s.state {
            SidecarState::Provided { by } => format!("provided — {by}; left alone"),
            // Services an older build wrote: the router's are `mecha setup
            // chat`'s to rewrite (F13), every other's `enable`'s.
            // Only where chat runs here, as `mecha setup` asks it: a machine
            // chatting through a hosted provider needs no router refreshed
            // (review of #691, pass 4).
            SidecarState::Installed
                if s.stale && s.id == mecha_core::router_unit::ID && chat_here =>
            {
                "installed by mecha; its service is out of date — `mecha setup chat` refreshes \
                 it (choosing the model it serves now downloads nothing)"
                    .to_string()
            }
            SidecarState::Installed if s.stale && not_needed.is_none() => format!(
                "installed by mecha; its service is out of date — `mecha features enable {}` \
                 refreshes it (no download)",
                p.feature.id()
            ),
            SidecarState::Installed => "installed by mecha".to_string(),
            // An unfinished install the machine no longer needs is not
            // resumed by `enable`, so the plan does not say it is.
            SidecarState::Incomplete => match not_needed {
                Some(why) => format!("an unfinished install mecha no longer offers — {why}"),
                None => format!(
                    "an install mecha began and did not finish — `mecha features enable {}` resumes it",
                    p.feature.id()
                ),
            },
            SidecarState::Missing { .. } if not_needed.is_some() => {
                not_needed.unwrap_or_default().to_string()
            }
            SidecarState::Missing { .. } if s.not_offered.is_some() => format!(
                "not offered here — {}",
                s.not_offered.as_deref().unwrap_or_default()
            ),
            // A build is said as one: its wait is a compile, not a download.
            SidecarState::Missing { .. } if s.id == mecha_core::graph_mcp::ID => format!(
                "not here — `mecha features enable {}` builds it from its pinned commit (a few \
                 minutes; needs cargo and a C compiler)",
                p.feature.id()
            ),
            SidecarState::Missing { .. } if s.id == mecha_core::mail_mcp::ID => format!(
                "not here — `mecha features enable {}` builds it from this mecha's own source \
                 (a few minutes; needs cargo)",
                p.feature.id()
            ),
            SidecarState::Missing { .. } if mecha_core::install::installable(s.id) => format!(
                "not here — `mecha features enable {}` installs it",
                p.feature.id()
            ),
            SidecarState::Missing { step } => {
                format!("not here — its installer arrives in step {step}")
            }
            SidecarState::Unknown { why } => format!("unknown — {why}; nothing is offered over it"),
        };
        let state = match s.bytes {
            Some(b) => format!("{state} ({} to download)", bytes_text(b)),
            None => state,
        };
        // A copy from another build, passed over: said, so the owner knows
        // why one on their PATH is not used (§15).
        let state = match &s.passed_over {
            Some(copy) => format!("{state}; {copy}, another build's, so it is not used"),
            None => state,
        };
        out.push_str(&format!("  {:<32} {state}\n", s.label));
    }
    out.push_str("\nThe models it loads:\n");
    for f in &p.files {
        let what = match (f.repo, f.path) {
            (Some(repo), path) => format!("{repo}/{path}"),
            (None, "") if f.state == FileState::Brought => "your own model".to_string(),
            (None, "") if f.model.is_empty() => "no recommended model".to_string(),
            (None, "") => f.model.to_string(),
            (None, path) => path.to_string(),
        };
        let state = match &f.state {
            FileState::Cached => "in the cache, matching its pin".to_string(),
            // The chat model's download is `mecha setup chat`'s, never
            // `enable`'s (F13), and is not in the total below.
            FileState::Download { bytes } if f.slot == "chat" && chat_here => format!(
                "not here — `mecha setup chat` chooses and fetches the chat model ({})",
                bytes_text(*bytes)
            ),
            // Chat served from elsewhere: the plan says so of the router, and
            // the same of its file (found on review of #627).
            FileState::Download { .. } if f.slot == "chat" => {
                "not needed here — the chat model is served from elsewhere".to_string()
            }
            FileState::Download { bytes } => format!("to download, {}", bytes_text(*bytes)),
            FileState::Mismatch => {
                "a different file is at its path — not replaced, and not used".to_string()
            }
            FileState::WithSidecar => "comes with its sidecar's install".to_string(),
            FileState::NotOffered { why } => format!("not fetched here — {why}"),
            FileState::HeldBy { sidecar } => format!("kept by the provided {sidecar}; not read"),
            FileState::Unverified => {
                "a file of the pinned size, placed by hand — not hashed by the plan".to_string()
            }
            FileState::NoRow => "no model is recommended for this machine's tier".to_string(),
            FileState::KeeperUnknown { sidecar } => {
                format!("kept by {sidecar}, which could not be checked — not priced")
            }
            FileState::Brought => {
                "your own GGUF, served by mecha's router (`mecha setup chat` changes it)"
                    .to_string()
            }
        };
        out.push_str(&format!(
            "  {:<16} {what}\n  {:<16}   {state}\n",
            f.slot, ""
        ));
    }
    out.push('\n');
    if p.nothing_to_do {
        out.push_str("Nothing to install: this machine has all of it.\n");
        return out;
    }
    let unknown = p
        .sidecars
        .iter()
        .filter(|s| matches!(s.state, SidecarState::Unknown { .. }))
        .count();
    if unknown > 0 {
        out.push_str(&format!(
            "{unknown} could not be checked — nothing is offered over them until they can be.\n"
        ));
    }
    let unverified = p
        .files
        .iter()
        .filter(|f| matches!(f.state, FileState::Unverified))
        .count();
    if unverified > 0 {
        out.push_str(&format!(
            "{unverified} model file(s) placed by hand are not hashed — `mecha features plan {} --verify` reads them.\n",
            p.feature.id()
        ));
    }
    let mismatched = p
        .files
        .iter()
        .filter(|f| matches!(f.state, FileState::Mismatch))
        .count();
    if mismatched > 0 {
        out.push_str(&format!(
            "{mismatched} model file(s) at their path do not match their pins — move them aside for the pinned ones.\n"
        ));
    }
    if p.files.iter().any(|f| matches!(f.state, FileState::NoRow)) {
        out.push_str("No model is recommended at this machine's tier — see the hardware page.\n");
    }
    if p.download_bytes > 0 {
        out.push_str(&format!("To download: {}.\n", bytes_text(p.download_bytes)));
    }
    if p.sidecars.iter().any(|s| {
        matches!(s.state, SidecarState::Missing { .. })
            && !mecha_core::install::installable(s.id)
            && mecha_core::install::not_needed(s.id, p.feature, chat_here).is_none()
    }) {
        out.push_str("Some installers are not built yet — each such line names its step.\n");
    }
    out
}

fn gib(mb: u64) -> String {
    format!("{:.1} GiB", mb as f64 / 1024.0)
}

fn peak_text(p: &Peak) -> String {
    match p {
        Peak::Measured { mb, machine, date } => {
            format!("{}  measured on {machine}, {date}", gib(*mb as u64))
        }
        Peak::Arithmetic { mb } => format!("{}  arithmetic", gib(*mb as u64)),
        Peak::Unmeasured => "unmeasured".to_string(),
    }
}

fn sum_text(name: &str, s: &Sum) -> String {
    let part = |v: Option<u64>, floor: &Floor| match v {
        Some(mb) => format!(
            "{} of {} ({:.0}%)",
            gib(mb),
            gib(s.total_mb),
            mb as f64 * 100.0 / s.total_mb.max(1) as f64
        ),
        None => format!(
            "unknown of {} — unmeasured: {}; the rest add up to {}",
            gib(s.total_mb),
            floor.unknown.join(", "),
            gib(floor.known_mb)
        ),
    };
    let band = s
        .band
        .map(|b| format!(" — {}", b.word()))
        .unwrap_or_default();
    format!(
        "{name}\n  resident:          {}\n  everything loaded: {}{band}\n",
        part(s.resident_mb, &s.resident_floor),
        part(s.loaded_mb, &s.loaded_floor)
    )
}

/// `mecha features --probe`: every model the shown features need, what each
/// holds and on what evidence, and the sums against this machine.
fn render_budget(b: &Budget) -> String {
    let mut out = String::new();
    let tier = match b.tier_gb {
        Some(t) => format!("the {t} GB tier"),
        None => "below the smallest tier (16 GB)".to_string(),
    };
    match b.machine {
        Machine::Unified {
            total_mb,
            gpu_unread,
        } => {
            out.push_str(&format!("One memory pool, {} — {tier}.\n", gib(total_mb)));
            if gpu_unread {
                out.push_str(
                    "  (Only `nvidia-smi` is asked about a card, and it did not answer; a card \
                     it cannot see is counted as part of this pool.)\n",
                );
            }
        }
        Machine::Discrete {
            gpu_mb,
            cards,
            host_mb,
        } => out.push_str(&match cards {
            1 => format!(
                "A separate GPU: {} on the card, {} of system memory — {tier}.\n",
                gib(gpu_mb),
                gib(host_mb)
            ),
            n => format!(
                "{n} GPUs, the largest {} — read by that one card, since a model that must sit \
                 on one device cannot split — and {} of system memory — {tier}.\n",
                gib(gpu_mb),
                gib(host_mb)
            ),
        }),
    }
    out.push('\n');
    for l in &b.lines {
        let model = l.model.unwrap_or("no model recommended at this tier");
        out.push_str(&format!(
            "  {:<16} {model} — {}\n",
            l.label,
            l.residency.word().to_lowercase()
        ));
        match &l.host {
            None => out.push_str(&format!("  {:<16}   {}\n", "", peak_text(&l.gpu))),
            Some(h) => {
                out.push_str(&format!("  {:<16}   card: {}\n", "", peak_text(&l.gpu)));
                out.push_str(&format!("  {:<16}   host: {}\n", "", peak_text(h)));
            }
        }
        if !l.own_row && l.model.is_some() {
            out.push_str(&format!(
                "  {:<16}   (no row for this machine: the nearest row's figure, carried as arithmetic)\n",
                ""
            ));
        }
    }
    out.push('\n');
    match &b.host {
        None => out.push_str(&sum_text("The pool", &b.gpu)),
        Some(h) => {
            out.push_str(&sum_text("The card", &b.gpu));
            out.push_str(&sum_text("System memory", h));
        }
    }
    if !b.not_counted.is_empty() {
        out.push_str("\nNot in these sums:\n");
        for n in &b.not_counted {
            out.push_str(&format!("  {n}\n"));
        }
    }
    out
}

/// Depth under the top level: a part of a part (`layout` under `ocr`)
/// indents twice.
fn depth(f: Feature) -> usize {
    std::iter::successors(f.part_of(), |p| p.part_of()).count()
}

fn render(rows: &[Row], announced: &[feature::Announcement]) -> String {
    let width = rows
        .iter()
        .map(|r| 2 * depth(r.id) + r.id.id().len())
        .max()
        .unwrap_or(0);
    let mut out = String::new();
    for row in rows {
        let name = format!("{}{}", "  ".repeat(depth(row.id)), row.id.id());
        let announcement = announced.iter().find(|a| a.id == row.id);
        let (say, fix) = match &row.state {
            // Set up here and unanswered: its own row, not a bare `off`.
            State::Off { .. } if announcement.is_some() => {
                let a = announcement.expect("checked");
                (
                    match &a.caveat {
                        Some(why) => format!("not enabled, but set up here ({why})"),
                        None => "not enabled, but set up here".to_string(),
                    },
                    Some(a.fix.clone()),
                )
            }
            State::On { detail } => (detail.clone(), None),
            State::Off { reason, fix } | State::Unready { reason, fix } => {
                (reason.clone(), fix.clone())
            }
            // The command of what blocks it, the same `next` the web app shows.
            State::Blocked { on } => (format!("needs {}", on.id()), row.next.clone()),
            State::Unknown { reason } => (reason.clone(), None),
        };
        out.push_str(&format!("{name:<width$}  {:<7}  {say}\n", row.state.word()));
        if let Some(fix) = fix {
            out.push_str(&format!("{:<width$}  {:<7}  → {fix}\n", "", ""));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The turbo decision's table: each state says one thing or fetches,
    /// someone else's server is never installed over, the fit check is
    /// asked only where nothing else checked the machine, and the guided
    /// setup fetches and restarts nothing it did not list (review of #650).
    #[test]
    fn the_turbo_decision_holds_its_table() {
        use mecha_core::sidecar::SidecarState as S;
        let bytes = mecha_core::comfyui_unit::turbo_file().bytes;
        let installed = S::Installed;
        let missing = S::Missing { step: "7e" };
        let provided = S::Provided {
            by: "unit comfyui.service".into(),
        };
        let unknown = S::Unknown { why: "?".into() };
        let base = |found| TurboFacts {
            wanted: Some("asked for with --turbo".into()),
            found,
            installing: false,
            cached: false,
            linked_here: false,
            placed_here: false,
            left_alone: false,
            asked: true,
            yes: false,
        };
        let fits = || None;
        let never = || -> Option<String> { panic!("the fit check was asked") };
        let said = |step: TurboStep, has: &str| match step {
            TurboStep::Say(line) => assert!(line.contains(has), "{line}"),
            other => panic!("{other:?}, not a line with {has:?}"),
        };

        // Not wanted: nothing, whatever the state.
        let f = TurboFacts {
            wanted: None,
            ..base(Some(&installed))
        };
        assert_eq!(turbo_step(f, never), TurboStep::Nothing);
        // Someone else's server, by evidence or by its port: told, never fetched.
        said(turbo_step(base(Some(&provided)), never), "comfyui.service");
        let f = TurboFacts {
            left_alone: true,
            ..base(Some(&missing))
        };
        said(turbo_step(f, never), "not mecha's");
        // A file the owner put there: said when asked, never fetched over.
        let f = TurboFacts {
            placed_here: true,
            ..base(Some(&installed))
        };
        said(turbo_step(f, never), "by hand");
        let f = TurboFacts {
            placed_here: true,
            asked: false,
            ..base(Some(&installed))
        };
        assert_eq!(turbo_step(f, never), TurboStep::Nothing);
        // Installed and linked to a verified blob: said only when asked.
        let linked = TurboFacts {
            cached: true,
            linked_here: true,
            ..base(Some(&installed))
        };
        said(turbo_step(linked, never), "installed already");
        let f = TurboFacts {
            cached: true,
            linked_here: true,
            asked: false,
            wanted: Some("this install linked it before".into()),
            ..base(Some(&installed))
        };
        assert_eq!(turbo_step(f, never), TurboStep::Nothing);
        // Linked to a blob gone from the hub: re-offered at its full size.
        let f = TurboFacts {
            linked_here: true,
            ..base(Some(&installed))
        };
        assert_eq!(
            turbo_step(f, fits),
            TurboStep::Fetch {
                why: "asked for with --turbo".into(),
                download: bytes,
                reinstall: true
            }
        );
        // Installed for turbo alone on a machine that no longer fits: the
        // fit check's reason, and no install to fail.
        said(
            turbo_step(base(Some(&installed)), || Some("the driver is 570".into())),
            "the driver is 570",
        );
        // Dropped by the fit check while missing: the same reason.
        said(
            turbo_step(base(Some(&missing)), || Some("the driver is 570".into())),
            "the driver is 570",
        );
        // Installing anyway: no second fit check, the blob's size rides along.
        let f = TurboFacts {
            installing: true,
            cached: true,
            ..base(Some(&missing))
        };
        assert_eq!(
            turbo_step(f, never),
            TurboStep::Fetch {
                why: "asked for with --turbo".into(),
                download: 0,
                reinstall: false
            }
        );
        // The guided setup (F14): a download it did not list is said, not
        // fetched; an installed server is not restarted to link a cached one;
        // a cached one rides along with an install it did list.
        let guided = |found, installing, cached| TurboFacts {
            asked: false,
            yes: true,
            installing,
            cached,
            wanted: Some("ada's picture settings choose it".into()),
            ..base(found)
        };
        said(
            turbo_step(guided(Some(&installed), false, false), fits),
            "not fetched by this setup",
        );
        assert_eq!(
            turbo_step(guided(Some(&installed), false, true), fits),
            TurboStep::Nothing
        );
        said(
            turbo_step(guided(Some(&missing), true, false), never),
            "not fetched by this setup",
        );
        assert!(matches!(
            turbo_step(guided(Some(&missing), true, true), never),
            TurboStep::Fetch {
                download: 0,
                reinstall: false,
                ..
            }
        ));
        // Nothing to install into: said only when asked.
        said(
            turbo_step(base(Some(&unknown)), fits),
            "could not be checked",
        );
        said(turbo_step(base(Some(&missing)), fits), "not installing");
        let f = TurboFacts {
            asked: false,
            ..base(Some(&missing))
        };
        assert_eq!(turbo_step(f, fits), TurboStep::Nothing);
    }

    /// A disable names what it leaves blocked through a part's needs as well
    /// as a switch's — the web app's voice parts — and never the disabled
    /// feature's own parts (review of #695).
    #[test]
    fn a_disable_names_what_it_leaves_blocked() {
        let mut cfg = Config::default();
        for f in [Feature::Web, Feature::Voice] {
            feature::set_switch(&mut cfg, f, true);
        }
        assert_eq!(
            left_blocked(&cfg, &[Feature::Web]),
            [Feature::Dictate, Feature::Calls, Feature::Cloning]
        );
        assert!(left_blocked(&cfg, &[Feature::Voice]).is_empty());
        feature::set_switch(&mut cfg, Feature::Voice, false);
        assert!(left_blocked(&cfg, &[Feature::Web]).is_empty());
    }

    /// A stale copy is refused with the way to the matching build — or, where
    /// that cannot run, why (review of #699).
    #[test]
    fn a_stale_copy_is_refused_with_what_can_fix_it() {
        let copy = std::path::Path::new("/u/.cargo/bin/mecha-mail");
        let s = stale_refusal("mecha-mail", copy, None);
        assert!(
            s.contains("another build's") && s.contains("`mecha features enable mail` builds it"),
            "{s}"
        );
        let s = stale_refusal("mecha-docs", copy, None);
        assert!(s.contains("enable docs"), "{s}");
        let s = stale_refusal(
            "mecha-mail",
            copy,
            Some("build from your checkout: `cargo install --path mecha-mail --locked`".into()),
        );
        assert!(
            s.contains("cargo install --path") && !s.contains("enable mail"),
            "{s}"
        );
        // Not one of mecha's own servers: run as named.
        assert_eq!(
            remedy_program("mecha").unwrap(),
            std::ffi::OsString::from("mecha")
        );
    }

    /// `--turbo` is the image model's, and fetches: both refusals come
    /// before the machine is read, and write nothing (review of #650).
    #[test]
    fn turbo_is_refused_without_image_or_with_no_install() {
        let home = crate::testenv::HomeGuard::new("features-turbo");
        let path = home.dir.join("config.toml");
        std::fs::write(&path, "# mine\n").unwrap();
        // A runtime of its own, so the home's lock is never held across an
        // await.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt
            .block_on(enable(&["graph".into()], false, true, false, false))
            .unwrap_err();
        assert!(format!("{err:#}").contains("image model's"), "{err:#}");
        let err = rt
            .block_on(enable(&["image".into()], true, true, false, false))
            .unwrap_err();
        assert!(format!("{err:#}").contains("--no-install"), "{err:#}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# mine\n");
    }

    /// The #445 leftover: a graph tool missing from a run's surface names the
    /// switch when that is why, and the `[[mcp]]` entry only when it is not —
    /// read from the global file, as every refusal is.
    #[test]
    fn a_missing_graph_tool_names_the_switch_when_that_is_the_cause() {
        let home = crate::testenv::HomeGuard::new("features-graph-absent");
        let write = |body: &str| std::fs::write(home.dir.join("config.toml"), body).unwrap();
        write("[features]\ngraph = false\n");
        let said = graph_tool_absent(Feature::Tasks, "kg_task_list");
        assert!(said.contains("`mecha features enable graph`"), "{said}");
        assert!(!said.contains("[[mcp]]"), "{said}");
        write("[features]\ngraph = true\n");
        let said = graph_tool_absent(Feature::Tasks, "kg_task_list");
        assert!(said.contains("Is `[[mcp]]` enabled?"), "{said}");
        assert!(require(Feature::Tasks).is_ok());
    }

    #[test]
    fn the_notice_is_one_line_with_one_command() {
        use mecha_core::feature::Announcement;
        assert_eq!(notice_line(&[]), None);
        let line = notice_line(&[
            Announcement {
                id: Feature::Mail,
                caveat: Some("no account is authorised".into()),
                fix: "mecha features enable mail".into(),
            },
            Announcement {
                id: Feature::Search,
                caveat: None,
                fix: "mecha features enable search".into(),
            },
        ])
        .unwrap();
        // Every feature is guarded since step 3b, so both are called off.
        assert!(
            line.contains("off until switched on: mail (no account is authorised), search"),
            "{line}"
        );
        assert!(!line.contains("still working"), "{line}");
        assert!(
            line.contains("`mecha features enable mail search`"),
            "{line}"
        );
    }

    #[test]
    fn parts_indent_under_their_parent_and_a_fix_gets_its_own_line() {
        // Every switch on, so each row shows its own settings' fix.
        let mut config = mecha_core::config::Config::default();
        for f in Feature::ALL.iter().filter(|f| f.has_switch()) {
            config.features.0.insert(f.id().to_string(), true);
        }
        let rows = feature::all(&feature::Facts {
            config,
            ..feature::Facts::default()
        });
        let text = render(&rows, &[]);
        assert!(text.contains("\n  tasks "), "{text}");
        assert!(text.contains("\n    layout "), "{text}");
        // Switched on without settings is unready, parts included, and never
        // a block on what depends on it (review of #449, pass 3).
        assert!(text.contains("unready"), "{text}");
        assert!(!text.contains("blocked"), "{text}");
        assert!(text.contains("→ add a [documents] table"), "{text}");
        assert!(text.contains("→ add an [image] table"), "{text}");
    }

    /// A card: two sums, the host one unknown because the chat row's host
    /// figure is unmeasured — said in words, never printed as a zero — and a
    /// model with no row of its own says its figure is carried.
    #[test]
    fn the_probe_on_a_card_shows_two_sums_and_says_what_it_does_not_know() {
        let b = recommend::budget(
            Machine::Discrete {
                gpu_mb: 32_768,
                cards: 1,
                host_mb: 65_536,
            },
            &[Feature::Ocr],
        );
        let text = render_budget(&b);
        assert!(
            text.contains(
                "A separate GPU: 32.0 GiB on the card, 64.0 GiB of system memory — the 32 GB tier."
            ),
            "{text}"
        );
        assert!(text.contains("The card\n"), "{text}");
        assert!(text.contains("System memory\n"), "{text}");
        // Each line's floor is its own: the on-demand OCR server's process
        // memory is on the everything-loaded line, never the resident one.
        assert!(
            text.contains("  resident:          unknown of 64.0 GiB — unmeasured: chat; the rest add up to 0.0 GiB\n"),
            "{text}"
        );
        assert!(
            text.contains("  everything loaded: unknown of 64.0 GiB — unmeasured: chat; the rest add up to 0.9 GiB\n"),
            "{text}"
        );
        assert!(text.contains("host: unmeasured"), "{text}");
        assert!(text.contains("the nearest row's figure, carried"), "{text}");
        assert!(!text.contains(" 0.0 GiB of 64"), "{text}");
    }

    /// One pool: one sum, with its band, and the exclusions named.
    #[test]
    fn the_probe_on_one_pool_names_what_it_leaves_out() {
        let b = recommend::budget(
            Machine::Unified {
                total_mb: 124_610,
                gpu_unread: false,
            },
            &[],
        );
        let text = render_budget(&b);
        assert!(
            text.contains("One memory pool, 121.7 GiB — the 128 GB tier."),
            "{text}"
        );
        assert!(text.contains("The pool\n"), "{text}");
        assert!(text.contains("— comfortable"), "{text}");
        assert!(text.contains("Not in these sums:\n  chat: "), "{text}");
        assert!(!text.contains("System memory"), "{text}");
    }

    /// Two cards are named as two, and read by the largest — never shown as
    /// one card holding their sum.
    #[test]
    fn the_probe_names_two_cards_and_reads_the_largest() {
        let b = recommend::budget(
            Machine::Discrete {
                gpu_mb: 24_576,
                cards: 2,
                host_mb: 65_536,
            },
            &[],
        );
        let text = render_budget(&b);
        assert!(text.starts_with("2 GPUs, the largest 24.0 GiB"), "{text}");
        assert!(text.contains("the 16 GB tier"), "{text}");
        assert!(!text.contains("48.0 GiB"), "{text}");
    }

    /// The plan in words: every state a person can meet, each with what to
    /// do — and never "nothing to install" beside a gap.
    /// The plan never names `enable` for the router: the chat model is
    /// `mecha setup chat`'s (ruling F13), so where chat runs here the plan
    /// names that command, and elsewhere says it is not needed.
    #[test]
    fn the_plan_names_setup_chat_for_the_router_never_enable() {
        use mecha_core::sidecar::{Plan, PlannedSidecar, SidecarState};
        let p = Plan {
            feature: Feature::Messages,
            sidecars: vec![PlannedSidecar {
                id: "router",
                label: "the chat router",
                state: SidecarState::Missing { step: "7c" },
                bytes: None,
                not_offered: None,
                passed_over: None,
                stale: false,
            }],
            // The chat model's files are named but not counted: the plan's
            // total is what `enable` fetches (review of #627).
            files: vec![mecha_core::sidecar::PlannedFile {
                slot: "chat",
                model: "the-chat-row",
                repo: Some("org/r"),
                path: "m.gguf",
                state: mecha_core::sidecar::FileState::Download { bytes: 22 << 30 },
            }],
            download_bytes: 0,
            nothing_to_do: false,
        };
        let here = render_plan(&p, true);
        assert!(here.contains("mecha setup chat"), "{here}");
        assert!(!here.contains("features enable"), "{here}");
        assert!(
            here.contains("chooses and fetches the chat model (22.0 GiB)"),
            "{here}"
        );
        assert!(!here.contains("To download"), "{here}");
        let elsewhere = render_plan(&p, false);
        assert!(elsewhere.contains("not needed here"), "{elsewhere}");
        assert!(!elsewhere.contains("features enable"), "{elsewhere}");
        assert!(!elsewhere.contains("setup chat"), "{elsewhere}");
    }

    /// A stale install says what refreshes it — `enable` for a feature's
    /// server, `setup chat` for the router — and is never nothing to do.
    #[test]
    fn the_plan_says_what_refreshes_a_stale_service() {
        use mecha_core::sidecar::{Plan, PlannedSidecar, SidecarState};
        let row = |id: &'static str| PlannedSidecar {
            id,
            label: "a server",
            state: SidecarState::Installed,
            bytes: None,
            not_offered: None,
            stale: true,
            passed_over: None,
        };
        let p = Plan {
            feature: Feature::Image,
            sidecars: vec![row(mecha_core::comfyui_unit::ID)],
            files: vec![],
            download_bytes: 0,
            nothing_to_do: false,
        };
        let text = render_plan(&p, true);
        assert!(
            text.contains("out of date — `mecha features enable image` refreshes it"),
            "{text}"
        );
        let p = Plan {
            feature: Feature::Messages,
            sidecars: vec![row("router")],
            ..p
        };
        let text = render_plan(&p, true);
        assert!(text.contains("`mecha setup chat` refreshes it"), "{text}");
        assert!(!text.contains("features enable"), "{text}");
        // Chatting elsewhere, the router is not this machine's to refresh.
        let text = render_plan(&p, false);
        assert!(!text.contains("setup chat"), "{text}");
        assert!(!text.contains("out of date"), "{text}");
    }

    #[test]
    fn the_plan_says_each_state_and_what_to_do() {
        use mecha_core::sidecar::{FileState, Plan, PlannedFile, PlannedSidecar, SidecarState};
        let p = Plan {
            feature: Feature::Documents,
            sidecars: vec![
                PlannedSidecar {
                    id: "llama",
                    label: "llama.cpp",
                    state: SidecarState::Provided {
                        by: "x on PATH".into(),
                    },
                    bytes: None,
                    not_offered: None,
                    passed_over: None,
                    stale: false,
                },
                PlannedSidecar {
                    // Every real sidecar has an installer since 7f: one
                    // that does not yet is a made-up id.
                    id: "not-yet",
                    label: "a later sidecar",
                    state: SidecarState::Missing { step: "7z" },
                    bytes: None,
                    not_offered: None,
                    passed_over: None,
                    stale: false,
                },
                PlannedSidecar {
                    id: "layout",
                    label: "layout",
                    state: SidecarState::Unknown {
                        why: "denied".into(),
                    },
                    bytes: None,
                    not_offered: None,
                    passed_over: None,
                    stale: false,
                },
            ],
            files: vec![
                PlannedFile {
                    slot: "chat",
                    model: "",
                    repo: None,
                    path: "",
                    state: FileState::NoRow,
                },
                PlannedFile {
                    slot: "ocr",
                    model: "m",
                    repo: Some("org/r"),
                    path: "a.gguf",
                    state: FileState::Mismatch,
                },
                // An unread keeper still names the tier's row; a brought
                // model is the owner's (review of #618).
                PlannedFile {
                    slot: "image",
                    model: "the-row-model",
                    repo: None,
                    path: "",
                    state: FileState::KeeperUnknown { sidecar: "ComfyUI" },
                },
                PlannedFile {
                    slot: "chat",
                    model: "",
                    repo: None,
                    path: "",
                    state: FileState::Brought,
                },
                PlannedFile {
                    slot: "ocr",
                    model: "m",
                    repo: Some("org/r"),
                    path: "b.gguf",
                    state: FileState::Download { bytes: 2 << 30 },
                },
            ],
            download_bytes: 2 << 30,
            nothing_to_do: false,
        };
        let text = render_plan(&p, true);
        for want in [
            "provided — x on PATH; left alone",
            "not here — its installer arrives in step 7z",
            "unknown — denied; nothing is offered over it",
            "no recommended model",
            "no model is recommended for this machine's tier",
            "the-row-model",
            "kept by ComfyUI, which could not be checked — not priced",
            "your own model",
            "1 could not be checked",
            "1 model file(s) at their path do not match their pins",
            "To download: 2.0 GiB.",
            "Some installers are not built yet",
        ] {
            assert!(text.contains(want), "missing {want:?} in:\n{text}");
        }
        assert!(!text.contains("Nothing to install"), "{text}");
    }

    /// The owner's server on the PATH is the entry's; else mecha's own build,
    /// by absolute path; a half-built tree is neither, and no other program
    /// is ever pointed at the graph's build.
    #[test]
    fn the_graph_entry_runs_the_owner_s_server_or_mecha_s_build() {
        use mecha_core::graph_mcp::{binary, root, PROGRAM, REPO, REV};
        let home = std::env::temp_dir().join(format!("mecha-mcpcmd-{}", uuid::Uuid::new_v4()));
        let theirs = std::path::PathBuf::from("/opt/bin/mecha-graph-mcp");
        assert_eq!(
            mcp_command(PROGRAM, Some(theirs.clone()), Some(&home)),
            Some(theirs)
        );
        assert_eq!(mcp_command(PROGRAM, None, Some(&home)), None);
        std::fs::create_dir_all(root(&home).join("bin")).unwrap();
        std::fs::write(binary(&home), b"#!/bin/sh\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(binary(&home), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        assert_eq!(
            mcp_command(PROGRAM, None, Some(&home)),
            None,
            "no record of the pinned build"
        );
        std::fs::write(
            root(&home).join(".crates2.json"),
            format!(r#"{{"installs":{{"{PROGRAM} 0.1.5 (git+{REPO}?rev={REV}#{REV})":{{}}}}}}"#),
        )
        .unwrap();
        let ours = mcp_command(PROGRAM, None, Some(&home)).unwrap();
        assert!(ours.is_absolute() && ours == binary(&home));
        assert_eq!(mcp_command("mecha-mail", None, Some(&home)), None);
        // A copy on the PATH from another build is passed over for mecha's
        // own; with none of mecha's here it is not wired either (review of
        // #699).
        let cargo = home.join("cargo");
        std::fs::create_dir_all(cargo.join("bin")).unwrap();
        std::fs::write(
            cargo.join(".crates2.json"),
            format!(
                r#"{{"installs":{{"{PROGRAM} 0.1.5 (registry+https://github.com/rust-lang/crates.io-index)":{{"bins":["{PROGRAM}"]}}}}}}"#
            ),
        )
        .unwrap();
        let stale = cargo.join("bin").join(PROGRAM);
        assert_eq!(
            mcp_command(PROGRAM, Some(stale.clone()), Some(&home)),
            Some(binary(&home))
        );
        std::fs::remove_file(root(&home).join(".crates2.json")).unwrap();
        assert_eq!(mcp_command(PROGRAM, Some(stale), Some(&home)), None);
        let _ = std::fs::remove_dir_all(&home);
    }
}
