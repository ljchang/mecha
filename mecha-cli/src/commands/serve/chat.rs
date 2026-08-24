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
//! Phase 2 posture: **read-only, sends stage.** There is no approval UI yet,
//! so every run carries `ModeApprover { ReadOnly }` — reads run, mutations
//! are refused as `Blocked` (machine policy, never mined as a correction),
//! and outbox-routed sends still stage, because staging executes nothing.
//! That is the trigger posture, and it makes the safe default also the
//! useful one. The live approver is Phase 4.

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
use mecha_core::tool::{ModeApprover, ToolCtx};
use mecha_core::work;

use crate::{setup, GlobalOpts};

/// Phase 2 serves one conversation; the map and the wire shapes are already
/// keyed so Phase 4's session rail extends this without breaking the page.
const SESSION_KEY: &str = "main";

pub struct ChatState {
    agent: Arc<Agent>,
    config: Config,
    provider_name: String,
    model: String,
    context_window: Option<u64>,
    outbox_root: PathBuf,
    sessions: Mutex<HashMap<String, WebSession>>,
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
        // Not interactive: no terminal approver, no ask_user.
        let prepared = setup::prepare(&opts, false).await?;
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
            provider_name: prepared.provider_name.clone(),
            model: prepared.model.clone(),
            context_window,
            outbox_root,
            sessions: Mutex::new(HashMap::new()),
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
fn session_workspace(key: &str) -> Result<PathBuf> {
    let dir = work::producer_dir("web")?.join(key);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    work::ensure_outside_mecha_home(&dir)?;
    Ok(dir)
}

// ---------------------------------------------------------------------------
// The wire: what the page receives, over SSE and in the transcript read.
// Pure functions of core types, so they are testable without a server.

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireEvent {
    Delta {
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
                            text.push_str(t);
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

fn ensure_session<'a>(
    chat: &Arc<ChatState>,
    sessions: &'a mut HashMap<String, WebSession>,
) -> Result<&'a mut WebSession> {
    if !sessions.contains_key(SESSION_KEY) {
        let workspace = session_workspace(SESSION_KEY)?;
        let session = Session::create(
            &Session::default_dir()?,
            SessionMeta {
                id: Session::new_id(),
                created_at: chrono::Utc::now(),
                provider: chat.provider_name.clone(),
                model: chat.model.clone(),
                workspace: workspace.clone(),
                title: Some(format!("web: {SESSION_KEY}")),
            },
        )?;
        session.append(&Record::Config(RunConfig::of(
            &chat.agent,
            &chat.config,
            &chat.provider_name,
        )))?;
        let (events, _) = broadcast::channel(512);
        sessions.insert(
            SESSION_KEY.to_string(),
            WebSession {
                conversation: Some(Conversation::new()),
                session: Arc::new(session),
                workspace,
                live: None,
                events,
                last_usage: Arc::new(StdMutex::new(None)),
            },
        );
    }
    Ok(sessions.get_mut(SESSION_KEY).expect("just inserted"))
}

/// GET /api/chat — what a fresh page load renders.
pub async fn transcript(State(state): Chat) -> axum::response::Response {
    let chat = match chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let mut sessions = chat.sessions.lock().await;
    let ws = match ensure_session(chat, &mut sessions) {
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
    Json(serde_json::json!({
        "session": ws.session.meta.id,
        "model": chat.model,
        "running": running,
        "held_by_run": ws.conversation.is_none(),
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

/// POST /api/chat/send — start a run, or steer the one in flight.
pub async fn send(State(state): Chat, Json(body): Json<SendBody>) -> axum::response::Response {
    let chat = match chat_state(&state) {
        Ok(c) => c.clone(),
        Err(resp) => return resp,
    };
    let text = body.text.trim().to_string();
    if text.is_empty() {
        return (StatusCode::BAD_REQUEST, "empty message\n").into_response();
    }

    let mut sessions = chat.sessions.lock().await;
    let ws = match ensure_session(&chat, &mut sessions) {
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

    let Some(mut conversation) = ws.conversation.take() else {
        return (
            StatusCode::CONFLICT,
            "conversation is held by a finished run still landing\n",
        )
            .into_response();
    };

    // Append the user message to the record *before* the run — the
    // `record_run` contract: `before` is what the file already holds.
    let user = Message::user(&text);
    conversation.push(user.clone());
    if let Err(e) = ws.session.append(&Record::Message(user)) {
        // Refuse to run a turn the record did not accept: an unrecorded run
        // is invisible to distill, recall and the run-quality corpus.
        conversation.messages.pop();
        ws.conversation = Some(conversation);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("recording: {e:#}\n"),
        )
            .into_response();
    }
    let before = conversation.messages.clone();

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
    cx.approver = Arc::new(ModeApprover {
        mode: PermissionMode::ReadOnly,
    });
    if cx.budget.max_turns.is_none() {
        cx.budget.max_turns = Some(40);
    }
    cx.cancel = Some(cancel.clone());
    cx.queued_input = Some(Arc::clone(&queue));
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

    let agent = Arc::clone(&chat.agent);
    let session = Arc::clone(&ws.session);
    let bcast = ws.events.clone();
    let last_usage = Arc::clone(&ws.last_usage);
    let context_window = chat.context_window;
    let state_for_task = chat.clone();

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
        if let Some(ws) = sessions.get_mut(SESSION_KEY) {
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
    });

    Json(serde_json::json!({ "started": true })).into_response()
}

/// POST /api/chat/cancel — stop at the next safe point, keep the partial.
pub async fn cancel(State(state): Chat) -> axum::response::Response {
    let chat = match chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let sessions = chat.sessions.lock().await;
    match sessions.get(SESSION_KEY).and_then(|ws| ws.live.as_ref()) {
        Some(live) => {
            live.cancel.cancel();
            Json(serde_json::json!({ "cancelled": true })).into_response()
        }
        None => Json(serde_json::json!({ "cancelled": false })).into_response(),
    }
}

/// GET /api/chat/events — the run, streamed. Subscribing is legal at any
/// time; a subscriber that falls behind gets a notice, never silence.
pub async fn events(State(state): Chat) -> axum::response::Response {
    let chat = match chat_state(&state) {
        Ok(c) => c.clone(),
        Err(resp) => return resp,
    };
    let rx = {
        let mut sessions = chat.sessions.lock().await;
        match ensure_session(&chat, &mut sessions) {
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

#[cfg(test)]
mod tests {
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
