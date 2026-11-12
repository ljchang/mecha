//! Explicit encrypted episodic retention for a single fresh ordinary run.
//! This host API never executes an action or a provider; retry only admits a seal.
mod authority;
pub mod chat;
mod inline_notes;
pub mod learning;
pub(crate) mod native_writer;
mod recall;
mod recording;
pub mod surface;
use crate::{
    goal::GoalRef,
    memory::{
        host_capture::*, ordinary_observation::OrdinaryObservationCapture, Access, ForgetReport,
        Memory, SemanticAssessment, StorageAccess,
    },
    session::Session,
    tool::Tool,
};
use anyhow::{ensure, Result};
use authority::{Authority, Frozen, Seal};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
pub const SESSION_MEMORY_PROTOCOL: &str = "ordinary-session-memory-v1";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SessionMemoryRequest {
    pub protocol: String,
    pub db: PathBuf,
    pub key_file: PathBuf,
    pub authority: PathBuf,
    pub domain: String,
    pub account: String,
    pub scopes: BTreeSet<String>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionMemoryStatus {
    Retained,
    Unavailable,
    Pending,
    Forgotten,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionMemoryReason {
    SourceUnavailable,
    RecordingUnavailable,
    AdmissionUnavailable,
    Withdrawn,
}
#[derive(Clone, Debug, Serialize)]
pub struct SessionMemoryReport {
    pub protocol: &'static str,
    pub run_id: String,
    pub status: SessionMemoryStatus,
    pub semantic_assessment: SemanticAssessment,
    pub receipt: Option<CaptureReceipt>,
    pub reason: Option<SessionMemoryReason>,
}
impl SessionMemoryReport {
    fn new(
        id: &str,
        status: SessionMemoryStatus,
        reason: Option<SessionMemoryReason>,
        receipt: Option<CaptureReceipt>,
    ) -> Self {
        Self {
            protocol: SESSION_MEMORY_PROTOCOL,
            run_id: id.into(),
            status,
            semantic_assessment: SemanticAssessment::NotEvaluated,
            reason,
            receipt,
        }
    }
}
#[derive(Default)]
struct RunState {
    next: usize,
    tool_created: bool,
    sealed: bool,
    task_started: bool,
    task_live: Option<std::sync::Weak<chat::task_plan::LivePlanTurn>>,
}
pub struct SessionMemoryRun {
    id: String,
    authority: String,
    state: Arc<Mutex<RunState>>,
}
impl SessionMemoryRun {
    pub fn run_id(&self) -> &str {
        &self.id
    }
}
pub struct SessionMemory {
    request: SessionMemoryRequest,
    workspace: Option<PathBuf>,
    storage: StorageAccess,
    authority: Arc<Authority>,
}
impl SessionMemory {
    /// Rejects a task-writable or aliased manifest before an ordinary run.
    pub fn validate_request_path(path: &Path, workspace: &Path) -> Result<()> {
        authority::request_path(path, workspace)
    }
    pub fn load_request(path: &Path) -> Result<SessionMemoryRequest> {
        let b = authority::read(path, 65536)?;
        let v: crate::memory::plan_extraction::UniqueValue = serde_json::from_slice(&b)?;
        let r: SessionMemoryRequest = serde_json::from_value(v.0)?;
        authority::validate_request(&r)?;
        Ok(r)
    }
    /// Initializes three new owner-selected paths, including a freshly generated key.
    pub fn initialize(request: &SessionMemoryRequest) -> Result<()> {
        authority::initialize(request)
    }
    pub fn open(request: &SessionMemoryRequest, workspace: &Path) -> Result<Self> {
        let (authority, storage) = authority::open(request, Some(workspace))?;
        Ok(Self {
            request: request.clone(),
            workspace: Some(workspace.canonicalize()?),
            storage,
            authority: Arc::new(authority),
        })
    }
    /// Opens existing storage for sealed-input retry and explicit forgetting only.
    /// A recovery handle cannot start a run, finalize a new capture, or expose tools.
    pub fn open_for_recovery(request: &SessionMemoryRequest) -> Result<Self> {
        let (authority, storage) = authority::open(request, None)?;
        Ok(Self {
            request: request.clone(),
            workspace: None,
            storage,
            authority: Arc::new(authority),
        })
    }
    fn execution_workspace(&self) -> Result<&Path> {
        self.workspace
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("ordinary recovery cannot dispatch"))
    }
    pub fn validate_surface(
        &self,
        names: &[String],
        workspace: &Path,
        grants: &[crate::grant::Grant],
    ) -> Result<()> {
        // A grant is the owner's exception to the jail; what a tool reads
        // under one is recorded with the result like any other.
        let _ = grants;
        recording::surface(names)?;
        ensure!(
            workspace.canonicalize()? == self.execution_workspace()?,
            "ordinary memory workspace changed"
        );
        self.authority.validate()?;
        authority::exclude_workspace(&self.request, workspace)?;
        Ok(())
    }
    fn access(&self) -> Access {
        Access {
            domain: self.request.domain.clone(),
            scopes: self.request.scopes.clone(),
        }
    }
    fn memory(&self) -> Result<Memory> {
        self.authority.validate()?;
        let m = Memory::open_with_storage(&self.request.db, self.storage.clone())?
            .with_source_guard(self.authority.clone());
        m.check_access(&self.access())?;
        ensure!(
            m.snapshot()?.instance == self.authority.instance,
            "ordinary database changed"
        );
        Ok(m)
    }
    pub fn begin(
        &self,
        session: &Session,
        prompt: &str,
        goal: &GoalRef,
    ) -> Result<SessionMemoryRun> {
        let workspace = self.execution_workspace()?;
        self.authority.validate()?;
        ensure!(
            !prompt.trim().is_empty() && prompt.len() <= 65_536,
            "ordinary memory prompt unavailable"
        );
        let bytes = authority::read(&session.path, authority::SOURCE_MAX)?;
        recording::initial(&bytes, session, prompt, goal, workspace)?;
        authority::exclude_path(&session.path, workspace)?;
        let frozen = Frozen {
            request: self.request.clone(),
            authority: self.authority.identity.clone(),
            session_id: session.meta.id.clone(),
            source: session.path.clone(),
            workspace: workspace.to_path_buf(),
            prompt: prompt.into(),
            goal: goal.to_string(),
            prefix: bytes,
            nonce: uuid::Uuid::new_v4().to_string(),
        };
        let bytes = serde_json::to_vec(&frozen)?;
        let id = authority::digest(&bytes);
        let claim = self.authority.root.join(format!(
            "{}.session",
            authority::digest(session.meta.id.as_bytes())
        ));
        ensure!(!claim.try_exists()?, "ordinary session already begun");
        authority::write(&claim, id.as_bytes())?;
        authority::write(&self.authority.file(&id, "begin")?, &bytes)?;
        Ok(SessionMemoryRun {
            id,
            authority: self.authority.identity.clone(),
            state: Arc::new(Mutex::new(RunState::default())),
        })
    }
    pub fn goal_memory(&self, run: &SessionMemoryRun) -> Result<Arc<dyn Tool>> {
        self.execution_workspace()?;
        ensure!(
            run.authority == self.authority.identity,
            "ordinary run authority changed"
        );
        let mut state = run
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("ordinary run unavailable"))?;
        ensure!(
            !state.tool_created && !state.sealed,
            "ordinary memory tool already bound"
        );
        let frozen = self.authority.frozen(&run.id)?;
        self.authority.check_prefix(&frozen)?;
        let tool = crate::tool::goal_memory::GoalMemory::open_with_storage_and_source_guard(
            &self.request.db,
            self.storage.clone(),
            self.access(),
            self.request.account.clone(),
            crate::memory::context::Situation {
                event: frozen.prompt,
                goal: frozen.goal,
                needs: None,
            },
            self.authority.clone(),
        )?;
        state.tool_created = true;
        Ok(Arc::new(recall::TrackedTool {
            inner: tool,
            authority: self.authority.clone(),
            id: run.id.clone(),
            state: run.state.clone(),
            storage: self.storage.clone(),
        }))
    }
    pub fn finish(&self, run: &SessionMemoryRun) -> SessionMemoryReport {
        if self.workspace.is_none() || run.authority != self.authority.identity {
            return SessionMemoryReport::new(
                &run.id,
                SessionMemoryStatus::Unavailable,
                Some(SessionMemoryReason::SourceUnavailable),
                None,
            );
        }
        let Ok(mut state) = run.state.lock() else {
            return SessionMemoryReport::new(
                &run.id,
                SessionMemoryStatus::Unavailable,
                Some(SessionMemoryReason::RecordingUnavailable),
                None,
            );
        };
        let prepared = (|| -> Result<()> {
            self.authority.validate()?;
            if self.authority.file(&run.id, "seal")?.try_exists()? {
                return Ok(());
            }
            let frozen = self.authority.frozen(&run.id)?;
            self.authority.check_prefix(&frozen)?;
            let source = authority::read(&frozen.source, authority::SOURCE_MAX)?;
            let attempts = self.authority.attempts(&run.id)?;
            let memory = self.memory()?;
            let payload = recording::capture(
                &frozen,
                &source,
                &attempts,
                &memory,
                &self.access(),
                &run.id,
            )?;
            let seal = Seal {
                frozen_digest: run.id.clone(),
                source_digest: authority::digest(&source),
                attempts,
                payload,
            };
            let bytes = serde_json::to_vec(&seal)?;
            authority::write(&self.authority.file(&run.id, "seal")?, &bytes)?;
            authority::write(
                &self.authority.file(&run.id, "seal-hash")?,
                authority::digest(&bytes).as_bytes(),
            )?;
            Ok(())
        })();
        if prepared.is_err() {
            return SessionMemoryReport::new(
                &run.id,
                SessionMemoryStatus::Unavailable,
                Some(SessionMemoryReason::RecordingUnavailable),
                None,
            );
        }
        state.sealed = true;
        drop(state);
        self.retry(&run.id)
    }
    pub fn retry(&self, run_id: &str) -> SessionMemoryReport {
        let prepared = (|| -> Result<HostCapture> {
            let (seal, revision) = self.authority.seal(run_id)?;
            self.authority.check_seal(run_id, &seal)?;
            let mut payload = seal.payload;
            payload.binding.revision = revision;
            Ok(payload)
        })();
        let payload = match prepared {
            Ok(p) => p,
            Err(_) => {
                return SessionMemoryReport::new(
                    run_id,
                    SessionMemoryStatus::Unavailable,
                    Some(SessionMemoryReason::SourceUnavailable),
                    None,
                )
            }
        };
        let mut memory = match self.memory() {
            Ok(m) => m,
            Err(_) => {
                return SessionMemoryReport::new(
                    run_id,
                    SessionMemoryStatus::Pending,
                    Some(SessionMemoryReason::AdmissionUnavailable),
                    None,
                )
            }
        };
        if memory
            .validate_delivered_context(&payload.consulted, &[], &self.access())
            .is_err()
        {
            return SessionMemoryReport::new(
                run_id,
                SessionMemoryStatus::Unavailable,
                Some(SessionMemoryReason::SourceUnavailable),
                None,
            );
        }
        let result = (|| -> Result<CaptureReceipt> {
            memory.admit_ordinary_observation(
                OrdinaryObservationCapture::try_from_host(payload)?,
                &self.access(),
            )
        })();
        match result {
            Ok(receipt) => {
                SessionMemoryReport::new(run_id, SessionMemoryStatus::Retained, None, Some(receipt))
            }
            Err(_) => SessionMemoryReport::new(
                run_id,
                SessionMemoryStatus::Pending,
                Some(SessionMemoryReason::AdmissionUnavailable),
                None,
            ),
        }
    }
    pub fn forget(&self, run_id: &str) -> Result<ForgetReport> {
        self.authority.validate()?;
        let (seal, _) = self.authority.seal(run_id)?;
        let frozen = self.authority.frozen(run_id)?;
        ensure!(
            seal.frozen_digest == run_id && seal.payload.identity.session_id == frozen.session_id,
            "ordinary seal unavailable"
        );
        let source = seal
            .payload
            .originals
            .first()
            .ok_or_else(|| anyhow::anyhow!("ordinary seal unavailable"))?
            .source
            .clone();
        authority::write(&self.authority.file(run_id, "withdrawn")?, b"withdrawn")?;
        self.memory()?.forget(&source)
    }
}
