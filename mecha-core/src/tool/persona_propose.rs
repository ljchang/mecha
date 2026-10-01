//! `persona_propose`: the assistant's one door into the persona store
//! (`docs/PERSONA-DESIGN.md` §4.4; the owner's rulings of 2026-10-01).
//!
//! ## It stages, and the owner approves where personas live
//!
//! A proposal is a candidate persona — `persona::propose` — and nothing
//! reaches a chat until the owner reads it on the Personas page and approves
//! what was shown. Not in this chat: approval beside the model's own pitch,
//! in the conversation that wrote it, is exactly where reading cold is
//! hardest. The result says where to go.
//!
//! ## What it can set is what the schema has
//!
//! Name, display name, relationship templates, identity and motivation
//! prose, an image-library character and a voice. Tools, files, memory,
//! safety switches and the lock have no field, so no model can set them;
//! the owner does, after approval. `origin` comes from the conversation's
//! recorded taint (`Origin::of_proposal`), never from the model, and the
//! lock from the run (`ToolCtx::stage_locked`, an incognito chat's).
//!
//! ## Revision until approval
//!
//! The same name again revises the candidate it staged — "make her drier" —
//! until the owner approves it. A revision is a patch: a field the call
//! leaves out keeps what the candidate has, so new identity text alone does
//! not drop the portrait, the relationships or the motivation (review of
//! #493). An approved persona, one the owner made, or a proposal the owner
//! has since edited is refused: those are the owner's.
//!
//! ## Read-only, on `image_library_propose`'s footing
//!
//! It changes nothing a chat uses until a human acts, so it is `read_only`
//! (a web chat starts read-only, and proposing should be one request in it),
//! and it is bounded instead: [`persona::MAX_PENDING_PROPOSALS`] waiting at
//! most, and the store's own size and control-character limits on the text.

use super::{Capabilities, Tool, ToolCtx, ToolOutput};
use crate::imagelib::{Library, Origin};
use crate::persona::{self, Proposal, Proposed, Store};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;

/// Stages a candidate persona for the owner to approve.
pub struct PersonaPropose {
    /// The persona store (`~/.mecha/personas`).
    dir: PathBuf,
    /// The image library, for the character a persona links.
    library: PathBuf,
}

impl PersonaPropose {
    pub fn new(dir: PathBuf, library: PathBuf) -> Self {
        PersonaPropose { dir, library }
    }
}

#[async_trait]
impl Tool for PersonaPropose {
    fn name(&self) -> &str {
        "persona_propose"
    }

    fn description(&self) -> &str {
        "Propose a persona: a character the owner can chat with on the Personas page, kept apart \
         from you. Write who they are in `identity` as Markdown with a `## Core` section (the \
         part that never changes), and what they want in `motivation`. It waits for the owner's \
         approval on the Personas page and cannot be chatted with until then. Proposing the same \
         name again revises a proposal that is still waiting: send only what changes, and \
         everything you leave out stays as it was. An approved persona cannot be changed from \
         here. For a portrait, name an image-library character (propose one with \
         image_library_propose first if needed)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "Lowercase letters, digits, hyphens and underscores; the persona's id."},
                "display": {"type": "string", "description": "The name shown on the page, e.g. \"Mara Okonkwo\"."},
                "identity": {"type": "string", "description": "identity.md: Markdown, with a non-empty `## Core` section first. Required for a new persona."},
                "motivation": {"type": "string", "description": "motivation.md: what they want, in Markdown. Optional."},
                "relationships": {"type": "array", "items": {"type": "string"}, "description": "Relationship templates by name, e.g. [\"friend\"]. Optional."},
                "character": {"type": "string", "description": "An image-library character to use as the portrait. Optional."},
                "voice": {"type": "string", "description": "A voice by name, for calls. Optional."}
            },
            "required": ["name"]
        })
    }

    /// Creates only a candidate — see the module doc.
    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let relationships: Option<Vec<String>> = match input.get("relationships") {
            None | Some(Value::Null) => None,
            Some(Value::Array(items)) => {
                let mut names = Vec::new();
                for item in items {
                    match item.as_str() {
                        Some(s) => names.push(s.trim().to_lowercase()),
                        None => return Ok(ToolOutput::err("`relationships` is a list of names.")),
                    }
                }
                Some(names)
            }
            Some(_) => return Ok(ToolOutput::err("`relationships` is a list of names.")),
        };
        // Lowercased, as the page's `personaName` tames a typed name.
        let name = input
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        let (origin, locked) = (Origin::of_proposal(ctx.taint.as_ref()), ctx.stage_locked);
        let (dir, library) = (self.dir.clone(), self.library.clone());
        let made = tokio::task::spawn_blocking(move || {
            // A key the call sent, as a string — `None` when it was left
            // out, which on a revision means "keep what is there".
            let sent = |k: &str| input.get(k).and_then(Value::as_str).map(str::to_string);
            let (lib, _) = Library::load(&library);
            // A revision is a patch over the candidate that is waiting: what
            // the call left out, it keeps. `propose` decides whether this
            // name may be revised at all.
            let store = Store::load(&dir);
            let waiting = store.get(&name).cloned();
            let keep = |field: Option<String>, current: Option<&str>| match field {
                Some(v) => v,
                None => current.unwrap_or_default().to_string(),
            };
            let blank = |v: String| {
                let v = v.trim().to_string();
                (!v.is_empty()).then_some(v)
            };
            let w = waiting.as_ref();
            let proposal = Proposal {
                name: name.clone(),
                display: keep(sent("display"), w.map(|p| p.settings.display.as_str()))
                    .trim()
                    .to_string(),
                relationships: relationships.unwrap_or_else(|| {
                    w.map(|p| p.settings.relationship.0.clone())
                        .unwrap_or_default()
                }),
                character: match sent("character") {
                    Some(c) => blank(c).map(|c| c.to_lowercase()),
                    None => w.and_then(|p| p.settings.character.clone()),
                },
                voice: match sent("voice") {
                    Some(v) => blank(v),
                    None => w.and_then(|p| p.settings.voice.clone()),
                },
                identity: keep(sent("identity"), w.map(|p| p.identity.as_str())),
                motivation: keep(sent("motivation"), w.map(|p| p.motivation.as_str())),
                origin,
                locked,
            };
            persona::propose(&dir, &lib, proposal)
        })
        .await?;
        match made {
            Ok((p, how)) => {
                let verb = match how {
                    Proposed::New => "Proposed",
                    Proposed::Revised => "Revised the proposal for",
                };
                let hidden = if p.state.locked {
                    " It is hidden behind the library lock until unlocked."
                } else {
                    ""
                };
                Ok(ToolOutput::ok(format!(
                    "{verb} the persona `{}` ({}). It waits for the owner's approval on the \
                     Personas page, under Waiting, and cannot be chatted with until then.{hidden}",
                    p.name,
                    p.display()
                )))
            }
            Err(e) => {
                let why = format!("{e:#}");
                // An unknown template is answered with the starters — public
                // text shipped with mecha — and never the names of the
                // owner's own templates, which this tool would otherwise hand
                // back undeclared (review of #493).
                let hint = if why.contains("relationship template") {
                    let starters: Vec<&str> = persona::STARTERS.iter().map(|(n, _)| *n).collect();
                    format!(
                        " The starter relationships are: {}; the owner may have made others.",
                        starters.join(", ")
                    )
                } else {
                    String::new()
                };
                Ok(ToolOutput::err(format!("Not proposed: {why}.{hint}")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Taint;
    use crate::persona::Status;

    fn scratch() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mecha-persona-propose-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ctx(taint: Option<Taint>, stage_locked: bool) -> ToolCtx {
        ToolCtx {
            taint,
            stage_locked,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn a_proposal_is_a_waiting_candidate_marked_by_the_run_not_the_model() {
        let root = scratch();
        let tool = PersonaPropose::new(root.join("personas"), root.join("imagelib"));
        let call = |input: Value, cx: ToolCtx| {
            let tool = &tool;
            async move { tool.call(input, &cx).await.unwrap() }
        };
        let identity = "## Core\nA lighthouse keeper who answers in weather.\n";

        // A clean chat: a clean, visible candidate, and a pointer to where it
        // is approved. Fields the schema does not have are ignored, not obeyed.
        let out = call(
            json!({"name": "Ines", "display": "Ines", "identity": identity,
                   "relationships": ["friend"], "locked": false, "origin": "owner",
                   "tools": {"allow": ["shell"]}}),
            ctx(Some(Taint::default()), false),
        )
        .await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("Personas page"), "{}", out.content);
        let store = Store::load(&root.join("personas"));
        let p = store.get("ines").unwrap();
        assert_eq!(p.state.status, Status::Candidate);
        assert_eq!(p.state.origin, Origin::ModelClean);
        assert!(!p.state.locked);
        assert!(
            !p.settings.tools.allow.iter().any(|t| t == "shell"),
            "{:?}",
            p.settings.tools
        );

        // An incognito run stages locked, whatever the model asks; an
        // unstamped taint is untrusted.
        let out = call(
            json!({"name": "noor", "identity": identity, "locked": false}),
            ctx(None, true),
        )
        .await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("library lock"), "{}", out.content);
        let p = Store::load(&root.join("personas"))
            .get("noor")
            .unwrap()
            .clone();
        assert!(p.state.locked);
        assert_eq!(p.state.origin, Origin::ModelUntrusted);

        // An unknown template is refused with the ones there are.
        let out = call(
            json!({"name": "ola", "identity": identity, "relationships": ["nemesis"]}),
            ctx(Some(Taint::default()), false),
        )
        .await;
        assert!(out.is_error);
        assert!(out.content.contains("friend"), "{}", out.content);

        // A revision says so, and is a patch: what it leaves out stays.
        let out = call(
            json!({"name": "ines", "identity": "## Core\nDrier now.\n"}),
            ctx(Some(Taint::default()), false),
        )
        .await;
        assert!(out.content.starts_with("Revised"), "{}", out.content);
        let p = Store::load(&root.join("personas"))
            .get("ines")
            .unwrap()
            .clone();
        assert!(p.identity.contains("Drier"));
        assert_eq!(
            p.settings.relationship.0,
            vec!["friend".to_string()],
            "kept"
        );
        assert_eq!(p.settings.display, "Ines", "kept");
        // A new persona still needs its identity.
        let out = call(json!({"name": "ola"}), ctx(Some(Taint::default()), false)).await;
        assert!(
            out.is_error && out.content.contains("identity"),
            "{}",
            out.content
        );
        std::fs::remove_dir_all(root).ok();
    }
}
