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
//! it. Linux only, as the other units are (§10.5).

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
    if !cfg!(target_os = "linux") {
        bail!(
            "the speech-to-text server is a systemd user unit, which this system does not have — \
             on macOS start it by hand until launchd is designed (FEATURES-DESIGN §10.5)"
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

    // --- the environment
    let uv = crate::install::ensure_uv(m, say).await?;
    let venv = dir.join("venv");
    let python = venv.join("bin/python");
    let model = dir.join(MODEL_DIR);
    // Each piece on its own line in the record, so a partly deleted install
    // reads incomplete rather than installed — the managed Python included,
    // which the venv's interpreter links into (`install_layout`'s reason).
    Manifest::record(home, ID, &venv)?;
    Manifest::record(home, ID, &dir.join("python"))?;
    Manifest::record(home, ID, &model)?;
    if !python.exists() {
        say(&format!("building a Python {PYTHON} environment"));
        let mut c = std::process::Command::new(&uv);
        c.args(["venv", "--quiet", "--clear", "--python", PYTHON])
            .arg(&venv);
        crate::install::uv_env(&mut c, &dir);
        crate::install::run(&mut c, "uv venv")?;
    }
    let lock = dir.join("requirements.txt");
    write_owned(home, ID, &lock, LOCK, 0o644)?;
    say("installing sherpa-onnx and the server, every file checked against the lock");
    let mut c = std::process::Command::new(&uv);
    c.args(["pip", "install", "--quiet", "--require-hashes", "--python"])
        .arg(&python)
        .arg("-r")
        .arg(&lock);
    crate::install::uv_env(&mut c, &dir);
    crate::install::run(&mut c, "uv pip install")?;

    // --- the model: fetched by its pin, unpacked beside, swapped in whole
    if !model.join("tokens.txt").is_file() {
        let (repo, tag, asset, sha256, bytes) = pin()?;
        say(&format!(
            "fetching the speech-to-text model ({:.0} MiB)",
            bytes as f64 / 1_048_576.0
        ));
        let downloads = dir.join("downloads");
        Manifest::record(home, ID, &downloads)?;
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
        Manifest::record(home, ID, &staging)?;
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
        let _ = std::fs::remove_dir_all(&model);
        std::fs::rename(&unpacked, &model)?;
        std::fs::remove_dir_all(&staging)?;
        Manifest::unrecord(home, ID, &staging)?;
        // The tarball has served its purpose: half a gigabyte kept for
        // nothing. A reinstall fetches it again.
        std::fs::remove_dir_all(&downloads)?;
        Manifest::unrecord(home, ID, &downloads)?;
    }

    // --- the server and its unit
    write_owned(home, ID, &dir.join("parakeet_server.py"), SERVER, 0o644)?;
    write_owned(home, ID, &unit, &render(&dir, naming), 0o644)?;
    crate::engine_gate::systemctl("daemon-reload", &[])?;
    crate::engine_gate::systemctl("enable", &["--now", &format!("{}.service", naming.unit)])?;
    say(&format!(
        "starting it on :{} and transcribing a second of silence",
        naming.port
    ));
    check(naming.port).await?;
    Manifest::finish(home, ID)
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
            Ok(r) if r.status().is_success() => break,
            Ok(r) if std::time::Instant::now() > deadline => bail!(
                "the speech-to-text server on :{port} answered {} to /health",
                r.status()
            ),
            Err(e) if std::time::Instant::now() > deadline => {
                return Err(e).with_context(|| {
                    format!(
                        "the speech-to-text server on :{port} did not answer within two minutes \
                         — `journalctl --user -u <its unit>` says why"
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
        assert!(!text.contains("Github") && !text.contains("WorkingDirectory"));
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
        let cleaned = remove(&m, &naming);
        eprintln!("{said:#?}");
        let _ = std::fs::remove_dir_all(&root);
        r.unwrap();
        cleaned.unwrap();
        let man = Manifest::read(&m.mecha_home).unwrap_or_default();
        assert!(
            !man.entries.iter().any(|e| e.sidecar == ID),
            "{:?}",
            man.entries
        );
    }
}
