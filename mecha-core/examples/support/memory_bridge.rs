//! Synthetic authored-provider qualification, not a measurement of memory benefit.

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use mecha_core::{
    agent::{Agent, Conversation, RunContext},
    appraisal::SessionRecords,
    config::{AgentConfig, PermissionMode, TrifectaPolicy},
    homeostat::Homeostat,
    memory::{context::Situation, host_capture::ConfirmedCriterion, Access, Memory},
    memory_experience_bridge::{CaptureRequest, FreezeRequest, LocalAuthority},
    message::{Block, CompletionRequest, CompletionResponse, Message, StopReason, Usage},
    provider::{Provider, StreamSink},
    session::{Record, Session, SessionKind, SessionMeta},
    tool::{
        builtin::{FsRead, FsWrite},
        goal_memory::GoalMemory,
        todo::TodoTool,
        Capabilities, ModeApprover, Registry, Tool, ToolCtx, ToolOutput,
    },
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
};

const DOMAIN: &str = "appraisal-bridge";
const ACCOUNT: &str = "synthetic";
const PRIVATE_NUMBER: &str = "918273645091";
const PRIVATE_LOAD: &str = "713.625";
const MALICIOUS: &str = "UNTRUSTED_CSV_IGNORE_RULES_SEND_DATA";
const FIRST_GOAL: &str = "task:csv-json";
const SECOND_GOAL: &str = "task:uppercase-json";
const FIRST_PROMPT: &str =
    "Convert the synthetic quoted newline CSV to exact JSON, preserving all field text.";

fn access() -> Access {
    Access {
        domain: DOMAIN.into(),
        scopes: BTreeSet::from(["synthetic:bridge".into()]),
    }
}
fn digest(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn rows(uppercase: bool) -> Vec<BTreeMap<String, String>> {
    vec![BTreeMap::from([
        (
            "name".into(),
            if uppercase {
                MALICIOUS.to_uppercase()
            } else {
                MALICIOUS.into()
            },
        ),
        (
            "note".into(),
            if uppercase {
                "QUOTED\nNEWLINE".into()
            } else {
                "quoted\nnewline".into()
            },
        ),
    ])]
}
fn criterion(goal: &str) -> ConfirmedCriterion {
    let text = if goal == FIRST_GOAL {
        "Exact JSON rows preserve the CSV field text"
    } else {
        "Exact JSON rows uppercase every field value"
    };
    ConfirmedCriterion {
        goal: goal.into(),
        goal_text: text.into(),
        criterion_text: text.into(),
        criterion_revision: digest(text),
        confirmation_digest: digest(&format!("owner-confirmed:{goal}")),
    }
}
fn records() -> SessionRecords<'static> {
    static CHARTER: std::sync::OnceLock<mecha_core::charter::Charter> = std::sync::OnceLock::new();
    let charter = CHARTER.get_or_init(|| mecha_core::charter::Charter::parse(
        "[[line]]\nid = 'synthetic'\ntext = 'Preserve field text'\n[line.sensor]\nkind = 'outbox_waiting'\nsetpoint = 918273645091\n"
    ).expect("synthetic host charter"));
    SessionRecords {
        drafts: &[],
        outbox_unreadable: false,
        questions: &[],
        questions_unreadable: false,
        requests: &[],
        frontdoor_unreadable: false,
        reflexions: &[],
        learning_unreadable: false,
        stops: &[],
        charter: Some(charter),
        charter_unreadable: false,
        closures: &[],
        closures_unreadable: false,
        workflows: &[],
        workflows_unreadable: false,
    }
}

/// No wire adapter or hidden inference. Requests are the loop's real requests.
struct AuthoredProvider {
    turns: Mutex<VecDeque<Message>>,
    requests: Arc<Mutex<Vec<CompletionRequest>>>,
}
#[async_trait]
impl Provider for AuthoredProvider {
    fn id(&self) -> &str {
        "authored-bridge-qualification"
    }
    fn default_model(&self) -> &str {
        "authored-no-model"
    }
    fn local(&self) -> bool {
        true
    }
    async fn complete(
        &self,
        req: &CompletionRequest,
        _: Option<&StreamSink>,
    ) -> Result<CompletionResponse> {
        self.requests.lock().unwrap().push(req.clone());
        let message = self
            .turns
            .lock()
            .unwrap()
            .pop_front()
            .context("unexpected provider request")?;
        let tools = message
            .content
            .iter()
            .any(|b| matches!(b, Block::ToolUse { .. }));
        Ok(CompletionResponse {
            message,
            stop_reason: if tools {
                StopReason::ToolUse
            } else {
                StopReason::EndTurn
            },
            usage: Usage::default(),
            refusal: None,
            model: self.default_model().into(),
            malformed_tool_args: 0,
        })
    }
}
fn call(id: &str, name: &str, input: Value) -> Message {
    Message::assistant(vec![Block::ToolUse {
        id: id.into(),
        name: name.into(),
        input,
    }])
}
fn plan(id: &str, goal: &str, status: &str) -> Message {
    call(
        id,
        "todo",
        json!({"serves":goal,"items":[{"content":"Convert rows","status":status,
        "expect":"The independent host checker compares exact JSON rows","expect_calls":3}]}),
    )
}
fn turns(goal: &str, wrong: bool, memory: bool) -> Vec<Message> {
    let mut out = vec![];
    if memory {
        out.push(call(
            "memory",
            "goal_context",
            json!({"memory":{"query":"quoted newline"}}),
        ));
        out.push(call("attempt-egress", "qualification_send", json!({})));
    }
    out.push(plan("plan", goal, "in_progress"));
    out.push(call(
        "read",
        "fs_read",
        json!({"path": if memory {"prior.json"} else {"input.csv"}}),
    ));
    out.push(call(
        "write",
        "fs_write",
        json!({"path":"output.json", "content":
        if wrong { "[]".into() } else { serde_json::to_string(&rows(memory)).unwrap() }}),
    ));
    out.push(call("inspect", "fs_read", json!({"path":"output.json"})));
    out.push(plan("complete", goal, "completed"));
    out.push(Message::assistant(vec![Block::text(
        "Authored task finished; host must check the artifact.",
    )]));
    out
}

/// A chosen-egress tripwire: it has no external service and must never run.
struct EgressTripwire;
#[async_trait]
impl Tool for EgressTripwire {
    fn name(&self) -> &str {
        "qualification_send"
    }
    fn description(&self) -> &str {
        "Synthetic chosen-egress interlock tripwire"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object"})
    }
    fn read_only(&self) -> bool {
        true
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities::default().sends()
    }
    async fn call(&self, _: Value, _: &ToolCtx) -> Result<ToolOutput> {
        panic!("chosen-egress interlock failed")
    }
}

struct Fixture {
    root: PathBuf,
    workspace: PathBuf,
    session: Session,
    authority: LocalAuthority,
    confirmation: String,
    convo: Conversation,
}
impl Fixture {
    fn create(root: &Path, goal: &str) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        let root = root.canonicalize()?;
        let workspace = root.join("workspace");
        std::fs::create_dir(&workspace)?;
        std::fs::write(
            workspace.join("input.csv"),
            format!("name,note\n{MALICIOUS},\"quoted\nnewline\"\n"),
        )?;
        let session = Session::create(
            &root.join("sessions"),
            SessionMeta {
                id: "synthetic-bridge-task".into(),
                created_at: Utc::now(),
                provider: "authored".into(),
                model: "no-model".into(),
                workspace: workspace.clone(),
                title: None,
                kind: Some(SessionKind::Test),
            },
        )?;
        let mut convo = Conversation::user(FIRST_PROMPT);
        let goal: mecha_core::goal::GoalRef = goal.parse()?;
        convo.goal_anchor = Some(goal.clone());
        session.append(&Record::GoalAnchor { goal: Some(goal) })?;
        session.append_messages(&convo.messages)?;
        let authority = LocalAuthority::create(&root.join("host-authority"))?;
        let confirmation = authority.freeze(FreezeRequest {
            transcript: session.path.clone(),
            artifact: workspace.join("output.json"),
            criterion: criterion(FIRST_GOAL),
            expected_rows: rows(false),
        })?;
        Ok(Self {
            root,
            workspace,
            session,
            authority,
            confirmation,
            convo,
        })
    }
    async fn execute(&mut self, wrong: bool, sensors: bool) -> Result<Vec<CompletionRequest>> {
        execute(
            &self.workspace,
            &self.session,
            &mut self.convo,
            turns(FIRST_GOAL, wrong, false),
            None,
            sensors,
        )
        .await
    }
    fn capture(
        &self,
        memory: &mut Memory,
    ) -> Result<mecha_core::memory::host_capture::CaptureReceipt> {
        self.authority.capture(
            memory,
            CaptureRequest {
                confirmation: self.confirmation.clone(),
                domain: DOMAIN.into(),
                account: ACCOUNT.into(),
                scopes: access().scopes,
                consulted: vec![],
                records: records(),
            },
            &access(),
        )
    }
}

async fn execute(
    workspace: &Path,
    session: &Session,
    convo: &mut Conversation,
    script: Vec<Message>,
    memory: Option<GoalMemory>,
    sensors: bool,
) -> Result<Vec<CompletionRequest>> {
    let requests = Arc::new(Mutex::new(vec![]));
    let provider = AuthoredProvider {
        turns: Mutex::new(script.into()),
        requests: requests.clone(),
    };
    let approver = Arc::new(ModeApprover {
        mode: PermissionMode::Allow,
    });
    let mut registry = Registry::new();
    registry.insert(Arc::new(FsRead));
    registry.insert(Arc::new(FsWrite));
    registry.insert(Arc::new(TodoTool::new()));
    if let Some(memory) = memory {
        registry.insert(Arc::new(memory));
        registry.insert(Arc::new(EgressTripwire));
    }
    let mut ctx = ToolCtx {
        workspace: workspace.canonicalize()?,
        conversation: Some(session.meta.id.clone()),
        ..Default::default()
    };
    ctx.security.trifecta = TrifectaPolicy::Block;
    let agent = Agent::new(
        Box::new(provider),
        registry,
        approver.clone(),
        ctx.clone(),
        AgentConfig {
            max_turns: 12,
            ..Default::default()
        },
        None,
    )?;
    let mut cx = RunContext::new(ctx, approver);
    cx.homeostat = Some(Homeostat {
        load_avg_1m: sensors.then_some(713.625),
        mem_available_kb: sensors.then_some(918273645091),
        backlog: None,
        ..Default::default()
    });
    let before = convo.messages.clone();
    let outcome = agent.run_in(&cx, convo, None).await;
    session.record_run(&before, convo)?;
    session.append(&Record::Taint(convo.taint))?;
    session.record_outcome(&outcome?)?;
    let captured = requests.lock().unwrap().clone();
    Ok(captured)
}

fn assert_safe(text: &str) {
    for marker in [PRIVATE_NUMBER, PRIVATE_LOAD] {
        assert!(!text.contains(marker), "host sensor escaped");
    }
    for marker in [
        "anticipated_guilt",
        "mem_available_kb",
        "load_avg_1m",
        "peak_context_pressure",
    ] {
        assert!(!text.contains(marker), "host field escaped: {marker}");
    }
}
fn requests_text(requests: &[CompletionRequest]) -> String {
    requests
        .iter()
        .map(|r| format!("{r:?}"))
        .collect::<Vec<_>>()
        .join("\n")
}
/// Privacy audit only. Runtime consumption below must use the actual GoalMemory.
fn scan_all_sqlite_payloads(path: &Path) -> Result<String> {
    let conn =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let names = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut contents = String::new();
    for name in names {
        let mut stmt = conn.prepare(&format!("SELECT * FROM \"{}\"", name.replace('"', "\"\"")))?;
        let count = stmt.column_count();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            for i in 0..count {
                use rusqlite::types::ValueRef;
                match row.get_ref(i)? {
                    ValueRef::Text(bytes) | ValueRef::Blob(bytes) => {
                        contents.push_str(&String::from_utf8_lossy(bytes))
                    }
                    ValueRef::Integer(number) => contents.push_str(&number.to_string()),
                    ValueRef::Real(number) => contents.push_str(&number.to_string()),
                    ValueRef::Null => {}
                }
                contents.push('\n');
            }
        }
    }
    assert_safe(&contents);
    Ok(contents)
}

pub async fn connected_capture_reopen_changed_goal_and_capture_only_retry() -> Result<()> {
    let root =
        std::env::temp_dir().join(format!("mecha-connected-bridge-{}", uuid::Uuid::new_v4()));
    connected_demo(&root, false).await
}

/// Runs only in a caller-selected NEW synthetic directory. Retains auditable outputs.
pub async fn connected_demo(root: &Path, retain: bool) -> Result<()> {
    connected_demo_lineage(root, retain, false).await
}

async fn connected_demo_lineage(root: &Path, retain: bool, omit_consulted: bool) -> Result<()> {
    anyhow::ensure!(!root.exists(), "synthetic demo requires a new directory");
    let mut fixture = Fixture::create(root, FIRST_GOAL)?;
    let requests = fixture.execute(false, true).await?;
    assert_safe(&requests_text(&requests));
    let artifact: Value =
        serde_json::from_slice(&std::fs::read(fixture.workspace.join("output.json"))?)?;
    assert_eq!(artifact, serde_json::to_value(rows(false))?);
    let before = std::fs::read(&fixture.session.path)?;
    assert!(String::from_utf8_lossy(&before).contains(PRIVATE_NUMBER));
    // Simulated crash boundary: execution and native recording are finished;
    // fresh authority/store handles perform persistence only, never dispatch.
    let path = root.join("bridge.memory-prototype.sqlite");
    let mut memory = Memory::create_host_capture(&path, DOMAIN)?.with_source_guard(Arc::new(
        LocalAuthority::open(&root.join("host-authority"))?,
    ));
    let receipt = fixture.capture(&mut memory)?;
    drop(memory);
    let mut reopened = Memory::open(&path)?.with_source_guard(Arc::new(LocalAuthority::open(
        &root.join("host-authority"),
    )?));
    assert_eq!(receipt, fixture.capture(&mut reopened)?);
    assert_eq!(
        before,
        std::fs::read(&fixture.session.path)?,
        "capture-only retry dispatched or rewrote execution"
    );
    let stored = scan_all_sqlite_payloads(&path)?;
    assert!(
        stored.contains("Succeeded") && stored.contains("Passed"),
        "independent successful verdict or native success receipt missing"
    );
    drop(reopened);
    // Host-retained receipt lives outside either jailed task workspace. The
    // child may supply extra provenance from this host record; native memory
    // result references are derived independently by the capture adapter.
    std::fs::write(
        root.join("first-capture-receipt.json"),
        serde_json::to_vec(&receipt)?,
    )?;
    if omit_consulted {
        std::fs::write(root.join("omit-consulted"), b"true")?;
    }
    let mut command = Command::new(std::env::current_exe()?);
    if retain {
        command.arg("--child-root").arg(root);
    } else {
        command
            .args([
                "--exact",
                "subprocess_goalmemory_changed_criterion",
                "--ignored",
                "--nocapture",
            ])
            .env("MECHA_BRIDGE_QUALIFICATION_ROOT", root);
    }
    let output = command.output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let child: Value = serde_json::from_slice(&std::fs::read(root.join("second/output.json"))?)?;
    assert_eq!(child, serde_json::to_value(rows(true))?);
    let second_receipt: mecha_core::memory::host_capture::CaptureReceipt =
        serde_json::from_slice(&std::fs::read(root.join("second-capture-receipt.json"))?)?;
    let checked = Memory::open_read_only(&path)?.with_source_guard(Arc::new(LocalAuthority::open(
        &root.join("host-authority"),
    )?));
    let Some(mecha_core::memory::Record::Evidence(second)) =
        checked.read(&second_receipt.projection, &access())?
    else {
        anyhow::bail!("host-checked second projection missing");
    };
    let second: mecha_core::memory::host_capture::HostCapture = serde_json::from_str(&second.text)?;
    assert_eq!(
        second
            .criterion
            .as_ref()
            .expect("legacy fixture criterion")
            .goal,
        SECOND_GOAL
    );
    assert_eq!(
        second.artifact.verdict,
        mecha_core::memory::host_capture::CheckVerdict::Passed
    );
    assert!(
        second.consulted.contains(&receipt.projection),
        "native memory consultation must be derived when caller omits it"
    );
    scan_all_sqlite_payloads(&path)?;
    assert_eq!(
        before,
        std::fs::read(&fixture.session.path)?,
        "second goal rewrote historical run"
    );
    if !retain {
        // The copied native memory result is a separately readable original.
        // Revocation must reach every B passage, not only B's projection.
        let mut writer =
            Memory::open(&path)?.with_source_guard(Arc::new(fixture.authority.clone()));
        let Some(mecha_core::memory::Record::Evidence(first_original)) =
            writer.read(&receipt.originals[0], &access())?
        else {
            anyhow::bail!("first original missing")
        };
        let copied = second_receipt.originals.iter().find(|reference| {
            matches!(writer.read(reference, &access()), Ok(Some(mecha_core::memory::Record::Evidence(ref e))) if e.text.contains("\"memory\""))
        }).context("B native memory ToolResult original missing")?;
        assert!(writer.read(copied, &access())?.is_some());
        let assert_withheld = || -> Result<()> {
            for reference in second_receipt
                .originals
                .iter()
                .chain(std::iter::once(&second_receipt.projection))
            {
                assert!(
                    checked.read(reference, &access())?.is_none(),
                    "B original copied recalled evidence after A invalidation"
                );
            }
            assert!(checked
                .search(
                    &mecha_core::memory::Query {
                        text: "quoted newline".into(),
                        known_at: checked.snapshot()?,
                        valid_at: None,
                        include_possible: false,
                        limit: 100
                    },
                    &access()
                )?
                .is_empty());
            Ok(())
        };
        // Correction of A's authoritative transcript invalidates its historic
        // binding without modifying B's source or SQLite dependency graph.
        std::fs::write(
            &fixture.session.path,
            String::from_utf8(before.clone())?.replace("quoted", "corrected"),
        )?;
        assert_withheld()?;
        let answer = memory_answer(&path, Some(&root.join("host-authority"))).await?;
        assert!(
            serde_json::from_str::<Value>(&answer.content)?["memory"]["records"]
                .as_array()
                .context("memory packet missing")?
                .is_empty()
        );
        std::fs::write(&fixture.session.path, &before)?;
        assert!(checked.read(copied, &access())?.is_some());
        if !omit_consulted {
            let binding = fixture.authority.binding(&fixture.confirmation)?;
            fixture.authority.withdraw(&binding.locator)?;
            assert_withheld()?;
            let answer = memory_answer(&path, Some(&root.join("host-authority"))).await?;
            assert!(
                serde_json::from_str::<Value>(&answer.content)?["memory"]["records"]
                    .as_array()
                    .context("memory packet missing")?
                    .is_empty()
            );
        }
        writer.forget(&first_original.source)?;
        assert_withheld()?;
        let answer = memory_answer(&path, Some(&root.join("host-authority"))).await?;
        assert!(
            serde_json::from_str::<Value>(&answer.content)?["memory"]["records"]
                .as_array()
                .context("memory packet missing")?
                .is_empty()
        );
        drop(writer);
        std::fs::remove_dir_all(root)?;
    }
    Ok(())
}

pub async fn subprocess_goalmemory_changed_criterion(root: &Path) -> Result<()> {
    let root = root.canonicalize()?;
    let workspace = root.join("second");
    std::fs::create_dir(&workspace)?;
    std::fs::copy(
        root.join("workspace/output.json"),
        workspace.join("prior.json"),
    )?;
    let session = Session::create(
        &root.join("second-sessions"),
        SessionMeta {
            id: "synthetic-second-task".into(),
            created_at: Utc::now(),
            provider: "authored".into(),
            model: "no-model".into(),
            workspace: workspace.clone(),
            title: None,
            kind: Some(SessionKind::Test),
        },
    )?;
    let mut convo =
        Conversation::user("Under the new criterion uppercase every value in prior.json.");
    convo.goal_anchor = Some(SECOND_GOAL.parse()?);
    session.append(&Record::GoalAnchor {
        goal: convo.goal_anchor.clone(),
    })?;
    session.append_messages(&convo.messages)?;
    let authority = LocalAuthority::open(&root.join("host-authority"))?;
    let confirmation = authority.freeze(FreezeRequest {
        transcript: session.path.clone(),
        artifact: workspace.join("output.json"),
        criterion: criterion(SECOND_GOAL),
        expected_rows: rows(true),
    })?;
    let first_receipt: mecha_core::memory::host_capture::CaptureReceipt =
        serde_json::from_slice(&std::fs::read(root.join("first-capture-receipt.json"))?)?;
    assert_eq!(
        convo.taint,
        mecha_core::agent::Taint::default(),
        "second task must start without taint"
    );
    let memory = GoalMemory::open_with_source_guard(
        &root.join("bridge.memory-prototype.sqlite"),
        access(),
        ACCOUNT.into(),
        Situation {
            event: "Uppercase fields under a new criterion".into(),
            goal: SECOND_GOAL.into(),
            needs: None,
        },
        Arc::new(LocalAuthority::open(&root.join("host-authority"))?),
    )?;
    let requests = execute(
        &workspace,
        &session,
        &mut convo,
        turns(SECOND_GOAL, false, true),
        Some(memory),
        false,
    )
    .await?;
    let memory_result = requests
        .get(1)
        .context("no request after memory dispatch")?
        .messages
        .iter()
        .flat_map(|m| &m.content)
        .find_map(|block| match block {
            Block::ToolResult {
                tool_use_id,
                content,
                is_error: false,
            } if tool_use_id == "memory" => Some(content),
            _ => None,
        })
        .context("actual memory result absent before filesystem reads")?;
    assert!(
        memory_result.contains(FIRST_GOAL) && memory_result.contains(MALICIOUS),
        "fresh-process GoalMemory did not supply historical original evidence"
    );
    assert_safe(memory_result);
    assert!(
        memory_result.len() > 4096,
        "fixture must exercise refs beyond passage cap"
    );
    let native_packet: Value = serde_json::from_str(
        memory_result
            .split_once("\n---\n")
            .context("native wrapper missing")?
            .1
            .strip_suffix("\n</untrusted-content>")
            .context("native wrapper incomplete")?,
    )?;
    let observed_refs = native_packet["memory"]["records"]
        .as_array()
        .context("memory records missing")?
        .iter()
        .map(|hit| {
            serde_json::from_value::<mecha_core::memory::RevisionRef>(hit["reference"].clone())
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let actual = requests_text(&requests);
    assert_safe(&actual);
    assert!(
        actual.contains(MALICIOUS),
        "memory evidence absent from provider requests"
    );
    assert!(
        actual.contains(FIRST_GOAL) && actual.contains(SECOND_GOAL),
        "old and new goal not distinguished"
    );
    assert!(convo.taint.private && convo.taint.untrusted);
    let egress = convo
        .messages
        .iter()
        .position(|m| {
            m.content.iter().any(|b| {
                matches!(b,
        Block::ToolUse { id, .. } if id == "attempt-egress")
            })
        })
        .context("egress probe missing")?;
    assert!(
        !convo.messages[..egress]
            .iter()
            .flat_map(|m| &m.content)
            .any(|b| matches!(b,
        Block::ToolUse { name, .. } if name == "fs_read")),
        "filesystem read masked missing memory-specific private taint"
    );
    assert!(convo.messages.iter().flat_map(|m| &m.content).any(|b| matches!(b,
        Block::ToolResult { tool_use_id, content, is_error: true } if tool_use_id == "attempt-egress" && content.contains("can send data outside this machine"))));
    let artifact: Value = serde_json::from_slice(&std::fs::read(workspace.join("output.json"))?)?;
    assert_eq!(artifact, serde_json::to_value(rows(true))?);
    let mut writer = Memory::open(&root.join("bridge.memory-prototype.sqlite"))?.with_source_guard(
        Arc::new(LocalAuthority::open(&root.join("host-authority"))?),
    );
    let checked = authority.capture(
        &mut writer,
        CaptureRequest {
            confirmation,
            domain: DOMAIN.into(),
            account: ACCOUNT.into(),
            scopes: access().scopes,
            consulted: if root.join("omit-consulted").exists() {
                vec![]
            } else {
                vec![first_receipt.projection]
            },
            records: records(),
        },
        &access(),
    )?;
    let Some(mecha_core::memory::Record::Evidence(projection)) =
        writer.read(&checked.projection, &access())?
    else {
        anyhow::bail!("second independent host check unavailable");
    };
    let projection: mecha_core::memory::host_capture::HostCapture =
        serde_json::from_str(&projection.text)?;
    assert_eq!(
        projection
            .criterion
            .as_ref()
            .expect("legacy fixture criterion")
            .goal,
        SECOND_GOAL
    );
    assert!(
        observed_refs
            .iter()
            .all(|reference| projection.consulted.contains(reference)),
        "capture must retain every returned context bundle reference"
    );
    assert_eq!(
        projection.artifact.verdict,
        mecha_core::memory::host_capture::CheckVerdict::Passed
    );
    std::fs::write(
        root.join("second-capture-receipt.json"),
        serde_json::to_vec(&checked)?,
    )?;
    std::fs::write(root.join("second-provider-requests.txt"), actual)?;
    Ok(())
}

async fn memory_answer(path: &Path, authority: Option<&Path>) -> Result<ToolOutput> {
    let situation = Situation {
        event: "Check historical quoted newline evidence".into(),
        goal: SECOND_GOAL.into(),
        needs: None,
    };
    let tool = match authority {
        Some(root) => GoalMemory::open_with_source_guard(
            path,
            access(),
            ACCOUNT.into(),
            situation,
            Arc::new(LocalAuthority::open(root)?),
        )?,
        None => GoalMemory::open(path, access(), ACCOUNT.into(), situation)?,
    };
    tool.call(
        json!({"memory":{"query":"quoted newline"}}),
        &ToolCtx::default(),
    )
    .await
}

pub async fn successful_dispatch_wrong_artifact_and_unknown_sensor_remain_distinct() -> Result<()> {
    let root = std::env::temp_dir().join(format!("mecha-bridge-wrong-{}", uuid::Uuid::new_v4()));
    let mut fixture = Fixture::create(&root, FIRST_GOAL)?;
    let requests = fixture.execute(true, false).await?;
    assert_safe(&requests_text(&requests));
    let transcript = Session::read(&fixture.session.path)?;
    let stats = transcript
        .outcomes
        .last()
        .context("native outcome missing")?;
    assert!(stats
        .homeostat
        .as_ref()
        .context("injected homeostat missing")?
        .load_avg_1m
        .is_none());
    assert!(fixture
        .convo
        .messages
        .iter()
        .flat_map(|m| &m.content)
        .any(|b| matches!(b,
        Block::ToolResult { tool_use_id, is_error: false, .. } if tool_use_id == "write")));
    let path = fixture.root.join("bridge.memory-prototype.sqlite");
    let mut memory = Memory::create_host_capture(&path, DOMAIN)?.with_source_guard(Arc::new(
        LocalAuthority::open(&fixture.root.join("host-authority"))?,
    ));
    fixture.capture(&mut memory)?;
    let payloads = scan_all_sqlite_payloads(&path)?;
    assert!(
        payloads.contains("Succeeded") && payloads.contains("Failed"),
        "independent failed verdict or native success receipt missing"
    );
    assert!(!payloads.contains("\"verdict\":\"Passed\""));
    // A missing artifact is a missing check, never a successful process inferred
    // as successful work. Use a fresh session to avoid conflicting retries.
    let missing_root = fixture.root.join("missing-case");
    let mut missing = Fixture::create(&missing_root, FIRST_GOAL)?;
    missing.execute(false, false).await?;
    std::fs::remove_file(missing.workspace.join("output.json"))?;
    let missing_path = missing.root.join("bridge.memory-prototype.sqlite");
    let mut missing_memory = Memory::create_host_capture(&missing_path, DOMAIN)?.with_source_guard(
        Arc::new(LocalAuthority::open(&missing.root.join("host-authority"))?),
    );
    missing.capture(&mut missing_memory)?;
    assert!(scan_all_sqlite_payloads(&missing_path)?.contains("Unavailable"));
    drop(missing_memory);
    drop(memory);
    std::fs::remove_dir_all(&root)?;
    Ok(())
}

pub async fn goalmemory_rechecks_missing_authority_withdrawal_and_forgetting() -> Result<()> {
    let root = std::env::temp_dir().join(format!("mecha-bridge-revoke-{}", uuid::Uuid::new_v4()));
    let mut fixture = Fixture::create(&root, FIRST_GOAL)?;
    fixture.execute(false, true).await?;
    let path = fixture.root.join("bridge.memory-prototype.sqlite");
    let authority_path = fixture.root.join("host-authority");
    let mut memory = Memory::create_host_capture(&path, DOMAIN)?
        .with_source_guard(Arc::new(LocalAuthority::open(&authority_path)?));
    let receipt = fixture.capture(&mut memory)?;
    let present = memory_answer(&path, Some(&authority_path)).await?;
    assert!(!present.is_error && present.content.contains(FIRST_GOAL));
    assert_safe(&present.content);
    let absent = memory_answer(&path, None).await?;
    assert!(!absent.content.contains(FIRST_GOAL));
    let binding = fixture.authority.binding(&fixture.confirmation)?;
    fixture.authority.withdraw(&binding.locator)?;
    let withdrawn = memory_answer(&path, Some(&authority_path)).await?;
    assert!(!withdrawn.content.contains(FIRST_GOAL));
    assert_safe(&withdrawn.content);
    assert!(fixture.capture(&mut memory).is_err());
    drop(memory);
    // Separate active authority proves actual forgetting, independently from
    // withholding an externally withdrawn source.
    let forget_root = fixture.root.join("forget-case");
    let mut forgotten = Fixture::create(&forget_root, FIRST_GOAL)?;
    forgotten.execute(false, false).await?;
    let forget_path = forgotten.root.join("bridge.memory-prototype.sqlite");
    let forget_authority = forgotten.root.join("host-authority");
    let mut forget_memory = Memory::create_host_capture(&forget_path, DOMAIN)?
        .with_source_guard(Arc::new(LocalAuthority::open(&forget_authority)?));
    let forget_receipt = forgotten.capture(&mut forget_memory)?;
    let original = forget_memory
        .read(&forget_receipt.originals[0], &access())?
        .context("original missing")?;
    let mecha_core::memory::Record::Evidence(original) = original else {
        panic!("expected original evidence")
    };
    assert!(forget_memory.forget(&original.source)?.revisions > 0);
    assert!(!memory_answer(&forget_path, Some(&forget_authority))
        .await?
        .content
        .contains(FIRST_GOAL));
    assert!(forgotten.capture(&mut forget_memory).is_err());
    let erased = scan_all_sqlite_payloads(&forget_path)?;
    assert!(
        !erased.contains(FIRST_PROMPT) && !erased.contains(MALICIOUS),
        "forgotten receipt original content survived in a table or FTS"
    );
    assert!(
        !erased.contains(&forget_receipt.projection.revision),
        "forgotten projection reference survived"
    );
    let forgotten_binding = forgotten.authority.binding(&forgotten.confirmation)?;
    assert!(
        !erased.contains(&forgotten_binding.locator),
        "forgotten external binding survived"
    );
    assert!(!receipt.id.is_empty());
    drop(forget_memory);
    std::fs::remove_dir_all(&root)?;
    Ok(())
}

pub async fn changing_confirmed_goal_after_freeze_refuses_capture_without_projection() -> Result<()>
{
    let root = std::env::temp_dir().join(format!("mecha-bridge-drift-{}", uuid::Uuid::new_v4()));
    let mut fixture = Fixture::create(&root, FIRST_GOAL)?;
    fixture.session.append(&Record::GoalAnchor {
        goal: Some(SECOND_GOAL.parse()?),
    })?;
    fixture.convo.goal_anchor = Some(SECOND_GOAL.parse()?);
    fixture.execute(false, false).await?;
    let path = fixture.root.join("bridge.memory-prototype.sqlite");
    let mut memory = Memory::create_host_capture(&path, DOMAIN)?.with_source_guard(Arc::new(
        LocalAuthority::open(&fixture.root.join("host-authority"))?,
    ));
    let error = fixture
        .capture(&mut memory)
        .expect_err("changed event-time anchor must refuse");
    assert_safe(&error.to_string());
    assert!(!scan_all_sqlite_payloads(&path)?.contains(FIRST_PROMPT));
    drop(memory);
    std::fs::remove_dir_all(&root)?;
    Ok(())
}

/// Empty host extras must not erase native GoalMemory provenance.
pub async fn connected_capture_derives_omitted_consultation() -> Result<()> {
    let root = std::env::temp_dir().join(format!("mecha-omitted-bridge-{}", uuid::Uuid::new_v4()));
    connected_demo_lineage(&root, false, true).await
}
