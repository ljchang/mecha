//! Host-owned multi-turn attachment. Recovery admits sealed observations only.
use super::{surface::ChatGraphSurface, *};
use crate::{
    goal_context::{OwnerInput, OwnerSource},
    memory::{chat_observation::*, RevisionRef},
    memory_experience_bridge::open_nofollow,
    session::{ChatMemoryMarker, Record, SessionKind},
};
use std::{
    fs::File,
    io::Read,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
};
#[path = "chat_recording.rs"]
mod capture;
mod task_audit;
pub(crate) mod task_host;
pub(crate) mod task_plan;
mod task_start;
pub use task_host::TaskChatTurn;
pub const CHAT_TASK_MEMORY_PROTOCOL: &str = "ordinary-chat-memory-v4";
const CHAT_TASK_START_PROTOCOL: &str = "ordinary-chat-task-start-v1";
#[path = "chat_recall.rs"]
mod tracking;
pub const CHAT_MEMORY_PROTOCOL: &str = "ordinary-chat-memory-v1";
pub const CHAT_GRAPH_MEMORY_PROTOCOL: &str = "ordinary-chat-memory-v2";
pub const CHAT_WEB_MEMORY_PROTOCOL: &str = "ordinary-chat-memory-v3";
/// Error-report tag only. Never accepted as attachment or native marker authority.
pub const CHAT_MEMORY_UNAVAILABLE_PROTOCOL: &str = "ordinary-chat-memory-unavailable";
const MAX_TURNS: u64 = 256;
#[derive(Clone, Copy)]
pub enum ChatAttachMode {
    Create,
    Resume,
}
#[derive(Debug, Serialize)]
pub struct ChatMemoryReport {
    pub protocol: &'static str,
    pub conversation_id: String,
    pub turn_ordinal: u64,
    pub run_id: String,
    pub status: SessionMemoryStatus,
    pub semantic_assessment: SemanticAssessment,
    pub receipt: Option<CaptureReceipt>,
    pub reason: Option<SessionMemoryReason>,
}
impl ChatMemoryReport {
    fn new(
        memory: &SessionMemory,
        id: &str,
        f: Option<&Turn>,
        status: SessionMemoryStatus,
        reason: Option<SessionMemoryReason>,
        receipt: Option<CaptureReceipt>,
    ) -> Self {
        Self {
            protocol: frozen_metadata(&memory.authority, id)
                .map(|(_, a)| a.report_protocol())
                .unwrap_or(CHAT_MEMORY_UNAVAILABLE_PROTOCOL),
            conversation_id: f.map(|f| f.session_id.clone()).unwrap_or_default(),
            turn_ordinal: f.map_or(0, |f| f.ordinal),
            run_id: id.into(),
            status,
            semantic_assessment: SemanticAssessment::NotEvaluated,
            reason,
            receipt,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WebSessionKind {
    Web,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WebOwnerSource {
    AuthenticatedWeb,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WebIngress {
    session_kind: WebSessionKind,
    owner_source: WebOwnerSource,
}
impl WebIngress {
    fn authenticated() -> Self {
        Self {
            session_kind: WebSessionKind::Web,
            owner_source: WebOwnerSource::AuthenticatedWeb,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Attachment {
    #[serde(skip)]
    task: Option<task_start::TaskAttachment>,
    // This is the original V1 wire shape. V2 is dispatched explicitly below;
    // its required graph field is validated before normalizing this value.
    #[serde(skip)]
    graph_surface: Option<ChatGraphSurface>,
    #[serde(skip)]
    web_ingress: Option<WebIngress>,
    protocol: String,
    request: SessionMemoryRequest,
    authority: String,
    session_id: String,
    source: PathBuf,
    workspace: PathBuf,
    device: u64,
    inode: u64,
    initial_length: u64,
    initial_digest: String,
    nonce: String,
}
impl Attachment {
    fn report_protocol(&self) -> &'static str {
        match self.protocol.as_str() {
            CHAT_MEMORY_PROTOCOL => CHAT_MEMORY_PROTOCOL,
            CHAT_GRAPH_MEMORY_PROTOCOL => CHAT_GRAPH_MEMORY_PROTOCOL,
            CHAT_WEB_MEMORY_PROTOCOL => CHAT_WEB_MEMORY_PROTOCOL,
            CHAT_TASK_MEMORY_PROTOCOL => CHAT_TASK_MEMORY_PROTOCOL,
            _ => unreachable!("attachment protocol authenticated before normalization"),
        }
    }
    fn validate_tools(&self, names: &[String]) -> Result<()> {
        if let Some(task) = &self.task {
            return task.validate_tools(names);
        }
        match &self.graph_surface {
            Some(graph) => graph.validate_tools(names),
            None => recording::surface(names),
        }
    }
    /// An ordinary local chat: whichever tools its configuration gives
    /// it, recorded however a turn went. The graph, task and web
    /// attachments are closed profiles with a recorder that refuses what
    /// it does not expect; the web one has a host built on those refusals
    /// (a stopped turn blocks the conversation until an explicit resume)
    /// and opens with that host.
    fn open(&self) -> bool {
        self.task.is_none() && self.graph_surface.is_none() && self.web_ingress.is_none()
    }
    fn owner_source(&self) -> OwnerSource {
        if self.web_ingress.is_some() {
            OwnerSource::AuthenticatedWeb
        } else {
            OwnerSource::CliChat
        }
    }
    fn session_kind(&self) -> SessionKind {
        if self.web_ingress.is_some() {
            SessionKind::Web
        } else {
            SessionKind::Chat
        }
    }
    fn observation_protocol(&self) -> (&'static str, &'static str) {
        if self.task.is_some() {
            (CHAT_TASK_OBSERVATION_PROTOCOL, CHAT_TASK_RECORDER_VERSION)
        } else if self.web_ingress.is_some() {
            (CHAT_WEB_OBSERVATION_PROTOCOL, CHAT_WEB_RECORDER_VERSION)
        } else if self.open() {
            (CHAT_OPEN_OBSERVATION_PROTOCOL, CHAT_OPEN_RECORDER_VERSION)
        } else {
            (CHAT_OBSERVATION_PROTOCOL, CHAT_RECORDER_VERSION)
        }
    }
    fn encoded(&self) -> Result<Vec<u8>> {
        match self.protocol.as_str() {
            CHAT_MEMORY_PROTOCOL => Ok(serde_json::to_vec(self)?), // Preserve original V1 bytes.
            CHAT_TASK_MEMORY_PROTOCOL => {
                let mut wire = serde_json::to_value(self)?;
                wire.as_object_mut().expect("attachment object").insert(
                    "task".into(),
                    serde_json::to_value(
                        self.task
                            .as_ref()
                            .ok_or_else(|| anyhow::anyhow!("task attachment missing"))?,
                    )?,
                );
                Ok(serde_json::to_vec(&wire)?)
            }
            CHAT_GRAPH_MEMORY_PROTOCOL => {
                let mut wire = serde_json::to_value(self)?;
                wire.as_object_mut().expect("attachment object").insert(
                    "graph_surface".into(),
                    serde_json::to_value(
                        self.graph_surface
                            .as_ref()
                            .ok_or_else(|| anyhow::anyhow!("chat graph surface missing"))?,
                    )?,
                );
                Ok(serde_json::to_vec(&wire)?)
            }
            CHAT_WEB_MEMORY_PROTOCOL => {
                ensure!(
                    self.graph_surface.is_none(),
                    "web graph authority unsupported"
                );
                let mut wire = serde_json::to_value(self)?;
                wire.as_object_mut().expect("attachment object").insert(
                    "ingress".into(),
                    serde_json::to_value(
                        self.web_ingress
                            .as_ref()
                            .ok_or_else(|| anyhow::anyhow!("web ingress missing"))?,
                    )?,
                );
                Ok(serde_json::to_vec(&wire)?)
            }
            _ => anyhow::bail!("unknown chat attachment protocol"),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Turn {
    attachment: String,
    session_id: String,
    ordinal: u64,
    previous: Option<String>,
    prefix_start: u64,
    start_length: u64,
    start_digest: String,
    owner: OwnerInput,
    dependencies: Vec<RevisionRef>,
    nonce: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatSeal {
    turn: String,
    length: u64,
    digest: String,
    attempts: Vec<authority::Attempt>,
    payload: HostCapture,
}
struct Lease {
    file: File,
    path: PathBuf,
}
impl Lease {
    fn acquire(a: &Authority, session: &str) -> Result<Self> {
        a.validate()?;
        let path = a.root.join(format!(
            "{}.chat-lock",
            authority::digest(session.as_bytes())
        ));
        if !path.try_exists()? {
            // A racing creator is fine only when the resulting private file validates.
            if let Err(error) = authority::write(&path, b"chat lease") {
                if !path.try_exists()? {
                    return Err(error);
                }
            }
        }
        authority::private(&path, false)?;
        let file = open_nofollow(&path, false)?;
        // SAFETY: owned descriptor; nonblocking so a second attachment refuses.
        ensure!(
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
            "chat memory conversation already attached"
        );
        let result = Self { file, path };
        result.validate()?;
        Ok(result)
    }
    fn validate(&self) -> Result<()> {
        authority::private(&self.path, false)?;
        let named = open_nofollow(&self.path, false)?.metadata()?;
        let held = self.file.metadata()?;
        ensure!(
            named.dev() == held.dev() && named.ino() == held.ino(),
            "chat lease replaced"
        );
        Ok(())
    }
}
// Retained turn ID, receipt, and sealed transcript byte endpoint.
type RetainedTurnCheckpoint = (String, CaptureReceipt, u64);
type ChatContinuation = (Vec<u8>, Option<RetainedTurnCheckpoint>);

pub struct ChatMemory {
    task_runtime: Option<task_host::TaskRuntime>,
    memory: Arc<SessionMemory>,
    attachment: Attachment,
    digest: String,
    lease: Arc<Lease>,
}
pub struct ChatMemoryTurn {
    id: String,
    authority: String,
    attachment: String,
    session_id: String,
    state: Arc<Mutex<RunState>>,
}
impl Drop for ChatMemoryTurn {
    fn drop(&mut self) {
        if let Some(live) = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .task_live
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
        {
            let _ = live.finish();
        }
    }
}
impl ChatMemoryTurn {
    pub fn run_id(&self) -> &str {
        &self.id
    }
}
fn attachment_path(a: &Authority, session: &str) -> PathBuf {
    a.root
        .join(format!("{}.chat", authority::digest(session.as_bytes())))
}
fn ordinal_path(a: &Authority, session: &str, ordinal: u64) -> PathBuf {
    a.root.join(format!(
        "{}.chat-turn-{ordinal}",
        authority::digest(session.as_bytes())
    ))
}
fn read_private<T: serde::de::DeserializeOwned>(path: &Path, max: usize) -> Result<T> {
    authority::private(path, false)?;
    let value: crate::memory::plan_extraction::UniqueValue =
        serde_json::from_slice(&authority::read(path, max)?)?;
    Ok(serde_json::from_value(value.0)?)
}
fn attachment(a: &Authority, session: &str) -> Result<(Attachment, String)> {
    a.validate()?;
    let path = attachment_path(a, session);
    authority::private(&path, false)?;
    let bytes = authority::read(&path, 65536)?;
    // Parse and hash the same read, including duplicate-key rejection. Do not
    // reserialize historical V1 bytes to derive their authority identity.
    let mut wire: crate::memory::plan_extraction::UniqueValue = serde_json::from_slice(&bytes)?;
    let mut web_ingress = None;
    let mut task = None;
    let graph = match wire.0.get("protocol").and_then(|v| v.as_str()) {
        Some(CHAT_TASK_MEMORY_PROTOCOL) => {
            ensure!(
                wire.0.get("graph_surface").is_none() && wire.0.get("ingress").is_none(),
                "task mixed profile"
            );
            task = Some(serde_json::from_value::<task_start::TaskAttachment>(
                wire.0
                    .as_object_mut()
                    .expect("object")
                    .remove("task")
                    .ok_or_else(|| anyhow::anyhow!("task attachment missing"))?,
            )?);
            None
        }
        Some(CHAT_MEMORY_PROTOCOL) => {
            ensure!(
                wire.0.get("graph_surface").is_none() && wire.0.get("ingress").is_none(),
                "V1 attachment cannot contain graph or web authority"
            );
            None
        }
        Some(CHAT_GRAPH_MEMORY_PROTOCOL) => {
            ensure!(
                wire.0.get("ingress").is_none(),
                "V2 attachment cannot contain web authority"
            );
            let graph = wire
                .0
                .as_object_mut()
                .expect("protocol-bearing object")
                .remove("graph_surface")
                .ok_or_else(|| anyhow::anyhow!("chat graph surface missing"))?;
            Some(serde_json::from_value::<ChatGraphSurface>(graph)?)
        }
        Some(CHAT_WEB_MEMORY_PROTOCOL) => {
            ensure!(
                wire.0.get("graph_surface").is_none(),
                "web graph authority unsupported"
            );
            let ingress = wire
                .0
                .as_object_mut()
                .expect("protocol-bearing object")
                .remove("ingress")
                .ok_or_else(|| anyhow::anyhow!("web ingress missing"))?;
            web_ingress = Some(serde_json::from_value::<WebIngress>(ingress)?);
            None
        }
        _ => anyhow::bail!("unknown chat attachment protocol"),
    };
    let mut value: Attachment = serde_json::from_value(wire.0)?;
    if let Some(graph) = &graph {
        graph.validate(&value.workspace)?;
    }
    value.graph_surface = graph;
    value.web_ingress = web_ingress;
    value.task = task;
    if let Some(task) = &value.task {
        task.validate_tools(&task.tools)?;
    }
    ensure!(
        value.session_id == session && value.authority == a.identity && value.request == a.request,
        "chat attachment authority changed"
    );
    Ok((value, authority::digest(&bytes)))
}
fn source_prefix(a: &Attachment, length: u64) -> Result<Vec<u8>> {
    ensure!(
        length <= authority::SOURCE_MAX as u64,
        "chat prefix exceeds 4 MiB profile"
    );
    let file = open_nofollow(&a.source, false)?;
    let meta = file.metadata()?;
    ensure!(
        meta.nlink() == 1
            && meta.dev() == a.device
            && meta.ino() == a.inode
            && meta.len() >= length,
        "chat source replaced, aliased or truncated"
    );
    let mut bytes = Vec::with_capacity(length as usize);
    file.take(length).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 == length, "chat source truncated");
    Ok(bytes)
}
fn frozen_metadata(a: &Authority, id: &str) -> Result<(Turn, Attachment)> {
    ensure!(
        !authority::claim_exists(&a.file(id, "begin")?)?,
        "ambiguous memory protocol"
    );
    let path = a.file(id, "chat-begin")?;
    let f: Turn = read_private(&path, 65536)?;
    ensure!(
        authority::digest(&authority::read(&path, 65536)?) == id,
        "chat begin changed"
    );
    let (attachment, digest) = attachment(a, &f.session_id)?;
    ensure!(
        f.attachment == digest
            && (1..=MAX_TURNS).contains(&f.ordinal)
            && f.owner.source() == attachment.owner_source(),
        "chat turn attachment changed"
    );
    let op = ordinal_path(a, &f.session_id, f.ordinal);
    authority::private(&op, false)?;
    ensure!(
        authority::read(&op, 128)? == id.as_bytes(),
        "chat turn order changed"
    );
    Ok((f, attachment))
}
// Validate the immutable native header without requiring its historical workspace
// to exist. The initial digest binds the original workspace spelling as admitted.
fn validate_source_ingress(a: &Attachment, bytes: &[u8], attachment_digest: &str) -> Result<()> {
    let initial = usize::try_from(a.initial_length)?;
    ensure!(
        initial <= bytes.len() && authority::digest(&bytes[..initial]) == a.initial_digest,
        "chat initial source changed"
    );
    let header = recording::records(&bytes[..initial])?;
    ensure!(
        header.len() == if a.task.is_some() { 2 } else { 1 }
            && matches!(&header[0], Record::Meta(m)
        if m.id == a.session_id && m.kind == Some(a.session_kind())),
        "chat native ingress changed"
    );
    if a.task.is_some() {
        task_start::validate_reservation(a, &header)?;
    }
    let marker_end = bytes[initial..]
        .iter()
        .position(|b| *b == b'\n')
        .ok_or_else(|| anyhow::anyhow!("chat marker unavailable"))?
        + initial
        + 1;
    let marker = recording::records(&bytes[initial..marker_end])?;
    ensure!(
        matches!(marker.as_slice(), [Record::ChatMemory { marker }]
        if marker.protocol == a.protocol && marker.conversation_id == a.session_id
        && marker.authority == a.authority && marker.attachment == attachment_digest),
        "chat attachment marker missing or changed"
    );
    Ok(())
}
fn frozen(a: &Authority, id: &str) -> Result<(Turn, Attachment)> {
    let (f, attachment) = frozen_metadata(a, id)?;
    let prefix = source_prefix(&attachment, f.start_length)?;
    validate_source_ingress(&attachment, &prefix, &f.attachment)?;
    ensure!(
        authority::digest(&prefix) == f.start_digest,
        "chat starting prefix changed"
    );
    Ok((f, attachment))
}
fn seal(a: &Authority, id: &str) -> Result<(ChatSeal, String, Turn)> {
    let (f, attachment) = frozen(a, id)?;
    ensure!(
        !a.file(id, "withdrawn")?.try_exists()?,
        "chat source withdrawn"
    );
    let path = a.file(id, "chat-seal")?;
    let s: ChatSeal = read_private(&path, authority::SOURCE_MAX)?;
    let revision = authority::digest(&authority::read(&path, authority::SOURCE_MAX)?);
    let hash = a.file(id, "chat-seal-hash")?;
    authority::private(&hash, false)?;
    ensure!(
        authority::read(&hash, 128)? == revision.as_bytes(),
        "chat seal changed"
    );
    ensure!(
        s.turn == id
            && s.length > f.start_length
            && s.digest == authority::digest(&source_prefix(&attachment, s.length)?)
            && a.attempts(id)? == s.attempts,
        "chat sealed source changed"
    );
    let (observation, recorder) = attachment.observation_protocol();
    let c = s
        .payload
        .chat_context
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("chat context missing"))?;
    ensure!(
        s.payload.identity.check_id == id
            && s.payload.identity.session_id == f.session_id
            && ((s.payload.identity.check_version == observation
                && s.payload.identity.evaluator_version == recorder)
                // A local chat begun before the open recorder has earlier
                // turns sealed under the strict one; they stay what they
                // are, and say so in their version.
                || (attachment.open()
                    && s.payload.identity.check_version == CHAT_OBSERVATION_PROTOCOL
                    && s.payload.identity.evaluator_version == CHAT_RECORDER_VERSION))
            && s.payload.binding.authority == a.identity
            && s.payload.binding.locator == id
            && s.payload.scopes == a.request.scopes
            && c.turn_ordinal == f.ordinal
            && c.previous_turn == f.previous
            && c.prefix_start == f.prefix_start
            && c.prefix_length == s.length
            && c.prefix_digest == s.digest
            && s.payload.consulted.starts_with(&f.dependencies),
        "chat seal identity changed"
    );
    OrdinaryObservationCapture::try_from_host(s.payload.clone())?;
    Ok((s, revision, f))
}
pub(super) fn check_binding(a: &Authority, b: &ExternalBinding) -> Result<()> {
    let (_, revision, _) = seal(a, &b.locator)?;
    ensure!(
        b.authority == a.identity && revision == b.revision,
        "chat binding changed"
    );
    Ok(())
}
impl SessionMemory {
    /// Graph-only attachment door. Resume authenticates existing authority
    /// without accepting a replacement live surface; compare it explicitly
    /// with `validate_graph_surface` before provider setup.
    pub fn attach_chat_with_graph(
        self,
        session: &Session,
        mode: ChatAttachMode,
        graph: Option<ChatGraphSurface>,
    ) -> Result<ChatMemory> {
        match mode {
            ChatAttachMode::Create => {
                let surface = graph
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("graph creation requires admitted surface"))?;
                surface.validate(self.execution_workspace()?)?;
            }
            ChatAttachMode::Resume => ensure!(
                graph.is_none(),
                "graph resume must authenticate stored surface first"
            ),
        }
        self.attach_chat_protocol(session, mode, graph, CHAT_GRAPH_MEMORY_PROTOCOL)
    }

    pub fn attach_chat(self, session: &Session, mode: ChatAttachMode) -> Result<ChatMemory> {
        self.attach_chat_protocol(session, mode, None, CHAT_MEMORY_PROTOCOL)
    }
    /// Host-only door: the caller must authenticate the browser's owner intent.
    /// A recorded AuthenticatedWeb label is not HTTP authentication.
    pub fn attach_authenticated_web_chat(
        self,
        session: &Session,
        mode: ChatAttachMode,
    ) -> Result<ChatMemory> {
        self.attach_chat_protocol(session, mode, None, CHAT_WEB_MEMORY_PROTOCOL)
    }
    fn attach_chat_protocol(
        self,
        session: &Session,
        mode: ChatAttachMode,
        graph: Option<ChatGraphSurface>,
        protocol: &str,
    ) -> Result<ChatMemory> {
        let workspace = self.execution_workspace()?.to_path_buf();
        let web = protocol == CHAT_WEB_MEMORY_PROTOCOL;
        let kind = if web {
            SessionKind::Web
        } else {
            SessionKind::Chat
        };
        ensure!(
            session.meta.kind == Some(kind),
            "chat native ingress does not match selected profile"
        );
        self.authority.validate()?;
        authority::exclude_path(&session.path, &workspace)?;
        let lease = Lease::acquire(&self.authority, &session.meta.id)?;
        if matches!(mode, ChatAttachMode::Create) {
            ensure!(
                !attachment_path(&self.authority, &session.meta.id).try_exists()?,
                "chat already attached"
            );
            ensure!(
                !self
                    .authority
                    .root
                    .join(format!(
                        "{}.session",
                        authority::digest(session.meta.id.as_bytes())
                    ))
                    .try_exists()?,
                "session already has one-shot authority"
            );
            let bytes = authority::read(&session.path, authority::SOURCE_MAX)?;
            let rs = recording::records(&bytes)?;
            ensure!(
                rs.len() == 1
                    && matches!(&rs[0],Record::Meta(m) if m.id==session.meta.id && m.kind==Some(kind) && m.workspace.canonicalize().is_ok_and(|w|w==workspace)),
                "chat attachment requires fresh recorded chat"
            );
            let meta = open_nofollow(&session.path, false)?.metadata()?;
            ensure!(meta.nlink() == 1, "chat source aliased");
            let a = Attachment {
                task: None,
                graph_surface: graph,
                web_ingress: web.then(WebIngress::authenticated),
                protocol: protocol.into(),
                request: self.request.clone(),
                authority: self.authority.identity.clone(),
                session_id: session.meta.id.clone(),
                source: session.path.clone(),
                workspace: workspace.clone(),
                device: meta.dev(),
                inode: meta.ino(),
                initial_length: bytes.len() as u64,
                initial_digest: authority::digest(&bytes),
                nonce: uuid::Uuid::new_v4().to_string(),
            };
            let data = a.encoded()?;
            authority::write(&attachment_path(&self.authority, &session.meta.id), &data)?;
            session.append(&Record::ChatMemory {
                marker: ChatMemoryMarker {
                    protocol: protocol.into(),
                    conversation_id: session.meta.id.clone(),
                    authority: self.authority.identity.clone(),
                    attachment: authority::digest(&data),
                },
            })?;
        }
        let (attachment, digest) = attachment(&self.authority, &session.meta.id)?;
        ensure!(
            attachment.protocol == protocol,
            "chat memory protocol does not match selected profile"
        );
        ensure!(
            attachment.workspace == workspace && attachment.source == session.path,
            "chat workspace/source changed"
        );
        let chat = ChatMemory {
            task_runtime: None,
            memory: Arc::new(self),
            attachment,
            digest,
            lease: Arc::new(lease),
        };
        chat.continuation()?;
        Ok(chat)
    }
    /// Protocol discrimination never falls back after a malformed or dual claim.
    pub fn is_chat_capture(&self, id: &str) -> Result<bool> {
        self.authority.validate()?;
        let chat = authority::claim_exists(&self.authority.file(id, "chat-begin")?)?;
        let ordinary = authority::claim_exists(&self.authority.file(id, "begin")?)?;
        ensure!(chat != ordinary, "memory capture missing or ambiguous");
        if chat {
            frozen_metadata(&self.authority, id)?;
        } else {
            self.authority.frozen(id)?;
        }
        Ok(chat)
    }
    pub fn retry_chat(&self, id: &str) -> ChatMemoryReport {
        let prepared = (|| -> Result<_> {
            let (f, _) = frozen_metadata(&self.authority, id)?;
            let lease = Lease::acquire(&self.authority, &f.session_id)?;
            Ok((f, lease))
        })();
        match prepared {
            Ok((f, _lease)) => {
                if let Ok((_, a)) = frozen_metadata(&self.authority, id) {
                    if a.task.is_some() && task_audit::recover(self, id).is_err() {
                        return ChatMemoryReport::new(
                            self,
                            id,
                            Some(&f),
                            SessionMemoryStatus::Pending,
                            Some(SessionMemoryReason::RecordingUnavailable),
                            None,
                        );
                    }
                }
                retry(self, id, Some(&f))
            }
            Err(_) => ChatMemoryReport::new(
                self,
                id,
                None,
                SessionMemoryStatus::Unavailable,
                Some(SessionMemoryReason::SourceUnavailable),
                None,
            ),
        }
    }
    pub fn forget_chat(&self, id: &str) -> Result<ForgetReport> {
        let (f, _) = frozen_metadata(&self.authority, id)?;
        let _lease = Lease::acquire(&self.authority, &f.session_id)?;
        // Forget remains possible when a source vanished or was edited.
        let path = self.authority.file(id, "chat-seal")?;
        let s: ChatSeal = read_private(&path, authority::SOURCE_MAX)?;
        ensure!(
            s.turn == id && s.payload.identity.session_id == f.session_id,
            "chat seal identity changed"
        );
        let hash = self.authority.file(id, "chat-seal-hash")?;
        ensure!(
            authority::read(&hash, 128)?
                == authority::digest(&authority::read(&path, authority::SOURCE_MAX)?).as_bytes(),
            "chat seal changed"
        );
        let source = s
            .payload
            .originals
            .first()
            .ok_or_else(|| anyhow::anyhow!("chat originals missing"))?
            .source
            .clone();
        if !self.authority.file(id, "withdrawn")?.try_exists()? {
            authority::write(&self.authority.file(id, "withdrawn")?, b"withdrawn")?;
        }
        self.memory()?.forget(&source)
    }
}
fn retry(memory: &SessionMemory, id: &str, known: Option<&Turn>) -> ChatMemoryReport {
    let prepared = (|| -> Result<_> {
        let (mut s, revision, f) = seal(&memory.authority, id)?;
        s.payload.binding.revision = revision;
        Ok((s.payload, f))
    })();
    let (payload, f) = match prepared {
        Ok(p) => p,
        Err(_) => {
            return ChatMemoryReport::new(
                memory,
                id,
                known,
                SessionMemoryStatus::Unavailable,
                Some(SessionMemoryReason::SourceUnavailable),
                None,
            )
        }
    };
    let mut db = match memory.memory() {
        Ok(m) => m,
        Err(_) => {
            return ChatMemoryReport::new(
                memory,
                id,
                Some(&f),
                SessionMemoryStatus::Pending,
                Some(SessionMemoryReason::AdmissionUnavailable),
                None,
            )
        }
    };
    if db
        .validate_dependency_roots(&payload.consulted, &memory.access())
        .is_err()
    {
        return ChatMemoryReport::new(
            memory,
            id,
            Some(&f),
            SessionMemoryStatus::Unavailable,
            Some(SessionMemoryReason::SourceUnavailable),
            None,
        );
    }
    let admitted = (|| -> Result<_> {
        let receipt = db.admit_ordinary_observation(
            OrdinaryObservationCapture::try_from_host(payload)?,
            &memory.access(),
        )?;
        let path = memory.authority.file(id, "chat-retained")?;
        if path.try_exists()? {
            let previous: CaptureReceipt = read_private(&path, 65536)?;
            ensure!(previous == receipt, "chat receipt changed");
        } else {
            authority::write(&path, &serde_json::to_vec(&receipt)?)?;
        }
        Ok(receipt)
    })();
    match admitted {
        Ok(r) => ChatMemoryReport::new(
            memory,
            id,
            Some(&f),
            SessionMemoryStatus::Retained,
            None,
            Some(r),
        ),
        Err(_) => ChatMemoryReport::new(
            memory,
            id,
            Some(&f),
            SessionMemoryStatus::Pending,
            Some(SessionMemoryReason::AdmissionUnavailable),
            None,
        ),
    }
}
impl ChatMemory {
    pub fn conversation_id(&self) -> &str {
        &self.attachment.session_id
    }
    pub fn validate_surface(
        &self,
        names: &[String],
        workspace: &Path,
        grants: &[crate::grant::Grant],
    ) -> Result<()> {
        self.lease.validate()?;
        if let Some(task) = &self.attachment.task {
            self.memory
                .validate_task_surface(names, workspace, grants)?;
            task.validate_tools(names)?;
            return Ok(());
        }
        let graph = self.graph_surface()?;
        if let Some(graph) = graph {
            ensure!(grants.is_empty(), "ordinary memory does not support grants");
            ensure!(
                workspace.canonicalize()? == self.memory.execution_workspace()?,
                "ordinary memory workspace changed"
            );
            graph.validate(self.memory.execution_workspace()?)?;
            graph.validate_tools(names)?;
            authority::exclude_workspace(&self.memory.request, workspace)?;
            Ok(())
        } else {
            // A grant that reaches the store or this conversation's
            // transcript is refused; any other is the owner's exception.
            authority::exclude_grants(&self.memory.request, Some(&self.attachment.source), grants)?;
            self.memory.validate_surface(names, workspace, grants)
        }
    }
    /// Whether this is an ordinary chat rather than a closed profile: its
    /// host goes on without remembering a turn the recorder cannot capture.
    pub fn is_open(&self) -> bool {
        self.attachment.open()
    }
    /// Authenticates immutable private attachment bytes, without runtime I/O.
    pub fn graph_surface(&self) -> Result<Option<ChatGraphSurface>> {
        self.lease.validate()?;
        let (a, digest) = attachment(&self.memory.authority, &self.attachment.session_id)?;
        ensure!(digest == self.digest, "chat attachment replaced");
        Ok(a.graph_surface)
    }
    pub fn validate_graph_surface(&self, actual: &ChatGraphSurface) -> Result<()> {
        actual.validate(&self.attachment.workspace)?;
        ensure!(
            self.graph_surface()?.as_ref() == Some(actual),
            "chat graph surface changed"
        );
        Ok(())
    }
    /// Stable schema before Config. Dispatch refuses until begin_turn binds a
    /// tracked reader; this placeholder retains metadata only, never storage.
    pub fn context_tool(&self) -> Result<Arc<dyn Tool>> {
        self.lease.validate()?;
        let reader = crate::tool::goal_memory::GoalMemory::open_chat_with_storage_and_source_guard(
            &self.memory.request.db,
            self.memory.storage.clone(),
            self.memory.access(),
            self.memory.request.account.clone(),
            self.memory.authority.clone(),
        )?;
        Ok(Arc::new(tracking::ContextSchema::of(&reader)))
    }
    fn continuation(&self) -> Result<ChatContinuation> {
        self.lease.validate()?;
        self.memory.authority.validate()?;
        let (a, d) = attachment(&self.memory.authority, &self.attachment.session_id)?;
        ensure!(d == self.digest, "chat attachment replaced");
        authority::exclude_workspace(&self.memory.request, &a.workspace)?;
        authority::exclude_path(&a.source, &a.workspace)?;
        let bytes = authority::read(&a.source, authority::SOURCE_MAX)?;
        ensure!(
            a.initial_length <= bytes.len() as u64,
            "chat initial source truncated"
        );
        ensure!(
            source_prefix(&a, bytes.len() as u64)? == bytes
                && authority::digest(&bytes[..a.initial_length as usize]) == a.initial_digest,
            "chat source changed"
        );
        validate_source_ingress(&a, &bytes, &d)?;
        let records = recording::records(&bytes)?;
        let transcript = Session::parse(&a.source, std::str::from_utf8(&bytes)?)?;
        ensure!(
            transcript.chat_memory
                == Some(ChatMemoryMarker {
                    protocol: a.protocol.clone(),
                    conversation_id: a.session_id.clone(),
                    authority: self.memory.authority.identity.clone(),
                    attachment: d
                }),
            "chat attachment marker missing or changed"
        );
        let mut last: Option<RetainedTurnCheckpoint> = None;
        let mut endpoint = 0;
        let mut gap = false;
        let db = self.memory.memory()?;
        for ordinal in 1..=MAX_TURNS {
            let path = ordinal_path(&self.memory.authority, &a.session_id, ordinal);
            if !path.try_exists()? {
                gap = true;
                continue;
            }
            ensure!(!gap, "chat turn order has a gap");
            authority::private(&path, false)?;
            let id = String::from_utf8(authority::read(&path, 128)?)?;
            // An ordinary chat passes over a turn that began and was never
            // sealed: the recorder could not capture it, or the process
            // died in it. Its records stay in the transcript as part of
            // what the next turn continues from, and its taint with them.
            // A seal that exists and does not verify is still a refusal.
            // A turn that was retained and has since lost its seal is not
            // one of these: its record is in the store, and the chain is
            // verified through it or not continued.
            if a.open()
                && !self.memory.authority.file(&id, "chat-seal")?.try_exists()?
                && !self
                    .memory
                    .authority
                    .file(&id, "chat-retained")?
                    .try_exists()?
            {
                frozen_metadata(&self.memory.authority, &id)?;
                continue;
            }
            let (mut s,revision,f)=seal(&self.memory.authority,&id).map_err(|e|anyhow::anyhow!("chat has an unfinished or unavailable turn; retry a sealed capture or start a fresh chat: {e:#}"))?;
            ensure!(
                f.ordinal == ordinal
                    && f.previous == last.as_ref().map(|(id, _, _)| id.clone())
                    && f.prefix_start == endpoint
                    && f.start_length >= endpoint,
                "chat turn chain changed"
            );
            s.payload.binding.revision = revision;
            let receipt = db.retained_ordinary_capture(&s.payload, &self.memory.access())?;
            let recorded: CaptureReceipt =
                read_private(&self.memory.authority.file(&id, "chat-retained")?, 65536)?;
            ensure!(recorded == receipt, "chat retained receipt changed");
            endpoint = s.length;
            last = Some((id, receipt, endpoint));
        }
        // Only startup Config records may follow the last retained turn.
        let mut offset = 0usize;
        for record in records {
            let end = offset + bytes[offset..].iter().position(|b| *b == b'\n').unwrap() + 1;
            if offset >= endpoint as usize {
                match record {
                    Record::Meta(_) | Record::ChatMemory { .. } if last.is_none() => {}
                    Record::Config(c) => {
                        ensure!(
                            c.workspace.canonicalize()? == a.workspace,
                            "chat configuration workspace changed"
                        );
                        a.validate_tools(&c.tools)?;
                    }
                    // Between an ordinary chat's turns: a late result, a
                    // title, a turn passed over. The next turn starts from
                    // all of it. Never a second header or marker.
                    Record::Meta(_) | Record::ChatMemory { .. } => {
                        anyhow::bail!("chat has an unsealed tail; start a fresh chat")
                    }
                    _ if a.open() => {}
                    _ => anyhow::bail!("chat has an unsealed tail; start a fresh chat"),
                }
            }
            offset = end;
        }
        Ok((bytes, last))
    }
    pub fn next_turn_ordinal(&self) -> Result<u64> {
        let (_, last) = self.continuation()?;
        self.ordinal_after(last.as_ref())
    }
    fn ordinal_after(&self, last: Option<&RetainedTurnCheckpoint>) -> Result<u64> {
        let ordinal = if self.attachment.open() {
            // After every turn begun, sealed or passed over.
            let mut next = 1;
            while next <= MAX_TURNS
                && ordinal_path(&self.memory.authority, &self.attachment.session_id, next)
                    .try_exists()?
            {
                next += 1;
            }
            next
        } else {
            match last {
                Some((id, _, _)) => frozen(&self.memory.authority, id)?.0.ordinal + 1,
                None => 1,
            }
        };
        ensure!(
            ordinal <= MAX_TURNS,
            "chat turn limit reached; start a fresh chat"
        );
        Ok(ordinal)
    }
    /// Authenticated web hosts reserve this ordinal before preparing any provider.
    pub fn begin_authenticated_web_turn(
        &mut self,
        session: &Session,
        owner: &OwnerInput,
        expected_ordinal: u64,
    ) -> Result<ChatMemoryTurn> {
        ensure!(
            self.attachment.protocol == CHAT_WEB_MEMORY_PROTOCOL,
            "web turn requires web attachment"
        );
        self.begin_turn_for(session, owner, Some(expected_ordinal))
    }
    pub fn begin_turn(&mut self, session: &Session, owner: &OwnerInput) -> Result<ChatMemoryTurn> {
        ensure!(
            self.attachment.web_ingress.is_none(),
            "CLI turn requires CLI attachment"
        );
        self.begin_turn_for(session, owner, None)
    }
    fn begin_turn_for(
        &mut self,
        session: &Session,
        owner: &OwnerInput,
        expected_ordinal: Option<u64>,
    ) -> Result<ChatMemoryTurn> {
        ensure!(
            session.path == self.attachment.source
                && session.meta.id == self.attachment.session_id
                && session.meta.kind == Some(self.attachment.session_kind())
                && owner.source() == self.attachment.owner_source()
                && owner.text().len() <= 2048,
            "chat owner input unavailable (maximum 2048 bytes)"
        );
        let (bytes, last) = self.continuation()?;
        let ordinal = self.ordinal_after(last.as_ref())?;
        ensure!(
            expected_ordinal.is_none_or(|expected| expected == ordinal),
            "chat turn ordinal changed"
        );
        let transcript = Session::parse(&session.path, std::str::from_utf8(&bytes)?)?;
        ensure!(
            !transcript.configs.is_empty(),
            "chat memory requires recorded tool configuration"
        );
        if let Some(version) = transcript.convo.goal_context.current() {
            ensure!(
                version.owner_inputs().len() < 64
                    && version
                        .owner_inputs()
                        .iter()
                        .all(|input| input.source() == self.attachment.owner_source()),
                "chat context owner limit reached; start a fresh chat"
            );
        }
        ensure!(
            bytes.len() + owner.text().len() + 4096 < authority::SOURCE_MAX,
            "chat transcript approaches its size limit; start a fresh chat"
        );
        let f = Turn {
            attachment: self.digest.clone(),
            session_id: session.meta.id.clone(),
            ordinal,
            previous: last.as_ref().map(|(id, _, _)| id.clone()),
            prefix_start: last.as_ref().map_or(0, |(_, _, n)| *n),
            start_length: bytes.len() as u64,
            start_digest: authority::digest(&bytes),
            owner: owner.clone(),
            dependencies: last.map(|(_, r, _)| vec![r.projection]).unwrap_or_default(),
            nonce: uuid::Uuid::new_v4().to_string(),
        };
        let data = serde_json::to_vec(&f)?;
        let id = authority::digest(&data);
        authority::write(&self.memory.authority.file(&id, "chat-begin")?, &data)?;
        authority::write(
            &ordinal_path(&self.memory.authority, &f.session_id, f.ordinal),
            id.as_bytes(),
        )?;
        Ok(ChatMemoryTurn {
            id,
            authority: self.memory.authority.identity.clone(),
            attachment: self.digest.clone(),
            session_id: self.attachment.session_id.clone(),
            state: Arc::new(Mutex::new(RunState::default())),
        })
    }
    pub fn goal_memory(&self, turn: &ChatMemoryTurn) -> Result<Arc<dyn Tool>> {
        self.lease.validate()?;
        ensure!(
            turn.authority == self.memory.authority.identity
                && turn.attachment == self.digest
                && turn.session_id == self.attachment.session_id,
            "chat turn authority changed"
        );
        let mut state = turn
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("chat tracking unavailable"))?;
        ensure!(
            !state.tool_created && !state.sealed,
            "chat tool already bound"
        );
        let (f, _) = frozen(&self.memory.authority, &turn.id)?;
        self.memory
            .memory()?
            .validate_dependency_roots(&f.dependencies, &self.memory.access())?;
        let inner = crate::tool::goal_memory::GoalMemory::open_chat_with_storage_and_source_guard(
            &self.memory.request.db,
            self.memory.storage.clone(),
            self.memory.access(),
            self.memory.request.account.clone(),
            self.memory.authority.clone(),
        )?;
        state.tool_created = true;
        Ok(Arc::new(tracking::TrackedChat {
            inner,
            authority: self.memory.authority.clone(),
            id: turn.id.clone(),
            state: turn.state.clone(),
            storage: self.memory.storage.clone(),
            lease: self.lease.clone(),
        }))
    }
    pub fn finish_turn(&mut self, turn: &ChatMemoryTurn) -> ChatMemoryReport {
        if let Some(live) = turn
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .task_live
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
        {
            let _ = live.finish();
        }
        let prepared = (|| -> Result<Turn> {
            self.lease.validate()?;
            ensure!(
                turn.authority == self.memory.authority.identity
                    && turn.attachment == self.digest
                    && turn.session_id == self.attachment.session_id,
                "chat authority changed"
            );
            if self.attachment.task.is_some() {
                task_audit::recover(&self.memory, &turn.id)?;
            }
            let mut state = turn
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("chat tracking unavailable"))?;
            let (f, a) = frozen(&self.memory.authority, &turn.id)?;
            if !self
                .memory
                .authority
                .file(&turn.id, "chat-seal")?
                .try_exists()?
            {
                // A turn that was passed over stays passed over: sealing
                // it now would take in the turns begun since.
                ensure!(
                    f.ordinal == MAX_TURNS
                        || !ordinal_path(&self.memory.authority, &f.session_id, f.ordinal + 1)
                            .try_exists()?,
                    "chat turn was passed over"
                );
                let bytes = authority::read(&a.source, authority::SOURCE_MAX)?;
                source_prefix(&a, bytes.len() as u64)?;
                let attempts = self.memory.authority.attempts(&turn.id)?;
                let payload = capture::capture(&self.memory, &f, &a, &bytes, &attempts, &turn.id)?;
                let s = ChatSeal {
                    turn: turn.id.clone(),
                    length: bytes.len() as u64,
                    digest: authority::digest(&bytes),
                    attempts,
                    payload,
                };
                let data = serde_json::to_vec(&s)?;
                authority::write(&self.memory.authority.file(&turn.id, "chat-seal")?, &data)?;
                authority::write(
                    &self.memory.authority.file(&turn.id, "chat-seal-hash")?,
                    authority::digest(&data).as_bytes(),
                )?;
            }
            state.sealed = true;
            Ok(f)
        })();
        match prepared {
            Ok(f) => retry(&self.memory, &turn.id, Some(&f)),
            Err(e) => {
                tracing::warn!(error=%e,"chat retention unavailable");
                let known = frozen_metadata(&self.memory.authority, &turn.id)
                    .ok()
                    .map(|(f, _)| f);
                ChatMemoryReport::new(
                    &self.memory,
                    &turn.id,
                    known.as_ref(),
                    SessionMemoryStatus::Unavailable,
                    Some(SessionMemoryReason::RecordingUnavailable),
                    None,
                )
            }
        }
    }
}

#[cfg(test)]
mod task_tests;
