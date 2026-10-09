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
                    if id == arg {
                        found = vec![e.path()];
                        break;
                    }
                    if id.starts_with(arg) {
                        found.push(e.path());
                    }
                }
            }
        }
        match found.len() {
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
/// unreadable is a refusal, since unknown is not clear.
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
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
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
    let branch: Branch = pr::branch_at(&path, &text, line)?;
    let overlay = match &args.overlay {
        Some(f) => Overlay::parse(
            &std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?,
        )?,
        None => Overlay::default(),
    };
    let overlay = Arc::new(overlay);

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
    if let Some(base) = &router {
        if let Some(models) = mecha_core::provider::router::models(base).await {
            let resident = mecha_core::provider::router::resident(&models);
            if resident.is_some_and(|r| r != bound.model) && !args.allow_load {
                bail!(
                    "the router holds `{}`, and this replay would load `{}` in its place — \
                     switch first (`mecha model use`), or pass --allow-load",
                    resident.unwrap_or_default(),
                    bound.model
                );
            }
        }
    }
    let tz = bound.config.agent.timezone();
    let recorded_clock = branch.config.as_ref().and_then(|c| c.clock);
    let from = recorded_clock.unwrap_or_else(chrono::Utc::now) - chrono::Duration::days(2);
    let clock_at = branch
        .calendar
        .as_deref()
        .and_then(|c| pr::clock_for(c, from, tz));
    if clock_at.is_none() {
        eprintln!(
            "note: the turn's calendar reference could not be matched to a day; \
             the replay sends the recorded run's clock instead"
        );
    }

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
    let surface = mecha_core::surface::fingerprint(&probe.registry().specs());
    let recorded_surface = branch.config.as_ref().and_then(|c| c.tools_hash.clone());
    let system_matches = branch
        .config
        .as_ref()
        .map(|c| c.system_prompt.as_deref() == probe.system());
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
            "arm": overlay.name,
            "overlay_digest": overlay.digest,
            "max_tokens": args.max_tokens,
            "samples": args.samples,
            "seed_base": args.seed_base,
            "binary": exe.display().to_string(),
            "binary_sha256": mecha_core::document::sha256_hex(&std::fs::read(&exe)?),
            "version": env!("CARGO_PKG_VERSION"),
            "surface": surface,
            "recorded_surface": recorded_surface,
            "surface_matches_recorded": recorded_surface.as_deref().map(|r| r == surface),
            "system_matches_recorded": system_matches,
            "calendar_matched": clock_at.is_some(),
            "readers_stamped": false,
            "tools_run": false,
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
        clock: mecha_core::clock::for_replay(clock_at.or(recorded_clock)),
        surface,
        work,
    };
    let mut samples = Vec::with_capacity(args.samples);
    for i in 0..args.samples {
        let sample = sampler.sample(i, args.seed_base + i as u64).await?;
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
    /// Today's tool surface, which the stand-ins must reproduce.
    surface: String,
    work: PathBuf,
}

impl Sampler {
    /// Sample `i` at `seed`: wait for the owner, hold the router, build the
    /// persona's agent and the turn's context as serve does, send one
    /// request, and report what came back. An error the sample itself hit
    /// is in the sample; `Err` is a harness that could not run it.
    pub async fn sample(&self, i: usize, seed: u64) -> Result<Sample> {
        wait_for_owner(self.router.as_deref()).await?;
        let (held, bound) = self
            .follower
            .enter("persona replay", |s| {
                eprintln!("waiting: the model is switching to {}", s.to)
            })
            .await?;
        let cancel = CancellationToken::new();
        let log = Arc::new(Mutex::new(Vec::new()));
        let capture = Capture {
            inner: crate::setup::persona_provider_seeded(&bound, PersonaUse::Converse, seed)?,
            overlay: Arc::clone(&self.overlay),
            max_tokens: self.max_tokens,
            requests: 1,
            cancel: cancel.clone(),
            log: Arc::clone(&log),
        };
        let (agent, _) =
            crate::setup::persona_agent(&bound, &self.pinned, Box::new(capture), &self.store)?;
        let mut agent = agent.with_clock(Arc::clone(&self.clock));
        // The same tools, described by their own words, that never run: the
        // first call cancels the run (`OnDivergence::Stop` with nothing
        // recorded), so a sample is one request and draws nothing.
        let names: Vec<String> = agent
            .registry()
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        let standins = replay_registry(
            &names,
            agent.registry(),
            None,
            &[],
            Vec::new(),
            OnDivergence::Stop,
            cancel.clone(),
        )?;
        if mecha_core::surface::fingerprint(&standins.specs()) != self.surface {
            bail!("the stand-in tools do not describe themselves as the real ones do");
        }
        *agent.registry_mut() = standins;
        let branch = &self.branch;
        let cx = mecha_core::persona::turn::context(
            &agent,
            mecha_core::persona::turn::Turn {
                workspace: self.work.clone(),
                scene: None,
                prompt_log: None,
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
            },
        )
        .with_cancel(cancel);
        let mut convo =
            mecha_core::agent::Conversation::resumed(branch.messages.clone(), branch.taint);
        let ran = agent.run_in(&cx, &mut convo, None).await;
        drop(held);
        let error = ran.err().map(|e| format!("{e:#}"));
        let log = log.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let sample = Sample::of(
            i,
            Some(seed),
            branch.line,
            &bound.model,
            &self.overlay,
            &log,
            error,
        );
        if log.is_empty() {
            bail!(
                "sample {i} sent no request: {}",
                sample.error.as_deref().unwrap_or("no error was given")
            );
        }
        Ok(sample)
    }
}

/// What a set of samples did, as counts: no words.
fn summarise(samples: &[Sample]) -> serde_json::Value {
    let mut stops: BTreeMap<String, usize> = BTreeMap::new();
    let mut tools: BTreeMap<String, usize> = BTreeMap::new();
    let mut digests: BTreeMap<String, usize> = BTreeMap::new();
    let (mut no_call, mut unparsed, mut errors) = (0, 0, 0);
    for s in samples {
        if s.error.is_some() {
            errors += 1;
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
    let output: Vec<u64> = samples.iter().map(|s| s.output_tokens).collect();
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
