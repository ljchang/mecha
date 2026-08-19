//! `mecha diagnose` — read the run corpus and propose one change to try.
//!
//! The stage between `doctor` (something is wrong) and `eval --ab-config`
//! (did the fix help). It is the only place in this loop a model authors
//! anything, and everything about how it is run is arranged around the fact
//! that it will usually be wrong: the proposal is typed, it carries a
//! prediction, and it is printed with the exact command that would falsify it
//! rather than applied.
//!
//! **It does not measure, and it does not apply.** Running the arms costs a
//! real model run per case per arm, so making that automatic would put an
//! hour of inference behind a command whose output is a suggestion. The
//! measurement is one line away and the human decides to spend it.

use crate::{setup, GlobalOpts};
use anyhow::{Context, Result};
use mecha_core::agent::Conversation;
use mecha_core::diagnose::{
    carries_over, parse_proposal, Evidence, DIAGNOSE_INSTRUCTION, DIAGNOSE_SYSTEM,
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

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let dir = Session::default_dir()?;
    let since = args
        .days
        .map(|d| chrono::Utc::now() - chrono::Duration::days(d));
    let corpus = Corpus::scan(
        &dir,
        &Scan {
            max_sessions: Some(args.limit),
            since,
        },
    )?;
    anyhow::ensure!(
        !corpus.is_empty(),
        "no recorded run outcomes in {} ({} session(s) read) — outcomes are recorded from \
         v0.1.8 on, so the corpus fills as you use it",
        dir.display(),
        corpus.sessions_read
    );

    // One model, because a rate blended across two describes neither.
    let by_model = corpus.by_model();
    let (model, slice) = match &args.model {
        Some(want) => {
            let slice = by_model
                .get(want)
                .with_context(|| format!("no recorded runs for model `{want}`"))?;
            (want.clone(), slice.clone())
        }
        None => {
            let (model, slice) = by_model
                .iter()
                .max_by_key(|(_, c)| c.len())
                .expect("corpus is not empty");
            (model.clone(), slice.clone())
        }
    };

    let mut evidence = Evidence::of(&model, &slice);
    // Doctor's own words, which are machine-authored — the findings are the
    // only text in the brief and they were written by this program.
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

    let brief = evidence.brief();
    if args.dry_run {
        println!("{brief}");
        return Ok(());
    }

    // Read-only, and the workspace is wherever the user is standing: the
    // diagnostician's most useful input is this repository's own
    // documentation, which records why each mechanism exists and is what
    // stops a proposal unpicking something load-bearing.
    let opts = GlobalOpts {
        read_only: true,
        yes: false,
        system: Some(DIAGNOSE_SYSTEM.to_string()),
        no_outbox: true,
        no_learned_rules: true,
        ..global.clone()
    };
    let prepared = setup::prepare(&opts, false).await?;

    eprintln!(
        "diagnosing {} run(s) of `{model}` · {} ({})",
        slice.len(),
        prepared.model,
        prepared.provider_name
    );

    let mut convo = Conversation::user(format!("{brief}\n---\n{DIAGNOSE_INSTRUCTION}"));
    let outcome = prepared.agent.run(&mut convo, None).await?;

    println!("{}\n", outcome.text.trim());

    let Some(proposal) = parse_proposal(&outcome.text) else {
        println!(
            "no proposal — the diagnostician either found nothing worth changing or wrote a \
             block that could not be measured. Both are better than a change nobody can \
             falsify."
        );
        return Ok(());
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

    println!("── proposal ──");
    println!("class:     {:?}", proposal.class);
    println!("change:    {}", proposal.change);
    println!("predicts:  lower {:?}", proposal.metric);
    println!("because:   {}", proposal.rationale);

    if let Some(run) = quoted {
        println!(
            "\nREFUSED: the proposal reproduces what it read — \"{run}\". A conclusion drawn \
             from a source is a proposal; a sentence lifted from one is the source's, and a \
             sentence in the prompt prefix is the longest-lived thing in this system."
        );
        return Ok(());
    }

    println!("\nnothing to do here yet — measure it:");
    println!(
        "  mecha eval --ab-config {} eval/cases.jsonl",
        proposal.change
    );
    println!(
        "\nthat runs the case set twice and judges the difference against a holdout. Until it \
         does, this is a guess: automated failure attribution is right about which step failed \
         roughly one time in seven, which is exactly why nothing here is applied."
    );
    Ok(())
}
