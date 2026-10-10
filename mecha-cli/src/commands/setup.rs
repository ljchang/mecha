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
use mecha_core::feature::{self, Feature};
use mecha_core::onboarding::{self, Facts, Status, Step};
use std::io::{IsTerminal, Write};

/// **The three verbs are mutually exclusive at the parser, not by precedence
/// in the body.** Each pair is meaningless, and each used to resolve by
/// whichever branch came first — so `mecha setup --json --write` printed a
/// plan, exited 1 and wrote nothing, which is the silent-no-op shape a flag
/// should never have. Saying it in the arg definition makes clap explain it;
/// leaving it to the body's ordering leaves it to be discovered.
#[derive(clap::Args, Debug)]
pub struct Args {
    /// Emit the plan as JSON and exit. Never prompts, even at a terminal.
    #[arg(long, conflicts_with_all = ["write", "undecline"])]
    pub json: bool,

    /// Rewrite the local provider's `model`, `context_window` and `vision`
    /// from what its server reports, instead of only reporting the
    /// disagreement.
    #[arg(long, conflicts_with = "undecline")]
    pub write: bool,

    /// Ask about a step you said "never" to again. `all` clears every one.
    ///
    /// The undo half of the `never` answer, and it exists so that answer is
    /// a preference rather than a door that locks behind you.
    #[arg(long, value_name = "STEP_ID")]
    pub undecline: Option<String>,

    /// Set up one optional feature: just its step (`mecha features` lists
    /// the ids; a part such as `ocr` sets up the feature it belongs to).
    #[arg(value_name = "FEATURE", conflicts_with_all = ["write", "undecline", "minimal"])]
    pub feature: Option<String>,

    /// Decline every optional step still outstanding, in one pass — the
    /// light install. Writes declines, never config; `mecha features enable`
    /// turns any one on later, and `--undecline` asks again.
    #[arg(long, conflicts_with_all = ["json", "write", "undecline"])]
    pub minimal: bool,

    /// With `engine`: measure the shipped llama.cpp against the engine the
    /// servers run, and move them onto it if it is no slower. At a terminal.
    #[arg(long, conflicts_with_all = ["json", "write", "undecline", "minimal", "rollback"])]
    pub adopt: bool,

    /// With `engine`: fetch a newer llama.cpp release (the newest, or
    /// `--to <tag>`), confirm its tag, measure it against the one the
    /// servers run, and move them onto it if it is no slower. At a terminal.
    #[arg(long, conflicts_with_all = ["json", "write", "undecline", "minimal", "adopt", "rollback"])]
    pub upgrade: bool,

    /// With `engine --upgrade`: the release tag to fetch (`b` and digits),
    /// instead of the newest.
    #[arg(long, value_name = "TAG", requires = "upgrade")]
    pub to: Option<String>,

    /// With `engine --adopt` or `--upgrade`: move the servers even if the
    /// new engine measures slower, or fails a check the old one passes.
    /// Refused with `--rollback` here, at the parser; "only with one of two
    /// verbs" is the half clap cannot say, checked in `setup_engine::run`.
    #[arg(long, conflicts_with = "rollback")]
    pub force: bool,

    /// With `engine`: move an adopted machine back onto the engine it ran
    /// before. Waits for running work, as any switch does. At a terminal.
    #[arg(long, conflicts_with_all = ["json", "write", "undecline", "minimal"])]
    pub rollback: bool,

    /// With `engine --rollback`: ask running work to stop rather than wait.
    #[arg(long, requires = "rollback")]
    pub now: bool,
}

pub async fn execute(global: &crate::GlobalOpts, args: Args) -> Result<()> {
    let cfg = mecha_core::config::Config::load_global()
        .context("reading the global config — run `mecha config init` first")?;
    // `engine` is a reserved noun here, never a feature id (§10.3): it owns
    // the engine's flags and needs no provider answering first.
    // `chat` is reserved the same way (ruling F11): a fresh machine's door to
    // a local chat model — engine, router and model — never a feature id.
    if args.feature.as_deref() == Some("chat") {
        anyhow::ensure!(
            !args.adopt && !args.upgrade && !args.rollback && !args.force && !args.json,
            "`mecha setup chat` takes no flags — it asks"
        );
        return super::setup_chat::run(&cfg).await;
    }
    if args.feature.as_deref() == Some("engine") {
        return super::setup_engine::run(
            &cfg,
            super::setup_engine::Flags {
                json: args.json,
                adopt: args.adopt,
                upgrade: args.upgrade,
                to: args.to.clone(),
                rollback: args.rollback,
                now: args.now,
                force: args.force,
            },
        )
        .await;
    }
    // A body check, against this struct's own rule, because clap cannot say
    // "only with the feature value `engine`" — and it refuses loudly.
    anyhow::ensure!(
        !args.adopt && !args.rollback && !args.upgrade && !args.force,
        "--adopt, --upgrade, --rollback and --force go with `mecha setup engine`"
    );
    let (name, pcfg) = cfg.provider(global.provider.as_deref())?;
    let home = mecha_core::work::mecha_home()?;

    // **Handled before anything else, and before any network call.**
    // It used to sit below the `--json` and `--write` returns, so
    // `mecha setup --json --undecline all` printed a plan, exited 1, and
    // undeclined nothing — silently, which is the worst way for a flag to
    // not work. And a verb whose entire job is rewriting one local JSON file
    // has no business waiting on a three-second loopback timeout first.
    if let Some(id) = &args.undecline {
        let one = (id != "all").then_some(id.as_str());
        let wrote = onboarding::undecline(&home, one)?;
        // **Reported off what changed, not off what was asked for.** A typo'd
        // id used to print "`slak` will be offered again" and exit 0, so the
        // person believed the way back had been taken and then met
        // `you said no thanks` on the step they thought they had restored.
        match (one, wrote.changed) {
            (Some(id), true) => println!("`{id}` will be offered again"),
            (Some(id), false) => println!(
                concat!(
                    "`{id}` was not declined — nothing to restore ",
                    "(`mecha setup --json` lists every step id)"
                ),
                id = id
            ),
            (None, true) => println!("every declined step will be offered again"),
            (None, false) => println!("nothing was declined — nothing to restore"),
        }
        report_salvage(wrote.salvaged);
        return Ok(());
    }

    // The one network call, and only for a local server: it is the only kind
    // known to answer `/props`, and a 404 from somebody else's endpoint would
    // be noise on a command people run when something is already confusing.
    let props = if pcfg.kind == "local" {
        match pcfg.base_url.as_deref() {
            Some(url) => mecha_core::provider::preflight::fetch(url, pcfg.model.as_deref()).await,
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
    let local_probe = if !a_local_provider_is_configured && pcfg.resolve_api_key().is_none() {
        probe_for_a_local_server().await
    } else {
        // **Recorded as "not attempted", never as "nothing answered".** The
        // step reports what the probe found, and those are different
        // findings: saying nothing answered at an address nobody asked is a
        // fact with no observation behind it, which is the rule this module's
        // header is about, inverted.
        onboarding::LocalProbe::NotAttempted
    };

    // Which feature `mecha setup <feature>` names, settled before any work:
    // a typo is refused by name rather than read as "nothing to do".
    let only: Option<Feature> = match &args.feature {
        None => None,
        Some(id) => Some(
            Feature::parse(id)
                .with_context(|| format!("`{id}` is not a feature — `mecha features` lists them"))?
                .switch_owner(),
        ),
    };

    let declined = onboarding::read_declined(&home);
    // Naming a feature is a stronger statement than a stored "never": its
    // step is offered again for this run (review of #460). Nothing is written
    // unless the answer is: a skip keeps the old decline, a fresh "never"
    // records it once more. **Never under `--json`**: nothing is offered
    // there, so reopening could only turn a recorded answer into a non-zero
    // exit — the "red over a choice already made" `Declined` exists to retire
    // (review of #461). A plain run reopens whether or not a terminal is
    // attached: the rule is the flag, not the terminal (the owner's ruling,
    // 2026-10-01), so one argv always plans the same. The gate goes first: the closure
    // removes from `honoured`, so `&&` must short-circuit before it runs.
    let mut honoured = declined.clone().unwrap_or_default();
    let reopened = !args.json
        && only.is_some_and(|f| {
            onboarding::decline_keys(f.id()).fold(false, |had, k| honoured.remove(&k) | had)
        });
    let facts = Facts {
        // The registry's readout against the global config, as `mecha
        // features` reads it — the feature steps are built from it.
        features: feature_rows(&home),
        provider_credential: pcfg.resolve_api_key().is_some(),
        props,
        scheduler_installed: scheduler_installed(),
        trigger_count: trigger_count(&home),
        config_file: mecha_core::config::Config::global_path().is_some_and(|p| p.is_file()),
        local_probe,
        charter: onboarding::charter_state(&home.join("charter.toml")),
        // `None` is the store being unreadable, not empty. Resolved towards
        // offering everything (see `Facts::declined`), and said out loud
        // below rather than swallowed — a checklist that quietly stopped
        // honouring your answers would be the worse half of this feature.
        declined: honoured,
    };

    let mut steps = onboarding::plan(&cfg, &name, &facts);
    if let Some(f) = only {
        // One feature's step and nothing else (FEATURES-DESIGN.md §4.2 item
        // 2). A feature blocked on another is offered with the chained
        // command, so its dependency is taken first in the same step.
        steps.retain(|s| s.id == f.id());
        if reopened {
            eprintln!(
                "`{}` was declined before — offered again because you named it",
                f.id()
            );
        }
        // A switch written `false` is the owner's answer in the config;
        // naming the feature asks for it to be offered, so its way back
        // becomes the remedy. Nothing runs without a `y`.
        if !args.json {
            for s in &mut steps {
                if let (Status::Declined, Some(undo)) = (s.status, s.undo.clone()) {
                    s.status = Status::Missing;
                    s.remedy = Some(mecha_core::doctor::Remedy {
                        description: format!("Switch `{}` back on.", s.id),
                        argv: undo,
                        needs_terminal: false,
                    });
                }
            }
        }
    }

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

    // **On stderr, and before the `--json` return.** This warning used to sit
    // after `render`, so it was unreachable from the one surface this change
    // made scriptable: a corrupt `setup-declined.json` made `--json` emit
    // every declined step as `"status": "missing"` and exit 1, with nothing in
    // the payload telling *you declined nothing* from *your declines could not
    // be read*. Somebody's check goes red with no reason in the
    // machine-readable output — the rule the rest of this file argues for, one
    // surface over. stderr keeps stdout a parseable array while saying it to
    // both spellings.
    if declined.is_none() {
        eprintln!(
            "mecha: the declined-steps file at {} could not be read, so everything is \
             being offered again — that is a read failure, not an empty list",
            onboarding::declined_path(&home).display()
        );
    }

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
            crate::exit_with(1);
        }
        return Ok(());
    }
    if args.write {
        return write_verified(&name, pcfg.kind != "local", &facts);
    }
    if args.minimal {
        return minimal(&outstanding, &home);
    }
    render(&steps);

    if outstanding.is_empty() {
        println!("\nNothing outstanding.");
        finished_note(&steps);
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        println!("\n{} step(s) outstanding.", outstanding.len());
        // Exit 1 like doctor: a script can act on it, and a person sees the
        // same list either way.
        crate::exit_with(1);
    }
    offer(&outstanding, &home, &mut std::io::stdin().lock())?;
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
    // Each declined step carries its own way back (`Step::undo`): an answer
    // given here is taken back with `--undecline`, a switch written `false`
    // with `mecha features enable` — naming `--undecline` for both answered
    // "nothing to restore" for the second (found on review of #460).
    let undoing = |verb: &str| {
        steps.iter().any(|s| {
            s.status == Status::Declined
                && s.undo
                    .as_ref()
                    .is_some_and(|u| u.get(1).map(String::as_str) == Some(verb))
        })
    };
    // One group, set apart from `Next:` whichever lines it holds — a switch
    // alone used to land flush against `mecha doctor`, reading as a fourth
    // thing to do (review of #461).
    let (setup_undo, switch_undo) = (undoing("setup"), undoing("features"));
    if setup_undo || switch_undo {
        println!();
    }
    if setup_undo {
        println!("    mecha setup --undecline <id>   ask about a skipped step again");
    }
    if switch_undo {
        println!("    mecha features enable <id>     turn on one you switched off");
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

/// Offer each remedy, EOF is no — the outbox `send` convention, and
/// `doctor`'s: silence is not consent.
///
/// `never` records the step id in `~/.mecha/setup-declined.json` so the
/// question stops being asked. It is only ever offered for a step that has
/// one: a broken charter or a server that disagrees with its config is a
/// fault rather than an optional extra, and there is nothing coherent to
/// decline about being told.
///
/// **The de-duplication is on what has been *run*, not on what has been
/// asked**, and the difference is a bug that shipped. `mail` and `docs` carry
/// the identical remedy argv when neither binary is on PATH — one
/// `cargo install mecha-mail` satisfies both — so skipping the second
/// *command* is right and skipping the second *question* was not: a `never`
/// at the mail prompt recorded only `mail`, left `docs` outstanding, and
/// `mecha setup` still exited 1 after somebody had answered every question
/// they were asked. They are two features that happen to share an installer,
/// and declining one is not declining the other.
///
/// `read` is a parameter so the loop is testable without a terminal. That is
/// not incidental: `setup_exits_zero_once_everything_outstanding_has_been_answered`
/// builds the declined store by hand and so was structurally unable to see
/// the bug above — the only way to catch it is to drive the answers.
fn offer(steps: &[&Step], home: &std::path::Path, read: &mut impl std::io::BufRead) -> Result<()> {
    offer_with(steps, home, read, &|id| after_feature(id, home))
}

/// `offer`, with what to do after a remedy succeeds handed in — so the loop
/// is testable without the global config `after_feature` reads.
fn offer_with(
    steps: &[&Step],
    home: &std::path::Path,
    read: &mut impl std::io::BufRead,
    after: &dyn Fn(&str),
) -> Result<()> {
    let mut already_run: Vec<&[String]> = Vec::new();
    for s in steps {
        let Some(remedy) = &s.remedy else { continue };
        // Already satisfied by a command answered `y` earlier in this pass.
        // Not asked again, because there is nothing left to ask: the thing it
        // would install is installed.
        if already_run.contains(&remedy.argv.as_slice()) {
            println!("\n{}", remedy.description);
            println!("already handled by the command above");
            // Satisfied by the same command, so it deserves the same reading
            // of where it now stands — `mail` and `docs` share an installer,
            // and the install is rarely the last step for either (review of
            // #460).
            after(&s.id);
            continue;
        }
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
        let answer = match read.read_line(&mut line) {
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
            Answer::Yes => {
                // Only a command that *succeeded* satisfies the steps that
                // share it. A failed install leaves the next one genuinely
                // outstanding, so it gets asked rather than told the work is
                // already done.
                if run(remedy)? {
                    already_run.push(&remedy.argv);
                    after(&s.id);
                }
            }
            Answer::Skip => println!("skipped — asked again next time"),
            Answer::Never => match onboarding::decline(home, &s.id) {
                // Said with the undo in the same breath: a decision nobody
                // can find their way back out of is one people are right to
                // hesitate over.
                Ok(wrote) => {
                    println!(
                        "noted — `{}` will not be offered again (`mecha setup --undecline {}` undoes it)",
                        s.id, s.id
                    );
                    report_salvage(wrote.salvaged);
                }
                // A store that could not be written must not read as a
                // recorded answer, or the question silently comes back and
                // the person believes they already settled it.
                Err(e) => println!("could not record that: {e} — it will be offered again"),
            },
        }
    }
    Ok(())
}

/// The registry's rows against the global configuration — empty when it does
/// not load, which the plan's own config step already reports.
fn feature_rows(home: &std::path::Path) -> Vec<mecha_core::feature::Row> {
    mecha_core::config::Config::load_global()
        .map(|global| feature::all(&feature::Facts::read(home, &global)))
        .unwrap_or_default()
}

/// After a feature step's remedy worked: where the feature stands now, read
/// afresh, and what is next. A switch turned on is rarely the last move — an
/// `[[mcp]]` entry, an account, a settings table — and saying so here is
/// what keeps `mecha setup <feature>` from ending on a step that only looks
/// finished. Nothing for a step that is not a feature's.
fn after_feature(id: &str, home: &std::path::Path) {
    let Some(f) = Feature::parse(id).filter(|f| f.has_switch()) else {
        return;
    };
    let Some(row) = feature_rows(home).into_iter().find(|r| r.id == f) else {
        return;
    };
    match (&row.state, &row.next) {
        (feature::State::On { .. }, _) => println!("`{id}` is on"),
        (state, Some(next)) => println!(
            "`{id}` now reads {} — next: {next} (`mecha setup {id}` again once that is done)",
            state.word()
        ),
        (state, None) => println!("`{id}` now reads {}", state.word()),
    }
}

/// `--minimal`: decline every optional step still outstanding, in one pass,
/// and say which. Declines only — never a config write, so a later `mecha
/// features enable` or `--undecline` undoes any one of them.
fn minimal(outstanding: &[&Step], home: &std::path::Path) -> Result<()> {
    let mut declined = Vec::new();
    for s in outstanding
        .iter()
        .filter(|s| s.optional && s.status == Status::Missing)
    {
        let wrote = onboarding::decline(home, &s.id)?;
        report_salvage(wrote.salvaged);
        declined.push(s.id.as_str());
    }
    if declined.is_empty() {
        println!("nothing optional is outstanding — nothing to decline");
    } else {
        println!(
            "declined {} — `mecha setup --undecline <id>` asks again, `mecha features enable <id>` turns one on",
            declined.join(", ")
        );
    }
    // What is left is what cannot be declined: said, as a plain run says it.
    let left: Vec<&str> = outstanding
        .iter()
        .filter(|s| !(s.optional && s.status == Status::Missing))
        .map(|s| s.id.as_str())
        .collect();
    if !left.is_empty() {
        println!("still outstanding, and not declinable: {}", left.join(", "));
        crate::exit_with(1);
    }
    Ok(())
}

/// Inheriting the real terminal, never captured: an OAuth flow needs a
/// keyboard and a screen, and `.output()` gives it a pipe and a closed stdin.
///
/// **Returns whether it actually worked**, which the caller needs and used to
/// assume. A remedy that failed is not one that satisfied anything, and
/// `offer` records the argv as handled off this answer — so swallowing the
/// status made a failed `cargo install mecha-mail` print *"already handled by
/// the command above"* at the `docs` prompt, a claim about a command that did
/// not succeed. The house rule one layer down: grade the artifact, never the
/// report.
fn run(remedy: &Remedy) -> Result<bool> {
    let (program, rest) = remedy
        .argv
        .split_first()
        .context("a remedy with an empty argv")?;
    let status = std::process::Command::new(program).args(rest).status();
    Ok(match status {
        Ok(s) if s.success() => {
            println!("done");
            true
        }
        Ok(s) => {
            println!("that exited {s} — nothing else was changed");
            false
        }
        Err(e) => {
            println!("could not run it: {e}");
            false
        }
    })
}

/// What `--write` says after putting a bad config back.
///
/// Constants rather than literals inline, so they are **reachable from a
/// test**. They are printed only when a written `config.toml` fails to parse,
/// which needs a server answering `/props` with a hostile model name — not
/// something a unit test can stage — so the strings would otherwise be the
/// one piece of user-facing prose here that nothing ever reads. That is
/// exactly how the lost-`\`-continuation defect landed on this line: five
/// occurrences in this branch, and each guard added for it only covered the
/// producer the last one came from.
const RESTORED: &str = concat!(
    "the previous config has been put back, so nothing is broken — please report ",
    "this, it is a bug here rather than anything you did"
);
const NOT_RESTORED: &str = "the previous config could NOT be put back; it is at the .bak beside it";

/// Say when an unreadable decline store was moved aside rather than
/// overwritten.
///
/// Printed rather than swallowed because the alternative is the failure this
/// whole path exists to avoid: somebody's recorded answers disappearing with
/// no word. They are still on disk, and this is the only line that says
/// where.
fn report_salvage(salvaged: Option<std::path::PathBuf>) {
    if let Some(path) = salvaged {
        println!(
            "  (the previous declined-steps file could not be read and was kept at {} \n\
             rather than overwritten — earlier answers are in there, not lost)",
            path.display()
        );
    }
}

/// Ask the documented local address whether a **model server** is serving
/// there.
///
/// Fail-soft: `preflight::fetch` has a three-second timeout and answers `None`
/// for a connection refused or a non-2xx.
///
/// **Parsing is not identification, and this used to conflate them.**
/// `preflight::Props` defaults every field so that a llama-server version bump
/// costs a check rather than a parse failure — which means `{}` with a 200
/// deserializes perfectly, and any JSON service answering on :8080 came back
/// as `Some`. It was then announced as "already serving (an unnamed model)",
/// and one `y` would repoint `default_provider` at it with no `model` and no
/// `context_window`. `answers_like_a_model_server` is the identification step
/// that tolerance left out; it lives in core beside the claim it supports, so
/// it is testable without a socket.
async fn probe_for_a_local_server() -> onboarding::LocalProbe {
    for base_url in onboarding::local_probe_candidates() {
        match mecha_core::provider::preflight::fetch(base_url, None).await {
            Some(props) if onboarding::answers_like_a_model_server(&props) => {
                return onboarding::LocalProbe::Found(onboarding::LocalServer {
                    base_url: (*base_url).to_string(),
                    props,
                })
            }
            // Something answered, but nothing that identifies itself as a
            // model server. Keep looking, and report *asked and found
            // nothing* rather than treating a stranger's 200 as a discovery.
            _ => continue,
        }
    }
    onboarding::LocalProbe::NothingAnswered
}

/// Write back what the server said about itself.
///
/// Deliberately prints the diff and asks, even though the values came from
/// the server rather than from a guess: this edits a file somebody may have
/// had reasons for, and the whole argument for reading values off the wire is
/// weakened if the reading is also unattended.
fn write_verified(provider: &str, hosted: bool, facts: &Facts) -> Result<()> {
    // The probed case: a server is running that no provider names. Writing it
    // down is the remedy the blocking step now offers, and it is the same act
    // as the one below — read the values off the wire, show them, ask — with
    // one addition, which is that the table does not exist yet.
    if let onboarding::LocalProbe::Found(found) = &facts.local_probe {
        return write_local_provider(found);
    }
    let Some(props) = &facts.props else {
        // "Nothing answered" is a claim about a probe, and it is only true
        // when one ran. With a hosted default provider that already has its
        // key, no local server is looked for at all (`run`'s `local_probe`),
        // so a llama-server that is up and serving read as "start the
        // server" — the wrong remedy for a server that was never asked.
        // `NotAttempted` has two causes and only one of them is this: a
        // configured local provider that is merely down is also not probed,
        // and "Start the server" below is exactly right for it (found on
        // review). The provider's kind is what separates them: a credential
        // is not, since a llama-server can be configured with an
        // `api_key_env` (found on the second review).
        if matches!(facts.local_probe, onboarding::LocalProbe::NotAttempted) && hosted {
            anyhow::bail!(
                "no local server was checked: `{provider}` is a hosted provider, so \
                 nothing was probed. To record a llama-server, run \
                 `mecha setup --write --provider <name>` for your local provider — adding a \
                 `[providers.<name>]` with `kind = \"local\"` and its `base_url` first if \
                 there is none."
            );
        }
        anyhow::bail!("nothing answered, so there is nothing to write down. Start the server.");
    };
    offer_settings(provider, props).map(|_| ())
}

/// Show what a server reports for an existing provider table, ask, and write
/// it — `--write`'s step, shared with `mecha setup chat` when the model it
/// installed replaces one the table already names. `true` when written: a
/// caller that goes on to move `default_provider` must not point it at a
/// table the owner declined to update (found on review of #618).
pub(super) fn offer_settings(
    provider: &str,
    props: &mecha_core::provider::preflight::Props,
) -> Result<bool> {
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
            return Ok(false);
        }
    } else {
        println!("\n(not a terminal, so nothing was written — copy the lines above)");
        return Ok(false);
    }
    apply(provider, &settings).map(|()| true)
}

/// Offer to make `provider` the default when `current` is something else —
/// `mecha setup chat` run against a config that already names its router but
/// still answers from a hosted model. Installing the router and its weights
/// and exiting 0 while every run still went to `current` is ruling F12's
/// case, reached through the command that exists to avoid it (found on
/// review of #568). Asked, never assumed: it changes what answers.
pub(super) fn offer_default(provider: &str, current: &str) -> Result<()> {
    if provider == current {
        return Ok(());
    }
    println!(
        "\nThe default provider is `{current}`, so runs do not use this model yet. To change \
         that:\n\n    default_provider = {}",
        onboarding::toml_string(provider)
    );
    if !std::io::stdin().is_terminal() {
        println!("\n(not a terminal, so nothing was written — copy the line above)");
        return Ok(());
    }
    print!("\nmake `{provider}` the default provider? [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut line)?;
    if !line.trim().eq_ignore_ascii_case("y") {
        println!("not written — runs still go to `{current}`");
        return Ok(());
    }
    let path = mecha_core::config::Config::global_path()
        .context("no global config path — is $HOME set?")?;
    backup(&path)?;
    set_default_provider(&path, provider)?;
    // Checked, not claimed — the same read-back `write_local_provider` makes.
    if let Err(e) = mecha_core::config::Config::load_global() {
        let restored = std::fs::copy(path.with_extension("toml.bak"), &path).is_ok();
        eprintln!(
            "what was written to {} does not parse: {e:#}",
            path.display()
        );
        eprintln!("{}", if restored { RESTORED } else { NOT_RESTORED });
        crate::exit_with(1);
    }
    println!("`{provider}` is now the default provider");
    Ok(())
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
pub(super) fn write_local_provider(found: &onboarding::LocalServer) -> Result<()> {
    let settings = onboarding::verified_settings(&found.props);
    println!(
        "Found a server at {} and nothing in the config names it.\n",
        found.base_url
    );
    println!("    [providers.local]");
    println!("    kind = \"local\"");
    println!(
        "    base_url = {}",
        onboarding::toml_string(&found.base_url)
    );
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
        format!("base_url = {}", onboarding::toml_string(&found.base_url)),
    ];
    table.extend(settings.iter().map(|(k, v)| format!("{k} = {v}")));
    append_table(&path, "local", &table)?;
    set_default_provider(&path, "local")?;

    // **Checked, not claimed.** `CharterEdit::SavedButInvalid` exists a few
    // files over because "saved" must never be said about a document a run
    // cannot load; the same applies here and harder, because these bytes came
    // off a server nobody named — `answers_like_a_model_server` establishes
    // that a stranger can be answering :8080 — and a `config.toml` that does
    // not parse takes out every later command with an error pointing at
    // `mecha config init` rather than at what actually happened. Recovery
    // would be finding `config.toml.bak` by hand.
    //
    // Read back through the loader a *run* uses, not a parse written here,
    // for the reason the charter path gives: there must be one answer to
    // "would this load".
    if let Err(e) = mecha_core::config::Config::load_global() {
        let backup = path.with_extension("toml.bak");
        let restored = std::fs::copy(&backup, &path).is_ok();
        eprintln!(
            "what was written to {} does not parse: {e:#}",
            path.display()
        );
        eprintln!("{}", if restored { RESTORED } else { NOT_RESTORED });
        crate::exit_with(1);
    }

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
    // Through `toml_string` like every other value this module writes —
    // live, not latent: `offer_default` passes a provider name read from the
    // config, and `ARCHITECTURE.md` claims *both* values on this path are
    // escaped for TOML.
    let assignment = format!("default_provider = {}", onboarding::toml_string(provider));
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
    let lines =
        apply_text(&text, provider, settings).with_context(|| format!("in {}", path.display()))?;
    let backup = path.with_extension("toml.bak");
    std::fs::copy(&path, &backup).ok();
    std::fs::write(&path, lines)?;
    println!(
        "written to {} (previous copy at {})",
        path.display(),
        backup.display()
    );
    Ok(())
}

/// The text of `apply`: each setting replaces its key in `[providers.<p>]`,
/// or is added after the table's last assignment. Not before the next
/// header — the comment block that introduces the next table sits above it,
/// and a key put there reads as that table's (found writing F12, where the
/// starter's `[providers.anthropic]` follows `[providers.local]`).
fn apply_text(text: &str, provider: &str, settings: &[(&'static str, String)]) -> Result<String> {
    let header = format!("[providers.{provider}]");
    let Some(start) = text.lines().position(|l| l.trim() == header) else {
        anyhow::bail!("no {header} table");
    };
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, l)| l.trim_start().starts_with('['))
        .map(|(i, _)| i)
        .unwrap_or(lines.len());
    let assignment_line = |l: &str| {
        let t = l.trim_start();
        !t.is_empty() && !t.starts_with('#') && t.contains('=')
    };
    let mut after = (start + 1..end)
        .rev()
        .find(|&i| assignment_line(&lines[i]))
        .map_or(start + 1, |i| i + 1);

    for (key, value) in settings {
        let assignment = format!("{key} = {value}");
        match (start + 1..end).find(|&i| lines[i].split('=').next().map(str::trim) == Some(*key)) {
            Some(i) => lines[i] = assignment,
            None => {
                lines.insert(after, assignment);
                after += 1;
                end += 1;
            }
        }
    }
    Ok(lines.join("\n") + "\n")
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

    /// `--write` and `setup chat` fill the starter's `[providers.local]`
    /// (ruling F12) without landing under the next table's introduction,
    /// and a key already set is replaced in place.
    #[test]
    fn settings_land_in_their_own_table_of_the_starter() {
        let settings = [
            ("model", "\"served-alias\"".to_string()),
            ("context_window", "65536".to_string()),
            ("vision", "true".to_string()),
        ];
        let text = apply_text(super::super::config::STARTER, "local", &settings).unwrap();
        let cfg: toml::Value = toml::from_str(&text).unwrap();
        let local = &cfg["providers"]["local"];
        assert_eq!(local["model"].as_str(), Some("served-alias"));
        assert_eq!(local["context_window"].as_integer(), Some(65536));
        assert_eq!(local["vision"].as_bool(), Some(true));
        assert_eq!(cfg["default_provider"].as_str(), Some("local"));
        let pos = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("{needle}"));
        assert!(pos("vision = true") < pos("# A hosted model"), "{text}");
        assert!(pos("base_url") < pos("model = \"served-alias\""), "{text}");

        let again = apply_text(&text, "local", &[("model", "\"other\"".to_string())]).unwrap();
        assert_eq!(
            again.matches("model = \"").count(),
            text.matches("model = \"").count()
        );
        assert!(again.contains("model = \"other\"") && !again.contains("served-alias"));
        assert!(apply_text(&text, "nowhere", &settings).is_err());
    }

    /// `NotAttempted` has two causes, and `--write` must name the right one.
    /// A hosted provider with its key skipped the probe: say nothing was
    /// checked. A configured local provider that is down skipped it too, and
    /// for that one "Start the server" is the answer — the first cut of this
    /// told it its provider "is not a local one" (found on review).
    #[test]
    fn write_without_props_names_why_nothing_answered() {
        let hosted = Facts {
            provider_credential: true,
            ..Facts::default()
        };
        let err = write_verified("anthropic", true, &hosted)
            .unwrap_err()
            .to_string();
        assert!(err.contains("no local server was checked"), "{err}");

        let local_but_down = Facts {
            provider_credential: false,
            ..Facts::default()
        };
        let err = write_verified("local", false, &local_but_down)
            .unwrap_err()
            .to_string();
        assert!(err.contains("Start the server"), "{err}");
        assert!(!err.contains("hosted"), "{err}");

        // A local server behind an API key is still local: down means start it.
        let local_with_a_key = Facts {
            provider_credential: true,
            ..Facts::default()
        };
        let err = write_verified("local", false, &local_with_a_key)
            .unwrap_err()
            .to_string();
        assert!(err.contains("Start the server"), "{err}");
    }

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    fn step_with(id: &str, argv: &[&str], optional: bool) -> Step {
        let mut s = Step {
            id: id.into(),
            title: id.into(),
            status: Status::Missing,
            detail: String::new(),
            remedy: Some(Remedy {
                description: format!("install for {id}"),
                argv: argv.iter().map(|a| a.to_string()).collect(),
                needs_terminal: false,
            }),
            optional,
            undo: None,
        };
        s.status = Status::Missing;
        s
    }

    fn scratch_home(tag: u32) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mecha-offer-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// **Two steps sharing one installer are two questions, not one.**
    ///
    /// `mail` and `docs` carry the identical remedy argv when neither binary
    /// is on PATH, and the de-duplication skipped the second *question* as
    /// well as the second command. So `never` at the mail prompt recorded
    /// only `mail`, left `docs` outstanding, and `mecha setup` still exited 1
    /// after somebody had answered every question they were asked — which is
    /// the one contract this whole feature is.
    ///
    /// It self-corrected on the next pass, which is exactly why no
    /// store-level test could see it: the state after two runs was right.
    #[test]
    fn declining_one_of_two_steps_sharing_an_installer_declines_only_that_one() {
        let home = scratch_home(line!());
        let argv = ["cargo", "install", "mecha-mail", "--locked"];
        let mail = step_with("mail", &argv, true);
        let docs = step_with("docs", &argv, true);
        let steps = [&mail, &docs];

        // "never" to the first, "never" to the second: both asked, both
        // recorded.
        let mut input = std::io::Cursor::new(b"never\nnever\n".to_vec());
        offer_with(&steps, &home, &mut input, &|_| {}).unwrap();
        let declined = onboarding::read_declined(&home).unwrap();
        assert!(
            declined.contains("mail") && declined.contains("docs"),
            "both questions must be asked and answered separately: {declined:?}"
        );

        // And declining only the first leaves the second genuinely open,
        // rather than silently swallowing it.
        let home = scratch_home(line!());
        let mut input = std::io::Cursor::new(b"never\n\n".to_vec());
        offer_with(&steps, &home, &mut input, &|_| {}).unwrap();
        let declined = onboarding::read_declined(&home).unwrap();
        assert!(declined.contains("mail"));
        assert!(
            !declined.contains("docs"),
            "a skip is not a decline, and one answer is not two: {declined:?}"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    /// After a remedy that worked, and only then, setup says where the step's
    /// feature now stands (`after_feature`) — including a step the same
    /// command satisfied, which is not asked again (`mail` and `docs` share an
    /// installer). A failed command moved nothing, and a skipped step ran
    /// nothing to report on. Each fixture has its own argv so every one is
    /// actually reached — an earlier version shared one, and the skip was
    /// never asked (review of #460).
    #[test]
    fn only_a_remedy_that_worked_is_followed_by_where_the_feature_stands() {
        let home = scratch_home(line!());
        let worked = step_with("mail", &["true"], true);
        let shared = step_with("docs", &["true"], true);
        let failed = step_with("graph", &["false"], true);
        let skipped = step_with("web", &["true", "skipped"], true);
        let seen = std::cell::RefCell::new(Vec::new());
        // mail: y; docs: not asked (handled above); graph: y, fails; web: skip.
        let mut input = std::io::Cursor::new(b"y\ny\n\n".to_vec());
        offer_with(
            &[&worked, &shared, &failed, &skipped],
            &home,
            &mut input,
            &|id| seen.borrow_mut().push(id.to_string()),
        )
        .unwrap();
        assert_eq!(*seen.borrow(), ["mail", "docs"]);
        assert_eq!(
            input.position() as usize,
            input.get_ref().len(),
            "every answer was read, so the skip was really asked"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    /// **A command that failed has satisfied nothing.**
    ///
    /// `offer` records an argv as handled off the `y` answer, and `run`
    /// swallowed the exit status — so a failed `cargo install mecha-mail`
    /// printed "already handled by the command above" at the `docs` prompt,
    /// asserting an outcome nothing had checked. The pair below distinguishes
    /// the two by whether the second step is *asked*: a skipped step consumes
    /// no answer, so the `never` reaches `docs` only when it was.
    #[test]
    fn a_failed_command_does_not_mark_the_steps_that_share_it_as_handled() {
        // `false` exits 1, `true` exits 0 — real binaries on every platform
        // this builds for, and the cheapest honest way to spawn each outcome.
        let failing = [
            step_with("mail", &["false"], true),
            step_with("docs", &["false"], true),
        ];
        let home = scratch_home(line!());
        let mut input = std::io::Cursor::new(b"y\nnever\n".to_vec());
        offer_with(&[&failing[0], &failing[1]], &home, &mut input, &|_| {}).unwrap();
        assert!(
            onboarding::read_declined(&home).unwrap().contains("docs"),
            "the install failed, so `docs` is still outstanding and must be asked"
        );
        let _ = std::fs::remove_dir_all(&home);

        // And the contrast: a command that worked *does* satisfy both, so the
        // second is not asked and the `never` is never consumed.
        let working = [
            step_with("mail", &["true"], true),
            step_with("docs", &["true"], true),
        ];
        let home = scratch_home(line!());
        let mut input = std::io::Cursor::new(b"y\nnever\n".to_vec());
        offer_with(&[&working[0], &working[1]], &home, &mut input, &|_| {}).unwrap();
        assert!(
            onboarding::read_declined(&home).unwrap().is_empty(),
            "one successful install satisfies both, so nothing was declined"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    /// EOF is a skip, never a decline — silence is not consent, and it is
    /// emphatically not a permanent decision either.
    #[test]
    fn silence_declines_nothing() {
        let home = scratch_home(line!());
        let mail = step_with("mail", &["cargo", "install", "mecha-mail"], true);
        let mut input = std::io::Cursor::new(Vec::new());
        offer_with(&[&mail], &home, &mut input, &|_| {}).unwrap();
        assert!(onboarding::read_declined(&home).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    /// A step that is not optional cannot be declined at the prompt, whatever
    /// is typed — the same guarantee `plan` enforces over the store, kept at
    /// the other end so neither is the only thing standing between a
    /// credential and a "never".
    #[test]
    fn a_non_optional_step_cannot_be_declined_at_the_prompt() {
        let home = scratch_home(line!());
        let credential = step_with("provider-credential", &["mecha", "config", "show"], false);
        let mut input = std::io::Cursor::new(b"never\n".to_vec());
        offer_with(&[&credential], &home, &mut input, &|_| {}).unwrap();
        assert!(
            onboarding::read_declined(&home).unwrap().is_empty(),
            "`never` on a required step is not an answer this may record"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The prose this module prints reads as prose.
    ///
    /// **Fifth occurrence of the same defect in one branch**, and the fourth
    /// guard: a `\`-continued literal that loses its backslash keeps the
    /// source's indentation mid-sentence. Each previous guard covered the
    /// producer the last instance came from — `plan()`'s step details, then
    /// named lines of `setup`'s stdout — and this one landed on a path
    /// neither reaches, printed only when a written config will not parse.
    ///
    /// Listed by name rather than scanned from source: the intentional
    /// column alignment elsewhere in this file (`finished_note`'s command
    /// list) is indistinguishable from the defect by any pattern simple
    /// enough to keep, and a check that fires on correct code gets deleted.
    #[test]
    fn the_prose_this_module_prints_carries_no_source_indentation() {
        for (name, text) in [("RESTORED", RESTORED), ("NOT_RESTORED", NOT_RESTORED)] {
            assert!(
                !text.contains("   "),
                "{name} carries indentation from its source literal: {text:?}"
            );
            assert!(!text.is_empty());
        }
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
