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
    let prepared = setup::prepare_tools(global, false).await?;
    let registry = &prepared.registry;

    if args.json {
        let specs: Vec<_> = registry
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name(),
                    "description": t.description(),
                    "read_only": t.read_only(),
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

    for tool in registry.iter() {
        let access = if tool.read_only() { "read-only" } else { "writes" };
        println!("{}  [{}]", tool.name(), access);
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

    println!("{} tools · workspace {}", registry.len(), prepared.workspace.display());

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
            let can_send = profile
                .tools
                .iter()
                .filter_map(|t| registry.get(t))
                .any(|t| t.capabilities().external_send);
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
