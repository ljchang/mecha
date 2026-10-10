//! Layers for touching scenes (IMAGE-DESIGN.md §15): a picture whose people
//! touch, built from a plate of the room, one cutout per person, a placing
//! pass and a finish, rather than drawn in one pass.
//!
//! The owner turns it on per persona (§15.1); the harness decides it before
//! anything is drawn, so the layers exist and are kept. This module holds
//! the parts that are functions of their inputs: when a picture qualifies,
//! the four prompts, and the cutout's flattening. `image_generate` runs the
//! renders.

/// One person in a layered build: who, what they wear, their part of the
/// act, and their library portrait.
#[derive(Debug, Clone)]
pub struct Person {
    /// The library name, as the record keys it.
    pub key: String,
    /// How the scene's words name them, to be replaced by their tag.
    pub shown: String,
    pub wearing: String,
    /// Their part from the split, or empty when the split fell back.
    pub part: String,
    /// Their expression as the call gave it, said with their part: the
    /// single pass carries it, so a layered build drops nothing it would
    /// draw (review of #626).
    pub expression: String,
    /// Where they stand in the frame ("on the left"), from the split's
    /// places: a placing pass that named no positions left a person out
    /// (mecha-a3, 2026-10-10: one of two never placed, 2 of 2 runs).
    pub at: Option<String>,
    pub portrait: Vec<u8>,
    pub ext: &'static str,
}

/// What a layered build needs, settled before the job starts.
#[derive(Debug, Clone)]
pub struct Plan {
    pub plate: String,
    pub light: String,
    pub people: Vec<Person>,
    /// What of the act no part says: the split's leftover, or the call's
    /// `together` as written when the split fell back (§15.3).
    pub leftover: Option<String>,
    /// The scene the build drew on: its whole origin (`Scene::origin`).
    pub origin: crate::scene::Origin,
    /// A library style's own words, laid on by the finish alone; the plate,
    /// the cutouts and the placing pass stay photographic.
    pub style: Option<String>,
    /// Every chat-derived string the passes' prompts are built from, for
    /// the prompt log's `words` (what `check-private` reads).
    pub words: Vec<String>,
}

/// The most people a layered build has been measured with (mecha-a3's gate:
/// every recorded touching call is two people). Three to five would be
/// n + 3 renders in one time budget and a fuller placing pass, unmeasured,
/// so they draw in one pass until a gate covers them (review of #624).
pub const MEASURED_PEOPLE: usize = 2;

/// Why a picture the switch would layer is drawn in one pass instead, or
/// `None` when it qualifies. Only touching scenes are asked about at all.
/// `placed` is whether everyone has a place: a split that applied always
/// gives one, and a split that fell back leaves only the call's own. A
/// placing pass with no places left one of two people out, 2 of 2 runs
/// (mecha-a3), so that build is not attempted (review of #624). `fits` is
/// whether the placing pass's references, the plate and a cutout each, fit
/// one edit: today the cast cap implies it, but the ordinary edit's budget
/// check never sees this pass, so it is asked here rather than assumed
/// (review of #624).
pub fn not_layered(
    people: usize,
    library_people: usize,
    with_portraits: usize,
    setting_words: bool,
    placed: bool,
    fits: bool,
) -> Option<&'static str> {
    if !fits {
        Some("it has more people than one edit can hold")
    } else if people > MEASURED_PEOPLE {
        Some("layers have been measured with two people, not more")
    } else if library_people < people {
        Some("someone in it is described in words, not drawn from the library")
    } else if with_portraits < people {
        Some("someone in it has no library portrait")
    } else if !setting_words {
        Some("it has no setting in words to build the room from")
    } else if !placed {
        Some("the roles could not be split, so not everyone has a place")
    } else {
        None
    }
}

/// The framing words a plate may take from a scene's `camera`: shot size and
/// angle, from a closed set. A camera field can describe a person, where
/// the shot dwells on someone, and a plate given those words drew a stranger
/// the placing pass then kept (mecha-a3, 2026-10-10); the closed set is what
/// keeps every person word out.
const FRAMING: [&str; 16] = [
    "extreme close-up",
    "medium close-up",
    "close-up",
    "medium shot",
    "wide shot",
    "full shot",
    "long shot",
    "establishing shot",
    "eye level",
    "eye-level",
    "low angle",
    "high angle",
    "overhead",
    "bird's-eye view",
    "dutch angle",
    "wide angle",
];

/// The framing in `camera`, from [`FRAMING`] only, longest match first.
pub fn framing(camera: &str) -> Option<String> {
    let lower = camera.to_lowercase();
    let found: Vec<&str> = FRAMING
        .iter()
        .copied()
        .filter(|f| lower.contains(f))
        .collect();
    let kept: Vec<&str> = found
        .iter()
        .copied()
        .filter(|f| !found.iter().any(|g| g != f && g.contains(f)))
        .collect();
    (!kept.is_empty()).then(|| kept.join(", "))
}

/// Words that put a person in a sentence. A plate's light that holds one,
/// or a cast member's name, is left out for neutral light.
const PERSON_WORDS: [&str; 24] = [
    "her", "his", "him", "she", "he", "they", "them", "their", "person", "people", "woman",
    "women", "man", "men", "girl", "boy", "body", "skin", "face", "hair", "back", "backside",
    "chest", "legs",
];

/// Whether `text` names nobody: no person word and none of `names`. A name
/// is matched whole, hyphens and all: a library name may hold one, and a
/// word split on it never saw `jean-luc` (review of #626).
pub fn names_no_one(text: &str, names: &[String]) -> bool {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .map(|w| w.trim_end_matches("'s"))
        .all(|w| !PERSON_WORDS.contains(&w))
        && names.iter().all(|n| word_spans(text, n).is_empty())
}

/// The setting's words a plate may show: its clauses that name nobody,
/// or `None` when every clause does. Light and camera were filtered from
/// the start, and the setting went in whole: "her coat over the chair"
/// can draw a figure from behind, which no face check catches, so the word
/// filter is the guard here as it is for light (review of #626).
pub fn room_words(setting: &str, names: &[String]) -> Option<String> {
    let kept: Vec<&str> = setting
        .split([',', ';', '.'])
        .map(str::trim)
        .filter(|c| !c.is_empty() && names_no_one(c, names))
        .collect();
    (!kept.is_empty()).then(|| kept.join(", "))
}

/// The plate: the room alone, in the scene's framing (§15.3, step 1).
/// `light` and `camera` must already be free of people ([`names_no_one`],
/// [`framing`]): the caller passes them through those. No style reaches the
/// plate: a library style can be a portrait recipe (skin, pores, focus on
/// the eyes), and on the plate it drew a person, so 22 of 63 layered
/// renders fell back at the plate check (mecha-a3's gate v1). The style is
/// laid on by the finish alone.
pub fn plate_prompt(setting: &str, light: Option<&str>, camera: Option<&str>) -> String {
    let mut s = closed(setting);
    for part in [light, camera].into_iter().flatten() {
        if !part.trim().is_empty() {
            s.push(' ');
            s.push_str(&closed(part));
        }
    }
    s.push_str(" No people.");
    s
}

/// A cutout: the person from their portrait, whole, in a neutral pose, on
/// a transparent background (§15.3, step 2, mecha-a3's change 1: the act is
/// carried by the placing pass, since a solo cutout naming someone absent
/// invites a second figure).
pub fn cutout_prompt(wearing: &str, light: &str) -> String {
    format!(
        "This is an RGBA image with transparency. A full-length realistic photograph of the \
         person in the image, wearing {}, standing, full length, arms relaxed, lit by {}. The \
         image has alpha channel and the background is transparent.",
        wearing.trim().trim_end_matches('.'),
        light.trim().trim_end_matches('.')
    )
}

/// A cutout posed in the person's part, as mecha-a3 first measured it: a
/// replay arm only (`--layers-posed`), against the neutral cutout.
pub fn cutout_prompt_posed(wearing: &str, part: &str, light: &str) -> String {
    format!(
        "This is an RGBA image with transparency. A full-length realistic photograph of the \
         person in the image, {}, {}, lit by {}. The image has alpha channel and the background \
         is transparent.",
        wearing.trim().trim_end_matches('.'),
        part.trim().trim_end_matches('.'),
        light.trim().trim_end_matches('.')
    )
}

/// The finish, with a library style's words when the scene has one.
pub fn finish_prompt(style: Option<&str>) -> String {
    match style.filter(|s| !s.trim().is_empty()) {
        Some(style) => format!("{FINISH} {}", closed(style)),
        None => FINISH.to_string(),
    }
}

/// The placing pass: the plate as `<image1>`, each person by their tag
/// with their part, then what of the act is left (§15.3, step 3). Names
/// never reach the prompt: each is replaced by its person's tag.
pub fn placing_prompt(people: &[Person], leftover: Option<&str>) -> String {
    let Placing {
        tags,
        parts,
        beside,
    } = placing(people);
    // Everyone who must appear, named before any part (mecha-a3: the
    // person whose part acts on the other, placed second, was left out 6 of
    // 6 on one call).
    let all: Vec<&str> = tags.iter().map(|(_, t)| t.as_str()).collect();
    let mut s = match all.as_slice() {
        [one, two] => format!("Both people are in the picture: {one} and {two}. "),
        [rest @ .., last] => format!(
            "All {} people are in the picture: {} and {last}. ",
            all.len(),
            rest.join(", ")
        ),
        [] => String::new(),
    };
    for (i, (p, (_, tag))) in people.iter().zip(&tags).enumerate() {
        let expression = untagged(&p.expression, &tags);
        let expression = expression.trim().trim_end_matches('.');
        let part = match (parts[i].as_str(), expression) {
            (part, "") => part.to_string(),
            ("", expression) => expression.to_string(),
            (part, expression) => format!("{part}, {expression}"),
        };
        let part = part.as_str();
        let mut at = match beside[i] {
            Some(j) => format!(" beside {}", tags[j].1),
            None => p.at.as_deref().map(|a| format!(" {a}")).unwrap_or_default(),
        };
        if parts[i].is_empty() && at.is_empty() {
            at = " in the scene".to_string();
        }
        if part.is_empty() {
            s.push_str(&format!("Place {tag}{at}. "));
        } else {
            s.push_str(&format!("Place {tag}{at}, {part}. "));
        }
    }
    if let Some(left) = leftover.map(|l| untagged(l, &tags)) {
        if !left.trim().is_empty() {
            s.push_str(&closed(&left));
            s.push(' ');
        }
    }
    s.push_str(
        "Keep <image1>'s room, framing, camera angle and light unchanged. Take each person's \
         face, hair, body and clothing from their own image; each appears exactly once. Lit by \
         the room's light, with natural contact and shadows.",
    );
    s
}

/// How the placing pass names and places each person: their tag, their part
/// with every name swapped for a tag, and whom they are placed beside.
struct Placing {
    tags: Vec<(String, String)>,
    parts: Vec<String>,
    beside: Vec<Option<usize>>,
}

fn placing(people: &[Person]) -> Placing {
    let tags: Vec<(String, String)> = people
        .iter()
        .enumerate()
        .map(|(i, p)| (p.shown.clone(), format!("the person from <image{}>", i + 2)))
        .collect();
    let parts: Vec<String> = people
        .iter()
        .map(|p| {
            untagged(&p.part, &tags)
                .trim()
                .trim_end_matches('.')
                .to_string()
        })
        .collect();
    // Whom each part acts on: the first other person it names.
    let acts_on: Vec<Option<usize>> = parts
        .iter()
        .enumerate()
        .map(|(i, part)| (0..tags.len()).find(|&j| j != i && part.contains(&tags[j].1)))
        .collect();
    // A part that acts on someone is placed beside them, not on a side of
    // the frame: "on the right" stood the actor apart and drew the act on his
    // own body, 3 of 3 (mecha-a3, 2026-10-10). The person acted on keeps
    // their side as the anchor; when two parts act on each other, the first
    // does.
    let beside = (0..people.len())
        .map(|i| acts_on[i].filter(|&j| acts_on[j] != Some(i) || j < i))
        .collect();
    Placing {
        tags,
        parts,
        beside,
    }
}

/// Whether two people the placing pass sets on a side of the frame share
/// one. Someone placed beside another person has no side of their own, so
/// two who act on each other may share a place: an embrace's split answers
/// "centre" for both, about 9% of two-person splits, and that is right
/// (mecha-a3). Two who do not would share one place tag (review of #624).
pub fn shares_a_place(people: &[Person]) -> bool {
    let beside = placing(people).beside;
    let sides: Vec<&str> = people
        .iter()
        .zip(&beside)
        .filter(|(_, b)| b.is_none())
        .filter_map(|(p, _)| p.at.as_deref())
        .collect();
    sides
        .iter()
        .enumerate()
        .any(|(i, a)| sides[..i].contains(a))
}

/// The finish: one keep-everything pass of light, depth of field and detail
/// (§15.3, step 4).
pub const FINISH: &str = "Keep everything in <image1> as it is. Relight the people with the \
room's light so their skin tones, highlights and shadows match the room, with natural contact \
shadows. Shot on an 85 mm lens at f/1.8: the people sharp, the room behind them softly out of \
focus. Add natural fine detail to skin, hair and fabric. Change nothing else.";

/// `text` with each person's name (and its possessive) replaced by their
/// tag, whole words only, any case.
fn untagged(text: &str, tags: &[(String, String)]) -> String {
    let mut out = text.to_string();
    let mut by_length: Vec<&(String, String)> = tags.iter().collect();
    by_length.sort_by_key(|(n, _)| std::cmp::Reverse(n.len()));
    for (name, tag) in by_length {
        out = replace_word(&out, name, tag);
    }
    out
}

/// Replace `word` where it stands as a whole word, any case.
fn replace_word(text: &str, word: &str, with: &str) -> String {
    let mut out = String::new();
    let mut at = 0;
    for (start, end) in word_spans(text, word) {
        out.push_str(&text[at..start]);
        out.push_str(with);
        at = end;
    }
    out.push_str(&text[at..]);
    out
}

/// Where `word` stands in `text` as a whole word, in any ASCII case. Names
/// are ASCII (`imagelib::validate_name`), and ASCII lowercasing keeps every
/// byte where it was, so no text makes the match give up: a full lowercase
/// changed byte lengths outside ASCII, and the guard then left every name
/// in place (review of #626).
fn word_spans(text: &str, word: &str) -> Vec<(usize, usize)> {
    let needle = word.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let lower = text.to_ascii_lowercase();
    let mut spans = Vec::new();
    let mut at = 0;
    while let Some(found) = lower[at..].find(&needle) {
        let start = at + found;
        let end = start + needle.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        if !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric) {
            spans.push((start, end));
        }
        at = end;
    }
    spans
}

fn closed(s: &str) -> String {
    let s = s.trim();
    if s.ends_with(['.', '!', '?']) {
        s.to_string()
    } else {
        format!("{s}.")
    }
}

/// A cutout flattened onto mid-grey (128), as a PNG: the encoder is never
/// handed a transparent reference (§15.3, step 2). A cutout with almost
/// nothing opaque in it is refused: there is nobody to place.
pub fn flatten_on_grey(png: &[u8]) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(png)
        .map_err(|e| format!("the cutout did not read: {e}"))?
        .to_rgba8();
    let opaque = img.pixels().filter(|p| p.0[3] > 127).count();
    if (opaque as f64) < img.pixels().len() as f64 * 0.01 {
        return Err("the cutout came back empty".into());
    }
    let flat = image::RgbImage::from_fn(img.width(), img.height(), |x, y| {
        let [r, g, b, a] = img.get_pixel(x, y).0;
        let a = u16::from(a);
        let mix = |c: u8| ((u16::from(c) * a + 128 * (255 - a) + 127) / 255) as u8;
        image::Rgb([mix(r), mix(g), mix(b)])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    flat.write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| format!("the cutout could not be written: {e}"))?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(shown: &str, part: &str) -> Person {
        Person {
            key: shown.to_lowercase(),
            shown: shown.into(),
            wearing: "a coat".into(),
            part: part.into(),
            expression: String::new(),
            at: None,
            portrait: Vec::new(),
            ext: "png",
        }
    }

    /// A name is matched whole, hyphens and all, and no text makes the match
    /// give up: a hyphenated library name passed the plate's filter, and a
    /// letter whose lowercase is longer left every name in a part (review
    /// of #626).
    #[test]
    fn names_are_found_whole_in_any_text() {
        let names = ["Jean-luc".to_string()];
        assert!(!names_no_one("jean-luc's scarf over a chair", &names));
        assert!(names_no_one("a jean jacket on a hook", &names));
        let tags = [("Maya".to_string(), "the person from <image2>".to_string())];
        assert_eq!(
            untagged("Maya walks through İzmir", &tags),
            "the person from <image2> walks through İzmir"
        );
        assert_eq!(untagged("Mayan ruins", &tags), "Mayan ruins");
    }

    /// The plate keeps the setting's clauses that name nobody, and has no
    /// room in words when every clause names someone (review of #626).
    #[test]
    fn the_plate_keeps_only_the_setting_that_names_nobody() {
        let names = ["Maya".to_string()];
        assert_eq!(
            room_words("a quiet harbour at dusk, Maya's coat over a rail", &names).as_deref(),
            Some("a quiet harbour at dusk")
        );
        assert_eq!(room_words("her flat; Maya's sofa", &names), None);
    }

    /// An expression is said with its person's part, or alone when the
    /// split gave no part, with names as tags (review of #626).
    #[test]
    fn the_placing_prompt_carries_each_expression() {
        let mut maya = person("Maya", "sitting on a bench with a map");
        maya.expression = "laughing at John".into();
        let mut john = person("John", "");
        john.expression = "grinning.".into();
        let s = placing_prompt(&[maya, john], None);
        assert!(
            s.contains("with a map, laughing at the person from <image3>. "),
            "{s}"
        );
        assert!(
            s.contains("Place the person from <image3> in the scene, grinning. "),
            "{s}"
        );
    }

    /// A plate takes framing from a closed set and no person words: a camera
    /// dwelling on someone drew a stranger into the empty room.
    #[test]
    fn the_plate_takes_framing_and_light_that_name_nobody() {
        assert_eq!(
            framing("Medium shot, low angle, lingering on Maya's scarf").as_deref(),
            Some("medium shot, low angle")
        );
        assert_eq!(
            framing("a medium close-up at eye level").as_deref(),
            Some("medium close-up, eye level")
        );
        assert_eq!(framing("handheld, intimate"), None);
        let names = ["Maya".to_string()];
        assert!(names_no_one(
            "warm lamplight, dusk outside the window",
            &names
        ));
        assert!(!names_no_one("a lamp warming his collar", &names));
        assert!(!names_no_one("a glow along Maya's sleeve", &names));
    }

    #[test]
    fn a_picture_is_layered_only_when_every_part_can_be_built() {
        assert_eq!(not_layered(2, 2, 2, true, true, true), None);
        assert!(not_layered(2, 1, 1, true, true, true)
            .unwrap()
            .contains("described"));
        assert!(not_layered(2, 2, 1, true, true, true)
            .unwrap()
            .contains("portrait"));
        assert!(not_layered(2, 2, 2, false, true, true)
            .unwrap()
            .contains("setting"));
        assert!(not_layered(2, 2, 2, true, false, true)
            .unwrap()
            .contains("place"));
        assert!(not_layered(6, 6, 6, true, true, false)
            .unwrap()
            .contains("one edit"));
        assert!(not_layered(3, 3, 3, true, true, true)
            .unwrap()
            .contains("two people"));
    }

    /// Names never reach the placing prompt: each becomes its person's tag,
    /// possessives and any case included, and a name inside another word is
    /// left alone.
    #[test]
    fn the_placing_prompt_names_people_by_their_tags() {
        let people = [
            person("Maya", "lifting John off the ground"),
            person("John", "with his arms around Maya's shoulders"),
        ];
        let p = placing_prompt(&people, Some("They laugh, and maya's scarf slips"));
        assert!(
            p.starts_with(
                "Both people are in the picture: the person from <image2> and the person from \
                 <image3>. Place the person from <image2>, lifting the person from <image3> off the \
                 ground. Place the person from <image3> beside the person from <image2>, with his \
                 arms around the person from <image2>'s shoulders. They laugh, and the person from <image2>'s scarf slips."
            ),
            "{p}"
        );
        assert!(!p.contains("Maya") && !p.contains("John"), "{p}");
        assert!(p.ends_with("with natural contact and shadows."));
        assert_eq!(
            replace_word("Johnson and John", "John", "X"),
            "Johnson and X"
        );
        // A person with no part is still placed.
        let p = placing_prompt(&[person("Maya", ""), person("John", "")], None);
        assert!(
            p.contains("Place the person from <image2> in the scene."),
            "{p}"
        );
        // Three people are all named before any part.
        let p = placing_prompt(
            &[person("Maya", ""), person("John", ""), person("Wren", "")],
            None,
        );
        assert!(
            p.starts_with(
                "All 3 people are in the picture: the person from <image2>, the person from \
                 <image3> and the person from <image4>."
            ),
            "{p}"
        );
        // A place, when the split gave one, is said.
        let mut left = person("Maya", "");
        left.at = Some("on the left".into());
        let mut right = person("John", "waving");
        right.at = Some("on the right".into());
        let p = placing_prompt(&[left, right], None);
        assert!(
            p.contains(
                "Place the person from <image2> on the left. Place the person from <image3> on \
                 the right, waving."
            ),
            "{p}"
        );
        // A part acting on someone is placed beside them, not on a side: the
        // one acted on keeps theirs as the anchor, whatever the order.
        let mut acted_on = person("Maya", "sitting on a bench with a map");
        acted_on.at = Some("in the centre".into());
        let mut actor = person("John", "straightening Maya's collar");
        actor.at = Some("on the right".into());
        let p = placing_prompt(&[actor, acted_on], None);
        assert!(
            p.contains(
                "Place the person from <image2> beside the person from <image3>, straightening \
                 the person from <image3>'s collar. Place the person from <image3> in the \
                 centre, sitting on a bench with a map."
            ),
            "{p}"
        );
        assert!(!p.contains("on the right"), "{p}");
    }

    /// People who act on each other may share a place, since only one of
    /// them keeps a side; two who do not would share a side's tag.
    #[test]
    fn a_shared_place_matters_only_between_people_on_their_own_sides() {
        let at = |mut p: Person, a: &str| {
            p.at = Some(a.into());
            p
        };
        let hug = [
            at(person("Maya", "with her arms around John"), "in the centre"),
            at(person("John", "holding Maya close"), "in the centre"),
        ];
        assert!(!shares_a_place(&hug));
        let one_way = [
            at(person("Maya", "reading"), "in the centre"),
            at(
                person("John", "reading over Maya's shoulder"),
                "in the centre",
            ),
        ];
        assert!(!shares_a_place(&one_way));
        let apart = [
            at(person("Maya", "reading"), "in the centre"),
            at(person("John", "waving"), "in the centre"),
        ];
        assert!(shares_a_place(&apart));
        let sides = [
            at(person("Maya", "reading"), "on the left"),
            at(person("John", "waving"), "on the right"),
        ];
        assert!(!shares_a_place(&sides));
    }

    #[test]
    fn the_plate_and_the_cutout_say_what_was_measured() {
        assert_eq!(
            plate_prompt(
                "a kitchen with a long oak table",
                Some("late sun"),
                Some("eye level")
            ),
            "a kitchen with a long oak table. late sun. eye level. No people."
        );
        // A style's words go on the finish alone, never the plate.
        assert!(finish_prompt(Some("ink and wash")).ends_with("Change nothing else. ink and wash."));
        assert_eq!(finish_prompt(None), FINISH);
        let posed = cutout_prompt_posed("a coat", "lifting the other person", "dusk");
        assert!(
            posed.contains("a coat, lifting the other person, lit by dusk."),
            "{posed}"
        );
        let c = cutout_prompt("a green coat.", "late sun");
        assert!(c.contains(
            "wearing a green coat, standing, full length, arms relaxed, lit by late sun."
        ));
        assert!(c.starts_with("This is an RGBA image with transparency."));
    }

    /// A cutout is flattened on mid-grey, its opaque pixels kept; one with
    /// nothing opaque is refused.
    #[test]
    fn a_cutout_is_flattened_on_grey_and_an_empty_one_refused() {
        let mut rgba = image::RgbaImage::from_pixel(10, 10, image::Rgba([0, 0, 0, 0]));
        for x in 0..10 {
            rgba.put_pixel(x, 5, image::Rgba([200, 10, 10, 255]));
        }
        let mut png = std::io::Cursor::new(Vec::new());
        rgba.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let flat = image::load_from_memory(&flatten_on_grey(png.get_ref()).unwrap())
            .unwrap()
            .to_rgb8();
        assert_eq!(flat.get_pixel(0, 0).0, [128, 128, 128]);
        assert_eq!(flat.get_pixel(3, 5).0, [200, 10, 10]);
        let empty = image::RgbaImage::from_pixel(10, 10, image::Rgba([0, 0, 0, 0]));
        let mut png = std::io::Cursor::new(Vec::new());
        empty.write_to(&mut png, image::ImageFormat::Png).unwrap();
        assert!(flatten_on_grey(png.get_ref())
            .unwrap_err()
            .contains("empty"));
    }
}
