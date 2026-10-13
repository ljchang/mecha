//! A server started by its first request and stopped when idle — what a
//! systemd socket, `systemd-socket-proxyd --exit-idle-time` and a
//! `StopWhenUnneeded=` unit do together on Linux, as one small process for a
//! system without them (`docs/FEATURES-DESIGN.md` §12).
//!
//! **The port is always held.** This process binds the public port from the
//! moment launchd starts it, so a request after a reboot or an idle stop waits
//! through a cold start instead of being refused — idle-stopped must never
//! look like absent (the 2026-08-19 incident `llama-embed.service` records).
//! It costs a listening socket and a few megabytes; the model costs nothing
//! until it is asked for.
//!
//! **One start, however many first requests.** Connections that arrive while
//! the server is starting wait on the same start; a server that died is
//! started again by the next connection.
//!
//! **Ready means `/health` says so**, as `mecha-wait-healthy` decides it for
//! the systemd unit: llama-server answers 503 while its model loads, and a
//! connection forwarded before then would get that 503.
//!
//! **Idle is no open connection for the whole idle time**, as the proxy's
//! `--exit-idle-time` counts it: a long request holds the server however long
//! it runs. The server is then stopped with SIGINT, as the unit's
//! `KillSignal=SIGINT` stops it, and killed if it has not exited in ten
//! seconds.

use anyhow::{bail, Context, Result};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

/// What the proxy runs, and where.
#[derive(Debug, Clone)]
pub struct Spec {
    /// The public address it holds.
    pub listen: SocketAddr,
    /// The loopback port the server is started on.
    pub backend: u16,
    /// How long with no open connection before the server is stopped.
    pub idle: Duration,
    /// How long a start may take before it is given up.
    pub start_timeout: Duration,
    /// The launcher, which starts the server on `backend`.
    pub launcher: PathBuf,
}

struct State {
    child: Option<Child>,
    /// When the last connection closed (or the server started).
    idle_since: Instant,
}

/// Run until killed: hold the port, start the server on demand, stop it when
/// idle.
pub async fn run(spec: Spec) -> Result<()> {
    let listener = TcpListener::bind(spec.listen)
        .await
        .with_context(|| format!("holding {}", spec.listen))?;
    eprintln!(
        "holding {}; the server starts on its first request (backend :{}), and stops after {} s idle",
        spec.listen,
        spec.backend,
        spec.idle.as_secs()
    );
    serve(listener, spec).await
}

async fn serve(listener: TcpListener, spec: Spec) -> Result<()> {
    let spec = Arc::new(spec);
    let state = Arc::new(Mutex::new(State {
        child: None,
        idle_since: Instant::now(),
    }));
    let open = Arc::new(AtomicUsize::new(0));
    // At least a second: `interval` panics on zero, and `--idle-secs 0`
    // would otherwise take the proxy down.
    let mut tick = tokio::time::interval(
        Duration::from_secs(5)
            .min(spec.idle)
            .max(Duration::from_secs(1)),
    );
    let mut term = signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut int = signal(tokio::signal::unix::SignalKind::interrupt())?;
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (conn, _) = accepted.context("accepting a connection")?;
                open.fetch_add(1, Ordering::SeqCst);
                let (spec, state, open) = (spec.clone(), state.clone(), open.clone());
                tokio::spawn(async move {
                    if let Err(e) = forward(conn, &spec, &state).await {
                        eprintln!("{e:#}");
                    }
                    // The idle clock starts when the last connection closes.
                    let mut s = state.lock().await;
                    if open.fetch_sub(1, Ordering::SeqCst) == 1 {
                        s.idle_since = Instant::now();
                    }
                });
            }
            _ = tick.tick() => {
                let mut s = state.lock().await;
                if open.load(Ordering::SeqCst) == 0
                    && s.child.is_some()
                    && s.idle_since.elapsed() >= spec.idle
                {
                    eprintln!("idle for {} s: stopping the server", spec.idle.as_secs());
                    if let Some(child) = s.child.take() {
                        stop(child).await;
                    }
                }
            }
            // Stopped by launchd (`bootout` sends SIGTERM): the server goes
            // with the proxy, never left running on a port nobody holds.
            _ = term.recv() => break,
            _ = int.recv() => break,
        }
    }
    if let Some(child) = state.lock().await.child.take() {
        stop(child).await;
    }
    Ok(())
}

fn signal(kind: tokio::signal::unix::SignalKind) -> Result<tokio::signal::unix::Signal> {
    tokio::signal::unix::signal(kind).context("installing a signal handler")
}

/// Start the server if it is not running, then pipe this connection to it.
async fn forward(mut conn: TcpStream, spec: &Spec, state: &Mutex<State>) -> Result<()> {
    {
        // Held across the start, so every connection that arrives meanwhile
        // waits on this one start rather than beginning its own.
        let mut s = state.lock().await;
        let running = match s.child.as_mut() {
            Some(c) => c.try_wait().ok().flatten().is_none(),
            None => false,
        };
        if !running {
            if let Some(status) = s.child.as_mut().and_then(|c| c.try_wait().ok().flatten()) {
                eprintln!("the server exited ({status}); starting it again");
            }
            s.child = Some(start(spec).await?);
            s.idle_since = Instant::now();
        }
    }
    let mut backend = TcpStream::connect(("127.0.0.1", spec.backend))
        .await
        .with_context(|| format!("connecting to the server on :{}", spec.backend))?;
    // A closed half on either side is the request ending, not a failure.
    let _ = tokio::io::copy_bidirectional(&mut conn, &mut backend).await;
    Ok(())
}

/// Start the launcher and wait for `/health` to say ok. A start that fails or
/// times out is stopped, never left half up.
async fn start(spec: &Spec) -> Result<Child> {
    eprintln!("starting {}", spec.launcher.display());
    let mut child = Command::new(&spec.launcher)
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("starting {}", spec.launcher.display()))?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let url = format!("http://127.0.0.1:{}/health", spec.backend);
    let began = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            bail!(
                "{} exited ({status}) before it was ready",
                spec.launcher.display()
            );
        }
        if client
            .get(&url)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            eprintln!("ready after {:.1} s", began.elapsed().as_secs_f64());
            return Ok(child);
        }
        if began.elapsed() > spec.start_timeout {
            stop(child).await;
            bail!(
                "{} did not answer /health within {} s",
                spec.launcher.display(),
                spec.start_timeout.as_secs()
            );
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// SIGINT, ten seconds to exit, then SIGKILL.
async fn stop(mut child: Child) {
    if let Some(pid) = child.id() {
        // SAFETY: `kill` with a pid this process spawned and has not reaped.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGINT);
        }
    }
    if tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A launcher that serves `/health` and an answer on the backend port —
    /// a stand-in llama-server, in Python, which every CI image has.
    fn fake_server(dir: &std::path::Path, backend: u16) -> PathBuf {
        let server = dir.join("server.py");
        std::fs::write(
            &server,
            format!(
                r#"import http.server
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"ok")
    def log_message(self, *a):
        pass
http.server.HTTPServer(("127.0.0.1", {backend}), H).serve_forever()
"#
            ),
        )
        .unwrap();
        let script = dir.join("fake-server");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho started >> '{}'\nexec python3 '{}' 2>> '{}'\n",
                dir.join("starts").display(),
                server.display(),
                dir.join("server.err").display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    /// A free port below every system's ephemeral range (Linux's starts at
    /// 32768, macOS's at 49152). A port the kernel handed out and took back
    /// is the next one it hands out: the test client's own outgoing
    /// connection took it on macOS, and the stand-in could not bind (macOS
    /// CI on #678).
    fn free_port() -> u16 {
        let seed = std::process::id() as u16 ^ (uuid::Uuid::new_v4().as_u128() as u16);
        (0..2000u16)
            .map(|i| 20_000 + (seed.wrapping_add(i * 7) % 10_000))
            .find(|p| std::net::TcpListener::bind(("127.0.0.1", *p)).is_ok())
            .expect("a free port in 20000..30000")
    }

    fn starts(dir: &std::path::Path) -> usize {
        std::fs::read_to_string(dir.join("starts"))
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }

    /// Nothing runs until asked; concurrent first requests share one start;
    /// the server stops when idle and starts again on the next request.
    #[tokio::test]
    async fn the_server_starts_on_demand_and_stops_when_idle() {
        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            // A skip reads as a pass in CI: refused where backends are
            // required (CLAUDE.md, testing without credentials).
            assert!(
                std::env::var_os("MECHA_TEST_REQUIRE_BACKENDS").is_none(),
                "no python3, and MECHA_TEST_REQUIRE_BACKENDS is set"
            );
            eprintln!("no python3: skipped");
            return;
        }
        let dir = std::env::temp_dir().join(format!("mecha-ondemand-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        // The public port first, held while the backend's is chosen: a port
        // chosen and let go, then a second bind, can be handed the same
        // number — the stand-in then cannot bind its own (found on macOS CI,
        // #679, and reproduced here).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let public = listener.local_addr().unwrap();
        let backend = loop {
            let p = free_port();
            if p != public.port() {
                break p;
            }
        };
        let spec = Spec {
            listen: public,
            backend,
            idle: Duration::from_secs(2),
            start_timeout: Duration::from_secs(30),
            launcher: fake_server(&dir, backend),
        };
        let proxy = tokio::spawn(serve(listener, spec));
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(starts(&dir), 0, "nothing starts before a request");

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("http://{public}/health");
        let (a, b, c) = tokio::join!(
            client.get(&url).send(),
            client.get(&url).send(),
            client.get(&url).send()
        );
        for r in [a, b, c] {
            // The stand-in's own error output, when it never answered: a
            // failure here is otherwise a bare connection reset (macOS CI on
            // #679).
            let r = r.unwrap_or_else(|e| {
                panic!(
                    "{e:#}\nthe stand-in server said:\n{}",
                    std::fs::read_to_string(dir.join("server.err")).unwrap_or_default()
                )
            });
            assert_eq!(r.text().await.unwrap(), "ok");
        }
        assert_eq!(starts(&dir), 1, "three first requests, one start");

        // Idle past the window: the backend port is let go.
        let mut gone = false;
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if TcpStream::connect(("127.0.0.1", backend)).await.is_err() {
                gone = true;
                break;
            }
        }
        assert!(gone, "the server was not stopped after the idle window");

        // And the next request starts it again.
        assert_eq!(
            client.get(&url).send().await.unwrap().text().await.unwrap(),
            "ok"
        );
        assert_eq!(starts(&dir), 2);
        proxy.abort();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
