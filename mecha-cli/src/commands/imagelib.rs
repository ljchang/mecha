//! `mecha imagelib` — the owner's door into the image library
//! (`docs/IMAGE-COMPILER-DESIGN.md` §6).
//!
//! Everything made here is the owner's and approved on creation; a model can
//! only stage a candidate (`image_library_propose`), and this is where a
//! candidate becomes usable. An **untrusted** candidate — proposed in a
//! conversation holding third-party content — is approved only after its text
//! has been shown and the question answered: `--yes` does not skip it, because
//! approved text rides into every future prompt that names the character.
//!
//! It builds no provider and connects to nothing, so the library can be
//! inspected and edited with the model and the image server both down.

use anyhow::{bail, Context, Result};
use mecha_core::imagelib::{self, Entry, Kind, Library, NewEntry, Origin, Status};
use std::path::PathBuf;

use crate::GlobalOpts;

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum KindArg {
    Character,
    Style,
}

impl From<KindArg> for Kind {
    fn from(k: KindArg) -> Kind {
        match k {
            KindArg::Character => Kind::Character,
            KindArg::Style => Kind::Style,
        }
    }
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Approved characters and styles; `--all` adds candidates.
    List {
        #[arg(long)]
        all: bool,
        /// Emit JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// One entry: its text, where it came from, and its portrait's path.
    Show {
        name: String,
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
    },
    /// Add a character — yours, so approved at once.
    AddCharacter {
        name: String,
        /// A front-facing portrait (PNG, JPEG or WebP).
        #[arg(long)]
        portrait: PathBuf,
        /// A short description, including build and height.
        #[arg(long)]
        description: String,
        /// The seed that drew the portrait, if it was generated.
        #[arg(long)]
        seed: Option<u64>,
        /// Hide it while browsing unless locked content is shown.
        #[arg(long)]
        locked: bool,
    },
    /// Add a style — yours, so approved at once.
    AddStyle {
        name: String,
        #[arg(long)]
        text: String,
        #[arg(long)]
        locked: bool,
    },
    /// Make a candidate usable, after reading what it says.
    Approve {
        name: String,
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
        /// Skip the question — refused for a candidate proposed in a
        /// conversation that held third-party content.
        #[arg(long)]
        yes: bool,
        /// Approve exactly the text another surface displayed: the digest
        /// the web page computed from what it showed (`shown_digest`). With
        /// it, `--yes` is allowed for an untrusted candidate, because the
        /// text was read — and refused if it has changed since.
        #[arg(long)]
        shown: Option<String>,
    },
    /// Set the password that shows locked entries while browsing the web
    /// library. Read from the terminal without echo, or from stdin.
    SetLockPassword,
    /// Remove a candidate.
    Reject {
        name: String,
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
    },
    /// Remove any entry (moved aside under `removed/`, not deleted).
    Remove {
        name: String,
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
        #[arg(long)]
        yes: bool,
    },
    /// Hide an entry while browsing. Generation is unaffected.
    Lock {
        name: String,
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
    },
    /// Show an entry while browsing.
    Unlock {
        name: String,
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
    },
    /// A new version of an entry; the old one is kept in its history.
    Update {
        name: String,
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
        /// A character's new description, or a style's new text.
        #[arg(long, alias = "description")]
        text: Option<String>,
        /// A character's new portrait.
        #[arg(long)]
        portrait: Option<PathBuf>,
        #[arg(long)]
        seed: Option<u64>,
    },
}

/// One y/N question, where EOF is "no" — the outbox's rule: a review surface
/// reached by a script with no stdin must not release anything.
fn confirm(question: &str) -> Result<bool> {
    use std::io::Write;
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
        println!();
        return Ok(false);
    }
    Ok(line.trim().eq_ignore_ascii_case("y"))
}

fn atty_stdin() -> bool {
    // SAFETY: isatty reads no memory of ours.
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
}

/// One line from stdin, without echo when stdin is a terminal. The password
/// is never an argument: arguments land in shell history and `ps`.
fn read_secret(prompt: &str) -> Result<String> {
    use std::io::Write;
    let tty = atty_stdin();
    let mut saved: Option<libc::termios> = None;
    if tty {
        eprint!("{prompt}");
        std::io::stderr().flush()?;
        // SAFETY: tcgetattr fills a zeroed termios for a descriptor we hold.
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut t) == 0 {
                saved = Some(t);
                t.c_lflag &= !libc::ECHO;
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t);
            }
        }
    }
    let mut line = String::new();
    let read = std::io::stdin().read_line(&mut line);
    if let Some(t) = saved {
        // SAFETY: restoring the settings read above.
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t);
        }
        eprintln!();
    }
    read?;
    Ok(line.trim_end_matches(['\n', '\r']).to_string())
}

/// The one entry a typed name means. Names are unique per kind, so a name
/// held by a character and a style needs `--kind`.
fn resolve<'a>(lib: &'a Library, name: &str, kind: Option<KindArg>) -> Result<&'a Entry> {
    let name = name.trim().to_lowercase();
    if let Some(kind) = kind {
        return lib
            .get(kind.into(), &name)
            .with_context(|| format!("no {} named `{name}`", Kind::from(kind).label()));
    }
    match lib.find(&name).as_slice() {
        [] => bail!("nothing in the image library is named `{name}`"),
        [one] => Ok(one),
        _ => bail!("both a character and a style are named `{name}`; pass --kind"),
    }
}

fn origin_label(o: Origin) -> &'static str {
    match o {
        Origin::Owner => "yours",
        Origin::ModelClean => "proposed by the model (clean conversation)",
        Origin::ModelUntrusted => {
            "proposed by the model in a conversation holding third-party content"
        }
    }
}

fn describe(lib: &Library, e: &Entry) {
    println!("{} {} (v{})", e.kind.label(), e.name, e.version);
    println!(
        "  status:  {}{}",
        match e.status {
            Status::Approved => "approved",
            Status::Candidate => "candidate",
        },
        if e.locked { ", locked" } else { "" }
    );
    println!("  origin:  {}", origin_label(e.origin));
    println!("  text:    {}", e.text);
    if let Some(blob) = &e.portrait {
        println!("  portrait: {}", lib.blob_path(blob).display());
    }
    if let Some(seed) = e.source_seed {
        println!("  seed:    {seed}");
    }
}

fn read_picture(path: &PathBuf) -> Result<Vec<u8>> {
    let meta = std::fs::metadata(path).with_context(|| format!("reading {}", path.display()))?;
    if !meta.is_file() {
        bail!("{} is not a file", path.display());
    }
    if meta.len() > imagelib::MAX_PORTRAIT_BYTES {
        bail!(
            "{} is over the {} MB portrait cap",
            path.display(),
            imagelib::MAX_PORTRAIT_BYTES / (1024 * 1024)
        );
    }
    Ok(std::fs::read(path)?)
}

pub async fn execute(_global: &GlobalOpts, args: Args) -> Result<()> {
    let dir = Library::default_dir()?;
    run(&dir, args.cmd)
}

fn run(dir: &std::path::Path, cmd: Cmd) -> Result<()> {
    let (lib, errors) = Library::load(dir);
    for e in &errors {
        eprintln!("mecha: {} did not load — {}", e.path.display(), e.why);
    }
    match cmd {
        Cmd::List { all, json } => {
            let entries: Vec<&Entry> = lib
                .all()
                .iter()
                .filter(|e| all || e.status == Status::Approved)
                .collect();
            if json {
                let out: Vec<_> = entries
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "kind": e.kind, "name": e.name, "version": e.version,
                            "status": e.status, "origin": e.origin, "locked": e.locked,
                            "text": e.text,
                            "portrait": e.portrait.as_deref().map(|b| lib.blob_path(b)),
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }
            if entries.is_empty() {
                println!("The image library at {} is empty.", dir.display());
                return Ok(());
            }
            for e in entries {
                let flags = [
                    (e.status == Status::Candidate).then_some("candidate"),
                    e.locked.then_some("locked"),
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
                println!(
                    "{:9} {} (v{}){flags}: {}",
                    e.kind.label(),
                    e.name,
                    e.version,
                    e.text
                );
            }
            let pending = lib.candidates().count();
            if !all && pending > 0 {
                println!("\n{pending} candidate(s) waiting — `mecha imagelib list --all`.");
            }
        }
        Cmd::Show { name, kind } => describe(&lib, resolve(&lib, &name, kind)?),
        Cmd::AddCharacter {
            name,
            portrait,
            description,
            seed,
            locked,
        } => {
            let e = imagelib::create(
                dir,
                NewEntry {
                    kind: Kind::Character,
                    name,
                    text: description,
                    portrait: Some(read_picture(&portrait)?),
                    source_seed: seed,
                    origin: Origin::Owner,
                    locked,
                },
            )?;
            println!("Added character `{}`.", e.name);
        }
        Cmd::AddStyle { name, text, locked } => {
            let e = imagelib::create(
                dir,
                NewEntry {
                    kind: Kind::Style,
                    name,
                    text,
                    portrait: None,
                    source_seed: None,
                    origin: Origin::Owner,
                    locked,
                },
            )?;
            println!("Added style `{}`.", e.name);
        }
        Cmd::Approve {
            name,
            kind,
            yes,
            shown,
        } => {
            let e = resolve(&lib, &name, kind)?;
            if e.status == Status::Approved {
                bail!("`{}` is already approved", e.name);
            }
            if let Some(shown) = shown {
                // The web door: the page displayed the text and sent back
                // its digest; core re-reads and compares at the write.
                imagelib::approve_as_shown(dir, e.kind, &e.name, &shown)?;
                println!("Approved `{}`.", e.name);
                return Ok(());
            }
            describe(&lib, e);
            if e.origin == Origin::ModelUntrusted && yes {
                bail!(
                    "`{}` was proposed in a conversation holding third-party content, and once \
                     approved its text rides into every prompt that names it. Read it above and \
                     answer the question — run again without --yes.",
                    e.name
                );
            }
            if !yes && !confirm(&format!("Approve {} `{}`?", e.kind.label(), e.name))? {
                println!("Not approved.");
                return Ok(());
            }
            imagelib::approve(dir, e.kind, &e.name)?;
            println!("Approved `{}`.", e.name);
        }
        Cmd::SetLockPassword => {
            let first = read_secret("New lock password: ")?;
            if atty_stdin() {
                let again = read_secret("Again: ")?;
                if again != first {
                    bail!("the two entries differ; nothing was changed");
                }
            }
            imagelib::set_lock_password(dir, &first)?;
            println!("Lock password set. Locked entries show in the web library once unlocked.");
        }
        Cmd::Reject { name, kind } => {
            let e = resolve(&lib, &name, kind)?;
            if e.status != Status::Candidate {
                bail!(
                    "`{}` is approved; `mecha imagelib remove` removes it",
                    e.name
                );
            }
            imagelib::reject(dir, e.kind, &e.name)?;
            println!("Rejected `{}`.", e.name);
        }
        Cmd::Remove { name, kind, yes } => {
            let e = resolve(&lib, &name, kind)?;
            describe(&lib, e);
            if !yes && !confirm(&format!("Remove {} `{}`?", e.kind.label(), e.name))? {
                println!("Kept.");
                return Ok(());
            }
            imagelib::remove(dir, e.kind, &e.name)?;
            println!("Removed `{}` (kept under removed/).", e.name);
        }
        Cmd::Lock { name, kind } => {
            let e = resolve(&lib, &name, kind)?;
            imagelib::set_locked(dir, e.kind, &e.name, true)?;
            println!("Locked `{}`: hidden while browsing, still usable.", e.name);
        }
        Cmd::Unlock { name, kind } => {
            let e = resolve(&lib, &name, kind)?;
            imagelib::set_locked(dir, e.kind, &e.name, false)?;
            println!("Unlocked `{}`.", e.name);
        }
        Cmd::Update {
            name,
            kind,
            text,
            portrait,
            seed,
        } => {
            let e = resolve(&lib, &name, kind)?;
            let portrait = portrait.as_ref().map(read_picture).transpose()?;
            let updated = imagelib::update(dir, e.kind, &e.name, text, portrait, seed)?;
            println!("`{}` is now v{}.", updated.name, updated.version);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mecha-imagelib-cli-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn propose(dir: &std::path::Path, name: &str, origin: Origin) {
        imagelib::create(
            dir,
            NewEntry {
                kind: Kind::Style,
                name: name.into(),
                text: "watercolour".into(),
                portrait: None,
                source_seed: None,
                origin,
                locked: false,
            },
        )
        .unwrap();
    }

    fn approve(dir: &std::path::Path, name: &str) -> Result<()> {
        run(
            dir,
            Cmd::Approve {
                name: name.into(),
                kind: None,
                yes: true,
                shown: None,
            },
        )
    }

    #[test]
    fn yes_never_skips_reading_an_untrusted_candidate() {
        let dir = scratch();
        propose(&dir, "dirty", Origin::ModelUntrusted);
        propose(&dir, "clean", Origin::ModelClean);
        let refused = approve(&dir, "dirty").unwrap_err();
        assert!(format!("{refused}").contains("without --yes"), "{refused}");
        approve(&dir, "clean").unwrap();
        let (lib, _) = Library::load(&dir);
        assert_eq!(
            lib.get(Kind::Style, "dirty").unwrap().status,
            Status::Candidate
        );
        assert_eq!(
            lib.get(Kind::Style, "clean").unwrap().status,
            Status::Approved
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_web_door_approves_an_untrusted_candidate_only_as_shown() {
        let dir = scratch();
        propose(&dir, "dirty", Origin::ModelUntrusted);
        let shown = {
            let (lib, _) = Library::load(&dir);
            imagelib::shown_digest(lib.get(Kind::Style, "dirty").unwrap())
        };
        let approve_shown = |digest: &str| {
            run(
                &dir,
                Cmd::Approve {
                    name: "dirty".into(),
                    kind: None,
                    yes: true,
                    shown: Some(digest.into()),
                },
            )
        };
        assert!(approve_shown("not-what-was-shown").is_err());
        approve_shown(&shown).unwrap();
        let (lib, _) = Library::load(&dir);
        assert_eq!(
            lib.get(Kind::Style, "dirty").unwrap().status,
            Status::Approved
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn locking_is_per_item_and_changes_nothing_else() {
        let dir = scratch();
        propose(&dir, "noir", Origin::Owner);
        run(
            &dir,
            Cmd::Lock {
                name: "noir".into(),
                kind: None,
            },
        )
        .unwrap();
        let (lib, _) = Library::load(&dir);
        let e = lib.get(Kind::Style, "noir").unwrap();
        assert!(e.locked);
        assert_eq!((e.status, e.version), (Status::Approved, 1));
        run(
            &dir,
            Cmd::Unlock {
                name: "noir".into(),
                kind: None,
            },
        )
        .unwrap();
        assert!(
            !Library::load(&dir)
                .0
                .get(Kind::Style, "noir")
                .unwrap()
                .locked
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn reject_is_for_candidates_only() {
        let dir = scratch();
        propose(&dir, "mine", Origin::Owner);
        let refused = run(
            &dir,
            Cmd::Reject {
                name: "mine".into(),
                kind: None,
            },
        )
        .unwrap_err();
        assert!(format!("{refused}").contains("remove"), "{refused}");
        std::fs::remove_dir_all(dir).ok();
    }
}
