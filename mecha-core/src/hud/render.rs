//! A dashboard drawn headless, for the model to look at
//! (`docs/LIVE-DASHBOARD-DESIGN.md` §5.3, §8 step 6).
//!
//! **The render reaches nothing, by the kernel's say-so.** `hud_preview`
//! declares `Egress::None`, and that rests on this render: the page is the
//! owner's renderer drawing a model-drafted spec over the owner's rows, and a
//! spec that slipped a destination past the walker must still have nowhere
//! to send it. So the browser runs inside `bwrap --unshare-all`: no network
//! namespace but its own loopback, and of this machine's files only four
//! binds — the browser's own directory (Playwright's, under the home, when
//! that is where it is), the bundle read-only and one output directory (both
//! in the store's scratch space), and this binary — beside the read-only
//! system. Nothing else of the home, so no `~/.mecha`, no keys, no
//! transcripts. Inside, `mecha hud render-inside`
//! serves the bundle on that loopback under the interactive class's CSP and
//! runs the browser at it — the factory's policy, so what renders here is
//! what a published page renders. Chromium's own sandbox is off inside
//! (`--no-sandbox`), because this machine's AppArmor refuses it the user
//! namespaces it needs; bwrap is the boundary instead, and it is the one
//! this machine's shell sandbox already trusts.
//!
//! **No browser, no bwrap: no picture, said.** Never a fallback to an
//! unconfined browser — a silently-degrading guard is worse than none.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use super::publish::interactive_csp;

/// The viewport the page is drawn in: desktop width, and tall enough for a
/// board of a dozen panels. The blank below the last panel is cropped.
pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 2400;

/// How long the browser may take, start to screenshot, before it is killed.
pub const TIMEOUT: Duration = Duration::from_secs(45);

/// The headless browser this machine has: a `chrome-headless-shell` binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Browser {
    /// The binary itself; its directory is what the sandbox binds.
    pub binary: PathBuf,
}

impl Browser {
    /// `chrome-headless-shell` on `PATH`, else the newest one Playwright
    /// installed under `~/.cache/ms-playwright`. A full Chrome is not taken:
    /// its sandbox and profile machinery want more of the machine than the
    /// render's sandbox gives it.
    pub fn find() -> Option<Browser> {
        if let Some(path) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path) {
                let candidate = dir.join("chrome-headless-shell");
                if candidate.is_file() {
                    return Some(Browser { binary: candidate });
                }
            }
        }
        let cache = dirs_home()?.join(".cache").join("ms-playwright");
        let mut found: Vec<PathBuf> = std::fs::read_dir(&cache)
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("chromium_headless_shell-"))
            })
            .filter_map(|dir| {
                std::fs::read_dir(&dir)
                    .ok()?
                    .filter_map(|e| e.ok())
                    .find_map(|e| {
                        let bin = e.path().join("chrome-headless-shell");
                        bin.is_file().then_some(bin)
                    })
            })
            .collect();
        // The newest revision sorts last (`chromium_headless_shell-1243`).
        found.sort();
        found.pop().map(|binary| Browser { binary })
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// What a render needs from outside the sandbox.
#[derive(Debug, Clone)]
pub struct Renderer {
    pub browser: Browser,
    /// The `bwrap` the shell sandbox runs (`PATH`'s, which this machine's
    /// units put `/usr/local/bin` first on).
    pub bwrap: PathBuf,
    /// This binary, which serves the bundle inside the sandbox.
    pub mecha: PathBuf,
    /// An IANA zone for the browser's clock (`TZ`), or UTC.
    pub timezone: Option<String>,
}

impl Renderer {
    /// Everything found, or why not — the reason is what the model is told.
    pub fn find() -> std::result::Result<Renderer, String> {
        let browser = Browser::find().ok_or_else(|| {
            "no headless browser on this machine (`chrome-headless-shell` on PATH, or \
             Playwright's under ~/.cache/ms-playwright)"
                .to_string()
        })?;
        let bwrap = which("bwrap").ok_or_else(|| {
            "no `bwrap` on PATH, and the render never runs a browser unconfined".to_string()
        })?;
        let mecha = std::env::current_exe()
            .map_err(|e| format!("this binary's own path could not be read: {e}"))?;
        Ok(Renderer {
            browser,
            bwrap,
            mecha,
            timezone: None,
        })
    }

    /// Draw the bundle in `bundle` and return the screenshot, cropped to the
    /// page's content. `scratch` is a directory the caller owns and removes.
    pub fn render(&self, bundle: &Path, scratch: &Path) -> Result<Cropped> {
        let out = scratch.join("out");
        crate::create_private_dir(&out)?;
        let browser_dir = self
            .browser
            .binary
            .parent()
            .context("the browser binary has a directory")?;
        let mut cmd = Command::new(&self.bwrap);
        cmd.args(sandbox_args(
            browser_dir,
            bundle,
            &out,
            &self.mecha,
            self.timezone.as_deref(),
        ))
        .args([
            "/mecha",
            "hud",
            "render-inside",
            "--bundle",
            "/bundle",
            "--out",
            "/out/shot.png",
            "--browser",
        ])
        .arg(
            Path::new("/browser").join(
                self.browser
                    .binary
                    .file_name()
                    .context("the browser binary has a name")?,
            ),
        )
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
        let mut child = cmd.spawn().context("starting the render's sandbox")?;
        // Drained while it runs: a chatty browser filling an unread pipe
        // blocks, and a working render would read as a timeout (review of
        // #662).
        let stderr = child.stderr.take().map(|mut e| {
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = e.read_to_end(&mut buf);
                let keep = buf.len().saturating_sub(16 * 1024);
                String::from_utf8_lossy(&buf[keep..]).into_owned()
            })
        });
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if started.elapsed() > TIMEOUT {
                let _ = child.kill();
                let _ = child.wait();
                bail!("the render took longer than {}s", TIMEOUT.as_secs());
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        let err = stderr.and_then(|t| t.join().ok()).unwrap_or_default();
        let shot = out.join("shot.png");
        if !status.success() || !shot.is_file() {
            let tail: String = err
                .lines()
                .rfind(|l| !l.trim().is_empty())
                .unwrap_or("no message")
                .chars()
                .take(300)
                .collect();
            bail!("the render failed ({status}): {tail}");
        }
        let png = std::fs::read(&shot)?;
        crop(&png)
    }
}

/// `bwrap`'s arguments: every namespace unshared (the network's included —
/// only the sandbox's own loopback remains), the system read-only, the
/// browser, the bundle and this binary read-only, and one writable output
/// directory. Nothing of the home is bound.
pub fn sandbox_args(
    browser_dir: &Path,
    bundle: &Path,
    out: &Path,
    mecha: &Path,
    timezone: Option<&str>,
) -> Vec<String> {
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let mut a: Vec<String> = [
        "--unshare-all",
        // In the argv, not left to the caller's `env_clear()`, as
        // `Sandbox::bwrap_args` does: keys live in the environment, and a
        // test over these arguments must be able to see that none is
        // inherited (review of #662).
        "--clearenv",
        "--die-with-parent",
        "--new-session",
        "--ro-bind",
        "/usr",
        "/usr",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--tmpfs",
        "/dev/shm",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    // The top-level system directories as this machine has them — each a
    // symlink into /usr on a merged-/usr distribution, or a real directory —
    // asked rather than assumed, as `Sandbox::bwrap_args` asks: x86-64's ELF
    // interpreter is in /lib64, and a sandbox without it cannot exec this
    // binary at all (review of #662).
    for dir in ["/bin", "/sbin", "/lib", "/lib32", "/lib64"] {
        match std::fs::symlink_metadata(dir) {
            Ok(meta) if meta.file_type().is_symlink() => {
                if let Ok(target) = std::fs::read_link(dir) {
                    a.extend(["--symlink".into(), s(&target), dir.into()]);
                }
            }
            Ok(_) => a.extend(["--ro-bind".into(), dir.into(), dir.into()]),
            Err(_) => {}
        }
    }
    // Fonts, so text renders as it does for the owner; skipped if absent.
    if Path::new("/etc/fonts").exists() {
        a.extend(["--ro-bind".into(), "/etc/fonts".into(), "/etc/fonts".into()]);
    }
    a.extend([
        "--ro-bind".into(),
        s(browser_dir),
        "/browser".into(),
        "--ro-bind".into(),
        s(bundle),
        "/bundle".into(),
        "--ro-bind".into(),
        s(mecha),
        "/mecha".into(),
        "--bind".into(),
        s(out),
        "/out".into(),
        "--setenv".into(),
        "PATH".into(),
        "/usr/bin:/bin".into(),
        "--setenv".into(),
        "HOME".into(),
        "/tmp".into(),
        "--setenv".into(),
        "MECHA_HOME".into(),
        "/tmp/mecha".into(),
    ]);
    // Only a zone name of the IANA shape: it is passed to the browser's
    // clock and nothing else (zoneinfo is under /usr, which is bound).
    if let Some(tz) = timezone.filter(|t| {
        !t.is_empty()
            && t.chars()
                .all(|c| c.is_ascii_alphanumeric() || "/_-+".contains(c))
    }) {
        a.extend(["--setenv".into(), "TZ".into(), tz.into()]);
    }
    a.push("--".into());
    a
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// Cut the empty background below the last panel, keeping a margin. The
/// background is the bottom-left pixel; a row is content if any pixel in it
/// differs from that.
pub fn crop(png: &[u8]) -> Result<Cropped> {
    let img = image::load_from_memory(png)
        .context("the screenshot is not a picture")?
        .to_rgba8();
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        bail!("the screenshot is empty");
    }
    let bg = *img.get_pixel(0, h - 1);
    // A page with no row unlike its background drew nothing — a style the
    // CSP refused, a dataset that did not load — and is an error, never
    // "the dashboard as drawn" (review of #662).
    let Some(last) = (0..h)
        .rev()
        .find(|&y| (0..w).any(|x| *img.get_pixel(x, y) != bg))
    else {
        bail!("the page drew nothing: the render is blank");
    };
    let keep = (last + 24).min(h);
    let cropped = image::imageops::crop_imm(&img, 0, 0, w, keep).to_image();
    let mut out = Vec::new();
    cropped
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .context("encoding the cropped screenshot")?;
    // Content in the last rows: the page ran past the viewport, and what is
    // below it is not in the picture (review of #662).
    Ok(Cropped {
        png: out,
        cut: last + 1 >= h,
        width: w,
        height: keep,
    })
}

/// A screenshot cropped to its content, and whether the page ran past the
/// viewport — panels below it are not in the picture, and the model is told.
#[derive(Debug, Clone)]
pub struct Cropped {
    pub png: Vec<u8>,
    pub cut: bool,
    pub width: u32,
    pub height: u32,
}

impl Cropped {
    /// The width the model will see once the picture is capped at the door
    /// (`image::MAX_EDGE`): a board taller than the cap is scaled down whole,
    /// and small labels with it (review of #662). `None` when it fits.
    pub fn scaled_width(&self) -> Option<u32> {
        let edge = crate::image::MAX_EDGE;
        let longest = self.width.max(self.height);
        (longest > edge).then(|| (self.width as u64 * edge as u64 / longest as u64) as u32)
    }
}

// ---- inside the sandbox -------------------------------------------------

/// Serve `bundle` on this namespace's loopback under the interactive CSP,
/// point `browser` at it, and write its screenshot to `out`. Runs inside the
/// sandbox [`Renderer::render`] builds; outside it, the browser would have
/// the machine's network, so the caller is the sandbox, never a person.
pub fn serve_and_shoot(bundle: &Path, out: &Path, browser: &Path) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").context("binding loopback")?;
    let port = listener.local_addr()?.port();
    let root = bundle
        .canonicalize()
        .with_context(|| format!("{} is not readable", bundle.display()))?;
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let root = root.clone();
            std::thread::spawn(move || {
                let _ = answer(stream, &root);
            });
        }
    });
    let status = Command::new(browser)
        .args([
            "--headless",
            "--no-sandbox",
            "--disable-gpu",
            "--hide-scrollbars",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--mute-audio",
        ])
        .arg(format!("--window-size={WIDTH},{HEIGHT}"))
        // Advances the page's clock past Vega's async render without
        // waiting for it in real time.
        .arg("--virtual-time-budget=15000")
        .arg(format!("--screenshot={}", out.display()))
        .arg(format!("http://127.0.0.1:{port}/index.html"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .context("starting the browser")?;
    if !status.success() || !out.is_file() {
        bail!("the browser did not write a screenshot ({status})");
    }
    Ok(())
}

/// One request: a `GET` for a file under `root`, nothing else. The path is
/// proved inside `root` after canonicalising, so `..` reaches nothing.
fn answer(mut stream: TcpStream, root: &Path) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    // Drain the headers; nothing in them is used.
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
    }
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let path = target.split(['?', '#']).next().unwrap_or("/");
    let rel = path.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    let file = root
        .join(rel)
        .canonicalize()
        .ok()
        .filter(|p| p.starts_with(root));
    let (status, body, kind) = match (method, file) {
        ("GET", Some(f)) if f.is_file() => ("200 OK", std::fs::read(&f)?, content_type(&f)),
        ("GET", _) => ("404 Not Found", b"not found\n".to_vec(), "text/plain"),
        _ => ("405 Method Not Allowed", Vec::new(), "text/plain"),
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
         Content-Security-Policy: {}\r\nX-Content-Type-Options: nosniff\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len(),
        interactive_csp(None),
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(&body)?;
    stream.flush()
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
