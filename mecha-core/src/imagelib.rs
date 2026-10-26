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
/// A location's text (`IMAGE-COMPILER-DESIGN.md` §9.3): it is added to a
/// scene's setting once, so it is held to a description's length.
pub const MAX_LOCATION_TEXT: usize = 400;
/// An outfit's text: it fills a cast member's `wearing`, so it is held to
/// `wearing`'s own length (§9.4, `MAX_CAST_FIELD`).
pub const MAX_OUTFIT_TEXT: usize = MAX_CAST_FIELD;
/// People per new picture. E3 held four in one pass; five is the owner's
/// provisional ceiling (IMAGE-DESIGN.md §5.2 and §11, C5).
pub const MAX_CAST: usize = 5;
/// `wearing` and `doing`, each.
pub const MAX_CAST_FIELD: usize = 300;
/// People in a scene who are no library character — a waiter, a crowd's
/// front row. They carry no reference, so the reference ceilings
/// ([`MAX_CAST`], [`MAX_BOUND_IMAGES`]) do not bound them; four is what has been asked of the model alongside
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
/// A reference's label: "left-forearm tattoo", not a paragraph. Only a
/// `detail`'s is ever sent, appended to the description it rides beside
/// (`IMAGE-COMPILER-DESIGN.md` §9.3).
pub const MAX_LABEL: usize = 100;
/// Images a new picture carries when each person is bound to two
/// (`IMAGE-COMPILER-DESIGN.md` §9.4): so full-figure references go to a cast
/// of one. E9b passed a cast of two by eye on two seeds, but a cast of two
/// with a body each drew someone twice in 6 of 13 live pictures (2026-10-17),
/// and portraits alone with a closed count took recorded calls from 19 of 21
/// doubled to 2 of 21 (E14); a cast of one with a body drew no one twice.
pub const MAX_BOUND_IMAGES: usize = 2;

/// What an entry is. The directory it lives under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Character,
    Style,
    /// A place: text for the scene's setting and up to three `view`
    /// pictures, which video sends as the setting (V-L, ruled passed by the
    /// owner 2026-10-25). Pictures use its text only, until E13.
    Location,
    /// Clothes: text that fills `wearing`, and a `view` picture of the
    /// garment alone, which video sends as what its wearer has on (the
    /// owner's ruling of 2026-10-25, reopening §9's "outfits stay text" for
    /// video after a garment picture held a swimsuit across a cut that the
    /// words alone did not). Pictures use its text only.
    Outfit,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Character, Kind::Style, Kind::Location, Kind::Outfit];

    fn dir(self) -> &'static str {
        match self {
            Kind::Character => "characters",
            Kind::Style => "styles",
            Kind::Location => "locations",
            Kind::Outfit => "outfits",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Character => "character",
            Kind::Style => "style",
            Kind::Location => "location",
            Kind::Outfit => "outfit",
        }
    }

    /// The label with its article, as a sentence says it: "an outfit", not
    /// "a outfit".
    pub fn with_article(self) -> &'static str {
        match self {
            Kind::Character => "a character",
            Kind::Style => "a style",
            Kind::Location => "a location",
            Kind::Outfit => "an outfit",
        }
    }

    /// Whether an entry of this kind has a portrait: a character must, and
    /// nothing else may.
    pub fn has_portrait(self) -> bool {
        match self {
            Kind::Character => true,
            Kind::Style | Kind::Location | Kind::Outfit => false,
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
    /// Typed pictures beside the portrait (`IMAGE-COMPILER-DESIGN.md` §9.3),
    /// written by the owner, or by a model as §9.3's provenance split allows
    /// (each then carries its `origin`). Last, so the TOML's `[[reference]]`
    /// tables follow every plain field.
    #[serde(default, rename = "reference", skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<Reference>,
    /// Set on load from the directory, never stored.
    #[serde(skip, default = "default_kind")]
    pub kind: Kind,
}

/// What a reference picture is of (`IMAGE-COMPILER-DESIGN.md` §9.3). A closed
/// enum in an on-disk store, so a wire format: a value this build does not
/// know loads as `Unknown`, is shown, and is never sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    /// A better face than the portrait shows; the source of video's crop.
    Face,
    /// Build and proportions, in a neutral, square-on stance.
    FullBody,
    /// A labelled mark: "left-forearm tattoo".
    Detail,
    /// A place's or an outfit's picture, for the kinds still to come.
    View,
    #[serde(other)]
    Unknown,
}

impl RefKind {
    /// Every kind this build can send; `Unknown` is what a later build's
    /// kind reads as, and is no kind anyone may write.
    pub const ALL: [RefKind; 4] = [
        RefKind::Face,
        RefKind::FullBody,
        RefKind::Detail,
        RefKind::View,
    ];

    pub fn label(self) -> &'static str {
        match self {
            RefKind::Face => "face",
            RefKind::FullBody => "full_body",
            RefKind::Detail => "detail",
            RefKind::View => "view",
            RefKind::Unknown => "unknown",
        }
    }

    /// Whether an entry of `kind` may carry this reference. A known kind on
    /// the wrong entry kind reads as unknown: never sent (§9.3).
    pub fn allowed_on(self, kind: Kind) -> bool {
        matches!(
            (kind, self),
            (
                Kind::Character,
                RefKind::Face | RefKind::FullBody | RefKind::Detail
            ) | (Kind::Location | Kind::Outfit, RefKind::View)
        )
    }

    /// How many of this kind one entry may hold (§9.3's store limits).
    pub fn limit(self) -> usize {
        match self {
            RefKind::Face | RefKind::FullBody => 1,
            RefKind::Detail => 4,
            RefKind::View => 3,
            RefKind::Unknown => 0,
        }
    }
}

/// A character's face picture, [`Library::face_picture`]: its bytes, their
/// extension as [`Library::read_portrait`] gives it, and which picture it
/// was.
#[derive(Debug, Clone, PartialEq)]
pub struct FacePicture {
    pub bytes: Vec<u8>,
    pub ext: &'static str,
    /// The character's `face` reference, rather than its portrait.
    pub from_face_reference: bool,
}

impl FacePicture {
    /// Which picture it was, as a sentence names it.
    pub fn which(&self) -> &'static str {
        if self.from_face_reference {
            "face reference"
        } else {
            "portrait"
        }
    }
}

/// One typed reference: a blob under `blobs/`, checked like the portrait.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reference {
    pub kind: RefKind,
    pub blob: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Who attached it, when a model did (the owner's ruling of 2026-10-14:
    /// a clean conversation writes directly). Absent is the owner's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

impl Entry {
    /// The references a generator may be sent, by the entry's own rules: a
    /// kind this build knows and this entry kind allows, within its limit,
    /// in stored order. Anything else is kept and shown, never sent.
    pub fn sendable_references(&self) -> Vec<&Reference> {
        let mut seen: std::collections::HashMap<RefKind, usize> = Default::default();
        self.references
            .iter()
            .filter(|r| r.kind.allowed_on(self.kind))
            .filter(|r| {
                let n = seen.entry(r.kind).or_default();
                *n += 1;
                *n <= r.kind.limit()
            })
            .collect()
    }
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
        Kind::Location => MAX_LOCATION_TEXT,
        Kind::Outfit => MAX_OUTFIT_TEXT,
    };
    if text.trim().is_empty() {
        bail!("{} needs text", kind.with_article());
    }
    if text.chars().count() > cap {
        bail!(
            "{}'s text is capped at {cap} characters",
            kind.with_article()
        );
    }
    if text.chars().any(|c| c.is_control() && c != '\n') {
        bail!(
            "control characters are not allowed in {}'s text",
            kind.with_article()
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
pub fn picture_type(bytes: &[u8]) -> Result<&'static str> {
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
        self.read_checked(blob, &format!("`{}`'s portrait", entry.name))
    }

    /// The picture of a character's face: its `face` reference when it has
    /// one ("a better face than the portrait shows", [`RefKind::Face`]), its
    /// portrait otherwise. Both under the hash check.
    pub fn face_picture(&self, entry: &Entry) -> Result<FacePicture> {
        match entry
            .sendable_references()
            .into_iter()
            .find(|r| r.kind == RefKind::Face)
        {
            Some(face) => {
                let (bytes, ext) = self.read_reference(entry, face)?;
                Ok(FacePicture {
                    bytes,
                    ext,
                    from_face_reference: true,
                })
            }
            None => {
                let (bytes, ext) = self.read_portrait(entry)?;
                Ok(FacePicture {
                    bytes,
                    ext,
                    from_face_reference: false,
                })
            }
        }
    }

    /// A reference's picture, under the portrait's hash check: bytes swapped
    /// under an approved hash are refused, not sent (§9.3).
    pub fn read_reference(
        &self,
        entry: &Entry,
        reference: &Reference,
    ) -> Result<(Vec<u8>, &'static str)> {
        // "Never sent" is a property of the read, not a filter a caller must
        // remember (review of #684): an unknown kind, a kind this entry kind
        // doesn't take, or one past its limit is refused here.
        // By identity, not value: a hand-edited second copy of a kind past
        // its limit equals the first and would pass a value check.
        if !entry
            .sendable_references()
            .iter()
            .any(|r| std::ptr::eq(*r, reference))
        {
            bail!(
                "`{}`'s {} reference is not one it may send",
                entry.name,
                reference.kind.label()
            );
        }
        self.read_checked(
            &reference.blob,
            &format!("`{}`'s {} reference", entry.name, reference.kind.label()),
        )
    }

    /// A picture this entry names, its portrait or any reference, under the
    /// portrait's hash check: for the owner's browsing, which shows a
    /// reference whatever its kind, where [`Library::read_reference`] is the
    /// generators' read and refuses one the entry may not send.
    pub fn read_named(&self, entry: &Entry, blob: &str) -> Result<(Vec<u8>, &'static str)> {
        if entry.portrait.as_deref() == Some(blob) {
            return self.read_portrait(entry);
        }
        if !entry.references.iter().any(|r| r.blob == blob) {
            bail!("`{}` names no picture `{blob}`", entry.name);
        }
        self.read_checked(blob, &format!("`{}`'s reference", entry.name))
    }

    /// [`Library::read_named`]'s picture at most [`THUMB_SIDE`] on its long
    /// side, as a JPEG: what a page shows in a grid or an avatar. Made once
    /// and kept under `thumbs/`, named by the blob, so it is as permanent as
    /// the content-addressed picture it shrinks; derived and regenerable,
    /// like the face crops beside it.
    pub fn thumbnail_named(&self, entry: &Entry, blob: &str) -> Result<Vec<u8>> {
        // Named first, so the cache is never a way around the lookup.
        if entry.portrait.as_deref() != Some(blob)
            && !entry.references.iter().any(|r| r.blob == blob)
        {
            bail!("`{}` names no picture `{blob}`", entry.name);
        }
        let path = thumb_path(&self.dir, blob);
        if let Ok(bytes) = std::fs::read(&path) {
            return Ok(bytes);
        }
        let (bytes, _) = self.read_named(entry, blob)?;
        let jpeg = thumbnail(&bytes)?;
        // A thumbnail that cannot be kept is still shown: the page should
        // not pay for a full disk, only the next request.
        let kept = make_private_dir(path.parent().unwrap_or(&self.dir))
            .and_then(|()| write_atomic_mode(&path, &jpeg, Some(0o600)));
        if let Err(e) = kept {
            tracing::warn!("thumbnail cache: {e:#}");
        }
        Ok(jpeg)
    }

    fn read_checked(&self, blob: &str, what: &str) -> Result<(Vec<u8>, &'static str)> {
        let bytes =
            std::fs::read(self.blob_path(blob)).with_context(|| format!("reading {what}"))?;
        let ext = picture_type(&bytes)?;
        if blob_name(&bytes, ext) != blob {
            bail!("{what} does not match its hash");
        }
        Ok((bytes, ext))
    }
}

/// A derived store's directory and any parent it lacks, owner-only like
/// the library's own files: the face crops and the thumbnails are faces.
pub(crate) fn make_private_dir(dir: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(dir)
        .with_context(|| format!("creating {}", dir.display()))
}

/// The long side of a library thumbnail, in pixels. The largest place one is
/// shown is a library grid tile, about 170 CSS pixels wide on a phone at
/// three device pixels each; an avatar zoomed in by its framing is smaller.
pub const THUMB_SIDE: u32 = 512;

/// The thumbnail cache's generation: bump it when [`THUMB_SIDE`] or the
/// encoding changes, or every picture keeps the thumbnail the old one made.
const THUMB_VERSION: &str = "v1";

/// What the picture route's `size` asks for to get a thumbnail: the
/// generation rides in the URL, so a bump changes the browser's cache key as
/// it does the disk's — a thumbnail is cached `immutable` for a year (review
/// of #735). `web/src/lib/library.js`'s `THUMB_SIZE` must say the same.
pub const THUMB_QUERY: &str = "thumb-v1";

fn thumb_path(dir: &Path, blob: &str) -> PathBuf {
    let stem = blob.split('.').next().unwrap_or(blob);
    dir.join("thumbs")
        .join(THUMB_VERSION)
        .join(format!("{stem}.jpg"))
}

/// A picture as a JPEG at most [`THUMB_SIDE`] on its long side, upright as
/// a browser shows it, any transparency laid over white. A smaller picture
/// keeps its size; only its encoding changes.
pub fn thumbnail(bytes: &[u8]) -> Result<Vec<u8>> {
    let rgba = crate::image::decode_upright(bytes, "the picture")?.to_rgba8();
    // Over white before the resize: a resampler averages a transparent
    // pixel's colour, often black, into its neighbours (review of #735).
    let rgb = image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let over = |c: u8| ((c as u16 * a as u16 + 255 * (255 - a as u16)) / 255) as u8;
        image::Rgb([over(r), over(g), over(b)])
    });
    let rgb = if rgb.width().max(rgb.height()) > THUMB_SIDE {
        image::DynamicImage::ImageRgb8(rgb)
            .thumbnail(THUMB_SIDE, THUMB_SIDE)
            .to_rgb8()
    } else {
        rgb
    };
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85)
        .encode_image(&rgb)
        .context("encoding the thumbnail")?;
    Ok(out)
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
    if kind.has_portrait() && entry.portrait.is_none() {
        bail!("{} without a portrait", kind.with_article());
    }
    // A plain file name, so a hand-edited entry cannot point a reference at
    // anything outside `blobs/`. Every blob the entry names, not only the
    // portrait (§9.3).
    if let Some(blob) = &entry.portrait {
        if !plain(blob) {
            bail!("portrait `{blob}` is not a blob name");
        }
    }
    for r in &entry.references {
        if !plain(&r.blob) {
            bail!(
                "{} reference `{}` is not a blob name",
                r.kind.label(),
                r.blob
            );
        }
        if r.label
            .as_deref()
            .is_some_and(|l| l.chars().count() > MAX_LABEL)
        {
            bail!(
                "a {} reference's label is over {MAX_LABEL} characters",
                r.kind.label()
            );
        }
    }
    Ok(entry)
}

/// A plain file name under `blobs/`: no separator, no leading dot, so a
/// hand-edited record cannot point a picture anywhere else.
fn plain(blob: &str) -> bool {
    !(blob.contains('/') || blob.contains('\\') || blob.starts_with('.'))
}

/// The blob name a picture's bytes would be stored under, for a reader that
/// checks bytes against a name outside this module.
pub fn blob_name_of(bytes: &[u8], ext: &str) -> String {
    blob_name(bytes, ext)
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
            "the picture is {} MB; library pictures are capped at {} MB",
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
    match (new.kind.has_portrait(), &new.portrait) {
        (true, None) => bail!("{} needs a portrait", new.kind.with_article()),
        (false, Some(_)) => bail!("{} has no portrait", new.kind.with_article()),
        (true, Some(_)) | (false, None) => {}
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
            bail!(
                "{} named `{}` already exists",
                new.kind.with_article(),
                new.name
            )
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
        // A new entry carries none: the owner adds references to an approved
        // entry (`add_reference`), so a proposal can never bring any.
        references: Vec::new(),
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
        if !kind.has_portrait() {
            bail!("{} has no portrait", kind.with_article());
        }
        entry.portrait = Some(store_blob(dir, &bytes)?);
        entry.source_seed = source_seed;
    } else if source_seed.is_some() {
        if !kind.has_portrait() {
            bail!("{} has no portrait, so no seed", kind.with_article());
        }
        entry.source_seed = source_seed;
    }
    if entry.text == old.text
        && entry.portrait == old.portrait
        && entry.source_seed == old.source_seed
    {
        bail!("nothing to change");
    }
    // Only a rewritten text is the owner's. A new portrait or seed is not a
    // reading of the text, so it must not approve a candidate or relabel its
    // provenance — that would launder a model's text past `approve`'s
    // untrusted-text question (found on review of #383).
    if entry.text != old.text {
        entry.origin = Origin::Owner;
        entry.status = Status::Approved;
    }
    // One place cuts a version (review of #684).
    new_version(dir, old, entry)
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
            forget_blob(dir, blob);
        }
    }
    Ok(())
}

/// A blob nothing names any more, gone with every thumbnail made of it: a
/// candidate's portrait is shown on the candidates pane, so a rejected one
/// left its thumbnail behind with nothing naming it (review of #736).
fn forget_blob(dir: &Path, blob: &str) {
    let _ = std::fs::remove_file(dir.join("blobs").join(blob));
    let stem = blob.split('.').next().unwrap_or(blob);
    for generation in std::fs::read_dir(dir.join("thumbs"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let _ = std::fs::remove_file(generation.path().join(format!("{stem}.jpg")));
    }
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
    // The whole store, not a list of the directories this build knows: a
    // delete must be fail-closed for a kind a later build adds (§9.3). The
    // blobs themselves are skipped, being no entry.
    let Ok(items) = std::fs::read_dir(dir) else {
        return dir.exists();
    };
    for item in items {
        let Ok(item) = item else { return true };
        let path = item.path();
        if path.file_name().is_some_and(|n| n == "blobs") {
            continue;
        }
        let named = if path.is_dir() {
            walk(&path, blob).unwrap_or(true)
        } else {
            path.extension().is_some_and(|e| e == "toml")
                && std::fs::read_to_string(&path).map_or(true, |t| t.contains(blob))
        };
        if named {
            return true;
        }
    }
    false
}

/// Attach a typed reference to an approved character: the owner's write, a
/// new version with the old one kept in its history (§9.3). A candidate is
/// refused, so a rejected one can never strand reference blobs.
pub fn add_reference(
    dir: &Path,
    name: &str,
    kind: RefKind,
    bytes: &[u8],
    label: Option<String>,
) -> Result<Entry> {
    add_reference_as(dir, name, kind, bytes, label, None)
}

/// [`add_reference`], recording who attached it: `None` for the owner, a
/// model's `Origin` when a clean conversation wrote it (§9.3 as amended
/// 2026-10-14). The store's rules are the same for both.
pub fn add_reference_as(
    dir: &Path,
    name: &str,
    kind: RefKind,
    bytes: &[u8],
    label: Option<String>,
    origin: Option<Origin>,
) -> Result<Entry> {
    add_reference_to(dir, Kind::Character, name, kind, bytes, label, origin)
}

/// The reference kinds an entry of `entry_kind` takes, as a refusal says them.
fn allowed_references(entry_kind: Kind) -> &'static str {
    match entry_kind {
        Kind::Character => "face, full_body or detail references",
        Kind::Location | Kind::Outfit => "view references",
        Kind::Style => "no references",
    }
}

/// [`add_reference_as`] on an entry of any kind: a location's `view`
/// pictures go on this way (§9.3).
pub fn add_reference_to(
    dir: &Path,
    entry_kind: Kind,
    name: &str,
    kind: RefKind,
    bytes: &[u8],
    label: Option<String>,
    origin: Option<Origin>,
) -> Result<Entry> {
    let old = current(dir, entry_kind, name)?;
    if old.status != Status::Approved {
        bail!("`{name}` is a candidate; approve it before adding references");
    }
    if !kind.allowed_on(entry_kind) {
        bail!(
            "{} takes {}, not {}",
            entry_kind.with_article(),
            allowed_references(entry_kind),
            kind.label()
        );
    }
    let label = label
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty());
    if kind == RefKind::Detail && label.is_none() {
        bail!("a detail needs a label saying what it shows, e.g. \"left-forearm tattoo\"");
    }
    if label
        .as_deref()
        .is_some_and(|l| l.chars().count() > MAX_LABEL)
    {
        bail!("a label is capped at {MAX_LABEL} characters");
    }
    let held = old.references.iter().filter(|r| r.kind == kind).count();
    if held >= kind.limit() {
        bail!(
            "`{name}` already has {} {} reference{}; remove one first",
            held,
            kind.label(),
            if held == 1 { "" } else { "s" }
        );
    }
    // Two of one kind under one label could never be told apart again:
    // `remove_reference` names a reference by its kind and label (review of
    // #684). Checked before the picture is stored, so a refusal leaves none.
    if old
        .references
        .iter()
        .any(|r| r.kind == kind && r.label == label)
    {
        bail!(
            "`{name}` already has a {} reference{}; use another label",
            kind.label(),
            label
                .as_deref()
                .map(|l| format!(" labelled \"{l}\""))
                .unwrap_or_default()
        );
    }
    let blob = store_blob(dir, bytes)?;
    if old.portrait.as_deref() == Some(blob.as_str())
        || old.references.iter().any(|r| r.blob == blob)
    {
        bail!("`{name}` already holds that picture");
    }
    let mut entry = old.clone();
    entry.references.push(Reference {
        kind,
        blob,
        label,
        origin,
    });
    new_version(dir, old, entry)
}

/// A model's reference write waiting on the owner (§9.3, the owner's ruling
/// of 2026-10-14): what a conversation that has read outside content asks,
/// and what an incognito chat stages, locked. Kept under `pending/` at the
/// store root, so its picture is protected by [`blob_referenced`]'s walk, and
/// never loaded as an entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingReference {
    pub id: String,
    pub name: String,
    pub kind: RefKind,
    #[serde(default)]
    pub action: PendingAction,
    /// The picture to attach; absent for a removal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub origin: Origin,
    /// Staged from an incognito chat: hidden behind the library lock.
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub created: String,
}

/// What a pending reference asks. A closed enum in a store, so an unknown
/// one loads as `Unknown` and is never applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingAction {
    Add,
    Remove,
    /// Unrecognised or missing: a truncated or hand-edited file degrades to
    /// this, never to an add (review of #692). This build always writes it.
    #[default]
    #[serde(other)]
    Unknown,
}

fn pending_dir(dir: &Path) -> PathBuf {
    dir.join("pending")
}

/// Every pending reference, oldest first, with the files that would not load.
pub fn pending_references(dir: &Path) -> (Vec<PendingReference>, Vec<LoadError>) {
    let mut found = Vec::new();
    let mut errors = Vec::new();
    let Ok(items) = std::fs::read_dir(pending_dir(dir)) else {
        return (found, errors);
    };
    for item in items.flatten() {
        let path = item.path();
        if path.extension().is_none_or(|e| e != "toml") {
            continue;
        }
        let read = std::fs::read_to_string(&path)
            .map_err(anyhow::Error::from)
            .and_then(|raw| toml::from_str::<PendingReference>(&raw).map_err(Into::into))
            .and_then(|p| {
                // The file is named by its id, and a blob by its hash: a
                // hand-edited one cannot point anywhere else.
                if path.file_stem().and_then(|s| s.to_str()) != Some(p.id.as_str()) {
                    bail!("its id does not match its file name");
                }
                if p.blob.as_deref().is_some_and(|b| !plain(b)) {
                    bail!("its picture is not a blob name");
                }
                Ok(p)
            });
        match read {
            Ok(p) => found.push(p),
            Err(e) => errors.push(LoadError {
                path,
                why: format!("{e:#}"),
            }),
        }
    }
    found.sort_by(|a, b| a.created.cmp(&b.created).then(a.id.cmp(&b.id)));
    (found, errors)
}

/// Stage a model's reference write for the owner. Checked as the write would
/// be — an approved character, a kind it takes, a label where one is needed —
/// so the owner is never asked to approve what the store would refuse; the
/// limit is checked again at approval, since the character may change first.
#[allow(clippy::too_many_arguments)]
pub fn propose_reference(
    dir: &Path,
    name: &str,
    kind: RefKind,
    action: PendingAction,
    bytes: Option<&[u8]>,
    label: Option<String>,
    origin: Origin,
    locked: bool,
) -> Result<PendingReference> {
    let entry = current(dir, Kind::Character, name)?;
    if entry.status != Status::Approved {
        bail!("`{name}` is a candidate; it takes references once the owner approves it");
    }
    if !kind.allowed_on(Kind::Character) {
        bail!("a character takes face, full_body or detail references");
    }
    let label = label
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty());
    if label
        .as_deref()
        .is_some_and(|l| l.chars().count() > MAX_LABEL)
    {
        bail!("a label is capped at {MAX_LABEL} characters");
    }
    // Unreadable files count: an unknown is never room on a resource bound.
    let (pending, unreadable) = pending_references(dir);
    if pending.len() + unreadable.len() >= MAX_PENDING {
        bail!("{MAX_PENDING} reference changes are already waiting on the owner");
    }
    let blob = match (action, bytes) {
        (PendingAction::Add, Some(bytes)) => {
            if kind == RefKind::Detail && label.is_none() {
                bail!("a detail needs a label saying what it shows, e.g. \"left-forearm tattoo\"");
            }
            // The write's own refusals, before the picture is stored, so the
            // owner is never shown an Approve that cannot succeed and a
            // doomed proposal leaves no blob (review of #692). Approval
            // checks again: the character may change meanwhile.
            let held = entry.references.iter().filter(|r| r.kind == kind).count();
            if held >= kind.limit() {
                bail!(
                    "`{name}` already has {held} {} reference{}; remove one first",
                    kind.label(),
                    if held == 1 { "" } else { "s" }
                );
            }
            if entry
                .references
                .iter()
                .any(|r| r.kind == kind && r.label == label)
            {
                bail!(
                    "`{name}` already has a {} reference{}; use another label",
                    kind.label(),
                    label
                        .as_deref()
                        .map(|l| format!(" labelled \"{l}\""))
                        .unwrap_or_default()
                );
            }
            // A picture the character already holds can never be added:
            // refused here, by its would-be name, before anything is stored
            // (review of #692).
            let would_be = blob_name(bytes, picture_type(bytes)?);
            if entry.portrait.as_deref() == Some(would_be.as_str())
                || entry.references.iter().any(|r| r.blob == would_be)
            {
                bail!("`{name}` already holds that picture");
            }
            // A proposal's bytes stay on disk until the owner rules, from a
            // conversation the owner has not seen: the proposal cap, as for
            // a proposed portrait.
            if bytes.len() as u64 > MAX_PROPOSED_PORTRAIT_BYTES {
                bail!(
                    "a proposed picture is capped at {} MB",
                    MAX_PROPOSED_PORTRAIT_BYTES / (1024 * 1024)
                );
            }
            Some(store_blob(dir, bytes)?)
        }
        (PendingAction::Add, None) => bail!("adding a reference needs its picture"),
        (PendingAction::Remove, _) => {
            let matching = entry
                .references
                .iter()
                .filter(|r| {
                    r.kind == kind && label.as_ref().is_none_or(|l| r.label.as_ref() == Some(l))
                })
                .count();
            if matching != 1 {
                bail!(
                    "`{name}` has {} {} reference{} matching that; a removal names exactly one",
                    matching,
                    kind.label(),
                    if matching == 1 { "" } else { "s" }
                );
            }
            None
        }
        (PendingAction::Unknown, _) => bail!("unknown reference action"),
    };
    let proposal = PendingReference {
        id: uuid::Uuid::new_v4().simple().to_string(),
        name: entry.name.clone(),
        kind,
        action,
        blob,
        label,
        origin,
        locked,
        created: now(),
    };
    let dir_p = pending_dir(dir);
    std::fs::create_dir_all(&dir_p)?;
    write_atomic(
        &dir_p.join(format!("{}.toml", proposal.id)),
        toml::to_string_pretty(&proposal)?.as_bytes(),
    )?;
    Ok(proposal)
}

fn take_pending(dir: &Path, id: &str) -> Result<PendingReference> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("not a pending reference id");
    }
    let (pending, _) = pending_references(dir);
    pending
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| anyhow!("no pending reference `{id}`"))
}

/// The owner approves a pending reference: applied as the write would have
/// been, recording the model's origin, and the pending file removed. A
/// refusal (a limit reached meanwhile, say) leaves it pending, said.
pub fn approve_reference(dir: &Path, id: &str) -> Result<Entry> {
    let p = take_pending(dir, id)?;
    let entry = match p.action {
        PendingAction::Add => {
            let blob = p
                .blob
                .as_deref()
                .ok_or_else(|| anyhow!("the pending reference has no picture"))?;
            let bytes = std::fs::read(dir.join("blobs").join(blob))
                .with_context(|| format!("reading the pending picture {blob}"))?;
            let ext = picture_type(&bytes)?;
            if blob_name(&bytes, ext) != blob {
                bail!("the pending picture does not match its hash");
            }
            add_reference_as(
                dir,
                &p.name,
                p.kind,
                &bytes,
                p.label.clone(),
                Some(p.origin),
            )?
        }
        PendingAction::Remove => {
            // A kind this build does not know, or one a character does not
            // take, is shown and kept, never removed on a model's say-so.
            if !p.kind.allowed_on(Kind::Character) {
                bail!("this build cannot apply that pending change");
            }
            remove_reference(dir, &p.name, p.kind, p.label.as_deref())?
        }
        PendingAction::Unknown => bail!("this build cannot apply that pending change"),
    };
    std::fs::remove_file(pending_dir(dir).join(format!("{id}.toml")))?;
    Ok(entry)
}

/// The owner rejects a pending reference: the file and, once nothing names
/// it, its picture are deleted, as a rejected candidate's portrait is.
pub fn reject_reference(dir: &Path, id: &str) -> Result<()> {
    let p = take_pending(dir, id)?;
    std::fs::remove_file(pending_dir(dir).join(format!("{id}.toml")))?;
    if let Some(blob) = &p.blob {
        if !blob_referenced(dir, blob) {
            forget_blob(dir, blob);
        }
    }
    Ok(())
}

/// Take a reference off a character: a new version, the picture kept in
/// `blobs/` for the history and any manifest that names it. With more than
/// one of the kind, `label` says which.
pub fn remove_reference(
    dir: &Path,
    name: &str,
    kind: RefKind,
    label: Option<&str>,
) -> Result<Entry> {
    remove_reference_from(dir, Kind::Character, name, kind, label)
}

/// [`remove_reference`] on an entry of any kind.
pub fn remove_reference_from(
    dir: &Path,
    entry_kind: Kind,
    name: &str,
    kind: RefKind,
    label: Option<&str>,
) -> Result<Entry> {
    let old = current(dir, entry_kind, name)?;
    // Stored labels are trimmed on the way in, so the lookup is too.
    let label = label.map(str::trim).filter(|l| !l.is_empty());
    let matching: Vec<usize> = old
        .references
        .iter()
        .enumerate()
        .filter(|(_, r)| r.kind == kind && label.is_none_or(|l| r.label.as_deref() == Some(l)))
        .map(|(i, _)| i)
        .collect();
    // With no label and several of the kind, the unlabelled one is meant
    // when there is exactly one: a location's first view and its `view 2`
    // left the first unremovable from the page, which sends no label for it
    // (review of #743).
    let unlabelled: Vec<usize> = matching
        .iter()
        .copied()
        .filter(|&i| old.references[i].label.is_none())
        .collect();
    let matching = match (label, matching.len(), unlabelled.as_slice()) {
        (None, n, [only]) if n > 1 => vec![*only],
        _ => matching,
    };
    let index = match matching.as_slice() {
        [one] => *one,
        [] => bail!(
            "`{name}` has no {} reference{}",
            kind.label(),
            label
                .map(|l| format!(" labelled \"{l}\""))
                .unwrap_or_default()
        ),
        _ => bail!(
            "`{name}` has several {} references; pass --label to say which",
            kind.label()
        ),
    };
    let mut entry = old.clone();
    entry.references.remove(index);
    new_version(dir, old, entry)
}

/// Write `entry` as the version after `old`, keeping `old` in the history.
/// The entry's text is untouched, so its origin and status stand.
fn new_version(dir: &Path, old: Entry, mut entry: Entry) -> Result<Entry> {
    let history = entry_dir(dir, old.kind, &old.name).join("history");
    std::fs::create_dir_all(&history)?;
    write_atomic(
        &history.join(format!("v{}.toml", old.version)),
        toml::to_string_pretty(&old)?.as_bytes(),
    )?;
    entry.version = old.version + 1;
    entry.updated = now();
    write_entry(dir, &entry)?;
    Ok(entry)
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
    // A removed character's staged reference changes can never apply: they
    // go with it, and their pictures once nothing names them, rather than
    // holding a `MAX_PENDING` slot and a blob forever (review of #692).
    if kind == Kind::Character {
        for p in pending_references(dir)
            .0
            .into_iter()
            .filter(|p| p.name == name)
        {
            reject_reference(dir, &p.id)?;
        }
    }
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
    use argon2::PasswordHasher;
    if password.chars().count() < MIN_LOCK_PASSWORD {
        bail!("the lock password needs at least {MIN_LOCK_PASSWORD} characters");
    }
    // The salt is argon2's own: sixteen bytes from the OS generator
    // (`getrandom`, a default of argon2 0.6).
    let hash = argon2::Argon2::default()
        .hash_password(password.as_bytes())
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
    use argon2::{PasswordHash, PasswordVerifier};
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

/// Which library voices are locked (owner, 2026-10-15: "lock voices just
/// like images and personas"). A voice is a clip in the TTS's own directory
/// with no record of ours to carry a flag, so the flags live here, beside
/// the password and autolock that show them — one lock over the three
/// stores. As for an entry, the lock hides a voice while browsing and never
/// withholds it: a call, a persona bound to it and Listen still speak in it.
const VOICE_LOCKS_FILE: &str = "voices.toml";

#[derive(Serialize, Deserialize, Default)]
struct VoiceLocksFile {
    #[serde(default)]
    locked: std::collections::BTreeSet<String>,
}

/// Serializes [`set_voice_locked`]'s read-modify-write within a process:
/// two taps on two voices must not each write a list missing the other.
static VOICE_LOCKS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The locked voices' names. Empty when nothing was ever locked; `Err` when
/// the file cannot be read — a browsing surface then shows no voice to a
/// locked page, never every voice (unknown is never clean).
pub fn locked_voices(dir: &Path) -> Result<std::collections::BTreeSet<String>> {
    let path = dir.join(VOICE_LOCKS_FILE);
    if !path.is_file() {
        return Ok(Default::default());
    }
    let text = std::fs::read_to_string(&path).context("reading the voice locks")?;
    let file: VoiceLocksFile = toml::from_str(&text).context("reading the voice locks")?;
    Ok(file.locked)
}

/// Lock or unlock one voice by name, answering whether anything changed. The
/// name is the voice's, as the TTS lists it — whether it exists is the
/// caller's to know (the TTS's list is not this crate's), so only its shape
/// is checked here.
pub fn set_voice_locked(dir: &Path, name: &str, locked: bool) -> Result<bool> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        bail!("`{name}` is not a voice name");
    }
    let _held = VOICE_LOCKS.lock().unwrap_or_else(|p| p.into_inner());
    // A damaged file is not rewritten from nothing: that would unlock every
    // voice it held.
    let mut set = locked_voices(dir)?;
    let changed = if locked {
        set.insert(name.to_string())
    } else {
        set.remove(name)
    };
    if !changed {
        return Ok(false);
    }
    std::fs::create_dir_all(dir)?;
    let text = toml::to_string_pretty(&VoiceLocksFile { locked: set })?;
    write_atomic_mode(&dir.join(VOICE_LOCKS_FILE), text.as_bytes(), Some(0o600))
        .context("writing the voice locks")?;
    Ok(true)
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
    /// The act already says what they have on, so the prompt states no
    /// outfit beside it (`imagegen::clothes_in_act`): the record keeps it.
    pub clothes_in_act: bool,
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
    /// The full-figure reference sent beside the portrait, when one was
    /// (§9.4): the entry's version says it was attached, not that it went.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_body: Option<String>,
}

/// A scene, compiled.
#[derive(Debug)]
pub struct Compiled {
    pub prompt: String,
    /// `(name, bytes, ext)` in `<imageN>` order; a full figure's name is
    /// `full_body:<name>`, after its person's portrait.
    pub references: Vec<(String, Vec<u8>, &'static str)>,
    pub used: Vec<Used>,
    /// Seeds that drew the cast's portraits, when known.
    pub source_seeds: Vec<u64>,
    /// Full-figure references attached and not sent, said in counts: the
    /// labels stay in the library (§9.4).
    pub notes: Vec<String>,
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
        // Only what the call named, never what else the library holds:
        // the roster is the owner's (`image_library` declares it private,
        // and a locked entry is never named where there is no unlock token,
        // #385), and never a tool to call, which a persona chat does not
        // have (2026-10-08; review of #603).
        _ => format!("No approved {} named `{name}`.", kind.label()),
    }
}

/// The approved, unlocked styles, by name, as one sentence for a model, or
/// `None` when there are none. Style names are not private (the owner's
/// ruling, 2026-10-08); a locked style is the owner's way to hide one, so it
/// is left out, without a count. Characters are never listed: their names
/// are the owner's.
pub(crate) fn styles_to_name(lib: &Library) -> Option<String> {
    let names: Vec<String> = style_names(lib).iter().map(|n| format!("`{n}`")).collect();
    (!names.is_empty()).then(|| format!("Styles you can name: {}", names.join(", ")))
}

/// `words` spelled as a library name could be: trimmed, lowercased, and each
/// run of spaces, underscores or hyphens a single hyphen. Names allow only
/// lowercase letters, digits and hyphens, so this can only find the name
/// meant.
pub fn spelled_as_name(words: &str) -> String {
    words
        .trim()
        .to_lowercase()
        .split(|c: char| c.is_whitespace() || c == '_' || c == '-')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// The approved, unlocked styles' names, in the library's order: what a
/// model may be offered (see [`styles_to_name`]).
pub fn style_names(lib: &Library) -> Vec<String> {
    lib.all()
        .iter()
        .filter(|e| e.kind == Kind::Style && e.status == Status::Approved && !e.locked)
        .map(|e| e.name.clone())
        .collect()
}

/// Approved, unlocked locations, sorted: the names a model is offered, as
/// [`style_names`] offers styles (§9.5, L5). A locked one still resolves by
/// name; the lock is for browsing.
pub fn location_names(lib: &Library) -> Vec<String> {
    lib.all()
        .iter()
        .filter(|e| e.kind == Kind::Location && e.status == Status::Approved && !e.locked)
        .map(|e| e.name.clone())
        .collect()
}

/// Approved, unlocked outfits, sorted, as [`location_names`] (§9.5, L5).
pub fn outfit_names(lib: &Library) -> Vec<String> {
    lib.all()
        .iter()
        .filter(|e| e.kind == Kind::Outfit && e.status == Status::Approved && !e.locked)
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
/// they are wearing and doing (E8: a reference supplies its own otherwise),
/// except that `doing` is empty when the scene says what its people do
/// together: a pose beside that drew a person twice (`imagegen::doing_words`
/// says a plain pose whenever the scene does not).
pub fn compile(
    lib: &Library,
    scene: &str,
    cast: &[CastMember],
    extras: &[String],
    style: Option<&str>,
) -> std::result::Result<Compiled, String> {
    compile_with(lib, scene, cast, extras, style, true)
}

/// [`compile`], with `bodies` false when the picture must be drawn from
/// portraits alone: one planned in layers, whose cutouts were measured with
/// a portrait each and whose fallback must draw what they would have.
///
/// With `bodies`, a cast of one with a full body attached is bound to two
/// images, portrait then full figure ([`MAX_BOUND_IMAGES`]); a larger cast
/// is drawn from portraits, and the notes say why (§9.4, E14: four images
/// for two people drew someone twice). The binding is E9b's measured
/// wording, with the name for its pronoun.
pub fn compile_with(
    lib: &Library,
    scene: &str,
    cast: &[CastMember],
    extras: &[String],
    style: Option<&str>,
    bodies: bool,
) -> std::result::Result<Compiled, String> {
    if extras.len() > MAX_EXTRAS {
        return Err(format!(
            "At most {MAX_EXTRAS} people who are not library characters fit one new picture, \
             not {}.",
            extras.len()
        ));
    }
    // Kept whole for the faces-only retry past the prompt cap.
    let given_extras = extras;
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
    // Everyone is checked and read before anyone is numbered: whether the
    // bodies go decides every person's `<imageN>`.
    struct Member<'a> {
        name: String,
        entry: &'a Entry,
        wearing: &'a str,
        doing: &'a str,
        clothes_in_act: bool,
        portrait: (Vec<u8>, &'static str),
        body: Option<&'a Reference>,
    }
    let mut members = Vec::with_capacity(cast.len());
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
        // A placeholder copied from a refusal's example is no answer. An
        // empty `doing` is a scene that says what its people do together
        // (`imagegen::doing_words` says a plain pose otherwise).
        if blank(wearing) || (!doing.is_empty() && blank(doing)) {
            return Err(format!(
                "`{name}` needs `wearing`, and `doing` unless the scene says what its people \
                 do together: a reference supplies its own outfit and pose when the scene does \
                 not say."
            ));
        }
        if wearing.chars().count() > MAX_CAST_FIELD || doing.chars().count() > MAX_CAST_FIELD {
            return Err(format!(
                "`wearing` and `doing` are capped at {MAX_CAST_FIELD} characters each."
            ));
        }
        let portrait = lib
            .read_portrait(entry)
            .map_err(|e| format!("`{name}`'s portrait cannot be read: {e:#}."))?;
        members.push(Member {
            name,
            entry,
            wearing,
            doing,
            clothes_in_act: member.clothes_in_act,
            portrait,
            body: entry
                .sendable_references()
                .into_iter()
                .find(|r| r.kind == RefKind::FullBody),
        });
    }

    // A cast of one only (§9.4, E14), and said in counts, never labels:
    // `image_generate` declares no private data, so its result names nothing
    // the library holds beyond what the call already named.
    let mut notes = Vec::new();
    let n = members.len();
    let attached = members.iter().filter(|m| m.body.is_some()).count();
    let mut bound: Vec<Option<(Vec<u8>, &'static str)>> = vec![None; n];
    if attached > 0 {
        let not_sent = "Full-figure references were not sent";
        if !bodies {
            notes.push(format!(
                "{not_sent}: a picture planned in layers is drawn from portraits."
            ));
        // The cap first: past it, attaching more cannot help, so the note
        // must not suggest it (review of #685).
        } else if 2 * n > MAX_BOUND_IMAGES {
            notes.push(format!(
                "{not_sent}: they go only to a picture of one person, and this one has {}.",
                number(n)
            ));
        } else {
            // Under the cap the cast is one person with a body today; a
            // member without one, should the cap move, is not sent rather
            // than a panic inside a tool call (review of #718).
            let read: Vec<_> = members
                .iter()
                .map(|m| {
                    let Some(body) = m.body else { return Err(()) };
                    lib.read_reference(m.entry, body).map_err(|e| {
                        tracing::warn!("imagelib: {e:#}");
                    })
                })
                .collect();
            if read.iter().all(Result::is_ok) {
                bound = read.into_iter().map(Result::ok).collect();
            } else {
                notes.push(format!("{not_sent}: it could not be read."));
            }
        }
    }
    let both = bound.iter().all(Option::is_some) && n > 0;

    let mut refs = Vec::with_capacity(n * if both { 2 } else { 1 });
    let mut used = Vec::new();
    let mut source_seeds = Vec::new();
    let mut people = Vec::with_capacity(n);
    let mut bindings = Vec::new();
    let mut figures = Vec::new();
    let mut names = Vec::with_capacity(n);
    for (i, (m, body)) in members.into_iter().zip(bound).enumerate() {
        let shown = crate::imagegen::capitalized(&m.name);
        names.push(shown.clone());
        let who = match &body {
            Some(_) => {
                let (face, figure) = (refs.len() + 1, refs.len() + 2);
                bindings.push(format!(
                    "<image{face}> and <image{figure}> show one person, {shown}: \
                     <image{face}> is {shown}'s portrait, <image{figure}> {shown}'s full figure."
                ));
                figures.push(format!("<image{figure}>"));
                format!("the person shown in <image{face}> and <image{figure}>")
            }
            None if n == 1 => "the person in the image".to_string(),
            None => format!("the person from <image{}>", i + 1),
        };
        source_seeds.extend(m.entry.source_seed);
        used.push(Used {
            kind: Kind::Character,
            name: m.name.clone(),
            version: m.entry.version,
            portrait: m.entry.portrait.clone(),
            full_body: body.as_ref().and(m.body.map(|r| r.blob.clone())),
        });
        let (bytes, ext) = m.portrait;
        refs.push((m.name.clone(), bytes, ext));
        if let Some((bytes, ext)) = body {
            refs.push((format!("full_body:{}", m.name), bytes, ext));
        }
        let mut line = format!("{who} ({})", m.entry.text.trim().trim_end_matches('.'));
        if !m.clothes_in_act {
            line.push_str(&format!(", wearing {}", m.wearing));
        }
        if !m.doing.is_empty() {
            line.push_str(&format!(", {}", m.doing));
        }
        people.push(line);
    }
    // E9b's binding, then its clothing clause with the colour taken out: the
    // scene states what each person wears, and a figure's own outfit is
    // only its stance's.
    let bound_words = if both {
        format!(
            " {} The clothing in {} is not worn here.",
            bindings.join(" "),
            join_and(&figures)
        )
    } else {
        String::new()
    };
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
                full_body: None,
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
    // Three measured rules. With extras, the head count counts everyone
    // (E11): "Exactly three people" beside a scene with a waiter pushed him
    // into the background, and counted as a new person he stood where the
    // scene put him. One person without extras gets no total (E12): the
    // model does not always use `extras` — a live run wrote the waiter into
    // the prose — and there "Exactly two people" erased him outright, while
    // "appears exactly once; anyone else is a new person" drew him, and with
    // four cast still drew exactly four. But that open wording let a cast of
    // two touching be drawn three or four times, 19 of 21 pictures of
    // recorded calls, and "Exactly two people are in the image, A and B, each
    // exactly once; no one else is in it" with portraits only was 2 of 21
    // (E14). So without extras the cast is counted closed, by name, in the
    // foreground (owner, 2026-10-19), and the background is left to the
    // scene: "no one else" would empty a busy café as it erased the waiter.
    // Background people are strangers, never a second copy of the cast.
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
        // One person bound to two images: E9b's two-person shape for one,
        // its pronoun-free singular.
        (1, 0) if both => prompt.push_str(&format!(
            " {}.{bound_words} Exactly one person is in the foreground, {}, once; anyone in \
             the background is a stranger, not from the images. The images are identity \
             references only; this is a new image with its own composition, pose and lighting.",
            capitalize(&people[0]),
            names[0]
        )),
        (1, _) if both => prompt.push_str(&format!(
            " {}.{bound_words} Also in the scene, not from the images: {others}. Exactly {} \
             people in the image: the one from the images, exactly once, and {new_people} not \
             from any image. The images are identity references only; this is a new image with \
             its own composition, pose and lighting.",
            capitalize(&people[0]),
            number(1 + m)
        )),
        (1, 0) => prompt.push_str(&format!(
            " {}. Exactly one person is in the foreground, {}, once; anyone in the background \
             is a stranger, not from the image. The image is an identity reference only; this \
             is a new image with its own composition, pose and lighting.",
            capitalize(&people[0]),
            names[0]
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
            " From left to right: {}.{bound_words} Exactly {} people are in the foreground, {}, \
             each exactly once; anyone in the background is a stranger, not from any image. All \
             images serve as identity sources only.",
            people.join("; "),
            number(n),
            join_and(&names)
        )),
        (n, _) => prompt.push_str(&format!(
            " From left to right: {}.{bound_words} Also in the scene, not from any image: \
             {others}. Exactly {} people in the image: the {} from the images, each exactly \
             once, and {new_people} not from any image. All images serve as identity sources \
             only.",
            people.join("; "),
            number(n + m),
            number(n)
        )),
    }
    if let Some(text) = style_text {
        prompt.push_str(&format!(" Style: {text}"));
    }
    if prompt.chars().count() > MAX_COMPILED_PROMPT {
        // The binding is prompt-only, so it must never turn a picture that
        // draws from faces into a refusal: past the cap, the faces-only
        // compile stands, said (review of #685). Its notes are replaced,
        // since the only one it could carry is the layers note.
        if both {
            let mut faces = compile_with(lib, scene, cast, given_extras, style, false)?;
            faces.notes = vec![
                "Full-figure references were not sent: the prompt would run over its limit \
                 with them."
                    .into(),
            ];
            return Ok(faces);
        }
        return Err(format!(
            "The compiled prompt is over {MAX_COMPILED_PROMPT} characters; shorten the scene."
        ));
    }
    Ok(Compiled {
        prompt,
        references: refs,
        used,
        source_seeds,
        notes,
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

/// "a", "a and b", "a, b and c".
pub fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
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
    /// A name the library does not hold is answered with that name alone:
    /// never the roster, a locked entry or a tool to call (2026-10-08: a
    /// persona chat has no image_library and retried the refusal naming it).
    #[test]
    fn a_missing_entry_names_only_what_was_asked() {
        let dir = scratch();
        create(dir.path(), style("noir")).unwrap();
        let mut hidden = style("hidden-look");
        hidden.locked = true;
        create(dir.path(), hidden).unwrap();
        create(dir.path(), character("wren", Origin::Owner)).unwrap();
        let lib = Library::load(dir.path()).0;
        assert_eq!(
            missing(&lib, Kind::Style, "pastel"),
            "No approved style named `pastel`."
        );
        assert_eq!(
            missing(&lib, Kind::Character, "ivo"),
            "No approved character named `ivo`."
        );
    }

    /// A thumbnail is the picture at most `THUMB_SIDE` long, as a JPEG,
    /// made once and kept; only a picture the entry names is shrunk.
    #[test]
    fn a_thumbnail_is_small_kept_and_only_of_a_named_picture() {
        let dir = scratch();
        let mut wide = character("wren", Origin::Owner);
        // Transparent black on the left, opaque red on the right.
        let img = image::RgbaImage::from_fn(1200, 800, |x, _| {
            if x < 600 {
                image::Rgba([0, 0, 0, 0])
            } else {
                image::Rgba([255, 0, 0, 255])
            }
        });
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        wide.portrait = Some(png.into_inner());
        create(dir.path(), wide).unwrap();
        create(dir.path(), character("ivo", Origin::Owner)).unwrap();
        let lib = Library::load(dir.path()).0;
        let wren = lib.get(Kind::Character, "wren").unwrap();
        let blob = wren.portrait.clone().unwrap();
        let jpeg = lib.thumbnail_named(wren, &blob).unwrap();
        let small = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((small.width(), small.height()), (THUMB_SIDE, 341));
        assert_eq!(&jpeg[..3], b"\xff\xd8\xff", "a JPEG");
        // Transparent pixels are laid over white, not the black JPEG keeps
        // under a dropped alpha, and before the resize: at the edge, red
        // meets white, never the transparent pixels' black (no dark fringe).
        let rgb = small.to_rgb8();
        assert!(rgb.get_pixel(10, 10).0.iter().all(|c| *c > 240));
        assert!(
            (0..THUMB_SIDE).all(|x| rgb.get_pixel(x, 170).0[0] > 235),
            "no fringe"
        );
        assert!(thumb_path(dir.path(), &blob).exists(), "kept");
        assert_eq!(lib.thumbnail_named(wren, &blob).unwrap(), jpeg);
        // Another entry's picture is not this one's to shrink.
        let ivo = lib.get(Kind::Character, "ivo").unwrap();
        assert!(lib.thumbnail_named(ivo, &blob).is_err());
        // A small picture keeps its size.
        let tiny = lib
            .thumbnail_named(ivo, ivo.portrait.as_deref().unwrap())
            .unwrap();
        let tiny = image::load_from_memory(&tiny).unwrap();
        assert_eq!((tiny.width(), tiny.height()), (2, 2));
        // A rejected candidate takes its thumbnail with its portrait.
        let mut sam = character("sam", Origin::ModelUntrusted);
        sam.portrait = Some({
            let img = image::RgbImage::from_pixel(3, 3, image::Rgb([1, 2, 3]));
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::Png).unwrap();
            out.into_inner()
        });
        create(dir.path(), sam).unwrap();
        let lib = Library::load(dir.path()).0;
        let sam = lib.get(Kind::Character, "sam").unwrap();
        let blob = sam.portrait.clone().unwrap();
        lib.thumbnail_named(sam, &blob).unwrap();
        assert!(thumb_path(dir.path(), &blob).exists());
        reject(dir.path(), Kind::Character, "sam").unwrap();
        assert!(!dir.path().join("blobs").join(&blob).exists());
        assert!(
            !thumb_path(dir.path(), &blob).exists(),
            "the thumbnail went too"
        );
    }

    /// The URL's generation is the disk's, or a bump moves the files and
    /// leaves every browser on the old ones for a year (review of #737).
    #[test]
    fn the_thumbnail_querys_generation_is_the_disks() {
        assert_eq!(THUMB_QUERY, format!("thumb-{THUMB_VERSION}"));
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
            clothes_in_act: false,
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

    /// A location is text plus up to three `view` pictures: no portrait, no
    /// person's references, and its names are offered as styles' are.
    #[test]
    fn a_location_is_text_and_view_pictures() {
        let dir = scratch();
        let place = |name: &str, locked| NewEntry {
            kind: Kind::Location,
            name: name.into(),
            text: "a small tiled room with a padded bench".into(),
            portrait: None,
            source_seed: None,
            origin: Origin::Owner,
            locked,
        };
        let e = create(dir.path(), place("boathouse", false)).unwrap();
        assert_eq!((e.kind, e.status), (Kind::Location, Status::Approved));
        create(dir.path(), place("attic", true)).unwrap();
        // A portrait, a face or a body is refused on a location.
        let with_portrait = NewEntry {
            portrait: Some(png()),
            ..place("cellar", false)
        };
        let err = create(dir.path(), with_portrait).unwrap_err();
        assert!(
            format!("{err}").contains("a location has no portrait"),
            "{err}"
        );
        let err = add_reference_to(
            dir.path(),
            Kind::Location,
            "boathouse",
            RefKind::Face,
            &png(),
            None,
            None,
        )
        .unwrap_err();
        assert!(format!("{err}").contains("takes view references"), "{err}");
        // A text past the location cap is refused.
        let long = NewEntry {
            text: "x".repeat(MAX_LOCATION_TEXT + 1),
            ..place("pier", false)
        };
        assert!(create(dir.path(), long).is_err());
        // Three views, distinct pictures; a fourth is past the limit.
        for shade in 0..3u8 {
            let img = image::RgbImage::from_pixel(2, 2, image::Rgb([shade, 1, 2]));
            let mut b = std::io::Cursor::new(Vec::new());
            img.write_to(&mut b, image::ImageFormat::Png).unwrap();
            add_reference_to(
                dir.path(),
                Kind::Location,
                "boathouse",
                RefKind::View,
                &b.into_inner(),
                Some(format!("angle {shade}")),
                None,
            )
            .unwrap();
        }
        let err = add_reference_to(
            dir.path(),
            Kind::Location,
            "boathouse",
            RefKind::View,
            &png(),
            Some("angle 9".into()),
            None,
        )
        .unwrap_err();
        assert!(format!("{err}").contains("already has 3 view"), "{err}");
        let (lib, errors) = Library::load(dir.path());
        assert!(errors.is_empty(), "{errors:?}");
        let boathouse = lib.get(Kind::Location, "boathouse").unwrap();
        assert_eq!(boathouse.sendable_references().len(), 3);
        let first = boathouse.sendable_references()[0];
        assert!(lib.read_reference(boathouse, first).is_ok());
        // A same-named character is a different entry.
        assert!(lib.get(Kind::Character, "boathouse").is_none());
        // Offered names: approved and unlocked only.
        assert_eq!(location_names(&lib), ["boathouse"]);
        let removed = remove_reference_from(
            dir.path(),
            Kind::Location,
            "boathouse",
            RefKind::View,
            Some("angle 1"),
        )
        .unwrap();
        assert_eq!(removed.references.len(), 2);
        // An unlabelled view beside labelled ones is removed with no label
        // (review of #743); with none unlabelled, no label is still refused.
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([9, 9, 9]));
        let mut b = std::io::Cursor::new(Vec::new());
        img.write_to(&mut b, image::ImageFormat::Png).unwrap();
        add_reference_to(
            dir.path(),
            Kind::Location,
            "boathouse",
            RefKind::View,
            &b.into_inner(),
            None,
            None,
        )
        .unwrap();
        let removed =
            remove_reference_from(dir.path(), Kind::Location, "boathouse", RefKind::View, None)
                .unwrap();
        assert_eq!(removed.references.len(), 2);
        assert!(removed.references.iter().all(|r| r.label.is_some()));
        assert!(remove_reference_from(
            dir.path(),
            Kind::Location,
            "boathouse",
            RefKind::View,
            None
        )
        .is_err());
    }

    /// An outfit is text up to `wearing`'s length and a garment picture; no
    /// portrait, no person's references.
    #[test]
    fn an_outfit_is_text_and_a_garment_picture() {
        let dir = scratch();
        let outfit = |name: &str, text: String| NewEntry {
            kind: Kind::Outfit,
            name: name.into(),
            text,
            portrait: None,
            source_seed: None,
            origin: Origin::Owner,
            locked: false,
        };
        create(
            dir.path(),
            outfit("red-raincoat", "a red hooded raincoat".into()),
        )
        .unwrap();
        assert!(create(dir.path(), outfit("long", "x".repeat(MAX_OUTFIT_TEXT + 1))).is_err());
        add_reference_to(
            dir.path(),
            Kind::Outfit,
            "red-raincoat",
            RefKind::View,
            &png(),
            None,
            None,
        )
        .unwrap();
        let err = add_reference_to(
            dir.path(),
            Kind::Outfit,
            "red-raincoat",
            RefKind::FullBody,
            &png(),
            None,
            None,
        )
        .unwrap_err();
        assert!(
            format!("{err}").contains("an outfit takes view references"),
            "{err}"
        );
        let (lib, errors) = Library::load(dir.path());
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(outfit_names(&lib), ["red-raincoat"]);
        assert_eq!(location_names(&lib), Vec::<String>::new());
        assert_eq!(
            lib.get(Kind::Outfit, "red-raincoat")
                .unwrap()
                .sendable_references()
                .len(),
            1
        );
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

    /// A lock set before argon2 0.6 still opens: the hash below was written
    /// by argon2 0.5.3's `hash_password` over a `SaltString::encode_b64` of
    /// sixteen raw bytes, as `set_lock_password` did then. A crate upgrade
    /// that changed how a salt is read would lock the owner out of their own
    /// library, and only a hash from the old code can show it has not.
    #[test]
    fn a_lock_set_by_argon2_0_5_still_verifies() {
        let dir = scratch();
        std::fs::write(
            dir.path().join(LOCK_FILE),
            "hash = \"$argon2id$v=19$m=19456,t=2,p=1$Zml4dHVyZS1zYWx0LTE2Yg\
             $CzgxBP50vmZ8uam6e1TshTmF845oIcLNCbBu0yKjuzA\"\n",
        )
        .unwrap();
        assert!(verify_lock_password(dir.path(), "lantern-orchard-42").unwrap());
        assert!(!verify_lock_password(dir.path(), "lantern-orchard-43").unwrap());
    }

    /// Every lock gets a salt of its own: one password set twice is two
    /// different hashes. A constant salt — a dropped or ignored argument after
    /// a crate change of how the salt is passed — would make a lock file
    /// precomputable while every other test stayed green (review of #670).
    #[test]
    fn two_locks_with_one_password_get_different_salts() {
        let (a, b) = (scratch(), scratch());
        set_lock_password(a.path(), "correct horse").unwrap();
        set_lock_password(b.path(), "correct horse").unwrap();
        let read = |d: &Scratch| std::fs::read_to_string(d.path().join(LOCK_FILE)).unwrap();
        assert_ne!(read(&a), read(&b));
        assert!(verify_lock_password(a.path(), "correct horse").unwrap());
        assert!(verify_lock_password(b.path(), "correct horse").unwrap());
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
    fn voice_locks_round_trip_and_a_damaged_file_is_never_rewritten_open() {
        let dir = scratch();
        assert!(locked_voices(dir.path()).unwrap().is_empty());
        set_voice_locked(dir.path(), "ada", true).unwrap();
        set_voice_locked(dir.path(), "vctk_p229", true).unwrap();
        assert!(
            !set_voice_locked(dir.path(), "ada", true).unwrap(),
            "twice is once"
        );
        let names: Vec<String> = locked_voices(dir.path()).unwrap().into_iter().collect();
        assert_eq!(names, ["ada", "vctk_p229"]);
        set_voice_locked(dir.path(), "ada", false).unwrap();
        set_voice_locked(dir.path(), "never-locked", false).unwrap();
        let names: Vec<String> = locked_voices(dir.path()).unwrap().into_iter().collect();
        assert_eq!(names, ["vctk_p229"]);
        // Not the password: locking a voice never makes one appear.
        assert!(!has_lock_password(dir.path()));
        for bad in ["", "../x", "Ada", "a b"] {
            assert!(set_voice_locked(dir.path(), bad, true).is_err(), "{bad:?}");
        }
        // Damage is a finding, and a write over it would unlock everything.
        std::fs::write(dir.path().join("voices.toml"), "locked = 3").unwrap();
        assert!(locked_voices(dir.path()).is_err());
        assert!(set_voice_locked(dir.path(), "bob", true).is_err());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("voices.toml")).unwrap(),
            "locked = 3"
        );
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
        assert!(
            c.prompt.contains(
                "Exactly one person is in the foreground, Maya, once; anyone in the background \
                 is a stranger, not from the image."
            ),
            "{}",
            c.prompt
        );
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

    /// A scene that says what its people do together sends no pose for each:
    /// the cast line carries no empty clause, and a placeholder is still no
    /// answer (mecha-a3, 2026-10-08).
    #[test]
    fn people_doing_something_together_need_no_pose_each() {
        let dir = scratch();
        for n in ["maya", "john"] {
            create(dir.path(), character(n, Origin::Owner)).unwrap();
        }
        let (lib, _) = Library::load(dir.path());
        let still = |name: &str| CastMember {
            name: name.into(),
            wearing: "a grey coat".into(),
            doing: String::new(),
            clothes_in_act: false,
        };
        let c = compile(
            &lib,
            "a clinic room. Maya ties John's shoelace",
            &[still("maya"), still("john")],
            &[],
            None,
        )
        .unwrap();
        assert!(
            c.prompt
                .contains("wearing a grey coat; the person from <image2>"),
            "{}",
            c.prompt
        );
        assert!(
            !c.prompt.contains(", ;") && !c.prompt.contains(", ."),
            "{}",
            c.prompt
        );
        let mut placeholder = still("maya");
        placeholder.doing = "…".into();
        assert!(compile(&lib, "a clinic room", &[placeholder], &[], None).is_err());
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
        assert!(
            c.prompt.contains(
                "Exactly three people are in the foreground, Maya, John and Priya, each exactly \
                 once; anyone in the background is a stranger"
            ),
            "{}",
            c.prompt
        );
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
        // No extras: the cast counted closed, by name, in the foreground,
        // and nothing that would forbid the background the scene describes
        // (E12's erased waiter, E14).
        let c = compile(&lib, "a park", &cast, &[], None).unwrap();
        assert!(
            c.prompt.ends_with(
                "Exactly three people are in the foreground, Maya, John and Priya, each exactly \
                 once; anyone in the background is a stranger, not from any image. All images \
                 serve as identity sources only."
            ),
            "{}",
            c.prompt
        );
        assert!(!c.prompt.contains("no one else"), "{}", c.prompt);
        // With extras, two are counted with them as before (E11).
        let c = compile(&lib, "a diner booth", &cast[..2], &waiter, None).unwrap();
        assert!(
            c.prompt.contains("Exactly three people in the image"),
            "{}",
            c.prompt
        );
        assert!(!c.prompt.contains("no one else"), "{}", c.prompt);
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

    /// A different real PNG per colour, so two references are two blobs.
    fn png_of(rgb: [u8; 3]) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb(rgb));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    /// A reference is the owner's write on an approved character: a new
    /// version, the old one kept, the picture read back under its hash
    /// (IMAGE-COMPILER-DESIGN.md §9.3).
    #[test]
    fn a_reference_is_a_new_version_read_under_its_hash() {
        let dir = scratch();
        create(dir.path(), character("wren", Origin::Owner)).unwrap();
        let e = add_reference(
            dir.path(),
            "wren",
            RefKind::FullBody,
            &png_of([1, 2, 3]),
            None,
        )
        .unwrap();
        assert_eq!(e.version, 2);
        assert!(entry_dir(dir.path(), Kind::Character, "wren")
            .join("history/v1.toml")
            .is_file());
        let lib = Library::load(dir.path()).0;
        let wren = lib.get(Kind::Character, "wren").unwrap();
        assert_eq!(wren.references.len(), 1);
        assert_eq!(wren.sendable_references().len(), 1);
        let (bytes, ext) = lib.read_reference(wren, &wren.references[0]).unwrap();
        assert_eq!((bytes, ext), (png_of([1, 2, 3]), "png"));
        // Its text is untouched, so the owner's write changes no provenance.
        assert_eq!(wren.origin, Origin::Owner);
        assert_eq!(wren.status, Status::Approved);
    }

    /// A candidate carries no references, so rejecting one can never strand
    /// reference blobs (§9.3, review of #680).
    #[test]
    fn a_candidate_takes_no_reference() {
        let dir = scratch();
        create(dir.path(), character("moss", Origin::ModelClean)).unwrap();
        let err = add_reference(
            dir.path(),
            "moss",
            RefKind::FullBody,
            &png_of([9, 9, 9]),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("candidate"), "{err}");
    }

    /// The store limits, a detail's label and the label's cap.
    #[test]
    fn references_keep_their_limits_and_labels() {
        let dir = scratch();
        create(dir.path(), character("wren", Origin::Owner)).unwrap();
        add_reference(
            dir.path(),
            "wren",
            RefKind::FullBody,
            &png_of([1, 0, 0]),
            None,
        )
        .unwrap();
        let second = add_reference(
            dir.path(),
            "wren",
            RefKind::FullBody,
            &png_of([2, 0, 0]),
            None,
        )
        .unwrap_err();
        assert!(second.to_string().contains("remove one first"), "{second}");
        let unlabelled = add_reference(
            dir.path(),
            "wren",
            RefKind::Detail,
            &png_of([3, 0, 0]),
            None,
        )
        .unwrap_err();
        assert!(
            unlabelled.to_string().contains("needs a label"),
            "{unlabelled}"
        );
        let long = add_reference(
            dir.path(),
            "wren",
            RefKind::Detail,
            &png_of([3, 0, 0]),
            Some("x".repeat(MAX_LABEL + 1)),
        )
        .unwrap_err();
        assert!(long.to_string().contains("capped"), "{long}");
        for (i, l) in ["tattoo", "scar", "freckles", "ring"].iter().enumerate() {
            add_reference(
                dir.path(),
                "wren",
                RefKind::Detail,
                &png_of([4, i as u8, 0]),
                Some(l.to_string()),
            )
            .unwrap();
        }
        let fifth = add_reference(
            dir.path(),
            "wren",
            RefKind::Detail,
            &png_of([5, 0, 0]),
            Some("mole".into()),
        )
        .unwrap_err();
        assert!(fifth.to_string().contains("remove one first"), "{fifth}");
        let view =
            add_reference(dir.path(), "wren", RefKind::View, &png_of([6, 0, 0]), None).unwrap_err();
        assert!(view.to_string().contains("not view"), "{view}");
    }

    /// An unknown kind, or a known kind this entry kind doesn't take, loads
    /// and is shown but is never sent (§9.3's fail-closed reading).
    #[test]
    fn an_unknown_or_misplaced_reference_loads_but_is_never_sent() {
        let dir = scratch();
        create(dir.path(), character("wren", Origin::Owner)).unwrap();
        let path = entry_dir(dir.path(), Kind::Character, "wren").join("entry.toml");
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("\n[[reference]]\nkind = \"hat\"\nblob = \"sha256-aa.png\"\n\n[[reference]]\nkind = \"view\"\nblob = \"sha256-bb.png\"\n");
        std::fs::write(&path, text).unwrap();
        let (lib, errors) = Library::load(dir.path());
        assert!(errors.is_empty(), "{errors:?}");
        let wren = lib.get(Kind::Character, "wren").unwrap();
        assert_eq!(wren.references.len(), 2);
        assert_eq!(wren.references[0].kind, RefKind::Unknown);
        assert!(wren.sendable_references().is_empty());
    }

    /// A character's face is its `face` reference when it has one, and its
    /// portrait otherwise; each says which it was.
    #[test]
    fn a_face_reference_is_preferred_over_the_portrait() {
        let dir = scratch();
        create(dir.path(), character("wren", Origin::Owner)).unwrap();
        let lib = Library::load(dir.path()).0;
        let wren = lib.get(Kind::Character, "wren").unwrap();
        let (portrait, _) = lib.read_portrait(wren).unwrap();
        let got = lib.face_picture(wren).unwrap();
        assert_eq!((got.which(), got.bytes), ("portrait", portrait));
        add_reference(dir.path(), "wren", RefKind::Face, &png_of([7, 7, 7]), None).unwrap();
        let lib = Library::load(dir.path()).0;
        let wren = lib.get(Kind::Character, "wren").unwrap();
        let got = lib.face_picture(wren).unwrap();
        assert_eq!(
            (got.which(), got.bytes),
            ("face reference", png_of([7, 7, 7]))
        );
    }

    /// A hand-edited reference can't point outside `blobs/`, and bytes
    /// swapped under an approved hash are refused, as for the portrait.
    #[test]
    fn a_reference_gets_the_portraits_name_and_hash_checks() {
        let dir = scratch();
        create(dir.path(), character("wren", Origin::Owner)).unwrap();
        let e = add_reference(dir.path(), "wren", RefKind::Face, &png_of([7, 7, 7]), None).unwrap();
        std::fs::write(
            dir.path().join("blobs").join(&e.references[0].blob),
            png_of([8, 8, 8]),
        )
        .unwrap();
        let lib = Library::load(dir.path()).0;
        let wren = lib.get(Kind::Character, "wren").unwrap();
        let swapped = lib.read_reference(wren, &wren.references[0]).unwrap_err();
        assert!(
            swapped.to_string().contains("does not match its hash"),
            "{swapped}"
        );

        let path = entry_dir(dir.path(), Kind::Character, "wren").join("entry.toml");
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace(&e.references[0].blob, "../../escape.png");
        std::fs::write(&path, text).unwrap();
        let (lib, errors) = Library::load(dir.path());
        assert!(lib.get(Kind::Character, "wren").is_none());
        assert!(
            errors.iter().any(|e| e.why.contains("not a blob name")),
            "{errors:?}"
        );
    }

    /// Several details need a label to say which goes; the picture stays for
    /// the history.
    #[test]
    fn removing_a_reference_is_a_new_version_and_keeps_the_picture() {
        let dir = scratch();
        create(dir.path(), character("wren", Origin::Owner)).unwrap();
        add_reference(
            dir.path(),
            "wren",
            RefKind::Detail,
            &png_of([1, 1, 0]),
            Some("tattoo".into()),
        )
        .unwrap();
        let e = add_reference(
            dir.path(),
            "wren",
            RefKind::Detail,
            &png_of([1, 2, 0]),
            Some("scar".into()),
        )
        .unwrap();
        let ambiguous = remove_reference(dir.path(), "wren", RefKind::Detail, None).unwrap_err();
        assert!(ambiguous.to_string().contains("--label"), "{ambiguous}");
        let gone = e.references[0].blob.clone();
        let after = remove_reference(dir.path(), "wren", RefKind::Detail, Some("tattoo")).unwrap();
        assert_eq!(after.version, 4);
        assert_eq!(after.references.len(), 1);
        assert!(dir.path().join("blobs").join(gone).is_file());
    }

    /// Two details under one label could never be told apart for removal,
    /// so the second is refused before its picture is stored (review of #684).
    #[test]
    fn a_label_is_unique_within_its_kind() {
        let dir = scratch();
        create(dir.path(), character("wren", Origin::Owner)).unwrap();
        add_reference(
            dir.path(),
            "wren",
            RefKind::Detail,
            &png_of([1, 3, 0]),
            Some("tattoo".into()),
        )
        .unwrap();
        let dup = add_reference(
            dir.path(),
            "wren",
            RefKind::Detail,
            &png_of([1, 4, 0]),
            Some(" tattoo ".into()),
        )
        .unwrap_err();
        assert!(dup.to_string().contains("use another label"), "{dup}");
        let blobs = std::fs::read_dir(dir.path().join("blobs")).unwrap().count();
        assert_eq!(blobs, 2, "the refused picture was stored");
    }

    /// The read itself refuses a reference the entry may not send, so no
    /// caller can ship one by iterating `references` (review of #684).
    #[test]
    fn a_reference_that_may_not_be_sent_cannot_be_read() {
        let dir = scratch();
        create(dir.path(), character("wren", Origin::Owner)).unwrap();
        let path = entry_dir(dir.path(), Kind::Character, "wren").join("entry.toml");
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("\n[[reference]]\nkind = \"view\"\nblob = \"sha256-bb.png\"\n");
        std::fs::write(&path, text).unwrap();
        let lib = Library::load(dir.path()).0;
        let wren = lib.get(Kind::Character, "wren").unwrap();
        let err = lib.read_reference(wren, &wren.references[0]).unwrap_err();
        assert!(err.to_string().contains("not one it may send"), "{err}");
    }

    /// A second copy of a kind past its limit equals the first by value; the
    /// gate goes by which reference it is, so the copy is refused (review of
    /// #684).
    #[test]
    fn a_copy_past_the_limit_is_refused_by_identity() {
        let dir = scratch();
        create(dir.path(), character("wren", Origin::Owner)).unwrap();
        add_reference(
            dir.path(),
            "wren",
            RefKind::FullBody,
            &png_of([5, 5, 5]),
            None,
        )
        .unwrap();
        let path = entry_dir(dir.path(), Kind::Character, "wren").join("entry.toml");
        let mut entry: Entry = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        entry.references.push(entry.references[0].clone());
        std::fs::write(&path, toml::to_string_pretty(&entry).unwrap()).unwrap();
        let lib = Library::load(dir.path()).0;
        let wren = lib.get(Kind::Character, "wren").unwrap();
        assert_eq!(wren.references.len(), 2);
        assert!(lib.read_reference(wren, &wren.references[0]).is_ok());
        let err = lib.read_reference(wren, &wren.references[1]).unwrap_err();
        assert!(err.to_string().contains("not one it may send"), "{err}");
    }

    /// Labels are trimmed when stored, so removal trims what it is given.
    #[test]
    fn removing_by_label_trims_it() {
        let dir = scratch();
        create(dir.path(), character("wren", Origin::Owner)).unwrap();
        for (rgb, label) in [([2, 1, 0], "tattoo"), ([2, 2, 0], "scar")] {
            add_reference(
                dir.path(),
                "wren",
                RefKind::Detail,
                &png_of(rgb),
                Some(label.into()),
            )
            .unwrap();
        }
        let after =
            remove_reference(dir.path(), "wren", RefKind::Detail, Some("  tattoo ")).unwrap();
        assert_eq!(after.references.len(), 1);
        assert_eq!(after.references[0].label.as_deref(), Some("scar"));
    }

    fn with_body(dir: &Path, name: &str, rgb: [u8; 3]) {
        create(dir, character(name, Origin::Owner)).unwrap();
        add_reference(dir, name, RefKind::FullBody, &png_of(rgb), None).unwrap();
    }

    /// E14's shape (§9.4): two people with a body each send their portraits
    /// only, said in the notes, and are counted closed, by name: four images
    /// for two people drew someone twice where two did not.
    #[test]
    fn a_cast_of_two_with_bodies_sends_portraits_and_a_closed_count() {
        let dir = scratch();
        with_body(dir.path(), "maya", [9, 1, 1]);
        with_body(dir.path(), "jo", [9, 2, 2]);
        let lib = Library::load(dir.path()).0;
        let c = compile(&lib, "a studio", &[member("maya"), member("jo")], &[], None).unwrap();
        let names: Vec<&str> = c.references.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(names, ["maya", "jo"]);
        assert_eq!(
            c.prompt,
            "a studio. From left to right: the person from <image1> (maya, a person with a \
             memorable face), wearing a yellow raincoat, laughing; the person from <image2> \
             (jo, a person with a memorable face), wearing a yellow raincoat, laughing. Exactly \
             two people are in the foreground, Maya and Jo, each exactly once; anyone in the \
             background is a stranger, not from any image. All images serve as identity sources \
             only."
        );
        assert_eq!(
            c.notes,
            [
                "Full-figure references were not sent: they go only to a picture of one person, \
              and this one has two."
            ]
        );
        assert!(c.used.iter().all(|u| u.full_body.is_none()));
    }

    /// One person: the same binding, in the singular.
    #[test]
    fn a_cast_of_one_with_a_body_is_bound_too() {
        let dir = scratch();
        with_body(dir.path(), "maya", [9, 3, 3]);
        let lib = Library::load(dir.path()).0;
        let c = compile(&lib, "a studio", &[member("maya")], &[], None).unwrap();
        assert_eq!(c.references.len(), 2);
        assert!(
            c.prompt.contains(
                "The person shown in <image1> and <image2> (maya, a person with a memorable \
                 face), wearing a yellow raincoat, laughing. <image1> and <image2> show one \
                 person, Maya: <image1> is Maya's portrait, <image2> Maya's full figure. The \
                 clothing in <image2> is not worn here. Exactly one person is in the \
                 foreground, Maya, once; anyone in the background is a stranger, not from the \
                 images."
            ),
            "{}",
            c.prompt
        );
    }

    /// A body goes only to a picture of one person (§9.4, E14), each refusal
    /// said in counts and never a label: a cast of more, a layered plan.
    #[test]
    fn bodies_go_to_a_cast_of_one_and_say_why_in_counts() {
        let dir = scratch();
        with_body(dir.path(), "maya", [9, 4, 4]);
        create(dir.path(), character("jo", Origin::Owner)).unwrap();
        with_body(dir.path(), "theo", [9, 5, 5]);
        with_body(dir.path(), "ada", [9, 6, 6]);
        let lib = Library::load(dir.path()).0;
        let faces_only = |c: &Compiled| {
            assert!(c.references.iter().all(|r| !r.0.starts_with("full_body:")));
            assert!(!c.prompt.contains("full figure"), "{}", c.prompt);
            assert!(c.used.iter().all(|u| u.full_body.is_none()));
        };

        let c = compile(&lib, "a studio", &[member("maya"), member("jo")], &[], None).unwrap();
        faces_only(&c);
        assert!(
            c.prompt.contains("the person from <image2>"),
            "{}",
            c.prompt
        );
        assert_eq!(
            c.notes,
            [
                "Full-figure references were not sent: they go only to a picture of one person, \
                 and this one has two."
            ]
        );

        // Past the cap the cap is the reason, attached or not: attaching
        // a body to the one without could not help.
        let partial = [member("maya"), member("jo"), member("theo")];
        let c = compile(&lib, "a studio", &partial, &[], None).unwrap();
        faces_only(&c);
        assert!(
            c.notes[0].ends_with("and this one has three."),
            "{:?}",
            c.notes
        );
        let three = [member("maya"), member("theo"), member("ada")];
        let c = compile(&lib, "a studio", &three, &[], None).unwrap();
        faces_only(&c);
        assert_eq!(
            c.notes,
            [
                "Full-figure references were not sent: they go only to a picture of one \
                 person, and this one has three."
            ]
        );

        let c = compile_with(&lib, "a studio", &[member("maya")], &[], None, false).unwrap();
        faces_only(&c);
        assert_eq!(
            c.notes,
            [
                "Full-figure references were not sent: a picture planned in layers is drawn from \
              portraits."
            ]
        );

        // Nobody has one: nothing was left out, so nothing is said.
        let c = compile(&lib, "a studio", &[member("jo")], &[], None).unwrap();
        assert!(c.notes.is_empty(), "{:?}", c.notes);
    }

    /// The binding is prompt-only: a scene that draws from faces just under
    /// the cap still draws, faces only and said, never refused (review of
    /// #685).
    #[test]
    fn bodies_past_the_prompt_cap_fall_back_to_faces() {
        let dir = scratch();
        with_body(dir.path(), "maya", [9, 9, 1]);
        let lib = Library::load(dir.path()).0;
        let one = [member("maya")];
        let base = compile_with(&lib, "x", &one, &[], None, false)
            .unwrap()
            .prompt
            .chars()
            .count();
        // "x" becomes a scene ten characters under the cap, faces only.
        let scene = "a".repeat(MAX_COMPILED_PROMPT - 10 - (base - 1));
        let faces = compile_with(&lib, &scene, &one, &[], None, false).unwrap();
        assert_eq!(faces.prompt.chars().count(), MAX_COMPILED_PROMPT - 10);
        let c = compile(&lib, &scene, &one, &[], None).unwrap();
        assert_eq!(c.prompt, faces.prompt);
        assert_eq!(c.references.len(), 1);
        assert!(c.used.iter().all(|u| u.full_body.is_none()));
        assert_eq!(
            c.notes,
            [
                "Full-figure references were not sent: the prompt would run over its limit with \
              them."
            ]
        );
        // Over the cap from faces alone is still the scene's to shorten.
        let long = "a".repeat(MAX_COMPILED_PROMPT);
        assert!(compile(&lib, &long, &one, &[], None)
            .unwrap_err()
            .contains("shorten the scene"));
    }

    /// A body whose bytes no longer match their hash is not sent; the
    /// picture still draws, from the portrait.
    #[test]
    fn a_body_that_fails_its_hash_is_not_sent() {
        let dir = scratch();
        with_body(dir.path(), "jo", [9, 8, 8]);
        let lib = Library::load(dir.path()).0;
        let jo = lib.get(Kind::Character, "jo").unwrap();
        std::fs::write(lib.blob_path(&jo.references[0].blob), png_of([0, 0, 1])).unwrap();
        let c = compile(&lib, "a studio", &[member("jo")], &[], None).unwrap();
        assert_eq!(c.references.len(), 1);
        assert_eq!(
            c.notes,
            ["Full-figure references were not sent: it could not be read."]
        );
    }

    /// A pending reference keeps its picture until the owner rules: the
    /// picture survives while it waits, and a rejection deletes it once
    /// nothing else names it.
    #[test]
    fn a_rejected_pending_reference_takes_its_picture_with_it() {
        let dir = scratch();
        create(dir.path(), character("wren", Origin::Owner)).unwrap();
        let p = propose_reference(
            dir.path(),
            "wren",
            RefKind::FullBody,
            PendingAction::Add,
            Some(&png_of([3, 4, 5])),
            None,
            Origin::ModelUntrusted,
            false,
        )
        .unwrap();
        let blob = dir.path().join("blobs").join(p.blob.as_ref().unwrap());
        assert!(blob.is_file());
        assert!(
            blob_referenced(dir.path(), p.blob.as_ref().unwrap()),
            "the pending file protects it"
        );
        // Not an entry: loading the library sees no new kind or character.
        assert_eq!(Library::load(dir.path()).0.all().len(), 1);
        reject_reference(dir.path(), &p.id).unwrap();
        assert!(!blob.exists());
        assert!(pending_references(dir.path()).0.is_empty());
    }

    /// A staged change the store would refuse is refused at the door, with
    /// no picture left behind; one whose action is missing or unknown loads
    /// but never applies (review of #692).
    #[test]
    fn a_doomed_proposal_is_refused_and_an_unknown_action_never_applies() {
        let dir = scratch();
        create(dir.path(), character("wren", Origin::Owner)).unwrap();
        add_reference(
            dir.path(),
            "wren",
            RefKind::FullBody,
            &png_of([8, 1, 1]),
            None,
        )
        .unwrap();
        let blobs = || std::fs::read_dir(dir.path().join("blobs")).unwrap().count();
        let before = blobs();
        let full = propose_reference(
            dir.path(),
            "wren",
            RefKind::FullBody,
            PendingAction::Add,
            Some(&png_of([8, 2, 2])),
            None,
            Origin::ModelUntrusted,
            false,
        )
        .unwrap_err();
        assert!(full.to_string().contains("remove one first"), "{full}");
        assert_eq!(blobs(), before, "a refused proposal stored its picture");
        // Nor a picture the character already holds, its portrait included.
        let same = propose_reference(
            dir.path(),
            "wren",
            RefKind::Face,
            PendingAction::Add,
            Some(&png()),
            None,
            Origin::ModelUntrusted,
            false,
        )
        .unwrap_err();
        assert!(
            same.to_string().contains("already holds that picture"),
            "{same}"
        );
        assert_eq!(blobs(), before);

        let pending = dir.path().join("pending");
        std::fs::create_dir_all(&pending).unwrap();
        for (id, action) in [("aa", "action = \"replace\"\n"), ("bb", "")] {
            std::fs::write(
                pending.join(format!("{id}.toml")),
                format!("id = \"{id}\"\nname = \"wren\"\nkind = \"face\"\n{action}origin = \"model_untrusted\"\n"),
            )
            .unwrap();
            let p = pending_references(dir.path())
                .0
                .into_iter()
                .find(|p| p.id == id)
                .unwrap();
            assert_eq!(p.action, PendingAction::Unknown, "{id}");
            assert!(approve_reference(dir.path(), id).is_err(), "{id} applied");
        }
    }

    /// Removing a character takes its staged changes with it: they could
    /// never apply, and would otherwise hold a queue slot and a picture
    /// forever (review of #692).
    #[test]
    fn removing_a_character_drops_its_staged_changes() {
        let dir = scratch();
        create(dir.path(), character("wren", Origin::Owner)).unwrap();
        create(dir.path(), character("jo", Origin::Owner)).unwrap();
        let stage = |name: &str, rgb| {
            propose_reference(
                dir.path(),
                name,
                RefKind::FullBody,
                PendingAction::Add,
                Some(&png_of(rgb)),
                None,
                Origin::ModelUntrusted,
                false,
            )
            .unwrap()
        };
        let wren = stage("wren", [7, 1, 1]);
        let jo = stage("jo", [7, 2, 2]);
        remove(dir.path(), Kind::Character, "wren").unwrap();
        let left: Vec<String> = pending_references(dir.path())
            .0
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(left, [jo.id]);
        assert!(!dir.path().join("blobs").join(wren.blob.unwrap()).exists());
    }

    /// A pending removal names exactly one reference, and approving it
    /// removes that one; a hand-edited file naming a path is not loaded.
    #[test]
    fn a_pending_removal_names_one_and_a_bad_file_is_a_finding() {
        let dir = scratch();
        create(dir.path(), character("wren", Origin::Owner)).unwrap();
        add_reference(
            dir.path(),
            "wren",
            RefKind::FullBody,
            &png_of([6, 6, 6]),
            None,
        )
        .unwrap();
        let none = propose_reference(
            dir.path(),
            "wren",
            RefKind::Face,
            PendingAction::Remove,
            None,
            None,
            Origin::ModelUntrusted,
            false,
        )
        .unwrap_err();
        assert!(none.to_string().contains("exactly one"), "{none}");
        let p = propose_reference(
            dir.path(),
            "wren",
            RefKind::FullBody,
            PendingAction::Remove,
            None,
            None,
            Origin::ModelUntrusted,
            false,
        )
        .unwrap();
        let e = approve_reference(dir.path(), &p.id).unwrap();
        assert!(e.references.is_empty());
        // A hand-edited file pointing outside blobs/ is reported, not loaded.
        let pending = dir.path().join("pending");
        std::fs::write(
            pending.join("abc.toml"),
            "id = \"abc\"\nname = \"wren\"\nkind = \"full_body\"\nblob = \"../escape.png\"\norigin = \"model_untrusted\"\n",
        )
        .unwrap();
        let (found, errors) = pending_references(dir.path());
        assert!(found.is_empty());
        assert!(
            errors.iter().any(|e| e.why.contains("not a blob name")),
            "{errors:?}"
        );
        assert!(approve_reference(dir.path(), "../x").is_err());
    }

    /// The delete gate walks the whole store: a blob named only under a
    /// directory this build doesn't know is still referenced (§9.3).
    #[test]
    fn a_blob_named_only_in_an_unknown_directory_is_still_referenced() {
        let dir = scratch();
        let later = dir.path().join("locations/harbour");
        std::fs::create_dir_all(&later).unwrap();
        std::fs::write(
            later.join("entry.toml"),
            "name = \"harbour\"\n[[reference]]\nkind = \"view\"\nblob = \"sha256-cc.png\"\n",
        )
        .unwrap();
        assert!(blob_referenced(dir.path(), "sha256-cc.png"));
        assert!(!blob_referenced(dir.path(), "sha256-dd.png"));
    }
}
