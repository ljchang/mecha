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
    /// The clip is seen through this person's eyes (`scene` cast `pov`):
    /// their face never appears, only their hands and body.
    pub pov: bool,
    /// Not a person: a library location, whose picture is the setting (V-L,
    /// ruled passed 2026-10-25).
    pub place: bool,
    /// Not a person: a library outfit, worn by the cast members named here
    /// (as the scene wrote them). Its picture is the garment alone (the
    /// owner's ruling, 2026-10-25).
    pub worn_by: Option<Vec<String>>,
}

/// A second picture of a library character, sent after every subject's own
/// (`IMAGE-COMPILER-DESIGN.md` §9.4): its `<Picture k>` and what it shows.
pub struct Extra {
    /// The subject it belongs to, numbered from 1.
    pub subject: usize,
    /// Its `<Picture k>`, numbered from 1.
    pub picture: usize,
    pub kind: ExtraKind,
}

pub enum ExtraKind {
    /// The whole person: sent unbound, "the person in <Picture n> and
    /// <Picture k>" (V-B's retest, owner 2026-10-14).
    Body,
    /// A labelled mark, shown close up (V-D, owner 2026-10-26: a tattoo
    /// picture kept the tattoo on one forearm where words alone put it on
    /// both).
    Detail(String),
}

impl Person {
    /// Neither a place nor clothes: someone the clip features.
    fn is_person(&self) -> bool {
        !self.place && self.worn_by.is_none()
    }
}

/// Everyone who wears an outfit, as the prompt names them, joined.
fn wearers(people: &[Person], who: &[String]) -> String {
    let named: Vec<String> = who.iter().map(|w| wearer(people, w)).collect();
    crate::imagelib::join_and(&named)
}

/// Who wears an outfit, as the prompt names them: their subject label when
/// they are a reference, else the words the scene used.
fn wearer(people: &[Person], who: &str) -> String {
    let key = crate::imagelib::spelled_as_name(who);
    people
        .iter()
        .position(|p| {
            p.is_person()
                && p.name.as_deref().map(crate::imagelib::spelled_as_name) == Some(key.clone())
        })
        .map(|i| format!("<Subject {}>", i + 1))
        .unwrap_or_else(|| who.to_string())
}

/// A description that mentions a tattoo says it is the only one: H3 puts a
/// described mark on whatever skin is in frame, and a forearm tattoo came out
/// on a lower leg in a clip whose legs filled the frame (2026-10-25).
fn only_marks(description: &str, who: &str) -> String {
    let lower = description.to_ascii_lowercase();
    // "No tattoos" names none, so "no other" would imply one somewhere.
    if lower.contains("tattoo") && !lower.contains("no tattoo") {
        format!(" No tattoo appears anywhere else on {who}'s body.")
    } else {
        String::new()
    }
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

/// A full-reference prompt: the six sections, in order. `extras` are the
/// library characters' second pictures, numbered after every subject's own.
pub fn references(
    parts: &Parts,
    people: &[Person],
    extras: &[Extra],
    pins: (bool, bool),
) -> String {
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
        // A place is defined as one: "what <Picture n> shows" read as a
        // person's line, and the room carried only because the setting text
        // named the picture by hand (2026-10-25).
        // Clothes are defined as clothes, worn: a garment picture read as a
        // person's would be a stranger, or a swimsuit drawn lying on a bench.
        if let Some(who) = &p.worn_by {
            let what = p.name.as_deref().map(title_case).unwrap_or_default();
            let on = wearers(people, who);
            let text = p
                .description
                .as_deref()
                .map(|t| format!(": {t}"))
                .unwrap_or_default();
            defs.push(format!(
                "<Subject {n}> is the outfit {what} in <Picture {n}>, worn by {on}{text}. It is clothing, never a person; its colour, cut and details are kept as <Picture {n}> shows them."
            ));
            continue;
        }
        if p.place {
            let what = p.name.as_deref().map(title_case).unwrap_or_default();
            let text = p
                .description
                .as_deref()
                .map(|t| format!(": {t}"))
                .unwrap_or_default();
            defs.push(format!(
                "<Subject {n}> is the location {what}, shown in <Picture {n}>{text}. The clip takes place there: its layout, fixtures, colours and light are kept as <Picture {n}> shows them."
            ));
            continue;
        }
        // What the picture gives: the face alone for a crop, the person for
        // a portrait. Clothing is the description's either way.
        let from = if p.face_only {
            "whose face comes from"
        } else {
            "whose face, hair, skin and build come from"
        };
        // Seen through, never seen: the picture gives the hands and body in
        // view, and the face is said to stay out. A clip seen through a
        // library character's eyes said their face "appears in the clip"
        // while the action said it never would (2026-10-25).
        // A face crop shows skin tone and nothing of the build, so a pov
        // person from one is promised only that (review of #738).
        let from = match (p.pov, p.face_only) {
            (true, true) => "whose skin tone comes from",
            (true, false) => "whose skin and build come from",
            (false, _) => from,
        };
        let unseen = if p.pov {
            format!(" The clip is seen through <Subject {n}>'s eyes, so <Subject {n}>'s face never appears.")
        } else {
            String::new()
        };
        let body = extras
            .iter()
            .find(|e| e.subject == n && matches!(e.kind, ExtraKind::Body))
            .map(|e| e.picture);
        // With a body, the two pictures are one person, unbound: no
        // face-from/build-from split, the wording V-B's retest passed with.
        let pictured = match body {
            Some(k) => format!("the person in <Picture {n}> and <Picture {k}>"),
            None => format!("{from} <Picture {n}>"),
        };
        let not_from = match body {
            Some(_) => "The clothing is not taken from either picture".to_string(),
            None => format!("The clothing is not taken from <Picture {n}>"),
        };
        let marks: String = extras
            .iter()
            .filter(|e| e.subject == n)
            .filter_map(|e| match &e.kind {
                ExtraKind::Detail(label) => Some(format!(
                    " <Subject {n}>'s {label} is shown close up in <Picture {}>.",
                    e.picture
                )),
                ExtraKind::Body => None,
            })
            .collect();
        defs.push(match (&p.name, &p.description) {
            (Some(name), Some(text)) => format!(
                "<Subject {n}> is {}, {pictured}: {text}.{}{marks} {not_from}; it is what the description gives.{unseen}",
                title_case(name),
                only_marks(text, &format!("<Subject {n}>"))
            ),
            (Some(name), None) => format!(
                "<Subject {n}> is {}, {pictured}.{marks} {not_from}; it is what the description gives.{unseen}",
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

    // People are featured; a place is where it is set, never one of them
    // (review of #740).
    let subjects: Vec<String> = (1..=people.len())
        .filter(|n| people[n - 1].is_person())
        .map(|n| format!("<Subject {n}>"))
        .collect();
    let places: Vec<String> = (1..=people.len())
        .filter(|n| people[n - 1].place)
        .map(|n| format!("<Subject {n}>"))
        .collect();
    let outfits: Vec<String> = (1..=people.len())
        .filter(|n| people[n - 1].worn_by.is_some())
        .map(|n| format!("<Subject {n}>"))
        .collect();
    // Each kind of subject said for what it is: people featured, a place
    // the clip is set in, clothes worn in it. A clip of only described
    // people and a location read "featuring , set in …" (review of #740),
    // and one whose only reference was a garment read "with no reference
    // subject" (review of #741).
    let mut parts_said = Vec::new();
    if !subjects.is_empty() {
        parts_said.push(format!(
            "featuring {}",
            crate::imagelib::join_and(&subjects)
        ));
    }
    if !places.is_empty() {
        parts_said.push(format!("set in {}", crate::imagelib::join_and(&places)));
    }
    if !outfits.is_empty() {
        parts_said.push(format!("with {} worn", crate::imagelib::join_and(&outfits)));
    }
    let what = if parts_said.is_empty() {
        "with no reference subject".to_string()
    } else {
        parts_said.join(", ")
    };
    let mut summary = format!(
        "[{}reference generation{}] The target video is a {:.1}-second clip {}{}{}.",
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
        what,
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
        let body = extras
            .iter()
            .find(|e| e.subject == n && matches!(e.kind, ExtraKind::Body))
            .map(|e| e.picture);
        // Each mark shown close up is kept where it is and nowhere else: the
        // wording V-D measured.
        let marks: String = extras
            .iter()
            .filter(|e| e.subject == n)
            .filter_map(|e| match &e.kind {
                ExtraKind::Detail(label) => Some(format!(
                    "; the {label} in <Picture {}> is kept exactly where it is, and nowhere else",
                    e.picture
                )),
                ExtraKind::Body => None,
            })
            .collect();
        kept.push(match &p.name {
            _ if p.worn_by.is_some() => format!(
                "<Subject {n}> (worn by {on} throughout): fully_preserved - the garment in <Picture {n}> is what {on} {wear} in every shot, exactly as it appears there; it is never shown on its own.",
                on = wearers(people, p.worn_by.as_deref().unwrap_or_default()),
                wear = if p.worn_by.as_ref().is_some_and(|w| w.len() > 1) { "wear" } else { "wears" }
            ),
            _ if p.place => format!(
                "<Subject {n}> (the setting): fully_preserved - the place in <Picture {n}> is where the clip happens, kept as it appears there."
            ),
            Some(name) if p.pov => format!(
                "<Subject {n}> (seen only as the camera's own body): partially_preserved - {}'s {} from <Picture {n}> {} retained for the hands, arms and body in view{}{marks}; the face is never shown, and the clothing follows the scene.",
                title_case(name),
                if p.face_only { "skin tone" } else { "skin and build" },
                if p.face_only { "is" } else { "are" },
                if p.face_only { ", the build follows the stored description" } else { "" },
            ),
            Some(name) if body.is_some() => format!(
                "<Subject {n}> (appears in the clip): partially_preserved - {}'s face, hair, skin and build from <Picture {n}> and <Picture {}> are retained{marks}; the clothing in them is replaced by the clothing in the description.",
                title_case(name),
                body.unwrap_or(n)
            ),
            Some(name) if p.face_only => format!(
                "<Subject {n}> (appears in the clip): partially_preserved - {}'s face from <Picture {n}> is retained{marks}; hair, build and the rest follow the stored description, and the clothing follows the scene.",
                title_case(name)
            ),
            Some(name) => format!(
                "<Subject {n}> (appears in the clip): partially_preserved - {}'s face, hair, skin and build from <Picture {n}> are retained{marks}; the clothing in <Picture {n}> is replaced by the clothing in the description.",
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
        let who = title_case(name);
        body.push_str(&format!(
            " {who} looks like this: {text}.{}",
            only_marks(text, &who)
        ));
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

/// Each library name, outside `<d>…</d>` and outside quoted speech, as its
/// `<Subject N>` label: a whole word, any ASCII case, a hyphenated name also
/// in its spaced form. Quotes are protected as `<d>` is: a model that wrote a
/// line inside an action, quoting a library name it addressed, had the name
/// inside the words replaced by a label (2026-10-25).
pub fn label_names(text: &str, named: &[(String, usize)]) -> String {
    let mut out = String::with_capacity(text.len() + 32);
    let mut rest = text;
    while !rest.is_empty() {
        let (outside, inside) = match protected(rest) {
            Some((open, close)) => (&rest[..open], &rest[open..close]),
            None => (rest, ""),
        };
        out.push_str(&label_outside(outside, named));
        out.push_str(inside);
        rest = &rest[outside.len() + inside.len()..];
    }
    out
}

/// The first span in `text` a label must not enter, as byte offsets: a
/// `<d>…</d>` line (to the end when unclosed, as H3 reads it), or a quote
/// (`quote_spans`).
fn protected(text: &str) -> Option<(usize, usize)> {
    let line = text.find("<d>").map(|open| {
        let close = text[open..]
            .find("</d>")
            .map(|c| open + c + 4)
            .unwrap_or(text.len());
        (open, close)
    });
    let quote = quote_spans(text).into_iter().next();
    match (line, quote) {
        (Some(l), Some(q)) => Some(if q.0 < l.0 { q } else { l }),
        (l, q) => l.or(q),
    }
}

/// Every quote in `text`, straight or curly, as byte spans including the
/// marks. A mark opens only at the start or after whitespace or an opening
/// bracket, colon or dash, with no space after it; it closes only with no
/// space before it and at the end or before whitespace or punctuation. So an
/// inch mark (`6" ruler`) opens nothing, an unclosed quote is no span, and a
/// stray mark never pairs with the next real quote and swallows the words
/// between (review of #742).
pub(crate) fn quote_spans(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find(['"', '\u{201c}']) {
        let open = from + i;
        let mark = text[open..].chars().next().unwrap_or('"');
        // A curly quote closes curly or straight: local models mix them.
        let close_with: &[char] = if mark == '"' {
            &['"']
        } else {
            &['\u{201d}', '"']
        };
        let body = open + mark.len_utf8();
        let before = text[..open].chars().next_back();
        let after = text[body..].chars().next();
        let opens = before.is_none_or(|c| c.is_whitespace() || "([:\u{2014}\u{2013}".contains(c))
            && after.is_some_and(|c| !c.is_whitespace());
        if !opens {
            from = body;
            continue;
        }
        // The first mark that can close it does; one that can only open
        // first means this quote never closed, and it pairs with nothing,
        // so it cannot swallow the words up to a later quote (review of
        // #742).
        let mut search = body;
        let mut found = None;
        while let Some(c) = text[search..].find(['"', '\u{201c}', '\u{201d}']) {
            let at = search + c;
            let here = text[at..].chars().next().unwrap_or('"');
            let end = at + here.len_utf8();
            let prev = text[..at].chars().next_back();
            let next = text[end..].chars().next();
            let closes = close_with.contains(&here)
                && prev.is_some_and(|p| !p.is_whitespace())
                && next.is_none_or(|n| {
                    n.is_whitespace()
                        || n.is_ascii_punctuation()
                        || "\u{2014}\u{2013}\u{2026}".contains(n)
                });
            if closes {
                found = Some(end);
                break;
            }
            let opens_again = here != '\u{201d}'
                && prev.is_none_or(|p| p.is_whitespace() || "([:\u{2014}\u{2013}".contains(p))
                && next.is_some_and(|n| !n.is_whitespace());
            if opens_again {
                break;
            }
            search = end;
        }
        match found {
            Some(end) => {
                out.push((open, end));
                from = end;
            }
            None => from = body,
        }
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
        // Quoted speech keeps its names, straight or curly; a stray quote
        // protects nothing after it.
        assert_eq!(
            label_names(
                "Maya turns to Mara Lee: \"Thanks, Maya.\" Mara Lee nods.",
                &named
            ),
            "<Subject 1> turns to <Subject 2>: \"Thanks, Maya.\" <Subject 2> nods."
        );
        assert_eq!(
            label_names(
                "Maya says \u{201c}Over here, Mara Lee\u{201d} to Mara Lee.",
                &named
            ),
            "<Subject 1> says \u{201c}Over here, Mara Lee\u{201d} to <Subject 2>."
        );
        assert_eq!(
            label_names("A 6\" ruler; Maya waves.", &named),
            "A 6\" ruler; <Subject 1> waves."
        );
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
                pov: false,
                place: false,
                worn_by: None,
            },
            Person {
                name: Some("wren".into()),
                description: None,
                audio: Some(2),
                face_only: true,
                pov: false,
                place: false,
                worn_by: None,
            },
        ];
        let out = references(
            &parts(
                "Live-action. Maya (S1) says, <d>[English] The kettle is on.</d> Wren (S2) laughs.",
            ),
            &people,
            &[],
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
        let pinned = references(&parts("Maya waves."), &people, &[], (true, false));
        assert!(
            pinned.contains("[keyframe completion + reference generation + audio reference]"),
            "{pinned}"
        );
        assert!(
            pinned.contains("[Shot 1] The shot begins exactly on the provided opening frame and continues from it. <Subject 1> waves."),
            "{pinned}"
        );
        // A newline after the tag keeps the sentence too (review of #732).
        let pinned = references(&parts("[Shot 1]\nMaya waves."), &people, &[], (true, false));
        assert!(
            pinned.contains("[Shot 1] The shot begins exactly on the provided opening frame and continues from it.\n<Subject 1> waves."),
            "{pinned}"
        );
    }

    /// The person the clip is seen through is defined and kept as a body
    /// with no face, and a described tattoo is said to be the only one.
    #[test]
    fn the_camera_person_has_no_face_and_a_tattoo_is_the_only_one() {
        let people = [
            Person {
                name: Some("maya".into()),
                description: Some("Tall, with a small tattoo on her left wrist".into()),
                audio: Some(1),
                face_only: true,
                pov: false,
                place: false,
                worn_by: None,
            },
            Person {
                name: Some("wren".into()),
                description: Some("Broad-shouldered".into()),
                audio: None,
                face_only: true,
                pov: true,
                place: false,
                worn_by: None,
            },
        ];
        let out = references(
            &parts("Maya looks up at Wren."),
            &people,
            &[],
            (false, false),
        );
        assert!(
            out.contains(
                "left wrist. No tattoo appears anywhere else on <Subject 1>'s body. The clothing"
            ),
            "{out}"
        );
        // From a face crop: skin tone only, the build from the words
        // (review of #738).
        assert!(out.contains("<Subject 2> is Wren, whose skin tone comes from <Picture 2>: Broad-shouldered. The clothing is not taken from <Picture 2>; it is what the description gives. The clip is seen through <Subject 2>'s eyes, so <Subject 2>'s face never appears."), "{out}");
        assert!(out.contains("<Subject 2> (seen only as the camera's own body): partially_preserved - Wren's skin tone from <Picture 2> is retained for the hands, arms and body in view, the build follows the stored description; the face"), "{out}");
        // From a whole portrait: skin and build.
        let whole = [Person {
            name: Some("wren".into()),
            description: Some("Broad-shouldered".into()),
            audio: None,
            face_only: false,
            pov: true,
            place: false,
            worn_by: None,
        }];
        let out2 = references(&parts("Wren reaches out."), &whole, &[], (false, false));
        assert!(
            out2.contains("<Subject 1> is Wren, whose skin and build come from <Picture 1>"),
            "{out2}"
        );
        assert!(out2.contains("Wren's skin and build from <Picture 1> are retained for the hands, arms and body in view; the face"), "{out2}");
        assert!(!out.contains("<Subject 2> (appears in the clip)"), "{out}");
        assert!(!out.contains("anywhere else on <Subject 2>"), "{out}");
        // "No tattoos" claims none, so nothing is added.
        assert_eq!(only_marks("Freckled, no tattoos", "Maya"), "");
        let words = frames(
            &parts("A beach."),
            false,
            false,
            &[("maya".into(), "A rose tattoo on one ankle".into())],
        );
        assert!(words.contains("Maya looks like this: A rose tattoo on one ankle. No tattoo appears anywhere else on Maya's body."), "{words}");
    }

    /// A library location is defined and kept as the setting, never as a
    /// person who appears; with no library person beside it, the clip is
    /// only set there.
    #[test]
    fn a_location_is_the_setting_not_a_person() {
        let alone = [Person {
            name: Some("boat-house".into()),
            description: None,
            audio: None,
            face_only: false,
            pov: false,
            place: true,
            worn_by: None,
        }];
        let out = references(&parts("A man rows past."), &alone, &[], (false, false));
        assert!(out.contains("-second clip set in <Subject 1>."), "{out}");
        assert!(!out.contains("featuring"), "{out}");
        let people = [
            Person {
                name: Some("maya".into()),
                description: None,
                audio: None,
                face_only: true,
                pov: false,
                place: false,
                worn_by: None,
            },
            Person {
                name: Some("boat-house".into()),
                description: Some("A timber room with oars on the wall".into()),
                audio: None,
                face_only: false,
                pov: false,
                place: true,
                worn_by: None,
            },
        ];
        let out = references(
            &parts("Maya waits. The scene takes place in Boat House."),
            &people,
            &[],
            (false, false),
        );
        assert!(out.contains("<Subject 2> is the location Boat House, shown in <Picture 2>: A timber room with oars on the wall. The clip takes place there"), "{out}");
        assert!(out.contains("<Subject 2> (the setting): fully_preserved - the place in <Picture 2> is where the clip happens"), "{out}");
        assert!(!out.contains("<Subject 2> (appears in the clip)"), "{out}");
        assert!(
            out.contains("The scene takes place in <Subject 2>.")
                && out.contains("clip featuring <Subject 1>, set in <Subject 2>."),
            "{out}"
        );
    }

    /// An outfit is clothes worn by the person it names, kept in every shot,
    /// and never one of who the clip features.
    #[test]
    fn an_outfit_is_worn_not_featured() {
        let people = [
            Person {
                name: Some("maya".into()),
                description: None,
                audio: None,
                face_only: true,
                pov: false,
                place: false,
                worn_by: None,
            },
            Person {
                name: Some("red-raincoat".into()),
                description: Some("A red hooded raincoat".into()),
                audio: None,
                face_only: false,
                pov: false,
                place: false,
                worn_by: Some(vec!["Maya".into()]),
            },
        ];
        let out = references(
            &parts("Maya wears Red Raincoat and waves."),
            &people,
            &[],
            (false, false),
        );
        assert!(out.contains("<Subject 2> is the outfit Red Raincoat in <Picture 2>, worn by <Subject 1>: A red hooded raincoat. It is clothing, never a person"), "{out}");
        assert!(out.contains("<Subject 2> (worn by <Subject 1> throughout): fully_preserved - the garment in <Picture 2> is what <Subject 1> wears in every shot"), "{out}");
        assert!(
            out.contains("clip featuring <Subject 1>, with <Subject 2> worn."),
            "{out}"
        );
        // Worn by someone described in words, the garment is still said
        // (review of #741).
        let alone = [Person {
            name: Some("red-raincoat".into()),
            description: None,
            audio: None,
            face_only: false,
            pov: false,
            place: false,
            worn_by: Some(vec!["a lifeguard".into()]),
        }];
        let worn_only = references(&parts("A lifeguard waves."), &alone, &[], (false, false));
        assert!(
            worn_only.contains("-second clip with <Subject 1> worn."),
            "{worn_only}"
        );
        assert!(worn_only.contains("worn by a lifeguard"), "{worn_only}");
        assert!(
            out.contains("<Subject 1> wears <Subject 2> and waves."),
            "{out}"
        );
    }

    /// A body binds a second picture to the person, unbound; a detail is
    /// said close up and kept where it is, nowhere else.
    #[test]
    fn a_body_and_a_detail_are_the_same_person() {
        let people = [Person {
            name: Some("maya".into()),
            description: Some("Tall".into()),
            audio: None,
            face_only: true,
            pov: false,
            place: false,
            worn_by: None,
        }];
        let extras = [
            Extra {
                subject: 1,
                picture: 2,
                kind: ExtraKind::Body,
            },
            Extra {
                subject: 1,
                picture: 3,
                kind: ExtraKind::Detail("rose tattoo, right ankle".into()),
            },
        ];
        let out = references(&parts("Maya waves."), &people, &extras, (false, false));
        assert!(out.contains("<Subject 1> is Maya, the person in <Picture 1> and <Picture 2>: Tall. <Subject 1>'s rose tattoo, right ankle is shown close up in <Picture 3>. The clothing is not taken from either picture"), "{out}");
        assert!(out.contains("Maya's face, hair, skin and build from <Picture 1> and <Picture 2> are retained; the rose tattoo, right ankle in <Picture 3> is kept exactly where it is, and nowhere else; the clothing in them is replaced"), "{out}");
        // A detail alone, beside a face crop.
        let out = references(&parts("Maya waves."), &people, &extras[1..], (false, false));
        assert!(out.contains("whose face comes from <Picture 1>: Tall. <Subject 1>'s rose tattoo, right ankle is shown close up in <Picture 3>."), "{out}");
        assert!(out.contains("Maya's face from <Picture 1> is retained; the rose tattoo, right ankle in <Picture 3> is kept exactly where it is"), "{out}");
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
