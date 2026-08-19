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

use crate::{setup, GlobalOpts};
use anyhow::{Context, Result};
use mecha_core::agent::{Budget, RunContext};
use mecha_core::config::PermissionMode;
use mecha_core::eval::{grade, stage_workspace, EvalCase, GradedCase, Judge, Scorecard};
use mecha_core::tool::ask::{AskUserTool, Asker};
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

    /// Measure a candidate config change: run the case set once as configured
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

    /// One episode in this many is held out of selection, for `--ab-config`.
    #[arg(long, default_value_t = 3, value_name = "N")]
    pub holdout_in: u64,
}

/// A candidate config override, parsed from `KEY=VALUE`.
///
/// Deliberately a closed set. An open one would let a proposer reach settings
/// whose effect is not measurable by this comparison — or worse, security
/// settings, which are never a measurement's to decide.
fn apply_override(opts: &mut GlobalOpts, spec: &str) -> Result<()> {
    let (key, value) = spec
        .split_once('=')
        .with_context(|| format!("--ab-config expects KEY=VALUE, got `{spec}`"))?;
    let num = |what: &str| -> Result<u64> {
        value
            .parse::<u64>()
            .with_context(|| format!("{what} takes a number, got `{value}`"))
    };
    match key {
        "compact_at_tokens" => opts.compact_at = Some(num("compact_at_tokens")?),
        "max_turns" => opts.max_turns = Some(num("max_turns")? as u32),
        "max_output_tokens" => opts.max_output_tokens = Some(num("max_output_tokens")?),
        "effort" => {
            opts.effort = Some(
                value
                    .parse()
                    .map_err(|_| anyhow::anyhow!("unknown effort `{value}`"))?,
            )
        }
        other => anyhow::bail!(
            "`{other}` is not an overridable key; try compact_at_tokens, max_turns, max_output_tokens or effort"
        ),
    }
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Report {
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

    let (scorecard, graded) = run_arm(global, &args, &cases, &fixture, false, &[], "").await?;

    print_scorecard(&scorecard, &graded, args.failures);

    if let Some(path) = &args.out {
        let report = Report {
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
    // Ambient MCP servers would make the tool list depend on local config,
    // so they are opt-in here even though they are on everywhere else.
    if !args.mcp {
        opts.no_mcp = true;
    }
    // Same reproducibility rule for learned rules: a scorecard that depends on
    // what this machine learned last night is not comparable to yesterday's,
    // or anyone else's. When the learning system itself is the thing being
    // measured, that wants a deliberate opt-in flag, not an ambient default —
    // `--ab-rules` is that flag, and its treatment arm is the one place this
    // is ever false.
    opts.no_learned_rules = !with_rules;
    // And for hooks: a user's local policy scripts firing inside eval cases
    // would grade this machine's config, not the model.
    opts.no_hooks = true;
    // And for the outbox: whether a tool executes or stages must not depend
    // on this machine's routing config, and an eval must not fill the user's
    // real outbox with drafts nobody will ever release.
    opts.no_outbox = true;
    // And for provider fallback: a scorecard grades the model it names, and a
    // case silently answered by a fallback model would be a measurement of
    // nothing — worse, an incomparable one mixed invisibly into the set.
    opts.no_fallback = true;
    // And for messaging: a case must not read this machine's real mailbox —
    // or worse, have a stray message from last night's trigger folded into
    // its conversation mid-case.
    opts.no_messages = true;
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

/// The `--ab-rules` report: both arms in full, so nothing about either is
/// hidden, under a top-level shape `--compare` cannot mistake for a
/// scorecard.
#[derive(serde::Serialize)]
struct AbReport {
    ab_rules: bool,
    without_rules: Report,
    with_rules: Report,
    flips: Vec<serde_json::Value>,
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
    // Fail before an hour of inference, not after: the treatment arm needs
    // rules to measure.
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

    eprintln!("── arm A: rules-free ──");
    let (a_card, a_graded) = run_arm(global, args, cases, fixture, false, &[], "a").await?;
    eprintln!("── arm B: with this machine's learned rules ──");
    let (b_card, b_graded) = run_arm(global, args, cases, fixture, true, &[], "b").await?;

    // A case passes an arm when every run of it passed — the same pass^k the
    // scorecard reports.
    let case_pass = |graded: &[GradedCase], id: &str| {
        let runs: Vec<&GradedCase> = graded.iter().filter(|g| g.id == id).collect();
        !runs.is_empty() && runs.iter().all(|g| g.passed)
    };

    let mut flips = Vec::new();
    println!("\n── rules A/B ──");
    println!(
        "arm A (rules-free):  {}/{} cases",
        a_card.passed, a_card.total
    );
    println!(
        "arm B (with rules):  {}/{} cases",
        b_card.passed, b_card.total
    );
    for case in cases {
        let (a, b) = (
            case_pass(&a_graded, &case.id),
            case_pass(&b_graded, &case.id),
        );
        if a != b {
            let label = if b { "IMPROVED" } else { "REGRESSED" };
            println!("  {label}: {}", case.id);
            flips.push(serde_json::json!({
                "id": case.id,
                "without_rules": a,
                "with_rules": b,
            }));
        }
    }
    let net = b_card.passed as i64 - a_card.passed as i64;
    println!(
        "net: {net:+} case(s); {} flip(s) — judge-graded flips are a prompt to read the \
         answers, not a verdict",
        flips.len()
    );

    if let Some(path) = &args.out {
        let report = |scorecard: &Scorecard, graded: &[GradedCase]| Report {
            scorecard: scorecard.clone(),
            cases: graded
                .iter()
                .map(|g| serde_json::to_value(g).unwrap_or(serde_json::Value::Null))
                .collect(),
        };
        let ab = AbReport {
            ab_rules: true,
            without_rules: report(&a_card, &a_graded),
            with_rules: report(&b_card, &b_graded),
            flips,
        };
        std::fs::write(path, serde_json::to_string_pretty(&ab)?)
            .with_context(|| format!("writing {}", path.display()))?;
        eprintln!("\nwrote {}", path.display());
    }
    Ok(())
}

/// Nobody is watching an eval run, so every question goes unanswered — which is
/// the honest thing for the tool to report, and leaves the model to proceed and
/// say which reading it chose.
struct NoOneToAsk;

#[async_trait::async_trait]
impl Asker for NoOneToAsk {
    async fn ask(&self, _question: &str, _options: &[String]) -> Option<String> {
        None
    }
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

fn load_cases(path: &Path, tags: &[String]) -> Result<Vec<EvalCase>> {
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
    use mecha_core::candidate::{judge_with, ChangeClass, Disposition};

    // Parse before an hour of inference, not after: a typo in an override
    // should cost a line of output, not two full arms.
    for spec in &args.ab_config {
        apply_override(&mut global.clone(), spec)?;
    }
    anyhow::ensure!(
        args.holdout_in >= 2,
        "--holdout-in must be at least 2, or every episode is held out and \
         nothing selects"
    );

    eprintln!("── arm A: as configured ──");
    let (a_card, a_graded) = run_arm(global, args, cases, fixture, false, &[], "a").await?;
    eprintln!("── arm B: {} ──", args.ab_config.join(", "));
    let (b_card, b_graded) =
        run_arm(global, args, cases, fixture, false, &args.ab_config, "b").await?;

    // pass^k in both arms, the same bar the scorecard reports.
    let case_pass = |graded: &[GradedCase], id: &str| {
        let runs: Vec<&GradedCase> = graded.iter().filter(|g| g.id == id).collect();
        !runs.is_empty() && runs.iter().all(|g| g.passed)
    };
    let calls = |graded: &[GradedCase], id: &str| -> u64 {
        graded
            .iter()
            .filter(|g| g.id == id)
            .map(|g| g.tools_called.len() as u64)
            .sum()
    };

    struct Outcome {
        id: String,
        was: bool,
        now: bool,
        work_a: u64,
        work_b: u64,
    }
    // Only cases that ran in both arms: one missing from an arm is missing,
    // not a tie, and scoring it either way lets a candidate that dies on the
    // hard cases look good on the ones it survived.
    let outcomes: Vec<Outcome> = cases
        .iter()
        .filter(|c| a_graded.iter().any(|g| g.id == c.id) && b_graded.iter().any(|g| g.id == c.id))
        .map(|c| Outcome {
            id: c.id.clone(),
            was: case_pass(&a_graded, &c.id),
            now: case_pass(&b_graded, &c.id),
            work_a: calls(&a_graded, &c.id),
            work_b: calls(&b_graded, &c.id),
        })
        .collect();

    fn cost(o: &Outcome) -> (&str, f64, f64) {
        (
            o.id.as_str(),
            f64::from(u8::from(!o.was)),
            f64::from(u8::from(!o.now)),
        )
    }
    let judgement = judge_with(
        ChangeClass::Config,
        &outcomes,
        cost,
        |o| (o.work_a, o.work_b),
        args.holdout_in,
    );

    println!("\n── config A/B ──");
    println!(
        "arm A (as configured): {}/{} cases",
        a_card.passed, a_card.total
    );
    println!(
        "arm B ({}): {}/{} cases",
        args.ab_config.join(", "),
        b_card.passed,
        b_card.total
    );
    for o in &outcomes {
        if o.was != o.now {
            println!(
                "  {}: {}",
                if o.now { "IMPROVED" } else { "REGRESSED" },
                o.id
            );
        }
    }
    println!(
        "\nselection  {}+ {}- {}=    holdout  {}+ {}- {}=",
        judgement.selection.wins,
        judgement.selection.losses,
        judgement.selection.ties,
        judgement.holdout.wins,
        judgement.holdout.losses,
        judgement.holdout.ties,
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
         sample of a non-deterministic measurement"
    );

    if let Some(path) = &args.out {
        let out = serde_json::json!({
            "ab_config": args.ab_config,
            "holdout_in": args.holdout_in,
            "judgement": judgement,
            "arm_a": { "passed": a_card.passed, "total": a_card.total },
            "arm_b": { "passed": b_card.passed, "total": b_card.total },
            "cases": outcomes.iter().map(|o| serde_json::json!({
                "id": o.id, "was": o.was, "now": o.now,
                "work_a": o.work_a, "work_b": o.work_b,
            })).collect::<Vec<_>>(),
        });
        std::fs::write(path, serde_json::to_string_pretty(&out)?)
            .with_context(|| format!("writing {}", path.display()))?;
        eprintln!("\nwrote {}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory that cleans up after itself, so a failing test
    /// does not leave litter that changes what the next run's `is_file`
    /// checks see.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("mecha-eval-test-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
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
