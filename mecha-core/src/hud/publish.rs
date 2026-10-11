//! A board on the factory (`docs/LIVE-DASHBOARD-DESIGN.md` §6, step 5).
//!
//! Two halves, both without a model:
//!
//! - **[`assemble`]** writes a board's release bundle — the page, the
//!   vendored renderer, the checked spec, and `datasets.json` (each dataset's
//!   columns and the digest of the loader being released). It is staged in the
//!   outbox; releasing it is the owner's review of both the page and the
//!   queries (§6.3).
//! - **[`plan`]** decides, for one refreshed dataset, whether home may push it:
//!   only to a release the factory client recorded, only a dataset that
//!   release declared, and only while its loader is the one released. A
//!   changed loader is a new publish, never a quiet push.
//!
//! What was released is read from the factory client's own local store
//! (`~/.mecha/bundles/<id>/alias.json` and the version's `datasets.json`),
//! written when the outbox ran `bundle_publish` — home never asks the box.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::loader::Column;
use super::store::Dataset;
use super::Installed;

/// Where `[web] assets` keeps the standalone renderer (`npm run build`
/// writes it there; `web/vite.hud.config.js`).
pub const RENDERER_DIR: &str = "hud-renderer";
/// Inside a bundle: the renderer, pinned by digest on the factory side.
pub const BUNDLE_RENDERER_DIR: &str = "renderer";
/// The file a release declares its datasets in — the factory's
/// `mecha_manifest::hud::DATASETS_FILE`.
pub const DATASETS_FILE: &str = "datasets.json";
/// Beside a board: which bundle it is published as.
const PUBLISHED_FILE: &str = "published.json";
/// Beside a board: the generation last pushed per dataset, and under which
/// release, so a minute's tick pushes only what changed.
const PUSHED_FILE: &str = "pushed.json";

/// Where release bundles wait for review: the HUD store's own directory,
/// never `~/.mecha/work/` — a run's workspace is a producer directory there
/// (a trigger named `hud` would be jailed over `work/hud`), and a pending
/// review object must not be writable by a run, nor swept by `work clean`
/// (review of #651).
pub fn staged_root(store_root: &Path) -> std::path::PathBuf {
    store_root.join("staged")
}

/// The bundle id a board is published as. A board id is already an
/// identifier, and the prefix keeps dashboards apart from every other bundle
/// on the box.
pub fn bundle_id(board: &str) -> String {
    format!("hud-{board}")
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Declared {
    pub datasets: BTreeMap<String, DeclaredDataset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclaredDataset {
    /// As the loader declares them: `loader::Column` is already the wire's
    /// `{name, type}`.
    pub columns: Vec<Column>,
    pub loader_digest: String,
}

/// What a release of `installed` declares: every dataset, its columns, and
/// the loader being released.
pub fn declared(installed: &Installed) -> Declared {
    Declared {
        datasets: installed
            .loaders()
            .iter()
            .map(|(name, loader)| {
                (
                    name.clone(),
                    DeclaredDataset {
                        columns: loader.columns().to_vec(),
                        loader_digest: loader.digest(),
                    },
                )
            })
            .collect(),
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Write `installed`'s release bundle into `out` (which must not exist yet):
/// the page, the renderer from `renderer`, the checked spec, `datasets.json`,
/// and the record that tells the factory client it is an interactive `hud`.
/// No dataset is written: rows live beside a version, never in one (§6.1).
pub fn assemble(installed: &Installed, renderer: &Path, out: &Path) -> Result<()> {
    for file in ["hud.js", "hud.css"] {
        if !renderer.join(file).is_file() {
            anyhow::bail!(
                "the dashboard renderer is not built at {} (missing {file}): \
                 `cd web && npm run build`, then copy dist/ to the web assets directory",
                renderer.display()
            );
        }
    }
    if out.exists() {
        anyhow::bail!("{} already exists", out.display());
    }
    let vendored = out.join(BUNDLE_RENDERER_DIR);
    crate::create_private_dir(&vendored)?;
    for file in ["hud.js", "hud.css"] {
        std::fs::copy(renderer.join(file), vendored.join(file))
            .with_context(|| format!("copying the renderer's {file}"))?;
    }
    let title = escape(installed.spec().title());
    std::fs::write(
        out.join("index.html"),
        format!(
            "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
             <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
             <title>{title}</title>\n\
             <link rel=\"stylesheet\" href=\"{BUNDLE_RENDERER_DIR}/hud.css\">\n</head>\n<body>\n\
             <div id=\"hud\"></div>\n\
             <script type=\"module\" src=\"{BUNDLE_RENDERER_DIR}/hud.js\"></script>\n\
             </body>\n</html>\n"
        ),
    )?;
    std::fs::write(
        out.join("hud.json"),
        serde_json::to_string_pretty(installed.spec())?,
    )?;
    std::fs::write(
        out.join(DATASETS_FILE),
        serde_json::to_string_pretty(&declared(installed))?,
    )?;
    // The factory client's render record: class and template (its
    // `render::RenderRecord`).
    std::fs::write(
        out.join("render.json"),
        serde_json::json!({ "class": "interactive", "template": "hud" }).to_string(),
    )?;
    Ok(())
}

/// What the factory client recorded as released: the version the alias
/// points at, and what that version declared.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: u32,
    pub declared: Declared,
}

/// The release recorded in the factory client's local store under
/// `bundles_root`, or `None` when the bundle has never been released (or has
/// been taken down).
pub fn release_at(bundles_root: &Path, bundle: &str) -> Result<Option<Release>> {
    let dir = bundles_root.join(bundle);
    let alias = match std::fs::read_to_string(dir.join("alias.json")) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {bundle}'s alias")),
    };
    let alias: serde_json::Value =
        serde_json::from_str(&alias).with_context(|| format!("parsing {bundle}'s alias"))?;
    let Some(version) = alias["version"]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
    else {
        return Ok(None);
    };
    let path = dir.join(version.to_string()).join(DATASETS_FILE);
    let declared: Declared = serde_json::from_str(
        &std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?,
    )
    .with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(Release { version, declared }))
}

/// What home should do with one refreshed dataset.
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// Push this body (`mecha_manifest::hud::DatasetPush` on the wire).
    Push(serde_json::Value),
    /// Already pushed under this release.
    Pushed,
    /// The release does not declare it: added after the release.
    Undeclared,
    /// Its loader is not the one released — a new publish, never a push.
    LoaderChanged,
}

/// Whether `dataset`, from `installed`'s loader `name`, may be pushed to
/// `release`, given the generation last pushed.
pub fn plan(
    installed: &Installed,
    name: &str,
    dataset: &Dataset,
    release: &Release,
    last_pushed: Option<u64>,
) -> Plan {
    let Some(declared) = release.declared.datasets.get(name) else {
        return Plan::Undeclared;
    };
    let Some(loader) = installed.loaders().get(name) else {
        return Plan::Undeclared;
    };
    if loader.digest() != declared.loader_digest
        || dataset.loader_digest != declared.loader_digest
        || loader.columns() != declared.columns.as_slice()
    {
        return Plan::LoaderChanged;
    }
    if last_pushed.is_some_and(|g| g >= dataset.generation) {
        return Plan::Pushed;
    }
    Plan::Push(serde_json::json!({
        "release": release.version,
        "generation": dataset.generation,
        "generated_at": dataset.generated_at.to_rfc3339(),
        "loader_digest": dataset.loader_digest,
        "columns": declared.columns,
        "rows": dataset.rows,
        "external": dataset.external,
    }))
}

/// Which bundle a board is published as — written when its publish is
/// staged, so the refresh knows to push.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Published {
    pub bundle: String,
}

pub fn published(board_dir: &Path) -> Result<Option<Published>> {
    match std::fs::read_to_string(board_dir.join(PUBLISHED_FILE)) {
        Ok(text) => Ok(Some(serde_json::from_str(&text)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn mark_published(board_dir: &Path, bundle: &str) -> Result<()> {
    write_atomic(
        &board_dir.join(PUBLISHED_FILE),
        &serde_json::to_string(&Published {
            bundle: bundle.to_string(),
        })?,
    )
}

/// The generation last pushed per dataset, under one release. A new release
/// starts it over.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Pushed {
    pub release: u32,
    pub generations: BTreeMap<String, u64>,
}

impl Pushed {
    /// What was pushed. None yet is an empty record; one that cannot be read
    /// is an error — never a fresh start, which would resend every dataset
    /// under a counter the box may already be past (review of #651).
    pub fn read(board_dir: &Path) -> Result<Pushed> {
        let path = board_dir.join(PUSHED_FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Pushed::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// The generation last pushed for `name` under `release`.
    pub fn last(&self, release: u32, name: &str) -> Option<u64> {
        (self.release == release)
            .then(|| self.generations.get(name).copied())
            .flatten()
    }

    pub fn record(&mut self, release: u32, name: &str, generation: u64) {
        if self.release != release {
            self.release = release;
            self.generations.clear();
        }
        self.generations.insert(name.to_string(), generation);
    }

    pub fn write(&self, board_dir: &Path) -> Result<()> {
        write_atomic(&board_dir.join(PUSHED_FILE), &serde_json::to_string(self)?)
    }
}

fn write_atomic(path: &Path, text: &str) -> Result<()> {
    // A name of its own per write: two overlapping refreshes (the timer and a
    // hand-run one) must not share a temp file and tear the record, which
    // `Pushed::read` then refuses (review of #651).
    let tmp = path.with_extension(format!(
        "json.{}.{}.tmp",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}
