//! What a feature runs beside mecha, whether this machine already has it,
//! and the plan an install would follow (FEATURES-DESIGN.md §10.2, step 7a-2).
//!
//! **A closed registry.** A [`Sidecar`] is a program, not mecha's own, that a
//! feature needs running — the llama.cpp engine, the router, ComfyUI, the
//! voice worker. Models are not sidecars: they are [`crate::recommend`]'s
//! slots, and the plan prices each pinned file from the hub cache
//! ([`crate::fetch::cached`]).
//!
//! **Provided, never installed over (F8).** A sidecar is *provided* when this
//! machine shows it — a systemd unit of its name that mecha did not write, a
//! binary on `PATH`, a directory, a Docker image — outside
//! `~/.mecha/sidecars/`, which is mecha's own tree. Every check here is a
//! file's existence, a `PATH` lookup or `docker image ls`: none opens a
//! socket, so none can start a socket-activated server (§4.3), and an
//! idle-stopped server whose unit exists reads provided, never absent (the
//! llama-embed incident of 2026-08-19).
//!
//! **Unknown is never absent.** A check that could not run — an unreadable
//! directory, a manifest that does not parse — is reported as unknown, and a
//! plan with an unknown in it says so rather than offering an install over
//! it.
//!
//! **Nothing installs yet.** Each sidecar names the step whose installer
//! brings it (7a-3 to 7f); until then the plan is read-only.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::feature::Feature;
use crate::recommend::{self, Machine, Source};

/// How a hand install of a sidecar shows itself on a machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub enum Evidence {
    /// A systemd user unit of this name, in any of the user unit directories.
    UserUnit(&'static str),
    /// A binary on `PATH`.
    OnPath(&'static str),
    /// A path under the home directory.
    Home(&'static str),
    /// A path under the mecha home — outside `sidecars/`, so a hand install.
    MechaHome(&'static str),
    /// A Docker image, by repository (any tag).
    DockerImage(&'static str),
}

/// A program a feature runs beside mecha.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Sidecar {
    pub id: &'static str,
    pub label: &'static str,
    /// Every feature that needs it; empty for what every feature needs.
    pub needed_by: &'static [Feature],
    /// The `recommend` slots whose models it runs.
    pub serves: &'static [&'static str],
    /// Any one of these makes it provided.
    pub evidence: &'static [Evidence],
    /// The build step whose installer brings it (§10.6).
    pub installer: &'static str,
    /// Its models live in the Hugging Face hub. `false` for ComfyUI, which
    /// keeps them in its own `models/` tree: a provided ComfyUI holds its
    /// own, and the plan does not read or price them.
    pub models_in_hub: bool,
}

/// The registry. Order is the order a plan reads.
pub const SIDECARS: &[Sidecar] = &[
    Sidecar {
        id: "llama",
        label: "llama.cpp (llama-server)",
        needed_by: &[],
        serves: &["chat", "embeddings", "ocr"],
        evidence: &[Evidence::OnPath("llama-server")],
        installer: "7b",
        models_in_hub: true,
    },
    Sidecar {
        id: "router",
        label: "the chat router",
        needed_by: &[],
        serves: &["chat"],
        evidence: &[Evidence::UserUnit("llama-local.service")],
        installer: "7c",
        models_in_hub: true,
    },
    Sidecar {
        id: "embed-server",
        label: "the embeddings server",
        needed_by: &[Feature::Graph, Feature::Documents, Feature::Personas],
        serves: &["embeddings"],
        evidence: &[
            Evidence::UserUnit("llama-embed.socket"),
            Evidence::UserUnit("llama-embed.service"),
        ],
        installer: "7c",
        models_in_hub: true,
    },
    Sidecar {
        id: "ocr-server",
        label: "the OCR server",
        needed_by: &[Feature::Ocr],
        serves: &["ocr"],
        evidence: &[
            Evidence::UserUnit("llama-ocr.socket"),
            Evidence::UserUnit("llama-ocr.service"),
        ],
        installer: "7c",
        models_in_hub: true,
    },
    Sidecar {
        id: "layout",
        label: "the layout model's environment",
        needed_by: &[Feature::Layout],
        serves: &["layout"],
        evidence: &[Evidence::MechaHome("layout/venv")],
        installer: "7a-3",
        models_in_hub: true,
    },
    Sidecar {
        id: "comfyui",
        label: "ComfyUI",
        needed_by: &[Feature::Image],
        serves: &["image"],
        evidence: &[
            Evidence::UserUnit("comfyui.service"),
            Evidence::Home("ComfyUI"),
        ],
        installer: "7e",
        models_in_hub: false,
    },
    Sidecar {
        id: "stt",
        label: "the speech-to-text server",
        needed_by: &[Feature::Voice],
        serves: &["stt"],
        evidence: &[Evidence::UserUnit("mecha-parakeet.service")],
        installer: "7d",
        models_in_hub: true,
    },
    Sidecar {
        id: "voice-worker",
        label: "the voice worker",
        needed_by: &[Feature::Voice],
        serves: &["turn"],
        evidence: &[Evidence::UserUnit("mecha-voice-worker.service")],
        installer: "7d",
        models_in_hub: true,
    },
    // The speech server: Breeze and its OpenAI-shaped adapter, the default
    // since 2026-10-03, or the Chatterbox container before it — either
    // serves the `tts` slot, so a machine still on Chatterbox is provided,
    // never offered Breeze over a working speech server (F8).
    Sidecar {
        id: "speech",
        label: "the speech server",
        needed_by: &[Feature::Voice],
        serves: &["tts"],
        evidence: &[
            Evidence::UserUnit("mecha-breeze-tts.service"),
            Evidence::UserUnit("mecha-breeze-adapter.service"),
            Evidence::DockerImage("mecha/chatterbox"),
        ],
        installer: "7f",
        models_in_hub: true,
    },
];

/// The record of what mecha itself wrote under `~/.mecha/sidecars/` (§10.2
/// item 2): read here, written by the installers from 7a-3 on.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub sidecar: String,
    /// Written before the first byte, cleared when the health check passes,
    /// so an interrupted install reads resumable, never provided.
    #[serde(default)]
    pub incomplete: bool,
    /// Every unit, file or directory the install wrote, by path.
    #[serde(default)]
    pub wrote: Vec<PathBuf>,
}

impl Manifest {
    pub fn path(mecha_home: &Path) -> PathBuf {
        mecha_home.join("sidecars").join("manifest.json")
    }

    /// Absent is empty; unreadable or unparseable is an error — a manifest
    /// that cannot be read cannot say what mecha wrote, so nothing may be
    /// taken as someone else's on its word.
    pub fn read(mecha_home: &Path) -> Result<Manifest> {
        let p = Manifest::path(mecha_home);
        match std::fs::read_to_string(&p) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("{} does not parse", p.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Manifest::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", p.display())),
        }
    }

    /// Write the manifest whole, through a temporary file and a rename, so a
    /// crash leaves the old record or the new one, never half of either.
    pub fn write(&self, mecha_home: &Path) -> Result<()> {
        let p = Manifest::path(mecha_home);
        let dir = p.parent().context("a manifest path with no parent")?;
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join("manifest.json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, &p).with_context(|| format!("writing {}", p.display()))?;
        Ok(())
    }

    /// Begin an install: the entry is written, marked incomplete, before the
    /// first byte (§10.2 item 4), so an interruption reads resumable.
    pub fn begin(mecha_home: &Path, id: &str) -> Result<()> {
        let mut m = Manifest::read(mecha_home)?;
        match m.entries.iter_mut().find(|e| e.sidecar == id) {
            Some(e) => e.incomplete = true,
            None => m.entries.push(Entry {
                sidecar: id.to_string(),
                incomplete: true,
                wrote: vec![],
            }),
        }
        m.write(mecha_home)
    }

    /// Record a path the install is about to write, before writing it.
    pub fn record(mecha_home: &Path, id: &str, path: &Path) -> Result<()> {
        let mut m = Manifest::read(mecha_home)?;
        let e = m
            .entries
            .iter_mut()
            .find(|e| e.sidecar == id)
            .with_context(|| format!("recording {} before its install began", path.display()))?;
        if !e.wrote.iter().any(|w| w == path) {
            e.wrote.push(path.to_path_buf());
        }
        m.write(mecha_home)
    }

    /// The health check passed: the entry is complete.
    pub fn finish(mecha_home: &Path, id: &str) -> Result<()> {
        let mut m = Manifest::read(mecha_home)?;
        let e = m
            .entries
            .iter_mut()
            .find(|e| e.sidecar == id)
            .with_context(|| format!("finishing {id} before its install began"))?;
        e.incomplete = false;
        m.write(mecha_home)
    }

    fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.sidecar == id)
    }

    fn wrote(&self, path: &Path) -> bool {
        self.entries
            .iter()
            .any(|e| e.wrote.iter().any(|w| w == path))
    }
}

/// Where the checks look — the real machine, or a test's.
pub struct Machinery {
    pub home: PathBuf,
    pub mecha_home: PathBuf,
    pub unit_dirs: Vec<PathBuf>,
    pub path: Vec<PathBuf>,
    pub docker: Box<dyn Fn(&str) -> Lookup>,
}

/// One check's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    /// What showed it, in words, and the path it was found at (none for a
    /// Docker image).
    Found(String, Option<PathBuf>),
    Absent,
    Unknown(String),
}

impl Machinery {
    pub fn real() -> Result<Machinery> {
        let home = dirs::home_dir().context("no home directory")?;
        let mecha_home = crate::work::mecha_home()?;
        // systemd's user search path, as `Facts::read` finds the config
        // directory (`dirs`, so `XDG_CONFIG_HOME` and `XDG_DATA_HOME` count):
        // a package's unit in /usr/share is as much a hand install as one in
        // ~/.config.
        let mut unit_dirs: Vec<PathBuf> = [dirs::config_dir(), dirs::data_dir()]
            .into_iter()
            .flatten()
            .map(|d| d.join("systemd/user"))
            .collect();
        unit_dirs.extend(
            [
                "/etc/systemd/user",
                "/usr/local/lib/systemd/user",
                "/usr/local/share/systemd/user",
                "/usr/lib/systemd/user",
                "/usr/share/systemd/user",
            ]
            .map(PathBuf::from),
        );
        let path = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        Ok(Machinery {
            home,
            mecha_home,
            unit_dirs,
            path,
            docker: Box::new(docker_image),
        })
    }

    fn sidecars_dir(&self) -> PathBuf {
        self.mecha_home.join("sidecars")
    }

    fn exists(&self, p: &Path) -> Lookup {
        match std::fs::symlink_metadata(p) {
            Ok(_) => Lookup::Found(p.display().to_string(), Some(p.to_path_buf())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Lookup::Absent,
            Err(e) => Lookup::Unknown(format!("{}: {e}", p.display())),
        }
    }

    fn check(&self, ev: &Evidence) -> Lookup {
        match ev {
            Evidence::UserUnit(name) => {
                let mut unknown = None;
                for d in &self.unit_dirs {
                    match self.exists(&d.join(name)) {
                        Lookup::Found(p, path) => {
                            return Lookup::Found(format!("unit {name} ({p})"), path)
                        }
                        Lookup::Unknown(why) => unknown = Some(why),
                        Lookup::Absent => {}
                    }
                }
                unknown.map_or(Lookup::Absent, Lookup::Unknown)
            }
            Evidence::OnPath(bin) => {
                // An entry that cannot be read is unknown, never absent: a
                // machine that may well have the binary must not be offered
                // an install over it.
                let mut unknown = None;
                for d in &self.path {
                    let p = d.join(bin);
                    match std::fs::metadata(&p) {
                        Ok(m) if m.is_file() => {
                            return Lookup::Found(format!("{} on PATH", p.display()), Some(p));
                        }
                        Ok(_) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => unknown = Some(format!("{}: {e}", p.display())),
                    }
                }
                unknown.map_or(Lookup::Absent, Lookup::Unknown)
            }
            Evidence::Home(rel) => self.exists(&self.home.join(rel)),
            Evidence::MechaHome(rel) => self.exists(&self.mecha_home.join(rel)),
            Evidence::DockerImage(repo) => (self.docker)(repo),
        }
    }
}

/// `docker image ls <repo>` — asks the daemon for its image list, which
/// starts nothing. No docker, or a daemon that does not answer, is unknown
/// only when docker is installed; with no docker at all it is absent.
fn docker_image(repo: &str) -> Lookup {
    let out = std::process::Command::new("docker")
        .args(["image", "ls", "--format", "{{.Repository}}:{{.Tag}}", repo])
        .output();
    match out {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Lookup::Absent,
        Err(e) => Lookup::Unknown(format!("docker: {e}")),
        Ok(o) if !o.status.success() => Lookup::Unknown(format!(
            "docker image ls: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        )),
        Ok(o) => match String::from_utf8_lossy(&o.stdout).lines().next() {
            Some(img) if !img.trim().is_empty() => {
                Lookup::Found(format!("docker image {}", img.trim()), None)
            }
            _ => Lookup::Absent,
        },
    }
}

/// What a plan says about one sidecar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SidecarState {
    /// Shown by this machine, and not by anything mecha wrote: left alone.
    Provided { by: String },
    /// mecha installed it.
    Installed,
    /// mecha began installing it and did not finish: resumable.
    Incomplete,
    /// Not here; its installer arrives in `step`.
    Missing { step: &'static str },
    /// A check could not run — never read as missing.
    Unknown { why: String },
}

/// What a plan says about one pinned model file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum FileState {
    /// In the hub cache and matching its pin: costs nothing.
    Cached,
    /// To download, at its pinned size.
    Download { bytes: u64 },
    /// Something is at its snapshot path and does not match: not mecha's to
    /// replace, and not handed to a launcher.
    Mismatch,
    /// Comes with its sidecar's own install (a package that fetches it).
    WithSidecar,
    /// Kept by a provided sidecar outside the hub (ComfyUI's `models/`):
    /// that install's own, not read here.
    HeldBy { sidecar: &'static str },
    /// A plain file of the pinned size at its path, put there by hand and not
    /// hashed by the plan.
    Unverified,
    /// No model is recommended for this machine's tier.
    NoRow,
    /// Kept outside the hub by a sidecar that could not be checked: not
    /// priced, never offered over.
    KeeperUnknown { sidecar: &'static str },
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannedSidecar {
    pub id: &'static str,
    pub label: &'static str,
    pub state: SidecarState,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannedFile {
    pub slot: &'static str,
    pub model: &'static str,
    pub repo: Option<&'static str>,
    pub path: &'static str,
    pub state: FileState,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub feature: Feature,
    pub sidecars: Vec<PlannedSidecar>,
    pub files: Vec<PlannedFile>,
    /// The sum of every file to download; what is cached costs nothing.
    pub download_bytes: u64,
    /// Every piece is provided, installed or cached: nothing to do.
    pub nothing_to_do: bool,
}

/// One pinned hub file's state. Only what the cheap check cannot vouch for
/// is read, and only with `verify`: a blob named by its sha256 is that hash.
fn file_state(
    hub: &Path,
    repo: &str,
    revision: &str,
    f: &crate::recommend::HubFile,
    verify: bool,
) -> Result<FileState> {
    use crate::fetch::{cached, Cached};
    let quick = cached(hub, repo, revision, f, false)?;
    let found = if verify && quick == Cached::Unverified {
        cached(hub, repo, revision, f, true)?
    } else {
        quick
    };
    Ok(match found {
        Cached::Verified => FileState::Cached,
        Cached::Mismatch => FileState::Mismatch,
        Cached::Unverified => FileState::Unverified,
        Cached::Absent => FileState::Download { bytes: f.bytes },
    })
}

/// Whether `f` is the feature named or one of its parts.
fn within(named: Feature, f: Feature) -> bool {
    f == named || f.switch_owner() == named
}

/// The plan for a feature (a part resolves to its parent, as `mecha setup
/// <part>` does): every sidecar it or its parts need, and every pinned model
/// file the machine's rows name. The shared ones — the engine, the router,
/// the chat model — are in every plan, since nothing works without them.
pub fn plan(
    named: Feature,
    m: &Machinery,
    machine: &Machine,
    hub: &Path,
    verify: bool,
) -> Result<Plan> {
    let named = named.switch_owner();
    let manifest = Manifest::read(&m.mecha_home)?;
    let sidecars_dir = m.sidecars_dir();
    let needs =
        |needed_by: &[Feature]| needed_by.is_empty() || needed_by.iter().any(|f| within(named, *f));

    let mut sidecars = Vec::new();
    for s in SIDECARS.iter().filter(|s| needs(s.needed_by)) {
        // What the machine shows, read only when mecha's record does not
        // settle it.
        let from_evidence = || {
            let mut state = SidecarState::Missing { step: s.installer };
            for ev in s.evidence {
                match m.check(ev) {
                    Lookup::Found(by, path) => {
                        // Something mecha wrote is never someone else's: not
                        // under its own tree, and not in its record.
                        let ours = path
                            .as_deref()
                            .is_some_and(|p| p.starts_with(&sidecars_dir) || manifest.wrote(p));
                        if !ours {
                            return SidecarState::Provided { by };
                        }
                    }
                    Lookup::Unknown(why) => state = SidecarState::Unknown { why },
                    Lookup::Absent => {}
                }
            }
            state
        };
        // The record says what was written; the disk says whether it is
        // still there. All of it: installed (or incomplete, if the install
        // never finished). Some: incomplete — resumable. None: the record is
        // stale, and the machine is read as if it were not there.
        let state = match manifest.entry(s.id) {
            None => from_evidence(),
            // Written before the first byte: nothing on disk yet, and
            // resumable — the one state `incomplete` exists to name.
            Some(e) if e.wrote.is_empty() && e.incomplete => SidecarState::Incomplete,
            Some(e) if e.wrote.is_empty() => SidecarState::Unknown {
                why: format!("mecha's record for {} names nothing it wrote", s.id),
            },
            Some(e) => {
                let mut present = 0;
                let mut unknown = None;
                for w in &e.wrote {
                    match m.exists(w) {
                        Lookup::Found(..) => present += 1,
                        Lookup::Unknown(why) => unknown = Some(why),
                        Lookup::Absent => {}
                    }
                }
                match (unknown, present) {
                    (Some(why), _) => SidecarState::Unknown { why },
                    (None, 0) => from_evidence(),
                    (None, n) if n == e.wrote.len() && !e.incomplete => SidecarState::Installed,
                    (None, _) => SidecarState::Incomplete,
                }
            }
        };
        sidecars.push(PlannedSidecar {
            id: s.id,
            label: s.label,
            state,
        });
    }

    let mut files = Vec::new();
    for slot in recommend::SLOTS.iter().filter(|s| needs(s.needed_by)) {
        let Some((row, _)) = recommend::row_for(slot, machine) else {
            // No model is recommended at this tier (the chat model below its
            // smallest): named, never skipped — a slot with no row is a
            // finding, not an empty queue.
            files.push(PlannedFile {
                slot: slot.id,
                model: "",
                repo: None,
                path: "",
                state: FileState::NoRow,
            });
            continue;
        };
        // A model kept outside the hub by a provided sidecar is that
        // install's: named, not priced.
        // A model kept outside the hub by a sidecar is that sidecar's: held
        // when it is here (provided or installed), fetched by its own
        // installer when that install is incomplete, and — when it could
        // not be checked — not priced at all, since nothing is offered over
        // an unknown.
        let keeper = SIDECARS
            .iter()
            .find(|sc| !sc.models_in_hub && sc.serves.contains(&slot.id))
            .and_then(|sc| {
                sidecars
                    .iter()
                    .find(|p| p.id == sc.id)
                    .map(|p| (sc, &p.state))
            });
        let held = match keeper {
            Some((sc, SidecarState::Provided { .. } | SidecarState::Installed)) => {
                Some(FileState::HeldBy { sidecar: sc.label })
            }
            Some((_, SidecarState::Incomplete)) => Some(FileState::WithSidecar),
            Some((sc, SidecarState::Unknown { .. })) => {
                Some(FileState::KeeperUnknown { sidecar: sc.label })
            }
            Some((_, SidecarState::Missing { .. })) | None => None,
        };
        if let Some(state) = held {
            files.push(PlannedFile {
                slot: slot.id,
                model: row.model,
                repo: None,
                path: "",
                state,
            });
            continue;
        }
        if row.sources.is_empty() {
            files.push(PlannedFile {
                slot: slot.id,
                model: row.model,
                repo: None,
                path: "",
                state: FileState::WithSidecar,
            });
        }
        for src in row.sources {
            match src {
                Source::HuggingFace {
                    repo,
                    revision,
                    files: fs,
                } => {
                    for f in *fs {
                        let state = file_state(hub, repo, revision, f, verify)?;
                        files.push(PlannedFile {
                            slot: slot.id,
                            model: row.model,
                            repo: Some(repo),
                            path: f.path,
                            state,
                        });
                    }
                }
                // Release assets and locks are fetched by their sidecar's
                // installer, not into the hub (7b, 7d).
                other => files.push(PlannedFile {
                    slot: slot.id,
                    model: row.model,
                    repo: None,
                    path: match other {
                        Source::ReleaseAsset { asset, .. } => asset,
                        Source::GitCommit { url, .. } => url,
                        Source::PythonLock { lock, .. } => lock,
                        Source::HuggingFace { .. } => unreachable!(),
                    },
                    state: FileState::WithSidecar,
                }),
            }
        }
    }

    let download_bytes = files
        .iter()
        .map(|f| match f.state {
            FileState::Download { bytes } => bytes,
            _ => 0,
        })
        .sum();
    let nothing_to_do = sidecars.iter().all(|s| {
        matches!(
            s.state,
            SidecarState::Provided { .. } | SidecarState::Installed
        )
    }) && files.iter().all(|f| {
        matches!(
            f.state,
            FileState::Cached | FileState::WithSidecar | FileState::HeldBy { .. }
        )
    });
    Ok(Plan {
        feature: named,
        sidecars,
        files,
        download_bytes,
        nothing_to_do,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recommend::Machine;

    fn scratch() -> PathBuf {
        let d = std::env::temp_dir().join(format!("mecha-sidecar-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn machinery(root: &Path) -> Machinery {
        Machinery {
            home: root.join("home"),
            mecha_home: root.join("home/.mecha"),
            unit_dirs: vec![root.join("home/.config/systemd/user")],
            path: vec![root.join("bin")],
            docker: Box::new(|_| Lookup::Absent),
        }
    }

    const GB10: Machine = Machine::Unified {
        total_mb: 124_610,
        gpu_unread: false,
    };

    #[test]
    fn every_sidecar_is_named_once_needs_real_slots_and_a_step() {
        let mut ids = std::collections::HashSet::new();
        for s in SIDECARS {
            assert!(ids.insert(s.id), "two sidecars named {}", s.id);
            assert!(
                !s.evidence.is_empty(),
                "{}: nothing could ever show it provided",
                s.id
            );
            for slot in s.serves {
                assert!(
                    recommend::SLOTS.iter().any(|r| r.id == *slot),
                    "{}: no slot {slot}",
                    s.id
                );
            }
            assert!(
                ["7a-3", "7b", "7c", "7d", "7e", "7f"].contains(&s.installer),
                "{}: installer step {}",
                s.id,
                s.installer
            );
        }
    }

    /// An empty machine: every sidecar the feature needs is missing, with the
    /// step that brings it, and every pinned file is a download.
    #[test]
    fn on_an_empty_machine_everything_is_missing_and_priced() {
        let root = scratch();
        let m = machinery(&root);
        let hub = root.join("hub");
        let p = plan(Feature::Ocr, &m, &GB10, &hub, false).unwrap();
        assert_eq!(p.feature, Feature::Documents, "a part plans as its parent");
        let ocr = p.sidecars.iter().find(|s| s.id == "ocr-server").unwrap();
        assert_eq!(ocr.state, SidecarState::Missing { step: "7c" });
        let layout = p.sidecars.iter().find(|s| s.id == "layout").unwrap();
        assert_eq!(layout.state, SidecarState::Missing { step: "7a-3" });
        assert!(
            p.sidecars.iter().any(|s| s.id == "llama"),
            "the shared engine is in every plan"
        );
        assert!(
            !p.sidecars.iter().any(|s| s.id == "comfyui"),
            "image's sidecar is not documents'"
        );
        assert!(p.download_bytes > 0);
        assert!(!p.nothing_to_do);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A hand install shows itself — a unit, a binary, a directory — and is
    /// provided; an idle-stopped server is a unit file, so it reads provided,
    /// never missing, without anything being asked of it.
    #[test]
    fn hand_installs_are_provided_whatever_shows_them() {
        let root = scratch();
        let m = machinery(&root);
        std::fs::create_dir_all(root.join("home/.config/systemd/user")).unwrap();
        std::fs::write(root.join("home/.config/systemd/user/llama-ocr.socket"), "").unwrap();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::write(root.join("bin/llama-server"), "").unwrap();
        std::fs::create_dir_all(root.join("home/.mecha/layout/venv")).unwrap();
        let p = plan(Feature::Documents, &m, &GB10, &root.join("hub"), false).unwrap();
        for id in ["ocr-server", "llama", "layout"] {
            let s = p.sidecars.iter().find(|s| s.id == id).unwrap();
            assert!(
                matches!(s.state, SidecarState::Provided { .. }),
                "{id}: {:?}",
                s.state
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// mecha's own tree is never someone else's: an entry in the manifest is
    /// installed, or — still marked incomplete — resumable.
    #[test]
    fn what_the_manifest_records_is_mecha_s_not_provided() {
        let root = scratch();
        let m = machinery(&root);
        let tree = root.join("home/.mecha/sidecars");
        std::fs::create_dir_all(tree.join("layout")).unwrap();
        std::fs::create_dir_all(tree.join("ocr")).unwrap();
        let manifest = Manifest {
            entries: vec![
                Entry {
                    sidecar: "layout".into(),
                    incomplete: true,
                    wrote: vec![tree.join("layout")],
                },
                Entry {
                    sidecar: "ocr-server".into(),
                    incomplete: false,
                    wrote: vec![tree.join("ocr")],
                },
            ],
        };
        std::fs::create_dir_all(root.join("home/.mecha/sidecars")).unwrap();
        std::fs::write(
            Manifest::path(&m.mecha_home),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
        let p = plan(Feature::Documents, &m, &GB10, &root.join("hub"), false).unwrap();
        let state = |id: &str| {
            p.sidecars
                .iter()
                .find(|s| s.id == id)
                .unwrap()
                .state
                .clone()
        };
        assert_eq!(state("layout"), SidecarState::Incomplete);
        assert_eq!(state("ocr-server"), SidecarState::Installed);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The record is checked against the disk: an install whose files are
    /// all gone is read as if it were not there; one with some gone is
    /// incomplete, resumable; an entry that names nothing is unknown.
    #[test]
    fn mecha_s_record_is_checked_against_the_disk() {
        let root = scratch();
        let m = machinery(&root);
        let tree = root.join("home/.mecha/sidecars");
        std::fs::create_dir_all(tree.join("ocr/a")).unwrap();
        let manifest = Manifest {
            entries: vec![
                Entry {
                    sidecar: "layout".into(),
                    incomplete: false,
                    wrote: vec![tree.join("layout")],
                },
                Entry {
                    sidecar: "ocr-server".into(),
                    incomplete: false,
                    wrote: vec![tree.join("ocr/a"), tree.join("ocr/b")],
                },
                Entry {
                    sidecar: "embed-server".into(),
                    incomplete: false,
                    wrote: vec![],
                },
            ],
        };
        std::fs::write(
            Manifest::path(&m.mecha_home),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
        let p = plan(Feature::Documents, &m, &GB10, &root.join("hub"), false).unwrap();
        let state = |id: &str| {
            p.sidecars
                .iter()
                .find(|s| s.id == id)
                .unwrap()
                .state
                .clone()
        };
        assert_eq!(
            state("layout"),
            SidecarState::Missing { step: "7a-3" },
            "every file gone: stale record"
        );
        assert_eq!(
            state("ocr-server"),
            SidecarState::Incomplete,
            "some files gone: resumable"
        );
        assert!(matches!(
            state("embed-server"),
            SidecarState::Unknown { .. }
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// An install interrupted before its first byte has written nothing yet
    /// and is resumable — the state the `incomplete` flag exists to name.
    #[test]
    fn an_install_stopped_before_its_first_byte_is_resumable() {
        let root = scratch();
        let m = machinery(&root);
        let manifest = Manifest {
            entries: vec![Entry {
                sidecar: "layout".into(),
                incomplete: true,
                wrote: vec![],
            }],
        };
        std::fs::create_dir_all(root.join("home/.mecha/sidecars")).unwrap();
        std::fs::write(
            Manifest::path(&m.mecha_home),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
        let p = plan(Feature::Documents, &m, &GB10, &root.join("hub"), false).unwrap();
        let layout = p.sidecars.iter().find(|s| s.id == "layout").unwrap();
        assert_eq!(layout.state, SidecarState::Incomplete);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A machine still on the Chatterbox container has a speech server: it
    /// is provided, and Breeze is never offered over it.
    #[test]
    fn the_chatterbox_container_still_provides_speech() {
        let root = scratch();
        let mut m = machinery(&root);
        m.docker = Box::new(|repo| {
            if repo == "mecha/chatterbox" {
                Lookup::Found("docker image mecha/chatterbox:serve".into(), None)
            } else {
                Lookup::Absent
            }
        });
        let p = plan(Feature::Voice, &m, &GB10, &root.join("hub"), false).unwrap();
        let speech = p.sidecars.iter().find(|s| s.id == "speech").unwrap();
        assert!(
            matches!(speech.state, SidecarState::Provided { .. }),
            "{:?}",
            speech.state
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A model a sidecar serves is needed by no feature that sidecar does not
    /// serve — or a plan would price a model with nothing to run it.
    #[test]
    fn sidecars_and_their_slots_agree_on_who_needs_them() {
        for sc in SIDECARS {
            for slot_id in sc.serves {
                let slot = recommend::SLOTS.iter().find(|s| s.id == *slot_id).unwrap();
                if sc.needed_by.is_empty() {
                    continue;
                }
                for f in slot.needed_by {
                    assert!(
                        sc.needed_by.contains(f),
                        "slot {slot_id} is needed by {f:?}, but sidecar {} is not",
                        sc.id
                    );
                }
                assert!(
                    !slot.needed_by.is_empty(),
                    "{slot_id} needs every feature but {} does not",
                    sc.id
                );
            }
        }
    }

    /// begin, record, finish: the entry is written before the first byte,
    /// each path before it is written, and complete only at the end.
    #[test]
    fn an_install_s_record_is_written_ahead_of_its_bytes() {
        let root = scratch();
        let home = root.join("home/.mecha");
        Manifest::begin(&home, "layout").unwrap();
        let m = Manifest::read(&home).unwrap();
        assert_eq!(
            m.entries,
            vec![Entry {
                sidecar: "layout".into(),
                incomplete: true,
                wrote: vec![]
            }]
        );
        Manifest::record(&home, "layout", &home.join("sidecars/layout")).unwrap();
        Manifest::record(&home, "layout", &home.join("sidecars/layout")).unwrap();
        assert_eq!(
            Manifest::read(&home).unwrap().entries[0].wrote.len(),
            1,
            "recorded once"
        );
        Manifest::finish(&home, "layout").unwrap();
        assert!(!Manifest::read(&home).unwrap().entries[0].incomplete);
        assert!(Manifest::record(&home, "never-begun", &home).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A manifest that cannot be read is an error — never an empty record
    /// that would let mecha's own files read as a hand install.
    #[test]
    fn an_unreadable_manifest_stops_the_plan() {
        let root = scratch();
        let m = machinery(&root);
        std::fs::create_dir_all(root.join("home/.mecha/sidecars")).unwrap();
        std::fs::write(Manifest::path(&m.mecha_home), "{not json").unwrap();
        let err = plan(Feature::Graph, &m, &GB10, &root.join("hub"), false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not parse"), "{err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A check that could not run is unknown, never missing — so nothing is
    /// offered over what may well be there.
    #[test]
    fn a_check_that_cannot_run_is_unknown_not_missing() {
        let root = scratch();
        let mut m = machinery(&root);
        m.docker = Box::new(|_| Lookup::Unknown("docker daemon did not answer".into()));
        assert!(matches!(
            m.check(&Evidence::DockerImage("any/image")),
            Lookup::Unknown(_)
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A provided ComfyUI keeps its models in its own tree: the plan names
    /// them as held, never prices a download over a working install; with
    /// no ComfyUI, they are downloads.
    #[test]
    fn a_provided_comfyui_holds_its_own_models() {
        let root = scratch();
        let m = machinery(&root);
        let hub = root.join("hub");
        let none = plan(Feature::Image, &m, &GB10, &hub, false).unwrap();
        assert!(none
            .files
            .iter()
            .any(|f| f.slot == "image" && matches!(f.state, FileState::Download { .. })));
        std::fs::create_dir_all(root.join("home/ComfyUI")).unwrap();
        let held = plan(Feature::Image, &m, &GB10, &hub, false).unwrap();
        let image: Vec<_> = held.files.iter().filter(|f| f.slot == "image").collect();
        assert_eq!(image.len(), 1);
        assert_eq!(image[0].state, FileState::HeldBy { sidecar: "ComfyUI" });
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A ComfyUI that could not be checked: its models are neither held nor
    /// priced as a download over it.
    #[test]
    fn an_unknown_comfyui_s_models_are_not_priced() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if unsafe { libc::geteuid() } == 0 {
                return; // root reads anything; the negative would be vacuous.
            }
            let root = scratch();
            let mut m = machinery(&root);
            // ComfyUI's `Home("ComfyUI")` evidence lives under a home that
            // cannot be read; mecha's own tree stays readable.
            m.home = root.join("closed");
            std::fs::create_dir_all(&m.home).unwrap();
            std::fs::set_permissions(&m.home, std::fs::Permissions::from_mode(0o000)).unwrap();
            let p = plan(Feature::Image, &m, &GB10, &root.join("hub"), false);
            std::fs::set_permissions(&m.home, std::fs::Permissions::from_mode(0o755)).unwrap();
            let p = p.unwrap();
            let comfy = p.sidecars.iter().find(|s| s.id == "comfyui").unwrap();
            assert!(
                matches!(comfy.state, SidecarState::Unknown { .. }),
                "{:?}",
                comfy.state
            );
            let image: Vec<_> = p.files.iter().filter(|f| f.slot == "image").collect();
            assert_eq!(image.len(), 1);
            assert_eq!(
                image[0].state,
                FileState::KeeperUnknown { sidecar: "ComfyUI" }
            );
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    /// The chat model has no 16 GB row: the plan names that, and never says
    /// there is nothing to do.
    #[test]
    fn a_slot_with_no_row_is_named_and_blocks_nothing_to_do() {
        let root = scratch();
        let mut m = machinery(&root);
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::write(root.join("bin/llama-server"), "").unwrap();
        std::fs::create_dir_all(root.join("home/.config/systemd/user")).unwrap();
        std::fs::write(
            root.join("home/.config/systemd/user/llama-local.service"),
            "",
        )
        .unwrap();
        m.docker = Box::new(|_| Lookup::Absent);
        let small = Machine::Unified {
            total_mb: 16_384,
            gpu_unread: false,
        };
        let p = plan(Feature::Web, &m, &small, &root.join("hub"), false).unwrap();
        let chat = p.files.iter().find(|f| f.slot == "chat").unwrap();
        assert_eq!(chat.state, FileState::NoRow);
        assert!(!p.nothing_to_do);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A ComfyUI mecha installed keeps its models in its own tree too: held,
    /// never priced as a hub download.
    #[test]
    fn an_installed_comfyui_holds_its_own_models_too() {
        let root = scratch();
        let m = machinery(&root);
        let tree = root.join("home/.mecha/sidecars/comfyui");
        std::fs::create_dir_all(&tree).unwrap();
        let manifest = Manifest {
            entries: vec![Entry {
                sidecar: "comfyui".into(),
                incomplete: false,
                wrote: vec![tree],
            }],
        };
        std::fs::create_dir_all(root.join("home/.mecha/sidecars")).unwrap();
        std::fs::write(
            Manifest::path(&m.mecha_home),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
        let p = plan(Feature::Image, &m, &GB10, &root.join("hub"), false).unwrap();
        let image: Vec<_> = p.files.iter().filter(|f| f.slot == "image").collect();
        assert_eq!(image.len(), 1);
        assert_eq!(image[0].state, FileState::HeldBy { sidecar: "ComfyUI" });
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A `PATH` entry that cannot be read is unknown, never absent — `llama`
    /// is in every plan, and its installer must not be offered over a
    /// binary that may well be there.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_path_entry_is_unknown_not_missing() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            return; // root reads anything; the negative would be vacuous.
        }
        let root = scratch();
        let m = machinery(&root);
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::set_permissions(root.join("bin"), std::fs::Permissions::from_mode(0o000)).unwrap();
        let p = plan(Feature::Graph, &m, &GB10, &root.join("hub"), false);
        std::fs::set_permissions(root.join("bin"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let p = p.unwrap();
        let llama = p.sidecars.iter().find(|s| s.id == "llama").unwrap();
        assert!(
            matches!(llama.state, SidecarState::Unknown { .. }),
            "{:?}",
            llama.state
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `verify` hashes a hand-placed file the cheap check cannot vouch for:
    /// unverified without it, cached with it when it is the pin, a mismatch
    /// when it is not.
    #[test]
    fn verify_hashes_only_what_the_cheap_check_cannot_vouch_for() {
        use sha2::{Digest, Sha256};
        let root = scratch();
        let hub = root.join("hub");
        let body = b"the pinned bytes";
        let sum: String = Sha256::digest(body)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let f = crate::recommend::HubFile {
            path: "m.gguf",
            sha256: Box::leak(sum.into_boxed_str()),
            bytes: body.len() as u64,
        };
        let snap = crate::fetch::snapshot_path(&hub, "org/name", "r", &f);
        std::fs::create_dir_all(snap.parent().unwrap()).unwrap();
        std::fs::write(&snap, body).unwrap();
        assert_eq!(
            file_state(&hub, "org/name", "r", &f, false).unwrap(),
            FileState::Unverified
        );
        assert_eq!(
            file_state(&hub, "org/name", "r", &f, true).unwrap(),
            FileState::Cached
        );
        std::fs::write(&snap, &b"other bytes, sam"[..body.len()]).unwrap();
        assert_eq!(
            file_state(&hub, "org/name", "r", &f, true).unwrap(),
            FileState::Mismatch
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A pinned file already in the hub, matching, costs nothing; one that is
    /// there but different is a mismatch, never a download over it.
    #[test]
    fn a_cached_model_costs_nothing_and_a_wrong_one_is_not_overwritten() {
        let root = scratch();
        let m = machinery(&root);
        let hub = root.join("hub");
        let slot = recommend::SLOTS.iter().find(|s| s.id == "layout").unwrap();
        let Source::HuggingFace {
            repo,
            revision,
            files,
        } = slot.rows[0].sources[0]
        else {
            panic!()
        };
        let snap = crate::fetch::snapshot_path(&hub, repo, revision, &files[0]);
        std::fs::create_dir_all(snap.parent().unwrap()).unwrap();
        std::fs::write(&snap, b"not the model").unwrap();
        let p = plan(Feature::Documents, &m, &GB10, &hub, false).unwrap();
        let f = p.files.iter().find(|f| f.slot == "layout").unwrap();
        assert_eq!(f.state, FileState::Mismatch);
        assert!(p.files.iter().all(|f| f.slot != "layout"
            || f.state
                != (FileState::Download {
                    bytes: files[0].bytes
                })));
        let _ = std::fs::remove_dir_all(&root);
    }
}
