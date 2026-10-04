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

/// Install the pinned engine for this machine: fetch, unpack, check, link.
/// Idempotent — a build already unpacked and answering is linked, not fetched.
pub async fn install_engine(m: &Machinery, say: Say<'_>) -> Result<PathBuf> {
    let target = choose(std::env::consts::OS, std::env::consts::ARCH, read_nvidia())
        .map_err(anyhow::Error::msg)?;
    install_target(m, target, say).await
}

async fn install_target(m: &Machinery, target: Target, say: Say<'_>) -> Result<PathBuf> {
    let id = "llama";
    let home = &m.mecha_home;
    let asset = asset(target).with_context(|| format!("no asset pinned for {target:?}"))?;
    let root = engine_root(home);
    let dir = root.join(PIN.tag);
    let link = current(home);
    Manifest::begin(home, id)?;
    Manifest::record(home, id, &root)?;
    Manifest::record(home, id, &dir)?;
    Manifest::record(home, id, &link)?;
    std::fs::create_dir_all(&root)?;

    // The unpacked tree is this machine's build only when the record says
    // it was unpacked for this target: a tree for another one (installed
    // under an older driver) passes every check under the same tag, and
    // recording it as the new target would make the record lie.
    if unpacked_target(home)? != Some(target) || health(&dir, target).is_err() {
        // Unpacked into a scratch directory and renamed into place, so a
        // run interrupted mid-unpack leaves no tree that looks like a build.
        let part = root.join(format!("{}.part", PIN.tag));
        let _ = std::fs::remove_dir_all(&part);
        std::fs::create_dir_all(&part)?;
        let downloads = root.join("download");
        for (name, sha, bytes) in asset.archives {
            say(&format!(
                "fetching {name} ({:.0} MiB)",
                *bytes as f64 / 1_048_576.0
            ));
            let archive = crate::fetch::fetch_release_asset(
                REPO,
                PIN.tag,
                name,
                sha,
                *bytes,
                &downloads,
                &mut |_| {},
            )
            .await?;
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
            PIN.tag,
            target.label()
        ));
        health(&part, target)?;
        if dir.exists() {
            std::fs::remove_dir_all(&dir).with_context(|| format!("clearing {}", dir.display()))?;
        }
        std::fs::rename(&part, &dir)?;
        let _ = std::fs::remove_dir_all(&downloads);
    }

    // The link names the build by its directory's name, relative, so the
    // home can move; swapped in one rename.
    let tmp = root.join("current.new");
    let _ = std::fs::remove_file(&tmp);
    #[cfg(unix)]
    std::os::unix::fs::symlink(PIN.tag, &tmp)?;
    std::fs::rename(&tmp, &link)?;

    Manifest::record_build(
        home,
        id,
        Build {
            tag: PIN.tag.into(),
            commit: PIN.commit.into(),
            target: Some(target),
        },
    )?;
    Manifest::finish(home, id)?;
    Ok(link.join("llama-server"))
}

/// The target the pinned tag was unpacked for, as the manifest records it;
/// `None` when no build of the tag is recorded, or its target is one this
/// mecha does not know.
fn unpacked_target(home: &Path) -> Result<Option<Target>> {
    Ok(Manifest::read(home)?
        .entries
        .iter()
        .find(|e| e.sidecar == "llama")
        .and_then(|e| e.builds.iter().find(|b| b.tag == PIN.tag))
        .and_then(|b| b.target))
}

/// The three checks a build passes before it counts (module doc).
fn health(dir: &Path, target: Target) -> Result<()> {
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
    if !reports_the_pin(&version) {
        bail!(
            "{} reports `{}`, not build {} at {}",
            server.display(),
            version.lines().next().unwrap_or("").trim(),
            PIN.build,
            &PIN.commit[..9]
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
/// release builder's `git rev-parse --short`, so any prefix of the pinned
/// commit of seven or more characters is the pin.
fn reports_the_pin(text: &str) -> bool {
    let Some(line) = text.lines().find(|l| l.contains("version:")) else {
        return false;
    };
    let build = format!("build {},", PIN.build);
    let Some(rest) = line.split_once("commit ").map(|(_, r)| r) else {
        return false;
    };
    let commit: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
    line.contains(&build) && commit.len() >= 7 && PIN.commit.starts_with(&commit)
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
    for entry in d.chunks_exact(16) {
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
        let err = health(&dir, Target::LinuxArm64Cuda13)
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
        assert_eq!(unpacked_target(&home).unwrap(), None);
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
            unpacked_target(&home).unwrap(),
            Some(Target::LinuxX64Cuda12)
        );
        assert_ne!(
            unpacked_target(&home).unwrap(),
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
