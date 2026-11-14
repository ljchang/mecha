//! Opted-in authenticated text chat. Each conversation owns its runtime and lease.
use super::chat::{self, ChatSlot, ChatState, Entry, WireEvent};
use crate::{commands::memory_profile, setup, GlobalOpts};
use anyhow::{Context, Result};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use mecha_core::{
    agent::{CancelHandle, CancelReason, Conversation},
    config::Config,
    goal_context::{OwnerInput, OwnerSource},
    memory_session::{
        chat::{ChatAttachMode, ChatMemory, ChatMemoryReport},
        SessionMemory, SessionMemoryStatus,
    },
    message::Message,
    session::{Record, RunConfig, Session, SessionKind, SessionMeta},
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{broadcast, oneshot};
use tokio_util::sync::CancellationToken;

#[cfg(test)]
#[derive(Default)]
pub(super) struct TestGate {
    pub(super) reached: tokio::sync::Notify,
    pub(super) release: tokio::sync::Notify,
}
#[cfg(test)]
impl TestGate {
    async fn pause(&self) {
        self.reached.notify_one();
        self.release.notified().await;
    }
}
#[cfg(test)]
#[derive(Default)]
pub(super) struct TestHooks {
    pub(super) resume_read: Option<Arc<TestGate>>,
    pub(super) reservation: Option<Arc<TestGate>>,
}

pub(super) struct MemoryHostProfile {
    path: PathBuf,
    opts: GlobalOpts,
}
impl MemoryHostProfile {
    pub(super) fn from_args(args: &super::Args, dynamic_goals: bool) -> Result<Option<Arc<Self>>> {
        let Some(path) = &args.episodic_memory else {
            return Ok(None);
        };
        anyhow::ensure!(
            dynamic_goals && SessionKind::test_override().is_none(),
            "web memory requires dynamic goals and no session-kind override"
        );
        let config = Config::load_global()?;
        let provider = args
            .memory_provider
            .clone()
            .context("web memory requires --memory-provider")?;
        let (_, configured) = config.provider(Some(&provider))?;
        let model = args
            .memory_model
            .clone()
            .or_else(|| configured.model.clone())
            .context("web memory requires --memory-model or a configured provider model")?;
        let opts = GlobalOpts {
            provider: Some(provider),
            model: Some(model),
            tools: args.memory_tools.clone(),
            dynamic_goals: true,
            global_config_only: true,
            surface: Some(SessionKind::Web),
            no_mcp: true,
            no_code_tool: true,
            no_grants: true,
            no_hooks: true,
            no_outbox: true,
            no_messages: true,
            no_fallback: true,
            no_skills: true,
            no_rules: true,
            no_learned_rules: true,
            no_compact_tool: true,
            no_situation_brief: true,
            no_past_appraisals: true,
            no_success_examples: true,
            ..GlobalOpts::default()
        };
        anyhow::ensure!(
            opts.tools.iter().any(|name| name == "goal_context")
                && opts.tools.iter().all(|name| matches!(
                    name.as_str(),
                    "goal_context" | "fs_read" | "fs_list" | "fs_write"
                ))
                && opts
                    .tools
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    == opts.tools.len(),
            "web memory requires unique explicit local tools including goal_context"
        );
        memory_profile::validate_memory_config(&opts, &config)?;
        let path = std::path::absolute(path)?;
        SessionMemory::load_request(&path)?;
        Ok(Some(Arc::new(Self { path, opts })))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(super) struct MemoryReportWire {
    protocol: String,
    conversation_id: String,
    turn_ordinal: u64,
    run_id: String,
    status: SessionMemoryStatus,
    semantic_assessment: &'static str,
    receipt_id: Option<String>,
    reason: Option<mecha_core::memory_session::SessionMemoryReason>,
}
impl From<ChatMemoryReport> for MemoryReportWire {
    fn from(report: ChatMemoryReport) -> Self {
        Self {
            protocol: report.protocol.into(),
            conversation_id: report.conversation_id,
            turn_ordinal: report.turn_ordinal,
            run_id: report.run_id,
            status: report.status,
            semantic_assessment: "not_evaluated",
            receipt_id: report.receipt.map(|r| r.id),
            reason: report.reason,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Phase {
    Ready,
    Starting,
    Running,
    Blocked,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MemorySnapshot {
    mode: &'static str,
    phase: Phase,
    next_turn_ordinal: Option<u64>,
    blocked_reason: Option<&'static str>,
    last_report: Option<MemoryReportWire>,
    limitations: Limitations,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
struct Limitations {
    typed_text_only: bool,
    fixed_profile: bool,
}

struct OwnedMemoryRuntime {
    prepared: setup::Prepared,
    memory: ChatMemory,
    session: Arc<Session>,
    conversation: Conversation,
    opts: GlobalOpts,
}
impl OwnedMemoryRuntime {
    fn entries(&self) -> Vec<Entry> {
        chat::transcript_entries_with(&self.conversation.messages, &|name| {
            self.prepared
                .agent
                .registry()
                .get(name)
                .is_some_and(|tool| tool.defers())
        })
    }
}

#[derive(Clone)]
struct RunControl {
    cancel: CancelHandle,
    stop: CancellationToken,
}
impl RunControl {
    fn new() -> Self {
        Self {
            cancel: CancelHandle::new(),
            stop: CancellationToken::new(),
        }
    }
    fn cancel(&self, reason: CancelReason) {
        self.cancel.cancel(reason);
        self.stop.cancel();
    }
}
enum Execution {
    Ready(Box<OwnedMemoryRuntime>),
    Starting(RunControl),
    Running(RunControl),
    Blocked,
}
pub(super) struct MemorySession {
    generation: String,
    pub(super) id: String,
    model: String,
    permission_mode: String,
    workspace: PathBuf,
    pub(super) events: broadcast::Sender<WireEvent>,
    execution: Execution,
    next_turn: Option<u64>,
    last_report: Option<MemoryReportWire>,
    blocked_reason: Option<&'static str>,
    entries: Vec<Entry>,
    taint: mecha_core::agent::Taint,
}
impl MemorySession {
    fn snapshot(&self) -> MemorySnapshot {
        MemorySnapshot {
            mode: "episodic",
            phase: match self.execution {
                Execution::Ready(_) => Phase::Ready,
                Execution::Starting(_) => Phase::Starting,
                Execution::Running(_) => Phase::Running,
                Execution::Blocked => Phase::Blocked,
            },
            next_turn_ordinal: self.next_turn,
            blocked_reason: self.blocked_reason,
            last_report: self.last_report.clone(),
            limitations: Limitations {
                typed_text_only: true,
                fixed_profile: true,
            },
        }
    }
    pub(super) fn response(&self, key: &str) -> serde_json::Value {
        let busy = matches!(
            self.execution,
            Execution::Starting(_) | Execution::Running(_)
        );
        serde_json::json!({"key":key, "session":self.id, "model":self.model,
            "incognito":false,"mode":self.permission_mode,"running":busy,"held_by_run":busy,
            "entries":self.entries,"taint":self.taint,"usage":null,"questions":[],"grants":[],
            "todo":[],"queue":[],"waiting":[],"memory":self.snapshot()})
    }
    pub(super) fn row(&self, key: &str) -> serde_json::Value {
        serde_json::json!({"key":key,"id":self.id,"session":self.id,"title":"Memory chat","memory":"episodic",
            "running":matches!(self.execution,Execution::Starting(_) | Execution::Running(_)),"incognito":false})
    }
    pub(super) fn stop(&self, reason: CancelReason) -> bool {
        match &self.execution {
            Execution::Starting(control) | Execution::Running(control) => {
                control.cancel(reason);
                true
            }
            _ => false,
        }
    }
    fn notify(&self) {
        let _ = self.events.send(WireEvent::Memory {
            memory: self.snapshot(),
        });
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OpenRequest {
    memory: MemoryChoice,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum MemoryChoice {
    Episodic,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResumeRequest {
    id: String,
    memory: MemoryChoice,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SendRequest {
    text: String,
    request_id: String,
    memory_turn: u64,
}

#[allow(clippy::result_large_err)]
pub(super) fn decode<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, Response> {
    serde_json::from_slice(body).map_err(|error| {
        refusal(
            if error.is_data() {
                StatusCode::UNPROCESSABLE_ENTITY
            } else {
                StatusCode::BAD_REQUEST
            },
            "invalid_request",
            "Invalid memory request",
        )
    })
}
pub(super) fn refusal(status: StatusCode, code: &'static str, message: &'static str) -> Response {
    (
        status,
        Json(serde_json::json!({"error":{"code":code,"message":message}})),
    )
        .into_response()
}
pub(super) fn unsupported() -> Response {
    refusal(
        StatusCode::CONFLICT,
        "memory_action_unsupported",
        "This action is unavailable in a memory chat",
    )
}
pub(super) async fn is_memory(chat: &ChatState, key: &str) -> bool {
    matches!(
        chat.sessions.lock().await.get(key),
        Some(ChatSlot::Memory(_))
    )
}
pub(super) async fn transcript(chat: &ChatState, key: &str) -> Option<Response> {
    let sessions = chat.sessions.lock().await;
    match sessions.get(key) {
        Some(ChatSlot::Memory(memory)) => Some(Json(memory.response(key)).into_response()),
        _ => None,
    }
}
pub(super) async fn events(chat: &ChatState, key: &str) -> Option<Response> {
    let sessions = chat.sessions.lock().await;
    match sessions.get(key) {
        Some(ChatSlot::Memory(memory)) => {
            Some(chat::sse(memory.events.subscribe(), chat.stopping.clone()))
        }
        _ => None,
    }
}
pub(super) async fn cancel(chat: &ChatState, key: &str, body: &[u8]) -> Option<Response> {
    let sessions = chat.sessions.lock().await;
    let Some(ChatSlot::Memory(memory)) = sessions.get(key) else {
        return None;
    };
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WholeRun {}
    if !body.is_empty() && decode::<WholeRun>(body).is_err() {
        return Some(unsupported());
    }
    Some(Json(serde_json::json!({"cancelled":memory.stop(CancelReason::Stopped)})).into_response())
}

pub(super) async fn open(
    chat: Arc<ChatState>,
    key: String,
    body: &[u8],
    _: super::VerifiedMutation,
) -> Response {
    let request = match decode::<OpenRequest>(body) {
        Ok(request) => request,
        Err(error) => return error,
    };
    let _ = request.memory;
    start(chat, key, None).await
}
pub(super) async fn resume(
    chat: Arc<ChatState>,
    body: &[u8],
    _: super::VerifiedMutation,
) -> Response {
    let request = match decode::<ResumeRequest>(body) {
        Ok(request) => request,
        Err(error) => return error,
    };
    let _ = request.memory;
    if request.id.is_empty() || request.id.len() > 128 {
        return refusal(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Invalid session ID",
        );
    }
    {
        let sessions = chat.sessions.lock().await;
        if let Some((key, ChatSlot::Memory(memory))) = sessions.iter().find(|(_,slot)|
            matches!(slot,ChatSlot::Memory(memory) if memory.id==request.id && !matches!(memory.execution,Execution::Blocked))) {
            return Json(memory.response(key)).into_response();
        }
    }
    let key = {
        let sessions = chat.sessions.lock().await;
        sessions
            .iter()
            .find_map(|(key, slot)| {
                matches!(slot,ChatSlot::Memory(memory) if memory.id==request.id)
                    .then(|| key.clone())
            })
            .unwrap_or_else(|| chat::resume_key(&request.id))
    };
    start(chat, key, Some(request.id)).await
}
async fn start(chat: Arc<ChatState>, key: String, resume: Option<String>) -> Response {
    let Some(profile) = chat.memory_profile.clone() else {
        return refusal(
            StatusCode::SERVICE_UNAVAILABLE,
            "memory_unavailable",
            "The memory chat profile is unavailable",
        );
    };
    if !chat::valid_key(&key)
        || super::incognito::is_incognito_key(&key)
        || super::persona_chat::is_persona_key(&key)
        || key.starts_with("t-")
    {
        return refusal(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Invalid memory chat key",
        );
    }
    let control = RunControl::new();
    let generation = uuid::Uuid::new_v4().to_string();
    let id = resume.clone().unwrap_or_else(Session::new_id);
    let (events, _) = broadcast::channel(512);
    let task_token = {
        let mut sessions = chat.sessions.lock().await;
        if chat.stopping.is_cancelled() {
            return refusal(
                StatusCode::SERVICE_UNAVAILABLE,
                "stopping",
                "Server is stopping",
            );
        }
        if let Some(slot) = sessions.get(&key) {
            match slot {
                ChatSlot::Memory(memory)
                    if matches!(memory.execution, Execution::Ready(_))
                        && (resume.is_none() || memory.id == id) =>
                {
                    return Json(memory.response(&key)).into_response()
                }
                ChatSlot::Memory(memory)
                    if matches!(memory.execution, Execution::Blocked)
                        && resume.as_deref() == Some(memory.id.as_str()) => {}
                _ => {
                    return refusal(
                        StatusCode::CONFLICT,
                        "mode_mismatch",
                        "This key is already in use; explicitly resume the memory conversation",
                    )
                }
            }
        }
        if resume.is_some()
            && sessions.iter().any(|(other, slot)| {
                other != &key && matches!(slot, ChatSlot::Memory(memory) if memory.id == id)
            })
        {
            return refusal(
                StatusCode::CONFLICT,
                "busy",
                "This memory conversation is already open",
            );
        }
        sessions.insert(
            key.clone(),
            ChatSlot::Memory(Box::new(MemorySession {
                generation: generation.clone(),
                id: id.clone(),
                model: profile.opts.model.clone().unwrap_or_default(),
                permission_mode: "read_only".into(),
                workspace: chat.work_dir().join(&key),
                events,
                execution: Execution::Starting(control.clone()),
                next_turn: None,
                last_report: None,
                blocked_reason: None,
                entries: Vec::new(),
                taint: Default::default(),
            })),
        );
        chat.runs.token()
    };
    #[cfg(test)]
    if let Some(gate) = &chat.memory_test_hooks.reservation {
        gate.pause().await;
    }
    let (reply, rx) = oneshot::channel();
    let task_chat = Arc::clone(&chat);
    chat.runs.spawn(async move {
        let _task_token = task_token;
        let result = if control.cancel.is_cancelled() {
            Err(anyhow::anyhow!("opening stopped before preparation"))
        } else {
            tokio::select! {
                _ = control.stop.cancelled() => Err(anyhow::anyhow!("opening stopped")),
                result = prepare_runtime(&task_chat, &profile, &key, &id, resume.is_some()) => result,
            }
        };
        let mut sessions = task_chat.sessions.lock().await;
        if let Some(ChatSlot::Memory(memory)) = sessions.get_mut(&key) {
            if memory.generation != generation { return; }
            let response = match result {
                Ok(runtime) if !control.cancel.is_cancelled() => {
                    memory.workspace = runtime.prepared.workspace.clone();
                    memory.permission_mode=chat::mode_wire(runtime.prepared.config.tools.permission_mode).into();
                    memory.entries = runtime.entries();
                    memory.taint = runtime.conversation.taint;
                    match runtime.memory.next_turn_ordinal() {
                        Ok(next) => { memory.next_turn=Some(next);memory.execution=Execution::Ready(Box::new(runtime));memory.notify();Json(memory.response(&key)).into_response() }
                        Err(_) => { drop(runtime);memory.execution=Execution::Blocked;memory.blocked_reason=Some("authentication_failed");memory.notify();refusal(StatusCode::CONFLICT,"memory_unavailable","Memory continuation could not be authenticated") }
                    }
                }
                _ => {
                    memory.execution=Execution::Blocked;memory.next_turn=None;
                    memory.blocked_reason=Some(if control.cancel.is_cancelled(){"interrupted"}else{"opening_failed"});
                    memory.notify();refusal(StatusCode::CONFLICT,"memory_unavailable","Memory chat could not be opened; explicit authenticated resume is required")
                }
            };
            let _ = reply.send(response);
        }
    });
    rx.await.unwrap_or_else(|_| {
        refusal(
            StatusCode::SERVICE_UNAVAILABLE,
            "memory_unavailable",
            "Memory opening was interrupted",
        )
    })
}

async fn prepare_runtime(
    chat: &ChatState,
    profile: &MemoryHostProfile,
    key: &str,
    id: &str,
    resume: bool,
) -> Result<OwnedMemoryRuntime> {
    let (workspace, prior) = if resume {
        let path = Session::find(chat.sessions_dir(), id)?;
        let transcript = Session::read(&path)?;
        #[cfg(test)]
        if let Some(gate) = &chat.memory_test_hooks.resume_read {
            gate.pause().await;
        }
        anyhow::ensure!(
            transcript.meta.id == id && transcript.meta.kind == Some(SessionKind::Web),
            "wrong web session identity"
        );
        (
            transcript.meta.workspace.canonicalize()?,
            Some((
                Session {
                    meta: transcript.meta,
                    path,
                },
                transcript.chat_memory,
            )),
        )
    } else {
        let workspace = chat.work_dir().join(key);
        std::fs::create_dir_all(&workspace)?;
        (workspace.canonicalize()?, None)
    };
    let mut opts = profile.opts.clone();
    opts.workspace = Some(workspace.clone());
    let config = Config::load_global()?;
    let memory = memory_profile::open_local_profile_at(&opts, &config, &profile.path, &workspace)?;
    let (session, prior_marker) = match prior {
        Some((session, marker)) => {
            anyhow::ensure!(
                session.meta.provider == opts.provider.as_deref().unwrap_or_default()
                    && session.meta.model == opts.model.as_deref().unwrap_or_default(),
                "memory profile changed"
            );
            (session, marker)
        }
        None => (
            Session::create(
                chat.sessions_dir(),
                SessionMeta {
                    id: id.into(),
                    created_at: chrono::Utc::now(),
                    provider: opts.provider.clone().unwrap(),
                    model: opts.model.clone().unwrap(),
                    workspace: workspace.clone(),
                    title: Some(format!("web: {key}")),
                    kind: Some(SessionKind::Web),
                },
            )?,
            None,
        ),
    };
    let memory = memory.attach_authenticated_web_chat(
        &session,
        if resume {
            ChatAttachMode::Resume
        } else {
            ChatAttachMode::Create
        },
    )?;
    memory.next_turn_ordinal()?;
    // The attachment now owns the exclusive lease and has authenticated the
    // continuation. Never restore the Conversation from the earlier metadata
    // read: another legitimate owner may have completed a turn before this
    // lease was acquired.
    let conversation = if resume {
        let authenticated = Session::read(&session.path)?;
        anyhow::ensure!(
            serde_json::to_value(&authenticated.meta)? == serde_json::to_value(&session.meta)?
                && authenticated.chat_memory == prior_marker,
            "memory source identity changed while acquiring the lease"
        );
        authenticated.convo
    } else {
        Conversation::new()
    };
    let mut tools = setup::prepare_tools(&opts, false).await?;
    memory_profile::validate_memory_config(&opts, &tools.config)?;
    let names = tools
        .registry
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<Vec<_>>();
    memory.validate_surface(&names, &tools.workspace, &[])?;
    anyhow::ensure!(
        names.len() == opts.tools.len() && names.iter().all(|name| opts.tools.contains(name)),
        "unexpected memory tools"
    );
    tools.registry.insert(memory.context_tool()?);
    let _hold =
        crate::follow::hold_router(&tools.config, opts.provider.as_deref(), "web memory setup")
            .await?;
    let prepared = setup::finish_prepared_tools(tools, &opts).await?;
    memory_profile::validate_memory_runtime(&opts, &prepared, &memory, &[])?;
    session.append(&Record::Config(RunConfig::of(
        &prepared.agent,
        &prepared.config,
        &prepared.provider_name,
        &prepared.levers_off,
        Some(&prepared.rules),
    )))?;
    Ok(OwnedMemoryRuntime {
        prepared,
        memory,
        session: Arc::new(session),
        conversation,
        opts,
    })
}

pub(super) async fn send(
    chat: Arc<ChatState>,
    key: String,
    body: &[u8],
    _: super::VerifiedMutation,
) -> Response {
    let request = match decode::<SendRequest>(body) {
        Ok(r) => r,
        Err(e) => return e,
    };
    if request.text.trim().is_empty()
        || request.text.len() > 2048
        || request.request_id.is_empty()
        || request.request_id.len() > 128
    {
        return refusal(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_request",
            "Use nonempty text up to 2048 bytes and a bounded request ID",
        );
    }
    let owner = match OwnerInput::new(OwnerSource::AuthenticatedWeb, request.text.clone()) {
        Ok(o) => o,
        Err(_) => {
            return refusal(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_request",
                "Invalid owner input",
            )
        }
    };
    let control = RunControl::new();
    let (mut runtime, generation, events, task_token) = {
        let mut sessions = chat.sessions.lock().await;
        if chat.stopping.is_cancelled() {
            return refusal(
                StatusCode::SERVICE_UNAVAILABLE,
                "stopping",
                "Server is stopping",
            );
        }
        let Some(ChatSlot::Memory(memory)) = sessions.get_mut(&key) else {
            return refusal(
                StatusCode::NOT_FOUND,
                "resume_required",
                "Explicit memory resume is required",
            );
        };
        if !matches!(memory.execution, Execution::Ready(_)) {
            return refusal(
                StatusCode::CONFLICT,
                "busy",
                "Memory chat is busy or blocked; the message was not accepted",
            );
        }
        if memory.next_turn != Some(request.memory_turn) {
            return refusal(
                StatusCode::CONFLICT,
                "stale_ordinal",
                "Refresh the conversation before sending this message",
            );
        }
        let Execution::Ready(runtime) =
            std::mem::replace(&mut memory.execution, Execution::Starting(control.clone()))
        else {
            unreachable!()
        };
        memory.next_turn = None;
        memory.notify();
        (
            runtime,
            memory.generation.clone(),
            memory.events.clone(),
            chat.runs.token(),
        )
    };
    #[cfg(test)]
    if let Some(gate) = &chat.memory_test_hooks.reservation {
        gate.pause().await;
    }
    let task_chat = Arc::clone(&chat);
    let accepted = request.request_id.clone();
    chat.runs.spawn(async move {
        let _task_token = task_token;
        let result = run_turn(
            &task_chat,
            &key,
            &generation,
            &mut runtime,
            (request, owner),
            &control,
            &events,
        )
        .await;
        let entries = runtime.entries();
        let taint = runtime.conversation.taint;
        let mut sessions = task_chat.sessions.lock().await;
        let Some(ChatSlot::Memory(memory)) = sessions.get_mut(&key) else {
            return;
        };
        if memory.generation != generation {
            return;
        }
        memory.entries = entries;
        memory.taint = taint;
        let (ok, stop, report) = match result {
            Ok((ok, stop, report)) => (ok, stop, Some(report)),
            Err(_) => (false, None, None),
        };
        let next = if ok
            && !control.cancel.is_cancelled()
            && report
                .as_ref()
                .is_some_and(|r| r.status == SessionMemoryStatus::Retained)
        {
            runtime.memory.next_turn_ordinal().ok()
        } else {
            None
        };
        memory.last_report = report.map(Into::into);
        memory.next_turn = next;
        if next.is_some() {
            memory.execution = Execution::Ready(runtime);
        } else {
            drop(runtime);
            memory.execution = Execution::Blocked;
            memory.blocked_reason = Some(if control.cancel.is_cancelled() {
                "interrupted"
            } else {
                "memory_unavailable"
            });
        }
        memory.notify();
        let _ = events.send(WireEvent::Done {
            ok,
            stop,
            taint_private: taint.private,
            taint_untrusted: taint.untrusted,
            error: (!ok).then(|| "Memory turn did not complete; continuation is blocked".into()),
        });
    });
    Json(serde_json::json!({"started":true,"request_id":accepted})).into_response()
}

async fn run_turn(
    chat: &ChatState,
    key: &str,
    generation: &str,
    runtime: &mut OwnedMemoryRuntime,
    ingress: (SendRequest, OwnerInput),
    control: &RunControl,
    events: &broadcast::Sender<WireEvent>,
) -> Result<(bool, Option<String>, ChatMemoryReport)> {
    let (request, owner) = ingress;
    // The runtime has exclusive Starting ownership. Full chain authentication
    // runs outside the shared map lock and before router observation or writes.
    anyhow::ensure!(
        runtime.memory.next_turn_ordinal()? == request.memory_turn,
        "memory continuation changed before admission"
    );
    let _hold = tokio::select! {
        _=control.stop.cancelled()=>anyhow::bail!("turn stopped before hold"),
        held=crate::follow::hold_router(&runtime.prepared.config,runtime.opts.provider.as_deref(),"web memory turn")=>held?,
    };
    anyhow::ensure!(
        !control.cancel.is_cancelled(),
        "turn stopped before admission"
    );
    let turn = runtime.memory.begin_authenticated_web_turn(
        &runtime.session,
        &owner,
        request.memory_turn,
    )?;
    let record_result = (|| -> Result<()> {
        let tool = runtime.memory.goal_memory(&turn)?;
        let old = runtime
            .prepared
            .agent
            .registry()
            .get("goal_context")
            .context("missing context tool")?;
        anyhow::ensure!(
            old.input_schema() == tool.input_schema() && old.description() == tool.description(),
            "memory tool schema changed"
        );
        runtime.prepared.agent.registry_mut().insert(tool);
        let user = Message::user(request.text.clone());
        runtime.session.append(&Record::Message(user.clone()))?;
        runtime.conversation.push(user);
        Ok(())
    })();
    if record_result.is_err() {
        return Ok((false, None, runtime.memory.finish_turn(&turn)));
    }
    let before = runtime.conversation.messages.clone();
    {
        let mut sessions = chat.sessions.lock().await;
        let Some(ChatSlot::Memory(memory)) = sessions.get_mut(key) else {
            anyhow::bail!("reservation lost");
        };
        anyhow::ensure!(memory.generation == generation, "reservation replaced");
        memory.execution = Execution::Running(control.clone());
        memory.notify();
    }
    let _ = events.send(WireEvent::User {
        text: request.text,
        spoken: false,
        request_id: Some(request.request_id),
    });
    let cx = runtime
        .prepared
        .agent
        .context()
        .as_ref()
        .clone()
        .with_cancel_handle(control.cancel.clone())
        .with_goal_ingress(vec![owner], Some(runtime.session.clone()));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let events = events.clone();
    let pump = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if let Some(wire) = chat::wire_event(&event, None) {
                let _ = events.send(wire);
            }
        }
    });
    let outcome = runtime
        .prepared
        .agent
        .run_in(&cx, &mut runtime.conversation, Some(tx))
        .await;
    let _ = pump.await;
    let mut recorded = runtime
        .session
        .record_run(&before, &runtime.conversation)
        .is_ok();
    let (ok, stop) = match outcome {
        Ok(outcome) => {
            recorded &= runtime.session.record_outcome(&outcome).is_ok();
            let completed = outcome.stop_cause == mecha_core::agent::StopCause::Completed;
            (completed, Some(format!("{:?}", outcome.stop_cause)))
        }
        Err(_) => (false, None),
    };
    recorded &= runtime
        .session
        .append(&Record::Taint(runtime.conversation.taint))
        .is_ok();
    let report = runtime.memory.finish_turn(&turn);
    Ok((
        ok && recorded && !control.cancel.is_cancelled(),
        stop,
        report,
    ))
}

#[allow(clippy::result_large_err)]
pub(super) async fn guard(
    state: &super::WebState,
    request: axum::extract::Request,
) -> Result<axum::extract::Request, Response> {
    use axum::extract::{FromRequestParts, MatchedPath, RawPathParams};
    let Some(chat) = &state.chat else {
        return Ok(request);
    };
    let template = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default();
    let (mut parts, body) = request.into_parts();
    let captures = RawPathParams::from_request_parts(&mut parts, &())
        .await
        .ok();
    let value = |name: &str| {
        captures
            .as_ref()
            .and_then(|p| p.iter().find(|(key, _)| *key == name))
            .map(|(_, value)| value.to_string())
    };
    if let Some(key) = value("key") {
        if is_memory(chat, &key).await
            && !matches!(
                template.as_str(),
                "/api/chat/{key}"
                    | "/api/chat/{key}/send"
                    | "/api/chat/{key}/events"
                    | "/api/chat/{key}/cancel"
            )
        {
            return Err(unsupported());
        }
    }
    Ok(axum::extract::Request::from_parts(parts, body))
}
