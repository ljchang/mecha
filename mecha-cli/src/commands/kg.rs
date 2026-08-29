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
//! no dependency on the graph's schema. Two exceptions to MCP-only, both for
//! the same reason — verbs the tool surface deliberately lacks: `mecha
//! review` (the merge queue), and `assert`/`retract` here, which spawn the
//! `mecha-graph` binary the way review does. An owner stating or retracting
//! a fact lands live by design (an instruction, not an inference), and that
//! authority must not exist on the surface a model drives — a model's facts
//! go through `kg_upsert`, which stages a candidate for review. Everything
//! else here is the tools' own job: search and entity reads, and a note is
//! `kg_upsert(kind=episode)`, which links entities on landing exactly as the
//! graph's own `note` verb does.
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
    /// Recent notes, newest first — the notebook view.
    Notes {
        #[arg(long, default_value_t = 20)]
        limit: u64,
        #[arg(long)]
        json: bool,
    },
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
    /// State a fact yourself. Lands live, never in the review queue — the
    /// owner asserting something is an instruction, not an inference about
    /// the world. A connection is a fact too (`edges` is a view over them),
    /// so this is also how a node gains one, and `retract` how it loses one.
    Assert {
        /// Subject: node id, name or alias.
        subject: String,
        /// Predicate — prefer the existing vocabulary; a new one is minted
        /// deliberately, never by typo (D4).
        predicate: String,
        /// Object: node id, name or alias. Omit for an attribute-style fact
        /// and give --value instead.
        object: Option<String>,
        /// A literal object, when the object is not a node.
        #[arg(long)]
        value: Option<String>,
        /// The sentence form — what search and a reader see. Composed from
        /// the parts when omitted.
        #[arg(long)]
        statement: Option<String>,
    },
    /// Retract a fact by the uid `entity --json` prints — never a text
    /// match, because retracting the wrong fact off a substring is the
    /// failure this graph keeps finding.
    Retract {
        /// The fact's uid.
        uid: String,
        /// When it stopped being true. Omitted, it is invalidated as of now
        /// — right for a claim that was never right, as against one that has
        /// simply ended.
        #[arg(long)]
        as_of: Option<String>,
    },
    /// The bounded neighborhood around one node: 1–2 hops over current
    /// facts. What an entity connects to, without pretending to be a map.
    Related {
        /// Name, alias, or id — the graph resolves it.
        #[arg(required = true, num_args = 1..)]
        name: Vec<String>,
        /// 1 or 2.
        #[arg(long, default_value_t = 1)]
        hops: u8,
        #[arg(long)]
        json: bool,
    },
    /// Bi-temporal history for an entity: superseded facts kept beside what
    /// replaced them, and the episode timeline.
    Timeline {
        /// Name, alias, or id — the graph resolves it.
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
        /// Rewrite an existing note in place, by the `source_id` that
        /// `mecha kg notes` prints. The graph's episode key is
        /// (source, source_id), so this updates the row rather than adding a
        /// near-duplicate beside it — and it re-mines: the cached embedding
        /// and enrichment are dropped, so the nightly extractor reads the new
        /// wording. Candidates already derived from the old wording stay in
        /// the review queue; editing a note is not a retraction.
        #[arg(long, value_name = "SOURCE_ID")]
        edit: Option<String>,
    },
}

pub async fn run(global: &GlobalOpts, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::Search { query, k, json } => search(global, &query.join(" "), k, json).await,
        Cmd::Entity { name, json } => entity(global, &name.join(" "), json).await,
        Cmd::Related { name, hops, json } => related(global, &name.join(" "), hops, json).await,
        Cmd::Timeline { name, json } => timeline(global, &name.join(" "), json).await,
        Cmd::Assert {
            subject,
            predicate,
            object,
            value,
            statement,
        } => assert_fact(&subject, &predicate, object, value, statement),
        Cmd::Retract { uid, as_of } => retract_fact(&uid, as_of),
        Cmd::Note { text, edit } => match edit {
            Some(id) => note_edit(global, &id, &text.join(" ")).await,
            None => note(global, &text.join(" ")).await,
        },
        Cmd::Notes { limit, json } => notes(global, limit, json).await,
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

/// Owner fact-authoring, through the `mecha-graph` binary exactly as
/// `mecha review` reaches the merge queue — and *not* through MCP, on
/// purpose: this authority must not exist on the surface a model drives.
/// The graph's own output passes through verbatim (its refusals name what
/// went wrong — an unresolvable subject, an unknown uid), so every caller
/// of this verb relays the one implementation's words.
///
/// Options before `--`, positionals after it: a fact's subject is prose and
/// prose can start with a dash, the same rule note capture learned.
fn assert_fact(
    subject: &str,
    predicate: &str,
    object: Option<String>,
    value: Option<String>,
    statement: Option<String>,
) -> Result<()> {
    if subject.trim().is_empty() || predicate.trim().is_empty() {
        bail!("a fact needs a subject and a predicate");
    }
    if object.is_none() && value.as_deref().map(str::trim).unwrap_or("").is_empty() {
        bail!("a fact needs an object (an entity) or --value (a literal)");
    }
    let mut args: Vec<&str> = vec!["assert"];
    if let Some(v) = value.as_deref() {
        args.extend(["--value", v]);
    }
    if let Some(s) = statement.as_deref() {
        args.extend(["--statement", s]);
    }
    args.push("--");
    args.push(subject);
    args.push(predicate);
    if let Some(o) = object.as_deref() {
        args.push(o);
    }
    print!("{}", crate::commands::review::graph_cli(&args)?);
    Ok(())
}

/// Retract by uid, relaying the graph's own account.
fn retract_fact(uid: &str, as_of: Option<String>) -> Result<()> {
    if uid.trim().is_empty() {
        bail!("which fact? `mecha kg entity <name> --json` prints uids");
    }
    let mut args: Vec<&str> = vec!["retract"];
    if let Some(t) = as_of.as_deref() {
        args.extend(["--as-of", t]);
    }
    args.push("--");
    args.push(uid);
    print!("{}", crate::commands::review::graph_cli(&args)?);
    Ok(())
}

/// The neighborhood: `kg_related`, rendered one row per neighbor. Hops are
/// clamped tool-side (1–2) — this passes what was asked and lets the graph
/// answer with what it actually did.
async fn related(global: &GlobalOpts, name: &str, hops: u8, as_json: bool) -> Result<()> {
    let out = call(global, "kg_related", json!({ "id": name, "hops": hops })).await?;
    if as_json {
        println!("{out}");
        return Ok(());
    }
    let root = out["root"]["name"].as_str().unwrap_or(name);
    let items = out["items"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if items.is_empty() {
        println!("nothing connects to `{root}` yet");
        return Ok(());
    }
    println!("around {root}:");
    for it in items {
        println!(
            "  {:<26} {:<10} {}",
            it["name"].as_str().unwrap_or("?"),
            it["type"].as_str().unwrap_or("?"),
            it["via"]["predicate"]
                .as_str()
                .map(|p| format!("via {p}"))
                .unwrap_or_default(),
        );
    }
    Ok(())
}

/// The history: `kg_timeline`, rendered with superseded facts kept visible.
/// A fact that stopped being true is evidence about when things changed, and
/// a listing that hides it re-tells the graph's story with the seams ironed
/// out.
async fn timeline(global: &GlobalOpts, name: &str, as_json: bool) -> Result<()> {
    let out = call(global, "kg_timeline", json!({ "entity": name })).await?;
    if as_json {
        println!("{out}");
        return Ok(());
    }
    let who = out["entity"]["name"].as_str().unwrap_or(name);
    let facts = out["facts"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let eps = out["episodes"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if facts.is_empty() && eps.is_empty() {
        println!("no history for `{who}` yet");
        return Ok(());
    }
    let day = |v: &Value| {
        v.as_str()
            .map(|d| d.chars().take(10).collect::<String>())
            .unwrap_or_else(|| "—".into())
    };
    if !facts.is_empty() {
        println!("facts for {who}:");
        for f in facts {
            let superseded = f["superseded"].as_bool() == Some(true);
            println!(
                "  {} {}  {}{}",
                if superseded { "✗" } else { "·" },
                day(&f["valid_from"]),
                f["statement"].as_str().unwrap_or("?"),
                if superseded {
                    format!("  (until {})", day(&f["valid_to"]))
                } else {
                    String::new()
                },
            );
        }
    }
    if !eps.is_empty() {
        println!("\nepisodes:");
        for ep in eps {
            println!(
                "  {}  {:<8} {}",
                day(&ep["occurred_at"]),
                ep["source"].as_str().unwrap_or("?"),
                ep["preview"]
                    .as_str()
                    .unwrap_or("")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(100)
                    .collect::<String>(),
            );
        }
    }
    Ok(())
}

/// The notebook view: `kg_notes`, rendered. JSON is the envelope verbatim
/// for the web page; text is one line per note for a terminal.
async fn notes(global: &GlobalOpts, limit: u64, as_json: bool) -> Result<()> {
    let out = call(global, "kg_notes", json!({ "limit": limit })).await?;
    if as_json {
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    let rows = out["notes"].as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        println!("no notes yet — `mecha kg note <text>` captures one");
        return Ok(());
    }
    for n in &rows {
        let body = n["body"].as_str().unwrap_or("?");
        let head: String = body.chars().take(80).collect();
        // The id is printed because it is the handle `--edit` takes, and a
        // listing whose rows cannot be named is a listing you can only read.
        println!(
            "  {}  {:<26}  {}",
            n["occurred_at"]
                .as_str()
                .unwrap_or("?")
                .chars()
                .take(16)
                .collect::<String>(),
            n["source_id"].as_str().unwrap_or("—"),
            head
        );
    }
    println!("\nedit one: mecha kg note --edit <id> <new text>");
    Ok(())
}

/// Rewrite one note in place.
///
/// Three things decide the shape:
///
/// - **The episode key is (source, source_id), not the uid.** `kg_notes`
///   prints both; only the second one can write. Re-upserting under it is an
///   UPDATE — the graph drops the stale embedding and enrichment, so the
///   nightly extractor re-reads the new wording — where a fresh id would
///   leave the old note sitting beside the new one, both true-looking.
/// - **The note's own moment is preserved, never re-stamped.** `upsert_episode`
///   writes every field it is handed, and `occurred_at` defaults to *now*
///   when omitted, so an edit that did not carry it would move the note to
///   today: a notebook rewriting when things happened because somebody fixed
///   a typo. It is read back from the listing the id came from rather than
///   accepted from a caller, so no surface can get it wrong on its own.
/// - **`unchanged` is reported as unchanged.** The graph hashes the body and
///   skips identical content, and printing that as "updated" would be a
///   surface confirming an edit that never happened.
async fn note_edit(global: &GlobalOpts, source_id: &str, text: &str) -> Result<()> {
    if text.trim().is_empty() {
        bail!("an empty note records nothing — reject it in review instead");
    }
    let listing = call(global, "kg_notes", json!({ "limit": 200 })).await?;
    let row = listing["notes"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .find(|n| n["source_id"].as_str() == Some(source_id))
        .with_context(|| {
            format!(
                "no note `{source_id}` in the last 200 — `mecha kg notes --limit 200` lists them"
            )
        })?;
    let occurred_at = row["occurred_at"].as_str().with_context(|| {
        format!("note `{source_id}` has no timestamp to preserve — refusing to re-stamp it as now")
    })?;
    let out = call(
        global,
        "kg_upsert",
        json!({
            "kind": "episode",
            "source": "note",
            "source_id": source_id,
            "body": text,
            "occurred_at": occurred_at,
        }),
    )
    .await?;
    match out["status"].as_str().unwrap_or("?") {
        "updated" => println!(
            "edited (episode {}, {} entities linked) — the extractor re-reads it tonight",
            out["episode_id"].as_i64().unwrap_or(0),
            out["entities_linked"].as_i64().unwrap_or(0),
        ),
        "unchanged" => println!("unchanged — the note already said exactly that"),
        "inserted" => println!(
            "no note carried that id, so this was captured as a new one (episode {})",
            out["episode_id"].as_i64().unwrap_or(0),
        ),
        other => bail!(
            "{other}: {}",
            out["note"].as_str().unwrap_or("the graph refused the edit")
        ),
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
    // A tombstoned (source, source_id) lands nothing and answers `status:
    // "tombstoned"` with episode_id 0. Printing the usual line there would
    // confirm a capture that was refused — "noted (episode 0)" reads as
    // success to everyone including the caller.
    if out["status"].as_str() == Some("tombstoned") {
        bail!(
            "{}",
            out["note"]
                .as_str()
                .unwrap_or("the graph refused this capture")
        );
    }
    println!(
        "noted (episode {}, {} entities linked)",
        out["episode_id"].as_i64().unwrap_or(0),
        out["entities_linked"].as_i64().unwrap_or(0),
    );
    Ok(())
}
