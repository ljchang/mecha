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
//! **CUDA 13 only, for now.** The locks are torch's cu130 builds, which need
//! a 580 driver (`engine::CUDA13_DRIVER`'s reason); any other machine is
//! told so before anything is fetched. ComfyUI-GGUF is not installed: since
//! #560 the default model is int8 safetensors through the core loader, and
//! the GGUF node is needed only for an owner's own `.gguf`.
//!
//! The commit is one constant, [`COMMIT`], so a bump is that line and the
//! locks regenerated against it (their header says how).
//! Linux only, rendered like `stt_unit`'s, so a test installs one beside the
//! live server on a free port and removes it.

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
/// ComfyUI's commit: v0.37.0-31, what the hand-built server ran on
/// 2026-10-10 (`TextEncodeQwenImage21` is in it).
pub const COMMIT: &str = "88ab4a06566454ad89db8f0bedb970d6c08cd1b7";
const REPO: &str = "https://github.com/comfyanonymous/ComfyUI";
const UNIT: &str = include_str!("../units/mecha-comfyui.service");
const LOCK_AARCH64: &str = include_str!("../locks/comfyui-cu130-aarch64.txt");
const LOCK_X86_64: &str = include_str!("../locks/comfyui-cu130-x86_64.txt");
/// The Python the locks were compiled for.
const PYTHON: &str = "3.12";
/// Written into the source tree once it is checked: the commit it is.
const COMMIT_STAMP: &str = ".mecha-commit";

/// Every node type [`crate::imagegen`] sends but the GGUF loader, which only
/// an owner's own `.gguf` needs. A test builds the graphs and holds the two
/// sets equal, both ways.
pub const NODES: [&str; 12] = [
    "CLIPLoader",
    "EmptyLatentImage",
    "ImageToMask",
    "KSampler",
    "LoadImage",
    "PreviewImage",
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

/// What `[image] server_temp_dir` must name for this install: the unit's
/// `--temp-directory` with `temp` appended, as ComfyUI lays it out.
pub fn server_temp_dir(mecha_home: &Path) -> PathBuf {
    dir(mecha_home).join("tmp").join("temp")
}

/// The lock for an architecture, or `None` where none is pinned.
fn lock_for(arch: &str) -> Option<&'static str> {
    match arch {
        "aarch64" => Some(LOCK_AARCH64),
        "x86_64" => Some(LOCK_X86_64),
        _ => None,
    }
}

/// What the environment downloads on an architecture, in bytes: the lock's
/// files as PyPI and download.pytorch.org size them, summed 2026-10-11.
/// It lands as ~6.3 GiB in `venv/` (measured on the GB10 the same day).
pub fn env_bytes(arch: &str) -> Option<u64> {
    match arch {
        "aarch64" => Some(3_848_000_000),
        "x86_64" => Some(3_724_000_000),
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
    use crate::engine::Nvidia;
    if os != "linux" {
        return Err(
            "the image server is a systemd user unit, which this system does not have — start \
             ComfyUI by hand until launchd is designed (FEATURES-DESIGN §10.5)"
                .into(),
        );
    }
    if lock_for(arch).is_none() {
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

/// Whether this install links the turbo model where ComfyUI looks.
pub fn turbo_linked(mecha_home: &Path) -> bool {
    dir(mecha_home)
        .join("ComfyUI")
        .join("models")
        .join(TURBO_FILE.path)
        .symlink_metadata()
        .is_ok()
}

/// Why an install here should fetch the turbo model, or `None`: it was
/// asked for, it is linked already (a reinstall keeps it), or a persona's
/// picture settings choose it. `[image] turbo_model` is not a reason — it
/// has a default, so it is set on every machine.
pub fn turbo_wanted(mecha_home: &Path, asked: bool) -> Option<String> {
    if asked {
        return Some("asked for with --turbo".into());
    }
    if turbo_linked(mecha_home) {
        return Some("this install has it already".into());
    }
    let personas = mecha_home.join("personas");
    let mut names: Vec<String> = std::fs::read_dir(&personas)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| {
            crate::persona::tuning(&personas, name).model == crate::sampling::Model::Turbo
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

/// `turbo` also fetches and links [`TURBO_FILE`], and checks ComfyUI
/// lists it.
pub async fn install(
    m: &Machinery,
    naming: &Naming,
    machine: &crate::recommend::Machine,
    hub: &Path,
    turbo: bool,
    say: Say<'_>,
) -> Result<()> {
    let arch = std::env::consts::ARCH;
    fits(std::env::consts::OS, arch, crate::engine::read_nvidia()).map_err(anyhow::Error::msg)?;
    let lock = lock_for(arch).context("no lock for this architecture")?;
    let home = &m.mecha_home;
    let dir = dir(home);
    let units = crate::engine_gate::unit_dir(m)?.to_path_buf();
    let unit = units.join(format!("{}.service", naming.unit));
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
        models.push((TURBO_REPO, TURBO_REVISION, &TURBO_FILE));
    }
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
        &format!(
            "torch for CUDA 13 and ComfyUI's requirements (about {:.1} GiB to fetch, about 6.3 \
             GiB on disk)",
            env_bytes(arch).unwrap_or(0) as f64 / (1u64 << 30) as f64
        ),
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

    // --- the unit
    write_owned(home, ID, &unit, &render(&dir, naming), 0o644)?;
    crate::engine_gate::systemctl("daemon-reload", &[])?;
    let service = format!("{}.service", naming.unit);
    crate::engine_gate::systemctl("enable", &[&service])?;
    // A restart, not `enable --now`, so a reinstall reaches a running server;
    // a failed restart or check stops it before the install says so (#639).
    let checked = match crate::engine_gate::systemctl("restart", &[&service]) {
        Ok(()) => {
            say(&format!(
                "starting it on :{} and asking it for every node and model mecha's pictures use",
                naming.port
            ));
            let names: Vec<&str> = models
                .iter()
                .filter_map(|(_, _, f)| f.path.rsplit('/').next())
                .collect();
            check(naming.port, &names).await
        }
        Err(e) => Err(e),
    };
    if let Err(e) = checked {
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
                        "ComfyUI on :{port} did not answer within five minutes — \
                         `journalctl --user -u <its unit>` says why"
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

/// Every file the three loaders offer, off `/object_info`: each loader's
/// first input's choices.
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
    doc.insert("image", toml_edit::Item::Table(table));
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
    let unit = crate::engine_gate::unit_dir(m)?.join(format!("{}.service", naming.unit));
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
    let unit = format!("{}.service", naming.unit);
    let _ = crate::engine_gate::systemctl("disable", &["--now", &unit]);
    let path = crate::engine_gate::unit_dir(m)?.join(&unit);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("removing {}", path.display())),
    }
    Manifest::unrecord(home, ID, &path)?;
    crate::engine_gate::systemctl("daemon-reload", &[])?;
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
    use super::*;

    /// The file fetched on demand is the one a turbo picture loads, by
    /// name, and lands in the directory the unet loader reads.
    #[test]
    fn the_turbo_pin_is_the_file_a_turbo_picture_loads() {
        assert_eq!(
            TURBO_FILE.path,
            format!("diffusion_models/{}", crate::sampling::TURBO_MODEL)
        );
        assert_eq!(
            crate::imagegen::ImageConfig::default().turbo_model,
            crate::sampling::TURBO_MODEL
        );
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
        let none = turbo_wanted(&home, false);
        let asked = turbo_wanted(&home, true);
        persona("ada", "cfg = 3.0");
        let standard = turbo_wanted(&home, false);
        persona("cleo", "model = \"turbo\"");
        persona("bea", "model = \"turbo\"");
        let chosen = turbo_wanted(&home, false);
        let _ = std::fs::remove_dir_all(home.join("personas"));
        let link = dir(&home)
            .join("ComfyUI")
            .join("models")
            .join(TURBO_FILE.path);
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink("/nonexistent/blob", &link).unwrap();
        let linked = turbo_wanted(&home, false);
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(none, None);
        assert!(asked.unwrap().contains("--turbo"));
        assert_eq!(standard, None);
        assert_eq!(
            chosen.as_deref(),
            Some("the picture settings of bea, cleo choose it")
        );
        assert!(
            linked.unwrap().contains("already"),
            "a dangling link still counts"
        );
    }

    /// The install checks exactly the node types mecha's graphs send — built
    /// here, not read off the source: a text-to-image graph, an edit, and a
    /// masked edit, with the default loader and a `.gguf` one. Both ways, so
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
        for wanted in [&d.diffusion_model, &d.text_encoder, &d.vae] {
            assert!(
                names.contains(&wanted.as_str()),
                "{wanted} not in {names:?}"
            );
        }
        for (_, _, f) in models(&machine).unwrap() {
            let top = f.path.split('/').next().unwrap();
            assert!(
                ["diffusion_models", "text_encoders", "vae"].contains(&top),
                "{} is not under a folder ComfyUI reads",
                f.path
            );
        }
    }

    #[test]
    fn only_a_cuda_13_linux_machine_fits() {
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
        assert!(fits("macos", "aarch64", None)
            .unwrap_err()
            .contains("launchd"));
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
            assert!(env_bytes(arch).is_some());
        }
    }

    /// Each lock names the commit it was compiled for, and it is `COMMIT`:
    /// a bump that moves the constant and not the locks fails here, not on
    /// an owner's machine (review of #645).
    #[test]
    fn the_locks_were_compiled_for_this_commit() {
        for lock in [LOCK_AARCH64, LOCK_X86_64] {
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
        seeded.push((TURBO_REPO, TURBO_REVISION, &TURBO_FILE));
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
        let cleaned = remove(&m, &naming);
        eprintln!("{said:#?}");
        let man = Manifest::read(&m.mecha_home);
        let _ = std::fs::remove_dir_all(&root);
        r.unwrap();
        cleaned.unwrap();
        assert_eq!(stamped.unwrap(), COMMIT);
        assert!(turbo, "the turbo model was linked");
        let man = man.unwrap();
        assert!(
            !man.entries.iter().any(|e| e.sidecar == ID),
            "{:?}",
            man.entries
        );
    }
}
