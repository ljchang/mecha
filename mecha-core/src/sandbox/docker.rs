//! A Docker MCP server is owned by container identity, not the attach CLI.
//! Create completes before start is possible. The creation worker retains
//! ownership if its caller is cancelled, and cleanup does not need Tokio to
//! stay alive (clients are also dropped during runtime shutdown).

use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone)]
struct Control {
    program: OsString,
    env: Vec<(OsString, Option<OsString>)>,
    cwd: PathBuf,
}

impl Control {
    fn pin_context(&mut self) -> Result<()> {
        if self.env.iter().any(|(key, value)| {
            (key == "DOCKER_HOST" || key == "DOCKER_CONTEXT")
                && value.as_ref().is_some_and(|v| !v.is_empty())
        }) {
            return Ok(());
        }
        // `docker context use` can change the default while a server lives.
        // Resolve it once so cleanup cannot quietly target another daemon.
        let context = self.run(
            &["context".into(), "show".into()],
            Duration::from_secs(10),
            true,
        )?;
        let context = std::str::from_utf8(&context)?.trim();
        if context.is_empty() {
            bail!("Docker returned no active context");
        }
        self.env
            .push(("DOCKER_CONTEXT".into(), Some(context.into())));
        Ok(())
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.current_dir(&self.cwd).env_clear();
        for (key, value) in &self.env {
            match value {
                Some(value) => {
                    command.env(key, value);
                }
                None => {
                    command.env_remove(key);
                }
            }
        }
        command
    }

    // Control commands do not carry MCP traffic. Captured output is one
    // context name or a listing filtered to our unique container name.
    fn run(&self, args: &[OsString], timeout: Duration, capture: bool) -> Result<Vec<u8>> {
        let mut command = self.command();
        command
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::piped());
        command.stdout(if capture {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        let child = command.spawn().context("starting Docker control command")?;
        Self::wait(child, timeout)
    }

    fn wait(mut child: Child, timeout: Duration) -> Result<Vec<u8>> {
        let start = Instant::now();
        // Docker may emit pull progress exceeding a pipe's capacity. Drain
        // continuously, retaining only the diagnostic tail; waiting for exit
        // before reading can deadlock an otherwise successful creation.
        let stderr = if let Some(mut pipe) = child.stderr.take() {
            let (tx, rx) = std::sync::mpsc::channel();
            if let Err(e) = std::thread::Builder::new()
                .name("mecha-docker-stderr".into())
                .spawn(move || {
                    let mut tail = Vec::new();
                    let mut buf = [0; 4096];
                    while let Ok(n) = pipe.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        tail.extend_from_slice(&buf[..n]);
                        if tail.len() > 4096 {
                            tail.drain(..tail.len() - 4096);
                        }
                    }
                    let _ = tx.send(tail);
                })
            {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e).context("starting Docker diagnostic reader");
            }
            Some(rx)
        } else {
            None
        };
        loop {
            let status = match child.try_wait() {
                Ok(status) => status,
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(e.into());
                }
            };
            if let Some(status) = status {
                if !status.success() {
                    // Bound the wait even if a helper inherited stderr.
                    let tail = stderr
                        .and_then(|rx| rx.recv_timeout(Duration::from_millis(100)).ok())
                        .unwrap_or_default();
                    bail!(
                        "Docker control command exited with {status}: {}",
                        String::from_utf8_lossy(&tail).trim()
                    );
                }
                let mut bytes = Vec::new();
                if let Some(stdout) = child.stdout.take() {
                    stdout.take(4096).read_to_end(&mut bytes)?;
                }
                return Ok(bytes);
            }
            if start.elapsed() >= timeout {
                let _ = child.kill();
                let _ = child.wait();
                bail!("Docker control command timed out");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn remove(&self, name: &str) -> Result<()> {
        let args = ["rm", "--force", "--volumes", name].map(OsString::from);
        let mut failure = None;
        for _ in 0..3 {
            match self.run(&args, Duration::from_secs(10), false) {
                Ok(_) => return Ok(()),
                Err(e) => failure = Some(e),
            }
            // --rm may already have removed a normally exiting server.
            // A failed inspect could also mean an unavailable daemon; a
            // successful listing with no match is the proof of absence.
            let query = [
                "ps",
                "--all",
                "--quiet",
                "--no-trunc",
                "--filter",
                &format!("name=^/{name}$"),
            ]
            .map(OsString::from);
            if self
                .run(&query, Duration::from_secs(10), true)
                .is_ok_and(|out| out.is_empty())
            {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(failure.expect("removal attempted"))
    }
}

pub(crate) struct DockerContainer {
    name: String,
    control: Control,
}

impl DockerContainer {
    /// Consume the already-confined `docker run` command so create, attach,
    /// and removal all use the same executable, environment and daemon.
    pub(crate) async fn create(run: tokio::process::Command) -> Result<Self> {
        let command = run.as_std();
        let mut args: Vec<OsString> = command.get_args().map(OsString::from).collect();
        if args.first().is_none_or(|arg| arg != "run") {
            bail!("expected a Docker run command");
        }
        let mut owned = Self {
            name: format!("mecha-mcp-{}", uuid::Uuid::new_v4()),
            control: Control {
                program: command.get_program().to_owned(),
                env: command
                    .get_envs()
                    .map(|(k, v)| (k.to_owned(), v.map(OsString::from)))
                    .collect(),
                cwd: match command.get_current_dir() {
                    Some(cwd) => cwd.to_owned(),
                    None => std::env::current_dir()?,
                },
            },
        };
        args[0] = "create".into();
        args.splice(
            1..1,
            [OsString::from("--name"), OsString::from(&owned.name)],
        );
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("mecha-docker-create".into())
            .spawn(move || {
                let result = (|| {
                    owned.control.pin_context()?;
                    owned
                        .control
                        .run(&args, Duration::from_secs(120), false)
                        .context(
                            "creating MCP container; check the Docker daemon and sandbox image",
                        )?;
                    Ok(owned)
                })();
                // If the future was cancelled, SendError drops the container
                // only after creation finishes. No server has started yet.
                let _ = tx.send(result);
            })
            .context("starting Docker creation worker")?;
        rx.await.context("Docker creation worker stopped")?
    }

    pub(crate) fn start_command(&self) -> tokio::process::Command {
        let mut command = self.control.command();
        command.args(["start", "--attach", "--interactive", &self.name]);
        command.into()
    }
}

impl Drop for DockerContainer {
    fn drop(&mut self) {
        let control = self.control.clone();
        let name = self.name.clone();
        // Launch before returning: an ordinary CLI may exit immediately
        // after dropping its clients, before a new thread gets scheduled.
        // The removal process can finish even after the harness exits.
        let cleanup = control
            .command()
            .args(["rm", "--force", "--volumes", &name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        // While the harness lives, a worker reaps the CLI and retries a
        // transient failure without blocking an async runtime worker.
        if let Err(e) = std::thread::Builder::new().name("mecha-docker-remove".into()).spawn(move || {
            if let Ok(child) = cleanup {
                if Control::wait(child, Duration::from_secs(10)).is_ok() {
                    return;
                }
            }
            if let Err(e) = control.remove(&name) {
                tracing::error!(container = %name, error = %e, "MCP container cleanup failed; remove it once Docker is available");
            }
        }) {
            tracing::error!(container = %self.name, error = %e, "could not start MCP container cleanup");
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn control_errors_keep_a_bounded_diagnostic_without_blocking_on_progress() {
        let control = Control {
            program: "/bin/sh".into(),
            env: vec![],
            cwd: std::env::temp_dir(),
        };
        let error = control.run(
            &["-c".into(), "i=0; while [ $i -lt 10000 ]; do printf 'pull progress........'; i=$((i+1)); done >&2; printf 'Cannot connect to the Docker daemon' >&2; exit 125".into()],
            Duration::from_secs(5), false,
        ).unwrap_err().to_string();
        assert!(
            error.contains("Cannot connect to the Docker daemon"),
            "{error}"
        );
        assert!(error.len() < 5000, "diagnostics must stay bounded");
    }

    fn fixture() -> (PathBuf, tokio::process::Command) {
        let dir =
            std::env::temp_dir().join(format!("mecha-docker-control-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let script = dir.join("docker");
        std::fs::write(
            &script,
            r#"#!/bin/sh
set -eu
case "$1" in
context) echo fixture-context ;;
create)
  test "$DOCKER_CONTEXT" = fixture-context
  touch "$PROBE_DIR/starting"
  while ! test -f "$PROBE_DIR/release"; do sleep 0.01; done
  touch "$PROBE_DIR/created"
  ;;
rm)
  test "$DOCKER_CONTEXT" = fixture-context
  rm -f "$PROBE_DIR/created"
  touch "$PROBE_DIR/removed"
  ;;
ps)
  if test -f "$PROBE_DIR/created"; then echo container; fi
  ;;
start) touch "$PROBE_DIR/server-started" ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut command = tokio::process::Command::new(script);
        command
            .arg("run")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("PROBE_DIR", &dir);
        (dir, command)
    }

    async fn wait_for(path: &std::path::Path) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !path.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn cancellation_during_creation_cleans_the_late_result_without_starting_it() {
        let (dir, command) = fixture();
        let connecting = tokio::spawn(DockerContainer::create(command));
        wait_for(&dir.join("starting")).await;
        connecting.abort();
        assert!(matches!(connecting.await, Err(e) if e.is_cancelled()));
        // Complete creation *after* the connect future is gone. Cleanup
        // performed at cancellation time alone would miss this container.
        std::fs::write(dir.join("release"), "").unwrap();
        wait_for(&dir.join("removed")).await;
        assert!(!dir.join("created").exists());
        assert!(!dir.join("server-started").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cleanup_still_runs_after_the_tokio_runtime_is_gone() {
        let (dir, command) = fixture();
        std::fs::write(dir.join("release"), "").unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let container = runtime.block_on(DockerContainer::create(command)).unwrap();
        drop(runtime);
        drop(container);
        let start = Instant::now();
        while !dir.join("removed").exists() && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(dir.join("removed").exists());
        assert!(!dir.join("created").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
