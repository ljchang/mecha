//! The owner's LoRA registry, for pictures and clips: one file the owner
//! writes (`~/.mecha/loras.toml`), each entry saying which model family it is
//! for, and that claim checked against the file itself (`docs/LORA-DESIGN.md`
//! §2–§3).
//!
//! mecha ships no LoRA and names none (the owner's ruling, 2026-10-23): this
//! is the mechanism. The format:
//!
//! ```toml
//! [[lora]]
//! name = "watercolor"
//! about = "loose watercolor washes"
//! model = "qwen-image"                  # checked against the file's keys
//! file = "watercolor_qi.safetensors"    # under [image] loras_dir
//! strength = 0.7
//! trigger = "watercolor painting"
//!
//! [[lora]]
//! name = "wave"
//! about = "a friendly wave"
//! model = "h3"
//! references = "wave_ref.safetensors"   # a file per H3 mode, under [video] loras_dir
//! frames = "wave_fl.safetensors"
//! ```
//!
//! **A LoRA trained for another model loads without an error and changes
//! little or nothing**, because the loader skips weights whose keys the model
//! does not have. So `model` is never trusted: each file's header is read (a
//! bounded prefix, never the tensors) and its key shapes decide its family,
//! out of a closed set. A file whose keys say otherwise, match neither family
//! or both, or that cannot be read costs that entry, never the registry.
//! Nothing read from a file reaches a prompt or a page: its keys decide one
//! value, and `__metadata__` is not read.
//!
//! The registry used to be clips-only, at the path `[video] loras` names.
//! That file is still read for one release when the new one does not exist,
//! its entries taken as `h3`; when both exist, the new one is the registry and
//! the old one is ignored, never merged.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{anyhow, bail, Result};

/// The registry's file name, in the mecha home (L2: a fixed path, like
/// triggers and skills, never a config key a repository could point).
pub const FILE: &str = "loras.toml";

/// A header longer than this is refused before it is used as a size: the
/// length is the file's own claim. The largest among the owner's 41 files
/// was 75 KB (2026-10-26).
const MAX_HEADER: u64 = 8 << 20;

/// How long a refused file whose stat cannot be taken waits for a retry.
const RETRY_UNSTATTED: Duration = Duration::from_secs(60);

/// Which model a LoRA is for: the closed set its keys can decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    /// Qwen-Image, the picture model.
    QwenImage,
    /// MiniMax H3, the clip model.
    H3,
}

impl Family {
    pub const ALL: [Family; 2] = [Family::QwenImage, Family::H3];

    /// How the registry spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            Family::QwenImage => "qwen-image",
            Family::H3 => "h3",
        }
    }

    fn parse(s: &str) -> Option<Family> {
        Family::ALL.into_iter().find(|f| f.as_str() == s)
    }

    /// The config key that names this family's server's LoRA folders.
    pub fn dir_key(self) -> &'static str {
        match self {
            Family::QwenImage => "[image] loras_dir",
            Family::H3 => "[video] loras_dir",
        }
    }
}

/// What one file's header said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    Family(Family),
    /// Its keys match neither family: refused, never assumed.
    Neither,
    /// Its keys match both, as a merged or hand-built file could.
    Both,
    /// It could not be read: perhaps mid-copy, so retried when it changes.
    Unreadable(String),
}

/// The family a header's tensor names say, out of the closed set.
///
/// Qwen-Image's blocks are `transformer_blocks.*`; FLUX's diffusers files
/// share that name beside `single_transformer_blocks.*`, so a file with the
/// second is not taken for Qwen. H3's are `blocks.*` beside `token_refiner.*`
/// or `audio_patch_proj.*`, or kohya's spelling `lora_unet_blocks_*`. Under
/// a `diffusion_model.`, `transformer.` or `model.diffusion_model.` prefix,
/// as trainers write them.
pub fn family_of_keys<'a>(keys: impl IntoIterator<Item = &'a str>) -> Check {
    let (mut qwen, mut flux, mut blocks, mut h3_marker, mut kohya_h3) =
        (false, false, false, false, false);
    for key in keys {
        if key == "__metadata__" {
            continue;
        }
        if key.starts_with("lora_unet_blocks_") {
            kohya_h3 = true;
            continue;
        }
        let k = ["model.diffusion_model.", "diffusion_model.", "transformer."]
            .iter()
            .find_map(|p| key.strip_prefix(p))
            .unwrap_or(key);
        qwen |= k.starts_with("transformer_blocks.");
        flux |= k.starts_with("single_transformer_blocks.");
        blocks |= k.starts_with("blocks.");
        h3_marker |= k.starts_with("token_refiner.") || k.starts_with("audio_patch_proj.");
    }
    let qwen = qwen && !flux;
    let h3 = (blocks && h3_marker) || kohya_h3;
    match (qwen, h3) {
        (true, false) => Check::Family(Family::QwenImage),
        (false, true) => Check::Family(Family::H3),
        (true, true) => Check::Both,
        (false, false) => Check::Neither,
    }
}

/// The header's tensor names, read as a bounded prefix of the file: the
/// 8-byte little-endian length, refused above [`MAX_HEADER`], then that many
/// bytes. A LoRA is 100 MB to several GB, and none of it past the header is
/// read.
fn header_keys(path: &Path) -> Result<Vec<String>> {
    let mut f = std::fs::File::open(path)?;
    let mut len = [0u8; 8];
    f.read_exact(&mut len)
        .map_err(|_| anyhow!("shorter than a safetensors header"))?;
    let len = u64::from_le_bytes(len);
    if len > MAX_HEADER {
        bail!("its header claims {len} bytes, over the {MAX_HEADER} a LoRA's needs");
    }
    let mut header = vec![0u8; len as usize];
    f.read_exact(&mut header)
        .map_err(|_| anyhow!("its header is longer than the file"))?;
    let map: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&header).map_err(|_| anyhow!("its header is not JSON"))?;
    Ok(map.into_iter().map(|(k, _)| k).collect())
}

/// One file's check, uncached.
fn check_uncached(path: &Path) -> Check {
    match header_keys(path) {
        Ok(keys) => family_of_keys(keys.iter().map(String::as_str)),
        Err(e) => Check::Unreadable(e.to_string()),
    }
}

/// A file's last check, and the stat it was taken under.
struct Cached {
    len: Option<u64>,
    mtime: Option<SystemTime>,
    at: Instant,
    check: Check,
}

static CHECKED: Mutex<Option<HashMap<PathBuf, Cached>>> = Mutex::new(None);

/// `path`'s check, read again only when its length or mtime moved (§3): an
/// unchanged file is never re-read, so nothing about it can move the clip
/// schema. A file that passed keeps its result when its stat cannot be
/// taken, so a flaky mount never flaps it; one refused as unreadable with
/// no stat is retried a minute later. The stat is a trigger, not a claim
/// about the content: a same-length swap that keeps the mtime keeps its old
/// result, and every process start reads every header afresh.
pub fn check_file(path: &Path) -> Check {
    let stat = std::fs::metadata(path).ok();
    let (len, mtime) = (
        stat.as_ref().map(|m| m.len()),
        stat.as_ref().and_then(|m| m.modified().ok()),
    );
    let mut guard = CHECKED.lock().unwrap_or_else(|p| p.into_inner());
    let cache = guard.get_or_insert_with(HashMap::new);
    if let Some(c) = cache.get(path) {
        let unchanged = stat.is_some() && c.len == len && c.mtime == mtime;
        let passed = matches!(c.check, Check::Family(_));
        let keep = unchanged
            || (stat.is_none() && passed)
            || (stat.is_none() && c.len.is_none() && c.at.elapsed() < RETRY_UNSTATTED);
        if keep {
            return c.check.clone();
        }
    }
    let check = check_uncached(path);
    cache.insert(
        path.to_path_buf(),
        Cached {
            len,
            mtime,
            at: Instant::now(),
            check: check.clone(),
        },
    );
    check
}

/// One registry entry, as the owner wrote it.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub about: String,
    pub model: Family,
    /// A picture LoRA's one file.
    pub file: Option<String>,
    /// An H3 LoRA's file per checkpoint: it offers only the modes it names.
    pub frames: Option<String>,
    pub references: Option<String>,
    pub strength: f64,
    pub trigger: Option<String>,
    /// Clips only: on every clip in a mode it has a file for, never offered.
    pub always: bool,
}

impl Entry {
    /// Every file the entry names, each once.
    pub fn files(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for f in [&self.file, &self.frames, &self.references]
            .into_iter()
            .flatten()
        {
            if !out.contains(&f.as_str()) {
                out.push(f);
            }
        }
        out
    }
}

/// A name the model types, or one part of a file's path: plain.
pub(crate) fn plain(s: &str, extra: &[char]) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && !s.starts_with('.')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c) || extra.contains(&c))
}

/// A file as a server lists it: a `.safetensors` path relative to one of
/// its LoRA folders, every `/`-separated part plain. The guard is lexical,
/// not a canonicalize-inside check, because a LoRA linked in from the hub or
/// another disk is mecha's own layout and must still read; `plain` already
/// rules out `..`, a leading dot and an absolute part.
pub fn file_ok(f: &str) -> bool {
    f.ends_with(".safetensors") && f.len() <= 512 && f.split('/').all(|p| plain(p, &[' ']))
}

/// One entry from its table. `legacy` is the old clips-only file, whose
/// entries are `h3` unless they say otherwise.
fn entry(t: &toml::Table, legacy: bool) -> Result<Entry> {
    let s = |k: &str| t.get(k).and_then(|v| v.as_str()).map(str::trim);
    let name = s("name").unwrap_or_default().to_string();
    if !plain(&name, &[]) || name.contains('.') {
        bail!("`name` must be letters, digits, - or _");
    }
    let about = s("about").unwrap_or_default().to_string();
    if about.is_empty() {
        bail!("`about` is required: it is how the model knows when to use it");
    }
    if about.len() > 300 {
        bail!("`about` is one line, at most 300 characters");
    }
    let model = match (s("model"), legacy) {
        (Some(m), _) => {
            Family::parse(m).ok_or_else(|| anyhow!("`model` is \"qwen-image\" or \"h3\""))?
        }
        (None, true) => Family::H3,
        (None, false) => bail!("`model` is required: \"qwen-image\" or \"h3\""),
    };
    // A LoRA tied to one library character (L6) applies only where the owner
    // turned it on and only with that character in the cast; until that is
    // built, an entry naming one is refused rather than offered everywhere.
    if t.contains_key("character") {
        bail!("`character` is not supported yet, so this entry is left out rather than applied to everyone");
    }
    let mut file = None;
    let mut frames = s("frames").map(str::to_string);
    let mut references = s("references").map(str::to_string);
    match model {
        Family::QwenImage => {
            if frames.is_some() || references.is_some() {
                bail!("`frames` and `references` are H3's modes; a picture LoRA gives `file`");
            }
            file = Some(s("file").ok_or_else(|| anyhow!("give `file`"))?.to_string());
        }
        Family::H3 => {
            if let Some(f) = s("file") {
                frames.get_or_insert_with(|| f.to_string());
                references.get_or_insert_with(|| f.to_string());
            }
            if frames.is_none() && references.is_none() {
                bail!("give `file`, or `frames` and/or `references`");
            }
        }
    }
    for f in file.iter().chain(frames.iter()).chain(references.iter()) {
        if !file_ok(f) {
            bail!("{f:?} is not a .safetensors path under the server's LoRA folders");
        }
    }
    let strength = match t.get("strength") {
        None => 1.0,
        Some(v) => v
            .as_float()
            .or_else(|| v.as_integer().map(|i| i as f64))
            .filter(|s| *s > 0.0 && *s <= 2.0)
            .ok_or_else(|| anyhow!("`strength` is a number above 0, at most 2"))?,
    };
    let trigger = s("trigger").filter(|t| !t.is_empty()).map(str::to_string);
    let always = match t.get("always") {
        None => false,
        Some(v) => v
            .as_bool()
            .ok_or_else(|| anyhow!("`always` is true or false"))?,
    };
    // For pictures the owner is the picker: an always-on entry would override
    // the owner's own list, even one set to none (§3).
    if always && model == Family::QwenImage {
        bail!("`always` is for clips; a LoRA wanted on every picture goes on the chat's list");
    }
    Ok(Entry {
        name,
        about,
        model,
        file,
        frames,
        references,
        strength,
        trigger,
        always,
    })
}

/// The entries in `text`, and a line per entry left out. One bad entry costs
/// itself; names are one namespace across both families.
pub fn parse(text: &str, legacy: bool) -> Result<(Vec<Entry>, Vec<String>)> {
    let table: toml::Table = text
        .parse()
        .map_err(|e| anyhow!("the file does not read: {e}"))?;
    let mut out: Vec<Entry> = Vec::new();
    let mut skipped = Vec::new();
    let entries = table
        .get("lora")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    for (i, v) in entries.iter().enumerate() {
        let Some(t) = v.as_table() else {
            skipped.push(format!("lora #{}: not a table", i + 1));
            continue;
        };
        match entry(t, legacy) {
            Ok(l) if out.iter().any(|o| o.name == l.name) => {
                skipped.push(format!("lora {:?}: a second entry by that name", l.name))
            }
            Ok(l) => out.push(l),
            Err(e) => skipped.push(format!("lora #{}: {e}", i + 1)),
        }
    }
    Ok((out, skipped))
}

/// Which file is the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// `~/.mecha/loras.toml`.
    Shared(PathBuf),
    /// The old clips-only file `[video] loras` names, read for one release.
    Legacy(PathBuf),
    /// Neither: no LoRAs.
    None,
}

/// The registry's file, and an old one still configured that it ignores.
/// The two are never merged: when the new file exists it is the registry,
/// so an entry is never read twice, never half from each.
pub fn source(mecha_home: &Path, legacy: Option<&Path>) -> (Source, Option<PathBuf>) {
    let shared = mecha_home.join(FILE);
    if shared.exists() {
        (Source::Shared(shared), legacy.map(Path::to_path_buf))
    } else if let Some(old) = legacy {
        (Source::Legacy(old.to_path_buf()), None)
    } else {
        (Source::None, None)
    }
}

/// Where an entry stands once its files were looked for and read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Every file found and read as the entry's own family.
    Checked,
    /// Its family's server has no LoRA folders configured.
    NoFolder,
    /// A file in none of the folders.
    Missing(String),
    /// A file that could not be read; retried when it changes.
    Unreadable(String, String),
    /// A file whose keys say another family, neither, or both.
    Mismatch(String, Check),
}

impl Status {
    pub fn is_checked(&self) -> bool {
        *self == Status::Checked
    }

    /// Why the entry is left out, for the operator: the page, stderr and
    /// `doctor`. Names the entry's file, never anything read from it.
    pub fn reason(&self, e: &Entry) -> Option<String> {
        Some(match self {
            Status::Checked => return None,
            Status::NoFolder => format!(
                "left out: {} is not set, so its file cannot be checked",
                e.model.dir_key()
            ),
            Status::Missing(f) => format!(
                "left out: {f:?} is in none of the {} folders",
                e.model.dir_key()
            ),
            Status::Unreadable(f, why) => format!("left out: {f:?} cannot be read ({why})"),
            Status::Mismatch(f, c) => format!(
                "left out: it says `model = \"{}\"`, and {f:?} {}",
                e.model.as_str(),
                match c {
                    Check::Family(other) => format!("is a {} LoRA", other.as_str()),
                    Check::Both => "has the keys of both families".into(),
                    _ => "has the keys of neither family".into(),
                }
            ),
        })
    }
}

/// `rel` under the first of `dirs` that has it.
pub fn find(rel: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|d| d.join(rel))
        .find(|p| p.metadata().is_ok())
}

/// An entry's status against its family's folders (`None` for unset).
pub fn status(e: &Entry, dirs: Option<&[PathBuf]>) -> Status {
    let Some(dirs) = dirs.filter(|d| !d.is_empty()) else {
        return Status::NoFolder;
    };
    for f in e.files() {
        let Some(path) = find(f, dirs) else {
            return Status::Missing(f.to_string());
        };
        match check_file(&path) {
            Check::Family(fam) if fam == e.model => {}
            Check::Unreadable(why) => return Status::Unreadable(f.to_string(), why),
            other => return Status::Mismatch(f.to_string(), other),
        }
    }
    Status::Checked
}

/// The LoRA folders of each family's server: `[image] loras_dir` and
/// `[video] loras_dir`, set by the owner. For a ComfyUI that mecha installed
/// itself, an unset `[image] loras_dir` is that install's `models/loras`,
/// answered from the install record alone (a store read, so `doctor` may use
/// it). A server mecha did not install has no default.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Folders {
    pub image: Option<Vec<PathBuf>>,
    pub video: Option<Vec<PathBuf>>,
}

impl Folders {
    pub fn of(
        mecha_home: &Path,
        image: Option<&crate::imagegen::ImageConfig>,
        video: Option<&crate::videogen::VideoConfig>,
    ) -> Folders {
        let set = |d: &Option<Vec<PathBuf>>| d.clone().filter(|d| !d.is_empty());
        Folders {
            image: image.and_then(|i| {
                set(&i.loras_dir).or_else(|| {
                    crate::comfyui_unit::installed_here(mecha_home)
                        .then(|| vec![crate::comfyui_unit::loras_dir(mecha_home)])
                })
            }),
            video: video.and_then(|v| set(&v.loras_dir)),
        }
    }

    pub fn get(&self, family: Family) -> Option<&[PathBuf]> {
        match family {
            Family::QwenImage => self.image.as_deref(),
            Family::H3 => self.video.as_deref(),
        }
    }
}

/// The registry as read and checked.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub source: Source,
    /// An old file still configured beside the new one, ignored.
    pub ignored: Option<PathBuf>,
    /// Every entry that parsed, in file order, with where it stands.
    pub entries: Vec<(Entry, Status)>,
    /// A line per entry that did not parse.
    pub skipped: Vec<String>,
    /// Why the file itself could not be read, when it could not.
    pub unreadable: Option<String>,
}

impl Loaded {
    /// The registry for this machine: its file, parsed, and each entry's
    /// files looked for and checked.
    pub fn read(mecha_home: &Path, legacy: Option<&Path>, folders: &Folders) -> Loaded {
        let (source, ignored) = source(mecha_home, legacy);
        let mut out = Loaded {
            source: source.clone(),
            ignored,
            entries: Vec::new(),
            skipped: Vec::new(),
            unreadable: None,
        };
        let (path, legacy) = match &source {
            Source::Shared(p) => (p, false),
            Source::Legacy(p) => (p, true),
            Source::None => return out,
        };
        let parsed = std::fs::read_to_string(path)
            .map_err(|e| anyhow!("cannot read it ({e})"))
            .and_then(|text| parse(&text, legacy));
        match parsed {
            Ok((entries, skipped)) => {
                out.skipped = skipped;
                out.entries = entries
                    .into_iter()
                    .map(|e| {
                        let st = status(&e, folders.get(e.model));
                        (e, st)
                    })
                    .collect();
            }
            Err(e) => out.unreadable = Some(e.to_string()),
        }
        out
    }

    /// The checked entries of one family.
    pub fn usable(&self, family: Family) -> impl Iterator<Item = &Entry> {
        self.entries
            .iter()
            .filter(move |(e, st)| e.model == family && st.is_checked())
            .map(|(e, _)| e)
    }

    /// A line for each thing about the registry that costs `family` an
    /// entry, for whoever builds the tool to say once.
    pub fn problems(&self, family: Family) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(why) = &self.unreadable {
            out.push(format!("the registry {why}, so no LoRAs apply"));
        }
        out.extend(self.skipped.iter().cloned());
        for (e, st) in &self.entries {
            if e.model == family {
                if let Some(r) = st.reason(e) {
                    out.push(format!("lora {:?}: {r}", e.name));
                }
            }
        }
        out
    }
}

/// A file with a safetensors header naming `keys` and no tensors: what a
/// test needs for the family check.
#[cfg(test)]
pub(crate) fn write_test_lora(path: &Path, keys: &[&str]) {
    let mut header = serde_json::Map::new();
    for k in keys {
        header.insert(
            k.to_string(),
            serde_json::json!({"dtype": "F32", "shape": [0], "data_offsets": [0, 0]}),
        );
    }
    let header = serde_json::to_vec(&header).unwrap();
    let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
    bytes.extend(header);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
}

/// Key shapes of each family, as the owner's files spell them.
#[cfg(test)]
pub(crate) const QWEN_KEYS: &[&str] =
    &["diffusion_model.transformer_blocks.0.attn.to_q.lora_A.weight"];
#[cfg(test)]
pub(crate) const H3_KEYS: &[&str] = &[
    "diffusion_model.blocks.0.attn.q.lora_A.weight",
    "diffusion_model.token_refiner.blocks.0.lora_A.weight",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "mecha-loras-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_family_is_read_from_the_key_shapes_and_unknown_is_refused() {
        let f = |keys: &[&str]| family_of_keys(keys.iter().copied());
        assert_eq!(f(QWEN_KEYS), Check::Family(Family::QwenImage));
        assert_eq!(
            f(&["transformer.transformer_blocks.3.ff.lora_B.weight"]),
            Check::Family(Family::QwenImage)
        );
        assert_eq!(f(H3_KEYS), Check::Family(Family::H3));
        assert_eq!(
            f(&[
                "diffusion_model.blocks.1.x.lora_A.weight",
                "diffusion_model.audio_patch_proj.lora_A.weight"
            ]),
            Check::Family(Family::H3)
        );
        assert_eq!(
            f(&["lora_unet_blocks_0_attn_q.lora_down.weight"]),
            Check::Family(Family::H3)
        );
        // `blocks.*` alone is many models' spelling, not H3's evidence.
        assert_eq!(
            f(&["diffusion_model.blocks.0.attn.lora_A.weight"]),
            Check::Neither
        );
        // FLUX's diffusers files share Qwen's block name.
        assert_eq!(
            f(&[
                "transformer.transformer_blocks.0.attn.lora_A.weight",
                "transformer.single_transformer_blocks.0.attn.lora_A.weight"
            ]),
            Check::Neither
        );
        assert_eq!(
            f(&["lora_unet_down_blocks_0.lora_down.weight"]),
            Check::Neither
        );
        let both: Vec<&str> = QWEN_KEYS.iter().chain(H3_KEYS).copied().collect();
        assert_eq!(f(&both), Check::Both);
        assert_eq!(f(&["__metadata__"]), Check::Neither);
    }

    #[test]
    fn a_header_is_read_as_a_bounded_prefix_and_an_absurd_length_is_refused() {
        let d = scratch();
        let ok = d.join("ok.safetensors");
        write_test_lora(&ok, H3_KEYS);
        // Trailing tensor bytes are never read.
        let mut bytes = std::fs::read(&ok).unwrap();
        bytes.extend(vec![7u8; 4096]);
        std::fs::write(&ok, &bytes).unwrap();
        assert_eq!(check_uncached(&ok), Check::Family(Family::H3));

        let huge = d.join("huge.safetensors");
        std::fs::write(&huge, u64::MAX.to_le_bytes()).unwrap();
        assert!(
            matches!(check_uncached(&huge), Check::Unreadable(ref w) if w.contains("claims")),
            "{:?}",
            check_uncached(&huge)
        );
        let short = d.join("short.safetensors");
        std::fs::write(&short, [1u8, 2, 3]).unwrap();
        assert!(matches!(check_uncached(&short), Check::Unreadable(_)));
        let cut = d.join("cut.safetensors");
        let mut b = 100u64.to_le_bytes().to_vec();
        b.extend(b"{\"a\":");
        std::fs::write(&cut, b).unwrap();
        assert!(matches!(check_uncached(&cut), Check::Unreadable(ref w) if w.contains("longer")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_file_is_read_again_only_when_its_stat_moves() {
        let d = scratch();
        let p = d.join("x.safetensors");
        // Mid-copy: too short to read.
        std::fs::write(&p, [0u8; 4]).unwrap();
        assert!(matches!(check_file(&p), Check::Unreadable(_)));
        // The copy finishes: the stat moved, so it is read again and joins.
        write_test_lora(&p, QWEN_KEYS);
        assert_eq!(check_file(&p), Check::Family(Family::QwenImage));
        // Gone: a file that passed keeps its result when it cannot be
        // stat'ed, so a flaky mount never flaps the schema.
        std::fs::remove_file(&p).unwrap();
        assert_eq!(check_file(&p), Check::Family(Family::QwenImage));
        let _ = std::fs::remove_dir_all(&d);
    }

    const SAMPLE: &str = r#"
[[lora]]
name = "watercolor"
about = "loose watercolor washes"
model = "qwen-image"
file = "styles/watercolor_qi.safetensors"
strength = 0.7

[[lora]]
name = "wave"
about = "a friendly wave"
model = "h3"
references = "wave_ref.safetensors"
frames = "wave_fl.safetensors"

[[lora]]
name = "always-picture"
about = "x"
model = "qwen-image"
file = "a.safetensors"
always = true

[[lora]]
name = "nomodel"
about = "x"
file = "n.safetensors"

[[lora]]
name = "modes"
about = "x"
model = "qwen-image"
frames = "m.safetensors"

[[lora]]
name = "tied"
about = "x"
model = "h3"
file = "t.safetensors"
character = "wren"

[[lora]]
name = "escape"
about = "x"
model = "h3"
file = "styles/../../secrets.safetensors"

[[lora]]
name = "wave"
about = "a second one"
model = "qwen-image"
file = "w2.safetensors"
"#;

    #[test]
    fn the_shared_registry_needs_a_model_and_keeps_always_to_clips() {
        let (entries, skipped) = parse(SAMPLE, false).unwrap();
        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["watercolor", "wave"]
        );
        assert_eq!(entries[0].model, Family::QwenImage);
        assert_eq!(entries[0].files(), ["styles/watercolor_qi.safetensors"]);
        assert_eq!(
            entries[1].files(),
            ["wave_fl.safetensors", "wave_ref.safetensors"]
        );
        let all = skipped.join("\n");
        for why in [
            "`always` is for clips",
            "`model` is required",
            "H3's modes",
            "`character` is not supported yet",
            "not a .safetensors path",
            "a second entry by that name",
        ] {
            assert!(all.contains(why), "{why}: {all}");
        }
        assert_eq!(skipped.len(), 6, "{all}");
        assert!(parse("[[lora]\n", false).is_err());
    }

    #[test]
    fn the_old_file_reads_as_h3() {
        let (entries, skipped) = parse(
            "[[lora]]\nname = \"wave\"\nabout = \"a wave\"\nfile = \"w.safetensors\"\nalways = true\n",
            true,
        )
        .unwrap();
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(entries[0].model, Family::H3);
        assert!(entries[0].always);
        assert_eq!(entries[0].frames.as_deref(), Some("w.safetensors"));
    }

    #[test]
    fn a_relative_path_is_checked_part_by_part() {
        assert!(file_ok("a.safetensors"));
        assert!(file_ok("sub dir/a b.safetensors"));
        for bad in [
            "../a.safetensors",
            "/abs/a.safetensors",
            "sub/../a.safetensors",
            "sub//a.safetensors",
            ".hidden/a.safetensors",
            "a.ckpt",
            "sub\\a.safetensors",
        ] {
            assert!(!file_ok(bad), "{bad}");
        }
    }

    #[test]
    fn the_new_file_wins_whole_and_the_old_one_is_named_as_ignored() {
        let home = scratch();
        let old = home.join("video-loras.toml");
        std::fs::write(&old, "").unwrap();
        assert_eq!(
            source(&home, Some(&old)),
            (Source::Legacy(old.clone()), None)
        );
        assert_eq!(source(&home, None), (Source::None, None));
        std::fs::write(home.join(FILE), "").unwrap();
        assert_eq!(
            source(&home, Some(&old)),
            (Source::Shared(home.join(FILE)), Some(old))
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn each_entry_is_checked_against_its_own_folders_and_family() {
        let home = scratch();
        let image = home.join("image-loras");
        let video = home.join("video-loras");
        let extra = home.join("video-extra");
        write_test_lora(&image.join("styles/watercolor_qi.safetensors"), QWEN_KEYS);
        write_test_lora(&video.join("wave_fl.safetensors"), H3_KEYS);
        // A second folder, as `extra_model_paths.yaml` adds one.
        write_test_lora(&extra.join("wave_ref.safetensors"), H3_KEYS);
        std::fs::write(
            home.join(FILE),
            r#"
[[lora]]
name = "watercolor"
about = "loose watercolor washes"
model = "qwen-image"
file = "styles/watercolor_qi.safetensors"

[[lora]]
name = "wave"
about = "a friendly wave"
model = "h3"
references = "wave_ref.safetensors"
frames = "wave_fl.safetensors"

[[lora]]
name = "liar"
about = "says h3, is a picture LoRA"
model = "h3"
file = "styles/watercolor_qi.safetensors"

[[lora]]
name = "gone"
about = "x"
model = "h3"
file = "gone.safetensors"
"#,
        )
        .unwrap();
        write_test_lora(&video.join("styles/watercolor_qi.safetensors"), QWEN_KEYS);
        let folders = Folders {
            image: Some(vec![image.clone()]),
            video: Some(vec![video.clone(), extra.clone()]),
        };
        let l = Loaded::read(&home, None, &folders);
        let st: Vec<(&str, &Status)> = l
            .entries
            .iter()
            .map(|(e, s)| (e.name.as_str(), s))
            .collect();
        assert_eq!(st[0], ("watercolor", &Status::Checked));
        assert_eq!(st[1], ("wave", &Status::Checked));
        assert!(matches!(
            st[2].1,
            Status::Mismatch(_, Check::Family(Family::QwenImage))
        ));
        assert_eq!(st[3].1, &Status::Missing("gone.safetensors".into()));
        assert_eq!(
            l.usable(Family::H3)
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["wave"]
        );
        let problems = l.problems(Family::H3).join("\n");
        assert!(
            problems.contains("is a qwen-image LoRA") && problems.contains("in none of"),
            "{problems}"
        );
        assert!(!l.problems(Family::QwenImage).join("").contains("liar"));

        // With a family's folders unset, its entries are left out and the key named.
        let none = Loaded::read(
            &home,
            None,
            &Folders {
                image: None,
                ..folders
            },
        );
        assert_eq!(none.entries[0].1, Status::NoFolder);
        assert!(none.problems(Family::QwenImage)[0].contains("[image] loras_dir"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_registry_that_does_not_read_is_said_and_applies_nothing() {
        let home = scratch();
        std::fs::write(home.join(FILE), "[[lora]\n").unwrap();
        let l = Loaded::read(&home, None, &Folders::default());
        assert!(l.entries.is_empty());
        assert!(l.problems(Family::H3)[0].contains("does not read"));
        let _ = std::fs::remove_dir_all(&home);
    }
}
