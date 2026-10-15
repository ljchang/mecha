//! launchd user agents: what a systemd user unit is on Linux, on macOS
//! (`docs/FEATURES-DESIGN.md` §12).
//!
//! **One plist per service, in `~/Library/LaunchAgents/`.** The agent runs
//! the same launcher the systemd unit runs — the launchers are POSIX `sh` and
//! read their settings from the environment — so what differs between the two
//! machines is only who starts the launcher and what it is told. The plist
//! carries the environment a unit's `Environment=` lines and engine drop-in
//! carry on Linux; there are no drop-ins.
//!
//! **Loaded by `bootout` then `bootstrap`, never by `kickstart` alone.**
//! launchd reads a plist when it is bootstrapped and not again, so an install
//! that rewrote it (a new model, another engine) must unload the old
//! definition first; `bootstrap` with `RunAtLoad` then starts it. `bootout`
//! returns before the job has exited, and a `bootstrap` racing it fails with
//! an I/O error, so the job is waited out between the two.
//!
//! **The domain is `gui/<uid>` where there is a login session**, which is
//! also where `~/Library/LaunchAgents` is loaded from at the next login. Over
//! SSH with nobody logged in at the screen there is no `gui` domain, and the
//! agent goes into `user/<uid>`: it runs now, and loads at the next graphical
//! login like any agent in that directory.
//!
//! **There is no journal.** The agent's standard output and error go to a
//! file under `~/Library/Logs/mecha/`, which launchd does not create — the
//! install does — and which is where an error message sends the owner.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The launchd label for a service stem: reverse DNS under mecha's domain.
pub fn label(stem: &str) -> String {
    format!("ai.mecha-factory.{stem}")
}

/// The plist's file name for a service stem.
pub fn plist_name(stem: &str) -> String {
    format!("{}.plist", label(stem))
}

/// The user's agents directory.
pub fn agents_dir(home: &Path) -> PathBuf {
    home.join("Library").join("LaunchAgents")
}

/// The file an agent's output goes to.
pub fn log_path(home: &Path, stem: &str) -> PathBuf {
    home.join("Library")
        .join("Logs")
        .join("mecha")
        .join(format!("{stem}.log"))
}

/// What one agent runs, and how.
#[derive(Debug, Clone, PartialEq)]
pub struct Agent {
    pub label: String,
    pub program: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Where it runs, as a unit's `WorkingDirectory=`.
    pub workdir: Option<PathBuf>,
    /// Seconds between restarts after a failure, as a unit's `RestartSec=`.
    pub throttle_secs: u32,
    pub log: PathBuf,
}

/// launchd's own default throttle, and the router's and on-demand servers'
/// `RestartSec=10`.
pub const THROTTLE_SECS: u32 = 10;

/// The agent a rendered systemd unit describes: its `ExecStart=`,
/// `Environment=` and `WorkingDirectory=` lines, with `%h` as `home` — so a
/// Mac runs what the Linux unit runs, from the one file, and the two cannot
/// drift (§12.2). `PATH` is the system directories unless the unit names one,
/// since an agent gets launchd's environment, not a shell's. A unit whose
/// command needs a shell's quoting is refused, never split wrongly.
pub fn agent_from_unit(label: &str, unit: &str, home: &Path, log: PathBuf) -> Result<Agent> {
    let expand = |s: &str| s.replace("%h", &home.display().to_string());
    let (mut program, mut env, mut workdir) = (None, Vec::new(), None);
    let mut throttle = THROTTLE_SECS;
    for line in unit.lines().map(str::trim) {
        if let Some(cmd) = line.strip_prefix("ExecStart=") {
            // systemd's prefixes (`-` ignore failure, `@`, `+`, `!`, `:`)
            // mean something an agent cannot say: refused, never taken as
            // part of the program's name (review of #679).
            if cmd.starts_with(['-', '@', '+', '!', ':']) || cmd.contains(['"', '\\', '\'', '$']) {
                bail!("{label}'s command needs a shell's quoting, which an agent does not have: {cmd}");
            }
            program = Some(cmd.split_whitespace().map(expand).collect::<Vec<_>>());
        } else if let Some(assignments) = line.strip_prefix("Environment=") {
            for kv in environment_words(label, assignments)? {
                let (k, v) = kv
                    .split_once('=')
                    .with_context(|| format!("{label}: `Environment={kv}` names no value"))?;
                env.push((k.to_string(), expand(v)));
            }
        } else if let Some(d) = line.strip_prefix("WorkingDirectory=") {
            workdir = Some(PathBuf::from(expand(d)));
        } else if let Some(secs) = line.strip_prefix("RestartSec=") {
            throttle = secs.trim().parse().with_context(|| {
                format!("{label}: `RestartSec={secs}` is not a whole number of seconds")
            })?;
        } else if let Some(v) = line.strip_prefix("Type=") {
            // The agent is a long-running program, as `exec` and `simple` are;
            // `oneshot` would become an agent relaunched forever (review of
            // #679).
            if !matches!(v.trim(), "exec" | "simple") {
                bail!("{label}'s unit says `{line}`, which a launchd agent does not mean");
            }
        } else if let Some(v) = line.strip_prefix("Restart=") {
            // Every agent is `KeepAlive` on a failed exit: `on-failure` only.
            if v.trim() != "on-failure" {
                bail!("{label}'s unit says `{line}`; an agent restarts on failure only");
            }
        } else if let Some((key, _)) = line.split_once('=') {
            // A closed set: every other directive is one an agent has an
            // answer for already, or it is refused. A new `ExecStartPre=` or
            // `EnvironmentFile=` would otherwise reach Linux and silently
            // not a Mac (review of #679).
            if !line.starts_with('#') && !AGENT_HAS_NO_USE_FOR.contains(&key.trim()) {
                bail!(
                    "{label}'s unit says `{line}`, which this reader does not carry into a \
                     launchd agent — teach `launchd::agent_from_unit` what it means first"
                );
            }
        }
    }
    let program = program.with_context(|| format!("{label}'s unit has no ExecStart="))?;
    // Every agent restarts on a failed exit, so a unit that does not would
    // mean one thing on Linux and another here (review of #679).
    if !unit.lines().any(|l| l.trim() == "Restart=on-failure") {
        bail!("{label}'s unit has no `Restart=on-failure`, which every agent is");
    }
    if !env.iter().any(|(k, _)| k == "PATH") {
        env.insert(0, ("PATH".into(), SYSTEM_PATH.into()));
    }
    Ok(Agent {
        label: label.to_string(),
        program,
        env,
        workdir,
        throttle_secs: throttle,
        log,
    })
}

/// One `Environment=` line's assignments, as systemd reads it: several per
/// line, separated by spaces, an assignment with a space in it quoted whole
/// (`"GREETING=hello there"`). A backslash or a single quote is refused
/// rather than guessed at — the reader exists so a Mac runs what Linux runs,
/// and a silently different value is the one thing it must not produce
/// (review of #679).
fn environment_words(label: &str, line: &str) -> Result<Vec<String>> {
    if line.contains(['\\', '\'']) {
        bail!("{label}: `Environment={line}` needs quoting this reader does not take");
    }
    let (mut words, mut word, mut quoted) = (Vec::new(), String::new(), false);
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            c => word.push(c),
        }
    }
    if quoted {
        bail!("{label}: `Environment={line}` has an unclosed quote");
    }
    if !word.is_empty() {
        words.push(word);
    }
    Ok(words)
}

/// The unit directives an agent has an answer for without reading them:
/// `Type=exec` and `Restart=on-failure` are what every agent is (`KeepAlive`
/// on a failed exit), the ordering and install lines are systemd's own, and
/// `OOMScoreAdjust=` has no launchd counterpart worth a guess.
const AGENT_HAS_NO_USE_FOR: &[&str] = &[
    "Description",
    "Documentation",
    "After",
    "Wants",
    "Requires",
    "WantedBy",
    "OOMScoreAdjust",
];

/// An agent's `PATH` when its unit names none: the system's own directories.
pub const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// The plist for an agent: started at load, restarted when it exits with a
/// failure (systemd's `Restart=on-failure`), `throttle_secs` apart
/// (`RestartSec=`).
pub fn plist_text(a: &Agent) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <!-- Written by mecha. `mecha setup chat` again rewrites it. -->\n\
         <plist version=\"1.0\">\n<dict>\n",
    );
    let key = |out: &mut String, k: &str| out.push_str(&format!("  <key>{}</key>\n", xml(k)));
    key(&mut out, "Label");
    out.push_str(&format!("  <string>{}</string>\n", xml(&a.label)));
    key(&mut out, "ProgramArguments");
    out.push_str("  <array>\n");
    for arg in &a.program {
        out.push_str(&format!("    <string>{}</string>\n", xml(arg)));
    }
    out.push_str("  </array>\n");
    key(&mut out, "EnvironmentVariables");
    out.push_str("  <dict>\n");
    for (k, v) in &a.env {
        out.push_str(&format!(
            "    <key>{}</key>\n    <string>{}</string>\n",
            xml(k),
            xml(v)
        ));
    }
    out.push_str("  </dict>\n");
    if let Some(dir) = &a.workdir {
        key(&mut out, "WorkingDirectory");
        out.push_str(&format!(
            "  <string>{}</string>\n",
            xml(&dir.display().to_string())
        ));
    }
    key(&mut out, "RunAtLoad");
    out.push_str("  <true/>\n");
    key(&mut out, "KeepAlive");
    out.push_str("  <dict>\n    <key>SuccessfulExit</key>\n    <false/>\n  </dict>\n");
    key(&mut out, "ThrottleInterval");
    out.push_str(&format!("  <integer>{}</integer>\n", a.throttle_secs));
    for k in ["StandardOutPath", "StandardErrorPath"] {
        key(&mut out, k);
        out.push_str(&format!(
            "  <string>{}</string>\n",
            xml(&a.log.display().to_string())
        ));
    }
    out.push_str("</dict>\n</plist>\n");
    out
}

/// Text made safe inside a plist element: a path with `&` or `<` in it is a
/// malformed plist launchd refuses to load, not a different path.
fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn uid() -> u32 {
    // SAFETY: `getuid` cannot fail and has no preconditions.
    unsafe { libc::getuid() }
}

/// Where an agent was loaded (module doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    /// `gui/<uid>`: a login session, which also loads the agent at each
    /// graphical login.
    Gui,
    /// `user/<uid>`: nobody at the screen. The agent runs now, and is loaded
    /// again only at the next graphical login, not at boot.
    User,
}

impl Domain {
    fn spec(self) -> String {
        match self {
            Domain::Gui => format!("gui/{}", uid()),
            Domain::User => format!("user/{}", uid()),
        }
    }
}

/// `gui/<uid>` when there is a login session to load into, else `user/<uid>`.
fn domain() -> Domain {
    let has_gui = std::process::Command::new("launchctl")
        .args(["print", &Domain::Gui.spec()])
        .output()
        .is_ok_and(|o| o.status.success());
    if has_gui {
        Domain::Gui
    } else {
        Domain::User
    }
}

/// `launchctl <args…>`, named on failure.
fn launchctl(args: &[&str]) -> Result<()> {
    let out = std::process::Command::new("launchctl")
        .args(args)
        .output()
        .with_context(|| format!("running launchctl {}", args.join(" ")))?;
    if !out.status.success() {
        bail!(
            "launchctl {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Whether launchd has a job by this label in `domain`.
fn loaded(domain: &str, label: &str) -> bool {
    std::process::Command::new("launchctl")
        .args(["print", &format!("{domain}/{label}")])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Unload the agent if it is loaded, and wait until launchd has let it go —
/// a `bootstrap` that follows too soon fails (module doc).
fn unload_in(domain: &str, label: &str) -> Result<()> {
    if !loaded(domain, label) {
        return Ok(());
    }
    // An error here is a job that went between the check and the call;
    // whether it is still there is asked below, not inferred.
    let _ = launchctl(&["bootout", &format!("{domain}/{label}")]);
    let started = std::time::Instant::now();
    while loaded(domain, label) {
        if started.elapsed() > Duration::from_secs(30) {
            bail!("launchd still holds {label} thirty seconds after it was told to unload it");
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

/// Load (or reload) the agent at `plist`, which starts it: the old
/// definition is unloaded first, so launchd reads the file as written now.
///
/// Unloaded from both domains, not only the one it goes into: an agent
/// bootstrapped into `user/<uid>` over SSH is loaded again into `gui/<uid>`
/// at the next login, and a reinstall that replaced only one copy would leave
/// the other holding the port (review of #678).
/// The domain it went into is returned, for the caller to say what that means.
/// It is enabled first in both, undoing a `disable` a failed install left.
pub fn load(plist: &Path, label: &str) -> Result<Domain> {
    unload(label)?;
    for d in [Domain::Gui, Domain::User] {
        // A domain with nobody in it refuses; the bootstrap below is what
        // must succeed.
        let _ = launchctl(&["enable", &format!("{}/{label}", d.spec())]);
    }
    let domain = domain();
    launchctl(&["bootstrap", &domain.spec(), &plist.display().to_string()])?;
    Ok(domain)
}

/// Unload the agent and keep it from loading again at the next login —
/// systemd's `disable --now`. `bootout` alone leaves the plist where launchd
/// reads it at login, so a rolled-back install would come back (review of
/// #679); `disable` is launchd's persistent override, cleared by [`load`].
/// Disabled in both domains; it is enough that one accepted it.
pub fn disable(label: &str) -> Result<()> {
    // The override is set whatever the unload answered: a bootout that timed
    // out is exactly when the agent must not come back at login (review of
    // #679). Its error is still returned after.
    let unloaded = unload(label);
    let mut first_err = None;
    let mut any = false;
    for d in [Domain::Gui, Domain::User] {
        match launchctl(&["disable", &format!("{}/{label}", d.spec())]) {
            Ok(()) => any = true,
            Err(e) => {
                first_err.get_or_insert(e);
            }
        }
    }
    unloaded?;
    match (any, first_err) {
        (true, _) | (false, None) => Ok(()),
        (false, Some(e)) => Err(e),
    }
}

/// Stop and unload the agent, wherever it is loaded. Both domains are tried
/// whatever the first answers; the first failure is returned after.
pub fn unload(label: &str) -> Result<()> {
    let gui = unload_in(&Domain::Gui.spec(), label);
    let user = unload_in(&Domain::User.spec(), label);
    gui.and(user)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The plist names the launcher, its environment and its log, restarts
    /// on failure only, and escapes what XML would misread.
    #[test]
    fn the_plist_carries_the_program_its_environment_and_its_log() {
        let text = plist_text(&Agent {
            label: label("llama-local"),
            program: vec!["/Users/a b/.mecha/sidecars/bin/mecha-router".into()],
            env: vec![
                ("MECHA_ROUTER_PORT".into(), "8080".into()),
                ("LLAMA_SERVER".into(), "/opt/R&D/llama-server".into()),
            ],
            workdir: None,
            throttle_secs: THROTTLE_SECS,
            log: "/Users/a b/Library/Logs/mecha/llama-local.log".into(),
        });
        for want in [
            "<string>ai.mecha-factory.llama-local</string>",
            "<string>/Users/a b/.mecha/sidecars/bin/mecha-router</string>",
            "<key>MECHA_ROUTER_PORT</key>\n    <string>8080</string>",
            "<string>/opt/R&amp;D/llama-server</string>",
            "<key>SuccessfulExit</key>\n    <false/>",
            "<key>RunAtLoad</key>\n  <true/>",
            "<key>StandardErrorPath</key>\n  <string>/Users/a b/Library/Logs/mecha/llama-local.log</string>",
        ] {
            assert!(text.contains(want), "{want} missing:\n{text}");
        }
        assert!(!text.contains("R&D"), "an unescaped ampersand:\n{text}");
    }

    /// When `plutil` is here (every Mac), the plist is one it accepts — the
    /// check launchd's own parser would make at `bootstrap` — in the form
    /// written, with its revision stamp (`user_service::stamp`).
    #[test]
    fn plutil_accepts_the_plist() {
        let dir = std::env::temp_dir().join(format!("mecha-launchd-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(plist_name("mecha-test"));
        std::fs::write(
            &path,
            crate::user_service::stamp(
                "comfyui",
                &plist_text(&Agent {
                    label: label("mecha-test"),
                    program: vec!["/bin/echo".into(), "<&>".into()],
                    env: vec![("A".into(), "\"quoted\"".into())],
                    workdir: Some(dir.clone()),
                    throttle_secs: 5,
                    log: dir.join("t.log"),
                }),
            ),
        )
        .unwrap();
        match std::process::Command::new("plutil")
            .arg("-lint")
            .arg(&path)
            .output()
        {
            Ok(o) => assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout)),
            // Off a Mac there is nothing to lint with; on one, a missing
            // plutil is a broken machine, not a pass.
            Err(e) if cfg!(target_os = "macos") => panic!("plutil is part of macOS: {e}"),
            Err(_) => {}
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
