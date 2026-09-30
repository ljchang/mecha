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

use crate::imagelib::{self, write_atomic_mode};
pub use crate::imagelib::{Origin, Status};

pub mod agent;
pub mod safety;

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
/// Line endings are read as LF: `read_prose` admits CRLF, and a template
/// saved on Windows must not silently lose its suggestions and render its
/// front matter into a prompt (found on review of #405).
fn front_matter(text: &str) -> Result<(Suggest, String)> {
    let text = text.replace("\r\n", "\n");
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
    Ok((suggest, body.to_string()))
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
        // The same door as every other file here: size cap, control
        // characters, then the parse.
        match read_prose(&state_path).and_then(|s| parse_toml::<State>(&s)) {
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

/// A folder of the store, owner-only like every `~/.mecha` leaf: what is in
/// here is about the owner. Re-applied to a folder that already exists.
fn private_dir(dir: &Path) -> Result<()> {
    crate::create_private_dir(dir).with_context(|| format!("creating {}", dir.display()))
}

/// Claim a new owner-only folder: fails if it exists, so two claims of one
/// name cannot both succeed.
fn claim_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Write-then-rename, owner-only.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    write_atomic_mode(path, bytes, Some(0o600))
}

/// Create the store's shared folders and files where missing, and offer the
/// starters. Never overwrites anything.
pub fn ensure_layout(dir: &Path) -> Result<()> {
    private_dir(dir)?;
    for sub in ["files", "groups", "relationships", "voices", "scenarios"] {
        private_dir(&dir.join(sub))?;
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
    private_dir(&g)?;
    private_dir(&g.join("files"))?;
    write_new(&g.join("about-me.md"), ABOUT_ME)?;
    Ok(())
}

/// Write `text` to `path` only if nothing is there. `Ok(false)` when
/// something was.
fn write_new(path: &Path, text: &str) -> Result<bool> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
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
    private_dir(dir)?;
    private_dir(&rel)?;
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
        write_private(&seeded_path, text.as_bytes())?;
    }
    Ok(copied)
}

/// Declare a group: appended to `groups.toml` (the owner's comments kept),
/// with its folder and about-me made.
pub fn add_group(dir: &Path, name: &str, description: &str) -> Result<()> {
    validate_name(name)?;
    // Held to the loader's rule before it is written: one bad value makes
    // the whole of groups.toml refuse to load, and every group link with it.
    if forbidden_control(description).is_some() || description.contains('\n') {
        bail!("a group's description is one line, without control characters");
    }
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
    write_private(&path, text.as_bytes())?;
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
        "# {name}: the settings the code reads. Yours — only you change it, here\n\
         # or in the page's form, which sets values in place and keeps comments.\n\
         # Who they are lives in identity.md, what they want in motivation.md.\n\
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
/// claimed with an exclusive create, so two creations of one name cannot
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
    claim_dir(&pdir).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            anyhow!("a persona named `{}` already exists", new.name)
        } else {
            anyhow!("creating {}: {e}", pdir.display())
        }
    })?;
    for sub in ["files", "sessions", "candidates", "versions"] {
        claim_dir(&pdir.join(sub))?;
    }
    write_private(
        &pdir.join("persona.toml"),
        render_settings(&new, &tools, answers).as_bytes(),
    )?;
    write_private(
        &pdir.join("identity.md"),
        identity_template(&new.display).as_bytes(),
    )?;
    write_private(
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
    write_private(&pdir.join("state.toml"), text.as_bytes())
}

fn current(dir: &Path, name: &str) -> Result<(Store, Persona)> {
    let store = Store::load(dir);
    if let Some(p) = store.get(name).cloned() {
        return Ok((store, p));
    }
    if let Some(e) = store
        .errors
        .iter()
        .find(|e| e.path == store.persona_dir(name).join("persona.toml"))
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
    private_dir(&versions)?;
    let target = versions.join(&digest);
    if !target.is_dir() {
        // Built aside and renamed in, so a half-written version never has
        // the digest's name.
        let tmp = versions.join(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
        let built = (|| -> Result<()> {
            for (file, text) in &files {
                let path = tmp.join(file);
                if let Some(parent) = path.parent() {
                    private_dir(parent)?;
                }
                write_new(&path, text)?;
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
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut log = options.open(versions.join("log.jsonl"))?;
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

/// Add a relationship template — the owner's own, beside the starters (§5).
/// Held to what the loader will accept (a name, the size cap, no control
/// characters, front matter that parses), and made with an exclusive create,
/// so an existing template — a starter the owner edited included — is never
/// overwritten from here.
pub fn add_relationship(dir: &Path, name: &str, text: &str) -> Result<()> {
    validate_name(name)?;
    if text.trim().is_empty() {
        bail!("a relationship needs text: how someone in it behaves");
    }
    if text.len() as u64 > MAX_PROSE_BYTES {
        bail!(
            "a relationship template is capped at {} KB",
            MAX_PROSE_BYTES / 1024
        );
    }
    if let Some(c) = forbidden_control(text) {
        bail!(
            "the template would hold a control character (U+{:04X}); only newlines and tabs are allowed",
            c as u32
        );
    }
    front_matter(text).context("reading the template")?;
    ensure_layout(dir)?;
    if !write_new(&dir.join("relationships").join(format!("{name}.md")), text)? {
        bail!("a relationship named `{name}` already exists");
    }
    Ok(())
}

/// One of the files of a persona the owner writes — what a surface other
/// than `$EDITOR` (the web page) may edit, and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerFile {
    Settings,
    Identity,
    Motivation,
}

impl OwnerFile {
    pub fn file_name(self) -> &'static str {
        match self {
            OwnerFile::Settings => "persona.toml",
            OwnerFile::Identity => "identity.md",
            OwnerFile::Motivation => "motivation.md",
        }
    }
}

/// A save made against a file that changed since it was opened — an edit
/// made elsewhere. Typed, so a surface can tell it from bad input (a 409, not
/// a 400).
#[derive(Debug)]
pub struct StaleEdit(pub OwnerFile);

impl std::fmt::Display for StaleEdit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} changed since it was opened (an edit made elsewhere); open it again",
            self.0.file_name()
        )
    }
}

impl std::error::Error for StaleEdit {}

/// The relationship templates a new persona can name, read without writing:
/// the owner's templates on disk, and each shipped starter not yet offered,
/// which `create` copies in on first use. A starter the owner deleted stays
/// deleted — it is in the offered ledger — so it is not listed again.
pub fn relationship_choices(dir: &Path) -> Vec<(String, bool)> {
    let store = Store::load(dir);
    let offered: BTreeSet<String> = std::fs::read_to_string(dir.join("relationships").join(SEEDED))
        .map(|s| {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let mut out: BTreeMap<String, bool> = store
        .relationships()
        .values()
        .map(|r| (r.name.clone(), r.starter))
        .collect();
    for (name, _) in STARTERS {
        if !offered.contains(name) {
            out.entry(name.to_string()).or_insert(true);
        }
    }
    out.into_iter().collect()
}

/// The names a settings form offers beside its fixed choices, read from the
/// stores where the form is built — the relationships a persona can name
/// ([`relationship_choices`]), the declared groups, the characters the
/// viewer may see.
#[derive(Debug, Clone, Default)]
pub struct FormChoices {
    pub relationships: Vec<String>,
    pub groups: Vec<String>,
    pub characters: Vec<String>,
}

/// Said on every switch the harness stores and does not read yet, so a form
/// never shows a toggle that silently does nothing.
const UNBUILT: &str = "Not built yet: saved now, used once it is.";

/// `persona.toml` as a form (`tomlform`): every key [`Settings`] reads
/// except `voice`, whose profiles are not built. Edited in place — the
/// owner's comments stay — and saved through [`edit_settings`].
pub fn settings_form(c: &FormChoices) -> crate::tomlform::Form {
    use crate::tomlform::{Field, Form, Kind, Opt, Section};
    let text = |max, placeholder: &str| Kind::Text {
        max,
        optional: true,
        placeholder: Some(placeholder.to_string()),
    };
    Form {
        sections: vec![
            Section::new("Who they are")
                .field(
                    Field::new("display", "Name", text(MAX_DISPLAY, "the folder name"))
                        .help("How the persona is shown and addressed."),
                )
                .field(
                    Field::new(
                        "relationship",
                        "Relationship",
                        Kind::Chips {
                            options: Opt::names(&c.relationships),
                            free: false,
                        },
                    )
                    .help("Templates that shape how it relates to you."),
                )
                .field(
                    Field::new(
                        "groups",
                        "Groups",
                        Kind::Chips {
                            options: Opt::names(&c.groups),
                            free: false,
                        },
                    )
                    .help("Groups share what you tell them with every member."),
                )
                .field(Field::new(
                    "character",
                    "Portrait",
                    Kind::Choice {
                        options: Opt::names(&c.characters),
                        none: Some("No portrait".into()),
                    },
                )),
            Section::new("Model and tools")
                .field(Field::new("model", "Model", text(128, "the default model")))
                .field(
                    Field::new(
                        "tools.allow",
                        "Tools",
                        Kind::Chips {
                            options: Vec::new(),
                            free: true,
                        },
                    )
                    .help(
                        "A tool a persona may never have, or one that could send \
                         somewhere the model names, is refused when a chat starts.",
                    ),
                )
                .field(Field::new(
                    "files.answers",
                    "Answers from",
                    Kind::Choice {
                        options: vec![
                            Opt::new("open", "Files and tools"),
                            Opt::new("files", "Files only")
                                .help("Tools that read the web are withheld."),
                        ],
                        none: None,
                    },
                )),
            Section::new("Safety")
                .help("On by default. Turning one off affects this persona only.")
                .field(
                    Field::toggle("safety.disclosure", "Disclosure")
                        .help("Every chat opens by saying this is an AI character you wrote."),
                )
                .field(Field::toggle("safety.crisis", "Crisis check").help(
                    "A message that suggests risk of self-harm pauses the persona \
                     and shows crisis resources.",
                ))
                .field(
                    Field::toggle("safety.reanchor", "Re-anchor")
                        .help("Every few turns, and after a gap, it is reminded who it is."),
                )
                .field(
                    Field::toggle("safety.dose", "Time spent")
                        .help("Counts turns per day and late at night, shown on the persona."),
                )
                .field(
                    Field::toggle("safety.breaks", "Break reminders")
                        .help(UNBUILT)
                        .unbuilt(),
                )
                .field(
                    Field::toggle("safety.farewell", "Farewell check")
                        .help(UNBUILT)
                        .unbuilt(),
                ),
            Section::new("Memory")
                .help(UNBUILT)
                .unbuilt()
                .field(Field::toggle("memory.episodic", "Past conversations"))
                .field(Field::toggle("memory.semantic", "Facts it learns"))
                .field(Field::new(
                    "memory.user_facts",
                    "Facts about you",
                    Kind::Choice {
                        options: vec![
                            Opt::new("shared", "Shared"),
                            Opt::new("own", "Its own"),
                            Opt::new("off", "None"),
                        ],
                        none: None,
                    },
                ))
                .field(Field::toggle("memory.about_me", "About-me notes"))
                .field(
                    Field::toggle("memory.self_update", "Self-update")
                        .help("Sections of its identity evolve; never Core or a fixed one."),
                )
                .field(Field::new(
                    "memory.fixed",
                    "Fixed sections",
                    Kind::Chips {
                        options: Vec::new(),
                        free: true,
                    },
                )),
        ],
    }
}

/// A form's values for this `persona.toml`, read through [`Settings`] so a
/// key the file leaves out shows its default.
pub fn settings_values(
    form: &crate::tomlform::Form,
    text: &str,
) -> Result<serde_json::Map<String, serde_json::Value>> {
    let settings: Settings = parse_toml(text)?;
    Ok(crate::tomlform::values(
        form,
        &serde_json::to_value(settings)?,
    ))
}

/// Apply a form's changes to `persona.toml` in place and save it through
/// [`write_owner_file`] — the same size, control-character, stale-`base`
/// and must-still-load checks a text save gets. A starter relationship
/// named for the first time is copied in first, as `create` does.
pub fn edit_settings(
    dir: &Path,
    name: &str,
    form: &crate::tomlform::Form,
    changes: &serde_json::Map<String, serde_json::Value>,
    base: &str,
) -> Result<State> {
    let text = read_owner_file(dir, name, OwnerFile::Settings)?;
    let edited = crate::tomlform::apply(form, &text, changes)?;
    // Before the write, not after: `write_owner_file` refuses a persona that
    // names a template not on disk, so a starter must be copied in first. A
    // refused save leaves it seeded — idempotent, and what `create` does.
    if changes.contains_key("relationship") {
        seed_starters(dir)?;
    }
    write_owner_file(dir, name, OwnerFile::Settings, &edited, Some(base))
}

/// Is this line a root-table `character = …` assignment? Line-based on
/// purpose, so a file that no longer parses is redacted all the same: every
/// line before the first `[table]` header whose key is `character`, bare or
/// quoted. A `# character = …` comment is not one.
fn is_character_line(line: &str) -> bool {
    let t = line.trim_start();
    let key = t
        .strip_prefix("character")
        .or_else(|| t.strip_prefix("\"character\""))
        .or_else(|| t.strip_prefix("'character'"));
    matches!(key, Some(rest) if rest.trim_start().starts_with('='))
}

/// The root-table lines of a TOML text, by index: those before the first
/// `[table]` or `[[array]]` header.
fn root_lines(text: &str) -> usize {
    text.lines()
        .position(|l| l.trim_start().starts_with('['))
        .unwrap_or(usize::MAX)
}

/// The lines, first and last inclusive, that a root `character = …` spans:
/// from its key to the end of its value, however many lines that is (a
/// `"""` string), read with `toml_edit`'s span-keeping parser. A text that
/// does not parse falls back to the line, and every line after it while a
/// multi-line string it opened is still open — never to less (review of
/// #430: a line-based cut left the rest of a multi-line value served).
fn character_lines(text: &str) -> Option<(usize, usize)> {
    let line_of = |at: usize| text[..at.min(text.len())].matches('\n').count();
    if let Ok(doc) = toml_edit::Document::parse(text) {
        let spanned = doc
            .as_table()
            .get_key_value("character")
            .and_then(|(k, v)| Some((k.span()?.start, v.span()?.end)));
        match spanned {
            Some((start, end)) => return Some((line_of(start), line_of(end.saturating_sub(1)))),
            None if doc.as_table().get("character").is_none() => return None,
            None => {}
        }
    }
    let root = root_lines(text);
    let lines: Vec<&str> = text.lines().collect();
    let first = (0..lines.len().min(root)).find(|&i| is_character_line(lines[i]))?;
    let opens = ["\"\"\"", "'''"]
        .into_iter()
        .find(|q| lines[first].matches(q).count() % 2 == 1);
    let last = match opens {
        Some(q) => (first + 1..lines.len())
            .find(|&i| lines[i].contains(q))
            .unwrap_or(lines.len() - 1),
        None => first,
    };
    Some((first, last))
}

/// `persona.toml` as a locked page may see it: without the `character`
/// assignment when the persona's portrait is a locked library character
/// (owner ruling, 2026-09-30 — the link is hidden, never cut). Everything
/// else is the file as written.
pub fn hide_character(text: &str) -> String {
    let Some((first, last)) = character_lines(text) else {
        return text.to_string();
    };
    let mut out: String = text
        .lines()
        .enumerate()
        .filter(|(i, _)| !(first..=last).contains(i))
        .map(|(_, l)| format!("{l}\n"))
        .collect();
    if !text.ends_with('\n') {
        out.pop();
    }
    out
}

/// A save from a page that was shown [`hide_character`]'s text: the hidden
/// assignment put back **as it is in `disk`** — its spelling, every line of
/// it and any comment on it — at its old line number, kept inside the root
/// table (review of #430: a fresh bare `character = …` lost the owner's
/// comment). Unless the owner wrote a `character` of their own, which stands.
pub fn restore_character(text: &str, disk: &str) -> String {
    if character_lines(text).is_some() {
        return text.to_string();
    }
    let Some((first, last)) = character_lines(disk) else {
        return text.to_string();
    };
    let hidden: Vec<&str> = disk.lines().skip(first).take(last - first + 1).collect();
    let mut lines: Vec<&str> = text.lines().collect();
    let at = first.min(root_lines(text)).min(lines.len());
    lines.splice(at..at, hidden);
    let mut out = lines.join("\n");
    if text.ends_with('\n') || text.is_empty() {
        out.push('\n');
    }
    out
}

/// The sections a Markdown owner file must keep: `## Core` in identity.md
/// is who they are at heart, and the re-anchor reads it.
pub fn fixed_sections(file: OwnerFile) -> &'static [&'static str] {
    match file {
        OwnerFile::Identity => &["Core"],
        OwnerFile::Motivation | OwnerFile::Settings => &[],
    }
}

/// Save a Markdown owner file from its form (`mdform`): written in the
/// canonical layout and saved through [`write_owner_file`], with every check
/// a text save gets.
pub fn edit_markdown(
    dir: &Path,
    name: &str,
    file: OwnerFile,
    doc: &crate::mdform::Doc,
    base: &str,
) -> Result<State> {
    if file == OwnerFile::Settings {
        bail!("persona.toml is not Markdown");
    }
    let text = crate::mdform::join(doc, fixed_sections(file))?;
    write_owner_file(dir, name, file, &text, Some(base))
}

/// A text's digest, as a page holds it to say which version of a file it
/// opened.
pub fn text_digest(text: &str) -> String {
    sha2::Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A persona's owner file as it stands (a missing motivation is empty).
pub fn read_owner_file(dir: &Path, name: &str, file: OwnerFile) -> Result<String> {
    validate_persona_name(name)?;
    let path = dir.join(name).join(file.file_name());
    if !dir.join(name).join("persona.toml").is_file() {
        bail!("no persona named `{name}`");
    }
    if file == OwnerFile::Motivation && !path.exists() {
        return Ok(String::new());
    }
    read_prose(&path)
}

/// Save the owner's text for one file **verbatim** — the text the owner
/// wrote, comments and all, never a re-serialisation, so an edit from the
/// page keeps the rule that code never rewrites an owner file (§4.4).
///
/// Refused, with the file left as it was: text over the size cap or holding
/// a control character; a `base` that is not the digest of the file as it
/// stands (an edit made elsewhere since the page opened it — neither side
/// silently overwrites the other); and a save after which the persona would
/// not load or could not be versioned (a misspelt key, a relationship that
/// is not there). Otherwise a version is taken, and new chats use it.
pub fn write_owner_file(
    dir: &Path,
    name: &str,
    file: OwnerFile,
    text: &str,
    base: Option<&str>,
) -> Result<State> {
    validate_persona_name(name)?;
    let pdir = dir.join(name);
    if !pdir.join("persona.toml").is_file() {
        bail!("no persona named `{name}`");
    }
    if text.len() as u64 > MAX_PROSE_BYTES {
        bail!(
            "{} is capped at {} KB",
            file.file_name(),
            MAX_PROSE_BYTES / 1024
        );
    }
    if let Some(c) = forbidden_control(text) {
        bail!(
            "{} would hold a control character (U+{:04X}); only newlines and tabs are allowed",
            file.file_name(),
            c as u32
        );
    }
    let path = pdir.join(file.file_name());
    let old = if path.exists() {
        Some(read_prose(&path)?)
    } else {
        None
    };
    if let Some(base) = base {
        if text_digest(old.as_deref().unwrap_or("")) != base {
            return Err(StaleEdit(file).into());
        }
    }
    write_private(&path, text.as_bytes())?;
    let restore = |why: anyhow::Error| -> anyhow::Error {
        let put_back = match &old {
            Some(old) => write_private(&path, old.as_bytes()),
            None => std::fs::remove_file(&path).map_err(Into::into),
        };
        match put_back {
            Ok(()) => why,
            Err(e) => why.context(format!(
                "and the previous {} could not be put back: {e:#}",
                file.file_name()
            )),
        }
    };
    // The same door every reader uses, so what is saved is what loads.
    let store = Store::load(dir);
    if store.get(name).is_none() {
        let why = store
            .errors()
            .iter()
            .find(|e| e.path == pdir.join("persona.toml"))
            .map(|e| e.why.clone())
            .unwrap_or_else(|| "it did not load".into());
        return Err(restore(anyhow!("not saved: {why}")));
    }
    snapshot(dir, name).map_err(|e| restore(e.context("not saved")))
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
    private_dir(&removed)?;
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    // The stamp alone is per second: remove, recreate and remove again in
    // one second must not collide with the first.
    let unique = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let to = removed.join(format!("{name}-{stamp}-{unique}"));
    std::fs::rename(&pdir, &to)?;
    Ok(to)
}

/// Fixtures shared by this module's tests and `agent`'s.
#[cfg(test)]
mod tests_support {
    use super::*;

    pub fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mecha-persona-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub fn no_lib() -> imagelib::Library {
        imagelib::Library::load(&std::env::temp_dir().join("mecha-persona-no-such-lib")).0
    }

    pub fn new(name: &str) -> NewPersona {
        NewPersona {
            name: name.into(),
            origin: Origin::Owner,
            ..NewPersona::default()
        }
    }

    pub fn fill_core(dir: &Path, name: &str) {
        let path = dir.join(name).join("identity.md");
        let text = std::fs::read_to_string(&path).unwrap().replace(
            "## Core\n",
            "## Core\nA marine ecologist who distrusts easy answers.\n",
        );
        std::fs::write(path, text).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::*;
    use super::*;

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
        // state.toml goes through the same door, and fails closed.
        std::fs::write(dir.join("mara/identity.md"), "## Core\nDry.\n").unwrap();
        let state = dir.join("mara/state.toml");
        let text = std::fs::read_to_string(&state).unwrap();
        std::fs::write(&state, text.replace("created = \"", "created = \"\\u001b")).unwrap();
        let p = Store::load(&dir).get("mara").cloned().unwrap();
        assert_eq!(p.state.status, Status::Candidate);
        assert!(
            p.notes.iter().any(|n| n.contains("state.toml")),
            "{:?}",
            p.notes
        );
        std::fs::write(&state, text).unwrap();
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

    #[cfg(unix)]
    #[test]
    fn the_store_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch();
        let mut n = new("mara");
        n.relationships = vec!["colleague".into()];
        let v = create(&dir, &no_lib(), n).unwrap().state;
        add_group(&dir, "work", "").unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        for d in [
            "",
            "mara",
            "mara/files",
            "mara/sessions",
            "mara/versions",
            "files",
            "relationships",
            "groups/work",
            "groups/work/files",
        ] {
            assert_eq!(mode(&dir.join(d)), 0o700, "{d}/");
        }
        let snap = format!("mara/versions/{}", v.digest);
        for f in [
            "mara/persona.toml",
            "mara/identity.md",
            "mara/motivation.md",
            "mara/state.toml",
            "mara/versions/log.jsonl",
            "about-me.md",
            "groups.toml",
            "groups/work/about-me.md",
            "relationships/colleague.md",
            "relationships/.seeded",
        ] {
            assert_eq!(mode(&dir.join(f)), 0o600, "{f}");
        }
        assert_eq!(mode(&dir.join(&snap)), 0o700);
        assert_eq!(mode(&dir.join(&snap).join("identity.md")), 0o600);
        assert_eq!(mode(&dir.join(&snap).join("relationships")), 0o700);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_candidate_character_is_named_as_unapproved() {
        let dir = scratch();
        let lib_dir = dir.join("imagelib");
        let mut png = Vec::new();
        image::RgbImage::new(4, 4)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        for (name, origin) in [("mara", Origin::Owner), ("ada", Origin::ModelClean)] {
            imagelib::create(
                &lib_dir,
                imagelib::NewEntry {
                    kind: imagelib::Kind::Character,
                    name: name.into(),
                    text: "tall, dark hair".into(),
                    portrait: Some(png.clone()),
                    source_seed: None,
                    origin,
                    locked: false,
                },
            )
            .unwrap();
        }
        let lib = imagelib::Library::load(&lib_dir).0;
        let mut n = new("mara");
        n.character = Some("mara".into());
        create(&dir.join("p"), &lib, n).unwrap();
        let mut n = new("ada");
        n.character = Some("ada".into());
        let refused = create(&dir.join("p"), &lib, n).unwrap_err();
        assert!(format!("{refused}").contains("candidate"), "{refused}");
        let mut n = new("priya");
        n.character = Some("priya".into());
        let refused = create(&dir.join("p"), &lib, n).unwrap_err();
        assert!(
            format!("{refused}").contains("not in the image library"),
            "{refused}"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_load_error_is_blamed_on_its_own_file() {
        let dir = scratch();
        ensure_layout(&dir).unwrap();
        // A voice profile named like a persona that does not exist.
        std::fs::create_dir_all(dir.join("voices/mara")).unwrap();
        std::fs::write(dir.join("voices/mara/profile.toml"), "").unwrap();
        let e = approve(&dir, "mara").unwrap_err();
        assert_eq!(format!("{e}"), "no persona named `mara`");
        std::fs::remove_dir_all(dir).ok();
    }

    /// The page's save: verbatim, versioned, and refused — with the file
    /// left as it was — when stale, unloadable or unversionable.
    #[test]
    fn an_owner_file_is_saved_verbatim_or_not_at_all() {
        let dir = scratch();
        create(&dir, &no_lib(), new("mara")).unwrap();
        let before = read_owner_file(&dir, "mara", OwnerFile::Identity).unwrap();
        let base = text_digest(&before);
        let text = "# Mara\n<!-- a note the page must keep -->\n## Core\nDry.\n";
        let state = write_owner_file(&dir, "mara", OwnerFile::Identity, text, Some(&base)).unwrap();
        assert_eq!(state.version, 2);
        assert_eq!(
            read_owner_file(&dir, "mara", OwnerFile::Identity).unwrap(),
            text
        );

        // Stale: opened before that save.
        let e = write_owner_file(
            &dir,
            "mara",
            OwnerFile::Identity,
            "## Core\nx\n",
            Some(&base),
        )
        .unwrap_err();
        assert!(format!("{e}").contains("changed since"), "{e}");

        // Unloadable settings: refused by name, and the file is as it was.
        let settings = read_owner_file(&dir, "mara", OwnerFile::Settings).unwrap();
        let bad = settings.replace("crisis     = true", "crsis = true");
        let e = write_owner_file(&dir, "mara", OwnerFile::Settings, &bad, None).unwrap_err();
        assert!(format!("{e:#}").contains("crsis"), "{e:#}");
        assert_eq!(
            read_owner_file(&dir, "mara", OwnerFile::Settings).unwrap(),
            settings
        );

        // Loadable but unversionable (a template that is not there): same.
        let missing = settings.replace(
            "# relationship = \"colleague\"",
            "relationship = \"rival\"\n# was",
        );
        assert!(write_owner_file(&dir, "mara", OwnerFile::Settings, &missing, None).is_err());
        assert_eq!(
            read_owner_file(&dir, "mara", OwnerFile::Settings).unwrap(),
            settings
        );

        // Control characters never reach the file.
        assert!(write_owner_file(&dir, "mara", OwnerFile::Motivation, "a\u{1b}[2J", None).is_err());
        assert!(read_owner_file(&dir, "nobody", OwnerFile::Identity).is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    /// The form's save: values set in place, every comment the template
    /// wrote still there, and the same stale check a text save gets.
    #[test]
    fn a_form_edit_keeps_the_owners_comments_and_checks_base() {
        use serde_json::json;
        let dir = scratch();
        create(&dir, &no_lib(), new("mara")).unwrap();
        let before = read_owner_file(&dir, "mara", OwnerFile::Settings).unwrap();
        let choices = FormChoices {
            relationships: relationship_choices(&dir)
                .into_iter()
                .map(|(n, _)| n)
                .collect(),
            ..FormChoices::default()
        };
        let form = settings_form(&choices);
        let values = settings_values(&form, &before).unwrap();
        assert_eq!(values["safety.crisis"], json!(true));
        assert_eq!(values["memory.user_facts"], json!("shared"));

        let starter = choices.relationships[0].clone();
        let changes = json!({ "safety.breaks": true, "relationship": [starter] });
        let state = edit_settings(
            &dir,
            "mara",
            &form,
            changes.as_object().unwrap(),
            &text_digest(&before),
        )
        .unwrap();
        assert_eq!(state.version, 2);
        let after = read_owner_file(&dir, "mara", OwnerFile::Settings).unwrap();
        let comments = |t: &str| t.lines().filter(|l| l.contains('#')).count();
        assert_eq!(comments(&after), comments(&before), "{after}");
        let v = settings_values(&form, &after).unwrap();
        assert_eq!(v["safety.breaks"], json!(true));
        assert_eq!(v["relationship"], json!([starter]));

        // Opened before that save: refused, and the file is as it was.
        let stale = json!({ "safety.crisis": false });
        let e = edit_settings(
            &dir,
            "mara",
            &form,
            stale.as_object().unwrap(),
            &text_digest(&before),
        )
        .unwrap_err();
        assert!(e.downcast_ref::<StaleEdit>().is_some(), "{e:#}");
        // A name the form does not offer: refused before anything is read.
        let stranger = json!({ "relationship": ["stranger"] });
        assert!(edit_settings(
            &dir,
            "mara",
            &form,
            stranger.as_object().unwrap(),
            &text_digest(&after)
        )
        .is_err());
        assert_eq!(
            read_owner_file(&dir, "mara", OwnerFile::Settings).unwrap(),
            after
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// Hidden, never cut: the served text has no `character` line, and a
    /// save of it gets the link back — unless the owner wrote their own.
    #[test]
    fn a_hidden_character_is_redacted_and_restored() {
        let text = "# c\n# character = \"example\"\ndisplay = \"Mara\"\ncharacter  = \"theo\"  # portrait\n\n[tools]\ncharacter = \"not-root\"\n";
        let shown = hide_character(text);
        assert!(!shown.contains("\"theo\""), "{shown}");
        assert!(shown.contains("# character = \"example\""), "comments stay");
        assert!(
            shown.contains("character = \"not-root\""),
            "only the root key"
        );
        let back = restore_character(&shown, text);
        let parsed: toml::Table = toml::from_str(&back).unwrap();
        assert_eq!(parsed["character"].as_str(), Some("theo"), "{back}");
        // Back as it was, comment and all, where it was.
        assert_eq!(back, text, "{back}");
        assert_eq!(parsed["tools"]["character"].as_str(), Some("not-root"));
        // The owner named a portrait of their own: theirs stands.
        let chosen = shown.replace(
            "display = \"Mara\"",
            "display = \"Mara\"\ncharacter = \"maya\"",
        );
        assert_eq!(restore_character(&chosen, text), chosen);
        // A file with no tables, and one that does not parse, both work.
        assert_eq!(hide_character("character = \"theo\"\n"), "");
        assert_eq!(
            restore_character("", "character = \"theo\"\n"),
            "character = \"theo\"\n"
        );
        assert!(!hide_character("character = \"theo\"\ndisplay = ").contains("theo"));
        // A multi-line value goes whole, parsed or not, and comes back whole.
        let multi = "display = \"Mara\"\ncharacter = \"\"\"\ntheo\"\"\"\n\n[tools]\nallow = []\n";
        let shown = hide_character(multi);
        assert!(!shown.contains("theo"), "{shown}");
        assert!(toml::from_str::<toml::Table>(&shown).is_ok(), "{shown}");
        assert_eq!(restore_character(&shown, multi), multi);
        let broken = "character = '''\ntheo\n'''\ndisplay = ";
        assert!(
            !hide_character(broken).contains("theo"),
            "{}",
            hide_character(broken)
        );
    }

    /// Every path the settings form declares is a key `Settings` carries: a
    /// typo or a rename would otherwise draw as "off" and fail only at save,
    /// as an unknown field (review of #430; the shape of
    /// `every_field_of_config_is_reachable_from_a_file`).
    #[test]
    fn every_settings_form_path_is_a_settings_key() {
        let typed = serde_json::to_value(parse_toml::<Settings>("").unwrap()).unwrap();
        let form = settings_form(&FormChoices::default());
        for field in form.sections.iter().flat_map(|s| &s.fields) {
            let (parents, leaf) = field.path.rsplit_once('.').unwrap_or(("", &field.path));
            let table = parents
                .split('.')
                .filter(|s| !s.is_empty())
                .try_fold(&typed, |at, seg| at.get(seg))
                .and_then(|t| t.as_object())
                .unwrap_or_else(|| panic!("no table for `{}`", field.path));
            assert!(
                table.contains_key(leaf),
                "`{}` is not a Settings key",
                field.path
            );
        }
    }

    #[test]
    fn a_new_relationship_is_added_and_never_overwrites() {
        let dir = scratch();
        add_relationship(&dir, "mentor", "# Mentor\n\nAsks what you tried first.\n").unwrap();
        let store = Store::load(&dir);
        assert!(!store.relationships()["mentor"].starter);
        // A starter the owner edited, and the one just made, both refuse.
        assert!(add_relationship(&dir, "friend", "# Mine").is_err());
        assert!(add_relationship(&dir, "mentor", "# Again").is_err());
        assert_eq!(
            std::fs::read_to_string(dir.join("relationships/mentor.md")).unwrap(),
            "# Mentor\n\nAsks what you tried first.\n"
        );
        for (name, text) in [
            ("Bad Name", "x"),
            ("files-ok", ""),
            ("ctl", "a\u{1b}b"),
            ("fm", "+++\ntool = []\n+++\n# T"),
        ] {
            assert!(add_relationship(&dir, name, text).is_err(), "{name}");
        }
        std::fs::remove_dir_all(dir).ok();
    }

    /// Listing what a persona can name writes nothing, and a deleted starter
    /// is not offered again.
    #[test]
    fn relationship_choices_are_read_without_writing() {
        let dir = scratch();
        let fresh = relationship_choices(&dir);
        assert_eq!(fresh.len(), STARTERS.len());
        assert!(
            !dir.join("relationships").exists(),
            "listing wrote the store"
        );
        seed_starters(&dir).unwrap();
        std::fs::remove_file(dir.join("relationships/coach.md")).unwrap();
        add_relationship(&dir, "mentor", "# Mentor\n").unwrap();
        let names: Vec<String> = relationship_choices(&dir)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert!(!names.contains(&"coach".to_string()) && names.contains(&"mentor".to_string()));
        std::fs::remove_dir_all(dir).ok();
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
        // A description the loader would refuse is refused at the door, and
        // the file it would have broken still loads.
        assert!(add_group(&dir, "play", "a\u{1b}[2J").is_err());
        assert!(add_group(&dir, "play", "two\nlines").is_err());
        assert_eq!(Store::load(&dir).groups().len(), 1);
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
        let (s, body) = front_matter("+++\r\nanswers = \"files\"\r\n+++\r\n# T\r\n").unwrap();
        assert_eq!(s.answers, Some(Answers::Files), "CRLF front matter");
        assert_eq!(body, "# T\n");
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
