//! `mecha eval` — the model bake-off rig.
//!
//! Runs a case set against a model and prints a scorecard. Built to answer one
//! question: *which model should I run locally?* Final text is a weak signal for
//! that, so cases are graded on the tool-call trace.
//!
//! Runs are forced read-only against a fixture workspace. That makes them
//! reproducible, safe at high concurrency, and comparable across models —
//! without it, case N's writes change what case N+1 sees.
//!
//! A case marked `sandbox` opts out of exactly that, and pays for it: it gets a
//! throwaway copy of the fixture all to itself, with writing tools enabled. The
//! shared fixture is still never mutated, so the two kinds of case can run in
//! the same pass at the same concurrency.

use crate::setup::NoOneToAsk;
use crate::{setup, GlobalOpts};
use anyhow::{Context, Result};
use mecha_core::agent::{Budget, RunContext};
use mecha_core::config::PermissionMode;
use mecha_core::eval::{grade, stage_workspace, EvalCase, GradedCase, Judge, Scorecard};
use mecha_core::harness::Lever;
use mecha_core::tool::ask::AskUserTool;
use mecha_core::tool::ModeApprover;
use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(clap::Args, Debug)]
pub struct Args {
    /// JSONL case file. Defaults to `eval/cases.jsonl`.
    #[arg(default_value = "eval/cases.jsonl")]
    pub cases: PathBuf,

    /// Workspace the agent reads during the run. Defaults to a `workspace`
    /// directory beside the case file.
    #[arg(long)]
    pub fixture: Option<PathBuf>,

    /// Write the full scorecard and per-case detail here as JSON.
    #[arg(long, short = 'o')]
    pub out: Option<PathBuf>,

    /// How many cases to run at once.
    #[arg(long, short = 'c', default_value_t = 4)]
    pub concurrency: usize,

    /// Run every case this many times and report pass^k — the fraction of
    /// cases that pass *all* k runs — beside pass@k (any run).
    ///
    /// Reliability decays much faster than mean success, and a single-run
    /// scorecard cannot see it: a case that passes 4 runs of 5 looks identical
    /// to one that passes 5 of 5, and the flaky one is the finding. Costs k×
    /// the inference, so the default stays 1.
    #[arg(long, short = 'k', default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
    pub runs: u32,

    /// Run only cases carrying this tag (repeatable).
    #[arg(long = "tag")]
    pub tags: Vec<String>,

    /// Show every failed check, not just the count.
    #[arg(long)]
    pub failures: bool,

    /// Model that grades `expect.judge` rubrics. Defaults to the model under
    /// test, which is worth avoiding — see the warning it prints.
    #[arg(long)]
    pub judge_model: Option<String>,

    /// Provider entry the judge model runs on. Defaults to the one under test.
    #[arg(long)]
    pub judge_provider: Option<String>,

    /// Keep the staged workspaces of sandboxed cases instead of deleting them.
    /// The only way to see what a failing case actually wrote.
    #[arg(long)]
    pub keep_workspaces: bool,

    /// Connect MCP servers during the eval.
    ///
    /// Off by default, which is a reproducibility decision rather than a
    /// performance one: a scorecard that silently depends on whichever servers
    /// happen to be configured on this machine today is not comparable to one
    /// taken yesterday, or on another machine, or by anyone else. An eval
    /// measures a model against a *fixed* tool surface.
    #[arg(long)]
    pub mcp: bool,

    /// Connect exactly the MCP servers named in this TOML file (`[[mcp]]`
    /// tables), instead of the machine's own config.
    ///
    /// This is how a case file measures against *fixture* servers — the same
    /// reproducibility rule that keeps `--mcp` off by default, made positive:
    /// the file travels with the cases, so the tool surface is the same on
    /// every machine. Relative paths in a server's `command`/`args` resolve
    /// against the file's directory. A server that fails to connect is fatal
    /// here where `setup` merely warns: a case written against fixture tools
    /// measures nothing without them.
    #[arg(long, conflicts_with = "mcp", value_name = "PATH")]
    pub mcp_file: Option<PathBuf>,

    /// Withhold `ask_user`, which is otherwise part of the tool surface.
    ///
    /// On by default because it is part of the harness people actually run, and
    /// an eval that withholds it measures a configuration nobody uses. Nobody
    /// is watching, so every question goes unanswered — which makes *asking*
    /// the thing a case can assert on, deterministically, instead of paying a
    /// judge to opine on whether the model asked nicely enough.
    #[arg(long)]
    pub no_ask_user: bool,

    /// Compare previously written scorecards side by side instead of running.
    #[arg(long, num_args = 1.., conflicts_with_all = ["out", "fixture"])]
    pub compare: Vec<PathBuf>,

    /// Run the whole set twice — rules-free, then with this machine's learned
    /// rules — and report the per-case flips.
    ///
    /// The deliberate opt-in the no-learned-rules default reserves space for:
    /// the delta is its own artifact, and neither arm is printed or written
    /// as an ordinary scorecard, because a scorecard shaped by what this
    /// machine learned last night is not comparable to anyone else's. This is
    /// the coarse task-outcome A/B beside the validation ledger's probes —
    /// with `--out`, both arms and the flips land in one clearly-marked JSON.
    #[arg(long, conflicts_with = "compare")]
    pub ab_rules: bool,

    /// Measure a candidate config change: run the case set once bare
    /// and once with these overrides, and judge the difference.
    ///
    /// `KEY=VALUE`, repeatable. Keys are run options, which is the honest
    /// boundary — the knobs an automated proposer may move are exactly the
    /// ones a run can be launched with, so both arms are built by the same
    /// code path and differ only in the override. A second construction site
    /// is how two arms silently stop being comparable.
    ///
    /// Graded on case *outcome* rather than on harness counters, because a
    /// replay holds tool results fixed and cannot see a change in what the
    /// model said — this is the content-sensitive arm a prose or prompt-facing
    /// change needs.
    #[arg(
        long = "ab-config",
        value_name = "KEY=VALUE",
        conflicts_with_all = ["compare", "ab_rules"]
    )]
    pub ab_config: Vec<String>,

    /// One episode in this many is held out of selection, for `--ab-config`
    /// and `--ab-rules`.
    #[arg(long, default_value_t = 3, value_name = "N")]
    pub holdout_in: u64,
}

/// A candidate config override, parsed from `KEY=VALUE`.
///
/// Deliberately a closed set. An open one would let a proposer reach settings
/// whose effect is not measurable by this comparison — or worse, security
/// settings, which are never a measurement's to decide.
fn apply_override(opts: &mut GlobalOpts, spec: &str) -> Result<()> {
    // The closed set itself lives in `mecha_core::harness` — one definition,
    // because `mecha harness` measures candidates by replay and applies them
    // through the same parse, and a second spelling of the set here is how
    // the two arms would silently stop being comparable.
    use mecha_core::harness::{parse_change, OverrideKey};
    let change =
        parse_change(spec).with_context(|| format!("--ab-config could not use `{spec}`"))?;
    match change.key {
        OverrideKey::CompactAtTokens => opts.compact_at = Some(change.value.parse()?),
        OverrideKey::MaxTurns => opts.max_turns = Some(change.value.parse()?),
        OverrideKey::MaxOutputTokens => opts.max_output_tokens = Some(change.value.parse()?),
        OverrideKey::Effort => {
            opts.effort = Some(change.value.parse().map_err(|e| anyhow::anyhow!("{e}"))?)
        }
    }
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Report {
    /// The one-arm experiment this scorecard was recorded as, when it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    experiment: Option<String>,
    scorecard: Scorecard,
    cases: Vec<serde_json::Value>,
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    if !args.compare.is_empty() {
        return compare(&args.compare);
    }

    let cases = load_cases(&args.cases, &args.tags)?;
    anyhow::ensure!(!cases.is_empty(), "no cases to run");

    let fixture = args.fixture.clone().unwrap_or_else(|| {
        args.cases
            .parent()
            .unwrap_or(Path::new("."))
            .join("workspace")
    });
    anyhow::ensure!(
        fixture.is_dir(),
        "fixture workspace {} does not exist",
        fixture.display()
    );

    if args.ab_rules {
        return ab_rules(global, &args, &cases, &fixture).await;
    }
    if !args.ab_config.is_empty() {
        return ab_config(global, &args, &cases, &fixture).await;
    }

    // A scorecard is a one-arm experiment, and is recorded as one before the
    // arm runs: the model, the preset, the machine's knobs and the flag's
    // opt-ins on a manifest, each case a trial row on the store — so the
    // condition a scorecard measured sits beside every comparison's and
    // `mecha exp status|export` read it. The scorecard is still what this
    // prints. The one-arm record is the convergence's last step short of
    // the runner itself (the owner's ruling, 2026-09-04).
    let recorded = record_measurement(global, &args, &cases, &fixture);
    let mut name = match &recorded {
        Ok(Some((store, manifest))) => {
            eprintln!(
                "recorded as experiment `{}` ({})",
                manifest.name,
                store.root().display()
            );
            Some(manifest.name.clone())
        }
        // A deliberate skip reads as one: "nothing happened" and "nothing
        // went wrong" are opposite findings, and the store-failed sentence
        // below must stay alarming (found on review).
        Ok(None) => {
            eprintln!(
                "mecha eval: not recorded as an experiment, by design: {}",
                measurement_skip(args.mcp_file.is_some(), args.no_ask_user).unwrap_or("skipped")
            );
            None
        }
        Err(e) => {
            eprintln!(
                "mecha eval: not recorded as an experiment ({e:#}); the scorecard still runs"
            );
            None
        }
    };

    let (scorecard, graded) = run_arm(global, &args, &cases, &fixture, false, &[], "").await?;

    if let Ok(Some((store, manifest))) = &recorded {
        let task_ids: Vec<String> = cases.iter().map(|c| c.id.clone()).collect();
        // One row per run: the planned row's `repetition` is the graded
        // run's `run` number, and a row with no run behind it (a case that
        // never ran) is left pending rather than written empty. Write
        // failures are counted and said once, as the A/B says them — and
        // a record no row reached is not named in the report, because
        // `experiment` exists to point at a record, and a manifest over
        // zero trials reads as "the eval never ran", the opposite of what
        // happened (found on review).
        let (mut saved, mut unsaved) = (0usize, 0usize);
        for planned in manifest.trials(&task_ids, &scorecard.provider, &scorecard.model) {
            let Some(run) = graded
                .iter()
                .find(|g| g.id == planned.task && g.run == planned.repetition)
            else {
                continue;
            };
            match store.save_trial(&trial_of(&planned, std::slice::from_ref(&run))) {
                Ok(()) => saved += 1,
                Err(e) => {
                    unsaved += 1;
                    eprintln!(
                        "mecha eval: trial `{}` could not be written: {e:#}",
                        planned.id
                    );
                }
            }
        }
        if unsaved > 0 {
            eprintln!(
                "mecha eval: {unsaved} trial row(s) not on the store; `mecha exp status {}` will show them pending",
                manifest.name
            );
        }
        if saved == 0 && unsaved > 0 {
            eprintln!(
                "mecha eval: no trial row reached `{}`; the report will not name it",
                manifest.name
            );
            name = None;
        }
    }

    print_scorecard(&scorecard, &graded, args.failures);

    if let Some(path) = &args.out {
        let report = Report {
            experiment: name,
            scorecard: scorecard.clone(),
            cases: graded
                .iter()
                .map(|g| serde_json::to_value(g).unwrap_or(serde_json::Value::Null))
                .collect(),
        };
        std::fs::write(path, serde_json::to_string_pretty(&report)?)
            .with_context(|| format!("writing {}", path.display()))?;
        eprintln!("\nwrote {}", path.display());
    }

    // Non-zero when anything failed, so this can gate CI.
    if scorecard.passed < scorecard.total {
        std::process::exit(1);
    }
    Ok(())
}

/// One full pass over the case set: prepare, run, grade. `with_rules` is the
/// A/B lever — everything else about the two arms is identical by
/// construction, because both come through here.
async fn run_arm(
    global: &GlobalOpts,
    args: &Args,
    cases: &[EvalCase],
    fixture: &Path,
    with_rules: bool,
    overrides: &[String],
    arm: &str,
) -> Result<(Scorecard, Vec<GradedCase>)> {
    // Force read-only and point the workspace at the fixture, whatever the
    // caller's flags or config said. An eval that can mutate its own fixture
    // isn't measuring anything repeatable.
    let mut opts = GlobalOpts {
        workspace: Some(fixture.to_path_buf()),
        read_only: true,
        yes: false,
        ..global.clone()
    };
    // Everything a scorecard must not depend on, in one call — see
    // `force_reproducible` for why the list is a function rather than forty
    // lines of prose, and which entry that shape lost.
    force_reproducible(&mut opts, args.mcp, with_rules);
    // The candidate arm's whole difference from the baseline, applied here so
    // both arms are built by one code path. Empty for every other caller.
    for spec in overrides {
        apply_override(&mut opts, spec)?;
    }
    let mut prepared = setup::prepare(&opts, false).await?;
    if !args.no_ask_user {
        prepared
            .agent
            .registry_mut()
            .insert(Arc::new(AskUserTool::new(Arc::new(NoOneToAsk))));
    }

    // Held for the whole run: dropping a client kills its server.
    let mut _fixture_mcp = Vec::new();
    if let Some(path) = &args.mcp_file {
        let servers = load_mcp_file(path)?;
        let sandbox = mecha_core::sandbox::Sandbox::new(prepared.config.sandbox.clone());
        let (tools, clients, errors) =
            mecha_core::mcp::connect_all(&servers, &sandbox, fixture).await;
        // Fatal where `setup` warns: a case file written against fixture
        // tools measures nothing without them — the judge rule again.
        anyhow::ensure!(
            errors.is_empty(),
            "fixture MCP server(s) failed to connect: {}",
            errors.join("; ")
        );
        for tool in tools {
            prepared.agent.registry_mut().insert(tool);
        }
        _fixture_mcp = clients;
    }

    // Build the judge before running anything. A case set that cannot be
    // graded should fail in the first second, not after an hour of inference.
    let judge = build_judge(args, &prepared, cases)?;

    // With `--runs k` every case becomes k independent batch items — each its
    // own conversation, and (for sandboxed cases) its own staged workspace —
    // so the k samples are as independent as the harness can make them. The
    // suffixed ids keep results, contexts and staging directories from
    // colliding; grading maps them back to the case.
    let runs = args.runs;
    let mut items = Vec::new();
    let mut item_of: HashMap<String, (usize, u32)> = HashMap::new();
    for (i, case) in cases.iter().enumerate() {
        for run in 1..=runs {
            let id = if runs == 1 {
                case.id.clone()
            } else {
                format!("{}#r{run}", case.id)
            };
            item_of.insert(id.clone(), (i, run));
            items.push(mecha_core::batch::BatchItem {
                id,
                prompt: case.prompt.clone(),
                meta: None,
            });
        }
    }

    // Stage a private workspace for every sandboxed item, up front, so a
    // staging failure is not discovered halfway through the run. The arm
    // label keeps an A/B's two passes — same process, same pid — from
    // staging into each other's directories.
    let sandbox_root = std::env::temp_dir().join(format!("mecha-eval-{}{arm}", std::process::id()));
    let item_cases: Vec<(&str, &EvalCase)> = items
        .iter()
        .map(|it| (it.id.as_str(), &cases[item_of[&it.id].0]))
        .collect();
    let contexts = prepare_contexts(&item_cases, fixture, &sandbox_root, &prepared)?;
    let sandboxed = item_cases.iter().filter(|(_, c)| c.sandbox).count();

    eprintln!(
        "mecha eval: {} cases{} · {} ({}) · {} tools · fixture {}{}",
        cases.len(),
        if runs > 1 {
            format!(" × {runs} runs")
        } else {
            String::new()
        },
        prepared.model,
        prepared.provider_name,
        prepared.agent.registry().len(),
        fixture.display(),
        if with_rules {
            " · learned rules INJECTED (A/B treatment arm)"
        } else {
            ""
        }
    );
    // Repeated runs only measure reliability if they are actually independent
    // samples. A pinned seed replays token-for-token when requests run one at
    // a time; only concurrent batching perturbs it. Warn rather than guess —
    // whether to unpin is the caller's call.
    if runs > 1 && args.concurrency == 1 {
        if let Ok((_, pc)) = prepared.config.provider(Some(&prepared.provider_name)) {
            if pc.seed.is_some() {
                eprintln!(
                    "mecha: --runs {runs} at --concurrency 1 with a pinned seed: identical \
                     sequential requests repeat token-for-token, so this may be one sample \
                     counted {runs} times. Raise --concurrency or unset `seed`."
                );
            }
        }
    }
    if sandboxed > 0 {
        eprintln!(
            "  {sandboxed} sandboxed case(s) staged under {}",
            sandbox_root.display()
        );
    }
    if let Some(j) = &judge {
        eprintln!("  judge: {}", j.model());
    }

    // A verify command is a test run, not a tool call, so it gets its own
    // ceiling rather than borrowing the one meant for the agent's shell.
    let verify_timeout =
        std::time::Duration::from_secs(prepared.config.tools.shell_timeout_secs.max(120));

    let started = std::time::Instant::now();
    let total = items.len();
    let mut done = 0usize;

    let results = mecha_core::batch::run_with(
        &prepared.agent,
        items,
        args.concurrency,
        |item| contexts.get(&item.id).cloned(),
        |result| {
            done += 1;
            eprint!("\r  {done}/{total} ");
            let _ = std::io::Write::flush(&mut std::io::stderr());
            let _ = result;
        },
    )
    .await;
    eprintln!();

    // Results come back in completion order; grade against the matching case.
    let mut graded: Vec<GradedCase> = Vec::new();
    for result in &results {
        let Some(&(case_idx, run)) = item_of.get(&result.id) else {
            continue;
        };
        let case = &cases[case_idx];
        let mut g = grade(case, result);
        g.run = run;

        // What the run left behind, checked before anything a model says about
        // it. For a codegen case this is the only check that isn't hearsay.
        if let Some(command) = &case.expect.verify {
            // `validate` guarantees a verify case is sandboxed, so a missing
            // context is a bug here rather than a case-file mistake. Fail the
            // check saying so — falling back to the shared fixture would run
            // the command against the very directory sandboxing protects.
            g.add_check(match contexts.get(&result.id) {
                Some(cx) => {
                    mecha_core::eval::verify_workspace(command, &cx.tools.workspace, verify_timeout)
                        .await
                }
                None => mecha_core::eval::Check {
                    name: "verify".into(),
                    passed: false,
                    detail: "no staged workspace for this case (internal error)".into(),
                },
            });
        }

        // The judge runs after the deterministic checks, one case at a time.
        // Sequential on purpose: it is a second model on the same hardware, and
        // racing it against nothing buys nothing.
        if let Some(judge) = &judge {
            if let Some(check) = judge.check(case, &result.text).await {
                g.add_check(check);
            }
        }
        graded.push(g);
    }
    // Report in case-file order (then run order) so two runs read the same way.
    graded.sort_by_key(|g| {
        (
            cases
                .iter()
                .position(|c| c.id == g.id)
                .unwrap_or(usize::MAX),
            g.run,
        )
    });

    let scorecard = Scorecard::of(
        &graded,
        prepared.model.clone(),
        prepared.provider_name.clone(),
        started.elapsed().as_millis() as u64,
    );

    if sandbox_root.exists() {
        if args.keep_workspaces {
            eprintln!("staged workspaces kept in {}", sandbox_root.display());
        } else if let Err(e) = std::fs::remove_dir_all(&sandbox_root) {
            // Leftover temp directories are untidy, not wrong. Say so and
            // carry on rather than failing a run that already has its answer.
            eprintln!("mecha: could not clean up {}: {e}", sandbox_root.display());
        }
    }

    Ok((scorecard, graded))
}

/// Run the set rules-free and rules-on and report the per-case flips.
///
/// Case-level pass means pass^k under `--runs`, per arm — reliability flips
/// count, not lucky single runs. Exit code is always success on a completed
/// measurement: the delta is a finding, not a gate.
async fn ab_rules(
    global: &GlobalOpts,
    args: &Args,
    cases: &[EvalCase],
    fixture: &Path,
) -> Result<()> {
    let has_rules = mecha_core::learning::LearningStore::open_existing_default()
        .and_then(|s| {
            s.rules_prompt_block_for(mecha_core::learning::RUN_DOMAINS)
                .ok()
                .flatten()
        })
        .is_some();
    anyhow::ensure!(
        has_rules,
        "--ab-rules: the learning store has no rules to measure"
    );
    let treatment = mecha_core::experiment::Arm {
        preset: Some(mecha_core::experiment::Preset::Bare),
        levers_on: vec!["learned_rules".into()],
        prediction: Some(mecha_core::experiment::Prediction {
            metric: mecha_core::experiment::ExpMetric::Failure,
            rationale: "--ab-rules: this machine's learned rules, bare otherwise".into(),
        }),
        ..Default::default()
    };
    ab_experiment(
        global,
        args,
        cases,
        fixture,
        "rules",
        treatment,
        "with this machine's learned rules",
        true,
    )
    .await
}

/// Both A/B flags, as the two-arm experiment they are (`docs/EXPERIMENT-DESIGN.md`,
/// the owner's ruling of 2026-09-04 that eval converges into `exp`): the
/// control is `bare`, the treatment is `bare` plus one delta, the design is
/// written to the experiment store *before* either arm runs, each case is
/// filed as a trial — scored pass^k over `--runs`, one pair per case, as
/// the old pairing scored it — and the verdict comes from
/// `experiment::judge`, with the holdout drawn by the manifest's seed
/// (derived from the delta, so a rerun holds out the same cases, which is
/// the property the hash-by-id holdout used to give). The arms still run
/// in-process through `run_arm`, on eval's forcings; what changed is that
/// the design and the verdict are on the record like any other
/// experiment's, and `mecha exp judge <name>` re-derives the verdict.
#[allow(clippy::too_many_arguments)]
async fn ab_experiment(
    global: &GlobalOpts,
    args: &Args,
    cases: &[EvalCase],
    fixture: &Path,
    kind: &str,
    treatment: mecha_core::experiment::Arm,
    label: &str,
    with_rules: bool,
) -> Result<()> {
    use mecha_core::candidate::Disposition;
    use mecha_core::experiment::{ExperimentStore, Manifest, Tasks, Trial};

    anyhow::ensure!(
        args.holdout_in >= 2,
        "--holdout-in must be at least 2, or every episode is held out and \
         nothing selects"
    );
    // The record must say what ran. `--mcp` puts the MCP lever on in both
    // arms, and the manifest says so through the shared `levers_on`;
    // `--mcp-file` adds servers no lever can name, so an A/B under it is
    // refused rather than filed as bare (found on review).
    anyhow::ensure!(
        args.mcp_file.is_none(),
        "--mcp-file cannot be recorded on an A/B's design (no lever names a fixture server); run the A/B without it"
    );
    let shared: Vec<String> = if args.mcp {
        vec!["mcp".into()]
    } else {
        Vec::new()
    };
    // And the knobs both arms inherit from this machine and the flags —
    // the four `OverrideKey`s reach a run from `config.toml` and
    // `GlobalOpts`, and `run_arm` carries them into both arms verbatim —
    // go on both records, or a control run at `--max-turns 60` is filed
    // as the default and hashes like one (found on review).
    let shared_overrides = effective_overrides(global, "the A/B's design")?;
    let name = ab_name(kind, chrono::Utc::now());
    let mut manifest = Manifest::two_arm(
        &name,
        "treatment",
        treatment,
        Tasks {
            cases: args.cases.clone(),
            fixture: fixture.to_path_buf(),
            ids: cases.iter().map(|c| c.id.clone()).collect(),
            tags: Vec::new(),
        },
        args.holdout_in,
        1,
        &shared,
        &shared_overrides,
    )?;
    // The one condition no lever can name: eval lifts the operator's
    // approval rules in both arms (`force_reproducible`), while `bare` on a
    // manifest means rules *on* and `Arm::resolve_levers` refuses the
    // name. Said in the one field eval sets, so a later `mecha exp run` of
    // this design — which would keep the rules — is not mistaken for the
    // same condition (found on review).
    manifest.description = format!(
        "mecha eval A/B ({kind}): {label}; {} run(s) per case, scored pass^k, one pair per case; \
         approval rules lifted in both arms (eval's fixture forcing, not expressible as a lever)",
        args.runs
    );
    let store = ExperimentStore::open_default(&name)?;
    store.create_manifest(&manifest)?;
    eprintln!(
        "recorded as experiment `{name}` ({})",
        store.root().display()
    );

    // **The record drives both arms.** Each arm runs with exactly the
    // overrides its manifest row carries — the shared knobs on both, the
    // delta on the treatment — so what was measured is what was written,
    // whatever this machine's config says; the first cut applied the shared
    // knobs to arm B only and left arm A on the machine's values, a confound
    // the manifest recorded as identical (found on review).
    let control_overrides = manifest.arms["bare"].overrides.clone();
    let overrides = manifest.arms["treatment"].overrides.clone();
    eprintln!("── arm A: bare ──");
    let (a_card, a_graded) =
        run_arm(global, args, cases, fixture, false, &control_overrides, "a").await?;
    eprintln!("── arm B: {label} ──");
    let (b_card, b_graded) =
        run_arm(global, args, cases, fixture, with_rules, &overrides, "b").await?;

    let task_ids: Vec<String> = cases.iter().map(|c| c.id.clone()).collect();
    let planned = manifest.trials(&task_ids, &a_card.provider, &a_card.model);
    let mut trials = Vec::new();
    for t in &planned {
        let graded: Vec<&GradedCase> = if t.arm == "bare" {
            &a_graded
        } else {
            &b_graded
        }
        .iter()
        .filter(|g| g.id == t.task)
        .collect();
        if graded.is_empty() {
            // Ran in neither arm or only one: missing is missing, not a tie.
            continue;
        }
        trials.push(trial_of(t, &graded));
    }
    // Both arms have run by here, so a row that fails to write must not
    // take the verdict and `--out` with it: the rows are the durable record,
    // the printed verdict is what the operator waited an hour for (found on
    // review). Failures are counted and said.
    let mut unsaved = 0usize;
    for t in &trials {
        if let Err(e) = store.save_trial(t) {
            unsaved += 1;
            eprintln!("mecha eval: trial `{}` could not be written: {e:#}", t.id);
        }
    }
    if unsaved > 0 {
        eprintln!(
            "mecha eval: {unsaved} trial row(s) not on the store; the verdict below is from memory and `mecha exp judge {name}` will not reproduce it"
        );
    }
    let verdicts = mecha_core::experiment::judge(&manifest, &trials, &[]);
    let verdict = verdicts
        .into_iter()
        .find(|v| v.arm == "treatment")
        .context("the treatment arm produced no verdict")?;
    let judgement = &verdict.judgement;

    let passed = |trials: &[Trial], arm: &str, task: &str| {
        trials
            .iter()
            .find(|t| t.arm == arm && t.task == task)
            .and_then(|t| t.passed)
    };
    println!("\n── {kind} A/B ──");
    println!("arm A (bare):  {}/{} cases", a_card.passed, a_card.total);
    println!("arm B ({label}):  {}/{} cases", b_card.passed, b_card.total);
    let mut flips = Vec::new();
    for case in cases {
        let (was, now) = (
            passed(&trials, "bare", &case.id),
            passed(&trials, "treatment", &case.id),
        );
        if let (Some(was), Some(now)) = (was, now) {
            if was != now {
                println!(
                    "  {}: {}",
                    if now { "IMPROVED" } else { "REGRESSED" },
                    case.id
                );
                flips.push(serde_json::json!({ "id": case.id, "was": was, "now": now }));
            }
        }
    }
    println!(
        "\nselection  {}+ {}- {}=    holdout  {}+ {}- {}=    ({} pairs)",
        judgement.selection.wins,
        judgement.selection.losses,
        judgement.selection.ties,
        judgement.holdout.wins,
        judgement.holdout.losses,
        judgement.holdout.ties,
        verdict.pairs,
    );
    println!(
        "work       {} tool calls → {}",
        judgement.work_baseline, judgement.work_candidate
    );
    match &judgement.disposition {
        Disposition::Accept => println!(
            "\nverdict: BETTER — beat the original on the selection slice and held on the \
             holdout"
        ),
        Disposition::Propose(why) => println!("\nverdict: READ IT — {why}"),
        Disposition::Reject(why) => println!("\nverdict: NO — {why}"),
    }
    println!(
        "\njudge-graded flips are a prompt to read the answers, not a verdict; this is one \
         sample of a non-deterministic measurement. `mecha exp judge {name}` re-derives it."
    );

    if let Some(path) = &args.out {
        let report = |scorecard: &Scorecard, graded: &[GradedCase]| Report {
            experiment: None,
            scorecard: scorecard.clone(),
            cases: graded
                .iter()
                .map(|g| serde_json::to_value(g).unwrap_or(serde_json::Value::Null))
                .collect(),
        };
        let out = serde_json::json!({
            "experiment": name,
            "ab_rules": with_rules,
            "ab_config": args.ab_config,
            "arm_b_overrides": overrides,
            "arm_a_overrides": control_overrides,
            "holdout_in": args.holdout_in,
            "judgement": judgement,
            "pairs": verdict.pairs,
            "arm_a": report(&a_card, &a_graded),
            "arm_b": report(&b_card, &b_graded),
            "flips": flips,
        });
        std::fs::write(path, serde_json::to_string_pretty(&out)?)
            .with_context(|| format!("writing {}", path.display()))?;
        eprintln!("\nwrote {}", path.display());
    }
    Ok(())
}

/// The four override knobs as both arms actually run them, spelled as
/// `KEY=VALUE` so they land on the manifest through the same parser an
/// arm's own overrides go through. **Loaded the way the arms load**:
/// `prepare_tools` reads the config against the working directory (or the
/// global file alone under `--global-config-only`) with the flags on top,
/// and the first cut read it against the fixture — a different file
/// whenever either end carried a `mecha.toml`, and the value it recorded
/// was then applied to one arm only (found on review). A knob whose
/// effective value the closed set will not accept (`compact_at_tokens`
/// under its floor is legal config) is dropped with a warning rather than
/// refusing the A/B: it is being recorded, not proposed. An unset knob is
/// not recorded — there is nothing to write.
/// `design` names the record the knob is missing from — "the A/B's
/// design" or "the measurement's design" — because the one line that says
/// a knob is absent from a condition hash must name the right record
/// (found on review).
fn effective_overrides(global: &GlobalOpts, design: &str) -> Result<Vec<String>> {
    let cfg = if global.global_config_only {
        mecha_core::config::Config::load_global()?
    } else {
        let cwd = std::env::current_dir().context("cannot determine the working directory")?;
        mecha_core::config::Config::load(&cwd)?
    };
    let mut candidates = Vec::new();
    let max_turns = global.max_turns.unwrap_or(cfg.agent.max_turns);
    candidates.push(format!("max_turns={max_turns}"));
    if let Some(n) = global.compact_at.or(cfg.agent.compact_at_tokens) {
        candidates.push(format!("compact_at_tokens={n}"));
    }
    if let Some(n) = global.max_output_tokens.or(cfg.agent.max_output_tokens) {
        candidates.push(format!("max_output_tokens={n}"));
    }
    if let Some(e) = global.effort.or(cfg.agent.effort) {
        candidates.push(format!("effort={}", e.as_str()));
    }
    let mut out = Vec::new();
    for spec in candidates {
        match mecha_core::harness::parse_change(&spec) {
            Ok(_) => out.push(spec),
            Err(e) => eprintln!(
                "mecha eval: this machine's effective `{spec}` is not recorded on {design} ({e:#}); the run still uses it"
            ),
        }
    }
    Ok(out)
}

/// A plain eval's record: a one-arm manifest, written before the arm runs.
/// The arm is `bare` plus whatever eval opts back in (`--mcp`), carrying
/// the provider and model that ran and the knobs both a scorecard and an
/// A/B inherit from this machine. `--runs k` is the manifest's
/// `repetitions`: each run of each case is its own trial row, so k rows
/// share a condition hash and differ by repetition — the hash's contract
/// ("the same hash was configured identically") holds, where one pass^k
/// row per case would have carried the hash of a single run (found on
/// review). An eval whose tool surface no lever can name is not recorded
/// at all — `Ok(None)`, distinct from a store that failed — see
/// [`measurement_skip`].
fn record_measurement(
    global: &GlobalOpts,
    args: &Args,
    cases: &[EvalCase],
    fixture: &Path,
) -> Result<
    Option<(
        mecha_core::experiment::ExperimentStore,
        mecha_core::experiment::Manifest,
    )>,
> {
    use mecha_core::experiment::{ExperimentStore, Manifest, Tasks};
    if measurement_skip(args.mcp_file.is_some(), args.no_ask_user).is_some() {
        return Ok(None);
    }
    let cfg = if global.global_config_only {
        mecha_core::config::Config::load_global()?
    } else {
        let cwd = std::env::current_dir().context("cannot determine the working directory")?;
        mecha_core::config::Config::load(&cwd)?
    };
    let arm = measurement_arm(global, args.mcp, &cfg)?;
    let name = eval_name(chrono::Utc::now());
    let mut manifest = Manifest::one_arm(
        &name,
        "bare",
        arm,
        Tasks {
            cases: args.cases.clone(),
            fixture: fixture.to_path_buf(),
            ids: cases.iter().map(|c| c.id.clone()).collect(),
            tags: Vec::new(),
        },
        args.runs,
    )?;
    manifest.description = format!(
        "mecha eval scorecard; {} run(s) per case, one trial row per run; approval rules lifted (eval's fixture forcing, not expressible as a lever)",
        args.runs,
    );
    let store = ExperimentStore::open_default(&name)?;
    store.create_manifest(&manifest)?;
    Ok(Some((store, manifest)))
}

/// Why a plain eval is *not* recorded, when it is not. A trial row's
/// `condition_hash` sees levers, overrides, provider, model and seed, and
/// its contract is "the same hash was configured identically" — so an
/// eval whose tool surface differs from bare in a way no lever names
/// (fixture servers under `--mcp-file`; `ask_user` withheld under
/// `--no-ask-user`, where a bare run has it present and declining) would
/// write rows carrying a bare eval's hash. The A/B refuses `--mcp-file`
/// outright and carries `--no-ask-user` on both arms, where it cancels;
/// the measurement's rows are read across experiments, so both skip. A
/// lever for the withheld tool is the truer record and waits on the
/// principal, which is where `ask_user` first gets an answerer (found on
/// review, twice).
fn measurement_skip(mcp_file: bool, no_ask_user: bool) -> Option<&'static str> {
    if mcp_file {
        return Some(
            "--mcp-file adds fixture servers no lever names, and the record's condition hash could not tell this eval from a bare one",
        );
    }
    if no_ask_user {
        return Some(
            "--no-ask-user withholds a tool no lever names, and the record's condition hash could not tell this eval from a bare one",
        );
    }
    None
}

/// The arm a scorecard measured. It names the provider and model that
/// *ran*, resolved the way `setup::build` resolves them — the flag, else
/// the provider's configured `model`, else the provider's own default —
/// off the same config the run loads. Two earlier cuts stopped short (the
/// flags alone, then the flags and the config) and each left the manifest
/// silent about the one fact a scorecard exists to hold (found on review,
/// twice).
fn measurement_arm(
    global: &GlobalOpts,
    mcp: bool,
    cfg: &mecha_core::config::Config,
) -> Result<mecha_core::experiment::Arm> {
    use mecha_core::experiment::{Arm, Preset};
    let (provider_name, provider_cfg) = cfg.provider(global.provider.as_deref())?;
    let model = match global.model.clone().or_else(|| provider_cfg.model.clone()) {
        Some(m) => m,
        None => mecha_core::provider::build(provider_cfg)?
            .default_model()
            .to_string(),
    };
    Ok(Arm {
        preset: Some(Preset::Bare),
        levers_on: if mcp { vec!["mcp".into()] } else { Vec::new() },
        overrides: effective_overrides(global, "the measurement's design")?,
        provider: Some(provider_name),
        model: Some(model),
        ..Arm::default()
    })
}

/// The experiment a plain eval records as; same shape and same test as
/// `ab_name`, because a bad stamp here is *quieter* than the A/B's was —
/// `record_measurement`'s error is printed and the scorecard runs on, so
/// every eval would silently record nothing.
fn eval_name(now: chrono::DateTime<chrono::Utc>) -> String {
    format!("eval-{}", now.format("%Y%m%d-%H%M%S"))
}

/// The experiment an A/B records as. A producer name, so lowercase, digits,
/// `-` and `_` only — the first cut stamped `%Y%m%dT%H%M%S`, whose `T`
/// failed `valid_producer` and killed every A/B before an arm ran (found
/// on review); the test below validates the exact string this builds.
fn ab_name(kind: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    format!("eval-ab-{kind}-{}", now.format("%Y%m%d-%H%M%S"))
}

/// One case's graded runs as a trial row: pass^k over the runs, the checks
/// concatenated, the stats folded — the same pair the old A/B scored, on the
/// experiment store's row.
fn trial_of(
    planned: &mecha_core::experiment::Trial,
    graded: &[&GradedCase],
) -> mecha_core::experiment::Trial {
    let passed = graded.iter().all(|g| g.passed);
    let checks = graded
        .iter()
        .flat_map(|g| g.checks.iter().cloned())
        .collect();
    // `RunStats::fold` is written for one session's *sequential* runs, so
    // its `stop_cause`, `exhausted` and `ended_on_failed_call` are
    // last-wins. These rows are independent replicates of one case, and
    // "the last replicate's" is not what those field names say — so on a
    // multi-run row the three are left unmeasured (`None` / their
    // defaults) rather than borrowed from run k, and `duration_secs` is
    // the total across replicates (found on review). A single run's row is
    // that run's, in full.
    let replicates = graded.len();
    let stats =
        mecha_core::session::RunStats::fold(graded.iter().map(|g| mecha_core::session::RunStats {
            turns: g.turns,
            usage: g.usage.clone(),
            tool_calls: g.tools_called.len() as u32,
            // `GradedCase` splits what `RunStats` folds, on opposite axes:
            // its `tool_errors` is `is_error && !unknown` (denials in,
            // invented tools apart), the run record's is
            // `unknown || (is_error && !denied)`. Re-derived here so the
            // row means what every row on the store means (found on
            // review).
            tool_errors: g.tool_errors.saturating_sub(g.tool_denied) + g.unknown_tools,
            tool_denied: g.tool_denied,
            malformed_tool_args: g.malformed_tool_args,
            duration_secs: Some(g.elapsed_ms as f64 / 1000.0),
            // Carried, not defaulted: a zero here would read as measured.
            compactions: g.compactions,
            ended_on_failed_call: replicates == 1 && g.ended_on_failed_call,
            blocked_sends: g.blocked_sends,
            stop_cause: if replicates == 1 { g.stop_cause } else { None },
            usage_complete: g.usage_complete,
            ..Default::default()
        }));
    mecha_core::experiment::Trial::finished(planned, passed, checks, stats)
}

/// Build the per-item contexts: a private staged workspace for sandboxed cases,
/// a raised turn budget for cases that ask for one, or both.
///
/// Keyed by *item* id, not case id: under `--runs` a sandboxed case appears k
/// times, and two runs sharing one workspace would see each other's writes —
/// the exact contamination the sandbox exists to prevent, and it would also
/// make the k samples dependent, which is what pass^k assumes they are not.
///
/// Items needing nothing get no entry and run on the agent's own context.
fn prepare_contexts(
    items: &[(&str, &EvalCase)],
    fixture: &Path,
    root: &Path,
    prepared: &setup::Prepared,
) -> Result<HashMap<String, Arc<RunContext>>> {
    let mut contexts = HashMap::new();

    for (id, case) in items
        .iter()
        .filter(|(_, c)| c.sandbox || c.max_turns.is_some() || c.compact_at_tokens.is_some())
    {
        let base = prepared.agent.context();

        let cx = if case.sandbox {
            let dir = root.join(safe_dir_name(id));
            stage_workspace(fixture, &dir)
                .with_context(|| format!("staging a workspace for `{id}`"))?;

            // Canonicalize now: the path jail compares canonical paths, and a
            // temp directory reached through a symlink would otherwise make the
            // jail refuse every path the case touches.
            let dir = dir
                .canonicalize()
                .with_context(|| format!("resolving {}", dir.display()))?;

            base.sandboxed(
                dir,
                Arc::new(ModeApprover {
                    mode: PermissionMode::Allow,
                }),
            )
        } else {
            base.as_ref().clone()
        };

        let budget = Budget {
            max_turns: case.max_turns,
            ..Budget::default()
        };
        let cx = cx
            .with_budget(budget)
            .with_compact_at(case.compact_at_tokens);
        contexts.insert(id.to_string(), Arc::new(cx));
    }
    Ok(contexts)
}

/// Case ids become directory names, and a `/` in one would silently stage the
/// workspace somewhere other than where cleanup looks for it.
fn safe_dir_name(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Build the judge, if any case needs one.
fn build_judge(
    args: &Args,
    prepared: &setup::Prepared,
    cases: &[EvalCase],
) -> Result<Option<Judge>> {
    let needed: Vec<&str> = cases
        .iter()
        .filter(|c| c.expect.judge.is_some())
        .map(|c| c.id.as_str())
        .collect();
    if needed.is_empty() {
        return Ok(None);
    }

    let name = args.judge_provider.as_deref();
    let (provider_name, provider_cfg) = prepared.config.provider(name).with_context(|| {
        format!(
            "{} case(s) need a judge ({}), but no usable provider was found",
            needed.len(),
            needed.join(", ")
        )
    })?;
    let provider = mecha_core::provider::build(provider_cfg)?;

    let model = args
        .judge_model
        .clone()
        .or_else(|| provider_cfg.model.clone());
    let judge = Judge::new(provider, model);

    // Not fatal, but it does undermine the result: a model asked whether its
    // own answer was good is not an independent check.
    if judge.model() == prepared.model && provider_name == prepared.provider_name {
        eprintln!(
            "mecha: the judge is the model under test ({}). Its verdicts are not \
             independent — pass --judge-model or --judge-provider.",
            judge.model()
        );
    }
    Ok(Some(judge))
}

/// The shape of an `--mcp-file`: just `[[mcp]]` tables, nothing else. Denying
/// unknown fields means a whole config file pasted here fails loudly instead
/// of silently contributing only its servers.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct McpFile {
    #[serde(default)]
    mcp: Vec<mecha_core::config::McpServerConfig>,
}

/// Parse an `--mcp-file` and resolve its relative paths.
///
/// A path in `command` or `args` that names an existing file *relative to the
/// TOML's own directory* becomes absolute, so the file travels with the case
/// set and works from any CWD. Anything else — `python3`, `--persona`, a flag
/// value — is left alone; existence on disk is what distinguishes a path from
/// an argument that merely looks like one.
fn load_mcp_file(path: &Path) -> Result<Vec<mecha_core::config::McpServerConfig>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let file: McpFile = toml::from_str(&text)
        .with_context(|| format!("{} is not an MCP server file", path.display()))?;
    anyhow::ensure!(
        !file.mcp.is_empty(),
        "{} names no MCP servers",
        path.display()
    );

    // Canonicalized, because the joined path must survive a change of
    // working directory: fixture servers are spawned in the run's WORKSPACE
    // (like every MCP server since servers started in the workspace), and a
    // path left relative to the invocation directory resolves there instead —
    // `eval/fixtures/server.py` became `<workspace>/eval/fixtures/server.py`
    // and every handshake failed.
    let base = path
        .parent()
        .unwrap_or(Path::new("."))
        .canonicalize()
        .with_context(|| format!("resolving the directory of {}", path.display()))?;
    let resolve = |s: String| -> String {
        let joined = base.join(&s);
        if Path::new(&s).is_relative() && joined.is_file() {
            joined.to_string_lossy().into_owned()
        } else {
            s
        }
    };

    Ok(file
        .mcp
        .into_iter()
        .map(|mut server| {
            server.command = resolve(server.command);
            server.args = server.args.into_iter().map(resolve).collect();
            server
        })
        .collect())
}

pub(crate) fn load_cases(path: &Path, tags: &[String]) -> Result<Vec<EvalCase>> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;

    let mut cases = Vec::new();
    for (i, line) in std::io::BufReader::new(file).lines().enumerate() {
        let line = line?;
        let line = line.trim();
        // `//` lets a case file carry section headers.
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let case: EvalCase = serde_json::from_str(line)
            .with_context(|| format!("{}:{}: not a valid eval case", path.display(), i + 1))?;
        case.validate()
            .with_context(|| format!("{}:{}", path.display(), i + 1))?;
        if tags.is_empty() || case.tags.iter().any(|t| tags.contains(t)) {
            cases.push(case);
        }
    }
    Ok(cases)
}

fn print_scorecard(card: &Scorecard, graded: &[GradedCase], show_failures: bool) {
    println!("\n{}  ({})", card.model, card.provider);
    println!("{}", "─".repeat(60));

    if card.runs_per_case > 1 {
        // The gap between these two lines is the model's unreliability, which
        // is the thing k runs were bought to measure.
        let k = card.runs_per_case;
        println!(
            "  {:<19} {}/{}  ({:.0}%)",
            format!("pass^{k} (all runs)"),
            card.passed,
            card.total,
            card.pass_rate() * 100.0
        );
        if let Some(any) = card.passed_any {
            println!(
                "  {:<19} {}/{}",
                format!("pass@{k} (any run)"),
                any,
                card.total
            );
        }
    } else {
        println!(
            "  cases passed        {}/{}  ({:.0}%)",
            card.passed,
            card.total,
            card.pass_rate() * 100.0
        );
    }
    println!("  checks passed       {:.0}%", card.check_pass_rate * 100.0);

    // The reliability block: these are what disqualify a model for loop use,
    // regardless of how well it scores on the cases themselves.
    println!("\n  malformed arguments {}", card.malformed_tool_args);
    println!("  invented tools      {}", card.unknown_tools);
    println!("  tool errors         {}", card.tool_errors);
    println!("  runs errored        {}", card.runs_errored);

    println!("\n  mean turns          {:.1}", card.mean_turns);
    println!(
        "  median latency      {:.1}s",
        card.median_latency_ms as f64 / 1000.0
    );
    println!(
        "  tokens              {} in / {} out",
        card.total_usage.total_input(),
        card.total_usage.output_tokens
    );
    println!(
        "  wall clock          {:.1}s",
        card.wall_clock_ms as f64 / 1000.0
    );

    if !card.by_tag.is_empty() {
        println!("\n  by tag");
        for tag in &card.by_tag {
            let bar = if tag.total == 0 {
                String::new()
            } else {
                let filled = (tag.passed * 10).div_ceil(tag.total.max(1));
                format!("{}{}", "█".repeat(filled), "·".repeat(10 - filled))
            };
            let any = tag
                .passed_any
                .filter(|a| *a != tag.passed)
                .map(|a| format!("  (any {a})"))
                .unwrap_or_default();
            println!(
                "    {:<18} {} {}/{}{}",
                tag.tag, bar, tag.passed, tag.total, any
            );
        }
    }

    let failed: Vec<_> = graded.iter().filter(|g| !g.passed).collect();
    if !failed.is_empty() {
        println!("\n  failed");
        for case in &failed {
            let reasons: Vec<&str> = case
                .checks
                .iter()
                .filter(|c| !c.passed)
                .map(|c| c.name.as_str())
                .collect();
            let label = if card.runs_per_case > 1 {
                format!("{} (run {})", case.id, case.run)
            } else {
                case.id.clone()
            };
            println!("    {label:<24} {}", reasons.join(", "));
            if show_failures {
                for check in case.checks.iter().filter(|c| !c.passed) {
                    if !check.detail.is_empty() {
                        println!("      {}: {}", check.name, check.detail);
                    }
                }
            }
        }
    }
    println!();
}

/// Side-by-side comparison — the actual output of a bake-off.
fn compare(paths: &[PathBuf]) -> Result<()> {
    let mut cards = Vec::new();
    for path in paths {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let report: Report = serde_json::from_str(&text)
            .with_context(|| format!("{} is not a mecha eval report", path.display()))?;
        cards.push(report.scorecard);
    }

    let w = cards
        .iter()
        .map(|c| c.model.len())
        .max()
        .unwrap_or(10)
        .max(10);
    let row = |label: &str, values: Vec<String>| {
        print!("  {label:<22}");
        for (card, value) in cards.iter().zip(values) {
            print!("{:>width$}  ", value, width = w.max(card.model.len()));
        }
        println!();
    };

    print!("  {:<22}", "");
    for card in &cards {
        print!("{:>width$}  ", card.model, width = w.max(card.model.len()));
    }
    println!("\n  {}", "─".repeat(24 + cards.len() * (w + 2)));

    row(
        "cases passed",
        cards
            .iter()
            .map(|c| format!("{}/{}", c.passed, c.total))
            .collect(),
    );
    // Only when some card is multi-run: `cases passed` above is then pass^k,
    // and these two rows are what make that legible — and warn that a
    // single-run card beside it is not measuring the same thing.
    if cards.iter().any(|c| c.runs_per_case > 1) {
        row(
            "runs/case",
            cards.iter().map(|c| c.runs_per_case.to_string()).collect(),
        );
        row(
            "any-run pass",
            cards
                .iter()
                .map(|c| {
                    c.passed_any
                        .map(|a| format!("{}/{}", a, c.total))
                        .unwrap_or_else(|| "—".into())
                })
                .collect(),
        );
    }
    row(
        "pass rate",
        cards
            .iter()
            .map(|c| format!("{:.0}%", c.pass_rate() * 100.0))
            .collect(),
    );
    row(
        "checks",
        cards
            .iter()
            .map(|c| format!("{:.0}%", c.check_pass_rate * 100.0))
            .collect(),
    );
    row(
        "malformed args",
        cards
            .iter()
            .map(|c| c.malformed_tool_args.to_string())
            .collect(),
    );
    row(
        "invented tools",
        cards.iter().map(|c| c.unknown_tools.to_string()).collect(),
    );
    row(
        "mean turns",
        cards
            .iter()
            .map(|c| format!("{:.1}", c.mean_turns))
            .collect(),
    );
    row(
        "median latency",
        cards
            .iter()
            .map(|c| format!("{:.1}s", c.median_latency_ms as f64 / 1000.0))
            .collect(),
    );
    row(
        "output tokens",
        cards
            .iter()
            .map(|c| c.total_usage.output_tokens.to_string())
            .collect(),
    );

    // Per-tag rows only where the tag exists in every report.
    let shared: Vec<String> = cards
        .first()
        .map(|c| {
            c.by_tag
                .iter()
                .map(|t| t.tag.clone())
                .filter(|tag| cards.iter().all(|c| c.by_tag.iter().any(|t| &t.tag == tag)))
                .collect()
        })
        .unwrap_or_default();

    if !shared.is_empty() {
        println!();
        for tag in shared {
            row(
                &tag,
                cards
                    .iter()
                    .map(|c| {
                        c.by_tag
                            .iter()
                            .find(|t| t.tag == tag)
                            .map(|t| format!("{}/{}", t.passed, t.total))
                            .unwrap_or_default()
                    })
                    .collect(),
            );
        }
    }
    println!();
    Ok(())
}

/// `--ab-config`: the case set run twice, and the difference judged.
///
/// The content-sensitive arm of the candidate gate. A case's *cost* is
/// failing it, so a pass is a win and every guardrail in `candidate.rs`
/// applies unchanged — the holdout split, the work floor, and the rule that
/// thin evidence proposes rather than rejects.
///
/// Like `--ab-rules`, neither arm is written as an ordinary scorecard: a
/// scorecard produced under a candidate override is not comparable to one
/// produced without it, and filing it as though it were is how an A/B
/// contaminates a series.
async fn ab_config(
    global: &GlobalOpts,
    args: &Args,
    cases: &[EvalCase],
    fixture: &Path,
) -> Result<()> {
    for spec in &args.ab_config {
        apply_override(&mut global.clone(), spec)?;
    }
    let treatment = mecha_core::experiment::Arm {
        preset: Some(mecha_core::experiment::Preset::Bare),
        overrides: args.ab_config.clone(),
        prediction: Some(mecha_core::experiment::Prediction {
            metric: mecha_core::experiment::ExpMetric::Failure,
            rationale: format!("--ab-config {}", args.ab_config.join(" ")),
        }),
        ..Default::default()
    };
    let label = args.ab_config.join(", ");
    ab_experiment(
        global, args, cases, fixture, "config", treatment, &label, false,
    )
    .await
}

/// Everything a scorecard must not depend on, asserted in one place.
///
/// **The list is the point.** Each of these was added for the same reason —
/// a scorecard that varies with this machine is not comparable to yesterday's
/// or anyone else's — and each was added on its own, in prose, at the point
/// it occurred to somebody. `compact` was the one that got missed: it is
/// registered from whether local config gives the run a compaction threshold,
/// and it sits at the front of the cached prefix, so two boxes graded
/// different prefixes and neither scorecard recorded which.
///
/// Named as a function so the answer to *what does eval force off* is one
/// thing a test can read, rather than a list nobody could see all of.
///
/// **Both `allow_*` parameters are deliberate exceptions, and they are
/// parameters rather than re-enables at the call site for a reason this
/// function itself demonstrated.** Consolidating the list flattened
/// `opts.no_learned_rules = !with_rules` — written at `run_arm`'s call
/// site — into an unconditional `true`, which left `--ab-rules` announcing
/// *"learned rules INJECTED (A/B treatment arm)"* over an arm that had none:
/// two identical arms, and every per-case flip it printed was noise. The
/// set-assertion test below did not catch it, because a test that asserts a
/// list is complete cannot notice that one member of it was supposed to be a
/// variable. A lever that lives *inside* the list cannot be lost while
/// consolidating the list, so it lives here.
///
/// Since the lever set exists it is expressed over it: the bare arm is
/// [`Lever::bare`] with the two opt-ins allowed, thrown through
/// `setup::switch_off`, so this list and `setup::levers_off` — the function
/// every front-end that writes a session record reads its
/// `RunConfig::levers_off` from — cannot name different sets; the test
/// `the_record_names_exactly_what_eval_forced` reads one through the other.
/// (Eval itself writes no session: it drives `run_in` directly, so what it
/// forced is on no record. The guarantee is that any run which *is*
/// recorded names the same absences.) Two things stay spelled here. `--mcp` opts
/// *in* by leaving the user's own `--no-mcp` alone rather than clearing it;
/// and learned rules are set in both directions, because `run_arm` builds
/// the treatment arm from the same `opts` it built the baseline from.
fn force_reproducible(opts: &mut GlobalOpts, allow_mcp: bool, allow_learned_rules: bool) {
    let mut allow = Vec::new();
    if allow_mcp {
        allow.push(Lever::Mcp);
    }
    if allow_learned_rules {
        allow.push(Lever::LearnedRules);
    }
    for lever in Lever::bare(&allow) {
        crate::setup::switch_off(opts, lever);
    }
    opts.no_learned_rules = !allow_learned_rules;
    // The one lever `bare` never throws, thrown here and nowhere else: a
    // `forbid` in this box's rules file would score a case's `shell` call
    // as `Blocked by policy:` here and not there, and eval's fixture
    // workspaces are what make lifting the operator's word defensible.
    crate::setup::switch_off(opts, Lever::ApprovalRules);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A scorecard must not vary with the machine that produced it**, and
    /// the way that breaks is one entry quietly missing from a list written in
    /// prose across forty lines. `compact` was missing: registered from local
    /// `context_window` / `compact_at_tokens`, and at the front of the cached
    /// prefix, so two differently-configured boxes graded different prefixes.
    ///
    /// Asserted as a set rather than one flag, so the next thing added to the
    /// surface has to be decided about here.
    #[test]
    fn eval_forces_off_everything_a_scorecard_must_not_depend_on() {
        let mut opts = GlobalOpts::default();
        force_reproducible(&mut opts, false, false);
        for (name, on) in [
            ("mcp", opts.no_mcp),
            ("learned rules", opts.no_learned_rules),
            ("hooks", opts.no_hooks),
            ("outbox", opts.no_outbox),
            ("fallback", opts.no_fallback),
            ("messages", opts.no_messages),
            ("skills", opts.no_skills),
            ("the charter", opts.no_charter),
            // The one that was missed. It changes the *tool list*, which is
            // the front of the cached prefix, not merely what a run may do.
            ("the compact tool", opts.no_compact_tool),
            // Off by default, but a machine's own config.toml could still
            // turn it on — a scorecard must not depend on that either.
            ("step escalation", opts.no_step_escalation),
            // Same shape: a `forbid` in this box's rules file would score a
            // case's `shell` call as `Blocked by policy:` here and not there.
            ("approval rules", opts.no_rules),
            // The two `[agent]` switches that ship *on*, missed until the
            // lever set named them: a notice in the model's context and a
            // second model call, each decided by this machine's config.
            ("boredom", opts.no_boredom),
            ("compact validation", opts.no_compact_validate),
            // The two dispositions that had no off position until the
            // second half of the lever set gave them one.
            ("predictive compaction", opts.no_predictive_compaction),
            ("carried state", opts.no_carried_state),
        ] {
            assert!(
                on,
                "eval must force {name} off, or a scorecard measures this machine"
            );
        }

        // `--mcp` is the one deliberate opt-in: the graph case set needs
        // servers in the surface, and says so.
        let mut with_mcp = GlobalOpts::default();
        force_reproducible(&mut with_mcp, true, false);
        assert!(!with_mcp.no_mcp, "--mcp is opt-in, not overridden");
        assert!(with_mcp.no_compact_tool, "and it opts into nothing else");
        assert!(with_mcp.no_step_escalation, "including this one");
    }

    /// The name an A/B records under is the name the store accepts — checked
    /// on the exact string the CLI builds, since the two-arm tests build
    /// their own names and the first cut's stamp failed the producer rule.
    #[test]
    fn the_ab_experiment_name_is_a_valid_producer_name() {
        for kind in ["config", "rules"] {
            let name = ab_name(kind, chrono::Utc::now());
            mecha_core::work::valid_producer(&name).unwrap();
            assert!(name.starts_with(&format!("eval-ab-{kind}-")));
            mecha_core::experiment::ExperimentStore::open(std::env::temp_dir(), &name).unwrap();
        }
        let name = eval_name(chrono::Utc::now());
        mecha_core::work::valid_producer(&name).unwrap();
        assert!(name.starts_with("eval-") && !name.starts_with("eval-ab-"));
        mecha_core::experiment::ExperimentStore::open(std::env::temp_dir(), &name).unwrap();
    }

    /// The measurement's arm names the model that ran through every link
    /// of `setup::build`'s chain: the flag, else the provider's configured
    /// model, else the provider's own default — never `None`.
    #[test]
    fn the_measurement_names_the_model_that_ran_even_when_nothing_named_it() {
        let mut cfg = mecha_core::config::Config::default();
        cfg.providers.insert(
            "box".into(),
            mecha_core::config::ProviderConfig {
                kind: "local".into(),
                model: None,
                api_key: Some("none".into()),
                base_url: Some("http://127.0.0.1:1/v1".into()),
                ..cfg.providers["anthropic"].clone()
            },
        );
        cfg.default_provider = "box".into();
        let builtin = mecha_core::provider::build(&cfg.providers["box"])
            .unwrap()
            .default_model()
            .to_string();
        let global = GlobalOpts::default();
        let arm = measurement_arm(&global, false, &cfg).unwrap();
        assert_eq!(arm.provider.as_deref(), Some("box"));
        assert_eq!(arm.model.as_deref(), Some(builtin.as_str()));
        assert!(arm.levers_on.is_empty());

        cfg.providers.get_mut("box").unwrap().model = Some("configured".into());
        let arm = measurement_arm(&global, true, &cfg).unwrap();
        assert_eq!(arm.model.as_deref(), Some("configured"));
        assert_eq!(arm.levers_on, vec!["mcp".to_string()]);

        let flagged = GlobalOpts {
            provider: Some("anthropic".into()),
            model: Some("flagged".into()),
            ..GlobalOpts::default()
        };
        let arm = measurement_arm(&flagged, false, &cfg).unwrap();
        assert_eq!(arm.provider.as_deref(), Some("anthropic"));
        assert_eq!(arm.model.as_deref(), Some("flagged"));
    }

    /// The two evals whose surface no lever can name are skipped, and a
    /// plain one is not.
    #[test]
    fn a_measurement_skips_exactly_the_surfaces_no_lever_names() {
        assert!(measurement_skip(false, false).is_none());
        assert!(measurement_skip(true, false)
            .unwrap()
            .contains("--mcp-file"));
        assert!(measurement_skip(false, true)
            .unwrap()
            .contains("--no-ask-user"));
        assert!(measurement_skip(true, true).unwrap().contains("--mcp-file"));
    }

    /// A plain eval under `--runs k` plans k rows per case — one per run,
    /// sharing the arm's hash and differing by repetition — and each row
    /// is that run's in full, stop cause included, where the A/B's pass^k
    /// fold leaves it unmeasured.
    #[test]
    fn a_measurement_writes_one_row_per_run() {
        use mecha_core::experiment::{Arm, Manifest, Preset, Tasks};
        let manifest = Manifest::one_arm(
            "eval-t",
            "bare",
            Arm {
                preset: Some(Preset::Bare),
                ..Arm::default()
            },
            Tasks {
                cases: "eval/cases.jsonl".into(),
                fixture: "eval/workspace".into(),
                ids: vec!["c".into()],
                tags: Vec::new(),
            },
            3,
        )
        .unwrap();
        let rows = manifest.trials(&["c".to_string()], "p", "m");
        assert_eq!(
            rows.iter().map(|t| t.repetition).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(rows
            .iter()
            .all(|t| t.condition_hash == rows[0].condition_hash));
        let run: GradedCase = serde_json::from_value(serde_json::json!({
            "id": "c", "run": 2, "passed": true, "tags": ["t"],
            "checks": [{"name": "contains", "passed": true, "detail": ""}],
            "turns": 4, "elapsed_ms": 500, "malformed_tool_args": 0,
            "unknown_tools": 0, "tool_errors": 0, "tool_denied": 0, "tools_called": ["shell"],
            "compactions": 0, "ended_on_failed_call": false, "blocked_sends": 0, "stop_cause": "completed", "usage_complete": true,
            "usage": {"input_tokens": 1, "output_tokens": 1, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0},
            "error": null, "text": "x"
        }))
        .unwrap();
        let planned = rows.iter().find(|t| t.repetition == run.run).unwrap();
        let row = trial_of(planned, std::slice::from_ref(&&run));
        assert_eq!(row.repetition, 2);
        assert_eq!(row.passed, Some(true));
        let stats = row.stats.unwrap();
        assert_eq!(
            stats.stop_cause,
            Some(mecha_core::agent::StopCause::Completed)
        );
        assert_eq!(stats.turns, 4);
    }

    /// A case's runs fold into one trial row the way the old pairing scored
    /// them: pass^k over the runs, the stats summed, the checks kept.
    #[test]
    fn a_cases_runs_fold_into_one_trial_scored_pass_k() {
        let graded = |run: u32, passed: bool, turns: u32| -> GradedCase {
            serde_json::from_value(serde_json::json!({
                "id": "c", "run": run, "passed": passed, "tags": ["t"],
                "checks": [{"name": "contains", "passed": passed, "detail": ""}],
                "turns": turns, "elapsed_ms": 500, "malformed_tool_args": 0,
                "unknown_tools": 1, "tool_errors": 2, "tool_denied": 1, "tools_called": ["shell", "fs_read"],
                "compactions": 1, "ended_on_failed_call": true, "blocked_sends": 1, "stop_cause": "completed", "usage_complete": true,
                "usage": {"input_tokens": 1, "output_tokens": 1, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0},
                "error": null, "text": "x"
            }))
            .unwrap()
        };
        let planned = mecha_core::experiment::Trial {
            id: "bare__c__r1".into(),
            arm: "bare".into(),
            task: "c".into(),
            seed: None,
            repetition: 1,
            condition_hash: "h".into(),
            status: mecha_core::experiment::TrialStatus::Pending,
            session_id: None,
            started_at: None,
            finished_at: None,
            error: None,
            passed: None,
            checks: Vec::new(),
            stats: None,
            position: None,
            lifetime: None,
        };
        let runs = [graded(1, true, 3), graded(2, false, 5)];
        let t = trial_of(&planned, &runs.iter().collect::<Vec<_>>());
        assert_eq!(
            t.passed,
            Some(false),
            "pass^k: one failed run fails the case"
        );
        assert_eq!(t.checks.len(), 2);
        let s = t.stats.unwrap();
        assert_eq!(s.turns, 8);
        assert_eq!(s.tool_calls, 4);
        // Per run: 2 graded errors, of which 1 a denial, plus 1 invented
        // tool → 2 run-record errors and 1 denial; two runs fold to 4 and 2.
        assert_eq!(s.tool_errors, 4);
        assert_eq!(s.tool_denied, 2);
        assert_eq!(s.compactions, 2, "carried, not defaulted");
        assert_eq!(s.blocked_sends, 2);
        assert!(
            !s.ended_on_failed_call,
            "unmeasured on a multi-run row, not run k's"
        );
        assert_eq!(s.stop_cause, None, "unmeasured on a multi-run row");
        let one = [graded(1, true, 3)];
        let single = trial_of(&planned, &one.iter().collect::<Vec<_>>())
            .stats
            .unwrap();
        assert!(
            single.ended_on_failed_call,
            "a single run's row is that run's"
        );
        assert_eq!(
            single.stop_cause,
            Some(mecha_core::agent::StopCause::Completed)
        );
        assert_eq!(s.duration_secs, Some(1.0));
        assert_eq!(t.status, mecha_core::experiment::TrialStatus::Done);
        let all_pass = [graded(1, true, 3), graded(2, true, 3)];
        assert_eq!(
            trial_of(&planned, &all_pass.iter().collect::<Vec<_>>()).passed,
            Some(true)
        );
    }

    /// `--ab-rules`' treatment arm must actually carry rules.
    ///
    /// The regression this fails on: consolidating the forced-off list turned
    /// `opts.no_learned_rules = !with_rules` into an unconditional `true`, so
    /// arm B ran rules-free while printing "learned rules INJECTED (A/B
    /// treatment arm)" — two identical arms, and every flip reported between
    /// them was noise. Asserted here rather than only in the set test above,
    /// because that one asserts the *baseline* and would pass just as happily
    /// against a lever that had stopped being a lever.
    #[test]
    fn the_ab_rules_treatment_arm_actually_carries_rules() {
        let mut treatment = GlobalOpts::default();
        force_reproducible(&mut treatment, false, true);
        assert!(
            !treatment.no_learned_rules,
            concat!(
                "--ab-rules' treatment arm must run with this machine's learned rules, ",
                "or the A/B measures nothing"
            )
        );
        // And it opts into nothing else — the lever is one flag wide.
        assert!(treatment.no_mcp, "the rules lever is not an MCP opt-in");
        assert!(treatment.no_skills);
        assert!(treatment.no_charter);
        assert!(treatment.no_hooks);
        assert!(treatment.no_outbox);
        assert!(treatment.no_fallback);
        assert!(treatment.no_messages);
        assert!(treatment.no_compact_tool);
        assert!(treatment.no_step_escalation);
        assert!(treatment.no_boredom);
        assert!(treatment.no_compact_validate);
        assert!(treatment.no_predictive_compaction);
        assert!(treatment.no_carried_state);
    }

    /// What eval forces and what a session record would say of the same
    /// switches must be one set, read through each other:
    /// `force_reproducible` throws switches, and `setup::levers_off` — the
    /// function behind `RunConfig::levers_off` — reads them back. Eval
    /// writes no session, so this is the only place the two meet; if they
    /// disagreed, an experiment pairing its own recorded bare arm against
    /// an eval scorecard would be comparing runs that name different
    /// absences — the whole reason the design keeps one definition.
    #[test]
    fn the_record_names_exactly_what_eval_forced() {
        // What `prepare_tools` hands `build`: this machine's defaults with
        // the flags folded in. `levers_off` reads the folded config, never
        // the flag, so the fold is part of what "eval forced" means.
        let folded = |opts: &GlobalOpts| {
            let mut cfg = mecha_core::config::Config::default();
            crate::setup::fold_agent_switches(&mut cfg.agent, opts);
            cfg
        };
        // Eval is the bare preset *plus* the operator's rules lifted — the
        // one lever `Lever::bare` refuses to throw, so it is asserted here
        // by name rather than folded into the preset.
        let bare_plus_rules = |allow: &[Lever]| {
            let mut v = Lever::bare(allow);
            v.push(Lever::ApprovalRules);
            Lever::ALL
                .into_iter()
                .filter(|l| v.contains(l))
                .collect::<Vec<_>>()
        };
        let mut bare = GlobalOpts::default();
        force_reproducible(&mut bare, false, false);
        assert_eq!(
            crate::setup::levers_off(&bare, &folded(&bare)),
            bare_plus_rules(&[]),
            "the bare arm records every lever off, the rules included"
        );
        assert_eq!(bare_plus_rules(&[]), Lever::ALL.to_vec());

        let mut with_mcp = GlobalOpts::default();
        force_reproducible(&mut with_mcp, true, false);
        assert_eq!(
            crate::setup::levers_off(&with_mcp, &folded(&with_mcp)),
            bare_plus_rules(&[Lever::Mcp])
        );

        let mut with_rules = GlobalOpts::default();
        force_reproducible(&mut with_rules, false, true);
        assert_eq!(
            crate::setup::levers_off(&with_rules, &folded(&with_rules)),
            bare_plus_rules(&[Lever::LearnedRules])
        );

        // And the other direction: a switch thrown by hand reads as off.
        for lever in Lever::ALL {
            let mut one = GlobalOpts::default();
            crate::setup::switch_off(&mut one, lever);
            assert!(
                crate::setup::levers_off(&one, &folded(&one)).contains(&lever),
                "{lever:?} thrown through switch_off must read as off"
            );
        }

        // And untouched opts record nothing off but what the config's own
        // defaults leave off — the record is the switch, never the effect,
        // so a provider without a context window changes `tools`, not this.
        let untouched = GlobalOpts::default();
        assert_eq!(
            crate::setup::levers_off(&untouched, &folded(&untouched)),
            vec![Lever::Messages, Lever::StepEscalation],
            "messaging and step escalation are the two switches that ship off"
        );
    }

    /// A scratch directory that cleans up after itself, so a failing test
    /// does not leave litter that changes what the next run's `is_file`
    /// checks see.
    ///
    /// **Canonicalized, because `temp_dir()` is not the path the code under
    /// test will produce.** On macOS it answers `/var/folders/…`, which is a
    /// symlink to `/private/var/folders/…` — so a fixture that built its
    /// expectation from `temp_dir()` compared it against a
    /// `load_mcp_file` result that had (correctly) canonicalized, and failed
    /// on a difference that is not about the code at all. Resolving here
    /// rather than at each assertion means the next fixture that compares a
    /// path is right without anyone remembering why.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("mecha-eval-test-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            // After creating it: canonicalize needs the directory to exist.
            let dir = dir.canonicalize().unwrap_or(dir);
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn an_mcp_file_parses_and_resolves_paths_against_its_own_directory() {
        let scratch = Scratch::new("resolve");
        std::fs::write(scratch.0.join("server.py"), "# fixture").unwrap();
        let toml_path = scratch.0.join("mcp.toml");
        std::fs::write(
            &toml_path,
            r#"
            [[mcp]]
            name = "graph"
            command = "python3"
            args = ["server.py", "--persona", "graph"]

            [mcp.capabilities]
            untrusted_input = true
            "#,
        )
        .unwrap();

        let servers = load_mcp_file(&toml_path).unwrap();
        assert_eq!(servers.len(), 1);
        // The existing file resolved to an absolute path...
        assert_eq!(
            servers[0].args[0],
            scratch.0.join("server.py").to_string_lossy()
        );
        // ...and everything that is not a file beside the TOML was left alone.
        assert_eq!(servers[0].command, "python3");
        assert_eq!(servers[0].args[1], "--persona");
        assert!(servers[0].capabilities.untrusted_input);
    }

    #[test]
    fn an_mcp_file_with_unknown_fields_is_rejected() {
        let scratch = Scratch::new("unknown");
        let toml_path = scratch.0.join("mcp.toml");
        // A whole config file pasted here must fail loudly, not silently
        // contribute only its `[[mcp]]` tables.
        std::fs::write(
            &toml_path,
            "default_provider = \"local\"\n[[mcp]]\nname = \"x\"\n",
        )
        .unwrap();
        assert!(load_mcp_file(&toml_path).is_err());
    }

    #[test]
    fn an_mcp_file_naming_no_servers_is_rejected() {
        let scratch = Scratch::new("empty");
        let toml_path = scratch.0.join("mcp.toml");
        std::fs::write(&toml_path, "# nothing here\n").unwrap();
        let err = load_mcp_file(&toml_path).unwrap_err();
        assert!(err.to_string().contains("names no MCP servers"), "{err}");
    }
}
