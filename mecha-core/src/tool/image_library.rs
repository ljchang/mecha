//! `image_library`, `image_library_propose` and `image_library_reference`: the
//! model's doors into the image library (`docs/IMAGE-COMPILER-DESIGN.md` §4,
//! §9.3).
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
//! ## It declares the private axis, because the library may hold people
//!
//! Untrusted is off; private is on. A character entry is a physical
//! description of a person, and the owner ruled against marking which entries
//! are real people (research §5, 2026-09-28) — so a lookup cannot tell an
//! invented character from someone's likeness, and unknown is never clean.
//! This is `goal_context`'s footing (the owner's charter, text only, declared
//! private), not `skill`'s (a procedure). The cost, stated: a conversation
//! that looked the library up and later reads untrusted content cannot use a
//! `Chosen` sender. `image_generate` itself returns only names and versions,
//! so drawing a character arms nothing (found on review of #383).
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
//!
//! In an incognito chat it stages **locked** (`ToolCtx::stage_locked`), on
//! `persona_propose`'s footing (`docs/INCOGNITO-DESIGN.md` R3, the owner's
//! ruling of 2026-10-12): the candidate is hidden behind the library lock
//! until the owner unlocks it, so it is a write the owner keeps on purpose,
//! not a trace of the room. The schema has no `locked` field, so the model
//! cannot ask its way out of the lock — nor into one elsewhere.
//!
//! ## A reference lands directly only from a clean conversation
//!
//! `image_library_reference` attaches or removes a character's typed picture
//! (the owner's ruling of 2026-10-14, amending §9's owner-only rule). A
//! reference rides into every later picture of that character, so how the
//! conversation stands decides how it lands, as provenance gates learning:
//! a conversation that has read nothing from outside writes it directly, as
//! a new version the owner can revert from the character's page; one that
//! has, or whose taint is unknown, stages it for the owner's approval in the
//! Waiting pane; an incognito chat stages it locked. It is `read_only` on
//! `image_generate`'s footing — a web chat starts read-only, and attaching a
//! picture the owner just asked for should be one request in it — with the
//! provenance split, not the permission mode, as what bounds it.

use super::{Capabilities, Tool, ToolCtx, ToolOutput};
use crate::imagelib::{self, Kind, Library, NewEntry, Origin, Status};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

/// Lists and searches the owner's approved characters, styles and locations.
pub struct ImageLibrary {
    dir: PathBuf,
}

impl ImageLibrary {
    pub fn new(dir: PathBuf) -> Self {
        ImageLibrary { dir }
    }
}

/// Every entry kind's label, in `Kind::ALL`'s order: the one list a schema
/// enum and a refusal spell kinds from, so a new kind needs no edit here
/// (`IMAGE-COMPILER-DESIGN.md` §9.5; review of #740).
fn kind_labels() -> Vec<&'static str> {
    Kind::ALL.iter().map(|k| k.label()).collect()
}

fn kind_named(label: &str) -> Option<Kind> {
    Kind::ALL.into_iter().find(|k| k.label() == label)
}

fn parse_kind(input: &Value) -> std::result::Result<Option<Kind>, String> {
    match input.get("kind").and_then(Value::as_str) {
        None | Some("all") => Ok(None),
        Some(label) => kind_named(label).map(Some).ok_or_else(|| {
            format!(
                "`kind` must be {} or all, not `{label}`.",
                kind_labels().join(", ")
            )
        }),
    }
}

#[async_trait]
impl Tool for ImageLibrary {
    fn name(&self) -> &str {
        "image_library"
    }

    /// Eligible for a persona (`docs/PERSONA-DESIGN.md` §3.3): it reads only the
    /// owner's *approved* characters and styles, shared with personas on purpose
    /// (§8.6), and sends nothing.
    fn for_persona(self: Arc<Self>) -> Option<Arc<dyn Tool>> {
        Some(self)
    }

    fn description(&self) -> &str {
        "List the owner's recurring characters, styles, locations and outfits — each with its \
         name and description. Put a character's name as a `who` in image_generate's \
         `scene.people` and a style's in `scene.style`; the library supplies how they look. In \
         video_generate, a location's name goes in `scene.setting.location`, and an outfit's \
         name as a cast member's whole `wearing`. Optionally filter by kind or search by a word."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "kind": {"type": "string", "enum": kind_labels().into_iter().chain(["all"]).collect::<Vec<_>>()},
                "query": {"type": "string", "description": "Names or words: a name picks that entry; otherwise every word must appear in an entry's name or description."}
            }
        })
    }

    fn read_only(&self) -> bool {
        true
    }

    /// The owner's own text (approved entries only), and possibly a real
    /// person's description — see the module doc.
    fn capabilities(&self) -> Capabilities {
        Capabilities::default().private()
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
        let (lib, errors) = Library::load(&self.dir);
        let search = Search::new(&lib, query.as_deref(), kind);
        let lines: Vec<String> = lib
            .approved()
            .filter(|e| kind.is_none_or(|k| e.kind == k))
            .filter(|e| query.as_deref().is_none_or(|q| search.matches(e, q)))
            .map(|e| format!("{} {} (v{}): {}", e.kind.label(), e.name, e.version, e.text))
            .collect();
        if lines.is_empty() {
            let what = if query.is_some() || kind.is_some() {
                "Nothing in the image library matches."
            } else {
                "The image library is empty. The owner adds characters, styles, locations \
                 and outfits; you can propose one with image_library_propose."
            };
            return Ok(ToolOutput::ok(what));
        }
        // Said in the result, where the model reads it at the moment it
        // decides: the first real run copied these descriptions into the
        // prompt and left `cast` out, and drew two strangers. Said in the
        // scene's terms since the redesign: a result naming retired fields
        // led a chat to list nobody in `people` (2026-10-09).
        let how = "To draw a character, put its name as `who` in image_generate's \
                   `scene.people`, with what they are wearing and doing; their look comes from \
                   their portrait. Do not copy these descriptions into the scene. Anyone else \
                   in it — a waiter, a stranger — is a person too, described in their own \
                   `who`. Put a style's name in `scene.style`. In video_generate, a location's \
                   name in `scene.setting.location` and an outfit's name as someone's whole \
                   `wearing` bring their pictures.";
        // A broken entry is said, never silently absent.
        let broken = if errors.is_empty() {
            String::new()
        } else {
            format!(
                "\n{} entr{} could not be read; the owner can check with `mecha imagelib list`.",
                errors.len(),
                if errors.len() == 1 { "y" } else { "ies" }
            )
        };
        Ok(ToolOutput::ok(format!(
            "{}\n\n{how}{broken}",
            lines.join("\n")
        )))
    }
}

/// How a query picks entries (review of #384, twice).
///
/// - **A word that *is* an entry's name picks by name only.** The first live
///   run searched "Maya John"; matching that as one phrase found nothing, and
///   matching any word against descriptions returned the whole library on any
///   phrase ("and" is in "tall and lean") — every character's physical
///   description into context, on a tool declared private.
/// - **Otherwise every word of three letters or more**, against names and
///   descriptions — all of them, since "and" survives the floor; words under
///   three are dropped ("a" is in "priya") —
/// - **unless none is left**, when the whole query matches as one substring,
///   so a character named `jo` can still be found by its own name.
struct Search {
    by_name: Option<std::collections::BTreeSet<String>>,
}

impl Search {
    /// Names resolve against what the listing can return — approved entries
    /// of the kind asked for. Against every entry, a candidate's name
    /// collapsed a search to name-only mode and then was filtered out,
    /// returning nothing, and the emptiness told the model a candidate by
    /// that name existed (review of #384).
    fn new(lib: &Library, query: Option<&str>, kind: Option<Kind>) -> Search {
        let by_name = query.and_then(|q| {
            let names: std::collections::BTreeSet<String> = words(q)
                .filter(|w| {
                    lib.approved()
                        .any(|e| &e.name == w && kind.is_none_or(|k| e.kind == k))
                })
                .collect();
            (!names.is_empty()).then_some(names)
        });
        Search { by_name }
    }

    fn matches(&self, e: &crate::imagelib::Entry, q: &str) -> bool {
        if let Some(names) = &self.by_name {
            return names.contains(&e.name);
        }
        let text = e.text.to_lowercase();
        let mut long = words(q)
            .filter(|w| w.chars().count() >= 3 && !STOPWORDS.contains(&w.as_str()))
            .peekable();
        if long.peek().is_none() {
            return e.name.contains(q) || text.contains(q);
        }
        // Every long word, not any: "and" is three letters and sits in "tall
        // and lean", so any-word returned the whole library on a phrase with
        // no name in it — the query a search is for (review of #384).
        long.all(|w| e.name.contains(&w) || text.contains(&w))
    }
}

/// Common words of three letters or more, which every word must otherwise
/// match: "picnic in the park" missed `picnic-park` because "the" is in
/// neither its name nor its description (review of #384).
const STOPWORDS: &[&str] = &[
    "the", "and", "with", "for", "her", "his", "she", "him", "they", "them", "their", "this",
    "that", "these", "those", "from", "into", "onto", "over", "under", "who", "are", "was", "were",
    "has", "have", "its", "our", "your", "you", "one", "two", "some", "any", "all", "but", "not",
    "out", "off", "near", "while", "wearing",
];

fn words(q: &str) -> impl Iterator<Item = String> + '_ {
    q.split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
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
        "Propose a new recurring character, style, location or outfit for the owner's image \
         library. A character needs a portrait — a front-facing image already in the workspace, \
         usually one you just generated — and a short description including build and height. \
         A style needs its text; a location its description (fixtures, layout, light); an \
         outfit what it is (garment, colour, cut, neckline). The owner adds a location's or \
         outfit's pictures. It waits for the owner's approval and cannot be used until then."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "kind": {"type": "string", "enum": kind_labels()},
                "name": {"type": "string", "description": "Lowercase letters, digits and hyphens."},
                "text": {"type": "string", "description": "A character's short description, a style's text, a location's description, or what an outfit is."},
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
        let kind = match input
            .get("kind")
            .and_then(Value::as_str)
            .and_then(kind_named)
        {
            Some(kind) => kind,
            None => {
                return Ok(ToolOutput::err(format!(
                    "`kind` must be {}.",
                    kind_labels().join(", ")
                )))
            }
        };
        let field = |k: &str| {
            input
                .get(k)
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default()
                .to_string()
        };
        // Lowercased, as `compile` lowercases a cast name: "Maya" in a cast
        // works, so "Maya" in a proposal should too.
        let (name, text) = (field("name").to_lowercase(), field("text"));
        // A character already in the library takes new pictures as
        // references, never as a second proposal: said, so a model asked to
        // save a picture "to Wren" has somewhere to go (2026-10-14: four
        // identical refusals in a row, with nothing to try next).
        if kind == Kind::Character {
            let lib = Library::load(&self.dir).0;
            if lib
                .get(Kind::Character, &name)
                .is_some_and(|e| e.status == Status::Approved)
            {
                return Ok(ToolOutput::err(format!(
                    "Not proposed: `{name}` is already in the library. A new picture of them \
                     goes on with `image_library_reference` — as a face, a full_body, or a \
                     labelled detail — never as a new character."
                )));
            }
        }
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
            (Kind::Style | Kind::Location | Kind::Outfit, Some(_)) => {
                let what = kind.with_article();
                return Ok(ToolOutput::err(format!(
                    "{}{} has no portrait.",
                    what[..1].to_uppercase(),
                    &what[1..]
                )));
            }
            (Kind::Style | Kind::Location | Kind::Outfit, None) => None,
        };
        // What the conversation had read before this turn's batch
        // (`ToolCtx::recorded_taint`), not the send gate's union of the
        // batch's reach (review of #659).
        let (origin, locked) = (
            Origin::of_proposal(ctx.recorded_taint.as_ref()),
            ctx.stage_locked,
        );
        let made = imagelib::create(
            &self.dir,
            NewEntry {
                kind,
                name,
                text,
                portrait,
                source_seed: input.get("seed").and_then(Value::as_u64),
                origin,
                locked,
            },
        );
        match made {
            Ok(entry) => {
                debug_assert_eq!(entry.status, Status::Candidate);
                let hidden = if entry.locked {
                    " It is hidden behind the library lock until unlocked."
                } else {
                    ""
                };
                Ok(ToolOutput::ok(format!(
                    "Proposed the {} `{}`. It waits for the owner's approval (`mecha imagelib \
                     approve {}`, or the web library) and cannot be used until then.{hidden}",
                    kind.label(),
                    entry.name,
                    entry.name
                )))
            }
            Err(e) => Ok(ToolOutput::err(format!("Not proposed: {e:#}."))),
        }
    }
}

/// Attaches or removes a character's typed picture (`full_body`, `face`,
/// `detail`): directly from a clean conversation, staged for the owner
/// otherwise — see the module doc.
pub struct ImageLibraryReference {
    dir: PathBuf,
}

impl ImageLibraryReference {
    pub fn new(dir: PathBuf) -> Self {
        ImageLibraryReference { dir }
    }
}

#[async_trait]
impl Tool for ImageLibraryReference {
    fn name(&self) -> &str {
        "image_library_reference"
    }

    fn description(&self) -> &str {
        "Attach a picture to an existing library character, or remove one: a full_body (head to \
         toe, standing square-on: its stance carries into pictures), a face, or a detail such as \
         a tattoo, with a label saying what and where. The picture is a workspace path, usually \
         one you just generated or the owner attached. From a conversation that has read outside \
         content, it waits for the owner's approval instead of landing at once."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "The character's library name."},
                "as": {"type": "string", "enum": ["full_body", "face", "detail"]},
                "picture": {"type": "string", "description": "A workspace path (images/...); not needed to remove."},
                "label": {"type": "string", "description": "What it shows and where; required for a detail."},
                "remove": {"type": "boolean", "description": "Remove the character's reference of this kind (and label) instead."}
            },
            "required": ["name", "as"]
        })
    }

    /// Bounded by provenance, not by the permission mode — see the module doc.
    fn read_only(&self) -> bool {
        true
    }

    /// Nothing private comes back, only what the call named; but it writes an
    /// approved version of a character with no approver from a clean
    /// conversation, so it says so as every other writing builtin does — and
    /// that is what keeps it from a `research` child (review of #692).
    fn capabilities(&self) -> Capabilities {
        Capabilities::default().destructive()
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        // Planning is read-only exploration, and the phase gate admits this
        // tool on its `read_only` flag, as it admits `message_send`: so the
        // refusal is here, nothing written or staged (review of #696).
        if ctx.phase == crate::agent::Phase::Plan {
            return Ok(ToolOutput::err(
                "image_library_reference is not available while planning: it changes a \
                 library character. Nothing was attached or staged.",
            ));
        }
        let kind = match input.get("as").and_then(Value::as_str) {
            Some("full_body") => imagelib::RefKind::FullBody,
            Some("face") => imagelib::RefKind::Face,
            Some("detail") => imagelib::RefKind::Detail,
            _ => return Ok(ToolOutput::err("`as` must be full_body, face or detail.")),
        };
        let name = input
            .get("name")
            .and_then(Value::as_str)
            .map(|n| n.trim().to_lowercase())
            .unwrap_or_default();
        let label = input
            .get("label")
            .and_then(Value::as_str)
            .map(str::to_string);
        let remove = input
            .get("remove")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // Answered as `image_library` would: only what the call named, never
        // what else the library holds.
        let lib = Library::load(&self.dir).0;
        if lib
            .get(Kind::Character, &name)
            .is_none_or(|e| e.status != Status::Approved)
        {
            return Ok(ToolOutput::err(imagelib::missing(
                &lib,
                Kind::Character,
                &name,
            )));
        }
        let bytes = if remove {
            None
        } else {
            let Some(path) = input.get("picture").and_then(Value::as_str) else {
                return Ok(ToolOutput::err(
                    "Attaching a reference needs its `picture`: a workspace path (images/...).",
                ));
            };
            match crate::imagegen::read_references(ctx, &[path.trim().to_string()]).await {
                Ok(mut refs) => Some(refs.remove(0).bytes),
                Err(why) => return Ok(ToolOutput::err(why)),
            }
        };
        // What the conversation had read before this turn's batch, as a
        // proposal's origin is recorded (`ImageLibraryPropose`).
        let origin = Origin::of_proposal(ctx.recorded_taint.as_ref());
        let direct = origin == Origin::ModelClean && !ctx.stage_locked;
        if direct {
            let done = match &bytes {
                Some(bytes) => {
                    imagelib::add_reference_as(&self.dir, &name, kind, bytes, label, Some(origin))
                }
                None => imagelib::remove_reference(&self.dir, &name, kind, label.as_deref()),
            };
            return Ok(match done {
                Ok(entry) => ToolOutput::ok(format!(
                    "{} `{}`'s {} reference (now v{}). The owner can see and undo it on the \
                     character's page in the library.",
                    if remove { "Removed" } else { "Attached" },
                    entry.name,
                    kind.label(),
                    entry.version
                )),
                Err(e) => ToolOutput::err(format!("Not changed: {e:#}.")),
            });
        }
        let action = if remove {
            imagelib::PendingAction::Remove
        } else {
            imagelib::PendingAction::Add
        };
        match imagelib::propose_reference(
            &self.dir,
            &name,
            kind,
            action,
            bytes.as_deref(),
            label,
            origin,
            ctx.stage_locked,
        ) {
            Ok(p) => {
                let why = if ctx.stage_locked {
                    "an incognito chat stages it behind the library lock"
                } else {
                    "this conversation has read outside content"
                };
                let change = if remove { "removing" } else { "adding" };
                Ok(ToolOutput::ok(format!(
                    "Staged {change} `{}`'s {} reference for the owner's approval, since {why}: \
                     the library's Waiting pane, or `mecha imagelib approve-reference {}`. It \
                     is not used until then.",
                    p.name,
                    kind.label(),
                    p.id
                )))
            }
            Err(e) => Ok(ToolOutput::err(format!("Not staged: {e:#}."))),
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
            recorded_taint: taint,
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
        assert!(!e.locked, "an ordinary chat stages unlocked");
        assert!(!out.content.contains("library lock"), "{}", out.content);

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
        assert!(list.content.contains("`scene.people`"), "{}", list.content);
        assert!(!list.content.contains("`cast`") && !list.content.contains("`extras`"));
        assert!(
            !list.external,
            "the owner's own text is not third-party content"
        );
        std::fs::remove_dir_all(lib).ok();
        std::fs::remove_dir_all(ws).ok();
    }

    #[tokio::test]
    async fn an_incognito_proposal_stages_locked() {
        let lib = scratch();
        let ws = scratch();
        std::fs::create_dir_all(ws.join("images")).unwrap();
        std::fs::write(ws.join("images/p.png"), png()).unwrap();
        let propose = ImageLibraryPropose::new(lib.clone());
        let incognito = ToolCtx {
            stage_locked: true,
            ..ctx(&ws, None)
        };
        // A `locked: false` the model sends is not a field the tool reads.
        let out = propose
            .call(
                json!({"kind": "character", "name": "noor", "text": "noor, short and wiry",
                       "portrait": "images/p.png", "locked": false}),
                &incognito,
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("library lock"), "{}", out.content);
        let out = propose
            .call(
                json!({"kind": "style", "name": "dusk", "text": "low warm light"}),
                &incognito,
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        let (loaded, _) = Library::load(&lib);
        for (kind, name) in [(Kind::Character, "noor"), (Kind::Style, "dusk")] {
            let e = loaded.get(kind, name).unwrap();
            assert!(e.locked, "{name} staged unlocked");
            assert_eq!(e.status, Status::Candidate);
            // An unstamped taint is untrusted.
            assert_eq!(e.origin, Origin::ModelUntrusted);
        }
        std::fs::remove_dir_all(lib).ok();
        std::fs::remove_dir_all(ws).ok();
    }

    #[tokio::test]
    async fn a_search_of_several_names_finds_each() {
        let lib = scratch();
        let ws = scratch();
        for name in ["maya", "john", "priya", "jo"] {
            imagelib::create(
                &lib,
                NewEntry {
                    kind: Kind::Character,
                    name: name.into(),
                    portrait: Some(png()),
                    // Descriptions that read like real ones: every one
                    // carries "and", which the phrase below also does.
                    text: format!("{name}, tall and lean, with dark hair"),
                    source_seed: None,
                    origin: Origin::Owner,
                    locked: false,
                },
            )
            .unwrap();
        }
        let out = ImageLibrary::new(lib.clone())
            .call(
                json!({"query": "a picture of Maya and John"}),
                &ctx(&ws, None),
            )
            .await
            .unwrap();
        assert!(
            out.content.contains("maya") && out.content.contains("john"),
            "{}",
            out.content
        );
        assert!(!out.content.contains("priya"), "{}", out.content);
        // A short name is found by name.
        let out = ImageLibrary::new(lib.clone())
            .call(json!({"query": "jo"}), &ctx(&ws, None))
            .await
            .unwrap();
        assert!(out.content.contains("character jo (v1)"), "{}", out.content);
        // A short word that is no one's name falls back to one substring —
        // the branch the name above never reaches: "ya" is in maya and priya.
        let out = ImageLibrary::new(lib.clone())
            .call(json!({"query": "ya"}), &ctx(&ws, None))
            .await
            .unwrap();
        assert!(
            out.content.contains("character maya (v1)")
                && out.content.contains("character priya (v1)")
                && !out.content.contains("character john"),
            "{}",
            out.content
        );
        // With no name in it, a query searches descriptions.
        let out = ImageLibrary::new(lib.clone())
            .call(json!({"query": "dark hair"}), &ctx(&ws, None))
            .await
            .unwrap();
        assert!(
            out.content.contains("priya") && out.content.contains("maya"),
            "{}",
            out.content
        );
        // A phrase with no name in it matches on every long word, not any —
        // "and" is in every description here, and matched them all.
        let out = ImageLibrary::new(lib.clone())
            .call(
                json!({"query": "a dog and a cat on a sofa"}),
                &ctx(&ws, None),
            )
            .await
            .unwrap();
        assert!(
            out.content
                .starts_with("Nothing in the image library matches."),
            "{}",
            out.content
        );
        std::fs::remove_dir_all(lib).ok();
        std::fs::remove_dir_all(ws).ok();
    }

    #[tokio::test]
    async fn a_phrase_finds_what_its_content_words_name() {
        let lib = scratch();
        let ws = scratch();
        imagelib::create(
            &lib,
            NewEntry {
                kind: Kind::Style,
                name: "picnic-park".into(),
                text: "a sunny park with checked blankets".into(),
                portrait: None,
                source_seed: None,
                origin: Origin::Owner,
                locked: false,
            },
        )
        .unwrap();
        let out = ImageLibrary::new(lib.clone())
            .call(json!({"query": "picnic in the park"}), &ctx(&ws, None))
            .await
            .unwrap();
        assert!(out.content.contains("style picnic-park"), "{}", out.content);
        std::fs::remove_dir_all(lib).ok();
        std::fs::remove_dir_all(ws).ok();
    }

    #[tokio::test]
    async fn a_candidates_name_does_not_empty_a_search() {
        let lib = scratch();
        let ws = scratch();
        imagelib::create(
            &lib,
            NewEntry {
                kind: Kind::Style,
                name: "picnic-park".into(),
                text: "a sunny park with checked blankets".into(),
                portrait: None,
                source_seed: None,
                origin: Origin::Owner,
                locked: false,
            },
        )
        .unwrap();
        // A candidate the model staged, named like the word searched for.
        imagelib::create(
            &lib,
            NewEntry {
                kind: Kind::Style,
                name: "picnic".into(),
                text: "proposed".into(),
                portrait: None,
                source_seed: None,
                origin: Origin::ModelClean,
                locked: false,
            },
        )
        .unwrap();
        let out = ImageLibrary::new(lib.clone())
            .call(json!({"query": "picnic"}), &ctx(&ws, None))
            .await
            .unwrap();
        assert!(out.content.contains("style picnic-park"), "{}", out.content);
        assert!(
            !out.content.contains("style picnic (v1)"),
            "{}",
            out.content
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

    fn library_with_wren() -> (PathBuf, PathBuf) {
        let lib = scratch();
        let ws = scratch();
        imagelib::create(
            &lib,
            NewEntry {
                kind: Kind::Character,
                name: "wren".into(),
                text: "wren, a fictional test character".into(),
                portrait: Some(png()),
                source_seed: None,
                origin: Origin::Owner,
                locked: false,
            },
        )
        .unwrap();
        std::fs::create_dir_all(ws.join("images")).unwrap();
        let tattoo = {
            let img = image::RgbImage::from_pixel(3, 3, image::Rgb([20, 60, 20]));
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::Png).unwrap();
            out.into_inner()
        };
        std::fs::write(ws.join("images/tattoo.png"), tattoo).unwrap();
        (lib, ws)
    }

    fn dirty() -> Option<Taint> {
        Some(Taint {
            untrusted: true,
            ..Taint::default()
        })
    }

    /// The owner's ruling of 2026-10-14: a clean conversation attaches a
    /// reference directly, recorded as the model's; one that has read
    /// outside content, has unknown taint, or is incognito stages it, and
    /// nothing reaches the character until the owner approves.
    #[tokio::test]
    async fn a_reference_lands_directly_only_from_a_clean_conversation() {
        let (lib, ws) = library_with_wren();
        let tool = ImageLibraryReference::new(lib.clone());
        let ask = json!({"name": "Wren", "as": "detail", "picture": "images/tattoo.png",
                         "label": "floral tattoo, left forearm"});
        let out = tool
            .call(ask.clone(), &ctx(&ws, Some(Taint::default())))
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("now v2"), "{}", out.content);
        let wren = Library::load(&lib)
            .0
            .get(Kind::Character, "wren")
            .unwrap()
            .clone();
        assert_eq!(wren.references.len(), 1);
        assert_eq!(wren.references[0].origin, Some(Origin::ModelClean));
        assert!(imagelib::pending_references(&lib).0.is_empty());

        // Removed again, directly, from the same clean conversation.
        let out = tool
            .call(
                json!({"name": "wren", "as": "detail", "label": "floral tattoo, left forearm", "remove": true}),
                &ctx(&ws, Some(Taint::default())),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(Library::load(&lib)
            .0
            .get(Kind::Character, "wren")
            .unwrap()
            .references
            .is_empty());

        // Tainted, unknown, incognito: staged, and the character untouched.
        let incognito = ToolCtx {
            stage_locked: true,
            ..ctx(&ws, Some(Taint::default()))
        };
        for (why, c) in [
            ("dirty", ctx(&ws, dirty())),
            ("unknown", ctx(&ws, None)),
            ("incognito", incognito),
        ] {
            let out = tool.call(ask.clone(), &c).await.unwrap();
            assert!(!out.is_error, "{why}: {}", out.content);
            assert!(out.content.contains("Staged"), "{why}: {}", out.content);
        }
        let wren = Library::load(&lib)
            .0
            .get(Kind::Character, "wren")
            .unwrap()
            .clone();
        assert!(
            wren.references.is_empty(),
            "a staged reference reached the character"
        );
        let (pending, _) = imagelib::pending_references(&lib);
        assert_eq!(pending.len(), 3);
        assert_eq!(
            pending.iter().filter(|p| p.locked).count(),
            1,
            "incognito stages locked"
        );
        assert!(pending
            .iter()
            .all(|p| p.origin == Origin::ModelUntrusted
                || (p.locked && p.origin == Origin::ModelClean)));

        // The owner approves one: applied, with the model's origin, and gone
        // from the queue; rejecting the rest leaves nothing behind.
        let first = pending[0].id.clone();
        let e = imagelib::approve_reference(&lib, &first).unwrap();
        assert_eq!(e.references.len(), 1);
        assert_eq!(e.references[0].origin, Some(pending[0].origin));
        for p in &pending[1..] {
            // The same picture twice is the store's to refuse at approval.
            imagelib::reject_reference(&lib, &p.id).unwrap();
        }
        assert!(imagelib::pending_references(&lib).0.is_empty());
        std::fs::remove_dir_all(lib).ok();
        std::fs::remove_dir_all(ws).ok();
    }

    /// Planning changes nothing: not attached, not staged, even from a clean
    /// conversation (review of #696).
    #[tokio::test]
    async fn planning_never_writes_a_reference() {
        let (lib, ws) = library_with_wren();
        let planning = ToolCtx {
            phase: crate::agent::Phase::Plan,
            ..ctx(&ws, Some(Taint::default()))
        };
        let out = ImageLibraryReference::new(lib.clone())
            .call(
                json!({"name": "wren", "as": "full_body", "picture": "images/tattoo.png"}),
                &planning,
            )
            .await
            .unwrap();
        assert!(out.is_error, "{}", out.content);
        assert!(out.content.contains("planning"), "{}", out.content);
        let wren = Library::load(&lib)
            .0
            .get(Kind::Character, "wren")
            .unwrap()
            .clone();
        assert!(wren.references.is_empty());
        assert!(imagelib::pending_references(&lib).0.is_empty());
        std::fs::remove_dir_all(lib).ok();
        std::fs::remove_dir_all(ws).ok();
    }

    /// A name the library does not hold is answered as `image_library`
    /// answers it — only what the call named — and nothing is staged.
    #[tokio::test]
    async fn an_unknown_character_is_answered_without_the_roster() {
        let (lib, ws) = library_with_wren();
        let out = ImageLibraryReference::new(lib.clone())
            .call(
                json!({"name": "nobody", "as": "full_body", "picture": "images/tattoo.png"}),
                &ctx(&ws, Some(Taint::default())),
            )
            .await
            .unwrap();
        assert!(out.is_error, "{}", out.content);
        assert!(!out.content.contains("wren"), "{}", out.content);
        assert!(imagelib::pending_references(&lib).0.is_empty());
        // A detail without a label is refused before anything is written.
        let out = ImageLibraryReference::new(lib.clone())
            .call(
                json!({"name": "wren", "as": "detail", "picture": "images/tattoo.png"}),
                &ctx(&ws, dirty()),
            )
            .await
            .unwrap();
        assert!(out.is_error, "{}", out.content);
        assert!(imagelib::pending_references(&lib).0.is_empty());
        std::fs::remove_dir_all(lib).ok();
        std::fs::remove_dir_all(ws).ok();
    }

    /// Proposing a character that is already in the library points at the
    /// reference tool, so a model asked to save a picture "to" someone has
    /// a next step rather than the same refusal again (2026-10-14).
    #[tokio::test]
    async fn proposing_an_existing_character_points_at_references() {
        let (lib, ws) = library_with_wren();
        let out = ImageLibraryPropose::new(lib.clone())
            .call(
                json!({"kind": "character", "name": "wren", "text": "x", "portrait": "images/tattoo.png"}),
                &ctx(&ws, Some(Taint::default())),
            )
            .await
            .unwrap();
        assert!(out.is_error, "{}", out.content);
        assert!(
            out.content.contains("image_library_reference"),
            "{}",
            out.content
        );
        std::fs::remove_dir_all(lib).ok();
        std::fs::remove_dir_all(ws).ok();
    }

    #[test]
    fn the_lookup_is_private_and_never_untrusted() {
        // Private on purpose: the library may hold a real person's
        // description and nothing marks which (see the module doc).
        let lookup = ImageLibrary::new(PathBuf::new()).capabilities();
        assert!(lookup.private_data);
        assert!(!lookup.untrusted_input && !lookup.destructive);
        assert_eq!(lookup.egress, crate::tool::Egress::None);
        // A proposal returns a sentence about itself, nothing from the store.
        let propose = ImageLibraryPropose::new(PathBuf::new()).capabilities();
        assert!(!propose.private_data && !propose.untrusted_input && !propose.destructive);
        // The reference writer changes an approved entry: declared, so a
        // profile can tell it from a public read.
        let reference = ImageLibraryReference::new(PathBuf::new()).capabilities();
        assert!(reference.destructive && !reference.private_data);
        assert_eq!(propose.egress, crate::tool::Egress::None);
    }
}
