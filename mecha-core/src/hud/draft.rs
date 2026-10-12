//! A board drafted in a conversation, and the proposals store it waits in
//! (`docs/LIVE-DASHBOARD-DESIGN.md` §5.3, the owner's ruling R18).
//!
//! ```text
//! ~/.mecha/hud/
//!   proposals/<id>/
//!     hud.json, loaders/*.toml      the draft, checked as an installed board is
//!     data/<name>.json              each loader's rows when it was proposed
//!     proposal.json                 when, by what kind of conversation, and its digest
//! ```
//!
//! **A draft is text handed to a tool, never files in a workspace.** A web
//! chat starts read-only, and drafting a dashboard must be one request in it
//! (R18), so `hud_preview` and `hud_propose` take the spec and the loaders as
//! their input. They are checked by the same code as an installed board —
//! written to a scratch directory under the store and read back through
//! [`Installed::load_dir`] — so a draft that previews clean is one that
//! installs.
//!
//! **Proposing stages; only the owner installs.** No model reaches
//! `boards/`: a proposal waits here until the owner accepts what was shown,
//! named by its [`Draft::digest`], so a revision that lands between the
//! owner's look and the owner's tap is refused rather than installed unseen.
//! A proposal under an installed board's id is a replacement, and accepting
//! it keeps the board's ledger and publish state.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::loader::{self, Column, Loader};
use super::runner::{run_sqlite, RunError, QUERY_TIMEOUT};
use super::source::{SourceKind, Sources};
use super::store::{Dataset, Outcome, Store, Which};
use super::{is_identifier, spec, Installed, Refusal, Refusals};
use crate::imagelib::Origin;

/// How many proposals may wait at once — a conversation proposing in a loop
/// fills the owner's queue to here and is then refused by name.
pub const MAX_PENDING: usize = 8;

/// The most datasets one draft may carry: the spec's own limit, enforced
/// before anything is written.
const MAX_LOADERS: usize = 16;

/// The rows of each dataset a preview hands back to the model.
pub const SAMPLE_ROWS: usize = 3;

/// A drafted board: its id, the spec's JSON text and each loader's TOML text,
/// by dataset name. Bounded and named before a byte is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    id: String,
    spec: String,
    loaders: BTreeMap<String, String>,
}

impl Draft {
    /// Every name proved an identifier and every text bounded, so writing the
    /// draft can neither escape its directory nor fill the disk.
    pub fn new(
        id: &str,
        spec: String,
        loaders: BTreeMap<String, String>,
    ) -> std::result::Result<Draft, Refusals> {
        let mut out = Vec::new();
        if !is_identifier(id) {
            out.push(Refusal::new(
                "id",
                format!("a board id matches [a-z][a-z0-9_]* (got {id:?})"),
            ));
        }
        if spec.len() > spec::MAX_BYTES {
            out.push(Refusal::new(
                "spec",
                format!(
                    "{} bytes, over the {}-byte limit",
                    spec.len(),
                    spec::MAX_BYTES
                ),
            ));
        }
        if loaders.is_empty() {
            out.push(Refusal::new("loaders", "a board needs at least one loader"));
        }
        if loaders.len() > MAX_LOADERS {
            out.push(Refusal::new(
                "loaders",
                format!("{} loaders, over the limit of {MAX_LOADERS}", loaders.len()),
            ));
        }
        for (name, text) in &loaders {
            if !is_identifier(name) {
                out.push(Refusal::new(
                    "loaders",
                    format!("a dataset name matches [a-z][a-z0-9_]* (got {name:?})"),
                ));
            } else if text.len() as u64 > loader::MAX_FILE_BYTES {
                out.push(Refusal::new(
                    format!("loaders/{name}"),
                    format!(
                        "{} bytes, over the {}-byte limit",
                        text.len(),
                        loader::MAX_FILE_BYTES
                    ),
                ));
            }
        }
        if out.is_empty() {
            Ok(Draft {
                id: id.to_string(),
                spec,
                loaders,
            })
        } else {
            Err(Refusals(out))
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn spec(&self) -> &str {
        &self.spec
    }

    pub fn loaders(&self) -> &BTreeMap<String, String> {
        &self.loaders
    }

    /// What the owner was shown, as one value: SHA-256 over the id, the spec
    /// and every loader, each length-prefixed so no two drafts share a
    /// digest by moving bytes across a boundary.
    pub fn digest(&self) -> String {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        let mut part = |label: &str, text: &str| {
            h.update(format!("{label}\n{}\n", text.len()));
            h.update(text);
        };
        part("id", &self.id);
        part("hud.json", &self.spec);
        for (name, text) in &self.loaders {
            part(&format!("loaders/{name}.toml"), text);
        }
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Write the draft as a board directory: `hud.json` and `loaders/`.
    fn write(&self, dir: &Path) -> Result<()> {
        crate::create_private_dir(&dir.join("loaders"))?;
        std::fs::write(dir.join("hud.json"), &self.spec)?;
        for (name, text) in &self.loaders {
            std::fs::write(dir.join("loaders").join(format!("{name}.toml")), text)?;
        }
        Ok(())
    }

    /// Read a board directory back as a draft — an installed board, to
    /// revise, or a waiting proposal. Bounded as `Installed::load_dir` is.
    fn read(dir: &Path, id: &str) -> Result<Draft> {
        let spec = super::read_bounded(&dir.join("hud.json"), spec::MAX_BYTES as u64)
            .with_context(|| format!("reading {}", dir.join("hud.json").display()))?;
        let mut loaders = BTreeMap::new();
        let loader_dir = dir.join("loaders");
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&loader_dir)
            .with_context(|| format!("reading {}", loader_dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "toml") && p.is_file())
            .collect();
        paths.sort();
        for path in paths {
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let text = super::read_bounded(&path, loader::MAX_FILE_BYTES)
                .with_context(|| format!("reading {}", path.display()))?;
            loaders.insert(name.to_string(), text);
        }
        Draft::new(id, spec, loaders).map_err(anyhow::Error::from)
    }
}

/// One loader run once against its source.
#[derive(Debug, Clone, PartialEq)]
pub enum Fetch {
    /// The query ran and its result has the declared shape.
    Rows {
        columns: Vec<Column>,
        rows: Vec<Vec<Value>>,
    },
    /// The loader's own fault: a refused statement, a shape that drifted,
    /// too many rows. What a refresh would record as refused.
    Refused(String),
    /// The environment's: an unregistered source, a file that will not open,
    /// a statement SQLite failed.
    Failed(String),
}

/// Run one loader against the registered sources: the confined runner, then
/// the shape check — the refresh's path exactly, writing nothing. The bool is
/// whether the source's rows are someone else's text ([`super::Source::external`]).
pub(crate) fn fetch(loader: &Loader, sources: &Sources) -> (Fetch, bool) {
    let Some(source) = sources.get(loader.source()) else {
        return (
            Fetch::Failed(format!(
                "source {:?} is not registered in sources.toml",
                loader.source()
            )),
            false,
        );
    };
    let external = source.external();
    let fetched = match &source.kind {
        SourceKind::Sqlite { path } => {
            run_sqlite(path, loader.query(), loader.max_rows(), QUERY_TIMEOUT)
        }
    };
    let fetched = match fetched {
        Ok(fetched) => fetched,
        Err(e) => {
            let reason = e.to_string();
            return (
                match e {
                    RunError::MultipleStatements
                    | RunError::NotAllowed(_)
                    | RunError::NotReadOnly
                    | RunError::Unrepresentable { .. }
                    | RunError::TooManyRows { .. }
                    | RunError::TooLarge { .. } => Fetch::Refused(reason),
                    RunError::Open(_) | RunError::Sql(_) | RunError::Timeout => {
                        Fetch::Failed(reason)
                    }
                },
                external,
            );
        }
    };
    match loader.shape(&fetched.columns, fetched.rows) {
        Ok(rows) => (
            Fetch::Rows {
                columns: loader.columns().to_vec(),
                rows,
            },
            external,
        ),
        Err(e) => (Fetch::Refused(e.to_string()), external),
    }
}

/// One dataset of a preview.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewDataset {
    pub name: String,
    pub source: String,
    pub fetch: Fetch,
    pub external: bool,
    /// The loader as checked, for a caller that keeps the rows.
    pub loader_digest: String,
}

/// A draft checked and run once.
#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    pub id: String,
    /// The spec's title, when the spec parsed.
    pub title: Option<String>,
    /// Every check the draft failed; empty when it passes them all.
    pub refusals: Vec<Refusal>,
    /// Each loader's run, when the checks passed.
    pub datasets: Vec<PreviewDataset>,
    /// Whether a board is installed under this id — a proposal is then its
    /// replacement.
    pub replaces: bool,
}

impl Preview {
    /// Passes every check, and every loader produced its rows.
    pub fn is_clean(&self) -> bool {
        self.refusals.is_empty()
            && self
                .datasets
                .iter()
                .all(|d| matches!(d.fetch, Fetch::Rows { .. }))
    }

    /// Whether any dataset carries someone else's text.
    pub fn external(&self) -> bool {
        self.datasets.iter().any(|d| d.external)
    }
}

/// Why a proposal waits, as `proposal.json` records it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposalMeta {
    pub proposed_at: DateTime<Utc>,
    /// The conversation's recorded taint when it proposed, never the model's
    /// say-so; an unreadable value loads untrusted.
    #[serde(default)]
    pub origin: Origin,
    /// The draft's digest when staged — what an accept must name.
    pub digest: String,
}

/// A waiting proposal, checked as a board.
#[derive(Debug, Clone)]
pub struct Proposal {
    pub draft: Draft,
    pub installed: Installed,
    pub meta: ProposalMeta,
    /// Whether accepting it replaces an installed board.
    pub replaces: bool,
}

/// What a proposal call did.
#[derive(Debug, Clone, PartialEq)]
pub enum Proposed {
    /// Staged; `revised` when it replaced a proposal already waiting.
    Staged { revised: bool, preview: Preview },
    /// Not staged: the draft does not preview clean.
    NotClean(Preview),
    /// Not staged: [`MAX_PENDING`] proposals already wait.
    TooMany,
}

/// What accepting a proposal did.
#[derive(Debug, Clone)]
pub struct Accepted {
    pub installed: Installed,
    pub replaced: bool,
    /// The refresh run straight after, so the board opens on fresh rows.
    pub refreshed: Vec<Outcome>,
}

/// A scratch directory under the store, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(store: &Store) -> Result<Scratch> {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = store.root().join(".scratch").join(format!(
            "{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        crate::create_private_dir(&dir)?;
        Ok(Scratch(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Store {
    fn proposals(&self) -> PathBuf {
        self.root().join("proposals")
    }

    /// A proposal's directory, the id proved an identifier at the join.
    fn proposal_dir(&self, id: &str) -> Result<PathBuf> {
        if !is_identifier(id) {
            anyhow::bail!("{id:?} is not a board id ([a-z][a-z0-9_]*)");
        }
        Ok(self.proposals().join(id))
    }

    /// Check a draft and run each of its loaders once, writing nothing a
    /// board or a refresh reads.
    pub fn preview(&self, draft: &Draft) -> Result<Preview> {
        let scratch = Scratch::new(self)?;
        let dir = scratch.0.join(draft.id());
        draft.write(&dir)?;
        let replaces = self.board_path(draft.id())?.is_dir();
        let installed = match self.validate(&dir, draft.id())? {
            Ok(installed) => installed,
            Err(r) => {
                // The title, when only the loaders are wrong, so the result
                // can name the board it is about.
                let title = spec::Spec::parse(draft.spec())
                    .ok()
                    .map(|s| s.title().to_string());
                return Ok(Preview {
                    id: draft.id().to_string(),
                    title,
                    refusals: r.0,
                    datasets: Vec::new(),
                    replaces,
                });
            }
        };
        let sources = match self.sources()? {
            Ok(sources) => sources,
            Err(r) => {
                return Ok(Preview {
                    id: draft.id().to_string(),
                    title: Some(installed.spec().title().to_string()),
                    refusals: super::prefixed("sources.toml", r.0).collect(),
                    datasets: Vec::new(),
                    replaces,
                })
            }
        };
        let datasets = installed
            .loaders()
            .iter()
            .map(|(name, loader)| {
                let (fetch, external) = fetch(loader, &sources);
                PreviewDataset {
                    name: name.clone(),
                    source: loader.source().to_string(),
                    fetch,
                    external,
                    loader_digest: loader.digest(),
                }
            })
            .collect();
        Ok(Preview {
            id: draft.id().to_string(),
            title: Some(installed.spec().title().to_string()),
            refusals: Vec::new(),
            datasets,
            replaces,
        })
    }

    /// An installed board's files, as a draft to revise.
    pub fn board_draft(&self, id: &str) -> Result<Option<Draft>> {
        let dir = self.board_path(id)?;
        if !dir.is_dir() {
            return Ok(None);
        }
        Draft::read(&dir, id).map(Some)
    }

    /// Ids with a proposal waiting, sorted. A directory whose name is not an
    /// identifier (an interrupted stage) is skipped.
    pub fn proposal_ids(&self) -> Result<Vec<String>> {
        let dir = self.proposals();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", dir.display())),
        };
        let mut ids: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .filter(|n| is_identifier(n))
            .collect();
        ids.sort();
        Ok(ids)
    }

    /// Stage a draft for the owner. It must preview clean — a board whose
    /// loaders fail is not one to put in front of the owner — and the same
    /// id again revises the proposal waiting under it.
    pub fn propose(&self, draft: &Draft, origin: Origin, now: DateTime<Utc>) -> Result<Proposed> {
        let preview = self.preview(draft)?;
        if !preview.is_clean() {
            return Ok(Proposed::NotClean(preview));
        }
        let waiting = self.proposal_ids()?;
        let revised = waiting.iter().any(|w| w == draft.id());
        if !revised && waiting.len() >= MAX_PENDING {
            return Ok(Proposed::TooMany);
        }

        crate::create_private_dir(&self.proposals())?;
        let staging =
            self.proposals()
                .join(format!(".stage-{}-{}", draft.id(), std::process::id()));
        if staging.exists() {
            std::fs::remove_dir_all(&staging)?;
        }
        draft.write(&staging)?;
        crate::create_private_dir(&staging.join("data"))?;
        for d in &preview.datasets {
            let Fetch::Rows { columns, rows } = &d.fetch else {
                continue;
            };
            let dataset = Dataset {
                name: d.name.clone(),
                generated_at: now,
                generation: 1,
                loader_digest: d.loader_digest.clone(),
                columns: columns.clone(),
                rows: rows.clone(),
                external: d.external,
            };
            std::fs::write(
                staging.join("data").join(format!("{}.json", d.name)),
                serde_json::to_vec(&dataset)?,
            )?;
        }
        let meta = ProposalMeta {
            proposed_at: now,
            origin,
            digest: draft.digest(),
        };
        std::fs::write(
            staging.join("proposal.json"),
            serde_json::to_vec_pretty(&meta)?,
        )?;
        let target = self.proposal_dir(draft.id())?;
        if target.exists() {
            std::fs::remove_dir_all(&target)
                .with_context(|| format!("replacing the proposal {:?}", draft.id()))?;
        }
        std::fs::rename(&staging, &target)
            .with_context(|| format!("staging the proposal {:?}", draft.id()))?;
        Ok(Proposed::Staged { revised, preview })
    }

    /// A waiting proposal. `Ok(None)` when there is none under `id`;
    /// `Ok(Some(Err))` when its files no longer pass the checks.
    pub fn proposal(&self, id: &str) -> Result<Option<std::result::Result<Proposal, Refusals>>> {
        let dir = self.proposal_dir(id)?;
        if !dir.is_dir() {
            return Ok(None);
        }
        let installed = match Installed::load_dir(&dir, id)? {
            Ok(installed) => installed,
            Err(r) => return Ok(Some(Err(r))),
        };
        let draft = Draft::read(&dir, id)?;
        // Unreadable metadata is not a reason to hide the proposal; it loads
        // untrusted with no digest, which no accept can name.
        let meta = std::fs::read(dir.join("proposal.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<ProposalMeta>(&b).ok())
            .unwrap_or(ProposalMeta {
                proposed_at: DateTime::<Utc>::MIN_UTC,
                origin: Origin::ModelUntrusted,
                digest: String::new(),
            });
        Ok(Some(Ok(Proposal {
            replaces: self.board_path(id)?.is_dir(),
            draft,
            installed,
            meta,
        })))
    }

    /// One dataset of a waiting proposal, as it was when proposed.
    pub fn proposal_dataset(&self, id: &str, name: &str) -> Result<Option<Dataset>> {
        if !is_identifier(name) {
            anyhow::bail!("{name:?} is not a dataset name ([a-z][a-z0-9_]*)");
        }
        let path = self
            .proposal_dir(id)?
            .join("data")
            .join(format!("{name}.json"));
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Some(
                serde_json::from_str(&text)
                    .with_context(|| format!("parsing {}", path.display()))?,
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// The owner's act: install the proposal waiting under `id` — or replace
    /// the board installed there — exactly as shown. `shown` is the digest
    /// the owner's surface displayed; a proposal revised since is refused,
    /// never installed unseen. The board is refreshed straight after.
    pub fn accept(
        &self,
        id: &str,
        shown: &str,
        now: DateTime<Utc>,
    ) -> Result<std::result::Result<Accepted, Refusals>> {
        let proposal = match self.proposal(id)? {
            None => {
                return Ok(Err(Refusals(vec![Refusal::new(
                    "",
                    format!("no proposal {id:?} is waiting"),
                )])))
            }
            Some(Err(r)) => return Ok(Err(r)),
            Some(Ok(p)) => p,
        };
        // Recomputed from the files, not read from proposal.json: what is
        // installed is what is on disk now.
        let digest = proposal.draft.digest();
        if shown.is_empty() || digest != shown || proposal.meta.digest != digest {
            return Ok(Err(Refusals(vec![Refusal::new(
                "",
                format!("the proposal {id:?} changed since it was shown — read it again"),
            )])));
        }
        let dir = self.proposal_dir(id)?;
        let installed = if proposal.replaces {
            self.replace(&dir, id, now)?
        } else {
            self.install(&dir, Some(id), now)?
        };
        let installed = match installed {
            Ok(installed) => installed,
            Err(r) => return Ok(Err(r)),
        };
        std::fs::remove_dir_all(&dir)
            .with_context(|| format!("clearing the accepted proposal {id:?}"))?;
        // Installed now, whatever the refresh says: a board that does not
        // load here is the refresh's finding (`mecha hud list`), never a
        // refusal of an accept that already landed (review of #659).
        let refreshed = self.refresh(id, now, Which::All)?.unwrap_or_default();
        Ok(Ok(Accepted {
            installed,
            replaced: proposal.replaces,
            refreshed,
        }))
    }

    /// Set a waiting proposal aside. `false` when none was waiting.
    pub fn reject(&self, id: &str) -> Result<bool> {
        let dir = self.proposal_dir(id)?;
        if !dir.is_dir() {
            return Ok(false);
        }
        std::fs::remove_dir_all(&dir).with_context(|| format!("rejecting {id:?}"))?;
        Ok(true)
    }

    /// Replace an installed board's spec and loaders with a draft's. What the
    /// board has accumulated beside them — the ledger, whose generations a
    /// publish's pushes count on, and the publish markers — comes across;
    /// the datasets do not, since the queries that made them are gone, and
    /// the caller refreshes.
    fn replace(
        &self,
        draft: &Path,
        id: &str,
        now: DateTime<Utc>,
    ) -> Result<std::result::Result<Installed, Refusals>> {
        let installed = match self.validate(draft, id)? {
            Ok(installed) => installed,
            Err(r) => return Ok(Err(r)),
        };
        let target = self.board_path(id)?;
        let parent = target.parent().context("a board has a parent")?;
        let _lock = super::store::lock(&target)?;
        let staging = parent.join(format!(".replace-{id}-{}", std::process::id()));
        if staging.exists() {
            std::fs::remove_dir_all(&staging)?;
        }
        crate::create_private_dir(&staging.join("loaders"))?;
        // The lock a refresh takes is a file in the board's directory, so
        // once `staging` is renamed into place a refresh would lock a fresh
        // file there: lock the one that travels with it too, and hold both
        // until the replacement is recorded (review of #659).
        let _new_lock = super::store::lock(&staging)?;
        std::fs::copy(draft.join("hud.json"), staging.join("hud.json"))?;
        for name in installed.loaders().keys() {
            let file = format!("{name}.toml");
            std::fs::copy(
                draft.join("loaders").join(&file),
                staging.join("loaders").join(&file),
            )?;
        }
        // Every plain file beside the spec but the spec and the lock: the
        // ledger and the publish markers today, whatever a later step adds.
        for entry in std::fs::read_dir(&target)? {
            let entry = entry?;
            let name = entry.file_name();
            let keep = entry.file_type()?.is_file() && name != "hud.json" && name != ".lock";
            if keep {
                std::fs::copy(entry.path(), staging.join(&name))?;
            }
        }
        let old = parent.join(format!(".old-{id}-{}", std::process::id()));
        if old.exists() {
            std::fs::remove_dir_all(&old)?;
        }
        std::fs::rename(&target, &old).with_context(|| format!("setting {id:?} aside"))?;
        if let Err(e) = std::fs::rename(&staging, &target) {
            // Put the old board back rather than leave none.
            let _ = std::fs::rename(&old, &target);
            return Err(e).with_context(|| format!("replacing board {id:?}"));
        }
        let _ = std::fs::remove_dir_all(&old);
        self.record_installed(id, now)?;
        Ok(Ok(installed))
    }
}

#[cfg(test)]
#[path = "draft_tests.rs"]
mod tests;
