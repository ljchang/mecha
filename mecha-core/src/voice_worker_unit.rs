//! The voice worker from nothing (`docs/FEATURES-DESIGN.md` §10.6, step
//! 7d-2): the Pipecat process that wires a call together over WebRTC — the
//! browser's audio, speech to text, the facade on `[voice] voice_port`, and
//! the speech server — at `[voice] offer_target`'s default, `:7860`.
//!
//! **What runs is what mecha wrote.** `worker.py` and the three modules it
//! imports ship inside the binary — copies of `scripts/voice/`, held byte
//! for byte to them by a test — and are written into
//! `~/.mecha/sidecars/voice-worker/`, beside an environment built from a lock
//! with every file pinned by hash (`locks/voice-worker-requirements.txt`).
//! The VAD and turn models come inside the pipecat wheel; nothing else is
//! downloaded.
//!
//! **The direct door is shut without a hostname.** The hand-built unit names
//! the tailnet's origins in `--allowed-origins`; this one names an origin no
//! page has, because `mecha serve` relays the offer server to server with no
//! `Origin` header (`serve::forward_offer`), and pipecat lets a request
//! without one through while refusing a browser's (measured 2026-10-10: a
//! cross-origin preflight is refused, an origin-less POST reaches the
//! handler). So the install needs nothing machine-specific.
//!
//! **One unit, rendered,** as `stt_unit`'s: name and port are parameters, so
//! a test installs one beside the live worker and removes it. Linux only.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::install::Say;
use crate::llama_units::write_owned;
use crate::sidecar::{Machinery, Manifest};

/// The sidecar this installs (`sidecar::SIDECARS`).
pub const ID: &str = "voice-worker";

/// The worker and the modules it imports, as `scripts/voice/` has them.
const FILES: [(&str, &str); 4] = [
    ("worker.py", include_str!("../units/voice/worker.py")),
    (
        "echo_filter.py",
        include_str!("../units/voice/echo_filter.py"),
    ),
    ("fragments.py", include_str!("../units/voice/fragments.py")),
    ("journal.py", include_str!("../units/voice/journal.py")),
];
/// The unit template: `@DIR@`, `@VENV@`, `@PORT@` and `@NO_PAGE@` rendered.
const UNIT: &str = include_str!("../units/mecha-voice-worker.service");
/// Every file of the worker's environment, pinned by hash.
const LOCK: &str = include_str!("../locks/voice-worker-requirements.txt");
/// The Python the lock was compiled for.
const PYTHON: &str = "3.12";
/// An origin no page has: the direct door's allowlist (module doc).
const NO_PAGE: &str = "http://127.0.0.1:1";

/// Where the unit is installed and what port it answers on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Naming {
    /// The unit's name without `.service`.
    pub unit: String,
    pub port: u16,
}

impl Naming {
    /// What `[voice] offer_target`'s default names.
    pub fn shipped() -> Naming {
        Naming {
            unit: "mecha-voice-worker".into(),
            port: 7860,
        }
    }
}

/// The install's tree under a mecha home.
pub fn dir(mecha_home: &Path) -> PathBuf {
    mecha_home.join("sidecars").join(ID)
}

/// The unit as installed: absolute paths under this mecha home, this port.
fn render(dir: &Path, naming: &Naming) -> String {
    UNIT.replace("@DIR@", &dir.display().to_string())
        .replace("@VENV@", &dir.join("venv").display().to_string())
        .replace("@PORT@", &naming.port.to_string())
        .replace("@NO_PAGE@", NO_PAGE)
}

pub async fn install(m: &Machinery, naming: &Naming, say: Say<'_>) -> Result<()> {
    if !cfg!(target_os = "linux") {
        bail!(
            "the voice worker is a systemd user unit, which this system does not have — on \
             macOS start it by hand until launchd is designed (FEATURES-DESIGN §10.5)"
        );
    }
    let home = &m.mecha_home;
    let dir = dir(home);
    let units = crate::engine_gate::unit_dir(m)?.to_path_buf();
    let unit = units.join(format!("{}.service", naming.unit));
    Manifest::begin(home, ID)?;
    Manifest::record(home, ID, &dir)?;
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    crate::install::locked_env(
        m,
        ID,
        &dir,
        PYTHON,
        LOCK,
        "pipecat and its WebRTC, VAD and turn models",
        say,
    )
    .await?;
    for (name, text) in FILES {
        write_owned(home, ID, &dir.join(name), text, 0o644)?;
    }
    write_owned(home, ID, &unit, &render(&dir, naming), 0o644)?;
    crate::engine_gate::systemctl("daemon-reload", &[])?;
    let service = format!("{}.service", naming.unit);
    crate::engine_gate::systemctl("enable", &[&service])?;
    // A restart, not `enable --now`: a reinstall must reach a running worker
    // with the new files, or `check` passes against the old process.
    crate::engine_gate::systemctl("restart", &[&service])?;
    say(&format!(
        "starting it on :{} and checking the direct door is shut",
        naming.port
    ));
    check(naming.port).await?;
    Manifest::finish(home, ID)
}

/// The worker serves, and refuses a page's direct request: a cross-origin
/// preflight to the offer route is not allowed through. Retried while it
/// starts — pipecat imports for a few seconds before it listens.
async fn check(port: u16) -> Result<()> {
    let base = format!("http://127.0.0.1:{port}");
    // Loopback, never through a proxy (`llama_units::check`'s reason).
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    loop {
        match client.get(format!("{base}/")).send().await {
            Ok(r) if !r.status().is_server_error() => break,
            Ok(r) if std::time::Instant::now() > deadline => {
                bail!("the voice worker on :{port} answered {}", r.status())
            }
            Err(e) if std::time::Instant::now() > deadline => {
                return Err(e).with_context(|| {
                    format!(
                        "the voice worker on :{port} did not answer within two minutes — \
                         `journalctl --user -u <its unit>` says why"
                    )
                })
            }
            _ => tokio::time::sleep(Duration::from_secs(2)).await,
        }
    }
    let preflight = client
        .request(reqwest::Method::OPTIONS, format!("{base}/api/offer"))
        .header("Origin", "https://a-page.example")
        .header("Access-Control-Request-Method", "POST")
        .send()
        .await
        .context("asking the worker's offer route from a page")?;
    let allowed = preflight
        .headers()
        .get("access-control-allow-origin")
        .is_some();
    if preflight.status().is_success() && allowed {
        bail!(
            "the voice worker on :{port} let a page's request through — its origin allowlist \
             is not in force, so any page in a browser here could start a call"
        );
    }
    // And the relay's door open: an offer with no `Origin`, as `mecha serve`
    // sends one, reaches the handler — refused for its empty body, never for
    // where it came from. A pipecat that stopped letting it through would
    // pass the half above and break every call (review of #639).
    let relayed = client
        .post(format!("{base}/api/offer"))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .context("relaying an offer as mecha serve does")?;
    if matches!(
        relayed.status(),
        reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::UNAUTHORIZED
    ) {
        bail!(
            "the voice worker on :{port} refused an offer with no Origin ({}) — `mecha serve`'s \
             relay sends none, so no call could start",
            relayed.status()
        );
    }
    Ok(())
}

/// Stop and remove a worker installed under `naming`, its tree included —
/// what the ignored test cleans up with; `mecha setup voice --remove` will
/// stand on it.
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
    let mut paths: Vec<PathBuf> = FILES.iter().map(|(n, _)| dir.join(n)).collect();
    paths.extend([
        dir.join("requirements.txt"),
        dir.join("venv"),
        dir.join("python"),
        dir.clone(),
    ]);
    // A record is forgotten only after its file is gone, so a removal that
    // fails keeps it, and the next install does not refuse mecha's own file.
    for p in paths {
        let gone = if p.is_dir() {
            std::fs::remove_dir_all(&p)
        } else {
            std::fs::remove_file(&p)
        };
        match gone {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("removing {}", p.display())),
        }
        Manifest::unrecord(home, ID, &p)?;
    }
    Manifest::forget_if_empty(home, ID)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What runs is what the hand install runs, byte for byte — so a fix made
    /// in one reaches the other or fails here.
    #[test]
    fn the_shipped_worker_is_the_scripts_copy() {
        let scripts = [
            include_str!("../../scripts/voice/worker.py"),
            include_str!("../../scripts/voice/echo_filter.py"),
            include_str!("../../scripts/voice/fragments.py"),
            include_str!("../../scripts/voice/journal.py"),
        ];
        for ((name, shipped), script) in FILES.iter().zip(scripts) {
            assert_eq!(*shipped, script, "{name}");
        }
    }

    /// Every local module the worker imports ships beside it: a new one added
    /// to `scripts/voice/` and imported would otherwise run from a checkout
    /// and fail from an install.
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
                    let local = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../scripts/voice")
                        .join(format!("{m}.py"));
                    if local.is_file() {
                        assert!(shipped.contains(&m), "{m} is imported and not shipped");
                    }
                }
            }
        }
    }

    /// The lock pins every file by hash, names what the worker imports, and
    /// keeps torch out — the smart-turn extra that pulls it is not the
    /// model the worker runs.
    #[test]
    fn the_lock_pins_every_file_and_carries_no_torch() {
        assert!(LOCK.contains("--hash=sha256:"));
        for pkg in [
            "pipecat-ai==",
            "aiortc==",
            "onnxruntime==",
            "openai==",
            "httpx==",
        ] {
            assert!(LOCK.contains(pkg), "{pkg}");
        }
        assert!(!LOCK.lines().any(|l| l.starts_with("torch==")));
        let mut lines = LOCK.lines().peekable();
        while let Some(l) = lines.next() {
            if l.contains("==") && !l.trim_start().starts_with('#') {
                assert!(
                    lines.peek().is_some_and(|n| n.contains("--hash=sha256:")),
                    "{l}"
                );
            }
        }
    }

    /// The shipped port is the one `[voice] offer_target` names by default.
    #[test]
    fn the_shipped_port_is_the_offer_target_default() {
        assert_eq!(
            crate::config::VoiceConfig::DEFAULT_OFFER_TARGET,
            format!("http://127.0.0.1:{}/api/offer", Naming::shipped().port)
        );
    }

    /// The unit names this home's tree and this port, runs the written copy,
    /// and shuts the direct door — nothing of a checkout or a hostname.
    #[test]
    fn the_unit_renders_this_home_and_port() {
        let dir = Path::new("/srv/owner/.mecha/sidecars/voice-worker");
        let text = render(
            dir,
            &Naming {
                unit: "x".into(),
                port: 9002,
            },
        );
        assert!(!text.contains('@'), "{text}");
        assert!(text.contains("WorkingDirectory=/srv/owner/.mecha/sidecars/voice-worker\n"));
        assert!(text.contains(
            "ExecStart=/srv/owner/.mecha/sidecars/voice-worker/venv/bin/python -B worker.py \
             --host 127.0.0.1 --port 9002"
        ));
        assert!(text.contains(&format!("--allowed-origins {NO_PAGE}")));
        assert!(!text.contains("Github") && !text.contains("ts.net"));
    }

    /// The real thing: build the environment, install a worker beside the
    /// live one on a free port under a scratch mecha home, check it serves
    /// and refuses a page, and remove it.
    /// `cargo test -p mecha-core -- --ignored a_voice_worker_installs`
    #[tokio::test]
    #[ignore]
    async fn a_voice_worker_installs_and_answers_beside_the_live_one() {
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let naming = Naming {
            unit: format!("mecha-test-voice-worker-{}", std::process::id()),
            port,
        };
        let root = std::env::temp_dir().join(format!("mecha-7d2-{}", uuid::Uuid::new_v4()));
        let mut m = Machinery::real().unwrap();
        m.mecha_home = root.join(".mecha");
        let mut said = Vec::new();
        let r = install(&m, &naming, &mut |s| said.push(s.to_string())).await;
        let cleaned = remove(&m, &naming);
        eprintln!("{said:#?}");
        // Read before the scratch tree goes: the manifest lives inside it
        // (`Manifest::path`), and an absent one reads as no entries — the
        // assertion would hold whatever `remove` left (review of #639).
        let man = Manifest::read(&m.mecha_home);
        let _ = std::fs::remove_dir_all(&root);
        r.unwrap();
        cleaned.unwrap();
        let man = man.unwrap();
        assert!(
            !man.entries.iter().any(|e| e.sidecar == ID),
            "{:?}",
            man.entries
        );
    }
}
