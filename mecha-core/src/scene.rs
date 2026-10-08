//! What a picture shows: the scene record (`IMAGE-DESIGN.md` §4, §6).
//!
//! A picture is a render of a scene: its setting (everything but its people
//! and its words), its light, camera and style, what its people do with each
//! other, the words it renders, and its people, each with who they are, where
//! they stand, and what they wear, do and show on their face. The scene is
//! the record; the image model's prompt is compiled from it, never kept.
//!
//! A render that lands advances the record by the call's change
//! ([`Scene::apply`]). **A value equal to the record's is no change**, so a
//! model restating everyone costs nothing, and [`Delta`] says what did
//! change, which is what the planner routes on.
//!
//! Three files per chat that keeps scenes, all written by the harness and none
//! in the model's jail:
//! - **The chat's copy** beside its transcript.
//! - **The latest** (`scene/latest.json`), which a persona's new chats start
//!   from (R7). The last render to land wins.
//! - **The index** (`scene/index/<sha256>.json`): the scene each picture was
//!   rendered as, keyed by the picture's bytes, never through a workspace
//!   manifest, which a run can write.
//!
//! **Origin is per field, and a scene's origin is the union over its
//! fields.** A clean write over a scene with one untrusted field leaves that
//! field, and the scene, untrusted. Unknown is untrusted.

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

/// Everything in the picture except its people and its words: a place and
/// its objects, or a whole subject for a picture without people.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Setting {
    Words {
        text: String,
    },
    /// A photo used as the room. `path` is a label, local to the chat that
    /// set it; `hash` is the handle the planner checks it by.
    Photo {
        path: String,
        hash: String,
    },
    /// A setting this build cannot read. Never clean, and no setting to draw
    /// on (a closed enum in a store is a wire format).
    Unknown,
}

/// Where someone stands in the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Where {
    Left,
    Centre,
    Right,
    Background,
}

impl Where {
    pub fn parse(s: &str) -> Option<Where> {
        match s.trim().to_lowercase().as_str() {
            "left" => Some(Where::Left),
            "centre" | "center" | "middle" => Some(Where::Centre),
            "right" => Some(Where::Right),
            "background" | "behind" | "back" => Some(Where::Background),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Where::Left => "on the left",
            Where::Centre => "in the centre",
            Where::Right => "on the right",
            Where::Background => "in the background",
        }
    }
}

/// Who someone is: a library character by name, or someone described in
/// words, drawn from those words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Who {
    Library(String),
    Described(String),
}

impl Who {
    /// The key a person is matched by across changes: the library name, or
    /// the description, lowercased.
    pub fn key(&self) -> String {
        match self {
            Who::Library(n) | Who::Described(n) => n.trim().to_lowercase(),
        }
    }
}

/// Someone in the scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Person {
    pub who: Who,
    #[serde(default)]
    pub at: Option<Where>,
    #[serde(default)]
    pub wearing: String,
    #[serde(default)]
    pub doing: String,
    #[serde(default)]
    pub expression: String,
    #[serde(default)]
    pub origin: Origin,
}

/// Words the picture renders, exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Words {
    pub words: String,
    #[serde(default)]
    pub at: String,
    #[serde(default)]
    pub look: String,
}

/// The scene record.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Scene {
    /// Read leniently: a setting this build does not know is kept as
    /// `Unknown`, untrusted, and the rest of the record survives.
    #[serde(default, deserialize_with = "lenient_setting")]
    pub setting: Option<Field<Setting>>,
    #[serde(default)]
    pub light: Option<Field<String>>,
    #[serde(default)]
    pub camera: Option<Field<String>>,
    #[serde(default)]
    pub style: Option<Field<String>>,
    #[serde(default)]
    pub together: Option<Field<String>>,
    #[serde(default)]
    pub text: Option<Field<Vec<Words>>>,
    #[serde(default)]
    pub people: Vec<Person>,
    /// Whether the record knows who is in the picture. A picture with no
    /// record, changed by a call that declared nobody, does not: its people
    /// are unknown, never none (`IMAGE-DESIGN.md` §5.1, review N2). Absent
    /// reads unknown.
    #[serde(default)]
    pub people_known: bool,
    /// The content hash of the picture this scene was last rendered as.
    #[serde(default)]
    pub picture: Option<String>,
    /// The seed the picture was drawn at, so a restage can reuse it and a
    /// redraw can be sure to differ.
    #[serde(default)]
    pub seed: Option<u64>,
    /// The chat whose render last advanced it.
    #[serde(default)]
    pub chat: Option<String>,
}

/// What a call says about a scene: only the fields it sends.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SceneChange {
    pub setting: Option<Setting>,
    pub light: Option<String>,
    pub camera: Option<String>,
    pub style: Option<String>,
    pub together: Option<String>,
    pub text: Option<Vec<Words>>,
    pub people: Vec<PersonChange>,
}

/// What a call says about one person: only the fields it sends.
#[derive(Debug, Clone, PartialEq)]
pub struct PersonChange {
    pub who: Who,
    pub at: Option<Where>,
    pub wearing: Option<String>,
    pub doing: Option<String>,
    pub expression: Option<String>,
    pub remove: bool,
}

/// What a change actually changed against the record. Equal values are no
/// change; the planner routes on this.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Delta {
    pub setting: bool,
    pub light: bool,
    pub camera: bool,
    pub style: bool,
    /// `together` as the call stated it.
    pub together: bool,
    /// A recorded `together` the acts' change ended (review T2). It restages
    /// nothing by itself: adding someone to a scene with a relation is still
    /// an edit (review of #597).
    pub together_cleared: bool,
    pub text: bool,
    /// People new to the scene.
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// People whose pose (`doing`) or place in the frame (`where`) changed.
    pub posed: Vec<String>,
    /// People whose clothes changed.
    pub dressed: Vec<String>,
    /// People whose expression changed.
    pub expressed: Vec<String>,
}

impl Delta {
    /// Nothing changed: the call is a redraw (§5.1, review B4).
    pub fn is_empty(&self) -> bool {
        *self == Delta::default()
    }

    /// The change redraws everyone: a pose or a place in the frame, the
    /// camera, the setting, the light, `together` as the call states it, or
    /// someone removed. A restage.
    pub fn restages(&self) -> bool {
        self.setting
            || self.camera
            || self.light
            || self.together
            || !self.posed.is_empty()
            || !self.removed.is_empty()
    }
}

/// Two prose values are the same when they say the same thing, ignoring
/// case, surrounding space and a final full stop.
fn same(a: &str, b: &str) -> bool {
    let n = |s: &str| s.trim().trim_end_matches('.').trim().to_lowercase();
    n(a) == n(b)
}

impl Scene {
    /// The scene's origin: the union over every field it holds.
    pub fn origin(&self) -> Origin {
        let mut o = Origin::Clean;
        for f in [&self.light, &self.camera, &self.style, &self.together]
            .into_iter()
            .flatten()
        {
            o = o.union(f.origin);
        }
        if let Some(s) = &self.setting {
            o = o.union(s.origin);
        }
        if let Some(t) = &self.text {
            o = o.union(t.origin);
        }
        for p in &self.people {
            o = o.union(p.origin);
        }
        o
    }

    /// The scene a change leaves, and what it changed (§5.1).
    ///
    /// - A value equal to the record's is no change, and keeps its origin.
    /// - A changed field takes the run's origin `by`; a person changed in
    ///   part keeps their old origin joined with `by`, so one new field never
    ///   launders the others.
    /// - `together` does not outlive the acts it describes: any pose change,
    ///   removal or addition clears it unless the change restates it (§4,
    ///   review T2).
    /// - A person the change introduces needs `wearing` and `doing`; the
    ///   caller checks that before calling.
    /// - With no base, the change defines the scene. Its people are known if
    ///   it names any, or if `people_known` says the caller knows the picture
    ///   holds nobody (a new picture).
    pub fn apply(
        base: Option<&Scene>,
        change: &SceneChange,
        by: Origin,
        people_known: bool,
    ) -> (Scene, Delta) {
        let empty = Scene::default();
        let prev = base.unwrap_or(&empty);
        let mut d = Delta::default();
        let mut next = prev.clone();
        let set_text = |slot: &mut Option<Field<String>>, v: &Option<String>, flag: &mut bool| {
            if let Some(v) = v {
                let changed = slot.as_ref().is_none_or(|f| !same(&f.value, v));
                if changed {
                    *slot = (!v.trim().is_empty()).then(|| Field {
                        value: v.trim().to_string(),
                        origin: by,
                    });
                    *flag = true;
                }
            }
        };
        if let Some(s) = &change.setting {
            let changed = next.setting.as_ref().is_none_or(|f| f.value != *s);
            if changed {
                next.setting = Some(Field {
                    value: s.clone(),
                    origin: by,
                });
                d.setting = true;
            }
        }
        set_text(&mut next.light, &change.light, &mut d.light);
        set_text(&mut next.camera, &change.camera, &mut d.camera);
        set_text(&mut next.style, &change.style, &mut d.style);
        if let Some(t) = &change.text {
            if next.text.as_ref().is_none_or(|f| f.value != *t) {
                next.text = (!t.is_empty()).then(|| Field {
                    value: t.clone(),
                    origin: by,
                });
                d.text = true;
            }
        }
        for c in &change.people {
            let key = c.who.key();
            let at = next.people.iter().position(|p| p.who.key() == key);
            match at {
                Some(i) if c.remove => {
                    next.people.remove(i);
                    d.removed.push(key);
                }
                None if c.remove => {}
                Some(i) => {
                    let p = &mut next.people[i];
                    let mut touched = false;
                    if let Some(w) = &c.wearing {
                        if !same(&p.wearing, w) {
                            p.wearing = w.trim().to_string();
                            d.dressed.push(key.clone());
                            touched = true;
                        }
                    }
                    if let Some(x) = &c.doing {
                        if !same(&p.doing, x) {
                            p.doing = x.trim().to_string();
                            d.posed.push(key.clone());
                            touched = true;
                        }
                    }
                    if let Some(x) = &c.at {
                        if p.at != Some(*x) {
                            p.at = Some(*x);
                            if !d.posed.contains(&key) {
                                d.posed.push(key.clone());
                            }
                            touched = true;
                        }
                    }
                    if let Some(e) = &c.expression {
                        if !same(&p.expression, e) {
                            p.expression = e.trim().to_string();
                            d.expressed.push(key.clone());
                            touched = true;
                        }
                    }
                    if touched {
                        p.origin = p.origin.union(by);
                    }
                }
                None => {
                    next.people.push(Person {
                        who: c.who.clone(),
                        at: c.at,
                        wearing: c.wearing.clone().unwrap_or_default().trim().to_string(),
                        doing: c.doing.clone().unwrap_or_default().trim().to_string(),
                        expression: c.expression.clone().unwrap_or_default().trim().to_string(),
                        origin: by,
                    });
                    d.added.push(key);
                }
            }
        }
        // A relation does not outlive the acts it describes (review T2).
        let acts_changed = !d.posed.is_empty() || !d.added.is_empty() || !d.removed.is_empty();
        match &change.together {
            Some(t) if next.together.as_ref().is_none_or(|f| !same(&f.value, t)) => {
                next.together = (!t.trim().is_empty()).then(|| Field {
                    value: t.trim().to_string(),
                    origin: by,
                });
                d.together = true;
            }
            Some(_) => {}
            None if acts_changed && next.together.is_some() => {
                next.together = None;
                d.together_cleared = true;
            }
            None => {}
        }
        // Bounded here, where everything that reaches the store passes.
        let cap = |s: &mut String, n: usize| {
            if s.chars().count() > n {
                *s = s.chars().take(n).collect();
            }
        };
        next.people.truncate(MAX_PEOPLE);
        for p in &mut next.people {
            cap(&mut p.wearing, crate::imagelib::MAX_CAST_FIELD);
            cap(&mut p.doing, crate::imagelib::MAX_CAST_FIELD);
            cap(&mut p.expression, crate::imagelib::MAX_CAST_FIELD);
        }
        // Known once a call declares someone in the picture; a removal
        // declares nobody (review of #597).
        next.people_known = (base.is_some_and(|b| b.people_known))
            || people_known
            || change.people.iter().any(|c| !c.remove);
        (next, d)
    }
}

/// The most people a scene keeps: twice what one picture draws with faces.
pub const MAX_PEOPLE: usize = 2 * crate::imagelib::MAX_CAST;

/// A setting that does not parse is an unknown setting, never a lost record,
/// and untrusted whatever origin it was written with.
fn lenient_setting<'de, D>(d: D) -> std::result::Result<Option<Field<Setting>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(v.filter(|v| !v.is_null()).map(|v| {
        serde_json::from_value::<Field<Setting>>(v)
            .ok()
            .filter(|f| f.value != Setting::Unknown)
            .unwrap_or(Field {
                value: Setting::Unknown,
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

/// The most of a setting described in words a note carries.
const NOTE_PLACE_CHARS: usize = 240;

/// The most of any other field a note carries.
const NOTE_FIELD_CHARS: usize = 160;

/// The scene as a run note: the setting, who is there, what each wears, does
/// and shows, and the camera and light. The present comes from here, so a
/// recalled day's outfit or blocking cannot stand in for it. `me` is the
/// persona's own library character, said as "You". `None` when the scene
/// holds nothing to say.
pub fn note(scene: &Scene, me: Option<&str>) -> Option<String> {
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
    match scene.setting.as_ref().map(|p| &p.value) {
        Some(Setting::Words { text }) => {
            out.push_str(&format!(" Place: {}.", clip(text, NOTE_PLACE_CHARS)));
        }
        Some(Setting::Photo { .. }) => out.push_str(" Place: the photo you were placed in."),
        Some(Setting::Unknown) | None => {}
    }
    let me = me.map(|m| m.trim().to_lowercase());
    for p in &scene.people {
        let who = match &p.who {
            Who::Library(n) if me.as_deref() == Some(n.trim().to_lowercase().as_str()) => {
                "You".to_string()
            }
            Who::Library(n) => crate::imagegen::capitalized(n),
            Who::Described(d) => {
                let d = clip(d, NOTE_FIELD_CHARS);
                let mut c = d.chars();
                c.next()
                    .map(|f| f.to_uppercase().chain(c).collect::<String>())
                    .unwrap_or_default()
            }
        };
        let mut said = vec![];
        if let Some(at) = p.at {
            said.push(at.name().to_string());
        }
        if !p.wearing.trim().is_empty() {
            said.push(format!("wearing {}", clip(&p.wearing, NOTE_FIELD_CHARS)));
        }
        if !p.doing.trim().is_empty() {
            said.push(clip(&p.doing, NOTE_FIELD_CHARS));
        }
        if !p.expression.trim().is_empty() {
            said.push(clip(&p.expression, NOTE_FIELD_CHARS));
        }
        if said.is_empty() {
            let verb = if who == "You" { "are" } else { "is" };
            out.push_str(&format!(" {who} {verb} there."));
        } else {
            out.push_str(&format!(" {who}: {}.", said.join(", ")));
        }
    }
    if let Some(t) = &scene.together {
        out.push_str(&format!(" Together: {}.", clip(&t.value, NOTE_FIELD_CHARS)));
    }
    if let Some(c) = &scene.camera {
        out.push_str(&format!(" Camera: {}.", clip(&c.value, NOTE_FIELD_CHARS)));
    }
    if let Some(l) = &scene.light {
        out.push_str(&format!(" Light: {}.", clip(&l.value, NOTE_FIELD_CHARS)));
    }
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

/// Where one chat's scene lives, stamped on its runs by the front end, never
/// by a model.
#[derive(Debug, Clone)]
pub struct SceneSlot {
    /// The chat's own copy, beside its transcript.
    pub chat_copy: PathBuf,
    /// The persona's `scene/` folder: its latest scene and the index.
    pub store: PathBuf,
    /// The chat's id, recorded on the scenes it advances.
    pub chat: String,
    /// Whether a chat with no scene yet starts from the store's latest: a
    /// persona's chats do (R7, one persona's continuity), the assistant's
    /// and incognito's never, since their store is shared by unrelated
    /// conversations (review of #597).
    pub from_latest: bool,
}

impl SceneSlot {
    /// The chat's scene now: its own copy, else the persona's latest, which
    /// the chat starts from (R7), else none. A copy that exists but cannot
    /// be read is none, never the latest: that is another chat's scene, and
    /// a broken copy is not a chat with no scene yet (review of #589, pass 8).
    pub fn current(&self) -> Option<Scene> {
        if self.chat_copy.exists() {
            read(&self.chat_copy)
        } else if self.from_latest {
            read(&self.store.join("latest.json"))
        } else {
            None
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
    forget_in(
        &persona_dir.join("sessions"),
        &persona_dir.join("scene"),
        chat,
    )
}

/// [`forget_chat`] for one of the assistant's kept chats, whose slot is
/// [`assistant_slot`]: the copy in `sessions`, the store beside it.
pub fn forget_assistant_chat(sessions: &Path, chat: &str) -> std::io::Result<SceneForget> {
    forget_in(sessions, &assistant_store(sessions), chat)
}

/// The assistant's kept chats (IMAGE-DESIGN.md §6, one path): each chat's
/// copy beside its transcript, and the assistant's latest and index in
/// `scene/` beside the sessions folder — the persona's layout one level up,
/// outside every jail, written only by the harness.
pub fn assistant_slot(sessions: &Path, chat: &str) -> SceneSlot {
    SceneSlot {
        chat_copy: sessions.join(format!("{chat}.scene.json")),
        store: assistant_store(sessions),
        chat: chat.to_string(),
        from_latest: false,
    }
}

/// An incognito chat's (R9): all of it in the room, beside the jail and
/// never in it, so it goes when the room does and nothing reaches the mecha
/// home.
pub fn room_slot(room: &Path, chat: &str) -> SceneSlot {
    SceneSlot {
        chat_copy: room.join("scene").join(format!("{chat}.scene.json")),
        store: room.join("scene"),
        chat: chat.to_string(),
        from_latest: false,
    }
}

fn assistant_store(sessions: &Path) -> PathBuf {
    sessions.parent().unwrap_or(sessions).join("scene")
}

fn forget_in(copies: &Path, store: &Path, chat: &str) -> std::io::Result<SceneForget> {
    if !is_chat_id(chat) {
        return Err(std::io::Error::other(format!("invalid chat id {chat:?}")));
    }
    let mut gone = 0;
    let copy = copies.join(format!("{chat}.scene.json"));
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
                from_latest: true,
            },
            root,
        )
    }

    fn person(name: &str, wearing: &str, doing: &str) -> PersonChange {
        PersonChange {
            who: Who::Library(name.into()),
            at: None,
            wearing: Some(wearing.into()),
            doing: Some(doing.into()),
            expression: None,
            remove: false,
        }
    }

    /// A new scene: a setting in words and two people.
    fn kitchen(by: Origin) -> Scene {
        let (s, _) = Scene::apply(
            None,
            &SceneChange {
                setting: Some(Setting::Words {
                    text: "a narrow kitchen".into(),
                }),
                light: Some("late afternoon sun".into()),
                people: vec![
                    person("maya", "a red coat", "sitting at the table"),
                    person("john", "an apron", "cooking"),
                ],
                ..SceneChange::default()
            },
            by,
            true,
        );
        s
    }

    fn landed(mut s: Scene, picture: &str, chat: &str) -> Scene {
        s.picture = Some(hash(picture.as_bytes()));
        s.chat = Some(chat.into());
        s
    }

    /// A value equal to the record's is no change, and a call that restates
    /// everyone changes nothing: an empty delta is a redraw (§5.1, B4).
    #[test]
    fn equal_is_unchanged_and_restating_everyone_is_an_empty_delta() {
        let base = kitchen(Origin::Clean);
        let (_, d) = Scene::apply(
            Some(&base),
            &SceneChange {
                setting: Some(Setting::Words {
                    text: "a narrow kitchen".into(),
                }),
                light: Some("Late afternoon sun.".into()),
                people: vec![
                    person("Maya", "a red coat", "sitting at the table"),
                    person("john", "an apron", "cooking"),
                ],
                ..SceneChange::default()
            },
            Origin::Clean,
            false,
        );
        assert!(d.is_empty(), "{d:?}");
    }

    /// Each kind of change is said for what it is: a pose and a removal
    /// restage; clothes, an expression and someone added do not.
    #[test]
    fn a_delta_names_what_changed() {
        let base = kitchen(Origin::Clean);
        let change = |people: Vec<PersonChange>| {
            Scene::apply(
                Some(&base),
                &SceneChange {
                    people,
                    ..SceneChange::default()
                },
                Origin::Clean,
                false,
            )
            .1
        };
        let d = change(vec![PersonChange {
            wearing: None,
            ..person("maya", "", "standing by the window")
        }]);
        assert_eq!(d.posed, ["maya"]);
        assert!(d.restages());
        let d = change(vec![PersonChange {
            doing: None,
            ..person("maya", "a green dress", "")
        }]);
        assert_eq!(d.dressed, ["maya"]);
        assert!(!d.restages());
        let d = change(vec![PersonChange {
            wearing: None,
            doing: None,
            expression: Some("smiling".into()),
            ..person("maya", "", "")
        }]);
        assert_eq!(d.expressed, ["maya"]);
        assert!(
            !d.restages(),
            "an expression is a retouch, not a restage (S2)"
        );
        let d = change(vec![person("wren", "a scarf", "reading")]);
        assert_eq!(d.added, ["wren"]);
        assert!(!d.restages());
        let d = change(vec![PersonChange {
            remove: true,
            ..person("john", "", "")
        }]);
        assert_eq!(d.removed, ["john"]);
        assert!(d.restages());
    }

    /// A relation does not outlive the acts it describes: a pose change
    /// clears it unless restated (§4, review T2).
    #[test]
    fn together_is_cleared_when_the_acts_change() {
        let (base, _) = Scene::apply(
            Some(&kitchen(Origin::Clean)),
            &SceneChange {
                together: Some("John hugs Maya from behind".into()),
                ..SceneChange::default()
            },
            Origin::Clean,
            false,
        );
        assert!(base.together.is_some());
        let (next, d) = Scene::apply(
            Some(&base),
            &SceneChange {
                people: vec![PersonChange {
                    wearing: None,
                    ..person("john", "", "sitting down")
                }],
                ..SceneChange::default()
            },
            Origin::Clean,
            false,
        );
        assert!(next.together.is_none() && d.together_cleared && !d.together);
        let (kept, _) = Scene::apply(
            Some(&base),
            &SceneChange {
                light: Some("candlelight".into()),
                ..SceneChange::default()
            },
            Origin::Clean,
            false,
        );
        assert!(kept.together.is_some(), "a change to the light keeps it");
    }

    /// No record and nobody declared: the picture's people are unknown, not
    /// none (§5.1, review N2). A new picture knows them, even if none.
    #[test]
    fn people_are_unknown_until_a_call_declares_them() {
        let (s, _) = Scene::apply(
            None,
            &SceneChange {
                light: Some("dusk".into()),
                ..SceneChange::default()
            },
            Origin::Clean,
            false,
        );
        assert!(!s.people_known);
        let (s, _) = Scene::apply(Some(&s), &SceneChange::default(), Origin::Clean, false);
        assert!(!s.people_known);
        let (s, _) = Scene::apply(
            Some(&s),
            &SceneChange {
                people: vec![person("maya", "a coat", "reading")],
                ..SceneChange::default()
            },
            Origin::Clean,
            false,
        );
        assert!(s.people_known);
        let (s, _) = Scene::apply(None, &SceneChange::default(), Origin::Clean, true);
        assert!(s.people_known, "a new picture knows who it drew");
        let absent: Scene = serde_json::from_str(r#"{"people": []}"#).unwrap();
        assert!(!absent.people_known, "absent reads unknown");
    }

    /// Origin is per field: a clean change over an untrusted scene leaves
    /// what it did not touch untrusted, and a person changed in part keeps
    /// their old origin joined with the run's.
    #[test]
    fn origin_is_per_field_and_never_laundered() {
        let base = kitchen(Origin::Untrusted);
        let (next, _) = Scene::apply(
            Some(&base),
            &SceneChange {
                camera: Some("from above".into()),
                people: vec![PersonChange {
                    doing: None,
                    ..person("maya", "a green dress", "")
                }],
                ..SceneChange::default()
            },
            Origin::Clean,
            false,
        );
        assert_eq!(next.camera.as_ref().unwrap().origin, Origin::Clean);
        assert_eq!(next.people[0].origin, Origin::Untrusted, "changed in part");
        assert_eq!(next.setting.as_ref().unwrap().origin, Origin::Untrusted);
        assert_eq!(next.origin(), Origin::Untrusted);
        let (fresh, _) = Scene::apply(None, &SceneChange::default(), Origin::Clean, true);
        assert_eq!(fresh.origin(), Origin::Clean);
    }

    /// The store is bounded however long the chain of changes.
    #[test]
    fn a_scene_is_bounded_in_people_and_in_each_field() {
        let people = (0..20)
            .map(|i| person(&format!("p{i}"), &"silk ".repeat(200), "waving"))
            .collect();
        let (s, _) = Scene::apply(
            None,
            &SceneChange {
                people,
                ..SceneChange::default()
            },
            Origin::Clean,
            true,
        );
        assert_eq!(s.people.len(), MAX_PEOPLE);
        assert!(s
            .people
            .iter()
            .all(|p| p.wearing.chars().count() <= crate::imagelib::MAX_CAST_FIELD));
    }

    /// A slot whose chat id forgetting would refuse writes nothing.
    #[test]
    fn a_slot_with_a_bad_chat_id_lands_nothing() {
        let (good, root) = slot();
        let bad = SceneSlot {
            chat: "../c1".into(),
            from_latest: true,
            ..good
        };
        assert!(bad
            .land(&landed(kitchen(Origin::Clean), "a", "../c1"))
            .is_err());
        assert!(!root.join("persona/scene/latest.json").exists());
        assert!(!root.join("persona/scene/index").exists());
        std::fs::remove_dir_all(root).ok();
    }

    /// A chat whose own copy is broken has no scene; it does not start from
    /// another chat's latest.
    #[test]
    fn a_broken_chat_copy_is_no_scene_not_the_latest() {
        let (slot, root) = slot();
        let s = landed(kitchen(Origin::Clean), "a", "c1");
        slot.land(&s).unwrap();
        assert!(slot.current().is_some());
        std::fs::write(&slot.chat_copy, b"{ not json").unwrap();
        assert!(slot.current().is_none());
        std::fs::remove_file(&slot.chat_copy).unwrap();
        assert_eq!(slot.current(), Some(s), "no copy yet: the latest");
        std::fs::remove_dir_all(root).ok();
    }

    /// A setting this build cannot read is kept as unknown, untrusted, and
    /// the rest of the record survives; it says nothing in a note.
    #[test]
    fn an_unknown_setting_costs_the_setting_only() {
        for setting in [
            r#"{"value": {"kind": "hologram"}, "origin": "clean"}"#,
            r#"{"value": {"kind": "words"}, "origin": "clean"}"#,
            r#"{"value": "a beach"}"#,
        ] {
            let s: Scene = serde_json::from_str(&format!(
                r#"{{"setting": {setting}, "people": [{{"who": {{"kind": "library", "value": "maya"}}, "origin": "clean"}}], "chat": "c1"}}"#
            ))
            .unwrap();
            let f = s.setting.as_ref().expect("kept, as unknown");
            assert_eq!(f.value, Setting::Unknown, "{setting}");
            assert_eq!(s.origin(), Origin::Untrusted, "{setting}");
            assert_eq!(s.people[0].who, Who::Library("maya".into()));
        }
        let unknown: Scene =
            serde_json::from_str(r#"{"setting": {"value": {"kind": "hologram"}}}"#).unwrap();
        assert_eq!(note(&unknown, None), None);
    }

    /// Landing writes the chat's copy, the latest and the index; a chat with
    /// no copy starts from the latest (R7); a picture's bytes find its scene;
    /// the last render to land wins the latest.
    #[test]
    fn landing_writes_three_files_and_a_picture_finds_its_scene() {
        let (slot, root) = slot();
        let s = landed(kitchen(Origin::Clean), "picture one", "c1");
        slot.land(&s).unwrap();
        assert!(slot.chat_copy.is_file());
        assert_eq!(slot.lookup(b"picture one"), Some(s.clone()));
        assert_eq!(slot.lookup(b"another picture"), None);
        let other = SceneSlot {
            chat_copy: root.join("persona/sessions/c2.scene.json"),
            chat: "c2".into(),
            from_latest: true,
            ..slot.clone()
        };
        assert_eq!(
            other.current(),
            Some(s),
            "a new chat starts from the latest"
        );
        let (s2, _) = Scene::apply(
            other.current().as_ref(),
            &SceneChange {
                people: vec![PersonChange {
                    doing: None,
                    ..person("maya", "a hat", "")
                }],
                ..SceneChange::default()
            },
            Origin::Clean,
            false,
        );
        other.land(&landed(s2, "picture two", "c2")).unwrap();
        assert_eq!(slot.current().unwrap().people[0].wearing, "a red coat");
        let latest = read(&slot.store.join("latest.json")).unwrap();
        assert_eq!(latest.chat.as_deref(), Some("c2"), "the last to land wins");
        std::fs::remove_dir_all(root).ok();
    }

    /// The note says where she is, who is with her and what each wears, does
    /// and shows, the persona herself as "You", with the light and what they
    /// do together; its stem follows the scene's origin.
    #[test]
    fn the_note_reads_the_scene_and_arms_by_its_stem() {
        let (s, _) = Scene::apply(
            Some(&kitchen(Origin::Clean)),
            &SceneChange {
                together: Some("John hands Maya a cup".into()),
                people: vec![PersonChange {
                    at: Some(Where::Left),
                    wearing: None,
                    doing: None,
                    expression: Some("smiling".into()),
                    ..person("maya", "", "")
                }],
                ..SceneChange::default()
            },
            Origin::Clean,
            false,
        );
        let n = note(&s, Some("Maya")).unwrap();
        assert!(n.starts_with(SCENE_STEM), "{n}");
        assert!(n.contains("Place: a narrow kitchen."), "{n}");
        assert!(
            n.contains("You: on the left, wearing a red coat, sitting at the table, smiling."),
            "{n}"
        );
        assert!(n.contains("John: wearing an apron, cooking."), "{n}");
        assert!(n.contains("Together: John hands Maya a cup."), "{n}");
        assert!(n.contains("Light: late afternoon sun."), "{n}");
        let mut t = crate::agent::Taint::default();
        t.arm_for_notes(&[n]);
        assert!(t.private && !t.untrusted);
        let n = note(&kitchen(Origin::Untrusted), None).unwrap();
        assert!(n.starts_with(UNTRUSTED_SCENE_STEM), "{n}");
        assert_eq!(note(&Scene::default(), None), None);
    }

    /// Forgetting a chat removes what its renders left, and nothing another
    /// chat wrote; a record nobody can read is kept and counted.
    #[test]
    fn forgetting_a_chat_removes_its_scenes_only() {
        let (slot, root) = slot();
        slot.land(&landed(kitchen(Origin::Clean), "one", "c1"))
            .unwrap();
        let other = SceneSlot {
            chat_copy: root.join("persona/sessions/c2.scene.json"),
            chat: "c2".into(),
            from_latest: true,
            ..slot.clone()
        };
        other
            .land(&landed(kitchen(Origin::Clean), "two", "c2"))
            .unwrap();
        let persona = root.join("persona");
        std::fs::write(root.join("persona/scene/index/broken.json"), b"{ half").unwrap();
        assert_eq!(
            forget_chat(&persona, "c1").unwrap(),
            SceneForget {
                removed: 2,
                unreadable: 1
            }
        );
        std::fs::remove_file(root.join("persona/scene/index/broken.json")).unwrap();
        assert!(slot.lookup(b"one").is_none());
        assert!(slot.lookup(b"two").is_some());
        assert_eq!(slot.current().unwrap().chat.as_deref(), Some("c2"));
        assert_eq!(forget_chat(&persona, "c2").unwrap().removed, 3);
        assert_eq!(slot.current(), None);
        assert!(forget_chat(&persona, "../escape").is_err());
        std::fs::remove_dir_all(root).ok();
    }

    /// The assistant's and incognito's chats never start from the store's
    /// latest: it is shared by unrelated conversations (review of #597). A
    /// persona's do (R7).
    #[test]
    fn only_a_persona_chat_starts_from_the_latest() {
        let dir = std::env::temp_dir().join(format!("mecha-scene-{}", uuid::Uuid::new_v4()));
        let sessions = dir.join("sessions");
        let first = assistant_slot(&sessions, "chat-a");
        let s = Scene {
            picture: Some(hash(b"x")),
            ..Scene::default()
        };
        first.land(&s).unwrap();
        assert!(first.current().is_some());
        assert!(assistant_slot(&sessions, "chat-b").current().is_none());
        let persona = SceneSlot {
            from_latest: true,
            ..assistant_slot(&sessions, "chat-c")
        };
        assert!(persona.current().is_some());
        std::fs::remove_dir_all(dir).ok();
    }
}
