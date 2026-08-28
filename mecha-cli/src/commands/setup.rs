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

    /// Ask about a step you said "never" to again. `all` clears every one.
    ///
    /// The undo half of the `never` answer, and it exists so that answer is
    /// a preference rather than a door that locks behind you.
    #[arg(long, value_name = "STEP_ID")]
    pub undecline: Option<String>,
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

    // Only when **no local provider is configured at all** and no credential
    // is available — a working install makes no extra call, and this one is
    // loopback, so nothing leaves the machine. It is what lets the step that
    // blocks every other one carry a remedy instead of a diagnosis.
    //
    // **Across every provider, not just the selected one.** Keying this on
    // `pcfg.kind` was narrower than the claim the step then makes: a config
    // with `default_provider = "anthropic"` (no key exported) *and* a
    // `[providers.local]` on :8080 selects anthropic, passes a `kind !=
    // "local"` test, and gets told "something is serving here and **nothing
    // in the config names it**" about a server the config names on the very
    // next line — with `mecha setup --write` offered as the remedy, which
    // then bails in `append_table` telling you to run the command you just
    // ran. Exactly the "the blocking step's remedy is not a way out" shape
    // this whole change set out to remove.
    //
    // The same test also covers the down-server case: a configured local
    // server that is merely *down* has no props and no api key either, and
    // probing there would find whatever else is on 8080. The right answer
    // for a down server is the `local-server` step's own "start it", which
    // `plan` already gives.
    let a_local_provider_is_configured = cfg.providers.values().any(|p| p.kind == "local");
    let local_server = if !a_local_provider_is_configured && pcfg.resolve_api_key().is_none() {
        probe_for_a_local_server().await
    } else {
        None
    };

    let home = mecha_core::work::mecha_home()?;
    let declined = onboarding::read_declined(&home);
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
        config_file: mecha_core::config::Config::global_path().is_some_and(|p| p.is_file()),
        local_server,
        charter: onboarding::charter_state(&home.join("charter.toml")),
        // `None` is the store being unreadable, not empty. Resolved towards
        // offering everything (see `Facts::declined`), and said out loud
        // below rather than swallowed — a checklist that quietly stopped
        // honouring your answers would be the worse half of this feature.
        declined: declined.clone().unwrap_or_default(),
    };

    let steps = onboarding::plan(&cfg, &name, &facts);

    // Nothing is offered when nobody is there to answer, which is the
    // `doctor` rule: a setup flow that acts with no one watching is the
    // shape this project keeps refusing.
    //
    // **`Declined` is not outstanding**, which is the whole point of it
    // existing: a scripted `mecha setup` on a machine whose owner does not
    // want Slack must exit 0, or the check is permanently red over a choice
    // somebody already made.
    let outstanding: Vec<&Step> = steps
        .iter()
        .filter(|s| !matches!(s.status, Status::Done | Status::Declined))
        .collect();

    if args.json {
        println!("{}", serde_json::to_string_pretty(&steps)?);
        // **Doctor's contract, which this used to document and not keep.**
        // `doctor --json` prints the findings and then falls through to the
        // shared exit check; `setup --json` returned early and skipped it, so
        // the documented "exit 1 when anything is outstanding, like doctor"
        // was false — and every `! mecha setup --json` written against it was
        // silently vacuous rather than loudly wrong, because `set -e` does
        // not apply to an inverted command. A machine-readable plan whose
        // exit code says nothing is a plan every caller has to parse to learn
        // what a status byte could have told it.
        if !outstanding.is_empty() {
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.write {
        return write_verified(&name, &facts);
    }
    if let Some(id) = &args.undecline {
        let one = (id != "all").then_some(id.as_str());
        onboarding::undecline(&home, one)?;
        match one {
            Some(id) => println!("`{id}` will be offered again"),
            None => println!("every declined step will be offered again"),
        }
        return Ok(());
    }

    render(&steps);
    if declined.is_none() {
        println!(
            "\n(the declined-steps file at {} could not be read, so everything is being \n\
             offered again — that is a read failure, not an empty list)",
            onboarding::declined_path(&home).display()
        );
    }

    if outstanding.is_empty() {
        println!("\nNothing outstanding.");
        finished_note(&steps);
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        println!("\n{} step(s) outstanding.", outstanding.len());
        // Exit 1 like doctor: a script can act on it, and a person sees the
        // same list either way.
        std::process::exit(1);
    }
    offer(&outstanding, &home)?;
    finished_note(&steps);
    Ok(())
}

/// The last thing a new install is told.
///
/// Setup used to end at the checklist, which answers *what is missing* and
/// never *what now* — so somebody who had just finished had no next move and
/// no idea that `doctor` is a different question. Three lines, and the
/// undecline is named here so the way back out of a "never" is never
/// something to go looking for.
fn finished_note(steps: &[Step]) {
    println!("\nNext:");
    println!("    mecha tools          what this agent can call — no provider needed");
    println!("    mecha run \"…\"        one task, one answer, in the current directory");
    println!("    mecha doctor         a different question: what is silently wrong");
    if steps.iter().any(|s| s.status == Status::Declined) {
        println!("\n    mecha setup --undecline <id>   ask about a skipped step again");
    }
    // The one trap a new install walks into unaided, and the only place a
    // person is standing when they might. `work.rs` refuses a workspace
    // containing the mecha home, which is right and arrives as an error —
    // saying it here turns it into advice instead.
    println!(
        "\nRun it from a project directory rather than your home directory: the workspace is\n\
         the jail, and one rooted over ~/.mecha would cover its own tokens and transcripts."
    );
}

fn render(steps: &[Step]) {
    for s in steps {
        let (mark, label) = match s.status {
            Status::Done => ("✓", "ok"),
            Status::Missing => ("·", "not set up"),
            Status::Wrong => ("!", "disagrees"),
            Status::Unknown => ("?", "unknown"),
            // Marked as settled rather than outstanding, because it is: a
            // person answered this. `mecha setup --undecline <id>` is in the
            // closing line, so the way back is never something to go looking
            // for.
            Status::Declined => ("–", "you said no thanks"),
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

/// What the person answered at one offer.
///
/// `Skip` and `Never` are deliberately different answers to the same
/// question, and folding them would lose the distinction this whole feature
/// is: *not now* is about today, *never* is about the install. EOF and a
/// blank line are `Skip` — silence is not consent, and it is emphatically
/// not a permanent decision either.
enum Answer {
    Yes,
    Skip,
    Never,
}

/// Offer each remedy once, EOF is no — the outbox `send` convention, and
/// `doctor`'s: silence is not consent.
///
/// `never` records the step id in `~/.mecha/setup-declined.json` so the
/// question stops being asked. It is only ever offered for a step that has
/// one: a broken charter or a server that disagrees with its config is a
/// fault rather than an optional extra, and there is nothing coherent to
/// decline about being told.
fn offer(steps: &[&Step], home: &std::path::Path) -> Result<()> {
    let stdin = std::io::stdin();
    let mut offered: Vec<&[String]> = Vec::new();
    for s in steps {
        let Some(remedy) = &s.remedy else { continue };
        if offered.contains(&remedy.argv.as_slice()) {
            continue;
        }
        offered.push(&remedy.argv);
        // Only an *absent* optional thing can be declined. `Wrong` is
        // something broken, and "stop telling me this is broken" is not a
        // preference a setup tool should be able to record — it is the
        // silently-degrading-guard shape. And `optional` is the step's own
        // property rather than an inference from `Missing`: a provider with
        // no credential is missing too, and declining *that* would report
        // `Nothing outstanding.` on an install that cannot answer a prompt.
        let declinable = s.optional && s.status == Status::Missing;
        println!("\n{}", remedy.description);
        print!(
            "run `{}`? [{}] ",
            shell_words(&remedy.argv),
            if declinable { "y/N/never" } else { "y/N" }
        );
        std::io::stdout().flush()?;
        let mut line = String::new();
        let answer = match std::io::BufRead::read_line(&mut stdin.lock(), &mut line) {
            Ok(0) => {
                println!();
                Answer::Skip
            }
            Ok(_) => match line.trim().to_ascii_lowercase().as_str() {
                "y" | "yes" => Answer::Yes,
                "never" | "n!" if declinable => Answer::Never,
                _ => Answer::Skip,
            },
            Err(_) => Answer::Skip,
        };
        match answer {
            Answer::Yes => run(remedy)?,
            Answer::Skip => println!("skipped — asked again next time"),
            Answer::Never => match onboarding::decline(home, &s.id) {
                // Said with the undo in the same breath: a decision nobody
                // can find their way back out of is one people are right to
                // hesitate over.
                Ok(()) => println!(
                    "noted — `{}` will not be offered again (`mecha setup --undecline {}` undoes it)",
                    s.id, s.id
                ),
                // A store that could not be written must not read as a
                // recorded answer, or the question silently comes back and
                // the person believes they already settled it.
                Err(e) => println!("could not record that: {e} — it will be offered again"),
            },
        }
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

/// Ask the documented local address whether anything is serving there.
///
/// Fail-soft by construction — `preflight::fetch` has a three-second timeout
/// and answers `None` for anything that is not a llama-server-shaped `/props`,
/// so a machine with something unrelated on 8080 gets the no-server branch
/// rather than a wrong provider written into its config.
async fn probe_for_a_local_server() -> Option<onboarding::LocalServer> {
    for base_url in onboarding::local_probe_candidates() {
        if let Some(props) = mecha_core::provider::preflight::fetch(base_url).await {
            return Some(onboarding::LocalServer {
                base_url: (*base_url).to_string(),
                props,
            });
        }
    }
    None
}

/// Write back what the server said about itself.
///
/// Deliberately prints the diff and asks, even though the values came from
/// the server rather than from a guess: this edits a file somebody may have
/// had reasons for, and the whole argument for reading values off the wire is
/// weakened if the reading is also unattended.
fn write_verified(provider: &str, facts: &Facts) -> Result<()> {
    // The probed case: a server is running that no provider names. Writing it
    // down is the remedy the blocking step now offers, and it is the same act
    // as the one below — read the values off the wire, show them, ask — with
    // one addition, which is that the table does not exist yet.
    if let Some(found) = &facts.local_server {
        return write_local_provider(found);
    }
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

/// Write down a local server that was found rather than configured.
///
/// **Every value comes off `/props`, and the secret-shaped one does not
/// exist.** A local provider needs no credential, which is why this is a
/// remedy at all: the no-server branch of the same step cannot be, because
/// its fix is an API key and `mecha setup` must never put a key in a file —
/// the config holds the *name* of an environment variable so that it can be
/// read, copied and committed without leaking one.
///
/// It also moves `default_provider`, which is a bigger change than the three
/// keys `--write` otherwise touches: it changes what answers. So it is
/// printed in full and confirmed, and the previous file is kept.
fn write_local_provider(found: &onboarding::LocalServer) -> Result<()> {
    let settings = onboarding::verified_settings(&found.props);
    println!(
        "Found a server at {} and nothing in the config names it.\n",
        found.base_url
    );
    println!("    [providers.local]");
    println!("    kind = \"local\"");
    println!("    base_url = {:?}", found.base_url);
    for (k, v) in &settings {
        println!("    {k} = {v}");
    }
    println!("\n    default_provider = \"local\"");
    println!(
        "\nRead back from the server rather than asked of you — which is the whole point: \
         `context_window` is the *per-slot* figure, `-c` divided by `-np`, and it is the \
         number people get wrong by hand."
    );

    if !std::io::stdin().is_terminal() {
        println!("\n(not a terminal, so nothing was written — copy the lines above)");
        return Ok(());
    }
    print!("\nwrite this, and make it the default provider? [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut line)?;
    if !line.trim().eq_ignore_ascii_case("y") {
        println!("not written");
        return Ok(());
    }

    let path = mecha_core::config::Config::global_path()
        .context("no global config path — is $HOME set?")?;
    // A new install may not have one yet, and `--write` is reachable without
    // having run `mecha config init` first. Seeded from the same starter that
    // command writes, so there is one commented file in the world rather than
    // two that drift.
    if !path.is_file() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, super::config::STARTER)
            .with_context(|| format!("writing {}", path.display()))?;
        println!("created {}", path.display());
    }

    let mut table = vec![
        String::new(),
        "# Written by `mecha setup --write` from this server's own /props.".to_string(),
        "[providers.local]".to_string(),
        "kind = \"local\"".to_string(),
        format!("base_url = {:?}", found.base_url),
    ];
    table.extend(settings.iter().map(|(k, v)| format!("{k} = {v}")));
    append_table(&path, "local", &table)?;
    set_default_provider(&path, "local")?;
    println!(
        "written to {} — `mecha setup` again to check it agrees with the server",
        path.display()
    );
    Ok(())
}

/// Append a provider table, refusing to duplicate one that is already there.
///
/// A second `[providers.local]` is not an error TOML reports usefully, and a
/// config with two of them is the kind of thing somebody debugs for an hour.
fn append_table(path: &std::path::Path, provider: &str, table: &[String]) -> Result<()> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {path:?}"))?;
    let header = format!("[providers.{provider}]");
    anyhow::ensure!(
        !text.lines().any(|l| l.trim() == header),
        "{} already has a {header} table — edit it, or `mecha setup --write` against it",
        path.display()
    );
    backup(path)?;
    let mut out = text;
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&table.join("\n"));
    out.push('\n');
    std::fs::write(path, out)?;
    Ok(())
}

/// Point `default_provider` at `provider`, in place.
///
/// Rewrites the assignment where there is one and inserts it where there is
/// not — never a parse-and-reserialise, for the reason [`apply`] gives: the
/// comments in this file are most of it.
fn set_default_provider(path: &std::path::Path, provider: &str) -> Result<()> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {path:?}"))?;
    let assignment = format!("default_provider = {provider:?}");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    match lines
        .iter()
        // Only before the first table header: a `default_provider` inside a
        // `[providers.x]` table would be that table's own key, and rewriting
        // it would silently edit something else entirely.
        .take_while(|l| !l.trim_start().starts_with('['))
        .position(|l| l.split('=').next().map(str::trim) == Some("default_provider"))
    {
        Some(i) => lines[i] = assignment,
        // **After the leading comment block, not at line 0.** A file that
        // opens with its own header comment — which the starter this writes
        // does, and which is the shape people's hand-written configs take —
        // would otherwise get the assignment *above* the comment describing
        // the file, so the comment reads as if it were about the assignment.
        // Valid TOML, and wrong about what the file says.
        None => lines.insert(leading_comment_end(&lines), assignment),
    }
    std::fs::write(path, lines.join("\n") + "\n")?;
    Ok(())
}

/// The first line that is neither a comment nor blank — where a new top-level
/// key belongs in a file that opens with prose about itself.
fn leading_comment_end(lines: &[String]) -> usize {
    let end = lines
        .iter()
        .position(|l| {
            let t = l.trim_start();
            !t.is_empty() && !t.starts_with('#')
        })
        .unwrap_or(lines.len());
    // Back up over blank lines so the key joins the content rather than
    // orphaning the blank that separated the header from it.
    lines[..end]
        .iter()
        .rposition(|l| l.trim_start().starts_with('#'))
        .map(|i| i + 1)
        .unwrap_or(end)
}

/// Keep the previous file beside the new one. Best-effort by design: a backup
/// that cannot be made must not stop the write it was protecting, or a
/// read-only backup path would make the tool useless.
fn backup(path: &std::path::Path) -> Result<()> {
    let backup = path.with_extension("toml.bak");
    if std::fs::copy(path, &backup).is_ok() {
        println!("previous copy at {}", backup.display());
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    /// A new top-level key goes *under* the file's own header comment, not
    /// above it. Found by running `--write` against a hand-written config:
    /// the result was valid TOML that lied about what the comment described.
    #[test]
    fn a_new_top_level_key_lands_below_the_files_own_header() {
        let l = lines("# My config.\n# Second line.\n\n[agent]\nmax_turns = 40\n");
        assert_eq!(leading_comment_end(&l), 2, "just past the comment block");

        // No header at all: the key goes first, which is where it belongs.
        assert_eq!(leading_comment_end(&lines("[agent]\nx = 1\n")), 0);

        // A file that is nothing but comments — the starter, before anybody
        // uncomments anything. `lines.len()` would be past the end for an
        // insert, so this has to be the comment count exactly.
        let all_comments = lines("# a\n# b\n");
        assert_eq!(leading_comment_end(&all_comments), all_comments.len());

        // Empty file: no panic, and index 0 is a legal insert point.
        assert_eq!(leading_comment_end(&[]), 0);
    }
}
