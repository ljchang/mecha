//! Authored host-boundary qualification; no provider, owner store, or network.
use super::*;
use crate::{
    agent::{Conversation, StopCause, Taint},
    goal_context::{CharterSnapshot, DecisionBinding},
    message::{Block, Message},
    session::{RunConfig, RunStats, SessionMeta},
    tool::{todo::memory::PlanCall, ToolCtx},
};
struct Fixture {
    root: PathBuf,
    request: SessionMemoryRequest,
    session: Session,
}
impl Fixture {
    fn new() -> Result<Self> {
        let root = std::env::temp_dir()
            .canonicalize()?
            .join(format!("task-host-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root)?;
        for dir in ["db", "keys", "work", "sessions"] {
            crate::create_private_dir(&root.join(dir))?;
        }
        let request = SessionMemoryRequest {
            protocol: SESSION_MEMORY_PROTOCOL.into(),
            db: root.join("db/test.memory-prototype.sqlite"),
            key_file: root.join("keys/key"),
            authority: root.join("authority"),
            domain: "fixture".into(),
            account: "fixture".into(),
            scopes: Default::default(),
        };
        SessionMemory::initialize(&request)?;
        let session = Session::create(
            &root.join("sessions"),
            SessionMeta {
                id: Session::new_id(),
                created_at: chrono::Utc::now(),
                provider: "fixture".into(),
                model: "fixture".into(),
                workspace: root.join("work"),
                title: None,
                kind: Some(SessionKind::Chat),
            },
        )?;
        Ok(Self {
            root,
            request,
            session,
        })
    }
    fn memory(&self) -> Result<SessionMemory> {
        SessionMemory::open(&self.request, &self.root.join("work"))
    }
    fn draft() -> crate::memory::TaskDraft {
        crate::memory::TaskDraft {
            title: "Authored container".into(),
            intent: "Private task metadata sentinel".into(),
        }
    }
    fn tools() -> Vec<String> {
        vec!["fs_read".into(), "goal_context".into(), "todo".into()]
    }
    fn chat(&self) -> Result<ChatMemory> {
        self.memory()?.enable_planning()?;
        let chat =
            self.memory()?
                .attach_task_chat(&self.session, Some(Self::draft()), &Self::tools())?;
        chat.task_record_config(RunConfig {
            workspace: self.root.join("work"),
            tools: Self::tools(),
            ..Default::default()
        })?;
        Ok(chat)
    }
    async fn prepared(
        &self,
        chat: &mut ChatMemory,
        count: u32,
    ) -> Result<(
        ChatMemoryTurn,
        super::super::native_writer::PreparedNativeAppend,
    )> {
        self.prepared_with_stop(chat, count, StopCause::Completed)
            .await
    }
    async fn prepared_with_stop(
        &self,
        chat: &mut ChatMemory,
        count: u32,
        stop: StopCause,
    ) -> Result<(
        ChatMemoryTurn,
        super::super::native_writer::PreparedNativeAppend,
    )> {
        let owner = OwnerInput::new(OwnerSource::CliChat, "Inspect authored fixture".into())?;
        let turn = chat.begin_turn(&self.session, &owner)?;
        chat.task_record_owner(&turn, &owner)?;
        let runtime = chat.task_runtime.as_ref().unwrap();
        let (live, live_owner) =
            task_plan::LivePlanTurn::from_host(Arc::clone(&runtime.binding), &turn)?;
        let mut convo = Conversation::default();
        convo.messages.push(Message::user(owner.text()));
        let before = convo.messages.clone();
        let version = convo
            .goal_context
            .transition(CharterSnapshot::disabled(), Some(owner))?
            .unwrap();
        let decision = DecisionBinding::new(
            convo.goal_context.next_decision(),
            version.revision(),
            &convo.messages,
            None,
            true,
            0,
        )?
        .with_conversation_messages(&convo.messages)?;
        let stamp = crate::planning::GoalDecisionStamp::capture(&decision, &version)?;
        convo.goal_context.bind(decision.clone())?;
        runtime
            .writer
            .lock()
            .unwrap()
            .append(&Record::GoalContextDecision {
                binding: decision,
                versions: vec![version.clone()],
            })?;
        if count > 0 {
            let mut response = Message::assistant(vec![]);
            let mut result = Message::tool_results(vec![]);
            let mut feedback = crate::planning::Feedback::default();
            for ordinal in 0..count {
                let call_id = format!("authored-{ordinal}");
                let input = serde_json::json!({"items":[{"content":"Inspect authored item","status":if ordinal == 0 {"in_progress"} else {"completed"},"check":"never execute this fixture check"}]});
                let call = Arc::new(PlanCall::new(
                    Arc::clone(&live),
                    ordinal,
                    0,
                    stamp.clone(),
                    None,
                    24000,
                ));
                let ctx = ToolCtx {
                    workspace: self.root.join("work"),
                    conversation: Some(self.session.meta.id.clone()),
                    call_id: Some(call_id.clone()),
                    task_plan: Some(Arc::clone(&live)),
                    plan_call: Some(Arc::clone(&call)),
                    goal_decision: Arc::new(Mutex::new(Some(stamp.clone()))),
                    goal_context_version: Arc::new(Mutex::new(Some(version.clone()))),
                    ..Default::default()
                };
                let out = runtime.todo.call(input.clone(), &ctx).await?;
                ensure!(!out.is_error, "{}", out.content);
                response.content.push(Block::ToolUse {
                    id: call_id.clone(),
                    name: "todo".into(),
                    input,
                });
                result.content.push(Block::ToolResult {
                    tool_use_id: call_id.clone(),
                    content: out.content,
                    is_error: false,
                });
                result.tool_provenance.insert(call_id, false);
                let (mut audit, contribution) = call.take().unwrap();
                audit.result_index = ordinal as usize;
                audit.steps_start = feedback.steps.len();
                audit.decisions_start = feedback.decisions.len();
                feedback.steps.extend(contribution.steps);
                feedback.decisions.extend(contribution.decisions);
                audit.steps_end = feedback.steps.len();
                audit.decisions_end = feedback.decisions.len();
                result.plan_results.push(audit);
            }
            result.planning = Some(feedback);
            convo.messages.extend([response, result]);
            let decision = DecisionBinding::new(
                convo.goal_context.next_decision(),
                version.revision(),
                &convo.messages,
                None,
                false,
                0,
            )?
            .with_conversation_messages(&convo.messages)?;
            convo.goal_context.bind(decision.clone())?;
            runtime
                .writer
                .lock()
                .unwrap()
                .append(&Record::GoalContextDecision {
                    binding: decision,
                    versions: vec![],
                })?;
        }
        convo.messages.push(Message::assistant(vec![Block::text(
            "Authored final response",
        )]));
        convo.taint = Taint {
            private: true,
            untrusted: false,
        };
        live_owner.finish()?;
        let mut prepared = runtime
            .writer
            .lock()
            .unwrap()
            .prepare_run(&before, &convo)?;
        prepared.push_record(&Record::Outcome(RunStats {
            stop_cause: Some(stop),
            turns: if count > 0 { 2 } else { 1 },
            context_overflows: Some(0),
            ..Default::default()
        }))?;
        prepared.push_record(&Record::Taint(convo.taint))?;
        Ok((turn, prepared))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[test]
fn task_host_migration_is_explicit_and_startup_is_not_owner_speech() -> Result<()> {
    let f = Fixture::new()?;
    assert!(f
        .memory()?
        .attach_task_chat(&f.session, Some(Fixture::draft()), &Fixture::tools())
        .is_err());
    assert_eq!(f.memory()?.memory()?.session_schema_version()?, 8);
    assert_eq!(f.memory()?.enable_planning()?, 10);
    assert_eq!(f.memory()?.enable_planning()?, 10);
    let chat =
        f.memory()?
            .attach_task_chat(&f.session, Some(Fixture::draft()), &Fixture::tools())?;
    let bytes = std::fs::read(&f.session.path)?;
    assert!(!String::from_utf8_lossy(&bytes).contains("Private task metadata sentinel"));
    assert!(Session::load(&f.session.path).is_err());
    assert_eq!(recording::records(&bytes)?.len(), 3);
    let task = chat.task_diagnostics()?.task;
    drop(chat);
    let recovery = SessionMemory::open_for_recovery(&f.request)?;
    recovery.retry_task_start(&f.session.meta.id, Fixture::draft())?;
    assert_eq!(bytes, std::fs::read(&f.session.path)?);
    let resumed = f
        .memory()?
        .attach_task_chat(&f.session, None, &Fixture::tools())?;
    assert_eq!(resumed.task_diagnostics()?.task, task);
    drop(resumed);
    let mut wrong = Fixture::draft();
    wrong.intent.push('x');
    assert!(recovery
        .retry_task_start(&f.session.meta.id, wrong)
        .is_err());
    assert_eq!(bytes, std::fs::read(&f.session.path)?);
    Ok(())
}
#[tokio::test]
async fn task_host_zero_todo_requires_expected_sync_gate() -> Result<()> {
    let f = Fixture::new()?;
    let mut chat = f.chat()?;
    let (turn, prepared) = f.prepared(&mut chat, 0).await?;
    chat.task_runtime
        .as_ref()
        .unwrap()
        .writer
        .lock()
        .unwrap()
        .append_prepared(prepared)?;
    assert_ne!(
        chat.finish_turn(&turn).status,
        SessionMemoryStatus::Retained
    );
    assert!(!chat.memory.authority.file(&turn.id, "chat-seal")?.exists());
    let bytes = std::fs::read(&f.session.path)?;
    let id = turn.id.clone();
    drop(chat);
    let recovery = SessionMemory::open_for_recovery(&f.request)?;
    assert_eq!(
        recovery.retry_chat(&id).status,
        SessionMemoryStatus::Pending
    );
    assert_eq!(bytes, std::fs::read(&f.session.path)?);
    Ok(())
}
#[tokio::test]
async fn task_host_exact_complete_recovery_audits_and_retains_once() -> Result<()> {
    for count in [0, 2] {
        let f = Fixture::new()?;
        let mut chat = f.chat()?;
        let (turn, prepared) = f.prepared(&mut chat, count).await?;
        task_audit::prepare(&chat.memory, &turn, &chat.attachment, &prepared)?;
        chat.task_runtime
            .as_ref()
            .unwrap()
            .writer
            .lock()
            .unwrap()
            .append_prepared(prepared)?;
        let id = turn.id.clone();
        let bytes = std::fs::read(&f.session.path)?;
        drop(chat);
        let recovery = SessionMemory::open_for_recovery(&f.request)?;
        let report = recovery.retry_chat(&id);
        assert_eq!(report.status, SessionMemoryStatus::Retained, "{:?}", report);
        assert_eq!(recovery.retry_chat(&id).receipt, report.receipt);
        assert_eq!(bytes, std::fs::read(&f.session.path)?);
        let chat = f
            .memory()?
            .attach_task_chat(&f.session, None, &Fixture::tools())?;
        let s: ChatSeal = read_private(
            &chat.memory.authority.file(&id, "chat-seal")?,
            authority::SOURCE_MAX,
        )?;
        assert!(s.payload.receipts.is_empty());
        assert!(!serde_json::to_string(&s.payload)?.contains("never execute this fixture check"));
    }
    Ok(())
}
#[tokio::test]
async fn task_host_partial_native_blocks_without_reexecution() -> Result<()> {
    let f = Fixture::new()?;
    let mut chat = f.chat()?;
    let (turn, prepared) = f.prepared(&mut chat, 2).await?;
    task_audit::prepare(&chat.memory, &turn, &chat.attachment, &prepared)?;
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&f.session.path)?
        .write_all(&prepared.bytes()[..9])?;
    let id = turn.id.clone();
    let bytes = std::fs::read(&f.session.path)?;
    drop(chat);
    let recovery = SessionMemory::open_for_recovery(&f.request)?;
    assert_eq!(
        recovery.retry_chat(&id).status,
        SessionMemoryStatus::Pending
    );
    assert_eq!(bytes, std::fs::read(&f.session.path)?);
    assert!(f
        .memory()?
        .attach_task_chat(&f.session, None, &Fixture::tools())
        .is_err());
    Ok(())
}
#[tokio::test]
async fn task_host_missing_seal_hash_repairs_only_exact_canonical_seal() -> Result<()> {
    for corrupt in [false, true] {
        let f = Fixture::new()?;
        let mut chat = f.chat()?;
        let (turn, prepared) = f.prepared(&mut chat, 2).await?;
        task_audit::prepare(&chat.memory, &turn, &chat.attachment, &prepared)?;
        chat.task_runtime
            .as_ref()
            .unwrap()
            .writer
            .lock()
            .unwrap()
            .append_prepared(prepared)?;
        task_audit::recover(&chat.memory, &turn.id)?;
        let hash = chat.memory.authority.file(&turn.id, "chat-seal-hash")?;
        std::fs::remove_file(&hash)?;
        if corrupt {
            std::fs::write(chat.memory.authority.file(&turn.id, "chat-seal")?, b"{}")?;
        }
        let id = turn.id.clone();
        drop(chat);
        let report = SessionMemory::open_for_recovery(&f.request)?.retry_chat(&id);
        assert_eq!(report.status == SessionMemoryStatus::Retained, !corrupt);
        assert_eq!(hash.exists(), !corrupt);
    }
    Ok(())
}

#[test]
fn task_host_startup_interruptions_reuse_exact_original_task() -> Result<()> {
    for stage in ["claim", "reservation", "task", "attachment"] {
        let f = Fixture::new()?;
        f.memory()?.enable_planning()?;
        task_start::stop_at(stage);
        assert!(f
            .memory()?
            .attach_task_chat(&f.session, Some(Fixture::draft()), &Fixture::tools())
            .is_err());
        let reserved = std::fs::read(&f.session.path)?;
        if stage != "claim" {
            assert!(Session::load(&f.session.path).is_err());
        }
        let recovery = SessionMemory::open_for_recovery(&f.request)?;
        let mut wrong = Fixture::draft();
        wrong.title.push('x');
        assert!(recovery
            .retry_task_start(&f.session.meta.id, wrong)
            .is_err());
        assert_eq!(reserved, std::fs::read(&f.session.path)?);
        recovery.retry_task_start(&f.session.meta.id, Fixture::draft())?;
        let chat = f
            .memory()?
            .attach_task_chat(&f.session, None, &Fixture::tools())?;
        let original = chat.task_diagnostics()?.task;
        drop(chat);
        let complete = std::fs::read(&f.session.path)?;
        recovery.retry_task_start(&f.session.meta.id, Fixture::draft())?;
        assert_eq!(complete, std::fs::read(&f.session.path)?);
        let chat = f
            .memory()?
            .attach_task_chat(&f.session, None, &Fixture::tools())?;
        assert_eq!(chat.task_diagnostics()?.task, original);
    }
    Ok(())
}
#[test]
fn task_host_torn_startup_refuses_without_truncation_or_task_adoption() -> Result<()> {
    use std::io::Write;
    for stage in ["claim", "task", "attachment"] {
        let f = Fixture::new()?;
        f.memory()?.enable_planning()?;
        task_start::stop_at(stage);
        assert!(f
            .memory()?
            .attach_task_chat(&f.session, Some(Fixture::draft()), &Fixture::tools())
            .is_err());
        std::fs::OpenOptions::new()
            .append(true)
            .open(&f.session.path)?
            .write_all(b"{\"record\":\"chat_memory\",\"marker\":")?;
        let bytes = std::fs::read(&f.session.path)?;
        assert!(SessionMemory::open_for_recovery(&f.request)?
            .retry_task_start(&f.session.meta.id, Fixture::draft())
            .is_err());
        assert_eq!(bytes, std::fs::read(&f.session.path)?);
        assert!(Session::load(&f.session.path).is_err());
    }
    Ok(())
}

#[tokio::test]
async fn task_host_native_audit_requires_complete_original_occurrence_set() -> Result<()> {
    let f = Fixture::new()?;
    let mut chat = f.chat()?;
    let (turn, prepared) = f.prepared(&mut chat, 2).await?;
    task_audit::prepare(&chat.memory, &turn, &chat.attachment, &prepared)?;
    chat.task_runtime
        .as_ref()
        .unwrap()
        .writer
        .lock()
        .unwrap()
        .append_prepared(prepared)?;
    let (frozen, attachment) = frozen_metadata(&chat.memory.authority, &turn.id)?;
    let mut db = chat.memory.memory()?;
    let task = &attachment.task.as_ref().unwrap().task;
    let session = db.task_session(
        &chat.memory.request.account,
        &attachment.session_id,
        chat.memory.access(),
    )?;
    let entries = db.plan_turn_entries(&session, task, &frozen.attachment, &turn.id)?;
    let proof = crate::memory::plan::VerifiedPlanAudit::from_host_verified(
        &session,
        task,
        &frozen.attachment,
        &turn.id,
        vec![entries[0].clone()],
    )?;
    db.plan_reconcile_audit(&session, task, &proof)?;
    let entries = db.plan_turn_entries(&session, task, &frozen.attachment, &turn.id)?;
    assert!(entries[0].audited && !entries[1].audited);
    let bytes = std::fs::read(&f.session.path)?;
    task_audit::validate_native(&attachment, &frozen, &turn.id, &bytes, &entries)?;
    assert!(
        task_audit::validate_native(&attachment, &frozen, &turn.id, &bytes, &entries[1..]).is_err(),
        "omitted audited original must refuse"
    );
    for mutation in [
        "missing",
        "duplicate",
        "wrong_turn",
        "swapped_ordinal",
        "clipped",
        "wrong_revision",
        "cross_range",
        "feedback",
    ] {
        let mut records = recording::records(&bytes)?;
        for record in &mut records {
            if let Record::Message(m) = record {
                if m.plan_results.is_empty() {
                    continue;
                }
                match mutation {
                    "missing" => {
                        m.plan_results.pop();
                    }
                    "duplicate" => {
                        m.plan_results.push(m.plan_results[0].clone());
                    }
                    "wrong_turn" => m.plan_results[0].turn_id = "foreign".into(),
                    "swapped_ordinal" => {
                        m.plan_results[0].ordinal = 1;
                        m.plan_results[1].ordinal = 0;
                    }
                    "clipped" => {
                        if let Block::ToolResult { content, .. } = &mut m.content[0] {
                            content.pop();
                        }
                    }
                    "wrong_revision" => m.plan_results[0].revision_id = "foreign".into(),
                    "cross_range" => {
                        m.plan_results[0].decisions_end = 2;
                    }
                    "feedback" => {
                        m.planning.as_mut().unwrap().decisions.clear();
                    }
                    _ => unreachable!(),
                }
                break;
            }
        }
        let changed = records
            .iter()
            .map(crate::session::encode_record_line)
            .collect::<Result<Vec<_>>>()?
            .concat();
        assert!(
            task_audit::validate_native(&attachment, &frozen, &turn.id, &changed, &entries)
                .is_err(),
            "{mutation}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn task_host_complete_native_after_sync_failure_recovers_without_append() -> Result<()> {
    let f = Fixture::new()?;
    let mut chat = f.chat()?;
    let (turn, prepared) = f.prepared(&mut chat, 2).await?;
    task_audit::prepare(&chat.memory, &turn, &chat.attachment, &prepared)?;
    let mut writer = chat.task_runtime.as_ref().unwrap().writer.lock().unwrap();
    writer.append_prepared(prepared)?;
    super::super::native_writer::test_io::fail_sync(false);
    assert!(writer.sync_verified().is_err());
    drop(writer);
    let bytes = std::fs::read(&f.session.path)?;
    let id = turn.id.clone();
    drop(chat);
    assert_eq!(
        SessionMemory::open_for_recovery(&f.request)?
            .retry_chat(&id)
            .status,
        SessionMemoryStatus::Retained
    );
    assert_eq!(bytes, std::fs::read(&f.session.path)?);
    Ok(())
}

#[test]
fn task_host_turn_drop_and_finish_revoke_retained_decision_recorder() -> Result<()> {
    for mode in ["active_drop", "turn_drop", "finish"] {
        let f = Fixture::new()?;
        let mut chat = f.chat()?;
        let owner = OwnerInput::new(OwnerSource::CliChat, "Inspect authored fixture".into())?;
        let turn = chat.begin_turn(&f.session, &owner)?;
        chat.task_record_owner(&turn, &owner)?;
        assert!(chat.task_record_owner(&turn, &owner).is_err());
        let context = crate::agent::RunContext::new(
            ToolCtx::default(),
            Arc::new(crate::tool::ModeApprover {
                mode: crate::config::PermissionMode::Allow,
            }),
        );
        let (cx, active) = chat.task_context(&turn, owner.clone(), context.clone())?;
        assert!(chat.task_context(&turn, owner.clone(), context).is_err());
        let recorder = cx
            .goal_ingress
            .as_ref()
            .unwrap()
            .task_recorder
            .as_ref()
            .unwrap()
            .clone();
        let mut state = crate::goal_context::GoalContextState::default();
        let version = state
            .transition(CharterSnapshot::disabled(), Some(owner.clone()))?
            .unwrap();
        let history = vec![Message::user(owner.text())];
        let binding = DecisionBinding::new(
            state.next_decision(),
            version.revision(),
            &history,
            None,
            true,
            0,
        )?
        .with_conversation_messages(&history)?;
        recorder.record(binding.clone(), vec![version.clone()])?;
        let bytes = std::fs::read(&f.session.path)?;
        match mode {
            "active_drop" => drop(active),
            "turn_drop" => {
                drop(turn);
                assert!(
                    recorder
                        .record(binding.clone(), vec![version.clone()])
                        .is_err(),
                    "dropping the native turn must revoke even while the active guard survives"
                );
                drop(active);
            }
            "finish" => {
                assert_ne!(
                    chat.finish_turn(&turn).status,
                    SessionMemoryStatus::Retained
                );
                drop(active);
            }
            _ => unreachable!(),
        }
        assert!(recorder.record(binding, vec![version]).is_err());
        assert_eq!(bytes, std::fs::read(&f.session.path)?);
    }
    Ok(())
}

#[tokio::test]
async fn task_host_early_exit_audits_original_receipts_without_inventing_completion() -> Result<()>
{
    let f = Fixture::new()?;
    let mut chat = f.chat()?;
    let (turn, prepared) = f
        .prepared_with_stop(&mut chat, 2, StopCause::Stopped)
        .await?;
    task_audit::prepare(&chat.memory, &turn, &chat.attachment, &prepared)?;
    chat.task_runtime
        .as_ref()
        .unwrap()
        .writer
        .lock()
        .unwrap()
        .append_prepared(prepared)?;
    let id = turn.id.clone();
    let bytes = std::fs::read(&f.session.path)?;
    drop(chat);
    let recovery = SessionMemory::open_for_recovery(&f.request)?;
    assert_eq!(
        recovery.retry_chat(&id).status,
        SessionMemoryStatus::Pending
    );
    let (turn, attachment) = frozen_metadata(&recovery.authority, &id)?;
    let db = recovery.memory()?;
    let session = db.task_session(
        &recovery.request.account,
        &attachment.session_id,
        recovery.access(),
    )?;
    let entries = db.plan_turn_entries(
        &session,
        &attachment.task.as_ref().unwrap().task,
        &turn.attachment,
        &id,
    )?;
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|e| e.audited));
    assert!(!recovery.authority.file(&id, "chat-seal")?.exists());
    assert_eq!(bytes, std::fs::read(&f.session.path)?);
    assert_eq!(
        recovery.retry_chat(&id).status,
        SessionMemoryStatus::Pending
    );
    Ok(())
}

#[test]
fn task_host_existing_reservation_sync_failure_precedes_any_task_create() -> Result<()> {
    let f = Fixture::new()?;
    f.memory()?.enable_planning()?;
    task_start::stop_at("reservation");
    assert!(f
        .memory()?
        .attach_task_chat(&f.session, Some(Fixture::draft()), &Fixture::tools())
        .is_err());
    let bytes = std::fs::read(&f.session.path)?;
    let counts = || -> Result<(i64, i64)> {
        let db = rusqlite::Connection::open_with_flags(
            &f.request.db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let key = std::fs::read_to_string(&f.request.key_file)?;
        db.pragma_update(None, "key", format!("x'{}'", key.trim()))?;
        Ok(db.query_row("SELECT (SELECT count(*) FROM object WHERE kind='task'), (SELECT count(*) FROM task_operation)", [], |r| Ok((r.get(0)?, r.get(1)?)))?)
    };
    assert_eq!(counts()?, (0, 0));
    super::super::native_writer::test_io::fail_sync(false);
    let recovery = SessionMemory::open_for_recovery(&f.request)?;
    assert!(recovery
        .retry_task_start(&f.session.meta.id, Fixture::draft())
        .is_err());
    assert_eq!(
        counts()?,
        (0, 0),
        "existing reservation must sync before SQLite Task mutation"
    );
    assert_eq!(bytes, std::fs::read(&f.session.path)?);
    recovery.retry_task_start(&f.session.meta.id, Fixture::draft())?;
    assert_eq!(counts()?, (1, 1));
    Ok(())
}

#[tokio::test]
async fn task_host_complete_native_recovery_does_not_require_old_workspace() -> Result<()> {
    let f = Fixture::new()?;
    let mut chat = f.chat()?;
    let (turn, prepared) = f.prepared(&mut chat, 2).await?;
    task_audit::prepare(&chat.memory, &turn, &chat.attachment, &prepared)?;
    chat.task_runtime
        .as_ref()
        .unwrap()
        .writer
        .lock()
        .unwrap()
        .append_prepared(prepared)?;
    let id = turn.id.clone();
    let bytes = std::fs::read(&f.session.path)?;
    drop(chat);
    std::fs::remove_dir(f.root.join("work"))?;
    let recovery = SessionMemory::open_for_recovery(&f.request)?;
    let report = recovery.retry_chat(&id);
    assert_eq!(
        report.status,
        SessionMemoryStatus::Retained,
        "storage-only recovery must not open the old workspace"
    );
    assert_eq!(recovery.retry_chat(&id).receipt, report.receipt);
    assert_eq!(bytes, std::fs::read(&f.session.path)?);
    Ok(())
}

#[tokio::test]
async fn task_host_historical_workspace_comparison_does_not_admit_foreign_config() -> Result<()> {
    let f = Fixture::new()?;
    let mut chat = f.chat()?;
    let (turn, prepared) = f.prepared(&mut chat, 0).await?;
    task_audit::prepare(&chat.memory, &turn, &chat.attachment, &prepared)?;
    chat.task_runtime
        .as_ref()
        .unwrap()
        .writer
        .lock()
        .unwrap()
        .append_prepared(prepared)?;
    let bytes = std::fs::read(&f.session.path)?;
    let (frozen_turn, attachment) = frozen_metadata(&chat.memory.authority, &turn.id)?;
    // The new door cannot be selected by a live host, even with valid bytes.
    assert!(capture::capture_task_recovery(
        &chat.memory,
        &frozen_turn,
        &attachment,
        &bytes,
        &[],
        &turn.id
    )
    .is_err());
    drop(chat);
    std::fs::remove_dir(f.root.join("work"))?;
    let recovery = SessionMemory::open_for_recovery(&f.request)?;
    capture::capture_task_recovery(&recovery, &frozen_turn, &attachment, &bytes, &[], &turn.id)?;
    // Same authenticated history, but the saved Config names a foreign path.
    // Recompute only the test's frozen prefix to isolate the historical-path
    // comparison; the production fingerprint gate separately rejects edits.
    let mut records = recording::records(&bytes)?;
    for record in &mut records {
        if let Record::Config(config) = record {
            config.workspace = f.root.join("evil");
        }
    }
    let changed = records
        .iter()
        .map(crate::session::encode_record_line)
        .collect::<Result<Vec<_>>>()?
        .concat();
    assert_eq!(changed.len(), bytes.len());
    let mut changed_turn = frozen_turn.clone();
    changed_turn.start_digest = authority::digest(&changed[..changed_turn.start_length as usize]);
    let error = capture::capture_task_recovery(
        &recovery,
        &changed_turn,
        &attachment,
        &changed,
        &[],
        &turn.id,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("task historical workspace changed"),
        "{error:#}"
    );
    assert!(
        capture::capture(&recovery, &frozen_turn, &attachment, &bytes, &[], &turn.id).is_err(),
        "legacy/live capture still requires canonicalization"
    );
    Ok(())
}
