//! A persona turn's run context: what a persona's run is sent with beyond
//! its messages, built in one place so that serve and a replay send the same.
//!
//! The persona chat used to build this inline in `serve`, and the one replay
//! rig was a Python copy of it. That copy drifted twice in one day, both
//! times in ways that changed a measured answer: the copy serialised history
//! in insertion order where serve's serde sorts keys (14/23 against the
//! shipped 1/25), and a staged record lost its photo's hash. A replay that
//! calls this function cannot drift that way, because serve calls it too
//! (`mecha replay --persona`, `docs/ARCHITECTURE.md` §Session records and
//! replay).
//!
//! What stays with the caller is the run's plumbing: the cancel handle, the
//! steering queue, the job sink a picture is deferred into, and the closing
//! line a panel turn sets once its draw is known. None of them changes the
//! first request a turn sends.

use crate::agent::{Agent, RunContext};
use crate::message::{Message, Role};
use crate::provider::Provider;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

/// The most turns a persona's run may take when its config sets none.
pub const MAX_TURNS: u32 = 40;

/// A persona's thinking is its reply being drafted: a short block plans,
/// and a long one re-drafts the same lines until the server's budget runs
/// out — up to 23 copies of one sentence, 40–50 s of silence on a call.
/// Replayed on nine of a chat's turns, judged blind by the same model with
/// the order swapped: a 512-token cap lost to the full budget 10–20 and
/// ended replies mid-sentence three times as often (it stops the draft
/// half-written); a 1024-token cap beat the full budget 24–10, with the
/// slowest reply 12.5 s against 41.3 s.
pub const SPOKEN_THINK_BUDGET: u32 = 1024;

/// Where a turn's one-shot readers get their model: the role splitter and
/// the scene reader each make a request of their own, on the persona's model
/// and untouched by its settings (`PersonaUse::Judge` in `setup`).
pub enum Readers<'a> {
    /// Built from this, once per reader. A reader that cannot be had is
    /// left out with a warning, and the tool draws the call as sent.
    From(&'a (dyn Fn() -> Result<Box<dyn Provider>> + Sync)),
    /// None stamped: a replay whose tools do not run, or an arm that
    /// measures the call without them.
    Off,
}

/// One persona turn, as its run is built.
pub struct Turn<'a> {
    /// The chat's jail.
    pub workspace: PathBuf,
    /// The chat's scene slot (`scene::persona_slot`).
    pub scene: Option<crate::scene::SceneSlot>,
    /// Where each picture's prompt is appended, read by no tool.
    pub prompt_log: Option<PathBuf>,
    /// The owner's words this turn, as typed — not the goal folded ahead of
    /// a first turn.
    pub owner: &'a str,
    /// The messages the run starts from, the owner's turn included.
    pub history: &'a [Message],
    /// The picture edit panel sent this turn: the panel's own reader drew
    /// the change, so no scene reader is stamped (a second read could turn
    /// a clothes edit into a pose, review of #610).
    pub panel: bool,
    /// A spoken turn, which caps thinking (`SPOKEN_THINK_BUDGET`).
    pub spoken: bool,
    /// The run's notes (PERSONA-CONTEXT-DESIGN.md §5.1).
    pub notes: Vec<String>,
    /// The persona's name, its library character, and the library: whom a
    /// reader may name.
    pub persona: String,
    pub character: Option<String>,
    pub library: PathBuf,
    /// The model the readers ask.
    pub model: String,
    pub readers: Readers<'a>,
}

/// The persona's latest reply before this turn, in words: what the scene
/// reader reads beside the owner's ask.
pub fn latest_reply(history: &[Message]) -> Option<String> {
    history
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant && !m.text().trim().is_empty())
        .map(|m| m.text())
}

/// The run context a persona turn runs under: the agent's own, jailed to the
/// chat's workspace, with the turn's stamps, notes and budgets. No brief,
/// homeostat, outbox or hooks: each is the owner's.
pub fn context(agent: &Agent, turn: Turn<'_>) -> RunContext {
    let mut cx = (**agent.context()).clone();
    let mut tools = agent.ctx().for_session(turn.workspace);
    // This chat's scene (IMAGE-DESIGN.md §6): its own copy beside its
    // transcript, and the persona's latest and index in its folder, all
    // outside the jail. Stamped by the caller's slot, never by a model.
    tools.scene = turn.scene;
    // Each picture's prompt, saved beside the transcript for the owner
    // ("save them for now", 2026-10-08).
    tools.prompt_log = turn.prompt_log;
    // `for_session` clones the agent's context, so each arm is set: `Off`
    // clears what the base might carry rather than inheriting it, since a
    // replay's header reports it as a fact (review of #612, pass 4).
    if let Readers::Off = turn.readers {
        tools.role_split = None;
        tools.scene_reader = None;
    }
    if let Readers::From(provider) = turn.readers {
        // Each person's part of a scene's `together`, read on the persona's
        // own model, untouched as the edit panel's reader is (`roles`): the
        // persona puts the whole act in one sentence, and drawn as one it
        // duplicated a person in 6 of 12 real calls. A model that cannot be
        // had leaves the tool drawing the call as sent.
        tools.role_split = match provider() {
            Ok(p) => Some(
                Arc::new(crate::roles::ModelSplit::new(p, turn.model.clone()))
                    as Arc<dyn crate::roles::RoleSplit>,
            ),
            Err(e) => {
                tracing::warn!("persona chat: no role splitter this turn: {e:#}");
                None
            }
        };
        // This turn's ask, read for what the persona's picture call leaves
        // out (`SceneReader`): the owner's words and the persona's latest
        // reply, on its own model. Not on a panel turn, whose change the
        // panel's own reader already drew from these words.
        if !turn.panel {
            tools.scene_reader = match provider() {
                Ok(provider) => Some(Arc::new(crate::persona::edit::ModelReader {
                    provider,
                    model: turn.model.clone(),
                    owner: turn.owner.to_string(),
                    reply: latest_reply(turn.history),
                    persona: turn.persona.clone(),
                    character: turn.character.clone(),
                    library: turn.library.clone(),
                })
                    as Arc<dyn crate::persona::edit::SceneReader>),
                Err(e) => {
                    tracing::warn!("persona chat: no scene reader this turn: {e:#}");
                    None
                }
            };
        }
    }
    cx.tools = Arc::new(tools);
    if cx.budget.max_turns.is_none() {
        cx.budget.max_turns = Some(MAX_TURNS);
    }
    cx.notes = turn.notes.into();
    // A persona's run ends as soon as its picture is queued: nothing after
    // the picture is the persona's job (IMAGE-DESIGN.md §5.5).
    cx.end_after_deferral = true;
    // Someone is waiting in silence on a spoken turn.
    if turn.spoken {
        cx = cx.with_think_budget(SPOKEN_THINK_BUDGET);
    }
    cx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latest_reply_skips_a_reply_with_no_words() {
        let history = vec![
            Message::user("first"),
            Message::assistant(vec![crate::message::Block::text("an answer")]),
            Message::user("second"),
            Message::assistant(vec![crate::message::Block::text("  ")]),
            Message::user("third"),
        ];
        assert_eq!(latest_reply(&history).as_deref(), Some("an answer"));
        assert_eq!(latest_reply(&history[..1]), None);
    }
}
