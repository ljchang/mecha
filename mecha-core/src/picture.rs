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
pub const MAX_FACES: usize = 5;

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
    let mask = text(obj.get("mask"), "mask", 200)?;
    let size = match text(obj.get("size"), "size", 20)? {
        None => None,
        Some(s) => Some(
            crate::imagegen::Size::parse(&s).ok_or("`size` is square, landscape or portrait.")?,
        ),
    };
    if mask.is_some() && retouch.is_none() {
        return Err("A painted area (`mask`) goes with a `retouch`: what to change there.".into());
    }
    if mask.is_some() && picture.is_none() {
        return Err("A painted area (`mask`) needs the `picture` it was painted on.".into());
    }
    let seed = match obj.get("seed") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().ok_or("`seed` is a whole number.")?),
    };
    if seed.is_some() && picture.is_some() {
        return Err("A `seed` draws a new picture; a change to one samples afresh.".into());
    }
    if retouch.is_some() && picture.is_none() {
        return Err("A `retouch` changes a picture: name it in `picture`.".into());
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
            change.style = text(sc.get("style"), "scene.style", crate::imagelib::MAX_NAME)?;
            change.together = text(sc.get("together"), "scene.together", field)?;
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
                        let at = match text(v.get("where"), "scene.people.where", 20)? {
                            None => None,
                            Some(w) => Some(
                                Where::parse(&w)
                                    .ok_or("`where` is left, centre, right or background.")?,
                            ),
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
    // One person once.
    for (i, p) in change.people.iter().enumerate() {
        if change.people[..i]
            .iter()
            .any(|q| q.who.key() == p.who.key())
        {
            return Err(format!(
                "{} appears twice in `scene.people`; each person is drawn once.",
                shown(&p.who)
            ));
        }
    }
    let has_scene = change != SceneChange::default() || setting_photo.is_some();
    if picture.is_none() && !has_scene {
        return Err(
            "Nothing to draw: give a `scene` for a new picture, or the `picture` to change.".into(),
        );
    }
    Ok(Call {
        picture,
        change,
        setting_photo,
        retouch,
        mask,
        size,
        seed,
    })
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
}

/// The left-to-right rank of a place in the frame; the background last.
fn rank(at: Option<Where>) -> u8 {
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
) -> Result<Plan, String> {
    let mut change = call.change.clone();
    if let (Some(path), Some(hash)) = (&call.setting_photo, setting_photo_hash) {
        change.setting = Some(Setting::Photo {
            path: path.clone(),
            hash,
        });
    }
    let new_picture = call.picture.is_none();
    // A new picture defines its scene; it knows who it drew.
    let (next, delta) = Scene::apply(base, &change, by, new_picture);
    // Someone the call introduces needs their clothes and what they do.
    for key in &delta.added {
        let p = next.people.iter().find(|p| &p.who.key() == key).unwrap();
        if p.wearing.trim().is_empty() || p.doing.trim().is_empty() {
            return Err(format!(
                "{} is new to this picture: give what they wear and what they are doing.",
                shown(&p.who)
            ));
        }
    }
    // A library name in prose is someone in the picture, or a stranger drawn
    // from words (E1): refused plainly unless they are in it.
    let in_picture: Vec<String> = next.people.iter().map(|p| p.who.key()).collect();
    let mut prose: Vec<(&str, String)> = Vec::new();
    for (k, v) in [
        ("light", &change.light),
        ("camera", &change.camera),
        ("together", &change.together),
        ("retouch", &call.retouch),
    ] {
        if let Some(v) = v {
            prose.push((k, v.clone()));
        }
    }
    if let Some(Setting::Words { text }) = &change.setting {
        prose.push(("setting", text.clone()));
    }
    for p in &change.people {
        for (k, v) in [
            ("wearing", &p.wearing),
            ("doing", &p.doing),
            ("expression", &p.expression),
        ] {
            if let Some(v) = v {
                prose.push((k, v.clone()));
            }
        }
        if let Who::Described(d) = &p.who {
            prose.push(("who", d.clone()));
        }
    }
    for (field, text) in &prose {
        for name in named_in(text) {
            if approved(&name) && !in_picture.contains(&name) {
                return Err(format!(
                    "{} is named in `{field}` but is not in the picture: add them to \
                     `scene.people` with what they wear and do.",
                    crate::imagegen::capitalized(&name)
                ));
            }
        }
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
    let setting_photo = match next.setting.as_ref().map(|f| &f.value) {
        Some(Setting::Photo { path, .. }) => Some(path.clone()),
        _ => None,
    };
    let keep_all = "the people, their clothes and poses, the room, the light and the camera";
    let mut out = Plan {
        render: Render::New,
        people: people.clone(),
        faces: faces_all.clone(),
        instruction: String::new(),
        keep: String::new(),
        seed: call.seed.map_or(Seed::Fresh, Seed::Given),
        size: call.size,
        mask: call.mask.clone(),
        next,
        delta: delta.clone(),
        route: "new",
        said: None,
    };
    // A new picture: its setting is a photo (people placed in it), or words.
    let Some(picture) = call.picture.clone() else {
        if let Some(photo) = setting_photo {
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
    if let Some(r) = &call.retouch {
        if !delta.is_empty() {
            return Err(
                "A `retouch` is one small change on its own; change the scene in a separate call."
                    .into(),
            );
        }
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
    if delta.is_empty() {
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
            (None, true) => out,
            // No record: there is no scene to redraw, only the picture.
            (None, false) => Plan {
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
    if restage && known {
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
            None => Plan {
                seed: base_seed.map_or(Seed::Fresh, Seed::Base),
                ..out
            },
        });
    }
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
            lines.push(format!("Have {name} {}.", p.doing.trim_end_matches('.')));
            touched.push(key.clone());
        }
    }
    for key in &delta.removed {
        lines.push(format!(
            "Take {} out of the picture.",
            crate::imagegen::capitalized(key)
        ));
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
            lines.push(format!("The words \"{}\" appear{at}{look}.", w.words));
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
    // Crops go to the people this edit changes, who must look like themselves
    // after it; everyone else is on the canvas already.
    out.faces.retain(|k| touched.contains(k));
    out.render = Render::Edit {
        canvas: Canvas::Picture(picture),
        camera_moves: delta.camera,
    };
    out.instruction = lines.join(" ");
    out.keep = if touched.is_empty() {
        keep_all.into()
    } else {
        "the room, the light and everyone else as they are".into()
    };
    out.route = "edited";
    if restage && !known {
        out.said = Some(
            "The picture's people are not known yet, so this was drawn as an edit of it rather \
             than redrawn from its setting. Saying who is in it lets later changes redraw it."
                .into(),
        );
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
        plan(c, base, Some("h".repeat(64)), Origin::Clean, &lib, &names).unwrap()
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
        assert!(parse(
            &json!({"picture": "images/a.png", "mask": "inbox/m.png"}),
            &lib,
            &me(),
            false
        )
        .unwrap_err()
        .contains("goes with a `retouch`"));
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
        let why = plan(
            &call(json!({"scene": {"setting": "a park", "people": [{"who": "john", "wearing": "a cap"}]}})),
            None,
            None,
            Origin::Clean,
            &lib,
            &names,
        )
        .unwrap_err();
        assert!(why.contains("John is new to this picture"), "{why}");
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

    /// A library name in prose is someone in the picture, or refused; in
    /// `together` it is expected.
    #[test]
    fn a_name_in_prose_must_be_someone_in_the_picture() {
        let why = plan(
            &call(json!({"scene": {"setting": "a park", "people": [
                {"who": "maya", "wearing": "a coat", "doing": "holding hands with John"}]}})),
            None,
            None,
            Origin::Clean,
            &lib,
            &names,
        )
        .unwrap_err();
        assert!(why.contains("John is named in `doing`"), "{why}");
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
        );
        assert!(ok.is_ok(), "{ok:?}");
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
