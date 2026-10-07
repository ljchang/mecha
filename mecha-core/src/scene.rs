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
    /// A place this build cannot read: a kind it does not know, or a known
    /// kind gone wrong. It is no place to draw on and is never clean, so a
    /// scene holding one reads untrusted (review of #589, pass 8).
    Unknown,
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
    /// People the call changed in part: their new fields as sent, beside
    /// the rest of their entry, under their old origin joined with the
    /// run's, so one new field never launders the others (review of #591).
    pub amended: Vec<(String, String, String)>,
    /// The call said `"cast": []`: the people have left the picture, so an
    /// edit keeps none of the scene's. Otherwise an edit keeps everyone the
    /// scene had that it did not name, as it keeps the place and the camera
    /// (review of #589, pass 4): a person whose library entry is gone is
    /// still in the picture.
    pub nobody: bool,
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
            // An unknown place is no place to keep: the canvas replaces it.
            None => prev
                .and_then(|s| s.place.clone())
                .filter(|p| p.value != Place::Unknown)
                .or_else(|| {
                    change.fallback_place.map(|p| Field {
                        value: p,
                        origin: by,
                    })
                }),
        };
        // Bounded here, where everything that reaches the store passes: a
        // carried person comes from a manifest a run can write, and an edit
        // keeps the people it did not name, so neither the count nor a name
        // may grow along a chain of edits (review of #589, pass 5). The
        // call's people come first, so the cap drops the oldest kept ones.
        let mut people: Vec<Person> = Vec::new();
        let cap = |s: String, n: usize| s.chars().take(n).collect::<String>();
        let mut push = |p: Person| {
            let p = Person {
                name: cap(p.name, crate::imagelib::MAX_NAME),
                wearing: cap(p.wearing, crate::imagelib::MAX_CAST_FIELD),
                doing: cap(p.doing, crate::imagelib::MAX_CAST_FIELD),
                ..p
            };
            if people.len() < MAX_PEOPLE && !people.iter().any(|q| q.name == p.name) {
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
        for (name, wearing, doing) in change.amended {
            let name = name.trim().to_lowercase();
            let was = prev
                .and_then(|s| s.people.iter().find(|p| p.name == name))
                .map_or(Origin::Untrusted, |p| p.origin);
            push(Person {
                name,
                wearing,
                doing,
                origin: was.union(by),
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
        if !change.nobody {
            for kept in prev.into_iter().flat_map(|s| s.people.iter()) {
                push(kept.clone());
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

/// The most people a scene keeps: twice what one picture draws with faces,
/// so a picture's record past `MAX_CAST` still fits, and a chain of edits
/// cannot grow the store without bound.
pub const MAX_PEOPLE: usize = 2 * crate::imagelib::MAX_CAST;

/// A place that does not parse is an unknown place, never a lost record,
/// and untrusted whatever origin it was written with: unknown is never clean,
/// and dropping it would drop the label that says so (review of #589, pass 8).
fn lenient_place<'de, D>(d: D) -> std::result::Result<Option<Field<Place>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(v.filter(|v| !v.is_null()).map(|v| {
        serde_json::from_value::<Field<Place>>(v)
            .ok()
            .filter(|f| f.value != Place::Unknown)
            .unwrap_or(Field {
                value: Place::Unknown,
                origin: Origin::Untrusted,
            })
    }))
}

/// The scene note's opening when every field came from a clean run
/// (`IMAGE-SCENE-DESIGN.md` §5.5, step 4). It rides in a run's notes, and
/// arms taint by its stem as memory's does: two stems, because notes arm at
/// every run start, and one stem arming both would make every chat with a
/// scene untrusted. Both arm `private`: a scene can carry an owner photo, or a
/// person from one, into a chat that never saw it.
pub const SCENE_STEM: &str = "(Where things stand in your pictures now, from the harness";
/// The opening when any field came from a run that read something from
/// outside.
pub const UNTRUSTED_SCENE_STEM: &str =
    "(Where things stand in your pictures now, some of it first read from outside, from the harness";

/// `(private, untrusted)` for a scene note, or `None` when `text` is not one.
pub fn stem_of(text: &str) -> Option<(bool, bool)> {
    let t = text.trim_start();
    if t.starts_with(UNTRUSTED_SCENE_STEM) {
        Some((true, true))
    } else if t.starts_with(SCENE_STEM) {
        Some((true, false))
    } else {
        None
    }
}

/// The most of a place described in words a note carries.
const NOTE_PLACE_CHARS: usize = 240;

/// The most of any other field (what someone wears or does, the camera) a
/// note carries.
const NOTE_FIELD_CHARS: usize = 160;

/// The scene as a run note: where the persona is, who is with her, what each
/// wears and does, and the camera. The present comes from here, so a
/// recalled day's outfit or blocking cannot stand in for it (§5.5). `me` is
/// the persona's own library character, said as "You". `None` when the
/// scene holds nothing to say.
pub fn note(scene: &Scene, me: Option<&str>) -> Option<String> {
    if scene.place.is_none() && scene.people.is_empty() && scene.camera.is_none() {
        return None;
    }
    // Every field is cut to a sentence's length, so the note rides on every
    // request at a bounded size whatever a scene holds (review of #590).
    let clip = |text: &str, n: usize| {
        let text = text.trim();
        let cut: String = text.chars().take(n).collect();
        let cut = if cut.len() < text.len() {
            format!("{}…", cut.rsplit_once(' ').map_or(cut.as_str(), |(a, _)| a))
        } else {
            cut
        };
        cut.trim_end_matches('.').to_string()
    };
    let mut out = String::new();
    match scene.place.as_ref().map(|p| &p.value) {
        Some(Place::Words { text }) => {
            out.push_str(&format!(" Place: {}.", clip(text, NOTE_PLACE_CHARS)));
        }
        Some(Place::Picture { .. }) => out.push_str(" Place: the picture you were placed in."),
        // Nothing to say about a place this build cannot read; the scene is
        // untrusted for it, so the note rides under the untrusted stem.
        Some(Place::Unknown) | None => {}
    }
    let me = me.map(|m| m.trim().to_lowercase());
    for p in &scene.people {
        let who = if me.as_deref() == Some(p.name.as_str()) {
            "You".to_string()
        } else {
            p.name
                .split(' ')
                .map(|w| {
                    let mut c = w.chars();
                    c.next()
                        .map(|f| f.to_uppercase().chain(c).collect::<String>())
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        let mut said = vec![];
        if !p.wearing.trim().is_empty() {
            said.push(format!("wearing {}", clip(&p.wearing, NOTE_FIELD_CHARS)));
        }
        if !p.doing.trim().is_empty() {
            said.push(clip(&p.doing, NOTE_FIELD_CHARS));
        }
        if said.is_empty() {
            let verb = if who == "You" { "are" } else { "is" };
            out.push_str(&format!(" {who} {verb} there."));
        } else {
            out.push_str(&format!(" {who}: {}.", said.join(", ")));
        }
    }
    if let Some(c) = &scene.camera {
        out.push_str(&format!(" Camera: {}.", clip(&c.value, NOTE_FIELD_CHARS)));
    }
    // Nothing to say is no note: an unknown place alone would otherwise send
    // the opening sentence by itself, and arm `untrusted` for no content
    // (review of #590).
    if out.is_empty() {
        return None;
    }
    let stem = match scene.origin() {
        Origin::Clean => SCENE_STEM,
        Origin::Untrusted => UNTRUSTED_SCENE_STEM,
    };
    Some(format!(
        "{stem}: what your last picture showed. Clothes and places come from here now, \
         not from a remembered day.){out}"
    ))
}

/// A picture's content hash, as the index keys it.
pub fn hash(bytes: &[u8]) -> String {
    crate::document::sha256_hex(bytes)
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
    /// the chat starts from (R7), else none. A copy that exists but cannot
    /// be read is none, never the latest: that is another chat's scene, and
    /// a broken copy is not a chat with no scene yet (review of #589, pass 8).
    pub fn current(&self) -> Option<Scene> {
        if self.chat_copy.exists() {
            read(&self.chat_copy)
        } else {
            read(&self.store.join("latest.json"))
        }
    }

    /// Record a landed render: the chat's copy, the persona's latest (the
    /// last to land wins), and the index entry for its picture.
    ///
    /// The index first: if it cannot be written, nothing is, rather than
    /// leaving a latest that names a picture no lookup can find (review of
    /// #589). The other two are not one write: a latest that lands with a
    /// failed chat copy leaves that chat a render behind, until its next.
    pub fn land(&self, scene: &Scene) -> std::io::Result<()> {
        // The id forgetting accepts, or a record could be written that
        // `forget_chat` refuses, and the chat's memories with it (review of
        // #589, pass 7).
        if !is_chat_id(&self.chat) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("`{}` is not a chat id", self.chat),
            ));
        }
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
pub fn forget_chat(persona_dir: &Path, chat: &str) -> std::io::Result<SceneForget> {
    if !is_chat_id(chat) {
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
    // A record that cannot be read cannot be shown to be this chat's, so it
    // is kept, and counted, so the owner is told rather than shown a clean
    // forget (review of #589, pass 9).
    let mut unreadable = 0;
    let mut drop_if_theirs = |path: &Path| -> std::io::Result<()> {
        match read(path) {
            Some(s) if s.chat.as_deref() == Some(chat) => {
                std::fs::remove_file(path)?;
                gone += 1;
            }
            Some(_) => {}
            None if path.exists() => unreadable += 1,
            None => {}
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
    Ok(SceneForget {
        removed: gone,
        unreadable,
    })
}

/// What [`forget_chat`] did: the records it removed, and the ones it could
/// not read and so kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SceneForget {
    pub removed: usize,
    pub unreadable: usize,
}

/// Whether `chat` can be a chat id here, where it is a file name: the shape
/// a session id has, and nothing that could walk out of a folder. A caller
/// that deletes elsewhere first asks this before deleting anything.
pub fn is_chat_id(chat: &str) -> bool {
    !chat.is_empty()
        && chat.len() <= 128
        && chat
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
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

    /// A record a run wrote cannot grow the store: a scene keeps at most
    /// `MAX_PEOPLE`, the call's people first, and no name longer than a
    /// library name, however long the chain of edits (review of #589,
    /// pass 5).
    #[test]
    fn a_scene_is_bounded_in_people_and_in_each_field() {
        let long = "n".repeat(5_000);
        let mut edit = change(false, "a");
        edit.declared = vec![("maya".into(), "a coat".into(), "reading".into())];
        edit.carried = (0..40)
            // Distinct before the cut, so the first round measures the cap,
            // not the dedupe of names the cut made equal (review of #589,
            // pass 9).
            .map(|i| (format!("p{i}-{long}"), "w".repeat(5_000), "d".into()))
            .collect();
        let mut s = Scene::advance(None, edit, Origin::Clean, "c1");
        assert_eq!(s.people.len(), MAX_PEOPLE, "the cap, on the first write");
        for _ in 0..5 {
            let mut next = change(false, "b");
            next.carried = (0..40)
                .map(|i| (format!("x{i}"), String::new(), String::new()))
                .collect();
            s = Scene::advance(Some(&s), next, Origin::Clean, "c1");
        }
        assert_eq!(s.people.len(), MAX_PEOPLE);
        assert!(s
            .people
            .iter()
            .all(|p| p.name.chars().count() <= crate::imagelib::MAX_NAME
                && p.wearing.chars().count() <= crate::imagelib::MAX_CAST_FIELD));
        let first = Scene::advance(
            None,
            Change {
                declared: vec![("maya".into(), String::new(), String::new())],
                carried: (0..40)
                    .map(|i| (format!("x{i}"), String::new(), String::new()))
                    .collect(),
                ..change(false, "c")
            },
            Origin::Clean,
            "c1",
        );
        assert_eq!(first.people[0].name, "maya", "the call's people come first");
    }

    /// A slot whose chat id forgetting would refuse writes nothing, so no
    /// record exists that cannot be forgotten (review of #589, pass 7).
    #[test]
    fn a_slot_with_a_bad_chat_id_lands_nothing() {
        let (good, root) = slot();
        let bad = SceneSlot {
            chat: "../c1".into(),
            ..good
        };
        let s = Scene::advance(None, change(true, "a"), Origin::Clean, "../c1");
        assert!(bad.land(&s).is_err());
        assert!(!root.join("persona/scene/latest.json").exists());
        assert!(!root.join("persona/scene/index").exists());
        std::fs::remove_dir_all(root).ok();
    }

    /// A chat whose own copy is broken has no scene; it does not start from
    /// another chat's latest, which is for a chat with no copy yet (review
    /// of #589, pass 8).
    #[test]
    fn a_broken_chat_copy_is_no_scene_not_the_latest() {
        let (slot, root) = slot();
        let s = Scene::advance(None, change(true, "a"), Origin::Clean, "c1");
        slot.land(&s).unwrap();
        assert!(slot.current().is_some());
        std::fs::write(&slot.chat_copy, b"{ not json").unwrap();
        assert!(slot.current().is_none());
        std::fs::remove_file(&slot.chat_copy).unwrap();
        assert_eq!(slot.current(), Some(s), "no copy yet: the latest");
        std::fs::remove_dir_all(root).ok();
    }

    /// A place this build cannot read is an unknown place, untrusted, and
    /// the rest of the record survives (review of #589, passes 1 and 8); an
    /// edit replaces it with its canvas.
    #[test]
    fn an_unknown_place_costs_the_place_only() {
        for place in [
            r#"{"value": {"kind": "hologram"}, "origin": "clean"}"#,
            r#"{"value": {"kind": "words"}, "origin": "clean"}"#,
            r#"{"value": {"kind": "unknown"}, "origin": "clean"}"#,
            r#"{"value": "a beach"}"#,
        ] {
            let s: Scene = serde_json::from_str(&format!(
                r#"{{"place": {place},
                    "people": [{{"name": "maya", "origin": "clean"}}], "chat": "c1"}}"#
            ))
            .unwrap();
            let p = s.place.as_ref().expect("kept, as unknown");
            assert_eq!(p.value, Place::Unknown, "{place}");
            // Unknown is never clean, whatever the record says (pass 8).
            assert_eq!(p.origin, Origin::Untrusted, "{place}");
            assert_eq!(s.origin(), Origin::Untrusted, "{place}");
            assert_eq!(s.people[0].name, "maya");
            assert_eq!(s.chat.as_deref(), Some("c1"));
            // And it stays so when written back and read again.
            let again: Scene = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
            assert_eq!(again.origin(), Origin::Untrusted);
        }
        let none: Scene = serde_json::from_str(r#"{"place": null, "chat": "c1"}"#).unwrap();
        assert!(none.place.is_none());
        let unknown: Scene =
            serde_json::from_str(r#"{"place": {"value": {"kind": "hologram"}}, "chat": "c1"}"#)
                .unwrap();
        let mut edit = change(false, "b");
        edit.fallback_place = Some(Place::Picture {
            path: "images/b.png".into(),
            hash: hash(b"b"),
        });
        let s = Scene::advance(Some(&unknown), edit, Origin::Clean, "c1");
        assert!(matches!(
            s.place.as_ref().unwrap().value,
            Place::Picture { .. }
        ));
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

    /// The note says where she is, who is with her and what each wears and
    /// does, the persona herself as "You"; its stem is untrusted when any
    /// field is; and a clean note arms `private` only, an untrusted one both.
    #[test]
    fn the_note_reads_the_scene_and_arms_by_its_stem() {
        let mut c = change(true, "a");
        c.place = Some(Place::Words {
            text: "a harbour".into(),
        });
        c.declared = vec![
            ("maya".into(), "a red coat".into(), "sitting".into()),
            ("mara quinn".into(), "a scarf".into(), String::new()),
        ];
        let clean = Scene::advance(None, c.clone(), Origin::Clean, "c1");
        let n = note(&clean, Some("Maya")).unwrap();
        assert!(n.starts_with(SCENE_STEM), "{n}");
        assert!(n.contains("Place: a harbour."), "{n}");
        assert!(n.contains("You: wearing a red coat, sitting."), "{n}");
        assert!(n.contains("Mara Quinn: wearing a scarf."), "{n}");
        let mut t = crate::agent::Taint::default();
        t.arm_for_notes(&[n]);
        assert!(t.private && !t.untrusted);
        let dirty = Scene::advance(None, c, Origin::Untrusted, "c1");
        let n = note(&dirty, None).unwrap();
        assert!(n.starts_with(UNTRUSTED_SCENE_STEM), "{n}");
        let mut t = crate::agent::Taint::default();
        t.arm_for_notes(&[n]);
        assert!(t.private && t.untrusted);
        assert_eq!(note(&Scene::default(), None), None);
        let mut long = change(true, "b");
        long.place = Some(Place::Words {
            text: "word ".repeat(100),
        });
        let n = note(&Scene::advance(None, long, Origin::Clean, "c1"), None).unwrap();
        assert!(n.len() < 600 && n.contains("…"), "{n}");
    }

    /// A scene with nothing to say gives no note, so an unknown place alone
    /// never arms `untrusted` with no content; and every field is cut, so the
    /// note is bounded whatever the scene holds (review of #590).
    #[test]
    fn a_note_says_something_or_nothing_and_is_bounded() {
        let unknown: Scene =
            serde_json::from_str(r#"{"place": {"value": {"kind": "hologram"}}, "chat": "c1"}"#)
                .unwrap();
        assert_eq!(note(&unknown, None), None);
        let mut big = change(true, "a");
        big.camera = Some("low ".repeat(2_000));
        big.declared = (0..MAX_PEOPLE)
            .map(|i| (format!("p{i}"), "silk ".repeat(100), "waving ".repeat(100)))
            .collect();
        let n = note(&Scene::advance(None, big, Origin::Clean, "c1"), None).unwrap();
        assert!(n.len() < 4_500, "{} bytes", n.len());
        assert!(n.contains("Camera: low") && n.contains("…"), "{n}");
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
        // A record nobody can read is kept, and said, not passed over as a
        // clean forget (review of #589, pass 9).
        std::fs::write(root.join("persona/scene/index/broken.json"), b"{ half").unwrap();
        assert_eq!(
            forget_chat(&persona, "c1").unwrap(),
            SceneForget {
                removed: 2,
                unreadable: 1
            },
            "c1's own copy and its index entry"
        );
        assert!(root.join("persona/scene/index/broken.json").exists());
        std::fs::remove_file(root.join("persona/scene/index/broken.json")).unwrap();
        assert!(slot.lookup(b"one").is_none());
        assert!(slot.lookup(b"two").is_some());
        // The forgotten chat now starts from the latest, which is c2's: its
        // own scene is gone, and its next render cannot land it back.
        assert_eq!(slot.current().unwrap().chat.as_deref(), Some("c2"));
        assert_eq!(
            forget_chat(&persona, "c2").unwrap().removed,
            3,
            "c2's copy, its entry and the latest"
        );
        assert_eq!(slot.current(), None);
        assert_eq!(
            forget_chat(&root.join("nowhere"), "c1").unwrap(),
            SceneForget::default()
        );
        assert!(forget_chat(&persona, "../escape").is_err());
        std::fs::remove_dir_all(root).ok();
    }
}
