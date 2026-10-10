//! `~/.mecha/hud/sources.toml`: the sources a loader may name.
//!
//! The owner writes this file; no model does. A loader names a source by
//! identifier and never holds a path or a credential, so what a model-drafted
//! loader can reach is exactly what the owner registered here (design §3.1).
//!
//! **Kind decides `external`; `content` only tightens it.** A remote kind is
//! always external whatever the file says — writing `content` on one is a
//! refusal, not a line that silently does nothing. A local source is
//! third-party unless the owner says `content = "owner"`: unknown is never
//! clean.
//!
//! Only `sqlite` is runnable in this build. The other kinds the design orders
//! (`duckdb`, `postgres`, `sheets`, `mecha`) are refused by name, so a file
//! written ahead of the build says what it is waiting for instead of failing
//! as a typo.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::{is_identifier, Refusal, Refusals};

/// The kinds the design names but this build cannot run yet.
const LATER_KINDS: &[&str] = &["duckdb", "postgres", "sheets", "mecha"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceKind {
    /// A local SQLite file, opened read-only and confined (`runner`).
    Sqlite { path: PathBuf },
}

/// Who wrote the values a source returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Content {
    Owner,
    ThirdParty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub name: String,
    pub kind: SourceKind,
    content: Content,
}

impl Source {
    /// Whether rows from this source carry text someone other than the owner
    /// wrote — what a tool returning them sets `.from_outside()` from.
    pub fn external(&self) -> bool {
        match self.kind {
            SourceKind::Sqlite { .. } => self.content == Content::ThirdParty,
        }
    }
}

/// The registered sources, by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sources {
    by_name: BTreeMap<String, Source>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    #[serde(default)]
    source: BTreeMap<String, WireSource>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSource {
    kind: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    url_env: Option<String>,
}

impl Sources {
    pub fn get(&self, name: &str) -> Option<&Source> {
        self.by_name.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str)
    }

    /// Read `sources.toml`. A missing file is no sources — the owner has
    /// registered none — and every loader that names one is then refused at
    /// install and at refresh, by name.
    pub fn load(path: &Path) -> anyhow::Result<Result<Sources, Refusals>> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Ok(Sources::default()))
            }
            Err(e) => {
                return Ok(Err(Refusals(vec![Refusal::new(
                    "sources.toml",
                    format!("cannot be read: {e}"),
                )])))
            }
        };
        Ok(Self::parse(&text, home_dir().as_deref()))
    }

    /// Parse and check a `sources.toml`; `home` expands a leading `~/`.
    pub fn parse(text: &str, home: Option<&Path>) -> Result<Sources, Refusals> {
        let wire: Wire = toml::from_str(text).map_err(|e| {
            Refusals(vec![Refusal::new(
                "sources.toml",
                format!("does not match the sources file's shape: {e}"),
            )])
        })?;
        let mut out = Vec::new();
        let mut by_name = BTreeMap::new();
        for (name, w) in wire.source {
            let at = format!("sources.toml#/source/{name}");
            if !is_identifier(&name) {
                out.push(Refusal::new(&at, "a source name matches [a-z][a-z0-9_]*"));
                continue;
            }
            match source(&name, w, home) {
                Ok(s) => {
                    by_name.insert(name, s);
                }
                Err(rule) => out.push(Refusal::new(&at, rule)),
            }
        }
        if out.is_empty() {
            Ok(Sources { by_name })
        } else {
            Err(Refusals(out))
        }
    }
}

fn source(name: &str, w: WireSource, home: Option<&Path>) -> Result<Source, String> {
    if LATER_KINDS.contains(&w.kind.as_str()) {
        return Err(format!(
            "kind {:?} is designed (LIVE-DASHBOARD-DESIGN §3.1) but not available in this build; \
             only \"sqlite\" runs today",
            w.kind
        ));
    }
    if w.kind != "sqlite" {
        return Err(format!(
            "unknown kind {:?}; this build runs \"sqlite\"",
            w.kind
        ));
    }
    if w.url_env.is_some() {
        return Err("`url_env` belongs to a remote kind; a sqlite source has a `path`".into());
    }
    let content = match w.content.as_deref() {
        None | Some("third-party") => Content::ThirdParty,
        Some("owner") => Content::Owner,
        Some(other) => {
            return Err(format!(
                "content is \"owner\" or \"third-party\" (unset is third-party), not {other:?}"
            ))
        }
    };
    let raw = w.path.ok_or("a sqlite source needs a `path`")?;
    let path = match raw.strip_prefix("~/") {
        Some(rest) => home
            .ok_or("`~/` used, but there is no home directory")?
            .join(rest),
        None => PathBuf::from(&raw),
    };
    if !path.is_absolute() {
        return Err(format!(
            "a source path is absolute or starts with `~/`, so it never depends on where mecha runs (got {raw:?})"
        ));
    }
    Ok(Source {
        name: name.to_string(),
        kind: SourceKind::Sqlite { path },
        content,
    })
}

fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}
