//! Dashboards: a spec the model writes, loaders the owner installs, and the
//! checks that stand between them and anything that renders or refreshes.
//!
//! `docs/LIVE-DASHBOARD-DESIGN.md` is the authority; this module is its §2
//! and §3 — the parts that are functions of the files alone. Nothing here
//! opens a source, runs a query, or schedules anything.
//!
//! Three decisions carry it:
//!
//! **A spec names no destinations.** The model writes a dashboard as data,
//! and a hand-written renderer draws it. The one property the spec must have
//! for that to be safe on the same origin as the outbox's approve button is
//! the one `Egress::Blind` is earned by: no field of it says where anything
//! comes from or goes to. Charts bind to a dataset by name, and every string
//! that is an address is refused wherever it sits ([`spec::Spec::parse`]).
//!
//! **Refusals are written for the model that will retry.** A 27B model that
//! reads "invalid spec" learns nothing; one that reads
//! `/panels/2/vegalite/encoding/href: the href channel ...` fixes the line.
//! So every check collects every [`Refusal`] it can find, each with a JSON
//! pointer and the rule it broke, rather than stopping at the first.
//!
//! **A shape refusal never echoes a value.** A loader's output may carry
//! third-party text, and a refusal is shown on the owner's surfaces and may
//! reach a later model context. It names the row, the column and the type it
//! expected — never what it found.

pub mod loader;
pub mod spec;
mod vegalite;

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub use loader::{Column, ColumnType, Loader, ShapeRefusal};
pub use spec::{Panel, Spec};

/// One thing a check refused: where, and which rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// A JSON pointer into the file the check read (`/panels/2/title`), or a
    /// file name and a pointer for checks that span files.
    pub at: String,
    pub rule: String,
}

impl Refusal {
    pub fn new(at: impl Into<String>, rule: impl Into<String>) -> Self {
        Refusal {
            at: at.into(),
            rule: rule.into(),
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let at = if self.at.is_empty() { "/" } else { &self.at };
        write!(f, "{at}: {}", self.rule)
    }
}

/// Every refusal one check found, in the order it found them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusals(pub Vec<Refusal>);

impl fmt::Display for Refusals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, r) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{r}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Refusals {}

/// `^[a-z][a-z0-9_]{0,63}$` — dataset, column, filter and source names. They
/// become file names and JSON keys, so the set is deliberately small.
pub(crate) fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some('a'..='z'))
        && s.len() <= 64
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_'))
}

/// Append one segment to a JSON pointer, escaping per RFC 6901.
pub(crate) fn pointer(at: &str, segment: &str) -> String {
    format!("{at}/{}", segment.replace('~', "~0").replace('/', "~1"))
}

/// `~/.mecha/dashboards/`.
pub fn dir() -> Result<PathBuf> {
    Ok(crate::work::mecha_home()?.join("dashboards"))
}

/// A dashboard as installed: its spec and the loader behind each dataset,
/// checked against each other.
#[derive(Debug, Clone)]
pub struct Installed {
    pub id: String,
    pub spec: Spec,
    /// Keyed by dataset name, which is the loader's file stem.
    pub loaders: BTreeMap<String, Loader>,
}

impl Installed {
    /// Read `<dir>/dashboard.json` and `<dir>/loaders/*.toml`, and check each
    /// alone and all of them together. `Err` is for I/O the caller cannot
    /// route around; a dashboard that is merely wrong is `Ok(Err(refusals))`,
    /// so a caller can show every reason at once.
    pub fn load(dir: &Path) -> Result<std::result::Result<Installed, Refusals>> {
        let id = dir
            .file_name()
            .and_then(|n| n.to_str())
            .context("a dashboard directory needs a name")?
            .to_string();
        let mut refusals = Vec::new();
        if !is_identifier(&id) {
            refusals.push(Refusal::new(
                "",
                format!("the dashboard's directory name must match [a-z][a-z0-9_]* (got {id:?})"),
            ));
        }

        let spec_path = dir.join("dashboard.json");
        // A directory with loaders and no spec is a half-written dashboard —
        // a refusal beside the others, not I/O to give up on.
        let spec = match std::fs::read_to_string(&spec_path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                refusals.push(Refusal::new(
                    "dashboard.json",
                    "a dashboard needs a dashboard.json beside its loaders/",
                ));
                None
            }
            Err(e) => {
                return Err(e).with_context(|| format!("reading {}", spec_path.display()));
            }
            Ok(text) => match Spec::parse(&text) {
                Ok(spec) => Some(spec),
                Err(r) => {
                    refusals.extend(prefixed("dashboard.json", r.0));
                    None
                }
            },
        };

        let mut loaders = BTreeMap::new();
        let loader_dir = dir.join("loaders");
        if loader_dir.is_dir() {
            let mut paths: Vec<PathBuf> = std::fs::read_dir(&loader_dir)
                .with_context(|| format!("reading {}", loader_dir.display()))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "toml"))
                .collect();
            paths.sort();
            for path in paths {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?;
                let file = format!("loaders/{name}.toml");
                match Loader::parse(&name, &text) {
                    Ok(loader) => {
                        loaders.insert(name, loader);
                    }
                    Err(r) => refusals.extend(prefixed(&file, r.0)),
                }
            }
        }

        if let Some(spec) = &spec {
            refusals.extend(cross_check(spec, &loaders));
        }
        match spec {
            Some(spec) if refusals.is_empty() => Ok(Ok(Installed { id, spec, loaders })),
            _ => Ok(Err(Refusals(refusals))),
        }
    }
}

fn prefixed(file: &str, refusals: Vec<Refusal>) -> impl Iterator<Item = Refusal> + '_ {
    refusals.into_iter().map(move |r| {
        let at = if r.at.is_empty() {
            file.to_string()
        } else {
            format!("{file}#{}", r.at)
        };
        Refusal::new(at, r.rule)
    })
}

/// What only the spec and its loaders together can answer: every declared
/// dataset has a loader and every loader a dataset, and every column a KPI,
/// table or filter names is one its loader declares.
///
/// Chart fields are not checked here — a chart's transforms mint new fields
/// (`calculate`'s `as`, `fold`'s `key`/`value`), and following them is the
/// renderer's job at preview, where a wrong field shows up as an empty chart
/// the model can see.
fn cross_check(spec: &Spec, loaders: &BTreeMap<String, Loader>) -> Vec<Refusal> {
    let mut out = Vec::new();
    for (i, name) in spec.datasets.iter().enumerate() {
        if !loaders.contains_key(name) {
            out.push(Refusal::new(
                format!("dashboard.json#/datasets/{i}"),
                format!("dataset {name:?} has no loader: expected loaders/{name}.toml"),
            ));
        }
    }
    for name in loaders.keys() {
        if !spec.datasets.contains(name) {
            out.push(Refusal::new(
                format!("loaders/{name}.toml"),
                format!("loader {name:?} feeds no dataset: add it to the spec's `datasets` or remove it"),
            ));
        }
    }

    let has_column = |dataset: &str, field: &str| {
        loaders
            .get(dataset)
            .is_none_or(|l| l.columns.iter().any(|c| c.name == field))
    };
    let missing = |at: String, dataset: &str, field: &str| {
        Refusal::new(
            at,
            format!("column {field:?} is not declared by the loader for dataset {dataset:?}"),
        )
    };
    for (i, filter) in spec.filters.iter().enumerate() {
        if !has_column(&filter.dataset, &filter.field) {
            out.push(missing(
                format!("dashboard.json#/filters/{i}/field"),
                &filter.dataset,
                &filter.field,
            ));
        }
    }
    for (i, panel) in spec.panels.iter().enumerate() {
        match panel {
            Panel::Kpi { dataset, value, .. } => {
                if let Some(field) = &value.field {
                    if !has_column(dataset, field) {
                        out.push(missing(
                            format!("dashboard.json#/panels/{i}/value/field"),
                            dataset,
                            field,
                        ));
                    }
                }
            }
            Panel::Table {
                dataset, columns, ..
            } => {
                for (j, field) in columns.iter().enumerate() {
                    if !has_column(dataset, field) {
                        out.push(missing(
                            format!("dashboard.json#/panels/{i}/columns/{j}"),
                            dataset,
                            field,
                        ));
                    }
                }
            }
            Panel::Chart { .. } | Panel::Text { .. } => {}
        }
    }
    out
}

#[cfg(test)]
mod tests;
