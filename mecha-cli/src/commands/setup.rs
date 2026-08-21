//! `mecha setup` — what this install still needs, and the way to each.
//!
//! The thin half. `mecha_core::onboarding::plan` decides; this gathers the
//! facts it decides over and, at a terminal, offers each remedy one at a
//! time. That split is `doctor`'s, and so is the offer loop — a confirmed
//! command is spawned **inheriting the real terminal**, because an OAuth
//! flow needs a keyboard and `.output()` hands it a closed stdin.
//!
//! Where it differs from `doctor`, and why both exist: doctor answers "what
//! is silently broken about a working install" in one pass with no network
//! and no model. Every question here needs to *ask a server something*, and
//! the answers change a config file. Folding them together would have put a
//! network call inside the one command whose whole contract is that it has
//! none.

use anyhow::{Context, Result};
use mecha_core::doctor::Remedy;
use mecha_core::onboarding::{self, Facts, Status, Step};
use std::io::{IsTerminal, Write};

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Emit the plan as JSON and exit. Never prompts, even at a terminal.
    #[arg(long)]
    pub json: bool,

    /// Rewrite the local provider's `model`, `context_window` and `vision`
    /// from what its server reports, instead of only reporting the
    /// disagreement.
    #[arg(long)]
    pub write: bool,
}

pub async fn execute(global: &crate::GlobalOpts, args: Args) -> Result<()> {
    let cfg = mecha_core::config::Config::load_global()
        .context("reading the global config — run `mecha config init` first")?;
    let (name, pcfg) = cfg.provider(global.provider.as_deref())?;

    // The one network call, and only for a local server: it is the only kind
    // known to answer `/props`, and a 404 from somebody else's endpoint would
    // be noise on a command people run when something is already confusing.
    let props = if pcfg.kind == "local" {
        match pcfg.base_url.as_deref() {
            Some(url) => mecha_core::provider::preflight::fetch(url).await,
            None => None,
        }
    } else {
        None
    };

    let home = mecha_core::work::mecha_home()?;
    let facts = Facts {
        has_mail_binary: onboarding::on_path("mecha-mail"),
        has_docs_binary: onboarding::on_path("mecha-docs"),
        has_graph_binary: onboarding::on_path("mecha-graph-mcp"),
        mail_accounts: onboarding::count_accounts(&home.join("mail")),
        docs_accounts: onboarding::count_accounts(&home.join("docs")),
        slack_linked: slack_linked(&home),
        provider_credential: pcfg.resolve_api_key().is_some(),
        props,
        scheduler_installed: scheduler_installed(),
        trigger_count: trigger_count(&home),
    };

    let steps = onboarding::plan(&cfg, &name, &facts);

    if args.json {
        println!("{}", serde_json::to_string_pretty(&steps)?);
        return Ok(());
    }
    if args.write {
        return write_verified(&name, &facts);
    }

    render(&steps);

    // Nothing is offered when nobody is there to answer, which is the
    // `doctor` rule: a setup flow that acts with no one watching is the
    // shape this project keeps refusing.
    let outstanding: Vec<&Step> = steps.iter().filter(|s| s.status != Status::Done).collect();
    if outstanding.is_empty() {
        println!("\nNothing outstanding.");
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        println!("\n{} step(s) outstanding.", outstanding.len());
        // Exit 1 like doctor: a script can act on it, and a person sees the
        // same list either way.
        std::process::exit(1);
    }
    offer(&outstanding)
}

fn render(steps: &[Step]) {
    for s in steps {
        let (mark, label) = match s.status {
            Status::Done => ("✓", "ok"),
            Status::Missing => ("·", "not set up"),
            Status::Wrong => ("!", "disagrees"),
            Status::Unknown => ("?", "unknown"),
        };
        println!("\n{mark} {}  [{label}]", s.title);
        for line in s.detail.lines() {
            println!("    {line}");
        }
        if let Some(r) = &s.remedy {
            println!("    → {}", shell_words(&r.argv));
        }
    }
}

/// Offer each remedy once, EOF is no — the outbox `send` convention, and
/// `doctor`'s: silence is not consent.
fn offer(steps: &[&Step]) -> Result<()> {
    let stdin = std::io::stdin();
    let mut offered: Vec<&[String]> = Vec::new();
    for s in steps {
        let Some(remedy) = &s.remedy else { continue };
        if offered.contains(&remedy.argv.as_slice()) {
            continue;
        }
        offered.push(&remedy.argv);
        println!("\n{}", remedy.description);
        print!("run `{}`? [y/N] ", shell_words(&remedy.argv));
        std::io::stdout().flush()?;
        let mut line = String::new();
        let said_yes = match std::io::BufRead::read_line(&mut stdin.lock(), &mut line) {
            Ok(0) => {
                println!();
                false
            }
            Ok(_) => line.trim().eq_ignore_ascii_case("y"),
            Err(_) => false,
        };
        if !said_yes {
            println!("skipped");
            continue;
        }
        run(remedy)?;
    }
    Ok(())
}

/// Inheriting the real terminal, never captured: an OAuth flow needs a
/// keyboard and a screen, and `.output()` gives it a pipe and a closed stdin.
fn run(remedy: &Remedy) -> Result<()> {
    let (program, rest) = remedy
        .argv
        .split_first()
        .context("a remedy with an empty argv")?;
    let status = std::process::Command::new(program).args(rest).status();
    match status {
        Ok(s) if s.success() => println!("done"),
        Ok(s) => println!("that exited {} — nothing else was changed", s),
        Err(e) => println!("could not run it: {e}"),
    }
    Ok(())
}

/// Write back what the server said about itself.
///
/// Deliberately prints the diff and asks, even though the values came from
/// the server rather than from a guess: this edits a file somebody may have
/// had reasons for, and the whole argument for reading values off the wire is
/// weakened if the reading is also unattended.
fn write_verified(provider: &str, facts: &Facts) -> Result<()> {
    let Some(props) = &facts.props else {
        anyhow::bail!("nothing answered, so there is nothing to write down. Start the server.");
    };
    let settings = onboarding::verified_settings(props);
    println!("Read back from the server, for [providers.{provider}]:\n");
    for (k, v) in &settings {
        println!("    {k} = {v}");
    }
    println!(
        "\nThese are what the server reports, not what it was asked for — which is the point: \
         `context_window` is the *per-slot* figure, so `-c` divided by `-np`."
    );
    if std::io::stdin().is_terminal() {
        print!("\nwrite them into the config? [y/N] ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut line)?;
        if !line.trim().eq_ignore_ascii_case("y") {
            println!("not written");
            return Ok(());
        }
    } else {
        println!("\n(not a terminal, so nothing was written — copy the lines above)");
        return Ok(());
    }
    apply(provider, &settings)
}

/// Edit the provider's table in place, preserving every comment.
///
/// A parse-and-reserialise round trip would be shorter and would throw away
/// the file's comments — which in this project are most of it, and are how
/// the next reader learns why a number is what it is. So this rewrites the
/// lines it owns and touches nothing else.
fn apply(provider: &str, settings: &[(&'static str, String)]) -> Result<()> {
    let path = mecha_core::config::Config::global_path()
        .context("no global config path — is $HOME set?")?;
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {path:?}"))?;
    let header = format!("[providers.{provider}]");
    let Some(start) = text.lines().position(|l| l.trim() == header) else {
        anyhow::bail!("no {header} table in {}", path.display());
    };
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, l)| l.trim_start().starts_with('['))
        .map(|(i, _)| i)
        .unwrap_or(lines.len());

    for (key, value) in settings {
        let assignment = format!("{key} = {value}");
        match lines[start + 1..end]
            .iter()
            .position(|l| l.split('=').next().map(str::trim) == Some(*key))
        {
            Some(rel) => lines[start + 1 + rel] = assignment,
            None => lines.insert(end, assignment),
        }
    }
    let backup = path.with_extension("toml.bak");
    std::fs::copy(&path, &backup).ok();
    std::fs::write(&path, lines.join("\n") + "\n")?;
    println!(
        "written to {} (previous copy at {})",
        path.display(),
        backup.display()
    );
    Ok(())
}

fn slack_linked(home: &std::path::Path) -> Option<bool> {
    let dir = home.join("slack");
    match std::fs::read_dir(&dir) {
        Ok(entries) => Some(entries.flatten().any(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x == "json" || x == "toml")
        })),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(false),
        Err(_) => None,
    }
}

fn trigger_count(home: &std::path::Path) -> usize {
    std::fs::read_dir(home.join("triggers"))
        .map(|e| {
            e.flatten()
                .filter(|f| f.path().extension().is_some_and(|x| x == "toml"))
                .count()
        })
        .unwrap_or(0)
}

/// Is anything actually going to fire a trigger?
///
/// Deliberately loose: being due is a function of the ledger and the clock,
/// so a systemd unit, a crontab line and a bare `mecha trigger daemon` are
/// all equally valid answers, and a check that only recognised systemd would
/// nag someone whose crontab works fine.
fn scheduler_installed() -> bool {
    let unit = std::env::var_os("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".config/systemd/user/mecha-triggers.service"))
        .is_some_and(|p| p.is_file());
    let running = std::process::Command::new("pgrep")
        .args(["-f", "mecha trigger daemon"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let crontab = std::process::Command::new("crontab")
        .arg("-l")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("trigger tick"))
        .unwrap_or(false);
    unit || running || crontab
}

fn shell_words(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            if a.contains(' ') {
                format!("{a:?}")
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
