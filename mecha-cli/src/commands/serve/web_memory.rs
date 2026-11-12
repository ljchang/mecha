//! Memory for ordinary web chats.
//!
//! One store for the server, one attachment per conversation. A typed turn
//! is begun before its message is recorded and sealed after its taint
//! record; a turn that memory cannot begin or capture is passed over and
//! the conversation goes on. The registry is shared by every conversation,
//! so the recall tool in it is a router that answers each call with the
//! reader bound to the conversation's own workspace.
use anyhow::{Context, Result};
use mecha_core::{
    goal_context::OwnerInput,
    grant::Grant,
    memory_session::{
        chat::{ChatAttachMode, ChatMemory, ChatMemoryReport, ChatMemoryTurn},
        router::RecallRouter,
        SessionMemory, SessionMemoryRequest, SessionMemoryStatus,
    },
    session::Session,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub(super) const OPEN_WEB_PROTOCOL: &str =
    mecha_core::memory_session::chat::CHAT_OPEN_WEB_MEMORY_PROTOCOL;

pub(super) struct OrdinaryMemory {
    path: PathBuf,
    request: SessionMemoryRequest,
    pub(super) router: Arc<RecallRouter>,
    /// By session id. A conversation is in here from the moment its memory
    /// attaches until the server stops; the attachment holds its lease.
    chats: Mutex<HashMap<String, Arc<Mutex<ChatMemory>>>>,
}

/// What a run carries from its start to its end.
pub(super) struct Remembered {
    chat: Arc<Mutex<ChatMemory>>,
    turn: ChatMemoryTurn,
    workspace: PathBuf,
}

impl OrdinaryMemory {
    /// `root` is the directory every web chat's workspace lives under; the
    /// store may not be inside it.
    pub(super) fn open(path: &Path, dynamic_goals: bool, root: &Path) -> Result<Arc<Self>> {
        anyhow::ensure!(
            dynamic_goals,
            "remembering web chats requires --dynamic-goals"
        );
        let path = std::path::absolute(path)?;
        let request = SessionMemory::load_request(&path)?;
        let schema = SessionMemory::open(&request, root)
            .context("opening the memory store for web chats")?
            .recall_schema()?;
        Ok(Arc::new(Self {
            path,
            request,
            router: RecallRouter::of(schema.as_ref()),
            chats: Mutex::new(HashMap::new()),
        }))
    }
    /// Attach a conversation: a new one before anything but its header is
    /// recorded, or one this store already holds.
    pub(super) fn attach(&self, session: &Session, workspace: &Path, resumed: bool) -> Result<()> {
        SessionMemory::validate_request_path(&self.path, workspace)?;
        let chat = SessionMemory::open(&self.request, workspace)?.attach_open_web_chat(
            session,
            if resumed {
                ChatAttachMode::Resume
            } else {
                ChatAttachMode::Create
            },
        )?;
        self.chats
            .lock()
            .map_err(|_| anyhow::anyhow!("web memory unavailable"))?
            .insert(session.meta.id.clone(), Arc::new(Mutex::new(chat)));
        Ok(())
    }
    fn chat(&self, session: &Session) -> Option<Arc<Mutex<ChatMemory>>> {
        self.chats.lock().ok()?.get(&session.meta.id).cloned()
    }
    /// Before a turn starts: a grant that reaches the store, its key or
    /// the transcript refuses the turn itself, since the next tool call
    /// could read or rewrite them.
    pub(super) fn grants_may_run(&self, session: &Session, grants: &[Grant]) -> Result<()> {
        match self.chat(session) {
            Some(chat) => chat
                .lock()
                .map_err(|_| anyhow::anyhow!("web memory unavailable"))?
                .validate_grants(grants),
            None => Ok(()),
        }
    }
    /// Begin a typed turn. `None` when this conversation has no memory or
    /// the turn cannot begin; the turn then runs unremembered, with no
    /// reader bound for it.
    pub(super) fn begin(
        &self,
        session: &Session,
        workspace: &Path,
        owner: Option<&OwnerInput>,
    ) -> Option<Remembered> {
        // Whatever the last turn left bound is not this turn's.
        self.router.release(workspace);
        let chat = self.chat(session)?;
        let owner = owner?;
        let begun = (|| -> Result<ChatMemoryTurn> {
            let mut memory = chat
                .lock()
                .map_err(|_| anyhow::anyhow!("web memory unavailable"))?;
            let turn = memory.begin_open_web_turn(session, owner)?;
            self.router.bind(workspace, memory.goal_memory(&turn)?)?;
            Ok(turn)
        })();
        match begun {
            Ok(turn) => Some(Remembered {
                chat,
                turn,
                workspace: workspace.to_path_buf(),
            }),
            Err(error) => {
                tracing::warn!(session = %session.meta.id, "this turn will not be remembered: {error:#}");
                self.router.release(workspace);
                None
            }
        }
    }
    /// Seal the turn. Called once its taint record is the last line of the
    /// transcript, and before anything else is appended.
    pub(super) fn finish(&self, remembered: Remembered) -> Option<ChatMemoryReport> {
        self.router.release(&remembered.workspace);
        let report = remembered
            .chat
            .lock()
            .ok()
            .map(|mut memory| memory.finish_turn(&remembered.turn))?;
        if report.status == SessionMemoryStatus::Retained {
            tracing::debug!(conversation = %report.conversation_id, turn = report.turn_ordinal, "turn remembered");
        } else {
            tracing::warn!(
                conversation = %report.conversation_id,
                turn = report.turn_ordinal,
                reason = ?report.reason,
                "turn not remembered; the conversation goes on"
            );
        }
        Some(report)
    }
}

/// Whether a transcript's memory marker is an ordinary web chat's. A hint
/// for routing a resume or labelling a row, read from the head of the file:
/// attaching is what authenticates the marker.
pub(super) fn open_marked(path: &Path) -> bool {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = Vec::new();
    if file.take(65_536).read_to_end(&mut head).is_err() {
        return false;
    }
    let mut markers = String::from_utf8_lossy(&head)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|record| record["record"] == "chat_memory")
        .map(|record| record["marker"]["protocol"] == OPEN_WEB_PROTOCOL)
        .collect::<Vec<_>>();
    markers.len() == 1 && markers.pop() == Some(true)
}
