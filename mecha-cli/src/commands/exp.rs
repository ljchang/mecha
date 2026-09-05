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
use mecha_core::experiment::{PrincipalPoint, StageLever};
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
        Cmd::Status { name, json } => status(&name, json).await,
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
        "created {} — {} arms ({}), kind {:?}",
        store.root().display(),
        manifest.arms.len(),
        control_label(&manifest),
        manifest.kind
    );
    Ok(())
}

/// `control \`x\`` for a comparison, or what a measurement is.
fn control_label(manifest: &Manifest) -> String {
    manifest
        .control
        .as_deref()
        .map(|c| format!("control `{c}`"))
        .unwrap_or_else(|| "a measurement, no control".into())
}

/// The tasks a manifest names, as eval cases — **in the manifest's order**
/// when `ids` names them, the file's otherwise. For a lifetime the
/// sequence *is* the design, and `position` on every row and the pairing
/// in `judge` follow it; the first cut kept the file's order, so a
/// manifest saying `["cross-file", "read-readme"]` walked them the other
/// way round with the manifest still claiming the sequence it did not run
/// (found on review). A single fans out over a set, so the same order
/// costs it nothing.
async fn cases_for(manifest: &Manifest) -> Result<Vec<mecha_core::eval::EvalCase>> {
    let cases = match manifest.tasks.cases_path() {
        Some(path) => crate::commands::eval::load_cases(path, &manifest.tasks.tags)?,
        // A task source: `list` answers with the tasks, the driver filters
        // by tag exactly as it filters a case file.
        None => source_list(&manifest.tasks)
            .await?
            .into_iter()
            .filter(|c| {
                manifest.tasks.tags.is_empty()
                    || c.tags.iter().any(|t| manifest.tasks.tags.contains(t))
            })
            .collect(),
    };
    let cases: Vec<_> = if manifest.tasks.ids.is_empty() {
        cases
    } else {
        let mut ordered = Vec::with_capacity(manifest.tasks.ids.len());
        for id in &manifest.tasks.ids {
            let case = cases.iter().find(|c| &c.id == id).with_context(|| {
                format!(
                    "task `{id}` is not among the tasks {}",
                    task_origin(&manifest.tasks)
                )
            })?;
            ordered.push(case.clone());
        }
        ordered
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

/// Where the tasks came from, for a message.
fn task_origin(tasks: &mecha_core::experiment::Tasks) -> String {
    match tasks.cases_path() {
        Some(p) => format!("in {}", p.display()),
        None => format!("the source `{}` lists", tasks.source.join(" ")),
    }
}

/// Run one of the task source's verbs (Part II §21.1: `list`, `setup
/// <task>`, `grade <task>`) as a child process on the run child's base
/// environment plus the pointers `source_env` names, from `cwd`, with
/// `stdin` handed over and the answer read from stdout. Fail-closed on
/// every edge: a non-zero exit, a timeout, no JSON, or JSON in a shape this
/// build does not know is an error, never an empty list or a pass. The
/// source's relative file paths resolve against the checkout `exp run`
/// starts from, like a fixture server's.
async fn source_call(
    tasks: &mecha_core::experiment::Tasks,
    verb: &[&str],
    env: &[(String, String)],
    cwd: &Path,
    stdin: Option<String>,
) -> Result<String> {
    let mut command = tasks.source.clone();
    let base = std::env::current_dir().context("cannot determine the working directory")?;
    mecha_core::experiment::resolve_file_args(&mut command, &base);
    let (exe, args) = command
        .split_first()
        .context("the task source names no executable")?;
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(args).args(verb);
    cmd.env_clear();
    for (k, v) in mecha_core::sandbox::Sandbox::child_env(&[]) {
        cmd.env(k, v);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    std::fs::create_dir_all(cwd)?;
    cmd.current_dir(cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawning the task source `{exe}`"))?;
    let mut pipe = child.stdin.take().context("the task source's stdin")?;
    let payload = stdin.unwrap_or_default();
    let exchange = async move {
        use tokio::io::AsyncWriteExt;
        let write = async move {
            pipe.write_all(payload.as_bytes()).await?;
            pipe.shutdown().await?;
            drop(pipe);
            Ok::<(), std::io::Error>(())
        };
        let (written, output) = tokio::join!(write, child.wait_with_output());
        written.context("handing the task source its input")?;
        output.context("waiting for the task source")
    };
    let output = match tokio::time::timeout(
        std::time::Duration::from_secs(tasks.source_timeout_secs),
        exchange,
    )
    .await
    {
        Ok(o) => o?,
        Err(_) => anyhow::bail!(
            "the task source did not answer `{}` within {}s",
            verb.join(" "),
            tasks.source_timeout_secs
        ),
    };
    anyhow::ensure!(
        output.status.success(),
        "the task source exited {} on `{}`: {}",
        output.status,
        verb.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The JSON a verb answered with, from wherever it starts on stdout — a
/// source may print a line first. `setup` answers with its exit code alone
/// and never comes through here.
fn source_json<'a>(text: &'a str, verb: &str) -> Result<&'a str> {
    let start = text
        .find(['{', '['])
        .with_context(|| format!("the task source printed no JSON on `{verb}`"))?;
    Ok(&text[start..])
}

/// `list`: the source's tasks as cases, in the source's order.
async fn source_list(
    tasks: &mecha_core::experiment::Tasks,
) -> Result<Vec<mecha_core::eval::EvalCase>> {
    // A directory of this process's own: two drivers listing at once shared
    // one (found on review).
    let scratch =
        std::env::temp_dir().join(format!("mecha-exp-source-list-{}", std::process::id()));
    let text = source_call(tasks, &["list"], &[], &scratch, None).await?;
    let listed: Vec<mecha_core::experiment::SourceTask> =
        serde_json::from_str(source_json(&text, "list")?)
            .context("the task source's `list` is not the contract's shape")?;
    anyhow::ensure!(!listed.is_empty(), "the task source lists no tasks");
    let mut seen = std::collections::BTreeSet::new();
    let mut cases = Vec::with_capacity(listed.len());
    for t in listed {
        anyhow::ensure!(
            seen.insert(t.id.clone()),
            "the task source lists `{}` twice",
            t.id
        );
        cases.push(t.into_case()?);
    }
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
    let cases = cases_for(&manifest).await?;
    let real = mecha_core::config::Config::load_global()?;
    let (provider, model) = provider_and_model(&real)?;
    let task_ids: Vec<String> = cases.iter().map(|c| c.id.clone()).collect();
    // The manifest's paths resolve against the checkout `exp run` starts
    // from, and the fixture charter's text is a term of every row's hash.
    let base = std::env::current_dir().context("cannot determine the working directory")?;
    let (planned, skipped) = store.plan(&manifest, &task_ids, &provider, &model, &base)?;
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
        if manifest.kind == TrialKind::Lifetime {
            let s = &manifest.schedule;
            println!(
                "schedule: reflect every {} · validate every {} · learn every {} · retire every {} · ruminate every {} (0 = never)",
                s.reflect, s.validate, s.learn, s.retire, s.ruminate
            );
        }
        for t in &todo {
            println!(
                "{}  arm={} task={} seed={} rep={}{} hash={}",
                t.id,
                t.arm,
                t.task,
                t.seed.map(|s| s.to_string()).unwrap_or_else(|| "-".into()),
                t.repetition,
                t.position.map(|p| format!(" pos={p}")).unwrap_or_default(),
                t.condition_hash
            );
        }
        return Ok(());
    }
    let mecha = std::env::current_exe().context("locating this binary")?;
    let ran = match manifest.kind {
        TrialKind::Single => {
            run_single_trials(&store, &manifest, &mecha, &real, &cases, &todo, limit).await?
        }
        TrialKind::Lifetime => {
            run_lifetimes(&store, &manifest, &mecha, &real, &cases, &planned, limit).await?
        }
    };
    eprintln!("mecha exp `{name}`: {ran} trial(s) run this invocation");
    Ok(())
}

/// The `single` driver: each pending row is one child run in its arm's home.
async fn run_single_trials(
    store: &ExperimentStore,
    manifest: &Manifest,
    mecha: &Path,
    real: &mecha_core::config::Config,
    cases: &[mecha_core::eval::EvalCase],
    todo: &[&Trial],
    limit: Option<usize>,
) -> Result<usize> {
    let mut ran = 0usize;
    for planned_trial in todo {
        if limit.is_some_and(|l| ran >= l) {
            break;
        }
        let mut trial = (*planned_trial).clone();
        let case = cases
            .iter()
            .find(|c| c.id == trial.task)
            .expect("planned from these cases");
        let arm = &manifest.arms[&trial.arm];
        ran += 1;
        eprintln!("· {} ({ran})", trial.id);
        let home = store.arm_home(&trial.arm)?;
        match run_one(store, manifest, mecha, real, arm, case, &home, &mut trial).await {
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
    Ok(ran)
}

/// The `lifetime` driver (Part II §14): one home per arm × seed ×
/// repetition, the sequence walked in position order, and after each task
/// the stages the schedule makes due — minus the arm's stage levers off,
/// minus what the ledger already shows done — run as child `mecha` verbs
/// in that home, sequentially, so a stage never contends with a task for
/// the model server's seats (§18). Resume is the store: a finished row is
/// not rerun, a stage the ledger lacks after a finished position runs
/// before the next task, and a lifetime whose sequence reaches a row this
/// build cannot read stops there rather than running the rest on a broken
/// history. `--limit` counts task runs; the stages due after the last
/// task run still run, because they belong to that position.
async fn run_lifetimes(
    store: &ExperimentStore,
    manifest: &Manifest,
    mecha: &Path,
    real: &mecha_core::config::Config,
    cases: &[mecha_core::eval::EvalCase],
    planned: &[Trial],
    limit: Option<usize>,
) -> Result<usize> {
    use mecha_core::experiment::stages_due;
    // Rows are contiguous per lifetime and in position order as planned;
    // grouped here without reordering so the walk is the design's.
    if !manifest.fixtures.is_empty() {
        let routed = manifest.fixtures.routed();
        eprintln!(
            "mecha exp: fixture world {} · outbox route: {}",
            manifest.fixtures.names().join(", "),
            if routed.is_empty() {
                "none — every fixture send executes unrouted into its store, and no draft will pend"
                    .to_string()
            } else {
                routed.join(", ")
            }
        );
    }
    let mut lifetimes: Vec<(String, Vec<&Trial>)> = Vec::new();
    for t in planned {
        let Some(id) = &t.lifetime else {
            anyhow::bail!(
                "row `{}` carries no lifetime; the plan is not a lifetime's",
                t.id
            );
        };
        match lifetimes.last_mut() {
            Some((last, rows)) if last == id => rows.push(t),
            _ => lifetimes.push((id.clone(), vec![t])),
        }
    }
    let mut ran = 0usize;
    for (lifetime, rows) in lifetimes {
        let first = rows[0];
        let arm = &manifest.arms[&first.arm];
        let stages_off = arm.resolve_stages()?;
        let home = store.lifetime_home(&lifetime)?;
        let (mut ledger, torn) = store.stage_runs(&lifetime)?;
        if torn > 0 {
            eprintln!(
                "mecha exp: {torn} line(s) of `{lifetime}`'s stage ledger could not be read and are counted, not rerun"
            );
        }
        // The arm's CLI-only levers ride as flags, and a stage that runs a
        // model — validate's probes, ruminate's diagnostician — is a run
        // against this home like any task's: the flags go with every stage,
        // or the stages run levers the row's hash says are off (found on
        // review).
        let ChildInvocation {
            flags, passthrough, ..
        } = mecha_core::experiment::child_invocation(real, arm, first.seed)?;
        for planned_trial in rows.iter().copied() {
            let position = planned_trial
                .position
                .context("a lifetime row without a position")?;
            let mut trial = planned_trial.clone();
            let case = cases
                .iter()
                .find(|c| c.id == trial.task)
                .expect("planned from these cases");
            // A due stage — or a principal's act after a task — can run
            // only while no later position has finished: past that it
            // would act on sessions its tasks never ran under, and its
            // success would release the judge's hold on a treatment that
            // did not occur. Recorded skipped instead.
            let late = mecha_core::experiment::out_of_sequence(position, rows.iter().copied());
            // Whether this position's home was rendered for it. False only
            // when the render ahead of the principal failed: the row fails
            // and neither principal point is asked, since an owner act at a
            // position that never had its world is a line the ledger must
            // not carry.
            let mut world_ready = true;
            let mut world_error = String::new();
            match trial.status {
                TrialStatus::Pending | TrialStatus::Running => {
                    if limit.is_some_and(|l| ran >= l) {
                        return Ok(ran);
                    }
                    ran += 1;
                    eprintln!("· {} ({ran}) · {lifetime} position {position}", trial.id);
                    // The principal first: it may script refusals for this
                    // task, which the run child reads from the home — and
                    // its verbs run against the home's config, so the home
                    // is rendered for this task before it is asked. A
                    // failure here **fails the row and asks the principal
                    // nothing at this position**: the first cut printed and
                    // went on, so the owner's verbs ran against a home that
                    // was still the last position's, and the ledger carried
                    // an owner act at a position that never had its world
                    // (found on review).
                    // Rendered here for every lifetime, principal or not: a
                    // render that failed only inside `run_one` left the
                    // stages running against a home with no config for the
                    // position (found on review).
                    if let Err(e) = render_home(manifest, real, arm, trial.seed, &home, false) {
                        world_ready = false;
                        world_error = format!("{e:#}");
                        trial.status = TrialStatus::Failed;
                        trial.error = Some(format!(
                            "the home could not be rendered for this position: {e:#}"
                        ));
                        trial.finished_at = Some(chrono::Utc::now().to_rfc3339());
                        eprintln!("  failed: {e:#}");
                        // The refusal is a record, not a local: a resumed
                        // driver finds the row `Failed`, re-initialises
                        // its own flag and would ask the principal after
                        // the task at a position that never had its
                        // world — unless the ledger already says both
                        // points were skipped for that reason (found on
                        // review).
                        for point in [PrincipalPoint::BeforeTask, PrincipalPoint::AfterTask] {
                            if manifest.principal.is_some()
                                && !principal_done(&ledger, position, point)
                            {
                                let run = unrendered_line(
                                    &lifetime,
                                    &trial.arm,
                                    position,
                                    ExperimentStore::next_attempt(
                                        &ledger,
                                        torn,
                                        position,
                                        StageLever::Principal,
                                    ),
                                    point,
                                    &format!("{e:#}"),
                                );
                                store.record_stage(&run)?;
                                ledger.push(run);
                            }
                        }
                    } else if let Some(principal) = &manifest.principal {
                        if !principal_done(&ledger, position, PrincipalPoint::BeforeTask) {
                            let run = principal_call(
                                store,
                                mecha,
                                &home,
                                &flags,
                                &passthrough,
                                manifest,
                                principal,
                                arm,
                                &trial,
                                case,
                                &lifetime,
                                position,
                                PrincipalPoint::BeforeTask,
                                ExperimentStore::next_attempt(
                                    &ledger,
                                    torn,
                                    position,
                                    StageLever::Principal,
                                ),
                            )
                            .await;
                            store.record_stage(&run)?;
                            ledger.push(run);
                        }
                    }
                    if world_ready {
                        match run_one(store, manifest, mecha, real, arm, case, &home, &mut trial)
                            .await
                        {
                            Ok(()) => {}
                            Err(e) => {
                                trial.status = TrialStatus::Failed;
                                trial.error = Some(format!("{e:#}"));
                                trial.finished_at = Some(chrono::Utc::now().to_rfc3339());
                                eprintln!("  failed: {e:#}");
                            }
                        }
                    }
                    store.save_trial(&trial)?;
                }
                TrialStatus::Done | TrialStatus::Failed => {}
                TrialStatus::Unknown => {
                    eprintln!(
                        "mecha exp: `{lifetime}` stops at position {position}: row `{}` is in a state this build does not know",
                        trial.id
                    );
                    break;
                }
            }
            // The principal after the task: it judges what the run left and
            // closes what gold closes. Due while the task has finished and
            // the ledger lacks its line; skipped, like a stage, once a later
            // position has finished.
            if let Some(principal) = &manifest.principal {
                if world_ready
                    && matches!(trial.status, TrialStatus::Done | TrialStatus::Failed)
                    && !principal_done(&ledger, position, PrincipalPoint::AfterTask)
                {
                    let attempt = ExperimentStore::next_attempt(
                        &ledger,
                        torn,
                        position,
                        StageLever::Principal,
                    );
                    let run = if late {
                        skipped_line(
                            &lifetime,
                            &trial.arm,
                            StageLever::Principal,
                            position,
                            attempt,
                            Some(PrincipalPoint::AfterTask),
                        )
                    } else {
                        principal_call(
                            store,
                            mecha,
                            &home,
                            &flags,
                            &passthrough,
                            manifest,
                            principal,
                            arm,
                            &trial,
                            case,
                            &lifetime,
                            position,
                            PrincipalPoint::AfterTask,
                            attempt,
                        )
                        .await
                    };
                    if run.status == mecha_core::experiment::StageStatus::Skipped {
                        eprintln!(
                            "  ↳ principal (after_task) · skipped: a later position had already finished"
                        );
                    }
                    store.record_stage(&run)?;
                    ledger.push(run);
                }
            }
            for stage in stages_due(&manifest.schedule, position, &stages_off, &ledger) {
                let attempt = ExperimentStore::next_attempt(&ledger, torn, position, stage);
                // A stage after a position whose home was never rendered would
                // run against a home with no config for it — at position 0,
                // with no config at all — and record `Done`; skipped with the
                // render's reason instead, which holds the judge, as a stage
                // that did not run must (found on review).
                if !world_ready {
                    let mut run =
                        skipped_line(&lifetime, &trial.arm, stage, position, attempt, None);
                    run.error = Some(format!(
                        "the home could not be rendered for position {position}, so the stage was not run: {world_error}"
                    ));
                    eprintln!(
                        "  ↳ {} · skipped: the home could not be rendered for this position",
                        stage.as_str()
                    );
                    store.record_stage(&run)?;
                    ledger.push(run);
                    continue;
                }
                if late {
                    let run = skipped_line(&lifetime, &trial.arm, stage, position, attempt, None);
                    eprintln!(
                        "  ↳ {} · skipped: a later position had already finished; a stage after {position} cannot run in sequence",
                        stage.as_str()
                    );
                    store.record_stage(&run)?;
                    ledger.push(run);
                    continue;
                }
                let run = run_stage(
                    store,
                    mecha,
                    &home,
                    &flags,
                    &passthrough,
                    manifest,
                    &trial,
                    &lifetime,
                    stage,
                    position,
                    attempt,
                )
                .await;
                store.record_stage(&run)?;
                ledger.push(run);
            }
        }
    }
    Ok(ran)
}

/// A ledger line for a stage — or a principal's call — the driver could
/// no longer run in sequence.
fn skipped_line(
    lifetime: &str,
    arm: &str,
    stage: StageLever,
    position: u32,
    attempt: u32,
    point: Option<PrincipalPoint>,
) -> mecha_core::experiment::StageRun {
    let now = chrono::Utc::now().to_rfc3339();
    mecha_core::experiment::StageRun {
        lifetime: lifetime.to_string(),
        arm: arm.to_string(),
        stage,
        after_position: position,
        attempt,
        started_at: now.clone(),
        finished_at: now,
        status: mecha_core::experiment::StageStatus::Skipped,
        exit_code: None,
        error: Some(format!(
            "a later position had finished before this could run after {position}; out of sequence, not run"
        )),
        point,
        acts: Vec::new(),
        refusals: Vec::new(),
    }
}

/// The principal's line for a position whose home could not be rendered:
/// skipped, with the render's error, at both points — so `principal_done`
/// holds on every later pass and no owner act is ever asked for at a
/// position that never had its world.
fn unrendered_line(
    lifetime: &str,
    arm: &str,
    position: u32,
    attempt: u32,
    point: PrincipalPoint,
    error: &str,
) -> mecha_core::experiment::StageRun {
    let mut run = skipped_line(
        lifetime,
        arm,
        StageLever::Principal,
        position,
        attempt,
        Some(point),
    );
    run.error = Some(format!(
        "the home could not be rendered for position {position}, so the principal was not asked: {error}"
    ));
    run
}

/// Whether the ledger shows the principal's call at this point done.
fn principal_done(
    ledger: &[mecha_core::experiment::StageRun],
    position: u32,
    point: PrincipalPoint,
) -> bool {
    ledger.iter().any(|r| {
        r.stage == StageLever::Principal
            && r.after_position == position
            && r.point == Some(point)
            && matches!(
                r.status,
                mecha_core::experiment::StageStatus::Done
                    | mecha_core::experiment::StageStatus::Skipped
            )
    })
}

/// The draft a release names, resolved by **the CLI's own selection rule**
/// (`outbox::select`, over every item in the store as `outbox approve`
/// itself resolves it) and then required to be pending — so the item the
/// driver vets is the item the verb will act on, and there is one rule,
/// not a mirror of it (the first cut hand-mirrored the prefix rule, and
/// nothing would have caught the two drifting apart; found on review).
fn release_target(
    items: &[mecha_core::outbox::OutboxItem],
    named: &str,
) -> std::result::Result<mecha_core::outbox::OutboxItem, String> {
    let selection = crate::commands::outbox::Selection {
        ids: vec![named.to_string()],
        ..Default::default()
    };
    let chosen = crate::commands::outbox::select(items.to_vec(), &selection)
        .map_err(|e| format!("{e:#}"))?;
    match chosen.as_slice() {
        [one] if one.status == "pending" => Ok(one.clone()),
        [one] => Err(format!(
            "`{named}` names draft {}, which is {} rather than pending",
            one.id, one.status
        )),
        _ => Err(format!("`{named}` names {} drafts; name one", chosen.len())),
    }
}

/// The principal's call at one point of one position (Part II §16, §21.1):
/// the trial's state on its stdin — the case, the graded row after the
/// task, what the run left in the outbox and the question store — and its
/// answer read back as the owner's verbs to run and the refusals to
/// script. **The driver runs the verbs**, each a child `mecha` against
/// the trial home from the closed set `allowed_verb` names, so the
/// principal is pure and the record is the driver's: every act, with its
/// exit status, is on the ledger line. Never an `Err`: a principal that
/// could not act is a failed line, and `stage_health` holds the verdict
/// over it — a treatment not known to have occurred.
#[allow(clippy::too_many_arguments)]
async fn principal_call(
    store: &ExperimentStore,
    mecha: &Path,
    home: &Path,
    flags: &[String],
    passthrough: &[String],
    manifest: &Manifest,
    principal: &mecha_core::experiment::Principal,
    arm: &mecha_core::experiment::Arm,
    trial: &Trial,
    case: &mecha_core::eval::EvalCase,
    lifetime: &str,
    position: u32,
    point: PrincipalPoint,
    attempt: u32,
) -> mecha_core::experiment::StageRun {
    use mecha_core::experiment::{PrincipalInput, PrincipalOutput, StageRun, StageStatus};
    let started = std::time::Instant::now();
    let mut run = StageRun {
        lifetime: lifetime.to_string(),
        arm: trial.arm.clone(),
        stage: StageLever::Principal,
        after_position: position,
        attempt,
        started_at: chrono::Utc::now().to_rfc3339(),
        finished_at: String::new(),
        status: StageStatus::Running,
        exit_code: None,
        error: None,
        point: Some(point),
        acts: Vec::new(),
        refusals: Vec::new(),
    };
    if let Err(e) = store.record_stage(&run) {
        run.status = StageStatus::Failed;
        run.error = Some(format!("the ledger could not take the running line: {e:#}"));
        run.finished_at = chrono::Utc::now().to_rfc3339();
        return run;
    }
    run.status = StageStatus::Failed;
    let log = store.stage_log(lifetime, position, StageLever::Principal, attempt);
    let workspace = store.stage_workspace(lifetime);
    let reference = ExperimentRef {
        exp_id: manifest.name.clone(),
        trial_id: lifetime.to_string(),
        arm: trial.arm.clone(),
        actor: lifetime.to_string(),
        role: Some("principal".into()),
        task: format!("principal:{}", point.as_str()),
        repetition: trial.repetition,
        condition_hash: trial.condition_hash.clone(),
    };
    // Two environments: the `mecha` children need the provider and search
    // key variables the run child gets; the principal executable — pure by
    // contract, never a model call, a third party named by a path — gets
    // the base set only, on the rule `connect` keeps for MCP servers
    // (found on review).
    let env_for = |cmd: &mut tokio::process::Command, keys: &[String]| -> Result<()> {
        cmd.env_clear();
        for (k, v) in mecha_core::sandbox::Sandbox::child_env(keys) {
            cmd.env(k, v);
        }
        cmd.env("MECHA_HOME", home)
            .env(mecha_core::session::SESSION_KIND_ENV, "experiment")
            .env(EXPERIMENT_REF_ENV, serde_json::to_string(&reference)?)
            .env("MECHA_BIN", mecha)
            .current_dir(&workspace);
        Ok(())
    };
    // The open questions, read once: the principal sees them, and an
    // answer's act is resolved against the same list.
    let open_questions: Vec<mecha_core::questions::Question> = if home.join("questions").is_dir() {
        mecha_core::questions::QuestionStore::open(home.join("questions"))
            .and_then(|s| s.open_items())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    // The pending drafts, read once for the same reason: a release names
    // one, and the driver checks the draft it names against this list. The
    // snapshot predates the principal's answer, and that is safe only
    // because `outbox edit` rewrites a draft's args and never its tool, and
    // a draft resolved between here and the act is refused by the CLI's own
    // pending check — an edit verb that could move a draft's tool would
    // invalidate this vet (found on review). A
    // read: open only what exists, on the doctor's rule that an
    // examination must not create what it was about to report. An
    // unreadable store is a failed call, never an empty queue.
    let outbox_items: Result<Vec<mecha_core::outbox::OutboxItem>> = if home.join("outbox").is_dir()
    {
        mecha_core::outbox::OutboxStore::open(home.join("outbox")).and_then(|s| s.items())
    } else {
        Ok(Vec::new())
    };
    let pending_outbox: Result<Vec<mecha_core::outbox::OutboxItem>> = outbox_items
        .as_ref()
        .map(|items| {
            items
                .iter()
                .filter(|i| i.status == "pending")
                .cloned()
                .collect()
        })
        .map_err(|e| anyhow::anyhow!("{e:#}"));
    // The fixture servers this arm's verbs can reach: the manifest's, or
    // none when the arm has MCP off — `--no-mcp` rides on every verb the
    // driver runs for the principal, so the board and the mailbox are out
    // of reach for that arm and the principal is told so.
    let fixtures: Vec<String> = if flags.iter().any(|f| f == "--no-mcp") {
        Vec::new()
    } else {
        manifest.fixtures.names()
    };
    let outcome: Result<PrincipalOutput> = async {
        // First, before anything else can fail: a principal that failed to
        // answer must not leave the last position's refusals armed for
        // this task — the failure is on the ledger, but the task would
        // have run under a treatment nobody asked for (found on review).
        if point == PrincipalPoint::BeforeTask {
            mecha_core::experiment::write_denials(home, &[])
                .context("clearing the last position's refusals")?;
        }
        std::fs::create_dir_all(&workspace)?;
        if let Some(parent) = log.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let input = PrincipalInput {
            point,
            experiment: manifest.name.clone(),
            lifetime: lifetime.to_string(),
            arm: trial.arm.clone(),
            position,
            home: home.to_path_buf(),
            workspace: workspace.clone(),
            case: case.clone(),
            trial: (point == PrincipalPoint::AfterTask).then(|| trial.clone()),
            pending_outbox: pending_outbox
                .as_ref()
                .map(|v| v.clone())
                .map_err(|e| anyhow::anyhow!("the home's outbox could not be read: {e:#}"))?,
            open_questions: open_questions.clone(),
            fixtures: fixtures.clone(),
        };
        // The manifest's relative file paths — the script, its policy —
        // are written against the checkout `exp run` is started from, and
        // the principal runs from the lifetime's scratch workspace, where
        // they name nothing. Resolved the way a fixture server's are; a
        // command on `PATH` and an absolute path are left alone.
        let mut command = principal.command.clone();
        let base = std::env::current_dir().context("cannot determine the working directory")?;
        mecha_core::experiment::resolve_file_args(&mut command, &base);
        let (exe, args) = command
            .split_first()
            .context("the principal names no executable")?;
        let mut cmd = tokio::process::Command::new(exe);
        cmd.args(args);
        env_for(&mut cmd, &[])?;
        let err =
            std::fs::File::create(&log).with_context(|| format!("creating {}", log.display()))?;
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::from(err));
        // One deadline over the whole exchange — the write of the state,
        // the wait, the read of the answer — driven concurrently, and the
        // child killed when the deadline drops it: a principal that never
        // drained a payload past the pipe buffer used to wedge the driver
        // on a running line with nothing to cancel the write (found on
        // review).
        cmd.kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawning the principal `{exe}`"))?;
        let mut stdin = child.stdin.take().context("the principal's stdin")?;
        let payload = serde_json::to_string(&input)?;
        let exchange = async move {
            use tokio::io::AsyncWriteExt;
            let write = async move {
                stdin.write_all(payload.as_bytes()).await?;
                stdin.shutdown().await?;
                // Dropped here, not at the end of the exchange: the drop is
                // what closes the pipe, and a principal reading its state
                // to end-of-file waits for exactly that (found on smoke).
                drop(stdin);
                Ok::<(), std::io::Error>(())
            };
            let (written, output) = tokio::join!(write, child.wait_with_output());
            written.context("handing the principal its state")?;
            output.context("waiting for the principal")
        };
        let output = match tokio::time::timeout(
            std::time::Duration::from_secs(principal.timeout_secs),
            exchange,
        )
        .await
        {
            Ok(o) => o?,
            Err(_) => anyhow::bail!(
                "the principal did not answer within {}s; its stderr is at {}",
                principal.timeout_secs,
                log.display()
            ),
        };
        anyhow::ensure!(
            output.status.success(),
            "the principal exited {}; its stderr is at {}",
            output.status,
            log.display()
        );
        let text = String::from_utf8_lossy(&output.stdout);
        let start = text.find('{').context("the principal printed no JSON")?;
        serde_json::from_str::<PrincipalOutput>(&text[start..])
            .context("the principal's answer is not the contract's shape")
    }
    .await;
    match outcome {
        Err(e) => run.error = Some(format!("{e:#}")),
        Ok(answer) => {
            let mut failures = Vec::new();
            // Refusals for the task about to run, written for the child;
            // an empty list clears what an earlier position scripted.
            if point == PrincipalPoint::BeforeTask {
                if let Err(e) = mecha_core::experiment::write_denials(home, &answer.deny) {
                    failures.push(format!("the denials file could not be written: {e:#}"));
                }
            } else if !answer.deny.is_empty() {
                failures.push("refusals are scripted before a task, not after it".into());
            }
            for request in answer.acts {
                let mut act = mecha_core::experiment::PrincipalAct::from(request);
                // The arm's fixtures, not the manifest's: under `--no-mcp`
                // the arm reaches none, and both server verbs read the same
                // word off the same scope (found on review).
                if let Err(reason) =
                    mecha_core::experiment::permitted_verb(&act.verb, !fixtures.is_empty())
                {
                    act.ok = Some(false);
                    failures.push(reason);
                    run.acts.push(act);
                    continue;
                }
                // A release is checked against the draft it names: the
                // draft must be pending in this home and its tool must be a
                // fixture server's, by prefix — a builtin sink (`http_fetch`)
                // or an unprefixed server's tool lands somewhere the driver
                // cannot see, so it is refused. `--all` is refused with them:
                // the principal names each draft, and the driver vets each.
                // And the release runs under the *local* `--yes`, the
                // driver's word: a draft written in a tainted conversation
                // confirms on a terminal the driver does not have, and the
                // principal may not carry the flag itself (found on the
                // design).
                if mecha_core::experiment::is_release(&act.verb) {
                    let refusal = match mecha_core::experiment::release_named(&act.verb) {
                        Err(e) => Some(e),
                        Ok(named) => match release_target(
                            outbox_items.as_deref().unwrap_or(&[]),
                            &named,
                        ) {
                            Err(e) => Some(e),
                            Ok(item)
                                if !mecha_core::experiment::release_target_is_fixture(
                                    &item.tool,
                                    &fixtures,
                                ) =>
                            {
                                Some(format!(
                                    "draft {} would execute `{}`, which is not a fixture server's tool ({})",
                                    item.id,
                                    item.tool,
                                    if fixtures.is_empty() {
                                        "this arm reaches none".to_string()
                                    } else {
                                        format!("fixtures: {}", fixtures.join(", "))
                                    }
                                ))
                            }
                            Ok(_) => None,
                        },
                    };
                    if let Some(reason) = refusal {
                        act.ok = Some(false);
                        failures.push(reason);
                        run.acts.push(act);
                        continue;
                    }
                    act.verb.push("--yes".into());
                }
                // A question's answer resumes a run whose jail the question
                // recorded, and the driver's `--workspace` beats the resume's
                // own fallback — so the act names that jail, and an id no
                // open question carries is refused rather than resumed
                // somewhere its run never saw (found on review). Any other
                // act runs from the point's directory: the lifetime's scratch
                // workspace before the task (the trial's is not staged yet),
                // the trial's after it.
                let resumes = act
                    .verb
                    .starts_with(&["questions".to_string(), "answer".to_string()]);
                let act_workspace: Option<PathBuf> = if resumes {
                    act.verb
                        .get(2)
                        .and_then(|id| open_questions.iter().find(|q| &q.id == id))
                        .map(|q| {
                            q.workspace.clone().unwrap_or_else(|| match point {
                                PrincipalPoint::AfterTask => store.workspace_for(&trial.id),
                                _ => workspace.clone(),
                            })
                        })
                } else {
                    Some(match point {
                        PrincipalPoint::AfterTask => store.workspace_for(&trial.id),
                        _ => workspace.clone(),
                    })
                };
                let Some(act_workspace) = act_workspace else {
                    act.ok = Some(false);
                    failures.push(format!(
                        "`{}` names no question open at this position",
                        act.verb.join(" ")
                    ));
                    run.acts.push(act);
                    continue;
                };
                let status: Result<std::process::ExitStatus> = async {
                    let mut cmd = tokio::process::Command::new(mecha);
                    // The driver's options *before* the verb: they are
                    // global, and `tasks steer` takes trailing arguments
                    // that would swallow anything after its text into the
                    // owner's steering message (found on review). The
                    // workspace is named because `[tools] workspace` rides
                    // into the home's config and beats the cwd.
                    // The trial's own workspace and the run's posture: an
                    // act may resume the parked run (`questions answer`),
                    // and a continuation jailed to the scratch directory
                    // failed every fixture read while exiting 0, and one
                    // without `--yes` had every call blocked under the
                    // operator's ask posture — both recorded done (found
                    // on review). `--workspace` and `--yes` are the
                    // driver's, so the principal cannot move either.
                    // Before the task the trial's workspace is not staged
                    // yet (run_one stages it), so an act runs from the
                    // lifetime's scratch workspace; after it, from the
                    // trial's, where the parked continuation lives (found
                    // on review).
                    cmd.arg("--workspace")
                        .arg(&act_workspace)
                        .arg("--yes")
                        .args(flags);
                    // The case's own ceilings, as the run child carries
                    // them: a verb that resumes the parked run is that task
                    // continuing, and a compaction case whose continuation
                    // compacted at the arm's threshold graded the harness
                    // rather than the arm (found on review). Under the same
                    // pins — the arm's overrides and the keys the home's
                    // loop moved.
                    let moved = mecha_core::experiment::home_moved_keys(home)?;
                    let pinned = |key: &str| arm_moves(arm, key) || moved.iter().any(|k| k == key);
                    if let Some(n) = case.max_turns.filter(|_| !pinned("max_turns")) {
                        cmd.arg("--max-turns").arg(n.to_string());
                    }
                    if let Some(n) = case
                        .compact_at_tokens
                        .filter(|_| !pinned("compact_at_tokens"))
                    {
                        cmd.arg("--compact-at").arg(n.to_string());
                    }
                    cmd.args(&act.verb);
                    // `env_for` also sets the child's cwd: the lifetime's
                    // scratch workspace beside the ledger, an empty directory
                    // the driver made, so `Config::load` finds no project
                    // `mecha.toml` there and nothing can replace the home's
                    // `[[mcp]]` under a vetted release — stricter than the
                    // trial's staged workspace, which is a fixture's copy
                    // (questioned on review; the cwd is set, here).
                    env_for(&mut cmd, passthrough)?;
                    // The refusals scripted for this task reach a verb that
                    // resumes the parked run (`questions answer`): the
                    // continuation is the same task under the same
                    // treatment (found on review).
                    let denials = mecha_core::experiment::denials_file(home);
                    if denials.exists() {
                        cmd.env(mecha_core::tool::DENIALS_FILE_ENV, &denials);
                    }
                    let out = std::fs::OpenOptions::new().append(true).open(&log)?;
                    let err = out.try_clone()?;
                    cmd.stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::from(out))
                        .stderr(std::process::Stdio::from(err));
                    // The principal's deadline bounds each verb too, and
                    // the child dies with the dropped future: a resumed run
                    // with no ceiling used to wedge the driver on a running
                    // line (found on review).
                    cmd.kill_on_drop(true);
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(principal.timeout_secs),
                        cmd.status(),
                    )
                    .await
                    {
                        Ok(status) => status
                            .with_context(|| format!("running `mecha {}`", act.verb.join(" "))),
                        Err(_) => anyhow::bail!(
                            "`mecha {}` did not finish within {}s",
                            act.verb.join(" "),
                            principal.timeout_secs
                        ),
                    }
                }
                .await;
                match status {
                    Ok(s) => {
                        act.exit_code = s.code();
                        act.ok = Some(s.success());
                        if !s.success() {
                            failures.push(format!("`mecha {}` exited {s}", act.verb.join(" ")));
                        }
                    }
                    Err(e) => {
                        act.ok = Some(false);
                        failures.push(format!("{e:#}"));
                    }
                }
                run.acts.push(act);
            }
            if failures.is_empty() {
                run.status = StageStatus::Done;
                run.exit_code = Some(0);
            } else {
                run.error = Some(failures.join("; "));
            }
        }
    }
    // After the acts, not before them: an act may resume the parked run on
    // the same session, and a refusal the continuation walked into must
    // count — a refusal that never fired is on the line, and said, rather
    // than recorded like one that did (found on review, twice).
    if point == PrincipalPoint::AfterTask {
        match mecha_core::experiment::read_denials(home) {
            Ok(rules) if !rules.is_empty() => {
                // A session that cannot be read is unknown, never zero: a
                // child that crashed after the model's turns may have
                // walked into every refusal, and the line fails rather
                // than claim it did not (found on review).
                let session_text = trial.session_id.as_ref().and_then(|id| {
                    std::fs::read_to_string(home.join("sessions").join(format!("{id}.jsonl"))).ok()
                });
                run.refusals =
                    mecha_core::experiment::refusal_outcomes(&rules, session_text.as_deref());
                if session_text.is_none() {
                    run.status = StageStatus::Failed;
                    let note = "the task's session could not be read, so the refusals' firings are unknown".to_string();
                    run.error = Some(match run.error.take() {
                        Some(prior) => format!("{prior}; {note}"),
                        None => note,
                    });
                }
                for r in run.refusals.iter().filter(|r| r.fired == Some(0)) {
                    eprintln!(
                        "  ↳ the scripted refusal of `{}` ({}) never fired at position {position}",
                        r.tool, r.reason
                    );
                }
            }
            Ok(_) => {}
            // An unreadable store is a finding: a line that read like a
            // position where nothing was scripted would be the reading
            // this field exists to prevent (found on review).
            Err(e) => {
                run.status = StageStatus::Failed;
                let note = format!("the denials file could not be read back: {e:#}");
                run.error = Some(match run.error.take() {
                    Some(prior) => format!("{prior}; {note}"),
                    None => note,
                });
            }
        }
    }
    run.finished_at = chrono::Utc::now().to_rfc3339();
    eprintln!(
        "  ↳ principal ({}) · {} · {} act(s) · {}s",
        point.as_str(),
        match run.status {
            StageStatus::Done => "ok",
            _ => "FAILED",
        },
        run.acts.len(),
        started.elapsed().as_secs()
    );
    run
}

/// One loop stage as a child `mecha` verb against the lifetime's home, with
/// the run child's environment allowlist and the same session kind, run
/// from a scratch workspace beside the ledger (never from the home, which
/// a path jail refuses to cover), its output on the store. Never an `Err`: a stage that failed is
/// a ledger line saying so, and the lifetime goes on — the failure is part
/// of what the next task started from.
#[allow(clippy::too_many_arguments)]
async fn run_stage(
    store: &ExperimentStore,
    mecha: &Path,
    home: &Path,
    flags: &[String],
    passthrough: &[String],
    manifest: &Manifest,
    after: &Trial,
    lifetime: &str,
    stage: mecha_core::experiment::StageLever,
    position: u32,
    attempt: u32,
) -> mecha_core::experiment::StageRun {
    use mecha_core::experiment::{StageRun, StageStatus};
    let started = std::time::Instant::now();
    let started_at = chrono::Utc::now().to_rfc3339();
    let log = store.stage_log(lifetime, position, stage, attempt);
    let mut run = StageRun {
        lifetime: lifetime.to_string(),
        arm: after.arm.clone(),
        stage,
        after_position: position,
        attempt,
        started_at,
        finished_at: String::new(),
        status: StageStatus::Running,
        exit_code: None,
        error: None,
        point: None,
        acts: Vec::new(),
        refusals: Vec::new(),
    };
    // The running line first, so a driver killed mid-stage leaves a record
    // and the rerun takes the next attempt number rather than this one's
    // log. Its own failure is the stage's: nothing spawns over a ledger
    // that cannot be written.
    if let Err(e) = store.record_stage(&run) {
        run.status = StageStatus::Failed;
        run.error = Some(format!("the ledger could not take the running line: {e:#}"));
        run.finished_at = chrono::Utc::now().to_rfc3339();
        return run;
    }
    run.status = StageStatus::Failed;
    let outcome: Result<std::process::ExitStatus> = async {
        let argv =
            stage_argv(stage, flags).context("a config-switch lever is not a stage to run")?;
        if let Some(parent) = log.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let out =
            std::fs::File::create(&log).with_context(|| format!("creating {}", log.display()))?;
        let err = out.try_clone()?;
        let reference = ExperimentRef {
            exp_id: manifest.name.clone(),
            trial_id: lifetime.to_string(),
            arm: after.arm.clone(),
            actor: lifetime.to_string(),
            role: Some(format!("stage:{}", stage.as_str())),
            task: format!("stage:{}", stage.as_str()),
            repetition: after.repetition,
            condition_hash: after.condition_hash.clone(),
        };
        let workspace = store.stage_workspace(lifetime);
        std::fs::create_dir_all(&workspace)
            .with_context(|| format!("creating {}", workspace.display()))?;
        let mut cmd = tokio::process::Command::new(mecha);
        // Explicit, as the run child's is: `[tools] workspace` rides into
        // the trial home's config verbatim and would beat the cwd, jailing
        // a stage's probes to the operator's project directory instead
        // (found on review).
        cmd.arg("--workspace")
            .arg(&workspace)
            .args(&argv)
            .current_dir(&workspace);
        cmd.env_clear();
        for (k, v) in mecha_core::sandbox::Sandbox::child_env(passthrough) {
            cmd.env(k, v);
        }
        cmd.env("MECHA_HOME", home)
            .env(mecha_core::session::SESSION_KIND_ENV, "experiment")
            .env(EXPERIMENT_REF_ENV, serde_json::to_string(&reference)?)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::from(out))
            .stderr(std::process::Stdio::from(err));
        cmd.status()
            .await
            .with_context(|| format!("spawning mecha {}", argv.join(" ")))
    }
    .await;
    match outcome {
        Ok(status) if status.success() => {
            run.status = StageStatus::Done;
            run.exit_code = status.code();
        }
        Ok(status) => {
            run.exit_code = status.code();
            run.error = Some(format!(
                "exit {}; its output is at {}",
                status,
                log.display()
            ));
        }
        Err(e) => run.error = Some(format!("{e:#}")),
    }
    run.finished_at = chrono::Utc::now().to_rfc3339();
    eprintln!(
        "  ↳ {} · {} · {}s",
        stage.as_str(),
        match run.status {
            StageStatus::Done => "ok",
            _ => "FAILED",
        },
        started.elapsed().as_secs()
    );
    run
}

/// What rendering a trial home for one task leaves the caller: the arm's
/// CLI-only levers as flags, the key variables the child needs, and the
/// knobs the home's own loop moved.
struct Rendered {
    flags: Vec<String>,
    passthrough: Vec<String>,
    moved: Vec<String>,
}

/// Render the home's `config.toml` for the task about to run: the arm
/// (`child_invocation`), then what the home's own stages accepted
/// (`fold_home_overrides`, a lifetime's only), then the manifest's fixtures
/// — the `[[mcp]]` list becomes exactly the fixture servers, each with its
/// store under the home created and seeded once — and the fixture charter
/// over the seeded one — every store fresh from its seed when `fresh`, a
/// `single`'s rule. Called by `run_one`, and by the lifetime driver
/// **before position 0's `before_task` principal call**: a verb the
/// principal asks for there runs against the home's config, and until this
/// wrote one the home had none — so `tasks set` found no board and
/// `outbox approve` no server, at the one position where the fixtures were
/// meant to be reachable first (found on the design).
fn render_home(
    manifest: &Manifest,
    real: &mecha_core::config::Config,
    arm: &mecha_core::experiment::Arm,
    seed: Option<u64>,
    home: &Path,
    fresh: bool,
) -> Result<Rendered> {
    let ChildInvocation {
        mut config,
        flags,
        passthrough,
    } = mecha_core::experiment::child_invocation(real, arm, seed)?;
    // What the home's own stages accepted rides into the next task, under
    // the arm's pins — or a lifetime's `ruminate` would measure as nothing.
    // A single runs no stage and folds nothing.
    let moved = if manifest.kind == TrialKind::Lifetime {
        mecha_core::experiment::fold_home_overrides(&mut config, home, arm)?
    } else {
        Vec::new()
    };
    // The manifest's paths are written against the checkout `exp run` is
    // started from; a server is spawned from the trial's workspace.
    let base = std::env::current_dir().context("cannot determine the working directory")?;
    manifest.fixtures.apply(&mut config, home, &base, fresh)?;
    manifest.fixtures.apply_charter(home, &base)?;
    std::fs::write(home.join("config.toml"), toml::to_string_pretty(&config)?)
        .with_context(|| format!("writing {}", home.join("config.toml").display()))?;
    Ok(Rendered {
        flags,
        passthrough,
        moved,
    })
}

/// One trial: the arm's home and config, a fresh workspace, the child, the
/// grade, the stats. Everything the trial learned is on its row when this
/// returns; a failure anywhere is the row's `error`, never a missing row.
#[allow(clippy::too_many_arguments)]
async fn run_one(
    store: &ExperimentStore,
    manifest: &Manifest,
    mecha: &Path,
    real: &mecha_core::config::Config,
    arm: &mecha_core::experiment::Arm,
    case: &mecha_core::eval::EvalCase,
    home: &Path,
    trial: &mut Trial,
) -> Result<()> {
    let prompt = match &case.prompt {
        Prompt::One(p) => p.clone(),
        Prompt::Many(_) => anyhow::bail!(
            "case `{}` is multi-turn; `exp` drives one prompt per trial today",
            case.id
        ),
    };
    let home = home.to_path_buf();
    let Rendered {
        flags,
        passthrough,
        moved,
    } = render_home(
        manifest,
        real,
        arm,
        trial.seed,
        &home,
        // A `single`'s per-arm home is shared by every trial of the arm, so
        // each starts its fixture stores from the seed; a lifetime carries
        // what the last task left (found on review).
        manifest.kind == TrialKind::Single,
    )?;
    // A knob is pinned for this task if the arm moves it *or* the home's
    // own loop did: the case's ceiling flag below must not override
    // either, since a flag beats the rendered config.
    let pinned = |key: &str| arm_moves(arm, key) || moved.iter().any(|k| k == key);

    let workspace = store.workspace_for(&trial.id);
    if workspace.exists() {
        std::fs::remove_dir_all(&workspace)?;
    }
    mecha_core::eval::stage_workspace(&manifest.tasks.fixture, &workspace)
        .with_context(|| format!("staging {}", manifest.tasks.fixture.display()))?;
    // A task source puts the world in this task's starting state — after
    // the home and its fixture stores are rendered, before the child runs.
    // Its failure is the row's: a task run against a world that was not
    // set up would grade something else under this task's name.
    if manifest.tasks.has_source() {
        let env = mecha_core::experiment::source_env(&home, &workspace, Some(&case.id));
        source_call(
            &manifest.tasks,
            &["setup", &case.id],
            &env,
            &workspace,
            None,
        )
        .await
        .with_context(|| format!("the task source could not set up `{}`", case.id))?;
    }

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
    if let Some(n) = case.max_turns.filter(|_| !pinned("max_turns")) {
        cmd.arg("--max-turns").arg(n.to_string());
    }
    if let Some(n) = case
        .compact_at_tokens
        .filter(|_| !pinned("compact_at_tokens"))
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
        .stdout(std::process::Stdio::piped());
    // The principal's scripted refusals for this task, when it left any:
    // the child's approver answers them ahead of its own, as "Denied by
    // the user" — the owner's word inside the trial home (D12). Armed
    // only under a manifest that has a principal — the file's existence
    // is not the design, and a manifest whose principal was dropped would
    // otherwise resume under the last position's refusals (found on
    // review); with no principal the file is cleared once.
    let denials = mecha_core::experiment::denials_file(&home);
    if manifest.principal.is_some() {
        if denials.exists() {
            cmd.env(mecha_core::tool::DENIALS_FILE_ENV, &denials);
        }
    } else if denials.exists() {
        mecha_core::experiment::write_denials(&home, &[])
            .context("clearing refusals no principal scripted")?;
    }
    cmd.stderr(std::process::Stdio::piped());
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
    // A task source grades the world's end state against the task, with the
    // run's whole result on stdin — the answer text, the calls, the taint,
    // the blocked sends. Its verdict is checks on the row, beside any trace
    // assertion the listing carried; a source that cannot grade fails the
    // row, never passes it.
    if manifest.tasks.has_source() {
        let env = mecha_core::experiment::source_env(&home, &workspace, Some(&case.id));
        let text = source_call(
            &manifest.tasks,
            &["grade", &case.id],
            &env,
            &workspace,
            Some(serde_json::to_string(&result)?),
        )
        .await
        .with_context(|| format!("the task source could not grade `{}`", case.id))?;
        let verdict: mecha_core::experiment::SourceGrade =
            serde_json::from_str(source_json(&text, "grade")?)
                .context("the task source's `grade` is not the contract's shape")?;
        for check in verdict.into_checks()? {
            graded.add_check(check);
        }
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

async fn status(name: &str, json: bool) -> Result<()> {
    let store = ExperimentStore::open_default(name)?;
    let manifest = store.manifest()?;
    // The plan, not the store alone: between `new` and the first `run` the
    // store holds no rows, and an all-zero table reads the same as a design
    // that calls for nothing (found on review). When the case file cannot
    // be read the store's rows stand in, and the readout says so.
    let planned: Result<_> = async {
        let cases = cases_for(&manifest).await?;
        let real = mecha_core::config::Config::load_global()?;
        let (provider, model) = provider_and_model(&real)?;
        let ids: Vec<String> = cases.iter().map(|c| c.id.clone()).collect();
        let base = std::env::current_dir().context("cannot determine the working directory")?;
        store.plan(&manifest, &ids, &provider, &model, &base)
    }
    .await;
    let (trials, skipped) = match planned {
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
    // A lifetime's readout is the trajectory, not the count: each home's
    // sequence by position, and what its stage ledger says ran.
    let lifetimes = lifetime_readout(&store, &trials)?;
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
        let lifetimes: Vec<_> = lifetimes
            .iter()
            .map(|l| {
                serde_json::json!({
                    "id": l.id,
                    "arm": l.arm,
                    "positions": l.positions.iter().map(|(p, t)| serde_json::json!({
                        "position": p, "task": t.task, "status": t.status, "passed": t.passed,
                    })).collect::<Vec<_>>(),
                    "stages_done": l.stages_done,
                    "stages_failed": l.stages_failed,
                    "stages_interrupted": l.stages_interrupted,
                    "stages_skipped": l.stages_skipped,
                    "stages_unknown": l.stages_unknown,
                    "unreadable_stage_lines": l.torn,
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
                "lifetimes": lifetimes,
                "unreadable_trials": skipped,
                // The world the trials ran in: on the readout for the same
                // reason it is in the condition hash.
                "fixtures": manifest.fixtures.names(),
                "outbox_route": manifest.fixtures.routed(),
            }))?
        );
        return Ok(());
    }
    // The world the trials ran in, when it was not the operator's. Below
    // the JSON early return: a prose line ahead of the document is a
    // readout that no longer parses (found on review).
    if !manifest.fixtures.is_empty() {
        let routed = manifest.fixtures.routed();
        println!(
            "fixtures: {} (the operator's servers are not in these homes) · outbox route: {}",
            manifest.fixtures.names().join(", "),
            if routed.is_empty() {
                "none".to_string()
            } else {
                routed.join(", ")
            }
        );
    }
    println!(
        "{} ({:?}, {})",
        manifest.name,
        manifest.kind,
        control_label(&manifest)
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
    if !lifetimes.is_empty() {
        println!(
            "\n{:<32} {:<24} {:>7} {:>7}",
            "lifetime", "positions", "stages", "failed"
        );
        for l in &lifetimes {
            let marks: String = l
                .positions
                .iter()
                .map(|(_, t)| match (t.status, t.passed) {
                    (TrialStatus::Done, Some(true)) => '✓',
                    (TrialStatus::Done, _) => '✗',
                    (TrialStatus::Failed, _) => '!',
                    (TrialStatus::Running, _) => '~',
                    (TrialStatus::Pending, _) => '·',
                    (TrialStatus::Unknown, _) => '?',
                })
                .collect();
            println!(
                "{:<32} {:<24} {:>7} {:>7}{}",
                l.id,
                marks,
                l.stages_done,
                l.stages_failed,
                {
                    let mut notes = Vec::new();
                    if l.stages_interrupted > 0 {
                        notes.push(format!("{} interrupted", l.stages_interrupted));
                    }
                    if l.stages_skipped > 0 {
                        notes.push(format!("{} skipped out of sequence", l.stages_skipped));
                    }
                    if l.stages_unknown > 0 {
                        notes.push(format!(
                            "{} stage line(s) in a status this build cannot read",
                            l.stages_unknown
                        ));
                    }
                    if l.torn > 0 {
                        notes.push(format!("{} ledger line(s) unreadable", l.torn));
                    }
                    if notes.is_empty() {
                        String::new()
                    } else {
                        format!("  ({})", notes.join("; "))
                    }
                }
            );
        }
        println!("(✓ pass · ✗ fail · ! errored · ~ running · · pending)");
    }
    Ok(())
}

/// A stage's full argv: the arm's CLI-only lever flags (`--no-skills`,
/// `--no-charter`, `--no-mcp`, …) *first*, then the verb the nightly
/// runs. The flags are global options and attach before a subcommand as
/// well as after; before is the placement a trailing-argument verb cannot
/// swallow, so every child the driver spawns uses it. `None` for the
/// lever that is a config switch and runs nothing.
fn stage_argv(stage: mecha_core::experiment::StageLever, flags: &[String]) -> Option<Vec<String>> {
    let mut argv: Vec<String> = flags.to_vec();
    argv.extend(stage.argv()?.iter().map(|s| s.to_string()));
    Some(argv)
}

/// One lifetime's sequence and ledger, for `status`.
struct LifetimeReadout {
    id: String,
    arm: String,
    positions: Vec<(u32, Trial)>,
    stages_done: usize,
    stages_failed: usize,
    /// A running line no terminal line superseded: the driver died
    /// mid-stage.
    stages_interrupted: usize,
    /// Due stages the driver could no longer run in sequence.
    stages_skipped: usize,
    /// Lines in a status this build cannot read: a finding, never a stage
    /// that was not scheduled.
    stages_unknown: usize,
    torn: usize,
}

fn lifetime_readout(
    store: &ExperimentStore,
    trials: &std::collections::BTreeMap<String, Trial>,
) -> Result<Vec<LifetimeReadout>> {
    let mut by_id: std::collections::BTreeMap<String, LifetimeReadout> = Default::default();
    for t in trials.values() {
        let (Some(id), Some(pos)) = (&t.lifetime, t.position) else {
            continue;
        };
        by_id
            .entry(id.clone())
            .or_insert_with(|| LifetimeReadout {
                id: id.clone(),
                arm: t.arm.clone(),
                positions: Vec::new(),
                stages_done: 0,
                stages_failed: 0,
                stages_interrupted: 0,
                stages_skipped: 0,
                stages_unknown: 0,
                torn: 0,
            })
            .positions
            .push((pos, t.clone()));
    }
    let mut out: Vec<LifetimeReadout> = by_id.into_values().collect();
    for l in &mut out {
        l.positions.sort_by_key(|(p, _)| *p);
        let (runs, torn) = store.stage_runs(&l.id)?;
        l.torn = torn;
        let h = mecha_core::experiment::stage_health(&runs);
        l.stages_done = h.done;
        l.stages_failed = h.failed;
        l.stages_interrupted = h.interrupted;
        l.stages_skipped = h.skipped;
        l.stages_unknown = h.unknown;
    }
    Ok(out)
}

fn judge_cmd(name: &str, json: bool) -> Result<()> {
    let store = ExperimentStore::open_default(name)?;
    let manifest = store.manifest()?;
    let (trials, skipped) = store.trials()?;
    let trials: Vec<Trial> = trials.into_values().collect();
    let (stages, torn_stages) = store.all_stage_runs()?;
    let verdicts = judge(&manifest, &trials, &stages, torn_stages);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "name": manifest.name,
                "control": manifest.control,
                "arms": verdicts,
                "unreadable_trials": skipped,
                "unreadable_stage_lines": torn_stages,
            }))?
        );
        return Ok(());
    }
    let Some(control) = &manifest.control else {
        println!(
            "{} is a measurement — {} arm(s), no control, nothing to judge; `status` and `export` read it",
            manifest.name,
            manifest.arms.len()
        );
        return Ok(());
    };
    println!("{} — each arm against `{control}`", manifest.name);
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
        if manifest.kind == TrialKind::Lifetime {
            println!(
                "  stages: treatment {} ok · {} failed · {} interrupted · {} skipped · {} unknown    control {} ok · {} failed · {} interrupted · {} skipped · {} unknown{}",
                v.stages.done,
                v.stages.failed,
                v.stages.interrupted,
                v.stages.skipped,
                v.stages.unknown,
                v.control_stages.done,
                v.control_stages.failed,
                v.control_stages.interrupted,
                v.control_stages.skipped,
                v.control_stages.unknown,
                if v.unreadable_stage_lines > 0 {
                    format!(
                        "    {} ledger line(s) unreadable",
                        v.unreadable_stage_lines
                    )
                } else {
                    String::new()
                }
            );
        }
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
    // The ledger rides in the export: for a lifetime it is the evidence
    // that a treatment occurred, and the reviewable object is the whole
    // record (found on review).
    let (stages, torn_stages) = store.all_stage_runs()?;
    let verdicts = judge(&manifest, &trials, &stages, torn_stages);
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "manifest": manifest,
            "trials": trials,
            "stages": stages,
            "judgements": verdicts,
            "unreadable_trials": skipped,
            "unreadable_stage_lines": torn_stages,
        }))?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A position whose home could not be rendered leaves the principal's
    /// two points on the ledger as skipped, so a resumed driver — which
    /// starts with no memory of the failure — sees them done and asks
    /// nothing there.
    #[test]
    fn an_unrendered_position_marks_both_principal_points_done_on_the_ledger() {
        let mut ledger = Vec::new();
        assert!(!principal_done(&ledger, 3, PrincipalPoint::AfterTask));
        for point in [PrincipalPoint::BeforeTask, PrincipalPoint::AfterTask] {
            let run = unrendered_line("full__r1", "full", 3, 1, point, "seed missing");
            assert_eq!(run.status, mecha_core::experiment::StageStatus::Skipped);
            assert_eq!(run.point, Some(point));
            assert!(run
                .error
                .as_deref()
                .unwrap()
                .contains("could not be rendered"));
            assert!(run.acts.is_empty());
            ledger.push(run);
        }
        assert!(principal_done(&ledger, 3, PrincipalPoint::BeforeTask));
        assert!(principal_done(&ledger, 3, PrincipalPoint::AfterTask));
        assert!(
            !principal_done(&ledger, 4, PrincipalPoint::AfterTask),
            "the next position is its own"
        );
    }

    /// A task source's three verbs, driven against the contract's reference
    /// stub: `list` becomes cases with their tags and ceilings, `setup` sees
    /// the pointers and leaves its mark under the fixtures root, `grade`
    /// reads the run's result and answers with checks — and every edge is
    /// fail-closed: an unknown verb, a missing task, a verdict that
    /// disagrees with its checks.
    #[tokio::test]
    async fn a_task_source_lists_sets_up_and_grades_through_the_contract() {
        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(true)
        {
            // A skip reads like a pass in CI; the repo's answer is the
            // variable that turns it into a failure (found on review).
            assert!(
                std::env::var("MECHA_TEST_REQUIRE_BACKENDS").is_err(),
                "python3 is unavailable, and MECHA_TEST_REQUIRE_BACKENDS is set"
            );
            eprintln!("SKIPPED: python3 is unavailable");
            return;
        }
        let stub = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../eval/fixtures/source_stub.py")
            .canonicalize()
            .unwrap();
        let tasks = mecha_core::experiment::Tasks {
            cases: None,
            source: vec!["python3".into(), stub.display().to_string()],
            source_timeout_secs: 30,
            fixture: "eval/workspace".into(),
            ids: Vec::new(),
            tags: Vec::new(),
        };
        let cases = source_list(&tasks).await.unwrap();
        assert_eq!(cases.len(), 2);
        assert_eq!(cases[0].id, "say-hello");
        assert_eq!(
            cases[1].tags,
            vec!["stub".to_string(), "farewell".to_string()]
        );
        assert_eq!(cases[1].max_turns, Some(2), "a ceiling rides along");
        assert!(matches!(&cases[0].prompt, Prompt::One(p) if p.contains("hello")));

        let dir = std::env::temp_dir().join(format!(
            "mecha-source-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        let home = dir.join("home");
        let workspace = dir.join("ws");
        let env = mecha_core::experiment::source_env(&home, &workspace, Some("say-hello"));
        source_call(&tasks, &["setup", "say-hello"], &env, &workspace, None)
            .await
            .unwrap();
        let mark: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(home.join("fixtures").join("stub-setup.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            mark["task"], "say-hello",
            "setup found the fixtures root through the pointer"
        );
        assert_eq!(mark["workspace"], workspace.display().to_string());

        let result = serde_json::json!({"id": "say-hello", "ok": true, "text": "Hello there", "tool_calls": []});
        let text = source_call(
            &tasks,
            &["grade", "say-hello"],
            &env,
            &workspace,
            Some(result.to_string()),
        )
        .await
        .unwrap();
        let verdict: mecha_core::experiment::SourceGrade =
            serde_json::from_str(source_json(&text, "grade").unwrap()).unwrap();
        assert!(verdict.passed);
        let checks = verdict.into_checks().unwrap();
        assert_eq!(checks.len(), 1);
        assert!(checks[0].passed && checks[0].name == "says hello");
        let result = serde_json::json!({"id": "say-hello", "ok": true, "text": "Good day", "tool_calls": []});
        let text = source_call(
            &tasks,
            &["grade", "say-hello"],
            &env,
            &workspace,
            Some(result.to_string()),
        )
        .await
        .unwrap();
        let verdict: mecha_core::experiment::SourceGrade =
            serde_json::from_str(source_json(&text, "grade").unwrap()).unwrap();
        assert!(!verdict.passed && !verdict.into_checks().unwrap()[0].passed);

        // Fail-closed edges.
        let err = source_call(
            &tasks,
            &["grade", "no-such-task"],
            &env,
            &workspace,
            Some("{}".into()),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("exited") && err.contains("no task"), "{err}");
        let err = source_call(&tasks, &["dance"], &env, &workspace, None)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown verb"), "{err}");
        let disagreeing = mecha_core::experiment::SourceGrade {
            passed: true,
            detail: String::new(),
            checks: vec![mecha_core::eval::Check {
                name: "x".into(),
                passed: false,
                detail: String::new(),
            }],
        };
        assert!(disagreeing
            .into_checks()
            .unwrap_err()
            .to_string()
            .contains("disagrees"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A release resolves its draft by the CLI's own rule and then requires
    /// it pending: exact or unique prefix, never a namesake, never a draft
    /// already resolved.
    #[test]
    fn a_release_names_one_pending_draft_by_the_clis_own_rule() {
        let item = |id: &str, status: &str| -> mecha_core::outbox::OutboxItem {
            serde_json::from_value(serde_json::json!({
                "id": id, "status": status, "tool": "mail__mail_send",
                "args_before": {}, "args": {}, "summary": "s", "created_at": "2026-09-05T00:00:00Z"
            }))
            .unwrap()
        };
        let items = vec![
            item("20260905T1-aaaa", "pending"),
            item("20260905T2-bbbb", "pending"),
            item("20260905T3-cccc", "sent"),
        ];
        assert_eq!(
            release_target(&items, "20260905T1-aaaa").unwrap().id,
            "20260905T1-aaaa"
        );
        assert_eq!(
            release_target(&items, "20260905T2").unwrap().id,
            "20260905T2-bbbb",
            "a unique prefix"
        );
        let err = release_target(&items, "20260905T").unwrap_err();
        assert!(
            err.contains("matches 3"),
            "ambiguous over the whole store, as the CLI sees it: {err}"
        );
        let err = release_target(&items, "20260905T3").unwrap_err();
        assert!(err.contains("sent rather than pending"), "{err}");
        assert!(release_target(&items, "nope")
            .unwrap_err()
            .contains("no outbox item"));
        assert!(release_target(&[], "x").is_err());
    }

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

    /// A stage runs with the arm's lever flags after its verb, so the
    /// levers a row's hash says are off are off for the stage too.
    #[test]
    fn a_stage_carries_the_arms_lever_flags() {
        use mecha_core::experiment::StageLever;
        let flags = vec!["--no-skills".to_string(), "--no-mcp".to_string()];
        assert_eq!(
            stage_argv(StageLever::Validate, &flags).unwrap(),
            vec!["--no-skills", "--no-mcp", "validate", "--unprocessed-only"]
        );
        assert_eq!(
            stage_argv(StageLever::Reflect, &[]).unwrap(),
            vec!["reflect"]
        );
        assert_eq!(stage_argv(StageLever::SensorsInBrief, &flags), None);
    }

    /// Every option the principal's verbs are refused is a global option
    /// this CLI really defines — a blocklist that named a field rather
    /// than a flag let the flag through (found on review).
    #[test]
    fn the_blocked_options_are_the_clis_global_options() {
        use clap::CommandFactory;
        let cmd = crate::Cli::command();
        let mut longs = std::collections::BTreeSet::new();
        let mut shorts = std::collections::BTreeSet::new();
        for a in cmd.get_arguments().filter(|a| a.is_global_set()) {
            if let Some(l) = a.get_long() {
                longs.insert(format!("--{l}"));
            }
            if let Some(s) = a.get_short() {
                shorts.insert(format!("-{s}"));
            }
        }
        for name in mecha_core::experiment::PRINCIPAL_BLOCKED_OPTIONS {
            assert!(
                longs.contains(name) || shorts.contains(name),
                "`{name}` is on the blocklist but is not a global option the CLI defines"
            );
        }
        assert!(
            longs.iter().any(|l| l.starts_with("--no-")),
            "the prefix rule covers something"
        );
        // The other direction: every global option is accounted for —
        // blocked, a `--no-` lever, or on the short list known harmless.
        use mecha_core::experiment::{PRINCIPAL_BLOCKED_OPTIONS, PRINCIPAL_HARMLESS_OPTIONS};
        for name in longs.iter().chain(shorts.iter()) {
            let accounted = PRINCIPAL_BLOCKED_OPTIONS.contains(&name.as_str())
                || PRINCIPAL_HARMLESS_OPTIONS.contains(&name.as_str())
                || name.starts_with("--no-");
            assert!(
                accounted,
                "global option `{name}` is neither blocked for a principal's verb nor listed as harmless"
            );
        }
    }

    /// A manifest's `ids` narrow the case file to the tasks it names, in the
    /// manifest's order, and a name the file does not carry is a refusal
    /// rather than a silently smaller experiment.
    #[tokio::test]
    async fn the_tasks_are_the_cases_the_manifest_names() {
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
        let got: Vec<String> = cases_for(&m)
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(
            got,
            vec!["a", "b"],
            "the manifest's order, not the file's: a lifetime's sequence is the design"
        );
        let mut m2 = m.clone();
        m2.tasks.ids = vec!["nope".into()];
        assert!(cases_for(&m2)
            .await
            .unwrap_err()
            .to_string()
            .contains("nope"));
        let mut m3 = m;
        m3.tasks.ids.clear();
        m3.tasks.tags = vec!["t".into()];
        assert_eq!(cases_for(&m3).await.unwrap().len(), 1, "tags narrow too");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
