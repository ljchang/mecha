//! `mecha-core` — an agent harness for local models.
//!
//! The library knows nothing about any particular CLI, UI, or project. It gives
//! you four things and lets you wire them together:
//!
//!   * [`provider`] — talk to a model (Anthropic, or anything OpenAI-shaped)
//!   * [`tool`] — things the agent can do, native or [`mcp`]-backed
//!   * [`agent`] — the loop that puts those together
//!   * [`session`] / [`batch`] — persistence and fan-out around the loop
//!
//! ```no_run
//! # async fn example() -> anyhow::Result<()> {
//! use mecha_core::{agent::Agent, agent::Conversation, config::Config};
//! use mecha_core::sandbox::Sandbox;
//! use mecha_core::tool::{ModeApprover, Registry, ToolCtx};
//! use std::sync::Arc;
//!
//! let cfg = Config::load(&std::env::current_dir()?)?;
//! let (_, provider_cfg) = cfg.provider(None)?;
//!
//! // How `shell` is confined. It decides that tool's declared capabilities,
//! // so it is built before the registry rather than consulted at call time.
//! let sandbox = Arc::new(Sandbox::new(cfg.sandbox.clone()));
//!
//! let agent = Agent::new(
//!     mecha_core::provider::build(provider_cfg)?,
//!     Registry::new().with_builtins(&cfg.tools, sandbox),
//!     Arc::new(ModeApprover { mode: cfg.tools.permission_mode }),
//!     ToolCtx {
//!         workspace: std::env::current_dir()?,
//!         shell_timeout: std::time::Duration::from_secs(cfg.tools.shell_timeout_secs),
//!         security: cfg.security.clone(),
//!         ..ToolCtx::default()
//!     },
//!     cfg.agent.clone(),
//!     None,
//! )?;
//!
//! // A conversation carries its own taint, so keeping it across turns keeps
//! // the trifecta interlock honest — see `agent::Conversation`.
//! let mut convo = Conversation::user("What changed in this repo today?");
//! let outcome = agent.run(&mut convo, None).await?;
//! println!("{}", outcome.text);
//! # Ok(())
//! # }
//! ```

pub mod agent;
pub mod backlog;
pub mod batch;
pub mod cache_lens;
pub mod candidate;
pub mod capture;
pub mod compact;
pub mod config;
pub mod counterfactual;
pub mod cron;
pub mod diagnose;
pub mod distill;
pub mod doctor;
pub mod eval;
pub mod frontdoor;
pub mod goal;
pub mod gossip;
pub mod harness;
pub mod homeostat;
pub mod hooks;
pub mod image;
pub mod learning;
pub mod mail_triage;
pub mod mailbox;
pub mod mcp;
pub mod message;
pub mod onboarding;
pub mod outbox;
pub mod outbox_source;
pub mod provider;
pub mod quarantine;
pub mod questions;
pub mod replay;
pub mod replay_run;
pub mod runlog;
pub mod runmarker;
pub mod sandbox;
pub mod search;
pub mod session;
pub mod skill;
pub mod subagent;
pub mod tool;
pub mod trigger;
pub mod work;

pub use agent::{Agent, AgentEvent, RunOutcome};
pub use config::Config;
pub use message::{Block, Effort, Message, Role, StopReason, Usage};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Create a directory (and its parents) and make the leaf owner-only.
///
/// Transcripts, staged outbox drafts, the learning store and spilled tool
/// output all carry the user's private data — mail bodies included, now that
/// mail is wired — so their directories get the rule the mail token files
/// already enforce on themselves (0600). The leaf only, on purpose: parents
/// like `~/.mecha` also hold things the user may deliberately share, and the
/// sensitive data lives below the leaf. Idempotent, and tightens a
/// pre-existing directory too.
/// Is this pid still around? `kill(pid, 0)` checks without delivering
/// anything; `EPERM` means it exists and is not ours, which still counts.
///
/// The range check is not defensive padding — it is the whole correctness of
/// the function. `kill(2)` gives non-positive pids entirely different
/// meanings: `0` is "every process in my group", `-1` is "every process I may
/// signal" (which succeeds, always), and any other negative is a process
/// group. A corrupt marker holding one of those would report a long-dead run
/// as alive and leave whatever owns the marker looking permanently busy in
/// every UI that asks. Found by a test using `u32::MAX`, which sign-flips to exactly the
/// `-1` case.
pub fn process_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    // SAFETY: signal 0 delivers nothing and only probes for the process.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().kind() == std::io::ErrorKind::PermissionDenied
}

pub fn create_private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod process_alive_tests {
    /// The case that found the bug, kept beside the function now that more
    /// than one subsystem depends on it: `u32::MAX` sign-flips to `-1`, which
    /// `kill(2)` reads as "every process I may signal" and answers yes to.
    #[test]
    fn a_pid_that_is_not_a_pid_is_never_alive() {
        assert!(!super::process_alive(u32::MAX));
        assert!(!super::process_alive(0), "0 means my whole process group");
        assert!(
            !super::process_alive(i32::MAX as u32),
            "real-looking and far above any pid_max"
        );
        assert!(
            super::process_alive(std::process::id()),
            "and the negative is not vacuous: we are alive"
        );
    }
}
