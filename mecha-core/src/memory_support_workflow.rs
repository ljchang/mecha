//! Frozen synthetic factual-support qualification on the ordinary native paths.
//! Authored tool execution establishes actions; independent later artifact reads
//! establish observations. Neither model pass grants action or factual authority.
use crate::{
    agent::{Agent, Conversation, RunContext},
    appraisal::SessionRecords,
    config::{AgentConfig, PermissionMode, TrifectaPolicy},
    memory::{
        context::{ContextPolicy, ContextRequest, Situation},
        host_capture::{CaptureReceipt, ConfirmedCriterion},
        interpretation::{
            InterpretationBearing, InterpretationContext, JudgmentReason, PreparedInterpretation,
            ProposedJudgment, ProposedUnit, SourceCitation, SourceKind, SupportProposal,
            SupportReply, SupportReport, UnitDisposition, UnitKind,
        },
        Access, Memory, RecallQuery, Record as MemoryRecord,
    },
    memory_bridge_trial::recorded_provider,
    memory_experience_bridge::{CaptureRequest, FreezeRequest, LocalAuthority},
    memory_factual_support::{self as support, GenerationBudget, PassOutcome},
    memory_situated_needs::{new_context_id, NeedsInput},
    message::{Block, CompletionRequest, CompletionResponse, Message, StopReason, Usage},
    provider::{Provider, StreamSink},
    session::{Record, Session, SessionKind, SessionMeta},
    tool::{
        builtin::{FsRead, FsWrite},
        goal_memory::GoalMemory,
        ModeApprover, Registry, Tool, ToolCtx,
    },
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

pub const PROTOCOL: &str = "memory-factual-support-v1";
pub const GROUNDED_PROTOCOL: &str = "memory-factual-support-v2";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupportProtocol {
    LegacyV1,
    GroundedV2,
}
const DOMAIN: &str = "synthetic-support-workflow";
const ACCOUNT: &str = "synthetic";
const STAGES: [&str; 8] = [
    "source",
    "capture",
    "generate",
    "evaluate_generated",
    "evaluate_probe",
    "retain",
    "recall",
    "changed_goal",
];

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCase {
    pub input_text: String,
    pub requested_output: String,
    pub observed_output: Option<String>,
    pub historical_criterion: String,
    pub current_criterion: String,
    pub goal_text: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CitationSelector {
    pub passage: String,
    pub quote: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeUnit {
    pub id: String,
    pub kind: UnitKind,
    pub text: String,
    pub citation: CitationSelector,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeJudgment {
    pub bearing: InterpretationBearing,
    pub because: Vec<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeCase {
    pub units: Vec<ProbeUnit>,
    pub judgments: Vec<ProbeJudgment>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialCase {
    pub version: u32,
    pub case_id: String,
    pub case_sha256: String,
    pub native: NativeCase,
    pub probe: ProbeCase,
}
impl TrialCase {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1
                && !self.case_id.is_empty()
                && self.case_id.len() <= 64
                && self
                    .case_id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "invalid synthetic case identity"
        );
        ensure!(
            self.case_sha256.len() == 64
                && self
                    .case_sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "invalid frozen case digest"
        );
        for text in [
            &self.native.input_text,
            &self.native.requested_output,
            &self.native.historical_criterion,
            &self.native.current_criterion,
            &self.native.goal_text,
        ] {
            ensure!(
                !text.is_empty() && text.len() <= 2048,
                "invalid native case text"
            );
        }
        ensure!(
            self.native
                .observed_output
                .as_ref()
                .is_none_or(|s| s.len() <= 2048),
            "oversized synthetic observation"
        );
        let _: Vec<BTreeMap<String, String>> = serde_json::from_str(&self.native.requested_output)?;
        ensure!(
            self.probe.units.len() == 3 && self.probe.judgments.len() == 2,
            "invalid frozen probe count"
        );
        for unit in &self.probe.units {
            ensure!(
                ![
                    "true",
                    "false",
                    "uncertain",
                    "supported",
                    "contradicted",
                    "unknown"
                ]
                .contains(&unit.id.to_lowercase().as_str()),
                "probe IDs must not expose host labels"
            );
            ensure!(
                ["artifact", "input", "action"].contains(&unit.citation.passage.as_str()),
                "invalid citation selector"
            );
            if let Some(quote) = &unit.citation.quote {
                ensure!(
                    (12..=300).contains(&quote.chars().count()),
                    "invalid exact quote length"
                );
            }
        }
        Ok(())
    }
}

/// Owner-selected input only; reject symlinks, nonregular files and overbounds.
pub fn load_case(path: &Path) -> Result<TrialCase> {
    use std::os::unix::fs::OpenOptionsExt;
    ensure!(
        path.is_absolute() && path.canonicalize()? == path,
        "canonical absolute case file required"
    );
    for parent in path.ancestors() {
        ensure!(
            !fs::symlink_metadata(parent)?.file_type().is_symlink(),
            "case symlink refused"
        );
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= 65536,
        "case must be a regular file <=64KiB"
    );
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(65537)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 65536, "case grew past limit");
    let case: TrialCase = serde_json::from_slice(&bytes)?;
    case.validate()?;
    Ok(case)
}

/// A factory selects only existing Provider implementations; it executes no calls.
#[async_trait::async_trait]
pub trait StageProviders: Send + Sync {
    async fn create(&self, stage: &str, cancel: &CancellationToken) -> Result<Box<dyn Provider>>;
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
fn write_value(path: &Path, value: &impl Serialize) -> Result<()> {
    write_new(path, &serde_json::to_vec(value)?)
}
fn checkpoint(root: &Path, report: &Value) -> Result<()> {
    let temporary = root.join("report.next.json");
    write_value(&temporary, report)?;
    fs::rename(temporary, root.join("report.json"))?;
    Ok(())
}
fn descriptor(root: &Path, path: &Path) -> Result<Value> {
    ensure!(
        path.starts_with(root) && !fs::symlink_metadata(path)?.file_type().is_symlink(),
        "invalid host artifact"
    );
    let bytes = fs::read(path)?;
    ensure!(
        bytes.len() <= 1024 * 1024,
        "host report artifact exceeds bound"
    );
    Ok(json!({"path":path.strip_prefix(root)?,"sha256":digest(&bytes),"bytes":bytes.len()}))
}
fn criterion(native: &NativeCase, current: bool) -> ConfirmedCriterion {
    let text = if current {
        &native.current_criterion
    } else {
        &native.historical_criterion
    };
    ConfirmedCriterion {
        goal: "task:synthetic-support".into(),
        goal_text: native.goal_text.clone(),
        criterion_text: text.clone(),
        criterion_revision: digest(text.as_bytes()),
        confirmation_digest: digest(format!("synthetic-owner-confirmation:{text}").as_bytes()),
    }
}

struct Script(Mutex<VecDeque<Message>>);
#[async_trait::async_trait]
impl Provider for Script {
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
        _: &CompletionRequest,
        _: Option<&StreamSink>,
    ) -> Result<CompletionResponse> {
        let message = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("authored source poisoned"))?
            .pop_front()
            .context("unexpected authored source call")?;
        let stop_reason = if message
            .content
            .iter()
            .any(|b| matches!(b, Block::ToolUse { .. }))
        {
            StopReason::ToolUse
        } else {
            StopReason::EndTurn
        };
        Ok(CompletionResponse {
            message,
            stop_reason,
            usage: Usage::default(),
            refusal: None,
            model: "authored".into(),
            malformed_tool_args: 0,
        })
    }
}
fn tool_call(id: &str, name: &str, input: Value) -> Message {
    Message::assistant(vec![Block::ToolUse {
        id: id.into(),
        name: name.into(),
        input,
    }])
}
struct SourceFixture {
    input: PathBuf,
    artifact: PathBuf,
    transcript: PathBuf,
    database: PathBuf,
    authority: LocalAuthority,
    access: Access,
    confirmation: String,
}
impl SourceFixture {
    fn memory(&self) -> Result<Memory> {
        Ok(Memory::open(&self.database)?.with_source_guard(Arc::new(self.authority.clone())))
    }
    fn capture(&self, memory: &mut Memory) -> Result<CaptureReceipt> {
        self.authority.capture_with_evidence(
            memory,
            CaptureRequest {
                confirmation: self.confirmation.clone(),
                domain: self.access.domain.clone(),
                account: ACCOUNT.into(),
                scopes: self.access.scopes.clone(),
                consulted: vec![],
                records: SessionRecords::default(),
            },
            &self.access,
        )
    }
}
async fn execute_task(
    root: &Path,
    native: &NativeCase,
    cancel: &CancellationToken,
) -> Result<SourceFixture> {
    ensure!(
        root.is_absolute() && !root.exists(),
        "new absolute native source root required"
    );
    fs::create_dir(root)?;
    let root = root.canonicalize()?;
    let workspace = root.join("workspace");
    fs::create_dir(&workspace)?;
    let input = workspace.join("input.txt");
    write_new(&input, native.input_text.as_bytes())?;
    let artifact = workspace.join("output.json");
    let database = root.join("support.memory-prototype.sqlite");
    drop(Memory::create_supported_situated(&database, DOMAIN)?);
    let authority = LocalAuthority::create(&root.join("authority"))?;
    let access = Access {
        domain: DOMAIN.into(),
        scopes: ["synthetic:support".into()].into(),
    };
    let historical = criterion(native, false);
    let session = Session::create(
        &root.join("sessions"),
        SessionMeta {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: chrono::Utc::now(),
            provider: "authored".into(),
            model: "authored".into(),
            workspace: workspace.clone(),
            title: None,
            kind: Some(SessionKind::Test),
        },
    )?;
    let mut conversation = Conversation::user(format!("{}. Read input.txt. Write exactly these requested bytes to output.json: {}. Confirmed historical criterion: {}.",
        native.goal_text, native.requested_output, native.historical_criterion));
    conversation.goal_anchor = Some(historical.goal.parse()?);
    session.append(&Record::GoalAnchor {
        goal: conversation.goal_anchor.clone(),
    })?;
    session.append_messages(&conversation.messages)?;
    let confirmation = authority.freeze(FreezeRequest {
        transcript: session.path.clone(),
        artifact: artifact.clone(),
        criterion: historical,
        expected_rows: serde_json::from_str(&native.requested_output)?,
    })?;
    let script = Script(Mutex::new(VecDeque::from([
        tool_call("read-input", "fs_read", json!({"path":"input.txt"})),
        tool_call(
            "write-output",
            "fs_write",
            json!({"path":"output.json","content":native.requested_output}),
        ),
        Message::assistant(vec![Block::text("The native file write completed.")]),
    ])));
    let mut registry = Registry::new();
    registry.insert(Arc::new(FsRead));
    registry.insert(Arc::new(FsWrite));
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
    let agent = Agent::new(
        Box::new(script),
        registry,
        approver.clone(),
        ctx.clone(),
        AgentConfig {
            max_turns: 4,
            max_tokens: 4096,
            max_output_tokens: Some(16384),
            force_final_answer: false,
            system_prompt: Some(
                "Complete the synthetic owner request through native tools.".into(),
            ),
            ..Default::default()
        },
        None,
    )?;
    let before = conversation.messages.clone();
    let cx = RunContext::new(ctx, approver).with_cancel(cancel.clone());
    let outcome = agent.run_in(&cx, &mut conversation, None).await;
    session.record_run(&before, &conversation)?;
    session.append(&Record::Taint(conversation.taint))?;
    let outcome = outcome?;
    session.record_outcome(&outcome)?;
    ensure!(
        !cancel.is_cancelled() && !outcome.stop_cause.is_early(),
        "authored source cancelled or exhausted"
    );
    ensure!(
        fs::read(&artifact)? == native.requested_output.as_bytes(),
        "native write did not produce requested bytes"
    );
    // Host-only world fixture change. Never append this cause/actor to the native
    // transcript: inference may know only the recorded action and later bytes.
    match &native.observed_output {
        Some(bytes) => fs::write(&artifact, bytes.as_bytes())?,
        None => fs::remove_file(&artifact)?,
    }
    Ok(SourceFixture {
        input,
        artifact,
        transcript: session.path,
        database,
        authority,
        access,
        confirmation,
    })
}

async fn read_memory(
    source: &SourceFixture,
    context: &InterpretationContext,
    interpretations: bool,
) -> Result<Value> {
    let mut tool = GoalMemory::open_with_source_guard(
        &source.database,
        source.access.clone(),
        ACCOUNT.into(),
        Situation {
            event: "Read the recorded native output and observations.".into(),
            goal: context.criterion().goal.clone(),
            needs: None,
        },
        Arc::new(source.authority.clone()),
    )?;
    if interpretations {
        tool = tool.with_interpretations(context.read_policy())?;
    }
    let output = tool
        .call(json!({"memory":{"query":"output"}}), &ToolCtx::default())
        .await?;
    ensure!(
        !output.is_error,
        "ordinary GoalMemory recall refused: {}",
        output.content
    );
    Ok(serde_json::from_str(&output.content)?)
}

fn probe_proposal(case: &TrialCase, prepared: &PreparedInterpretation) -> Result<SupportProposal> {
    let mut units = Vec::new();
    for offered in &case.probe.units {
        let source = prepared
            .sources()
            .iter()
            .find(|source| match offered.citation.passage.as_str() {
                "artifact" => source.kind == SourceKind::ArtifactObservation,
                "action" => {
                    source.kind == SourceKind::ActionInvocation && source.text.contains("fs_write")
                }
                "input" => {
                    source.kind == SourceKind::Observation
                        && source.text.contains(case.native.input_text.trim())
                }
                _ => false,
            })
            .context("frozen probe source selector unavailable")?;
        let quote = offered
            .citation
            .quote
            .as_ref()
            .unwrap_or(&source.text)
            .clone();
        ensure!(
            (12..=300).contains(&quote.chars().count()) && source.text.contains(&quote),
            "complete exact probe quote unavailable; no clipping permitted"
        );
        units.push(ProposedUnit {
            id: offered.id.clone(),
            kind: offered.kind,
            text: offered.text.clone(),
            citations: vec![SourceCitation {
                source_id: source.id.clone(),
                quote,
            }],
        });
    }
    Ok(SupportProposal {
        units,
        judgments: case
            .probe
            .judgments
            .iter()
            .map(|j| ProposedJudgment {
                goal: prepared.context().criterion().goal.clone(),
                bearing: j.bearing,
                because: j.because.clone(),
            })
            .collect(),
    })
}

fn packet_bytes(request: &CompletionRequest) -> usize {
    request.system.as_ref().map_or(0, String::len)
        + request.response_schema.as_ref().map_or(0, |schema| {
            serde_json::to_vec(schema)
                .expect("JSON schema serializes")
                .len()
        })
        + request
            .messages
            .iter()
            .map(|m| m.text().len())
            .sum::<usize>()
}
fn set_stage(report: &mut Value, name: &str, status: &str, error: Option<String>) {
    report["stages"][name]["status"] = json!(status);
    report["stages"][name]["error"] = json!(error);
}
fn write_audit_exports<T>(
    root: &Path,
    report: &mut Value,
    name: &str,
    outcome: &PassOutcome<T>,
) -> Result<()> {
    let stage = &mut report["stages"][name];
    stage["model"] = json!(outcome.served_model);
    stage["elapsed_ms"] = json!(outcome.elapsed.as_millis());
    stage["reply_bytes"] = json!(outcome.reply_bytes);
    if let Some(text) = &outcome.raw_reply {
        write_new(&root.join(format!("{name}-raw.txt")), text.as_bytes())?;
    }
    let audit_path = root.join(format!("{name}-provider.jsonl"));
    if audit_path.exists() {
        stage["provider_audit"] = descriptor(root, &audit_path)?;
        for line in fs::read_to_string(&audit_path)?.lines() {
            let event: Value = serde_json::from_str(line)?;
            let kind = event["event"].as_str().unwrap_or_default();
            if kind == "error" {
                if let Ok(usage) = serde_json::from_value::<Usage>(
                    event["rejected_response"]["native_usage"].clone(),
                ) {
                    // Identity rejection still incurred known usage. As in the
                    // recorder's normal response path, zero cannot distinguish
                    // reported zero from absent native usage and stays unknown.
                    let input = usage
                        .input_tokens
                        .checked_add(usage.cache_creation_input_tokens)
                        .and_then(|n| n.checked_add(usage.cache_read_input_tokens))
                        .filter(|n| *n > 0);
                    stage["input_tokens"] = json!(input);
                    stage["output_tokens"] =
                        json!((usage.output_tokens > 0).then_some(usage.output_tokens));
                    stage["usage_source"] = json!("rejected_response");
                }
            }
            let label = match kind {
                "request_started" => "request",
                "response" => "reply",
                _ => continue,
            };
            let path = root.join(format!("{name}-{label}.json"));
            write_new(&path, line.as_bytes())?;
            stage[label] = descriptor(root, &path)?;
            if kind == "response" {
                stage["input_tokens"] = event["input_tokens"].clone();
                stage["output_tokens"] = event["output_tokens"].clone();
                stage["stop_reason"] = event["stop_reason"].clone();
            }
        }
    }
    Ok(())
}
async fn stage_provider(
    root: &Path,
    name: &str,
    providers: &dyn StageProviders,
    cancel: &CancellationToken,
) -> Result<Box<dyn Provider>> {
    ensure!(
        !cancel.is_cancelled(),
        "case cancelled before provider setup"
    );
    recorded_provider(
        providers.create(name, cancel).await?,
        &root.join(format!("{name}-provider.jsonl")),
        4096,
        cancel.clone(),
    )
}

fn durable_support(
    memory: &Memory,
    receipt: &crate::memory::interpretation::InterpretationReceipt,
    access: &Access,
    context: &InterpretationContext,
) -> Result<SupportReport> {
    let packet = memory.context_with_interpretations(
        &ContextRequest {
            query: "output".into(),
            bindings: vec![],
        },
        &ContextPolicy {
            access: access.clone(),
            recall: RecallQuery {
                text: "output".into(),
                known_at: memory.snapshot()?,
                account: Some(ACCOUNT.into()),
                session_id: None,
                occurred_after: None,
                occurred_before: None,
                limit: 8,
                neighbors: 0,
                max_bytes: 20_000,
            },
            valid_at: None,
        },
        &Situation {
            event: "Read the evaluated native output record.".into(),
            goal: context.criterion().goal.clone(),
            needs: None,
        },
        &context.read_policy(),
    )?;
    let hit = packet
        .records
        .into_iter()
        .find(|hit| hit.reference == receipt.reference)
        .context("stored evaluated interpretation withheld by contextual reader")?;
    match hit.record {
        MemoryRecord::Interpretation(record) => record
            .draft
            .support
            .map(|report| *report)
            .context("stored evaluated report missing"),
        _ => anyhow::bail!("retained interpretation did not reopen as an interpretation"),
    }
}
fn probe_results(stored: &SupportReport, reply: &SupportReply) -> Value {
    let units: Vec<_> = stored
        .units
        .iter()
        .map(|entry| {
            let (id, status, admitted) = match entry {
                UnitDisposition::Accepted { unit, status } => (&unit.id, status, true),
                UnitDisposition::Rejected { id, status } => (id, status, false),
            };
            let check = reply
                .checks
                .iter()
                .find(|c| c.target == format!("unit:{id}"));
            json!({"id":id,"status":status,"admitted":admitted,"withheld":!admitted,
            "unsupported_factual_premise":check.map(|c|c.unsupported_factual_premise)})
        })
        .collect();
    let judgments: Vec<_> = stored.judgments.iter().map(|j| {
        let disposition = match j.reason { JudgmentReason::Retained | JudgmentReason::ExplicitUnknown => "retained",
            JudgmentReason::DuplicateGoal => "dropped", _ => "downgraded" };
        let check = reply.checks.iter().find(|c| c.target == format!("judgment:{}", j.index));
        json!({"index":j.index,"status":j.status,"offered_bearing":j.offered_bearing,"bearing":j.bearing,
            "disposition":disposition,"reason":j.reason,"unsupported_factual_premise":check.map(|c|c.unsupported_factual_premise)})
    }).collect();
    json!({"units":units,"judgments":judgments})
}

/// Execute one frozen case. A failed case still writes all eight stage slots.
/// The caller owns the one600-second cancellation token; no stage retries.
pub async fn run_case(
    root: &Path,
    case: &TrialCase,
    model: &str,
    providers: &dyn StageProviders,
    cancel: &CancellationToken,
) -> Result<Value> {
    run_case_with_protocol(
        root,
        case,
        model,
        providers,
        cancel,
        SupportProtocol::LegacyV1,
    )
    .await
}

pub async fn run_case_with_protocol(
    root: &Path,
    case: &TrialCase,
    model: &str,
    providers: &dyn StageProviders,
    cancel: &CancellationToken,
    protocol: SupportProtocol,
) -> Result<Value> {
    case.validate()?;
    ensure!(
        root.is_absolute() && !root.exists(),
        "new absolute case root required"
    );
    fs::create_dir(root)?;
    let root = root.canonicalize()?;
    let started = Instant::now();
    let stages: serde_json::Map<String, Value> = STAGES
        .into_iter()
        .map(|name| (name.into(), json!({"status":"pending","error":null})))
        .collect();
    let protocol_name = match protocol {
        SupportProtocol::LegacyV1 => PROTOCOL,
        SupportProtocol::GroundedV2 => GROUNDED_PROTOCOL,
    };
    let mut report = json!({"protocol":protocol_name,"case_id":case.case_id,"case_sha256":case.case_sha256,
        "model":model,"seed":70510,"status":"failed","run_error":null,"stages":stages,
        "active_stage":"source","artifacts":{"initial_input":null,"native_transcript":null,"observed_output":null,"admitted_generated":null},
        "initial_hashes":null,"source_seal":null,"support_reports":{"generated":null,"probe":null},
        "probe":{"units":[],"judgments":[]},"input_tokens":null,"output_tokens":null});
    checkpoint(&root, &report)?;
    let outcome = run_inner(&root, case, model, providers, cancel, protocol, &mut report).await;
    if let Err(error) = outcome {
        let active = report["active_stage"]
            .as_str()
            .unwrap_or("source")
            .to_string();
        set_stage(&mut report, &active, "failed", Some(format!("{error:#}")));
        report["run_error"] = json!(format!("{error:#}"));
    }
    for name in STAGES {
        if report["stages"][name]["status"] == "pending" {
            set_stage(
                &mut report,
                name,
                "withheld",
                Some("upstream stage unavailable".into()),
            );
        }
    }
    for key in ["input_tokens", "output_tokens"] {
        let values: Vec<_> = ["generate", "evaluate_generated", "evaluate_probe"]
            .iter()
            .map(|name| report["stages"][name][key].as_u64())
            .collect();
        report[format!("observed_{key}")] = json!(values.iter().flatten().sum::<u64>());
        report[key] = json!(values
            .iter()
            .copied()
            .collect::<Option<Vec<_>>>()
            .map(|v| v.iter().sum::<u64>()));
    }
    let failed = STAGES
        .iter()
        .any(|name| report["stages"][name]["status"] == "failed");
    if !failed && !cancel.is_cancelled() {
        report["status"] = json!("completed");
        report["run_error"] = Value::Null;
    } else if report["run_error"].is_null() {
        report["run_error"] = json!("case contains failed stages or was cancelled");
    }
    report["elapsed_ms"] = json!(started.elapsed().as_millis());
    report["cancelled"] = json!(cancel.is_cancelled());
    report
        .as_object_mut()
        .context("invalid report")?
        .remove("active_stage");
    checkpoint(&root, &report)?;
    Ok(report)
}

async fn run_inner(
    root: &Path,
    case: &TrialCase,
    model: &str,
    providers: &dyn StageProviders,
    cancel: &CancellationToken,
    protocol: SupportProtocol,
    report: &mut Value,
) -> Result<()> {
    let source = execute_task(&root.join("native"), &case.native, cancel).await?;
    report["artifacts"]["initial_input"] = descriptor(root, &source.input)?;
    report["artifacts"]["native_transcript"] = descriptor(root, &source.transcript)?;
    if case.native.observed_output.is_some() {
        report["artifacts"]["observed_output"] = descriptor(root, &source.artifact)?;
    }
    report["initial_hashes"] = json!({"initial_input":report["artifacts"]["initial_input"]["sha256"],
        "native_transcript":report["artifacts"]["native_transcript"]["sha256"],
        "observed_output":report["artifacts"]["observed_output"].get("sha256").cloned().unwrap_or(Value::Null)});
    let seal_path = root.join("source-seal.json");
    write_value(&seal_path, &report["initial_hashes"])?;
    report["source_seal"] = descriptor(root, &seal_path)?;
    set_stage(report, "source", "completed", None);
    checkpoint(root, report)?;
    report["active_stage"] = json!("capture");
    let mut memory = source.memory()?;
    let capture = source.capture(&mut memory)?;
    write_value(&root.join("source-capture.json"), &capture)?;
    let current = InterpretationContext::new(criterion(&case.native, true), new_context_id())?;
    let source_packet = read_memory(&source, &current, false).await?;
    write_value(&root.join("source-only-packet.json"), &source_packet)?;
    let capture_bytes = serde_json::to_vec(&source_packet["memory"])?.len();
    let source_records = source_packet["memory"]["records"]
        .as_array()
        .context("source-only recall has no records array")?;
    ensure!(
        source_records.iter().any(|record| {
            record["reference"] == json!(capture.projection)
                || capture
                    .originals
                    .iter()
                    .any(|reference| record["reference"] == json!(reference))
        }),
        "source-only recall did not include captured source references"
    );
    ensure!(
        capture_bytes <= 20000 && source_packet["memory"]["budget_omitted"] != true,
        "complete source-only packet withheld by atomic budget"
    );
    let prepared = memory.prepare_interpretation(
        &capture,
        &[],
        &current,
        &memory.snapshot()?,
        &source.access,
    )?;
    // Keep all controls source-only before either proposal is evaluated/retained.
    let source_control = root.join("source-only.memory-prototype.sqlite");
    fs::copy(&source.database, &source_control)?;
    let probe_database = root.join("probe.memory-prototype.sqlite");
    fs::copy(&source.database, &probe_database)?;
    report["source_only_database"] = descriptor(root, &source_control)?;
    report["source_only_packet"] = descriptor(root, &root.join("source-only-packet.json"))?;
    report["capture_receipt"] = descriptor(root, &root.join("source-capture.json"))?;
    report["stages"]["capture"]["packet_bytes"] = json!(capture_bytes);
    report["stages"]["capture"]["withheld"] = json!(false);
    let now = chrono::Utc::now();
    let missing = NeedsInput::new(now, Duration::from_secs(60), None).assess(now)?;
    let empty = NeedsInput::new(now, Duration::from_secs(60), Some(vec![])).assess(now)?;
    ensure!(missing != empty, "authored needs cases did not differ");
    let alternate = InterpretationContext::new(current.criterion().clone(), new_context_id())?;
    let alternate_prepared = memory.prepare_interpretation(
        &capture,
        &[],
        &alternate,
        &memory.snapshot()?,
        &source.access,
    )?;
    report["needs_request_identical"] =
        json!(prepared.generation_input() == alternate_prepared.generation_input());
    ensure!(
        report["needs_request_identical"] == true,
        "private needs/context identity changed request"
    );
    if protocol == SupportProtocol::GroundedV2 {
        // Opaque context-binding keys may differ; private needs never enter the semantic input.
        report["needs_invariance_scope"] = json!("canonical_source_and_criterion_input");
    }
    set_stage(report, "capture", "completed", None);
    checkpoint(root, report)?;
    let budget = GenerationBudget::new(4096, Duration::from_secs(180))?;
    report["active_stage"] = json!("generate");
    let generation = async {
        let request = match protocol {
            SupportProtocol::LegacyV1 => support::generation_request(&prepared, model, budget)?,
            SupportProtocol::GroundedV2 => {
                support::grounded_generation_request(&prepared, model, budget)?
            }
        };
        report["stages"]["generate"]["packet_bytes"] = json!(packet_bytes(&request));
        let provider = stage_provider(root, "generate", providers, cancel).await?;
        let outcome = match protocol {
            SupportProtocol::LegacyV1 => {
                support::generate(provider.as_ref(), model, &prepared, budget, cancel).await
            }
            SupportProtocol::GroundedV2 => {
                support::generate_grounded(provider.as_ref(), model, &prepared, budget, cancel)
                    .await
            }
        };
        write_audit_exports(root, report, "generate", &outcome)?;
        let served = outcome
            .served_model
            .clone()
            .context("generator identity unavailable")?;
        let proposal = outcome
            .result
            .map_err(|e| anyhow::anyhow!("generation refused: {e:?}"))?;
        write_value(&root.join("generated-proposal.json"), &proposal)?;
        Ok::<_, anyhow::Error>((proposal, served))
    }
    .await;
    let generated = match generation {
        Ok(value) => {
            set_stage(report, "generate", "completed", None);
            Some(value)
        }
        Err(error) => {
            set_stage(report, "generate", "failed", Some(format!("{error:#}")));
            None
        }
    };
    checkpoint(root, report)?;
    report["active_stage"] = json!("evaluate_generated");
    let evaluated = if let Some((proposal, served)) = generated {
        let evaluation = async {
            let request =
                memory.prepare_support_with_model(&prepared, proposal, &served, &source.access)?;
            let body = match protocol {
                SupportProtocol::LegacyV1 => support::evaluation_request(&request, model, budget)?,
                SupportProtocol::GroundedV2 => {
                    support::grounded_evaluation_request(&request, model, budget)?
                }
            };
            report["stages"]["evaluate_generated"]["packet_bytes"] = json!(packet_bytes(&body));
            let provider = stage_provider(root, "evaluate_generated", providers, cancel).await?;
            let outcome = match protocol {
                SupportProtocol::LegacyV1 => {
                    support::evaluate(provider.as_ref(), model, request, budget, cancel).await
                }
                SupportProtocol::GroundedV2 => {
                    support::evaluate_grounded(provider.as_ref(), model, request, budget, cancel)
                        .await
                }
            };
            write_audit_exports(root, report, "evaluate_generated", &outcome)?;
            outcome
                .result
                .map_err(|e| anyhow::anyhow!("generated evaluation refused: {e:?}"))
        }
        .await;
        match evaluation {
            Ok(value) => {
                set_stage(report, "evaluate_generated", "completed", None);
                Some(value)
            }
            Err(error) => {
                set_stage(
                    report,
                    "evaluate_generated",
                    "failed",
                    Some(format!("{error:#}")),
                );
                None
            }
        }
    } else {
        set_stage(
            report,
            "evaluate_generated",
            "withheld",
            Some("generator unavailable; no substitute proposal".into()),
        );
        None
    };
    checkpoint(root, report)?;
    report["active_stage"] = json!("retain");
    let retained = if let Some(evaluated) = evaluated {
        match memory.retain_evaluated_interpretation(prepared.clone(), evaluated, &source.access) {
            Ok(receipt) => {
                let stored = durable_support(&memory, &receipt, &source.access, &current)?;
                let path = root.join("support-generated.json");
                write_value(&path, &stored)?;
                report["support_reports"]["generated"] = descriptor(root, &path)?;
                let accepted: Vec<_> = stored
                    .units
                    .iter()
                    .filter_map(|entry| match entry {
                        UnitDisposition::Accepted { unit, status } => {
                            let mut value = serde_json::to_value(unit).ok()?;
                            value["status"] = json!(status);
                            Some(value)
                        }
                        _ => None,
                    })
                    .collect();
                let path = root.join("admitted-generated.json");
                write_value(&path, &accepted)?;
                report["artifacts"]["admitted_generated"] = descriptor(root, &path)?;
                write_value(&root.join("generated-receipt.json"), &receipt)?;
                set_stage(report, "retain", "completed", None);
                Some(receipt)
            }
            Err(error) => {
                set_stage(report, "retain", "failed", Some(format!("{error:#}")));
                None
            }
        }
    } else {
        set_stage(
            report,
            "retain",
            "withheld",
            Some("no evaluated generated receipt".into()),
        );
        None
    };
    if report["artifacts"]["admitted_generated"].is_null() {
        let path = root.join("admitted-generated.json");
        write_value(&path, &json!([]))?;
        report["artifacts"]["admitted_generated"] = descriptor(root, &path)?;
    }
    checkpoint(root, report)?;
    // This independent probe copy contains exactly the original frozen sources,
    // never the generator output or generated interpretation.
    report["active_stage"] = json!("evaluate_probe");
    let probe_evaluation = async {
        let mut probe_memory =
            Memory::open(&probe_database)?.with_source_guard(Arc::new(source.authority.clone()));
        let probe_prepared = probe_memory.prepare_interpretation(
            &capture,
            &[],
            &current,
            &probe_memory.snapshot()?,
            &source.access,
        )?;
        let proposal = probe_proposal(case, &probe_prepared)?;
        write_value(&root.join("probe-proposal.json"), &proposal)?;
        let request = probe_memory.prepare_support_with_model(
            &probe_prepared,
            proposal,
            "host-authored-probe",
            &source.access,
        )?;
        let body = match protocol {
            SupportProtocol::LegacyV1 => support::evaluation_request(&request, model, budget)?,
            SupportProtocol::GroundedV2 => {
                support::grounded_evaluation_request(&request, model, budget)?
            }
        };
        report["stages"]["evaluate_probe"]["packet_bytes"] = json!(packet_bytes(&body));
        let provider = stage_provider(root, "evaluate_probe", providers, cancel).await?;
        let outcome = match protocol {
            SupportProtocol::LegacyV1 => {
                support::evaluate(provider.as_ref(), model, request, budget, cancel).await
            }
            SupportProtocol::GroundedV2 => {
                support::evaluate_grounded(provider.as_ref(), model, request, budget, cancel).await
            }
        };
        write_audit_exports(root, report, "evaluate_probe", &outcome)?;
        let evaluated = outcome
            .result
            .map_err(|e| anyhow::anyhow!("probe evaluation refused: {e:?}"))?;
        let raw_reply = evaluated.reply().clone();
        let receipt = probe_memory.retain_evaluated_interpretation(
            probe_prepared,
            evaluated,
            &source.access,
        )?;
        let stored = durable_support(&probe_memory, &receipt, &source.access, &current)?;
        report["probe"] = probe_results(&stored, &raw_reply);
        let path = root.join("support-probe.json");
        write_value(&path, &stored)?;
        report["support_reports"]["probe"] = descriptor(root, &path)?;
        write_value(&root.join("probe-receipt.json"), &receipt)?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    match probe_evaluation {
        Ok(()) => set_stage(report, "evaluate_probe", "completed", None),
        Err(error) => set_stage(
            report,
            "evaluate_probe",
            "failed",
            Some(format!("{error:#}")),
        ),
    }
    checkpoint(root, report)?;
    report["active_stage"] = json!("recall");
    let recall = read_memory(&source, &current, true).await?;
    let recall_bytes = serde_json::to_vec(&recall["memory"])?.len();
    write_value(&root.join("recall.json"), &recall)?;
    report["stages"]["recall"]["packet_bytes"] = json!(recall_bytes);
    let omitted = recall["memory"]["budget_omitted"] == true;
    report["stages"]["recall"]["withheld"] = json!(omitted);
    let mut positive_current = false;
    if let Some(receipt) = &retained {
        let selected = recall["memory"]["interpretation_roles"]
            .as_array()
            .is_some_and(|roles| {
                roles.iter().any(|r| {
                    r["reference"] == json!(receipt.reference) && r["role"] == "SelectedCurrent"
                })
            });
        report["stages"]["recall"]["selected_current"] = json!(selected);
        positive_current = selected;
        ensure!(
            selected || omitted,
            "ordinary GoalMemory omitted the current interpretation without a budget reason"
        );
    }
    if omitted {
        set_stage(
            report,
            "recall",
            "withheld",
            Some("complete contextual bundle exceeds atomic packet budget".into()),
        );
    } else {
        set_stage(report, "recall", "completed", None);
    }
    checkpoint(root, report)?;
    report["active_stage"] = json!("changed_goal");
    let mut followup = current.criterion().clone();
    followup.goal = "task:synthetic-followup".into();
    followup.goal_text = "Verify a prior artifact for a distinct synthetic follow-up.".into();
    followup.criterion_text =
        "Request fresh verification before relying on any historical artifact.".into();
    followup.criterion_revision = digest(followup.criterion_text.as_bytes());
    followup.confirmation_digest = digest(b"owner-confirmed-synthetic-followup");
    let changed = InterpretationContext::new(followup, new_context_id())?;
    let packet = read_memory(&source, &changed, true).await?;
    ensure!(
        !packet["memory"]["interpretation_roles"]
            .as_array()
            .is_some_and(|roles| roles.iter().any(|r| r["role"] == "SelectedCurrent")),
        "historical context promoted across changed goal"
    );
    write_value(&root.join("changed-goal.json"), &packet)?;
    report["stages"]["changed_goal"]["packet_bytes"] =
        json!(serde_json::to_vec(&packet["memory"])?.len());
    report["stages"]["changed_goal"]["withheld_previous_current"] = json!(true);
    if positive_current {
        set_stage(report, "changed_goal", "completed", None);
    } else {
        set_stage(report, "changed_goal", "withheld", Some(
            "prior current interpretation was unavailable; negative changed-goal read is not qualification proof".into()));
    }
    ensure!(
        descriptor(root, &source.input)?["sha256"] == report["initial_hashes"]["initial_input"]
            && descriptor(root, &source.transcript)?["sha256"]
                == report["initial_hashes"]["native_transcript"],
        "source changed after seal"
    );
    match &case.native.observed_output {
        Some(_) => ensure!(
            descriptor(root, &source.artifact)?["sha256"]
                == report["initial_hashes"]["observed_output"],
            "artifact changed after seal"
        ),
        None => ensure!(
            !source.artifact.exists(),
            "missing artifact appeared after seal"
        ),
    }
    ensure!(
        descriptor(root, &source_control)? == report["source_only_database"],
        "source-only control changed"
    );
    checkpoint(root, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn grounded_workflow_uses_constrained_wire_and_canonical_recall() {
        let temporary = temporary();
        let root = temporary.join("grounded");
        let factory = providers(false, false);
        let report = run_case_with_protocol(
            &root,
            &trial_case(),
            "authored-support",
            &factory,
            &CancellationToken::new(),
            SupportProtocol::GroundedV2,
        )
        .await
        .unwrap();
        assert_eq!(report["protocol"], GROUNDED_PROTOCOL);
        assert_eq!(report["status"], "completed", "{report:#}");
        assert_eq!(report["stages"]["recall"]["selected_current"], true);
        assert_eq!(report["probe"]["units"][0]["admitted"], true);
        assert_eq!(report["probe"]["units"][1]["admitted"], false);
        for stage in ["generate", "evaluate_generated", "evaluate_probe"] {
            let request: Value = serde_json::from_slice(
                &fs::read(root.join(format!("{stage}-request.json"))).unwrap(),
            )
            .unwrap();
            assert!(request["request"]["response_schema"].is_object());
            assert_eq!(
                request["wire_body"]["response_format"]["type"],
                "json_object"
            );
            assert_eq!(
                request["wire_body"]["response_format"]["schema"],
                request["request"]["response_schema"]
            );
        }
        fs::remove_dir_all(temporary).unwrap();
    }
    fn temporary() -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("mecha-support-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        root.canonicalize().unwrap()
    }
    fn native(requested: &str, observed: Option<&str>) -> NativeCase {
        NativeCase {
            input_text: "volume,page,approved\na3,14,yes\na3,15,no\n".into(),
            requested_output: requested.into(),
            observed_output: observed.map(str::to_string),
            historical_criterion: "Include exactly approved=yes pages.".into(),
            current_criterion: "Include exactly approved=yes pages.".into(),
            goal_text: "Preserve the requested archive page selection.".into(),
        }
    }
    #[tokio::test]
    async fn native_requested_write_and_later_artifact_are_independent_observations() {
        let temporary = temporary();
        let requested = "[{\"volume\":\"a3\",\"page\":\"14\"}]";
        let observed = "[{\"volume\":\"a3\",\"page\":\"15\"}]";
        let source = execute_task(
            &temporary.join("source"),
            &native(requested, Some(observed)),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(fs::read_to_string(&source.artifact).unwrap(), observed);
        let text = fs::read_to_string(&source.transcript).unwrap();
        let transcript = Session::parse(&source.transcript, &text).unwrap();
        assert!(transcript.convo.messages.iter().flat_map(|m| &m.content).any(|block| matches!(block,
            Block::ToolUse { name, input, .. } if name == "fs_write" && input["content"] == requested)));
        assert!(!transcript.convo.messages.iter().flat_map(|m| &m.content).any(|block| matches!(block,
            Block::ToolUse { name, input, .. } if name == "fs_write" && input["content"] == observed)));
        let mut memory = source.memory().unwrap();
        let receipt = source.capture(&mut memory).unwrap();
        let context = InterpretationContext::new(
            criterion(&native(requested, Some(observed)), true),
            new_context_id(),
        )
        .unwrap();
        let prepared = memory
            .prepare_interpretation(
                &receipt,
                &[],
                &context,
                &memory.snapshot().unwrap(),
                &source.access,
            )
            .unwrap();
        assert!(prepared
            .sources()
            .iter()
            .any(|s| s.kind == SourceKind::ActionInvocation
                && serde_json::from_str::<Value>(&s.text).unwrap()["input"]["content"]
                    == requested));
        assert!(prepared
            .sources()
            .iter()
            .any(|s| s.kind == SourceKind::ArtifactObservation
                && serde_json::from_str::<Value>(&s.text).unwrap()["artifact_text"] == observed));
        assert!(source.input.is_file());
        fs::remove_dir_all(temporary).unwrap();
    }

    fn trial_case() -> TrialCase {
        TrialCase {
            version: 1,
            case_id: "authored-archive".into(),
            case_sha256: "a".repeat(64),
            native: native(
                "[{\"volume\":\"a3\",\"page\":\"14\"}]",
                Some("[{\"volume\":\"a3\",\"page\":\"15\"}]"),
            ),
            probe: ProbeCase {
                units: vec![
                    ProbeUnit {
                        id: "q1".into(),
                        kind: UnitKind::RecordedEvent,
                        text: "The observed artifact records page15.".into(),
                        citation: CitationSelector {
                            passage: "artifact".into(),
                            quote: None,
                        },
                    },
                    ProbeUnit {
                        id: "q2".into(),
                        kind: UnitKind::RecordedEvent,
                        text: "The observed artifact records page14.".into(),
                        citation: CitationSelector {
                            passage: "artifact".into(),
                            quote: None,
                        },
                    },
                    ProbeUnit {
                        id: "q3".into(),
                        kind: UnitKind::RecordedEvent,
                        text: "An independent reviewer approved this artifact.".into(),
                        citation: CitationSelector {
                            passage: "input".into(),
                            quote: None,
                        },
                    },
                ],
                judgments: vec![
                    ProbeJudgment {
                        bearing: InterpretationBearing::Bad,
                        because: vec!["q1".into()],
                    },
                    ProbeJudgment {
                        bearing: InterpretationBearing::Good,
                        because: vec!["q1".into()],
                    },
                ],
            },
        }
    }
    struct AuthoredProviders {
        fail_generation: bool,
        cancel_generation: bool,
        seen: Arc<Mutex<Vec<String>>>,
    }
    struct AuthoredSupport {
        stage: String,
        fail: bool,
        cancel: Option<CancellationToken>,
    }
    #[async_trait::async_trait]
    impl StageProviders for AuthoredProviders {
        async fn create(
            &self,
            stage: &str,
            cancel: &CancellationToken,
        ) -> Result<Box<dyn Provider>> {
            self.seen.lock().unwrap().push(stage.into());
            Ok(Box::new(AuthoredSupport {
                stage: stage.into(),
                fail: self.fail_generation && stage == "generate",
                cancel: (self.cancel_generation && stage == "generate").then(|| cancel.clone()),
            }))
        }
    }
    #[async_trait::async_trait]
    impl Provider for AuthoredSupport {
        fn id(&self) -> &str {
            "authored-support"
        }
        fn default_model(&self) -> &str {
            "authored-support"
        }
        fn structured_output(&self) -> bool {
            true
        }
        fn local(&self) -> bool {
            true
        }
        fn wire_body(&self, request: &CompletionRequest, stream: bool) -> Option<Value> {
            // Reuse the production serializer without calling its transport.
            crate::provider::openai::OpenAiCompatible::from_local_config(
                &crate::config::ProviderConfig {
                    kind: "local".into(),
                    base_url: Some("http://127.0.0.1:1".into()),
                    model: Some("authored-support".into()),
                    temperature: Some(0.6),
                    seed: Some(70510),
                    max_retries: Some(0),
                    structured_output: if request.response_schema.is_some() {
                        crate::config::StructuredOutput::LlamaJson
                    } else {
                        crate::config::StructuredOutput::Disabled
                    },
                    ..Default::default()
                },
            )
            .unwrap()
            .without_autoload()
            .wire_body(request, stream)
        }
        async fn complete(
            &self,
            request: &CompletionRequest,
            _: Option<&StreamSink>,
        ) -> Result<CompletionResponse> {
            if let Some(cancel) = &self.cancel {
                cancel.cancel();
                std::future::pending::<()>().await;
            }
            ensure!(!self.fail, "authored generator transport failure");
            let text = request.messages[0].text();
            let packet: Value = serde_json::from_str(
                text.strip_prefix("<support-input>\n")
                    .unwrap()
                    .strip_suffix("\n</support-input>")
                    .unwrap(),
            )?;
            let reply = if request.response_schema.is_some() {
                authored_grounded_reply(&packet, &self.stage)?
            } else if self.stage == "generate" {
                let source = packet["sources"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|s| s["kind"] == "ArtifactObservation")
                    .unwrap();
                json!({"units":[{"id":"g1","kind":"RecordedEvent","text":"The observed artifact records page15.",
                    "citations":[{"source_id":source["id"],"quote":source["text"]}]}],
                    "judgments":[{"goal":packet["criterion"]["goal"],"bearing":"Bad","because":["g1"]}]})
            } else {
                let mut checks = vec![];
                for unit in packet["proposal"]["units"].as_array().unwrap() {
                    let id = unit["id"].as_str().unwrap();
                    let (status, premise, questions) = match id {
                        "q2" => ("Contradicted", true, vec![]),
                        "q3" => (
                            "Unknown",
                            false,
                            vec!["Was an independent approval recorded?"],
                        ),
                        _ => ("Supported", false, vec![]),
                    };
                    checks.push(json!({"target":format!("unit:{id}"),"status":status,"unsupported_factual_premise":premise,
                        "citations":unit["citations"],"reason":"Authored mechanical check, no model quality claim.","questions":questions}));
                }
                let citation = &packet["proposal"]["units"][0]["citations"];
                for (index, judgment) in packet["proposal"]["judgments"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                {
                    checks.push(json!({"target":format!("judgment:{index}"),"status":if judgment["bearing"] == "Bad" {"Supported"} else {"Contradicted"},
                        "unsupported_factual_premise":false,"citations":citation,"reason":"Authored current-criterion check.","questions":[]}));
                }
                json!({"checks":checks})
            };
            Ok(CompletionResponse {
                message: Message::assistant(vec![Block::text(reply.to_string())]),
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 11,
                    output_tokens: 7,
                    ..Usage::default()
                },
                refusal: None,
                model: "authored-support".into(),
                malformed_tool_args: 0,
            })
        }
    }
    fn authored_grounded_reply(packet: &Value, stage: &str) -> Result<Value> {
        let rows = packet["catalog"]["citations"]
            .as_array()
            .context("missing catalog")?;
        let artifact = rows
            .iter()
            .find(|r| r["kind"] == "ArtifactObservation")
            .context("missing artifact citation")?;
        let evidence = json!([artifact["handle"]]);
        if stage == "generate" {
            return Ok(json!({"request_key":packet["request_key"],
                "units":[{"id":"g1","kind":"RecordedEvent","text":"The observed artifact records page15.","evidence":evidence}],
                "judgments":[{"bearing":"Bad","because":["g1"]}]}));
        }
        let units: Vec<_> = packet["proposal"]["units"].as_array().context("missing units")?.iter().map(|unit| {
            let id = unit["id"].as_str().unwrap();
            let (status, premise, questions) = match id {
                "q2" => ("Contradicted", "Contradicted", vec![]),
                "q3" => ("Unknown", "Unknown", vec!["Was independent approval recorded?"]),
                _ => ("Supported", "Supported", vec![]),
            };
            json!({"target":format!("unit:{id}"),"evidence":evidence,"reason":"Authored mechanical check, not model quality.",
                "questions":questions,"factual_premises":premise,"status":status})
        }).collect();
        let judgments: Vec<_> = packet["proposal"]["judgments"].as_array().context("missing judgments")?.iter().enumerate().map(|(i, _)| {
            json!({"target":format!("judgment:{i}"),"evidence":evidence,"reason":"Authored current-criterion check.",
                "questions":[],"premises_sufficient":true})
        }).collect();
        Ok(
            json!({"request_key":packet["request_key"],"criterion":{"evidence":evidence,"reason":"Observed page15 differs from required page14.",
            "questions":[],"state":"NotMet"},"units":units,"judgments":judgments}),
        )
    }
    fn providers(fail_generation: bool, cancel_generation: bool) -> AuthoredProviders {
        AuthoredProviders {
            fail_generation,
            cancel_generation,
            seen: Arc::new(Mutex::new(vec![])),
        }
    }
    #[tokio::test]
    async fn authored_workflow_retains_reads_withholds_changed_goal_and_preserves_controls() {
        let temporary = temporary();
        let root = temporary.join("case");
        let factory = providers(false, false);
        let report = run_case(
            &root,
            &trial_case(),
            "authored-support",
            &factory,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(report["status"], "completed", "{report:#}");
        for stage in STAGES {
            assert_eq!(
                report["stages"][stage]["status"], "completed",
                "{stage}: {report:#}"
            );
        }
        assert_eq!(report["stages"]["recall"]["selected_current"], true);
        assert_eq!(
            report["stages"]["changed_goal"]["withheld_previous_current"],
            true
        );
        assert_eq!(report["needs_request_identical"], true);
        assert_eq!(report["probe"]["units"][0]["admitted"], true);
        assert_eq!(report["probe"]["units"][1]["admitted"], false);
        assert_eq!(report["probe"]["units"][2]["status"], "Unknown");
        assert_eq!(
            *factory.seen.lock().unwrap(),
            vec!["generate", "evaluate_generated", "evaluate_probe"]
        );
        for stage in ["generate", "evaluate_generated", "evaluate_probe"] {
            let audit = fs::read_to_string(root.join(format!("{stage}-provider.jsonl"))).unwrap();
            let events: Vec<Value> = audit
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            let requests: Vec<_> = events
                .iter()
                .filter(|r| r["event"] == "request_started")
                .collect();
            let replies: Vec<_> = events.iter().filter(|r| r["event"] == "response").collect();
            assert_eq!(requests.len(), 1);
            assert_eq!(replies.len(), 1);
            let exported: Value = serde_json::from_slice(
                &fs::read(root.join(format!("{stage}-request.json"))).unwrap(),
            )
            .unwrap();
            assert_eq!(&exported, requests[0]);
        }
        let control = Memory::open(&root.join("source-only.memory-prototype.sqlite")).unwrap();
        let primary = Memory::open(&root.join("native/support.memory-prototype.sqlite")).unwrap();
        let probe = Memory::open(&root.join("probe.memory-prototype.sqlite")).unwrap();
        assert!(control.snapshot().unwrap().sequence < primary.snapshot().unwrap().sequence);
        assert_eq!(
            primary.snapshot().unwrap().sequence,
            probe.snapshot().unwrap().sequence
        );
        let seal: Value =
            serde_json::from_slice(&fs::read(root.join("source-seal.json")).unwrap()).unwrap();
        assert_eq!(seal, report["initial_hashes"]);
        fs::remove_dir_all(temporary).unwrap();
    }
    #[tokio::test]
    async fn generator_failure_still_evaluates_independent_probe_and_keeps_all_stage_slots() {
        let temporary = temporary();
        let root = temporary.join("failed");
        let factory = providers(true, false);
        let report = run_case(
            &root,
            &trial_case(),
            "authored-support",
            &factory,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(report["status"], "failed");
        assert_eq!(report["stages"]["generate"]["status"], "failed");
        assert_eq!(report["stages"]["evaluate_generated"]["status"], "withheld");
        assert_eq!(
            report["stages"]["evaluate_probe"]["status"], "completed",
            "{report:#}"
        );
        assert_eq!(report["stages"].as_object().unwrap().len(), 8);
        assert_eq!(
            *factory.seen.lock().unwrap(),
            vec!["generate", "evaluate_probe"]
        );
        assert_eq!(report["stages"]["changed_goal"]["status"], "withheld");
        fs::remove_dir_all(temporary).unwrap();
    }
    #[tokio::test]
    async fn runtime_cancellation_keeps_request_audit_and_does_not_issue_later_calls() {
        let temporary = temporary();
        let root = temporary.join("cancelled");
        let factory = providers(false, true);
        let cancel = CancellationToken::new();
        let report = tokio::time::timeout(
            Duration::from_secs(3),
            run_case(&root, &trial_case(), "authored-support", &factory, &cancel),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(report["status"], "failed");
        assert_eq!(report["cancelled"], true);
        assert_eq!(report["stages"].as_object().unwrap().len(), 8);
        assert_eq!(*factory.seen.lock().unwrap(), vec!["generate"]);
        assert!(fs::read_to_string(root.join("generate-provider.jsonl"))
            .unwrap()
            .contains("request_started"));
        fs::remove_dir_all(temporary).unwrap();
    }
    #[tokio::test]
    async fn source_revocation_withholds_interpretation_and_source_descendants() {
        for protocol in [SupportProtocol::LegacyV1, SupportProtocol::GroundedV2] {
            let temporary = temporary();
            let root = temporary.join("revoked");
            let report = run_case_with_protocol(
                &root,
                &trial_case(),
                "authored-support",
                &providers(false, false),
                &CancellationToken::new(),
                protocol,
            )
            .await
            .unwrap();
            assert_eq!(report["status"], "completed", "{report:#}");
            let authority_path = root.join("native/authority");
            let binding_path = fs::read_dir(&authority_path)
                .unwrap()
                .map(|e| e.unwrap().path())
                .find(|p| p.extension().is_some_and(|e| e == "binding"))
                .unwrap();
            let binding: crate::memory::host_capture::ExternalBinding =
                serde_json::from_slice(&fs::read(&binding_path).unwrap()).unwrap();
            let authority = LocalAuthority::open(&authority_path).unwrap();
            authority.withdraw(&binding.locator).unwrap();
            let recall: Value =
                serde_json::from_slice(&fs::read(root.join("recall.json")).unwrap()).unwrap();
            let record = recall["memory"]["records"]
                .as_array()
                .unwrap()
                .iter()
                .find(|hit| hit["record"]["kind"] == "interpretation")
                .unwrap();
            let interpretation: crate::memory::interpretation::Interpretation =
                serde_json::from_value(record["record"]["data"].clone()).unwrap();
            let context = InterpretationContext::new(
                interpretation.criterion,
                interpretation.host_context_id,
            )
            .unwrap();
            let source = SourceFixture {
                input: root.join("native/workspace/input.txt"),
                artifact: root.join("native/workspace/output.json"),
                transcript: root.join(
                    report["artifacts"]["native_transcript"]["path"]
                        .as_str()
                        .unwrap(),
                ),
                database: root.join("native/support.memory-prototype.sqlite"),
                authority,
                access: Access {
                    domain: DOMAIN.into(),
                    scopes: ["synthetic:support".into()].into(),
                },
                confirmation: String::new(),
            };
            let revoked = read_memory(&source, &context, true).await.unwrap();
            assert!(revoked["memory"]["records"].as_array().unwrap().is_empty());
            fs::remove_dir_all(temporary).unwrap();
        }
    }

    #[tokio::test]
    async fn native_goal_memory_consultation_rich_recapture_preserves_changed_goal_lineage() {
        let temporary = temporary();
        let root = temporary.join("case");
        let report = run_case(
            &root,
            &trial_case(),
            "authored-support",
            &providers(false, false),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(report["status"], "completed", "{report:#}");
        let recalled: Value =
            serde_json::from_slice(&fs::read(root.join("recall.json")).unwrap()).unwrap();
        let original_hit = recalled["memory"]["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|h| h["record"]["kind"] == "interpretation")
            .unwrap();
        let original: crate::memory::interpretation::Interpretation =
            serde_json::from_value(original_hit["record"]["data"].clone()).unwrap();
        let original_ref: crate::memory::RevisionRef =
            serde_json::from_value(original_hit["reference"].clone()).unwrap();
        let current =
            InterpretationContext::new(original.criterion.clone(), original.host_context_id)
                .unwrap();
        let access = Access {
            domain: DOMAIN.into(),
            scopes: ["synthetic:support".into()].into(),
        };
        let database = root.join("native/support.memory-prototype.sqlite");
        let authority_path = root.join("native/authority");
        let authority = LocalAuthority::open(&authority_path).unwrap();
        let original_binding_path = fs::read_dir(&authority_path)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "binding"))
            .unwrap();
        let original_binding: crate::memory::host_capture::ExternalBinding =
            serde_json::from_slice(&fs::read(original_binding_path).unwrap()).unwrap();
        let next = temporary.join("followup");
        fs::create_dir(&next).unwrap();
        let workspace = next.join("workspace");
        fs::create_dir(&workspace).unwrap();
        let artifact = workspace.join("output.json");
        let session = Session::create(
            &next.join("sessions"),
            SessionMeta {
                id: uuid::Uuid::new_v4().to_string(),
                created_at: chrono::Utc::now(),
                provider: "authored".into(),
                model: "authored".into(),
                workspace: workspace.clone(),
                title: None,
                kind: Some(SessionKind::Test),
            },
        )
        .unwrap();
        let mut conversation = Conversation::user("Read the historical output through goal_context, then write the pending verification record to output.json.");
        conversation.goal_anchor = Some(current.criterion().goal.parse().unwrap());
        session
            .append(&Record::GoalAnchor {
                goal: conversation.goal_anchor.clone(),
            })
            .unwrap();
        session.append_messages(&conversation.messages).unwrap();
        let confirmation = authority
            .freeze(FreezeRequest {
                transcript: session.path.clone(),
                artifact: artifact.clone(),
                criterion: current.criterion().clone(),
                expected_rows: vec![BTreeMap::from([("verification".into(), "pending".into())])],
            })
            .unwrap();
        let mut registry = Registry::new();
        registry.insert(Arc::new(FsWrite));
        registry.insert(Arc::new(
            GoalMemory::open_with_source_guard(
                &database,
                access.clone(),
                ACCOUNT.into(),
                Situation {
                    event: "Verify the prior output.".into(),
                    goal: current.criterion().goal.clone(),
                    needs: None,
                },
                Arc::new(authority.clone()),
            )
            .unwrap()
            .with_interpretations(current.read_policy())
            .unwrap(),
        ));
        let script = Script(Mutex::new(VecDeque::from([
            tool_call(
                "consult",
                "goal_context",
                json!({"memory":{"query":"output"}}),
            ),
            tool_call(
                "write-review",
                "fs_write",
                json!({"path":"output.json","content":"[{\"verification\":\"pending\"}]"}),
            ),
            Message::assistant(vec![Block::text(
                "The pending verification record was written.",
            )]),
        ])));
        let approver = Arc::new(ModeApprover {
            mode: PermissionMode::Allow,
        });
        let ctx = ToolCtx {
            workspace,
            conversation: Some(session.meta.id.clone()),
            spill_dir: Some(next.join("spill")),
            ..Default::default()
        };
        let agent = Agent::new(
            Box::new(script),
            registry,
            approver.clone(),
            ctx.clone(),
            AgentConfig {
                max_turns: 4,
                max_tokens: 4096,
                max_output_tokens: Some(16384),
                force_final_answer: false,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        let before = conversation.messages.clone();
        let outcome = agent
            .run_in(&RunContext::new(ctx, approver), &mut conversation, None)
            .await
            .unwrap();
        session.record_run(&before, &conversation).unwrap();
        session.append(&Record::Taint(conversation.taint)).unwrap();
        session.record_outcome(&outcome).unwrap();
        assert!(!outcome.stop_cause.is_early());
        let consulted_result = conversation
            .messages
            .iter()
            .flat_map(|m| &m.content)
            .find_map(|b| match b {
                Block::ToolResult {
                    tool_use_id,
                    content,
                    is_error: false,
                } if tool_use_id == "consult" => Some(content),
                _ => None,
            })
            .unwrap();
        assert!(
            consulted_result.contains(&original_ref.revision),
            "native GoalMemory result must positively include interpreted ancestor"
        );
        let mut memory = Memory::open(&database)
            .unwrap()
            .with_source_guard(Arc::new(authority.clone()));
        let capture = authority
            .capture_with_evidence(
                &mut memory,
                CaptureRequest {
                    confirmation,
                    domain: DOMAIN.into(),
                    account: ACCOUNT.into(),
                    scopes: access.scopes.clone(),
                    consulted: vec![],
                    records: SessionRecords::default(),
                },
                &access,
            )
            .unwrap_or_else(|e| {
                panic!(
                    "native rich recapture refused (GoalMemory result {} bytes): {e:#}",
                    consulted_result.len()
                )
            });
        let mut changed_criterion = current.criterion().clone();
        changed_criterion.goal = "task:changed-verification".into();
        changed_criterion.criterion_text =
            "For the new follow-up, verify the pending review before relying on it.".into();
        changed_criterion.criterion_revision = digest(changed_criterion.criterion_text.as_bytes());
        changed_criterion.confirmation_digest = digest(b"owner-confirmed-changed-verification");
        let changed = InterpretationContext::new(changed_criterion, new_context_id()).unwrap();
        let prepared = memory
            .prepare_interpretation(
                &capture,
                &[],
                &changed,
                &memory.snapshot().unwrap(),
                &access,
            )
            .unwrap();
        assert!(prepared
            .sources()
            .iter()
            .any(|s| s.source == original_ref && s.kind == SourceKind::PriorInterpretation));
        assert!(prepared
            .sources()
            .iter()
            .any(|s| s.kind == SourceKind::InterpretationInfluenced
                && s.text.contains("write-review")));
        assert!(prepared
            .sources()
            .iter()
            .any(|s| s.kind == SourceKind::InterpretationInfluenced
                && s.text.contains("artifact_text")));
        authority.withdraw(&original_binding.locator).unwrap();
        drop(memory);
        let reopened = Memory::open(&database)
            .unwrap()
            .with_source_guard(Arc::new(authority));
        // Repeat the positive contextual preparation above: default read would
        // already withhold interpreted descendants even before revocation.
        assert!(
            reopened
                .prepare_interpretation(
                    &capture,
                    &[],
                    &changed,
                    &reopened.snapshot().unwrap(),
                    &access
                )
                .is_err(),
            "previously authorized rich descendant survived ancestor withdrawal"
        );
        fs::remove_dir_all(temporary).unwrap();
    }

    #[test]
    fn rejected_model_response_usage_is_counted_without_inventing_absent_cost() {
        for mode in 0..3 {
            let root = temporary();
            let usage = match mode {
                0 => Some(Usage {
                    input_tokens: 11,
                    output_tokens: 7,
                    cache_creation_input_tokens: 3,
                    cache_read_input_tokens: 5,
                }),
                1 => Some(Usage::default()),
                _ => None,
            };
            let rejected = usage.map(|usage| json!({"model":"other-model","native_usage":usage}));
            let event = json!({"event":"error","call":1,"error":"served identity mismatch","rejected_response":rejected});
            write_new(
                &root.join("generate-provider.jsonl"),
                event.to_string().as_bytes(),
            )
            .unwrap();
            let mut report =
                json!({"stages":{"generate":{"status":"failed","error":"identity mismatch"}}});
            let outcome: PassOutcome<SupportProposal> = PassOutcome {
                result: Err(support::SupportFailure::Provider(
                    "identity mismatch".into(),
                )),
                usage: None,
                elapsed: Duration::ZERO,
                raw_reply: None,
                reply_bytes: 0,
                served_model: None,
            };
            write_audit_exports(&root, &mut report, "generate", &outcome).unwrap();
            if mode == 0 {
                assert_eq!(report["stages"]["generate"]["input_tokens"], 19);
                assert_eq!(report["stages"]["generate"]["output_tokens"], 7);
                assert_eq!(
                    report["stages"]["generate"]["usage_source"],
                    "rejected_response"
                );
            } else {
                assert!(report["stages"]["generate"]["input_tokens"].is_null());
                assert!(report["stages"]["generate"]["output_tokens"].is_null());
            }
            assert_eq!(report["stages"]["generate"]["status"], "failed");
            assert!(report["stages"]["generate"]["reply"].is_null());
            fs::remove_dir_all(root).unwrap();
        }
    }
}
