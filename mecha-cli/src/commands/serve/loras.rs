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
    let mut base = url.map_err(|e| format!("{e:#}"))?;
    // `join` replaces the last path segment unless the base ends in `/`.
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    let url = base
        .join("object_info/LoraLoaderModelOnly")
        .map_err(|e| e.to_string())?;
    // Loopback, never through a proxy and never a redirect away from the
    // address `loopback_url` vetted (`comfy::Client::new`'s reason; review
    // of #750).
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
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
    let folders = here.folders();
    let loaded = Loaded::read(&here.home, here.legacy(), &folders);
    let dirs = folders.get(family);
    let (source, path) = match &loaded.source {
        Source::Shared(p) => ("shared", Some(p.clone())),
        Source::Legacy(p) => ("legacy", Some(p.clone())),
        Source::None => ("none", None),
    };
    let mine: Vec<(loras::Entry, Status)> = loaded
        .entries
        .iter()
        .filter(|(e, _)| e.model == family)
        .cloned()
        .collect();
    let entries: Vec<Value> = mine
        .iter()
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
            // Only this family's entries claim a file: the same relative name
            // on the other server is another file (review of #750).
            loras::unused(&files, &mine, &turbo, dirs, family.dir_key())
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
        let here = Here::read().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, false, format!("{e:#}")))?;
        // Which file is the registry, and nothing more: no parse, no header.
        if let (Source::Legacy(old), _) = loras::source(&here.home, here.legacy()) {
            let new = here.home.join(loras::FILE);
            if kind != Op::Add {
                return Err((
                    StatusCode::CONFLICT,
                    false,
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
                    true,
                    format!(
                        "Saving creates {}, and {} stops being read: move its entries there first, or confirm.",
                        new.display(),
                        old.display()
                    ),
                ));
            }
        }
        op(&here.home).map_err(|e| match e {
            loras::WriteError::NotFound => (StatusCode::NOT_FOUND, false, e.to_string()),
            loras::WriteError::Invalid(_) => (StatusCode::BAD_REQUEST, false, e.to_string()),
            loras::WriteError::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, false, e.to_string()),
        })
    })
    .await;
    match done {
        Ok(Ok(())) => no_store(Json(json!({ "ok": true })).into_response()),
        // A 409 says whether sending it again with consent would go ahead:
        // only an add creating the new file can (review of #750).
        Ok(Err((StatusCode::CONFLICT, consent, why))) => (
            StatusCode::CONFLICT,
            Json(json!({ "message": why, "consent": consent })),
        )
            .into_response(),
        Ok(Err((code, _, why))) => (code, format!("{why}\n")).into_response(),
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
    let held = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = std::sync::Arc::clone(&held);
    let r = write(state, q.unlock, false, Op::Remove, move |home| {
        loras::remove_entry(home, family, &name)?;
        // A removed picture entry leaves every layer's list too (§6); a
        // clip entry's lists are step 2's.
        if family == Family::QwenImage {
            let n = loras::strip_from_layers(
                &mecha_core::sampling::chat_path(home),
                &home.join("personas"),
                &name,
            )
            .map_err(loras::WriteError::Io)?;
            count.store(n, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(())
    })
    .await;
    if r.status() != StatusCode::OK {
        return r;
    }
    let held = held.load(std::sync::atomic::Ordering::Relaxed);
    no_store(Json(json!({ "ok": true, "layers": held })).into_response())
}

#[derive(Debug, Deserialize)]
pub struct LayerBody {
    #[serde(default)]
    unlock: Option<String>,
    /// Follow the chat's list (a persona's only).
    #[serde(default)]
    inherit: bool,
    #[serde(default)]
    names: Vec<String>,
}

/// Which picture layer a route names: the chat's, or a persona's.
enum Which {
    Chat,
    Persona(String),
}

fn layer_word(l: &loras::Layer) -> &'static str {
    match l {
        loras::Layer::Inherit => "inherit",
        loras::Layer::Set(_) => "set",
        loras::Layer::Damaged(_) => "damaged",
    }
}

fn names_of(l: &loras::Layer) -> Vec<String> {
    match l {
        loras::Layer::Set(n) => n.clone(),
        _ => Vec::new(),
    }
}

/// The persona's directory in the store, when it has one.
fn persona_dir(home: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    mecha_core::persona::validate_name(name).ok()?;
    let dir = home.join("personas");
    dir.join(name).join("state.toml").exists().then_some(dir)
}

/// One picture layer's LoRA list, with the picture entries it may name.
fn layer_view(here: &Here, which: &Which) -> Result<Value, (StatusCode, String)> {
    let chat_path = mecha_core::sampling::chat_path(&here.home);
    let chat = mecha_core::sampling::chat_loras(&chat_path);
    let layer = match which {
        Which::Chat => chat.clone(),
        Which::Persona(name) => {
            let dir = persona_dir(&here.home, name)
                .ok_or((StatusCode::NOT_FOUND, "no such persona".to_string()))?;
            mecha_core::persona::loras(&dir, name)
        }
    };
    let loaded = Loaded::read(&here.home, None, &here.folders());
    let choices: Vec<Value> = loaded
        .entries
        .iter()
        .filter(|(e, _)| e.model == Family::QwenImage)
        .map(|(e, st)| {
            json!({"name": e.name, "about": e.about, "status": status_word(st), "reason": st.reason(e)})
        })
        .collect();
    Ok(json!({
        "locked": false,
        "layer": layer_word(&layer),
        "names": names_of(&layer),
        // What inherit means for a persona: the chat's list as it is now.
        "chat": names_of(&chat),
        "chat_damaged": matches!(chat, loras::Layer::Damaged(_)),
        "max": loras::MAX_PER_PICTURE,
        "choices": choices,
    }))
}

async fn layer_get(state: St, q: UnlockQuery, which: Which) -> Response {
    if !state.library.unlocked(q.unlock.as_deref()) {
        return locked();
    }
    let got = tokio::task::spawn_blocking(move || {
        let here =
            Here::read().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
        layer_view(&here, &which)
    })
    .await;
    match got {
        Ok(Ok(v)) => no_store(Json(v).into_response()),
        Ok(Err((code, why))) => (code, format!("{why}\n")).into_response(),
        Err(e) => failed(e),
    }
}

/// Write one picture layer's list, checked against the registry first
/// (`loras::check_layer`): only picture entries, none twice, at most the
/// cap. This route alone writes the lists; the picture-settings routes keep
/// them as they are.
async fn layer_put(state: St, body: LayerBody, which: Which) -> Response {
    if !state.library.unlocked(body.unlock.as_deref()) {
        return (StatusCode::FORBIDDEN, "locked\n").into_response();
    }
    let done = tokio::task::spawn_blocking(move || {
        let here =
            Here::read().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
        let layer = if body.inherit {
            if matches!(which, Which::Chat) {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "the chat's list has no layer above it to inherit".to_string(),
                ));
            }
            loras::Layer::Inherit
        } else {
            let loaded = Loaded::read(&here.home, None, &here.folders());
            if let Some(why) = &loaded.unreadable {
                return Err((StatusCode::CONFLICT, format!("the registry {why}")));
            }
            // A list stored past the cap (hand-written, or before a lower
            // cap) may be shortened a name at a time without being refused
            // at every step: a write that only drops names from the stored
            // list passes the cap (review of #751).
            let stored = match &which {
                Which::Chat => {
                    mecha_core::sampling::chat_loras(&mecha_core::sampling::chat_path(&here.home))
                }
                Which::Persona(name) => persona_dir(&here.home, name)
                    .map(|dir| mecha_core::persona::loras(&dir, name))
                    .unwrap_or_default(),
            };
            let shrinking = matches!(&stored, loras::Layer::Set(old)
                if body.names.len() < old.len()
                    && body.names.iter().all(|n| old.contains(n)));
            let cap = if shrinking {
                body.names.len().max(loras::MAX_PER_PICTURE)
            } else {
                loras::MAX_PER_PICTURE
            };
            loras::Layer::Set(
                loras::check_layer_within(&body.names, &loaded, cap)
                    .map_err(|e| (StatusCode::BAD_REQUEST, format!("{e:#}")))?,
            )
        };
        let wrote = match &which {
            Which::Chat => mecha_core::sampling::set_chat_loras(
                &mecha_core::sampling::chat_path(&here.home),
                &layer,
            ),
            Which::Persona(name) => {
                let dir = persona_dir(&here.home, name)
                    .ok_or((StatusCode::NOT_FOUND, "no such persona".to_string()))?;
                mecha_core::persona::set_loras(&dir, name, layer).map(|_| ())
            }
        };
        wrote.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
        layer_view(&here, &which)
    })
    .await;
    match done {
        Ok(Ok(v)) => no_store(Json(v).into_response()),
        Ok(Err((code, why))) => (code, format!("{why}\n")).into_response(),
        Err(e) => failed(e),
    }
}

pub async fn image_chat_layer(state: St, Query(q): Query<UnlockQuery>) -> Response {
    layer_get(state, q, Which::Chat).await
}
pub async fn image_chat_layer_save(state: St, Json(b): Json<LayerBody>) -> Response {
    layer_put(state, b, Which::Chat).await
}
pub async fn image_persona_layer(
    state: St,
    Path(name): Path<String>,
    Query(q): Query<UnlockQuery>,
) -> Response {
    layer_get(state, q, Which::Persona(name)).await
}
pub async fn image_persona_layer_save(
    state: St,
    Path(name): Path<String>,
    Json(b): Json<LayerBody>,
) -> Response {
    layer_put(state, b, Which::Persona(name)).await
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
        assert!(
            text.contains("stops being read") && text.contains("\"consent\":true"),
            "{text}"
        );
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

    /// The picture layers' lists (§6): behind the unlock like every LoRA
    /// route; a write is checked against the registry (picture entries
    /// only, the cap); a persona may inherit and the chat may not; and a
    /// removed entry leaves every list that held it, with the count said.
    #[tokio::test]
    async fn the_picture_layers_are_read_and_written_behind_the_unlock() {
        let _home = crate::testenv::HomeGuard::new("loras-layers");
        let home = mecha_core::work::mecha_home().unwrap();
        std::fs::write(
            home.join(loras::FILE),
            "[[lora]]\nname = \"paint\"\nabout = \"a made-up paint look\"\nmodel = \"qwen-image\"\nfile = \"p.safetensors\"\n\n\
             [[lora]]\nname = \"wave\"\nabout = \"a made-up wave\"\nmodel = \"h3\"\nfile = \"w.safetensors\"\n",
        )
        .unwrap();
        std::fs::write(
            home.join("config.toml"),
            "[image]\nurl = \"http://127.0.0.1:9\"\n",
        )
        .unwrap();
        let lib = mecha_core::imagelib::Library::default();
        mecha_core::persona::create(
            &home.join("personas"),
            &lib,
            mecha_core::persona::NewPersona {
                name: "mara".into(),
                display: "Mara".into(),
                origin: mecha_core::persona::Origin::Owner,
                ..Default::default()
            },
        )
        .unwrap();
        let state = super::super::tests::test_state();
        let token = Some(state.library.grant_for_tests());
        let get = |which: &str, unlock: Option<String>| {
            let state = state.clone();
            let which = which.to_string();
            async move {
                let q = UnlockQuery { unlock };
                let r = if which == "chat" {
                    image_chat_layer(State(state), Query(q)).await
                } else {
                    image_persona_layer(State(state), Path(which), Query(q)).await
                };
                body(r).await
            }
        };
        let put = |which: &str, inherit: bool, names: &[&str], unlock: Option<String>| {
            let state = state.clone();
            let which = which.to_string();
            let b = LayerBody {
                unlock,
                inherit,
                names: names.iter().map(|n| n.to_string()).collect(),
            };
            async move {
                let r = if which == "chat" {
                    image_chat_layer_save(State(state), Json(b)).await
                } else {
                    image_persona_layer_save(State(state), Path(which), Json(b)).await
                };
                body(r).await
            }
        };
        // Locked: no names, and no write.
        let (code, text) = get("chat", None).await;
        assert_eq!((code, text.trim()), (StatusCode::OK, "{\"locked\":true}"));
        let (code, _) = put("chat", false, &["paint"], None).await;
        assert_eq!(code, StatusCode::FORBIDDEN);
        assert_eq!(
            mecha_core::sampling::chat_loras(&mecha_core::sampling::chat_path(&home)),
            loras::Layer::Inherit
        );
        // Checked against the registry.
        let (code, text) = put("chat", false, &["wave"], token.clone()).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert!(text.contains("clip LoRA"), "{text}");
        let (code, _) = put("chat", true, &[], token.clone()).await;
        assert_eq!(
            code,
            StatusCode::BAD_REQUEST,
            "the chat has nothing above it"
        );
        let (code, text) = put("chat", false, &["paint"], token.clone()).await;
        assert_eq!(code, StatusCode::OK, "{text}");
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["names"], json!(["paint"]));
        assert_eq!(v["choices"][0]["name"], "paint");
        assert_eq!(v["choices"].as_array().unwrap().len(), 1, "pictures only");
        // A list stored past the cap shrinks a name at a time; it never
        // grows, and a new long list is still refused (review of #751).
        std::fs::write(
            mecha_core::sampling::chat_path(&home),
            "loras = [\"paint\", \"paint2\", \"paint3\", \"paint4\", \"paint5\"]\n",
        )
        .unwrap();
        let mut long = String::new();
        for n in 2..=5 {
            long.push_str(&format!(
                "\n[[lora]]\nname = \"paint{n}\"\nabout = \"x\"\nmodel = \"qwen-image\"\nfile = \"p{n}.safetensors\"\n"
            ));
        }
        let registry = std::fs::read_to_string(home.join(loras::FILE)).unwrap();
        std::fs::write(home.join(loras::FILE), format!("{registry}{long}")).unwrap();
        let (code, text) = put(
            "chat",
            false,
            &["paint", "paint2", "paint3", "paint4"],
            token.clone(),
        )
        .await;
        assert_eq!(code, StatusCode::OK, "{text}");
        let (code, _) = put(
            "chat",
            false,
            &["paint", "paint2", "paint3", "paint5"],
            token.clone(),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST, "a swap is not a shrink");
        let (code, text) = put("chat", false, &["paint"], token.clone()).await;
        assert_eq!(code, StatusCode::OK, "{text}");
        std::fs::write(home.join(loras::FILE), registry).unwrap();
        // A persona starts inheriting, may choose none, and inherits again.
        let (_, text) = get("mara", token.clone()).await;
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            (v["layer"].as_str(), &v["chat"]),
            (Some("inherit"), &json!(["paint"]))
        );
        let (code, text) = put("mara", false, &[], token.clone()).await;
        assert_eq!(code, StatusCode::OK, "{text}");
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap()["layer"],
            "set"
        );
        let (code, _) = get("nobody", token.clone()).await;
        assert_eq!(code, StatusCode::NOT_FOUND);
        put("mara", false, &["paint"], token.clone()).await;
        // Removing the entry takes it off both lists, and says how many.
        let r = remove(
            State(state.clone()),
            "paint".into(),
            UnlockQuery {
                unlock: token.clone(),
            },
            Family::QwenImage,
        )
        .await;
        let (code, text) = body(r).await;
        assert_eq!(code, StatusCode::OK, "{text}");
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["layers"], 2);
        let (_, text) = get("mara", token.clone()).await;
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap()["names"],
            json!([])
        );
        let (_, text) = get("chat", token).await;
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap()["names"],
            json!([])
        );
    }
}
