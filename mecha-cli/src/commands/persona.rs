//! `mecha persona` — the owner's door into the persona store
//! (`docs/PERSONA-DESIGN.md` §4).
//!
//! A persona made here is the owner's and approved on creation. Who it is
//! lives in Markdown the owner writes with an editor; `edit` hands the
//! terminal over and reads the result back through the ordinary load, so a
//! file that no longer parses is reported exactly as a hand-run `vi` would
//! leave it. Each saved change is a new version, which a chat will pin.
//!
//! Like `mecha imagelib`, it builds no provider and connects to nothing.

use anyhow::{bail, Result};
use mecha_core::imagelib::{self, Library};
use mecha_core::persona::{self, NewPersona, Origin, Persona, Status, Store};
use std::path::Path;

use super::imagelib::confirm;
use crate::GlobalOpts;

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Every persona, with anything wrong with it.
    List {
        #[arg(long)]
        json: bool,
    },
    /// One persona: its settings, links, version and problems.
    Show {
        name: String,
        #[arg(long)]
        json: bool,
    },
    /// Make a persona — yours, so approved at once. Fill in who they are with
    /// `mecha persona edit`.
    New {
        name: String,
        /// How they are shown and addressed; the name when omitted.
        #[arg(long)]
        display: Option<String>,
        /// A relationship template to start from (repeatable); its suggested
        /// tools are copied into the new persona.toml.
        #[arg(long = "relationship", value_name = "TEMPLATE")]
        relationships: Vec<String>,
        /// An image-library character: the persona's portrait.
        #[arg(long)]
        character: Option<String>,
        /// A voice profile in the store's voices/ folder.
        #[arg(long)]
        voice: Option<String>,
        /// A group declared with `mecha persona group add` (repeatable).
        #[arg(long = "group", value_name = "GROUP")]
        groups: Vec<String>,
        /// Hide it while browsing until the library is unlocked.
        #[arg(long)]
        locked: bool,
        /// Open identity.md in $EDITOR straight away.
        #[arg(long)]
        edit: bool,
    },
    /// Edit who a persona is (identity.md) in $EDITOR; a saved change is a
    /// new version, used by new chats.
    Edit {
        name: String,
        /// Edit motivation.md instead.
        #[arg(long, conflicts_with = "settings")]
        motivation: bool,
        /// Edit persona.toml instead.
        #[arg(long)]
        settings: bool,
    },
    /// Approve a persona made outside mecha, after reading it.
    Approve {
        name: String,
        /// Skip the question — refused for one of unknown provenance.
        #[arg(long)]
        yes: bool,
    },
    /// Hide a persona and its chats while browsing. Not encryption.
    Lock { name: String },
    /// Show a persona while browsing.
    Unlock { name: String },
    /// Remove a persona — chats and memory with it — moved aside under
    /// removed/, not deleted.
    Remove {
        name: String,
        #[arg(long)]
        yes: bool,
    },
    /// Relationship templates: list, or edit one.
    Relationship {
        #[command(subcommand)]
        cmd: RelationshipCmd,
    },
    /// Groups of personas that share an about-me and a files/ folder.
    Group {
        #[command(subcommand)]
        cmd: GroupCmd,
    },
    /// What a persona remembers, and the owner's edits to it (§9.8).
    Memory {
        #[command(subcommand)]
        cmd: MemoryCmd,
    },
}

#[derive(clap::Subcommand, Debug)]
pub enum MemoryCmd {
    /// Its episodes, its own facts, and what it learned about you —
    /// candidates included, marked.
    Show {
        name: String,
        #[arg(long)]
        json: bool,
    },
    /// Everything it remembers, as JSON lines in a fixed order.
    Export {
        name: String,
    },
    /// Let a candidate be recalled. It keeps its origin, so a recalled
    /// untrusted record still marks the chat untrusted.
    Approve {
        name: String,
        id: String,
    },
    /// Replace a fact with your wording; the old one is kept, invalidated.
    Correct {
        name: String,
        id: String,
        text: String,
    },
    /// Recall it first.
    Pin {
        name: String,
        id: String,
    },
    Unpin {
        name: String,
        id: String,
    },
    /// Delete one record — and any shared copy of it — or, with --chat,
    /// everything remembered from one chat.
    Forget {
        name: String,
        #[arg(required_unless_present = "chat", conflicts_with = "chat")]
        id: Option<String>,
        #[arg(long)]
        chat: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    /// Share a fact about you with every persona, or with one group.
    Share {
        name: String,
        id: String,
        #[arg(long, conflicts_with = "group", required_unless_present = "group")]
        everyone: bool,
        #[arg(long)]
        group: Option<String>,
    },
    /// What you have shared, and with whom.
    Shared {
        #[arg(long)]
        json: bool,
    },
    /// Stop sharing one copy; the persona that learned it keeps its own.
    Unshare {
        id: String,
    },
    /// Write what is new in persona chats into memory (§9.6) — what the
    /// nightly runs. Every persona, or one; on the local model only, since
    /// the writer reads whole transcripts.
    Write {
        name: Option<String>,
        /// One chat of that persona, by id; written even if it is recent.
        #[arg(long, requires = "name")]
        chat: Option<String>,
        /// Leave a chat alone if it changed within this many minutes — it
        /// may still be going.
        #[arg(long, default_value_t = 15)]
        idle_minutes: u64,
    },
}

#[derive(clap::Subcommand, Debug)]
pub enum RelationshipCmd {
    /// The templates in the store, and which personas name each.
    List,
    /// Edit a template in $EDITOR; personas naming it take the new text in
    /// their next chat.
    Edit { name: String },
}

#[derive(clap::Subcommand, Debug)]
pub enum GroupCmd {
    /// The declared groups and their members.
    List,
    /// Declare a group; personas join it with `groups = [...]` in their own
    /// persona.toml.
    Add {
        name: String,
        #[arg(long, default_value = "")]
        description: String,
    },
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    // Reading the owner's own writing stays open with personas off; every
    // change to the store is refused — `remove` too, which moves a persona
    // aside rather than deleting data, as `imagelib remove` does, and
    // `relationship list`, which seeds the starter templates as it reads.
    if !matches!(
        args.cmd,
        Cmd::List { .. }
            | Cmd::Show { .. }
            | Cmd::Group {
                cmd: GroupCmd::List
            }
            | Cmd::Memory {
                cmd: MemoryCmd::Show { .. } | MemoryCmd::Export { .. } | MemoryCmd::Shared { .. }
            }
    ) {
        super::features::require(mecha_core::feature::Feature::Personas)?;
    }
    let dir = Store::default_dir()?;
    if let Cmd::Memory {
        cmd:
            MemoryCmd::Write {
                name,
                chat,
                idle_minutes,
            },
    } = args.cmd
    {
        return write_memory(global, &dir, name, chat, idle_minutes).await;
    }
    let lib_dir = Library::default_dir()?;
    run(&dir, &lib_dir, args.cmd)
}

/// Embed whatever of one persona's memory has no vector yet, in batches.
async fn embed_memory(
    dir: &Path,
    persona: &str,
    embedder: &mecha_core::embed::Embedder,
) -> Result<usize> {
    use mecha_core::persona::memory::Memory;
    let m = Memory::open_to_edit(dir, persona)?;
    let mut done = 0usize;
    loop {
        let todo = m.unembedded(32)?;
        if todo.is_empty() {
            return Ok(done);
        }
        let texts: Vec<String> = todo.iter().map(|(_, t)| t.clone()).collect();
        let vectors = embedder
            .embed(&texts, mecha_core::embed::Task::Passage)
            .await?;
        // Position ties a vector to its record; a short reply is refused.
        if vectors.len() != todo.len() {
            anyhow::bail!("asked for {} embeddings, got {}", todo.len(), vectors.len());
        }
        let pairs: Vec<(String, Vec<f32>)> =
            todo.into_iter().map(|(uid, _)| uid).zip(vectors).collect();
        m.store_vectors(&pairs)?;
        done += pairs.len();
    }
}

/// `mecha persona memory write`: the writer over every persona's chats, or
/// one persona's, or one chat (§9.6).
async fn write_memory(
    global: &GlobalOpts,
    dir: &Path,
    name: Option<String>,
    chat: Option<String>,
    idle_minutes: u64,
) -> Result<()> {
    use mecha_core::persona::memory::Memory;
    use mecha_core::persona::writer::{self, Writer};

    let store = load(dir);
    let personas: Vec<&Persona> = match &name {
        Some(n) => vec![find(&store, n)?],
        None => store.all().iter().collect(),
    };
    let cwd = std::env::current_dir()?;
    let cfg = mecha_core::config::Config::load(&cwd)?;
    let (provider_name, provider_cfg) = cfg.provider(global.provider.as_deref())?;
    // The writer reads whole transcripts of private conversations, so it
    // runs where the appraisal does: on the local model, never a cloud one
    // (R29's rule for reading transcripts).
    // The address, not the dialect: `kind = "local"` can point at another
    // machine. The incognito gate's check, fallbacks included (review of
    // #468).
    if let Err(why) = mecha_core::config::provider_is_local(&cfg, &provider_name) {
        bail!("{why} — the memory writer reads whole persona chats, which stay on this machine");
    }
    let provider = mecha_core::provider::build(provider_cfg)?;
    let model = global.model.clone().or_else(|| provider_cfg.model.clone());
    let mut writer = Writer::new(provider, model);
    let fallback = writer.model().to_owned();

    // Each chat is written by the model it ran on, and only while that model
    // is the router's resident one (`writer::pick_model`): the owner's two
    // rulings together. A `--model` or `--provider` is the owner choosing,
    // and stands for every chat.
    use mecha_core::provider::router;
    let pinned = global.provider.is_some() || global.model.is_some();
    let base = provider_cfg.base_url.as_deref().map(router::base);
    let (warnings, seen) = router::observe_seen(&cfg, !pinned).await;
    for w in warnings {
        eprintln!("mecha: {w}");
    }
    // A default that follows a router which then did not answer is the one
    // case with no safe model to name: every chat waits (review of #468).
    let follows = !pinned
        && cfg
            .providers
            .get(&cfg.default_provider)
            .is_some_and(router::follows_here);
    let here = base
        .as_ref()
        .and_then(|b| seen.iter().find(|s| &s.base_url == b));
    let on_router = follows && here.is_some();
    // Not seen is two things: a router that would not say what it has
    // loaded, where naming a model could be the swap, and a plain server
    // marked `follow_loaded`, which serves one model whatever is named. Only
    // the first waits (review of #468).
    let unanswered = follows
        && here.is_none()
        && match &base {
            Some(b) => router::is_router(b).await != Some(false),
            None => false,
        };
    let mut resident: Option<String> = here.and_then(|s| s.resident.clone());
    if unanswered {
        eprintln!(
            "the router at {} did not say what it has loaded; every chat waits for a run when it does",
            base.as_deref().unwrap_or("?")
        );
    } else if on_router {
        eprintln!(
            "writing persona memory with each chat's own model, while it is loaded ({provider_name}; \
             loaded now: {})",
            resident.as_deref().unwrap_or("nothing")
        );
    } else {
        eprintln!("writing persona memory with {fallback} ({provider_name})");
    }

    let idle = std::time::Duration::from_secs(idle_minutes.saturating_mul(60));
    let (mut chats, mut failed, mut waiting, mut marked) = (0usize, 0usize, 0usize, 0usize);
    for p in personas {
        let pending = writer::pending_chats(&store.sessions_dir(&p.name), chat.as_deref(), idle);
        for problem in &pending.problems {
            eprintln!("{}: {problem}", p.name);
        }
        failed += pending.problems.len();
        marked += pending.marked;
        let mut todo = pending.due;
        if let Some(c) = &chat {
            if pending.marked > 0 {
                bail!(
                    "{} `{c}` is a test or experiment chat; memory leaves those out",
                    p.name
                );
            }
            if todo.is_empty() {
                bail!("{} has no chat `{c}`", p.name);
            }
        }
        todo.sort();
        if todo.is_empty() {
            continue;
        }
        // One persona's store failing to open is that persona's finding, not
        // the end of the night for every persona after it.
        let m = match Memory::open(dir, &p.name) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("{}: memory could not be opened ({e:#})", p.name);
                failed += 1;
                continue;
            }
        };
        for (id, path) in todo {
            let text = match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!(
                        "{}: {id} could not be read ({e}); left for a later run",
                        p.name
                    );
                    failed += 1;
                    continue;
                }
            };
            // Only wait for a model seat when there is something to ask.
            // A chat named explicitly may still be going: its tail waits for
            // the checkpoint rather than being written as untrusted.
            let parsed = if chat.is_some() {
                writer::read_chat(&text).settled()
            } else {
                writer::read_chat(&text)
            };
            match m.written_upto(&id) {
                Ok(upto) if upto as usize >= parsed.turns.len() => continue,
                Ok(_) => {}
                Err(e) => {
                    eprintln!("{} {id}: the ledger could not be read ({e:#})", p.name);
                    failed += 1;
                    continue;
                }
            }
            let server = if unanswered {
                writer::Server::Unanswered
            } else if on_router {
                writer::Server::Router(resident.as_deref())
            } else {
                writer::Server::One
            };
            match writer::pick_model(parsed.model.as_deref(), server, &fallback) {
                writer::ModelPick::Wait(model) => {
                    println!(
                        "{} {id}: waits until {model} is loaded — writing it now would swap \
                         out the model in use",
                        p.name
                    );
                    waiting += 1;
                    continue;
                }
                writer::ModelPick::Use(model) => {
                    // Loading into an empty router makes it the resident one
                    // for the rest of this run, so one night never swaps
                    // between two models.
                    if on_router && resident.is_none() {
                        resident = Some(model.clone());
                    }
                    writer.set_model(model);
                }
            }
            let seat = super::distill::take_seat(&format!("persona memory {} {id}", p.name)).await;
            let report = writer::write_chat(&writer, &m, p, &id, &parsed).await;
            drop(seat);
            chats += 1;
            match report {
                Ok(r) => {
                    for (from, to, origin, a) in &r.stretches {
                        println!(
                            "{} {id} turns {from}–{}{}: {} episode(s), {} added, {} updated, \
                             {} withdrawn, {} turned away",
                            p.name,
                            to.saturating_sub(1),
                            if *origin == Origin::ModelUntrusted {
                                " (untrusted — waiting on you)"
                            } else {
                                ""
                            },
                            a.episodes,
                            a.added,
                            a.updated,
                            a.invalidated,
                            a.refused
                        );
                    }
                    if r.raced > 0 {
                        println!("{} {id}: another writer got there first", p.name);
                    }
                }
                Err(e) => {
                    eprintln!("{} {id}: {e:#}; left for a later run", p.name);
                    failed += 1;
                }
            }
        }
    }

    // Vectors for what is remembered and not yet embedded (§9.7): new
    // records, and on a store's first night every record it already had.
    // Best-effort — without the embeddings server, recall is by words, and
    // the next night tries again.
    let mut embedded = 0usize;
    if let Some(embedder) = crate::setup::file_embedder(&cfg) {
        for p in store.all() {
            if !store
                .persona_dir(&p.name)
                .join(mecha_core::persona::memory::MEMORY_DB)
                .is_file()
            {
                continue;
            }
            match embed_memory(dir, &p.name, &embedder).await {
                Ok(n) => embedded += n,
                Err(e) => eprintln!("{}: memory left searchable by words only ({e:#})", p.name),
            }
        }
    }
    println!(
        "{chats} chat(s) read, {embedded} memories embedded, {waiting} waiting for their model, {marked} test or \
         experiment chat(s) left out, {failed} left for a later run"
    );
    if failed > 0 {
        bail!("{failed} chat(s) could not be written");
    }
    Ok(())
}

fn origin_label(o: Origin) -> &'static str {
    match o {
        Origin::Owner => "yours",
        Origin::ModelClean => "proposed by the model (clean conversation)",
        Origin::ModelUntrusted => "unknown — made outside mecha, or proposed by a model",
    }
}

fn load(dir: &Path) -> Store {
    let store = Store::load(dir);
    for e in store.errors() {
        eprintln!("mecha: {} did not load — {}", e.path.display(), e.why);
    }
    store
}

/// Why `name` did not load, when it is a persona folder that did not.
fn load_error(store: &Store, name: &str) -> Option<String> {
    let manifest = store.persona_dir(name).join("persona.toml");
    store
        .errors()
        .iter()
        .find(|e| e.path == manifest)
        .map(|e| e.why.clone())
}

fn lower_all(names: Vec<String>) -> Vec<String> {
    names.into_iter().map(|n| n.trim().to_lowercase()).collect()
}

fn find<'a>(store: &'a Store, name: &str) -> Result<&'a Persona> {
    let name = name.trim().to_lowercase();
    if let Some(p) = store.get(&name) {
        return Ok(p);
    }
    if let Some(why) = load_error(store, &name) {
        bail!("persona `{name}` does not load: {why}\n`mecha persona edit {name} --settings` to fix it");
    }
    bail!("no persona named `{name}` in {}", store.dir().display())
}

fn summary_json(store: &Store, lib: &Library, p: &Persona) -> serde_json::Value {
    serde_json::json!({
        "name": p.name,
        "display": p.display(),
        "relationship": p.settings.relationship.0,
        "character": p.settings.character,
        "voice": p.settings.voice,
        "groups": p.settings.groups,
        "model": p.settings.model,
        "tools": p.settings.tools.allow,
        "safety": p.settings.safety,
        "files": p.settings.files,
        "memory": p.settings.memory,
        "status": p.state.status,
        "origin": p.state.origin,
        "locked": p.state.locked,
        "version": p.state.version,
        "digest": p.state.digest,
        // `null` when the persona cannot be rendered: unknown, not "edited".
        "edited_since_version": store.content_digest(p).ok().map(|d| d != p.state.digest),
        "dir": store.persona_dir(&p.name),
        "sessions": store.sessions_dir(&p.name),
        "notes": p.notes,
        "problems": store.problems(p, lib),
    })
}

fn describe(store: &Store, lib: &Library, p: &Persona) {
    let s = &p.settings;
    println!("{} — `{}` (v{})", p.display(), p.name, p.state.version);
    println!(
        "  status:       {}{}",
        match p.state.status {
            Status::Approved => "approved",
            Status::Candidate => "not approved",
        },
        if p.state.locked { ", locked" } else { "" }
    );
    println!("  origin:       {}", origin_label(p.state.origin));
    let or_none = |v: &[String]| {
        if v.is_empty() {
            "none".to_string()
        } else {
            v.join(", ")
        }
    };
    println!("  relationship: {}", or_none(&s.relationship.0));
    println!(
        "  character:    {}",
        s.character.as_deref().unwrap_or("none")
    );
    println!("  voice:        {}", s.voice.as_deref().unwrap_or("none"));
    println!("  groups:       {}", or_none(&s.groups));
    println!("  tools asked:  {}", or_none(&s.tools.allow));
    println!(
        "  answers from: {}",
        match s.files.answers {
            persona::Answers::Files => "its files only",
            persona::Answers::Open => "its files and its tools",
        }
    );
    let off = s.safety.switched_off();
    if off.is_empty() {
        println!("  safety:       every protection on");
    } else {
        println!("  safety:       OFF for this persona — {}", off.join(", "));
    }
    if s.memory.self_update {
        println!("  self-update:  on — identity.md's sections may evolve on their own");
    }
    println!("  folder:       {}", store.persona_dir(&p.name).display());
    match store.content_digest(p) {
        Ok(d) if d != p.state.digest => println!(
            "  edited since v{} — the next chat or `mecha persona edit` takes a new version",
            p.state.version
        ),
        _ => {}
    }
    for n in &p.notes {
        println!("  note:         {n}");
    }
    for problem in store.problems(p, lib) {
        println!("  problem:      {problem}");
    }
}

/// What `edit` reports after the editor closes: the file re-read through the
/// ordinary load, and a version taken if anything changed.
fn after_edit(dir: &Path, lib: &Library, name: &str, again: &str) -> Result<()> {
    let store = Store::load(dir);
    let Some(p) = store.get(name) else {
        let why = load_error(&store, name).unwrap_or_else(|| "it is gone".into());
        bail!(
            "saved, but `{name}` will NOT load: {why}\n\
             no chat can open it until this is fixed — `{again}`"
        );
    };
    // By digest, not number: a persona made outside mecha is v1 before its
    // first version and v1 after it.
    let before = p.state.digest.clone();
    let snapshot = persona::snapshot(dir, name);
    match &snapshot {
        Ok(state) if state.digest == before => println!("unchanged"),
        Ok(state) => println!(
            "saved — `{name}` is v{}; new chats use it, open chats keep the version they began with",
            state.version
        ),
        Err(_) => eprintln!("saved, but no new version could be taken (the reason follows)"),
    }
    let store = Store::load(dir);
    if let Some(p) = store.get(name) {
        for n in &p.notes {
            eprintln!("note: {n}");
        }
        for problem in store.problems(p, lib) {
            eprintln!("problem: {problem}");
        }
    }
    snapshot.map(|_| ())
}

fn run(dir: &Path, lib_dir: &Path, cmd: Cmd) -> Result<()> {
    run_with(dir, lib_dir, cmd, &crate::editor::edit_file)
}

/// [`run`], with the editor passed in so a test can script it without
/// touching `$EDITOR`, which other tests set concurrently.
fn run_with(
    dir: &Path,
    lib_dir: &Path,
    cmd: Cmd,
    edit_file: &dyn Fn(&Path) -> Result<()>,
) -> Result<()> {
    let (lib, _) = Library::load(lib_dir);
    match cmd {
        Cmd::List { json } => {
            let store = load(dir);
            if json {
                let out: Vec<_> = store
                    .all()
                    .iter()
                    .map(|p| summary_json(&store, &lib, p))
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }
            if store.all().is_empty() {
                println!(
                    "No personas in {} yet — `mecha persona new <name>` makes one.",
                    dir.display()
                );
                return Ok(());
            }
            for p in store.all() {
                let problems = store.problems(p, &lib).len();
                let flags = [
                    (p.state.status != Status::Approved).then_some("not approved".to_string()),
                    p.state.locked.then_some("locked".to_string()),
                    (!p.settings.safety.switched_off().is_empty())
                        .then_some("safety off".to_string()),
                    (problems > 0).then(|| format!("{problems} problem(s)")),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", ");
                let flags = if flags.is_empty() {
                    String::new()
                } else {
                    format!(" [{flags}]")
                };
                let rel = p.settings.relationship.0.join(", ");
                println!(
                    "{:16} {} (v{}){}{flags}",
                    p.name,
                    p.display(),
                    p.state.version,
                    if rel.is_empty() {
                        String::new()
                    } else {
                        format!(" — {rel}")
                    }
                );
            }
        }
        Cmd::Show { name, json } => {
            let store = load(dir);
            let p = find(&store, &name)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&summary_json(&store, &lib, p))?
                );
            } else {
                describe(&store, &lib, p);
            }
        }
        Cmd::New {
            name,
            display,
            relationships,
            character,
            voice,
            groups,
            locked,
            edit,
        } => {
            let p = persona::create(
                dir,
                &lib,
                NewPersona {
                    name: name.trim().to_lowercase(),
                    display: display.unwrap_or_default(),
                    // Names are lowercase everywhere they are typed, as
                    // `group add` and `relationship edit` already make them.
                    relationships: lower_all(relationships),
                    character: character.map(|c| c.trim().to_lowercase()),
                    voice: voice.map(|v| v.trim().to_lowercase()),
                    groups: lower_all(groups),
                    locked,
                    origin: Origin::Owner,
                },
            )?;
            let pdir = dir.join(&p.name);
            println!("Made `{}` in {}.", p.name, pdir.display());
            if edit {
                edit_file(&pdir.join("identity.md"))?;
                after_edit(
                    dir,
                    &lib,
                    &p.name,
                    &format!("mecha persona edit {}", p.name),
                )?;
            } else {
                println!(
                    "Next: `mecha persona edit {}` to write who they are — `## Core` first.",
                    p.name
                );
            }
        }
        Cmd::Edit {
            name,
            motivation,
            settings,
        } => {
            // By folder, not by load: a persona whose file no longer parses
            // is exactly the one that needs its editor back.
            let name = name.trim().to_lowercase();
            persona::validate_persona_name(&name)?;
            let pdir = dir.join(&name);
            if !pdir.join("persona.toml").is_file() {
                bail!("no persona named `{name}` in {}", dir.display());
            }
            let (file, flag) = if settings {
                ("persona.toml", " --settings")
            } else if motivation {
                ("motivation.md", " --motivation")
            } else {
                ("identity.md", "")
            };
            edit_file(&pdir.join(file))?;
            after_edit(
                dir,
                &lib,
                &name,
                &format!("mecha persona edit {name}{flag}"),
            )?;
        }
        Cmd::Approve { name, yes } => {
            let store = load(dir);
            let p = find(&store, &name)?;
            if p.state.status == Status::Approved {
                bail!("`{}` is already approved", p.name);
            }
            describe(&store, &lib, p);
            // Both files go into every chat's prompt, so both are read here.
            println!("\n── identity.md ──\n{}", p.identity.trim());
            if !p.motivation.trim().is_empty() {
                println!("\n── motivation.md ──\n{}", p.motivation.trim());
            }
            if p.state.origin == Origin::ModelUntrusted && yes {
                bail!(
                    "`{}` came from somewhere mecha cannot vouch for, and once approved its text \
                     rides in every chat with it. Read it above and answer the question — run \
                     again without --yes.",
                    p.name
                );
            }
            if !yes && !confirm(&format!("Approve `{}`?", p.name))? {
                println!("Not approved.");
                return Ok(());
            }
            let state = persona::approve(dir, &p.name)?;
            println!("Approved `{}` (v{}).", p.name, state.version);
        }
        Cmd::Lock { name } => {
            let store = load(dir);
            let p = find(&store, &name)?;
            persona::set_locked(dir, &p.name, true)?;
            let password = if imagelib::has_lock_password(lib_dir) {
                "the image library's lock password"
            } else {
                "a lock password, which is not set yet — `mecha imagelib set-lock-password`"
            };
            println!(
                "Locked `{}`: it and its chats are hidden while browsing until the library is \
                 unlocked with {password}. This is not encryption — its files and chats stay \
                 plaintext on disk, and it still works when opened.",
                p.name
            );
        }
        Cmd::Unlock { name } => {
            let store = load(dir);
            let p = find(&store, &name)?;
            persona::set_locked(dir, &p.name, false)?;
            println!("Unlocked `{}`.", p.name);
        }
        Cmd::Remove { name, yes } => {
            let store = load(dir);
            let p = find(&store, &name)?;
            describe(&store, &lib, p);
            if !yes && !confirm(&format!("Remove `{}`, with its chats and memory?", p.name))? {
                println!("Kept.");
                return Ok(());
            }
            let to = persona::remove(dir, &p.name)?;
            println!("Removed `{}` — kept at {}.", p.name, to.display());
        }
        Cmd::Memory { cmd } => memory_cmd(dir, cmd)?,
        Cmd::Relationship { cmd } => {
            persona::seed_starters(dir)?;
            let store = load(dir);
            match cmd {
                RelationshipCmd::List => {
                    for (name, r) in store.relationships() {
                        let users: Vec<&str> = store
                            .all()
                            .iter()
                            .filter(|p| p.settings.relationship.0.contains(name))
                            .map(|p| p.name.as_str())
                            .collect();
                        println!(
                            "{name:16}{}{}",
                            if r.starter {
                                " (starter, unedited)"
                            } else {
                                ""
                            },
                            if users.is_empty() {
                                String::new()
                            } else {
                                format!(" — {}", users.join(", "))
                            }
                        );
                    }
                    println!(
                        "\nTemplates live in {}.",
                        dir.join("relationships").display()
                    );
                }
                RelationshipCmd::Edit { name } => {
                    let name = name.trim().to_lowercase();
                    persona::validate_name(&name)?;
                    let path = dir.join("relationships").join(format!("{name}.md"));
                    if !path.is_file() {
                        bail!(
                            "no template `{name}` — make one by writing {}",
                            path.display()
                        );
                    }
                    edit_file(&path)?;
                    let store = Store::load(dir);
                    if let Some(e) = store.errors().iter().find(|e| e.path == path) {
                        bail!("saved, but `{name}` will NOT load: {}", e.why);
                    }
                    let users: Vec<&str> = store
                        .all()
                        .iter()
                        .filter(|p| p.settings.relationship.0.contains(&name))
                        .map(|p| p.name.as_str())
                        .collect();
                    if users.is_empty() {
                        println!("saved — no persona names `{name}` yet");
                    } else {
                        println!(
                            "saved — {} take the new text in their next chat",
                            users.join(", ")
                        );
                    }
                }
            }
        }
        Cmd::Group { cmd } => match cmd {
            GroupCmd::List => {
                let store = load(dir);
                if store.groups().is_empty() {
                    println!("No groups yet — `mecha persona group add <name>` declares one.");
                }
                for (name, g) in store.groups() {
                    let members: Vec<&str> = store
                        .all()
                        .iter()
                        .filter(|p| p.settings.groups.contains(name))
                        .map(|p| p.name.as_str())
                        .collect();
                    println!(
                        "{name:16} {}{}",
                        if members.is_empty() {
                            "no members".to_string()
                        } else {
                            members.join(", ")
                        },
                        if g.description.is_empty() {
                            String::new()
                        } else {
                            format!(" — {}", g.description)
                        }
                    );
                }
            }
            GroupCmd::Add { name, description } => {
                let name = name.trim().to_lowercase();
                persona::add_group(dir, &name, &description)?;
                println!(
                    "Declared `{name}`. A persona joins it with `groups = [\"{name}\"]` in its \
                     persona.toml; what the group may know about you goes in {}.",
                    dir.join("groups").join(&name).join("about-me.md").display()
                );
            }
        },
    }
    Ok(())
}

/// `mecha persona memory …`: the owner's curation of what a persona
/// remembers (§9.8). Every id printed is the first eight characters of the
/// record's; any unambiguous prefix is accepted back.
fn memory_cmd(dir: &Path, cmd: MemoryCmd) -> Result<()> {
    use mecha_core::persona::memory::{self, Audience, Filter, Memory, Shared, Table};

    let store = load(dir);
    let named = |name: &str| -> Result<String> { Ok(find(&store, name)?.name.clone()) };
    match cmd {
        MemoryCmd::Show { name, json } => {
            let name = named(&name)?;
            let Some(m) = Memory::open_existing(dir, &name)? else {
                if json {
                    println!("{}", serde_json::json!({"episodes": [], "facts": []}));
                } else {
                    println!("{name} remembers nothing yet.");
                }
                return Ok(());
            };
            let episodes = m.episodes(Filter::All)?;
            let mut facts = Vec::new();
            for t in [Table::Persona, Table::User, Table::Inferred] {
                facts.extend(m.facts(t, Filter::All)?);
            }
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({"episodes": episodes, "facts": facts})
                    )?
                );
                return Ok(());
            }
            let mark = |status: memory::Status, origin: Origin, pinned: bool| {
                let mut tags = Vec::new();
                if pinned {
                    tags.push("pinned");
                }
                match status {
                    memory::Status::Candidate => tags.push("waiting on you"),
                    memory::Status::Invalidated => tags.push("invalidated"),
                    memory::Status::Active => {}
                }
                match origin {
                    Origin::Owner => tags.push("yours"),
                    Origin::ModelUntrusted => tags.push("untrusted"),
                    Origin::ModelClean => {}
                }
                if tags.is_empty() {
                    String::new()
                } else {
                    format!("  [{}]", tags.join(", "))
                }
            };
            println!("Episodes ({})", episodes.len());
            for e in &episodes {
                println!(
                    "  {}  {} turns {}–{}{}",
                    &e.uid[..8],
                    e.source.chat,
                    e.source.from,
                    e.source.to,
                    mark(e.status, e.origin, e.pinned)
                );
                println!("            {}", e.summary);
                if !e.open_threads.is_empty() {
                    println!("            open: {}", e.open_threads.join("; "));
                }
            }
            for (t, heading) in [
                (Table::Persona, "Its own facts"),
                (Table::User, "What it learned about you"),
                (Table::Inferred, "What it inferred about you"),
            ] {
                let rows: Vec<_> = facts.iter().filter(|f| f.table == t).collect();
                println!("{heading} ({})", rows.len());
                for f in rows {
                    println!(
                        "  {}  {}{}",
                        &f.uid[..8],
                        f.text,
                        mark(f.status, f.origin, f.pinned)
                    );
                }
            }
        }
        MemoryCmd::Export { name } => {
            let name = named(&name)?;
            if let Some(m) = Memory::open_existing(dir, &name)? {
                print!("{}", m.export()?);
            }
        }
        MemoryCmd::Approve { name, id } => {
            let m = Memory::open_to_edit(dir, &named(&name)?)?;
            m.approve(&m.resolve(&id)?)?;
            println!("Approved — it can be recalled now.");
        }
        MemoryCmd::Correct { name, id, text } => {
            let m = Memory::open_to_edit(dir, &named(&name)?)?;
            let new = m.correct(&m.resolve(&id)?, &text)?;
            println!("Corrected — now {}.", &new.uid[..8]);
        }
        MemoryCmd::Pin { name, id } => {
            let m = Memory::open_to_edit(dir, &named(&name)?)?;
            m.pin(&m.resolve(&id)?, true)?;
            println!("Pinned — recalled first.");
        }
        MemoryCmd::Unpin { name, id } => {
            let m = Memory::open_to_edit(dir, &named(&name)?)?;
            m.pin(&m.resolve(&id)?, false)?;
            println!("Unpinned.");
        }
        MemoryCmd::Forget {
            name,
            id,
            chat,
            yes,
        } => {
            let name = named(&name)?;
            if let Some(chat) = chat {
                let question = format!(
                    "Delete everything {name} remembers from chat {chat}? This cannot be undone."
                );
                if !yes && !confirm(&question)? {
                    println!("Kept.");
                    return Ok(());
                }
                let out = memory::forget_chat(dir, &name, &chat)?;
                println!(
                    "Forgotten: {} episodes, {} facts, {} shared copies.",
                    out.episodes, out.facts, out.shared
                );
            } else {
                let id = id.expect("clap requires an id or --chat");
                let full = Memory::open_to_edit(dir, &name)?.resolve(&id)?;
                if !yes && !confirm(&format!("Delete {} for good?", &full[..8]))? {
                    println!("Kept.");
                    return Ok(());
                }
                let copies = memory::forget(dir, &name, &full)?;
                println!("Forgotten, with {copies} shared copies.");
            }
        }
        MemoryCmd::Share {
            name,
            id,
            everyone,
            group,
        } => {
            let m = Memory::open_to_edit(dir, &named(&name)?)?;
            let full = m.resolve(&id)?;
            let fact = m
                .fact(&full)?
                .ok_or_else(|| anyhow::anyhow!("`{id}` is an episode; only facts are shared"))?;
            let audience = match (everyone, group) {
                (true, _) => Audience::Everyone,
                (false, Some(g)) => Audience::Group(g.trim().to_lowercase()),
                (false, None) => unreachable!("clap requires --everyone or --group"),
            };
            let declared: Vec<String> = store.groups().keys().cloned().collect();
            let copy = Shared::open(dir)?.share(&fact, audience, &declared)?;
            println!("Shared — {}.", &copy.uid[..8]);
        }
        MemoryCmd::Shared { json } => {
            let listed = match Shared::open_existing(dir)? {
                Some(s) => s.all()?,
                None => memory::Listing::default(),
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&listed)?);
                return Ok(());
            }
            if listed.facts.is_empty() && listed.unreadable == 0 {
                println!("Nothing shared.");
            }
            if listed.unreadable > 0 {
                println!(
                    "  {} shared record(s) this mecha cannot read, never shown to a persona",
                    listed.unreadable
                );
            }
            for r in listed.facts {
                let with = match &r.audience {
                    Audience::Everyone => "everyone".to_string(),
                    Audience::Group(g) => format!("group {g}"),
                };
                println!(
                    "  {}  {}  [{with}; learned by {}]",
                    &r.uid[..8],
                    r.text,
                    r.learned_by
                );
            }
        }
        // Async — `execute` runs it before this synchronous dispatch.
        MemoryCmd::Write { .. } => bail!("`persona memory write` is dispatched by `execute`"),
        MemoryCmd::Unshare { id } => {
            if !Shared::path(dir).is_file() {
                bail!("nothing is shared");
            }
            let shared = Shared::open(dir)?;
            shared.unshare(&shared.resolve(&id)?)?;
            println!("No longer shared.");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mecha-persona-cli-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn new(name: &str) -> Cmd {
        Cmd::New {
            name: name.into(),
            display: None,
            relationships: vec!["friend".into()],
            character: None,
            voice: None,
            groups: vec![],
            locked: false,
            edit: false,
        }
    }

    #[test]
    fn yes_never_skips_reading_a_persona_of_unknown_provenance() {
        let dir = scratch();
        let lib = dir.join("imagelib");
        let store = dir.join("personas");
        persona::ensure_layout(&store).unwrap();
        std::fs::create_dir_all(store.join("ada")).unwrap();
        std::fs::write(store.join("ada/persona.toml"), "").unwrap();
        std::fs::write(store.join("ada/identity.md"), "## Core\nA patient.\n").unwrap();
        let refused = run(
            &store,
            &lib,
            Cmd::Approve {
                name: "ada".into(),
                yes: true,
            },
        )
        .unwrap_err();
        assert!(format!("{refused}").contains("without --yes"), "{refused}");
        assert_eq!(
            Store::load(&store).get("ada").unwrap().state.status,
            Status::Candidate
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_persona_that_does_not_load_can_still_be_edited_and_says_why() {
        let dir = scratch();
        let lib = dir.join("imagelib");
        let store = dir.join("personas");
        run(&store, &lib, new("mara")).unwrap();
        let toml = store.join("mara/persona.toml");
        let text = std::fs::read_to_string(&toml).unwrap();
        std::fs::write(&toml, text.replace("episodic ", "episodc ")).unwrap();
        let shown = run(
            &store,
            &lib,
            Cmd::Show {
                name: "mara".into(),
                json: false,
            },
        )
        .unwrap_err();
        assert!(format!("{shown}").contains("episodc"), "{shown}");
        // An editor that saves nothing: the point is that the door opens.
        let opened = std::cell::Cell::new(false);
        let edited = run_with(
            &store,
            &lib,
            Cmd::Edit {
                name: "mara".into(),
                motivation: false,
                settings: true,
            },
            &|path: &Path| {
                assert!(path.ends_with("mara/persona.toml"), "{}", path.display());
                opened.set(true);
                Ok(())
            },
        )
        .unwrap_err();
        assert!(opened.get());
        let edited = format!("{edited}");
        assert!(edited.contains("will NOT load"), "{edited}");
        assert!(edited.contains("edit mara --settings"), "{edited}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn new_then_lock_then_group() {
        let dir = scratch();
        let lib = dir.join("imagelib");
        let store = dir.join("personas");
        run(&store, &lib, new("Mara")).unwrap();
        run(
            &store,
            &lib,
            Cmd::Lock {
                name: "mara".into(),
            },
        )
        .unwrap();
        let loaded = Store::load(&store);
        let p = loaded.get("mara").unwrap();
        assert!(p.state.locked);
        assert_eq!(p.settings.relationship.0, vec!["friend".to_string()]);
        run(
            &store,
            &lib,
            Cmd::Group {
                cmd: GroupCmd::Add {
                    name: "work".into(),
                    description: String::new(),
                },
            },
        )
        .unwrap();
        assert!(Store::load(&store).groups().contains_key("work"));
        // The show paths read without panicking on a real store.
        run(
            &store,
            &lib,
            Cmd::Show {
                name: "mara".into(),
                json: true,
            },
        )
        .unwrap();
        run(&store, &lib, Cmd::List { json: false }).unwrap();
        std::fs::remove_dir_all(dir).ok();
    }
}
