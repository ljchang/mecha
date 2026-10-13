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
//! a test installs one beside the live worker and removes it. A launchd
//! agent on a Mac, read from the same unit (`user_service`, §12.2).

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
    if !crate::router_unit::installs_here() {
        bail!(
            "the voice worker installs as a systemd user unit or a launchd agent, and this \
             system has neither — start it by hand"
        );
    }
    let home = &m.mecha_home;
    let dir = dir(home);
    // A worker on the port whose unit mecha did not write is someone's —
    // the hand-built one under another name, or a terminal's. Nothing is
    // installed beside it: it would pass the checks below while mecha's own
    // restart-looped against the port (`stt_unit`'s refusal, #638).
    // `features enable` leaves such a worker out of what it offers
    // (`someone_elses`), so this is the backstop for one that started since.
    if someone_elses(m, naming)? {
        bail!(
            "something already answers on :{} and mecha did not install it — nothing is \
             installed over it; `mecha features enable voice --no-install` switches voice on \
             and uses it as it is",
            naming.port
        );
    }
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
        // Not in the plan's total (as layout's is not): said here instead,
        // both figures (review of #639) — 256 MB of wheels on linux-aarch64
        // by the lock's own pins (PyPI, 2026-10-10), 781 MB in `venv/` once
        // installed from this lock (measured the same day), plus the Python.
        "pipecat and its WebRTC, VAD and turn models (about 250 MB to fetch, about 800 MB on \
         disk)",
        say,
    )
    .await?;
    for (name, text) in FILES {
        write_owned(home, ID, &dir.join(name), text, 0o644)?;
    }
    crate::user_service::write(m, ID, &naming.unit, &render(&dir, naming))?;
    let service = naming.unit.as_str();
    // A restart, not `enable --now`: a reinstall must reach a running worker
    // with the new files, or `check` passes against the old process.
    // A failed restart, either check failing: each stops the worker before
    // the install says so. One that would take a page's call is never left
    // listening behind the error, and one that never started or answered is
    // not left enabled to restart forever at boot — `Type=exec` makes a bad
    // start fail the restart itself, so that is the likeliest way here
    // (review of #639).
    let checked = match crate::user_service::restart(m, &[service], &mut *say) {
        Ok(()) => {
            say(&format!(
                "starting it on :{} and checking the direct door is shut",
                naming.port
            ));
            match serves(naming.port).await {
                Ok(()) => door_shut(naming.port).await,
                Err(e) => Err(e),
            }
        }
        Err(e) => Err(e),
    };
    if let Err(e) = checked {
        // Said as it went: a stop that failed leaves the worker listening,
        // and the owner's next step depends on knowing (review of #639).
        let stopped = match crate::user_service::stop(&[service]) {
            Ok(()) => format!("{service} was stopped and disabled"),
            Err(stop) => format!(
                "{service} could NOT be stopped ({stop:#}) — it may still be listening on :{}; \
                 run `{}`",
                naming.port,
                crate::user_service::stop_by_hand(&[service])
            ),
        };
        return Err(e.context(stopped));
    }
    Manifest::finish(home, ID)
}

/// Whether a worker answers on the port whose unit mecha did not write —
/// someone's own, used as it is (`stt_unit::someone_elses`'s twin).
pub fn someone_elses(m: &Machinery, naming: &Naming) -> Result<bool> {
    let unit = crate::user_service::file(m, &naming.unit)?;
    Ok(!wrote_unit(&m.mecha_home, &unit)? && crate::router_unit::port_answers(naming.port))
}

/// Whether the record names this unit's file (`stt_unit`'s twin).
fn wrote_unit(home: &Path, unit: &Path) -> Result<bool> {
    Ok(Manifest::read(home)?
        .entries
        .iter()
        .any(|e| e.sidecar == ID && e.wrote.iter().any(|w| w == unit)))
}

/// The worker answers. Retried while it starts — pipecat imports for a few
/// seconds before it listens.
async fn serves(port: u16) -> Result<()> {
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
                        "the voice worker on :{port} did not answer within two minutes — {}",
                        crate::user_service::logs_anywhere()
                    )
                })
            }
            _ => tokio::time::sleep(Duration::from_secs(2)).await,
        }
    }
    Ok(())
}

/// The direct door is shut and the relay's is open: a page's preflight is
/// refused, a page's simple request is not read as an offer, and an offer
/// with no `Origin` — as `mecha serve` relays one — reaches the handler.
async fn door_shut(port: u16) -> Result<()> {
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
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
    // The relay's door open: an offer with no `Origin`, as `mecha serve`
    // sends one, reaches the handler — refused for its empty body, never for
    // where it came from. A pipecat that stopped letting it through would
    // pass the rest and break every call (review of #639).
    let relayed = client
        .post(format!("{base}/api/offer"))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .context("relaying an offer as mecha serve does")?;
    let relayed_status = relayed.status();
    if matches!(
        relayed_status,
        reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::UNAUTHORIZED
    ) {
        bail!(
            "the voice worker on :{port} refused an offer with no Origin ({relayed_status}) — \
             `mecha serve`'s relay sends none, so no call could start"
        );
    }
    // What reading a body as an offer answers, measured rather than named:
    // the same `{}` from a page must be answered otherwise.
    let read_as_offer = (relayed_status, relayed.text().await.unwrap_or_default());
    // A page needs no preflight for a "simple" request, so the second half of
    // the direct door is that no simple body is read as an offer — each of
    // the three content types a page may send without asking. (Pipecat's
    // offer handler checks no origin itself — a page's JSON POST reaches it,
    // measured 2026-10-10 — so a JSON offer is held off by the refused
    // preflight above, and a simple one only by this.)
    for kind in [
        "text/plain",
        "application/x-www-form-urlencoded",
        "multipart/form-data; boundary=mecha",
    ] {
        let simple = client
            .post(format!("{base}/api/offer"))
            .header("Origin", "https://a-page.example")
            .header("content-type", kind)
            .body("{}")
            .send()
            .await
            .with_context(|| format!("posting a page's {kind} request"))?;
        let status = simple.status();
        let answer = (status, simple.text().await.unwrap_or_default());
        if status.is_success() || answer == read_as_offer {
            bail!(
                "the voice worker on :{port} read a page's {kind} request as an offer \
                 ({status}) — a page could start a call without a preflight"
            );
        }
    }
    Ok(())
}

/// Stop and remove a worker installed under `naming`, its tree included —
/// what the ignored test cleans up with; `mecha setup voice --remove` will
/// stand on it.
pub fn remove(m: &Machinery, naming: &Naming) -> Result<()> {
    let home = &m.mecha_home;
    let _ = crate::user_service::stop(&[&naming.unit]);
    let path = crate::user_service::file(m, &naming.unit)?;
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("removing {}", path.display())),
    }
    Manifest::unrecord(home, ID, &path)?;
    crate::user_service::after_remove()?;
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
        // A fifth file without a fifth script would end the zip early and go
        // unchecked (review of #639).
        assert_eq!(FILES.len(), scripts.len());
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
        // What keeps "nothing else is downloaded" true: with no hub client,
        // smart-turn's weights can only be the ones inside the wheel.
        assert!(!LOCK.lines().any(|l| l.starts_with("huggingface-hub==")));
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

    /// A worker on the port is someone's until the record names the unit
    /// mecha wrote there.
    #[test]
    fn a_worker_is_someone_elses_until_mecha_wrote_its_unit() {
        let root = std::env::temp_dir().join(format!("mecha-7d2-whose-{}", uuid::Uuid::new_v4()));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let naming = Naming {
            unit: "mecha-test-worker-whose".into(),
            port: listener.local_addr().unwrap().port(),
        };
        let m = Machinery {
            home: root.clone(),
            mecha_home: root.join(".mecha"),
            unit_dirs: vec![root.join("units")],
            path: Vec::new(),
            docker: Box::new(|_| crate::sidecar::Lookup::Absent),
        };
        let stranger = someone_elses(&m, &naming).unwrap();
        Manifest::begin(&m.mecha_home, ID).unwrap();
        Manifest::record(
            &m.mecha_home,
            ID,
            &crate::user_service::file(&m, &naming.unit).unwrap(),
        )
        .unwrap();
        let ours = someone_elses(&m, &naming).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert!(stranger);
        assert!(!ours);
    }

    /// A worker mecha did not install on the port is refused before a byte
    /// is written or recorded.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_strangers_worker_on_the_port_is_not_installed_over() {
        let root = std::env::temp_dir().join(format!("mecha-7d2-port-{}", uuid::Uuid::new_v4()));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let naming = Naming {
            unit: "mecha-test-worker-stranger".into(),
            port: listener.local_addr().unwrap().port(),
        };
        let m = Machinery {
            home: root.clone(),
            mecha_home: root.join(".mecha"),
            unit_dirs: vec![root.join("units")],
            path: Vec::new(),
            docker: Box::new(|_| crate::sidecar::Lookup::Absent),
        };
        let r = install(&m, &naming, &mut |_| {}).await;
        let wrote = root.join(".mecha").exists();
        let _ = std::fs::remove_dir_all(&root);
        let err = format!("{:#}", r.unwrap_err());
        assert!(err.contains("mecha did not install it"), "{err}");
        assert!(!wrote, "nothing written or recorded");
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
