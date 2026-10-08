//! The image library: characters and styles a scene compiles against.
//!
//! `docs/IMAGE-COMPILER-DESIGN.md` is the contract and
//! `docs/IMAGE-COMPILER-RESEARCH.md` the evidence; the numbers cited here
//! (E1–E10) are that document's experiments.
//!
//! Two halves. The **store** is `~/.mecha/imagelib/`, global only, with one
//! directory per entry and pictures content-addressed under `blobs/`. The
//! **compiler** turns a scene the model wrote — who is in it, what each is
//! wearing and doing — into the prompt and references `image_generate` sends:
//! the model writes what varies, this code writes what persists, and a
//! character's description is pasted verbatim, never paraphrased.
//!
//! Who writes what: the owner creates approved entries (the CLI now, the web
//! surface next); the model can only stage a candidate, and a candidate never
//! reaches a prompt or the model until the owner approves it.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::path::{Path, PathBuf};

/// An entry name: `[a-z0-9][a-z0-9-]*`, at most this long.
pub const MAX_NAME: usize = 64;
/// A character's description. Short on purpose: it rides beside the
/// reference pointer (E1), and identity lives in the picture, not the words.
pub const MAX_DESCRIPTION: usize = 400;
/// A style's text, pasted verbatim at the end of every prompt that names it.
pub const MAX_STYLE_TEXT: usize = 1000;
/// People per new picture. E3 held four in one pass; five is the owner's
/// provisional ceiling (IMAGE-DESIGN.md §6, C5).
pub const MAX_CAST: usize = 5;
/// `wearing` and `doing`, each.
pub const MAX_CAST_FIELD: usize = 300;
/// People in a scene who are no library character — a waiter, a crowd's
/// front row. They carry no reference, so the four-reference ceiling (E3)
/// does not bound them; four is what has been asked of the model alongside
/// a cast, not a measured limit.
pub const MAX_EXTRAS: usize = 4;
/// Candidates waiting on the owner. A model that proposes in a loop fills a
/// queue nobody reads; past this it is refused.
pub const MAX_PENDING: usize = 50;
/// A portrait larger than this is refused rather than read.
pub const MAX_PORTRAIT_BYTES: u64 = 25 * 1024 * 1024;
/// A *proposed* portrait: kept on disk until the owner rules on it, from a
/// tool that needs no approval, so the pending cap has to bound bytes as well
/// as entries — 50 × 4 MB, not 50 × 25 MB (review of #383). A 1024² PNG from
/// the image server measured 1.85 MB on 2026-09-28, so 4 MB refuses nothing
/// ordinary; references go at [`REFERENCE_SIZE`] regardless.
pub const MAX_PROPOSED_PORTRAIT_BYTES: u64 = 4 * 1024 * 1024;
/// The size references are sent at, in pixels a side. E2: four references at
/// 1024² took 190 s and four at 512² 79 s; E10: a whole portrait at 512² held
/// identity within a few hundredths of a tight face crop.
pub const REFERENCE_SIZE: u32 = 512;

/// What an entry is. The directory it lives under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Character,
    Style,
}

impl Kind {
    pub const ALL: [Kind; 2] = [Kind::Character, Kind::Style];

    fn dir(self) -> &'static str {
        match self {
            Kind::Character => "characters",
            Kind::Style => "styles",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Character => "character",
            Kind::Style => "style",
        }
    }
}

/// Whether the owner has approved an entry. **A closed enum on disk is a wire
/// format**: anything unreadable loads as `Candidate`, so a damaged or
/// future-written status can never make an entry usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Approved,
    #[default]
    #[serde(other)]
    Candidate,
}

/// Who wrote an entry's text. Recorded mechanically — the owner's doors write
/// `Owner`, a model's proposal writes what the conversation's taint says —
/// and an unreadable value loads as `ModelUntrusted`: unknown is never clean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Owner,
    ModelClean,
    #[default]
    #[serde(other)]
    ModelUntrusted,
}

impl Origin {
    /// A model's proposal, classified from the conversation's taint when it
    /// was made. `None` — a run wired outside the loop — is untrusted.
    pub fn of_proposal(taint: Option<&crate::agent::Taint>) -> Origin {
        match taint {
            Some(t) if !t.untrusted => Origin::ModelClean,
            _ => Origin::ModelUntrusted,
        }
    }
}

fn one() -> u32 {
    1
}

/// One character or style, as stored in `<kind>/<name>/entry.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Pinned to the directory on load; a file that disagrees is refused.
    pub name: String,
    /// A character's description or a style's text.
    #[serde(default)]
    pub text: String,
    /// A character's portrait: `sha256-<hex>.<ext>` under `blobs/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portrait: Option<String>,
    /// The seed that drew the portrait, when known — a scene sampled at it
    /// risks redrawing the portrait instead of placing its subject.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_seed: Option<u64>,
    #[serde(default)]
    pub status: Status,
    #[serde(default)]
    pub origin: Origin,
    /// A browsing filter only (the owner's ruling, 2026-09-28): generation
    /// ignores it.
    #[serde(default)]
    pub locked: bool,
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub updated: String,
    /// Set on load from the directory, never stored.
    #[serde(skip, default = "default_kind")]
    pub kind: Kind,
}

fn default_kind() -> Kind {
    Kind::Character
}

/// An entry that would not load, kept so the CLI can say so. A silently
/// missing character looks exactly like one never made.
#[derive(Debug, Clone)]
pub struct LoadError {
    pub path: PathBuf,
    pub why: String,
}

/// Every entry on the machine, sorted by kind then name.
#[derive(Debug, Clone, Default)]
pub struct Library {
    dir: PathBuf,
    entries: Vec<Entry>,
    /// What did not load, kept beside what did so a lookup can tell "no such
    /// entry" from "an entry that is broken".
    errors: Vec<LoadError>,
}

pub fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-');
    if !ok {
        bail!(
            "`{name}` is not a valid name: use lowercase letters, digits and hyphens, \
             starting with a letter or digit, at most {MAX_NAME} characters"
        );
    }
    Ok(())
}

fn validate_text(kind: Kind, text: &str) -> Result<()> {
    let cap = match kind {
        Kind::Character => MAX_DESCRIPTION,
        Kind::Style => MAX_STYLE_TEXT,
    };
    if text.trim().is_empty() {
        bail!("a {} needs text", kind.label());
    }
    if text.chars().count() > cap {
        bail!("a {}'s text is capped at {cap} characters", kind.label());
    }
    if text.chars().any(|c| c.is_control() && c != '\n') {
        bail!(
            "control characters are not allowed in a {}'s text",
            kind.label()
        );
    }
    Ok(())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    write_atomic_mode(path, bytes, None)
}

/// Write-then-rename through a temp file that is new or not at all: a fresh
/// random name, `create_new`, and no symlink followed. A fixed `.tmp` name
/// opened with `create(true)` kept an existing file's mode — so a lock file
/// promised 0600 could arrive with any mode — and wrote through a symlink
/// planted there, which the rename then installed as the file itself (review
/// of #385; `serve`'s `write_private_temp` is the same fix, #258).
pub(crate) fn write_atomic_mode(path: &Path, bytes: &[u8], mode: Option<u32>) -> Result<()> {
    use std::io::Write;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow!("{} has no file name", path.display()))?;
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
        if let Some(mode) = mode {
            options.mode(mode);
        }
    }
    #[cfg(not(unix))]
    let _ = mode;
    let written = (|| -> Result<()> {
        let mut f = options
            .open(&tmp)
            .with_context(|| format!("writing {}", tmp.display()))?;
        f.write_all(bytes)?;
        f.sync_all()?;
        Ok(())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        // A failed rename leaves a fresh random name behind each time, where
        // the old fixed name left at most one: take it back (review of #385).
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("installing {}", path.display()));
    }
    Ok(())
}

/// A picture's type from its bytes, and a check that its header decodes.
fn picture_type(bytes: &[u8]) -> Result<&'static str> {
    let ext = crate::imagegen::sniff_image(bytes)
        .ok_or_else(|| anyhow!("not a PNG, JPEG or WebP image"))?;
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .context("reading the image header")?
        .into_dimensions()
        .context("the image header does not decode")?;
    Ok(ext)
}

impl Library {
    /// `~/.mecha/imagelib`.
    pub fn default_dir() -> Result<PathBuf> {
        Ok(crate::work::mecha_home()?.join("imagelib"))
    }

    /// Read every `<dir>/<kind>/<name>/entry.toml`. Best-effort per entry, and
    /// **read-only** — a missing directory is an empty library.
    pub fn load(dir: &Path) -> (Library, Vec<LoadError>) {
        let mut entries = Vec::new();
        let mut errors = Vec::new();
        for kind in Kind::ALL {
            let Ok(names) = std::fs::read_dir(dir.join(kind.dir())) else {
                continue;
            };
            for item in names.flatten() {
                let path = item.path().join("entry.toml");
                if !path.is_file() {
                    continue;
                }
                match load_entry(kind, &item.path()) {
                    Ok(entry) => entries.push(entry),
                    Err(e) => errors.push(LoadError {
                        path,
                        why: format!("{e:#}"),
                    }),
                }
            }
        }
        entries.sort_by(|a, b| (a.kind, &a.name).cmp(&(b.kind, &b.name)));
        (
            Library {
                dir: dir.to_path_buf(),
                entries,
                errors: errors.clone(),
            },
            errors,
        )
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn all(&self) -> &[Entry] {
        &self.entries
    }

    pub fn get(&self, kind: Kind, name: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.kind == kind && e.name == name)
    }

    /// Any entry by name, whatever its kind — for the CLI, where a name is
    /// typed without one. Names are unique per kind, not across kinds.
    pub fn find(&self, name: &str) -> Vec<&Entry> {
        self.entries.iter().filter(|e| e.name == name).collect()
    }

    pub fn approved(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.status == Status::Approved)
    }

    /// Entries that did not load.
    pub fn errors(&self) -> &[LoadError] {
        &self.errors
    }

    pub fn candidates(&self) -> impl Iterator<Item = &Entry> {
        self.entries
            .iter()
            .filter(|e| e.status == Status::Candidate)
    }

    /// Where a portrait's bytes live.
    pub fn blob_path(&self, blob: &str) -> PathBuf {
        self.dir.join("blobs").join(blob)
    }

    /// A character's portrait, checked against the hash it is stored under:
    /// a blob edited on disk is refused, not sent.
    pub fn read_portrait(&self, entry: &Entry) -> Result<(Vec<u8>, &'static str)> {
        let blob = entry
            .portrait
            .as_deref()
            .ok_or_else(|| anyhow!("`{}` has no portrait", entry.name))?;
        let bytes = std::fs::read(self.blob_path(blob))
            .with_context(|| format!("reading `{}`'s portrait", entry.name))?;
        if blob_name(&bytes, picture_type(&bytes)?) != blob {
            bail!("`{}`'s portrait does not match its hash", entry.name);
        }
        let ext = picture_type(&bytes)?;
        Ok((bytes, ext))
    }
}

fn load_entry(kind: Kind, dir: &Path) -> Result<Entry> {
    let raw = std::fs::read_to_string(dir.join("entry.toml"))?;
    let mut entry: Entry = toml::from_str(&raw)?;
    entry.kind = kind;
    let dir_name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow!("unreadable directory name"))?;
    if entry.name != dir_name {
        bail!(
            "names itself `{}` but lives in `{dir_name}/`; the directory is the name",
            entry.name
        );
    }
    validate_name(&entry.name)?;
    if kind == Kind::Character && entry.portrait.is_none() {
        bail!("a character without a portrait");
    }
    if let Some(blob) = &entry.portrait {
        // A plain file name, so a hand-edited entry cannot point a reference
        // at anything outside `blobs/`.
        if blob.contains('/') || blob.contains('\\') || blob.starts_with('.') {
            bail!("portrait `{blob}` is not a blob name");
        }
    }
    Ok(entry)
}

fn blob_name(bytes: &[u8], ext: &str) -> String {
    let digest = sha2::Sha256::digest(bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256-{hex}.{ext}")
}

/// Copy a picture into `blobs/` under its hash; return the blob name.
fn store_blob(dir: &Path, bytes: &[u8]) -> Result<String> {
    if bytes.len() as u64 > MAX_PORTRAIT_BYTES {
        bail!(
            "the portrait is {} MB; portraits are capped at {} MB",
            bytes.len() / (1024 * 1024),
            MAX_PORTRAIT_BYTES / (1024 * 1024)
        );
    }
    let ext = picture_type(bytes)?;
    let name = blob_name(bytes, ext);
    let blobs = dir.join("blobs");
    std::fs::create_dir_all(&blobs)?;
    let path = blobs.join(&name);
    if !path.exists() {
        write_atomic(&path, bytes)?;
    }
    Ok(name)
}

/// What a new entry is made of.
#[derive(Debug, Clone)]
pub struct NewEntry {
    pub kind: Kind,
    pub name: String,
    pub text: String,
    pub portrait: Option<Vec<u8>>,
    pub source_seed: Option<u64>,
    pub origin: Origin,
    pub locked: bool,
}

/// Create an entry. The owner's are approved on creation; a model's are
/// candidates. The entry's directory is claimed with a plain `create_dir`, so
/// two creations of one name cannot both succeed.
pub fn create(dir: &Path, new: NewEntry) -> Result<Entry> {
    validate_name(&new.name)?;
    validate_text(new.kind, &new.text)?;
    match (new.kind, &new.portrait) {
        (Kind::Character, None) => bail!("a character needs a portrait"),
        (Kind::Style, Some(_)) => bail!("a style has no portrait"),
        _ => {}
    }
    let status = if new.origin == Origin::Owner {
        Status::Approved
    } else {
        Status::Candidate
    };
    if status == Status::Candidate {
        if let Some(bytes) = &new.portrait {
            if bytes.len() as u64 > MAX_PROPOSED_PORTRAIT_BYTES {
                bail!(
                    "a proposed portrait is capped at {} MB; this one is {:.1} MB",
                    MAX_PROPOSED_PORTRAIT_BYTES / (1024 * 1024),
                    bytes.len() as f64 / (1024.0 * 1024.0)
                );
            }
        }
        let (lib, _) = Library::load(dir);
        if lib.candidates().count() >= MAX_PENDING {
            bail!(
                "{MAX_PENDING} candidates are already waiting on the owner; no more can be \
                 proposed until some are approved or rejected"
            );
        }
    }
    let kind_dir = dir.join(new.kind.dir());
    std::fs::create_dir_all(&kind_dir)?;
    let entry_dir = kind_dir.join(&new.name);
    // Claim the name before storing anything, so a collision leaves nothing.
    match std::fs::create_dir(&entry_dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            bail!("a {} named `{}` already exists", new.kind.label(), new.name)
        }
        Err(e) => return Err(e).context("creating the entry"),
    }
    let portrait = match new
        .portrait
        .as_deref()
        .map(|b| store_blob(dir, b))
        .transpose()
    {
        Ok(portrait) => portrait,
        Err(e) => {
            // Give the name back: an entry directory with no entry is a
            // claim nothing can use or clear.
            let _ = std::fs::remove_dir(&entry_dir);
            return Err(e);
        }
    };
    let stamp = now();
    let entry = Entry {
        name: new.name,
        text: new.text.trim().to_string(),
        portrait,
        source_seed: new.source_seed,
        status,
        origin: new.origin,
        locked: new.locked,
        version: 1,
        created: stamp.clone(),
        updated: stamp,
        kind: new.kind,
    };
    if let Err(e) = write_entry(dir, &entry) {
        // The same reason as the rollback above: a claimed name with no entry
        // is invisible to `load` and refuses every later `create` (found on
        // review of #383).
        let _ = std::fs::remove_dir_all(&entry_dir);
        return Err(e);
    }
    Ok(entry)
}

fn entry_dir(dir: &Path, kind: Kind, name: &str) -> PathBuf {
    dir.join(kind.dir()).join(name)
}

fn write_entry(dir: &Path, entry: &Entry) -> Result<()> {
    let text = toml::to_string_pretty(entry)?;
    write_atomic(
        &entry_dir(dir, entry.kind, &entry.name).join("entry.toml"),
        text.as_bytes(),
    )
}

fn current(dir: &Path, kind: Kind, name: &str) -> Result<Entry> {
    validate_name(name)?;
    let path = entry_dir(dir, kind, name);
    if !path.join("entry.toml").is_file() {
        bail!("no {} named `{name}`", kind.label());
    }
    load_entry(kind, &path)
}

/// The owner's approval of a candidate. The CLI shows the text first; a
/// `ModelUntrusted` candidate cannot be approved without it being shown.
pub fn approve(dir: &Path, kind: Kind, name: &str) -> Result<Entry> {
    let mut entry = current(dir, kind, name)?;
    if entry.status == Status::Approved {
        bail!("`{name}` is already approved");
    }
    entry.status = Status::Approved;
    entry.updated = now();
    write_entry(dir, &entry)?;
    Ok(entry)
}

/// Approve a candidate as it was *shown*: re-read at the moment of writing,
/// and refused unless its [`shown_digest`] still matches what the approval
/// surface displayed. The web door's form of `approve`'s "read it first".
pub fn approve_as_shown(dir: &Path, kind: Kind, name: &str, shown: &str) -> Result<Entry> {
    let entry = current(dir, kind, name)?;
    if shown_digest(&entry) != shown {
        bail!("`{name}` changed since it was shown; look at it again before approving");
    }
    approve(dir, kind, name)
}

/// The browse filter, per item.
pub fn set_locked(dir: &Path, kind: Kind, name: &str, locked: bool) -> Result<Entry> {
    let mut entry = current(dir, kind, name)?;
    entry.locked = locked;
    entry.updated = now();
    write_entry(dir, &entry)?;
    Ok(entry)
}

/// A new version of an entry, by the owner. The old `entry.toml` is kept as
/// `history/v<N>.toml`, so a manifest naming an old version still resolves.
pub fn update(
    dir: &Path,
    kind: Kind,
    name: &str,
    text: Option<String>,
    portrait: Option<Vec<u8>>,
    source_seed: Option<u64>,
) -> Result<Entry> {
    let old = current(dir, kind, name)?;
    let mut entry = old.clone();
    if let Some(text) = text {
        validate_text(kind, &text)?;
        entry.text = text.trim().to_string();
    }
    if let Some(bytes) = portrait {
        if kind != Kind::Character {
            bail!("a style has no portrait");
        }
        entry.portrait = Some(store_blob(dir, &bytes)?);
        entry.source_seed = source_seed;
    } else if source_seed.is_some() {
        if kind != Kind::Character {
            bail!("a style has no portrait, so no seed");
        }
        entry.source_seed = source_seed;
    }
    if entry.text == old.text
        && entry.portrait == old.portrait
        && entry.source_seed == old.source_seed
    {
        bail!("nothing to change");
    }
    let history = entry_dir(dir, kind, name).join("history");
    std::fs::create_dir_all(&history)?;
    write_atomic(
        &history.join(format!("v{}.toml", old.version)),
        toml::to_string_pretty(&old)?.as_bytes(),
    )?;
    entry.version = old.version + 1;
    // Only a rewritten text is the owner's. A new portrait or seed is not a
    // reading of the text, so it must not approve a candidate or relabel its
    // provenance — that would launder a model's text past `approve`'s
    // untrusted-text question (found on review of #383).
    if entry.text != old.text {
        entry.origin = Origin::Owner;
        entry.status = Status::Approved;
    }
    entry.updated = now();
    write_entry(dir, &entry)?;
    Ok(entry)
}

/// Reject a candidate: deleted outright, and its portrait with it unless
/// something else still names that blob.
///
/// Not moved aside like [`remove`]: nothing was ever generated from a
/// candidate, so no manifest names it — and moving it aside freed the name
/// and the pending slot while its portrait stayed, so propose, reject,
/// propose deposited up to 25 MB a cycle of model-supplied bytes in the mecha
/// home from a tool that needs no approval (found on review of #383).
pub fn reject(dir: &Path, kind: Kind, name: &str) -> Result<()> {
    let entry = current(dir, kind, name)?;
    if entry.status != Status::Candidate {
        bail!("`{name}` is approved, not a candidate");
    }
    std::fs::remove_dir_all(entry_dir(dir, kind, name))?;
    if let Some(blob) = &entry.portrait {
        if !blob_referenced(dir, blob) {
            let _ = std::fs::remove_file(dir.join("blobs").join(blob));
        }
    }
    Ok(())
}

/// Whether any entry — current, historical, or removed — names `blob`.
/// Unreadable means referenced: a blob is never deleted on a guess.
fn blob_referenced(dir: &Path, blob: &str) -> bool {
    fn walk(path: &Path, blob: &str) -> std::io::Result<bool> {
        for item in std::fs::read_dir(path)? {
            let path = item?.path();
            if path.is_dir() {
                if walk(&path, blob)? {
                    return Ok(true);
                }
            } else if path.extension().is_some_and(|e| e == "toml")
                && std::fs::read_to_string(&path)?.contains(blob)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    ["characters", "styles", "removed"]
        .iter()
        .map(|d| dir.join(d))
        .filter(|d| d.exists())
        .any(|d| walk(&d, blob).unwrap_or(true))
}

/// Remove an entry — moved aside under `removed/`, not deleted, because a
/// manifest may still name it, and so its portrait stays in `blobs/`.
pub fn remove(dir: &Path, kind: Kind, name: &str) -> Result<()> {
    current(dir, kind, name)?;
    let removed = dir.join("removed");
    std::fs::create_dir_all(&removed)?;
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    std::fs::rename(
        entry_dir(dir, kind, name),
        removed.join(format!("{}-{name}-{stamp}", kind.label())),
    )?;
    Ok(())
}

// ─── The browse lock and approval binding ──────────────────────────────────

/// The lock password's file: an argon2id hash, mode 0600, beside the entries.
const LOCK_FILE: &str = "lock.toml";
/// Shorter than this is refused when the password is set.
pub const MIN_LOCK_PASSWORD: usize = 6;

#[derive(Serialize, Deserialize)]
struct LockFile {
    /// A PHC-format argon2id string.
    hash: String,
}

/// Whether a lock password has been set. Until one is, locked entries stay
/// hidden while browsing and nothing can show them — the owner's ruling makes
/// the lock a browse filter, and a filter with no key is still a filter.
pub fn has_lock_password(dir: &Path) -> bool {
    dir.join(LOCK_FILE).is_file()
}

/// Set (or replace) the lock password. The owner's act from the CLI only:
/// the password is never typed into a chat, where it would land in a
/// transcript, and never set from the page.
pub fn set_lock_password(dir: &Path, password: &str) -> Result<()> {
    use argon2::password_hash::{PasswordHasher, SaltString};
    if password.chars().count() < MIN_LOCK_PASSWORD {
        bail!("the lock password needs at least {MIN_LOCK_PASSWORD} characters");
    }
    // A v4 UUID's bytes from the OS generator — 122 random bits, six being
    // the UUID's version and variant: no second RNG crate for one salt.
    let salt = SaltString::encode_b64(uuid::Uuid::new_v4().as_bytes())
        .map_err(|e| anyhow!("making a salt: {e}"))?;
    let hash = argon2::Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow!("hashing the lock password: {e}"))?
        .to_string();
    std::fs::create_dir_all(dir)?;
    let text = toml::to_string_pretty(&LockFile { hash })?;
    write_atomic_mode(&dir.join(LOCK_FILE), text.as_bytes(), Some(0o600))
        .context("writing the lock file")
}

/// Whether `password` is the lock password. `Ok(false)` when none is set;
/// `Err` when the file cannot be read or parsed — an unreadable lock is a
/// finding, never an open one.
pub fn verify_lock_password(dir: &Path, password: &str) -> Result<bool> {
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    let path = dir.join(LOCK_FILE);
    if !path.is_file() {
        return Ok(false);
    }
    let file: LockFile =
        toml::from_str(&std::fs::read_to_string(&path)?).context("reading the lock file")?;
    let parsed =
        PasswordHash::new(&file.hash).map_err(|e| anyhow!("the lock file is damaged: {e}"))?;
    Ok(argon2::Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

/// The autolock's file, beside the lock's: how long an unlock lasts with no
/// one using it. Its own file, because `lock.toml`'s presence is what says a
/// password is set.
const AUTOLOCK_FILE: &str = "autolock.toml";
/// Minutes an unlock lasts without use when the owner has not said.
pub const DEFAULT_AUTOLOCK_MINUTES: u32 = 15;
/// The longest the owner may choose: the lock is a browse filter, and an
/// unlock left open for a day is not one.
pub const MAX_AUTOLOCK_MINUTES: u32 = 240;

#[derive(Serialize, Deserialize)]
struct AutolockFile {
    idle_minutes: u32,
}

/// Minutes of no use before an unlock lapses — the page's idle clock and the
/// server's token both. [`DEFAULT_AUTOLOCK_MINUTES`] when nothing is set;
/// `Err` when the file cannot be read or holds a value out of range — a
/// damaged setting is a finding, never a longer unlock.
pub fn autolock_minutes(dir: &Path) -> Result<u32> {
    let path = dir.join(AUTOLOCK_FILE);
    if !path.is_file() {
        return Ok(DEFAULT_AUTOLOCK_MINUTES);
    }
    let text = std::fs::read_to_string(&path).context("reading the autolock setting")?;
    let file: AutolockFile = toml::from_str(&text).context("reading the autolock setting")?;
    check_autolock(file.idle_minutes)?;
    Ok(file.idle_minutes)
}

/// Set how many minutes of no use lock the library and personas again.
pub fn set_autolock_minutes(dir: &Path, minutes: u32) -> Result<()> {
    check_autolock(minutes)?;
    std::fs::create_dir_all(dir)?;
    let text = toml::to_string_pretty(&AutolockFile {
        idle_minutes: minutes,
    })?;
    write_atomic_mode(&dir.join(AUTOLOCK_FILE), text.as_bytes(), Some(0o600))
        .context("writing the autolock setting")
}

fn check_autolock(minutes: u32) -> Result<()> {
    if !(1..=MAX_AUTOLOCK_MINUTES).contains(&minutes) {
        bail!("the autolock is 1 to {MAX_AUTOLOCK_MINUTES} minutes, not {minutes}");
    }
    Ok(())
}

/// What an approval surface showed the owner: a digest over the entry's kind,
/// name, version and text. The web page sends back the digest of the text it
/// displayed; approval proceeds only if it still matches, so an untrusted
/// candidate is approved as *read*, not as whatever it says by the time the
/// click lands.
pub fn shown_digest(entry: &Entry) -> String {
    let digest = sha2::Sha256::digest(
        format!(
            "{}/{}/v{}\n{}",
            entry.kind.label(),
            entry.name,
            entry.version,
            entry.text
        )
        .as_bytes(),
    );
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// HMAC-SHA256 (RFC 2104) of a [`shown_digest`] under `key`, hex. The web
/// door signs what it displays with a key only its process holds, so an
/// approval can prove a page showed the text — a bare digest proves only
/// that the text did not move, and anyone who can read the store can
/// compute one (review of #385).
pub fn sign_shown(key: &[u8; 32], digest: &str) -> String {
    let mut block = [0u8; 64];
    block[..32].copy_from_slice(key);
    let pad = |b: u8| block.iter().map(|x| x ^ b).collect::<Vec<u8>>();
    let inner = sha2::Sha256::new()
        .chain_update(pad(0x36))
        .chain_update(digest.as_bytes())
        .finalize();
    let outer = sha2::Sha256::new()
        .chain_update(pad(0x5c))
        .chain_update(inner)
        .finalize();
    outer.iter().map(|b| format!("{b:02x}")).collect()
}

// ─── Compiling a scene ─────────────────────────────────────────────────────

/// One person in a scene, as the model wrote them.
#[derive(Debug, Clone, PartialEq)]
pub struct CastMember {
    pub name: String,
    pub wearing: String,
    pub doing: String,
}

/// What an entry contributed to a generation — for the result text and the
/// manifest.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Used {
    pub kind: Kind,
    pub name: String,
    pub version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub portrait: Option<String>,
}

/// A scene, compiled.
#[derive(Debug)]
pub struct Compiled {
    pub prompt: String,
    /// `(name, bytes, ext)` in `<imageN>` order.
    pub references: Vec<(String, Vec<u8>, &'static str)>,
    pub used: Vec<Used>,
    /// Seeds that drew the cast's portraits, when known.
    pub source_seeds: Vec<u64>,
}

/// The whole compiled prompt, a cap on the sum of its parts.
pub const MAX_COMPILED_PROMPT: usize = 9_000;

fn number(n: usize) -> &'static str {
    [
        "no", "one", "two", "three", "four", "five", "six", "seven", "eight",
    ]
    .get(n)
    .copied()
    .unwrap_or("several")
}

pub(crate) fn missing(lib: &Library, kind: Kind, name: &str) -> String {
    // A corrupt entry is a finding, not an absence: said as such, or the
    // model is told a character the owner can see does not exist.
    let entry_dir = lib.dir.join(kind.dir()).join(name);
    if lib
        .errors
        .iter()
        .any(|e| e.path.parent() == Some(entry_dir.as_path()))
    {
        return format!(
            "The {} `{name}` is in the library but its entry could not be read; the owner can \
             check it with `mecha imagelib list`.",
            kind.label()
        );
    }
    match lib.get(kind, name) {
        Some(e) if e.status == Status::Candidate => format!(
            "The {} `{name}` is waiting for the owner's approval and cannot be used yet.",
            kind.label()
        ),
        _ => match what_there_is(lib, kind) {
            Some(there) => format!("No approved {} named `{name}`. {there}.", kind.label()),
            None => format!("No approved {} named `{name}`.", kind.label()),
        },
    }
}

/// The approved, unlocked entries of `kind`, by name: what a refusal or a
/// note may say the library holds. A locked entry is left out without a
/// count, as the page leaves it out while browsing.
pub(crate) fn approved_names(lib: &Library, kind: Kind) -> Vec<String> {
    lib.all()
        .iter()
        .filter(|e| e.kind == kind && e.status == Status::Approved && !e.locked)
        .map(|e| e.name.clone())
        .collect()
}

/// Whether `name` is simply not in the library as `kind`: no entry in any
/// state, and none that failed to load. A candidate waiting on the owner or
/// an entry that did not load is a finding, said by [`missing`], never an
/// absence (review of #603).
pub(crate) fn absent(lib: &Library, kind: Kind, name: &str) -> bool {
    let entry_dir = lib.dir.join(kind.dir()).join(name);
    lib.get(kind, name).is_none()
        && !lib
            .errors
            .iter()
            .any(|e| e.path.parent() == Some(entry_dir.as_path()))
}

/// The names of `kind` a refusal or a note may offer, as one sentence, or
/// `None` when there are none to offer. Never a tool to call (a persona chat
/// has no `image_library`, and one retried a refusal naming it until its
/// turns ran out, 2026-10-08), and never "there are none": a locked entry is
/// left out of what is offered but still draws, so absence is not claimed
/// (review of #603).
pub(crate) fn what_there_is(lib: &Library, kind: Kind) -> Option<String> {
    let names: Vec<String> = approved_names(lib, kind)
        .iter()
        .map(|n| format!("`{n}`"))
        .collect();
    (!names.is_empty()).then(|| {
        format!(
            "{} you can name: {}",
            capitalize(kind.dir()),
            names.join(", ")
        )
    })
}

/// Empty, or only the refusal's own placeholder copied back — no answer.
pub(crate) fn blank(s: &str) -> bool {
    s.chars().all(|c| c == '…' || c == '.')
}

/// Compile a scene. Errors are sentences for the model.
///
/// The prompt's shape is Qwen-Image 2.1's and each clause is a measurement:
/// one person is "the person in the image" (the rewriter's rule for a single
/// reference); two or more are `<image1>…` left to right (E2, E9: each
/// reference slot tends to become a person), each said to appear exactly
/// once — with a total only when `extras` are counted into it (E11), since a
/// total with no extras erased a person the scene described (E12); every
/// person carries their stored description beside the pointer (E1) and what
/// they are wearing and doing (E8: a reference supplies its own otherwise).
pub fn compile(
    lib: &Library,
    scene: &str,
    cast: &[CastMember],
    extras: &[String],
    style: Option<&str>,
) -> std::result::Result<Compiled, String> {
    if extras.len() > MAX_EXTRAS {
        return Err(format!(
            "At most {MAX_EXTRAS} people who are not library characters fit one new picture, \
             not {}.",
            extras.len()
        ));
    }
    let extras: Vec<&str> = extras
        .iter()
        .map(|e| e.trim().trim_end_matches('.'))
        .collect();
    // Each person's fields are capped where the call is read; a described
    // person here is those fields joined, so no cap of its own.
    if extras.iter().any(|e| blank(e)) {
        return Err(
            "Each person not from the library is described in `who`: what they look like.".into(),
        );
    }
    if cast.len() > MAX_CAST {
        return Err(format!(
            "At most {MAX_CAST} library characters fit one new picture, not {}.",
            cast.len()
        ));
    }
    // An extra is someone the library does not hold. One who names a cast
    // member is that person again as "a new person not from any image": two
    // slots for one face, the duplicate the rest of this shape prevents
    // (review of #390).
    for extra in &extras {
        if let Some(name) = named_in(lib, extra)
            .into_iter()
            .find(|n| cast.iter().any(|m| m.name.trim().to_lowercase() == *n))
        {
            return Err(format!(
                "`{name}` is drawn from the library already, and a description naming them \
                 draws them twice. Describe the other person without the name, and put what \
                 they do together in `together`."
            ));
        }
    }
    let mut refs = Vec::with_capacity(cast.len());
    let mut used = Vec::new();
    let mut source_seeds = Vec::new();
    let mut people = Vec::with_capacity(cast.len());
    for (i, member) in cast.iter().enumerate() {
        let name = member.name.trim().to_lowercase();
        if cast[..i]
            .iter()
            .any(|m| m.name.trim().to_lowercase() == name)
        {
            return Err(format!(
                "`{name}` is in `scene.people` twice; each person is drawn once."
            ));
        }
        let entry = lib
            .get(Kind::Character, &name)
            .filter(|e| e.status == Status::Approved)
            .ok_or_else(|| missing(lib, Kind::Character, &name))?;
        let (wearing, doing) = (member.wearing.trim(), member.doing.trim());
        // A placeholder copied from a refusal's example is no answer.
        if blank(wearing) || blank(doing) {
            return Err(format!(
                "`{name}` needs `wearing` and `doing`: a reference supplies its own outfit and \
                 pose when the scene does not say."
            ));
        }
        if wearing.chars().count() > MAX_CAST_FIELD || doing.chars().count() > MAX_CAST_FIELD {
            return Err(format!(
                "`wearing` and `doing` are capped at {MAX_CAST_FIELD} characters each."
            ));
        }
        let (bytes, ext) = lib
            .read_portrait(entry)
            .map_err(|e| format!("`{name}`'s portrait cannot be read: {e:#}."))?;
        refs.push((name.clone(), bytes, ext));
        source_seeds.extend(entry.source_seed);
        used.push(Used {
            kind: Kind::Character,
            name: name.clone(),
            version: entry.version,
            portrait: entry.portrait.clone(),
        });
        let who = if cast.len() == 1 {
            "the person in the image".to_string()
        } else {
            format!("the person from <image{}>", i + 1)
        };
        people.push(format!(
            "{who} ({}), wearing {wearing}, {doing}",
            entry.text.trim().trim_end_matches('.')
        ));
    }
    let style_text = match style.map(|s| s.trim().to_lowercase()) {
        None => None,
        Some(name) if name.is_empty() => None,
        Some(name) => {
            let entry = lib
                .get(Kind::Style, &name)
                .filter(|e| e.status == Status::Approved)
                .ok_or_else(|| missing(lib, Kind::Style, &name))?;
            used.push(Used {
                kind: Kind::Style,
                name,
                version: entry.version,
                portrait: None,
            });
            Some(entry.text.trim().to_string())
        }
    };

    let scene = scene.trim();
    let mut prompt = if scene.ends_with(['.', '!', '?']) {
        scene.to_string()
    } else {
        format!("{scene}.")
    };
    // Two measured rules. With extras, the head count counts everyone (E11):
    // "Exactly three people" beside a scene with a waiter pushed him into the
    // background, and counted as a new person he stood where the scene put
    // him. Without extras, no total at all (E12): the model does not always
    // use `extras` — a live run wrote the waiter into the prose — and there
    // "Exactly two people" erased him outright, while "each appears exactly
    // once; anyone else is a new person" drew him and, with four cast and no
    // one else, still drew exactly four with no duplicate.
    let others = extras.join("; ");
    let m = extras.len();
    let new_people = if m == 1 {
        "one new person".to_string()
    } else {
        format!("{} new people", number(m))
    };
    match (people.len(), m) {
        (0, 0) => {}
        (0, _) => prompt.push_str(&format!(" Also in the scene: {others}.")),
        (1, 0) => prompt.push_str(&format!(
            " {}. The person from the image appears exactly once; anyone else the scene \
             describes is a new person, not from the image. The image is an identity reference \
             only; this is a new image with its own composition, pose and lighting.",
            capitalize(&people[0])
        )),
        (1, _) => prompt.push_str(&format!(
            " {}. Also in the scene, not from the image: {others}. Exactly {} people in the \
             image: the one from the image, exactly once, and {new_people} not from any image. \
             The image is an identity reference only; this is a new image with its own \
             composition, pose and lighting.",
            capitalize(&people[0]),
            number(1 + m)
        )),
        (n, 0) => prompt.push_str(&format!(
            " From left to right: {}. Each of the {} people from the images appears exactly \
             once; anyone else the scene describes is a new person, not from any image. All \
             images serve as identity sources only.",
            people.join("; "),
            number(n)
        )),
        (n, _) => prompt.push_str(&format!(
            " From left to right: {}. Also in the scene, not from any image: {others}. Exactly \
             {} people in the image: the {} from the images, each exactly once, and \
             {new_people} not from any image. All images serve as identity sources only.",
            people.join("; "),
            number(n + m),
            number(n)
        )),
    }
    if let Some(text) = style_text {
        prompt.push_str(&format!(" Style: {text}"));
    }
    if prompt.chars().count() > MAX_COMPILED_PROMPT {
        return Err(format!(
            "The compiled prompt is over {MAX_COMPILED_PROMPT} characters; shorten the scene."
        ));
    }
    Ok(Compiled {
        prompt,
        references: refs,
        used,
        source_seeds,
    })
}

/// Approved characters a prompt names as whole words, sorted.
///
/// The guard behind a failure the first real run hit: the model looked the
/// characters up, then wrote their descriptions into the prompt without
/// naming them as people — and from words alone they came out as two
/// strangers (ArcFace 0.02–0.17 against their portraits; E1 measured the
/// same, 0.33). So the image tool never draws a library name from a prose
/// field: for someone not in the picture the image model reads "the
/// viewer" (`picture::plan`'s `offstage` and `picture::as_viewer`; the
/// owner's ruling of 2026-10-08).
///
/// **In order of first mention**, so a note names them as written:
/// sorted, "Maya and John" came back as `[john, maya]` (review of #384).
pub fn named_in(lib: &Library, prompt: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in prompt
        .split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
    {
        if out.contains(&word) {
            continue;
        }
        if lib
            .get(Kind::Character, &word)
            .is_some_and(|e| e.status == Status::Approved)
        {
            out.push(word);
        }
    }
    out
}

/// Characters whose entries did not load, named as whole words in a prompt.
/// A broken entry is invisible to [`named_in`], and the guard behind it must
/// not read that invisibility as absence.
pub fn broken_named_in(lib: &Library, prompt: &str) -> Vec<String> {
    let words: std::collections::BTreeSet<String> = prompt
        .split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let characters = lib.dir.join(Kind::Character.dir());
    let mut out: Vec<String> = lib
        .errors
        .iter()
        .filter_map(|e| {
            let entry = e.path.parent()?;
            (entry.parent()? == characters.as_path()).then_some(())?;
            entry.file_name()?.to_str().map(str::to_string)
        })
        .filter(|name| words.contains(name))
        .collect();
    out.sort();
    out.dedup();
    out
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real 2×2 PNG, so the header decodes.
    fn png() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([200, 10, 10]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn character(name: &str, origin: Origin) -> NewEntry {
        NewEntry {
            kind: Kind::Character,
            name: name.into(),
            text: format!("{name}, a person with a memorable face"),
            portrait: Some(png()),
            source_seed: Some(1001),
            origin,
            locked: false,
        }
    }

    fn style(name: &str) -> NewEntry {
        NewEntry {
            kind: Kind::Style,
            name: name.into(),
            text: "high-contrast black and white film noir".into(),
            portrait: None,
            source_seed: None,
            origin: Origin::Owner,
            locked: false,
        }
    }

    /// A fresh directory per test; removed when dropped.
    struct Scratch(PathBuf);
    impl Scratch {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    /// A name the library does not hold is answered with what it does hold,
    /// never a tool to call, and never a locked entry (2026-10-08: a persona
    /// chat has no image_library and retried the refusal naming it).
    #[test]
    fn a_missing_entry_names_what_there_is_never_a_tool() {
        let dir = scratch();
        create(dir.path(), style("noir")).unwrap();
        let mut hidden = style("hidden-look");
        hidden.locked = true;
        create(dir.path(), hidden).unwrap();
        create(dir.path(), character("wren", Origin::Owner)).unwrap();
        let lib = Library::load(dir.path()).0;
        let said = missing(&lib, Kind::Style, "pastel");
        assert!(said.contains("`noir`"), "{said}");
        assert!(
            !said.contains("hidden-look") && !said.contains("image_library"),
            "{said}"
        );
        assert!(missing(&lib, Kind::Character, "ivo").contains("`wren`"));
        let empty = scratch();
        let none = Library::load(empty.path()).0;
        let said = missing(&none, Kind::Style, "pastel");
        assert_eq!(
            said, "No approved style named `pastel`.",
            "nothing offered, no absence claimed"
        );
    }

    #[test]
    fn a_failed_install_leaves_no_temp_file_behind() {
        let dir = scratch();
        // A directory where the file should go: the write succeeds, the
        // rename over it fails.
        let path = dir.path().join("entry.toml");
        std::fs::create_dir_all(path.join("occupied")).unwrap();
        for _ in 0..2 {
            assert!(write_atomic_mode(&path, b"x", Some(0o600)).is_err());
        }
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            left,
            ["entry.toml"],
            "a temp file survived the failed rename"
        );
    }

    fn scratch() -> Scratch {
        let dir = std::env::temp_dir().join(format!("mecha-imagelib-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn member(name: &str) -> CastMember {
        CastMember {
            name: name.into(),
            wearing: "a yellow raincoat".into(),
            doing: "laughing".into(),
        }
    }

    #[test]
    fn owner_entries_are_approved_and_model_entries_are_candidates() {
        let dir = scratch();
        let owned = create(dir.path(), character("maya", Origin::Owner)).unwrap();
        let proposed = create(dir.path(), character("john", Origin::ModelClean)).unwrap();
        assert_eq!(owned.status, Status::Approved);
        assert_eq!(proposed.status, Status::Candidate);
        let (lib, errors) = Library::load(dir.path());
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            lib.approved().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["maya"]
        );
        assert_eq!(lib.candidates().count(), 1);
    }

    #[test]
    fn a_name_is_claimed_once() {
        let dir = scratch();
        create(dir.path(), character("maya", Origin::Owner)).unwrap();
        let again = create(dir.path(), character("maya", Origin::ModelClean)).unwrap_err();
        assert!(format!("{again}").contains("already exists"), "{again}");
    }

    #[test]
    fn names_are_plain() {
        for bad in ["", "Maya", "../x", "a/b", "-x", "x y", &"a".repeat(65)] {
            assert!(validate_name(bad).is_err(), "{bad:?} accepted");
        }
        validate_name("maya-2").unwrap();
    }

    #[test]
    fn an_unknown_status_or_origin_fails_closed() {
        let dir = scratch();
        create(dir.path(), character("maya", Origin::Owner)).unwrap();
        let path = dir.path().join("characters/maya/entry.toml");
        let raw = std::fs::read_to_string(&path)
            .unwrap()
            .replace("status = \"approved\"", "status = \"blessed\"")
            .replace("origin = \"owner\"", "origin = \"oracle\"");
        std::fs::write(&path, raw).unwrap();
        let (lib, errors) = Library::load(dir.path());
        assert!(errors.is_empty(), "{errors:?}");
        let e = lib.get(Kind::Character, "maya").unwrap();
        assert_eq!(e.status, Status::Candidate);
        assert_eq!(e.origin, Origin::ModelUntrusted);
    }

    #[test]
    fn the_directory_is_the_name() {
        let dir = scratch();
        create(dir.path(), character("maya", Origin::Owner)).unwrap();
        std::fs::rename(
            dir.path().join("characters/maya"),
            dir.path().join("characters/theo"),
        )
        .unwrap();
        let (lib, errors) = Library::load(dir.path());
        assert!(lib.all().is_empty());
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn a_portrait_edited_on_disk_is_refused() {
        let dir = scratch();
        let e = create(dir.path(), character("maya", Origin::Owner)).unwrap();
        let (lib, _) = Library::load(dir.path());
        lib.read_portrait(&e).unwrap();
        let mut other = png();
        other.extend_from_slice(b"trailing");
        std::fs::write(lib.blob_path(e.portrait.as_deref().unwrap()), other).unwrap();
        assert!(lib.read_portrait(&e).is_err());
    }

    #[test]
    fn a_hand_edited_portrait_cannot_leave_blobs() {
        let dir = scratch();
        create(dir.path(), character("maya", Origin::Owner)).unwrap();
        let path = dir.path().join("characters/maya/entry.toml");
        let raw = std::fs::read_to_string(&path).unwrap();
        let raw = raw
            .lines()
            .map(|l| {
                if l.starts_with("portrait") {
                    "portrait = \"../../etc/passwd\"".to_string()
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, raw).unwrap();
        let (lib, errors) = Library::load(dir.path());
        assert!(lib.all().is_empty());
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn an_update_keeps_the_old_version() {
        let dir = scratch();
        create(dir.path(), character("maya", Origin::Owner)).unwrap();
        let e = update(
            dir.path(),
            Kind::Character,
            "maya",
            Some("maya, now with shorter hair".into()),
            None,
            None,
        )
        .unwrap();
        assert_eq!(e.version, 2);
        assert!(dir.path().join("characters/maya/history/v1.toml").is_file());
    }

    #[test]
    fn a_new_portrait_or_seed_never_approves_a_candidate() {
        let dir = scratch();
        create(dir.path(), character("theo", Origin::ModelUntrusted)).unwrap();
        let e = update(dir.path(), Kind::Character, "theo", None, None, Some(7)).unwrap();
        assert_eq!(e.status, Status::Candidate);
        assert_eq!(e.origin, Origin::ModelUntrusted);
        // Rewriting the text is the owner authoring it.
        let e = update(
            dir.path(),
            Kind::Character,
            "theo",
            Some("theo, as the owner describes him".into()),
            None,
            None,
        )
        .unwrap();
        assert_eq!((e.status, e.origin), (Status::Approved, Origin::Owner));
    }

    #[test]
    fn a_collision_stores_nothing() {
        let dir = scratch();
        create(dir.path(), character("maya", Origin::Owner)).unwrap();
        let mut other = character("maya", Origin::Owner);
        let img = image::RgbImage::from_pixel(3, 3, image::Rgb([1, 2, 3]));
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        other.portrait = Some(png.into_inner());
        assert!(create(dir.path(), other).is_err());
        assert_eq!(
            std::fs::read_dir(dir.path().join("blobs")).unwrap().count(),
            1
        );
    }

    #[test]
    fn a_broken_entry_is_said_to_be_broken() {
        let dir = scratch();
        create(dir.path(), character("maya", Origin::Owner)).unwrap();
        std::fs::write(dir.path().join("characters/maya/entry.toml"), "not = [toml").unwrap();
        let (lib, errors) = Library::load(dir.path());
        assert_eq!(errors.len(), 1);
        let why = compile(&lib, "x", &[member("maya")], &[], None).unwrap_err();
        assert!(why.contains("could not be read"), "{why}");
    }

    #[test]
    fn rejecting_a_candidate_takes_its_portrait_unless_shared() {
        let dir = scratch();
        let kept = create(dir.path(), character("maya", Origin::Owner)).unwrap();
        // Same bytes as maya's portrait: one blob, named twice.
        create(dir.path(), character("twin", Origin::ModelUntrusted)).unwrap();
        let mut other = character("theo", Origin::ModelUntrusted);
        let img = image::RgbImage::from_pixel(3, 3, image::Rgb([7, 7, 7]));
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        other.portrait = Some(png.into_inner());
        let theo = create(dir.path(), other).unwrap();
        let blobs = || std::fs::read_dir(dir.path().join("blobs")).unwrap().count();
        assert_eq!(blobs(), 2);

        reject(dir.path(), Kind::Character, "theo").unwrap();
        assert_eq!(blobs(), 1, "theo's own portrait goes with him");
        assert!(!dir
            .path()
            .join("blobs")
            .join(theo.portrait.unwrap())
            .exists());
        reject(dir.path(), Kind::Character, "twin").unwrap();
        assert_eq!(blobs(), 1, "a portrait maya still names stays");
        assert!(dir
            .path()
            .join("blobs")
            .join(kept.portrait.unwrap())
            .exists());
        // The name is free, and nothing lingers under removed/.
        assert!(!dir.path().join("removed").exists());
        assert!(reject(dir.path(), Kind::Character, "maya").is_err());
    }

    #[test]
    fn a_proposed_portrait_is_capped_in_bytes_and_the_owners_is_not() {
        let dir = scratch();
        let mut big = png();
        big.resize((MAX_PROPOSED_PORTRAIT_BYTES + 1) as usize, 0);
        let mut proposed = character("big", Origin::ModelClean);
        proposed.portrait = Some(big.clone());
        let why = create(dir.path(), proposed).unwrap_err();
        assert!(format!("{why}").contains("capped at 4 MB"), "{why}");
        assert!(!dir.path().join("characters/big").exists());
        let mut owned = character("big", Origin::Owner);
        owned.portrait = Some(big);
        create(dir.path(), owned).unwrap();
    }

    #[test]
    fn the_lock_password_verifies_and_is_private_on_disk() {
        let dir = scratch();
        assert!(!has_lock_password(dir.path()));
        assert!(!verify_lock_password(dir.path(), "anything").unwrap());
        assert!(set_lock_password(dir.path(), "short").is_err());
        set_lock_password(dir.path(), "correct horse").unwrap();
        assert!(has_lock_password(dir.path()));
        assert!(verify_lock_password(dir.path(), "correct horse").unwrap());
        assert!(!verify_lock_password(dir.path(), "correct hors").unwrap());
        let raw = std::fs::read_to_string(dir.path().join("lock.toml")).unwrap();
        assert!(raw.contains("$argon2id$") && !raw.contains("correct horse"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("lock.toml"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // A planted `lock.tmp` — a symlink to a file the planter owns — is
        // neither written through nor installed as the lock.
        #[cfg(unix)]
        {
            let decoy = dir.path().join("decoy");
            std::fs::write(&decoy, "untouched").unwrap();
            let _ = std::fs::remove_file(dir.path().join("lock.tmp"));
            std::os::unix::fs::symlink(&decoy, dir.path().join("lock.tmp")).unwrap();
            set_lock_password(dir.path(), "another horse").unwrap();
            assert_eq!(std::fs::read_to_string(&decoy).unwrap(), "untouched");
            assert!(!std::fs::symlink_metadata(dir.path().join("lock.toml"))
                .unwrap()
                .file_type()
                .is_symlink());
            assert!(verify_lock_password(dir.path(), "another horse").unwrap());
        }
        // A damaged lock is a finding, never an open door.
        std::fs::write(dir.path().join("lock.toml"), "hash = \"nonsense\"").unwrap();
        assert!(verify_lock_password(dir.path(), "correct horse").is_err());
    }

    #[test]
    fn the_autolock_defaults_keeps_its_range_and_is_not_the_password() {
        let dir = scratch();
        assert_eq!(
            autolock_minutes(dir.path()).unwrap(),
            DEFAULT_AUTOLOCK_MINUTES
        );
        set_autolock_minutes(dir.path(), 5).unwrap();
        assert_eq!(autolock_minutes(dir.path()).unwrap(), 5);
        // Setting it never makes a password appear: the lock stays a toggle.
        assert!(!has_lock_password(dir.path()));
        for out in [0, MAX_AUTOLOCK_MINUTES + 1] {
            assert!(set_autolock_minutes(dir.path(), out).is_err(), "{out}");
        }
        assert_eq!(autolock_minutes(dir.path()).unwrap(), 5);
        // A hand-edited value out of range is damage, not a long unlock.
        std::fs::write(dir.path().join("autolock.toml"), "idle_minutes = 100000\n").unwrap();
        assert!(autolock_minutes(dir.path()).is_err());
        std::fs::write(dir.path().join("autolock.toml"), "nonsense").unwrap();
        assert!(autolock_minutes(dir.path()).is_err());
    }

    #[test]
    fn the_shown_signature_is_rfc_2104_hmac_sha256() {
        // Known answer from Python's `hmac` (key 0..31, message "abc").
        let key: [u8; 32] = std::array::from_fn(|i| i as u8);
        assert_eq!(
            sign_shown(&key, "abc"),
            "f0133729c4163dede81e21cd47839256da58171238c8a0d874397c73b14e1e47"
        );
        let other: [u8; 32] = std::array::from_fn(|i| 31 - i as u8);
        assert_ne!(sign_shown(&key, "abc"), sign_shown(&other, "abc"));
    }

    #[test]
    fn the_shown_digest_changes_with_the_text_and_the_version() {
        let dir = scratch();
        let e = create(dir.path(), character("theo", Origin::ModelUntrusted)).unwrap();
        let before = shown_digest(&e);
        let mut edited = e.clone();
        edited.text.push_str(" and something else");
        assert_ne!(before, shown_digest(&edited));
        let mut bumped = e.clone();
        bumped.version += 1;
        assert_ne!(before, shown_digest(&bumped));
        assert_eq!(before, shown_digest(&e));
    }

    #[test]
    fn approval_as_shown_refuses_a_text_that_moved() {
        let dir = scratch();
        let e = create(dir.path(), character("theo", Origin::ModelUntrusted)).unwrap();
        let shown = shown_digest(&e);
        assert!(approve_as_shown(dir.path(), Kind::Character, "theo", "0000").is_err());
        update(dir.path(), Kind::Character, "theo", None, None, Some(9)).unwrap();
        // The seed moved, the version bumped: what was shown is not current.
        assert!(approve_as_shown(dir.path(), Kind::Character, "theo", &shown).is_err());
        let now = Library::load(dir.path())
            .0
            .get(Kind::Character, "theo")
            .unwrap()
            .clone();
        let e = approve_as_shown(dir.path(), Kind::Character, "theo", &shown_digest(&now)).unwrap();
        assert_eq!(e.status, Status::Approved);
    }

    #[test]
    fn candidates_are_capped() {
        let dir = scratch();
        for i in 0..MAX_PENDING {
            create(dir.path(), character(&format!("c{i}"), Origin::ModelClean)).unwrap();
        }
        let over = create(dir.path(), character("one-more", Origin::ModelClean)).unwrap_err();
        assert!(format!("{over}").contains("waiting on the owner"), "{over}");
        // The owner's door is not the model's queue.
        create(dir.path(), character("owners", Origin::Owner)).unwrap();
    }

    #[test]
    fn one_person_is_the_person_in_the_image() {
        let dir = scratch();
        create(dir.path(), character("maya", Origin::Owner)).unwrap();
        let (lib, _) = Library::load(dir.path());
        let c = compile(&lib, "a diner at night.", &[member("maya")], &[], None).unwrap();
        assert!(c
            .prompt
            .starts_with("a diner at night. The person in the image (maya, a person"));
        assert!(c.prompt.contains("wearing a yellow raincoat, laughing"));
        assert!(c
            .prompt
            .contains("The person from the image appears exactly once"));
        let c = compile(&lib, "a surprise party!", &[member("maya")], &[], None).unwrap();
        assert!(
            c.prompt.starts_with("a surprise party! The person"),
            "{}",
            c.prompt
        );
        assert!(!c.prompt.contains("<image1>"));
        assert_eq!(c.references.len(), 1);
        assert_eq!(c.source_seeds, vec![1001]);
    }

    #[test]
    fn several_people_are_bound_left_to_right_with_the_head_count() {
        let dir = scratch();
        for n in ["maya", "john", "priya"] {
            create(dir.path(), character(n, Origin::Owner)).unwrap();
        }
        create(dir.path(), style("noir")).unwrap();
        let (lib, _) = Library::load(dir.path());
        let cast = [member("maya"), member("John"), member("priya")];
        let c = compile(&lib, "a diner booth", &cast, &[], Some("noir")).unwrap();
        let first = c.prompt.find("<image1> (maya").unwrap();
        let second = c.prompt.find("<image2> (john").unwrap();
        assert!(first < second);
        assert!(c.prompt.contains("<image3> (priya"));
        assert!(c
            .prompt
            .contains("Each of the three people from the images appears exactly once"));
        assert!(c
            .prompt
            .ends_with("Style: high-contrast black and white film noir"));
        assert_eq!(
            c.references
                .iter()
                .map(|r| r.0.as_str())
                .collect::<Vec<_>>(),
            ["maya", "john", "priya"]
        );
        assert_eq!(c.used.len(), 4);
    }

    #[test]
    fn extras_are_counted_and_called_new_people() {
        let dir = scratch();
        for n in ["maya", "john", "priya"] {
            create(dir.path(), character(n, Origin::Owner)).unwrap();
        }
        let (lib, _) = Library::load(dir.path());
        let cast = [member("maya"), member("john"), member("priya")];
        let waiter = vec!["a waiter in a white apron, pouring coffee.".to_string()];
        let c = compile(&lib, "a diner booth", &cast, &waiter, None).unwrap();
        assert!(
            c.prompt.contains(
                "Also in the scene, not from any image: a waiter in a white apron, pouring coffee."
            ),
            "{}",
            c.prompt
        );
        assert!(
            c.prompt.contains(
                "Exactly four people in the image: the three from the images, each exactly \
                 once, and one new person not from any image."
            ),
            "{}",
            c.prompt
        );
        // Extras carry no reference.
        assert_eq!(c.references.len(), 3);
        // One cast member keeps the single-image wording, counted with the extras.
        let two = vec!["a child".to_string(), "a dog walker".to_string()];
        let c = compile(&lib, "a park", &[member("maya")], &two, None).unwrap();
        assert!(
            c.prompt.contains("The person in the image (maya"),
            "{}",
            c.prompt
        );
        assert!(
            c.prompt.contains("Exactly three people in the image: the one from the image, exactly once, and two new people"),
            "{}",
            c.prompt
        );
        // No cast: extras are scene text, and no head count is claimed.
        let c = compile(&lib, "a park", &[], &two, None).unwrap();
        assert_eq!(
            c.prompt,
            "a park. Also in the scene: a child; a dog walker."
        );
        // No extras: no total that would forbid a person the prose describes.
        let c = compile(&lib, "a park", &cast, &[], None).unwrap();
        assert!(
            c.prompt.contains(
                "Each of the three people from the images appears exactly once; anyone else"
            ),
            "{}",
            c.prompt
        );
    }

    #[test]
    fn a_library_person_named_again_in_a_description_is_refused() {
        let dir = scratch();
        for n in ["maya", "john"] {
            create(dir.path(), character(n, Origin::Owner)).unwrap();
        }
        let (lib, _) = Library::load(dir.path());
        let cast = [member("maya"), member("john")];
        let why = compile(
            &lib,
            "a diner",
            &cast,
            &["John waving from the door".into()],
            None,
        )
        .unwrap_err();
        assert!(
            why.contains("`john` is drawn from the library already"),
            "{why}"
        );
        // A stranger who shares no name with the cast is fine.
        compile(
            &lib,
            "a diner",
            &cast,
            &["a waiter pouring coffee".into()],
            None,
        )
        .unwrap();
    }

    #[test]
    fn an_extra_must_be_a_person_and_there_are_at_most_four() {
        let dir = scratch();
        create(dir.path(), character("maya", Origin::Owner)).unwrap();
        let (lib, _) = Library::load(dir.path());
        for bad in ["", "  ", "…", "..."] {
            let why = compile(&lib, "x", &[member("maya")], &[bad.to_string()], None).unwrap_err();
            assert!(why.contains("described in `who`"), "{bad:?}: {why}");
        }
        let five: Vec<String> = (0..5).map(|i| format!("person {i}")).collect();
        assert!(compile(&lib, "x", &[], &five, None)
            .unwrap_err()
            .contains("At most 4"));
    }

    #[test]
    fn candidates_never_compile() {
        let dir = scratch();
        create(dir.path(), character("maya", Origin::ModelClean)).unwrap();
        let (lib, _) = Library::load(dir.path());
        let why = compile(&lib, "x", &[member("maya")], &[], None).unwrap_err();
        assert!(why.contains("waiting for the owner's approval"), "{why}");
    }

    #[test]
    fn wearing_and_doing_are_required() {
        let dir = scratch();
        create(dir.path(), character("maya", Origin::Owner)).unwrap();
        let (lib, _) = Library::load(dir.path());
        let mut m = member("maya");
        m.wearing = " ".into();
        let why = compile(&lib, "x", &[m], &[], None).unwrap_err();
        assert!(why.contains("wearing"), "{why}");
        // The refusal's own placeholder, copied literally, is no answer.
        let mut m = member("maya");
        m.doing = "…".into();
        assert!(compile(&lib, "x", &[m], &[], None)
            .unwrap_err()
            .contains("doing"));
    }

    #[test]
    fn a_person_is_drawn_once_and_at_most_five() {
        let dir = scratch();
        let names = ["a", "b", "c", "d", "e", "f"];
        for n in names {
            create(dir.path(), character(n, Origin::Owner)).unwrap();
        }
        let (lib, _) = Library::load(dir.path());
        assert!(compile(&lib, "x", &[member("a"), member("a")], &[], None)
            .unwrap_err()
            .contains("twice"));
        let five: Vec<_> = names[..5].iter().map(|n| member(n)).collect();
        assert!(compile(&lib, "x", &five, &[], None).is_ok());
        let six: Vec<_> = names.iter().map(|n| member(n)).collect();
        assert!(compile(&lib, "x", &six, &[], None)
            .unwrap_err()
            .contains("At most 5"));
    }

    #[test]
    fn a_prompt_naming_a_character_is_caught_as_a_whole_word() {
        let dir = scratch();
        create(dir.path(), character("maya", Origin::Owner)).unwrap();
        create(dir.path(), character("jo", Origin::Owner)).unwrap();
        create(dir.path(), character("theo", Origin::ModelClean)).unwrap();
        let (lib, _) = Library::load(dir.path());
        assert_eq!(named_in(&lib, "Maya and John on a bench"), ["maya"]);
        create(dir.path(), character("john", Origin::Owner)).unwrap();
        let (lib, _) = Library::load(dir.path());
        // First mention first, whatever the library's order; once each.
        assert_eq!(
            named_in(&lib, "Maya and John, then Maya again"),
            ["maya", "john"]
        );
        assert_eq!(named_in(&lib, "John beside Maya"), ["john", "maya"]);
        // Whole words only; a candidate is not a character yet.
        assert!(named_in(&lib, "a mayan temple, joyful, theo").is_empty());
    }

    #[test]
    fn proposal_origin_follows_taint_and_fails_closed() {
        use crate::agent::Taint;
        assert_eq!(Origin::of_proposal(None), Origin::ModelUntrusted);
        let clean = Taint::default();
        assert_eq!(Origin::of_proposal(Some(&clean)), Origin::ModelClean);
        let dirty = Taint {
            untrusted: true,
            ..Taint::default()
        };
        assert_eq!(Origin::of_proposal(Some(&dirty)), Origin::ModelUntrusted);
    }
}
