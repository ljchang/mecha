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
    Ok(())
}
