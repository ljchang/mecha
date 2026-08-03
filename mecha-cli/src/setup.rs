//! Turning CLI flags plus config into a ready-to-run [`Agent`].
//!
//! Every command that runs an agent goes through here, so precedence is
//! defined once: config file, then environment, then flags.

use crate::approve::TerminalApprover;
use crate::GlobalOpts;
use anyhow::{Context, Result};
use mecha_core::agent::Agent;
use mecha_core::config::{Config, PermissionMode};
use mecha_core::config::SearchBackendConfig;
use mecha_core::mcp::{self, McpClient};
use mecha_core::search::{Exa, SearchBackend, SearchChain, Searxng, Tavily, WebSearch};
use mecha_core::subagent::{Subagent, SubagentProfile};
use mecha_core::tool::{Approver, ModeApprover, Registry, ToolCtx};
use std::path::PathBuf;
use std::sync::Arc;

pub struct Prepared {
    pub agent: Agent,
    pub provider_name: String,
    pub model: String,
    pub workspace: PathBuf,
    /// The resolved config, for commands that need to build a *second*
    /// connection — `eval` and its judge model.
    pub config: Config,
    /// Held for the lifetime of the run: dropping a client kills its server.
    pub _mcp: Vec<Arc<McpClient>>,
}

/// Everything except the model connection. Split out so `mecha tools` can list
/// what an agent *would* have without needing provider credentials.
pub struct PreparedTools {
    pub registry: Registry,
    pub sandbox: Arc<mecha_core::sandbox::Sandbox>,
    pub workspace: PathBuf,
    pub config: Config,
    pub approver: Arc<dyn Approver>,
    pub _mcp: Vec<Arc<McpClient>>,
}

/// Build an agent. `interactive` decides whether an un-approved tool call can
/// prompt a human or must fall back to the configured [`PermissionMode`].
pub async fn prepare(opts: &GlobalOpts, interactive: bool) -> Result<Prepared> {
    build(prepare_tools(opts, interactive).await?, opts)
}

/// Build an agent that asks a caller-supplied approver.
///
/// The TUI needs this: its approver talks to the event loop over a channel, and
/// `prepare` would otherwise install one that writes prompts straight to a
/// terminal the interface has taken over. The approver is still only consulted
/// in `Ask` mode — a run configured read-only stays read-only.
pub async fn prepare_with_approver(
    opts: &GlobalOpts,
    approver: Arc<dyn Approver>,
) -> Result<Prepared> {
    let mut tools = prepare_tools(opts, true).await?;
    if tools.config.tools.permission_mode == PermissionMode::Ask {
        tools.approver = approver;
    }
    build(tools, opts)
}

fn build(tools: PreparedTools, opts: &GlobalOpts) -> Result<Prepared> {
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

    // Subagents are built from the same tool pool but get their own registry —
    // an allowlist, not an inheritance. Do this before the parent takes
    // ownership of the registry.
    let mut registry = tools.registry;
    for profile in &cfg.subagents {
        // A profile may point at a different provider entry entirely.
        let (_, child_provider_cfg) = cfg.provider(profile.provider.as_deref())?;
        let child = build_subagent(profile, &registry, &cfg, child_provider_cfg, &ctx)?;
        registry.insert(Arc::new(child));
    }

    let agent = Agent::new(
        provider,
        registry,
        tools.approver,
        ctx,
        cfg.agent.clone(),
        Some(model.clone()),
    )?
    .with_pricing(provider_cfg.pricing());

    Ok(Prepared {
        agent,
        provider_name,
        model,
        workspace: tools.workspace,
        config: cfg,
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
    if opts.max_output_tokens.is_some() {
        cfg.agent.max_output_tokens = opts.max_output_tokens;
    }
    if opts.max_cost.is_some() {
        cfg.agent.max_cost_usd = opts.max_cost;
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
    let sandbox = Arc::new(mecha_core::sandbox::Sandbox::new(cfg.sandbox.clone()));
    let mut registry = Registry::new().with_builtins(&cfg.tools, Arc::clone(&sandbox));

    // Search is only registered when a backend is configured — an agent with a
    // `web_search` tool that always errors is worse than no tool at all.
    if !cfg.search.is_empty() {
        let (chain, errors) = build_search_chain(&cfg.search);
        for error in errors {
            eprintln!("mecha: search backend unavailable — {error}");
        }
        if !chain.is_empty() {
            let allowed = opts.tools.is_empty() || opts.tools.iter().any(|t| t == "web_search");
            if allowed {
                registry.insert(Arc::new(WebSearch::new(Arc::new(chain))));
            }
        }
    }
    let mut clients = Vec::new();
    if !opts.no_mcp && !cfg.mcp.is_empty() {
        let (tools, connected, errors) = mcp::connect_all(&cfg.mcp, &sandbox, &workspace).await;
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

    // Prove the sandbox works before the agent can call anything, and refuse
    // to start if it doesn't. Falling back to unconfined execution would be
    // worse than never having configured one: `shell` declares narrower
    // capabilities when confined, and the loop's interlock trusts that claim.
    if sandbox.is_enabled() && registry.get("shell").is_some() {
        sandbox
            .preflight(&workspace)
            .await
            .context("sandbox preflight failed — refusing to run `shell` unconfined")?;
    }

    Ok(PreparedTools { registry, sandbox, workspace, config: cfg, approver, _mcp: clients })
}

/// Build the search chain in configured order, skipping backends that cannot
/// be constructed (usually a missing key) rather than failing the whole run.
fn build_search_chain(configs: &[SearchBackendConfig]) -> (SearchChain, Vec<String>) {
    let mut backends: Vec<Box<dyn SearchBackend>> = Vec::new();
    let mut errors = Vec::new();

    for cfg in configs.iter().filter(|c| !c.disabled) {
        let built: Result<Box<dyn SearchBackend>> = match cfg.kind.as_str() {
            "exa" => cfg
                .resolve_api_key()
                .context("no API key (set api_key_env, e.g. EXA_API_KEY)")
                .and_then(|k| Ok(Box::new(Exa::new(k, cfg.base_url.clone())?) as Box<dyn SearchBackend>)),
            "tavily" => cfg
                .resolve_api_key()
                .context("no API key (set api_key_env, e.g. TAVILY_API_KEY)")
                .and_then(|k| {
                    Ok(Box::new(Tavily::new(k, cfg.base_url.clone())?) as Box<dyn SearchBackend>)
                }),
            "searxng" => cfg
                .base_url
                .clone()
                .context("searxng needs `base_url` pointing at your instance")
                .and_then(|u| Ok(Box::new(Searxng::new(u)?) as Box<dyn SearchBackend>)),
            other => Err(anyhow::anyhow!(
                "unknown search backend {other:?} (expected: exa, tavily, searxng)"
            )),
        };

        match built {
            Ok(b) => backends.push(b),
            Err(e) => errors.push(format!("{}: {e}", cfg.kind)),
        }
    }

    (SearchChain::new(backends), errors)
}

/// Build one subagent: a child [`Agent`] with a restricted registry, wrapped as
/// a tool the parent can call.
fn build_subagent(
    profile: &SubagentProfile,
    pool: &Registry,
    cfg: &Config,
    provider_cfg: &mecha_core::config::ProviderConfig,
    ctx: &ToolCtx,
) -> Result<Subagent> {
    let mut child_registry = Registry::new();
    for wanted in &profile.tools {
        match pool.get(wanted) {
            Some(tool) => child_registry.insert(Arc::clone(tool)),
            // A typo here silently produces a child that cannot do its job, so
            // say so rather than starting a crippled agent.
            None => anyhow::bail!(
                "subagent `{}` asks for tool `{wanted}`, which is not available. \
                 Available: {}",
                profile.name,
                pool.iter().map(|t| t.name()).collect::<Vec<_>>().join(", ")
            ),
        }
    }

    // A child cannot prompt anyone, so `ask` degrades to read-only rather than
    // to a blanket denial that would make the child useless.
    let mode = match cfg.tools.permission_mode {
        PermissionMode::Ask => PermissionMode::ReadOnly,
        other => other,
    };

    let mut child_cfg = cfg.agent.clone();
    child_cfg.max_turns = profile.max_turns;
    child_cfg.system_prompt = profile.system_prompt.clone();
    child_cfg.system_prompt_file = None;

    let child = Agent::new(
        mecha_core::provider::build(provider_cfg)?,
        child_registry,
        Arc::new(ModeApprover { mode }),
        ToolCtx {
            workspace: ctx.workspace.clone(),
            shell_timeout: ctx.shell_timeout,
            security: ctx.security.clone(),
        },
        child_cfg,
        // Profile model wins; otherwise the child uses its provider's default.
        profile.model.clone().or_else(|| provider_cfg.model.clone()),
    )?;

    Ok(Subagent::new(profile.clone(), Arc::new(child)))
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
