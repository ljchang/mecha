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

use anyhow::{bail, Context, Result};
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
        /// A voice from the voice library (Library → Voices): a name the
        /// voice server lists. Its speed is `voice_speed` in persona.toml.
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
    /// Her week: the anchors her days are built around, and a model's draft
    /// waiting on you (`PERSONA-LIFE-DESIGN.md` §4.1).
    Routine {
        #[command(subcommand)]
        cmd: RoutineCmd,
    },
    /// Her day: drawn from her routine and time-use data the first time it
    /// is asked for, then kept (`PERSONA-LIFE-DESIGN.md` §4.2). Needs
    /// `[life] world = true` and a `cell` in routine.toml.
    Day {
        name: String,
        /// YYYY-MM-DD; today when left out.
        #[arg(long)]
        date: Option<String>,
        /// As JSON. A day runs 4am to 4am, so a minute past 1439 is the
        /// next morning.
        #[arg(long)]
        json: bool,
    },
}

#[derive(clap::Subcommand, Debug)]
pub enum RoutineCmd {
    /// The routine, a waiting draft, and the activities an anchor may name.
    Show {
        name: String,
        #[arg(long)]
        json: bool,
    },
    /// Edit routine.toml in $EDITOR, starting from a commented template.
    Edit {
        name: String,
        /// Edit the waiting draft instead, before approving it.
        #[arg(long)]
        draft: bool,
    },
    /// Make the waiting draft the routine, after reading it. The routine it
    /// replaces is kept as routine.previous.toml.
    Approve {
        name: String,
        #[arg(long)]
        yes: bool,
    },
    /// Drop the waiting draft; the routine is untouched.
    Discard { name: String },
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
            | Cmd::Routine {
                cmd: RoutineCmd::Show { .. }
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
    for &p in &personas {
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
        // The personas this run was asked about, as above — `--name` narrows
        // this too (review of #481).
        for &p in &personas {
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
        "life": p.settings.life,
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

/// How much `p`'s replies repeated earlier ones, read from its chats active
/// over the window `mecha persona show` reads (`persona::echo`).
fn recent_echoes(store: &Store, p: &Persona) -> anyhow::Result<persona::echo::Echoes> {
    let window = std::time::Duration::from_secs(persona::echo::SHOWN_DAYS as u64 * 86_400);
    let since = std::time::SystemTime::now()
        .checked_sub(window)
        .unwrap_or(std::time::UNIX_EPOCH);
    persona::echo::echoes(&store.sessions_dir(&p.name), since)
}

/// One line for `mecha persona show`: the echo reading in words.
fn echo_line(echoes: &anyhow::Result<persona::echo::Echoes>) -> String {
    let days = persona::echo::SHOWN_DAYS;
    let e = match echoes {
        Err(e) => return format!("unreadable — {e:#}"),
        Ok(e) => e,
    };
    let unreadable = match e.unreadable {
        0 => String::new(),
        n => format!(" ({n} chat(s) could not be read)"),
    };
    match e.max {
        None => format!("no replies to compare in chats from the last {days} days{unreadable}"),
        Some(max) => format!(
            "{} of {} replies in {} chat(s) from the last {days} days repeated an earlier one (highest {max:.2}){unreadable}",
            e.repeated, e.replies, e.chats
        ),
    }
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
            // The echo reads a week of transcripts, so only `show` asks for
            // it — not `list`, nor the confirmations that describe a persona.
            let echoes = recent_echoes(&store, p);
            if json {
                let mut summary = summary_json(&store, &lib, p);
                // `null` when unreadable — never an empty reading.
                summary["echo"] = serde_json::to_value(echoes.ok())?;
                println!("{}", serde_json::to_string_pretty(&summary)?);
            } else {
                describe(&store, &lib, p);
                println!("  echo:         {}", echo_line(&echoes));
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
        Cmd::Routine { cmd } => routine_cmd(dir, cmd, edit_file)?,
        Cmd::Day { name, date, json } => day_cmd(dir, &name, date.as_deref(), json)?,
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

/// `mecha persona routine`: the owner's door to `routine.toml` and a
/// waiting draft. The editor writes the file itself, as `edit` does, and
/// what it left is read back through the sampler's own door.
fn day_cmd(dir: &Path, name: &str, date: Option<&str>, json: bool) -> Result<()> {
    use mecha_core::persona::world;
    // The owner's calendar day, not the machine's UTC one.
    let cfg = mecha_core::config::Config::load(&std::env::current_dir()?)?;
    let today = match cfg.agent.timezone() {
        Some(tz) => chrono::Utc::now().with_timezone(&tz).date_naive(),
        None => chrono::Local::now().date_naive(),
    };
    let date = match date {
        None => today,
        Some(d) => chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .with_context(|| format!("`{d}` is not a date (YYYY-MM-DD)"))?,
    };
    let name = name.trim().to_lowercase();
    let day = world::day(dir, &name, date, today)?;
    print!("{}", render_day(&name, &day, json)?);
    Ok(())
}

/// A day as `mecha persona day` prints it.
fn render_day(name: &str, day: &mecha_core::persona::sampler::Day, json: bool) -> Result<String> {
    use mecha_core::persona::sampler;
    use std::fmt::Write;
    let mut out = String::new();
    let clock = |m: u32| format!("{:02}:{:02}", (m / 60) % 24, m % 60);
    let company = |s: &sampler::Stretch| -> Option<Vec<&'static str>> {
        s.company
            .as_ref()
            .map(|c| c.iter().map(|k| k.id()).collect())
    };
    if json {
        let value = serde_json::json!({
            "date": day.date.to_string(),
            "cell": day.cell.id(),
            "day_type": day.day_type.id(),
            "backoffs": day.backoffs,
            "stretches": day.stretches.iter().map(|s| serde_json::json!({
                "activity": s.activity.id(),
                "start_minute": s.start,
                "end_minute": s.end,
                "company": company(s),
                "anchor": s.anchor,
            })).collect::<Vec<_>>(),
            "carries": day.carries.as_ref().map(|c| serde_json::json!({
                "activity": c.activity.id(),
                "anchor": c.label,
                "until_minute": c.until,
            })),
        });
        writeln!(out, "{}", serde_json::to_string_pretty(&value)?)?;
        return Ok(out);
    }
    writeln!(
        out,
        "{name}, {} {} ({}, {})",
        day.date.format("%A"),
        day.date,
        day.day_type.id(),
        day.cell.id()
    )?;
    let mut midnight = false;
    for s in &day.stretches {
        if !midnight && s.start >= 24 * 60 {
            writeln!(out, "  ── after midnight ──")?;
            midnight = true;
        }
        let who = match company(s) {
            None => String::new(),
            Some(c) if c == ["alone"] => "alone".into(),
            Some(c) => format!("with {}", c.join(", ")),
        };
        let anchor = s
            .anchor
            .as_deref()
            .map(|a| format!("[{a}] "))
            .unwrap_or_default();
        writeln!(
            out,
            "  {}–{}  {:<16} {anchor}{who}",
            clock(s.start),
            clock(s.end),
            s.activity.id()
        )?;
    }
    if day.backoffs > 0 {
        writeln!(
            out,
            "\n{} draws read past the {} rows, to the pool or a nearer hour.",
            day.backoffs,
            day.cell.id()
        )?;
    }
    Ok(out)
}

fn routine_cmd(dir: &Path, cmd: RoutineCmd, edit_file: &dyn Fn(&Path) -> Result<()>) -> Result<()> {
    use mecha_core::persona::activity::Activity;
    use mecha_core::persona::routine::{self, Routine};
    // By folder, as `edit` finds a persona: one whose persona.toml no longer
    // parses still has a week to look at. `routine::read` checks the name.
    let named = |name: &str| -> Result<String> { Ok(name.trim().to_lowercase()) };
    let anchors = |r: &Routine| -> Vec<serde_json::Value> {
        r.anchors
            .iter()
            .map(|a| {
                serde_json::json!({
                    "what": a.what,
                    "activity": a.activity,
                    "days": (0..7).filter(|d| a.on(*d)).collect::<Vec<_>>(),
                    "start_minute": a.start,
                    "minutes": a.minutes,
                    "chance": a.chance,
                })
            })
            .collect()
    };
    match cmd {
        RoutineCmd::Show { name, json } => {
            let name = named(&name)?;
            let texts = routine::read(dir, &name)?;
            let parsed = texts.routine.as_deref().map(routine::parse).transpose();
            if json {
                let out = serde_json::json!({
                    // `null` with an error beside it when it does not load —
                    // never an empty week.
                    "routine": parsed.as_ref().ok().map(|r| r.as_ref().map(anchors)),
                    "cell": parsed.as_ref().ok().and_then(|r| r.as_ref()).and_then(|r| r.cell).map(|c| c.id()),
                    "error": parsed.as_ref().err().map(|e| format!("{e:#}")),
                    "draft": texts.draft,
                    "activities": Activity::ALL.iter().map(|a| a.id()).collect::<Vec<_>>(),
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }
            match (&texts.routine, &parsed) {
                (None, _) => println!(
                    "`{name}` has no routine, so no day is drawn for her: it names her cell, \
                     the kind of week her days come from. \
                     `mecha persona routine edit {name}` writes one."
                ),
                (Some(_), Err(e)) => println!("routine.toml does not load: {e:#}"),
                (Some(text), Ok(r)) => {
                    println!("── routine.toml ──\n{}", text.trim());
                    if r.as_ref().is_some_and(|r| r.cell.is_none()) {
                        println!(
                            "\nIt names no cell, so no day is drawn for her until it does \
                             (`cell = \"student\"`, `full_time`, `full_time_parent`, \
                             `part_time`, `at_home` or `retired`)."
                        );
                    }
                }
            }
            if let Some(draft) = &texts.draft {
                println!(
                    "\n── routine.draft.toml (waiting on you) ──\n{}\n\n\
                     `mecha persona routine approve {name}` makes it the routine; \
                     `discard` drops it.",
                    draft.trim()
                );
            }
            let ids: Vec<&str> = Activity::ALL.iter().map(|a| a.id()).collect();
            println!("\nActivities an anchor may name: {}", ids.join(", "));
        }
        RoutineCmd::Edit { name, draft } => {
            let name = named(&name)?;
            let file = if draft {
                if routine::read(dir, &name)?.draft.is_none() {
                    bail!("`{name}` has no routine draft waiting");
                }
                dir.join(&name).join(routine::DRAFT)
            } else {
                routine::ensure(dir, &name)?
            };
            edit_file(&file)?;
            // Read back through the store's door, so a save is checked as
            // strictly as the sampler will load it.
            let texts = routine::read(dir, &name)?;
            let Some(text) = (if draft { texts.draft } else { texts.routine }) else {
                println!(
                    "{} was removed{}.",
                    file.file_name().unwrap_or_default().to_string_lossy(),
                    if draft {
                        ""
                    } else {
                        ": no day is drawn for her until a routine names her cell"
                    }
                );
                return Ok(());
            };
            match routine::parse(&text) {
                Ok(r) => println!(
                    "Saved: {} anchor(s), {}{}.",
                    r.anchors.len(),
                    match r.cell {
                        Some(c) => format!("cell {}", c.id()),
                        None => "no cell, so no day is drawn yet".into(),
                    },
                    if draft {
                        format!(" — `mecha persona routine approve {name}` when it reads right")
                    } else {
                        String::new()
                    }
                ),
                Err(e) => bail!(
                    "{} was saved but does not load, so {}: {e:#}. \
                     `mecha persona routine edit {name}{}` to fix it.",
                    file.file_name().unwrap_or_default().to_string_lossy(),
                    if draft {
                        "it cannot be approved yet"
                    } else {
                        "no day is built from it"
                    },
                    if draft { " --draft" } else { "" },
                ),
            }
        }
        RoutineCmd::Approve { name, yes } => {
            let name = named(&name)?;
            let texts = routine::read(dir, &name)?;
            let Some(draft) = texts.draft else {
                bail!("`{name}` has no routine draft waiting");
            };
            println!("── routine.draft.toml ──\n{}", draft.trim());
            if texts.routine.is_some() {
                println!("\nIt replaces the routine `{name}` has now, which is kept as routine.previous.toml.");
            }
            if !yes && !confirm(&format!("Make this `{name}`'s routine?"))? {
                println!("Not approved.");
                return Ok(());
            }
            let r = routine::approve_draft_as_shown(dir, &name, &persona::text_digest(&draft))?;
            println!(
                "Approved: `{name}`'s routine has {} anchor(s).",
                r.anchors.len()
            );
        }
        RoutineCmd::Discard { name } => {
            let name = named(&name)?;
            routine::discard_draft(dir, &name)?;
            println!("Dropped `{name}`'s routine draft.");
        }
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
                    "Forgotten: {} episodes, {} facts, {} shared copies, {} scene records.",
                    out.episodes, out.facts, out.shared, out.scenes
                );
                if out.scenes_unreadable > 0 {
                    println!(
                        "{} scene records could not be read, so could not be shown to be this \
                         chat's, and were kept (in {}).",
                        out.scenes_unreadable,
                        dir.join(&name).join("scene").display()
                    );
                }
                if out.scenes_failed {
                    println!("The scene records could not be reached and were not forgotten.");
                }
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

    #[test]
    fn a_day_prints_its_clock_company_anchors_and_the_turn_of_midnight() {
        use mecha_core::persona::activity::Activity;
        use mecha_core::persona::atus::{Cell, Company, DayType};
        use mecha_core::persona::sampler::{Carry, Day, Stretch};
        let stretch =
            |activity, start, end, company: Option<Vec<Company>>, anchor: Option<&str>| Stretch {
                activity,
                start,
                end,
                company,
                anchor: anchor.map(Into::into),
            };
        let day = Day {
            date: chrono::NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
            cell: Cell::Student,
            day_type: DayType::Weekday,
            stretches: vec![
                stretch(Activity::Sleep, 240, 600, None, None),
                stretch(
                    Activity::Class,
                    600,
                    675,
                    Some(vec![Company::Alone]),
                    Some("lecture"),
                ),
                stretch(
                    Activity::Eating,
                    675,
                    1500,
                    Some(vec![Company::Partner, Company::Friend]),
                    None,
                ),
                stretch(
                    Activity::Work,
                    1500,
                    1680,
                    Some(vec![Company::Colleague]),
                    Some("ward"),
                ),
            ],
            backoffs: 2,
            carries: Some(Carry {
                activity: Activity::Work,
                label: "ward".into(),
                until: 360,
            }),
        };
        let text = render_day("juno", &day, false).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "juno, Monday 2026-11-02 (weekday, student)");
        assert!(
            lines[1].starts_with("  04:00–10:00  sleep") && lines[1].trim_end().ends_with("sleep")
        );
        assert!(lines[2].ends_with("[lecture] alone"), "{}", lines[2]);
        assert!(lines[3].ends_with("with partner, friend"), "{}", lines[3]);
        assert_eq!(lines[4], "  ── after midnight ──");
        assert!(lines[5].starts_with("  01:00–04:00  work"), "{}", lines[5]);
        assert!(text.contains("2 draws read past the student rows"));
        let json: serde_json::Value =
            serde_json::from_str(&render_day("juno", &day, true).unwrap()).unwrap();
        assert_eq!(json["stretches"][3]["end_minute"], 1680);
        assert_eq!(json["stretches"][0]["company"], serde_json::Value::Null);
        assert_eq!(json["stretches"][1]["company"][0], "alone");
        assert_eq!(json["carries"]["until_minute"], 360);
        assert_eq!(json["backoffs"], 2);
    }

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

    /// The echo reads a week of transcripts, so only `show` asks for it: the
    /// summary `list --json` prints once per persona must not carry it.
    #[test]
    fn the_summary_list_prints_reads_no_transcripts() {
        let dir = scratch();
        let lib_dir = dir.join("imagelib");
        let store = dir.join("personas");
        run(&store, &lib_dir, new("ada")).unwrap();
        let loaded = Store::load(&store);
        let (lib, _) = Library::load(&lib_dir);
        let summary = summary_json(&loaded, &lib, loaded.get("ada").unwrap());
        assert!(summary.get("echo").is_none(), "{summary}");
        // The one place the `[life]` switches are read back per persona:
        // the settings form does not carry them yet (review of #775).
        assert_eq!(summary["life"]["world"], false, "{summary}");
        std::fs::remove_dir_all(dir).ok();
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
    fn a_routine_starts_from_the_template_and_a_draft_waits_for_approval() {
        use mecha_core::persona::routine;
        let dir = scratch();
        let lib = dir.join("imagelib");
        let store = dir.join("personas");
        run(&store, &lib, new("mara")).unwrap();
        let edit = |draft: bool, write: &'static str| {
            run_with(
                &store,
                &lib,
                Cmd::Routine {
                    cmd: RoutineCmd::Edit {
                        name: "Mara".into(),
                        draft,
                    },
                },
                &move |path: &Path| {
                    let before = std::fs::read_to_string(path).unwrap();
                    std::fs::write(path, format!("{before}{write}")).unwrap();
                    Ok(())
                },
            )
        };
        // The editor opens on the template, and an anchor appended to it loads.
        edit(false, "[[anchor]]\nactivity = \"work\"\ndays = [\"weekdays\"]\nat = \"09:00\"\nminutes = 480\n").unwrap();
        let pdir = store.join("mara");
        let text = std::fs::read_to_string(pdir.join(routine::FILE)).unwrap();
        assert!(text.starts_with(routine::TEMPLATE), "{text}");
        assert_eq!(routine::load(&pdir).unwrap().unwrap().anchors.len(), 1);

        // A save that does not load is kept as written and says how to fix it.
        let refused = edit(false, "[[anchor]]\nactivity = \"nap\"\n").unwrap_err();
        assert!(
            format!("{refused}").contains("edit mara` to fix"),
            "{refused}"
        );
        let loaded = Store::load(&store);
        let problems = loaded.problems(loaded.get("mara").unwrap(), &Library::load(&lib).0);
        assert!(
            problems
                .iter()
                .any(|p| p.contains("routine.toml does not load")),
            "{problems:?}"
        );
        std::fs::write(pdir.join(routine::FILE), &text).unwrap();

        assert!(format!("{}", edit(true, "").unwrap_err()).contains("no routine draft"));
        let draft = "[[anchor]]\nwhat = \"choir\"\nactivity = \"hobbies\"\ndays = [\"wed\"]\nat = \"19:30\"\nminutes = 90\n";
        routine::stage_draft(&store, "mara", draft, Origin::ModelClean).unwrap();
        edit(true, "chance = 0.8\n").unwrap();
        run(
            &store,
            &lib,
            Cmd::Routine {
                cmd: RoutineCmd::Approve {
                    name: "mara".into(),
                    yes: true,
                },
            },
        )
        .unwrap();
        let r = routine::load(&pdir).unwrap().unwrap();
        assert_eq!(
            (r.anchors.len(), r.anchors[0].chance),
            (1, 0.8),
            "the owner's edit to the draft was lost"
        );
        assert_eq!(
            std::fs::read_to_string(pdir.join(routine::PREVIOUS)).unwrap(),
            text
        );
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
