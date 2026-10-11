//! A picture edit sent from the edit panel: what the harness tells the
//! persona when the owner's message is an instruction to the image model
//! rather than something said to it.
//!
//! The panel composes `Edit <picture>: <change>` (`web/src/lib/image-edit.js`)
//! and the page marks the turn as one (`SendBody::edit`), so the turn is known
//! by the door it came through, never by parsing the words. Without a note,
//! the persona answered an edit by describing a picture it has not seen and
//! then reusing the rest of its previous reply: across every persona chat, 11
//! of 34 edit replies were at least half copied from the five before
//! (2026-10-05), against 9 of 201 turns that drew nothing. Replayed on 22 edit
//! turns from two chats, three samples each, the recorded tool call kept and
//! only the reply after it re-sampled:
//!
//! | | at least half copied | longest copy | median length |
//! |---|---|---|---|
//! | no note | 37/64 | 206 words | 744 chars |
//! | this note | 16/66 | 126 words | 209 chars |
//!
//! Judged blind by the same model with the order swapped, 66–59 for the note
//! (a tie within noise); the edit prompts it wrote were unchanged (44/44
//! called with the picture, 38 vs 36 in the "Keep …" form).
//!
//! Typed turns only: on a call the panel's words go out as speech, and the
//! call note already says a picture reaches the owner unseen
//! (`persona::call`).
//!
//! **Since `IMAGE-DESIGN.md` §5.3 the persona no longer makes the panel's
//! call.** The owner's words and the picture's record go through a one-shot
//! extraction ([`extraction_request`], a [`QuarantinedPass`]); the harness
//! draws the typed change through `Agent::dispatch_one`; the history keeps
//! one fact ([`fact`]); and the persona replies in a line under
//! [`DONE`], with `tool_choice: "none"`. The note above is retired: a turn
//! the panel sends carries none, and [`is_note`] stays only to recognise it
//! in older transcripts.
//!
//! [`QuarantinedPass`]: crate::quarantine::QuarantinedPass

/// What the retired note opened with. Kept to recognise it in older
/// transcripts' recorded notes, where it is the harness's voice.
pub const EDIT_STEM: &str = "(From the harness: this message came from the picture edit panel";

/// Whether `text` is the retired note.
pub fn is_note(text: &str) -> bool {
    text.trim_start().starts_with(EDIT_STEM)
}

/// What the fact a panel edit leaves in the owner's turn opens with;
/// registered as the harness's voice.
pub const FACT_STEM: &str = "(From the harness: the owner changed a picture from the edit panel";

/// The fact a panel edit leaves in the history (§5.3 step 3): which picture,
/// the typed change, and the tool's own result, whose first line names the
/// new picture (`image: images/…png`) or says why nothing was drawn. Folded
/// into the owner's turn, so the transcript stays valid (no `tool_use` the
/// model did not make) and the page draws the card from it.
pub fn fact(picture: &str, change: &str, result: &str) -> String {
    // One line before the result, whatever the owner typed: the card finds
    // the result after the first newline (review of #598).
    let one_line = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ");
    format!(
        "{FACT_STEM}, and the harness drew it, not you. The picture: {}. The change: {}.)\n\
         {result}",
        one_line(picture),
        one_line(change)
    )
}

/// Whether `text` is a panel edit's fact.
pub fn is_fact(text: &str) -> bool {
    text.trim_start().starts_with(FACT_STEM)
}

/// The tool's result a fact carries: everything after its first line.
pub fn fact_result(text: &str) -> Option<&str> {
    is_fact(text)
        .then(|| text.split_once('\n').map(|(_, rest)| rest))
        .flatten()
}

/// The change a Regenerate draws: the picture's scene as it is, at a new
/// seed, with no model turn (IMAGE-DESIGN.md §5.4).
pub const REDRAWN: &str = "drawn again as it is, at a new seed";

/// Regenerate's call: the picture alone. The planner reads a call that
/// changes nothing as the scene drawn again at a new seed (`redrawn`).
pub fn redraw_call(picture: &str) -> serde_json::Value {
    serde_json::json!({ "picture": picture })
}

/// The line a Regenerate says in a call, which the owner hears the call
/// take as typed: the page says it, and the server matches it to the
/// Regenerate the page registered (`persona_chat::call_regenerate`). The
/// words bind the turn and grant nothing: a spoken turn that says them with
/// no registration is words. The page's own `composeRegenerateMessage`
/// writes the same line for a typed Regenerate.
pub fn regenerate_line(picture: &str) -> String {
    format!("Regenerate {picture}")
}

/// The picture a Regenerate's fact drew again, so the page shows the new one
/// as a version on that picture's card (§5.4). `None` for any other fact.
pub fn fact_redraw_of(text: &str) -> Option<&str> {
    let first = text.lines().next().filter(|l| is_fact(l))?;
    let (picture, change) = first
        .split_once("The picture: ")?
        .1
        .split_once(". The change: ")?;
    (change.trim_end_matches(".)") == REDRAWN).then_some(picture)
}

/// The closing line of the persona's reply after a panel edit: the run's
/// first request is already a closing one (`RunContext::close_with`).
pub const DONE: &str = "(From the harness: the picture change the owner asked for from the \
edit panel is drawn and on their screen; you have not seen it. No tool is needed now; answer the \
owner in a sentence or two, in your own voice.)";

/// What the owner sees in place of a reply to [`DONE`] that was empty or
/// only a tool call written as text.
pub const DONE_REPLY: &str = "Done.";

/// The closing line when the owner's change was not drawn: the fact above
/// it says why, and the persona says so in its own words (§5.3 steps 6–7).
pub const NOT_DRAWN: &str = "(From the harness: the picture change the owner asked for from the \
edit panel was not drawn; the reason is in their turn above. No tool is needed now; tell the \
owner in a sentence, in your own voice.)";

/// What the owner sees in place of a reply to [`NOT_DRAWN`] that was empty
/// or only a tool call written as text.
pub const NOT_DRAWN_REPLY: &str = "That change didn't go through.";

/// Why an extraction was not understood, as the fact's result says it.
pub fn not_understood(why: &str) -> String {
    format!("Nothing was drawn. The change was not understood: {why}.")
}

/// How the record names a setting that is the owner's photo: a stand-in
/// for the reader, never words to draw.
pub const PHOTO_SETTING: &str = "the owner's photo";

/// The photo stand-in coming back as `setting`: said alone it is the record
/// restated, so it is left out and the photo stays; with more words ("the
/// owner's photo, in watercolour") they would replace the owner's photo in
/// the record and the picture would no longer be placed in it, so that is
/// refused with the way on (review of #605). A real change of place over the
/// photo still goes through, as the owner asked. Inside the reader, so every
/// caller has it.
fn photo_setting(
    scene: &mut serde_json::Map<String, serde_json::Value>,
    record: Option<&crate::scene::Scene>,
) -> Result<bool, String> {
    let on_photo = record
        .and_then(|r| r.setting.as_ref())
        .is_some_and(|f| matches!(f.value, crate::scene::Setting::Photo { .. }));
    if !on_photo {
        return Ok(false);
    }
    let Some(said) = scene.get("setting").and_then(serde_json::Value::as_str) else {
        return Ok(false);
    };
    let said = said
        .trim()
        .trim_end_matches('.')
        .to_lowercase()
        .replace(['\u{2018}', '\u{2019}'], "'");
    if said == PHOTO_SETTING {
        scene.remove("setting");
        return Ok(true);
    } else if said.contains(PHOTO_SETTING) {
        scene.remove("setting");
        return Err(
            "this picture is placed on the owner's photo, and words in its setting \
                    would replace the photo: say the change without restating the photo, or \
                    name one of the library's styles for a new look"
                .into(),
        );
    }
    Ok(false)
}

/// The extraction's instructions: a3's measured wording (EVIDENCE §B.8,
/// §D), with the clothes home the fast misses lacked and the `kind`
/// discriminator gone.
const EXTRACTION_SYSTEM: &str = "You turn the owner's request to change a picture into the \
fields that change. Given the picture's scene record and the owner's words, fill only what \
changes: people (by `who` from the record) with `doing` for a new pose or action, `expression` \
for the face, `wearing` for clothes (clothes alone: only `wearing`), `where` for their place \
in the frame; `remove` to take \
someone out; someone new with their `who`, `wearing` and `doing`; `together` for what the people \
in the picture do with each other, as one line naming them; `camera`, `light`, `setting` when \
those change; `look` when the owner asks for a different look (a style, a medium, an era), \
copied short in the owner's own words; `retouch` for a small change to something that is not a \
person. \
Leave out \
everything that stays the same. If the words only ask for another try, answer {}. Answer with \
JSON only.";

/// The shape the extraction answers in (a3's, measured): flat, with no
/// `kind`, so clothes have an obvious home in `people[].wearing`.
pub fn extraction_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "people": {"type": "array", "items": {"type": "object", "properties": {
                "who": {"type": "string"},
                "wearing": {"type": "string"},
                "doing": {"type": "string"},
                "expression": {"type": "string"},
                "where": {"type": "string", "enum": ["left", "centre", "right", "background"]},
                "remove": {"type": "boolean"}
            }, "required": ["who"]}},
            "together": {"type": "string"},
            "camera": {"type": "string"},
            "light": {"type": "string"},
            "setting": {"type": "string"},
            "look": {"type": "string"},
            "retouch": {"type": "string"}
        }
    })
}

/// The picture's record as the extraction reads it: what it was drawn as,
/// with the people by the names the owner would use. No origins: the pass
/// has no tools and no history, so an untrusted field can only shape a
/// typed change the tool checks again.
pub fn record_for(scene: &crate::scene::Scene) -> serde_json::Value {
    use crate::scene::Setting;
    let mut out = serde_json::Map::new();
    match scene.setting.as_ref().map(|f| &f.value) {
        Some(Setting::Words { text }) => {
            out.insert("setting".into(), text.clone().into());
        }
        Some(Setting::Photo { .. }) => {
            out.insert("setting".into(), PHOTO_SETTING.into());
        }
        _ => {}
    }
    for (k, f) in [
        ("light", &scene.light),
        ("camera", &scene.camera),
        ("together", &scene.together),
        ("style", &scene.style),
    ] {
        if let Some(f) = f {
            out.insert(k.into(), f.value.clone().into());
        }
    }
    let people: Vec<serde_json::Value> = scene
        .people
        .iter()
        .map(|p| {
            let mut m = serde_json::Map::new();
            m.insert("who".into(), crate::picture::shown(&p.who).into());
            for (k, v) in [
                ("wearing", &p.wearing),
                ("doing", &p.doing),
                ("expression", &p.expression),
            ] {
                if !v.trim().is_empty() {
                    m.insert(k.into(), v.clone().into());
                }
            }
            if let Some(at) = p.at {
                m.insert("where".into(), at.name().into());
            }
            serde_json::Value::Object(m)
        })
        .collect();
    if !people.is_empty() {
        out.insert("people".into(), people.into());
    }
    serde_json::Value::Object(out)
}

/// The one-shot request (§5.3 step 1): no tools, one message, no
/// thinking (fast: ~2 s against ~80 s, a3's §D), the schema where the
/// provider honours one. `persona` names who "her" or "you" most likely
/// means in a persona chat.
pub fn extraction_request(
    model: &str,
    record: Option<&crate::scene::Scene>,
    words: &str,
    persona: Option<&str>,
    styles: &[String],
    structured: bool,
) -> crate::message::CompletionRequest {
    reading_request(model, record, words, None, persona, styles, structured)
}

/// The sentence that ranks the owner's words over the persona's reply when
/// the reader is given both (mecha-a3's (A), 2026-10-08: on explicit asks the
/// owner's content landed 10 of 10 and hers 0; on open ones, "show me what
/// you want", hers 10 of 10).
pub const REPLY_DECIDES: &str = "The owner's words decide; the persona's reply, when given, fills \
in only what the owner left to it.";

/// The reader's request with the persona's latest reply beside the owner's
/// words, as mecha-a3 measured it: appended to the words, and the deciding
/// sentence before the last one of the instructions. Without a reply it is
/// the edit panel's request exactly.
pub fn reading_request(
    model: &str,
    record: Option<&crate::scene::Scene>,
    words: &str,
    reply: Option<&str>,
    persona: Option<&str>,
    styles: &[String],
    structured: bool,
) -> crate::message::CompletionRequest {
    let (words, system) = match reply.map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => (
            format!("{words}\n\n(What the persona said in its last reply: {r})"),
            EXTRACTION_SYSTEM.replace(
                "Answer with JSON only.",
                &format!("{REPLY_DECIDES} Answer with JSON only."),
            ),
        ),
        None => (words.to_string(), EXTRACTION_SYSTEM.to_string()),
    };
    let mut user = serde_json::Map::new();
    user.insert(
        "scene".into(),
        record.map_or(serde_json::json!({}), record_for),
    );
    user.insert("owner".into(), words.into());
    if let Some(p) = persona {
        user.insert("persona".into(), p.into());
    }
    // The styles a change may name (not private, the owner's ruling of
    // 2026-10-08): "change the style to hyperrealistic" had no field and
    // became a retouch that drew the same picture again.
    if !styles.is_empty() {
        user.insert("styles".into(), styles.into());
    }
    crate::quarantine::QuarantinedPass::new(model, crate::provider::LOCAL_MAX_TOKENS)
        .system(system)
        .no_thinking()
        .response_schema(structured.then(extraction_schema))
        .ask(serde_json::Value::Object(user).to_string())
}

/// Who reads a persona turn's picture ask for what the persona's own call
/// left out: a persona copies its earlier calls and drops the owner's ask
/// from them (3 of 25 carried it), while this reader over the owner's words
/// and the persona's latest reply carried it 25 of 25 (mecha-a3, 2026-10-08).
/// Handed to `image_generate` per turn (`ToolCtx::scene_reader`).
#[async_trait::async_trait]
pub trait SceneReader: Send + Sync + std::fmt::Debug {
    /// The ask as a typed change against `record` (the picture before the
    /// turn), or why it could not be read.
    async fn read(&self, record: Option<&crate::scene::Scene>) -> Result<Extracted, String>;
}

/// The reader on the persona's own model, holding this turn's words.
pub struct ModelReader {
    pub provider: Box<dyn crate::provider::Provider>,
    pub model: String,
    /// The owner's words this turn.
    pub owner: String,
    /// The persona's latest reply before this turn, in words.
    pub reply: Option<String>,
    /// The persona's display name and its library character, which "her"
    /// and "you" most likely mean, and which the reader may name.
    pub persona: String,
    pub character: Option<String>,
    pub library: std::path::PathBuf,
}

impl std::fmt::Debug for ModelReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelReader")
            .field("model", &self.model)
            .finish()
    }
}

#[async_trait::async_trait]
impl SceneReader for ModelReader {
    async fn read(&self, record: Option<&crate::scene::Scene>) -> Result<Extracted, String> {
        let lib = crate::imagelib::Library::load(&self.library).0;
        let styles = crate::imagelib::style_names(&lib);
        let shown_persona = self
            .character
            .as_deref()
            .map(|c| crate::picture::shown(&crate::scene::Who::Library(c.into())));
        let request = reading_request(
            &self.model,
            record,
            &self.owner,
            self.reply.as_deref(),
            shown_persona.as_deref(),
            &styles,
            self.provider.structured_output(),
        );
        let response = self
            .provider
            .complete(&request, None)
            .await
            .map_err(|e| format!("the reader could not be reached ({e:#})"))?;
        // The envelope before the content: a refusal arrives as success.
        if response.stop_reason == crate::message::StopReason::Refusal {
            return Err("the reader refused".into());
        }
        let text = response.message.text();
        if text.trim().is_empty() {
            return Err("the reader's answer was empty".into());
        }
        let known = |who: &str| {
            let key = who.trim().to_lowercase();
            lib.get(crate::imagelib::Kind::Character, &key)
                .is_some_and(|e| e.status == crate::imagelib::Status::Approved)
                || self.character.as_deref() == Some(key.as_str())
                || self.persona.to_lowercase() == key
                || record.is_some_and(|r| {
                    r.people
                        .iter()
                        .any(|p| crate::picture::shown(&p.who).to_lowercase() == key)
                })
        };
        read_extraction_for(
            &text,
            &known,
            None,
            &Looks {
                styles: &styles,
                record,
                library: Some(&lib),
            },
        )
    }
}

/// The extraction's answer, read: the JSON object in it, with only the
/// fields the schema has, as the tool's `scene` and `retouch`. A `retouch`
/// beside a scene change is left out and said, since the tool draws one or
/// the other. `known` answers whether a `who` names someone this picture or
/// the library holds; a bare name it does not is a failure (§5.3 step 6),
/// never a stranger drawn under that name.
pub fn read_extraction(text: &str, known: &dyn Fn(&str) -> bool) -> Result<Extracted, String> {
    read_extraction_for(text, known, None, &Looks::default())
}

/// What a `look` is matched against: the library's style names, and the
/// picture's record, whose setting an unmatched look joins.
#[derive(Default)]
pub struct Looks<'a> {
    /// The styles offered to the reader: approved and unlocked.
    pub styles: &'a [String],
    pub record: Option<&'a crate::scene::Scene>,
    /// The library itself, when there is one: what a look is matched
    /// against, as `image_generate` checks a style (any approved one, locked
    /// ones too, since generation draws a locked style when it is named; the
    /// lock only hides it from being offered). A look naming a style that
    /// waits on the owner or did not load is that finding, said by name,
    /// never words in the setting (review of #605).
    pub library: Option<&'a crate::imagelib::Library>,
}

/// A held style replacing an earlier look in words: those words come off
/// the setting, the change's own or the record's, so the old look does not
/// ride beside the new style (review of #605).
fn drop_earlier_look(
    looks: &Looks,
    scene: &mut serde_json::Map<String, serde_json::Value>,
) -> bool {
    let words = match scene.get("setting").and_then(serde_json::Value::as_str) {
        Some(given) => Some(given.to_string()),
        None => match looks
            .record
            .and_then(|r| r.setting.as_ref())
            .map(|f| &f.value)
        {
            Some(crate::scene::Setting::Words { text }) => Some(text.clone()),
            _ => None,
        },
    };
    match words.as_deref().and_then(|w| w.split_once(LOOK_JOIN)) {
        Some((before, _)) => {
            scene.insert("setting".into(), before.trim().into());
            true
        }
        None => false,
    }
}

/// The words `look_into` joins a look to a setting with; a later look
/// replaces the earlier one rather than stacking beside it.
const LOOK_JOIN: &str = ", in the look of ";

/// The owner's look, in their words, made a change by code (mecha-a3's G605:
/// the reader copies a look faithfully, 27 of 27, but given `style` it chose
/// the nearest name or invented one, 0 of 18 unheld looks right). A look the
/// library has a style for is that `style`, spelled as names are and with a
/// trailing "style" or "look" dropped; any other look joins the setting in
/// words. Over the owner's photo it cannot: words would replace the photo,
/// so that is refused with the way on.
fn look_into(
    look: &str,
    looks: &Looks,
    scene: &mut serde_json::Map<String, serde_json::Value>,
) -> Result<bool, String> {
    let words = look.trim();
    let lower = words.to_lowercase();
    // The whole spelling first, so a style named `…-look` is found; then
    // without a trailing "style" or "look".
    let bare = ["style", "look"]
        .iter()
        .find_map(|tail| lower.strip_suffix(tail))
        .map_or(lower.as_str(), str::trim);
    if bare.is_empty() {
        // "style" alone names no look.
        return Ok(false);
    }
    let keys = [
        crate::imagelib::spelled_as_name(&lower),
        crate::imagelib::spelled_as_name(bare),
    ];
    // The picture's own style said back is no change (the record shows it,
    // and a reader restates it): never words in the setting.
    let current = looks
        .record
        .and_then(|r| r.style.as_ref())
        .map(|f| crate::imagelib::spelled_as_name(&f.value));
    if keys.iter().any(|k| Some(k) == current.as_ref()) {
        // Unless an earlier look in words rides beside it: naming the style
        // again takes those words off, and that is a change (review of
        // #605: "put it back to ink wash" otherwise kept the sketch).
        return Ok(drop_earlier_look(looks, scene));
    }
    for key in &keys {
        let approved = looks.library.is_some_and(|lib| {
            lib.get(crate::imagelib::Kind::Style, key)
                .is_some_and(|e| e.status == crate::imagelib::Status::Approved)
        });
        if looks.styles.contains(key) || approved {
            scene.insert("style".into(), key.clone().into());
            drop_earlier_look(looks, scene);
            return Ok(true);
        }
    }
    if let Some(lib) = looks.library {
        let kind = crate::imagelib::Kind::Style;
        if let Some(key) = keys.iter().find(|k| !crate::imagelib::absent(lib, kind, k)) {
            return Err(crate::imagelib::missing(lib, kind, key));
        }
    }
    // Over a picture drawn in a library style, that style's words still ride
    // the restage beside these: a style cannot be cleared yet. Refusing would
    // block nearly every look on this owner's pictures (24 of 24 today carry
    // a style, mecha-a3), so the words go in and the gap is a known
    // follow-up.
    use crate::scene::Setting;
    let base = match scene.get("setting").and_then(serde_json::Value::as_str) {
        Some(given) => Some(given.to_string()),
        None => match looks
            .record
            .and_then(|r| r.setting.as_ref())
            .map(|f| &f.value)
        {
            Some(Setting::Photo { .. }) => {
                return Err(
                    "this picture is placed on the owner's photo, and a look the \
                            library has no style for would replace the photo: name one of the \
                            library's styles instead"
                        .into(),
                )
            }
            Some(Setting::Words { text }) => Some(text.clone()),
            _ => None,
        },
    };
    // A look asked for before is replaced, not stacked beside this one.
    let base = base.map(|b| match b.find(LOOK_JOIN) {
        Some(at) => b[..at].to_string(),
        None => b,
    });
    // No setting to join: "in the look of X" alone names no place, and a
    // restage would draw the picture from those words. Refused as the photo
    // is, for the same reason (review of #605).
    let setting = match base.as_deref().map(|b| b.trim().trim_end_matches('.')) {
        Some(b) if !b.is_empty() => format!("{b}{LOOK_JOIN}{words}"),
        _ => {
            return Err(
                "this picture has no setting a look can join, and words alone would \
                        replace what it shows: name one of the library's styles instead"
                    .into(),
            )
        }
    };
    scene.insert("setting".into(), setting.into());
    Ok(true)
}

/// [`read_extraction`], knowing the record's one person when it holds
/// exactly one: a bare `{"remove": true}` then takes them out, since nobody
/// else could be meant (mecha-a3's G1: 4 removal wordings came back so).
pub fn read_extraction_for(
    text: &str,
    known: &dyn Fn(&str) -> bool,
    sole: Option<&str>,
    looks: &Looks,
) -> Result<Extracted, String> {
    let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) else {
        return Err("the answer held no JSON object".into());
    };
    if end < start {
        return Err("the answer held no JSON object".into());
    }
    let v: serde_json::Value = serde_json::from_str(&text[start..=end])
        .map_err(|e| format!("the answer did not read as JSON: {e}"))?;
    let obj = v.as_object().ok_or("the answer was not an object")?;
    let mut scene = serde_json::Map::new();
    for k in ["together", "camera", "light", "setting"] {
        match obj.get(k) {
            None | Some(serde_json::Value::Null) => {}
            Some(serde_json::Value::String(t)) if t.trim().is_empty() => {}
            Some(serde_json::Value::String(t)) => {
                scene.insert(k.into(), t.trim().into());
            }
            Some(_) => return Err(format!("`{k}` was not text")),
        }
    }
    // One person answered at the top level, as a model without the schema
    // does: that person, as `people`'s one entry (mecha-a3's G1: with no
    // record and no schema, 20 pose and camera asks came back this way and
    // were read as nothing, then drawn again).
    let bare_removal = obj.get("who").is_none()
        && obj.get("people").is_none_or(serde_json::Value::is_null)
        && obj.get("remove").and_then(serde_json::Value::as_bool) == Some(true);
    let top_level = match (bare_removal, sole) {
        (true, Some(who)) => Some(serde_json::json!([{ "who": who, "remove": true }])),
        _ => obj
            .get("who")
            .filter(|w| w.as_str().is_some_and(|w| !w.trim().is_empty()))
            .map(|_| serde_json::Value::Array(vec![v.clone()])),
    };
    let people = obj
        .get("people")
        .filter(|p| !p.is_null())
        .or(top_level.as_ref());
    if let Some(people) = people {
        let list = people.as_array().ok_or("`people` was not a list")?;
        let mut out = Vec::new();
        for p in list {
            let who = p["who"]
                .as_str()
                .map(str::trim)
                .filter(|w| !w.is_empty())
                .ok_or("a person had no `who`")?;
            if looks_like_a_name(who) && !known(who) {
                return Err(format!("{who} is not in this picture or the image library"));
            }
            let mut m = serde_json::Map::new();
            m.insert("who".into(), who.into());
            for k in ["wearing", "doing", "expression", "where"] {
                if let Some(t) = p[k].as_str().map(str::trim).filter(|t| !t.is_empty()) {
                    m.insert(k.into(), t.into());
                }
            }
            if p["remove"].as_bool() == Some(true) {
                m.insert("remove".into(), true.into());
            }
            out.push(serde_json::Value::Object(m));
        }
        if !out.is_empty() {
            scene.insert("people".into(), out.into());
        }
    }
    // The stand-in said back alone was understood: the photo stays, and it
    // counts as nothing asked, like a no-op look (review of #605).
    // A look or a setting that cannot be drawn is left out beside the rest
    // of the change and said, refused only when nothing else was asked, as
    // `image_generate` rules (review of #605: "a smile, and watercolour"
    // over a photo drew nothing at all).
    let mut cannot: Vec<String> = Vec::new();
    let restated_photo = match photo_setting(&mut scene, looks.record) {
        Ok(dropped) => dropped,
        Err(why) => {
            cannot.push(why);
            false
        }
    };
    let mut no_op_look = false;
    // `style` too: the record shows that key, and a reader without the
    // schema answers in it (review of #605).
    match obj
        .get("look")
        .filter(|v| !v.is_null() && v.as_str().is_none_or(|t| !t.trim().is_empty()))
        .or_else(|| obj.get("style"))
    {
        None | Some(serde_json::Value::Null) => {}
        Some(serde_json::Value::String(t)) if t.trim().is_empty() => {}
        Some(serde_json::Value::String(t)) => match look_into(t, looks, &mut scene) {
            Ok(true) => {}
            // A look that changes nothing (the picture's own style, or
            // "style" alone) was said, and understood: another try.
            Ok(false) => no_op_look = true,
            Err(why) => cannot.push(why),
        },
        Some(_) => return Err("`look` was not text".into()),
    }
    let retouch = match obj.get("retouch") {
        Some(serde_json::Value::String(t)) if !t.trim().is_empty() => Some(t.trim().to_string()),
        None | Some(serde_json::Value::Null) | Some(serde_json::Value::String(_)) => None,
        Some(_) => return Err("`retouch` was not text".into()),
    };
    // Only an answer with nothing in it is "another try" (§5.3 step 5). One
    // whose keys this reader could not use is a failure, said, never a redraw
    // (mecha-a3's G1).
    // Keys present but empty or null are nothing asked, as a schema-held
    // model writes "nothing changed" (review of #598): another try.
    // Nothing else to draw: what could not be drawn is the answer.
    if scene.is_empty() && retouch.is_none() && !cannot.is_empty() {
        return Err(cannot.join("; "));
    }
    let said_something = obj
        .iter()
        .filter(|(k, _)| !(no_op_look && (*k == "look" || *k == "style")))
        .filter(|(k, _)| !(restated_photo && *k == "setting"))
        .map(|(_, v)| v)
        .any(|v| match v {
            serde_json::Value::Null => false,
            serde_json::Value::String(t) => !t.trim().is_empty(),
            serde_json::Value::Array(a) => !a.is_empty(),
            serde_json::Value::Object(o) => !o.is_empty(),
            _ => true,
        });
    if scene.is_empty() && retouch.is_none() && said_something {
        let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        return Err(format!(
            "the answer held nothing this change can use (it gave {})",
            keys.join(", ")
        ));
    }
    let (retouch, left_out) = if scene.is_empty() {
        (retouch, None)
    } else {
        (None, retouch)
    };
    let left_out = match (left_out, cannot.is_empty()) {
        (l, true) => l,
        (None, false) => Some(cannot.join("; ")),
        (Some(l), false) => Some(format!("{l}; {}", cannot.join("; "))),
    };
    Ok(Extracted {
        scene,
        retouch,
        left_out,
    })
}

/// One or two capitalised words: a name, as opposed to a description ("a
/// waiter with a tray").
fn looks_like_a_name(who: &str) -> bool {
    let words: Vec<&str> = who.split_whitespace().collect();
    (1..=2).contains(&words.len())
        && words.iter().all(|w| {
            w.chars().next().is_some_and(char::is_uppercase)
                && w.chars()
                    .all(|c| c.is_alphabetic() || c == '-' || c == '\'')
        })
}

/// What an extraction asked for, as the tool takes it.
#[derive(Debug, Clone, PartialEq)]
pub struct Extracted {
    pub scene: serde_json::Map<String, serde_json::Value>,
    pub retouch: Option<String>,
    /// A retouch the extraction gave beside a scene change, not drawn.
    pub left_out: Option<String>,
}

impl Extracted {
    /// The `image_generate` call for `picture`: the scene change, or the
    /// retouch with the owner's painted mask. Nothing at all is a redraw
    /// (§5.3 step 5): `picture` alone changes nothing.
    pub fn call(&self, picture: &str) -> serde_json::Value {
        let mut call = serde_json::json!({ "picture": picture });
        if !self.scene.is_empty() {
            call["scene"] = serde_json::Value::Object(self.scene.clone());
        } else if let Some(r) = &self.retouch {
            call["retouch"] = r.clone().into();
        }
        call
    }

    /// The change in words for the fact: the typed fields, or that it was
    /// drawn again.
    pub fn summary(&self) -> String {
        let mut said = if !self.scene.is_empty() {
            serde_json::Value::Object(self.scene.clone()).to_string()
        } else if let Some(r) = &self.retouch {
            format!("retouch: {r}")
        } else {
            "drawn again".to_string()
        };
        if let Some(l) = &self.left_out {
            said.push_str(&format!("; not drawn, beside the scene change: {l}"));
        }
        said
    }
}

/// One painted region from the edit panel: its colour and its words.
#[derive(serde::Deserialize, Clone, Debug, PartialEq)]
pub struct PanelRegion {
    pub colour: String,
    pub words: String,
}

/// The edit panel's regions call (IMAGE-REGION-EDIT-RESEARCH.md §7, M2):
/// the picture, the region index painted over it, and each region's words.
/// The tool builds the legend; the panel sends values only.
pub fn regions_call(picture: &str, index: &str, regions: &[PanelRegion]) -> serde_json::Value {
    serde_json::json!({
        "picture": picture,
        "mask": index,
        "regions": regions
            .iter()
            .map(|r| serde_json::json!({"colour": r.colour, "words": r.words}))
            .collect::<Vec<_>>(),
    })
}

/// What a regions edit's fact says changed: each colour and its words.
pub fn regions_change(regions: &[PanelRegion]) -> String {
    let each: Vec<String> = regions
        .iter()
        .map(|r| format!("{}: {}", r.colour, r.words.trim()))
        .collect();
    format!("in the painted regions, {}", each.join("; "))
}

/// A painted area is a retouch by definition: the owner's words go to the
/// tool as written, with the mask, and no extraction runs.
pub fn masked_call(picture: &str, mask: &str, words: &str) -> serde_json::Value {
    serde_json::json!({ "picture": picture, "retouch": words, "mask": mask })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Regenerate's fact names the picture it drew again; an edit's, none.
    #[test]
    fn a_regenerate_fact_names_the_picture_it_drew_again() {
        let redraw = fact("images/a.png", REDRAWN, "image: images/b.png\nA picture.");
        assert!(is_fact(&redraw));
        assert_eq!(fact_redraw_of(&redraw), Some("images/a.png"));
        assert_eq!(
            redraw_call("images/a.png"),
            serde_json::json!({"picture": "images/a.png"})
        );
        let changed = fact("images/a.png", "make the sky pink", "image: images/c.png");
        assert_eq!(fact_redraw_of(&changed), None);
        assert_eq!(fact_redraw_of("plain words"), None);
    }

    #[test]
    fn the_fact_is_the_harness_speaking_never_the_owner() {
        let fact = fact(
            "images/a.png",
            "drawn again",
            "image: images/b.png\nA new picture.",
        );
        assert!(is_fact(&fact));
        assert!(crate::agent::is_harness_voice(&fact));
        assert_eq!(
            fact_result(&fact),
            Some("image: images/b.png\nA new picture.")
        );
        let mut m = crate::message::Message::user("Edit images/a.png: make the sky pink");
        m.content.push(crate::message::Block::Text { text: fact });
        assert_eq!(
            crate::agent::owner_text(&m),
            "Edit images/a.png: make the sky pink"
        );
        assert!(crate::agent::is_harness_voice(DONE));
        assert!(crate::agent::is_harness_voice(NOT_DRAWN));
    }

    #[test]
    fn an_extraction_reads_into_the_tools_call() {
        let known = |w: &str| ["Maya", "John"].contains(&w);
        let e = read_extraction(
            r#"Sure: {"people": [{"who": "Maya", "wearing": "a red scarf"}], "camera": " "}"#,
            &known,
        )
        .unwrap();
        assert_eq!(
            e.call("images/a.png"),
            serde_json::json!({"picture": "images/a.png",
                "scene": {"people": [{"who": "Maya", "wearing": "a red scarf"}]}})
        );
        // Nothing asked: a redraw of the picture.
        let e = read_extraction("{}", &known).unwrap();
        assert_eq!(
            e.call("images/a.png"),
            serde_json::json!({"picture": "images/a.png"})
        );
        assert_eq!(e.summary(), "drawn again");
        // Keys a schema-held model sends empty are nothing asked, too.
        let e = read_extraction(r#"{"light": null, "camera": "", "people": []}"#, &known).unwrap();
        assert_eq!(e.call("p"), serde_json::json!({"picture": "p"}));
        // A newline in what the owner typed never moves the result's line.
        let f = fact("images/a.png", "a red\nscarf", "image: images/b.png");
        assert_eq!(fact_result(&f), Some("image: images/b.png"));
        // A retouch beside a scene change is left out and said.
        let e =
            read_extraction(r#"{"light": "dusk", "retouch": "a red umbrella"}"#, &known).unwrap();
        assert_eq!(e.retouch, None);
        assert!(e.summary().contains("not drawn"), "{}", e.summary());
        // A retouch alone.
        let e = read_extraction(r#"{"retouch": "a red umbrella"}"#, &known).unwrap();
        assert_eq!(e.call("p")["retouch"], "a red umbrella");
    }

    #[test]
    fn a_name_nobody_holds_is_a_failure_and_a_description_is_not() {
        let known = |w: &str| w == "Maya";
        let why = read_extraction(
            r#"{"people": [{"who": "Bob", "wearing": "a coat", "doing": "waving"}]}"#,
            &known,
        )
        .unwrap_err();
        assert!(why.contains("Bob is not in this picture"), "{why}");
        assert!(read_extraction(
            r#"{"people": [{"who": "a waiter with a tray", "wearing": "black", "doing": "serving"}]}"#,
            &known
        )
        .is_ok());
        // One person at the top level, as a model without the schema answers.
        let e = read_extraction(
            r#"{"who": "Maya", "doing": "sitting on the steps"}"#,
            &known,
        )
        .unwrap();
        assert_eq!(
            e.call("p"),
            serde_json::json!({"picture": "p", "scene": {"people": [{"who": "Maya", "doing": "sitting on the steps"}]}})
        );
        // A bare removal takes out the record's one person; with no sole
        // person it is a failure, said.
        let e = read_extraction_for(
            r#"{"remove": true}"#,
            &known,
            Some("Maya"),
            &Looks::default(),
        )
        .unwrap();
        assert_eq!(e.scene["people"][0]["remove"], true);
        assert_eq!(e.scene["people"][0]["who"], "Maya");
        assert!(read_extraction(r#"{"remove": true}"#, &known).is_err());
        // Keys this reader cannot use are a failure, never a redraw.
        let why = read_extraction(r#"{"pose": "sitting", "mood": "calm"}"#, &known).unwrap_err();
        assert!(why.contains("nothing this change can use"), "{why}");
        for bad in [
            "no json here",
            "[1, 2]",
            r#"{"people": "x"}"#,
            r#"{"light": 3}"#,
        ] {
            assert!(read_extraction(bad, &known).is_err(), "{bad}");
        }
    }

    /// A change of look is the owner's words in `look`, made a change by
    /// code: a held style by name, any other look in the setting's words, and
    /// over the owner's photo a refusal. "Change the style to hyperrealistic"
    /// once had no field and became a retouch that drew the same picture
    /// again; given `style`, the reader then chose the nearest name for a look
    /// the library lacks (mecha-a3's G605, 2026-10-08).
    #[test]
    fn a_change_of_look_is_matched_by_code() {
        assert!(extraction_schema()["properties"]["look"].is_object());
        assert!(extraction_schema()["properties"].get("style").is_none());
        assert!(EXTRACTION_SYSTEM.contains("`look` when the owner asks for a different look"));
        let styles = vec!["ink-wash".to_string(), "black-and-white".to_string()];
        let r = extraction_request("m", None, "make it noir", None, &styles, true);
        let body: serde_json::Value = serde_json::from_str(&r.messages[0].text()).unwrap();
        assert_eq!(
            body["styles"],
            serde_json::json!(["ink-wash", "black-and-white"])
        );
        let known = |_: &str| true;
        let words = |text: &str| crate::scene::Scene {
            setting: Some(crate::scene::Field {
                value: crate::scene::Setting::Words { text: text.into() },
                origin: crate::scene::Origin::Clean,
            }),
            ..Default::default()
        };
        let harbour = words("a quiet harbour.");
        let looks = Looks {
            styles: &styles,
            record: Some(&harbour),
            ..Default::default()
        };
        let read = |t: &str| read_extraction_for(t, &known, None, &looks);
        // A held style, however it is said, and a retouch beside it left out.
        for said in ["Black and white", "black-and-white style", "INK WASH look"] {
            let e = read(&format!(r#"{{"look": "{said}", "retouch": "{said}"}}"#)).unwrap();
            assert!(
                ["black-and-white", "ink-wash"].contains(&e.scene["style"].as_str().unwrap()),
                "{said}: {:?}",
                e.scene
            );
            assert!(e.call("p").get("retouch").is_none() && e.left_out.is_some());
        }
        // Any other look joins the record's setting, in the owner's words.
        let e = read(r#"{"look": "pencil sketch"}"#).unwrap();
        assert_eq!(
            e.scene["setting"],
            "a quiet harbour, in the look of pencil sketch"
        );
        assert!(e.scene.get("style").is_none());
        // With no setting to join, words alone would replace the picture:
        // refused, as the photo is.
        let bare_record = crate::scene::Scene::default();
        let no_setting = Looks {
            styles: &styles,
            record: Some(&bare_record),
            ..Default::default()
        };
        let why = read_extraction_for(r#"{"look": "pencil sketch"}"#, &known, None, &no_setting)
            .unwrap_err();
        assert!(why.contains("no setting a look can join"), "{why}");
        // An empty or null `look` beside a `style` falls through to it.
        let e = read(r#"{"look": "", "style": "ink wash"}"#).unwrap();
        assert_eq!(e.scene["style"], "ink-wash");
        let e = read(r#"{"look": null, "style": "ink wash"}"#).unwrap();
        assert_eq!(e.scene["style"], "ink-wash");
        // The picture's own style named again, over an earlier look in
        // words, takes those words off: a change, not a redraw.
        let both = crate::scene::Scene {
            style: Some(crate::scene::Field {
                value: "ink-wash".to_string(),
                origin: crate::scene::Origin::Clean,
            }),
            ..words("a quiet harbour, in the look of pencil sketch")
        };
        let back = Looks {
            styles: &styles,
            record: Some(&both),
            ..Default::default()
        };
        let e = read_extraction_for(r#"{"look": "ink wash"}"#, &known, None, &back).unwrap();
        assert_eq!(e.scene["setting"], "a quiet harbour");
        // Beside a new place, it joins that one.
        let e = read(r#"{"setting": "a windswept beach", "look": "1920s postcard"}"#).unwrap();
        assert_eq!(
            e.scene["setting"],
            "a windswept beach, in the look of 1920s postcard"
        );
        // Over the owner's photo, words would replace it: refused.
        let photo = crate::scene::Scene {
            setting: Some(crate::scene::Field {
                value: crate::scene::Setting::Photo {
                    path: "inbox/room.jpg".into(),
                    hash: "h".into(),
                },
                origin: crate::scene::Origin::Clean,
            }),
            ..Default::default()
        };
        let on_photo = Looks {
            styles: &styles,
            record: Some(&photo),
            ..Default::default()
        };
        let why = read_extraction_for(r#"{"look": "pencil sketch"}"#, &known, None, &on_photo)
            .unwrap_err();
        assert!(why.contains("name one of the library's styles"), "{why}");
        // Beside another change it is left out and said; the change draws.
        let e = read_extraction_for(
            r#"{"people": [{"who": "Maya", "expression": "a smile"}], "look": "pencil sketch"}"#,
            &known,
            None,
            &on_photo,
        )
        .unwrap();
        assert_eq!(e.scene["people"][0]["expression"], "a smile");
        assert!(e.scene.get("setting").is_none());
        assert!(e
            .left_out
            .as_deref()
            .is_some_and(|l| l.contains("would replace the photo")));
        // A held style over the photo is fine: the photo stays.
        let e = read_extraction_for(r#"{"look": "ink wash"}"#, &known, None, &on_photo).unwrap();
        assert_eq!(e.scene["style"], "ink-wash");
        let styled = crate::scene::Scene {
            style: Some(crate::scene::Field {
                value: "ink-wash".to_string(),
                origin: crate::scene::Origin::Clean,
            }),
            ..Default::default()
        };
        assert_eq!(record_for(&styled)["style"], "ink-wash");
        // A second look replaces the first, never stacks beside it.
        let sketched = words("a quiet harbour, in the look of pencil sketch");
        let again = Looks {
            styles: &styles,
            record: Some(&sketched),
            ..Default::default()
        };
        // And a held style after it takes the earlier look's words off.
        let e = read_extraction_for(r#"{"look": "ink wash"}"#, &known, None, &again).unwrap();
        assert_eq!(e.scene["style"], "ink-wash");
        assert_eq!(e.scene["setting"], "a quiet harbour");
        let e = read_extraction_for(r#"{"look": "pixel art"}"#, &known, None, &again).unwrap();
        assert_eq!(
            e.scene["setting"],
            "a quiet harbour, in the look of pixel art"
        );
        // The picture's own style said back is no change; a locked style
        // (approved in the library, never offered) is still that style.
        let dir = std::env::temp_dir().join(format!("mecha-edit-{}", uuid::Uuid::new_v4()));
        crate::imagelib::create(
            &dir,
            crate::imagelib::NewEntry {
                kind: crate::imagelib::Kind::Style,
                name: "secret-wash".into(),
                text: "a look the owner keeps out of sight".into(),
                portrait: None,
                source_seed: None,
                origin: crate::imagelib::Origin::Owner,
                locked: true,
            },
        )
        .unwrap();
        let lib = crate::imagelib::Library::load(&dir).0;
        let mine = crate::scene::Scene {
            style: Some(crate::scene::Field {
                value: "secret-wash".to_string(),
                origin: crate::scene::Origin::Clean,
            }),
            ..words("a quiet harbour")
        };
        let held = Looks {
            styles: &styles,
            record: Some(&mine),
            library: Some(&lib),
        };
        let e = read_extraction_for(
            r#"{"style": "secret wash", "light": "dusk"}"#,
            &known,
            None,
            &held,
        )
        .unwrap();
        assert!(e.scene.get("style").is_none() && e.scene.get("setting").is_none());
        let elsewhere = Looks {
            styles: &styles,
            record: Some(&harbour),
            library: Some(&lib),
        };
        let e =
            read_extraction_for(r#"{"look": "secret wash"}"#, &known, None, &elsewhere).unwrap();
        assert_eq!(e.scene["style"], "secret-wash");
        // Over a picture drawn in a library style, an unheld look still goes
        // in the setting's words, never refused.
        let e = read_extraction_for(r#"{"look": "pencil sketch"}"#, &known, None, &held).unwrap();
        assert_eq!(
            e.scene["setting"],
            "a quiet harbour, in the look of pencil sketch"
        );
        // The picture's own style said alone is understood: another try.
        let e = read_extraction_for(r#"{"look": "secret wash"}"#, &known, None, &held).unwrap();
        assert!(e.scene.is_empty() && e.retouch.is_none(), "{:?}", e.scene);
        std::fs::remove_dir_all(dir).ok();
    }

    /// The photo stand-in said alone is the record restated and left out;
    /// with a look added it would replace the owner's photo, and is refused;
    /// a real change of place goes through. And a style named `…-look`, the
    /// record's own `style` key, and "style" alone are each read right
    /// (review of #605).
    #[test]
    fn a_look_over_the_owners_photo_never_replaces_it() {
        let known = |_: &str| true;
        let styles = vec!["secret-look".to_string(), "ink-wash".to_string()];
        let photo = crate::scene::Scene {
            setting: Some(crate::scene::Field {
                value: crate::scene::Setting::Photo {
                    path: "inbox/room.jpg".into(),
                    hash: "h".into(),
                },
                origin: crate::scene::Origin::Clean,
            }),
            ..Default::default()
        };
        let looks = Looks {
            styles: &styles,
            record: Some(&photo),
            ..Default::default()
        };
        let read = |t: &str| read_extraction_for(t, &known, None, &looks);
        // A typographic apostrophe is the same stand-in.
        for said in [
            "The owner's photo, in watercolour",
            "the owner\u{2019}s photo, as a sketch",
        ] {
            let why = read(&format!(r#"{{"setting": "{said}"}}"#)).unwrap_err();
            assert!(why.contains("would replace the photo"), "{why}");
        }
        // Off a photo there is nothing to replace: the guard stays out.
        let plain = Looks {
            styles: &styles,
            ..Default::default()
        };
        assert!(read_extraction_for(
            r#"{"setting": "the owner's photo, in watercolour"}"#,
            &known,
            None,
            &plain
        )
        .is_ok());
        let e = read(
            r#"{"setting": "the owner's photo.", "people": [{"who": "Maya", "expression": "a smile"}]}"#,
        )
        .unwrap();
        assert!(e.scene.get("setting").is_none(), "{:?}", e.scene);
        assert_eq!(e.scene["people"][0]["expression"], "a smile");
        // Said back alone, it is nothing asked: another try, not a failure.
        let e = read(r#"{"setting": "the owner's photo"}"#).unwrap();
        assert!(e.scene.is_empty() && e.retouch.is_none());
        let e = read(r#"{"setting": "a windswept beach"}"#).unwrap();
        assert_eq!(e.scene["setting"], "a windswept beach");
        assert_eq!(
            read(r#"{"look": "secret look"}"#).unwrap().scene["style"],
            "secret-look"
        );
        assert_eq!(
            read(r#"{"style": "Ink Wash"}"#).unwrap().scene["style"],
            "ink-wash"
        );
        let e = read(r#"{"look": "style", "light": "dusk"}"#).unwrap();
        assert!(e.scene.get("setting").is_none() && e.scene.get("style").is_none());
    }

    /// A look naming a style that waits on the owner is that finding, said
    /// by name, never words in the setting (review of #605).
    #[test]
    fn a_look_naming_a_waiting_style_is_said_by_name() {
        let dir = std::env::temp_dir().join(format!("mecha-edit-{}", uuid::Uuid::new_v4()));
        crate::imagelib::create(
            &dir,
            crate::imagelib::NewEntry {
                kind: crate::imagelib::Kind::Style,
                name: "chalk-pastel".into(),
                text: "soft chalk pastel on toned paper".into(),
                portrait: None,
                source_seed: None,
                origin: crate::imagelib::Origin::ModelUntrusted,
                locked: false,
            },
        )
        .unwrap();
        let lib = crate::imagelib::Library::load(&dir).0;
        let looks = Looks {
            library: Some(&lib),
            ..Default::default()
        };
        let known = |_: &str| true;
        let why =
            read_extraction_for(r#"{"look": "chalk pastel"}"#, &known, None, &looks).unwrap_err();
        assert!(why.contains("waiting for the owner's approval"), "{why}");
        std::fs::remove_dir_all(dir).ok();
    }

    /// With the persona's latest reply, the reader is asked as mecha-a3
    /// measured: the reply appended to the owner's words, and the sentence
    /// that ranks them before the last instruction; without one, the edit
    /// panel's request exactly.
    #[test]
    fn a_reply_rides_beside_the_owners_words_ranked_below_them() {
        let r = reading_request(
            "m",
            None,
            "draw us",
            Some("a walk by the sea"),
            None,
            &[],
            true,
        );
        let body: serde_json::Value = serde_json::from_str(&r.messages[0].text()).unwrap();
        assert_eq!(
            body["owner"],
            "draw us\n\n(What the persona said in its last reply: a walk by the sea)"
        );
        let system = r.system.as_deref().unwrap_or_default().to_string();
        assert!(system.contains(REPLY_DECIDES), "{system}");
        assert!(
            system.ends_with(&format!("{REPLY_DECIDES} Answer with JSON only.")),
            "{system}"
        );
        let plain = reading_request("m", None, "draw us", None, None, &[], true);
        let panel = extraction_request("m", None, "draw us", None, &[], true);
        assert_eq!(plain.system, panel.system);
        assert_eq!(plain.messages[0].text(), panel.messages[0].text());
        assert!(!plain
            .system
            .as_deref()
            .unwrap_or_default()
            .contains(REPLY_DECIDES));
    }

    #[test]
    fn the_request_is_quarantined_fast_and_names_the_record() {
        let r = extraction_request("m", None, "make her smile", Some("Maya"), &[], true);
        assert!(r.tools.is_empty());
        assert_eq!(r.messages.len(), 1);
        assert_eq!(r.think, Some(false));
        assert!(r.response_schema.is_some());
        let body: serde_json::Value = serde_json::from_str(&r.messages[0].text()).unwrap();
        assert_eq!(body["owner"], "make her smile");
        assert_eq!(body["persona"], "Maya");
        assert_eq!(body["scene"], serde_json::json!({}));
        assert!(body.get("styles").is_none(), "no styles, no key");
        assert!(extraction_request("m", None, "x", None, &[], false)
            .response_schema
            .is_none());
    }
}
