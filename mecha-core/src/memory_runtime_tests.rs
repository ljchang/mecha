use super::*;
use crate::config::{HookConfig, PermissionMode};
use crate::policy::{ExecPolicy, RuleConfig, RuleDecision};
use crate::tool::{Approver, Decision, Egress, ModeApprover, ToolCtx};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

// Pure dispatcher cases exercise its guards without constructing a lifecycle.
// Public acquire always supplies the sealed durable guard; lifecycle cases below
// exercise that path and its second-connection cancellation boundary.
async fn dispatch_read(
    spec: &TaskSpec,
    cx: &RunContext,
    convo: &mut Conversation,
) -> Result<ReadResult> {
    super::dispatch_read(spec, cx, convo, None).await
}

struct Fixture {
    root: PathBuf,
    spec: TaskSpec,
    cx: RunContext,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("memory-read-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        std::fs::write(
            root.join("fixture.txt"),
            "synthetic observation\nsecond line\n",
        )
        .unwrap();
        let spec = TaskSpec {
            workspace: root.clone(),
            path: "fixture.txt".into(),
            max_bytes: 4096,
            max_lines: 1,
        };
        let cx = RunContext::new(
            ToolCtx {
                workspace: root.clone(),
                spill_dir: None,
                ..ToolCtx::default()
            },
            Arc::new(ModeApprover {
                mode: PermissionMode::Allow,
            }),
        );
        Self { root, spec, cx }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn bounded_read_preserves_taint_and_does_not_append_orphan_results() {
    let f = Fixture::new();
    let mut convo = Conversation::user("Read the synthetic fixture");
    convo.taint.untrusted = true;
    let before = serde_json::to_value(&convo.messages).unwrap();
    let result = dispatch_read(&f.spec, &f.cx, &mut convo).await.unwrap();
    assert_eq!(result.status, ReadStatus::Successful);
    assert!(result.content.contains("synthetic observation"));
    assert!(!result.content.contains("second line"));
    assert!(!result.external);
    assert!(convo.taint.trifecta_armed());
    assert_eq!(serde_json::to_value(&convo.messages).unwrap(), before);
    assert_eq!(result.trace.len(), 1);
    assert_eq!(result.trace[0].name, "fs_read");
    let mut registry = Registry::new();
    registry.insert(Arc::new(ChosenSend));
    let agent = Agent::new(
        Box::new(NoInference),
        registry,
        f.cx.approver.clone(),
        (*f.cx.tools).clone(),
        AgentConfig::default(),
        None,
    )
    .unwrap();
    let blocked = agent
        .dispatch_one(&f.cx, &mut convo, "chosen_send", json!({}), &None)
        .await;
    assert!(blocked.denied);
    assert!(blocked.trace[0].blocked);
}

#[tokio::test]
async fn prior_cancellation_does_not_dispatch() {
    let mut f = Fixture::new();
    let token = CancellationToken::new();
    token.cancel();
    f.cx = f.cx.clone().with_cancel(token);
    let result = dispatch_read(&f.spec, &f.cx, &mut Conversation::new())
        .await
        .unwrap();
    assert_eq!(result.status, ReadStatus::CancelledBeforeDispatch);
    assert!(result.cancelled);
    assert!(result.call_id.is_none());
    assert!(result.trace.is_empty());
}

#[tokio::test]
async fn policy_refusal_and_tool_failure_remain_distinct() {
    let mut f = Fixture::new();
    f.cx = f.cx.clone().with_policy(Arc::new(
        ExecPolicy::from_config(
            &[RuleConfig {
                tool: "fs_read".into(),
                decision: RuleDecision::Forbid,
                ..RuleConfig::default()
            }],
            true,
        )
        .unwrap(),
    ));
    let result = dispatch_read(&f.spec, &f.cx, &mut Conversation::new())
        .await
        .unwrap();
    assert_eq!(result.status, ReadStatus::RefusedPolicy);
    assert!(!result.external);
    assert!(!result.private_taint);
    f.cx = f.cx.clone().with_policy(Arc::new(ExecPolicy::default()));
    std::fs::remove_file(f.root.join("fixture.txt")).unwrap();
    let failed = dispatch_read(&f.spec, &f.cx, &mut Conversation::new())
        .await
        .unwrap();
    assert_eq!(failed.status, ReadStatus::Failed);
}

#[tokio::test]
async fn pre_tool_hook_is_not_bypassed() {
    let mut f = Fixture::new();
    f.cx = f.cx.clone().with_hooks(Arc::new(
        crate::hooks::HookSet::from_config(&[HookConfig {
            event: "pre_tool".into(),
            command: "exit 2".into(),
            tools: vec!["fs_read".into()],
            timeout_secs: Some(1),
        }])
        .unwrap(),
    ));
    let result = dispatch_read(&f.spec, &f.cx, &mut Conversation::new())
        .await
        .unwrap();
    assert_eq!(result.status, ReadStatus::RefusedPolicy);
    assert!(!result.private_taint);
}

struct ChosenSend;

#[async_trait]
impl Tool for ChosenSend {
    fn name(&self) -> &str {
        "chosen_send"
    }
    fn description(&self) -> &str {
        "Synthetic chosen egress that must never execute"
    }
    fn input_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            egress: Egress::Chosen,
            ..Capabilities::default()
        }
    }
    async fn call(&self, _: Value, _: &ToolCtx) -> Result<ToolOutput> {
        panic!("tainted send executed")
    }
}

#[test]
fn recovery_never_resets_or_guesses_clean_taint() {
    let mut conversation = Conversation::new();
    restore_observation_taint(None, &mut conversation);
    assert!(conversation.taint.trifecta_armed());
    restore_observation_taint(
        Some(&json!({ "observation": {
            "private_taint": false, "untrusted_taint": false,
        }})),
        &mut conversation,
    );
    assert!(conversation.taint.trifecta_armed());
    let mut clean = Conversation::new();
    restore_observation_taint(
        Some(&json!({ "observation": {
            "private_taint": true, "untrusted_taint": false,
        }})),
        &mut clean,
    );
    assert!(clean.taint.private);
    assert!(!clean.taint.untrusted);
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_escape_is_refused_without_requesting_a_grant() {
    let f = Fixture::new();
    let outside = Fixture::new();
    std::fs::remove_file(f.root.join("fixture.txt")).unwrap();
    std::os::unix::fs::symlink(outside.root.join("fixture.txt"), f.root.join("fixture.txt"))
        .unwrap();
    let result = dispatch_read(&f.spec, &f.cx, &mut Conversation::new())
        .await
        .unwrap();
    assert_eq!(result.status, ReadStatus::RefusedPolicy);
    assert!(!result.content.contains("synthetic observation"));
    assert!(!result.private_taint);
    assert!(f.cx.tools.grants.live().is_empty());
}

struct CancellingApprover(CancellationToken);

#[async_trait]
impl Approver for CancellingApprover {
    async fn approve(&self, _: &dyn Tool, _: &Value) -> Decision {
        self.0.cancel();
        Decision::Allow
    }
    async fn consult(&self, tool: &dyn Tool, input: &Value, _: &str) -> Decision {
        self.approve(tool, input).await
    }
}

#[tokio::test]
async fn cancellation_during_approval_prevents_the_read() {
    let mut f = Fixture::new();
    let token = CancellationToken::new();
    f.cx = f.cx.clone().with_cancel(token.clone());
    f.cx.approver = Arc::new(CancellingApprover(token));
    f.cx = f.cx.clone().with_policy(Arc::new(
        ExecPolicy::from_config(
            &[RuleConfig {
                tool: "fs_read".into(),
                decision: RuleDecision::Prompt,
                ..RuleConfig::default()
            }],
            true,
        )
        .unwrap(),
    ));
    let result = dispatch_read(&f.spec, &f.cx, &mut Conversation::new())
        .await
        .unwrap();
    assert_eq!(result.status, ReadStatus::RefusedPolicy);
    assert!(result.cancelled);
    assert!(!result.content.contains("synthetic observation"));
    // The existing dispatcher conservatively arms private reach for a tool that
    // ran, including an in-process refusal. The adapter never clears it.
    assert!(result.private_taint);
}

fn lifecycle(f: &Fixture) -> (Memory, Receipt, Access, PathBuf) {
    let path = f.root.join("test.memory-prototype.sqlite");
    let mut memory = Memory::create_durable(&path, "synthetic").unwrap();
    let access = Access {
        domain: "synthetic".into(),
        scopes: ["fixture".into()].into(),
    };
    let receipt = memory.durable_create(f.spec.clone(), &[], &access).unwrap();
    (memory, receipt, access, path)
}

#[tokio::test]
async fn lifecycle_reuses_original_bytes_after_reopen_and_restores_taint() {
    let f = Fixture::new();
    let (mut memory, receipt, access, path) = lifecycle(&f);
    let mut convo = Conversation::new();
    convo.taint.untrusted = true;
    let first = acquire(&mut memory, &receipt, &access, &f.cx, &mut convo)
        .await
        .unwrap();
    assert_eq!(first.phase, Phase::Observed);
    std::fs::remove_file(f.root.join("fixture.txt")).unwrap();
    drop(memory);
    let mut reopened = Memory::open(&path).unwrap();
    let receipt = reopened.durable_reopen(receipt.id(), &access).unwrap();
    let mut resumed = Conversation::new();
    let again = acquire(&mut reopened, &receipt, &access, &f.cx, &mut resumed)
        .await
        .unwrap();
    assert_eq!(again.observation, first.observation);
    assert!(resumed.taint.trifecta_armed());
    let mut registry = Registry::new();
    registry.insert(Arc::new(ChosenSend));
    let agent = Agent::new(
        Box::new(NoInference),
        registry,
        f.cx.approver.clone(),
        (*f.cx.tools).clone(),
        AgentConfig::default(),
        None,
    )
    .unwrap();
    let blocked = agent
        .dispatch_one(&f.cx, &mut resumed, "chosen_send", json!({}), &None)
        .await;
    assert!(blocked.denied && blocked.trace[0].blocked);
    let result = reopened
        .durable_observation(&receipt, &access)
        .unwrap()
        .unwrap();
    assert!(result["observation"]["content"]
        .as_str()
        .unwrap()
        .contains("synthetic observation"));
    assert_eq!(result["observation"]["external"], false);
}

#[tokio::test]
async fn pre_cancelled_lifecycle_stays_cancelled_after_reopen_and_cannot_commit() {
    let mut f = Fixture::new();
    let (mut memory, receipt, access, path) = lifecycle(&f);
    let token = CancellationToken::new();
    token.cancel();
    f.cx = f.cx.clone().with_cancel(token);
    let status = acquire(
        &mut memory,
        &receipt,
        &access,
        &f.cx,
        &mut Conversation::new(),
    )
    .await
    .unwrap();
    assert_eq!(status.phase, Phase::Cancelled);
    let result = memory
        .durable_observation(&receipt, &access)
        .unwrap()
        .unwrap();
    assert_eq!(result["observation"]["status"], "cancelled_before_dispatch");
    assert_eq!(result["observation"]["call_id"], Value::Null);
    drop(memory);
    let mut reopened = Memory::open(&path).unwrap();
    let receipt = reopened.durable_reopen(receipt.id(), &access).unwrap();
    assert_eq!(
        reopened.durable_status(&receipt, &access).unwrap().phase,
        Phase::Cancelled
    );
    assert!(reopened
        .durable_reserve_reconsideration(&receipt, &access)
        .is_err());
}

#[tokio::test]
async fn cancellation_after_read_retains_observation_and_prevents_later_commit() {
    let mut f = Fixture::new();
    let (mut memory, receipt, access, path) = lifecycle(&f);
    let token = CancellationToken::new();
    f.cx = f.cx.clone().with_cancel(token.clone()).with_hooks(Arc::new(
        crate::hooks::HookSet::from_config(&[HookConfig {
            event: "post_tool".into(),
            command: ": > observed; while [ ! -e release ]; do sleep 0.01; done".into(),
            tools: vec!["fs_read".into()],
            timeout_secs: Some(5),
        }])
        .unwrap(),
    ));
    let root = f.root.clone();
    let canceller = tokio::spawn(async move {
        tokio::time::timeout(std::time::Duration::from_secs(4), async {
            while !root.join("observed").exists() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        token.cancel();
        std::fs::write(root.join("release"), "").unwrap();
    });
    let status = acquire(
        &mut memory,
        &receipt,
        &access,
        &f.cx,
        &mut Conversation::new(),
    )
    .await
    .unwrap();
    canceller.await.unwrap();
    assert_eq!(status.phase, Phase::Cancelled);
    let result = memory
        .durable_observation(&receipt, &access)
        .unwrap()
        .unwrap();
    assert_eq!(result["observation"]["status"], "successful");
    assert_eq!(result["observation"]["cancelled"], true);
    assert!(result["observation"]["content"]
        .as_str()
        .unwrap()
        .contains("synthetic observation"));
    drop(memory);
    let mut reopened = Memory::open(&path).unwrap();
    let receipt = reopened.durable_reopen(receipt.id(), &access).unwrap();
    assert!(reopened
        .durable_reserve_reconsideration(&receipt, &access)
        .is_err());
}

struct HumanDenial;

#[async_trait]
impl Approver for HumanDenial {
    async fn approve(&self, _: &dyn Tool, _: &Value) -> Decision {
        Decision::Deny("synthetic owner refusal".into())
    }
    async fn consult(&self, tool: &dyn Tool, input: &Value, _: &str) -> Decision {
        self.approve(tool, input).await
    }
}

#[tokio::test]
async fn user_refusal_and_withheld_tool_keep_their_recorded_origin() {
    let mut f = Fixture::new();
    f.cx.approver = Arc::new(HumanDenial);
    f.cx = f.cx.clone().with_policy(Arc::new(
        ExecPolicy::from_config(
            &[RuleConfig {
                tool: "fs_read".into(),
                decision: RuleDecision::Prompt,
                ..RuleConfig::default()
            }],
            true,
        )
        .unwrap(),
    ));
    let result = dispatch_read(&f.spec, &f.cx, &mut Conversation::new())
        .await
        .unwrap();
    assert_eq!(result.status, ReadStatus::RefusedUser);
    assert!(!result.external);
    f.cx = f.cx.clone().withholding(["fs_read".into()]);
    let result = dispatch_read(&f.spec, &f.cx, &mut Conversation::new())
        .await
        .unwrap();
    assert_eq!(result.status, ReadStatus::RefusedPolicy);
    assert!(!result.private_taint);
}

#[tokio::test]
async fn byte_bound_and_host_selected_input_cannot_expand() {
    let mut f = Fixture::new();
    f.spec.max_bytes = 1024;
    std::fs::write(f.root.join("fixture.txt"), "x".repeat(20_000)).unwrap();
    let result = dispatch_read(&f.spec, &f.cx, &mut Conversation::new())
        .await
        .unwrap();
    assert!(result.content.len() <= f.spec.max_bytes);
    assert!(result.content.contains("truncated"));
    let bound = BoundRead {
        input: bound_input(&f.spec),
        workspace: f.root.clone(),
        max_bytes: f.spec.max_bytes,
        guard: None,
    };
    let result = bound
        .call(
            json!({"path":"other.txt", "offset":1, "limit":256}),
            &f.cx.tools,
        )
        .await
        .unwrap();
    assert!(result.refusal);
    f.spec.path = "../outside.txt".into();
    assert!(dispatch_read(&f.spec, &f.cx, &mut Conversation::new())
        .await
        .is_err());
    f.spec.path = ".mecha/sessions/source.jsonl".into();
    assert!(dispatch_read(&f.spec, &f.cx, &mut Conversation::new())
        .await
        .is_err());
}

#[tokio::test]
async fn an_outbox_route_cannot_create_a_second_read_owner() {
    let mut f = Fixture::new();
    let store = crate::outbox::OutboxStore::open(f.root.join("outbox")).unwrap();
    f.cx =
        f.cx.clone()
            .with_outbox(Arc::new(crate::outbox::OutboxRoute::new(
                store,
                ["fs_read".into()],
                [],
            )));
    let result = dispatch_read(&f.spec, &f.cx, &mut Conversation::new())
        .await
        .unwrap();
    assert_eq!(result.status, ReadStatus::RefusedPolicy);
    assert!(result.call_id.is_none());
    assert!(f
        .cx
        .outbox
        .as_ref()
        .unwrap()
        .store
        .items()
        .unwrap()
        .is_empty());
}

struct PersistedCancellation {
    path: PathBuf,
    id: String,
    access: Access,
}

#[async_trait]
impl Approver for PersistedCancellation {
    async fn approve(&self, _: &dyn Tool, _: &Value) -> Decision {
        let mut other = Memory::open(&self.path).unwrap();
        let receipt = other.durable_reopen(&self.id, &self.access).unwrap();
        other.durable_cancel(&receipt, &self.access).unwrap();
        Decision::Allow
    }
    async fn consult(&self, tool: &dyn Tool, input: &Value, _: &str) -> Decision {
        self.approve(tool, input).await
    }
}

#[tokio::test]
async fn persisted_cancellation_during_approval_prevents_the_read_without_a_token() {
    let mut f = Fixture::new();
    let (mut memory, receipt, access, path) = lifecycle(&f);
    let token = CancellationToken::new();
    f.cx =
        f.cx.clone()
            .with_cancel(token.clone())
            .with_policy(Arc::new(
                ExecPolicy::from_config(
                    &[RuleConfig {
                        tool: "fs_read".into(),
                        decision: RuleDecision::Prompt,
                        ..RuleConfig::default()
                    }],
                    true,
                )
                .unwrap(),
            ));
    f.cx.approver = Arc::new(PersistedCancellation {
        path,
        id: receipt.id().into(),
        access: access.clone(),
    });
    let status = acquire(
        &mut memory,
        &receipt,
        &access,
        &f.cx,
        &mut Conversation::new(),
    )
    .await
    .unwrap();
    assert!(
        !token.is_cancelled(),
        "this test must exercise persisted cancellation alone"
    );
    assert_eq!(status.phase, Phase::Cancelled);
    let result = memory
        .durable_observation(&receipt, &access)
        .unwrap()
        .unwrap();
    assert_eq!(result["observation"]["status"], "refused_policy");
    assert!(!result["observation"]["content"]
        .as_str()
        .unwrap()
        .contains("synthetic observation"));
    assert!(memory
        .durable_reserve_reconsideration(&receipt, &access)
        .is_err());
}
