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
use serde_json::Value;

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
/// scene names none).
pub fn compile(scene: &Value, default_seconds: f64) -> Result<Compiled> {
    let mut notes = Vec::new();
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
            if !speakers.iter().any(|s| s.eq_ignore_ascii_case(&who)) {
                speakers.push(who);
            }
        }
    }
    let id_of = |who: &str| {
        speakers
            .iter()
            .position(|s| s.eq_ignore_ascii_case(who))
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

    let mut out = String::new();
    if let Some(style) = text(scene, "style") {
        out.push_str(&sentence(&style));
        out.push(' ');
    }
    for (i, shot) in shots.iter().enumerate() {
        let framing = text(shot, "framing").unwrap_or_else(|| "a medium shot".into());
        if i == 0 {
            out.push_str(&format!("[Shot 1] {}", sentence(&framing)));
            // The setting and the people, once, at the opening.
            if let Some(setting) = scene.get("setting") {
                if let Some(d) = text(setting, "description") {
                    out.push(' ');
                    out.push_str(&sentence(&d));
                }
                if let Some(l) = text(setting, "light") {
                    out.push(' ');
                    out.push_str(&sentence(&l));
                }
            }
            for c in &cast {
                let Some(name) = text(c, "name") else {
                    continue;
                };
                let mut said = name.clone();
                match text(c, "wearing") {
                    Some(w) => said.push_str(&format!(" wears {w}")),
                    None => notes.push(format!(
                        "No `wearing` for {name}, so the video model chose the outfit: clothes come from the words alone, not the portrait."
                    )),
                }
                if let Some(d) = text(c, "doing") {
                    let join = if said == name { " " } else { " and " };
                    said.push_str(&format!("{join}{d}"));
                }
                if said != name {
                    out.push(' ');
                    out.push_str(&sentence(&said));
                }
                if let Some(extra) = text(c, "notes") {
                    out.push(' ');
                    out.push_str(&sentence(&extra));
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
        // This shot's lines: a line names its shot, or belongs to the first.
        for l in lines
            .iter()
            .filter(|l| l.get("shot").and_then(Value::as_u64).unwrap_or(1) as usize == i + 1)
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
            let listeners: Vec<String> = cast
                .iter()
                .filter_map(|c| text(c, "name"))
                .filter(|n| !n.eq_ignore_ascii_case(&who))
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

    let spoken: usize = lines
        .iter()
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
        let c = compile(&workshop(), 5.0).unwrap();
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
        let c = compile(&s, 5.0).unwrap();
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
        let c = compile(&s, 5.0).unwrap();
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
        let c = compile(&s, 5.0).unwrap();
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
        let c = compile(&s, 5.0).unwrap();
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
                .any(|n| n.starts_with("The lines hold 17 words")),
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
        assert!(compile(&json!({"cast": []}), 5.0).is_err());
    }

    #[test]
    fn stamps_are_minutes_seconds_and_millis() {
        assert_eq!(stamp(1.5), "00:01.500");
        assert_eq!(stamp(75.25), "01:15.250");
        assert_eq!(stamp(-1.0), "00:00.000");
    }
}
