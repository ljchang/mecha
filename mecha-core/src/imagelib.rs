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
/// People per scene. E3 held four in one pass; five is unmeasured.
pub const MAX_CAST: usize = 4;
/// `wearing` and `doing`, each.
pub const MAX_CAST_FIELD: usize = 300;
/// Candidates waiting on the owner. A model that proposes in a loop fills a
/// queue nobody reads; past this it is refused.
pub const MAX_PENDING: usize = 50;
/// A portrait larger than this is refused rather than read.
pub const MAX_PORTRAIT_BYTES: u64 = 25 * 1024 * 1024;
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
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("installing {}", path.display()))?;
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
        let (lib, _) = Library::load(dir);
        if lib.candidates().count() >= MAX_PENDING {
            bail!(
                "{MAX_PENDING} candidates are already waiting on the owner; no more can be \
                 proposed until some are approved or rejected"
            );
        }
    }
    let portrait = new
        .portrait
        .as_deref()
        .map(|b| store_blob(dir, b))
        .transpose()?;
    let kind_dir = dir.join(new.kind.dir());
    std::fs::create_dir_all(&kind_dir)?;
    let entry_dir = kind_dir.join(&new.name);
    match std::fs::create_dir(&entry_dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            bail!("a {} named `{}` already exists", new.kind.label(), new.name)
        }
        Err(e) => return Err(e).context("creating the entry"),
    }
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
    write_entry(dir, &entry)?;
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
    // The owner wrote the new version, so it is theirs whoever wrote the old.
    entry.origin = Origin::Owner;
    entry.status = Status::Approved;
    entry.updated = now();
    write_entry(dir, &entry)?;
    Ok(entry)
}

/// Remove an entry — moved aside under `removed/`, not deleted, because a
/// manifest may still name it. Portraits stay in `blobs/`.
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
    ["no", "one", "two", "three", "four"]
        .get(n)
        .copied()
        .unwrap_or("several")
}

fn missing(lib: &Library, kind: Kind, name: &str) -> String {
    match lib.get(kind, name) {
        Some(e) if e.status == Status::Candidate => format!(
            "The {} `{name}` is waiting for the owner's approval and cannot be used yet.",
            kind.label()
        ),
        _ => format!(
            "No approved {} named `{name}`. Call image_library to see what exists.",
            kind.label()
        ),
    }
}

/// Compile a scene. Errors are sentences for the model.
///
/// The prompt's shape is Qwen-Image 2.1's and each clause is a measurement:
/// one person is "the person in the image" (the rewriter's rule for a single
/// reference); two or more are `<image1>…` left to right with the head count
/// stated (E2, E9: each reference slot tends to become a person); every
/// person carries their stored description beside the pointer (E1) and what
/// they are wearing and doing (E8: a reference supplies its own otherwise).
pub fn compile(
    lib: &Library,
    scene: &str,
    cast: &[CastMember],
    style: Option<&str>,
) -> std::result::Result<Compiled, String> {
    if cast.len() > MAX_CAST {
        return Err(format!(
            "At most {MAX_CAST} people in `cast`, not {}.",
            cast.len()
        ));
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
                "`{name}` appears twice in `cast`; each person is drawn once."
            ));
        }
        let entry = lib
            .get(Kind::Character, &name)
            .filter(|e| e.status == Status::Approved)
            .ok_or_else(|| missing(lib, Kind::Character, &name))?;
        let (wearing, doing) = (member.wearing.trim(), member.doing.trim());
        if wearing.is_empty() || doing.is_empty() {
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

    let scene = scene.trim().trim_end_matches('.');
    let mut prompt = format!("{scene}.");
    match people.len() {
        0 => {}
        1 => prompt.push_str(&format!(
            " {}. Exactly one person in the image. The image is an identity reference only; \
             this is a new image with its own composition, pose and lighting.",
            capitalize(&people[0])
        )),
        n => prompt.push_str(&format!(
            " From left to right: {}. Exactly {} people in the image. All images serve as \
             identity sources only; each person appears exactly once.",
            people.join("; "),
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
/// characters up, then wrote their descriptions into the prompt and left
/// `cast` out — and from words alone they came out as two strangers (ArcFace
/// 0.02–0.17 against their portraits; E1 measured the same, 0.33).
pub fn named_in(lib: &Library, prompt: &str) -> Vec<String> {
    let words: std::collections::BTreeSet<String> = prompt
        .split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    lib.approved()
        .filter(|e| e.kind == Kind::Character && words.contains(&e.name))
        .map(|e| e.name.clone())
        .collect()
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
        let c = compile(&lib, "a diner at night.", &[member("maya")], None).unwrap();
        assert!(c
            .prompt
            .starts_with("a diner at night. The person in the image (maya, a person"));
        assert!(c.prompt.contains("wearing a yellow raincoat, laughing"));
        assert!(c.prompt.contains("Exactly one person"));
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
        let c = compile(&lib, "a diner booth", &cast, Some("noir")).unwrap();
        let first = c.prompt.find("<image1> (maya").unwrap();
        let second = c.prompt.find("<image2> (john").unwrap();
        assert!(first < second);
        assert!(c.prompt.contains("<image3> (priya"));
        assert!(c.prompt.contains("Exactly three people"));
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
    fn candidates_never_compile() {
        let dir = scratch();
        create(dir.path(), character("maya", Origin::ModelClean)).unwrap();
        let (lib, _) = Library::load(dir.path());
        let why = compile(&lib, "x", &[member("maya")], None).unwrap_err();
        assert!(why.contains("waiting for the owner's approval"), "{why}");
    }

    #[test]
    fn wearing_and_doing_are_required() {
        let dir = scratch();
        create(dir.path(), character("maya", Origin::Owner)).unwrap();
        let (lib, _) = Library::load(dir.path());
        let mut m = member("maya");
        m.wearing = " ".into();
        let why = compile(&lib, "x", &[m], None).unwrap_err();
        assert!(why.contains("wearing"), "{why}");
    }

    #[test]
    fn a_person_is_drawn_once_and_at_most_four() {
        let dir = scratch();
        for n in ["a", "b", "c", "d", "e"] {
            create(dir.path(), character(n, Origin::Owner)).unwrap();
        }
        let (lib, _) = Library::load(dir.path());
        assert!(compile(&lib, "x", &[member("a"), member("a")], None)
            .unwrap_err()
            .contains("twice"));
        let five: Vec<_> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|n| member(n))
            .collect();
        assert!(compile(&lib, "x", &five, None)
            .unwrap_err()
            .contains("At most 4"));
    }

    #[test]
    fn a_prompt_naming_a_character_is_caught_as_a_whole_word() {
        let dir = scratch();
        create(dir.path(), character("maya", Origin::Owner)).unwrap();
        create(dir.path(), character("jo", Origin::Owner)).unwrap();
        create(dir.path(), character("theo", Origin::ModelClean)).unwrap();
        let (lib, _) = Library::load(dir.path());
        assert_eq!(named_in(&lib, "Maya and John on a bench"), ["maya"]);
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
