//! The llama-server router and its chat model, installed from nothing
//! (`docs/FEATURES-DESIGN.md` §10.6, step 7c-2; ruling F11: the door is
//! `mecha setup chat`, and `mecha features enable` takes the same path with
//! the recommended row where the chat model is served here).
//!
//! **One unit, one launcher, one presets file.** The router is a resident
//! service (not socket-started like the embeddings and OCR servers): the chat
//! model is loaded at start and kept. Its launcher runs
//! `${LLAMA_SERVER} --models-preset <file> --models-max 1`, and the presets
//! file is mecha's, written from the chat row it installed — so the flags the
//! server runs with are the ones the plan priced (`recommend::ChatGeometry`),
//! never a working tree's script (§10.2 item 2).
//!
//! **The pinned row's preset is the GB10's**, as `scripts/start-router.sh`
//! writes it: the tier's context and slots, the host prompt cache, MTP
//! drafting, the image token floor, and Qwen's model-card sampling. A model
//! the owner brings gets a plain preset — its file, its projector if given,
//! a 32k window in one slot — because nothing about it is known; the owner
//! edits the file from there.
//!
//! **What the provider is told is read back from the server** (§10.4): this
//! module installs and starts; `mecha setup chat` then hands the router's own
//! `/props` — for the model installed, never the router's placeholder — to
//! setup's existing write-and-confirm flow.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::install::Say;
use crate::llama_units::{bin_dir, write_owned, Engine, PATH_CONF, WAIT_HEALTHY};
use crate::sidecar::{Machinery, Manifest};

const ROUTER_SERVICE: &str = include_str!("../units/llama-local.service");
const ROUTER_LAUNCHER: &str = include_str!("../units/mecha-router");

/// The sidecar id `sidecar::SIDECARS` knows the router by.
pub const ID: &str = "router";

/// The unit and port the router runs under: the shipped ones in use, others
/// in a test beside the live router.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Naming {
    pub stem: String,
    pub port: u16,
}

impl Naming {
    pub fn shipped() -> Naming {
        Naming {
            stem: "llama-local".into(),
            port: 8080,
        }
    }

    pub fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

/// What the owner chose to serve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// The chat row recommended for this machine's tier.
    Recommended,
    /// A GGUF the owner brings, with its projector if it has one.
    Own {
        model: PathBuf,
        mmproj: Option<PathBuf>,
    },
}

/// The preset name the pinned chat model is served under — the GB10's, so
/// `mecha model list` reads the same on either kind of install.
pub const PINNED_ALIAS: &str = "qwen3.6-35b-a3b";

/// One preset section, as the router reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub alias: String,
    pub model: PathBuf,
    pub mmproj: Option<PathBuf>,
    pub ctx: u32,
    pub slots: u32,
    pub cache_ram_mb: Option<u32>,
    /// The pinned row's own lines: MTP, the image floor, sampling.
    pub pinned: bool,
}

/// The presets file: `[*]` shared lines, then the one model, loaded at start.
pub fn presets_text(p: &Preset) -> String {
    let mut out = String::from(
        "# Written by `mecha setup chat` (or `mecha features enable`) from the chat row it\n\
         # installed. The router reads it at start: `mecha setup chat` again rewrites it.\n\
         version = 1\n\n[*]\nn-gpu-layers = 999\njinja = true\n\n",
    );
    out.push_str(&format!("[{}]\nmodel = {}\n", p.alias, p.model.display()));
    if let Some(mm) = &p.mmproj {
        out.push_str(&format!("mmproj = {}\n", mm.display()));
    }
    out.push_str(&format!("ctx-size = {}\nparallel = {}\n", p.ctx, p.slots));
    if let Some(c) = p.cache_ram_mb {
        out.push_str(&format!("cache-ram = {c}\n"));
    }
    if p.pinned {
        // As `scripts/start-router.sh` serves it on the GB10: MTP drafting,
        // the vision token floor, and the model card's thinking-mode sampling.
        out.push_str(
            "image-min-tokens = 1024\nspec-type = draft-mtp\ntemp = 0.6\ntop-p = 0.95\n\
             top-k = 20\nmin-p = 0.0\npresence-penalty = 0.0\nrepeat-penalty = 1.0\n\
             reasoning-budget = 4096\nreasoning-preserve = true\n",
        );
    }
    out.push_str("load-on-startup = true\n");
    out
}

/// A preset name for a model the owner brings: its file stem, lowercased,
/// anything but letters, digits, `.` and `-` made `-`.
pub fn alias_for(model: &Path) -> String {
    let stem = model
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "local".into());
    let a: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let a = a.trim_matches('-').to_string();
    if a.is_empty() {
        "local".into()
    } else {
        a
    }
}

/// The presets file mecha writes.
pub fn presets_path(mecha_home: &Path) -> PathBuf {
    mecha_home
        .join("sidecars")
        .join("router")
        .join("models.ini")
}

/// The unit file and launcher, rendered for this install.
fn render(text: &str, naming: &Naming, bin: &Path, presets: &Path) -> String {
    text.replace("8080", &naming.port.to_string())
        .replace("llama-local", &naming.stem)
        .replace("%h/.local/bin/", &format!("{}/", bin.display()))
        .replace("@PRESETS@", &presets.display().to_string())
}

/// The preset for a choice on this machine, its model fetched when it is the
/// pinned row's. A brought model must already be a file.
pub async fn preset_for(
    choice: &Choice,
    machine: &crate::recommend::Machine,
    hub: &Path,
    say: Say<'_>,
) -> Result<Preset> {
    match choice {
        Choice::Own { model, mmproj } => {
            for f in std::iter::once(model).chain(mmproj.iter()) {
                if !f.is_file() {
                    bail!("{} is not a file", f.display());
                }
            }
            Ok(Preset {
                alias: alias_for(model),
                model: model.clone(),
                mmproj: mmproj.clone(),
                ctx: 32_768,
                slots: 1,
                cache_ram_mb: None,
                pinned: false,
            })
        }
        Choice::Recommended => {
            let slot = crate::recommend::SLOTS
                .iter()
                .find(|s| s.id == "chat")
                .context("no chat slot")?;
            let (row, _) = crate::recommend::row_for(slot, machine).context(
                "no chat model is recommended at this machine's tier — `mecha setup chat` lets \
                 you bring your own GGUF",
            )?;
            let geo = crate::recommend::chat_geometry(row.tier_gb)
                .with_context(|| format!("no chat geometry for the {} GB tier", row.tier_gb))?;
            let mut paths = Vec::new();
            for source in row.sources {
                let crate::recommend::Source::HuggingFace {
                    repo,
                    revision,
                    files,
                } = source
                else {
                    bail!("the chat model is not a Hugging Face file");
                };
                for f in *files {
                    if crate::fetch::cached(hub, repo, revision, f, false)?
                        == crate::fetch::Cached::Verified
                    {
                        say(&format!("{repo}/{} is in the cache", f.path));
                    } else {
                        say(&format!(
                            "fetching {repo}/{} ({:.1} GiB)",
                            f.path,
                            f.bytes as f64 / 1_073_741_824.0
                        ));
                    }
                    paths.push(
                        crate::fetch::fetch_hub_file(hub, repo, revision, f, &mut |_| {}).await?,
                    );
                }
            }
            let model = paths
                .first()
                .cloned()
                .context("the chat row pins no file")?;
            let mmproj = paths
                .iter()
                .find(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().contains("mmproj"))
                })
                .cloned();
            Ok(Preset {
                alias: PINNED_ALIAS.into(),
                model,
                mmproj,
                ctx: geo.ctx,
                slots: geo.slots,
                cache_ram_mb: Some(geo.cache_ram_mb),
                pinned: true,
            })
        }
    }
}

/// Install the router serving `choice` under `naming`: the engine resolved
/// first (nothing is fetched for a machine that has none), the model fetched,
/// the presets file, launcher and unit written, the service enabled and
/// started, and the model asked to load. The alias it serves, when it does.
pub async fn install(
    m: &Machinery,
    choice: &Choice,
    naming: &Naming,
    machine: &crate::recommend::Machine,
    hub: &Path,
    say: Say<'_>,
) -> Result<String> {
    if !cfg!(target_os = "linux") {
        bail!(
            "the router is a systemd user unit, which this system does not have — on macOS start \
             llama-server by hand until launchd is designed (FEATURES-DESIGN §10.5)"
        );
    }
    let home = &m.mecha_home;
    let engine = Engine::resolve(m)?;
    let units = crate::engine_gate::unit_dir(m)?.to_path_buf();
    let bin = bin_dir(home);
    Manifest::begin(home, ID)?;

    let preset = preset_for(choice, machine, hub, &mut *say).await?;
    let presets = presets_path(home);
    write_owned(home, ID, &presets, &presets_text(&preset), 0o644)?;
    let r = |t: &str| render(t, naming, &bin, &presets);
    write_owned(
        home,
        ID,
        &bin.join("mecha-router"),
        &r(ROUTER_LAUNCHER),
        0o755,
    )?;
    write_owned(
        home,
        ID,
        &bin.join("mecha-wait-healthy"),
        WAIT_HEALTHY,
        0o755,
    )?;
    let stem = &naming.stem;
    write_owned(
        home,
        ID,
        &units.join(format!("{stem}.service")),
        &r(ROUTER_SERVICE),
        0o644,
    )?;
    let dropins = units.join(format!("{stem}.service.d"));
    write_owned(home, ID, &dropins.join("path.conf"), PATH_CONF, 0o644)?;
    engine.write(home, ID, &dropins)?;

    crate::engine_gate::systemctl("daemon-reload", &[])?;
    crate::engine_gate::systemctl("enable", &[&format!("{stem}.service")])?;
    crate::engine_gate::systemctl("restart", &[&format!("{stem}.service")])?;
    say(&format!(
        "loading {} on the router at :{} — up to fifteen minutes for a large model",
        preset.alias, naming.port
    ));
    check(naming, &preset.alias).await?;
    linger_note(say);
    Manifest::finish(home, ID)?;
    Ok(preset.alias)
}

/// The router answers, and the model is resident — asked of the router, never
/// inferred from the service starting.
async fn check(naming: &Naming, alias: &str) -> Result<()> {
    let base = naming.base();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let started = std::time::Instant::now();
    while !client
        .get(format!("{base}/health"))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
    {
        if started.elapsed() > Duration::from_secs(180) {
            bail!(
                "the router on :{} did not answer /health within 180 s — `journalctl --user -u \
                 {}` says why",
                naming.port,
                naming.stem
            );
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    crate::provider::router::load(&base, alias, Duration::from_secs(900))
        .await
        .with_context(|| format!("loading {alias} on the router"))?;
    let resident = crate::provider::router::models(&base)
        .await
        .and_then(|ms| crate::provider::router::resident(&ms).map(str::to_owned));
    if resident.as_deref() != Some(alias) {
        bail!(
            "the router answers, but {alias} is not resident (it holds {})",
            resident.as_deref().unwrap_or("nothing")
        );
    }
    Ok(())
}

/// A user unit starts at boot only for a user who lingers; without it, the
/// router starts at the first login. Said, never changed: `enable-linger` is
/// the owner's to run.
fn linger_note(say: Say<'_>) {
    let user = std::env::var("USER").unwrap_or_default();
    let lingers = std::process::Command::new("loginctl")
        .args(["show-user", &user, "--property=Linger", "--value"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "yes");
    if !lingers && !user.is_empty() {
        say(&format!(
            "the router starts with your session, not at boot — `loginctl enable-linger {user}` \
             starts it at boot"
        ));
    }
}

/// Stop and remove a router installed under `naming` — what the ignored test
/// cleans up with. Each record is forgotten only after its file is gone, and
/// the entry goes when nothing is left in it.
pub fn remove(m: &Machinery, naming: &Naming) -> Result<()> {
    let home = &m.mecha_home;
    let stem = &naming.stem;
    let _ = crate::engine_gate::systemctl("disable", &["--now", &format!("{stem}.service")]);
    let gone = |p: &Path, ids: &[&str]| -> Result<()> {
        match std::fs::remove_file(p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("removing {}", p.display())),
        }
        for id in ids {
            Manifest::unrecord(home, id, p)?;
        }
        Ok(())
    };
    let units = crate::engine_gate::unit_dir(m)?.to_path_buf();
    gone(&units.join(format!("{stem}.service")), &[ID])?;
    let dropins = units.join(format!("{stem}.service.d"));
    gone(&dropins.join("path.conf"), &[ID])?;
    gone(&dropins.join("engine-provided.conf"), &[ID])?;
    gone(&dropins.join("mecha-engine.conf"), &["llama", ID])?;
    let _ = std::fs::remove_dir(&dropins);
    let bin = bin_dir(home);
    gone(&bin.join("mecha-router"), &[ID])?;
    gone(&presets_path(home), &[ID])?;
    let wait = bin.join("mecha-wait-healthy");
    let others = Manifest::read(home)?
        .entries
        .iter()
        .any(|e| e.sidecar != ID && e.wrote.contains(&wait));
    if others {
        Manifest::unrecord(home, ID, &wait)?;
    } else {
        gone(&wait, &[ID])?;
    }
    crate::engine_gate::systemctl("daemon-reload", &[])?;
    Manifest::forget_if_empty(home, ID)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pinned row's preset is the GB10's, at the tier's geometry; a
    /// brought model's is plain.
    #[test]
    fn presets_carry_the_row_s_geometry_and_the_pinned_lines() {
        let geo = crate::recommend::chat_geometry(128).unwrap();
        let pinned = presets_text(&Preset {
            alias: PINNED_ALIAS.into(),
            model: "/hub/q.gguf".into(),
            mmproj: Some("/hub/mmproj-BF16.gguf".into()),
            ctx: geo.ctx,
            slots: geo.slots,
            cache_ram_mb: Some(geo.cache_ram_mb),
            pinned: true,
        });
        for want in [
            "[qwen3.6-35b-a3b]",
            "model = /hub/q.gguf",
            "mmproj = /hub/mmproj-BF16.gguf",
            "ctx-size = 1048576",
            "parallel = 4",
            "cache-ram = 16384",
            "spec-type = draft-mtp",
            "reasoning-budget = 4096",
            "load-on-startup = true",
            "n-gpu-layers = 999",
        ] {
            assert!(pinned.contains(want), "{want} missing:\n{pinned}");
        }
        let own = presets_text(&Preset {
            alias: "my-model".into(),
            model: "/m/My Model.gguf".into(),
            mmproj: None,
            ctx: 32_768,
            slots: 1,
            cache_ram_mb: None,
            pinned: false,
        });
        assert!(
            own.contains("[my-model]") && own.contains("ctx-size = 32768"),
            "{own}"
        );
        assert!(
            !own.contains("spec-type") && !own.contains("mmproj"),
            "{own}"
        );
    }

    #[test]
    fn a_brought_model_is_named_from_its_file() {
        assert_eq!(
            alias_for(Path::new("/m/Llama-3.1-8B_Q4.gguf")),
            "llama-3.1-8b-q4"
        );
        assert_eq!(alias_for(Path::new("/m/___.gguf")), "local");
    }

    /// Rendered for a test naming, nothing of the live router remains.
    #[test]
    fn the_router_renders_onto_another_name_and_port() {
        let n = Naming {
            stem: "mecha-test-router".into(),
            port: 41080,
        };
        let unit = render(
            ROUTER_SERVICE,
            &n,
            Path::new("/srv/m/sidecars/bin"),
            Path::new("/srv/m/sidecars/router/models.ini"),
        );
        assert!(unit.contains("MECHA_ROUTER_PORT=41080"), "{unit}");
        assert!(
            unit.contains("ExecStart=/srv/m/sidecars/bin/mecha-router"),
            "{unit}"
        );
        assert!(
            unit.contains("MECHA_ROUTER_PRESETS=/srv/m/sidecars/router/models.ini"),
            "{unit}"
        );
        assert!(unit.contains("mecha-wait-healthy 41080"), "{unit}");
        for live in ["8080", "llama-local", "%h/.local", "@PRESETS@"] {
            assert!(!unit.contains(live), "{live} left in:\n{unit}");
        }
    }

    /// The geometry the preset serves is the one the row's memory figure was
    /// computed from: one table, so the plan and the server cannot disagree.
    #[test]
    fn every_chat_row_has_its_geometry() {
        let slot = crate::recommend::SLOTS
            .iter()
            .find(|s| s.id == "chat")
            .unwrap();
        for row in slot.rows {
            assert!(
                crate::recommend::chat_geometry(row.tier_gb).is_some(),
                "the {} GB chat row has no geometry",
                row.tier_gb
            );
        }
    }

    /// The real thing beside the live router: a router installed under its
    /// own name on a free port in this user's systemd, serving a small model
    /// brought as the owner's (the embeddings GGUF, already in the hub),
    /// started, the model resident — then removed, record and all.
    #[tokio::test]
    #[ignore]
    async fn a_router_installs_and_serves_beside_the_live_one() {
        let hub = crate::fetch::hub_dir().unwrap();
        let model =
            std::fs::read_dir(hub.join("models--mradermacher--harrier-oss-v1-0.6b-GGUF/snapshots"))
                .unwrap()
                .flatten()
                .map(|e| e.path().join("harrier-oss-v1-0.6b.f16.gguf"))
                .find(|p| p.is_file())
                .expect("the embeddings model in the hub");
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let naming = Naming {
            stem: format!("mecha-test-router-{}", std::process::id()),
            port,
        };
        let root = std::env::temp_dir().join(format!("mecha-7c2-{}", uuid::Uuid::new_v4()));
        let mut m = Machinery::real().unwrap();
        m.mecha_home = root.join(".mecha");
        let machine = crate::recommend::Machine::read().unwrap();
        let mut said = Vec::new();
        let r = install(
            &m,
            &Choice::Own {
                model,
                mmproj: None,
            },
            &naming,
            &machine,
            &hub,
            &mut |s| said.push(s.to_string()),
        )
        .await;
        let cleaned = remove(&m, &naming);
        eprintln!("{said:#?}");
        assert_eq!(r.unwrap(), "harrier-oss-v1-0.6b.f16");
        cleaned.unwrap();
        let man = Manifest::read(&m.mecha_home).unwrap();
        assert!(
            !man.entries.iter().any(|e| e.sidecar == ID),
            "{:?}",
            man.entries
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
