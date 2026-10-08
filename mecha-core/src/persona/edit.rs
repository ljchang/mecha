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

/// A `setting` that restates the photo stand-in ("the owner's photo, in
/// watercolour") would replace the owner's photo in the record with words,
/// and the picture would no longer be placed in it (review of #605). That
/// one is refused with the way on; a real change of place over the photo
/// still goes through, as the owner asked.
pub fn photo_kept(extracted: &Extracted) -> Result<(), String> {
    let echoes = extracted
        .scene
        .get("setting")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|s| s.to_lowercase().contains(PHOTO_SETTING));
    if echoes {
        return Err(
            "this picture is placed on the owner's photo, and a look in words would \
                    replace the photo: name one of the library's styles instead"
                .into(),
        );
    }
    Ok(())
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
        .system(EXTRACTION_SYSTEM)
        .no_thinking()
        .response_schema(structured.then(extraction_schema))
        .ask(serde_json::Value::Object(user).to_string())
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
    pub styles: &'a [String],
    pub record: Option<&'a crate::scene::Scene>,
}

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
) -> Result<(), String> {
    let words = look.trim();
    let lower = words.to_lowercase();
    let bare = ["style", "look"]
        .iter()
        .find_map(|tail| lower.strip_suffix(tail))
        .map_or(lower.as_str(), str::trim);
    let key = crate::imagelib::spelled_as_name(bare);
    if looks.styles.contains(&key) {
        scene.insert("style".into(), key.into());
        return Ok(());
    }
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
    let setting = match base {
        Some(b) => format!("{}, in the look of {words}", b.trim_end_matches('.')),
        None => format!("in the look of {words}"),
    };
    scene.insert("setting".into(), setting.into());
    Ok(())
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
    match obj.get("look") {
        None | Some(serde_json::Value::Null) => {}
        Some(serde_json::Value::String(t)) if t.trim().is_empty() => {}
        Some(serde_json::Value::String(t)) => look_into(t, looks, &mut scene)?,
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
    let said_something = obj.values().any(|v| match v {
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

/// A painted area is a retouch by definition: the owner's words go to the
/// tool as written, with the mask, and no extraction runs.
pub fn masked_call(picture: &str, mask: &str, words: &str) -> serde_json::Value {
    serde_json::json!({ "picture": picture, "retouch": words, "mask": mask })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        let why = read_extraction_for(r#"{"look": "pencil sketch"}"#, &known, None, &on_photo)
            .unwrap_err();
        assert!(why.contains("name one of the library's styles"), "{why}");
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
