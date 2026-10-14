//! The on-demand embeddings and OCR servers, installed from nothing
//! (`docs/FEATURES-DESIGN.md` §10.2 item 2, step 7c-1).
//!
//! **What runs is what mecha wrote, never a working tree.** The units and
//! launchers ship inside the binary — copies of `scripts/llama/`, held byte
//! for byte to them by a test, as layout's lock is — and are written into the
//! user unit directory and `~/.mecha/sidecars/bin/`, so a checkout switching
//! branches changes nothing that runs (the 2026-08-20 rule
//! `scripts/llama/install.sh` already keeps).
//!
//! **Each server is three units and a launcher**, as on the machine they were
//! written for: a socket holds the public port from boot, a
//! `systemd-socket-proxyd` service forwards it and stops after ten idle
//! minutes, and the server itself starts on a private backend port, its unit
//! waiting until `/health` answers. A first request is a cold start, so the
//! install's own check outwaits one (LLAMA-SERVER.md §Document OCR).
//!
//! **Rendered, not copied verbatim.** The installed units name the launchers
//! by absolute path under this mecha home (`MECHA_HOME` honoured), name
//! `systemd-socket-proxyd` where this distribution keeps it, and carry two
//! drop-ins: the hub directory the model was fetched into (`mecha-hub.conf`,
//! so the launcher, under the user manager's environment, reads the same
//! cache), and the engine. mecha's own engine is named by `mecha-engine.conf`,
//! recorded under the engine as `setup engine --adopt` records it; a provided
//! `llama-server` — found on the PATH mecha ran with, which can be wider than
//! the unit's fixed one — is named by `engine-provided.conf`, which sorts
//! before it, so a later adopt's drop-in wins and its rollback never touches
//! this one. The unit names and ports are parameters, so a test can install
//! a pair beside the live one without touching it.
//!
//! **On macOS, one launchd agent per server** (§12): launchd can hold a
//! socket, but llama-server cannot take one handed to it, so the agent runs
//! `mecha features on-demand` (`on_demand.rs`) — which holds the public port,
//! starts the same launcher on the backend port at the first request, and
//! stops it after the same ten idle minutes. The launcher, the model and the
//! check are shared; the plist carries what the drop-ins carry.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::install::Say;
use crate::sidecar::{Machinery, Manifest};

const EMBED_SOCKET: &str = include_str!("../units/llama-embed.socket");
const EMBED_PROXY: &str = include_str!("../units/llama-embed-proxy.service");
const EMBED_SERVICE: &str = include_str!("../units/llama-embed.service");
const EMBED_LAUNCHER: &str = include_str!("../units/mecha-embed-server");
const OCR_SOCKET: &str = include_str!("../units/llama-ocr.socket");
const OCR_PROXY: &str = include_str!("../units/llama-ocr-proxy.service");
const OCR_SERVICE: &str = include_str!("../units/llama-ocr.service");
const OCR_LAUNCHER: &str = include_str!("../units/mecha-ocr-server");
pub(crate) const WAIT_HEALTHY: &str = include_str!("../units/mecha-wait-healthy");
pub(crate) const PATH_CONF: &str = include_str!("../units/path.conf");

/// One on-demand server, as its shipped files name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Embeddings,
    Ocr,
}

impl Which {
    /// The sidecar id `sidecar::SIDECARS` knows it by.
    pub fn sidecar(self) -> &'static str {
        match self {
            Which::Embeddings => "embed-server",
            Which::Ocr => "ocr-server",
        }
    }

    /// The recommendation slot whose pinned model it serves.
    fn slot(self) -> &'static str {
        match self {
            Which::Embeddings => "embeddings",
            Which::Ocr => "ocr",
        }
    }

    /// The names and ports the shipped files carry.
    pub fn shipped(self) -> Naming {
        match self {
            Which::Embeddings => Naming {
                stem: "llama-embed".into(),
                public: 8081,
                backend: 18081,
            },
            Which::Ocr => Naming {
                stem: "llama-ocr".into(),
                public: 8085,
                backend: 18085,
            },
        }
    }

    fn files(
        self,
    ) -> (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ) {
        match self {
            Which::Embeddings => (
                EMBED_SOCKET,
                EMBED_PROXY,
                EMBED_SERVICE,
                EMBED_LAUNCHER,
                "mecha-embed-server",
            ),
            Which::Ocr => (
                OCR_SOCKET,
                OCR_PROXY,
                OCR_SERVICE,
                OCR_LAUNCHER,
                "mecha-ocr-server",
            ),
        }
    }
}

/// The unit stem and the two ports a server is installed under: the shipped
/// ones in use, others in a test beside the live pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Naming {
    pub stem: String,
    pub public: u16,
    pub backend: u16,
}

/// Where launchers mecha writes live: in its own tree, recorded, removable.
pub fn bin_dir(mecha_home: &Path) -> PathBuf {
    mecha_home.join("sidecars").join("bin")
}

/// `systemd-socket-proxyd`, where this distribution keeps it.
fn socket_proxyd() -> Result<&'static str> {
    [
        "/usr/lib/systemd/systemd-socket-proxyd",
        "/lib/systemd/systemd-socket-proxyd",
    ]
    .into_iter()
    .find(|p| Path::new(p).is_file())
    .context(
        "systemd-socket-proxyd is not installed (it comes with systemd) — the on-demand \
         servers need it to start on their first request",
    )
}

/// One shipped file, rendered for this install: the stem and ports swapped
/// for the naming's, the launchers named under `bin`, the proxy where it is.
/// The backend port is swapped before the public one, since the shipped
/// backend (`18085`) contains the public (`8085`).
fn render(text: &str, which: Which, naming: &Naming, bin: &Path, proxyd: &str) -> String {
    let shipped = which.shipped();
    text.replace(&shipped.backend.to_string(), "\u{0}BACKEND\u{0}")
        .replace(
            &format!(":{}", shipped.public),
            &format!(":{}", naming.public),
        )
        .replace("\u{0}BACKEND\u{0}", &naming.backend.to_string())
        .replace(&shipped.stem, &naming.stem)
        .replace("%h/.local/bin/", &format!("{}/", bin.display()))
        .replace("/usr/lib/systemd/systemd-socket-proxyd", proxyd)
}

/// A file mecha writes, unless something it did not write is there already:
/// the same bytes are left as they are, different ones refuse — a file mecha
/// did not write is never overwritten (§10.2 item 4).
pub(crate) fn write_owned(home: &Path, id: &str, path: &Path, text: &str, mode: u32) -> Result<()> {
    let ours = Manifest::read(home)?
        .entries
        .iter()
        .any(|e| e.wrote.iter().any(|w| w == path));
    // A dangling link is a path someone put there, not an empty one.
    let link = std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink());
    match std::fs::read_to_string(path) {
        Ok(existing) if existing == text && !link => {}
        Ok(_) if !ours => bail!(
            "{} is there already and mecha did not write it — move it aside to install mecha's",
            path.display()
        ),
        // Unknown is never clean: bytes that are not text, a file that cannot
        // be read, a link to nothing — whose it is cannot be said, so it is
        // not replaced.
        Err(e) if !ours && (link || e.kind() != std::io::ErrorKind::NotFound) => bail!(
            "{} is there already and mecha cannot read it ({e}), so it cannot say whose it is — \
             move it aside to install mecha's",
            path.display()
        ),
        _ => {
            Manifest::record(home, id, path)?;
            std::fs::create_dir_all(path.parent().context("a path with no parent")?)?;
            let tmp = path.with_file_name(format!(
                "{}.mecha-new",
                path.file_name().and_then(|n| n.to_str()).unwrap_or("unit")
            ));
            std::fs::write(&tmp, text)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
            }
            std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
            return Ok(());
        }
    }
    // Present and identical: recorded under *this* sidecar whatever else
    // records it — a shared file (`mecha-wait-healthy`) is then kept while
    // any server that uses it remains — and given the mode it needs, since a
    // hand copy with the right bytes may not be executable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .with_context(|| format!("setting the mode of {}", path.display()))?;
    }
    Manifest::record(home, id, path)
}

/// Install one on-demand server under `naming`: its pinned model fetched, its
/// launcher and units written, the socket enabled, and one request through
/// the public port answered — a cold start, outwaited.
pub async fn install(
    m: &Machinery,
    which: Which,
    naming: &Naming,
    machine: &crate::recommend::Machine,
    hub: &Path,
    say: Say<'_>,
) -> Result<()> {
    if !crate::router_unit::installs_here() {
        bail!(
            "the on-demand servers install as systemd user units or launchd agents, and this \
             system has neither — start the server by hand"
        );
    }
    let id = which.sidecar();
    let home = &m.mecha_home;
    let macos = cfg!(target_os = "macos");
    // The proxy is systemd's; on a Mac mecha is its own (module doc). Its
    // path is unused there, and rendered into nothing that runs.
    let proxyd = if macos { "" } else { socket_proxyd()? };
    let units = crate::engine_gate::unit_dir(m)?.to_path_buf();
    let bin = bin_dir(home);
    // On a Mac nothing binds the port at install — `bootstrap` returns before
    // the agent does — so a port another server holds would give a proxy that
    // restarts every ten seconds while the stranger answers the check: refused
    // here instead, unless the agent there is mecha's own (`router_unit`'s
    // rule; review of #678). systemd's socket fails to bind on its own.
    if macos {
        let ours = Manifest::read(home)?
            .entries
            .iter()
            .any(|e| e.sidecar == id && !e.wrote.is_empty());
        if !ours && crate::router_unit::port_answers(naming.public) {
            bail!(
                "something already answers on :{} and mecha did not install it — nothing is \
                 installed over it",
                naming.public
            );
        }
    }
    Manifest::begin(home, id)?;

    // The engine before anything is fetched: a machine with none would
    // otherwise download a model and write units that cannot start.
    let engine = Engine::resolve(m)?;

    // The model first: a unit that starts before its model is there fails
    // its first request, and the launcher refuses rather than start blind.
    let slot = crate::recommend::SLOTS
        .iter()
        .find(|s| s.id == which.slot())
        .context("no recommendation slot for this server")?;
    let (row, _) = crate::recommend::row_for(slot, machine)
        .context("no model is recommended for this server at this machine's tier")?;
    for source in row.sources {
        let crate::recommend::Source::HuggingFace {
            repo,
            revision,
            files,
        } = source
        else {
            bail!("this server's model is not a Hugging Face file");
        };
        for f in *files {
            if crate::fetch::cached(hub, repo, revision, f, false)?
                == crate::fetch::Cached::Verified
            {
                say(&format!("{repo}/{} is in the cache", f.path));
            } else {
                say(&format!(
                    "fetching {repo}/{} ({:.0} MiB)",
                    f.path,
                    f.bytes as f64 / 1_048_576.0
                ));
            }
            crate::fetch::fetch_hub_file(hub, repo, revision, f, &mut |_| {}).await?;
        }
    }

    let (socket, proxy, service, launcher, launcher_name) = which.files();
    let r = |t: &str| render(t, which, naming, &bin, proxyd);
    write_owned(home, id, &bin.join(launcher_name), &r(launcher), 0o755)?;
    if macos {
        let domain = start_agent(m, which, naming, &bin.join(launcher_name), &engine, hub)?;
        // As the router says it: loaded with nobody at the screen, the agent
        // returns after a restart only at a console login (review of #678).
        if domain == crate::launchd::Domain::User {
            say(
                "nobody is logged in at this Mac's screen, so after a restart this server's port \
                 is held again only once someone logs in there",
            );
        }
    } else {
        start_units(
            m,
            which,
            naming,
            &units,
            &bin,
            &engine,
            hub,
            &r,
            (socket, proxy, service),
        )?;
    }
    say(&format!(
        "starting it once through :{} — a cold start, up to 200 s",
        naming.public
    ));
    check(which, naming.public).await?;
    Manifest::finish(home, id)
}

/// The systemd half: the socket, proxy and server units and their drop-ins,
/// the socket enabled.
#[allow(clippy::too_many_arguments)]
fn start_units(
    m: &Machinery,
    which: Which,
    naming: &Naming,
    units: &Path,
    bin: &Path,
    engine: &Engine,
    hub: &Path,
    r: &dyn Fn(&str) -> String,
    (socket, proxy, service): (&str, &str, &str),
) -> Result<()> {
    let id = which.sidecar();
    let home = &m.mecha_home;
    write_owned(
        home,
        id,
        &bin.join("mecha-wait-healthy"),
        WAIT_HEALTHY,
        0o755,
    )?;
    let stem = &naming.stem;
    write_owned(
        home,
        id,
        &units.join(format!("{stem}.socket")),
        &crate::user_service::stamp(id, &r(socket)),
        0o644,
    )?;
    write_owned(
        home,
        id,
        &units.join(format!("{stem}-proxy.service")),
        &crate::user_service::stamp(id, &r(proxy)),
        0o644,
    )?;
    write_owned(
        home,
        id,
        &units.join(format!("{stem}.service")),
        &crate::user_service::stamp(id, &r(service)),
        0o644,
    )?;
    let dropins = units.join(format!("{stem}.service.d"));
    write_owned(home, id, &dropins.join("path.conf"), PATH_CONF, 0o644)?;
    // Where the model was fetched: the launcher resolves the hub again under
    // the user manager's environment, which has none of the shell's `HF_*`.
    write_owned(
        home,
        id,
        &dropins.join("mecha-hub.conf"),
        &hub_text(hub),
        0o644,
    )?;
    // The engine the launcher runs, named in the unit (module doc).
    engine.write(home, id, &dropins)?;

    crate::engine_gate::systemctl("daemon-reload", &[])?;
    crate::engine_gate::systemctl("enable", &["--now", &format!("{stem}.socket")])
}

/// The launchd half: one agent running mecha's own on-demand proxy over the
/// launcher, with the environment the drop-ins carry on Linux.
fn start_agent(
    m: &Machinery,
    which: Which,
    naming: &Naming,
    launcher: &Path,
    engine: &Engine,
    hub: &Path,
) -> Result<crate::launchd::Domain> {
    let id = which.sidecar();
    let home = &m.mecha_home;
    // The proxy is this binary: the one `mecha features enable` runs as,
    // named by absolute path, as `mecha trigger daemon --unit` names itself.
    let mecha = std::env::current_exe().context("finding the mecha binary for the agent")?;
    let plist = crate::engine_gate::unit_dir(m)?.join(crate::launchd::plist_name(&naming.stem));
    let log = crate::launchd::log_path(&m.home, &naming.stem);
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let agent = agent(naming, launcher, engine.path(), hub, &mecha, home, log);
    write_owned(
        home,
        id,
        &plist,
        &crate::user_service::stamp(id, &crate::launchd::plist_text(&agent)),
        0o644,
    )?;
    crate::launchd::load(&plist, &agent.label)
}

/// An on-demand server's launchd agent: mecha's proxy on the public port,
/// under the log cap, starting the launcher at the first connection.
fn agent(
    naming: &Naming,
    launcher: &Path,
    engine: &Path,
    hub: &Path,
    mecha: &Path,
    mecha_home: &Path,
    log: PathBuf,
) -> crate::launchd::Agent {
    crate::launchd::Agent {
        label: crate::launchd::label(&naming.stem),
        program: crate::log_cap::wrap(
            mecha,
            &log,
            vec![
                mecha.display().to_string(),
                "features".into(),
                "on-demand".into(),
                "--listen".into(),
                format!("127.0.0.1:{}", naming.public),
                "--backend".into(),
                naming.backend.to_string(),
                "--idle-secs".into(),
                IDLE_SECS.to_string(),
                "--launcher".into(),
                launcher.display().to_string(),
            ],
        ),
        env: vec![
            ("PATH".into(), crate::launchd::SYSTEM_PATH.into()),
            ("LLAMA_SERVER".into(), engine.display().to_string()),
            ("HF_HUB".into(), hub.display().to_string()),
            // `log-cap` (and `on-demand`) resolve the mecha home from the
            // environment to check they run only mecha's own programs: an
            // agent under a MECHA_HOME other than ~/.mecha must carry it
            // (review of #679).
            ("MECHA_HOME".into(), mecha_home.display().to_string()),
        ],
        workdir: None,
        throttle_secs: crate::launchd::THROTTLE_SECS,
        log,
    }
}

/// Ten minutes, as the proxy unit's `MECHA_LLAMA_IDLE`.
const IDLE_SECS: u64 = 600;

/// The `llama-server` a unit runs, resolved before anything is installed:
/// mecha's own engine when it is installed, else a provided one on the PATH
/// mecha ran with (module doc).
pub(crate) enum Engine {
    Mecha(PathBuf),
    Provided(PathBuf),
}

impl Engine {
    /// The `llama-server` it names.
    pub(crate) fn path(&self) -> &Path {
        match self {
            Engine::Mecha(p) | Engine::Provided(p) => p,
        }
    }

    pub(crate) fn resolve(m: &Machinery) -> Result<Engine> {
        let managed = crate::engine_gate::managed_binary(&m.mecha_home);
        if managed.is_file() {
            return Ok(Engine::Mecha(managed));
        }
        m.path
            .iter()
            .map(|d| d.join("llama-server"))
            .find(|p| p.is_file())
            .map(Engine::Provided)
            .context(
                "there is no llama-server for these servers to run — `mecha features enable` \
                 installs mecha's engine where it is offered, or put a llama.cpp build on PATH",
            )
    }

    /// The drop-in naming it in a unit's `.d` directory: mecha's engine by
    /// `mecha-engine.conf`, recorded under the engine as an adopt records it;
    /// a provided one by `engine-provided.conf`, recorded under `id`.
    pub(crate) fn write(&self, home: &Path, id: &str, dropins: &Path) -> Result<()> {
        match self {
            Engine::Mecha(p) => write_owned(
                home,
                "llama",
                &dropins.join("mecha-engine.conf"),
                &crate::engine_gate::drop_in_text(p),
                0o644,
            ),
            Engine::Provided(p) => write_owned(
                home,
                id,
                &dropins.join("engine-provided.conf"),
                &provided_engine_text(p),
                0o644,
            ),
        }
    }
}

/// The engine the chat router runs, asked before anything is fetched: always
/// mecha's own, installed beside any other (`docs/FEATURES-DESIGN.md` §12).
/// A provided `llama-server` — Homebrew's, a distribution's, a hand build — is
/// left where it is and not run: the router's presets were measured on the
/// pin, and a package manager's binary changes under a running router on its
/// next upgrade, past the measurement gate (Homebrew's on a Mac was 1,431
/// builds behind the pin, 2026-10-13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouterEngine {
    /// mecha's engine is installed.
    Ours,
    /// It is not; this provided one is on the PATH, and stays there.
    Beside(PathBuf),
    /// It is not, and there is no other.
    Absent,
}

pub fn router_engine(m: &Machinery) -> RouterEngine {
    match Engine::resolve(m) {
        Ok(Engine::Mecha(_)) => RouterEngine::Ours,
        Ok(Engine::Provided(path)) => RouterEngine::Beside(path),
        Err(_) => RouterEngine::Absent,
    }
}

/// The drop-in naming the hub a server's model was fetched into.
fn hub_text(hub: &Path) -> String {
    format!(
        "# Written by `mecha features enable`: the Hugging Face cache this server's\n\
         # model was fetched into, so the launcher reads the same one under the user\n\
         # manager's environment.\n\
         [Service]\n\
         Environment=\"HF_HUB={}\"\n",
        hub.display()
    )
}

/// The drop-in naming a provided `llama-server`: the one found on the PATH
/// mecha ran with, left where it is.
pub(crate) fn provided_engine_text(engine: &Path) -> String {
    format!(
        "# Written by `mecha features enable`: the llama-server this server runs, found\n\
         # on the PATH mecha ran with, which can be wider than the unit's own. The\n\
         # binary is left alone; mecha's engine, once adopted, is named by\n\
         # mecha-engine.conf, which sorts after this file and wins.\n\
         [Service]\n\
         Environment=\"LLAMA_SERVER={}\"\n",
        engine.display()
    )
}

/// One request through the public port, waited out across the cold start the
/// socket triggers; then, for embeddings, one embedding.
async fn check(which: Which, port: u16) -> Result<()> {
    let base = format!("http://127.0.0.1:{port}");
    // Loopback, never through a proxy: on a machine that needs one to reach
    // the hub, `http_proxy` is exported for the fetch above, and a proxy
    // answering 2xx would pass a server that never started (as
    // `document.rs` and `imagegen.rs` found on review).
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(200))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    // Refused connections are waited out for a short while: `systemctl
    // enable --now` returns with the socket already listening, but launchd's
    // `bootstrap` returns before the agent has bound the port, so on a Mac
    // the first request can come too early (review of #678). Once something
    // listens, the one request below outwaits the cold start.
    let started = std::time::Instant::now();
    let health = loop {
        match client.get(format!("{base}/health")).send().await {
            Err(e) if e.is_connect() && started.elapsed() < Duration::from_secs(30) => {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            r => {
                break r.with_context(|| {
                    format!("the server on :{port} did not answer its first request")
                })?
            }
        }
    };
    if !health.status().is_success() {
        bail!(
            "the server on :{port} answered {}: {}",
            health.status(),
            health.text().await.unwrap_or_default().trim()
        );
    }
    if which == Which::Embeddings {
        let v: serde_json::Value = client
            .post(format!("{base}/v1/embeddings"))
            .json(&serde_json::json!({"input": "mecha installed this server"}))
            .send()
            .await?
            .json()
            .await
            .context("reading the first embedding")?;
        if v["data"][0]["embedding"]
            .as_array()
            .is_none_or(Vec::is_empty)
        {
            bail!("the embeddings server answered without an embedding: {v}");
        }
    }
    Ok(())
}

/// Stop and remove a server installed under `naming` — what the ignored test
/// cleans up with; `mecha setup <feature> --remove` will stand on it.
pub fn remove(m: &Machinery, which: Which, naming: &Naming) -> Result<()> {
    let home = &m.mecha_home;
    let id = which.sidecar();
    let stem = &naming.stem;
    if cfg!(target_os = "macos") {
        // A job launchd still holds keeps its plist (as the router's does).
        crate::launchd::unload(&crate::launchd::label(stem))?;
    } else {
        let _ = crate::engine_gate::systemctl("disable", &["--now", &format!("{stem}.socket")]);
        let _ = crate::engine_gate::systemctl(
            "stop",
            &[&format!("{stem}-proxy.service"), &format!("{stem}.service")],
        );
    }
    // A record is forgotten only after its file is gone: a removal that
    // fails keeps the record, so the next install does not refuse mecha's
    // own file as someone else's.
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
    for p in [
        units.join(format!("{stem}.socket")),
        units.join(format!("{stem}-proxy.service")),
        units.join(format!("{stem}.service")),
        units.join(crate::launchd::plist_name(stem)),
    ] {
        gone(&p, &[id])?;
    }
    let dropins = units.join(format!("{stem}.service.d"));
    for f in ["path.conf", "mecha-hub.conf", "engine-provided.conf"] {
        gone(&dropins.join(f), &[id])?;
    }
    // mecha's engine drop-in is the engine's record (as an adopt's is).
    gone(&dropins.join("mecha-engine.conf"), &["llama", id])?;
    let _ = std::fs::remove_dir(&dropins);
    let bin = bin_dir(home);
    let (_, _, _, _, launcher_name) = which.files();
    gone(&bin.join(launcher_name), &[id])?;
    // `mecha-wait-healthy` is shared: each server keeps its own record of it,
    // and the file goes only when no other record names it.
    let wait = bin.join("mecha-wait-healthy");
    let others = Manifest::read(home)?
        .entries
        .iter()
        .any(|e| e.sidecar != id && e.wrote.contains(&wait));
    if others {
        Manifest::unrecord(home, id, &wait)?;
    } else {
        gone(&wait, &[id])?;
    }
    if !cfg!(target_os = "macos") {
        crate::engine_gate::systemctl("daemon-reload", &[])?;
    }
    // Removed whole: the entry goes too, so the plan reads the machine again.
    Manifest::forget_if_empty(home, id)
}

#[cfg(test)]
mod tests {
    /// Each on-demand server's revision is held to what its install writes:
    /// the socket, proxy and server units, the launcher, the drop-ins, the
    /// wait script, and the Mac agent.
    #[test]
    fn each_server_s_revision_is_held_to_what_it_writes() {
        for (which, pinned) in [
            (
                Which::Embeddings,
                (
                    1,
                    "f188554d852240691f55c876fe25d1c75c3deb76378d1077c9a1852a6aac1f2b",
                ),
            ),
            (
                Which::Ocr,
                (
                    1,
                    "e88655ea5516a7f46a6cba0b7a0e666e574d626dbbef5059504c81cba312f862",
                ),
            ),
        ] {
            let n = which.shipped();
            let bin = Path::new("/h/.mecha/sidecars/bin");
            let r = |t: &str| render(t, which, &n, bin, "/usr/lib/systemd/systemd-socket-proxyd");
            let (socket, proxy, service, launcher, name) = which.files();
            let hub = Path::new("/h/.cache/huggingface/hub");
            let engine = Path::new("/h/.mecha/sidecars/llama/current/llama-server");
            let plist = crate::launchd::plist_text(&agent(
                &n,
                &bin.join(name),
                engine,
                hub,
                Path::new("/u/.cargo/bin/mecha"),
                Path::new("/h/.mecha"),
                "/u/Library/Logs/mecha/x.log".into(),
            ));
            crate::user_service::assert_rev(
                which.sidecar(),
                pinned,
                &[
                    &r(socket),
                    &r(proxy),
                    &r(service),
                    &r(launcher),
                    PATH_CONF,
                    WAIT_HEALTHY,
                    &hub_text(hub),
                    &provided_engine_text(engine),
                    &crate::engine_gate::drop_in_text(engine),
                    &plist,
                ],
            );
        }
    }

    use super::*;

    /// The router's engine is mecha's own or none: a `llama-server` on the
    /// PATH is `Beside`, never run by the router — and once mecha's is
    /// installed, it is `Ours` whatever else is on the PATH (review of #678:
    /// nothing failed if this folded back into "any engine will do").
    #[test]
    fn the_router_runs_mecha_s_engine_never_a_provided_one() {
        let root = std::env::temp_dir().join(format!("mecha-re-{}", uuid::Uuid::new_v4()));
        let provided = root.join("bin");
        std::fs::create_dir_all(&provided).unwrap();
        let m = |path: Vec<PathBuf>| Machinery {
            home: root.clone(),
            mecha_home: root.join(".mecha"),
            unit_dirs: vec![root.join("units")],
            path,
            docker: Box::new(|_| crate::sidecar::Lookup::Absent),
        };
        assert_eq!(
            router_engine(&m(vec![provided.clone()])),
            RouterEngine::Absent
        );
        std::fs::write(provided.join("llama-server"), b"#!/bin/sh\n").unwrap();
        assert_eq!(
            router_engine(&m(vec![provided.clone()])),
            RouterEngine::Beside(provided.join("llama-server"))
        );
        let managed = crate::engine_gate::managed_binary(&root.join(".mecha"));
        std::fs::create_dir_all(managed.parent().unwrap()).unwrap();
        std::fs::write(&managed, b"#!/bin/sh\n").unwrap();
        assert_eq!(router_engine(&m(vec![provided])), RouterEngine::Ours);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The plist each on-demand install writes is the one the plan looks
    /// for: a renamed stem would otherwise read every Mac's server as absent
    /// (review of #678).
    #[test]
    fn the_shipped_agents_are_the_servers_evidence() {
        for which in [Which::Embeddings, Which::Ocr] {
            let name = crate::launchd::plist_name(&which.shipped().stem);
            let sidecar = crate::sidecar::SIDECARS
                .iter()
                .find(|s| s.id == which.sidecar())
                .unwrap();
            assert!(
                sidecar
                    .evidence
                    .iter()
                    .any(|e| matches!(e, crate::sidecar::Evidence::UserUnit(n) if *n == name)),
                "{name} is not in {:?}",
                sidecar.evidence
            );
        }
    }

    /// The units and launchers in the binary are the ones in `scripts/llama/`:
    /// one set of files, two copies, held equal so they cannot drift.
    #[test]
    fn the_shipped_units_are_the_scripts() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/llama");
        for (name, shipped) in [
            ("llama-embed.socket", EMBED_SOCKET),
            ("llama-embed-proxy.service", EMBED_PROXY),
            ("llama-embed.service", EMBED_SERVICE),
            ("mecha-embed-server", EMBED_LAUNCHER),
            ("llama-ocr.socket", OCR_SOCKET),
            ("llama-ocr-proxy.service", OCR_PROXY),
            ("llama-ocr.service", OCR_SERVICE),
            ("mecha-ocr-server", OCR_LAUNCHER),
            ("mecha-wait-healthy", WAIT_HEALTHY),
            ("path.conf", PATH_CONF),
        ] {
            assert_eq!(
                std::fs::read_to_string(dir.join(name)).unwrap(),
                shipped,
                "{name} drifted from scripts/llama"
            );
        }
        // And nothing in scripts/llama is left out: a file added there (its
        // installers aside) must be shipped too, or the two installs diverge.
        let shipped = [
            "llama-embed.socket",
            "llama-embed-proxy.service",
            "llama-embed.service",
            "mecha-embed-server",
            "llama-ocr.socket",
            "llama-ocr-proxy.service",
            "llama-ocr.service",
            "mecha-ocr-server",
            "mecha-wait-healthy",
            "path.conf",
        ];
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("install") {
                continue;
            }
            assert!(
                shipped.contains(&name.as_str()),
                "scripts/llama/{name} is not shipped"
            );
        }
    }

    /// Rendered under the shipped naming, a unit differs from the script only
    /// in where its launcher is and where the proxy is; under another naming,
    /// every name and port moves and nothing of the live pair remains.
    #[test]
    fn rendering_moves_every_name_and_port() {
        let bin = Path::new("/srv/mecha/sidecars/bin");
        let proxyd = "/lib/systemd/systemd-socket-proxyd";
        let ocr = Which::Ocr;
        let same = render(OCR_SERVICE, ocr, &ocr.shipped(), bin, proxyd);
        assert!(same.contains("ExecStart=/srv/mecha/sidecars/bin/mecha-ocr-server"));
        assert!(same.contains("MECHA_OCR_PORT=18085"));
        assert!(!same.contains("%h/.local/bin"));

        let test = Naming {
            stem: "mecha-test-ocr".into(),
            public: 41000,
            backend: 41001,
        };
        let proxy = render(OCR_PROXY, ocr, &test, bin, proxyd);
        assert!(proxy.contains("Requires=mecha-test-ocr.service"), "{proxy}");
        assert!(proxy.contains(&format!("{proxyd} ")), "{proxy}");
        assert!(proxy.contains("127.0.0.1:41001"), "{proxy}");
        let socket = render(OCR_SOCKET, ocr, &test, bin, proxyd);
        assert!(socket.contains("ListenStream=127.0.0.1:41000"), "{socket}");
        assert!(
            socket.contains("Service=mecha-test-ocr-proxy.service"),
            "{socket}"
        );
        let service = render(OCR_SERVICE, ocr, &test, bin, proxyd);
        assert!(service.contains("MECHA_OCR_PORT=41001"), "{service}");
        assert!(service.contains("mecha-wait-healthy 41001"), "{service}");
        let launcher = render(OCR_LAUNCHER, ocr, &test, bin, proxyd);
        for rendered in [&proxy, &socket, &service, &launcher] {
            for live in [":8085", "18085", "llama-ocr.", "llama-ocr-"] {
                assert!(!rendered.contains(live), "{live} left in:\n{rendered}");
            }
        }
        let embed = render(
            EMBED_SOCKET,
            Which::Embeddings,
            &Naming {
                stem: "mecha-test-embed".into(),
                public: 41010,
                backend: 41011,
            },
            bin,
            proxyd,
        );
        assert!(embed.contains("ListenStream=127.0.0.1:41010"), "{embed}");
        assert!(!embed.contains("8081"), "{embed}");
        let embed_launcher = render(
            EMBED_LAUNCHER,
            Which::Embeddings,
            &Naming {
                stem: "mecha-test-embed".into(),
                public: 41010,
                backend: 41011,
            },
            bin,
            proxyd,
        );
        assert!(!embed_launcher.contains(":8081"), "{embed_launcher}");
    }

    /// A file mecha did not write is never overwritten: identical bytes are
    /// adopted into the record, different ones refuse.
    #[test]
    fn a_file_mecha_did_not_write_is_left_or_refused() {
        let home = std::env::temp_dir().join(format!("mecha-units-{}", uuid::Uuid::new_v4()));
        Manifest::begin(&home, "ocr-server").unwrap();
        let p = home.join("bin/mecha-wait-healthy");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, WAIT_HEALTHY).unwrap();
        write_owned(&home, "ocr-server", &p, WAIT_HEALTHY, 0o755).unwrap();
        // Recorded when found identical, so it is mecha's from then on: a
        // later write replaces it.
        std::fs::write(&p, "#!/bin/sh\necho changed\n").unwrap();
        write_owned(&home, "ocr-server", &p, WAIT_HEALTHY, 0o755).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), WAIT_HEALTHY);
        // Bytes that are not text, and a link to nothing, cannot be said to be
        // anyone's: refused, untouched.
        let binary = home.join("bin/binary");
        std::fs::write(&binary, [0xff, 0xfe]).unwrap();
        let err = write_owned(&home, "ocr-server", &binary, "ours", 0o755)
            .unwrap_err()
            .to_string();
        assert!(err.contains("cannot read"), "{err}");
        assert_eq!(std::fs::read(&binary).unwrap(), vec![0xff, 0xfe]);
        #[cfg(unix)]
        {
            let dangling = home.join("bin/dangling");
            std::os::unix::fs::symlink(home.join("nowhere"), &dangling).unwrap();
            let err = write_owned(&home, "ocr-server", &dangling, "ours", 0o755)
                .unwrap_err()
                .to_string();
            assert!(err.contains("cannot read"), "{err}");
            assert!(std::fs::symlink_metadata(&dangling)
                .unwrap()
                .file_type()
                .is_symlink());
        }
        // A file both servers write is recorded under each: removing one
        // must not delete what the other's unit still runs.
        Manifest::begin(&home, "embed-server").unwrap();
        let shared = home.join("bin/mecha-wait-healthy-shared");
        write_owned(&home, "embed-server", &shared, WAIT_HEALTHY, 0o755).unwrap();
        write_owned(&home, "ocr-server", &shared, WAIT_HEALTHY, 0o755).unwrap();
        let man = Manifest::read(&home).unwrap();
        for id in ["embed-server", "ocr-server"] {
            let e = man.entries.iter().find(|e| e.sidecar == id).unwrap();
            assert!(
                e.wrote.contains(&shared),
                "{id} does not record the shared file"
            );
        }
        // An adopted copy gets the mode it needs.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let hand = home.join("bin/hand-copy");
            std::fs::write(&hand, WAIT_HEALTHY).unwrap();
            std::fs::set_permissions(&hand, std::fs::Permissions::from_mode(0o644)).unwrap();
            write_owned(&home, "ocr-server", &hand, WAIT_HEALTHY, 0o755).unwrap();
            let mode = std::fs::metadata(&hand).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755);
        }
        let fresh = home.join("bin/other");
        std::fs::write(&fresh, "theirs").unwrap();
        let err = write_owned(&home, "ocr-server", &fresh, "ours", 0o755)
            .unwrap_err()
            .to_string();
        assert!(err.contains("did not write"), "{err}");
        assert_eq!(std::fs::read_to_string(&fresh).unwrap(), "theirs");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The real thing, beside the live pair: an OCR server installed under its
    /// own names on free ports in this user's systemd, started through its
    /// socket (a cold start), answering, then removed. Needs systemd, the
    /// OCR model in the hub, and an engine on PATH or mecha's own.
    #[tokio::test]
    #[ignore]
    async fn an_ocr_server_installs_and_answers_beside_the_live_one() {
        // Both ports held until both are taken: a port released and reused
        // would point the proxy back at its own socket.
        let (a, b) = (
            std::net::TcpListener::bind("127.0.0.1:0").unwrap(),
            std::net::TcpListener::bind("127.0.0.1:0").unwrap(),
        );
        let (public, backend) = (
            a.local_addr().unwrap().port(),
            b.local_addr().unwrap().port(),
        );
        drop((a, b));
        assert_ne!(public, backend);
        let naming = Naming {
            stem: format!("mecha-test-ocr-{}", std::process::id()),
            public,
            backend,
        };
        let root = std::env::temp_dir().join(format!("mecha-7c1-{}", uuid::Uuid::new_v4()));
        let mut m = Machinery::real().unwrap();
        m.mecha_home = root.join(".mecha");
        let machine = crate::recommend::Machine::read().unwrap();
        let hub = crate::fetch::hub_dir().unwrap();
        let mut said = Vec::new();
        let r = install(&m, Which::Ocr, &naming, &machine, &hub, &mut |s| {
            said.push(s.to_string())
        })
        .await;
        let cleaned = remove(&m, Which::Ocr, &naming);
        eprintln!("{said:#?}");
        r.unwrap();
        cleaned.unwrap();
        // Removed is removed from the record too: the entry goes, so the plan
        // reads the machine again rather than an empty record it would call
        // unknown and never offer over.
        let man = Manifest::read(&m.mecha_home).unwrap();
        assert!(
            !man.entries.iter().any(|e| e.sidecar == "ocr-server"),
            "{:?}",
            man.entries
        );
        let unit = crate::engine_gate::unit_dir(&m)
            .unwrap()
            .join(format!("{}.socket", naming.stem));
        assert!(!unit.exists(), "removed");
        let _ = std::fs::remove_dir_all(&root);
    }
}
