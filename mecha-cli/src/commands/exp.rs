//! `mecha exp` — the experiment surface (`docs/EXPERIMENT-DESIGN.md` §3,
//! Part II §14): `new` writes a design, `run` drives its trials, `status`
//! and `judge` read them back, `export` hands the whole record over.
//!
//! An arm is a model and a harness configuration, and an experiment may
//! vary either or both. `mecha eval` is the special case — arms that name
//! models under the `bare` preset, run in-process, printed as a scorecard —
//! and the intended end state is eval as a thin front over this (the
//! owner's ruling, 2026-09-04). The two share the substrate today: the case
//! file and its graders, the fixture staging, the candidate gate.
//!
//! **Every trial is a child `mecha run`** (D3), started with `MECHA_HOME`
//! pointing at its arm's home under the experiment directory, whose
//! `config.toml` *is* the arm (D12), marked `SessionKind::Experiment`
//! (D13), and told which trial it is through `MECHA_EXPERIMENT_REF` so its
//! session record names the trial back (§4). Nothing in a trial home is
//! ever copied into the real one.

use crate::GlobalOpts;
use anyhow::{Context, Result};
use mecha_core::batch::Prompt;
use mecha_core::experiment::{
    judge, ChildInvocation, ExperimentRef, ExperimentStore, Manifest, Trial, TrialKind,
    TrialStatus, EXPERIMENT_REF_ENV,
};
use std::path::{Path, PathBuf};

/// How long a case's `expect.verify` command may run in the trial's
/// workspace. Eval's own default; a verify that needs longer is a case
/// that needs a smaller check.
const VERIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Does this arm move `key` through an override? An arm that does is the
/// treatment for that knob, and a case's own value for it must not be
/// passed as a flag over the arm's config.
fn arm_moves(arm: &mecha_core::experiment::Arm, key: &str) -> bool {
    arm.overrides.iter().any(|o| {
        o.split_once('=')
            .map(|(k, _)| k.trim() == key)
            .unwrap_or(false)
    })
}

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Create an experiment from a manifest file. The design is written
    /// once, before anything runs; a second `new` over the same name refuses.
    New {
        /// The manifest (TOML). Its `name` names the experiment.
        manifest: PathBuf,
    },
    /// Drive every trial the design calls for that has not finished. Resumes:
    /// a trial found `running` at start crashed with its runner and is
    /// rerun; a finished one is never rerun.
    Run {
        name: String,
        /// Stop after this many trials this invocation.
        #[arg(long)]
        limit: Option<usize>,
        /// Plan and print the trials without spawning anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Where the trials stand, per arm.
    Status {
        name: String,
        #[arg(long)]
        json: bool,
    },
    /// Judge every treatment arm against the control through the gate.
    Judge {
        name: String,
        #[arg(long)]
        json: bool,
    },
    /// The whole record — manifest, trials, judgements — as one JSON.
    Export { name: String },
}

pub async fn execute(_global: &GlobalOpts, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::New { manifest } => new(&manifest),
        Cmd::Run {
            name,
            limit,
            dry_run,
        } => run(&name, limit, dry_run).await,
        Cmd::Status { name, json } => status(&name, json),
        Cmd::Judge { name, json } => judge_cmd(&name, json),
        Cmd::Export { name } => export(&name),
    }
}

fn new(path: &Path) -> Result<()> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let manifest = Manifest::parse(&text).with_context(|| path.display().to_string())?;
    let store = ExperimentStore::open_default(&manifest.name)?;
    store.create(&text)?;
    println!(
        "created {} — {} arms (control `{}`), kind {:?}",
        store.root().display(),
        manifest.arms.len(),
        manifest.control,
        manifest.kind
    );
    Ok(())
}

/// The tasks a manifest names, as eval cases, in the case file's order.
fn cases_for(manifest: &Manifest) -> Result<Vec<mecha_core::eval::EvalCase>> {
    let cases = crate::commands::eval::load_cases(&manifest.tasks.cases, &manifest.tasks.tags)?;
    let cases: Vec<_> = if manifest.tasks.ids.is_empty() {
        cases
    } else {
        for id in &manifest.tasks.ids {
            anyhow::ensure!(
                cases.iter().any(|c| &c.id == id),
                "task `{id}` is not in {}",
                manifest.tasks.cases.display()
            );
        }
        cases
            .into_iter()
            .filter(|c| manifest.tasks.ids.contains(&c.id))
            .collect()
    };
    anyhow::ensure!(!cases.is_empty(), "the manifest names no tasks");
    // `eval::grade` is pure and never sees `expect.judge`; eval appends the
    // judge's verdict afterwards, and this driver does not build one yet.
    // A case whose only assertion is a rubric would then pass every arm
    // unconditionally — a tie on every pair — with nothing on the row
    // saying the rubric evaporated. Refused by name until the driver can
    // grade it (found on review).
    let judged: Vec<&str> = cases
        .iter()
        .filter(|c| c.expect.judge.is_some())
        .map(|c| c.id.as_str())
        .collect();
    anyhow::ensure!(
        judged.is_empty(),
        "case(s) {} carry an `expect.judge` rubric, which `mecha exp` cannot grade yet; name cases with deterministic checks",
        judged.join(", ")
    );
    Ok(cases)
}

/// The provider and model the trials will run against, from the operator's
/// config — recorded into every trial's condition hash, so a trial run
/// against a different model later does not pair with one run today.
fn provider_and_model(cfg: &mecha_core::config::Config) -> Result<(String, String)> {
    let provider = cfg.default_provider.clone();
    let p = cfg
        .providers
        .get(&provider)
        .with_context(|| format!("no [providers.{provider}] in the operator's config"))?;
    Ok((
        provider,
        p.model.clone().unwrap_or_else(|| "(default)".into()),
    ))
}

async fn run(name: &str, limit: Option<usize>, dry_run: bool) -> Result<()> {
    let store = ExperimentStore::open_default(name)?;
    let manifest = store.manifest()?;
    anyhow::ensure!(
        manifest.kind == TrialKind::Single,
        "`{name}` is a {:?} experiment; only `single` trials run today (the lifetime driver is the next step)",
        manifest.kind
    );
    let cases = cases_for(&manifest)?;
    let real = mecha_core::config::Config::load_global()?;
    let (provider, model) = provider_and_model(&real)?;
    let task_ids: Vec<String> = cases.iter().map(|c| c.id.clone()).collect();
    let (planned, skipped) = store.plan(&manifest, &task_ids, &provider, &model)?;
    if skipped > 0 {
        eprintln!(
            "mecha exp: {skipped} trial file(s) could not be read and are counted, not rerun"
        );
    }
    let todo: Vec<&Trial> = planned
        .iter()
        .filter(|t| matches!(t.status, TrialStatus::Pending | TrialStatus::Running))
        .collect();
    let done = planned.len() - todo.len();
    eprintln!(
        "mecha exp `{name}`: {} trials planned, {done} finished, {} to run{} · {provider} ({model})",
        planned.len(),
        todo.len(),
        limit.map(|l| format!(" (limit {l})")).unwrap_or_default()
    );
    if dry_run {
        for t in &todo {
            println!(
                "{}  arm={} task={} seed={} rep={} hash={}",
                t.id,
                t.arm,
                t.task,
                t.seed.map(|s| s.to_string()).unwrap_or_else(|| "-".into()),
                t.repetition,
                t.condition_hash
            );
        }
        return Ok(());
    }
    let mecha = std::env::current_exe().context("locating this binary")?;
    let mut ran = 0usize;
    for planned_trial in todo {
        if limit.is_some_and(|l| ran >= l) {
            break;
        }
        let mut trial = planned_trial.clone();
        let case = cases
            .iter()
            .find(|c| c.id == trial.task)
            .expect("planned from these cases");
        let arm = &manifest.arms[&trial.arm];
        ran += 1;
        eprintln!("· {} ({ran})", trial.id);
        match run_one(&store, &manifest, &mecha, &real, arm, case, &mut trial).await {
            Ok(()) => {}
            Err(e) => {
                trial.status = TrialStatus::Failed;
                trial.error = Some(format!("{e:#}"));
                trial.finished_at = Some(chrono::Utc::now().to_rfc3339());
                eprintln!("  failed: {e:#}");
            }
        }
        store.save_trial(&trial)?;
    }
    eprintln!("mecha exp `{name}`: {ran} trial(s) run this invocation");
    Ok(())
}

/// One trial: the arm's home and config, a fresh workspace, the child, the
/// grade, the stats. Everything the trial learned is on its row when this
/// returns; a failure anywhere is the row's `error`, never a missing row.
async fn run_one(
    store: &ExperimentStore,
    manifest: &Manifest,
    mecha: &Path,
    real: &mecha_core::config::Config,
    arm: &mecha_core::experiment::Arm,
    case: &mecha_core::eval::EvalCase,
    trial: &mut Trial,
) -> Result<()> {
    let prompt = match &case.prompt {
        Prompt::One(p) => p.clone(),
        Prompt::Many(_) => anyhow::bail!(
            "case `{}` is multi-turn; `exp single` drives one prompt per trial today",
            case.id
        ),
    };
    let home = store.arm_home(&trial.arm)?;
    let ChildInvocation {
        config,
        flags,
        passthrough,
    } = mecha_core::experiment::child_invocation(real, arm, trial.seed)?;
    std::fs::write(home.join("config.toml"), toml::to_string_pretty(&config)?)
        .with_context(|| format!("writing {}", home.join("config.toml").display()))?;

    let workspace = store.workspace_for(&trial.id);
    if workspace.exists() {
        std::fs::remove_dir_all(&workspace)?;
    }
    mecha_core::eval::stage_workspace(&manifest.tasks.fixture, &workspace)
        .with_context(|| format!("staging {}", manifest.tasks.fixture.display()))?;

    trial.status = TrialStatus::Running;
    trial.started_at = Some(chrono::Utc::now().to_rfc3339());
    trial.error = None;
    store.save_trial(trial)?;

    let reference = ExperimentRef {
        exp_id: manifest.name.clone(),
        trial_id: trial.id.clone(),
        arm: trial.arm.clone(),
        actor: trial.id.clone(),
        role: None,
        task: trial.task.clone(),
        repetition: trial.repetition,
        condition_hash: trial.condition_hash.clone(),
    };
    let mut cmd = tokio::process::Command::new(mecha);
    cmd.arg("run")
        .arg("--json")
        .arg("--yes")
        .arg("--no-stream")
        .arg("--workspace")
        .arg(&workspace)
        // The staged workspace is the child's cwd, so no `mecha.toml` in
        // whatever checkout the runner was started from layers over the
        // arm — the hazard `global_config_only` names for the trigger
        // daemon, which is not reachable from a child's argv (found on
        // review). Holds as long as no fixture ships a `mecha.toml`.
        .current_dir(&workspace);
    for f in &flags {
        cmd.arg(f);
    }
    // A case's own turn ceiling and compaction threshold are part of the
    // task, and eval applies both — unless the arm moves the same knob, in
    // which case the arm is the treatment and wins; the flag would otherwise
    // override the arm's config unconditionally. A compaction case that did
    // not get its threshold graded the harness rather than the arm (found on
    // review).
    if let Some(n) = case.max_turns.filter(|_| !arm_moves(arm, "max_turns")) {
        cmd.arg("--max-turns").arg(n.to_string());
    }
    if let Some(n) = case
        .compact_at_tokens
        .filter(|_| !arm_moves(arm, "compact_at_tokens"))
    {
        cmd.arg("--compact-at").arg(n.to_string());
    }
    cmd.arg(&prompt);
    // **An allowlist, not a denylist.** The child's whole store is the arm's
    // home, and `MECHA_HOME` is not the only variable that moves a store:
    // `MECHA_LEARNING_DIR`, `MECHA_OUTBOX_DIR`, `MECHA_QUESTIONS_DIR`,
    // `MECHA_MESSAGES_DIR` and `MECHA_TRIGGERS_DIR` each point one store at
    // the real one ahead of the home, and `MECHA_PROVIDER` / `MECHA_MODEL` /
    // `MECHA_EFFORT` rewrite the arm above its config — so an operator with
    // any of them exported would have run a trial against their real
    // learning store, or hashed a condition for a model that did not run
    // (found on review). Cleared, then only what the child needs, on
    // `Sandbox::child_env`'s shape: the base set, the provider's key
    // variable, and the three that name this trial.
    cmd.env_clear();
    for (k, v) in mecha_core::sandbox::Sandbox::child_env(&passthrough) {
        cmd.env(k, v);
    }
    cmd.env("MECHA_HOME", &home)
        .env(mecha_core::session::SESSION_KIND_ENV, "experiment")
        .env(EXPERIMENT_REF_ENV, serde_json::to_string(&reference)?)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let output = cmd.output().await.context("spawning mecha run")?;
    let trial_dir = store.workspace_for(&trial.id);
    let log = trial_dir
        .parent()
        .map(|p| p.join("stderr.log"))
        .unwrap_or_else(|| trial_dir.join("stderr.log"));
    let _ = std::fs::write(&log, &output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let start = stdout.find('{').with_context(|| {
        format!(
            "the child printed no JSON (exit {}); its stderr is at {}",
            output.status,
            log.display()
        )
    })?;
    let value: serde_json::Value = serde_json::from_str(&stdout[start..]).with_context(|| {
        format!(
            "parsing the child's JSON; its stderr is at {}",
            log.display()
        )
    })?;
    let result: mecha_core::batch::BatchResult =
        serde_json::from_value(value.clone()).context("the child's JSON is not a run result")?;
    trial.session_id = value
        .get("session")
        .and_then(|s| s.as_str())
        .map(str::to_string);

    let mut graded = mecha_core::eval::grade(case, &result);
    // What the run left behind, checked the way eval checks it: `grade` is
    // pure and never sees `expect.verify`, and a codegen case whose only
    // assertion is the verify command would otherwise pass every arm with
    // zero checks — the same evaporation `expect.judge` is refused for
    // (found on review). The staged workspace is this trial's own.
    if let Some(command) = &case.expect.verify {
        graded.add_check(
            mecha_core::eval::verify_workspace(command, &workspace, VERIFY_TIMEOUT).await,
        );
    }
    trial.passed = Some(graded.passed);
    trial.checks = graded.checks;
    trial.stats = match &trial.session_id {
        Some(id) => mecha_core::session::Session::episode_stats(
            &home.join("sessions").join(format!("{id}.jsonl")),
        )
        .unwrap_or(None),
        None => None,
    };
    trial.status = TrialStatus::Done;
    trial.finished_at = Some(chrono::Utc::now().to_rfc3339());
    eprintln!(
        "  {} · {} turns · {}",
        if graded.passed { "pass" } else { "FAIL" },
        result.turns,
        trial
            .stats
            .as_ref()
            .and_then(|s| s.stop_cause)
            .map(|c| c.describe())
            .unwrap_or("no outcome recorded")
    );
    Ok(())
}

fn status(name: &str, json: bool) -> Result<()> {
    let store = ExperimentStore::open_default(name)?;
    let manifest = store.manifest()?;
    // The plan, not the store alone: between `new` and the first `run` the
    // store holds no rows, and an all-zero table reads the same as a design
    // that calls for nothing (found on review). When the case file cannot
    // be read the store's rows stand in, and the readout says so.
    let (trials, skipped) = match cases_for(&manifest).and_then(|cases| {
        let real = mecha_core::config::Config::load_global()?;
        let (provider, model) = provider_and_model(&real)?;
        let ids: Vec<String> = cases.iter().map(|c| c.id.clone()).collect();
        store.plan(&manifest, &ids, &provider, &model)
    }) {
        Ok((planned, skipped)) => (
            planned
                .into_iter()
                .map(|t| (t.id.clone(), t))
                .collect::<std::collections::BTreeMap<_, _>>(),
            skipped,
        ),
        Err(e) => {
            eprintln!("mecha exp: the design's tasks could not be planned ({e:#}); showing the store's rows only");
            store.trials()?
        }
    };
    let mut by_arm: std::collections::BTreeMap<&str, [usize; 5]> = Default::default();
    for arm in manifest.arms.keys() {
        by_arm.insert(arm.as_str(), [0; 5]);
    }
    let mut passed: std::collections::BTreeMap<&str, (usize, usize)> = Default::default();
    for t in trials.values() {
        let row = by_arm.entry(t.arm.as_str()).or_default();
        match t.status {
            TrialStatus::Pending => row[0] += 1,
            TrialStatus::Running => row[1] += 1,
            TrialStatus::Done => row[2] += 1,
            TrialStatus::Failed => row[3] += 1,
            TrialStatus::Unknown => row[4] += 1,
        }
        if let Some(p) = t.passed {
            let e = passed.entry(t.arm.as_str()).or_default();
            e.1 += 1;
            if p {
                e.0 += 1;
            }
        }
    }
    if json {
        let rows: Vec<_> = by_arm
            .iter()
            .map(|(arm, c)| {
                serde_json::json!({
                    "arm": arm,
                    "pending": c[0], "running": c[1], "done": c[2], "failed": c[3], "unknown": c[4],
                    "passed": passed.get(arm).map(|(p, _)| p),
                    "graded": passed.get(arm).map(|(_, n)| n),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "name": manifest.name,
                "kind": manifest.kind,
                "control": manifest.control,
                "arms": rows,
                "unreadable_trials": skipped,
            }))?
        );
        return Ok(());
    }
    println!(
        "{} ({:?}, control `{}`)",
        manifest.name, manifest.kind, manifest.control
    );
    println!(
        "{:<20} {:>8} {:>8} {:>6} {:>7} {:>8}",
        "arm", "pending", "running", "done", "failed", "passed"
    );
    for (arm, c) in &by_arm {
        let pass = passed
            .get(arm)
            .map(|(p, n)| format!("{p}/{n}"))
            .unwrap_or_else(|| "—".into());
        println!(
            "{arm:<20} {:>8} {:>8} {:>6} {:>7} {pass:>8}",
            c[0], c[1], c[2], c[3]
        );
    }
    if skipped > 0 {
        println!("{skipped} trial file(s) unreadable");
    }
    Ok(())
}

fn judge_cmd(name: &str, json: bool) -> Result<()> {
    let store = ExperimentStore::open_default(name)?;
    let manifest = store.manifest()?;
    let (trials, skipped) = store.trials()?;
    let trials: Vec<Trial> = trials.into_values().collect();
    let verdicts = judge(&manifest, &trials);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "name": manifest.name,
                "control": manifest.control,
                "arms": verdicts,
                "unreadable_trials": skipped,
            }))?
        );
        return Ok(());
    }
    println!(
        "{} — each arm against `{}`",
        manifest.name, manifest.control
    );
    for v in &verdicts {
        let j = &v.judgement;
        println!(
            "\n{}  [{}]  {} pairs ({} selection, {} holdout)",
            v.arm,
            v.metric.as_str(),
            v.pairs,
            v.selection,
            v.holdout
        );
        println!(
            "  selection: {} wins · {} losses · {} ties    holdout: {} wins · {} losses · {} ties",
            j.selection.wins,
            j.selection.losses,
            j.selection.ties,
            j.holdout.wins,
            j.holdout.losses,
            j.holdout.ties
        );
        println!(
            "  work: control {} calls, treatment {} calls",
            j.work_baseline, j.work_candidate
        );
        println!(
            "  {}",
            match &j.disposition {
                mecha_core::candidate::Disposition::Accept =>
                    "ACCEPT — wins on selection, confirmed on the holdout".to_string(),
                mecha_core::candidate::Disposition::Propose(why) => format!("PROPOSE — {why}"),
                mecha_core::candidate::Disposition::Reject(why) => format!("REJECT — {why}"),
            }
        );
    }
    if verdicts.is_empty() {
        println!("no treatment arm has finished trials to judge");
    }
    Ok(())
}

fn export(name: &str) -> Result<()> {
    let store = ExperimentStore::open_default(name)?;
    let manifest = store.manifest()?;
    let (trials, skipped) = store.trials()?;
    let trials: Vec<Trial> = trials.into_values().collect();
    let verdicts = judge(&manifest, &trials);
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "manifest": manifest,
            "trials": trials,
            "judgements": verdicts,
            "unreadable_trials": skipped,
        }))?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A case's own knob is passed to the child unless the arm moves that
    /// knob — then the arm is the treatment and wins.
    #[test]
    fn an_arm_that_moves_a_knob_wins_over_the_case() {
        let mut arm = mecha_core::experiment::Arm::default();
        assert!(!arm_moves(&arm, "max_turns"));
        arm.overrides = vec!["max_turns=20".into(), " compact_at_tokens = 5000".into()];
        assert!(arm_moves(&arm, "max_turns"));
        assert!(arm_moves(&arm, "compact_at_tokens"));
        assert!(!arm_moves(&arm, "max_output_tokens"));
        assert!(
            !arm_moves(&arm, "max_turns_extra"),
            "the whole key, not a prefix"
        );
    }

    /// A manifest's `ids` narrow the case file to the tasks it names, in the
    /// file's order, and a name the file does not carry is a refusal rather
    /// than a silently smaller experiment.
    #[test]
    fn the_tasks_are_the_cases_the_manifest_names() {
        let dir = std::env::temp_dir().join(format!("mecha-exp-cli-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cases = dir.join("cases.jsonl");
        std::fs::write(
            &cases,
            concat!(
                "{\"id\":\"b\",\"prompt\":\"x\",\"expect\":{\"contains\":[\"x\"]},\"tags\":[\"t\"]}\n",
                "{\"id\":\"a\",\"prompt\":\"y\",\"expect\":{\"contains\":[\"y\"]},\"tags\":[\"u\"]}\n",
            ),
        )
        .unwrap();
        let text = format!(
            r#"
name = "cli"
control = "full"
split_seed = 1
[tasks]
cases = "{}"
fixture = "{}"
ids = ["a", "b"]
[arms.full]
[arms.bare]
preset = "bare"
[arms.bare.prediction]
metric = "failure"
rationale = "r"
"#,
            cases.display(),
            dir.display()
        );
        let m = Manifest::parse(&text).unwrap();
        let got: Vec<String> = cases_for(&m).unwrap().into_iter().map(|c| c.id).collect();
        assert_eq!(got, vec!["b", "a"], "the file's order, not the manifest's");
        let mut m2 = m.clone();
        m2.tasks.ids = vec!["nope".into()];
        assert!(cases_for(&m2).unwrap_err().to_string().contains("nope"));
        let mut m3 = m;
        m3.tasks.ids.clear();
        m3.tasks.tags = vec!["t".into()];
        assert_eq!(cases_for(&m3).unwrap().len(), 1, "tags narrow too");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
