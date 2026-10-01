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

pub async fn execute(_global: &GlobalOpts, args: Args) -> Result<()> {
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
    ) {
        super::features::require(mecha_core::feature::Feature::Personas)?;
    }
    let dir = Store::default_dir()?;
    let lib_dir = Library::default_dir()?;
    run(&dir, &lib_dir, args.cmd)
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
