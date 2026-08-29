//! `mecha charter` — the standing priorities in `~/.mecha/charter.toml`, as
//! the owner's run would see them, and `edit` to change them.
//!
//! **This file used to say there would never be an `edit`.** That was a
//! misstatement of the invariant rather than a decision anybody made: the
//! rule is *no model authors a priority*, and it was written down as *no
//! verb writes the file* — which was already untrue of the TUI's `/charter`
//! (`e` hands the file to `$EDITOR`) and of the web settings page (a
//! validated save). Stating it as the absence of a verb made the CLI the
//! only surface where the owner could not edit their own document, which
//! protects nothing. See `mecha_core::charter`'s module doc for the
//! invariant said properly.
//!
//! So `edit` hands the terminal to `$EDITOR` on the file itself, exactly as
//! the TUI does and through the same helper. mecha writes one thing here
//! ever: the comments-only template, when no file exists yet, because `vi`
//! on an empty buffer is how a first charter ends up shaped wrong. There is
//! still no `--add`, no `--set`, and no tool — nothing that would let a
//! model put a sentence in this file.
//!
//! Validation feedback is what makes this better than a hand-run `vi`: a
//! duplicate id or a typo'd table name is reported the moment the editor
//! closes, not at the next run's startup where it scrolls past.

use anyhow::Result;
use mecha_core::charter::Charter;

use crate::editor::CharterEdit;
use crate::GlobalOpts;

/// **`args_conflicts_with_subcommands`, not `conflicts_with` on the flag.**
/// A subcommand is not an argument id, so `#[arg(conflicts_with = "cmd")]`
/// matches nothing and silently keeps the behaviour it was meant to refuse —
/// a fix that does not fix, which is worse than the no-op it replaces because
/// it reads as handled.
#[derive(clap::Args, Debug)]
#[command(args_conflicts_with_subcommands = true)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,

    /// Emit JSON instead of a table.
    ///
    /// **Refused alongside a subcommand rather than ignored.** clap accepts
    /// parent arguments before a subcommand, so `mecha charter --json edit`
    /// parsed, set this, and then `execute` matched on `cmd` and never read
    /// it — a flag that did nothing and said nothing, which is the shape
    /// `commands::setup`'s own arg block argues against and fixes the same
    /// way. `edit` hands over an editor; there is no JSON it could emit.
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Open the charter in `$EDITOR`, creating a commented template first if
    /// there is no file yet.
    Edit,
}

pub async fn execute(_global: &GlobalOpts, args: Args) -> Result<()> {
    match args.cmd {
        Some(Cmd::Edit) => edit(),
        None => show(args.json),
    }
}

/// Hand the file to `$EDITOR` and say what actually landed.
///
/// The outcome is read off the *file*, never off the editor's exit code —
/// see `editor::edit_charter_with` for the two cases where those disagree.
fn edit() -> Result<()> {
    let path = Charter::default_path()?;
    // Asked here rather than returned from `edit_charter_with`: that helper
    // reports what the *edit* did, and creating the file is something that
    // happened before it. `CharterEdit::TemplateCreated` is not the same
    // fact — it means created *and* nothing added, so a first edit that types
    // a line comes back as `Saved`.
    let existed = path.is_file();
    let outcome = crate::editor::edit_charter_with(&path, crate::editor::edit_file)?;
    if !existed {
        println!("created {} from a commented template", path.display());
    }
    // Every "saved" arm carries the same honest clause: the charter is
    // rendered into the system prompt when an agent is built, so an edit
    // reaches the *next* run rather than one already in flight.
    match outcome {
        CharterEdit::TemplateCreated => println!(
            "no priorities yet — the template is in place; `mecha charter edit` again to fill it in"
        ),
        CharterEdit::Unchanged => println!("unchanged"),
        CharterEdit::Saved => {
            println!("saved — it rides in the prompt from the next run");
            show_lines(&Charter::load(&path)?);
        }
        // Louder than "saved", because the cost is every future run rather
        // than this command.
        CharterEdit::SavedButInvalid(e) => {
            eprintln!(
                "saved, but it will NOT load: {e}\n\
                 every run starts un-chartered until this parses — `mecha charter edit` to fix it"
            );
            std::process::exit(1);
        }
        CharterEdit::EditorFailedButChanged { error, loads } => match loads {
            None => println!(
                "the editor exited with an error ({error}), but the file changed and loads"
            ),
            Some(e) => {
                eprintln!("the editor exited with an error ({error}); the file changed and will NOT load: {e}");
                std::process::exit(1);
            }
        },
        CharterEdit::EditorFailed(e) => {
            eprintln!("charter unchanged: {e}");
            std::process::exit(1);
        }
    }
    Ok(())
}

fn show_lines(charter: &Charter) {
    for (i, line) in charter.lines().iter().enumerate() {
        println!("  {}. {} — {}", i + 1, line.id, line.text);
    }
}

/// The read, unchanged: the lines in rank order, or the one honest failure.
fn show(json: bool) -> Result<()> {
    let path = Charter::default_path()?;
    let charter = match Charter::load(&path) {
        Ok(charter) => charter,
        Err(e) => {
            // A charter that fails to parse is not silently ignored the way
            // a missing file is: it's the one document ranking every other
            // priority, and a run that started with an empty one because of
            // a typo would be silently un-chartered.
            //
            // Structured even on failure when `--json` was asked for, on
            // `mecha skills --json`'s rule that machine output reports an
            // error in the payload rather than only as a bare exit code —
            // this is doctor's own remedy for a broken charter, and a
            // scripted consumer of it deserves the same contract doctor's
            // own `--json` gives everything else.
            if json {
                // `{e:#}` for the full chain, not `.to_string()`/`{e}` —
                // anyhow's default `Display` prints only the outermost
                // context ("parsing <path>") and drops the actual TOML error
                // underneath it, same as the human-facing branch three lines
                // down already does.
                let out = serde_json::json!({
                    "path": path,
                    "error": format!("{e:#}"),
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                eprintln!("mecha: charter at {} did not load — {e:#}", path.display());
            }
            std::process::exit(1);
        }
    };

    if json {
        let out = serde_json::json!({
            "path": path,
            // A missing file and a present-but-empty one both load as an
            // empty `Charter` — the same distinction the plain-text branch
            // below makes by checking this, and doctor makes as two
            // different findings, so a scripted consumer needs it too.
            "exists": path.is_file(),
            "over_budget": charter.over_budget(),
            "char_count": charter.char_count(),
            "lines": charter.lines().iter().map(|l| serde_json::json!({
                "id": l.id,
                "text": l.text,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if charter.is_empty() {
        // Distinguished from "no file at all" — this is the message doctor's
        // "charter file exists but has no lines" finding sends people to, and
        // printing the same "no charter at" line either way would reproduce
        // exactly the indistinguishability that finding exists to break.
        if path.is_file() {
            println!(
                "{} exists but has no `[[line]]` entries — nothing from it rides in any prompt",
                path.display()
            );
        } else {
            println!("no charter at {}", path.display());
        }
        println!(
            "a charter is `[[line]]` tables, ranked highest first by their order in the \
             file — `id = \"...\"` and `text = \"...\"` each"
        );
        return Ok(());
    }

    println!("{}\n", path.display());
    for (i, line) in charter.lines().iter().enumerate() {
        println!("{}. {} — {}", i + 1, line.id, line.text);
    }
    if charter.over_budget() {
        println!(
            "\n{} characters, over the {}-character budget — it still rides in the prompt \
             in full, but costs more of the cached prefix than argued",
            charter.char_count(),
            mecha_core::charter::CHARTER_CHAR_BUDGET
        );
    }
    Ok(())
}
