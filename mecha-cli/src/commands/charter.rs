//! `mecha charter` — the standing priorities in `~/.mecha/charter.toml`, as
//! the owner's run would see them.
//!
//! Read-only, on `crate::commands::skills`'s own rule: a charter is edited
//! with a text editor, never by this command and never by a model
//! (`docs/GOAL-SYSTEM-DESIGN.md` §11). There is no `--edit`, no `--add`, and
//! there never will be — the absence is the safety argument, not a missing
//! feature.

use anyhow::Result;
use mecha_core::charter::Charter;

use crate::GlobalOpts;

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Emit JSON instead of a table.
    #[arg(long)]
    pub json: bool,
}

pub async fn execute(_global: &GlobalOpts, args: Args) -> Result<()> {
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
            if args.json {
                let out = serde_json::json!({
                    "path": path,
                    "error": e.to_string(),
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                eprintln!("mecha: charter at {} did not load — {e:#}", path.display());
            }
            std::process::exit(1);
        }
    };

    if args.json {
        let out = serde_json::json!({
            "path": path,
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
                "{} exists but has no `[[line]]` entries — check the table name",
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
