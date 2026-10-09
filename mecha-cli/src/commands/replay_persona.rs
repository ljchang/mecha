//! `mecha replay --persona` — sample a persona chat's next step from one of
//! its turns, under today's build or an arm's overlay.
//!
//! The run is the one serve would send. Its context comes from
//! `persona::turn::context` and its agent from `setup::persona_agent` on the
//! version the chat pinned, which are the functions serve calls. The history
//! and the turn's notes are the transcript's own, read up to the turn's
//! record line. What differs from a served turn is stated in the header
//! every output file opens with:
//!
//! - each sample is seeded (`--seed-base` + its index), where serve sends no
//!   seed, because N samples at one seed would be one sample N times;
//! - the clock is set to a day whose calendar reference reads as the one the
//!   turn was sent, or the header says it could not be matched;
//! - tools are described as today's build describes them and never run: the
//!   first call stops the run, so each sample is one model request;
//! - the turn's one-shot readers are not stamped, since no picture is drawn.
//!
//! The output file holds what the model wrote, so it goes in the private
//! research store (0600). Standard output carries counts only.

use crate::follow::Follower;
use crate::setup::PersonaUse;
use crate::GlobalOpts;
use anyhow::{bail, Context, Result};
use mecha_core::persona::replay::{self as pr, Branch, Capture, Overlay, Sample};
use mecha_core::replay_run::{replay_registry, OnDivergence};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

#[derive(clap::Args, Debug, Default)]
pub struct SampleArgs {
    /// List the chat's owner turns (record line, messages, notes) and stop.
    #[arg(long, requires = "persona")]
    pub list: bool,

    /// The owner turn to branch at, by its record line (`--list`).
    #[arg(
        long,
        value_name = "LINE",
        requires = "persona",
        conflicts_with = "turn"
    )]
    pub at: Option<usize>,

    /// The owner turn to branch at, counting from 1.
    #[arg(long, value_name = "N", requires = "persona")]
    pub turn: Option<usize>,

    /// How many samples of the turn's first request.
    #[arg(long, default_value_t = 10, requires = "persona")]
    pub samples: usize,

    /// The first sample's seed; sample i is sent seed-base + i.
    #[arg(long, default_value_t = 1, requires = "persona")]
    pub seed_base: u64,

    /// An arm's text edits to the built request (TOML; see
    /// `persona::replay::Overlay`).
    #[arg(long, value_name = "FILE", requires = "persona")]
    pub overlay: Option<PathBuf>,

    /// Send each request with this `max_tokens`.
    #[arg(long, value_name = "N", requires = "persona")]
    pub max_tokens: Option<u32>,

    /// Where the samples go (JSONL, 0600). Default: the research store,
    /// `~/.mecha/research/replay/<persona>/`.
    #[arg(long, value_name = "FILE", requires = "persona")]
    pub out: Option<PathBuf>,

    /// Sample even when the router holds a different model, which loads
    /// this one in its place.
    #[arg(long, requires = "persona")]
    pub allow_load: bool,

    /// Branch inside the turn's run, after the batch holding its Nth tool
    /// call (1-based) and the results that answered it.
    #[arg(long, value_name = "N", requires = "persona")]
    pub at_call: Option<usize>,

    /// Send the request only: no picture call runs. By default a sample's
    /// picture call runs the real `image_generate` against the chat's scene
    /// staged as of the turn, in a scratch store of its own.
    #[arg(long, requires = "persona")]
    pub no_render: bool,

    /// Render without the turn's scene reader and role splitter (an arm
    /// that measures the call drawn as sent).
    #[arg(long, requires = "persona")]
    pub no_readers: bool,

    /// Which tool text the request carries: `today` (this build's, the
    /// default: the binary is the arm) or `recorded` (the surface the turn
    /// was sent, from the surface store by its `tools_hash`), so a wording
    /// experiment on an older chat starts from what the model saw. Recorded
    /// is call-only: a tool that runs is today's and says so in its own words.
    #[arg(
        long,
        value_name = "today|recorded",
        default_value = "today",
        requires = "persona"
    )]
    pub surface: String,

    /// Requests a sample may send (call-only). A picture call whose
    /// arguments did not parse is answered by the real tool's own refusal,
    /// as it was on the turn; a parsed one ends the sample. Each sample then
    /// reports the first request whose call parsed within --parsed-limit.
    #[arg(long, default_value_t = 1, requires = "persona")]
    pub attempts: usize,

    /// The most argument bytes a call may carry and count as parsed.
    #[arg(long, default_value_t = 2048, requires = "persona")]
    pub parsed_limit: usize,

    /// Sample i's pictures draw their fresh seeds from a stream seeded
    /// image-seed-base + i, so two arms run with the same base render each
    /// sample at the same seeds.
    #[arg(long, default_value_t = 1, requires = "persona")]
    pub image_seed_base: u64,
}

#[derive(serde::Deserialize)]
struct Pin {
    persona: String,
    digest: String,
}

/// The transcript of a persona chat, and whose it is.
fn resolve(store: &Path, arg: &str) -> Result<(PathBuf, String, String)> {
    let path = PathBuf::from(arg);
    let path = if path.is_file() {
        path
    } else {
        let mut found = Vec::new();
        let mut exact = None;
        for persona in std::fs::read_dir(store)
            .with_context(|| format!("reading {}", store.display()))?
            .flatten()
        {
            let sessions = persona.path().join("sessions");
            let Ok(entries) = std::fs::read_dir(&sessions) else {
                continue;
            };
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if let Some(id) = name.strip_suffix(".jsonl") {
                    // An exact id wins over every prefix match, wherever
                    // the walk finds it (review of #612, pass 5).
                    if id == arg {
                        exact = Some(e.path());
                    } else if id.starts_with(arg) {
                        found.push(e.path());
                    }
                }
            }
        }
        match found.len() {
            _ if exact.is_some() => exact.unwrap_or_default(),
            1 => found.remove(0),
            0 => bail!("no persona chat `{arg}` in {}", store.display()),
            n => bail!("`{arg}` names {n} persona chats; give more of the id"),
        }
    };
    let sessions = path.parent().context("a transcript has a folder")?;
    if sessions.file_name().and_then(|n| n.to_str()) != Some("sessions") {
        bail!("{} is not in a persona's sessions folder", path.display());
    }
    let persona = sessions
        .parent()
        .and_then(Path::file_name)
        .and_then(|n| n.to_str())
        .context("a persona folder")?
        .to_string();
    let id = path
        .file_stem()
        .and_then(|n| n.to_str())
        .context("a transcript name")?
        .to_string();
    Ok((path, persona, id))
}

/// Whether a call is live now. The speech engine logs a real-time
/// factor per sentence it speaks, and a hold shows only a turn in flight,
/// not a call between turns. Not installed is no call; installed and
/// unreadable is a refusal, since unknown is not clear. A `systemctl` that
/// cannot be run at all reads as not installed: on a machine without a user
/// systemd there is no speech engine unit to be on a call through.
fn voice_live() -> Result<bool> {
    const UNIT: &str = "mecha-breeze-tts.service";
    let installed = std::process::Command::new("systemctl")
        .args(["--user", "cat", UNIT])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !installed {
        return Ok(false);
    }
    let out = std::process::Command::new("journalctl")
        .args([
            "--user",
            "-u",
            UNIT,
            "--since",
            "-3 min",
            "--no-pager",
            "-o",
            "cat",
        ])
        .output()
        .context("reading the speech engine's journal for a live call")?;
    if !out.status.success() {
        bail!(
            "the speech engine's journal did not read, so a live call cannot be ruled out: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|l| l.contains("RTF")))
}

/// Wait while the owner is using the model: a call, or a turn in flight on
/// the router (a hold some other process took). Checked before every
/// sample, because a long run outlasts the moment it started in.
async fn wait_for_owner(router: Option<&str>) -> Result<()> {
    let me = std::process::id();
    let mut said = false;
    loop {
        let call = voice_live()?;
        let turns = match router {
            Some(base) => mecha_core::hold::Holds::open_default()?
                .live(base)
                .into_iter()
                .filter(|h| h.pid != me)
                .map(|h| h.what)
                .collect(),
            None => Vec::new(),
        };
        if !call && turns.is_empty() {
            return Ok(());
        }
        if !said {
            eprintln!(
                "waiting: {}",
                if call {
                    "a voice call is live".to_string()
                } else {
                    format!("the model is in use ({})", turns.join(", "))
                }
            );
            said = true;
        }
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
    }
}

fn private_file(path: &Path) -> Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    // Narrowed to 0700 only when this call makes it: a folder the owner
    // named (`--out ~/c1.jsonl`) is not the harness's to narrow. The file
    // itself is 0600 either way (review of #612, pass 4).
    if let Some(dir) = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty() && !d.is_dir())
    {
        mecha_core::create_private_dir(dir)
            .with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("creating {}", path.display()))
}

/// Refuse a sample that would load `model` over what the router holds,
/// unless `allow`. `resident` is a claim about statuses, so it is gated on
/// `readable` as every reader of it is: a list this build cannot read, or
/// more than one model resident, is not "nothing loaded", and read that way
/// the refusal would never fire while the sample evicted the owner's pick.
fn refuse_load(
    base: &str,
    models: &[mecha_core::provider::router::RouterModel],
    model: &str,
    allow: bool,
) -> Result<()> {
    use mecha_core::provider::router::{readable, resident};
    if allow {
        return Ok(());
    }
    if !readable(models) {
        bail!(
            "the router at {base} answered /models with a list this cannot read (empty, or a \
             status it does not know), so whether this replay would evict the owner's model \
             is unknown — pass --allow-load to sample anyway"
        );
    }
    let loaded = models.iter().filter(|m| m.is_resident()).count();
    match resident(models) {
        Some(r) if r != model => bail!(
            "the router holds `{r}`, and this replay would load `{model}` in its place — \
             switch first (`mecha model use`), or pass --allow-load"
        ),
        None if loaded > 1 => bail!(
            "the router at {base} has {loaded} models resident, so which one this replay \
             would swap out is a guess — switch first (`mecha model use`), or pass --allow-load"
        ),
        _ => Ok(()),
    }
}

/// [`refuse_load`] against the router as it answers now. A server that is
/// not a router has nothing to evict. One that did not answer whether it is
/// one, or did not list its models, is unknown, and unknown refuses: this
/// process names its provider and never follows, so its first request
/// really can load a model over the owner's.
async fn guard_load(router: Option<&str>, model: &str, allow: bool) -> Result<()> {
    use mecha_core::provider::router::{is_router, models};
    let Some(base) = router else {
        return Ok(());
    };
    if allow {
        return Ok(());
    }
    match is_router(base).await {
        Some(false) => Ok(()),
        Some(true) => match models(base).await {
            Some(list) => refuse_load(base, &list, model, allow),
            None => bail!(
                "the router at {base} did not list its models, so whether this replay \
                 would evict the owner's model is unknown — pass --allow-load to sample anyway"
            ),
        },
        None => bail!(
            "the model server at {base} did not answer /props, so whether this replay \
             would evict the owner's model is unknown — pass --allow-load to sample anyway"
        ),
    }
}

/// The tool surface `branch`'s turn was sent: its recorded names, and the
/// specs the surface store holds under its `tools_hash`. Refused when the
/// recording names no surface or the store no longer holds it, rather than
/// standing today's text in for it.
fn recorded_tools_of(
    branch: &Branch,
    hash: Option<&str>,
) -> Result<(Vec<String>, Vec<mecha_core::message::ToolSpec>)> {
    let names = branch
        .config
        .as_ref()
        .map(|c| c.tools.clone())
        .context("the turn has no recorded run config, so no recorded surface")?;
    let hash = hash.context("the turn's run config names no tool surface (`tools_hash`)")?;
    let specs = mecha_core::surface::SurfaceStore::open_default()
        .and_then(|s| s.load(hash))
        .with_context(|| format!("the surface store no longer holds surface {hash}"))?;
    if mecha_core::surface::fingerprint(&specs) != hash {
        bail!("surface {hash} in the store does not hash to its name");
    }
    Ok((names, specs))
}

/// Refuse a replay that would send to another model than the turn ran on,
/// unless the command line asked for one (`asked`: `-m` or `-p`).
fn refuse_model_drift(recorded: Option<&str>, sending: &str, asked: bool) -> Result<()> {
    match recorded {
        Some(r) if r != sending && !asked => bail!(
            "the turn ran on `{r}`, and this replay would send to `{sending}` — pass -m or -p \
             to sample on another model deliberately"
        ),
        _ => Ok(()),
    }
}

/// An arm's overlay, read from `path`. One with no edits is the baseline
/// under another name, so it is refused here, as an edit that matches
/// nothing is refused per sample.
fn load_overlay(path: Option<&Path>) -> Result<Overlay> {
    let Some(f) = path else {
        return Ok(Overlay::default());
    };
    let overlay = Overlay::parse(
        &std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?,
    )?;
    if overlay.is_empty() {
        bail!(
            "{} has no `[[replace]]` edits, so the arm would measure the baseline",
            f.display()
        );
    }
    Ok(overlay)
}

pub async fn execute(global: &GlobalOpts, arg: &str, args: &SampleArgs, json: bool) -> Result<()> {
    let store = mecha_core::persona::Store::default_dir()?;
    let (path, persona, id) = resolve(&store, arg)?;
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;

    if args.list {
        let turns = pr::owner_turns(&path, &text)?;
        if json {
            println!("{}", serde_json::to_string(&turns)?);
        } else {
            println!(
                "{:>6} {:>5} {:>9} {:>6}  kind",
                "line", "turn", "messages", "notes"
            );
            for t in &turns {
                let kind = match (t.spoken, t.panel) {
                    _ if t.compacted => "compacted",
                    (_, true) => "panel",
                    (true, _) => "spoken",
                    _ => "typed",
                };
                println!(
                    "{:>6} {:>5} {:>9} {:>6}  {kind}",
                    t.line, t.turn, t.messages, t.notes
                );
            }
        }
        return Ok(());
    }

    let line = match (args.at, args.turn) {
        (Some(line), _) => line,
        (None, Some(k)) => pr::owner_turns(&path, &text)?
            .into_iter()
            .find(|t| t.turn == k)
            .map(|t| t.line)
            .with_context(|| format!("{id} has no owner turn {k}"))?,
        (None, None) => bail!("name the turn to branch at: --at <line> or --turn <n> (--list)"),
    };
    if args.samples == 0 {
        bail!("--samples must be at least 1");
    }
    let branch: Branch = match args.at_call {
        Some(call) => pr::branch_at_call(&path, &text, line, call)?,
        None => pr::branch_at(&path, &text, line)?,
    };
    let overlay = Arc::new(load_overlay(args.overlay.as_deref())?);

    let pin: Pin = serde_json::from_slice(
        &std::fs::read(path.with_file_name(format!("{id}.persona.json")))
            .context("reading which version the chat is pinned to")?,
    )?;
    if pin.persona != persona {
        bail!("the chat's pin names `{}`, not `{persona}`", pin.persona);
    }
    let pinned = mecha_core::persona::agent::load_version(&store, &persona, &pin.digest)?;

    // The binding: the provider the turn ran on, unless `-p` names another.
    let mut opts = global.clone();
    if opts.provider.is_none() {
        opts.provider = branch.config.as_ref().map(|c| c.provider.clone());
    }
    let follower = Follower::start(opts, Box::new(|_| {})).await?;
    let router = follower.router_base();
    let bound = follower.current();
    // Checked here so a run that would evict the owner's model stops before
    // it writes anything, and again before every sample (`Sampler::sample`).
    guard_load(router.as_deref(), &bound.model, args.allow_load).await?;
    let tz = bound.config.agent.timezone();
    let recorded_clock = branch.config.as_ref().and_then(|c| c.clock);
    let from = recorded_clock.unwrap_or_else(chrono::Utc::now) - chrono::Duration::days(2);
    let clock_at = branch
        .calendar
        .as_deref()
        .and_then(|c| pr::clock_for(c, from, tz));
    // What the replay's clock reads, said as it is: the matched day, the
    // recorded run's clock, or, with neither, today (review of #612, pass 5).
    let sent_clock = clock_at.or(recorded_clock);
    let fallback = match recorded_clock {
        Some(_) => "the recorded run's clock",
        None => "today's date (no run clock was recorded either)",
    };
    match (&branch.calendar, clock_at) {
        (None, _) => eprintln!(
            "note: no calendar reference was recorded for the turn's run; the replay sends \
             {fallback}"
        ),
        (Some(_), None) => eprintln!(
            "note: the turn's calendar reference renders on no day near the recorded clock; \
             the replay sends {fallback}"
        ),
        _ => {}
    }

    // The model the turn ran on, against the one this binding sends to: a
    // replay on another preset is another arm, so it is asked for (`-m`,
    // `-p`) rather than taken from whatever the config resolves today, as
    // `mecha replay` defaults to the recorded model (review of #612, pass 3).
    // Before the output file exists, so a refusal leaves none behind.
    let recorded_model = branch.config.as_ref().map(|c| c.model.clone());
    let model_matches = recorded_model.as_deref().map(|m| m == bound.model);
    refuse_model_drift(
        recorded_model.as_deref(),
        &bound.model,
        global.model.is_some() || global.provider.is_some(),
    )?;
    let out = match &args.out {
        // Absolute, so a bare file name's folder is the working directory
        // rather than an empty path no folder can be made at.
        Some(p) => {
            std::path::absolute(p).with_context(|| format!("resolving --out {}", p.display()))?
        }
        None => mecha_core::work::mecha_home()?
            .join("research/replay")
            .join(&persona)
            .join(format!(
                "{id}-L{line}-{}.jsonl",
                chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
            )),
    };
    let mut file = private_file(&out)?;
    let work = out.with_extension("work");
    mecha_core::create_private_dir(&work)?;

    // What today's build sends, against what the turn was sent: the
    // surface and the system text, by fingerprint.
    let probe = crate::setup::persona_agent(
        &bound,
        &pinned,
        crate::setup::persona_provider(&bound, PersonaUse::Converse)?,
        &store,
    )?
    .0;
    let today = mecha_core::surface::fingerprint(&probe.registry().specs());
    let recorded_surface = branch.config.as_ref().and_then(|c| c.tools_hash.clone());
    let recorded_tools = match args.surface.as_str() {
        "today" => None,
        "recorded" => Some(recorded_tools_of(&branch, recorded_surface.as_deref())?),
        other => bail!("--surface is today or recorded, not `{other}`"),
    };
    // What the requests will carry, by fingerprint: the stand-ins are
    // checked against it before every sample.
    let surface = recorded_tools
        .as_ref()
        .map(|(_, specs)| mecha_core::surface::fingerprint(specs))
        .unwrap_or_else(|| today.clone());
    if recorded_tools.is_none()
        && !overlay.is_empty()
        && recorded_surface.as_deref().is_some_and(|r| r != today)
    {
        eprintln!(
            "note: this turn was sent another tool surface ({}) than this build's ({today}); \
             the overlay edits today's text — `--surface recorded` starts from what the \
             model saw",
            recorded_surface.as_deref().unwrap_or_default()
        );
    }
    let system_matches = branch
        .config
        .as_ref()
        .map(|c| c.system_prompt.as_deref() == probe.system());
    // A picture call renders unless asked not to, where the persona can
    // draw at all.
    let render = !args.no_render && probe.registry().get(IMAGE_TOOL).is_some();
    if !args.no_render && !render {
        eprintln!("note: this persona has no `{IMAGE_TOOL}`, so nothing is drawn");
    }
    if args.attempts == 0 {
        bail!("--attempts must be at least 1");
    }
    if render && args.attempts > 1 {
        bail!(
            "--attempts is call-only: a parsed picture call ends the sample rather than \
             drawing — add --no-render"
        );
    }
    if args.attempts > 1 && recorded_tools.is_some() {
        bail!(
            "--surface recorded and --attempts both describe the picture tool: an attempt's \
             unparsed call is answered by today's tool, in today's words — use one or the other"
        );
    }
    if render && recorded_tools.is_some() {
        bail!(
            "--surface recorded is call-only: a picture call that runs is today's tool, \
             described in today's words — add --no-render"
        );
    }
    let exe = std::env::current_exe()?;
    let header = serde_json::json!({
        "header": {
            "session": id,
            "persona": persona,
            "pinned_version": pinned.version,
            "line": line,
            "messages": branch.messages.len(),
            "notes": branch.notes.len(),
            "spoken": branch.spoken,
            "provider": bound.provider_name,
            "model": bound.model,
            "recorded_model": recorded_model,
            "model_matches_recorded": model_matches,
            "arm": overlay.name,
            "overlay_digest": overlay.digest,
            "max_tokens": args.max_tokens,
            "samples": args.samples,
            "seed_base": args.seed_base,
            "binary": exe.display().to_string(),
            "binary_sha256": mecha_core::document::sha256_hex(&std::fs::read(&exe)?),
            "version": env!("CARGO_PKG_VERSION"),
            "surface_mode": args.surface,
            "surface": surface,
            "today_surface": today,
            "recorded_surface": recorded_surface,
            // Of what was sent: an overlay that edits tool text sends
            // another surface than either fingerprint names.
            "surface_matches_recorded": recorded_surface
                .as_deref()
                .map(|r| r == surface && !overlay.edits_tools()),
            "surface_edited_by_overlay": overlay.edits_tools(),
            "system_matches_recorded": system_matches,
            "calendar_matched": clock_at.is_some(),
            // `null`: the system clock, today's date.
            "clock": sent_clock,
            "call": branch.call,
            "render": render,
            "attempts": args.attempts,
            "parsed_limit": (args.attempts > 1).then_some(args.parsed_limit),
            "readers_stamped": render && !args.no_readers,
            "image_seed_base": render.then_some(args.image_seed_base),
            "tools_run": if render { vec![IMAGE_TOOL] } else { Vec::new() },
        }
    });
    writeln!(file, "{header}")?;
    drop(probe);

    let sampler = Sampler {
        follower,
        router,
        store,
        persona,
        pinned,
        branch,
        overlay,
        max_tokens: args.max_tokens,
        clock: mecha_core::clock::for_replay(sent_clock),
        recorded_tools,
        attempts: args.attempts,
        parsed_limit: args.parsed_limit,
        pinned_model: bound.model.clone(),
        allow_load: args.allow_load,
        surface,
        work,
        chat: id.clone(),
        run_start: clock_at
            .and_then(|t| pr::start_of_day(t, tz))
            .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        render,
        readers: !args.no_readers,
    };
    let mut samples = Vec::with_capacity(args.samples);
    for i in 0..args.samples {
        let sample = sampler
            .sample(
                i,
                args.seed_base + i as u64,
                args.image_seed_base + i as u64,
            )
            .await?;
        writeln!(file, "{}", serde_json::to_string(&sample)?)?;
        eprintln!(
            "sample {}/{}: {} call(s), {}",
            i + 1,
            args.samples,
            sample.tool_calls.len(),
            sample
                .stop_reason
                .map(|s| format!("{s:?}"))
                .or_else(|| sample.error.as_ref().map(|_| "error".into()))
                .unwrap_or_default()
        );
        samples.push(sample);
    }

    // Removed when nothing was staged into it; a render's pictures stay.
    let _ = std::fs::remove_dir(&sampler.work);
    let summary = summarise(&samples);
    if json {
        println!(
            "{}",
            serde_json::json!({"out": out.display().to_string(), "summary": summary})
        );
    } else {
        println!("samples written to {}", out.display());
        println!("{}", serde_json::to_string_pretty(&summary)?);
    }
    Ok(())
}

/// Everything a sample needs, set up once per branch point and arm. One
/// [`Sampler::sample`] is one sample from start to finish, so a later
/// `exp` trial kind calls the same function `replay` does (owner ruling,
/// 2026-10-09: render in replay now, build into exp later).
pub struct Sampler {
    follower: Follower,
    router: Option<String>,
    store: PathBuf,
    persona: String,
    pinned: mecha_core::persona::agent::Pinned,
    branch: Branch,
    overlay: Arc<Overlay>,
    max_tokens: Option<u32>,
    clock: Arc<dyn mecha_core::clock::Clock>,
    /// The recorded surface's tool names and specs, for `--surface
    /// recorded`: the stand-ins are described by them.
    recorded_tools: Option<(Vec<String>, Vec<mecha_core::message::ToolSpec>)>,
    /// Requests a sample may send, and the parsed-call size that ends it.
    attempts: usize,
    parsed_limit: usize,
    /// The model the replay sends to, and whether it may load it over
    /// what the router holds (`guard_load`).
    pinned_model: String,
    allow_load: bool,
    /// Today's tool surface, which the stand-ins must reproduce.
    surface: String,
    work: PathBuf,
    /// The chat's id, which its scene is staged by.
    chat: String,
    /// A bound the turn's run began after: the start of the day its
    /// calendar reference names, in the run's zone. The approximate stage
    /// takes the persona's latest written at or before it, so it can be up
    /// to a day early, never late: a later bound (noon) staged a scene drawn
    /// after a morning turn (review of #615). `None` falls back to the last
    /// recorded config's clock.
    run_start: Option<String>,
    /// Run the picture call against a scene staged per sample.
    render: bool,
    /// Stamp the scene reader and role splitter when rendering.
    readers: bool,
}

/// The one tool a sample runs, when it renders.
const IMAGE_TOOL: &str = "image_generate";

/// The picture tool for an `--attempts` sample: a call whose arguments did
/// not parse is answered by the real tool, which refuses it before any
/// drawing (there is no `scene` or `picture` to draw), as it was answered on
/// the turn; a parsed call ends the sample unrun.
struct AnswerUnparsed {
    inner: Arc<dyn mecha_core::tool::Tool>,
    cancel: CancellationToken,
}

#[async_trait::async_trait]
impl mecha_core::tool::Tool for AnswerUnparsed {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    fn input_schema(&self) -> serde_json::Value {
        self.inner.input_schema()
    }
    fn read_only(&self) -> bool {
        self.inner.read_only()
    }
    fn capabilities(&self) -> mecha_core::tool::Capabilities {
        self.inner.capabilities()
    }
    async fn call(
        &self,
        input: serde_json::Value,
        ctx: &mecha_core::tool::ToolCtx,
    ) -> Result<mecha_core::tool::ToolOutput> {
        if is_unparsed(&input) {
            return self.inner.call(input, ctx).await;
        }
        self.cancel.cancel();
        Ok(mecha_core::tool::ToolOutput::err(
            "replay: a parsed call ends the sample; it was not run",
        ))
    }
}

/// Whether a call's input is the provider's wrapper for arguments that did
/// not parse, and nothing else.
fn is_unparsed(input: &serde_json::Value) -> bool {
    input
        .as_object()
        .is_some_and(|o| o.len() == 1 && o.contains_key("__malformed_arguments"))
}

/// Every file under `dir`, relative.
fn files_under(dir: &Path) -> std::collections::BTreeSet<PathBuf> {
    let mut out = std::collections::BTreeSet::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(dir) {
                out.insert(rel.to_path_buf());
            }
        }
    }
    out
}

impl Sampler {
    /// Sample `i` at `seed`: wait for the owner, hold the router, build the
    /// persona's agent and the turn's context as serve does, send one
    /// request, and report what came back. An error the sample itself hit
    /// is in the sample; `Err` is a harness that could not run it.
    pub async fn sample(&self, i: usize, seed: u64, image_seed: u64) -> Result<Sample> {
        wait_for_owner(self.router.as_deref()).await?;
        // Again per sample: a long run can outlast a `mecha model use`, and
        // this process does not follow the switch.
        guard_load(self.router.as_deref(), &self.pinned_model, self.allow_load).await?;
        let (held, bound) = self
            .follower
            .enter("persona replay", |s| {
                eprintln!("waiting: the model is switching to {}", s.to)
            })
            .await?;
        let cancel = CancellationToken::new();
        let log = Arc::new(Mutex::new(Vec::new()));
        let branch = &self.branch;
        // The chat's scene as of the turn, staged fresh for this sample: a
        // store shared across samples carries one sample's picture into the
        // next one's scene.
        let staged = if self.render {
            let scratch = self.work.join(format!("sample-{i:03}"));
            Some(mecha_core::scene::stage::stage_at(
                &self.store,
                &self.persona,
                &self.chat,
                branch.line - 1,
                self.run_start.as_deref(),
                &scratch,
            )?)
        } else {
            None
        };
        let capture = Capture {
            inner: crate::setup::persona_provider_seeded(&bound, PersonaUse::Converse, seed)?,
            overlay: Arc::clone(&self.overlay),
            max_tokens: self.max_tokens,
            requests: self.attempts,
            // A render must run its call, and an attempt's unparsed call must
            // be answered: the sample ends when the run asks past its
            // requests (`SampleEnded`), not as the response comes back.
            cancel_after_last: staged.is_none() && self.attempts == 1,
            cancel: cancel.clone(),
            log: Arc::clone(&log),
        };
        let (agent, _) =
            crate::setup::persona_agent(&bound, &self.pinned, Box::new(capture), &self.store)?;
        let mut agent = agent.with_clock(Arc::clone(&self.clock));
        // The same tools, described by their own words, that never run: a
        // call to one cancels the run (`OnDivergence::Stop` with nothing
        // recorded). Rendering, the picture tool is the real one.
        let real = (staged.is_some() || self.attempts > 1)
            .then(|| agent.registry().get(IMAGE_TOOL).cloned())
            .flatten();
        let (names, specs, fallback) = match &self.recorded_tools {
            // The recording's names and words; a tool this build no longer
            // has is described from its spec and never runs.
            Some((names, specs)) => (
                names.clone(),
                specs.as_slice(),
                Some(crate::setup::surface_only_registry()),
            ),
            None => (
                agent
                    .registry()
                    .iter()
                    .map(|t| t.name().to_string())
                    .filter(|n| real.is_none() || n != IMAGE_TOOL)
                    .collect(),
                &[][..],
                None,
            ),
        };
        let mut tools = replay_registry(
            &names,
            agent.registry(),
            fallback.as_ref(),
            specs,
            Vec::new(),
            OnDivergence::Stop,
            cancel.clone(),
        )?;
        if let Some(real) = real {
            if staged.is_some() {
                tools.insert(real);
            } else {
                tools.insert(Arc::new(AnswerUnparsed {
                    inner: real,
                    cancel: cancel.clone(),
                }));
            }
        }
        if mecha_core::surface::fingerprint(&tools.specs()) != self.surface {
            bail!("the replay's tools do not describe themselves as the real ones do");
        }
        *agent.registry_mut() = tools;
        let judge = || crate::setup::persona_provider(&bound, PersonaUse::Judge);
        let mut turn = mecha_core::persona::turn::Turn {
            workspace: self.work.clone(),
            scene: None,
            prompt_log: None,
            image_seeds: None,
            owner: &branch.owner,
            history: &branch.messages,
            panel: false,
            spoken: branch.spoken,
            notes: branch.notes.clone(),
            persona: self.persona.clone(),
            character: self.pinned.settings.character.clone(),
            library: mecha_core::imagelib::Library::default_dir().unwrap_or_default(),
            model: bound.model.clone(),
            readers: mecha_core::persona::turn::Readers::Off,
        };
        // Stamped into the scratch store only (`scene::stage` refuses one
        // that overlaps the real store or workspace), never the chat's own.
        if let Some(st) = &staged {
            turn.workspace = st.workspace.clone();
            turn.scene = Some(st.slot.clone());
            turn.prompt_log = Some(st.prompt_log.clone());
            turn.image_seeds = Some(Arc::new(mecha_core::sample::SeedStream::new(image_seed)));
            if self.readers {
                turn.readers = mecha_core::persona::turn::Readers::From(&judge);
            }
        }
        let before = staged
            .as_ref()
            .map(|st| files_under(&st.workspace))
            .unwrap_or_default();
        let cx = mecha_core::persona::turn::context(&agent, turn).with_cancel(cancel);
        let mut convo =
            mecha_core::agent::Conversation::resumed(branch.messages.clone(), branch.taint);
        let ran = agent.run_in(&cx, &mut convo, None).await;
        drop(held);
        let error = ran
            .err()
            .filter(|e| !pr::ended(e))
            .map(|e| format!("{e:#}"));
        let log = log.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut sample = Sample::of(
            i,
            Some(seed),
            branch.line,
            &bound.model,
            &self.overlay,
            &log,
            error,
        );
        sample.call = branch.call;
        if self.attempts > 1 {
            // Every exchange, even one: a sample that parsed on its first
            // request is a 1 in the histogram, not missing from it.
            sample.exchanges = log.iter().map(pr::ExchangeFacts::of).collect();
            sample.attempts_to_parsed = sample.first_parsed_within(self.parsed_limit);
        }
        if let Some(st) = &staged {
            let after = &convo.messages[branch.messages.len().min(convo.messages.len())..];
            let new: Vec<PathBuf> = files_under(&st.workspace)
                .difference(&before)
                .cloned()
                .collect();
            let abs = |p: &PathBuf| st.workspace.join(p).display().to_string();
            sample.render = Some(pr::RenderFacts {
                scratch: st.store.parent().unwrap_or(&st.store).display().to_string(),
                as_of: format!("{:?}", st.as_of),
                missing: st.missing.clone(),
                readers: self.readers,
                image_seed,
                results: pr::results_of(after, IMAGE_TOOL),
                pictures: new
                    .iter()
                    .filter(|p| p.extension().is_some_and(|x| x == "png"))
                    .map(abs)
                    .collect(),
                manifests: new
                    .iter()
                    .filter(|p| p.extension().is_some_and(|x| x == "json"))
                    .filter_map(|p| std::fs::read(st.workspace.join(p)).ok())
                    .filter_map(|b| serde_json::from_slice(&b).ok())
                    .collect(),
                prompt_log: std::fs::read_to_string(&st.prompt_log)
                    .map(|t| t.lines().map(str::to_string).collect())
                    .unwrap_or_default(),
            });
        }
        if log.is_empty() {
            bail!(
                "sample {i} sent no request: {}",
                sample.error.as_deref().unwrap_or("no error was given")
            );
        }
        Ok(sample)
    }
}

/// How many samples first parsed a call within the limit on each request;
/// how many parsed one only over it (`"too_big"`: a parsed call ends the
/// sample, so it cannot try again); and how many never parsed one
/// (`"none"`). `null` when no sample was allowed more than one.
fn attempts_histogram(samples: &[Sample]) -> Option<BTreeMap<String, usize>> {
    let tried: Vec<&Sample> = samples
        .iter()
        .filter(|s| s.error.is_none() && !s.exchanges.is_empty())
        .collect();
    if tried.is_empty() {
        return None;
    }
    let mut h = BTreeMap::new();
    for s in tried {
        // A parsed call ends the sample whatever its size, so one over the
        // limit is its own outcome, never "none" (which is: no call parsed).
        let too_big = s
            .exchanges
            .iter()
            .flat_map(|e| &e.tool_calls)
            .any(|c| c.parsed);
        let key = match s.attempts_to_parsed {
            Some(n) => n.to_string(),
            None if too_big => "too_big".to_string(),
            None => "none".to_string(),
        };
        *h.entry(key).or_default() += 1;
    }
    Some(h)
}

/// What a set of samples did, as counts: no words.
fn summarise(samples: &[Sample]) -> serde_json::Value {
    let mut stops: BTreeMap<String, usize> = BTreeMap::new();
    let mut tools: BTreeMap<String, usize> = BTreeMap::new();
    let mut digests: BTreeMap<String, usize> = BTreeMap::new();
    let (mut no_call, mut unparsed, mut errors) = (0, 0, 0);
    for s in samples {
        // A sample with no response observed nothing: it made no call because
        // it got no answer, and its tokens are unknown rather than zero.
        // Counted as an error and in no other bucket. One whose response came
        // back and whose run failed afterwards is counted for what it did.
        if s.stop_reason.is_none() {
            errors += 1;
            continue;
        }
        if let Some(r) = s.stop_reason {
            *stops.entry(format!("{r:?}")).or_default() += 1;
        }
        if s.tool_calls.is_empty() {
            no_call += 1;
        }
        for c in &s.tool_calls {
            *tools.entry(c.name.clone()).or_default() += 1;
            if !c.parsed {
                unparsed += 1;
            }
        }
        if let Some(d) = &s.messages_digest {
            *digests.entry(d.clone()).or_default() += 1;
        }
    }
    let rendered = samples
        .iter()
        .filter(|s| s.render.as_ref().is_some_and(|r| !r.pictures.is_empty()))
        .count();
    let render_errors = samples
        .iter()
        .filter_map(|s| s.render.as_ref())
        .flat_map(|r| &r.results)
        .filter(|r| r.is_error)
        .count();
    let output: Vec<u64> = samples
        .iter()
        .filter(|s| s.stop_reason.is_some())
        .map(|s| s.output_tokens)
        .collect();
    serde_json::json!({
        "samples": samples.len(),
        "errors": errors,
        "no_call": no_call,
        "calls_by_tool": tools,
        "unparsed_calls": unparsed,
        "stop_reasons": stops,
        "output_tokens_mean": (!output.is_empty())
            .then(|| output.iter().sum::<u64>() as f64 / output.len() as f64),
        "distinct_histories": digests.len(),
        "attempts_to_parsed": attempts_histogram(samples),
        "rendered": rendered,
        "render_errors": render_errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, status: &str) -> mecha_core::provider::router::RouterModel {
        serde_json::from_value(serde_json::json!({"id": id, "status": {"value": status}})).unwrap()
    }

    #[test]
    fn a_sample_never_loads_over_a_model_it_cannot_rule_out() {
        let base = "http://127.0.0.1:8080";
        let ok = |m: &[_], allow| refuse_load(base, m, "a", allow).is_ok();
        assert!(ok(&[model("a", "loaded"), model("b", "unloaded")], false));
        assert!(
            ok(&[model("a", "unloaded"), model("b", "unloaded")], false),
            "nothing loaded"
        );
        assert!(
            !ok(&[model("b", "loaded"), model("a", "unloaded")], false),
            "another loaded"
        );
        assert!(
            !ok(&[model("a", "loaded"), model("b", "loaded")], false),
            "two resident"
        );
        assert!(
            !ok(&[model("a", "warming")], false),
            "a status this build does not know"
        );
        assert!(!ok(&[], false), "an empty list");
        assert!(ok(&[model("b", "loaded")], true), "--allow-load");
    }

    /// A server that does not answer is not "no router": the guard refuses,
    /// unless told to sample anyway (review of #612, pass 2).
    #[tokio::test]
    async fn a_router_that_does_not_answer_refuses_a_sample() {
        let silent = Some("http://127.0.0.1:9");
        let err = guard_load(silent, "a", false).await.unwrap_err();
        assert!(format!("{err:#}").contains("did not answer"), "{err:#}");
        assert!(guard_load(silent, "a", true).await.is_ok(), "--allow-load");
        assert!(
            guard_load(None, "a", false).await.is_ok(),
            "no local router"
        );
    }

    /// `--surface recorded` reads the turn's own tool text back from the
    /// surface store, and refuses rather than stand today's in for one the
    /// store no longer holds.
    #[test]
    fn the_recorded_surface_is_the_turns_own_or_nothing() {
        let _home = crate::testenv::HomeGuard::new("replay-surface");
        let specs = vec![mecha_core::message::ToolSpec {
            name: "widget".into(),
            description: "An old description.".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }];
        let hash = mecha_core::surface::SurfaceStore::open_default()
            .unwrap()
            .record(&specs)
            .unwrap();
        let branch = |hash: Option<&str>| Branch {
            line: 1,
            messages: Vec::new(),
            taint: Default::default(),
            notes: Vec::new(),
            owner: String::new(),
            spoken: false,
            config: Some(mecha_core::session::RunConfig {
                tools: vec!["widget".into()],
                tools_hash: hash.map(str::to_string),
                ..Default::default()
            }),
            calendar: None,
            call: None,
        };
        let (names, got) = recorded_tools_of(&branch(Some(&hash)), Some(&hash)).unwrap();
        assert_eq!(names, ["widget"]);
        assert_eq!(got[0].description, "An old description.");
        let err = recorded_tools_of(&branch(Some("0000000000000000")), Some("0000000000000000"))
            .unwrap_err();
        assert!(format!("{err:#}").contains("no longer holds"), "{err:#}");
        assert!(recorded_tools_of(&branch(None), None).is_err());
    }

    struct Counting(Arc<Mutex<usize>>);

    #[async_trait::async_trait]
    impl mecha_core::tool::Tool for Counting {
        fn name(&self) -> &str {
            "image_generate"
        }
        fn description(&self) -> &str {
            "a picture"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn call(
            &self,
            _input: serde_json::Value,
            _ctx: &mecha_core::tool::ToolCtx,
        ) -> Result<mecha_core::tool::ToolOutput> {
            *self.0.lock().unwrap() += 1;
            Ok(mecha_core::tool::ToolOutput::err("Nothing was drawn."))
        }
    }

    /// An attempt's unparsed call reaches the real tool's refusal; a parsed
    /// one never runs, and ends the sample.
    #[tokio::test]
    async fn only_an_unparsed_call_reaches_the_picture_tool() {
        use mecha_core::tool::Tool;
        let calls = Arc::new(Mutex::new(0));
        let cancel = CancellationToken::new();
        let tool = AnswerUnparsed {
            inner: Arc::new(Counting(Arc::clone(&calls))),
            cancel: cancel.clone(),
        };
        let ctx = mecha_core::tool::ToolCtx::default();
        let out = tool
            .call(
                serde_json::json!({"__malformed_arguments": "{\"scene\": "}),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(out.content, "Nothing was drawn.");
        assert!(!cancel.is_cancelled());
        let out = tool
            .call(serde_json::json!({"scene": {"setting": "a room"}}), &ctx)
            .await
            .unwrap();
        assert!(
            out.is_error && out.content.contains("not run"),
            "{}",
            out.content
        );
        assert!(cancel.is_cancelled());
        assert_eq!(*calls.lock().unwrap(), 1, "the parsed call never ran");
        assert!(!is_unparsed(
            &serde_json::json!({"__malformed_arguments": "x", "scene": {}})
        ));
    }

    #[test]
    fn attempts_count_the_first_request_with_a_small_parsed_call() {
        let call = |bytes: usize, parsed: bool| pr::CallFacts {
            name: "image_generate".into(),
            argument_bytes: bytes,
            parsed,
            arguments: serde_json::json!({}),
        };
        let exchange = |calls: Vec<pr::CallFacts>| pr::ExchangeFacts {
            tool_calls: calls,
            ..Default::default()
        };
        let sample = |ex: Vec<pr::ExchangeFacts>| {
            let mut s = Sample::of(0, None, 1, "m", &Overlay::default(), &[], None);
            s.exchanges = ex;
            s.attempts_to_parsed = s.first_parsed_within(2048);
            s
        };
        let first = sample(vec![exchange(vec![call(400, true)])]);
        let second = sample(vec![
            exchange(vec![call(30_000, false)]),
            exchange(vec![call(500, true)]),
        ]);
        let too_big = sample(vec![exchange(vec![call(9_000, true)])]);
        let words = sample(vec![exchange(Vec::new())]);
        assert_eq!(first.attempts_to_parsed, Some(1));
        assert_eq!(second.attempts_to_parsed, Some(2));
        assert_eq!(too_big.attempts_to_parsed, None);
        let h = attempts_histogram(&[first, second, too_big, words]).unwrap();
        assert_eq!(h["1"], 1);
        assert_eq!(h["2"], 1);
        assert_eq!(h["too_big"], 1, "parsed but over the limit is not unparsed");
        assert_eq!(h["none"], 1);
        assert!(attempts_histogram(&[]).is_none());
    }

    #[test]
    fn a_replay_on_another_model_is_asked_for() {
        assert!(refuse_model_drift(Some("a"), "a", false).is_ok());
        let err = refuse_model_drift(Some("a"), "b", false).unwrap_err();
        assert!(format!("{err:#}").contains("ran on `a`"), "{err:#}");
        assert!(refuse_model_drift(Some("a"), "b", true).is_ok(), "-m or -p");
        assert!(
            refuse_model_drift(None, "b", false).is_ok(),
            "nothing recorded"
        );
    }

    #[test]
    fn an_overlay_with_no_edits_is_refused() {
        let dir = std::env::temp_dir().join(format!("mecha-rp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("arm.toml");
        std::fs::write(&f, "name = \"C\"\n# [[replace]]\n").unwrap();
        let err = load_overlay(Some(&f)).unwrap_err();
        assert!(
            format!("{err:#}").contains("no `[[replace]]` edits"),
            "{err:#}"
        );
        assert!(
            load_overlay(None).unwrap().is_empty(),
            "no overlay is the baseline"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_errored_sample_counts_as_an_error_and_nothing_else() {
        let blank = |error: Option<&str>, tokens: u64, calls: usize| Sample {
            output_tokens: tokens,
            error: error.map(str::to_string),
            stop_reason: error
                .is_none()
                .then_some(mecha_core::message::StopReason::EndTurn),
            tool_calls: (0..calls)
                .map(|_| pr::CallFacts {
                    name: "widget".into(),
                    argument_bytes: 2,
                    parsed: true,
                    arguments: serde_json::json!({}),
                })
                .collect(),
            ..Sample::of(0, None, 1, "m", &Overlay::default(), &[], None)
        };
        let s = summarise(&[
            blank(None, 100, 1),
            blank(None, 300, 0),
            blank(Some("a 500"), 0, 0),
        ]);
        assert_eq!(s["errors"], 1);
        assert_eq!(
            s["no_call"], 1,
            "the errored sample is not a choice not to call"
        );
        assert_eq!(s["output_tokens_mean"], 200.0);
        // A response that came back, from a run that failed after it, is
        // counted for what it did.
        let mut late = blank(None, 50, 1);
        late.error = Some("the run failed after the answer".into());
        let s2 = summarise(&[late]);
        assert_eq!(s2["errors"], 0);
        assert_eq!(s2["calls_by_tool"]["widget"], 1);
        let all_failed = summarise(&[blank(Some("a 500"), 0, 0)]);
        assert!(
            all_failed["output_tokens_mean"].is_null(),
            "a mean over nothing is none"
        );
    }

    /// A bare file name lands in the working directory, and a failure names
    /// the path (found by mecha-a3: `--out c1.jsonl` failed every sample
    /// with an error that named nothing).
    #[test]
    fn an_output_file_named_bare_is_created_where_it_is_named() {
        let dir = std::env::temp_dir().join(format!("mecha-rp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let bare = std::path::absolute(dir.join("c1.jsonl")).unwrap();
        private_file(&bare).unwrap();
        assert!(bare.is_file());
        let err = private_file(&bare).unwrap_err();
        assert!(format!("{err:#}").contains("c1.jsonl"), "{err:#}");
        assert!(private_file(Path::new("")).is_err());
        // A folder the owner named keeps its mode; one this makes is 0700.
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        private_file(&dir.join("c2.jsonl")).unwrap();
        assert_eq!(mode(&dir), 0o755, "an existing folder is not narrowed");
        private_file(&dir.join("made").join("c3.jsonl")).unwrap();
        assert_eq!(mode(&dir.join("made")), 0o700);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
