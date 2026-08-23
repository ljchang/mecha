//! `mecha kg` — the knowledge graph from the terminal: search it, read an
//! entity, capture a note.
//!
//! The command line does everything first and the TUI (`/find`, `/note`)
//! drives it, on the front door's rule: one implementation per verb, and no
//! way for a UI to do something the terminal cannot.
//!
//! **The graph is reached the same way the model reaches it — through the
//! MCP tool surface**, exactly as `mecha tasks` does: `kg_search`,
//! `kg_entity` and `kg_upsert` already answer in JSON, the lookup matches on
//! the tool suffix so a renamed server keeps working, and `mecha-cli` takes
//! no dependency on the graph's schema. The one exception to MCP-only remains
//! `mecha review` (the merge queue needs verbs the tool surface deliberately
//! lacks); nothing here needs one — search and entity reads are the tools'
//! own job, and a note is `kg_upsert(kind=episode)`, which links entities on
//! landing exactly as the graph's own `note` verb does.
//!
//! No approver and no interlock, deliberately, as `/tasks` argued it: the
//! person at the keyboard is the authority, nothing here reaches a third
//! party, and a note is the owner's own words entering their own store.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::setup::{find_tool, tool_ctx};
use crate::{setup, GlobalOpts};

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Search the graph: entities, facts, episodes — the same context pack
    /// the model gets. `#tag` tokens filter to hand-tagged episodes.
    Search {
        /// Natural-language query. Trailing words are joined, no quoting.
        #[arg(required = true, num_args = 1..)]
        query: Vec<String>,
        /// Max results.
        #[arg(long, short = 'k', default_value_t = 10)]
        k: usize,
        #[arg(long)]
        json: bool,
    },
    /// Everything about one entity: facts, interaction recency, episodes.
    Entity {
        /// Name, alias, or email — the graph resolves it.
        #[arg(required = true, num_args = 1..)]
        name: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// Capture a note as an episode. Entities named in it are linked on
    /// landing; the nightly extractor mines it like any other evidence.
    Note {
        /// The note. Trailing words are joined, so it needs no quoting.
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
    },
}

pub async fn run(global: &GlobalOpts, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::Search { query, k, json } => search(global, &query.join(" "), k, json).await,
        Cmd::Entity { name, json } => entity(global, &name.join(" "), json).await,
        Cmd::Note { text } => note(global, &text.join(" ")).await,
    }
}

/// Call one `kg_*` tool and return its parsed answer — `tasks::call`'s twin.
async fn call(global: &GlobalOpts, tool: &str, args: Value) -> Result<Value> {
    let prepared = setup::prepare_tools(global, false).await?;
    let found = find_tool(&prepared.registry, tool).with_context(|| {
        format!("no knowledge-graph server in this configuration — `{tool}` is not on the tool surface. Is `[[mcp]]` enabled?")
    })?;
    let out = found.call(args, &tool_ctx(&prepared)).await?;
    if out.is_error {
        bail!("{}: {}", tool, out.content.trim());
    }
    serde_json::from_str(&out.content)
        .with_context(|| format!("{tool} did not answer with JSON: {}", out.content))
}

async fn search(global: &GlobalOpts, query: &str, k: usize, as_json: bool) -> Result<()> {
    let pack = call(global, "kg_search", json!({ "query": query, "k": k })).await?;
    if as_json {
        println!("{pack}");
        return Ok(());
    }
    let items = pack["items"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if items.is_empty() {
        println!("nothing found for `{query}`");
        return Ok(());
    }
    for it in items {
        // One row per hit: an episode's body is multi-line prose, and a
        // listing where one result is forty lines buries the other nine.
        let text: String = it["text"]
            .as_str()
            .unwrap_or("")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        println!(
            "{:<8} {}  {}",
            it["kind"].as_str().unwrap_or("?"),
            it["occurred_at"]
                .as_str()
                .map(|d| d.chars().take(10).collect::<String>())
                .unwrap_or_else(|| "—".into()),
            text.chars().take(160).collect::<String>(),
        );
    }
    if let Some(entities) = pack["entities"].as_array().filter(|e| !e.is_empty()) {
        let names: Vec<&str> = entities.iter().filter_map(|e| e.as_str()).collect();
        if !names.is_empty() {
            println!("\nentities: {}", names.join(" · "));
        }
    }
    if pack["truncated"].as_bool() == Some(true) {
        println!("(truncated to the token budget — narrow the query for more)");
    }
    Ok(())
}

async fn entity(global: &GlobalOpts, name: &str, as_json: bool) -> Result<()> {
    let e = call(global, "kg_entity", json!({ "name_or_id": name })).await?;
    if as_json {
        println!("{e}");
        return Ok(());
    }
    if e["found"].as_bool() != Some(true) {
        println!("no entity matches `{name}`");
        return Ok(());
    }
    // Several entities answer to this name: list them and stop, exactly as
    // the tool hands the model a disambiguation. The id is printed because
    // it is the one spelling that cannot be ambiguous.
    if let Some(m) = e["ambiguous"].as_array().filter(|m| !m.is_empty()) {
        println!("`{name}` is ambiguous:");
        for c in m.iter() {
            println!(
                "  {:<26} last seen {}  {}",
                c["name"].as_str().unwrap_or("?"),
                c["last_seen"]
                    .as_str()
                    .map(|d| d.chars().take(10).collect::<String>())
                    .unwrap_or_else(|| "—".into()),
                c["id"].as_str().unwrap_or(""),
            );
        }
        return Ok(());
    }
    let node = &e["node"];
    println!(
        "{}  ({})  {}",
        node["name"].as_str().unwrap_or("?"),
        node["type"].as_str().unwrap_or("?"),
        node["id"].as_str().unwrap_or(""),
    );
    if let Some(aliases) = node["aliases"].as_array().filter(|a| !a.is_empty()) {
        let a: Vec<&str> = aliases.iter().filter_map(|x| x.as_str()).collect();
        println!("aka: {}", a.join(" · "));
    }
    let i = &e["interaction"];
    if let Some(last) = i["last_seen_at"].as_str() {
        println!(
            "seen: {} times, last {last} via {}",
            i["interaction_count"].as_i64().unwrap_or(0),
            i["last_channel"].as_str().unwrap_or("?"),
        );
    }
    if let Some(facts) = e["facts"].as_array().filter(|f| !f.is_empty()) {
        println!("\nfacts:");
        for f in facts {
            println!("  · {}", f["statement"].as_str().unwrap_or("?"));
        }
    }
    if let Some(eps) = e["episodes"].as_array().filter(|x| !x.is_empty()) {
        println!("\nepisodes:");
        for ep in eps {
            println!(
                "  {}  {}",
                ep["occurred_at"]
                    .as_str()
                    .map(|d| d.chars().take(10).collect::<String>())
                    .unwrap_or_else(|| "—".into()),
                ep["preview"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(100)
                    .collect::<String>(),
            );
        }
    }
    Ok(())
}

async fn note(global: &GlobalOpts, text: &str) -> Result<()> {
    if text.trim().is_empty() {
        bail!("an empty note records nothing");
    }
    // The same landing as the graph's own `note` verb: source "note", a
    // fresh id (the idempotence key — fresh because every capture is a new
    // moment, unlike a distilled session which re-pushes under its own id).
    let out = call(
        global,
        "kg_upsert",
        json!({
            "kind": "episode",
            "source": "note",
            "source_id": mecha_core::session::Session::new_id(),
            "body": text,
        }),
    )
    .await?;
    println!(
        "noted (episode {}, {} entities linked)",
        out["episode_id"].as_i64().unwrap_or(0),
        out["entities_linked"].as_i64().unwrap_or(0),
    );
    Ok(())
}
