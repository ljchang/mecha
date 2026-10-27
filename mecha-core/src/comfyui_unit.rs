//! The image server from nothing (`docs/FEATURES-DESIGN.md` §10.6, step 7e):
//! ComfyUI at a pinned commit, its environment from a hash lock, the image
//! slot's pinned models, and one rendered unit on `[image] url`'s default,
//! `:8188`.
//!
//! **What runs is what mecha fetched.** The source is the pinned commit,
//! fetched by its hash with git and checked against it, its `.git` dropped;
//! the environment is torch for CUDA 13 and ComfyUI's requirements, every
//! file pinned by sha256 in one lock per architecture
//! (`locks/comfyui-cu130-*.txt`), torch by direct URL so no second index is
//! ever consulted. The models are the image slot's own pins
//! (`recommend::SLOTS`), fetched into the hub like every other model and
//! linked into ComfyUI's `models/` tree. The plan reads an installed
//! ComfyUI's models from the hub (`sidecar::plan`), so a cleared cache reads
//! as a download and the install is offered again — a dangling link alone
//! would read present, since the record's check is an `lstat`.
//!
//! **The check is the graph, not the process.** ComfyUI answering proves
//! little; the install asks `/object_info` for every node type
//! [`crate::imagegen`] sends, and for the three model files by the names
//! `[image]`'s defaults load. No picture is drawn — a load from cold takes
//! ~19 GiB of a unified pool the chat model shares, and the check must not
//! be the thing that starves it.
//!
//! **CUDA 13 on Linux, MPS on Apple silicon.** The Linux locks are torch's
//! cu130 builds, which need a 580 driver (`engine::CUDA13_DRIVER`'s reason);
//! the Mac's (`locks/comfyui-macos-arm64.txt`) is stock PyPI torch, which
//! carries MPS. Any other machine is told so before anything is fetched.
//! ComfyUI-GGUF is not installed: since #560 the default model is int8
//! safetensors through the core loader, and the GGUF node is needed only for
//! an owner's own `.gguf`.
//!
//! **A Mac draws with bf16 files** (the owner, 2026-10-13; FEATURES-DESIGN
//! §12.3): MPS has no int8 kernels, and the emulated int8 picture took ~128 s
//! against ~79 s for bf16. The image slot's Mac row
//! (`recommend::Slot::mac_rows`) and [`turbo_file`] name them, and
//! `[image]`'s defaults follow (`sampling::standard_model`). The text encoder
//! stays on the CPU there: its float8 scales have no MPS type, so the unit
//! never passes `--gpu-only`.
//!
//! **Idle memory goes back the way the hand install's does — on Linux.** A
//! loaded server holds ~13.6 GB until something unloads it, so the install
//! also writes `scripts/comfyui/comfyui-idle-reset` — byte for byte, a test holds
//! the copies equal — and a one-minute timer that runs it against this unit
//! and port (`docs/ARCHITECTURE.md` §Image generation). The timer starts
//! before the server, so a failure stops the install before anything runs;
//! a failed check stops both. It is named `<unit>-idle` so a
//! hand install's `mecha-comfyui-idle-reset` can sit beside it.
//!
//! **On a Mac, ComfyUI runs on demand** (the owner's ruling, 2026-10-14):
//! the script reads `/proc` and restarts through `systemctl`, and mecha's
//! own `unload_after_secs` dies with a `mecha run`, so a loaded server held
//! ~35 GiB until something else gave way. Instead the agent runs mecha's
//! on-demand proxy (`on_demand.rs`, the embeddings and OCR servers'), which
//! holds the public port, starts ComfyUI on [`mac_backend_port`] at the
//! first picture and stops it after fifteen idle minutes, returning all of
//! its memory ([`for_mac`], [`mac_launcher_text`]).
//!
//! The commit is one constant, [`COMMIT`], so a bump is that line and the
//! locks regenerated against it (their header says how).
//! Rendered like `stt_unit`'s, through `user_service` (a launchd agent read
//! from the same unit on a Mac), so a test installs one beside the live
//! server on a free port and removes it.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::install::Say;
use crate::llama_units::write_owned;
use crate::sidecar::{Machinery, Manifest};

/// The sidecar this installs (`sidecar::SIDECARS`).
pub const ID: &str = "comfyui";
/// The turbo distillation's repository and revision. A later revision than
/// the image slot's: the turbo file is not in that one.
pub const TURBO_REPO: &str = "Comfy-Org/Qwen-Image-2.1";
pub const TURBO_REVISION: &str = "df94239739eef7205973a145ffa6f441f7b64e84";
/// The turbo model, fetched on demand rather than with the slot (the owner,
/// 2026-10-11: until it has been tested more): `--turbo`, a persona whose
/// picture settings choose it, or an install that already links it.
pub const TURBO_FILE: crate::recommend::HubFile = crate::recommend::HubFile {
    path: "diffusion_models/qwen_image_2.1_turbo_int8_convrot.safetensors",
    sha256: "8144a0182964aa8403ad6dbc011db837304167d235161b9aabaa8532e967314c",
    bytes: 7_256_787_829,
};
/// The turbo model on a Mac: bf16, at the same revision (the owner,
/// 2026-10-13). ~16 s a picture warm on an M5 Max against the int8's ~24 s.
pub const TURBO_FILE_MAC: crate::recommend::HubFile = crate::recommend::HubFile {
    path: "diffusion_models/qwen_image_2.1_turbo_bf16.safetensors",
    sha256: "8b74237a7f505773e2c66726fa696444b4c704585c93a347cff9044937f7914a",
    bytes: 14_230_280_616,
};

/// The turbo file this system fetches on demand.
pub fn turbo_file() -> &'static crate::recommend::HubFile {
    turbo_file_on(std::env::consts::OS)
}

/// [`turbo_file`] on `os`.
pub fn turbo_file_on(os: &str) -> &'static crate::recommend::HubFile {
    if os == "macos" {
        &TURBO_FILE_MAC
    } else {
        &TURBO_FILE
    }
}
/// ComfyUI's commit: v0.37.0-31, what the hand-built server ran on
/// 2026-10-10 (`TextEncodeQwenImage21` is in it).
pub const COMMIT: &str = "88ab4a06566454ad89db8f0bedb970d6c08cd1b7";
const REPO: &str = "https://github.com/comfyanonymous/ComfyUI";
const UNIT: &str = include_str!("../units/mecha-comfyui.service");
/// `scripts/comfyui/comfyui-idle-reset`, and the timer and service that run it.
const IDLE_RESET: &str = include_str!("../units/mecha-comfyui-idle-reset");
const IDLE_SERVICE: &str = include_str!("../units/mecha-comfyui-idle.service");
const IDLE_TIMER: &str = include_str!("../units/mecha-comfyui-idle.timer");
const LOCK_AARCH64: &str = include_str!("../locks/comfyui-cu130-aarch64.txt");
const LOCK_X86_64: &str = include_str!("../locks/comfyui-cu130-x86_64.txt");
const LOCK_MACOS_ARM64: &str = include_str!("../locks/comfyui-macos-arm64.txt");
/// The Python the locks were compiled for.
const PYTHON: &str = "3.12";
/// Written into the source tree once it is checked: the commit it is.
const COMMIT_STAMP: &str = ".mecha-commit";

/// Every node type [`crate::imagegen`] sends but the GGUF loader, which only
/// an owner's own `.gguf` needs: the picture's graphs and the segmentation's
/// (`imagegen::sam_graph`), so an install on a ComfyUI with no SAM 3 node
/// fails its check rather than linking a checkpoint nothing can load. A test
/// builds the graphs and holds the two sets equal, both ways.
pub const NODES: [&str; 16] = [
    "CLIPLoader",
    "CLIPTextEncode",
    "CheckpointLoaderSimple",
    "EmptyLatentImage",
    "ImageToMask",
    "KSampler",
    "LoadImage",
    "MaskToImage",
    "PreviewImage",
    "SAM3_Detect",
    "SetLatentNoiseMask",
    "TextEncodeQwenImage21",
    "UNETLoader",
    "VAEDecode",
    "VAEEncode",
    "VAELoader",
];

/// Where the unit is installed and what port it answers on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Naming {
    /// The unit's name without `.service`.
    pub unit: String,
    pub port: u16,
}

impl Naming {
    /// The idle reset's service and timer, without their suffixes.
    pub fn idle(&self) -> String {
        format!("{}-idle", self.unit)
    }

    /// What `[image] url`'s default names.
    pub fn shipped() -> Naming {
        Naming {
            unit: "mecha-comfyui".into(),
            port: 8188,
        }
    }
}

/// The install's tree under a mecha home.
pub fn dir(mecha_home: &Path) -> PathBuf {
    mecha_home.join("sidecars").join(ID)
}

/// Where this install's ComfyUI looks for LoRAs: what an unset
/// `[image] loras_dir` means when mecha installed the server
/// (`loras::Folders`).
pub fn loras_dir(mecha_home: &Path) -> PathBuf {
    dir(mecha_home).join("ComfyUI").join("models").join("loras")
}

/// Whether mecha installed the image server here, from the install record
/// alone: a finished entry that wrote something. A store read with no port
/// probe, so `doctor` may ask it; a manifest that does not read is not an
/// install (`someone_elses` probes the port too, and is for the page).
pub fn installed_here(mecha_home: &Path) -> bool {
    Manifest::read(mecha_home).is_ok_and(|m| {
        m.entries
            .iter()
            .any(|e| e.sidecar == ID && !e.incomplete && !e.wrote.is_empty())
    })
}

/// What `[image] server_temp_dir` must name for this install: the unit's
/// `--temp-directory` with `temp` appended, as ComfyUI lays it out.
pub fn server_temp_dir(mecha_home: &Path) -> PathBuf {
    dir(mecha_home).join("tmp").join("temp")
}

/// The lock for a system and architecture, or `None` where none is pinned.
fn lock_for(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => Some(LOCK_MACOS_ARM64),
        ("macos", _) => None,
        (_, "aarch64") => Some(LOCK_AARCH64),
        (_, "x86_64") => Some(LOCK_X86_64),
        _ => None,
    }
}

/// What the environment downloads on a system and architecture, in bytes:
/// the lock's files as PyPI and download.pytorch.org size them, summed
/// 2026-10-11 (the Mac's 2026-10-13: no CUDA libraries, a fifth of Linux's).
/// Linux's lands as ~6.3 GiB in `venv/` (measured on the GB10).
pub fn env_bytes(os: &str, arch: &str) -> Option<u64> {
    match (os, arch) {
        ("macos", "aarch64") => Some(762_000_000),
        ("macos", _) => None,
        (_, "aarch64") => Some(3_848_000_000),
        (_, "x86_64") => Some(3_724_000_000),
        _ => None,
    }
}

/// Whether this machine can run the pinned environment, or why not — asked
/// before anything is fetched. A pure function of what was read.
pub fn fits(
    os: &str,
    arch: &str,
    nvidia: crate::engine::Nvidia,
) -> std::result::Result<(), String> {
    // The macOS version, read only on a Mac: off one there is nothing to ask.
    let macos = if os == "macos" && cfg!(target_os = "macos") {
        macos_major()
    } else {
        None
    };
    fits_with(os, arch, nvidia, macos)
}

/// The oldest macOS the Mac lock's torch runs on: its wheels are tagged
/// `macosx_14_0_arm64`, and on an older system `uv` finds no wheel and fails
/// mid-install with its own words (review of #681).
pub const MACOS_FLOOR: u32 = 14;

/// The running macOS's major version, from `sw_vers -productVersion`.
fn macos_major() -> Option<u32> {
    let out = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .split('.')
        .next()?
        .parse()
        .ok()
}

/// [`fits`] over what was read, so every row is a test. `macos` is the
/// running macOS's major version, `None` where it was not read — refused on
/// a Mac, since unknown is never new enough.
fn fits_with(
    os: &str,
    arch: &str,
    nvidia: crate::engine::Nvidia,
    macos: Option<u32>,
) -> std::result::Result<(), String> {
    use crate::engine::Nvidia;
    match os {
        "linux" => {}
        // MPS, from PyPI's torch: no driver to ask (§12.3), but a floor.
        "macos" if arch == "aarch64" => {
            return match macos {
                Some(v) if v >= MACOS_FLOOR => Ok(()),
                Some(v) => Err(format!(
                    "this Mac runs macOS {v}; the pinned torch needs macOS {MACOS_FLOOR} or \
                     newer — update macOS to install image generation"
                )),
                None => Err(format!(
                    "this Mac's macOS version could not be read (`sw_vers -productVersion`), and \
                     the pinned torch needs macOS {MACOS_FLOOR} or newer"
                )),
            }
        }
        "macos" => {
            return Err(
                "image generation on a Mac needs Apple silicon: the pinned torch draws on MPS, \
                 and an Intel Mac's GPU has none"
                    .into(),
            )
        }
        _ => {
            return Err(
                "the image server installs as a systemd user unit or a launchd agent, and this \
                 system has neither — start ComfyUI by hand"
                    .into(),
            )
        }
    }
    if lock_for(os, arch).is_none() {
        return Err(format!(
            "mecha pins ComfyUI's environment for aarch64 and x86_64, not {arch}"
        ));
    }
    match nvidia {
        // The engine's floor, one constant: a bump moves both (review of #645).
        Nvidia::Driver(ma, mi) if (ma, mi) >= crate::engine::CUDA13_DRIVER => Ok(()),
        Nvidia::Driver(ma, mi) => Err(format!(
            "the NVIDIA driver is {ma}.{mi}; the pinned torch is built for CUDA 13, which needs \
             {}.{} or newer — update the driver",
            crate::engine::CUDA13_DRIVER.0,
            crate::engine::CUDA13_DRIVER.1
        )),
        Nvidia::Unread => Err(
            "this machine shows an NVIDIA device, but `nvidia-smi` could not be read — fix it \
             and run this again"
                .into(),
        ),
        Nvidia::None => Err(
            "image generation needs an NVIDIA GPU here: the pinned torch is CUDA's, and a picture \
             on the CPU would take most of an hour"
                .into(),
        ),
    }
}

/// The unit as installed: absolute paths under this mecha home, this port.
fn render(dir: &Path, naming: &Naming) -> String {
    UNIT.replace("@COMFY@", &dir.join("ComfyUI").display().to_string())
        .replace("@VENV@", &dir.join("venv").display().to_string())
        .replace("@TEMP@", &dir.join("tmp").display().to_string())
        .replace("@PORT@", &naming.port.to_string())
}

/// On a Mac ComfyUI is started on demand and stopped when idle (the owner's
/// ruling, 2026-10-14): mecha's proxy (`on_demand.rs`) holds the public port
/// and runs [`mac_launcher_text`] on this backend port at the first picture.
/// Nothing else frees a Mac's ComfyUI — `[image] unload_after_secs` lives in
/// the drawing process, which a `mecha run` ends at once, and Linux's idle
/// reset reads `/proc` and calls `systemctl` — and a loaded server held
/// ~35 GiB of the owner's Mac until something else gave way (measured
/// 2026-10-14: the memory level stayed at 41 % from 84 % for 15 minutes).
pub fn mac_backend_port(public: u16) -> u16 {
    // Never the public port itself: one too high to add to is moved down.
    public
        .checked_add(10_000)
        .unwrap_or(public.wrapping_sub(10_000))
}

/// How long a Mac's ComfyUI may sit with no open connection before the
/// proxy stops it: `[image] unload_after_secs`' default, and Linux's idle
/// reset's fifteen minutes.
const MAC_IDLE_SECS: u64 = 900;

/// The launcher the proxy runs on a Mac: the unit's own command, rendered on
/// the backend port, from the unit's own directory — one command for both
/// systems, as `user_service` keeps them.
fn mac_launcher_text(backend_unit: &str) -> Result<String> {
    let line = |key: &str| {
        backend_unit
            .lines()
            .map(str::trim)
            .find_map(|l| l.strip_prefix(key))
            .with_context(|| format!("the ComfyUI unit has no {key}"))
    };
    let exec = line("ExecStart=")?;
    let dir = line("WorkingDirectory=")?;
    // The unit's `ExecStartPre` makes the temp directory on every start; on a
    // Mac ComfyUI starts at every first picture after an idle stop, so the
    // launcher makes it (review of #689).
    let temp = line("ExecStartPre=")?
        .strip_prefix("/bin/mkdir -p ")
        .context("the ComfyUI unit's ExecStartPre is not its temp directory's mkdir")?;
    Ok(format!(
        "#!/bin/sh\n\
         # Written by mecha: ComfyUI on its backend port, started by mecha's\n\
         # on-demand proxy at the first picture and stopped when idle.\n\
         mkdir -p '{temp}' || exit 1\n\
         cd '{dir}' || exit 1\n\
         exec {exec}\n"
    ))
}

/// The unit as a Mac's agent reads it: mecha's on-demand proxy in front of
/// [`mac_launcher_text`], its `PATH` the system's own (the Linux line names
/// `/usr/local/cuda/bin` for Triton's compiler, which a Mac has neither of —
/// review of #681), and the two lines an agent has no counterpart for
/// dropped: `ExecStartPre`'s temp directory, which the install makes on both
/// systems, and `TimeoutStartSec`, whose job the install's own check does.
/// `launchd::agent_from_unit` refuses any line it cannot carry, so these are
/// taken out here, by name, rather than ignored there (#679).
fn for_mac(unit: &str, mecha: &Path, launcher: &Path, port: u16) -> String {
    unit.lines()
        .filter(|l| {
            let l = l.trim_start();
            !l.starts_with("ExecStartPre=") && !l.starts_with("TimeoutStartSec=")
        })
        .map(|l| {
            let t = l.trim_start();
            if t.starts_with("Environment=PATH=") {
                format!("Environment=PATH={}", crate::launchd::SYSTEM_PATH)
            } else if t.starts_with("ExecStart=") {
                format!(
                    "ExecStart={} features on-demand --listen 127.0.0.1:{port} --backend {} \
                     --idle-secs {MAC_IDLE_SECS} --ready-path /system_stats --launcher {}",
                    mecha.display(),
                    mac_backend_port(port),
                    launcher.display()
                )
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// The idle reset's script under the install's tree.
fn idle_reset_path(dir: &Path) -> PathBuf {
    dir.join("idle-reset")
}

/// The idle reset's service and timer as installed: the script's absolute
/// path, the server's unit and port.
fn render_idle(dir: &Path, naming: &Naming) -> (String, String) {
    let fill = |t: &str| {
        t.replace("@RESET@", &idle_reset_path(dir).display().to_string())
            .replace("@UNIT@", &naming.unit)
            .replace("@PORT@", &naming.port.to_string())
    };
    (fill(IDLE_SERVICE), fill(IDLE_TIMER))
}

/// What is at the turbo model's path in this install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurboHere {
    Absent,
    /// mecha's link into the hub (its blob may since have gone).
    Linked,
    /// A file someone put there by hand: theirs, never replaced (review of
    /// #650).
    Placed,
}

pub fn turbo_here(mecha_home: &Path) -> TurboHere {
    match dir(mecha_home)
        .join("ComfyUI")
        .join("models")
        .join(turbo_file().path)
        .symlink_metadata()
    {
        Err(_) => TurboHere::Absent,
        Ok(m) if m.file_type().is_symlink() => TurboHere::Linked,
        Ok(_) => TurboHere::Placed,
    }
}

/// Whether this install links the turbo model where ComfyUI looks.
pub fn turbo_linked(mecha_home: &Path) -> bool {
    turbo_here(mecha_home) == TurboHere::Linked
}

/// Why an install here should fetch the turbo model, or `None`: it was
/// asked for, it is linked already (a reinstall keeps it), or the chat's
/// or a persona's picture settings choose it while `configured` — `[image] turbo_model`,
/// `None` without a table — is the pinned file. The key is never a reason
/// of its own: it has a default, so it is set on every machine. Naming
/// another file, it is what a turbo picture loads, which this fetch would
/// not supply (review of #650).
pub fn turbo_wanted(mecha_home: &Path, asked: bool, configured: Option<&str>) -> Option<String> {
    if asked {
        return Some("asked for with --turbo".into());
    }
    if turbo_linked(mecha_home) {
        return Some("this install linked it before".into());
    }
    if configured.is_some_and(|c| c != crate::sampling::turbo_model()) {
        return None;
    }
    // The chat's settings are every persona's that leaves the model unset.
    // A damaged or unreadable file is said by `sampling::chat` itself, in a
    // warning, and reads here as nothing chosen.
    let chat = crate::sampling::chat(&crate::sampling::chat_path(mecha_home));
    if chat.model == Some(crate::sampling::Model::Turbo) {
        return Some("the chat's picture settings choose it".into());
    }
    let personas = mecha_home.join("personas");
    // An unreadable store is said, never read as one where no persona
    // chooses turbo (review of #650); a missing one is no personas.
    let entries = match std::fs::read_dir(&personas) {
        Ok(entries) => Some(entries),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            tracing::warn!(
                "{} could not be read ({e}), so no persona's choice of the turbo model is \
                 known; `mecha features enable image --turbo` fetches it",
                personas.display()
            );
            None
        }
    };
    let mut names: Vec<String> = entries
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| {
            crate::persona::tuning(&personas, name).model == Some(crate::sampling::Model::Turbo)
        })
        .collect();
    names.sort();
    match names.as_slice() {
        [] => None,
        [one] => Some(format!("{one}'s picture settings choose it")),
        many => Some(format!(
            "the picture settings of {} choose it",
            many.join(", ")
        )),
    }
}

/// The image slot's pinned files for this machine: (repo, revision, file).
fn models(
    machine: &crate::recommend::Machine,
) -> Result<
    Vec<(
        &'static str,
        &'static str,
        &'static crate::recommend::HubFile,
    )>,
> {
    let slot = crate::recommend::SLOTS
        .iter()
        .find(|s| s.id == "image")
        .context("no image slot")?;
    let (row, _) = crate::recommend::row_for(slot, machine)
        .context("no image model is recommended at this machine's tier")?;
    let mut out = Vec::new();
    for source in row.sources {
        let crate::recommend::Source::HuggingFace {
            repo,
            revision,
            files,
        } = source
        else {
            bail!("the image model is not a Hugging Face file");
        };
        out.extend(files.iter().map(|f| (*repo, *revision, f)));
    }
    Ok(out)
}

/// `turbo` also fetches and links [`turbo_file`], and checks ComfyUI
/// lists it.
pub async fn install(
    m: &Machinery,
    naming: &Naming,
    machine: &crate::recommend::Machine,
    hub: &Path,
    turbo: bool,
    say: Say<'_>,
) -> Result<()> {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    fits(os, arch, crate::engine::read_nvidia()).map_err(anyhow::Error::msg)?;
    let lock = lock_for(os, arch).context("no lock for this architecture")?;
    let macos = os == "macos";
    let home = &m.mecha_home;
    let dir = dir(home);
    let units = crate::engine_gate::unit_dir(m)?.to_path_buf();
    // A server on the port whose unit mecha did not write is someone's — a
    // ComfyUI started from a terminal or under another unit name. Nothing is
    // installed beside it (`stt_unit`'s refusal, #638).
    if someone_elses(m, naming)? {
        bail!(
            "something already answers on :{} and mecha did not install it — nothing is \
             installed over it; `mecha features enable image --no-install` switches images on \
             and uses it as it is",
            naming.port
        );
    }
    let mut models = models(machine)?;
    if turbo {
        models.push((TURBO_REPO, TURBO_REVISION, turbo_file()));
    }
    // Before anything is fetched — the source, the environment and ~21 GB of
    // models — as the voice installs ask it (review of #681).
    crate::user_service::preflight(m)?;
    Manifest::begin(home, ID)?;
    Manifest::record(home, ID, &dir)?;
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }

    // --- the source, by its commit
    let comfy = dir.join("ComfyUI");
    Manifest::record(home, ID, &comfy)?;
    let ours = std::fs::read_to_string(comfy.join(COMMIT_STAMP)).is_ok_and(|s| s.trim() == COMMIT);
    if !ours {
        say(&format!("fetching ComfyUI at {}", &COMMIT[..9]));
        fetch_source(&dir, &comfy)?;
    }

    // --- the environment
    crate::install::locked_env(
        m,
        ID,
        &dir,
        PYTHON,
        lock,
        &if macos {
            format!(
                "torch for MPS and ComfyUI's requirements (about {:.1} GiB to fetch)",
                env_bytes(os, arch).unwrap_or(0) as f64 / (1u64 << 30) as f64
            )
        } else {
            format!(
                "torch for CUDA 13 and ComfyUI's requirements (about {:.1} GiB to fetch, about \
                 6.3 GiB on disk)",
                env_bytes(os, arch).unwrap_or(0) as f64 / (1u64 << 30) as f64
            )
        },
        say,
    )
    .await?;

    // --- the models, from the hub, linked where ComfyUI looks
    for (repo, revision, f) in &models {
        if crate::fetch::cached(hub, repo, revision, f, false)? == crate::fetch::Cached::Verified {
            say(&format!("{repo}/{} is in the cache", f.path));
        } else {
            say(&format!(
                "fetching {repo}/{} ({:.1} GiB)",
                f.path,
                f.bytes as f64 / (1u64 << 30) as f64
            ));
        }
        let snap = crate::fetch::fetch_hub_file(hub, repo, revision, f, &mut |_| {}).await?;
        // `diffusion_models/…`, `text_encoders/…`, `vae/…`: the hub path is
        // ComfyUI's own layout under `models/`.
        let link = comfy.join("models").join(f.path);
        Manifest::record(home, ID, &link)?;
        if let Some(parent) = link.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _ = std::fs::remove_file(&link);
        #[cfg(unix)]
        std::os::unix::fs::symlink(&snap, &link)
            .with_context(|| format!("linking {}", link.display()))?;
    }

    // --- the unit, and (on Linux) the idle reset beside it
    // The unit's `ExecStartPre` makes the temp directory on every start; a
    // launchd agent has no such step, so the install makes it on both.
    std::fs::create_dir_all(dir.join("tmp"))?;
    let unit = render(&dir, naming);
    let unit = if macos {
        // The proxy in front, ComfyUI behind it on the backend port (module
        // doc, `mac_backend_port`).
        let backend = render(
            &dir,
            &Naming {
                unit: naming.unit.clone(),
                port: mac_backend_port(naming.port),
            },
        );
        let launcher = dir.join("bin").join("comfyui-on-demand");
        write_owned(home, ID, &launcher, &mac_launcher_text(&backend)?, 0o755)?;
        let mecha = std::env::current_exe().context("finding the mecha binary for the agent")?;
        for_mac(&unit, &mecha, &launcher, naming.port)
    } else {
        unit
    };
    crate::user_service::write(m, ID, &naming.unit, &unit)?;
    let names: Vec<&str> = models
        .iter()
        .filter_map(|(_, _, f)| f.path.rsplit('/').next())
        .collect();
    if macos {
        // No idle reset on a Mac — its proxy stops ComfyUI when idle (module
        // doc): the agent alone, loaded, checked through the proxy (a cold
        // start), and stopped if the check fails.
        let service = naming.unit.as_str();
        let checked = match crate::user_service::restart(m, &[service], &mut *say) {
            Ok(()) => {
                say(&format!(
                    "starting it on :{} and asking it for every node and model mecha's pictures \
                     use",
                    naming.port
                ));
                check(naming.port, &names).await
            }
            Err(e) => Err(e),
        };
        if let Err(e) = checked {
            let stopped = match crate::user_service::stop(&[service]) {
                Ok(()) => format!("{service} was stopped"),
                Err(stop) => format!(
                    "{service} could NOT be stopped ({stop:#}) — it may still be listening on \
                     :{}; run `{}`",
                    naming.port,
                    crate::user_service::stop_by_hand(&[service])
                ),
            };
            return Err(e.context(stopped));
        }
        say(&format!(
            "ComfyUI starts at the first picture (a cold start adds ~20 s) and is stopped after \
             {} idle minutes, giving its memory back",
            MAC_IDLE_SECS / 60
        ));
        return Manifest::finish(home, ID);
    }
    let (idle_service, idle_timer) = render_idle(&dir, naming);
    write_owned(home, ID, &idle_reset_path(&dir), IDLE_RESET, 0o755)?;
    let idle = naming.idle();
    write_owned(
        home,
        ID,
        &units.join(format!("{idle}.service")),
        &crate::user_service::stamp(ID, &idle_service),
        0o644,
    )?;
    write_owned(
        home,
        ID,
        &units.join(format!("{idle}.timer")),
        &crate::user_service::stamp(ID, &idle_timer),
        0o644,
    )?;
    crate::engine_gate::systemctl("daemon-reload", &[])?;
    let service = format!("{}.service", naming.unit);
    // The timer before the server is enabled or started: without it a loaded
    // server holds its memory for good, so a timer that cannot start fails
    // the install — here, before anything is started or checked, so the
    // caller's later steps (the `[image]` table) are left for a re-run rather
    // than skipped behind a running server (review of #661). A reinstall's
    // server was enabled and may be running, so it is stopped and disabled:
    // neither now nor at boot does a server run with no reset (review of
    // #661, passes 2 and 3). A
    // tick before the server answers finds nothing to release.
    let timer = format!("{idle}.timer");
    if let Err(e) = crate::engine_gate::systemctl("enable", &["--now", &timer]) {
        // `--now`: a reinstall's server may be running, and a running server
        // with no reset is the state this branch exists to refuse (review of
        // #661, pass 3).
        let stopped = match crate::engine_gate::systemctl("disable", &["--now", &service]) {
            Ok(()) => format!("{service} was stopped and disabled"),
            Err(stop) => format!("{service} could NOT be stopped ({stop:#})"),
        };
        return Err(e).context(format!(
            "{timer}, the idle reset, could not be started, so a loaded model would stay in \
             memory — {stopped}; run `systemctl --user status {timer}` to see why"
        ));
    }
    if let Err(e) = crate::engine_gate::systemctl("enable", &[&service]) {
        // Every exit agrees: no timer ticks across reboots against a server
        // that never starts (review of #661, pass 4).
        let _ = crate::engine_gate::systemctl("disable", &["--now", &timer]);
        return Err(e);
    }
    // A restart, not `enable --now`, so a reinstall reaches a running server;
    // a failed restart or check stops it before the install says so (#639).
    let checked = match crate::engine_gate::systemctl("restart", &[&service]) {
        Ok(()) => {
            say(&format!(
                "starting it on :{} and asking it for every node and model mecha's pictures use",
                naming.port
            ));
            check(naming.port, &names).await
        }
        Err(e) => Err(e),
    };
    if let Err(e) = checked {
        let _ = crate::engine_gate::systemctl("disable", &["--now", &timer]);
        let stopped = match crate::engine_gate::systemctl("disable", &["--now", &service]) {
            Ok(()) => format!("{service} was stopped and disabled"),
            Err(stop) => format!(
                "{service} could NOT be stopped ({stop:#}) — it may still be listening on :{}; \
                 run `systemctl --user disable --now {service}`",
                naming.port
            ),
        };
        return Err(e.context(stopped));
    }
    say(&format!(
        "{timer} restarts it once it has held a model unused for 15 minutes"
    ));
    Manifest::finish(home, ID)
}

/// The pinned commit into `comfy`: fetched by its hash into a scratch tree,
/// checked, its `.git` dropped, stamped, and swapped in whole.
fn fetch_source(dir: &Path, comfy: &Path) -> Result<()> {
    let part = dir.join("ComfyUI.part");
    let _ = std::fs::remove_dir_all(&part);
    std::fs::create_dir_all(&part)?;
    let git = |args: &[&str]| {
        let mut c = std::process::Command::new("git");
        c.arg("-C")
            .arg(&part)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0");
        c
    };
    let fetched = (|| -> Result<()> {
        crate::install::run(&mut git(&["init", "--quiet"]), "git init")?;
        crate::install::run(
            &mut git(&["fetch", "--quiet", "--depth", "1", REPO, COMMIT]),
            "fetching ComfyUI",
        )?;
        crate::install::run(
            &mut git(&[
                "-c",
                "advice.detachedHead=false",
                "checkout",
                "--quiet",
                "FETCH_HEAD",
            ]),
            "checking out ComfyUI",
        )?;
        let out = git(&["rev-parse", "HEAD"])
            .output()
            .context("reading ComfyUI's commit")?;
        let head = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if head != COMMIT {
            bail!("the ComfyUI fetched is commit {head}, not the pinned {COMMIT}");
        }
        std::fs::remove_dir_all(part.join(".git"))?;
        std::fs::write(part.join(COMMIT_STAMP), COMMIT)?;
        Ok(())
    })();
    if let Err(e) = fetched {
        let _ = std::fs::remove_dir_all(&part);
        return Err(e);
    }
    if comfy.exists() {
        std::fs::remove_dir_all(comfy).with_context(|| format!("clearing {}", comfy.display()))?;
    }
    std::fs::rename(&part, comfy)?;
    Ok(())
}

/// ComfyUI answers, knows every node type mecha sends, and lists the model
/// files by the names `[image]` loads. Retried while it starts — torch's
/// import and the node scan take ~20 s.
async fn check(port: u16, model_files: &[&str]) -> Result<()> {
    let base = format!("http://127.0.0.1:{port}");
    // Loopback, never through a proxy (`llama_units::check`'s reason).
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let deadline = std::time::Instant::now() + Duration::from_secs(300);
    loop {
        match client.get(format!("{base}/system_stats")).send().await {
            Ok(r) if r.status().is_success() => break,
            Ok(r) if std::time::Instant::now() > deadline => {
                bail!(
                    "ComfyUI on :{port} answered {} to /system_stats",
                    r.status()
                )
            }
            Err(e) if std::time::Instant::now() > deadline => {
                return Err(e).with_context(|| {
                    format!(
                        "ComfyUI on :{port} did not answer within five minutes — {}",
                        crate::user_service::logs_anywhere()
                    )
                })
            }
            _ => tokio::time::sleep(Duration::from_secs(2)).await,
        }
    }
    let info: serde_json::Value = client
        .get(format!("{base}/object_info"))
        .send()
        .await
        .context("asking ComfyUI for its node types")?
        .error_for_status()?
        .json()
        .await
        .context("reading ComfyUI's node types")?;
    let missing: Vec<&str> = NODES
        .iter()
        .copied()
        .filter(|n| info.get(n).is_none())
        .collect();
    if !missing.is_empty() {
        bail!(
            "ComfyUI on :{port} has no {} — mecha's pictures could not be drawn on it",
            missing.join(", ")
        );
    }
    let listed = listed_models(&info);
    let unseen: Vec<&str> = model_files
        .iter()
        .copied()
        .filter(|f| !listed.iter().any(|l| l == f))
        .collect();
    if !unseen.is_empty() {
        bail!(
            "ComfyUI on :{port} does not list {} among its models — the links under models/ \
             did not land where it looks",
            unseen.join(", ")
        );
    }
    Ok(())
}

/// Every file the four loaders offer, off `/object_info`: each loader's
/// first input's choices. The checkpoint loader is SAM 3.1's
/// (`recommend::SAM_SOURCE`).
fn listed_models(info: &serde_json::Value) -> Vec<String> {
    let choices = |node: &str, input: &str| -> Vec<String> {
        info[node]["input"]["required"][input][0]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut out = choices("UNETLoader", "unet_name");
    out.extend(choices("CLIPLoader", "clip_name"));
    out.extend(choices("VAELoader", "vae_name"));
    out.extend(choices("CheckpointLoaderSimple", "ckpt_name"));
    out
}

/// After an install: give the config an `[image]` table naming this
/// install's `server_temp_dir`, when it has no `[image]` table at all — the
/// switch alone reads off without one, and every other key's default is what
/// the install serves. A table the owner wrote is never edited: `Ok(false)`,
/// and the caller says the one line to add. In place through `toml_edit`, so
/// the file's comments stay (`feature::write_switches`' reason).
pub fn write_image_table(path: &Path, temp: Option<&Path>) -> Result<bool> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("{} is not valid TOML; nothing was changed", path.display()))?;
    if doc.contains_key("image") {
        return Ok(false);
    }
    // An empty table registers the tool on its defaults (`:8188`); the temp
    // dir is named only where it is known — this install's (review of #645).
    let mut table = toml_edit::Table::new();
    if let Some(temp) = temp {
        table.insert(
            "server_temp_dir",
            toml_edit::value(temp.display().to_string()),
        );
    }
    crate::config::append_table(&mut doc, "image", table);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = path.with_extension("toml.image-tmp");
    std::fs::write(&tmp, doc.to_string()).with_context(|| format!("writing {}", tmp.display()))?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(true)
}

/// Whether a server answers on the port whose unit mecha did not write —
/// someone's own ComfyUI, used as it is (`stt_unit::someone_elses`'s twin).
pub fn someone_elses(m: &Machinery, naming: &Naming) -> Result<bool> {
    let unit = crate::user_service::file(m, &naming.unit)?;
    let wrote = Manifest::read(&m.mecha_home)?
        .entries
        .iter()
        .any(|e| e.sidecar == ID && e.wrote.contains(&unit));
    Ok(!wrote && crate::router_unit::port_answers(naming.port))
}

/// Stop and remove a server installed under `naming`, its tree included —
/// what the ignored test cleans up with. The models stay in the hub, where
/// every other reader of them finds them.
pub fn remove(m: &Machinery, naming: &Naming) -> Result<()> {
    let home = &m.mecha_home;
    // The timer first, so no reset restarts the server being removed.
    let timer = format!("{}.timer", naming.idle());
    if !cfg!(target_os = "macos") {
        let _ = crate::engine_gate::systemctl("disable", &["--now", &timer]);
    }
    let _ = crate::user_service::stop(&[&naming.unit]);
    let units = crate::engine_gate::unit_dir(m)?.to_path_buf();
    for path in [
        units.join(timer),
        units.join(format!("{}.service", naming.idle())),
        crate::user_service::file(m, &naming.unit)?,
    ] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("removing {}", path.display())),
        }
        Manifest::unrecord(home, ID, &path)?;
    }
    crate::user_service::after_remove()?;
    let dir = dir(home);
    // Recorded paths first, deepest first, then the tree: a record is
    // forgotten only after its file is gone.
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
    let _ = std::fs::remove_dir_all(&dir);
    Manifest::forget_if_empty(home, ID)
}

#[cfg(test)]
mod tests {
    /// The image server's revision is held to what its install writes: the
    /// unit, Linux's idle reset, and a Mac's on-demand launcher and agent.
    #[test]
    fn the_image_server_s_revision_is_held_to_what_it_writes() {
        let n = Naming::shipped();
        let dir = Path::new("/h/.mecha/sidecars/comfyui");
        let unit = render(dir, &n);
        let (idle_service, idle_timer) = render_idle(dir, &n);
        let backend = render(
            dir,
            &Naming {
                unit: n.unit.clone(),
                port: mac_backend_port(n.port),
            },
        );
        let launcher = dir.join("bin").join("comfyui-on-demand");
        let mac = for_mac(&unit, Path::new("/u/.cargo/bin/mecha"), &launcher, n.port);
        crate::user_service::assert_rev(
            ID,
            (
                1,
                "ac3352765c9ddd5cfbf0a1d1fa4821a08dfd4f981eea21513db8d5fd98ea2838",
            ),
            &[
                &unit,
                IDLE_RESET,
                &idle_service,
                &idle_timer,
                &mac_launcher_text(&backend).unwrap(),
                &mac,
                &crate::user_service::test_agent_text(&n.unit, &mac),
            ],
        );
    }

    use super::*;

    /// The file fetched on demand is the one a turbo picture loads, by
    /// name, and lands in the directory the unet loader reads.
    #[test]
    fn the_turbo_pin_is_the_file_a_turbo_picture_loads() {
        // On either system: the int8 file on Linux, the bf16 on a Mac.
        for os in ["linux", "macos"] {
            assert_eq!(
                turbo_file_on(os).path,
                format!("diffusion_models/{}", crate::sampling::turbo_model_on(os)),
                "{os}"
            );
        }
        assert_eq!(TURBO_FILE.path, turbo_file_on("linux").path);
        assert_eq!(
            crate::imagegen::ImageConfig::default().turbo_model,
            crate::sampling::turbo_model()
        );
    }

    /// The standard model the image slot fetches is the one `[image]`'s
    /// default loads, on either system — the slot's Mac row names the bf16
    /// file `sampling::standard_model_on("macos")` does (§12.3).
    #[test]
    fn each_system_fetches_the_standard_model_it_loads() {
        let slot = crate::recommend::SLOTS
            .iter()
            .find(|s| s.id == "image")
            .unwrap();
        let machine = crate::recommend::Machine::Unified {
            total_mb: 131_072,
            gpu_unread: false,
        };
        for os in ["linux", "macos"] {
            let (row, _) = crate::recommend::row_for_os(slot, &machine, os).unwrap();
            let paths: Vec<&str> = row
                .sources
                .iter()
                .flat_map(|s| match s {
                    crate::recommend::Source::HuggingFace { files, .. } => {
                        files.iter().map(|f| f.path).collect()
                    }
                    _ => Vec::new(),
                })
                .collect();
            let want = format!(
                "diffusion_models/{}",
                crate::sampling::standard_model_on(os)
            );
            assert!(paths.contains(&want.as_str()), "{os}: {paths:?}");
        }
    }

    /// Turbo is fetched when asked for, when already linked, or when a
    /// persona's picture settings choose it — and not because of anything
    /// else in a persona's `[image]` table.
    #[test]
    fn turbo_is_wanted_when_asked_linked_or_chosen() {
        let home = std::env::temp_dir().join(format!("mecha-turbo-{}", uuid::Uuid::new_v4()));
        let persona = |name: &str, image: &str| {
            let d = home.join("personas").join(name);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("state.toml"), format!("[image]\n{image}\n")).unwrap();
        };
        let pinned = Some(crate::sampling::turbo_model());
        let none = turbo_wanted(&home, false, pinned);
        let asked = turbo_wanted(&home, true, pinned);
        persona("ada", "cfg = 3.0");
        let standard = turbo_wanted(&home, false, pinned);
        persona("cleo", "model = \"turbo\"");
        persona("bea", "model = \"turbo\"");
        let chosen = turbo_wanted(&home, false, pinned);
        let untabled = turbo_wanted(&home, false, None);
        // The chat's settings choosing turbo are every unset persona's.
        crate::sampling::set_chat(
            &crate::sampling::chat_path(&home),
            crate::sampling::Tuning {
                model: Some(crate::sampling::Model::Turbo),
                ..Default::default()
            },
        )
        .unwrap();
        let by_chat = turbo_wanted(&home, false, pinned);
        std::fs::remove_file(crate::sampling::chat_path(&home)).unwrap();
        // Another configured file is what a turbo picture loads: fetching
        // the pinned one for it would supply nothing.
        let elsewhere = turbo_wanted(&home, false, Some("my-turbo.gguf"));
        let _ = std::fs::remove_dir_all(home.join("personas"));
        let link = dir(&home)
            .join("ComfyUI")
            .join("models")
            .join(turbo_file().path);
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink("/nonexistent/blob", &link).unwrap();
        let linked = turbo_wanted(&home, false, pinned);
        // A file put there by hand is no link of mecha's.
        std::fs::remove_file(&link).unwrap();
        std::fs::write(&link, b"theirs").unwrap();
        let placed = (turbo_here(&home), turbo_wanted(&home, false, pinned));
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(placed, (TurboHere::Placed, None));
        assert_eq!(none, None);
        assert!(asked.unwrap().contains("--turbo"));
        assert_eq!(standard, None);
        assert_eq!(
            chosen.as_deref(),
            Some("the picture settings of bea, cleo choose it")
        );
        assert_eq!(untabled, chosen);
        assert_eq!(
            by_chat.as_deref(),
            Some("the chat's picture settings choose it")
        );
        assert_eq!(elsewhere, None);
        assert!(
            linked.unwrap().contains("before"),
            "a dangling link still counts"
        );
    }

    /// The install checks exactly the node types mecha's graphs send — built
    /// here, not read off the source: a text-to-image graph, an edit, a
    /// masked edit and a segmentation, with the default loader and a `.gguf`
    /// one. Both ways, so
    /// a node added to a graph fails here, and so does one left in `NODES`
    /// that no graph sends any more (review of #645).
    #[test]
    fn the_install_checks_exactly_the_nodes_the_graphs_send() {
        use crate::imagegen::{comfy_graph, ImageConfig, Request, EDIT_REFERENCE_SIZE};
        let req = Request {
            prompt: "a lighthouse".into(),
            size: Some((1024, 1024)),
            sampling: crate::sampling::Sampling::reference(4),
            seed: 1,
            references: Vec::new(),
            reference_size: EDIT_REFERENCE_SIZE,
            mask: None,
        };
        let gguf = ImageConfig {
            diffusion_model: "model.gguf".into(),
            ..ImageConfig::default()
        };
        let refs = ["ref.png".to_string()];
        let mut sent = std::collections::BTreeSet::new();
        for cfg in [ImageConfig::default(), gguf] {
            for g in [
                comfy_graph(&cfg, &req, &[], None),
                comfy_graph(&cfg, &req, &refs, None),
                comfy_graph(&cfg, &req, &refs, Some("mask.png")),
                crate::imagegen::sam_graph("ref.png", "the sky"),
            ] {
                for node in g.as_object().unwrap().values() {
                    sent.insert(node["class_type"].as_str().unwrap().to_string());
                }
            }
        }
        // The GGUF loader is the one sent node the install does not bring.
        assert!(sent.remove("UnetLoaderGGUF"), "{sent:?}");
        let checked: std::collections::BTreeSet<String> =
            NODES.iter().map(|n| n.to_string()).collect();
        assert_eq!(sent, checked);
    }

    /// The models linked are the image slot's, and their names are the ones
    /// `[image]`'s defaults load — so the default config draws on a fresh
    /// install with nothing set.
    #[test]
    fn the_linked_models_are_the_ones_image_loads_by_default() {
        let machine = crate::recommend::Machine::Unified {
            total_mb: 124_610,
            gpu_unread: false,
        };
        let names: Vec<&str> = models(&machine)
            .unwrap()
            .iter()
            .filter_map(|(_, _, f)| f.path.rsplit('/').next())
            .collect();
        let d = crate::imagegen::ImageConfig::default();
        let sam = crate::imagegen::SEGMENT_MODEL.to_string();
        for wanted in [&d.diffusion_model, &d.text_encoder, &d.vae, &sam] {
            assert!(
                names.contains(&wanted.as_str()),
                "{wanted} not in {names:?}"
            );
        }
        for (_, _, f) in models(&machine).unwrap() {
            let top = f.path.split('/').next().unwrap();
            assert!(
                ["diffusion_models", "text_encoders", "vae", "checkpoints"].contains(&top),
                "{} is not under a folder ComfyUI reads",
                f.path
            );
        }
    }

    #[test]
    fn a_cuda_13_linux_machine_or_apple_silicon_fits() {
        use crate::engine::Nvidia::*;
        assert!(fits("linux", "aarch64", Driver(580, 95)).is_ok());
        assert!(fits("linux", "x86_64", Driver(590, 0)).is_ok());
        assert!(fits("linux", "x86_64", Driver(570, 1))
            .unwrap_err()
            .contains("580"));
        assert!(fits("linux", "aarch64", None)
            .unwrap_err()
            .contains("NVIDIA"));
        assert!(fits("linux", "aarch64", Unread)
            .unwrap_err()
            .contains("nvidia-smi"));
        assert!(fits("linux", "riscv64", Driver(580, 0)).is_err());
        // MPS: no driver asked (§12.3), a macOS floor instead; an Intel
        // Mac has no MPS.
        assert!(fits_with("macos", "aarch64", None, Some(MACOS_FLOOR)).is_ok());
        assert!(fits_with("macos", "aarch64", None, Some(27)).is_ok());
        assert!(fits_with("macos", "aarch64", None, Some(13))
            .unwrap_err()
            .contains("macOS 13"));
        assert!(fits_with("macos", "aarch64", None, Option::None)
            .unwrap_err()
            .contains("could not be read"));
        assert!(fits("macos", "x86_64", None)
            .unwrap_err()
            .contains("Apple silicon"));
        assert!(fits("freebsd", "x86_64", Driver(580, 0))
            .unwrap_err()
            .contains("neither"));
    }

    /// Each lock pins every file by hash, names no index, and takes torch
    /// and torchvision from one cu130 wheel for its own architecture.
    #[test]
    fn each_lock_pins_torch_by_one_wheel_for_its_architecture() {
        for (arch, lock) in [("aarch64", LOCK_AARCH64), ("x86_64", LOCK_X86_64)] {
            assert!(!lock.contains("--index-url") && !lock.contains("--extra-index-url"));
            let lines: Vec<&str> = lock.lines().collect();
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
                    "{arch}: {l} has no hash"
                );
                if let Some(rest) = l
                    .strip_prefix("torch @ ")
                    .or_else(|| l.strip_prefix("torchvision @ "))
                {
                    assert!(
                        rest.starts_with("https://download-r2.pytorch.org/whl/cu130/"),
                        "{l}"
                    );
                    assert!(
                        rest.contains(&format!("cp312-cp312-manylinux_2_28_{arch}.whl")),
                        "{l}"
                    );
                    // One hash, the wheel's own: the next line ends the list.
                    assert!(
                        !lines[i + 1].ends_with('\\'),
                        "{arch}: {l} lists more than one hash"
                    );
                }
            }
            assert!(pins > 80, "{arch}: {pins}");
            assert!(lock.contains("torch @ ") && lock.contains("torchvision @ "));
            assert!(env_bytes("linux", arch).is_some());
        }
    }

    /// The Mac's lock pins every file by hash, names no index, takes torch
    /// from PyPI (whose macOS wheels carry MPS), and carries nothing of
    /// CUDA's.
    #[test]
    fn the_mac_lock_is_pypi_torch_and_nothing_of_cuda() {
        let lock = LOCK_MACOS_ARM64;
        assert!(!lock.contains("--index-url") && !lock.contains("--extra-index-url"));
        let lines: Vec<&str> = lock.lines().collect();
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
            assert!(!l.contains(" @ "), "{l}: a direct URL, not PyPI");
            for cuda in ["nvidia-", "triton", "cuda-"] {
                assert!(!l.starts_with(cuda), "{l}");
            }
        }
        assert!(pins > 60, "{pins}");
        assert!(lock.contains("\ntorch==2.14.1 ") && lock.contains("\ntorchvision==0.29.1 "));
        assert_eq!(lock_for("macos", "aarch64"), Some(lock));
        assert_eq!(lock_for("macos", "x86_64"), None);
        assert!(env_bytes("macos", "aarch64").is_some());
    }

    /// Each lock names the commit it was compiled for, and it is `COMMIT`:
    /// a bump that moves the constant and not the locks fails here, not on
    /// an owner's machine (review of #645).
    #[test]
    fn the_locks_were_compiled_for_this_commit() {
        for lock in [LOCK_AARCH64, LOCK_X86_64, LOCK_MACOS_ARM64] {
            let header: String = lock
                .lines()
                .take_while(|l| l.starts_with('#'))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(header.contains(&COMMIT[..8]), "{header}");
        }
    }

    #[test]
    fn the_unit_renders_this_home_and_port() {
        let dir = Path::new("/srv/owner/.mecha/sidecars/comfyui");
        let text = render(
            dir,
            &Naming {
                unit: "mecha-comfyui".into(),
                port: 9188,
            },
        );
        assert!(!text.contains('@'), "{text}");
        // A Mac reads the same unit as its agent (`user_service`): the same
        // command, in the same directory, and never `--gpu-only`, which
        // would put the text encoder's float8 scales on MPS (§12.3).
        // The Linux unit as written is refused (its `ExecStartPre` has no
        // agent counterpart); `for_mac` is what a Mac reads.
        assert!(crate::launchd::agent_from_unit(
            "mecha-comfyui",
            &text,
            Path::new("/Users/owner"),
            "/l".into(),
        )
        .is_err());
        // On a Mac the agent runs mecha's on-demand proxy, and the proxy runs
        // the unit's own command on the backend port (owner's ruling,
        // 2026-10-14).
        let mecha = Path::new("/Users/owner/.cargo/bin/mecha");
        let launcher = dir.join("bin/comfyui-on-demand");
        let agent = crate::launchd::agent_from_unit(
            "mecha-comfyui",
            &for_mac(&text, mecha, &launcher, 9188),
            Path::new("/Users/owner"),
            "/l".into(),
        )
        .unwrap();
        assert_eq!(
            agent.program,
            [
                "/Users/owner/.cargo/bin/mecha",
                "features",
                "on-demand",
                "--listen",
                "127.0.0.1:9188",
                "--backend",
                "19188",
                "--idle-secs",
                "900",
                "--ready-path",
                "/system_stats",
                "--launcher",
                "/srv/owner/.mecha/sidecars/comfyui/bin/comfyui-on-demand",
            ]
        );
        let path = agent.env.iter().find(|(k, _)| k == "PATH").unwrap();
        assert_eq!(path.1, crate::launchd::SYSTEM_PATH);
        // The launcher is the unit's command on the backend port, from its
        // directory, and never `--gpu-only`, which would put the text
        // encoder's float8 scales on MPS (§12.3).
        let backend = render(
            dir,
            &Naming {
                unit: "mecha-comfyui".into(),
                port: mac_backend_port(9188),
            },
        );
        let script = mac_launcher_text(&backend).unwrap();
        assert!(script.contains("cd '/srv/owner/.mecha/sidecars/comfyui/ComfyUI' || exit 1"));
        assert!(script.contains("mkdir -p '/srv/owner/.mecha/sidecars/comfyui/tmp' || exit 1"));
        // The proxy's window and the Linux timer's default are one figure:
        // held equal here, not by a comment in each module.
        assert_eq!(
            MAC_IDLE_SECS,
            crate::imagegen::ImageConfig::default().unload_after_secs
        );
        assert!(script.contains(
            "exec /srv/owner/.mecha/sidecars/comfyui/venv/bin/python -B main.py --listen \
             127.0.0.1 --port 19188 --temp-directory /srv/owner/.mecha/sidecars/comfyui/tmp"
        ));
        assert!(!script.contains("--gpu-only"));
        assert!(text.contains("WorkingDirectory=/srv/owner/.mecha/sidecars/comfyui/ComfyUI"));
        assert!(text.contains(
            "ExecStart=/srv/owner/.mecha/sidecars/comfyui/venv/bin/python -B main.py --listen \
             127.0.0.1 --port 9188 --temp-directory /srv/owner/.mecha/sidecars/comfyui/tmp"
        ));
        // What `[image] server_temp_dir` must name: the temp directory plus
        // ComfyUI's own `temp`.
        assert_eq!(
            server_temp_dir(Path::new("/srv/owner/.mecha")),
            dir.join("tmp/temp")
        );
    }

    /// The idle reset that runs is the hand install's script, byte for byte —
    /// so a fix made in one reaches the other or fails here.
    #[test]
    fn the_shipped_idle_reset_is_the_scripts_copy() {
        let script = include_str!("../../scripts/comfyui/comfyui-idle-reset");
        assert_eq!(IDLE_RESET, script);
    }

    /// The units beside the script are renamed and placeholdered, so they
    /// cannot be byte-equal to the hand install's — but what must agree does:
    /// the `[Timer]` schedule, and the `PATH` the script's `nvidia-smi`,
    /// `curl`, `ss` and `python3` resolve through (review of #661, pass 4).
    #[test]
    fn the_idle_units_keep_the_hand_installs_schedule_and_path() {
        let timer_body = |t: &str| -> String {
            t.lines()
                .skip_while(|l| l.trim() != "[Timer]")
                .take_while(|l| !l.starts_with("[Install]"))
                .filter(|l| !l.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let path_line = |t: &str| -> String {
            t.lines()
                .find(|l| l.starts_with("Environment=PATH="))
                .unwrap_or_default()
                .to_string()
        };
        let hand_timer = include_str!("../../scripts/comfyui/mecha-comfyui-idle-reset.timer");
        let hand_service = include_str!("../../scripts/comfyui/mecha-comfyui-idle-reset.service");
        assert!(timer_body(hand_timer).contains("OnUnitActiveSec="));
        assert_eq!(timer_body(IDLE_TIMER), timer_body(hand_timer));
        assert!(!path_line(hand_service).is_empty());
        assert_eq!(path_line(IDLE_SERVICE), path_line(hand_service));
    }

    /// The timer runs the script under this home against this unit and port,
    /// under a name a hand install's reset does not use.
    #[test]
    fn the_idle_reset_renders_this_home_unit_and_port() {
        let dir = Path::new("/srv/owner/.mecha/sidecars/comfyui");
        let naming = Naming {
            unit: "mecha-comfyui".into(),
            port: 9188,
        };
        let (service, timer) = render_idle(dir, &naming);
        assert!(!service.contains('@'), "{service}");
        assert!(!timer.contains('@'), "{timer}");
        assert!(service.contains("ExecStart=/srv/owner/.mecha/sidecars/comfyui/idle-reset\n"));
        // The script's defaults name the hand install's `comfyui` on :8188;
        // every one of them is set here.
        for line in [
            "Environment=COMFY_UNIT=mecha-comfyui\n",
            "Environment=COMFY_URL=http://127.0.0.1:9188\n",
            "Environment=COMFY_PORT=9188\n",
        ] {
            assert!(service.contains(line), "{line} in {service}");
        }
        assert!(timer.contains("OnUnitActiveSec=1min"));
        // `scripts/comfyui/install.sh` installs `mecha-comfyui-idle-reset`.
        assert_eq!(Naming::shipped().idle(), "mecha-comfyui-idle");
    }

    /// A config with no `[image]` gets one naming the install's temp dir,
    /// which loads; one the owner wrote is left as it is.
    #[test]
    fn the_image_table_is_written_only_where_there_is_none() {
        let root = std::env::temp_dir().join(format!("mecha-7e-cfg-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("config.toml");
        let home = root.join(".mecha");
        std::fs::write(&path, "# mine\ndefault_provider = \"local\"\n").unwrap();
        let wrote = write_image_table(&path, Some(&server_temp_dir(&home))).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let owned = root.join("owned.toml");
        std::fs::write(&owned, "[image]\nurl = \"http://127.0.0.1:9000\"\n").unwrap();
        let left = write_image_table(&owned, Some(&server_temp_dir(&home))).unwrap();
        let owned_text = std::fs::read_to_string(&owned).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert!(wrote && !left);
        assert!(text.starts_with("# mine"), "{text}");
        let parsed: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(
            parsed["image"]["server_temp_dir"].as_str(),
            Some(server_temp_dir(&home).to_str().unwrap())
        );
        assert_eq!(owned_text, "[image]\nurl = \"http://127.0.0.1:9000\"\n");
    }

    /// The registry knows the unit this installs: with its record lost, a
    /// running install still shows itself rather than reading as missing.
    #[test]
    fn the_registry_names_the_unit_this_installs() {
        let unit = format!("{}.service", Naming::shipped().unit);
        let sc = crate::sidecar::SIDECARS
            .iter()
            .find(|s| s.id == ID)
            .unwrap();
        assert!(
            sc.evidence
                .iter()
                .any(|e| matches!(e, crate::sidecar::Evidence::UserUnit(u) if *u == unit)),
            "{unit}"
        );
    }

    /// The table `enable image` writes with no install — an empty one —
    /// loads as `[image]` on its defaults, which is what turns images on
    /// (`feature::state` reads `cfg.image.is_some()`; review of #645).
    #[test]
    fn an_empty_image_table_loads_and_switches_images_on() {
        let root = std::env::temp_dir().join(format!("mecha-7e-empty-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("config.toml");
        std::fs::write(&path, "# mine\n").unwrap();
        let wrote = write_image_table(&path, None).unwrap();
        let cfg = crate::config::Config::load_layers(Some(&path), None);
        let _ = std::fs::remove_dir_all(&root);
        assert!(wrote);
        let image = cfg.unwrap().image.expect("an [image] table");
        assert_eq!(image.url, crate::imagegen::ImageConfig::default().url);
        assert_eq!(image.server_temp_dir, None);
    }

    #[test]
    fn the_shipped_port_is_the_image_url_default() {
        let url = crate::imagegen::ImageConfig::default().url;
        assert!(
            url.ends_with(&format!(":{}", Naming::shipped().port)),
            "{url}"
        );
    }

    /// The real thing: fetch the source, build the environment from the
    /// lock, link the models from the hub (fetched if they are not there),
    /// install a unit beside the live server on a free port under a scratch
    /// mecha home, check it serves every node and model, and remove it.
    /// Gate it on the memory floor: the environment's ~6 GiB lands on disk,
    /// and ComfyUI's start takes ~1 GiB beside the live one.
    /// `cargo test -p mecha-core -- --ignored an_image_server_installs`
    #[tokio::test]
    #[ignore]
    async fn an_image_server_installs_and_answers_beside_the_live_one() {
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let naming = Naming {
            unit: format!("mecha-test-comfyui-{}", std::process::id()),
            port,
        };
        let root = std::env::temp_dir().join(format!("mecha-7e-{}", uuid::Uuid::new_v4()));
        let mut m = Machinery::real().unwrap();
        m.mecha_home = root.join(".mecha");
        let machine = crate::recommend::Machine::read().unwrap();
        // A scratch hub, seeded with hard links to a hand-built ComfyUI's
        // copies where there is one: 14 GB neither fetched nor copied, and
        // still checked by sha256 on the way in. The fetcher writes a `.part`
        // and renames it, so a linked blob is never written through.
        let hub = root.join("hub");
        let live = dirs::home_dir().unwrap().join("ComfyUI/models");
        // With turbo, so the on-demand file's fetch, link and check run too.
        let mut seeded = models(&machine).unwrap();
        seeded.push((TURBO_REPO, TURBO_REVISION, turbo_file()));
        for (repo, _, f) in seeded {
            let blob = hub
                .join(format!("models--{}", repo.replace('/', "--")))
                .join("blobs")
                .join(f.sha256);
            let have = live.join(f.path);
            if have.metadata().is_ok_and(|md| md.len() == f.bytes) {
                std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
                std::fs::hard_link(&have, &blob).unwrap();
            }
        }
        let mut said = Vec::new();
        let r = install(&m, &naming, &machine, &hub, true, &mut |s| {
            said.push(s.to_string())
        })
        .await;
        let stamped =
            std::fs::read_to_string(dir(&m.mecha_home).join("ComfyUI").join(COMMIT_STAMP));
        let turbo = turbo_linked(&m.mecha_home);
        let timer = format!("{}.timer", naming.idle());
        // The idle reset is Linux's (module doc): a Mac installs no timer.
        let macos = cfg!(target_os = "macos");
        let ticking = macos
            || std::process::Command::new("systemctl")
                .args(["--user", "is-active", "--quiet", &timer])
                .status()
                .is_ok_and(|s| s.success());
        // The service's own file, written on either system: its removal is
        // measured on a Mac too, where the timer's never existed (review of
        // #681).
        let service = crate::user_service::file(&m, &naming.unit).unwrap();
        let written = service.exists();
        let cleaned = remove(&m, &naming);
        let units = crate::engine_gate::unit_dir(&m).unwrap().to_path_buf();
        let left: Vec<PathBuf> = [
            units.join(&timer),
            units.join(format!("{}.service", naming.idle())),
            service,
        ]
        .into_iter()
        .filter(|p| p.exists())
        .collect();
        eprintln!("{said:#?}");
        let man = Manifest::read(&m.mecha_home);
        let _ = std::fs::remove_dir_all(&root);
        r.unwrap();
        cleaned.unwrap();
        assert_eq!(stamped.unwrap(), COMMIT);
        assert!(turbo, "the turbo model was linked");
        assert!(written, "the service's definition was written");
        assert!(ticking, "{timer} was started");
        assert!(left.is_empty(), "removal left {left:?}");
        let man = man.unwrap();
        assert!(
            !man.entries.iter().any(|e| e.sidecar == ID),
            "{:?}",
            man.entries
        );
    }

    /// The plist mecha writes for a Mac's ComfyUI is one the image tool reads
    /// as proxy-fronted — built through the real chain (`for_mac`, the
    /// agent reader, the log cap, the plist), so renaming the proxy command
    /// or changing `for_mac` cannot silently re-arm the unload timer, which
    /// would leave ~35 GiB held (review of #689, pass 4).
    #[test]
    fn the_plist_mecha_writes_reads_as_behind_the_proxy() {
        let home = std::env::temp_dir().join(format!("mecha-comfy-p-{}", uuid::Uuid::new_v4()));
        let dir = home.join(".mecha/sidecars/comfyui");
        let naming = Naming::shipped();
        let unit = for_mac(
            &render(&dir, &naming),
            Path::new("/Users/owner/.cargo/bin/mecha"),
            &dir.join("bin/comfyui-on-demand"),
            naming.port,
        );
        let log = crate::launchd::log_path(&home, &naming.unit);
        let mut agent = crate::launchd::agent_from_unit(
            &crate::launchd::label(&naming.unit),
            &unit,
            &home,
            log,
        )
        .unwrap();
        agent.program = crate::log_cap::wrap(
            Path::new("/Users/owner/.cargo/bin/mecha"),
            &agent.log,
            agent.program,
        );
        let plist =
            crate::launchd::agents_dir(&home).join(crate::launchd::plist_name(&naming.unit));
        std::fs::create_dir_all(plist.parent().unwrap()).unwrap();
        std::fs::write(&plist, crate::launchd::plist_text(&agent)).unwrap();
        let url = format!("http://127.0.0.1:{}", naming.port);
        assert!(crate::imagegen::behind_proxy_at(&home, &url));
        let _ = std::fs::remove_dir_all(&home);
    }
}
