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
//! this process's memory only and expires after the owner's autolock
//! (`imagelib::autolock_minutes`, 15 minutes unless set) without use — the
//! same span the page counts down from the last touch, so a page left open
//! locks itself and a page closed leaves a token that lapses on its own.
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

/// Wrong passwords allowed per window before the door answers 429.
const MAX_FAILURES: usize = 5;
const FAILURE_WINDOW: Duration = Duration::from_secs(5 * 60);

/// The library's directory and the unlocks this process has granted.
pub struct LibraryState {
    pub dir: PathBuf,
    /// Each live token: when it lapses, and the idle span that refreshes it
    /// — the autolock as it stood at the unlock.
    unlocks: Mutex<HashMap<String, (Instant, Duration)>>,
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
    pub(super) fn sign(&self, digest: &str) -> String {
        imagelib::sign_shown(&self.key, digest)
    }

    /// Whether `presented` is this process's signature of `digest`, compared
    /// without an early exit.
    pub(super) fn signed(&self, digest: &str, presented: &str) -> bool {
        let want = self.sign(digest);
        want.len() == presented.len()
            && want
                .bytes()
                .zip(presented.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
    }

    /// Whether `token` is a live unlock, refreshing its idle clock if so.
    pub(super) fn unlocked(&self, token: Option<&str>) -> bool {
        let Some(token) = token.filter(|t| !t.is_empty()) else {
            return false;
        };
        let mut unlocks = self.unlocks.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        unlocks.retain(|_, (expires, _)| *expires > now);
        match unlocks.get_mut(token) {
            Some((expires, idle)) => {
                *expires = now + *idle;
                true
            }
            None => false,
        }
    }

    /// An unlock token, as `POST /api/library/unlock` grants one — for
    /// the persona door's tests, which need the lock both ways.
    #[cfg(test)]
    pub(super) fn grant_for_tests(&self) -> String {
        self.grant(Duration::from_secs(
            u64::from(imagelib::DEFAULT_AUTOLOCK_MINUTES) * 60,
        ))
    }

    fn grant(&self, idle: Duration) -> String {
        let token = uuid::Uuid::new_v4().simple().to_string();
        self.unlocks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(token.clone(), (Instant::now() + idle, idle));
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

/// The picture route's query: the unlock token, and `size=thumb` for the
/// small JPEG a grid or an avatar shows ([`Library::thumbnail_named`]).
#[derive(Deserialize, Default)]
pub struct PictureQuery {
    #[serde(default)]
    unlock: Option<String>,
    #[serde(default)]
    size: Option<String>,
}

pub(super) fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

fn kind_label(kind: Kind) -> &'static str {
    kind.label()
}

pub(super) fn portrait_url(entry: &Entry, token: Option<&str>) -> Option<String> {
    Some(picture_url(entry, entry.portrait.as_deref()?, token))
}

/// Any picture an entry names, through the one route that checks the lock.
fn picture_url(entry: &Entry, blob: &str, token: Option<&str>) -> String {
    match (entry.locked, token) {
        (true, Some(t)) => format!("/api/library/portrait/{blob}?unlock={t}"),
        _ => format!("/api/library/portrait/{blob}"),
    }
}

/// An entry's typed pictures for the page (`IMAGE-COMPILER-DESIGN.md` §9.5):
/// kind, label, picture, and whether a generator may be sent it — the same
/// identity test `Library::read_reference` makes, so the page never says
/// "sent" of a copy the read refuses.
fn references_json(entry: &Entry, token: Option<&str>) -> Vec<serde_json::Value> {
    let sendable = entry.sendable_references();
    entry
        .references
        .iter()
        .map(|r| {
            serde_json::json!({
                "kind": r.kind.label(),
                "label": r.label,
                "picture": picture_url(entry, &r.blob, token),
                "sendable": sendable.iter().any(|s| std::ptr::eq(*s, r)),
                // Attached by a model from a clean conversation (the owner's
                // ruling of 2026-10-14); absent is the owner's own.
                "from_chat": r.origin.is_some(),
            })
        })
        .collect()
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
        let (pending, pending_errors) = imagelib::pending_references(&dir);
        (
            lib,
            errors,
            imagelib::has_lock_password(&dir),
            pending,
            pending_errors.len(),
        )
    })
    .await;
    let Ok((lib, errors, has_password, pending, pending_unreadable)) = loaded else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "reading the library\n").into_response();
    };
    // No count of what is hidden goes out: "one hidden" tells whoever holds
    // the phone there is something to find, which is what the lock is for.
    let entries: Vec<serde_json::Value> = lib
        .all()
        .iter()
        .filter(|e| !e.locked || unlocked)
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
                "references": references_json(e, token.as_deref()),
                "shown": lib_state.sign(&imagelib::shown_digest(e)),
            })
        })
        .collect();
    // A model's staged reference changes (§9.3, 2026-10-14), hidden as an
    // entry is: one staged locked (incognito), or for a locked character,
    // only while unlocked. Its picture rides the one image route, which
    // serves a pending picture only as `pending_picture` below allows.
    let pending: Vec<serde_json::Value> = pending
        .iter()
        .filter(|p| {
            let character_locked = lib.get(Kind::Character, &p.name).is_none_or(|e| e.locked);
            (!p.locked && !character_locked) || unlocked
        })
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "name": p.name,
                "kind": p.kind.label(),
                "action": p.action,
                "label": p.label,
                "origin": p.origin,
                "locked": p.locked,
                "created": p.created,
                "picture": p.blob.as_deref().map(|b| pending_url(&p.id, b, token.as_deref())),
            })
        })
        .collect();
    no_store(
        Json(serde_json::json!({
            "entries": entries,
            "pending_references": pending,
            "unlocked": unlocked,
            "has_password": has_password,
            "unreadable": errors.len(),
            // Said apart, since `mecha imagelib pending` is what shows them.
            "pending_unreadable": pending_unreadable,
        }))
        .into_response(),
    )
}

fn pending_url(id: &str, blob: &str, token: Option<&str>) -> String {
    match token {
        Some(t) => format!("/api/library/pending/{id}/{blob}?unlock={t}"),
        None => format!("/api/library/pending/{id}/{blob}"),
    }
}

/// GET /api/library/pending/{id}/{blob} — a staged reference's picture, so
/// the owner sees what they approve. Only the blob that pending change names,
/// hidden as the listing hides it, and never cached: it may be rejected.
pub async fn pending_picture(
    State(state): St,
    UrlPath((id, blob)): UrlPath<(String, String)>,
    Query(q): Query<UnlockQuery>,
) -> Response {
    if !valid_blob(&blob) || id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return (StatusCode::NOT_FOUND, "no such picture\n").into_response();
    }
    let unlocked = state.library.unlocked(q.unlock.as_deref());
    let dir = state.library.dir.clone();
    let read = tokio::task::spawn_blocking(move || {
        let (pending, _) = imagelib::pending_references(&dir);
        let p = pending.into_iter().find(|p| p.id == id)?;
        if p.blob.as_deref() != Some(blob.as_str()) {
            return None;
        }
        let lib = Library::load(&dir).0;
        let character_locked = lib.get(Kind::Character, &p.name).is_none_or(|e| e.locked);
        if (p.locked || character_locked) && !unlocked {
            return None;
        }
        let bytes = std::fs::read(lib.blob_path(&blob)).ok()?;
        let ext = imagelib::picture_type(&bytes).ok()?;
        (imagelib::blob_name_of(&bytes, ext) == blob).then_some((bytes, ext))
    })
    .await;
    let Ok(Some((bytes, ext))) = read else {
        return (StatusCode::NOT_FOUND, "no such picture\n").into_response();
    };
    let content_type = match ext {
        "jpg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "image/png",
    };
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response()
}

#[derive(Deserialize, Default)]
pub struct PendingBody {
    #[serde(default)]
    unlock: Option<String>,
}

/// POST /api/library/pending/{id}/{approve|reject} — the owner rules on a
/// staged reference change: `mecha imagelib approve-reference` /
/// `reject-reference`. Hidden answers as missing, as for an entry.
pub async fn act_pending(
    State(state): St,
    UrlPath((id, action)): UrlPath<(String, String)>,
    body: Option<Json<PendingBody>>,
) -> Response {
    let verb = match action.as_str() {
        "approve" => "approve-reference",
        "reject" => "reject-reference",
        _ => return (StatusCode::NOT_FOUND, "no such action\n").into_response(),
    };
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return (StatusCode::BAD_REQUEST, "not a pending id\n").into_response();
    }
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let unlocked = state.library.unlocked(body.unlock.as_deref());
    let dir = state.library.dir.clone();
    let want = id.clone();
    let visible = tokio::task::spawn_blocking(move || {
        let (pending, _) = imagelib::pending_references(&dir);
        let p = pending.into_iter().find(|p| p.id == want)?;
        let lib = Library::load(&dir).0;
        let character_locked = lib.get(Kind::Character, &p.name).is_none_or(|e| e.locked);
        Some(!(p.locked || character_locked) || unlocked)
    })
    .await;
    if visible.ok().flatten() != Some(true) {
        return (StatusCode::NOT_FOUND, "no such pending change\n").into_response();
    }
    super::review::verb(&state, &["imagelib", verb, &id]).await
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

/// GET /api/library/portrait/{blob} — a portrait or a typed reference, only
/// as some entry the owner may see now names it; `?size=thumb` answers it as
/// a small JPEG, under the same lookup and cache rule. Everything else, including a
/// blob only a superseded version names, is the same 404. Both halves of
/// #385's answer cover every picture an entry names, not only its portrait
/// (`IMAGE-COMPILER-DESIGN.md` §9.5): the lookup that answers 404, and the
/// cache fold below, or a locked entry's `full_body` would be cached for a
/// year.
pub async fn portrait(
    State(state): St,
    UrlPath(blob): UrlPath<String>,
    Query(q): Query<PictureQuery>,
) -> Response {
    if !valid_blob(&blob) {
        return (StatusCode::NOT_FOUND, "no such portrait\n").into_response();
    }
    let thumb = match q.size.as_deref() {
        None => false,
        Some("thumb") => true,
        Some(_) => return (StatusCode::BAD_REQUEST, "size is thumb or absent\n").into_response(),
    };
    let unlocked = state.library.unlocked(q.unlock.as_deref());
    let dir = state.library.dir.clone();
    let read = tokio::task::spawn_blocking(move || {
        let (lib, _) = Library::load(&dir);
        let naming: Vec<&Entry> = lib
            .all()
            .iter()
            .filter(|e| {
                e.portrait.as_deref() == Some(blob.as_str())
                    || e.references.iter().any(|r| r.blob == blob)
            })
            .collect();
        let open = naming.iter().any(|e| !e.locked);
        let entry = naming.into_iter().find(|e| !e.locked || unlocked)?;
        // After the lock's lookup, as the full picture is: a thumbnail is
        // the same face.
        if thumb {
            // One that cannot be made shows the picture itself rather than
            // a broken tile (review of #735).
            match lib.thumbnail_named(entry, &blob) {
                Ok(jpeg) => return Some((jpeg, "jpg", open)),
                Err(e) => tracing::warn!("thumbnail of {blob}: {e:#}"),
            }
        }
        let (bytes, ext) = lib.read_named(entry, &blob).ok()?;
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
    // The autolock first: a damaged setting is a finding, never an unlock
    // that lasts longer than the owner chose.
    let dir = state.library.dir.clone();
    let idle = match tokio::task::spawn_blocking(move || imagelib::autolock_minutes(&dir)).await {
        Ok(Ok(minutes)) => Duration::from_secs(u64::from(minutes) * 60),
        Ok(Err(e)) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response();
        }
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "reading the autolock\n").into_response();
        }
    };
    let granted = |state: &super::WebState| {
        no_store(
            Json(serde_json::json!({
                "token": state.library.grant(idle),
                "idle_secs": idle.as_secs(),
            }))
            .into_response(),
        )
    };
    // No password set: the lock is a plain show/hide toggle (the owner's
    // ruling, 2026-09-28 — the password is optional). Checked by the file's
    // presence, so a damaged lock file still goes through verification below
    // and errors, never opens.
    let dir = state.library.dir.clone();
    let has = tokio::task::spawn_blocking(move || imagelib::has_lock_password(&dir))
        .await
        .unwrap_or(true);
    if !has {
        return granted(&state);
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
        Ok(Ok(true)) => granted(&state),
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

/// GET /api/settings/lock — the autolock, and whether a password is set.
/// Neither says anything about what the lock hides.
pub async fn lock_settings(State(state): St) -> Response {
    let dir = state.library.dir.clone();
    let read = tokio::task::spawn_blocking(move || {
        (
            imagelib::autolock_minutes(&dir),
            imagelib::has_lock_password(&dir),
        )
    })
    .await;
    let Ok((minutes, has_password)) = read else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "reading the lock settings\n",
        )
            .into_response();
    };
    Json(serde_json::json!({
        // Unknown is said as unknown: a damaged setting is no number at all,
        // never the default standing in for it.
        "idle_minutes": minutes.as_ref().ok(),
        "error": minutes.as_ref().err().map(|e| format!("{e:#}")),
        "default_minutes": imagelib::DEFAULT_AUTOLOCK_MINUTES,
        "max_minutes": imagelib::MAX_AUTOLOCK_MINUTES,
        "has_password": has_password,
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct LockSettingsBody {
    idle_minutes: u32,
}

/// POST /api/settings/lock — set the autolock, as `mecha imagelib
/// set-autolock` does (it is that child). Needs no unlock: a longer span
/// opens nothing, and only an unlock — password and all — starts one.
pub async fn set_lock_settings(State(state): St, Json(body): Json<LockSettingsBody>) -> Response {
    // The child checks the range too; refused here first, so a bad value
    // never costs a process.
    if !(1..=imagelib::MAX_AUTOLOCK_MINUTES).contains(&body.idle_minutes) {
        return (
            StatusCode::BAD_REQUEST,
            format!(
                "the autolock is 1 to {} minutes\n",
                imagelib::MAX_AUTOLOCK_MINUTES
            ),
        )
            .into_response();
    }
    let minutes = body.idle_minutes.to_string();
    super::review::verb(&state, &["imagelib", "set-autolock", &minutes]).await
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
async fn stage_portrait(b64: &str, what: &str) -> Result<tempdir::Dir, Box<Response>> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|_| {
            Box::new(
                (
                    StatusCode::BAD_REQUEST,
                    format!("the {what} is not base64\n"),
                )
                    .into_response(),
            )
        })?;
    if bytes.is_empty() {
        return Err(Box::new(
            (StatusCode::BAD_REQUEST, format!("the {what} is empty\n")).into_response(),
        ));
    }
    if bytes.len() as u64 > imagelib::MAX_PORTRAIT_BYTES {
        return Err(Box::new(
            (
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("the {what} is over the picture cap\n"),
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
        ("character", Some(b64)) => match stage_portrait(b64, "portrait").await {
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
        Some(b64) => match stage_portrait(b64, "portrait").await {
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
pub struct ReferenceBody {
    name: String,
    /// `face`, `full_body` or `detail`.
    #[serde(rename = "as")]
    as_kind: String,
    /// The picture, base64; for adding only.
    #[serde(default)]
    picture: Option<String>,
    /// What it shows; required for a detail, and says which one to remove.
    #[serde(default)]
    label: Option<String>,
    /// The unlock token, for a locked character.
    #[serde(default)]
    unlock: Option<String>,
}

/// The kinds the owner attaches from the page: a closed set, spelled as the
/// CLI's `--as` takes them, so nothing else reaches the child.
fn parse_ref_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "face" => Some("face"),
        "full_body" => Some("full_body"),
        "detail" => Some("detail"),
        _ => None,
    }
}

/// What both reference writes check before a child runs: the kind, the
/// name, and the character as this request may see it, approved.
async fn reference_target(
    state: &super::WebState,
    body: &ReferenceBody,
) -> Result<&'static str, Box<Response>> {
    let Some(kind) = parse_ref_kind(&body.as_kind) else {
        return Err(Box::new(
            (
                StatusCode::BAD_REQUEST,
                "a reference is a face, full_body or detail\n",
            )
                .into_response(),
        ));
    };
    if imagelib::validate_name(&body.name).is_err() {
        return Err(Box::new(
            (StatusCode::BAD_REQUEST, "not an entry name\n").into_response(),
        ));
    }
    let Some(entry) = visible(state, "character", &body.name, body.unlock.as_deref()).await else {
        return Err(Box::new(
            (StatusCode::NOT_FOUND, "no such entry\n").into_response(),
        ));
    };
    if entry.status != imagelib::Status::Approved {
        return Err(Box::new(
            (
                StatusCode::CONFLICT,
                format!(
                    "`{}` is waiting for a decision — approve or reject it first\n",
                    entry.name
                ),
            )
                .into_response(),
        ));
    }
    Ok(kind)
}

/// POST /api/library/reference — attach a typed picture to an approved
/// character: `mecha imagelib add-reference`. The owner's write, as the edit
/// page's is; the child keeps the store's limits and label rules.
pub async fn add_reference(State(state): St, Json(body): Json<ReferenceBody>) -> Response {
    let kind = match reference_target(&state, &body).await {
        Ok(kind) => kind,
        Err(refusal) => return *refusal,
    };
    let Some(b64) = body.picture.as_deref() else {
        return (StatusCode::BAD_REQUEST, "a reference needs its picture\n").into_response();
    };
    let staged = match stage_portrait(b64, "picture").await {
        Ok(dir) => dir,
        Err(refusal) => return *refusal,
    };
    let path = staged.file().to_string_lossy().into_owned();
    // `--label=…`: a label that opens with a dash is a label (review of #385).
    let label = body.label.as_ref().map(|l| format!("--label={l}"));
    let mut args = vec!["imagelib", "add-reference", &body.name, "--as", kind, &path];
    if let Some(label) = &label {
        args.push(label);
    }
    let response = super::review::verb(&state, &args).await;
    drop(staged);
    response
}

/// POST /api/library/reference/remove — take one off: `mecha imagelib
/// remove-reference`, a new version that keeps the picture for the history.
pub async fn remove_reference(State(state): St, Json(body): Json<ReferenceBody>) -> Response {
    let kind = match reference_target(&state, &body).await {
        Ok(kind) => kind,
        Err(refusal) => return *refusal,
    };
    let label = body.label.as_ref().map(|l| format!("--label={l}"));
    let mut args = vec!["imagelib", "remove-reference", &body.name, "--as", kind];
    if let Some(label) = &label {
        args.push(label);
    }
    super::review::verb(&state, &args).await
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

/// Refuse a key that is not a session key at all.
fn refuse_bad_key(key: &str) -> Option<Response> {
    (!super::chat::valid_key(key))
        .then(|| (StatusCode::BAD_REQUEST, "bad session key\n").into_response())
}

/// Whether a save lands locked. An incognito chat's always does, whatever
/// the box said (`docs/INCOGNITO-DESIGN.md` R3, the owner's ruling of
/// 2026-10-12): the character is the owner's to keep, hidden behind the
/// library lock until unlocked, as `persona_propose` stages a persona from
/// the same room. The page shows the box checked and fixed; this is the rule.
fn save_locked(key: &str, asked: bool) -> bool {
    asked || super::incognito::is_incognito_key(key)
}

/// GET /api/library/source?key&path — what the save dialog starts from: the
/// seed that drew the picture, who was cast in it, and whether any of them
/// is locked — the lock box starts checked when one is (the owner's ruling:
/// inherited by default, overridable).
pub async fn source(State(state): St, Query(q): Query<SourceQuery>) -> Response {
    if let Some(refusal) = refuse_bad_key(&q.key) {
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
/// own act, so the entry is approved on creation — locked when it comes from
/// an incognito chat ([`save_locked`]). Every answer is `no-store` (`source`
/// marks its own answer the same way): the key rides in the body, so the
/// middleware's incognito path needle never sees it, and a reply names the
/// entry (INCOGNITO-DESIGN §8: `no-store` on every incognito route).
pub async fn save(state: St, body: Json<SaveBody>) -> Response {
    no_store(save_unmarked(state, body).await)
}

async fn save_unmarked(State(state): St, Json(body): Json<SaveBody>) -> Response {
    if let Some(refusal) = refuse_bad_key(&body.key) {
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
    let scratch = match scratch_parent(&body.key, &ws) {
        Ok(dir) => dir,
        Err(why) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{why}\n")).into_response(),
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
            Ok(Some((bytes, seed))) => match tempdir::Dir::new_in(&scratch)
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
    if save_locked(&body.key, body.locked) {
        args.push("--locked");
    }
    let response = super::review::verb(&state, &args).await;
    drop(staged);
    response
}

/// Where a save stages its copy of the picture for the child. An incognito
/// chat's goes in its room — beside the jail, never in it, and removed with
/// the room — and only while the room is RAM: the host temp directory is
/// commonly a disk (this machine's `/tmp` is ext4), where an unlinked copy
/// stays recoverable after the room is gone, and R3's exception covers the
/// library entry, not a scratch copy (review of #657, pass 3). A room that
/// is not tmpfs refuses the save rather than falling through to the disk,
/// as `incognito::images_forgettable` withholds `image_generate`. Any other
/// chat stages in the host temp directory, as before.
fn scratch_parent(key: &str, ws: &std::path::Path) -> Result<PathBuf, &'static str> {
    if !super::incognito::is_incognito_key(key) {
        return Ok(std::env::temp_dir());
    }
    let room = ws
        .parent()
        .ok_or("this incognito chat has no room to stage the picture in")?;
    match super::incognito::is_tmpfs(room) {
        Ok(true) => Ok(room.to_path_buf()),
        _ => Err("this incognito chat's room is not in RAM, so the picture was not staged"),
    }
}

/// A private scratch directory for one staged portrait, removed on drop.
mod tempdir {
    use std::path::{Path, PathBuf};

    pub struct Dir(PathBuf);

    impl Dir {
        pub fn new() -> std::io::Result<Dir> {
            Dir::new_in(&std::env::temp_dir())
        }

        /// One under `parent`, which must already exist.
        pub fn new_in(parent: &Path) -> std::io::Result<Dir> {
            let dir = parent.join(format!("mecha-libsave-{}", uuid::Uuid::new_v4()));
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
        let idle = Duration::from_secs(60);
        let t = s.grant(idle);
        assert!(s.unlocked(Some(&t)));
        s.revoke(&t);
        assert!(!s.unlocked(Some(&t)));
        // An expired token is gone, not merely refused.
        let t = s.grant(idle);
        s.unlocks
            .lock()
            .unwrap()
            .insert(t.clone(), (Instant::now() - Duration::from_secs(1), idle));
        assert!(!s.unlocked(Some(&t)));
        assert!(s.unlocks.lock().unwrap().is_empty());
    }

    /// A use refreshes a token by its own span — the autolock as it stood
    /// at its unlock — not by one span for every token.
    #[test]
    fn a_use_refreshes_a_token_by_its_own_span() {
        let s = LibraryState::new(PathBuf::new());
        let short = s.grant(Duration::from_secs(60));
        let long = s.grant(Duration::from_secs(3600));
        for t in [&short, &long] {
            let mut unlocks = s.unlocks.lock().unwrap();
            unlocks.get_mut(t).unwrap().0 = Instant::now() + Duration::from_secs(1);
        }
        assert!(s.unlocked(Some(&short)) && s.unlocked(Some(&long)));
        let left = |t: &str| s.unlocks.lock().unwrap()[t].0 - Instant::now();
        assert!(left(&short) <= Duration::from_secs(60) && left(&short) > Duration::from_secs(50));
        assert!(left(&long) > Duration::from_secs(3500));
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
                stt_url: Arc::new(mecha_core::config::VoiceConfig::DEFAULT_STT_URL.to_string()),
                library: Arc::new(LibraryState::new(dir.clone())),
                features_at_start: Arc::default(),
                gate: Arc::default(),
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
        assert!(
            list.get("hidden_locked").is_none(),
            "a count of hidden entries says there is something to find"
        );
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
        assert_eq!(
            names(&list),
            ["maya", "sam"],
            "still hidden until the toggle"
        );
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
    async fn an_unlock_lasts_the_owners_autolock_and_a_damaged_one_opens_nothing() {
        let f = fixture();
        std::fs::remove_file(f.dir.join("lock.toml")).unwrap();
        let unlock = || async {
            f.app
                .clone()
                .oneshot(post("/api/library/unlock", serde_json::json!({})))
                .await
                .unwrap()
        };
        let r = unlock().await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(json(r).await["idle_secs"], 15 * 60, "the default");
        imagelib::set_autolock_minutes(&f.dir, 5).unwrap();
        assert_eq!(json(unlock().await).await["idle_secs"], 5 * 60);
        let settings = json(
            f.app
                .clone()
                .oneshot(get("/api/settings/lock"))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(settings["idle_minutes"], 5);
        assert_eq!(settings["has_password"], false);
        // Damaged: no unlock at all, and the settings say unknown, not 15.
        std::fs::write(f.dir.join("autolock.toml"), "idle_minutes = 0\n").unwrap();
        assert_eq!(unlock().await.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let settings = json(
            f.app
                .clone()
                .oneshot(get("/api/settings/lock"))
                .await
                .unwrap(),
        )
        .await;
        assert!(settings["idle_minutes"].is_null() && settings["error"].is_string());
        // Out of range is refused before any child could run.
        for bad in [0, imagelib::MAX_AUTOLOCK_MINUTES + 1] {
            let r = f
                .app
                .clone()
                .oneshot(post(
                    "/api/settings/lock",
                    serde_json::json!({ "idle_minutes": bad }),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{bad}");
        }
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

    #[test]
    fn an_incognito_save_is_locked_whatever_the_box_says() {
        let incognito = "incognito-0123456789abcdef012345";
        assert!(save_locked(incognito, false));
        assert!(save_locked(incognito, true));
        assert!(!save_locked("main", false));
        assert!(save_locked("main", true));
    }

    #[test]
    fn an_incognito_save_never_stages_on_the_host_temp_dir() {
        let incognito = "incognito-0123456789abcdef012345";
        assert_eq!(
            scratch_parent("main", std::path::Path::new("/anywhere/main")),
            Ok(std::env::temp_dir())
        );
        // A room wherever this machine's temp directory is: staged in it
        // only when that is RAM, and refused — never the host temp
        // directory — when it is not.
        let room = std::env::temp_dir().join(format!("mecha-room-{}", uuid::Uuid::new_v4()));
        let ws = room.join(incognito);
        std::fs::create_dir_all(&ws).unwrap();
        let got = scratch_parent(incognito, &ws);
        match super::super::incognito::is_tmpfs(&room) {
            Ok(true) => assert_eq!(got, Ok(room.clone())),
            _ => assert!(got.is_err(), "{got:?}"),
        }
        assert_ne!(got, Ok(std::env::temp_dir()));
        std::fs::remove_dir_all(&room).ok();
        // And a room that is RAM stages in it, where one can be made.
        let shm = std::path::Path::new("/dev/shm");
        if super::super::incognito::is_tmpfs(shm).unwrap_or(false) {
            let room = shm.join(format!("mecha-room-{}", uuid::Uuid::new_v4()));
            let ws = room.join(incognito);
            std::fs::create_dir_all(&ws).unwrap();
            assert_eq!(scratch_parent(incognito, &ws), Ok(room.clone()));
            std::fs::remove_dir_all(&room).ok();
        }
    }

    #[tokio::test]
    async fn an_incognito_key_reaches_the_save_and_source_routes() {
        // No such chat is open, so each answers as any unknown chat does —
        // the point is that neither refuses the key for being incognito.
        let f = fixture_with(Some(super::super::chat::test_chat()));
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
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            r.headers()
                .get(header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("no-store"),
            "the key rides in the body, out of the middleware's sight"
        );
        let incognito_body = axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap();
        // The same answer an ordinary chat that is not open gets: unknown,
        // never refused for being incognito.
        let r = f
            .app
            .clone()
            .oneshot(post(
                "/api/library/save",
                serde_json::json!({"key": "notopen", "path": "images/a.png",
                                   "name": "x", "description": "y"}),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap(),
            incognito_body
        );
        let r = f
            .app
            .clone()
            .oneshot(get(
                "/api/library/source?key=incognito-0123456789abcdef012345&path=images/a.png",
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        let r = f
            .app
            .clone()
            .oneshot(get("/api/library/source?key=..%2Fx&path=images/a.png"))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
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

    /// A reference is served as its entry's portrait is: by the one route,
    /// only while its entry may be seen, and never cached when only a locked
    /// entry names it — both halves of #385's answer (review of #680, pass
    /// 30). The page lists it with whether a generator may be sent it.
    #[tokio::test]
    async fn a_reference_follows_its_entry_through_the_lock_and_the_cache() {
        let f = fixture();
        for (name, shade) in [("maya", 50), ("theo", 60)] {
            imagelib::add_reference(&f.dir, name, imagelib::RefKind::FullBody, &png(shade), None)
                .unwrap();
        }
        let list = json(f.app.clone().oneshot(get("/api/library")).await.unwrap()).await;
        let maya = &list["entries"].as_array().unwrap()[0];
        let refs = maya["references"].as_array().unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0]["kind"], "full_body");
        assert_eq!(refs[0]["sendable"], true);
        let open = refs[0]["picture"].as_str().unwrap().to_string();
        let r = f.app.clone().oneshot(get(&open)).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert!(r.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("immutable"));

        // theo is locked: his body is the same 404 until unlocked, and then
        // never cached.
        let theo = Library::load(&f.dir)
            .0
            .get(Kind::Character, "theo")
            .unwrap()
            .clone();
        let blob = &theo.references[0].blob;
        let bare = format!("/api/library/portrait/{blob}");
        let r = f.app.clone().oneshot(get(&bare)).await.unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        let token = f.app.clone();
        let r = token
            .oneshot(post(
                "/api/library/unlock",
                serde_json::json!({"password": "correct horse"}),
            ))
            .await
            .unwrap();
        let token = json(r).await["token"].as_str().unwrap().to_string();
        let list = json(
            f.app
                .clone()
                .oneshot(get(&format!("/api/library?unlock={token}")))
                .await
                .unwrap(),
        )
        .await;
        let theo = &list["entries"].as_array().unwrap()[2];
        let url = theo["references"][0]["picture"].as_str().unwrap();
        assert!(url.contains(&format!("unlock={token}")), "{url}");
        let r = f.app.clone().oneshot(get(url)).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()[header::CACHE_CONTROL], "no-store");
    }

    /// A thumbnail rides the picture route under the same lock and cache
    /// rules as the picture: a JPEG cached for good when its entry is open,
    /// the same 404 for a locked one until unlocked, and never cached then.
    #[tokio::test]
    async fn a_thumbnail_follows_its_picture_through_the_lock_and_the_cache() {
        let f = fixture();
        let list = json(f.app.clone().oneshot(get("/api/library")).await.unwrap()).await;
        let maya = list["entries"].as_array().unwrap()[0]["portrait"]
            .as_str()
            .unwrap()
            .to_string();
        let r = f
            .app
            .clone()
            .oneshot(get(&format!("{maya}?size=thumb")))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()[header::CONTENT_TYPE], "image/jpeg");
        assert!(r.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("immutable"));
        let r = f
            .app
            .clone()
            .oneshot(get(&format!("{maya}?size=huge")))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);

        let theo = Library::load(&f.dir)
            .0
            .get(Kind::Character, "theo")
            .unwrap()
            .clone();
        let blob = theo.portrait.as_deref().unwrap();
        let bare = format!("/api/library/portrait/{blob}?size=thumb");
        let r = f.app.clone().oneshot(get(&bare)).await.unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        let r = f
            .app
            .clone()
            .oneshot(post(
                "/api/library/unlock",
                serde_json::json!({"password": "correct horse"}),
            ))
            .await
            .unwrap();
        let token = json(r).await["token"].as_str().unwrap().to_string();
        let r = f
            .app
            .clone()
            .oneshot(get(&format!("{bare}&unlock={token}")))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()[header::CACHE_CONTROL], "no-store");
    }

    /// The reference writes refuse before any child runs: a kind outside the
    /// three, a bad name, a hidden or waiting character, no picture.
    #[tokio::test]
    async fn reference_writes_refuse_before_any_child_runs() {
        use base64::Engine;
        let f = fixture();
        let b64 = base64::engine::general_purpose::STANDARD.encode(png(70));
        let status = |uri: &'static str, body: serde_json::Value| {
            let app = f.app.clone();
            async move { app.oneshot(post(uri, body)).await.unwrap().status() }
        };
        let add = "/api/library/reference";
        let remove = "/api/library/reference/remove";
        for (uri, body, want) in [
            (
                add,
                serde_json::json!({"name": "maya", "as": "view", "picture": b64}),
                StatusCode::BAD_REQUEST,
            ),
            (
                add,
                serde_json::json!({"name": "Maya!", "as": "face", "picture": b64}),
                StatusCode::BAD_REQUEST,
            ),
            (
                add,
                serde_json::json!({"name": "maya", "as": "face"}),
                StatusCode::BAD_REQUEST,
            ),
            (
                add,
                serde_json::json!({"name": "maya", "as": "face", "picture": "%%%"}),
                StatusCode::BAD_REQUEST,
            ),
            // Locked and no token: hidden, and hidden answers as missing.
            (
                add,
                serde_json::json!({"name": "theo", "as": "face", "picture": b64}),
                StatusCode::NOT_FOUND,
            ),
            (
                add,
                serde_json::json!({"name": "nobody", "as": "face", "picture": b64}),
                StatusCode::NOT_FOUND,
            ),
            (
                remove,
                serde_json::json!({"name": "theo", "as": "face"}),
                StatusCode::NOT_FOUND,
            ),
            (
                remove,
                serde_json::json!({"name": "maya", "as": "portrait"}),
                StatusCode::BAD_REQUEST,
            ),
        ] {
            assert_eq!(status(uri, body.clone()).await, want, "{uri} {body}");
        }
        // A candidate takes none, in the handler's own words: a failed child
        // answers 409 too, so the status alone would pass without the guard
        // (review of #688).
        let r = f
            .app
            .clone()
            .oneshot(post(
                add,
                serde_json::json!({"name": "sam", "as": "face", "picture": b64}),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::CONFLICT);
        let text =
            String::from_utf8(to_bytes(r.into_body(), 10_000).await.unwrap().to_vec()).unwrap();
        assert!(text.contains("waiting for a decision"), "{text}");
        let lib = Library::load(&f.dir).0;
        assert!(lib.all().iter().all(|e| e.references.is_empty()));
    }

    /// The page's copies of the store's reference rules match the store:
    /// a drifted limit would hide an add the store allows, or offer one it
    /// refuses (review of #688).
    #[test]
    fn the_pages_reference_limits_are_the_stores() {
        use imagelib::RefKind;
        let js = include_str!("../../../../web/src/lib/library.js");
        let limits = format!(
            "export const REF_LIMIT = {{ face: {}, full_body: {}, detail: {} }};",
            RefKind::Face.limit(),
            RefKind::FullBody.limit(),
            RefKind::Detail.limit()
        );
        assert!(js.contains(&limits), "library.js lacks `{limits}`");
        let label = format!("export const LABEL_MAX = {};", imagelib::MAX_LABEL);
        assert!(js.contains(&label), "library.js lacks `{label}`");
    }

    /// A model's staged reference changes (2026-10-14): listed with a
    /// picture the owner sees before approving, hidden as their character
    /// is while locked, the picture uncached and only that change's own;
    /// a hidden change answers as missing, and approving one applies it.
    #[tokio::test]
    async fn staged_reference_changes_follow_the_lock() {
        let f = fixture();
        let stage = |name: &str, shade: u8| {
            imagelib::propose_reference(
                &f.dir,
                name,
                imagelib::RefKind::FullBody,
                imagelib::PendingAction::Add,
                Some(&png(shade)),
                None,
                imagelib::Origin::ModelUntrusted,
                false,
            )
            .unwrap()
        };
        let maya = stage("maya", 80);
        let theo = stage("theo", 90);

        let list = json(f.app.clone().oneshot(get("/api/library")).await.unwrap()).await;
        let pending = list["pending_references"].as_array().unwrap();
        assert_eq!(pending.len(), 1, "theo is locked: {list}");
        assert_eq!(pending[0]["id"], maya.id.as_str());
        let url = pending[0]["picture"].as_str().unwrap();
        let r = f.app.clone().oneshot(get(url)).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()[header::CACHE_CONTROL], "no-store");
        // Only the blob that change names, and not a locked one's.
        let other = format!(
            "/api/library/pending/{}/{}",
            maya.id,
            theo.blob.as_ref().unwrap()
        );
        assert_eq!(
            f.app.clone().oneshot(get(&other)).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        let hidden = format!(
            "/api/library/pending/{}/{}",
            theo.id,
            theo.blob.as_ref().unwrap()
        );
        assert_eq!(
            f.app.clone().oneshot(get(&hidden)).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        let r = f
            .app
            .clone()
            .oneshot(post(
                &format!("/api/library/pending/{}/approve", theo.id),
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        assert_eq!(
            r.status(),
            StatusCode::NOT_FOUND,
            "a hidden change answers as missing"
        );
        for bad in [
            "/api/library/pending/zz!/approve",
            "/api/library/pending/abc/detonate",
        ] {
            let r = f
                .app
                .clone()
                .oneshot(post(bad, serde_json::json!({})))
                .await
                .unwrap();
            assert!(
                r.status() == StatusCode::BAD_REQUEST || r.status() == StatusCode::NOT_FOUND,
                "{bad}"
            );
        }
        // Unlocked, theo's shows too.
        let r = f
            .app
            .clone()
            .oneshot(post(
                "/api/library/unlock",
                serde_json::json!({"password": "correct horse"}),
            ))
            .await
            .unwrap();
        let token = json(r).await["token"].as_str().unwrap().to_string();
        let list = json(
            f.app
                .clone()
                .oneshot(get(&format!("/api/library?unlock={token}")))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(list["pending_references"].as_array().unwrap().len(), 2);
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
