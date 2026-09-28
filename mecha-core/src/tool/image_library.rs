//! `image_library` and `image_library_propose`: the model's two doors into the
//! image library (`docs/IMAGE-COMPILER-DESIGN.md` §4).
//!
//! ## What the lookup returns decides what it declares
//!
//! `image_library` returns **approved entries only**, and so declares
//! [`Capabilities::default`], on `skill`'s footing: every approved entry's
//! text crossed the owner, either because the owner wrote it or because the
//! owner approved it — and an untrusted candidate's text is shown to the owner
//! before it can be approved. Candidates never come back through it. They
//! could not be returned safely by marking only the result: the loop taints on
//! `caps.untrusted_input && out.external`, with `caps` read per tool, so the
//! capability would have to be declared on the tool — which would arm every
//! lookup, approved-only included.
//!
//! ## A proposal changes nothing the owner uses
//!
//! `image_library_propose` stages a candidate and nothing else: a candidate is
//! never compiled into a prompt and never listed to the model. So it is
//! `read_only` on the footing `image_generate` has — a web chat starts
//! read-only, and proposing a character should be one request in it — and it
//! is bounded instead: names cannot collide with any existing entry, and past
//! [`crate::imagelib::MAX_PENDING`] candidates it refuses. Its `origin` is
//! recorded from the conversation's taint, never from what the model says.

use super::{Capabilities, Tool, ToolCtx, ToolOutput};
use crate::imagelib::{self, Kind, Library, NewEntry, Origin, Status};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;

/// Lists and searches the owner's approved characters and styles.
pub struct ImageLibrary {
    dir: PathBuf,
}

impl ImageLibrary {
    pub fn new(dir: PathBuf) -> Self {
        ImageLibrary { dir }
    }
}

fn parse_kind(input: &Value) -> std::result::Result<Option<Kind>, String> {
    match input.get("kind").and_then(Value::as_str) {
        None | Some("all") => Ok(None),
        Some("character") => Ok(Some(Kind::Character)),
        Some("style") => Ok(Some(Kind::Style)),
        Some(other) => Err(format!(
            "`kind` must be character, style or all, not `{other}`."
        )),
    }
}

#[async_trait]
impl Tool for ImageLibrary {
    fn name(&self) -> &str {
        "image_library"
    }

    fn description(&self) -> &str {
        "List the owner's recurring characters and styles for image_generate — each with its name \
         and description. Name characters in image_generate's cast and a style in its style; the \
         library supplies how they look. Optionally filter by kind or search by a word."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "kind": {"type": "string", "enum": ["character", "style", "all"]},
                "query": {"type": "string", "description": "A word to match in names and descriptions."}
            }
        })
    }

    fn read_only(&self) -> bool {
        true
    }

    /// The owner's own text, approved entries only — see the module doc.
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    async fn call(&self, input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
        let kind = match parse_kind(&input) {
            Ok(kind) => kind,
            Err(why) => return Ok(ToolOutput::err(why)),
        };
        let query = input
            .get("query")
            .and_then(Value::as_str)
            .map(|q| q.trim().to_lowercase())
            .filter(|q| !q.is_empty());
        let (lib, _) = Library::load(&self.dir);
        let lines: Vec<String> = lib
            .approved()
            .filter(|e| kind.is_none_or(|k| e.kind == k))
            .filter(|e| {
                query
                    .as_deref()
                    .is_none_or(|q| e.name.contains(q) || e.text.to_lowercase().contains(q))
            })
            .map(|e| format!("{} {} (v{}): {}", e.kind.label(), e.name, e.version, e.text))
            .collect();
        if lines.is_empty() {
            let what = if query.is_some() || kind.is_some() {
                "Nothing in the image library matches."
            } else {
                "The image library is empty. The owner adds characters and styles; you can \
                 propose one with image_library_propose."
            };
            return Ok(ToolOutput::ok(what));
        }
        // Said in the result, where the model reads it at the moment it
        // decides: the first real run copied these descriptions into the
        // prompt and left `cast` out, and drew two strangers.
        let how = "To draw a character, put its name in image_generate's `cast` with what they \
                   are wearing and doing; their look comes from their portrait. Do not copy \
                   these descriptions into the prompt. Put a style's name in `style`.";
        Ok(ToolOutput::ok(format!("{}\n\n{how}", lines.join("\n"))))
    }
}

/// Stages a candidate character or style for the owner to approve.
pub struct ImageLibraryPropose {
    dir: PathBuf,
}

impl ImageLibraryPropose {
    pub fn new(dir: PathBuf) -> Self {
        ImageLibraryPropose { dir }
    }
}

#[async_trait]
impl Tool for ImageLibraryPropose {
    fn name(&self) -> &str {
        "image_library_propose"
    }

    fn description(&self) -> &str {
        "Propose a new recurring character or style for the owner's image library. A character \
         needs a portrait — a front-facing image already in the workspace, usually one you just \
         generated — and a short description including build and height. A style needs its \
         text. It waits for the owner's approval and cannot be used until then."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "kind": {"type": "string", "enum": ["character", "style"]},
                "name": {"type": "string", "description": "Lowercase letters, digits and hyphens."},
                "text": {"type": "string", "description": "A character's short description, or a style's text."},
                "portrait": {"type": "string", "description": "A character's portrait: a workspace path (images/...)."},
                "seed": {"type": "integer", "minimum": 0, "description": "The seed that drew the portrait, if you generated it."}
            },
            "required": ["kind", "name", "text"]
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
        let kind = match input.get("kind").and_then(Value::as_str) {
            Some("character") => Kind::Character,
            Some("style") => Kind::Style,
            _ => return Ok(ToolOutput::err("`kind` must be character or style.")),
        };
        let field = |k: &str| {
            input
                .get(k)
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default()
                .to_string()
        };
        let (name, text) = (field("name"), field("text"));
        let portrait = match (kind, input.get("portrait").and_then(Value::as_str)) {
            (Kind::Character, Some(path)) => {
                match crate::imagegen::read_references(ctx, &[path.trim().to_string()]).await {
                    Ok(mut refs) => Some(refs.remove(0).bytes),
                    Err(why) => return Ok(ToolOutput::err(why)),
                }
            }
            (Kind::Character, None) => {
                return Ok(ToolOutput::err(
                    "A character needs a `portrait`: a front-facing image in the workspace.",
                ))
            }
            (Kind::Style, Some(_)) => return Ok(ToolOutput::err("A style has no portrait.")),
            (Kind::Style, None) => None,
        };
        let origin = Origin::of_proposal(ctx.taint.as_ref());
        let made = imagelib::create(
            &self.dir,
            NewEntry {
                kind,
                name,
                text,
                portrait,
                source_seed: input.get("seed").and_then(Value::as_u64),
                origin,
                locked: false,
            },
        );
        match made {
            Ok(entry) => {
                debug_assert_eq!(entry.status, Status::Candidate);
                Ok(ToolOutput::ok(format!(
                    "Proposed the {} `{}`. It waits for the owner's approval (`mecha imagelib \
                     approve {}`, or the web library) and cannot be used until then.",
                    kind.label(),
                    entry.name,
                    entry.name
                )))
            }
            Err(e) => Ok(ToolOutput::err(format!("Not proposed: {e:#}."))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Taint;

    fn scratch() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mecha-imagelib-tool-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn png() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([9, 9, 9]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn ctx(workspace: &std::path::Path, taint: Option<Taint>) -> ToolCtx {
        ToolCtx {
            workspace: workspace.to_path_buf(),
            taint,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn a_proposal_is_a_candidate_the_lookup_never_returns() {
        let lib = scratch();
        let ws = scratch();
        std::fs::create_dir_all(ws.join("images")).unwrap();
        std::fs::write(ws.join("images/p.png"), png()).unwrap();
        let propose = ImageLibraryPropose::new(lib.clone());
        let out = propose
            .call(
                json!({"kind": "character", "name": "theo", "text": "theo, tall and lean",
                       "portrait": "images/p.png", "seed": 42}),
                &ctx(&ws, Some(Taint::default())),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        let (loaded, _) = Library::load(&lib);
        let e = loaded.get(Kind::Character, "theo").unwrap();
        assert_eq!(e.status, Status::Candidate);
        assert_eq!(e.origin, Origin::ModelClean);
        assert_eq!(e.source_seed, Some(42));

        let list = ImageLibrary::new(lib.clone())
            .call(json!({}), &ctx(&ws, None))
            .await
            .unwrap();
        assert!(!list.content.contains("theo"), "{}", list.content);

        imagelib::approve(&lib, Kind::Character, "theo").unwrap();
        let list = ImageLibrary::new(lib.clone())
            .call(json!({"query": "LEAN"}), &ctx(&ws, None))
            .await
            .unwrap();
        assert!(
            list.content
                .starts_with("character theo (v1): theo, tall and lean\n"),
            "{}",
            list.content
        );
        assert!(list.content.contains("`cast`"), "{}", list.content);
        assert!(
            !list.external,
            "the owner's own text is not third-party content"
        );
        std::fs::remove_dir_all(lib).ok();
        std::fs::remove_dir_all(ws).ok();
    }

    #[tokio::test]
    async fn a_proposal_from_a_tainted_or_unknown_conversation_is_untrusted() {
        let lib = scratch();
        let ws = scratch();
        let propose = ImageLibraryPropose::new(lib.clone());
        for (name, taint) in [
            (
                "dirty",
                Some(Taint {
                    untrusted: true,
                    ..Taint::default()
                }),
            ),
            ("unknown", None),
        ] {
            propose
                .call(
                    json!({"kind": "style", "name": name, "text": "watercolour"}),
                    &ctx(&ws, taint),
                )
                .await
                .unwrap();
        }
        let (loaded, _) = Library::load(&lib);
        for name in ["dirty", "unknown"] {
            assert_eq!(
                loaded.get(Kind::Style, name).unwrap().origin,
                Origin::ModelUntrusted
            );
        }
        std::fs::remove_dir_all(lib).ok();
        std::fs::remove_dir_all(ws).ok();
    }

    #[tokio::test]
    async fn a_portrait_outside_the_workspace_is_refused() {
        let lib = scratch();
        let ws = scratch();
        let out = ImageLibraryPropose::new(lib.clone())
            .call(
                json!({"kind": "character", "name": "x", "text": "x",
                       "portrait": "/etc/hostname"}),
                &ctx(&ws, Some(Taint::default())),
            )
            .await
            .unwrap();
        assert!(out.is_error, "{}", out.content);
        assert!(Library::load(&lib).0.all().is_empty());
        std::fs::remove_dir_all(lib).ok();
        std::fs::remove_dir_all(ws).ok();
    }

    #[test]
    fn neither_door_arms_anything() {
        for caps in [
            ImageLibrary::new(PathBuf::new()).capabilities(),
            ImageLibraryPropose::new(PathBuf::new()).capabilities(),
        ] {
            assert!(!caps.private_data && !caps.untrusted_input && !caps.destructive);
            assert_eq!(caps.egress, crate::tool::Egress::None);
        }
    }
}
