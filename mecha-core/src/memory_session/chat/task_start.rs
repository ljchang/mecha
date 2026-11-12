//! Private deterministic task startup; the native reservation precedes SQLite.
use super::super::native_writer::{BoundNativeSession, NativeIdentity};
use super::*;
use crate::memory::{TaskDraft, TaskOperationId, TaskRef};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TaskAttachment {
    pub(super) task: TaskRef,
    pub(super) tools: Vec<String>,
    pub(super) startup_digest: String,
}
impl TaskAttachment {
    pub(super) fn validate_tools(&self, tools: &[String]) -> Result<()> {
        surface(tools)?;
        let mut actual = tools.to_vec();
        actual.sort();
        ensure!(actual == self.tools, "task selected surface changed");
        Ok(())
    }
}
pub(super) fn surface(names: &[String]) -> Result<()> {
    let unique: BTreeSet<_> = names.iter().collect();
    ensure!(
        unique.len() == names.len()
            && names.iter().all(|n| matches!(
                n.as_str(),
                "fs_read" | "fs_list" | "fs_write" | "goal_context" | "todo"
            ))
            && names.iter().any(|n| n == "todo")
            && names.iter().any(|n| n == "goal_context"),
        "task chat requires explicit local todo and goal_context surface"
    );
    Ok(())
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Startup {
    protocol: String,
    request: SessionMemoryRequest,
    authority: String,
    instance: String,
    session: crate::session::SessionMeta,
    source: PathBuf,
    workspace: PathBuf,
    device: u64,
    inode: u64,
    meta_length: u64,
    meta_digest: String,
    draft_digest: String,
    tools: Vec<String>,
}
fn path(a: &Authority, id: &str) -> PathBuf {
    a.root.join(format!(
        "{}.chat-task-start",
        authority::digest(id.as_bytes())
    ))
}
fn reservation(s: &Startup, digest: &str) -> Record {
    Record::ChatMemory {
        marker: ChatMemoryMarker {
            protocol: CHAT_TASK_START_PROTOCOL.into(),
            conversation_id: s.session.id.clone(),
            authority: s.authority.clone(),
            attachment: digest.into(),
        },
    }
}
pub(super) fn validate_reservation(a: &Attachment, records: &[Record]) -> Result<()> {
    let task = a
        .task
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("task absent"))?;
    let claim_path = a.request.authority.join(format!(
        "{}.chat-task-start",
        authority::digest(a.session_id.as_bytes())
    ));
    let claim: Startup = read_private(&claim_path, 65536)?;
    let claim_bytes = authority::read(&claim_path, 65536)?;
    ensure!(
        serde_json::to_vec(&claim)? == claim_bytes
            && authority::digest(&claim_bytes) == task.startup_digest
            && claim.protocol == CHAT_TASK_START_PROTOCOL
            && claim.request == a.request
            && claim.authority == a.authority
            && claim.session.id == a.session_id
            && claim.source == a.source
            && claim.workspace == a.workspace
            && claim.device == a.device
            && claim.inode == a.inode
            && claim.tools == task.tools
            && a.nonce == task.startup_digest,
        "task startup authority changed"
    );
    let meta = crate::session::encode_record_line(
        records
            .first()
            .ok_or_else(|| anyhow::anyhow!("task Meta absent"))?,
    )?;
    ensure!(
        meta.len() as u64 == claim.meta_length
            && authority::digest(&meta) == claim.meta_digest
            && matches!(records.first(), Some(Record::Meta(m)) if serde_json::to_vec(m)? == serde_json::to_vec(&claim.session)?),
        "task startup Meta changed"
    );
    ensure!(
        matches!(records, [Record::Meta(_), Record::ChatMemory { marker }] if marker.protocol == CHAT_TASK_START_PROTOCOL && marker.authority == a.authority && marker.conversation_id == a.session_id && marker.attachment == task.startup_digest),
        "task reservation mismatch"
    );
    Ok(())
}
impl SessionMemory {
    pub fn load_task_draft(path: &Path, workspace: &Path) -> Result<TaskDraft> {
        authority::request_path(path, workspace)?;
        let bytes = authority::read(path, 16384)?;
        let wire: crate::memory::plan_extraction::UniqueValue = serde_json::from_slice(&bytes)?;
        let draft: TaskDraft = serde_json::from_value(wire.0)?;
        draft.validate()?;
        Ok(draft)
    }
    pub fn validate_task_surface(
        &self,
        names: &[String],
        workspace: &Path,
        grants: &[crate::grant::Grant],
    ) -> Result<()> {
        surface(names)?;
        ensure!(
            grants.is_empty() && workspace.canonicalize()? == self.execution_workspace()?,
            "task workspace or grants changed"
        );
        authority::exclude_workspace(&self.request, workspace)?;
        self.validate_planning()?;
        Ok(())
    }
    pub fn validate_planning(&self) -> Result<()> {
        let db = self.memory()?;
        ensure!(
            db.session_schema_version()? == 10,
            "task chat requires explicit memory-session enable-planning (schema v10)"
        );
        db.task_session(&self.request.account, "planning-preflight", self.access())?;
        Ok(())
    }
    /// Explicit owner migration; each version transition is separately atomic.
    pub fn enable_planning(&self) -> Result<i64> {
        let mut db = self.memory()?;
        let result = (|| -> Result<()> {
            match db.session_schema_version()? {
                8 => {
                    db.enable_session_tasks()?;
                    db.enable_session_plans()?;
                }
                9 => db.enable_session_plans()?,
                10 => {}
                _ => anyhow::bail!("unsupported planning schema"),
            }
            db.task_session(&self.request.account, "planning-preflight", self.access())?;
            Ok(())
        })();
        result.map_err(|e| {
            anyhow::anyhow!(
                "enable-planning failed; surviving schema_version={}: {e:#}",
                db.session_schema_version().unwrap_or(-1)
            )
        })?;
        db.session_schema_version()
    }
    pub fn attach_task_chat(
        self,
        session: &Session,
        draft: Option<TaskDraft>,
        tools: &[String],
    ) -> Result<ChatMemory> {
        let workspace = self.execution_workspace()?.to_path_buf();
        self.validate_task_surface(tools, &workspace, &[])?;
        ensure!(
            session.meta.kind == Some(SessionKind::Chat),
            "task requires native CLI chat"
        );
        authority::exclude_path(&session.path, &workspace)?;
        let lease = Arc::new(Lease::acquire(&self.authority, &session.meta.id)?);
        if let Some(draft) = draft {
            draft.validate()?;
            ensure!(
                !path(&self.authority, &session.meta.id).try_exists()?
                    && !attachment_path(&self.authority, &session.meta.id).try_exists()?,
                "task startup already claimed; use retry-task-start"
            );
            let bytes = authority::read(&session.path, authority::SOURCE_MAX)?;
            ensure!(
                matches!(recording::records(&bytes)?.as_slice(), [Record::Meta(m)] if serde_json::to_vec(m)? == serde_json::to_vec(&session.meta)? && m.workspace.canonicalize()? == workspace),
                "task start requires exact fresh Meta"
            );
            let meta = open_nofollow(&session.path, false)?.metadata()?;
            ensure!(meta.nlink() == 1, "task source aliased");
            let mut tools = tools.to_vec();
            tools.sort();
            let claim = Startup {
                protocol: CHAT_TASK_START_PROTOCOL.into(),
                request: self.request.clone(),
                authority: self.authority.identity.clone(),
                instance: self.authority.instance.clone(),
                session: session.meta.clone(),
                source: session.path.clone(),
                workspace,
                device: meta.dev(),
                inode: meta.ino(),
                meta_length: bytes.len() as u64,
                meta_digest: authority::digest(&bytes),
                draft_digest: authority::digest(&serde_json::to_vec(&draft)?),
                tools,
            };
            authority::write(
                &path(&self.authority, &session.meta.id),
                &serde_json::to_vec(&claim)?,
            )?;
            #[cfg(test)]
            checkpoint("claim")?;
            self.complete_task_start(&session.meta.id, draft, &lease)?;
        }
        let (a, digest) = attachment(&self.authority, &session.meta.id)?;
        ensure!(
            a.protocol == CHAT_TASK_MEMORY_PROTOCOL
                && a.source == session.path
                && a.workspace == self.execution_workspace()?,
            "task attachment/source mismatch"
        );
        a.validate_tools(tools)?;
        let mut chat = ChatMemory {
            memory: Arc::new(self),
            attachment: a,
            digest,
            lease,
            task_runtime: None,
            verified: std::sync::Mutex::new(None),
        };
        // Authorized pending diagnostics are evaluated before continuation rejects an unsealed turn.
        chat.task_diagnostics()?;
        for ordinal in 1..=MAX_TURNS {
            let index = ordinal_path(&chat.memory.authority, &chat.attachment.session_id, ordinal);
            if !index.try_exists()? {
                break;
            }
            let id = String::from_utf8(authority::read(&index, 128)?)?;
            if !chat
                .memory
                .authority
                .file(&id, "chat-retained")?
                .try_exists()?
            {
                anyhow::bail!("task {} turn {} has unfinished native recording or audit complete with retention pending; use memory-session retry", chat.attachment.session_id, id);
            }
        }
        let (bytes, _) = chat.continuation()?;
        let binding = task_plan::PlanBinding::from_host(
            &chat,
            chat.attachment.task.as_ref().unwrap().task.clone(),
        )?;
        let todo = Arc::new(crate::tool::todo::TodoTool::with_memory(Arc::clone(
            &binding,
        ))?);
        let writer = BoundNativeSession::bind(
            session,
            NativeIdentity {
                device: chat.attachment.device,
                inode: chat.attachment.inode,
            },
            &bytes,
        )?;
        chat.task_runtime = Some(task_host::TaskRuntime {
            binding,
            todo,
            writer: Arc::new(Mutex::new(writer)),
        });
        Ok(chat)
    }
    pub fn retry_task_start_file(&self, session_id: &str, file: &Path) -> Result<()> {
        let claim: Startup = read_private(&path(&self.authority, session_id), 65536)?;
        let draft = Self::load_task_draft(file, &claim.workspace)?;
        self.retry_task_start(session_id, draft)
    }
    pub fn retry_task_start(&self, session_id: &str, draft: TaskDraft) -> Result<()> {
        let lease = Lease::acquire(&self.authority, session_id)?;
        self.complete_task_start(session_id, draft, &lease)
    }
    fn complete_task_start(&self, id: &str, draft: TaskDraft, lease: &Lease) -> Result<()> {
        lease.validate()?;
        self.validate_planning()?;
        let claim_path = path(&self.authority, id);
        let claim: Startup = read_private(&claim_path, 65536)?;
        let claim_bytes = authority::read(&claim_path, 65536)?;
        ensure!(
            serde_json::to_vec(&claim)? == claim_bytes
                && claim.protocol == CHAT_TASK_START_PROTOCOL
                && claim.request == self.request
                && claim.authority == self.authority.identity
                && claim.instance == self.authority.instance
                && claim.session.id == id
                && claim.session.kind == Some(SessionKind::Chat)
                && claim.draft_digest == authority::digest(&serde_json::to_vec(&draft)?),
            "startup claim/draft mismatch"
        );
        draft.validate()?;
        surface(&claim.tools)?;
        authority::exclude_workspace(&self.request, &claim.workspace)?;
        authority::exclude_path(&claim.source, &claim.workspace)?;
        let digest = authority::digest(&claim_bytes);
        let bytes = authority::read(&claim.source, authority::SOURCE_MAX)?;
        let meta_end = usize::try_from(claim.meta_length)?;
        ensure!(
            meta_end <= bytes.len()
                && authority::digest(&bytes[..meta_end]) == claim.meta_digest
                && matches!(recording::records(&bytes[..meta_end])?.as_slice(), [Record::Meta(m)] if serde_json::to_vec(m)? == serde_json::to_vec(&claim.session)?),
            "startup Meta changed"
        );
        let session = Session {
            meta: claim.session.clone(),
            path: claim.source.clone(),
        };
        let mut writer = BoundNativeSession::bind(
            &session,
            NativeIdentity {
                device: claim.device,
                inode: claim.inode,
            },
            &bytes,
        )?;
        let mut prefix = bytes[..meta_end].to_vec();
        prefix.extend(crate::session::encode_record_line(&reservation(
            &claim, &digest,
        ))?);
        if bytes.len() == meta_end {
            writer.append(&reservation(&claim, &digest))?;
        } else {
            ensure!(
                bytes.starts_with(&prefix),
                "startup reservation torn or changed"
            );
        }
        // Require exact known tail before any create/replay of the deterministic operation.
        let existing = if attachment_path(&self.authority, id).try_exists()? {
            Some(attachment(&self.authority, id)?)
        } else {
            None
        };
        if bytes.len() > prefix.len() {
            let (a, d) = existing
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("startup foreign tail"))?;
            let marker = Record::ChatMemory {
                marker: ChatMemoryMarker {
                    protocol: CHAT_TASK_MEMORY_PROTOCOL.into(),
                    conversation_id: id.into(),
                    authority: self.authority.identity.clone(),
                    attachment: d.clone(),
                },
            };
            let mut exact = prefix.clone();
            exact.extend(crate::session::encode_record_line(&marker)?);
            ensure!(
                bytes == exact && a.task.as_ref().is_some_and(|t| t.startup_digest == digest),
                "startup final marker torn or changed"
            );
        } else {
            ensure!(
                bytes.len() == meta_end || bytes == prefix,
                "startup reservation torn"
            );
        }
        // Existing exact bytes may come from an interrupted or failed sync.
        // No Task create or idempotent replay is authorized until this descriptor
        // and its directory entry have both been synced and reverified.
        writer.sync_verified()?;
        #[cfg(test)]
        checkpoint("reservation")?;
        let mut db = self.memory()?;
        let task_session = db.task_session(&self.request.account, id, self.access())?;
        let operation = TaskOperationId::new(&format!("task-start-{}", digest))?;
        let task = db.task_create(&task_session, &operation, draft)?.task;
        #[cfg(test)]
        checkpoint("task")?;
        let a = Attachment {
            task: Some(TaskAttachment {
                task,
                tools: claim.tools.clone(),
                startup_digest: digest.clone(),
            }),
            graph_surface: None,
            web_ingress: None,
            protocol: CHAT_TASK_MEMORY_PROTOCOL.into(),
            request: self.request.clone(),
            authority: self.authority.identity.clone(),
            session_id: id.into(),
            source: claim.source,
            workspace: claim.workspace,
            device: claim.device,
            inode: claim.inode,
            initial_length: prefix.len() as u64,
            initial_digest: authority::digest(&prefix),
            nonce: digest,
        };
        let encoded = a.encoded()?;
        authority::write(&attachment_path(&self.authority, id), &encoded)?;
        #[cfg(test)]
        checkpoint("attachment")?;
        let final_marker = Record::ChatMemory {
            marker: ChatMemoryMarker {
                protocol: CHAT_TASK_MEMORY_PROTOCOL.into(),
                conversation_id: id.into(),
                authority: self.authority.identity.clone(),
                attachment: authority::digest(&encoded),
            },
        };
        if bytes.len() <= prefix.len() {
            writer.append(&final_marker)?;
        }
        writer.sync_verified()?;
        Ok(())
    }
}

#[cfg(test)]
thread_local! { static STOP: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) }; }
#[cfg(test)]
pub(super) fn stop_at(stage: &'static str) {
    STOP.with(|s| s.set(Some(stage)));
}
#[cfg(test)]
fn checkpoint(stage: &str) -> Result<()> {
    let stop = STOP.with(|s| {
        if s.get() == Some(stage) {
            s.set(None);
            true
        } else {
            false
        }
    });
    ensure!(!stop, "injected startup interruption at {stage}");
    Ok(())
}
