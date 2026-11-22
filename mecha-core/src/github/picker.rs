//! The picker (`docs/GITHUB-DASHBOARD-DESIGN.md` step 6, ruling G3): the
//! owner chooses which repositories the collector watches, and corrects the
//! package mapping, from a page in the web app. This module is the page's
//! two halves that are not HTTP — the read it draws from, and the edit it
//! asks for.
//!
//! **The read never calls GitHub** (§2). It lists from the inventory the
//! collector last wrote, beside the two owner files as they read now, so a
//! slow API is a stale list with its age shown, never a page that hangs. A
//! store or a file that cannot be read is said, never drawn as an empty list:
//! "nothing is selected" and "the selection could not be read" are opposite
//! findings.
//!
//! **The write is the owner's act and the files stay the owner's.** Each edit
//! is set in place with `toml_edit`, so every comment and line the owner wrote
//! by hand survives, and a table the edit removes hands its comments on. It is
//! refused when the file changed since the page read it (a digest the page
//! sends back, compared under one lock, so a hand edit is never written over
//! silently), when the file as it stands cannot be used (it is the owner's to
//! fix by hand, never the page's to replace), and when the result is not
//! something the collector's own reader accepts ([`super::parse_selection`],
//! [`usage::parse_packages`]) — so the page can write nothing the collector
//! would refuse. Nothing else writes these files: there is no tool and no
//! agent path, and the collector only reads them.

use super::usage::{self, canonical_name, Registry};
use super::{has_column, parse_selection, split_full_name, valid_owner, Include, Repo, Selection};
use crate::tomlform::comments_of;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Key, Table, Value};

/// The most changes one save may carry. The page sends one per click; a
/// select-all is one change, however many repositories it covers.
pub const MAX_CHANGES: usize = 500;
/// The most either file may grow to through the page. Four hundred
/// `[[repo]]` exceptions are about 20 KiB.
pub const MAX_FILE_BYTES: usize = 256 * 1024;

/// The header a file the page creates begins with, so a reader of it knows
/// where it came from and that a hand edit is safe.
const SELECTION_HEADER: &str = "\
# Which repositories `mecha github collect` watches
# (docs/GITHUB-DASHBOARD-DESIGN.md §2). Written by the web app's picker
# (Settings → GitHub); hand edits are kept, and a page that read an older
# copy of this file is refused rather than written over.
";
const PACKAGES_HEADER: &str = "\
# The owner's corrections to the GitHub board's package mapping
# (docs/GITHUB-DASHBOARD-DESIGN.md §7.2). Written by the web app's picker
# (Settings → GitHub); hand edits are kept, and a page that read an older
# copy of this file is refused rather than written over.
";

/// The collector's schedule as shipped: the timer's `OnCalendar` line. What
/// runs is the installed unit; the page says so beside it.
const TIMER: &str = include_str!("../../../scripts/mecha-github-collect.timer");

/// The file's sha256, hex: what the page sends back as `base`.
pub fn digest(text: &str) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `daily at 06:40 America/New_York` from the shipped timer, or its
/// `OnCalendar` as written when it is not a daily one.
pub fn schedule() -> Option<String> {
    let cal = TIMER
        .lines()
        .find_map(|l| l.trim().strip_prefix("OnCalendar="))?
        .trim();
    Some(match cal.strip_prefix("*-*-* ") {
        Some(rest) => format!("daily at {rest}"),
        None => cal.to_string(),
    })
}

// ---- the read ----

/// One owner file as it reads now.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileView {
    pub path: PathBuf,
    pub state: FileState,
    /// Why it cannot be used, when it cannot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The digest of its bytes, which a save sends back; `None` when it is
    /// absent (a save then sends `null`) or could not be read as text.
    pub digest: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileState {
    Absent,
    Read,
    /// There, and cannot be used: the collector selects nothing from it (or
    /// keeps the mapping as it was), and the page edits nothing in it.
    Invalid,
}

/// The collector's store, as the page reads it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum StoreView {
    /// No collection has run on this machine.
    Absent { path: PathBuf },
    /// It is there and could not be read; nothing below is from it.
    Unreadable { path: PathBuf, error: String },
    Read {
        path: PathBuf,
        /// When the inventory was last listed (its rows' `seen_at`) — older
        /// than the last run when that run's listing failed.
        listed_at: Option<String>,
        /// The last run that wrote its `finished` row.
        last_collect: Option<LastCollect>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LastCollect {
    pub started: String,
    pub finished: String,
}

/// An owner's line in `repos.toml`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OwnerLine {
    /// `all` or `listed`.
    pub include: &'static str,
    pub forks: bool,
    pub archived: bool,
}

/// One owner: its line, if the file has one, and every repository of it the
/// inventory holds.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OwnerView {
    pub login: String,
    pub line: Option<OwnerLine>,
    pub repos: Vec<RepoView>,
    /// `[[repo]]` lines naming a repository of this owner the inventory does
    /// not hold (not collected yet, or gone), with what each says.
    pub unseen: Vec<RepoLine>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RepoLine {
    pub name: String,
    pub include: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RepoView {
    pub full_name: String,
    pub name: String,
    pub fork: bool,
    pub archived: bool,
    pub private: bool,
    pub pushed_at: Option<String>,
    /// In the login's own listing, rather than fetched because a line named
    /// it.
    pub listed: bool,
    /// Whether the last listing's run watched it (`inventory.selected`).
    pub collected: bool,
    /// Whether the file as it reads now selects it — what the next collect
    /// does. `None` when the file cannot be used (the collector then selects
    /// nothing, which the page says once, above the list).
    pub included: Option<bool>,
    /// What its owner's line alone gives it, before any `[[repo]]` line.
    pub by_owner: Option<bool>,
    /// Its own `[[repo]]` line, if the file has one.
    pub line: Option<bool>,
    pub packages: Vec<PackageView>,
}

/// One repository's mapping for one registry.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PackageView {
    pub registry: Registry,
    pub state: PackageState,
    /// The name counted: the owner's, else the detected one.
    pub name: Option<String>,
    /// What the manifests said, whatever the owner's line says.
    pub detected: Option<String>,
    pub manifest: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageState {
    /// `packages.toml` names it.
    Owner,
    /// `packages.toml` says it publishes nothing there.
    OwnerNone,
    /// A manifest named it.
    Detected,
    /// Its manifests were read and none named a package for this registry.
    NoneFound,
    /// Its manifests have not been read yet.
    NotScanned,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct View {
    pub selection: FileView,
    pub packages: FileView,
    pub store: StoreView,
    pub owners: Vec<OwnerView>,
    pub registries: Vec<Registry>,
    /// The collector's schedule as shipped ([`schedule`]).
    pub schedule: Option<String>,
}

/// A file read once: its state, and its text when it is text.
fn read_file(
    path: &Path,
    check: impl Fn(&str) -> Result<(), String>,
) -> (FileView, Option<String>) {
    let mut view = FileView {
        path: path.to_path_buf(),
        state: FileState::Absent,
        error: None,
        digest: None,
    };
    match std::fs::read_to_string(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (view, None),
        Err(e) => {
            view.state = FileState::Invalid;
            view.error = Some(format!("{}: {e}", path.display()));
            (view, None)
        }
        Ok(text) => {
            view.digest = Some(digest(&text));
            match check(&text) {
                Ok(()) => view.state = FileState::Read,
                Err(e) => {
                    view.state = FileState::Invalid;
                    view.error = Some(format!("{}: {e}", path.display()));
                }
            }
            (view, Some(text))
        }
    }
}

/// What the store holds for the page.
struct Store {
    inventory: Vec<(Repo, bool)>,
    listed_at: Option<String>,
    last_collect: Option<LastCollect>,
    /// Lower-cased repository names whose manifests were read.
    scanned: BTreeSet<String>,
    /// `(repo lower-cased, registry)` → `(name, manifest)`.
    detected: BTreeMap<(String, Registry), (String, String)>,
}

fn table_exists(conn: &Connection, name: &str) -> rusqlite::Result<bool> {
    conn.prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1")?
        .exists([name])
}

fn read_store(db: &Path) -> Result<Store, String> {
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    let q = |e: rusqlite::Error| e.to_string();
    let mut store = Store {
        inventory: Vec::new(),
        listed_at: None,
        last_collect: None,
        scanned: BTreeSet::new(),
        detected: BTreeMap::new(),
    };
    // A store from before a table existed has none of its rows: an empty
    // inventory is "never listed" (the store says when it was), and no
    // package tables is "not scanned yet" — both true, neither unreadable.
    if table_exists(&conn, "inventory").map_err(q)? {
        let push = if has_column(&conn, "inventory", "push").map_err(|e| e.to_string())? {
            "push"
        } else {
            "NULL"
        };
        let mut stmt = conn
            .prepare(&format!(
                "SELECT repo, owner, name, fork, archived, private, default_branch, pushed_at, listed, selected, {push}, seen_at
                 FROM inventory ORDER BY lower(owner), lower(name)"
            ))
            .map_err(q)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    Repo {
                        full_name: r.get(0)?,
                        owner: r.get(1)?,
                        name: r.get(2)?,
                        fork: r.get(3)?,
                        archived: r.get(4)?,
                        private: r.get(5)?,
                        default_branch: r.get(6)?,
                        pushed_at: r.get(7)?,
                        listed: r.get(8)?,
                        push: r.get(10)?,
                    },
                    r.get::<_, bool>(9)?,
                    r.get::<_, String>(11)?,
                ))
            })
            .map_err(q)?;
        for row in rows {
            let (repo, selected, seen) = row.map_err(q)?;
            if store.listed_at.as_ref().is_none_or(|s| seen > *s) {
                store.listed_at = Some(seen);
            }
            store.inventory.push((repo, selected));
        }
    }
    if table_exists(&conn, "collect_log").map_err(q)? {
        store.last_collect = conn
            .query_row(
                "SELECT run, at FROM collect_log WHERE kind = 'finished' ORDER BY run DESC, id DESC LIMIT 1",
                [],
                |r| {
                    Ok(LastCollect {
                        started: r.get(0)?,
                        finished: r.get(1)?,
                    })
                },
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                e => Err(e),
            })
            .map_err(q)?;
    }
    if table_exists(&conn, "package_scans").map_err(q)? {
        let mut stmt = conn.prepare("SELECT repo FROM package_scans").map_err(q)?;
        for r in stmt.query_map([], |r| r.get::<_, String>(0)).map_err(q)? {
            store.scanned.insert(r.map_err(q)?.to_ascii_lowercase());
        }
    }
    if table_exists(&conn, "package_detected").map_err(q)? {
        let mut stmt = conn
            .prepare("SELECT repo, registry, name, manifest FROM package_detected")
            .map_err(q)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(q)?;
        for row in rows {
            let (repo, registry, name, manifest) = row.map_err(q)?;
            // A registry this build does not know is a newer build's row:
            // not one the page can offer a correction for.
            if let Some(reg) = Registry::parse(&registry) {
                store
                    .detected
                    .insert((repo.to_ascii_lowercase(), reg), (name, manifest));
            }
        }
    }
    Ok(store)
}

/// Everything the page draws, read now.
pub fn view(home: &Path) -> View {
    let (selection_file, selection_text) = read_file(&super::selection_path(home), |t| {
        parse_selection(t).map(|_| ())
    });
    // No file selects nothing, which is known; a file that cannot be used
    // selects nothing too, but what the owner meant by it is not known, so
    // the page offers no edit until it is fixed by hand.
    let selection: Option<Selection> = match selection_file.state {
        FileState::Absent => Some(Selection::default()),
        FileState::Read => selection_text
            .as_deref()
            .and_then(|t| parse_selection(t).ok()),
        FileState::Invalid => None,
    };
    let (packages_file, packages_text) = read_file(&usage::packages_path(home), |t| {
        usage::parse_packages(t).map(|_| ())
    });
    // `(repo lower-cased, registry)` → the owner's line: `Some(name)` or
    // `None` for `none = true`. Empty when the file is absent; unknown —
    // and the page says so once — when it cannot be used.
    let overrides: BTreeMap<(String, Registry), Option<String>> = match packages_file.state {
        FileState::Read => packages_text
            .as_deref()
            .and_then(|t| usage::parse_packages(t).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|r| {
                let name = if r.none { None } else { r.name };
                ((r.repo.to_ascii_lowercase(), r.registry), name)
            })
            .collect(),
        _ => BTreeMap::new(),
    };

    let db = super::db_path(home);
    let (store_view, store) = match std::fs::metadata(&db) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            (StoreView::Absent { path: db.clone() }, None)
        }
        Err(e) => (
            StoreView::Unreadable {
                path: db.clone(),
                error: e.to_string(),
            },
            None,
        ),
        Ok(_) => match read_store(&db) {
            Ok(s) => (
                StoreView::Read {
                    path: db.clone(),
                    listed_at: s.listed_at.clone(),
                    last_collect: s.last_collect.clone(),
                },
                Some(s),
            ),
            Err(error) => (
                StoreView::Unreadable {
                    path: db.clone(),
                    error,
                },
                None,
            ),
        },
    };

    // Owners: each the inventory holds, and each the file names.
    let mut owners: BTreeMap<String, OwnerView> = BTreeMap::new();
    let owner_line = |login: &str| -> Option<OwnerLine> {
        selection
            .as_ref()?
            .owners
            .iter()
            .find(|o| o.login.eq_ignore_ascii_case(login))
            .map(|o| OwnerLine {
                include: match o.include {
                    Include::All => "all",
                    Include::Listed => "listed",
                },
                forks: o.forks,
                archived: o.archived,
            })
    };
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (repo, collected) in store
        .as_ref()
        .map(|s| s.inventory.as_slice())
        .unwrap_or(&[])
    {
        seen.insert(repo.full_name.to_ascii_lowercase());
        let key = repo.owner.to_ascii_lowercase();
        let entry = owners.entry(key).or_insert_with(|| OwnerView {
            login: repo.owner.clone(),
            line: owner_line(&repo.owner),
            repos: Vec::new(),
            unseen: Vec::new(),
        });
        let lower = repo.full_name.to_ascii_lowercase();
        let packages = Registry::ALL
            .into_iter()
            .map(|reg| {
                let found = store
                    .as_ref()
                    .and_then(|s| s.detected.get(&(lower.clone(), reg)));
                let detected = found.map(|(n, _)| n.clone());
                let manifest = found.map(|(_, m)| m.clone());
                let scanned = store.as_ref().is_some_and(|s| s.scanned.contains(&lower));
                match overrides.get(&(lower.clone(), reg)) {
                    Some(Some(name)) => PackageView {
                        registry: reg,
                        state: PackageState::Owner,
                        name: Some(name.clone()),
                        detected,
                        manifest,
                    },
                    Some(None) => PackageView {
                        registry: reg,
                        state: PackageState::OwnerNone,
                        name: None,
                        detected,
                        manifest,
                    },
                    None => PackageView {
                        registry: reg,
                        state: if detected.is_some() {
                            PackageState::Detected
                        } else if scanned {
                            PackageState::NoneFound
                        } else {
                            PackageState::NotScanned
                        },
                        name: detected.clone(),
                        detected,
                        manifest,
                    },
                }
            })
            .collect();
        entry.repos.push(RepoView {
            full_name: repo.full_name.clone(),
            name: repo.name.clone(),
            fork: repo.fork,
            archived: repo.archived,
            private: repo.private,
            pushed_at: repo.pushed_at.clone(),
            listed: repo.listed,
            collected: *collected,
            included: selection.as_ref().map(|s| s.selects(repo)),
            by_owner: selection.as_ref().map(|s| s.by_owner(repo)),
            line: selection.as_ref().and_then(|s| s.line_for(&repo.full_name)),
            packages,
        });
    }
    if let Some(s) = &selection {
        for o in &s.owners {
            owners
                .entry(o.login.to_ascii_lowercase())
                .or_insert_with(|| OwnerView {
                    login: o.login.clone(),
                    line: owner_line(&o.login),
                    repos: Vec::new(),
                    unseen: Vec::new(),
                });
        }
        for r in &s.repos {
            if seen.contains(&r.name.to_ascii_lowercase()) {
                continue;
            }
            let Some((owner, _)) = split_full_name(&r.name) else {
                continue;
            };
            owners
                .entry(owner.to_ascii_lowercase())
                .or_insert_with(|| OwnerView {
                    login: owner.to_string(),
                    line: owner_line(owner),
                    repos: Vec::new(),
                    unseen: Vec::new(),
                })
                .unseen
                .push(RepoLine {
                    name: r.name.clone(),
                    include: r.include,
                });
        }
    }

    View {
        selection: selection_file,
        packages: packages_file,
        store: store_view,
        owners: owners.into_values().collect(),
        registries: Registry::ALL.to_vec(),
        schedule: schedule(),
    }
}

// ---- the write ----

/// One edit to `repos.toml`, as the page sends it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum SelectionChange {
    /// Every repository of the owner, less forks and archived ones unless
    /// its line takes them: `include = "all"`, and the owner's `[[repo]]`
    /// lines that exclude one are removed — or select-all would not select
    /// all. A line that includes a fork or an archived one stays.
    SelectAll { login: String },
    /// None of the owner's repositories: `include = "listed"`, and the
    /// owner's `[[repo]]` lines are removed, either way.
    SelectNone { login: String },
    /// The owner's line's filters (G4): whether its forks and its archived
    /// repositories are taken. The line is written if the file has none.
    Filters {
        login: String,
        forks: bool,
        archived: bool,
    },
    /// One repository's own line: `true` or `false`, or `null` to remove it
    /// and leave the repository to its owner's line.
    Repo { name: String, include: Option<bool> },
}

/// What a `[[package]]` line says for one `(repo, registry)`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PackageTo {
    /// Count this name.
    Name(String),
    /// It publishes nothing to this registry: ignore what is detected.
    None,
    /// No line: what the manifests say stands.
    Detected,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageChange {
    pub repo: String,
    pub registry: Registry,
    pub to: PackageTo,
}

/// Why a save wrote nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The file is not what the page read: a hand edit, or another tab.
    Stale,
    /// The file as it stands cannot be used or edited in place; it is the
    /// owner's to fix by hand, never the page's to replace.
    Unusable(String),
    /// A change, or the file it would make, is not one the collector's
    /// reader accepts.
    Invalid(String),
    /// The request itself is malformed: no change, or too many.
    BadRequest(String),
    Io(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Stale => write!(
                f,
                "the file changed since this page read it — reload to see the change, then choose again"
            ),
            Refusal::Unusable(why) => write!(
                f,
                "{why} — fix the file by hand; the page edits only a file the collector can read"
            ),
            Refusal::Invalid(why) | Refusal::BadRequest(why) | Refusal::Io(why) => {
                write!(f, "{why}")
            }
        }
    }
}

/// One save at a time, in this process: the digest comparison and the
/// rename happen under it, so two tabs cannot both pass the check against
/// the same file and land one over the other.
static WRITE: Mutex<()> = Mutex::new(());

/// Read `path`, check it is the file the page read (`base`), edit it, check
/// the result with the collector's reader, and put it in place atomically.
/// Returns the new digest, or `None` when there is still no file: an edit
/// of an absent file that leaves nothing to say (a correction cleared that
/// was never written) creates none.
pub(super) fn save(
    path: &Path,
    base: Option<&str>,
    header: &str,
    usable: impl Fn(&str) -> Result<(), String>,
    edit: impl FnOnce(&mut DocumentMut) -> Result<(), Refusal>,
) -> Result<Option<String>, Refusal> {
    let _held = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let disk = match std::fs::read_to_string(path) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            return Err(Refusal::Unusable(format!(
                "{} cannot be read: {e}",
                path.display()
            )))
        }
    };
    match (base, &disk) {
        (None, None) => {}
        (Some(b), Some(t)) if digest(t) == b => {}
        _ => return Err(Refusal::Stale),
    }
    if let Some(t) = &disk {
        usable(t)
            .map_err(|e| Refusal::Unusable(format!("{} cannot be used: {e}", path.display())))?;
    }
    let start = disk.as_deref().unwrap_or(header);
    let mut doc: DocumentMut = start
        .parse()
        .map_err(|e| Refusal::Unusable(format!("{} is not TOML: {e}", path.display())))?;
    let had_tables = has_tables(&doc);
    edit(&mut doc)?;
    if !had_tables {
        lift_trailing(&mut doc);
    }
    let text = doc.to_string();
    if text.len() > MAX_FILE_BYTES {
        return Err(Refusal::Invalid(format!(
            "the file would be {} bytes; the page writes at most {MAX_FILE_BYTES}",
            text.len()
        )));
    }
    // The collector's own reader, on the very bytes that will land.
    usable(&text)
        .map_err(|e| Refusal::Invalid(format!("the collector would refuse the result: {e}")))?;
    if disk.as_deref() == Some(text.as_str()) {
        return Ok(Some(digest(&text)));
    }
    if disk.is_none() && !has_tables(&doc) {
        return Ok(None);
    }
    let dir = path
        .parent()
        .ok_or_else(|| Refusal::Io(format!("{} has no directory", path.display())))?;
    crate::create_private_dir(dir)
        .map_err(|e| Refusal::Io(format!("creating {}: {e}", dir.display())))?;
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        // An owner file keeps its mode; a new one is the owner's alone.
        Some(match &disk {
            Some(_) => std::fs::metadata(path)
                .map(|m| m.permissions().mode() & 0o777)
                .unwrap_or(0o600),
            None => 0o600,
        })
    };
    #[cfg(not(unix))]
    let mode = None;
    crate::imagelib::write_atomic_mode(path, text.as_bytes(), mode)
        .map_err(|e| Refusal::Io(format!("{e:#}")))?;
    Ok(Some(digest(&text)))
}

fn check_count(n: usize) -> Result<(), Refusal> {
    if n == 0 {
        return Err(Refusal::BadRequest(
            "a save carries at least one change".into(),
        ));
    }
    if n > MAX_CHANGES {
        return Err(Refusal::BadRequest(format!(
            "a save carries at most {MAX_CHANGES} changes"
        )));
    }
    Ok(())
}

/// Apply `changes` to `<home>/github/repos.toml`, in order, if it is still
/// the file whose digest is `base` (`None`: the page read no file).
pub fn save_selection(
    home: &Path,
    base: Option<&str>,
    changes: &[SelectionChange],
) -> Result<Option<String>, Refusal> {
    check_count(changes.len())?;
    save(
        &super::selection_path(home),
        base,
        SELECTION_HEADER,
        |t| parse_selection(t).map(|_| ()),
        |doc| {
            for c in changes {
                apply_selection(doc, c)?;
            }
            Ok(())
        },
    )
}

/// Apply `changes` to `<home>/github/packages.toml`, as [`save_selection`].
pub fn save_packages(
    home: &Path,
    base: Option<&str>,
    changes: &[PackageChange],
) -> Result<Option<String>, Refusal> {
    check_count(changes.len())?;
    save(
        &usage::packages_path(home),
        base,
        PACKAGES_HEADER,
        |t| usage::parse_packages(t).map(|_| ()),
        |doc| {
            for c in changes {
                apply_package(doc, c)?;
            }
            Ok(())
        },
    )
}

fn check_login(login: &str) -> Result<(), Refusal> {
    if valid_owner(login) {
        Ok(())
    } else {
        Err(Refusal::Invalid(format!("{login:?} is not a GitHub login")))
    }
}

fn check_repo(name: &str) -> Result<(), Refusal> {
    match split_full_name(name) {
        Some(_) => Ok(()),
        None => Err(Refusal::Invalid(format!(
            "{name:?} is not owner/name in GitHub's grammar"
        ))),
    }
}

fn apply_selection(doc: &mut DocumentMut, c: &SelectionChange) -> Result<(), Refusal> {
    let of_owner = |login: &str| {
        let prefix = format!("{}/", login.to_ascii_lowercase());
        move |t: &Table| {
            t.get("name")
                .and_then(Item::as_str)
                .is_some_and(|n| n.to_ascii_lowercase().starts_with(&prefix))
        }
    };
    match c {
        SelectionChange::SelectAll { login } => {
            check_login(login)?;
            owner_line(doc, login, Some("all"), None)?;
            let of = of_owner(login);
            remove_tables(doc, "repo", |t| {
                of(t) && t.get("include").and_then(Item::as_bool) == Some(false)
            })?;
        }
        SelectionChange::SelectNone { login } => {
            check_login(login)?;
            owner_line(doc, login, Some("listed"), None)?;
            remove_tables(doc, "repo", of_owner(login))?;
        }
        SelectionChange::Filters {
            login,
            forks,
            archived,
        } => {
            check_login(login)?;
            owner_line(doc, login, None, Some((*forks, *archived)))?;
        }
        SelectionChange::Repo { name, include } => {
            check_repo(name)?;
            let is = |t: &Table| {
                t.get("name")
                    .and_then(Item::as_str)
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            };
            match include {
                None => remove_tables(doc, "repo", is)?,
                Some(b) => match find_table(doc, "repo", is)? {
                    Some(t) => set_value(t, "include", Value::from(*b)),
                    None => push_table(
                        doc,
                        "repo",
                        vec![
                            ("name", Value::from(name.as_str())),
                            ("include", Value::from(*b)),
                        ],
                    )?,
                },
            }
        }
    }
    Ok(())
}

/// Set the owner's line: `include` when given, `forks` and `archived` when
/// given — and written out as `false` where the line does not say, so the
/// file states G4 rather than leaving it to a default. A new line takes
/// `include = "listed"` unless told otherwise: a filter alone selects
/// nothing new.
fn owner_line(
    doc: &mut DocumentMut,
    login: &str,
    include: Option<&str>,
    filters: Option<(bool, bool)>,
) -> Result<(), Refusal> {
    let is = |t: &Table| {
        t.get("login")
            .and_then(Item::as_str)
            .is_some_and(|l| l.eq_ignore_ascii_case(login))
    };
    match find_table(doc, "owner", is)? {
        Some(t) => {
            if let Some(i) = include {
                set_value(t, "include", Value::from(i));
            }
            match filters {
                Some((forks, archived)) => {
                    set_value(t, "forks", Value::from(forks));
                    set_value(t, "archived", Value::from(archived));
                }
                None => {
                    for k in ["forks", "archived"] {
                        if !t.contains_key(k) {
                            set_value(t, k, Value::from(false));
                        }
                    }
                }
            }
        }
        None => {
            let (forks, archived) = filters.unwrap_or((false, false));
            push_table(
                doc,
                "owner",
                vec![
                    ("login", Value::from(login)),
                    ("include", Value::from(include.unwrap_or("listed"))),
                    ("forks", Value::from(forks)),
                    ("archived", Value::from(archived)),
                ],
            )?;
        }
    }
    Ok(())
}

fn apply_package(doc: &mut DocumentMut, c: &PackageChange) -> Result<(), Refusal> {
    check_repo(&c.repo)?;
    let reg = c.registry.as_str();
    let is = |t: &Table| {
        t.get("repo")
            .and_then(Item::as_str)
            .is_some_and(|r| r.eq_ignore_ascii_case(&c.repo))
            && t.get("registry").and_then(Item::as_str) == Some(reg)
    };
    match &c.to {
        PackageTo::Detected => remove_tables(doc, "package", is)?,
        PackageTo::Name(n) => {
            let name = canonical_name(c.registry, n)
                .ok_or_else(|| Refusal::Invalid(format!("{n:?} is not a {reg} package name")))?;
            match find_table(doc, "package", is)? {
                Some(t) => replace_key(t, "none", "name", Value::from(name.as_str())),
                None => push_table(
                    doc,
                    "package",
                    vec![
                        ("repo", Value::from(c.repo.as_str())),
                        ("registry", Value::from(reg)),
                        ("name", Value::from(name.as_str())),
                    ],
                )?,
            }
        }
        PackageTo::None => match find_table(doc, "package", is)? {
            Some(t) => replace_key(t, "name", "none", Value::from(true)),
            None => push_table(
                doc,
                "package",
                vec![
                    ("repo", Value::from(c.repo.as_str())),
                    ("registry", Value::from(reg)),
                    ("none", Value::from(true)),
                ],
            )?,
        },
    }
    Ok(())
}

// ---- editing in place ----

/// The list of tables at `key`, made if absent. A list written some other
/// way (`owner = [{ … }]`) is the owner's to keep: refused, never rewritten.
fn tables<'a>(doc: &'a mut DocumentMut, key: &str) -> Result<&'a mut ArrayOfTables, Refusal> {
    let root = doc.as_table_mut();
    if !root.contains_key(key) {
        root.insert(key, Item::ArrayOfTables(ArrayOfTables::new()));
    }
    root.get_mut(key)
        .and_then(Item::as_array_of_tables_mut)
        .ok_or_else(|| {
            Refusal::Unusable(format!(
                "`{key}` is not written as [[{key}]] tables, which is the only form the page edits"
            ))
        })
}

fn find_table<'a>(
    doc: &'a mut DocumentMut,
    key: &str,
    is: impl Fn(&Table) -> bool,
) -> Result<Option<&'a mut Table>, Refusal> {
    Ok(tables(doc, key)?.iter_mut().find(|t| is(t)))
}

fn has_tables(doc: &DocumentMut) -> bool {
    doc.as_table().iter().any(|(_, item)| match item {
        Item::ArrayOfTables(a) => !a.is_empty(),
        Item::Table(_) => true,
        _ => false,
    })
}

/// Every positioned table in the document, as `(list, index, position)`.
fn positions(doc: &DocumentMut) -> Vec<(String, usize, isize)> {
    let mut out = Vec::new();
    for (k, item) in doc.as_table().iter() {
        if let Item::ArrayOfTables(a) = item {
            for (i, t) in a.iter().enumerate() {
                if let Some(p) = t.position() {
                    out.push((k.to_string(), i, p));
                }
            }
        }
    }
    out
}

/// A new table at the end of the file, after every table already there.
fn push_table(
    doc: &mut DocumentMut,
    key: &str,
    entries: Vec<(&str, Value)>,
) -> Result<(), Refusal> {
    let next = positions(doc).iter().map(|(_, _, p)| *p).max().unwrap_or(0) + 1;
    let mut t = Table::new();
    for (k, v) in entries {
        t.insert(k, Item::Value(v));
    }
    t.set_position(Some(next));
    t.decor_mut().set_prefix("\n");
    tables(doc, key)?.push(t);
    Ok(())
}

/// Remove each table of `key` that `is`, handing its comments to the table
/// after it in the file (the file's header rides on its first table), or to
/// the end of the file when it was the last.
fn remove_tables(
    doc: &mut DocumentMut,
    key: &str,
    is: impl Fn(&Table) -> bool,
) -> Result<(), Refusal> {
    loop {
        let list = tables(doc, key)?;
        let Some(i) = list.iter().position(&is) else {
            return Ok(());
        };
        let gone = list.remove(i);
        let carried = comments_of(&gone);
        if carried.is_empty() {
            continue;
        }
        let at = gone.position();
        let all = positions(doc);
        let next = all
            .iter()
            .filter(|(_, _, p)| at.is_none_or(|a| *p > a))
            .min_by_key(|(_, _, p)| *p)
            .cloned();
        match next {
            Some((k, j, p)) => {
                // Now the file's first table: no blank line above the
                // comments it takes over (the file's header, usually).
                let lead = if all.iter().any(|(_, _, q)| *q < p) {
                    "\n"
                } else {
                    ""
                };
                let t = doc
                    .as_table_mut()
                    .get_mut(&k)
                    .and_then(Item::as_array_of_tables_mut)
                    .and_then(|a| a.get_mut(j))
                    .expect("a position just listed");
                let own = t
                    .decor()
                    .prefix()
                    .and_then(|r| r.as_str())
                    .unwrap_or("")
                    .to_string();
                t.decor_mut()
                    .set_prefix(format!("{lead}{carried}{}", own.trim_start_matches('\n')));
            }
            None => {
                let end = doc.trailing().as_str().unwrap_or("").to_string();
                doc.set_trailing(format!("{end}{carried}"));
            }
        }
    }
}

/// Set `k` to `v`, keeping the comment that trails the old value; a value
/// that already says `v` is left exactly as written.
fn set_value(t: &mut Table, k: &str, v: Value) {
    let same = match (t.get(k).and_then(Item::as_value), &v) {
        (Some(Value::String(a)), Value::String(b)) => a.value() == b.value(),
        (Some(Value::Boolean(a)), Value::Boolean(b)) => a.value() == b.value(),
        _ => false,
    };
    if same {
        return;
    }
    let mut v = v;
    if let Some(old) = t.get(k).and_then(Item::as_value) {
        *v.decor_mut() = old.decor().clone();
    }
    t.insert(k, Item::Value(v));
}

/// Remove `old` and set `new` in its stead, the comment lines above `old`
/// moving to `new` — the `name` and `none = true` of one package line are
/// two spellings of one decision.
fn replace_key(t: &mut Table, old: &str, new: &str, v: Value) {
    let prefix = t
        .get_key_value(old)
        .and_then(|(k, _)| k.leaf_decor().prefix())
        .and_then(|r| r.as_str())
        .map(str::to_string);
    t.remove(old);
    match prefix {
        Some(p) if !t.contains_key(new) && !p.is_empty() => {
            let mut key = Key::new(new);
            key.leaf_decor_mut().set_prefix(p);
            t.insert_formatted(&key, value(v));
        }
        _ => set_value(t, new, v),
    }
}

/// A file that held only comments (the page's header, or the owner's) keeps
/// them all as its trailing text, which `toml_edit` writes after every
/// table: moved above the first one, where they were.
fn lift_trailing(doc: &mut DocumentMut) {
    let top = doc.trailing().as_str().unwrap_or("").to_string();
    if top.trim().is_empty() {
        return;
    }
    let Some((k, j, _)) = positions(doc).into_iter().min_by_key(|(_, _, p)| *p) else {
        return;
    };
    doc.set_trailing("");
    if let Some(t) = doc
        .as_table_mut()
        .get_mut(&k)
        .and_then(Item::as_array_of_tables_mut)
        .and_then(|a| a.get_mut(j))
    {
        t.decor_mut().set_prefix(format!("{}\n\n", top.trim_end()));
    }
}
