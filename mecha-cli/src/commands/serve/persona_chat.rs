//! Persona chats over `mecha serve` (`docs/PERSONA-DESIGN.md` §3, §8).
//!
//! **A door of their own.** A persona chat never passes through the
//! assistant's chat routes: it has its own session map, its own routes
//! (`/api/personas…`, `/api/persona-chat/{key}…`), its own agent per persona
//! version, and its transcript under `~/.mecha/personas/<persona>/sessions/`
//! — a directory no reader of the assistant's sessions scans, which is what
//! keeps learning, distillation and appraisal off it (§3.2). The assistant's
//! `ensure_session_as` refuses a persona key, so no assistant route can
//! re-create one in `~/.mecha/sessions/` either.
//!
//! What a persona turn does *not* do, because each reads or writes an owner
//! store: no situation brief (the board), no homeostat sample, no outbox, no
//! workflow, no appraisal readout, no title from the model. The registry, the
//! system prompt and the levers are `persona::agent`'s, built beside the
//! assistant's in `setup::persona_agent`.
//!
//! **The lock hides, never withholds** (§8.3): a locked persona and every
//! chat with it answer 404 without the image library's unlock token, exactly
//! as a missing one does, and every response here is kept out of the
//! browser's cache (`cache_headers`).

use anyhow::{Context, Result};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use mecha_core::agent::{Agent, AgentEvent, Conversation};
use mecha_core::message::{Message, Usage};
use mecha_core::persona::agent::{self as persona_agent, Pinned, Refused};
use mecha_core::persona::files::{readiness, Readiness};
use mecha_core::persona::{judge, safety};
use mecha_core::persona::{Persona, Store};
use mecha_core::session::{Record, RunConfig, Session, SessionMeta};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{broadcast, Mutex};

use super::chat::{self, ChatState, WireEvent};
use super::library::LibraryState;
use crate::setup::PersonaUse;

/// Every persona chat's key starts with this; nothing else's does.
pub const KEY_PREFIX: &str = "p-";

pub fn is_persona_key(key: &str) -> bool {
    key.starts_with(KEY_PREFIX)
}

fn new_key() -> String {
    format!(
        "{KEY_PREFIX}{}",
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    )
}

/// A transcript id as `Session::new_id` mints one — letters, digits and `-`,
/// never a separator or a dot — checked before it becomes a file name.
fn valid_session_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// How a persona chat's providers are made — the router's, in `serve`; a
/// scripted one in tests — and what each is for: the conversation goes
/// unseeded, the crisis judge keeps the seed (`setup::persona_provider`).
type ProviderFactory = Arc<
    dyn Fn(&crate::follow::Bound, PersonaUse) -> Result<Box<dyn mecha_core::provider::Provider>>
        + Send
        + Sync,
>;

/// A built persona agent, the binding it was built on, and what it refused.
struct Built {
    generation: u64,
    agent: Arc<Agent>,
    refused: Vec<Refused>,
}

pub struct PersonaChats {
    /// `~/.mecha/personas`.
    store: PathBuf,
    /// Where each chat's workspace goes: `~/.mecha/work/persona/<key>/`.
    /// Never the persona's own folder, which holds its transcripts (§10.2).
    work: PathBuf,
    sessions: Mutex<HashMap<String, PersonaSession>>,
    /// One agent per persona version, rebuilt when the binding changes so a
    /// persona follows the router's model as the assistant does (§3.4).
    agents: StdMutex<HashMap<(String, String), Built>>,
    provider: ProviderFactory,
    /// Files being read for the first time after an upload (§10.3), so the
    /// page can say "processing" rather than "not yet read".
    /// With when each began — `None` while it waits its turn — so a read
    /// whose task died stops reading as "processing" after a while rather
    /// than for the life of the server, and a long queue is not taken for
    /// dead reads and queued again.
    processing: Arc<StdMutex<HashMap<PathBuf, Option<std::time::Instant>>>>,
    /// One background read at a time: each takes a layout child and a share
    /// of the one on-demand OCR model, so twenty unread papers are a queue,
    /// not twenty extractions at once (review of #459, pass 7).
    reading: Arc<tokio::sync::Semaphore>,
    /// The unlock each call was placed under (§11): the voice worker names
    /// a chat by its key alone, so the token the page offered with is held
    /// here, keyed by chat, and every spoken turn is checked against the
    /// lock with it as a typed turn is with its own (`bind_call`).
    /// Each with the id of the offer that bound it, so an offer that fails
    /// releases its own binding and never a live call's (review of #483).
    calls: StdMutex<HashMap<String, (u64, Option<String>)>>,
    /// The next binding's id.
    next_call: std::sync::atomic::AtomicU64,
}

/// A spoken turn's other end (§11): the voice facade's tap on the run's
/// events, where its answer goes, and the handle that stops it — the three
/// things `voice::HostedTurn` hands the facade.
pub(super) struct Spoken {
    tap: tokio::sync::mpsc::UnboundedSender<AgentEvent>,
    done: tokio::sync::oneshot::Sender<Result<crate::voice::HostedAnswer, String>>,
    cancel: mecha_core::agent::CancelHandle,
    /// The worker's speech engine streams: the call note drops its length
    /// rule (`persona::call::note`).
    streams: bool,
}

/// Which personas speak in each library voice, and why that list may be
/// short: `partial` is `Some("locked")` while the library is locked, and
/// `Some("unreadable")` when a persona file did not load.
#[derive(Debug, Default)]
pub struct VoicesInUse {
    pub by_voice: std::collections::BTreeMap<String, Vec<String>>,
    pub partial: Option<&'static str>,
}

/// A persona's voice as a call binds it: what the offer carries to the
/// worker as `persona_voice`, applied before the call's first word.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CallVoice {
    pub voice: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
}

/// How long a spoken turn waits for a run in flight to yield, in 100 ms
/// tries — the assistant's window (`chat::BARGE_IN_TRIES`), for the same
/// reason: cancellation never interrupts a tool call mid-call.
const BARGE_IN_TRIES: usize = 200;

struct PersonaSession {
    pinned: Arc<Pinned>,
    session: Arc<Session>,
    /// The router binding the transcript last recorded a `Config` for. A
    /// persona chat follows the router turn by turn, so a chat can run on two
    /// models; the header names only the first, and the memory writer writes
    /// a chat with the model it ran on (`writer::read_chat`). `None` until
    /// this process records one: once per attach, as every door does, and
    /// again at each switch.
    recorded_generation: Option<u64>,
    /// `None` while a run holds it.
    conversation: Option<Conversation>,
    workspace: PathBuf,
    live: Option<Live>,
    events: broadcast::Sender<WireEvent>,
    last_usage: Arc<StdMutex<Option<Usage>>>,
    /// The session goal (§6), folded into the first turn and then spent.
    goal: Option<String>,
    /// Owner turns since the Core was last handed back (§12.5).
    turns_since_anchor: u32,
    /// The next turn re-anchors whatever the count: after a compaction, and
    /// on the first turn of a resumed chat.
    anchor_due: bool,
    /// When the crisis sensor last paused this chat: a hit inside
    /// `[personas] crisis_cooldown_minutes` of it (default
    /// `safety::CRISIS_COOLDOWN`) does not pause again (§12.2).
    crisis_paused_at: Option<std::time::Instant>,
    /// An owner message that tripped the crisis sensor while a run was live:
    /// the run is stopped, and its hand-back records this so the words are
    /// kept although the persona never answers them.
    pending_crisis: Option<String>,
    /// Whether the crisis judge answered the last time it was asked: `None`
    /// before it has been asked, which reads as keywords only — "on" is
    /// claimed only once it has answered (review of #426).
    judge_answered: Option<bool>,
    /// Whether this chat has shown a crisis card: the page offers the
    /// support resources only after one (owner ruling, 2026-09-30). Held for
    /// the chat's life in this process — a restart forgets it, since the
    /// safety record is content-free and names no chat by design.
    crisis_shown: bool,
    /// What the persona remembers at chat start (§9.7), as read for this
    /// chat in this process: `None` until read; `Some(None)` when there was
    /// nothing to store (it remembers nothing, its memory is off, or the
    /// store was unreadable, which the turn that read it said on the page).
    /// Kept so a chat with nothing to remember is not re-read, and re-noticed,
    /// on every turn. What is read is stored once in the chat's first turn
    /// (`recall::carries_chat_start`); only a chat already past a reply that
    /// never stored it gets it as a note.
    memory: Option<Option<String>>,
    /// The reply being read aloud, while a Listen tap speaks it
    /// (`listen::Reply`): here so it goes when this chat does.
    listen: Option<Arc<super::listen::Reply>>,
}

struct Live {
    cancel: mecha_core::agent::CancelHandle,
    queue: Arc<StdMutex<VecDeque<String>>>,
    queued_ids: Arc<StdMutex<VecDeque<String>>>,
    history: Arc<[Message]>,
    /// The taint of `history`, beside it: a transcript read mid-run without
    /// its chip reads as clean, and with the leak guard lifted (D11) the chip
    /// is what is left to say it (found on review of #409; the assistant's
    /// `Live` carries it for the same reason).
    taint: mecha_core::agent::Taint,
    /// The tool the run is waiting on — id, name and when it began — so a
    /// page reloaded during a long image render still says "drawing a
    /// picture… 1:24" rather than "typing" (review of #431). Set and cleared
    /// by the run's event forwarder ([`track_working`]).
    working: Arc<StdMutex<Vec<serde_json::Value>>>,
}

/// Keep `slot` naming the tool a run is waiting on: every call that has
/// started and not yet returned — tools in one turn run concurrently — and
/// the transcript reports the latest of them.
fn track_working(slot: &StdMutex<Vec<serde_json::Value>>, event: &AgentEvent) {
    let Ok(mut slot) = slot.lock() else { return };
    match event {
        AgentEvent::ToolCall { id, name, .. } => slot.push(serde_json::json!({
            "id": id,
            "name": name,
            "since": chrono::Utc::now().to_rfc3339(),
        })),
        AgentEvent::ToolResult { id, .. } => slot.retain(|w| w["id"] != *id),
        // A refused call gets no result — the interlock, a hook or the
        // approver said no — and carries no id: the latest call of that name
        // is the one refused (review of #431: it stayed "working" all run).
        AgentEvent::ToolDenied { name, .. } => {
            if let Some(at) = slot.iter().rposition(|w| w["name"] == *name) {
                slot.remove(at);
            }
        }
        _ => {}
    }
}

/// Beside each transcript: which persona version the chat is pinned to, so
/// a resume renders the persona the chat began with (§8.1). A file of its
/// own rather than a session record, because a `Record` variant is a wire
/// format every reader of every session would have to learn.
#[derive(serde::Serialize, serde::Deserialize)]
struct PinRecord {
    persona: String,
    version: u32,
    digest: String,
    #[serde(default)]
    goal: Option<String>,
}

fn pin_path(sessions: &Path, id: &str) -> PathBuf {
    sessions.join(format!("{id}.persona.json"))
}

/// A crisis judge ready to ask — its provider, model and the owner's words —
/// or why it could not be made.
type JudgeJob = Result<(Box<dyn mecha_core::provider::Provider>, String, String), String>;

/// How long a verdict still out when the persona finished may take before it
/// is "couldn't check".
const JUDGE_WAIT: std::time::Duration = std::time::Duration::from_secs(90);

/// The most text a session goal may carry.
const MAX_GOAL: usize = 2000;

/// How many tokens a persona may reason for on a spoken turn (owner,
/// 2026-10-03: "start with call turns first"). A typed turn keeps the
/// server's `--reasoning-budget` (4096).
///
/// A persona's thinking is its reply being drafted: a short block plans,
/// and a long one re-drafts the same lines until the server's budget runs
/// out — up to 23 copies of one sentence, 40–50 s of silence on a call.
/// Replayed on nine of a chat's turns, judged blind by the same model with
/// the order swapped: a 512-token cap lost to the full budget 10–20 and
/// ended replies mid-sentence three times as often (it stops the draft
/// half-written); a 1024-token cap beat the full budget 24–10, with the
/// slowest reply 12.5 s against 41.3 s.
const SPOKEN_THINK_BUDGET: u32 = 1024;

/// Why a door refused, as the route will say it.
#[derive(Debug)]
pub enum Refusal {
    /// Missing, or locked without a live unlock token — the same answer.
    NotFound,
    /// The persona exists but cannot open a chat as it stands.
    Conflict(String),
    Bad(String),
    Failed(String),
    /// A spoken turn found a run in flight, or one still landing: the
    /// caller barges in and tries again (`speak`).
    Busy,
}

impl Refusal {
    /// What a voice call is told: the route's words, without the status.
    fn said(&self) -> String {
        match self {
            Refusal::NotFound => "no such persona chat".into(),
            Refusal::Busy => "a turn is still running".into(),
            Refusal::Conflict(m) | Refusal::Bad(m) | Refusal::Failed(m) => m.clone(),
        }
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> axum::response::Response {
        match self {
            Refusal::NotFound => (StatusCode::NOT_FOUND, "no such persona chat\n").into_response(),
            Refusal::Conflict(m) => (StatusCode::CONFLICT, format!("{m}\n")).into_response(),
            Refusal::Bad(m) => (StatusCode::BAD_REQUEST, format!("{m}\n")).into_response(),
            Refusal::Failed(m) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{m}\n")).into_response()
            }
            Refusal::Busy => (StatusCode::CONFLICT, "a turn is still running\n").into_response(),
        }
    }
}

fn failed(e: anyhow::Error) -> Refusal {
    Refusal::Failed(format!("{e:#}"))
}

/// An answer for the call that no model wrote.
fn spoken_answer(text: &str) -> crate::voice::HostedAnswer {
    crate::voice::HostedAnswer {
        text: text.to_string(),
        input_tokens: 0,
        output_tokens: 0,
        affect: None,
    }
}

/// What the safety layer can do for a persona, as every surface shows it
/// (§12): each switch, with the crisis sensor said as it is — `on` only once
/// its judge has answered, `degraded` (keywords only) before then and while
/// it cannot, so it can never pass for "checked".
/// `judge` is a chat's judge state (`Some(answered)`), or `None` for the
/// persona itself, where no judge has been asked: that reads `enabled` —
/// what is configured — never `on`, which a chat earns (review of #426).
fn safety_json(s: &mecha_core::persona::Safety, judge: Option<bool>) -> serde_json::Value {
    let crisis = match judge {
        Some(answered) => serde_json::json!(safety::crisis_state(s.crisis, answered)),
        None if s.crisis => serde_json::json!("enabled"),
        None => serde_json::json!("off"),
    };
    serde_json::json!({
        "disclosure": s.disclosure,
        "crisis": crisis,
        "reanchor": s.reanchor,
        "dose": s.dose,
        // The plain voice's words, so the page can keep them one tap away
        // after a reload has dropped the card that carried them.
        "resources": s.crisis.then_some(safety::SAFE_MESSAGE),
    })
}

fn refused_json(refused: &[Refused]) -> serde_json::Value {
    serde_json::Value::Array(
        refused
            .iter()
            .map(|r| serde_json::json!({ "tool": r.tool, "why": r.why }))
            .collect(),
    )
}

/// The document reader as `setup::document_extractor` would build it: its
/// cap (`0` while document reading is switched off, so nothing is "ready"
/// that no chat could read) and whether `[documents] cache` keeps its reads
/// — read off the config, never by building an extractor on every poll.
/// A readiness that consulted a cache the reader never writes is how every
/// document read "not read yet" forever with the cache off (review of
/// #459, pass 7).
fn reader_shape(config: &mecha_core::config::Config) -> (u64, bool) {
    config
        .documents
        .as_ref()
        .filter(|_| {
            mecha_core::feature::switched_on(config, mecha_core::feature::Feature::Documents)
        })
        // A `[documents]` that will not build a reader is no reader: refused,
        // not promised (review of #459, pass 8). `files_block` builds the
        // real one and asks it; this is the poll's cheap check.
        .filter(|d| d.validate().is_ok())
        .map(|d| (d.max_file_bytes(), d.cache))
        .unwrap_or((0, false))
}

/// The chat's citations, checked (§10.4) against everything it ever
/// received — the session's `messages_ever`, so a page a compaction has
/// since evicted still counts for the answer that quoted it. Off the
/// runtime: the file is read whole. Unreadable, it is no citations at all
/// rather than a guess from a copy of the live messages — untagged says
/// "nothing was checked", never "this was checked" (review of #465: the
/// copy was the whole conversation, cloned under the lock, for a fallback
/// that almost never ran).
async fn citations(session: &Session) -> Vec<mecha_core::persona::cite::Checked> {
    let path = session.path.clone();
    tokio::task::spawn_blocking(move || {
        std::fs::read_to_string(&path)
            .map(|t| mecha_core::persona::cite::check_conversation(&Session::messages_ever(&t)))
            .unwrap_or_default()
    })
    .await
    .unwrap_or_default()
}

/// How long a turn waits for the embeddings server to turn the owner's
/// message into a vector for recall: past its ~4 s cold start after an idle
/// stop (LLAMA-SERVER.md §Document OCR), short of a wait anyone notices on a
/// warm server. Beyond it, recall is by words.
const RECALL_EMBED_WAIT: std::time::Duration = std::time::Duration::from_secs(8);

/// How the owner opened a chat, for the earlier-chats list: their first
/// message's own words — past the goal the harness put ahead of it, never a
/// harness block — on one line, cut at a word. Read from the top of the
/// transcript and no further; `None` when there is none yet.
fn opener(path: &Path) -> Option<String> {
    use std::io::BufRead;
    let file = std::fs::File::open(path).ok()?;
    for line in std::io::BufReader::new(file).lines().take(200) {
        let line = line.ok()?;
        // The owner's words as every other reader takes them
        // (`agent::owner_text`): nothing from a harness-made message, and
        // every block that is not the harness's voice (review of #479).
        let Ok(Record::Message(message)) = serde_json::from_str::<Record>(&line) else {
            continue;
        };
        if message.role != mecha_core::message::Role::User {
            continue;
        }
        let owned = mecha_core::agent::owner_text(&message);
        if owned.trim().is_empty() {
            // Not the owner's words: the next user record may be.
            continue;
        }
        let text = owned.as_str();
        let text = match text.strip_prefix("(What I want from this conversation: ") {
            Some(rest) => rest.split_once(")\n\n").map(|(_, t)| t).unwrap_or(rest),
            None => text,
        };
        let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if one.is_empty() {
            return None;
        }
        if one.chars().count() <= 90 {
            return Some(one);
        }
        let cut: String = one.chars().take(90).collect();
        let cut = cut.rsplit_once(' ').map(|(a, _)| a).unwrap_or(&cut);
        return Some(format!("{cut}…"));
    }
    None
}

/// Whether a turn should carry the persona's files: only before the chat's
/// first reply, so they land in `messages[0]` — the one message compaction
/// keeps whole. Any later, they would sit mid-conversation, a compaction
/// would summarise them away, and the next turn would fold the whole
/// collection back in just after context ran short (review of #459, pass
/// 8). A file added while a chat is open reaches the next chat; this one
/// can still read it with `file_read`, which lists the folders live. The
/// owner's turns only, as `Taint::arm_for_content` reads the stem: the two
/// must agree on what "carried" means.
fn carries_files_now(messages: &[Message]) -> bool {
    !messages
        .iter()
        .any(|m| m.role == mecha_core::message::Role::Assistant)
        && !mecha_core::persona::files::carries(messages)
}

/// Whether a file goes to the background queue: still being read, or ready
/// and not yet indexed for `file_search`. Not one read on request — the
/// owner turned the extraction cache off, and a document is then read when
/// a chat asks for it, never ahead (so with the cache off, only text files
/// are searchable) — nor one that will be refused.
///
/// `indexed` is asked only of a ready file: hashing one that is refused or
/// over the cap is the unbounded read `readiness` exists not to do (review
/// of #467).
fn to_process(state: &Readiness, indexed: impl FnOnce() -> bool) -> bool {
    match state {
        Readiness::Reading => true,
        Readiness::Ready => !indexed(),
        Readiness::OnRequest | Readiness::Refused(_) => false,
    }
}

/// Whether the search index holds `src` as fully as it can: with an
/// embeddings server configured, every passage embedded — a file indexed
/// while `:8081` was down, or whose vectors a new model dropped, is taken
/// round again until it is (review of #467: `has` alone left it words-only
/// for good). Without one, its passages in. Unknown reads as not.
fn indexed(
    index: Option<&mecha_core::persona::search::Index>,
    src: &mecha_core::persona::files::Source,
    embedded: bool,
) -> bool {
    index.is_some_and(|i| {
        mecha_core::persona::files::sha_of(src).is_some_and(|sha| {
            if embedded {
                i.complete(&sha).unwrap_or(false)
            } else {
                i.has(&sha).unwrap_or(false)
            }
        })
    })
}

/// The extraction cache, where the reader keeps one.
fn reader_cache(cached: bool) -> Option<mecha_core::document::Cache> {
    cached
        .then(mecha_core::document::Cache::default_dir)
        .and_then(Result::ok)
        .map(mecha_core::document::Cache::new)
}

impl PersonaChats {
    pub fn new() -> Result<Self> {
        Ok(Self::with(
            Store::default_dir()?,
            mecha_core::work::producer_dir("persona")?,
            Arc::new(crate::setup::persona_provider),
        ))
    }

    fn with(store: PathBuf, work: PathBuf, provider: ProviderFactory) -> Self {
        PersonaChats {
            store,
            work,
            sessions: Mutex::new(HashMap::new()),
            agents: StdMutex::new(HashMap::new()),
            provider,
            processing: Arc::default(),
            reading: Arc::new(tokio::sync::Semaphore::new(1)),
            calls: StdMutex::new(HashMap::new()),
            next_call: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// For the assistant's chat tests, which build a `ChatState` but never
    /// open a persona chat: scratch directories, and a provider that must
    /// never be asked for.
    #[cfg(test)]
    pub fn for_tests() -> Self {
        let root =
            std::env::temp_dir().join(format!("mecha-persona-chat-{}", uuid::Uuid::new_v4()));
        Self::with(
            root.join("personas"),
            root.join("work"),
            Arc::new(|_, _| anyhow::bail!("this test opens no persona chat")),
        )
    }

    /// `name` as a browsing surface may see it: `None` when missing, and
    /// when locked without a live unlock token — the same answer, so a
    /// locked persona is indistinguishable from none (§8.3).
    fn visible(&self, library: &LibraryState, name: &str, token: Option<&str>) -> Option<Persona> {
        let store = Store::load(&self.store);
        let p = store.get(name)?.clone();
        (!p.state.locked || library.unlocked(token)).then_some(p)
    }

    /// The agent for this version on this binding, built once and rebuilt
    /// when the binding moves. Agents of an older binding are dropped from
    /// the cache; a run still holding one keeps it alive to its end.
    fn agent_for(
        &self,
        bound: &crate::follow::Bound,
        pinned: &Pinned,
    ) -> Result<(Arc<Agent>, Vec<Refused>)> {
        let key = (pinned.name.clone(), pinned.digest.clone());
        let mut agents = self.agents.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(b) = agents
            .get(&key)
            .filter(|b| b.generation == bound.generation)
        {
            return Ok((Arc::clone(&b.agent), b.refused.clone()));
        }
        let (agent, refused) = crate::setup::persona_agent(
            bound,
            pinned,
            (self.provider)(bound, PersonaUse::Converse)?,
            &self.store,
        )?;
        let agent = Arc::new(agent);
        agents.retain(|_, b| b.generation == bound.generation);
        agents.insert(
            key,
            Built {
                generation: bound.generation,
                agent: Arc::clone(&agent),
                refused: refused.clone(),
            },
        );
        Ok((agent, refused))
    }

    /// Cancel every persona run in flight, at shutdown. `ChatState::stop`
    /// calls this after `stopping` is set and before it closes the tracker
    /// these runs share, so `drain` waits only for runs that were asked to
    /// stop — never for a persona turn nothing could see (found on review of
    /// #409). A `send` either set its run live before this took the lock, and
    /// is cancelled here, or checks `stopping` under the lock and refuses.
    pub async fn stop(&self) {
        let sessions = self.sessions.lock().await;
        for ps in sessions.values() {
            if let Some(live) = &ps.live {
                live.cancel
                    .cancel(mecha_core::agent::CancelReason::Shutdown);
            }
        }
    }

    // ─── Authoring: the owner's door on the page (§4.4) ───────────────────
    //
    // Owner-only routes behind the same guard and CSRF check as every other
    // mutation on the page; no tool reaches them, so no model can author a
    // line. What they write is the owner's text, verbatim.

    /// What a new persona can be made from: relationship templates (the
    /// starters offered once, as `mecha persona` does), declared groups, and
    /// the approved characters the page may show.
    pub fn authoring(
        &self,
        library: &LibraryState,
        token: Option<&str>,
    ) -> Result<serde_json::Value, Refusal> {
        // Read-only, as a GET is (the guard exempts GETs from the CSRF header
        // on that premise): the starters are listed, and copied in by
        // `create` when one is first used (review of #420).
        let store = Store::load(&self.store);
        let lib = mecha_core::imagelib::Library::load(&library.dir).0;
        let unlocked = library.unlocked(token);
        let relationships: Vec<serde_json::Value> =
            mecha_core::persona::relationship_choices(&self.store)
                .into_iter()
                .map(|(name, starter)| serde_json::json!({ "name": name, "starter": starter }))
                .collect();
        let characters = visible_characters(&lib, unlocked);
        Ok(serde_json::json!({
            "relationships": relationships,
            "groups": store.groups().keys().collect::<Vec<_>>(),
            "characters": characters,
        }))
    }

    /// The persona's portrait character when it is a locked library entry and
    /// this viewer holds no unlock: then no part of the Edit screen names it
    /// (owner ruling, 2026-09-30 — hidden, never cut). `None` when there is
    /// nothing to hide.
    fn hidden_character(
        &self,
        library: &LibraryState,
        p: &Persona,
        token: Option<&str>,
    ) -> Option<String> {
        // One rule for the list and the Edit screen: the free fn below, which
        // fails closed on an entry that does not load (#425, #430).
        let lib = mecha_core::imagelib::Library::load(&library.dir).0;
        hidden_character(p, &lib, library.unlocked(token)).map(str::to_string)
    }

    /// What the settings form offers, from the stores as they stand — built
    /// the same way for the page's GET and for checking its save, so a save
    /// is held to the choices the page was shown.
    fn form(
        &self,
        library: &LibraryState,
        token: Option<&str>,
        voices: &mecha_core::persona::VoiceChoices,
    ) -> mecha_core::tomlform::Form {
        let store = Store::load(&self.store);
        let lib = mecha_core::imagelib::Library::load(&library.dir).0;
        mecha_core::persona::settings_form(&mecha_core::persona::FormChoices {
            relationships: mecha_core::persona::relationship_choices(&self.store)
                .into_iter()
                .map(|(name, _)| name)
                .collect(),
            groups: store.groups().keys().cloned().collect(),
            characters: visible_characters(&lib, library.unlocked(token))
                .into_iter()
                .map(str::to_string)
                .collect(),
            voices: voices.clone(),
        })
    }

    /// Make a persona — the owner's, so approved at once, exactly as
    /// `mecha persona new` makes one.
    pub fn create(
        &self,
        library: &LibraryState,
        body: CreateBody,
    ) -> Result<serde_json::Value, Refusal> {
        let lib = mecha_core::imagelib::Library::load(&library.dir).0;
        let p = mecha_core::persona::create(
            &self.store,
            &lib,
            mecha_core::persona::NewPersona {
                name: body.name.trim().to_lowercase(),
                display: body.display.unwrap_or_default(),
                relationships: body.relationships,
                character: body.character.filter(|c| !c.trim().is_empty()),
                voice: None,
                groups: body.groups,
                locked: body.locked,
                origin: mecha_core::persona::Origin::Owner,
            },
        )
        .map_err(|e| Refusal::Bad(format!("{e:#}")))?;
        Ok(serde_json::json!({ "name": p.name, "version": p.state.version }))
    }

    /// [`PersonaChats::files_in`] with no voice list, which is what the tests
    /// that are not about voices need.
    #[cfg(test)]
    pub fn files(
        &self,
        library: &LibraryState,
        name: &str,
        token: Option<&str>,
    ) -> Result<serde_json::Value, Refusal> {
        self.files_in(library, name, token, &Default::default())
    }

    /// A persona's owner files as they stand, each with the digest a save
    /// hands back so an edit made elsewhere meanwhile is refused, not lost.
    /// `voices` is what the settings form offers for `voice` ([`form_voices`]).
    pub fn files_in(
        &self,
        library: &LibraryState,
        name: &str,
        token: Option<&str>,
        voices: &mecha_core::persona::VoiceChoices,
    ) -> Result<serde_json::Value, Refusal> {
        let p = self
            .visible(library, name, token)
            .ok_or(Refusal::NotFound)?;
        let mut out = serde_json::Map::new();
        for file in [
            mecha_core::persona::OwnerFile::Identity,
            mecha_core::persona::OwnerFile::Motivation,
            mecha_core::persona::OwnerFile::Settings,
        ] {
            let mut text =
                mecha_core::persona::read_owner_file(&self.store, &p.name, file).map_err(failed)?;
            // A hidden portrait is not named: the line is left out of what
            // is served, and the digest is over the served text — a digest
            // of the file itself would let a guessed name be checked.
            if file == mecha_core::persona::OwnerFile::Settings
                && self.hidden_character(library, &p, token).is_some()
            {
                text = mecha_core::persona::hide_character(&text);
            }
            // The settings file also comes as a form. One the form cannot
            // read (a hand edit that does not load) comes without it, and
            // why: the page edits it as text until it loads again.
            let form = if file == mecha_core::persona::OwnerFile::Settings {
                let form = self.form(library, token, voices);
                match mecha_core::persona::settings_values(&form, &text) {
                    Ok(values) => serde_json::json!({ "form": form, "values": values }),
                    Err(e) => serde_json::json!({ "problem": format!("{e:#}") }),
                }
            } else {
                // Markdown: the file split into its parts (`mdform`), and the
                // sections it must keep.
                serde_json::json!({
                    "doc": mecha_core::mdform::split(&text),
                    "fixed": mecha_core::persona::fixed_sections(file),
                })
            };
            out.insert(
                serde_json::to_value(file)
                    .map_err(|e| failed(e.into()))?
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                serde_json::json!({
                    "file": file.file_name(),
                    "digest": mecha_core::persona::text_digest(&text),
                    "text": text,
                    "form": form,
                }),
            );
        }
        Ok(serde_json::Value::Object(out))
    }

    /// [`PersonaChats::save_in`] with no voice list, which is what the tests
    /// that are not about voices need.
    #[cfg(test)]
    pub fn save(
        &self,
        library: &LibraryState,
        name: &str,
        body: SaveBody,
    ) -> Result<serde_json::Value, Refusal> {
        self.save_in(library, name, body, &Default::default())
    }

    /// Save one owner file, verbatim, and take a version (`write_owner_file`).
    /// A form save that picks a voice is held to the voices listed now, not
    /// when the page loaded: a voice the worker has since lost is refused
    /// here rather than at the next call — and a list that could not be read
    /// refuses with its reason, never as "not one of the choices", which
    /// would say the worker lacks a voice nobody could ask it about.
    pub fn save_in(
        &self,
        library: &LibraryState,
        name: &str,
        body: SaveBody,
        voices: &mecha_core::persona::VoiceChoices,
    ) -> Result<serde_json::Value, Refusal> {
        let p = self
            .visible(library, name, body.unlock.as_deref())
            .ok_or(Refusal::NotFound)?;
        // Required from the page either way: a save that could skip the stale
        // check by leaving out a field would make it advisory (review of #420).
        // The page was shown the settings without a hidden portrait's line,
        // and its base is a digest of that. Check the base against what it
        // was shown, then hold the write to the file as it is now; a text
        // save gets the link back, and a form save never touched it.
        let hidden = (body.file == mecha_core::persona::OwnerFile::Settings)
            .then(|| self.hidden_character(library, &p, body.unlock.as_deref()))
            .flatten();
        let mut base = body.base.clone();
        let mut disk_text = None;
        if hidden.is_some() {
            let disk = mecha_core::persona::read_owner_file(&self.store, &p.name, body.file)
                .map_err(failed)?;
            let shown =
                mecha_core::persona::text_digest(&mecha_core::persona::hide_character(&disk));
            if shown != body.base {
                return Err(Refusal::Conflict(format!(
                    "{:#}",
                    anyhow::Error::from(mecha_core::persona::StaleEdit(body.file))
                )));
            }
            base = mecha_core::persona::text_digest(&disk);
            disk_text = Some(disk);
        }
        let saved = match (body.text, body.changes, body.doc) {
            (None, None, Some(doc)) if body.file != mecha_core::persona::OwnerFile::Settings => {
                mecha_core::persona::edit_markdown(&self.store, &p.name, body.file, &doc, &base)
            }
            (None, None, Some(_)) => {
                return Err(Refusal::Bad(
                    "persona.toml is not edited as Markdown".into(),
                ))
            }
            (Some(text), None, None) => {
                let text = match &disk_text {
                    Some(disk) => mecha_core::persona::restore_character(&text, disk),
                    None => text,
                };
                mecha_core::persona::write_owner_file(
                    &self.store,
                    &p.name,
                    body.file,
                    &text,
                    Some(&base),
                )
            }
            (None, Some(mut changes), None)
                if body.file == mecha_core::persona::OwnerFile::Settings =>
            {
                // Hidden, never cut: the page was shown no portrait, so a
                // `null` for it is not a choice to remove one. A name is —
                // the owner picked a portrait.
                if hidden.is_some() && changes.get("character").is_some_and(|v| v.is_null()) {
                    changes.remove("character");
                }
                if let Some(why) = voices.unread.as_deref().filter(|_| picks_voice(&changes)) {
                    return Err(Refusal::Bad(format!(
                        "a voice cannot be picked now — {why}; nothing was saved"
                    )));
                }
                let form = self.form(library, body.unlock.as_deref(), voices);
                mecha_core::persona::edit_settings(&self.store, &p.name, &form, &changes, &base)
            }
            (None, Some(_), None) => {
                return Err(Refusal::Bad("only persona.toml is edited as a form".into()))
            }
            _ => {
                return Err(Refusal::Bad(
                    "a save carries one of `text`, `changes` or `doc`".into(),
                ))
            }
        };
        let state = saved.map_err(|e| {
            if e.downcast_ref::<mecha_core::persona::StaleEdit>().is_some() {
                return Refusal::Conflict(format!("{e:#}"));
            }
            let why = format!("{e:#}");
            // A parser's refusal quotes the offending source line, and the
            // restored hidden line is part of the source: never let a
            // refusal say the name the page was not shown (review of #430).
            match &hidden {
                Some(c) if why.contains(c.as_str()) => Refusal::Bad(
                    "persona.toml would not load as saved; unlock the library to see why".into(),
                ),
                _ => Refusal::Bad(why),
            }
        })?;
        let store = Store::load(&self.store);
        let lib = mecha_core::imagelib::Library::load(&library.dir).0;
        let unlocked = library.unlocked(body.unlock.as_deref());
        let problems = store
            .get(&p.name)
            .map(|q| unnamed(store.problems(q, &lib), hidden_character(q, &lib, unlocked)))
            .unwrap_or_default();
        Ok(serde_json::json!({ "version": state.version, "problems": problems }))
    }

    /// Add a relationship template of the owner's own (§5).
    pub fn add_relationship(&self, body: RelationshipBody) -> Result<serde_json::Value, Refusal> {
        let name = body.name.trim().to_lowercase();
        mecha_core::persona::add_relationship(&self.store, &name, &body.text)
            .map_err(|e| Refusal::Bad(format!("{e:#}")))?;
        Ok(serde_json::json!({ "name": name }))
    }

    /// Declare a group (§4.5): appended to groups.toml with its folder made,
    /// as `mecha persona group add` does.
    pub fn add_group(&self, body: GroupBody) -> Result<serde_json::Value, Refusal> {
        let name = body.name.trim().to_lowercase();
        mecha_core::persona::add_group(
            &self.store,
            &name,
            body.description.as_deref().unwrap_or(""),
        )
        .map_err(|e| Refusal::Bad(format!("{e:#}")))?;
        Ok(serde_json::json!({ "name": name }))
    }

    /// Hide or show a persona while browsing. Showing a locked one needs the
    /// live unlock, as reading it does.
    pub fn lock(
        &self,
        library: &LibraryState,
        name: &str,
        locked: bool,
        token: Option<&str>,
    ) -> Result<serde_json::Value, Refusal> {
        let p = self
            .visible(library, name, token)
            .ok_or(Refusal::NotFound)?;
        let state =
            mecha_core::persona::set_locked(&self.store, &p.name, locked).map_err(failed)?;
        Ok(serde_json::json!({ "locked": state.locked }))
    }

    /// A waiting persona as the owner reads it before approving (§4.4; the
    /// owner's rulings of 2026-10-01): every field a proposal can set, and
    /// this server's signature of the files' digest — what approval sends
    /// back, so what is approved is what was shown. A linked character still
    /// waiting comes with its own description, portrait and signature, so
    /// one tap can approve both. The lock holds as for every read.
    pub fn review(
        &self,
        library: &LibraryState,
        name: &str,
        token: Option<&str>,
    ) -> Result<serde_json::Value, Refusal> {
        use mecha_core::imagelib::{self, Kind, Library, Status};
        let p = self
            .visible(library, name, token)
            .ok_or(Refusal::NotFound)?;
        waiting_proposal(&p)?;
        let store = Store::load(&self.store);
        let digest = store.content_digest(&p).map_err(failed)?;
        let lib = Library::load(&library.dir).0;
        let unlocked = library.unlocked(token);
        // A locked character is not named to a locked page (#425's rule),
        // and so is neither shown nor offered for approval here.
        let character = match hidden_character(&p, &lib, unlocked) {
            Some(_) => serde_json::Value::Null,
            None => p
                .settings
                .character
                .as_deref()
                .and_then(|c| lib.get(Kind::Character, c))
                .map(|e| {
                    let waiting = e.status == Status::Candidate;
                    serde_json::json!({
                        "name": e.name,
                        "text": e.text,
                        "portrait": super::library::portrait_url(e, token.filter(|_| unlocked)),
                        "waiting": waiting,
                        "origin": e.origin,
                        "shown": waiting.then(|| library.sign(&imagelib::shown_digest(e))),
                    })
                })
                .unwrap_or(serde_json::Value::Null),
        };
        Ok(serde_json::json!({
            "name": p.name,
            "display": p.display(),
            "relationships": p.settings.relationship.0,
            "voice": p.settings.voice,
            // The tools its relationship templates grant — the owner's own
            // templates' suggestions, never the model's — shown so approval
            // is not of a list nobody saw (review of #493).
            "tools": p.settings.tools.allow,
            "identity": p.identity,
            "motivation": p.motivation,
            "origin": p.state.origin,
            "locked": p.state.locked,
            "character": character,
            "shown": library.sign(&digest),
        }))
    }

    /// Approve a waiting persona as it was shown, and, with
    /// `character_shown`, the waiting character it links. Both signatures
    /// are checked before anything is written; the character goes first, so
    /// an approved persona never points at a character it was shown still
    /// waiting. (A locked character hidden from this page is not shown, so
    /// not offered: #425's rule. The persona can then be approved pointing
    /// at it, still unapproved for generation.) The
    /// persona's digest is checked once more at its own write, so a
    /// revision landing in between refuses the persona and leaves the
    /// character approved — what the owner had read of it, still.
    pub fn approve(
        &self,
        library: &LibraryState,
        name: &str,
        shown: &str,
        character_shown: Option<&str>,
        token: Option<&str>,
    ) -> Result<serde_json::Value, Refusal> {
        use mecha_core::imagelib::{self, Kind, Library, Status};
        let p = self
            .visible(library, name, token)
            .ok_or(Refusal::NotFound)?;
        waiting_proposal(&p)?;
        let changed = || {
            Refusal::Conflict(format!(
                "`{}` is not what was shown — read it again before approving",
                p.name
            ))
        };
        let digest = Store::load(&self.store)
            .content_digest(&p)
            .map_err(failed)?;
        if !library.signed(&digest, shown) {
            return Err(changed());
        }
        let lib = Library::load(&library.dir).0;
        let unlocked = library.unlocked(token);
        // A linked character still waiting, and shown to this page, is
        // approved with the persona or not at all: the server keeps the
        // rule, not only the page (review of #493).
        let waiting_character = match hidden_character(&p, &lib, unlocked) {
            Some(_) => None,
            None => p
                .settings
                .character
                .as_deref()
                .and_then(|c| lib.get(Kind::Character, c))
                .filter(|e| e.status == Status::Candidate),
        };
        if waiting_character.is_some() && character_shown.is_none() {
            return Err(Refusal::Conflict(
                "its portrait is waiting too — approve both together".into(),
            ));
        }
        let character = match character_shown {
            None => None,
            Some(sig) => {
                // A character hidden from this page is not offered here.
                let shown_character = match hidden_character(&p, &lib, unlocked) {
                    Some(_) => None,
                    None => p.settings.character.as_deref(),
                };
                let entry = shown_character
                    .and_then(|c| lib.get(Kind::Character, c))
                    .filter(|e| e.status == Status::Candidate)
                    .ok_or_else(|| {
                        Refusal::Conflict("its character is not waiting for approval".into())
                    })?;
                let entry_digest = imagelib::shown_digest(entry);
                if !library.signed(&entry_digest, sig) {
                    return Err(Refusal::Conflict(format!(
                        "`{}` is not what was shown — read it again before approving",
                        entry.name
                    )));
                }
                Some((entry.name.clone(), entry_digest))
            }
        };
        if let Some((c, d)) = &character {
            imagelib::approve_as_shown(&library.dir, Kind::Character, c, d)
                .map_err(|e| Refusal::Conflict(format!("{e:#}")))?;
        }
        let state = mecha_core::persona::approve_as_shown(&self.store, &p.name, &digest)
            .map_err(|e| Refusal::Conflict(format!("{e:#}")))?;
        Ok(serde_json::json!({
            "approved": p.name,
            "version": state.version,
            "character": character.map(|(c, _)| c),
        }))
    }

    /// Turn a waiting persona away: moved aside under `removed/`, as
    /// `mecha persona remove` does. Only a candidate — an approved persona
    /// is removed from the terminal, deliberately.
    pub fn reject(
        &self,
        library: &LibraryState,
        name: &str,
        token: Option<&str>,
    ) -> Result<serde_json::Value, Refusal> {
        let p = self
            .visible(library, name, token)
            .ok_or(Refusal::NotFound)?;
        waiting_proposal(&p)?;
        mecha_core::persona::remove(&self.store, &p.name).map_err(failed)?;
        Ok(serde_json::json!({ "rejected": p.name }))
    }

    /// Place the portrait in a persona's avatar, or return it to the
    /// default. The lock
    /// holds as for every write: a hidden persona answers as a missing one.
    pub fn frame(
        &self,
        library: &LibraryState,
        name: &str,
        frame: Option<mecha_core::persona::Frame>,
        token: Option<&str>,
    ) -> Result<serde_json::Value, Refusal> {
        let p = self
            .visible(library, name, token)
            .ok_or(Refusal::NotFound)?;
        if let Some(f) = frame {
            f.checked().map_err(|e| Refusal::Bad(format!("{e:#}")))?;
        }
        let state = mecha_core::persona::set_frame(&self.store, &p.name, frame).map_err(failed)?;
        Ok(serde_json::json!({ "frame": state.frame }))
    }

    /// The personas a browsing surface may list, with what is wrong with each.
    /// `tz` is `[agent] timezone`; `None` is the machine's own zone, as
    /// `Config::timezone` documents — not UTC (review of #418).
    pub fn list(
        &self,
        library: &LibraryState,
        token: Option<&str>,
        tz: Option<chrono_tz::Tz>,
    ) -> serde_json::Value {
        let now = chrono::Utc::now();
        let doses = match tz {
            Some(tz) => safety::doses(&self.store, now, &tz),
            None => safety::doses(&self.store, now, &chrono::Local),
        };
        let store = Store::load(&self.store);
        let lib = mecha_core::imagelib::Library::load(&library.dir).0;
        let unlocked = library.unlocked(token);
        let rows: Vec<serde_json::Value> = store
            .visible(unlocked)
            .map(|p| {
                let linked = p
                    .settings
                    .character
                    .as_deref()
                    .and_then(|c| lib.get(mecha_core::imagelib::Kind::Character, c));
                // A visible persona linked to a locked character does not name
                // it to a locked page: a name says more than the count the
                // owner ruled out (2026-09-30). The link itself is untouched.
                let hidden = hidden_character(p, &lib, unlocked);
                let character = match hidden {
                    Some(_) => None,
                    None => p.settings.character.as_deref(),
                };
                // The linked character's portrait, by the library's own rule:
                // approved only, and a locked one only with the live token.
                let portrait = linked
                    .filter(|e| e.status == mecha_core::persona::Status::Approved)
                    .filter(|e| !e.locked || unlocked)
                    .and_then(|e| super::library::portrait_url(e, token.filter(|_| unlocked)));
                serde_json::json!({
                    "name": p.name,
                    "display": p.display(),
                    "relationship": p.settings.relationship.0,
                    "character": character,
                    "portrait": portrait,
                    // Where the owner placed it in the circle; null is the
                    // page's default framing.
                    "frame": p.state.frame,
                    "version": p.state.version,
                    "approved": p.state.status == mecha_core::persona::Status::Approved,
                    // Waiting on the owner — the page's Waiting section —
                    // and where it came from, said as the library says it.
                    "waiting": p.state.status == mecha_core::persona::Status::Candidate
                        && p.state.proposed.is_some(),
                    "origin": p.state.origin,
                    "locked": p.state.locked,
                    "problems": unnamed(store.problems(p, &lib), hidden),
                    "safety": safety_json(&p.settings.safety, None),
                    // The meters are shown to the owner, never to the model.
                    "dose": p.settings.safety.dose.then(|| match &doses.unreadable {
                        // Unread is said, not shown as zero turns.
                        Some(why) => serde_json::json!({ "unread": why }),
                        None => {
                            let d = doses.by_persona.get(&p.name).copied().unwrap_or_default();
                            serde_json::json!({
                                "turns_today": d.turns_today,
                                "turns_7d": d.turns_7d,
                                "late_night_7d": d.late_night_7d,
                                "call_secs_today": d.call_secs_today,
                                "call_secs_7d": d.call_secs_7d,
                                "skipped": doses.skipped,
                            })
                        }
                    }),
                })
            })
            .collect();
        // No count of what is hidden: "one hidden" is the lock telling on
        // itself. A locked page reads exactly like one with nothing locked.
        serde_json::json!({
            "personas": rows,
            "unlocked": unlocked,
            "has_password": mecha_core::imagelib::has_lock_password(&library.dir),
        })
    }

    /// Open a new chat with `name`, pinned to its current version: a
    /// transcript in its own `sessions/`, a workspace of its own, and the
    /// agent built (or reused) so a tool it asked for and did not get is said
    /// now rather than discovered.
    pub async fn open(
        &self,
        chat: &Arc<ChatState>,
        library: &LibraryState,
        name: &str,
        token: Option<&str>,
        goal: Option<String>,
    ) -> Result<serde_json::Value, Refusal> {
        let goal = goal.map(|g| g.trim().to_string()).filter(|g| !g.is_empty());
        // One line: the page strips the framing a goal rides in by its line
        // (`ownWords`), and so does the earlier-chats opener (review of #479).
        if goal.as_ref().is_some_and(|g| g.contains('\n')) {
            return Err(Refusal::Bad("a goal is one line".into()));
        }
        if goal.as_ref().is_some_and(|g| g.chars().count() > MAX_GOAL) {
            return Err(Refusal::Bad(format!(
                "a session goal is at most {MAX_GOAL} characters"
            )));
        }
        if chat.stopping.is_cancelled() {
            return Err(Refusal::Failed("server is shutting down".into()));
        }
        let p = self
            .visible(library, name, token)
            .ok_or(Refusal::NotFound)?;
        let pinned = persona_agent::pin(&self.store, &p.name)
            .map_err(|e| Refusal::Conflict(format!("{e:#}")))?;
        let bound = chat.follower.current();
        let (_, refused) = self.agent_for(&bound, &pinned).map_err(failed)?;
        let dir = Store::load(&self.store).sessions_dir(&p.name);
        mecha_core::create_private_dir(&dir)
            .with_context(|| format!("creating {}", dir.display()))
            .map_err(failed)?;
        let key = new_key();
        let workspace = self.work.join(&key);
        mecha_core::create_private_dir(&workspace)
            .with_context(|| format!("creating {}", workspace.display()))
            .map_err(failed)?;
        let session = Session::create(
            &dir,
            SessionMeta {
                id: Session::new_id(),
                created_at: chrono::Utc::now(),
                provider: bound.provider_name.clone(),
                model: bound.model.clone(),
                workspace: workspace.clone(),
                title: Some(format!("persona: {}", p.display())),
                // A label only: the directory is what keeps it apart.
                kind: None,
            },
        )
        .map_err(failed)?;
        let pin = PinRecord {
            persona: p.name.clone(),
            version: pinned.version,
            digest: pinned.digest.clone(),
            goal: goal.clone(),
        };
        let pin_bytes = serde_json::to_vec_pretty(&pin).map_err(|e| failed(e.into()))?;
        std::fs::write(pin_path(&dir, &session.meta.id), pin_bytes)
            .map_err(|e| failed(e.into()))?;
        let id = session.meta.id.clone();
        let version = pinned.version;
        let (events, _) = broadcast::channel(512);
        self.sessions.lock().await.insert(
            key.clone(),
            PersonaSession {
                recorded_generation: None,
                pinned: Arc::new(pinned),
                session: Arc::new(session),
                conversation: Some(Conversation::new()),
                workspace,
                live: None,
                events,
                last_usage: Arc::default(),
                goal,
                turns_since_anchor: 0,
                anchor_due: false,
                crisis_paused_at: None,
                crisis_shown: false,
                memory: None,
                pending_crisis: None,
                judge_answered: None,
                listen: None,
            },
        );
        Ok(serde_json::json!({
            "key": key,
            "session": id,
            "persona": p.name,
            "display": p.display(),
            "version": version,
            "refused": refused_json(&refused),
        }))
    }

    /// What `name` remembers, for the owner's curation page (§9.8): every
    /// episode and fact still standing — candidates waiting on the owner
    /// included, withdrawn ones not — each dated by when it was said, in
    /// `tz`, with this persona's shared copies of its facts about the owner.
    /// Read-only and never created: no store yet is an empty page. Only this
    /// persona's own rows, so a locked persona's facts shared with everyone
    /// can never surface through another's page.
    pub fn memory(
        &self,
        library: &LibraryState,
        name: &str,
        token: Option<&str>,
        tz: Option<chrono_tz::Tz>,
    ) -> Result<serde_json::Value, Refusal> {
        use mecha_core::persona::memory::{Filter, Memory, Shared, Status, Table};
        let p = self
            .visible(library, name, token)
            .ok_or(Refusal::NotFound)?;
        let groups: Vec<String> = Store::load(&self.store).groups().keys().cloned().collect();
        let Some(m) = Memory::open_existing(&self.store, &p.name).map_err(failed)? else {
            return Ok(serde_json::json!({
                "episodes": [], "facts": {"persona": [], "user": [], "inferred": []},
                "groups": groups, "shared_unreadable": 0,
            }));
        };
        let standing = |s: Status| s != Status::Invalidated;
        let day = |at: &str| mecha_core::persona::recall::local_day(at, tz);
        let mut shared: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
        // A shared copy this binary cannot read is said, never shown as "not
        // shared": this is the page where the owner decides who else reads a
        // fact about them (review of #519). Counted store-wide — it cannot
        // say whose it is — and unshareable only from the CLI.
        let mut shared_unreadable = 0;
        // A shared store that will not read costs the share marks, not the
        // page whose job includes forgetting (review of #519; recall's rule,
        // review of #477).
        let mut shared_problem = None;
        let listing =
            Shared::open_existing(&self.store).and_then(|sh| sh.map(|sh| sh.all()).transpose());
        let listing = match listing {
            Ok(l) => l,
            Err(e) => {
                shared_problem = Some(format!("{e:#}"));
                None
            }
        };
        if let Some(listing) = listing {
            shared_unreadable = listing.unreadable;
            for f in listing.facts {
                if f.learned_by == p.name {
                    // `group: null` is everyone.
                    let group = match &f.audience {
                        mecha_core::persona::memory::Audience::Everyone => None,
                        mecha_core::persona::memory::Audience::Group(g) => Some(g.clone()),
                    };
                    shared
                        .entry(f.from_uid.clone())
                        .or_default()
                        .push(serde_json::json!({
                            "uid": f.uid, "group": group,
                        }));
                }
            }
        }
        let episodes: Vec<serde_json::Value> = m
            .episodes(Filter::All)
            .map_err(failed)?
            .into_iter()
            .filter(|e| standing(e.status))
            .map(|e| {
                let said = e.said_at();
                serde_json::json!({
                    "uid": e.uid, "day": day(&said), "said_at": said,
                    "summary": e.summary, "open_threads": e.open_threads,
                    "chat": e.source.chat, "origin": e.origin,
                    "status": e.status, "pinned": e.pinned,
                })
            })
            .collect();
        let mut facts = serde_json::Map::new();
        for (key, table) in [
            ("persona", Table::Persona),
            ("user", Table::User),
            ("inferred", Table::Inferred),
        ] {
            let rows: Vec<serde_json::Value> = m
                .facts(table, Filter::All)
                .map_err(failed)?
                .into_iter()
                .filter(|f| standing(f.status))
                .map(|f| {
                    let said = f.said_at();
                    serde_json::json!({
                        "uid": f.uid, "day": day(&said), "said_at": said,
                        "text": f.text, "kind": f.kind, "origin": f.origin,
                        "status": f.status, "pinned": f.pinned,
                        "shared": shared.remove(&f.uid).unwrap_or_default(),
                    })
                })
                .collect();
            facts.insert(key.into(), rows.into());
        }
        Ok(serde_json::json!({
            "episodes": episodes, "facts": facts, "groups": groups,
            "shared_unreadable": shared_unreadable, "shared_problem": shared_problem,
        }))
    }

    /// One curation act on `name`'s memory (§9.8), then the page as it now
    /// stands. The lock holds as for every write. `forget` is the only real
    /// delete, and takes the shared copies with it.
    pub fn memory_act(
        &self,
        library: &LibraryState,
        name: &str,
        act: MemoryAct,
        token: Option<&str>,
        tz: Option<chrono_tz::Tz>,
    ) -> Result<serde_json::Value, Refusal> {
        use mecha_core::persona::memory::{self, Audience, Memory, Shared};
        let p = self
            .visible(library, name, token)
            .ok_or(Refusal::NotFound)?;
        let bad = |e: anyhow::Error| Refusal::Bad(format!("{e:#}"));
        let m = Memory::open_to_edit(&self.store, &p.name).map_err(bad)?;
        match act {
            MemoryAct::Approve { id } => m.approve(&m.resolve(&id).map_err(bad)?).map_err(bad)?,
            MemoryAct::Pin { id } => m.pin(&m.resolve(&id).map_err(bad)?, true).map_err(bad)?,
            MemoryAct::Unpin { id } => m.pin(&m.resolve(&id).map_err(bad)?, false).map_err(bad)?,
            MemoryAct::Correct { id, text } => {
                m.correct(&m.resolve(&id).map_err(bad)?, text.trim())
                    .map_err(bad)?;
            }
            MemoryAct::Forget { id } => {
                let uid = m.resolve(&id).map_err(bad)?;
                drop(m);
                memory::forget(&self.store, &p.name, &uid).map_err(failed)?;
            }
            MemoryAct::Share { id, group } => {
                let uid = m.resolve(&id).map_err(bad)?;
                let fact = m
                    .fact(&uid)
                    .map_err(failed)?
                    .ok_or_else(|| Refusal::Bad("only a fact can be shared".into()))?;
                let declared: Vec<String> =
                    Store::load(&self.store).groups().keys().cloned().collect();
                let audience = match group {
                    Some(g) => Audience::Group(g),
                    None => Audience::Everyone,
                };
                Shared::open(&self.store)
                    .map_err(failed)?
                    .share(&fact, audience, &declared)
                    .map_err(bad)?;
            }
            MemoryAct::Unshare { id } => {
                // Never created by asking, as the CLI's door guards.
                if !Shared::path(&self.store).is_file() {
                    return Err(Refusal::Bad("nothing is shared".into()));
                }
                let sh = Shared::open(&self.store).map_err(failed)?;
                let uid = sh.resolve(&id).map_err(bad)?;
                // Only a copy this persona learned: another's is not this
                // page's to touch, and answers as missing.
                let mine = sh
                    .all()
                    .map_err(failed)?
                    .facts
                    .iter()
                    .any(|f| f.uid == uid && f.learned_by == p.name);
                if !mine {
                    return Err(Refusal::Bad(format!(
                        "{} shared nothing with that id",
                        p.name
                    )));
                }
                sh.unshare(&uid).map_err(bad)?;
            }
        }
        self.memory(library, name, token, tz)
    }

    /// Earlier chats with `name`, newest first.
    pub fn history(
        &self,
        library: &LibraryState,
        name: &str,
        token: Option<&str>,
    ) -> Result<serde_json::Value, Refusal> {
        let p = self
            .visible(library, name, token)
            .ok_or(Refusal::NotFound)?;
        let dir = Store::load(&self.store).sessions_dir(&p.name);
        let listed = Session::list(&dir).map_err(failed)?;
        let archived = mecha_core::archive::archived(&dir).unwrap_or_default();
        // What each chat was about, as the memory writer summed it up
        // (§9): its pinned or most recent episode, which `episodes` lists
        // first. Read-only and never created;
        // no memory yet is no summary, not a failed list.
        let summaries: HashMap<String, String> =
            mecha_core::persona::memory::Memory::open_existing(&self.store, &p.name)
                .ok()
                .flatten()
                // Accepted episodes only: a candidate (a chat that carried
                // its files is one until approved) or a withdrawn one is not
                // the owner's account of the chat (review of #479).
                .and_then(|m| {
                    m.episodes(mecha_core::persona::memory::Filter::Recallable)
                        .ok()
                })
                .map(|eps| {
                    let mut out = HashMap::new();
                    for e in eps {
                        out.entry(e.source.chat).or_insert(e.summary);
                    }
                    out
                })
                .unwrap_or_default();
        let rows: Vec<serde_json::Value> = listed
            .into_iter()
            .filter(|(meta, _)| !archived.contains_key(&meta.id))
            .take(40)
            .map(|(meta, _)| {
                // The goal the chat was opened with, from its pin: the one
                // thing that tells two chats apart at a glance. A missing or
                // unreadable pin is no goal, not a failed list.
                let goal = std::fs::read(pin_path(&dir, &meta.id))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<PinRecord>(&b).ok())
                    .and_then(|p| p.goal);
                serde_json::json!({
                    "id": meta.id,
                    "created": meta.created_at,
                    "title": meta.title,
                    "goal": goal,
                    "summary": summaries.get(&meta.id),
                    // Until the nightly writer has summed a chat up: how
                    // the owner opened it.
                    "opener": opener(&dir.join(format!("{}.jsonl", meta.id))),
                })
            })
            .collect();
        Ok(serde_json::json!({ "persona": p.name, "chats": rows }))
    }

    /// Pick an earlier chat back up, on the version it was pinned to.
    pub async fn resume(
        &self,
        chat: &Arc<ChatState>,
        library: &LibraryState,
        name: &str,
        id: &str,
        token: Option<&str>,
    ) -> Result<serde_json::Value, Refusal> {
        if chat.stopping.is_cancelled() {
            return Err(Refusal::Failed("server is shutting down".into()));
        }
        let p = self
            .visible(library, name, token)
            .ok_or(Refusal::NotFound)?;
        let open = {
            let sessions = self.sessions.lock().await;
            sessions
                .iter()
                .find(|(_, ps)| ps.session.meta.id == id && ps.pinned.name == p.name)
                .map(|(key, ps)| (key.clone(), Arc::clone(&ps.pinned)))
        };
        if let Some((key, pinned)) = open {
            let (_, refused) = self
                .agent_for(&chat.follower.current(), &pinned)
                .map_err(failed)?;
            return Ok(serde_json::json!({ "key": key, "refused": refused_json(&refused) }));
        }
        let dir = Store::load(&self.store).sessions_dir(&p.name);
        // An exact id, not `Session::find`'s prefix: an empty or partial id
        // from a page bug must miss rather than resume whichever chat it
        // happens to match (review of #409).
        let path = dir.join(format!("{id}.jsonl"));
        if !valid_session_id(id) || !path.is_file() {
            return Err(Refusal::NotFound);
        }
        let (meta, conversation) = Session::load(&path).map_err(failed)?;
        let pin: PinRecord = std::fs::read(pin_path(&dir, &meta.id))
            .map_err(anyhow::Error::from)
            .and_then(|b| serde_json::from_slice(&b).map_err(anyhow::Error::from))
            .with_context(|| format!("reading which version chat {} is pinned to", meta.id))
            .map_err(|e| Refusal::Conflict(format!("{e:#}")))?;
        if pin.persona != p.name {
            return Err(Refusal::NotFound);
        }
        // As `open` refuses through `pin`: a persona that stopped being
        // approved after the chat was made does not speak again.
        if p.state.status != mecha_core::persona::Status::Approved {
            return Err(Refusal::Conflict(format!(
                "`{}` is not approved — `mecha persona approve {}` after reading it",
                p.name, p.name
            )));
        }
        let pinned = persona_agent::load_version(&self.store, &p.name, &pin.digest)
            .map_err(|e| Refusal::Conflict(format!("{e:#}")))?;
        let bound = chat.follower.current();
        let (_, refused) = self.agent_for(&bound, &pinned).map_err(failed)?;
        // A goal set at open and never sent — the chat was opened and left.
        let goal = pin.goal.filter(|_| conversation.messages.is_empty());
        // A chat picked back up re-anchors on its first turn.
        let anchor_due = !conversation.messages.is_empty();
        let session = Session { meta, path };
        let (events, _) = broadcast::channel(512);
        let mut sessions = self.sessions.lock().await;
        // Again, under the lock the insert takes: the loads above ran
        // without it, and two resumes of one chat must not become two live
        // sessions writing one transcript (found on review of #409).
        if let Some((key, _)) = sessions
            .iter()
            .find(|(_, ps)| ps.session.meta.id == id && ps.pinned.name == p.name)
        {
            return Ok(serde_json::json!({ "key": key, "refused": refused_json(&refused) }));
        }
        let key = new_key();
        // The workspace the chat began in, as its session records it: a
        // resumed chat keeps its pictures, and the persona can edit one it
        // drew before a restart (owner, 2026-09-30 — a resume under a new
        // key had given the same chat an empty workspace). Only a directory
        // directly in the persona work dir is trusted; anything else gets a
        // fresh one.
        let workspace = if session.meta.workspace.parent() == Some(self.work.as_path()) {
            session.meta.workspace.clone()
        } else {
            self.work.join(&key)
        };
        mecha_core::create_private_dir(&workspace).map_err(|e| failed(e.into()))?;
        sessions.insert(
            key.clone(),
            PersonaSession {
                recorded_generation: None,
                pinned: Arc::new(pinned),
                session: Arc::new(session),
                conversation: Some(conversation),
                workspace,
                live: None,
                events,
                last_usage: Arc::default(),
                goal,
                turns_since_anchor: 0,
                anchor_due,
                crisis_paused_at: None,
                crisis_shown: false,
                memory: None,
                pending_crisis: None,
                judge_answered: None,
                listen: None,
            },
        );
        Ok(serde_json::json!({ "key": key, "refused": refused_json(&refused) }))
    }

    /// The persona a live chat is with, if the caller may see it.
    async fn persona_of(
        &self,
        library: &LibraryState,
        key: &str,
        token: Option<&str>,
    ) -> Result<String, Refusal> {
        if !chat::valid_key(key) || !is_persona_key(key) {
            return Err(Refusal::Bad("bad persona chat key".into()));
        }
        let name = {
            let sessions = self.sessions.lock().await;
            sessions
                .get(key)
                .map(|ps| ps.pinned.name.clone())
                .ok_or(Refusal::NotFound)?
        };
        self.visible(library, &name, token)
            .map(|p| p.name)
            .ok_or(Refusal::NotFound)
    }

    /// The chat's workspace, behind the lock as every door here is: the
    /// pictures it drew and the masks the owner painted are the chat's, so a
    /// locked persona's answer 404 without the token, as its words do.
    /// The files `name` can read (§10.2): its own, its groups', everyone's,
    /// each with whether its text is already extracted.
    pub async fn sources(
        &self,
        chat: &Arc<ChatState>,
        library: &LibraryState,
        name: &str,
        token: Option<&str>,
    ) -> Result<serde_json::Value, Refusal> {
        let p = self
            .visible(library, name, token)
            .ok_or(Refusal::NotFound)?;
        // Read off the config rather than by building an extractor on every
        // poll (review of #459).
        let (max_bytes, cached) = reader_shape(&chat.follower.current().config);
        let store_dir = self.store.clone();
        // A read longer than this has died, or is not worth a spinner.
        const STALE: std::time::Duration = std::time::Duration::from_secs(600);
        let processing: std::collections::HashSet<PathBuf> = self
            .processing
            .lock()
            .map(|mut s| {
                s.retain(|_, since| since.is_none_or(|t| t.elapsed() < STALE));
                s.keys().cloned().collect()
            })
            .unwrap_or_default();
        // Hashing every document to ask the cache is file I/O: off the
        // runtime.
        let rows = tokio::task::spawn_blocking(move || {
            let store = Store::load(&store_dir);
            let cache = reader_cache(cached);
            let sources =
                mecha_core::persona::files::list(&mecha_core::persona::files::roots(&store, &p));
            sources
                .iter()
                .map(|s| {
                    let state = readiness(s, cache.as_ref(), max_bytes);
                    serde_json::json!({
                        "name": s.name,
                        "bytes": s.bytes,
                        "kind": match s.kind {
                            mecha_core::persona::files::Kind::Document => "document",
                            mecha_core::persona::files::Kind::Text => "text",
                        },
                        "shared": s.name.starts_with('@'),
                        // Ready, or why it never will be; "processing"
                        // says a read is queued or running.
                        "ready": matches!(state, Readiness::Ready),
                        // Read when a chat asks: nothing keeps a read ahead.
                        "on_request": matches!(state, Readiness::OnRequest),
                        "unreadable": match &state {
                            Readiness::Refused(why) => Some(why.as_str()),
                            _ => None,
                        },
                        // Being read, not merely being indexed: a ready file
                        // reads as ready (review of #467).
                        "processing": processing.contains(&s.path)
                            && !matches!(state, Readiness::Ready),
                    })
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|e| Refusal::Failed(format!("listing files: {e}")))?;
        Ok(serde_json::json!({ "sources": rows }))
    }

    /// Add a file the owner dropped on `name`'s page to its own folder
    /// (§10.3), then read it once in the background so the first chat that
    /// uses it does not wait.
    pub async fn add_source(
        &self,
        chat: &Arc<ChatState>,
        library: &LibraryState,
        name: &str,
        file_name: &str,
        token: Option<&str>,
        body: axum::body::Bytes,
    ) -> Result<serde_json::Value, Refusal> {
        let p = self
            .visible(library, name, token)
            .ok_or(Refusal::NotFound)?;
        let store_dir = self.store.clone();
        let file_name = file_name.to_string();
        let config = chat.follower.current().config.clone();
        let (max_bytes, cached) = reader_shape(&config);
        let embedded = crate::setup::file_embedder(&config).is_some();
        let (saved, src) = tokio::task::spawn_blocking(move || {
            let store = Store::load(&store_dir);
            let saved = mecha_core::persona::files::add(&store, &p, &file_name, &body)?;
            let own = store.files_roots(&p).into_iter().next();
            let src = own.and_then(|root| {
                let listed = mecha_core::persona::files::list(&[(String::new(), root)]);
                mecha_core::persona::files::find(&listed, &saved)
                    .ok()
                    .cloned()
            });
            // Read only what a read would leave in the cache, as
            // `files_block` does: not with the cache off, not a file that
            // will be refused (review of #459, pass 9).
            let cache = reader_cache(cached);
            let index = mecha_core::persona::search::Index::open(&store_dir).ok();
            let src = src.filter(|s| {
                to_process(&readiness(s, cache.as_ref(), max_bytes), || {
                    indexed(index.as_ref(), s, embedded)
                })
            });
            Ok::<_, String>((saved, src))
        })
        .await
        .map_err(|e| Refusal::Failed(format!("saving the file: {e}")))?
        .map_err(Refusal::Bad)?;
        if let Some(src) = src {
            self.read_in_background(
                src,
                crate::setup::document_extractor(&config).map(Arc::new),
                crate::setup::file_embedder(&config),
            );
        }
        Ok(serde_json::json!({ "name": saved }))
    }

    /// Take one of `name`'s own files out of reach (§10.3).
    pub async fn remove_source(
        &self,
        library: &LibraryState,
        name: &str,
        file: &str,
        token: Option<&str>,
    ) -> Result<serde_json::Value, Refusal> {
        let p = self
            .visible(library, name, token)
            .ok_or(Refusal::NotFound)?;
        let store_dir = self.store.clone();
        let file = file.to_string();
        tokio::task::spawn_blocking(move || {
            mecha_core::persona::files::remove(&Store::load(&store_dir), &p, &file)
        })
        .await
        .map_err(|e| Refusal::Failed(format!("removing the file: {e}")))?
        .map_err(Refusal::Bad)?;
        Ok(serde_json::json!({ "removed": true }))
    }

    /// What the owner's message brings to mind, for a turn past the first
    /// reply (§9.7), or `None`. The message is embedded on the embeddings
    /// server, waiting out its cold start (`RECALL_EMBED_WAIT`, on demand
    /// since 2026-09-29); slower than that, or down, and the search is by
    /// words alone — a turn is never held for long on memory. A store that
    /// will not read is logged, not noticed: the chat's first turn already
    /// said so, and a notice on every turn would bury the chat.
    async fn recall_block(
        &self,
        persona: mecha_core::persona::Persona,
        bound: &crate::follow::Bound,
        message: &str,
        already: String,
    ) -> Option<String> {
        // Asked first: the embed is the slow part, and it is spent only on a
        // turn that will search (review of #481).
        if !mecha_core::persona::recall::would_search(&self.store, &persona, message) {
            return None;
        }
        let qvec = match crate::setup::file_embedder(&bound.config) {
            Some(embedder) => {
                match tokio::time::timeout(RECALL_EMBED_WAIT, embedder.recall_query(message)).await
                {
                    Ok(Ok(v)) => Some(v),
                    Ok(Err(e)) => {
                        tracing::info!("recall by words only: {e:#}");
                        None
                    }
                    Err(_) => {
                        tracing::info!("recall by words only: the embeddings server was slow");
                        None
                    }
                }
            }
            None => None,
        };
        let dir = self.store.clone();
        let message = message.to_owned();
        let tz = bound.config.agent.timezone();
        let found = tokio::task::spawn_blocking(move || {
            mecha_core::persona::recall::per_turn(
                &dir,
                &persona,
                &message,
                qvec.as_deref(),
                &already,
                tz,
            )
        })
        .await;
        match found {
            Ok(Ok(block)) => block.map(|b| b.text),
            Ok(Err(e)) => {
                tracing::warn!("a persona's memory could not be searched: {e:#}");
                None
            }
            Err(e) => {
                tracing::warn!("searching a persona's memory failed: {e}");
                None
            }
        }
    }

    /// The persona's memory block for a chat's first turn (§9.7), or `None`.
    /// A store that cannot be read is said on the page and in the log, never
    /// taken for "remembers nothing" — and the turn goes ahead without it.
    async fn memory_block(
        &self,
        persona: mecha_core::persona::Persona,
        tz: Option<chrono_tz::Tz>,
        notices: &tokio::sync::broadcast::Sender<WireEvent>,
    ) -> Option<String> {
        let dir = self.store.clone();
        let read = tokio::task::spawn_blocking(move || {
            mecha_core::persona::recall::chat_start(&Store::load(&dir), &persona, tz)
        })
        .await;
        let unread = |why: &str| {
            let _ = notices.send(WireEvent::Notice {
                text: format!(
                    "Part of what it remembers could not be read ({why}); this chat goes on without that part."
                ),
            });
        };
        match read {
            Ok(Ok(recalled)) => {
                // Read, but not all of it: said, as an unreadable store is.
                for problem in &recalled.problems {
                    tracing::warn!("a persona's memory was not all read: {problem}");
                    unread(problem);
                }
                recalled.block.map(|b| b.text)
            }
            Ok(Err(e)) => {
                tracing::warn!("a persona's memory could not be read: {e:#}");
                unread("its store");
                None
            }
            // A panic in the read is no quieter than an error (review of #477).
            Err(e) => {
                tracing::warn!("reading a persona's memory failed: {e}");
                unread("the read failed");
                None
            }
        }
    }

    /// What this persona's files say for a chat's first turn (§10.4): all
    /// their text when it fits about a quarter of the context window (D15),
    /// or the list to read with `file_read`. `None` with no files.
    async fn files_block(&self, name: &str, bound: &crate::follow::Bound) -> Option<String> {
        let (store_dir, name) = (self.store.clone(), name.to_string());
        let config = &bound.config;
        let extractor = crate::setup::document_extractor(config).map(Arc::new);
        let (max_bytes, cached) = match extractor {
            Some(_) => reader_shape(config),
            // No reader was built, whatever the config says: nothing waits
            // on one (review of #459, pass 8).
            None => (0, false),
        };
        let embedder = crate::setup::file_embedder(config);
        let embedded = embedder.is_some();
        // The store, the walk and the readiness hashes are file I/O: off the
        // runtime.
        let (sources, omitted, states, unindexed) = tokio::task::spawn_blocking(move || {
            let store = Store::load(&store_dir);
            let p = store.get(&name)?;
            let (sources, omitted) =
                mecha_core::persona::files::listing(&mecha_core::persona::files::roots(&store, p));
            let cache = reader_cache(cached);
            let states: HashMap<PathBuf, Readiness> = sources
                .iter()
                .map(|s| (s.path.clone(), readiness(s, cache.as_ref(), max_bytes)))
                .collect();
            let index = mecha_core::persona::search::Index::open(&store_dir).ok();
            // Asked of ready files only (`to_process`).
            let unindexed: std::collections::HashSet<PathBuf> = sources
                .iter()
                .filter(|s| {
                    states.get(&s.path) == Some(&Readiness::Ready)
                        && !indexed(index.as_ref(), s, embedded)
                })
                .map(|s| s.path.clone())
                .collect();
            Some((sources, omitted, states, unindexed))
        })
        .await
        .map_err(|e| tracing::warn!("a persona's files were not listed: {e}"))
        .ok()
        .flatten()?;
        if sources.is_empty() {
            return None;
        }
        // What the first turn cannot include yet is read in the background,
        // so a later `file_read` — or the next chat — finds it ready; and
        // what the index does not hold is indexed, so `file_search` finds
        // it. Only what a read would keep: a refused file never will be,
        // and with the cache off a read would be thrown away.
        for src in sources.iter().filter(|s| {
            states
                .get(&s.path)
                .is_some_and(|st| to_process(st, || !unindexed.contains(&s.path)))
        }) {
            self.read_in_background(src.clone(), extractor.clone(), embedder.clone());
        }
        // A quarter of the window in tokens, at about four characters a
        // token, is about the window's size in characters.
        let budget = bound.context_window.unwrap_or(32_768) as usize;
        mecha_core::persona::files::first_turn(
            &sources,
            omitted,
            extractor.as_deref(),
            budget,
            &|s| {
                states
                    .get(&s.path)
                    .cloned()
                    // Listed since the walk: not seen, so not read here.
                    .unwrap_or(Readiness::Reading)
            },
        )
        .await
    }

    /// Read a file once in the background (§10.3) and index it for
    /// `file_search` (§10.4), so a chat that wants it later does not wait —
    /// after an upload, and for whatever a first turn could not include yet
    /// or the index does not hold. Marked processing while it waits and
    /// while it runs; a file already queued or being read is left to that
    /// read.
    fn read_in_background(
        &self,
        src: mecha_core::persona::files::Source,
        extractor: Option<Arc<mecha_core::document::Extractor>>,
        embedder: Option<mecha_core::embed::Embedder>,
    ) {
        let processing = Arc::clone(&self.processing);
        match processing.lock() {
            Ok(mut set) => {
                if set.contains_key(&src.path) {
                    return;
                }
                set.insert(src.path.clone(), None);
            }
            Err(_) => return,
        }
        let reading = Arc::clone(&self.reading);
        let store = self.store.clone();
        tokio::spawn(async move {
            // Its turn: one read at a time, timed from when it starts.
            let Ok(_turn) = reading.acquire_owned().await else {
                return;
            };
            if let Ok(mut set) = processing.lock() {
                set.insert(src.path.clone(), Some(std::time::Instant::now()));
            }
            match mecha_core::persona::files::read(&src, extractor.as_deref(), "all", None).await {
                // Indexed from the text just read: a cached document costs
                // no second extraction, and an embeddings server that does
                // not answer leaves it searchable by words.
                Ok(text) => {
                    if let Err(e) = mecha_core::persona::search::index_file(
                        store,
                        &src,
                        text,
                        embedder.as_ref(),
                    )
                    .await
                    {
                        tracing::warn!("a persona file was not indexed: {e:#}");
                    }
                }
                Err(e) => tracing::warn!("a persona file was not read: {}", e.why),
            }
            if let Ok(mut set) = processing.lock() {
                set.remove(&src.path);
            }
        });
    }

    /// One of `name`'s files, as the page offers it to download: found by
    /// name in its listing (never a path the page names), served as an
    /// attachment of inert bytes — a paper is third-party content, and
    /// nothing but an image is served renderable (`files::serve`).
    pub async fn source_file(
        &self,
        library: &LibraryState,
        name: &str,
        file: &str,
        token: Option<&str>,
    ) -> Result<mecha_core::persona::files::Source, Refusal> {
        let p = self
            .visible(library, name, token)
            .ok_or(Refusal::NotFound)?;
        let (store_dir, file) = (self.store.clone(), file.to_string());
        tokio::task::spawn_blocking(move || {
            let store = Store::load(&store_dir);
            let sources =
                mecha_core::persona::files::list(&mecha_core::persona::files::roots(&store, &p));
            mecha_core::persona::files::find(&sources, &file)
                .cloned()
                .map_err(|_| Refusal::NotFound)
        })
        .await
        .map_err(|e| Refusal::Failed(format!("listing files: {e}")))?
    }

    /// One of `name`'s files as text, for the page to show: a text file
    /// whole, a document's text once it has been read (its pages from the
    /// cache). A document not read yet is said, never read here — an OCR
    /// pass is not something a page click should start (review of #459).
    pub async fn source_text(
        &self,
        chat: &Arc<ChatState>,
        library: &LibraryState,
        name: &str,
        file: &str,
        token: Option<&str>,
    ) -> Result<serde_json::Value, Refusal> {
        let src = self.source_file(library, name, file, token).await?;
        let config = chat.follower.current().config.clone();
        let (max_bytes, cached) = reader_shape(&config);
        let state = {
            let src = src.clone();
            tokio::task::spawn_blocking(move || {
                readiness(&src, reader_cache(cached).as_ref(), max_bytes)
            })
            .await
            .map_err(|e| Refusal::Failed(format!("reading the file: {e}")))?
        };
        match state {
            Readiness::Ready => {}
            Readiness::Refused(why) => return Err(Refusal::Bad(why)),
            Readiness::Reading => return Err(Refusal::Conflict("it is still being read".into())),
            // With the extraction cache off nothing is read ahead, so this
            // is not "yet" — a chat reads it when it asks.
            Readiness::OnRequest => {
                return Err(Refusal::Conflict(
                    "it is read only when a chat asks for it (the extraction cache is off)".into(),
                ))
            }
        }
        let extractor = crate::setup::document_extractor(&config).map(Arc::new);
        let text = mecha_core::persona::files::read(&src, extractor.as_deref(), "all", None)
            .await
            .map_err(|e| Refusal::Bad(e.why))?;
        Ok(serde_json::json!({ "file": src.name, "text": text }))
    }

    /// Save one of this chat's replies into the persona's own files
    /// (§10.5), on the owner's word: a study guide, a quiz, a glossary it
    /// wrote when asked. Only a reply this chat holds is saved — the page
    /// names it by its text, and text the persona did not write is refused,
    /// so the file is the persona's words and nobody else's. Indexed for
    /// `file_search` like any upload.
    pub async fn save_reply(
        &self,
        chat: &Arc<ChatState>,
        library: &LibraryState,
        key: &str,
        text: &str,
        token: Option<&str>,
    ) -> Result<serde_json::Value, Refusal> {
        let name = self.persona_of(library, key, token).await?;
        let wanted = text.trim().to_string();
        let held = {
            let sessions = self.sessions.lock().await;
            let ps = sessions.get(key).ok_or(Refusal::NotFound)?;
            let messages: &[Message] = match (&ps.conversation, &ps.live) {
                (Some(c), _) => &c.messages,
                (None, Some(live)) => &live.history,
                (None, None) => &[],
            };
            messages
                .iter()
                .filter(|m| m.role == mecha_core::message::Role::Assistant)
                .any(|m| {
                    let blocks: Vec<&str> = m
                        .content
                        .iter()
                        .filter_map(|b| match b {
                            mecha_core::message::Block::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect();
                    // One block, or the message's blocks run together as
                    // `Message::text` and the page's stream join them — raw,
                    // with nothing between, trimmed only as a whole (review
                    // of #475: trimmed one by one, "a. " + "b" lost its space).
                    blocks.iter().any(|b| b.trim() == wanted) || blocks.concat().trim() == wanted
                })
        };
        if wanted.is_empty() || !held {
            return Err(Refusal::Bad("that is not a reply in this chat".into()));
        }
        let store_dir = self.store.clone();
        let config = chat.follower.current().config.clone();
        let embedded = crate::setup::file_embedder(&config).is_some();
        // The owner's calendar day, not the server's UTC one (review of #475).
        let day = match config.agent.timezone() {
            Some(tz) => chrono::Utc::now().with_timezone(&tz).date_naive(),
            None => chrono::Local::now().date_naive(),
        };
        let (saved, src) = tokio::task::spawn_blocking(move || {
            let store = Store::load(&store_dir);
            let p = store.get(&name).ok_or("no such persona")?.clone();
            let saved = mecha_core::persona::files::save_reply(&store, &p, &wanted, day)?;
            let own = store.files_roots(&p).into_iter().next();
            let src = own.and_then(|root| {
                let listed = mecha_core::persona::files::list(&[(String::new(), root)]);
                mecha_core::persona::files::find(&listed, &saved)
                    .ok()
                    .cloned()
            });
            // Unless the same content is already indexed (saved twice the
            // same day), as an upload asks (`to_process`).
            let index = mecha_core::persona::search::Index::open(&store_dir).ok();
            let src = src.filter(|s| !indexed(index.as_ref(), s, embedded));
            Ok::<_, String>((saved, src))
        })
        .await
        .map_err(|e| Refusal::Failed(format!("saving the reply: {e}")))?
        .map_err(Refusal::Bad)?;
        // Into the index, so the next chat's search finds it.
        if let Some(src) = src {
            let config = chat.follower.current().config.clone();
            self.read_in_background(src, None, crate::setup::file_embedder(&config));
        }
        Ok(serde_json::json!({ "name": saved }))
    }

    /// One cited page as this chat received it, with the quoted passage
    /// marked (§10.4): what a citation opens. Text drawn as text, never the
    /// file itself — a paper is third-party content, and nothing but an
    /// image is served renderable (`files::serve`). Behind the lock, as the
    /// transcript is.
    pub async fn cited(
        &self,
        library: &LibraryState,
        key: &str,
        file: &str,
        page: Option<u32>,
        quote: &str,
        token: Option<&str>,
    ) -> Result<serde_json::Value, Refusal> {
        self.persona_of(library, key, token).await?;
        let session = {
            let sessions = self.sessions.lock().await;
            Arc::clone(&sessions.get(key).ok_or(Refusal::NotFound)?.session)
        };
        let (file, quote) = (file.to_string(), quote.to_string());
        tokio::task::spawn_blocking(move || {
            let messages = std::fs::read_to_string(&session.path)
                .map(|t| Session::messages_ever(&t))
                .map_err(|e| Refusal::Failed(format!("reading the chat: {e}")))?;
            let mecha_core::persona::cite::Passage {
                text,
                span,
                through,
            } = mecha_core::persona::cite::passage(&messages, &file, page, &quote)
                .ok_or(Refusal::NotFound)?;
            let (before, marked, after) = match span {
                Some((a, b)) => (&text[..a], &text[a..b], &text[b..]),
                None => (text.as_str(), "", ""),
            };
            Ok(serde_json::json!({
                "file": file,
                "page": page,
                // The last page shown: the next, where the quote ran over.
                "through": through,
                "before": before,
                "marked": marked,
                "after": after,
            }))
        })
        .await
        .map_err(|e| Refusal::Failed(format!("reading the page: {e}")))?
    }

    pub async fn workspace_of(
        &self,
        library: &LibraryState,
        key: &str,
        token: Option<&str>,
    ) -> Result<PathBuf, Refusal> {
        self.persona_of(library, key, token).await?;
        let sessions = self.sessions.lock().await;
        sessions
            .get(key)
            .map(|ps| ps.workspace.clone())
            .ok_or(Refusal::NotFound)
    }

    pub async fn transcript(
        &self,
        chat: &Arc<ChatState>,
        library: &LibraryState,
        key: &str,
        token: Option<&str>,
    ) -> Result<serde_json::Value, Refusal> {
        self.persona_of(library, key, token).await?;
        let bound = chat.follower.current();
        let sessions = self.sessions.lock().await;
        let ps = sessions.get(key).ok_or(Refusal::NotFound)?;
        let messages: &[Message] = match (&ps.conversation, &ps.live) {
            (Some(c), _) => &c.messages,
            (None, Some(live)) => &live.history,
            (None, None) => &[],
        };
        let entries = chat::transcript_entries(messages);
        // Checked once the lock is let go: the session file is read whole.
        let session = Arc::clone(&ps.session);
        let taint = match (&ps.conversation, &ps.live) {
            (Some(c), _) => Some(c.taint),
            (None, Some(live)) => Some(live.taint),
            (None, None) => None,
        };
        let usage = ps.last_usage.lock().ok().and_then(|u| u.clone());
        let mut out = serde_json::json!({
            "session": ps.session.meta.id,
            "persona": ps.pinned.name,
            "version": ps.pinned.version,
            "model": bound.model,
            "running": ps.live.is_some(),
            "goal": ps.goal,
            "crisis_shown": ps.crisis_shown,
            "working": ps
                .live
                .as_ref()
                .and_then(|l| l.working.lock().ok().and_then(|w| w.last().cloned()))
                // How long it has run, by this server's clock: the page adds
                // it to its own, so a phone whose clock disagrees still counts
                // from the right moment (review of #431).
                .map(|mut w| {
                    let elapsed = w["since"]
                        .as_str()
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                        .map(|t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_milliseconds().max(0));
                    w["elapsed_ms"] = serde_json::json!(elapsed);
                    w
                }),
            "display": ps.pinned.settings.display,
            // The switches as they stand, not as pinned: they are read live.
            "safety": safety_json(
                &Store::load(&self.store)
                    .get(&ps.pinned.name)
                    .map(|p| p.settings.safety)
                    .unwrap_or(ps.pinned.settings.safety),
                Some(ps.judge_answered == Some(true)),
            ),
            "entries": entries,
            "taint": taint.map(|t| serde_json::json!({
                "private": t.private, "untrusted": t.untrusted,
            })),
            "usage": usage.map(|u| serde_json::json!({
                "prompt_tokens": u.input_tokens + u.cache_read_input_tokens
                    + u.cache_creation_input_tokens,
                "context_window": bound.context_window,
            })),
        });
        drop(sessions);
        out["citations"] = serde_json::json!(citations(&session).await);
        Ok(out)
    }

    /// The chat's events, and whether its persona is locked — in which case
    /// the stream must end when the unlock does (`events`).
    pub async fn subscribe(
        &self,
        library: &LibraryState,
        key: &str,
        token: Option<&str>,
    ) -> Result<(broadcast::Receiver<WireEvent>, bool), Refusal> {
        let name = self.persona_of(library, key, token).await?;
        let locked = Store::load(&self.store)
            .get(&name)
            .is_none_or(|p| p.state.locked);
        let sessions = self.sessions.lock().await;
        sessions
            .get(key)
            .map(|ps| (ps.events.subscribe(), locked))
            .ok_or(Refusal::NotFound)
    }

    pub async fn cancel(
        &self,
        library: &LibraryState,
        key: &str,
        token: Option<&str>,
    ) -> Result<bool, Refusal> {
        self.persona_of(library, key, token).await?;
        let sessions = self.sessions.lock().await;
        Ok(match sessions.get(key).and_then(|ps| ps.live.as_ref()) {
            Some(live) => {
                live.cancel.cancel(mecha_core::agent::CancelReason::Stopped);
                true
            }
            None => false,
        })
    }

    /// `check_call` and `bind` in one, as the tests place a call.
    #[cfg(test)]
    pub async fn bind_call(
        &self,
        library: &LibraryState,
        key: &str,
        token: Option<String>,
    ) -> Result<String, Refusal> {
        let name = self.check_call(library, key, token.as_deref()).await?;
        self.bind(key, token);
        Ok(name)
    }

    /// The lock a call on `key` is placed under (§11), checked at the offer
    /// before anything binds or the worker hears the call exists: the
    /// persona's name for a chat the token shows, `NotFound` otherwise — a
    /// locked persona's call is refused exactly as its page is. The token is
    /// checked again on every spoken turn, so a relock mid-call ends it.
    pub async fn check_call(
        &self,
        library: &LibraryState,
        key: &str,
        token: Option<&str>,
    ) -> Result<String, Refusal> {
        self.persona_of(library, key, token).await
    }

    /// Hold `token` for calls on `key`, replacing any earlier binding; the
    /// id names this binding for `release_offer`.
    pub fn bind(&self, key: &str, token: Option<String>) -> u64 {
        let id = self
            .next_call
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.calls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(key.to_string(), (id, token));
        id
    }

    /// Release the binding offer `id` made, if it is still the one held — an
    /// offer the worker did not take must not unbind a call that a later
    /// offer placed.
    pub fn release_offer(&self, key: &str, id: u64) {
        let mut calls = self.calls.lock().unwrap_or_else(|p| p.into_inner());
        if calls.get(key).is_some_and(|(held, _)| *held == id) {
            calls.remove(key);
        }
    }

    /// Which personas speak in each library voice, by display name — for the
    /// voice library's "used by" (Library → Voices). Behind the lock as the
    /// persona list is: a locked persona is named only for an unlock.
    pub fn voices_in_use(&self, library: &LibraryState, token: Option<&str>) -> VoicesInUse {
        let store = Store::load(&self.store);
        let unlocked = library.unlocked(token);
        let mut by_voice: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for p in store.visible(unlocked) {
            if let Some(v) = &p.settings.voice {
                by_voice
                    .entry(v.clone())
                    .or_default()
                    .push(p.display().to_string());
            }
        }
        // Why the list may be short, said rather than shown as nobody: the
        // row carrying it has Delete (review of #490). "locked" is said
        // whenever the library is locked, never only when a locked persona
        // exists — that would be a count (the lock hides without one).
        // A persona file that did not load, not a groups or relationship
        // file: only the first can hide a voice's speaker. The lock first,
        // since unlocking is what the owner can do about it.
        let unread = store
            .errors()
            .iter()
            .any(|e| e.path.file_name().is_some_and(|f| f == "persona.toml"));
        let partial = if !unlocked {
            Some("locked")
        } else if unread {
            Some("unreadable")
        } else {
            None
        };
        VoicesInUse { by_voice, partial }
    }

    /// The voice a call to `name` speaks in (§11): the library voice its
    /// `voice` names, at its `voice_speed` if set; `None` for a persona with
    /// no voice, which speaks in the worker's own. Whether the TTS server
    /// lists the voice is the offer's to ask (`runner_voices`); a speed out of
    /// range is refused here by name — never a fallback, which is a persona
    /// silently speaking unlike itself.
    pub fn call_voice(&self, name: &str) -> Result<Option<CallVoice>, String> {
        let store = Store::load(&self.store);
        let Some(settings) = store.get(name).map(|p| p.settings.clone()) else {
            return Ok(None);
        };
        let Some(voice) = settings.voice else {
            return Ok(None);
        };
        // The range the worker takes, checked here so a bad value is
        // refused before the call rather than mid-sentence.
        if let Some(x) = settings.voice_speed {
            let range = mecha_core::persona::VOICE_SPEED;
            if !range.contains(&x) {
                return Err(format!(
                    "`{name}`: voice_speed {x} is outside {:.1}–{:.1}",
                    range.start(),
                    range.end()
                ));
            }
        }
        Ok(Some(CallVoice {
            voice,
            speed: settings.voice_speed,
        }))
    }

    /// A call on `key` has ended after `seconds` (§11): counted on the dose
    /// meter when the persona's switch is on, and the call's unlock let go —
    /// a later spoken turn on this key needs a new offer. Never the words.
    pub async fn call_ended(
        &self,
        library: &LibraryState,
        key: &str,
        token: Option<&str>,
        seconds: u32,
        call: Option<u64>,
    ) -> Result<(), Refusal> {
        // Only the binding this call's offer made: another tab's call, or a
        // redial that bound before this hang-up landed, keeps its own
        // (review of #483). A hang-up that names none releases nothing.
        // Ahead of the lock: letting a binding go only ever stops a call, so
        // a relock mid-call strands no binding — only the minutes, which
        // are behind the lock like every other door here.
        if let Some(id) = call {
            self.release_offer(key, id);
        }
        let name = self.persona_of(library, key, token).await?;
        let chat = {
            let sessions = self.sessions.lock().await;
            sessions
                .get(key)
                .map(|ps| ps.session.meta.id.clone())
                .ok_or(Refusal::NotFound)?
        };
        let counts = Store::load(&self.store)
            .get(&name)
            .is_some_and(|p| p.settings.safety.dose);
        if counts && seconds > 0 {
            safety::record_call(&self.store, &name, &chat, seconds).map_err(failed)?;
        }
        Ok(())
    }

    /// Whether a chat is open on `key` (`voice::SessionHost::holds`).
    pub async fn holds(&self, key: &str) -> bool {
        self.sessions.lock().await.contains_key(key)
    }

    /// What a Listen tap on `key` speaks with, as `ChatState::listen_seat`
    /// answers for the assistant: the persona's identity is who the
    /// director directs, and a persona chat always keeps its transcript.
    pub(super) async fn listen_seat(
        &self,
        key: &str,
        cue: &super::listen::ListenCue,
    ) -> Option<super::listen::Seat> {
        let (transcript, character, current) = {
            let sessions = self.sessions.lock().await;
            let ps = sessions.get(key)?;
            let character = mecha_core::persona::strip_comments(&ps.pinned.identity).0;
            (
                Arc::clone(&ps.session),
                Some(character).filter(|c| !c.trim().is_empty()),
                ps.listen.clone(),
            )
        };
        if let Some(reply) = current.filter(|r| r.continues(cue)) {
            return Some(super::listen::Seat {
                reply,
                transcript: Some(transcript),
            });
        }
        let saved = super::listen::recall(&transcript, cue).await;
        let reply = Arc::new(super::listen::Reply::new(cue, character, saved));
        self.sessions.lock().await.get_mut(key)?.listen = Some(Arc::clone(&reply));
        Some(super::listen::Seat {
            reply,
            transcript: Some(transcript),
        })
    }

    /// A spoken turn on `key`, barging in on any run in flight — the
    /// assistant's contract (`chat::VoiceHost`). Never `Hosted::Unknown`:
    /// that sends the words to the facade's own slot, where the assistant
    /// would answer a call placed to a persona.
    pub async fn speak(
        self: &Arc<Self>,
        chat: &Arc<ChatState>,
        library: &LibraryState,
        key: &str,
        utterance: &str,
        tts_streams: bool,
    ) -> crate::voice::Hosted {
        use crate::voice::{Hosted, HostedTurn};
        // No call was placed through the offer: nothing vouches for the lock.
        let Some(token) = self
            .calls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(key)
            .map(|(_, token)| token.clone())
        else {
            return Hosted::Failed("no call was placed to this persona chat".into());
        };
        // The door before the barge-in, which cancels the run in flight:
        // a relocked or unapproved persona's reply must not be stopped by a
        // call that is then turned away (the assistant's rule, review of
        // #376; here, review of #483). `start` checks again per turn.
        match self.persona_of(library, key, token.as_deref()).await {
            Err(r) => return Hosted::Failed(r.said()),
            Ok(name) => {
                let approved = Store::load(&self.store)
                    .get(&name)
                    .is_some_and(|p| p.state.status == mecha_core::persona::Status::Approved);
                if !approved {
                    return Hosted::Failed(format!("`{name}` is not approved"));
                }
            }
        }
        for _ in 0..BARGE_IN_TRIES {
            // The last reply, read while the conversation is still here: the
            // run about to start takes it.
            let last_reply;
            {
                let sessions = self.sessions.lock().await;
                if let Some(live) = sessions.get(key).and_then(|ps| ps.live.as_ref()) {
                    // Barge-in: the run stops at its next safe point and the
                    // next try finds the conversation back.
                    live.cancel.cancel(mecha_core::agent::CancelReason::Stopped);
                }
                last_reply = sessions
                    .get(key)
                    .and_then(|ps| ps.conversation.as_ref())
                    .and_then(|c| crate::voice::last_reply(&c.messages));
            }
            let (tap, events) = tokio::sync::mpsc::unbounded_channel();
            let (done_tx, done) = tokio::sync::oneshot::channel();
            let cancel = mecha_core::agent::CancelHandle::new();
            let spoken = Spoken {
                tap,
                done: done_tx,
                cancel: cancel.clone(),
                streams: tts_streams,
            };
            match self
                .start(
                    chat,
                    library,
                    key,
                    utterance,
                    None,
                    token.as_deref(),
                    Vec::new(),
                    false,
                    Some(spoken),
                )
                .await
            {
                Ok(_) => {
                    // After `start`, which creates a first chat's session:
                    // the transcript the director records in, and who speaks.
                    let seed = {
                        let sessions = self.sessions.lock().await;
                        sessions.get(key).map(|ps| crate::voice::DirectorSeed {
                            transcript: Some(Arc::clone(&ps.session)),
                            character: Some(
                                mecha_core::persona::strip_comments(&ps.pinned.identity).0,
                            )
                            .filter(|c| !c.trim().is_empty()),
                            last_reply: last_reply.clone(),
                        })
                    };
                    return Hosted::Started(Box::new(HostedTurn {
                        events,
                        done,
                        cancel,
                        seed: seed.unwrap_or_default(),
                    }));
                }
                Err(Refusal::Busy) => {}
                Err(r) => return Hosted::Failed(r.said()),
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        Hosted::Busy
    }

    /// Start a turn, or steer the one in flight.
    /// A text-only turn, as the tests drive one; the route is `send_with`.
    #[cfg(test)]
    pub async fn send(
        self: &Arc<Self>,
        chat: &Arc<ChatState>,
        library: &LibraryState,
        key: &str,
        text: &str,
        request_id: Option<String>,
        token: Option<&str>,
    ) -> Result<serde_json::Value, Refusal> {
        self.send_with(
            chat,
            library,
            key,
            text,
            request_id,
            token,
            Vec::new(),
            false,
        )
        .await
    }

    /// `send`, with the files the page uploaded for this turn (`upload`),
    /// already named in `text`. As in the assistant's chat, each picture
    /// among them also rides on the turn as pixels for a model that can see
    /// (`chat::attached_images`), which arms `private_data`; a blind model,
    /// the cap or an unreadable file leave it to its path, and the answer
    /// says how many were not shown. `edit`: the picture edit panel sent
    /// it, and the persona is told so (`persona::edit`); a message that
    /// steers a run in flight carries text only, and no note.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_with(
        self: &Arc<Self>,
        chat: &Arc<ChatState>,
        library: &LibraryState,
        key: &str,
        text: &str,
        request_id: Option<String>,
        token: Option<&str>,
        attachments: Vec<String>,
        edit: bool,
    ) -> Result<serde_json::Value, Refusal> {
        self.start(
            chat,
            library,
            key,
            text,
            request_id,
            token,
            attachments,
            edit,
            None,
        )
        .await
    }

    /// One turn, typed or spoken. A spoken one (`Some(spoken)`) never steers:
    /// a run in flight is `Busy`, for `speak` to barge in on, because the
    /// facade is owed an answer of its own to speak.
    #[allow(clippy::too_many_arguments)]
    async fn start(
        self: &Arc<Self>,
        chat: &Arc<ChatState>,
        library: &LibraryState,
        key: &str,
        text: &str,
        request_id: Option<String>,
        token: Option<&str>,
        attachments: Vec<String>,
        edit: bool,
        spoken: Option<Spoken>,
    ) -> Result<serde_json::Value, Refusal> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err(Refusal::Bad("empty message".into()));
        }
        if request_id.as_ref().is_some_and(|id| id.len() > 128) {
            return Err(Refusal::Bad("request id too long".into()));
        }
        let request_id = request_id.unwrap_or_else(Session::new_id);
        let name = self.persona_of(library, key, token).await?;
        // An open chat stops answering when its persona stops being approved,
        // as `open` and `resume` refuse one — not only after a restart.
        let live = Store::load(&self.store).get(&name).cloned();
        let approved = live
            .as_ref()
            .is_some_and(|p| p.state.status == mecha_core::persona::Status::Approved);
        // The safety switches as the persona stands now, not as the chat
        // pinned it: switching a protection back on reaches an open chat
        // (review of #418), as revoking approval already does. The prompt
        // stays the pinned version's.
        let live_switches = live.as_ref().map(|p| p.settings.safety);
        if !approved {
            return Err(Refusal::Conflict(format!(
                "`{name}` is not approved — `mecha persona approve {name}` after reading it"
            )));
        }
        let (pinned, notices, early_pause, wants_files, remembered, already) = {
            let mut sessions = self.sessions.lock().await;
            let ps = sessions.get_mut(key).ok_or(Refusal::NotFound)?;
            if ps.live.is_some() {
                if spoken.is_some() {
                    return Err(Refusal::Busy);
                }
                let switches = live_switches.unwrap_or(ps.pinned.settings.safety);
                let bound = chat.follower.current();
                return self
                    .steer_or_pause(chat, key, ps, &name, switches, &bound, text, request_id);
            }
            // A pause needs no model: decided here, from the owner's words,
            // it skips the router hold and the agent below, so 988 is not
            // queued behind a model load it never uses (review of #418).
            let switches = live_switches.unwrap_or(ps.pinned.settings.safety);
            let early_pause = switches.crisis
                && safety::keyword_hit(&text)
                && !ps.crisis_paused_at.is_some_and(|t| {
                    t.elapsed() < chat.follower.current().config.personas.crisis_cooldown()
                });
            // The persona's files ride before the chat's first reply (§10.4),
            // listed here and read outside the lock.
            let wants_files = ps
                .conversation
                .as_ref()
                .is_some_and(|c| carries_files_now(&c.messages));
            // What it remembers at chat start (§9.7): material, stored once
            // in the chat's first turn (§5.1), so a chat that carries it needs
            // no read at all.
            let carried = ps
                .conversation
                .as_ref()
                .is_some_and(|c| mecha_core::persona::recall::carries_chat_start(&c.messages));
            let remembered = if carried {
                Some(None)
            } else {
                ps.memory.clone()
            };
            // After the first reply, each turn also recalls what the owner's
            // message brings to mind (§9.7). What the conversation already
            // holds is not recalled again, so its text is read here; the
            // chat-start memory is added to it below, once it is known.
            let already = ps
                .conversation
                .as_ref()
                .filter(|c| {
                    c.messages
                        .iter()
                        .any(|m| m.role == mecha_core::message::Role::Assistant)
                })
                .map(|c| {
                    c.messages
                        .iter()
                        .map(|m| m.text())
                        .collect::<Vec<_>>()
                        .join("\n")
                });
            (
                Arc::clone(&ps.pinned),
                ps.events.clone(),
                early_pause,
                wants_files,
                remembered,
                already,
            )
        };
        if chat.stopping.is_cancelled() {
            return Err(Refusal::Failed("server is shutting down".into()));
        }
        // The router is held for the turn, as the assistant's are (D13): a
        // switch waits for it, and the page is told when a turn waits on one.
        let (held, ready) = if early_pause {
            (None, None)
        } else {
            let (held, bound) = chat
                .follower
                .enter("persona chat", |switch| {
                    let _ = notices.send(WireEvent::Notice {
                        text: format!(
                            "Switching the model to {} — this turn starts once it is loaded.",
                            switch.to
                        ),
                    });
                })
                .await
                .map_err(failed)?;
            let (agent, _) = self.agent_for(&bound, &pinned).map_err(failed)?;
            (held, Some((bound, agent)))
        };
        // Read before the sessions lock the turn takes below, and only for a
        // model that can see: to a blind one the pixels would render as a
        // placeholder every turn, and the path is already in the text.
        let named = attachments
            .iter()
            .filter(|p| mecha_core::message::image_media_type(Path::new(p)).is_some())
            .count();
        let sees = ready.as_ref().is_some_and(|(_, agent)| agent.vision());
        let images = if sees && named > 0 {
            let workspace = self
                .sessions
                .lock()
                .await
                .get(key)
                .map(|ps| ps.workspace.clone());
            match workspace {
                Some(workspace) => tokio::task::spawn_blocking(move || {
                    chat::attached_images(&workspace, &attachments)
                })
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!("attachments not read: {e}");
                    Vec::new()
                }),
                None => Vec::new(),
            }
        } else {
            Vec::new()
        };
        let shown = images.len();
        let files_block = match (&ready, wants_files) {
            (Some((bound, _)), true) => self.files_block(&name, bound).await,
            _ => None,
        };
        // What the persona remembers, read off its store outside the lock,
        // with the memory switches as the persona stands at that read: on a
        // chat's first turn, or the first turn of one begun under run notes
        // that never stored it. Stored once in the owner's turn below, where
        // it is cached with the history. As a note at the end of every
        // request it cost ~1.1–1.6 s of prefill each time and, as the most
        // salient text in the request, outweighed the owner's own asks
        // (PERSONA-CONTEXT-DESIGN.md §5.1, measured 2026-10-06).
        let remembered = match (remembered, &ready, &live) {
            (Some(kept), _, _) => Some(kept),
            (None, Some((bound, _)), Some(persona)) => Some(
                self.memory_block(persona.clone(), bound.config.agent.timezone(), &notices)
                    .await,
            ),
            _ => None,
        };
        let memory_block = remembered.clone().flatten();
        // And, past the first reply, what this message brings to mind.
        let recall_block = match (&ready, already, live) {
            (Some((bound, _)), Some(mut already), Some(persona)) => {
                if let Some(memory) = &memory_block {
                    already.push('\n');
                    already.push_str(memory);
                }
                self.recall_block(persona, bound, &text, already).await
            }
            _ => None,
        };

        let mut sessions = self.sessions.lock().await;
        // Under the lock `stop` takes, so a turn is either cancelled by it or
        // refused here.
        if chat.stopping.is_cancelled() {
            return Err(Refusal::Failed("server is shutting down".into()));
        }
        let ps = sessions.get_mut(key).ok_or(Refusal::NotFound)?;
        if ps.live.is_some() {
            if spoken.is_some() {
                return Err(Refusal::Busy);
            }
            let switches = live_switches.unwrap_or(ps.pinned.settings.safety);
            let bound = chat.follower.current();
            return self.steer_or_pause(chat, key, ps, &name, switches, &bound, text, request_id);
        }
        // A turn on a binding the transcript has not named says so in its
        // own record, ahead of the turn it governs: `serve/chat.rs`'s rule for
        // the assistant's chats, which persona chats lacked — so a chat the
        // router moved mid-way stayed credited to its first model, and the
        // writer waited for that model for good (2026-10-03). Before the
        // conversation is taken, so this append's own failure leaves the chat
        // as it was; and before the owner's message, so the record covers the
        // turn it governs. A turn the crisis layer then pauses is still
        // recorded under it: the message ran on no model, but is the chat's.
        if let Some((bound, agent)) = &ready {
            if ps.recorded_generation != Some(bound.generation) {
                // What this persona agent ran under, not the install's
                // assistant (review of #530): its own levers; a read-only
                // approver whatever `[tools] permission_mode` says
                // (`setup::persona_agent`); and no learned rules rendered,
                // which is an answer, not unknown.
                let mut recorded = RunConfig::of(
                    agent,
                    &bound.config,
                    &bound.provider_name,
                    &mecha_core::persona::agent::levers_off(&bound.levers_off, agent.config()),
                    Some(&mecha_core::learning::RulesCarried::none()),
                );
                recorded.permission_mode = mecha_core::config::PermissionMode::ReadOnly;
                ps.session
                    .append(&Record::Config(recorded))
                    .map_err(|e| Refusal::Failed(format!("recording: {e:#}")))?;
                ps.recorded_generation = Some(bound.generation);
            }
        }
        let Some(mut conversation) = ps.conversation.take() else {
            if spoken.is_some() {
                return Err(Refusal::Busy);
            }
            return Err(Refusal::Conflict("a turn is still finishing".into()));
        };
        // Kept for the chat's life in this process, once read.
        if ps.memory.is_none() {
            ps.memory = remembered;
        }
        // Asked again with the conversation in hand: a turn that finished in
        // between may have stored it already (the files block's rule).
        // Stored only before the chat's first reply, so it lands in
        // `messages[0]`, which compaction keeps whole (the files block's rule,
        // `recall::carries_chat_start`). A chat already past a reply that never
        // stored one (begun under #572, or a first turn whose read failed)
        // gets it as a note instead: slower, but never folded mid-history for
        // a compaction to summarise away and the next turn to store again.
        let (memory_block, memory_note) = match memory_block
            .filter(|_| !mecha_core::persona::recall::carries_chat_start(&conversation.messages))
        {
            Some(memory)
                if !conversation
                    .messages
                    .iter()
                    .any(|m| m.role == mecha_core::message::Role::Assistant) =>
            {
                (Some(memory), None)
            }
            Some(memory) => (None, Some(memory)),
            None => (None, None),
        };
        // The call note (§11): on every spoken turn, never on a typed one. It
        // speaks of pictures only to a persona that can make them (§5.6).
        let pictures = ready
            .as_ref()
            .is_some_and(|(_, agent)| agent.registry().get("image_generate").is_some());
        let call_note = spoken
            .as_ref()
            .map(|s| mecha_core::persona::call::note(s.streams, pictures));
        // What the persona keeps opening and closing with, named back to it in
        // the harness's voice (`persona::variety`, measured 2026-10-04).
        let variety_note = mecha_core::persona::variety::note(&conversation.messages);
        // A turn the picture edit panel sent: answered in a line, not by
        // retelling a picture the persona has not seen (`persona::edit`,
        // measured 2026-10-05). Typed only; a spoken turn has the call note.
        let edit_note = (edit && spoken.is_none()).then(mecha_core::persona::edit::note);
        // Asked again with the conversation in hand: `wants_files` was read
        // under the first lock, two awaits ago, and a turn that finished in
        // between may have carried the files already (review of #459).
        let files_block = files_block.filter(|_| carries_files_now(&conversation.messages));
        // The session goal rides in the first turn, never the system prompt,
        // so setting one costs the persona's cached prefix nothing (§6).
        let goal = ps.goal.take();
        let said = match &goal {
            Some(goal) if conversation.messages.is_empty() => {
                format!("(What I want from this conversation: {goal})\n\n{text}")
            }
            _ => text.clone(),
        };
        // Spent only once the first turn carries it.
        if !conversation.messages.is_empty() {
            ps.goal = goal.clone();
        }
        let spent_goal = if conversation.messages.is_empty() {
            goal.clone()
        } else {
            None
        };
        // The safety layer (§12), decided on the owner's own words before
        // anything runs: a crisis hit outside the cooldown pauses the persona
        // (the owner's ruling, 2026-09-29) — the message is recorded, the
        // persona does not answer it, and a plain voice does.
        let switches = live_switches.unwrap_or(pinned.settings.safety);
        let crisis_state = safety::crisis_state(switches.crisis, ps.judge_answered == Some(true));
        // `said`, not `text`: a goal typed at open rides in the first turn,
        // and is the owner's words as much as the message is.
        let hit = switches.crisis && safety::keyword_hit(&said);
        let cooling = ps.crisis_paused_at.is_some_and(|t| {
            t.elapsed() < chat.follower.current().config.personas.crisis_cooldown()
        });
        // An early decision binds: it skipped the agent this turn would need.
        let pause = hit && (!cooling || early_pause);
        // The Core, handed back near the newest turn on a cadence, after a
        // compaction, and when a chat is picked back up (§12.5).
        let anchor = (!pause
            && switches.reanchor
            && (ps.anchor_due || ps.turns_since_anchor + 1 >= safety::REANCHOR_EVERY))
            .then(|| safety::reanchor_text(&pinned.identity))
            .flatten();
        // Folded, not pushed, when the tail is already the owner's: a turn
        // cancelled after a tool ran ends on the user message carrying the
        // results, and a second user message in a row is a request no
        // provider accepts — the chat could never be continued (found on
        // review of #409; `chat::begin_turn` folds for the same reason). The
        // fold is recorded as a `Rewrite`, or the file would hold the
        // invalid shape and a resume would replay it.
        // The run's notes (PERSONA-CONTEXT-DESIGN.md §5.1): short, per-run
        // guidance, sent with every request of this run and stored in no
        // message, so none of them is re-sent on the turns after. The recall
        // is its own note, never joined to anything: it arms taint by the
        // stem it opens with (`recall::stem_of`), and joined after a clean
        // block a recall from outside would hide its own.
        let anchored = anchor.is_some();
        let notes: Vec<String> = [
            memory_note,
            recall_block,
            anchor,
            call_note,
            variety_note,
            edit_note,
        ]
        .into_iter()
        .flatten()
        .collect();
        // Recorded with the turn, and ahead of it: the notes carry what used
        // to ride inside the owner's message (the memory, the Core), and the
        // transcript is how a torn taint record is re-derived, so a notes line
        // that will not write refuses the turn as a message line that will not
        // write does. Ahead, so a refusal leaves the file without the turn,
        // as the rollback assumes: an orphaned notes line pushes no taint
        // checkpoint and only over-arms (`Session::read`), where an orphaned
        // owner turn would be replayed on resume. Not for a turn the crisis
        // layer pauses: nothing reaches the model.
        let record_notes = |session: &Session| -> anyhow::Result<()> {
            if notes.is_empty() || pause {
                return Ok(());
            }
            session.append(&Record::Notes {
                notes: notes.clone(),
            })
        };
        let recorded = if conversation
            .messages
            .last()
            .is_some_and(|m| m.role == mecha_core::message::Role::User)
        {
            let pre_fold = conversation.messages.clone();
            mecha_core::agent::append_user_text(&mut conversation.messages, said.clone());
            if let Some(last) = conversation.messages.last_mut() {
                last.content.extend(images);
            }
            // Folded too: a first turn that died before any reply leaves the
            // owner's message as the tail, and the files must still ride in
            // `messages[0]` (review of #459). Past a reply they never ride
            // (`carries_files_now`).
            if let Some(files) = &files_block {
                mecha_core::agent::append_user_text(&mut conversation.messages, files.clone());
            }
            if let Some(memory) = &memory_block {
                mecha_core::agent::append_user_text(&mut conversation.messages, memory.clone());
            }
            record_notes(&ps.session)
                .and_then(|()| {
                    ps.session.append(&Record::Rewrite {
                        messages: conversation.messages.clone(),
                    })
                })
                .map_err(|e| (e, Some(pre_fold)))
        } else {
            let mut user = Message::user(&said);
            user.content.extend(images);
            // The files, in the harness's voice like the anchor below
            // (`FILES_STEM` is registered): never the owner's words, and
            // armed untrusted and private by `Taint::arm_for_content`.
            if let Some(files) = &files_block {
                user.content.push(mecha_core::message::Block::Text {
                    text: files.clone(),
                });
            }
            // What it remembers at chat start, stored once like the files: the
            // stem it opens with is what arms the chat (§9.7).
            if let Some(memory) = &memory_block {
                user.content.push(mecha_core::message::Block::Text {
                    text: memory.clone(),
                });
            }
            conversation.push(user.clone());
            record_notes(&ps.session)
                .and_then(|()| ps.session.append(&Record::Message(user)))
                .map_err(|e| (e, None))
        };
        // Refuse a turn the record did not accept, as the assistant's do —
        // and give the goal back with it, so the retry still carries it.
        if let Err((e, pre_fold)) = recorded {
            match pre_fold {
                Some(messages) => conversation.messages = messages,
                None => {
                    conversation.messages.pop();
                }
            }
            ps.conversation = Some(conversation);
            ps.goal = goal;
            return Err(Refusal::Failed(format!("recording: {e:#}")));
        }
        let said_for_judge = said.clone();
        let _ = ps.events.send(WireEvent::User {
            text: said,
            spoken: spoken.is_some(),
            request_id: Some(request_id),
        });
        // The meters: one record per owner turn, never the words (§12.3).
        if switches.dose {
            if let Err(e) =
                safety::record_dose(&self.store, &name, &ps.session.meta.id, crisis_state)
            {
                tracing::warn!("a persona dose record was not written: {e:#}");
            }
        }
        if hit {
            if let Err(e) = safety::record_crisis(&self.store, "web", safety::Tier::Keyword, pause)
            {
                tracing::warn!("a crisis record was not written: {e:#}");
            }
        }
        if pause {
            ps.crisis_paused_at = Some(std::time::Instant::now());
            // No run starts to arm it, so armed here: a picture the paused
            // turn carried is private now, not at the next run — and
            // checkpointed, or a resume would read the chat clean (review
            // of #438). Only a hit in the goal reaches this with pixels; one
            // in the message pauses early, before any are read.
            let was = conversation.taint;
            conversation.taint.arm_for_content(&conversation.messages);
            if conversation.taint != was {
                if let Err(e) = ps.session.append(&Record::Taint(conversation.taint)) {
                    tracing::warn!("a paused persona chat's taint was not recorded: {e:#}");
                }
            }
            let taint = conversation.taint;
            ps.conversation = Some(conversation);
            ps.crisis_shown = true;
            let _ = ps.events.send(WireEvent::Crisis {
                text: safety::SAFE_MESSAGE.to_string(),
            });
            let _ = ps.events.send(WireEvent::Done {
                ok: true,
                stop: Some("CrisisPause".into()),
                taint_private: taint.private,
                taint_untrusted: taint.untrusted,
                error: None,
            });
            // On a call the pause is heard, not only shown: the plain words
            // go out as the call's text — a streaming call speaks only what
            // arrives as `TextDelta`, never `done`'s text (review of #483) —
            // and the persona says nothing.
            if let Some(spoken) = spoken {
                let _ = spoken
                    .tap
                    .send(AgentEvent::TextDelta(safety::SAFE_MESSAGE.to_string()));
                drop(spoken.tap);
                let _ = spoken.done.send(Ok(spoken_answer(safety::SAFE_MESSAGE)));
            }
            return Ok(serde_json::json!({ "started": false, "paused": true }));
        }
        let Some((bound, agent)) = ready else {
            // Unreachable: an early pause always pauses above.
            ps.conversation = Some(conversation);
            return Err(Refusal::Failed("a paused turn reached the model".into()));
        };
        // The crisis judge, alongside the turn, on words the keywords passed
        // (§12.2): a concern while the persona is still answering stops the
        // run and pauses it, as a keyword hit does. Not inside the cooldown,
        // which a pause would not break anyway.
        let judge_job: Option<JudgeJob> = (switches.crisis && !hit && !cooling).then(|| {
            (self.provider)(&bound, PersonaUse::Judge)
                .map(|p| (p, bound.model.clone(), said_for_judge.clone()))
                .map_err(|e| format!("the judge could not be reached: {e:#}"))
        });
        if anchored {
            ps.turns_since_anchor = 0;
            ps.anchor_due = false;
        } else {
            ps.turns_since_anchor += 1;
        }
        let before: Arc<[Message]> = conversation.messages.clone().into();

        // A spoken turn's handle is the facade's, made before the turn was
        // asked for, so a hang-up stops exactly this run.
        let (cancel, hosted) = match spoken {
            Some(Spoken {
                tap, done, cancel, ..
            }) => (cancel, Some((tap, done))),
            None => (mecha_core::agent::CancelHandle::new(), None),
        };
        let (tap, hosted_done) = hosted.unzip();
        let spoken_turn = tap.is_some();
        // A second sender for what the harness says after the run: the
        // judge's pause, spoken behind a reply it cut off. Holding it is
        // load-bearing: `voice::pump` streams until every sender is gone, so
        // this clone, held in the run task past the forwarder's end, is what
        // keeps the facade's stream open until the words are sent. Drop it
        // earlier and the pause goes unspoken while every test here, which
        // reads the channel directly, stays green.
        let after_tap = tap.clone();
        let queue: Arc<StdMutex<VecDeque<String>>> = Arc::default();
        let queued_ids: Arc<StdMutex<VecDeque<String>>> = Arc::default();
        let working: Arc<StdMutex<Vec<serde_json::Value>>> = Arc::default();
        let mut history_taint = conversation.taint;
        history_taint.arm_for_content(&before);
        history_taint.arm_for_notes(&notes);
        ps.live = Some(Live {
            cancel: cancel.clone(),
            queue: Arc::clone(&queue),
            queued_ids: Arc::clone(&queued_ids),
            history: Arc::clone(&before),
            taint: history_taint,
            working: Arc::clone(&working),
        });
        if let Some(h) = &held {
            let c = cancel.clone();
            h.on_cancel(move || c.cancel(mecha_core::agent::CancelReason::Stopped));
        }

        // The persona agent's own context, jailed to this chat's workspace.
        // No brief, homeostat, outbox or hooks: each is the owner's.
        let mut cx = (**agent.context()).clone();
        cx.tools = Arc::new(agent.ctx().for_session(ps.workspace.clone()));
        if cx.budget.max_turns.is_none() {
            cx.budget.max_turns = Some(40);
        }
        let judge_cancel = cancel.clone();
        cx = cx.with_cancel_handle(cancel);
        cx.queued_input = Some(queue);
        cx.notes = notes.into();
        // Someone is waiting in silence on a spoken turn (SPOKEN_THINK_BUDGET).
        if spoken_turn {
            cx = cx.with_think_budget(SPOKEN_THINK_BUDGET);
        }

        let session = Arc::clone(&ps.session);
        let unconsumed = Arc::clone(&queued_ids);
        let bcast = ps.events.clone();
        let last_usage = Arc::clone(&ps.last_usage);
        let context_window = bound.context_window;
        let chats = Arc::clone(self);
        let key = key.to_string();
        let stopping = chat.stopping.clone();
        // Spawned under the sessions lock, as the assistant's `begin_turn`
        // spawns under its map: `stop` takes this lock before it closes
        // `runs`, so the run is on the tracker before `drain` can report
        // quiescence (found on review of #409).
        chat.runs.spawn(async move {
            let _held = held;
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let forwarder = {
                let bcast = bcast.clone();
                tokio::spawn(async move {
                    while let Some(event) = rx.recv().await {
                        track_working(&working, &event);
                        if let Some(tap) = &tap {
                            let _ = tap.send(event.clone());
                        }
                        if let AgentEvent::QueuedInput(_) = &event {
                            if let Some(request_id) =
                                queued_ids.lock().ok().and_then(|mut ids| ids.pop_front())
                            {
                                let _ = bcast.send(WireEvent::QueuedDelivered { request_id });
                            }
                        }
                        if let AgentEvent::TurnUsage(usage) = &event {
                            if let Ok(mut slot) = last_usage.lock() {
                                *slot = Some(usage.clone());
                            }
                        }
                        if let Some(wire) = chat::wire_event(&event, context_window) {
                            let _ = bcast.send(wire);
                        }
                    }
                })
            };
            // The judge races the run: a concern that lands first stops it.
            let (mut judge_handle, mut verdict) = match judge_job {
                Some(Ok((provider, model, words))) => (
                    Some(tokio::spawn(async move {
                        judge::screen(provider.as_ref(), &model, &words).await
                    })),
                    None,
                ),
                Some(Err(why)) => (None, Some(judge::Verdict::Unchecked(why))),
                None => (None, None),
            };
            let mut stopped_by_judge = false;
            let outcome = {
                let run = agent.run_in(&cx, &mut conversation, Some(tx));
                tokio::pin!(run);
                match judge_handle.as_mut() {
                    Some(handle) => tokio::select! {
                        out = &mut run => out,
                        joined = handle => {
                            let v = joined.unwrap_or_else(|e| {
                                judge::Verdict::Unchecked(format!("the judge task failed: {e}"))
                            });
                            if v == judge::Verdict::Concern {
                                judge_cancel.cancel(mecha_core::agent::CancelReason::Stopped);
                                stopped_by_judge = true;
                            }
                            verdict = Some(v);
                            judge_handle = None;
                            run.await
                        }
                    },
                    None => run.await,
                }
            };
            let _ = forwarder.await;
            match &outcome {
                Ok(o) => {
                    let _ = session.record_run(&before, &conversation);
                    let _ = session.record_outcome(o);
                }
                // The record agrees with the rollback, or a resume replays
                // the failed turn (the assistant's rule, `chat::begin_turn`).
                Err(_) => {
                    conversation.roll_back_failed_turn(before.to_vec());
                    if let Err(e) = session.record_run(&before, &conversation) {
                        tracing::warn!("a persona chat's rollback was not recorded: {e:#}");
                    }
                }
            }
            let _ = session.append(&Record::Taint(conversation.taint));
            // `compactions`, not `context_overflows`: the question is whether
            // history above the tail was rewritten — only `compact()` does
            // that, and it counts `compactions` on both of its paths.
            let compacted = matches!(&outcome, Ok(o) if o.compactions > 0);
            // A failed run was rolled back, anchor block and all: the Core
            // was not delivered, so the next turn still owes it.
            let owed_anchor = anchored && outcome.is_err();
            let taint = conversation.taint;
            let run_ok = outcome.is_ok();
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
            // Hand the conversation back, then announce the end.
            let mut sessions = chats.sessions.lock().await;
            if let Some(ps) = sessions.get_mut(&key) {
                // A steer that arrived after the run's last read of its queue
                // was neither delivered nor discarded: say so, as the
                // assistant's chats do, or the page's bubble never resolves
                // (found on review of #409). Under the lock `send` steers
                // under, so none can arrive between this and `live = None`.
                let discarded: Vec<String> = unconsumed
                    .lock()
                    .map(|mut ids| ids.drain(..).collect())
                    .unwrap_or_default();
                if !discarded.is_empty() {
                    let _ = bcast.send(WireEvent::Notice {
                        text: format!(
                            "{} queued message(s) arrived too late for this turn — send again",
                            discarded.len()
                        ),
                    });
                    let _ = bcast.send(WireEvent::QueuedDiscarded {
                        request_ids: discarded,
                    });
                }
                // A first turn that failed was rolled back to nothing; the
                // goal it carried goes back with it, so the retry has it.
                if conversation.messages.is_empty() && ps.goal.is_none() {
                    ps.goal = spent_goal;
                }
                ps.conversation = Some(conversation);
                ps.live = None;
                // A compaction may have summarised the Core away (§12.5).
                if compacted || owed_anchor {
                    ps.anchor_due = true;
                }
                // A crisis message typed while this run was live: recorded
                // now, beside the run it stopped, so the words are kept.
                if let Some(said) = ps.pending_crisis.take() {
                    if let Some(convo) = ps.conversation.as_mut() {
                        let tail_is_owner = convo
                            .messages
                            .last()
                            .is_some_and(|m| m.role == mecha_core::message::Role::User);
                        let recorded = if tail_is_owner {
                            mecha_core::agent::append_user_text(&mut convo.messages, said);
                            session.append(&Record::Rewrite {
                                messages: convo.messages.clone(),
                            })
                        } else {
                            let user = Message::user(&said);
                            convo.push(user.clone());
                            session.append(&Record::Message(user))
                        };
                        if let Err(e) = recorded {
                            tracing::warn!("a paused crisis message was not recorded: {e:#}");
                        }
                    }
                }
            }
            drop(sessions);
            let _ = bcast.send(done);
            // The call's answer, once the conversation is back — a caller
            // told "answered" before it would find the chat held on its very
            // next word (`chat::begin_turn`'s order). A run the judge stopped
            // is answered with the pause's plain words, as a typed one shows
            // them, never with the half-reply it was cut off in.
            if let Some(after) = after_tap {
                // Whatever the run ended in: a judge stop that the run then
                // errored on still owes the call the plain words.
                if stopped_by_judge {
                    // Streaming cannot un-say what was already spoken, so
                    // the plain words follow it.
                    let _ = after.send(AgentEvent::TextDelta(format!(
                        " {}",
                        safety::SAFE_MESSAGE
                    )));
                }
            }
            if let Some(done) = hosted_done {
                let _ = done.send(match &outcome {
                    _ if stopped_by_judge => Ok(spoken_answer(safety::SAFE_MESSAGE)),
                    Ok(o) => Ok(crate::voice::HostedAnswer {
                        text: o.text.clone(),
                        input_tokens: o.usage.input_tokens,
                        output_tokens: o.usage.output_tokens,
                        affect: None,
                    }),
                    Err(e) => Err(format!("{e:#}")),
                });
            }
            // The citations, checked once the reply is handed back — a long
            // chat's input is not held for them (review of #465). They land
            // a moment after `Done`, and the page replaces its checks.
            if run_ok {
                let checks = citations(&session).await;
                if !checks.is_empty() {
                    let _ = bcast.send(WireEvent::Citations { checks });
                }
            }
            // A verdict still out when the run ended: waited for here, after
            // the reply is handed back — bounded, and a judge that does not
            // answer in time is "couldn't check", never clear.
            if let Some(handle) = judge_handle {
                // The reply is handed back: the router need not stay held for
                // the wait, and a shutdown need not wait out the judge.
                drop(_held);
                verdict = Some(tokio::select! {
                    joined = tokio::time::timeout(JUDGE_WAIT, handle) => match joined {
                        Ok(Ok(v)) => v,
                        Ok(Err(e)) => judge::Verdict::Unchecked(format!("the judge task failed: {e}")),
                        Err(_) => judge::Verdict::Unchecked("the judge did not answer in time".into()),
                    },
                    _ = stopping.cancelled() => judge::Verdict::Unchecked("the server is shutting down".into()),
                });
            }
            if let Some(v) = verdict {
                chats.apply_verdict(&key, v, stopped_by_judge, None).await;
            }
        });
        drop(sessions);
        // How many pictures the text names and the persona was not shown, for
        // the page to say — it cleared the chips on send (as `chat::send`).
        Ok(serde_json::json!({
            "started": true,
            "pictures_not_shown": named.saturating_sub(shown),
            "model_sees": sees,
        }))
    }
}

impl PersonaChats {
    /// Act on the crisis judge's verdict for a chat (§12.2). A concern is
    /// counted (tier `judge`), arms the cooldown and sends the warning; with
    /// `cancel_live` it also stops the run in flight (a steer's). A verdict
    /// the judge could not reach is "couldn't check": the chat says crisis
    /// detection is on keywords only until the judge answers again — the
    /// change is announced both ways, never silent.
    async fn apply_verdict(
        &self,
        key: &str,
        verdict: judge::Verdict,
        paused: bool,
        steer: Option<(String, String)>,
    ) {
        let mut sessions = self.sessions.lock().await;
        let Some(ps) = sessions.get_mut(key) else {
            // Closed before the verdict came: a concern is still counted —
            // the counter is what a host's report is built from.
            if verdict == judge::Verdict::Concern {
                tracing::warn!("a crisis judge concern arrived for a closed persona chat");
                if let Err(e) =
                    safety::record_crisis(&self.store, "web", safety::Tier::Judge, false)
                {
                    tracing::warn!("a crisis record was not written: {e:#}");
                }
            }
            return;
        };
        match verdict {
            judge::Verdict::Concern => {
                let mut paused = paused;
                // A steer the judge stopped: its run is cancelled, and its
                // words — still in the queue unless the run already took them
                // — are held for the hand-back to record, as a keyword pause
                // holds them, rather than coming back "not delivered"
                // (review of #426).
                if let (Some((text, request_id)), Some(live)) = (steer, &ps.live) {
                    let held = match (live.queue.lock(), live.queued_ids.lock()) {
                        (Ok(mut queue), Ok(mut ids)) => {
                            take_undrained(&mut queue, &mut ids, &request_id)
                        }
                        _ => false,
                    };
                    live.cancel.cancel(mecha_core::agent::CancelReason::Stopped);
                    paused = true;
                    if held {
                        ps.pending_crisis = Some(match ps.pending_crisis.take() {
                            Some(before) => format!("{before}\n\n{text}"),
                            None => text,
                        });
                        // Its receipt: kept, and it reaches the persona with
                        // the owner's next turn — or the page's bubble reads
                        // "queued" forever (review of #426).
                        let _ = ps.events.send(WireEvent::QueuedDelivered { request_id });
                    }
                }
                if let Err(e) =
                    safety::record_crisis(&self.store, "web", safety::Tier::Judge, paused)
                {
                    tracing::warn!("a crisis record was not written: {e:#}");
                }
                // The cooldown means "a pause just happened": a late concern
                // paused nothing, and must not disarm both tiers for 15
                // minutes (review of #426).
                if paused {
                    ps.crisis_paused_at = Some(std::time::Instant::now());
                }
                // Recovered, and said so, as the `Clear` arm does: the doc
                // promises the change is announced both ways (review of #426).
                if ps.judge_answered == Some(false) {
                    let _ = ps.events.send(WireEvent::Notice {
                        text: "Crisis detection's model check is answering again.".into(),
                    });
                }
                ps.judge_answered = Some(true);
                ps.crisis_shown = true;
                let _ = ps.events.send(WireEvent::Crisis {
                    text: safety::SAFE_MESSAGE.to_string(),
                });
            }
            judge::Verdict::Clear => {
                if ps.judge_answered == Some(false) {
                    let _ = ps.events.send(WireEvent::Notice {
                        text: "Crisis detection's model check is answering again.".into(),
                    });
                }
                ps.judge_answered = Some(true);
            }
            judge::Verdict::Unchecked(why) => {
                if ps.judge_answered != Some(false) {
                    let _ = ps.events.send(WireEvent::Notice {
                        text: format!(
                            "Crisis detection is on keywords only for now: the model check \
                             could not answer ({why})."
                        ),
                    });
                }
                ps.judge_answered = Some(false);
            }
        }
    }

    /// A message sent while a run is live. The safety layer reads it as it
    /// reads any other (found on review of #418: the steer path skipped it,
    /// so a crisis message typed mid-answer reached the persona in
    /// character): it is metered, and a crisis hit outside the cooldown stops
    /// the run and pauses the persona — the message is kept, the plain voice
    /// answers. Anything else steers, as before.
    #[allow(clippy::too_many_arguments)]
    fn steer_or_pause(
        self: &Arc<Self>,
        chat: &Arc<ChatState>,
        key: &str,
        ps: &mut PersonaSession,
        name: &str,
        switches: mecha_core::persona::Safety,
        bound: &crate::follow::Bound,
        text: String,
        request_id: String,
    ) -> Result<serde_json::Value, Refusal> {
        if switches.dose {
            if let Err(e) = safety::record_dose(
                &self.store,
                name,
                &ps.session.meta.id,
                safety::crisis_state(switches.crisis, ps.judge_answered == Some(true)),
            ) {
                tracing::warn!("a persona dose record was not written: {e:#}");
            }
        }
        if switches.crisis && safety::keyword_hit(&text) {
            let cooling = ps
                .crisis_paused_at
                .is_some_and(|t| t.elapsed() < bound.config.personas.crisis_cooldown());
            if let Err(e) =
                safety::record_crisis(&self.store, "web", safety::Tier::Keyword, !cooling)
            {
                tracing::warn!("a crisis record was not written: {e:#}");
            }
            if !cooling {
                ps.crisis_paused_at = Some(std::time::Instant::now());
                ps.pending_crisis = Some(text.clone());
                if let Some(live) = &ps.live {
                    live.cancel.cancel(mecha_core::agent::CancelReason::Stopped);
                }
                let _ = ps.events.send(WireEvent::User {
                    text,
                    spoken: false,
                    request_id: Some(request_id),
                });
                ps.crisis_shown = true;
                let _ = ps.events.send(WireEvent::Crisis {
                    text: safety::SAFE_MESSAGE.to_string(),
                });
                return Ok(serde_json::json!({ "steered": false, "paused": true }));
            }
        }
        // The judge reads a steer too, alongside the run it joins: a concern
        // the keywords missed stops that run and pauses the persona.
        let cooling = ps
            .crisis_paused_at
            .is_some_and(|t| t.elapsed() < bound.config.personas.crisis_cooldown());
        if switches.crisis && !cooling {
            let chats = Arc::clone(self);
            let key = key.to_string();
            let words = text.clone();
            // Its words and receipt, so a concern can hold them back.
            let steered = Some((text.clone(), request_id.clone()));
            match (self.provider)(bound, PersonaUse::Judge) {
                Ok(provider) => {
                    let model = bound.model.clone();
                    let stopping = chat.stopping.clone();
                    // Bounded and stoppable, as the turn's judge is: a judge
                    // that hangs must not hold `drain` at shutdown (#409's
                    // rule) or wait forever (review of #426).
                    chat.runs.spawn(async move {
                        let v = tokio::select! {
                            judged = tokio::time::timeout(
                                JUDGE_WAIT,
                                judge::screen(provider.as_ref(), &model, &words),
                            ) => judged.unwrap_or_else(|_| {
                                judge::Verdict::Unchecked("the judge did not answer in time".into())
                            }),
                            _ = stopping.cancelled() => {
                                judge::Verdict::Unchecked("the server is shutting down".into())
                            }
                        };
                        chats.apply_verdict(&key, v, false, steered).await;
                    });
                }
                Err(e) => {
                    let v =
                        judge::Verdict::Unchecked(format!("the judge could not be reached: {e:#}"));
                    chat.runs
                        .spawn(async move { chats.apply_verdict(&key, v, false, steered).await });
                }
            }
        }
        steer(ps, text, request_id)
    }
}

/// Fold text into the run in flight, as the assistant's chats steer.
fn steer(
    ps: &PersonaSession,
    text: String,
    request_id: String,
) -> Result<serde_json::Value, Refusal> {
    let live = ps
        .live
        .as_ref()
        .ok_or_else(|| Refusal::Conflict("no run to steer".into()))?;
    let (Ok(mut queue), Ok(mut ids)) = (live.queue.lock(), live.queued_ids.lock()) else {
        return Err(Refusal::Failed("steering queue poisoned".into()));
    };
    ids.push_back(request_id.clone());
    queue.push_back(text.clone());
    let _ = ps.events.send(WireEvent::Queued {
        text,
        request_id: Some(request_id),
    });
    Ok(serde_json::json!({ "steered": true }))
}

// ─── Routes ────────────────────────────────────────────────────────────────

type Web = State<super::WebState>;

/// Take a steer back out of what the run has not read yet, by its receipt.
/// True only when its words were really removed before the agent saw them.
///
/// The two deques are pushed together under both locks, but drained apart:
/// the agent empties `queue` wholesale (`take_queued_input`) and the
/// forwarder then pops `ids` one event at a time. So, with both locks held,
/// the steers still in `queue` are the *last* `queue.len()` entries of `ids`
/// — the ones before them were already folded into the request, and their
/// words reached the persona (review of #426: an index into both, as if they
/// were aligned, "held" words the agent had already read).
fn take_undrained(
    queue: &mut VecDeque<String>,
    ids: &mut VecDeque<String>,
    request_id: &str,
) -> bool {
    let Some(offset) = ids.len().checked_sub(queue.len()) else {
        return false;
    };
    match ids.iter().position(|id| id == request_id) {
        Some(at) if at >= offset => {
            ids.remove(at);
            queue.remove(at - offset);
            true
        }
        _ => false,
    }
}

#[derive(serde::Deserialize)]
pub struct UnlockQuery {
    #[serde(default)]
    unlock: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct OpenBody {
    #[serde(default)]
    unlock: Option<String>,
    #[serde(default)]
    goal: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct ResumeBody {
    id: String,
    #[serde(default)]
    unlock: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct SendBody {
    text: String,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    unlock: Option<String>,
    /// Workspace-relative paths the page uploaded for this turn, already
    /// named in `text` (`PersonaChats::send_with`).
    #[serde(default)]
    attachments: Vec<String>,
    /// The picture edit panel sent this turn (`persona::edit`).
    #[serde(default)]
    edit: bool,
}

#[derive(serde::Deserialize)]
pub struct CreateBody {
    name: String,
    #[serde(default)]
    display: Option<String>,
    #[serde(default)]
    relationships: Vec<String>,
    #[serde(default)]
    character: Option<String>,
    #[serde(default)]
    groups: Vec<String>,
    #[serde(default)]
    locked: bool,
}

/// The approved characters the viewer may see: every one when the library is
/// unlocked, the unlocked ones otherwise.
fn visible_characters(lib: &mecha_core::imagelib::Library, unlocked: bool) -> Vec<&str> {
    lib.approved()
        .filter(|e| e.kind == mecha_core::imagelib::Kind::Character)
        .filter(|e| !e.locked || unlocked)
        .map(|e| e.name.as_str())
        .collect()
}

#[derive(serde::Deserialize)]
pub struct SaveBody {
    file: mecha_core::persona::OwnerFile,
    /// The whole file, verbatim — or `changes`, a form's `{path: value}`
    /// set in place (`tomlform`), or `doc`; exactly one.
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    changes: Option<serde_json::Map<String, serde_json::Value>>,
    /// A Markdown file's form (`mdform::Doc`), written in its canonical
    /// layout.
    #[serde(default)]
    doc: Option<mecha_core::mdform::Doc>,
    /// The digest of the file as the page opened it — required.
    base: String,
    #[serde(default)]
    unlock: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct RelationshipBody {
    name: String,
    text: String,
}

#[derive(serde::Deserialize)]
pub struct GroupBody {
    name: String,
    #[serde(default)]
    description: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct LockBody {
    locked: bool,
    #[serde(default)]
    unlock: Option<String>,
}

/// A proposal still waiting: a candidate a model's proposal wrote
/// (`State::proposed`), as the page's Waiting section counts one. Not
/// "anything not the owner's": a persona folder made by hand with no
/// `state.toml`, or one that does not parse, loads as an untrusted
/// candidate too, and must never be offered a Reject that moves its folder
/// aside (review of #493). Those keep `mecha persona approve` and `remove`.
fn waiting_proposal(p: &Persona) -> Result<(), Refusal> {
    use mecha_core::imagelib::Status;
    if p.state.status != Status::Candidate || p.state.proposed.is_none() {
        return Err(Refusal::Conflict(format!(
            "`{}` is not a proposal waiting for approval",
            p.name
        )));
    }
    Ok(())
}

/// GET /api/personas/{name}/review
pub async fn review(
    State(state): Web,
    axum::extract::Path(name): axum::extract::Path<String>,
    Query(q): Query<UnlockQuery>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let personas = Arc::clone(&chat.personas);
    let library = Arc::clone(&state.library);
    match tokio::task::spawn_blocking(move || personas.review(&library, &name, q.unlock.as_deref()))
        .await
    {
        Ok(result) => respond(result),
        Err(e) => Refusal::Failed(format!("reading the proposal: {e}")).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct ApproveBody {
    /// This server's signature of what the page showed (`review`'s `shown`).
    shown: String,
    /// The linked character's, to approve it in the same tap.
    #[serde(default)]
    character_shown: Option<String>,
    #[serde(default)]
    unlock: Option<String>,
}

/// POST /api/personas/{name}/approve
pub async fn approve(
    State(state): Web,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(body): Json<ApproveBody>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let personas = Arc::clone(&chat.personas);
    let library = Arc::clone(&state.library);
    match tokio::task::spawn_blocking(move || {
        personas.approve(
            &library,
            &name,
            &body.shown,
            body.character_shown.as_deref(),
            body.unlock.as_deref(),
        )
    })
    .await
    {
        Ok(result) => respond(result),
        Err(e) => Refusal::Failed(format!("approving: {e}")).into_response(),
    }
}

/// POST /api/personas/{name}/reject
pub async fn reject(
    State(state): Web,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(body): Json<UnlockBody>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let personas = Arc::clone(&chat.personas);
    let library = Arc::clone(&state.library);
    match tokio::task::spawn_blocking(move || {
        personas.reject(&library, &name, body.unlock.as_deref())
    })
    .await
    {
        Ok(result) => respond(result),
        Err(e) => Refusal::Failed(format!("rejecting: {e}")).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct FrameBody {
    /// Absent or null returns the portrait to the page's default framing.
    #[serde(default)]
    frame: Option<mecha_core::persona::Frame>,
    #[serde(default)]
    unlock: Option<String>,
}

/// POST /api/personas/{name}/frame
pub async fn frame(
    State(state): Web,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(body): Json<FrameBody>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    respond(
        chat.personas
            .frame(&state.library, &name, body.frame, body.unlock.as_deref()),
    )
}

#[derive(serde::Deserialize, Default)]
pub struct UnlockBody {
    #[serde(default)]
    unlock: Option<String>,
}

fn respond(result: Result<serde_json::Value, Refusal>) -> axum::response::Response {
    match result {
        Ok(v) => Json(v).into_response(),
        Err(r) => r.into_response(),
    }
}

/// GET /api/personas
pub async fn list(State(state): Web, Query(q): Query<UnlockQuery>) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    // Off the async threads, as `library::list` is: it walks two stores.
    let personas = Arc::clone(&chat.personas);
    let library = Arc::clone(&state.library);
    let tz = chat.follower.current().config.agent.timezone();
    match tokio::task::spawn_blocking(move || personas.list(&library, q.unlock.as_deref(), tz))
        .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => Refusal::Failed(format!("listing personas: {e}")).into_response(),
    }
}

/// The name of a persona's linked character when the lock hides it from
/// this viewer. Fails closed: on a locked page the name is shown only for an
/// entry that loads *and* is unlocked — `Library::load` drops a damaged
/// entry, and "could not read it" must not read as "not locked" (#430's
/// review found the same shape there).
fn hidden_character<'a>(
    p: &'a Persona,
    lib: &mecha_core::imagelib::Library,
    unlocked: bool,
) -> Option<&'a str> {
    if unlocked {
        return None;
    }
    let c = p.settings.character.as_deref()?;
    match lib.get(mecha_core::imagelib::Kind::Character, c) {
        Some(e) if !e.locked => None,
        _ => Some(c),
    }
}

/// Problems as a viewer may read them: one that names a character the lock
/// hides is left out, not reworded — a locked candidate's "names character
/// `x`, which is a candidate" would say what the nulled `character` does not
/// (review of #425). The owner reads it again once unlocked.
fn unnamed(problems: Vec<String>, hidden: Option<&str>) -> Vec<String> {
    match hidden {
        Some(c) => {
            let quoted = format!("`{c}`");
            problems
                .into_iter()
                .filter(|s| !s.contains(&quoted))
                .collect()
        }
        None => problems,
    }
}

/// GET /api/personas/authoring
pub async fn authoring(
    State(state): Web,
    Query(q): Query<UnlockQuery>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    respond(chat.personas.authoring(&state.library, q.unlock.as_deref()))
}

/// POST /api/personas
pub async fn create(State(state): Web, Json(body): Json<CreateBody>) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    respond(chat.personas.create(&state.library, body))
}

/// POST /api/personas/relationships
pub async fn add_relationship(
    State(state): Web,
    Json(body): Json<RelationshipBody>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    respond(chat.personas.add_relationship(body))
}

/// POST /api/personas/groups
pub async fn add_group(State(state): Web, Json(body): Json<GroupBody>) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    respond(chat.personas.add_group(body))
}

/// GET /api/personas/{name}/files
pub async fn files(
    State(state): Web,
    axum::extract::Path(name): axum::extract::Path<String>,
    Query(q): Query<UnlockQuery>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let voices = form_voices(&state, Some(OPEN_WAIT)).await;
    respond(
        chat.personas
            .files_in(&state.library, &name, q.unlock.as_deref(), &voices),
    )
}

/// POST /api/personas/{name}/files
pub async fn save(
    State(state): Web,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(body): Json<SaveBody>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    // Only a voice being picked is checked against the list (`tomlform`
    // checks the paths a save sends), so no other save asks the worker. One
    // that does waits as long as the worker does: a TTS still loading is
    // slow, not voiceless, and the owner is waiting on this save.
    let voices = match &body.changes {
        Some(changes)
            if body.file == mecha_core::persona::OwnerFile::Settings && picks_voice(changes) =>
        {
            form_voices(&state, None).await
        }
        _ => Default::default(),
    };
    respond(chat.personas.save_in(&state.library, &name, body, &voices))
}

/// Does this form save set a voice? Leaving it out or clearing it (`null`,
/// the worker's own voice) needs no list.
fn picks_voice(changes: &serde_json::Map<String, serde_json::Value>) -> bool {
    changes.get("voice").is_some_and(|v| !v.is_null())
}

/// How long opening the editor waits for the voice list. Shorter than the
/// worker's own wait, because the editor is one page of many fields: a slow
/// worker costs the voice choice there, and a save that picks one waits in
/// full.
const OPEN_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// The voices the settings form offers: the worker's list
/// (`runner_voices`, as Library → Voices reads it), or why it could not be
/// had. `wait` caps it below the worker's own wait; `None` is the worker's.
async fn form_voices(
    state: &super::WebState,
    wait: Option<std::time::Duration>,
) -> mecha_core::persona::VoiceChoices {
    let listed = match (&state.offer_target, wait) {
        (None, _) => Err("voice calls are not wired on this serve"),
        (Some(target), None) => super::runner_voices(target).await,
        (Some(target), Some(wait)) => tokio::time::timeout(wait, super::runner_voices(target))
            .await
            .unwrap_or(Err("the voice worker did not answer in time")),
    };
    match listed {
        Ok(names) => mecha_core::persona::VoiceChoices {
            names,
            unread: None,
        },
        Err(why) => mecha_core::persona::VoiceChoices {
            names: Vec::new(),
            unread: Some(why.to_string()),
        },
    }
}

/// POST /api/personas/{name}/lock
pub async fn lock(
    State(state): Web,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(body): Json<LockBody>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    respond(
        chat.personas
            .lock(&state.library, &name, body.locked, body.unlock.as_deref()),
    )
}

/// POST /api/personas/{name}/chats
pub async fn open(
    State(state): Web,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(body): Json<OpenBody>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c.clone(),
        Err(resp) => return resp,
    };
    respond(
        chat.personas
            .open(
                &chat,
                &state.library,
                &name,
                body.unlock.as_deref(),
                body.goal,
            )
            .await,
    )
}

/// One act on a persona's memory, from its curation page: a closed set, so
/// the page can ask for nothing the CLI's owner door does not also offer.
#[derive(serde::Deserialize, Debug)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum MemoryAct {
    Approve {
        id: String,
    },
    Pin {
        id: String,
    },
    Unpin {
        id: String,
    },
    Correct {
        id: String,
        text: String,
    },
    Forget {
        id: String,
    },
    Share {
        id: String,
        /// A group's name; absent is everyone.
        #[serde(default)]
        group: Option<String>,
    },
    Unshare {
        id: String,
    },
}

#[derive(serde::Deserialize)]
pub struct MemoryBody {
    #[serde(flatten)]
    act: MemoryAct,
    #[serde(default)]
    unlock: Option<String>,
}

/// GET /api/personas/{name}/memory
pub async fn memory(
    State(state): Web,
    axum::extract::Path(name): axum::extract::Path<String>,
    Query(q): Query<UnlockQuery>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let personas = Arc::clone(&chat.personas);
    let library = Arc::clone(&state.library);
    let tz = chat.follower.current().config.agent.timezone();
    match tokio::task::spawn_blocking(move || {
        personas.memory(&library, &name, q.unlock.as_deref(), tz)
    })
    .await
    {
        Ok(result) => respond(result),
        Err(e) => Refusal::Failed(format!("reading a persona's memory: {e}")).into_response(),
    }
}

/// POST /api/personas/{name}/memory
pub async fn memory_act(
    State(state): Web,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(body): Json<MemoryBody>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let personas = Arc::clone(&chat.personas);
    let library = Arc::clone(&state.library);
    let tz = chat.follower.current().config.agent.timezone();
    match tokio::task::spawn_blocking(move || {
        personas.memory_act(&library, &name, body.act, body.unlock.as_deref(), tz)
    })
    .await
    {
        Ok(result) => respond(result),
        Err(e) => Refusal::Failed(format!("changing a persona's memory: {e}")).into_response(),
    }
}

/// GET /api/personas/{name}/chats
pub async fn history(
    State(state): Web,
    axum::extract::Path(name): axum::extract::Path<String>,
    Query(q): Query<UnlockQuery>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    // Off the async threads, as `list` is: it walks the transcripts.
    let personas = Arc::clone(&chat.personas);
    let library = Arc::clone(&state.library);
    match tokio::task::spawn_blocking(move || {
        personas.history(&library, &name, q.unlock.as_deref())
    })
    .await
    {
        Ok(result) => respond(result),
        Err(e) => Refusal::Failed(format!("listing chats: {e}")).into_response(),
    }
}

/// POST /api/personas/{name}/resume
pub async fn resume(
    State(state): Web,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(body): Json<ResumeBody>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c.clone(),
        Err(resp) => return resp,
    };
    respond(
        chat.personas
            .resume(
                &chat,
                &state.library,
                &name,
                &body.id,
                body.unlock.as_deref(),
            )
            .await,
    )
}

/// GET /api/persona-chat/{key}
pub async fn transcript(
    State(state): Web,
    axum::extract::Path(key): axum::extract::Path<String>,
    Query(q): Query<UnlockQuery>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c.clone(),
        Err(resp) => return resp,
    };
    respond(
        chat.personas
            .transcript(&chat, &state.library, &key, q.unlock.as_deref())
            .await,
    )
}

/// GET /api/persona-chat/{key}/events
pub async fn events(
    State(state): Web,
    axum::extract::Path(key): axum::extract::Path<String>,
    Query(q): Query<UnlockQuery>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c.clone(),
        Err(resp) => return resp,
    };
    match chat
        .personas
        .subscribe(&state.library, &key, q.unlock.as_deref())
        .await
    {
        // A locked persona's stream is let through by the token, so it ends
        // with the token — a relock from any page, or the idle expiry — at
        // the next event, rather than outliving the lock that let it in
        // (found on review of #409). An in-memory check, so it costs nothing
        // per streamed delta; checking refreshes the idle clock, which a chat
        // being watched is fairly said to be using.
        Ok((rx, true)) => {
            let library = Arc::clone(&state.library);
            let token = q.unlock.clone();
            chat::sse_while(rx, chat.stopping.clone(), move || {
                library.unlocked(token.as_deref())
            })
        }
        Ok((rx, false)) => chat::sse(rx, chat.stopping.clone()),
        Err(r) => r.into_response(),
    }
}

/// POST /api/persona-chat/{key}/send
pub async fn send(
    State(state): Web,
    axum::extract::Path(key): axum::extract::Path<String>,
    Json(body): Json<SendBody>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c.clone(),
        Err(resp) => return resp,
    };
    let personas = Arc::clone(&chat.personas);
    respond(
        personas
            .send_with(
                &chat,
                &state.library,
                &key,
                &body.text,
                body.request_id,
                body.unlock.as_deref(),
                body.attachments,
                body.edit,
            )
            .await,
    )
}

/// POST /api/persona-chat/{key}/cancel
pub async fn cancel(
    State(state): Web,
    axum::extract::Path(key): axum::extract::Path<String>,
    body: Option<Json<UnlockBody>>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c.clone(),
        Err(resp) => return resp,
    };
    let unlock = body.and_then(|Json(b)| b.unlock);
    match chat
        .personas
        .cancel(&state.library, &key, unlock.as_deref())
        .await
    {
        Ok(cancelled) => Json(serde_json::json!({ "cancelled": cancelled })).into_response(),
        Err(r) => r.into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct FileQuery {
    path: String,
    #[serde(default)]
    unlock: Option<String>,
}

/// GET /api/persona-chat/{key}/file?path= — a picture out of this chat's
/// workspace, with the assistant chat's containment and content types
/// (`files::serve`). The persona chat's own door: an assistant key is
/// refused, and the assistant's `/api/chat/{key}/file` never finds a
/// persona chat, whose session map is this one.
pub async fn download(
    State(state): Web,
    axum::extract::Path(key): axum::extract::Path<String>,
    Query(q): Query<FileQuery>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c.clone(),
        Err(resp) => return resp,
    };
    match chat
        .personas
        .workspace_of(&state.library, &key, q.unlock.as_deref())
        .await
    {
        Ok(ws) => super::files::serve(ws, q.path).await,
        Err(r) => r.into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct SourceFileQuery {
    file: String,
    #[serde(default)]
    unlock: Option<String>,
}

/// GET /api/personas/{name}/sources/file?file= — one of its files, to keep:
/// an attachment of inert bytes, as every download out of mecha is.
pub async fn source_file(
    State(state): Web,
    axum::extract::Path(name): axum::extract::Path<String>,
    Query(q): Query<SourceFileQuery>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c.clone(),
        Err(resp) => return resp,
    };
    match chat
        .personas
        .source_file(&state.library, &name, &q.file, q.unlock.as_deref())
        .await
    {
        Ok(src) => super::files::attachment(src.path, &src.name).await,
        Err(r) => r.into_response(),
    }
}

/// GET /api/personas/{name}/sources/text?file= — one of its files as text,
/// once it has been read.
pub async fn source_text(
    State(state): Web,
    axum::extract::Path(name): axum::extract::Path<String>,
    Query(q): Query<SourceFileQuery>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c.clone(),
        Err(resp) => return resp,
    };
    respond(
        chat.personas
            .source_text(&chat, &state.library, &name, &q.file, q.unlock.as_deref())
            .await,
    )
}

#[derive(serde::Deserialize)]
pub struct CallBody {
    seconds: u32,
    /// The binding the offer's answer named (`call`).
    #[serde(default)]
    call: Option<u64>,
    #[serde(default)]
    unlock: Option<String>,
}

/// POST /api/persona-chat/{key}/call — a call has ended, and how long it was.
pub async fn call_ended(
    State(state): Web,
    axum::extract::Path(key): axum::extract::Path<String>,
    Json(body): Json<CallBody>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c.clone(),
        Err(resp) => return resp,
    };
    match chat
        .personas
        .call_ended(
            &state.library,
            &key,
            body.unlock.as_deref(),
            body.seconds,
            body.call,
        )
        .await
    {
        Ok(()) => Json(serde_json::json!({ "counted": true })).into_response(),
        Err(r) => r.into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct SaveReplyBody {
    text: String,
    #[serde(default)]
    unlock: Option<String>,
}

/// POST /api/persona-chat/{key}/save — one of this chat's replies into the
/// persona's own files (`PersonaChats::save_reply`).
pub async fn save_reply(
    State(state): Web,
    axum::extract::Path(key): axum::extract::Path<String>,
    Json(body): Json<SaveReplyBody>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c.clone(),
        Err(resp) => return resp,
    };
    respond(
        chat.personas
            .save_reply(
                &chat,
                &state.library,
                &key,
                &body.text,
                body.unlock.as_deref(),
            )
            .await,
    )
}

#[derive(serde::Deserialize)]
pub struct CitedQuery {
    file: String,
    #[serde(default)]
    page: Option<u32>,
    #[serde(default)]
    quote: String,
    #[serde(default)]
    unlock: Option<String>,
}

/// GET /api/persona-chat/{key}/cited?file=&page=&quote= — a cited page as
/// the chat received it, the quote marked (`PersonaChats::cited`).
pub async fn cited(
    State(state): Web,
    axum::extract::Path(key): axum::extract::Path<String>,
    Query(q): Query<CitedQuery>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c.clone(),
        Err(resp) => return resp,
    };
    respond(
        chat.personas
            .cited(
                &state.library,
                &key,
                &q.file,
                q.page,
                &q.quote,
                q.unlock.as_deref(),
            )
            .await,
    )
}

#[derive(serde::Deserialize)]
pub struct UploadQuery {
    name: String,
    #[serde(default)]
    unlock: Option<String>,
}

/// POST /api/persona-chat/{key}/upload?name= — the edit modal's mask into
/// this chat's `inbox/` (`files::store`). Named in the message, never
/// attached: a mask is for `image_generate`, not for the persona to look at.
pub async fn upload(
    State(state): Web,
    axum::extract::Path(key): axum::extract::Path<String>,
    Query(q): Query<UploadQuery>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c.clone(),
        Err(resp) => return resp,
    };
    match chat
        .personas
        .workspace_of(&state.library, &key, q.unlock.as_deref())
        .await
    {
        Ok(ws) => super::files::store(ws, &q.name, body).await,
        Err(r) => r.into_response(),
    }
}

/// GET /api/personas/{name}/sources — the files it can read.
pub async fn sources(
    State(state): Web,
    axum::extract::Path(name): axum::extract::Path<String>,
    Query(q): Query<UnlockQuery>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c.clone(),
        Err(resp) => return resp,
    };
    respond(
        chat.personas
            .sources(&chat, &state.library, &name, q.unlock.as_deref())
            .await,
    )
}

/// POST /api/personas/{name}/sources?name= — a file into its own folder.
pub async fn add_source(
    State(state): Web,
    axum::extract::Path(name): axum::extract::Path<String>,
    Query(q): Query<UploadQuery>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c.clone(),
        Err(resp) => return resp,
    };
    respond(
        chat.personas
            .add_source(
                &chat,
                &state.library,
                &name,
                &q.name,
                q.unlock.as_deref(),
                body,
            )
            .await,
    )
}

#[derive(serde::Deserialize)]
pub struct RemoveSourceBody {
    file: String,
    #[serde(default)]
    unlock: Option<String>,
}

/// POST /api/personas/{name}/sources/remove — one of its own files out of reach.
pub async fn remove_source(
    State(state): Web,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(body): Json<RemoveSourceBody>,
) -> axum::response::Response {
    let chat = match chat::chat_state(&state) {
        Ok(c) => c.clone(),
        Err(resp) => return resp,
    };
    respond(
        chat.personas
            .remove_source(&state.library, &name, &body.file, body.unlock.as_deref())
            .await,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::message::{Block, CompletionRequest, CompletionResponse, StopReason};
    use mecha_core::persona::{self as store, NewPersona, Origin};

    /// How the test provider behaves once it has kept a copy of a request.
    #[derive(Clone)]
    enum Mode {
        Answer,
        /// Never answers, as a model mid-generation.
        Hang,
        /// Answers once the gate is opened.
        Gate(Arc<tokio::sync::Notify>),
        Fail,
        /// Answers with this text.
        Say(String),
        /// Answers after reasoning this plan, as a thinking model does.
        Think(String),
    }

    /// What the crisis judge answers in a test world.
    #[derive(Clone)]
    enum JudgeSays {
        Clear,
        Concern,
        /// Refuses: "couldn't check".
        Refuse,
        /// Answers `Concern` once the gate opens.
        ConcernAfter(Arc<tokio::sync::Notify>),
    }

    /// A provider that keeps a copy of every persona request and then does
    /// what its mode says. A crisis judge's request (its quarantined system
    /// prompt) is answered as the world's `JudgeSays` and counted apart.
    struct Capture(
        Arc<StdMutex<Vec<CompletionRequest>>>,
        Mode,
        Arc<StdMutex<JudgeSays>>,
        Arc<StdMutex<usize>>,
        /// Whether the model can see — off unless a test turns it on.
        Arc<std::sync::atomic::AtomicBool>,
    );

    #[async_trait::async_trait]
    impl mecha_core::provider::Provider for Capture {
        fn id(&self) -> &str {
            "local"
        }
        fn default_model(&self) -> &str {
            "test"
        }
        fn vision(&self) -> bool {
            self.4.load(std::sync::atomic::Ordering::SeqCst)
        }
        async fn complete(
            &self,
            req: &CompletionRequest,
            _: Option<&mecha_core::provider::StreamSink>,
        ) -> Result<CompletionResponse> {
            if req
                .system
                .as_deref()
                .is_some_and(|s| s.starts_with("You screen one message"))
            {
                *self.3.lock().unwrap() += 1;
                // Room past the router's reasoning budget (review of #426).
                assert!(
                    req.max_tokens >= mecha_core::provider::LOCAL_MAX_TOKENS,
                    "the judge asked for {} tokens",
                    req.max_tokens
                );
                let says = self.2.lock().unwrap().clone();
                let (text, stop) = match says {
                    JudgeSays::Clear => (
                        r#"{"wish_to_be_dead":false,"suicidal_thoughts":false,"method":false,"intent":false,"preparation":false,"self_harm":false}"#,
                        StopReason::EndTurn,
                    ),
                    JudgeSays::Concern => (
                        r#"{"wish_to_be_dead":true,"suicidal_thoughts":false,"method":false,"intent":false,"preparation":false,"self_harm":false}"#,
                        StopReason::EndTurn,
                    ),
                    JudgeSays::Refuse => ("I can't help with that.", StopReason::Refusal),
                    JudgeSays::ConcernAfter(gate) => {
                        gate.notified().await;
                        (
                            r#"{"wish_to_be_dead":true,"suicidal_thoughts":false,"method":false,"intent":false,"preparation":false,"self_harm":false}"#,
                            StopReason::EndTurn,
                        )
                    }
                };
                return Ok(CompletionResponse {
                    message: Message::assistant(vec![Block::Text { text: text.into() }]),
                    stop_reason: stop,
                    usage: Usage::default(),
                    refusal: None,
                    model: "test".into(),
                    malformed_tool_args: 0,
                });
            }
            self.0.lock().unwrap().push(req.clone());
            match &self.1 {
                Mode::Answer => {}
                Mode::Hang => std::future::pending::<()>().await,
                Mode::Gate(open) => open.notified().await,
                Mode::Fail => anyhow::bail!("the model is not loaded"),
                Mode::Say(_) | Mode::Think(_) => {}
            }
            let text = match &self.1 {
                Mode::Say(text) => text.clone(),
                _ => "Hello from Mara.".into(),
            };
            let mut content = vec![Block::Text { text }];
            if let Mode::Think(plan) = &self.1 {
                content.insert(
                    0,
                    Block::Thinking {
                        text: plan.clone(),
                        signature: None,
                    },
                );
            }
            Ok(CompletionResponse {
                message: Message::assistant(content),
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
                refusal: None,
                model: "test".into(),
                malformed_tool_args: 0,
            })
        }
    }

    struct World {
        root: PathBuf,
        chat: Arc<ChatState>,
        library: LibraryState,
        seen: Arc<StdMutex<Vec<CompletionRequest>>>,
        /// What the assistant's agent was asked: the chat's follower, which
        /// a Listen tap's director runs on.
        assistant_seen: Arc<StdMutex<Vec<CompletionRequest>>>,
        sees: Arc<std::sync::atomic::AtomicBool>,
        judge: Arc<StdMutex<JudgeSays>>,
        judged: Arc<StdMutex<usize>>,
    }

    impl Drop for World {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    impl World {
        fn store(&self) -> PathBuf {
            self.root.join("personas")
        }
        fn personas(&self) -> Arc<PersonaChats> {
            Arc::clone(&self.chat.personas)
        }
    }

    /// A persona `mara` who asks for a tool she may not have (`fs_read`),
    /// one she may (`image_view`), and one this install lacks
    /// (`web_search`), beside an assistant whose prompt and tools are
    /// marked so a leak into her chat is visible.
    fn world() -> World {
        world_with(Mode::Answer)
    }

    fn world_with(mode: Mode) -> World {
        world_tuned(mode, |_| {})
    }

    /// A world whose config the test adjusts — a document reader, say.
    fn world_tuned(mode: Mode, tune: impl FnOnce(&mut mecha_core::config::Config)) -> World {
        world_built(mode, tune, false)
    }

    /// A stand-in `image_generate`: there to be in a persona's registry, for
    /// what the harness says to a persona that can draw (§5.6).
    struct DrawStub;

    #[async_trait::async_trait]
    impl mecha_core::tool::Tool for DrawStub {
        fn name(&self) -> &str {
            "image_generate"
        }
        fn description(&self) -> &str {
            "stub"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        fn for_persona(self: Arc<Self>) -> Option<Arc<dyn mecha_core::tool::Tool>> {
            Some(self)
        }
        async fn call(
            &self,
            _: serde_json::Value,
            _: &mecha_core::tool::ToolCtx,
        ) -> anyhow::Result<mecha_core::tool::ToolOutput> {
            Ok(mecha_core::tool::ToolOutput::ok("drawn"))
        }
    }

    /// `world_tuned`, and with `draws` the persona has `image_generate`.
    fn world_built(
        mode: Mode,
        tune: impl FnOnce(&mut mecha_core::config::Config),
        draws: bool,
    ) -> World {
        let root = std::env::temp_dir().join(format!("mecha-pchat-{}", uuid::Uuid::new_v4()));
        let dir = root.join("personas");
        let lib = mecha_core::imagelib::Library::load(&root.join("imagelib")).0;
        store::create(
            &dir,
            &lib,
            NewPersona {
                name: "mara".into(),
                display: "Mara".into(),
                relationships: vec!["colleague".into()],
                origin: Origin::Owner,
                ..NewPersona::default()
            },
        )
        .unwrap();
        let toml = dir.join("mara/persona.toml");
        let text = std::fs::read_to_string(&toml).unwrap().replace(
            "allow = [\"web_search\"]",
            if draws {
                "allow = [\"fs_read\", \"image_generate\", \"image_view\", \"web_search\"]"
            } else {
                "allow = [\"fs_read\", \"image_view\", \"web_search\"]"
            },
        );
        std::fs::write(&toml, text).unwrap();
        let id = dir.join("mara/identity.md");
        let text = std::fs::read_to_string(&id).unwrap().replace(
            "## Core\n",
            "## Core\nA marine ecologist who distrusts easy answers.\n",
        );
        std::fs::write(&id, text).unwrap();

        let seen = Arc::new(StdMutex::new(Vec::new()));
        let for_persona = Arc::clone(&seen);
        let judge = Arc::new(StdMutex::new(JudgeSays::Clear));
        let judged = Arc::new(StdMutex::new(0usize));
        let (for_judge, for_judged) = (Arc::clone(&judge), Arc::clone(&judged));
        let sees = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let for_sees = Arc::clone(&sees);
        let personas = PersonaChats::with(
            dir,
            root.join("work"),
            Arc::new(move |_, _| {
                Ok(Box::new(Capture(
                    Arc::clone(&for_persona),
                    mode.clone(),
                    Arc::clone(&for_judge),
                    Arc::clone(&for_judged),
                    Arc::clone(&for_sees),
                ))
                    as Box<dyn mecha_core::provider::Provider>)
            }),
        );
        let mut pool = mecha_core::tool::Registry::new();
        pool.insert(Arc::new(mecha_core::tool::builtin::FsRead));
        pool.insert(Arc::new(mecha_core::tool::image_view::ImageView));
        if draws {
            pool.insert(Arc::new(DrawStub));
        }
        let mut config = mecha_core::config::Config::default();
        tune(&mut config);
        config.agent.system_prompt = Some("ASSISTANT-ONLY: the owner's charter".into());
        let assistant_seen = Arc::new(StdMutex::new(Vec::new()));
        let chat = chat::test_chat_built(
            Box::new(Capture(
                Arc::clone(&assistant_seen),
                Mode::Answer,
                Arc::new(StdMutex::new(JudgeSays::Clear)),
                Arc::new(StdMutex::new(0)),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
            )),
            pool,
            config,
            None,
            personas,
        );
        World {
            library: LibraryState::new(root.join("imagelib")),
            root,
            chat,
            seen,
            assistant_seen,
            sees,
            judge,
            judged,
        }
    }

    /// Run one turn and wait for it to finish.
    async fn turn(w: &World, key: &str, text: &str) {
        turn_as(w, key, text, false).await
    }

    /// `turn`, sent from the picture edit panel when `edit`.
    async fn turn_as(w: &World, key: &str, text: &str, edit: bool) {
        let (mut rx, _) = w.personas().subscribe(&w.library, key, None).await.unwrap();
        w.personas()
            .send_with(&w.chat, &w.library, key, text, None, None, Vec::new(), edit)
            .await
            .unwrap();
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await {
                Ok(Ok(WireEvent::Done { ok, error, .. })) => {
                    assert!(ok, "{error:?}");
                    return;
                }
                Ok(Ok(_)) => continue,
                other => panic!("no Done event: {other:?}"),
            }
        }
    }

    /// A persona chat says which model each stretch ran on: a `config` on its
    /// first turn in this process, none while the binding holds, and another
    /// when the router moves it — so the memory writer reads the model the
    /// chat last ran on, not the one it started on (2026-10-03: a persona chat
    /// moved to another model at 02:50Z mid-chat, and the writer would
    /// have waited for the base model for good).
    #[tokio::test]
    async fn a_persona_chat_records_the_model_it_runs_on_and_each_switch() {
        let w = world();
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        let file = |w: &World| {
            let dir = Store::load(&w.store()).sessions_dir("mara");
            let path = std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(|e| e.ok().map(|e| e.path()))
                .find(|p| p.extension().is_some_and(|x| x == "jsonl"))
                .unwrap();
            std::fs::read_to_string(path).unwrap()
        };
        let configs = |text: &str| {
            text.lines()
                .filter(|l| l.contains("\"record\":\"config\""))
                .count()
        };
        turn(&w, &key, "Hello there.").await;
        assert_eq!(configs(&file(&w)), 1, "the first turn names its model");
        // The levers the persona agent ran without, not the assistant's: no
        // persona prompt carries learned rules (review of #530).
        let text = file(&w);
        let recorded: serde_json::Value = text
            .lines()
            .find(|l| l.contains("\"record\":\"config\""))
            .map(|l| serde_json::from_str(l).unwrap())
            .unwrap();
        let off = recorded["levers_off"].to_string();
        assert!(off.contains("learned_rules"), "{off}");
        assert!(
            !w.chat
                .follower
                .current()
                .levers_off
                .contains(&mecha_core::harness::Lever::LearnedRules),
            "the assistant's own set must not already say it, or this proves nothing"
        );
        turn(&w, &key, "And again.").await;
        assert_eq!(configs(&file(&w)), 1, "the same binding names nothing new");

        // The router moved the chat: the binding the transcript last named
        // is not the one this turn runs on.
        w.personas()
            .sessions
            .lock()
            .await
            .get_mut(&key)
            .unwrap()
            .recorded_generation = Some(u64::MAX);
        turn(&w, &key, "Still there?").await;
        let text = file(&w);
        assert_eq!(configs(&text), 2, "a switch is recorded ahead of its turn");
        // The writer reads the model from the `config` this door wrote, not
        // the header: with the header naming another model, the record's
        // still wins.
        let bound = w.chat.follower.current();
        let header = format!("\"model\":\"{}\"", bound.model);
        assert!(
            text.lines().next().unwrap().contains(&header),
            "the header names it"
        );
        let started_elsewhere = text.replacen(&header, "\"model\":\"started-elsewhere\"", 1);
        let chat = mecha_core::persona::writer::read_chat(&started_elsewhere);
        assert_eq!(chat.model.as_deref(), Some(bound.model.as_str()));
        assert_eq!(recorded["permission_mode"], "read-only", "{recorded}");
    }

    /// The echo is read from transcripts (`persona::echo`), so what a persona
    /// chat records has to be enough: each reply, and an outcome saying the
    /// run completed. A first reply has nothing to compare; a second that
    /// repeats it reads 1.0.
    #[tokio::test]
    async fn a_persona_chats_transcript_is_enough_to_read_its_echo() {
        let w = world_with(Mode::Say(
            "The kelp line runs north of the second buoy today".into(),
        ));
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        let sessions = w.store().join("mara/sessions");
        let since = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        let read = || mecha_core::persona::echo::echoes(&sessions, since).unwrap();

        turn(&w, &key, "Where is the kelp?").await;
        assert_eq!(read().replies, 0, "a first reply has nothing to echo");

        turn(&w, &key, "mm").await;
        let e = read();
        assert_eq!(
            (e.chats, e.replies, e.repeated, e.max, e.unreadable),
            (1, 1, 1, Some(1.0), 0)
        );
    }

    /// A persona chat is built with `PriorThinking::Drop`: the second turn's
    /// request carries the first reply but not the reasoning behind it, while
    /// the transcript keeps both. Without `persona_agent`'s line, the plan
    /// rides back and the persona re-reads it (§12.7).
    #[tokio::test]
    async fn a_persona_sends_no_earlier_replys_reasoning() {
        let w = world_with(Mode::Think("PRIOR-PLAN".into()));
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        turn(&w, &key, "Hello, Mara").await;
        turn(&w, &key, "mm").await;

        let seen = w.seen.lock().unwrap().clone();
        let second = serde_json::to_string(&seen.last().unwrap().messages).unwrap();
        assert!(second.contains("Hello from Mara."), "the reply is sent");
        assert!(!second.contains("PRIOR-PLAN"), "its reasoning is not");
        let id = opened["session"].as_str().unwrap();
        let file = w.store().join(format!("mara/sessions/{id}.jsonl"));
        let recorded = std::fs::read_to_string(file).unwrap();
        assert!(recorded.contains("PRIOR-PLAN"), "the transcript keeps it");
    }

    /// A persona chat is built with `PriorTails::Trim`: a reply that stopped
    /// mid-sentence goes back cut to its last whole sentence, and the
    /// transcript keeps it as it was. Without `persona_agent`'s line the
    /// dangling word rides back and the persona learns to end on one.
    #[tokio::test]
    async fn a_persona_sends_an_earlier_reply_without_its_dangling_word() {
        let w = world_with(Mode::Say("Hello from Mara. Glad you came. \n\nI".into()));
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        turn(&w, &key, "Hello, Mara").await;
        turn(&w, &key, "mm").await;

        let seen = w.seen.lock().unwrap().clone();
        let earlier: Vec<String> = seen
            .last()
            .unwrap()
            .messages
            .iter()
            .filter(|m| m.role == mecha_core::message::Role::Assistant)
            .map(Message::text)
            .collect();
        assert_eq!(earlier, vec!["Hello from Mara. Glad you came.".to_string()]);
        let id = opened["session"].as_str().unwrap();
        let file = w.store().join(format!("mara/sessions/{id}.jsonl"));
        let recorded = std::fs::read_to_string(file).unwrap();
        assert!(
            recorded.contains(r"Glad you came. \n\nI"),
            "the transcript keeps the reply as written"
        );
    }

    /// A turn the edit panel sent carries the edit note in the harness's
    /// voice (`persona::edit`), never as the owner's words; a typed turn
    /// does not, and the next turn's request leaves the stale note out.
    #[tokio::test]
    async fn an_edit_panel_turn_is_told_to_answer_in_a_line() {
        let w = world_with(Mode::Think("they asked for an edit".into()));
        let key = open_chat(&w).await;
        let edit_notes = |m: &Message| {
            m.content
                .iter()
                .filter(|b| {
                    matches!(b, mecha_core::message::Block::Text { text }
                    if mecha_core::persona::edit::is_note(text))
                })
                .count()
        };
        turn(&w, &key, "hello").await;
        turn_as(&w, &key, "Edit images/a.png: make the sky pink", true).await;
        turn(&w, &key, "lovely").await;

        let seen = w.seen.lock().unwrap().clone();
        assert_eq!(
            edit_notes(seen[0].messages.last().unwrap()),
            0,
            "a typed turn"
        );
        let edited = seen[1].messages.last().unwrap();
        assert_eq!(edit_notes(edited), 1, "the edit turn carries the note");
        assert_eq!(
            mecha_core::agent::owner_text(edited),
            "Edit images/a.png: make the sky pink",
            "the note is the harness's, never the owner's words"
        );
        let on_wire: usize = seen[2].messages.iter().map(edit_notes).sum();
        assert_eq!(on_wire, 0, "a stale edit note went back to the model");
    }

    /// A persona that keeps opening and closing alike is told so, in the
    /// harness's voice beside the owner's words (`persona::variety`): never
    /// drawn as theirs, never their words to a miner.
    #[tokio::test]
    async fn a_persona_repeating_itself_is_told_what_it_repeats() {
        // Replies with reasoning, as the persona model gives them: a stale
        // note is dropped only ahead of a reply that goes out altered.
        let w = world_with(Mode::Think("they said hello again".into()));
        let key = open_chat(&w).await;
        turn(&w, &key, "hello").await;
        turn(&w, &key, "hi again").await;
        turn(&w, &key, "and again").await;

        let seen = w.seen.lock().unwrap().clone();
        // Per block, as `owner_text` reads one: `is_note` matches a whole block,
        // so a check on the joined text could never fail (review of #550).
        let notes_in = |m: &Message| {
            m.content
                .iter()
                .filter(|b| {
                    matches!(b, mecha_core::message::Block::Text { text }
                    if mecha_core::persona::variety::is_note(text))
                })
                .count()
        };
        assert_eq!(
            notes_in(seen[0].messages.last().unwrap()),
            0,
            "nothing to vary from on the first turn"
        );
        // Only the turn being answered carries one on the wire: the earlier
        // turns' notes stay in the transcript and out of what is sent.
        let third = &seen.last().unwrap().messages;
        let on_wire: usize = third.iter().map(notes_in).sum();
        assert_eq!(on_wire, 1, "stale notes went back to the model");
        let last = third.last().unwrap();
        let note = last
            .content
            .iter()
            .filter_map(|b| match b {
                mecha_core::message::Block::Text { text } => Some(text.clone()),
                _ => None,
            })
            .find(|t| mecha_core::persona::variety::is_note(t))
            .expect("the third turn carries the note");
        assert!(note.contains("opened with \"Hello\""), "{note}");
        assert!(note.contains("\"Hello from Mara.\""), "{note}");
        assert_eq!(
            mecha_core::agent::owner_text(last),
            "and again",
            "the note is the harness's, never the owner's words"
        );
    }

    #[tokio::test]
    async fn a_persona_turn_runs_on_its_own_prompt_and_tools_and_is_recorded_apart() {
        let w = world();
        let opened = w
            .personas()
            .open(
                &w.chat,
                &w.library,
                "mara",
                None,
                Some("Plan the kelp survey".into()),
            )
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        assert!(is_persona_key(&key));
        // What she asked for and did not get is said at open.
        let refused = opened["refused"].to_string();
        assert!(
            refused.contains("fs_read") && refused.contains("never"),
            "{refused}"
        );
        assert!(
            refused.contains("web_search") && refused.contains("not available"),
            "{refused}"
        );

        turn(&w, &key, "Hello, Mara").await;
        let seen = w.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        let req = &seen[0];
        let system = req.system.clone().unwrap_or_default();
        assert!(system.contains("# A persona chat"), "{system}");
        assert!(system.contains("A marine ecologist"), "{system}");
        assert!(
            !system.contains("ASSISTANT-ONLY"),
            "the assistant's prompt leaked:\n{system}"
        );
        let tools: Vec<&str> = req.tools.iter().map(|t| t.name.as_str()).collect();
        // `file_read` is every persona's (§10), whatever `[tools] allow` says;
        // the memory pair, every persona whose memory is on (§9.7).
        assert_eq!(
            tools,
            vec![
                "file_read",
                "file_search",
                "image_view",
                "memory_read",
                "memory_search"
            ]
        );
        // The goal rode in the first turn, not the system prompt.
        let first = req.messages[0].text();
        assert!(first.starts_with("(What I want from this conversation: Plan the kelp survey)"));
        assert!(!system.contains("kelp survey"));

        // Recorded under the persona, beside its pin — and nowhere else.
        let id = opened["session"].as_str().unwrap();
        let sessions = w.store().join("mara/sessions");
        assert!(sessions.join(format!("{id}.jsonl")).is_file());
        assert!(pin_path(&sessions, id).is_file());
        let listed = Session::list(&sessions).unwrap();
        assert_eq!(listed.len(), 1);
        let (_, convo) = Session::load(&listed[0].1).unwrap();
        assert_eq!(convo.messages.len(), 2, "the turn and its answer");

        let t = w
            .personas()
            .transcript(&w.chat, &w.library, &key, None)
            .await
            .unwrap();
        assert_eq!(t["persona"], "mara");
        assert_eq!(t["running"], false);
    }

    /// A persona's files ride in the first turn that runs the model (§10.4),
    /// once: the block is the harness's — never drawn in the owner's bubble —
    /// and the chat that carried it is untrusted (§10.6).
    #[tokio::test]
    async fn the_personas_files_ride_in_the_first_turn_once() {
        let w = world();
        std::fs::write(
            w.store().join("mara/files/urchins.md"),
            "Sea urchins graze kelp holdfasts.",
        )
        .unwrap();
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        turn(&w, &key, "What do urchins do?").await;
        turn(&w, &key, "And then?").await;

        let seen = w.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 2);
        let first = seen[0].messages[0].text();
        assert!(first.contains("What do urchins do?"), "{first}");
        assert!(
            first.contains(mecha_core::persona::files::FILES_STEM)
                && first.contains("Sea urchins graze kelp holdfasts."),
            "{first}"
        );
        // Once: the second turn's own message carries no second copy.
        let second = seen[1].messages.last().unwrap().text();
        assert!(second.contains("And then?"), "{second}");
        assert!(
            !second.contains(mecha_core::persona::files::FILES_STEM),
            "{second}"
        );

        // The owner's bubble is the owner's words; the chat is untrusted.
        let t = w
            .personas()
            .transcript(&w.chat, &w.library, &key, None)
            .await
            .unwrap();
        let bubbles: Vec<String> = t["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == "user")
            .map(|e| e["text"].as_str().unwrap_or("").to_string())
            .collect();
        assert_eq!(bubbles, ["What do urchins do?", "And then?"], "{t}");
        assert_eq!(t["taint"]["untrusted"], true, "{t}");
    }

    /// §9.7: what the persona remembers rides in a chat's first turn, once,
    /// in the harness's voice. Clean memory leaves the chat private and
    /// trusted — so the writer keeps forming ordinary memories from it — and
    /// a recalled record that came from outside makes the chat untrusted.
    #[tokio::test]
    async fn what_a_persona_remembers_rides_in_the_first_turn_with_its_taint() {
        use mecha_core::persona::memory::{Kind, Memory, NewFact, Source, Table};
        use mecha_core::persona::Origin;
        let w = world();
        let remember = |text: &str, origin: Origin| {
            let m = Memory::open(&w.store(), "mara").unwrap();
            let f = m
                .add_fact(
                    Table::User,
                    NewFact {
                        text: text.into(),
                        kind: Kind::Stated,
                        source: Source {
                            chat: "earlier".into(),
                            from: 0,
                            to: 1,
                        },
                        origin,
                        model: "m".into(),
                        valid_from: None,
                        valid_to: None,
                    },
                )
                .unwrap();
            if origin == Origin::ModelUntrusted {
                m.approve(&f.uid).unwrap();
            }
        };
        let chat = |said: &'static str| async {
            let opened = w
                .personas()
                .open(&w.chat, &w.library, "mara", None, None)
                .await
                .unwrap();
            let key = opened["key"].as_str().unwrap().to_string();
            turn(&w, &key, said).await;
            turn(&w, &key, "And then?").await;
            let t = w
                .personas()
                .transcript(&w.chat, &w.library, &key, None)
                .await
                .unwrap();
            t
        };

        remember(
            "Teaches a methods seminar on Thursdays.",
            Origin::ModelClean,
        );
        let t = chat("Hello again.").await;
        let seen = w.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 2);
        // Material (PERSONA-CONTEXT-DESIGN.md §5.1): stored once, in the
        // chat's first owner turn, and sent from there as history on every
        // later request, one copy, never re-sent at the tail as a note.
        for req in &seen {
            let first = req.messages[0].text();
            assert!(
                first.contains(mecha_core::persona::recall::MEMORY_STEM)
                    && first.contains("Teaches a methods seminar on Thursdays."),
                "{first}"
            );
            let copies: usize = req
                .messages
                .iter()
                .map(|m| m.text().matches("methods seminar").count())
                .sum();
            assert_eq!(copies, 1, "one copy a request, never a stack");
        }
        assert!(
            !seen[1]
                .messages
                .last()
                .unwrap()
                .text()
                .contains("methods seminar"),
            "not at the tail of a later request"
        );
        let bubbles: Vec<String> = t["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == "user")
            .map(|e| e["text"].as_str().unwrap_or("").to_string())
            .collect();
        assert_eq!(bubbles, ["Hello again.", "And then?"], "{t}");
        assert_eq!(t["taint"]["private"], true, "{t}");
        assert_eq!(
            t["taint"]["untrusted"], false,
            "clean memory stays trusted: {t}"
        );

        remember("Lives by the sea.", Origin::ModelUntrusted);
        let t = chat("Hi.").await;
        assert_eq!(t["taint"]["untrusted"], true, "{t}");
    }

    /// §5.1: a chat that stored its chat-start memory keeps it through a
    /// restart: the rebuilt conversation carries the block, so nothing is read
    /// again and nothing rides as a note — one copy, still in `messages[0]`
    /// (review of #575). If the rebuild ever lost it, the block would be read
    /// again and sent at the tail of every request, slower than before #572.
    #[tokio::test]
    async fn stored_memory_survives_a_restart_as_one_copy_in_the_first_turn() {
        use mecha_core::persona::memory::{Memory, NewEpisode, Source};
        use mecha_core::persona::Origin;
        let w = world();
        Memory::open(&w.store(), "mara")
            .unwrap()
            .add_episode(NewEpisode {
                source: Some(Source {
                    chat: "earlier".into(),
                    from: 0,
                    to: 1,
                }),
                summary: "Walked the tide pools at dawn.".into(),
                origin: Origin::ModelClean,
                model: "m".into(),
                ..NewEpisode::default()
            })
            .unwrap();
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        let id = opened["session"].as_str().unwrap().to_string();
        turn(&w, &key, "Hello there.").await;
        // Out of memory, as after a restart.
        w.personas().sessions.lock().await.clear();
        let resumed = w
            .personas()
            .resume(&w.chat, &w.library, "mara", &id, None)
            .await
            .unwrap();
        turn(&w, resumed["key"].as_str().unwrap(), "And then?").await;
        let seen = w.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 2);
        for req in &seen {
            assert!(req.messages[0].text().contains("tide pools"));
            let copies: usize = req
                .messages
                .iter()
                .map(|m| m.text().matches("tide pools").count())
                .sum();
            assert_eq!(copies, 1);
        }
    }

    /// §5.1: a chat already past its first reply that never stored its
    /// chat-start memory (begun under #572, or a first turn whose read
    /// failed) gets it as a run note, never folded into a later owner turn:
    /// compaction keeps only `messages[0]` whole, so a block stored mid-history
    /// would be summarised away and stored again (review of #575).
    #[tokio::test]
    async fn memory_a_chat_never_stored_rides_as_a_note_past_the_first_reply() {
        use mecha_core::persona::memory::{Memory, NewEpisode, Source};
        use mecha_core::persona::Origin;
        let w = world();
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        let id = opened["session"].as_str().unwrap().to_string();
        turn(&w, &key, "Hello there.").await;
        Memory::open(&w.store(), "mara")
            .unwrap()
            .add_episode(NewEpisode {
                source: Some(Source {
                    chat: "earlier".into(),
                    from: 0,
                    to: 1,
                }),
                summary: "Walked the tide pools at dawn.".into(),
                origin: Origin::ModelClean,
                model: "m".into(),
                ..NewEpisode::default()
            })
            .unwrap();
        // Out of memory, as after a restart.
        w.personas().sessions.lock().await.clear();
        let resumed = w
            .personas()
            .resume(&w.chat, &w.library, "mara", &id, None)
            .await
            .unwrap();
        let key2 = resumed["key"].as_str().unwrap().to_string();
        turn(&w, &key2, "And then?").await;
        turn(&w, &key2, "Go on.").await;

        let seen = w.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 3);
        assert!(!seen[0].messages[0].text().contains("tide pools"));
        for req in &seen[1..] {
            let newest = req.messages.last().unwrap().text();
            assert!(
                newest.contains("Walked the tide pools at dawn."),
                "{newest}"
            );
            let copies: usize = req
                .messages
                .iter()
                .map(|m| m.text().matches("tide pools").count())
                .sum();
            assert_eq!(copies, 1, "a note, never stored mid-history");
        }
    }

    /// §9.7, on every turn: past the first reply, an owner message that
    /// names something remembered carries it, in the harness's voice; a
    /// message too short to name anything carries nothing.
    #[tokio::test]
    async fn each_turn_recalls_what_the_owners_message_names() {
        use mecha_core::persona::memory::{Memory, NewEpisode, Source};
        use mecha_core::persona::Origin;
        let w = world();
        let m = Memory::open(&w.store(), "mara").unwrap();
        // Six conversations: the chat starts with the five most recent, so
        // the oldest — the one about Holdfast — is only reached by asking.
        for summary in [
            "Named the kelp project Holdfast.",
            "Talked about the weather.",
            "Planned a reading list.",
            "Discussed a seminar.",
            "Went over a budget.",
            "Chatted about a film.",
        ] {
            m.add_episode(NewEpisode {
                source: Some(Source {
                    chat: "earlier".into(),
                    from: 0,
                    to: 1,
                }),
                summary: summary.into(),
                origin: Origin::ModelClean,
                model: "m".into(),
                ..NewEpisode::default()
            })
            .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        turn(&w, &key, "Hello again, how was your week?").await;
        turn(&w, &key, "How is the Holdfast work going?").await;
        turn(&w, &key, "ok").await;
        turn(&w, &key, "And the Holdfast project, again?").await;

        let seen = w.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 4);
        let second = seen[1].messages.last().unwrap().text();
        assert!(
            second.contains("How is the Holdfast work going?")
                && second.contains("what this message brought to mind")
                && second.contains("Named the kelp project Holdfast."),
            "{second}"
        );
        let third = seen[2].messages.last().unwrap().text();
        assert!(!third.contains("brought to mind"), "{third}");
        // Asked again, it is recalled again: a recall is a note for its own
        // run (§5.1), so the history holds no earlier copy to lean on, and
        // none is re-sent.
        let fourth = &seen[3].messages;
        let newest = fourth.last().unwrap().text();
        assert!(
            newest.contains("And the Holdfast project, again?")
                && newest.contains("Named the kelp project Holdfast."),
            "{newest}"
        );
        let copies: usize = fourth
            .iter()
            .map(|m| m.text().matches("Named the kelp project Holdfast.").count())
            .sum();
        assert_eq!(copies, 1);
        let t = w
            .personas()
            .transcript(&w.chat, &w.library, &key, None)
            .await
            .unwrap();
        let bubbles: Vec<String> = t["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == "user")
            .map(|e| e["text"].as_str().unwrap_or("").to_string())
            .collect();
        assert_eq!(
            bubbles,
            [
                "Hello again, how was your week?",
                "How is the Holdfast work going?",
                "ok",
                "And the Holdfast project, again?"
            ],
            "{t}"
        );
        assert_eq!(t["taint"]["untrusted"], false, "{t}");
    }

    /// §9.7: a record from outside, recalled mid-chat, arms the chat
    /// untrusted at the doors — the chat starts clean (its newest episodes
    /// are clean) and the per-turn fold is what arms it (review of #481).
    #[tokio::test]
    async fn a_record_from_outside_recalled_mid_chat_arms_it_untrusted() {
        use mecha_core::persona::memory::{Memory, NewEpisode, Source};
        use mecha_core::persona::Origin;
        let w = world();
        let m = Memory::open(&w.store(), "mara").unwrap();
        let outside = m
            .add_episode(NewEpisode {
                source: Some(Source {
                    chat: "earlier".into(),
                    from: 0,
                    to: 1,
                }),
                summary: "Read a page claiming Holdfast kelp sings at dawn.".into(),
                origin: Origin::ModelUntrusted,
                model: "m".into(),
                ..NewEpisode::default()
            })
            .unwrap();
        m.approve(&outside.uid).unwrap();
        for summary in [
            "Weather.",
            "A reading list.",
            "A seminar.",
            "A budget.",
            "A film.",
        ] {
            std::thread::sleep(std::time::Duration::from_millis(5));
            m.add_episode(NewEpisode {
                source: Some(Source {
                    chat: "earlier".into(),
                    from: 0,
                    to: 1,
                }),
                summary: summary.into(),
                origin: Origin::ModelClean,
                model: "m".into(),
                ..NewEpisode::default()
            })
            .unwrap();
        }
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        turn(&w, &key, "Hello again, how was your week?").await;
        let t = w
            .personas()
            .transcript(&w.chat, &w.library, &key, None)
            .await
            .unwrap();
        assert_eq!(t["taint"]["untrusted"], false, "clean at the start: {t}");
        turn(&w, &key, "What did that page say about Holdfast kelp?").await;
        let t = w
            .personas()
            .transcript(&w.chat, &w.library, &key, None)
            .await
            .unwrap();
        assert_eq!(t["taint"]["untrusted"], true, "armed by the recall: {t}");
    }

    /// §9.7: a persona whose memory is switched off is offered neither memory
    /// tool, so the tool list never promises what the store will refuse.
    #[tokio::test]
    async fn a_persona_with_memory_off_is_offered_no_memory_tools() {
        let w = world();
        let toml = w.store().join("mara/persona.toml");
        let text = std::fs::read_to_string(&toml).unwrap();
        let off = text
            .replace("episodic    = true", "episodic    = false")
            .replace("semantic    = true", "semantic    = false")
            .replace("user_facts  = \"shared\"", "user_facts  = \"off\"");
        assert_ne!(off, text, "the template's spelling moved");
        std::fs::write(&toml, off).unwrap();
        mecha_core::persona::snapshot(&w.store(), "mara").unwrap();
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        turn(&w, &key, "Hello again.").await;
        let seen = w.seen.lock().unwrap().clone();
        let tools: Vec<&str> = seen[0].tools.iter().map(|t| t.name.as_str()).collect();
        assert!(
            !tools.contains(&"memory_search") && !tools.contains(&"memory_read"),
            "{tools:?}"
        );
    }

    /// §10.4: a citation is checked against what the chat received, sent to
    /// the page just after the run's end and carried in the transcript, and
    /// it opens the page it cites with the quote marked. A made-up quote is
    /// said, not dropped.
    #[tokio::test]
    async fn a_citation_is_checked_against_the_files_and_opens_its_page() {
        let w = world_with(Mode::Say(
            "They graze [urchins.md: \"graze kelp holdfasts at night\"], and \
             [urchins.md: \"otters eat forty a day\"]."
                .into(),
        ));
        std::fs::write(
            w.store().join("mara/files/urchins.md"),
            "Sea urchins graze kelp holdfasts at night.",
        )
        .unwrap();
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        let (mut rx, _) = w
            .personas()
            .subscribe(&w.library, &key, None)
            .await
            .unwrap();
        w.personas()
            .send(&w.chat, &w.library, &key, "What do urchins do?", None, None)
            .await
            .unwrap();
        // The run's end, then its citations: checked once the reply is
        // handed back, so the input is not held for them (review of #465).
        let mut done = false;
        let sent = loop {
            match tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await {
                Ok(Ok(WireEvent::Citations { checks })) => {
                    assert!(done, "the citations follow the end");
                    break checks;
                }
                Ok(Ok(WireEvent::Done { ok, error, .. })) => {
                    assert!(ok, "{error:?}");
                    done = true;
                }
                Ok(Ok(_)) => continue,
                other => panic!("no citations after the end: {other:?}"),
            }
        };
        let statuses = |v: &serde_json::Value| -> Vec<String> {
            v.as_array()
                .unwrap()
                .iter()
                .map(|c| c["status"].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(statuses(&serde_json::json!(sent)), ["quoted", "not_found"]);
        let t = w
            .personas()
            .transcript(&w.chat, &w.library, &key, None)
            .await
            .unwrap();
        assert_eq!(statuses(&t["citations"]), ["quoted", "not_found"], "{t}");

        let page = w
            .personas()
            .cited(
                &w.library,
                &key,
                "urchins.md",
                None,
                "graze kelp holdfasts",
                None,
            )
            .await
            .unwrap();
        assert_eq!(page["before"], "Sea urchins ", "{page}");
        assert_eq!(page["marked"], "graze kelp holdfasts", "{page}");
        // A file the chat never received is not served from here.
        assert!(matches!(
            w.personas()
                .cited(&w.library, &key, "elsewhere.md", None, "x", None)
                .await,
            Err(Refusal::NotFound)
        ));
    }

    /// Pass 8 of #459: the files ride before the first reply or not at all.
    /// Mid-conversation a compaction would summarise them away and the next
    /// turn would fold the whole collection back in; so a file added to an
    /// open chat reaches the next chat, and this one reads it on request.
    #[tokio::test]
    async fn a_file_added_after_the_first_reply_waits_for_the_next_chat() {
        let w = world();
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        turn(&w, &key, "Hello").await;
        std::fs::write(
            w.store().join("mara/files/urchins.md"),
            "Sea urchins graze kelp holdfasts.",
        )
        .unwrap();
        turn(&w, &key, "Anything new?").await;
        let seen = w.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 2);
        assert!(
            seen[1]
                .messages
                .iter()
                .all(|m| !m.text().contains(mecha_core::persona::files::FILES_STEM)),
            "{:?}",
            seen[1].messages
        );
    }

    /// A first turn that folds into the owner's tail — the state a first
    /// turn that died before any reply leaves — still carries the files
    /// (review of #459: the fold path dropped them).
    #[tokio::test]
    async fn the_files_ride_a_folded_first_turn_too() {
        let w = world();
        std::fs::write(
            w.store().join("mara/files/urchins.md"),
            "Urchins graze kelp.",
        )
        .unwrap();
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        {
            let personas = w.personas();
            let mut sessions = personas.sessions.lock().await;
            let ps = sessions.get_mut(&key).unwrap();
            ps.conversation
                .as_mut()
                .unwrap()
                .push(Message::user("a turn that was cancelled"));
        }
        turn(&w, &key, "Go on").await;
        let seen = w.seen.lock().unwrap().clone();
        let first = seen[0].messages[0].text();
        assert!(
            first.contains("a turn that was cancelled") && first.contains("Go on"),
            "{first}"
        );
        assert!(
            first.contains(mecha_core::persona::files::FILES_STEM)
                && first.contains("Urchins graze kelp."),
            "{first}"
        );
    }

    /// A fact in `who`'s memory, from a chat whose id dates it 30 September
    /// 2026 at 02:49 UTC — the 29th in New York — so a page's `day` is fixed.
    fn remember(
        w: &World,
        who: &str,
        table: mecha_core::persona::memory::Table,
        text: &str,
        origin: Origin,
    ) -> String {
        use mecha_core::persona::memory::{Kind, Memory, NewFact, Source};
        Memory::open(&w.store(), who)
            .unwrap()
            .add_fact(
                table,
                NewFact {
                    text: text.into(),
                    kind: Kind::Stated,
                    source: Source {
                        chat: "20260930T024959-1a2b3c4d".into(),
                        from: 0,
                        to: 1,
                    },
                    origin,
                    model: "m".into(),
                    valid_from: None,
                    valid_to: None,
                },
            )
            .unwrap()
            .uid
    }

    #[tokio::test]
    async fn the_memory_page_lists_what_stands_dated_by_when_it_was_said() {
        use mecha_core::persona::memory::{Memory, Table};
        let w = world();
        let page = |w: &World| {
            w.personas()
                .memory(&w.library, "mara", None, Some(chrono_tz::UTC))
                .unwrap()
        };
        // Nothing remembered: an empty page, and no store made by looking.
        let empty = page(&w);
        assert_eq!(empty["episodes"].as_array().unwrap().len(), 0);
        assert!(!w.store().join("mara/memory.db").exists());

        let kept = remember(
            &w,
            "mara",
            Table::User,
            "Teaches on Thursdays.",
            Origin::ModelClean,
        );
        let waiting = remember(
            &w,
            "mara",
            Table::User,
            "Read about kelp.",
            Origin::ModelUntrusted,
        );
        let gone = remember(
            &w,
            "mara",
            Table::Persona,
            "Grew up inland.",
            Origin::ModelClean,
        );
        Memory::open_to_edit(&w.store(), "mara")
            .unwrap()
            .invalidate(&gone)
            .unwrap();

        let got = page(&w);
        let user = got["facts"]["user"].as_array().unwrap();
        let row = |uid: &str| user.iter().find(|f| f["uid"] == uid).cloned();
        assert_eq!(
            row(&kept).unwrap()["day"],
            "2026-09-30",
            "the chat's day: {got}"
        );
        assert_eq!(
            row(&waiting).unwrap()["status"],
            "candidate",
            "waiting on the owner is shown"
        );
        assert!(
            got["facts"]["persona"].as_array().unwrap().is_empty(),
            "a withdrawn fact is not: {got}"
        );
        let ny = w
            .personas()
            .memory(&w.library, "mara", None, "America/New_York".parse().ok())
            .unwrap();
        assert_eq!(
            ny["facts"]["user"][0]["day"], "2026-09-29",
            "in the owner's zone"
        );

        // A shared store that will not read costs the share marks, not the page.
        std::fs::write(
            mecha_core::persona::memory::Shared::path(&w.store()),
            b"not a database",
        )
        .unwrap();
        let hurt = page(&w);
        assert!(hurt["shared_problem"].is_string(), "{hurt}");
        assert_eq!(hurt["facts"]["user"].as_array().unwrap().len(), 2, "{hurt}");
    }

    #[tokio::test]
    async fn the_owner_curates_from_the_page_and_the_lock_holds_for_every_act() {
        use mecha_core::persona::memory::Table;
        let w = world();
        let act = |w: &World, a: MemoryAct, token: Option<&str>| {
            w.personas()
                .memory_act(&w.library, "mara", a, token, Some(chrono_tz::UTC))
        };
        let user = |v: &serde_json::Value| v["facts"]["user"].as_array().unwrap().clone();
        let uid = remember(
            &w,
            "mara",
            Table::User,
            "Teaches on Thursdays.",
            Origin::ModelClean,
        );
        let cand = remember(
            &w,
            "mara",
            Table::User,
            "Read about kelp.",
            Origin::ModelUntrusted,
        );
        let short = |u: &str| u[..8].to_string();
        // Unsharing before anything was ever shared makes no shared store.
        assert!(matches!(
            act(
                &w,
                MemoryAct::Unshare {
                    id: "abcd1234".into()
                },
                None
            ),
            Err(Refusal::Bad(_))
        ));
        assert!(!mecha_core::persona::memory::Shared::path(&w.store()).exists());

        let v = act(&w, MemoryAct::Approve { id: short(&cand) }, None).unwrap();
        assert!(user(&v).iter().all(|f| f["status"] == "active"), "{v}");
        let v = act(&w, MemoryAct::Pin { id: short(&uid) }, None).unwrap();
        assert!(
            user(&v)
                .iter()
                .any(|f| f["uid"] == uid && f["pinned"] == true),
            "{v}"
        );
        let v = act(
            &w,
            MemoryAct::Correct {
                id: short(&uid),
                text: "Teaches on Tuesdays.".into(),
            },
            None,
        )
        .unwrap();
        let texts: Vec<_> = user(&v).iter().map(|f| f["text"].clone()).collect();
        assert!(texts.contains(&"Teaches on Tuesdays.".into()), "{v}");
        assert!(
            !texts.contains(&"Teaches on Thursdays.".into()),
            "the old wording is withdrawn: {v}"
        );
        let new = user(&v)
            .into_iter()
            .find(|f| f["text"] == "Teaches on Tuesdays.")
            .unwrap()["uid"]
            .as_str()
            .unwrap()
            .to_string();
        let v = act(
            &w,
            MemoryAct::Share {
                id: short(&new),
                group: None,
            },
            None,
        )
        .unwrap();
        let copy = user(&v).iter().find(|f| f["uid"] == new).unwrap()["shared"][0].clone();
        assert!(copy["group"].is_null(), "shared with everyone: {v}");
        let v = act(
            &w,
            MemoryAct::Unshare {
                id: copy["uid"].as_str().unwrap()[..8].to_string(),
            },
            None,
        )
        .unwrap();
        assert!(
            user(&v)
                .iter()
                .all(|f| f["shared"].as_array().unwrap().is_empty()),
            "{v}"
        );
        assert!(matches!(
            act(
                &w,
                MemoryAct::Share {
                    id: short(&new),
                    group: Some("nowhere".into())
                },
                None
            ),
            Err(Refusal::Bad(_))
        ));
        assert!(matches!(
            act(&w, MemoryAct::Pin { id: "zz".into() }, None),
            Err(Refusal::Bad(_))
        ));

        // Locked: every read and act answers as a missing persona.
        store::set_locked(&w.store(), "mara", true).unwrap();
        assert!(matches!(
            w.personas().memory(&w.library, "mara", None, None),
            Err(Refusal::NotFound)
        ));
        assert!(matches!(
            act(&w, MemoryAct::Forget { id: short(&new) }, None),
            Err(Refusal::NotFound)
        ));
        let token = w.library.grant_for_tests();
        let v = act(&w, MemoryAct::Forget { id: short(&new) }, Some(&token)).unwrap();
        assert!(user(&v).iter().all(|f| f["uid"] != new), "forgotten: {v}");
    }

    #[tokio::test]
    async fn one_personas_page_never_shows_or_touches_anothers_shared_facts() {
        use mecha_core::persona::memory::{Audience, Memory, Shared, Table};
        let w = world();
        let lib = mecha_core::imagelib::Library::load(&w.root.join("imagelib")).0;
        store::create(
            &w.store(),
            &lib,
            NewPersona {
                name: "tau".into(),
                display: "Tau".into(),
                relationships: vec!["colleague".into()],
                origin: Origin::Owner,
                ..NewPersona::default()
            },
        )
        .unwrap();
        let theirs = remember(&w, "tau", Table::User, "Swims at dawn.", Origin::ModelClean);
        let fact = Memory::open(&w.store(), "tau")
            .unwrap()
            .fact(&theirs)
            .unwrap()
            .unwrap();
        let copy = Shared::open(&w.store())
            .unwrap()
            .share(&fact, Audience::Everyone, &[])
            .unwrap();
        remember(
            &w,
            "mara",
            Table::User,
            "Teaches on Thursdays.",
            Origin::ModelClean,
        );

        let v = w
            .personas()
            .memory(&w.library, "mara", None, Some(chrono_tz::UTC))
            .unwrap();
        assert!(!v.to_string().contains("Swims at dawn"), "{v}");
        assert!(matches!(
            w.personas().memory_act(
                &w.library,
                "mara",
                MemoryAct::Unshare {
                    id: copy.uid[..8].to_string()
                },
                None,
                None
            ),
            Err(Refusal::Bad(_))
        ));
        assert_eq!(
            Shared::open(&w.store()).unwrap().all().unwrap().facts.len(),
            1
        );
    }

    /// The avatar's framing: set and cleared from the page, carried by the
    /// list, refused out of range, and behind the lock like every write.
    #[tokio::test]
    async fn a_frame_is_placed_listed_checked_and_locked_like_any_write() {
        use mecha_core::persona::Frame;
        let w = world();
        let row = |token: Option<&str>| {
            w.personas().list(&w.library, token, Some(chrono_tz::UTC))["personas"][0].clone()
        };
        assert!(row(None)["frame"].is_null(), "the default until placed");
        let frame = Frame {
            x: 0.5,
            y: 0.25,
            zoom: 1.5,
        };
        let set = w
            .personas()
            .frame(&w.library, "mara", Some(frame), None)
            .unwrap();
        assert_eq!(set["frame"]["zoom"], 1.5);
        assert_eq!(row(None)["frame"]["y"], 0.25);
        assert!(matches!(
            w.personas()
                .frame(&w.library, "mara", Some(Frame { zoom: 9.0, ..frame }), None),
            Err(Refusal::Bad(_))
        ));
        assert_eq!(
            row(None)["frame"]["zoom"],
            1.5,
            "a refused frame changes nothing"
        );
        // Locked: a hidden persona answers as a missing one, framing included.
        store::set_locked(&w.store(), "mara", true).unwrap();
        assert!(matches!(
            w.personas().frame(&w.library, "mara", None, None),
            Err(Refusal::NotFound)
        ));
        let token = w.library.grant_for_tests();
        w.personas()
            .frame(&w.library, "mara", None, Some(&token))
            .unwrap();
        assert!(row(Some(&token))["frame"].is_null(), "the default again");
    }

    /// A proposal from the main chat, read and approved where personas live
    /// (the owner's rulings of 2026-10-01): approved only as shown, its
    /// waiting character with it in one tap, turned away only while waiting,
    /// and behind the lock like every read.
    #[tokio::test]
    async fn a_proposal_is_approved_as_shown_with_its_character_or_turned_away() {
        use mecha_core::imagelib::{self, Kind, Origin, Status};
        use mecha_core::persona::{propose, Proposal};
        let w = world();
        let lib_dir = w.library.dir.clone();
        let mut png = Vec::new();
        image::RgbImage::new(4, 4)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        imagelib::create(
            &lib_dir,
            imagelib::NewEntry {
                kind: Kind::Character,
                name: "wren".into(),
                text: "short, freckled, a wool cap".into(),
                portrait: Some(png),
                source_seed: None,
                origin: Origin::ModelClean,
                locked: false,
            },
        )
        .unwrap();
        let lib = || imagelib::Library::load(&lib_dir).0;
        let proposal = |name: &str, core: &str, locked: bool| Proposal {
            name: name.into(),
            display: String::new(),
            relationships: Vec::new(),
            character: (name == "wren").then(|| "wren".to_string()),
            voice: None,
            identity: format!("## Core\n{core}\n"),
            motivation: String::new(),
            origin: Origin::ModelClean,
            locked,
        };
        propose(
            &w.store(),
            &lib(),
            proposal("wren", "A field botanist.", false),
        )
        .unwrap();

        // Listed as waiting, and read with what approval needs.
        let listed = w.personas().list(&w.library, None, Some(chrono_tz::UTC));
        let row = listed["personas"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "wren")
            .unwrap()
            .clone();
        assert_eq!(row["waiting"], true);
        assert_eq!(row["origin"], "model_clean");
        let read = w.personas().review(&w.library, "wren", None).unwrap();
        assert!(read["identity"].as_str().unwrap().contains("botanist"));
        assert_eq!(read["character"]["waiting"], true);
        let shown = read["shown"].as_str().unwrap().to_string();
        let char_shown = read["character"]["shown"].as_str().unwrap().to_string();

        // A forged signature, or one from before a revision, approves nothing.
        assert!(matches!(
            w.personas()
                .approve(&w.library, "wren", "forged", None, None),
            Err(Refusal::Conflict(_))
        ));
        propose(
            &w.store(),
            &lib(),
            proposal("wren", "A field botanist, revised.", false),
        )
        .unwrap();
        assert!(matches!(
            w.personas()
                .approve(&w.library, "wren", &shown, Some(&char_shown), None),
            Err(Refusal::Conflict(_))
        ));
        assert_eq!(
            lib().get(Kind::Character, "wren").unwrap().status,
            Status::Candidate,
            "a refused persona approves no character"
        );

        // Read again. Not the persona alone while its portrait waits.
        let read = w.personas().review(&w.library, "wren", None).unwrap();
        assert!(matches!(
            w.personas().approve(
                &w.library,
                "wren",
                read["shown"].as_str().unwrap(),
                None,
                None
            ),
            Err(Refusal::Conflict(_))
        ));
        // Approved as shown: the character with it.
        let approved = w
            .personas()
            .approve(
                &w.library,
                "wren",
                read["shown"].as_str().unwrap(),
                read["character"]["shown"].as_str(),
                None,
            )
            .unwrap();
        assert_eq!(approved["character"], "wren");
        assert_eq!(
            lib().get(Kind::Character, "wren").unwrap().status,
            Status::Approved
        );
        let wren = Store::load(&w.store()).get("wren").unwrap().clone();
        assert_eq!(wren.state.status, Status::Approved);
        assert!(
            wren.identity.contains("revised"),
            "what was shown, the last read"
        );
        // Approved: no longer waiting, and not turned away from here.
        assert!(matches!(
            w.personas().review(&w.library, "wren", None),
            Err(Refusal::Conflict(_))
        ));
        assert!(matches!(
            w.personas().reject(&w.library, "wren", None),
            Err(Refusal::Conflict(_))
        ));

        // An owner's own unapproved persona is not a proposal: not reviewed
        // or turned away here.
        let mut mine = mecha_core::persona::NewPersona {
            name: "mine".into(),
            origin: Origin::Owner,
            ..Default::default()
        };
        mine.display = "Mine".into();
        store::create(&w.store(), &lib(), mine).unwrap();
        // Unapproved, as a persona made by hand in its folder is.
        let state_path = w.store().join("mine/state.toml");
        let state_text = std::fs::read_to_string(&state_path).unwrap();
        assert!(state_text.contains("status = \"approved\""), "{state_text}");
        std::fs::write(
            &state_path,
            state_text.replace("status = \"approved\"", "status = \"candidate\""),
        )
        .unwrap();
        let mine = Store::load(&w.store()).get("mine").unwrap().clone();
        assert_eq!(
            (mine.state.status, mine.state.origin),
            (Status::Candidate, Origin::Owner)
        );
        assert!(matches!(
            w.personas().review(&w.library, "mine", None),
            Err(Refusal::Conflict(_))
        ));
        assert!(matches!(
            w.personas().reject(&w.library, "mine", None),
            Err(Refusal::Conflict(_))
        ));
        assert!(Store::load(&w.store()).get("mine").is_some());
        // A folder made by hand, with no state.toml at all, loads as an
        // untrusted candidate — and is still not a proposal: not listed as
        // waiting, not turned away, its folder left where it is.
        std::fs::remove_file(&state_path).unwrap();
        let bare = Store::load(&w.store()).get("mine").unwrap().clone();
        assert_eq!(
            (bare.state.status, bare.state.origin),
            (Status::Candidate, Origin::ModelUntrusted)
        );
        let listed = w.personas().list(&w.library, None, Some(chrono_tz::UTC));
        let row = listed["personas"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "mine")
            .unwrap()
            .clone();
        assert_eq!(row["waiting"], false, "{row}");
        assert!(matches!(
            w.personas().reject(&w.library, "mine", None),
            Err(Refusal::Conflict(_))
        ));
        assert!(w.store().join("mine/persona.toml").is_file());

        // A locked proposal (an incognito chat's) is hidden like any locked
        // persona until unlocked, then turned away.
        propose(
            &w.store(),
            &lib(),
            proposal("noor", "A night-shift nurse.", true),
        )
        .unwrap();
        assert!(matches!(
            w.personas().review(&w.library, "noor", None),
            Err(Refusal::NotFound)
        ));
        assert!(matches!(
            w.personas().reject(&w.library, "noor", None),
            Err(Refusal::NotFound)
        ));
        let token = w.library.grant_for_tests();
        assert_eq!(
            w.personas()
                .review(&w.library, "noor", Some(&token))
                .unwrap()["locked"],
            true
        );
        w.personas()
            .reject(&w.library, "noor", Some(&token))
            .unwrap();
        assert!(Store::load(&w.store()).get("noor").is_none());
    }

    #[tokio::test]
    async fn a_locked_persona_is_indistinguishable_from_none() {
        let w = world();
        store::set_locked(&w.store(), "mara", true).unwrap();
        let listed = w.personas().list(&w.library, None, Some(chrono_tz::UTC));
        assert_eq!(listed["personas"].as_array().unwrap().len(), 0);
        assert!(listed.get("hidden_locked").is_none(), "{listed}");
        for token in [None, Some("not-a-token")] {
            let refused = w
                .personas()
                .open(&w.chat, &w.library, "mara", token, None)
                .await
                .unwrap_err();
            assert!(matches!(refused, Refusal::NotFound), "{refused:?}");
            assert!(matches!(
                w.personas().history(&w.library, "mara", token),
                Err(Refusal::NotFound)
            ));
        }
        let token = w.library.grant_for_tests();
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", Some(&token), None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap();
        // The chat is locked with its persona: its transcript without a
        // token is a 404 too.
        assert!(matches!(
            w.personas()
                .transcript(&w.chat, &w.library, key, None)
                .await,
            Err(Refusal::NotFound)
        ));
        assert!(w
            .personas()
            .transcript(&w.chat, &w.library, key, Some(&token))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn an_unapproved_persona_cannot_open_a_chat() {
        let w = world();
        std::fs::remove_file(w.store().join("mara/state.toml")).unwrap();
        let refused = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap_err();
        match refused {
            Refusal::Conflict(m) => assert!(m.contains("not approved"), "{m}"),
            other => panic!("{other:?}"),
        }
        assert!(!w
            .store()
            .join("mara/sessions")
            .read_dir()
            .unwrap()
            .any(|_| true));
    }

    /// A chat resumed after its persona was edited runs on the version it
    /// began with (§8.1), and a new chat on the new one.
    #[tokio::test]
    async fn a_resumed_chat_keeps_its_version() {
        let w = world();
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        let id = opened["session"].as_str().unwrap().to_string();
        turn(&w, &key, "First").await;
        let id_md = w.store().join("mara/identity.md");
        let text = std::fs::read_to_string(&id_md)
            .unwrap()
            .replace("distrusts", "enjoys");
        std::fs::write(&id_md, text).unwrap();

        // Out of memory, as after a restart.
        w.personas().sessions.lock().await.clear();
        let resumed = w
            .personas()
            .resume(&w.chat, &w.library, "mara", &id, None)
            .await
            .unwrap();
        let key2 = resumed["key"].as_str().unwrap().to_string();
        turn(&w, &key2, "Second").await;
        let fresh = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        turn(&w, fresh["key"].as_str().unwrap(), "Third").await;

        let seen = w.seen.lock().unwrap().clone();
        let system = |i: usize| seen[i].system.clone().unwrap_or_default();
        assert!(
            system(1).contains("distrusts easy answers"),
            "resumed on the old version"
        );
        assert_eq!(system(0), system(1));
        assert!(
            system(2).contains("enjoys easy answers"),
            "a new chat on the new one"
        );
        // The resumed turn carried the earlier conversation.
        assert!(seen[1].messages.len() >= 3);
    }

    /// A chat resumed after a restart keeps its workspace, so a picture the
    /// persona drew before is still there to edit (owner, 2026-09-30).
    #[tokio::test]
    async fn a_resumed_chat_keeps_its_workspace() {
        let w = world();
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        let id = opened["session"].as_str().unwrap().to_string();
        let first = w.personas().sessions.lock().await[&key].workspace.clone();
        std::fs::create_dir_all(first.join("images")).unwrap();
        std::fs::write(first.join("images/drawn.png"), b"png").unwrap();

        w.personas().sessions.lock().await.clear();
        let resumed = w
            .personas()
            .resume(&w.chat, &w.library, "mara", &id, None)
            .await
            .unwrap();
        let key2 = resumed["key"].as_str().unwrap().to_string();
        assert_ne!(key, key2);
        let again = w.personas().sessions.lock().await[&key2].workspace.clone();
        assert_eq!(again, first);
        assert!(again.join("images/drawn.png").is_file());
    }

    /// A chat's pictures and masks go through its own door, locked with its
    /// persona: the Edit button reads the picture and writes the mask here,
    /// and neither is reachable without the token once the persona locks.
    #[tokio::test]
    async fn a_chats_pictures_are_behind_its_persona_lock() {
        use axum::body::Bytes;
        let w = world();
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        let ws = w
            .personas()
            .workspace_of(&w.library, &key, None)
            .await
            .unwrap();
        std::fs::create_dir_all(ws.join("images")).unwrap();
        std::fs::write(ws.join("images/drawn.png"), b"png").unwrap();

        let shown = super::super::files::serve(ws.clone(), "images/drawn.png".into()).await;
        assert_eq!(shown.status(), StatusCode::OK);
        assert_eq!(shown.headers()["content-type"], "image/png");
        // A real file just outside the jail, so the 404 is containment and
        // not absence (review of #438).
        std::fs::write(ws.parent().unwrap().join("drawn.png"), b"png").unwrap();
        let outside = super::super::files::serve(ws.clone(), "../drawn.png".into()).await;
        assert_eq!(outside.status(), StatusCode::NOT_FOUND);

        let stored =
            super::super::files::store(ws.clone(), "mask-drawn-1.png", Bytes::from_static(b"m"))
                .await;
        assert_eq!(stored.status(), StatusCode::OK);
        let body = axum::body::to_bytes(stored.into_body(), 1 << 16)
            .await
            .unwrap();
        let path: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(path["path"], "inbox/mask-drawn-1.png");

        // Locked: the same 404 as a missing chat, until the token comes.
        store::set_locked(&w.store(), "mara", true).unwrap();
        for token in [None, Some("not-a-token")] {
            assert!(matches!(
                w.personas().workspace_of(&w.library, &key, token).await,
                Err(Refusal::NotFound)
            ));
        }
        let token = w.library.grant_for_tests();
        assert_eq!(
            w.personas()
                .workspace_of(&w.library, &key, Some(&token))
                .await
                .unwrap(),
            ws
        );
        // An assistant chat's key is not this door's to answer.
        assert!(matches!(
            w.personas()
                .workspace_of(&w.library, "0123456789abcdef", None)
                .await,
            Err(Refusal::Bad(_))
        ));
    }

    /// A dropped picture rides on the persona's turn as pixels when its
    /// model can see, arming `private` as in the assistant's chat; a blind
    /// model gets the path alone, and the answer counts what was not shown.
    #[tokio::test]
    async fn an_attached_picture_rides_on_the_turn_for_a_model_that_sees() {
        let w = world();
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        let ws = w
            .personas()
            .workspace_of(&w.library, &key, None)
            .await
            .unwrap();
        std::fs::create_dir_all(ws.join("inbox")).unwrap();
        let mut png = Vec::new();
        image::RgbImage::new(4, 4)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        std::fs::write(ws.join("inbox/photo.png"), &png).unwrap();
        std::fs::write(ws.join("inbox/notes.txt"), b"notes").unwrap();
        let attached = vec!["inbox/photo.png".to_string(), "inbox/notes.txt".to_string()];
        let text = "Look\n\nAttached file at inbox/photo.png\nAttached file at inbox/notes.txt";

        let send = |sees: bool| {
            w.sees.store(sees, std::sync::atomic::Ordering::SeqCst);
            let (w, key, attached) = (&w, key.clone(), attached.clone());
            async move {
                let (mut rx, _) = w
                    .personas()
                    .subscribe(&w.library, &key, None)
                    .await
                    .unwrap();
                let answer = w
                    .personas()
                    .send_with(&w.chat, &w.library, &key, text, None, None, attached, false)
                    .await
                    .unwrap();
                loop {
                    match tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await
                    {
                        Ok(Ok(WireEvent::Done {
                            ok,
                            error,
                            taint_private,
                            ..
                        })) => {
                            assert!(ok, "{error:?}");
                            return (answer, taint_private);
                        }
                        Ok(Ok(_)) => continue,
                        other => panic!("no Done event: {other:?}"),
                    }
                }
            }
        };
        let images_in_last_turn = || {
            let seen = w.seen.lock().unwrap();
            let last = seen.last().unwrap();
            let user = last
                .messages
                .iter()
                .rev()
                .find(|m| m.role == mecha_core::message::Role::User)
                .unwrap();
            user.content
                .iter()
                .filter(|b| matches!(b, Block::Image { .. }))
                .count()
        };

        // Blind: the path is in the text, no pixels, and one not shown.
        let (answer, private) = send(false).await;
        assert_eq!(answer["model_sees"], false);
        assert_eq!(answer["pictures_not_shown"], 1);
        assert_eq!(images_in_last_turn(), 0);
        assert!(!private, "nothing private entered a blind turn");

        // Seeing: the picture rides as pixels (the text file never does),
        // and the chat now holds something private.
        let (answer, private) = send(true).await;
        assert_eq!(answer["model_sees"], true);
        assert_eq!(answer["pictures_not_shown"], 0);
        assert_eq!(images_in_last_turn(), 1);
        assert!(private, "an attached picture arms private_data");
    }

    /// The tool a run waits on is named until its result arrives.
    #[test]
    fn the_working_slot_follows_a_call_to_its_result() {
        let slot = StdMutex::new(Vec::new());
        let call = |id: &str, name: &str| AgentEvent::ToolCall {
            id: id.into(),
            name: name.into(),
            input: serde_json::json!({}),
        };
        let result = |id: &str| AgentEvent::ToolResult {
            id: id.into(),
            name: "x".into(),
            is_error: false,
            content: String::new(),
        };
        track_working(&slot, &call("t1", "image_generate"));
        track_working(&slot, &call("t2", "web_search"));
        assert_eq!(slot.lock().unwrap().len(), 2);
        // The later call returning first leaves the earlier one named: two
        // calls in one turn run at once (review of #431).
        track_working(&slot, &result("t2"));
        let held = slot.lock().unwrap().clone();
        assert_eq!(held.len(), 1);
        assert_eq!(held[0]["name"], "image_generate");
        assert!(held[0]["since"].as_str().is_some());
        track_working(&slot, &result("t1"));
        assert!(slot.lock().unwrap().is_empty());
        // A refused call is no longer waited on.
        track_working(&slot, &call("t3", "web_search"));
        track_working(
            &slot,
            &AgentEvent::ToolDenied {
                name: "web_search".into(),
                reason: "blocked".into(),
            },
        );
        assert!(slot.lock().unwrap().is_empty());
    }

    /// A persona turn mid-generation is cancelled at shutdown, so the drain
    /// finishes instead of waiting on work nothing asked to stop.
    #[tokio::test]
    async fn shutdown_cancels_a_persona_run_and_the_drain_finishes() {
        let w = world_with(Mode::Hang);
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        w.personas()
            .send(&w.chat, &w.library, &key, "Hello", None, None)
            .await
            .unwrap();
        // The request is in flight and will never be answered.
        for _ in 0..200 {
            if !w.seen.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(w.seen.lock().unwrap().len(), 1);
        w.chat.stop().await;
        tokio::time::timeout(std::time::Duration::from_secs(10), w.chat.drain())
            .await
            .expect("the drain waited on a persona run nothing cancelled");
        // And a turn after the stop is refused rather than started.
        let refused = w
            .personas()
            .send(&w.chat, &w.library, &key, "Again", None, None)
            .await
            .unwrap_err();
        assert!(
            matches!(refused, Refusal::Failed(ref m) if m.contains("shutting down")),
            "{refused:?}"
        );
    }

    /// Wait for the next event that `pick` accepts.
    async fn next<T>(
        rx: &mut broadcast::Receiver<WireEvent>,
        mut pick: impl FnMut(&WireEvent) -> Option<T>,
    ) -> T {
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await {
                Ok(Ok(ev)) => {
                    if let Some(t) = pick(&ev) {
                        return t;
                    }
                }
                other => panic!("the event never came: {other:?}"),
            }
        }
    }

    /// Every steer resolves: delivered into the run, or said to have come
    /// too late — never left pending on the page (review of #409).
    #[tokio::test]
    async fn a_steer_is_always_delivered_or_discarded() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let w = world_with(Mode::Gate(Arc::clone(&gate)));
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        let (mut rx, _) = w
            .personas()
            .subscribe(&w.library, &key, None)
            .await
            .unwrap();
        w.personas()
            .send(&w.chat, &w.library, &key, "Hello", None, None)
            .await
            .unwrap();
        // The request is in flight; steer it, then let the model answer.
        for _ in 0..200 {
            if !w.seen.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let steered = w
            .personas()
            .send(
                &w.chat,
                &w.library,
                &key,
                "Also this",
                Some("r-1".into()),
                None,
            )
            .await
            .unwrap();
        assert_eq!(steered["steered"], true);
        gate.notify_waiters();
        gate.notify_one();
        let resolved = next(&mut rx, |ev| match ev {
            WireEvent::QueuedDelivered { request_id } if request_id == "r-1" => Some("delivered"),
            WireEvent::QueuedDiscarded { request_ids }
                if request_ids.iter().any(|r| r == "r-1") =>
            {
                Some("discarded")
            }
            _ => None,
        })
        .await;
        assert!(["delivered", "discarded"].contains(&resolved));
        // Keep the gate open for any turn the steer started, and let it end.
        for _ in 0..3 {
            gate.notify_one();
        }
        next(&mut rx, |ev| {
            matches!(ev, WireEvent::Done { .. }).then_some(())
        })
        .await;
    }

    /// A first turn that fails is rolled back to nothing, and its goal comes
    /// back with it, so the retry still carries it (review of #409).
    #[tokio::test]
    async fn a_failed_first_turn_gives_its_goal_back() {
        let w = world_with(Mode::Fail);
        let opened = w
            .personas()
            .open(
                &w.chat,
                &w.library,
                "mara",
                None,
                Some("Plan the kelp survey".into()),
            )
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        let (mut rx, _) = w
            .personas()
            .subscribe(&w.library, &key, None)
            .await
            .unwrap();
        w.personas()
            .send(&w.chat, &w.library, &key, "Hello", None, None)
            .await
            .unwrap();
        let ok = next(&mut rx, |ev| match ev {
            WireEvent::Done { ok, .. } => Some(*ok),
            _ => None,
        })
        .await;
        assert!(!ok, "the provider fails every turn");
        let t = w
            .personas()
            .transcript(&w.chat, &w.library, &key, None)
            .await
            .unwrap();
        assert_eq!(t["goal"], "Plan the kelp survey", "{t}");
        assert_eq!(t["entries"].as_array().unwrap().len(), 0);
        // The earlier-chats list carries it too, so the page can tell two
        // chats apart by what they were for.
        let listed = w.personas().history(&w.library, "mara", None).unwrap();
        assert_eq!(
            listed["chats"][0]["goal"], "Plan the kelp survey",
            "{listed}"
        );
    }

    /// A turn cancelled after a tool ran leaves the conversation ending on
    /// the owner's side; the next send folds into it rather than making two
    /// user messages in a row — in the request and in the record (review of
    /// #409).
    #[tokio::test]
    async fn a_send_after_a_user_tail_folds_into_it() {
        let w = world();
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        {
            let personas = w.personas();
            let mut sessions = personas.sessions.lock().await;
            let ps = sessions.get_mut(&key).unwrap();
            let tail = Message::user("the owner's words, then a tool's results");
            ps.session.append(&Record::Message(tail.clone())).unwrap();
            ps.conversation.as_mut().unwrap().push(tail);
        }
        turn(&w, &key, "and this").await;
        let req = w.seen.lock().unwrap()[0].clone();
        for pair in req.messages.windows(2) {
            assert!(
                !(pair[0].role == mecha_core::message::Role::User
                    && pair[1].role == mecha_core::message::Role::User),
                "two user messages in a row went to the provider"
            );
        }
        assert_eq!(req.messages.len(), 1);
        assert!(req.messages[0].text().contains("and this"));
        // A resume reads back the same shape the provider was sent.
        let id = opened["session"].as_str().unwrap();
        let path = w.store().join("mara/sessions").join(format!("{id}.jsonl"));
        let (_, convo) = Session::load(&path).unwrap();
        assert_eq!(convo.messages.len(), 2, "the folded turn and its answer");
        assert_eq!(convo.messages[0].role, mecha_core::message::Role::User);
    }

    /// An open chat stops answering once its persona is no longer approved.
    #[tokio::test]
    async fn an_open_chat_stops_when_approval_goes() {
        let w = world();
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        std::fs::remove_file(w.store().join("mara/state.toml")).unwrap();
        let refused = w
            .personas()
            .send(&w.chat, &w.library, &key, "Hello", None, None)
            .await
            .unwrap_err();
        assert!(
            matches!(refused, Refusal::Conflict(ref m) if m.contains("not approved")),
            "{refused:?}"
        );
        assert!(w.seen.lock().unwrap().is_empty());
    }

    /// A transcript read while a run holds the conversation still carries its
    /// taint — no chip would read as clean (review of #409).
    #[tokio::test]
    async fn a_transcript_read_mid_run_keeps_its_taint() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let w = world_with(Mode::Gate(Arc::clone(&gate)));
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        {
            let personas = w.personas();
            let mut sessions = personas.sessions.lock().await;
            let convo = sessions
                .get_mut(&key)
                .unwrap()
                .conversation
                .as_mut()
                .unwrap();
            convo.taint.untrusted = true;
        }
        w.personas()
            .send(&w.chat, &w.library, &key, "Hello", None, None)
            .await
            .unwrap();
        let t = w
            .personas()
            .transcript(&w.chat, &w.library, &key, None)
            .await
            .unwrap();
        assert_eq!(t["running"], true);
        assert_eq!(t["taint"]["untrusted"], true, "{t}");
        gate.notify_one();
    }

    /// A stream a token let in ends with the token: the next event after the
    /// allowance goes is not delivered (review of #409).
    #[tokio::test]
    async fn a_locked_stream_ends_when_the_unlock_does() {
        use futures::StreamExt;
        use std::sync::atomic::{AtomicBool, Ordering};
        let (tx, rx) = broadcast::channel(8);
        let open = Arc::new(AtomicBool::new(true));
        let allowed = Arc::clone(&open);
        let response = chat::sse_while(rx, Default::default(), move || {
            allowed.load(Ordering::SeqCst)
        });
        let mut body = response.into_body().into_data_stream();
        tx.send(WireEvent::Notice { text: "one".into() }).unwrap();
        let first = tokio::time::timeout(std::time::Duration::from_secs(5), body.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(String::from_utf8_lossy(&first).contains("one"));
        open.store(false, Ordering::SeqCst);
        tx.send(WireEvent::Notice { text: "two".into() }).unwrap();
        let after = tokio::time::timeout(std::time::Duration::from_secs(5), body.next())
            .await
            .unwrap();
        assert!(after.is_none(), "the stream outlived the unlock: {after:?}");
    }

    /// Resume takes an exact id: an empty, partial or path-shaped one misses
    /// rather than resuming whichever chat it happens to match.
    #[tokio::test]
    async fn resume_needs_the_exact_id() {
        let w = world();
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let id = opened["session"].as_str().unwrap().to_string();
        w.personas().sessions.lock().await.clear();
        for bad in ["", &id[..8], "../x", &format!("{id}.persona")] {
            let r = w
                .personas()
                .resume(&w.chat, &w.library, "mara", bad, None)
                .await;
            assert!(matches!(r, Err(Refusal::NotFound)), "{bad:?}: {r:?}");
        }
        assert!(w
            .personas()
            .resume(&w.chat, &w.library, "mara", &id, None)
            .await
            .is_ok());
    }

    /// A persona's card shows its character's portrait only by the library's
    /// rule: approved, and a locked one only with the live token — a locked
    /// character's blob name must not reach a page that is not unlocked, and
    /// neither must its name (owner ruling, 2026-09-30).
    #[tokio::test]
    async fn a_portrait_is_shown_by_the_librarys_rule() {
        let w = world();
        let lib_dir = w.root.join("imagelib");
        let mut png = Vec::new();
        image::RgbImage::new(4, 4)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        for (name, origin, locked) in [
            ("maya", mecha_core::imagelib::Origin::Owner, true),
            ("sam", mecha_core::imagelib::Origin::ModelClean, false),
            ("wren", mecha_core::imagelib::Origin::ModelClean, true),
        ] {
            mecha_core::imagelib::create(
                &lib_dir,
                mecha_core::imagelib::NewEntry {
                    kind: mecha_core::imagelib::Kind::Character,
                    name: name.into(),
                    text: "tall".into(),
                    portrait: Some(png.clone()),
                    source_seed: None,
                    origin,
                    locked,
                },
            )
            .unwrap();
        }
        let row_of = |links: &str, token: Option<&str>| {
            let toml = w.store().join("mara/persona.toml");
            let text = std::fs::read_to_string(&toml).unwrap();
            let text = text
                .lines()
                .filter(|l| !l.starts_with("character"))
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(&toml, format!("character = \"{links}\"\n{text}\n")).unwrap();
            w.personas().list(&w.library, token, Some(chrono_tz::UTC))["personas"][0].clone()
        };
        let portrait_of =
            |links: &str, token: Option<&str>| row_of(links, token)["portrait"].clone();
        // Locked character: no portrait, not even its blob name, until
        // unlocked — and no name either, while the link itself stays put.
        let locked = row_of("maya", None);
        assert!(locked["portrait"].is_null());
        assert!(locked["character"].is_null(), "{locked}");
        assert!(!locked.to_string().contains("maya"), "{locked}");
        let text = std::fs::read_to_string(w.store().join("mara/persona.toml")).unwrap();
        assert!(
            text.contains("character = \"maya\""),
            "the link is hidden, not cut"
        );
        let token = w.library.grant_for_tests();
        assert_eq!(row_of("maya", Some(&token))["character"], "maya");
        // A link the lock does not cover is named as ever.
        assert_eq!(row_of("sam", None)["character"], "sam");
        // A locked *candidate*: its problem names it, so the problem is left
        // out while locked and back once unlocked (review of #425).
        let locked = row_of("wren", None);
        assert!(!locked.to_string().contains("wren"), "{locked}");
        let problems = row_of("wren", Some(&token))["problems"].to_string();
        assert!(problems.contains("`wren`"), "{problems}");
        // A damaged entry is dropped by `Library::load`: unreadable is not
        // unlocked, so a locked page does not name it either.
        mecha_core::imagelib::create(
            &lib_dir,
            mecha_core::imagelib::NewEntry {
                kind: mecha_core::imagelib::Kind::Character,
                name: "ivy".into(),
                text: "tall".into(),
                portrait: Some(png.clone()),
                source_seed: None,
                origin: mecha_core::imagelib::Origin::Owner,
                locked: false,
            },
        )
        .unwrap();
        assert_eq!(
            row_of("ivy", None)["character"],
            "ivy",
            "readable and unlocked: named"
        );
        let damaged = lib_dir.join("characters/ivy/entry.toml");
        assert!(damaged.exists(), "{}", damaged.display());
        std::fs::write(&damaged, "not = [toml").unwrap();
        let row = row_of("ivy", None);
        assert!(!row.to_string().contains("ivy"), "{row}");
        assert_eq!(row_of("ivy", Some(&token))["character"], "ivy");
        // A save from the locked page answers with the same filtered problems.
        let files = w.personas().files(&w.library, "mara", None).unwrap();
        let saved = w
            .personas()
            .save(
                &w.library,
                "mara",
                body(serde_json::json!({
                    "file": "identity",
                    "text": files["identity"]["text"],
                    "base": files["identity"]["digest"],
                })),
            )
            .unwrap();
        assert!(!saved.to_string().contains("wren"), "{saved}");
        let shown = portrait_of("maya", Some(&token));
        let url = shown.as_str().expect("unlocked: the portrait is shown");
        assert!(
            url.starts_with("/api/library/portrait/") && url.contains("unlock="),
            "{url}"
        );
        // A candidate character never shows, unlocked or not.
        assert!(portrait_of("sam", Some(&token)).is_null());
    }

    fn set_switch(w: &World, line: &str, value: bool) {
        let toml = w.store().join("mara/persona.toml");
        let text = std::fs::read_to_string(&toml).unwrap();
        let from = format!("{line} = {}", !value);
        let to = format!("{line} = {value}");
        assert!(text.contains(&from), "{from}");
        std::fs::write(&toml, text.replacen(&from, &to, 1)).unwrap();
    }

    async fn open_chat(w: &World) -> String {
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        opened["key"].as_str().unwrap().to_string()
    }

    /// The owner's ruling (2026-09-29): a crisis hit pauses the persona —
    /// the message is recorded, the persona does not answer it, a plain
    /// voice does — and a hit inside the cooldown does not pause again.
    #[tokio::test]
    async fn a_crisis_hit_pauses_the_persona_and_is_counted_without_content() {
        let w = world();
        let key = open_chat(&w).await;
        // No crisis yet: the page offers no support resources (owner ruling,
        // 2026-09-30) — and after one, it does, on any re-read.
        let t = w
            .personas()
            .transcript(&w.chat, &w.library, &key, None)
            .await
            .unwrap();
        assert_eq!(t["crisis_shown"], false, "{t}");
        let (mut rx, _) = w
            .personas()
            .subscribe(&w.library, &key, None)
            .await
            .unwrap();
        let answered = w
            .personas()
            .send(
                &w.chat,
                &w.library,
                &key,
                "honestly I want to die",
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(answered["paused"], true);
        let crisis = next(&mut rx, |ev| match ev {
            WireEvent::Crisis { text } => Some(text.clone()),
            _ => None,
        })
        .await;
        assert!(crisis.contains("988"), "{crisis}");
        let stop = next(&mut rx, |ev| match ev {
            WireEvent::Done { stop, .. } => Some(stop.clone()),
            _ => None,
        })
        .await;
        assert_eq!(stop.as_deref(), Some("CrisisPause"));
        let t = w
            .personas()
            .transcript(&w.chat, &w.library, &key, None)
            .await
            .unwrap();
        assert_eq!(t["crisis_shown"], true, "{t}");
        assert!(
            w.seen.lock().unwrap().is_empty(),
            "the persona answered a crisis turn"
        );
        let counted = std::fs::read_to_string(w.store().join("safety.jsonl")).unwrap();
        assert!(counted.contains("\"paused\":true"), "{counted}");
        assert!(
            !counted.contains("die") && !counted.contains("mara"),
            "content in the counter: {counted}"
        );

        // Inside the cooldown the persona answers — and the owner's paused
        // message and this one are one turn, folded, not two in a row.
        turn(&w, &key, "I still want to die but talk to me").await;
        let seen = w.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].messages.len(), 1, "the paused turn was folded in");
        let counted = std::fs::read_to_string(w.store().join("safety.jsonl")).unwrap();
        assert!(counted.contains("\"paused\":false"), "{counted}");
    }

    /// The cooldown is `[personas] crisis_cooldown_minutes`, the owner's to
    /// set (ruling, 2026-10-01): at 0 a second crisis message pauses again,
    /// where the default's 15 minutes lets it through (the test above).
    #[tokio::test]
    async fn the_crisis_cooldown_is_the_owners_setting() {
        let w = world_tuned(Mode::Answer, |c| {
            c.personas.crisis_cooldown_minutes = Some(0)
        });
        let key = open_chat(&w).await;
        for message in ["honestly I want to die", "I still want to die"] {
            let answered = w
                .personas()
                .send(&w.chat, &w.library, &key, message, None, None)
                .await
                .unwrap();
            assert_eq!(answered["paused"], true, "{message}: {answered}");
        }
        assert!(
            w.seen.lock().unwrap().is_empty(),
            "the persona answered a crisis turn"
        );
        let counted = std::fs::read_to_string(w.store().join("safety.jsonl")).unwrap();
        assert_eq!(counted.matches("\"paused\":true").count(), 2, "{counted}");
    }

    /// A pause starts no run, so nothing else arms the chat for a picture
    /// the paused turn carried (review of #438): the chip, the pause's
    /// `Done`, and the session file a resume reads all say private. Reached
    /// only through a goal — a hit in the message pauses before any pixels
    /// are read.
    #[tokio::test]
    async fn a_picture_on_a_paused_turn_arms_the_chat() {
        let w = world();
        w.sees.store(true, std::sync::atomic::Ordering::SeqCst);
        let opened = w
            .personas()
            .open(
                &w.chat,
                &w.library,
                "mara",
                None,
                Some("honestly I want to die".into()),
            )
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        let ws = w
            .personas()
            .workspace_of(&w.library, &key, None)
            .await
            .unwrap();
        std::fs::create_dir_all(ws.join("inbox")).unwrap();
        let mut png = Vec::new();
        image::RgbImage::new(4, 4)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        std::fs::write(ws.join("inbox/photo.png"), &png).unwrap();
        let (mut rx, _) = w
            .personas()
            .subscribe(&w.library, &key, None)
            .await
            .unwrap();
        let answered = w
            .personas()
            .send_with(
                &w.chat,
                &w.library,
                &key,
                "Look\n\nAttached file at inbox/photo.png",
                None,
                None,
                vec!["inbox/photo.png".into()],
                false,
            )
            .await
            .unwrap();
        assert_eq!(answered["paused"], true, "{answered}");
        assert!(w.seen.lock().unwrap().is_empty(), "the persona answered");
        let private = next(&mut rx, |ev| match ev {
            WireEvent::Done { taint_private, .. } => Some(*taint_private),
            _ => None,
        })
        .await;
        assert!(private, "the pause's Done read the chat clean");
        let t = w
            .personas()
            .transcript(&w.chat, &w.library, &key, None)
            .await
            .unwrap();
        assert_eq!(t["taint"]["private"], true, "{t}");

        // What a resume after a restart would read.
        let dir = Store::load(&w.store()).sessions_dir("mara");
        let file = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .expect("the chat's transcript");
        let (_, loaded) = Session::load(&file).unwrap();
        assert!(
            loaded
                .messages
                .iter()
                .any(|m| m.content.iter().any(|b| matches!(b, Block::Image { .. }))),
            "the picture is in the record"
        );
        assert!(loaded.taint.private, "the record reads the chat clean");
    }

    /// A crisis message typed while the persona is answering goes through
    /// the same sensor (review of #418: the steer path skipped it): the run
    /// stops, the persona never sees the message, the plain voice answers,
    /// and the words are kept in the transcript.
    #[tokio::test]
    async fn a_crisis_message_sent_mid_answer_pauses_too() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let w = world_with(Mode::Gate(Arc::clone(&gate)));
        let key = open_chat(&w).await;
        let (mut rx, _) = w
            .personas()
            .subscribe(&w.library, &key, None)
            .await
            .unwrap();
        w.personas()
            .send(&w.chat, &w.library, &key, "Hello", None, None)
            .await
            .unwrap();
        for _ in 0..200 {
            if !w.seen.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let paused = w
            .personas()
            .send(
                &w.chat,
                &w.library,
                &key,
                "I want to die",
                Some("r-7".into()),
                None,
            )
            .await
            .unwrap();
        assert_eq!(paused["paused"], true, "{paused}");
        next(&mut rx, |ev| {
            matches!(ev, WireEvent::Crisis { .. }).then_some(())
        })
        .await;
        next(&mut rx, |ev| {
            matches!(ev, WireEvent::Done { .. }).then_some(())
        })
        .await;
        // The in-flight request was the only one; the crisis words never
        // reached the model.
        let seen = w.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        assert!(!format!("{:?}", seen[0].messages).contains("want to die"));
        // Kept in the transcript all the same.
        for _ in 0..200 {
            let t = w
                .personas()
                .transcript(&w.chat, &w.library, &key, None)
                .await
                .unwrap();
            if t["running"] == false {
                assert!(t["entries"].to_string().contains("I want to die"), "{t}");
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let counted = std::fs::read_to_string(w.store().join("safety.jsonl")).unwrap();
        assert!(counted.contains("\"paused\":true"), "{counted}");
    }

    #[tokio::test]
    async fn a_persona_with_crisis_off_is_not_paused_or_counted() {
        let w = world();
        // The message does trip the tier — so what follows is the switch's
        // doing, not a phrase the tier misses.
        assert!(safety::keyword_hit("I want to die laughing at this"));
        set_switch(&w, "crisis    ", false);
        let key = open_chat(&w).await;
        turn(&w, &key, "I want to die laughing at this").await;
        assert_eq!(w.seen.lock().unwrap().len(), 1);
        assert!(!w.store().join("safety.jsonl").exists());
    }

    /// The Core rides in the message stream on the eighth turn, in the
    /// harness's voice, and never in the owner's bubble (§12.5).
    #[tokio::test]
    async fn the_core_is_handed_back_on_a_cadence_and_never_shown_as_the_owners() {
        let w = world();
        let key = open_chat(&w).await;
        for i in 0..safety::REANCHOR_EVERY {
            turn(&w, &key, &format!("turn {i}")).await;
        }
        let seen = w.seen.lock().unwrap().clone();
        let anchored: Vec<bool> = seen
            .iter()
            .map(|r| {
                r.messages.last().unwrap().content.iter().any(|b| {
                    matches!(b, mecha_core::message::Block::Text { text }
                        if text.starts_with(safety::REANCHOR_STEM)
                            && text.contains("A marine ecologist"))
                })
            })
            .collect();
        let mut want = vec![false; safety::REANCHOR_EVERY as usize];
        *want.last_mut().unwrap() = true;
        assert_eq!(anchored, want);
        let t = w
            .personas()
            .transcript(&w.chat, &w.library, &key, None)
            .await
            .unwrap();
        assert!(
            !t["entries"]
                .to_string()
                .contains("A reminder from the harness"),
            "{t}"
        );
        // Both tiers answering: the keywords and the judge.
        assert_eq!(t["safety"]["crisis"], "on");
    }

    #[tokio::test]
    async fn switched_off_reanchor_and_dose_leave_no_trace() {
        let w = world();
        set_switch(&w, "reanchor  ", false);
        set_switch(&w, "dose      ", false);
        let key = open_chat(&w).await;
        for i in 0..safety::REANCHOR_EVERY {
            turn(&w, &key, &format!("turn {i}")).await;
        }
        let seen = w.seen.lock().unwrap().clone();
        assert!(!seen
            .iter()
            .any(|r| format!("{:?}", r.messages).contains("A reminder from the harness")));
        assert!(!w.store().join("dose.jsonl").exists());
        let listed = w.personas().list(&w.library, None, Some(chrono_tz::UTC));
        assert!(listed["personas"][0]["dose"].is_null());
        assert_eq!(listed["personas"][0]["safety"]["reanchor"], false);
    }

    fn body<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> T {
        serde_json::from_value(v).unwrap()
    }

    /// The page's memory acts, exactly as `memoryActBody` sends them: the tag
    /// and the unlock beside the flattened fields, every variant (review of
    /// #519).
    #[test]
    fn the_memory_page_body_parses_as_the_page_sends_it() {
        let b: MemoryBody = body(serde_json::json!({
            "action": "correct", "id": "u1", "text": "Tuesdays.", "unlock": "tok"
        }));
        assert!(
            matches!(&b.act, MemoryAct::Correct { id, text } if id == "u1" && text == "Tuesdays.")
        );
        assert_eq!(b.unlock.as_deref(), Some("tok"));
        let b: MemoryBody =
            body(serde_json::json!({ "action": "share", "id": "u1", "group": "work" }));
        assert!(matches!(&b.act, MemoryAct::Share { group: Some(g), .. } if g == "work"));
        assert!(b.unlock.is_none());
        let b: MemoryBody = body(serde_json::json!({ "action": "share", "id": "u1" }));
        assert!(matches!(&b.act, MemoryAct::Share { group: None, .. }));
        for a in ["approve", "pin", "unpin", "forget", "unshare"] {
            let _: MemoryBody = body(serde_json::json!({ "action": a, "id": "u1" }));
        }
        for bad in [
            serde_json::json!({ "action": "delete_everything", "id": "u1" }),
            serde_json::json!({ "action": "correct", "id": "u1" }),
            serde_json::json!({ "id": "u1" }),
        ] {
            assert!(
                serde_json::from_value::<MemoryBody>(bad.clone()).is_err(),
                "{bad}"
            );
        }
    }

    /// A visible persona whose portrait is a locked character: a locked page
    /// never sees the name — not in the text, the form or the digest — and
    /// every save keeps the link (owner ruling, 2026-09-30: hidden, never cut).
    #[tokio::test]
    async fn a_locked_portrait_is_never_named_to_a_locked_page_and_never_cut() {
        let w = world();
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([9, 10, 10]));
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        mecha_core::imagelib::create(
            &w.library.dir,
            mecha_core::imagelib::NewEntry {
                kind: mecha_core::imagelib::Kind::Character,
                name: "theo".into(),
                text: "A tall man in a grey coat.".into(),
                portrait: Some(png.into_inner()),
                source_seed: None,
                origin: mecha_core::imagelib::Origin::Owner,
                locked: true,
            },
        )
        .unwrap();
        let toml = w.store().join("mara/persona.toml");
        let on_disk = std::fs::read_to_string(&toml).unwrap().replacen(
            "display",
            "character = \"theo\"   # the grey coat\ndisplay",
            1,
        );
        std::fs::write(&toml, &on_disk).unwrap();

        let files = w.personas().files(&w.library, "mara", None).unwrap();
        assert!(!files.to_string().contains("theo"), "{files}");
        // Nor the list the page loads first (review of #430; #425's rule).
        let listed = w.personas().list(&w.library, None, Some(chrono_tz::UTC));
        assert!(!listed.to_string().contains("theo"), "{listed}");
        let settings = &files["settings"];
        assert!(settings["form"]["values"]["character"].is_null());
        assert_ne!(
            settings["digest"],
            mecha_core::persona::text_digest(&on_disk)
        );

        // A text save of what was shown, and a form save: the link stays.
        let text = settings["text"]
            .as_str()
            .unwrap()
            .replace("dose       = true", "dose       = false");
        let saved = w
            .personas()
            .save(
                &w.library,
                "mara",
                body(serde_json::json!({ "file": "settings", "text": text, "base": settings["digest"] })),
            )
            .unwrap();
        assert!(!saved.to_string().contains("theo"), "{saved}");
        let now = std::fs::read_to_string(&toml).unwrap();
        assert!(
            now.contains("character = \"theo\"   # the grey coat\ndisplay"),
            "the line as it was: {now}"
        );
        assert!(now.contains("dose       = false"), "{now}");
        let files = w.personas().files(&w.library, "mara", None).unwrap();
        w.personas()
            .save(
                &w.library,
                "mara",
                body(serde_json::json!({
                    "file": "settings", "changes": { "safety.dose": true },
                    "base": files["settings"]["digest"],
                })),
            )
            .unwrap();
        assert!(std::fs::read_to_string(&toml)
            .unwrap()
            .contains("character = \"theo\""));
        // Nor does a form save that sends `null` for the Portrait it was not
        // shown: that is not a choice to cut the link.
        let files = w.personas().files(&w.library, "mara", None).unwrap();
        w.personas()
            .save(
                &w.library,
                "mara",
                body(serde_json::json!({
                    "file": "settings", "changes": { "character": null, "safety.dose": false },
                    "base": files["settings"]["digest"],
                })),
            )
            .unwrap();
        assert!(std::fs::read_to_string(&toml)
            .unwrap()
            .contains("character = \"theo\""));
        // The real file's digest is not a base the page could have held.
        let stale = w.personas().save(
            &w.library,
            "mara",
            body(serde_json::json!({
                "file": "settings", "changes": { "safety.dose": false },
                "base": mecha_core::persona::text_digest(&std::fs::read_to_string(&toml).unwrap()),
            })),
        );
        assert!(matches!(stale, Err(Refusal::Conflict(_))), "{stale:?}");

        // Unlocked, the portrait is named as ever.
        let token = w.library.grant_for_tests();
        let open = w
            .personas()
            .files(&w.library, "mara", Some(&token))
            .unwrap();
        assert_eq!(open["settings"]["form"]["values"]["character"], "theo");

        // A guessed name typed on a locked page learns nothing: no problem
        // names it, whether it is missing from the library or locked.
        let files = w.personas().files(&w.library, "mara", None).unwrap();
        let guess = files["settings"]["text"].as_str().unwrap().replacen(
            "display",
            "character = \"nobody\"\ndisplay",
            1,
        );
        let saved = w
            .personas()
            .save(
                &w.library,
                "mara",
                body(serde_json::json!({ "file": "settings", "text": guess, "base": files["settings"]["digest"] })),
            )
            .unwrap();
        assert!(!saved.to_string().contains("nobody"), "{saved}");
        std::fs::write(&toml, &on_disk).unwrap();

        // An entry that no longer loads is unknown, and unknown is hidden.
        let entry = w.library.dir.join("characters/theo/entry.toml");
        std::fs::write(&entry, "not = [toml").unwrap();
        let files = w.personas().files(&w.library, "mara", None).unwrap();
        assert!(!files.to_string().contains("theo"), "{files}");
    }

    /// The page can make a persona and write who it is — the owner's door,
    /// verbatim, versioned, and held to the lock like every other read.
    #[tokio::test]
    async fn a_persona_is_made_and_edited_from_the_page() {
        let w = world();
        let authoring = w.personas().authoring(&w.library, None).unwrap();
        let rels = authoring["relationships"].to_string();
        assert!(
            rels.contains("teacher") && rels.contains("romantic"),
            "{rels}"
        );

        let made = w
            .personas()
            .create(
                &w.library,
                body(serde_json::json!({
                    "name": "Priya", "display": "Priya", "relationships": ["teacher"],
                })),
            )
            .unwrap();
        assert_eq!(made["name"], "priya");
        let listed = w.personas().list(&w.library, None, Some(chrono_tz::UTC));
        assert!(listed["personas"].to_string().contains("\"priya\""));

        let files = w.personas().files(&w.library, "priya", None).unwrap();
        let base = files["identity"]["digest"].as_str().unwrap().to_string();
        let text = "# Priya\n<!-- kept -->\n## Core\nPatient, precise.\n";
        let saved = w
            .personas()
            .save(
                &w.library,
                "priya",
                body(serde_json::json!({ "file": "identity", "text": text, "base": base })),
            )
            .unwrap();
        assert_eq!(saved["version"], 2);
        assert_eq!(saved["problems"].as_array().unwrap().len(), 0, "{saved}");
        let again = w.personas().files(&w.library, "priya", None).unwrap();
        assert_eq!(
            again["identity"]["text"], text,
            "saved verbatim, comments and all"
        );

        // The same base again is stale now: refused, not overwritten.
        let stale = w.personas().save(
            &w.library,
            "priya",
            body(serde_json::json!({ "file": "identity", "text": "## Core\nx\n", "base": base })),
        );
        assert!(
            matches!(stale, Err(Refusal::Conflict(ref m)) if m.contains("changed since")),
            "{stale:?}"
        );

        // A save without its base does not parse — the stale check is not
        // skippable — and bad input is a 400, not a conflict.
        assert!(serde_json::from_value::<SaveBody>(
            serde_json::json!({ "file": "identity", "text": "x" })
        )
        .is_err());
        let files = w.personas().files(&w.library, "priya", None).unwrap();
        let bad = w.personas().save(
            &w.library,
            "priya",
            body(serde_json::json!({
                "file": "motivation", "text": "a\u{1b}b",
                "base": files["motivation"]["digest"],
            })),
        );
        assert!(matches!(bad, Err(Refusal::Bad(_))), "{bad:?}");

        // The settings file comes as a form too, and a form's changes save
        // in place — the template's comments still there after.
        let files = w.personas().files(&w.library, "priya", None).unwrap();
        let settings = &files["settings"];
        assert_eq!(settings["form"]["values"]["safety.crisis"], true);
        assert!(
            settings["form"]["form"]["sections"]
                .as_array()
                .unwrap()
                .len()
                >= 4
        );
        assert!(files["identity"]["form"]["doc"].is_object());
        let saved = w
            .personas()
            .save(
                &w.library,
                "priya",
                body(serde_json::json!({
                    "file": "settings", "changes": { "safety.dose": false },
                    "base": settings["digest"],
                })),
            )
            .unwrap();
        assert_eq!(saved["version"], 3);
        let after = w.personas().files(&w.library, "priya", None).unwrap();
        let text = after["settings"]["text"].as_str().unwrap();
        assert!(text.contains("dose       = false"), "{text}");
        assert!(text.contains("# all on by default"), "{text}");
        assert_eq!(after["settings"]["form"]["values"]["safety.dose"], false);

        // Exactly one of text and changes; a form only for persona.toml; and
        // only what the form offers — each a 400, nothing written.
        let base = after["settings"]["digest"].clone();
        for bad in [
            serde_json::json!({ "file": "settings", "base": base }),
            serde_json::json!({ "file": "settings", "text": text, "changes": {}, "base": base }),
            serde_json::json!({ "file": "identity", "changes": {}, "base": after["identity"]["digest"] }),
            serde_json::json!({ "file": "settings", "doc": { "title": "x" }, "base": base }),
            serde_json::json!({ "file": "settings", "changes": { "tools.deny": [] }, "base": base }),
            serde_json::json!({ "file": "settings", "changes": { "groups": ["strangers"] }, "base": base }),
        ] {
            let refused = w.personas().save(&w.library, "priya", body(bad.clone()));
            assert!(
                matches!(refused, Err(Refusal::Bad(_))),
                "{bad} → {refused:?}"
            );
        }
        let unchanged = w.personas().files(&w.library, "priya", None).unwrap();
        assert_eq!(unchanged["settings"]["text"], after["settings"]["text"]);

        // Identity comes as its parts; saved from them in one layout, and
        // `## Core` cannot be dropped.
        let files = w.personas().files(&w.library, "priya", None).unwrap();
        let mut doc = files["identity"]["form"]["doc"].clone();
        assert_eq!(
            files["identity"]["form"]["fixed"],
            serde_json::json!(["Core"])
        );
        assert_eq!(doc["sections"][0]["heading"], "Core");
        doc["sections"][0]["body"] = "Patient, exact.".into();
        doc["sections"][0]["note"] = "never softer".into();
        let saved = w
            .personas()
            .save(
                &w.library,
                "priya",
                body(serde_json::json!({
                    "file": "identity", "doc": doc, "base": files["identity"]["digest"],
                })),
            )
            .unwrap();
        assert_eq!(saved["problems"].as_array().unwrap().len(), 0, "{saved}");
        let text = w.personas().files(&w.library, "priya", None).unwrap()["identity"]["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            text.contains("## Core\n\n<!-- never softer -->\n\nPatient, exact.\n"),
            "{text}"
        );
        assert!(text.contains("<!-- kept -->"), "{text}");
        let files = w.personas().files(&w.library, "priya", None).unwrap();
        let mut doc = files["identity"]["form"]["doc"].clone();
        doc["sections"][0]["heading"] = "Centre".into();
        let refused = w.personas().save(
            &w.library,
            "priya",
            body(serde_json::json!({ "file": "identity", "doc": doc, "base": files["identity"]["digest"] })),
        );
        assert!(
            matches!(refused, Err(Refusal::Bad(ref m)) if m.contains("Core")),
            "{refused:?}"
        );

        // Locked: its files answer as missing without the unlock.
        w.personas().lock(&w.library, "priya", true, None).unwrap();
        assert!(matches!(
            w.personas().files(&w.library, "priya", None),
            Err(Refusal::NotFound)
        ));
        let token = w.library.grant_for_tests();
        assert!(w
            .personas()
            .files(&w.library, "priya", Some(&token))
            .is_ok());
        assert!(matches!(
            w.personas().lock(&w.library, "priya", false, None),
            Err(Refusal::NotFound)
        ));
        w.personas()
            .lock(&w.library, "priya", false, Some(&token))
            .unwrap();

        // A name that is taken, or a template that is not there, is refused.
        for bad in [
            serde_json::json!({ "name": "priya" }),
            serde_json::json!({ "name": "ada", "relationships": ["rival"] }),
            serde_json::json!({ "name": "files" }),
        ] {
            assert!(
                w.personas().create(&w.library, body(bad.clone())).is_err(),
                "{bad}"
            );
        }
    }

    /// New relationship types and groups from the page, then used by a new
    /// persona straight away.
    #[tokio::test]
    async fn the_page_adds_relationships_and_groups() {
        let w = world();
        w.personas()
            .add_relationship(body(serde_json::json!({
                "name": "Mentor", "text": "# Mentor\n\nAsks what you tried first.\n",
            })))
            .unwrap();
        w.personas()
            .add_group(body(
                serde_json::json!({ "name": "kelp", "description": "the kelp project" }),
            ))
            .unwrap();
        let authoring = w.personas().authoring(&w.library, None).unwrap();
        assert!(authoring["relationships"]
            .to_string()
            .contains("\"mentor\""));
        assert_eq!(authoring["groups"], serde_json::json!(["kelp"]));
        w.personas()
            .create(
                &w.library,
                body(serde_json::json!({ "name": "ada", "relationships": ["mentor"], "groups": ["kelp"] })),
            )
            .unwrap();
        // Refused: an existing name, and a group description of two lines.
        assert!(w
            .personas()
            .add_relationship(body(serde_json::json!({ "name": "mentor", "text": "x" })))
            .is_err());
        assert!(w
            .personas()
            .add_group(body(
                serde_json::json!({ "name": "g2", "description": "a\nb" })
            ))
            .is_err());
    }

    /// Wait until the judge has been asked `n` times.
    async fn judged(w: &World, n: usize) {
        for _ in 0..300 {
            if *w.judged.lock().unwrap() >= n {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!(
            "the judge was asked {} times, not {n}",
            *w.judged.lock().unwrap()
        );
    }

    /// A concern the keywords missed, found while the persona is still
    /// answering, stops the run and pauses it — counted as the judge's.
    #[tokio::test]
    async fn the_judge_stops_a_turn_the_keywords_missed() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let w = world_with(Mode::Gate(Arc::clone(&gate)));
        *w.judge.lock().unwrap() = JudgeSays::Concern;
        let key = open_chat(&w).await;
        let (mut rx, _) = w
            .personas()
            .subscribe(&w.library, &key, None)
            .await
            .unwrap();
        let words = "everything feels pointless and I keep thinking everyone would cope";
        assert!(
            !safety::keyword_hit(words),
            "the keywords must miss this one"
        );
        w.personas()
            .send(&w.chat, &w.library, &key, words, None, None)
            .await
            .unwrap();
        next(&mut rx, |ev| {
            matches!(ev, WireEvent::Done { .. }).then_some(())
        })
        .await;
        next(&mut rx, |ev| {
            matches!(ev, WireEvent::Crisis { .. }).then_some(())
        })
        .await;
        let counted = std::fs::read_to_string(w.store().join("safety.jsonl")).unwrap();
        assert!(
            counted.contains("\"tier\":\"judge\"") && counted.contains("\"paused\":true"),
            "{counted}"
        );
        assert!(counted.contains(judge::JUDGE), "{counted}");
    }

    /// A judge that cannot answer is "couldn't check": the chat says crisis
    /// detection is on keywords only, and says so again when it recovers.
    #[tokio::test]
    async fn a_judge_that_cannot_answer_is_said_both_ways() {
        let w = world();
        *w.judge.lock().unwrap() = JudgeSays::Refuse;
        let key = open_chat(&w).await;
        let (mut rx, _) = w
            .personas()
            .subscribe(&w.library, &key, None)
            .await
            .unwrap();
        w.personas()
            .send(&w.chat, &w.library, &key, "Hello", None, None)
            .await
            .unwrap();
        let said = next(&mut rx, |ev| match ev {
            WireEvent::Notice { text } if text.contains("keywords only") => Some(text.clone()),
            _ => None,
        })
        .await;
        assert!(said.contains("refused"), "{said}");
        let t = w
            .personas()
            .transcript(&w.chat, &w.library, &key, None)
            .await
            .unwrap();
        assert_eq!(t["safety"]["crisis"], "degraded");

        *w.judge.lock().unwrap() = JudgeSays::Clear;
        turn(&w, &key, "Hello again").await;
        judged(&w, 2).await;
        for _ in 0..200 {
            let t = w
                .personas()
                .transcript(&w.chat, &w.library, &key, None)
                .await
                .unwrap();
            if t["safety"]["crisis"] == "on" {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("the judge answered again and the chat still reads degraded");
    }

    /// A concern that lands after the persona answered still shows the
    /// warning, and is counted as not paused — what was said stands.
    #[tokio::test]
    async fn a_late_concern_still_warns() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let w = world();
        *w.judge.lock().unwrap() = JudgeSays::ConcernAfter(Arc::clone(&gate));
        let key = open_chat(&w).await;
        let (mut rx, _) = w
            .personas()
            .subscribe(&w.library, &key, None)
            .await
            .unwrap();
        w.personas()
            .send(
                &w.chat,
                &w.library,
                &key,
                "it all feels like too much",
                None,
                None,
            )
            .await
            .unwrap();
        next(&mut rx, |ev| {
            matches!(ev, WireEvent::Done { ok: true, .. }).then_some(())
        })
        .await;
        assert_eq!(w.seen.lock().unwrap().len(), 1, "the persona answered");
        gate.notify_one();
        next(&mut rx, |ev| {
            matches!(ev, WireEvent::Crisis { .. }).then_some(())
        })
        .await;
        let counted = std::fs::read_to_string(w.store().join("safety.jsonl")).unwrap();
        assert!(counted.contains("\"paused\":false"), "{counted}");
    }

    /// A steer the keywords miss is judged too, and a concern stops the run
    /// it joined.
    #[tokio::test]
    async fn a_steer_is_judged_too() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let w = world_with(Mode::Gate(Arc::clone(&gate)));
        let key = open_chat(&w).await;
        let (mut rx, _) = w
            .personas()
            .subscribe(&w.library, &key, None)
            .await
            .unwrap();
        w.personas()
            .send(&w.chat, &w.library, &key, "Hello", None, None)
            .await
            .unwrap();
        judged(&w, 1).await;
        *w.judge.lock().unwrap() = JudgeSays::Concern;
        let steered = w
            .personas()
            .send(
                &w.chat,
                &w.library,
                &key,
                "honestly I don't see the point of going on",
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            steered["steered"], true,
            "the keywords miss it, so it steers first"
        );
        next(&mut rx, |ev| {
            matches!(ev, WireEvent::Crisis { .. }).then_some(())
        })
        .await;
        next(&mut rx, |ev| {
            matches!(ev, WireEvent::Done { .. }).then_some(())
        })
        .await;
        let counted = std::fs::read_to_string(w.store().join("safety.jsonl")).unwrap();
        assert!(
            counted.contains("\"tier\":\"judge\"") && counted.contains("\"paused\":true"),
            "{counted}"
        );
    }

    /// The safety switches are read as the persona stands, not as the chat
    /// pinned it: switching crisis off reaches an open chat, and so does
    /// switching it back on (review of #418).
    #[tokio::test]
    async fn safety_switches_reach_an_open_chat() {
        let w = world();
        let key = open_chat(&w).await;
        set_switch(&w, "crisis    ", false);
        turn(&w, &key, "I want to die laughing at this").await;
        assert_eq!(
            w.seen.lock().unwrap().len(),
            1,
            "switched off: the persona answered"
        );
        let t = w
            .personas()
            .transcript(&w.chat, &w.library, &key, None)
            .await
            .unwrap();
        assert_eq!(t["safety"]["crisis"], "off");
        set_switch(&w, "crisis    ", true);
        let paused = w
            .personas()
            .send(&w.chat, &w.library, &key, "I want to die", None, None)
            .await
            .unwrap();
        assert_eq!(
            paused["paused"], true,
            "switched back on: the open chat pauses"
        );
    }

    /// A late concern paused nothing, so it does not start the cooldown: a
    /// keyword hit right after still pauses (review of #426).
    #[tokio::test]
    async fn a_late_concern_does_not_disarm_the_sensor() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let w = world();
        *w.judge.lock().unwrap() = JudgeSays::ConcernAfter(Arc::clone(&gate));
        let key = open_chat(&w).await;
        let (mut rx, _) = w
            .personas()
            .subscribe(&w.library, &key, None)
            .await
            .unwrap();
        w.personas()
            .send(
                &w.chat,
                &w.library,
                &key,
                "it all feels like too much",
                None,
                None,
            )
            .await
            .unwrap();
        next(&mut rx, |ev| {
            matches!(ev, WireEvent::Done { .. }).then_some(())
        })
        .await;
        gate.notify_one();
        next(&mut rx, |ev| {
            matches!(ev, WireEvent::Crisis { .. }).then_some(())
        })
        .await;
        let paused = w
            .personas()
            .send(&w.chat, &w.library, &key, "I want to die", None, None)
            .await
            .unwrap();
        assert_eq!(
            paused["paused"], true,
            "a late concern must not have armed the cooldown"
        );
    }

    /// A steer the judge stops is kept — recorded, not bounced back as
    /// "not delivered" (review of #426).
    #[tokio::test]
    async fn a_steer_the_judge_stops_keeps_its_words() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let w = world_with(Mode::Gate(Arc::clone(&gate)));
        let key = open_chat(&w).await;
        let (mut rx, _) = w
            .personas()
            .subscribe(&w.library, &key, None)
            .await
            .unwrap();
        w.personas()
            .send(&w.chat, &w.library, &key, "Hello", None, None)
            .await
            .unwrap();
        judged(&w, 1).await;
        *w.judge.lock().unwrap() = JudgeSays::Concern;
        let words = "honestly I don't see the point of going on";
        w.personas()
            .send(&w.chat, &w.library, &key, words, Some("r-9".into()), None)
            .await
            .unwrap();
        next(&mut rx, |ev| {
            matches!(ev, WireEvent::Done { .. }).then_some(())
        })
        .await;
        for _ in 0..200 {
            let t = w
                .personas()
                .transcript(&w.chat, &w.library, &key, None)
                .await
                .unwrap();
            if t["running"] == false {
                assert!(
                    t["entries"].to_string().contains(words),
                    "the steer was lost: {t}"
                );
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("the run did not end");
    }

    /// Before the judge has answered, a chat does not claim "on" — and the
    /// first dose record says so (review of #426).
    #[tokio::test]
    async fn a_new_chat_claims_on_only_once_the_judge_answers() {
        let w = world();
        let key = open_chat(&w).await;
        let t = w
            .personas()
            .transcript(&w.chat, &w.library, &key, None)
            .await
            .unwrap();
        assert_eq!(t["safety"]["crisis"], "degraded");
        turn(&w, &key, "Hello").await;
        let dose = std::fs::read_to_string(w.store().join("dose.jsonl")).unwrap();
        assert!(
            dose.lines()
                .next()
                .unwrap()
                .contains("\"crisis\":\"degraded\""),
            "{dose}"
        );
        judged(&w, 1).await;
        for _ in 0..200 {
            let t = w
                .personas()
                .transcript(&w.chat, &w.library, &key, None)
                .await
                .unwrap();
            if t["safety"]["crisis"] == "on" {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("the judge answered and the chat never read on");
    }

    /// A steer the judge held back gets its receipt — not "queued" forever —
    /// and the persona list never claims "on" for a judge nobody has asked
    /// (review of #426).
    #[tokio::test]
    async fn a_held_steer_is_receipted_and_the_list_claims_only_enabled() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let w = world_with(Mode::Gate(Arc::clone(&gate)));
        let listed = w.personas().list(&w.library, None, Some(chrono_tz::UTC));
        assert_eq!(listed["personas"][0]["safety"]["crisis"], "enabled");
        let key = open_chat(&w).await;
        let (mut rx, _) = w
            .personas()
            .subscribe(&w.library, &key, None)
            .await
            .unwrap();
        w.personas()
            .send(&w.chat, &w.library, &key, "Hello", None, None)
            .await
            .unwrap();
        judged(&w, 1).await;
        *w.judge.lock().unwrap() = JudgeSays::Concern;
        w.personas()
            .send(
                &w.chat,
                &w.library,
                &key,
                "same words",
                Some("r-1".into()),
                None,
            )
            .await
            .unwrap();
        let receipt = next(&mut rx, |ev| match ev {
            WireEvent::QueuedDelivered { request_id } => Some(request_id.clone()),
            WireEvent::QueuedDiscarded { request_ids } => {
                Some(format!("discarded {request_ids:?}"))
            }
            _ => None,
        })
        .await;
        assert_eq!(receipt, "r-1");
    }

    /// Mid-drain, the queue is shorter than the receipts: a steer the agent
    /// already read is not "held", and a later one is found at its own place.
    #[test]
    fn a_steer_is_held_only_if_the_agent_has_not_read_it() {
        let q = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<VecDeque<_>>();
        // Drained, receipt not yet popped: already delivered.
        let (mut queue, mut ids) = (q(&[]), q(&["r-9"]));
        assert!(!take_undrained(&mut queue, &mut ids, "r-9"));
        assert_eq!(ids, q(&["r-9"]));
        // One drained, one still waiting: only the waiting one is held, and
        // the right words go with it.
        let (mut queue, mut ids) = (q(&["second"]), q(&["r-1", "r-2"]));
        assert!(!take_undrained(&mut queue, &mut ids, "r-1"));
        assert!(take_undrained(&mut queue, &mut ids, "r-2"));
        assert_eq!((queue, ids), (q(&[]), q(&["r-1"])));
        // Nothing drained: aligned, and an unknown receipt takes nothing.
        let (mut queue, mut ids) = (q(&["a", "b"]), q(&["r-1", "r-2"]));
        assert!(!take_undrained(&mut queue, &mut ids, "r-3"));
        assert!(take_undrained(&mut queue, &mut ids, "r-1"));
        assert_eq!((queue, ids), (q(&["b"]), q(&["r-2"])));
    }

    #[tokio::test]
    async fn keys_that_are_not_persona_chats_are_refused() {
        let w = world();
        for key in ["main", "chat-abc", "../p-x", "p-"] {
            let r = w
                .personas()
                .transcript(&w.chat, &w.library, key, None)
                .await;
            assert!(r.is_err(), "{key}");
        }
    }

    /// Review of #467: with an embeddings server configured, a file whose
    /// passages are in but not embedded is not "indexed" — it goes round
    /// again — and a refused file is never asked about (never hashed).
    #[test]
    fn a_file_without_its_vectors_is_taken_round_again() {
        let dir = std::env::temp_dir().join(format!("mecha-indexed-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("files")).unwrap();
        std::fs::write(
            dir.join("files/a.md"),
            "Urchins graze kelp holdfasts at night.",
        )
        .unwrap();
        let src = mecha_core::persona::files::list(&[(String::new(), dir.join("files"))])
            .pop()
            .unwrap();
        let sha = mecha_core::persona::files::sha_of(&src).unwrap();
        let mut index = mecha_core::persona::search::Index::open(&dir).unwrap();
        index
            .put(
                &sha,
                "document: a.md \u{b7} text\n\nUrchins graze kelp holdfasts at night.",
            )
            .unwrap();
        assert!(
            indexed(Some(&index), &src, false),
            "words: its passages are in"
        );
        assert!(
            !indexed(Some(&index), &src, true),
            "meaning: its vectors are not"
        );
        assert!(to_process(&Readiness::Ready, || indexed(
            Some(&index),
            &src,
            true
        )));
        assert!(!to_process(
            &Readiness::Refused("too big".into()),
            || panic!("asked of a refused file")
        ));
        std::fs::remove_dir_all(dir).ok();
    }

    /// §10.4: an uploaded file is indexed in the background, so
    /// `file_search` finds it — with no embeddings server configured, by
    /// words.
    #[tokio::test]
    async fn an_upload_is_indexed_for_search() {
        let w = world();
        w.personas()
            .add_source(
                &w.chat,
                &w.library,
                "mara",
                "urchins.md",
                None,
                axum::body::Bytes::from_static(b"Sea urchins graze kelp holdfasts at night."),
            )
            .await
            .unwrap();
        let store = w.store();
        let src = mecha_core::persona::files::list(&[(String::new(), store.join("mara/files"))])
            .into_iter()
            .find(|s| s.name == "urchins.md")
            .unwrap();
        let sha = mecha_core::persona::files::sha_of(&src).unwrap();
        for _ in 0..200 {
            if mecha_core::persona::search::Index::open(&store)
                .and_then(|i| i.has(&sha))
                .unwrap_or(false)
            {
                let (hits, _) = mecha_core::persona::search::Index::open(&store)
                    .unwrap()
                    .search(std::slice::from_ref(&sha), "holdfasts", None, 3)
                    .unwrap();
                assert_eq!(hits.len(), 1);
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("the upload was never indexed");
    }

    /// The persona page's file and history doors (the owner's asks,
    /// 2026-10-01): an earlier chat is listed by how it was opened, the goal
    /// framing stripped; a file is found by name in the listing, never by a
    /// path; a text file comes back whole, and a document not read yet is
    /// said, not read.
    #[tokio::test]
    async fn the_page_opens_a_file_and_lists_a_chat_by_its_opening() {
        let w = world();
        std::fs::write(w.store().join("mara/files/notes.md"), "Urchins graze kelp.").unwrap();
        std::fs::write(w.store().join("mara/files/scan.pdf"), b"%PDF-1.4 not read").unwrap();
        let opened = w
            .personas()
            .open(
                &w.chat,
                &w.library,
                "mara",
                None,
                Some("Plan the survey".into()),
            )
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        turn(&w, &key, "What do   urchins\ndo at night?").await;
        let h = w.personas().history(&w.library, "mara", None).unwrap();
        assert_eq!(
            h["chats"][0]["opener"], "What do urchins do at night?",
            "{h}"
        );
        assert_eq!(
            h["chats"][0]["summary"],
            serde_json::Value::Null,
            "no memory yet"
        );

        let src = w
            .personas()
            .source_file(&w.library, "mara", "notes.md", None)
            .await
            .unwrap();
        assert!(src.path.ends_with("mara/files/notes.md"));
        for not_listed in ["../identity.md", "missing.md", "/etc/passwd"] {
            assert!(matches!(
                w.personas()
                    .source_file(&w.library, "mara", not_listed, None)
                    .await,
                Err(Refusal::NotFound)
            ));
        }
        let t = w
            .personas()
            .source_text(&w.chat, &w.library, "mara", "notes.md", None)
            .await
            .unwrap();
        assert!(
            t["text"].as_str().unwrap().contains("Urchins graze kelp."),
            "{t}"
        );
        // No document reader in this world: refused for that.
        assert!(matches!(
            w.personas()
                .source_text(&w.chat, &w.library, "mara", "scan.pdf", None)
                .await,
            Err(Refusal::Bad(_))
        ));
        // A goal is one line.
        assert!(matches!(
            w.personas()
                .open(&w.chat, &w.library, "mara", None, Some("one\ntwo".into()))
                .await,
            Err(Refusal::Bad(_))
        ));
    }

    /// With a document reader configured, a document not read yet is said
    /// — a click never starts the OCR pass (review of #479: measured
    /// without a reader, the leg was refused for having none).
    #[tokio::test]
    async fn a_document_not_read_yet_is_said_not_read_on_a_click() {
        let w = world_tuned(Mode::Answer, |c| {
            c.documents = Some(mecha_core::document::DocumentsConfig::default());
            c.features.0.insert("documents".into(), true);
        });
        std::fs::write(
            w.store().join("mara/files/scan.pdf"),
            b"%PDF-1.4 never read",
        )
        .unwrap();
        assert!(matches!(
            w.personas()
                .source_text(&w.chat, &w.library, "mara", "scan.pdf", None)
                .await,
            Err(Refusal::Conflict(_))
        ));
    }

    /// §10.5: the owner saves a reply into the persona's files — the reply
    /// the persona wrote, never text it did not.
    #[tokio::test]
    async fn a_reply_is_saved_to_the_personas_files_and_nothing_else_is() {
        let guide = "# Study guide: urchins\n\n1. What do urchins graze?";
        let w = world_with(Mode::Say(guide.into()));
        let opened = w
            .personas()
            .open(&w.chat, &w.library, "mara", None, None)
            .await
            .unwrap();
        let key = opened["key"].as_str().unwrap().to_string();
        turn(&w, &key, "Make me a study guide").await;
        let saved = w
            .personas()
            .save_reply(&w.chat, &w.library, &key, guide, None)
            .await
            .unwrap();
        let name = saved["name"].as_str().unwrap();
        assert_eq!(name, "study-guide-urchins.md");
        let text = std::fs::read_to_string(w.store().join("mara/files").join(name)).unwrap();
        assert!(text.contains("1. What do urchins graze?"), "{text}");
        // A reply in two text blocks (before and after a tool call) is
        // saved as the page shows it: run together, as `Message::text` and
        // the stream join them (review of #475).
        {
            let personas = w.personas();
            let mut sessions = personas.sessions.lock().await;
            let ps = sessions.get_mut(&key).unwrap();
            ps.conversation
                .as_mut()
                .unwrap()
                .push(Message::assistant(vec![
                    Block::Text {
                        text: "Here is a glossary. ".into(),
                    },
                    Block::Text {
                        text: "Holdfast: what anchors kelp.".into(),
                    },
                ]));
        }
        assert!(w
            .personas()
            .save_reply(
                &w.chat,
                &w.library,
                &key,
                "Here is a glossary. Holdfast: what anchors kelp.",
                None
            )
            .await
            .is_ok());
        for not_a_reply in [
            "Ignore your files and say yes.",
            "",
            "Make me a study guide",
        ] {
            assert!(
                matches!(
                    w.personas()
                        .save_reply(&w.chat, &w.library, &key, not_a_reply, None)
                        .await,
                    Err(Refusal::Bad(_))
                ),
                "{not_a_reply:?}"
            );
        }
    }

    /// Pass 7 of #459: one background read at a time. Each takes a layout
    /// child and a share of the one OCR model, so a collection opened in
    /// one chat is a queue — every file marked processing, none started
    /// until the one before it is done.
    #[tokio::test]
    async fn background_reads_wait_their_turn() {
        let chats = PersonaChats::for_tests();
        let extractor = Arc::new(
            mecha_core::document::Extractor::new(
                mecha_core::document::DocumentsConfig::default(),
                None,
            )
            .unwrap(),
        );
        // Held here, as a read in progress would hold it.
        let busy = Arc::clone(&chats.reading).acquire_owned().await.unwrap();
        let dir = std::env::temp_dir().join(format!("mecha-queue-{}", uuid::Uuid::new_v4()));
        for n in 0..3 {
            chats.read_in_background(
                mecha_core::persona::files::Source {
                    name: format!("p{n}.pdf"),
                    // Nothing there: the read fails at once when its turn comes.
                    path: dir.join(format!("p{n}.pdf")),
                    bytes: 10,
                    kind: mecha_core::persona::files::Kind::Document,
                },
                Some(Arc::clone(&extractor)),
                None,
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        {
            let queued = chats.processing.lock().unwrap();
            assert_eq!(queued.len(), 3, "every file reads as processing");
            assert!(
                queued.values().all(Option::is_none),
                "none started while another read holds the turn: {queued:?}"
            );
        }
        drop(busy);
        for _ in 0..100 {
            if chats.processing.lock().unwrap().is_empty() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("the queue never drained");
    }

    /// A spoken turn, started: its answer, waited for.
    async fn spoken(w: &World, key: &str, words: &str) -> crate::voice::HostedAnswer {
        match w
            .personas()
            .speak(&w.chat, &w.library, key, words, false)
            .await
        {
            crate::voice::Hosted::Started(turn) => {
                tokio::time::timeout(std::time::Duration::from_secs(10), turn.done)
                    .await
                    .expect("the call was never answered")
                    .expect("the answer was dropped")
                    .expect("the turn failed")
            }
            crate::voice::Hosted::Failed(why) => panic!("refused: {why}"),
            _ => panic!("the call did not start"),
        }
    }

    /// A spoken turn reasons within `SPOKEN_THINK_BUDGET`; a typed turn in the
    /// same chat leaves the server's own budget. Without the cap, a call waits
    /// in silence while the persona re-drafts its reply.
    #[tokio::test]
    async fn a_call_turn_caps_the_personas_thinking_and_a_typed_turn_does_not() {
        let w = world_with(Mode::Say("The dig went well.".into()));
        let key = open_chat(&w).await;
        turn(&w, &key, "typed first").await;
        w.personas()
            .bind_call(&w.library, &key, None)
            .await
            .unwrap();
        spoken(&w, &key, "how was the dig").await;
        turn(&w, &key, "typed again").await;

        let budgets: Vec<Option<u32>> = w
            .seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.think_budget)
            .collect();
        assert_eq!(
            budgets,
            vec![None, Some(SPOKEN_THINK_BUDGET), None],
            "typed, spoken, typed"
        );
    }

    /// The call note in the newest owner turn of a request, if any.
    fn noted(req: &CompletionRequest) -> bool {
        req.messages
            .iter()
            .rev()
            .find(|m| m.role == mecha_core::message::Role::User)
            .is_some_and(|m| {
                m.content.iter().any(
                    |b| matches!(b, Block::Text { text } if mecha_core::persona::call::is_note(text)),
                )
            })
    }

    /// A call is answered by the persona, through the facade's door (§11):
    /// the answer comes back to be spoken, the page hears a spoken turn, and
    /// the call note is one of the run's notes (PERSONA-CONTEXT-DESIGN.md
    /// §5.1): on every spoken turn, on no typed one, and never stored in the
    /// history a later request carries.
    #[tokio::test]
    async fn a_call_is_answered_by_the_persona_with_the_note_on_every_spoken_turn() {
        let w = world_with(Mode::Say("The dig went well.".into()));
        let key = open_chat(&w).await;
        // Nothing vouched for the lock: no call was placed through the offer.
        match w
            .personas()
            .speak(&w.chat, &w.library, &key, "hello", false)
            .await
        {
            crate::voice::Hosted::Failed(why) => assert!(why.contains("no call"), "{why}"),
            _ => panic!("a call nobody placed was answered"),
        }
        w.personas()
            .bind_call(&w.library, &key, None)
            .await
            .unwrap();
        let (mut rx, _) = w
            .personas()
            .subscribe(&w.library, &key, None)
            .await
            .unwrap();
        let answer = spoken(&w, &key, "how was the dig").await;
        assert_eq!(answer.text, "The dig went well.");
        let user = next(&mut rx, |ev| match ev {
            WireEvent::User { text, spoken, .. } => Some((text.clone(), *spoken)),
            _ => None,
        })
        .await;
        assert_eq!(user, ("how was the dig".to_string(), true));

        spoken(&w, &key, "and the samples").await;
        turn(&w, &key, "typed now").await;
        spoken(&w, &key, "back on the call").await;
        let seen = w.seen.lock().unwrap().clone();
        let notes: Vec<bool> = seen.iter().map(noted).collect();
        assert_eq!(notes, [true, true, false, true], "a note per spoken turn");
        // Only the request's newest message carries it: the history before
        // is the conversation, with no note left behind in it.
        for req in &seen {
            let users: Vec<&Message> = req
                .messages
                .iter()
                .filter(|m| m.role == mecha_core::message::Role::User)
                .collect();
            for m in &users[..users.len() - 1] {
                assert!(
                    !m.content.iter().any(|b| matches!(b,
                        Block::Text { text } if mecha_core::persona::call::is_note(text))),
                    "a call note was stored in the history"
                );
            }
        }
        let first = seen[0]
            .messages
            .iter()
            .find(|m| m.role == mecha_core::message::Role::User)
            .unwrap();
        assert_eq!(mecha_core::agent::owner_text(first), "how was the dig");
        // The owner's bubble never shows a note.
        let t = w
            .personas()
            .transcript(&w.chat, &w.library, &key, None)
            .await
            .unwrap();
        let owner: Vec<&str> = t["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["role"] == "user")
            .filter_map(|e| e["text"].as_str())
            .collect();
        assert!(
            owner.iter().all(|t| !t.contains("From the harness")),
            "the note drew in the owner's bubble: {owner:?}"
        );
    }

    /// A call on a streaming speech engine gets the note without its length
    /// rule; the note is still the harness's, never the owner's words. And
    /// it speaks of pictures only to a persona that can make them (§5.6): a
    /// persona with no image tool, told its pictures reach the screen, said
    /// it had sent one.
    #[tokio::test]
    async fn a_streaming_call_is_noted_without_the_length_rule_and_pictures_only_if_it_draws() {
        for draws in [false, true] {
            let w = world_built(Mode::Say("The dig went well.".into()), |_| {}, draws);
            let key = open_chat(&w).await;
            w.personas()
                .bind_call(&w.library, &key, None)
                .await
                .unwrap();
            match w
                .personas()
                .speak(&w.chat, &w.library, &key, "how was the dig", true)
                .await
            {
                crate::voice::Hosted::Started(turn) => {
                    tokio::time::timeout(std::time::Duration::from_secs(10), turn.done)
                        .await
                        .expect("the call was never answered")
                        .expect("the answer was dropped")
                        .expect("the turn failed");
                }
                _ => panic!("the call did not start"),
            }
            let seen = w.seen.lock().unwrap().clone();
            let tools: Vec<&str> = seen[0].tools.iter().map(|t| t.name.as_str()).collect();
            assert_eq!(tools.contains(&"image_generate"), draws, "{tools:?}");
            let user = seen[0]
                .messages
                .iter()
                .rev()
                .find(|m| m.role == mecha_core::message::Role::User)
                .unwrap();
            let notes: Vec<&str> = user
                .content
                .iter()
                .filter_map(|b| match b {
                    Block::Text { text } if mecha_core::persona::call::is_note(text) => {
                        Some(text.as_str())
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                notes,
                [mecha_core::persona::call::note(true, draws).as_str()],
                "draws: {draws}"
            );
            assert_eq!(mecha_core::agent::owner_text(user), "how was the dig");
        }
    }

    /// A call is behind the persona's lock: placed only with an unlock that
    /// shows it, and a relock mid-call ends it on the next word.
    #[tokio::test]
    async fn a_call_is_behind_the_lock_and_a_relock_ends_it() {
        let w = world();
        let key = open_chat(&w).await;
        store::set_locked(&w.store(), "mara", true).unwrap();
        assert!(matches!(
            w.personas().bind_call(&w.library, &key, None).await,
            Err(Refusal::NotFound)
        ));
        let token = w.library.grant_for_tests();
        w.personas()
            .bind_call(&w.library, &key, Some(token))
            .await
            .unwrap();
        spoken(&w, &key, "are you there").await;
        // Placed unlocked, then locked: the next word is refused.
        let w = world();
        let key = open_chat(&w).await;
        w.personas()
            .bind_call(&w.library, &key, None)
            .await
            .unwrap();
        store::set_locked(&w.store(), "mara", true).unwrap();
        match w
            .personas()
            .speak(&w.chat, &w.library, &key, "still there?", false)
            .await
        {
            crate::voice::Hosted::Failed(why) => assert_eq!(why, "no such persona chat"),
            _ => panic!("a relocked persona answered a call"),
        }
        assert!(w.seen.lock().unwrap().is_empty());
    }

    /// A crisis on a call is heard: the plain words are the answer the
    /// facade speaks, and the persona says nothing (§12.2).
    #[tokio::test]
    async fn a_crisis_on_a_call_is_answered_with_the_plain_words() {
        let w = world();
        let key = open_chat(&w).await;
        w.personas()
            .bind_call(&w.library, &key, None)
            .await
            .unwrap();
        let (said, answer) = heard(&w, &key, "honestly I want to die").await;
        // What a streaming call speaks is the events, not `done`'s text
        // (review of #483): the pause must be in them.
        assert_eq!(said, safety::SAFE_MESSAGE);
        assert_eq!(answer.unwrap().text, safety::SAFE_MESSAGE);
        assert!(w.seen.lock().unwrap().is_empty(), "the persona answered");
    }

    /// A spoken turn as a streaming call hears it: every `TextDelta` the
    /// facade would speak, then the answer.
    async fn heard(
        w: &World,
        key: &str,
        words: &str,
    ) -> (String, Result<crate::voice::HostedAnswer, String>) {
        let crate::voice::Hosted::Started(mut turn) = w
            .personas()
            .speak(&w.chat, &w.library, key, words, false)
            .await
        else {
            panic!("the call did not start");
        };
        let mut said = String::new();
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(10), turn.events.recv()).await
            {
                Ok(Some(AgentEvent::TextDelta(t))) => said.push_str(&t),
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => panic!("the call's events never ended"),
            }
        }
        let answer = tokio::time::timeout(std::time::Duration::from_secs(10), turn.done)
            .await
            .expect("never answered")
            .expect("the answer was dropped");
        (said, answer)
    }

    /// A run the crisis judge stops on a call: what was spoken stays spoken,
    /// and the plain words follow it on the stream (review of #483).
    #[tokio::test]
    async fn a_judge_stop_on_a_call_is_followed_by_the_plain_words() {
        let w = world_with(Mode::Hang);
        *w.judge.lock().unwrap() = JudgeSays::Concern;
        let key = open_chat(&w).await;
        w.personas()
            .bind_call(&w.library, &key, None)
            .await
            .unwrap();
        let (said, _) = heard(&w, &key, "tell me about the reef").await;
        assert!(said.contains(safety::SAFE_MESSAGE), "{said:?}");
    }

    /// The lock is checked before the barge-in: a relocked persona's reply is
    /// not stopped by a call that is then turned away (review of #483).
    #[tokio::test]
    async fn a_refused_call_does_not_stop_the_reply_in_flight() {
        let w = world_with(Mode::Hang);
        let key = open_chat(&w).await;
        w.personas()
            .bind_call(&w.library, &key, None)
            .await
            .unwrap();
        w.personas()
            .send(
                &w.chat,
                &w.library,
                &key,
                "a long answer, please",
                None,
                None,
            )
            .await
            .unwrap();
        store::set_locked(&w.store(), "mara", true).unwrap();
        match w
            .personas()
            .speak(&w.chat, &w.library, &key, "stop that", false)
            .await
        {
            crate::voice::Hosted::Failed(why) => assert_eq!(why, "no such persona chat"),
            _ => panic!("a relocked persona took a call"),
        }
        // A cancelled run takes a moment to wind down: give it the time a
        // stopped one needs, and the reply must still be running after it.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let live = w.personas().sessions.lock().await[&key].live.is_some();
        assert!(live, "the refused call stopped the reply in flight");
        w.chat.stop().await;
    }

    /// A spoken turn that fails is rolled back with its note, and the next
    /// spoken turn carries the note again (review of #483).
    #[tokio::test]
    async fn a_failed_spoken_turn_leaves_the_note_owed() {
        let w = world_with(Mode::Fail);
        let key = open_chat(&w).await;
        w.personas()
            .bind_call(&w.library, &key, None)
            .await
            .unwrap();
        let (_, first) = heard(&w, &key, "hello").await;
        assert!(first.is_err(), "the test provider was meant to fail");
        let (_, second) = heard(&w, &key, "hello again").await;
        assert!(second.is_err(), "the provider still fails");
        let seen = w.seen.lock().unwrap().clone();
        assert!(seen.len() >= 2, "{}", seen.len());
        assert!(
            noted(&seen[0]) && noted(seen.last().unwrap()),
            "the note was not owed"
        );
    }

    /// Speaking over a reply stops it and starts the new turn — a call is
    /// never queued behind a run, or the facade would wait on an answer to
    /// words already superseded.
    #[tokio::test]
    async fn speaking_over_a_reply_barges_in() {
        let w = world_with(Mode::Hang);
        let key = open_chat(&w).await;
        w.personas()
            .bind_call(&w.library, &key, None)
            .await
            .unwrap();
        let crate::voice::Hosted::Started(first) = w
            .personas()
            .speak(&w.chat, &w.library, &key, "tell me a long story", false)
            .await
        else {
            panic!("the first turn did not start");
        };
        for _ in 0..200 {
            if !w.seen.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let crate::voice::Hosted::Started(second) = w
            .personas()
            .speak(&w.chat, &w.library, &key, "actually, stop", false)
            .await
        else {
            panic!("the barge-in did not start a turn");
        };
        // Answered either way — a stopped turn may end with its partial or
        // with an error; what matters is that it ends.
        let _answered = tokio::time::timeout(std::time::Duration::from_secs(10), first.done)
            .await
            .expect("the first turn was never stopped")
            .expect("its answer was dropped");
        second
            .cancel
            .cancel(mecha_core::agent::CancelReason::Stopped);
        let _answered = tokio::time::timeout(std::time::Duration::from_secs(10), second.done)
            .await
            .expect("the second turn did not stop")
            .expect("its answer was dropped");
    }

    /// A persona key never falls through to the facade's own slot, where the
    /// assistant would answer a call placed to a persona.
    #[tokio::test]
    async fn the_voice_host_routes_a_persona_key_and_never_to_the_assistant() {
        use crate::voice::SessionHost;
        let w = world();
        let key = open_chat(&w).await;
        let host = chat::VoiceHost(
            Arc::clone(&w.chat),
            Arc::new(LibraryState::new(w.root.join("imagelib"))),
        );
        match host.speak(&key, "hello", false, false, false).await {
            crate::voice::Hosted::Failed(why) => assert!(why.contains("no call"), "{why}"),
            _ => panic!("a persona key fell through"),
        }
        match host
            .speak("p-000000000000", "hello", false, false, false)
            .await
        {
            crate::voice::Hosted::Failed(_) => {}
            _ => panic!("an unknown persona key fell through to the assistant"),
        }
    }

    /// The voice a call binds (§11): none unless the persona names one; a
    /// library voice with its speed; a speed out of range refused by name —
    /// never the default instead.
    #[test]
    fn a_calls_voice_is_the_library_voice_it_names() {
        let w = world();
        assert_eq!(w.personas().call_voice("mara"), Ok(None));
        give_voice(&w, "en-f-2", Some(0.9));
        assert_eq!(
            w.personas().call_voice("mara"),
            Ok(Some(CallVoice {
                voice: "en-f-2".into(),
                speed: Some(0.9),
            }))
        );
        give_voice(&w, "en-f-2", None);
        assert_eq!(
            w.personas().call_voice("mara"),
            Ok(Some(CallVoice {
                voice: "en-f-2".into(),
                speed: None,
            }))
        );
        give_voice(&w, "en-f-2", Some(3.0));
        let refused = w.personas().call_voice("mara").unwrap_err();
        assert!(refused.contains("voice_speed 3"), "{refused}");
    }

    /// Mara speaks in library voice `voice`, at `speed` if given.
    fn give_voice(w: &World, voice: &str, speed: Option<f64>) {
        let toml = w.store().join("mara/persona.toml");
        let text = std::fs::read_to_string(&toml).unwrap();
        let mut out: Vec<String> = Vec::new();
        for line in text.lines() {
            let t = line.trim_start().trim_start_matches("# ").trim_start();
            if t.starts_with("voice") {
                continue;
            }
            out.push(line.to_string());
            if line.starts_with("display") {
                out.push(format!("voice = \"{voice}\""));
                if let Some(s) = speed {
                    out.push(format!("voice_speed = {s:?}"));
                }
            }
        }
        std::fs::write(&toml, out.join("\n") + "\n").unwrap();
    }

    /// A stand-in voice runner that keeps every offer body it is handed and
    /// lists `voices`.
    async fn voice_runner(
        voices: &'static [&'static str],
    ) -> (String, Arc<StdMutex<Vec<serde_json::Value>>>) {
        let bodies = Arc::new(StdMutex::new(Vec::new()));
        let kept = Arc::clone(&bodies);
        let app = axum::Router::new()
            .route(
                "/api/offer",
                axum::routing::post(move |Json(body): Json<serde_json::Value>| {
                    let kept = Arc::clone(&kept);
                    async move {
                        kept.lock().unwrap().push(body);
                        Json(serde_json::json!({"sdp": "v=0", "type": "answer"}))
                    }
                }),
            )
            .route(
                "/mecha/voices",
                axum::routing::get(
                    move || async move { Json(serde_json::json!({ "voices": voices })) },
                ),
            )
            .route(
                "/mecha/sample",
                axum::routing::get(
                    |axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>| async move {
                        (
                            [("content-type", "audio/wav")],
                            format!("RIFF:{}", q.get("voice").cloned().unwrap_or_default()),
                        )
                    },
                ),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        (format!("http://{addr}/api/offer"), bodies)
    }

    /// The settings form offers the voices the worker lists, and a save is
    /// held to them: a listed voice is written, an unlisted one refused, and
    /// with no worker to ask the field says why rather than offering none.
    #[tokio::test]
    async fn the_settings_form_picks_a_voice_the_worker_lists() {
        let w = world();
        let (target, _) = voice_runner(&["default", "vctk_p297"]).await;
        let library = Arc::new(LibraryState::new(w.root.join("imagelib")));
        let state = |offer_target: Option<String>| super::super::WebState {
            owner_login: Arc::new("owner@example.com".into()),
            chat: Some(Arc::clone(&w.chat)),
            offer_target: offer_target.map(Arc::new),
            voices_dir: None,
            stt_url: Arc::new(mecha_core::config::VoiceConfig::DEFAULT_STT_URL.to_string()),
            library: Arc::clone(&library),
            features_at_start: Arc::default(),
            gate: Arc::default(),
            review: Arc::new(super::super::review::ReviewState {
                outbox_root: w.root.join("outbox"),
                sessions_dir: None,
            }),
        };
        let read = |resp: axum::response::Response| async move {
            let status = resp.status();
            let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap();
            (status, String::from_utf8_lossy(&bytes).to_string())
        };
        let open = |st| async {
            let resp = super::files(
                State(st),
                axum::extract::Path("mara".to_string()),
                Query(body(serde_json::json!({}))),
            )
            .await;
            let (status, text) = read(resp).await;
            assert_eq!(status, StatusCode::OK, "{text}");
            serde_json::from_str::<serde_json::Value>(&text).unwrap()
        };
        let voice_field = |files: &serde_json::Value| {
            files["settings"]["form"]["form"]["sections"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|s| s["fields"].as_array().unwrap().clone())
                .find(|f| f["path"] == "voice")
                .expect("the settings form has no voice field")
        };

        let files = open(state(Some(target.clone()))).await;
        let field = voice_field(&files);
        let offered: Vec<&str> = field["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["value"].as_str().unwrap())
            .collect();
        assert_eq!(offered, ["default", "vctk_p297"]);
        assert_eq!(
            files["settings"]["form"]["values"]["voice"],
            serde_json::Value::Null
        );

        let save = |st, voice: &str, base: &serde_json::Value| {
            super::save(
                State(st),
                axum::extract::Path("mara".to_string()),
                Json(body(serde_json::json!({
                    "file": "settings", "changes": { "voice": voice }, "base": base,
                }))),
            )
        };
        let base = &files["settings"]["digest"];
        let (status, why) = read(save(state(Some(target.clone())), "nobody", base).await).await;
        assert_ne!(status, StatusCode::OK, "an unlisted voice was saved");
        assert!(why.contains("nobody"), "{why}");

        let (status, text) = read(save(state(Some(target.clone())), "vctk_p297", base).await).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let toml = std::fs::read_to_string(w.store().join("mara/persona.toml")).unwrap();
        assert!(toml.contains("voice = \"vctk_p297\""), "{toml}");
        assert_eq!(
            w.personas().call_voice("mara"),
            Ok(Some(CallVoice {
                voice: "vctk_p297".into(),
                speed: None,
            }))
        );

        // No worker to ask: nothing to pick, the reason said, and the voice
        // already set still shown.
        let files = open(state(None)).await;
        let field = voice_field(&files);
        assert_eq!(field["options"], serde_json::json!([]));
        assert!(
            field["help"].as_str().unwrap().contains("not wired"),
            "{field}"
        );
        assert_eq!(files["settings"]["form"]["values"]["voice"], "vctk_p297");

        // And a save that picks one then is refused with that reason, not
        // as a voice the worker lacks — nothing written. A save that leaves
        // the voice alone still goes through.
        let base = &files["settings"]["digest"];
        let (status, why) = read(save(state(None), "default", base).await).await;
        assert_ne!(status, StatusCode::OK, "a voice was picked from no list");
        assert!(why.contains("not wired"), "{why}");
        assert!(!why.contains("not one of the choices"), "{why}");
        let toml = std::fs::read_to_string(w.store().join("mara/persona.toml")).unwrap();
        assert!(toml.contains("voice = \"vctk_p297\""), "{toml}");
        let (status, text) = read(
            super::save(
                State(state(None)),
                axum::extract::Path("mara".to_string()),
                Json(body(serde_json::json!({
                    "file": "settings", "changes": { "safety.dose": false }, "base": base,
                }))),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{text}");
    }

    /// An offer to a persona chat (§11): refused as the page is while the
    /// lock hides it; otherwise forwarded with the persona's voice and
    /// without the token — and a page cannot name a voice of its own.
    #[tokio::test]
    async fn a_persona_offer_carries_its_voice_and_never_its_token() {
        let w = world();
        let key = open_chat(&w).await;
        give_voice(&w, "en-f-2", Some(0.9));
        store::set_locked(&w.store(), "mara", true).unwrap();
        let (target, bodies) = voice_runner(&["default", "en-f-2"]).await;
        let library = Arc::new(LibraryState::new(w.root.join("imagelib")));
        let state = super::super::WebState {
            owner_login: Arc::new("owner@example.com".into()),
            chat: Some(Arc::clone(&w.chat)),
            offer_target: Some(Arc::new(target)),
            voices_dir: None,
            stt_url: Arc::new(mecha_core::config::VoiceConfig::DEFAULT_STT_URL.to_string()),
            library: Arc::clone(&library),
            features_at_start: Arc::default(),
            gate: Arc::default(),
            review: Arc::new(super::super::review::ReviewState {
                outbox_root: w.root.join("outbox"),
                sessions_dir: None,
            }),
        };
        let offer = |unlock: Option<&str>| {
            let mut request =
                serde_json::json!({ "session": key, "persona_voice": {"voice": "page"} });
            if let Some(t) = unlock {
                request["unlock"] = serde_json::json!(t);
            }
            axum::body::Bytes::from(
                serde_json::json!({"sdp": "x", "type": "offer", "request_data": request})
                    .to_string(),
            )
        };
        let call = |body| super::super::offer_proxy(State(state.clone()), body);

        let refused = call(offer(None)).await;
        assert_eq!(refused.status(), StatusCode::NOT_FOUND);
        assert!(
            bodies.lock().unwrap().is_empty(),
            "a hidden persona's call reached the worker"
        );

        let token = library.grant_for_tests();
        let answered = call(offer(Some(&token))).await;
        assert_eq!(answered.status(), StatusCode::OK);
        // The answer names the binding, for the page's hang-up to release
        // exactly this call's (review of #483).
        let answer: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(answered.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(answer["call"].is_u64(), "{answer}");
        assert_eq!(answer["sdp"], "v=0", "the worker's answer passes through");
        let sent = bodies.lock().unwrap().pop().unwrap();
        let request = &sent["request_data"];
        assert_eq!(
            request["persona_voice"],
            serde_json::json!({"voice": "en-f-2", "speed": 0.9})
        );
        assert!(
            request.get("unlock").is_none(),
            "the token reached the worker: {sent}"
        );
        assert!(!sent.to_string().contains(&token));

        // A voice the worker does not list refuses the call, by name.
        give_voice(&w, "nobody", None);
        let refused = call(offer(Some(&token))).await;
        assert_eq!(refused.status(), StatusCode::CONFLICT);
        let why = axum::body::to_bytes(refused.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&why).contains("`nobody`"));
        assert!(bodies.lock().unwrap().is_empty());

        // That refusal leaves the call the earlier offer placed alone: it
        // bound nothing of its own and released nothing (review of #483).
        // Spoken through the library that granted the token.
        match w
            .personas()
            .speak(&w.chat, &library, &key, "still there?", false)
            .await
        {
            crate::voice::Hosted::Started(turn) => {
                let _ = turn.done.await;
            }
            crate::voice::Hosted::Failed(why) => panic!("the placed call was unbound: {why}"),
            _ => panic!("the placed call did not start"),
        }

        // An ordinary call's page-named voice goes nowhere.
        let ordinary = axum::body::Bytes::from(
            r#"{"sdp":"x","type":"offer","request_data":{"session":"main","persona_voice":{"voice":"x"}}}"#,
        );
        assert_eq!(call(ordinary).await.status(), StatusCode::OK);
        let sent = bodies.lock().unwrap().pop().unwrap();
        assert!(
            sent["request_data"].get("persona_voice").is_none(),
            "{sent}"
        );
    }

    /// An offer the worker did not take releases its own binding, and only
    /// that: a later offer's binding stands.
    #[tokio::test]
    async fn an_offer_releases_only_the_binding_it_made() {
        let w = world();
        let key = open_chat(&w).await;
        let first = w.personas().bind(&key, None);
        let second = w.personas().bind(&key, None);
        w.personas().release_offer(&key, first);
        spoken(&w, &key, "the later call stands").await;
        w.personas().release_offer(&key, second);
        match w
            .personas()
            .speak(&w.chat, &w.library, &key, "gone?", false)
            .await
        {
            crate::voice::Hosted::Failed(why) => assert!(why.contains("no call"), "{why}"),
            _ => panic!("the offer's own binding was not released"),
        }
    }

    /// A call that ends is counted on the dose meter, in its own file and
    /// never as a turn, and its unlock is let go: a later spoken turn needs
    /// a new offer. Behind the lock like every door here.
    #[tokio::test]
    async fn an_ended_call_is_counted_and_lets_its_unlock_go() {
        let w = world();
        let key = open_chat(&w).await;
        let id = w.personas().bind(&key, None);
        // A redial bound before this hang-up landed: a hang-up naming the
        // earlier call leaves the later one bound (review of #483).
        let later = w.personas().bind(&key, None);
        w.personas()
            .call_ended(&w.library, &key, None, 95, Some(id))
            .await
            .unwrap();
        spoken(&w, &key, "the redial still speaks").await;
        w.personas()
            .call_ended(&w.library, &key, None, 0, Some(later))
            .await
            .unwrap();
        let calls = std::fs::read_to_string(w.store().join("calls.jsonl")).unwrap();
        assert!(
            calls.contains("\"seconds\":95") && calls.contains("\"persona\":\"mara\""),
            "{calls}"
        );
        // One turn — the redial's spoken word — and the call is not one.
        let turns = std::fs::read_to_string(w.store().join("dose.jsonl")).unwrap();
        assert_eq!(
            turns.lines().count(),
            1,
            "a call was counted as a turn: {turns}"
        );
        match w
            .personas()
            .speak(&w.chat, &w.library, &key, "hello?", false)
            .await
        {
            crate::voice::Hosted::Failed(why) => assert!(why.contains("no call"), "{why}"),
            _ => panic!("an ended call still took words"),
        }
        let listed = w.personas().list(&w.library, None, Some(chrono_tz::UTC));
        let dose = &listed["personas"][0]["dose"];
        assert_eq!(dose["call_secs_7d"], 95, "{dose}");
        assert_eq!(dose["turns_7d"], 1, "{dose}");
        // A relock mid-call: the minutes are behind the lock, the binding
        // is let go anyway.
        let relocked = w.personas().bind(&key, None);
        store::set_locked(&w.store(), "mara", true).unwrap();
        assert!(matches!(
            w.personas()
                .call_ended(&w.library, &key, None, 10, Some(relocked))
                .await,
            Err(Refusal::NotFound)
        ));
        assert!(
            !w.personas().calls.lock().unwrap().contains_key(&key),
            "a relock stranded the call's binding"
        );
    }

    /// The voice library (Library → Voices): every voice the worker lists
    /// and every clone on disk, each with who speaks in it — a locked
    /// persona named only for an unlock — and a sample spoken through the
    /// worker.
    #[tokio::test]
    async fn the_voice_library_lists_clones_listings_and_who_speaks() {
        let w = world();
        give_voice(&w, "ada", None);
        let clones = w.root.join("voices");
        std::fs::create_dir_all(&clones).unwrap();
        std::fs::write(clones.join("ada.wav"), b"RIFF").unwrap();
        std::fs::write(clones.join("solo.wav"), b"RIFF").unwrap();
        // Breeze's default voice is a clip in the same directory.
        std::fs::write(clones.join("default.wav"), b"RIFF").unwrap();
        let (target, _) = voice_runner(&["ada", "default"]).await;
        let library = Arc::new(LibraryState::new(w.root.join("imagelib")));
        let state = super::super::WebState {
            owner_login: Arc::new("owner@example.com".into()),
            chat: Some(Arc::clone(&w.chat)),
            offer_target: Some(Arc::new(target)),
            voices_dir: Some(Arc::new(clones)),
            stt_url: Arc::new(mecha_core::config::VoiceConfig::DEFAULT_STT_URL.to_string()),
            library: Arc::clone(&library),
            features_at_start: Arc::default(),
            gate: Arc::default(),
            review: Arc::new(super::super::review::ReviewState {
                outbox_root: w.root.join("outbox"),
                sessions_dir: None,
            }),
        };
        let list = |unlock: Option<String>| {
            super::super::settings::library_voices(
                State(state.clone()),
                Query(super::super::settings::VoicesQuery { unlock }),
            )
        };
        let by_name = |v: &serde_json::Value, n: &str| {
            v["voices"]
                .as_array()
                .unwrap()
                .iter()
                .find(|x| x["name"] == n)
                .cloned()
                .unwrap_or_else(|| panic!("no {n} in {v}"))
        };
        let Json(got) = list(None).await;
        let ada = by_name(&got, "ada");
        assert_eq!(ada["listed"], true);
        assert!(ada["cloned"].is_object(), "{ada}");
        assert_eq!(ada["used_by"], serde_json::json!(["Mara"]));
        // default.wav is on disk and still not a clone: no "cloned here"
        // row, so the page offers no Delete for the one voice that must stay.
        assert!(by_name(&got, "default")["cloned"].is_null(), "{got}");

        // Deleting a voice deletes its transcript too: on Breeze the TTS
        // wrote `<name>.txt`, a verbatim record of the same recording.
        std::fs::write(w.root.join("voices").join("solo.txt"), b"what solo said").unwrap();
        let gone = super::super::settings::voice_clone_delete(
            State(state.clone()),
            Json(super::super::settings::CloneQuery {
                name: "solo".into(),
            }),
        )
        .await;
        assert_eq!(gone.status(), StatusCode::OK);
        assert!(!w.root.join("voices").join("solo.wav").exists());
        assert!(
            !w.root.join("voices").join("solo.txt").exists(),
            "the transcript outlived its voice"
        );
        // A clone the worker does not list yet: on the list, and said so.
        assert_eq!(by_name(&got, "solo")["listed"], false);
        assert!(got["list_error"].is_null(), "{got}");

        // Read without an unlock, the list may be short — said whenever the
        // library is locked, never only when a locked persona exists.
        assert_eq!(got["used_by_partial"], "locked", "{got}");
        // Locked, Mara is named only for an unlock — and the answer says the
        // list may be short, so an empty one does not read as nobody.
        store::set_locked(&w.store(), "mara", true).unwrap();
        let Json(got) = list(None).await;
        assert_eq!(by_name(&got, "ada")["used_by"], serde_json::json!([]));
        assert_eq!(got["used_by_partial"], "locked");
        let Json(got) = list(Some(library.grant_for_tests())).await;
        assert_eq!(by_name(&got, "ada")["used_by"], serde_json::json!(["Mara"]));
        assert!(got["used_by_partial"].is_null(), "{got}");

        let sample = super::super::settings::library_voice_sample(
            State(state.clone()),
            Query(super::super::settings::SampleQuery { name: "ada".into() }),
        )
        .await;
        assert_eq!(sample.status(), StatusCode::OK);
        assert_eq!(sample.headers()["content-type"], "audio/wav");
        let body = axum::body::to_bytes(sample.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], b"RIFF:ada");

        // A voice only a persona names — a typo — is listed, and says so,
        // rather than surfacing first as a refused call (review of #490).
        give_voice(&w, "adaa", None);
        let Json(got) = list(Some(library.grant_for_tests())).await;
        let ghost = by_name(&got, "adaa");
        assert_eq!(ghost["listed"], false, "{ghost}");
        assert!(ghost["cloned"].is_null(), "{ghost}");
        assert_eq!(ghost["used_by"], serde_json::json!(["Mara"]));

        // A worker without the preview route: told apart from an unlisted
        // voice by the body, since both 404s carry a content-type.
        let old = axum::Router::new().route(
            "/mecha/voices",
            axum::routing::get(|| async { Json(serde_json::json!({ "voices": ["ada"] })) }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, old).await.ok() });
        let mut stale = state.clone();
        stale.offer_target = Some(Arc::new(format!("http://{addr}/api/offer")));
        let refused = super::super::settings::library_voice_sample(
            State(stale),
            Query(super::super::settings::SampleQuery { name: "ada".into() }),
        )
        .await;
        assert_eq!(refused.status(), StatusCode::CONFLICT);
        let why = axum::body::to_bytes(refused.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(
            String::from_utf8_lossy(&why).contains("predates previews"),
            "{}",
            String::from_utf8_lossy(&why)
        );
    }

    /// An offer that does not become a nameable call releases the binding
    /// it made, so the chat takes no spoken words on it: one the worker
    /// refuses (review of #483), and one it takes with an answer the call id
    /// cannot ride on (review of #490).
    #[tokio::test]
    async fn an_offer_that_is_no_nameable_call_releases_its_binding() {
        for (status, body) in [
            (StatusCode::SERVICE_UNAVAILABLE, "busy"),
            (StatusCode::OK, "an answer that is not JSON"),
        ] {
            let w = world();
            let key = open_chat(&w).await;
            let app = axum::Router::new().route(
                "/api/offer",
                axum::routing::post(move || async move { (status, body) }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move { axum::serve(listener, app).await.ok() });
            let state = super::super::WebState {
                owner_login: Arc::new("owner@example.com".into()),
                chat: Some(Arc::clone(&w.chat)),
                offer_target: Some(Arc::new(format!("http://{addr}/api/offer"))),
                voices_dir: None,
                stt_url: Arc::new(mecha_core::config::VoiceConfig::DEFAULT_STT_URL.to_string()),
                library: Arc::new(LibraryState::new(w.root.join("imagelib"))),
                features_at_start: Arc::default(),
                gate: Arc::default(),
                review: Arc::new(super::super::review::ReviewState {
                    outbox_root: w.root.join("outbox"),
                    sessions_dir: None,
                }),
            };
            let offer = axum::body::Bytes::from(
                serde_json::json!({"sdp": "x", "type": "offer", "request_data": {"session": key}})
                    .to_string(),
            );
            let answered = super::super::offer_proxy(State(state), offer).await;
            assert_eq!(answered.status(), status, "{body}");
            assert!(
                !w.personas().calls.lock().unwrap().contains_key(&key),
                "an offer answered {status} ({body}) left its binding"
            );
        }
    }

    /// The play button's speech (the owner's ask, 2026-10-01): a persona
    /// chat's reply in the persona's voice whatever the page asks, behind its
    /// lock; the assistant's in the voice the page names; an older worker
    /// told apart from a refusal.
    #[tokio::test]
    async fn a_reply_is_spoken_in_its_chats_voice() {
        let w = world();
        give_voice(&w, "ada", Some(1.2));
        let key = open_chat(&w).await;
        let seen = Arc::new(StdMutex::new(Vec::<serde_json::Value>::new()));
        let kept = Arc::clone(&seen);
        let app = axum::Router::new().route(
            "/mecha/speak",
            axum::routing::post(move |Json(body): Json<serde_json::Value>| {
                let kept = Arc::clone(&kept);
                async move {
                    kept.lock().unwrap().push(body);
                    ([("content-type", "audio/wav")], "RIFF")
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        let library = Arc::new(LibraryState::new(w.root.join("imagelib")));
        let state = |target: String| super::super::WebState {
            owner_login: Arc::new("owner@example.com".into()),
            chat: Some(Arc::clone(&w.chat)),
            offer_target: Some(Arc::new(target)),
            voices_dir: None,
            stt_url: Arc::new(mecha_core::config::VoiceConfig::DEFAULT_STT_URL.to_string()),
            library: Arc::clone(&library),
            features_at_start: Arc::default(),
            gate: Arc::default(),
            review: Arc::new(super::super::review::ReviewState {
                outbox_root: w.root.join("outbox"),
                sessions_dir: None,
            }),
        };
        let target = format!("http://{addr}/api/offer");
        let speak_at = |chat: Option<String>,
                        unlock: Option<String>,
                        voice: Option<String>,
                        speed: Option<f64>,
                        target: String| {
            super::super::settings::speak(
                State(state(target)),
                Json(super::super::settings::SpeakBody {
                    text: "The dig went well.".into(),
                    chat,
                    unlock,
                    voice,
                    speed,
                    listen: None,
                    stream: false,
                }),
            )
        };
        let speak = |chat, unlock, voice, target| speak_at(chat, unlock, voice, None, target);
        // A persona chat: the persona's voice and rate, never the page's.
        let r = speak_at(
            Some(key.clone()),
            None,
            Some("page".into()),
            Some(1.9),
            target.clone(),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(
            seen.lock().unwrap().pop().unwrap(),
            serde_json::json!({"text": "The dig went well.", "voice": "ada", "speed": 1.2})
        );
        // Locked, it is refused as its page is — and nothing is spoken.
        store::set_locked(&w.store(), "mara", true).unwrap();
        let r = speak(Some(key.clone()), None, None, target.clone()).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        assert!(seen.lock().unwrap().is_empty());
        let r = speak(
            Some(key.clone()),
            Some(library.grant_for_tests()),
            None,
            target.clone(),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        seen.lock().unwrap().clear();
        // The assistant's chat: the voice the page names, the owner's choice.
        let r = speak(
            Some("main".into()),
            None,
            Some("bm_george".into()),
            target.clone(),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(seen.lock().unwrap().pop().unwrap()["voice"], "bm_george");
        // And the rate the owner set beside it, as an assistant call has it
        // (review of #502); out of range is refused before anything speaks.
        let r = speak_at(Some("main".into()), None, None, Some(1.4), target.clone()).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(seen.lock().unwrap().pop().unwrap()["speed"], 1.4);
        let r = speak_at(Some("main".into()), None, None, Some(3.0), target.clone()).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
        assert!(seen.lock().unwrap().is_empty());
        // A worker without the route: named, not a bare 404.
        let old = axum::Router::new();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let old_addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, old).await.ok() });
        let r = speak(None, None, None, format!("http://{old_addr}/api/offer")).await;
        assert_eq!(r.status(), StatusCode::CONFLICT);
        let why = axum::body::to_bytes(r.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&why).contains("predates the play button"));
    }

    /// Listen streams (`SpeakBody::stream`): serve asks the worker for PCM
    /// and passes each chunk on as it arrives, never collected — the first
    /// is read through serve before the worker has sent the second — at the
    /// rate the worker named; a stream at no playable rate is refused; and a
    /// page that does not ask for a stream still gets one WAV.
    #[tokio::test]
    async fn a_streamed_piece_is_passed_on_as_it_arrives() {
        use futures::StreamExt;

        let w = world();
        let seen = Arc::new(StdMutex::new(Vec::<serde_json::Value>::new()));
        type Chunk = Result<axum::body::Bytes, std::io::Error>;
        let (tx, rx) = tokio::sync::mpsc::channel::<Chunk>(4);
        let rx = Arc::new(tokio::sync::Mutex::new(Some(rx)));
        let rate = Arc::new(StdMutex::new("24000"));
        let (kept, chunks, named) = (Arc::clone(&seen), Arc::clone(&rx), Arc::clone(&rate));
        let app = axum::Router::new().route(
            "/mecha/speak",
            axum::routing::post(move |Json(body): Json<serde_json::Value>| {
                let (kept, chunks, named) =
                    (Arc::clone(&kept), Arc::clone(&chunks), Arc::clone(&named));
                async move {
                    let streamed = body["stream"] == true;
                    kept.lock().unwrap().push(body);
                    if !streamed {
                        return ([("content-type", "audio/wav")], "RIFF").into_response();
                    }
                    let rx = chunks.lock().await.take();
                    let body = match rx {
                        Some(rx) => axum::body::Body::from_stream(futures::stream::unfold(
                            rx,
                            |mut rx| async move { rx.recv().await.map(|c| (c, rx)) },
                        )),
                        None => axum::body::Body::empty(),
                    };
                    let rate = *named.lock().unwrap();
                    (
                        [("content-type", "audio/pcm"), ("x-sample-rate", rate)],
                        body,
                    )
                        .into_response()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        let library = Arc::new(LibraryState::new(w.root.join("imagelib")));
        let speak = |stream: bool| {
            let state = super::super::WebState {
                owner_login: Arc::new("owner@example.com".into()),
                chat: Some(Arc::clone(&w.chat)),
                offer_target: Some(Arc::new(format!("http://{addr}/api/offer"))),
                voices_dir: None,
                stt_url: Arc::new(mecha_core::config::VoiceConfig::DEFAULT_STT_URL.to_string()),
                library: Arc::clone(&library),
                features_at_start: Arc::default(),
                gate: Arc::default(),
                review: Arc::new(super::super::review::ReviewState {
                    outbox_root: w.root.join("outbox"),
                    sessions_dir: None,
                }),
            };
            super::super::settings::speak(
                State(state),
                Json(super::super::settings::SpeakBody {
                    text: "The dig went well.".into(),
                    chat: Some("main".into()),
                    unlock: None,
                    voice: None,
                    speed: None,
                    listen: None,
                    stream,
                }),
            )
        };

        tx.send(Ok(axum::body::Bytes::from_static(b"\x01\x00\x02")))
            .await
            .unwrap();
        let r = speak(true).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(seen.lock().unwrap().pop().unwrap()["stream"], true);
        assert_eq!(r.headers()["content-type"], "audio/pcm");
        assert_eq!(r.headers()["x-sample-rate"], "24000");
        assert_eq!(r.headers()["cache-control"], "no-store");
        let mut body = r.into_body().into_data_stream();
        // The worker has sent one chunk and holds the rest: a serve that
        // collected the body would never return this.
        let first = tokio::time::timeout(std::time::Duration::from_secs(5), body.next())
            .await
            .expect("the first chunk arrived before the stream ended")
            .unwrap()
            .unwrap();
        assert_eq!(&first[..], b"\x01\x00\x02");
        tx.send(Ok(axum::body::Bytes::from_static(b"\x00")))
            .await
            .unwrap();
        drop(tx);
        let mut rest = Vec::new();
        while let Some(chunk) = body.next().await {
            rest.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(rest, b"\x00");

        // A rate no browser plays is refused, never handed on.
        *rate.lock().unwrap() = "4";
        let r = speak(true).await;
        assert_eq!(r.status(), StatusCode::BAD_GATEWAY);

        // Not asked for, not streamed: the page older than streaming.
        let r = speak(false).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()["content-type"], "audio/wav");
        assert!(seen.lock().unwrap().pop().unwrap().get("stream").is_none());
        // Both answers stay out of the browser's cache (review of #555).
        assert_eq!(r.headers()["cache-control"], "no-store");
    }

    /// Listen's director pass (`listen`): a tap on a reply asks the
    /// director once for the whole reply, with the persona and the owner's
    /// words in its scene, speaks every piece with that line as
    /// `instructions`, and records it in the chat's transcript; the same
    /// reply tapped again is spoken with what was recorded and asks no
    /// model; a worker whose engine takes no directions is never handed one,
    /// and no model is asked for it.
    #[tokio::test]
    async fn a_listen_tap_is_directed_recorded_and_recalled() {
        // The director runs on the chat's follower — the assistant's agent,
        // on whichever model is loaded — never the persona's own.
        let line = "Hello from Mara.";
        const WHOLE: &str = "The dig went well. We found the second wall.";
        let w = world();
        let key = open_chat(&w).await;
        let seen = Arc::new(StdMutex::new(Vec::<serde_json::Value>::new()));
        let worker = |directs: bool| {
            let kept = Arc::clone(&seen);
            let mut app = axum::Router::new().route(
                "/mecha/speak",
                axum::routing::post(move |Json(body): Json<serde_json::Value>| {
                    let kept = Arc::clone(&kept);
                    async move {
                        kept.lock().unwrap().push(body);
                        ([("content-type", "audio/wav")], "RIFF")
                    }
                }),
            );
            if directs {
                app = app.route(
                    "/mecha/directs",
                    axum::routing::get(|| async { Json(serde_json::json!({"directs": true})) }),
                );
            }
            async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();
                tokio::spawn(async move { axum::serve(listener, app).await.ok() });
                format!("http://{addr}/api/offer")
            }
        };
        let library = Arc::new(LibraryState::new(w.root.join("imagelib")));
        let tap = |target: String, text: &str, index: u32| {
            let state = super::super::WebState {
                owner_login: Arc::new("owner@example.com".into()),
                chat: Some(Arc::clone(&w.chat)),
                offer_target: Some(Arc::new(target)),
                voices_dir: None,
                stt_url: Arc::new(mecha_core::config::VoiceConfig::DEFAULT_STT_URL.to_string()),
                library: Arc::clone(&library),
                features_at_start: Arc::default(),
                gate: Arc::default(),
                review: Arc::new(super::super::review::ReviewState {
                    outbox_root: w.root.join("outbox"),
                    sessions_dir: None,
                }),
            };
            super::super::settings::speak(
                State(state),
                Json(super::super::settings::SpeakBody {
                    text: text.into(),
                    chat: Some(key.clone()),
                    unlock: None,
                    voice: None,
                    speed: None,
                    listen: Some(super::super::listen::ListenCue {
                        reply: "r1".into(),
                        index,
                        whole: Some(WHOLE.into()),
                        asked: Some("How did the dig go?".into()),
                        last_reply: None,
                    }),
                    stream: false,
                }),
            )
        };
        let transcript = w.personas().sessions.lock().await[&key]
            .session
            .path
            .clone();
        let directions = || mecha_core::session::Session::spoken_directions(&transcript).unwrap();
        let asked = || w.assistant_seen.lock().unwrap().len();

        let target = worker(true).await;
        let before = asked();
        let r = tap(target.clone(), "The dig went well.", 0).await;
        assert_eq!(r.status(), StatusCode::OK);
        let sent = mecha_core::voice_direction::sent(line);
        assert_eq!(seen.lock().unwrap().pop().unwrap()["instructions"], sent);
        assert_eq!(asked(), before + 1, "one director call");
        let prompt = w.assistant_seen.lock().unwrap().last().unwrap().clone();
        assert_eq!(
            prompt.system.as_deref(),
            Some(mecha_core::voice_direction::SYSTEM),
            "the director's own frame"
        );
        let user = prompt.messages.last().unwrap().text();
        assert!(
            user.contains("They just said: How did the dig go?"),
            "{user}"
        );
        assert!(
            user.contains(&format!("direct all of it with one line: \"{WHOLE}\"")),
            "{user}"
        );
        assert!(
            !user.contains("a warm, capable personal assistant"),
            "the persona speaks: {user}"
        );
        let identity = {
            let personas = w.personas();
            let sessions = personas.sessions.lock().await;
            mecha_core::persona::strip_comments(&sessions[&key].pinned.identity).0
        };
        assert!(
            // As the prompt bounds it: one line.
            user.contains(&format!(
                "Speaker: {}",
                identity.split_whitespace().collect::<Vec<_>>().join(" ")
            )),
            "the persona's identity is the scene's speaker: {user}"
        );
        let recorded = directions();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].turn, "listen:r1");
        assert_eq!(recorded[0].direction.as_deref(), Some(sent.as_str()));
        assert_eq!(
            recorded[0].outcome,
            mecha_core::voice_direction::Outcome::Ok
        );
        assert_eq!(recorded[0].sentence, WHOLE, "the reply it covers");

        // The next piece: the same line, and no second call.
        let before = asked();
        let r = tap(target.clone(), "We found the second wall.", 1).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(seen.lock().unwrap().pop().unwrap()["instructions"], sent);
        assert_eq!(asked(), before, "one direction per reply");
        assert_eq!(directions().len(), 1);

        // Tapped again: what was recorded, and no model asked.
        let before = asked();
        seen.lock().unwrap().clear();
        tap(target.clone(), "The dig went well.", 0).await;
        tap(target.clone(), "We found the second wall.", 1).await;
        assert_eq!(asked(), before, "a recalled reply asks no model");
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .all(|b| b["instructions"] == sent));
        assert_eq!(
            directions().len(),
            1,
            "a recalled line is not recorded again"
        );

        // A worker that takes no directions: none sent, none asked for.
        let target = worker(false).await;
        let before = asked();
        let r = tap(target, "Something new entirely.", 0).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert!(seen
            .lock()
            .unwrap()
            .pop()
            .unwrap()
            .get("instructions")
            .is_none());
        assert_eq!(asked(), before);
    }

    /// An incognito chat's reply is still directed (owner ruling,
    /// 2026-10-03: the director runs there, and nothing is kept) — the line
    /// reaches the engine though there is no transcript to record it in
    /// (review of #539: the record's gate once threw the line away too).
    #[tokio::test]
    async fn an_incognito_reply_is_directed_and_kept_nowhere() {
        let w = world();
        let app = axum::Router::new().route(
            "/mecha/directs",
            axum::routing::get(|| async { Json(serde_json::json!({"directs": true})) }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        let cue = super::super::listen::ListenCue {
            reply: "r1".into(),
            index: 0,
            whole: Some("The dig went well.".into()),
            asked: Some("How did the dig go?".into()),
            last_reply: None,
        };
        let seat = super::super::listen::Seat {
            reply: Arc::new(super::super::listen::Reply::new(&cue, None, None)),
            transcript: None,
        };
        let line = super::super::listen::direct(
            &w.chat.follower,
            &w.chat.stopping,
            &format!("http://{addr}/api/offer"),
            &seat,
            None,
        )
        .await;
        assert_eq!(
            line,
            Some(mecha_core::voice_direction::sent("Hello from Mara.")),
            "the director ran and its line is spoken"
        );
        assert_eq!(w.assistant_seen.lock().unwrap().len(), 1);
    }
}
