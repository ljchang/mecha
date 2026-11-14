//! Explicit, synthetic-only entry point for the default-off memory prototype.
use anyhow::{ensure, Context, Result};
use clap::{Args as ClapArgs, Subcommand};
use mecha_core::{
    memory::{
        evaluation, fixture, transcript_source, KeyFilePolicy, Memory, Query, RecallQuery,
        StorageAccess,
    },
    memory_trial_provider::{LocalTrialConfig, LocalTrialProvider},
    provider::Provider,
};
use serde_json::json;
use std::{
    future::Future,
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

#[derive(ClapArgs)]
pub struct Args {
    /// Acknowledge this is a synthetic-data prototype.
    #[arg(long, required = true)]
    pub experimental: bool,
    /// Explicit file ending in .memory-prototype.sqlite; never a live store.
    #[arg(long)]
    pub db: PathBuf,
    /// Owner-selected key file for SQLite memory only. The host stores session JSONL,
    /// task artifacts, provider audits and host-authority files in plaintext.
    #[arg(long)]
    pub key_file: Option<PathBuf>,
    /// Additional directory the key file must be outside. Repeat for multiple roots.
    #[arg(long, requires = "key_file")]
    pub key_exclude_root: Vec<PathBuf>,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Generate a new private key file without opening or creating the database.
    KeyGenerate,
    /// Write a NEW encrypted database snapshot. Does not back up keys, JSONL,
    /// host authority, task artifacts or provider audits.
    Backup {
        #[arg(long)]
        destination: PathBuf,
        #[arg(long)]
        destination_key: PathBuf,
    },
    /// Restore a NEW offline historical encrypted copy using CURRENT deletion/access
    /// authority. Newer facts/withdrawals are not merged; no live installation,
    /// source-file recovery or future cross-copy forgetting is provided.
    Restore {
        #[arg(long)]
        current: PathBuf,
        #[arg(long)]
        current_key: PathBuf,
        #[arg(long)]
        destination: PathBuf,
        #[arg(long)]
        destination_key: PathBuf,
    },
    /// Create an empty synthetic prototype database.
    Init,
    /// Create a NEW v8 synthetic native-evidence store and host authority.
    InitNative {
        #[arg(long, required = true)]
        synthetic_input: bool,
        #[arg(long)]
        domain: String,
        #[arg(long)]
        authority: PathBuf,
    },
    /// Execute one synthetic Test session in a NEW jail; never replay a started root.
    NativeSession {
        #[arg(long, required = true)]
        synthetic_input: bool,
        /// Explicit host-only JSON contract, including frozen criterion/check rows.
        #[arg(long)]
        request: PathBuf,
        /// Optional host-only single-query lexical/dense recall contract.
        #[arg(long)]
        recall_request: Option<PathBuf>,
        /// Optional public output shape for one bounded completion intervention.
        #[arg(long)]
        completion_request: Option<PathBuf>,
        #[arg(long)]
        authority: PathBuf,
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        endpoint: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        holds_dir: PathBuf,
        #[arg(long, default_value_t = 7)]
        seed: u64,
    },
    /// Create a NEW synthetic database with the durable lifecycle schema.
    InitDurable {
        #[arg(long)]
        domain: String,
    },
    /// Create a NEW synthetic database with separately budgeted model decisions.
    InitModel {
        #[arg(long)]
        domain: String,
    },
    /// Create a NEW v5 synthetic database with one separately authorized action.
    InitAction {
        #[arg(long)]
        domain: String,
        #[arg(long, required = true)]
        synthetic_input: bool,
    },
    /// Bind, run or inspect one closed host synthetic action; no model calls.
    DurableAct(DurableActArgs),
    /// Reappraise retained actual evidence under current synthetic contracts; read-only.
    EvaluateOutcomeRecall(OutcomeRecallArgs),
    /// Bind, run or inspect a bounded model decision over a retained guarded read.
    DurableDecide(DurableDecideArgs),
    /// Explicit synthetic guarded-read lifecycle; no model or live-store discovery.
    DurableRead(DurableReadArgs),
    /// Import an explicitly selected synthetic native Mecha JSONL session.
    ImportSession {
        path: PathBuf,
        #[arg(long)]
        account: String,
        /// Acknowledge that the input contains only synthetic test records.
        #[arg(long, required = true)]
        synthetic_input: bool,
    },
    /// Retrieve source evidence with lexical ranking and adjacent records.
    Recall {
        text: String,
        #[arg(long)]
        account: Option<String>,
        #[arg(long, requires = "account")]
        session: Option<String>,
        #[arg(long)]
        known_at: Option<i64>,
        #[arg(long)]
        occurred_after: Option<String>,
        #[arg(long)]
        occurred_before: Option<String>,
        #[arg(long, default_value_t = 5)]
        limit: usize,
        #[arg(long, default_value_t = 1)]
        neighbors: usize,
        /// Budget for serialized evidence; whole records only.
        #[arg(long, default_value_t = 20_000)]
        max_bytes: usize,
    },
    /// Run 32 synthetic evidence-retrieval cases in a NEW database; no model.
    Evaluate,
    /// Replay frozen synthetic searches with explicit mention bindings; no model.
    EvaluateMentions,
    /// Compare lexical, dense and hybrid retrieval in a NEW synthetic fixture.
    EvaluateHybrid {
        #[arg(long, required = true)]
        synthetic_input: bool,
        /// Explicit base URL of the numeric loopback embeddings server.
        #[arg(long)]
        endpoint: String,
        /// Alias that the server must report.
        #[arg(long)]
        expected_alias: String,
        /// Host-declared identity of the embedding weights.
        #[arg(long)]
        artifact_revision: String,
        #[arg(long)]
        dimensions: usize,
        #[arg(long)]
        pooling: String,
        #[arg(long)]
        normalization: String,
    },
    /// Compare retrieval with a resident local model on synthetic cases.
    EvaluateModel {
        /// Explicit loopback llama-server router URL; never loads a model.
        #[arg(long)]
        endpoint: String,
        #[arg(long)]
        model: String,
        /// Existing router coordination directory (e.g. ~/.mecha/holds).
        #[arg(long)]
        holds_dir: PathBuf,
        #[arg(long, default_value_t = 7)]
        seed: u64,
        /// Compare question-only and evidence-informed searches with constrained citations.
        #[arg(long, group = "comparison")]
        follow_evidence: bool,
        /// Compare ignoring versus enforcing one shared source-grounded name plan.
        #[arg(long, group = "comparison")]
        auto_bindings: bool,
        /// Use the selected protocol's previously evaluated development cases.
        #[arg(long, requires = "comparison")]
        development: bool,
    },
    /// Persist synthetic experience and compare none/retrieved/oracle decisions; JSONL output.
    WorkflowTrial {
        #[arg(long, required = true)]
        synthetic_input: bool,
        #[arg(long)]
        endpoint: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        holds_dir: PathBuf,
        #[arg(long, default_value_t = 7)]
        seed: u64,
        /// Compare legacy, measurements-only and scoped historical appraisal views.
        #[arg(long, conflicts_with_all = ["unseen", "checked", "observe"])]
        recall_views: bool,
        /// Fixed measurements view on new synthetic incomplete/corrected evidence cases.
        #[arg(long, conflicts_with_all = ["recall_views", "checked", "observe"])]
        unseen: bool,
        /// Compare a shared initial draft with one source-grounded reconsideration.
        #[arg(long, conflicts_with_all = ["recall_views", "unseen", "observe"])]
        checked: bool,
        /// One bounded observation request followed by a fresh checked decision.
        #[arg(long, conflicts_with_all = ["recall_views", "unseen", "checked"])]
        observe: bool,
        /// Reconsider each checked observation decision at most once.
        #[arg(long, requires = "observe")]
        reconsider: bool,
    },
    /// Translate explicit synthetic passages and plan text; no database access or actions.
    ExtractPlan {
        path: PathBuf,
        #[arg(long, required = true)]
        synthetic_input: bool,
        #[arg(long)]
        endpoint: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        holds_dir: PathBuf,
        #[arg(long, default_value_t = 7)]
        seed: u64,
        /// Request reasoning; off by default. Report counts instead of reasoning text.
        #[arg(long)]
        thinking: bool,
        /// Per-request reasoning cap (1-4096), leaving at least 4096 of 8192 tokens for the answer.
        /// Does not alter shared server settings.
        #[arg(long, requires = "thinking", value_parser = clap::value_parser!(u32).range(1..=4096))]
        thinking_budget: Option<u32>,
        /// Compare a direct coverage judgment against structured extraction.
        #[arg(long)]
        direct: bool,
    },
    /// Assess synthetic candidate plans and optionally revise/reassess once.
    EvaluatePlans {
        /// Explicit JSON: query, event, goal, candidates [{id, steps}].
        path: PathBuf,
        #[arg(long, required = true)]
        synthetic_input: bool,
        #[arg(long)]
        account: String,
        #[arg(long)]
        endpoint: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        holds_dir: PathBuf,
        #[arg(long, default_value_t = 7)]
        seed: u64,
        /// Repair/reassess once only if no candidate passes; never execute actions.
        #[arg(long)]
        revise: bool,
    },
    /// Generate and retain one provisional concept from explicit synthetic sources.
    DistillConcept {
        /// Source object:revision from query/recall output. Repeat for 2..=8 records.
        #[arg(long, required = true)]
        source: Vec<String>,
        /// Existing concept object:revision to replace; stale revisions are refused.
        #[arg(long)]
        revise: Option<String>,
        #[arg(long)]
        endpoint: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        holds_dir: PathBuf,
        #[arg(long, default_value_t = 7)]
        seed: u64,
    },
    /// Purge one imported record locally; leaves the input JSONL unchanged.
    ForgetSessionRecord {
        #[arg(long)]
        account: String,
        #[arg(long)]
        session: String,
        #[arg(long)]
        line: usize,
    },
    /// Create a new database containing the synthetic temporal fixture.
    Demo,
    /// Read the current snapshot identifier.
    Inspect,
    /// Search records; valid-time filtering selects grounded relations only.
    Query {
        text: String,
        /// Revision sequence to read (defaults to the current snapshot).
        #[arg(long)]
        known_at: Option<i64>,
        /// Time the relation must hold, as RFC3339.
        #[arg(long)]
        valid_at: Option<String>,
        /// Include relations whose temporal bounds are unknown.
        #[arg(long)]
        include_possible: bool,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Purge a fixture source and dependent revisions, including old snapshots.
    Forget { source_id: String },
}

#[derive(ClapArgs)]
pub struct DurableDecideArgs {
    #[arg(long, required = true)]
    synthetic_input: bool,
    #[arg(long)]
    domain: String,
    #[arg(long, required = true)]
    scope: Vec<String>,
    #[command(subcommand)]
    cmd: DurableDecideCommand,
}
#[derive(Clone, Copy, clap::ValueEnum)]
pub enum DecisionView {
    None,
    Observed,
}
#[derive(Subcommand)]
pub enum DurableDecideCommand {
    /// Seal a task contract and model settings; does not call a model.
    Bind {
        #[arg(long)]
        read: String,
        /// Inline strict JSON task contract; never an inferred goal.
        #[arg(long)]
        contract: String,
        #[arg(long)]
        model: String,
        #[arg(long, default_value_t = 7)]
        seed: u64,
        #[arg(long, value_enum)]
        view: DecisionView,
    },
    /// Resume without replaying any spent model attempt; never executes a recommendation.
    Run {
        #[arg(long)]
        task: String,
        #[arg(long)]
        endpoint: String,
        #[arg(long)]
        holds_dir: PathBuf,
    },
    Status {
        #[arg(long)]
        task: String,
    },
    Cancel {
        #[arg(long)]
        task: String,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum OutcomeResponseFormat {
    ChoiceFirst,
    FeasibilityFirst,
    FeasibilityProvenance,
}
impl From<OutcomeResponseFormat> for mecha_core::memory_outcome_reappraisal::ResponseFormat {
    fn from(value: OutcomeResponseFormat) -> Self {
        match value {
            OutcomeResponseFormat::ChoiceFirst => Self::ChoiceFirst,
            OutcomeResponseFormat::FeasibilityFirst => Self::FeasibilityFirst,
            OutcomeResponseFormat::FeasibilityProvenance => Self::FeasibilityProvenance,
        }
    }
}
#[derive(ClapArgs)]
pub struct OutcomeRecallArgs {
    #[arg(long, required = true)]
    synthetic_input: bool,
    #[arg(long)]
    domain: String,
    #[arg(long, required = true)]
    scope: Vec<String>,
    #[arg(long)]
    action: String,
    /// Strict inline JSON current contract, not inferred from the historical goal.
    #[arg(long)]
    contract: String,
    #[arg(long)]
    query: String,
    #[arg(long)]
    model: String,
    #[arg(long, default_value_t = 7)]
    seed: u64,
    #[arg(long)]
    outcome_first: bool,
    /// Opt into a diagnostic format (v3) or feasibility-provenance (v4); omission preserves legacy request bytes.
    #[arg(long, value_enum)]
    response_format: Option<OutcomeResponseFormat>,
    /// Per-request reasoning cap, within the 8192-token total allowance.
    #[arg(long, requires = "response_format", value_parser = clap::value_parser!(u32).range(1..=4096))]
    thinking_budget: Option<u32>,

    /// Original exported Snapshot JSON; required for inference, reused for every later preparation.
    #[arg(long, required_unless_present = "prepare_only")]
    snapshot: Option<String>,
    /// Emit the exact frozen pair without provider discovery or setup.
    #[arg(long)]
    prepare_only: bool,
    #[arg(long, required_unless_present = "prepare_only")]
    endpoint: Option<String>,
    #[arg(long, required_unless_present = "prepare_only")]
    holds_dir: Option<PathBuf>,
}

#[derive(ClapArgs)]
pub struct DurableActArgs {
    #[arg(long, required = true)]
    synthetic_input: bool,
    #[arg(long)]
    domain: String,
    #[arg(long, required = true)]
    scope: Vec<String>,
    #[command(subcommand)]
    cmd: DurableActCommand,
}
#[derive(Subcommand)]
pub enum DurableActCommand {
    /// Explicitly authorize a closed fixture for an authenticated recommendation.
    Bind {
        #[arg(long)]
        model: String,
        /// Strict JSON: closed fixture version and state; no path or choice.
        #[arg(long)]
        spec: String,
    },
    /// Resume one synthetic handler and feedback; never replay an unresolved action.
    Run {
        #[arg(long)]
        task: String,
    },
    Status {
        #[arg(long)]
        task: String,
    },
    Cancel {
        #[arg(long)]
        task: String,
    },
}

#[derive(ClapArgs)]
pub struct DurableReadArgs {
    #[arg(long, required = true)]
    synthetic_input: bool,
    #[arg(long)]
    domain: String,
    /// Current host-authorized scopes; must match the original task context.
    #[arg(long, required = true)]
    scope: Vec<String>,
    #[command(subcommand)]
    cmd: DurableReadCommand,
}

#[derive(Subcommand)]
pub enum DurableReadCommand {
    /// Bind a synthetic file and existing source revisions; does not read the file.
    Start {
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long)]
        file: PathBuf,
        #[arg(long, required = true)]
        source: Vec<String>,
        #[arg(long, default_value_t = 16_384)]
        max_bytes: usize,
        #[arg(long, default_value_t = 128)]
        max_lines: usize,
    },
    /// Acquire once or reuse the stored result; interrupted acquisitions are not replayed.
    Resume {
        #[arg(long)]
        task: String,
        /// Reassert the current host workspace; a stored task does not grant access.
        #[arg(long)]
        workspace: PathBuf,
    },
    /// Retain one untrusted proposal against the original evidence; not a success verdict.
    Finish {
        #[arg(long)]
        task: String,
        #[arg(long)]
        proposal: String,
    },
    /// Show operational state without acquiring or reconstructing evidence.
    Status {
        #[arg(long)]
        task: String,
    },
    /// Cancel future work while retaining an already observed result until forgetting.
    Cancel {
        #[arg(long)]
        task: String,
    },
}

fn key_policy(args: &Args) -> Result<KeyFilePolicy> {
    let mut roots = args.key_exclude_root.clone();
    roots.push(std::env::current_dir()?.canonicalize()?);
    roots.push(
        args.db
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
    );
    match &args.cmd {
        Cmd::InitNative { authority, .. } => roots.push(authority.clone()),
        Cmd::Restore {
            current,
            destination,
            ..
        } => {
            for path in [current, destination] {
                roots.push(
                    path.parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .unwrap_or(Path::new("."))
                        .to_path_buf(),
                );
            }
        }
        Cmd::Backup { destination, .. } => roots.push(
            destination
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .to_path_buf(),
        ),
        Cmd::NativeSession {
            authority, root, ..
        } => {
            roots.push(authority.clone());
            roots.push(root.clone());
        }
        Cmd::KeyGenerate
        | Cmd::Inspect
        | Cmd::Query { .. }
        | Cmd::Recall { .. }
        | Cmd::Forget { .. }
        | Cmd::ForgetSessionRecord { .. } => {}
        Cmd::Init
        | Cmd::InitDurable { .. }
        | Cmd::InitModel { .. }
        | Cmd::InitAction { .. }
        | Cmd::DurableAct(_)
        | Cmd::EvaluateOutcomeRecall(_)
        | Cmd::DurableDecide(_)
        | Cmd::DurableRead(_)
        | Cmd::ImportSession { .. }
        | Cmd::Evaluate
        | Cmd::EvaluateMentions
        | Cmd::EvaluateHybrid { .. }
        | Cmd::EvaluateModel { .. }
        | Cmd::WorkflowTrial { .. }
        | Cmd::ExtractPlan { .. }
        | Cmd::EvaluatePlans { .. }
        | Cmd::DistillConcept { .. }
        | Cmd::Demo => {
            anyhow::bail!("this memory command does not support --key-file");
        }
    }
    KeyFilePolicy::new(&roots)
}

pub async fn execute(args: Args) -> Result<()> {
    execute_with_native_provider(args, |config, cancel| async move {
        Ok(Box::new(LocalTrialProvider::new(config, cancel).await?) as Box<dyn Provider>)
    })
    .await
}

async fn execute_with_native_provider<F, Fut>(args: Args, native_provider: F) -> Result<()>
where
    F: FnOnce(LocalTrialConfig, CancellationToken) -> Fut,
    Fut: Future<Output = Result<Box<dyn Provider>>>,
{
    ensure!(args.experimental, "--experimental is required");
    let policy = args
        .key_file
        .as_ref()
        .map(|_| key_policy(&args))
        .transpose()?;
    let key_selection = args.key_file.as_deref().zip(policy.as_ref());
    let storage = match (key_selection, &args.cmd) {
        (Some((key, policy)), command) if !matches!(command, Cmd::KeyGenerate) => {
            StorageAccess::load_key_file(key, policy)?
        }
        _ => StorageAccess::plaintext_fixture(),
    };
    let open_read_only = || Memory::open_read_only_with_storage(&args.db, storage.clone());
    let result = match args.cmd {
        Cmd::KeyGenerate => {
            let (key, policy) = key_selection.context("--key-file is required for key-generate")?;
            StorageAccess::generate_key_file(key, policy)?;
            json!({"key_created":true,"database_created":false})
        }
        Cmd::Backup {
            destination,
            destination_key,
        } => {
            ensure!(
                storage.is_encrypted(),
                "backup requires --key-file and an encrypted source"
            );
            let destination_storage = StorageAccess::load_key_file(
                &destination_key,
                policy.as_ref().context("backup key policy missing")?,
            )?;
            serde_json::to_value(
                open_read_only()?.backup_encrypted(&destination, destination_storage)?,
            )?
        }
        Cmd::Restore {
            current,
            current_key,
            destination,
            destination_key,
        } => {
            use mecha_core::memory::{restore_encrypted, RestoreAuthority};
            ensure!(
                storage.is_encrypted(),
                "restore requires --key-file and an encrypted backup"
            );
            let policy = policy.as_ref().context("restore key policy missing")?;
            let current_storage = StorageAccess::load_key_file(&current_key, policy)?;
            let destination_storage = StorageAccess::load_key_file(&destination_key, policy)?;
            let authority = RestoreAuthority::current(&current, current_storage);
            serde_json::to_value(restore_encrypted(
                &args.db,
                storage.clone(),
                &destination,
                destination_storage,
                &authority,
            )?)?
        }
        Cmd::Init => {
            let memory = Memory::create(&args.db, "synthetic-demo")?;
            json!({"snapshot":memory.snapshot()?,"fixture_only":true})
        }
        Cmd::InitNative {
            synthetic_input,
            domain,
            authority,
        } => {
            use mecha_core::memory_experience_bridge::LocalAuthority;
            ensure!(synthetic_input, "--synthetic-input is required");
            ensure!(
                args.db.is_absolute()
                    && authority.is_absolute()
                    && !args.db.exists()
                    && !authority.exists(),
                "native initialization requires NEW absolute database and authority paths"
            );
            let memory =
                Memory::create_supported_situated_with_storage(&args.db, &domain, storage.clone())?;
            LocalAuthority::create(&authority)
                .context("native database created but host authority initialization failed")?;
            json!({"snapshot":memory.snapshot()?,"fixture_only":true,"native_capture":true,"authority":authority})
        }
        Cmd::NativeSession {
            synthetic_input,
            request,
            recall_request,
            completion_request,
            authority,
            root,
            endpoint,
            model,
            holds_dir,
            seed,
        } => {
            use mecha_core::{
                memory_experience_bridge::LocalAuthority,
                memory_situated_workflow::{
                    run_native_session_with_completion, run_native_session_with_recall,
                    run_native_session_with_storage, NativeCompletionRequest, NativeRecallRequest,
                    NativeSessionRequest,
                },
            };
            use std::io::Read;
            ensure!(synthetic_input, "--synthetic-input is required");
            ensure!(
                request.is_absolute() && authority.is_absolute(),
                "explicit absolute request and authority paths required"
            );
            let mut bytes = Vec::new();
            std::fs::File::open(request)?
                .take(1_048_577)
                .read_to_end(&mut bytes)?;
            ensure!(bytes.len() <= 1_048_576, "oversized native request file");
            let request: NativeSessionRequest = serde_json::from_slice(&bytes)?;
            let recall = recall_request
                .map(|path| -> Result<_> {
                    ensure!(
                        path.is_absolute(),
                        "explicit absolute recall request path required"
                    );
                    let mut bytes = Vec::new();
                    std::fs::File::open(path)?
                        .take(1_048_577)
                        .read_to_end(&mut bytes)?;
                    ensure!(
                        bytes.len() <= 1_048_576,
                        "oversized native recall request file"
                    );
                    serde_json::from_slice::<NativeRecallRequest>(&bytes)?.into_config()
                })
                .transpose()?;
            let completion = completion_request
                .map(|path| -> Result<_> {
                    ensure!(
                        path.is_absolute(),
                        "explicit absolute completion request path required"
                    );
                    let mut bytes = Vec::new();
                    std::fs::File::open(path)?
                        .take(1_048_577)
                        .read_to_end(&mut bytes)?;
                    ensure!(
                        bytes.len() <= 1_048_576,
                        "oversized native completion request file"
                    );
                    serde_json::from_slice::<NativeCompletionRequest>(&bytes)?.into_config()
                })
                .transpose()?;
            let authority = LocalAuthority::open(&authority)?;
            let cancel = CancellationToken::new();
            let signal_cancel = cancel.clone();
            let signal = tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    signal_cancel.cancel();
                }
            });
            let factory = |run_cancel| {
                native_provider(
                    LocalTrialConfig {
                        endpoint,
                        model,
                        holds_dir,
                        seed,
                    },
                    run_cancel,
                )
            };
            let result = if let Some(completion) = completion {
                run_native_session_with_completion(
                    &args.db,
                    storage.clone(),
                    &authority,
                    &request,
                    recall,
                    completion,
                    &root,
                    factory,
                    &cancel,
                )
                .await
            } else if let Some(recall) = recall {
                run_native_session_with_recall(
                    &args.db,
                    storage.clone(),
                    &authority,
                    &request,
                    recall,
                    &root,
                    factory,
                    &cancel,
                )
                .await
            } else {
                run_native_session_with_storage(
                    &args.db,
                    storage.clone(),
                    &authority,
                    &request,
                    &root,
                    factory,
                    &cancel,
                )
                .await
            };
            signal.abort();
            let report_path = root.join("report.json");
            let report = result.with_context(|| {
                format!(
                    "native session failed; task report: {}",
                    report_path.display()
                )
            })?;
            let value = serde_json::to_value(&report)?;
            let recall_failed = report
                .recall
                .as_ref()
                .is_some_and(|recall| !recall.protocol_satisfied());
            let completion_failed = report
                .completion
                .as_ref()
                .is_some_and(|completion| !completion.protocol_satisfied());
            if report.run_error.is_some()
                || report.capture_error.is_some()
                || recall_failed
                || completion_failed
            {
                println!("{}", serde_json::to_string_pretty(&value)?);
                let mut recall_diagnostic = report.recall.as_ref().map_or_else(String::new, |_| {
                    format!("; recall protocol failed: {recall_failed}")
                });
                if report.completion.is_some() {
                    recall_diagnostic.push_str(&format!(
                        "; completion protocol failed: {completion_failed}"
                    ));
                }
                anyhow::bail!(
                    "native session incomplete; inspect {} (execution: {}; capture: {}{})",
                    report_path.display(),
                    report.run_error.as_deref().unwrap_or("recorded"),
                    report.capture_error.as_deref().unwrap_or("recorded"),
                    recall_diagnostic
                );
            }
            value
        }
        Cmd::InitDurable { domain } => {
            let memory = Memory::create_durable(&args.db, &domain)?;
            json!({"snapshot":memory.snapshot()?,"fixture_only":true,"durable":true})
        }
        Cmd::InitModel { domain } => {
            let memory = Memory::create_durable_model(&args.db, &domain)?;
            json!({"snapshot":memory.snapshot()?,"fixture_only":true,"durable_model":true})
        }
        Cmd::InitAction {
            domain,
            synthetic_input,
        } => {
            ensure!(synthetic_input, "--synthetic-input is required");
            let memory = Memory::create_durable_action(&args.db, &domain)?;
            json!({"snapshot":memory.snapshot()?,"fixture_only":true,"durable_action":true,"model_called":false})
        }
        Cmd::DurableAct(action) => execute_durable_act(&args.db, action)?,
        Cmd::EvaluateOutcomeRecall(input) => execute_outcome_recall(&args.db, input).await?,
        Cmd::DurableDecide(decision) => execute_durable_decide(&args.db, decision).await?,
        Cmd::DurableRead(read) => execute_durable_read(&args.db, read).await?,
        Cmd::ImportSession {
            path,
            account,
            synthetic_input,
        } => {
            ensure!(synthetic_input, "--synthetic-input is required");
            let canonical = path.canonicalize()?;
            ensure!(
                !canonical
                    .components()
                    .any(|c| c.as_os_str() == ".mecha" || c.as_os_str() == ".mecha-graph"),
                "live Mecha directories are not synthetic inputs"
            );
            let input = std::fs::File::open(canonical)?;
            ensure!(input.metadata()?.is_file(), "expected a regular JSONL file");
            let mut memory = Memory::open(&args.db)?;
            serde_json::to_value(memory.import_session_jsonl(
                input,
                &account,
                fixture::access().scopes,
            )?)?
        }
        Cmd::Recall {
            text,
            account,
            session,
            known_at,
            occurred_after,
            occurred_before,
            limit,
            neighbors,
            max_bytes,
        } => {
            let memory = open_read_only()?;
            let mut snapshot = memory.snapshot()?;
            if let Some(sequence) = known_at {
                snapshot.sequence = sequence;
            }
            serde_json::to_value(memory.recall(
                &RecallQuery {
                    text,
                    known_at: snapshot,
                    account,
                    session_id: session,
                    occurred_after,
                    occurred_before,
                    limit,
                    neighbors,
                    max_bytes,
                },
                &fixture::access(),
            )?)?
        }
        Cmd::EvaluateModel {
            endpoint,
            model,
            holds_dir,
            seed,
            follow_evidence,
            auto_bindings,
            development,
        } => {
            use mecha_core::memory::model_evaluation::followup::Split;
            let split = (follow_evidence || auto_bindings).then_some(if development {
                Split::Development
            } else {
                Split::Holdout
            });
            evaluate_model(
                &args.db,
                &endpoint,
                &model,
                holds_dir,
                seed,
                split,
                auto_bindings,
            )
            .await?
        }
        Cmd::WorkflowTrial {
            synthetic_input,
            endpoint,
            model,
            holds_dir,
            seed,
            recall_views,
            unseen,
            checked,
            observe,
            reconsider,
        } => {
            use std::io::Write;
            ensure!(synthetic_input, "--synthetic-input is required");
            let mut client = LocalMemoryModel::new(&endpoint, &model, holds_dir, seed, true)?;
            client.pass = mecha_core::quarantine::QuarantinedPass::new(&model, 2048)
                .no_thinking()
                .local_only();
            use mecha_core::memory::experience::Trial;
            let trial = if recall_views {
                Trial::RecallViews
            } else if checked {
                Trial::Checked
            } else if unseen {
                Trial::Unseen
            } else {
                Trial::Baseline
            };
            let report = if observe {
                serde_json::to_value(
                    mecha_core::memory::experience::observations::run_with_reconsideration(
                        &args.db,
                        &model,
                        reconsider,
                        |prompt, schema| client.complete(prompt, schema),
                        |row| {
                            println!("{}", json!({"type":"row","row":row}));
                            std::io::stdout().flush()?;
                            Ok(())
                        },
                    )
                    .await?,
                )?
            } else {
                serde_json::to_value(
                    mecha_core::memory::experience::run_trial(
                        &args.db,
                        &model,
                        trial,
                        |prompt, schema| client.complete(prompt, schema),
                        |row| {
                            println!("{}", json!({"type":"row","row":row}));
                            std::io::stdout().flush()?;
                            Ok(())
                        },
                    )
                    .await?,
                )?
            };
            println!(
                "{}",
                json!({"type":"report","report":report,"transport":{
                "endpoint":endpoint,"model":model,"seed":seed,"temperature":0.6,
                "thinking":false,"max_tokens":2048,"request_timeout_seconds":180,"retries":0}})
            );
            return Ok(());
        }
        Cmd::ExtractPlan {
            path,
            synthetic_input,
            endpoint,
            model,
            holds_dir,
            seed,
            thinking,
            thinking_budget,
            direct,
        } => {
            use mecha_core::memory::plan_extraction;
            use std::{io::Read, time::Instant};
            ensure!(synthetic_input, "--synthetic-input is required");
            let path = path.canonicalize()?;
            ensure!(
                !path
                    .components()
                    .any(|c| c.as_os_str() == ".mecha" || c.as_os_str() == ".mecha-graph"),
                "live Mecha directories are not synthetic inputs"
            );
            let file = std::fs::File::open(path)?;
            ensure!(
                file.metadata()?.is_file(),
                "expected regular synthetic JSON file"
            );
            let mut bytes = Vec::new();
            file.take(32_769).read_to_end(&mut bytes)?;
            ensure!(bytes.len() <= 32_768, "extraction input exceeds 32 KiB");
            let prepared = plan_extraction::prepare(serde_json::from_slice(&bytes)?)?;
            let request = if direct {
                prepared.direct_request()
            } else {
                prepared.request()
            };
            let mut client = LocalMemoryModel::new(&endpoint, &model, holds_dir, seed, true)?;
            client.pass = client.pass.think(thinking);
            let transport = json!({"endpoint":endpoint,"seed":seed,"temperature":0.6,"thinking":thinking,"thinking_budget":thinking_budget,"max_tokens":8192,"request_timeout_seconds":180,"retries":0});
            let start = Instant::now();
            let mut calls = Vec::new();
            let result=async {
                let (reply, reasoning_chars)=client.complete_audited(request.prompt,Some(request.response_schema),thinking_budget).await?;
                calls.push(json!({"reasoning_chars":reasoning_chars,"model":reply.model,"input_tokens":reply.input_tokens,"output_tokens":reply.output_tokens,"elapsed_ms":start.elapsed().as_millis(),"untrusted_model_output":reply.text,"error":reply.error}));
                ensure!(reply.model==model,"extraction response model differs from requested resident model");
                ensure!(reply.error.is_none(),"extraction generation failed: {}",reply.error.as_deref().unwrap_or("unknown"));
                if direct { serde_json::to_value(plan_extraction::finish_direct(&prepared,&reply.text)?) .map_err(Into::into) } else { serde_json::to_value(plan_extraction::finish(&prepared,&reply.text)?) .map_err(Into::into) }
            }.await;
            let report = match result {
                Ok(report) => report,
                Err(error) => {
                    println!(
                        "{}",
                        json!({"fixture_only":true,"extraction":null,"direct_judgment":null,"calls":calls,"transport":transport,"error":format!("{error:#}")})
                    );
                    return Err(error);
                }
            };
            json!({"fixture_only":true,"extraction":if direct { serde_json::Value::Null } else { report.clone() },"direct_judgment":if direct { report } else { serde_json::Value::Null },"calls":calls,"transport":transport})
        }
        Cmd::EvaluatePlans {
            path,
            synthetic_input,
            account,
            endpoint,
            model,
            holds_dir,
            seed,
            revise,
        } => {
            use mecha_core::memory::{
                context::{ContextPolicy, ContextRequest, Situation},
                plan_assessment::Candidate,
            };
            use std::{
                io::Read,
                sync::{Arc, Mutex},
                time::Instant,
            };
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Input {
                query: String,
                event: String,
                goal: String,
                candidates: Vec<Candidate>,
            }
            ensure!(synthetic_input, "--synthetic-input is required");
            ensure!(
                !account.trim().is_empty() && account.len() <= 256,
                "invalid synthetic account"
            );
            let path = path.canonicalize()?;
            ensure!(
                !path
                    .components()
                    .any(|c| c.as_os_str() == ".mecha" || c.as_os_str() == ".mecha-graph"),
                "live Mecha directories are not synthetic inputs"
            );
            let file = std::fs::File::open(path)?;
            ensure!(
                file.metadata()?.is_file(),
                "expected a regular JSON input file"
            );
            let mut bytes = Vec::new();
            file.take(20_001).read_to_end(&mut bytes)?;
            ensure!(bytes.len() <= 20_000, "plan input exceeds 20000 bytes");
            let input: Input = serde_json::from_slice(&bytes)?;
            let _: mecha_core::goal::GoalRef = input.goal.parse()?;
            let memory = Memory::open_read_only(&args.db)?;
            let policy = ContextPolicy {
                access: fixture::access(),
                recall: RecallQuery {
                    text: String::new(),
                    known_at: memory.snapshot()?,
                    account: Some(account),
                    session_id: None,
                    occurred_after: None,
                    occurred_before: None,
                    limit: 5,
                    neighbors: 0,
                    max_bytes: 20_000,
                },
                valid_at: Some(chrono::Utc::now().to_rfc3339()),
            };
            let request = ContextRequest {
                query: input.query,
                bindings: vec![],
            };
            let situation = Situation {
                event: input.event,
                goal: input.goal,
                needs: None,
            };
            let client = LocalMemoryModel::new(&endpoint, &model, holds_dir, seed, true)?;
            let calls = Arc::new(Mutex::new(Vec::new()));
            let report=memory.evaluate_plans(&request,&policy,&situation,&input.candidates,revise,|call|{
                let client=&client;let calls=calls.clone();let expected=&model;
                async move {
                    let start=Instant::now();
                    let reply=client.complete(call.prompt,Some(call.response_schema)).await?;
                    calls.lock().map_err(|_|anyhow::anyhow!("assessment metrics unavailable"))?.push(json!({"stage":call.stage,"model":reply.model,"input_tokens":reply.input_tokens,"output_tokens":reply.output_tokens,"elapsed_ms":start.elapsed().as_millis(),"untrusted_model_output":reply.text,"error":reply.error}));
                    ensure!(reply.model==*expected,"assessment response model differs from requested resident model");
                    ensure!(reply.error.is_none(),"assessment generation failed: {}",reply.error.as_deref().unwrap_or("unknown"));
                    Ok(reply.text)
                }
            }).await;
            let calls = calls
                .lock()
                .map_err(|_| anyhow::anyhow!("assessment metrics unavailable"))?
                .clone();
            let report = match report {
                Ok(report) => report,
                Err(error) => {
                    println!(
                        "{}",
                        json!({"fixture_only":true,"evaluation":null,"error":error.to_string(),"calls":calls})
                    );
                    return Err(error);
                }
            };
            json!({"fixture_only":true,"evaluation":report,"calls":calls,"transport":{"endpoint":endpoint,"seed":seed,"temperature":0.6,"thinking":false,"max_tokens":8192,"request_timeout_seconds":180,"retries":0}})
        }
        Cmd::DistillConcept {
            source,
            revise,
            endpoint,
            model,
            holds_dir,
            seed,
        } => {
            let references = source
                .iter()
                .map(|s| parse_reference(s))
                .collect::<Result<Vec<_>>>()?;
            let expected = revise.as_deref().map(parse_reference).transpose()?;
            let mut memory = Memory::open(&args.db)?;
            let access = fixture::access();
            let receipt = memory.prepare_concept(&references, &memory.snapshot()?, &access)?;
            let generation = receipt.generation();
            let client = LocalMemoryModel::new(&endpoint, &model, holds_dir, seed, true)?;
            let reply = client
                .complete(generation.prompt, Some(generation.response_schema))
                .await?;
            ensure!(
                reply.error.is_none(),
                "concept generation failed: {}",
                reply.error.as_deref().unwrap_or("unknown")
            );
            ensure!(
                reply.model == model,
                "concept response model differs from requested resident model"
            );
            let reference =
                memory.retain_concept(receipt, &reply.text, &access, expected.as_ref())?;
            json!({"fixture_only":true,"reference":reference,"concept":memory.read(&reference,&access)?,"model":reply.model,
                "input_tokens":reply.input_tokens,"output_tokens":reply.output_tokens,
                "limit":"Provisional hypothesis with verified source quotations, not verified entailment or a trusted behavioral rule."})
        }
        Cmd::EvaluateMentions => {
            mecha_core::memory::model_evaluation::followup::mention_evaluation::run(&args.db)?
        }
        Cmd::Evaluate => serde_json::to_value(evaluation::run(&args.db)?)?,
        Cmd::EvaluateHybrid {
            synthetic_input,
            endpoint,
            expected_alias,
            artifact_revision,
            dimensions,
            pooling,
            normalization,
        } => {
            use mecha_core::memory_hybrid_workflow::{run, EvaluationOptions};
            ensure!(synthetic_input, "--synthetic-input is required");
            serde_json::to_value(
                run(
                    &args.db,
                    &EvaluationOptions {
                        endpoint,
                        expected_alias,
                        artifact_revision,
                        dimensions,
                        pooling,
                        normalization,
                    },
                )
                .await?,
            )?
        }
        Cmd::ForgetSessionRecord {
            account,
            session,
            line,
        } => {
            ensure!(line > 0, "line must be positive");
            let mut memory = Memory::open_with_storage(&args.db, storage.clone())?;
            serde_json::to_value(memory.forget(&transcript_source(&account, &session, line))?)?
        }
        Cmd::Demo => fixture::demo(&args.db)?,
        Cmd::Inspect => {
            let memory = open_read_only()?;
            json!({"snapshot": memory.snapshot()?, "fixture_only": true})
        }
        Cmd::Query {
            text,
            known_at,
            valid_at,
            include_possible,
            limit,
        } => {
            let memory = open_read_only()?;
            let mut snapshot = memory.snapshot()?;
            if let Some(sequence) = known_at {
                snapshot.sequence = sequence;
            }
            serde_json::to_value(memory.search(
                &Query {
                    text,
                    known_at: snapshot,
                    valid_at,
                    include_possible,
                    limit,
                },
                &fixture::access(),
            )?)?
        }
        Cmd::Forget { source_id } => {
            let mut memory = Memory::open_with_storage(&args.db, storage.clone())?;
            serde_json::to_value(memory.forget(&fixture::source_key(&source_id))?)?
        }
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

async fn execute_durable_read(
    db: &std::path::Path,
    args: DurableReadArgs,
) -> Result<serde_json::Value> {
    use mecha_core::{
        agent::{Conversation, RunContext},
        config::PermissionMode,
        memory::{durable::TaskSpec, Access},
        tool::{ModeApprover, ToolCtx},
    };
    use std::sync::Arc;

    ensure!(args.synthetic_input, "--synthetic-input is required");
    let access = Access {
        domain: args.domain,
        scopes: args.scope.into_iter().collect(),
    };
    let mut memory = Memory::open(db)?;
    let status = match args.cmd {
        DurableReadCommand::Start {
            workspace,
            file,
            source,
            max_bytes,
            max_lines,
        } => {
            let sources = source
                .iter()
                .map(|value| parse_reference(value))
                .collect::<Result<Vec<_>>>()?;
            let receipt = memory.durable_create(
                TaskSpec {
                    workspace: workspace.canonicalize().context("resolve task workspace")?,
                    path: file,
                    max_bytes,
                    max_lines,
                },
                &sources,
                &access,
            )?;
            memory.durable_status(&receipt, &access)?
        }
        DurableReadCommand::Resume { task, workspace } => {
            let receipt = memory.durable_reopen(&task, &access)?;
            let ctx = RunContext::new(
                ToolCtx {
                    workspace,
                    spill_dir: None,
                    ..Default::default()
                },
                Arc::new(ModeApprover {
                    mode: PermissionMode::ReadOnly,
                }),
            );
            let mut conversation = Conversation::default();
            let status = mecha_core::memory_runtime::acquire(
                &mut memory,
                &receipt,
                &access,
                &ctx,
                &mut conversation,
            )
            .await?;
            return Ok(json!({
                "status": status,
                "observation": memory.durable_observation(&receipt, &access)?,
                "taint": conversation.taint,
                "fixture_only": true,
                "model_called": false,
            }));
        }
        DurableReadCommand::Finish { task, proposal } => {
            ensure!(proposal.len() <= 16_384, "proposal exceeds 16384 bytes");
            let proposal: serde_json::Value = serde_json::from_str(&proposal)?;
            let receipt = memory.durable_reopen(&task, &access)?;
            let permit = memory.durable_reserve_reconsideration(&receipt, &access)?;
            memory.durable_commit(permit, proposal, &access)?
        }
        DurableReadCommand::Status { task } => {
            let receipt = memory.durable_reopen(&task, &access)?;
            memory.durable_status(&receipt, &access)?
        }
        DurableReadCommand::Cancel { task } => {
            let receipt = memory.durable_reopen(&task, &access)?;
            memory.durable_cancel(&receipt, &access)?
        }
    };
    Ok(json!({"status": status, "fixture_only": true, "model_called": false}))
}

fn execute_durable_act(db: &std::path::Path, args: DurableActArgs) -> Result<serde_json::Value> {
    use mecha_core::{
        memory::{durable_action::ActionPhase, Access},
        memory_action_runtime::{self as runtime, ActionSpec},
    };
    ensure!(args.synthetic_input, "--synthetic-input is required");
    let access = Access {
        domain: args.domain,
        scopes: args.scope.into_iter().collect(),
    };
    let mut memory = Memory::open(db)?;
    let receipt = match args.cmd {
        DurableActCommand::Bind { model, spec } => {
            ensure!(spec.len() <= 16_384, "action spec exceeds 16384 bytes");
            let spec: ActionSpec = serde_json::from_str(&spec)?;
            let model = memory.durable_model_reopen(&model, &access)?;
            runtime::bind(&mut memory, &model, spec, &access)?
        }
        DurableActCommand::Run { task } => {
            let receipt = memory.durable_action_reopen(&task, &access)?;
            runtime::run(&mut memory, &receipt, &access)?;
            receipt
        }
        DurableActCommand::Status { task } => memory.durable_action_reopen(&task, &access)?,
        DurableActCommand::Cancel { task } => {
            let receipt = memory.durable_action_reopen(&task, &access)?;
            memory.durable_action_cancel(&receipt, &access)?;
            receipt
        }
    };
    let status = memory.durable_action_status(&receipt, &access)?;
    let content = (|| -> Result<serde_json::Value> {
        ensure!(status.phase != ActionPhase::Forgotten, "action forgotten");
        Ok(json!({"spec":memory.durable_action_spec(&receipt,&access)?,
            "observation":memory.durable_action_observation(&receipt,&access)?,
            "feedback":memory.durable_action_feedback(&receipt,&access)?}))
    })();
    let mut result = content.unwrap_or_else(
        |_| json!({"spec":null,"observation":null,"feedback":null,"content_unavailable":true}),
    );
    result["status"] = serde_json::to_value(status)?;
    result["fixture_only"] = json!(true);
    result["model_called"] = json!(false);
    Ok(result)
}

async fn execute_durable_decide(
    db: &std::path::Path,
    args: DurableDecideArgs,
) -> Result<serde_json::Value> {
    use mecha_core::{
        memory::{
            decision::{Contract, EvidenceView},
            durable_decision::{AttemptKind, ModelPhase},
            Access,
        },
        memory_decision_runtime::{self as runtime, ModelSpec},
    };
    ensure!(args.synthetic_input, "--synthetic-input is required");
    let access = Access {
        domain: args.domain,
        scopes: args.scope.into_iter().collect(),
    };
    let mut memory = Memory::open(db)?;
    let receipt = match args.cmd {
        DurableDecideCommand::Bind {
            read,
            contract,
            model,
            seed,
            view,
        } => {
            ensure!(contract.len() <= 16_384, "contract exceeds 16384 bytes");
            // Deserialize directly into the strict struct: serde rejects duplicate fields.
            let contract: Contract = serde_json::from_str(&contract)?;
            let read = memory.durable_reopen(&read, &access)?;
            let receipt = runtime::bind(
                &mut memory,
                &read,
                ModelSpec {
                    contract,
                    model,
                    seed,
                    view: match view {
                        DecisionView::None => EvidenceView::None,
                        DecisionView::Observed => EvidenceView::Observed,
                    },
                },
                &access,
            )?;
            return Ok(
                json!({"status":memory.durable_model_status(&receipt,&access)?,"fixture_only":true}),
            );
        }
        DurableDecideCommand::Run {
            task,
            endpoint,
            holds_dir,
        } => {
            let receipt = memory.durable_model_reopen(&task, &access)?;
            let status = memory.durable_model_status(&receipt, &access)?;
            if status.phase == ModelPhase::Active {
                let spec = runtime::spec(&memory, &receipt, &access)?;
                let client =
                    LocalMemoryModel::new(&endpoint, &spec.model, holds_dir, spec.seed, true)?;
                runtime::decide(&mut memory, &receipt, &access, |prompt, schema, guard| {
                    client.complete_guarded(prompt, schema, guard)
                })
                .await?;
            }
            receipt
        }
        DurableDecideCommand::Status { task } => memory.durable_model_reopen(&task, &access)?,
        DurableDecideCommand::Cancel { task } => {
            let receipt = memory.durable_model_reopen(&task, &access)?;
            memory.durable_model_cancel(&receipt, &access)?;
            receipt
        }
    };
    let status = memory.durable_model_status(&receipt, &access)?;
    if status.phase == ModelPhase::Forgotten {
        return Ok(
            json!({"status":status,"spec":null,"initial":null,"retry":null,"decision":null,"fixture_only":true}),
        );
    }
    // Operational status survives loss of content authority. Confirm cancellation
    // even when current sources forbid dereferencing its retained audit records.
    let content = (|| -> Result<serde_json::Value> {
        Ok(json!({"spec":runtime::spec(&memory,&receipt,&access)?,
            "initial":memory.durable_model_attempt(&receipt,AttemptKind::Initial,&access)?,
            "retry":memory.durable_model_attempt(&receipt,AttemptKind::Retry,&access)?,
            "decision":memory.durable_model_decision(&receipt,&access)?}))
    })();
    let mut result = match content {
        Ok(value) => value,
        Err(_) => {
            json!({"spec":null,"initial":null,"retry":null,"decision":null,"content_unavailable":true})
        }
    };
    result["status"] = serde_json::to_value(status)?;
    result["fixture_only"] = json!(true);
    Ok(result)
}

/// Transport and router coordination only; experiment logic lives in core.
async fn evaluate_model(
    db: &std::path::Path,
    endpoint: &str,
    model: &str,
    holds_dir: PathBuf,
    seed: u64,
    split: Option<mecha_core::memory::model_evaluation::followup::Split>,
    auto_bindings: bool,
) -> Result<serde_json::Value> {
    use mecha_core::memory::model_evaluation::{self, followup, Row};
    let client = LocalMemoryModel::new(endpoint, model, holds_dir, seed, split.is_some())?;
    let complete =
        |prompt: String, schema: Option<serde_json::Value>| client.complete(prompt, schema);
    let progress = |row: &Row| {
        eprintln!(
            "{} {:?}: grounded={} error={}",
            row.case_id,
            row.arm,
            row.score.as_ref().is_some_and(|s| s.grounded),
            row.error.as_deref().unwrap_or("none")
        )
    };
    let mut result = if auto_bindings {
        followup::binding_evaluation::run(
            db,
            model,
            split.context("binding split required")?,
            |req| complete(req.prompt, Some(req.response_schema)),
            progress,
        )
        .await?
    } else if let Some(split) = split {
        serde_json::to_value(
            followup::run(
                db,
                model,
                split,
                |req| complete(req.prompt, Some(req.response_schema)),
                progress,
            )
            .await?,
        )?
    } else {
        serde_json::to_value(
            model_evaluation::run(db, model, |prompt| complete(prompt, None), progress).await?,
        )?
    };
    result["transport"] = json!({"endpoint":endpoint,"seed":seed,"temperature":0.6,"thinking":false,"max_tokens":8192,"request_timeout_seconds":180,"retries":0,"structured_output":if split.is_some(){"llama_json_schema"}else{"disabled"}});
    Ok(result)
}

fn parse_reference(value: &str) -> Result<mecha_core::memory::RevisionRef> {
    let (object, revision) = value.split_once(':').context("expected object:revision")?;
    ensure!(
        !object.is_empty() && !revision.is_empty() && value.len() <= 256 && !revision.contains(':'),
        "expected bounded object:revision"
    );
    Ok(mecha_core::memory::RevisionRef {
        object: object.into(),
        revision: revision.into(),
    })
}

async fn execute_outcome_recall(
    path: &std::path::Path,
    input: OutcomeRecallArgs,
) -> Result<serde_json::Value> {
    use mecha_core::{
        memory::{Access, Snapshot},
        memory_outcome_reappraisal::{self as outcome, Arm},
    };
    ensure!(input.synthetic_input, "--synthetic-input is required");
    ensure!(
        !input.model.trim().is_empty() && input.model.len() <= 256,
        "explicit bounded model required"
    );
    let db = Memory::open_read_only(path)?;
    let access = Access {
        domain: input.domain,
        scopes: input.scope.into_iter().collect(),
    };
    let action = db.durable_action_reopen(&input.action, &access)?;
    let snapshot: Snapshot = match input.snapshot {
        Some(text) => serde_json::from_str(&text)?,
        None => db.snapshot()?,
    };
    let query = RecallQuery {
        text: input.query,
        known_at: snapshot,
        account: None,
        session_id: None,
        occurred_after: None,
        occurred_before: None,
        limit: 100,
        neighbors: 0,
        max_bytes: 100000,
    };
    let current = serde_json::from_str(&input.contract)?;
    let pair = match input.response_format {
        Some(format) => outcome::prepare_pair_with_options(
            &db,
            &action,
            &current,
            &query,
            &access,
            outcome::PairOptions {
                response_format: format.into(),
            },
        )?,
        None => outcome::prepare_pair(&db, &action, &current, &query, &access)?,
    };
    let controls = json!({"temperature":0.6,"max_tokens":8192,"thinking":input.thinking_budget.is_some(),"max_retries":0,"local_only":true,"structured_output":"llama_json","timeout_seconds":180,"thinking_budget":input.thinking_budget});
    if input.prepare_only {
        let mut value = serde_json::to_value(pair)?;
        if input.response_format.is_some() {
            value["model_controls"] = controls;
        }
        return Ok(value);
    }
    let mut local = LocalMemoryModel::new(
        input
            .endpoint
            .as_deref()
            .context("explicit --endpoint required")?,
        &input.model,
        input.holds_dir.context("explicit --holds-dir required")?,
        input.seed,
        true,
    )?;
    if input.thinking_budget.is_some() {
        local.pass = local.pass.think(true);
    }
    let reasoning_counts = std::cell::RefCell::new(Vec::new());
    let order = if input.outcome_first {
        [Arm::SourceAndOutcome, Arm::SourceOnly]
    } else {
        [Arm::SourceOnly, Arm::SourceAndOutcome]
    };
    let report = outcome::evaluate_pair(
        &db,
        &pair,
        &access,
        &input.model,
        order,
        |prompt, schema, guard| {
            let local = &local;
            let counts = &reasoning_counts;
            async move {
                let result = local
                    .complete_with_guard(
                        prompt,
                        schema,
                        input.thinking_budget,
                        Some(MemoryCallGuard::Outcome(guard)),
                    )
                    .await;
                counts
                    .borrow_mut()
                    .push(result.as_ref().ok().map(|(_, count)| *count));
                result.map(|(reply, _)| reply)
            }
        },
    )
    .await?;
    let mut value = serde_json::to_value(report)?;
    if input.response_format.is_some() {
        value["model_controls"] = controls.clone();
        value["preparation"]["model_controls"] = controls;
        for (row, count) in value["rows"]
            .as_array_mut()
            .context("missing paired rows")?
            .iter_mut()
            .zip(reasoning_counts.into_inner())
        {
            row["reasoning_char_count"] = json!(count);
        }
    }
    value["seed"] = json!(input.seed);
    if input.response_format.is_none() {
        value["model_controls"] = json!({"temperature":0.6,"max_tokens":8192,"thinking":false,"max_retries":0,"local_only":true,"structured_output":"llama_json","timeout_seconds":180});
    }
    Ok(value)
}

enum MemoryCallGuard {
    Decision(mecha_core::memory::durable_decision::ModelGuard),
    Outcome(mecha_core::memory_outcome_reappraisal::OutcomeGuard),
    Learning(mecha_core::memory_session::learning::LearningGuard),
}
impl MemoryCallGuard {
    fn check(self) -> Result<()> {
        match self {
            Self::Decision(g) => g.check(),
            Self::Outcome(g) => g.check(),
            Self::Learning(g) => g.check(),
        }
    }
}

/// Shared local-only transport. No global configuration, source discovery, tools,
/// retries or model switching; existing evaluation protocols retain their inputs.
pub(super) struct LocalMemoryModel {
    provider: mecha_core::provider::openai::OpenAiCompatible,
    holds: mecha_core::hold::Holds,
    pass: mecha_core::quarantine::QuarantinedPass,
    endpoint: String,
    model: String,
}
impl LocalMemoryModel {
    pub(super) fn new(
        endpoint: &str,
        model: &str,
        holds_dir: PathBuf,
        seed: u64,
        structured: bool,
    ) -> Result<Self> {
        use mecha_core::{
            config::{ProviderConfig, StructuredOutput},
            hold::Holds,
            provider::{openai::OpenAiCompatible, router},
            quarantine::QuarantinedPass,
        };
        ensure!(
            router::is_loopback(endpoint),
            "evaluation endpoint must be loopback"
        );
        let url = reqwest::Url::parse(endpoint)?;
        ensure!(
            url.scheme() == "http"
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "expected plain loopback HTTP endpoint without credentials/query/fragment"
        );
        ensure!(!model.trim().is_empty(), "explicit model required");
        let provider = OpenAiCompatible::from_local_config(&ProviderConfig {
            kind: "local".into(),
            model: Some(model.into()),
            base_url: Some(endpoint.into()),
            temperature: Some(0.6),
            seed: Some(seed),
            max_retries: Some(0),
            structured_output: if structured {
                StructuredOutput::LlamaJson
            } else {
                StructuredOutput::Disabled
            },
            ..Default::default()
        })?
        .without_autoload();
        Ok(Self {
            provider,
            holds: Holds::new(holds_dir),
            pass: QuarantinedPass::new(model, 8192).no_thinking().local_only(),
            endpoint: endpoint.into(),
            model: model.into(),
        })
    }
    async fn complete(
        &self,
        prompt: String,
        schema: Option<serde_json::Value>,
    ) -> Result<mecha_core::memory::model_evaluation::ModelReply> {
        Ok(self.complete_audited(prompt, schema, None).await?.0)
    }
    async fn complete_audited(
        &self,
        prompt: String,
        schema: Option<serde_json::Value>,
        thinking_budget: Option<u32>,
    ) -> Result<(mecha_core::memory::model_evaluation::ModelReply, usize)> {
        self.complete_with_guard(prompt, schema, thinking_budget, None)
            .await
    }
    pub(super) async fn complete_learning(
        &self,
        generation: mecha_core::memory::concept::ConceptGeneration,
        guard: mecha_core::memory_session::learning::LearningGuard,
    ) -> Result<mecha_core::memory::model_evaluation::ModelReply> {
        let mut reply = self
            .complete_with_guard(
                generation.prompt,
                Some(generation.response_schema),
                None,
                Some(MemoryCallGuard::Learning(guard)),
            )
            .await?
            .0;
        // The decoded provider Usage cannot distinguish an absent counter from
        // an explicit zero. Preserve each known positive count independently;
        // the benchmark's wire audit retains exact field presence and zeroes.
        reply.input_tokens = reply.input_tokens.filter(|count| *count > 0);
        reply.output_tokens = reply.output_tokens.filter(|count| *count > 0);
        if reply.model != self.model {
            let mismatch = format!(
                "model identity mismatch: expected {:?}, received {:?}",
                self.model, reply.model
            );
            reply.error = Some(match reply.error {
                Some(error) => format!("{error}; {mismatch}"),
                None => mismatch,
            });
        }
        Ok(reply)
    }
    async fn complete_guarded(
        &self,
        prompt: String,
        schema: Option<serde_json::Value>,
        guard: mecha_core::memory::durable_decision::ModelGuard,
    ) -> Result<mecha_core::memory::model_evaluation::ModelReply> {
        Ok(self
            .complete_with_guard(prompt, schema, None, Some(MemoryCallGuard::Decision(guard)))
            .await?
            .0)
    }
    async fn complete_with_guard(
        &self,
        prompt: String,
        schema: Option<serde_json::Value>,
        thinking_budget: Option<u32>,
        guard: Option<MemoryCallGuard>,
    ) -> Result<(mecha_core::memory::model_evaluation::ModelReply, usize)> {
        use mecha_core::{
            memory::model_evaluation::ModelReply,
            message::StopReason,
            provider::{router, Provider},
        };
        let held = self
            .holds
            .try_hold(&self.endpoint, "synthetic memory comparison")?
            .map_err(|_| anyhow::anyhow!("model switch pending; evaluation yields"))?;
        let cancel = tokio_util::sync::CancellationToken::new();
        let signal = cancel.clone();
        held.on_cancel(move || signal.cancel());
        let models = router::local_models(&self.endpoint)
            .await
            .context("cannot verify resident router model")?;
        ensure!(
            router::resident(&models) == Some(self.model.as_str()),
            "requested model is not resident; refusing to load/switch it"
        );
        let mut request = self.pass.clone().response_schema(schema).ask(prompt);
        ensure!(
            thinking_budget.is_none() || request.think == Some(true),
            "reasoning budget requires thinking"
        );
        request.think_budget = thinking_budget;
        // Provider discovery awaited above: recheck durable authority at the dispatch boundary.
        if let Some(guard) = guard {
            guard.check()?;
        }
        let response = tokio::select! {
            _=cancel.cancelled()=>anyhow::bail!("evaluation cancelled for model switch"),
            response=tokio::time::timeout(std::time::Duration::from_secs(180),self.provider.complete(&request,None))=>response.context("model request exceeded 180 seconds")??,
        };
        let known_usage = response.usage.total_input() > 0 || response.usage.output_tokens > 0;
        let reasoning_chars = response.message.thinking().chars().count();
        Ok((
            ModelReply {
                text: response.message.text(),
                model: response.model,
                input_tokens: known_usage.then(|| response.usage.total_input()),
                output_tokens: known_usage.then_some(response.usage.output_tokens),
                error: (!matches!(response.stop_reason, StopReason::EndTurn))
                    .then(|| format!("incomplete model response: {:?}", response.stop_reason)),
            },
            reasoning_chars,
        ))
    }
}

#[cfg(test)]
mod local_model_tests {
    use super::LocalMemoryModel;
    use axum::{http::StatusCode, Json, Router};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    async fn call_resident_model(status: StatusCode) -> (anyhow::Result<String>, Vec<String>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let requests = seen.clone();
        let app = Router::new().fallback(move |request: axum::extract::Request| {
            let requests = requests.clone();
            async move {
                requests.lock().unwrap().push(format!(
                    "{} {}",
                    request.method(),
                    request.uri()
                ));
                match request.uri().path() {
                    "/props" => (StatusCode::OK, Json(json!({"role": "router"}))),
                    "/models" => (
                        StatusCode::OK,
                        Json(json!({"data": [{"id": "fixture-model", "status": {"value": "loaded"}}]})),
                    ),
                    "/v1/chat/completions" if status == StatusCode::OK => (
                        status,
                        Json(json!({"model": "fixture-model", "choices": [{"message": {"role": "assistant", "content": "fixture reply"}, "finish_reason": "stop"}]})),
                    ),
                    "/v1/chat/completions" => (
                        status,
                        Json(json!({"error": {"message": "model is no longer loaded", "type": "unavailable_error"}})),
                    ),
                    _ => (StatusCode::NOT_FOUND, Json(json!({}))),
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let holds =
            std::env::temp_dir().join(format!("mecha-memory-local-{}", uuid::Uuid::new_v4()));
        let model =
            LocalMemoryModel::new(&endpoint, "fixture-model", holds.clone(), 42, false).unwrap();
        let reply = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            model.complete("authored fixture question".into(), None),
        )
        .await;
        server.abort();
        std::fs::remove_dir_all(holds).unwrap();
        let requests = seen.lock().unwrap().clone();
        (
            reply.expect("local request should finish").map(|r| r.text),
            requests,
        )
    }

    #[tokio::test]
    async fn resident_inference_disables_autoload_on_the_actual_post() {
        let (reply, requests) = call_resident_model(StatusCode::OK).await;
        assert_eq!(reply.unwrap(), "fixture reply");
        assert_eq!(
            requests,
            [
                "GET /props",
                "GET /models",
                "POST /v1/chat/completions?autoload=false"
            ]
        );
    }

    #[tokio::test]
    async fn unavailable_after_resident_check_refuses_without_retry_or_load() {
        let (reply, requests) = call_resident_model(StatusCode::SERVICE_UNAVAILABLE).await;
        assert!(reply.unwrap_err().to_string().contains("503"));
        assert_eq!(
            requests,
            [
                "GET /props",
                "GET /models",
                "POST /v1/chat/completions?autoload=false"
            ]
        );
    }
}

#[cfg(test)]
mod durable_cli_tests {
    use clap::Parser;

    #[test]
    fn extract_plan_thinking_budget_reserves_final_answer_tokens() {
        let args = [
            "mecha",
            "memory",
            "--experimental",
            "--db",
            "/tmp/test.memory-prototype.sqlite",
            "extract-plan",
            "/tmp/synthetic-plan.json",
            "--synthetic-input",
            "--endpoint",
            "http://127.0.0.1:8080",
            "--model",
            "fixture-model",
            "--holds-dir",
            "/tmp/holds",
        ];
        assert!(crate::Cli::try_parse_from(args).is_ok());
        for (budget, accepted) in [
            ("1", true),
            ("4096", true),
            ("0", false),
            ("4097", false),
            ("8191", false),
        ] {
            let mut bounded = args.to_vec();
            bounded.extend(["--thinking", "--thinking-budget", budget]);
            assert_eq!(
                crate::Cli::try_parse_from(bounded).is_ok(),
                accepted,
                "thinking budget {budget}"
            );
        }
        let mut without_thinking = args.to_vec();
        without_thinking.extend(["--thinking-budget", "4096"]);
        let error = crate::Cli::try_parse_from(without_thinking)
            .err()
            .expect("a thinking budget requires --thinking");
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn outcome_recall_prepare_requires_explicit_synthetic_access_and_model() {
        let args = [
            "mecha",
            "memory",
            "--experimental",
            "--db",
            "/tmp/test.memory-prototype.sqlite",
            "evaluate-outcome-recall",
            "--synthetic-input",
            "--domain",
            "synthetic",
            "--scope",
            "fixture",
            "--action",
            "action-id",
            "--contract",
            r#"{"workload":"durable-action-fixture-v1","objective":"minimize_minutes","energy_budget":4,"minimum_quality":7}"#,
            "--query",
            "Nori",
            "--model",
            "fixture-model",
            "--prepare-only",
        ];
        assert!(crate::Cli::try_parse_from(args).is_ok());
        let mut diagnostic = args.to_vec();
        diagnostic.extend([
            "--response-format",
            "feasibility-first",
            "--thinking-budget",
            "2048",
        ]);
        assert!(crate::Cli::try_parse_from(&diagnostic).is_ok());
        for budget in ["0", "4097"] {
            *diagnostic.last_mut().unwrap() = budget;
            assert!(crate::Cli::try_parse_from(&diagnostic).is_err());
        }
        let mut legacy_thinking = args.to_vec();
        legacy_thinking.extend(["--thinking-budget", "2048"]);
        assert!(crate::Cli::try_parse_from(&legacy_thinking).is_err());

        for omitted in [
            "--synthetic-input",
            "--domain",
            "--scope",
            "--action",
            "--contract",
            "--query",
            "--model",
        ] {
            let mut incomplete = Vec::new();
            let mut i = 0;
            while i < args.len() {
                if args[i] == omitted {
                    i += if omitted == "--synthetic-input" { 1 } else { 2 };
                } else {
                    incomplete.push(args[i]);
                    i += 1;
                }
            }
            assert!(crate::Cli::try_parse_from(incomplete).is_err(), "{omitted}");
        }
        let mut runtime = args[..args.len() - 1].to_vec();
        assert!(crate::Cli::try_parse_from(&runtime).is_err());
        runtime.extend([
            "--endpoint",
            "http://127.0.0.1:8080",
            "--holds-dir",
            "/tmp/holds",
        ]);
        assert!(crate::Cli::try_parse_from(&runtime).is_err());
        runtime.extend(["--snapshot", r#"{"instance":"fixture","sequence":1}"#]);
        assert!(crate::Cli::try_parse_from(runtime).is_ok());
    }

    #[test]
    fn durable_read_requires_explicit_authority_and_source() {
        let args = [
            "mecha",
            "memory",
            "--experimental",
            "--db",
            "/tmp/task.memory-prototype.sqlite",
            "durable-read",
            "--synthetic-input",
            "--domain",
            "synthetic-demo",
            "--scope",
            "fixture-mail",
            "start",
            "--workspace",
            "/tmp/fixture",
            "--file",
            "observation.txt",
            "--source",
            "object:revision",
        ];
        assert!(crate::Cli::try_parse_from(args).is_ok());
        for (flag, count) in [
            ("--experimental", 1),
            ("--synthetic-input", 1),
            ("--domain", 2),
            ("--scope", 2),
            ("--workspace", 2),
            ("--file", 2),
            ("--source", 2),
        ] {
            let mut missing = args.to_vec();
            let index = missing.iter().position(|value| *value == flag).unwrap();
            missing.drain(index..index + count);
            assert!(crate::Cli::try_parse_from(missing).is_err(), "{flag}");
        }
    }

    #[test]
    fn durable_resume_cannot_inherit_workspace_implicitly() {
        let args = [
            "mecha",
            "memory",
            "--experimental",
            "--db",
            "/tmp/task.memory-prototype.sqlite",
            "durable-read",
            "--synthetic-input",
            "--domain",
            "synthetic-demo",
            "--scope",
            "fixture-mail",
            "resume",
            "--task",
            "task-id",
        ];
        assert!(crate::Cli::try_parse_from(args).is_err());
        let mut complete = args.to_vec();
        complete.extend(["--workspace", "/tmp/fixture"]);
        assert!(crate::Cli::try_parse_from(complete).is_ok());
    }
}

#[cfg(test)]
mod concept_cli_tests {
    use super::*;
    use clap::Parser;
    #[test]
    fn concept_command_requires_explicit_sources_and_transport() {
        let args = [
            "mecha",
            "memory",
            "--experimental",
            "--db",
            "/tmp/fixture.memory-prototype.sqlite",
            "distill-concept",
            "--source",
            "object:revision",
            "--endpoint",
            "http://127.0.0.1:8080",
            "--model",
            "fixture",
            "--holds-dir",
            "/tmp/holds",
        ];
        assert!(crate::Cli::try_parse_from(args).is_ok());
        for flag in ["--source", "--endpoint", "--model", "--holds-dir"] {
            let mut missing = args.to_vec();
            let at = missing.iter().position(|arg| *arg == flag).unwrap();
            missing.drain(at..at + 2);
            assert!(crate::Cli::try_parse_from(missing).is_err(), "{flag}");
        }
        assert!(parse_reference("a:b").is_ok());
        for bad in ["a", ":b", "a:", "a:b:c"] {
            assert!(parse_reference(bad).is_err());
        }
    }
}

#[cfg(test)]
mod extraction_cli_tests {
    use clap::Parser;
    #[test]
    fn extraction_requires_explicit_synthetic_input_and_local_transport() {
        let args = [
            "mecha",
            "memory",
            "--experimental",
            "--db",
            "/tmp/unused.memory-prototype.sqlite",
            "extract-plan",
            "/tmp/synthetic.json",
            "--synthetic-input",
            "--endpoint",
            "http://127.0.0.1:8080",
            "--model",
            "fixture",
            "--holds-dir",
            "/tmp/holds",
        ];
        assert!(crate::Cli::try_parse_from(args).is_ok());
        let mut modes = args.to_vec();
        modes.extend(["--thinking", "--direct"]);
        assert!(crate::Cli::try_parse_from(modes).is_ok());
        for budget in ["0", "8192", "-1"] {
            let mut bounded = args.to_vec();
            bounded.extend(["--thinking", "--thinking-budget", budget]);
            assert!(crate::Cli::try_parse_from(bounded).is_err());
        }
        let mut bounded = args.to_vec();
        bounded.extend(["--thinking", "--thinking-budget", "512"]);
        assert!(crate::Cli::try_parse_from(bounded).is_ok());
        let mut no_thinking = args.to_vec();
        no_thinking.extend(["--thinking-budget", "512"]);
        assert!(crate::Cli::try_parse_from(no_thinking).is_err());
        for flag in ["--synthetic-input", "--endpoint", "--model", "--holds-dir"] {
            let mut missing = args.to_vec();
            let at = missing.iter().position(|a| *a == flag).unwrap();
            missing.drain(at..at + if flag == "--synthetic-input" { 1 } else { 2 });
            assert!(crate::Cli::try_parse_from(missing).is_err(), "{flag}");
        }
    }
}

#[cfg(test)]
mod workflow_cli_tests {
    use clap::Parser;
    #[test]
    fn workflow_requires_synthetic_acknowledgment_and_explicit_transport() {
        let args = [
            "mecha",
            "memory",
            "--experimental",
            "--db",
            "/tmp/new.memory-prototype.sqlite",
            "workflow-trial",
            "--synthetic-input",
            "--endpoint",
            "http://127.0.0.1:8080",
            "--model",
            "fixture",
            "--holds-dir",
            "/tmp/holds",
        ];
        assert!(crate::Cli::try_parse_from(args).is_ok());
        for flag in ["--recall-views", "--unseen", "--checked", "--observe"] {
            let mut selected = args.to_vec();
            selected.push(flag);
            assert!(crate::Cli::try_parse_from(selected).is_ok());
        }
        let mut reconsider = args.to_vec();
        reconsider.push("--reconsider");
        assert!(crate::Cli::try_parse_from(&reconsider).is_err());
        reconsider.push("--observe");
        assert!(crate::Cli::try_parse_from(&reconsider).is_ok());
        for pair in [
            ["--recall-views", "--unseen"],
            ["--checked", "--unseen"],
            ["--checked", "--recall-views"],
            ["--observe", "--checked"],
            ["--observe", "--unseen"],
            ["--observe", "--recall-views"],
        ] {
            let mut both = args.to_vec();
            both.extend(pair);
            assert!(crate::Cli::try_parse_from(both).is_err());
        }
        for flag in ["--synthetic-input", "--endpoint", "--model", "--holds-dir"] {
            let mut missing = args.to_vec();
            let at = missing.iter().position(|a| *a == flag).unwrap();
            missing.drain(at..at + if flag == "--synthetic-input" { 1 } else { 2 });
            assert!(crate::Cli::try_parse_from(missing).is_err(), "{flag}");
        }
    }
}

#[cfg(test)]
mod action_cli_tests {
    use clap::Parser;
    #[test]
    fn actions_require_explicit_synthetic_input_and_have_no_provider_arguments() {
        let base = [
            "mecha",
            "memory",
            "--experimental",
            "--db",
            "/tmp/action.memory-prototype.sqlite",
            "durable-act",
            "--synthetic-input",
            "--domain",
            "fixture",
            "--scope",
            "fixture",
            "run",
            "--task",
            "action-id",
        ];
        assert!(crate::Cli::try_parse_from(base).is_ok());
        let mut endpoint = base.to_vec();
        endpoint.extend(["--endpoint", "http://127.0.0.1:8080"]);
        assert!(crate::Cli::try_parse_from(endpoint).is_err());
        let missing: Vec<_> = base
            .into_iter()
            .filter(|arg| *arg != "--synthetic-input")
            .collect();
        assert!(crate::Cli::try_parse_from(missing).is_err());
        assert!(crate::Cli::try_parse_from([
            "mecha",
            "memory",
            "--experimental",
            "--db",
            "/tmp/action.memory-prototype.sqlite",
            "init-action",
            "--domain",
            "fixture",
            "--synthetic-input"
        ])
        .is_ok());
    }
}

#[cfg(test)]
mod native_storage_cli_tests {
    use super::*;
    use clap::Parser;
    use std::{
        fs,
        os::unix::fs::DirBuilderExt,
        sync::atomic::{AtomicUsize, Ordering},
    };

    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn parse_memory(arguments: &[&str]) -> Result<Args> {
        let cli = crate::Cli::try_parse_from(arguments)?;
        if let crate::Command::Memory(args) = cli.command {
            Ok(args)
        } else {
            anyhow::bail!("expected experimental memory command")
        }
    }

    struct AuthoredRecallProvider {
        step: AtomicUsize,
        recall: bool,
    }
    #[async_trait::async_trait]
    impl Provider for AuthoredRecallProvider {
        fn id(&self) -> &str {
            "authored"
        }
        fn default_model(&self) -> &str {
            "authored"
        }
        fn local(&self) -> bool {
            true
        }
        async fn complete(
            &self,
            _: &mecha_core::message::CompletionRequest,
            _: Option<&mecha_core::provider::StreamSink>,
        ) -> Result<mecha_core::message::CompletionResponse> {
            use mecha_core::message::{Block, CompletionResponse, Message, StopReason, Usage};
            let step = self.step.fetch_add(1, Ordering::SeqCst);
            let content = if self.recall && step == 0 {
                Block::ToolUse {
                    id: "recall".into(),
                    name: "goal_context".into(),
                    input: json!({"memory":{"query":"calibration"}}),
                }
            } else if step == usize::from(self.recall) {
                Block::ToolUse {
                    id: "write".into(),
                    name: "fs_write".into(),
                    input: json!({"path":"output.json","content":if self.recall {r#"[{"value":"wrong"}]"#} else {"[]"}}),
                }
            } else {
                Block::text("Done.")
            };
            let stop_reason = if matches!(content, Block::ToolUse { .. }) {
                StopReason::ToolUse
            } else {
                StopReason::EndTurn
            };
            Ok(CompletionResponse {
                message: Message::assistant(vec![content]),
                stop_reason,
                usage: Usage::default(),
                refusal: None,
                model: "authored".into(),
                malformed_tool_args: 0,
            })
        }
    }

    #[tokio::test]
    async fn native_cli_recall_protocol_failure_is_separate_from_artifact_failure() -> Result<()> {
        let fixture = Fixture(
            std::env::temp_dir()
                .canonicalize()?
                .join(format!("native-recall-cli-{}", uuid::Uuid::new_v4())),
        );
        fs::create_dir(&fixture.0)?;
        let db = fixture.0.join("native.memory-prototype.sqlite");
        let authority = fixture.0.join("authority");
        let request = fixture.0.join("request.json");
        let recall_request = fixture.0.join("recall.json");
        fs::write(
            &request,
            serde_json::to_vec(
                &json!({"prompt":"Save output.json.","files":{},"criterion":{"goal":"task:check","goal_text":"Check rows.","criterion_text":"Keep approved rows.","criterion_revision":"a".repeat(64),"confirmation_digest":"b".repeat(64)},"expected_rows":[],"domain":"synthetic","scopes":["synthetic:test"],"account":"synthetic"}),
            )?,
        )?;
        fs::write(
            &recall_request,
            r#"{"mode":"lexical","query":"calibration"}"#,
        )?;
        execute(parse_memory(&[
            "mecha",
            "memory",
            "--experimental",
            "--db",
            db.to_str().unwrap(),
            "init-native",
            "--synthetic-input",
            "--domain",
            "synthetic",
            "--authority",
            authority.to_str().unwrap(),
        ])?)
        .await?;
        for recall in [false, true] {
            let task = fixture.0.join(if recall {
                "wrong-artifact"
            } else {
                "missing-recall"
            });
            let args = parse_memory(&[
                "mecha",
                "memory",
                "--experimental",
                "--db",
                db.to_str().unwrap(),
                "native-session",
                "--synthetic-input",
                "--request",
                request.to_str().unwrap(),
                "--recall-request",
                recall_request.to_str().unwrap(),
                "--authority",
                authority.to_str().unwrap(),
                "--root",
                task.to_str().unwrap(),
                "--endpoint",
                "http://127.0.0.1:1",
                "--model",
                "authored",
                "--holds-dir",
                fixture.0.to_str().unwrap(),
            ])?;
            let result = execute_with_native_provider(args, |_, _| async {
                Ok(Box::new(AuthoredRecallProvider {
                    step: AtomicUsize::new(0),
                    recall,
                }) as Box<dyn Provider>)
            })
            .await;
            let report: serde_json::Value =
                serde_json::from_slice(&fs::read(task.join("report.json"))?)?;
            assert!(
                report["run_error"].is_null() && report["capture_error"].is_null(),
                "{report}"
            );
            if recall {
                assert!(
                    result.is_ok(),
                    "an artifact mismatch is a recorded outcome: {result:?}"
                );
                assert_eq!(report["check"]["verdict"], "Failed");
                assert_eq!(report["recall"]["status"], "completed");
            } else {
                assert!(result
                    .unwrap_err()
                    .to_string()
                    .contains("recall protocol failed: true"));
                assert_eq!(report["check"]["verdict"], "Passed");
                assert_eq!(report["recall"]["status"], "not_called");
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn native_cli_storage_reaches_preflight_without_using_a_real_provider() -> Result<()> {
        let fixture = Fixture(std::env::temp_dir().canonicalize()?.join(format!(
            "native-memory-cli-storage-{}",
            uuid::Uuid::new_v4()
        )));
        fs::create_dir(&fixture.0)?;
        let data = fixture.0.join("data");
        let owner = fixture.0.join("owner");
        fs::create_dir(&data)?;
        fs::DirBuilder::new().mode(0o700).create(&owner)?;
        let db = data.join("native.memory-prototype.sqlite");
        let authority = data.join("authority");
        let key = owner.join("key");
        let wrong = owner.join("wrong");
        let task = fixture.0.join("task");
        let request = fixture.0.join("request.json");
        fs::write(
            &request,
            serde_json::to_vec(&json!({
                "prompt":"Write the authored output.",
                "files":{},
                "criterion":{
                    "goal":"task:check",
                    "goal_text":"Write the selected row.",
                    "criterion_text":"The row must match the authored contract.",
                    "criterion_revision":"a".repeat(64),
                    "confirmation_digest":"b".repeat(64)
                },
                "expected_rows":[{"route":"Citrine"}],
                "domain":"synthetic",
                "scopes":["synthetic:cli"],
                "account":"synthetic"
            }))?,
        )?;
        let db_path = db.to_str().unwrap();
        let auth_path = authority.to_str().unwrap();
        let root_path = task.to_str().unwrap();
        let req_path = request.to_str().unwrap();
        let key_path = key.to_str().unwrap();
        let args = |key| {
            [
                "mecha",
                "memory",
                "--experimental",
                "--db",
                db_path,
                "--key-file",
                key,
            ]
        };
        for path in [key_path, wrong.to_str().unwrap()] {
            let mut generate = args(path).to_vec();
            generate.push("key-generate");
            execute(parse_memory(&generate)?).await?;
        }
        let mut init = args(key_path).to_vec();
        init.extend([
            "init-native",
            "--synthetic-input",
            "--domain",
            "synthetic",
            "--authority",
            auth_path,
        ]);
        execute(parse_memory(&init)?).await?;
        let bytes = fs::read(&db)?;
        let calls = AtomicUsize::new(0);
        for (path, expected) in [(wrong.to_str().unwrap(), 0), (key_path, 1)] {
            let mut native = args(path).to_vec();
            native.extend([
                "native-session",
                "--synthetic-input",
                "--request",
                req_path,
                "--authority",
                auth_path,
                "--root",
                root_path,
                "--endpoint",
                "http://127.0.0.1:1",
                "--model",
                "authored",
                "--holds-dir",
                data.to_str().unwrap(),
            ]);
            let result = execute_with_native_provider(parse_memory(&native)?, |_, _| async {
                calls.fetch_add(1, Ordering::Relaxed);
                anyhow::bail!("authored stop after preflight");
            })
            .await;
            assert!(result.is_err());
            assert_eq!(calls.load(Ordering::Relaxed), expected);
            assert_eq!(task.join("started.json").exists(), expected == 1);
            assert_eq!(fs::read(&db)?, bytes);
        }
        Ok(())
    }
}
