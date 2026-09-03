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
//! - **A call may be someone else's conversation** (D3). When the caller
//!   names one in `X-Chat-Session` and a [`SessionHost`] owns it, the turn
//!   runs *there* — same messages, same taint, same transcript, same jail —
//!   and this module keeps no record of it at all. Talking and typing stop
//!   being two threads. An unnamed or unrecognised key still gets a slot of
//!   its own, so nothing that worked before D3 stopped working.
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
use mecha_core::agent::{is_plain_user_text, Agent, AgentEvent, Conversation};
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

pub mod confirm;

/// Loopback, by design rather than default — see the module docs.
const LISTEN_HOST: &str = "127.0.0.1";

/// The largest request body honoured. An utterance is text; even a pasted
/// document fits with room to spare, and a declared Content-Length must
/// never size an allocation on its own say-so.
const MAX_BODY_BYTES: usize = 8 << 20;

/// D10: the one load-bearing prompt. Static byte-for-byte across sessions,
/// because it rides in the cached prefix and TTFT is the latency budget.
///
/// The first-sentence rule is a latency control, not a style note.
/// Pipecat's TTS service aggregates by sentence, so time-to-first-sound is
/// the synthesis cost of sentence one alone -- measured on Chatterbox
/// Turbo at ~3.9x realtime: "Sure." is 0.33 s, a full clause is 1.08 s.
/// It pays twice, because a short opener is also fewer tokens to generate
/// before speech can start at all. Every other line here shapes the whole
/// reply; this one is the only one on the critical path.
pub(crate) const VOICE_BLOCK: &str = "\
Voice mode: everything you write is spoken aloud by a text-to-speech voice, \
and the user is listening, not reading. Answer in short conversational \
sentences. Make the first sentence a short one, a handful of words: \
speaking begins as soon as that sentence is finished, so a long opener \
is silence the listener sits through. Never use markdown, bullet lists, \
headings, tables or code blocks; write numbers, dates and times as they \
are spoken. When a tool \
returns something long, say the gist in a sentence or two instead of \
reciting it. Before a slow step, say one short line about what you are \
doing. When a message, email or calendar change was staged for review \
rather than sent, say one short clause and nothing more -- \"That is \
drafted.\" -- and do NOT describe what is in it or where it is waiting: \
the harness reads the draft back word for word and asks whether to send \
it, so anything you add is the same thing said twice. Keep replies brief \
unless the user asks you to go deep.";

// ------------------------------------------------------- the hosted door
//
// D3: a call the page named speaks into *that* conversation, rather than
// into a second one of the facade's own. The seam is a trait so `voice/`
// never learns that `serve/` exists — the `Approver`/`Asker` shape, applied
// to "whose conversation is this".
//
// The alternative considered and rejected was merging a voice conversation
// onto the web session when the call ends: much smaller, and it buys a
// duplicate record — the same turns in two session JSONLs for `recall`,
// `distill` and the run-quality corpus each to count twice.

/// What a hosted turn hands back: the run's events as they happen, the
/// answer when it lands, and the handle that stops it. Deliberately the
/// same three things a facade-owned slot produces, so the SSE pump below
/// cannot behave differently depending on whose conversation answered.
pub struct HostedTurn {
    pub events: tokio::sync::mpsc::UnboundedReceiver<AgentEvent>,
    pub done: tokio::sync::oneshot::Receiver<Result<HostedAnswer, String>>,
    pub cancel: CancellationToken,
}

/// A hosted run's outcome, in the currency this facade answers in.
pub struct HostedAnswer {
    pub text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// §6.2's readout, lowercase and `None` on `Neutral` — the same value
    /// the page's `WireEvent::Affect` carries, computed once and threaded
    /// out through both doors.
    pub affect: Option<String>,
}

/// What asking a host to speak can come to.
pub enum Hosted {
    Started(Box<HostedTurn>),
    /// Not a conversation this host owns — an invalid key, or a call that
    /// named nothing. The facade falls back to its own slot map, which is
    /// what every call did before D3.
    Unknown,
    /// The conversation would not come free inside the barge-in window,
    /// usually a tool call outlasting it. Answered exactly as a busy slot is.
    Busy,
    Failed(String),
}

/// A front-end that owns conversations a call can speak into.
#[async_trait::async_trait]
pub trait SessionHost: Send + Sync {
    /// Start a spoken turn on `key`, barging in on any run in flight.
    ///
    /// `approve_all` is `--voice-yes` travelling with the turn rather than
    /// with the conversation: the owner is present and speaking, and an
    /// approval card cannot be tapped mid-sentence. It is deliberately not
    /// the host's posture to decide, and deliberately not sticky — a typed
    /// turn in the same conversation still runs at whatever the page says.
    async fn speak(&self, key: &str, utterance: &str, approve_all: bool) -> Hosted;
}

/// Open a spoken turn with the D10 block when the conversation has not just
/// been spoken into — at the start of a call, and again after any typed
/// turn, because the model has been writing for a reader since.
///
/// One rule, two callers: a facade slot is spoken-only, so "the previous
/// turn was spoken" is exactly "the conversation is not empty"; a hosted
/// conversation carries the flag because typed and spoken turns share it.
/// Prepending costs nothing in cache terms — the transcript is append-only,
/// so the block lands at the end and every earlier byte still matches.
pub(crate) fn open_spoken_turn(text: &str, previous_turn_was_spoken: bool) -> String {
    if previous_turn_was_spoken {
        text.to_string()
    } else {
        format!("{VOICE_BLOCK}\n\n{text}")
    }
}

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

/// How the facade is mounted. Standalone (`Mount::default()`) is what
/// `mecha voice-serve` builds its own agent for; the unified `mecha serve`
/// fills all three in. A struct rather than three positional flags, because
/// two of them are booleans that mean opposite things about the same run.
#[derive(Default)]
pub struct Mount {
    /// True when the agent is shared with other front-ends (the unified
    /// `mecha serve`): the D10 voice block then cannot ride the system
    /// prompt, so it opens each spoken stretch of a conversation instead —
    /// one copy per stretch, cached thereafter.
    pub inject_voice_block: bool,
    /// The owner-present posture for voice runs. The shared agent carries
    /// the config's approver — `Ask`, which a non-interactive run answers
    /// with Blocked — so a mounted facade must say so explicitly or every
    /// voice tool call is refused ("I don't have access to your calendar",
    /// live, 2026-08-24). Outbox routing is untouched: sends still stage.
    pub approve_all: bool,
    /// The front-end whose conversations a call may speak into (D3). None
    /// is the pre-unification world: every call is the facade's own.
    pub host: Option<Arc<dyn SessionHost>>,
}

struct Shared {
    agent: Arc<Agent>,
    mount: Mount,
    slots: Mutex<HashMap<String, SlotState>>,
    session_dir: PathBuf,
    outbox_root: PathBuf,
    provider_name: String,
    model: String,
    config: mecha_core::config::Config,
    /// For `RunConfig::levers_off`; the switches are not readable off the
    /// shared agent, so the front-end that built it hands them over.
    levers_off: Vec<mecha_core::harness::Lever>,
    token: Option<String>,
    /// The open "send it?" question per conversation. Lives beside the slots
    /// rather than inside one, because a hosted call (D3) has no slot here
    /// and still gets asked.
    confirmations: confirm::Confirmations,
    /// §6.2's voice readout — the last non-`Neutral` label a hosted turn
    /// (D3) came back with, per chat session key.
    ///
    /// **Lags by one turn, honestly rather than by accident.** `Affect` is a
    /// function of the *finished* `RunOutcome` (`appraisal::live`), and
    /// `pump` streams the answer's text — and therefore starts feeding
    /// Pipecat's TTS, sentence by sentence — before `hosted_completion`
    /// updates this map, which happens only after `turn.done` resolves. So
    /// the mood a sentence is spoken with is whichever turn's label was
    /// cached *before* this one started, not this one's own. There is no
    /// way to do better without holding speech until the whole answer is
    /// known, which is the latency the streaming exists to avoid — this is
    /// a documented trade-off, not a bug to chase.
    ///
    /// Set-and-overwrite, never take-once: the worker latches this once per
    /// answer (`on_turn_context_created`, not a per-sentence poll — a
    /// consuming read here would starve every sentence after the first).
    /// Cleared on
    /// **every** completed turn regardless of outcome — `Neutral` removes
    /// the entry, and so does an error, on the same "silence means neutral"
    /// rule the TUI's `Err` arm and the web page's `sawAffectThisRun` both
    /// follow — so a synthesis for a later turn never inherits a stale
    /// mood from one that failed.
    ///
    /// **Only the `chat:{id}` half of this map's key space is ever
    /// written.** `hosted_completion` (D3) is the one writer; the
    /// facade's own unhosted slot path (`voice:{key}`, used when a call
    /// names no chat session, or falls through on `Hosted::Unknown`) never
    /// computes an `Affect` at all — that would need `appraisal::live` on
    /// a conversation this module keeps entirely to itself, which is a
    /// real feature and not yet built. Found on review: the `Hosted::
    /// Unknown` fall-through is the one place this matters live, since
    /// `confirm_key` there is still `chat:{id}` from the call that named a
    /// session nobody held, and the turn runs unhosted anyway — cleared at
    /// that fall-through rather than left to answer with an earlier
    /// hosted turn's mood.
    affects: Mutex<HashMap<String, String>>,
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
        levers_off: Vec<mecha_core::harness::Lever>,
        outbox_root: PathBuf,
        token: Option<String>,
        mount: Mount,
    ) -> Result<Self> {
        Ok(Self {
            shared: Arc::new(Shared {
                agent,
                mount,
                slots: Mutex::new(HashMap::new()),
                session_dir: Session::default_dir()?,
                outbox_root,
                provider_name,
                model,
                config,
                levers_off,
                token,
                confirmations: confirm::Confirmations::default(),
                affects: Mutex::new(HashMap::new()),
            }),
        })
    }

    /// Bind the loopback listener — separated from serving so a host can
    /// announce *after* the bind succeeds. Announcing first was a live bug:
    /// a second serve printed "voice facade on :8990" while another process
    /// held the port, and the bind error died inside a task nobody read —
    /// the silently-degrading shape, caught by the remote-surface session.
    pub async fn bind(&self, port: u16) -> Result<TcpListener> {
        let addr = format!("{LISTEN_HOST}:{port}");
        TcpListener::bind(&addr)
            .await
            .with_context(|| format!("binding {addr}"))
    }

    /// Serve on a bound listener until cancelled. Signal handling
    /// deliberately lives with the caller — a mounted facade must not
    /// compete with its host process for SIGTERM.
    pub async fn serve(&self, listener: TcpListener, stop: CancellationToken) -> Result<()> {
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
        prepared.levers_off,
        outbox_root,
        args.token.clone(),
        // Standalone: the voice block already rides this agent's system
        // prompt via system_extra, so nothing to inject per conversation;
        // the launch flags (--yes/--read-only) already shaped the agent's
        // own approver, so no override either; and there is no other
        // front-end in this process holding conversations to speak into.
        Mount::default(),
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
    let listener = facade.bind(args.port).await?;
    let server = facade.serve(listener, stop.clone());
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
    /// `X-Chat-Session`: a conversation the *caller* named, for the facade
    /// to speak into rather than opening one of its own (D3). Deliberately
    /// a second header rather than a namespace inside the first — one
    /// header carrying two meanings is a value nobody can validate, and a
    /// page is free to name a session `webrtc-anything`.
    chat: Option<String>,
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
            let mut chat = None;
            for h in req.headers.iter() {
                if h.name.eq_ignore_ascii_case("content-length") {
                    content_length = std::str::from_utf8(h.value)?.trim().parse()?;
                } else if h.name.eq_ignore_ascii_case("authorization") {
                    authorization = Some(String::from_utf8_lossy(h.value).into_owned());
                } else if h.name.eq_ignore_ascii_case("x-voice-session") {
                    session = Some(String::from_utf8_lossy(h.value).trim().to_string());
                } else if h.name.eq_ignore_ascii_case("x-chat-session") {
                    chat = Some(String::from_utf8_lossy(h.value).trim().to_string());
                }
            }
            Ok(Some(Head {
                method: req.method.unwrap_or("").to_string(),
                path: req.path.unwrap_or("").to_string(),
                content_length,
                authorization,
                session,
                chat,
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

    // Routing ignores the query string; `/v1/mecha-affect` is the only
    // route that reads one, off `head.path` itself.
    let route = head.path.split('?').next().unwrap_or(&head.path);
    match (head.method.as_str(), route) {
        ("GET", "/health") => write_json(&mut stream, 200, &json!({"status": "ok"})).await,
        ("POST", "/v1/chat/completions") => completion(&mut stream, &shared, &head, &body).await,
        ("GET", "/v1/mecha-affect") => affect_status(&mut stream, &shared, &head).await,
        _ => write_json(&mut stream, 404, &json!({"error": "not found"})).await,
    }
}

/// §6.2's voice side-channel. Not authenticated, matching `/health`: this is
/// loopback-only like everything else in this module, and the worst a
/// compromised loopback caller learns is which of four mood words a session
/// last carried — no more sensitive than the health check already answers
/// with no token.
///
/// A plain HTTP GET rather than piggybacking the OpenAI-compatible
/// completion response: `worker.py`'s LLM service is a stock
/// `pipecat.services.openai.llm.OpenAILLMService`, which parses that
/// response through the real `openai` SDK's typed models — an unrecognised
/// top-level field there is silently dropped before any pipecat frame
/// processor ever sees it, so that channel cannot carry this.
async fn affect_status(stream: &mut TcpStream, shared: &Arc<Shared>, head: &Head) -> Result<()> {
    let Some(session) = query_param(&head.path, "session") else {
        return write_json(stream, 400, &json!({"error": "missing ?session="})).await;
    };
    // Cloned out from under the lock before the socket write: a `match`
    // scrutinee temporary lives to the end of the match, so holding the
    // guard here would keep it locked across `write_json`'s `.await` (and
    // the 204 arm's `write_all`) — and `hosted_completion` takes this same
    // lock at the end of every turn, so a caller that stops reading would
    // stall the completion path, not just its own poll.
    let label = shared.affects.lock().await.get(session.as_str()).cloned();
    match label {
        Some(label) => write_json(stream, 200, &json!({"affect": label})).await,
        None => {
            stream
                .write_all(b"HTTP/1.1 204 No Content\r\nconnection: close\r\n\r\n")
                .await?;
            Ok(())
        }
    }
}

/// Hand-rolled rather than pulling in a URL/query-string crate: this module
/// already hand-rolls its whole HTTP surface (it is a raw-socket loopback
/// facade, not an axum app), and one query parameter does not change that.
///
/// **Must decode.** The cache is keyed by `confirm_key` — `"chat:{id}"` or
/// `"voice:{slot}"` — which contains a colon, and `worker.py` sends it via
/// httpx's `params=`, which percent-encodes it (`urlencode`'s default
/// `quote_plus`, `safe=''`) to `chat%3Amain`. A version of this that
/// returned the raw slice looked right, built, and passed every existing
/// test — because every test used a colon-free key — while missing every
/// real lookup in production and always falling back to the baseline
/// silently, exactly as the caller's own failure path is designed to.
fn query_param(path: &str, key: &str) -> Option<String> {
    let (_, query) = path.split_once('?')?;
    query
        .split('&')
        .find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            (k == key).then_some(v)
        })
        .map(percent_decode)
}

/// `%XX` and `+` only — the two forms a query string actually needs and the
/// two `urlencode` actually produces. A malformed escape (`%` with fewer
/// than two hex digits after it, or non-hex digits) is passed through
/// byte-for-byte rather than dropped, on the same "absent is not zero, and
/// a caller cannot check for it" fail-safe reasoning as everywhere else in
/// this codebase that refuses to silently discard bytes.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 3 <= bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
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
                            kind: Some(mecha_core::session::SessionKind::Voice),
                        },
                    )
                    .and_then(|session| {
                        session.append(&Record::Config(RunConfig::of(
                            &shared.agent,
                            &shared.config,
                            &shared.provider_name,
                            &shared.levers_off,
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

/// Stream the reply as the run produces it, or (non-streaming) drain the
/// events without rendering them. Returns true when the client hung up —
/// the third spelling of interrupt, which cancels.
///
/// Shared by the facade's own slots and by a hosted conversation on
/// purpose: what the worker hears must not depend on whose conversation
/// answered it.
async fn pump(
    stream: &mut TcpStream,
    id: &str,
    model: &str,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<AgentEvent>,
    cancel: &CancellationToken,
    want_stream: bool,
) -> bool {
    if !want_stream {
        while rx.recv().await.is_some() {}
        return false;
    }
    let mut disconnected = false;
    let head_bytes = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n";
    if stream.write_all(head_bytes.as_bytes()).await.is_err() {
        cancel.cancel();
        disconnected = true;
    }
    if !disconnected {
        let first = sse_chunk(id, model, json!({"role": "assistant"}), None);
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
                    let chunk = sse_chunk(id, model, json!({"content": t}), None);
                    if write_chunk(stream, chunk.as_bytes()).await.is_err() {
                        cancel.cancel();
                        disconnected = true;
                    }
                }
                Some(_) => {}
                // The run dropped its sender: it is over. The caller
                // collects the outcome.
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
    disconnected
}

/// Close the SSE body. A failure must be *audible*: a clean "stop" after
/// silence reads as the assistant ignoring the user, and the server-side
/// log is the one place a voice user will never look.
async fn finish_stream(stream: &mut TcpStream, id: &str, model: &str, error: Option<&str>) {
    if let Some(e) = error {
        let spoken = sse_chunk(
            id,
            model,
            json!({"content": format!("I hit a problem and could not answer: {e}")}),
            None,
        );
        let _ = write_chunk(stream, spoken.as_bytes()).await;
    }
    let done = sse_chunk(id, model, json!({}), Some("stop"));
    let _ = write_chunk(stream, done.as_bytes()).await;
    let _ = write_chunk(stream, b"data: [DONE]\n\n").await;
    let _ = stream.write_all(b"0\r\n\r\n").await;
}

/// A turn spoken into a conversation another front-end owns (D3). The
/// facade keeps no state for it at all — no slot, no session file, no
/// conversation — because a second copy of any of those is the duplicate
/// record this shape exists to avoid.
#[allow(clippy::too_many_arguments)]
async fn hosted_completion(
    stream: &mut TcpStream,
    shared: &Arc<Shared>,
    id: &str,
    want_stream: bool,
    mut turn: HostedTurn,
    confirm_key: &str,
    baseline: &Option<std::collections::HashSet<String>>,
) -> Result<()> {
    let disconnected = pump(
        stream,
        id,
        &shared.model,
        &mut turn.events,
        &turn.cancel,
        want_stream,
    )
    .await;
    let answer = turn
        .done
        .await
        .unwrap_or_else(|_| Err("the run ended without answering".to_string()));
    // §6.2: cache (or clear) this turn's label. Deliberately *after* `pump`
    // has already streamed this turn's own text above — the label is a
    // function of the finished `RunOutcome`, so it cannot exist before this
    // point, and this turn's own sentences have already gone to the worker
    // by the time it lands. What this sets is what the *next* turn's
    // sentences will poll (the `affects` field's own docstring is the honest
    // account of that lag).
    //
    // Cleared on `Err` too, not only on an `Ok` answer with no label — a
    // failed run has nothing to report, and leaving the previous turn's
    // entry in place here would be exactly the staleness the rest of this
    // cache exists to avoid, one arm over.
    {
        let mut affects = shared.affects.lock().await;
        match &answer {
            Ok(a) => match &a.affect {
                Some(label) => {
                    affects.insert(confirm_key.to_string(), label.clone());
                }
                None => {
                    affects.remove(confirm_key);
                }
            },
            Err(_) => {
                affects.remove(confirm_key);
            }
        }
    }
    // The offer comes after the model's own words and only when the turn
    // produced an answer: a run that failed has staged nothing worth
    // confirming, and asking about drafts on top of an error is a question
    // over the top of the thing that needs saying.
    let offer = match &answer {
        Ok(_) => offer_for_turn(shared, confirm_key, baseline).await,
        Err(_) => None,
    };
    if want_stream {
        if !disconnected {
            if let Some(offer) = &offer {
                say(stream, shared, id, &format!(" {offer}")).await;
            }
            finish_stream(
                stream,
                id,
                &shared.model,
                answer.as_ref().err().map(|e| &**e),
            )
            .await;
        }
        return Ok(());
    }
    match answer {
        Ok(a) => {
            let content = match &offer {
                Some(offer) => format!("{} {offer}", a.text),
                None => a.text,
            };
            write_json(
                stream,
                200,
                &json!({
                    "id": id,
                    "object": "chat.completion",
                    "created": chrono::Utc::now().timestamp(),
                    "model": shared.model,
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": content},
                        "finish_reason": "stop",
                    }],
                    "usage": {
                        "prompt_tokens": a.input_tokens,
                        "completion_tokens": a.output_tokens,
                    },
                }),
            )
            .await
        }
        Err(e) => write_json(stream, 500, &json!({"error": e})).await,
    }
}

/// The ids of everything currently waiting in the outbox.
///
/// `None` when the store could not be read, and that is not the same as an
/// empty set: no baseline means no diff, and no diff means nothing is
/// offered — a surface that could not look before the run must not conclude
/// afterwards that the whole backlog is new.
fn pending_outbox_ids(root: &std::path::Path) -> Option<std::collections::HashSet<String>> {
    OutboxStore::open(root).ok()?.items().ok().map(|items| {
        items
            .into_iter()
            .filter(|i| i.status == "pending")
            .map(|i| i.id)
            .collect()
    })
}

/// The question to ask about whatever this turn staged, if anything.
///
/// Runs after every spoken turn, hosted or not. The web page makes the same
/// offer as a card for the same drafts — deliberately: a call and the page
/// are two views of one conversation, and whichever one the owner acts in,
/// the other's copy simply finds the item already resolved.
async fn offer_for_turn(
    shared: &Arc<Shared>,
    confirm_key: &str,
    baseline: &Option<std::collections::HashSet<String>>,
) -> Option<String> {
    let baseline = baseline.as_ref()?;
    let staged = crate::review_policy::staged_since(
        OutboxStore::open(&shared.outbox_root).ok()?.items().ok()?,
        baseline,
    );
    let offer = confirm::compose_offer(&staged)?;
    shared.confirmations.set(confirm_key, offer.pending).await;
    Some(offer.speech)
}

/// Say something the harness composed, on whichever channel this request
/// wanted. One utterance, one place, so the streaming and blocking paths
/// cannot word the same fact differently.
async fn say(stream: &mut TcpStream, shared: &Arc<Shared>, id: &str, text: &str) {
    let chunk = sse_chunk(id, &shared.model, json!({"content": text}), None);
    let _ = write_chunk(stream, chunk.as_bytes()).await;
}

/// Answer a spoken yes/later/read-it-out, without running a model turn.
///
/// `None` means the words were not an answer at all: the caller carries on to
/// the model with them, and the question is already gone from the store of
/// pending confirmations — dropped rather than held, so a "yes" three turns
/// later cannot land on a draft nobody was talking about any more.
async fn answer_completion(
    stream: &mut TcpStream,
    shared: &Arc<Shared>,
    id: &str,
    want_stream: bool,
    confirm_key: &str,
    pending: &confirm::Pending,
    text: &str,
) -> Option<Result<()>> {
    // The draft as it is *now*, not as it was when the question was asked:
    // it may have been sent from the page, edited there, or swept in between.
    let head = pending
        .queue
        .front()
        .and_then(|id| confirm::item_now(&shared.outbox_root, id));
    // The one after it, so a reply that moves on can ask a whole question
    // rather than promising a word the parser does not know.
    let next = pending
        .queue
        .get(1)
        .and_then(|id| confirm::item_now(&shared.outbox_root, id));
    match confirm::react(text, pending, head.as_ref(), next.as_ref()) {
        confirm::Reaction::PassToModel => None,
        confirm::Reaction::Reread(said) => {
            // The head is still the open question: hearing it again is not
            // answering it.
            shared.confirmations.set(confirm_key, pending.clone()).await;
            Some(finish_with(stream, shared, id, want_stream, &said).await)
        }
        confirm::Reaction::Say(said) => {
            let mut rest = pending.clone();
            rest.queue.pop_front();
            shared.confirmations.set(confirm_key, rest).await;
            Some(finish_with(stream, shared, id, want_stream, &said).await)
        }
        confirm::Reaction::Release {
            acknowledge,
            id: item,
        } => {
            // The acknowledgement goes out *before* the work: a release
            // rebuilds a tool surface and can take seconds, and silence after
            // "yes" reads as the call having dropped.
            if want_stream {
                say(stream, shared, id, &acknowledge).await;
            }
            let outcome = confirm::release(&item).await;
            let mut rest = pending.clone();
            rest.queue.pop_front();
            let report = confirm::report_release(outcome, next.as_ref());
            shared.confirmations.set(confirm_key, rest).await;
            let spoken = if want_stream {
                report
            } else {
                format!("{acknowledge} {report}")
            };
            Some(finish_with(stream, shared, id, want_stream, &spoken).await)
        }
    }
}

/// Close out a harness-authored reply on whichever channel was asked for.
async fn finish_with(
    stream: &mut TcpStream,
    shared: &Arc<Shared>,
    id: &str,
    want_stream: bool,
    text: &str,
) -> Result<()> {
    if want_stream {
        say(stream, shared, id, text).await;
        finish_stream(stream, id, &shared.model, None).await;
        return Ok(());
    }
    write_json(
        stream,
        200,
        &json!({
            "id": id,
            "object": "chat.completion",
            "created": chrono::Utc::now().timestamp(),
            "model": shared.model,
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": text},
                "finish_reason": "stop",
            }],
            // No model ran, so no tokens were spent. Reporting a guess here
            // would put fiction into whatever counts them.
            "usage": {"prompt_tokens": 0, "completion_tokens": 0},
        }),
    )
    .await
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
    let id = format!("chatcmpl-{}", Session::new_id());

    // The conversation this utterance belongs to, for the purpose of "was a
    // question asked here". A hosted call names a chat session and keeps no
    // slot; an unhosted one has a slot and no chat key. They must not collide,
    // so the namespace is part of the key.
    let confirm_key = match head.chat.as_deref().filter(|c| !c.is_empty()) {
        Some(chat) => format!("chat:{chat}"),
        None => format!("voice:{key}"),
    };

    // **Before the model sees a word**: is this an answer to a question the
    // harness asked out loud? Release policy must not be decidable by
    // anything sharing a context window with third-party text, and the way
    // that rule is kept here is that the decision never enters one — the
    // question was composed from the store and the answer is matched by
    // `review_policy::parse_answer`, which recognises a bare yes and nothing
    // else. Anything that is not an answer drops the question and falls
    // through to the model as ordinary words.
    if let Some(pending) = shared.confirmations.take(&confirm_key).await {
        if let Some(handled) = answer_completion(
            stream,
            shared,
            &id,
            want_stream,
            &confirm_key,
            &pending,
            &text,
        )
        .await
        {
            return handled;
        }
    }

    // What was already waiting before this turn. Taken here, before anything
    // runs, so a draft this turn stages cannot appear in its own baseline.
    let outbox_baseline = pending_outbox_ids(&shared.outbox_root);

    // D3: the caller named a conversation, so speak into that one. Only a
    // key the host does not recognise falls through to the facade's own
    // slot — a warning rather than a refusal, because the fallback is
    // exactly what every call did before D3 and a dead call is a worse
    // answer than an unshared one. What the fall-through costs is visible
    // where it matters: the page's transcript simply does not move.
    if let (Some(chat_key), Some(host)) = (&head.chat, &shared.mount.host) {
        if !chat_key.is_empty() {
            match host.speak(chat_key, &text, shared.mount.approve_all).await {
                Hosted::Started(turn) => {
                    return hosted_completion(
                        stream,
                        shared,
                        &id,
                        want_stream,
                        *turn,
                        &confirm_key,
                        &outbox_baseline,
                    )
                    .await
                }
                Hosted::Busy => return write_json(
                    stream,
                    503,
                    &json!({"error": "still finishing the previous step — try again in a moment"}),
                )
                .await,
                Hosted::Failed(e) => {
                    tracing::error!("voice turn on chat session {chat_key:?} failed: {e}");
                    return write_json(stream, 500, &json!({"error": e})).await;
                }
                Hosted::Unknown => {
                    tracing::warn!(
                        "voice call named chat session {chat_key:?}, which no front-end \
                         holds — answering in a conversation of its own instead"
                    );
                    // `confirm_key` is still `chat:{chat_key}` below — this
                    // turn runs in the facade's own untracked slot instead,
                    // so nothing will update that entry for it the way
                    // `hosted_completion` does. Cleared rather than left
                    // stale: an earlier hosted turn's label must not be
                    // spoken over an answer it has nothing to do with.
                    shared.affects.lock().await.remove(&confirm_key);
                }
            }
        }
    }

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
    if shared.mount.approve_all {
        cx.approver = Arc::new(mecha_core::tool::ModeApprover {
            mode: mecha_core::config::PermissionMode::Allow,
        });
    }
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
    // spoken stretch opens with it. A facade slot is spoken-only, so "the
    // previous turn was spoken" is exactly "this conversation is not new".
    let text = if shared.mount.inject_voice_block {
        open_spoken_turn(&text, !slot.convo.is_empty())
    } else {
        text
    };
    // Folded, not pushed, when the tail is already a user message — a
    // barge-in mid-tool-turn leaves the transcript ending on the user
    // message carrying tool results, and pushing there makes two user
    // messages in a row (invalid on the Anthropic backend, merely tolerated
    // by llama-server). Recorded at submit either way, so `recorded` is
    // what the file holds — the contract every arm downstream assumes.
    let folded = slot
        .convo
        .messages
        .last()
        .is_some_and(|m| m.role == mecha_core::message::Role::User);
    let recorded = if folded {
        mecha_core::agent::append_user_text(&mut slot.convo.messages, text.clone());
        // One direct `Rewrite`, recorded at submit like every other surface
        // — `record_run` between runs would replay the previous run's
        // still-uncleared `rewritten` states. Best-effort (`let _`), which
        // is this surface's posture for every session write.
        let _ = slot.session.append(&Record::Rewrite {
            messages: slot.convo.messages.clone(),
        });
        slot.convo.messages.clone()
    } else {
        let user = Message::user(&text);
        slot.convo.push(user.clone());
        let _ = slot.session.append(&Record::Message(user));
        slot.convo.messages.clone()
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
    let agent = Arc::clone(&shared.agent);
    let run = tokio::spawn(async move {
        let outcome = agent.run_in(&cx, &mut slot.convo, Some(tx)).await;
        (slot, outcome)
    });

    let disconnected = pump(stream, &id, &shared.model, &mut rx, &cancel, want_stream).await;

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
            //
            // Only a *plain* user tail — found on review, the fifth pop
            // site: a barge-in mid-tool-turn also lands here (`run_in`
            // pushes the tool results and reads the cancel at the top of
            // the *next* turn, returning Ok(Interrupted)), and tool results
            // ride in a `Role::User` message, so the bare role check popped
            // them and orphaned the assistant's `tool_use` — which
            // `record_run` then persisted, 400ing the slot and any resume
            // of the id. A completed tool round is a valid tail; only two
            // user *texts* in a row are not, and the next push folds into
            // it the way steering already does.
            if slot.convo.messages.last().is_some_and(is_plain_user_text) {
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
            // collide with — `Conversation::roll_back_failed_turn` carries
            // the reasoning. This was the fourth failed-turn arm, found on
            // review missing what the other three had each grown their own
            // copy of — and the worst of the four to miss: the slot survives
            // in `shared.slots`, so the *next successful* turn's record_run
            // diffed memory against memory and appended its tail on top of
            // the user turn the file still held, permanently writing two
            // consecutive user messages that 400 anything resuming the id.
            // Recording the rolled-back state (a rewrite) is what keeps the
            // file agreeing with memory; taint is kept either way — a failed
            // turn that read a hostile page still read it. The rollback
            // itself knows a folded turn from a pushed one (its pop is
            // conditional on a plain user tail), so no flag travels from the
            // push site to here. And the record is the one write whose
            // failure reproduces the resume-time 400 this arm exists to
            // prevent, so it is the one that must not fail silently.
            slot.convo.roll_back_failed_turn(recorded.clone());
            if let Err(e) = slot.session.record_run(&recorded, &slot.convo) {
                tracing::warn!(
                    "the rollback was not recorded — a resume of this session \
                     will replay the failed turn: {e:#}"
                );
            }
            let _ = slot.session.append(&Record::Taint(slot.convo.taint));
        }
    }

    // Same rule as the hosted path: the drafts this turn staged are offered
    // after the answer, and only when there was one.
    let offer = match &outcome {
        Ok(_) => offer_for_turn(shared, &confirm_key, &outbox_baseline).await,
        Err(_) => None,
    };

    if want_stream {
        if !disconnected {
            if let Some(offer) = &offer {
                say(stream, shared, &id, &format!(" {offer}")).await;
            }
            let failed = outcome.as_ref().err().map(|e| format!("{e:#}"));
            finish_stream(stream, &id, &shared.model, failed.as_deref()).await;
        }
    } else {
        match &outcome {
            Ok(o) => {
                let content = match &offer {
                    Some(offer) => format!("{} {offer}", o.text),
                    None => o.text.clone(),
                };
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
                            "message": {"role": "assistant", "content": content},
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

    /// The fifth pop site's regression, pinned at the predicate: the old
    /// guard was `role == User`, which is true of the message carrying tool
    /// results — so a barge-in mid-tool-turn popped them and orphaned the
    /// `tool_use`. Only the owner's own dangling text may be trimmed.
    #[test]
    fn a_tool_result_tail_is_not_a_poppable_user_turn() {
        use mecha_core::message::{Block, Role};
        let plain = Message::user("say that again?");
        assert!(is_plain_user_text(&plain));

        let results = Message {
            role: Role::User,
            content: vec![
                Block::ToolResult {
                    tool_use_id: "t1".into(),
                    content: "ok".into(),
                    is_error: false,
                },
                // Steering text riding beside results still counts as a tool
                // round — popping it would orphan the `tool_use` all the same.
                Block::text("change of plan"),
            ],
        };
        assert!(!is_plain_user_text(&results));

        let assistant = Message::assistant(vec![Block::text("done")]);
        assert!(!is_plain_user_text(&assistant));
    }

    /// The barge-in composition end to end: an interrupted tool round keeps
    /// its pair on disk, and the next utterance folds rather than pushes —
    /// no orphaned `tool_use`, no two user texts in a row, and a resume
    /// loads exactly what memory holds.
    #[test]
    fn a_barge_in_mid_tool_turn_keeps_the_round_and_folds_the_next_utterance() {
        use mecha_core::message::{Block, Role};
        let dir = std::env::temp_dir().join(format!(
            "mecha-voice-bargein-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let session = mecha_core::session::Session::create(
            &dir,
            mecha_core::session::SessionMeta {
                id: "20260101T000000-bargein".into(),
                created_at: chrono::Utc::now(),
                provider: "local".into(),
                model: "m".into(),
                workspace: std::path::PathBuf::from("/tmp"),
                title: None,
                kind: Some(mecha_core::session::SessionKind::Voice),
            },
        )
        .unwrap();

        // Turn 1, as the Ok(Interrupted) arm sees it: the user spoke, the
        // run opened a tool round, the barge-in cancelled after the results
        // landed.
        let user = Message::user("look that up");
        session
            .append(&mecha_core::session::Record::Message(user.clone()))
            .unwrap();
        let recorded = vec![user];
        let mut convo = Conversation::from(recorded.clone());
        convo.messages.push(Message::assistant(vec![Block::ToolUse {
            id: "t1".into(),
            name: "web_search".into(),
            input: serde_json::json!({}),
        }]));
        convo.messages.push(Message {
            role: Role::User,
            content: vec![Block::ToolResult {
                tool_use_id: "t1".into(),
                content: "found it".into(),
                is_error: false,
            }],
        });

        // The Ok arm's guard: a tool-result tail is not poppable.
        if convo.messages.last().is_some_and(is_plain_user_text) {
            convo.messages.pop();
        }
        session.record_run(&recorded, &convo).unwrap();

        // Turn 2: the tail is a user message, so the utterance folds — and
        // the fold is recorded at submit as one direct Rewrite, exactly as
        // the push site now runs it.
        assert!(convo.messages.last().is_some_and(|m| m.role == Role::User));
        mecha_core::agent::append_user_text(&mut convo.messages, "actually, stop".into());
        session
            .append(&mecha_core::session::Record::Rewrite {
                messages: convo.messages.clone(),
            })
            .unwrap();

        let (_, resumed) = mecha_core::session::Session::load(&session.path).unwrap();
        assert_eq!(
            resumed.messages, convo.messages,
            "a resume must load exactly what memory holds"
        );
        // Every tool_use still has its result…
        let uses: Vec<_> = resumed
            .messages
            .iter()
            .flat_map(|m| &m.content)
            .filter(|b| matches!(b, Block::ToolUse { .. }))
            .collect();
        let results: Vec<_> = resumed
            .messages
            .iter()
            .flat_map(|m| &m.content)
            .filter(|b| matches!(b, Block::ToolResult { .. }))
            .collect();
        assert_eq!(uses.len(), results.len(), "no orphaned tool_use");
        // …and no two user messages sit adjacent.
        assert!(
            !resumed
                .messages
                .windows(2)
                .any(|w| w[0].role == Role::User && w[1].role == Role::User),
            "the fold, not a push, carries the next utterance"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

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

    /// §6.2's side channel: the one query parameter this module reads.
    #[test]
    fn query_param_reads_the_named_pair_and_nothing_else_matches() {
        assert_eq!(
            query_param("/v1/mecha-affect?session=main", "session"),
            Some("main".to_string())
        );
        assert_eq!(
            query_param("/v1/mecha-affect?other=x&session=main", "session"),
            Some("main".to_string())
        );
        assert_eq!(query_param("/v1/mecha-affect", "session"), None);
        assert_eq!(
            query_param("/v1/mecha-affect?session=", "session"),
            Some(String::new())
        );
        assert_eq!(query_param("/v1/mecha-affect?session=main", "other"), None);
    }

    /// The regression this exists to catch: `hosted_completion`'s
    /// `confirm_key` is `"chat:{id}"`/`"voice:{slot}"`, both containing a
    /// colon, and `worker.py` sends it through httpx's `params=`, which
    /// percent-encodes it. A version of this returning the raw slice passed
    /// every other case here — none of them used a colon — while missing
    /// every real lookup in production.
    #[test]
    fn query_param_decodes_the_percent_encoded_key_the_worker_actually_sends() {
        assert_eq!(
            query_param("/v1/mecha-affect?session=chat%3Amain", "session"),
            Some("chat:main".to_string())
        );
        assert_eq!(
            query_param("/v1/mecha-affect?session=voice%3Awebrtc-1a2b", "session"),
            Some("voice:webrtc-1a2b".to_string())
        );
    }

    #[test]
    fn percent_decode_handles_escapes_plus_and_malformed_input_without_dropping_bytes() {
        assert_eq!(percent_decode("chat%3Amain"), "chat:main");
        assert_eq!(percent_decode("a+b"), "a b");
        assert_eq!(percent_decode("plain"), "plain");
        // A trailing or malformed escape is passed through rather than
        // dropped — absent is not the same as zero, one door over.
        assert_eq!(percent_decode("50%"), "50%");
        assert_eq!(percent_decode("50%2"), "50%2");
        assert_eq!(percent_decode("50%zz"), "50%zz");
    }

    #[test]
    fn the_two_session_headers_do_not_bleed_into_each_other() {
        // They mean different things: one is the facade's own slot, the
        // other names a conversation someone else owns. A parser that
        // conflated them would silently answer in the wrong transcript.
        let raw = b"POST /v1/chat/completions HTTP/1.1\r\nX-Voice-Session: webrtc-1a2b\r\nX-Chat-Session: main\r\nContent-Length: 0\r\n\r\n";
        let head = parse_head(raw).unwrap().expect("complete");
        assert_eq!(head.session.as_deref(), Some("webrtc-1a2b"));
        assert_eq!(head.chat.as_deref(), Some("main"));

        // The old shape still parses, and names no conversation — which is
        // what keeps every pre-D3 caller working unchanged.
        let raw = b"POST /v1/chat/completions HTTP/1.1\r\nX-Voice-Session: webrtc-1a2b\r\nContent-Length: 0\r\n\r\n";
        let head = parse_head(raw).unwrap().expect("complete");
        assert_eq!(head.session.as_deref(), Some("webrtc-1a2b"));
        assert_eq!(head.chat, None);
    }

    #[test]
    fn the_voice_block_opens_a_spoken_stretch_and_nothing_else() {
        // The rule is "the block accompanies a switch into speech": once at
        // the start of a call, again after any typed turn, and never on the
        // second consecutive spoken turn — where it would be pure repetition
        // in a prompt that already carries it.
        let opened = open_spoken_turn("what is on my calendar", false);
        assert!(opened.starts_with(VOICE_BLOCK));
        assert!(opened.ends_with("what is on my calendar"));
        assert_eq!(
            open_spoken_turn("and tomorrow?", true),
            "and tomorrow?",
            "a spoken turn following a spoken turn must not re-send the block"
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
