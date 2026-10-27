//! Settings → LoRAs: the owner's LoRA registry (`mecha_core::loras`), read
//! and edited from the page (`docs/LORA-DESIGN.md` §6).
//!
//! One set of routes per family, each under its own feature
//! (`/api/loras/image` under `image`, `/api/loras/video` under `video`),
//! because a route belongs to exactly one feature and the pane shows each
//! section only while its feature is on.
//!
//! **Every route needs a live library unlock (L3)**, reads included: the
//! registry lists explicit descriptions, and the lock exists so a glance at
//! the screen does not show them. A locked read answers `{"locked": true}`
//! with no names, abouts or triggers, so hiding the pane is not left to the
//! page. What a model is offered never depends on the lock.
//!
//! Writes go through the core's parser (`loras::write_entry`), so the page
//! cannot write an entry the file would refuse, and replace the file
//! atomically. While the old clips-only file is still the registry, the
//! first write creates `~/.mecha/loras.toml` only once the page has said
//! that the old file then stops being read (`create_new`).

use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mecha_core::loras::{self, Check, EntryInput, Family, Folders, Loaded, Source, Status};
use serde::Deserialize;
use serde_json::{json, Value};

use super::library::no_store;

type St = State<super::WebState>;

#[derive(Debug, Deserialize)]
pub struct UnlockQuery {
    #[serde(default)]
    unlock: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WriteBody {
    #[serde(default)]
    unlock: Option<String>,
    entry: EntryInput,
    /// The page said the old file stops being read once this creates the
    /// new one.
    #[serde(default)]
    create_new: bool,
}

/// What a read of the registry needs: the home, and `[image]`/`[video]` as
/// the global config holds them now.
struct Here {
    home: std::path::PathBuf,
    image: Option<mecha_core::imagegen::ImageConfig>,
    video: Option<mecha_core::videogen::VideoConfig>,
}

impl Here {
    fn read() -> anyhow::Result<Here> {
        let cfg = mecha_core::config::Config::load_global()?;
        Ok(Here {
            home: mecha_core::work::mecha_home()?,
            image: cfg.image,
            video: cfg.video,
        })
    }

    fn legacy(&self) -> Option<&std::path::Path> {
        self.video.as_ref().and_then(|v| v.loras.as_deref())
    }

    fn folders(&self) -> Folders {
        Folders::of(&self.home, self.image.as_ref(), self.video.as_ref())
    }

    fn loaded(&self) -> Loaded {
        Loaded::read(&self.home, self.legacy(), &self.folders())
    }
}

fn locked() -> Response {
    no_store(Json(json!({ "locked": true })).into_response())
}

fn failed(e: impl std::fmt::Display) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}\n")).into_response()
}

fn status_word(st: &Status) -> &'static str {
    match st {
        Status::Checked => "checked",
        Status::NoFolder => "no_folder",
        Status::Missing(_) => "missing",
        Status::Unreadable(..) => "unreadable",
        Status::Mismatch(..) => "mismatch",
    }
}

fn check_word(c: &Check) -> Option<&'static str> {
    match c {
        Check::Family(f) => Some(f.as_str()),
        _ => None,
    }
}

/// Why a file's family could not be told, in the page's words.
fn check_note(c: &Check) -> Option<String> {
    match c {
        Check::Family(_) => None,
        Check::Neither => Some("its keys match neither model".into()),
        Check::Both => Some("its keys match both models".into()),
        Check::Unreadable(why) => Some(format!("it cannot be read ({why})")),
    }
}

/// The server's LoRA files, as its own Add list names them
/// (`LoraLoaderModelOnly`'s `lora_name`), or why it did not answer.
async fn listed(url: anyhow::Result<reqwest::Url>) -> Result<Vec<String>, String> {
    let url = url.map_err(|e| format!("{e:#}"))?;
    let url = url
        .join("object_info/LoraLoaderModelOnly")
        .map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .map_err(|e| e.to_string())?;
    let body: Value = client
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|_| "the server did not answer, so its files are not listed".to_string())?
        .json()
        .await
        .map_err(|_| "the server's answer did not read".to_string())?;
    let names = body["LoraLoaderModelOnly"]["input"]["required"]["lora_name"][0]
        .as_array()
        .ok_or_else(|| "the server lists no LoRA loader".to_string())?;
    Ok(names
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect())
}

async fn list(state: St, q: UnlockQuery, family: Family) -> Response {
    if !state.library.unlocked(q.unlock.as_deref()) {
        return locked();
    }
    let here = match tokio::task::spawn_blocking(Here::read).await {
        Ok(Ok(h)) => h,
        Ok(Err(e)) => return failed(format!("{e:#}")),
        Err(e) => return failed(e),
    };
    let url = match family {
        Family::QwenImage => here
            .image
            .as_ref()
            .map(|i| mecha_core::imagegen::loopback_url(&i.url)),
        Family::H3 => here.video.as_ref().map(|v| v.loopback_url()),
    };
    let files = match url {
        Some(url) => listed(url).await,
        None => Err("its server is not configured".to_string()),
    };
    let body = match tokio::task::spawn_blocking(move || describe(&here, family, files)).await {
        Ok(b) => b,
        Err(e) => return failed(e),
    };
    no_store(Json(body).into_response())
}

/// The pane's view of one family: the registry's source, each entry with
/// where it stands, and the server's files no entry uses yet.
fn describe(here: &Here, family: Family, files: Result<Vec<String>, String>) -> Value {
    let loaded = here.loaded();
    let folders = here.folders();
    let dirs = folders.get(family);
    let (source, path) = match &loaded.source {
        Source::Shared(p) => ("shared", Some(p.clone())),
        Source::Legacy(p) => ("legacy", Some(p.clone())),
        Source::None => ("none", None),
    };
    let entries: Vec<Value> = loaded
        .entries
        .iter()
        .filter(|(e, _)| e.model == family)
        .map(|(e, st)| {
            json!({
                "name": e.name,
                "about": e.about,
                "file": e.file,
                "frames": e.frames,
                "references": e.references,
                "strength": e.strength,
                "trigger": e.trigger,
                "always": e.always,
                "status": status_word(st),
                "reason": st.reason(e),
            })
        })
        .collect();
    // The video graph's own turbo LoRAs are config keys, not entries: never
    // offered as something to add.
    let turbo: Vec<&str> = here
        .video
        .as_ref()
        .filter(|_| family == Family::H3)
        .map(|v| {
            vec![
                v.frames_turbo_lora.as_str(),
                v.references_turbo_lora.as_str(),
            ]
        })
        .unwrap_or_default();
    let (unused, unused_error) = match files {
        Ok(files) => (
            loras::unused(&files, &loaded.entries, &turbo, dirs, family.dir_key())
                .into_iter()
                .map(|u| match u.check {
                    Ok(c) => {
                        json!({"file": u.file, "family": check_word(&c), "note": check_note(&c)})
                    }
                    Err(why) => json!({"file": u.file, "family": null, "note": why}),
                })
                .collect::<Vec<_>>(),
            None,
        ),
        Err(why) => (Vec::new(), Some(why)),
    };
    json!({
        "locked": false,
        "family": family.as_str(),
        "source": source,
        "path": path,
        "new_path": here.home.join(loras::FILE),
        "ignored": loaded.ignored,
        "unreadable": loaded.unreadable,
        "skipped": loaded.skipped,
        "dir_key": family.dir_key(),
        "folders": dirs,
        "entries": entries,
        "unused": unused,
        "unused_error": unused_error,
    })
}

/// Which write the page asked for: with the old file as the registry, only
/// an add can go ahead (creating the new file, on consent). An edit or a
/// remove would act on the old file, which mecha never writes (§3), so it
/// is refused with what to do instead (review of #750).
#[derive(Clone, Copy, PartialEq)]
enum Op {
    Add,
    Edit,
    Remove,
}

/// A write, behind the unlock and the old-file confirmation.
async fn write(
    state: St,
    token: Option<String>,
    create_new: bool,
    kind: Op,
    op: impl FnOnce(&std::path::Path) -> Result<(), loras::WriteError> + Send + 'static,
) -> Response {
    if !state.library.unlocked(token.as_deref()) {
        return (StatusCode::FORBIDDEN, "locked\n").into_response();
    }
    let done = tokio::task::spawn_blocking(move || {
        let here = Here::read().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
        if let Source::Legacy(old) = &here.loaded().source {
            let new = here.home.join(loras::FILE);
            if kind != Op::Add {
                return Err((
                    StatusCode::CONFLICT,
                    format!(
                        "This entry is in the old file {}, which mecha does not edit: change it there, or move its entries to {} first.",
                        old.display(),
                        new.display()
                    ),
                ));
            }
            if !create_new {
                return Err((
                    StatusCode::CONFLICT,
                    format!(
                        "Saving creates {}, and {} stops being read: move its entries there first, or confirm.",
                        new.display(),
                        old.display()
                    ),
                ));
            }
        }
        op(&here.home).map_err(|e| match e {
            loras::WriteError::NotFound => (StatusCode::NOT_FOUND, e.to_string()),
            loras::WriteError::Invalid(_) => (StatusCode::BAD_REQUEST, e.to_string()),
            loras::WriteError::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        })
    })
    .await;
    match done {
        Ok(Ok(())) => no_store(Json(json!({ "ok": true })).into_response()),
        Ok(Err((code, why))) => (code, format!("{why}\n")).into_response(),
        Err(e) => failed(e),
    }
}

async fn add(state: St, body: WriteBody, family: Family) -> Response {
    let WriteBody {
        unlock,
        entry,
        create_new,
    } = body;
    write(state, unlock, create_new, Op::Add, move |home| {
        loras::write_entry(home, family, &entry, None)
    })
    .await
}

async fn edit(state: St, name: String, body: WriteBody, family: Family) -> Response {
    let WriteBody {
        unlock,
        entry,
        create_new,
    } = body;
    write(state, unlock, create_new, Op::Edit, move |home| {
        loras::write_entry(home, family, &entry, Some(&name))
    })
    .await
}

async fn remove(state: St, name: String, q: UnlockQuery, family: Family) -> Response {
    write(state, q.unlock, false, Op::Remove, move |home| {
        loras::remove_entry(home, family, &name)
    })
    .await
}

pub async fn image_list(state: St, Query(q): Query<UnlockQuery>) -> Response {
    list(state, q, Family::QwenImage).await
}
pub async fn video_list(state: St, Query(q): Query<UnlockQuery>) -> Response {
    list(state, q, Family::H3).await
}
pub async fn image_add(state: St, Json(b): Json<WriteBody>) -> Response {
    add(state, b, Family::QwenImage).await
}
pub async fn video_add(state: St, Json(b): Json<WriteBody>) -> Response {
    add(state, b, Family::H3).await
}
pub async fn image_edit(state: St, Path(name): Path<String>, Json(b): Json<WriteBody>) -> Response {
    edit(state, name, b, Family::QwenImage).await
}
pub async fn video_edit(state: St, Path(name): Path<String>, Json(b): Json<WriteBody>) -> Response {
    edit(state, name, b, Family::H3).await
}
pub async fn image_remove(
    state: St,
    Path(name): Path<String>,
    Query(q): Query<UnlockQuery>,
) -> Response {
    remove(state, name, q, Family::QwenImage).await
}
pub async fn video_remove(
    state: St,
    Path(name): Path<String>,
    Query(q): Query<UnlockQuery>,
) -> Response {
    remove(state, name, q, Family::H3).await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn body(r: Response) -> (StatusCode, String) {
        let code = r.status();
        let bytes = axum::body::to_bytes(r.into_body(), usize::MAX)
            .await
            .unwrap();
        (code, String::from_utf8_lossy(&bytes).into_owned())
    }

    fn entry(name: &str, file: &str) -> EntryInput {
        EntryInput {
            name: name.into(),
            about: "a made-up wave".into(),
            file: Some(file.into()),
            ..Default::default()
        }
    }

    /// Locked, nothing is read or written: no names, abouts or triggers
    /// reach a page without the unlock (L3).
    #[tokio::test]
    async fn every_lora_route_needs_the_unlock() {
        let _home = crate::testenv::HomeGuard::new("loras-locked");
        let home = mecha_core::work::mecha_home().unwrap();
        std::fs::write(
            home.join(loras::FILE),
            "[[lora]]\nname = \"wave\"\nabout = \"a secret about\"\nmodel = \"h3\"\nfile = \"w.safetensors\"\n",
        )
        .unwrap();
        let state = super::super::tests::test_state();
        let (code, text) = body(
            list(
                State(state.clone()),
                UnlockQuery { unlock: None },
                Family::H3,
            )
            .await,
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap(),
            json!({"locked": true})
        );
        let wrong = Some("not-a-token".to_string());
        let (_, text) = body(
            list(
                State(state.clone()),
                UnlockQuery {
                    unlock: wrong.clone(),
                },
                Family::H3,
            )
            .await,
        )
        .await;
        assert!(!text.contains("secret"), "{text}");
        let w = WriteBody {
            unlock: wrong.clone(),
            entry: entry("b", "b.safetensors"),
            create_new: true,
        };
        assert_eq!(
            add(State(state.clone()), w, Family::H3).await.status(),
            StatusCode::FORBIDDEN
        );
        let r = remove(
            State(state.clone()),
            "wave".into(),
            UnlockQuery { unlock: wrong },
            Family::H3,
        )
        .await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert!(std::fs::read_to_string(home.join(loras::FILE))
            .unwrap()
            .contains("wave"));
    }

    /// Unlocked: each family sees its own entries with where they stand; an
    /// unanswered server is said rather than read as no files; a write while
    /// the old file is the registry asks first, then creates the new one.
    #[tokio::test]
    async fn the_pane_reads_and_writes_the_registry() {
        let _home = crate::testenv::HomeGuard::new("loras-pane");
        let home = mecha_core::work::mecha_home().unwrap();
        let old = home.join("video-loras.toml");
        std::fs::write(
            &old,
            "[[lora]]\nname = \"wave\"\nabout = \"a wave\"\nfile = \"w.safetensors\"\n",
        )
        .unwrap();
        // Port 9 answers nothing on loopback.
        std::fs::write(
            home.join("config.toml"),
            format!(
                "[image]\nurl = \"http://127.0.0.1:9\"\n[video]\nurl = \"http://127.0.0.1:9\"\nloras = \"{}\"\n",
                old.display()
            ),
        )
        .unwrap();
        let state = super::super::tests::test_state();
        let token = Some(state.library.grant_for_tests());
        let read = |family| {
            let state = state.clone();
            let token = token.clone();
            async move {
                let (code, text) =
                    body(list(State(state), UnlockQuery { unlock: token }, family).await).await;
                assert_eq!(code, StatusCode::OK, "{text}");
                serde_json::from_str::<Value>(&text).unwrap()
            }
        };
        let v = read(Family::H3).await;
        assert_eq!(v["source"], "legacy");
        assert_eq!(v["entries"][0]["name"], "wave");
        assert_eq!(v["entries"][0]["status"], "no_folder");
        assert!(v["entries"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("[video] loras_dir"));
        assert!(
            v["unused_error"]
                .as_str()
                .unwrap()
                .contains("did not answer"),
            "{v}"
        );
        assert_eq!(read(Family::QwenImage).await["entries"], json!([]));

        // The old file is the registry: the first write asks.
        let w = |create_new| WriteBody {
            unlock: token.clone(),
            entry: entry("washes", "wc.safetensors"),
            create_new,
        };
        let (code, text) = body(add(State(state.clone()), w(false), Family::QwenImage).await).await;
        assert_eq!(code, StatusCode::CONFLICT, "{text}");
        assert!(text.contains("stops being read"), "{text}");
        // An old entry is edited or removed in the old file, consent or not.
        let edit_old = WriteBody {
            unlock: token.clone(),
            entry: entry("wave", "w.safetensors"),
            create_new: true,
        };
        let (code, text) =
            body(edit(State(state.clone()), "wave".into(), edit_old, Family::H3).await).await;
        assert_eq!(code, StatusCode::CONFLICT, "{text}");
        assert!(text.contains("does not edit"), "{text}");
        let (code, text) = body(
            remove(
                State(state.clone()),
                "wave".into(),
                UnlockQuery {
                    unlock: token.clone(),
                },
                Family::H3,
            )
            .await,
        )
        .await;
        assert_eq!(code, StatusCode::CONFLICT, "{text}");
        assert!(text.contains("does not edit"), "{text}");
        assert!(!home.join(loras::FILE).exists());
        let (code, text) = body(add(State(state.clone()), w(true), Family::QwenImage).await).await;
        assert_eq!(code, StatusCode::OK, "{text}");
        let v = read(Family::QwenImage).await;
        assert_eq!(v["source"], "shared");
        assert_eq!(v["entries"][0]["name"], "washes");
        assert_eq!(v["ignored"], json!(old));
        // The clip section no longer reads the old file's entries.
        assert_eq!(read(Family::H3).await["entries"], json!([]));

        // A bad entry is refused with the parser's reason, a family's route
        // edits only its own entries, and remove takes it out.
        let bad = WriteBody {
            unlock: token.clone(),
            entry: EntryInput {
                always: true,
                ..entry("x", "x.safetensors")
            },
            create_new: false,
        };
        let (code, text) = body(add(State(state.clone()), bad, Family::QwenImage).await).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert!(text.contains("`always` is for clips"), "{text}");
        let r = edit(State(state.clone()), "washes".into(), w(false), Family::H3).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        let r = remove(
            State(state.clone()),
            "washes".into(),
            UnlockQuery {
                unlock: token.clone(),
            },
            Family::QwenImage,
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(read(Family::QwenImage).await["entries"], json!([]));
    }
}
