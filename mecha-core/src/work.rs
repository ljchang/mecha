//! `~/.mecha/work/<producer>/` — where a run's generated output goes.
//!
//! Two directories, and they mean opposite things:
//!
//! ```text
//! ~/.mecha/work/<producer>/     generated · mutable · disposable · cleanable
//! ~/.mecha/bundles/<id>/<ver>/  published · immutable · versioned · never deleted
//! ```
//!
//! A *producer* is whatever made the output: a trigger's name, or `chat`, or a
//! session id. The directory is **stable across runs of the same producer**,
//! which is the whole point — yesterday's briefing is an ordinary file in
//! today's run rather than something that has to be fetched back from
//! somewhere. It is also the run's workspace, and that fixes three things at
//! once:
//!
//! - **The jail default.** A trigger with no explicit workspace fell through to
//!   `std::env::current_dir()`, and the daemon's unit sets
//!   `WorkingDirectory=%h`. So an unattended run with filesystem tools was
//!   path-jailed to `$HOME`, which contains `~/.mecha/` — the mail OAuth
//!   tokens, every session transcript, the learning store. Rooting it here
//!   roots it somewhere holding nothing sensitive. (The interactive half of
//!   that hazard is [`ensure_outside_mecha_home`].)
//! - **Cross-run read-back**, as above.
//! - **`notify`.** The shipped morning trigger ended with
//!   `mkdir -p ~/.mecha/briefings && cat > …` — a shell redirect into a
//!   directory it created on the way past, outside every path jail, so no
//!   later run could read it. That existed only because there was no
//!   designated place to write.
//!
//! **Retention is a policy, not an intention.** Anything without one becomes a
//! pile nobody opens, so [`clean`] keeps the last *N* entries per producer and
//! says what it removed. One hard rule: it never removes anything a published
//! bundle names as a source, because "regenerate last week's report" must not
//! silently lose its input.

use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// How many entries per producer survive a [`clean`] that does not say.
///
/// Enough to hold a week and a half of a daily producer, so "what did
/// yesterday's run say" and "what changed since Monday" are both still on
/// disk. A placeholder in the honest sense: it wants a week of real output to
/// tune, and `[work] keep` in config is where that tuning goes.
pub const DEFAULT_KEEP: usize = 10;

/// `~/.mecha`, or `$MECHA_HOME`.
///
/// The override exists for tests and for anyone running two mechas side by
/// side; nothing in a normal install sets it.
pub fn mecha_home() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("MECHA_HOME") {
        if !dir.is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }
    let home = dirs::home_dir().context("cannot determine home directory")?;
    Ok(home.join(".mecha"))
}

/// The owner's `~/.mecha` as the **password database** names it for the
/// current uid — never from `MECHA_HOME` or `HOME`, both of which a command
/// can set on its own command line.
///
/// This is the one location a process descended from the model's `shell`
/// cannot redirect, so the closure guard's lookups read it *beside*
/// [`mecha_home`] (review of #293: `MECHA_HOME=/tmp/x mecha tasks set …`
/// read an empty shell registry and empty run markers, and landed on the
/// owner's-terminal rule). `None` when the database has no entry or no home
/// for this uid, and always under `cfg(test)`: unit tests must never read
/// the owner's real stores.
pub fn owner_mecha_home() -> Option<PathBuf> {
    #[cfg(test)]
    {
        None
    }
    #[cfg(not(test))]
    {
        // SAFETY: `getuid` cannot fail and has no preconditions.
        passwd_home(unsafe { libc::getuid() }).map(|h| h.join(".mecha"))
    }
}

/// The home directory the password database records for `uid`.
#[cfg(not(test))]
fn passwd_home(uid: libc::uid_t) -> Option<PathBuf> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt;
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    // SAFETY: `passwd` is plain data; zeroed is a valid initial value that
    // `getpwuid_r` overwrites.
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: every pointer is to a live local; `getpwuid_r` writes the
    // strings into `buf`, which outlives the reads below.
    let rc = unsafe { libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result) };
    if rc != 0 || result.is_null() || pwd.pw_dir.is_null() {
        return None;
    }
    // SAFETY: `pw_dir` points into `buf`, NUL-terminated by `getpwuid_r`.
    let dir = unsafe { CStr::from_ptr(pwd.pw_dir) };
    let path = PathBuf::from(std::ffi::OsStr::from_bytes(dir.to_bytes()));
    (!path.as_os_str().is_empty()).then_some(path)
}

/// The homes the closure guard reads its stores under: [`mecha_home`], and
/// the owner's real one ([`owner_mecha_home`]) when that differs. Once when
/// they agree, which is the normal case.
///
/// **A real home that cannot be found is not agreement** (review of #294).
/// With no password-database entry to compare against — a container run as
/// a uid its image does not name, an unavailable NSS module — a
/// `MECHA_HOME` a command could have set on its own line cannot be
/// corroborated, so this refuses rather than reading the one redirectable
/// home as the only one. Without `MECHA_HOME` it reads the home `HOME`
/// names, which a command can also set: named residue for a host with no
/// passwd entry, where the sandbox is the answer.
pub fn guard_homes() -> Result<Vec<PathBuf>> {
    guard_homes_from(
        mecha_home()?,
        owner_mecha_home(),
        // Read as `mecha_home` reads it: an empty value is no override.
        std::env::var_os("MECHA_HOME").is_some_and(|v| !v.is_empty()),
    )
}

/// [`guard_homes`]' rule, pure: `mine` is this process's home, `owner` the
/// password database's, `overridden` whether `MECHA_HOME` chose `mine`.
fn guard_homes_from(
    mine: PathBuf,
    owner: Option<PathBuf>,
    overridden: bool,
) -> Result<Vec<PathBuf>> {
    match owner {
        Some(owner) if owner == mine => Ok(vec![mine]),
        Some(owner) => Ok(vec![mine, owner]),
        None if overridden => anyhow::bail!(
            "MECHA_HOME is set, and this account's home is not in the password database, \
             so the override cannot be checked against the owner's real stores"
        ),
        None => Ok(vec![mine]),
    }
}

/// `~/.mecha/work`.
pub fn root() -> Result<PathBuf> {
    Ok(mecha_home()?.join("work"))
}

/// `~/.mecha/bundles` — the published mirror. Written by the publisher, read
/// here only to find out what [`clean`] must not remove.
pub fn bundles_root() -> Result<PathBuf> {
    Ok(mecha_home()?.join("bundles"))
}

/// A producer name is a directory name, a CLI argument and a log line. Keep it
/// to what is unambiguous in all three — the same rule trigger names follow,
/// because a trigger name *is* a producer name.
pub fn valid_producer(name: &str) -> Result<()> {
    anyhow::ensure!(!name.is_empty(), "a producer needs a name");
    anyhow::ensure!(
        name.len() <= 64,
        "producer name `{name}` is too long (64 characters max)"
    );
    anyhow::ensure!(
        name.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'),
        "producer name `{name}` may only contain lowercase letters, digits, `-` and `_`"
    );
    Ok(())
}

/// The directory for one producer, without creating it.
pub fn producer_dir(producer: &str) -> Result<PathBuf> {
    valid_producer(producer)?;
    Ok(root()?.join(producer))
}

/// The directory for one producer, created if absent.
///
/// Owner-only like every other directory under `~/.mecha`: a run's scratch
/// output is as private as the transcript it came from.
pub fn ensure(producer: &str) -> Result<PathBuf> {
    let dir = producer_dir(producer)?;
    crate::create_private_dir(&dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(dir)
}

/// Refuse a workspace that **contains** the mecha home.
///
/// `$HOME` contains `~/.mecha`, so a run started there is jailed over the mail
/// OAuth tokens, every session transcript and the learning store — which is
/// close to no jail at all, and is the silently-degrading-sandbox shape this
/// project keeps naming. The interlock is still a backstop (reading a token
/// arms `private_data`, and exfiltration needs an `external_send` it refuses),
/// but a backstop is not a boundary.
///
/// Note the direction. A workspace *inside* the mecha home is fine and is in
/// fact the new default — `~/.mecha/work/morning/` holds nothing sensitive.
/// What is refused is a workspace the mecha home sits *under*.
pub fn ensure_outside_mecha_home(workspace: &Path) -> Result<()> {
    let home = mecha_home()?;
    // The home may not exist yet on a first run, and a path that cannot be
    // canonicalized is compared as written — over-refusing a workspace is
    // recoverable, under-refusing one is the bug.
    let home = home.canonicalize().unwrap_or(home);
    let workspace_c = workspace.canonicalize();
    let ws = workspace_c.as_deref().unwrap_or(workspace);
    if home.starts_with(ws) {
        anyhow::bail!(
            "workspace {} contains the mecha home ({}), so the path jail would \
             cover the mail tokens, every session transcript and the learning \
             store.\n\
             Run from a project directory instead, or name one explicitly with \
             `--workspace <dir>`.",
            ws.display(),
            home.display()
        );
    }
    Ok(())
}

/// One producer's directory, as [`list`] reports it.
#[derive(Debug, Clone)]
pub struct Producer {
    pub name: String,
    pub path: PathBuf,
    /// Top-level entries, newest first.
    pub entries: Vec<Entry>,
    pub bytes: u64,
}

/// One top-level entry in a producer's directory. A run's output may be a file
/// or a directory (a rendered bundle is a directory), so retention counts
/// entries rather than files.
#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub modified: std::time::SystemTime,
    pub bytes: u64,
    pub is_dir: bool,
}

/// Every producer with a directory, alphabetically.
pub fn list() -> Result<Vec<Producer>> {
    let root = root()?;
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for dir_entry in std::fs::read_dir(&root)? {
        let path = dir_entry?.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let entries = entries_of(&path)?;
        // The producer's own `.spill` is not an entry (`entries_of`), but it
        // is on disk: leaving it out of the total would hide the one
        // directory here that grows between sweeps.
        let bytes = entries.iter().map(|e| e.bytes).sum::<u64>()
            + dir_bytes(&path.join(crate::tool::SPILL_DIR));
        out.push(Producer {
            name,
            path,
            entries,
            bytes,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// A producer's top-level entries, newest first.
fn entries_of(dir: &Path) -> Result<Vec<Entry>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        // A session's spill directory is not an artifact: a board task's
        // workspace is a producer directory itself, so its `.spill` would
        // otherwise take a `keep` slot and, once enough artifacts landed
        // after it, be removed under a conversation whose cut results still
        // name files in it (found on review of #313). It lives as long as
        // the workspace does.
        if entry.file_name() == crate::tool::SPILL_DIR {
            continue;
        }
        let path = entry.path();
        let meta = entry.metadata()?;
        let is_dir = meta.is_dir();
        out.push(Entry {
            modified: meta.modified().unwrap_or(std::time::UNIX_EPOCH),
            bytes: if is_dir { dir_bytes(&path) } else { meta.len() },
            path,
            is_dir,
        });
    }
    // Newest first, with the path as a tiebreak so a `clean` is deterministic
    // when two entries share a timestamp — which they will, since a single run
    // writes them.
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.path.cmp(&b.path)));
    Ok(out)
}

fn dir_bytes(dir: &Path) -> u64 {
    let mut total = 0;
    let Ok(read) = std::fs::read_dir(dir) else {
        return 0;
    };
    for entry in read.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        total += if meta.is_dir() {
            dir_bytes(&entry.path())
        } else {
            meta.len()
        };
    }
    total
}

/// Paths a published bundle names as its source, which [`clean`] must never
/// remove.
///
/// The contract with the publisher, and it is deliberately one field of data
/// rather than a shared type: a mirrored version directory
/// (`~/.mecha/bundles/<id>/<ver>/`) may carry a `bundle.json` with a
/// `"sources": ["<absolute path>", …]` array naming what it was rendered from.
/// Anything else in that file is the publisher's business. A mirror that does
/// not exist yet — which is every install until `mecha-factory-publish` is
/// wired — protects nothing, and that is correct rather than a stub.
pub fn protected_sources() -> Result<BTreeSet<PathBuf>> {
    let mut out = BTreeSet::new();
    let root = bundles_root()?;
    if !root.is_dir() {
        return Ok(out);
    }
    for bundle in std::fs::read_dir(&root)?.flatten() {
        let Ok(versions) = std::fs::read_dir(bundle.path()) else {
            continue;
        };
        for version in versions.flatten() {
            let manifest = version.path().join("bundle.json");
            let Ok(text) = std::fs::read_to_string(&manifest) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                tracing::warn!("unreadable bundle manifest {}", manifest.display());
                continue;
            };
            let Some(sources) = value.get("sources").and_then(|s| s.as_array()) else {
                continue;
            };
            for source in sources.iter().filter_map(|s| s.as_str()) {
                let path = PathBuf::from(source);
                out.insert(path.canonicalize().unwrap_or(path));
            }
        }
    }
    Ok(out)
}

/// How long spilled output is kept: a spill directory under `$TMPDIR` whose
/// newest file is older than this, and a file in a workspace's `.spill`
/// older than this.
///
/// Age is when the output was last *written*, which is the best signal this
/// store has of whether a conversation still reads it — not liveness: a
/// session left open a fortnight after its only spill loses that file, and
/// the model's re-read finds it missing rather than wrong.
pub const SPILL_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);

/// Spill directories in `tmp` older than `max_age` as of `now`: the ones a
/// context minted (`tool::SPILL_PREFIX` followed by a UUID — nothing else
/// under a shared `$TMPDIR` is ours to delete) and a process never removed.
/// A symlink is never an entry, whatever its name.
pub fn stale_spills(
    tmp: &Path,
    max_age: std::time::Duration,
    now: std::time::SystemTime,
) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(tmp) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in read.flatten() {
        let name = entry.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|n| n.strip_prefix(crate::tool::SPILL_PREFIX))
        else {
            continue;
        };
        if uuid::Uuid::parse_str(id).is_err() {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        // The newest write inside, not the directory's own mtime — which
        // moves only when an entry is added or removed (found on review of
        // #313).
        let modified = newest_mtime(&path, meta.modified().unwrap_or(std::time::UNIX_EPOCH));
        if now.duration_since(modified).unwrap_or_default() < max_age {
            continue;
        }
        out.push(Entry {
            bytes: dir_bytes(&path),
            path,
            modified,
            is_dir: true,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// What a [`clean`] did, or would do.
#[derive(Debug, Default)]
pub struct CleanReport {
    pub removed: Vec<Entry>,
    /// Entries that were past the keep window but survive because a published
    /// bundle names them. Reported rather than silent — an unexplained
    /// survivor reads as a bug in the retention.
    pub protected: Vec<Entry>,
    pub dry_run: bool,
}

impl CleanReport {
    pub fn bytes_removed(&self) -> u64 {
        self.removed.iter().map(|e| e.bytes).sum()
    }
}

/// Keep the `keep` most recent entries in each producer's directory and remove
/// the rest.
///
/// `only` restricts it to one producer. The producer directories themselves are
/// never removed: a producer with nothing left in it is an empty directory, not
/// an absence, and deleting it would make tomorrow's run recreate it.
pub fn clean(keep: usize, only: Option<&str>, dry_run: bool) -> Result<CleanReport> {
    let protected = protected_sources()?;
    let mut report = CleanReport {
        dry_run,
        ..Default::default()
    };
    let now = std::time::SystemTime::now();
    for producer in list()? {
        if only.is_some_and(|name| name != producer.name) {
            continue;
        }
        // Every workspace's spilled output ages out on the same floor, by
        // file: a `.spill` directly in a producer directory (a board task's
        // workspace, the voice agent's) is not a retention entry and would
        // otherwise be reclaimed by nothing; one inside a kept entry (a web
        // session reused for weeks) would grow for as long as the session
        // does (found on review of #313).
        let mut spills = vec![producer.path.join(crate::tool::SPILL_DIR)];
        spills.extend(
            producer
                .entries
                .iter()
                .take(keep)
                .filter(|e| e.is_dir)
                .map(|e| e.path.join(crate::tool::SPILL_DIR)),
        );
        for spill in spills {
            for file in stale_spill_files(&spill, SPILL_MAX_AGE, now) {
                if !dry_run {
                    std::fs::remove_file(&file.path)
                        .with_context(|| format!("removing {}", file.path.display()))?;
                }
                report.removed.push(file);
            }
        }
        for entry in producer.entries.into_iter().skip(keep) {
            let canonical = entry
                .path
                .canonicalize()
                .unwrap_or_else(|_| entry.path.clone());
            if protected.contains(&canonical) {
                report.protected.push(entry);
                continue;
            }
            if !dry_run {
                let removed = if entry.is_dir {
                    std::fs::remove_dir_all(&entry.path)
                } else {
                    std::fs::remove_file(&entry.path)
                };
                removed.with_context(|| format!("removing {}", entry.path.display()))?;
            }
            report.removed.push(entry);
        }
    }
    Ok(report)
}

/// The newest modification time of `dir` itself (`own`) and the entries
/// directly in it.
fn newest_mtime(dir: &Path, own: std::time::SystemTime) -> std::time::SystemTime {
    let Ok(read) = std::fs::read_dir(dir) else {
        return own;
    };
    read.flatten()
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .fold(own, std::cmp::max)
}

/// Files in a workspace spill directory older than `max_age` as of `now`.
/// Regular files only — a symlink or a directory in there is not spilled
/// output, whatever put it there.
pub fn stale_spill_files(
    spill: &Path,
    max_age: std::time::Duration,
    now: std::time::SystemTime,
) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(spill) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        if now.duration_since(modified).unwrap_or_default() < max_age {
            continue;
        }
        out.push(Entry {
            bytes: meta.len(),
            path,
            modified,
            is_dir: false,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Remove the spill directories [`stale_spills`] finds in `tmp`, or list them
/// on a dry run. Separate from [`clean`] because it reaches outside the mecha
/// home: `mecha work clean` calls it with `$TMPDIR`, and a test hands it a
/// scratch directory rather than the machine's.
pub fn clean_spills(tmp: &Path, dry_run: bool) -> Result<Vec<Entry>> {
    let stale = stale_spills(tmp, SPILL_MAX_AGE, std::time::SystemTime::now());
    if dry_run {
        return Ok(stale);
    }
    // One undeletable directory (a look-alike another user owns in a shared
    // `/tmp`) must not stop the rest, nor hide what `clean` already removed:
    // report it and go on, and list only what was removed.
    let mut removed = Vec::with_capacity(stale.len());
    for entry in stale {
        match std::fs::remove_dir_all(&entry.path) {
            Ok(()) => removed.push(entry),
            Err(e) => {
                tracing::warn!(path = %entry.path.display(), "cannot remove a stale spill directory: {e}")
            }
        }
    }
    Ok(removed)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// `MECHA_HOME` is process-global, so the tests that set it hold one lock
    /// and restore it. Cheaper than threading a root parameter through an API
    /// whose whole job is to know where the mecha home is. `pub(crate)` so
    /// a test in another module that moves the home shares *this* lock
    /// rather than racing it with one of its own.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// The per-store overrides that would point a reader outside the moved
    /// home, cleared for the guard's lifetime and put back after.
    const STORE_OVERRIDES: [&str; 4] = [
        "MECHA_OUTBOX_DIR",
        "MECHA_QUESTIONS_DIR",
        // Also relocates the harness store, which sits under it. Without
        // this a developer with the variable exported ran the no-side-effect
        // backlog test against their real learning store (found on review).
        "MECHA_LEARNING_DIR",
        // A run's saturation streak reads the session store it records
        // into (`Homeostat::at_start`); exported, it would read the real one.
        "MECHA_SESSION_DIR",
    ];

    pub(crate) struct HomeGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        previous: Option<String>,
        overrides: Vec<(&'static str, Option<String>)>,
        dir: PathBuf,
    }

    impl HomeGuard {
        pub(crate) fn new() -> Self {
            let lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
            let previous = std::env::var("MECHA_HOME").ok();
            let overrides = STORE_OVERRIDES
                .iter()
                .map(|k| {
                    let v = std::env::var(k).ok();
                    std::env::remove_var(k);
                    (*k, v)
                })
                .collect();
            let dir = std::env::temp_dir().join(format!("mecha-work-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_var("MECHA_HOME", &dir);
            HomeGuard {
                _lock: lock,
                previous,
                overrides,
                dir,
            }
        }

        /// The moved home.
        pub(crate) fn dir(&self) -> &Path {
            &self.dir
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(v) => std::env::set_var("MECHA_HOME", v),
                None => std::env::remove_var("MECHA_HOME"),
            }
            for (k, v) in &self.overrides {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Write a file with a modification time `age` seconds in the past, so the
    /// ordering under test is the one being asserted rather than whatever the
    /// filesystem's timestamp granularity happened to record for four writes
    /// in the same millisecond.
    fn write_aged(dir: &Path, name: &str, age: i64) {
        use std::os::unix::ffi::OsStrExt;
        let path = dir.join(name);
        std::fs::write(&path, name).unwrap();
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        let when = libc::timeval {
            tv_sec: 1_700_000_000 - age,
            tv_usec: 0,
        };
        let times = [when, when];
        // SAFETY: a valid NUL-terminated path and a two-element timeval array,
        // which is exactly what utimes(2) takes.
        assert_eq!(unsafe { libc::utimes(c.as_ptr(), times.as_ptr()) }, 0);
    }

    #[test]
    fn a_producer_directory_is_stable_and_private() {
        let home = HomeGuard::new();
        let first = ensure("morning").unwrap();
        let second = ensure("morning").unwrap();
        assert_eq!(first, second, "the same producer gets the same directory");
        assert_eq!(first, home.dir.join("work").join("morning"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&first).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "owner-only, like every ~/.mecha leaf");
        }
    }

    #[test]
    fn a_producer_name_that_is_not_a_safe_directory_name_is_refused() {
        let _home = HomeGuard::new();
        for bad in ["", "../escape", "has space", "Upper", "a/b"] {
            assert!(
                producer_dir(bad).is_err(),
                "`{bad}` should not be a producer name"
            );
        }
        assert!(producer_dir("morning-brief_2").is_ok());
    }

    /// The bug this whole module exists to close: a workspace that contains
    /// `~/.mecha` jails over the mail tokens and the transcripts. Fails on the
    /// old behaviour, which accepted any directory that existed.
    #[test]
    fn a_workspace_containing_the_mecha_home_is_refused() {
        let home = HomeGuard::new();
        let parent = home.dir.parent().unwrap();

        let err = ensure_outside_mecha_home(parent).unwrap_err().to_string();
        assert!(
            err.contains("contains the mecha home"),
            "unexpected message: {err}"
        );
        assert!(
            err.contains("--workspace"),
            "the message names the fix: {err}"
        );

        // The home itself contains itself, and holds the secrets directly.
        assert!(ensure_outside_mecha_home(&home.dir).is_err());
    }

    /// And the direction that must stay allowed, or the new default workspace
    /// would refuse itself.
    #[test]
    fn a_workspace_inside_the_mecha_home_is_allowed() {
        let _home = HomeGuard::new();
        let work = ensure("morning").unwrap();
        ensure_outside_mecha_home(&work).unwrap();
    }

    #[test]
    fn clean_keeps_the_newest_n_per_producer_and_reports_what_it_removed() {
        let _home = HomeGuard::new();
        let morning = ensure("morning").unwrap();
        let evening = ensure("evening").unwrap();
        for (i, name) in ["a.md", "b.md", "c.md", "d.md"].iter().enumerate() {
            write_aged(&morning, name, i as i64 * 100);
            write_aged(&evening, name, i as i64 * 100);
        }

        let preview = clean(2, None, true).unwrap();
        assert_eq!(preview.removed.len(), 4, "two producers, two stale each");
        assert!(
            morning.join("d.md").exists(),
            "a dry run removes nothing at all"
        );

        let report = clean(2, Some("morning"), false).unwrap();
        let removed: Vec<_> = report
            .removed
            .iter()
            .map(|e| e.path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(removed, ["c.md", "d.md"], "the two oldest, newest kept");
        assert!(morning.join("a.md").exists());
        assert!(morning.join("b.md").exists());
        assert!(
            evening.join("d.md").exists(),
            "`--producer` restricts the sweep"
        );
        assert!(morning.is_dir(), "the producer directory itself survives");
    }

    /// Set a path's modification time `days` ago.
    fn age(path: &Path, days: u64) {
        let when =
            std::time::SystemTime::now() - std::time::Duration::from_secs(days * 24 * 60 * 60);
        std::fs::File::open(path)
            .unwrap()
            .set_modified(when)
            .unwrap();
    }

    #[test]
    fn spilled_output_ages_out_by_file_at_every_level() {
        let _home = HomeGuard::new();
        // A producer that is itself a workspace (a board task's), and one
        // whose entries are workspaces (web chats).
        let task = ensure("task-9").unwrap();
        let web_main = ensure("web").unwrap().join("main");
        for spill in [
            crate::tool::spill_within(&task),
            crate::tool::spill_within(&web_main),
        ] {
            std::fs::create_dir_all(&spill).unwrap();
            std::fs::write(spill.join("old.txt"), "old").unwrap();
            std::fs::write(spill.join("new.txt"), "new").unwrap();
            age(&spill.join("old.txt"), 30);
        }
        let report = clean(10, None, false).unwrap();
        for spill in [
            crate::tool::spill_within(&task),
            crate::tool::spill_within(&web_main),
        ] {
            assert!(
                !spill.join("old.txt").exists(),
                "{} kept a month-old spill",
                spill.display()
            );
            assert!(spill.join("new.txt").exists(), "a fresh spill survives");
        }
        assert_eq!(report.removed.len(), 2);
        // And `mecha work list` counts what is on disk, spill included.
        let producer = list()
            .unwrap()
            .into_iter()
            .find(|p| p.name == "task-9")
            .unwrap();
        assert_eq!(producer.bytes, 3);
    }

    #[test]
    fn a_spill_directory_is_as_old_as_its_newest_file() {
        // A directory's own mtime moves only when an entry is added, so one
        // written into a month ago but still being overwritten is live.
        let tmp = std::env::temp_dir().join(format!("mecha-spillage-{}", uuid::Uuid::new_v4()));
        let dir = tmp.join(format!(
            "{}{}",
            crate::tool::SPILL_PREFIX,
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("shell-1.txt"), "fresh").unwrap();
        age(&dir, 30);
        assert!(
            stale_spills(&tmp, SPILL_MAX_AGE, std::time::SystemTime::now()).is_empty(),
            "a fresh file inside keeps the directory"
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn a_workspaces_spill_directory_is_never_a_retention_entry() {
        // A board task's workspace is a producer directory itself, so its
        // `.spill` sits where retention looks — and the oldest entry there
        // must still not be it.
        let _home = HomeGuard::new();
        let task = ensure("task-7").unwrap();
        let spill = crate::tool::spill_within(&task);
        std::fs::create_dir_all(&spill).unwrap();
        std::fs::write(spill.join("shell-1.txt"), "still named by a cut result").unwrap();
        write_aged(&task, "report.md", 0);

        let producer = list()
            .unwrap()
            .into_iter()
            .find(|p| p.name == "task-7")
            .unwrap();
        let names: Vec<_> = producer
            .entries
            .iter()
            .map(|e| e.path.file_name().unwrap().to_str().unwrap().to_string())
            .collect();
        assert_eq!(names, ["report.md"], "the spill directory is not listed");

        clean(0, None, false).unwrap();
        assert!(
            spill.join("shell-1.txt").exists(),
            "keep = 0 still spares it"
        );
        assert!(!task.join("report.md").exists());
    }

    /// The one hard rule: an input a published bundle names is not scratch,
    /// however old it is.
    #[test]
    fn clean_never_removes_a_published_bundles_source() {
        let home = HomeGuard::new();
        let work = ensure("morning").unwrap();
        for (i, name) in ["new.md", "old.md"].iter().enumerate() {
            write_aged(&work, name, i as i64 * 100);
        }
        let source = work.join("old.md").canonicalize().unwrap();

        let version = home.dir.join("bundles").join("brief").join("3");
        std::fs::create_dir_all(&version).unwrap();
        std::fs::write(
            version.join("bundle.json"),
            serde_json::json!({ "sources": [source] }).to_string(),
        )
        .unwrap();

        let report = clean(1, None, false).unwrap();
        assert!(
            report.removed.is_empty(),
            "nothing was eligible but the source"
        );
        assert_eq!(report.protected.len(), 1);
        assert!(work.join("old.md").exists());
    }

    #[test]
    fn only_our_own_old_spill_directories_are_stale() {
        let tmp = std::env::temp_dir().join(format!("mecha-spilltest-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let ours = tmp.join(format!(
            "{}{}",
            crate::tool::SPILL_PREFIX,
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&ours).unwrap();
        std::fs::write(ours.join("shell-1-out.txt"), "x".repeat(10)).unwrap();
        // Same prefix, not a UUID: not a directory this program minted.
        std::fs::create_dir_all(tmp.join(format!("{}notes", crate::tool::SPILL_PREFIX))).unwrap();
        // Someone else's directory, and a file wearing our name.
        std::fs::create_dir_all(tmp.join("other-program")).unwrap();
        let file = tmp.join(format!(
            "{}{}",
            crate::tool::SPILL_PREFIX,
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&file, "not a dir").unwrap();
        // A symlink with our name, pointing at something that must survive.
        let target = tmp.join("keep-me");
        std::fs::create_dir_all(&target).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            &target,
            tmp.join(format!(
                "{}{}",
                crate::tool::SPILL_PREFIX,
                uuid::Uuid::new_v4()
            )),
        )
        .unwrap();

        let now = std::time::SystemTime::now();
        // Fresh: nothing is stale yet.
        assert!(stale_spills(&tmp, SPILL_MAX_AGE, now).is_empty());
        // Eight days on: only ours.
        let later = now + std::time::Duration::from_secs(8 * 24 * 60 * 60);
        let stale = stale_spills(&tmp, SPILL_MAX_AGE, later);
        assert_eq!(stale.len(), 1, "{stale:?}");
        assert_eq!(stale[0].path, ours);
        assert_eq!(stale[0].bytes, 10);
        std::fs::remove_dir_all(&tmp).ok();
    }

    /// A mirror that does not exist protects nothing, and must not be an error
    /// — that is every install until the publisher is wired.
    #[test]
    fn no_bundle_mirror_means_no_protected_sources() {
        let _home = HomeGuard::new();
        assert!(protected_sources().unwrap().is_empty());
    }
}

#[cfg(test)]
mod guard_homes_tests {
    use super::*;

    /// An override the password database cannot corroborate is refused, not
    /// read as the only home (review of #294: `None` collapsed into the
    /// agreeing case and reopened #293's `MECHA_HOME` redirect).
    #[test]
    fn an_uncorroborated_home_override_is_refused_not_trusted() {
        let mine = PathBuf::from("/tmp/elsewhere/.mecha");
        let owner = PathBuf::from("/home/owner/.mecha");
        assert_eq!(
            guard_homes_from(owner.clone(), Some(owner.clone()), false).unwrap(),
            vec![owner.clone()]
        );
        assert_eq!(
            guard_homes_from(mine.clone(), Some(owner.clone()), true).unwrap(),
            vec![mine.clone(), owner]
        );
        assert!(guard_homes_from(mine.clone(), None, true).is_err());
        assert_eq!(
            guard_homes_from(mine.clone(), None, false).unwrap(),
            vec![mine]
        );
    }
}
