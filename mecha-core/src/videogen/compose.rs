//! The prompt H3 is sent, in the sectioned shape it was trained on
//! (MiniMax's prompt-writing guides for full-reference and keyframe clips).
//!
//! The split is the image compiler's (`imagelib::compile`): the model writes
//! what varies — the shot timeline, the speakers, the sound — and this code
//! writes what persists: who each reference is, what of them is kept, whose
//! voice each audio clip is. A name in the model's text becomes its
//! `<Subject N>` label everywhere but inside dialogue, so a line that says a
//! name is still said.
//!
//! A model that wrote H3's sections itself (`detailed_description:` or
//! `integrated_multimodal_description:` in its text) is sent what it wrote,
//! labelled and with its lines closed. The call's `soundscape` and `music`,
//! and a reference prompt's pin sentences, are dropped on that path on
//! purpose: the sections it wrote are the ones that would carry them.

/// One reference image, as the prompt describes it.
pub struct Person {
    /// The library name, for a character; `None` for a picture from the chat.
    pub name: Option<String>,
    /// The library's stored description.
    pub description: Option<String>,
    /// Its `<Audio j>` slot, numbered from 1, when it has a voice here.
    pub audio: Option<usize>,
    /// The picture is a crop of the face alone: everything below it comes
    /// from the description.
    pub face_only: bool,
}

/// What the model wrote, and the clip's length.
pub struct Parts<'a> {
    pub prompt: &'a str,
    pub soundscape: Option<&'a str>,
    pub music: Option<&'a str>,
    /// The effective length, frames / fps.
    pub seconds: f64,
}

/// Said when the model gives no soundscape: the guide keeps `N/A` for a
/// clip that is silent on purpose, which a clip without a soundscape is not.
const DEFAULT_SOUNDSCAPE: &str =
    "Natural ambient sound that fits the setting, with the physical sounds of the actions described.";

/// `maya` → `Maya`, `mara-lee` → `Mara Lee`: a library name as a prompt says it.
pub fn title_case(name: &str) -> String {
    name.split('-')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn field(text: Option<&str>) -> Option<String> {
    text.map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

/// A full-reference prompt: the six sections, in order.
pub fn references(parts: &Parts, people: &[Person], pins: (bool, bool)) -> String {
    let named: Vec<(String, usize)> = people
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.name.clone().map(|n| (n, i + 1)))
        .collect();
    let labelled = closed_lines(&label_names(parts.prompt.trim(), &named));
    // A model that wrote the sections itself is sent what it wrote, labelled.
    if labelled.contains("detailed_description:") {
        return labelled;
    }
    let mut body = with_first_shot(&labelled);
    // A pinned frame, said where it applies (the guide's "the shot begins
    // from …"); it carries no <Picture> label, so it takes no slot. After the
    // tag, whatever follows it: a model that wrote `[Shot 1]` and a newline
    // lost the sentence to a match on the tag and a space (review of #732).
    if pins.0 {
        body = body.replacen(
            "[Shot 1]",
            "[Shot 1] The shot begins exactly on the provided opening frame and continues from it.",
            1,
        );
    }
    if pins.1 {
        body.push_str(" The final shot ends exactly on the provided closing frame.");
    }
    let ids = speakers(&body, people.len());

    let mut defs = Vec::new();
    for (i, p) in people.iter().enumerate() {
        let n = i + 1;
        // What the picture gives: the face alone for a crop, the person for
        // a portrait. Clothing is the description's either way.
        let from = if p.face_only {
            "whose face comes from"
        } else {
            "whose face, hair, skin and build come from"
        };
        defs.push(match (&p.name, &p.description) {
            (Some(name), Some(text)) => format!(
                "<Subject {n}> is {}, {from} <Picture {n}>: {text}. The clothing is not taken from <Picture {n}>; it is what the description gives.",
                title_case(name)
            ),
            (Some(name), None) => format!(
                "<Subject {n}> is {}, {from} <Picture {n}>. The clothing is not taken from <Picture {n}>; it is what the description gives.",
                title_case(name)
            ),
            (None, _) => format!("<Subject {n}> is what <Picture {n}> shows, kept as it appears there."),
        });
    }
    let voiced: Vec<(usize, usize)> = people
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.audio.map(|j| (j, i + 1)))
        .collect();
    for &(j, n) in &voiced {
        let id = ids
            .get(n)
            .and_then(|s| s.as_deref())
            .map(|s| format!(" ({s})"))
            .unwrap_or_default();
        defs.push(format!(
            "<Audio {j}> is the voice-timbre reference for <Subject {n}>{id}."
        ));
    }

    let subjects: Vec<String> = (1..=people.len())
        .map(|n| format!("<Subject {n}>"))
        .collect();
    let mut summary = format!(
        "[{}reference generation{}] The target video is a {:.1}-second clip featuring {}{}{}.",
        if pins.0 || pins.1 {
            "keyframe completion + "
        } else {
            ""
        },
        if voiced.is_empty() {
            ""
        } else {
            " + audio reference"
        },
        parts.seconds,
        crate::imagelib::join_and(&subjects),
        if pins.0 {
            ", starting from the provided opening frame"
        } else {
            ""
        },
        if pins.1 {
            ", ending on the provided closing frame"
        } else {
            ""
        },
    );
    if !voiced.is_empty() {
        let pairs: Vec<String> = voiced
            .iter()
            .map(|(j, n)| format!("<Audio {j}> for <Subject {n}>"))
            .collect();
        summary.push_str(&format!(
            " Voice timbres come from {}.",
            crate::imagelib::join_and(&pairs)
        ));
    }

    let mut kept = Vec::new();
    for (i, p) in people.iter().enumerate() {
        let n = i + 1;
        kept.push(match &p.name {
            Some(name) if p.face_only => format!(
                "<Subject {n}> (appears in the clip): partially_preserved - {}'s face from <Picture {n}> is retained; hair, build and the rest follow the stored description, and the clothing follows the scene.",
                title_case(name)
            ),
            Some(name) => format!(
                "<Subject {n}> (appears in the clip): partially_preserved - {}'s face, hair, skin and build from <Picture {n}> are retained; the clothing in <Picture {n}> is replaced by the clothing in the description.",
                title_case(name)
            ),
            None => format!("<Subject {n}> (appears in the clip): fully_preserved - what <Picture {n}> shows is retained."),
        });
    }
    for &(j, n) in &voiced {
        kept.push(format!(
            "<Audio {j}>: reference - the speech of <Subject {n}> follows the voice timbre of <Audio {j}> without copying its words."
        ));
    }

    format!(
        "subject_definitions:\n{}\n\nsummary:\n{summary}\n\nretention_analysis:\n{}\n\ndetailed_description:\n{body}\n\noverall_soundscape:\n{}\n\nnon_diegetic_music:\n{}",
        defs.join("\n"),
        kept.join("\n"),
        field(parts.soundscape).unwrap_or_else(|| DEFAULT_SOUNDSCAPE.to_string()),
        field(parts.music).unwrap_or_else(|| "N/A".to_string()),
    )
}

/// A text or keyframe prompt (T2VA / I2VA / FL2VA / L2VA): the alignment
/// line the guide fixes for the frames given, then the three core fields.
/// `in_words` describes the library characters who cannot be shown here.
pub fn frames(parts: &Parts, first: bool, last: bool, in_words: &[(String, String)]) -> String {
    // A model that wrote the sections itself is sent what it wrote, with
    // its lines closed as `references` closes them (review of #732).
    if parts.prompt.contains("integrated_multimodal_description:") {
        return closed_lines(parts.prompt.trim());
    }
    let mut body = with_first_shot(&closed_lines(parts.prompt.trim()));
    for (name, text) in in_words {
        body.push_str(&format!(" {} looks like this: {text}.", title_case(name)));
    }
    let last_shot = shot_count(&body).max(1);
    let secs = format!("{:.2}", parts.seconds);
    let instruction = match (first, last) {
        (true, true) => Some(format!(
            "How the reference pictures align with the target video — Picture 1 (from Shot 1) aligns with the 0.00-second mark of the target video; Picture 2 (from Shot {last_shot}) aligns with the {secs}-second mark of the target video."
        )),
        (true, false) => Some(
            "For the target video, at 0.00 seconds into the target video, <Picture 1> (from [Shot 1]) is fully referenced.".to_string(),
        ),
        (false, true) => Some(format!(
            "How the reference pictures align with the target video — <Picture 1> (from [Shot {last_shot}]) aligns with the {secs}-second mark of the target video."
        )),
        (false, false) => None,
    };
    let fields = format!(
        "integrated_multimodal_description: {body}\n\noverall_soundscape: {}\n\nnon_diegetic_music: {}",
        field(parts.soundscape).unwrap_or_else(|| DEFAULT_SOUNDSCAPE.to_string()),
        field(parts.music).unwrap_or_else(|| "N/A".to_string()),
    );
    match instruction {
        Some(line) => format!("{line}\n\n{fields}"),
        None => fields,
    }
}

/// `[Shot 1]` opens the timeline. A prompt without one gets it at the very
/// start, ahead of the style line too.
fn with_first_shot(text: &str) -> String {
    if text.contains("[Shot 1]") {
        return text.to_string();
    }
    format!("[Shot 1] {text}")
}

fn shot_count(text: &str) -> usize {
    (1..=64)
        .take_while(|n| text.contains(&format!("[Shot {n}]")))
        .count()
}

/// Every spoken line ends with `.`, `?` or `!` before `</d>`: the guide's
/// rule, and an open line is one of the reported causes of mumbled speech.
pub fn closed_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut rest = text;
    while let Some(end) = rest.find("</d>") {
        let line = rest[..end].trim_end();
        out.push_str(line);
        if !line.ends_with(['.', '?', '!', '…']) && !line.ends_with("<d>") {
            out.push('.');
        }
        out.push_str("</d>");
        rest = &rest[end + 4..];
    }
    out.push_str(rest);
    out
}

/// Each library name, outside `<d>…</d>`, as its `<Subject N>` label: a
/// whole word, any ASCII case, a hyphenated name also in its spaced form.
pub fn label_names(text: &str, named: &[(String, usize)]) -> String {
    let mut out = String::with_capacity(text.len() + 32);
    let mut rest = text;
    while !rest.is_empty() {
        let (outside, inside) = match rest.find("<d>") {
            Some(open) => {
                let close = rest[open..]
                    .find("</d>")
                    .map(|c| open + c + 4)
                    .unwrap_or(rest.len());
                (&rest[..open], &rest[open..close])
            }
            None => (rest, ""),
        };
        out.push_str(&label_outside(outside, named));
        out.push_str(inside);
        rest = &rest[outside.len() + inside.len()..];
    }
    out
}

fn label_outside(text: &str, named: &[(String, usize)]) -> String {
    // Longest form first: in cast order, a name that is a whole word of a
    // longer one (`ana` beside `ana-sofia`) took its place in "Ana Sofia",
    // and the longer name's label never appeared (review of #732). Once
    // replaced, the longer name is a label the shorter cannot match.
    let mut forms: Vec<(String, usize)> = named
        .iter()
        .flat_map(|(name, n)| [(name.clone(), *n), (name.replace('-', " "), *n)])
        .collect();
    forms.sort_by_key(|(form, _)| std::cmp::Reverse(form.len()));
    forms.dedup();
    let mut text = text.to_string();
    for (form, n) in forms {
        text = replace_word(&text, &form, &format!("<Subject {n}>"));
    }
    text
}

fn replace_word(text: &str, word: &str, with: &str) -> String {
    // An empty word is found at every offset and never advances.
    if word.is_empty() {
        return text.to_string();
    }
    let lower = text.to_ascii_lowercase();
    let word = word.to_ascii_lowercase();
    let is_word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(found) = lower[at..].find(&word) {
        let start = at + found;
        let end = start + word.len();
        let bounded =
            !is_word(text[..start].chars().next_back()) && !is_word(text[end..].chars().next());
        out.push_str(&text[at..start]);
        out.push_str(if bounded { with } else { &text[start..end] });
        at = end;
    }
    out.push_str(&text[at..]);
    out
}

/// The speaker ID each subject is first given, `<Subject N> (Sx)`, indexed by
/// subject number. A compound `(S1,S2)` names no one subject and is skipped.
fn speakers(text: &str, count: usize) -> Vec<Option<String>> {
    let mut ids = vec![None; count + 1];
    for (n, slot) in ids.iter_mut().enumerate().skip(1) {
        let tag = format!("<Subject {n}> (S");
        if let Some(at) = text.find(&tag) {
            let after = &text[at + tag.len()..];
            let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
            if !digits.is_empty() && after[digits.len()..].starts_with(')') {
                *slot = Some(format!("S{digits}"));
            }
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(prompt: &str) -> Parts<'_> {
        Parts {
            prompt,
            soundscape: Some("Rain on a tin roof."),
            music: None,
            seconds: 6.0,
        }
    }

    #[test]
    fn every_spoken_line_is_closed() {
        assert_eq!(
            closed_lines("A says, <d>[English] Good morning</d> B says, <d>[English] Really?</d>"),
            "A says, <d>[English] Good morning.</d> B says, <d>[English] Really?</d>"
        );
    }

    #[test]
    fn names_become_labels_but_never_inside_dialogue() {
        let named = [("maya".to_string(), 1), ("mara-lee".to_string(), 2)];
        assert_eq!(
            label_names(
                "Maya (S1) says, <d>[English] Hi Mara Lee!</d> Mara Lee waves at MAYA.",
                &named
            ),
            "<Subject 1> (S1) says, <d>[English] Hi Mara Lee!</d> <Subject 2> waves at <Subject 1>."
        );
        assert_eq!(label_names("Mayapple jam.", &named), "Mayapple jam.");
    }

    /// A name that is a whole word of a longer one does not take its place
    /// (review of #732), and an empty name changes nothing rather than
    /// looping.
    #[test]
    fn a_longer_name_keeps_its_own_label() {
        let named = [("ana".to_string(), 1), ("ana-sofia".to_string(), 2)];
        assert_eq!(
            label_names("Ana Sofia waves at Ana.", &named),
            "<Subject 2> waves at <Subject 1>."
        );
        assert_eq!(
            label_names("Ana waves.", &[(String::new(), 1)]),
            "Ana waves."
        );
    }

    /// A keyframe prompt the model sectioned itself still has its lines
    /// closed, as a reference prompt's are (review of #732).
    #[test]
    fn a_sectioned_frames_prompt_still_has_its_lines_closed() {
        let p = "integrated_multimodal_description:\nMaya says, <d>[English] Hello</d>";
        assert_eq!(
            frames(&parts(p), false, false, &[]),
            "integrated_multimodal_description:\nMaya says, <d>[English] Hello.</d>"
        );
    }

    #[test]
    fn a_full_reference_prompt_has_the_six_sections_and_binds_voices_to_speakers() {
        let people = [
            Person {
                name: Some("maya".into()),
                description: Some("Tall, with short grey hair".into()),
                audio: Some(1),
                face_only: false,
            },
            Person {
                name: Some("wren".into()),
                description: None,
                audio: Some(2),
                face_only: true,
            },
        ];
        let out = references(
            &parts(
                "Live-action. Maya (S1) says, <d>[English] The kettle is on.</d> Wren (S2) laughs.",
            ),
            &people,
            (false, false),
        );
        let order = [
            "subject_definitions:",
            "summary:",
            "retention_analysis:",
            "detailed_description:",
            "overall_soundscape:",
            "non_diegetic_music:",
        ];
        let at: Vec<usize> = order
            .iter()
            .map(|s| out.find(s).unwrap_or_else(|| panic!("{s} missing:\n{out}")))
            .collect();
        assert!(at.windows(2).all(|w| w[0] < w[1]), "{out}");
        assert!(out.contains("<Subject 1> is Maya, whose face, hair, skin and build come from <Picture 1>: Tall, with short grey hair. The clothing is not taken from <Picture 1>"), "{out}");
        assert!(out.contains("partially_preserved - Maya's face, hair, skin and build from <Picture 1> are retained; the clothing in <Picture 1> is replaced"), "{out}");
        assert!(!out.contains("fully_preserved - Maya"), "{out}");
        // A face crop claims the face alone.
        assert!(
            out.contains("<Subject 2> is Wren, whose face comes from <Picture 2>."),
            "{out}"
        );
        assert!(
            out.contains("Wren's face from <Picture 2> is retained; hair, build"),
            "{out}"
        );
        assert!(
            out.contains("<Audio 1> is the voice-timbre reference for <Subject 1> (S1)."),
            "{out}"
        );
        assert!(
            out.contains("<Audio 2> is the voice-timbre reference for <Subject 2> (S2)."),
            "{out}"
        );
        assert!(
            out.contains("[reference generation + audio reference]"),
            "{out}"
        );
        assert!(
            out.contains("[Shot 1] Live-action. <Subject 1> (S1) says"),
            "{out}"
        );
        assert!(out.ends_with("non_diegetic_music:\nN/A"), "{out}");
        let pinned = references(&parts("Maya waves."), &people, (true, false));
        assert!(
            pinned.contains("[keyframe completion + reference generation + audio reference]"),
            "{pinned}"
        );
        assert!(
            pinned.contains("[Shot 1] The shot begins exactly on the provided opening frame and continues from it. <Subject 1> waves."),
            "{pinned}"
        );
        // A newline after the tag keeps the sentence too (review of #732).
        let pinned = references(&parts("[Shot 1]\nMaya waves."), &people, (true, false));
        assert!(
            pinned.contains("[Shot 1] The shot begins exactly on the provided opening frame and continues from it.\n<Subject 1> waves."),
            "{pinned}"
        );
    }

    #[test]
    fn keyframe_prompts_carry_the_guides_alignment_lines() {
        let p = Parts {
            prompt: "[Shot 1] A slow walk. [Shot 2] At 00:02.750, a glance back.",
            soundscape: None,
            music: Some("Piano."),
            seconds: 5.166,
        };
        let both = frames(&p, true, true, &[]);
        assert!(both.starts_with("How the reference pictures align with the target video — Picture 1 (from Shot 1) aligns with the 0.00-second mark of the target video; Picture 2 (from Shot 2) aligns with the 5.17-second mark of the target video.\n\nintegrated_multimodal_description: [Shot 1]"), "{both}");
        assert!(frames(&p, true, false, &[]).starts_with("For the target video, at 0.00 seconds into the target video, <Picture 1> (from [Shot 1]) is fully referenced."));
        assert!(frames(&p, false, true, &[]).starts_with("How the reference pictures align with the target video — <Picture 1> (from [Shot 2]) aligns with the 5.17-second mark"));
        let text = frames(
            &Parts {
                prompt: "A harbour at dawn.",
                soundscape: None,
                music: None,
                seconds: 5.0,
            },
            false,
            false,
            &[("maya".into(), "Tall, with short grey hair".into())],
        );
        assert!(text.starts_with("integrated_multimodal_description: [Shot 1] A harbour at dawn. Maya looks like this: Tall, with short grey hair."), "{text}");
        assert!(text.contains("non_diegetic_music: N/A"), "{text}");
    }
}
