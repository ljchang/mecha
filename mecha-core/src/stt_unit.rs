//! The speech-to-text server from nothing (`docs/FEATURES-DESIGN.md` §10.6,
//! step 7d-1): Parakeet TDT 0.6B v3 int8 on sherpa-onnx, behind the
//! OpenAI-shaped `/v1/audio/transcriptions` that `[voice] stt_url` names.
//!
//! **Its own tree, its own environment.** Everything lands under
//! `~/.mecha/sidecars/stt/`: a Python built by mecha's `uv` from a lock with
//! every file pinned by hash (`locks/stt-requirements.txt`), the model
//! extracted from its pinned release tarball, and the server script — a copy
//! of `scripts/voice/parakeet_server.py` shipped inside the binary, held byte
//! for byte to it by a test. What runs is what mecha wrote, never a working
//! tree (the 2026-08-20 rule `llama_units` keeps for the same reason).
//!
//! **The record leads the bytes.** Every path is recorded in the manifest
//! before it is written, so an install interrupted anywhere reads
//! *incomplete* — resumable — rather than installed or someone else's.
//!
//! **One unit, rendered.** A plain service on loopback, not the on-demand
//! socket trio: the model loads in seconds and holds about 700 MB of RAM,
//! and a dictation must not wait on a cold start. The unit's name and port
//! are parameters, so a test installs one beside the live server and removes
//! it. A systemd user unit on Linux, a launchd agent on a Mac read from the
//! same unit (`user_service`, FEATURES-DESIGN §12.2).

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::install::Say;
use crate::llama_units::write_owned;
use crate::sidecar::{Machinery, Manifest};

/// The sidecar this installs (`sidecar::SIDECARS`).
pub const ID: &str = "stt";

/// The server script, as `scripts/voice/parakeet_server.py` has it.
const SERVER: &str = include_str!("../units/parakeet_server.py");
/// The unit template: `@DIR@`, `@VENV@`, `@MODEL@` and `@PORT@` rendered.
const UNIT: &str = include_str!("../units/mecha-parakeet.service");
/// Every file of the server's environment, pinned by hash.
const LOCK: &str = include_str!("../locks/stt-requirements.txt");
/// The Python the lock was compiled for.
const PYTHON: &str = "3.12";
/// What the tarball unpacks to, and what the server reads.
const MODEL_DIR: &str = "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8";
/// Written into the model's tree when it is unpacked: the sha256 of the
/// tarball it came from, so a later install knows whose model it is.
const PIN_STAMP: &str = ".mecha-pin";

/// Where the unit is installed and what port it answers on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Naming {
    /// The unit's name without `.service`.
    pub unit: String,
    pub port: u16,
}

impl Naming {
    /// What `[voice] stt_url`'s default names.
    pub fn shipped() -> Naming {
        Naming {
            unit: "mecha-parakeet".into(),
            port: 8992,
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
        .replace("@MODEL@", &dir.join(MODEL_DIR).display().to_string())
        .replace("@PORT@", &naming.port.to_string())
}

/// The pinned tarball, from the `stt` recommendation slot.
fn pin() -> Result<(&'static str, &'static str, &'static str, &'static str, u64)> {
    let slot = crate::recommend::SLOTS
        .iter()
        .find(|s| s.id == "stt")
        .context("no speech-to-text slot")?;
    for row in slot.rows {
        for source in row.sources {
            if let crate::recommend::Source::ReleaseAsset {
                repo,
                tag,
                asset,
                sha256,
                bytes,
            } = source
            {
                return Ok((repo, tag, asset, sha256, *bytes));
            }
        }
    }
    bail!("the speech-to-text slot pins no release asset")
}

/// What the install downloads that is known before it runs: the model's
/// tarball. The environment's wheels are not priced, as layout's are not.
pub fn download_bytes() -> Option<u64> {
    pin().ok().map(|(.., bytes)| bytes)
}

pub async fn install(m: &Machinery, naming: &Naming, say: Say<'_>) -> Result<()> {
    if !crate::router_unit::installs_here() {
        bail!(
            "the speech-to-text server installs as a systemd user unit or a launchd agent, and \
             this system has neither — start it by hand"
        );
    }
    let home = &m.mecha_home;
    let dir = dir(home);
    // A server on the port whose unit mecha did not write is someone's — a
    // Parakeet started from a terminal, or under another unit name. Nothing
    // is installed beside it: it would answer the check below while mecha's
    // own restart-looped against the port (`router_unit`'s refusal, #568;
    // review of #638).
    // `features enable` leaves such a server out of what it offers
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

    // --- the environment
    crate::install::locked_env(m, ID, &dir, PYTHON, LOCK, "sherpa-onnx and the server", say)
        .await?;
    let model = dir.join(MODEL_DIR);
    Manifest::record(home, ID, &model)?;

    // --- the model: fetched by its pin, unpacked beside, swapped in whole.
    // Kept only when it is this pin's: the tag is a rolling one, so a model
    // that is merely present may be an older pin's (review of #638).
    let (repo, tag, asset, sha256, bytes) = pin()?;
    let stamp = model.join(PIN_STAMP);
    let this_pin = model.join("tokens.txt").is_file()
        && std::fs::read_to_string(&stamp).is_ok_and(|s| s.trim() == sha256);
    if !this_pin {
        say(&format!(
            "fetching the speech-to-text model ({:.0} MiB)",
            bytes as f64 / 1_048_576.0
        ));
        // The two scratch trees live inside the recorded tree and are not
        // recorded themselves: a record whose path is gone reads the install
        // incomplete for good, and one interrupted after the model landed
        // would never come back to unrecord them (review of #638).
        let downloads = dir.join("downloads");
        let tarball = crate::fetch::fetch_release_asset(
            repo,
            tag,
            asset,
            sha256,
            bytes,
            &downloads,
            &mut |_| {},
        )
        .await?;
        say("unpacking it");
        let staging = dir.join("model.new");
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging)?;
        let mut c = std::process::Command::new("tar");
        c.arg("-xjf").arg(&tarball).arg("-C").arg(&staging);
        crate::install::run(&mut c, "unpacking the model")?;
        let unpacked = staging.join(MODEL_DIR);
        for f in [
            "encoder.int8.onnx",
            "decoder.int8.onnx",
            "joiner.int8.onnx",
            "tokens.txt",
        ] {
            if !unpacked.join(f).is_file() {
                bail!("the model's tarball has no {MODEL_DIR}/{f}");
            }
        }
        std::fs::write(unpacked.join(PIN_STAMP), sha256)?;
        let _ = std::fs::remove_dir_all(&model);
        std::fs::rename(&unpacked, &model)?;
        std::fs::remove_dir_all(&staging)?;
        // The tarball has served its purpose: half a gigabyte kept for
        // nothing. A reinstall fetches it again.
        std::fs::remove_dir_all(&downloads)?;
    }

    // --- the server and its unit
    write_owned(home, ID, &dir.join("parakeet_server.py"), SERVER, 0o644)?;
    crate::user_service::write(m, ID, &naming.unit, &render(&dir, naming))?;
    // A restart, not `enable --now`: a reinstall over a running server must
    // reach it with the new script and model, or `check` passes against the
    // old process (`router_unit`'s reason; review of #638).
    // A failed start or check stops the server before the install says so,
    // as the worker's and the speech server's do (#639's pattern): on a Mac,
    // `bootstrap` succeeds for a program that cannot start, and the agent
    // would otherwise relaunch forever after an install that failed (review
    // of #679).
    let checked = match crate::user_service::restart(m, &[&naming.unit], &mut *say) {
        Ok(()) => {
            say(&format!(
                "starting it on :{} and transcribing a second of silence",
                naming.port
            ));
            check(naming.port).await
        }
        Err(e) => Err(e),
    };
    if let Err(e) = checked {
        let service = naming.unit.as_str();
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

/// Whether a `/health` answer says the model is in: `{"status": "ok"}`.
async fn ready(r: reqwest::Response) -> bool {
    r.json::<serde_json::Value>()
        .await
        .is_ok_and(|v| v["status"] == "ok")
}

/// Whether a server answers on the port whose unit mecha did not write —
/// someone's own Parakeet, from a terminal or under another unit name. Its
/// install would be refused, so it is never offered: it is used as it is.
/// One loopback connect; nothing is sent.
pub fn someone_elses(m: &Machinery, naming: &Naming) -> Result<bool> {
    let unit = crate::user_service::file(m, &naming.unit)?;
    Ok(!wrote_unit(&m.mecha_home, &unit)? && crate::router_unit::port_answers(naming.port))
}

/// Whether the record names this unit's file: mecha installed the server on
/// its port, and a reinstall may find it answering.
fn wrote_unit(home: &Path, unit: &Path) -> Result<bool> {
    Ok(Manifest::read(home)?
        .entries
        .iter()
        .any(|e| e.sidecar == ID && e.wrote.iter().any(|w| w == unit)))
}

/// The server answers `/health` and transcribes a clip: the model loads and
/// runs, not only the process. Retried while it starts — uvicorn listens
/// only once the model is loaded.
async fn check(port: u16) -> Result<()> {
    let base = format!("http://127.0.0.1:{port}");
    // Loopback, never through a proxy (`llama_units::check`'s reason).
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    loop {
        match client.get(format!("{base}/health")).send().await {
            // The body, not only the status: the server answers 200 with
            // `"loading"` until its model is in (review of #638).
            Ok(r) => {
                let status = r.status();
                if status.is_success() && ready(r).await {
                    break;
                }
                if std::time::Instant::now() > deadline {
                    bail!(
                        "the speech-to-text server on :{port} answered {status} to /health, \
                         not ready"
                    );
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) if std::time::Instant::now() > deadline => {
                return Err(e).with_context(|| {
                    format!(
                        "the speech-to-text server on :{port} did not answer within two minutes \
                         — {}",
                        crate::user_service::logs_anywhere()
                    )
                })
            }
            _ => tokio::time::sleep(Duration::from_secs(2)).await,
        }
    }
    // One multipart part, written out: the crate's reqwest has no multipart
    // feature, and a dependency feature for one health check is not worth it.
    let boundary = "mecha-stt-check";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; \
         filename=\"check.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(&silent_wav(16_000));
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let r = client
        .post(format!("{base}/v1/audio/transcriptions"))
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await
        .context("the first transcription")?;
    let status = r.status();
    let body: serde_json::Value = r.json().await.unwrap_or_default();
    if !status.is_success() || !body["text"].is_string() {
        bail!("the speech-to-text server answered {status} to its first transcription: {body}");
    }
    Ok(())
}

/// One second of 16-bit mono silence, as a WAV file.
fn silent_wav(rate: u32) -> Vec<u8> {
    let data = rate * 2;
    let mut w = Vec::with_capacity(44 + data as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data.to_le_bytes());
    w.resize(44 + data as usize, 0);
    w
}

/// Stop and remove a server installed under `naming`, its tree included —
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
    // A record is forgotten only after its file is gone, so a removal that
    // fails keeps it, and the next install does not refuse mecha's own file.
    for p in [
        dir.join("parakeet_server.py"),
        dir.join("requirements.txt"),
        dir.join(MODEL_DIR),
        dir.join("venv"),
        dir.join("python"),
        dir.join("downloads"),
        dir.join("model.new"),
        dir.clone(),
    ] {
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

    /// What runs is the script the hand install runs, byte for byte — so a
    /// fix made in one reaches the other or fails here.
    #[test]
    fn the_shipped_server_is_the_scripts_copy() {
        let script = include_str!("../../scripts/voice/parakeet_server.py");
        assert_eq!(SERVER, script);
    }

    /// The lock pins every file by hash and names the server's imports.
    #[test]
    fn the_lock_pins_every_file() {
        assert!(LOCK.contains("--hash=sha256:"));
        for pkg in [
            "sherpa-onnx==",
            "sherpa-onnx-core==",
            "fastapi==",
            "uvicorn==",
            "numpy==",
            "python-multipart==",
        ] {
            assert!(LOCK.contains(pkg), "{pkg}");
        }
        // Every requirement line carries a hash: `--require-hashes` refuses
        // a lock with one that does not, at install time on someone's box.
        let mut lines = LOCK.lines().peekable();
        while let Some(l) = lines.next() {
            if l.contains("==") && !l.starts_with('#') && !l.trim_start().starts_with('#') {
                assert!(
                    lines.peek().is_some_and(|n| n.contains("--hash=sha256:")),
                    "{l}"
                );
            }
        }
    }

    /// The unit names this home's tree and this port, and nothing of a
    /// checkout or of anyone's home directory.
    #[test]
    fn the_unit_renders_this_home_and_port() {
        let dir = Path::new("/srv/owner/.mecha/sidecars/stt");
        let text = render(
            dir,
            &Naming {
                unit: "x".into(),
                port: 9001,
            },
        );
        assert!(!text.contains('@'), "{text}");
        assert!(text.contains("--port 9001"), "{text}");
        assert!(
            text.contains("ExecStart=/srv/owner/.mecha/sidecars/stt/venv/bin/uvicorn --app-dir /srv/owner/.mecha/sidecars/stt parakeet_server:app"),
            "{text}"
        );
        assert!(text.contains(&format!(
            "PARAKEET_DIR=/srv/owner/.mecha/sidecars/stt/{MODEL_DIR}"
        )));
        assert!(!text.contains("WorkingDirectory"));
    }

    /// The pin is the tarball this unpacks: named for the directory the
    /// server reads, bzip2 as `tar -xjf` expects, the slot's only row — so a
    /// pin bump fails here, not on someone's machine (review of #638).
    #[test]
    fn the_pin_is_the_tarball_this_unpacks() {
        let (_, _, asset, _, bytes) = pin().unwrap();
        assert_eq!(asset, format!("{MODEL_DIR}.tar.bz2"));
        assert_eq!(download_bytes(), Some(bytes));
        let slot = crate::recommend::SLOTS
            .iter()
            .find(|s| s.id == "stt")
            .unwrap();
        assert_eq!(slot.rows.len(), 1, "a second row would need choosing");
    }

    /// The shipped port is the one `[voice] stt_url` names by default.
    #[test]
    fn the_shipped_port_is_the_stt_url_default() {
        assert_eq!(
            crate::config::VoiceConfig::DEFAULT_STT_URL,
            format!("http://127.0.0.1:{}/v1", Naming::shipped().port)
        );
    }

    /// The clip the check sends is a WAV the server's reader accepts: 44
    /// bytes of header, a second of samples.
    #[test]
    fn the_check_clip_is_a_second_of_wav() {
        let w = silent_wav(16_000);
        assert_eq!(w.len(), 44 + 32_000);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()), 32_000);
    }

    /// A server on the port is someone's until the record names the unit
    /// mecha wrote there — then it is mecha's own, being reinstalled.
    #[test]
    fn a_server_is_someone_elses_until_mecha_wrote_its_unit() {
        let root = std::env::temp_dir().join(format!("mecha-7d1-whose-{}", uuid::Uuid::new_v4()));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let naming = Naming {
            unit: "mecha-test-stt-whose".into(),
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
            // The service's own file on this system: a `.service` unit, or a
            // launchd plist on a Mac (macOS CI on #679).
            &crate::user_service::file(&m, &naming.unit).unwrap(),
        )
        .unwrap();
        let ours = someone_elses(&m, &naming).unwrap();
        drop(listener);
        let nobody = someone_elses(&m, &naming).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert!(stranger);
        assert!(!ours);
        assert!(!nobody);
    }

    /// A server mecha did not install on the port is refused before a byte
    /// is written or recorded (review of #638).
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_strangers_server_on_the_port_is_not_installed_over() {
        let root = std::env::temp_dir().join(format!("mecha-7d1-port-{}", uuid::Uuid::new_v4()));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let naming = Naming {
            unit: "mecha-test-stt-stranger".into(),
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

    /// The real thing: build the environment, fetch and unpack the model,
    /// install a unit beside the live one on a free port under a scratch
    /// mecha home, transcribe silence through it, and remove it.
    /// `cargo test -p mecha-core -- --ignored a_speech_to_text_server_installs`
    #[tokio::test]
    #[ignore]
    async fn a_speech_to_text_server_installs_and_answers_beside_the_live_one() {
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let naming = Naming {
            unit: format!("mecha-test-stt-{}", std::process::id()),
            port,
        };
        let root = std::env::temp_dir().join(format!("mecha-7d1-{}", uuid::Uuid::new_v4()));
        let mut m = Machinery::real().unwrap();
        m.mecha_home = root.join(".mecha");
        let mut said = Vec::new();
        let r = install(&m, &naming, &mut |s| said.push(s.to_string())).await;
        // Read before the tree goes: the model says whose pin it is.
        let stamped = std::fs::read_to_string(dir(&m.mecha_home).join(MODEL_DIR).join(PIN_STAMP));
        let cleaned = remove(&m, &naming);
        eprintln!("{said:#?}");
        // Read before the scratch tree goes: the manifest lives inside it
        // (`Manifest::path`), and an absent one reads as no entries — the
        // assertion would hold whatever `remove` left (review of #639).
        let man = Manifest::read(&m.mecha_home);
        let _ = std::fs::remove_dir_all(&root);
        r.unwrap();
        cleaned.unwrap();
        assert_eq!(stamped.unwrap(), pin().unwrap().3);
        let man = man.unwrap();
        assert!(
            !man.entries.iter().any(|e| e.sidecar == ID),
            "{:?}",
            man.entries
        );
    }
}
