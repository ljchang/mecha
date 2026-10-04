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

use crate::recommend::{self, Source};
use crate::sidecar::{Machinery, Manifest};

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

/// The sidecars mecha can install today — the rest name the step that brings
/// their installer.
pub fn installable(id: &str) -> bool {
    id == "layout"
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
    if bin.is_file() && uv_version(&bin).is_some_and(|v| v.contains(UV_VERSION)) {
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
    if !version.contains(UV_VERSION) {
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
        other => bail!("mecha cannot install {other} yet"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    /// later step agree: an installable sidecar is a registered one whose step
    /// is this one.
    #[test]
    fn installable_and_the_registry_agree() {
        assert!(installable("layout"));
        assert!(!installable("comfyui"));
        assert!(crate::sidecar::SIDECARS.iter().any(|s| s.id == "layout"));
        for s in crate::sidecar::SIDECARS {
            if installable(s.id) {
                assert_eq!(
                    s.installer, "7a-3",
                    "{} is installable but its step says {}",
                    s.id, s.installer
                );
            }
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
