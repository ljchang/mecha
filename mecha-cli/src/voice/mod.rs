//! `mecha voice-serve` — the OpenAI-compatible facade in front of the agent
//! loop. The voice worker sees "an LLM"; tools, taint, the outbox and the
//! interlock stay inside the harness. `docs/VOICE-RESEARCH.md` D2, D3 and
//! D10 are the decisions this file encodes; §7 is the build log.
//!
//! Deliberate shape, each a bug if undone:
//!
//! - **Loopback only, and not configurable.** The worker runs on this
//!   machine; nothing on any network — the tailnet included — can reach the
//!   agent directly. `LISTEN_HOST` is a constant, not a flag, and a test
//!   asserts it stays loopback (the `show_file` no-destination-argument
//!   rule, applied to a bind address).
//! - **A voice session is a `Conversation`.** One per session id, held for
//!   the daemon's lifetime, recorded as an ordinary session JSONL — so
//!   distillation and the run-quality corpus see voice for free. The
//!   framework re-sends its whole chat history every request; only the last
//!   user message is read, because mecha's `Conversation` is the state
//!   (the Slack-thread precedent).
//! - **A new request for a busy session is barge-in.** It cancels the run
//!   in flight and waits for the slot; the partial turn survives in the
//!   conversation, exactly as with Ctrl-C. Client disconnect mid-stream
//!   cancels the same way — hanging up is the third spelling of interrupt.
//! - **`recall` is deliberately absent**, as on Slack and for the same
//!   reason: the registry belongs to the one shared agent, and per-session
//!   transcripts would cross-wire it. Registering it needs an agent per
//!   session, which is an MCP startup per session.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use mecha_core::agent::{Agent, AgentEvent, Conversation};
use mecha_core::message::Message;
use mecha_core::outbox::{OutboxRoute, OutboxStore};
use mecha_core::session::{Record, RunConfig, Session, SessionMeta};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::commands::voice_serve::Args;
use crate::GlobalOpts;

/// Loopback, by design rather than default — see the module docs.
const LISTEN_HOST: &str = "127.0.0.1";

/// The largest request body honoured. An utterance is text; even a pasted
/// document fits with room to spare, and a declared Content-Length must
/// never size an allocation on its own say-so.
const MAX_BODY_BYTES: usize = 8 << 20;

/// D10: the one load-bearing prompt. Static byte-for-byte across sessions,
/// because it rides in the cached prefix and TTFT is the latency budget.
const VOICE_BLOCK: &str = "\
Voice mode: everything you write is spoken aloud by a text-to-speech voice, \
and the user is listening, not reading. Answer in short conversational \
sentences. Never use markdown, bullet lists, headings, tables or code \
blocks; write numbers, dates and times as they are spoken. When a tool \
returns something long, say the gist in a sentence or two instead of \
reciting it. Before a slow step, say one short line about what you are \
doing. When a message or email was staged for review rather than sent, say \
so out loud. Keep replies brief unless the user asks you to go deep.";

/// One voice session between runs: its conversation and its transcript.
struct Slot {
    convo: Conversation,
    session: Session,
}

/// What the map holds per session id. `Running` carries the cancel handle so
/// a new request (or a hang-up) can barge in.
enum SlotState {
    Idle(Box<Slot>),
    Running(CancellationToken),
}

struct Shared {
    agent: Arc<Agent>,
    /// True when the agent is shared with other front-ends (the unified
    /// `mecha serve`): the D10 voice block then cannot ride the system
    /// prompt, so it is prepended to each voice conversation's first user
    /// message instead — one copy per conversation, cached thereafter.
    inject_voice_block: bool,
    slots: Mutex<HashMap<String, SlotState>>,
    session_dir: PathBuf,
    outbox_root: PathBuf,
    provider_name: String,
    model: String,
    config: mecha_core::config::Config,
    token: Option<String>,
}

/// The facade as a mountable component: `mecha voice-serve` builds its own
/// agent and hands it here; the unified `mecha serve` hands in the shared
/// one. Either way the semantics below are identical — this split exists so
/// there is exactly one implementation of them.
pub struct Facade {
    shared: Arc<Shared>,
}

impl Facade {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        agent: Arc<Agent>,
        provider_name: String,
        model: String,
        config: mecha_core::config::Config,
        outbox_root: PathBuf,
        token: Option<String>,
        inject_voice_block: bool,
    ) -> Result<Self> {
        Ok(Self {
            shared: Arc::new(Shared {
                agent,
                inject_voice_block,
                slots: Mutex::new(HashMap::new()),
                session_dir: Session::default_dir()?,
                outbox_root,
                provider_name,
                model,
                config,
                token,
            }),
        })
    }

    /// Bind the loopback listener and serve until cancelled. Signal
    /// handling deliberately lives with the caller — a mounted facade must
    /// not compete with its host process for SIGTERM.
    pub async fn serve(&self, port: u16, stop: CancellationToken) -> Result<()> {
        let addr = format!("{LISTEN_HOST}:{port}");
        let listener = TcpListener::bind(&addr)
            .await
            .with_context(|| format!("binding {addr}"))?;
        tracing::info!("voice facade listening on http://{addr}/v1/chat/completions");
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (stream, _) = accepted?;
                    let shared = Arc::clone(&self.shared);
                    tokio::spawn(async move {
                        if let Err(e) = handle(stream, shared).await {
                            tracing::debug!("voice connection ended: {e}");
                        }
                    });
                }
                _ = stop.cancelled() => return Ok(()),
            }
        }
    }

    /// Cancel everything in flight, then wait (bounded) for handlers to
    /// record their runs and return the slots — exiting without this tears
    /// down the runtime mid-record.
    pub async fn shutdown(&self) {
        {
            let slots = self.shared.slots.lock().await;
            for state in slots.values() {
                if let SlotState::Running(tok) = state {
                    tok.cancel();
                }
            }
        }
        for _ in 0..150 {
            let busy = {
                let slots = self.shared.slots.lock().await;
                slots.values().any(|s| matches!(s, SlotState::Running(_)))
            };
            if !busy {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

pub async fn run(global: &GlobalOpts, args: Args) -> Result<()> {
    let mut opts = global.clone();
    // A daemon must not take its tool surface from whatever repository it
    // was started in — the trigger runner's rule, for the same reason.
    opts.global_config_only = true;
    opts.system_extra = Some(VOICE_BLOCK.to_string());
    if opts.workspace.is_none() {
        // The stable producer dir (D9): Tuesday's sketch is an ordinary
        // file on Thursday, and retention already governs it.
        let dir = mecha_core::work::producer_dir("voice")?;
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        opts.workspace = Some(dir);
    }
    let prepared = crate::setup::prepare(&opts, false).await?;

    let global_cfg = mecha_core::config::Config::load_global()?;
    let outbox_root = match global_cfg.outbox.dir.clone() {
        Some(dir) => dir,
        None => OutboxStore::default_root()?,
    };

    let workspace = prepared.workspace.clone();
    let facade = Facade::new(
        Arc::new(prepared.agent),
        prepared.provider_name.clone(),
        prepared.model.clone(),
        prepared.config,
        outbox_root,
        args.token.clone(),
        // Standalone: the voice block already rides this agent's system
        // prompt via system_extra, so nothing to inject per conversation.
        false,
    )?;

    println!(
        "mecha voice-serve · {} ({}) · listening on http://{LISTEN_HOST}:{}/v1/chat/completions · workspace {}",
        facade.shared.model,
        facade.shared.provider_name,
        args.port,
        workspace.display()
    );

    // SIGTERM is how systemd stops this service, so it must mean what
    // Ctrl-C means: cancel, let partial turns land in their transcripts,
    // then go.
    let stop = CancellationToken::new();
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let server = facade.serve(args.port, stop.clone());
    tokio::pin!(server);
    tokio::select! {
        r = &mut server => r?,
        _ = tokio::signal::ctrl_c() => stop.cancel(),
        _ = sigterm.recv() => stop.cancel(),
    }
    facade.shutdown().await;
    println!("\nvoice-serve: shutting down.");
    Ok(())
}

// ---------------------------------------------------------------- HTTP

/// The parsed head of a request, pure so it is testable.
struct Head {
    method: String,
    path: String,
    content_length: usize,
    authorization: Option<String>,
    session: Option<String>,
    body_start: usize,
}

fn parse_head(buf: &[u8]) -> Result<Option<Head>> {
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut req = httparse::Request::new(&mut headers);
    match req.parse(buf)? {
        httparse::Status::Partial => Ok(None),
        httparse::Status::Complete(body_start) => {
            let mut content_length = 0usize;
            let mut authorization = None;
            let mut session = None;
            for h in req.headers.iter() {
                if h.name.eq_ignore_ascii_case("content-length") {
                    content_length = std::str::from_utf8(h.value)?.trim().parse()?;
                } else if h.name.eq_ignore_ascii_case("authorization") {
                    authorization = Some(String::from_utf8_lossy(h.value).into_owned());
                } else if h.name.eq_ignore_ascii_case("x-voice-session") {
                    session = Some(String::from_utf8_lossy(h.value).trim().to_string());
                }
            }
            Ok(Some(Head {
                method: req.method.unwrap_or("").to_string(),
                path: req.path.unwrap_or("").to_string(),
                content_length,
                authorization,
                session,
                body_start,
            }))
        }
    }
}

fn auth_ok(required: &Option<String>, header: &Option<String>) -> bool {
    match required {
        None => true,
        Some(want) => matches!(header, Some(h) if h.strip_prefix("Bearer ") == Some(want)),
    }
}

async fn handle(mut stream: TcpStream, shared: Arc<Shared>) -> Result<()> {
    let mut buf = Vec::with_capacity(8192);
    let head = loop {
        let mut chunk = [0u8; 8192];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(()); // closed before a full request
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(head) = parse_head(&buf)? {
            break head;
        }
        if buf.len() > 1 << 20 {
            anyhow::bail!("request head too large");
        }
    };
    // Refusals that need only the head happen before the body is read: the
    // token must gate the allocation, and a declared length is a claim, not
    // an entitlement — the head is capped, so the body is too.
    if head.method == "POST" && !auth_ok(&shared.token, &head.authorization) {
        return write_json(&mut stream, 401, &json!({"error": "unauthorized"})).await;
    }
    if head.content_length > MAX_BODY_BYTES {
        return write_json(&mut stream, 413, &json!({"error": "body too large"})).await;
    }
    let mut body = buf[head.body_start..].to_vec();
    while body.len() < head.content_length {
        let mut chunk = [0u8; 8192];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            anyhow::bail!("connection closed mid-body");
        }
        body.extend_from_slice(&chunk[..n]);
    }

    match (head.method.as_str(), head.path.as_str()) {
        ("GET", "/health") => write_json(&mut stream, 200, &json!({"status": "ok"})).await,
        ("POST", "/v1/chat/completions") => completion(&mut stream, &shared, &head, &body).await,
        _ => write_json(&mut stream, 404, &json!({"error": "not found"})).await,
    }
}

async fn write_json(stream: &mut TcpStream, status: u16, body: &Value) -> Result<()> {
    let text = body.to_string();
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        text.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(text.as_bytes()).await?;
    Ok(())
}

// ------------------------------------------------------- the completion

/// The session id: the `X-Voice-Session` header when the client sends one
/// (the worker stamps a per-connection key there — pipecat's LLM service
/// exposes `default_headers` but no `user` field), else OpenAI's `user`
/// field, else "default".
fn session_key(body: &Value, header: &Option<String>) -> String {
    if let Some(h) = header {
        if !h.is_empty() {
            return h.clone();
        }
    }
    body.get("user")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("default")
        .to_string()
}

/// The newest user message's text. String content or text parts; the rest of
/// the re-sent history is deliberately ignored — the `Conversation` is the
/// state, not the client's transcript.
fn last_user_text(body: &Value) -> Option<String> {
    let messages = body.get("messages")?.as_array()?;
    let last = messages
        .iter()
        .rev()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("user"))?;
    match last.get("content") {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
        Some(Value::Array(parts)) => {
            let text: String = parts
                .iter()
                .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(" ");
            if text.trim().is_empty() {
                None
            } else {
                Some(text)
            }
        }
        _ => None,
    }
}

fn sse_chunk(id: &str, model: &str, delta: Value, finish: Option<&str>) -> String {
    let payload = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": chrono::Utc::now().timestamp(),
        "model": model,
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
    });
    format!("data: {payload}\n\n")
}

async fn write_chunk(stream: &mut TcpStream, data: &[u8]) -> std::io::Result<()> {
    stream
        .write_all(format!("{:x}\r\n", data.len()).as_bytes())
        .await?;
    stream.write_all(data).await?;
    stream.write_all(b"\r\n").await
}

/// Take the session's slot, cancelling any run in flight (barge-in) and
/// creating the session on first use. `Ok(None)` means the slot never came
/// free — the caller owes the client an answer, not a dropped connection.
///
/// The session file is created *outside* the lock: `Session::create` is
/// synchronous disk I/O, and holding the one map mutex across it would
/// stall every other session (and shutdown) behind a slow disk. The key is
/// reserved as `Running` first so nobody else creates the same session.
///
/// Known and accepted: two requests barging in on the same key are not
/// ordered — whichever waiter polls first wins. Pipecat's user aggregator
/// serialises turns per connection and keys are per-connection, so
/// concurrent same-key requests take a client that misbehaves; ordering
/// machinery here would outweigh the failure it prevents.
async fn take_slot(
    shared: &Arc<Shared>,
    key: &str,
) -> Result<Option<(Box<Slot>, CancellationToken)>> {
    for _ in 0..200 {
        {
            let mut slots = shared.slots.lock().await;
            match slots.remove(key) {
                None => {
                    let token = CancellationToken::new();
                    slots.insert(key.to_string(), SlotState::Running(token.clone()));
                    drop(slots);
                    let created = Session::create(
                        &shared.session_dir,
                        SessionMeta {
                            id: Session::new_id(),
                            created_at: chrono::Utc::now(),
                            provider: shared.provider_name.clone(),
                            model: shared.model.clone(),
                            workspace: shared.agent.context().tools.workspace.clone(),
                            title: Some(format!("voice: {key}")),
                        },
                    )
                    .and_then(|session| {
                        session.append(&Record::Config(RunConfig::of(
                            &shared.agent,
                            &shared.config,
                            &shared.provider_name,
                        )))?;
                        Ok(session)
                    });
                    match created {
                        Ok(session) => {
                            return Ok(Some((
                                Box::new(Slot {
                                    convo: Conversation::new(),
                                    session,
                                }),
                                token,
                            )))
                        }
                        Err(e) => {
                            // Release the reservation, or the key is dead
                            // until the daemon restarts.
                            shared.slots.lock().await.remove(key);
                            return Err(e);
                        }
                    }
                }
                Some(SlotState::Idle(slot)) => {
                    let token = CancellationToken::new();
                    slots.insert(key.to_string(), SlotState::Running(token.clone()));
                    return Ok(Some((slot, token)));
                }
                Some(SlotState::Running(tok)) => {
                    // Barge-in: cancel and wait for the slot to come back.
                    // A tool call is never interrupted mid-call, so a long
                    // one can outlast this whole window — that is the
                    // Ok(None) the caller answers with a 503.
                    tok.cancel();
                    slots.insert(key.to_string(), SlotState::Running(tok));
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(None)
}

async fn completion(
    stream: &mut TcpStream,
    shared: &Arc<Shared>,
    head: &Head,
    body: &[u8],
) -> Result<()> {
    let body: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => {
            return write_json(stream, 400, &json!({"error": "invalid JSON body"})).await;
        }
    };
    let key = session_key(&body, &head.session);
    let Some(text) = last_user_text(&body) else {
        return write_json(stream, 400, &json!({"error": "no user message"})).await;
    };
    let want_stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);

    let Some((mut slot, cancel)) = take_slot(shared, &key).await? else {
        // The in-flight run would not yield — usually a tool call longer
        // than the barge-in window, which cancellation never interrupts
        // mid-call. An answer the worker can speak beats a dropped socket
        // that reads as the assistant ignoring the user.
        return write_json(
            stream,
            503,
            &json!({"error": "still finishing the previous step — try again in a moment"}),
        )
        .await;
    };

    // From here the slot must always find its way back into the map, so
    // nothing below uses `?` until it has.
    let mut cx = (**shared.agent.context()).clone();
    cx.cancel = Some(cancel.clone());
    // Its own outbox route carrying this session's id — the shared route is
    // one Arc across every session, and the stamp is what attributes a draft
    // to the run that wrote it. Fail closed like the connector: a run that
    // would stage drafts without attribution must not run at all.
    if let Some(shared_route) = &shared.agent.context().outbox {
        match OutboxStore::open(&shared.outbox_root) {
            Ok(store) => {
                let mine = OutboxRoute::new(
                    store,
                    shared_route.routed().map(String::from).collect::<Vec<_>>(),
                    shared_route
                        .publishes()
                        .map(String::from)
                        .collect::<Vec<_>>(),
                );
                mine.set_session_id(&slot.session.meta.id);
                cx.outbox = Some(Arc::new(mine));
            }
            Err(e) => {
                tracing::error!("outbox store unavailable, refusing the turn: {e}");
                shared.slots.lock().await.insert(key, SlotState::Idle(slot));
                return write_json(stream, 503, &json!({"error": "outbox store unavailable"}))
                    .await;
            }
        }
    }

    // On a shared agent the D10 block cannot ride the system prompt, so a
    // new voice conversation opens with it — one copy, cached thereafter.
    let text = if shared.inject_voice_block && slot.convo.is_empty() {
        format!("{VOICE_BLOCK}\n\n{text}")
    } else {
        text
    };
    let user = Message::user(&text);
    slot.convo.push(user.clone());
    let _ = slot.session.append(&Record::Message(user));
    let recorded = slot.convo.messages.clone();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
    let agent = Arc::clone(&shared.agent);
    let run = tokio::spawn(async move {
        let outcome = agent.run_in(&cx, &mut slot.convo, Some(tx)).await;
        (slot, outcome)
    });

    let id = format!("chatcmpl-{}", Session::new_id());
    let mut disconnected = false;

    if want_stream {
        let head_bytes = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n";
        if stream.write_all(head_bytes.as_bytes()).await.is_err() {
            cancel.cancel();
            disconnected = true;
        }
        if !disconnected {
            let first = sse_chunk(&id, &shared.model, json!({"role": "assistant"}), None);
            if write_chunk(stream, first.as_bytes()).await.is_err() {
                cancel.cancel();
                disconnected = true;
            }
        }
        let mut keepalive = tokio::time::interval(Duration::from_secs(5));
        keepalive.reset();
        loop {
            tokio::select! {
                ev = rx.recv() => match ev {
                    Some(AgentEvent::TextDelta(t)) if !disconnected => {
                        let chunk = sse_chunk(&id, &shared.model, json!({"content": t}), None);
                        if write_chunk(stream, chunk.as_bytes()).await.is_err() {
                            // The hang-up is the third spelling of interrupt.
                            cancel.cancel();
                            disconnected = true;
                        }
                    }
                    Some(_) => {}
                    // The run dropped its sender: it is over. Keep going to
                    // collect the outcome below.
                    None => break,
                },
                _ = keepalive.tick() => {
                    if !disconnected
                        && write_chunk(stream, b": ping\n\n").await.is_err() {
                        cancel.cancel();
                        disconnected = true;
                    }
                }
            }
        }
    } else {
        // Non-streaming: drain events without rendering them.
        while rx.recv().await.is_some() {}
    }

    let (mut slot, outcome) = match run.await {
        Ok(pair) => pair,
        Err(e) => {
            tracing::error!("voice run task died: {e}");
            // The slot is gone with the task; drop the key so the next
            // request starts a fresh conversation instead of hanging.
            shared.slots.lock().await.remove(&key);
            return Ok(());
        }
    };

    match &outcome {
        Ok(o) => {
            // A run cancelled before its first token keeps nothing, so the
            // conversation still ends on the user message just pushed — and
            // the next push would make two user turns in a row, which is
            // invalid everywhere in this codebase. Pop it; record_run sees
            // the divergence from `recorded` and writes a rewrite record,
            // so the transcript stays honest about what happened.
            if slot
                .convo
                .messages
                .last()
                .is_some_and(|m| matches!(m.role, mecha_core::message::Role::User))
            {
                slot.convo.messages.pop();
            }
            let _ = slot.session.record_run(&recorded, &slot.convo);
            let _ = slot.session.record_outcome(o);
            let _ = slot.session.append(&Record::Taint(slot.convo.taint));
        }
        Err(e) => {
            tracing::error!("voice run failed: {e}");
            // The chat REPL's rule: drop the turn so a failed request does
            // not leave a dangling user message the next request would
            // collide with. Restored from the snapshot, not truncated — a
            // mid-run compaction leaves the list shorter than it started.
            slot.convo.messages = recorded.clone();
            slot.convo.messages.pop();
        }
    }

    if want_stream {
        if !disconnected {
            // A failure must be audible: a clean "stop" after silence reads
            // as the assistant ignoring the user, and the server-side log
            // is the one place a voice user will never look.
            if let Err(e) = &outcome {
                let spoken = sse_chunk(
                    &id,
                    &shared.model,
                    json!({"content": format!("I hit a problem and could not answer: {e:#}")}),
                    None,
                );
                let _ = write_chunk(stream, spoken.as_bytes()).await;
            }
            let done = sse_chunk(&id, &shared.model, json!({}), Some("stop"));
            let _ = write_chunk(stream, done.as_bytes()).await;
            let _ = write_chunk(stream, b"data: [DONE]\n\n").await;
            let _ = stream.write_all(b"0\r\n\r\n").await;
        }
    } else {
        match &outcome {
            Ok(o) => {
                let _ = write_json(
                    stream,
                    200,
                    &json!({
                        "id": id,
                        "object": "chat.completion",
                        "created": chrono::Utc::now().timestamp(),
                        "model": shared.model,
                        "choices": [{
                            "index": 0,
                            "message": {"role": "assistant", "content": o.text},
                            "finish_reason": "stop",
                        }],
                        "usage": {
                            "prompt_tokens": o.usage.input_tokens,
                            "completion_tokens": o.usage.output_tokens,
                        },
                    }),
                )
                .await;
            }
            Err(e) => {
                let _ = write_json(stream, 500, &json!({"error": e.to_string()})).await;
            }
        }
    }

    shared.slots.lock().await.insert(key, SlotState::Idle(slot));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bind_address_is_loopback_and_stays_that_way() {
        // D2: the facade is unreachable from any network. If this constant
        // grows a config knob, the voice layer stops being the only door.
        assert!(LISTEN_HOST.starts_with("127.0.0.1"));
    }

    #[test]
    fn the_voice_block_carries_no_markdown() {
        // D10: the block teaches ear-shaped output; it had better practice
        // what it preaches, because it rides in every voice prompt.
        for banned in ["```", "\n- ", "# ", "**"] {
            assert!(
                !VOICE_BLOCK.contains(banned),
                "voice block contains {banned:?}"
            );
        }
    }

    #[test]
    fn last_user_text_reads_string_content() {
        let body = serde_json::json!({"messages": [
            {"role": "system", "content": "ignored"},
            {"role": "user", "content": "first"},
            {"role": "assistant", "content": "reply"},
            {"role": "user", "content": "second"},
        ]});
        assert_eq!(last_user_text(&body).as_deref(), Some("second"));
    }

    #[test]
    fn last_user_text_joins_text_parts_and_skips_other_kinds() {
        let body = serde_json::json!({"messages": [{"role": "user", "content": [
            {"type": "text", "text": "hello"},
            {"type": "input_audio", "input_audio": {"data": "zzz"}},
            {"type": "text", "text": "there"},
        ]}]});
        assert_eq!(last_user_text(&body).as_deref(), Some("hello there"));
    }

    #[test]
    fn a_history_with_no_user_turn_is_refused_not_guessed() {
        let body = serde_json::json!({"messages": [{"role": "system", "content": "x"}]});
        assert_eq!(last_user_text(&body), None);
        let empty = serde_json::json!({"messages": [{"role": "user", "content": "  "}]});
        assert_eq!(last_user_text(&empty), None);
    }

    #[test]
    fn session_key_prefers_header_then_user_field_then_default() {
        let none = None;
        assert_eq!(
            session_key(&serde_json::json!({"user": "call-7"}), &none),
            "call-7"
        );
        assert_eq!(session_key(&serde_json::json!({}), &none), "default");
        assert_eq!(
            session_key(&serde_json::json!({"user": ""}), &none),
            "default"
        );
        // The worker's per-connection header outranks the body field.
        let header = Some("conn-abc".to_string());
        assert_eq!(
            session_key(&serde_json::json!({"user": "call-7"}), &header),
            "conn-abc"
        );
    }

    #[test]
    fn parse_head_reads_method_path_length_and_auth() {
        let raw = b"POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\nAuthorization: Bearer tok\r\n\r\nhello";
        let head = parse_head(raw).unwrap().expect("complete");
        assert_eq!(head.method, "POST");
        assert_eq!(head.path, "/v1/chat/completions");
        assert_eq!(head.content_length, 5);
        assert_eq!(head.authorization.as_deref(), Some("Bearer tok"));
        assert_eq!(&raw[head.body_start..], b"hello");
    }

    #[test]
    fn a_partial_head_asks_for_more_rather_than_erroring() {
        assert!(parse_head(b"POST /v1/chat").unwrap().is_none());
    }

    #[test]
    fn auth_is_open_without_a_token_and_exact_with_one() {
        assert!(auth_ok(&None, &None));
        let want = Some("secret".to_string());
        assert!(auth_ok(&want, &Some("Bearer secret".into())));
        assert!(!auth_ok(&want, &Some("Bearer wrong".into())));
        assert!(!auth_ok(&want, &None));
    }

    #[test]
    fn sse_chunks_are_data_framed_json() {
        let chunk = sse_chunk("id1", "m", serde_json::json!({"content": "hi"}), None);
        assert!(chunk.starts_with("data: "));
        assert!(chunk.ends_with("\n\n"));
        let v: Value = serde_json::from_str(chunk.trim_start_matches("data: ").trim()).unwrap();
        assert_eq!(v["choices"][0]["delta"]["content"], "hi");
        assert!(v["choices"][0]["finish_reason"].is_null());
    }
}
