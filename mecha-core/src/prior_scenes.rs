//! Earlier picture calls, sent as the scenes they drew.
//!
//! A persona copies its own earlier `image_generate` calls. After a few
//! pictures its calls in the chat are bare lists of who stands where, the
//! scene's actions and clothes carried by the record rather than the call,
//! and it copies that shape onto the next ask: the owner's request reached
//! the call in 3 of 25 replays (mecha-a3, 2026-10-08). With those earlier
//! calls shown as the full scene each one drew, the action came back (5 of 5,
//! 9 of 9 and 10 of 10 on the turns that had lost it).
//!
//! A [`RequestView`]: the stored conversation is untouched, and only the
//! request carries the rewrite. Each rewrite is read from the landed record of
//! that call's picture, which is fixed by the picture's content hash, so a
//! rewritten call reads the same on every later request and the cached prefix
//! holds. A record whose origin is not clean is never rewritten in: its words
//! would land in the model's own past turns, where nothing marks them as
//! another's (mecha-05's review); that call is sent as it was made.

use std::borrow::Cow;
use std::path::PathBuf;

use crate::agent::RequestView;
use crate::message::{Block, Message};
use crate::scene::{Origin, Scene, SceneSlot, Setting};

/// The view's name, recorded as `RunConfig::request_view`.
pub const NAME: &str = "picture-scenes";

/// The persona chat's view of its earlier picture calls.
pub struct PriorScenes {
    /// The chat's workspace, where each picture's manifest sits beside it.
    workspace: PathBuf,
    /// The chat's scene store, whose index holds each picture's record.
    slot: SceneSlot,
}

impl PriorScenes {
    pub fn new(workspace: PathBuf, slot: SceneSlot) -> Self {
        PriorScenes { workspace, slot }
    }

    /// The record of the picture a result names: the first `images/….png` in
    /// it, that picture's manifest, and the scene hash the manifest carries.
    /// Only a plain file name under `images/` is followed: the path came
    /// from a tool result, and nothing here walks out of the workspace.
    fn record_for(&self, result: &str) -> Option<Scene> {
        let at = result.find("images/")?;
        let name: String = result[at + "images/".len()..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            .collect();
        let stem = name.strip_suffix(".png")?;
        if stem.is_empty() || stem.contains("..") {
            return None;
        }
        let manifest = self.workspace.join("images").join(format!("{stem}.json"));
        let m: serde_json::Value = serde_json::from_slice(&std::fs::read(manifest).ok()?).ok()?;
        let hash = m["scene"]["picture"].as_str()?;
        self.slot.lookup_hash(hash)
    }
}

/// A recorded scene as a call would send it: mecha-a3's measured shape —
/// the setting (words, or the photo by path), light, camera, style and
/// `together` when set, and each person's who, where, wearing, doing and
/// expression, empty ones left out.
pub fn full_scene(scene: &Scene) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    match scene.setting.as_ref().map(|f| &f.value) {
        Some(Setting::Words { text }) if !text.trim().is_empty() => {
            out.insert("setting".into(), text.clone().into());
        }
        Some(Setting::Photo { path, .. }) => {
            out.insert("setting".into(), serde_json::json!({ "photo": path }));
        }
        _ => {}
    }
    for (k, f) in [
        ("light", &scene.light),
        ("camera", &scene.camera),
        ("style", &scene.style),
        ("together", &scene.together),
    ] {
        if let Some(f) = f.as_ref().filter(|f| !f.value.trim().is_empty()) {
            out.insert(k.into(), f.value.clone().into());
        }
    }
    let people: Vec<serde_json::Value> = scene
        .people
        .iter()
        .map(|p| {
            let mut m = serde_json::Map::new();
            m.insert("who".into(), crate::picture::shown(&p.who).into());
            if let Some(at) = p.at {
                m.insert("where".into(), where_word(at).into());
            }
            for (k, v) in [
                ("wearing", &p.wearing),
                ("doing", &p.doing),
                ("expression", &p.expression),
            ] {
                if !v.trim().is_empty() {
                    m.insert(k.into(), v.clone().into());
                }
            }
            serde_json::Value::Object(m)
        })
        .collect();
    out.insert("people".into(), people.into());
    serde_json::Value::Object(out)
}

/// A place as the call's schema spells it.
fn where_word(at: crate::scene::Where) -> &'static str {
    use crate::scene::Where;
    match at {
        Where::Left => "left",
        Where::Centre => "centre",
        Where::Right => "right",
        Where::Background => "background",
    }
}

impl RequestView for PriorScenes {
    fn name(&self) -> &str {
        NAME
    }

    fn view<'a>(
        &self,
        messages: Cow<'a, [Message]>,
        answering: Option<usize>,
    ) -> Cow<'a, [Message]> {
        // Only turns before the one being answered: this run's own calls
        // are its real output in flight.
        let end = answering.unwrap_or(messages.len()).min(messages.len());
        let mut results: std::collections::HashMap<&str, &str> = Default::default();
        for m in &messages[..end] {
            for b in &m.content {
                if let Block::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } = b
                {
                    results.insert(tool_use_id.as_str(), content.as_str());
                }
            }
        }
        let mut rewrites: Vec<(usize, usize, serde_json::Value)> = Vec::new();
        for (i, m) in messages[..end].iter().enumerate() {
            for (j, b) in m.content.iter().enumerate() {
                let Block::ToolUse { id, name, input } = b else {
                    continue;
                };
                if name != "image_generate" {
                    continue;
                }
                let Some(record) = results.get(id.as_str()).and_then(|r| self.record_for(r)) else {
                    continue;
                };
                // Only a scene the record can say in full, and only one of
                // clean origin (see the module doc).
                if record.people.is_empty() || record.origin() != Origin::Clean {
                    continue;
                }
                let mut rewritten = serde_json::Map::new();
                if let Some(p) = input.get("picture").filter(|p| p.is_string()) {
                    rewritten.insert("picture".into(), p.clone());
                }
                rewritten.insert("scene".into(), full_scene(&record));
                rewrites.push((i, j, serde_json::Value::Object(rewritten)));
            }
        }
        if rewrites.is_empty() {
            return messages;
        }
        let mut owned = messages.into_owned();
        for (i, j, input) in rewrites {
            if let Block::ToolUse { input: slot, .. } = &mut owned[i].content[j] {
                *slot = input;
            }
        }
        Cow::Owned(owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Field, Person, Where, Who};

    fn temp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("mecha-prior-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(d.join("images")).unwrap();
        d
    }

    fn clean<T>(value: T) -> Option<Field<T>> {
        Some(Field {
            value,
            origin: Origin::Clean,
        })
    }

    /// A landed record for a picture, its manifest beside it, and the view.
    fn staged(origin: Origin) -> (PathBuf, PriorScenes) {
        let dir = temp();
        let slot = SceneSlot {
            chat_copy: dir.join("chat.scene.json"),
            store: dir.join("scene"),
            chat: "chat-a".into(),
            from_latest: false,
        };
        let hash = "a".repeat(64);
        let scene = Scene {
            setting: clean(Setting::Words {
                text: "a quiet harbour".into(),
            }),
            together: Some(Field {
                value: "Maya hands John a cup".into(),
                origin,
            }),
            people: vec![
                Person {
                    who: Who::Library("maya".into()),
                    at: Some(Where::Left),
                    wearing: "a coat".into(),
                    doing: String::new(),
                    expression: String::new(),
                    origin: Origin::Clean,
                },
                Person {
                    who: Who::Library("john".into()),
                    at: Some(Where::Right),
                    wearing: "a suit".into(),
                    doing: "smiling".into(),
                    expression: String::new(),
                    origin: Origin::Clean,
                },
            ],
            picture: Some(hash.clone()),
            ..Default::default()
        };
        slot.land(&scene).unwrap();
        std::fs::write(
            dir.join("images/20261008-1-1.json"),
            serde_json::json!({"scene": {"picture": hash}}).to_string(),
        )
        .unwrap();
        (dir.clone(), PriorScenes::new(dir, slot))
    }

    /// An earlier turn's bare call and its result, then this turn.
    fn history() -> Vec<Message> {
        vec![
            Message::user("draw us at the harbour"),
            Message::assistant(vec![Block::ToolUse {
                id: "c1".into(),
                name: "image_generate".into(),
                input: serde_json::json!({"scene": {"people": [{"who": "Maya"}, {"who": "John"}]}}),
            }]),
            Message::tool_results(vec![Block::ToolResult {
                tool_use_id: "c1".into(),
                content: "image: images/20261008-1-1.png\nA new picture.".into(),
                is_error: false,
            }]),
            Message::user("now another"),
            Message::assistant(vec![Block::ToolUse {
                id: "c2".into(),
                name: "image_generate".into(),
                input: serde_json::json!({"scene": {"people": [{"who": "Maya"}]}}),
            }]),
            Message::tool_results(vec![Block::ToolResult {
                tool_use_id: "c2".into(),
                content: "image: images/20261008-1-1.png".into(),
                is_error: false,
            }]),
        ]
    }

    fn input_of(m: &Message) -> serde_json::Value {
        m.content
            .iter()
            .find_map(|b| match b {
                Block::ToolUse { input, .. } => Some(input.clone()),
                _ => None,
            })
            .unwrap()
    }

    /// An earlier call reads as the full scene it drew, the same bytes on
    /// every request; this run's own call is left as made.
    #[test]
    fn an_earlier_call_is_sent_as_the_scene_it_drew() {
        let (dir, view) = staged(Origin::Clean);
        let msgs = history();
        let out = view.view(Cow::Borrowed(&msgs), Some(3));
        let first = input_of(&out[1]);
        assert_eq!(first["scene"]["setting"], "a quiet harbour");
        assert_eq!(first["scene"]["together"], "Maya hands John a cup");
        assert_eq!(first["scene"]["people"][0]["who"], "Maya");
        assert_eq!(first["scene"]["people"][0]["where"], "left");
        assert_eq!(first["scene"]["people"][1]["doing"], "smiling");
        assert!(
            first["scene"]["people"][0].get("doing").is_none(),
            "empties left out"
        );
        // At or after the turn being answered: unchanged.
        assert_eq!(input_of(&out[4]), input_of(&msgs[4]));
        // Deterministic: the same bytes on the next request.
        let again = view.view(Cow::Borrowed(&msgs), Some(3));
        assert_eq!(
            serde_json::to_string(&*out).unwrap(),
            serde_json::to_string(&*again).unwrap()
        );
        // The stored history is untouched.
        assert_eq!(
            input_of(&msgs[1])["scene"]["people"][0],
            serde_json::json!({"who": "Maya"})
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// A record with any untrusted field is never rewritten into the model's
    /// own turns; nor is a call whose picture has no manifest, or a path that
    /// tries to leave `images/`.
    #[test]
    fn an_untrusted_or_unknown_record_leaves_the_call_as_made() {
        let (dir, view) = staged(Origin::Untrusted);
        let msgs = history();
        let out = view.view(Cow::Borrowed(&msgs), Some(3));
        assert!(matches!(out, Cow::Borrowed(_)), "nothing rewritten");
        let (dir2, view) = staged(Origin::Clean);
        let mut msgs = history();
        if let Block::ToolResult { content, .. } = &mut msgs[2].content[0] {
            *content = "image: images/../../secret.png".into();
        }
        assert!(matches!(
            view.view(Cow::Borrowed(&msgs), Some(3)),
            Cow::Borrowed(_)
        ));
        for d in [dir, dir2] {
            std::fs::remove_dir_all(d).ok();
        }
    }
}
