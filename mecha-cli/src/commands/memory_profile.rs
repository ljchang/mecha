//! Shared fail-closed profile for explicit ordinary memory surfaces.
use crate::GlobalOpts;
use anyhow::{Context, Result};
use mecha_core::{config::Config, memory_session::SessionMemory, session::SessionKind};
use std::path::Path;
pub(crate) fn validate_memory_config(
    global: &GlobalOpts,
    config: &mecha_core::config::Config,
) -> Result<()> {
    use mecha_core::{mailbox::InboundPolicy, provider::router, tool::profile::ToolProfile};

    let (_, provider) = config.provider(global.provider.as_deref())?;
    anyhow::ensure!(
        global.provider.is_some() || global.model.is_some() || !router::follows_here(provider),
        "ordinary memory needs an explicit --provider or --model with router following enabled"
    );
    anyhow::ensure!(
        global.no_fallback || provider.fallbacks.is_empty(),
        "ordinary memory requires no provider fallbacks (use --no-fallback)"
    );
    anyhow::ensure!(
        global.no_hooks || config.hooks.is_empty(),
        "ordinary memory requires no effective hooks"
    );
    // Owner-only config loading normally defaults inbound mail to Accept.
    // A memory run must preserve its one-shot boundary.
    anyhow::ensure!(
        global.no_messages
            || !config.messages.on()
            || matches!(
                config.messages.inbound,
                Some(InboundPolicy::Hold | InboundPolicy::Refuse)
            ),
        "ordinary memory requires held or disabled inbound messages"
    );
    anyhow::ensure!(
        !matches!(
            global.tool_profile,
            Some(ToolProfile::Research | ToolProfile::Coding)
        ) && global.tools.iter().all(|name| config.tools.registers(name)),
        "ordinary memory requires all selected tools to be enabled, including goal_context"
    );
    // Setup exposes a child whenever its tool list fits the parent allowlist,
    // even when the child's name was not selected. Its name can shadow a builtin.
    anyhow::ensure!(
        !config
            .subagents
            .iter()
            .any(|child| { child.tools.iter().all(|name| global.tools.contains(name)) }),
        "ordinary memory requires no effective subagents"
    );
    anyhow::ensure!(
        global.no_outbox
            || !global
                .tools
                .iter()
                .any(|name| config.outbox.tools.contains(name)),
        "ordinary memory requires direct tools; the outbox routes a selected tool"
    );
    Ok(())
}

pub(crate) fn open_profile(global: &mut GlobalOpts, path: &Path) -> Result<SessionMemory> {
    // Check the merged options before configuration or provider startup.
    anyhow::ensure!(
        global.no_mcp && global.no_code_tool && global.no_grants && !global.tools.is_empty(),
        "ordinary memory requires --no-mcp, --no-code-tool, --no-grants, and explicit --tool selection"
    );
    anyhow::ensure!(
        SessionKind::test_override().is_none(),
        "ordinary memory requires a fresh run without a session-kind override"
    );
    let cwd = std::env::current_dir().context("cannot determine the working directory")?;
    match std::fs::symlink_metadata(cwd.join(Config::PROJECT_FILE)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("ordinary memory cannot inspect project config"),
        Ok(_) => anyhow::bail!("ordinary memory refuses a project mecha.toml"),
    }
    let config = Config::load_global()?;
    validate_memory_config(global, &config)?;
    let workspace = global
        .workspace
        .as_ref()
        .or(config.tools.workspace.as_ref())
        .unwrap_or(&cwd);
    let workspace = workspace
        .canonicalize()
        .context("ordinary memory workspace unavailable")?;
    mecha_core::work::ensure_outside_mecha_home(&workspace)?;
    let path = std::path::absolute(path)?;
    SessionMemory::validate_request_path(&path, &workspace)?;
    let request = SessionMemory::load_request(&path)?;
    let memory = SessionMemory::open(&request, &workspace)?;
    memory.validate_surface(&global.tools, &workspace, &[])?;
    global.global_config_only = true;
    Ok(memory)
}
