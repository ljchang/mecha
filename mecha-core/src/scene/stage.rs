//! One persona chat's scene as it stood when an owner turn was sent, staged
//! into a scratch store for a replay that renders.
//!
//! The scene store keeps only the present: each chat's copy, the persona's
//! latest, and an index entry per picture, all overwritten as renders land,
//! none timestamped. The past is recovered from the transcript instead: every
//! picture that landed in the chat before the turn is named on an `image: `
//! line of a delivered result, its bytes are in the chat's workspace, and
//! their hash keys the index entry that render wrote, which is the chat's
//! scene as of that render. So the chat's last landed picture before the
//! turn gives the scene exactly. A chat with no picture of its own yet read
//! the persona's latest (R7), and that is recovered only approximately: the
//! index entry last written at or before the turn's run began.
//!
//! Everything is copied; nothing in the real store or workspace is written.
//! The scratch folder must be new or empty and lie outside both.

use super::{is_chat_id, is_hash, read, SceneSlot, Setting};
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::path::{Component, Path, PathBuf};

/// Where a staged chat lives in the scratch folder, and what it was staged
/// from.
#[derive(Debug, Clone)]
pub struct Staged {
    /// The scratch personas root, laid out as the real one:
    /// `<persona>/scene/` and `<persona>/sessions/`.
    pub store: PathBuf,
    /// The chat's scene slot inside it, as serve stamps one.
    pub slot: SceneSlot,
    /// Where the chat's prompt log would go, beside the slot's copy; not
    /// created.
    pub prompt_log: PathBuf,
    /// The chat's workspace as of the turn: every picture landed before it,
    /// with its manifest, and the scene's room photo.
    pub workspace: PathBuf,
    /// What the scene was recovered from.
    pub as_of: AsOf,
    /// Files the transcript or the scene names that could not be staged:
    /// gone from the workspace, or not the bytes the record holds.
    pub missing: Vec<String>,
}

/// What the staged scene was recovered from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsOf {
    /// The chat's own last landed picture before the turn: exact.
    ChatPicture { picture: String },
    /// No picture of the chat's own yet, so the persona's latest: the index
    /// entry last written at or before `at`, the turn's run start.
    /// Approximate: write times, not a record.
    PersonaLatest { at: String, picture: String },
    /// No picture of its own and none in the persona's index by then: the
    /// chat started with no scene.
    Nothing,
}

/// Stage persona `persona`'s chat `chat` as of the owner message on line
/// `line` (0-based) of its transcript, into `scratch`.
///
/// `store` is the personas root (`~/.mecha/personas`). Refused, before
/// anything is written: a name or id that is not one path segment, a line
/// that is not an owner message, a scratch folder that is not empty or lies
/// inside the store or the workspace, and a chat whose last picture before
/// the turn is gone, since its scene then cannot be told.
pub fn stage_at(
    store: &Path,
    persona: &str,
    chat: &str,
    line: usize,
    scratch: &Path,
) -> Result<Staged> {
    if !is_chat_id(chat) || !is_chat_id(persona) {
        bail!("`{persona}`/`{chat}` is not a persona and a chat id");
    }
    let real = store.join(persona);
    let transcript = real.join("sessions").join(format!("{chat}.jsonl"));
    let text = std::fs::read_to_string(&transcript)
        .with_context(|| format!("reading {}", transcript.display()))?;
    // Non-blank lines, as `Session::read` counts records (a torn write can
    // leave a blank one) and as `--at` names them: two conventions meeting
    // here staged a turn's scene from an earlier one (review of #615).
    let records: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or(Value::Null))
        .collect();
    let Some(turn) = records.get(line) else {
        bail!(
            "line {line} is past the transcript's {} lines",
            records.len()
        );
    };
    let results = turn["content"]
        .as_array()
        .is_some_and(|c| c.iter().any(|b| b["type"] == "tool_result"));
    if !(turn["record"] == "message" && turn["role"] == "user") || results {
        bail!("line {line} is not an owner message");
    }
    let before = &records[..line];
    let workspace = before
        .iter()
        .find(|r| r["record"] == "meta")
        .and_then(|r| r["workspace"].as_str())
        .map(PathBuf::from)
        .context("the transcript names no workspace")?;
    // The run the turn starts: its config record carries the clock.
    let at = before
        .iter()
        .rev()
        .find(|r| r["record"] == "config")
        .and_then(|r| r["clock"].as_str())
        .or_else(|| before.iter().find_map(|r| r["created_at"].as_str()))
        .map(str::to_string);

    // The chat's last picture before the turn decides its scene, so it is
    // read, and its index entry found, before anything is written.
    let real_index = real.join("scene").join("index");
    let landed = landed_before(before);
    let last = match landed.last() {
        Some(path) => {
            let entry = relative(path)
                .then(|| std::fs::read(workspace.join(path)).ok())
                .flatten()
                .map(|bytes| super::hash(&bytes))
                .map(|h| (real_index.join(format!("{h}.json")), h))
                .filter(|(entry, _)| entry.exists());
            let Some((entry, h)) = entry else {
                bail!(
                    "the chat's last picture before line {line}, {path}, is gone from its \
                     workspace or its index, so its scene then cannot be told"
                );
            };
            Some((h, entry))
        }
        None => None,
    };

    refuse_overlap(scratch, store)?;
    refuse_overlap(scratch, &workspace)?;
    if std::fs::read_dir(scratch).is_ok_and(|mut d| d.next().is_some()) {
        bail!("{} is not empty", scratch.display());
    }

    let out_store = scratch.join("personas");
    let out_persona = out_store.join(persona);
    let out_work = scratch.join("work");
    let slot = super::persona_slot(&out_store, persona, chat);
    std::fs::create_dir_all(slot.store.join("index"))?;
    std::fs::create_dir_all(out_persona.join("sessions"))?;
    std::fs::create_dir_all(&out_work)?;

    let mut missing = Vec::new();
    for path in &landed {
        match copy_into(&workspace, &out_work, path)? {
            Some(bytes) => {
                let h = super::hash(&bytes);
                let manifest = Path::new(path).with_extension("json");
                copy_into(&workspace, &out_work, &manifest.to_string_lossy())?;
                let entry = real_index.join(format!("{h}.json"));
                if entry.exists() {
                    std::fs::copy(&entry, slot.store.join("index").join(format!("{h}.json")))?;
                } else {
                    // Staged without its record, a call that names it draws
                    // with none of its people or clothes: said, not dropped.
                    missing.push(format!("{path} (its scene record)"));
                }
            }
            None => missing.push(path.clone()),
        }
    }

    let as_of = match last {
        Some((h, entry)) => {
            std::fs::copy(&entry, &slot.chat_copy)?;
            std::fs::copy(&entry, slot.store.join("latest.json"))?;
            AsOf::ChatPicture { picture: h }
        }
        None => match at.as_deref().and_then(|a| latest_by(&real_index, a)) {
            Some((h, entry)) => {
                let at = at.clone().unwrap_or_default();
                std::fs::copy(&entry, slot.store.join("latest.json"))?;
                std::fs::copy(&entry, slot.store.join("index").join(format!("{h}.json")))?;
                AsOf::PersonaLatest { at, picture: h }
            }
            None => AsOf::Nothing,
        },
    };

    // The room photo the scene stands in, checked by its hash as the
    // renderer checks it: a different file under that name is not staged.
    if let Some(Setting::Photo { path, hash }) =
        slot.current().and_then(|s| s.setting).map(|f| f.value)
    {
        match copy_into(&workspace, &out_work, &path)? {
            Some(bytes) if super::hash(&bytes) == hash => {}
            Some(_) => {
                std::fs::remove_file(out_work.join(&path))?;
                missing.push(path);
            }
            None => missing.push(path),
        }
    }

    Ok(Staged {
        prompt_log: out_persona
            .join("sessions")
            .join(format!("{chat}.prompts.log")),
        store: out_store,
        slot,
        workspace: out_work,
        as_of,
        missing,
    })
}

/// The pictures delivered into the chat before the turn, in order: each
/// `image: ` line of a delivered tool result, a late result, or a block the
/// harness appended (the edit panel's). An owner's words and the model's
/// are never read for it, and a compaction's rewrite repeats what was
/// already delivered.
fn landed_before(records: &[Value]) -> Vec<String> {
    // `image: ` also opens `image_view`'s answer: only a picture call's
    // results name a picture that landed (review of #615).
    let drawn: std::collections::BTreeSet<&str> = records
        .iter()
        .filter(|r| r["record"] == "message" && r["role"] == "assistant")
        .flat_map(|r| r["content"].as_array().into_iter().flatten())
        .filter(|b| b["type"] == "tool_use" && b["name"] == "image_generate")
        .filter_map(|b| b["id"].as_str())
        .collect();
    let mut texts: Vec<&str> = Vec::new();
    for r in records {
        match r["record"].as_str() {
            Some("late_result") => texts.extend(r["content"].as_str()),
            Some("extend") => {
                for b in r["blocks"].as_array().into_iter().flatten() {
                    texts.extend(b["text"].as_str());
                    texts.extend(b["content"].as_str());
                }
            }
            Some("message") if r["role"] == "user" => {
                for b in r["content"].as_array().into_iter().flatten() {
                    if b["type"] == "tool_result"
                        && b["tool_use_id"]
                            .as_str()
                            .is_some_and(|id| drawn.contains(id))
                    {
                        texts.extend(b["content"].as_str());
                    }
                }
            }
            _ => {}
        }
    }
    let mut out: Vec<String> = Vec::new();
    for line in texts.iter().flat_map(|t| t.lines()) {
        if let Some(p) = line.trim().strip_prefix("image: ") {
            if relative(p) && !out.iter().any(|o| o == p) {
                out.push(p.to_string());
            }
        }
    }
    out
}

/// The index entry written last at or before `at`, by its file's write time.
fn latest_by(index: &Path, at: &str) -> Option<(String, PathBuf)> {
    let at: std::time::SystemTime = chrono::DateTime::parse_from_rfc3339(at).ok()?.into();
    std::fs::read_dir(index)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let h = path.file_stem()?.to_str()?.to_string();
            let written = e.metadata().ok()?.modified().ok()?;
            (is_hash(&h) && written <= at && read(&path).is_some()).then_some((written, h, path))
        })
        .max_by_key(|(written, _, _)| *written)
        .map(|(_, h, path)| (h, path))
}

/// Copy `rel` from one folder to the same place under another, its bytes
/// returned; `None` when it is not there.
fn copy_into(from: &Path, to: &Path, rel: &str) -> Result<Option<Vec<u8>>> {
    if !relative(rel) {
        return Ok(None);
    }
    let bytes = match std::fs::read(from.join(rel)) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {rel}")),
    };
    let dest = to.join(rel);
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&dest, &bytes)?;
    Ok(Some(bytes))
}

/// A path that stays inside the folder it is joined to.
fn relative(p: &str) -> bool {
    !p.is_empty()
        && Path::new(p)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

/// Refuse a scratch folder inside `real`, or `real` inside it: a stage must
/// never write where the chat lives.
fn refuse_overlap(scratch: &Path, real: &Path) -> Result<()> {
    let canon = |p: &Path| {
        // The scratch folder may not exist yet: its nearest existing
        // ancestor stands in, which still lies inside `real` if it does.
        p.ancestors()
            .find_map(|a| std::fs::canonicalize(a).ok().map(|c| (c, a)))
            .map(|(c, a)| c.join(p.strip_prefix(a).unwrap_or(Path::new(""))))
    };
    let (Some(s), Some(r)) = (canon(scratch), canon(real)) else {
        bail!("cannot resolve {} or {}", scratch.display(), real.display());
    };
    if s.starts_with(&r) || r.starts_with(&s) {
        bail!(
            "the scratch folder {} overlaps {}, where the chat lives",
            scratch.display(),
            real.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{Field, Origin, Scene};
    use super::*;
    use serde_json::json;

    const CHAT: &str = "20260101T090000-0000abcd";

    struct World {
        root: PathBuf,
        store: PathBuf,
        work: PathBuf,
    }

    impl World {
        fn new(tag: &str) -> World {
            let root = std::env::temp_dir().join(format!(
                "mecha-stage-{tag}-{}",
                uuid::Uuid::new_v4().simple()
            ));
            let store = root.join("personas");
            let work = root.join("work").join("p-1");
            std::fs::create_dir_all(store.join("wren").join("sessions")).unwrap();
            std::fs::create_dir_all(store.join("wren").join("scene").join("index")).unwrap();
            std::fs::create_dir_all(work.join("images")).unwrap();
            World { root, store, work }
        }

        fn transcript(&self, lines: &[Value]) {
            let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
            std::fs::write(
                self.store
                    .join("wren")
                    .join("sessions")
                    .join(format!("{CHAT}.jsonl")),
                text,
            )
            .unwrap();
        }

        fn picture(&self, name: &str, bytes: &[u8]) -> String {
            std::fs::write(self.work.join("images").join(name), bytes).unwrap();
            super::super::hash(bytes)
        }

        fn index(&self, h: &str, scene: &Scene) -> PathBuf {
            let path = self
                .store
                .join("wren")
                .join("scene")
                .join("index")
                .join(format!("{h}.json"));
            std::fs::write(&path, serde_json::to_vec(scene).unwrap()).unwrap();
            path
        }

        /// Every file under the real store and workspace, with its bytes.
        fn snapshot(&self) -> Vec<(PathBuf, Vec<u8>)> {
            fn walk(p: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
                for e in std::fs::read_dir(p).unwrap().flatten() {
                    let path = e.path();
                    if path.is_dir() {
                        walk(&path, out);
                    } else {
                        out.push((path.clone(), std::fs::read(&path).unwrap()));
                    }
                }
            }
            let mut out = Vec::new();
            walk(&self.store, &mut out);
            walk(&self.work, &mut out);
            out.sort();
            out
        }
    }

    impl Drop for World {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).ok();
        }
    }

    fn header(w: &World, clock: &str) -> Vec<Value> {
        vec![
            json!({"record": "meta", "id": CHAT, "workspace": w.work, "created_at": clock}),
            json!({"record": "config", "clock": clock}),
        ]
    }

    fn owner(text: &str) -> Value {
        json!({"record": "message", "role": "user", "content": [{"type": "text", "text": text}]})
    }

    fn scene(chat: &str, picture: &str) -> Scene {
        Scene {
            together: Some(Field {
                value: "two people share an umbrella".into(),
                origin: Origin::Clean,
            }),
            people_known: true,
            picture: Some(picture.into()),
            chat: Some(chat.into()),
            ..Default::default()
        }
    }

    /// The scene as of a turn is the index entry of the chat's last picture
    /// before it, with the record's room photo staged by its hash, even when
    /// a later render in another chat has moved the persona's latest on. The
    /// real store and workspace are byte-for-byte as they were.
    #[test]
    fn a_photo_placed_scene_is_staged_as_of_the_turn() {
        let w = World::new("photo");
        let room = b"a room photo, not really a jpeg";
        std::fs::create_dir_all(w.work.join("uploads")).unwrap();
        std::fs::write(w.work.join("uploads").join("room.jpg"), room).unwrap();
        let first = w.picture("1-1.png", b"first picture");
        std::fs::write(w.work.join("images").join("1-1.json"), b"{}").unwrap();
        let mut placed = scene(CHAT, &first);
        placed.setting = Some(Field {
            value: Setting::Photo {
                path: "uploads/room.jpg".into(),
                hash: super::super::hash(room),
            },
            origin: Origin::Clean,
        });
        w.index(&first, &placed);
        // A picture this chat drew after the turn, and the latest it left:
        // neither is the scene as of the turn.
        let later = w.picture("2-2.png", b"a later picture");
        w.index(&later, &scene(CHAT, &later));
        std::fs::write(
            w.store.join("wren").join("scene").join("latest.json"),
            serde_json::to_vec(&scene("20260101T100000-other", &later)).unwrap(),
        )
        .unwrap();
        let mut lines = header(&w, "2026-01-01T09:00:00Z");
        lines.extend([
            owner("draw us"),
            json!({"record": "message", "role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": "being made: images/1-1.png"}]}),
            json!({"record": "late_result", "index": 3, "tool_use_id": "t1",
                   "content": "image: images/1-1.png\nA new picture."}),
            owner("now in the rain"),
            json!({"record": "late_result", "index": 6, "tool_use_id": "t2",
                   "content": "image: images/2-2.png\nA new picture."}),
        ]);
        w.transcript(&lines);
        let before = w.snapshot();

        let scratch = w.root.join("scratch");
        let staged = stage_at(&w.store, "wren", CHAT, 5, &scratch).unwrap();

        assert_eq!(
            staged.as_of,
            AsOf::ChatPicture {
                picture: first.clone()
            }
        );
        assert_eq!(staged.slot.current(), Some(placed.clone()));
        assert_eq!(
            std::fs::read(staged.workspace.join("uploads").join("room.jpg")).unwrap(),
            room
        );
        assert!(staged.workspace.join("images").join("1-1.png").exists());
        assert!(staged.workspace.join("images").join("1-1.json").exists());
        assert!(!staged.workspace.join("images").join("2-2.png").exists());
        assert_eq!(staged.slot.lookup(b"first picture"), Some(placed));
        assert!(staged.missing.is_empty(), "{:?}", staged.missing);
        assert!(staged.prompt_log.starts_with(&scratch) && !staged.prompt_log.exists());
        assert_eq!(
            w.snapshot(),
            before,
            "the real store and workspace were written"
        );
    }

    /// Review of #615: a blank line (a torn write) before the turn does not
    /// shift which record is the turn; `image_view`'s `image: ` answer is not
    /// a landed picture; and a landed picture whose scene record is gone is
    /// said to be missing rather than staged without it.
    #[test]
    fn the_turn_and_its_pictures_are_read_as_the_transcript_counts_them() {
        let w = World::new("count");
        let earlier = w.picture("0-0.png", b"an earlier picture, its record gone");
        let last = w.picture("1-1.png", b"the last picture");
        w.picture("viewed.png", b"a file the persona looked at");
        w.index(&last, &scene(CHAT, &last));
        let mut lines = header(&w, "2026-01-01T09:00:00Z");
        lines.extend([
            owner("draw us"),
            json!({"record": "late_result", "index": 3, "tool_use_id": "t0",
                   "content": "image: images/0-0.png\nA new picture."}),
            json!({"record": "message", "role": "assistant", "content": [
                {"type": "tool_use", "id": "t1", "name": "image_generate", "input": {}}]}),
            json!({"record": "message", "role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t1",
                 "content": "image: images/1-1.png\nA new picture."}]}),
            json!({"record": "message", "role": "assistant", "content": [
                {"type": "tool_use", "id": "t2", "name": "image_view", "input": {}}]}),
            json!({"record": "message", "role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t2",
                 "content": "image: images/viewed.png"}]}),
            owner("now in the rain"),
        ]);
        let mut text: String = lines[..lines.len() - 1]
            .iter()
            .map(|l| format!("{l}\n"))
            .collect();
        text.push('\n');
        text.push_str(&format!("{}\n", lines[lines.len() - 1]));
        std::fs::write(
            w.store
                .join("wren")
                .join("sessions")
                .join(format!("{CHAT}.jsonl")),
            text,
        )
        .unwrap();

        let staged = stage_at(&w.store, "wren", CHAT, 8, &w.root.join("scratch")).unwrap();
        assert_eq!(
            staged.as_of,
            AsOf::ChatPicture {
                picture: last.clone()
            }
        );
        assert!(!staged.workspace.join("images").join("viewed.png").exists());
        assert_eq!(staged.missing, ["images/0-0.png (its scene record)"]);
        assert_ne!(earlier, last);
        // A tool-result batch is not an owner message.
        let err = stage_at(&w.store, "wren", CHAT, 5, &w.root.join("scratch2")).unwrap_err();
        assert!(
            format!("{err:#}").contains("not an owner message"),
            "{err:#}"
        );
    }

    /// A room photo whose bytes are not the record's is not staged, and is
    /// said to be missing, as the renderer would refuse it.
    #[test]
    fn a_changed_room_photo_is_missing_not_staged() {
        let w = World::new("changed");
        std::fs::create_dir_all(w.work.join("uploads")).unwrap();
        std::fs::write(w.work.join("uploads").join("room.jpg"), b"another room").unwrap();
        let first = w.picture("1-1.png", b"first picture");
        let mut placed = scene(CHAT, &first);
        placed.setting = Some(Field {
            value: Setting::Photo {
                path: "uploads/room.jpg".into(),
                hash: super::super::hash(b"the room it was"),
            },
            origin: Origin::Clean,
        });
        w.index(&first, &placed);
        let mut lines = header(&w, "2026-01-01T09:00:00Z");
        lines.extend([
            owner("draw us"),
            json!({"record": "late_result", "index": 3, "tool_use_id": "t1",
                   "content": "image: images/1-1.png\nA new picture."}),
            owner("again"),
        ]);
        w.transcript(&lines);
        let staged = stage_at(&w.store, "wren", CHAT, 4, &w.root.join("scratch")).unwrap();
        assert_eq!(staged.missing, vec!["uploads/room.jpg".to_string()]);
        assert!(!staged.workspace.join("uploads").join("room.jpg").exists());
    }

    /// A chat with no picture of its own yet reads the persona's latest as of
    /// its run: the index entry written last at or before the clock.
    #[test]
    fn a_first_turn_reads_the_persona_latest_by_write_time() {
        let w = World::new("latest");
        let old = w.picture("0-0.png", b"older");
        let new = w.picture("0-1.png", b"newer");
        let at = |s: &str| -> std::time::SystemTime {
            chrono::DateTime::parse_from_rfc3339(s).unwrap().into()
        };
        let set = |p: &Path, t: &str| {
            std::fs::File::options()
                .write(true)
                .open(p)
                .unwrap()
                .set_modified(at(t))
                .unwrap()
        };
        set(
            &w.index(&old, &scene("20260101T080000-aa", &old)),
            "2026-01-01T08:00:00Z",
        );
        set(
            &w.index(&new, &scene("20260101T100000-bb", &new)),
            "2026-01-01T10:00:00Z",
        );
        let mut lines = header(&w, "2026-01-01T09:00:00Z");
        lines.push(owner("hello"));
        w.transcript(&lines);
        let staged = stage_at(&w.store, "wren", CHAT, 2, &w.root.join("scratch")).unwrap();
        assert_eq!(
            staged.as_of,
            AsOf::PersonaLatest {
                at: "2026-01-01T09:00:00Z".into(),
                picture: old.clone()
            }
        );
        assert_eq!(staged.slot.current().and_then(|s| s.picture), Some(old));
        assert!(!staged.slot.chat_copy.exists());
    }

    /// The refusals: a last picture gone from the workspace, a line that is
    /// not the owner's, a scratch folder inside the store or not empty.
    #[test]
    fn what_cannot_be_staged_is_refused_before_anything_is_written() {
        let w = World::new("refuse");
        let mut lines = header(&w, "2026-01-01T09:00:00Z");
        lines.extend([
            owner("draw us"),
            json!({"record": "late_result", "index": 3, "tool_use_id": "t1",
                   "content": "image: images/gone.png\nA new picture."}),
            owner("again"),
        ]);
        w.transcript(&lines);
        let err = |line: usize, scratch: &Path| {
            stage_at(&w.store, "wren", CHAT, line, scratch)
                .unwrap_err()
                .to_string()
        };
        assert!(err(4, &w.root.join("s1")).contains("cannot be told"));
        assert!(
            !w.root.join("s1").exists(),
            "a refused stage wrote its scratch folder"
        );
        assert!(err(3, &w.root.join("s2")).contains("not an owner message"));
        assert!(err(2, &w.store.join("wren").join("tmp")).contains("overlaps"));
        assert!(err(2, &w.work.join("tmp")).contains("overlaps"));
        let full = w.root.join("full");
        std::fs::create_dir_all(&full).unwrap();
        std::fs::write(full.join("x"), b"x").unwrap();
        assert!(err(2, &full).contains("not empty"));
        assert!(stage_at(&w.store, "../wren", CHAT, 4, &w.root.join("s3")).is_err());
    }
}
