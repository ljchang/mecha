//! Shared fail-closed profile for explicit ordinary memory surfaces.
use crate::GlobalOpts;
use anyhow::{Context, Result};
use mecha_core::{config::Config, memory_session::SessionMemory, session::SessionKind};
use std::path::Path;

pub(crate) struct GraphSelection {
    pub server: mecha_core::config::McpServerConfig,
    pub remote_names: std::collections::BTreeSet<String>,
    pub local_names: Vec<String>,
}

pub(crate) struct ChatProfilePreparation {
    pub memory: SessionMemory,
    pub graph: Option<GraphSelection>,
}

pub(crate) fn validate_memory_config(
    global: &GlobalOpts,
    config: &mecha_core::config::Config,
) -> Result<()> {
    validate_memory_config_with_graph(global, config, &[])
}

pub(crate) fn validate_memory_config_with_graph(
    global: &GlobalOpts,
    config: &Config,
    graph_names: &[String],
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
        ) && global
            .tools
            .iter()
            .all(|name| graph_names.contains(name) || config.tools.registers(name)),
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

/// An ordinary session's memory, a one-shot run's or a chat's: the store
/// named by the manifest, opened for the workspace the session will use.
/// Which tools, servers, hooks and messages it has is its own configuration
/// and no condition of remembering it; the recorder stores what each tool
/// returned beside where it came from, and leaves out what it cannot place.
pub(crate) fn open_run_profile(global: &GlobalOpts, path: &Path) -> Result<SessionMemory> {
    // The recorder reads a `Run` header; a session marked as a test is
    // excluded from every corpus and is not remembered either. A run an
    // experiment starts is remembered, in its trial home's own store: that
    // is what a memory arm measures (`experiment::ArmMemory`).
    // The mark alone can be set by hand, which would hide a session from
    // every corpus while it wrote into the operator's own store. A trial is
    // the mark and the driver's reference together (review of #875).
    anyhow::ensure!(
        match SessionKind::test_override() {
            None => true,
            Some(SessionKind::Experiment) =>
                mecha_core::experiment::ExperimentRef::from_env().is_some(),
            Some(_) => false,
        },
        "ordinary memory requires a fresh run without a session-kind override"
    );
    let cwd = std::env::current_dir().context("cannot determine the working directory")?;
    let config = if global.global_config_only {
        Config::load_global()?
    } else {
        Config::load(&cwd)?
    };
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
    SessionMemory::open(&request, &workspace)
}

pub(crate) fn open_chat_profile(
    global: &mut GlobalOpts,
    path: &Path,
    graph_server: Option<&str>,
    task: bool,
) -> Result<ChatProfilePreparation> {
    if graph_server.is_none() && !task {
        return Ok(ChatProfilePreparation {
            memory: open_run_profile(global, path)?,
            graph: None,
        });
    }
    // The graph and task profiles are closed: check the merged options
    // before configuration or provider startup.
    anyhow::ensure!(
        global.no_code_tool && global.no_grants && !global.tools.is_empty(),
        "ordinary memory requires --no-code-tool, --no-grants, and explicit --tool selection"
    );
    anyhow::ensure!(
        global.no_mcp == graph_server.is_none(),
        "ordinary memory requires --no-mcp unless a chat explicitly selects --memory-graph-server"
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
    let graph = graph_server
        .map(|name| select_graph(global, &config, name))
        .transpose()?;
    let graph_names = graph.as_ref().map_or(&[][..], |g| g.local_names.as_slice());
    validate_memory_config_with_graph(global, &config, graph_names)?;
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
    let local_tools: Vec<_> = global
        .tools
        .iter()
        .filter(|name| !graph_names.contains(name))
        .cloned()
        .collect();
    let memory = if task {
        anyhow::ensure!(
            graph_server.is_none(),
            "task chat does not admit graph tools"
        );
        SessionMemory::validate_request_path(&path, &workspace)?;
        let request = SessionMemory::load_request(&path)?;
        let memory = SessionMemory::open(&request, &workspace)?;
        memory.validate_task_surface(&local_tools, &workspace, &[])?;
        memory
    } else {
        open_memory_at(&path, &workspace, &local_tools)?
    };
    global.global_config_only = true;
    Ok(ChatProfilePreparation { memory, graph })
}

/// Workspace-explicit local profile shared by CLI-independent hosts.
pub(crate) fn open_local_profile_at(
    global: &GlobalOpts,
    config: &Config,
    path: &Path,
    workspace: &Path,
) -> Result<SessionMemory> {
    anyhow::ensure!(
        global.no_mcp && global.no_code_tool && global.no_grants && !global.tools.is_empty(),
        "memory requires an explicit local tool surface"
    );
    validate_memory_config(global, config)?;
    mecha_core::work::ensure_outside_mecha_home(workspace)?;
    open_memory_at(path, workspace, &global.tools)
}

fn open_memory_at(path: &Path, workspace: &Path, tools: &[String]) -> Result<SessionMemory> {
    SessionMemory::validate_request_path(path, workspace)?;
    let request = SessionMemory::load_request(path)?;
    let memory = SessionMemory::open(&request, workspace)?;
    memory.validate_surface(tools, workspace, &[])?;
    Ok(memory)
}

pub(crate) fn validate_memory_runtime(
    opts: &GlobalOpts,
    prepared: &crate::setup::Prepared,
    memory: &mecha_core::memory_session::chat::ChatMemory,
    graph_names: &[String],
) -> Result<()> {
    if memory.is_open() {
        // An ordinary chat: its tools are its own configuration. What is
        // checked is that the workspace is the one the store was opened
        // for and that no grant reaches the store or the transcript. The
        // recall tool is installed after this, whether or not selected.
        let mut names: Vec<String> = prepared
            .agent
            .registry()
            .iter()
            .map(|tool| tool.name().into())
            .collect();
        if !names.iter().any(|name| name == "goal_context") {
            names.push("goal_context".into());
        }
        return memory.validate_surface(
            &names,
            &prepared.workspace,
            &prepared.agent.context().tools.grants.all(),
        );
    }
    validate_memory_config_with_graph(opts, &prepared.config, graph_names)?;
    let cx = prepared.agent.context();
    anyhow::ensure!(
        cx.hooks.is_empty(),
        "chat memory requires no effective hooks"
    );
    anyhow::ensure!(
        !prepared
            .agent
            .registry()
            .iter()
            .any(|tool| tool.runs_a_fresh_conversation() || tool.defers() || tool.hosts_programs()),
        "chat memory requires direct tools"
    );
    let names: Vec<String> = prepared
        .agent
        .registry()
        .iter()
        .map(|tool| tool.name().into())
        .collect();
    memory.validate_surface(&names, &prepared.workspace, &cx.tools.grants.all())?;
    anyhow::ensure!(
        names.len() == opts.tools.len() && names.iter().all(|n| opts.tools.contains(n)),
        "chat memory final registry differs from selected tools"
    );
    Ok(())
}

fn select_graph(global: &GlobalOpts, config: &Config, name: &str) -> Result<GraphSelection> {
    use mecha_core::feature::{self, Feature};
    anyhow::ensure!(
        !global
            .no_mcp_servers
            .iter()
            .any(|disabled| disabled == name),
        "selected memory graph server is disabled"
    );
    let matching: Vec<_> = config
        .mcp
        .iter()
        .filter(|server| server.name == name)
        .collect();
    anyhow::ensure!(
        matching.len() == 1,
        "memory graph needs one exact configured server"
    );
    let server = matching[0];
    anyhow::ensure!(
        server.from_operator
            && !server.disabled
            && feature::server_feature(server) == Some(Feature::Graph)
            && feature::switched_on(config, Feature::Graph),
        "memory graph requires an enabled owner-configured graph server"
    );
    let unique: std::collections::BTreeSet<_> = global.tools.iter().collect();
    anyhow::ensure!(
        unique.len() == global.tools.len(),
        "memory tools must be unique"
    );
    let mut remote_names = std::collections::BTreeSet::new();
    let mut local_names = Vec::new();
    for remote in ["kg_search", "kg_entity", "kg_task_list"] {
        let local = if server.prefix_tools == Some(false) {
            remote.to_owned()
        } else {
            format!("{}__{remote}", server.name)
        };
        if global.tools.contains(&local) {
            anyhow::ensure!(
                !config.tools.disabled.contains(&local)
                    && !server
                        .disabled_tools
                        .iter()
                        .any(|disabled| disabled == remote),
                "selected memory graph tool is disabled"
            );
            remote_names.insert(remote.to_owned());
            local_names.push(local);
        }
    }
    anyhow::ensure!(
        !remote_names.is_empty(),
        "select a supported read-only graph tool"
    );
    local_names.sort();
    Ok(GraphSelection {
        server: server.clone(),
        remote_names,
        local_names,
    })
}

#[cfg(test)]
mod graph_selection_tests {
    use super::*;
    use clap::Parser;
    use mecha_core::config::McpServerConfig;

    fn fixture(prefix: bool) -> (GlobalOpts, Config) {
        let graph_tool = if prefix {
            "chosen__kg_search"
        } else {
            "kg_search"
        };
        let opts = crate::Cli::try_parse_from([
            "mecha",
            "chat",
            "--tool",
            "goal_context",
            "--tool",
            graph_tool,
        ])
        .unwrap()
        .global;
        let mut config = Config::default();
        config.features.0.insert("graph".into(), true);
        config.mcp.push(McpServerConfig {
            name: "chosen".into(),
            command: "/authored/mecha-graph-mcp".into(),
            from_operator: true,
            prefix_tools: Some(prefix),
            ..Default::default()
        });
        (opts, config)
    }

    #[test]
    fn graph_selection_uses_exact_owner_mapping_and_closed_read_surface() {
        for prefix in [false, true] {
            let (opts, config) = fixture(prefix);
            let selected = select_graph(&opts, &config, "chosen").unwrap();
            assert_eq!(selected.remote_names, ["kg_search".to_owned()].into());
            assert_eq!(selected.local_names, vec![opts.tools[1].clone()]);
            assert!(select_graph(&opts, &config, "other").is_err());
            let mut wrong = opts.clone();
            wrong.tools[1] = "other__kg_search".into();
            assert!(select_graph(&wrong, &config, "chosen").is_err());
            wrong.tools[1] = if prefix {
                "chosen__kg_upsert"
            } else {
                "kg_upsert"
            }
            .into();
            assert!(select_graph(&wrong, &config, "chosen").is_err());
        }
    }

    #[test]
    fn graph_selection_refuses_disabled_ambiguous_and_project_authority() {
        for variant in 0..8 {
            let (mut opts, mut config) = fixture(true);
            match variant {
                0 => config.mcp[0].from_operator = false,
                1 => config.mcp[0].disabled = true,
                2 => {
                    config.features.0.insert("graph".into(), false);
                }
                3 => {
                    config.mcp.push(config.mcp[0].clone());
                }
                4 => opts.no_mcp_servers.push("chosen".into()),
                5 => config.mcp[0].disabled_tools.push("kg_search".into()),
                6 => config.tools.disabled.push("chosen__kg_search".into()),
                _ => opts.tools.push("chosen__kg_search".into()),
            }
            assert!(
                select_graph(&opts, &config, "chosen").is_err(),
                "accepted {variant}"
            );
        }
    }
}
