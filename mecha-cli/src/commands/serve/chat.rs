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
    /// The board task this conversation is about, when it is about one —
    /// the record as the board returned it, kept so the page can head the
    /// conversation with the goal and its details without a second trip.
    /// "I can't see what the goal is" was a complaint about a chat that knew
    /// perfectly well and did not say.
    task: Option<serde_json::Value>,
    /// Permission posture for this session's runs. Read-only is the default
    /// (the trigger posture: reads run, sends stage); `ask` turns tool calls
    /// into live approval cards. `allow` is deliberately not offered from
    /// the page yet. Shared with the running approver, so a change lands on
    /// the run's next call (the Slack mode-cell pattern).
    mode: Arc<StdMutex<PermissionMode>>,
    /// Outstanding approval/ask cards for this session.
    questions: super::present::Questions,
    /// How many owner turns this conversation had when it was last named.
    ///
    /// Zero for one that has never been: a session is created under a key,
    /// and a key is an address rather than a name. The schedule that reads
    /// this is `title::due` — front-loaded, three names in a session's life,
    /// so a long conversation does not spend a generation per turn moving a
    /// label. Not persisted: on a resume the recorded name comes back with
    /// the transcript, and re-earning it once is cheaper than another record
    /// that can disagree with the one beside it.
    titled_at: usize,
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
            // down in the Slack section of docs/ARCHITECTURE.md).
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
        /// The result's opening, capped — enough for a person to see what
        /// the model was just handed without the page holding a transcript
        /// of its own. Third-party content rides in here (a mail body, a
        /// web page), so the page must render it as TEXT, never markup.
        preview: String,
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
    /// §6.2's readout — how the just-finished run appraised. Sent only when
    /// the label is not `Neutral` (the overwhelming common case per the
    /// rung 7 corpus), so the page's logo tint has a plain absence to fall
    /// back to rather than a stream of `"neutral"` events saying nothing.
    Affect {
        label: String,
    },
    /// This conversation has a name now — see `mecha_core::title`. Sent when
    /// a run's end earns one, carrying the name without its `web: ` prefix,
    /// which is a storage detail no header should render. The page reloads
    /// the rail rather than trusting this string into its own state: the
    /// rail is where every other surface reads a session's name, and two
    /// paths to one label is how a header and a drawer row start disagreeing.
    Titled {
        title: String,
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
        AgentEvent::ToolResult {
            name,
            is_error,
            content,
            ..
        } => Some(WireEvent::ToolResult {
            name: name.clone(),
            is_error: *is_error,
            preview: result_preview(content),
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
        /// What the tool answered, capped by [`result_preview`] — the page
        /// shows the tool's own words behind a tap, because a chip that
        /// says only `graph__kg_search` is a claim the reader cannot check.
        #[serde(skip_serializing_if = "Option::is_none")]
        preview: Option<String>,
    },
}

/// The first stretch of a tool result, char-safe, with the cut declared.
/// 1,500 chars is a screenful on a phone: enough to see what came back,
/// small enough that a transcript of forty calls stays a page, not a dump.
fn result_preview(content: &str) -> String {
    const CAP: usize = 1_500;
    if content.chars().count() <= CAP {
        return content.to_string();
    }
    let head: String = content.chars().take(CAP).collect();
    let rest = content.chars().count() - CAP;
    format!(
        "{head}
… (+{rest} more chars)"
    )
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
                            content,
                        } => {
                            let name = names
                                .get(tool_use_id)
                                .cloned()
                                .unwrap_or_else(|| "tool".into());
                            entries.push(Entry::Tool {
                                name,
                                is_error: Some(*is_error),
                                preview: Some(result_preview(content)),
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
                // **Pick the recorded conversation back up.** The board has
                // held this link since the conversation was opened, and it is
                // what makes a task chat survive a restart of this process:
                // the map is a cache, the transcript is the record. Refused
                // by `Session::find` if it is gone, which falls back to a new
                // conversation rather than to an error — a task whose
                // transcript was swept still deserves to be talked about.
                resume_from: task["session"].as_str().map(str::to_string),
                workspace: Some(workspace),
                title: Some(format!("{TASK_TITLE_PREFIX}{name}")),
                withheld: vec!["kg_task_update".to_string()],
                task: Some(task.clone()),
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

/// The same, for an ordinary chat. Sessions are *renamed* as they grow
/// (`mecha_core::title`), and the prefix is what `history` filters on and
/// what tells a renamed session apart from a delegation — so a rename
/// carries it, and only sessions wearing it are eligible to be renamed at
/// all. A task conversation already has the task's name, and its title is
/// read by `task_withholding`.
pub(super) const WEB_TITLE_PREFIX: &str = "web: ";

/// What a session should be created *as*, when it does not exist yet.
///
/// Only a task conversation needs any of this today, and it is threaded
/// through the one constructor rather than given a second one: two creation
/// sites is how two kinds of session silently stop agreeing about the jail,
/// the recorded config, or the question routing.
#[derive(Default)]
pub(super) struct SessionInit {
    /// A recorded transcript to pick back up instead of starting fresh.
    ///
    /// **The board is the durable index; this map is a cache.** A task
    /// conversation lives in this process's memory and in a JSONL, and only
    /// the second survives a restart — so re-opening a task after one was
    /// minting a blank conversation under the same key, losing the thread,
    /// the task header and the D6 withholding in one go. The session id has
    /// been recorded on the task since the conversation was opened; it simply
    /// was not read back.
    pub resume_from: Option<String>,
    pub workspace: Option<PathBuf>,
    pub title: Option<String>,
    pub withheld: Vec<String>,
    pub task: Option<serde_json::Value>,
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
        // Picking one back up, or starting one. `Session::load` restores the
        // messages *and* the recorded taint, so a conversation that read a
        // hostile page before the restart still remembers after it.
        let recorded = init.resume_from.as_deref().and_then(|id| {
            let dir = Session::default_dir().ok()?;
            let path = Session::find(&dir, id).ok()?;
            Session::load(&path)
                .ok()
                .map(|(meta, convo)| (meta, path, convo))
        });
        let workspace = match init.workspace {
            Some(w) => {
                std::fs::create_dir_all(&w)?;
                w
            }
            None => session_workspace(key)?,
        };
        let (session, conversation) = match recorded {
            Some((meta, path, convo)) => {
                // The plan comes back with it (D15), from the transcript the
                // model is about to re-read anyway.
                if let Some(todo) = &chat.todo {
                    todo.rehydrate(&workspace, &convo.messages);
                }
                (Session { meta, path }, convo)
            }
            None => (
                Session::create(
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
                        title: Some(
                            init.title
                                .clone()
                                .unwrap_or_else(|| format!("{WEB_TITLE_PREFIX}{key}")),
                        ),
                    },
                )?,
                Conversation::new(),
            ),
        };
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
        let park = init
            .task
            .as_ref()
            .and_then(|t| t["id"].as_str().map(str::to_string))
            .and_then(|task| {
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
                conversation: Some(conversation),
                session: Arc::new(session),
                workspace,
                live: None,
                events,
                last_usage: Arc::new(StdMutex::new(None)),
                mode: Arc::new(StdMutex::new(PermissionMode::ReadOnly)),
                questions,
                last_turn_spoken: false,
                titled_at: 0,
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
        // What this conversation is *about*, when it is about a board task.
        // The record as the board returned it, so the page can head the
        // transcript with the goal, the dates and where it came from —
        // absent for an ordinary chat, which the page renders as it always
        // did.
        "task": ws.task,
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

    // Folded, not pushed, when the tail is already a user message — a
    // barge-in mid-tool-turn (`VoiceHost::speak` cancels the live run and
    // resubmits through here) leaves the conversation ending on the user
    // message carrying tool results, and pushing there makes two user
    // messages in a row. The fold is recorded at submit, so either way
    // `before` is what the file already holds — the `record_run` contract
    // every arm downstream assumes.
    let before = if conversation
        .messages
        .last()
        .is_some_and(|m| m.role == Role::User)
    {
        // Recorded immediately (one direct `Rewrite` — `record_run` here
        // would replay the previous run's still-uncleared `rewritten`
        // states), and fail-closed exactly as the push branch below is: an
        // unrecorded turn is invisible to distill, recall and the
        // run-quality corpus, whichever way it entered the conversation.
        let pre_fold = conversation.messages.clone();
        mecha_core::agent::append_user_text(&mut conversation.messages, text.clone());
        if let Err(e) = ws.session.append(&Record::Rewrite {
            messages: conversation.messages.clone(),
        }) {
            conversation.messages = pre_fold;
            ws.conversation = Some(conversation);
            return Err(TurnError::Failed(format!("recording: {e:#}")));
        }
        conversation.messages.clone()
    } else {
        let user = Message::user(&text);
        conversation.push(user.clone());
        if let Err(e) = ws.session.append(&Record::Message(user)) {
            // Refuse to run a turn the record did not accept: an unrecorded
            // run is invisible to distill, recall and the run-quality corpus.
            conversation.messages.pop();
            ws.conversation = Some(conversation);
            return Err(TurnError::Failed(format!("recording: {e:#}")));
        }
        conversation.messages.clone()
    };

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

        match &outcome {
            Ok(o) => {
                let _ = session.record_run(&before, &conversation);
                let _ = session.record_outcome(o);
            }
            // The transcript must agree with the rollback, or the failure
            // survives a resume — found on review: recording the mutated
            // conversation unconditionally persisted the very turn the error
            // arm below used to remove from memory only, and
            // `ensure_session_as` resumes straight off the file, restoring a
            // history that ends on an orphaned tool_result (or, since the
            // user message is appended at submit, on a dangling user turn).
            // The rolled-back list is not an extension of `before`, so
            // `record_run` expresses it as a `Rewrite` — a resume then loads
            // exactly what memory holds. The rollback happens HERE rather
            // than in the hand-back below so there is one rolled-back state
            // and both readers of it (the file, the next request) agree.
            Err(_) => {
                conversation.roll_back_failed_turn(before.clone());
                // The one write whose failure reproduces the resume-time 400
                // this arm exists to prevent — it must not fail silently.
                if let Err(e) = session.record_run(&before, &conversation) {
                    tracing::warn!(
                        "the rollback was not recorded — a resume of this \
                         session will replay the failed turn: {e:#}"
                    );
                }
            }
        }
        // Taint is kept either way — a failed turn that read a hostile page
        // still read it, and taint only ever grows.
        let _ = session.append(&Record::Taint(conversation.taint));

        // §6.2's readout: how this session's just-finished run appraises,
        // right now. Voice rides this same path (`VoiceHost`/`SessionHost`),
        // so one computation covers both doors — the wire event for the
        // page's logo, and `affect_label` below for the voice worker's TTS
        // style, threaded out through `HostedAnswer`. `Neutral` — the
        // overwhelming common case per the rung 7 corpus — sends no wire
        // event: the page's "show nothing" convention lives client-side
        // (mirroring the TUI's), and there is no reason to put an event on
        // every turn for a label that says nothing.
        let mut affect_label = None;
        if let Ok(o) = &outcome {
            // No drafts here (`appraisal::live`'s own doc comment) — found
            // on review, a draft resolved on an earlier or later turn than
            // this one has nothing to do with how *this* run went.
            //
            // `before.len()`, not `conversation.messages.len()`: `live`
            // needs where *this run's own* messages start, and `before` is
            // exactly that boundary — captured right after the triggering
            // user turn was appended, before this run added anything.
            let label =
                mecha_core::appraisal::live(&session.meta.id, o, &conversation, before.len());
            if label != mecha_core::appraisal::Affect::Neutral {
                affect_label = Some(label.wire());
                let _ = bcast.send(WireEvent::Affect {
                    label: affect_label.clone().unwrap(),
                });
            }
        }

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

        // What the titler is allowed to read, taken before the conversation
        // goes back: the owner's own turns and nothing else (`title`'s module
        // comment has the argument — a title outlives every answer in the
        // conversation and is rendered on every surface, so third-party text
        // must not reach the thing that composes one).
        // The one thing a block filter in core cannot strip, because it is
        // not a block: a spoken turn is `VOICE_BLOCK` *prefixed onto* the
        // owner's own words, and since D3 it lands in this same `web:`
        // conversation. The constant is this crate's, so the stripping is
        // too — the same helper the session listing already runs for the
        // same reason.
        let owner_turns: Vec<String> = mecha_core::title::owner_turns(&conversation.messages)
            .iter()
            .map(|t| strip_voice_preamble(t).to_string())
            .filter(|t| !t.is_empty())
            .collect();

        // Hand the conversation back, then announce the end — a stream left
        // open is indistinguishable from a run still working. A failed
        // turn's conversation was already rolled back beside the record
        // above, where the file and this hand-back could be made to agree.
        let mut sessions = state_for_task.sessions.lock().await;
        let mut name_it = false;
        if let Some(ws) = sessions.get_mut(&key_for_task) {
            ws.conversation = Some(conversation);
            ws.live = None;
            // Only an ordinary chat is renamed: a delegation already carries
            // the task's name (and `task_withholding` reads that title), and
            // a voice session is the same conversation from another door.
            name_it = ws
                .session
                .meta
                .title
                .as_deref()
                .is_some_and(|t| t.starts_with(WEB_TITLE_PREFIX))
                && mecha_core::title::due(owner_turns.len(), ws.titled_at);
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
                affect: affect_label,
            }),
            Err(e) => Err(format!("{e:#}")),
        });

        // **Last of all, because nothing waits on it.** Naming the
        // conversation costs a small generation on the same server the run
        // just used; doing it before the hand-back would hold the session,
        // and before the answer would delay it. A failure here is a session
        // that keeps the name it has, which is why nothing above depends on
        // it and why the miss is logged rather than shown.
        if name_it {
            let turns = owner_turns.len();
            match mecha_core::title::summarise(
                state_for_task.agent.provider(),
                &state_for_task.model,
                &owner_turns,
            )
            .await
            {
                Ok(Some(name)) => {
                    let recorded = format!("{WEB_TITLE_PREFIX}{name}");
                    // The record first: a name the page shows and the
                    // transcript does not is one that vanishes on the next
                    // restart, which is worse than never having had it.
                    if let Err(e) = session.append(&Record::Title {
                        title: recorded.clone(),
                    }) {
                        tracing::warn!("naming this conversation was not recorded: {e:#}");
                    } else {
                        let mut sessions = state_for_task.sessions.lock().await;
                        if let Some(ws) = sessions.get_mut(&key_for_task) {
                            let mut meta = ws.session.meta.clone();
                            meta.title = Some(recorded);
                            ws.session = Arc::new(Session {
                                meta,
                                path: ws.session.path.clone(),
                            });
                            ws.titled_at = turns;
                        }
                        drop(sessions);
                        let _ = bcast.send(WireEvent::Titled { title: name });
                    }
                }
                // The model had nothing usable to say. A miss, not a name:
                // the session keeps the one it has, and the next threshold
                // will ask again.
                Ok(None) => tracing::debug!("no usable name came back for {key_for_task}"),
                Err(e) => tracing::debug!("naming {key_for_task} failed: {e:#}"),
            }
        }
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
fn first_user_snippet(path: &std::path::Path) -> Option<Listing> {
    use std::io::BufRead;
    let file = std::fs::File::open(path).ok()?;
    let reader = std::io::BufReader::new(file);
    let mut listing = Listing::default();
    let mut read = 0usize;
    for line in reader.lines().take(LISTING_SCAN_LINES) {
        let line = line.ok()?;
        // **Bound the bytes, not just the lines.** A line count is not a cost
        // here: a `rewrite` record is the entire message list on one line, so
        // a session that compacted twice contributes two lines each the size
        // of the whole conversation, and the pre-filter below removes the
        // *parse*, never the read. Forty transcripts of those is a listing
        // that stalls the drawer — which now refetches on open, on `titled`,
        // and when the layout docks. A title record is ~60 bytes and is
        // written at owner turn 1, so the early rename this listing exists to
        // show is nowhere near either bound.
        read += line.len();
        if read > LISTING_SCAN_BYTES {
            break;
        }
        // **Rule the line out on its bytes before parsing it.** This loop
        // used to return at the first user message — line two or three of
        // the file — and a later `title` has to win, so it cannot any more.
        // What that costs is not the extra lines, it is what a line *is*: a
        // `message` record carries whole tool results and a `rewrite` record
        // carries the entire message list, so parsing forty transcripts'
        // worth of them to learn they are not titles is the whole cost of
        // this scan. Two record types matter and both are a substring test.
        let could_be_title = line.contains("\"record\":\"title\"");
        if !could_be_title && listing.snippet.is_some() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // A rename, from `mecha_core::title`. Last one within the scan wins.
        if v["record"] == "title" {
            if let Some(t) = v["title"].as_str() {
                listing.title = Some(t.to_string());
            }
        }
        if listing.snippet.is_none() && v["record"] == "message" && v["role"] == "user" {
            for block in v["content"].as_array().into_iter().flatten() {
                if let Some(text) = block["text"].as_str() {
                    let text = strip_voice_preamble(text).trim();
                    if !text.is_empty() {
                        listing.snippet = Some(text.chars().take(140).collect());
                        break;
                    }
                }
            }
        }
    }
    listing.snippet.is_some().then_some(listing)
}

/// What one row of the "earlier" listing needs from a transcript.
#[derive(Default)]
struct Listing {
    /// The owner's opening line. `None` means no user ever spoke — a shell
    /// created by a page load, which the listing skips.
    snippet: Option<String>,
    /// The recorded name, when the session has earned one.
    title: Option<String>,
}

/// How far into a transcript a *listing* reads.
///
/// The bound is the whole reason this is a scan and not a `Session::read`: a
/// transcript can be megabytes and a listing walks forty of them, so a
/// listing that read whole files is a listing nobody opens twice.
///
/// **The baseline this has to be honest against is three lines, not the
/// whole file** — found in review. Before renaming existed the loop returned
/// at the first user message, so `/api/history` parsed about three JSON
/// values per transcript; a later title has to win, so it cannot return
/// early any more, and the line count alone would have made this endpoint
/// forty times more expensive on records that carry whole tool results. The
/// byte pre-filter in the loop is what keeps the *parse* count near the old
/// baseline; this constant only bounds the read.
///
/// What the bound itself costs is small: renaming is front-loaded
/// (`title::due` names a session at its 1st, 3rd and 8th owner turn), so a
/// rename past this line lives in the rail without reaching the listing,
/// where the opening line stands in for it.
const LISTING_SCAN_LINES: usize = 400;

/// And how many *bytes*, which is the bound that actually holds.
///
/// Lines are not a cost model for this file — see the loop. Whichever bound
/// trips first ends the scan.
const LISTING_SCAN_BYTES: usize = 256 * 1024;

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
                t.starts_with(WEB_TITLE_PREFIX)
                    || t.starts_with("voice: ")
                    || t.starts_with(TASK_TITLE_PREFIX)
            })
    });
    metas.sort_by_key(|(m, _)| std::cmp::Reverse(m.created_at));
    let mut rows = Vec::new();
    for (meta, path) in metas {
        if rows.len() >= 40 {
            break;
        }
        let Some(listing) = first_user_snippet(&path) else {
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
            "snippet": listing.snippet,
            // The name it earned, when it has one — the page prefers this and
            // falls back to the opening line, which is what it showed before
            // conversations had names at all.
            "title": listing.title,
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
            titled_at: 0,
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
    /// A spoken turn is the owner's words with the voice preamble *prefixed
    /// onto them*, in the same conversation as the typed turns (D3) — so it
    /// is not a block the core filter can drop, and the door that added the
    /// decoration is the one that has to remove it. Without this the titler
    /// names a voice conversation after the harness's own instructions to
    /// itself.
    #[test]
    fn a_spoken_turn_reaches_the_titler_as_what_was_said() {
        let spoken = crate::voice::open_spoken_turn("what did I promise Hollis?", false);
        assert!(
            spoken.starts_with(crate::voice::VOICE_BLOCK),
            "fixture is not decorated"
        );
        let stripped = super::strip_voice_preamble(&spoken);
        assert_eq!(stripped, "what did I promise Hollis?");
        assert!(!stripped.contains("Voice mode"));
    }

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
mod rollback_tests {
    use super::*;

    /// The review's follow-up finding: the in-memory rollback was right and
    /// the transcript still kept what it discarded — `record_run` ran
    /// unconditionally on the mutated conversation, so a resume
    /// (`ensure_session_as` → `Session::load`) restored the orphaned turn
    /// the rollback had removed from memory only. Recording the
    /// *rolled-back* conversation instead makes the file express it as a
    /// `Rewrite`, and a resume loads exactly what memory holds.
    ///
    /// **Scope, stated so nobody over-reads a green run:** this re-types
    /// `begin_turn`'s error-arm sequence by hand and pins the
    /// *composition* — rollback then `record_run` yields a resumable file.
    /// It does not drive `begin_turn` itself (that means standing up the
    /// server), so dropping or reordering the calls at the real site would
    /// not fail here; the site's own comment carries that contract.
    #[test]
    fn a_failed_turn_s_transcript_resumes_to_the_rolled_back_state() {
        // Nanos, not just the pid: `cargo test` runs every test of this
        // binary in one process, so a pid-keyed dir collides with any
        // sibling using the same scheme. Leaked on panic, accepted — the
        // assertion message names the dir's content, not its path.
        let dir = std::env::temp_dir().join(format!(
            "mecha-chat-rollback-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let session = mecha_core::session::Session::create(
            &dir,
            SessionMeta {
                id: "20260101T000000-rollback".into(),
                created_at: chrono::Utc::now(),
                provider: "local".into(),
                model: "m".into(),
                workspace: std::path::PathBuf::from("/tmp"),
                title: None,
            },
        )
        .unwrap();

        // What the file holds at run start — begin_turn appends the
        // triggering user message at submit, before the run.
        let before = vec![
            Message::user("earlier turn"),
            Message::assistant(vec![Block::text("earlier answer")]),
            Message::user("do the thing"),
        ];
        session.append_messages(&before).unwrap();

        // What run_in leaves in memory when the provider dies mid-tool-turn.
        let mut conversation = Conversation::from(before.clone());
        conversation
            .messages
            .push(Message::assistant(vec![Block::ToolUse {
                id: "t1".into(),
                name: "shell".into(),
                input: serde_json::json!({}),
            }]));
        conversation.messages.push(Message {
            role: Role::User,
            content: vec![Block::ToolResult {
                tool_use_id: "t1".into(),
                content: "ok".into(),
                is_error: false,
            }],
        });

        // The error arm's sequence, exactly as begin_turn's completion task
        // runs it.
        conversation.roll_back_failed_turn(before.clone());
        session.record_run(&before, &conversation).unwrap();

        let (_, resumed) = mecha_core::session::Session::load(&session.path).unwrap();
        assert_eq!(
            resumed.messages, conversation.messages,
            "a resume must load exactly the rolled-back state, not the failed turn"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The regression this pins: the error arm used to pop the *mutated*
    /// list's tail instead of restoring the snapshot first — the only one of
    /// the four pop sites in this crate that skipped the restore. A failure
    /// after a tool turn then orphaned the assistant's `tool_use`, and every
    /// later request on the session 400'd.
    #[test]
    fn a_failure_after_a_tool_turn_leaves_no_orphaned_tool_use() {
        let before = vec![
            Message::user("earlier turn"),
            Message::assistant(vec![Block::text("earlier answer")]),
            Message::user("do the thing"),
        ];
        let mut conversation = Conversation::from(before.clone());
        // What `run_in` leaves behind when the provider dies after one tool
        // round: the call, its result, and no final answer.
        conversation
            .messages
            .push(Message::assistant(vec![Block::ToolUse {
                id: "t1".into(),
                name: "shell".into(),
                input: serde_json::json!({}),
            }]));
        conversation.messages.push(Message {
            role: Role::User,
            content: vec![Block::ToolResult {
                tool_use_id: "t1".into(),
                content: "ok".into(),
                is_error: false,
            }],
        });

        conversation.roll_back_failed_turn(before);

        assert!(
            !conversation
                .messages
                .iter()
                .flat_map(|m| &m.content)
                .any(|b| matches!(b, Block::ToolUse { .. })),
            "a dangling tool_use survived the rollback: {:?}",
            conversation.messages
        );
        assert_eq!(
            conversation.messages.last().map(|m| m.role),
            Some(Role::Assistant),
            "the triggering user message must be gone too, or the next \
             request resends it"
        );
    }

    /// The second failure mode the siblings' comments name: a mid-run
    /// compaction leaves the live list *shorter* than the snapshot, so the
    /// old bare pop removed an arbitrary rewritten message and kept the very
    /// user message the arm exists to drop.
    #[test]
    fn a_failure_after_a_compaction_still_drops_the_triggering_message() {
        let before = vec![
            Message::user("a long history"),
            Message::assistant(vec![Block::text("...")]),
            Message::user("the triggering message"),
        ];
        let mut conversation =
            Conversation::from(vec![Message::user("[summary of the run so far]")]);

        conversation.roll_back_failed_turn(before);

        assert!(
            !conversation
                .messages
                .iter()
                .flat_map(|m| &m.content)
                .any(|b| m_text(b) == "the triggering message"),
            "the triggering user message must not survive the rollback"
        );
        assert_eq!(conversation.messages.len(), 2);
    }

    fn m_text(b: &Block) -> &str {
        match b {
            Block::Text { text } => text,
            _ => "",
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
                    preview: Some("3 results".into()),
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

    /// §6.2: the wire shape a `Chat.svelte`'s logo tint keys on.
    #[test]
    fn an_affect_event_reaches_the_page_under_the_name_the_logo_switches_on() {
        let wire = serde_json::to_value(WireEvent::Affect {
            label: "anger".into(),
        })
        .unwrap();
        assert_eq!(wire["type"], "affect");
        assert_eq!(wire["label"], "anger");
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
                preview: Some("…".into()),
            }]
        );
    }
}
