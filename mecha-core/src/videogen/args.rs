//! What a `video_generate` call asks for, filled in before anything is
//! resolved: the chat's kept settings, a typed scene's timeline, the
//! persona's own character, and the library characters the prompt names.
//!
//! Every function here is a function of its inputs — the call's arguments
//! and what the stores said — so the tool's call reads the stores once and
//! these decide. Ported from the private H3 server with its behaviour
//! unchanged; the fixtures are invented.

use super::{graph, scene};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::collections::HashMap;

/// The settings a chat's clips keep from one to the next (owner,
/// 2026-10-16): asked once ("no turbo, 10 seconds"), they held for one
/// clip, then the model left them out and the next rendered turbo at 5 s,
/// 9 of 14 and 7 of 18 times.
pub const KEPT: [&str; 4] = ["turbo", "seconds", "size", "aspect"];

/// The file name a chat's kept settings live under, from the chat's folder
/// relative to the mecha home: `work/persona/p-1` → `work_persona_p-1`. The
/// same name the private server gave them, so a chat's settings carry over.
pub fn settings_name(relative: &std::path::Path) -> String {
    relative
        .to_string_lossy()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The chat's last clip's settings (`last`), where this call says nothing
/// of its own (a scene's `seconds` and `aspect` count as said). Each kept
/// value that differs from the default is said, so a draft or a turbo-off
/// that was not wanted again can be undone in words.
pub fn with_chat_settings(args: &Value, last: Option<&Value>) -> (Value, Vec<String>) {
    let mut out = args.clone();
    // Indexing a value that is not an object to set a key panics; such a
    // call is refused later, with a sentence.
    if !(args.is_object() || args.is_null()) {
        return (out, Vec::new());
    }
    let Some(last) = last else {
        return (out, Vec::new());
    };
    let mut said = Vec::new();
    for key in KEPT {
        let given = !args[key].is_null()
            || (matches!(key, "seconds" | "aspect") && !args["scene"][key].is_null());
        if given || last[key].is_null() {
            continue;
        }
        // A kept length is held to the range a call is: the private server
        // shares this file and writes the length it rendered, a frame or so
        // past what was asked (15.08 s for 15), which `plan` would refuse
        // with no word of where it came from.
        let kept = match (key, last[key].as_f64()) {
            ("seconds", Some(s)) => json!(s.clamp(graph::MIN_SECONDS, graph::MAX_SECONDS)),
            ("seconds", None) => continue,
            _ => last[key].clone(),
        };
        out[key] = kept.clone();
        match key {
            "turbo" if kept == json!(false) => said.push("full quality (turbo off)".to_string()),
            "seconds" => {
                if let Some(s) = kept.as_f64().filter(|s| (s - 5.0).abs() > 0.01) {
                    said.push(if s.fract() == 0.0 {
                        format!("{s:.0} s")
                    } else {
                        format!("{s:.1} s")
                    });
                }
            }
            "size" if last[key] == json!("draft") => said.push("draft size".to_string()),
            "aspect" => {
                if let Some(a) = last[key].as_str().filter(|a| *a != "16:9") {
                    said.push(format!("aspect {a}"));
                }
            }
            _ => {}
        }
    }
    let notes = if said.is_empty() {
        Vec::new()
    } else {
        vec![format!(
            "Kept from this chat's last clip: {}. Say otherwise to change it.",
            said.join(", ")
        )]
    };
    (out, notes)
}

/// What the next clip in a chat keeps: the settings a finished one was made
/// at, read from its record.
pub fn kept_settings(record: &Value) -> Value {
    // The length asked for where the record has it: `seconds` is the
    // rendered length, snapped up to the frame grid (review of #733).
    let seconds = match &record["asked_seconds"] {
        Value::Null => record["seconds"].clone(),
        asked => asked.clone(),
    };
    json!({
        "turbo": record["turbo"], "seconds": seconds,
        "size": record["size"], "aspect": record["aspect"],
    })
}

/// A call that gives a typed `scene`: its timeline becomes `prompt`, and its
/// sound, length and shape fill whatever the call left out. The compiler's
/// notes ride along to the result.
pub fn with_scene(args: &Value) -> Result<(Value, Vec<String>)> {
    let Some(scene_args) = args.get("scene").filter(|s| s.is_object()) else {
        if args["prompt"].as_str().is_none_or(|p| p.trim().is_empty()) {
            bail!("give `scene` (typed; see video_prompt_guide) or `prompt`");
        }
        return Ok((args.clone(), Vec::new()));
    };
    let default = args["seconds"].as_f64().unwrap_or(5.0);
    let c = scene::compile(scene_args, default)?;
    let mut out = args.clone();
    let mut notes = c.notes;
    if let Some(p) = args["prompt"].as_str().filter(|p| !p.trim().is_empty()) {
        // What only the prompt said is lost with it: say so by name, so the
        // next call puts it in the scene (6 of 63 clips carried this note).
        let mut lost = Vec::new();
        if p.contains("<d>")
            && scene_args
                .get("lines")
                .is_none_or(|l| l.as_array().is_none_or(|a| a.is_empty()))
        {
            lost.push("its dialogue (put speech in `scene.lines`)");
        }
        notes.push(if lost.is_empty() {
            "Both `scene` and `prompt` were given; the scene was used, and the prompt left out."
                .into()
        } else {
            format!(
                "Both `scene` and `prompt` were given; the scene was used, so the prompt was left out, with {}.",
                lost.join(" and ")
            )
        });
    }
    out["prompt"] = json!(c.prompt);
    // Who the clip is seen through, for the references `plan` builds: their
    // face is said to stay out, and a silent one carries no voice. Only the
    // compiler writes these: a call's own are dropped first (review of #738).
    if let Some(o) = out.as_object_mut() {
        o.remove("pov");
        o.remove("pov_silent");
    }
    if !c.pov.is_empty() {
        out["pov"] = json!(c.pov);
    }
    if !c.silent_pov.is_empty() {
        out["pov_silent"] = json!(c.silent_pov);
    }
    for (key, value) in [
        ("soundscape", c.soundscape),
        ("music", c.music),
        ("aspect", c.aspect),
    ] {
        if out[key].as_str().is_none() {
            if let Some(v) = value {
                out[key] = json!(v);
            }
        }
    }
    // The scene's own length times its cuts, so the clip is rendered at it:
    // a different top-level `seconds` drew the clip at one length with its
    // shots cut for another.
    if let Some(s) = c
        .seconds
        .filter(|s| (graph::MIN_SECONDS..=graph::MAX_SECONDS).contains(s))
    {
        if let Some(top) = out["seconds"].as_f64().filter(|t| (t - s).abs() > 0.01) {
            notes.push(format!(
                "The scene's own length ({s:.0} s) was used, not `seconds` ({top:.0} s)."
            ));
        }
        out["seconds"] = json!(s);
    }
    Ok((out, notes))
}

/// A persona's call: `self` names the persona's own character, and a call
/// that gives no references gets that character as its one reference, so the
/// persona looks like herself without the model having to ask (owner,
/// 2026-10-13: three persona clips went out text-only). A pinned frame does
/// not exclude her: frames are pinned beside the references. Without a
/// persona, `self` is left as written and fails as an unknown name would.
pub fn persona_defaults(args: &Value, character: Option<&str>) -> Value {
    let Some(character) = character else {
        return args.clone();
    };
    let mut args = with_self(args, character);
    if args["references"].as_array().is_none_or(|a| a.is_empty()) && args.is_object() {
        args["references"] = json!([character]);
    }
    args
}

fn with_self(args: &Value, character: &str) -> Value {
    let mut args = args.clone();
    let swap = |v: &mut Value| {
        if v.as_str() == Some("self") {
            *v = json!(character);
        }
    };
    if let Some(refs) = args.get_mut("references").and_then(Value::as_array_mut) {
        refs.iter_mut().for_each(swap);
    }
    for key in ["first_frame", "last_frame"] {
        if let Some(v) = args.get_mut(key) {
            swap(v);
        }
    }
    args
}

/// Whether a reference names a library character rather than a picture:
/// no path separator and no extension.
pub fn looks_like_name(given: &str) -> bool {
    !given.contains('/') && !given.contains('.')
}

/// A character's name as the library keys it: `Mara Lee` → `mara-lee`, the
/// library's own spelling rule (`imagelib::spelled_as_name`), so `Mara_Lee`
/// finds her too (review of #733).
pub fn library_key(given: &str) -> String {
    crate::imagelib::spelled_as_name(given)
}

/// Every reference and pinned frame that names a character, in the
/// library's form, so the lookups, the voices and the labels all agree.
pub fn keyed_names(args: &Value) -> Value {
    let mut args = args.clone();
    let key = |v: &mut Value| {
        if let Some(s) = v.as_str().filter(|s| looks_like_name(s)) {
            *v = json!(library_key(s));
        }
    };
    if let Some(refs) = args.get_mut("references").and_then(Value::as_array_mut) {
        refs.iter_mut().for_each(key);
    }
    for k in ["first_frame", "last_frame"] {
        if let Some(v) = args.get_mut(k) {
            key(v);
        }
    }
    args
}

/// The library characters a prompt names, in order of first mention: a
/// whole word, any case, a hyphenated name also as its spaced form
/// (`mara-lee` as "Mara Lee"). The model writes the name in the prose and
/// forgets the reference; the name is the request (owner, 2026-10-13:
/// persona clips named someone and drew someone else).
pub fn named_in(prompt: &str, names: &[String]) -> Vec<String> {
    let lower = prompt.to_lowercase();
    let is_word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let first_at = |needle: &str| {
        if needle.is_empty() {
            return None;
        }
        lower.match_indices(needle).map(|(i, _)| i).find(|&i| {
            !is_word(lower[..i].chars().next_back())
                && !is_word(lower[i + needle.len()..].chars().next())
        })
    };
    let mut found: Vec<(usize, String)> = names
        .iter()
        .filter_map(|n| {
            let spaced = n.replace('-', " ");
            let at = [first_at(n), first_at(&spaced)]
                .into_iter()
                .flatten()
                .min()?;
            Some((at, n.clone()))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, n)| n).collect()
}

/// Every library character the prompt names becomes a reference, after any
/// the call gave, up to H3's nine.
pub fn with_named(args: &Value, named: &[String]) -> Value {
    if named.is_empty() || !args.is_object() {
        return args.clone();
    }
    let mut args = args.clone();
    let mut refs: Vec<Value> = args["references"].as_array().cloned().unwrap_or_default();
    for n in named {
        if refs.len() >= graph::MAX_REFERENCES {
            break;
        }
        if !refs.iter().any(|r| r.as_str() == Some(n.as_str())) {
            refs.push(json!(n));
        }
    }
    args["references"] = json!(refs);
    args
}

/// A library character's voice in a clip: which reference it speaks for,
/// and the voice-library name it speaks in.
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceFor {
    pub reference: usize,
    pub character: String,
    pub name: String,
}

/// The voice each library character among `references` speaks in: the
/// calling persona's own voice for her character (`own`), else the owner's
/// map (`owner`), else the voice of the persona that links that character
/// (`personas`). References without one are silent: H3 voices them itself.
pub fn voices_for(
    references: &[String],
    own: Option<(&str, &str)>,
    owner: &HashMap<String, String>,
    personas: &HashMap<String, String>,
) -> Vec<VoiceFor> {
    references
        .iter()
        .enumerate()
        .filter(|(_, r)| looks_like_name(r))
        .filter_map(|(i, c)| {
            let name = match own {
                Some((mine, voice)) if mine == c => voice.to_string(),
                _ => owner.get(c).or_else(|| personas.get(c))?.clone(),
            };
            Some(VoiceFor {
                reference: i,
                character: c.clone(),
                name,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chat's clips keep its last settings where a call says nothing, and
    /// say the ones that are not the defaults; what a call or its scene
    /// gives is its own (owner, 2026-10-16).
    #[test]
    fn a_chats_clips_keep_its_last_settings_unless_told_otherwise() {
        let (same, said) = with_chat_settings(&json!({"prompt": "x"}), None);
        assert_eq!((same, said.len()), (json!({"prompt": "x"}), 0));
        let last = kept_settings(
            &json!({"turbo": false, "seconds": 10.0, "size": "draft", "aspect": "9:16"}),
        );
        let (out, said) = with_chat_settings(&json!({"prompt": "x"}), Some(&last));
        assert_eq!(
            (out["turbo"].clone(), out["seconds"].clone()),
            (json!(false), json!(10.0))
        );
        assert_eq!(
            (out["size"].clone(), out["aspect"].clone()),
            (json!("draft"), json!("9:16"))
        );
        assert_eq!(
            said,
            ["Kept from this chat's last clip: full quality (turbo off), 10 s, draft size, aspect 9:16. Say otherwise to change it."]
        );
        let (out, _) = with_chat_settings(
            &json!({"turbo": true, "scene": {"seconds": 6, "aspect": "16:9"}}),
            Some(&last),
        );
        assert_eq!(out["turbo"], json!(true), "the call's own turbo");
        assert!(out["seconds"].is_null(), "the scene's own length: {out}");
        assert!(out["aspect"].is_null(), "the scene's own aspect: {out}");
        let short = kept_settings(&json!({"turbo": true, "seconds": 4.5}));
        let (_, said) = with_chat_settings(&json!({}), Some(&short));
        assert_eq!(
            said,
            ["Kept from this chat's last clip: 4.5 s. Say otherwise to change it."]
        );
        // A length the private server wrote, a frame past the range, is
        // held to it rather than refused at the next prompt-only call.
        let (out, said) = with_chat_settings(&json!({}), Some(&json!({"seconds": 15.083})));
        assert_eq!(out["seconds"], json!(graph::MAX_SECONDS));
        assert_eq!(
            said,
            ["Kept from this chat's last clip: 15 s. Say otherwise to change it."]
        );
        let (odd, said) = with_chat_settings(&json!([1]), Some(&last));
        assert_eq!(
            (odd, said.len()),
            (json!([1]), 0),
            "not an object: untouched"
        );
    }

    #[test]
    fn a_chats_settings_are_named_by_its_folder() {
        assert_eq!(
            settings_name(std::path::Path::new("work/persona/p-1")),
            "work_persona_p-1"
        );
    }

    /// The scene's own length is the clip's, since it times the cuts; a
    /// different top-level `seconds` is said, not rendered. A prompt beside a
    /// scene says what it took with it.
    #[test]
    fn only_the_compiler_says_who_the_camera_is() {
        let scene = json!({"shots": [{"framing": "A long view down the hall", "action": "Maya waves"}],
                           "cast": [{"name": "Maya", "wearing": "a coat"}]});
        // A call's own `pov` and `pov_silent` are dropped, not passed on.
        let (out, _) = with_scene(&json!({
            "scene": scene, "pov": ["maya"], "pov_silent": ["maya"]
        }))
        .unwrap();
        assert!(out.get("pov").is_none(), "{out}");
        assert!(out.get("pov_silent").is_none(), "{out}");
        // The scene's own are kept.
        let mut seen = scene.clone();
        seen["cast"][0]["pov"] = json!(true);
        let (out, _) = with_scene(&json!({"scene": seen})).unwrap();
        assert_eq!(out["pov"], json!(["Maya"]));
        assert_eq!(out["pov_silent"], json!(["Maya"]));
    }

    #[test]
    fn the_scenes_length_is_the_clips_and_a_lost_prompt_says_what_it_held() {
        let scene = json!({"seconds": 8, "shots": [{"action": "Wren waves."}]});
        let (out, notes) = with_scene(&json!({"seconds": 5, "scene": scene})).unwrap();
        assert_eq!(out["seconds"], json!(8.0));
        assert!(
            notes.iter().any(|n| n.contains("scene's own length (8 s)")),
            "{notes:?}"
        );
        let (_, notes) = with_scene(&json!({
            "prompt": "Wren (S1) says, <d>[English] Morning.</d>", "scene": scene}))
        .unwrap();
        assert!(
            notes.iter().any(|n| n.contains("its dialogue")),
            "{notes:?}"
        );
        assert!(with_scene(&json!({})).is_err(), "neither scene nor prompt");
    }

    #[test]
    fn a_persona_call_with_nothing_to_look_like_gets_the_persona() {
        let args = json!({"prompt": "She walks along the pier."});
        let out = persona_defaults(&args, Some("maya"));
        assert_eq!(out["references"], json!(["maya"]));
        // `self` still names her where the model wrote it.
        let out = persona_defaults(
            &json!({"prompt": "p", "references": ["self", "wren"]}),
            Some("maya"),
        );
        assert_eq!(out["references"], json!(["maya", "wren"]));
        // A pinned frame keeps its frame and still gets the persona.
        let out = persona_defaults(
            &json!({"prompt": "p", "first_frame": "images/a.png"}),
            Some("maya"),
        );
        assert_eq!(out["references"], json!(["maya"]));
        assert_eq!(out["first_frame"], "images/a.png");
        // No persona: nothing is added.
        let out = persona_defaults(&args, None);
        assert!(out.get("references").is_none(), "{out}");
    }

    #[test]
    fn the_characters_a_prompt_names_become_references_in_order() {
        let names: Vec<String> = ["maya", "wren", "mara-lee", "sam"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            named_in("Wren waves as MAYA walks in; Mara Lee laughs.", &names),
            ["wren", "maya", "mara-lee"]
        );
        // A whole word only: "same" and "wrench" name no one.
        assert!(named_in("The same wrench again.", &names).is_empty());
        let args = json!({"prompt": "p", "references": ["maya"]});
        assert_eq!(
            with_named(&args, &["wren".into(), "maya".into()])["references"],
            json!(["maya", "wren"])
        );
        let pinned = json!({"prompt": "p", "first_frame": "images/a.png"});
        assert_eq!(
            with_named(&pinned, &["wren".into()])["references"],
            json!(["wren"])
        );
    }

    #[test]
    fn a_name_is_keyed_as_the_library_keys_it_and_a_path_is_left_alone() {
        let args =
            json!({"references": ["Mara Lee", "images/a.png", "Wren"], "first_frame": "Maya"});
        let out = keyed_names(&args);
        assert_eq!(
            out["references"],
            json!(["mara-lee", "images/a.png", "wren"])
        );
        assert_eq!(out["first_frame"], "maya");
    }

    /// The persona's own voice for her character, then the owner's map, then
    /// the persona linking the character; a picture has none.
    #[test]
    fn each_character_speaks_in_the_voice_its_owner_gave_it() {
        let refs: Vec<String> = ["maya", "wren", "images/a.png", "sam"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let owner = HashMap::from([("wren".to_string(), "wren-low".to_string())]);
        let personas = HashMap::from([
            ("wren".to_string(), "wren-bright".to_string()),
            ("maya".to_string(), "maya-1".to_string()),
        ]);
        let got = voices_for(&refs, Some(("maya", "maya-own")), &owner, &personas);
        assert_eq!(
            got.iter()
                .map(|v| (v.reference, v.name.as_str()))
                .collect::<Vec<_>>(),
            [(0, "maya-own"), (1, "wren-low")]
        );
        let got = voices_for(&refs, None, &HashMap::new(), &personas);
        assert_eq!(got[0].name, "maya-1");
    }
}
