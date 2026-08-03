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

    /// Compare previously written scorecards side by side instead of running.
    #[arg(long, num_args = 1.., conflicts_with_all = ["out", "fixture"])]
    pub compare: Vec<PathBuf>,
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

    let fixture = args
        .fixture
        .clone()
        .unwrap_or_else(|| args.cases.parent().unwrap_or(Path::new(".")).join("workspace"));
    anyhow::ensure!(
        fixture.is_dir(),
        "fixture workspace {} does not exist",
        fixture.display()
    );

    // Force read-only and point the workspace at the fixture, whatever the
    // caller's flags or config said. An eval that can mutate its own fixture
    // isn't measuring anything repeatable.
    let opts = GlobalOpts {
        workspace: Some(fixture.clone()),
        read_only: true,
        yes: false,
        ..global.clone()
    };
    let prepared = setup::prepare(&opts, false).await?;

    // Build the judge before running anything. A case set that cannot be
    // graded should fail in the first second, not after an hour of inference.
    let judge = build_judge(&args, &prepared, &cases)?;

    // Stage a private workspace for every sandboxed case, up front, so a
    // staging failure is not discovered halfway through the run.
    let sandbox_root =
        std::env::temp_dir().join(format!("mecha-eval-{}", std::process::id()));
    let contexts = prepare_contexts(&cases, &fixture, &sandbox_root, &prepared)?;
    let sandboxed = cases.iter().filter(|c| c.sandbox).count();

    eprintln!(
        "mecha eval: {} cases · {} ({}) · {} tools · fixture {}",
        cases.len(),
        prepared.model,
        prepared.provider_name,
        prepared.agent.registry().len(),
        fixture.display()
    );
    if sandboxed > 0 {
        eprintln!("  {sandboxed} sandboxed case(s) staged under {}", sandbox_root.display());
    }
    if let Some(j) = &judge {
        eprintln!("  judge: {}", j.model());
    }

    // A verify command is a test run, not a tool call, so it gets its own
    // ceiling rather than borrowing the one meant for the agent's shell.
    let verify_timeout = std::time::Duration::from_secs(
        prepared.config.tools.shell_timeout_secs.max(120),
    );

    let items: Vec<_> = cases.iter().map(EvalCase::to_item).collect();
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
        let Some(case) = cases.iter().find(|c| c.id == result.id) else { continue };
        let mut g = grade(case, result);

        // What the run left behind, checked before anything a model says about
        // it. For a codegen case this is the only check that isn't hearsay.
        if let Some(command) = &case.expect.verify {
            // `validate` guarantees a verify case is sandboxed, so a missing
            // context is a bug here rather than a case-file mistake. Fail the
            // check saying so — falling back to the shared fixture would run
            // the command against the very directory sandboxing protects.
            g.add_check(match contexts.get(&case.id) {
                Some(cx) => {
                    mecha_core::eval::verify_workspace(
                        command,
                        &cx.tools.workspace,
                        verify_timeout,
                    )
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
    // Report in case-file order so two runs read the same way.
    graded.sort_by_key(|g| cases.iter().position(|c| c.id == g.id).unwrap_or(usize::MAX));

    let scorecard = Scorecard::of(
        &graded,
        prepared.model.clone(),
        prepared.provider_name.clone(),
        started.elapsed().as_millis() as u64,
    );

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

    if sandbox_root.exists() {
        if args.keep_workspaces {
            eprintln!("staged workspaces kept in {}", sandbox_root.display());
        } else if let Err(e) = std::fs::remove_dir_all(&sandbox_root) {
            // Leftover temp directories are untidy, not wrong. Say so and
            // carry on rather than failing a run that already has its answer.
            eprintln!("mecha: could not clean up {}: {e}", sandbox_root.display());
        }
    }

    // Non-zero when anything failed, so this can gate CI.
    if scorecard.passed < scorecard.total {
        std::process::exit(1);
    }
    Ok(())
}

/// Build the per-case contexts: a private staged workspace for sandboxed cases,
/// a raised turn budget for cases that ask for one, or both.
///
/// Cases needing neither get no entry and run on the agent's own context.
fn prepare_contexts(
    cases: &[EvalCase],
    fixture: &Path,
    root: &Path,
    prepared: &setup::Prepared,
) -> Result<HashMap<String, Arc<RunContext>>> {
    let mut contexts = HashMap::new();

    for case in cases
        .iter()
        .filter(|c| c.sandbox || c.max_turns.is_some() || c.compact_at_tokens.is_some())
    {
        let base = prepared.agent.context();

        let cx = if case.sandbox {
            let dir = root.join(safe_dir_name(&case.id));
            stage_workspace(fixture, &dir)
                .with_context(|| format!("staging a workspace for case `{}`", case.id))?;

            // Canonicalize now: the path jail compares canonical paths, and a
            // temp directory reached through a symlink would otherwise make the
            // jail refuse every path the case touches.
            let dir = dir
                .canonicalize()
                .with_context(|| format!("resolving {}", dir.display()))?;

            base.sandboxed(dir, Arc::new(ModeApprover { mode: PermissionMode::Allow }))
        } else {
            base.as_ref().clone()
        };

        let budget = Budget { max_turns: case.max_turns, ..Budget::default() };
        let cx = cx.with_budget(budget).with_compact_at(case.compact_at_tokens);
        contexts.insert(case.id.clone(), Arc::new(cx));
    }
    Ok(contexts)
}

/// Case ids become directory names, and a `/` in one would silently stage the
/// workspace somewhere other than where cleanup looks for it.
fn safe_dir_name(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
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

    let model = args.judge_model.clone().or_else(|| provider_cfg.model.clone());
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

fn load_cases(path: &Path, tags: &[String]) -> Result<Vec<EvalCase>> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("opening {}", path.display()))?;

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

    println!(
        "  cases passed        {}/{}  ({:.0}%)",
        card.passed,
        card.total,
        card.pass_rate() * 100.0
    );
    println!("  checks passed       {:.0}%", card.check_pass_rate * 100.0);

    // The reliability block: these are what disqualify a model for loop use,
    // regardless of how well it scores on the cases themselves.
    println!("\n  malformed arguments {}", card.malformed_tool_args);
    println!("  invented tools      {}", card.unknown_tools);
    println!("  tool errors         {}", card.tool_errors);
    println!("  runs errored        {}", card.runs_errored);

    println!("\n  mean turns          {:.1}", card.mean_turns);
    println!("  median latency      {:.1}s", card.median_latency_ms as f64 / 1000.0);
    println!(
        "  tokens              {} in / {} out",
        card.total_usage.total_input(),
        card.total_usage.output_tokens
    );
    println!("  wall clock          {:.1}s", card.wall_clock_ms as f64 / 1000.0);

    if !card.by_tag.is_empty() {
        println!("\n  by tag");
        for tag in &card.by_tag {
            let bar = if tag.total == 0 {
                String::new()
            } else {
                let filled = (tag.passed * 10).div_ceil(tag.total.max(1));
                format!("{}{}", "█".repeat(filled), "·".repeat(10 - filled))
            };
            println!("    {:<18} {} {}/{}", tag.tag, bar, tag.passed, tag.total);
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
            println!("    {:<24} {}", case.id, reasons.join(", "));
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
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let report: Report = serde_json::from_str(&text)
            .with_context(|| format!("{} is not a mecha eval report", path.display()))?;
        cards.push(report.scorecard);
    }

    let w = cards.iter().map(|c| c.model.len()).max().unwrap_or(10).max(10);
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
    row(
        "pass rate",
        cards.iter().map(|c| format!("{:.0}%", c.pass_rate() * 100.0)).collect(),
    );
    row(
        "checks",
        cards.iter().map(|c| format!("{:.0}%", c.check_pass_rate * 100.0)).collect(),
    );
    row(
        "malformed args",
        cards.iter().map(|c| c.malformed_tool_args.to_string()).collect(),
    );
    row("invented tools", cards.iter().map(|c| c.unknown_tools.to_string()).collect());
    row("mean turns", cards.iter().map(|c| format!("{:.1}", c.mean_turns)).collect());
    row(
        "median latency",
        cards
            .iter()
            .map(|c| format!("{:.1}s", c.median_latency_ms as f64 / 1000.0))
            .collect(),
    );
    row(
        "output tokens",
        cards.iter().map(|c| c.total_usage.output_tokens.to_string()).collect(),
    );

    // Per-tag rows only where the tag exists in every report.
    let shared: Vec<String> = cards
        .first()
        .map(|c| {
            c.by_tag
                .iter()
                .map(|t| t.tag.clone())
                .filter(|tag| {
                    cards.iter().all(|c| c.by_tag.iter().any(|t| &t.tag == tag))
                })
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
