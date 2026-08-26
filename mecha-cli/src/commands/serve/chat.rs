//! Phase 2 of `mecha serve`: chat — the agent lives in the process.
//!
//! The shape is the Slack connector's, because the problem is the same: **one
//! agent, many conversations.** The agent is built once (`global_config_only`,
//! MCP servers rooted at the `web` producer directory, no skills — the
//! registry is shared state); everything per-conversation rides on a
//! [`RunContext`]: jail, approver, budget, cancel token, steering queue, and
//! an outbox route stamped with this run's session id. A conversation is
//! *moved into* the spawned run task and handed back at the end, so the map
//! stays single-writer without a lock held across a run.
//!
//! Default posture: **read-only, sends stage** — the trigger posture, which
//! makes the safe default also the useful one (reads run; mutations are
//! refused as `Blocked`, machine policy, never mined as a correction;
//! outbox-routed sends still stage, because staging executes nothing). A
//! session flipped to `ask` gets the live approver instead — see
//! [`super::present`], where a tool call becomes a card on the page and a
//! deny-with-reason is a real user correction. `allow` is deliberately not
//! offered from the page.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::{Context as _, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;
use tokio::sync::{broadcast, Mutex};
use tokio_util::sync::CancellationToken;

use mecha_core::agent::{Agent, AgentEvent, Conversation};
use mecha_core::config::{Config, PermissionMode};
use mecha_core::message::{Block, Message, Role, Usage};
use mecha_core::outbox::{OutboxRoute, OutboxStore};
use mecha_core::session::{Record, RunConfig, Session, SessionMeta};
use mecha_core::tool::ToolCtx;
use mecha_core::work;

use crate::{setup, GlobalOpts};

/// The default conversation — what the chat tab opens onto.
pub(super) const DEFAULT_SESSION: &str = "main";

/// A session key becomes a directory name under the producer root, so it is
/// validated like a producer name: short, lowercase, no path in it. The
/// check is containment, not politeness — a key is model-adjacent input the
/// moment a page script can choose it.
pub(super) fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 32
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && !key.starts_with(['-', '_'])
}

/// The sync routing table `WebAsker` reads: session key → question plumbing.
/// Separate from the async sessions map so a tool call can route without an
/// async lock.
type QuestionRoutes = Arc<
    StdMutex<
        HashMap<
            String,
            (
                super::present::Questions,
                broadcast::Sender<WireEvent>,
                Option<Arc<mecha_core::questions::ParkingAsker>>,
            ),
        >,
    >,
>;

pub struct ChatState {
    agent: Arc<Agent>,
    routes: QuestionRoutes,
    config: Config,
    provider_name: String,
    model: String,
    context_window: Option<u64>,
    outbox_root: PathBuf,
    sessions: Mutex<HashMap<String, WebSession>>,
    /// The todo tool the shared agent is using, so a resumed session can have
    /// its plan restored from its own transcript (D15). One handle, many
    /// sessions: the list is keyed by each run's jail, which for this surface
    /// is the session key's directory.
    todo: Option<Arc<mecha_core::tool::todo::TodoTool>>,
    /// Dropping an MCP client kills its server; held for the process lifetime.
    _mcp: Vec<Arc<mecha_core::mcp::McpClient>>,
}

struct WebSession {
    /// `None` while a run holds it — the move-and-return that keeps the map
    /// single-writer (the Slack connector's pattern).
    conversation: Option<Conversation>,
    session: Arc<Session>,
    workspace: PathBuf,
    live: Option<Live>,
    /// Outlives any one run, so a page can subscribe before, during, after.
    events: broadcast::Sender<WireEvent>,
    /// Last reported usage, for the context gauge on a fresh page load.
    last_usage: Arc<StdMutex<Option<Usage>>>,
    /// Tools this session's runs may not dispatch, whatever the shared
    /// registry holds.
    ///
    /// **D6, in a process that has one agent for every conversation.** A
    /// spawned `mecha tasks work` keeps *the agent may not close its own
    /// task* by taking `kg_task_update` off its own private registry; there is
    /// no private registry here, so the rule rides on the run instead. Per
    /// session and not per process, or opening one task conversation would
    /// narrow every other chat with it.
    withheld: Arc<[String]>,
    /// The board task this conversation is about, when it is about one.
    task: Option<String>,
    /// Permission posture for this session's runs. Read-only is the default
    /// (the trigger posture: reads run, sends stage); `ask` turns tool calls
    /// into live approval cards. `allow` is deliberately not offered from
    /// the page yet. Shared with the running approver, so a change lands on
    /// the run's next call (the Slack mode-cell pattern).
    mode: Arc<StdMutex<PermissionMode>>,
    /// Outstanding approval/ask cards for this session.
    questions: super::present::Questions,
    /// Was the last turn spoken? Since D3 typed and spoken turns share one
    /// conversation, so "does this turn need the voice block" is no longer
    /// answerable from the messages — a spoken turn and a typed one look
    /// identical once recorded. False on create and on resume: injecting
    /// one extra copy of the block costs a few hundred cached tokens,
    /// where omitting it costs a markdown reply read aloud.
    last_turn_spoken: bool,
}

struct Live {
    cancel: CancellationToken,
    queue: Arc<StdMutex<VecDeque<String>>>,
}

impl ChatState {
    /// Build the in-process agent. Fails loudly; the caller degrades the
    /// surface to dashboard-only and says so, rather than serving a chat
    /// door that silently cannot answer.
    pub async fn build() -> Result<Self> {
        let opts = GlobalOpts {
            // Global config only, like a trigger run and the Slack
            // connector: a project's mecha.toml must not shape a run someone
            // drives from their phone.
            global_config_only: true,
            // MCP servers are spawned once with the agent and cannot follow
            // a per-session jail; rooting them at the producer directory
            // means every session's jail is a subdirectory of where the
            // servers already point (the limitation is real and written
            // down in the Slack section of CLAUDE.md).
            workspace: Some(producer_root()?),
            // The registry belongs to the agent and one agent serves every
            // session — a loaded skill would be shared state across them.
            no_skills: true,
            ..GlobalOpts::default()
        };
        // Not interactive: no terminal approver — and then `ask_user` IS
        // registered, against the Slack connector's precedent, because this
        // front-end can do what that one could not: route the question to
        // the human who owns the run that asked (the jail's directory name
        // is the session key; see `present::WebAsker`). An unanswered card
        // resolves as the tool's measured decline, never a guess.
        let mut prepared = setup::prepare(&opts, false).await?;
        let routes: QuestionRoutes = Arc::default();
        let lookup: super::present::SessionLookup = {
            let routes = Arc::clone(&routes);
            Arc::new(move |key: &str| routes.lock().ok().and_then(|m| m.get(key).cloned()))
        };
        prepared
            .agent
            .registry_mut()
            .insert(Arc::new(mecha_core::tool::ask::AskUserTool::new(Arc::new(
                super::present::WebAsker { lookup },
            ))));
        let outbox_root = match prepared.config.outbox.dir.clone() {
            Some(dir) => dir,
            None => OutboxStore::default_root()?,
        };
        let context_window = prepared
            .config
            .providers
            .get(&prepared.provider_name)
            .and_then(|p| p.context_window);
        Ok(Self {
            agent: Arc::new(prepared.agent),
            routes,
            provider_name: prepared.provider_name.clone(),
            model: prepared.model.clone(),
            context_window,
            outbox_root,
            sessions: Mutex::new(HashMap::new()),
            todo: prepared.todo,
            config: prepared.config,
            _mcp: prepared._mcp.clone(),
        })
    }
}

impl ChatState {
    /// What the mounted voice facade needs from the shared build — the
    /// unification seam: one agent, one prefix, two dialects
    /// (docs/VOICE-RESEARCH.md, the serve unification entry).
    pub fn voice_parts(
        &self,
    ) -> (
        Arc<Agent>,
        String,
        String,
        mecha_core::config::Config,
        PathBuf,
    ) {
        (
            Arc::clone(&self.agent),
            self.provider_name.clone(),
            self.model.clone(),
            self.config.clone(),
            self.outbox_root.clone(),
        )
    }
}

fn producer_root() -> Result<PathBuf> {
    let root = work::producer_dir("web")?;
    std::fs::create_dir_all(&root).with_context(|| format!("creating {}", root.display()))?;
    Ok(root)
}

/// One producer, per-session subdirectories — so `work clean`'s retention
/// retires whole old sessions (the Slack thread pattern).
pub(super) fn session_workspace(key: &str) -> Result<PathBuf> {
    let dir = work::producer_dir("web")?.join(key);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    work::ensure_outside_mecha_home(&dir)?;
    Ok(dir)
}

// ---------------------------------------------------------------------------
// The wire: what the page receives, over SSE and in the transcript read.
// Pure functions of core types, so they are testable without a server.

/// [`mecha_core::outbox::DraftView`] on the wire.
///
/// A projection rather than a `Serialize` on the type itself: the store's
/// shape is the store's, and a page is not the only thing that reads it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WireDraft {
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    pub other: Vec<(String, String)>,
}

/// How much of one field a card carries, and why there is a bound at all.
///
/// The arguments beside it have been clipped since the card existed; the
/// shaped view has to be, for the same reason and one more. `content` is a
/// body field, so a `fs_write` of a large file would put the whole file in
/// the card — broadcast to every open page, held for the reload path, and
/// re-sent on every transcript read after that. It would also push Allow
/// and Deny off the bottom of a phone, which is the surface this is for.
///
/// A clip is marked where it cuts. The exact bytes were never here to begin
/// with — that is what the session record is — and a card claiming to show
/// a whole call it silently shortened is the failure this shaping exists to
/// prevent, arriving one layer in.
const CARD_BODY_CHARS: usize = 2_000;
const CARD_VALUE_CHARS: usize = 400;

fn clip_to(text: String, max: usize) -> String {
    if text.chars().count() <= max {
        return text;
    }
    let head: String = text.chars().take(max).collect();
    format!("{head}… (clipped — the whole call is in the session record)")
}

impl WireDraft {
    /// `None` when the call has nothing a person would read as a message —
    /// an empty object, or a card that is prose already. A card with no
    /// shape falls back to the arguments, which is what it always showed.
    pub fn of(input: &serde_json::Value) -> Option<WireDraft> {
        let view = mecha_core::outbox::DraftView::of(input);
        if view.headers.is_empty() && view.body.is_none() && view.other.is_empty() {
            return None;
        }
        let clip_fields = |fields: Vec<(String, String)>| {
            fields
                .into_iter()
                .map(|(k, v)| (k, clip_to(v, CARD_VALUE_CHARS)))
                .collect()
        };
        Some(WireDraft {
            headers: clip_fields(view.headers),
            body: view.body.map(|b| clip_to(b, CARD_BODY_CHARS)),
            other: clip_fields(view.other),
        })
    }
}

/// The one spelling of a permission mode on the wire.
///
/// The state read seeds the page's chip and [`WireEvent::Mode`] keeps it
/// honest afterwards; both go through here, because two spellings of one
/// mode is a chip that disagrees with the run it describes. The page
/// switches on these exact strings.
/// The inverse of [`mode_wire`], and the only place a mode is parsed.
///
/// The pair has to round-trip: a mode the page can be *told* about but not
/// *ask for* is what the surface shipped with — the state read answered
/// `"allow"` and the POST refused it — so the chip could render a mode no
/// tap could reach. `read-only` is accepted alongside the canonical spelling
/// because the hyphen is what a person types by hand.
pub(super) fn parse_mode(raw: &str) -> Option<PermissionMode> {
    match raw {
        "ask" => Some(PermissionMode::Ask),
        "allow" => Some(PermissionMode::Allow),
        "read_only" | "read-only" => Some(PermissionMode::ReadOnly),
        _ => None,
    }
}

pub(super) fn mode_wire(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Ask => "ask",
        PermissionMode::Allow => "allow",
        PermissionMode::ReadOnly => "read_only",
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireEvent {
    Delta {
        text: String,
    },
    /// A turn started with words the page did not type — today that means
    /// spoken (D3). A typed send echoes locally, so broadcasting it too
    /// would render it twice on the page that sent it; what a second device
    /// watching the same session misses is a separate gap, and this is not
    /// the place to half-close it.
    User {
        text: String,
    },
    Queued {
        text: String,
    },
    Tool {
        name: String,
    },
    ToolResult {
        name: String,
        is_error: bool,
    },
    Denied {
        name: String,
        reason: String,
    },
    Usage {
        prompt_tokens: u64,
        output_tokens: u64,
        context_window: Option<u64>,
    },
    Notice {
        text: String,
    },
    /// The session's permission mode, whenever it changes. Structured, and
    /// deliberately not left to the notice beside it: the chip this drives
    /// tells a person whether the next write stops to ask, so deriving it
    /// from prose would break the first time the sentence was reworded —
    /// and a stale chip is a person believing a run is gated when it is not.
    Mode {
        mode: String,
    },
    Question {
        qid: u64,
        kind: String,
        tool: Option<String>,
        args: Option<String>,
        /// The call as a person reads one, when there is a shape to read —
        /// `None` for an `ask_user` card, which is already prose.
        ///
        /// The arguments stay in `args` beside it and nothing is dropped:
        /// this is the *essentials*, and the reviewer expands to the whole
        /// call. The outbox's rule, arriving where it was always needed —
        /// an approval card is a review surface, and one that prints
        /// `{"body_markdown": "Dear Dirk,\n\nThank…"}` is asking for a
        /// decision it has made hard to take.
        draft: Option<WireDraft>,
        question: Option<String>,
        options: Vec<String>,
        timeout_secs: u64,
    },
    QuestionDone {
        qid: u64,
    },
    /// This run staged drafts, and here is what it staged.
    ///
    /// Ids only, deliberately: the page fetches each from `/api/outbox/{id}`,
    /// so the card it renders is built from the *store* rather than from
    /// whatever the event carried. That is the same rule the release path
    /// already follows — a reviewer reading one thing while approving
    /// another is the failure this whole surface exists to prevent — and it
    /// costs one request per draft, on a queue that is almost always one.
    ///
    /// Sent before `Done`: the offer is part of how the run ended, not a
    /// thing that happens after it.
    Staged {
        ids: Vec<String>,
    },
    Done {
        ok: bool,
        stop: Option<String>,
        taint_private: bool,
        taint_untrusted: bool,
        error: Option<String>,
    },
}

/// Which agent events reach the page, and as what.
///
/// Thinking never leaves the box (the pump's rule: broadcasting a scratchpad
/// invites reading it as the answer), `AssistantText` is redundant with the
/// deltas that already streamed, and nested subagent events are collapsed to
/// their own surface later — Phase 2 shows the parent run.
fn wire_event(event: &AgentEvent, context_window: Option<u64>) -> Option<WireEvent> {
    match event {
        AgentEvent::TextDelta(text) => Some(WireEvent::Delta { text: text.clone() }),
        AgentEvent::QueuedInput(text) => Some(WireEvent::Queued { text: text.clone() }),
        AgentEvent::ToolCall { name, .. } => Some(WireEvent::Tool { name: name.clone() }),
        AgentEvent::ToolResult { name, is_error, .. } => Some(WireEvent::ToolResult {
            name: name.clone(),
            is_error: *is_error,
        }),
        AgentEvent::ToolDenied { name, reason } => Some(WireEvent::Denied {
            name: name.clone(),
            reason: reason.clone(),
        }),
        AgentEvent::TurnUsage(usage) => Some(WireEvent::Usage {
            prompt_tokens: usage.input_tokens
                + usage.cache_read_input_tokens
                + usage.cache_creation_input_tokens,
            output_tokens: usage.output_tokens,
            context_window,
        }),
        AgentEvent::Compacted {
            messages_before,
            messages_after,
            ..
        } => Some(WireEvent::Notice {
            text: format!("compacted {messages_before} messages to {messages_after}"),
        }),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Entry {
    User {
        text: String,
    },
    Assistant {
        text: String,
    },
    Tool {
        name: String,
        is_error: Option<bool>,
    },
}

/// The transcript a fresh page load renders, derived from the conversation —
/// the messages are the record; this is a rendering, never a second store.
/// The voice facade prefixes its spoken-style preamble onto the first user
/// turn of every call. For a person re-reading the conversation it is
/// harness plumbing, not their words, so display strips it — keyed on the
/// constant, with a paragraph-cut fallback for transcripts recorded under
/// an earlier revision of the block. Display only: the record keeps it.
fn strip_voice_preamble(text: &str) -> &str {
    match text.strip_prefix(crate::voice::VOICE_BLOCK) {
        Some(rest) => rest.trim_start(),
        None if text.starts_with("Voice mode:") => text
            .split_once("\n\n")
            .map(|(_, rest)| rest.trim_start())
            .unwrap_or(""),
        None => text,
    }
}

fn transcript_entries(messages: &[Message]) -> Vec<Entry> {
    let mut names: HashMap<String, String> = HashMap::new();
    let mut entries = Vec::new();
    for message in messages {
        match message.role {
            Role::User => {
                let mut text = String::new();
                for block in &message.content {
                    match block {
                        Block::Text { text: t } => {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(strip_voice_preamble(t));
                        }
                        Block::ToolResult {
                            tool_use_id,
                            is_error,
                            ..
                        } => {
                            let name = names
                                .get(tool_use_id)
                                .cloned()
                                .unwrap_or_else(|| "tool".into());
                            entries.push(Entry::Tool {
                                name,
                                is_error: Some(*is_error),
                            });
                        }
                        Block::Image { .. } => {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str("[image]");
                        }
                        _ => {}
                    }
                }
                if !text.is_empty() {
                    entries.push(Entry::User { text });
                }
            }
            Role::Assistant => {
                for block in &message.content {
                    match block {
                        Block::Text { text } => {
                            if !text.trim().is_empty() {
                                entries.push(Entry::Assistant { text: text.clone() });
                            }
                        }
                        Block::ToolUse { id, name, .. } => {
                            names.insert(id.clone(), name.clone());
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    entries
}

// ---------------------------------------------------------------------------
// Handlers.

#[derive(serde::Deserialize)]
pub struct SendBody {
    pub text: String,
}

type Chat = State<super::WebState>;

// The Err arm carries a whole `Response`; it is built once per refused
// request, so the size is irrelevant next to the allocation it wraps.
#[allow(clippy::result_large_err)]
fn chat_state(state: &super::WebState) -> Result<&Arc<ChatState>, axum::response::Response> {
    state.chat.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "chat is not available: the agent failed to build at startup — see the serve log\n",
        )
            .into_response()
    })
}

/// Open (or re-open) the conversation about a board task, and return its
/// session id.
///
/// **One chat surface, not a second one shaped like it.** Everything the
/// owner asked for here — voice, uploads, watching it work, answering its
/// questions, steering it mid-run — already exists on a chat session, so a
/// task conversation *is* a chat session with three differences: its jail is
/// the task's own work directory (the same producer directory a spawned
/// `tasks work` uses, so the two agree about where a relative path points),
/// its title is `task: …`, and it withholds `kg_task_update` (D6).
///
/// Idempotent by key, so tapping twice returns the conversation rather than
/// minting a twin — one conversation, one writer, the rule this file already
/// enforces two other ways.
pub(super) async fn open_task_conversation(
    state: &super::WebState,
    task_id: &str,
) -> anyhow::Result<String> {
    let chat = chat_state(state)
        .map_err(|_| anyhow::anyhow!("this process has no chat agent configured"))?
        .clone();

    // The record, through the same door the board page reads it through.
    let board = super::review::self_json(state, &["tasks", "list", "--closed", "--json"]).await?;
    let task = board["items"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .find(|t| t["id"].as_str() == Some(task_id))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no such task: {task_id}"))?;
    let name = task["name"].as_str().unwrap_or("(unnamed)").to_string();

    let key = task_key(task_id);
    let workspace = mecha_core::work::ensure(task_id)?;
    let seed = crate::commands::tasks::discuss_prompt(
        &task,
        board["today"].as_str().unwrap_or_default(),
        &crate::commands::tasks::Reach::of(chat.agent.registry()),
    );

    let (session_id, fresh) = {
        let mut sessions = chat.sessions.lock().await;
        let ws = ensure_session_as(
            &chat,
            &mut sessions,
            &key,
            SessionInit {
                workspace: Some(workspace),
                title: Some(format!("{TASK_TITLE_PREFIX}{name}")),
                withheld: vec!["kg_task_update".to_string()],
                task: Some(task_id.to_string()),
            },
        )?;
        let fresh = ws
            .conversation
            .as_ref()
            .is_some_and(|c| c.messages.is_empty());
        (ws.session.meta.id.clone(), fresh)
    };

    // **The link the card offers as "open the conversation".** Written by the
    // harness, never by the model (D5) — and without moving the status, because
    // `waiting_on` says who has the ball and while the owner is reading this,
    // they do. That is also why the card no longer vanishes out of the view it
    // was tapped in.
    let _ = super::review::verb_output(state, &["tasks", "set", task_id, "--session", &session_id])
        .await;

    // The model speaks first, but only on a conversation that has not started:
    // re-opening is picking a conversation back up, and re-seeding it would
    // restate the brief over the top of whatever was agreed.
    if fresh {
        let mut sessions = chat.sessions.lock().await;
        let _ = begin_turn(
            &chat,
            &mut sessions,
            &key,
            &seed,
            TurnOpts {
                spoken: false,
                approve_all: false,
            },
        );
    }
    Ok(session_id)
}

/// Release a task's conversation so a detached run can take it over.
///
/// **A transfer of the single writer, not a copy.** The conversation is
/// already on disk; what this process holds is the right to append to it. So
/// handing over is: refuse if a run is in flight here (a conversation cannot
/// be in two loops at once), drop the session out of the map, and let the
/// child load the same transcript — messages *and* taint, so the change of
/// hands cannot launder what was read. The run marker the child writes is
/// what stops this process reclaiming it, and `resume` answers 409 until it
/// is done.
///
/// The question routing goes with it. A card this process is still willing to
/// show for a conversation it no longer owns is a card nobody will ever
/// answer, and its 120-second expiry would resolve as a refusal inside
/// somebody else's run.
pub(super) async fn release_task_conversation(
    state: &super::WebState,
    task_id: &str,
) -> anyhow::Result<String> {
    let chat = chat_state(state)
        .map_err(|_| anyhow::anyhow!("this process has no chat agent configured"))?
        .clone();
    let key = task_key(task_id);
    let mut sessions = chat.sessions.lock().await;
    let ws = sessions
        .get(&key)
        .ok_or_else(|| anyhow::anyhow!("no open conversation for {task_id}"))?;
    if ws.live.is_some() {
        // Not a race to win: the loop owns the message list right now, and a
        // hand-over mid-turn would take a transcript out from under it.
        anyhow::bail!(
            "it is working right now — let it finish, or stop it first, then hand it over"
        );
    }
    let id = ws.session.meta.id.clone();
    sessions.remove(&key);
    if let Ok(mut routes) = chat.routes.lock() {
        routes.remove(&key);
    }
    Ok(id)
}

/// A task's conversation key: stable, so tapping twice reaches one
/// conversation, and derived from the id rather than stored anywhere.
fn task_key(task_id: &str) -> String {
    let tail: String = task_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("t-{}", tail.to_ascii_lowercase())
}

/// What a task conversation may not call, from its recorded title.
///
/// D6 — *the agent may not close its own task* — enforced by absence rather
/// than by asking. The harness keeps its own handle on `kg_task_update` and
/// moves the status itself; the model working the task simply has no tool
/// that does, exactly as a spawned `mecha tasks work` arranges by taking it
/// off a private registry.
pub(super) fn task_withholding(title: &Option<String>) -> Vec<String> {
    match title {
        Some(t) if t.starts_with(TASK_TITLE_PREFIX) => vec!["kg_task_update".to_string()],
        _ => Vec::new(),
    }
}

/// The prefix the drawer filters on and `task_withholding` reads.
pub(super) const TASK_TITLE_PREFIX: &str = "task: ";

/// What a session should be created *as*, when it does not exist yet.
///
/// Only a task conversation needs any of this today, and it is threaded
/// through the one constructor rather than given a second one: two creation
/// sites is how two kinds of session silently stop agreeing about the jail,
/// the recorded config, or the question routing.
#[derive(Default)]
pub(super) struct SessionInit {
    pub workspace: Option<PathBuf>,
    pub title: Option<String>,
    pub withheld: Vec<String>,
    pub task: Option<String>,
}

fn ensure_session<'a>(
    chat: &Arc<ChatState>,
    sessions: &'a mut HashMap<String, WebSession>,
    key: &str,
) -> Result<&'a mut WebSession> {
    ensure_session_as(chat, sessions, key, SessionInit::default())
}

fn ensure_session_as<'a>(
    chat: &Arc<ChatState>,
    sessions: &'a mut HashMap<String, WebSession>,
    key: &str,
    init: SessionInit,
) -> Result<&'a mut WebSession> {
    if !sessions.contains_key(key) {
        let workspace = match init.workspace {
            Some(w) => {
                std::fs::create_dir_all(&w)?;
                w
            }
            None => session_workspace(key)?,
        };
        let session = Session::create(
            &Session::default_dir()?,
            SessionMeta {
                id: Session::new_id(),
                created_at: chrono::Utc::now(),
                provider: chat.provider_name.clone(),
                model: chat.model.clone(),
                workspace: workspace.clone(),
                // `task: …` for a delegation, so the drawer's task filter
                // and `runlog` see it as the delegation it is rather than as
                // an unrelated web chat that happens to mention one.
                title: Some(init.title.clone().unwrap_or_else(|| format!("web: {key}"))),
            },
        )?;
        session.append(&Record::Config(RunConfig::of(
            &chat.agent,
            &chat.config,
            &chat.provider_name,
        )))?;
        let (events, _) = broadcast::channel(512);
        let questions = super::present::Questions::default();
        // **Only a delegation parks.** A question in an ordinary chat is
        // asked of somebody who is reading; a question in a task
        // conversation may be asked at four in the morning of a run nobody
        // is watching, and the honest ending for that one is a stored
        // question and a stopped run rather than a decline the model then
        // reasons around.
        let park = init.task.as_ref().and_then(|task| {
            let store = mecha_core::questions::QuestionStore::default_root()
                .and_then(mecha_core::questions::QuestionStore::open)
                .ok()?;
            Some(Arc::new(mecha_core::questions::ParkingAsker::new(
                Arc::new(store),
                session.meta.id.clone(),
                Some(task.clone()),
            )))
        });
        if let Ok(mut routes) = chat.routes.lock() {
            routes.insert(key.to_string(), (questions.clone(), events.clone(), park));
        }
        sessions.insert(
            key.to_string(),
            WebSession {
                conversation: Some(Conversation::new()),
                session: Arc::new(session),
                workspace,
                live: None,
                events,
                last_usage: Arc::new(StdMutex::new(None)),
                mode: Arc::new(StdMutex::new(PermissionMode::ReadOnly)),
                questions,
                last_turn_spoken: false,
                withheld: Arc::from(init.withheld),
                task: init.task,
            },
        );
    }
    Ok(sessions.get_mut(key).expect("just inserted"))
}

/// GET /api/chat/{key} — what a fresh page load renders.
pub async fn transcript(
    State(state): Chat,
    axum::extract::Path(key): axum::extract::Path<String>,
) -> axum::response::Response {
    let chat = match chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    if !valid_key(&key) {
        return (StatusCode::BAD_REQUEST, "bad session key\n").into_response();
    }
    let mut sessions = chat.sessions.lock().await;
    let ws = match ensure_session(chat, &mut sessions, &key) {
        Ok(ws) => ws,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response(),
    };
    let running = ws.live.is_some();
    let (entries, taint) = match &ws.conversation {
        Some(convo) => (transcript_entries(&convo.messages), Some(convo.taint)),
        // A run holds the conversation; the page catches up over SSE.
        None => (Vec::new(), None),
    };
    let usage = ws.last_usage.lock().ok().and_then(|u| u.clone());
    let mode = ws.mode.lock().map(|m| mode_wire(*m)).unwrap_or("read_only");
    Json(serde_json::json!({
        "session": ws.session.meta.id,
        "model": chat.model,
        "mode": mode,
        "running": running,
        "questions": ws.questions.cards(),
        "held_by_run": ws.conversation.is_none(),
        // The plan, live. Keyed by this session's jail (D14), so one shared
        // agent hands back the right list — and readable *while a run holds
        // the conversation*, which is exactly when it is worth looking at.
        // The page refetches when the SSE stream announces a `todo` result;
        // no new event type is needed because every write already shows up
        // there as a tool call.
        "todo": chat
            .todo
            .as_ref()
            .map(|t| t.items_in(&ws.workspace))
            .unwrap_or_default(),
        "entries": entries,
        "taint": taint.map(|t| serde_json::json!({
            "private": t.private, "untrusted": t.untrusted,
        })),
        "usage": usage.map(|u| serde_json::json!({
            "prompt_tokens": u.input_tokens + u.cache_read_input_tokens
                + u.cache_creation_input_tokens,
            "context_window": chat.context_window,
        })),
    }))
    .into_response()
}

/// POST /api/chat/{key}/send — start a run, or steer the one in flight.
pub async fn send(
    State(state): Chat,
    axum::extract::Path(key): axum::extract::Path<String>,
    Json(body): Json<SendBody>,
) -> axum::response::Response {
    let chat = match chat_state(&state) {
        Ok(c) => c.clone(),
        Err(resp) => return resp,
    };
    if !valid_key(&key) {
        return (StatusCode::BAD_REQUEST, "bad session key\n").into_response();
    }
    let text = body.text.trim().to_string();
    if text.is_empty() {
        return (StatusCode::BAD_REQUEST, "empty message\n").into_response();
    }

    let mut sessions = chat.sessions.lock().await;
    let ws = match ensure_session(&chat, &mut sessions, &key) {
        Ok(ws) => ws,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response(),
    };

    // A run in flight: this is steering, folded into the tool-results turn by
    // the loop (never a bare user message — two in a row are invalid).
    if let Some(live) = &ws.live {
        if let Ok(mut queue) = live.queue.lock() {
            queue.push_back(text);
            return Json(serde_json::json!({ "steered": true })).into_response();
        }
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "steering queue poisoned\n",
        )
            .into_response();
    }

    match begin_turn(
        &chat,
        &mut sessions,
        &key,
        &text,
        TurnOpts {
            spoken: false,
            approve_all: false,
        },
    ) {
        Ok(_started) => Json(serde_json::json!({ "started": true })).into_response(),
        Err(TurnError::Held) => (
            StatusCode::CONFLICT,
            "conversation is held by a finished run still landing\n",
        )
            .into_response(),
        Err(TurnError::Failed(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}\n")).into_response()
        }
    }
}

/// Which door a turn came through, and what that changes about it.
struct TurnOpts {
    /// Spoken rather than typed (D3). Two consequences and no others: the
    /// D10 voice block opens the turn when the last one was typed, and the
    /// user's words are broadcast because there is no page-side echo.
    spoken: bool,
    /// `--voice-yes`: run with approvals off. Deliberately a property of
    /// the *turn*, not of the conversation — a typed turn in the same
    /// session still runs at whatever the page's mode says, and nothing
    /// here is sticky. Decision A, 2026-08-25: the flag already owned this
    /// risk in writing, and unification was meant to change which
    /// transcript a turn lands in, not what a turn may do. Everything
    /// structural is untouched: the interlock sits ahead of the approver,
    /// sends still stage through the outbox, and taint now accumulates
    /// across both doors instead of being reset by opening a call.
    approve_all: bool,
}

/// Why a turn could not start.
enum TurnError {
    /// The conversation is not there to take — a finished run still landing.
    Held,
    Failed(String),
}

/// A started turn, for a caller that wants to follow it rather than fire
/// and forget. The typed door drops all three, which costs nothing: a tap
/// with no receiver is a send that fails, and the run never notices.
struct Started {
    events: tokio::sync::mpsc::UnboundedReceiver<AgentEvent>,
    done: tokio::sync::oneshot::Receiver<Result<crate::voice::HostedAnswer, String>>,
    cancel: CancellationToken,
}

/// Start a turn on a session that is idle and still holds its conversation.
///
/// One implementation, two doors — typed through `send`, spoken through
/// [`VoiceHost::speak`] — because two constructions of "a run on a web
/// session" is how the two silently stop agreeing about the jail, the
/// outbox stamp or the recording contract. The caller holds the sessions
/// lock across this whole call, which is what keeps the map single-writer
/// (the Slack connector's pattern).
fn begin_turn(
    chat: &Arc<ChatState>,
    sessions: &mut HashMap<String, WebSession>,
    key: &str,
    text: &str,
    opts: TurnOpts,
) -> Result<Started, TurnError> {
    let ws = sessions
        .get_mut(key)
        .ok_or_else(|| TurnError::Failed("no such session".into()))?;

    let text = if opts.spoken {
        crate::voice::open_spoken_turn(text, ws.last_turn_spoken)
    } else {
        text.to_string()
    };

    let Some(mut conversation) = ws.conversation.take() else {
        return Err(TurnError::Held);
    };
    ws.last_turn_spoken = opts.spoken;

    // Append the user message to the record *before* the run — the
    // `record_run` contract: `before` is what the file already holds.
    let user = Message::user(&text);
    conversation.push(user.clone());
    if let Err(e) = ws.session.append(&Record::Message(user)) {
        // Refuse to run a turn the record did not accept: an unrecorded run
        // is invisible to distill, recall and the run-quality corpus.
        conversation.messages.pop();
        ws.conversation = Some(conversation);
        return Err(TurnError::Failed(format!("recording: {e:#}")));
    }
    let before = conversation.messages.clone();

    // A typed turn is echoed by the page that typed it; a spoken one has no
    // local echo anywhere, so it is announced — with the voice block
    // stripped, which is harness plumbing and not the owner's words.
    if opts.spoken {
        let _ = ws.events.send(WireEvent::User {
            text: strip_voice_preamble(&text).to_string(),
        });
    }

    let cancel = CancellationToken::new();
    let queue: Arc<StdMutex<VecDeque<String>>> = Arc::default();
    ws.live = Some(Live {
        cancel: cancel.clone(),
        queue: Arc::clone(&queue),
    });

    // Per-run context on the shared agent: jail, approver, budget, cancel,
    // steering, and an outbox route stamped with this session's id.
    let mut cx = (**chat.agent.context()).clone();
    cx.tools = Arc::new(ToolCtx {
        workspace: ws.workspace.clone(),
        ..(*chat.agent.ctx()).clone()
    });
    cx.approver = if opts.approve_all {
        Arc::new(mecha_core::tool::ModeApprover {
            mode: PermissionMode::Allow,
        }) as Arc<dyn mecha_core::tool::Approver>
    } else {
        Arc::new(super::present::WebApprover {
            mode: Arc::clone(&ws.mode),
            questions: ws.questions.clone(),
            events: ws.events.clone(),
            timeout: super::present::APPROVAL_TIMEOUT,
        })
    };
    // **A delegation's ceiling, not a chat's.** These two numbers are the
    // only thing standing between a run and forever, and neither was chosen
    // for the other's work: forty round trips is generous for a conversation
    // and short for a task, where stopping at the limit reports as
    // `MaxTurns` and reads to the owner as the model giving up. Note this is
    // an override rather than a minimum — `budget.max_turns.unwrap_or(cfg)`
    // — so a machine whose `[agent] max_turns` is tightened for chat does not
    // silently tighten delegations with it.
    if cx.budget.max_turns.is_none() {
        cx.budget.max_turns = Some(match ws.task {
            Some(_) => crate::commands::tasks::TASK_MAX_TURNS,
            None => 40,
        });
    }
    cx.cancel = Some(cancel.clone());
    cx.queued_input = Some(Arc::clone(&queue));
    // Whatever this session may not dispatch. Empty for an ordinary chat, so
    // the assignment costs nothing and there is one place it is applied.
    cx.withheld = Arc::clone(&ws.withheld);
    if let Some(shared) = &chat.agent.context().outbox {
        if let Ok(store) = OutboxStore::open(&chat.outbox_root) {
            let mine = OutboxRoute::new(
                store,
                shared.routed().map(String::from).collect::<Vec<_>>(),
                shared.publishes().map(String::from).collect::<Vec<_>>(),
            );
            mine.set_session_id(&ws.session.meta.id);
            cx.outbox = Some(Arc::new(mine));
        }
    }

    // What was already waiting, before this run staged anything. Taken here
    // rather than inside the task so it is genuinely a *before* — a run that
    // stages in its first second must not find its own draft in its baseline.
    // `None` when the store could not be read: no baseline, no offer (see
    // `review_policy::staged_since`).
    let outbox_baseline: Option<std::collections::HashSet<String>> =
        OutboxStore::open(&chat.outbox_root)
            .ok()
            .and_then(|store| store.items().ok())
            .map(|items| {
                items
                    .into_iter()
                    .filter(|i| i.status == "pending")
                    .map(|i| i.id)
                    .collect()
            });
    let outbox_root = chat.outbox_root.clone();

    let agent = Arc::clone(&chat.agent);
    let key_for_task = key.to_string();
    let session = Arc::clone(&ws.session);
    let bcast = ws.events.clone();
    let last_usage = Arc::clone(&ws.last_usage);
    let context_window = chat.context_window;
    let state_for_task = Arc::clone(chat);
    let (tap_tx, tap_rx) = tokio::sync::mpsc::unbounded_channel();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();

    tokio::spawn(async move {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let forwarder = {
            let bcast = bcast.clone();
            let last_usage = Arc::clone(&last_usage);
            tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    if let AgentEvent::TurnUsage(usage) = &event {
                        if let Ok(mut slot) = last_usage.lock() {
                            *slot = Some(usage.clone());
                        }
                    }
                    // The private tap: a caller streaming this run somewhere
                    // of its own (the voice facade, answering the worker).
                    let _ = tap_tx.send(event.clone());
                    if let Some(wire) = wire_event(&event, context_window) {
                        let _ = bcast.send(wire);
                    }
                }
            })
        };

        let outcome = agent.run_in(&cx, &mut conversation, Some(tx)).await;
        let _ = forwarder.await;

        let _ = session.record_run(&before, &conversation);
        if let Ok(o) = &outcome {
            let _ = session.record_outcome(o);
        }
        let _ = session.append(&Record::Taint(conversation.taint));

        // Leftover steering that never reached a drain point: dropped with a
        // notice rather than silently (auto-resubmit is the TUI's move and
        // needs a recursion this phase does not want).
        if let Ok(queue) = queue.lock() {
            if !queue.is_empty() {
                let _ = bcast.send(WireEvent::Notice {
                    text: format!(
                        "{} queued message(s) arrived too late for this run — send again",
                        queue.len()
                    ),
                });
            }
        }

        // `ReviewMode::Now`, which the TUI and Slack have had all along and
        // this surface never did: a draft you just asked for is a draft you
        // are about to read, so the run's own drafts are put in front of you
        // instead of only incrementing a badge. Offered however the run
        // ended — an interrupted run's drafts are exactly the ones worth
        // looking at — because only `Auto` may act unattended.
        if let Some(baseline) = &outbox_baseline {
            let staged = OutboxStore::open(&outbox_root)
                .ok()
                .and_then(|store| store.items().ok())
                .map(|items| crate::review_policy::staged_since(items, baseline))
                .unwrap_or_default();
            if !staged.is_empty() {
                let _ = bcast.send(WireEvent::Staged {
                    ids: staged.into_iter().map(|i| i.id).collect(),
                });
            }
        }

        let taint = conversation.taint;
        let done = match &outcome {
            Ok(o) => WireEvent::Done {
                ok: true,
                stop: Some(format!("{:?}", o.stop_cause)),
                taint_private: taint.private,
                taint_untrusted: taint.untrusted,
                error: None,
            },
            Err(e) => WireEvent::Done {
                ok: false,
                stop: None,
                taint_private: taint.private,
                taint_untrusted: taint.untrusted,
                error: Some(format!("{e:#}")),
            },
        };

        // Hand the conversation back, then announce the end — a stream left
        // open is indistinguishable from a run still working.
        let mut sessions = state_for_task.sessions.lock().await;
        if let Some(ws) = sessions.get_mut(&key_for_task) {
            let outcome_err = outcome.is_err();
            if outcome_err {
                // A failed request must not leave a dangling user message the
                // next request would resend (the chat command's rule).
                conversation.messages.pop();
            }
            ws.conversation = Some(conversation);
            ws.live = None;
        }
        drop(sessions);
        let _ = bcast.send(done);
        // Last, deliberately: a caller told "answered" before the
        // conversation is back would find it held on its very next turn.
        let _ = done_tx.send(match outcome {
            Ok(o) => Ok(crate::voice::HostedAnswer {
                text: o.text,
                input_tokens: o.usage.input_tokens,
                output_tokens: o.usage.output_tokens,
            }),
            Err(e) => Err(format!("{e:#}")),
        });
    });

    Ok(Started {
        events: tap_rx,
        done: done_rx,
        cancel,
    })
}

/// How long a spoken turn will wait for a run in flight to yield, in 100 ms
/// tries. The facade's own slot map waits exactly as long, because the
/// thing being waited on is the same: a tool call, which cancellation never
/// interrupts mid-call.
const BARGE_IN_TRIES: usize = 200;

/// `ChatState` as something a voice call can speak into (D3).
///
/// A wrapper rather than an impl on `ChatState` itself, because starting a
/// turn needs an owned `Arc<ChatState>` to hand the spawned run — the same
/// handle `send` clones out of axum's state.
pub struct VoiceHost(pub Arc<ChatState>);

#[async_trait::async_trait]
impl crate::voice::SessionHost for VoiceHost {
    async fn speak(&self, key: &str, utterance: &str, approve_all: bool) -> crate::voice::Hosted {
        use crate::voice::Hosted;
        // Containment stays with the side that owns the filesystem: a
        // session key becomes a directory name under the producer root, and
        // it is model-adjacent input the moment a page script can choose it.
        if !valid_key(key) {
            return Hosted::Unknown;
        }
        for _ in 0..BARGE_IN_TRIES {
            {
                let mut sessions = self.0.sessions.lock().await;
                let (live, idle) = match ensure_session(&self.0, &mut sessions, key) {
                    Ok(ws) => (ws.live.is_some(), ws.conversation.is_some()),
                    Err(e) => return Hosted::Failed(format!("{e:#}")),
                };
                if live {
                    // Barge-in, the facade's own contract, and deliberately
                    // not steering: steering folds the words into the run
                    // already streaming, whose output goes to the page —
                    // and the worker is owed a reply it can speak. Order
                    // matters exactly as it does in `cancel`: a run parked
                    // on an approval card never sees the token, so the
                    // cards are dropped first.
                    if let Some(ws) = sessions.get_mut(key) {
                        ws.questions.drain();
                        if let Some(live) = &ws.live {
                            live.cancel.cancel();
                        }
                    }
                } else if idle {
                    match begin_turn(
                        &self.0,
                        &mut sessions,
                        key,
                        utterance,
                        TurnOpts {
                            spoken: true,
                            approve_all,
                        },
                    ) {
                        Ok(started) => {
                            return Hosted::Started(Box::new(crate::voice::HostedTurn {
                                events: started.events,
                                done: started.done,
                                cancel: started.cancel,
                            }))
                        }
                        // Held: a finished run still landing. Fall through
                        // to the sleep — another try costs 100 ms and
                        // usually finds the conversation back.
                        Err(TurnError::Held) => {}
                        Err(TurnError::Failed(e)) => return Hosted::Failed(e),
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        Hosted::Busy
    }
}

/// POST /api/chat/{key}/cancel — stop at the next safe point, keep the partial.
pub async fn cancel(
    State(state): Chat,
    axum::extract::Path(key): axum::extract::Path<String>,
) -> axum::response::Response {
    let chat = match chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let sessions = chat.sessions.lock().await;
    match sessions.get(&key) {
        Some(ws) if ws.live.is_some() => {
            // Order matters: a run parked in `approve()` or `ask_user` never
            // sees the token, so the pending cards are dropped first — each
            // resolves as a machine refusal — and the token stops the rest.
            ws.questions.drain();
            if let Some(live) = &ws.live {
                live.cancel.cancel();
            }
            Json(serde_json::json!({ "cancelled": true })).into_response()
        }
        _ => Json(serde_json::json!({ "cancelled": false })).into_response(),
    }
}

/// GET /api/chat/{key}/events — the run, streamed. Subscribing is legal at
/// any time; a subscriber that falls behind gets a notice, never silence.
pub async fn events(
    State(state): Chat,
    axum::extract::Path(key): axum::extract::Path<String>,
) -> axum::response::Response {
    let chat = match chat_state(&state) {
        Ok(c) => c.clone(),
        Err(resp) => return resp,
    };
    if !valid_key(&key) {
        return (StatusCode::BAD_REQUEST, "bad session key\n").into_response();
    }
    let rx = {
        let mut sessions = chat.sessions.lock().await;
        match ensure_session(&chat, &mut sessions, &key) {
            Ok(ws) => ws.events.subscribe(),
            Err(e) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response()
            }
        }
    };
    let stream = futures::stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Ok(wire) => {
                let event = SseEvent::default().json_data(&wire).ok()?;
                Some((Ok::<_, std::convert::Infallible>(event), rx))
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                let notice = WireEvent::Notice {
                    text: format!("{n} events missed — reload for the full transcript"),
                };
                let event = SseEvent::default().json_data(&notice).ok()?;
                Some((Ok(event), rx))
            }
            Err(broadcast::error::RecvError::Closed) => None,
        }
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

#[derive(serde::Deserialize)]
pub struct AnswerBody {
    pub qid: u64,
    /// approval cards: allow / deny (+ optional reason)
    pub allow: Option<bool>,
    pub reason: Option<String>,
    /// ask cards: the answer text, or decline
    pub answer: Option<String>,
    pub decline: Option<bool>,
}

/// POST /api/chat/{key}/answer — one endpoint for both card kinds.
pub async fn answer(
    State(state): Chat,
    axum::extract::Path(key): axum::extract::Path<String>,
    Json(body): Json<AnswerBody>,
) -> axum::response::Response {
    let chat = match chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let sessions = chat.sessions.lock().await;
    let Some(ws) = sessions.get(&key) else {
        return (StatusCode::NOT_FOUND, "no such session\n").into_response();
    };
    let answer = if body.decline == Some(true) {
        super::present::Answer::Decline
    } else if let Some(text) = body.answer {
        super::present::Answer::Text(text)
    } else if body.allow == Some(true) {
        super::present::Answer::Approve
    } else if body.allow == Some(false) {
        super::present::Answer::Deny(body.reason.unwrap_or_default())
    } else {
        return (
            StatusCode::BAD_REQUEST,
            "an answer needs allow, answer, or decline\n",
        )
            .into_response();
    };
    if ws.questions.answer(body.qid, answer) {
        Json(serde_json::json!({ "ok": true })).into_response()
    } else {
        // Already answered from another device, expired, or never existed —
        // the card is gone either way, and the page should drop it.
        (StatusCode::GONE, "that question is no longer waiting\n").into_response()
    }
}

#[derive(serde::Deserialize)]
pub struct ModeBody {
    pub mode: String,
}

/// POST /api/chat/{key}/mode — read_only | ask | allow, an explicit control
/// only: a mode is never inferred from anything the model or a page said.
///
/// `allow` is sticky like the other two, and the argument for that is what
/// it does *not* waive. The interlock runs ahead of the approver, so a
/// conversation holding private data and third-party text still refuses an
/// `external_send` tool with nobody asked; outbox-routed calls still stage
/// rather than send. What `allow` removes is the tap, which is the part a
/// person who set it deliberately is choosing to stop giving. A mode that
/// decayed on a timer would instead put the tap back at an unpredictable
/// moment — silently, on a page nobody is looking at — and "why did that
/// run park for two minutes" is the same unanswerable question the chip
/// staleness bug produced.
pub async fn set_mode(
    State(state): Chat,
    axum::extract::Path(key): axum::extract::Path<String>,
    Json(body): Json<ModeBody>,
) -> axum::response::Response {
    let chat = match chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let mode = match parse_mode(&body.mode) {
        Some(mode) => mode,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                format!("unknown mode {:?}\n", body.mode),
            )
                .into_response()
        }
    };
    let mut sessions = chat.sessions.lock().await;
    let ws = match ensure_session(chat, &mut sessions, &key) {
        Ok(ws) => ws,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response(),
    };
    // A lock we could not take is a mode that did not change, so it must
    // not be announced as one — that is the staleness bug in miniature, and
    // it lies in the dangerous direction in both: a chip reading `allow`
    // over a run still parking on every write, or reading `read_only` over
    // one that is not. The approver falls back to `ReadOnly` on the same
    // poisoned lock, so refusing here leaves the page and the run agreeing.
    match ws.mode.lock() {
        Ok(mut cell) => *cell = mode,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "the mode could not be changed; this session is still running at \
                 whatever it was\n",
            )
                .into_response()
        }
    }
    // Both, and they are not redundant: the event is the state every open
    // page reconciles its chip against — the reason a change made on a
    // phone reaches the laptop watching the same session — and the notice
    // is the line in the transcript saying a person changed it and when.
    let _ = ws.events.send(WireEvent::Mode {
        mode: mode_wire(mode).to_string(),
    });
    let _ = ws.events.send(WireEvent::Notice {
        text: format!("mode set to {}", mode_wire(mode)),
    });
    Json(serde_json::json!({ "ok": true })).into_response()
}

/// The first user line of a transcript, for a history listing — the thing
/// a person recognises a conversation by. Bounded: a transcript can be
/// megabytes, and a listing that reads whole files is a listing nobody
/// opens twice. `None` means no user ever spoke — a shell created by a page
/// load — and the listing skips it.
fn first_user_snippet(path: &std::path::Path) -> Option<String> {
    use std::io::BufRead;
    let file = std::fs::File::open(path).ok()?;
    let reader = std::io::BufReader::new(file);
    for line in reader.lines().take(300) {
        let line = line.ok()?;
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v["record"] == "message" && v["role"] == "user" {
            for block in v["content"].as_array().into_iter().flatten() {
                if let Some(text) = block["text"].as_str() {
                    let text = strip_voice_preamble(text).trim();
                    if !text.is_empty() {
                        let head: String = text.chars().take(140).collect();
                        return Some(head);
                    }
                }
            }
        }
    }
    None
}

/// GET /api/history — recorded web and voice sessions from the store,
/// newest first: what the drawer's "earlier" section lists. A row carries
/// the session id (what resume takes), when it started, which door it came
/// through, its first user line, and — when this process already holds it —
/// the live key, so the drawer never offers to resume a conversation into a
/// second copy of itself.
pub async fn history(State(state): Chat) -> axum::response::Response {
    let chat = match chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let attached: std::collections::HashMap<String, String> = {
        let sessions = chat.sessions.lock().await;
        sessions
            .iter()
            .map(|(k, ws)| (ws.session.meta.id.clone(), k.clone()))
            .collect()
    };
    let dir = match Session::default_dir() {
        Ok(d) => d,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response(),
    };
    let mut metas = match Session::list(&dir) {
        Ok(m) => m,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response(),
    };
    metas.retain(|(m, _)| {
        m.title
            .as_deref()
            // D10 — a run the owner cannot find is a run they will start
            // twice, and a task run is the one they are most likely to want
            // back: it was started from a card and left a question or a
            // draft behind.
            .is_some_and(|t| {
                t.starts_with("web: ") || t.starts_with("voice: ") || t.starts_with("task: ")
            })
    });
    metas.sort_by_key(|(m, _)| std::cmp::Reverse(m.created_at));
    let mut rows = Vec::new();
    for (meta, path) in metas {
        if rows.len() >= 40 {
            break;
        }
        let Some(snippet) = first_user_snippet(&path) else {
            continue; // a page load that never spoke is not a conversation
        };
        let kind = match meta.title.as_deref() {
            Some(t) if t.starts_with("voice") => "voice",
            Some(t) if t.starts_with("task: ") => "task",
            _ => "web",
        };
        rows.push(serde_json::json!({
            "id": meta.id,
            "kind": kind,
            "created_at": meta.created_at.to_rfc3339(),
            "snippet": snippet,
            "attached_key": attached.get(&meta.id),
        }));
    }
    Json(serde_json::json!({ "sessions": rows })).into_response()
}

#[derive(serde::Deserialize)]
pub struct ResumeBody {
    pub id: String,
}

/// A live key for a resumed transcript: the id's unique tail, which is
/// already lowercase hex and therefore already a valid key.
fn resume_key(id: &str) -> String {
    let tail: String = id
        .chars()
        .rev()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(10)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("r-{}", tail.to_ascii_lowercase())
}

/// POST /api/resume — pick a recorded conversation back up, exactly as
/// `mecha chat --resume` does: `Session::load` restores the messages AND
/// the taint (recorded so resuming cannot launder it), appends continue in
/// the same transcript, and a config record marks the pickup. The response
/// is the live key the page should switch to; resuming a session this
/// process already holds returns that key rather than minting a twin — one
/// conversation must never have two writers.
pub async fn resume(State(state): Chat, Json(body): Json<ResumeBody>) -> axum::response::Response {
    let chat = match chat_state(&state) {
        Ok(c) => c.clone(),
        Err(resp) => return resp,
    };
    let dir = match Session::default_dir() {
        Ok(d) => d,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response(),
    };
    let mut sessions = chat.sessions.lock().await;
    if let Some((k, _)) = sessions
        .iter()
        .find(|(_, ws)| ws.session.meta.id == body.id)
    {
        return Json(serde_json::json!({ "key": k })).into_response();
    }
    // **And the same question asked of the other processes.** The check above
    // is this process's own, and a delegated run is a *detached child*: its
    // `Conversation` lives in memory nothing here shares, and the transcript
    // has one writer. Resuming a run still in flight would give that JSONL
    // two — the child appending its turns and this process appending the
    // owner's — which is the failure the in-process check exists to prevent,
    // arriving through the one door it cannot see. The marker names the
    // session it is writing, and a marker whose process is gone is swept on
    // the way past, so a crashed run does not lock its transcript out.
    if let Some(task) = crate::commands::tasks::markers()
        .ok()
        .and_then(|m| m.live_writer_of(&body.id))
    {
        return (
            StatusCode::CONFLICT,
            format!(
                "a run is working {task} in this transcript — stop it first \
                 (`mecha tasks stop {task}`), or steer it where it is\n"
            ),
        )
            .into_response();
    }
    let path = match Session::find(&dir, &body.id) {
        Ok(p) => p,
        Err(e) => return (StatusCode::NOT_FOUND, format!("{e:#}\n")).into_response(),
    };
    let (meta, conversation) = match Session::load(&path) {
        Ok(x) => x,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response(),
    };
    // The recorded jail, re-proved: a transcript names the workspace its
    // tool calls resolved against, and continuing it anywhere else would be
    // the outbox's wrong-bytes release through another door. Still checked
    // against the mecha home, because the record is data, not authority.
    let workspace = meta.workspace.clone();
    if let Err(e) = std::fs::create_dir_all(&workspace)
        .map_err(anyhow::Error::from)
        .and_then(|()| mecha_core::work::ensure_outside_mecha_home(&workspace))
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response();
    }
    // D15 — the plan comes back with the conversation, and two things then
    // read it: the transcript response serves it to the page, and
    // `carried_state` carries it across a compaction so a resumed session that
    // summarises itself keeps its real plan rather than nothing.
    if let Some(todo) = &chat.todo {
        if let Some(n) = todo.rehydrate(&workspace, &conversation.messages) {
            tracing::debug!(items = n, "restored the resumed session's todo list");
        }
    }
    let key = resume_key(&meta.id);
    if !valid_key(&key) || sessions.contains_key(&key) {
        return (
            StatusCode::CONFLICT,
            "could not mint a key for this session\n",
        )
            .into_response();
    }
    let withheld = task_withholding(&meta.title);
    let session = Session { meta, path };
    // On resume as on create: a session picked up under different flags
    // should say so in its own record.
    if let Err(e) = session.append(&Record::Config(RunConfig::of(
        &chat.agent,
        &chat.config,
        &chat.provider_name,
    ))) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response();
    }
    let (events, _) = broadcast::channel(512);
    let questions = super::present::Questions::default();
    if let Ok(mut routes) = chat.routes.lock() {
        // A resumed task transcript parks too — the title is what says it is
        // one, the same signal `task_withholding` reads for D6.
        let park = (!withheld.is_empty())
            .then(|| {
                let store = mecha_core::questions::QuestionStore::default_root()
                    .and_then(mecha_core::questions::QuestionStore::open)
                    .ok()?;
                Some(Arc::new(mecha_core::questions::ParkingAsker::new(
                    Arc::new(store),
                    session.meta.id.clone(),
                    None,
                )))
            })
            .flatten();
        routes.insert(key.clone(), (questions.clone(), events.clone(), park));
    }
    sessions.insert(
        key.clone(),
        WebSession {
            conversation: Some(conversation),
            session: Arc::new(session),
            workspace,
            live: None,
            events,
            last_usage: Arc::new(StdMutex::new(None)),
            mode: Arc::new(StdMutex::new(PermissionMode::ReadOnly)),
            questions,
            last_turn_spoken: false,
            // **A resumed delegation is still a delegation.** D6 is a
            // property of the conversation, not of how it was opened, so
            // picking a task transcript back up must not hand back the tool
            // that closes the task. The title is the signal because the
            // harness is its only writer — `tasks work` and the task chat
            // both stamp `task: …`, and nothing lets a model set one.
            withheld: Arc::from(withheld),
            task: None,
        },
    );
    Json(serde_json::json!({ "key": key })).into_response()
}

/// GET /api/sessions — the rail: live sessions in this process, the default
/// first, then by name. (Resuming a recorded session from disk is a later
/// wiring; the rail lists what the process holds, honestly.)
pub async fn sessions(State(state): Chat) -> axum::response::Response {
    let chat = match chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let sessions = chat.sessions.lock().await;
    let mut rows: Vec<serde_json::Value> = sessions
        .iter()
        .map(|(key, ws)| {
            serde_json::json!({
                "key": key,
                "id": ws.session.meta.id,
                "title": ws.session.meta.title,
                "running": ws.live.is_some(),
                "taint": ws.conversation.as_ref().map(|c| serde_json::json!({
                    "private": c.taint.private, "untrusted": c.taint.untrusted,
                })),
                "turns": ws.conversation.as_ref().map(|c| c.len()),
            })
        })
        .collect();
    rows.sort_by_key(|r| {
        (
            r["key"] != DEFAULT_SESSION,
            r["key"].as_str().unwrap_or("").to_string(),
        )
    });
    Json(serde_json::json!({ "sessions": rows })).into_response()
}

#[cfg(test)]
mod tests {
    #[test]
    fn session_keys_that_could_leave_the_producer_dir_are_refused() {
        use super::valid_key;
        for bad in [
            "../x",
            "a/b",
            "",
            "-lead",
            "UPPER",
            "dot.dot",
            &"x".repeat(33),
        ] {
            assert!(!valid_key(bad), "{bad:?} must be refused");
        }
        for good in ["main", "grant-review", "walk_2", "a"] {
            assert!(valid_key(good), "{good:?} should pass");
        }
    }
}

#[cfg(test)]
mod wire_tests {
    use super::*;
    use mecha_core::agent::AgentEvent;

    #[test]
    fn thinking_never_reaches_the_wire() {
        assert_eq!(
            wire_event(&AgentEvent::ThinkingDelta("secret".into()), None),
            None
        );
    }

    #[test]
    fn usage_reports_the_whole_prompt_not_just_uncached_input() {
        let usage = Usage {
            input_tokens: 8,
            output_tokens: 100,
            cache_creation_input_tokens: 2_000,
            cache_read_input_tokens: 18_000,
        };
        let wire = wire_event(&AgentEvent::TurnUsage(usage), Some(32_768)).unwrap();
        match wire {
            WireEvent::Usage { prompt_tokens, .. } => assert_eq!(prompt_tokens, 20_008),
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn transcript_names_a_tool_result_from_its_call() {
        let messages = vec![
            Message {
                role: Role::User,
                content: vec![Block::Text { text: "hi".into() }],
            },
            Message {
                role: Role::Assistant,
                content: vec![
                    Block::Text {
                        text: "looking".into(),
                    },
                    Block::ToolUse {
                        id: "t1".into(),
                        name: "mail_search".into(),
                        input: serde_json::json!({}),
                    },
                ],
            },
            Message {
                role: Role::User,
                content: vec![Block::ToolResult {
                    tool_use_id: "t1".into(),
                    content: "3 results".into(),
                    is_error: false,
                }],
            },
        ];
        let entries = transcript_entries(&messages);
        assert_eq!(
            entries,
            vec![
                Entry::User { text: "hi".into() },
                Entry::Assistant {
                    text: "looking".into()
                },
                Entry::Tool {
                    name: "mail_search".into(),
                    is_error: Some(false),
                },
            ]
        );
    }

    #[test]
    fn what_a_spoken_turn_opens_with_is_exactly_what_display_strips() {
        // Two modules, one convention: `voice::open_spoken_turn` writes the
        // preamble and `strip_voice_preamble` takes it off. Since D3 they
        // meet in one conversation, so a drift between them would render
        // harness plumbing to the owner as their own words — or, worse,
        // eat the first paragraph of what they actually said.
        let opened = crate::voice::open_spoken_turn("what is on my calendar", false);
        assert_eq!(strip_voice_preamble(&opened), "what is on my calendar");
        // And a turn that carries no preamble is passed through untouched.
        let plain = crate::voice::open_spoken_turn("and tomorrow?", true);
        assert_eq!(strip_voice_preamble(&plain), "and tomorrow?");
    }

    #[test]
    fn a_spoken_turn_reads_back_as_the_owners_words() {
        let messages = vec![Message {
            role: Role::User,
            content: vec![Block::Text {
                text: crate::voice::open_spoken_turn("book the room", false),
            }],
        }];
        assert_eq!(
            transcript_entries(&messages),
            vec![Entry::User {
                text: "book the room".into()
            }]
        );
    }

    #[test]
    fn a_spoken_turn_reaches_the_page_under_the_name_the_page_switches_on() {
        // The page's SSE handler keys on the literal `"user"`; a rename
        // here would leave spoken turns arriving and nothing rendering
        // them, which looks exactly like voice not being wired at all.
        let wire = serde_json::to_value(WireEvent::User {
            text: "book the room".into(),
        })
        .unwrap();
        assert_eq!(wire["type"], "user");
        assert_eq!(wire["text"], "book the room");
    }

    #[test]
    fn every_mode_the_page_can_be_told_about_is_one_it_can_ask_for() {
        // The bug this names shipped: the state read answered "allow" and
        // the POST refused it, so a mode was renderable and unreachable.
        // Renderer and parser are two matches over one enum, which is
        // exactly the pair that drifts, so the round-trip is asserted
        // rather than remembered.
        use super::{mode_wire, parse_mode};
        for mode in [
            PermissionMode::Ask,
            PermissionMode::Allow,
            PermissionMode::ReadOnly,
        ] {
            assert_eq!(
                parse_mode(mode_wire(mode)),
                Some(mode),
                "{} does not round-trip",
                mode_wire(mode)
            );
        }
        assert_eq!(parse_mode("read-only"), Some(PermissionMode::ReadOnly));
        assert_eq!(parse_mode("plan"), None, "an unknown mode must not resolve");
    }

    #[test]
    fn a_mode_change_reaches_the_page_under_the_name_the_page_switches_on() {
        // The chip keys on the literal `"mode"` and reads `ev.mode`; a
        // rename here leaves every open page showing the mode it had when
        // it loaded, which is the staleness this event exists to end — and
        // it looks exactly like the feature working on the tab that tapped.
        let wire = serde_json::to_value(WireEvent::Mode {
            mode: super::mode_wire(PermissionMode::Allow).to_string(),
        })
        .unwrap();
        assert_eq!(wire["type"], "mode");
        assert_eq!(wire["mode"], "allow");
    }

    #[test]
    fn an_approval_card_leads_with_the_essentials_and_keeps_the_whole_call() {
        // What a person needs to answer "may I?" about a meeting is its
        // name and when it is — not an argument map in alphabetical order,
        // where an event reads end before start.
        let event = serde_json::json!({
            "title": "B4 brown bag",
            "start_time": "2026-08-28T12:00:00-04:00",
            "end_time": "2026-08-28T13:30:00-04:00",
            "account": "dartmouth",
            "attendees": ["menghan@example.edu"],
        });
        let draft = super::WireDraft::of(&event).expect("a calendar call has a shape");
        let heads: Vec<&str> = draft.headers.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(heads, ["title", "start_time", "end_time", "account"]);
        // Nothing the reviewer would need is only in the expansion, and
        // nothing is dropped: what is not an essential is still carried.
        assert!(draft.other.iter().any(|(k, _)| k == "attendees"));

        // A letter's essentials are its addressing; its body is the prose,
        // with real newlines rather than escape sequences.
        let mail = serde_json::json!({
            "to": "dirk@example.edu",
            "subject": "Re: the letter",
            "body": "Dear Dirk,\n\nThank you for the note.",
        });
        let draft = super::WireDraft::of(&mail).expect("a mail call has a shape");
        assert_eq!(draft.headers[0], ("to".into(), "dirk@example.edu".into()));
        assert!(draft.body.as_deref().unwrap().contains('\n'));
    }

    #[test]
    fn a_large_write_does_not_put_the_whole_file_on_the_card() {
        // `content` is a body field, so the unbounded version of this put a
        // whole file into an event broadcast to every page and re-sent on
        // every transcript read after that — and pushed the buttons off a
        // phone. The arguments beside it were clipped all along.
        let big = "x".repeat(50_000);
        let draft = super::WireDraft::of(&serde_json::json!({
            "path": "notes.md",
            "content": big,
        }))
        .unwrap();
        let body = draft.body.unwrap();
        assert!(
            body.chars().count() < 2_200,
            "body was {} chars",
            body.chars().count()
        );
        assert!(
            body.ends_with("in the session record)"),
            "a clip has to say so"
        );
    }

    #[test]
    fn a_card_with_no_shape_still_shows_everything_it_ever_did() {
        // The fallback matters more than the shaping: an unanticipated tool
        // must degrade to the raw arguments, never to an empty card that
        // reads as "nothing to see" over a call that does something.
        assert_eq!(super::WireDraft::of(&serde_json::json!({})), None);
        let odd = super::WireDraft::of(&serde_json::json!({"pattern": "*.rs"})).unwrap();
        assert_eq!(odd.other, vec![("pattern".to_string(), "*.rs".to_string())]);
    }

    #[test]
    fn a_tool_result_only_message_adds_no_empty_user_entry() {
        let messages = vec![Message {
            role: Role::User,
            content: vec![Block::ToolResult {
                tool_use_id: "t9".into(),
                content: "…".into(),
                is_error: true,
            }],
        }];
        let entries = transcript_entries(&messages);
        assert_eq!(
            entries,
            vec![Entry::Tool {
                name: "tool".into(),
                is_error: Some(true),
            }]
        );
    }
}
