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
    /// A user service definition of this file name, in any of the
    /// `Machinery::unit_dirs`: a systemd user unit on Linux, a launchd agent's
    /// plist on macOS. A sidecar that installs on both names both spellings;
    /// neither system looks in the other's directories.
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
        evidence: &[
            Evidence::UserUnit("llama-local.service"),
            Evidence::UserUnit("ai.mecha-factory.llama-local.plist"),
        ],
        installer: "7c-2",
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
            Evidence::UserUnit("ai.mecha-factory.llama-embed.plist"),
        ],
        installer: "7c-1",
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
            Evidence::UserUnit("ai.mecha-factory.llama-ocr.plist"),
        ],
        installer: "7c-1",
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
        // Its own unit first, as `stt`'s and the worker's name theirs: with
        // the record lost, mecha's server must not read as missing (review
        // of #645).
        evidence: &[
            Evidence::UserUnit("mecha-comfyui.service"),
            Evidence::UserUnit("comfyui.service"),
            Evidence::UserUnit("ai.mecha-factory.mecha-comfyui.plist"),
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
        evidence: &[
            Evidence::UserUnit("mecha-parakeet.service"),
            Evidence::UserUnit("ai.mecha-factory.mecha-parakeet.plist"),
        ],
        installer: "7d-1",
        // Unpacked from its release tarball into its own tree, never the hub.
        models_in_hub: false,
    },
    Sidecar {
        id: "voice-worker",
        label: "the voice worker",
        needed_by: &[Feature::Voice],
        serves: &["turn"],
        evidence: &[
            Evidence::UserUnit("mecha-voice-worker.service"),
            Evidence::UserUnit("ai.mecha-factory.mecha-voice-worker.plist"),
        ],
        installer: "7d-2",
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
            Evidence::UserUnit("ai.mecha-factory.mecha-breeze-tts.plist"),
            Evidence::UserUnit("ai.mecha-factory.mecha-breeze-adapter.plist"),
            Evidence::DockerImage("mecha/chatterbox"),
        ],
        installer: "7f",
        // A speech server run by hand (Breeze, the Chatterbox container) keeps
        // its own models: held, never priced. Mecha's own install links them
        // from the hub, read there as ComfyUI's are (review of #649).
        models_in_hub: false,
    },
    // The knowledge graph's MCP server: one on the PATH is used when it is
    // the pinned build or the owner's own checkout build; otherwise `enable
    // graph` builds the pinned commit into mecha's tree (`graph_mcp.rs`, the
    // owner's rulings of 2026-10-13 and 2026-10-15, §15).
    Sidecar {
        id: "graph-mcp",
        label: "the knowledge graph's MCP server (mecha-graph-mcp)",
        needed_by: &[Feature::Graph],
        serves: &[],
        // `~/.cargo/bin` as well as PATH, as `onboarding::find_program`
        // looks: a `cargo install`ed server not on this shell's PATH is still
        // the owner's, never rebuilt beside (review of #683).
        evidence: &[
            Evidence::OnPath("mecha-graph-mcp"),
            Evidence::Home(".cargo/bin/mecha-graph-mcp"),
        ],
        installer: "13",
        // No models: nothing of the hub's is its to hold.
        models_in_hub: false,
    },
    // The mail and documents servers, one crate (`mail_mcp.rs`): built from
    // this mecha's own source, as a copy from another build speaks another
    // protocol (§15, the owner's ruling of 2026-10-15).
    Sidecar {
        id: "mail-mcp",
        label: "the mail and documents servers (mecha-mail, mecha-docs)",
        needed_by: &[Feature::Mail, Feature::Docs],
        serves: &[],
        evidence: &[
            Evidence::OnPath("mecha-mail"),
            Evidence::Home(".cargo/bin/mecha-mail"),
        ],
        installer: "15",
        models_in_hub: false,
    },
    // Code mode's worker (`interpreter::worker`): the `monty` binary from
    // PyPI's pinned wheel, identified by its sha256 at its fixed path under
    // mecha's tree. Never provided — a `monty` on PATH is whatever somebody
    // installed last, and only the pinned bytes may run under the sandbox —
    // so it has no evidence ([`ONLY_MECHAS`]).
    Sidecar {
        id: crate::interpreter::worker::SIDECAR,
        label: "code mode's interpreter (the pinned monty worker)",
        needed_by: &[Feature::Code],
        serves: &[],
        evidence: &[],
        installer: "code",
        models_in_hub: false,
    },
];

/// The sidecars only mecha's own install can supply, so nothing on a machine
/// shows them provided: a pinned binary checked by its hash at a fixed path.
pub const ONLY_MECHAS: &[&str] = &[crate::interpreter::worker::SIDECAR];

/// The package and the build a copy of one of mecha's own servers must be,
/// for a sidecar that is one (§15): the graph's server at its pin, the mail
/// servers at this mecha's own source. `None` for every other sidecar, and
/// for the mail servers when this build cannot tell its source.
pub fn own_server(id: &str) -> Option<(&'static str, crate::own_build::Source)> {
    match id {
        crate::graph_mcp::ID => Some((
            crate::graph_mcp::PROGRAM,
            crate::own_build::Source::Git(crate::graph_mcp::REV.to_string()),
        )),
        crate::mail_mcp::ID => Some((crate::mail_mcp::PACKAGE, crate::own_build::this()?)),
        _ => None,
    }
}

/// Whether `bin` is a copy of one of mecha's own servers, by `program`,
/// from another build than the one wanted (§15).
pub fn stale_copy(program: &str, bin: &Path) -> bool {
    stale_copy_with(program, bin, crate::onboarding::find_program)
}

/// [`stale_copy`] with the PATH lookup to resolve a bare name by. An
/// `[[mcp]]` entry's `command` is often just `mecha-graph-mcp` — the form the
/// docs ship — which has no directory to find cargo's record beside, so it is
/// resolved as the process that starts it would (review of #699).
pub fn stale_copy_with(program: &str, bin: &Path, find: impl Fn(&str) -> Option<PathBuf>) -> bool {
    let resolved;
    let bin = if bin.components().count() == 1 {
        match bin.to_str().and_then(&find) {
            Some(found) => {
                resolved = found;
                resolved.as_path()
            }
            None => return false,
        }
    } else {
        bin
    };
    let id = if program == crate::graph_mcp::PROGRAM {
        crate::graph_mcp::ID
    } else if crate::mail_mcp::PROGRAMS.contains(&program) {
        crate::mail_mcp::ID
    } else {
        return false;
    };
    own_server(id).is_some_and(|(package, want)| {
        matches!(
            crate::own_build::copy_of(bin, package, &want),
            crate::own_build::CopyState::Stale(_)
        )
    })
}

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
    /// The engine's builds on disk, each by tag and resolved commit (§10.3);
    /// empty for every other sidecar.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub builds: Vec<crate::engine::Build>,
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
    /// first byte (§10.2 item 4), so an interruption reads resumable. A
    /// finished install run again — a repair — stays finished, because
    /// marking it incomplete would hand extraction to a tree that may not
    /// exist (`document::layout_tree`). Not every repair step is atomic: the
    /// layout model's link and the engine's `current` link swap in one
    /// rename, but `uv pip install` changes the venv in place, `uv venv
    /// --clear` rebuilds it (only when its interpreter already dangles, so
    /// extraction was failing before the repair began), and the engine
    /// replaces a build of the same tag by removing it and renaming the
    /// checked one in — between the two, `current/llama-server` is absent.
    /// The record recovers from either window on its own: a recorded path
    /// that is gone reads the entry `Incomplete`, and `enable` resumes it.
    pub fn begin(mecha_home: &Path, id: &str) -> Result<()> {
        let mut m = Manifest::read(mecha_home)?;
        match m.entries.iter_mut().find(|e| e.sidecar == id) {
            Some(_) => {}
            None => m.entries.push(Entry {
                sidecar: id.to_string(),
                incomplete: true,
                wrote: vec![],
                builds: vec![],
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

    /// Forget a path the install no longer owns, after removing it — so a
    /// path taken away on purpose (an adopt's drop-in, rolled back) never
    /// reads as an install that did not finish.
    pub fn unrecord(mecha_home: &Path, id: &str, path: &Path) -> Result<()> {
        let mut m = Manifest::read(mecha_home)?;
        let Some(e) = m.entries.iter_mut().find(|e| e.sidecar == id) else {
            // Nothing recorded: nothing to forget, and no manifest to make.
            return Ok(());
        };
        e.wrote.retain(|w| w != path);
        m.write(mecha_home)
    }

    /// Drop an entry that no longer records anything — a sidecar removed whole
    /// — so the plan reads the machine again rather than an empty, finished
    /// record (which it would call unknown, and never offer over).
    pub fn forget_if_empty(mecha_home: &Path, id: &str) -> Result<()> {
        let mut m = Manifest::read(mecha_home)?;
        let before = m.entries.len();
        m.entries
            .retain(|e| !(e.sidecar == id && e.wrote.is_empty() && e.builds.is_empty()));
        if m.entries.len() == before {
            return Ok(());
        }
        m.write(mecha_home)
    }

    /// Record a build the install unpacked and checked, replacing an earlier
    /// record of the same tag.
    pub fn record_build(mecha_home: &Path, id: &str, build: crate::engine::Build) -> Result<()> {
        let mut m = Manifest::read(mecha_home)?;
        let e = m
            .entries
            .iter_mut()
            .find(|e| e.sidecar == id)
            .with_context(|| format!("recording a build of {id} before its install began"))?;
        e.builds.retain(|b| b.tag != build.tag);
        e.builds.push(build);
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
    /// Where user services are defined: systemd's user unit directories on
    /// Linux, launchd's agents directories on macOS. The first is where mecha
    /// writes.
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
        let unit_dirs = if cfg!(target_os = "macos") {
            // launchd's agents directories: the user's, where mecha writes,
            // then the machine's (`launchd.rs`).
            vec![
                crate::launchd::agents_dir(&home),
                PathBuf::from("/Library/LaunchAgents"),
            ]
        } else {
            // systemd's user search path, as `Facts::read` finds the config
            // directory (`dirs`, so `XDG_CONFIG_HOME` and `XDG_DATA_HOME`
            // count): a package's unit in /usr/share is as much a hand
            // install as one in ~/.config.
            let mut dirs: Vec<PathBuf> = [dirs::config_dir(), dirs::data_dir()]
                .into_iter()
                .flatten()
                .map(|d| d.join("systemd/user"))
                .collect();
            dirs.extend(
                [
                    "/etc/systemd/user",
                    "/usr/local/lib/systemd/user",
                    "/usr/local/share/systemd/user",
                    "/usr/lib/systemd/user",
                    "/usr/share/systemd/user",
                ]
                .map(PathBuf::from),
            );
            dirs
        };
        let path = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        Ok(Machinery {
            home,
            mecha_home,
            unit_dirs,
            path,
            // The one Docker evidence is the Chatterbox speech container.
            // Docker Desktop is usually not running on a Mac, which read
            // every Mac as "unknown" and withheld the speech server for good
            // (found on the owner's Mac, 2026-10-13): there, a daemon that
            // does not answer reads as no container.
            docker: if cfg!(target_os = "macos") {
                // A daemon that does not answer is not a container serving
                // speech; an image that is there still is one (review of
                // #679: dropping `Found` too would offer Breeze over it).
                Box::new(|repo| match docker_image(repo) {
                    Lookup::Unknown(_) => Lookup::Absent,
                    other => other,
                })
            } else {
                Box::new(docker_image)
            },
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
    /// What a sidecar would fetch whose install is not offered on this
    /// machine (`install::price`): not priced, and why.
    NotOffered { why: String },
    /// The chat slot served by mecha's router from a GGUF the owner brought
    /// (`mecha setup chat`): the pinned row is not this machine's model, so
    /// it is neither priced nor offered.
    Brought,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannedSidecar {
    pub id: &'static str,
    pub label: &'static str,
    pub state: SidecarState,
    /// A copy of one of mecha's own servers that mecha did *not* install —
    /// one on the PATH, from another build — and how it differs (§15). It is
    /// passed over: the row reads missing, `enable` installs mecha's beside
    /// it, and this is said so the owner knows why. Never set on a row mecha
    /// installed; compare `stale`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passed_over: Option<String>,
    /// mecha's *own* install, with service files an older build wrote (or
    /// forced by `enable --reinstall`): `enable` and `setup` offer to refresh
    /// it — its install run again, fetching nothing current (§14). Only on an
    /// installed row; compare `passed_over`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stale: bool,
    /// What installing it downloads, where that is known before the install
    /// (`install::price`): the engine's pinned archives for this machine.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    /// Why `enable` will not offer its install on this machine, though it
    /// is missing (`install::price`): said on its row as on its files', so
    /// one plan does not contradict itself (review of #649).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_offered: Option<String>,
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

/// The model slots a feature's plan prices: the ones it or its parts need,
/// and the chat model's, which every feature needs. A part resolves to its
/// parent.
pub fn needed_slots(named: Feature) -> impl Iterator<Item = &'static recommend::Slot> {
    let named = named.switch_owner();
    recommend::SLOTS
        .iter()
        .filter(move |s| s.needed_by.is_empty() || s.needed_by.iter().any(|f| within(named, *f)))
}

/// The sidecars a feature's plan reads: the ones it or its parts need, and
/// the shared ones every feature needs. A part resolves to its parent.
pub fn needed(named: Feature) -> impl Iterator<Item = &'static Sidecar> {
    let named = named.switch_owner();
    SIDECARS
        .iter()
        .filter(move |s| s.needed_by.is_empty() || s.needed_by.iter().any(|f| within(named, *f)))
}

/// Whether any service definition mecha wrote for `id` carries a revision
/// older than `rev`, or none — `Err` naming one that could not be read.
fn stale_definitions(entry: Option<&Entry>, id: &str, rev: u32) -> Result<bool, String> {
    let Some(entry) = entry else {
        return Ok(false);
    };
    for path in entry
        .wrote
        .iter()
        .filter(|p| crate::user_service::is_definition(p))
    {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                if crate::user_service::rev_of(&text, id).is_none_or(|r| r < rev) {
                    return Ok(true);
                }
            }
            // Gone is the record's business (`Incomplete`), not this one's.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    Ok(false)
}

/// The sidecars mecha installed, finished, whose services an older build
/// wrote — what `mecha setup` offers to refresh (`user_service::REVS`). A
/// definition that cannot be read is an error naming it: unknown, never
/// current.
pub fn stale_services(mecha_home: &Path) -> Result<Vec<&'static Sidecar>> {
    let manifest = Manifest::read(mecha_home)?;
    let mut stale = Vec::new();
    for s in SIDECARS {
        let Some(rev) = crate::user_service::rev(s.id) else {
            continue;
        };
        let Some(entry) = manifest.entry(s.id).filter(|e| !e.incomplete) else {
            continue;
        };
        if stale_definitions(Some(entry), s.id, rev).map_err(anyhow::Error::msg)? {
            stale.push(s);
        }
    }
    Ok(stale)
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

    let mut sidecars = Vec::new();
    for s in needed(named) {
        // What the machine shows, read only when mecha's record does not
        // settle it.
        // A copy of one of mecha's own servers from another build: not
        // provided, but in the way — said on the row (§15).
        let passed_over = std::cell::RefCell::new(None::<String>);
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
                            if let (Some((package, want)), Some(p)) = (own_server(s.id), &path) {
                                if let crate::own_build::CopyState::Stale(why) =
                                    crate::own_build::copy_of(p, package, &want)
                                {
                                    *passed_over.borrow_mut() =
                                        Some(format!("{} is {why}", p.display()));
                                    continue;
                                }
                            }
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
        // The graph's server records only its tree, so "the tree is there"
        // is not "the pinned build is": a build of an older commit, or a tree
        // whose binary is gone, reads unfinished — offered, and rebuilt by
        // `enable` — or a bump of `graph_mcp::REV` would never reach a
        // machine that built the last one (review of #683).
        // Code mode's worker is the pinned bytes or nothing: installed when
        // the file at its path hashes to its pin, whatever the record says,
        // and unfinished when the record says installed and it does not.
        let state = if s.id == crate::interpreter::worker::SIDECAR {
            let worker = crate::interpreter::worker::installed_path(&m.mecha_home);
            match crate::interpreter::worker::verify(&worker) {
                Ok(_) => SidecarState::Installed,
                Err(_) if state == SidecarState::Installed => SidecarState::Incomplete,
                Err(_) => state,
            }
        } else {
            state
        };
        let state = match state {
            SidecarState::Installed
                if s.id == crate::graph_mcp::ID && !crate::graph_mcp::installed(&m.mecha_home) =>
            {
                SidecarState::Incomplete
            }
            // The mail servers likewise, against this mecha's source: every
            // upgrade of mecha moves it, so a tree built for the last one
            // reads unfinished and `enable` builds the matching one (review
            // of #699).
            SidecarState::Installed
                if s.id == crate::mail_mcp::ID && !crate::mail_mcp::installed(&m.mecha_home) =>
            {
                SidecarState::Incomplete
            }
            other => other,
        };
        // Installed, and its services written by an older build: stale, so
        // `enable` offers to refresh them (`user_service::REVS`). A
        // definition that cannot be read is unknown, never current.
        let (state, stale) = match (state, crate::user_service::rev(s.id)) {
            (SidecarState::Installed, Some(rev)) => {
                match stale_definitions(manifest.entry(s.id), s.id, rev) {
                    Ok(stale) => (SidecarState::Installed, stale),
                    Err(why) => (SidecarState::Unknown { why }, false),
                }
            }
            (state, _) => (state, false),
        };
        sidecars.push(PlannedSidecar {
            id: s.id,
            label: s.label,
            state,
            bytes: None,
            not_offered: None,
            passed_over: passed_over.into_inner(),
            stale,
        });
    }

    // What mecha's own router serves, when it is here: a brought model makes
    // the chat row's pinned files nobody's to fetch.
    let router_here = sidecars.iter().any(|p| {
        p.id == crate::router_unit::ID
            && matches!(p.state, SidecarState::Installed | SidecarState::Incomplete)
    });
    let mut files = Vec::new();
    for slot in needed_slots(named) {
        if slot.id == "chat" && router_here {
            let state = match crate::router_unit::installed_choice(&m.mecha_home) {
                Ok(Some(crate::router_unit::Choice::Own { .. })) => Some(FileState::Brought),
                Ok(_) => None,
                Err(_) => Some(FileState::KeeperUnknown {
                    sidecar: "the chat router",
                }),
            };
            if let Some(state) = state {
                // An unreadable presets file still has the tier's row named:
                // left empty, the line would read "no recommended model"
                // beside "could not be checked" (found on review of #618).
                let model = match state {
                    FileState::Brought => "",
                    _ => recommend::row_for(slot, machine).map_or("", |(row, _)| row.model),
                };
                files.push(PlannedFile {
                    slot: slot.id,
                    model,
                    repo: None,
                    path: "",
                    state,
                });
                continue;
            }
        }
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
            // Mecha's own ComfyUI links the slot's files from the hub
            // (`comfyui_unit`), so they are read there like any model: a
            // cleared cache reads as a download, and `offered` offers the
            // install again — a dangling link alone would read as present
            // (`exists` is an lstat; review of #645).
            // An interrupted install too: it resumes by fetching what the
            // hub lacks, so that is what it is priced at (review of #645).
            Some((sc, SidecarState::Installed | SidecarState::Incomplete))
                if sc.id == crate::comfyui_unit::ID || sc.id == crate::speech_unit::ID =>
            {
                None
            }
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

    // The total is what `features enable` would fetch, and the chat model —
    // its files — is `mecha setup chat`'s, never `enable`'s (ruling F13):
    // counted here, a plan for `web` read "To download: 22 GiB" about a
    // download `enable web` would never make (found on review of #627). Its
    // rows still show, and a machine without it never reads as having all.
    let download_bytes = files
        .iter()
        .filter(|f| f.slot != "chat")
        .map(|f| match f.state {
            FileState::Download { bytes } => bytes,
            _ => 0,
        })
        .sum();
    // A stale install is something to do: its refresh — except the
    // router's, which is `mecha setup chat`'s and never a feature's plan's,
    // or a machine chatting elsewhere would never read "nothing to do"
    // (review of #691, pass 5).
    let nothing_to_do = sidecars.iter().all(|s| {
        !(s.stale && s.id != crate::router_unit::ID)
            && matches!(
                s.state,
                SidecarState::Provided { .. } | SidecarState::Installed
            )
    }) && files.iter().all(|f| {
        matches!(
            f.state,
            FileState::Cached
                | FileState::WithSidecar
                | FileState::HeldBy { .. }
                | FileState::Brought
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

    /// A finished install's services read stale when a definition carries
    /// no stamp or an older number, and current when every one carries this
    /// build's; a program file beside them is not a definition, and an
    /// unfinished install is `enable`'s to resume, not to refresh.
    #[test]
    fn a_service_from_an_older_build_reads_stale() {
        let home = std::env::temp_dir().join(format!("mecha-stale-{}", uuid::Uuid::new_v4()));
        let unit = home.join("units/mecha-parakeet.service");
        std::fs::create_dir_all(unit.parent().unwrap()).unwrap();
        Manifest::begin(&home, "stt").unwrap();
        let write = |text: &str| {
            std::fs::write(&unit, text).unwrap();
            Manifest::record(&home, "stt", &unit).unwrap();
        };
        let ids = |home: &Path| -> Vec<&str> {
            stale_services(home).unwrap().iter().map(|s| s.id).collect()
        };
        write("[Unit]\n");
        assert!(ids(&home).is_empty(), "an unfinished install is not stale");
        Manifest::finish(&home, "stt").unwrap();
        assert_eq!(ids(&home), ["stt"], "no stamp: written before the stamp");
        write(&crate::user_service::stamp("stt", "[Unit]\n"));
        assert!(ids(&home).is_empty(), "this build's stamp is current");
        write("# mecha-service-rev: stt 0\n[Unit]\n");
        assert_eq!(ids(&home), ["stt"], "an older number");
        write(&crate::user_service::stamp("stt", "[Unit]\n"));
        let script = home.join("stt/parakeet_server.py");
        std::fs::create_dir_all(script.parent().unwrap()).unwrap();
        std::fs::write(&script, "print()\n").unwrap();
        Manifest::record(&home, "stt", &script).unwrap();
        assert!(ids(&home).is_empty(), "a program file carries no stamp");
        // A definition that cannot be read is unknown, never current: a
        // directory where the unit should be cannot be read as text.
        std::fs::remove_file(&unit).unwrap();
        std::fs::create_dir(&unit).unwrap();
        assert!(stale_services(&home).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    /// An entry emptied by a removal is dropped; one still naming a path, or
    /// another sidecar's, is kept.
    #[test]
    fn an_emptied_entry_is_forgotten_and_nothing_else() {
        let home = std::env::temp_dir().join(format!("mecha-forget-{}", uuid::Uuid::new_v4()));
        Manifest::begin(&home, "ocr-server").unwrap();
        Manifest::begin(&home, "embed-server").unwrap();
        Manifest::record(&home, "embed-server", &home.join("x")).unwrap();
        Manifest::forget_if_empty(&home, "embed-server").unwrap();
        Manifest::forget_if_empty(&home, "ocr-server").unwrap();
        let ids: Vec<String> = Manifest::read(&home)
            .unwrap()
            .entries
            .into_iter()
            .map(|e| e.sidecar)
            .collect();
        assert_eq!(ids, vec!["embed-server".to_string()]);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// A path taken away on purpose and unrecorded leaves the install
    /// whole: recorded-but-gone reads `Incomplete`, forgotten reads as before.
    #[test]
    fn an_unrecorded_path_no_longer_reads_incomplete() {
        let home = std::env::temp_dir().join(format!("mecha-unrecord-{}", uuid::Uuid::new_v4()));
        let kept = home.join("sidecars/llama");
        let drop_in = home.join("units/llama-local.service.d/mecha-engine.conf");
        std::fs::create_dir_all(&kept).unwrap();
        std::fs::create_dir_all(drop_in.parent().unwrap()).unwrap();
        std::fs::write(&drop_in, "").unwrap();
        Manifest::begin(&home, "llama").unwrap();
        Manifest::record(&home, "llama", &kept).unwrap();
        Manifest::record(&home, "llama", &drop_in).unwrap();
        Manifest::finish(&home, "llama").unwrap();
        let state = |home: &Path| {
            let m = Manifest::read(home).unwrap();
            let e = m.entry("llama").unwrap();
            let present = e.wrote.iter().filter(|p| p.exists()).count();
            (present, e.wrote.len())
        };
        assert_eq!(state(&home), (2, 2));
        std::fs::remove_file(&drop_in).unwrap();
        assert_eq!(state(&home), (1, 2), "gone but recorded: incomplete");
        Manifest::unrecord(&home, "llama", &drop_in).unwrap();
        assert_eq!(state(&home), (1, 1), "forgotten: whole again");
        // Unrecording under an id with no entry makes no manifest.
        let empty = home.join("elsewhere");
        Manifest::unrecord(&empty, "llama", &drop_in).unwrap();
        assert!(!Manifest::path(&empty).exists());
        let _ = std::fs::remove_dir_all(&home);
    }
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
            assert_eq!(
                s.evidence.is_empty(),
                ONLY_MECHAS.contains(&s.id),
                "{}: nothing could ever show it provided, and it is not one only mecha supplies",
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
                ["7a-3", "7b", "7c-1", "7c-2", "7d-1", "7d-2", "7e", "7f", "13", "15", "code"]
                    .contains(&s.installer),
                "{}: installer step {}",
                s.id,
                s.installer
            );
        }
    }

    /// Code mode's worker is the pinned bytes at mecha's path or nothing: a
    /// `monty` on `PATH` never provides it, and a file at its path that is
    /// not the pin is not installed, whatever the record says.
    #[test]
    fn code_modes_worker_is_its_pin_at_its_path_or_missing() {
        use crate::interpreter::worker;
        let root = scratch();
        let m = machinery(&root);
        let hub = root.join("hub");
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::write(root.join("bin/monty"), "somebody's monty").unwrap();
        let state = |m: &Machinery| {
            plan(Feature::Code, m, &GB10, &hub, false)
                .unwrap()
                .sidecars
                .into_iter()
                .find(|s| s.id == worker::SIDECAR)
                .expect("code's plan names its worker")
                .state
        };
        assert_eq!(state(&m), SidecarState::Missing { step: "code" });
        let at = worker::installed_path(&m.mecha_home);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(&at, "not the pinned worker").unwrap();
        Manifest::begin(&m.mecha_home, worker::SIDECAR).unwrap();
        Manifest::record(&m.mecha_home, worker::SIDECAR, at.parent().unwrap()).unwrap();
        Manifest::finish(&m.mecha_home, worker::SIDECAR).unwrap();
        assert_eq!(
            state(&m),
            SidecarState::Incomplete,
            "a recorded install whose file is not the pin is unfinished, and offered again"
        );
        let _ = std::fs::remove_dir_all(&root);
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
        assert_eq!(ocr.state, SidecarState::Missing { step: "7c-1" });
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
                    builds: vec![],
                },
                Entry {
                    sidecar: "ocr-server".into(),
                    incomplete: false,
                    wrote: vec![tree.join("ocr")],
                    builds: vec![],
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
                    builds: vec![],
                },
                Entry {
                    sidecar: "ocr-server".into(),
                    incomplete: false,
                    wrote: vec![tree.join("ocr/a"), tree.join("ocr/b")],
                    builds: vec![],
                },
                Entry {
                    sidecar: "embed-server".into(),
                    incomplete: false,
                    wrote: vec![],
                    builds: vec![],
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

    /// A router `mecha setup chat` installed with the owner's own GGUF serves
    /// that model: the chat row's pinned files are neither priced nor a
    /// reason to offer the router again — which would rewrite its presets
    /// with the recommended row (found on review of #568). The pinned row's
    /// router, its files gone, is neither offered nor in the plan's total:
    /// re-running `mecha setup chat` fetches it (ruling F13).
    #[test]
    fn a_router_serving_a_brought_model_is_not_offered_the_row() {
        use crate::router_unit::{presets_path, presets_text, Preset, PINNED_ALIAS};
        let root = scratch();
        let m = machinery(&root);
        let presets = presets_path(&m.mecha_home);
        std::fs::create_dir_all(presets.parent().unwrap()).unwrap();
        let manifest = Manifest {
            entries: vec![Entry {
                sidecar: "router".into(),
                incomplete: false,
                wrote: vec![presets.clone()],
                builds: vec![],
            }],
        };
        std::fs::write(
            Manifest::path(&m.mecha_home),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
        let with = |alias: &str| {
            std::fs::write(
                &presets,
                presets_text(&Preset {
                    alias: alias.into(),
                    model: "/m/mine.gguf".into(),
                    mmproj: None,
                    ctx: 8192,
                    slots: 1,
                    cache_ram_mb: None,
                    pinned: alias == PINNED_ALIAS,
                }),
            )
            .unwrap();
            plan(Feature::Messages, &m, &GB10, &root.join("hub"), false).unwrap()
        };

        let own = with("mine");
        let router = own.sidecars.iter().find(|s| s.id == "router").unwrap();
        assert_eq!(router.state, SidecarState::Installed);
        let chat: Vec<_> = own.files.iter().filter(|f| f.slot == "chat").collect();
        assert_eq!(chat.len(), 1, "{chat:?}");
        assert_eq!(chat[0].state, FileState::Brought);
        assert!(!crate::install::offered(&own, true)
            .iter()
            .any(|s| s.id == "router"));

        let pinned = with(PINNED_ALIAS);
        assert!(pinned
            .files
            .iter()
            .any(|f| f.slot == "chat" && matches!(f.state, FileState::Download { .. })));
        assert_eq!(
            pinned.download_bytes, 0,
            "the chat model is not enable's to fetch (review of #627)"
        );
        // Its model gone, the pinned row's router is still not `enable`'s
        // to fetch again: `mecha setup chat` is (ruling F13).
        assert!(!crate::install::offered(&pinned, true)
            .iter()
            .any(|s| s.id == "router"));

        std::fs::write(&presets, "version = 1\n").unwrap();
        let unread = plan(Feature::Messages, &m, &GB10, &root.join("hub"), false).unwrap();
        assert!(unread
            .files
            .iter()
            .any(|f| f.slot == "chat" && matches!(f.state, FileState::KeeperUnknown { .. })));
        // …and still names the tier's row: an unknown is never "none"
        // (found on review of #618).
        let row = recommend::row_for(
            recommend::SLOTS.iter().find(|s| s.id == "chat").unwrap(),
            &GB10,
        )
        .unwrap()
        .0;
        assert!(unread
            .files
            .iter()
            .any(|f| f.slot == "chat" && f.model == row.model && !f.model.is_empty()));
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
                builds: vec![],
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
        // Its models are its own: held, never priced — nothing will fetch
        // the slot's GGUF on a machine already speaking (review of #649).
        let tts: Vec<_> = p.files.iter().filter(|f| f.slot == "tts").collect();
        assert!(!tts.is_empty());
        assert!(
            tts.iter()
                .all(|f| matches!(f.state, FileState::HeldBy { .. })),
            "{tts:#?}"
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
                wrote: vec![],
                builds: vec![],
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

    /// The graph's MCP server: missing with nothing here; the owner's on the
    /// PATH is provided and left alone; mecha's own build, recorded, is
    /// installed — never read as the owner's for being on no PATH (§13).
    #[test]
    fn the_graph_server_is_missing_provided_or_mecha_s_own() {
        let root = scratch();
        let m = machinery(&root);
        let state = |m: &Machinery| {
            plan(Feature::Graph, m, &GB10, &root.join("hub"), false)
                .unwrap()
                .sidecars
                .into_iter()
                .find(|s| s.id == crate::graph_mcp::ID)
                .expect("graph's plan names its MCP server")
                .state
        };
        assert!(matches!(state(&m), SidecarState::Missing { .. }));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::write(root.join("bin/mecha-graph-mcp"), b"#!/bin/sh\n").unwrap();
        assert!(matches!(state(&m), SidecarState::Provided { .. }));
        // A copy cargo recorded from another build than the pin is passed
        // over: missing, and said (§15). Its own checkout build stays the
        // owner's.
        let row = |m: &Machinery| {
            plan(Feature::Graph, m, &GB10, &root.join("hub"), false)
                .unwrap()
                .sidecars
                .into_iter()
                .find(|s| s.id == crate::graph_mcp::ID)
                .unwrap()
        };
        let cargo_record = |source: &str| {
            std::fs::write(
                root.join(".crates2.json"),
                format!(
                    r#"{{"installs":{{"mecha-graph-mcp 0.1.5 ({source})":{{"bins":["mecha-graph-mcp"]}}}}}}"#
                ),
            )
            .unwrap()
        };
        cargo_record("registry+https://github.com/rust-lang/crates.io-index");
        let r = row(&m);
        assert!(matches!(r.state, SidecarState::Missing { .. }), "{r:?}");
        assert!(
            r.passed_over
                .as_deref()
                .is_some_and(|c| c.contains("the published 0.1.5")),
            "{r:?}"
        );
        cargo_record("path+file:///home/u/mecha-graph/mecha-graph-mcp");
        assert!(matches!(state(&m), SidecarState::Provided { .. }));
        std::fs::remove_file(root.join(".crates2.json")).unwrap();
        std::fs::remove_file(root.join("bin/mecha-graph-mcp")).unwrap();
        // A finished record over a tree with no pinned build in it is not
        // installed: it is offered again, so `enable` rebuilds it.
        use crate::graph_mcp::{binary, root as tree_of, PROGRAM, REPO, REV};
        let tree = tree_of(&m.mecha_home);
        std::fs::create_dir_all(tree.join("bin")).unwrap();
        Manifest::begin(&m.mecha_home, crate::graph_mcp::ID).unwrap();
        Manifest::record(&m.mecha_home, crate::graph_mcp::ID, &tree).unwrap();
        Manifest::finish(&m.mecha_home, crate::graph_mcp::ID).unwrap();
        assert_eq!(state(&m), SidecarState::Incomplete, "no binary");
        std::fs::write(binary(&m.mecha_home), b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                binary(&m.mecha_home),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        let record = |rev: &str| {
            std::fs::write(
                tree.join(".crates2.json"),
                format!(
                    r#"{{"installs":{{"{PROGRAM} 0.1.5 (git+{REPO}?rev={rev}#{rev})":{{}}}}}}"#
                ),
            )
            .unwrap()
        };
        record("0000000000000000000000000000000000000000");
        assert_eq!(
            state(&m),
            SidecarState::Incomplete,
            "an older commit's build is not the pin"
        );
        record(REV);
        assert_eq!(state(&m), SidecarState::Installed);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// An entry's bare `command` (`mecha-graph-mcp`, the form the docs ship)
    /// is judged as the binary the PATH resolves it to: a stale copy there is
    /// stale under its bare name too, and a name nothing resolves is not
    /// judged at all (review of #699).
    #[test]
    fn a_bare_command_is_judged_as_what_the_path_resolves() {
        let root = scratch();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let copy = root.join("bin/mecha-graph-mcp");
        std::fs::write(&copy, "").unwrap();
        std::fs::write(
            root.join(".crates2.json"),
            r#"{"installs":{"mecha-graph-mcp 0.1.5 (registry+https://github.com/rust-lang/crates.io-index)":{"bins":["mecha-graph-mcp"]}}}"#,
        )
        .unwrap();
        let program = crate::graph_mcp::PROGRAM;
        let finds = |name: &str| (name == program).then(|| copy.clone());
        assert!(stale_copy_with(program, Path::new(program), finds));
        assert!(
            stale_copy_with(program, &copy, finds),
            "the full path agrees"
        );
        assert!(!stale_copy_with(program, Path::new(program), |_| None));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// mecha's mail servers, recorded as finished, read unfinished while the
    /// tree is not this mecha's build — so an upgrade of mecha is offered the
    /// matching servers rather than reading installed over the last ones
    /// (review of #699).
    #[test]
    fn a_mail_tree_from_another_build_reads_unfinished() {
        let root = scratch();
        let m = machinery(&root);
        let tree = crate::mail_mcp::root(&m.mecha_home);
        std::fs::create_dir_all(tree.join("bin")).unwrap();
        Manifest::begin(&m.mecha_home, crate::mail_mcp::ID).unwrap();
        Manifest::record(&m.mecha_home, crate::mail_mcp::ID, &tree).unwrap();
        Manifest::finish(&m.mecha_home, crate::mail_mcp::ID).unwrap();
        let state = plan(Feature::Mail, &m, &GB10, &root.join("hub"), false)
            .unwrap()
            .sidecars
            .into_iter()
            .find(|s| s.id == crate::mail_mcp::ID)
            .expect("mail's plan names its servers")
            .state;
        assert_eq!(state, SidecarState::Incomplete);
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

    /// A ComfyUI mecha installed links its models from the hub, so they are
    /// read there: an empty hub reads as a download, never as held — which is
    /// what makes a cleared cache visible and the install offered again. An
    /// interrupted install the same: it resumes by fetching what the hub
    /// lacks, so that is its price (review of #645).
    #[test]
    fn an_installed_comfyui_reads_its_models_from_the_hub() {
        for incomplete in [false, true] {
            installed_comfyui_reads_the_hub(incomplete);
        }
    }

    fn installed_comfyui_reads_the_hub(incomplete: bool) {
        let root = scratch();
        let m = machinery(&root);
        let tree = root.join("home/.mecha/sidecars/comfyui");
        std::fs::create_dir_all(&tree).unwrap();
        let manifest = Manifest {
            entries: vec![Entry {
                sidecar: "comfyui".into(),
                incomplete,
                wrote: vec![tree],
                builds: vec![],
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
        // The model's three files and SAM 3.1's checkpoint.
        assert_eq!(image.len(), 4, "incomplete={incomplete}: {image:#?}");
        assert!(
            image
                .iter()
                .all(|f| matches!(f.state, FileState::Download { .. })),
            "{image:#?}"
        );
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

    /// The one Docker evidence is Chatterbox's: on a Mac an unanswering
    /// daemon reads as no container, a rule written for that image alone. A
    /// second `DockerImage` would inherit it quietly (review of #679).
    #[test]
    fn chatterbox_is_the_only_docker_evidence() {
        let docker: Vec<&str> = SIDECARS
            .iter()
            .flat_map(|s| s.evidence)
            .filter_map(|e| match e {
                Evidence::DockerImage(r) => Some(*r),
                _ => None,
            })
            .collect();
        assert_eq!(docker, ["mecha/chatterbox"]);
    }
}
