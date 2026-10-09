//! The image tool's call and plan (`IMAGE-DESIGN.md` §5).
//!
//! One call shape: the picture being changed, a scene (a whole one for a new
//! picture, or the fields that change), one retouch in words, a mask, a size.
//! [`parse`] reads it and resolves who each person is; [`plan`] reads it
//! against the picture's record and picks the one render it becomes, with the
//! scene the render lands as. Both are pure: reading pictures, crops and
//! portraits, and rendering, are `imagegen`'s.

use crate::scene::{Delta, PersonChange, Scene, SceneChange, Setting, Where, Who, Words};
use serde_json::Value;

/// The most people with faces one picture draws: a new picture's portraits,
/// or an edit's crops beside its canvas. Five, on the owner's ruling of
/// 2026-10-07 (C5 and N5 drew five correctly; provisional).
pub const MAX_FACES: usize = crate::imagelib::MAX_CAST;

/// The persona's own names, which `who` resolves to its linked character.
#[derive(Debug, Clone, Default)]
pub struct SelfNames {
    /// The persona's approved library character, if it has one.
    pub character: Option<String>,
    /// Every name the persona answers to: its display name and folder.
    pub names: Vec<String>,
}

/// One call, read and resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    /// The picture being changed, a workspace path.
    pub picture: Option<String>,
    pub change: SceneChange,
    /// A photo used as the setting, a workspace path.
    pub setting_photo: Option<String>,
    pub retouch: Option<String>,
    pub mask: Option<String>,
    pub size: Option<crate::imagegen::Size>,
    /// A new picture's seed: only where `seeds` admits one (the CLI and
    /// evals, never a chat; IMAGE-DESIGN.md §5.1).
    pub seed: Option<u64>,
    /// What the call over-filled and was left out, said in the result
    /// rather than refused (mecha-a3's G1b: optional fields a model fills
    /// with the wrong thing are its most common shape refusal).
    pub notes: Vec<String>,
}

impl Call {
    /// Whether the call describes something to draw without a `picture`: a
    /// scene change or a photo for the setting. Parsing refuses a call with
    /// neither, and a made-up `picture` is left out only beside one.
    pub fn has_scene(&self) -> bool {
        self.change != SceneChange::default() || self.setting_photo.is_some()
    }
}

/// Read a call. `approved` says whether a name is an approved library
/// character; `me` resolves `self` and the persona's names; `seeds` admits a
/// new picture's `seed`, which no chat schema carries.
pub fn parse(
    input: &Value,
    approved: &dyn Fn(&str) -> bool,
    me: &SelfNames,
    seeds: bool,
) -> Result<Call, String> {
    let obj = input.as_object().ok_or("image_generate takes an object.")?;
    for retired in [
        "prompt",
        "cast",
        "extras",
        "edit",
        "negative_prompt",
        "reference_images",
    ]
    .into_iter()
    .chain((!seeds).then_some("seed"))
    {
        if obj.get(retired).is_some_and(|v| !v.is_null()) {
            return Err(format!(
                "`{retired}` is not part of this tool. Describe the picture in `scene` (a whole \
                 scene for a new picture, or only what changes), name the picture being changed \
                 in `picture`, and put one small change to the picture itself in `retouch`."
            ));
        }
    }
    let text = |v: Option<&Value>, k: &str, cap: usize| -> Result<Option<String>, String> {
        match v {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(t)) if t.trim().is_empty() => Ok(None),
            Some(Value::String(t)) if t.chars().count() > cap => {
                Err(format!("`{k}` is at most {cap} characters."))
            }
            Some(Value::String(t)) => Ok(Some(t.trim().to_string())),
            Some(_) => Err(format!("`{k}` must be text.")),
        }
    };
    let field = crate::imagelib::MAX_CAST_FIELD;
    let prose = crate::imagegen::PROMPT_CAP;
    let picture = text(obj.get("picture"), "picture", 200)?;
    let retouch = text(obj.get("retouch"), "retouch", prose)?;
    let mut notes: Vec<String> = Vec::new();
    // A mask is a painted picture's path, from the owner's message. Anything
    // else in it (prose, a name, an object) is left out and said, never a
    // refusal (G1b: 3 of 65).
    let mut mask = match obj.get("mask") {
        None | Some(Value::Null) => None,
        Some(Value::String(t)) if t.trim().is_empty() => None,
        Some(Value::String(t)) if looks_like_a_picture_path(t) => Some(t.trim().to_string()),
        Some(_) => {
            notes.push(
                "The `mask` given was not a painted picture's path, so it was left out.".into(),
            );
            None
        }
    };
    let size = match text(obj.get("size"), "size", 20)? {
        None => None,
        Some(s) => Some(
            crate::imagegen::Size::parse(&s).ok_or("`size` is square, landscape or portrait.")?,
        ),
    };
    if mask.is_some() && (retouch.is_none() || picture.is_none()) {
        notes.push(
            "The `mask` was left out: a painted area goes with a `retouch` of the `picture` it \
             was painted on."
                .into(),
        );
        mask = None;
    }
    let seed = match obj.get("seed") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().ok_or("`seed` is a whole number.")?),
    };
    if seed.is_some() && picture.is_some() {
        return Err("A `seed` draws a new picture; a change to one samples afresh.".into());
    }
    let mut change = SceneChange::default();
    let mut setting_photo = None;
    match obj.get("scene") {
        None | Some(Value::Null) => {}
        Some(Value::Object(sc)) => {
            match sc.get("setting") {
                None | Some(Value::Null) => {}
                Some(Value::String(t)) if t.trim().is_empty() => {}
                Some(Value::String(t)) => {
                    if t.chars().count() > prose {
                        return Err(format!("`scene.setting` is at most {prose} characters."));
                    }
                    change.setting = Some(Setting::Words {
                        text: t.trim().to_string(),
                    });
                }
                Some(Value::Object(o)) => {
                    let p = text(o.get("photo"), "scene.setting.photo", 200)?
                        .ok_or("`scene.setting` as a photo is `{\"photo\": <path>}`.")?;
                    setting_photo = Some(p);
                }
                Some(_) => {
                    return Err(
                        "`scene.setting` is words, or `{\"photo\": <path>}` for a room.".into(),
                    )
                }
            }
            change.light = text(sc.get("light"), "scene.light", field)?;
            change.camera = text(sc.get("camera"), "scene.camera", field)?;
            // A style longer than a library name cannot be one: clipped to
            // the field cap, never a shape refusal, so the tool leaves it out
            // and says so as it does any style the library does not hold
            // (review of #603).
            change.style = match sc.get("style") {
                Some(Value::String(t)) if t.chars().count() > field => {
                    Some(t.trim().chars().take(field).collect())
                }
                other => text(other, "scene.style", field)?,
            };
            // Clipped at a sentence rather than refused (G1b: 3 of 65).
            change.together = match sc.get("together") {
                Some(Value::String(t)) if t.chars().count() > field => {
                    Some(clip_at_sentence(t.trim(), field))
                }
                other => text(other, "scene.together", field)?,
            };
            if let Some(t) = sc.get("text").filter(|v| !v.is_null()) {
                let items = t
                    .as_array()
                    .ok_or("`scene.text` is a list of words to render.")?;
                let mut words = Vec::new();
                for it in items {
                    let w = text(it.get("words"), "scene.text.words", field)?
                        .ok_or("each `scene.text` entry needs `words`.")?;
                    words.push(Words {
                        words: w,
                        at: text(it.get("where"), "scene.text.where", field)?.unwrap_or_default(),
                        look: text(it.get("look"), "scene.text.look", field)?.unwrap_or_default(),
                    });
                }
                change.text = Some(words);
            }
            match sc.get("people") {
                None | Some(Value::Null) => {}
                Some(Value::Array(items)) => {
                    if items.len() > crate::scene::MAX_PEOPLE {
                        return Err(format!(
                            "`scene.people` names at most {} people.",
                            crate::scene::MAX_PEOPLE
                        ));
                    }
                    for v in items {
                        let who = text(v.get("who"), "scene.people.who", field)?
                            .ok_or("each `scene.people` entry needs `who`.")?;
                        // A place in the frame outside the four is left out
                        // and said, never a refusal (mecha-a3's G1).
                        let at = match v.get("where").and_then(Value::as_str).map(str::trim) {
                            None | Some("") => None,
                            Some(w) => match Where::parse(w) {
                                Some(at) => Some(at),
                                None => {
                                    notes.push(format!(
                                        "`where` for {who} was left out: it is left, centre, \
                                         right or background."
                                    ));
                                    None
                                }
                            },
                        };
                        let remove = match v.get("remove") {
                            None | Some(Value::Null) => false,
                            Some(Value::Bool(b)) => *b,
                            Some(_) => return Err("`remove` must be true or false.".into()),
                        };
                        change.people.push(PersonChange {
                            who: resolve(&who, approved, me)?,
                            at,
                            wearing: text(v.get("wearing"), "scene.people.wearing", field)?,
                            doing: text(v.get("doing"), "scene.people.doing", field)?,
                            expression: text(
                                v.get("expression"),
                                "scene.people.expression",
                                field,
                            )?,
                            remove,
                        });
                    }
                }
                Some(_) => return Err("`scene.people` must be a list of people.".into()),
            }
        }
        Some(_) => return Err("`scene` must be an object.".into()),
    }
    // One person once: two entries that resolve to the same person (`self`
    // and the persona's own name, say) are one, the first's fields filled
    // from the second's (mecha-a3's G1b: 3 of 65 refused as "twice").
    let mut merged: Vec<PersonChange> = Vec::new();
    for p in std::mem::take(&mut change.people) {
        match merged.iter_mut().find(|q| q.who.key() == p.who.key()) {
            Some(q) => {
                q.at = q.at.or(p.at);
                q.wearing = q.wearing.take().or(p.wearing);
                q.doing = q.doing.take().or(p.doing);
                q.expression = q.expression.take().or(p.expression);
                q.remove = q.remove || p.remove;
            }
            None => merged.push(p),
        }
    }
    change.people = merged;
    // A relation naming one person, with nobody in `people`: that person,
    // doing it (review T1 folds a solo relation into the one `doing`; G1b:
    // 4 of 65 named the persona only there), on a new picture or a change.
    if change.people.is_empty() {
        if let Some(t) = change.together.clone() {
            // Only the persona: someone is drawn only when listed in
            // `people` (owner, 2026-10-08), and the measured case was the
            // persona naming itself (review of #597, pass 8).
            let named: Vec<Who> = names_in(&t, approved, me);
            let mine = me.character.as_ref().map(|c| Who::Library(c.clone()));
            let folded = mine
                .as_ref()
                .filter(|m| named.len() == 1 && named[0] == **m);
            if let Some(who) = folded {
                // The relation names them; as their own `doing` it reads
                // without the name, or an edit line says "Have Maya Maya
                // waves" (review of #597, pass 7).
                let doing = strip_leading_name(&t, who, me);
                change.people.push(PersonChange {
                    who: who.clone(),
                    at: None,
                    wearing: None,
                    doing: Some(doing),
                    expression: None,
                    remove: false,
                });
                change.together = None;
            }
        }
    }
    let call = Call {
        picture,
        change,
        setting_photo,
        retouch,
        mask,
        size,
        seed,
        notes,
    };
    if call.picture.is_none() && !call.has_scene() {
        return Err(
            "Nothing to draw: give a `scene` for a new picture, or the `picture` to change.".into(),
        );
    }
    Ok(call)
}

/// How the tool opens the sentence for words the picture renders: the one
/// place `as_viewer` leaves a name as written.
pub(crate) const RENDERED: &str = "The words ";

/// `text` with each whole-word mention of `names` (library keys, any case,
/// possessives kept) read as "the viewer": what the image model reads for
/// someone named but not in the picture. The words the picture renders
/// (`keep`, from `scene.text`) are left exactly as written, found as their
/// own quoted phrase, so neither a stray `"` elsewhere (review of #597,
/// pass 7) nor a rendered word that is also a name (pass 8) can switch the
/// rewrite off anywhere else.
pub fn as_viewer(text: &str, names: &[String], keep: &[&str]) -> String {
    if names.is_empty() {
        return text.to_string();
    }
    // Each rendered phrase, set aside under a marker no prompt holds.
    let mut held = text.to_string();
    // Only the phrase where the tool renders it, in the sentence it writes
    // for rendered words (`RENDERED`): a bare substring ("John", or "o"), or
    // the same phrase quoted in prose, would shield the name there too
    // (review of #597, passes 8 and 9).
    for (i, k) in keep.iter().enumerate().filter(|(_, k)| !k.is_empty()) {
        held = held.replace(
            &format!("{RENDERED}\"{k}\""),
            &format!("{RENDERED}\"\u{E000}{i}\u{E000}\""),
        );
    }
    let mut out = String::with_capacity(held.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if names.iter().any(|n| n.eq_ignore_ascii_case(word)) {
            out.push_str("the viewer");
        } else {
            out.push_str(word);
        }
        word.clear();
    };
    for c in held.chars() {
        if c.is_alphanumeric() || c == '-' {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    for (i, k) in keep.iter().enumerate().filter(|(_, k)| !k.is_empty()) {
        out = out.replace(&format!("\u{E000}{i}\u{E000}"), k);
    }
    out
}

/// `t` without a leading mention of `who` (or the persona's own names).
fn strip_leading_name(t: &str, who: &Who, me: &SelfNames) -> String {
    let mut names: Vec<String> = vec![shown(who)];
    names.extend(me.names.iter().cloned());
    for n in names.iter().filter(|n| !n.trim().is_empty()) {
        let n = n.trim();
        if t.len() > n.len()
            && t.is_char_boundary(n.len())
            && t[..n.len()].eq_ignore_ascii_case(n)
            && t[n.len()..].starts_with(' ')
        {
            return t[n.len()..].trim().to_string();
        }
    }
    t.to_string()
}

/// A painted picture's workspace path: one plain path to an image file.
fn looks_like_a_picture_path(t: &str) -> bool {
    let t = t.trim();
    let lower = t.to_lowercase();
    t.chars().count() <= 200
        && t.contains('/')
        && !t.contains(char::is_whitespace)
        && [".png", ".jpg", ".jpeg", ".webp"]
            .iter()
            .any(|e| lower.ends_with(e))
}

/// `t` cut to at most `cap` characters, at the last sentence end inside it,
/// else at the last word.
pub(crate) fn clip_at_sentence(t: &str, cap: usize) -> String {
    let head: String = t.chars().take(cap).collect();
    let cut = head
        .rfind(['.', '!', '?'])
        .map(|i| i + 1)
        .or_else(|| head.rfind(' '))
        .unwrap_or(head.len());
    head[..cut].trim().to_string()
}

/// The people a sentence names: the persona by any of its names, and
/// approved library characters, each once.
fn names_in(text: &str, approved: &dyn Fn(&str) -> bool, me: &SelfNames) -> Vec<Who> {
    let mut out: Vec<Who> = Vec::new();
    for word in text
        .split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .filter(|w| !w.is_empty())
    {
        let key = word.to_lowercase();
        let who = if me.names.iter().any(|n| n.trim().to_lowercase() == key)
            || me.character.as_deref() == Some(key.as_str())
        {
            me.character.as_ref().map(|c| Who::Library(c.clone()))
        } else if approved(&key) {
            Some(Who::Library(key))
        } else {
            None
        };
        if let Some(w) = who {
            if !out.contains(&w) {
                out.push(w);
            }
        }
    }
    out
}

/// Who a `who` names: the persona itself, a library character, or someone
/// described in words.
fn resolve(who: &str, approved: &dyn Fn(&str) -> bool, me: &SelfNames) -> Result<Who, String> {
    let key = who.trim().to_lowercase();
    let is_me = key == "self"
        || me.names.iter().any(|n| n.trim().to_lowercase() == key)
        || me
            .character
            .as_deref()
            .is_some_and(|c| c.trim().to_lowercase() == key);
    if is_me {
        return me
            .character
            .as_deref()
            .map(|c| Who::Library(c.trim().to_lowercase()))
            .ok_or_else(|| {
                "This persona has no approved library character, so there is no \"self\" to \
                 draw; describe the persona in `who` instead."
                    .to_string()
            });
    }
    Ok(if approved(&key) {
        Who::Library(key)
    } else {
        Who::Described(who.trim().to_string())
    })
}

/// A person as a sentence names them.
pub fn shown(who: &Who) -> String {
    match who {
        Who::Library(n) => crate::imagegen::capitalized(n),
        Who::Described(d) => d.clone(),
    }
}

/// What a call becomes on the image model (§5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Render {
    /// A new picture from words and library portraits.
    New,
    /// An edit of a canvas: a photo used as the setting, or the current
    /// picture. `camera_moves` says whether the canvas keeps its camera.
    Edit { canvas: Canvas, camera_moves: bool },
}

/// The picture an edit-shaped render works on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Canvas {
    /// The scene's setting photo, the people placed into it afresh.
    Setting(String),
    /// The picture being changed.
    Picture(String),
}

/// How a render takes its seed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seed {
    /// The seed a CLI or eval call gave a new picture.
    Given(u64),
    /// A fresh seed: every edit (#306), and a new picture.
    Fresh,
    /// The base picture's seed: a restage on words keeps its room (§2.6).
    Base(u64),
}

/// The one render a call becomes, and the scene it lands as.
#[derive(Debug, Clone)]
pub struct Plan {
    pub render: Render,
    /// Who is drawn, in left-to-right order.
    pub people: Vec<crate::scene::Person>,
    /// Of them, who gets a face (a portrait or a crop), by key.
    pub faces: Vec<String>,
    /// For an edit: the one change, as an instruction, in the #408 form's
    /// second half. Empty for a new picture.
    pub instruction: String,
    /// For an edit: what stays, the #408 form's first half.
    pub keep: String,
    pub seed: Seed,
    pub size: Option<crate::imagegen::Size>,
    pub mask: Option<String>,
    /// The scene this render lands as.
    pub next: Scene,
    pub delta: Delta,
    /// What the plan was, for the record and the result line: "new",
    /// "placed", "restaged", "edited", "retouched", "redrawn".
    pub route: &'static str,
    /// Said in the result: e.g. why a restage became an edit.
    pub said: Option<String>,
    /// A `retouch` given beside a scene change (or for a new picture): one
    /// more line of the render, folded in rather than refused (G1b: 13 of
    /// 65 shape refusals were this).
    pub also: Option<String>,
    /// Library characters named in the words but not in the picture: never
    /// drawn, and "the viewer" in what the image model reads ([`as_viewer`]).
    pub offstage: Vec<String>,
}

/// The left-to-right rank of a place in the frame; the background last.
pub(crate) fn rank(at: Option<Where>) -> u8 {
    match at {
        Some(Where::Left) => 0,
        Some(Where::Centre) | None => 1,
        Some(Where::Right) => 2,
        Some(Where::Background) => 3,
    }
}

/// Plan a call against its picture's record (§5.2).
///
/// `base` is the record of `call.picture` (`None`: a new picture, or a
/// picture this chat has no record of). `setting_photo_hash` is the hash of
/// the photo `call.setting_photo` names, read by the caller. `by` is the
/// run's origin. `approved` answers for library names in prose fields.
pub fn plan(
    call: &Call,
    base: Option<&Scene>,
    setting_photo_hash: Option<String>,
    by: crate::scene::Origin,
    approved: &dyn Fn(&str) -> bool,
    named_in: &dyn Fn(&str) -> Vec<String>,
    worn: &dyn Fn(&str) -> Option<(String, crate::scene::Origin)>,
) -> Result<Plan, String> {
    let mut change = call.change.clone();
    if let (Some(path), Some(hash)) = (&call.setting_photo, setting_photo_hash) {
        change.setting = Some(Setting::Photo {
            path: path.clone(),
            hash,
        });
    }
    let new_picture = call.picture.is_none();
    // A new picture that lists nobody in `people` but names library
    // characters in its words draws them, from the library (owner,
    // 2026-10-09): a chat asked for two of them at a café, listed neither,
    // and two strangers were drawn under "the viewer". Where `people` lists
    // anyone, a name left out of it stays the viewer (owner, 2026-10-08).
    let mut from_words: Vec<String> = Vec::new();
    if new_picture && change.people.is_empty() {
        let setting = match &change.setting {
            Some(Setting::Words { text }) => Some(text.as_str()),
            _ => None,
        };
        let words = [
            change.together.as_deref(),
            setting,
            change.light.as_deref(),
            change.camera.as_deref(),
            call.retouch.as_deref(),
        ];
        for text in words.into_iter().flatten() {
            for name in named_in(text) {
                if approved(&name) && !from_words.contains(&name) {
                    from_words.push(name);
                }
            }
        }
        // Bounded by what one picture can draw: the overflow stays offstage
        // ("the viewer"), never a refusal to `remove` someone the call never
        // listed (review of #613).
        from_words.truncate(MAX_FACES);
        change.people = from_words
            .iter()
            .map(|name| PersonChange {
                who: Who::Library(name.clone()),
                at: None,
                wearing: None,
                doing: None,
                expression: None,
                remove: false,
            })
            .collect();
    }
    // Someone to take out whom the record does not hold: on a picture whose
    // people are known, there is nobody to take out, and that is said; on
    // one whose people are not known yet, it is an edit of the picture
    // (review of #597: it was dropped, and the call read as a redraw).
    let known_before = base.is_some_and(|b| b.people_known);
    let mut unknown_removes: Vec<String> = Vec::new();
    for c in change.people.iter().filter(|c| c.remove) {
        let held = base.is_some_and(|b| b.people.iter().any(|p| p.who.key() == c.who.key()));
        if held {
            continue;
        }
        if known_before {
            return Err(format!(
                "{} is not in this picture, so there is nobody to take out.",
                shown(&c.who)
            ));
        }
        unknown_removes.push(shown(&c.who));
    }
    // A new picture defines its scene; it knows who it drew.
    let (mut next, delta) = Scene::apply(base, &change, by, new_picture);
    let mut notes: Vec<String> = call.notes.clone();
    if !from_words.is_empty() {
        let named: Vec<String> = from_words
            .iter()
            .map(|k| crate::imagegen::capitalized(k))
            .collect();
        notes.push(format!(
            "{} named in the words with nobody in `scene.people`, so drawn from the library. \
             List people in `scene.people` to say what each wears and does.",
            named.join(", ")
        ));
    }
    // A painted area keeps everything outside it, so it cannot carry a
    // scene change: the change is drawn, and the mask left out and said.
    let mask = call
        .mask
        .clone()
        .filter(|_| delta.is_empty() && unknown_removes.is_empty());
    if call.mask.is_some() && mask.is_none() {
        notes.push(
            "The `mask` was left out: a scene change redraws more than a painted area.".into(),
        );
    }
    // Someone the call introduces needs their clothes. Clothes left out are
    // what the chat's record has them in (`worn`), else, for a library
    // character, asked for (G1b: 8 of 65 shape refusals were a newcomer
    // missing one). A pose left out stays empty in the record: the plain one
    // is said where the prompt is built, and only when the scene does not
    // say what its people do together (`imagegen::doing_words`). Stored, it
    // could not be told from one the owner wrote, and came back beside a
    // later `together` (review of #608).
    for key in &delta.added {
        // Someone added past the scene's bound was cut from it (`apply`):
        // refused here, never drawn as a picture without them.
        let Some(p) = next.people.iter_mut().find(|p| &p.who.key() == key) else {
            return Err(format!(
                "A picture holds at most {} people; take someone out with `remove` first.",
                crate::scene::MAX_PEOPLE
            ));
        };
        if p.wearing.trim().is_empty() {
            match (worn(key), &p.who) {
                (Some((w, from)), _) => {
                    notes.push(format!(
                        // Never the recorded words themselves: they may be
                        // untrusted, and a tool result is not marked from
                        // outside (review of #597, pass 5).
                        "{} is wearing what this chat last drew them in.",
                        shown(&p.who)
                    ));
                    p.wearing = w;
                    // Copied words keep where they came from: an untrusted
                    // record's clothes never land clean (review of #597).
                    p.origin = p.origin.union(from);
                }
                (None, Who::Described(_)) => {
                    p.wearing = "clothes that suit the scene".into();
                }
                (None, Who::Library(_)) => {
                    // Absorbed, and said (mecha-a3's G1: a refusal here was
                    // the panel's most common one on a picture with no record).
                    notes.push(format!(
                        "{} had no clothes given or on record, so they wear clothes that suit \
                         the scene.",
                        shown(&p.who)
                    ));
                    p.wearing = "clothes that suit the scene".into();
                }
            }
        }
    }
    // A library name in prose is someone in the picture, or a stranger drawn
    // from words (E1): refused plainly unless they are in it.
    let in_picture: Vec<String> = next.people.iter().map(|p| p.who.key()).collect();
    // The words the prompt will carry: the scene as it lands (`next`), not
    // only the call's own fields, since a restage recompiles the record's
    // words too; one innocuous follow-up re-sent a recorded "Maya waves at
    // John" (review of #597, pass 6). Rendered `text` is not prose: it is
    // drawn as written, and `as_viewer` leaves quoted words alone.
    let mut prose: Vec<(&str, String)> = Vec::new();
    for (k, v) in [
        ("light", &next.light),
        ("camera", &next.camera),
        ("together", &next.together),
    ] {
        if let Some(f) = v {
            prose.push((k, f.value.clone()));
        }
    }
    if let Some(r) = &call.retouch {
        prose.push(("retouch", r.clone()));
    }
    if let Some(Setting::Words { text }) = next.setting.as_ref().map(|f| &f.value) {
        prose.push(("setting", text.clone()));
    }
    for p in &next.people {
        for (k, v) in [
            ("wearing", &p.wearing),
            ("doing", &p.doing),
            ("expression", &p.expression),
        ] {
            if !v.trim().is_empty() {
                prose.push((k, v.clone()));
            }
        }
        if let Who::Described(d) = &p.who {
            prose.push(("who", d.clone()));
        }
    }
    // Only where the picture's people are known: on a picture with no record
    // a name may well be someone already in it, and an edit keeps their face
    // from the canvas (mecha-a3's G1).
    let checks_names = new_picture || known_before;
    // Someone is drawn only when listed in `people` (the owner's ruling,
    // 2026-10-08). A library name in the words for someone not in the
    // picture is not drawn and not refused: it reaches the image model as
    // "the viewer", so no stranger is drawn under it (E1), and the result
    // says so. A persona addressing the owner by name ("…with Luke
    // watching") was every G1b refusal.
    let mut offstage: Vec<String> = Vec::new();
    for (_, text) in prose.iter().filter(|_| checks_names) {
        for name in named_in(text) {
            if approved(&name) && !in_picture.contains(&name) && !offstage.contains(&name) {
                offstage.push(name);
            }
        }
    }
    if !offstage.is_empty() {
        let named: Vec<String> = offstage
            .iter()
            .map(|k| crate::imagegen::capitalized(k))
            .collect();
        notes.push(format!(
            "{} named in the words but not in the picture, so not drawn: the image model was \
             told \"the viewer\". To draw them, put them in `scene.people`.",
            named.join(", ")
        ));
    }
    let mut people = next.people.clone();
    people.sort_by_key(|p| rank(p.at));
    let library = |p: &&crate::scene::Person| matches!(p.who, Who::Library(_));
    let faces_all: Vec<String> = people.iter().filter(library).map(|p| p.who.key()).collect();
    if faces_all.len() > MAX_FACES {
        return Err(format!(
            "This picture would hold {} people with faces ({}), and one picture draws at most \
             {MAX_FACES}. Take someone out with `remove`.",
            faces_all.len(),
            faces_all
                .iter()
                .map(|k| crate::imagegen::capitalized(k))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let base_seed = base.and_then(|b| b.seed);
    // Whether the scene has words to draw it afresh from: a restage or a
    // redraw drawn new needs them, or it draws an empty scene (review of
    // #597, pass 3; #591's `on_place` was this rule).
    let words_setting = matches!(
        next.setting.as_ref().map(|f| &f.value),
        Some(Setting::Words { .. })
    );
    let setting_photo = match next.setting.as_ref().map(|f| &f.value) {
        Some(Setting::Photo { path, .. }) => Some(path.clone()),
        _ => None,
    };
    let mut out = Plan {
        render: Render::New,
        people: people.clone(),
        faces: faces_all.clone(),
        instruction: String::new(),
        keep: String::new(),
        seed: call.seed.map_or(Seed::Fresh, Seed::Given),
        size: call.size,
        mask,
        next,
        delta: delta.clone(),
        route: "new",
        said: (!notes.is_empty()).then(|| notes.join(" ")),
        also: call
            .retouch
            .clone()
            .filter(|_| call.picture.is_none() || !delta.is_empty() || !unknown_removes.is_empty()),
        offstage,
    };
    // A new picture: its setting is a photo (people placed in it), or words.
    let Some(picture) = call.picture.clone() else {
        if let Some(photo) = setting_photo {
            // Placing nobody would send the keep sentence alone, which
            // returns the photo as it was (#408; review of #597, pass 5).
            if out.people.is_empty() {
                return Err(
                    "A room photo as the setting needs someone to place in it: give \
                     `scene.people`. To change the photo itself, name it in `picture` and use \
                     `retouch`."
                        .into(),
                );
            }
            out.render = Render::Edit {
                canvas: Canvas::Setting(photo),
                camera_moves: change.camera.is_some(),
            };
            out.instruction = "Place the people described below in this room.".into();
            out.keep = "the room and its furniture".into();
            out.route = "placed";
        }
        return Ok(out);
    };
    // A retouch: the one free-text change to the picture itself.
    if let Some(r) = call
        .retouch
        .as_ref()
        .filter(|_| delta.is_empty() && unknown_removes.is_empty())
    {
        out.render = Render::Edit {
            canvas: Canvas::Picture(picture),
            camera_moves: false,
        };
        out.faces.clear();
        out.instruction = r.clone();
        out.keep = "everything else".into();
        out.route = "retouched";
        return Ok(out);
    }
    // Nothing changed at all: draw the scene again at a new seed (§5.1, B4).
    if delta.is_empty() && unknown_removes.is_empty() {
        out.route = "redrawn";
        return Ok(match (setting_photo, base.is_some()) {
            (Some(photo), _) => Plan {
                render: Render::Edit {
                    canvas: Canvas::Setting(photo),
                    camera_moves: false,
                },
                instruction: "Place the people described below in this room.".into(),
                keep: "the room and its furniture".into(),
                ..out
            },
            (None, true) if words_setting => out,
            // No record, or one with no setting to draw from: there is no
            // scene to redraw, only the picture.
            (None, _) => Plan {
                render: Render::Edit {
                    canvas: Canvas::Picture(picture),
                    camera_moves: false,
                },
                faces: Vec::new(),
                instruction: "Draw this picture again, with the same people, clothes and room."
                    .into(),
                keep: String::new(),
                ..out
            },
        });
    }
    let restage = delta.restages() || delta.style;
    let known = out.next.people_known;
    if restage && known && (setting_photo.is_some() || words_setting) {
        out.route = "restaged";
        return Ok(match setting_photo {
            Some(photo) => Plan {
                render: Render::Edit {
                    canvas: Canvas::Setting(photo),
                    camera_moves: delta.camera,
                },
                instruction: "Place the people described below in this room.".into(),
                keep: "the room and its furniture".into(),
                ..out
            },
            // At another latent size the base seed cannot hold the room, so
            // a restage that names a size samples afresh (§5.2).
            None => Plan {
                seed: match (call.size, base_seed) {
                    (None, Some(seed)) => Seed::Base(seed),
                    _ => Seed::Fresh,
                },
                ..out
            },
        });
    }
    let no_setting = restage && known;
    // An edit of the current picture: clothes, an expression, someone added,
    // the words it renders, or a restage on a picture whose people are not
    // known yet.
    let mut lines: Vec<String> = Vec::new();
    let mut touched: Vec<String> = Vec::new();
    for p in &out.people {
        let key = p.who.key();
        let name = shown(&p.who);
        if delta.added.contains(&key) {
            let at = p.at.map(|a| format!(" {}", a.name())).unwrap_or_default();
            lines.push(format!("Add {name}{at}."));
            touched.push(key);
            continue;
        }
        if delta.dressed.contains(&key) {
            lines.push(format!(
                "Dress {name} in {}.",
                p.wearing.trim_end_matches('.')
            ));
            touched.push(key.clone());
        }
        if delta.expressed.contains(&key) {
            lines.push(format!(
                "Give {name} this expression: {}.",
                p.expression.trim_end_matches('.')
            ));
            touched.push(key.clone());
        }
        if delta.posed.contains(&key) {
            // A pose is empty in the record when none was given (the plain
            // one is said at the prompt), and a move of place alone is a pose
            // change too: never "Have Maya ." (review of #608).
            let doing = p.doing.trim().trim_end_matches('.');
            lines.push(match (doing.is_empty(), p.at) {
                (false, _) => format!("Have {name} {doing}."),
                (true, Some(at)) => format!("Place {name} {}.", at.name()),
                (true, None) => format!("Have {name} standing naturally."),
            });
            touched.push(key.clone());
        }
    }
    for key in &delta.removed {
        // Named as the record names them: a described person by their words.
        let name = base
            .and_then(|b| b.people.iter().find(|p| &p.who.key() == key))
            .map(|p| shown(&p.who))
            .unwrap_or_else(|| crate::imagegen::capitalized(key));
        lines.push(format!("Take {name} out of the picture."));
    }
    for who in &unknown_removes {
        lines.push(format!("Take {who} out of the picture."));
    }
    if delta.text && out.next.text.as_ref().is_none_or(|t| t.value.is_empty()) {
        lines.push("Take the words out of the picture.".into());
    }
    if delta.text {
        for w in out.next.text.iter().flat_map(|t| t.value.iter()) {
            let at = if w.at.is_empty() {
                String::new()
            } else {
                format!(" {}", w.at)
            };
            let look = if w.look.is_empty() {
                String::new()
            } else {
                format!(", in {}", w.look)
            };
            lines.push(format!("{RENDERED}\"{}\" appear{at}{look}.", w.words));
        }
    }
    if delta.camera {
        if let Some(c) = &out.next.camera {
            lines.push(format!("{}.", c.value.trim_end_matches('.')));
        }
    }
    if delta.setting {
        if let Some(Setting::Words { text }) = out.next.setting.as_ref().map(|f| &f.value) {
            lines.push(format!("Set the scene in {}.", text.trim_end_matches('.')));
        }
    }
    if delta.light {
        if let Some(l) = &out.next.light {
            lines.push(format!("Make the light {}.", l.value.trim_end_matches('.')));
        }
    }
    if delta.together {
        if let Some(t) = &out.next.together {
            lines.push(format!("{}.", t.value.trim_end_matches('.')));
        }
    }
    // A style on a picture whose people are not known: the whole picture
    // redrawn in it, and the style's own words follow (`imagegen`).
    if delta.style {
        if let Some(st) = &out.next.style {
            lines.push(format!(
                "Redraw the whole picture in the {} style.",
                st.value.trim_end_matches('.')
            ));
        }
    }
    // Crops go to the people this edit changes, who must look like themselves
    // after it; everyone else is on the canvas already.
    out.faces.retain(|k| touched.contains(k));
    out.render = Render::Edit {
        canvas: Canvas::Picture(picture),
        camera_moves: delta.camera,
    };
    // An edit with no line would send the keep sentence alone, the shape
    // that returns the picture unchanged (#408): refused, saying what would
    // draw (review of #597).
    if lines.is_empty() {
        return Err(
            "This change gives the picture nothing to draw: say who is in it, or put the change \
             in `retouch`."
                .into(),
        );
    }
    out.instruction = lines.join(" ");
    // What stays, named from what the change leaves alone: the keep
    // sentence must never name the axis the instruction moves (review of
    // #597), and it is load-bearing (#408: 12/12 against 8/12 without it).
    let mut keep: Vec<&str> = Vec::new();
    if delta.style {
        keep.push("the people and what they are doing");
    } else if delta.together {
        // A relation moves the people it names; only their clothes stay.
        keep.push("everyone's clothes");
    } else if touched.is_empty() && delta.removed.is_empty() && unknown_removes.is_empty() {
        keep.push("the people, their clothes and poses");
    } else {
        keep.push("everyone else as they are");
    }
    if !delta.setting && !delta.style {
        keep.push("the room");
    }
    if !delta.light && !delta.style {
        keep.push("the light");
    }
    if !delta.camera {
        keep.push("the camera");
    }
    out.keep = match keep.split_last() {
        Some((last, [])) => last.to_string(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    };
    out.route = "edited";
    if restage && (!known || no_setting) {
        let line = if no_setting {
            "The scene has no setting to redraw it from, so this was drawn as an edit of the \
             picture. Giving `scene.setting` lets later changes redraw it."
        } else {
            "The picture's people are not known yet, so this was drawn as an edit of it rather \
             than redrawn from its setting. Saying who is in it lets later changes redraw it."
        };
        out.said = Some(match out.said.take() {
            Some(said) => format!("{said} {line}"),
            None => line.to_string(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Origin;
    use serde_json::json;

    fn lib(n: &str) -> bool {
        ["maya", "john", "wren", "ivo", "tamsin", "pell"].contains(&n)
    }
    fn names(t: &str) -> Vec<String> {
        let t = t.to_lowercase();
        ["maya", "john", "wren", "ivo", "tamsin", "pell"]
            .into_iter()
            .filter(|n| t.split(|c: char| !c.is_alphanumeric()).any(|w| w == *n))
            .map(String::from)
            .collect()
    }
    fn me() -> SelfNames {
        SelfNames {
            character: Some("maya".into()),
            names: vec!["Maya".into()],
        }
    }
    fn call(v: Value) -> Call {
        parse(&v, &lib, &me(), false).unwrap()
    }
    fn planned(c: &Call, base: Option<&Scene>) -> Plan {
        plan(
            c,
            base,
            Some("h".repeat(64)),
            Origin::Clean,
            &lib,
            &names,
            &|_| None,
        )
        .unwrap()
    }
    fn landed(p: &Plan, seed: u64) -> Scene {
        let mut s = p.next.clone();
        s.seed = Some(seed);
        s
    }

    /// The retired inputs are refused with what to do instead; nothing to
    /// draw is refused; a mask needs a retouch and a picture.
    #[test]
    fn retired_inputs_and_empty_calls_are_refused_plainly() {
        for k in [
            "prompt",
            "cast",
            "extras",
            "edit",
            "negative_prompt",
            "reference_images",
            "seed",
        ] {
            let why = parse(
                &json!({k: "x", "scene": {"setting": "a beach"}}),
                &lib,
                &me(),
                false,
            )
            .unwrap_err();
            assert!(
                why.contains("Describe the picture in `scene`"),
                "{k}: {why}"
            );
        }
        assert!(parse(&json!({}), &lib, &me(), false).is_err());
        // A mask with no retouch is left out and said (G1b), never refused.
        let c = parse(
            &json!({"picture": "images/a.png", "mask": "inbox/m.png"}),
            &lib,
            &me(),
            false,
        )
        .unwrap();
        assert!(c.mask.is_none() && c.notes[0].contains("goes with a `retouch`"));
        // A retouch with nothing to change and no scene is nothing to draw.
        assert!(parse(&json!({"retouch": "a red hat"}), &lib, &me(), false).is_err());
    }

    /// `self` and the persona's names resolve to its character; a library
    /// name to that character; anything else is described.
    #[test]
    fn who_resolves_self_library_and_description() {
        let c = call(json!({"scene": {"setting": "a park", "people": [
            {"who": "self", "wearing": "a coat", "doing": "reading"},
            {"who": "John", "wearing": "a cap", "doing": "fishing"},
            {"who": "a waiter with a tray", "wearing": "a white jacket", "doing": "pouring"}]}}));
        let whos: Vec<_> = c.change.people.iter().map(|p| p.who.clone()).collect();
        assert_eq!(
            whos,
            [
                Who::Library("maya".into()),
                Who::Library("john".into()),
                Who::Described("a waiter with a tray".into())
            ]
        );
        let no_self = SelfNames::default();
        assert!(parse(
            &json!({"scene": {"people": [{"who": "self", "wearing": "a", "doing": "b"}]}}),
            &lib,
            &no_self,
            false
        )
        .is_err());
    }

    /// A new picture plans as new; with a setting photo, as people placed in
    /// it; someone new needs wearing and doing.
    #[test]
    fn a_new_picture_is_new_or_placed_on_its_photo() {
        let p = planned(
            &call(json!({"scene": {"setting": "a narrow kitchen", "people": [
                {"who": "maya", "wearing": "a coat", "doing": "cooking"}]}})),
            None,
        );
        assert_eq!(p.render, Render::New);
        assert_eq!(p.route, "new");
        assert!(p.next.people_known);
        let p = planned(
            &call(
                json!({"scene": {"setting": {"photo": "inbox/room.jpeg"}, "people": [
                {"who": "maya", "wearing": "a coat", "doing": "sitting on the bed"}]}}),
            ),
            None,
        );
        assert_eq!(
            p.render,
            Render::Edit {
                canvas: Canvas::Setting("inbox/room.jpeg".into()),
                camera_moves: false
            }
        );
        assert_eq!(p.route, "placed");
        // A library newcomer with no clothes and none on record: dressed for
        // the scene, and said (mecha-a3's G1).
        let p = plan(
            &call(json!({"scene": {"setting": "a park", "people": [{"who": "john", "doing": "jogging"}]}})),
            None,
            None,
            Origin::Clean,
            &lib,
            &names,
            &|_| None,
        )
        .unwrap();
        assert_eq!(p.next.people[0].wearing, "clothes that suit the scene");
        assert!(p.said.unwrap().contains("John had no clothes given"));
    }

    /// Review of #597, pass 6: a name the record carries is read as the
    /// prompt will carry it, so a follow-up that changes only the light still
    /// makes a recorded "John" the viewer; words to render stay as written.
    #[test]
    fn a_recorded_name_is_the_viewer_on_a_follow_up_and_text_is_kept() {
        let first = planned(
            &call(
                json!({"scene": {"setting": "a pier", "together": "Maya waves at John", "people": [
                {"who": "maya", "wearing": "a coat", "doing": "standing"}]}}),
            ),
            None,
        );
        let base = landed(&first, 4);
        let p = planned(
            &call(json!({"picture": "images/a.png", "scene": {"light": "dusk"}})),
            Some(&base),
        );
        assert_eq!(p.offstage, vec!["john".to_string()]);
        assert_eq!(
            as_viewer(
                "Maya waves at John. The words \"Happy Birthday John\" appear.",
                &p.offstage,
                &["Happy Birthday John"]
            ),
            "Maya waves at the viewer. The words \"Happy Birthday John\" appear."
        );
    }

    /// A new picture naming library characters with nobody in `people`
    /// draws them from the library, says so, and makes nobody the viewer
    /// (owner, 2026-10-09). Fails on the old planner, which drew neither.
    #[test]
    fn named_characters_with_nobody_listed_are_drawn_from_the_library() {
        let p = planned(
            &call(json!({"scene": {"setting": "a café with tall windows",
                "together": "Maya and John share a pot of tea"}})),
            None,
        );
        let keys: Vec<String> = p.next.people.iter().map(|q| q.who.key()).collect();
        assert_eq!(keys, ["maya", "john"]);
        assert!(p.offstage.is_empty(), "{:?}", p.offstage);
        let said = p.said.as_deref().unwrap_or("");
        assert!(said.contains("Maya, John named in the words"), "{said}");
        // Past what one picture draws, the rest are the viewer, never a
        // refusal to remove someone nobody listed.
        let many = planned(
            &call(json!({"scene": {"setting": "a long table",
                "together": "Maya, John, Wren, Ivo, Tamsin and Pell share a pot of tea"}})),
            None,
        );
        assert_eq!(many.next.people.len(), MAX_FACES);
        assert_eq!(many.offstage, vec!["pell".to_string()]);
        // Listing anyone keeps the 10-08 rule: a name left out is the viewer.
        let q = planned(
            &call(
                json!({"scene": {"setting": "a pier", "together": "Maya waves at John",
                "people": [{"who": "maya", "wearing": "a coat", "doing": "standing"}]}}),
            ),
            None,
        );
        assert_eq!(q.offstage, vec!["john".to_string()]);
        assert_eq!(q.next.people.len(), 1);
    }

    /// Review of #597, pass 7: a stray quote mark never switches the viewer
    /// rewrite off, and a folded solo relation reads without the name.
    #[test]
    fn a_stray_quote_keeps_the_rewrite_and_a_fold_drops_the_name() {
        let names = vec!["john".to_string()];
        assert_eq!(
            as_viewer("A 32\" screen; Maya waves at John.", &names, &[]),
            "A 32\" screen; Maya waves at the viewer."
        );
        // A rendered word that is also a name shields only itself, and
        // the same phrase quoted in prose is still rewritten.
        assert_eq!(
            as_viewer(
                "A sign reading \"John\". Maya waves at John. The words \"John\" appear.",
                &names,
                &["John", "o"]
            ),
            "A sign reading \"the viewer\". Maya waves at the viewer. The words \"John\" appear."
        );
        // The fold is the persona's only: another library name in a relation
        // is not drawn from it.
        let c = call(json!({"scene": {"setting": "a beach", "together": "John waves at the sea"}}));
        assert!(c.change.people.is_empty());
        let c = call(json!({"scene": {"setting": "a beach", "together": "Maya waves at the sea"}}));
        assert_eq!(
            c.change.people[0].doing.as_deref(),
            Some("waves at the sea")
        );
    }

    /// A room photo as the setting with nobody to place is refused, never a
    /// prompt that returns the photo unchanged (review of #597, pass 5).
    #[test]
    fn placing_nobody_on_a_photo_is_refused() {
        let why = plan(
            &call(json!({"scene": {"setting": {"photo": "inbox/room.jpeg"}, "light": "dusk"}})),
            None,
            Some("h".repeat(64)),
            Origin::Clean,
            &lib,
            &names,
            &|_| None,
        )
        .unwrap_err();
        assert!(why.contains("needs someone to place in it"), "{why}");
    }

    /// A restage that names a size draws at a fresh seed: the base seed holds
    /// the room only at the same latent size (§5.2).
    #[test]
    fn a_restage_that_names_a_size_samples_afresh() {
        let first = planned(
            &call(json!({"scene": {"setting": "a study", "people": [
                {"who": "maya", "wearing": "a coat", "doing": "reading"}]}})),
            None,
        );
        let base = landed(&first, 21);
        let on = |v: Value| planned(&call(v), Some(&base));
        let p = on(json!({"picture": "images/a.png", "scene": {"camera": "from above"}}));
        assert_eq!((p.route, p.seed), ("restaged", Seed::Base(21)));
        let p = on(
            json!({"picture": "images/a.png", "size": "landscape", "scene": {"camera": "from above"}}),
        );
        assert_eq!((p.route, p.seed), ("restaged", Seed::Fresh));
    }

    /// Review of #597, pass 3: a restage of a scene with no setting is an
    /// edit, said, never a new picture from an empty scene; the keep
    /// sentence never keeps the people a removal or a relation moves; a
    /// retouch beside a removal on a picture with no record rides along.
    #[test]
    fn a_scene_without_a_setting_restages_as_an_edit_and_keeps_honestly() {
        let first = planned(
            &call(json!({"picture": "inbox/her.jpg", "scene": {"people": [
                {"who": "maya", "wearing": "a coat", "doing": "standing"}]}})),
            None,
        );
        let base = landed(&first, 9);
        assert!(base.people_known && base.setting.is_none());
        let p = planned(
            &call(json!({"picture": "images/a.png", "scene": {"camera": "from a low angle"}})),
            Some(&base),
        );
        assert_eq!(p.route, "edited");
        assert!(matches!(p.render, Render::Edit { .. }));
        assert!(p.said.unwrap().contains("no setting to redraw it from"));
        // A removal on a picture with no record keeps everyone else.
        let p = planned(
            &call(json!({"picture": "inbox/her.jpg", "retouch": "a red hat",
                "scene": {"people": [{"who": "a man in a hat", "remove": true}]}})),
            None,
        );
        assert!(!p.keep.contains("the people, their clothes"), "{}", p.keep);
        assert_eq!(p.also.as_deref(), Some("a red hat"));
        // A relation keeps only the clothes.
        let p = planned(
            &call(
                json!({"picture": "inbox/two.jpg", "scene": {"together": "Maya and John wave at each other"}}),
            ),
            None,
        );
        assert!(
            p.keep.contains("everyone's clothes") && !p.keep.contains("poses"),
            "{}",
            p.keep
        );
    }

    /// mecha-a3's G1b and G1: `self` and the persona's own name are one
    /// person, merged; a `where` outside the four is left out and said; on a
    /// picture with no record a library name in prose is not refused, since
    /// they may be in it already.
    #[test]
    fn one_person_twice_is_merged_and_a_loose_where_is_dropped() {
        let c = call(json!({"scene": {"setting": "a cafe", "people": [
            {"who": "self", "wearing": "a coat"},
            {"who": "Maya", "doing": "reading", "where": "by the window, slightly left"}]}}));
        assert_eq!(c.change.people.len(), 1);
        let maya = &c.change.people[0];
        assert_eq!(
            (maya.wearing.as_deref(), maya.doing.as_deref()),
            (Some("a coat"), Some("reading"))
        );
        assert!(maya.at.is_none());
        assert!(
            c.notes.iter().any(|n| n.contains("`where`")),
            "{:?}",
            c.notes
        );
        let p = planned(
            &call(json!({"picture": "inbox/photo.jpg", "retouch": "give Maya a red umbrella"})),
            None,
        );
        assert_eq!(p.route, "retouched");
    }

    /// Review of #597, pass 2: copied clothes keep their origin; a removal
    /// of someone the record does not hold is said, or on a picture whose
    /// people are unknown is an edit; a removal alone does not make the
    /// people known; clearing the words is a line; an edit with no line is
    /// refused rather than sent as a keep sentence alone.
    #[test]
    fn removals_words_and_copied_clothes_are_honest() {
        let first = planned(
            &call(
                json!({"scene": {"setting": "a deck", "text": [{"words": "OPEN"}], "people": [
                {"who": "maya", "wearing": "a coat", "doing": "standing"}]}}),
            ),
            None,
        );
        let base = landed(&first, 3);
        // Clothes copied from an untrusted record stay untrusted.
        let p = plan(
            &call(json!({"picture": "images/a.png", "scene": {"people": [{"who": "john", "doing": "waving"}]}})),
            Some(&base),
            None,
            Origin::Clean,
            &lib,
            &names,
            &|_| Some(("a striped scarf".to_string(), Origin::Untrusted)),
        )
        .unwrap();
        let john = p
            .next
            .people
            .iter()
            .find(|q| q.who.key() == "john")
            .unwrap();
        assert_eq!(john.origin, Origin::Untrusted);
        // Nobody to take out, on a known picture: said.
        let why = plan(
            &call(json!({"picture": "images/a.png", "scene": {"people": [{"who": "wren", "remove": true}]}})),
            Some(&base),
            None,
            Origin::Clean,
            &lib,
            &names,
            &|_| None,
        )
        .unwrap_err();
        assert!(why.contains("Wren is not in this picture"), "{why}");
        // On a picture with no record: an edit, and the people stay unknown.
        let p = planned(
            &call(
                json!({"picture": "inbox/her.jpg", "scene": {"people": [{"who": "a man in a hat", "remove": true}]}}),
            ),
            None,
        );
        assert_eq!(p.route, "edited");
        assert!(
            p.instruction
                .contains("Take a man in a hat out of the picture."),
            "{}",
            p.instruction
        );
        assert!(!p.next.people_known);
        // Clearing the words is a line of its own.
        let p = planned(
            &call(json!({"picture": "images/a.png", "scene": {"text": []}})),
            Some(&base),
        );
        assert!(
            p.instruction.contains("Take the words out of the picture."),
            "{}",
            p.instruction
        );
    }

    /// mecha-a3's G1b: what a model over-fills is absorbed and said, never a
    /// shape refusal. A retouch beside a scene change rides along; a
    /// newcomer's pose defaults and their clothes come from the chat's
    /// record; a relation naming one person is that person; an overlong
    /// relation is clipped; a mask that is not a painted picture's path, or
    /// has no retouch, is left out; an empty `picture` is none.
    #[test]
    fn over_filled_calls_are_absorbed_not_refused() {
        let first = planned(
            &call(json!({"scene": {"setting": "a kitchen", "people": [
                {"who": "maya", "wearing": "a red coat", "doing": "sitting"}]}})),
            None,
        );
        let base = landed(&first, 5);
        // a) A retouch beside a scene change.
        let p = planned(
            &call(
                json!({"picture": "images/a.png", "retouch": "a vase of tulips",
                "scene": {"people": [{"who": "maya", "wearing": "a green dress"}]}}),
            ),
            Some(&base),
        );
        assert_eq!(p.route, "edited");
        assert_eq!(p.also.as_deref(), Some("a vase of tulips"));
        // b) A newcomer without a pose, and with clothes on the chat's record.
        let p = plan(
            &call(json!({"picture": "images/a.png", "scene": {"people": [{"who": "john"}]}})),
            Some(&base),
            None,
            Origin::Clean,
            &lib,
            &names,
            &|k| (k == "john").then(|| ("an apron".to_string(), Origin::Clean)),
        )
        .unwrap();
        let john = p
            .next
            .people
            .iter()
            .find(|q| q.who.key() == "john")
            .unwrap();
        assert_eq!(
            (john.wearing.as_str(), john.doing.as_str()),
            ("an apron", "")
        );
        assert!(p.said.unwrap().contains("what this chat last drew them in"));
        // With `together` saying what they do, nobody is given a pose of
        // their own beside it (mecha-a3: "standing naturally" beside a kneel
        // drew her twice).
        let p = planned(
            &call(json!({"scene": {"setting": "a clinic room",
                "together": "Maya ties John's shoelace",
                "people": [{"who": "maya", "wearing": "a coat"}, {"who": "john", "wearing": "a suit"}]}})),
            None,
        );
        assert!(
            p.next.people.iter().all(|q| q.doing.is_empty()),
            "{:?}",
            p.next.people
        );
        // A later call that poses one of them clears the `together`; the
        // other's pose stays empty in the record, said plainly at the prompt
        // (`imagegen::doing_words`), never stored.
        let posed = planned(
            &call(
                json!({"picture": "images/a.png", "scene": {"people": [{"who": "maya", "doing": "waving"}]}}),
            ),
            Some(&p.next),
        );
        assert!(posed.next.together.is_none());
        let john = posed
            .next
            .people
            .iter()
            .find(|q| q.who.key() == "john")
            .unwrap();
        assert_eq!(john.doing, "");
        // A move of place alone over an empty pose is said as the move,
        // never "Have Maya ." (review of #608).
        let placed = planned(
            &call(json!({"scene": {"people": [{"who": "maya", "wearing": "a coat"}]}})),
            None,
        );
        let moved = planned(
            &call(
                json!({"picture": "images/a.png", "scene": {"people": [{"who": "maya", "where": "left"}]}}),
            ),
            Some(&placed.next),
        );
        assert!(
            !moved.instruction.contains("Have Maya ."),
            "{}",
            moved.instruction
        );
        assert!(
            moved.instruction.contains("Place Maya on the left."),
            "{}",
            moved.instruction
        );
        // A described newcomer without clothes is dressed for the scene.
        let p = planned(
            &call(json!({"picture": "images/a.png", "scene": {"people": [{"who": "a waiter"}]}})),
            Some(&base),
        );
        assert!(p
            .next
            .people
            .iter()
            .any(|q| q.wearing == "clothes that suit the scene"));
        // c) A relation naming the persona alone, with nobody in `people`.
        let c =
            call(json!({"scene": {"setting": "a beach", "together": "Maya waves at the viewer"}}));
        assert_eq!(c.change.people.len(), 1);
        assert_eq!(c.change.people[0].who, Who::Library("maya".into()));
        assert!(c.change.together.is_none());
        // d) An overlong relation, clipped at a sentence.
        let long = format!(
            "Maya and John dance. {}",
            "They laugh and spin. ".repeat(30)
        );
        let c = call(
            json!({"scene": {"setting": "a hall", "together": long, "people": [
            {"who": "maya", "wearing": "a", "doing": "b"}, {"who": "john", "wearing": "a", "doing": "b"}]}}),
        );
        let t = c.change.together.unwrap();
        assert!(
            t.chars().count() <= crate::imagelib::MAX_CAST_FIELD && t.ends_with('.'),
            "{t}"
        );
        // e) A mask that is prose, and one with no retouch.
        let c = call(
            json!({"picture": "images/a.png", "retouch": "a hat", "mask": "the brass weathervane"}),
        );
        assert!(
            c.mask.is_none() && c.notes[0].contains("left out"),
            "{:?}",
            c.notes
        );
        let c = call(json!({"picture": "images/a.png", "mask": "inbox/m.png",
            "scene": {"light": "dusk"}}));
        assert!(c.mask.is_none() && !c.notes.is_empty());
        let c =
            call(json!({"picture": "images/a.png", "retouch": "a hat", "mask": {"camera": "low"}}));
        assert!(c.mask.is_none());
        // f) Empty strings are absent.
        let c = call(json!({"picture": "", "mask": "", "scene": {"setting": "a field"}}));
        assert!(c.picture.is_none() && c.mask.is_none());
    }

    /// Review of #597: someone added past the scene's bound is refused,
    /// never a panic; adding someone to a scene with a relation is an edit,
    /// not a restage; and an edit of a picture with no record names in its
    /// keep sentence only what the change leaves alone, a style included.
    #[test]
    fn edits_keep_only_what_they_leave_and_a_full_scene_refuses() {
        let two = planned(
            &call(
                json!({"scene": {"setting": "a pier", "together": "maya and john share a coat",
                "people": [
                    {"who": "maya", "wearing": "a coat", "doing": "standing"},
                    {"who": "john", "wearing": "a coat", "doing": "standing"}]}}),
            ),
            None,
        );
        let base = landed(&two, 7);
        // Ten more, described: no face budget, past the scene's bound.
        let crowd: Vec<Value> = (0..10)
            .map(|i| json!({"who": format!("a fisherman number {i}"), "wearing": "oilskins", "doing": "mending a net"}))
            .collect();
        let why = plan(
            &call(json!({"picture": "images/a.png", "scene": {"people": crowd}})),
            Some(&base),
            None,
            Origin::Clean,
            &lib,
            &names,
            &|_| None,
        )
        .unwrap_err();
        assert!(why.contains("at most 10 people"), "{why}");
        // Someone added to a scene with a relation: an edit.
        let p = planned(
            &call(json!({"picture": "images/a.png", "scene": {"people": [
                {"who": "wren", "wearing": "a scarf", "doing": "waving"}]}})),
            Some(&base),
        );
        assert_eq!(p.route, "edited");
        // No record: a light change keeps everything but the light.
        let p = planned(
            &call(json!({"picture": "inbox/her.jpg", "scene": {"light": "candlelight"}})),
            None,
        );
        assert_eq!(p.route, "edited");
        assert!(!p.keep.contains("the light"), "{}", p.keep);
        assert!(
            p.keep.contains("the camera") && p.keep.contains("the room"),
            "{}",
            p.keep
        );
        assert!(p.instruction.contains("candlelight"), "{}", p.instruction);
        // No record: a style says what to do, and keeps the people.
        let p = planned(
            &call(json!({"picture": "inbox/her.jpg", "scene": {"style": "ink wash"}})),
            None,
        );
        assert_eq!(p.route, "edited");
        assert!(
            p.instruction.contains("ink wash style"),
            "{}",
            p.instruction
        );
        assert!(
            !p.keep.contains("the light") && !p.keep.contains("the room"),
            "{}",
            p.keep
        );
    }

    /// The planner routes by what changed (§5.2): a pose or camera restages
    /// at the base seed; clothes, an expression or someone added edit the
    /// picture; restating everyone redraws.
    #[test]
    fn the_planner_routes_by_what_changed() {
        let first = planned(
            &call(json!({"scene": {"setting": "a narrow kitchen", "people": [
                {"who": "maya", "wearing": "a red coat", "doing": "sitting"},
                {"who": "john", "wearing": "an apron", "doing": "cooking"}]}})),
            None,
        );
        let base = landed(&first, 77);
        let on = |v: Value| planned(&call(v), Some(&base));
        let p = on(
            json!({"picture": "images/a.png", "scene": {"people": [{"who": "maya", "doing": "standing by the window"}]}}),
        );
        assert_eq!(
            (p.route, p.render.clone(), p.seed),
            ("restaged", Render::New, Seed::Base(77))
        );
        let p = on(json!({"picture": "images/a.png", "scene": {"camera": "from above"}}));
        assert_eq!(p.route, "restaged");
        let p = on(
            json!({"picture": "images/a.png", "scene": {"people": [{"who": "maya", "wearing": "a green dress"}]}}),
        );
        assert_eq!(p.route, "edited");
        assert_eq!(p.instruction, "Dress Maya in a green dress.");
        assert_eq!(p.faces, ["maya"], "a crop for who changed only");
        let p = on(
            json!({"picture": "images/a.png", "scene": {"people": [{"who": "maya", "expression": "smiling"}]}}),
        );
        assert_eq!(
            p.route, "edited",
            "an expression is a retouch, not a restage"
        );
        let p = on(json!({"picture": "images/a.png", "scene": {"people": [
            {"who": "wren", "where": "right", "wearing": "a scarf", "doing": "reading"}]}}));
        assert_eq!(p.route, "edited");
        assert_eq!(p.instruction, "Add Wren on the right.");
        let p = on(json!({"picture": "images/a.png", "scene": {"people": [
            {"who": "maya", "wearing": "a red coat", "doing": "sitting"},
            {"who": "john", "wearing": "an apron", "doing": "cooking"}]}}));
        assert_eq!(p.route, "redrawn");
        assert!(p.delta.is_empty());
    }

    /// A restage on a photo setting edits the photo afresh, the camera moving
    /// when the change moves it.
    #[test]
    fn a_restage_on_a_photo_setting_draws_on_the_photo() {
        let first = planned(
            &call(
                json!({"scene": {"setting": {"photo": "inbox/room.jpeg"}, "people": [
                {"who": "maya", "wearing": "a coat", "doing": "sitting"}]}}),
            ),
            None,
        );
        let base = landed(&first, 5);
        let p = planned(
            &call(json!({"picture": "images/a.png", "scene": {"camera": "from low by the door"}})),
            Some(&base),
        );
        assert_eq!(
            p.render,
            Render::Edit {
                canvas: Canvas::Setting("inbox/room.jpeg".into()),
                camera_moves: true
            }
        );
        assert_eq!(p.route, "restaged");
    }

    /// A picture with no record: a retouch is a retouch; a pose change,
    /// whose people are unknown, is an edit that says so (§5.1, N2).
    #[test]
    fn a_picture_without_a_record_is_the_current_picture() {
        let p = planned(
            &call(json!({"picture": "inbox/her.jpg", "retouch": "Give her a red umbrella."})),
            None,
        );
        assert_eq!(p.route, "retouched");
        assert_eq!(p.keep, "everything else");
        let p = planned(
            &call(json!({"picture": "inbox/her.jpg", "scene": {"camera": "from above"}})),
            None,
        );
        assert_eq!(p.route, "edited");
        assert!(p.said.unwrap().contains("not known yet"));
    }

    /// Someone is drawn only when listed in `people` (the owner's ruling,
    /// 2026-10-08): a library name in the words for someone not in the
    /// picture is not refused and not drawn; it is "the viewer" to the image
    /// model, and said. In `together` between two listed people it is as
    /// written.
    #[test]
    fn a_name_in_prose_for_someone_not_drawn_is_the_viewer() {
        let p = plan(
            &call(json!({"scene": {"setting": "a park", "people": [
                {"who": "maya", "wearing": "a coat", "doing": "holding hands with John"}]}})),
            None,
            None,
            Origin::Clean,
            &lib,
            &names,
            &|_| None,
        )
        .unwrap();
        assert_eq!(p.offstage, vec!["john".to_string()]);
        assert!(p
            .said
            .unwrap()
            .contains("John named in the words but not in the picture"));
        assert_eq!(
            as_viewer(
                "Maya holds hands with John, by John's bench.",
                &p.offstage,
                &[]
            ),
            "Maya holds hands with the viewer, by the viewer's bench."
        );
        let ok = plan(
            &call(
                json!({"scene": {"setting": "a park", "together": "Maya and John hold hands",
                "people": [
                {"who": "maya", "wearing": "a coat", "doing": "walking"},
                {"who": "john", "wearing": "a cap", "doing": "walking"}]}}),
            ),
            None,
            None,
            Origin::Clean,
            &lib,
            &names,
            &|_| None,
        )
        .unwrap();
        assert!(ok.offstage.is_empty());
    }

    /// More faces than one picture draws are refused, naming them; people
    /// drawn from words cost nothing.
    #[test]
    fn faces_are_bounded_and_described_people_are_free() {
        let p = |n: &str| json!({"who": n, "wearing": "a coat", "doing": "waving"});
        let why = plan(
            &call(json!({"scene": {"setting": "a jetty", "people": [
                p("maya"), p("john"), p("wren"), p("ivo"), p("tamsin"), p("pell")]}})),
            None,
            None,
            Origin::Clean,
            &lib,
            &names,
            &|_| None,
        )
        .unwrap_err();
        assert!(why.contains("6 people with faces"), "{why}");
        let ok = planned(
            &call(json!({"scene": {"setting": "a jetty", "people": [
                p("maya"), p("john"), p("wren"), p("ivo"), p("tamsin"),
                p("a fisherman in oilskins")]}})),
            None,
        );
        assert_eq!(ok.faces.len(), 5);
    }

    /// `where` orders the people left to right, the background last.
    #[test]
    fn where_orders_the_people() {
        let p = planned(
            &call(json!({"scene": {"setting": "a hall", "people": [
                {"who": "john", "where": "right", "wearing": "a", "doing": "b"},
                {"who": "wren", "where": "background", "wearing": "a", "doing": "b"},
                {"who": "maya", "where": "left", "wearing": "a", "doing": "b"}]}})),
            None,
        );
        let order: Vec<_> = p.people.iter().map(|x| x.who.key()).collect();
        assert_eq!(order, ["maya", "john", "wren"]);
    }
}
