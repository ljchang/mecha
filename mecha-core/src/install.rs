//! Installing what a feature runs (FEATURES-DESIGN.md §10.2, step 7a-3): a
//! pinned `uv`, and the first installer — layout's environment and model.
//!
//! **Every source is a pin.** `uv` is a release asset per platform, checked
//! against a sha256 a reviewer committed; layout's packages come from a
//! `--require-hashes` lock shipped in the binary; its model is the
//! `recommend` registry's Hugging Face pin. `uv` fetches the Python it builds
//! the environment on, verified against the checksums its own pinned version
//! carries (`UV_PYTHON_PREFERENCE=only-managed`, so no system Python's
//! version is ever a requirement).
//!
//! **The record leads the bytes.** The manifest entry is written before the
//! first byte and each path before it is written ([`Manifest::begin`],
//! [`Manifest::record`]), and the entry is complete only after the health
//! check ([`Manifest::finish`]) — so an interrupted install reads resumable,
//! and running it again carries on.
//!
//! **The lock, not the index, is the control.** `uv pip install` honours
//! whatever package index the environment names (`UV_INDEX_URL`,
//! `PIP_INDEX_URL`); `--require-hashes` makes that harmless, since a file
//! whose sha256 is not in the lock is refused wherever it came from.
//!
//! **One tree per install.** Layout's environment, the Python under it and
//! uv's cache all live in `~/.mecha/sidecars/layout/`, so removing it removes
//! everything it put there, and the confined worker's readable roots (the
//! environment and its interpreter's prefix) are inside it and never contain
//! the mecha home.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use crate::feature::Feature;
use crate::recommend::{self, Source};
use crate::sidecar::{FileState, Machinery, Manifest, Plan, PlannedSidecar, SidecarState};

/// The `uv` mecha fetches when the machine has none on `PATH`.
pub const UV_VERSION: &str = "0.12.23";

/// One `uv` release asset per platform: (target, asset, sha256, bytes). The
/// sha256s were read from GitHub's per-asset `digest` when the pin was
/// authored; only these committed values are trusted at install time.
const UV_ASSETS: &[(&str, &str, &str, u64)] = &[
    (
        "aarch64-unknown-linux-gnu",
        "uv-aarch64-unknown-linux-gnu.tar.gz",
        "6524bd338177ed50d035d39354e12545e993bbeba2ecbddf0480c5b3a81d313f",
        18_965_616,
    ),
    (
        "x86_64-unknown-linux-gnu",
        "uv-x86_64-unknown-linux-gnu.tar.gz",
        "9167d72b3319674b6303c4cbe071854bba13ebdf3d76b1a7cbdc175471fb66d6",
        19_906_150,
    ),
    (
        "aarch64-apple-darwin",
        "uv-aarch64-apple-darwin.tar.gz",
        "50487ae565ccd96e499056b4674d438f4c53170202617b4c759defe0c6a1b544",
        17_044_217,
    ),
    (
        "x86_64-apple-darwin",
        "uv-x86_64-apple-darwin.tar.gz",
        "960da44cb4b73685206ddd250b19e0a117fa41095710c1038f081f5cb613efb4",
        21_263_712,
    ),
];

/// Layout's packages, every file pinned by hash. The same file as
/// `scripts/layout/requirements.txt` (a test holds them equal); kept in the
/// crate because a published crate cannot reach outside its own directory.
const LAYOUT_LOCK: &str = include_str!("../locks/layout-requirements.txt");

/// The Python version the lock was compiled for.
const LAYOUT_PYTHON: &str = "3.12";

/// This machine's `uv` target, if mecha pins one for it.
pub fn uv_target() -> Option<&'static str> {
    match (std::env::consts::ARCH, std::env::consts::OS) {
        ("aarch64", "linux") => Some("aarch64-unknown-linux-gnu"),
        ("x86_64", "linux") => Some("x86_64-unknown-linux-gnu"),
        ("aarch64", "macos") => Some("aarch64-apple-darwin"),
        ("x86_64", "macos") => Some("x86_64-apple-darwin"),
        _ => None,
    }
}

fn uv_asset(target: &str) -> Option<(&'static str, &'static str, u64)> {
    UV_ASSETS
        .iter()
        .find(|(t, ..)| *t == target)
        .map(|(_, asset, sha, bytes)| (*asset, *sha, *bytes))
}

/// The sidecars `features enable` can install today — the rest name the step
/// that brings their installer. The router is not among them: the chat model
/// is chosen and installed by `mecha setup chat` alone (ruling F13).
pub fn installable(id: &str) -> bool {
    match id {
        "layout" | "llama" => true,
        // systemd user units: on macOS they stay manual (§10.5).
        "embed-server" | "ocr-server" => cfg!(target_os = "linux"),
        _ => false,
    }
}

/// Whether a sidecar's install brings the models it serves. Layout's and the
/// on-demand servers' fetch theirs; the engine's does not — so a model gone
/// from the hub is a reason to run their installs again, never the engine's.
/// (The router's model is `mecha setup chat`'s to fetch again, F13.)
fn fetches_models(id: &str) -> bool {
    matches!(id, "layout" | "embed-server" | "ocr-server")
}

/// Whether the chat model is served from this machine: the default provider
/// is `local` and names no address, or a loopback one — the same reading of
/// "this machine" the router's follow uses (`provider::router::is_loopback`).
/// A machine that chats through a hosted provider, or a llama-server
/// elsewhere, needs no engine for chat.
pub fn chat_runs_here(cfg: &crate::config::Config) -> bool {
    let Some(p) = cfg.providers.get(&cfg.default_provider) else {
        return false;
    };
    if p.kind != "local" {
        return false;
    }
    match p.base_url.as_deref() {
        None => true,
        Some(url) => crate::provider::router::is_loopback(url),
    }
}

/// Why `enable` does not install a sidecar for a feature, or `None` when it
/// does. The chat model is `mecha setup chat`'s (ruling F13, 2026-10-10): the
/// model is chosen there — the recommended row or a GGUF the owner brings —
/// and `enable web` on a fresh machine must not stand between the owner and
/// a switch with a ~22 GiB download. So the router is never `enable`'s, and a
/// shared sidecar (the engine) is needed only by a feature whose embeddings or
/// OCR server it runs; `chat_here` chooses which reason is given. A function
/// of the feature's slots, not of a plan, so `enable` can ask it before
/// reading the machine.
pub fn not_needed(id: &str, feature: Feature, chat_here: bool) -> Option<&'static str> {
    if id == crate::router_unit::ID {
        return Some(if chat_here {
            "the chat model's — `mecha setup chat` installs it, and the model is chosen there"
        } else {
            "not needed here — the chat model is served from elsewhere"
        });
    }
    let s = crate::sidecar::SIDECARS.iter().find(|s| s.id == id)?;
    if !s.needed_by.is_empty() {
        return None;
    }
    let other_server = crate::sidecar::needed_slots(feature)
        .any(|slot| slot.id != "chat" && s.serves.contains(&slot.id));
    (!other_server).then_some(if chat_here {
        "for the chat model, `mecha setup chat` installs it — this feature runs nothing else \
         on it"
    } else {
        "not needed here — the chat model is served from elsewhere, and this feature runs \
         nothing else on it"
    })
}

/// Whether `enable` could offer anything for a feature — asked before the
/// machine is read, so a switch that installs nothing (`enable messages` on
/// a machine chatting through a hosted provider) probes no GPU.
pub fn may_offer(feature: Feature, chat_here: bool) -> bool {
    crate::sidecar::needed(feature)
        .any(|s| installable(s.id) && not_needed(s.id, feature, chat_here).is_none())
}

/// Price what a plan would install whose size is known before installing:
/// the engine's pinned archives for this machine, on its line and in the
/// total. `nvidia` is read only when the engine is to be installed, so a plan
/// with nothing to fetch probes no driver. A machine `engine::choose` refuses
/// is priced at nothing, and its install says why.
pub fn price(plan: &mut Plan, chat_here: bool, nvidia: impl FnOnce() -> crate::engine::Nvidia) {
    let feature = plan.feature;
    let Some(s) = plan.sidecars.iter_mut().find(|s| {
        s.id == "llama"
            && matches!(
                s.state,
                SidecarState::Missing { .. } | SidecarState::Incomplete
            )
            && not_needed(s.id, feature, chat_here).is_none()
    }) else {
        return;
    };
    if let Ok(target) =
        crate::engine::choose(std::env::consts::OS, std::env::consts::ARCH, nvidia())
    {
        let bytes = crate::engine::download_bytes(target);
        s.bytes = Some(bytes);
        plan.download_bytes += bytes;
    }
}

/// What `mecha features enable` offers to install for one plan: each
/// installable sidecar that is missing or unfinished — or installed, with a
/// model its own install fetches now gone from the hub (a cleared cache
/// leaves its link dangling; the install is idempotent, so running it again
/// fetches only what is missing) — and needed here.
pub fn offered(plan: &Plan, chat_here: bool) -> Vec<&PlannedSidecar> {
    plan.sidecars
        .iter()
        .filter(|s| {
            let serves = crate::sidecar::SIDECARS
                .iter()
                .find(|sc| sc.id == s.id)
                .map(|sc| sc.serves)
                .unwrap_or(&[]);
            let model_gone = fetches_models(s.id)
                && plan.files.iter().any(|f| {
                    serves.contains(&f.slot) && matches!(f.state, FileState::Download { .. })
                });
            let wanted = matches!(
                s.state,
                SidecarState::Missing { .. } | SidecarState::Incomplete
            ) || (matches!(s.state, SidecarState::Installed) && model_gone);
            wanted && installable(s.id) && not_needed(s.id, plan.feature, chat_here).is_none()
        })
        .collect()
}

/// What an install says as it goes, for the terminal.
pub type Say<'a> = &'a mut dyn FnMut(&str);

/// A `uv` to build environments with: one on `PATH` outside mecha's tree when
/// the machine has it (§10.2 item 5), else mecha's pinned copy, fetched and
/// unpacked into `~/.mecha/sidecars/uv/` on first use.
pub async fn ensure_uv(m: &Machinery, say: Say<'_>) -> Result<PathBuf> {
    let sidecars = m.mecha_home.join("sidecars");
    // A uv on PATH is used when it is new enough to honour what the install
    // relies on (`UV_PYTHON_PREFERENCE`, `--require-hashes`): an older one
    // would ignore the variable silently and build on a system Python, so it
    // is passed over for the pinned copy rather than trusted.
    for d in &m.path {
        let p = d.join("uv");
        if p.is_file() && !p.starts_with(&sidecars) {
            match uv_version(&p).and_then(|v| parse_version(&v)) {
                Some(v) if v >= UV_FLOOR => return Ok(p),
                Some(_) => say(&format!(
                    "{} is older than uv {}.{}; using mecha's pinned copy",
                    p.display(),
                    UV_FLOOR.0,
                    UV_FLOOR.1
                )),
                None => say(&format!(
                    "{} did not answer `--version`; using mecha's pinned copy",
                    p.display()
                )),
            }
            break;
        }
    }
    let dir = sidecars.join("uv");
    let bin = dir.join("uv");
    // Ask the cached binary, not the record: a later pin must replace it.
    if bin.is_file() && uv_version(&bin).is_some_and(|v| is_pinned_uv(&v)) {
        // An unpack interrupted before its `finish` left the binary whole
        // and the record open; the version check is the health check, so
        // the record catches up here rather than reading unfinished forever.
        if Manifest::read(&m.mecha_home)?
            .entries
            .iter()
            .any(|e| e.sidecar == "uv" && e.incomplete)
        {
            Manifest::finish(&m.mecha_home, "uv")?;
        }
        return Ok(bin);
    }
    let target =
        uv_target().context("mecha pins no uv for this platform; install uv and run this again")?;
    let (asset, sha, bytes) = uv_asset(target).context("no uv asset pinned for this platform")?;
    Manifest::begin(&m.mecha_home, "uv")?;
    Manifest::record(&m.mecha_home, "uv", &dir)?;
    say(&format!(
        "fetching uv {UV_VERSION} ({:.0} MiB)",
        bytes as f64 / 1_048_576.0
    ));
    let archive = crate::fetch::fetch_release_asset(
        "astral-sh/uv",
        UV_VERSION,
        asset,
        sha,
        bytes,
        &dir.join("download"),
        &mut |_| {},
    )
    .await?;
    // The archive holds one directory, `uv-<target>/`, with `uv` and `uvx`.
    let status = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(&dir)
        .arg("--strip-components=1")
        .status()
        .context("running tar to unpack uv")?;
    if !status.success() {
        bail!("tar could not unpack {}", archive.display());
    }
    let version = uv_version(&bin).unwrap_or_default();
    if !is_pinned_uv(&version) {
        bail!(
            "the unpacked uv reports `{}`, not {UV_VERSION}",
            version.trim()
        );
    }
    // The archive is verified and unpacked; it is no use kept.
    let _ = std::fs::remove_dir_all(dir.join("download"));
    Manifest::finish(&m.mecha_home, "uv")?;
    Ok(bin)
}

/// The oldest `uv` taken from `PATH`: it must know `UV_PYTHON_PREFERENCE`
/// and `--require-hashes` (both well before 0.5).
const UV_FLOOR: (u32, u32, u32) = (0, 5, 0);

/// `uv 0.11.7 (…)` → (0, 11, 7).
/// `uv 0.12.23 (…)` is the pin; `uv 0.12.230` and `uv 0.12.2` are not —
/// compared as numbers, never as a substring.
fn is_pinned_uv(text: &str) -> bool {
    parse_version(text).is_some()
        && parse_version(text) == parse_version(&format!("uv {UV_VERSION}"))
}

fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let v = text.split_whitespace().nth(1)?;
    let mut it = v.split('.').map(|n| n.parse::<u32>().ok());
    Some((it.next()??, it.next()??, it.next().flatten().unwrap_or(0)))
}

/// `uv --version`, or `None` when it cannot be run.
fn uv_version(bin: &Path) -> Option<String> {
    let out = std::process::Command::new(bin)
        .arg("--version")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Where mecha installs layout, and what `document.rs` finds there.
pub fn layout_dir(mecha_home: &Path) -> PathBuf {
    mecha_home.join("sidecars/layout")
}

/// Install layout: the environment (Python 3.12 by `uv`, the hash-locked
/// packages) and the pinned model, then the check `scripts/layout/install.sh`
/// runs — the model loads in that environment. Run again after an
/// interruption, it carries on.
pub async fn install_layout(
    m: &Machinery,
    machine: &recommend::Machine,
    hub: &Path,
    say: Say<'_>,
) -> Result<()> {
    let id = "layout";
    let home = &m.mecha_home;
    let dir = layout_dir(home);
    Manifest::begin(home, id)?;
    Manifest::record(home, id, &dir)?;
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }

    let uv = ensure_uv(m, say).await?;
    let venv = dir.join("venv");
    let python = venv.join("bin/python");
    let link = dir.join("PP-DocLayoutV3.onnx");
    // Each piece on its own line in the record, so a partly deleted install
    // reads incomplete — resumable — rather than installed.
    Manifest::record(home, id, &venv)?;
    // The managed Python the venv's interpreter links into: unrecorded,
    // removing it would leave `venv/bin/python` dangling while the record
    // read installed — `plan` does not follow the link, and
    // `document::layout_tree` does, so they would disagree with no way out.
    Manifest::record(home, id, &dir.join("python"))?;
    Manifest::record(home, id, &link)?;
    let uv_env = |c: &mut std::process::Command| {
        c.env("UV_PYTHON_INSTALL_DIR", dir.join("python"))
            .env("UV_CACHE_DIR", dir.join(".uv-cache"))
            .env("UV_PYTHON_PREFERENCE", "only-managed")
            .env("UV_NO_CONFIG", "1")
            // `only-managed` needs the download; an operator's `never` would
            // turn the install into a confusing failure.
            .env("UV_PYTHON_DOWNLOADS", "automatic");
    };
    if !python.exists() {
        say(&format!("building a Python {LAYOUT_PYTHON} environment"));
        // `--clear`: a venv whose interpreter dangles (its managed Python
        // removed) is still a directory, and uv will not build over one
        // without being told. It is mecha's own tree, and the install below
        // runs every time, so nothing in it is lost.
        let mut c = std::process::Command::new(&uv);
        c.args(["venv", "--quiet", "--clear", "--python", LAYOUT_PYTHON])
            .arg(&venv);
        uv_env(&mut c);
        run(&mut c, "uv venv")?;
    }
    let lock = dir.join("requirements.txt");
    std::fs::write(&lock, LAYOUT_LOCK)?;
    say("installing onnxruntime and numpy, every file checked against the lock");
    let mut c = std::process::Command::new(&uv);
    c.args(["pip", "install", "--quiet", "--require-hashes", "--python"])
        .arg(&python)
        .arg("-r")
        .arg(&lock);
    uv_env(&mut c);
    run(&mut c, "uv pip install")?;

    let slot = recommend::SLOTS
        .iter()
        .find(|s| s.id == "layout")
        .context("no layout slot")?;
    let Some(Source::HuggingFace {
        repo,
        revision,
        files,
    }) = recommend::row_for(slot, machine).and_then(|(r, _)| r.sources.first())
    else {
        bail!("the layout slot pins no Hugging Face file");
    };
    let file = files.first().context("the layout pin names no file")?;
    say(&format!(
        "fetching the layout model ({:.0} MiB)",
        file.bytes as f64 / 1_048_576.0
    ));
    let snap = crate::fetch::fetch_hub_file(hub, repo, revision, file, &mut |_| {}).await?;
    // Swapped in one rename, so a run interrupted here leaves the old link
    // or the new one, never none.
    let tmp = dir.join("PP-DocLayoutV3.onnx.new");
    let _ = std::fs::remove_file(&tmp);
    #[cfg(unix)]
    std::os::unix::fs::symlink(&snap, &tmp)?;
    #[cfg(not(unix))]
    std::fs::copy(&snap, &tmp).map(|_| ())?;
    std::fs::rename(&tmp, &link)?;

    say("checking the model loads");
    let mut c = std::process::Command::new(&python);
    c.args([
        "-I",
        "-B",
        "-c",
        "import onnxruntime as ort, sys; ort.InferenceSession(sys.argv[1], providers=['CPUExecutionProvider']); print(ort.__version__)",
    ])
    .arg(&link);
    run(&mut c, "the layout health check")?;
    Manifest::finish(home, id)?;
    Ok(())
}

/// Run a step; a failure is an error carrying its own output.
fn run(c: &mut std::process::Command, what: &str) -> Result<()> {
    let out = c.output().with_context(|| format!("starting {what}"))?;
    if !out.status.success() {
        bail!(
            "{what} failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Install one sidecar by id — the installers built so far.
pub async fn install(
    id: &str,
    m: &Machinery,
    machine: &recommend::Machine,
    hub: &Path,
    say: Say<'_>,
) -> Result<()> {
    match id {
        "layout" => install_layout(m, machine, hub, say).await,
        "embed-server" | "ocr-server" => {
            let which = if id == "embed-server" {
                crate::llama_units::Which::Embeddings
            } else {
                crate::llama_units::Which::Ocr
            };
            crate::llama_units::install(m, which, &which.shipped(), machine, hub, say).await
        }
        "llama" => {
            let server = crate::engine::install_engine(m, say).await?;
            say(&format!(
                "the engine is at {} — the router's, embeddings' and OCR servers' units name it",
                server.display()
            ));
            Ok(())
        }
        other => bail!("mecha cannot install {other} yet"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_of(feature: Feature, llama: SidecarState) -> Plan {
        Plan {
            feature,
            sidecars: vec![PlannedSidecar {
                id: "llama",
                label: "llama.cpp (llama-server)",
                state: llama,
                bytes: None,
            }],
            files: vec![],
            download_bytes: 0,
            nothing_to_do: false,
        }
    }

    /// The engine is offered where it runs something: the chat model here,
    /// or a feature's embeddings or OCR server. A machine chatting through a
    /// hosted provider is not handed 700 MB for `enable messages` — nor made
    /// to wait on a GPU probe for it.
    #[test]
    fn the_engine_is_offered_only_where_it_runs_something() {
        let missing = || SidecarState::Missing { step: "7b" };
        // Chat's engine is `setup chat`'s (F13): messages runs nothing
        // else, so `enable messages` installs nothing, chat here or not.
        let messages = plan_of(Feature::Messages, missing());
        assert!(install_ids(&messages, true).is_empty());
        assert!(install_ids(&messages, false).is_empty());
        assert!(not_needed("llama", Feature::Messages, false).is_some());
        assert!(not_needed("llama", Feature::Messages, true).is_some());
        assert!(!may_offer(Feature::Messages, false));
        assert!(!may_offer(Feature::Messages, true));
        // Documents runs an OCR server on the engine; graph an embeddings one.
        let documents = plan_of(Feature::Documents, missing());
        assert_eq!(install_ids(&documents, false), vec!["llama"]);
        assert_eq!(install_ids(&documents, true), vec!["llama"]);
        assert!(not_needed("llama", Feature::Graph, false).is_none());
        assert!(may_offer(Feature::Graph, false));
        // The router is never `enable`'s, chat here or not (F13); here it
        // names the command that installs it. A feature's own sidecar
        // always is `enable`'s.
        assert!(not_needed("router", Feature::Documents, false).is_some());
        let here = not_needed("router", Feature::Documents, true).unwrap();
        assert!(here.contains("mecha setup chat"), "{here}");
        assert!(!installable("router"));
        assert!(not_needed("layout", Feature::Messages, false).is_none());
    }

    /// The engine's install fetches no model, so a model missing from the hub
    /// never re-offers an installed engine — it would offer, install nothing
    /// new, and offer again on every enable.
    #[test]
    fn a_missing_model_does_not_re_offer_the_engine() {
        let mut installed = plan_of(Feature::Documents, SidecarState::Installed);
        installed.files.push(crate::sidecar::PlannedFile {
            slot: "ocr",
            model: "m",
            repo: Some("org/r"),
            path: "f.gguf",
            state: FileState::Download { bytes: 1 },
        });
        assert!(install_ids(&installed, true).is_empty());
        let unfinished = plan_of(Feature::Documents, SidecarState::Incomplete);
        assert_eq!(install_ids(&unfinished, true), vec!["llama"]);
    }

    /// The engine's archives are on its line and in the total — what `enable`
    /// is about to fetch, shown before the yes — and only when it is to be
    /// installed: a provided or unneeded engine probes no driver.
    #[test]
    fn the_engine_s_download_is_in_the_plan() {
        use crate::engine::Nvidia;
        let mut p = plan_of(Feature::Documents, SidecarState::Missing { step: "7b" });
        p.download_bytes = 5;
        price(&mut p, true, || Nvidia::None);
        let b = p.sidecars[0].bytes.expect("priced");
        assert!(b > 1_000_000, "{b}");
        assert_eq!(p.download_bytes, 5 + b);

        let mut provided = plan_of(
            Feature::Documents,
            SidecarState::Provided { by: "x".into() },
        );
        price(&mut provided, true, || {
            panic!("probed for a provided engine")
        });
        assert_eq!(provided.sidecars[0].bytes, None);
        let mut unneeded = plan_of(Feature::Messages, SidecarState::Missing { step: "7b" });
        price(&mut unneeded, false, || {
            panic!("probed for an unneeded engine")
        });
        assert_eq!(unneeded.download_bytes, 0);
    }

    fn install_ids(p: &Plan, chat_here: bool) -> Vec<&'static str> {
        offered(p, chat_here).iter().map(|s| s.id).collect()
    }

    #[test]
    fn chat_runs_here_only_for_a_local_provider_on_this_machine() {
        let with = |kind: &str, url: Option<&str>| {
            let mut cfg = crate::config::Config::default();
            let p = cfg
                .providers
                .get_mut(&cfg.default_provider.clone())
                .unwrap();
            p.kind = kind.into();
            p.base_url = url.map(str::to_owned);
            chat_runs_here(&cfg)
        };
        assert!(with("local", None));
        assert!(with("local", Some("http://127.0.0.1:8080/v1")));
        assert!(with("local", Some("http://localhost:8080/v1")));
        // All of 127/8 is this machine: Debian names its hostname 127.0.1.1.
        assert!(with("local", Some("http://127.0.1.1:8080/v1")));
        assert!(with("local", Some("http://[::1]:8080/v1")));
        assert!(!with("local", Some("http://gpu-box.tailnet:8080/v1")));
        assert!(!with("anthropic", None));
        assert!(!with("openai", Some("http://127.0.0.1:8080/v1")));
    }

    /// The lock in the binary is the lock the script installs from: one
    /// file's two copies, held equal so they cannot drift.
    #[test]
    fn the_shipped_layout_lock_is_the_script_s() {
        let script = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/layout/requirements.txt"),
        )
        .unwrap();
        assert_eq!(LAYOUT_LOCK, script);
        assert!(
            LAYOUT_LOCK.contains("--hash=sha256:"),
            "a lock without hashes is not a lock"
        );
    }

    #[test]
    fn a_uv_version_is_read_and_held_to_the_floor() {
        assert_eq!(
            parse_version("uv 0.11.7 (aarch64-unknown-linux-gnu)"),
            Some((0, 11, 7))
        );
        assert_eq!(parse_version("uv 0.12.23"), Some((0, 12, 23)));
        assert!(parse_version("uv 0.4.30").unwrap() < UV_FLOOR);
        assert!(parse_version("uv 0.11.7").unwrap() >= UV_FLOOR);
        assert_eq!(parse_version("not uv"), None);
    }

    #[test]
    fn every_uv_asset_is_pinned_and_every_target_has_one() {
        for (target, asset, sha, bytes) in UV_ASSETS {
            assert_eq!(*asset, format!("uv-{target}.tar.gz"));
            assert!(
                sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
                "{target}"
            );
            assert!(*bytes > 0);
        }
        for t in [
            "aarch64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
        ] {
            assert!(uv_asset(t).is_some(), "{t}");
        }
        if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
            assert_eq!(uv_target(), Some("aarch64-unknown-linux-gnu"));
        }
    }

    /// What `install` can install and what the registry says arrives in a
    /// later step agree, both ways: a sidecar is installable exactly when its
    /// step is one already built.
    #[test]
    fn only_the_pinned_uv_is_the_pin() {
        assert!(is_pinned_uv(&format!("uv {UV_VERSION} (abc 2026-09-30)")));
        assert!(!is_pinned_uv(&format!("uv {UV_VERSION}0")));
        assert!(!is_pinned_uv("uv 0.12.2"));
        assert!(!is_pinned_uv(""));
    }

    #[test]
    fn installable_and_the_registry_agree() {
        // The on-demand servers' installer is built, for systemd machines.
        let built: &[&str] = if cfg!(target_os = "linux") {
            &["7a-3", "7b", "7c-1", "7c-2"]
        } else {
            &["7a-3", "7b"]
        };
        assert!(installable("layout"));
        assert!(installable("llama"));
        assert!(!installable("comfyui"));
        assert!(!installable("router"), "the router is `setup chat`'s (F13)");
        for s in crate::sidecar::SIDECARS {
            // The router's installer is built, and `setup chat` runs it —
            // never `enable` (ruling F13).
            if s.id == crate::router_unit::ID {
                continue;
            }
            assert_eq!(
                installable(s.id),
                built.contains(&s.installer),
                "{} is installable: {}, but its step says {}",
                s.id,
                installable(s.id),
                s.installer
            );
        }
    }

    const GB10: recommend::Machine = recommend::Machine::Unified {
        total_mb: 124_610,
        gpu_unread: false,
    };

    fn which_bwrap() -> bool {
        std::process::Command::new("bwrap")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    /// The real install, end to end, into a scratch mecha home and hub:
    /// fetches uv (or uses one on PATH), Python 3.12, the locked packages and
    /// the 130 MB model, then the health check — and a second run is a
    /// no-op that still passes. `cargo test -p mecha-core --lib install -- --ignored`.
    #[tokio::test]
    #[ignore = "network: fetches uv, a Python build, onnxruntime and the layout model"]
    async fn layout_installs_end_to_end_and_again() {
        let root = std::env::temp_dir().join(format!("mecha-install-{}", uuid::Uuid::new_v4()));
        let m = Machinery {
            home: root.join("home"),
            mecha_home: root.join("home/.mecha"),
            unit_dirs: vec![],
            path: vec![],
            docker: Box::new(|_| crate::sidecar::Lookup::Absent),
        };
        let hub = root.join("hub");
        let mut log = Vec::new();
        install_layout(&m, &GB10, &hub, &mut |s| log.push(s.to_string()))
            .await
            .unwrap();
        let dir = layout_dir(&m.mecha_home);
        assert!(dir.join("venv/bin/python").exists());
        assert!(dir.join("PP-DocLayoutV3.onnx").exists());
        let man = Manifest::read(&m.mecha_home).unwrap();
        assert!(man
            .entries
            .iter()
            .any(|e| e.sidecar == "layout" && !e.incomplete));
        assert!(man
            .entries
            .iter()
            .any(|e| e.sidecar == "uv" && !e.incomplete));
        // The confined worker takes it: every readable root is inside the
        // install or the hub, none contains a home, and under bwrap the
        // worker starts and loads the model.
        let layout = crate::layout::LayoutModel::new(
            dir.join("venv/bin/python"),
            dir.join("PP-DocLayoutV3.onnx"),
            crate::sandbox::Backend::Bwrap,
            4096,
            2,
            std::time::Duration::from_secs(60),
        );
        let located = layout.locate().unwrap();
        for r in &located.readable {
            assert!(
                r.starts_with(&dir) || r.starts_with(&hub),
                "readable root outside the install: {}",
                r.display()
            );
        }
        if which_bwrap() {
            let child = layout.start().await.unwrap();
            drop(child);
        }
        // Again: everything is there, and it still passes its check.
        install_layout(&m, &GB10, &hub, &mut |_| {}).await.unwrap();
        // The managed Python removed leaves the venv's interpreter dangling:
        // the plan reads that resumable, and the install repairs it.
        std::fs::remove_dir_all(dir.join("python")).unwrap();
        let state = |m: &Machinery| {
            crate::sidecar::plan(crate::feature::Feature::Documents, m, &GB10, &hub, false)
                .unwrap()
                .sidecars
                .into_iter()
                .find(|s| s.id == "layout")
                .unwrap()
                .state
        };
        assert!(
            matches!(state(&m), crate::sidecar::SidecarState::Incomplete),
            "{:?}",
            state(&m)
        );
        install_layout(&m, &GB10, &hub, &mut |_| {}).await.unwrap();
        assert!(dir.join("venv/bin/python").exists());
        assert!(matches!(state(&m), crate::sidecar::SidecarState::Installed));
        let _ = std::fs::remove_dir_all(&root);
    }
}
