//! Turning CLI flags plus config into a ready-to-run [`Agent`].
//!
//! Every command that runs an agent goes through here, so precedence is
//! defined once: config file, then environment, then flags.

use crate::approve::TerminalApprover;
use crate::GlobalOpts;
use anyhow::{Context, Result};
use mecha_core::agent::Agent;
use mecha_core::config::{Config, PermissionMode};
use mecha_core::mcp::{self, McpClient};
use mecha_core::tool::{Approver, ModeApprover, Registry, ToolCtx};
use std::path::PathBuf;
use std::sync::Arc;

pub struct Prepared {
    pub agent: Agent,
    pub provider_name: String,
    pub model: String,
    pub workspace: PathBuf,
    /// Held for the lifetime of the run: dropping a client kills its server.
    pub _mcp: Vec<Arc<McpClient>>,
}

/// Everything except the model connection. Split out so `mecha tools` can list
/// what an agent *would* have without needing provider credentials.
pub struct PreparedTools {
    pub registry: Registry,
    pub workspace: PathBuf,
    pub config: Config,
    pub approver: Arc<dyn Approver>,
    pub _mcp: Vec<Arc<McpClient>>,
}

/// Build an agent. `interactive` decides whether an un-approved tool call can
/// prompt a human or must fall back to the configured [`PermissionMode`].
pub async fn prepare(opts: &GlobalOpts, interactive: bool) -> Result<Prepared> {
    let tools = prepare_tools(opts, interactive).await?;
    let cfg = tools.config;

    let (provider_name, provider_cfg) = cfg.provider(opts.provider.as_deref())?;
    let provider = mecha_core::provider::build(provider_cfg)?;

    let model = opts
        .model
        .clone()
        .or_else(|| provider_cfg.model.clone())
        .unwrap_or_else(|| provider.default_model().to_string());

    let ctx = ToolCtx {
        workspace: tools.workspace.clone(),
        shell_timeout: std::time::Duration::from_secs(cfg.tools.shell_timeout_secs),
        security: cfg.security.clone(),
    };

    let agent = Agent::new(
        provider,
        tools.registry,
        tools.approver,
        ctx,
        cfg.agent.clone(),
        Some(model.clone()),
    )?;

    Ok(Prepared {
        agent,
        provider_name,
        model,
        workspace: tools.workspace,
        _mcp: tools._mcp,
    })
}

/// Resolve config, workspace, tools, and the approval policy.
pub async fn prepare_tools(opts: &GlobalOpts, interactive: bool) -> Result<PreparedTools> {
    let cwd = std::env::current_dir().context("cannot determine the working directory")?;
    let mut cfg = Config::load(&cwd)?;

    // --- flags override config ---
    if let Some(effort) = opts.effort {
        cfg.agent.effort = Some(effort);
    }
    if let Some(max_turns) = opts.max_turns {
        cfg.agent.max_turns = max_turns;
    }
    if opts.no_thinking {
        cfg.agent.thinking = false;
        // Disabling thinking above `high` effort is rejected by the API. The
        // user asked for no thinking, so honour that and cap the effort.
        if matches!(
            cfg.agent.effort,
            Some(mecha_core::Effort::XHigh) | Some(mecha_core::Effort::Max)
        ) {
            cfg.agent.effort = Some(mecha_core::Effort::High);
        }
    }
    if let Some(system) = &opts.system {
        cfg.agent.system_prompt = Some(read_maybe_file(system)?);
        cfg.agent.system_prompt_file = None;
    }
    if !opts.tools.is_empty() {
        cfg.tools.enabled = opts.tools.clone();
    }
    if opts.yes {
        cfg.tools.permission_mode = PermissionMode::Allow;
    }
    if opts.read_only {
        cfg.tools.permission_mode = PermissionMode::ReadOnly;
    }

    let workspace = opts
        .workspace
        .clone()
        .or_else(|| cfg.tools.workspace.clone())
        .unwrap_or(cwd);
    let workspace = workspace
        .canonicalize()
        .with_context(|| format!("workspace {} does not exist", workspace.display()))?;

    // --- tools ---
    let mut registry = Registry::new().with_builtins(&cfg.tools);
    let mut clients = Vec::new();
    if !opts.no_mcp && !cfg.mcp.is_empty() {
        let (tools, connected, errors) = mcp::connect_all(&cfg.mcp).await;
        for error in errors {
            // A dead server is worth saying out loud, but it shouldn't stop the
            // run — the other tools still work.
            eprintln!("mecha: MCP server unavailable — {error}");
        }
        for tool in tools {
            // `--tool` filters MCP tools too, so a run can be narrowed to
            // exactly one remote capability.
            if opts.tools.is_empty() || opts.tools.iter().any(|t| t == tool.name()) {
                registry.insert(tool);
            }
        }
        clients = connected;
    }

    let approver: Arc<dyn Approver> =
        if interactive && cfg.tools.permission_mode == PermissionMode::Ask {
            Arc::new(TerminalApprover::default())
        } else {
            Arc::new(ModeApprover { mode: cfg.tools.permission_mode })
        };

    Ok(PreparedTools { registry, workspace, config: cfg, approver, _mcp: clients })
}

/// `@path` reads from a file; anything else is the literal value. Lets
/// `--system @prompts/reviewer.md` work without a second flag.
pub fn read_maybe_file(value: &str) -> Result<String> {
    match value.strip_prefix('@') {
        Some(path) => std::fs::read_to_string(path)
            .with_context(|| format!("reading system prompt from {path}")),
        None => Ok(value.to_string()),
    }
}
