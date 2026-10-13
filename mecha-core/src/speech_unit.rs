//! The speech server from nothing (`docs/FEATURES-DESIGN.md` §10.6, step 7f):
//! Breeze TTS 2 on qwentts.cpp — the engine on `:8886` and the OpenAI-shaped
//! adapter the voice worker speaks to on `:8887`, its `MECHA_VOICE_TTS`
//! default.
//!
//! **The engine is built here, from a pinned commit.** The Breeze port lives
//! on the `breeze` branch of `github.com/ljchang/qwentts.cpp` (published
//! 2026-10-11, the owner's ruling); there is no release to fetch. The source
//! is fetched by its hash and checked against it, and built — for CUDA with
//! `$ORIGIN` RUNPATH on Linux, copied flat with the toolkit's runtime beside
//! it, or for Metal with `@loader_path` on Apple silicon (§12.2) —
//! `engine::build_from_source`'s recipe, through its helpers. The build must
//! report the pinned commit once its source tree is gone, and on Linux pass
//! `engine::self_contained`.
//!
//! **The weights are published, so they are pins.** The converted GGUF and
//! the codec are the `tts` slot's sources (`recommend::SLOTS`), on
//! `mecha-factory/mecha-speech-gguf`; the default voice (VCTK p297, CC BY
//! 4.0) is [`VOICE`]. All are fetched into the hub by sha256 and linked.
//! **Breeze TTS 2's weights are research and non-commercial only**: the
//! install says so before fetching them (owner's ruling, 2026-10-11: fetch,
//! with notices).
//!
//! **The adapter is what mecha wrote.** `breeze_server.py` and the two
//! modules it imports ship in the binary, held equal to `scripts/voice/` by a
//! test, and run from an environment built from a hash lock.
//!
//! **The check is a spoken sentence.** The install asks the adapter, which
//! asks the engine, to say a sentence in the default voice and reads back a
//! WAV of plausible length — the whole path a call takes, not a process
//! that started. Linux with an NVIDIA GPU, or Apple silicon: on a CPU the
//! engine runs slower than real time.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::engine::{Target, Toolchain};
use crate::install::Say;
use crate::llama_units::write_owned;
use crate::recommend::HubFile;
use crate::sidecar::{Machinery, Manifest};

/// The sidecar this installs (`sidecar::SIDECARS`).
pub const ID: &str = "speech";
/// The Breeze port's repository and the commit built from it.
const FORK: &str = "https://github.com/ljchang/qwentts.cpp";
pub const COMMIT: &str = "5b0b4cd17b5c89666afe3a71e95107982753aed3";
/// Where the converted weights and the default voice are published.
pub const WEIGHTS_REPO: &str = "mecha-factory/mecha-speech-gguf";
pub const WEIGHTS_REVISION: &str = "db42593e9653ca9d38f2b3ee2621803cbd797083";
/// The default voice: one VCTK reading and its transcript.
pub const VOICE: [HubFile; 2] = [
    HubFile {
        path: "voices/default.wav",
        sha256: "edd632dc47fece3af134a00c572dc4f51f2968e9a161ba43ac56d0090264a2e4",
        bytes: 391_558,
    },
    HubFile {
        path: "voices/default.txt",
        sha256: "77e8e13dc0804676d926513a1a65983a9013e3ac6a49cd2a647628f49710b084",
        bytes: 103,
    },
];
/// The adapter and the modules it imports, as `scripts/voice/` has them.
const FILES: [(&str, &str); 3] = [
    (
        "breeze_server.py",
        include_str!("../units/speech/breeze_server.py"),
    ),
    (
        "audio_stretch.py",
        include_str!("../units/speech/audio_stretch.py"),
    ),
    ("fragments.py", include_str!("../units/voice/fragments.py")),
];
const ENGINE_UNIT: &str = include_str!("../units/mecha-breeze-tts.service");
const ADAPTER_UNIT: &str = include_str!("../units/mecha-breeze-adapter.service");
const LOCK: &str = include_str!("../locks/speech-adapter-requirements.txt");
const PYTHON: &str = "3.12";
/// Written into the engine's tree once it is checked: the commit it is.
const COMMIT_STAMP: &str = ".mecha-commit";
/// The licence notice, said before the weights are fetched.
pub const NOTICE: &str = "Breeze TTS 2's weights are licensed for research and non-commercial use \
     only (the BreezeBlue Research and Non-Commercial License; see \
     https://huggingface.co/mecha-factory/mecha-speech-gguf). The default voice is VCTK's p297 \
     (CSTR, University of Edinburgh), CC BY 4.0.";

/// Where the units are installed and what ports they answer on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Naming {
    pub engine_unit: String,
    pub adapter_unit: String,
    pub engine_port: u16,
    pub adapter_port: u16,
}

impl Naming {
    /// What the hand install and the worker's `MECHA_VOICE_TTS` default name.
    pub fn shipped() -> Naming {
        Naming {
            engine_unit: "mecha-breeze-tts".into(),
            adapter_unit: "mecha-breeze-adapter".into(),
            engine_port: 8886,
            adapter_port: 8887,
        }
    }
}

/// The install's tree under a mecha home.
pub fn dir(mecha_home: &Path) -> PathBuf {
    mecha_home.join("sidecars").join(ID)
}

/// Where the installed adapter reads its voices (`VOICES_DIR` in its unit),
/// and so what `[voice] voices_dir` must name for recording and cloning to
/// reach it.
pub fn voices_dir(mecha_home: &Path) -> PathBuf {
    dir(mecha_home).join("voices")
}

/// What to say about a [`write_voices_dir`] outcome — one wording for the
/// installer and for `features enable voice`, which reaches an install that
/// already has the speech server and so never runs the installer (review of
/// #669). `None` when there is nothing to say.
pub fn voices_dir_said(
    outcome: &anyhow::Result<VoicesDir>,
    voices: &Path,
    config: &Path,
) -> Option<String> {
    Some(match outcome {
        Ok(VoicesDir::Wrote) => format!(
            "wrote [voice] voices_dir = \"{}\" into {}",
            voices.display(),
            config.display()
        ),
        Ok(VoicesDir::AlreadyNamed(dir)) if dir != voices => format!(
            "[voice] voices_dir names {}, but the installed speech server reads {} — recorded \
             voices will not reach it until they agree",
            dir.display(),
            voices.display()
        ),
        Ok(VoicesDir::AlreadyNamed(_)) => return None,
        Ok(VoicesDir::OwnersTable) => format!(
            "your [voice] table is left as it is — for recording and cloning to reach the speech \
             server, it needs voices_dir = \"{}\"",
            voices.display()
        ),
        Err(e) => format!(
            "[voice] voices_dir was not written ({e:#}) — add voices_dir = \"{}\" under [voice]",
            voices.display()
        ),
    })
}

/// What [`write_voices_dir`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoicesDir {
    /// No `[voice]` table and no voices directory named: one was written.
    Wrote,
    /// A voices directory is already named (`[voice]`, or the old `[web]`
    /// key still read for one release): left as it is.
    AlreadyNamed(PathBuf),
    /// The owner's own `[voice]` table names none: never edited, so the
    /// caller says the line to add.
    OwnersTable,
}

/// Name the installed adapter's voices directory in `[voice] voices_dir`
/// where nothing names one, so recording and cloning in the web app reach
/// the voices the speech server reads. Without it they said "not
/// configured" after a complete install (found 2026-10-12). A `[voice]`
/// table the owner wrote is never edited — `comfyui_unit::write_image_table`'s
/// rule — and the file's comments stay (`toml_edit`).
pub fn write_voices_dir(path: &Path, voices: &Path) -> Result<VoicesDir> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("{} is not valid TOML; nothing was changed", path.display()))?;
    // Expanded as the loader expands it (`Config::expand_home`), so a `~/`
    // form naming the right directory is not taken for another (review of
    // #669).
    let named = |table: &str| {
        doc.get(table)
            .and_then(|t| t.get("voices_dir"))
            .and_then(|v| v.as_str())
            .map(|v| match v.strip_prefix("~/") {
                Some(rest) => dirs::home_dir()
                    .map(|h| h.join(rest))
                    .unwrap_or_else(|| PathBuf::from(v)),
                None => PathBuf::from(v),
            })
    };
    if let Some(dir) = named("voice").or_else(|| named("web")) {
        return Ok(VoicesDir::AlreadyNamed(dir));
    }
    if doc.contains_key("voice") {
        return Ok(VoicesDir::OwnersTable);
    }
    let mut table = toml_edit::Table::new();
    table.insert("voices_dir", toml_edit::value(voices.display().to_string()));
    doc.insert("voice", toml_edit::Item::Table(table));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = path.with_extension("toml.voice-tmp");
    std::fs::write(&tmp, doc.to_string()).with_context(|| format!("writing {}", tmp.display()))?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(VoicesDir::Wrote)
}

/// The slot's pinned files — the model and the codec — for this machine.
fn weights(
    machine: &crate::recommend::Machine,
) -> Result<Vec<(&'static str, &'static str, &'static HubFile)>> {
    let slot = crate::recommend::SLOTS
        .iter()
        .find(|s| s.id == "tts")
        .context("no tts slot")?;
    let (row, _) = crate::recommend::row_for(slot, machine)
        .context("no speech model is recommended at this machine's tier")?;
    let mut out = Vec::new();
    for source in row.sources {
        let crate::recommend::Source::HuggingFace {
            repo,
            revision,
            files,
        } = source
        else {
            bail!("the speech model is not a Hugging Face file");
        };
        out.extend(files.iter().map(|f| (*repo, *revision, f)));
    }
    Ok(out)
}

/// The file names the engine unit loads: the model and the codec.
fn model_and_codec(
    weights: &[(&'static str, &'static str, &'static HubFile)],
) -> Result<(&'static str, &'static str)> {
    let name = |f: &'static HubFile| f.path.rsplit('/').next().unwrap_or(f.path);
    let codec = weights
        .iter()
        .map(|(_, _, f)| name(f))
        .find(|n| n.starts_with("qwen-tokenizer"))
        .context("the speech slot pins no codec")?;
    let model = weights
        .iter()
        .map(|(_, _, f)| name(f))
        .find(|n| !n.starts_with("qwen-tokenizer"))
        .context("the speech slot pins no model")?;
    Ok((model, codec))
}

/// Copy `from` to `to` as a new file stamped now — never with `from`'s times,
/// which `std::fs::copy` keeps on macOS.
fn copy_fresh(from: &Path, to: &Path) -> Result<()> {
    let bytes = std::fs::read(from).with_context(|| format!("reading {}", from.display()))?;
    std::fs::write(to, bytes).with_context(|| format!("writing {}", to.display()))
}

/// The units as installed.
fn render(text: &str, dir: &Path, naming: &Naming, model: &str, codec: &str) -> String {
    text.replace("@DIR@", &dir.display().to_string())
        .replace("@ENGINE_UNIT@", &naming.engine_unit)
        .replace("@ENGINE_PORT@", &naming.engine_port.to_string())
        .replace("@ADAPTER_PORT@", &naming.adapter_port.to_string())
        .replace("@MODEL@", model)
        .replace("@CODEC@", codec)
        .replace(
            "@MODEL_NAME@",
            &model.trim_end_matches(".gguf").to_ascii_lowercase(),
        )
}

/// Whether this machine can build and run the engine, or every reason not —
/// read from the OS and the driver, the toolchain asked only when those
/// pass.
pub fn fits(os: &str, nvidia: crate::engine::Nvidia) -> std::result::Result<Toolchain, String> {
    fits_with(os, std::env::consts::ARCH, nvidia, |target| {
        crate::engine::toolchain_missing(target)
    })
}

/// [`fits`] over what was read, so every row is a test.
fn fits_with(
    os: &str,
    arch: &str,
    nvidia: crate::engine::Nvidia,
    toolchain: impl FnOnce(Target) -> std::result::Result<Toolchain, Vec<String>>,
) -> std::result::Result<Toolchain, String> {
    use crate::engine::Nvidia;
    let missing = |missing: Vec<String>| {
        // In this module's own words: the engine has no releases, and the
        // pin that matters is the fork's commit, not llama.cpp's (review of
        // #649).
        format!(
            "the speech engine is built on this machine from its pinned commit ({}), and that \
             needs {}, which this machine does not have",
            &COMMIT[..7],
            missing.join(", ")
        )
    };
    match os {
        "linux" => {}
        // Metal on Apple silicon: measured at RTF 0.36 on an M5 Max
        // (2026-10-13, §12.1). No CUDA, so the toolchain is the CPU build's.
        "macos" if arch == "aarch64" => return toolchain(Target::BuiltCpu).map_err(missing),
        "macos" => {
            return Err(
                "the speech engine runs on Metal, which an Intel Mac's GPU does not run fast \
                 enough to speak in real time"
                    .into(),
            )
        }
        _ => {
            return Err(
                "the speech server installs as systemd user units or launchd agents, and this \
                 system has neither — start it by hand"
                    .into(),
            )
        }
    }
    match nvidia {
        Nvidia::Driver(..) => {}
        // A device is there and could not be read: said as that, never as
        // no GPU — unknown is not absent (review of #649).
        Nvidia::Unread => {
            return Err(
                "this machine shows an NVIDIA device, but `nvidia-smi` could not be read — fix \
                 it and run this again"
                    .into(),
            )
        }
        Nvidia::None => {
            return Err(
                "the speech engine needs an NVIDIA GPU: on the CPU it speaks slower than real \
                 time"
                    .into(),
            )
        }
    }
    toolchain(Target::BuiltCuda).map_err(missing)
}

pub async fn install(
    m: &Machinery,
    naming: &Naming,
    machine: &crate::recommend::Machine,
    hub: &Path,
    say: Say<'_>,
) -> Result<()> {
    let tc =
        fits(std::env::consts::OS, crate::engine::read_nvidia()).map_err(anyhow::Error::msg)?;
    let home = &m.mecha_home;
    let dir = dir(home);
    if someone_elses(m, naming)? {
        bail!(
            "something already answers on :{} or :{} and mecha did not install it — nothing is \
             installed over it; `mecha features enable voice --no-install` switches voice on and \
             uses it as it is",
            naming.engine_port,
            naming.adapter_port
        );
    }
    let weights = weights(machine)?;
    let (model, codec) = model_and_codec(&weights)?;
    // Before anything is fetched or recorded (review of #679).
    crate::user_service::preflight(m)?;
    Manifest::begin(home, ID)?;
    Manifest::record(home, ID, &dir)?;
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }

    // --- the engine, built from its commit
    let engine = dir.join("engine");
    Manifest::record(home, ID, &engine)?;
    let built = std::fs::read_to_string(engine.join(COMMIT_STAMP))
        .is_ok_and(|s| s.trim() == COMMIT)
        && engine_health(&engine).is_ok();
    if !built {
        build_engine(&dir, &engine, &tc, say).await?;
    }

    // --- the weights and the voice, from the hub, linked where the units look
    say(NOTICE);
    let voice: Vec<(&str, &str, &HubFile)> = VOICE
        .iter()
        .map(|f| (WEIGHTS_REPO, WEIGHTS_REVISION, f))
        .collect();
    for (repo, revision, f) in weights.iter().copied().chain(voice) {
        if crate::fetch::cached(hub, repo, revision, f, false)? != crate::fetch::Cached::Verified {
            say(&format!(
                "fetching {repo}/{} ({:.0} MiB)",
                f.path,
                f.bytes as f64 / 1_048_576.0
            ));
        }
        let snap = crate::fetch::fetch_hub_file(hub, repo, revision, f, &mut |_| {}).await?;
        let name = f.path.rsplit('/').next().unwrap_or(f.path);
        let link = if f.path.starts_with("voices/") {
            dir.join("voices").join(name)
        } else {
            dir.join("models").join(name)
        };
        Manifest::record(home, ID, &link)?;
        if let Some(parent) = link.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _ = std::fs::remove_file(&link);
        if f.path.starts_with("voices/") {
            // Copied, in `VOICE`'s order — the clip, then its transcript.
            // The adapter trusts a transcript only when it is no older than
            // its clip, and a link reads the hub blob's time, which is
            // whenever the hub was filled: a hub `hf download` filled wrote
            // the transcript first, and the adapter sent the shipped voice
            // to speech to text, and (with STT up) overwrote its curated
            // transcript (found on the owner's Mac, 2026-10-13). Written
            // fresh, so its time is this install's: `fs::copy` on macOS
            // clones or `copyfile`s with the source's times, which would
            // carry the blob's over (review of #679). The clip is 391 KB.
            copy_fresh(&snap, &link)?;
        } else {
            #[cfg(unix)]
            std::os::unix::fs::symlink(&snap, &link)
                .with_context(|| format!("linking {}", link.display()))?;
        }
    }

    // --- the adapter
    crate::install::locked_env(
        m,
        ID,
        &dir,
        PYTHON,
        LOCK,
        "the speech adapter's FastAPI, httpx and numpy",
        say,
    )
    .await?;
    let adapter = dir.join("adapter");
    for (name, text) in FILES {
        write_owned(home, ID, &adapter.join(name), text, 0o644)?;
    }

    // --- the units
    let r = |t: &str| render(t, &dir, naming, model, codec);
    crate::user_service::write(m, ID, &naming.engine_unit, &r(ENGINE_UNIT))?;
    crate::user_service::write(m, ID, &naming.adapter_unit, &r(ADAPTER_UNIT))?;
    // The engine first: the adapter is ordered after it (`After=` under
    // systemd; launchd loads them in this order).
    let names = [naming.engine_unit.as_str(), naming.adapter_unit.as_str()];
    // A restart, so a reinstall reaches running servers; a failed restart or
    // check stops both before the install says so (#639's pattern).
    let checked = match crate::user_service::restart(m, &names, &mut *say) {
        Ok(()) => {
            say(&format!(
                "starting the engine on :{} and the adapter on :{}, and speaking a sentence \
                 through both",
                naming.engine_port, naming.adapter_port
            ));
            check(naming.adapter_port).await
        }
        Err(e) => Err(e),
    };
    if let Err(e) = checked {
        let stopped = match crate::user_service::stop(&names) {
            Ok(()) => format!("{} were stopped and disabled", names.join(" and ")),
            Err(stop) => format!(
                "{} could NOT be stopped ({stop:#}) — they may still be listening; run `{}`",
                names.join(" and "),
                crate::user_service::stop_by_hand(&names)
            ),
        };
        return Err(e.context(stopped));
    }
    Manifest::finish(home, ID)
}

/// Build the pinned commit into `engine`: fetched by hash, its ggml
/// submodule with it, configured for CUDA with `$ORIGIN` RUNPATH, the
/// server and its libraries copied flat with the toolkit's runtime beside
/// them, checked, stamped and swapped in whole. The source tree goes either
/// way.
async fn build_engine(dir: &Path, engine: &Path, tc: &Toolchain, say: Say<'_>) -> Result<()> {
    let src = dir.join("engine.src");
    let part = dir.join("engine.part");
    for p in [&src, &part] {
        let _ = std::fs::remove_dir_all(p);
    }
    std::fs::create_dir_all(&src)?;
    std::fs::create_dir_all(&part)?;
    let built = build_into(&src, &part, tc, say).await;
    let _ = std::fs::remove_dir_all(&src);
    // Checked once the source tree is gone: with `build/` still there, a
    // library named by an absolute path into it would load and pass (review
    // of #679), so the check would prove nothing about `@loader_path` or
    // `$ORIGIN`.
    let built = built.and_then(|()| {
        engine_health(&part)?;
        std::fs::write(part.join(COMMIT_STAMP), COMMIT)?;
        Ok(())
    });
    if let Err(e) = built {
        let _ = std::fs::remove_dir_all(&part);
        return Err(e);
    }
    if engine.exists() {
        std::fs::remove_dir_all(engine)?;
    }
    std::fs::rename(&part, engine)?;
    Ok(())
}

async fn build_into(src: &Path, part: &Path, tc: &Toolchain, say: Say<'_>) -> Result<()> {
    use crate::engine::step;
    let git = |args: &[&str]| {
        let mut c = std::process::Command::new(&tc.git);
        c.arg("-C")
            .arg(src)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0");
        c
    };
    say(&format!(
        "fetching the speech engine's source at {}",
        &COMMIT[..7]
    ));
    step(git(&["init", "--quiet"]), "git init").await?;
    step(
        git(&["fetch", "--quiet", "--depth", "1", FORK, COMMIT]),
        "fetching the speech engine's source",
    )
    .await?;
    step(
        git(&[
            "-c",
            "advice.detachedHead=false",
            "checkout",
            "--quiet",
            "FETCH_HEAD",
        ]),
        "checking out the speech engine's source",
    )
    .await?;
    let head = step(git(&["rev-parse", "HEAD"]), "reading the commit").await?;
    if head.trim() != COMMIT {
        bail!(
            "the speech engine's source is commit {}, not the pinned {COMMIT}",
            head.trim()
        );
    }
    // ggml at the commit the fork names, which the submodule records.
    step(
        git(&[
            "submodule",
            "update",
            "--quiet",
            "--init",
            "--depth",
            "1",
            "ggml",
        ]),
        "fetching ggml for the speech engine",
    )
    .await?;

    // CUDA on Linux; Metal on a Mac, where `@loader_path` is `$ORIGIN`'s
    // spelling and there is no toolkit runtime to carry (§12.2, built and
    // measured on the owner's Mac 2026-10-13).
    let macos = cfg!(target_os = "macos");
    let cuda = if macos {
        None
    } else {
        Some(
            tc.cuda
                .as_ref()
                .context("the speech engine is built for CUDA, and no nvcc was found")?,
        )
    };
    let build = src.join("build");
    let mut c = std::process::Command::new(&tc.cmake);
    c.arg("-S").arg(src).arg("-B").arg(&build);
    if let Some(ninja) = &tc.ninja {
        c.args(["-G", "Ninja"])
            .arg(format!("-DCMAKE_MAKE_PROGRAM={}", ninja.display()));
    }
    c.args([
        "-DCMAKE_BUILD_TYPE=Release",
        "-DCMAKE_BUILD_WITH_INSTALL_RPATH=ON",
    ]);
    match cuda {
        Some((nvcc, arch)) => {
            c.args(["-DCMAKE_INSTALL_RPATH=$ORIGIN", "-DGGML_CUDA=ON"])
                .arg(format!("-DCMAKE_CUDA_COMPILER={}", nvcc.display()))
                .arg(format!("-DCMAKE_CUDA_ARCHITECTURES={arch}"));
        }
        None => {
            c.args(["-DCMAKE_INSTALL_RPATH=@loader_path", "-DGGML_METAL=ON"]);
        }
    }
    say("configuring the speech engine's build");
    step(c, "configuring the speech engine (cmake)").await?;
    let available = crate::system::Reader::new()
        .now(crate::system::Measurement::MemoryAvailable)
        .value()
        .map(|b| b as u64);
    let cpus = std::thread::available_parallelism().map_or(1, |n| n.get());
    let jobs = crate::engine::build_jobs(cpus, available);
    say(&format!(
        "building the speech engine with {jobs} jobs — {}",
        if macos {
            "under a minute on Apple silicon"
        } else {
            "about seven minutes on a GB10"
        }
    ));
    let mut c = std::process::Command::new(&tc.cmake);
    c.arg("--build")
        .arg(&build)
        .args(["--config", "Release", "--target", "tts-server", "-j"])
        .arg(jobs.to_string());
    step(c, "building the speech engine").await?;
    // The server and its libraries sit at the top of `build/`, nothing else
    // of it is wanted: the rest is the build's own.
    for entry in std::fs::read_dir(&build)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let library = name.starts_with("lib") && (name.contains(".so") || name.ends_with(".dylib"));
        if !(name == "tts-server" || library) {
            continue;
        }
        let dest = part.join(&name);
        if entry.file_type()?.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::read_link(entry.path())?, &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)
                .with_context(|| format!("copying {}", entry.path().display()))?;
        }
    }
    if let Some((nvcc, _)) = cuda {
        let dirs = crate::engine::toolkit_dirs(nvcc, &build);
        let copied = crate::engine::copy_cuda_runtime(&dirs, part)?;
        if copied.is_empty() {
            bail!(
                "the speech engine links no toolkit library found in {} — check where your \
                 toolkit's libcudart is",
                dirs.iter()
                    .map(|d| d.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        say(&format!(
            "copied the CUDA toolkit's {} beside it",
            copied.join(", ")
        ));
    }
    Ok(())
}

/// The engine in `dir` is self-contained and reports the pinned commit:
/// `tts-server --help` opens with `qwentts.cpp <short commit> (<date>)`.
fn engine_health(dir: &Path) -> Result<()> {
    let server = dir.join("tts-server");
    if !server.is_file() {
        bail!("{} is not there", server.display());
    }
    // The ELF check; a Mac's Mach-O libraries are found by `@loader_path`,
    // which the run below proves by starting at all (as `engine::health`
    // skips it there).
    if cfg!(target_os = "linux") {
        crate::engine::self_contained(dir)?;
    }
    let out = std::process::Command::new(&server)
        .arg("--help")
        .env_remove("LD_LIBRARY_PATH")
        .env_remove("DYLD_LIBRARY_PATH")
        .output()
        .with_context(|| format!("starting {}", server.display()))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if !reports_commit(&text, COMMIT) {
        bail!(
            "{} reports `{}`, not commit {}",
            server.display(),
            text.lines().next().unwrap_or("").trim(),
            &COMMIT[..7]
        );
    }
    Ok(())
}

/// `qwentts.cpp 5b0b4cd (2026-10-11)` names `commit` when its short hash is
/// a prefix of it, seven characters or more.
fn reports_commit(text: &str, commit: &str) -> bool {
    text.lines()
        .find_map(|l| l.trim().strip_prefix("qwentts.cpp "))
        .and_then(|rest| rest.split_whitespace().next())
        .is_some_and(|short| short.len() >= 7 && commit.starts_with(short))
}

/// The adapter answers and speaks: a sentence in the default voice comes
/// back as a WAV of at least a second. Retried while the engine loads.
async fn check(adapter_port: u16) -> Result<()> {
    let base = format!("http://127.0.0.1:{adapter_port}");
    // Loopback, never through a proxy (`llama_units::check`'s reason).
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let deadline = std::time::Instant::now() + Duration::from_secs(300);
    loop {
        match client.get(format!("{base}/health")).send().await {
            Ok(r) if r.status().is_success() => break,
            Ok(r) if std::time::Instant::now() > deadline => {
                bail!(
                    "the speech adapter on :{adapter_port} answered {}",
                    r.status()
                )
            }
            Err(e) if std::time::Instant::now() > deadline => {
                return Err(e).with_context(|| {
                    format!(
                        "the speech adapter on :{adapter_port} did not answer within five \
                         minutes — {}",
                        crate::user_service::logs_anywhere()
                    )
                })
            }
            _ => tokio::time::sleep(Duration::from_secs(2)).await,
        }
    }
    // The engine loads its model on start; the first sentence may wait on
    // it, so it is asked until the deadline rather than once.
    let body = serde_json::json!({
        "input": "The lighthouse keeper counted seven boats before noon.",
        "voice": "default",
        "response_format": "wav",
    });
    loop {
        let r = client
            .post(format!("{base}/v1/audio/speech"))
            .json(&body)
            .send()
            .await;
        match r {
            Ok(r) if r.status().is_success() => {
                let wav = r.bytes().await?;
                let seconds =
                    wav_seconds(&wav).context("the speech adapter's answer is not a WAV")?;
                if seconds < 1.0 {
                    bail!(
                        "the speech adapter answered a sentence with {seconds:.2} s of audio — \
                         not speech"
                    );
                }
                return Ok(());
            }
            Ok(r) if std::time::Instant::now() > deadline => bail!(
                "the speech adapter on :{adapter_port} answered {} to a sentence: {}",
                r.status(),
                r.text().await.unwrap_or_default().trim()
            ),
            Err(e) if std::time::Instant::now() > deadline => {
                return Err(e).context("asking the speech adapter for a sentence")
            }
            _ => tokio::time::sleep(Duration::from_secs(3)).await,
        }
    }
}

/// A 16-bit PCM WAV's length in seconds, from its header: `None` for
/// anything that is not one.
fn wav_seconds(wav: &[u8]) -> Option<f64> {
    if wav.len() < 44 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let u16_at = |i: usize| u16::from_le_bytes([wav[i], wav[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([wav[i], wav[i + 1], wav[i + 2], wav[i + 3]]);
    // Walk the chunks: `fmt ` gives the rate and frame size, `data` its length.
    let (mut i, mut rate, mut frame) = (12usize, 0u32, 0u16);
    while i + 8 <= wav.len() {
        let id = &wav[i..i + 4];
        let len = u32_at(i + 4) as usize;
        if id == b"fmt " && i + 8 + 16 <= wav.len() {
            rate = u32_at(i + 12);
            frame = u16_at(i + 20);
        }
        if id == b"data" {
            if rate == 0 || frame == 0 {
                return None;
            }
            let bytes = len.min(wav.len().saturating_sub(i + 8));
            return Some(bytes as f64 / (rate as f64 * frame as f64));
        }
        i += 8 + len + (len & 1);
    }
    None
}

/// Whether a server answers on either port whose unit mecha did not write —
/// someone's own speech server, used as it is (`stt_unit::someone_elses`'s
/// twin).
pub fn someone_elses(m: &Machinery, naming: &Naming) -> Result<bool> {
    let manifest = Manifest::read(&m.mecha_home)?;
    // Paths read first, so an unreadable unit directory is an error, never
    // "someone else's server" (review of #679).
    let engine_file = crate::user_service::file(m, &naming.engine_unit)?;
    let adapter_file = crate::user_service::file(m, &naming.adapter_unit)?;
    let wrote = |unit: &str| {
        let path = if unit == naming.engine_unit {
            &engine_file
        } else {
            &adapter_file
        };
        manifest
            .entries
            .iter()
            .any(|e| e.sidecar == ID && e.wrote.contains(path))
    };
    let stranger = |unit: &str, port: u16| !wrote(unit) && crate::router_unit::port_answers(port);
    Ok(stranger(&naming.engine_unit, naming.engine_port)
        || stranger(&naming.adapter_unit, naming.adapter_port))
}

/// Stop and remove a server installed under `naming`, its tree included.
/// The weights stay in the hub.
pub fn remove(m: &Machinery, naming: &Naming) -> Result<()> {
    let home = &m.mecha_home;
    let services = [naming.adapter_unit.as_str(), naming.engine_unit.as_str()];
    for s in services {
        let _ = crate::user_service::stop(&[s]);
    }
    for s in services {
        let path = crate::user_service::file(m, s)?;
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("removing {}", path.display())),
        }
        Manifest::unrecord(home, ID, &path)?;
    }
    crate::user_service::after_remove()?;
    let mut recorded: Vec<PathBuf> = Manifest::read(home)?
        .entries
        .iter()
        .filter(|e| e.sidecar == ID)
        .flat_map(|e| e.wrote.clone())
        .collect();
    recorded.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for p in recorded {
        let gone = match std::fs::symlink_metadata(&p) {
            Ok(md) if md.is_dir() => std::fs::remove_dir_all(&p),
            Ok(_) => std::fs::remove_file(&p),
            Err(e) => Err(e),
        };
        match gone {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("removing {}", p.display())),
        }
        Manifest::unrecord(home, ID, &p)?;
    }
    let _ = std::fs::remove_dir_all(dir(home));
    Manifest::forget_if_empty(home, ID)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The installed adapter's voices directory is named where nothing names
    /// one; one already named (either key) and an owner's own `[voice]` table
    /// are left untouched, comments kept.
    #[test]
    fn the_voices_dir_is_named_only_where_nothing_names_one() {
        let root = std::env::temp_dir().join(format!("mecha-voices-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let voices = voices_dir(&root.join(".mecha"));
        let case = |name: &str, text: &str| {
            let p = root.join(name);
            std::fs::write(&p, text).unwrap();
            let got = write_voices_dir(&p, &voices).unwrap();
            (got, std::fs::read_to_string(&p).unwrap())
        };
        let (fresh, text) = case("fresh.toml", "# mine\ndefault_provider = \"local\"\n");
        let (named, named_text) = case("named.toml", "[voice]\nvoices_dir = \"/srv/v\"\n");
        let (legacy, _) = case("legacy.toml", "[web]\nvoices_dir = \"/srv/old\"\n");
        let (owned, owned_text) = case("owned.toml", "[voice]\nstt_url = \"http://x\"\n");
        let (tilde, _) = case("tilde.toml", "[voice]\nvoices_dir = \"~/v\"\n");
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(fresh, VoicesDir::Wrote);
        assert!(text.starts_with("# mine"), "{text}");
        let parsed: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(parsed["voice"]["voices_dir"].as_str(), voices.to_str());
        assert!(voices.ends_with("sidecars/speech/voices"));
        assert_eq!(named, VoicesDir::AlreadyNamed("/srv/v".into()));
        assert_eq!(named_text, "[voice]\nvoices_dir = \"/srv/v\"\n");
        assert_eq!(legacy, VoicesDir::AlreadyNamed("/srv/old".into()));
        assert_eq!(owned, VoicesDir::OwnersTable);
        // `~/` is expanded as the loader expands it.
        assert_eq!(
            tilde,
            VoicesDir::AlreadyNamed(dirs::home_dir().unwrap().join("v"))
        );
        assert_eq!(owned_text, "[voice]\nstt_url = \"http://x\"\n");
    }

    /// The adapter's unit reads the directory `voices_dir` names.
    #[test]
    fn the_adapter_reads_the_voices_dir_the_config_names() {
        assert!(ADAPTER_UNIT.contains("Environment=VOICES_DIR=@DIR@/voices"));
        assert_eq!(
            voices_dir(Path::new("/h/.mecha")),
            dir(Path::new("/h/.mecha")).join("voices")
        );
    }

    /// What runs is what the hand install runs, byte for byte.
    #[test]
    fn the_shipped_adapter_is_the_scripts_copy() {
        let scripts = [
            include_str!("../../scripts/voice/breeze_server.py"),
            include_str!("../../scripts/voice/audio_stretch.py"),
            include_str!("../../scripts/voice/fragments.py"),
        ];
        assert_eq!(FILES.len(), scripts.len());
        for ((name, shipped), script) in FILES.iter().zip(scripts) {
            assert_eq!(*shipped, script, "{name}");
        }
    }

    /// Every local module the adapter imports ships beside it.
    #[test]
    fn every_local_import_ships() {
        let shipped: Vec<&str> = FILES
            .iter()
            .map(|(n, _)| n.trim_end_matches(".py"))
            .collect();
        for (_, text) in FILES {
            for line in text.lines().map(str::trim_start) {
                let module = line
                    .strip_prefix("from ")
                    .or_else(|| line.strip_prefix("import "))
                    .and_then(|r| r.split([' ', ',', '.']).next());
                if let Some(m) = module {
                    let local = Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../scripts/voice")
                        .join(format!("{m}.py"));
                    if local.is_file() {
                        assert!(shipped.contains(&m), "{m} is imported and not shipped");
                    }
                }
            }
        }
    }

    #[test]
    fn the_lock_pins_every_file() {
        let lines: Vec<&str> = LOCK.lines().collect();
        let mut pins = 0;
        for (i, l) in lines.iter().enumerate() {
            if l.starts_with('#') || l.starts_with(' ') || l.is_empty() {
                continue;
            }
            pins += 1;
            assert!(
                lines
                    .get(i + 1)
                    .is_some_and(|n| n.trim_start().starts_with("--hash=sha256:")),
                "{l} has no hash"
            );
        }
        assert!(pins >= 15, "{pins}");
        for need in ["fastapi==", "httpx==", "numpy==", "uvicorn==", "pydantic=="] {
            assert!(LOCK.contains(need), "{need}");
        }
    }

    /// The slot pins the model and the codec on the published repository,
    /// and the voice sits beside them there.
    #[test]
    fn the_weights_are_the_slots_pins_on_the_published_repo() {
        let machine = crate::recommend::Machine::Unified {
            total_mb: 124_610,
            gpu_unread: false,
        };
        let w = weights(&machine).unwrap();
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w
            .iter()
            .all(|(repo, rev, _)| *repo == WEIGHTS_REPO && *rev == WEIGHTS_REVISION));
        let (model, codec) = model_and_codec(&w).unwrap();
        assert!(model.ends_with(".gguf") && codec.starts_with("qwen-tokenizer"));
        assert_eq!(WEIGHTS_REVISION.len(), 40);
        assert_eq!(COMMIT.len(), 40);
        for f in VOICE {
            assert!(f.path.starts_with("voices/") && f.sha256.len() == 64);
        }
    }

    #[test]
    fn the_units_render_this_home_and_ports() {
        let dir = Path::new("/srv/owner/.mecha/sidecars/speech");
        let n = Naming {
            engine_unit: "e".into(),
            adapter_unit: "a".into(),
            engine_port: 9886,
            adapter_port: 9887,
        };
        let engine = render(ENGINE_UNIT, dir, &n, "m-Q5_K_M.gguf", "c.gguf");
        let adapter = render(ADAPTER_UNIT, dir, &n, "m-Q5_K_M.gguf", "c.gguf");
        for t in [&engine, &adapter] {
            assert!(!t.contains('@'), "{t}");
        }
        assert!(engine.contains(
            "ExecStart=/srv/owner/.mecha/sidecars/speech/engine/tts-server --model \
             /srv/owner/.mecha/sidecars/speech/models/m-Q5_K_M.gguf --codec \
             /srv/owner/.mecha/sidecars/speech/models/c.gguf --host 127.0.0.1 --port 9886"
        ));
        assert!(adapter.contains("Environment=BREEZE_TTS_URL=http://127.0.0.1:9886"));
        assert!(adapter.contains("Environment=BREEZE_PORT=9887"));
        assert!(adapter.contains("Environment=VOICES_DIR=/srv/owner/.mecha/sidecars/speech/voices"));
        assert!(adapter.contains("Environment=BREEZE_MODEL_NAME=m-q5_k_m"));
        assert!(adapter.contains("After=e.service"));
    }

    #[test]
    fn only_nvidia_linux_or_apple_silicon_with_the_toolchain_fits() {
        use crate::engine::Nvidia as N;
        let tc = |_| {
            Ok(Toolchain {
                git: "/bin/git".into(),
                cmake: "/bin/cmake".into(),
                ninja: None,
                cuda: Some(("/bin/nvcc".into(), "121".into())),
            })
        };
        let lacks = |_| {
            Err(vec![
                "nvcc (the CUDA toolkit, matching this driver)".to_string()
            ])
        };
        assert!(fits_with("linux", "aarch64", N::Driver(570, 1), tc).is_ok());
        let e = fits_with("linux", "aarch64", N::Driver(580, 0), lacks).unwrap_err();
        assert!(
            e.contains("speech engine") && e.contains("nvcc") && e.contains(&COMMIT[..7]),
            "{e}"
        );
        assert!(!e.contains("llama.cpp") && !e.contains("release"), "{e}");
        assert!(fits_with("linux", "aarch64", N::Unread, tc)
            .unwrap_err()
            .contains("nvidia-smi"));
        assert!(fits_with("linux", "aarch64", N::None, tc)
            .unwrap_err()
            .contains("NVIDIA GPU"));
        // Apple silicon builds for Metal: no NVIDIA asked, the CPU build's
        // toolchain asked for.
        let mut asked = None;
        assert!(fits_with("macos", "aarch64", N::None, |t| {
            asked = Some(t);
            tc(t)
        })
        .is_ok());
        assert_eq!(asked, Some(Target::BuiltCpu));
        assert!(
            fits_with("macos", "aarch64", N::None, |_| Err(vec!["cmake".into()]))
                .unwrap_err()
                .contains("cmake")
        );
        assert!(fits_with("macos", "x86_64", N::None, tc)
            .unwrap_err()
            .contains("Intel"));
        assert!(fits_with("freebsd", "x86_64", N::None, tc)
            .unwrap_err()
            .contains("neither"));
    }

    #[test]
    fn the_engine_reports_its_commit() {
        let c = "5b0b4cd1234567890abcdef1234567890abcdef1";
        assert!(reports_commit(
            "qwentts.cpp 5b0b4cd (2026-10-11)\n\nUsage: …",
            c
        ));
        assert!(!reports_commit("qwentts.cpp 6fae929 (2026-09-28)", c));
        assert!(!reports_commit("qwentts.cpp unknown (unknown)", c));
        assert!(!reports_commit("", c));
    }

    #[test]
    fn a_wav_is_read_for_its_length() {
        let mut wav = Vec::new();
        let pcm = vec![0u8; 24_000 * 2 * 3 / 2];
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&24_000u32.to_le_bytes());
        wav.extend_from_slice(&48_000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        wav.extend_from_slice(&pcm);
        assert_eq!(wav_seconds(&wav), Some(1.5));
        assert_eq!(wav_seconds(b"not a wav"), None);
    }

    #[test]
    fn the_shipped_ports_are_the_workers_defaults() {
        let worker = include_str!("../units/voice/worker.py");
        let n = Naming::shipped();
        assert!(worker.contains(&format!("http://127.0.0.1:{}/v1", n.adapter_port)));
        let adapter = include_str!("../units/speech/breeze_server.py");
        assert!(adapter.contains(&format!("http://127.0.0.1:{}", n.engine_port)));
    }

    /// The real thing: build the engine from its commit, fetch the weights
    /// and the voice, build the adapter's environment, install both units
    /// beside the live ones on free ports under a scratch mecha home, speak a
    /// sentence through them, and remove them. ~8 minutes on a GB10, most of
    /// it the CUDA build; gate it on the memory floor and no live call.
    /// `cargo test -p mecha-core -- --ignored a_speech_server_installs`
    #[tokio::test]
    #[ignore]
    async fn a_speech_server_installs_and_speaks_beside_the_live_one() {
        let free = || {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let pid = std::process::id();
        let naming = Naming {
            engine_unit: format!("mecha-test-breeze-tts-{pid}"),
            adapter_unit: format!("mecha-test-breeze-adapter-{pid}"),
            engine_port: free(),
            adapter_port: free(),
        };
        let root = std::env::temp_dir().join(format!("mecha-7f-{}", uuid::Uuid::new_v4()));
        let mut m = Machinery::real().unwrap();
        m.mecha_home = root.join(".mecha");
        let machine = crate::recommend::Machine::read().unwrap();
        let hub = crate::fetch::hub_dir().unwrap();
        let mut said = Vec::new();
        let r = install(&m, &naming, &machine, &hub, &mut |s| {
            said.push(s.to_string())
        })
        .await;
        let stamped = std::fs::read_to_string(dir(&m.mecha_home).join("engine").join(COMMIT_STAMP));
        let cleaned = remove(&m, &naming);
        eprintln!("{said:#?}");
        let man = Manifest::read(&m.mecha_home);
        let _ = std::fs::remove_dir_all(&root);
        r.unwrap();
        cleaned.unwrap();
        assert_eq!(stamped.unwrap(), COMMIT);
        assert!(
            said.iter().any(|s| s.contains("non-commercial")),
            "{said:?}"
        );
        let man = man.unwrap();
        assert!(
            !man.entries.iter().any(|e| e.sidecar == ID),
            "{:?}",
            man.entries
        );
    }

    /// The voice pair is stamped in install order, whatever times the hub's
    /// blobs carry: a transcript blob older than its clip — as `hf download`
    /// wrote them — still lands newer (review of #679: `fs::copy` kept the
    /// blobs' times on macOS).
    #[test]
    fn the_voice_pair_is_stamped_as_installed() {
        // The install copies `VOICE` in order, so the clip must come first
        // for its transcript to land newer (review of #679).
        assert!(VOICE[0].path.ends_with(".wav") && VOICE[1].path.ends_with(".txt"));
        let d = std::env::temp_dir().join(format!("mecha-voice-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let (wav, txt) = (d.join("blob-wav"), d.join("blob-txt"));
        std::fs::write(&wav, b"RIFF").unwrap();
        std::fs::write(&txt, b"words").unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        let older = old - std::time::Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(&wav)
            .unwrap()
            .set_modified(old)
            .unwrap();
        std::fs::File::options()
            .write(true)
            .open(&txt)
            .unwrap()
            .set_modified(older)
            .unwrap();
        let (to_wav, to_txt) = (d.join("default.wav"), d.join("default.txt"));
        copy_fresh(&wav, &to_wav).unwrap();
        copy_fresh(&txt, &to_txt).unwrap();
        let t = |p: &Path| std::fs::metadata(p).unwrap().modified().unwrap();
        assert!(t(&to_txt) >= t(&to_wav));
        assert!(t(&to_wav) > old, "the clip took the blob's time");
        assert_eq!(std::fs::read(&to_txt).unwrap(), b"words");
        let _ = std::fs::remove_dir_all(&d);
    }
}
