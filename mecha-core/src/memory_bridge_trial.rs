//! Bounded, explicit synthetic trials through the ordinary native agent loop.
//! Gold, source originals, authority and provider audit never enter the jail.
use crate::{
    agent::{Agent, Conversation, RunContext, Taint},
    appraisal::SessionRecords,
    config::{AgentConfig, PermissionMode, TrifectaPolicy},
    memory::{
        context::Situation,
        host_capture::{ArtifactCheck, CaptureReceipt, CheckVerdict, ConfirmedCriterion},
        Access, Memory, RevisionRef,
    },
    memory_experience_bridge::{CaptureRequest, FreezeRequest, LocalAuthority},
    message::{Block, CompletionRequest, CompletionResponse},
    provider::{Provider, StreamEvent, StreamSink},
    session::{Record, Session, SessionKind, SessionMeta},
    tool::{
        builtin::{FsRead, FsWrite},
        goal_memory::GoalMemory,
        todo::TodoTool,
        ModeApprover, Registry, ToolCtx,
    },
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const DOMAIN: &str = "synthetic-bridge-trial";
const ACCOUNT: &str = "synthetic";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialSource {
    pub database: PathBuf,
    pub authority: PathBuf,
    pub confirmation: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialRequest {
    pub version: u32,
    pub task_id: String,
    pub prompt: String,
    pub criterion: ConfirmedCriterion,
    pub expected_rows: Vec<BTreeMap<String, String>>,
    pub files: BTreeMap<String, String>,
    pub source: Option<TrialSource>,
    pub withdraw_source: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialConfig {
    /// Opt-in finalization advice; old serialized configurations remain off.
    #[serde(default, skip_serializing_if = "budget_notice_disabled")]
    pub budget_notice: bool,
    pub max_turns: u32,
    pub max_tokens: u32,
    pub max_output_tokens: u64,
    pub timeout_secs: u64,
}
fn budget_notice_disabled(value: &bool) -> bool {
    !*value
}
impl Default for TrialConfig {
    fn default() -> Self {
        Self {
            budget_notice: false,
            max_turns: 8,
            max_tokens: 4096,
            max_output_tokens: 16384,
            timeout_secs: 600,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialReport {
    pub version: u32,
    pub task_id: String,
    pub status: String,
    pub run_error: Option<String>,
    pub artifact_check: ArtifactCheck,
    pub capture_receipt: Option<CaptureReceipt>,
    pub capture_error: Option<String>,
    pub confirmation: Option<String>,
    pub database: PathBuf,
    pub authority: PathBuf,
    pub transcript: PathBuf,
    pub artifact: PathBuf,
    pub provider_audit: PathBuf,
    pub turns: u32,
    /// Provider attempts, including checks/errors before any HTTP inference.
    pub model_calls_started: u64,
    pub model_calls_completed: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub elapsed_ms: u64,
    pub recall_calls: u64,
    pub recall_references: Vec<RevisionRef>,
    pub taint: Taint,
    pub config: TrialConfig,
}
fn access() -> Access {
    Access {
        domain: DOMAIN.into(),
        scopes: BTreeSet::from(["synthetic:bridge-trial".into()]),
    }
}
fn digest_shape(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn flat(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        && name != "."
        && name != ".."
}
fn validate(root: &Path, r: &TrialRequest, c: &TrialConfig) -> Result<()> {
    ensure!(
        root.is_absolute() && !root.exists() && root.file_name().is_some(),
        "trial requires a NEW absolute directory"
    );
    ensure!(
        root.components()
            .all(|x| !matches!(x, Component::ParentDir | Component::CurDir)),
        "non-normal trial path"
    );
    ensure!(
        r.version == 1
            && !r.task_id.is_empty()
            && r.task_id.len() <= 128
            && !r.prompt.is_empty()
            && r.prompt.len() <= 16384,
        "invalid trial request"
    );
    let _: crate::goal::GoalRef = r.criterion.goal.parse()?;
    ensure!(
        r.criterion.goal_text.len() <= 2048
            && r.criterion.criterion_text.len() <= 2048
            && digest_shape(&r.criterion.criterion_revision)
            && digest_shape(&r.criterion.confirmation_digest),
        "invalid frozen criterion"
    );
    ensure!(
        r.files.len() <= 16
            && r.files
                .iter()
                .all(|(n, s)| flat(n) && n != "output.json" && s.len() <= 65536)
            && r.files.values().map(String::len).sum::<usize>() <= 262144,
        "invalid trial inputs"
    );
    ensure!(
        serde_json::to_vec(&r.expected_rows)?.len() <= 262144,
        "oversized trial gold"
    );
    ensure!(
        !r.withdraw_source || r.source.is_some(),
        "withdrawal requires explicit source"
    );
    if let Some(s) = &r.source {
        ensure!(
            s.database.is_absolute() && s.authority.is_absolute() && digest_shape(&s.confirmation),
            "invalid source paths/confirmation"
        );
    }
    ensure!(
        (1..=8).contains(&c.max_turns)
            && (1..=4096).contains(&c.max_tokens)
            && (1..=16384).contains(&c.max_output_tokens)
            && (1..=600).contains(&c.timeout_secs),
        "trial bounds cannot be enlarged"
    );
    Ok(())
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
// Host-selected source paths only. Reject symlinks/special files and bound every
// copy. These inputs are never deserialized from model tool arguments.
fn read_source(path: &Path, max: u64) -> Result<Vec<u8>> {
    use std::os::unix::fs::OpenOptionsExt;
    ensure!(path.is_absolute(), "source must be absolute");
    let canonical = path.canonicalize()?;
    ensure!(
        canonical == path,
        "source paths must be canonical and not symlinks"
    );
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.len() <= max,
        "source exceeds regular-file bound"
    );
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= max, "source exceeds bound");
    Ok(bytes)
}
fn clone_source(source: &TrialSource, report: &TrialReport) -> Result<()> {
    ensure!(
        source.authority.canonicalize()? == source.authority,
        "authority must be canonical"
    );
    for suffix in ["-wal", "-journal"] {
        let p = PathBuf::from(format!("{}{suffix}", source.database.display()));
        ensure!(
            !p.exists(),
            "source database must be closed without journal"
        );
    }
    write_new(
        &report.database,
        &read_source(&source.database, 64 * 1024 * 1024)?,
    )?;
    private_dir(&report.authority)?;
    let mut count = 0;
    let mut total = 0;
    for entry in fs::read_dir(&source.authority)? {
        let entry = entry?;
        count += 1;
        ensure!(count <= 256, "too many authority files");
        let name = entry.file_name();
        let name = name.to_str().context("invalid authority name")?;
        ensure!(flat(name), "invalid authority name");
        let bytes = read_source(&entry.path(), 8 * 1024 * 1024)?;
        total += bytes.len();
        ensure!(total <= 32 * 1024 * 1024, "oversized authority");
        write_new(&report.authority.join(name), &bytes)?;
    }
    let authority = LocalAuthority::open(&report.authority)?;
    authority.binding(&source.confirmation)?;
    Ok(())
}
fn private_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(path)?;
    Ok(())
}
#[derive(Default)]
struct AuditStats {
    started: u64,
    completed: u64,
    input: Option<u64>,
    output: Option<u64>,
    charged: u64,
}
struct Recorder {
    inner: Box<dyn Provider>,
    file: Mutex<File>,
    stats: Arc<Mutex<AuditStats>>,
    cap: u64,
    cancel: CancellationToken,
}

/// Reuse the trial's audited, bounded, fast-mode provider for another explicit
/// synthetic experiment. The audit is host-only and must be a new file.
pub fn recorded_provider(
    inner: Box<dyn Provider>,
    path: &Path,
    max_output_tokens: u64,
    cancel: CancellationToken,
) -> Result<Box<dyn Provider>> {
    use std::os::unix::fs::OpenOptionsExt;
    ensure!(
        path.is_absolute() && max_output_tokens > 0,
        "invalid trial audit configuration"
    );
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    Ok(Box::new(Recorder {
        inner,
        file: Mutex::new(file),
        stats: Arc::new(Mutex::new(AuditStats::default())),
        cap: max_output_tokens,
        cancel,
    }))
}
impl Recorder {
    fn effective(req: &CompletionRequest) -> CompletionRequest {
        let mut req = req.clone();
        req.think = Some(false);
        req.think_budget = None;
        req.local_only = true;
        req
    }
    async fn complete_recorded(
        &self,
        req: &CompletionRequest,
        sink: Option<&StreamSink>,
        call: u64,
    ) -> Result<CompletionResponse> {
        let Some(sink) = sink else {
            return self.inner.complete(req, None).await;
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let future = self.inner.complete(req, Some(&tx));
        tokio::pin!(future);
        let mut count = 0usize;
        let mut bytes = 0usize;
        loop {
            tokio::select! {
                biased;
                Some(event)=rx.recv()=>self.forward_event(event,sink,call,&mut count,&mut bytes)?,
                response=&mut future=>{
                    // A synchronous error may follow queued deltas in the same
                    // poll. Drain those actual events before retaining failure.
                    while let Ok(event)=rx.try_recv() {self.forward_event(event,sink,call,&mut count,&mut bytes)?;}
                    return response;
                }
            }
        }
    }
    fn forward_event(
        &self,
        event: StreamEvent,
        sink: &StreamSink,
        call: u64,
        count: &mut usize,
        bytes: &mut usize,
    ) -> Result<()> {
        let value = match &event {
            StreamEvent::TextDelta(text) => {
                json!({"event":"stream","call":call,"kind":"text_delta","text":text})
            }
            StreamEvent::ThinkingDelta(text) => {
                json!({"event":"stream","call":call,"kind":"thinking_delta","text":text})
            }
            StreamEvent::ToolUseStart { name } => {
                json!({"event":"stream","call":call,"kind":"tool_use_start","name":name})
            }
            StreamEvent::Usage(usage) => {
                json!({"event":"stream","call":call,"kind":"usage","native_usage":usage})
            }
        };
        *count += 1;
        *bytes += serde_json::to_vec(&value)?.len();
        if *count > 16384 || *bytes > 1024 * 1024 {
            self.append(json!({"event":"stream_refused","call":call,"reason":"bounded stream audit exceeded","events":count,"bytes":bytes}))?;
            anyhow::bail!("bounded stream audit exceeded");
        }
        self.append(value)?;
        let _ = sink.send(event);
        Ok(())
    }
    fn append(&self, value: serde_json::Value) -> Result<()> {
        let mut f = self
            .file
            .lock()
            .map_err(|_| anyhow::anyhow!("audit poisoned"))?;
        serde_json::to_writer(&mut *f, &value)?;
        f.write_all(b"\n")?;
        f.sync_data()?;
        Ok(())
    }
}
#[async_trait::async_trait]
impl Provider for Recorder {
    fn id(&self) -> &str {
        self.inner.id()
    }
    fn default_model(&self) -> &str {
        self.inner.default_model()
    }
    fn local(&self) -> bool {
        self.inner.local()
    }
    fn vision(&self) -> bool {
        self.inner.vision()
    }
    fn structured_output(&self) -> bool {
        self.inner.structured_output()
    }
    fn supports_effort(&self) -> bool {
        self.inner.supports_effort()
    }
    fn wire_body(&self, req: &CompletionRequest, stream: bool) -> Option<serde_json::Value> {
        self.inner.wire_body(&Self::effective(req), stream)
    }
    async fn complete(
        &self,
        req: &CompletionRequest,
        sink: Option<&StreamSink>,
    ) -> Result<CompletionResponse> {
        let mut req = Self::effective(req);
        let call = {
            let mut s = self
                .stats
                .lock()
                .map_err(|_| anyhow::anyhow!("audit poisoned"))?;
            ensure!(s.charged < self.cap, "trial total output budget exhausted");
            req.max_tokens = req.max_tokens.min((self.cap - s.charged) as u32);
            s.started += 1;
            s.charged += u64::from(req.max_tokens);
            s.started
        };
        self.append(json!({"event":"request_started","call":call,"request":{"model":req.model,"system":req.system,"messages":req.messages,"tools":req.tools,"max_tokens":req.max_tokens,"effort":req.effort,"thinking":req.thinking,"think":req.think,"think_budget":req.think_budget,"local_only":req.local_only,"cache_prompt":req.cache_prompt,"response_schema":req.response_schema,"trailing_notes":req.trailing_notes,"tool_choice":format!("{:?}",req.tool_choice)},"wire_body":self.inner.wire_body(&req,sink.is_some())}))?;
        crate::provider::refuse_unless_local(self.inner.as_ref(), &req)?;
        let response = self.complete_recorded(&req, sink, call).await;
        match &response {
            Ok(r) => {
                // Native Usage cannot distinguish absent usage from reported zero.
                // Conservatively retain null; reserve the request cap if unknown.
                let input = (r.usage.total_input() > 0).then_some(r.usage.total_input());
                let output = (r.usage.output_tokens > 0).then_some(r.usage.output_tokens);
                {
                    let mut s = self
                        .stats
                        .lock()
                        .map_err(|_| anyhow::anyhow!("audit poisoned"))?;
                    s.completed += 1;
                    s.input = if call == 1 {
                        input
                    } else {
                        s.input.zip(input).map(|(a, b)| a + b)
                    };
                    s.output = if call == 1 {
                        output
                    } else {
                        s.output.zip(output).map(|(a, b)| a + b)
                    };
                    if let Some(n) = output {
                        s.charged = s.charged - u64::from(req.max_tokens) + n;
                    }
                }
                self.append(json!({"event":"response","call":call,"model":r.model,"message":r.message,"stop_reason":r.stop_reason,"refusal":r.refusal,"malformed_tool_args":r.malformed_tool_args,"input_tokens":input,"output_tokens":output,"native_usage":r.usage}))?;
            }
            Err(e) => {
                let rejected=e.downcast_ref::<crate::memory_trial_provider::ReturnedModelMismatch>().map(|error| {
                    let r=&error.response;
                    json!({"model":r.model,"message":r.message,"stop_reason":r.stop_reason,"refusal":r.refusal,"malformed_tool_args":r.malformed_tool_args,"native_usage":r.usage})
                });
                self.append(json!({"event":"error","call":call,"error":format!("{e:#}"),"rejected_response":rejected}))?;
            }
        }
        // A provider may observe the same cancellation first and return an
        // error. Let the native loop take its cancellation branch, which drains
        // the streamed partial text instead of treating it as a transport loss.
        if self.cancel.is_cancelled() && response.is_err() {
            return std::future::pending().await;
        }
        response
    }
}

/// Runtime failures are retained as report data after structural validation.
/// Cancellation never invents a completed native outcome or retries execution.
pub async fn run_trial(
    root: &Path,
    request: TrialRequest,
    provider: Box<dyn Provider>,
    config: TrialConfig,
    cancel: CancellationToken,
) -> Result<TrialReport> {
    validate(root, &request, &config)?;
    let started = Instant::now();
    private_dir(root)?;
    let root = root.canonicalize()?;
    let mut report = TrialReport {
        version: 1,
        task_id: request.task_id.clone(),
        status: "error".into(),
        run_error: None,
        artifact_check: ArtifactCheck {
            artifact_digest: None,
            verdict: CheckVerdict::Unavailable,
        },
        capture_receipt: None,
        capture_error: None,
        confirmation: None,
        database: root.join("trial.memory-prototype.sqlite"),
        authority: root.join("host-authority"),
        transcript: root.join("sessions/trial.jsonl"),
        artifact: root.join("workspace/output.json"),
        provider_audit: root.join("provider-audit.jsonl"),
        turns: 0,
        model_calls_started: 0,
        model_calls_completed: 0,
        input_tokens: None,
        output_tokens: None,
        elapsed_ms: 0,
        recall_calls: 0,
        recall_references: vec![],
        taint: Taint::default(),
        config,
    };
    if let Err(error) = execute(&root, &request, provider, &cancel, &mut report, started).await {
        report.status = "error".into();
        report.run_error = Some(format!("{error:#}"));
    }
    report.elapsed_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    write_new(
        &root.join("report.json"),
        &serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}
async fn execute(
    root: &Path,
    request: &TrialRequest,
    provider: Box<dyn Provider>,
    cancel: &CancellationToken,
    report: &mut TrialReport,
    started: Instant,
) -> Result<()> {
    let workspace = root.join("workspace");
    private_dir(&workspace)?;
    for (name, content) in &request.files {
        write_new(&workspace.join(name), content.as_bytes())?;
    }
    if let Some(source) = &request.source {
        clone_source(source, report)?;
    } else {
        LocalAuthority::create(&report.authority)?;
        drop(Memory::create_host_capture(&report.database, DOMAIN)?);
    }
    let authority = LocalAuthority::open(&report.authority)?;
    if request.withdraw_source {
        let source = request.source.as_ref().context("source missing")?;
        authority.withdraw(&authority.binding(&source.confirmation)?.locator)?;
    }
    let mut memory = Memory::open(&report.database)?.with_source_guard(Arc::new(authority.clone()));
    let session = Session::create(
        &root.join("sessions"),
        SessionMeta {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: chrono::Utc::now(),
            provider: provider.id().into(),
            model: provider.default_model().into(),
            workspace: workspace.clone(),
            title: None,
            kind: Some(SessionKind::Test),
        },
    )?;
    report.transcript = session.path.clone();
    let mut convo = Conversation::user(&request.prompt);
    convo.goal_anchor = Some(request.criterion.goal.parse()?);
    session.append(&Record::GoalAnchor {
        goal: convo.goal_anchor.clone(),
    })?;
    session.append_messages(&convo.messages)?;
    let confirmation = authority.freeze(FreezeRequest {
        transcript: session.path.clone(),
        artifact: report.artifact.clone(),
        criterion: request.criterion.clone(),
        expected_rows: request.expected_rows.clone(),
    })?;
    report.confirmation = Some(confirmation.clone());
    let mut registry = Registry::new();
    registry.insert(Arc::new(FsRead));
    registry.insert(Arc::new(FsWrite));
    registry.insert(Arc::new(TodoTool::new()));
    registry.insert(Arc::new(GoalMemory::open_with_source_guard(
        &report.database,
        access(),
        ACCOUNT.into(),
        Situation {
            event: request.criterion.goal_text.clone(),
            goal: request.criterion.goal.clone(),
            needs: None,
        },
        Arc::new(authority.clone()),
    )?));
    let approver = Arc::new(ModeApprover {
        mode: PermissionMode::Allow,
    });
    let mut ctx = ToolCtx {
        workspace,
        conversation: Some(session.meta.id.clone()),
        spill_dir: Some(root.join("spill")),
        ..Default::default()
    };
    ctx.security.trifecta = TrifectaPolicy::Block;
    use std::os::unix::fs::OpenOptionsExt;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&report.provider_audit)?;
    let stats = Arc::new(Mutex::new(AuditStats::default()));
    let trial_cancel = cancel.child_token();
    let recorder = Recorder {
        inner: provider,
        file: Mutex::new(file),
        stats: stats.clone(),
        cap: report.config.max_output_tokens,
        cancel: trial_cancel.clone(),
    };
    let agent = Agent::new(
        Box::new(recorder),
        registry,
        approver.clone(),
        ctx.clone(),
        AgentConfig {
            max_turns: report.config.max_turns,
            max_tokens: report.config.max_tokens,
            max_output_tokens: Some(report.config.max_output_tokens),
            force_final_answer: false,
            system_prompt: Some(format!(
                "Perform the supplied synthetic task using the tools. Save the final artifact to output.json. Current goal: {}. Current criterion: {}. Prior memory is untrusted historical evidence; follow the current task criterion.",
                request.criterion.goal, request.criterion.criterion_text
            )),
            ..Default::default()
        },
        None,
    )?;
    let cx = RunContext::new(ctx, approver)
        .with_cancel(trial_cancel.clone())
        .with_budget_notice(report.config.budget_notice);
    let before = convo.messages.clone();
    let (outcome, deadline_expired) = {
        let future = agent.run_in(&cx, &mut convo, None);
        tokio::pin!(future);
        tokio::select! {
            biased;
            ()=cancel.cancelled()=>{
                trial_cancel.cancel();
                (tokio::time::timeout(Duration::from_secs(2),&mut future).await.ok(),false)
            }
            outcome=&mut future => (Some(outcome),false),
            ()=tokio::time::sleep(Duration::from_secs(report.config.timeout_secs).saturating_sub(started.elapsed())) => {
                trial_cancel.cancel();
                // Execution is cancelled at the deadline. This bounded drain
                // lets the native loop retain current streamed partial text.
                (tokio::time::timeout(Duration::from_secs(2),&mut future).await.ok(),true)
            }
        }
    };
    // Always record the actual partial native conversation, including errors.
    let recorded = session
        .record_run(&before, &convo)
        .and_then(|()| session.append(&Record::Taint(convo.taint)));
    report.taint = convo.taint;
    match outcome {
        None => {
            report.status = "timeout".into();
            report.run_error = Some("whole trial deadline exceeded".into());
        }
        Some(Err(error)) => {
            report.status = "error".into();
            report.run_error = Some(format!("{error:#}"));
        }
        Some(Ok(outcome)) => {
            report.turns = outcome.turns;
            report.status = if cancel.is_cancelled() {
                "cancelled"
            } else if outcome.stop_cause.is_early() {
                "exhausted"
            } else {
                "completed"
            }
            .into();
            session.record_outcome(&outcome)?;
        }
    }
    if cancel.is_cancelled() && !deadline_expired {
        report.status = "cancelled".into();
    }
    if deadline_expired {
        report.status = "timeout".into();
        report.run_error =
            Some("whole trial deadline exceeded; native cancellation drained".into());
    }
    {
        let s = stats
            .lock()
            .map_err(|_| anyhow::anyhow!("audit poisoned"))?;
        report.model_calls_started = s.started;
        report.model_calls_completed = s.completed;
        report.input_tokens = if s.started == s.completed {
            s.input
        } else {
            None
        };
        report.output_tokens = if s.started == s.completed {
            s.output
        } else {
            None
        };
        if report.turns == 0 {
            report.turns = s.completed.try_into().unwrap_or(u32::MAX);
        }
    }
    recall(&convo, report);
    recorded?;
    report.artifact_check = authority.check_artifact(&confirmation)?;
    match authority.capture(
        &mut memory,
        CaptureRequest {
            confirmation,
            domain: DOMAIN.into(),
            account: ACCOUNT.into(),
            scopes: access().scopes,
            consulted: vec![],
            records: SessionRecords::default(),
        },
        &access(),
    ) {
        Ok(receipt) => report.capture_receipt = Some(receipt),
        Err(error) => report.capture_error = Some(error.to_string()),
    }
    Ok(())
}
fn recall(convo: &Conversation, report: &mut TrialReport) {
    let mut calls = BTreeSet::new();
    for message in &convo.messages {
        for block in &message.content {
            match block {
                Block::ToolUse { id, name, input }
                    if name == "goal_context" && input.get("memory").is_some() =>
                {
                    calls.insert(id.clone());
                    report.recall_calls += 1;
                }
                Block::ToolResult {
                    tool_use_id,
                    content,
                    is_error: false,
                } if calls.contains(tool_use_id) => {
                    // Unwrap only the native external-content envelope. References come
                    // from successful native result packets, never assistant assertions.
                    let raw = content
                        .strip_prefix("<untrusted-content source=\"goal_context\">\n")
                        .and_then(|s| s.split_once("---\n").map(|(_, v)| v))
                        .and_then(|s| s.strip_suffix("\n</untrusted-content>"))
                        .unwrap_or(content);
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) {
                        if let Some(records) =
                            value.pointer("/memory/records").and_then(|v| v.as_array())
                        {
                            for record in records {
                                if let Some(reference) = record.get("reference").and_then(|v| {
                                    serde_json::from_value::<RevisionRef>(v.clone()).ok()
                                }) {
                                    if !report.recall_references.contains(&reference) {
                                        report.recall_references.push(reference);
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}
#[cfg(test)]
#[path = "memory_bridge_trial_tests.rs"]
mod tests;
