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
    slots: Mutex<HashMap<String, SlotState>>,
    session_dir: PathBuf,
    outbox_root: PathBuf,
    provider_name: String,
    model: String,
    config: mecha_core::config::Config,
    token: Option<String>,
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

    let shared = Arc::new(Shared {
        agent: Arc::new(prepared.agent),
        slots: Mutex::new(HashMap::new()),
        session_dir: Session::default_dir()?,
        outbox_root,
        provider_name: prepared.provider_name.clone(),
        model: prepared.model.clone(),
        config: prepared.config,
        token: args.token.clone(),
    });

    let addr = format!("{LISTEN_HOST}:{}", args.port);
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    println!(
        "mecha voice-serve · {} ({}) · listening on http://{addr}/v1/chat/completions · workspace {}",
        shared.model,
        shared.provider_name,
        prepared.workspace.display()
    );

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let shared = Arc::clone(&shared);
                tokio::spawn(async move {
                    if let Err(e) = handle(stream, shared).await {
                        tracing::debug!("voice connection ended: {e}");
                    }
                });
            }
            _ = tokio::signal::ctrl_c() => {
                // Cancel whatever is in flight so partial turns land in
                // their transcripts before the process goes.
                let slots = shared.slots.lock().await;
                for state in slots.values() {
                    if let SlotState::Running(tok) = state {
                        tok.cancel();
                    }
                }
                println!("\nvoice-serve: shutting down.");
                return Ok(());
            }
        }
    }
}

// ---------------------------------------------------------------- HTTP

/// The parsed head of a request, pure so it is testable.
struct Head {
    method: String,
    path: String,
    content_length: usize,
    authorization: Option<String>,
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
            for h in req.headers.iter() {
                if h.name.eq_ignore_ascii_case("content-length") {
                    content_length = std::str::from_utf8(h.value)?.trim().parse()?;
                } else if h.name.eq_ignore_ascii_case("authorization") {
                    authorization = Some(String::from_utf8_lossy(h.value).into_owned());
                }
            }
            Ok(Some(Head {
                method: req.method.unwrap_or("").to_string(),
                path: req.path.unwrap_or("").to_string(),
                content_length,
                authorization,
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
        ("POST", "/v1/chat/completions") => {
            if !auth_ok(&shared.token, &head.authorization) {
                return write_json(&mut stream, 401, &json!({"error": "unauthorized"})).await;
            }
            completion(&mut stream, &shared, &body).await
        }
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

/// The session id: OpenAI's `user` field, which every framework can set.
fn session_key(body: &Value) -> String {
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
/// creating the session on first use. Returns the slot and the token the new
/// run will honour.
async fn take_slot(shared: &Arc<Shared>, key: &str) -> Result<(Box<Slot>, CancellationToken)> {
    for _ in 0..200 {
        {
            let mut slots = shared.slots.lock().await;
            match slots.remove(key) {
                None => {
                    let session = Session::create(
                        &shared.session_dir,
                        SessionMeta {
                            id: Session::new_id(),
                            created_at: chrono::Utc::now(),
                            provider: shared.provider_name.clone(),
                            model: shared.model.clone(),
                            workspace: shared.agent.context().tools.workspace.clone(),
                            title: Some(format!("voice: {key}")),
                        },
                    )?;
                    session.append(&Record::Config(RunConfig::of(
                        &shared.agent,
                        &shared.config,
                        &shared.provider_name,
                    )))?;
                    let token = CancellationToken::new();
                    slots.insert(key.to_string(), SlotState::Running(token.clone()));
                    return Ok((
                        Box::new(Slot {
                            convo: Conversation::new(),
                            session,
                        }),
                        token,
                    ));
                }
                Some(SlotState::Idle(slot)) => {
                    let token = CancellationToken::new();
                    slots.insert(key.to_string(), SlotState::Running(token.clone()));
                    return Ok((slot, token));
                }
                Some(SlotState::Running(tok)) => {
                    // Barge-in: cancel and wait for the slot to come back.
                    tok.cancel();
                    slots.insert(key.to_string(), SlotState::Running(tok));
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    anyhow::bail!("session {key} is busy and did not yield within 20s")
}

async fn completion(stream: &mut TcpStream, shared: &Arc<Shared>, body: &[u8]) -> Result<()> {
    let body: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => {
            return write_json(stream, 400, &json!({"error": "invalid JSON body"})).await;
        }
    };
    let key = session_key(&body);
    let Some(text) = last_user_text(&body) else {
        return write_json(stream, 400, &json!({"error": "no user message"})).await;
    };
    let want_stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);

    let (mut slot, cancel) = take_slot(shared, &key).await?;

    // From here the slot must always find its way back into the map, so
    // nothing below uses `?` until it has.
    let mut cx = (**shared.agent.context()).clone();
    cx.cancel = Some(cancel.clone());
    // Its own outbox route carrying this session's id — the shared route is
    // one Arc across every session, and the stamp is what attributes a draft
    // to the run that wrote it (the connector's rule).
    if let Some(shared_route) = &shared.agent.context().outbox {
        if let Ok(store) = OutboxStore::open(&shared.outbox_root) {
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
    }

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
        let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n";
        if stream.write_all(head.as_bytes()).await.is_err() {
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

    let (slot, outcome) = match run.await {
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
            let _ = slot.session.record_run(&recorded, &slot.convo);
            let _ = slot.session.record_outcome(o);
            let _ = slot.session.append(&Record::Taint(slot.convo.taint));
        }
        Err(e) => tracing::error!("voice run failed: {e}"),
    }

    if want_stream {
        if !disconnected {
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
    fn session_key_reads_the_user_field_and_defaults() {
        assert_eq!(
            session_key(&serde_json::json!({"user": "call-7"})),
            "call-7"
        );
        assert_eq!(session_key(&serde_json::json!({})), "default");
        assert_eq!(session_key(&serde_json::json!({"user": ""})), "default");
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
