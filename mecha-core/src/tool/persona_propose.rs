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
//! The same name again rewrites the candidate it staged — "make her drier" —
//! until the owner approves it. An approved persona, or one the owner made,
//! is refused: those are the owner's to edit.
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
         name again revises a proposal that is still waiting; an approved persona cannot be \
         changed from here. For a portrait, name an image-library character (propose one with \
         image_library_propose first if needed)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "Lowercase letters, digits, hyphens and underscores; the persona's id."},
                "display": {"type": "string", "description": "The name shown on the page, e.g. \"Mara Okonkwo\"."},
                "identity": {"type": "string", "description": "identity.md: Markdown, with a non-empty `## Core` section first."},
                "motivation": {"type": "string", "description": "motivation.md: what they want, in Markdown. Optional."},
                "relationships": {"type": "array", "items": {"type": "string"}, "description": "Relationship templates by name, e.g. [\"friend\"]. Optional."},
                "character": {"type": "string", "description": "An image-library character to use as the portrait. Optional."},
                "voice": {"type": "string", "description": "A voice by name, for calls. Optional."}
            },
            "required": ["name", "identity"]
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
        let text = |k: &str| {
            input
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let optional = |k: &str| {
            input
                .get(k)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let relationships: Vec<String> = match input.get("relationships") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(items)) => {
                let mut names = Vec::new();
                for item in items {
                    match item.as_str() {
                        Some(s) => names.push(s.trim().to_lowercase()),
                        None => return Ok(ToolOutput::err("`relationships` is a list of names.")),
                    }
                }
                names
            }
            Some(_) => return Ok(ToolOutput::err("`relationships` is a list of names.")),
        };
        let proposal = Proposal {
            // Lowercased, as the page's `personaName` tames a typed name.
            name: text("name").trim().to_lowercase(),
            display: text("display").trim().to_string(),
            relationships,
            character: optional("character").map(|c| c.to_lowercase()),
            voice: optional("voice"),
            identity: text("identity"),
            motivation: text("motivation"),
            origin: Origin::of_proposal(ctx.taint.as_ref()),
            locked: ctx.stage_locked,
        };
        let (dir, library) = (self.dir.clone(), self.library.clone());
        let made = tokio::task::spawn_blocking(move || {
            let (lib, _) = Library::load(&library);
            let result = persona::propose(&dir, &lib, proposal);
            // A refusal naming a template that is not there lists the ones
            // that are, so the model can correct it in one step.
            let known = Store::load(&dir)
                .relationship_names()
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>();
            (result, known)
        })
        .await?;
        match made {
            (Ok((p, how)), _) => {
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
            (Err(e), known) => {
                let why = format!("{e:#}");
                let hint = if why.contains("relationship template") && !known.is_empty() {
                    format!(" Relationships there are: {}.", known.join(", "))
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

        // A revision says so.
        let out = call(
            json!({"name": "ines", "identity": "## Core\nDrier now.\n"}),
            ctx(Some(Taint::default()), false),
        )
        .await;
        assert!(out.content.starts_with("Revised"), "{}", out.content);
        std::fs::remove_dir_all(root).ok();
    }
}
