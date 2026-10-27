//! The owner's own LoRAs for clips, by name: the `h3` entries of the
//! owner's registry (`crate::loras`) whose files were found and checked,
//! from which the model picks per clip through `motion`. The model names an
//! entry, never a file: the file, its strength and its trigger words are the
//! registry's.
//!
//! mecha ships no LoRA and names none (the owner's ruling, 2026-10-23): this
//! is the mechanism, and which LoRAs exist is the owner's files and registry.
//!
//! **Each mode gets only LoRAs trained for its checkpoint.** References mode
//! runs H3's `ref2va` model and frames mode its `fl2va` one; a LoRA trained
//! on the other gave broken clips in the owner's tests (2026-10-24), so an
//! entry names its file per mode, and a pick without a file for the clip's
//! mode is left out with what would work. **And only LoRAs trained for H3**:
//! an entry whose file's keys say another model never reaches a clip
//! (`crate::loras::Status`).

use anyhow::{bail, Result};
use serde_json::{json, Value};

use super::graph::Mode;

/// At most this many LoRAs the model picks for one clip, beside turbo and
/// the always-on ones: each one shifts the whole model, and two strong
/// action LoRAs together broke a clip that each passed alone (2026-10-24).
pub const MAX_PER_CLIP: usize = 4;

#[derive(Debug, Clone, PartialEq)]
pub struct Lora {
    pub name: String,
    pub about: String,
    pub frames: Option<String>,
    pub references: Option<String>,
    pub strength: f64,
    pub trigger: Option<String>,
    /// Applied to every clip in a mode it has a file for; not offered to the
    /// model, and not counted against [`MAX_PER_CLIP`].
    pub always: bool,
}

impl Lora {
    /// The file this LoRA loads in `mode`, if it offers that mode.
    pub fn file(&self, mode: Mode) -> Option<&str> {
        match mode {
            Mode::Frames => self.frames.as_deref(),
            Mode::References => self.references.as_deref(),
        }
    }

    fn modes(&self) -> &'static str {
        match (self.frames.is_some(), self.references.is_some()) {
            (true, true) => "",
            (true, false) => " (frames mode only: a clip with no references)",
            _ => " (references mode only)",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Registry {
    pub loras: Vec<Lora>,
}

impl Registry {
    /// The clip entries of `loaded` that can apply: `h3`, every file found
    /// and read as H3's. The rest are `loaded`'s problems to say.
    pub fn from_loaded(loaded: &crate::loras::Loaded) -> Registry {
        Registry {
            loras: loaded
                .usable(crate::loras::Family::H3)
                .map(|e| Lora {
                    name: e.name.clone(),
                    about: e.about.clone(),
                    frames: e.frames.clone(),
                    references: e.references.clone(),
                    strength: e.strength,
                    trigger: e.trigger.clone(),
                    always: e.always,
                })
                .collect(),
        }
    }

    /// The registry in `text`, as the old clips-only file, unchecked: what a
    /// test of picking needs.
    #[cfg(test)]
    pub(crate) fn parse(text: &str) -> (Registry, Vec<String>) {
        let Ok((entries, skipped)) = crate::loras::parse(text, true) else {
            return (Registry::default(), vec!["the file does not read".into()]);
        };
        let loaded = crate::loras::Loaded {
            source: crate::loras::Source::None,
            ignored: None,
            entries: entries
                .into_iter()
                .map(|e| (e, crate::loras::Status::Checked))
                .collect(),
            skipped: Vec::new(),
            unreadable: None,
        };
        (Registry::from_loaded(&loaded), skipped)
    }

    pub fn get(&self, name: &str) -> Option<&Lora> {
        self.loras.iter().find(|l| l.name == name)
    }

    /// The LoRAs the model may pick: every entry but the always-on ones.
    fn offered(&self) -> impl Iterator<Item = &Lora> {
        self.loras.iter().filter(|l| !l.always)
    }

    /// The `motion` property for the tool's schema, or `None` with nothing
    /// to pick, when the schema reads exactly as it does without a registry.
    /// Built once with the tool: it sits in the cached prefix.
    pub fn schema(&self) -> Option<Value> {
        self.offered().next()?;
        let names: Vec<&str> = self.offered().map(|l| l.name.as_str()).collect();
        let lines: Vec<String> = self
            .offered()
            .map(|l| format!("{} — {}{}", l.name, l.about, l.modes()))
            .collect();
        Some(json!({
            "type": "array",
            "items": {"type": "string", "enum": names},
            "maxItems": MAX_PER_CLIP,
            "description": format!(
                "The owner's LoRAs for this clip, by name, at most {MAX_PER_CLIP}; only when the clip shows what one is for. Not carried over to the next clip. {}.",
                lines.join("; ")
            ),
        }))
    }

    /// The always-on LoRAs that have a file for `mode`, then the model's
    /// `motion` picks in the order given, and a note for each pick left out.
    /// Unknown names, repeats or too many are refused, with what would work;
    /// a pick with no file for this clip's mode is left out and said, not
    /// refused: the model cannot know the mode (a library character it names
    /// makes it references mode), and the first real call spent three
    /// refusals finding out (2026-10-25). Naming an always-on LoRA is a
    /// no-op, since it is already there.
    pub fn pick(&self, motion: &Value, mode: Mode) -> Result<(Vec<&Lora>, Vec<String>)> {
        let mut out: Vec<&Lora> = self
            .loras
            .iter()
            .filter(|l| l.always && l.file(mode).is_some())
            .collect();
        let mut notes = Vec::new();
        let names: Vec<&str> = match motion {
            Value::Null => return Ok((out, notes)),
            Value::Array(a) => a.iter().map(|v| v.as_str().unwrap_or("")).collect(),
            Value::String(s) => vec![s.as_str()],
            _ => bail!("`motion` is a list of LoRA names"),
        };
        let names: Vec<&str> = names
            .into_iter()
            .filter(|n| !self.get(n).is_some_and(|l| l.always))
            .collect();
        if names.len() > MAX_PER_CLIP {
            bail!("at most {MAX_PER_CLIP} LoRAs in `motion` on one clip");
        }
        // Repeats are checked over what was named, not what was kept: a
        // left-out pick named twice was a silent repeat and a doubled note
        // (review of #742).
        let mut named: Vec<&str> = Vec::new();
        for n in names {
            let Some(l) = self.get(n) else {
                let known: Vec<&str> = self.offered().map(|l| l.name.as_str()).collect();
                bail!("no LoRA named {n:?} in `motion`; the names are {known:?}");
            };
            if named.contains(&l.name.as_str()) {
                bail!("LoRA {n:?} is named twice in `motion`");
            }
            named.push(l.name.as_str());
            if l.file(mode).is_none() {
                notes.push(format!(
                    "The LoRA `{n}` was left out: it has no {} file, and this clip renders in {} mode{}.",
                    mode.name(),
                    mode.name(),
                    match mode {
                        Mode::References => " (it has reference pictures)",
                        Mode::Frames => "",
                    }
                ));
                continue;
            }
            out.push(l);
        }
        Ok((out, notes))
    }
}

/// The prompt with each picked LoRA's trigger words, where the prompt does
/// not already say them (any case): ahead of the timeline, where H3 reads
/// the clip's subject. A model that wrote H3's sections itself gets them at
/// the start of its description field, not ahead of the header, where they
/// would sit outside every field H3 reads (review of #739).
pub fn with_triggers(prompt: &str, picked: &[&Lora]) -> String {
    let lower = prompt.to_lowercase();
    let missing: Vec<&str> = picked
        .iter()
        .filter_map(|l| l.trigger.as_deref())
        .filter(|t| !lower.contains(&t.to_lowercase()))
        .collect();
    if missing.is_empty() {
        return prompt.to_string();
    }
    let words = format!("{}. ", missing.join(", "));
    for field in [
        "detailed_description:",
        "integrated_multimodal_description:",
    ] {
        if let Some(at) = prompt.find(field) {
            let mut start = at + field.len();
            // After the header's own spacing, so the field reads on.
            start += prompt[start..].len() - prompt[start..].trim_start().len();
            return format!("{}{words}{}", &prompt[..start], &prompt[start..]);
        }
    }
    format!("{words}{prompt}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[[lora]]
name = "wave"
about = "a friendly wave"
file = "wave_h3.safetensors"
strength = 0.8
trigger = "waving hello"

[[lora]]
name = "spin"
about = "a gentle sideways tilt"
references = "spin_ref.safetensors"

[[lora]]
name = "detail"
about = "fine detail"
frames = "detail_fl.safetensors"
references = "detail_ref.safetensors"
strength = 0.6
always = true

[[lora]]
name = "bad/name"
about = "x"
file = "x.safetensors"

[[lora]]
name = "nofile"
about = "x"

[[lora]]
name = "escape"
about = "x"
file = "../secrets.safetensors"

[[lora]]
name = "loud"
about = "x"
file = "loud.safetensors"
strength = 5
"#;

    #[test]
    fn a_bad_entry_costs_only_itself() {
        let (r, skipped) = Registry::parse(SAMPLE);
        assert_eq!(
            r.loras.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            ["wave", "spin", "detail"]
        );
        assert_eq!(skipped.len(), 4, "{skipped:?}");
        let wave = r.get("wave").unwrap();
        assert_eq!(wave.file(Mode::Frames), Some("wave_h3.safetensors"));
        assert_eq!(wave.file(Mode::References), Some("wave_h3.safetensors"));
        assert_eq!(wave.strength, 0.8);
        assert_eq!(r.get("spin").unwrap().file(Mode::Frames), None);
    }

    #[test]
    fn a_file_that_does_not_read_is_empty_and_says_why() {
        let (r, skipped) = Registry::parse("[[lora]\nname=");
        assert!(r.loras.is_empty());
        assert_eq!(skipped.len(), 1);
    }

    #[test]
    fn pick_refuses_unknown_repeated_too_many_and_wrong_mode() {
        let (mut r, _) = Registry::parse(SAMPLE);
        r.loras.retain(|l| !l.always);
        assert!(r.pick(&Value::Null, Mode::Frames).unwrap().0.is_empty());
        assert_eq!(
            r.pick(&json!(["wave"]), Mode::Frames).unwrap().0[0].name,
            "wave"
        );
        assert_eq!(r.pick(&json!("wave"), Mode::Frames).unwrap().0.len(), 1);
        assert!(r
            .pick(&json!(["nope"]), Mode::Frames)
            .unwrap_err()
            .to_string()
            .contains("\"wave\""));
        assert!(r.pick(&json!(["wave", "wave"]), Mode::Frames).is_err());
        assert!(r.pick(&json!(["spin"]), Mode::Frames).unwrap().1[0].contains("has no frames file"));
        // Named twice, even when left out: refused (review of #742).
        assert!(r.pick(&json!(["spin", "spin"]), Mode::Frames).is_err());
        assert_eq!(
            r.pick(&json!(["wave", "spin"]), Mode::References)
                .unwrap()
                .0
                .len(),
            2
        );
        assert!(r
            .pick(&json!(["wave", "spin", "a", "b", "c"]), Mode::References)
            .unwrap_err()
            .to_string()
            .contains("at most"));
        assert!(r.pick(&json!(7), Mode::Frames).is_err());
    }

    #[test]
    fn always_on_loras_ride_every_clip_and_are_never_offered() {
        let (r, _) = Registry::parse(SAMPLE);
        let names = |v: Vec<&Lora>| v.iter().map(|l| l.name.clone()).collect::<Vec<_>>();
        assert_eq!(
            names(r.pick(&Value::Null, Mode::Frames).unwrap().0),
            ["detail"]
        );
        assert_eq!(
            names(r.pick(&json!(["wave"]), Mode::References).unwrap().0),
            ["detail", "wave"]
        );
        // Naming it is harmless, and does not count against the cap.
        assert_eq!(
            names(r.pick(&json!(["detail", "wave"]), Mode::Frames).unwrap().0),
            ["detail", "wave"]
        );
        let s = r.schema().unwrap();
        assert_eq!(s["items"]["enum"], json!(["wave", "spin"]));
        assert!(!s["description"].as_str().unwrap().contains("detail"));
        // A registry of only always-on LoRAs offers nothing to pick.
        let (only, _) = Registry::parse(
            "[[lora]]\nname = \"d\"\nabout = \"x\"\nfile = \"d.safetensors\"\nalways = true\n",
        );
        assert!(only.schema().is_none());
        assert_eq!(only.pick(&Value::Null, Mode::Frames).unwrap().0.len(), 1);
    }

    #[test]
    fn the_schema_lists_names_and_is_absent_with_no_loras() {
        assert!(Registry::default().schema().is_none());
        let (r, _) = Registry::parse(SAMPLE);
        let s = r.schema().unwrap();
        assert_eq!(s["maxItems"], json!(MAX_PER_CLIP));
        let d = s["description"].as_str().unwrap();
        assert!(
            d.contains("wave — a friendly wave") && d.contains("references mode only"),
            "{d}"
        );
    }

    #[test]
    fn triggers_are_added_once_and_only_when_missing() {
        let (r, _) = Registry::parse(SAMPLE);
        let wave = r.get("wave").unwrap();
        assert_eq!(with_triggers("A park.", &[wave]), "waving hello. A park.");
        assert_eq!(
            with_triggers("Maya is Waving Hello in a park.", &[wave]),
            "Maya is Waving Hello in a park."
        );
        assert_eq!(
            with_triggers("A park.", &[r.get("spin").unwrap()]),
            "A park."
        );
        // Inside a field the model wrote, never ahead of its header.
        assert_eq!(
            with_triggers("detailed_description:\n[Shot 1] A park.", &[wave]),
            "detailed_description:\nwaving hello. [Shot 1] A park."
        );
        assert_eq!(
            with_triggers("integrated_multimodal_description: A park.", &[wave]),
            "integrated_multimodal_description: waving hello. A park."
        );
    }
}
