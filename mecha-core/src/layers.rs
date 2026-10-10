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
    /// The parts are prose from the scene's `together`: its origin.
    pub origin: crate::scene::Origin,
}

/// Why a picture the switch would layer is drawn in one pass instead, or
/// `None` when it qualifies. Only touching scenes are asked about at all.
pub fn not_layered(
    people: usize,
    library_people: usize,
    with_portraits: usize,
    setting_words: bool,
    style: bool,
) -> Option<&'static str> {
    if library_people < people {
        Some("someone in it is described in words, not drawn from the library")
    } else if with_portraits < people {
        Some("someone in it has no library portrait")
    } else if !setting_words {
        Some("it has no setting in words to build the room from")
    } else if style {
        Some("a style is not yet built in layers")
    } else {
        None
    }
}

/// The plate: the room alone, in the scene's framing (§15.3, step 1).
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

/// The placing pass: the plate as `<image1>`, each person by their tag
/// with their part, then what of the act is left (§15.3, step 3). Names
/// never reach the prompt: each is replaced by its person's tag.
pub fn placing_prompt(people: &[Person], leftover: Option<&str>) -> String {
    let tags: Vec<(String, String)> = people
        .iter()
        .enumerate()
        .map(|(i, p)| (p.shown.clone(), format!("the person from <image{}>", i + 2)))
        .collect();
    let mut s = String::new();
    for (p, (_, tag)) in people.iter().zip(&tags) {
        let part = untagged(&p.part, &tags);
        let part = part.trim().trim_end_matches('.');
        if part.is_empty() {
            s.push_str(&format!("Place {tag} in the scene. "));
        } else {
            s.push_str(&format!("Place {tag}, {part}. "));
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
    if word.trim().is_empty() {
        return text.to_string();
    }
    let lower = text.to_lowercase();
    let needle = word.to_lowercase();
    // Lowercasing can change byte lengths outside ASCII; then leave it.
    if lower.len() != text.len() {
        return text.to_string();
    }
    let mut out = String::new();
    let mut at = 0;
    while let Some(found) = lower[at..].find(&needle) {
        let start = at + found;
        let end = start + needle.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        let bounded =
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric);
        out.push_str(&text[at..start]);
        out.push_str(if bounded { with } else { &text[start..end] });
        at = end;
    }
    out.push_str(&text[at..]);
    out
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
            portrait: Vec::new(),
            ext: "png",
        }
    }

    #[test]
    fn a_picture_is_layered_only_when_every_part_can_be_built() {
        assert_eq!(not_layered(2, 2, 2, true, false), None);
        assert!(not_layered(2, 1, 1, true, false)
            .unwrap()
            .contains("described"));
        assert!(not_layered(2, 2, 1, true, false)
            .unwrap()
            .contains("portrait"));
        assert!(not_layered(2, 2, 2, false, false)
            .unwrap()
            .contains("setting"));
        assert!(not_layered(2, 2, 2, true, true).unwrap().contains("style"));
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
                "Place the person from <image2>, lifting the person from <image3> off the \
                 ground. Place the person from <image3>, with his arms around the person from \
                 <image2>'s shoulders. They laugh, and the person from <image2>'s scarf slips."
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
            p.starts_with("Place the person from <image2> in the scene."),
            "{p}"
        );
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
