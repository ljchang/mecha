//! A typed scene, compiled to the shot timeline [`super::compose`] takes as
//! its `detailed_description`.
//!
//! The owner says something loose in the chat; the chat model fills these
//! fields; this turns them into the timeline H3 follows. Typed are only the
//! parts this code acts on: who is in it and what they wear (clothes must be
//! said — a portrait's never carry over), when shots cut, who speaks and in
//! what order. Everything descriptive — the action, the framing, the
//! delivery, the style that carries the mood — is the model's own prose (the
//! owner's ruling of 2026-10-13: mood rides in the style line, untyped).
//!
//! Problems with the timing are notes, said back with the clip, never a
//! refusal: a draft that runs is worth more than a lecture.

use anyhow::{bail, Result};
use serde_json::{json, Value};

/// Spoken words per second H3 fits comfortably (the guide's rule of thumb).
const WORDS_PER_SECOND: f64 = 2.5;

pub struct Compiled {
    /// The shot timeline, names as written (labels come later).
    pub prompt: String,
    pub soundscape: Option<String>,
    pub music: Option<String>,
    pub seconds: Option<f64>,
    pub aspect: Option<String>,
    /// Timing and wording problems, said back with the clip.
    pub notes: Vec<String>,
    /// The cast members the clip is seen through (`pov`), as written.
    pub pov: Vec<String>,
    /// The library location the scene names (`setting.location`), as
    /// written: `plan` sends its picture as the setting.
    pub location: Option<String>,
    /// Those of them with no line kept: their voice is a slot H3 may give
    /// words to. Speakers are matched as the compiler numbers them, in any
    /// case, so a line `who: "wren"` keeps Wren's voice (review of #738).
    pub silent_pov: Vec<String>,
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn sentence(s: &str) -> String {
    let s = s.trim();
    if s.ends_with(['.', '!', '?']) {
        s.to_string()
    } else {
        format!("{s}.")
    }
}

/// A sentence the scene opens with its first letter up: `flat overhead
/// light` stood lower-case mid-timeline after a full stop.
fn capital(s: &str) -> String {
    let mut c = s.trim().chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// One person however a name is spelled: any case, and hyphens, underscores
/// and spaces alike, as the library keys names (`args::library_key`). A pov
/// speaker cast as "Mara Lee" and given lines as `mara-lee` was classed
/// silent and lost her voice (review of #738).
fn same_name(a: &str, b: &str) -> bool {
    let words = |s: &str| -> Vec<String> {
        s.split(|c: char| c == '-' || c == '_' || c.is_whitespace())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect()
    };
    words(a) == words(b)
}

/// Five-letter-and-longer words that end in -ing without being a verb's
/// -ing form: "bring her a towel" read "is bring her a towel", and `doing:
/// "nothing"` read "is nothing" (review of #738).
const NOT_PROGRESSIVE: [&str; 10] = [
    "bring",
    "swing",
    "cling",
    "fling",
    "sting",
    "string",
    "nothing",
    "something",
    "anything",
    "everything",
];

/// A `doing` that opens on an -ing word takes `is`: "Luke wears nothing and
/// lying on his back" was the compiled sentence (2026-10-25); "kneels beside
/// an upturned bike" reads as written.
fn as_predicate(doing: &str) -> String {
    let first = doing.split_whitespace().next().unwrap_or("");
    let word: String = first.chars().filter(|c| c.is_alphabetic()).collect();
    let lower = word.to_lowercase();
    if word.chars().count() > 4
        && lower.ends_with("ing")
        && !NOT_PROGRESSIVE.contains(&lower.as_str())
    {
        format!("is {}", lower_first(doing))
    } else {
        doing.to_string()
    }
}

/// Words in an action that say someone talks there: where a line without a
/// `shot` belongs. A clip whose second shot read "…gazes into the camera and
/// speaks" had both its lines compiled into the first (2026-10-25). Only
/// words that are speech in nearly any sentence: "calls" and "answers" are
/// as often a kettle or a door (review of #738).
const SPEECH_CUES: [&str; 6] = ["says", "speaks", "asks", "whispers", "replies", "murmurs"];

fn speech_cues(action: &str) -> usize {
    action
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| SPEECH_CUES.contains(&w.to_ascii_lowercase().as_str()))
        .count()
}

/// Words that make a quote lettering rather than speech: a sign that reads
/// "NO ENTRY" is not a line.
const LETTERING: [&str; 9] = [
    "reads",
    "read",
    "sign",
    "label",
    "labelled",
    "written",
    "printed",
    "caption",
    "lettering",
];

/// The forms a cast name is written in: as given, and hyphens as spaces.
fn name_forms(name: &str) -> Vec<String> {
    let lower = name.trim().to_ascii_lowercase();
    let spaced = lower.replace(['-', '_'], " ");
    if spaced == lower {
        vec![lower]
    } else {
        vec![lower, spaced]
    }
}

/// Each closed quote in `text`, straight or curly: its byte span and the
/// words inside.
fn quotes(text: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find(['"', '\u{201c}']) {
        let open = from + i;
        let close_with = if text[open..].starts_with('"') {
            '"'
        } else {
            '\u{201d}'
        };
        let body = open + text[open..].chars().next().map_or(1, char::len_utf8);
        let Some(c) = text[body..].find(close_with) else {
            break;
        };
        let end = body + c + close_with.len_utf8();
        out.push((open, end, text[body..body + c].trim().to_string()));
        from = end;
    }
    out
}

/// A scene as a model wrote it, mended where its words say more than its
/// fields, each mend said in a note (2026-10-25: the local model's first
/// scene through the built-in tool did both):
///
/// - **The camera person.** A shot that says "Luke's POV" (or point of
///   view, eyes, perspective) makes Luke `pov`, unless the scene set `pov`
///   for him either way. Unset, his face was described and kept as someone
///   who appears.
/// - **Speech written inside an action.** With no `lines` at all, a quote of
///   two words or more in an action, after "<name> says/speaks/…", becomes
///   that person's line in that shot, and leaves the action. A line written
///   there had no speaker tag, so no voice and no lip sync, and a name inside
///   it was labelled. A quote with no named speaker before it is left alone,
///   unless one cue word stands before it and only one cast member can speak.
fn repair(scene: &Value, known: &[String]) -> (Value, Vec<String>) {
    let mut out = scene.clone();
    let mut notes = Vec::new();
    let mut cast: Vec<String> = scene
        .get("cast")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|c| text(c, "name")).collect())
        .unwrap_or_default();
    let shot_words: String = scene
        .get("shots")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .flat_map(|s| ["framing", "action", "camera"].map(|k| text(s, k)))
                .flatten()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
        .to_lowercase()
        .replace('\u{2019}', "'");

    // A library character the shots name but `cast` leaves out is a person
    // in the clip all the same: the first real scene listed no cast at all
    // and named both people in its prose (2026-10-25).
    let mentioned: Vec<String> = known
        .iter()
        .filter(|k| {
            name_forms(k).iter().any(|f| {
                shot_words.match_indices(f.as_str()).any(|(at, _)| {
                    let before = shot_words[..at].chars().next_back();
                    let after = shot_words[at + f.len()..].chars().next();
                    !before.is_some_and(char::is_alphanumeric)
                        && !after.is_some_and(char::is_alphanumeric)
                })
            })
        })
        .filter(|k| !cast.iter().any(|c| same_name(c, k)))
        .map(|k| super::compose::title_case(k))
        .collect();
    for name in &mentioned {
        let seen_through = name_forms(name).iter().any(|f| {
            [
                "pov",
                "point of view",
                "point-of-view",
                "eyes",
                "perspective",
            ]
            .iter()
            .any(|w| shot_words.contains(&format!("{f}'s {w}")))
        });
        if seen_through {
            if !out["cast"].is_array() {
                out["cast"] = json!([]);
            }
            if let Some(members) = out["cast"].as_array_mut() {
                members.push(json!({"name": name, "pov": true}));
            }
            notes.push(format!(
                "The shots say the clip is seen through {name}'s eyes, so {name} was taken as the camera (`pov`). List them in `cast` with `pov` yourself to say otherwise."
            ));
        }
    }
    cast.extend(mentioned);

    if let Some(members) = out.get_mut("cast").and_then(Value::as_array_mut) {
        for member in members.iter_mut() {
            if member.get("pov").is_some_and(|v| !v.is_null()) {
                continue;
            }
            let Some(name) = text(member, "name") else {
                continue;
            };
            let seen_through = name_forms(&name).iter().any(|f| {
                [
                    "pov",
                    "point of view",
                    "point-of-view",
                    "eyes",
                    "perspective",
                ]
                .iter()
                .any(|w| shot_words.contains(&format!("{f}'s {w}")))
            });
            if seen_through {
                member["pov"] = json!(true);
                notes.push(format!(
                    "The shots say the clip is seen through {name}'s eyes, so {name} was taken as the camera (`pov`). Set `pov` yourself to say otherwise."
                ));
            }
        }
    }
    let pov: Vec<String> = out
        .get("cast")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|c| c.get("pov").and_then(Value::as_bool) == Some(true))
                .filter_map(|c| text(c, "name"))
                .collect()
        })
        .unwrap_or_default();
    let speakers: Vec<&String> = cast.iter().filter(|n| !pov.contains(n)).collect();

    let has_lines = scene
        .get("lines")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty());
    if has_lines {
        return (out, notes);
    }
    let mut moved = Vec::new();
    if let Some(shots) = out.get_mut("shots").and_then(Value::as_array_mut) {
        for (i, shot) in shots.iter_mut().enumerate() {
            let Some(action) = text(shot, "action") else {
                continue;
            };
            // ASCII-lowered, so its byte offsets are the action's own.
            let lower = action.to_ascii_lowercase();
            let mut kept = String::new();
            let mut last = 0;
            // Where the previous quote ended, and who said it if it moved.
            let mut since = 0;
            let mut speaking: Option<String> = None;
            for (open, end, words) in quotes(&action) {
                let between = &lower[since..open];
                since = end;
                if words.split_whitespace().count() < 2 {
                    speaking = None;
                    continue;
                }
                let names_in = |text: &str| -> Vec<(usize, String)> {
                    cast.iter()
                        .filter_map(|n| {
                            name_forms(n)
                                .iter()
                                .filter_map(|f| text.rfind(f.as_str()))
                                .max()
                                .map(|at| (at, n.clone()))
                        })
                        .collect()
                };
                // The cast member named last before a speech word, since the
                // previous quote.
                let named = cast
                    .iter()
                    .filter_map(|n| {
                        name_forms(n)
                            .iter()
                            .flat_map(|f| {
                                SPEECH_CUES
                                    .iter()
                                    .filter_map(|cue| between.rfind(&format!("{f} {cue}")))
                                    .collect::<Vec<_>>()
                            })
                            .max()
                            .map(|at| (at, n.clone()))
                    })
                    .max_by_key(|(at, _)| *at)
                    .map(|(_, n)| n);
                let words_between: Vec<&str> = between
                    .split(|c: char| !c.is_alphabetic())
                    .filter(|w| !w.is_empty())
                    .collect();
                let lettering = words_between.iter().any(|w| LETTERING.contains(w));
                let cued = words_between.iter().any(|w| SPEECH_CUES.contains(w));
                // "…" She moans. Then: "…" — the same speaker goes on, unless
                // someone else is named or the words are lettering.
                let goes_on = speaking
                    .clone()
                    .filter(|_| !lettering && names_in(between).is_empty());
                let who = named.or(goes_on).or_else(|| {
                    (cued && !lettering && speakers.len() == 1).then(|| speakers[0].clone())
                });
                let Some(who) = who else {
                    speaking = None;
                    continue;
                };
                speaking = Some(who.clone());
                kept.push_str(&action[last..open]);
                last = end;
                moved.push(json!({"who": who, "says": words, "shot": i + 1}));
            }
            if last > 0 {
                kept.push_str(&action[last..]);
                let tidy = kept.split_whitespace().collect::<Vec<_>>().join(" ");
                shot["action"] = json!(tidy.replace(" .", ".").replace(" ,", ","));
            }
        }
    }
    if !moved.is_empty() {
        notes.push(format!(
            "{} spoken line{} written inside `action` became `lines`, so each is voiced and lip-synced; write speech in `lines` yourself.",
            moved.len(),
            if moved.len() == 1 { "" } else { "s" }
        ));
        out["lines"] = json!(moved);
    }
    (out, notes)
}

/// `1.5` → `00:01.500`.
fn stamp(at: f64) -> String {
    let ms = (at * 1000.0).round().max(0.0) as u64;
    format!(
        "{:02}:{:02}.{:03}",
        ms / 60_000,
        (ms / 1000) % 60,
        ms % 1000
    )
}

/// Compile `scene` for a clip of `seconds` (the caller's default when the
/// scene names none). `known` is the library's character names, which a
/// model may name in its prose without listing them in `cast`.
pub fn compile(scene: &Value, default_seconds: f64, known: &[String]) -> Result<Compiled> {
    let (repaired, mut notes) = repair(scene, known);
    let scene = &repaired;
    let seconds = scene.get("seconds").and_then(Value::as_f64);
    let length = seconds.unwrap_or(default_seconds);

    let cast: Vec<&Value> = scene
        .get("cast")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    let shots: Vec<&Value> = scene
        .get("shots")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    let lines: Vec<&Value> = scene
        .get("lines")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    if shots.is_empty() {
        bail!("`scene.shots` needs at least one shot: its framing and action");
    }

    // Speaker IDs in the order people first speak (the guide's rule),
    // counting only the lines that are kept: a line naming a shot the scene
    // does not have is dropped below, and numbering its speaker left the
    // clip with an S2 and no S1 (review of #732).
    let kept = |l: &&&Value| {
        l.get("shot")
            .and_then(Value::as_u64)
            .is_none_or(|k| k >= 1 && k as usize <= shots.len())
    };
    let mut speakers: Vec<String> = Vec::new();
    for l in lines.iter().filter(kept) {
        if let Some(who) = text(l, "who") {
            if !speakers.iter().any(|s| same_name(s, &who)) {
                speakers.push(who);
            }
        }
    }
    let id_of = |who: &str| {
        speakers
            .iter()
            .position(|s| same_name(s, who))
            .map(|i| i + 1)
    };

    // Cut times: the scene's own where given, else evenly spread.
    let n = shots.len();
    let mut times: Vec<f64> = shots
        .iter()
        .enumerate()
        .map(|(i, s)| {
            s.get("at")
                .and_then(Value::as_f64)
                .unwrap_or(length * i as f64 / n as f64)
        })
        .collect();
    times[0] = 0.0;
    for i in 1..n {
        let previous = times[i - 1];
        if times[i] <= previous || times[i] >= length {
            // The even spot when it still falls after the previous cut;
            // otherwise an even share of what is left, so every cut stays
            // after the one before it and inside the clip. The even spot
            // alone could land on or before a cut already kept: two shots
            // asked for at 4.0 s of 6 cut twice at 4.0 s (review of #732).
            let even = length * i as f64 / n as f64;
            let moved = if even > previous && even < length {
                even
            } else {
                previous + (length - previous) / (n - i + 1) as f64
            };
            notes.push(format!(
                "Shot {}'s cut at {:.2} s fell outside the clip or before the previous cut, so it was moved to {:.2} s.",
                i + 1,
                times[i],
                moved
            ));
            times[i] = moved;
        }
    }

    // Each line's shot: its own `shot`, else the next place an action says
    // someone speaks, in order, else the first shot. A cue an explicitly
    // placed line already fills is not offered again.
    let explicit = |l: &Value| {
        l.get("shot")
            .and_then(Value::as_u64)
            .map(|k| k as usize)
            .filter(|&k| k >= 1 && k <= n)
    };
    let mut free: Vec<usize> = shots
        .iter()
        .map(|s| text(s, "action").map_or(0, |a| speech_cues(&a)))
        .collect();
    for l in &lines {
        if let Some(k) = explicit(l) {
            free[k - 1] = free[k - 1].saturating_sub(1);
        }
    }
    let slots: Vec<usize> = free
        .iter()
        .enumerate()
        .flat_map(|(i, &c)| std::iter::repeat_n(i + 1, c))
        .collect();
    // Lines are spoken in the order given, so an inferred line never lands
    // in a shot before the line ahead of it.
    let mut next = 0;
    let mut floor = 1;
    let mut moved = Vec::new();
    let mut placed: Vec<usize> = Vec::with_capacity(lines.len());
    for (j, l) in lines.iter().enumerate() {
        // `shot: null` is no shot: placed by cue, as an omitted one is
        // (review of #738).
        if l.get("shot").is_some_and(|v| !v.is_null()) {
            // Out of range stays out of range: dropped below, with a note.
            let k = l
                .get("shot")
                .and_then(Value::as_u64)
                .map_or(1, |k| k as usize);
            if explicit(l).is_some() {
                floor = floor.max(k);
            }
            placed.push(k);
            continue;
        }
        while slots.get(next).is_some_and(|&s| s < floor) {
            next += 1;
        }
        let shot = slots
            .get(next)
            .copied()
            .unwrap_or_else(|| slots.last().copied().unwrap_or(1).max(floor));
        next += 1;
        floor = shot;
        if shot != 1 {
            moved.push(format!("line {} in shot {shot}", j + 1));
        }
        placed.push(shot);
    }
    if !moved.is_empty() {
        notes.push(format!(
            "Lines without a `shot` went where the actions say someone speaks, or after the line spoken before them: {}. Give `shot` to place a line yourself.",
            moved.join(", ")
        ));
    }

    let pov: Vec<String> = cast
        .iter()
        .filter(|c| c.get("pov").and_then(Value::as_bool) == Some(true))
        .filter_map(|c| text(c, "name"))
        .collect();

    let mut out = String::new();
    if let Some(style) = text(scene, "style") {
        out.push_str(&sentence(&capital(&style)));
        out.push(' ');
    }
    for (i, shot) in shots.iter().enumerate() {
        let framing = text(shot, "framing").unwrap_or_else(|| "a medium shot".into());
        if i == 0 {
            out.push_str(&format!("[Shot 1] {}", sentence(&framing)));
            // The setting and the people, once, at the opening.
            if let Some(setting) = scene.get("setting") {
                // Named, so `compose` labels it as the subject its picture
                // defines; the location's own words go with that definition.
                if let Some(place) = text(setting, "location") {
                    out.push_str(&format!(
                        " The scene takes place in {}.",
                        // As the library keys it, so `compose` can label it
                        // and the plan resolves the same name: `Boat_House`
                        // stayed literal (review of #740).
                        super::compose::title_case(&crate::imagelib::spelled_as_name(&place))
                    ));
                }
                if let Some(d) = text(setting, "description") {
                    out.push(' ');
                    out.push_str(&sentence(&capital(&d)));
                }
                if let Some(l) = text(setting, "light") {
                    out.push(' ');
                    out.push_str(&sentence(&capital(&l)));
                }
            }
            for c in &cast {
                let Some(name) = text(c, "name") else {
                    continue;
                };
                // Seen through, never seen: said before anything else about
                // them, so the hands and body the action names are theirs
                // and no face is drawn for them.
                if c.get("pov").and_then(Value::as_bool) == Some(true) {
                    out.push_str(&format!(
                        " The clip is seen through {name}'s eyes: the camera is {name}'s point of view, so {name}'s face never appears, and only {name}'s hands, arms and body are seen."
                    ));
                }
                let mut said = name.clone();
                match text(c, "wearing") {
                    Some(w) => said.push_str(&format!(" wears {w}")),
                    None => notes.push(format!(
                        "No `wearing` for {name}, so the video model chose the outfit: clothes come from the words alone, not the portrait."
                    )),
                }
                if let Some(d) = text(c, "doing") {
                    let join = if said == name { " " } else { " and " };
                    said.push_str(&format!("{join}{}", as_predicate(&d)));
                }
                if said != name {
                    out.push(' ');
                    out.push_str(&sentence(&said));
                }
                if let Some(extra) = text(c, "notes") {
                    out.push(' ');
                    out.push_str(&sentence(&capital(&extra)));
                }
            }
        } else {
            out.push_str(&format!(
                " [Shot {}] At {}, the shot cuts to {}",
                i + 1,
                stamp(times[i]),
                sentence(&lower_first(&framing))
            ));
        }
        if let Some(action) = text(shot, "action") {
            out.push(' ');
            out.push_str(&sentence(&action));
        }
        if let Some(camera) = text(shot, "camera") {
            out.push(' ');
            out.push_str(&sentence(&camera));
        }
        // This shot's lines, as placed above.
        for l in lines
            .iter()
            .zip(&placed)
            .filter(|(_, k)| **k == i + 1)
            .map(|(l, _)| l)
        {
            let (Some(who), Some(says)) = (text(l, "who"), text(l, "says")) else {
                continue;
            };
            let id = id_of(&who).unwrap_or(1);
            let how = text(l, "delivery")
                .map(|d| format!(" {d}"))
                .unwrap_or_default();
            out.push_str(&format!(
                " {who} (S{id}) says{how}, <d>[English] {}</d>",
                sentence(&says)
            ));
            // The person the clip is seen through has no mouth in frame:
            // telling it to stay closed contradicts the face never appearing
            // (review of #738).
            let listeners: Vec<String> = cast
                .iter()
                .filter_map(|c| text(c, "name"))
                .filter(|n| !same_name(n, &who))
                .filter(|n| !pov.iter().any(|p| same_name(p, n)))
                .collect();
            if !listeners.is_empty() {
                let one = listeners.len() == 1;
                out.push_str(&format!(
                    " {} {} with {} closed.",
                    crate::imagelib::join_and(&listeners),
                    if one { "listens" } else { "listen" },
                    if one { "mouth" } else { "mouths" },
                ));
            }
        }
    }
    for l in &lines {
        if let Some(k) = l.get("shot").and_then(Value::as_u64) {
            if k == 0 || k as usize > n {
                notes.push(format!(
                    "A line named shot {k}, and the scene has no such shot, so the line was dropped."
                ));
            }
        }
    }

    // Only the lines kept: a dropped line's words are not in the clip, and
    // counting them asked for cuts to words nobody would hear (review of
    // #732).
    let spoken: usize = lines
        .iter()
        .filter(kept)
        .filter_map(|l| text(l, "says"))
        .map(|s| s.split_whitespace().count())
        .sum();
    let room = (length - 1.0).max(1.0) * WORDS_PER_SECOND;
    if spoken as f64 > room {
        notes.push(format!(
            "The lines hold {spoken} words, and {length:.1} s fits about {room:.0}: expect rushed or clipped speech. Cut words or raise `seconds`."
        ));
    }

    Ok(Compiled {
        prompt: out.trim().to_string(),
        soundscape: text(scene, "soundscape"),
        music: text(scene, "music"),
        seconds,
        aspect: text(scene, "aspect"),
        notes,
        location: scene.get("setting").and_then(|s| text(s, "location")),
        silent_pov: pov.iter().filter(|p| id_of(p).is_none()).cloned().collect(),
        pov,
    })
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_lowercase().chain(c).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn workshop() -> Value {
        json!({
            "style": "Live-action, cinematic, calm and wry, cool overcast light",
            "setting": {"description": "A cluttered bicycle workshop", "light": "Grey light from a skylight"},
            "cast": [
                {"name": "Maya", "wearing": "a navy boiler suit with rolled sleeves", "doing": "kneels beside an upturned bike"},
                {"name": "Wren", "wearing": "a yellow raincoat"}
            ],
            "shots": [
                {"framing": "A wide two-shot", "action": "Wren shakes rain off her hood in the doorway", "camera": "The camera holds still"},
                {"at": 3.25, "framing": "A low angle on the bench", "action": "Maya spins the front wheel and listens"}
            ],
            "lines": [
                {"who": "Maya", "says": "Chain or brakes", "delivery": "without looking up"},
                {"who": "Wren", "says": "Both, probably."}
            ],
            "soundscape": "Rain drumming on a skylight.",
            "seconds": 6
        })
    }

    #[test]
    fn a_scene_becomes_the_timeline_in_order() {
        let c = compile(&workshop(), 5.0, &[]).unwrap();
        assert_eq!(
            c.prompt,
            "Live-action, cinematic, calm and wry, cool overcast light. [Shot 1] A wide two-shot. A cluttered bicycle workshop. Grey light from a skylight. \
             Maya wears a navy boiler suit with rolled sleeves and kneels beside an upturned bike. Wren wears a yellow raincoat. \
             Wren shakes rain off her hood in the doorway. The camera holds still. \
             Maya (S1) says without looking up, <d>[English] Chain or brakes.</d> Wren listens with mouth closed. \
             Wren (S2) says, <d>[English] Both, probably.</d> Maya listens with mouth closed. \
             [Shot 2] At 00:03.250, the shot cuts to a low angle on the bench. Maya spins the front wheel and listens."
        );
        assert_eq!(c.seconds, Some(6.0));
        assert_eq!(
            c.soundscape.as_deref(),
            Some("Rain drumming on a skylight.")
        );
        assert!(c.notes.is_empty(), "{:?}", c.notes);
    }

    /// A moved cut lands after the one before it, never on it: two shots
    /// asked for at 5.2 s of a 7 s clip cut at 5.2 s and then at 6.1 s, not
    /// twice at 5.2 s (review of #732).
    #[test]
    fn every_cut_comes_after_the_one_before_it() {
        let mut s = workshop();
        s["shots"] = json!([
            {"framing": "A wide shot", "action": "Wren comes in"},
            {"at": 5.2, "framing": "A close-up of Maya", "action": "Maya looks up"},
            {"at": 5.2, "framing": "A close-up of Wren", "action": "Wren grins"}
        ]);
        s["seconds"] = json!(7);
        let c = compile(&s, 5.0, &[]).unwrap();
        assert!(c.prompt.contains("[Shot 2] At 00:05.200"), "{}", c.prompt);
        assert!(c.prompt.contains("[Shot 3] At 00:06.100"), "{}", c.prompt);
        assert!(
            c.notes
                .iter()
                .any(|n| n.starts_with("Shot 3's cut at 5.20 s")),
            "{:?}",
            c.notes
        );
        // A cut asked for before a late one kept is moved after it, never to
        // the even spot behind it.
        s["shots"][1]["at"] = json!(6.3);
        s["shots"][2]["at"] = json!(1.0);
        let c = compile(&s, 5.0, &[]).unwrap();
        assert!(c.prompt.contains("[Shot 2] At 00:06.300"), "{}", c.prompt);
        assert!(c.prompt.contains("[Shot 3] At 00:06.650"), "{}", c.prompt);
    }

    /// A dropped line's speaker is not numbered: the first line kept is S1
    /// (review of #732).
    #[test]
    fn a_dropped_line_takes_no_speaker_id() {
        let mut s = workshop();
        s["lines"] = json!([
            {"who": "Wren", "says": "Over here.", "shot": 5},
            {"who": "Maya", "says": "Chain or brakes?"}
        ]);
        let c = compile(&s, 5.0, &[]).unwrap();
        assert!(c.prompt.contains("Maya (S1) says"), "{}", c.prompt);
        assert!(!c.prompt.contains("(S2)"), "{}", c.prompt);
    }

    #[test]
    fn timing_problems_are_notes_not_refusals() {
        let mut s = workshop();
        s["seconds"] = json!(6.5);
        s["shots"][1]["at"] = json!(9.0);
        s["lines"][0]["says"] =
            json!("Hand me the spanner and the pump, then hold the wheel still while I check");
        s["lines"][1]["shot"] = json!(3);
        s["cast"][1].as_object_mut().unwrap().remove("wearing");
        let c = compile(&s, 5.0, &[]).unwrap();
        assert!(c.prompt.contains("[Shot 2] At 00:03.250"), "{}", c.prompt);
        assert!(
            c.notes
                .iter()
                .any(|n| n.starts_with("Shot 2's cut at 9.00 s")),
            "{:?}",
            c.notes
        );
        assert!(
            c.notes
                .iter()
                .any(|n| n.starts_with("The lines hold 15 words")),
            "{:?}",
            c.notes
        );
        assert!(
            c.notes
                .iter()
                .any(|n| n.starts_with("No `wearing` for Wren")),
            "{:?}",
            c.notes
        );
        assert!(
            c.notes.iter().any(|n| n.starts_with("A line named shot 3")),
            "{:?}",
            c.notes
        );
        assert!(!c.prompt.contains("Both, probably"), "{}", c.prompt);
        assert!(compile(&json!({"cast": []}), 5.0, &[]).is_err());
    }

    /// A line without a `shot` goes where an action says someone speaks:
    /// both lines of a two-shot clip whose second shot "speaks" were compiled
    /// into the first (2026-10-25). An explicit `shot` keeps its place and
    /// its cue, and with no cue anywhere every line stays in the first.
    #[test]
    fn a_line_goes_to_the_shot_whose_action_speaks() {
        let mut s = workshop();
        s["shots"] = json!([
            {"framing": "A wide two-shot", "action": "Maya looks up and speaks"},
            {"at": 3.0, "framing": "A close-up of Wren", "action": "Wren grins, then says something"}
        ]);
        let c = compile(&s, 5.0, &[]).unwrap();
        let first = c.prompt.find("[Shot 2]").unwrap();
        assert!(
            c.prompt[..first].contains("Chain or brakes")
                && c.prompt[first..].contains("Both, probably"),
            "{}",
            c.prompt
        );
        assert!(
            c.notes.iter().any(|n| n.contains("line 2 in shot 2")),
            "{:?}",
            c.notes
        );
        // Placed explicitly, a line takes its shot's cue.
        s["lines"][0]["shot"] = json!(2);
        let c = compile(&s, 5.0, &[]).unwrap();
        let first = c.prompt.find("[Shot 2]").unwrap();
        assert!(
            c.prompt[first..].contains("Chain or brakes"),
            "{}",
            c.prompt
        );
        assert!(c.prompt[first..].contains("Both, probably"), "{}", c.prompt);
        // `shot: null` is no shot: placed by cue, not pinned to shot 1.
        s["lines"][0]["shot"] = json!(null);
        s["lines"][1]["shot"] = json!(null);
        let c = compile(&s, 5.0, &[]).unwrap();
        let first = c.prompt.find("[Shot 2]").unwrap();
        assert!(c.prompt[first..].contains("Both, probably"), "{}", c.prompt);
        assert!(
            c.prompt[..first].contains("Chain or brakes"),
            "{}",
            c.prompt
        );
        // No cue anywhere: the first shot, as before, and no note.
        let c = compile(&workshop(), 5.0, &[]).unwrap();
        assert!(
            !c.notes.iter().any(|n| n.starts_with("Lines without")),
            "{:?}",
            c.notes
        );
    }

    /// A first-person cast member is said to be the camera, ahead of what
    /// they wear and do; `doing` that opens on an -ing word reads as a
    /// sentence; and the setting's sentences open with a capital.
    #[test]
    fn the_camera_person_and_the_cast_sentences_read_as_written() {
        let s = json!({
            "setting": {"description": "a small changing room", "light": "flat overhead light"},
            "cast": [
                {"name": "Maya", "wearing": "a green raincoat", "doing": "kneeling on the tile floor"},
                {"name": "Wren", "pov": true, "wearing": "nothing", "doing": "standing over her"}
            ],
            "shots": [{"framing": "A first-person shot looking down", "action": "Maya looks up"}]
        });
        let c = compile(&s, 5.0, &[]).unwrap();
        assert!(
            c.prompt
                .contains("A small changing room. Flat overhead light."),
            "{}",
            c.prompt
        );
        assert!(
            c.prompt
                .contains("Maya wears a green raincoat and is kneeling on the tile floor."),
            "{}",
            c.prompt
        );
        assert!(
            c.prompt.contains("The clip is seen through Wren's eyes: the camera is Wren's point of view, so Wren's face never appears, and only Wren's hands, arms and body are seen. Wren wears nothing and is standing over her."),
            "{}",
            c.prompt
        );
        assert_eq!(c.pov, vec!["Wren".to_string()]);
        assert_eq!(c.silent_pov, vec!["Wren".to_string()]);
        assert!(compile(&workshop(), 5.0, &[]).unwrap().pov.is_empty());
        // A line in any case makes them a speaker (review of #738).
        let mut s = s;
        s["lines"] = json!([{"who": "wren", "says": "Over here."}]);
        let c = compile(&s, 5.0, &[]).unwrap();
        assert_eq!(c.pov, vec!["Wren".to_string()]);
        assert!(c.silent_pov.is_empty(), "{:?}", c.silent_pov);
        // Wren speaks, and Maya listens; Wren, never seen, is never told to.
        s["lines"] = json!([{"who": "Maya", "says": "Over here."}]);
        let c = compile(&s, 5.0, &[]).unwrap();
        assert!(!c.prompt.contains("Wren listens"), "{}", c.prompt);
        assert!(!c.prompt.contains("listens with"), "{}", c.prompt);
        // Spelled another way, still the speaker (review of #738).
        s["lines"] = json!([{"who": "WREN", "says": "Over here."}]);
        assert!(compile(&s, 5.0, &[]).unwrap().silent_pov.is_empty());
        let mut hyphened = s.clone();
        hyphened["cast"][1]["name"] = json!("Wren Ash");
        hyphened["lines"] = json!([{"who": "wren-ash", "says": "Over here."}]);
        assert!(compile(&hyphened, 5.0, &[]).unwrap().silent_pov.is_empty());
        // Nor is "nothing", the undressed cast's idiom one field over.
        s["cast"][0]["doing"] = json!("nothing in particular");
        assert!(
            !compile(&s, 5.0, &[]).unwrap().prompt.contains("is nothing"),
            "pronoun"
        );
        // A base verb ending in -ing is not made progressive.
        s["cast"][0]["doing"] = json!("bring the towels in");
        assert!(
            compile(&s, 5.0, &[])
                .unwrap()
                .prompt
                .contains("and bring the towels in."),
            "base verb"
        );
        // An -ing doing written with a capital reads mid-sentence.
        s["cast"][0]["doing"] = json!("Kneeling on the tile floor");
        assert!(
            compile(&s, 5.0, &[])
                .unwrap()
                .prompt
                .contains("and is kneeling on the tile floor."),
            "lower-cased"
        );
    }

    /// The local model's first scene through the built-in tool, as it wrote
    /// it (2026-10-25, names changed): the camera person said only in the
    /// shot's words, and the lines quoted inside an action.
    #[test]
    fn a_scene_is_mended_where_its_words_say_more_than_its_fields() {
        let s = json!({
            "cast": [
                {"name": "Maya", "wearing": "a green raincoat", "doing": "kneeling on the bench"},
                {"name": "Wren", "wearing": "nothing"}
            ],
            "shots": [
                {"framing": "A first-person shot", "action": "Wren's POV, his hands on the rail. Maya laughs."},
                {"at": 4.0, "framing": "A close-up of Maya", "action": "Maya speaks, looking back at Wren. \"Over here, Wren.\" She laughs, catching her breath. Then: \u{201c}Not yet, not yet.\u{201d} She grins. A sign reads \"NO ENTRY HERE\"."}
            ]
        });
        let c = compile(&s, 8.0, &[]).unwrap();
        assert_eq!(c.pov, vec!["Wren".to_string()]);
        let second = &c.prompt[c.prompt.find("[Shot 2]").unwrap()..];
        assert!(
            second.contains("Maya (S1) says, <d>[English] Over here, Wren.</d>"),
            "{}",
            c.prompt
        );
        assert!(
            second.contains("Maya (S1) says, <d>[English] Not yet, not yet.</d>"),
            "{}",
            c.prompt
        );
        // Gone from the action, which still reads; the sign is left alone.
        assert!(!c.prompt.contains("\"Over here"), "{}", c.prompt);
        assert!(
            second.contains("She laughs, catching her breath. Then: She grins."),
            "{}",
            c.prompt
        );
        assert!(c.prompt.contains("\"NO ENTRY HERE\""), "{}", c.prompt);
        assert!(
            c.notes
                .iter()
                .any(|n| n.starts_with("2 spoken lines written inside `action`")),
            "{:?}",
            c.notes
        );
        assert!(
            c.notes.iter().any(|n| n.contains("taken as the camera")),
            "{:?}",
            c.notes
        );
        // A scene that set either field itself is left as written.
        let mut set = s.clone();
        set["cast"][1]["pov"] = json!(false);
        set["lines"] = json!([{"who": "Maya", "says": "Hello there."}]);
        let c = compile(&set, 8.0, &[]).unwrap();
        assert!(c.pov.is_empty());
        assert!(c.prompt.contains("\"Over here, Wren.\""), "{}", c.prompt);
    }

    /// The real call's shape (2026-10-25, names changed): no `cast` at all,
    /// both people named only in the prose, the camera said as "<name>'s
    /// POV", and two lines quoted in the last shot's action.
    #[test]
    fn a_scene_with_no_cast_is_mended_from_the_librarys_names() {
        let s = json!({
            "seconds": 10,
            "shots": [
                {"framing": "A POV shot from behind Maya", "action": "The camera shakes slightly with each step \u{2014} Wren's POV, his hands on Maya's shoulders. Maya leans on the rail."},
                {"at": 3.5, "framing": "Close-up of Maya's face over her shoulder", "action": "Maya turns her head back, eyes locked on Wren."},
                {"at": 5.5, "framing": "Close-up of Maya's face over her shoulder", "action": "Maya speaks, looking back at Wren. \"Hold on to me.\" She laughs, out of breath. Then: \"Not so fast, Wren.\" Her eyes shine."}
            ]
        });
        let known = ["maya".to_string(), "wren".to_string(), "theo".to_string()];
        let c = compile(&s, 5.0, &known).unwrap();
        assert_eq!(c.pov, vec!["Wren".to_string()]);
        assert!(
            c.prompt.contains("The clip is seen through Wren's eyes"),
            "{}",
            c.prompt
        );
        let third = &c.prompt[c.prompt.find("[Shot 3]").unwrap()..];
        assert!(
            third.contains("Maya (S1) says, <d>[English] Hold on to me.</d>"),
            "{}",
            c.prompt
        );
        assert!(
            third.contains("Maya (S1) says, <d>[English] Not so fast, Wren.</d>"),
            "{}",
            c.prompt
        );
        // A library name the scene never mentions is not added.
        assert!(!c.prompt.contains("Theo"), "{}", c.prompt);
        // Without the library's names, nothing can be mended.
        let bare = compile(&s, 5.0, &[]).unwrap();
        assert!(bare.pov.is_empty());
    }

    #[test]
    fn stamps_are_minutes_seconds_and_millis() {
        assert_eq!(stamp(1.5), "00:01.500");
        assert_eq!(stamp(75.25), "01:15.250");
        assert_eq!(stamp(-1.0), "00:00.000");
    }
}
