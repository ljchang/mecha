//! Confining `shell`.
//!
//! Every other tool is bounded by [`ToolCtx::resolve`](crate::tool::ToolCtx),
//! which proves a path stays inside the workspace before touching it. `shell`
//! cannot work that way: the path jail cannot see inside `bash -c`, and a
//! command is free to `cd /`, read `~/.aws/credentials`, and `curl` it out.
//! The capability model has always said so — `shell` is marked
//! private + sends + destructive — but saying it is not enforcing it.
//!
//! This is the enforcement. A sandboxed `shell` gets the workspace and a
//! read-only system, no home directory, no environment, and by default no
//! network.
//!
//! ## The rule that matters
//!
//! **A configured sandbox that does not work must stop the run, never quietly
//! degrade.** Falling back to unconfined execution when `bwrap` is missing is
//! worse than having no sandbox at all: the operator believes commands are
//! confined and writes policy on that belief. So [`Sandbox::preflight`] runs a
//! real command through the real backend, and a failure is an error with
//! instructions rather than a warning.
//!
//! ## What this is not
//!
//! Not a security boundary against a determined kernel exploit. It is the
//! difference between "an injected command can read your SSH keys" and "an
//! injected command can read the files you pointed the agent at", which is the
//! difference that decides whether an agent can be woken by an email.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Push a run of arguments. A closure would borrow the vector for its whole
/// lifetime, which collides with the interleaved dynamic pushes below.
macro_rules! args {
    ($v:expr, $($s:expr),+ $(,)?) => {{ $( $v.push($s.to_string()); )+ }};
}

/// How commands are confined.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// Run directly, as you, with your credentials. The historical behaviour,
    /// and the only sane default for a supervised CLI on a machine where the
    /// alternatives may not be installed.
    #[default]
    None,
    /// User namespaces via `bwrap`. Cheap — no daemon, a few milliseconds —
    /// and the right choice where unprivileged user namespaces are permitted.
    Bwrap,
    /// A throwaway container. Works where user namespaces are locked down,
    /// costs a container start per command.
    Docker,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::None => "none",
            Backend::Bwrap => "bwrap",
            Backend::Docker => "docker",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SandboxConfig {
    pub kind: Backend,
    /// Let confined commands reach the network.
    ///
    /// Off by default, and this is the single most valuable line in the file:
    /// with it off, `shell` stops being an exfiltration route, which is what
    /// lets the trifecta interlock relax rather than tighten.
    pub network: bool,
    /// Extra paths mounted writable, on top of the workspace.
    pub writable: Vec<PathBuf>,
    /// Extra paths mounted read-only. Use for a toolchain or a cache that
    /// lives outside the workspace.
    pub readable: Vec<PathBuf>,
    /// Environment variables passed through by name. Nothing else survives —
    /// an allowlist, because the interesting variables are the secret ones.
    pub env: Vec<String>,
    /// Container image for the `docker` backend.
    pub image: String,
    /// Memory ceiling in megabytes (`docker` only).
    pub memory_mb: Option<u64>,
    /// CPU ceiling (`docker` only), e.g. `2.0`.
    pub cpus: Option<f64>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        SandboxConfig {
            kind: Backend::None,
            network: false,
            writable: Vec::new(),
            readable: Vec::new(),
            env: Vec::new(),
            // Small, ubiquitous, and has a shell and coreutils. Anything the
            // agent actually needs to build with should be a different image.
            image: "debian:stable-slim".into(),
            memory_mb: None,
            cpus: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Sandbox {
    cfg: SandboxConfig,
}

impl Sandbox {
    pub fn new(cfg: SandboxConfig) -> Self {
        Sandbox { cfg }
    }

    pub fn backend(&self) -> Backend {
        self.cfg.kind
    }

    /// The same policy with the network decided differently.
    ///
    /// Exists because network is otherwise one switch for everything, and the
    /// common case wants them split: a third-party MCP server that has to reach
    /// its own API, confined, while `shell` still has no way out. Sharing one
    /// flag would mean opening `shell` to satisfy the server.
    pub fn with_network(&self, network: bool) -> Self {
        Sandbox { cfg: SandboxConfig { network, ..self.cfg.clone() } }
    }

    pub fn is_enabled(&self) -> bool {
        self.cfg.kind != Backend::None
    }

    /// Can a confined command still reach off this machine?
    ///
    /// This is what decides whether `shell` counts as an `external_send` sink.
    /// Unconfined, it always can. Confined without network, it cannot — and the
    /// interlock should stop treating it as a way out, because it isn't one.
    pub fn can_reach_network(&self) -> bool {
        !self.is_enabled() || self.cfg.network
    }

    /// Can a confined command read data outside the workspace?
    ///
    /// Unconfined it reads your whole home directory. Confined it sees the
    /// workspace and a read-only system — the same reach `fs_read` already has,
    /// which is classified `private` on the same reasoning.
    pub fn reaches_beyond_workspace(&self) -> bool {
        !self.is_enabled() || !self.cfg.writable.is_empty() || !self.cfg.readable.is_empty()
    }

    /// Build the process that will run `command`.
    ///
    /// `workspace` is mounted writable; `cwd` must be inside it — the caller has
    /// already proved that through `ToolCtx::resolve`.
    pub fn command(
        &self,
        command: &str,
        workspace: &Path,
        cwd: &Path,
    ) -> Result<tokio::process::Command> {
        match self.cfg.kind {
            Backend::None => {
                let mut c = tokio::process::Command::new("bash");
                c.arg("-lc").arg(command).current_dir(cwd);
                Ok(c)
            }
            _ => self.wrap_argv("bash", &["-lc".into(), command.into()], workspace, cwd),
        }
    }

    /// Confine an explicit argv, with no shell in between.
    ///
    /// For long-lived children — an MCP server — where routing through
    /// `bash -lc` would mean quoting caller-supplied arguments correctly, and
    /// getting that wrong is a command-injection bug rather than a typo.
    pub fn wrap_argv(
        &self,
        program: &str,
        args: &[String],
        workspace: &Path,
        cwd: &Path,
    ) -> Result<tokio::process::Command> {
        match self.cfg.kind {
            Backend::None => {
                let mut c = tokio::process::Command::new(program);
                c.args(args).current_dir(cwd);
                Ok(c)
            }
            Backend::Bwrap => {
                let mut c = tokio::process::Command::new("bwrap");
                c.args(self.bwrap_args(workspace, cwd)?);
                c.arg("--").arg(program).args(args);
                Ok(c)
            }
            Backend::Docker => {
                let mut c = tokio::process::Command::new("docker");
                c.args(self.docker_args(workspace, cwd)?);
                c.arg(program).args(args);
                Ok(c)
            }
        }
    }

    /// The environment a child should get, given a passthrough allowlist.
    ///
    /// Inheriting mecha's environment hands a third-party process every secret
    /// you have exported — provider keys first among them. So the rule is the
    /// same as inside the sandbox: a minimal base, plus what was named.
    ///
    /// `HOME` and `PATH` are in the base because without them most runtimes
    /// (node, python) cannot find their own modules, and a server that cannot
    /// start teaches the operator to turn this off.
    pub fn child_env(passthrough: &[String]) -> Vec<(String, String)> {
        const BASE: [&str; 5] = ["PATH", "HOME", "LANG", "LC_ALL", "TZ"];

        BASE.iter()
            .map(|s| s.to_string())
            .chain(passthrough.iter().cloned())
            .filter_map(|name| std::env::var(&name).ok().map(|v| (name, v)))
            .collect()
    }

    /// Arguments up to (but not including) the command itself. Split out so the
    /// policy can be asserted on in tests without spawning anything.
    pub fn bwrap_args(&self, workspace: &Path, cwd: &Path) -> Result<Vec<String>> {
        let mut a: Vec<String> = Vec::new();

        // `--die-with-parent` so a wedged command cannot outlive mecha.
        // `--new-session` detaches the controlling terminal, which blocks the
        // TIOCSTI trick of pushing characters into the parent's input queue.
        args!(
            a,
            "--die-with-parent",
            "--new-session",
            "--unshare-user",
            "--unshare-pid",
            "--unshare-ipc",
            "--unshare-uts",
            "--unshare-cgroup-try",
        );
        if !self.cfg.network {
            args!(a, "--unshare-net");
        }

        // The system, read-only. `/bin`, `/lib` and friends are symlinks into
        // `/usr` on merged systems and real directories on older ones, so ask
        // rather than assume — binding a symlink as a directory fails.
        for dir in ["/usr", "/etc", "/opt"] {
            if Path::new(dir).is_dir() {
                args!(a, "--ro-bind-try", dir, dir);
            }
        }
        for dir in ["/bin", "/sbin", "/lib", "/lib32", "/lib64"] {
            match std::fs::symlink_metadata(dir) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    let target = std::fs::read_link(dir)?;
                    args!(a, "--symlink", target.to_string_lossy(), dir);
                }
                Ok(_) => args!(a, "--ro-bind-try", dir, dir),
                Err(_) => {}
            }
        }

        // A private /tmp, so a command cannot leave anything behind for the
        // next one or read what the last one left.
        args!(a, "--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp");

        for path in &self.cfg.readable {
            args!(a, "--ro-bind-try", path.display(), path.display());
        }

        let workspace = absolute(workspace)?;
        args!(a, "--bind", workspace.display(), workspace.display());
        for path in &self.cfg.writable {
            args!(a, "--bind-try", path.display(), path.display());
        }

        args!(a, "--chdir", absolute(cwd)?.display());

        // Nothing from the parent environment survives unless it is named.
        // API keys live in the environment; a confined command that inherits
        // them is confined in the least interesting way.
        args!(
            a,
            "--clearenv",
            "--setenv",
            "PATH",
            "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
            "--setenv",
            "HOME",
            workspace.display(),
        );
        for name in &self.cfg.env {
            if let Ok(value) = std::env::var(name) {
                args!(a, "--setenv", name, value);
            }
        }

        Ok(a)
    }

    /// Arguments up to (but not including) the command itself.
    pub fn docker_args(&self, workspace: &Path, cwd: &Path) -> Result<Vec<String>> {
        let workspace = absolute(workspace)?;
        let mut a: Vec<String> = Vec::new();
        args!(a, "run", "--rm", "-i");

        args!(a, "--network", if self.cfg.network { "bridge" } else { "none" });

        // Root inside a container writing into a bind-mounted workspace leaves
        // root-owned files behind on the host, which the user then cannot
        // delete. Run as the caller.
        #[cfg(unix)]
        {
            let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
            args!(a, "--user", format!("{uid}:{gid}"));
        }

        args!(a, "--security-opt", "no-new-privileges", "--cap-drop", "ALL");

        if let Some(mb) = self.cfg.memory_mb {
            args!(a, "--memory", format!("{mb}m"));
        }
        if let Some(cpus) = self.cfg.cpus {
            args!(a, "--cpus", cpus);
        }

        for path in &self.cfg.readable {
            args!(a, "-v", format!("{}:{}:ro", path.display(), path.display()));
        }
        args!(a, "-v", format!("{}:{}", workspace.display(), workspace.display()));
        for path in &self.cfg.writable {
            args!(a, "-v", format!("{}:{}", path.display(), path.display()));
        }

        args!(a, "-w", absolute(cwd)?.display());

        for name in &self.cfg.env {
            if let Ok(value) = std::env::var(name) {
                args!(a, "-e", format!("{name}={value}"));
            }
        }

        args!(a, self.cfg.image);
        Ok(a)
    }

    /// Prove the sandbox actually works, by running something through it.
    ///
    /// Called once at startup rather than on the first tool call, so a
    /// misconfiguration is a clear message at launch instead of a confusing
    /// tool error twenty turns into a run.
    pub async fn preflight(&self, workspace: &Path) -> Result<()> {
        if !self.is_enabled() {
            return Ok(());
        }

        let marker = "mecha-sandbox-ok";
        let mut command = self
            .command(&format!("echo {marker}"), workspace, workspace)
            .context("building the sandbox command")?;

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            command.stdin(std::process::Stdio::null()).output(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("the {} sandbox timed out starting", self.cfg.kind.as_str()))?
        .with_context(|| {
            format!(
                "cannot run `{}` — is it installed?",
                match self.cfg.kind {
                    Backend::Docker => "docker",
                    _ => "bwrap",
                }
            )
        })?;

        if output.status.success()
            && String::from_utf8_lossy(&output.stdout).contains(marker)
        {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(
            "the {} sandbox does not work here: {}{}",
            self.cfg.kind.as_str(),
            if stderr.is_empty() { "no output".into() } else { stderr.clone() },
            diagnose(self.cfg.kind, &stderr)
        )
    }
}

/// Turn a backend's error into something the operator can act on.
///
/// The Ubuntu 24.04 case is the one worth spelling out: `bwrap` is installed,
/// `unprivileged_userns_clone` is 1, and it still fails, because AppArmor
/// gained a separate switch that nothing mentions.
fn diagnose(kind: Backend, stderr: &str) -> String {
    match kind {
        Backend::Bwrap if stderr.contains("uid map") || stderr.contains("user namespace") => {
            "\n\nUnprivileged user namespaces are blocked. On Ubuntu 23.10+ this is \
             usually AppArmor rather than the kernel:\n  \
             sysctl kernel.apparmor_restrict_unprivileged_userns   # 1 means blocked\n\
             Either install an AppArmor profile for bwrap, or set \
             `kernel.apparmor_restrict_unprivileged_userns=0` (system-wide, weaker), \
             or use `kind = \"docker\"` instead."
                .into()
        }
        Backend::Bwrap if stderr.contains("loopback") => {
            "\n\nbwrap could not configure loopback in the new network namespace. \
             Set `network = true` to share the host's, or use `kind = \"docker\"`."
                .into()
        }
        Backend::Docker if stderr.contains("permission denied") => {
            "\n\nThe docker socket is not accessible. Add yourself to the `docker` \
             group, or use `kind = \"bwrap\"`."
                .into()
        }
        Backend::Docker => {
            "\n\nCheck the image exists (`docker pull <image>`) and the daemon is running."
                .into()
        }
        _ => String::new(),
    }
}

fn absolute(path: &Path) -> Result<PathBuf> {
    path.canonicalize()
        .with_context(|| format!("cannot resolve {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(kind: Backend) -> SandboxConfig {
        SandboxConfig { kind, ..SandboxConfig::default() }
    }

    #[test]
    fn a_disabled_sandbox_runs_bash_directly() {
        let sandbox = Sandbox::new(cfg(Backend::None));
        assert!(!sandbox.is_enabled());
        // Nothing is confined, so every capability `shell` declares still holds.
        assert!(sandbox.can_reach_network());
        assert!(sandbox.reaches_beyond_workspace());
    }

    #[test]
    fn confinement_without_network_closes_the_exfiltration_route() {
        let sandbox = Sandbox::new(cfg(Backend::Bwrap));
        assert!(!sandbox.can_reach_network());
        assert!(!sandbox.reaches_beyond_workspace());

        // ...and asking for the network opens it again. This is the whole
        // basis for relaxing the interlock, so it must not be sloppy.
        let sandbox =
            Sandbox::new(SandboxConfig { network: true, ..cfg(Backend::Bwrap) });
        assert!(sandbox.can_reach_network());
    }

    #[test]
    fn a_bind_outside_the_workspace_is_still_reach_beyond_it() {
        let sandbox = Sandbox::new(SandboxConfig {
            readable: vec![PathBuf::from("/opt/toolchain")],
            ..cfg(Backend::Bwrap)
        });
        assert!(
            sandbox.reaches_beyond_workspace(),
            "an extra bind is exactly how private data gets back in reach"
        );
    }

    #[test]
    fn bwrap_confines_the_environment_and_the_network() {
        let workspace = std::env::temp_dir();
        let args = Sandbox::new(cfg(Backend::Bwrap))
            .bwrap_args(&workspace, &workspace)
            .unwrap();

        assert!(args.contains(&"--unshare-net".into()), "no network by default");
        assert!(args.contains(&"--clearenv".into()), "the parent env must not leak");
        assert!(args.contains(&"--unshare-user".into()));
        assert!(args.contains(&"--die-with-parent".into()));
        // Blocks TIOCSTI input injection into the parent's terminal.
        assert!(args.contains(&"--new-session".into()));

        // The workspace is the only writable bind.
        let workspace = workspace.canonicalize().unwrap();
        let binds: Vec<_> = args
            .iter()
            .enumerate()
            .filter(|(_, a)| *a == "--bind")
            .map(|(i, _)| args[i + 1].clone())
            .collect();
        assert_eq!(binds, vec![workspace.display().to_string()]);
    }

    #[test]
    fn network_is_shared_only_when_asked_for() {
        let workspace = std::env::temp_dir();
        let args = Sandbox::new(SandboxConfig { network: true, ..cfg(Backend::Bwrap) })
            .bwrap_args(&workspace, &workspace)
            .unwrap();
        assert!(!args.contains(&"--unshare-net".into()));
    }

    #[test]
    fn docker_drops_privileges_and_the_network() {
        let workspace = std::env::temp_dir();
        let args = Sandbox::new(cfg(Backend::Docker))
            .docker_args(&workspace, &workspace)
            .unwrap();

        assert_eq!(args[0], "run");
        assert!(args.contains(&"--rm".into()), "containers must not accumulate");
        assert!(args.windows(2).any(|w| w[0] == "--network" && w[1] == "none"));
        assert!(args.windows(2).any(|w| w[0] == "--cap-drop" && w[1] == "ALL"));
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--security-opt" && w[1] == "no-new-privileges"));
        // Running as root would leave root-owned files in the user's workspace.
        assert!(args.iter().any(|a| a == "--user"));
    }

    #[test]
    fn a_child_inherits_only_the_base_and_what_was_named() {
        // The leak this closes: `Command::envs()` adds to the inherited
        // environment rather than replacing it, so an MCP server used to start
        // holding every provider key mecha had. Nothing crosses now unless the
        // config named it.
        std::env::set_var("MECHA_TEST_TOKEN", "sk-should-not-cross");
        std::env::set_var("MECHA_TEST_WANTED", "fine");

        let names: Vec<String> =
            Sandbox::child_env(&["MECHA_TEST_WANTED".into()]).into_iter().map(|(k, _)| k).collect();

        assert!(names.contains(&"MECHA_TEST_WANTED".to_string()));
        assert!(
            !names.contains(&"MECHA_TEST_TOKEN".to_string()),
            "an unnamed variable must not reach a third-party process"
        );
        // PATH and HOME are the base: without them most runtimes cannot start,
        // and a server that will not start teaches people to turn this off.
        assert!(names.contains(&"PATH".to_string()));
        assert!(names.contains(&"HOME".to_string()));
    }

    #[test]
    fn wrapping_an_argv_does_not_route_through_a_shell() {
        // A server's args are config, but quoting them into `bash -lc` would
        // still be a command-injection bug waiting for one entry with a space.
        let workspace = std::env::temp_dir();
        let sandbox = Sandbox::new(cfg(Backend::Bwrap));
        let command = sandbox
            .wrap_argv("node", &["server.js".into(), "--flag with space".into()], &workspace, &workspace)
            .unwrap();

        let argv: Vec<_> = command
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(!argv.iter().any(|a| a == "-lc"), "no shell should be involved");
        assert!(argv.contains(&"--flag with space".to_string()), "args stay one argv entry");
    }

    #[test]
    fn only_named_environment_variables_cross_the_boundary() {
        std::env::set_var("MECHA_TEST_ALLOWED", "yes");
        std::env::set_var("MECHA_TEST_SECRET", "no");

        let workspace = std::env::temp_dir();
        let sandbox = Sandbox::new(SandboxConfig {
            env: vec!["MECHA_TEST_ALLOWED".into()],
            ..cfg(Backend::Bwrap)
        });
        let args = sandbox.bwrap_args(&workspace, &workspace).unwrap();

        assert!(args.contains(&"MECHA_TEST_ALLOWED".into()));
        assert!(
            !args.iter().any(|a| a == "MECHA_TEST_SECRET" || a == "no"),
            "an unlisted variable must not cross"
        );
    }
}
