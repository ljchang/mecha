//! `mecha-core` — a standalone agent harness.
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
pub mod batch;
pub mod compact;
pub mod config;
pub mod eval;
pub mod mcp;
pub mod message;
pub mod provider;
pub mod replay;
pub mod replay_run;
pub mod sandbox;
pub mod search;
pub mod session;
pub mod subagent;
pub mod tool;

pub use agent::{Agent, AgentEvent, RunOutcome};
pub use config::Config;
pub use message::{Block, Effort, Message, Role, StopReason, Usage};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
