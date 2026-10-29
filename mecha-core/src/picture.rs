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
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SelfNames {
    /// The persona's approved library character, if it has one.
    pub character: Option<String>,
    /// Every name the persona answers to: its display name and folder.
    pub names: Vec<String>,
}

/// The colours a region may be painted in, by name and exact pixel value.
/// The page picks four, those least present in the picture, and paints them
/// unsmoothed so every pixel of the index is one of these or black
/// (IMAGE-REGION-EDIT-RESEARCH.md §7.4).
pub const REGION_COLOURS: [(&str, [u8; 3]); 8] = [
    ("magenta", [255, 0, 255]),
    ("cyan", [0, 255, 255]),
    ("blue", [0, 64, 255]),
    ("green", [0, 200, 0]),
    ("orange", [255, 128, 0]),
    ("yellow", [255, 230, 0]),
    ("red", [230, 0, 0]),
    ("purple", [128, 0, 255]),
];

/// The most regions one edit carries.
pub const MAX_REGIONS: usize = 4;

/// One painted region: its colour, what changes inside it, and a picture the
/// owner dropped on it to take that change from (a face, most often).
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    pub colour: String,
    pub rgb: [u8; 3],
    pub words: String,
    /// A picture in this chat (`inbox/...` or `images/...`), read through the
    /// path jail like the picture and the mask.
    pub reference: Option<String>,
}

impl Call {
    /// Whether this call may load SAM inside its job, and so is charged for
    /// it at admission: words in `mask` (S1), or a painted region whose
    /// words are about a face, the only regions that take one and so are
    /// edged by segmentation (S2). A region edit with no face never is
    /// (review of #710).
    pub fn may_segment(&self) -> bool {
        self.mask_words.is_some() || self.regions.iter().any(|r| about_a_face(&r.words))
    }
}

/// An outfit as the image model is given it: one that opens with nudity
/// ("nude", "naked", "nothing") said outright as "nothing at all, completely
/// naked". A portrait's own clothes fill whatever the words leave open: on
/// the replay rig on 2026-10-28, "Nude" beside a clothed portrait drew the
/// portrait's outfit at 3 of 3 seeds and the spelled-out form 0 of 3.
pub fn outfit_words(wearing: &str) -> String {
    let w = wearing.trim().trim_end_matches('.');
    let (first, rest) = match w.find([',', ';']) {
        Some(i) => (&w[..i], &w[i..]),
        None => (w, ""),
    };
    let nude = matches!(
        first.trim().to_lowercase().as_str(),
        "nude"
            | "naked"
            | "nothing"
            | "nothing at all"
            | "no clothes"
            | "completely nude"
            | "completely naked"
            | "fully nude"
            | "fully naked"
            | "totally naked"
    );
    if nude {
        format!("nothing at all, completely naked{rest}")
    } else {
        w.to_string()
    }
}

/// Garments that cover the chest, and garments for the lower body only, as
/// whole words: what [`removed_top`] reads.
const UPPER: &[&str] = &[
    "top",
    "tops",
    "shirt",
    "t-shirt",
    "tee",
    "blouse",
    "bra",
    "bralette",
    "tank",
    "camisole",
    "cami",
    "sweater",
    "jumper",
    "hoodie",
    "sweatshirt",
    "cardigan",
    "jacket",
    "blazer",
    "coat",
    "vest",
    "corset",
    "bustier",
    "bodysuit",
    "leotard",
    "swimsuit",
    "one-piece",
    "dress",
    "gown",
    "robe",
    "kimono",
    "jumpsuit",
    "romper",
    "overalls",
    "pyjamas",
    "pajamas",
    "nightie",
    "nightgown",
    "bikini",
    "halter",
    "tube",
    "towel",
    "apron",
    "turtleneck",
    "polo",
    "tunic",
    "lingerie",
    "teddy",
    "babydoll",
    "kaftan",
    "poncho",
    "parka",
    "shawl",
];
const LOWER: &[&str] = &[
    "shorts",
    "pants",
    "trousers",
    "jeans",
    "leggings",
    "skirt",
    "panties",
    "underwear",
    "briefs",
    "boxers",
    "thong",
    "knickers",
    "bottoms",
    "joggers",
    "sweatpants",
    "hotpants",
    "culottes",
    "chinos",
    "slacks",
    "g-string",
];

/// Whether an outfit names a garment from `set`, as a whole word: hyphens
/// kept, so "t-shirt" is a shirt and "half-top" is not "top".
fn names_any(wearing: &str, set: &[&str]) -> bool {
    wearing
        .to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .any(|w| set.contains(&w))
}

/// Whether a change to someone's outfit takes their top off: the old outfit
/// covered the chest, the new one names a lower garment and nothing for the
/// chest. Said outright then (", no top"), since a portrait's top fills an
/// unsaid chest: on the replay rig on 2026-10-28, shorts alone after a shirt
/// came off drew the portrait's top at 3 of 3 seeds, and with "no top" 0 of
/// 3. Only a removal is read so: an outfit first given as "jeans and boots"
/// says nothing either way, and is left to the persona's words.
pub fn removed_top(before: &str, after: &str) -> bool {
    names_any(before, UPPER)
        && !names_any(after, UPPER)
        && names_any(after, LOWER)
        && !after.to_lowercase().contains("topless")
}

/// Whether a painted region's words are about a face: the one place a
/// library name in them takes that character's face, and the only words a
/// dropped picture gives its face to rather than its look (§5.3).
pub fn about_a_face(words: &str) -> bool {
    words
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| matches!(w.to_lowercase().as_str(), "face" | "faces"))
}

/// The legend the model reads for regions, in the measured M2 form: the
/// outlines are in `<image2>`, the picture is `<image1>`, and each colour's
/// words follow it (mecha-a3, 2026-10-09: 0 of 24 wrong targets).
pub fn region_legend(regions: &[Region]) -> String {
    let each: Vec<String> = regions
        .iter()
        .map(|r| {
            format!(
                "in the {} outline, {}",
                r.colour,
                r.words.trim().trim_end_matches('.')
            )
        })
        .collect();
    format!(
        "The coloured outlines in <image2> mark where each change goes in <image1>: {}. Do \
         not draw the outlines.",
        each.join("; ")
    )
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
    /// The area to change, named in words rather than painted ("the blue
    /// scarf"): found by segmentation inside the picture job, and masked as
    /// a painted area would be (`docs/SEGMENTATION-DESIGN.md` S1). Never
    /// beside `mask` or `regions`.
    pub mask_words: Option<String>,
    /// Coloured regions painted in `mask`, each with its own words (the
    /// owner's edit panel; IMAGE-REGION-EDIT-RESEARCH.md §7, M2). With
    /// them, `mask` is a region index and `retouch` is their legend.
    pub regions: Vec<Region>,
    pub size: Option<crate::imagegen::Size>,
    /// A new picture's seed: only where `seeds` admits one (the CLI and
    /// evals, never a chat; IMAGE-DESIGN.md §5.1).
    pub seed: Option<u64>,
    /// What the call over-filled and was left out, said in the result
    /// rather than refused (mecha-a3's G1b: optional fields a model fills
    /// with the wrong thing are its most common shape refusal).
    pub notes: Vec<String>,
    /// Who "self" and the persona's names are, so an older record's people
    /// are read by the same rule as the call's own (`migrated`).
    pub me: SelfNames,
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
    let mut retouch = text(obj.get("retouch"), "retouch", prose)?;
    let mut notes: Vec<String> = Vec::new();
    // A mask is a painted picture's path, from the owner's message. Anything
    // else in it (prose, a name, an object) is left out and said, never a
    // refusal (G1b: 3 of 65).
    // Words and paths are told apart by shape (`looks_like_a_picture_path`),
    // and a path-shaped one stays a path by design: one that does not
    // resolve is left out and said, never read as words
    // (`docs/SEGMENTATION-DESIGN.md` S1).
    let mut mask_words: Option<String> = None;
    let mut mask = match obj.get("mask") {
        None | Some(Value::Null) => None,
        Some(Value::String(t)) if t.trim().is_empty() => None,
        Some(Value::String(t)) if looks_like_a_picture_path(t) => Some(t.trim().to_string()),
        Some(Value::String(t)) => {
            if t.trim().chars().count() > crate::imagegen::MASK_WORDS_CAP {
                notes.push(format!(
                    "The `mask` words were past {} characters, so they were left out.",
                    crate::imagegen::MASK_WORDS_CAP
                ));
            } else {
                mask_words = Some(t.trim().to_string());
            }
            None
        }
        Some(_) => {
            notes.push(
                "The `mask` given was neither a painted picture's path nor words, so it was left \
                 out."
                    .into(),
            );
            None
        }
    };
    // Regions are the edit panel's, never a model's: refused plainly when
    // malformed, since nobody is left on that path to be told a part was
    // dropped.
    let mut regions: Vec<Region> = Vec::new();
    match obj.get("regions") {
        None | Some(Value::Null) => {}
        Some(Value::Array(list)) => {
            if list.len() > MAX_REGIONS {
                return Err(format!("An edit carries at most {MAX_REGIONS} regions."));
            }
            for r in list {
                let colour = r["colour"].as_str().map(str::trim).unwrap_or_default();
                let Some((name, rgb)) = REGION_COLOURS.iter().find(|(n, _)| *n == colour) else {
                    return Err("A region's `colour` is not one of the region colours.".into());
                };
                if regions.iter().any(|x| x.colour == *name) {
                    return Err(format!("Two regions are painted {name}."));
                }
                let words = r["words"].as_str().map(str::trim).unwrap_or_default();
                if words.is_empty() || crate::imagelib::blank(words) {
                    return Err(format!("The {name} region says nothing to change."));
                }
                if words.chars().count() > field {
                    return Err(format!(
                        "The {name} region's words are past {field} characters."
                    ));
                }
                let reference = match r.get("reference") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(t)) if looks_like_a_picture_path(t) => {
                        Some(t.trim().to_string())
                    }
                    Some(_) => {
                        return Err(format!(
                            "The {name} region's `reference` is not a picture's path."
                        ))
                    }
                };
                regions.push(Region {
                    colour: name.to_string(),
                    rgb: *rgb,
                    words: words.to_string(),
                    reference,
                });
            }
        }
        Some(_) => return Err("`regions` is a list of {colour, words, reference?}.".into()),
    }
    if !regions.is_empty() {
        // Before the generic check: words in `mask` leave `mask` empty, and
        // are refused by name (review of #710, pass 3).
        if mask_words.is_some() {
            return Err(
                "Painted regions carry their own mask; a `mask` in words cannot go beside them."
                    .into(),
            );
        }
        if mask.is_none() || picture.is_none() {
            return Err(
                "Regions go with the `mask` they were painted in and the `picture` it was \
                 painted on."
                    .into(),
            );
        }
        if retouch.is_some() {
            return Err("Regions carry their own words; no `retouch` goes beside them.".into());
        }
        retouch = Some(region_legend(&regions));
    }
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
    if mask_words.is_some() && (retouch.is_none() || picture.is_none()) {
        notes.push(
            "The `mask` was left out: an area named in words goes with a `retouch` of the \
             `picture` it is in."
                .into(),
        );
        mask_words = None;
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
                            who: resolve(&who, approved, me, &mut notes)?,
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
        mask_words,
        regions,
        size,
        seed,
        notes,
        me: me.clone(),
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

/// A painted picture's workspace path: anything ending in a picture's
/// extension. A bare file name is one, so `mask.png` is left out and said as
/// a path the chat does not hold (review of #710, pass 2), and so is a path
/// with a space in it, which words naming an area never end like (pass 5).
fn looks_like_a_picture_path(t: &str) -> bool {
    let t = t.trim();
    let lower = t.to_lowercase();
    t.chars().count() <= 200
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
///
/// A name followed by a description ("Maya, a tall woman with…") is
/// that character: the library supplies how they look, so the words after
/// the name are dropped. Read as a stranger, the name lost its portrait, the
/// reader's clothes and pose for it matched no one, and the name inside the
/// description reached the image model as "the viewer" (owner, 2026-10-27:
/// a persona chat drew someone else, clothed, standing). Only a separator ends
/// the name, so "John's sister" or "Maya and John" stay descriptions. The
/// words left out are said in `notes`, never dropped silently: they may be
/// an act that belongs in `doing`.
fn resolve(
    who: &str,
    approved: &dyn Fn(&str) -> bool,
    me: &SelfNames,
    notes: &mut Vec<String>,
) -> Result<Who, String> {
    if let Some(named) = resolve_name(who, approved, me) {
        return named;
    }
    if let Some((head, said_more)) = leading_name(who) {
        // Only a resolved name, never the no-character refusal: with words
        // after the name there is something to draw from, and an over-filled
        // `who` is absorbed, not refused (review of #756).
        if let Some(Ok(named)) = resolve_name(head, approved, me) {
            if said_more {
                notes.push(format!(
                    "`who` \"{}\" was read as {}: the library says how they look, so the \
                     words after the name were left out. Put an act in `doing` and clothes in \
                     `wearing`.",
                    who.trim(),
                    head.trim()
                ));
            }
            return Ok(named);
        }
    }
    Ok(Who::Described(who.trim().to_string()))
}

/// The words before the first separator (`,` `(` `:` `—` `–` `;` ` - `) of a
/// `who` that has one, and whether anything worth saying follows ("Maya,"
/// leaves nothing out). `None` when there is no separator.
fn leading_name(who: &str) -> Option<(&str, bool)> {
    let head = who
        .split([',', '(', ':', '—', '–', ';'])
        .next()
        .and_then(|h| h.split(" - ").next())
        .unwrap_or_default();
    if head.trim().is_empty() || head.trim().len() >= who.trim().len() {
        return None;
    }
    // "Maya (self)" says nothing more than "Maya": no note for it.
    let rest: String = who[head.len()..]
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    let said_more = !rest.is_empty() && !rest.eq_ignore_ascii_case("self");
    Some((head, said_more))
}

/// A record written before a name leading a description was read as that
/// character (#756) holds it as `Described("maya, a potter …")`. Where this
/// change names `Library("maya")` and the record holds no Maya by name, that
/// described person is Maya: rewritten here, so an edit does not add her a
/// second time beside herself, nor a `remove` say she is not there.
fn migrated(
    base: &Scene,
    change: &SceneChange,
    approved: &dyn Fn(&str) -> bool,
    me: &SelfNames,
) -> Option<Scene> {
    let named: Vec<String> = change
        .people
        .iter()
        .filter(|c| matches!(c.who, Who::Library(_)))
        .map(|c| c.who.key())
        .filter(|k| !base.people.iter().any(|p| p.who.key() == *k))
        .collect();
    let mut out = base.clone();
    let mut taken: Vec<String> = Vec::new();
    for p in &mut out.people {
        let Who::Described(d) = &p.who else { continue };
        let Some((head, _)) = leading_name(d) else {
            continue;
        };
        // Read by `resolve`'s own rule, so "self, …" and the persona's names
        // reach its character even where its key is not its name (review of
        // #756, pass 4).
        let Some(Ok(Who::Library(key))) = resolve_name(head, approved, me) else {
            continue;
        };
        // The first such person only: two described people led by one name
        // would otherwise become one character twice (review of #756, pass 3).
        if named.contains(&key) && !taken.contains(&key) {
            p.who = Who::Library(key.clone());
            taken.push(key);
        }
    }
    (!taken.is_empty()).then_some(out)
}

/// A `who` that is wholly a name: "self", one of the persona's names, or an
/// approved library character. `None` for anything else.
fn resolve_name(
    who: &str,
    approved: &dyn Fn(&str) -> bool,
    me: &SelfNames,
) -> Option<Result<Who, String>> {
    let key = who.trim().to_lowercase();
    let is_me = key == "self"
        || me.names.iter().any(|n| n.trim().to_lowercase() == key)
        || me
            .character
            .as_deref()
            .is_some_and(|c| c.trim().to_lowercase() == key);
    if is_me {
        return Some(
            me.character
                .as_deref()
                .map(|c| Who::Library(c.trim().to_lowercase()))
                .ok_or_else(|| {
                    "This persona has no approved library character, so there is no \"self\" to \
                     draw; describe the persona in `who` instead."
                        .to_string()
                }),
        );
    }
    approved(&key).then_some(Ok(Who::Library(key)))
}

/// A camera's words, lowercased, split on anything not a letter or an
/// apostrophe: what [`faces_away`] and [`is_framing`] read.
fn camera_words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphabetic() || c == '\''))
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// Whether the camera owns "behind" or "rear" in `text`, rather than
/// scenery or light: "behind" opening the camera's words, or after "from",
/// "is", "sits", "seen", "shot", "camera" or "lens"; a rear view, shot,
/// angle or quarter; "from the rear". Never "pots behind her" (scenery),
/// "lit from behind" (light), or a later clause that opens with "behind",
/// which is scenery's shape too ("wide on her, behind her the kiln";
/// review of #757). Read per clause, so a comma starts one.
fn camera_behind(text: &str) -> bool {
    const OWNS: &[&str] = &[
        "from", "sits", "seen", "shot", "camera", "lens", "viewed", "filmed",
    ];
    const LIGHT: &[&str] = &[
        "lit", "backlit", "light", "lighting", "glow", "glowing", "sun", "sunlight",
    ];
    text.split([',', ';', ':', '.'])
        .enumerate()
        .any(|(k, clause)| {
            let w = camera_words(clause);
            let behind = w.iter().enumerate().any(|(i, t)| {
                t == "behind"
                    && ((i == 0 && k == 0)
                        || (i > 0 && OWNS.contains(&w[i - 1].as_str()))
                        // "is behind" only with the camera as its subject:
                        // "the kiln is behind her" is scenery (review of
                        // #757, pass 5).
                        || (i >= 2
                            && w[i - 1] == "is"
                            && ["camera", "lens", "shot", "view", "viewer", "framing"]
                                .contains(&w[i - 2].as_str())))
                    && !(i >= 2 && w[i - 1] == "from" && LIGHT.contains(&w[i - 2].as_str()))
            });
            let rear = w.iter().enumerate().any(|(i, t)| {
                t == "rear"
                    && (w.get(i + 1).is_some_and(|n| {
                        ["view", "shot", "angle", "quarter"].contains(&n.as_str())
                    }) || (i >= 2 && w[i - 1] == "the" && w[i - 2] == "from"))
            });
            behind || rear
        })
}

/// Whether `w` holds `seq` as consecutive words.
fn has_run(w: &[String], seq: &[&str]) -> bool {
    w.windows(seq.len())
        .any(|win| win.iter().zip(seq).all(|(a, b)| a == b))
}

/// Whether a camera sees its people from behind, so their faces are turned
/// away (`imagelib::CastMember::face_away`): "behind" or "rear", a back
/// view, the back toward the camera, or a face turned away. A front, an
/// over-the-shoulder shot, or a face toward the camera or the viewer reads
/// as not, since a face is then meant to show.
pub fn faces_away(camera: &str) -> bool {
    let w = camera_words(camera);
    let has = |x: &str| w.iter().any(|t| t == x);
    let turned = |i: usize| {
        w[i.saturating_sub(2)..(i + 3).min(w.len())]
            .iter()
            .any(|t| t == "away")
    };
    let away = camera_behind(camera)
        || has_run(&w, &["back", "view"])
        // "From the back" ending its clause: never "the back wall" or "the
        // back of the shed", which are places (review of #757, pass 7).
        || camera
            .split([',', ';', ':', '.'])
            .any(|clause| camera_words(clause).ends_with(&["from".into(), "the".into(), "back".into()]))
        // Someone's back turned to the camera: a possessive first, so "pulled
        // back to show us the bench" is distance, not a back (review of #757,
        // passes 1 and 7).
        || w.iter().enumerate().any(|(i, t)| {
            t == "back"
                && i > 0
                && ["her", "his", "their", "its"].contains(&w[i - 1].as_str())
                && w
                    .get(i + 1)
                    .is_some_and(|n| ["to", "toward", "towards", "is", "turned"].contains(&n.as_str()))
                && w[i + 1..(i + 5).min(w.len())]
                    .iter()
                    .any(|x| ["camera", "lens", "viewer", "us"].contains(&x.as_str()))
        })
        || w.iter()
            .enumerate()
            .any(|(i, t)| ["face", "faces", "facing", "turned"].contains(&t.as_str()) && turned(i));
    // A face meant to show: the front, a shoulder looked over, or a face,
    // a look or eyes toward the camera, the viewer or us, unless turned away.
    // "front" as the camera's side, never the positional "in front of"
    // ("the wheel in front of her hands" places a wheel; review of #757,
    // pass 6).
    let facing = w
        .iter()
        .enumerate()
        .any(|(i, t)| t == "front" && !(i > 0 && w[i - 1] == "in"))
        || has("frontal")
        || w.iter().enumerate().any(|(i, t)| {
            t == "over"
                && w[i + 1..(i + 3).min(w.len())]
                    .iter()
                    .any(|x| x == "shoulder")
        })
        || w.iter().enumerate().any(|(i, t)| {
            ["face", "faces", "facing", "looking", "looks", "eyes"].contains(&t.as_str())
                && !turned(i)
                && w[i + 1..(i + 4).min(w.len())]
                    .iter()
                    .any(|x| ["camera", "viewer", "us", "lens"].contains(&x.as_str()))
        });
    away && !facing
}

/// Whose own eyes a camera is, when it is someone's: a first-person camera
/// ("first person", "first-person", "through her own eyes", "through John's
/// eyes") and the one library name in it that is its holder, by a
/// possessive ("from John's first-person view") or as the only name the
/// camera holds ("first person, John at the stove"). `names` are the
/// library keys named in `camera`. A point of view alone ("a wide shot
/// from John's POV") is not first person: it is how a persona says where a
/// wide shot stands, with the holder in it.
pub fn first_person_holder(camera: &str, names: &[String]) -> Option<String> {
    let w = camera_words(camera);
    let through_eyes = w
        .iter()
        .enumerate()
        .any(|(i, t)| t == "through" && w[i + 1..(i + 4).min(w.len())].iter().any(|x| x == "eyes"));
    if !(has_run(&w, &["first", "person"]) || has_run(&w, &["own", "eyes"]) || through_eyes) {
        return None;
    }
    let possessive = names
        .iter()
        .find(|n| w.iter().any(|t| *t == format!("{n}'s")));
    match (possessive, names) {
        (Some(n), _) => Some(n.clone()),
        (None, [only]) => Some(only.clone()),
        _ => None,
    }
}

/// Whether words name a shot rather than a look: framing, distance or
/// angle. A reader that filed a framing word as a `look` had it joined to
/// the setting, where it rode every later picture (owner, 2026-10-27); such
/// words are the camera's.
pub fn is_framing(words: &str) -> bool {
    let w = camera_words(words);
    let has = |x: &str| w.iter().any(|t| t == x);
    const PAIRS: &[[&str; 2]] = &[
        ["close", "up"],
        ["full", "body"],
        ["whole", "body"],
        ["wide", "shot"],
        ["medium", "shot"],
        ["long", "shot"],
        ["low", "angle"],
        ["high", "angle"],
        ["camera", "angle"],
        ["from", "above"],
        ["from", "below"],
        ["bird's", "eye"],
        ["eye", "level"],
        ["back", "view"],
    ];
    has("closeup")
        || has("zoom")
        || has("overhead")
        || camera_behind(words)
        || has("shoulder")
        || PAIRS.iter().any(|p| has_run(&w, p))
        || has_run(&w, &["point", "of", "view"])
}

/// What a person wears when nobody said and nothing is on record: drawn
/// as the scene suits. A judge reads it as no claim about the picture
/// (`persona::replay_judge::clauses`), since every outfit suits it.
pub const WEARING_UNSAID: &str = "clothes that suit the scene";

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
    /// An area named in words, segmented in the job (`Call::mask_words`).
    pub mask_words: Option<String>,
    /// The scene this render lands as.
    pub next: Scene,
    pub delta: Delta,
    /// What the plan was, for the record and the result line: "new",
    /// "placed", "restaged", "edited", "retouched", "redrawn".
    pub route: &'static str,
    /// Said in the result: e.g. why a restage became an edit.
    pub said: Option<String>,
    /// Where a newcomer's clothes came from, by the name the result uses:
    /// said after `said` unless the prompt left that person's outfit out
    /// (`imagegen::clothes_in_act`), when the sentence would be false
    /// (review of #647, pass 4).
    pub clothes_said: Vec<(String, String)>,
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

/// A new picture that lists nobody in `people` but names library
/// characters in its words draws them, from the library (owner,
/// 2026-10-09): a chat asked for two of them at a café, listed neither, and
/// two strangers were drawn under "the viewer". Where `people` lists anyone,
/// a name left out of it stays the viewer (owner, 2026-10-08).
///
/// Adds them to `change.people` when it is empty, at most [`MAX_FACES`] (the
/// rest stay the viewer, never a refusal to remove someone nobody listed),
/// and returns the note the result says. Called by `image_generate` before
/// the scene reader, so what the owner said they wear reaches them (review
/// of #613), and by [`plan`] for any caller that did not.
pub fn draw_named(
    change: &mut SceneChange,
    retouch: Option<&str>,
    approved: &dyn Fn(&str) -> bool,
    named_in: &dyn Fn(&str) -> Vec<String>,
) -> Option<String> {
    if !change.people.is_empty() {
        return None;
    }
    let setting = match &change.setting {
        Some(Setting::Words { text }) => Some(text.as_str()),
        _ => None,
    };
    let words = [
        change.together.as_deref(),
        setting,
        change.light.as_deref(),
        change.camera.as_deref(),
        retouch,
    ];
    let mut from_words: Vec<String> = Vec::new();
    for text in words.into_iter().flatten() {
        for name in named_in(text) {
            if approved(&name) && !from_words.contains(&name) {
                from_words.push(name);
            }
        }
    }
    from_words.truncate(MAX_FACES);
    if from_words.is_empty() {
        return None;
    }
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
    let named: Vec<String> = from_words
        .iter()
        .map(|k| crate::imagegen::capitalized(k))
        .collect();
    Some(format!(
        "{} named in the words with nobody in `scene.people`, so drawn from the library. \
         List people in `scene.people` to say what each wears and does.",
        named.join(", ")
    ))
}

/// `d` without the scene fields a first naming call records but does not
/// draw: setting, light, camera and relation.
fn undrawn_scene(d: &crate::scene::Delta) -> crate::scene::Delta {
    let mut d = d.clone();
    d.setting = false;
    d.light = false;
    d.camera = false;
    d.together = false;
    d.together_cleared = false;
    d
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
    let carried = base.and_then(|b| migrated(b, &change, approved, &call.me));
    let base = carried.as_ref().or(base);
    let mut notes_ahead: Vec<String> = Vec::new();
    if let (Some(path), Some(hash)) = (&call.setting_photo, setting_photo_hash) {
        change.setting = Some(Setting::Photo {
            path: path.clone(),
            hash,
        });
    }
    let new_picture = call.picture.is_none();
    if new_picture && change.people.is_empty() {
        if let Some(note) = draw_named(&mut change, call.retouch.as_deref(), approved, named_in) {
            notes_ahead.push(note);
        }
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
    notes.extend(notes_ahead);
    // On a picture whose people were not known before this call (a photo
    // with no record), the people the call names are who is in it, not
    // people to add (§5.1, N2: "once a call declares them, the record knows
    // them"). Drawn as "Add Maya." on a photo of Maya, the edit draws her a
    // second time; and their clothes are the photo's, so none are made up.
    let identified: Vec<String> = if new_picture || known_before {
        Vec::new()
    } else {
        delta.added.clone()
    };
    // Naming who is in such a photo, and saying nothing more about them,
    // changes nothing in it. Their clothes, pose or expression, said, are a
    // change to draw.
    let only_identified = !identified.is_empty()
        && {
            // Naming them ends a recorded relation between unknown people
            // (`together_cleared`), which draws nothing on its own.
            let mut rest = undrawn_scene(&delta);
            rest.added.clear();
            rest.is_empty()
        }
        && next
            .people
            .iter()
            .filter(|p| identified.contains(&p.who.key()))
            .all(|p| {
                p.wearing.trim().is_empty()
                    && p.doing.trim().is_empty()
                    && p.expression.trim().is_empty()
                    && p.at.is_none()
            });
    let unchanged = (delta.is_empty() || only_identified) && unknown_removes.is_empty();
    // What is drawn of the change: all of it, except on the call that first
    // names a photo's people, whose setting, light, camera and relation are
    // recorded but not drawn. On that call a restated-by-habit field cannot
    // be told from an asked one, and drawn over the owner's photo it was the
    // unasked change this rule exists to stop (review of #707, pass 6).
    let drawn = if identified.is_empty() {
        delta.clone()
    } else {
        undrawn_scene(&delta)
    };
    // A painted area keeps everything outside it, so it cannot carry a
    // scene change: the change is drawn, and the mask left out and said.
    let mask = call.mask.clone().filter(|_| unchanged);
    // Regions are the owner's words for places in this picture, and their
    // legend is already the retouch: with the mask dropped it would ride
    // into a redrawn picture's prompt describing outlines that are not there
    // (review of #623, pass 4).
    if call.mask.is_some() && mask.is_none() && !call.regions.is_empty() {
        return Err(
            "The painted regions keep everything outside them, so they cannot carry a \
                    scene change: ask for the scene change on its own."
                .into(),
        );
    }
    if call.mask.is_some() && mask.is_none() {
        notes.push(
            "The `mask` was left out: a scene change redraws more than a painted area.".into(),
        );
    }
    // Words name an area exactly as paint does, and are kept exactly when
    // paint is: no scene change beside them, but the call that first names a
    // photo's people changes nothing drawn (review of #710).
    let mask_words = call.mask_words.clone().filter(|_| unchanged);
    if call.mask_words.is_some() && mask_words.is_none() {
        notes.push(
            "The `mask` was left out: a scene change redraws more than the area it names.".into(),
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
    let mut clothes_said: Vec<(String, String)> = Vec::new();
    // Which identified people this call itself dressed: only those get a
    // "Dress …" line. The others' clothes are the photo's.
    let identified_dressed: Vec<String> = next
        .people
        .iter()
        .filter(|p| identified.contains(&p.who.key()) && !p.wearing.trim().is_empty())
        .map(|p| p.who.key())
        .collect();
    for key in &delta.added {
        // Someone added past the scene's bound was cut from it (`apply`):
        // refused here, never drawn as a picture without them.
        let Some(p) = next.people.iter_mut().find(|p| &p.who.key() == key) else {
            return Err(format!(
                "A picture holds at most {} people; take someone out with `remove` first.",
                crate::scene::MAX_PEOPLE
            ));
        };
        // An identified person's clothes are the photo's: nothing is drawn
        // or said for them, but the record still holds clothes, or the next
        // restage of this picture is refused for a blank `wearing`
        // (`imagelib::compile_with`; review of #707, pass 4).
        if identified.contains(key) {
            if p.wearing.trim().is_empty() {
                p.wearing = WEARING_UNSAID.into();
            }
            continue;
        }
        if p.wearing.trim().is_empty() {
            match (worn(key), &p.who) {
                (Some((w, from)), _) => {
                    clothes_said.push((
                        shown(&p.who),
                        format!(
                            // Never the recorded words themselves: they may be
                            // untrusted, and a tool result is not marked from
                            // outside (review of #597, pass 5).
                            "{} is wearing what this chat last drew them in.",
                            shown(&p.who)
                        ),
                    ));
                    p.wearing = w;
                    // Copied words keep where they came from: an untrusted
                    // record's clothes never land clean (review of #597).
                    p.origin = p.origin.union(from);
                }
                (None, Who::Described(_)) => {
                    p.wearing = WEARING_UNSAID.into();
                }
                (None, Who::Library(_)) => {
                    // Absorbed, and said (mecha-a3's G1: a refusal here was
                    // the panel's most common one on a picture with no record).
                    clothes_said.push((
                        shown(&p.who),
                        format!(
                            "{} had no clothes given or on record, so they wear clothes that suit \
                         the scene.",
                            shown(&p.who)
                        ),
                    ));
                    p.wearing = WEARING_UNSAID.into();
                }
            }
        }
    }
    // A top a change took off is said, into the record too, so the next
    // picture keeps it off (`removed_top`): never left to a portrait, whose
    // own top fills an unsaid chest.
    if let Some(b) = base {
        for p in next.people.iter_mut() {
            let before = b.people.iter().find(|q| q.who.key() == p.who.key());
            if before.is_some_and(|q| removed_top(&q.wearing, &p.wearing)) {
                p.wearing = format!("{}, no top", p.wearing.trim().trim_end_matches(['.', ',']));
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
    // With regions the retouch is the harness's legend, whose own words
    // ("mark", a colour's name) would read as library names: what is checked
    // is what the owner wrote in each region (review of #623, pass 12).
    // A region about a face is where a library name is brought in on
    // purpose, to take that face (§5.3): it is not someone offstage, whom
    // the image model would read as "the viewer" (review of #706, pass 3).
    if !call.regions.is_empty() {
        // Only where that face is sent: one approved name, and no dropped
        // picture outranking it (review of #706, pass 4). A face that still
        // cannot be had (no detector, no face found) is read as "the viewer"
        // by the image tool, which knows it.
        for r in call.regions.iter().filter(|r| {
            !(about_a_face(&r.words) && r.reference.is_none() && named_in(&r.words).len() == 1)
        }) {
            prose.push(("retouch", r.words.clone()));
        }
    } else if let Some(r) = &call.retouch {
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
    // A cast member who holds a first-person camera (`first_person_holder`)
    // is left out, beside someone the camera looks at: the record keeps them, the
    // image model reads "the viewer", and their clothes and act are one line
    // of what that viewer's eyes see. Sent as a cast member under a
    // first-person camera, the holder's portrait drew his face in all four
    // renders of 2026-10-28, and the people beside him twice over. Only for
    // a picture drawn from the scene: a retouch of a picture keeps whoever is
    // in it, and a painted region may name the holder on purpose, to take
    // their face.
    let mut seen_from: Option<String> = None;
    let drawn_from_scene = call.regions.is_empty()
        && call.mask.is_none()
        && call.mask_words.is_none()
        && (new_picture || !unchanged);
    if let Some(camera) = next
        .camera
        .as_ref()
        .map(|f| f.value.as_str())
        .filter(|_| drawn_from_scene)
    {
        let holder = first_person_holder(camera, &named_in(camera)).filter(|h| {
            people.len() >= 2
                && people
                    .iter()
                    .any(|p| matches!(&p.who, Who::Library(n) if n == h))
        });
        if let Some(h) = holder {
            let at = people
                .iter()
                .position(|p| matches!(&p.who, Who::Library(n) if *n == h))
                .expect("found above");
            let p = people.remove(at);
            let mut own: Vec<&str> = Vec::new();
            if !p.wearing.trim().is_empty() && p.wearing.trim() != WEARING_UNSAID {
                own.push(p.wearing.trim());
            }
            if !p.doing.trim().is_empty() {
                own.push(p.doing.trim());
            }
            seen_from = Some(if own.is_empty() {
                "The camera is the viewer's own eyes: the viewer's face is never in the picture."
                    .into()
            } else {
                format!(
                    "The camera is the viewer's own eyes: the viewer's face is never in the \
                     picture, only their body as they see it ({}).",
                    own.join(", ")
                )
            });
            if !offstage.contains(&h) {
                offstage.push(h.clone());
            }
            let name = crate::imagegen::capitalized(&h);
            notes.push(format!(
                "The camera is {name}'s own eyes, so {name} is left out of the cast and read \
                 as \"the viewer\", whose face never shows."
            ));
        }
    }
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
        mask_words,
        next,
        delta: delta.clone(),
        route: "new",
        said: (!notes.is_empty()).then(|| notes.join(" ")),
        clothes_said,
        also: match (
            call.retouch
                .clone()
                .filter(|_| call.picture.is_none() || !unchanged),
            seen_from,
        ) {
            (Some(r), Some(s)) => Some(format!("{}. {s}", r.trim().trim_end_matches('.'))),
            (r, s) => r.or(s),
        },
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
    // People identified on a photo are drawn from it on every route, not only
    // the edit: they leave `added` (who the image tool would describe, with a
    // library description and a default pose) and the crops before any route
    // returns, and the result says how they were read, since naming someone
    // here no longer adds them (review of #707, pass 3).
    if !identified.is_empty() {
        // A room photo given in the call that names them is refused by what it
        // is, whatever else the call says about them: that restage drew them
        // from the library onto the room and dropped the owner's photo, and an
        // edit would record a room the picture was never drawn in (review of
        // #707, pass 5).
        if delta.setting && setting_photo.is_some() {
            return Err(
                "A picture whose people are not known yet cannot be moved into a room photo in \
                 the same call that names them: name them first, then send the room."
                    .into(),
            );
        }
        out.delta.added.retain(|k| !identified.contains(k));
        out.faces.retain(|k| !identified.contains(k));
        // Nor recorded: the next call restaged from a setting the owner may
        // never have asked for, and dropped the photo one call later (review
        // of #707, pass 7). The record keeps what it held before; a call that
        // gives them once the people are known restages as asked.
        out.next.setting = base.and_then(|b| b.setting.clone());
        out.next.light = base.and_then(|b| b.light.clone());
        out.next.camera = base.and_then(|b| b.camera.clone());
        out.next.together = base.and_then(|b| b.together.clone());
        out.delta = undrawn_scene(&out.delta);
        let shown_of = |keys: &[&String]| -> Vec<String> {
            out.next
                .people
                .iter()
                .filter(|p| keys.contains(&&p.who.key()))
                .map(|p| shown(&p.who))
                .collect()
        };
        let all: Vec<&String> = identified.iter().collect();
        let bare: Vec<&String> = identified
            .iter()
            .filter(|k| !identified_dressed.contains(k))
            .collect();
        let names = shown_of(&all);
        let undressed = shown_of(&bare);
        let mut line = format!(
            "This picture had no record of who is in it, so {} {} taken as already in it, not \
             added. Its people are known now: later changes can add someone, and one that gives \
             a setting can restage it.",
            crate::imagelib::join_and(&names),
            if names.len() == 1 { "is" } else { "are" }
        );
        // What the call gave that this edit of the photo does not draw, said,
        // since the record now holds it for the next restage.
        if delta.setting || delta.light || delta.camera || delta.together {
            line.push_str(
                " The setting, light, camera or relation given here were left out, neither \
                 drawn nor recorded: this was an edit of the photo. Give them again to restage \
                 it now that its people are known.",
            );
        }
        if !undressed.is_empty() {
            line.push_str(&format!(
                " Until their clothes are said, a later redraw dresses {} in clothes that suit \
                 the scene.",
                crate::imagelib::join_and(&undressed)
            ));
        }
        out.said = Some(match out.said.take() {
            Some(said) => format!("{said} {line}"),
            None => line,
        });
    }
    // A retouch: the one free-text change to the picture itself.
    if let Some(r) = call.retouch.as_ref().filter(|_| unchanged) {
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
    // Naming who is in a photo with no record is no change to it: drawn
    // again as it is, and its record now knows its people.
    if unchanged {
        out.route = "redrawn";
        // Naming a photo's people is never a redraw from the record's words
        // or room photo: the photo is what they are in (review of #707).
        // Nor is any all-equal call on a picture whose people were not known:
        // the record's setting has nobody in it to redraw.
        let setting_photo = setting_photo.filter(|_| known_before);
        let words_setting = words_setting && known_before;
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
                // "place", never "room": a redraw of a park read "room" as
                // indoors, 2 of 2 (mecha-a3's Regenerate gate, 2026-10-09).
                instruction: "Draw this picture again, with the same people, clothes and place."
                    .into(),
                keep: String::new(),
                ..out
            },
        });
    }
    let restage = delta.restages() || delta.style;
    // Known before this call, not after it: the call that first declares a
    // photo's people is an edit of the photo, and later ones may restage it
    // (§5.1, N2). Read after, the first call naming the person on the
    // owner's photo redrew it from words, clothes, hair and all
    // (owner, 2026-10-16).
    let known = known_before;
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
        if identified.contains(&key) {
            // Already in the picture: only what the call says about them is
            // drawn, from the photo itself (they left the crops above).
            let before = lines.len();
            if identified_dressed.contains(&key) {
                lines.push(format!(
                    "Dress {name} in {}.",
                    p.wearing.trim_end_matches('.')
                ));
            }
            if !p.expression.trim().is_empty() {
                lines.push(format!(
                    "Give {name} this expression: {}.",
                    p.expression.trim_end_matches('.')
                ));
            }
            let doing = p.doing.trim().trim_end_matches('.');
            match (doing.is_empty(), p.at) {
                (false, _) => lines.push(format!("Have {name} {doing}.")),
                (true, Some(at)) => lines.push(format!("Place {name} {}.", at.name())),
                (true, None) => {}
            }
            if lines.len() > before {
                touched.push(key);
            }
            continue;
        }
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
    if drawn.camera {
        if let Some(c) = &out.next.camera {
            lines.push(format!("{}.", c.value.trim_end_matches('.')));
        }
    }
    if drawn.setting {
        if let Some(Setting::Words { text }) = out.next.setting.as_ref().map(|f| &f.value) {
            lines.push(format!("Set the scene in {}.", text.trim_end_matches('.')));
        }
    }
    if drawn.light {
        if let Some(l) = &out.next.light {
            lines.push(format!("Make the light {}.", l.value.trim_end_matches('.')));
        }
    }
    if drawn.together {
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
    // after it; everyone else is on the canvas already. (People identified on
    // a photo left the crops and `added` before any route returned.)
    out.faces.retain(|k| touched.contains(k));
    out.render = Render::Edit {
        canvas: Canvas::Picture(picture),
        camera_moves: drawn.camera,
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
    } else if drawn.together {
        // A relation moves the people it names; only their clothes stay.
        keep.push("everyone's clothes");
    } else if touched.is_empty() && delta.removed.is_empty() && unknown_removes.is_empty() {
        keep.push("the people, their clothes and poses");
    } else {
        keep.push("everyone else as they are");
    }
    if !drawn.setting && !delta.style {
        keep.push("the room");
    }
    if !drawn.light && !delta.style {
        keep.push("the light");
    }
    if !drawn.camera {
        keep.push("the camera");
    }
    out.keep = match keep.split_last() {
        Some((last, [])) => last.to_string(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    };
    out.route = "edited";
    // Said already for people identified on a photo (above).
    if restage && (!known || no_setting) && (no_setting || identified.is_empty()) {
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

    /// Regions go with a mask and a picture, in palette colours, each with
    /// words, at most four; their legend becomes the retouch.
    #[test]
    fn regions_are_read_strictly_and_become_the_legend() {
        let c = call(json!({"picture": "images/a.png", "mask": "inbox/r.png",
            "regions": [{"colour": "magenta", "words": "make it red."},
                        {"colour": "blue", "words": "remove the cup"}]}));
        assert_eq!(c.regions.len(), 2);
        assert_eq!(c.regions[1].rgb, [0, 64, 255]);
        assert_eq!(
            c.retouch.as_deref(),
            Some(
                "The coloured outlines in <image2> mark where each change goes in <image1>: in \
                 the magenta outline, make it red; in the blue outline, remove the cup. Do not \
                 draw the outlines."
            )
        );
        assert_eq!(c.mask.as_deref(), Some("inbox/r.png"));
        let bad = |v: Value| parse(&v, &lib, &me(), false).unwrap_err();
        assert!(bad(json!({"picture": "images/a.png",
            "regions": [{"colour": "magenta", "words": "x"}]}))
        .contains("go with the `mask`"));
        // Words in `mask` beside regions are refused by name (review of
        // #710, pass 3).
        assert!(bad(json!({"picture": "images/a.png", "mask": "the sky",
            "regions": [{"colour": "magenta", "words": "x"}]}))
        .contains("a `mask` in words cannot go beside them"));
        assert!(bad(
            json!({"picture": "images/a.png", "mask": "inbox/r.png", "retouch": "y",
            "regions": [{"colour": "magenta", "words": "x"}]})
        )
        .contains("their own words"));
        assert!(bad(json!({"picture": "images/a.png", "mask": "inbox/r.png",
            "regions": [{"colour": "teal", "words": "x"}]}))
        .contains("not one of"));
        assert!(bad(json!({"picture": "images/a.png", "mask": "inbox/r.png",
            "regions": [{"colour": "cyan", "words": "x"}, {"colour": "cyan", "words": "y"}]}))
        .contains("Two regions are painted cyan"));
        assert!(bad(json!({"picture": "images/a.png", "mask": "inbox/r.png",
            "regions": [{"colour": "cyan", "words": " "}]}))
        .contains("says nothing"));
        let five: Vec<Value> = ["magenta", "cyan", "blue", "green", "orange"]
            .iter()
            .map(|c| json!({"colour": c, "words": "x"}))
            .collect();
        assert!(
            bad(json!({"picture": "images/a.png", "mask": "inbox/r.png", "regions": five}))
                .contains("at most 4")
        );
        // A picture dropped on a region is its path in this chat, or refused
        // by name, never dropped (review of #706).
        let c = call(json!({"picture": "images/a.png", "mask": "inbox/r.png",
            "regions": [{"colour": "magenta", "words": "her face", "reference": "inbox/f.png"},
                        {"colour": "cyan", "words": "x"}]}));
        assert_eq!(c.regions[0].reference.as_deref(), Some("inbox/f.png"));
        assert_eq!(c.regions[1].reference, None);
        assert!(bad(json!({"picture": "images/a.png", "mask": "inbox/r.png",
            "regions": [{"colour": "magenta", "words": "x", "reference": "a stranger's face"}]}))
        .contains("magenta region's `reference` is not a picture's path"));
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

    /// A name followed by a description is that character, never a stranger
    /// whose name is then "the viewer" (owner, 2026-10-27); with no
    /// separator after the name, the words stay a description.
    #[test]
    fn a_name_leading_a_description_is_that_character() {
        let c = call(json!({"scene": {"setting": "a pottery shed", "people": [
            {"who": "Maya, a potter in a clay-flecked apron", "where": "centre"},
            {"who": "John (her brother), in his forties"},
            {"who": "Wren — the neighbour"},
            {"who": "Ivo - a painter"},
            {"who": "John's sister"},
            {"who": "a stranger, tall"}]}}));
        let whos: Vec<_> = c.change.people.iter().map(|p| p.who.clone()).collect();
        assert_eq!(
            whos,
            [
                Who::Library("maya".into()),
                Who::Library("john".into()),
                Who::Library("wren".into()),
                Who::Library("ivo".into()),
                Who::Described("John's sister".into()),
                Who::Described("a stranger, tall".into()),
            ]
        );
        // The persona's own name leading a description is the persona.
        let c = call(json!({"scene": {"setting": "an orchard", "people": [
            {"who": "Maya (self), carrying a basket of plums"}]}}));
        let p = planned(&c, None);
        assert_eq!(p.next.people[0].who, Who::Library("maya".into()));
        assert!(p.offstage.is_empty(), "{:?}", p.offstage);
        // The words left out are said, so an act can move to `doing`.
        assert!(
            c.notes
                .iter()
                .any(|n| n.contains("basket of plums") && n.contains("`doing`")),
            "{:?}",
            c.notes
        );
        // A persona with no approved character is drawn from its words, never
        // refused for having no "self" (review of #756).
        let unlinked = SelfNames {
            character: None,
            names: vec!["Maya".into()],
        };
        let c = parse(
            &json!({"scene": {"setting": "an orchard", "people": [
                {"who": "Maya, a potter in a clay-flecked apron"}]}}),
            &|_| false,
            &unlinked,
            false,
        )
        .unwrap();
        assert_eq!(
            c.change.people[0].who,
            Who::Described("Maya, a potter in a clay-flecked apron".into())
        );
    }

    /// A record written while a name leading a description read as a
    /// stranger holds `Described("maya, …")`: an edit naming Maya is that
    /// person, never a second Maya, and a `remove` finds her (review of #756).
    /// A name with nothing after its separator leaves nothing out to say.
    #[test]
    fn an_old_record_of_a_name_and_description_is_that_character() {
        let first = planned(
            &call(json!({"scene": {"setting": "a pottery shed", "people": [
                {"who": "a potter", "wearing": "an apron", "doing": "throwing a pot"}]}})),
            None,
        );
        let mut base = landed(&first, 3);
        base.people[0].who = Who::Described("maya, a potter in a clay-flecked apron".into());
        let p = planned(
            &call(json!({"picture": "images/a.png", "scene": {"people": [
                {"who": "Maya", "wearing": "a blue dress"}]}})),
            Some(&base),
        );
        assert_eq!(p.next.people.len(), 1, "{:?}", p.next.people);
        assert_eq!(p.next.people[0].who, Who::Library("maya".into()));
        let p = planned(
            &call(json!({"picture": "images/a.png", "scene": {"people": [
                {"who": "Maya", "remove": true}]}})),
            Some(&base),
        );
        assert!(p.next.people.is_empty(), "{:?}", p.next.people);
        for who in ["Maya,", "Maya (self)"] {
            let c = call(json!({"scene": {"setting": "a shed", "people": [{"who": who}]}}));
            assert_eq!(c.change.people[0].who, Who::Library("maya".into()));
            assert!(
                !c.notes.iter().any(|n| n.contains("left out")),
                "{who}: {:?}",
                c.notes
            );
        }
        // Two described people led by one name: only the first becomes her.
        let mut two = base.clone();
        two.people.push(two.people[0].clone());
        two.people[1].who = Who::Described("maya (self) in the background".into());
        let p = planned(
            &call(json!({"picture": "images/a.png", "scene": {"people": [
                {"who": "Maya", "wearing": "a blue dress"}]}})),
            Some(&two),
        );
        let mayas = p
            .next
            .people
            .iter()
            .filter(|q| q.who == Who::Library("maya".into()))
            .count();
        assert_eq!(mayas, 1, "{:?}", p.next.people);
        // A persona whose character is not its name: the record's "<Name>
        // (self)" and "self, …" are read as the call's own `who` is (review
        // of #756, pass 4).
        let paper = SelfNames {
            character: Some("wren".into()),
            names: vec!["Paper".into()],
        };
        for old in ["paper (self), by the window", "self, by the window"] {
            let mut rec = base.clone();
            rec.people[0].who = Who::Described(old.into());
            let c = parse(
                &json!({"picture": "images/a.png", "scene": {"people": [
                    {"who": "self", "wearing": "a blue dress"}]}}),
                &lib,
                &paper,
                false,
            )
            .unwrap();
            let p = plan(&c, Some(&rec), None, Origin::Clean, &lib, &names, &|_| None).unwrap();
            assert_eq!(p.next.people.len(), 1, "{old}: {:?}", p.next.people);
            assert_eq!(p.next.people[0].who, Who::Library("wren".into()), "{old}");
        }
    }

    /// A camera behind the people turns their faces away; one that names
    /// the front, a shoulder looked over, or a face toward the lens does
    /// not, and a back toward the lens is still from behind.
    #[test]
    fn faces_away_reads_the_camera() {
        for away in [
            "tight on the wheel, from behind the potter",
            "rear quarter view by the kiln",
            "the lens sits behind him at the bench",
            "the camera is behind him, low",
            "from behind her, the wheel in front of her hands",
            "she works with her back to us",
            "his back toward the camera, glaze buckets beside",
            "from the back, shoulders filling the frame",
            "she has turned away from the lens",
            "her back is to the lens, kiln glowing",
        ] {
            assert!(faces_away(away), "{away}");
        }
        for not in [
            "wide on the workshop, eye level",
            "behind the bench, then round to the front",
            "from behind the kiln, over her shoulder",
            "rear quarter, her face toward the lens",
            "the lens pulled back, wide on the workshop",
            "his apron tied back, the lens close",
            "tight on her face, shelves of pots behind her",
            "wide on the potter at her wheel, behind her the kiln",
            "tight on her hands, behind her the shelves",
            "pulled back to show us the whole bench",
            "shot from the back wall of the shed, wide on her face",
            "stepped back toward the camera",
            "a medium shot so the mural is behind him",
            "tight on her face so the kiln is behind her",
            "from the back of the shed, wide on her face",
            "lit from behind by the kiln, eye level",
            "standing at the rear of the shed, toward the lens",
            "tight on his face and the clay",
            "",
        ] {
            assert!(!faces_away(not), "{not}");
        }
        for shot in [
            "Close-up",
            "behind the wheel",
            "full body, wide",
            "low angle by the kiln",
        ] {
            assert!(is_framing(shot), "{shot}");
        }
        for look in [
            "pencil sketch",
            "1930s lantern slide",
            "watercolour",
            "back lit dusk",
        ] {
            assert!(!is_framing(look), "{look}");
        }
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
        assert!(p.clothes_said[0].1.contains("John had no clothes given"));
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

    /// A first-person camera's holder is the name it gives them by a
    /// possessive, or its one name; a point of view alone is not first
    /// person, and two names with no possessive are nobody's eyes.
    #[test]
    fn a_first_person_camera_names_whose_eyes_it_is() {
        let n = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        for (camera, names, holder) in [
            (
                "first person, showing John at the stove",
                n(&["john"]),
                Some("john"),
            ),
            (
                "from john's first-person view, his face out of frame",
                n(&["john"]),
                Some("john"),
            ),
            (
                "a first-person shot from John's side of the table, Maya opposite",
                n(&["john", "maya"]),
                Some("john"),
            ),
            (
                "close on Maya, seen through John's eyes",
                n(&["maya", "john"]),
                Some("john"),
            ),
            (
                "a wide angle from John's POV across the garden",
                n(&["john"]),
                None,
            ),
            (
                "first person view of Maya and John",
                n(&["maya", "john"]),
                None,
            ),
            ("a first person shot", n(&[]), None),
        ] {
            assert_eq!(
                first_person_holder(camera, &names).as_deref(),
                holder,
                "{camera}"
            );
        }
    }

    /// Owner, 2026-10-28: someone whose own eyes the camera is, listed in
    /// `people`, had their portrait sent and their face drawn in 4 of 4
    /// renders. They stay in the record, are left out of the cast, read as
    /// "the viewer", and their act is the viewer's line. Fails on the old
    /// planner, which cast them.
    #[test]
    fn a_first_person_cameras_holder_is_the_viewer_not_the_cast() {
        let p = planned(
            &call(json!({"scene": {"setting": "a kitchen",
                "camera": "first-person view through John's eyes, across the kitchen",
                "people": [
                    {"who": "maya", "wearing": "a coat", "doing": "stirring a pot"},
                    {"who": "john", "wearing": "pyjamas", "doing": "leaning on the counter"}]}})),
            None,
        );
        let cast: Vec<String> = p.people.iter().map(|q| q.who.key()).collect();
        assert_eq!(cast, ["maya"]);
        assert!(!p.faces.contains(&"john".to_string()), "{:?}", p.faces);
        let kept: Vec<String> = p.next.people.iter().map(|q| q.who.key()).collect();
        assert_eq!(kept, ["maya", "john"], "the record keeps him");
        assert_eq!(p.offstage, vec!["john".to_string()]);
        let also = p.also.as_deref().unwrap_or("");
        assert!(also.contains("the viewer's own eyes"), "{also}");
        assert!(also.contains("pyjamas, leaning on the counter"), "{also}");
        assert!(p.said.as_deref().unwrap_or("").contains("John's own eyes"));
        // Not first person: a wide shot from his point of view draws him.
        let wide = planned(
            &call(json!({"scene": {"setting": "a kitchen",
                "camera": "a wide angle from John's POV",
                "people": [
                    {"who": "maya", "wearing": "a coat", "doing": "sitting"},
                    {"who": "john", "wearing": "pyjamas", "doing": "leaning on the counter"}]}})),
            None,
        );
        assert_eq!(wide.people.len(), 2);
        assert!(wide.also.is_none());
        // Alone, the camera has nobody else to see: he is drawn.
        let alone = planned(
            &call(json!({"scene": {"setting": "a kitchen",
                "camera": "first-person view through John's eyes, across the kitchen",
                "people": [{"who": "john", "wearing": "pyjamas", "doing": "reading"}]}})),
            None,
        );
        assert_eq!(alone.people.len(), 1);
    }

    /// Nudity opening an outfit is said outright; anything else is as given.
    #[test]
    fn a_nude_outfit_is_spelled_out() {
        for nude in ["Nude", "naked.", "nothing at all", "Completely nude"] {
            assert_eq!(
                outfit_words(nude),
                "nothing at all, completely naked",
                "{nude}"
            );
        }
        assert_eq!(
            outfit_words("Nude, perched on a stool"),
            "nothing at all, completely naked, perched on a stool"
        );
        for kept in [
            "a nude-coloured slip dress",
            "jeans and boots",
            "a red coat.",
        ] {
            assert_eq!(outfit_words(kept), kept.trim_end_matches('.'), "{kept}");
        }
    }

    /// A top is taken off only when the old outfit covered the chest and the
    /// new one names lower garments alone; said once.
    #[test]
    fn a_top_taken_off_is_read_from_the_two_outfits() {
        assert!(removed_top(
            "a linen shirt over a sports bra, cargo shorts",
            "cargo shorts"
        ));
        assert!(removed_top("a grey t-shirt and jeans", "jeans"));
        assert!(!removed_top(
            "a grey t-shirt and jeans",
            "a hoodie and jeans"
        ));
        assert!(!removed_top("jeans and boots", "jeans"), "no top before");
        assert!(
            !removed_top("a sundress", "barefoot"),
            "no lower garment named"
        );
        assert!(
            !removed_top("a sundress", "cargo shorts, no top"),
            "already said"
        );
        assert!(!removed_top("a sundress", "cargo shorts, topless"));
    }

    /// 2026-10-28: a change took the shirt off and kept the shorts, and the
    /// portrait's top came back at 3 of 3 seeds. The change and the record
    /// say "no top", so the next picture keeps it off. Fails on the old
    /// planner, which passed the shorts alone.
    #[test]
    fn a_change_that_takes_the_top_off_says_so_in_the_record() {
        let first = planned(
            &call(json!({"scene": {"setting": "a porch",
                "people": [{"who": "maya", "wearing": "a denim shirt and cargo shorts",
                    "doing": "leaning on the rail"}]}})),
            None,
        );
        let base = landed(&first, 3);
        let p = planned(
            &call(json!({"picture": "images/a.png", "scene": {"people": [
                {"who": "maya", "wearing": "cargo shorts"}]}})),
            Some(&base),
        );
        assert_eq!(p.next.people[0].wearing, "cargo shorts, no top");
        // A later change of pose alone keeps it so.
        let after = planned(
            &call(json!({"picture": "images/b.png", "scene": {"people": [
                {"who": "maya", "doing": "sitting on the step"}]}})),
            Some(&landed(&p, 3)),
        );
        assert_eq!(after.next.people[0].wearing, "cargo shorts, no top");
    }

    /// A retouch of a picture whose camera is first person keeps its holder:
    /// a painted region may name them to take their face, and the planner
    /// must not make them the viewer there (measured 2026-10-28: the region's
    /// words had the name rewritten under a face taken by that name).
    #[test]
    fn a_retouch_under_a_first_person_camera_keeps_its_holder() {
        let first = planned(
            &call(json!({"scene": {"setting": "a kitchen",
                "camera": "first-person view through John's eyes",
                "people": [
                    {"who": "maya", "wearing": "a coat", "doing": "stirring a pot"},
                    {"who": "john", "wearing": "pyjamas", "doing": "leaning on the counter"}]}})),
            None,
        );
        let base = landed(&first, 7);
        let p = planned(
            &call(json!({"picture": "images/a.png", "mask": "inbox/r.png",
                "regions": [{"colour": "blue", "words": "John's face, sharper"}]})),
            Some(&base),
        );
        assert!(p.offstage.is_empty(), "{:?}", p.offstage);
        assert!(!p.also.as_deref().unwrap_or("").contains("own eyes"));
        // A scene change on it is drawn from the scene: the holder is the
        // viewer again.
        let q = planned(
            &call(json!({"picture": "images/a.png", "scene": {"light": "dusk"}})),
            Some(&base),
        );
        assert_eq!(q.offstage, vec!["john".to_string()]);
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

    /// A picture with no record is redrawn from itself, and its words name
    /// the place, never a "room" an outdoor picture would be moved into.
    #[test]
    fn a_picture_with_no_record_is_redrawn_in_its_own_place() {
        let p = planned(&call(json!({"picture": "images/a.png"})), None);
        assert_eq!(p.route, "redrawn");
        assert!(p.instruction.contains("place"), "{}", p.instruction);
        assert!(!p.instruction.contains("room"), "{}", p.instruction);
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

    /// The names check reads each region's own words, never the legend the
    /// harness wrote: a character keyed `mark` or `red` would otherwise be
    /// offstage on every regions edit, and the measured legend rewritten to
    /// "the viewer" (review of #623, pass 12).
    #[test]
    fn a_regions_legend_is_not_read_for_names() {
        let first = planned(
            &call(json!({"scene": {"setting": "a study", "people": [
                {"who": "maya", "wearing": "a coat", "doing": "reading"}]}})),
            None,
        );
        let base = landed(&first, 3);
        let approved = |n: &str| n == "mark" || n == "red" || lib(n);
        let named = |t: &str| {
            let t = t.to_lowercase();
            ["mark", "red", "maya", "wren"]
                .into_iter()
                .filter(|n| t.split(|c: char| !c.is_alphanumeric()).any(|w| w == *n))
                .map(String::from)
                .collect::<Vec<_>>()
        };
        let regions = |words: &str| {
            call(json!({"picture": "images/a.png", "mask": "inbox/r.png",
                "regions": [{"colour": "red", "words": words}]}))
        };
        let p = plan(
            &regions("a green scarf"),
            Some(&base),
            None,
            Origin::Clean,
            &approved,
            &named,
            &|_| None,
        )
        .unwrap();
        assert!(p.offstage.is_empty(), "{:?}", p.offstage);
        // The owner's own words still are.
        let p = plan(
            &regions("Wren's green scarf"),
            Some(&base),
            None,
            Origin::Clean,
            &approved,
            &named,
            &|_| None,
        )
        .unwrap();
        assert_eq!(p.offstage, vec!["wren".to_string()]);
        // A region about a face brings the name in on purpose: its face is
        // taken (§5.3), so it is not "the viewer" (review of #706, pass 3).
        let p = plan(
            &regions("Wren's face"),
            Some(&base),
            None,
            Origin::Clean,
            &approved,
            &named,
            &|_| None,
        )
        .unwrap();
        assert!(p.offstage.is_empty(), "{:?}", p.offstage);
        // Only where that face is sent: two names take neither, and a dropped
        // picture outranks the name (review of #706, pass 4).
        let p = plan(
            &regions("Wren and Mark's faces"),
            Some(&base),
            None,
            Origin::Clean,
            &approved,
            &named,
            &|_| None,
        )
        .unwrap();
        assert_eq!(p.offstage, vec!["mark".to_string(), "wren".to_string()]);
        let dropped = call(json!({"picture": "images/a.png", "mask": "inbox/r.png",
            "regions": [{"colour": "red", "words": "Wren's face", "reference": "inbox/f.png"}]}));
        let p = plan(
            &dropped,
            Some(&base),
            None,
            Origin::Clean,
            &approved,
            &named,
            &|_| None,
        )
        .unwrap();
        assert_eq!(p.offstage, vec!["wren".to_string()]);
    }

    /// Regions beside a scene change are refused: the change drops the mask,
    /// and the regions' legend would ride into a redrawn picture's prompt
    /// describing outlines that are not there (review of #623, pass 4).
    #[test]
    fn regions_beside_a_scene_change_are_refused() {
        let first = planned(
            &call(json!({"scene": {"setting": "a study", "people": [
                {"who": "maya", "wearing": "a coat", "doing": "reading"}]}})),
            None,
        );
        let base = landed(&first, 3);
        let why = plan(
            &call(json!({"picture": "images/a.png", "mask": "inbox/r.png",
                "regions": [{"colour": "magenta", "words": "make it red"}],
                "scene": {"camera": "from above"}})),
            Some(&base),
            None,
            Origin::Clean,
            &lib,
            &names,
            &|_| None,
        )
        .unwrap_err();
        assert!(why.contains("cannot carry a scene change"), "{why}");
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
        assert!(p
            .clothes_said
            .iter()
            .any(|(who, note)| who == "John" && note.contains("what this chat last drew them in")));
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
        // Words are kept as words (`docs/SEGMENTATION-DESIGN.md` S1).
        assert!(c.mask.is_none() && c.notes.is_empty(), "{:?}", c.notes);
        assert_eq!(c.mask_words.as_deref(), Some("the brass weathervane"));
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

    /// The first call to name a photo's people is an edit of the photo,
    /// never a restage from words (§5.1, N2: the decision reads what was
    /// known before the call). They are who is in it, not people to add:
    /// only what the call says about them is drawn (owner, 2026-10-16: the
    /// first call naming the person on a photo redrew it, clothes and all).
    #[test]
    fn a_photos_first_call_naming_its_people_edits_it_and_never_adds_them() {
        let p = planned(
            &call(json!({"picture": "inbox/her.jpg", "scene": {
                "setting": "a narrow greenhouse", "camera": "from the doorway",
                "people": [{"who": "maya", "wearing": "a mustard raincoat"}]}})),
            None,
        );
        assert_eq!(p.route, "edited");
        assert!(matches!(
            p.render,
            Render::Edit {
                canvas: Canvas::Picture(_),
                ..
            }
        ));
        assert!(
            p.instruction.contains("Dress Maya in a mustard raincoat."),
            "{}",
            p.instruction
        );
        assert!(!p.instruction.contains("Add Maya"), "{}", p.instruction);
        // The restated setting and camera are recorded, not drawn, and the
        // keep sentence keeps them (review of #707, pass 6).
        assert!(
            !p.instruction.contains("Set the scene"),
            "{}",
            p.instruction
        );
        assert!(
            !p.instruction.contains("from the doorway"),
            "{}",
            p.instruction
        );
        assert!(
            p.keep.contains("the room") && p.keep.contains("the camera"),
            "{}",
            p.keep
        );
        assert!(
            p.faces.is_empty(),
            "drawn from the photo, never a portrait's crop"
        );
        assert!(
            p.delta.added.is_empty(),
            "identified, not added: {:?}",
            p.delta
        );
        assert!(
            p.said.as_deref().unwrap_or_default().contains("known now"),
            "{:?}",
            p.said
        );
        assert!(p.next.people_known);
        // What the edit did not draw is said, since the record holds it
        // (review of #707, pass 5).
        let said = p.said.as_deref().unwrap_or_default();
        assert!(said.contains("neither drawn nor recorded"), "{said}");
        assert!(
            p.next.setting.is_none() && p.next.camera.is_none(),
            "{:?}",
            p.next
        );
        assert!(!said.contains("Until their clothes are said"), "{said}");
        // Once known, a later change restages it as any recorded picture.
        let base = landed(&p, 9);
        let later = planned(
            &call(
                json!({"picture": "images/a.png", "scene": {"camera": "from the potting bench"}}),
            ),
            Some(&base),
        );
        // Nothing restaged from a setting that was never asked for: with none
        // recorded, a camera change is an edit of the photo (review of #707,
        // pass 7).
        assert_eq!(later.route, "edited");
    }

    /// Only words in `mask` or a face region are charged for segmentation.
    #[test]
    fn a_call_may_segment_only_with_words_or_a_face_region() {
        let words = call(json!({"picture": "images/a.png", "retouch": "x", "mask": "the sky"}));
        assert!(words.may_segment());
        let paint = call(json!({"picture": "images/a.png", "retouch": "x", "mask": "inbox/m.png"}));
        assert!(!paint.may_segment());
        let region = |w: &str| {
            call(json!({"picture": "images/a.png", "mask": "inbox/r.png",
                "regions": [{"colour": "magenta", "words": w}]}))
        };
        assert!(!region("make the shirt red").may_segment());
        assert!(region("her face").may_segment());
    }

    /// A bare file name in `mask` is a path, never words for segmentation
    /// (review of #710, pass 2).
    #[test]
    fn a_bare_file_name_in_mask_is_a_path() {
        for path in ["mask.png", "inbox/My Photo.png"] {
            let c = call(json!({"picture": "images/a.png", "retouch": "x", "mask": path}));
            assert_eq!(c.mask.as_deref(), Some(path));
            assert!(c.mask_words.is_none());
        }
    }

    /// Words in `mask` are kept exactly when paint is: on the call that
    /// first names a photo's people, which draws no scene change, the area
    /// named is still the mask (review of #710).
    #[test]
    fn mask_words_survive_the_call_that_first_names_a_photos_people() {
        let p = planned(
            &call(
                json!({"picture": "inbox/her.jpg", "retouch": "Make the scarf blue.",
                "mask": "the scarf", "scene": {"people": [{"who": "maya"}]}}),
            ),
            None,
        );
        assert_eq!(p.mask_words.as_deref(), Some("the scarf"), "{:?}", p.said);
        assert!(p.next.people_known);
        // A scene change that is drawn still drops them.
        let moved = planned(
            &call(
                json!({"picture": "inbox/her.jpg", "retouch": "Make the scarf blue.",
                "mask": "the scarf", "scene": {"camera": "from below"}}),
            ),
            None,
        );
        assert!(moved.mask_words.is_none());
    }

    /// A photo whose record has a setting but whose people are still
    /// unknown: naming them is an edit of the photo, never a redraw from the
    /// record's words; a place said for them is drawn (review of #707).
    #[test]
    fn naming_a_photos_people_after_its_setting_keeps_the_photo() {
        let first = planned(
            &call(json!({"picture": "inbox/her.jpg", "scene": {
                "setting": "a narrow greenhouse", "camera": "from the doorway"}})),
            None,
        );
        assert!(first
            .said
            .as_deref()
            .unwrap_or_default()
            .contains("not known yet"));
        let base = landed(&first, 9);
        assert!(!base.people_known && base.setting.is_some());
        let named = planned(
            &call(json!({"picture": "images/a.png", "scene": {"people": [{"who": "maya"}]}})),
            Some(&base),
        );
        assert!(
            matches!(
                named.render,
                Render::Edit {
                    canvas: Canvas::Picture(_),
                    ..
                }
            ),
            "{:?}",
            named.render
        );
        assert!(named.next.people_known);
        let placed = planned(
            &call(json!({"picture": "images/a.png", "scene": {"people": [
                {"who": "maya", "where": "left"}]}})),
            Some(&base),
        );
        assert_eq!(placed.route, "edited");
        assert!(
            placed.instruction.contains("Place Maya on the left."),
            "{}",
            placed.instruction
        );
    }

    /// A person named on a photo lands in the record with clothes, so the
    /// next restage of that picture compiles rather than being refused for a
    /// blank `wearing`; no clothes line was drawn for them (review of #707,
    /// pass 4).
    #[test]
    fn a_photos_named_people_can_be_restaged_later() {
        let first = planned(
            &call(json!({"picture": "inbox/her.jpg", "scene": {
                "setting": "a narrow greenhouse", "people": [{"who": "maya"}]}})),
            None,
        );
        assert!(
            !first.instruction.contains("Dress"),
            "{}",
            first.instruction
        );
        let base = landed(&first, 9);
        assert!(
            base.people.iter().all(|p| !p.wearing.trim().is_empty()),
            "{:?}",
            base.people
        );
        let later = planned(
            &call(json!({"picture": "images/a.png", "scene": {
                "setting": "a narrow greenhouse", "camera": "from the doorway"}})),
            Some(&base),
        );
        assert_eq!(later.route, "restaged");
    }

    /// A room photo beside the call that first names a photo's people is
    /// refused by what it is: that restage drew them from the library and
    /// dropped the owner's photo. Named first, the room can follow.
    #[test]
    fn a_room_photo_beside_a_naming_is_refused_by_name() {
        let refused = plan(
            &call(json!({"picture": "inbox/her.jpg", "scene": {
                "setting": {"photo": "inbox/room.png"}, "people": [{"who": "maya"}]}})),
            None,
            Some("h".repeat(64)),
            Origin::Clean,
            &lib,
            &names,
            &|_| None,
        )
        .unwrap_err();
        assert!(
            refused.contains("name them first, then send the room"),
            "{refused}"
        );
        // Whatever else the call says about them (review of #707, pass 5).
        let dressed = plan(
            &call(json!({"picture": "inbox/her.jpg", "scene": {
                "setting": {"photo": "inbox/room.png"},
                "people": [{"who": "maya", "wearing": "a mustard raincoat"}]}})),
            None,
            Some("h".repeat(64)),
            Origin::Clean,
            &lib,
            &names,
            &|_| None,
        )
        .unwrap_err();
        assert!(
            dressed.contains("name them first, then send the room"),
            "{dressed}"
        );
    }

    /// Naming who is in a photo, and nothing else, changes nothing in it:
    /// drawn again as it is, and the record learns its people. A retouch
    /// beside the naming is the change, sent once, and its mask is kept.
    #[test]
    fn naming_who_is_in_a_photo_alone_changes_nothing_in_it() {
        let named = json!([{"who": "maya"}]);
        let p = planned(
            &call(json!({"picture": "inbox/her.jpg", "scene": {"people": named}})),
            None,
        );
        assert_eq!(p.route, "redrawn");
        assert!(
            p.instruction.starts_with("Draw this picture again"),
            "{}",
            p.instruction
        );
        assert!(p.faces.is_empty());
        assert!(
            p.delta.added.is_empty(),
            "identified on every route: {:?}",
            p.delta
        );
        assert!(
            p.said
                .as_deref()
                .unwrap_or_default()
                .contains("Maya is taken as already in it"),
            "{:?}",
            p.said
        );
        assert!(p.next.people_known);
        let p = planned(
            &call(
                json!({"picture": "inbox/her.jpg", "retouch": "Hang a brass lantern by the door.",
                "mask": "inbox/m.png", "scene": {"people": named}}),
            ),
            None,
        );
        assert_eq!(p.route, "retouched");
        assert_eq!(p.instruction, "Hang a brass lantern by the door.");
        assert_eq!(p.also, None, "the retouch is sent once");
        assert_eq!(p.mask.as_deref(), Some("inbox/m.png"));
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
