//! What a persona's picture shows now: the scene record
//! (`IMAGE-SCENE-DESIGN.md` §5.1, §5.6, step 2 of §10).
//!
//! A scene is the place, the people in it (with what they wear and do), the
//! camera and the style. It is the record, and a picture is a render of it.
//! Each render that lands advances it:
//! - a new picture defines it afresh;
//! - an edit changes what it declared and keeps the rest.
//!
//! Three files, all written by the harness and none in the model's jail:
//! - **The chat's copy** beside its transcript. A chat starts from the
//!   persona's latest and works on its own copy (R7).
//! - **The persona's latest** (`scene/latest.json`). The last render to land
//!   wins.
//! - **The index** (`scene/index/<sha256>.json`): the scene each picture was
//!   rendered as, keyed by the picture's bytes. A picture attached in another
//!   chat is found by its content, never through a workspace manifest, which
//!   a run can write.
//!
//! **Origin is per field, and a scene's origin is the union over its
//! fields.** A clean write over a scene with one untrusted field leaves that
//! field, and the scene, untrusted. Unknown is untrusted: a record with no
//! origin reads that way, never as the default clean. Nothing here reads a
//! scene into a prompt yet: that is step 4, which carries the origin by a
//! stem.
//!
//! An incognito chat never holds a slot: incognito is the assistant's chat,
//! and the assistant has no scene store. So R9's "never writes back" holds by
//! construction, with no switch to turn.

use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::path::{Path, PathBuf};

/// Where a field's value came from: a run with no untrusted input in it, or
/// anything else. A closed enum written to a store, so an unknown value
/// reads untrusted rather than failing the record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Clean,
    #[default]
    #[serde(other)]
    Untrusted,
}

impl Origin {
    /// The origin of a write from a run with this taint. Unstamped is
    /// untrusted: a run wired outside the loop must not write a clean label
    /// by omission (`ToolCtx::taint`).
    pub fn of(taint: Option<&crate::agent::Taint>) -> Self {
        match taint {
            Some(t) if !t.untrusted => Origin::Clean,
            _ => Origin::Untrusted,
        }
    }

    pub fn union(self, other: Origin) -> Origin {
        if self == Origin::Clean && other == Origin::Clean {
            Origin::Clean
        } else {
            Origin::Untrusted
        }
    }
}

/// A value and the origin of the write that set it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field<T> {
    pub value: T,
    #[serde(default)]
    pub origin: Origin,
}

/// Where the scene is: described in words, or a picture it was placed in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Place {
    Words {
        text: String,
    },
    /// `path` is a label, local to the chat that placed the scene there;
    /// `hash` is the handle any other chat finds the picture by.
    Picture {
        path: String,
        hash: String,
    },
}

/// Someone in the scene, by library name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Person {
    pub name: String,
    #[serde(default)]
    pub wearing: String,
    #[serde(default)]
    pub doing: String,
    #[serde(default)]
    pub origin: Origin,
}

/// The scene record.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Scene {
    /// Read leniently: a place this build does not know (a kind a later step
    /// adds) reads as no place, and the rest of the record survives. The
    /// store is a wire format (review of #589).
    #[serde(default, deserialize_with = "lenient_place")]
    pub place: Option<Field<Place>>,
    #[serde(default)]
    pub people: Vec<Person>,
    #[serde(default)]
    pub camera: Option<Field<String>>,
    #[serde(default)]
    pub style: Option<Field<String>>,
    /// The content hash of the picture this scene was last rendered as.
    #[serde(default)]
    pub picture: Option<String>,
    /// The chat whose render last advanced it.
    #[serde(default)]
    pub chat: Option<String>,
}

/// What one landed render says about its scene.
#[derive(Debug, Clone, Default)]
pub struct Change {
    /// A new picture: the scene is defined afresh, nothing carried.
    pub fresh: bool,
    /// The place, when the render says where. An edit that does not keeps
    /// the scene's; `fallback_place` is used only when there is none yet.
    pub place: Option<Place>,
    pub fallback_place: Option<Place>,
    /// People the call itself named, with what it says they wear and do.
    pub declared: Vec<(String, String, String)>,
    /// People carried over from the picture's record or scene, by name, with
    /// what that record says. They keep the scene's entry when it has one.
    /// Otherwise their origin is untrusted, because the record came from a
    /// file a run could write.
    pub carried: Vec<(String, String, String)>,
    pub camera: Option<String>,
    pub style: Option<String>,
    /// The landed picture's content hash.
    pub picture: String,
}

impl Scene {
    /// The scene's origin: the union over every field it holds.
    pub fn origin(&self) -> Origin {
        let mut o = Origin::Clean;
        if let Some(p) = &self.place {
            o = o.union(p.origin);
        }
        for p in &self.people {
            o = o.union(p.origin);
        }
        if let Some(c) = &self.camera {
            o = o.union(c.origin);
        }
        if let Some(s) = &self.style {
            o = o.union(s.origin);
        }
        o
    }

    /// The scene after a render lands: `prev` advanced by `change`, written
    /// by a run of origin `by`. Every field the change sets takes `by`; every
    /// field it leaves keeps its own.
    pub fn advance(prev: Option<&Scene>, change: Change, by: Origin, chat: &str) -> Scene {
        let prev = if change.fresh { None } else { prev };
        let set = |v: String| Field {
            value: v,
            origin: by,
        };
        let place = match change.place {
            Some(p) => Some(Field {
                value: p,
                origin: by,
            }),
            None => prev.and_then(|s| s.place.clone()).or_else(|| {
                change.fallback_place.map(|p| Field {
                    value: p,
                    origin: by,
                })
            }),
        };
        let mut people: Vec<Person> = Vec::new();
        let mut push = |p: Person| {
            if !people.iter().any(|q| q.name == p.name) {
                people.push(p);
            }
        };
        for (name, wearing, doing) in change.declared {
            push(Person {
                name: name.trim().to_lowercase(),
                wearing,
                doing,
                origin: by,
            });
        }
        for (name, wearing, doing) in change.carried {
            let name = name.trim().to_lowercase();
            match prev.and_then(|s| s.people.iter().find(|p| p.name == name)) {
                Some(kept) => push(kept.clone()),
                None => push(Person {
                    name,
                    wearing,
                    doing,
                    origin: Origin::Untrusted,
                }),
            }
        }
        Scene {
            place,
            people,
            camera: match change.camera {
                Some(c) => Some(set(c)),
                None => prev.and_then(|s| s.camera.clone()),
            },
            style: match change.style {
                Some(s) => Some(set(s)),
                None => prev.and_then(|s| s.style.clone()),
            },
            picture: Some(change.picture),
            chat: Some(chat.to_string()),
        }
    }
}

/// A place that does not parse is no place, never a lost record.
fn lenient_place<'de, D>(d: D) -> std::result::Result<Option<Field<Place>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(v.and_then(|v| serde_json::from_value(v).ok()))
}

/// A picture's content hash, as the index keys it.
pub fn hash(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Where one chat's scene lives, stamped on its runs by the front end (a
/// persona chat), never by a model.
#[derive(Debug, Clone)]
pub struct SceneSlot {
    /// The chat's own copy, beside its transcript.
    pub chat_copy: PathBuf,
    /// The persona's `scene/` folder: its latest scene and the index.
    pub store: PathBuf,
    /// The chat's id, recorded on the scenes it advances.
    pub chat: String,
}

impl SceneSlot {
    /// The chat's scene now: its own copy, else the persona's latest, which
    /// the chat starts from (R7), else none.
    pub fn current(&self) -> Option<Scene> {
        read(&self.chat_copy).or_else(|| read(&self.store.join("latest.json")))
    }

    /// Record a landed render: the chat's copy, the persona's latest (the
    /// last to land wins), and the index entry for its picture.
    ///
    /// The index first: if it cannot be written, nothing is, rather than
    /// leaving a latest that names a picture no lookup can find (review of
    /// #589).
    pub fn land(&self, scene: &Scene) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(scene).map_err(std::io::Error::other)?;
        if let Some(h) = scene.picture.as_deref().filter(|h| is_hash(h)) {
            write_atomic(&self.store.join("index").join(format!("{h}.json")), &bytes)?;
        }
        write_atomic(&self.store.join("latest.json"), &bytes)?;
        write_atomic(&self.chat_copy, &bytes)
    }

    /// The scene a picture with these bytes was rendered as, from the
    /// persona's index only.
    pub fn lookup(&self, bytes: &[u8]) -> Option<Scene> {
        self.lookup_hash(&hash(bytes))
    }

    /// The same, by a hash already taken.
    pub fn lookup_hash(&self, h: &str) -> Option<Scene> {
        is_hash(h)
            .then(|| read(&self.store.join("index").join(format!("{h}.json"))))
            .flatten()
    }
}

/// Remove what one chat's renders left: the chat's own copy (or its next
/// render would land the scene back, review of #589), each index entry that
/// chat last advanced, and the latest when it was that chat's. `persona_dir`
/// is the persona's folder. The count removed; nothing there is zero, never
/// an error. A chat id is a file name here, so it must be one.
pub fn forget_chat(persona_dir: &Path, chat: &str) -> std::io::Result<usize> {
    if chat.is_empty()
        || chat.len() > 128
        || !chat
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(std::io::Error::other(format!("invalid chat id {chat:?}")));
    }
    let store = persona_dir.join("scene");
    let mut gone = 0;
    let copy = persona_dir
        .join("sessions")
        .join(format!("{chat}.scene.json"));
    match std::fs::remove_file(&copy) {
        Ok(()) => gone += 1,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let mut drop_if_theirs = |path: &Path| -> std::io::Result<()> {
        if read(path).is_some_and(|s| s.chat.as_deref() == Some(chat)) {
            std::fs::remove_file(path)?;
            gone += 1;
        }
        Ok(())
    };
    drop_if_theirs(&store.join("latest.json"))?;
    match std::fs::read_dir(store.join("index")) {
        Ok(entries) => {
            for e in entries {
                drop_if_theirs(&e?.path())?;
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    Ok(gone)
}

fn is_hash(h: &str) -> bool {
    h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A scene file, or `None` when there is none or it cannot be read. An
/// unreadable one is said in the log rather than taken for an empty scene
/// silently.
fn read(path: &Path) -> Option<Scene> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!("scene record {} unreadable: {e}", path.display());
            return None;
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!("scene record {} does not parse: {e}", path.display());
            None
        }
    }
}

/// Write by rename, so a reader never sees half a scene.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("a scene path has no folder"))?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot() -> (SceneSlot, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("scene-test-{}", uuid::Uuid::new_v4().simple()));
        (
            SceneSlot {
                chat_copy: root.join("persona/sessions/c1.scene.json"),
                store: root.join("persona/scene"),
                chat: "c1".into(),
            },
            root,
        )
    }

    fn change(fresh: bool, picture: &str) -> Change {
        Change {
            fresh,
            picture: hash(picture.as_bytes()),
            ..Change::default()
        }
    }

    /// A clean write over a scene with an untrusted field leaves the field,
    /// and so the scene, untrusted; only what the clean write replaced takes
    /// its origin (§5.1).
    #[test]
    fn origin_is_per_field_and_the_scene_takes_the_union() {
        let mut first = change(true, "a");
        first.place = Some(Place::Words {
            text: "a beach".into(),
        });
        first.declared = vec![("maya".into(), "a dress".into(), "walking".into())];
        let s1 = Scene::advance(None, first, Origin::Untrusted, "c1");
        assert_eq!(s1.origin(), Origin::Untrusted);
        let mut edit = change(false, "b");
        edit.camera = Some("from above".into());
        edit.carried = vec![("maya".into(), String::new(), String::new())];
        let s2 = Scene::advance(Some(&s1), edit, Origin::Clean, "c1");
        assert_eq!(s2.camera.as_ref().unwrap().origin, Origin::Clean);
        assert_eq!(
            s2.people[0].origin,
            Origin::Untrusted,
            "kept, with its own origin"
        );
        assert_eq!(s2.place.as_ref().unwrap().origin, Origin::Untrusted);
        assert_eq!(s2.origin(), Origin::Untrusted);
        let mut fresh = change(true, "c");
        fresh.place = Some(Place::Words {
            text: "a park".into(),
        });
        let s3 = Scene::advance(Some(&s2), fresh, Origin::Clean, "c1");
        assert_eq!(
            s3.origin(),
            Origin::Clean,
            "a new picture replaces every field"
        );
    }

    /// Someone carried over from a record the scene does not hold is
    /// untrusted: the record is a file a run could write.
    #[test]
    fn a_carried_person_not_in_the_scene_is_untrusted() {
        let mut edit = change(false, "a");
        edit.carried = vec![("john".into(), "an apron".into(), "cooking".into())];
        let s = Scene::advance(None, edit, Origin::Clean, "c1");
        assert_eq!(s.people[0].origin, Origin::Untrusted);
        assert_eq!(s.people[0].wearing, "an apron");
    }

    /// A place of a kind this build does not know reads as no place, and
    /// the rest of the record survives (review of #589).
    #[test]
    fn an_unknown_place_costs_the_place_only() {
        let s: Scene = serde_json::from_str(
            r#"{"place": {"value": {"kind": "hologram"}, "origin": "clean"},
                "people": [{"name": "maya", "origin": "clean"}], "chat": "c1"}"#,
        )
        .unwrap();
        assert!(s.place.is_none());
        assert_eq!(s.people[0].name, "maya");
        assert_eq!(s.chat.as_deref(), Some("c1"));
    }

    /// A record with no origin, or one this build does not know, reads
    /// untrusted.
    #[test]
    fn an_unknown_or_missing_origin_reads_untrusted() {
        let s: Scene = serde_json::from_str(
            r#"{"people": [{"name": "maya"}, {"name": "john", "origin": "sideways"}]}"#,
        )
        .unwrap();
        assert!(s.people.iter().all(|p| p.origin == Origin::Untrusted));
    }

    /// Landing writes the chat's copy, the persona's latest and the index;
    /// a chat with no copy starts from the latest (R7); a picture's bytes find
    /// its scene; the last render to land wins the latest.
    #[test]
    fn landing_writes_three_files_and_a_picture_finds_its_scene() {
        let (slot, root) = slot();
        let mut c = change(true, "picture one");
        c.declared = vec![("maya".into(), "a coat".into(), "sitting".into())];
        let s = Scene::advance(slot.current().as_ref(), c, Origin::Clean, "c1");
        slot.land(&s).unwrap();
        assert!(slot.chat_copy.is_file());
        assert_eq!(slot.lookup(b"picture one"), Some(s.clone()));
        assert_eq!(slot.lookup(b"another picture"), None);
        let other = SceneSlot {
            chat_copy: root.join("persona/sessions/c2.scene.json"),
            chat: "c2".into(),
            ..slot.clone()
        };
        assert_eq!(
            other.current(),
            Some(s),
            "a new chat starts from the latest"
        );
        let mut c2 = change(true, "picture two");
        c2.declared = vec![("maya".into(), "a hat".into(), "laughing".into())];
        let s2 = Scene::advance(other.current().as_ref(), c2, Origin::Clean, "c2");
        other.land(&s2).unwrap();
        assert_eq!(
            slot.current().unwrap().people[0].wearing,
            "a coat",
            "each chat keeps its own copy"
        );
        let latest = read(&slot.store.join("latest.json")).unwrap();
        assert_eq!(latest.chat.as_deref(), Some("c2"), "the last to land wins");
        std::fs::remove_dir_all(root).ok();
    }

    /// Forgetting a chat removes what its renders left in the persona's
    /// store, and nothing another chat wrote.
    #[test]
    fn forgetting_a_chat_removes_its_scenes_only() {
        let (slot, root) = slot();
        let mut c = change(true, "one");
        c.declared = vec![("maya".into(), "a coat".into(), "sitting".into())];
        slot.land(&Scene::advance(None, c, Origin::Clean, "c1"))
            .unwrap();
        let other = SceneSlot {
            chat_copy: root.join("persona/sessions/c2.scene.json"),
            chat: "c2".into(),
            ..slot.clone()
        };
        other
            .land(&Scene::advance(
                None,
                change(true, "two"),
                Origin::Clean,
                "c2",
            ))
            .unwrap();
        let persona = root.join("persona");
        assert_eq!(
            forget_chat(&persona, "c1").unwrap(),
            2,
            "c1's own copy and its index entry"
        );
        assert!(slot.lookup(b"one").is_none());
        assert!(slot.lookup(b"two").is_some());
        // The forgotten chat now starts from the latest, which is c2's: its
        // own scene is gone, and its next render cannot land it back.
        assert_eq!(slot.current().unwrap().chat.as_deref(), Some("c2"));
        assert_eq!(
            forget_chat(&persona, "c2").unwrap(),
            3,
            "c2's copy, its entry and the latest"
        );
        assert_eq!(slot.current(), None);
        assert_eq!(forget_chat(&root.join("nowhere"), "c1").unwrap(), 0);
        assert!(forget_chat(&persona, "../escape").is_err());
        std::fs::remove_dir_all(root).ok();
    }
}
