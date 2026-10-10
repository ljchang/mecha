//! `~/.mecha/hud/`: installed boards, their datasets, and the ledger that
//! says when each loader last ran and how it went.
//!
//! ```text
//! ~/.mecha/hud/
//!   sources.toml                    owner-written (source.rs)
//!   boards/<id>/
//!     hud.json, loaders/*.toml      installed, checked together
//!     data/<name>.json              the latest dataset of each loader
//!     ledger.jsonl                  append-only: installed, refreshed, refused, failed
//! ```
//!
//! **Due-ness is answered backwards from the ledger**, as triggers answer it:
//! the most recent cron slot at or before now, against the last slot already
//! accounted for. A machine asleep for a week owes one refresh per loader,
//! not hundreds, and a scheduler with no state of its own (a timer running
//! `mecha hud refresh --due`) reaches the same answer as one that never
//! stopped. A refresh started by hand records no slot, so it never cancels
//! the next scheduled one.
//!
//! **Refused and failed are kept apart.** A refusal is the loader's own fault —
//! its output drifted from the declared shape, it returned too many rows, it
//! tried to write — and the previous dataset stays, shown as stale with the
//! reason. A failure is the environment's — a source not registered, a file
//! that will not open, a query that ran out of time. Both are findings for
//! `mecha doctor`; neither is ever silent.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::loader::{Column, Loader};
use super::runner::{run_sqlite, RunError, QUERY_TIMEOUT};
use super::source::{SourceKind, Sources};
use super::{is_identifier, Installed, Refusal, Refusals};

/// One loader's latest output, as written to `data/<name>.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dataset {
    pub name: String,
    /// When the query ran — the freshness every surface shows.
    pub generated_at: DateTime<Utc>,
    /// Monotonic per loader at home; a push carries it.
    pub generation: u64,
    /// The loader that produced this, as review would name it.
    pub loader_digest: String,
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Value>>,
    /// Whether the rows carry text someone other than the owner wrote — the
    /// source's, by kind then content (`source.rs`).
    pub external: bool,
}

/// One ledger line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub at: DateTime<Utc>,
    #[serde(flatten)]
    pub event: Event,
}

/// What happened. A closed enum written to an append-only store is a wire
/// format: a line this build does not understand is skipped on read, never a
/// reason to fail the whole ledger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Installed,
    Refreshed {
        loader: String,
        /// The cron slot this refresh accounted for; `None` for a refresh run
        /// by hand, which never advances the schedule.
        slot: Option<DateTime<Utc>>,
        generation: u64,
        rows: usize,
    },
    Refused {
        loader: String,
        slot: Option<DateTime<Utc>>,
        reason: String,
    },
    Failed {
        loader: String,
        slot: Option<DateTime<Utc>>,
        reason: String,
    },
}

impl Event {
    fn loader(&self) -> Option<&str> {
        match self {
            Event::Installed => None,
            Event::Refreshed { loader, .. }
            | Event::Refused { loader, .. }
            | Event::Failed { loader, .. } => Some(loader),
        }
    }

    fn slot(&self) -> Option<DateTime<Utc>> {
        match self {
            Event::Installed => None,
            Event::Refreshed { slot, .. }
            | Event::Refused { slot, .. }
            | Event::Failed { slot, .. } => *slot,
        }
    }
}

/// A board's ledger, read whole. Small: one line per refresh.
#[derive(Debug, Clone, Default)]
pub struct Ledger {
    pub entries: Vec<Entry>,
    /// Lines that did not parse — kept as a count so a reader can say so.
    pub unreadable: usize,
}

impl Ledger {
    pub fn installed_at(&self) -> Option<DateTime<Utc>> {
        self.entries
            .iter()
            .find(|e| e.event == Event::Installed)
            .map(|e| e.at)
    }

    /// The last slot accounted for — refreshed, refused or failed alike: a
    /// refused slot was attempted, and retrying it every minute until the
    /// query changes would only fill the ledger.
    pub fn last_slot(&self, loader: &str) -> Option<DateTime<Utc>> {
        self.entries
            .iter()
            .filter(|e| e.event.loader() == Some(loader))
            .filter_map(|e| e.event.slot())
            .max()
    }

    /// The highest generation a refresh of this loader recorded. Read from
    /// the ledger, not the dataset file: a file that will not parse must not
    /// read as "never refreshed" and restart the count at 1.
    pub fn last_generation(&self, loader: &str) -> Option<u64> {
        self.entries
            .iter()
            .filter_map(|e| match &e.event {
                Event::Refreshed {
                    loader: l,
                    generation,
                    ..
                } if l == loader => Some(*generation),
                _ => None,
            })
            .max()
    }

    /// The most recent event for one loader.
    pub fn last(&self, loader: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .rev()
            .find(|e| e.event.loader() == Some(loader))
    }
}

/// Which loaders a refresh runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    /// Only those whose cron slot has passed — what the timer runs.
    Due,
    /// Every loader, now, recording no slot.
    All,
}

/// What one loader's refresh did.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Outcome {
    pub board: String,
    pub loader: String,
    pub event: Event,
}

/// One board, as `list` and the doctor see it.
#[derive(Debug, Clone, Serialize)]
pub struct BoardStatus {
    pub id: String,
    pub title: Option<String>,
    /// Why the board does not load, if it does not.
    pub refusals: Vec<String>,
    pub loaders: Vec<LoaderStatus>,
    pub unreadable_ledger_lines: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoaderStatus {
    pub name: String,
    pub source: String,
    pub last: Option<Entry>,
    pub generated_at: Option<DateTime<Utc>>,
    /// Older than two of the loader's periods, or never produced once two
    /// periods have passed since install.
    pub stale: bool,
    /// Why the dataset file could not be read, if it could not — a finding of
    /// its own, never "never produced".
    pub dataset_error: Option<String>,
}

/// The store, rooted at a directory (`~/.mecha/hud/` in use, a scratch one
/// in tests).
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn at(root: impl Into<PathBuf>) -> Store {
        Store { root: root.into() }
    }

    /// `~/.mecha/hud/`.
    pub fn open() -> Result<Store> {
        Ok(Store::at(super::dir()?))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn boards(&self) -> PathBuf {
        self.root.join("boards")
    }

    /// A board's directory — for an id already proved an identifier.
    /// Containment is proved at the join, here, for every public entry point
    /// that takes an id from its caller: a request path or a model may supply
    /// one (step 4's routes), and `../..` must never reach the filesystem.
    fn board(&self, id: &str) -> Result<PathBuf> {
        if !is_identifier(id) {
            anyhow::bail!("{id:?} is not a board id ([a-z][a-z0-9_]*)");
        }
        Ok(self.boards().join(id))
    }

    pub fn sources(&self) -> Result<std::result::Result<Sources, Refusals>> {
        Sources::load(&self.root.join("sources.toml"))
    }

    /// Installed board ids, sorted. A directory whose name is not an
    /// identifier is not a board (an interrupted install's staging directory,
    /// say) and is skipped.
    pub fn board_ids(&self) -> Result<Vec<String>> {
        let dir = self.boards();
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

    pub fn load(&self, id: &str) -> Result<std::result::Result<Installed, Refusals>> {
        Installed::load(&self.boards(), id)
    }

    pub fn ledger(&self, id: &str) -> Result<Ledger> {
        let path = self.board(id)?.join("ledger.jsonl");
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Ledger::default()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let mut ledger = Ledger::default();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str::<Entry>(line) {
                Ok(entry) => ledger.entries.push(entry),
                Err(_) => ledger.unreadable += 1,
            }
        }
        Ok(ledger)
    }

    fn append(&self, id: &str, entry: &Entry) -> Result<()> {
        let path = self.board(id)?.join("ledger.jsonl");
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        writeln!(file, "{}", serde_json::to_string(entry)?)?;
        Ok(())
    }

    pub fn dataset(&self, id: &str, name: &str) -> Result<Option<Dataset>> {
        if !is_identifier(name) {
            anyhow::bail!("{name:?} is not a dataset name ([a-z][a-z0-9_]*)");
        }
        let path = self.board(id)?.join("data").join(format!("{name}.json"));
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Some(
                serde_json::from_str(&text)
                    .with_context(|| format!("parsing {}", path.display()))?,
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Check a draft directory — its spec, its loaders, and that every loader
    /// names a registered source — without installing it.
    pub fn validate(
        &self,
        draft: &Path,
        id: &str,
    ) -> Result<std::result::Result<Installed, Refusals>> {
        // The question install will ask, asked first: an id install refuses
        // must not validate.
        if !is_identifier(id) {
            return Ok(Err(Refusals(vec![Refusal::new(
                "",
                format!("a board id matches [a-z][a-z0-9_]* (got {id:?}); pass --id"),
            )])));
        }
        let installed = match Installed::load_dir(draft, id)? {
            Ok(installed) => installed,
            Err(r) => return Ok(Err(r)),
        };
        let sources = match self.sources()? {
            Ok(sources) => sources,
            Err(r) => return Ok(Err(r)),
        };
        let mut out = Vec::new();
        for (name, loader) in installed.loaders() {
            if sources.get(loader.source()).is_none() {
                out.push(Refusal::new(
                    format!("loaders/{name}.toml#/source"),
                    format!(
                        "source {:?} is not registered in sources.toml",
                        loader.source()
                    ),
                ));
            }
        }
        Ok(if out.is_empty() {
            Ok(installed)
        } else {
            Err(Refusals(out))
        })
    }

    /// Install a draft as a board — the owner's act (design §5.3). The id is
    /// the draft directory's own name unless one is given; it is checked
    /// before it is joined, and a board already installed under it is
    /// refused rather than overwritten. The copy lands in a staging directory
    /// and is renamed into place, so a reader never sees half a board. The
    /// files are read again for the copy, so what lands need not be the bytes
    /// that were checked — which is why every refresh re-checks the board
    /// (`Installed::load`) rather than trusting the install. Keep that.
    pub fn install(
        &self,
        draft: &Path,
        id: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<std::result::Result<Installed, Refusals>> {
        let id = match id {
            Some(id) => id.to_string(),
            None => draft
                .file_name()
                .and_then(|n| n.to_str())
                .context("the draft directory needs a name, or pass --id")?
                .to_string(),
        };
        if !is_identifier(&id) {
            return Ok(Err(Refusals(vec![Refusal::new(
                "",
                format!("a board id matches [a-z][a-z0-9_]* (got {id:?}); pass --id"),
            )])));
        }
        let target = self.board(&id)?;
        if target.exists() {
            return Ok(Err(Refusals(vec![Refusal::new(
                "",
                format!("board {id:?} is already installed; remove it before installing again"),
            )])));
        }
        let installed = match self.validate(draft, &id)? {
            Ok(installed) => installed,
            Err(r) => return Ok(Err(r)),
        };

        crate::create_private_dir(&self.boards())?;
        let staging = self
            .boards()
            .join(format!(".install-{id}-{}", std::process::id()));
        if staging.exists() {
            std::fs::remove_dir_all(&staging)?;
        }
        crate::create_private_dir(&staging.join("loaders"))?;
        std::fs::copy(draft.join("hud.json"), staging.join("hud.json"))?;
        for name in installed.loaders().keys() {
            let file = format!("{name}.toml");
            std::fs::copy(
                draft.join("loaders").join(&file),
                staging.join("loaders").join(&file),
            )?;
        }
        std::fs::rename(&staging, &target).with_context(|| format!("installing board {id:?}"))?;
        self.append(
            &id,
            &Entry {
                at: now,
                event: Event::Installed,
            },
        )?;
        Ok(Ok(installed))
    }

    /// Refresh one board's loaders. `Ok(Err)` when the board itself does not
    /// load — every loader is then unrunnable, and the refusals say why.
    pub fn refresh(
        &self,
        id: &str,
        now: DateTime<Utc>,
        which: Which,
    ) -> Result<std::result::Result<Vec<Outcome>, Refusals>> {
        // A typo is "no such board", not a board that looks corrupt.
        if is_identifier(id) && !self.board(id)?.is_dir() {
            return Ok(Err(Refusals(vec![Refusal::new(
                "",
                format!("no board named {id:?} is installed (`mecha hud list`)"),
            )])));
        }
        let installed = match self.load(id)? {
            Ok(installed) => installed,
            Err(r) => return Ok(Err(r)),
        };
        let _lock = lock(&self.board(id)?)?;
        let ledger = self.ledger(id)?;
        let sources = self.sources()?;
        let mut outcomes = Vec::new();
        for (name, loader) in installed.loaders() {
            let slot = match which {
                Which::All => None,
                Which::Due => match due(loader, &ledger, now) {
                    Some(slot) => Some(slot),
                    None => continue,
                },
            };
            let event = match &sources {
                Err(r) => Event::Failed {
                    loader: name.clone(),
                    slot,
                    reason: format!("sources.toml does not load: {r}"),
                },
                Ok(sources) => self.run_one(id, loader, sources, &ledger, slot, now)?,
            };
            self.append(
                id,
                &Entry {
                    at: now,
                    event: event.clone(),
                },
            )?;
            outcomes.push(Outcome {
                board: id.to_string(),
                loader: name.clone(),
                event,
            });
        }
        Ok(Ok(outcomes))
    }

    fn run_one(
        &self,
        id: &str,
        loader: &Loader,
        sources: &Sources,
        ledger: &Ledger,
        slot: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<Event> {
        let name = loader.name().to_string();
        let Some(source) = sources.get(loader.source()) else {
            return Ok(Event::Failed {
                loader: name,
                slot,
                reason: format!(
                    "source {:?} is not registered in sources.toml",
                    loader.source()
                ),
            });
        };
        let fetched = match &source.kind {
            SourceKind::Sqlite { path } => {
                run_sqlite(path, loader.query(), loader.max_rows(), QUERY_TIMEOUT)
            }
        };
        let fetched = match fetched {
            Ok(fetched) => fetched,
            Err(e) => {
                let reason = e.to_string();
                return Ok(match e {
                    RunError::MultipleStatements
                    | RunError::NotAllowed(_)
                    | RunError::NotReadOnly
                    | RunError::Unrepresentable { .. }
                    | RunError::TooManyRows { .. }
                    | RunError::TooLarge { .. } => Event::Refused {
                        loader: name,
                        slot,
                        reason,
                    },
                    RunError::Open(_) | RunError::Sql(_) | RunError::Timeout => Event::Failed {
                        loader: name,
                        slot,
                        reason,
                    },
                });
            }
        };
        let rows = match loader.shape(&fetched.columns, fetched.rows) {
            Ok(rows) => rows,
            Err(e) => {
                return Ok(Event::Refused {
                    loader: name,
                    slot,
                    reason: e.to_string(),
                })
            }
        };
        let generation = ledger.last_generation(&name).map_or(1, |g| g + 1);
        let count = rows.len();
        let dataset = Dataset {
            name: name.clone(),
            generated_at: now,
            generation,
            loader_digest: loader.digest(),
            columns: loader.columns().to_vec(),
            rows,
            external: source.external(),
        };
        // A dataset that cannot be written is the environment's failure and
        // gets its ledger line like any other — never an error that leaves
        // the loader reading green and aborts the boards after it.
        let written = serde_json::to_string(&dataset)
            .map_err(anyhow::Error::from)
            .and_then(|json| {
                write_atomic(
                    &self.board(id)?.join("data").join(format!("{name}.json")),
                    json.as_bytes(),
                )
            });
        if let Err(e) = written {
            return Ok(Event::Failed {
                loader: name,
                slot,
                reason: format!("the dataset could not be written: {e:#}"),
            });
        }
        Ok(Event::Refreshed {
            loader: name,
            slot,
            generation,
            rows: count,
        })
    }

    /// Every board's state, for `list` and the doctor. A board that does not
    /// load is reported with its refusals, never skipped.
    pub fn status(&self, now: DateTime<Utc>) -> Result<Vec<BoardStatus>> {
        let mut out = Vec::new();
        for id in self.board_ids()? {
            let ledger = self.ledger(&id)?;
            match self.load(&id)? {
                Err(r) => out.push(BoardStatus {
                    id,
                    title: None,
                    refusals: r.0.iter().map(|r| r.to_string()).collect(),
                    loaders: Vec::new(),
                    unreadable_ledger_lines: ledger.unreadable,
                }),
                Ok(installed) => {
                    let mut loaders = Vec::new();
                    for (name, loader) in installed.loaders() {
                        let (generated_at, dataset_error) = match self.dataset(&id, name) {
                            Ok(d) => (d.map(|d| d.generated_at), None),
                            Err(e) => (None, Some(format!("{e:#}"))),
                        };
                        loaders.push(LoaderStatus {
                            name: name.clone(),
                            source: loader.source().to_string(),
                            last: ledger.last(name).cloned(),
                            generated_at,
                            stale: stale(loader, generated_at.or(ledger.installed_at()), now),
                            dataset_error,
                        });
                    }
                    out.push(BoardStatus {
                        title: Some(installed.spec().title().to_string()),
                        id,
                        refusals: Vec::new(),
                        loaders,
                        unreadable_ledger_lines: ledger.unreadable,
                    });
                }
            }
        }
        Ok(out)
    }
}

fn tz(loader: &Loader) -> Tz {
    loader
        .timezone()
        .and_then(|t| t.parse().ok())
        .unwrap_or(chrono_tz::UTC)
}

/// The slot to refresh for, if one has passed since the last slot accounted
/// for (or since install, for a loader that has never run).
pub fn due(loader: &Loader, ledger: &Ledger, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let slot = loader.schedule().prev_at_or_before(now, tz(loader))?;
    let anchor = ledger.last_slot(loader.name()).or(ledger.installed_at());
    if anchor.is_some_and(|a| slot <= a) {
        None
    } else {
        Some(slot)
    }
}

/// Older than two of the loader's periods. `since` is the dataset's
/// `generated_at`, or the install time for one never produced.
fn stale(loader: &Loader, since: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    let Some(since) = since else {
        return false;
    };
    let tz = tz(loader);
    let period = loader
        .schedule()
        .prev_at_or_before(now, tz)
        .and_then(|prev| {
            loader
                .schedule()
                .next_after(prev, tz)
                .map(|next| next - prev)
        });
    period.is_some_and(|p| now - since > p * 2)
}

/// Temp sibling, then rename: a reader sees the old file or the new one.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("a dataset path has a parent")?;
    crate::create_private_dir(dir)?;
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("data"),
        std::process::id()
    ));
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// The board's writer lock: two refreshes of one board — the timer's and a
/// hand-run one — take turns, so generations never collide.
struct BoardLock {
    _file: std::fs::File,
}

fn lock(board: &Path) -> Result<BoardLock> {
    use std::os::unix::io::AsRawFd;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(board.join(".lock"))?;
    // SAFETY: flock on an fd we own, held open by the returned guard; the
    // kernel releases it if the process dies.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(std::io::Error::last_os_error()).context("locking the board");
    }
    Ok(BoardLock { _file: file })
}

#[cfg(test)]
mod lock_tests {
    use super::lock;
    use std::os::unix::io::AsRawFd;

    fn try_lock(path: &std::path::Path) -> bool {
        let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        // SAFETY: flock on an fd we own; LOCK_NB so a held lock answers now.
        let ok = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
        if ok {
            // SAFETY: as above.
            unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_UN) };
        }
        ok
    }

    /// Two refreshes of one board take turns: while the guard lives, a second
    /// claim is refused, and dropping the guard releases it.
    #[test]
    fn the_board_lock_holds_until_its_guard_drops() {
        let dir = std::env::temp_dir().join(format!("mecha-hud-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let guard = lock(&dir).unwrap();
        assert!(!try_lock(&dir.join(".lock")), "a second claim while held");
        drop(guard);
        assert!(
            try_lock(&dir.join(".lock")),
            "released when the guard drops"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
