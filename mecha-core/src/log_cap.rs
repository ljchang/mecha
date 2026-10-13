//! A program's output to a file that cannot grow without bound — what the
//! journal is to a systemd unit, for a launchd agent (`docs/FEATURES-DESIGN.md`
//! §12). launchd writes `StandardOutPath` forever and nothing rotates it
//! (`newsyslog` reads only its own config, which a user cannot add to), and a
//! resident router logs every request (review of #678).
//!
//! **One file and one older one.** The program's stdout and stderr go to the
//! log as they arrive; a write that would take it past the cap first moves it
//! to `<log>.1` (replacing the previous one) and starts a new one, so the two
//! together never pass twice the cap.
//!
//! **The program is the agent, as far as launchd can tell.** A stop signal is
//! passed on to it, and this process exits with its status — so `KeepAlive`
//! on an unsuccessful exit still means the program's unsuccessful exit.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::Mutex;

/// What runs, and where its output goes.
#[derive(Debug, Clone)]
pub struct Spec {
    pub log: PathBuf,
    pub max_bytes: u64,
    pub program: Vec<String>,
}

/// The cap an agent's log is written under: 20 MiB, and one older file.
pub const MAX_MB: u64 = 20;

/// The program line an agent runs instead of `program`: this binary, writing
/// `program`'s output to `log` under the cap.
pub fn wrap(mecha: &Path, log: &Path, program: Vec<String>) -> Vec<String> {
    let mut out = vec![
        mecha.display().to_string(),
        "features".into(),
        "log-cap".into(),
        "--log".into(),
        log.display().to_string(),
        "--max-mb".into(),
        MAX_MB.to_string(),
        "--".into(),
    ];
    out.extend(program);
    out
}

struct Sink {
    path: PathBuf,
    file: std::fs::File,
    size: u64,
    max: u64,
}

impl Sink {
    fn open(path: &Path, max: u64) -> Result<Sink> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("opening {}", path.display()))?;
        let size = file.metadata()?.len();
        Ok(Sink {
            path: path.to_path_buf(),
            file,
            size,
            max,
        })
    }

    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        use std::io::Write;
        if self.size > 0 && self.size + bytes.len() as u64 > self.max {
            let older = self.path.with_extension(format!(
                "{}.1",
                self.path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("log")
            ));
            std::fs::rename(&self.path, &older)
                .with_context(|| format!("rotating {}", self.path.display()))?;
            *self = Sink::open(&self.path, self.max)?;
        }
        self.file.write_all(bytes)?;
        self.size += bytes.len() as u64;
        Ok(())
    }
}

async fn pump(mut from: impl AsyncRead + Unpin, sink: Arc<Mutex<Sink>>) {
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        match from.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                // A log that cannot be written is not a reason to stop the
                // program it logs.
                let _ = sink.lock().await.write(&buf[..n]);
            }
        }
    }
}

/// Run the program until it exits, passing on a signal `stop` yields; its
/// exit code (128 + the signal for one killed by a signal).
pub async fn run_until(spec: Spec, mut stop: tokio::sync::mpsc::Receiver<i32>) -> Result<i32> {
    let (prog, args) = spec
        .program
        .split_first()
        .context("log-cap was given no program")?;
    if let Some(dir) = spec.log.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let sink = Arc::new(Mutex::new(Sink::open(&spec.log, spec.max_bytes)?));
    let mut child = tokio::process::Command::new(prog)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {prog}"))?;
    let out = tokio::spawn(pump(
        child.stdout.take().context("no stdout")?,
        sink.clone(),
    ));
    let err = tokio::spawn(pump(
        child.stderr.take().context("no stderr")?,
        sink.clone(),
    ));
    let status = loop {
        tokio::select! {
            status = child.wait() => break status?,
            Some(sig) = stop.recv() => {
                if let Some(pid) = child.id() {
                    // SAFETY: a pid this process spawned and has not reaped.
                    unsafe { libc::kill(pid as libc::pid_t, sig); }
                }
            }
        }
    };
    let _ = out.await;
    let _ = err.await;
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return Ok(128 + sig);
        }
    }
    Ok(status.code().unwrap_or(1))
}

/// [`run_until`] with this process's SIGTERM, SIGINT and SIGHUP passed on.
pub async fn run(spec: Spec) -> Result<i32> {
    use tokio::signal::unix::{signal, SignalKind};
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    for (kind, sig) in [
        (SignalKind::terminate(), libc::SIGTERM),
        (SignalKind::interrupt(), libc::SIGINT),
        (SignalKind::hangup(), libc::SIGHUP),
    ] {
        let mut s = signal(kind).context("installing a signal handler")?;
        let tx = tx.clone();
        tokio::spawn(async move {
            while s.recv().await.is_some() {
                if tx.send(sig).await.is_err() {
                    break;
                }
            }
        });
    }
    run_until(spec, rx).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("mecha-logcap-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A program that writes far past the cap leaves the log and one older
    /// file, neither past the cap, the newest output in the log.
    #[tokio::test]
    async fn the_log_rotates_at_its_cap() {
        let d = dir();
        let log = d.join("x.log");
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let code = run_until(
            Spec {
                log: log.clone(),
                max_bytes: 4096,
                program: vec![
                    "/bin/sh".into(),
                    "-c".into(),
                    "i=0; while [ $i -lt 2000 ]; do echo line $i; echo err $i >&2; i=$((i+1)); done; echo last"
                        .into(),
                ],
            },
            rx,
        )
        .await
        .unwrap();
        assert_eq!(code, 0);
        let now = std::fs::metadata(&log).unwrap().len();
        let older = std::fs::metadata(d.join("x.log.1")).unwrap().len();
        assert!(now <= 4096 && older <= 4096, "{now} {older}");
        assert!(std::fs::read_to_string(&log).unwrap().contains("last"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The program's exit code is this process's, and a stop is passed on.
    #[tokio::test]
    async fn the_exit_and_a_stop_are_the_program_s() {
        let d = dir();
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let code = run_until(
            Spec {
                log: d.join("a.log"),
                max_bytes: 1 << 20,
                program: vec!["/bin/sh".into(), "-c".into(), "exit 3".into()],
            },
            rx,
        )
        .await
        .unwrap();
        assert_eq!(code, 3);

        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let run = tokio::spawn(run_until(
            Spec {
                log: d.join("b.log"),
                max_bytes: 1 << 20,
                program: vec!["/bin/sleep".into(), "30".into()],
            },
            rx,
        ));
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        tx.send(libc::SIGTERM).await.unwrap();
        let code = tokio::time::timeout(std::time::Duration::from_secs(5), run)
            .await
            .expect("the stop reached the program")
            .unwrap()
            .unwrap();
        assert_eq!(code, 128 + libc::SIGTERM);
        let _ = std::fs::remove_dir_all(&d);
    }
}
