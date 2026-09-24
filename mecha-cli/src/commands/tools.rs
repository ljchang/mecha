//! `mecha tools` — what the agent can actually do right now.
//!
//! Deliberately does not build a provider: you should be able to see (and
//! debug) your tool surface before any credentials are configured.

use crate::{setup, GlobalOpts};
use anyhow::Result;

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Print the full JSON schema for each tool, exactly as the model sees it.
    #[arg(long)]
    pub schema: bool,

    /// Emit JSON instead of a table.
    #[arg(long)]
    pub json: bool,
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    // Connects to MCP servers, so this doubles as a check that they start.
    let mut prepared = setup::prepare_tools(global, false).await?;
    // The *model-facing* surface, which is what this command exists to
    // audit ("ask the artifact, don't assert"): `setup::build` guards
    // `kg_task_update` on every agent registry, and without the same guard
    // here the audit view printed the unguarded description. This mirror
    // cannot itself detect the guard's absence from `build` — what makes
    // the mirror honest is `closure_guard::verify`, which `build` runs on
    // every start and which refuses to hand an unguarded surface to any
    // agent, so the surface this prints is the only one that can exist.
    // Applied to this process's own local registry only; `tasks set`'s
    // separate `prepare_tools` instance is untouched as ever.
    crate::closure_guard::guard(&mut prepared.registry);
    let registry = &prepared.registry;

    let outbox_routed =
        |name: &str| !global.no_outbox && prepared.config.outbox.tools.iter().any(|t| t == name);
    // Servers the operator vouches for (`[[mcp]] trust_result_claims`), named
    // here because the switch trusts a server more rather than less and must
    // stay one visible decision (docs/PROVENANCE-DESIGN.md §3). A prefixed
    // server's tools carry its name; an unprefixed one's cannot be attributed
    // by name, so it is named in the header only.
    let vouched: Vec<&mecha_core::config::McpServerConfig> = prepared
        .config
        .mcp
        .iter()
        .filter(|s| s.trust_result_claims && !s.disabled)
        .collect();
    let claims_believed = |name: &str| {
        vouched
            .iter()
            .any(|s| s.prefix_tools != Some(false) && name.starts_with(&format!("{}__", s.name)))
    };
    // With a vouched server whose tools are unprefixed, a tool not
    // attributable to a prefixed one *might* be that server's: its answer is
    // unknown, and `--json` says `null` rather than a `false` a script would
    // read as "no" (review of #290 — a dash is never zero).
    let unattributable = vouched.iter().any(|s| s.prefix_tools == Some(false));

    if args.json {
        let specs: Vec<_> = registry
            .iter()
            .map(|t| {
                // Capabilities are in the JSON because this is the auditable
                // view: `shell` declaring `external_send: false` is a claim the
                // sandbox is making, and it should be inspectable without
                // reading source.
                let caps = t.capabilities();
                serde_json::json!({
                    "name": t.name(),
                    "description": t.description(),
                    "read_only": t.read_only(),
                    "outbox_routed": outbox_routed(t.name()),
                    // This tool's server is believed when it says a failed
                    // call dispatched nothing — see `trust_result_claims`.
                    "result_claims_believed": if claims_believed(t.name()) {
                        serde_json::Value::Bool(true)
                    } else if unattributable {
                        serde_json::Value::Null
                    } else {
                        serde_json::Value::Bool(false)
                    },
                    "capabilities": {
                        "private_data": caps.private_data,
                        "untrusted_input": caps.untrusted_input,
                        // Kept, and kept meaning "can data leave at all",
                        // so a consumer reading this field still reads the
                        // truth. `egress` beside it is the part the bool
                        // could not say: who picks the recipient.
                        "external_send": caps.can_send(),
                        "egress": caps.egress.as_str(),
                        "destructive": caps.destructive,
                    },
                    "input_schema": t.input_schema(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&specs)?);
        return Ok(());
    }

    if registry.is_empty() {
        println!("no tools registered");
        return Ok(());
    }

    for s in &vouched {
        println!(
            "result claims believed from `{}` (trust_result_claims){}",
            s.name,
            if s.prefix_tools == Some(false) {
                " — its tools are unprefixed, so not marked below"
            } else {
                ""
            }
        );
    }
    if !vouched.is_empty() {
        println!();
    }

    for tool in registry.iter() {
        let access = if tool.read_only() {
            "read-only"
        } else {
            "writes"
        };
        let routing = if outbox_routed(tool.name()) {
            " · outbox: staged for review"
        } else {
            ""
        };
        let claims = if claims_believed(tool.name()) {
            " · result claims believed"
        } else {
            ""
        };
        println!("{}  [{}{}{}]", tool.name(), access, routing, claims);
        for line in tool.description().lines() {
            println!("    {line}");
        }
        if args.schema {
            let schema = serde_json::to_string_pretty(&tool.input_schema())?;
            for line in schema.lines() {
                println!("    {line}");
            }
        }
        println!();
    }

    println!(
        "{} tools · workspace {}",
        registry.len(),
        prepared.workspace.display()
    );

    // The sandbox decides what `shell` actually is, so say so plainly. An
    // operator who thinks commands are confined when they aren't is the exact
    // failure this whole subsystem exists to prevent.
    if registry.get("shell").is_some() {
        println!("\nshell {}", setup::sandbox_line(&prepared.sandbox));
        if !prepared.sandbox.is_enabled() {
            println!(
                "  the path jail does not cover them. \
                 Set [sandbox] kind = \"bwrap\" (or \"docker\") to confine them."
            );
        }
    }

    // An MCP server is somebody else's code on your machine. Say which ones
    // are confined and which are not, because that is the fact an operator
    // most needs and is least likely to remember.
    if !prepared.config.mcp.is_empty() {
        println!("\nmcp servers");
        for server in prepared.config.mcp.iter().filter(|s| !s.disabled) {
            let confinement = if !server.sandbox {
                "unconfined — runs as you".to_string()
            } else if !prepared.sandbox.is_enabled() {
                // It refuses to start in this state, so don't describe a
                // confinement it does not have.
                "will not start — asks for confinement, no backend set".to_string()
            } else {
                let network = server
                    .network
                    .unwrap_or(prepared.sandbox.can_reach_network());
                format!(
                    "{} · network {}",
                    prepared.sandbox.backend().as_str(),
                    if network { "on" } else { "off" }
                )
            };
            println!("  {}  →  {}", server.name, confinement);
            if !server.env_passthrough.is_empty() {
                println!(
                    "      env passed through: {}",
                    server.env_passthrough.join(", ")
                );
            }
            if server.sandbox && !prepared.sandbox.is_enabled() {
                println!("      ⚠ set [sandbox] kind, or drop `sandbox = true`");
            }
        }
    }

    // Subagents need a provider to build, which this command deliberately does
    // not require. List the profiles from config instead — that also shows the
    // capability boundary, which is the thing worth eyeballing.
    if !prepared.config.subagents.is_empty() {
        println!("\nsubagents");
        for profile in &prepared.config.subagents {
            let granted = if profile.tools.is_empty() {
                "(no tools)".to_string()
            } else {
                profile.tools.join(", ")
            };
            let model = profile.model.as_deref().unwrap_or("(inherits)");
            println!("  {}  →  {}", profile.name, granted);
            println!("      model {model} · max {} turns", profile.max_turns);

            // Warn about a profile that grants both halves of the trifecta:
            // isolation you did not actually get is worse than none, because
            // you think you have it.
            let reaches_untrusted = profile
                .tools
                .iter()
                .filter_map(|t| registry.get(t))
                .any(|t| t.capabilities().untrusted_input);
            // `Chosen` rather than "can send at all": a profile whose only
            // sender is blind holds no route an injection can *direct*, which
            // is what this warning is about. It is still a leak path, and
            // `block_sends_after_private` is the control that owns that — so
            // warning here would contradict the design rather than reinforce
            // it (`docs/EGRESS-DESIGN.md` §D3).
            let can_send = profile
                .tools
                .iter()
                .filter_map(|t| registry.get(t))
                .any(|t| t.capabilities().egress == mecha_core::tool::Egress::Chosen);
            let has_private = profile
                .tools
                .iter()
                .filter_map(|t| registry.get(t))
                .any(|t| t.capabilities().private_data);
            if reaches_untrusted && can_send && has_private {
                println!(
                    "      ⚠ holds all three of private / untrusted / send — \
                     the isolation this profile implies is not real"
                );
            }
            if profile.trusted_output && reaches_untrusted {
                println!(
                    "      ⚠ trusted_output is set on a profile that reads untrusted \
                     content — the parent's interlock will not fire on its answers"
                );
            }
        }
    }

    Ok(())
}
