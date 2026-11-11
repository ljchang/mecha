//! Concrete host capability. No key, opener or execution permit enters a DTO.
use super::*;
use crate::memory::{
    plan::{PlanCommitRequest, PlanCommitResult, PlanTurnPermit, PlanView},
    TaskRef, TaskSession,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Weak,
};

pub(crate) struct PlanBinding {
    memory: Arc<SessionMemory>,
    lease: Weak<Lease>,
    attachment: String,
    session: TaskSession,
    task: TaskRef,
    session_id: String,
}
impl PlanBinding {
    pub(super) fn from_host(chat: &ChatMemory, task: TaskRef) -> Result<Arc<Self>> {
        chat.lease.validate()?;
        let (a, digest) = attachment(&chat.memory.authority, &chat.attachment.session_id)?;
        ensure!(
            digest == chat.digest
                && a.protocol == CHAT_TASK_MEMORY_PROTOCOL
                && a.task.as_ref().is_some_and(|bound| bound.task == task),
            "task attachment changed or wrong TaskRef"
        );
        let db = chat.memory.memory()?;
        let session = db.task_session(
            &chat.memory.request.account,
            &a.session_id,
            chat.memory.access(),
        )?;
        db.task_read(&session, &task)?;
        Ok(Arc::new(Self {
            memory: Arc::clone(&chat.memory),
            lease: Arc::downgrade(&chat.lease),
            attachment: digest,
            session,
            task,
            session_id: a.session_id,
        }))
    }
    fn authenticate(&self) -> Result<Arc<Lease>> {
        let lease = self
            .lease
            .upgrade()
            .ok_or_else(|| anyhow::anyhow!("task lease ended"))?;
        lease.validate()?;
        self.memory.execution_workspace()?;
        let (_, digest) = attachment(&self.memory.authority, &self.session_id)?;
        ensure!(digest == self.attachment, "task attachment changed");
        Ok(lease)
    }
    pub(crate) fn read(&self) -> Result<Option<PlanView>> {
        let _lease = self.authenticate()?;
        self.memory.memory()?.plan_read(&self.session, &self.task)
    }
}

pub struct LivePlanTurn {
    binding: Arc<PlanBinding>,
    permit: PlanTurnPermit,
    turn_id: String,
    finished: AtomicBool,
    occurrences: Mutex<(u32, u32)>,
}
impl std::fmt::Debug for LivePlanTurn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LivePlanTurn(<host capability>)")
    }
}
pub(super) struct PlanTurnOwner(Arc<LivePlanTurn>);
impl PlanTurnOwner {
    pub(super) fn finish(&self) -> Result<()> {
        self.0.finish()
    }
}
impl Drop for PlanTurnOwner {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
impl LivePlanTurn {
    pub(crate) fn finish(&self) -> Result<()> {
        self.finished.store(true, Ordering::Release);
        self.permit.revoke()
    }
    pub(crate) fn validate_live(&self) -> Result<()> {
        let _lease = self.binding.authenticate()?;
        self.binding.read()?;
        ensure!(!self.finished.load(Ordering::Acquire), "task turn ended");
        frozen(&self.binding.memory.authority, &self.turn_id)?;
        Ok(())
    }
    pub(super) fn from_host(
        binding: Arc<PlanBinding>,
        turn: &ChatMemoryTurn,
    ) -> Result<(Arc<Self>, PlanTurnOwner)> {
        let _lease = binding.authenticate()?;
        let (f, _) = frozen(&binding.memory.authority, &turn.id)?;
        ensure!(
            turn.attachment == binding.attachment
                && turn.session_id == binding.session_id
                && turn.authority == binding.memory.authority.identity
                && f.attachment == binding.attachment
                && !turn
                    .state
                    .lock()
                    .map_err(|_| anyhow::anyhow!("turn state poisoned"))?
                    .sealed,
            "task turn changed or finished"
        );
        let permit = PlanTurnPermit::from_host(
            &binding.session,
            &binding.task,
            &binding.attachment,
            &turn.id,
        )?;
        let live = Arc::new(Self {
            binding,
            permit,
            turn_id: turn.id.clone(),
            finished: AtomicBool::new(false),
            occurrences: Mutex::new((0, 0)),
        });
        Ok((Arc::clone(&live), PlanTurnOwner(live)))
    }
    pub(crate) fn matches(&self, binding: &Arc<PlanBinding>) -> bool {
        Arc::ptr_eq(&self.binding, binding)
    }
    pub(crate) fn reserve_batch(&self, count: usize) -> Result<(u32, u32)> {
        ensure!(!self.finished.load(Ordering::Acquire), "task turn finished");
        let mut positions = self
            .occurrences
            .lock()
            .map_err(|_| anyhow::anyhow!("task occurrence poisoned"))?;
        let count = u32::try_from(count)?;
        ensure!(
            positions.0.checked_add(count).is_some_and(|n| n <= 256),
            "task occurrence limit"
        );
        let result = *positions;
        positions.0 += count;
        positions.1 += 1;
        Ok(result)
    }
    pub(crate) fn turn_id(&self) -> &str {
        &self.turn_id
    }
    pub(crate) fn commit(
        &self,
        expected: Option<&RevisionRef>,
        request: PlanCommitRequest,
    ) -> Result<PlanCommitResult> {
        let _lease = self.binding.authenticate()?;
        ensure!(!self.finished.load(Ordering::Acquire), "task turn finished");
        let (f, _) = frozen(&self.binding.memory.authority, &self.turn_id)?;
        ensure!(
            f.attachment == self.binding.attachment
                && request.occurrence.turn_id == self.turn_id
                && !self
                    .binding
                    .memory
                    .authority
                    .file(&self.turn_id, "withdrawn")?
                    .try_exists()?
                && !self
                    .binding
                    .memory
                    .authority
                    .file(&self.turn_id, "chat-seal")?
                    .try_exists()?,
            "task turn unavailable"
        );
        self.binding.memory.memory()?.plan_commit(
            &self.binding.session,
            &self.permit,
            &self.binding.task,
            expected,
            request,
        )
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        goal_context::{CharterSnapshot, DecisionBinding, GoalContextState},
        session::{RunConfig, SessionMeta},
        tool::{
            todo::{memory::PlanCall, TodoTool},
            GoalTrack, StepCounts, Tool, ToolCtx,
        },
    };
    use serde_json::json;
    pub(crate) struct Fixture {
        root: PathBuf,
        pub(crate) chat: ChatMemory,
        pub(crate) live: Arc<LivePlanTurn>,
        owner: PlanTurnOwner,
        pub(crate) tool: Arc<TodoTool>,
        pub(crate) ctx: ToolCtx,
    }
    impl Fixture {
        pub(crate) fn new() -> Result<Self> {
            let root = std::env::temp_dir()
                .canonicalize()?
                .join(format!("durable-todo-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root)?;
            for path in ["keys", "db", "work", "sessions"] {
                crate::create_private_dir(&root.join(path))?;
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
            let memory = SessionMemory::open(&request, &root.join("work"))?;
            let mut db = memory.memory()?;
            db.enable_session_tasks()?;
            db.enable_session_plans()?;
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
            let tools = vec!["fs_read".into(), "goal_context".into(), "todo".into()];
            let mut chat = memory.attach_task_chat(
                &session,
                Some(crate::memory::TaskDraft {
                    title: "Authored task".into(),
                    intent: "Inspect the synthetic fixture".into(),
                }),
                &tools,
            )?;
            let task = chat.attachment.task.as_ref().unwrap().task.clone();
            chat.task_record_config(RunConfig {
                workspace: root.join("work"),
                tools,
                ..Default::default()
            })?;
            let owner_input =
                OwnerInput::new(OwnerSource::CliChat, "Inspect the synthetic fixture".into())?;
            let turn = chat.begin_turn(&session, &owner_input)?;
            let binding = PlanBinding::from_host(&chat, task)?;
            let tool = Arc::new(TodoTool::with_memory(Arc::clone(&binding))?);
            let (live, owner) = LivePlanTurn::from_host(binding, &turn)?;
            let mut context = GoalContextState::default();
            let version = context
                .transition(CharterSnapshot::disabled(), Some(owner_input))?
                .unwrap();
            let decision = DecisionBinding::new(
                0,
                version.revision(),
                &[crate::message::Message::user(
                    "Inspect the synthetic fixture",
                )],
                None,
                true,
                0,
            )?;
            let stamp = crate::planning::GoalDecisionStamp::capture(&decision, &version)?;
            let ctx = ToolCtx {
                workspace: root.join("work"),
                conversation: Some(session.meta.id.clone()),
                task_plan: Some(Arc::clone(&live)),
                goal_decision: Arc::new(Mutex::new(Some(stamp))),
                goal_context_version: Arc::new(Mutex::new(Some(version))),
                goal_track: Some(Arc::new(GoalTrack::carrying(Some(
                    "task:synthetic".parse()?,
                )))),
                step_counts: Some(Arc::new(StepCounts::default())),
                work: Some(crate::step::Work::default().in_run(1)),
                ..Default::default()
            };
            Ok(Self {
                root,
                chat,
                live,
                owner,
                tool,
                ctx,
            })
        }
        pub(crate) fn stop(&self) -> Result<()> {
            self.owner.finish()
        }
        pub(crate) fn call_ctx(&self, ordinal: u32, batch: u32, cap: usize) -> ToolCtx {
            let goal = self.ctx.goal_decision.lock().unwrap().clone().unwrap();
            ToolCtx {
                call_id: Some("synthetic-call".into()),
                plan_call: Some(Arc::new(PlanCall::new(
                    Arc::clone(&self.live),
                    ordinal,
                    batch,
                    goal,
                    self.ctx.goal_track.as_ref().unwrap().anchor(),
                    cap,
                ))),
                ..self.ctx.clone()
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    fn plan(status: &str, check: &str) -> serde_json::Value {
        json!({"items":[{"content":"Inspect fixture","status":status,"check":check}],"serves":"task:synthetic"})
    }
    #[tokio::test]
    async fn durable_todo_commit_retry_and_failed_second_preserve_published_effects() -> Result<()>
    {
        let f = Fixture::new()?;
        let first = f.call_ctx(0, 0, 24000);
        let out = f
            .tool
            .call(plan("in_progress", "inspect fixture"), &first)
            .await?;
        assert!(!out.is_error, "{}", out.content);
        let audit = first.plan_call.as_ref().unwrap().take().unwrap();
        assert_eq!(audit.1.decisions.len(), 1);
        let before_counts = f.ctx.step_counts.as_ref().unwrap().snapshot();
        let before_goal = f.ctx.goal_track.as_ref().unwrap().snapshot();
        let before = f.live.binding.read()?.unwrap();
        assert_eq!(
            before.value.request.input_json,
            serde_json::to_string(&plan("in_progress", "inspect fixture"))?
        );
        assert!(!before.value.request.snapshot.declarations["Inspect fixture"].frozen);
        let replay = f
            .tool
            .call(plan("in_progress", "inspect fixture"), &first)
            .await?;
        assert_eq!(replay.content, out.content);
        assert!(first.plan_call.as_ref().unwrap().take().is_none());
        assert_eq!(f.ctx.goal_track.as_ref().unwrap().snapshot(), before_goal);
        // A real storage authority failure after a successful write.
        let mut db = f.chat.memory.memory()?;
        db.task_set_status(
            &f.live.binding.session,
            &crate::memory::TaskOperationId::new("close-fixture")?,
            &f.live.binding.task,
            crate::memory::TaskStatus::Closed,
        )?;
        let key =
            crate::planning::verification_key(Some(&"task:synthetic".parse()?), "Inspect fixture");
        f.ctx.verified_steps.lock().unwrap().insert(key.clone());
        let second = f.call_ctx(1, 0, 24000);
        let failed = f
            .tool
            .call(plan("completed", "replacement"), &second)
            .await?;
        assert!(failed.is_error);
        assert_eq!(
            f.ctx.step_counts.as_ref().unwrap().snapshot(),
            before_counts
        );
        assert_eq!(f.ctx.goal_track.as_ref().unwrap().snapshot(), before_goal);
        assert!(f.ctx.verified_steps.lock().unwrap().contains(&key));
        assert!(second.plan_call.as_ref().unwrap().take().is_none());
        assert_eq!(
            f.tool.items_in(&crate::tool::todo::PlanKey::of(&f.ctx))[0].status,
            crate::tool::todo::Status::InProgress
        );
        assert_eq!(
            f.tool.tampered_in(&crate::tool::todo::PlanKey::of(&f.ctx)),
            0
        );
        Ok(())
    }
    #[tokio::test]
    async fn durable_todo_rejected_then_success_skips_checks_and_preserves_raw_input() -> Result<()>
    {
        let f = Fixture::new()?;
        assert!(
            f.tool
                .call(json!({"items":"invalid"}), &f.call_ctx(0, 0, 24000))
                .await?
                .is_error
        );
        let first = f.call_ctx(1, 0, 24000);
        assert!(
            !f.tool
                .call(plan("in_progress", "inspect fixture"), &first)
                .await?
                .is_error
        );
        let second = f.call_ctx(2, 1, 24000);
        let raw = plan("completed", "replacement");
        let out = f.tool.call(raw.clone(), &second).await?;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("completion remains unverified"));
        assert!(!out.content.contains("replacement"));
        let view = f.live.binding.read()?.unwrap();
        assert_eq!(view.value.request.input_json, serde_json::to_string(&raw)?);
        assert_eq!(
            view.value.request.snapshot.items[0].check.as_deref(),
            Some("inspect fixture")
        );
        assert!(view.value.request.snapshot.declarations["Inspect fixture"].frozen);
        let (_, feedback) = second.plan_call.as_ref().unwrap().take().unwrap();
        assert!(feedback.steps.iter().all(|s| matches!(
            s.verification,
            crate::planning::Verification::Skipped | crate::planning::Verification::NotDeclared
        )));
        assert!(f.ctx.step_checks.is_none() && f.ctx.step_escalation.is_none());
        let restored = TodoTool::with_memory(Arc::clone(&f.live.binding))?;
        assert!(!restored.read_only());
        assert!(restored.capabilities().private_data);
        assert_eq!(
            restored.effect(),
            Some(crate::tool::effect::Effect::StoreWrite)
        );
        assert!(restored
            .rehydrate(&crate::tool::todo::PlanKey::of(&f.ctx), &[])
            .is_none());
        let third = f.call_ctx(3, 2, 24000);
        let reopened = restored
            .call(plan("in_progress", "replacement"), &third)
            .await?;
        assert!(!reopened.is_error, "{}", reopened.content);
        assert_eq!(
            f.live.binding.read()?.unwrap().value.request.snapshot.items[0]
                .check
                .as_deref(),
            Some("inspect fixture")
        );
        Ok(())
    }
    #[tokio::test]
    async fn durable_todo_cap_and_revocation_refuse_before_store_change() -> Result<()> {
        let f = Fixture::new()?;
        assert!(
            f.tool
                .call(plan("pending", "inspect fixture"), &f.call_ctx(0, 0, 1))
                .await?
                .is_error
        );
        assert!(f.live.binding.read()?.is_none());
        f.owner.finish()?;
        assert!(
            f.tool
                .call(plan("pending", "inspect fixture"), &f.call_ctx(1, 1, 24000))
                .await?
                .is_error
        );
        assert!(f.live.binding.read()?.is_none());
        assert_eq!(f.ctx.step_counts.as_ref().unwrap().snapshot(), (0, 0, 0, 0));
        assert_eq!(f.ctx.goal_track.as_ref().unwrap().snapshot().1, 0);
        Ok(())
    }
    #[tokio::test]
    async fn durable_todo_capability_rejects_wrong_binding_and_replaced_lease() -> Result<()> {
        let f = Fixture::new()?;
        let other = Fixture::new()?;
        assert!(
            f.tool
                .call(
                    plan("pending", "inspect fixture"),
                    &other.call_ctx(0, 0, 24000)
                )
                .await?
                .is_error
        );
        assert!(f.live.binding.read()?.is_none());
        let lease_path = f.chat.lease.path.clone();
        std::fs::rename(&lease_path, lease_path.with_extension("old"))?;
        authority::write(&lease_path, b"replacement")?;
        assert!(
            f.tool
                .call(plan("pending", "inspect fixture"), &f.call_ctx(0, 0, 24000))
                .await?
                .is_error
        );
        assert_eq!(f.ctx.goal_track.as_ref().unwrap().snapshot().1, 0);
        let live = Arc::clone(&other.live);
        drop(other);
        assert!(
            live.binding.lease.upgrade().is_none(),
            "controller cannot retain the lease"
        );
        assert!(live.binding.read().is_err());
        Ok(())
    }
}
