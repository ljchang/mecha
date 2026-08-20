//! `mecha skills` — what the agent knows how to do, as distinct from what it
//! can call.
//!
//! `mecha tools` answers half of "what can this agent actually do"; this is
//! the other half, in the same shape and for the same reason. It builds no
//! provider and connects to nothing, so a store can be inspected and a
//! malformed `SKILL.md` diagnosed before any credential exists.
//!
//! It reports what a *run* would carry, config applied — a skill sitting in
//! the store that `[skills] disabled` withholds is listed as withheld rather
//! than omitted, because "why is my skill not firing" has to be answerable
//! here rather than by reading two files and intersecting them by hand.

use anyhow::Result;
use mecha_core::config::Config;
use mecha_core::skill::SkillStore;

use crate::GlobalOpts;

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Print each skill's full body, exactly as the model would receive it.
    #[arg(long)]
    pub show: bool,

    /// Emit JSON instead of a table.
    #[arg(long)]
    pub json: bool,
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let cfg = if global.global_config_only {
        Config::load_global()?
    } else {
        Config::load(&std::env::current_dir()?)?
    };
    let dir = match cfg.skills.dir.clone() {
        Some(dir) => dir,
        None => SkillStore::default_dir()?,
    };
    let (store, errors) = SkillStore::load(&dir);
    // `--skill` narrows too, and leaving it out made this command disagree
    // with the run it claims to describe: `mecha skills --skill audit` would
    // mark every config-selected skill as carried while the run carried one.
    let selected: Vec<_> = store
        .select(&cfg.skills.enabled, &cfg.skills.disabled)
        .into_iter()
        .filter(|s| global.skills.is_empty() || global.skills.iter().any(|n| n == &s.name))
        .collect();
    let carried = |name: &str| selected.iter().any(|s| s.name == name);

    if args.json {
        let out = serde_json::json!({
            "dir": dir,
            "skills": store.all().iter().map(|s| serde_json::json!({
                "name": s.name,
                "description": s.description,
                "triggers": s.triggers,
                "tools": s.tools,
                "dir": s.dir,
                "carried": carried(&s.name),
                // The body is the expensive field and the reason `--show`
                // exists, so it rides only when asked for.
                "body": args.show.then(|| s.body.clone()),
            })).collect::<Vec<_>>(),
            // Failures are in the machine view too: a consumer counting
            // skills would otherwise report a store that half-loaded as a
            // smaller store.
            "errors": errors.iter().map(|e| serde_json::json!({
                "dir": e.dir,
                "error": e.why,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if store.is_empty() && errors.is_empty() {
        println!("no skills in {}", dir.display());
        println!(
            "a skill is a directory holding a SKILL.md — YAML frontmatter with \
             `name` and `description`, then the procedure as markdown"
        );
        return Ok(());
    }

    println!("{}\n", dir.display());
    for skill in store.all() {
        // Withheld rather than absent, so the answer to "why is this not
        // firing" is on the screen instead of in two config files.
        let mark = if carried(&skill.name) { " " } else { "-" };
        println!("{mark} {}", skill.name);
        println!("    {}", skill.description);
        if !skill.triggers.is_empty() {
            println!("    keywords: {}", skill.triggers.join(", "));
        }
        if let Some(tools) = &skill.tools {
            println!("    narrows the tool surface to: {}", tools.join(", "));
        }
        if args.show {
            for line in skill.body.lines() {
                println!("    │ {line}");
            }
        }
        println!();
    }

    if selected.len() != store.all().len() {
        println!(
            "{} of {} carried; `-` is withheld by [skills] or --skill",
            selected.len(),
            store.all().len()
        );
    }
    for e in &errors {
        eprintln!(
            "skill `{}` did not load — {}",
            e.dir.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
            e.why
        );
    }
    // Exit non-zero when something did not load, so this works as a check in
    // a script the way `doctor` does. A store that is merely empty is healthy.
    if !errors.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}
