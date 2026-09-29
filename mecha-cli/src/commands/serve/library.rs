//! The image library's web door (`docs/IMAGE-COMPILER-DESIGN.md` §7).
//!
//! **Reads here, writes by the CLI.** The list and the portraits are read
//! straight from the store; every change — approve, reject, lock, remove,
//! save — is a `mecha imagelib …` child, the house rule every other surface
//! on this server keeps, so the page can do nothing the owner's terminal
//! could not.
//!
//! **The lock is a browse filter, enforced here, not on the page** (the
//! owner's ruling, 2026-09-28). Locked entries are left out of the list and
//! their portraits answer 404 unless the request carries a live unlock token
//! — a blurred thumbnail would still ship its bytes to the page and the
//! browser's cache. The token is minted by `POST /api/library/unlock` — from
//! the password `mecha imagelib set-lock-password` set, or, when none is set,
//! for the asking: the password is optional, and without one the lock is a
//! plain show/hide toggle (the owner's ruling, 2026-09-28). It is held in
//! this process's memory only and expires after [`UNLOCK_IDLE`] without use.
//! The page keeps it in a variable — no cookie and no storage, which
//! `web/test/no-storage.mjs` forbids — so a reload locks again. Generation
//! ignores all of this: the lock hides, it never withholds.
//!
//! **Approval is of what was shown, and only this server can vouch for it.**
//! The list carries, per entry, an HMAC of its `shown_digest` under a key
//! this process draws at start and never writes down; the approve button
//! sends it back, and approval happens here, in process, only if the entry
//! as re-read still signs the same. A plain digest would not do: it is a
//! hash of text anyone who can read the store can see, so a `--shown` flag
//! on the CLI let any shell compute it and approve a model's proposal with
//! no one reading it (found on review of #385). This is the one write not
//! made by a CLI child, because the CLI has no way to prove a person saw
//! the text — the web page is that proof, and only this process can check it.
//! What that buys, stated exactly: approval now requires a client of this
//! server, as every approve route on it does; an owner-authenticated client
//! can fetch a signature and send it back. The gap it closed was a shell
//! with no server at all.
//!
//! **Writes honour the lock as reads do.** An action on a locked entry needs
//! a live unlock token, and a hidden entry and a missing one answer the same
//! 404 before any child runs — otherwise `unlock` was a way to reveal a
//! hidden entry with no password, and the 200/409 split named which exist.
//!
//! **The owner adds and edits here too** — a character from an uploaded
//! portrait, a style from its text, a new description or portrait for an
//! approved entry — each the same `mecha imagelib` child the terminal runs.
//! A portrait arrives base64 in the JSON body beside its name and text, so
//! none of the owner's words ride in a URL, and is staged in a private
//! scratch directory for the child to read. Edit is for approved entries
//! only: a candidate is approved or rejected as the model wrote it, where it
//! can be read, rather than rewritten into approval on the way past.
//!
//! `add` and `save` do not go through the lock: a new name that is taken
//! answers "already exists" whether or not the entry holding it is locked.
//! That names a locked entry to anyone adding one, and nothing more — no
//! text, no portrait, no change — and is the cost of being told the name is
//! taken rather than silently failing (review of #394).

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use mecha_core::imagelib::{self, Entry, Kind, Library};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

type St = State<super::WebState>;

/// How long an unlock lasts without use.
pub const UNLOCK_IDLE: Duration = Duration::from_secs(30 * 60);
/// Wrong passwords allowed per window before the door answers 429.
const MAX_FAILURES: usize = 5;
const FAILURE_WINDOW: Duration = Duration::from_secs(5 * 60);

/// The library's directory and the unlocks this process has granted.
pub struct LibraryState {
    pub dir: PathBuf,
    unlocks: Mutex<HashMap<String, Instant>>,
    failures: Mutex<Vec<Instant>>,
    /// Signs what the page was shown; drawn at start, never stored.
    key: [u8; 32],
}

impl LibraryState {
    pub fn new(dir: PathBuf) -> LibraryState {
        let mut key = [0u8; 32];
        key[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        key[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        LibraryState {
            dir,
            unlocks: Mutex::new(HashMap::new()),
            failures: Mutex::new(Vec::new()),
            key,
        }
    }

    /// HMAC-SHA256 of an entry's `shown_digest` under this process's key,
    /// hex: what the page sends back to approve exactly what it displayed.
    fn sign(&self, digest: &str) -> String {
        imagelib::sign_shown(&self.key, digest)
    }

    /// Whether `presented` is this process's signature of `digest`, compared
    /// without an early exit.
    fn signed(&self, digest: &str, presented: &str) -> bool {
        let want = self.sign(digest);
        want.len() == presented.len()
            && want
                .bytes()
                .zip(presented.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
    }

    /// Whether `token` is a live unlock, refreshing its idle clock if so.
    fn unlocked(&self, token: Option<&str>) -> bool {
        let Some(token) = token.filter(|t| !t.is_empty()) else {
            return false;
        };
        let mut unlocks = self.unlocks.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        unlocks.retain(|_, expires| *expires > now);
        match unlocks.get_mut(token) {
            Some(expires) => {
                *expires = now + UNLOCK_IDLE;
                true
            }
            None => false,
        }
    }

    fn grant(&self) -> String {
        let token = uuid::Uuid::new_v4().simple().to_string();
        self.unlocks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(token.clone(), Instant::now() + UNLOCK_IDLE);
        token
    }

    fn revoke(&self, token: &str) {
        self.unlocks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(token);
    }

    /// Reserve an attempt, counted as a failure until it proves otherwise;
    /// `false` once the window is full. Reserving before the check is what
    /// makes the ceiling hold for guesses sent in parallel (review of #385).
    fn may_try(&self) -> bool {
        let mut failures = self.failures.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        failures.retain(|t| now.duration_since(*t) < FAILURE_WINDOW);
        if failures.len() >= MAX_FAILURES {
            return false;
        }
        failures.push(now);
        true
    }

    /// A reserved attempt succeeded: it was not a failure after all.
    fn succeeded(&self) {
        self.failures
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .pop();
    }
}

#[derive(Deserialize, Default)]
pub struct UnlockQuery {
    #[serde(default)]
    unlock: Option<String>,
}

fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

fn kind_label(kind: Kind) -> &'static str {
    kind.label()
}

fn portrait_url(entry: &Entry, token: Option<&str>) -> Option<String> {
    let blob = entry.portrait.as_deref()?;
    Some(match (entry.locked, token) {
        (true, Some(t)) => format!("/api/library/portrait/{blob}?unlock={t}"),
        _ => format!("/api/library/portrait/{blob}"),
    })
}

/// GET /api/library — every entry the owner may see now: approved and
/// candidate, locked ones only while unlocked. `no-store`, because a list
/// that held locked entries must not linger in a cache.
pub async fn list(State(state): St, Query(q): Query<UnlockQuery>) -> Response {
    let lib_state = &state.library;
    let unlocked = lib_state.unlocked(q.unlock.as_deref());
    let token = unlocked.then(|| q.unlock.clone()).flatten();
    let dir = lib_state.dir.clone();
    let loaded = tokio::task::spawn_blocking(move || {
        let (lib, errors) = Library::load(&dir);
        (lib, errors, imagelib::has_lock_password(&dir))
    })
    .await;
    let Ok((lib, errors, has_password)) = loaded else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "reading the library\n").into_response();
    };
    let mut hidden = 0usize;
    let entries: Vec<serde_json::Value> = lib
        .all()
        .iter()
        .filter(|e| {
            let show = !e.locked || unlocked;
            if !show {
                hidden += 1;
            }
            show
        })
        .map(|e| {
            serde_json::json!({
                "kind": kind_label(e.kind),
                "name": e.name,
                "version": e.version,
                "status": e.status,
                "origin": e.origin,
                "locked": e.locked,
                "text": e.text,
                "created": e.created,
                "portrait": portrait_url(e, token.as_deref()),
                "shown": lib_state.sign(&imagelib::shown_digest(e)),
            })
        })
        .collect();
    no_store(
        Json(serde_json::json!({
            "entries": entries,
            "hidden_locked": hidden,
            "unlocked": unlocked,
            "has_password": has_password,
            "unreadable": errors.len(),
        }))
        .into_response(),
    )
}

/// A blob name as the store writes it, and nothing else.
fn valid_blob(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("sha256-") else {
        return false;
    };
    let Some((hex, ext)) = rest.split_once('.') else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && matches!(ext, "png" | "jpg" | "webp")
}

/// GET /api/library/portrait/{blob} — a portrait, only as some entry the
/// owner may see now names it. Everything else, including a blob only a
/// superseded version names, is the same 404.
pub async fn portrait(
    State(state): St,
    UrlPath(blob): UrlPath<String>,
    Query(q): Query<UnlockQuery>,
) -> Response {
    if !valid_blob(&blob) {
        return (StatusCode::NOT_FOUND, "no such portrait\n").into_response();
    }
    let unlocked = state.library.unlocked(q.unlock.as_deref());
    let dir = state.library.dir.clone();
    let read = tokio::task::spawn_blocking(move || {
        let (lib, _) = Library::load(&dir);
        let naming: Vec<&Entry> = lib
            .all()
            .iter()
            .filter(|e| e.portrait.as_deref() == Some(blob.as_str()))
            .collect();
        let open = naming.iter().any(|e| !e.locked);
        let entry = naming.into_iter().find(|e| !e.locked || unlocked)?;
        let (bytes, ext) = lib.read_portrait(entry).ok()?;
        Some((bytes, ext, open))
    })
    .await;
    let Ok(Some((bytes, ext, open))) = read else {
        return (StatusCode::NOT_FOUND, "no such portrait\n").into_response();
    };
    let content_type = match ext {
        "jpg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "image/png",
    };
    // Content-addressed, so an open portrait never changes under its name;
    // one only a locked entry names is never cached. What this cannot do is
    // reach back: a portrait browsed while its entry was open stays in that
    // browser's cache after the entry is locked — the list hides its name
    // from then on, so nothing asks for it, but the bytes are there until the
    // cache evicts them or the browser's data is cleared (review of #385).
    let cache = if open {
        "private, max-age=31536000, immutable"
    } else {
        "no-store"
    };
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, cache),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct UnlockBody {
    /// Absent when no password is set: then the lock is a plain toggle.
    #[serde(default)]
    password: String,
}

/// POST /api/library/unlock — the lock password for a token. Rate-limited
/// per process; a wrong password and an unset one answer the same 403, so
/// the door says nothing about which it was.
pub async fn unlock(State(state): St, Json(body): Json<UnlockBody>) -> Response {
    // No password set: the lock is a plain show/hide toggle (the owner's
    // ruling, 2026-09-28 — the password is optional). Checked by the file's
    // presence, so a damaged lock file still goes through verification below
    // and errors, never opens.
    let dir = state.library.dir.clone();
    let has = tokio::task::spawn_blocking(move || imagelib::has_lock_password(&dir))
        .await
        .unwrap_or(true);
    if !has {
        return no_store(
            Json(serde_json::json!({
                "token": state.library.grant(),
                "idle_secs": UNLOCK_IDLE.as_secs(),
            }))
            .into_response(),
        );
    }
    if !state.library.may_try() {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "too many wrong passwords; wait a few minutes\n",
        )
            .into_response();
    }
    let dir = state.library.dir.clone();
    let verdict =
        tokio::task::spawn_blocking(move || imagelib::verify_lock_password(&dir, &body.password))
            .await;
    if matches!(verdict, Ok(Ok(true))) {
        state.library.succeeded();
    }
    match verdict {
        Ok(Ok(true)) => no_store(
            Json(serde_json::json!({
                "token": state.library.grant(),
                "idle_secs": UNLOCK_IDLE.as_secs(),
            }))
            .into_response(),
        ),
        Ok(Ok(false)) => (StatusCode::FORBIDDEN, "wrong password\n").into_response(),
        // A damaged lock file is a finding, never an open door.
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "checking the password\n").into_response(),
    }
}

#[derive(Deserialize)]
pub struct TokenBody {
    token: String,
}

/// POST /api/library/relock — end an unlock before its idle expiry.
pub async fn relock(State(state): St, Json(body): Json<TokenBody>) -> Response {
    state.library.revoke(&body.token);
    Json(serde_json::json!({"ok": true})).into_response()
}

fn parse_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "character" => Some("character"),
        "style" => Some("style"),
        _ => None,
    }
}

#[derive(Deserialize, Default)]
pub struct ActBody {
    /// This server's signature of what the page displayed, for `approve`.
    #[serde(default)]
    shown: Option<String>,
    /// The unlock token, for an action on a locked entry.
    #[serde(default)]
    unlock: Option<String>,
}

/// POST /api/library/{kind}/{name}/{action} — approve (as shown), reject,
/// lock, unlock, remove; each a `mecha imagelib` child.
pub async fn act(
    State(state): St,
    UrlPath((kind, name, action)): UrlPath<(String, String, String)>,
    body: Option<Json<ActBody>>,
) -> Response {
    let Some(kind) = parse_kind(&kind) else {
        return (StatusCode::BAD_REQUEST, "kind is character or style\n").into_response();
    };
    if imagelib::validate_name(&name).is_err() {
        return (StatusCode::BAD_REQUEST, "not an entry name\n").into_response();
    }
    let body = body.map(|Json(b)| b).unwrap_or_default();
    if !matches!(
        action.as_str(),
        "approve" | "reject" | "lock" | "unlock" | "remove"
    ) {
        return (StatusCode::NOT_FOUND, "no such action\n").into_response();
    }
    let Some(entry) = visible(&state, kind, &name, body.unlock.as_deref()).await else {
        return (StatusCode::NOT_FOUND, "no such entry\n").into_response();
    };
    let args: Vec<&str> = match action.as_str() {
        "approve" => {
            let Some(shown) = body.shown else {
                return (StatusCode::BAD_REQUEST, "approve needs what was shown\n").into_response();
            };
            return approve_shown(&state, entry, shown).await;
        }
        "reject" => vec!["imagelib", "reject", &name, "--kind", kind],
        "lock" => vec!["imagelib", "lock", &name, "--kind", kind],
        "unlock" => vec!["imagelib", "unlock", &name, "--kind", kind],
        "remove" => vec!["imagelib", "remove", &name, "--kind", kind, "--yes"],
        _ => unreachable!("actions are matched above"),
    };
    super::review::verb(&state, &args).await
}

/// The entry as this request may see it. The lock holds for writes as for
/// reads: a locked entry is acted on only while unlocked, and hidden answers
/// exactly as missing does.
async fn visible(
    state: &super::WebState,
    kind: &'static str,
    name: &str,
    unlock: Option<&str>,
) -> Option<Entry> {
    let unlocked = state.library.unlocked(unlock);
    let dir = state.library.dir.clone();
    let n = name.to_string();
    let kind = if kind == "style" {
        Kind::Style
    } else {
        Kind::Character
    };
    tokio::task::spawn_blocking(move || Library::load(&dir).0.get(kind, &n).cloned())
        .await
        .ok()
        .flatten()
        .filter(|e| !e.locked || unlocked)
}

/// The largest body `add` and `edit` take: a portrait at the store's cap,
/// base64'd, and room for the rest of the JSON.
pub const MAX_WRITE_BODY: usize =
    (imagelib::MAX_PORTRAIT_BYTES as usize).div_ceil(3) * 4 + 64 * 1024;

/// A portrait as the page sends it — base64 — decoded, capped, and staged in
/// a private scratch directory for the child to read.
async fn stage_portrait(b64: &str) -> Result<tempdir::Dir, Box<Response>> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|_| {
            Box::new((StatusCode::BAD_REQUEST, "the portrait is not base64\n").into_response())
        })?;
    if bytes.is_empty() {
        return Err(Box::new(
            (StatusCode::BAD_REQUEST, "the portrait is empty\n").into_response(),
        ));
    }
    if bytes.len() as u64 > imagelib::MAX_PORTRAIT_BYTES {
        return Err(Box::new(
            (
                StatusCode::PAYLOAD_TOO_LARGE,
                "the picture is over the portrait cap\n",
            )
                .into_response(),
        ));
    }
    tokio::task::spawn_blocking(move || {
        tempdir::Dir::new().and_then(|d| std::fs::write(d.file(), &bytes).map(|_| d))
    })
    .await
    .ok()
    .and_then(Result::ok)
    .ok_or_else(|| {
        Box::new(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not stage the picture for saving\n",
            )
                .into_response(),
        )
    })
}

#[derive(Deserialize)]
pub struct AddBody {
    kind: String,
    name: String,
    /// A character's description, or a style's text.
    text: String,
    #[serde(default)]
    locked: bool,
    /// A character's portrait, base64.
    #[serde(default)]
    portrait: Option<String>,
}

/// POST /api/library/add — a new character from an uploaded portrait, or a
/// new style from its text: `mecha imagelib add-character` / `add-style`.
/// The owner's own act, so approved on creation.
pub async fn add(State(state): St, Json(body): Json<AddBody>) -> Response {
    let Some(kind) = parse_kind(&body.kind) else {
        return (StatusCode::BAD_REQUEST, "kind is character or style\n").into_response();
    };
    if imagelib::validate_name(&body.name).is_err() {
        return (
            StatusCode::BAD_REQUEST,
            "a name is lowercase letters, digits and hyphens\n",
        )
            .into_response();
    }
    // `--flag=…` throughout: a text that opens with a dash is text, not a
    // flag (review of #385).
    let text = match kind {
        "character" => format!("--description={}", body.text),
        _ => format!("--text={}", body.text),
    };
    let staged = match (kind, body.portrait.as_deref()) {
        ("character", Some(b64)) => match stage_portrait(b64).await {
            Ok(dir) => Some(dir),
            Err(refusal) => return *refusal,
        },
        ("character", None) => {
            return (StatusCode::BAD_REQUEST, "a character needs a portrait\n").into_response()
        }
        (_, Some(_)) => {
            return (StatusCode::BAD_REQUEST, "a style has no portrait\n").into_response()
        }
        (_, None) => None,
    };
    let portrait = staged
        .as_ref()
        .map(|d| d.file().to_string_lossy().into_owned());
    let mut args = match &portrait {
        Some(path) => vec!["imagelib", "add-character", &body.name, "--portrait", path],
        None => vec!["imagelib", "add-style", &body.name],
    };
    args.push(&text);
    if body.locked {
        args.push("--locked");
    }
    let response = super::review::verb(&state, &args).await;
    drop(staged);
    response
}

#[derive(Deserialize)]
pub struct EditBody {
    kind: String,
    name: String,
    /// A new description or style text; absent leaves it.
    #[serde(default)]
    text: Option<String>,
    /// A character's new portrait, base64; absent leaves it.
    #[serde(default)]
    portrait: Option<String>,
    /// The unlock token, for a locked entry.
    #[serde(default)]
    unlock: Option<String>,
}

/// POST /api/library/edit — a new version of an approved entry: `mecha
/// imagelib update`. The old version stays in its history, so a picture's
/// manifest naming it still resolves. An uploaded portrait has no seed, so
/// the entry's recorded seed goes with the old portrait.
pub async fn edit(State(state): St, Json(body): Json<EditBody>) -> Response {
    let Some(kind) = parse_kind(&body.kind) else {
        return (StatusCode::BAD_REQUEST, "kind is character or style\n").into_response();
    };
    if imagelib::validate_name(&body.name).is_err() {
        return (StatusCode::BAD_REQUEST, "not an entry name\n").into_response();
    }
    if body.text.is_none() && body.portrait.is_none() {
        return (StatusCode::BAD_REQUEST, "nothing to change\n").into_response();
    }
    if kind == "style" && body.portrait.is_some() {
        return (StatusCode::BAD_REQUEST, "a style has no portrait\n").into_response();
    }
    let Some(entry) = visible(&state, kind, &body.name, body.unlock.as_deref()).await else {
        return (StatusCode::NOT_FOUND, "no such entry\n").into_response();
    };
    if entry.status != imagelib::Status::Approved {
        return (
            StatusCode::CONFLICT,
            format!(
                "`{}` is waiting for a decision — approve or reject it as written first\n",
                entry.name
            ),
        )
            .into_response();
    }
    let staged = match body.portrait.as_deref() {
        Some(b64) => match stage_portrait(b64).await {
            Ok(dir) => Some(dir),
            Err(refusal) => return *refusal,
        },
        None => None,
    };
    let portrait = staged
        .as_ref()
        .map(|d| d.file().to_string_lossy().into_owned());
    let text = body.text.as_ref().map(|t| format!("--text={t}"));
    let mut args = vec!["imagelib", "update", &body.name, "--kind", kind];
    if let Some(text) = &text {
        args.push(text);
    }
    if let Some(path) = &portrait {
        args.extend(["--portrait", path]);
    }
    let response = super::review::verb(&state, &args).await;
    drop(staged);
    response
}

#[derive(Deserialize)]
pub struct SourceQuery {
    key: String,
    path: String,
}

/// Approve `entry` as the page showed it, in this process: the signature
/// must be ours, over the entry as re-read now. `approve_as_shown` re-reads
/// once more at the write, so a text that moves between the check and the
/// write is refused too.
async fn approve_shown(state: &super::WebState, entry: Entry, shown: String) -> Response {
    let lib = std::sync::Arc::clone(&state.library);
    let done = tokio::task::spawn_blocking(move || {
        let digest = imagelib::shown_digest(&entry);
        if !lib.signed(&digest, &shown) {
            return Err((
                StatusCode::CONFLICT,
                format!(
                    "`{}` is not what was shown — reload and read it again\n",
                    entry.name
                ),
            ));
        }
        imagelib::approve_as_shown(&lib.dir, entry.kind, &entry.name, &digest)
            .map_err(|e| (StatusCode::CONFLICT, format!("{e:#}\n")))
    })
    .await;
    match done {
        Ok(Ok(e)) => Json(serde_json::json!({
            "ok": true,
            "output": format!("Approved `{}`.", e.name),
        }))
        .into_response(),
        Ok(Err(refusal)) => refusal.into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "approving\n").into_response(),
    }
}

/// The manifest beside a chat image, when it has one.
fn read_manifest(ws: &std::path::Path, png: &str) -> Option<serde_json::Value> {
    use std::io::Read;
    let json = png.strip_suffix(".png")?.to_string() + ".json";
    let (mut file, _) = mecha_core::workspace_files::WorkspaceFiles::open(ws)
        .ok()?
        .read(&json)
        .ok()?;
    let mut text = String::new();
    file.by_ref()
        .take(256 * 1024)
        .read_to_string(&mut text)
        .ok()?;
    serde_json::from_str(&text).ok()
}

/// Refuse a key the library must not be written from: an incognito room
/// writes nothing outside itself, and saving to the library would.
fn refuse_incognito(key: &str) -> Option<Response> {
    if !super::chat::valid_key(key) {
        return Some((StatusCode::BAD_REQUEST, "bad session key\n").into_response());
    }
    super::incognito::is_incognito_key(key).then(|| {
        (
            StatusCode::FORBIDDEN,
            "an incognito chat cannot save to the library\n",
        )
            .into_response()
    })
}

/// GET /api/library/source?key&path — what the save dialog starts from: the
/// seed that drew the picture, who was cast in it, and whether any of them
/// is locked — the lock box starts checked when one is (the owner's ruling:
/// inherited by default, overridable).
pub async fn source(State(state): St, Query(q): Query<SourceQuery>) -> Response {
    if let Some(refusal) = refuse_incognito(&q.key) {
        return refusal;
    }
    let ws = match super::chat::attachment_workspace(&state, &q.key, false).await {
        Ok(ws) => ws,
        Err(response) => return *response,
    };
    let dir = state.library.dir.clone();
    let path = q.path.clone();
    let out = tokio::task::spawn_blocking(move || {
        let manifest = read_manifest(&ws, &path);
        let cast: Vec<String> = manifest
            .as_ref()
            .and_then(|m| m["cast"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|c| c["name"].as_str().map(str::to_string))
            .collect();
        let (lib, _) = Library::load(&dir);
        // A yes or no, never names: the page may hold no unlock token, and
        // naming the locked characters in a picture would undo the list's
        // hiding (review of #385). Nor the cast, which can hold a locked name.
        let locked = cast
            .iter()
            .any(|n| lib.get(Kind::Character, n).is_some_and(|e| e.locked));
        serde_json::json!({
            "seed": manifest.as_ref().and_then(|m| m["seed"].as_u64()),
            "suggest_locked": locked,
            "has_password": imagelib::has_lock_password(&dir),
        })
    })
    .await;
    match out {
        Ok(v) => no_store(Json(v).into_response()),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "reading the picture\n").into_response(),
    }
}

#[derive(Deserialize)]
pub struct SaveBody {
    key: String,
    path: String,
    name: String,
    description: String,
    #[serde(default)]
    locked: bool,
}

/// POST /api/library/save — make a chat picture a character: the bytes are
/// copied in now through the path jail (a chat's files are served only while
/// it is open, so a pointer would not survive), the seed read from its
/// manifest, and `mecha imagelib add-character` does the rest. The owner's
/// own act, so the entry is approved on creation.
pub async fn save(State(state): St, Json(body): Json<SaveBody>) -> Response {
    if let Some(refusal) = refuse_incognito(&body.key) {
        return refusal;
    }
    if imagelib::validate_name(&body.name).is_err() {
        return (
            StatusCode::BAD_REQUEST,
            "a name is lowercase letters, digits and hyphens\n",
        )
            .into_response();
    }
    let ws = match super::chat::attachment_workspace(&state, &body.key, false).await {
        Ok(ws) => ws,
        Err(response) => return *response,
    };
    let path = body.path.clone();
    enum Staged {
        Ready(tempdir::Dir, Option<u64>),
        TooLarge,
        Missing,
        /// Our own scratch space failed — not the owner's picture.
        Scratch,
    }
    let staged = tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let read = || -> std::io::Result<Option<(Vec<u8>, Option<u64>)>> {
            let (mut file, _) =
                mecha_core::workspace_files::WorkspaceFiles::open(&ws)?.read(&path)?;
            let mut bytes = Vec::new();
            file.by_ref()
                .take(imagelib::MAX_PORTRAIT_BYTES + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() as u64 > imagelib::MAX_PORTRAIT_BYTES {
                return Ok(None);
            }
            Ok(Some((
                bytes,
                read_manifest(&ws, &path).and_then(|m| m["seed"].as_u64()),
            )))
        };
        match read() {
            Ok(Some((bytes, seed))) => match tempdir::Dir::new()
                .and_then(|d| std::fs::write(d.file(), &bytes).map(|_| d))
            {
                Ok(dir) => Staged::Ready(dir, seed),
                Err(_) => Staged::Scratch,
            },
            Ok(None) => Staged::TooLarge,
            Err(_) => Staged::Missing,
        }
    })
    .await;
    let (staged, seed) = match staged {
        Ok(Staged::Ready(dir, seed)) => (dir, seed),
        Ok(Staged::TooLarge) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                "the picture is over the portrait cap\n",
            )
                .into_response()
        }
        Ok(Staged::Missing) => return (StatusCode::NOT_FOUND, "no such picture\n").into_response(),
        // Our scratch space, or the blocking task, failed: something went
        // wrong, which is not the same finding as "no such picture".
        Ok(Staged::Scratch) | Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not stage the picture for saving\n",
            )
                .into_response()
        }
    };
    let portrait = staged.file().to_string_lossy().into_owned();
    let seed = seed.map(|s| s.to_string());
    // `--description=…` rather than two arguments: the owner types this, and
    // a description that opens with a dash is a description, not a flag
    // (`settings.rs`'s `reason_arg` records the same lesson; review of #385).
    let description = format!("--description={}", body.description);
    let mut args = vec![
        "imagelib",
        "add-character",
        &body.name,
        "--portrait",
        &portrait,
        &description,
    ];
    if let Some(seed) = &seed {
        args.extend(["--seed", seed]);
    }
    if body.locked {
        args.push("--locked");
    }
    let response = super::review::verb(&state, &args).await;
    drop(staged);
    response
}

/// A private scratch directory for one staged portrait, removed on drop.
mod tempdir {
    use std::path::{Path, PathBuf};

    pub struct Dir(PathBuf);

    impl Dir {
        pub fn new() -> std::io::Result<Dir> {
            let dir = std::env::temp_dir().join(format!("mecha-libsave-{}", uuid::Uuid::new_v4()));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(&dir)?;
            Ok(Dir(dir))
        }

        pub fn file(&self) -> PathBuf {
            self.0.join("portrait")
        }

        #[allow(dead_code)]
        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
pub(super) fn state_for_tests(dir: PathBuf) -> std::sync::Arc<LibraryState> {
    std::sync::Arc::new(LibraryState::new(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_store_blob_name_is_a_blob() {
        let hex = "a".repeat(64);
        assert!(valid_blob(&format!("sha256-{hex}.png")));
        for bad in [
            format!("sha256-{hex}.svg"),
            format!("sha256-{}.png", "a".repeat(63)),
            format!("sha256-{}.png", "A".repeat(64)),
            "../lock.toml".to_string(),
            format!("sha256-{hex}.png/../x"),
        ] {
            assert!(!valid_blob(&bad), "{bad} accepted");
        }
    }

    #[test]
    fn an_unlock_expires_and_can_be_revoked() {
        let s = LibraryState::new(PathBuf::new());
        assert!(!s.unlocked(None));
        assert!(!s.unlocked(Some("guess")));
        let t = s.grant();
        assert!(s.unlocked(Some(&t)));
        s.revoke(&t);
        assert!(!s.unlocked(Some(&t)));
        // An expired token is gone, not merely refused.
        let t = s.grant();
        s.unlocks
            .lock()
            .unwrap()
            .insert(t.clone(), Instant::now() - Duration::from_secs(1));
        assert!(!s.unlocked(Some(&t)));
        assert!(s.unlocks.lock().unwrap().is_empty());
    }

    #[test]
    fn wrong_passwords_are_limited() {
        let s = LibraryState::new(PathBuf::new());
        // Reservations count before any answer lands: parallel guesses run
        // out as sequential ones do.
        for _ in 0..MAX_FAILURES {
            assert!(s.may_try());
        }
        assert!(!s.may_try());
        // A success gives its reservation back.
        let s = LibraryState::new(PathBuf::new());
        assert!(s.may_try());
        s.succeeded();
        for _ in 0..MAX_FAILURES {
            assert!(s.may_try());
        }
        assert!(!s.may_try());
    }
}

/// The routes, driven through the real router with the owner's headers.
#[cfg(test)]
mod route_tests {
    use super::super::{review, router, WebState, TAILSCALE_LOGIN};
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use mecha_core::imagelib::{NewEntry, Origin};
    use std::sync::Arc;
    use tower::util::ServiceExt;

    struct Fixture {
        dir: PathBuf,
        app: axum::Router,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn png(shade: u8) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([shade, 10, 10]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    /// maya open, theo locked, sam a candidate; a lock password set.
    fn fixture() -> Fixture {
        fixture_with(None)
    }

    fn fixture_with(chat: Option<Arc<super::super::chat::ChatState>>) -> Fixture {
        let dir =
            std::env::temp_dir().join(format!("mecha-library-routes-{}", uuid::Uuid::new_v4()));
        for (name, shade, origin, locked) in [
            ("maya", 10, Origin::Owner, false),
            ("theo", 20, Origin::Owner, true),
            ("sam", 30, Origin::ModelUntrusted, false),
        ] {
            imagelib::create(
                &dir,
                NewEntry {
                    kind: Kind::Character,
                    name: name.into(),
                    text: format!("{name}, a memorable face"),
                    portrait: Some(png(shade)),
                    source_seed: None,
                    origin,
                    locked,
                },
            )
            .unwrap();
        }
        imagelib::set_lock_password(&dir, "correct horse").unwrap();
        let app = router(
            WebState {
                owner_login: Arc::new("owner@example.com".into()),
                chat,
                offer_target: None,
                voices_dir: None,
                library: Arc::new(LibraryState::new(dir.clone())),
                review: Arc::new(review::ReviewState {
                    outbox_root: std::env::temp_dir().join("mecha-serve-test-outbox"),
                    sessions_dir: None,
                }),
            },
            None,
        );
        Fixture { dir, app }
    }

    fn get(uri: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .header(TAILSCALE_LOGIN, "owner@example.com")
            .body(Body::empty())
            .unwrap()
    }

    fn post(uri: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(TAILSCALE_LOGIN, "owner@example.com")
            .header("x-mecha-request", "1")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    async fn json(response: Response) -> serde_json::Value {
        serde_json::from_slice(&to_bytes(response.into_body(), 1_000_000).await.unwrap()).unwrap()
    }

    fn names(list: &serde_json::Value) -> Vec<String> {
        list["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_string())
            .collect()
    }

    #[tokio::test]
    async fn a_locked_entry_is_absent_until_unlocked_and_never_cached() {
        let f = fixture();
        let r = f.app.clone().oneshot(get("/api/library")).await.unwrap();
        assert_eq!(r.headers()[header::CACHE_CONTROL], "no-store");
        let list = json(r).await;
        assert_eq!(names(&list), ["maya", "sam"]);
        assert_eq!(list["hidden_locked"], 1);
        assert_eq!(list["has_password"], true);

        let wrong = f
            .app
            .clone()
            .oneshot(post(
                "/api/library/unlock",
                serde_json::json!({"password": "nope"}),
            ))
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::FORBIDDEN);
        let r = f
            .app
            .clone()
            .oneshot(post(
                "/api/library/unlock",
                serde_json::json!({"password": "correct horse"}),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let token = json(r).await["token"].as_str().unwrap().to_string();

        let list = json(
            f.app
                .clone()
                .oneshot(get(&format!("/api/library?unlock={token}")))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(names(&list), ["maya", "sam", "theo"]);
        let theo = &list["entries"].as_array().unwrap()[2];
        let url = theo["portrait"].as_str().unwrap().to_string();
        assert!(url.contains(&format!("unlock={token}")), "{url}");

        // The portrait follows the entry: 404 without the token, never cached with it.
        let bare = url.split_once('?').unwrap().0;
        let r = f.app.clone().oneshot(get(bare)).await.unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        let r = f.app.clone().oneshot(get(&url)).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(r.headers()[header::CONTENT_TYPE], "image/png");

        // An open portrait is content-addressed, so it may be kept.
        let maya = &list["entries"].as_array().unwrap()[0];
        let r = f
            .app
            .clone()
            .oneshot(get(maya["portrait"].as_str().unwrap()))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert!(r.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("immutable"));

        // Relocking ends it.
        f.app
            .clone()
            .oneshot(post(
                "/api/library/relock",
                serde_json::json!({"token": token}),
            ))
            .await
            .unwrap();
        let r = f.app.clone().oneshot(get(&url)).await.unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn wrong_passwords_run_out() {
        let f = fixture();
        for _ in 0..MAX_FAILURES {
            let r = f
                .app
                .clone()
                .oneshot(post(
                    "/api/library/unlock",
                    serde_json::json!({"password": "x"}),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::FORBIDDEN);
        }
        // Even the right one waits now.
        let r = f
            .app
            .clone()
            .oneshot(post(
                "/api/library/unlock",
                serde_json::json!({"password": "correct horse"}),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn an_approval_must_name_what_was_shown() {
        let f = fixture();
        let r = f
            .app
            .clone()
            .oneshot(post(
                "/api/library/character/sam/approve",
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
        for (uri, want) in [
            ("/api/library/planet/sam/approve", StatusCode::BAD_REQUEST),
            (
                "/api/library/character/Sam!/approve",
                StatusCode::BAD_REQUEST,
            ),
            ("/api/library/character/sam/detonate", StatusCode::NOT_FOUND),
        ] {
            let r = f
                .app
                .clone()
                .oneshot(post(uri, serde_json::json!({})))
                .await
                .unwrap();
            assert_eq!(r.status(), want, "{uri}");
        }
    }

    #[tokio::test]
    async fn a_locked_entry_is_acted_on_only_while_unlocked_and_hides_as_missing_does() {
        let f = fixture();
        let theo = f
            .app
            .clone()
            .oneshot(post(
                "/api/library/character/theo/unlock",
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(theo.status(), StatusCode::NOT_FOUND);
        let theo = to_bytes(theo.into_body(), 1000).await.unwrap();
        let ghost = f
            .app
            .clone()
            .oneshot(post(
                "/api/library/character/ghost/unlock",
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(ghost.status(), StatusCode::NOT_FOUND);
        let ghost = to_bytes(ghost.into_body(), 1000).await.unwrap();
        // Hidden and missing answer alike: no oracle for which names exist.
        assert_eq!(theo, ghost);
        // Nothing changed on disk.
        assert!(
            Library::load(&f.dir)
                .0
                .get(Kind::Character, "theo")
                .unwrap()
                .locked
        );
    }

    #[tokio::test]
    async fn approval_takes_this_servers_signature_not_a_computable_digest() {
        let f = fixture();
        let list = json(f.app.clone().oneshot(get("/api/library")).await.unwrap()).await;
        let sam = list["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["name"] == "sam")
            .unwrap()
            .clone();
        let signed = sam["shown"].as_str().unwrap().to_string();
        // Anyone who can read the store can compute the digest; it is not
        // what the page was given, and it approves nothing.
        let digest = {
            let (lib, _) = Library::load(&f.dir);
            imagelib::shown_digest(lib.get(Kind::Character, "sam").unwrap())
        };
        assert_ne!(digest, signed);
        let r = f
            .app
            .clone()
            .oneshot(post(
                "/api/library/character/sam/approve",
                serde_json::json!({"shown": digest}),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::CONFLICT);
        assert_eq!(
            Library::load(&f.dir)
                .0
                .get(Kind::Character, "sam")
                .unwrap()
                .status,
            imagelib::Status::Candidate
        );
        // What the page was shown approves it, here, with no CLI child.
        let r = f
            .app
            .clone()
            .oneshot(post(
                "/api/library/character/sam/approve",
                serde_json::json!({"shown": signed}),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(
            Library::load(&f.dir)
                .0
                .get(Kind::Character, "sam")
                .unwrap()
                .status,
            imagelib::Status::Approved
        );
    }

    #[tokio::test]
    async fn save_reads_only_inside_the_chats_jail() {
        let f = fixture_with(Some(super::super::chat::test_chat()));
        let up = f
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/chat/libtest/upload?name=face.png")
                    .header(TAILSCALE_LOGIN, "owner@example.com")
                    .header("x-mecha-request", "1")
                    .body(Body::from(png(40)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(up.status(), StatusCode::OK);
        for path in ["../../etc/passwd", "/etc/passwd", "inbox/nothing-here.png"] {
            let r = f
                .app
                .clone()
                .oneshot(post(
                    "/api/library/save",
                    serde_json::json!({"key": "libtest", "path": path, "name": "x",
                                       "description": "y"}),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::NOT_FOUND, "{path}");
        }
        assert!(Library::load(&f.dir).0.get(Kind::Character, "x").is_none());
    }

    #[tokio::test]
    async fn without_a_password_the_lock_is_a_plain_toggle() {
        let f = fixture();
        std::fs::remove_file(f.dir.join("lock.toml")).unwrap();
        let list = json(f.app.clone().oneshot(get("/api/library")).await.unwrap()).await;
        assert_eq!(list["has_password"], false);
        assert_eq!(list["hidden_locked"], 1, "still hidden until the toggle");
        let r = f
            .app
            .clone()
            .oneshot(post("/api/library/unlock", serde_json::json!({})))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let token = json(r).await["token"].as_str().unwrap().to_string();
        let list = json(
            f.app
                .clone()
                .oneshot(get(&format!("/api/library?unlock={token}")))
                .await
                .unwrap(),
        )
        .await;
        assert!(names(&list).contains(&"theo".to_string()));
    }

    #[tokio::test]
    async fn a_damaged_lock_file_never_opens_as_if_absent() {
        let f = fixture();
        std::fs::write(f.dir.join("lock.toml"), "hash = \"nonsense\"").unwrap();
        let r = f
            .app
            .clone()
            .oneshot(post("/api/library/unlock", serde_json::json!({})))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn a_source_says_whether_not_who() {
        let f = fixture_with(Some(super::super::chat::test_chat()));
        let up = f
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/chat/srctest/upload?name=face.png")
                    .header(TAILSCALE_LOGIN, "owner@example.com")
                    .header("x-mecha-request", "1")
                    .body(Body::from(png(50)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(up.status(), StatusCode::OK);
        let r = f
            .app
            .clone()
            .oneshot(get("/api/library/source?key=srctest&path=inbox/face.png"))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let v = json(r).await;
        assert_eq!(v["suggest_locked"], false);
        assert_eq!(v["has_password"], true);
        // No names of any kind: a cast can hold a locked character's.
        assert!(
            v.get("cast").is_none() && v.get("locked_cast").is_none(),
            "{v}"
        );
    }

    #[tokio::test]
    async fn an_incognito_chat_cannot_save_or_look_up_a_source() {
        let f = fixture();
        let r = f
            .app
            .clone()
            .oneshot(post(
                "/api/library/save",
                serde_json::json!({"key": "incognito-0123456789abcdef012345", "path": "images/a.png",
                                   "name": "x", "description": "y"}),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        let r = f
            .app
            .clone()
            .oneshot(get(
                "/api/library/source?key=incognito-0123456789abcdef012345&path=images/a.png",
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn a_request_without_the_owner_is_refused() {
        let f = fixture();
        let r = f
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/library")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
    }

    /// Every refusal `add` and `edit` make before a child runs — the writes
    /// themselves are driven against a real `mecha serve` in
    /// `tests/image_library_web.rs`, since here the child would be this test.
    #[tokio::test]
    async fn add_and_edit_refuse_before_any_child_runs() {
        use base64::Engine;
        let f = fixture();
        let b64 = base64::engine::general_purpose::STANDARD.encode(png(40));
        let status = |uri: &'static str, body: serde_json::Value| {
            let app = f.app.clone();
            async move { app.oneshot(post(uri, body)).await.unwrap().status() }
        };
        let add = "/api/library/add";
        let edit = "/api/library/edit";
        for (body, want) in [
            // A character needs a portrait; a style has none.
            (
                serde_json::json!({"kind": "character", "name": "ada", "text": "x"}),
                StatusCode::BAD_REQUEST,
            ),
            (
                serde_json::json!({"kind": "style", "name": "ink", "text": "x", "portrait": b64}),
                StatusCode::BAD_REQUEST,
            ),
            (
                serde_json::json!({"kind": "character", "name": "ada", "text": "x", "portrait": "%%%"}),
                StatusCode::BAD_REQUEST,
            ),
            (
                serde_json::json!({"kind": "character", "name": "Ada", "text": "x", "portrait": b64}),
                StatusCode::BAD_REQUEST,
            ),
            (
                serde_json::json!({"kind": "person", "name": "ada", "text": "x"}),
                StatusCode::BAD_REQUEST,
            ),
        ] {
            assert_eq!(status(add, body.clone()).await, want, "{body}");
        }
        for (body, want) in [
            (
                serde_json::json!({"kind": "character", "name": "maya"}),
                StatusCode::BAD_REQUEST,
            ),
            (
                serde_json::json!({"kind": "style", "name": "maya", "portrait": b64}),
                StatusCode::BAD_REQUEST,
            ),
            // Locked and no token: hidden, and hidden answers as missing.
            (
                serde_json::json!({"kind": "character", "name": "theo", "text": "new"}),
                StatusCode::NOT_FOUND,
            ),
            (
                serde_json::json!({"kind": "character", "name": "nobody", "text": "new"}),
                StatusCode::NOT_FOUND,
            ),
            // A candidate is decided as written, not edited into approval.
            (
                serde_json::json!({"kind": "character", "name": "sam", "text": "new"}),
                StatusCode::CONFLICT,
            ),
        ] {
            assert_eq!(status(edit, body.clone()).await, want, "{body}");
        }
        // Nothing reached the store: sam is still the model's words.
        let (lib, _) = Library::load(&f.dir);
        let sam = lib.get(Kind::Character, "sam").unwrap();
        assert_eq!(sam.status, imagelib::Status::Candidate);
        assert_eq!(sam.text, "sam, a memorable face");
    }

    /// A portrait at the store's cap fits the body limit; axum's 2 MB default
    /// would have answered 413 before the handler's own cap could speak.
    #[tokio::test]
    async fn a_large_portrait_reaches_the_handler() {
        let f = fixture();
        // 3 MB of base64 that is not valid base64 past the limit check: the
        // handler, not the body limit, must be what refuses it.
        let body = serde_json::json!({
            "kind": "character", "name": "ada", "text": "x",
            "portrait": "%".repeat(3 * 1024 * 1024),
        });
        let r = f
            .app
            .clone()
            .oneshot(post("/api/library/add", body))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
        let text =
            String::from_utf8(to_bytes(r.into_body(), 10_000).await.unwrap().to_vec()).unwrap();
        assert!(text.contains("not base64"), "{text}");
    }
}
