//! The llama.cpp engine, installed from a pinned release (`docs/FEATURES-DESIGN.md`
//! §10.3, step 7b-1).
//!
//! One engine serves the router, the embeddings server and the OCR server, so
//! it is the sidecar every local feature stands on. It is installed **side by
//! side**: each build in `~/.mecha/sidecars/llama/<tag>/`, and a `current` link
//! naming the one the servers run — so a later upgrade is a new directory and
//! a link swap, and the previous build stays for a rollback (7b-3).
//!
//! **Which asset** is decided from the machine, never from config: the OS, the
//! architecture, and — for NVIDIA — the driver's version from `nvidia-smi`,
//! because a CUDA 13 build needs a 580 driver and a CUDA 12 build a 525 one
//! (minor-version compatibility, which is how CUDA 13.4's runtime ran on this
//! box's 580 driver, CUDA 13.0, on 2026-10-04). A machine that shows an NVIDIA
//! device but whose `nvidia-smi` cannot be read gets **no** engine rather than
//! a CPU one: installing the CPU build there would be a GPU machine silently
//! answering at a tenth of the speed, with nothing to say why.
//!
//! **What is trusted** is only the committed sha256 of each asset (§10.2 item
//! 1); a CUDA asset's runtime libraries come in a second, separately pinned
//! archive (`cudart-…`), unpacked into the same directory.
//!
//! **What is checked** before the build counts as installed:
//! - every ELF in the directory names its libraries `$ORIGIN`-relative, never by
//!   an absolute path — the from-source failure §10.3 records, where a build
//!   tree's path baked into RUNPATH made a copied engine run the old libraries;
//! - `llama-server --version` reports the pinned build and commit, run with
//!   `LD_LIBRARY_PATH` cleared so the libraries beside it are the ones it loads;
//! - a GPU build lists a GPU in `--list-devices`. A CUDA build that finds no
//!   device still runs — on the CPU — so "it started" proves nothing here.
//!
//! macOS assets are Mach-O and resolve through `@loader_path`; the RUNPATH
//! check is ELF-only, and the version and device checks still run there.
//!
//! The RUNPATH check reads the build directory's top level only, because the
//! release is flat — every binary and library side by side. A future pin
//! that nested libraries would fail closed rather than pass: `llama-server`
//! would need a RUNPATH that climbs (`$ORIGIN/../lib`), which
//! `within_origin` refuses, and the CUDA runtime would not be found beside
//! it. Widening the walk belongs with such a pin.
//!
//! `~/.mecha/sidecars/llama/` holds more than builds — the `download/`
//! cache, a `<tag>.part` being checked, `current.new` mid-swap — so the
//! builds on disk are listed from the manifest's record (`Entry::builds`),
//! never from the directory: 7b-3's upgrade and rollback read them there.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::install::Say;
use crate::sidecar::{Machinery, Manifest};

/// The shipped engine: the newest the project has measured (§10.3). Moved by a
/// reviewed change; `--upgrade` (7b-3) is the owner's way past it.
pub const PIN: Pin = Pin {
    tag: "b11391",
    build: 11391,
    commit: "2bc563573479d53b30b8793039485887bc0fdda8",
    assets: &[
        Asset {
            target: Target::LinuxArm64Cuda13,
            archives: &[
                (
                    "llama-b11391-bin-ubuntu-cuda-13.4-arm64.tar.gz",
                    "5bca8f1039fef06b8972adc06369044de6e04b155e62d6f91a161623d1a279e1",
                    147_596_012,
                ),
                (
                    "cudart-llama-b11391-bin-ubuntu-cuda-13.4-arm64.tar.gz",
                    "3643a1ff6a6dde792b9bf6cf4dcf731b53503355f443f5d0d9ab4f3f4bcd92ab",
                    552_521_398,
                ),
            ],
        },
        Asset {
            target: Target::LinuxX64Cuda13,
            archives: &[
                (
                    "llama-b11391-bin-ubuntu-cuda-13.4-x64.tar.gz",
                    "e414fc5c77adbbb0547e76a5bb38672e611de61c74dec3de409e7de443fa3b0d",
                    152_344_197,
                ),
                (
                    "cudart-llama-b11391-bin-ubuntu-cuda-13.4-x64.tar.gz",
                    "d3b2ffdde468f7247a352385c9c92871844e545ad47069bcb8d2adc153c5b10c",
                    440_236_642,
                ),
            ],
        },
        Asset {
            target: Target::LinuxX64Cuda12,
            archives: &[
                (
                    "llama-b11391-bin-ubuntu-cuda-12.8-x64.tar.gz",
                    "3a76d8d1b0f3d558d0de2d060283f2da0afac79a9f68dd1486b987b9fedc5bc5",
                    171_499_631,
                ),
                (
                    "cudart-llama-b11391-bin-ubuntu-cuda-12.8-x64.tar.gz",
                    "8cd7789d811ed44d8b5ccc53feb7c1e2a0e3ab92e8e3fc450f43282fb2c81a87",
                    594_377_794,
                ),
            ],
        },
        Asset {
            target: Target::LinuxArm64Cpu,
            archives: &[(
                "llama-b11391-bin-ubuntu-arm64.tar.gz",
                "76f0ede98765d74e6d2e02ce8f0f22f4cbed5c4411ebca8594d13c076d0bc7fb",
                13_667_865,
            )],
        },
        Asset {
            target: Target::LinuxX64Cpu,
            archives: &[(
                "llama-b11391-bin-ubuntu-x64.tar.gz",
                "b620b4143ab92b050e435209d612415c0d3b19d71e6afd60fa6b96458fae891b",
                17_637_234,
            )],
        },
        Asset {
            target: Target::MacArm64,
            archives: &[(
                "llama-b11391-bin-macos-arm64.tar.gz",
                "25d2bb54a394de7c75d11d4b4a678f600704349c5e5a5366443f746d862df27a",
                11_915_952,
            )],
        },
        Asset {
            target: Target::MacX64,
            archives: &[(
                "llama-b11391-bin-macos-x64.tar.gz",
                "6c6b671a6e54fc50b84ea361e9b2861dd98a542784cca4152ad0c6549931917b",
                11_466_225,
            )],
        },
    ],
};

const REPO: &str = "ggml-org/llama.cpp";

/// A release pinned whole: its tag, the build number and commit
/// `llama-server --version` reports, and one asset per target.
pub struct Pin {
    pub tag: &'static str,
    pub build: u32,
    pub commit: &'static str,
    pub assets: &'static [Asset],
}

/// One target's archives — (name, sha256, bytes) — unpacked in order into one
/// directory.
pub struct Asset {
    pub target: Target,
    pub archives: &'static [(&'static str, &'static str, u64)],
}

/// The release builds mecha installs. Vulkan, ROCm and SYCL builds exist
/// upstream and are not offered yet: a machine that wants one gets the CPU
/// build named as such, and the build fallback (7b-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Target {
    LinuxArm64Cuda13,
    LinuxX64Cuda13,
    LinuxX64Cuda12,
    LinuxArm64Cpu,
    LinuxX64Cpu,
    MacArm64,
    MacX64,
}

impl Target {
    /// What the build runs on, in words.
    pub fn label(self) -> &'static str {
        match self {
            Target::LinuxArm64Cuda13 | Target::LinuxX64Cuda13 => "CUDA 13",
            Target::LinuxX64Cuda12 => "CUDA 12",
            Target::LinuxArm64Cpu | Target::LinuxX64Cpu | Target::MacX64 => "CPU",
            Target::MacArm64 => "Metal",
        }
    }

    /// Whether the build must find a GPU to be what was chosen.
    fn wants_gpu(self) -> bool {
        matches!(
            self,
            Target::LinuxArm64Cuda13
                | Target::LinuxX64Cuda13
                | Target::LinuxX64Cuda12
                | Target::MacArm64
        )
    }
}

/// What the machine shows about its NVIDIA driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nvidia {
    /// No NVIDIA device on the machine.
    None,
    /// `nvidia-smi` answered with this driver version (major, minor).
    Driver(u32, u32),
    /// A device is there (`/proc/driver/nvidia`), but `nvidia-smi` could not
    /// be read — missing, wedged, or answering something unparseable.
    Unread,
}

/// CUDA 13's minimum driver on Linux, and CUDA 12's (minor-version
/// compatibility: a 12.x runtime runs on any 525+ driver).
const CUDA13_DRIVER: (u32, u32) = (580, 0);
const CUDA12_DRIVER: (u32, u32) = (525, 60);

/// The build for a machine, or why there is none. A pure function of what was
/// read, so every row of the table is a test.
pub fn choose(os: &str, arch: &str, nvidia: Nvidia) -> std::result::Result<Target, String> {
    match (os, arch, nvidia) {
        ("macos", "aarch64", _) => Ok(Target::MacArm64),
        ("macos", "x86_64", _) => Ok(Target::MacX64),
        ("linux", _, Nvidia::Unread) => Err(
            "this machine shows an NVIDIA device, but `nvidia-smi` could not be read, so the \
             driver cannot be matched to a build; mecha installs no CPU build in its place — \
             fix `nvidia-smi` and run this again"
                .into(),
        ),
        ("linux", "aarch64", Nvidia::Driver(ma, mi)) if (ma, mi) >= CUDA13_DRIVER => {
            Ok(Target::LinuxArm64Cuda13)
        }
        ("linux", "aarch64", Nvidia::Driver(ma, mi)) => Err(format!(
            "the NVIDIA driver is {ma}.{mi}; the arm64 CUDA build needs 580 or newer, and there \
             is no older arm64 CUDA build to fall back on — update the driver, or wait for the \
             build-from-source installer"
        )),
        ("linux", "x86_64", Nvidia::Driver(ma, mi)) if (ma, mi) >= CUDA13_DRIVER => {
            Ok(Target::LinuxX64Cuda13)
        }
        ("linux", "x86_64", Nvidia::Driver(ma, mi)) if (ma, mi) >= CUDA12_DRIVER => {
            Ok(Target::LinuxX64Cuda12)
        }
        ("linux", "x86_64", Nvidia::Driver(ma, mi)) => Err(format!(
            "the NVIDIA driver is {ma}.{mi}; the oldest CUDA build needs 525.60 or newer — \
             update the driver"
        )),
        ("linux", "aarch64", Nvidia::None) => Ok(Target::LinuxArm64Cpu),
        ("linux", "x86_64", Nvidia::None) => Ok(Target::LinuxX64Cpu),
        (os, arch, _) => Err(format!("mecha pins no llama.cpp build for {os} on {arch}")),
    }
}

/// Read the driver: `nvidia-smi` when it answers, else whether the machine
/// shows an NVIDIA GPU at all — the kernel module's `/proc/driver/nvidia`, or,
/// with no module loaded, an NVIDIA display device on the PCI bus. Without
/// the second, a card whose driver tools are missing would read as no card
/// and be handed the CPU build.
pub fn read_nvidia() -> Nvidia {
    match crate::recommend::nvidia_smi("driver_version") {
        Some(text) => parse_driver(&text).map_or(Nvidia::Unread, |(a, b)| Nvidia::Driver(a, b)),
        None if Path::new("/proc/driver/nvidia").exists()
            || pci_nvidia_display(Path::new("/sys/bus/pci/devices")) =>
        {
            Nvidia::Unread
        }
        None => Nvidia::None,
    }
}

/// Whether any PCI device under `root` is NVIDIA's (vendor `0x10de`) and a
/// display controller (class `0x03xxxx`) — a GPU, not one of the PCIe
/// bridges NVIDIA also makes (`0x0604xx`, which a GB10 shows six of).
fn pci_nvidia_display(root: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    entries.filter_map(|e| e.ok()).any(|e| {
        let read = |f: &str| std::fs::read_to_string(e.path().join(f)).unwrap_or_default();
        read("vendor").trim() == "0x10de" && read("class").trim().starts_with("0x03")
    })
}

/// `580.173.02` → (580, 173). Several cards share one driver; the first line
/// is read.
fn parse_driver(text: &str) -> Option<(u32, u32)> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let mut parts = line.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor))
}

/// What installing the pinned engine for a target downloads, in bytes.
pub fn download_bytes(target: Target) -> u64 {
    asset(target).map_or(0, |a| a.archives.iter().map(|(_, _, b)| b).sum())
}

/// The pinned asset for a target.
pub fn asset(target: Target) -> Option<&'static Asset> {
    PIN.assets.iter().find(|a| a.target == target)
}

/// `~/.mecha/sidecars/llama/`.
pub fn engine_root(mecha_home: &Path) -> PathBuf {
    mecha_home.join("sidecars").join("llama")
}

/// The link the servers run: `~/.mecha/sidecars/llama/current`.
pub fn current(mecha_home: &Path) -> PathBuf {
    engine_root(mecha_home).join("current")
}

/// One build on disk, as the manifest records it: a tag is a label, the
/// commit is what a rollback and an audit need (§10.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Build {
    pub tag: String,
    pub commit: String,
    /// Which asset, kebab-case; an unknown value from a newer mecha reads
    /// as `None` rather than failing the record.
    #[serde(default, deserialize_with = "lenient_target")]
    pub target: Option<Target>,
}

fn lenient_target<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Target>, D::Error> {
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(v.and_then(|v| serde_json::from_value(v).ok()))
}

/// What a build must report, and where its bytes come from.
pub struct Spec {
    pub tag: String,
    pub build: u32,
    pub commit: String,
    pub source: Source,
}

/// A build's archives: the pin's, each by a sha256 a reviewer committed, or
/// — F10's one exception (§10.3) — a release the owner confirmed by its tag
/// at a terminal, each by the digest the release API gave for it.
pub enum Source {
    Pinned(&'static Asset),
    OwnerConfirmed(Vec<Archive>),
}

/// One archive of an owner-confirmed release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archive {
    pub name: String,
    pub sha256: String,
    pub bytes: u64,
}

impl Spec {
    /// The shipped pin, for a target.
    pub fn pin(target: Target) -> Result<Spec> {
        Ok(Spec {
            tag: PIN.tag.into(),
            build: PIN.build,
            commit: PIN.commit.into(),
            source: Source::Pinned(
                asset(target).with_context(|| format!("no asset pinned for {target:?}"))?,
            ),
        })
    }

    /// What the archives download, in bytes.
    pub fn bytes(&self) -> u64 {
        match &self.source {
            Source::Pinned(a) => a.archives.iter().map(|(_, _, b)| b).sum(),
            Source::OwnerConfirmed(v) => v.iter().map(|a| a.bytes).sum(),
        }
    }
}

/// Install the pinned engine for this machine: fetch, unpack, check, link.
/// Idempotent — a build already unpacked and answering is linked, not fetched.
pub async fn install_engine(m: &Machinery, say: Say<'_>) -> Result<PathBuf> {
    let target = choose(std::env::consts::OS, std::env::consts::ARCH, read_nvidia())
        .map_err(anyhow::Error::msg)?;
    let id = "llama";
    let home = &m.mecha_home;
    let link = current(home);
    Manifest::begin(home, id)?;
    Manifest::record(home, id, &engine_root(home))?;
    Manifest::record(home, id, &engine_root(home).join(PIN.tag))?;
    Manifest::record(home, id, &link)?;
    install_build(m, target, &Spec::pin(target)?, say).await?;
    settle_current_on_pin(home)?;
    Manifest::finish(home, id)?;
    Ok(link.join("llama-server"))
}

/// `current` is set to the pin only when it names nothing (a first install)
/// or names the pin already. A machine an upgrade moved on keeps its build:
/// re-running the pin's install — `features enable` repairing an unfinished
/// entry — must never demote it outside the gate.
fn settle_current_on_pin(home: &Path) -> Result<()> {
    match link_tag(&current(home)) {
        None => point_current(home, PIN.tag),
        Some(t) if t == PIN.tag => point_current(home, PIN.tag),
        Some(_) => Ok(()),
    }
}

/// Unpack, check and record one build in its own directory under the engine
/// root — never touching `current`: which build the servers run is the
/// promotion's to change, after the gate. Idempotent for a build already
/// unpacked for this target and answering its checks. The engine root is
/// the recorded path an upgrade's builds live under; each build is recorded
/// by tag and commit once it has passed (`Entry::builds`).
pub async fn install_build(
    m: &Machinery,
    target: Target,
    spec: &Spec,
    say: Say<'_>,
) -> Result<PathBuf> {
    let home = &m.mecha_home;
    let root = engine_root(home);
    let dir = root.join(&spec.tag);
    std::fs::create_dir_all(&root)?;

    // The unpacked tree is this machine's build only when the record says
    // it was unpacked for this target: a tree for another one (installed
    // under an older driver) passes every check under the same tag, and
    // recording it as the new target would make the record lie.
    if unpacked_target(home, &spec.tag)? != Some(target) || health(&dir, target, spec).is_err() {
        // Unpacked into a scratch directory and renamed into place, so a
        // run interrupted mid-unpack leaves no tree that looks like a build.
        let part = root.join(format!("{}.part", spec.tag));
        let _ = std::fs::remove_dir_all(&part);
        std::fs::create_dir_all(&part)?;
        let downloads = root.join("download");
        let archives: Vec<(String, u64)> = match &spec.source {
            Source::Pinned(a) => a
                .archives
                .iter()
                .map(|(n, _, b)| (n.to_string(), *b))
                .collect(),
            Source::OwnerConfirmed(v) => v.iter().map(|a| (a.name.clone(), a.bytes)).collect(),
        };
        for (i, (name, bytes)) in archives.iter().enumerate() {
            say(&format!(
                "fetching {name} ({:.0} MiB)",
                *bytes as f64 / 1_048_576.0
            ));
            let archive = match &spec.source {
                Source::Pinned(a) => {
                    let (n, sha, b) = a.archives[i];
                    crate::fetch::fetch_release_asset(
                        REPO,
                        PIN.tag,
                        n,
                        sha,
                        b,
                        &downloads,
                        &mut |_| {},
                    )
                    .await?
                }
                Source::OwnerConfirmed(v) => {
                    crate::fetch::fetch_confirmed_release_asset(
                        REPO,
                        &spec.tag,
                        &v[i].name,
                        &v[i].sha256,
                        v[i].bytes,
                        &downloads,
                        &mut |_| {},
                    )
                    .await?
                }
            };
            // Each archive holds one directory; its contents land side by
            // side, so the CUDA runtime sits beside the libraries that
            // find it through `$ORIGIN`.
            let status = std::process::Command::new("tar")
                .arg("-xzf")
                .arg(&archive)
                .arg("-C")
                .arg(&part)
                .arg("--strip-components=1")
                .status()
                .context("running tar to unpack the engine")?;
            if !status.success() {
                bail!("tar could not unpack {}", archive.display());
            }
        }
        // Checked where it was unpacked — `$ORIGIN` makes the checks the same
        // under either name — so a build that fails them never replaces one
        // that passed, and a repair is never less usable than what it repairs.
        say(&format!(
            "checking llama.cpp {} ({})",
            spec.tag,
            target.label()
        ));
        health(&part, target, spec)?;
        if dir.exists() {
            std::fs::remove_dir_all(&dir).with_context(|| format!("clearing {}", dir.display()))?;
        }
        std::fs::rename(&part, &dir)?;
        let _ = std::fs::remove_dir_all(&downloads);
    }
    Manifest::record_build(
        home,
        "llama",
        Build {
            tag: spec.tag.clone(),
            commit: spec.commit.clone(),
            target: Some(target),
        },
    )?;
    Ok(dir)
}

/// A llama.cpp release as the release API describes it — the source F10
/// trusts, and only for a tag the owner then confirms.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub tag: String,
    pub build: u32,
    pub commit: String,
    pub published: String,
    /// Every asset, by name, with the digest and size the API gave.
    pub assets: Vec<Archive>,
}

impl Release {
    /// The archives for a target: the main build, and for CUDA its runtime
    /// archive of the same platform. Asset names carry the CUDA minor
    /// (`ubuntu-cuda-13.4-arm64`), which moves upstream, so the target is
    /// matched by its major and architecture, the newest minor first.
    pub fn archives_for(&self, target: Target) -> Result<Vec<Archive>> {
        let prefix = format!("llama-{}-bin-", self.tag);
        let mut platforms: Vec<&str> = self
            .assets
            .iter()
            .filter_map(|a| a.name.strip_prefix(&prefix)?.strip_suffix(".tar.gz"))
            .filter(|p| platform_is(target, p))
            .collect();
        // Newest CUDA minor first, compared as a number: `13.10` is newer
        // than `13.9`, which a string sort gets backwards.
        let minor = |p: &str| -> u32 {
            p.strip_prefix("ubuntu-cuda-")
                .and_then(|r| r.split(['.', '-']).nth(1))
                .and_then(|m| m.parse().ok())
                .unwrap_or(0)
        };
        platforms.sort_by_key(|p| std::cmp::Reverse(minor(p)));
        let platform = platforms.first().with_context(|| {
            format!(
                "llama.cpp {} publishes no {} build for this machine",
                self.tag,
                target.label()
            )
        })?;
        let find = |name: String| {
            self.assets
                .iter()
                .find(|a| a.name == name)
                .cloned()
                .with_context(|| format!("llama.cpp {} has no {name}", self.tag))
        };
        let mut out = vec![find(format!("{prefix}{platform}.tar.gz"))?];
        if target.label().starts_with("CUDA") {
            out.push(find(format!("cudart-{prefix}{platform}.tar.gz"))?);
        }
        Ok(out)
    }
}

/// Whether a release asset's platform (`ubuntu-cuda-13.4-arm64`) is a target's.
fn platform_is(target: Target, platform: &str) -> bool {
    let cuda = |major: &str, arch: &str| {
        platform
            .strip_prefix(&format!("ubuntu-cuda-{major}."))
            .and_then(|rest| rest.strip_suffix(&format!("-{arch}")))
            .is_some_and(|minor| !minor.is_empty() && minor.bytes().all(|c| c.is_ascii_digit()))
    };
    match target {
        Target::LinuxArm64Cuda13 => cuda("13", "arm64"),
        Target::LinuxX64Cuda13 => cuda("13", "x64"),
        Target::LinuxX64Cuda12 => cuda("12", "x64"),
        Target::LinuxArm64Cpu => platform == "ubuntu-arm64",
        Target::LinuxX64Cpu => platform == "ubuntu-x64",
        Target::MacArm64 => platform == "macos-arm64",
        Target::MacX64 => platform == "macos-x64",
    }
}

const API_BASE: &str = "https://api.github.com";

/// Resolve a release: the tag given, or the newest `b` release that
/// publishes a build for `target`. The per-merge `b` releases are marked
/// prerelease and `releases/latest` names a semver release with no binaries
/// (§10.3, measured 2026-10-04), so the newest is read from the list, never
/// from `latest`.
pub async fn resolve_release(tag: Option<&str>, target: Target) -> Result<Release> {
    resolve_release_from(API_BASE, tag, target).await
}

async fn resolve_release_from(base: &str, tag: Option<&str>, target: Target) -> Result<Release> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("mecha/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let get = |url: String| {
        let client = client.clone();
        async move {
            let resp = client
                .get(&url)
                .header("Accept", "application/vnd.github+json")
                .send()
                .await
                .with_context(|| format!("GET {url}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                bail!("{url} answered {status}: {}", text.trim());
            }
            serde_json::from_str::<serde_json::Value>(&text)
                .with_context(|| format!("{url} answered no JSON"))
        }
    };
    let release = match tag {
        Some(t) => {
            if !(t.len() > 1 && t.starts_with('b') && t[1..].bytes().all(|c| c.is_ascii_digit())) {
                bail!(
                    "`{t}` is not a llama.cpp build tag — they are `b` and digits, like {}",
                    PIN.tag
                );
            }
            get(format!("{base}/repos/{REPO}/releases/tags/{t}")).await?
        }
        None => {
            let list = get(format!("{base}/repos/{REPO}/releases?per_page=30")).await?;
            list.as_array()
                .into_iter()
                .flatten()
                .find(|r| {
                    !r["draft"].as_bool().unwrap_or(true)
                        && parse_release(r).is_ok_and(|rel| rel.archives_for(target).is_ok())
                })
                .cloned()
                .with_context(|| {
                    format!(
                        "none of llama.cpp's 30 newest releases publishes a {} build for this machine",
                        target.label()
                    )
                })?
        }
    };
    let mut rel = parse_release(&release)?;
    // The commit the tag names, from the tag itself: `target_commitish` can
    // be a branch name. An annotated tag is followed to its commit.
    let mut obj =
        get(format!("{base}/repos/{REPO}/git/ref/tags/{}", rel.tag)).await?["object"].clone();
    if obj["type"].as_str() == Some("tag") {
        let sha = obj["sha"].as_str().unwrap_or_default().to_string();
        obj = get(format!("{base}/repos/{REPO}/git/tags/{sha}")).await?["object"].clone();
    }
    rel.commit = obj["sha"]
        .as_str()
        .filter(|s| s.len() == 40 && s.bytes().all(|c| c.is_ascii_hexdigit()))
        .with_context(|| format!("the tag {} names no commit", rel.tag))?
        .to_string();
    Ok(rel)
}

/// A release object to a `Release`, the commit left to the tag's own lookup.
fn parse_release(r: &serde_json::Value) -> Result<Release> {
    let tag = r["tag_name"]
        .as_str()
        .context("a release with no tag")?
        .to_string();
    let build: u32 = tag
        .strip_prefix('b')
        .and_then(|n| n.parse().ok())
        .with_context(|| format!("`{tag}` is not a build tag"))?;
    let assets = r["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            Some(Archive {
                name: a["name"].as_str()?.to_string(),
                // F10 trusts the API's digest and nothing else: an asset
                // without one is not offered.
                sha256: a["digest"].as_str()?.strip_prefix("sha256:")?.to_string(),
                bytes: a["size"].as_u64()?,
            })
        })
        .collect();
    Ok(Release {
        tag,
        build,
        commit: String::new(),
        published: r["published_at"].as_str().unwrap_or("").to_string(),
        assets,
    })
}

/// Swap a link under the engine root to name `tag`'s directory — relative,
/// so the home can move — in one rename.
fn swap_link(home: &Path, name: &str, tag: &str) -> Result<()> {
    let root = engine_root(home);
    let tmp = root.join(format!("{name}.new"));
    let _ = std::fs::remove_file(&tmp);
    #[cfg(unix)]
    std::os::unix::fs::symlink(tag, &tmp)?;
    std::fs::rename(&tmp, root.join(name)).with_context(|| format!("pointing {name} at {tag}"))?;
    Ok(())
}

/// Point `current` — what an adopted unit's drop-in names — at a build.
pub fn point_current(home: &Path, tag: &str) -> Result<()> {
    swap_link(home, "current", tag)
}

/// The link the build before the current one is kept under, for
/// `--rollback` (today's `.prev` copy, made structural).
pub fn previous(mecha_home: &Path) -> PathBuf {
    engine_root(mecha_home).join("previous")
}

/// Point `previous` at a build.
pub fn point_previous(home: &Path, tag: &str) -> Result<()> {
    swap_link(home, "previous", tag)
}

/// The tag a link under the engine root names, if it is one.
pub fn link_tag(link: &Path) -> Option<String> {
    std::fs::read_link(link)
        .ok()
        .and_then(|t| t.file_name().map(|n| n.to_string_lossy().into_owned()))
}

/// Remove every build that is neither `current` nor `previous`, and its
/// record — so upgrades keep two builds on disk, not every one ever tried.
pub fn prune_builds(home: &Path) -> Result<Vec<String>> {
    // Without a readable `current` nothing is known to be in use, and
    // pruning would remove every build: refused, never guessed.
    if link_tag(&current(home)).is_none() {
        bail!("`current` names no build, so no build is pruned");
    }
    let keep: Vec<String> = [current(home), previous(home)]
        .iter()
        .filter_map(|l| link_tag(l))
        .collect();
    let mut gone = Vec::new();
    let mut manifest = Manifest::read(home)?;
    if let Some(e) = manifest.entries.iter_mut().find(|e| e.sidecar == "llama") {
        for b in e.builds.clone() {
            if keep.contains(&b.tag) {
                continue;
            }
            let dir = engine_root(home).join(&b.tag);
            if dir.exists() {
                std::fs::remove_dir_all(&dir)
                    .with_context(|| format!("removing {}", dir.display()))?;
            }
            gone.push(b.tag.clone());
        }
        e.builds.retain(|b| keep.contains(&b.tag));
        // The pin's directory is a recorded path of the original install;
        // gone from disk on purpose, it is forgotten, not "unfinished".
        e.wrote
            .retain(|w| !gone.iter().any(|t| *w == engine_root(home).join(t)));
    }
    manifest.write(home)?;
    Ok(gone)
}

/// The target `tag` was unpacked for, as the manifest records it; `None`
/// when no build of the tag is recorded, or its target is one this mecha
/// does not know.
fn unpacked_target(home: &Path, tag: &str) -> Result<Option<Target>> {
    Ok(Manifest::read(home)?
        .entries
        .iter()
        .find(|e| e.sidecar == "llama")
        .and_then(|e| e.builds.iter().find(|b| b.tag == tag))
        .and_then(|b| b.target))
}

/// The three checks a build passes before it counts (module doc).
fn health(dir: &Path, target: Target, spec: &Spec) -> Result<()> {
    let server = dir.join("llama-server");
    if !server.is_file() {
        bail!("{} is not there", server.display());
    }
    if cfg!(target_os = "linux") {
        self_contained(dir)?;
    }
    // The CUDA runtime is the second archive's whole contribution, and a
    // machine with its own CUDA would load that one without a word — so its
    // presence is checked, not inferred from the engine starting.
    if target.label().starts_with("CUDA") && !ships_cudart(dir) {
        bail!(
            "{} has no libcudart beside the engine — the CUDA runtime archive did not land",
            dir.display()
        );
    }
    let version = engine_output(&server, "--version")?;
    if !reports(&version, spec.build, &spec.commit) {
        bail!(
            "{} reports `{}`, not build {} at {}",
            server.display(),
            version.lines().next().unwrap_or("").trim(),
            spec.build,
            &spec.commit[..spec.commit.len().min(9)]
        );
    }
    if target.wants_gpu() {
        let devices = engine_output(&server, "--list-devices")?;
        if !lists_a_gpu(&devices) {
            bail!(
                "the {} build of llama.cpp found no GPU — it would run on the CPU. \
                 `{} --list-devices` says:\n{}",
                target.label(),
                server.display(),
                devices.trim()
            );
        }
    }
    Ok(())
}

fn ships_cudart(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().starts_with("libcudart.so"))
    })
}

/// How long one check may take. `--list-devices` brings up the GPU backend,
/// which is what a wedged driver hangs on; past this it is a named failure,
/// never a silent wait.
const CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Run the engine with one flag and the library path cleared: the libraries
/// beside it, found through `$ORIGIN`, are the ones it must load.
fn engine_output(server: &Path, flag: &str) -> Result<String> {
    run_within(server, flag, CHECK_TIMEOUT)
}

fn run_within(server: &Path, flag: &str, limit: std::time::Duration) -> Result<String> {
    // `ExecutableFileBusy` is a file just written whose descriptor another
    // thread's fork still holds for the instant before its exec; it clears
    // on its own, so it is retried briefly rather than failing the check.
    let mut tries = 0;
    let mut child = loop {
        match std::process::Command::new(server)
            .arg(flag)
            .env_remove("LD_LIBRARY_PATH")
            .env_remove("DYLD_LIBRARY_PATH")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 20 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            other => break other.with_context(|| format!("starting {}", server.display()))?,
        }
    };
    // Drained while it runs: a check that writes more than a pipe holds
    // would otherwise block on the write and read as one that hung.
    let drain = |pipe: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        })
    };
    let out_t = drain(child.stdout.take().map(|p| Box::new(p) as _));
    let err_t = drain(child.stderr.take().map(|p| Box::new(p) as _));
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "`{} {flag}` did not answer within {} s — a GPU driver that hangs when the \
                 engine brings it up; nothing was switched on",
                server.display(),
                limit.as_secs_f32()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let stdout = out_t.join().unwrap_or_default();
    let stderr = err_t.join().unwrap_or_default();
    // llama.cpp writes these to stderr; either stream will do.
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );
    if !status.success() {
        bail!(
            "`{} {flag}` failed ({status}): {}",
            server.display(),
            text.trim()
        );
    }
    Ok(text)
}

/// `--version`'s first line names the build and an abbreviated commit:
/// `version: 0.5.0-dev (build 11391, commit 2bc563573)` from the pinned
/// release, read on the GB10 on 2026-10-04. The abbreviation's length is the
/// release builder's `git rev-parse --short`, so any prefix of the expected
/// commit of seven or more characters is that build.
pub fn reports(text: &str, build: u32, commit: &str) -> bool {
    let Some(line) = text.lines().find(|l| l.contains("version:")) else {
        return false;
    };
    let wanted = format!("build {build},");
    let Some(rest) = line.split_once("commit ").map(|(_, r)| r) else {
        return false;
    };
    let short: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
    line.contains(&wanted) && short.len() >= 7 && commit.starts_with(&short)
}

/// The pin's own version check.
#[cfg(test)]
fn reports_the_pin(text: &str) -> bool {
    reports(text, PIN.build, PIN.commit)
}

/// `/props`' `build_info` — `b11391-2bc563573` — names the same build
/// `--version` does: asked of the router after a promotion, it is the model's
/// own process saying which engine it runs.
pub fn build_info_is(info: &str, build: u32, commit: &str) -> bool {
    let Some(rest) = info.strip_prefix(&format!("b{build}-")) else {
        return false;
    };
    let short: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
    short.len() >= 7 && commit.starts_with(&short)
}

/// `--list-devices` names each backend device under `Available devices:` —
/// `CUDA0: NVIDIA GB10 (…)`, `MTL0: Apple M2 (…)`. The CPU is not listed.
fn lists_a_gpu(text: &str) -> bool {
    text.lines()
        .skip_while(|l| !l.contains("Available devices"))
        .skip(1)
        .any(|l| {
            let l = l.trim();
            l.starts_with("CUDA") || l.starts_with("MTL") || l.starts_with("Metal")
        })
}

/// Every ELF in `dir` finds its libraries relative to itself: no RUNPATH or
/// RPATH entry is an absolute path, and an ELF that needs a library shipped
/// in `dir` reaches it through `$ORIGIN`.
pub fn self_contained(dir: &Path) -> Result<()> {
    let shipped: std::collections::BTreeSet<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        // A symlink names a file that is checked under its own name.
        if !std::fs::symlink_metadata(&path)?.is_file() {
            continue;
        }
        // Not an ELF is skipped; an ELF this check could not read stops the
        // install — a check that could not run never passes.
        let file = std::fs::File::open(&path)?;
        let Some(dynamic) = elf_dynamic_in(&mut FileAt(file))
            .with_context(|| format!("reading {}'s dynamic section", path.display()))?
        else {
            continue;
        };
        for p in dynamic.search_paths() {
            if !within_origin(p) {
                bail!(
                    "{} looks for libraries in `{p}`, outside its own directory — a build \
                     that would run another tree's libraries",
                    path.display()
                );
            }
        }
        let needs_shipped = dynamic.needed.iter().any(|n| shipped.contains(n));
        if needs_shipped && !dynamic.search_paths().any(within_origin) {
            bail!(
                "{} needs a library shipped beside it but has no `$ORIGIN` RUNPATH to find it by",
                path.display()
            );
        }
    }
    Ok(())
}

/// A search path that stays in the binary's own directory: `$ORIGIN`, or a
/// directory under it — never one that climbs out with `..`, which would
/// reach `sidecars/llama/` and survive neither a tag swap nor a rollback.
fn within_origin(p: &str) -> bool {
    let rest = p
        .strip_prefix("${ORIGIN}")
        .or_else(|| p.strip_prefix("$ORIGIN"));
    match rest {
        Some("") => true,
        Some(sub) if sub.starts_with('/') => !sub.split('/').any(|c| c == ".."),
        _ => false,
    }
}

/// What an ELF's dynamic section says about finding its libraries.
#[derive(Debug, Default, PartialEq)]
pub struct Dynamic {
    pub needed: Vec<String>,
    pub runpath: Option<String>,
    pub rpath: Option<String>,
}

impl Dynamic {
    /// Every directory RUNPATH and RPATH name.
    fn search_paths(&self) -> impl Iterator<Item = &str> {
        self.runpath
            .iter()
            .chain(self.rpath.iter())
            .flat_map(|s| s.split(':'))
            .filter(|s| !s.is_empty())
    }
}

/// Bytes at an offset — a slice in tests, a file at install — so the check
/// reads a library's headers and strings and never the library: the CUDA
/// runtime's `libcublasLt` alone is hundreds of MB.
pub trait ReadAt {
    fn read_at(&mut self, off: u64, len: usize) -> Option<Vec<u8>>;
}

impl ReadAt for &[u8] {
    fn read_at(&mut self, off: u64, len: usize) -> Option<Vec<u8>> {
        let start = usize::try_from(off).ok()?;
        self.get(start..start.checked_add(len)?).map(<[u8]>::to_vec)
    }
}

/// A file read at offsets, short reads being `None`.
pub struct FileAt(pub std::fs::File);

impl ReadAt for FileAt {
    fn read_at(&mut self, off: u64, len: usize) -> Option<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        self.0.seek(SeekFrom::Start(off)).ok()?;
        let mut buf = vec![0u8; len];
        self.0.read_exact(&mut buf).ok()?;
        Some(buf)
    }
}

/// Parse an ELF's dynamic section. A file that is not an ELF is `Ok(None)`;
/// an ELF that is not 64-bit little-endian — the linux targets mecha installs
/// are x86_64 and aarch64 — or one whose headers, dynamic section or strings
/// do not parse is an error, never a skip. One with no dynamic section
/// (static) needs nothing and reads empty.
pub fn elf_dynamic(mut b: &[u8]) -> Result<Option<Dynamic>> {
    elf_dynamic_in(&mut b)
}

fn elf_dynamic_in(r: &mut impl ReadAt) -> Result<Option<Dynamic>> {
    match r.read_at(0, 4) {
        Some(m) if m == b"\x7fELF" => {}
        _ => return Ok(None),
    }
    match r.read_at(4, 2) {
        Some(id) if id == [2, 1] => {}
        _ => bail!("an ELF this check cannot read: not 64-bit little-endian, or truncated"),
    }
    parse_elf64le(r)
        .map(Some)
        .context("an ELF whose headers, dynamic section or strings do not parse")
}

/// The most a dynamic section, or one string in it, may be before the parse
/// calls the file malformed rather than allocate for it.
const MAX_DYNAMIC: u64 = 1 << 20;
const MAX_STRING: usize = 4096;

fn parse_elf64le(r: &mut impl ReadAt) -> Option<Dynamic> {
    let le64 = |b: &[u8], o: usize| Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?));
    let le32 = |b: &[u8], o: usize| Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?));
    let le16 = |b: &[u8], o: usize| Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?));
    let header = r.read_at(0, 64)?;
    let phoff = le64(&header, 0x20)?;
    let phentsize = u64::from(le16(&header, 0x36)?);
    let phnum = u64::from(le16(&header, 0x38)?);
    if phentsize < 56 {
        return None;
    }
    // (vaddr, offset, filesz) of each PT_LOAD, to map DT_STRTAB's address.
    let mut loads = Vec::new();
    let mut dynamic = None;
    for i in 0..phnum {
        let h = r.read_at(phoff.checked_add(i.checked_mul(phentsize)?)?, 56)?;
        let (offset, vaddr, filesz) = (le64(&h, 8)?, le64(&h, 16)?, le64(&h, 32)?);
        match le32(&h, 0)? {
            1 => loads.push((vaddr, offset, filesz)),
            2 => dynamic = Some((offset, filesz)),
            _ => {}
        }
    }
    let Some((dyn_off, dyn_len)) = dynamic else {
        // Statically linked: it loads no library, from anywhere.
        return Some(Dynamic::default());
    };
    if dyn_len > MAX_DYNAMIC {
        return None;
    }
    let d = r.read_at(dyn_off, usize::try_from(dyn_len).ok()?)?;
    let (mut strtab, mut needed, mut runpath, mut rpath) = (None, Vec::new(), None, None);
    for entry in d.as_chunks::<16>().0 {
        let (tag, val) = (le64(entry, 0)?, le64(entry, 8)?);
        match tag {
            0 => break,
            1 => needed.push(val),
            5 => strtab = Some(val),
            15 => rpath = Some(val),
            29 => runpath = Some(val),
            _ => {}
        }
    }
    if needed.is_empty() && runpath.is_none() && rpath.is_none() {
        return Some(Dynamic::default());
    }
    let addr = strtab?;
    let base = loads.iter().find_map(|(v, off, sz)| {
        let into = addr.checked_sub(*v)?;
        (into < *sz).then(|| off.checked_add(into)).flatten()
    })?;
    // A string, read up to its NUL; one that never ends, or one past the
    // file, fails the parse rather than reading as missing.
    let mut string = |o: u64| -> Option<String> {
        let start = base.checked_add(o)?;
        let mut out = Vec::new();
        while out.len() < MAX_STRING {
            let step = 256.min(MAX_STRING - out.len());
            let at = start.checked_add(out.len() as u64)?;
            // Near the end of the file, a short tail still holds the NUL.
            let chunk = (1..=step).rev().find_map(|n| r.read_at(at, n))?;
            if let Some(end) = chunk.iter().position(|c| *c == 0) {
                out.extend_from_slice(&chunk[..end]);
                return Some(String::from_utf8_lossy(&out).into_owned());
            }
            out.extend_from_slice(&chunk);
        }
        None
    };
    let needed = needed
        .into_iter()
        .map(&mut string)
        .collect::<Option<Vec<_>>>()?;
    let runpath = match runpath {
        Some(o) => Some(string(o)?),
        None => None,
    };
    let rpath = match rpath {
        Some(o) => Some(string(o)?),
        None => None,
    };
    Some(Dynamic {
        needed,
        runpath,
        rpath,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_target_has_one_pinned_asset_with_hashes() {
        for t in [
            Target::LinuxArm64Cuda13,
            Target::LinuxX64Cuda13,
            Target::LinuxX64Cuda12,
            Target::LinuxArm64Cpu,
            Target::LinuxX64Cpu,
            Target::MacArm64,
            Target::MacX64,
        ] {
            let a = asset(t).unwrap_or_else(|| panic!("{t:?} has no asset"));
            assert!(!a.archives.is_empty());
            for (name, sha, bytes) in a.archives {
                assert!(name.contains(PIN.tag), "{name} is not the pin's");
                assert_eq!(sha.len(), 64, "{name}");
                assert!(sha.bytes().all(|c| c.is_ascii_hexdigit()), "{name}");
                assert!(*bytes > 1_000_000, "{name}");
            }
            // A CUDA build ships its runtime in a second archive, and must
            // carry it: the first alone would load whatever CUDA the machine
            // has, or none.
            if t.label().starts_with("CUDA") {
                assert!(a.archives.iter().any(|(n, ..)| n.starts_with("cudart-")));
            }
        }
        assert_eq!(PIN.commit.len(), 40);
        assert_eq!(PIN.tag, format!("b{}", PIN.build));
    }

    #[test]
    fn the_build_is_chosen_from_the_driver() {
        use Nvidia::*;
        assert_eq!(
            choose("linux", "aarch64", Driver(580, 173)),
            Ok(Target::LinuxArm64Cuda13)
        );
        assert!(choose("linux", "aarch64", Driver(570, 1)).is_err());
        assert_eq!(
            choose("linux", "x86_64", Driver(590, 0)),
            Ok(Target::LinuxX64Cuda13)
        );
        assert_eq!(
            choose("linux", "x86_64", Driver(570, 86)),
            Ok(Target::LinuxX64Cuda12)
        );
        assert_eq!(
            choose("linux", "x86_64", Driver(525, 60)),
            Ok(Target::LinuxX64Cuda12)
        );
        assert!(choose("linux", "x86_64", Driver(525, 59)).is_err());
        assert_eq!(choose("linux", "x86_64", None), Ok(Target::LinuxX64Cpu));
        assert_eq!(choose("linux", "aarch64", None), Ok(Target::LinuxArm64Cpu));
        assert_eq!(choose("macos", "aarch64", None), Ok(Target::MacArm64));
        assert!(choose("windows", "x86_64", None).is_err());
    }

    /// A GPU that cannot be read is never answered with the CPU build.
    #[test]
    fn an_unread_driver_installs_nothing() {
        for arch in ["aarch64", "x86_64"] {
            let err = choose("linux", arch, Nvidia::Unread).unwrap_err();
            assert!(err.contains("nvidia-smi"), "{err}");
        }
    }

    /// A GPU with no driver tools is still a GPU: seen on the PCI bus as
    /// NVIDIA's display class, never confused with NVIDIA's PCIe bridges.
    #[test]
    fn an_nvidia_display_device_is_seen_on_the_bus() {
        let root = std::env::temp_dir().join(format!("mecha-pci-{}", uuid::Uuid::new_v4()));
        let dev = |name: &str, vendor: &str, class: &str| {
            let d = root.join(name);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("vendor"), format!("{vendor}\n")).unwrap();
            std::fs::write(d.join("class"), format!("{class}\n")).unwrap();
        };
        dev("0000:00:00.0", "0x10de", "0x060400");
        dev("0001:00:00.0", "0x8086", "0x030000");
        assert!(!pci_nvidia_display(&root), "a bridge and an Intel GPU");
        dev("000f:01:00.0", "0x10de", "0x030000");
        assert!(pci_nvidia_display(&root));
        assert!(!pci_nvidia_display(&root.join("absent")));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A CUDA build without its runtime beside it is refused: on a machine
    /// with its own CUDA it would start, loading a runtime nobody pinned.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_cuda_build_without_its_runtime_is_refused() {
        let dir = std::env::temp_dir().join(format!("mecha-cudart-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("llama-server"), "#!/bin/sh\n").unwrap();
        let err = health(
            &dir,
            Target::LinuxArm64Cuda13,
            &Spec::pin(Target::LinuxArm64Cuda13).unwrap(),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("libcudart"), "{err}");
        std::fs::write(dir.join("libcudart.so.13"), "").unwrap();
        assert!(ships_cudart(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A check that never answers is a named failure, not a wait.
    #[cfg(unix)]
    #[test]
    fn a_check_that_hangs_is_cut_off() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("mecha-hang-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let server = dir.join("llama-server");
        std::fs::write(&server, "#!/bin/sh\nexec sleep 600\n").unwrap();
        std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = run_within(
            &server,
            "--list-devices",
            std::time::Duration::from_millis(300),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("did not answer"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The version check over the text the pinned release printed on the
    /// GB10 — the one check `health` makes that CI could not otherwise see,
    /// since the end-to-end test is ignored there.
    #[test]
    fn the_pinned_release_s_version_reads_as_the_pin() {
        let real = "version: 0.5.0-dev (build 11391, commit 2bc563573)\n\
                    built with GNU 14.2.0 for Linux aarch64\n";
        assert!(reports_the_pin(real));
        // Another abbreviation length is the same commit.
        assert!(reports_the_pin(
            "version: 0.5.0-dev (build 11391, commit 2bc5635)"
        ));
        // This box's from-source engine, another build, another commit.
        assert!(!reports_the_pin("version: 11205 (95887577)"));
        assert!(!reports_the_pin(
            "version: 0.5.0-dev (build 113910, commit 2bc563573)"
        ));
        assert!(!reports_the_pin(
            "version: 0.5.0-dev (build 11391, commit 2bc56)"
        ));
        assert!(!reports_the_pin(
            "version: 0.5.0-dev (build 11391, commit deadbeef0)"
        ));
        assert!(!reports_the_pin(""));
    }

    /// A tree unpacked for another target is not this machine's build, however
    /// well it answers: a driver upgrade refetches rather than relabels.
    #[test]
    fn the_record_decides_which_target_is_unpacked() {
        let home = std::env::temp_dir().join(format!("mecha-target-{}", uuid::Uuid::new_v4()));
        assert_eq!(unpacked_target(&home, PIN.tag).unwrap(), None);
        Manifest::begin(&home, "llama").unwrap();
        Manifest::record_build(
            &home,
            "llama",
            Build {
                tag: PIN.tag.into(),
                commit: PIN.commit.into(),
                target: Some(Target::LinuxX64Cuda12),
            },
        )
        .unwrap();
        assert_eq!(
            unpacked_target(&home, PIN.tag).unwrap(),
            Some(Target::LinuxX64Cuda12)
        );
        assert_ne!(
            unpacked_target(&home, PIN.tag).unwrap(),
            Some(Target::LinuxX64Cuda13)
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn only_paths_inside_origin_are_its_own() {
        for ok in ["$ORIGIN", "${ORIGIN}", "$ORIGIN/lib", "$ORIGIN/./lib"] {
            assert!(within_origin(ok), "{ok}");
        }
        for bad in [
            "$ORIGIN/../lib",
            "$ORIGIN/lib/../..",
            "$ORIGINAL",
            "/opt/cuda/lib64",
            "lib",
        ] {
            assert!(!within_origin(bad), "{bad}");
        }
    }

    /// A check that writes more than a pipe holds is read while it runs, not
    /// after — otherwise it blocks on the write and reads as one that hung.
    #[cfg(unix)]
    #[test]
    fn a_chatty_check_is_not_mistaken_for_a_hung_one() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("mecha-chatty-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let server = dir.join("llama-server");
        std::fs::write(
            &server,
            "#!/bin/sh\nhead -c 300000 /dev/zero | tr '\\0' x\nhead -c 300000 /dev/zero | tr '\\0' y >&2\n",
        )
        .unwrap();
        std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o755)).unwrap();
        let text = run_within(
            &server,
            "--list-devices",
            std::time::Duration::from_secs(20),
        )
        .unwrap();
        assert_eq!(text.len(), 600_000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_download_is_the_sum_of_the_target_s_archives() {
        assert_eq!(
            download_bytes(Target::LinuxArm64Cuda13),
            147_596_012 + 552_521_398
        );
        assert_eq!(download_bytes(Target::LinuxX64Cpu), 17_637_234);
    }

    fn release(names: &[&str]) -> Release {
        Release {
            tag: "b20000".into(),
            build: 20000,
            commit: "c".repeat(40),
            published: String::new(),
            assets: names
                .iter()
                .map(|n| Archive {
                    name: n.to_string(),
                    sha256: "a".repeat(64),
                    bytes: 1,
                })
                .collect(),
        }
    }

    /// A release's archives for a target: matched by CUDA major and
    /// architecture, whatever minor upstream ships, with the runtime archive
    /// of the same platform; a CPU target takes no runtime.
    #[test]
    fn a_release_s_archives_are_matched_to_the_target() {
        let r = release(&[
            "llama-b20000-bin-ubuntu-cuda-13.6-arm64.tar.gz",
            "cudart-llama-b20000-bin-ubuntu-cuda-13.6-arm64.tar.gz",
            "llama-b20000-bin-ubuntu-cuda-12.8-x64.tar.gz",
            "cudart-llama-b20000-bin-ubuntu-cuda-12.8-x64.tar.gz",
            "llama-b20000-bin-ubuntu-arm64.tar.gz",
            "llama-b20000-bin-win-cuda-13.6-arm64.zip",
        ]);
        let names = |t| -> Vec<String> {
            r.archives_for(t)
                .unwrap()
                .into_iter()
                .map(|a| a.name)
                .collect()
        };
        assert_eq!(
            names(Target::LinuxArm64Cuda13),
            vec![
                "llama-b20000-bin-ubuntu-cuda-13.6-arm64.tar.gz",
                "cudart-llama-b20000-bin-ubuntu-cuda-13.6-arm64.tar.gz"
            ]
        );
        assert_eq!(
            names(Target::LinuxArm64Cpu),
            vec!["llama-b20000-bin-ubuntu-arm64.tar.gz"]
        );
        assert!(
            r.archives_for(Target::LinuxX64Cuda13).is_err(),
            "no 13.x x64 build"
        );
        assert!(r.archives_for(Target::MacArm64).is_err());
        // The newest minor wins as a number: 13.10 over 13.9.
        let two = release(&[
            "llama-b20000-bin-ubuntu-cuda-13.9-arm64.tar.gz",
            "cudart-llama-b20000-bin-ubuntu-cuda-13.9-arm64.tar.gz",
            "llama-b20000-bin-ubuntu-cuda-13.10-arm64.tar.gz",
            "cudart-llama-b20000-bin-ubuntu-cuda-13.10-arm64.tar.gz",
        ]);
        assert_eq!(
            two.archives_for(Target::LinuxArm64Cuda13).unwrap()[0].name,
            "llama-b20000-bin-ubuntu-cuda-13.10-arm64.tar.gz"
        );
        // A CUDA build whose runtime archive is missing is not offered.
        let half = release(&["llama-b20000-bin-ubuntu-cuda-13.6-arm64.tar.gz"]);
        assert!(half.archives_for(Target::LinuxArm64Cuda13).is_err());
    }

    /// An asset the API gave no sha256 digest for is not offered: F10 trusts
    /// the digest, and nothing else.
    #[test]
    fn an_asset_without_a_digest_is_dropped() {
        let r = parse_release(&serde_json::json!({
            "tag_name": "b20000",
            "published_at": "2026-10-05T00:00:00Z",
            "assets": [
                {"name": "a.tar.gz", "size": 5, "digest": "sha256:abcd"},
                {"name": "b.tar.gz", "size": 5},
                {"name": "c.tar.gz", "size": 5, "digest": "md5:ffff"},
            ],
        }))
        .unwrap();
        assert_eq!(r.build, 20000);
        assert_eq!(
            r.assets.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            vec!["a.tar.gz"]
        );
        assert!(parse_release(&serde_json::json!({"tag_name": "v0.5.0", "assets": []})).is_err());
    }

    #[test]
    fn build_info_names_its_build() {
        let commit = "2bc563573479d53b30b8793039485887bc0fdda8";
        assert!(build_info_is("b11391-2bc563573", 11391, commit));
        assert!(build_info_is("b11391-2bc5635", 11391, commit));
        assert!(!build_info_is("b1193-95887577", 11391, commit));
        assert!(!build_info_is("b11391-95887577", 11391, commit));
        assert!(!build_info_is("b113910-2bc563573", 11391, commit));
        assert!(!build_info_is("", 11391, commit));
    }

    /// A mock release API: route → JSON, everything else 404.
    fn mock_api(routes: Vec<(&'static str, serde_json::Value)>) -> String {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in l.incoming().flatten() {
                let mut s = stream;
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req.split_whitespace().nth(1).unwrap_or("");
                let hit = routes.iter().find(|(r, _)| *r == path);
                let (status, body) = match hit {
                    Some((_, v)) => ("200 OK", v.to_string()),
                    None => ("404 Not Found", "{}".to_string()),
                };
                let _ = write!(
                    s,
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        base
    }

    fn rel(tag: &str, draft: bool) -> serde_json::Value {
        serde_json::json!({
            "tag_name": tag, "draft": draft, "published_at": "2026-10-05T00:00:00Z",
            "assets": [{"name": format!("llama-{tag}-bin-ubuntu-arm64.tar.gz"), "size": 9, "digest": format!("sha256:{}", "a".repeat(64))}],
        })
    }

    /// The newest scan skips a draft and a non-`b` release, an annotated tag
    /// is followed to its commit, and a tag naming no commit is refused —
    /// before anything could be fetched.
    #[tokio::test]
    async fn release_resolution_walks_the_list_and_follows_the_tag() {
        let commit = "d".repeat(40);
        let base = mock_api(vec![
            (
                "/repos/ggml-org/llama.cpp/releases?per_page=30",
                serde_json::json!([
                    rel("b30001", true),
                    rel("v0.5.0", false),
                    rel("b30000", false)
                ]),
            ),
            (
                "/repos/ggml-org/llama.cpp/git/ref/tags/b30000",
                serde_json::json!({"object": {"type": "tag", "sha": "t".repeat(40)}}),
            ),
            (
                "/repos/ggml-org/llama.cpp/git/tags/tttttttttttttttttttttttttttttttttttttttt",
                serde_json::json!({"object": {"type": "commit", "sha": commit}}),
            ),
            (
                "/repos/ggml-org/llama.cpp/releases/tags/b29999",
                rel("b29999", false),
            ),
            (
                "/repos/ggml-org/llama.cpp/git/ref/tags/b29999",
                serde_json::json!({"object": {"type": "commit", "sha": "not-a-sha"}}),
            ),
        ]);
        let r = resolve_release_from(&base, None, Target::LinuxArm64Cpu)
            .await
            .unwrap();
        assert_eq!(r.tag, "b30000", "the draft and the v-tag are skipped");
        assert_eq!(
            r.commit, commit,
            "the annotated tag is followed to its commit"
        );
        let err = resolve_release_from(&base, Some("b29999"), Target::LinuxArm64Cpu)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("names no commit"), "{err}");
    }

    /// The release API's digests for the pinned tag are the pinned sha256s —
    /// the check that F10's source and a reviewer's pin agree where both
    /// exist. Real network.
    #[tokio::test]
    #[ignore]
    async fn the_release_api_agrees_with_the_pin() {
        let r = resolve_release(Some(PIN.tag), Target::LinuxArm64Cuda13)
            .await
            .unwrap();
        assert_eq!(r.commit, PIN.commit);
        assert_eq!(r.build, PIN.build);
        for t in [
            Target::LinuxArm64Cuda13,
            Target::LinuxX64Cuda13,
            Target::LinuxX64Cuda12,
            Target::LinuxArm64Cpu,
            Target::LinuxX64Cpu,
            Target::MacArm64,
            Target::MacX64,
        ] {
            let got: Vec<(String, String, u64)> = r
                .archives_for(t)
                .unwrap()
                .into_iter()
                .map(|a| (a.name, a.sha256, a.bytes))
                .collect();
            let pinned: Vec<(String, String, u64)> = asset(t)
                .unwrap()
                .archives
                .iter()
                .map(|(n, s, b)| (n.to_string(), s.to_string(), *b))
                .collect();
            assert_eq!(got, pinned, "{t:?}");
        }
        let newest = resolve_release(None, Target::LinuxArm64Cuda13)
            .await
            .unwrap();
        assert!(newest.build >= PIN.build, "{newest:?}");
    }

    /// Two builds on disk, not every one tried: `current` and `previous` are
    /// kept, every other build goes with its record — the pin's own recorded
    /// directory forgotten, not left reading "unfinished".
    #[test]
    fn pruning_keeps_current_and_previous() {
        let home = std::env::temp_dir().join(format!("mecha-prune-{}", uuid::Uuid::new_v4()));
        let root = engine_root(&home);
        Manifest::begin(&home, "llama").unwrap();
        Manifest::record(&home, "llama", &root).unwrap();
        Manifest::record(&home, "llama", &root.join("b1")).unwrap();
        for t in ["b1", "b2", "b3"] {
            std::fs::create_dir_all(root.join(t)).unwrap();
            Manifest::record_build(
                &home,
                "llama",
                Build {
                    tag: t.into(),
                    commit: "c".repeat(40),
                    target: None,
                },
            )
            .unwrap();
        }
        point_current(&home, "b3").unwrap();
        point_previous(&home, "b2").unwrap();
        assert_eq!(link_tag(&current(&home)).as_deref(), Some("b3"));
        let gone = prune_builds(&home).unwrap();
        assert_eq!(gone, vec!["b1".to_string()]);
        assert!(!root.join("b1").exists() && root.join("b2").exists() && root.join("b3").exists());
        let m = Manifest::read(&home).unwrap();
        let e = m.entries.iter().find(|e| e.sidecar == "llama").unwrap();
        let tags: Vec<&str> = e.builds.iter().map(|b| b.tag.as_str()).collect();
        assert_eq!(tags, vec!["b2", "b3"]);
        assert!(
            !e.wrote.contains(&root.join("b1")),
            "the pruned pin dir is forgotten"
        );
        assert!(e.wrote.contains(&root), "the root stays recorded");
        // With no readable `current`, nothing is known to be in use: refused,
        // and nothing removed.
        std::fs::remove_file(current(&home)).unwrap();
        assert!(prune_builds(&home).is_err());
        assert!(root.join("b2").exists() && root.join("b3").exists());
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The pin's install sets `current` on a first run and leaves an upgraded
    /// machine's alone — the link rule `install_engine` follows.
    #[test]
    fn the_pin_install_never_demotes_an_upgraded_current() {
        let home = std::env::temp_dir().join(format!("mecha-demote-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(engine_root(&home)).unwrap();
        point_current(&home, "b20000").unwrap();
        settle_current_on_pin(&home).unwrap();
        assert_eq!(link_tag(&current(&home)).as_deref(), Some("b20000"));
        // A first install, and a machine on the pin, get the pin.
        std::fs::remove_file(current(&home)).unwrap();
        settle_current_on_pin(&home).unwrap();
        assert_eq!(link_tag(&current(&home)).as_deref(), Some(PIN.tag));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn driver_versions_parse() {
        assert_eq!(parse_driver("580.173.02\n"), Some((580, 173)));
        assert_eq!(parse_driver("535.104.05\n535.104.05\n"), Some((535, 104)));
        assert_eq!(parse_driver("[N/A]"), None);
        assert_eq!(parse_driver(""), None);
    }

    #[test]
    fn a_gpu_is_read_from_list_devices() {
        assert!(lists_a_gpu(
            "Available devices:\n  CUDA0: NVIDIA GB10 (124609 MiB, 44089 MiB free)\n"
        ));
        assert!(lists_a_gpu(
            "Available devices:\n  MTL0: Apple M2 (10922 MiB)\n"
        ));
        assert!(!lists_a_gpu("Available devices:\n"));
        assert!(!lists_a_gpu(
            "ggml_cuda_init: no CUDA devices\nAvailable devices:\n"
        ));
    }

    /// A minimal ELF64 with one PT_LOAD over the whole file and a dynamic
    /// section naming `needed` and a RUNPATH.
    fn elf(needed: &[&str], runpath: Option<&str>) -> Vec<u8> {
        let mut strtab = vec![0u8];
        let mut add = |s: &str| {
            let o = strtab.len() as u64;
            strtab.extend_from_slice(s.as_bytes());
            strtab.push(0);
            o
        };
        let needed: Vec<u64> = needed.iter().map(|n| add(n)).collect();
        let rp = runpath.map(&mut add);
        let mut dynamic: Vec<(u64, u64)> = needed.iter().map(|o| (1, *o)).collect();
        if let Some(o) = rp {
            dynamic.push((29, o));
        }
        let ph = 64usize;
        let dyn_off = ph + 2 * 56;
        let str_off = dyn_off + (dynamic.len() + 2) * 16;
        dynamic.push((5, str_off as u64 + 0x1000));
        dynamic.push((0, 0));
        let mut b = vec![0u8; str_off];
        b[..4].copy_from_slice(b"\x7fELF");
        b[4] = 2;
        b[5] = 1;
        b[0x20..0x28].copy_from_slice(&(ph as u64).to_le_bytes());
        b[0x36..0x38].copy_from_slice(&56u16.to_le_bytes());
        b[0x38..0x3a].copy_from_slice(&2u16.to_le_bytes());
        let total = (str_off + strtab.len()) as u64;
        // PT_LOAD: offset 0 at vaddr 0x1000.
        b[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes());
        b[ph + 16..ph + 24].copy_from_slice(&0x1000u64.to_le_bytes());
        b[ph + 32..ph + 40].copy_from_slice(&total.to_le_bytes());
        // PT_DYNAMIC.
        let d = ph + 56;
        b[d..d + 4].copy_from_slice(&2u32.to_le_bytes());
        b[d + 8..d + 16].copy_from_slice(&(dyn_off as u64).to_le_bytes());
        b[d + 32..d + 40].copy_from_slice(&((dynamic.len() * 16) as u64).to_le_bytes());
        for (i, (t, v)) in dynamic.iter().enumerate() {
            let o = dyn_off + i * 16;
            b[o..o + 8].copy_from_slice(&t.to_le_bytes());
            b[o + 8..o + 16].copy_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&strtab);
        b
    }

    #[test]
    fn the_dynamic_section_reads() {
        let d = elf_dynamic(&elf(&["libggml.so.0", "libc.so.6"], Some("$ORIGIN")))
            .unwrap()
            .unwrap();
        assert_eq!(d.needed, vec!["libggml.so.0", "libc.so.6"]);
        assert_eq!(d.runpath.as_deref(), Some("$ORIGIN"));
        assert_eq!(d.rpath, None);
        assert_eq!(elf_dynamic(b"#!/bin/sh\n").unwrap(), None);
        assert_eq!(elf_dynamic(b"MIT License").unwrap(), None);
    }

    /// An ELF the parser gives up on is an error, never a skip: a shipped
    /// library whose strings cannot be found would otherwise pass the check
    /// unread.
    #[test]
    fn an_elf_that_does_not_parse_stops_the_check() {
        // Truncated inside the program headers.
        let full = elf(&["libggml.so.0"], Some("$ORIGIN"));
        assert!(elf_dynamic(&full[..80]).is_err());
        // DT_STRTAB pointing outside every PT_LOAD.
        let mut bad = full.clone();
        let ph = 64;
        let total = bad.len() as u64;
        bad[ph + 32..ph + 40].copy_from_slice(&(total / 4).to_le_bytes());
        let err = elf_dynamic(&bad).unwrap_err();
        assert!(format!("{err:#}").contains("do not parse"), "{err:#}");
        // A 32-bit ELF is not read as anything.
        let mut elf32 = full;
        elf32[4] = 1;
        assert!(elf_dynamic(&elf32).is_err());
        // And `self_contained` stops on it, naming the file.
        let dir = std::env::temp_dir().join(format!("mecha-engine-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("libggml-cuda.so"), &bad).unwrap();
        let err = format!("{:#}", self_contained(&dir).unwrap_err());
        assert!(err.contains("libggml-cuda.so"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The from-source failure §10.3 records: a build tree's absolute path in
    /// RUNPATH. A copied engine with it would run the old tree's libraries.
    #[test]
    fn an_absolute_runpath_is_refused_and_origin_passes() {
        let dir = std::env::temp_dir().join(format!("mecha-engine-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("libggml.so.0"), elf(&["libc.so.6"], None)).unwrap();
        std::fs::write(
            dir.join("llama-server"),
            elf(&["libggml.so.0"], Some("$ORIGIN")),
        )
        .unwrap();
        std::fs::write(dir.join("LICENSE"), "MIT").unwrap();
        self_contained(&dir).unwrap();

        std::fs::write(
            dir.join("llama-server"),
            elf(&["libggml.so.0"], Some("/home/someone/llama.cpp/build/bin")),
        )
        .unwrap();
        let err = self_contained(&dir).unwrap_err().to_string();
        assert!(err.contains("/home/someone/llama.cpp/build/bin"), "{err}");

        // Needing a shipped library with no RUNPATH at all is refused too.
        std::fs::write(dir.join("llama-server"), elf(&["libggml.so.0"], None)).unwrap();
        assert!(self_contained(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_build_record_with_an_unknown_target_still_reads() {
        let b: Build =
            serde_json::from_str(r#"{"tag":"b1","commit":"abc","target":"linux-riscv64-vulkan"}"#)
                .unwrap();
        assert_eq!(b.target, None);
        let b: Build =
            serde_json::from_str(r#"{"tag":"b1","commit":"abc","target":"linux-arm64-cuda13"}"#)
                .unwrap();
        assert_eq!(b.target, Some(Target::LinuxArm64Cuda13));
    }

    /// The upgrade's install path, over the network: today's newest `b`
    /// release resolved through the API, fetched by its published digests,
    /// unpacked and checked to report the build and the commit its tag names
    /// — beside, never over, `current`, which stays unset.
    #[tokio::test]
    #[ignore]
    async fn an_owner_confirmed_release_installs_side_by_side() {
        let target = choose(std::env::consts::OS, std::env::consts::ARCH, read_nvidia()).unwrap();
        let release = resolve_release(None, target).await.unwrap();
        let root = std::env::temp_dir().join(format!("mecha-upgrade-{}", uuid::Uuid::new_v4()));
        let m = Machinery {
            home: root.join("home"),
            mecha_home: root.join("home/.mecha"),
            unit_dirs: vec![],
            path: vec![],
            docker: Box::new(|_| crate::sidecar::Lookup::Absent),
        };
        Manifest::begin(&m.mecha_home, "llama").unwrap();
        let spec = Spec {
            tag: release.tag.clone(),
            build: release.build,
            commit: release.commit.clone(),
            source: Source::OwnerConfirmed(release.archives_for(target).unwrap()),
        };
        let dir = install_build(&m, target, &spec, &mut |s| eprintln!("{s}"))
            .await
            .unwrap();
        eprintln!(
            "installed {} ({}) at {}",
            release.tag,
            &release.commit[..9],
            dir.display()
        );
        let v = engine_output(&dir.join("llama-server"), "--version").unwrap();
        assert!(reports(&v, release.build, &release.commit), "{v}");
        assert!(
            std::fs::symlink_metadata(current(&m.mecha_home)).is_err(),
            "current untouched"
        );
        let man = Manifest::read(&m.mecha_home).unwrap();
        assert!(man.entries[0].builds.iter().any(|b| b.tag == release.tag));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The real thing, over the network (~700 MB on a CUDA machine): fetch,
    /// unpack, check, link — then again, which fetches nothing.
    #[tokio::test]
    #[ignore]
    async fn the_engine_installs_end_to_end_and_again() {
        let root = std::env::temp_dir().join(format!("mecha-engine-{}", uuid::Uuid::new_v4()));
        let m = Machinery {
            home: root.join("home"),
            mecha_home: root.join("home/.mecha"),
            unit_dirs: vec![],
            path: vec![],
            docker: Box::new(|_| crate::sidecar::Lookup::Absent),
        };
        let mut log = Vec::new();
        let server = install_engine(&m, &mut |s| log.push(s.to_string()))
            .await
            .unwrap();
        assert!(server.is_file(), "{}", server.display());
        // The check read the real thing, not nothing: the server's dynamic
        // section parses, names a library shipped beside it, and finds it
        // through `$ORIGIN`.
        let d = elf_dynamic(&std::fs::read(&server).unwrap())
            .unwrap()
            .expect("an ELF");
        assert_eq!(d.runpath.as_deref(), Some("$ORIGIN"), "{d:?}");
        assert!(d.needed.iter().any(|n| n.starts_with("libllama")), "{d:?}");
        let man = Manifest::read(&m.mecha_home).unwrap();
        let e = man.entries.iter().find(|e| e.sidecar == "llama").unwrap();
        assert!(!e.incomplete);
        assert_eq!(e.builds[0].commit, PIN.commit);
        assert!(log.iter().any(|l| l.starts_with("fetching")), "{log:?}");
        let mut again = Vec::new();
        install_engine(&m, &mut |s| again.push(s.to_string()))
            .await
            .unwrap();
        assert!(
            !again.iter().any(|l| l.starts_with("fetching")),
            "a second install fetched: {again:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
