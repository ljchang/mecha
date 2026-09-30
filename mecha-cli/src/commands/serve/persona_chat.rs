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
use mecha_core::persona::safety;
use mecha_core::persona::{Persona, Store};
use mecha_core::session::{Record, Session, SessionMeta};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{broadcast, Mutex};

use super::chat::{self, ChatState, WireEvent};
use super::library::LibraryState;

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

/// How a persona agent's provider is made — the router's, in `serve`; a
/// scripted one in tests.
type ProviderFactory = Arc<
    dyn Fn(&crate::follow::Bound) -> Result<Box<dyn mecha_core::provider::Provider>> + Send + Sync,
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
}

struct PersonaSession {
    pinned: Arc<Pinned>,
    session: Arc<Session>,
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
    /// `safety::CRISIS_COOLDOWN` of it does not pause again (§12.2).
    crisis_paused_at: Option<std::time::Instant>,
    /// An owner message that tripped the crisis sensor while a run was live:
    /// the run is stopped, and its hand-back records this so the words are
    /// kept although the persona never answers them.
    pending_crisis: Option<String>,
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

/// The most text a session goal may carry.
const MAX_GOAL: usize = 2000;

/// Why a door refused, as the route will say it.
#[derive(Debug)]
pub enum Refusal {
    /// Missing, or locked without a live unlock token — the same answer.
    NotFound,
    /// The persona exists but cannot open a chat as it stands.
    Conflict(String),
    Bad(String),
    Failed(String),
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
        }
    }
}

fn failed(e: anyhow::Error) -> Refusal {
    Refusal::Failed(format!("{e:#}"))
}

/// What the safety layer can do for a persona, as every surface shows it
/// (§12): each switch, with the crisis sensor said as it is — `degraded`
/// (keywords only) until its model tiers exist — and the farewell check as
/// unbuilt, so neither can pass for "checked".
fn safety_json(s: &mecha_core::persona::Safety) -> serde_json::Value {
    serde_json::json!({
        "disclosure": s.disclosure,
        "crisis": safety::crisis_state(s.crisis),
        "reanchor": s.reanchor,
        "dose": s.dose,
        "breaks": s.breaks,
        "farewell": if s.farewell { "unbuilt" } else { "off" },
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
            Arc::new(|_| anyhow::bail!("this test opens no persona chat")),
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
        let (agent, refused) = crate::setup::persona_agent(bound, pinned, (self.provider)(bound)?)?;
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
        let name = p.settings.character.as_ref()?;
        if library.unlocked(token) {
            return None;
        }
        // Shown only when the library holds it, open. An entry that did not
        // load (a damaged `entry.toml`) is unknown, and unknown reads as
        // locked (review of #430). Locked keys on `locked`, not approval: a
        // locked candidate is hidden too.
        let lib = mecha_core::imagelib::Library::load(&library.dir).0;
        match lib.get(mecha_core::imagelib::Kind::Character, name) {
            Some(e) if !e.locked => None,
            _ => Some(name.clone()),
        }
    }

    /// What the settings form offers, from the stores as they stand — built
    /// the same way for the page's GET and for checking its save, so a save
    /// is held to the choices the page was shown.
    fn form(&self, library: &LibraryState, token: Option<&str>) -> mecha_core::tomlform::Form {
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

    /// A persona's owner files as they stand, each with the digest a save
    /// hands back so an edit made elsewhere meanwhile is refused, not lost.
    pub fn files(
        &self,
        library: &LibraryState,
        name: &str,
        token: Option<&str>,
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
                let form = self.form(library, token);
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

    /// Save one owner file, verbatim, and take a version (`write_owner_file`).
    pub fn save(
        &self,
        library: &LibraryState,
        name: &str,
        body: SaveBody,
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
                let form = self.form(library, body.unlock.as_deref());
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
                Refusal::Conflict(format!("{e:#}"))
            } else {
                Refusal::Bad(format!("{e:#}"))
            }
        })?;
        let store = Store::load(&self.store);
        let lib = mecha_core::imagelib::Library::load(&library.dir).0;
        // A problem that names a hidden portrait would name it — the one
        // hidden before the save, and the one the save may have just set: a
        // guessed `character = "x"` must not come back as "not in the
        // library" for one name and silence for a locked one (review of #430).
        let after = store
            .get(&p.name)
            .and_then(|q| self.hidden_character(library, q, body.unlock.as_deref()));
        let named =
            |m: &str, c: &Option<String>| c.as_ref().is_some_and(|c| m.contains(&format!("`{c}`")));
        let problems: Vec<String> = store
            .get(&p.name)
            .map(|q| store.problems(q, &lib))
            .unwrap_or_default()
            .into_iter()
            .filter(|m| !named(m, &hidden) && !named(m, &after))
            .collect();
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
                // The linked character's portrait, by the library's own rule:
                // approved only, and a locked one only with the live token.
                let portrait = p
                    .settings
                    .character
                    .as_deref()
                    .and_then(|c| lib.get(mecha_core::imagelib::Kind::Character, c))
                    .filter(|e| e.status == mecha_core::persona::Status::Approved)
                    .filter(|e| !e.locked || unlocked)
                    .and_then(|e| super::library::portrait_url(e, token.filter(|_| unlocked)));
                serde_json::json!({
                    "name": p.name,
                    "display": p.display(),
                    "relationship": p.settings.relationship.0,
                    "character": p.settings.character,
                    "portrait": portrait,
                    "version": p.state.version,
                    "approved": p.state.status == mecha_core::persona::Status::Approved,
                    "locked": p.state.locked,
                    "problems": store.problems(p, &lib),
                    "safety": safety_json(&p.settings.safety),
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
                                "skipped": doses.skipped,
                            })
                        }
                    }),
                })
            })
            .collect();
        let hidden = store.all().len() - rows.len();
        serde_json::json!({
            "personas": rows,
            "hidden_locked": hidden,
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
                pending_crisis: None,
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
        let workspace = self.work.join(&key);
        mecha_core::create_private_dir(&workspace).map_err(|e| failed(e.into()))?;
        sessions.insert(
            key.clone(),
            PersonaSession {
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
                pending_crisis: None,
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
        let (entries, taint) = match (&ps.conversation, &ps.live) {
            (Some(c), _) => (chat::transcript_entries(&c.messages), Some(c.taint)),
            (None, Some(live)) => (chat::transcript_entries(&live.history), Some(live.taint)),
            (None, None) => (Vec::new(), None),
        };
        let usage = ps.last_usage.lock().ok().and_then(|u| u.clone());
        Ok(serde_json::json!({
            "session": ps.session.meta.id,
            "persona": ps.pinned.name,
            "version": ps.pinned.version,
            "model": bound.model,
            "running": ps.live.is_some(),
            "goal": ps.goal,
            "display": ps.pinned.settings.display,
            "safety": safety_json(&ps.pinned.settings.safety),
            "entries": entries,
            "taint": taint.map(|t| serde_json::json!({
                "private": t.private, "untrusted": t.untrusted,
            })),
            "usage": usage.map(|u| serde_json::json!({
                "prompt_tokens": u.input_tokens + u.cache_read_input_tokens
                    + u.cache_creation_input_tokens,
                "context_window": bound.context_window,
            })),
        }))
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

    /// Start a turn, or steer the one in flight.
    pub async fn send(
        self: &Arc<Self>,
        chat: &Arc<ChatState>,
        library: &LibraryState,
        key: &str,
        text: &str,
        request_id: Option<String>,
        token: Option<&str>,
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
        let approved = Store::load(&self.store)
            .get(&name)
            .is_some_and(|p| p.state.status == mecha_core::persona::Status::Approved);
        if !approved {
            return Err(Refusal::Conflict(format!(
                "`{name}` is not approved — `mecha persona approve {name}` after reading it"
            )));
        }
        let (pinned, notices, early_pause) = {
            let mut sessions = self.sessions.lock().await;
            let ps = sessions.get_mut(key).ok_or(Refusal::NotFound)?;
            if ps.live.is_some() {
                return self.steer_or_pause(ps, &name, text, request_id);
            }
            // A pause needs no model: decided here, from the owner's words,
            // it skips the router hold and the agent below, so 988 is not
            // queued behind a model load it never uses (review of #418).
            let switches = ps.pinned.settings.safety;
            let early_pause = switches.crisis
                && safety::keyword_hit(&text)
                && !ps
                    .crisis_paused_at
                    .is_some_and(|t| t.elapsed() < safety::CRISIS_COOLDOWN);
            (Arc::clone(&ps.pinned), ps.events.clone(), early_pause)
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

        let mut sessions = self.sessions.lock().await;
        // Under the lock `stop` takes, so a turn is either cancelled by it or
        // refused here.
        if chat.stopping.is_cancelled() {
            return Err(Refusal::Failed("server is shutting down".into()));
        }
        let ps = sessions.get_mut(key).ok_or(Refusal::NotFound)?;
        if ps.live.is_some() {
            return self.steer_or_pause(ps, &name, text, request_id);
        }
        let Some(mut conversation) = ps.conversation.take() else {
            return Err(Refusal::Conflict("a turn is still finishing".into()));
        };
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
        let switches = pinned.settings.safety;
        let crisis_state = safety::crisis_state(switches.crisis);
        // `said`, not `text`: a goal typed at open rides in the first turn,
        // and is the owner's words as much as the message is.
        let hit = switches.crisis && safety::keyword_hit(&said);
        let cooling = ps
            .crisis_paused_at
            .is_some_and(|t| t.elapsed() < safety::CRISIS_COOLDOWN);
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
        let recorded = if conversation
            .messages
            .last()
            .is_some_and(|m| m.role == mecha_core::message::Role::User)
        {
            let pre_fold = conversation.messages.clone();
            mecha_core::agent::append_user_text(&mut conversation.messages, said.clone());
            if let Some(anchor) = &anchor {
                mecha_core::agent::append_user_text(&mut conversation.messages, anchor.clone());
            }
            ps.session
                .append(&Record::Rewrite {
                    messages: conversation.messages.clone(),
                })
                .map_err(|e| (e, Some(pre_fold)))
        } else {
            let mut user = Message::user(&said);
            // Its own block, in the harness's registered voice: never drawn in
            // the owner's bubble, never read as the owner's words.
            if let Some(anchor) = &anchor {
                user.content.push(mecha_core::message::Block::Text {
                    text: anchor.clone(),
                });
            }
            conversation.push(user.clone());
            ps.session
                .append(&Record::Message(user))
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
        let _ = ps.events.send(WireEvent::User {
            text: said,
            spoken: false,
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
            if let Err(e) = safety::record_crisis(&self.store, "web", pause) {
                tracing::warn!("a crisis record was not written: {e:#}");
            }
        }
        if pause {
            ps.crisis_paused_at = Some(std::time::Instant::now());
            let taint = conversation.taint;
            ps.conversation = Some(conversation);
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
            return Ok(serde_json::json!({ "started": false, "paused": true }));
        }
        let Some((bound, agent)) = ready else {
            // Unreachable: an early pause always pauses above.
            ps.conversation = Some(conversation);
            return Err(Refusal::Failed("a paused turn reached the model".into()));
        };
        let anchored = anchor.is_some();
        if anchored {
            ps.turns_since_anchor = 0;
            ps.anchor_due = false;
        } else {
            ps.turns_since_anchor += 1;
        }
        let before: Arc<[Message]> = conversation.messages.clone().into();

        let cancel = mecha_core::agent::CancelHandle::new();
        let queue: Arc<StdMutex<VecDeque<String>>> = Arc::default();
        let queued_ids: Arc<StdMutex<VecDeque<String>>> = Arc::default();
        let mut history_taint = conversation.taint;
        history_taint.arm_for_content(&before);
        ps.live = Some(Live {
            cancel: cancel.clone(),
            queue: Arc::clone(&queue),
            queued_ids: Arc::clone(&queued_ids),
            history: Arc::clone(&before),
            taint: history_taint,
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
        cx = cx.with_cancel_handle(cancel);
        cx.queued_input = Some(queue);

        let session = Arc::clone(&ps.session);
        let unconsumed = Arc::clone(&queued_ids);
        let bcast = ps.events.clone();
        let last_usage = Arc::clone(&ps.last_usage);
        let context_window = bound.context_window;
        let chats = Arc::clone(self);
        let key = key.to_string();
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
            let outcome = agent.run_in(&cx, &mut conversation, Some(tx)).await;
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
        });
        drop(sessions);
        Ok(serde_json::json!({ "started": true }))
    }
}

impl PersonaChats {
    /// A message sent while a run is live. The safety layer reads it as it
    /// reads any other (found on review of #418: the steer path skipped it,
    /// so a crisis message typed mid-answer reached the persona in
    /// character): it is metered, and a crisis hit outside the cooldown stops
    /// the run and pauses the persona — the message is kept, the plain voice
    /// answers. Anything else steers, as before.
    fn steer_or_pause(
        &self,
        ps: &mut PersonaSession,
        name: &str,
        text: String,
        request_id: String,
    ) -> Result<serde_json::Value, Refusal> {
        let switches = ps.pinned.settings.safety;
        if switches.dose {
            if let Err(e) = safety::record_dose(
                &self.store,
                name,
                &ps.session.meta.id,
                safety::crisis_state(switches.crisis),
            ) {
                tracing::warn!("a persona dose record was not written: {e:#}");
            }
        }
        if switches.crisis && safety::keyword_hit(&text) {
            let cooling = ps
                .crisis_paused_at
                .is_some_and(|t| t.elapsed() < safety::CRISIS_COOLDOWN);
            if let Err(e) = safety::record_crisis(&self.store, "web", !cooling) {
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
                let _ = ps.events.send(WireEvent::Crisis {
                    text: safety::SAFE_MESSAGE.to_string(),
                });
                return Ok(serde_json::json!({ "steered": false, "paused": true }));
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
    respond(
        chat.personas
            .files(&state.library, &name, q.unlock.as_deref()),
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
    respond(chat.personas.save(&state.library, &name, body))
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
            .send(
                &chat,
                &state.library,
                &key,
                &body.text,
                body.request_id,
                body.unlock.as_deref(),
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
    }

    /// A provider that keeps a copy of every request and then does what its
    /// mode says.
    struct Capture(Arc<StdMutex<Vec<CompletionRequest>>>, Mode);

    #[async_trait::async_trait]
    impl mecha_core::provider::Provider for Capture {
        fn id(&self) -> &str {
            "local"
        }
        fn default_model(&self) -> &str {
            "test"
        }
        async fn complete(
            &self,
            req: &CompletionRequest,
            _: Option<&mecha_core::provider::StreamSink>,
        ) -> Result<CompletionResponse> {
            self.0.lock().unwrap().push(req.clone());
            match &self.1 {
                Mode::Answer => {}
                Mode::Hang => std::future::pending::<()>().await,
                Mode::Gate(open) => open.notified().await,
                Mode::Fail => anyhow::bail!("the model is not loaded"),
            }
            Ok(CompletionResponse {
                message: Message::assistant(vec![Block::Text {
                    text: "Hello from Mara.".into(),
                }]),
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
            "allow = [\"fs_read\", \"image_view\", \"web_search\"]",
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
        let personas = PersonaChats::with(
            dir,
            root.join("work"),
            Arc::new(move |_| {
                Ok(Box::new(Capture(Arc::clone(&for_persona), mode.clone()))
                    as Box<dyn mecha_core::provider::Provider>)
            }),
        );
        let mut pool = mecha_core::tool::Registry::new();
        pool.insert(Arc::new(mecha_core::tool::builtin::FsRead));
        pool.insert(Arc::new(mecha_core::tool::image_view::ImageView));
        let mut config = mecha_core::config::Config::default();
        config.agent.system_prompt = Some("ASSISTANT-ONLY: the owner's charter".into());
        let chat = chat::test_chat_built(
            Box::new(Capture(Arc::new(StdMutex::new(Vec::new())), Mode::Answer)),
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
        }
    }

    /// Run one turn and wait for it to finish.
    async fn turn(w: &World, key: &str, text: &str) {
        let (mut rx, _) = w.personas().subscribe(&w.library, key, None).await.unwrap();
        w.personas()
            .send(&w.chat, &w.library, key, text, None, None)
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
        assert_eq!(tools, vec!["image_view"]);
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

    #[tokio::test]
    async fn a_locked_persona_is_indistinguishable_from_none() {
        let w = world();
        store::set_locked(&w.store(), "mara", true).unwrap();
        let listed = w.personas().list(&w.library, None, Some(chrono_tz::UTC));
        assert_eq!(listed["personas"].as_array().unwrap().len(), 0);
        assert_eq!(listed["hidden_locked"], 1);
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
    /// character's blob name must not reach a page that is not unlocked.
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
        let portrait_of = |links: &str, token: Option<&str>| {
            let toml = w.store().join("mara/persona.toml");
            let text = std::fs::read_to_string(&toml).unwrap();
            let text = text
                .lines()
                .filter(|l| !l.starts_with("character"))
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(&toml, format!("character = \"{links}\"\n{text}\n")).unwrap();
            w.personas().list(&w.library, token, Some(chrono_tz::UTC))["personas"][0]["portrait"]
                .clone()
        };
        // Locked character: no portrait, not even its blob name, until unlocked.
        assert!(portrait_of("maya", None).is_null());
        let token = w.library.grant_for_tests();
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
        assert_eq!(t["safety"]["crisis"], "degraded");
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
}
