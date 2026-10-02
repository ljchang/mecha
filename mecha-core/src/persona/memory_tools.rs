//! A persona's own memory tools (`docs/PERSONA-DESIGN.md` §9.7, "on demand"):
//! `memory_search` and `memory_read` — §9.7's `recall` and `recall_open`,
//! renamed because `recall` is already the assistant's tool for its own
//! conversation (`tool::recall`), and named as the pair `file_search` and
//! `file_read` are: find a memory, then read it in full.
//!
//! Recall already happens without them — at a chat's start and on every turn
//! (`persona::recall`), keyed on the **owner's** words so a persona does not
//! steer what it is reminded of. These are the other half: the persona
//! asking, when it realises it needs something the owner's message did not
//! name, and then reading what was actually said rather than the summary of
//! it (§9.2: memory loses detail in the summary; the pointer is how it is
//! recovered).
//!
//! **Taint, as recall's.** Both declare `private` (memory of the owner) and
//! `untrusted`; a result is marked as from outside only when something it
//! returns was — a record of untrusted origin, or a turn of the earlier chat
//! that read something from outside. Clean memory leaves the chat clean.
//!
//! **`memory_read` reads another conversation, which is what
//! `tool::recall` forbids itself**: re-surfacing a different chat's text
//! without its taint would launder it. So the turns it returns carry their
//! own recorded taint (`writer::read_chat`), and an uncovered turn is
//! unknown, which counts as from outside. The path is never the model's: the
//! chat and its turns come from the stored episode, the file from this
//! persona's own `sessions/`, and a chat id that is not a session id is
//! refused.
//!
//! **Switches, read live.** Each call reads the persona's `[memory]` as it
//! stands, as the safety switches are read: turning memory off reaches an
//! open chat at its next call. Registration follows the chat's pinned
//! version, so the tool list — the front of the cached prefix — never
//! changes mid-chat.
//!
//! **Files only still remembers** (owner ruling 2026-10-02). `answers =
//! "files"` withholds the tools that bring third-party content in (§10.4);
//! these are memory, whose own switches are its control, so they stay. What
//! changes is what `memory_read` returns: the owner's words and the
//! persona's, with each tool result left out where it stood
//! ([`words_only`]), so a chat from when the persona answered from anything
//! cannot bring the web back in by memory.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

use super::memory::{Filter, Memory, Recallable, Table, MEMORY_DB};
use super::writer::{bound, messages_in, named, read_chat, Stretch};
use super::{Answers, Origin, Persona, Settings, Store, UserFacts};
use crate::embed::Embedder;
use crate::message::{Block, Message, Role};
use crate::tool::{Capabilities, Tool, ToolCtx, ToolOutput};

/// Records `memory_search` returns by default, and at most.
const DEFAULT_HITS: usize = 6;
const MAX_HITS: usize = 12;
/// The most of a past conversation one `memory_read` returns.
pub const MAX_READ_CHARS: usize = 8000;

/// How much of `MAX_READ_CHARS` goes to a long conversation's opening; the
/// rest keeps its end, where it usually landed.
const READ_HEAD_CHARS: usize = 2000;

/// What a tool result reads as for a persona that answers from its files.
const LEFT_OUT: &str = "(left out: you answer from your files, not from what a tool brought back)";
/// How long a search waits for the embeddings server; slower, and it is by
/// words (`persona_chat`'s `RECALL_EMBED_WAIT`, for the same cold start).
const EMBED_WAIT: std::time::Duration = std::time::Duration::from_secs(8);

/// What memory the persona's settings let a search reach — the one reading
/// of the switches, shared with `recall::per_turn`.
pub fn kinds(s: &Settings) -> Vec<Recallable> {
    let m = &s.memory;
    let mut out = Vec::new();
    if m.episodic {
        out.push(Recallable::Episodes);
    }
    if m.semantic {
        out.push(Recallable::Facts(Table::Persona));
    }
    if m.user_facts != UserFacts::Off {
        out.push(Recallable::Facts(Table::User));
        out.push(Recallable::Facts(Table::Inferred));
    }
    out
}

/// Whether `memory_search` belongs in a chat with these settings: any
/// memory switched on.
pub fn offers_search(s: &Settings) -> bool {
    !kinds(s).is_empty()
}

/// Whether `memory_read` does: past conversations remembered.
pub fn offers_read(s: &Settings) -> bool {
    s.memory.episodic
}

/// The id an episode is shown under — its first eight characters — and the
/// one `memory_read` takes back.
pub fn short(uid: &str) -> &str {
    uid.get(..8).unwrap_or(uid)
}

/// The persona as it stands now, off the runtime.
async fn live(store: PathBuf, name: String) -> Option<Persona> {
    tokio::task::spawn_blocking(move || Store::load(&store).get(&name).cloned())
        .await
        .ok()
        .flatten()
}

/// `memory_search`: the persona searching what it remembers.
pub struct MemorySearch {
    store: PathBuf,
    persona: String,
    embedder: Option<Embedder>,
}

impl MemorySearch {
    pub fn new(store: PathBuf, persona: String, embedder: Option<Embedder>) -> MemorySearch {
        MemorySearch {
            store,
            persona,
            embedder,
        }
    }
}

#[async_trait]
impl Tool for MemorySearch {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Search what you remember from earlier conversations with the owner — what you talked \
         about, what is true between you, what you know about them. For when you need something \
         the owner did not just name. A past conversation comes back with an id in brackets, \
         which `memory_read` opens in full when you have it."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "What you are trying to remember, in plain words"},
                "limit": {"type": "integer", "description": "Memories to return, 1 to 12. Default 6"}
            },
            "required": ["query"]
        })
    }

    fn read_only(&self) -> bool {
        true
    }

    /// Memory of the owner, so private; a record written from a stretch
    /// that read something from outside can be among the results.
    fn capabilities(&self) -> Capabilities {
        Capabilities::default().private().untrusted()
    }

    fn for_persona(self: Arc<Self>) -> Option<Arc<dyn Tool>> {
        Some(self)
    }

    async fn call(&self, input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
        let Some(query) = input
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|q| !q.is_empty())
        else {
            return Ok(ToolOutput::err(
                "`query` is required: what you are trying to remember.",
            ));
        };
        let limit = input
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| (n as usize).clamp(1, MAX_HITS))
            .unwrap_or(DEFAULT_HITS);
        let Some(persona) = live(self.store.clone(), self.persona.clone()).await else {
            return Ok(ToolOutput::err("Your memory could not be read just now."));
        };
        let kinds = kinds(&persona.settings);
        if kinds.is_empty() {
            return Ok(ToolOutput::err("Your memory is switched off."));
        }
        if !self.store.join(&self.persona).join(MEMORY_DB).is_file() {
            return Ok(ToolOutput::ok("You remember nothing yet."));
        }
        let qvec = match &self.embedder {
            Some(e) => tokio::time::timeout(EMBED_WAIT, e.recall_query(query))
                .await
                .ok()
                .and_then(Result::ok),
            None => None,
        };
        let (store, name, q) = (self.store.clone(), self.persona.clone(), query.to_owned());
        let found = tokio::task::spawn_blocking(move || {
            Memory::open_to_edit(&store, &name)?.recall_search(
                &q,
                qvec.as_deref(),
                &kinds,
                &|_| false,
                limit,
            )
        })
        .await;
        let (found, by_meaning) = match found {
            Ok(Ok(found)) => found,
            _ => {
                return Ok(ToolOutput::err(
                    "Your memory could not be searched just now.",
                ))
            }
        };
        if found.is_empty() {
            return Ok(ToolOutput::ok("Nothing you remember matches that."));
        }
        let mut untrusted = false;
        let mut lines = vec![if by_meaning {
            "From your memory, by meaning and by words:".to_string()
        } else {
            "From your memory, by words:".to_string()
        }];
        for r in found {
            untrusted |= r.origin == Origin::ModelUntrusted;
            let day = r.date.get(..10).unwrap_or(&r.date);
            lines.push(match r.kind {
                Recallable::Episodes => {
                    format!("- {day} · a conversation [{}]: {}", short(&r.uid), r.text)
                }
                Recallable::Facts(Table::User) => format!("- {day} · about the owner: {}", r.text),
                Recallable::Facts(Table::Inferred) => {
                    format!("- {day} · your guess about the owner: {}", r.text)
                }
                Recallable::Facts(Table::Persona) => format!("- {day} · between you: {}", r.text),
            });
        }
        let out = ToolOutput::ok(lines.join("\n"));
        Ok(if untrusted { out.from_outside() } else { out })
    }
}

/// `memory_read`: a remembered conversation in full — the transcript turns
/// its episode points at.
pub struct MemoryRead {
    store: PathBuf,
    persona: String,
}

impl MemoryRead {
    pub fn new(store: PathBuf, persona: String) -> MemoryRead {
        MemoryRead { store, persona }
    }
}

/// A chat id as a session id is minted — letters, digits and `-` — so one
/// read from the store can never name a path outside `sessions/`.
fn is_chat_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// What reading one episode found: the text, and whether any of it came
/// from outside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    pub text: String,
    pub untrusted: bool,
}

/// Read the conversation behind episode `id` for `persona`: off the runtime,
/// and testable without a provider. `Err` names what to tell the persona.
pub fn read_episode(
    store: &std::path::Path,
    persona: &Persona,
    id: &str,
) -> std::result::Result<Read, String> {
    let id = id.trim().to_ascii_lowercase();
    if id.len() < 4 || id.len() > 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(
            "That is not a conversation id: use the one in brackets from your memory.".into(),
        );
    }
    let m = Memory::open_existing(store, &persona.name)
        .map_err(|_| "Your memory could not be read just now.".to_string())?
        .ok_or_else(|| "You remember no conversations yet.".to_string())?;
    let episodes = m
        .episodes(Filter::Recallable)
        .map_err(|_| "Your memory could not be read just now.".to_string())?;
    let mut hits = episodes.into_iter().filter(|e| e.uid.starts_with(&id));
    let ep = match (hits.next(), hits.next()) {
        (Some(e), None) => e,
        (None, _) => return Err("You remember no conversation with that id.".into()),
        (Some(_), Some(_)) => {
            return Err("That id names more than one conversation; give more of it.".into())
        }
    };
    if !is_chat_id(&ep.source.chat) {
        return Err("That conversation cannot be opened.".into());
    }
    let path = store
        .join(&persona.name)
        .join("sessions")
        .join(format!("{}.jsonl", ep.source.chat));
    let text = std::fs::read_to_string(&path).map_err(|_| {
        "That conversation is no longer kept, so only the memory of it is left.".to_string()
    })?;
    let chat = read_chat(&text);
    let n = chat.turns.len() as u32;
    let (from, to) = (ep.source.from.min(n), (ep.source.to + 1).min(n));
    if from >= to {
        return Err("That conversation's turns are no longer in its record.".into());
    }
    // Each turn's own recorded taint; an uncovered or unreadable one is
    // unknown, and unknown is never clean.
    let untrusted = chat.turns[from as usize..to as usize]
        .iter()
        .any(|t| t.message.is_none() || t.taint.is_none_or(|t| t.untrusted));
    // `messages_in` reads no origin; the taint is the turns' own, above.
    let mut messages = messages_in(
        &chat,
        Stretch {
            from,
            to,
            origin: Origin::ModelClean,
        },
    );
    if persona.settings.files.answers == Answers::Files {
        messages = words_only(messages);
    }
    let rendered = named(&messages, persona.display());
    let day = ep
        .started_at
        .as_deref()
        .unwrap_or(&ep.ingested_at)
        .get(..10)
        .unwrap_or("");
    let body = bound(&rendered, READ_HEAD_CHARS, MAX_READ_CHARS - READ_HEAD_CHARS);
    Ok(Read {
        text: format!(
            "The conversation behind [{}] ({day}): {}\n\n{body}",
            short(&ep.uid),
            ep.summary
        ),
        untrusted,
    })
}

/// An earlier conversation as a files-only persona may read it (§10.4,
/// owner ruling 2026-10-02): the owner's words and its own. What a tool
/// brought back is left out where it stood, and its call with it, so a chat
/// from when the persona answered from anything cannot bring the web back in
/// by memory. The harness's folded text (files, memory, notices) is dropped:
/// the persona reaches its files and its memory through their own tools.
fn words_only(messages: Vec<Message>) -> Vec<Message> {
    messages
        .into_iter()
        .map(|mut m| {
            let role = m.role;
            m.content = m
                .content
                .into_iter()
                .filter_map(|b| match b {
                    Block::ToolUse { .. } => None,
                    Block::ToolResult { tool_use_id, .. } => Some(Block::ToolResult {
                        tool_use_id,
                        content: LEFT_OUT.into(),
                        is_error: false,
                    }),
                    Block::Text { text }
                        if role == Role::User && crate::agent::is_harness_voice(&text) =>
                    {
                        None
                    }
                    b => Some(b),
                })
                .collect();
            m
        })
        .collect()
}

#[async_trait]
impl Tool for MemoryRead {
    fn name(&self) -> &str {
        "memory_read"
    }

    fn description(&self) -> &str {
        "Read a remembered conversation in full — what was actually said — by its id: the one \
         in brackets in your memory or from `memory_search`. For when the summary is not enough \
         and you want the owner's own words."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "The conversation's id, as shown in brackets"}
            },
            "required": ["id"]
        })
    }

    fn read_only(&self) -> bool {
        true
    }

    /// The owner's words from an earlier chat, so private; and that chat may
    /// have read something from outside.
    fn capabilities(&self) -> Capabilities {
        Capabilities::default().private().untrusted()
    }

    fn for_persona(self: Arc<Self>) -> Option<Arc<dyn Tool>> {
        Some(self)
    }

    async fn call(&self, input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
        let Some(id) = input.get("id").and_then(Value::as_str).map(str::to_owned) else {
            return Ok(ToolOutput::err(
                "`id` is required: the conversation's id, as shown in brackets.",
            ));
        };
        let Some(persona) = live(self.store.clone(), self.persona.clone()).await else {
            return Ok(ToolOutput::err("Your memory could not be read just now."));
        };
        if !offers_read(&persona.settings) {
            return Ok(ToolOutput::err(
                "Remembering past conversations is switched off.",
            ));
        }
        let store = self.store.clone();
        let read = tokio::task::spawn_blocking(move || read_episode(&store, &persona, &id)).await;
        Ok(match read {
            Ok(Ok(r)) if r.untrusted => ToolOutput::ok(r.text).from_outside(),
            Ok(Ok(r)) => ToolOutput::ok(r.text),
            Ok(Err(why)) => ToolOutput::err(why),
            Err(_) => ToolOutput::err("That conversation could not be read just now."),
        })
    }
}

#[cfg(test)]
mod tests;
