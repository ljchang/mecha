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
            eprintln!("mecha: charter at {} did not load — {e:#}", path.display());
            std::process::exit(1);
        }
    };

    if args.json {
        let out = serde_json::json!({
            "path": path,
            "lines": charter.lines().iter().map(|l| serde_json::json!({
                "id": l.id,
                "text": l.text,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if charter.is_empty() {
        println!("no charter at {}", path.display());
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
    Ok(())
}
