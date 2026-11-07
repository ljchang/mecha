use super::*;
use crate::memory::host_capture::CheckVerdict;
use crate::message::{Block, Message, StopReason, Usage};
use crate::{
    memory::host_capture::ConfirmedCriterion,
    message::{CompletionRequest, CompletionResponse},
    provider::{Provider, StreamSink},
};
use anyhow::Result;
use serde_json::json;
use std::collections::VecDeque;
use std::{collections::BTreeMap, fs, path::Path, sync::Mutex};
use tokio_util::sync::CancellationToken;

struct Script(Mutex<VecDeque<Result<Message>>>);
#[async_trait::async_trait]
impl Provider for Script {
    fn id(&self) -> &str {
        "script"
    }
    fn default_model(&self) -> &str {
        "script"
    }
    fn local(&self) -> bool {
        true
    }
    fn wire_body(&self, req: &CompletionRequest, _: bool) -> Option<serde_json::Value> {
        Some(json!({"think":req.think,"think_budget":req.think_budget,"local_only":req.local_only}))
    }
    async fn complete(
        &self,
        req: &CompletionRequest,
        _: Option<&StreamSink>,
    ) -> Result<CompletionResponse> {
        assert_eq!(req.think, Some(false));
        assert!(req.local_only);
        assert!(req.think_budget.is_none());
        let message = self
            .0
            .lock()
            .unwrap()
            .pop_front()
            .expect("extra inference")?;
        let tool = message
            .content
            .iter()
            .any(|b| matches!(b, Block::ToolUse { .. }));
        Ok(CompletionResponse {
            message,
            stop_reason: if tool {
                StopReason::ToolUse
            } else {
                StopReason::EndTurn
            },
            usage: Usage::default(),
            refusal: None,
            model: "script".into(),
            malformed_tool_args: 0,
        })
    }
}
fn call(id: &str, name: &str, input: serde_json::Value) -> Result<Message> {
    Ok(Message::assistant(vec![Block::ToolUse {
        id: id.into(),
        name: name.into(),
        input,
    }]))
}
fn done() -> Result<Message> {
    Ok(Message::assistant(vec![Block::text("done")]))
}
fn req() -> TrialRequest {
    TrialRequest{ version:1,task_id:"synthetic".into(),prompt:"Write JSON rows to output.json. Prior calibration evidence can be retrieved with goal_context memory query calibration.".into(),criterion:ConfirmedCriterion{goal:"task:calibration".into(),goal_text:"calibration".into(),criterion_text:"Preserve calibration rows".into(),criterion_revision:"a".repeat(64),confirmation_digest:"b".repeat(64)},expected_rows:vec![BTreeMap::from([("value".into(),"GOLD_ONLY_CANARY".into())])],files:BTreeMap::from([("input.csv".into(),"value\ncalibration\n".into())]),source:None,withdraw_source:false}
}
async fn trial(root: &Path, request: TrialRequest, script: Vec<Result<Message>>) -> TrialReport {
    run_trial(
        root,
        request,
        Box::new(Script(Mutex::new(script.into()))),
        TrialConfig::default(),
        CancellationToken::new(),
    )
    .await
    .unwrap()
}
#[tokio::test]
async fn native_wrong_artifact_capture_and_unknown_usage_stay_distinct() {
    let t = Temp::new();
    let r = trial(
        &t.path().join("run"),
        req(),
        vec![
            call(
                "w",
                "fs_write",
                json!({"path":"output.json","content":"[]"}),
            ),
            done(),
        ],
    )
    .await;
    assert_eq!(r.status, "completed");
    assert_eq!(r.artifact_check.verdict, CheckVerdict::Failed);
    assert!(r.capture_receipt.is_some(), "{:?}", r.capture_error);
    assert_eq!(r.model_calls_started, 2);
    assert_eq!(r.model_calls_completed, 2);
    assert_eq!(r.input_tokens, None);
    assert_eq!(r.output_tokens, None);
    let audit = fs::read_to_string(&r.provider_audit).unwrap();
    assert!(!audit.contains("GOLD_ONLY_CANARY"));
    assert!(audit.contains("wire_body"));
    assert!(r.database.is_file());
    assert!(r.transcript.is_file());
}
#[tokio::test]
async fn error_retains_partial_execution_and_refuses_capture() {
    let t = Temp::new();
    let r = trial(
        &t.path().join("run"),
        req(),
        vec![
            call(
                "w",
                "fs_write",
                json!({"path":"output.json","content":"[]"}),
            ),
            Err(anyhow::anyhow!("transport failed")),
        ],
    )
    .await;
    assert_eq!(r.status, "error");
    assert!(r.run_error.unwrap().contains("transport failed"));
    assert_eq!(r.model_calls_started, 2);
    assert_eq!(r.model_calls_completed, 1);
    assert!(r.capture_receipt.is_none());
    assert!(r.capture_error.is_some());
    assert!(fs::read_to_string(r.transcript)
        .unwrap()
        .contains("fs_write"));
    assert_eq!(r.artifact_check.verdict, CheckVerdict::Failed);
}
#[tokio::test]
async fn source_copy_recall_withdrawal_empty_control_and_original_immutability() {
    let t = Temp::new();
    let a = trial(
        &t.path().join("a"),
        req(),
        vec![
            call(
                "w",
                "fs_write",
                json!({"path":"output.json","content":"[{\"value\":\"GOLD_ONLY_CANARY\"}]"}),
            ),
            done(),
        ],
    )
    .await;
    assert_eq!(a.artifact_check.verdict, CheckVerdict::Passed);
    let original_db = fs::read(&a.database).unwrap();
    let original_transcript = fs::read(&a.transcript).unwrap();
    let original_authority: BTreeMap<_, _> = fs::read_dir(&a.authority)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (e.file_name(), fs::read(e.path()).unwrap())
        })
        .collect();
    for (name, source, withdraw, expect) in [
        ("memory", true, false, true),
        ("control", false, false, false),
        ("withdraw", true, true, false),
    ] {
        let mut b = req();
        b.source = source.then(|| TrialSource {
            database: a.database.clone(),
            authority: a.authority.clone(),
            confirmation: a.confirmation.clone().unwrap(),
        });
        b.withdraw_source = withdraw;
        let r = trial(
            &t.path().join(name),
            b,
            vec![
                call(
                    "m",
                    "goal_context",
                    json!({"memory":{"query":"calibration"}}),
                ),
                Ok(Message::assistant(vec![
                    Block::ToolUse{id:"jail".into(),name:"fs_read".into(),input:json!({"path":a.artifact})},
                    Block::ToolUse{id:"gold-jail".into(),name:"fs_read".into(),input:json!({"path":a.authority.join(format!("{}.confirmation",a.confirmation.as_ref().unwrap()))})}
                ])),
                done(),
            ],
        )
        .await;
        assert_eq!(r.recall_calls, 1);
        assert_eq!(!r.recall_references.is_empty(), expect);
        assert_eq!(r.taint.untrusted, expect);
        let transcript = Session::read(&r.transcript).unwrap();
        for id in ["jail", "gold-jail"] {
            assert!(transcript.convo.messages.iter().flat_map(|m|&m.content).any(|b|matches!(b,Block::ToolResult{tool_use_id,is_error:true,..} if tool_use_id==id)),"outside path should be refused");
        }
        let audit = fs::read_to_string(r.provider_audit).unwrap();
        if !expect {
            assert!(!audit.contains("GOLD_ONLY_CANARY"));
        }
    }
    assert_eq!(fs::read(a.database).unwrap(), original_db);
    assert_eq!(fs::read(a.transcript).unwrap(), original_transcript);
    let after_authority: BTreeMap<_, _> = fs::read_dir(&a.authority)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (e.file_name(), fs::read(e.path()).unwrap())
        })
        .collect();
    assert_eq!(after_authority, original_authority);
}
#[tokio::test]
async fn rejects_unsafe_input_before_directory_creation() {
    let t = Temp::new();
    let path = t.path().join("run");
    let mut r = req();
    r.files.insert("../gold.json".into(), "x".into());
    assert!(run_trial(
        &path,
        r,
        Box::new(Script(Mutex::new(VecDeque::new()))),
        TrialConfig::default(),
        CancellationToken::new()
    )
    .await
    .is_err());
    assert!(!path.exists());
}

struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("mecha-trial-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&p).unwrap();
        Self(p.canonicalize().unwrap())
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn independent_artifact_survives_existing_passage_limit_refusal() {
    let t = Temp::new();
    let mut blocks = (0..17)
        .map(|_| Block::text("recorded text"))
        .collect::<Vec<_>>();
    blocks.push(Block::ToolUse {
        id: "write".into(),
        name: "fs_write".into(),
        input: json!({"path":"output.json","content":"[{\"value\":\"GOLD_ONLY_CANARY\"}]"}),
    });
    let r = trial(
        &t.path().join("run"),
        req(),
        vec![Ok(Message::assistant(blocks)), done()],
    )
    .await;
    assert_eq!(r.status, "completed");
    assert_eq!(r.artifact_check.verdict, CheckVerdict::Passed);
    assert!(r.capture_receipt.is_none());
    assert!(r.capture_error.is_some());
}
struct Hangs;
#[async_trait::async_trait]
impl Provider for Hangs {
    fn id(&self) -> &str {
        "hangs"
    }
    fn default_model(&self) -> &str {
        "hangs"
    }
    fn local(&self) -> bool {
        true
    }
    async fn complete(
        &self,
        _: &CompletionRequest,
        _: Option<&StreamSink>,
    ) -> Result<CompletionResponse> {
        std::future::pending().await
    }
}
#[tokio::test]
async fn timeout_preserves_started_audit_and_does_not_invent_completion() {
    let t = Temp::new();
    let r = run_trial(
        &t.path().join("run"),
        req(),
        Box::new(Hangs),
        TrialConfig {
            timeout_secs: 1,
            ..Default::default()
        },
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(r.status, "timeout");
    assert_eq!(r.model_calls_started, 1);
    assert_eq!(r.model_calls_completed, 0);
    assert!(r.capture_receipt.is_none());
    assert!(r.capture_error.is_some());
    assert!(fs::read_to_string(r.provider_audit)
        .unwrap()
        .contains("request_started"));
    assert!(fs::read_to_string(r.transcript)
        .unwrap()
        .contains("interrupted"));
}

struct PartialThenHangs;
#[async_trait::async_trait]
impl Provider for PartialThenHangs {
    fn id(&self) -> &str {
        "partial"
    }
    fn default_model(&self) -> &str {
        "partial"
    }
    fn local(&self) -> bool {
        true
    }
    async fn complete(
        &self,
        _: &CompletionRequest,
        sink: Option<&StreamSink>,
    ) -> Result<CompletionResponse> {
        sink.unwrap()
            .send(crate::provider::StreamEvent::TextDelta(
                "PARTIAL_NATIVE_TEXT".into(),
            ))
            .unwrap();
        std::future::pending().await
    }
}
#[tokio::test]
async fn deadline_drains_native_partial_stream_before_recording() {
    let t = Temp::new();
    let r = run_trial(
        &t.path().join("run"),
        req(),
        Box::new(PartialThenHangs),
        TrialConfig {
            timeout_secs: 1,
            ..Default::default()
        },
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(r.status, "timeout");
    assert_eq!(r.model_calls_started, 1);
    assert_eq!(r.model_calls_completed, 0);
    let text = fs::read_to_string(r.transcript).unwrap();
    assert!(
        text.contains("PARTIAL_NATIVE_TEXT"),
        "deadline discarded native streamed partial"
    );
    assert!(r.elapsed_ms < 3000);
    assert_eq!(r.config.timeout_secs, 1);
}

struct PartialThenError;
#[async_trait::async_trait]
impl Provider for PartialThenError {
    fn id(&self) -> &str {
        "partial-error"
    }
    fn default_model(&self) -> &str {
        "partial-error"
    }
    fn local(&self) -> bool {
        true
    }
    async fn complete(
        &self,
        _: &CompletionRequest,
        sink: Option<&StreamSink>,
    ) -> Result<CompletionResponse> {
        sink.unwrap()
            .send(crate::provider::StreamEvent::TextDelta(
                "PARTIAL_TRANSPORT_SENTINEL".into(),
            ))
            .unwrap();
        anyhow::bail!("transport ended midstream")
    }
}
#[tokio::test]
async fn transport_error_retains_actual_stream_events_without_fabricated_response() {
    let t = Temp::new();
    let r = run_trial(
        &t.path().join("run"),
        req(),
        Box::new(PartialThenError),
        TrialConfig::default(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(r.status, "error");
    assert_eq!(r.model_calls_started, 1);
    assert_eq!(r.model_calls_completed, 0);
    assert!(r.run_error.unwrap().contains("transport ended midstream"));
    assert!(
        fs::read_to_string(r.provider_audit)
            .unwrap()
            .contains("PARTIAL_TRANSPORT_SENTINEL"),
        "audit discarded received streaming data"
    );
    assert!(!fs::read_to_string(r.transcript)
        .unwrap()
        .contains("\"record\":\"outcome\""));
}
