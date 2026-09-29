//! Personas: characters the owner authors and talks to, kept apart from the
//! assistant.
//!
//! `docs/PERSONA-DESIGN.md` is the contract; the section numbers cited here
//! are its. This module is the **store** (§4, build step 1): the folder under
//! `~/.mecha/personas/`, what each file in it means, who may write it, and the
//! links by name between a persona and its relationship templates, groups,
//! library character and voice profile. Chats, memory and the web page are
//! later steps, and read the store through this module.
//!
//! Who writes what (§4.4):
//!
//! - **The owner writes** `persona.toml`, the Markdown, relationship
//!   templates, `groups.toml` and `about-me.md`. Code never rewrites those
//!   files — a template is copied in once, and a new group is appended — so
//!   the owner's comments survive, the charter's rule.
//! - **The harness writes** `state.toml` (status, provenance, the browse lock,
//!   the version) and `versions/`. Keeping machine state out of the owner's
//!   file is what lets `lock` and a version bump happen without re-serialising
//!   a file somebody annotated.
//!
//! Separation from the assistant is by **location** (§3.2): persona
//! transcripts will live under `<persona>/sessions/`, which no reader of
//! `~/.mecha/sessions/` scans — a session kind cannot carry that, because
//! `runlog::Scan::admits` admits every kind it does not know.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::imagelib::{self, write_atomic};
pub use crate::imagelib::{Origin, Status};

/// A name — persona, relationship, group or voice: `[a-z0-9][a-z0-9_-]*`.
pub const MAX_NAME: usize = 64;
/// A display name, as the page and the prompt show it.
pub const MAX_DISPLAY: usize = 80;
/// Any one Markdown file the store reads whole. Bounds a read, not a prompt:
/// the prompt's budget is the chat's business (§8.2).
pub const MAX_PROSE_BYTES: u64 = 64 * 1024;

/// Folders beside the personas at the top of the store. A persona's folder
/// sits among them, so none of these can name one.
pub const RESERVED: [&str; 7] = [
    "files",
    "groups",
    "relationships",
    "voices",
    "scenarios",
    "removed",
    "sessions",
];

/// The starters of §5, shipped in the binary and copied into the owner's
/// `relationships/` once (`seed_starters`).
pub const STARTERS: [(&str, &str); 10] = [
    ("character", include_str!("persona/starters/character.md")),
    ("coach", include_str!("persona/starters/coach.md")),
    (
        "collaborator",
        include_str!("persona/starters/collaborator.md"),
    ),
    ("colleague", include_str!("persona/starters/colleague.md")),
    (
        "devils_advocate",
        include_str!("persona/starters/devils_advocate.md"),
    ),
    ("friend", include_str!("persona/starters/friend.md")),
    ("reflective", include_str!("persona/starters/reflective.md")),
    ("romantic", include_str!("persona/starters/romantic.md")),
    ("simulated", include_str!("persona/starters/simulated.md")),
    ("teacher", include_str!("persona/starters/teacher.md")),
];

/// Which starters have been offered, one name a line, in `relationships/`.
/// A starter the owner deleted is not copied back; one added by an upgrade is.
const SEEDED: &str = ".seeded";

pub fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        && name.as_bytes()[0].is_ascii_alphanumeric();
    if !ok {
        bail!(
            "`{name}` is not a valid name: use lowercase letters, digits, `-` and `_`, \
             starting with a letter or digit, at most {MAX_NAME} characters"
        );
    }
    Ok(())
}

/// A persona's name: a valid name that is not one of the store's own folders.
pub fn validate_persona_name(name: &str) -> Result<()> {
    validate_name(name)?;
    if RESERVED.contains(&name) {
        bail!("`{name}` is reserved for the store's own `{name}/` folder; pick another name");
    }
    Ok(())
}

// ─── persona.toml: the owner's ─────────────────────────────────────────────

/// One name or a list of them — `relationship = "colleague"` and
/// `relationship = ["colleague", "teacher"]` both read (§5: one, several or
/// none).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(from = "OneOrMany", into = "Vec<String>")]
pub struct Names(pub Vec<String>);

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl From<OneOrMany> for Names {
    fn from(v: OneOrMany) -> Names {
        match v {
            OneOrMany::One(s) => Names(vec![s]),
            OneOrMany::Many(v) => Names(v),
        }
    }
}

impl From<Names> for Vec<String> {
    fn from(n: Names) -> Vec<String> {
        n.0
    }
}

/// `persona.toml`. Unknown keys fail the load by name. An unknown *value* in
/// a closed set loads as its narrowest variant and is reported
/// ([`Persona::notes`]), so a typo narrows a persona and says so rather than
/// widening it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// How the persona is shown and addressed; the folder name when empty.
    #[serde(default)]
    pub display: String,
    /// Relationship templates in `relationships/` (§5).
    #[serde(default)]
    pub relationship: Names,
    /// An image-library character: the persona's portrait and "self" (§8.6).
    #[serde(default)]
    pub character: Option<String>,
    /// A profile in `voices/` (§11).
    #[serde(default)]
    pub voice: Option<String>,
    /// Groups declared in `groups.toml` this persona belongs to (§4.5).
    #[serde(default)]
    pub groups: Vec<String>,
    /// The model the persona is pinned to (§12.6); unset is the default.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub tools: Tools,
    #[serde(default)]
    pub safety: Safety,
    #[serde(default)]
    pub files: FilesSettings,
    #[serde(default)]
    pub memory: MemorySettings,
}

/// `[tools]`: what the owner wants the persona to have. Intersected, when a
/// chat is built, with an eligibility every tool must declare and that
/// defaults to ineligible (§3.3) — no file can widen it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tools {
    #[serde(default)]
    pub allow: Vec<String>,
}

fn yes() -> bool {
    true
}

/// `[safety]` (§12): every switch on by default, break reminders excepted.
/// The owner turns one off, per persona; a template or a model never does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Safety {
    #[serde(default = "yes")]
    pub disclosure: bool,
    #[serde(default = "yes")]
    pub crisis: bool,
    #[serde(default = "yes")]
    pub dose: bool,
    #[serde(default)]
    pub breaks: bool,
    #[serde(default = "yes")]
    pub farewell: bool,
    #[serde(default = "yes")]
    pub reanchor: bool,
}

impl Default for Safety {
    fn default() -> Safety {
        Safety {
            disclosure: true,
            crisis: true,
            dose: true,
            breaks: false,
            farewell: true,
            reanchor: true,
        }
    }
}

impl Safety {
    /// The switches that are off and are on by default — what `show` names
    /// loudly, because each is a protection removed.
    pub fn switched_off(&self) -> Vec<&'static str> {
        [
            ("disclosure", self.disclosure),
            ("crisis", self.crisis),
            ("dose", self.dose),
            ("farewell", self.farewell),
            ("reanchor", self.reanchor),
        ]
        .into_iter()
        .filter(|(_, on)| !on)
        .map(|(n, _)| n)
        .collect()
    }
}

/// Where a persona's answers may come from (§10.4, D16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Answers {
    /// Its files and its tools.
    #[default]
    Open,
    /// Only its files; web tools withheld from the chat. The narrower, so an
    /// unreadable value lands here.
    #[serde(other)]
    Files,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesSettings {
    #[serde(default)]
    pub answers: Answers,
}

/// Which facts about the owner a persona reads (§9.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UserFacts {
    /// Only what it learned itself.
    Own,
    /// Its own, plus what the owner shared with everyone or its groups.
    #[default]
    Shared,
    /// None. The narrowest, so an unreadable value lands here.
    #[serde(other)]
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemorySettings {
    #[serde(default = "yes")]
    pub episodic: bool,
    #[serde(default = "yes")]
    pub semantic: bool,
    #[serde(default)]
    pub user_facts: UserFacts,
    /// Whether the everyone- and group-level `about-me.md` are read.
    #[serde(default = "yes")]
    pub about_me: bool,
    /// §9.12: sections of `identity.md` evolve on their own. Off by default.
    #[serde(default)]
    pub self_update: bool,
    /// Sections of `identity.md` that never evolve; `Core` never does anyway.
    #[serde(default)]
    pub fixed: Vec<String>,
}

impl Default for MemorySettings {
    fn default() -> MemorySettings {
        MemorySettings {
            episodic: true,
            semantic: true,
            user_facts: UserFacts::default(),
            about_me: true,
            self_update: false,
            fixed: Vec::new(),
        }
    }
}

/// Closed-set values in a `persona.toml` that this binary does not know.
/// `serde(other)` has already narrowed them; this is what makes it audible.
fn unknown_values(raw: &toml::Table) -> Vec<String> {
    let checks: [(&str, &str, &[&str]); 2] = [
        ("files", "answers", &["files", "open"]),
        ("memory", "user_facts", &["own", "shared", "off"]),
    ];
    let mut out = Vec::new();
    for (table, key, known) in checks {
        let value = raw
            .get(table)
            .and_then(|t| t.get(key))
            .and_then(|v| v.as_str());
        if let Some(v) = value {
            if !known.contains(&v) {
                out.push(format!(
                    "[{table}] {key} = \"{v}\" is not one of {}; read as the narrowest",
                    known.join(", ")
                ));
            }
        }
    }
    out
}

// ─── state.toml: the harness's ─────────────────────────────────────────────

fn one() -> u32 {
    1
}

/// `state.toml`: machine-written, rewritten whole. A missing or damaged file
/// loads as an unapproved persona of unknown provenance — unknown is never
/// clean — and `mecha persona approve` is the way out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub status: Status,
    #[serde(default)]
    pub origin: Origin,
    /// Hides the persona and its chats while browsing; never encryption, and
    /// nothing is withheld from a chat or the memory writer (§8.3).
    #[serde(default)]
    pub locked: bool,
    #[serde(default = "one")]
    pub version: u32,
    /// The [`content_digest`] `version` was taken at; empty before the first.
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub updated: String,
}

impl Default for State {
    fn default() -> State {
        State {
            status: Status::Candidate,
            origin: Origin::ModelUntrusted,
            locked: false,
            version: 1,
            digest: String::new(),
            created: String::new(),
            updated: String::new(),
        }
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ─── Markdown ──────────────────────────────────────────────────────────────

/// `text` with every `<!-- … -->` removed: what reaches a prompt. An
/// unterminated comment swallows the rest of the file — the fail-closed
/// reading, since a comment is where an evolved section keeps the text it
/// replaced (§9.12) — and `true` says it happened.
pub fn strip_comments(text: &str) -> (String, bool) {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start + 4..].find("-->") {
            Some(end) => rest = &rest[start + 4 + end + 3..],
            None => return (out, true),
        }
    }
    out.push_str(rest);
    (out, false)
}

/// One `## ` section of a Markdown file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub heading: String,
    pub body: String,
}

/// The level-two sections of `text`, comments stripped first so a heading
/// inside one is not a section, and fenced code skipped. Text before the
/// first `## ` is not a section.
pub fn sections(text: &str) -> Vec<Section> {
    let (text, _) = strip_comments(text);
    let mut out: Vec<Section> = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        if !fenced {
            if let Some(h) = line.strip_prefix("## ") {
                out.push(Section {
                    heading: h.trim().to_string(),
                    body: String::new(),
                });
                continue;
            }
        }
        if let Some(s) = out.last_mut() {
            s.body.push_str(line);
            s.body.push('\n');
        }
    }
    for s in &mut out {
        s.body = s.body.trim().to_string();
    }
    out
}

/// A template's suggestions, from its `+++` front matter (§5): copied into a
/// persona created from it, and nothing after that.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suggest {
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub answers: Option<Answers>,
}

/// Split `+++ … +++` front matter off a template. No front matter is none.
fn front_matter(text: &str) -> Result<(Suggest, &str)> {
    let Some(rest) = text.strip_prefix("+++\n") else {
        return Ok((Suggest::default(), text));
    };
    let (head, body) = if let Some(body) = rest.strip_prefix("+++\n") {
        ("", body)
    } else {
        let end = rest
            .find("\n+++\n")
            .map(|i| (i, i + 5))
            .or_else(|| rest.strip_suffix("\n+++").map(|h| (h.len(), rest.len())))
            .ok_or_else(|| anyhow!("front matter opened with `+++` is never closed"))?;
        (&rest[..end.0], &rest[end.1..])
    };
    let suggest: Suggest = parse_toml(head).context("reading the front matter")?;
    Ok((suggest, body))
}

// ─── The loaded store ──────────────────────────────────────────────────────

/// Something in the store that would not load, kept so the CLI can say so:
/// a persona that silently fails to load looks exactly like one never made.
#[derive(Debug, Clone)]
pub struct LoadError {
    pub path: PathBuf,
    pub why: String,
}

/// A relationship template (§5).
#[derive(Debug, Clone, PartialEq)]
pub struct Relationship {
    pub name: String,
    pub suggest: Suggest,
    /// The file as written, front matter and comments included — what a
    /// version digests and snapshots.
    pub raw: String,
    /// Whether it is still byte-for-byte a shipped starter.
    pub starter: bool,
}

/// `voices/<name>/profile.toml` (§11). Read and linked in this step; bound
/// to a call in step 7.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceProfile {
    /// A voice the TTS server already lists, when there is no clip.
    #[serde(default)]
    pub voice: Option<String>,
    /// A reference clip beside this file: a plain file name.
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub speed: Option<f64>,
    #[serde(default)]
    pub exaggeration: Option<f64>,
    #[serde(default)]
    pub cfg_weight: Option<f64>,
}

/// One group, as `groups.toml` declares it. Membership lives in each
/// persona's `groups`, once.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupDecl {
    #[serde(default)]
    pub description: String,
}

/// A persona as loaded.
#[derive(Debug, Clone, PartialEq)]
pub struct Persona {
    /// The folder's name.
    pub name: String,
    pub settings: Settings,
    pub state: State,
    /// `identity.md` as written.
    pub identity: String,
    /// `motivation.md` as written; empty when absent.
    pub motivation: String,
    /// Loaded, but not as written: an unknown value narrowed, a missing
    /// `state.toml`. Shown beside the persona, never swallowed.
    pub notes: Vec<String>,
}

impl Persona {
    pub fn display(&self) -> &str {
        if self.settings.display.trim().is_empty() {
            &self.name
        } else {
            self.settings.display.trim()
        }
    }
}

/// Every persona on the machine and what they link to, sorted by name.
#[derive(Debug, Clone, Default)]
pub struct Store {
    dir: PathBuf,
    personas: Vec<Persona>,
    relationships: BTreeMap<String, Relationship>,
    voices: BTreeMap<String, VoiceProfile>,
    groups: BTreeMap<String, GroupDecl>,
    errors: Vec<LoadError>,
}

fn read_prose(path: &Path) -> Result<String> {
    let meta = std::fs::metadata(path).with_context(|| format!("reading {}", path.display()))?;
    if meta.len() > MAX_PROSE_BYTES {
        bail!(
            "{} is {} KB; files here are capped at {} KB",
            path.display(),
            meta.len() / 1024,
            MAX_PROSE_BYTES / 1024
        );
    }
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    if let Some(c) = forbidden_control(&text) {
        bail!(
            "{} holds a control character (U+{:04X}); only newlines and tabs are allowed",
            path.display(),
            c as u32
        );
    }
    Ok(text)
}

/// The first control character in `text` a terminal or page could act on.
/// A carriage return is allowed only as half of a CRLF: alone it returns
/// the cursor to overwrite the line it is on.
fn forbidden_control(text: &str) -> Option<char> {
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() != Some(&'\n') {
                return Some(c);
            }
        } else if is_forbidden_control(c) {
            return Some(c);
        }
    }
    None
}

/// Parse a TOML file of this store with every string in it — key or value,
/// however deep — held to the prose rule. A TOML escape (`"\u001b"`) gets a
/// control character past [`read_prose`]'s check on the file's bytes, and
/// any field can reach the terminal (`show`, `problems`, the approval
/// screen), so the check is over the parsed values, not a list of fields.
fn parse_toml<T: serde::de::DeserializeOwned>(text: &str) -> Result<T> {
    fn walk(v: &toml::Value) -> Option<char> {
        match v {
            toml::Value::String(s) => forbidden_control(s),
            toml::Value::Array(a) => a.iter().find_map(walk),
            toml::Value::Table(t) => t
                .iter()
                .find_map(|(k, v)| forbidden_control(k).or_else(|| walk(v))),
            _ => None,
        }
    }
    let table: toml::Table = toml::from_str(text)?;
    if let Some(c) = walk(&toml::Value::Table(table.clone())) {
        bail!(
            "a value holds a control character (U+{:04X}); only newlines and tabs are allowed",
            c as u32
        );
    }
    Ok(table.try_into()?)
}

/// A control character a terminal or page could act on. Prose from this
/// store is printed raw before a `[y/N]` (`mecha persona approve`), so text
/// that could repaint the screen would repaint the review of itself —
/// `imagelib::validate_text`'s rule, with tabs allowed for Markdown.
fn is_forbidden_control(c: char) -> bool {
    c.is_control() && !matches!(c, '\n' | '\t')
}

fn dir_name(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
}

impl Store {
    /// `~/.mecha/personas`.
    pub fn default_dir() -> Result<PathBuf> {
        Ok(crate::work::mecha_home()?.join("personas"))
    }

    /// Read the whole store. Best-effort per item and **read-only**: a
    /// missing directory is an empty store, and a broken file is an entry in
    /// [`Store::errors`], never a silent absence.
    pub fn load(dir: &Path) -> Store {
        let mut store = Store {
            dir: dir.to_path_buf(),
            ..Store::default()
        };
        store.load_relationships();
        store.load_voices();
        store.load_groups();
        let Ok(items) = std::fs::read_dir(dir) else {
            return store;
        };
        let mut dirs: Vec<PathBuf> = items
            .flatten()
            .map(|i| i.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for path in dirs {
            let Some(name) = dir_name(&path) else {
                continue;
            };
            let manifest = path.join("persona.toml");
            if name.starts_with('.') {
                continue;
            }
            if RESERVED.contains(&name.as_str()) {
                // The store's own folder. One holding a persona is a persona
                // nobody can address, which is worth saying.
                if manifest.is_file() {
                    store.errors.push(LoadError {
                        path: manifest,
                        why: format!("`{name}` is a reserved name; a persona cannot live here"),
                    });
                }
                continue;
            }
            if !manifest.is_file() {
                continue;
            }
            match load_persona(&path, &name) {
                Ok(p) => store.personas.push(p),
                Err(e) => store.errors.push(LoadError {
                    path: manifest,
                    why: format!("{e:#}"),
                }),
            }
        }
        store
    }

    fn load_relationships(&mut self) {
        let Ok(items) = std::fs::read_dir(self.dir.join("relationships")) else {
            return;
        };
        for item in items.flatten() {
            let path = item.path();
            let Some(file) = dir_name(&path) else {
                continue;
            };
            let Some(name) = file.strip_suffix(".md") else {
                continue;
            };
            if file.starts_with('.') || !path.is_file() {
                continue;
            }
            let loaded = validate_name(name).and_then(|()| {
                let raw = read_prose(&path)?;
                let (suggest, _) = front_matter(&raw)?;
                let starter = STARTERS.iter().any(|(n, t)| *n == name && *t == raw);
                Ok(Relationship {
                    name: name.to_string(),
                    suggest,
                    raw,
                    starter,
                })
            });
            match loaded {
                Ok(r) => {
                    self.relationships.insert(r.name.clone(), r);
                }
                Err(e) => self.errors.push(LoadError {
                    path,
                    why: format!("{e:#}"),
                }),
            }
        }
    }

    fn load_voices(&mut self) {
        let Ok(items) = std::fs::read_dir(self.dir.join("voices")) else {
            return;
        };
        for item in items.flatten() {
            let path = item.path();
            let Some(name) = dir_name(&path) else {
                continue;
            };
            let file = path.join("profile.toml");
            if name.starts_with('.') || !file.is_file() {
                continue;
            }
            let loaded = validate_name(&name).and_then(|()| {
                let profile: VoiceProfile = parse_toml(&read_prose(&file)?)?;
                match (&profile.voice, &profile.reference) {
                    (None, None) => bail!("a profile needs a `voice` or a `reference` clip"),
                    (_, Some(clip)) => {
                        if clip.contains('/') || clip.contains('\\') || clip.starts_with('.') {
                            bail!("reference `{clip}` must be a file beside profile.toml");
                        }
                        if !path.join(clip).is_file() {
                            bail!("reference clip `{clip}` is not in {}", path.display());
                        }
                    }
                    _ => {}
                }
                Ok(profile)
            });
            match loaded {
                Ok(p) => {
                    self.voices.insert(name, p);
                }
                Err(e) => self.errors.push(LoadError {
                    path: file,
                    why: format!("{e:#}"),
                }),
            }
        }
    }

    fn load_groups(&mut self) {
        let path = self.dir.join("groups.toml");
        if !path.is_file() {
            return;
        }
        let loaded = read_prose(&path).and_then(|raw| {
            let groups: BTreeMap<String, GroupDecl> = parse_toml(&raw)?;
            for name in groups.keys() {
                validate_name(name)?;
            }
            Ok(groups)
        });
        match loaded {
            Ok(g) => self.groups = g,
            Err(e) => self.errors.push(LoadError {
                path,
                why: format!("{e:#}"),
            }),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn all(&self) -> &[Persona] {
        &self.personas
    }

    pub fn get(&self, name: &str) -> Option<&Persona> {
        self.personas.iter().find(|p| p.name == name)
    }

    /// What a browsing surface lists: locked personas only once the library
    /// is unlocked. The lock hides; it never withholds (§8.3).
    pub fn visible(&self, unlocked: bool) -> impl Iterator<Item = &Persona> {
        self.personas
            .iter()
            .filter(move |p| unlocked || !p.state.locked)
    }

    pub fn relationships(&self) -> &BTreeMap<String, Relationship> {
        &self.relationships
    }

    pub fn voices(&self) -> &BTreeMap<String, VoiceProfile> {
        &self.voices
    }

    pub fn groups(&self) -> &BTreeMap<String, GroupDecl> {
        &self.groups
    }

    pub fn errors(&self) -> &[LoadError] {
        &self.errors
    }

    /// The persona's folder — for the owner's doors (the CLI, the page).
    /// **Never a jail root:** `sessions/`, `state.toml` and, later,
    /// `memory.db` sit in it beside `files/`. A tool reading a persona's
    /// material is jailed to [`Store::files_roots`] instead.
    pub fn persona_dir(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// The `files/` folders `p` may read, most specific first: its own, each
    /// group it joins that `groups.toml` declares, and everyone's (§10.2).
    /// These are the only roots a persona's file tools may resolve in —
    /// joined here, not checked: canonicalising and containing a path in one
    /// (and a root that does not exist yet) is the resolver's job when step 3
    /// builds it. A group that is not declared
    /// contributes nothing: a broken link grants no access.
    pub fn files_roots(&self, p: &Persona) -> Vec<PathBuf> {
        let mut roots = vec![self.dir.join(&p.name).join("files")];
        for g in &p.settings.groups {
            if self.groups.contains_key(g) {
                roots.push(self.dir.join("groups").join(g).join("files"));
            }
        }
        roots.push(self.dir.join("files"));
        roots
    }

    /// Where this persona's chats will be kept (§3.2) — apart from
    /// `~/.mecha/sessions/`, which is the separation.
    pub fn sessions_dir(&self, name: &str) -> PathBuf {
        self.dir.join(name).join("sessions")
    }

    /// Everything wrong with how `p` links to the rest of the store and the
    /// image library, each named. Empty is a persona that can run as written.
    pub fn problems(&self, p: &Persona, lib: &imagelib::Library) -> Vec<String> {
        let mut out = Vec::new();
        if p.state.status != Status::Approved {
            out.push(format!(
                "not approved — `mecha persona approve {}` after reading it",
                p.name
            ));
        }
        for r in &p.settings.relationship.0 {
            if !self.relationships.contains_key(r) {
                out.push(format!(
                    "names relationship `{r}`, which is not in {}",
                    self.dir.join("relationships").display()
                ));
            }
        }
        if let Some(c) = &p.settings.character {
            match lib.get(imagelib::Kind::Character, c) {
                None => out.push(format!(
                    "names character `{c}`, which is not in the image library"
                )),
                Some(e) if e.status != Status::Approved => out.push(format!(
                    "names character `{c}`, which is a candidate the owner has not approved"
                )),
                Some(_) => {}
            }
        }
        if let Some(v) = &p.settings.voice {
            if !self.voices.contains_key(v) {
                out.push(format!(
                    "names voice `{v}`, which is not a profile in {}",
                    self.dir.join("voices").display()
                ));
            }
        }
        for g in &p.settings.groups {
            if !self.groups.contains_key(g) {
                out.push(format!(
                    "joins group `{g}`, which groups.toml does not declare"
                ));
            }
        }
        let secs = sections(&p.identity);
        match secs.iter().find(|s| s.heading.eq_ignore_ascii_case("core")) {
            None => out.push(
                "identity.md has no `## Core` section — the part that never changes and \
                 is re-read in long chats"
                    .to_string(),
            ),
            Some(core) if core.body.is_empty() => {
                out.push("identity.md's `## Core` section is empty".to_string())
            }
            Some(_) => {}
        }
        for f in &p.settings.memory.fixed {
            if !secs.iter().any(|s| s.heading.eq_ignore_ascii_case(f)) {
                out.push(format!(
                    "[memory] fixed names section `{f}`, which identity.md does not have"
                ));
            }
        }
        for (file, text) in [
            ("identity.md", &p.identity),
            ("motivation.md", &p.motivation),
        ] {
            if strip_comments(text).1 {
                out.push(format!(
                    "{file} has an unclosed `<!--`: everything after it is left out"
                ));
            }
        }
        out
    }

    /// The bytes a version is taken over: the persona's own files and the
    /// relationship templates it names, framed by name so no two layouts
    /// digest alike. `Err` when a named template is missing — there is no
    /// version of a persona whose prompt cannot be rendered.
    fn version_files(&self, p: &Persona) -> Result<Vec<(String, String)>> {
        let dir = self.persona_dir(&p.name);
        let mut files = vec![
            (
                "persona.toml".to_string(),
                read_prose(&dir.join("persona.toml"))?,
            ),
            ("identity.md".to_string(), p.identity.clone()),
            ("motivation.md".to_string(), p.motivation.clone()),
        ];
        for r in &p.settings.relationship.0 {
            let t = self
                .relationships
                .get(r)
                .ok_or_else(|| anyhow!("names relationship `{r}`, which is not there"))?;
            files.push((format!("relationships/{r}.md"), t.raw.clone()));
        }
        Ok(files)
    }

    /// The digest of what a chat with `p` would render from, as the files
    /// stand now.
    pub fn content_digest(&self, p: &Persona) -> Result<String> {
        Ok(digest_of(&self.version_files(p)?))
    }
}

fn digest_of(files: &[(String, String)]) -> String {
    let mut h = sha2::Sha256::new();
    for (name, text) in files {
        h.update(format!("{}\n{}\n", name, text.len()).as_bytes());
        h.update(text.as_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn load_persona(dir: &Path, name: &str) -> Result<Persona> {
    validate_persona_name(name)?;
    let raw = read_prose(&dir.join("persona.toml"))?;
    let settings: Settings = parse_toml(&raw)?;
    let mut notes = unknown_values(&toml::from_str(&raw)?);
    if settings.display.chars().count() > MAX_DISPLAY || settings.display.contains(['\n', '\t']) {
        bail!("display is one line of at most {MAX_DISPLAY} characters");
    }
    for r in &settings.relationship.0 {
        validate_name(r).context("in `relationship`")?;
    }
    for g in &settings.groups {
        validate_name(g).context("in `groups`")?;
    }
    let state_path = dir.join("state.toml");
    let state = if state_path.is_file() {
        match std::fs::read_to_string(&state_path)
            .map_err(anyhow::Error::from)
            .and_then(|s| toml::from_str::<State>(&s).map_err(anyhow::Error::from))
        {
            Ok(s) => s,
            Err(e) => {
                notes.push(format!(
                    "state.toml does not read ({e:#}); treated as unapproved"
                ));
                State::default()
            }
        }
    } else {
        notes.push("no state.toml: made outside mecha, so unapproved until you approve it".into());
        State::default()
    };
    let identity_path = dir.join("identity.md");
    if !identity_path.is_file() {
        bail!("no identity.md");
    }
    let identity = read_prose(&identity_path)?;
    let motivation_path = dir.join("motivation.md");
    let motivation = if motivation_path.is_file() {
        read_prose(&motivation_path)?
    } else {
        String::new()
    };
    Ok(Persona {
        name: name.to_string(),
        settings,
        state,
        identity,
        motivation,
        notes,
    })
}

// ─── Writing ───────────────────────────────────────────────────────────────

const ABOUT_ME: &str = "\
<!-- What personas may know about you. Every persona with `about_me = true`
     (the default) reads this file at the level it sits at: the top of the
     store is everyone, groups/<group>/about-me.md is that group only.
     No model writes a line here. Comments like this one are never read. -->
";

const GROUPS_TOML: &str = "\
# Groups of personas that share an about-me.md and a files/ folder
# (groups/<group>/). Declare a group here; a persona joins it with
# `groups = [\"work\"]` in its own persona.toml — membership lives there, once.
#
# [work]
# description = \"the kelp project\"
";

/// Create the store's shared folders and files where missing, and offer the
/// starters. Never overwrites anything.
pub fn ensure_layout(dir: &Path) -> Result<()> {
    for sub in ["files", "groups", "relationships", "voices", "scenarios"] {
        std::fs::create_dir_all(dir.join(sub))
            .with_context(|| format!("creating {}", dir.join(sub).display()))?;
    }
    write_new(&dir.join("about-me.md"), ABOUT_ME)?;
    write_new(&dir.join("groups.toml"), GROUPS_TOML)?;
    let store = Store::load(dir);
    for g in store.groups.keys() {
        ensure_group_dirs(dir, g)?;
    }
    seed_starters(dir)?;
    Ok(())
}

fn ensure_group_dirs(dir: &Path, group: &str) -> Result<()> {
    let g = dir.join("groups").join(group);
    std::fs::create_dir_all(g.join("files"))?;
    write_new(&g.join("about-me.md"), ABOUT_ME)?;
    Ok(())
}

/// Write `text` to `path` only if nothing is there. `Ok(false)` when
/// something was.
fn write_new(path: &Path, text: &str) -> Result<bool> {
    use std::io::Write;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut f) => {
            f.write_all(text.as_bytes())
                .with_context(|| format!("writing {}", path.display()))?;
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e).with_context(|| format!("creating {}", path.display())),
    }
}

/// Copy each starter the owner has not yet been offered into
/// `relationships/`. A starter is offered once: deleting it keeps it
/// deleted, editing it keeps the edit, and an upgrade that ships a new one
/// adds just that one. Returns the names copied.
pub fn seed_starters(dir: &Path) -> Result<Vec<String>> {
    let rel = dir.join("relationships");
    std::fs::create_dir_all(&rel)?;
    let seeded_path = rel.join(SEEDED);
    let mut seeded: BTreeSet<String> = match std::fs::read_to_string(&seeded_path) {
        Ok(s) => s
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_string)
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", seeded_path.display())),
    };
    let mut copied = Vec::new();
    let mut offered = false;
    for (name, text) in STARTERS {
        if seeded.contains(name) {
            continue;
        }
        if write_new(&rel.join(format!("{name}.md")), text)? {
            copied.push(name.to_string());
        }
        // Offered even when the owner already had a file of that name.
        seeded.insert(name.to_string());
        offered = true;
    }
    if offered || !seeded_path.exists() {
        let mut text: String = seeded.iter().map(|n| format!("{n}\n")).collect();
        text.insert_str(
            0,
            "# Starters already offered; one deleted from here is offered again.\n",
        );
        write_atomic(&seeded_path, text.as_bytes())?;
    }
    Ok(copied)
}

/// Declare a group: appended to `groups.toml` (the owner's comments kept),
/// with its folder and about-me made.
pub fn add_group(dir: &Path, name: &str, description: &str) -> Result<()> {
    validate_name(name)?;
    ensure_layout(dir)?;
    let store = Store::load(dir);
    if let Some(e) = store
        .errors
        .iter()
        .find(|e| e.path.ends_with("groups.toml"))
    {
        bail!("groups.toml does not read ({}); fix it first", e.why);
    }
    if store.groups.contains_key(name) {
        bail!("group `{name}` is already declared");
    }
    let path = dir.join("groups.toml");
    let mut text = std::fs::read_to_string(&path)?;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&format!("\n[{name}]\n"));
    if !description.trim().is_empty() {
        text.push_str(&format!(
            "description = {}\n",
            toml::Value::String(description.trim().to_string())
        ));
    }
    write_atomic(&path, text.as_bytes())?;
    ensure_group_dirs(dir, name)
}

/// What a new persona is made of.
#[derive(Debug, Clone, Default)]
pub struct NewPersona {
    pub name: String,
    pub display: String,
    pub relationships: Vec<String>,
    pub character: Option<String>,
    pub voice: Option<String>,
    pub groups: Vec<String>,
    pub locked: bool,
    /// Who wrote it. Only `Owner` is approved on creation; the default is
    /// `ModelUntrusted`, so a door that forgets to say makes a candidate.
    pub origin: Origin,
}

fn quoted(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

fn quoted_list(items: &[String]) -> String {
    let inner: Vec<String> = items.iter().map(|s| quoted(s)).collect();
    format!("[{}]", inner.join(", "))
}

/// The commented `persona.toml` a new persona starts with. Every switch is
/// written out at its default, so the owner reads what is on rather than
/// having to know it.
fn render_settings(new: &NewPersona, tools: &[String], answers: Answers) -> String {
    let opt = |key: &str, v: &Option<String>, hint: &str| match v {
        Some(v) => format!("{key:<12} = {}\n", quoted(v)),
        None => format!("# {key:<10} = {hint}\n"),
    };
    let relationship = match new.relationships.as_slice() {
        [] => "# relationship = \"colleague\"   # a template in ../relationships/, or several\n"
            .to_string(),
        [one] => format!("relationship = {}\n", quoted(one)),
        many => format!("relationship = {}\n", quoted_list(many)),
    };
    format!(
        "# {name}: the settings the code reads. Yours — mecha never rewrites this\n\
         # file. Who they are lives in identity.md, what they want in motivation.md.\n\
         # docs/PERSONA-DESIGN.md §4.3 explains each field.\n\
         \n\
         display      = {display}\n\
         {relationship}\
         {character}\
         {voice}\
         groups       = {groups}   # declared in ../groups.toml\n\
         # model      = \"local\"      # pin a model; unset is the default\n\
         \n\
         [tools]\n\
         # Asked for, not granted: a chat gets only tools that declare themselves\n\
         # safe for a persona, and nothing that reads your mail, calendar or graph.\n\
         allow = {tools}\n\
         \n\
         [safety]   # all on by default; turn one off for this persona only\n\
         disclosure = true\n\
         crisis     = true\n\
         dose       = true\n\
         breaks     = false\n\
         farewell   = true\n\
         reanchor   = true\n\
         \n\
         [files]\n\
         answers = {answers}   # \"files\": only its files, web tools withheld · \"open\"\n\
         \n\
         [memory]\n\
         episodic    = true\n\
         semantic    = true\n\
         user_facts  = \"shared\"   # own | shared | off\n\
         about_me    = true\n\
         self_update = false      # let identity.md's sections evolve on their own\n\
         fixed       = []         # sections that never evolve; Core never does\n",
        name = new.name,
        display = quoted(&new.display),
        character = opt(
            "character",
            &new.character,
            "\"mara\"   # an image-library character: the portrait"
        ),
        voice = opt(
            "voice",
            &new.voice,
            "\"mara-low\"   # a profile in ../voices/"
        ),
        groups = quoted_list(&new.groups),
        tools = quoted_list(tools),
        answers = quoted(match answers {
            Answers::Files => "files",
            Answers::Open => "open",
        }),
    )
}

fn identity_template(display: &str) -> String {
    format!(
        "# {display}\n\
         \n\
         <!-- Who {display} is, in your words. Everything outside comments like\n     \
         this one is read into every chat with them; comments are notes to\n     \
         yourself and never reach a chat.\n\
         \n     \
         `## Core` is who they are at heart: it never changes on its own, and\n     \
         it is re-read to them in long chats. Add, rename or remove the other\n     \
         sections freely. -->\n\
         \n\
         ## Core\n\
         \n\
         ## How they talk\n\
         \n\
         ## Background\n"
    )
}

fn motivation_template(display: &str) -> String {
    format!(
        "# What {display} wants\n\
         \n\
         <!-- Their wants and values, as part of who they are — not your\n     \
         priorities, and never a charter. A goal for one conversation is set\n     \
         when the chat opens, not here. -->\n"
    )
}

/// Create a persona. The owner's are approved on creation. The folder is
/// claimed with a plain `create_dir`, so two creations of one name cannot
/// both succeed, and every link it names must resolve — a persona is never
/// *created* broken, though a later hand edit can break it (and is told so).
pub fn create(dir: &Path, lib: &imagelib::Library, new: NewPersona) -> Result<Persona> {
    validate_persona_name(&new.name)?;
    let mut new = new;
    if new.display.trim().is_empty() {
        new.display = new.name.clone();
    }
    if new.display.chars().count() > MAX_DISPLAY || new.display.chars().any(char::is_control) {
        bail!("a display name is one line of at most {MAX_DISPLAY} characters");
    }
    ensure_layout(dir)?;
    let store = Store::load(dir);
    let mut tools: Vec<String> = Vec::new();
    let mut answers = Answers::Open;
    for r in &new.relationships {
        let t = store.relationships.get(r).ok_or_else(|| {
            anyhow!(
                "no relationship template `{r}` in {}",
                dir.join("relationships").display()
            )
        })?;
        for tool in &t.suggest.tools {
            if !tools.contains(tool) {
                tools.push(tool.clone());
            }
        }
        // Several templates: the narrower suggestion wins.
        if t.suggest.answers == Some(Answers::Files) {
            answers = Answers::Files;
        }
    }
    let probe = Persona {
        name: new.name.clone(),
        settings: Settings {
            display: new.display.clone(),
            relationship: Names(new.relationships.clone()),
            character: new.character.clone(),
            voice: new.voice.clone(),
            groups: new.groups.clone(),
            model: None,
            tools: Tools::default(),
            safety: Safety::default(),
            files: FilesSettings::default(),
            memory: MemorySettings::default(),
        },
        state: State {
            status: Status::Approved,
            ..State::default()
        },
        identity: "## Core\nx\n".into(),
        motivation: String::new(),
        notes: Vec::new(),
    };
    if let Some(broken) = store.problems(&probe, lib).into_iter().next() {
        bail!("`{}` {broken}", new.name);
    }

    let pdir = dir.join(&new.name);
    std::fs::create_dir(&pdir).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            anyhow!("a persona named `{}` already exists", new.name)
        } else {
            anyhow!("creating {}: {e}", pdir.display())
        }
    })?;
    for sub in ["files", "sessions", "candidates", "versions"] {
        std::fs::create_dir(pdir.join(sub))?;
    }
    write_atomic(
        &pdir.join("persona.toml"),
        render_settings(&new, &tools, answers).as_bytes(),
    )?;
    write_atomic(
        &pdir.join("identity.md"),
        identity_template(&new.display).as_bytes(),
    )?;
    write_atomic(
        &pdir.join("motivation.md"),
        motivation_template(&new.display).as_bytes(),
    )?;
    let at = now();
    let origin = new.origin;
    write_state(
        &pdir,
        &State {
            status: if origin == Origin::Owner {
                Status::Approved
            } else {
                Status::Candidate
            },
            origin,
            locked: new.locked,
            version: 0,
            digest: String::new(),
            created: at.clone(),
            updated: at,
        },
    )?;
    snapshot(dir, &new.name)?;
    Store::load(dir)
        .get(&new.name)
        .cloned()
        .ok_or_else(|| anyhow!("`{}` was written but does not load", new.name))
}

fn write_state(pdir: &Path, state: &State) -> Result<()> {
    let text = format!(
        "# Written by mecha: approval, the browse lock and the version.\n\
         # Your settings are in persona.toml.\n\n{}",
        toml::to_string_pretty(state)?
    );
    write_atomic(&pdir.join("state.toml"), text.as_bytes())
}

fn current(dir: &Path, name: &str) -> Result<(Store, Persona)> {
    let store = Store::load(dir);
    if let Some(p) = store.get(name).cloned() {
        return Ok((store, p));
    }
    if let Some(e) = store
        .errors
        .iter()
        .find(|e| e.path.parent().is_some_and(|p| p.ends_with(name)))
    {
        bail!("persona `{name}` does not load: {}", e.why);
    }
    bail!("no persona named `{name}`")
}

/// One line of `versions/log.jsonl`: which digest each version number was.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionRecord {
    pub version: u32,
    pub digest: String,
    pub at: String,
}

/// Take a version of `name` if its files changed since the last one: the
/// files copied under `versions/<digest>/`, the number bumped, and a line
/// appended to `versions/log.jsonl`, so a chat pinned to a version can still
/// render the persona it started with (§8.1). Idempotent: unchanged files
/// return the state as it was.
pub fn snapshot(dir: &Path, name: &str) -> Result<State> {
    let (store, p) = current(dir, name)?;
    let files = store.version_files(&p)?;
    let digest = digest_of(&files);
    let mut state = p.state.clone();
    if state.digest == digest {
        return Ok(state);
    }
    let pdir = store.persona_dir(name);
    let versions = pdir.join("versions");
    std::fs::create_dir_all(&versions)?;
    let target = versions.join(&digest);
    if !target.is_dir() {
        // Built aside and renamed in, so a half-written version never has
        // the digest's name.
        let tmp = versions.join(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
        let built = (|| -> Result<()> {
            for (file, text) in &files {
                let path = tmp.join(file);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, text)?;
            }
            std::fs::rename(&tmp, &target)?;
            Ok(())
        })();
        if let Err(e) = built {
            let _ = std::fs::remove_dir_all(&tmp);
            if !target.is_dir() {
                return Err(e).context("writing the version");
            }
        }
    }
    // The first version is 1 however the state arrived: made here, by hand,
    // or read back from a damaged file.
    state.version = if state.digest.is_empty() {
        1
    } else {
        state.version + 1
    };
    state.digest = digest.clone();
    state.updated = now();
    let record = VersionRecord {
        version: state.version,
        digest,
        at: state.updated.clone(),
    };
    {
        use std::io::Write;
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(versions.join("log.jsonl"))?;
        writeln!(log, "{}", serde_json::to_string(&record)?)?;
    }
    write_state(&pdir, &state)?;
    Ok(state)
}

/// The owner's approval of a persona made outside mecha or proposed by a
/// model. The CLI shows it first.
pub fn approve(dir: &Path, name: &str) -> Result<State> {
    let (store, p) = current(dir, name)?;
    if p.state.status == Status::Approved {
        bail!("`{name}` is already approved");
    }
    // Everything that can refuse runs before the approval is written, so a
    // refusal leaves the persona exactly as unapproved as it was.
    store
        .version_files(&p)
        .with_context(|| format!("`{name}` cannot be approved yet"))?;
    let mut state = p.state;
    state.status = Status::Approved;
    if state.created.is_empty() {
        state.created = now();
    }
    state.updated = now();
    write_state(&dir.join(name), &state)?;
    snapshot(dir, name)
}

/// The browse filter, per persona.
pub fn set_locked(dir: &Path, name: &str, locked: bool) -> Result<State> {
    let (_, p) = current(dir, name)?;
    let mut state = p.state;
    state.locked = locked;
    state.updated = now();
    write_state(&dir.join(name), &state)?;
    Ok(state)
}

/// Remove a persona — its whole folder, chats and memory included, moved
/// aside under `removed/`, not deleted.
pub fn remove(dir: &Path, name: &str) -> Result<PathBuf> {
    validate_persona_name(name)?;
    let pdir = dir.join(name);
    if !pdir.join("persona.toml").is_file() {
        bail!("no persona named `{name}`");
    }
    let removed = dir.join("removed");
    std::fs::create_dir_all(&removed)?;
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    // The stamp alone is per second: remove, recreate and remove again in
    // one second must not collide with the first.
    let unique = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let to = removed.join(format!("{name}-{stamp}-{unique}"));
    std::fs::rename(&pdir, &to)?;
    Ok(to)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mecha-persona-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn no_lib() -> imagelib::Library {
        imagelib::Library::load(&std::env::temp_dir().join("mecha-persona-no-such-lib")).0
    }

    fn new(name: &str) -> NewPersona {
        NewPersona {
            name: name.into(),
            origin: Origin::Owner,
            ..NewPersona::default()
        }
    }

    fn fill_core(dir: &Path, name: &str) {
        let path = dir.join(name).join("identity.md");
        let text = std::fs::read_to_string(&path).unwrap().replace(
            "## Core\n",
            "## Core\nA marine ecologist who distrusts easy answers.\n",
        );
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn create_lays_out_the_store_and_takes_version_one() {
        let dir = scratch();
        let p = create(&dir, &no_lib(), new("mara")).unwrap();
        for sub in ["files", "sessions", "candidates", "versions"] {
            assert!(dir.join("mara").join(sub).is_dir(), "{sub}");
        }
        for sub in ["files", "groups", "relationships", "voices", "scenarios"] {
            assert!(dir.join(sub).is_dir(), "{sub}");
        }
        assert!(dir.join("about-me.md").is_file());
        assert_eq!(p.state.status, Status::Approved);
        assert_eq!(p.state.origin, Origin::Owner);
        assert_eq!(p.state.version, 1);
        assert!(dir.join("mara/versions").join(&p.state.digest).is_dir());
        assert_eq!(p.display(), "mara");
        // A fresh identity has an empty Core, and says so.
        let store = Store::load(&dir);
        let problems = store.problems(store.get("mara").unwrap(), &no_lib());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("Core"), "{problems:?}");
        fill_core(&dir, "mara");
        let store = Store::load(&dir);
        assert!(store
            .problems(store.get("mara").unwrap(), &no_lib())
            .is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_rendered_settings_read_back_as_the_defaults() {
        let dir = scratch();
        let p = create(&dir, &no_lib(), new("priya")).unwrap();
        assert_eq!(p.settings.safety, Safety::default());
        assert_eq!(p.settings.memory, MemorySettings::default());
        assert_eq!(p.settings.files.answers, Answers::Open);
        assert!(p.notes.is_empty(), "{:?}", p.notes);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_persona_can_never_be_named_for_a_store_folder() {
        for name in RESERVED {
            assert!(validate_persona_name(name).is_err(), "{name}");
        }
        let dir = scratch();
        assert!(create(&dir, &no_lib(), new("files")).is_err());
        // A persona planted in one by hand is a load error, not a persona.
        std::fs::create_dir_all(dir.join("voices")).unwrap();
        std::fs::write(dir.join("voices/persona.toml"), "").unwrap();
        std::fs::write(dir.join("voices/identity.md"), "## Core\nx\n").unwrap();
        let store = Store::load(&dir);
        assert!(store.get("voices").is_none());
        assert!(store.errors().iter().any(|e| e.why.contains("reserved")));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn names_are_checked() {
        for bad in [
            "",
            "Mara",
            "-x",
            "_x",
            "a/b",
            "a.b",
            &"x".repeat(MAX_NAME + 1),
        ] {
            assert!(validate_name(bad).is_err(), "{bad:?}");
        }
        for good in ["mara", "devils_advocate", "mara-2"] {
            validate_name(good).unwrap();
        }
    }

    #[test]
    fn two_creations_of_one_name_cannot_both_succeed() {
        let dir = scratch();
        create(&dir, &no_lib(), new("ada")).unwrap();
        let again = create(&dir, &no_lib(), new("ada")).unwrap_err();
        assert!(format!("{again}").contains("already exists"), "{again}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn an_unknown_key_fails_the_load_by_name() {
        let dir = scratch();
        create(&dir, &no_lib(), new("mara")).unwrap();
        let path = dir.join("mara/persona.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace("crisis     = true", "crsis = false")).unwrap();
        let store = Store::load(&dir);
        assert!(store.get("mara").is_none());
        assert!(
            store.errors().iter().any(|e| e.why.contains("crsis")),
            "{:?}",
            store.errors()
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn an_unknown_value_narrows_and_says_so() {
        let dir = scratch();
        create(&dir, &no_lib(), new("mara")).unwrap();
        let path = dir.join("mara/persona.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            text.replace("answers = \"open\"", "answers = \"opne\"")
                .replace("user_facts  = \"shared\"", "user_facts = \"everyone\""),
        )
        .unwrap();
        let store = Store::load(&dir);
        let p = store.get("mara").unwrap();
        assert_eq!(p.settings.files.answers, Answers::Files);
        assert_eq!(p.settings.memory.user_facts, UserFacts::Off);
        assert_eq!(p.notes.len(), 2, "{:?}", p.notes);
        assert!(p.notes[0].contains("opne"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_hand_made_persona_is_unapproved_until_approved() {
        let dir = scratch();
        ensure_layout(&dir).unwrap();
        std::fs::create_dir_all(dir.join("ada")).unwrap();
        std::fs::write(dir.join("ada/persona.toml"), "display = \"Ada\"\n").unwrap();
        std::fs::write(dir.join("ada/identity.md"), "## Core\nA patient.\n").unwrap();
        let store = Store::load(&dir);
        let p = store.get("ada").unwrap();
        assert_eq!(p.state.status, Status::Candidate);
        assert_eq!(p.state.origin, Origin::ModelUntrusted);
        assert!(p.notes.iter().any(|n| n.contains("state.toml")));
        assert!(store.problems(p, &no_lib())[0].contains("not approved"));
        let state = approve(&dir, "ada").unwrap();
        assert_eq!((state.status, state.version), (Status::Approved, 1));
        // Provenance is not laundered by approval.
        assert_eq!(state.origin, Origin::ModelUntrusted);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn links_are_resolved_and_broken_ones_named() {
        let dir = scratch();
        create(&dir, &no_lib(), new("mara")).unwrap();
        fill_core(&dir, "mara");
        let path = dir.join("mara/persona.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        let text = text
            .replace("display      = \"mara\"", "display = \"Mara\"\nrelationship = [\"colleague\", \"rival\"]\ncharacter = \"mara\"\nvoice = \"mara-low\"")
            .replace("groups       = []", "groups = [\"work\"]")
            .replace("fixed       = []", "fixed = [\"Background\", \"Hobbies\"]");
        std::fs::write(&path, text).unwrap();
        let store = Store::load(&dir);
        let p = store.get("mara").unwrap();
        let problems = store.problems(p, &no_lib()).join("\n");
        for needle in [
            "`rival`",
            "character `mara`",
            "voice `mara-low`",
            "group `work`",
            "`Hobbies`",
        ] {
            assert!(problems.contains(needle), "{needle} in:\n{problems}");
        }
        assert!(!problems.contains("`colleague`"), "{problems}");
        assert!(!problems.contains("`Background`"), "{problems}");
        // Declared and profiled, the links resolve.
        add_group(&dir, "work", "the kelp project").unwrap();
        std::fs::create_dir_all(dir.join("voices/mara-low")).unwrap();
        std::fs::write(
            dir.join("voices/mara-low/profile.toml"),
            "voice = \"en-f-2\"\n",
        )
        .unwrap();
        let store = Store::load(&dir);
        let problems = store
            .problems(store.get("mara").unwrap(), &no_lib())
            .join("\n");
        assert!(!problems.contains("group"), "{problems}");
        assert!(!problems.contains("voice"), "{problems}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn create_refuses_a_broken_link() {
        let dir = scratch();
        let mut n = new("mara");
        n.relationships = vec!["rival".into()];
        assert!(format!("{}", create(&dir, &no_lib(), n).unwrap_err()).contains("rival"));
        let mut n = new("mara");
        n.groups = vec!["work".into()];
        assert!(format!("{}", create(&dir, &no_lib(), n).unwrap_err()).contains("work"));
        assert!(!dir.join("mara").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_template_suggests_once_and_the_narrower_answer_wins() {
        let dir = scratch();
        let mut n = new("priya");
        n.relationships = vec!["colleague".into(), "teacher".into()];
        let p = create(&dir, &no_lib(), n).unwrap();
        assert_eq!(p.settings.tools.allow, vec!["web_search".to_string()]);
        assert_eq!(p.settings.files.answers, Answers::Files);
        assert_eq!(
            p.settings.relationship.0,
            vec!["colleague".to_string(), "teacher".to_string()]
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn starters_are_offered_once_and_never_overwritten() {
        let dir = scratch();
        let first = seed_starters(&dir).unwrap();
        assert_eq!(first.len(), STARTERS.len());
        let rel = dir.join("relationships");
        std::fs::write(rel.join("friend.md"), "# Mine now\n").unwrap();
        std::fs::remove_file(rel.join("coach.md")).unwrap();
        assert!(seed_starters(&dir).unwrap().is_empty());
        assert_eq!(
            std::fs::read_to_string(rel.join("friend.md")).unwrap(),
            "# Mine now\n"
        );
        assert!(
            !rel.join("coach.md").exists(),
            "a deleted starter came back"
        );
        let seeded = std::fs::read_to_string(rel.join(SEEDED)).unwrap();
        assert_eq!(seeded.matches('#').count(), 1, "{seeded}");
        // An upgrade shipping a starter not yet offered adds just that one.
        let seeded = std::fs::read_to_string(rel.join(SEEDED)).unwrap();
        std::fs::write(rel.join(SEEDED), seeded.replace("romantic\n", "")).unwrap();
        std::fs::remove_file(rel.join("romantic.md")).unwrap();
        assert_eq!(seed_starters(&dir).unwrap(), vec!["romantic".to_string()]);
        let store = Store::load(&dir);
        assert!(store.relationships()["romantic"].starter);
        assert!(!store.relationships()["friend"].starter);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn every_starter_loads_and_keeps_the_shipped_wording_rules() {
        // §15: no shipped starter uses a licensed-profession or treatment
        // word; "clinical" appears only in the not-a-substitute notice.
        let avoid = [
            "therapist",
            "therapy",
            "counselor",
            "counsellor",
            "psycholog",
            "psychiatr",
            "doctor",
            "dr.",
            "licensed",
            "treat",
            "diagnos",
            "prevent",
            "mental health",
            "confidential",
        ];
        for (name, text) in STARTERS {
            validate_name(name).unwrap();
            let (_, body) = front_matter(text).unwrap_or_else(|e| panic!("{name}: {e:#}"));
            assert!(body.contains("# "), "{name}");
            let lower = text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            for word in avoid {
                assert!(!lower.contains(word), "{name} uses `{word}`");
            }
            let clinical = lower.matches("clinical").count();
            let notice = lower.matches("substitute for clinical care").count();
            assert_eq!(
                clinical, notice,
                "{name} uses `clinical` outside the notice"
            );
        }
        for name in ["reflective", "coach"] {
            let text = STARTERS.iter().find(|(n, _)| *n == name).unwrap().1;
            let (prompt, _) = strip_comments(text);
            let prompt = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(
                prompt.contains("not a substitute for clinical care"),
                "{name} must carry the notice outside a comment"
            );
        }
    }

    #[test]
    fn a_version_is_taken_only_when_something_changed() {
        let dir = scratch();
        let mut n = new("mara");
        n.relationships = vec!["colleague".into()];
        let v1 = create(&dir, &no_lib(), n).unwrap().state;
        assert_eq!(snapshot(&dir, "mara").unwrap(), v1);
        fill_core(&dir, "mara");
        let v2 = snapshot(&dir, "mara").unwrap();
        assert_eq!(v2.version, 2);
        assert_ne!(v2.digest, v1.digest);
        // The template is part of what a chat renders, so it is part of the
        // version, and the snapshot carries it.
        std::fs::write(
            dir.join("relationships/colleague.md"),
            "# Colleague\nBrisk.\n",
        )
        .unwrap();
        let v3 = snapshot(&dir, "mara").unwrap();
        assert_eq!(v3.version, 3);
        let snap = dir.join("mara/versions").join(&v3.digest);
        assert_eq!(
            std::fs::read_to_string(snap.join("relationships/colleague.md")).unwrap(),
            "# Colleague\nBrisk.\n"
        );
        assert!(snap.join("identity.md").is_file());
        let log = std::fs::read_to_string(dir.join("mara/versions/log.jsonl")).unwrap();
        let records: Vec<VersionRecord> = log
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(
            records.iter().map(|r| r.version).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(records[2].digest, v3.digest);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn locking_hides_while_browsing_and_changes_nothing_else() {
        let dir = scratch();
        create(&dir, &no_lib(), new("mara")).unwrap();
        create(&dir, &no_lib(), new("priya")).unwrap();
        let before = Store::load(&dir).get("mara").unwrap().state.clone();
        let locked = set_locked(&dir, "mara", true).unwrap();
        assert!(locked.locked);
        assert_eq!(
            (locked.version, &locked.digest, locked.status),
            (before.version, &before.digest, before.status)
        );
        let store = Store::load(&dir);
        let names = |unlocked| {
            store
                .visible(unlocked)
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(false), vec!["priya".to_string()]);
        assert_eq!(names(true), vec!["mara".to_string(), "priya".to_string()]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_persona_reads_only_its_files_roots() {
        let dir = scratch();
        add_group(&dir, "work", "").unwrap();
        let mut n = new("mara");
        n.groups = vec!["work".into()];
        create(&dir, &no_lib(), n).unwrap();
        // `play` joined by hand edit but never declared: no access from it.
        let path = dir.join("mara/persona.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace("[\"work\"]", "[\"work\", \"play\"]")).unwrap();
        let store = Store::load(&dir);
        let roots = store.files_roots(store.get("mara").unwrap());
        assert_eq!(
            roots,
            vec![
                dir.join("mara/files"),
                dir.join("groups/work/files"),
                dir.join("files"),
            ]
        );
        for root in &roots {
            assert!(root.is_dir(), "{}", root.display());
            assert!(!dir.join("mara/sessions").starts_with(root));
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn prose_that_could_repaint_a_terminal_does_not_load() {
        let dir = scratch();
        create(&dir, &no_lib(), new("mara")).unwrap();
        create(&dir, &no_lib(), new("priya")).unwrap();
        std::fs::write(dir.join("mara/identity.md"), "## Core\nDry.\x1b[2J\tok\n").unwrap();
        let toml = dir.join("priya/persona.toml");
        let text = std::fs::read_to_string(&toml).unwrap();
        std::fs::write(&toml, text.replace("\"priya\"", "\"pri\\u001bya\"")).unwrap();
        let store = Store::load(&dir);
        assert!(store.get("mara").is_none());
        assert!(store.get("priya").is_none());
        let why: Vec<&str> = store.errors().iter().map(|e| e.why.as_str()).collect();
        assert!(why.iter().any(|w| w.contains("U+001B")), "{why:?}");
        assert!(why.iter().any(|w| w.contains("a value holds")), "{why:?}");
        // Every string field, not just `display`, and a bare carriage return.
        for (field, value) in [
            ("# voice      = \"mara-low\"", "voice = \"x\\u001b[2J\""),
            ("allow = []", "allow = [\"web\\u0007\"]"),
            ("# model      = \"local\"", "model = \"a\\rb\""),
        ] {
            create(&dir, &no_lib(), new("ada")).unwrap();
            let toml = dir.join("ada/persona.toml");
            let text = std::fs::read_to_string(&toml).unwrap();
            assert!(text.contains(field), "{field}");
            std::fs::write(&toml, text.replace(field, value)).unwrap();
            let store = Store::load(&dir);
            assert!(store.get("ada").is_none(), "{value} loaded");
            remove(&dir, "ada").unwrap();
        }
        std::fs::write(dir.join("mara/identity.md"), "## Core\nDry.\rWet.\n").unwrap();
        assert!(Store::load(&dir).get("mara").is_none(), "a bare CR loaded");
        std::fs::write(
            dir.join("groups.toml"),
            "[work]\ndescription = \"\\u001b[31m\"\n",
        )
        .unwrap();
        assert!(Store::load(&dir).groups().is_empty());
        // Tabs are Markdown, not an attack.
        std::fs::write(dir.join("mara/identity.md"), "## Core\n\tDry.\r\n").unwrap();
        assert!(Store::load(&dir).get("mara").is_some());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_door_that_forgets_the_origin_makes_a_candidate() {
        let dir = scratch();
        let p = create(
            &dir,
            &no_lib(),
            NewPersona {
                name: "ada".into(),
                ..NewPersona::default()
            },
        )
        .unwrap();
        assert_eq!(p.state.status, Status::Candidate);
        assert_eq!(p.state.origin, Origin::ModelUntrusted);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn an_approval_that_cannot_version_writes_nothing() {
        let dir = scratch();
        ensure_layout(&dir).unwrap();
        std::fs::create_dir_all(dir.join("ada")).unwrap();
        std::fs::write(dir.join("ada/persona.toml"), "relationship = \"rival\"\n").unwrap();
        std::fs::write(dir.join("ada/identity.md"), "## Core\nA patient.\n").unwrap();
        let refused = approve(&dir, "ada").unwrap_err();
        assert!(format!("{refused:#}").contains("rival"), "{refused:#}");
        assert!(!dir.join("ada/state.toml").exists());
        assert_eq!(
            Store::load(&dir).get("ada").unwrap().state.status,
            Status::Candidate
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// §3.2's boundary is an absence — no reader of the assistant's
    /// sessions looks under `personas/` — which is the shape that stops being
    /// true silently. So: a real transcript in a persona's `sessions/`, the
    /// real readers run over the real layout, and a control proving the same
    /// readers *would* take that transcript if they were pointed at it.
    #[test]
    fn no_session_reader_sees_a_persona_transcript() {
        use crate::runlog::{Corpus, Scan};
        use crate::session::{split_admitted, Session, SessionKind, SessionMeta};
        let home = scratch();
        let meta = |id: &str, kind| SessionMeta {
            id: id.into(),
            created_at: chrono::Utc::now(),
            provider: "local".into(),
            model: "m".into(),
            workspace: home.clone(),
            title: None,
            kind,
        };
        let sessions = home.join("sessions");
        Session::create(
            &sessions,
            meta("20260929T000000-assist01", Some(SessionKind::Web)),
        )
        .unwrap();
        let dir = home.join("personas");
        create(&dir, &no_lib(), new("mara")).unwrap();
        let persona_sessions = Store::load(&dir).sessions_dir("mara");
        // The worst case: a kind every default reader admits.
        Session::create(&persona_sessions, meta("20260929T000001-persona1", None)).unwrap();

        let ids = |d: &Path| -> Vec<String> {
            Session::list(d)
                .unwrap()
                .into_iter()
                .map(|(m, _)| m.id)
                .collect()
        };
        let admitted = |d: &Path| -> Vec<String> {
            split_admitted(Session::list(d).unwrap())
                .0
                .into_iter()
                .map(|(m, _)| m.id)
                .collect()
        };
        let scan = Scan {
            include_tests: true,
            ..Scan::default()
        };
        let read = |d: &Path| Corpus::scan(d, &scan).unwrap().sessions_read;

        // What the assistant's readers see: its own session, never Mara's.
        assert_eq!(ids(&sessions), vec!["20260929T000000-assist01".to_string()]);
        assert_eq!(
            Session::list_headers_counting(&sessions).unwrap().0.len(),
            1
        );
        assert!(!admitted(&sessions).iter().any(|id| id.contains("persona")));
        assert_eq!(read(&sessions), read(&home.join("no-such-dir")) + 1);

        // Control: the transcript is readable and would be admitted.
        assert_eq!(
            ids(&persona_sessions),
            vec!["20260929T000001-persona1".to_string()]
        );
        if std::env::var_os("MECHA_SESSION_KIND").is_none() {
            assert_eq!(admitted(&persona_sessions).len(), 1);
        }
        assert_eq!(read(&persona_sessions), 1);

        // And the default layout keeps the two trees apart.
        if std::env::var_os("MECHA_SESSION_DIR").is_none() {
            let (s, p) = (
                Session::default_dir().unwrap(),
                Store::default_dir().unwrap(),
            );
            assert!(!p.starts_with(&s) && !s.starts_with(&p), "{s:?} / {p:?}");
        }
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn removal_moves_the_whole_folder_aside() {
        let dir = scratch();
        create(&dir, &no_lib(), new("mara")).unwrap();
        std::fs::write(dir.join("mara/sessions/one.jsonl"), "{}\n").unwrap();
        let to = remove(&dir, "mara").unwrap();
        assert!(to.join("sessions/one.jsonl").is_file());
        assert!(Store::load(&dir).get("mara").is_none());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn adding_a_group_keeps_the_owners_comments() {
        let dir = scratch();
        add_group(&dir, "work", "the \"kelp\" project").unwrap();
        let text = std::fs::read_to_string(dir.join("groups.toml")).unwrap();
        assert!(text.starts_with("# Groups of personas"), "{text}");
        let store = Store::load(&dir);
        assert_eq!(store.groups()["work"].description, "the \"kelp\" project");
        assert!(dir.join("groups/work/files").is_dir());
        assert!(dir.join("groups/work/about-me.md").is_file());
        assert!(add_group(&dir, "work", "").is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn comments_never_reach_a_section() {
        let text = "# Mara\n<!-- ## Hidden\nnotes -->\n## Core\nDry.\n<!-- evolved · was: \"x\" -->\nPrecise.\n\
                    ```\n## not a heading\n```\n## How she talks\nSlowly.\n";
        let secs = sections(text);
        assert_eq!(
            secs.iter().map(|s| s.heading.as_str()).collect::<Vec<_>>(),
            vec!["Core", "How she talks"]
        );
        assert_eq!(secs[0].body, "Dry.\n\nPrecise.\n```\n## not a heading\n```");
        let (open, unclosed) = strip_comments("keep <!-- never closed\n## Core\nleak");
        assert_eq!((open.as_str(), unclosed), ("keep ", true));
    }

    #[test]
    fn front_matter_is_optional_and_closed() {
        assert_eq!(front_matter("# Plain\n").unwrap().0, Suggest::default());
        let (s, body) = front_matter("+++\nanswers = \"files\"\n+++\n# T\n").unwrap();
        assert_eq!(s.answers, Some(Answers::Files));
        assert_eq!(body, "# T\n");
        assert!(front_matter("+++\nanswers = \"files\"\n# never closed\n").is_err());
        assert!(front_matter("+++\ntool = []\n+++\n").is_err());
    }

    #[test]
    fn a_voice_profile_needs_a_voice_or_a_clip_beside_it() {
        let dir = scratch();
        let v = dir.join("voices");
        for (name, body) in [
            ("none", ""),
            ("escape", "reference = \"../x.wav\"\n"),
            ("missing", "reference = \"clip.wav\"\n"),
            ("ok", "reference = \"clip.wav\"\nspeed = 1.1\n"),
        ] {
            std::fs::create_dir_all(v.join(name)).unwrap();
            std::fs::write(v.join(name).join("profile.toml"), body).unwrap();
        }
        std::fs::write(v.join("ok/clip.wav"), b"RIFF").unwrap();
        let store = Store::load(&dir);
        assert_eq!(store.voices().keys().collect::<Vec<_>>(), vec!["ok"]);
        assert_eq!(store.errors().len(), 3, "{:?}", store.errors());
        std::fs::remove_dir_all(dir).ok();
    }
}
