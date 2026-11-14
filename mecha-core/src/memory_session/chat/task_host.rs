//! Live task host. The CLI receives operations, never the native descriptor.
use super::super::native_writer::BoundNativeSession;
use super::*;
use crate::{
    agent::{Conversation, RunContext, RunOutcome},
    memory::{TaskStatus, TaskView},
    message::Message,
    tool::todo::TodoTool,
};

pub(super) struct TaskRuntime {
    pub(super) binding: Arc<task_plan::PlanBinding>,
    pub(super) todo: Arc<TodoTool>,
    pub(super) writer: Arc<Mutex<BoundNativeSession>>,
}
pub struct TaskChatTurn {
    owner: task_plan::PlanTurnOwner,
    recorder: Arc<TaskDecisionRecorder>,
}
pub(crate) struct TaskDecisionRecorder {
    writer: Arc<Mutex<BoundNativeSession>>,
    live: Arc<task_plan::LivePlanTurn>,
}
impl TaskDecisionRecorder {
    pub(crate) fn record(
        &self,
        binding: crate::goal_context::DecisionBinding,
        versions: Vec<crate::goal_context::ContextVersion>,
    ) -> Result<()> {
        self.live.validate_live()?;
        self.writer
            .lock()
            .map_err(|_| anyhow::anyhow!("task writer poisoned"))?
            .append(&Record::GoalContextDecision { binding, versions })
    }
}
impl ChatMemory {
    pub fn is_task_chat(&self) -> bool {
        self.attachment.task.is_some()
    }
    pub fn task_todo(&self) -> Result<Arc<TodoTool>> {
        self.lease.validate()?;
        Ok(Arc::clone(
            &self
                .task_runtime
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("task runtime absent"))?
                .todo,
        ))
    }
    pub fn task_diagnostics(&self) -> Result<TaskView> {
        self.lease.validate()?;
        let task = &self
            .attachment
            .task
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("task attachment absent"))?
            .task;
        let db = self.memory.memory()?;
        let session = db.task_session(
            &self.memory.request.account,
            &self.attachment.session_id,
            self.memory.access(),
        )?;
        let view = db.task_read(&session, task)?;
        ensure!(
            !matches!(
                view.value.status,
                TaskStatus::Closed | TaskStatus::Cancelled
            ),
            "task is terminal"
        );
        let pending = db.plan_pending(&session, task)?;
        if !pending.is_empty() {
            anyhow::bail!("task {} has committed plan pending native audit for turn {}; use memory-session retry", self.attachment.session_id, pending[0].value.request.occurrence.turn_id);
        }
        Ok(view)
    }
    pub fn task_plan_summary(&self) -> Result<Option<String>> {
        let runtime = self
            .task_runtime
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("task runtime absent"))?;
        runtime
            .binding
            .read()?
            .map(|view| serde_json::to_string(&view.value.request.snapshot).map_err(Into::into))
            .transpose()
    }
    pub fn task_conversation(&self) -> Result<(Session, Conversation)> {
        self.task_diagnostics()?;
        let (bytes, _) = self.continuation()?;
        let transcript = Session::parse(&self.attachment.source, std::str::from_utf8(&bytes)?)?;
        Ok((
            Session {
                meta: transcript.meta,
                path: self.attachment.source.clone(),
            },
            transcript.convo,
        ))
    }
    pub fn task_record_config(&self, config: crate::session::RunConfig) -> Result<()> {
        self.task_diagnostics()?;
        self.continuation()?;
        self.attachment.validate_tools(&config.tools)?;
        self.task_runtime
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("task runtime absent"))?
            .writer
            .lock()
            .map_err(|_| anyhow::anyhow!("task writer poisoned"))?
            .append(&Record::Config(config))
    }
    pub fn task_record_owner(&self, turn: &ChatMemoryTurn, owner: &OwnerInput) -> Result<()> {
        self.task_diagnostics()?;
        let (f, _) = frozen(&self.memory.authority, &turn.id)?;
        ensure!(
            f.attachment == self.digest
                && f.owner == *owner
                && authority::read(&self.attachment.source, authority::SOURCE_MAX)?.len() as u64
                    == f.start_length,
            "task owner changed or already recorded"
        );
        self.task_runtime
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("task runtime absent"))?
            .writer
            .lock()
            .map_err(|_| anyhow::anyhow!("task writer poisoned"))?
            .append(&Record::Message(Message::user(owner.text())))
    }
    pub fn task_context(
        &self,
        turn: &ChatMemoryTurn,
        owner: OwnerInput,
        context: RunContext,
    ) -> Result<(RunContext, TaskChatTurn)> {
        let runtime = self
            .task_runtime
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("task runtime absent"))?;
        let mut state = turn
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("task turn poisoned"))?;
        ensure!(
            !state.task_started && !state.sealed,
            "task execution already admitted"
        );
        state.task_started = true;
        drop(state);
        let (live, turn_owner) =
            task_plan::LivePlanTurn::from_host(Arc::clone(&runtime.binding), turn)?;
        turn.state
            .lock()
            .map_err(|_| anyhow::anyhow!("task turn poisoned"))?
            .task_live = Some(Arc::downgrade(&live));
        let recorder = Arc::new(TaskDecisionRecorder {
            writer: Arc::clone(&runtime.writer),
            live: Arc::clone(&live),
        });
        let mut context = context.with_goal_ingress(vec![owner], None);
        context
            .goal_ingress
            .as_mut()
            .expect("set ingress")
            .task_recorder = Some(Arc::clone(&recorder));
        let mut tools = context.tools.as_ref().clone();
        tools.task_plan = Some(live);
        context.tools = Arc::new(tools);
        Ok((
            context,
            TaskChatTurn {
                owner: turn_owner,
                recorder,
            },
        ))
    }
    /// Await Agent and its publication before calling; errors never lead to finish_turn.
    pub fn task_record_final(
        &self,
        turn: &ChatMemoryTurn,
        active: TaskChatTurn,
        before: &[Message],
        convo: &Conversation,
        outcome: Option<&RunOutcome>,
    ) -> Result<()> {
        active.owner.finish()?;
        self.lease.validate()?;
        self.task_runtime
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("task runtime absent"))?
            .binding
            .read()?;
        let mut writer = active
            .recorder
            .writer
            .lock()
            .map_err(|_| anyhow::anyhow!("task writer poisoned"))?;
        let mut prepared = writer.prepare_run(before, convo)?;
        if let Some(outcome) = outcome {
            prepared.push_record(&crate::session::outcome_record(outcome))?;
        }
        prepared.push_record(&Record::Taint(convo.taint))?;
        task_audit::prepare(&self.memory, turn, &self.attachment, &prepared)?;
        writer.append_prepared(prepared)?;
        let synced = writer.sync_verified()?;
        task_audit::reconcile(&self.memory, &turn.id, &synced)?;
        Ok(())
    }
}
