//! A persona's memory: the store (`docs/PERSONA-DESIGN.md` §9, build step 5).
//!
//! Two kinds of file, and the split is the separation:
//!
//! - **`<persona>/memory.db`** — that persona's, and no one else's: the
//!   episodes of its chats, the facts true in its relationship or story, and
//!   what it learned about the owner, with inferred facts in a table of their
//!   own (D18). A chat opens exactly one of these, so there is no
//!   `WHERE persona = …` anywhere to forget (§9.10).
//! - **`shared.db`** at the top of the store — the user facts the *owner*
//!   chose to share, with everyone or with one group (§4.5, D17). It is the
//!   one place a filter separates personas, so the filter lives in one
//!   function, [`Shared::visible_to`], with its own test.
//!
//! What every row carries, and why (§9.4):
//!
//! - **`source` is mandatory** — the chat and the turns a record came from.
//!   It is the pointer back to the ground truth, and it is the deletion key:
//!   [`forget_chat`] removes every row whose source is a chat, in both files,
//!   in one pass. A record with no source could never be forgotten, so the
//!   schema refuses one.
//! - **`origin`** is classified from the conversation's taint, as image
//!   library candidates are ([`Origin::of_proposal`]). A model-untrusted
//!   record is stored as a **candidate** and never recalled until the owner
//!   approves it (§9.6) — memory is not a laundering path.
//! - **Append-only text.** A correction invalidates the old row and adds a
//!   new one pointing at it ([`Fact::replaces`]), so what was believed on a
//!   given day stays answerable. Only forgetting deletes, and it deletes with
//!   `secure_delete` on and the write-ahead log truncated after, so the text
//!   survives neither in freed pages nor in the log.
//!
//! Closed sets are stored through their serde names, so a value a newer
//! binary wrote loads as the narrowest variant rather than failing the row:
//! an unknown origin is untrusted, an unknown status is a candidate.
//!
//! Nothing here calls a model, reads a transcript or touches a prompt. The
//! writer (§9.6) and recall (§9.7) are later slices and go through this API.

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Row};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{validate_name, validate_persona_name, Origin};

/// A persona's own memory, inside its folder.
pub const MEMORY_DB: &str = "memory.db";
/// What the owner shared, at the top of the store.
pub const SHARED_DB: &str = "shared.db";
/// One sentence, not a document: a fact longer than this is a summary, and
/// belongs in an episode.
pub const MAX_FACT_CHARS: usize = 500;
/// An episode's summary.
pub const MAX_SUMMARY_CHARS: usize = 4000;

/// `PRAGMA user_version` this binary writes. Migrations are additive:
/// 1 is the records, 2 the writer's ledger (`written`), 3 the recall index
/// (`recall_fts`, `vectors`).
const SCHEMA: i64 = 3;

/// Which table of `memory.db` a fact lives in. A closed set mapped to fixed
/// names, so no caller's string ever reaches SQL as an identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Table {
    /// The persona's own semantic memory: names, canon, what it said about
    /// itself (§9.3).
    Persona,
    /// What it learned about the owner, said or seen (§9.5).
    User,
    /// What it inferred about the owner — kept apart so turning inferences
    /// off, or measuring them, is one switch (D18).
    Inferred,
}

impl Table {
    const ALL: [Table; 3] = [Table::Persona, Table::User, Table::Inferred];

    fn sql(self) -> &'static str {
        match self {
            Table::Persona => "facts",
            Table::User => "user_facts",
            Table::Inferred => "inferred_user_facts",
        }
    }

    /// Facts about the owner — the only ones the owner can share (§9.5).
    pub fn is_about_owner(self) -> bool {
        matches!(self, Table::User | Table::Inferred)
    }
}

/// How a fact is known (§9.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The owner said it.
    Stated,
    /// It happened in the chat.
    Observed,
    /// The writer's reading. The narrowest claim, so an unknown kind lands
    /// here.
    #[default]
    #[serde(other)]
    Inferred,
}

/// Whether a record reaches a chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Recalled.
    Active,
    /// Superseded or withdrawn; kept so the past stays answerable.
    Invalidated,
    /// Waiting on the owner — never recalled. An unknown status lands here.
    #[default]
    #[serde(other)]
    Candidate,
}

impl Status {
    /// Where a newly written record starts (§9.5, §9.6): untrusted waits on
    /// the owner; anything else — inferred facts included (D18) — is active.
    pub fn initial(origin: Origin) -> Status {
        match origin {
            Origin::ModelUntrusted => Status::Candidate,
            Origin::Owner | Origin::ModelClean => Status::Active,
        }
    }
}

/// The chat and turns a record came from — its ground truth and its
/// deletion key. Turns are indexes into the transcript's messages, inclusive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub chat: String,
    pub from: u32,
    pub to: u32,
}

impl Source {
    /// When the chat began, as RFC 3339 UTC, read from its id: a session id
    /// starts with its UTC start (`session::new_id`). `None` for an id that
    /// does not, so a caller falls back rather than invents.
    pub fn chat_began(&self) -> Option<String> {
        let stamp = self.chat.get(..15)?;
        chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%dT%H%M%S")
            .ok()
            .map(|t| {
                t.and_utc()
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            })
    }

    fn check(&self) -> Result<()> {
        if self.chat.trim().is_empty() {
            bail!("a memory record needs the chat it came from");
        }
        if self.to < self.from {
            bail!("turns {}..{} run backwards", self.from, self.to);
        }
        Ok(())
    }
}

/// Who may read a shared fact (§4.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Audience {
    Everyone,
    Group(String),
}

impl Audience {
    /// `everyone` or `group:<name>` — a name cannot hold `:`, so the two can
    /// never be confused, whatever a group is called.
    fn wire(&self) -> String {
        match self {
            Audience::Everyone => "everyone".into(),
            Audience::Group(g) => format!("group:{g}"),
        }
    }

    fn from_wire(s: &str) -> Result<Audience> {
        match s {
            "everyone" => Ok(Audience::Everyone),
            _ => match s.strip_prefix("group:") {
                Some(g) if validate_name(g).is_ok() => Ok(Audience::Group(g.into())),
                _ => bail!("unreadable audience `{s}`"),
            },
        }
    }
}

/// A fact to write. The store stamps the rest.
#[derive(Debug, Clone, PartialEq)]
pub struct NewFact {
    pub text: String,
    pub kind: Kind,
    pub source: Source,
    pub origin: Origin,
    /// The model the chat ran on.
    pub model: String,
    /// When it became true in the world, if known.
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
}

/// A fact as stored (§9.4).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Fact {
    pub uid: String,
    pub table: Table,
    pub text: String,
    pub kind: Kind,
    pub source: Source,
    /// The persona whose chat it came from.
    pub learned_by: String,
    pub origin: Origin,
    pub model: String,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    pub ingested_at: String,
    pub invalidated_at: Option<String>,
    pub status: Status,
    pub pinned: bool,
    /// The row this one corrected, if any.
    pub replaces: Option<String>,
}

/// When a record was said, which is what a reader dates it by: the time it
/// names as true from, else the start of the chat it came from, and only
/// then the night it was written. Dated by the write, a fever from Tuesday
/// read on Friday as Friday's news (2026-10-02, the first real night).
fn said_at(valid_from: Option<&str>, source: &Source, ingested_at: &str) -> String {
    valid_from
        .map(str::to_owned)
        .or_else(|| source.chat_began())
        .unwrap_or_else(|| ingested_at.to_owned())
}

impl Fact {
    /// See [`said_at`].
    pub fn said_at(&self) -> String {
        said_at(self.valid_from.as_deref(), &self.source, &self.ingested_at)
    }
}

/// An episode to write (§9.2).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NewEpisode {
    pub source: Option<Source>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub summary: String,
    pub topics: Vec<String>,
    pub decisions: Vec<String>,
    pub open_threads: Vec<String>,
    pub scenario: Option<String>,
    /// Files the persona drew on.
    pub files: Vec<String>,
    pub origin: Origin,
    pub model: String,
}

/// An episode as stored: the index card for a stretch of a chat.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Episode {
    pub uid: String,
    pub source: Source,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub summary: String,
    pub topics: Vec<String>,
    pub decisions: Vec<String>,
    pub open_threads: Vec<String>,
    pub scenario: Option<String>,
    pub files: Vec<String>,
    pub origin: Origin,
    pub model: String,
    pub ingested_at: String,
    pub invalidated_at: Option<String>,
    pub status: Status,
    pub pinned: bool,
}

/// A user fact the owner shared, as `shared.db` holds it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SharedFact {
    pub uid: String,
    pub audience: Audience,
    /// The row in the learner's `memory.db` this was copied from.
    pub from_uid: String,
    pub from_table: Table,
    pub text: String,
    pub kind: Kind,
    /// Kept from the original, so forgetting the chat removes the copy too.
    pub source: Source,
    pub learned_by: String,
    pub origin: Origin,
    pub model: String,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    pub ingested_at: String,
    pub shared_at: String,
}

impl SharedFact {
    /// When the original was said, as [`Fact::said_at`]; never when it was
    /// shared, which is the owner's act, not the fact's date.
    pub fn said_at(&self) -> String {
        said_at(self.valid_from.as_deref(), &self.source, &self.ingested_at)
    }
}

impl Episode {
    /// When the conversation happened: its end, its start, else the start of
    /// the chat — a stretch past the first turn has no start of its own —
    /// and only then the night it was written.
    pub fn said_at(&self) -> String {
        self.ended_at
            .clone()
            .or_else(|| self.started_at.clone())
            .unwrap_or_else(|| said_at(None, &self.source, &self.ingested_at))
    }
}

/// What [`forget_chat`] removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Forgotten {
    pub episodes: usize,
    pub facts: usize,
    pub shared: usize,
}

// ── wire helpers ────────────────────────────────────────────────────────

/// A closed set's serde name, as stored.
fn wire<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        other => unreachable!("a unit variant serialises as a string, got {other:?}"),
    }
}

/// The reverse, narrowing anything unreadable to the type's default — which
/// for every set stored here is its narrowest variant.
fn unwire<T: DeserializeOwned + Default>(s: &str) -> T {
    serde_json::from_value(serde_json::Value::String(s.to_owned())).unwrap_or_default()
}

fn list(v: &[String]) -> String {
    serde_json::to_string(v).expect("a list of strings serialises")
}

fn unlist(s: &str) -> Vec<String> {
    serde_json::from_str(s).unwrap_or_default()
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn uid() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn check_text(text: &str, max: usize, what: &str) -> Result<()> {
    if text.trim().is_empty() {
        bail!("{what} is empty");
    }
    if text.chars().count() > max {
        bail!("{what} is longer than {max} characters");
    }
    Ok(())
}

// ── opening ─────────────────────────────────────────────────────────────

/// Open (creating, owner-only) or open existing (read-only, never creating).
fn connect(path: &Path, create: bool) -> Result<Option<Connection>> {
    if !create && !path.is_file() {
        return Ok(None);
    }
    if create && !path.exists() {
        // Owner-only from the first byte: SQLite would create it with the
        // umask's mode, and its -wal and -shm files copy the main file's.
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        match opts.open(path) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e).with_context(|| format!("creating {}", path.display())),
        }
    }
    let flags = if create {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    };
    let conn = Connection::open_with_flags(path, flags)
        .with_context(|| format!("opening {}", path.display()))?;
    // The nightly writer, the page and a chat can all hold one at once.
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    if create {
        // Per connection: a delete overwrites the freed page, so `forget`
        // means it (§9.10). Both are read back rather than trusted: on a
        // filesystem without shared memory WAL does not engage, and
        // `wal_checkpoint` on a database not in WAL mode is a no-op that
        // reports `busy = 0` — so `scrub` would pass having checked nothing
        // (review of #463).
        conn.pragma_update(None, "secure_delete", "ON")?;
        let secure: i64 = conn.pragma_query_value(None, "secure_delete", |r| r.get(0))?;
        let mode: String =
            conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
        if secure != 1 || !mode.eq_ignore_ascii_case("wal") {
            bail!(
                "{} cannot forget safely here (secure_delete {secure}, journal mode {mode}); \
                 memory needs a local filesystem",
                path.display()
            );
        }
    }
    Ok(Some(conn))
}

fn fact_columns(extra: &str) -> String {
    format!(
        "uid TEXT PRIMARY KEY,
         text TEXT NOT NULL,
         kind TEXT NOT NULL,
         source_chat TEXT NOT NULL CHECK (length(trim(source_chat)) > 0),
         source_from INTEGER NOT NULL,
         source_to INTEGER NOT NULL CHECK (source_to >= source_from),
         learned_by TEXT NOT NULL,
         origin TEXT NOT NULL,
         model TEXT NOT NULL,
         valid_from TEXT,
         valid_to TEXT,
         ingested_at TEXT NOT NULL,
         {extra}"
    )
}

fn migrate_memory(conn: &Connection) -> Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version < 1 {
        migrate_memory_v1(conn)?;
    }
    if version < 2 {
        // The writer's ledger (§9.6): how many of a chat's messages it has
        // written up to. Messages are counted as `message` records in the
        // transcript file, which is append-only, so the count never moves
        // under a compaction.
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS written (
                chat TEXT PRIMARY KEY,
                upto INTEGER NOT NULL,
                at TEXT NOT NULL
             );
             PRAGMA user_version = 2;
             COMMIT;",
        )?;
    }
    if version < 3 {
        // The recall index (§9.7): words by FTS5, meaning by vectors kept
        // as blobs (D19). FTS5 keeps its own copy of the text and its index
        // can hold a deleted term until a merge, so `secure-delete` is on:
        // forgetting a record forgets it from the index too (§9.9). Built
        // from every record already there, so a store from before it is
        // searchable at once.
        //
        // The first step that is not idempotent, and per-turn recall makes
        // concurrent writable opens routine: so the write lock is taken
        // first and the version read again under it, and the backfill skips
        // a record already indexed. Run twice, every record would be indexed
        // twice and outrank the one that answered (review of #481).
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
        let now: i64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if now < 3 {
            let mut sql = String::from(
                "CREATE VIRTUAL TABLE IF NOT EXISTS recall_fts USING fts5 (uid UNINDEXED, text);
                 INSERT INTO recall_fts (recall_fts, rank) VALUES ('secure-delete', 1);
                 CREATE TABLE IF NOT EXISTS vectors (uid TEXT PRIMARY KEY, vec BLOB NOT NULL);",
            );
            for t in Table::ALL {
                sql.push_str(&format!(
                    "INSERT INTO recall_fts (uid, text) SELECT uid, text FROM {}
                     WHERE uid NOT IN (SELECT uid FROM recall_fts);",
                    t.sql()
                ));
            }
            tx.execute_batch(&sql)?;
            // Episodes through `episode_words`, the one definition of an
            // episode's text, rather than its raw JSON columns (review of #481).
            let episodes: Vec<(String, String, String, String, String)> = {
                let mut stmt = tx.prepare(
                    "SELECT uid, summary, topics, decisions, open_threads FROM episodes
                     WHERE uid NOT IN (SELECT uid FROM recall_fts)",
                )?;
                let rows = stmt.query_map([], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })?;
                rows.collect::<rusqlite::Result<_>>()?
            };
            for (uid, summary, topics, decisions, open) in episodes {
                let words = words_of(
                    &summary,
                    &unlist(&topics),
                    &unlist(&decisions),
                    &unlist(&open),
                );
                tx.execute(
                    "INSERT INTO recall_fts (uid, text) VALUES (?1, ?2)",
                    params![uid, words],
                )?;
            }
            tx.execute_batch("PRAGMA user_version = 3;")?;
        }
        tx.commit()?;
        debug_assert_eq!(SCHEMA, 3, "a new step goes above, and this moves with it");
    }
    Ok(())
}

fn migrate_memory_v1(conn: &Connection) -> Result<()> {
    let mut sql = String::from(
        "CREATE TABLE IF NOT EXISTS episodes (
            uid TEXT PRIMARY KEY,
            source_chat TEXT NOT NULL CHECK (length(trim(source_chat)) > 0),
            source_from INTEGER NOT NULL,
            source_to INTEGER NOT NULL CHECK (source_to >= source_from),
            started_at TEXT,
            ended_at TEXT,
            summary TEXT NOT NULL,
            topics TEXT NOT NULL DEFAULT '[]',
            decisions TEXT NOT NULL DEFAULT '[]',
            open_threads TEXT NOT NULL DEFAULT '[]',
            scenario TEXT,
            files TEXT NOT NULL DEFAULT '[]',
            origin TEXT NOT NULL,
            model TEXT NOT NULL,
            ingested_at TEXT NOT NULL,
            invalidated_at TEXT,
            status TEXT NOT NULL,
            pinned INTEGER NOT NULL DEFAULT 0
         );
         CREATE INDEX IF NOT EXISTS episodes_chat ON episodes(source_chat);",
    );
    for t in Table::ALL {
        let name = t.sql();
        sql.push_str(&format!(
            "CREATE TABLE IF NOT EXISTS {name} ({cols});
             CREATE INDEX IF NOT EXISTS {name}_chat ON {name}(source_chat);",
            cols = fact_columns(
                "invalidated_at TEXT,
                 status TEXT NOT NULL,
                 pinned INTEGER NOT NULL DEFAULT 0,
                 replaces TEXT"
            ),
        ));
    }
    sql.push_str("PRAGMA user_version = 1;");
    conn.execute_batch(&format!("BEGIN; {sql} COMMIT;"))?;
    Ok(())
}

fn migrate_shared(conn: &Connection) -> Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version >= 1 {
        return Ok(());
    }
    conn.execute_batch(&format!(
        "BEGIN;
         CREATE TABLE IF NOT EXISTS shared_facts ({cols});
         CREATE INDEX IF NOT EXISTS shared_chat ON shared_facts(learned_by, source_chat);
         CREATE INDEX IF NOT EXISTS shared_audience ON shared_facts(audience);
         PRAGMA user_version = 1;
         COMMIT;",
        cols = fact_columns(
            "audience TEXT NOT NULL,
             from_uid TEXT NOT NULL,
             from_table TEXT NOT NULL,
             shared_at TEXT NOT NULL,
             UNIQUE (learned_by, from_uid, audience)"
        ),
    ))?;
    Ok(())
}

/// After a delete: fold each write-ahead log back and truncate it, so the
/// deleted text does not survive in the log's copy of the page.
///
/// A checkpoint another reader blocks does not fail — SQLite reports it as
/// `busy = 1` in the returned row — so the row is read, and an untruncated
/// log is an error: reporting it as done would be the silently-degrading
/// guard. Every connection is tried before reporting, and callers run their
/// deletes first, so a busy log never stops a delete from happening; the
/// next delete's checkpoint clears what this one could not (review of #463).
fn scrub(conns: &[&Connection]) -> Result<()> {
    let mut busy = 0;
    for conn in conns {
        let b: i64 = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
        busy += usize::from(b != 0);
    }
    if busy > 0 {
        bail!(
            "done, but the write-ahead log could not be truncated while a chat \
             or the page was reading memory; the old text may stay in the log \
             until the next delete"
        );
    }
    Ok(())
}

// ── memory.db ───────────────────────────────────────────────────────────

/// One persona's `memory.db`.
pub struct Memory {
    conn: Connection,
    /// The store it sits in: where `shared.db` is, which a correction or a
    /// withdrawal must reach.
    store_dir: PathBuf,
    persona: String,
    writable: bool,
}

const FACT_SELECT: &str = "uid, text, kind, source_chat, source_from, source_to, learned_by,
     origin, model, valid_from, valid_to, ingested_at, invalidated_at, status, pinned, replaces";

const EPISODE_SELECT: &str = "uid, source_chat, source_from, source_to, started_at, ended_at,
     summary, topics, decisions, open_threads, scenario, files, origin, model, ingested_at,
     invalidated_at, status, pinned";

fn source_of(r: &Row, at: usize) -> rusqlite::Result<Source> {
    Ok(Source {
        chat: r.get(at)?,
        from: r.get(at + 1)?,
        to: r.get(at + 2)?,
    })
}

fn fact_of(table: Table, r: &Row) -> rusqlite::Result<Fact> {
    Ok(Fact {
        uid: r.get(0)?,
        table,
        text: r.get(1)?,
        kind: unwire(&r.get::<_, String>(2)?),
        source: source_of(r, 3)?,
        learned_by: r.get(6)?,
        origin: unwire(&r.get::<_, String>(7)?),
        model: r.get(8)?,
        valid_from: r.get(9)?,
        valid_to: r.get(10)?,
        ingested_at: r.get(11)?,
        invalidated_at: r.get(12)?,
        status: unwire(&r.get::<_, String>(13)?),
        pinned: r.get(14)?,
        replaces: r.get(15)?,
    })
}

fn episode_of(r: &Row) -> rusqlite::Result<Episode> {
    Ok(Episode {
        uid: r.get(0)?,
        source: source_of(r, 1)?,
        started_at: r.get(4)?,
        ended_at: r.get(5)?,
        summary: r.get(6)?,
        topics: unlist(&r.get::<_, String>(7)?),
        decisions: unlist(&r.get::<_, String>(8)?),
        open_threads: unlist(&r.get::<_, String>(9)?),
        scenario: r.get(10)?,
        files: unlist(&r.get::<_, String>(11)?),
        origin: unwire(&r.get::<_, String>(12)?),
        model: r.get(13)?,
        ingested_at: r.get(14)?,
        invalidated_at: r.get(15)?,
        status: unwire(&r.get::<_, String>(16)?),
        pinned: r.get(17)?,
    })
}

/// Reciprocal-rank fusion's constant, as `persona::search` uses it.
const RRF_K: f32 = 60.0;

/// The least cosine a meaning-only hit needs to be recalled. A guess until
/// measured on real persona memory: below it, the nearest record to "ok
/// sounds good" is noise, and a fold on every turn would carry it in.
pub const MIN_COSINE: f32 = 0.5;

/// What a search may return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recallable {
    Episodes,
    Facts(Table),
}

/// One record a search found, as a line for the model.
#[derive(Debug, Clone, PartialEq)]
pub struct Recollection {
    pub uid: String,
    pub kind: Recallable,
    pub text: String,
    pub date: String,
    pub origin: Origin,
}

/// Words that say nothing about what a message is about. With them in, a
/// message would match any record on "the" — and an incidental match is
/// not only noise: recalling an approved record from outside arms the
/// whole chat untrusted for good (review of #481). A short list on purpose:
/// what matters is that no common word can be the only thing two texts
/// share.
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "are", "but", "not", "you", "your", "yours", "all", "any", "can", "had",
    "has", "have", "her", "hers", "him", "his", "how", "its", "may", "our", "ours", "out", "she",
    "they", "them", "their", "theirs", "this", "that", "these", "those", "was", "were", "what",
    "when", "where", "which", "who", "whom", "why", "will", "with", "would", "could", "should",
    "about", "after", "again", "also", "been", "before", "being", "did", "does", "doing", "done",
    "from", "into", "just", "more", "most", "much", "only", "over", "same", "some", "such", "than",
    "then", "there", "here", "very", "well", "yes", "yeah", "okay", "thanks", "thank", "please",
    "really", "today", "tonight", "now", "still", "too", "one", "get", "got", "going", "know",
    "think", "like", "want", "let", "say", "said",
];

/// A message as the recall index reads it: its content words — three
/// letters or more, not a stopword — each quoted so nothing in them is FTS
/// syntax, any of them. `None` when it has none, and then nothing is
/// recalled by words.
fn recall_words(query: &str) -> Option<String> {
    let mut words: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 3 && !STOPWORDS.contains(&w.as_str()))
        .map(|w| format!("\"{w}\""))
        .collect();
    words.sort();
    words.dedup();
    (!words.is_empty()).then(|| words.join(" OR "))
}

/// An episode's words — what the index holds and what is embedded. The one
/// definition, so a record backfilled and one inserted live are the same
/// text, and a vector is made from prose, not from JSON (review of #481).
fn episode_words(e: &Episode) -> String {
    words_of(&e.summary, &e.topics, &e.decisions, &e.open_threads)
}

fn words_of(summary: &str, topics: &[String], decisions: &[String], open: &[String]) -> String {
    let mut out = summary.trim().to_owned();
    for part in [topics, decisions, open] {
        if !part.is_empty() {
            out.push(' ');
            out.push_str(&part.join("; "));
        }
    }
    out
}

/// Which records a listing returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    /// What a chat may be given: active only.
    Recallable,
    /// Everything, for the owner's curation.
    All,
}

impl Filter {
    /// The `WHERE` clause and its binding — the status bound through its
    /// serde name, so the compiler keeps the two in step.
    fn sql(self) -> (&'static str, Vec<String>) {
        match self {
            Filter::Recallable => (" WHERE status = ?1", vec![wire(&Status::Active)]),
            Filter::All => ("", Vec::new()),
        }
    }
}

/// A short id as the owner typed it, or refused: at least four hex digits,
/// so nothing shorter matches a record by accident.
fn short_id(short: &str) -> Result<String> {
    let short = short.trim().to_ascii_lowercase();
    if short.len() < 4 || !short.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("`{short}` is not a memory id (at least four hex digits)");
    }
    Ok(short)
}

impl Memory {
    /// The persona's memory, created (owner-only) if it is not there yet.
    /// For the writer and the owner's doors — never for an incognito chat,
    /// which writes nothing (§9.9, D7).
    pub fn open(store_dir: &Path, persona: &str) -> Result<Memory> {
        validate_persona_name(persona)?;
        let pdir = store_dir.join(persona);
        if !pdir.join("persona.toml").is_file() {
            bail!("no persona named `{persona}`");
        }
        let conn = connect(&pdir.join(MEMORY_DB), true)?.expect("created");
        migrate_memory(&conn)?;
        Ok(Memory {
            conn,
            store_dir: store_dir.to_path_buf(),
            persona: persona.into(),
            writable: true,
        })
    }

    /// The persona's memory for an edit to a record that must already be
    /// there — the owner's doors. Refuses rather than creating an empty
    /// database just to report that the id is not in it.
    pub fn open_to_edit(store_dir: &Path, persona: &str) -> Result<Memory> {
        validate_persona_name(persona)?;
        if !store_dir.join(persona).join(MEMORY_DB).is_file() {
            bail!("{persona} remembers nothing yet");
        }
        Memory::open(store_dir, persona)
    }

    /// The persona's memory if it has any, read-only and never created —
    /// what recall and an incognito chat use. `None` is "nothing remembered
    /// yet", which is not an error.
    pub fn open_existing(store_dir: &Path, persona: &str) -> Result<Option<Memory>> {
        validate_persona_name(persona)?;
        let Some(conn) = connect(&store_dir.join(persona).join(MEMORY_DB), false)? else {
            return Ok(None);
        };
        Ok(Some(Memory {
            conn,
            store_dir: store_dir.to_path_buf(),
            persona: persona.into(),
            writable: false,
        }))
    }

    pub fn persona(&self) -> &str {
        &self.persona
    }

    /// Whether the file is a database this can read at all. SQLite opens
    /// lazily, so a file that is not one opens without error and fails at
    /// the first query; a reader that wants to degrade per store asks first.
    pub fn readable(&self) -> Result<()> {
        self.conn
            .query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))?;
        Ok(())
    }

    fn writable(&self) -> Result<()> {
        if !self.writable {
            bail!("{}'s memory was opened read-only", self.persona);
        }
        Ok(())
    }

    /// Write a fact. Facts about the owner are sorted by how they are known,
    /// structurally: an inferred one goes in [`Table::Inferred`] and nowhere
    /// else (D18).
    pub fn add_fact(&self, table: Table, new: NewFact) -> Result<Fact> {
        self.writable()?;
        check_text(&new.text, MAX_FACT_CHARS, "a fact")?;
        new.source.check()?;
        match (table, new.kind) {
            (Table::User, Kind::Inferred) => {
                bail!("an inferred fact about the owner belongs in the inferred table")
            }
            (Table::Inferred, k) if k != Kind::Inferred => {
                bail!("only inferred facts belong in the inferred table")
            }
            _ => {}
        }
        let fact = Fact {
            uid: uid(),
            table,
            text: new.text.trim().to_owned(),
            kind: new.kind,
            source: new.source,
            learned_by: self.persona.clone(),
            origin: new.origin,
            model: new.model,
            valid_from: new.valid_from,
            valid_to: new.valid_to,
            ingested_at: now(),
            invalidated_at: None,
            status: Status::initial(new.origin),
            pinned: false,
            replaces: None,
        };
        self.insert_fact(&fact)?;
        Ok(fact)
    }

    fn insert_fact(&self, f: &Fact) -> Result<()> {
        self.conn.execute(
            &format!(
                "INSERT INTO {} ({FACT_SELECT}) VALUES
                 (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                f.table.sql()
            ),
            params![
                f.uid,
                f.text,
                wire(&f.kind),
                f.source.chat,
                f.source.from,
                f.source.to,
                f.learned_by,
                wire(&f.origin),
                f.model,
                f.valid_from,
                f.valid_to,
                f.ingested_at,
                f.invalidated_at,
                wire(&f.status),
                f.pinned,
                f.replaces,
            ],
        )?;
        self.index(&f.uid, &f.text)
    }

    /// Put a record's words in the recall index.
    fn index(&self, uid: &str, text: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO recall_fts (uid, text) VALUES (?1, ?2)",
            params![uid, text],
        )?;
        Ok(())
    }

    /// Take records out of the recall index and their vectors with them —
    /// what deleting a record owes the index.
    fn unindex(&self, uids: &[String]) -> Result<()> {
        for uid in uids {
            self.conn
                .execute("DELETE FROM recall_fts WHERE uid = ?1", [uid])?;
            self.conn
                .execute("DELETE FROM vectors WHERE uid = ?1", [uid])?;
        }
        Ok(())
    }

    /// Write an episode.
    pub fn add_episode(&self, new: NewEpisode) -> Result<Episode> {
        self.writable()?;
        check_text(&new.summary, MAX_SUMMARY_CHARS, "an episode's summary")?;
        let source = new
            .source
            .ok_or_else(|| anyhow!("an episode needs the chat it came from"))?;
        source.check()?;
        let ep = Episode {
            uid: uid(),
            source,
            started_at: new.started_at,
            ended_at: new.ended_at,
            summary: new.summary.trim().to_owned(),
            topics: new.topics,
            decisions: new.decisions,
            open_threads: new.open_threads,
            scenario: new.scenario,
            files: new.files,
            origin: new.origin,
            model: new.model,
            ingested_at: now(),
            invalidated_at: None,
            status: Status::initial(new.origin),
            pinned: false,
        };
        self.conn.execute(
            &format!(
                "INSERT INTO episodes ({EPISODE_SELECT}) VALUES
                 (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)"
            ),
            params![
                ep.uid,
                ep.source.chat,
                ep.source.from,
                ep.source.to,
                ep.started_at,
                ep.ended_at,
                ep.summary,
                list(&ep.topics),
                list(&ep.decisions),
                list(&ep.open_threads),
                ep.scenario,
                list(&ep.files),
                wire(&ep.origin),
                ep.model,
                ep.ingested_at,
                ep.invalidated_at,
                wire(&ep.status),
                ep.pinned,
            ],
        )?;
        self.index(&ep.uid, &episode_words(&ep))?;
        Ok(ep)
    }

    /// Facts in one table: pinned first, then newest first.
    pub fn facts(&self, table: Table, filter: Filter) -> Result<Vec<Fact>> {
        let (filter_sql, binds) = filter.sql();
        let sql = format!(
            "SELECT {FACT_SELECT} FROM {}{filter_sql} ORDER BY pinned DESC, ingested_at DESC, uid",
            table.sql()
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(binds), |r| fact_of(table, r))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Episodes, pinned first, then the most recent chat first.
    pub fn episodes(&self, filter: Filter) -> Result<Vec<Episode>> {
        let (filter_sql, binds) = filter.sql();
        let sql = format!(
            "SELECT {EPISODE_SELECT} FROM episodes{filter_sql}
             ORDER BY pinned DESC, coalesce(ended_at, started_at, ingested_at) DESC, uid"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(binds), episode_of)?;
        let mut out: Vec<Episode> = rows.collect::<rusqlite::Result<_>>()?;
        // By when it happened, which the SQL cannot see for a stretch dated
        // by its chat's id; stable, so the uid order breaks ties.
        out.sort_by_cached_key(|e| (std::cmp::Reverse(e.pinned), std::cmp::Reverse(e.said_at())));
        Ok(out)
    }

    /// One fact, wherever it lives.
    pub fn fact(&self, uid: &str) -> Result<Option<Fact>> {
        for t in Table::ALL {
            let found = self
                .conn
                .query_row(
                    &format!("SELECT {FACT_SELECT} FROM {} WHERE uid = ?1", t.sql()),
                    [uid],
                    |r| fact_of(t, r),
                )
                .optional()?;
            if found.is_some() {
                return Ok(found);
            }
        }
        Ok(None)
    }

    /// The full id a short one names — what the owner types after reading
    /// `mecha persona memory show`. A prefix two records share is refused,
    /// never guessed.
    pub fn resolve(&self, short: &str) -> Result<String> {
        let short = short_id(short)?;
        let mut hits = Vec::new();
        let tables = std::iter::once("episodes").chain(Table::ALL.map(Table::sql));
        for t in tables {
            let mut stmt = self.conn.prepare(&format!(
                "SELECT uid FROM {t} WHERE substr(uid, 1, ?2) = ?1"
            ))?;
            let rows = stmt.query_map(params![short, short.len()], |r| r.get::<_, String>(0))?;
            hits.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        match hits.len() {
            1 => Ok(hits.remove(0)),
            0 => bail!("{} remembers nothing with id `{short}`", self.persona),
            n => bail!("`{short}` matches {n} records; give more of the id"),
        }
    }

    /// Which table holds `uid`: `episodes`, or a fact table.
    fn locate(&self, uid: &str) -> Result<&'static str> {
        let tables = std::iter::once("episodes").chain(Table::ALL.map(Table::sql));
        for t in tables {
            let hit: Option<i64> = self
                .conn
                .query_row(&format!("SELECT 1 FROM {t} WHERE uid = ?1"), [uid], |r| {
                    r.get(0)
                })
                .optional()?;
            if hit.is_some() {
                return Ok(t);
            }
        }
        bail!("{} remembers nothing with id `{uid}`", self.persona)
    }

    fn status_in(&self, table: &str, uid: &str) -> Result<Status> {
        let s: String = self.conn.query_row(
            &format!("SELECT status FROM {table} WHERE uid = ?1"),
            [uid],
            |r| r.get(0),
        )?;
        Ok(unwire(&s))
    }

    /// The owner approves a candidate: it may now be recalled. Its origin is
    /// kept, so recalling it re-arms the taint it was written under (§9.6).
    /// Only a candidate: a withdrawn record stays withdrawn — approving it
    /// would erase when it was withdrawn and, for a corrected fact, make the
    /// old wording and its correction both recallable.
    pub fn approve(&self, uid: &str) -> Result<()> {
        self.writable()?;
        let table = self.locate(uid)?;
        match self.status_in(table, uid)? {
            Status::Candidate => {}
            Status::Active => bail!("that record is already recalled"),
            Status::Invalidated => bail!("that record was withdrawn, not waiting on you"),
        }
        self.conn.execute(
            &format!("UPDATE {table} SET status = ?2 WHERE uid = ?1"),
            params![uid, wire(&Status::Active)],
        )?;
        Ok(())
    }

    /// Withdraw a record without deleting it, and stop sharing it: a fact
    /// withdrawn here is not one other personas should still read. The first
    /// withdrawal's time is kept. Returns the shared copies removed.
    pub fn invalidate(&self, uid: &str) -> Result<usize> {
        self.writable()?;
        let table = self.locate(uid)?;
        self.conn.execute(
            &format!(
                "UPDATE {table} SET status = ?2, invalidated_at = coalesce(invalidated_at, ?3)
                 WHERE uid = ?1"
            ),
            params![uid, wire(&Status::Invalidated), now()],
        )?;
        let Some(shared) = self.shared()? else {
            return Ok(0);
        };
        let n = shared.unshare_from(&self.persona, &[uid.to_owned()])?;
        scrub(&[&shared.conn])?;
        Ok(n)
    }

    /// `shared.db` for writing, if anything was ever shared — never created
    /// here, since nothing in this file adds to it.
    fn shared(&self) -> Result<Option<Shared>> {
        if Shared::path(&self.store_dir).is_file() {
            Ok(Some(Shared::open(&self.store_dir)?))
        } else {
            Ok(None)
        }
    }

    /// `uid` and every row it corrected, newest first — so a copy shared
    /// before a correction is still found from the row the owner sees.
    fn lineage(&self, uid: &str) -> Result<Vec<String>> {
        let mut out = vec![uid.to_owned()];
        while let Some(f) = self.fact(out.last().expect("never empty"))? {
            match f.replaces {
                // A cycle can only be hand-made; stop rather than spin.
                Some(prev) if !out.contains(&prev) => out.push(prev),
                _ => break,
            }
        }
        Ok(out)
    }

    /// Pin or unpin: a pinned record is recalled first.
    pub fn pin(&self, uid: &str, pinned: bool) -> Result<()> {
        self.writable()?;
        let table = self.locate(uid)?;
        self.conn.execute(
            &format!("UPDATE {table} SET pinned = ?2 WHERE uid = ?1"),
            params![uid, pinned],
        )?;
        Ok(())
    }

    /// The owner's correction (§9.8): the old row is invalidated and a new,
    /// owner-origin one replaces it. The source is kept — it is still the
    /// chat the fact came from, so forgetting that chat forgets the
    /// correction too. A shared copy follows: it carries the new wording and
    /// points at the new row, so other personas never read what the owner
    /// replaced, and forgetting the new row finds it.
    pub fn correct(&self, uid: &str, text: &str) -> Result<Fact> {
        self.writable()?;
        check_text(text, MAX_FACT_CHARS, "a fact")?;
        let old = self
            .fact(uid)?
            .ok_or_else(|| anyhow!("{} has no fact with id `{uid}`", self.persona))?;
        // Only a live row is corrected: correcting a withdrawn one would
        // re-stamp when it was withdrawn and grow a second live replacement
        // of the same fact — the pair `approve` refuses (review of #463).
        if old.status == Status::Invalidated {
            bail!("that record was withdrawn; correct the row that replaced it, or add the fact again");
        }
        let tx = self.conn.unchecked_transaction()?;
        let stamp = now();
        tx.execute(
            &format!(
                "UPDATE {} SET status = ?3, invalidated_at = coalesce(invalidated_at, ?2)
                 WHERE uid = ?1",
                old.table.sql()
            ),
            params![uid, stamp, wire(&Status::Invalidated)],
        )?;
        let new = Fact {
            uid: self::uid(),
            text: text.trim().to_owned(),
            origin: Origin::Owner,
            ingested_at: stamp,
            invalidated_at: None,
            status: Status::Active,
            replaces: Some(old.uid.clone()),
            ..old
        };
        self.insert_fact(&new)?;
        tx.commit()?;
        // Two files cannot share one commit, so a failure here is said, not
        // swallowed; `forget` walks `replaces` either way, so a copy left
        // behind can never outlive the fact it copied. Each failure is
        // worded where it happens (review of #463): a failed update leaves
        // the copy stale, a busy log leaves the copy right and the old
        // wording only in the log.
        if let Some(shared) = self.shared()? {
            shared.follow_correction(&self.persona, &old.uid, &new)?;
        }
        Ok(new)
    }

    /// Delete one record outright — the only edit that removes text.
    /// Private, and unscrubbed: it does not reach `shared.db` or the log, so
    /// [`forget`] — which walks `replaces`, drops the copies and scrubs both
    /// files — is the only door.
    fn delete(&self, uid: &str) -> Result<()> {
        self.writable()?;
        let table = self.locate(uid)?;
        self.unindex(&[uid.to_owned()])?;
        self.conn
            .execute(&format!("DELETE FROM {table} WHERE uid = ?1"), [uid])?;
        Ok(())
    }

    /// Delete every record whose source is `chat`; (episodes, facts) removed.
    ///
    /// **The writer's ledger row stays, on purpose.** The transcript is still
    /// in `sessions/`; with the row gone, the next nightly would read the chat
    /// from turn 0 and remember again everything the owner just forgot. A
    /// tidy-looking cleanup of that row is a privacy regression —
    /// `a_forgotten_chat_is_not_remembered_again` holds it (review of #468).
    fn forget_chat(&self, chat: &str) -> Result<(usize, usize)> {
        self.writable()?;
        let tx = self.conn.unchecked_transaction()?;
        // Out of the index first, while the rows still say which they are.
        let mut uids: Vec<String> = Vec::new();
        for t in std::iter::once("episodes").chain(Table::ALL.map(Table::sql)) {
            let mut stmt = tx.prepare(&format!("SELECT uid FROM {t} WHERE source_chat = ?1"))?;
            let rows = stmt.query_map([chat], |r| r.get::<_, String>(0))?;
            uids.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        self.unindex(&uids)?;
        let episodes = tx.execute("DELETE FROM episodes WHERE source_chat = ?1", [chat])?;
        let mut facts = 0;
        for t in Table::ALL {
            facts += tx.execute(
                &format!("DELETE FROM {} WHERE source_chat = ?1", t.sql()),
                [chat],
            )?;
        }
        tx.commit()?;
        Ok((episodes, facts))
    }

    /// How many of `chat`'s messages the writer has written up to — `0` for
    /// a chat it has never read (§9.6), and on a store from before the
    /// ledger, which a read-only handle does not upgrade.
    pub fn written_upto(&self, chat: &str) -> Result<u32> {
        let ledger: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'written'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if ledger.is_none() {
            return Ok(0);
        }
        Ok(self
            .conn
            .query_row("SELECT upto FROM written WHERE chat = ?1", [chat], |r| {
                r.get(0)
            })
            .optional()?
            .unwrap_or(0))
    }

    /// Write one stretch of a chat: `f`'s records and the ledger's advance
    /// from `from` to `upto`, in one transaction — or nothing, when another
    /// writer advanced the ledger past `from` while this one was asking the
    /// model (`Ok(None)`). The model call happens before this, with no lock
    /// held; the check here is what keeps two writers from recording one
    /// stretch twice.
    ///
    /// The transaction is `memory.db`'s. A withdrawal inside `f` also drops
    /// the fact's shared copies from `shared.db`, which it does not cover: if
    /// a later step fails and this rolls back, those copies stay gone — two
    /// files cannot share one commit (review of #468).
    pub fn write_stretch<T>(
        &self,
        chat: &str,
        from: u32,
        upto: u32,
        f: impl FnOnce(&Memory) -> Result<T>,
    ) -> Result<Option<T>> {
        self.writable()?;
        if upto < from {
            bail!("a stretch cannot end before it starts");
        }
        // IMMEDIATE: the write lock is taken before the ledger is read, so a
        // second writer waits here and then sees the first one's advance,
        // rather than both reading `from` and racing to write.
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        if self.written_upto(chat)? != from {
            return Ok(None);
        }
        let out = f(self)?;
        tx.execute(
            "INSERT INTO written (chat, upto, at) VALUES (?1, ?2, ?3)
             ON CONFLICT (chat) DO UPDATE SET upto = excluded.upto, at = excluded.at",
            params![chat, upto, now()],
        )?;
        tx.commit()?;
        Ok(Some(out))
    }

    /// The writer's update (§9.3): `old` is withdrawn and `new` replaces it,
    /// pointing back — Mem0's update as an invalidation plus an addition, so
    /// what was believed before stays answerable. Unlike [`Self::correct`]
    /// this opens no transaction of its own, so it runs inside
    /// [`Self::write_stretch`]; the new row's origin is the writer's, not the
    /// owner's.
    pub(super) fn supersede(&self, old: &Fact, new: NewFact) -> Result<Fact> {
        let replacement = self.add_fact(old.table, new)?;
        self.invalidate(&old.uid)?;
        self.conn.execute(
            &format!(
                "UPDATE {} SET replaces = ?2 WHERE uid = ?1",
                old.table.sql()
            ),
            params![replacement.uid, old.uid],
        )?;
        Ok(Fact {
            replaces: Some(old.uid.clone()),
            ..replacement
        })
    }

    /// Active records with no vector yet — episodes, then each fact table —
    /// at most `limit`: what the writer embeds after writing.
    pub fn unembedded(&self, limit: usize) -> Result<Vec<(String, String)>> {
        let embedded: std::collections::HashSet<String> = {
            let mut stmt = self.conn.prepare("SELECT uid FROM vectors")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut out: Vec<(String, String)> = self
            .episodes(Filter::Recallable)?
            .into_iter()
            .filter(|e| !embedded.contains(&e.uid))
            .map(|e| {
                let words = episode_words(&e);
                (e.uid, words)
            })
            .collect();
        for t in Table::ALL.map(Table::sql) {
            let mut stmt = self.conn.prepare(&format!(
                "SELECT uid, text FROM {t} WHERE status = ?1
                 AND uid NOT IN (SELECT uid FROM vectors) ORDER BY ingested_at"
            ))?;
            let rows = stmt.query_map([wire(&Status::Active)], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            out.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        out.truncate(limit);
        Ok(out)
    }

    /// Store vectors made by the embeddings server, by record.
    pub fn store_vectors(&self, vectors: &[(String, Vec<f32>)]) -> Result<()> {
        self.writable()?;
        let tx = self.conn.unchecked_transaction()?;
        for (uid, v) in vectors {
            tx.execute(
                "INSERT INTO vectors (uid, vec) VALUES (?1, ?2)
                 ON CONFLICT (uid) DO UPDATE SET vec = excluded.vec",
                params![uid, crate::embed::to_blob(v)],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The records of `kinds` that best answer `query` (§9.7): words by FTS5
    /// and, given `qvec`, meaning by cosine, fused by reciprocal rank with
    /// recency as a third ranking — "weighted toward recent". Active records
    /// only; `skip` names records the caller already has.
    ///
    /// A meaning hit needs `MIN_COSINE`: a vector search ranks *something*
    /// first whatever is asked, and a fold on every turn would otherwise
    /// carry the nearest noise into the chat. Only vectors of the query's
    /// length count, and only when at least half the candidates have one —
    /// d7's rule in `persona::search`, for d7's reason. `bool` is whether
    /// meaning took part.
    pub fn recall_search(
        &self,
        query: &str,
        qvec: Option<&[f32]>,
        kinds: &[Recallable],
        skip: &dyn Fn(&str) -> bool,
        k: usize,
    ) -> Result<(Vec<Recollection>, bool)> {
        if !self.has_index()? {
            return Ok((Vec::new(), false));
        }
        let mut pool: HashMap<String, Recollection> = HashMap::new();
        for kind in kinds {
            match kind {
                Recallable::Episodes => {
                    for e in self.episodes(Filter::Recallable)? {
                        let date = e.said_at();
                        let mut text = e.summary.clone();
                        if !e.open_threads.is_empty() {
                            text.push_str(&format!(" (Left open: {}.)", e.open_threads.join("; ")));
                        }
                        pool.insert(
                            e.uid.clone(),
                            Recollection {
                                uid: e.uid,
                                kind: *kind,
                                text,
                                date,
                                origin: e.origin,
                            },
                        );
                    }
                }
                Recallable::Facts(t) => {
                    for f in self.facts(*t, Filter::Recallable)? {
                        let date = f.said_at();
                        pool.insert(
                            f.uid.clone(),
                            Recollection {
                                uid: f.uid,
                                kind: *kind,
                                text: f.text,
                                date,
                                origin: f.origin,
                            },
                        );
                    }
                }
            }
        }
        pool.retain(|_, r| !skip(&r.text));
        if pool.is_empty() {
            return Ok((Vec::new(), false));
        }

        let mut scores: HashMap<String, f32> = HashMap::new();
        let mut candidates: Vec<String> = Vec::new();
        let rrf = |rank: usize| 1.0 / (RRF_K + rank as f32 + 1.0);

        if let Some(fts) = recall_words(query) {
            let mut stmt = self.conn.prepare(
                "SELECT uid FROM recall_fts WHERE recall_fts MATCH ?1
                 ORDER BY bm25(recall_fts)",
            )?;
            let uids = stmt.query_map([fts], |r| r.get::<_, String>(0))?;
            let mut rank = 0;
            for uid in uids {
                let uid = uid?;
                if pool.contains_key(&uid) {
                    *scores.entry(uid.clone()).or_default() += rrf(rank);
                    candidates.push(uid);
                    rank += 1;
                }
            }
        }

        let mut by_meaning = false;
        if let Some(q) = qvec {
            let mut stmt = self.conn.prepare("SELECT uid, vec FROM vectors")?;
            let rows = stmt.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
            })?;
            let mut embedded = 0usize;
            let mut ranked: Vec<(String, f32)> = Vec::new();
            for row in rows {
                let (uid, blob) = row?;
                if !pool.contains_key(&uid) {
                    continue;
                }
                let Some(v) = crate::embed::from_blob(&blob).filter(|v| v.len() == q.len()) else {
                    continue;
                };
                embedded += 1;
                let c = crate::embed::cosine(q, &v);
                if c >= MIN_COSINE {
                    ranked.push((uid, c));
                }
            }
            by_meaning = embedded * 2 >= pool.len() && embedded > 0;
            if by_meaning {
                // Equal by meaning, the newer first: otherwise row order would
                // rank the older higher here and cancel the recency ranking
                // below, leaving the tie to the uid.
                ranked.sort_by(|a, b| {
                    b.1.total_cmp(&a.1)
                        .then_with(|| pool[&b.0].date.cmp(&pool[&a.0].date))
                });
                for (rank, (uid, _)) in ranked.into_iter().enumerate() {
                    *scores.entry(uid.clone()).or_default() += rrf(rank);
                    candidates.push(uid);
                }
            }
        }

        // Recency reorders what words or meaning found; it never adds one.
        candidates.sort();
        candidates.dedup();
        let mut by_date: Vec<&String> = candidates.iter().collect();
        by_date.sort_by(|a, b| pool[*b].date.cmp(&pool[*a].date));
        for (rank, uid) in by_date.into_iter().enumerate() {
            *scores.entry(uid.clone()).or_default() += rrf(rank);
        }

        let mut best: Vec<(String, f32)> = scores.into_iter().collect();
        // Equal scores are common — a word rank and a recency rank swapped
        // tie exactly — so the newer wins before the uid, a coin flip, is
        // asked (review of #481).
        best.sort_by(|a, b| {
            b.1.total_cmp(&a.1)
                .then_with(|| pool[&b.0].date.cmp(&pool[&a.0].date))
                .then(a.0.cmp(&b.0))
        });
        Ok((
            best.into_iter()
                .take(k)
                .filter_map(|(uid, _)| pool.remove(&uid))
                .collect(),
            by_meaning,
        ))
    }

    /// Whether this store has the recall index — a read-only handle to a
    /// store from before it does not build one.
    fn has_index(&self) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE name = 'recall_fts'",
                [],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Everything, as JSON lines in a fixed order — what `cat` was for the
    /// file stores, and byte-stable for one database's contents, so an
    /// experiment's condition digest can hash it (§9.10).
    pub fn export(&self) -> Result<String> {
        let mut out = String::new();
        let mut eps = self.episodes(Filter::All)?;
        eps.sort_by(|a, b| a.uid.cmp(&b.uid));
        for e in eps {
            out.push_str(&serde_json::to_string(&serde_json::json!({"episode": e}))?);
            out.push('\n');
        }
        for t in Table::ALL {
            let mut facts = self.facts(t, Filter::All)?;
            facts.sort_by(|a, b| a.uid.cmp(&b.uid));
            for f in facts {
                out.push_str(&serde_json::to_string(&serde_json::json!({"fact": f}))?);
                out.push('\n');
            }
        }
        Ok(out)
    }
}

// ── shared.db ───────────────────────────────────────────────────────────

const SHARED_SELECT: &str = "uid, audience, from_uid, from_table, text, kind, source_chat,
     source_from, source_to, learned_by, origin, model, valid_from, valid_to, ingested_at,
     shared_at";

/// A shared row, or `None` when its audience or table is one this binary
/// cannot read. Such a row is never shown to a persona — an audience it
/// cannot read is not one it can prove a persona belongs to — and it never
/// fails the listing around it: it is counted ([`Listing::unreadable`]).
fn shared_of(r: &Row) -> rusqlite::Result<Option<SharedFact>> {
    let audience: String = r.get(1)?;
    let table: String = r.get(3)?;
    let (Ok(audience), Ok(from_table)) = (
        Audience::from_wire(&audience),
        serde_json::from_value::<Table>(serde_json::Value::String(table)),
    ) else {
        return Ok(None);
    };
    Ok(Some(SharedFact {
        uid: r.get(0)?,
        audience,
        from_uid: r.get(2)?,
        from_table,
        text: r.get(4)?,
        kind: unwire(&r.get::<_, String>(5)?),
        source: source_of(r, 6)?,
        learned_by: r.get(9)?,
        origin: unwire(&r.get::<_, String>(10)?),
        model: r.get(11)?,
        valid_from: r.get(12)?,
        valid_to: r.get(13)?,
        ingested_at: r.get(14)?,
        shared_at: r.get(15)?,
    }))
}

/// Shared facts as listed: the readable ones, and how many were not — an
/// unreadable record is a finding, never an empty store.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Listing {
    pub facts: Vec<SharedFact>,
    pub unreadable: usize,
}

fn listing(rows: impl Iterator<Item = rusqlite::Result<Option<SharedFact>>>) -> Result<Listing> {
    let mut out = Listing::default();
    for row in rows {
        match row? {
            Some(f) => out.facts.push(f),
            None => out.unreadable += 1,
        }
    }
    Ok(out)
}

/// The store's `shared.db`.
pub struct Shared {
    conn: Connection,
    writable: bool,
}

impl Shared {
    pub fn path(store_dir: &Path) -> PathBuf {
        store_dir.join(SHARED_DB)
    }

    /// Open for writing, created owner-only if absent. The owner's doors only.
    pub fn open(store_dir: &Path) -> Result<Shared> {
        let conn = connect(&Self::path(store_dir), true)?.expect("created");
        migrate_shared(&conn)?;
        Ok(Shared {
            conn,
            writable: true,
        })
    }

    /// Read-only and never created; `None` when nothing was ever shared.
    pub fn open_existing(store_dir: &Path) -> Result<Option<Shared>> {
        Ok(connect(&Self::path(store_dir), false)?.map(|conn| Shared {
            conn,
            writable: false,
        }))
    }

    fn writable(&self) -> Result<()> {
        if !self.writable {
            bail!("shared.db was opened read-only");
        }
        Ok(())
    }

    /// Whether the file is a database this can read at all —
    /// [`Memory::readable`]'s question, for the same lazily-opened reason.
    pub fn readable(&self) -> Result<()> {
        self.conn
            .query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))?;
        Ok(())
    }

    /// The owner shares a fact about themselves (§9.5): a copy, keeping its
    /// source and who learned it. Only an active fact about the owner can be
    /// shared — a persona's own canon is its own, and a candidate has not
    /// been approved. `groups` is the store's declared groups: a group that
    /// does not exist is refused by name rather than shared into nowhere.
    pub fn share(
        &self,
        fact: &Fact,
        audience: Audience,
        declared: &[String],
    ) -> Result<SharedFact> {
        self.writable()?;
        if !fact.table.is_about_owner() {
            bail!("only facts about you can be shared; this is the persona's own");
        }
        if fact.status != Status::Active {
            bail!("approve this fact before sharing it");
        }
        if let Audience::Group(g) = &audience {
            validate_name(g)?;
            if !declared.iter().any(|d| d == g) {
                bail!("no group named `{g}` in groups.toml");
            }
        }
        let shared = SharedFact {
            uid: uid(),
            audience,
            from_uid: fact.uid.clone(),
            from_table: fact.table,
            text: fact.text.clone(),
            kind: fact.kind,
            source: fact.source.clone(),
            learned_by: fact.learned_by.clone(),
            origin: fact.origin,
            model: fact.model.clone(),
            valid_from: fact.valid_from.clone(),
            valid_to: fact.valid_to.clone(),
            ingested_at: fact.ingested_at.clone(),
            shared_at: now(),
        };
        let inserted = self.conn.execute(
            &format!(
                "INSERT OR IGNORE INTO shared_facts ({SHARED_SELECT}) VALUES
                 (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)"
            ),
            params![
                shared.uid,
                shared.audience.wire(),
                shared.from_uid,
                wire(&shared.from_table),
                shared.text,
                wire(&shared.kind),
                shared.source.chat,
                shared.source.from,
                shared.source.to,
                shared.learned_by,
                wire(&shared.origin),
                shared.model,
                shared.valid_from,
                shared.valid_to,
                shared.ingested_at,
                shared.shared_at,
            ],
        )?;
        if inserted == 0 {
            bail!("already shared with {}", shared.audience.wire());
        }
        Ok(shared)
    }

    /// The shared facts a persona in `groups` may read: everyone's, and its
    /// groups'. **The one query that separates personas** (§9.10) — a
    /// persona outside a group sees none of that group's rows.
    pub fn visible_to(&self, groups: &[String]) -> Result<Listing> {
        let mut audiences = vec![Audience::Everyone.wire()];
        audiences.extend(groups.iter().map(|g| Audience::Group(g.clone()).wire()));
        let marks = vec!["?"; audiences.len()].join(", ");
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SHARED_SELECT} FROM shared_facts WHERE audience IN ({marks})
             ORDER BY shared_at DESC, uid"
        ))?;
        let rows = stmt.query_map(rusqlite::params_from_iter(audiences.iter()), shared_of)?;
        listing(rows)
    }

    /// Everything shared, for the owner.
    pub fn all(&self) -> Result<Listing> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SHARED_SELECT} FROM shared_facts ORDER BY shared_at DESC, uid"
        ))?;
        let rows = stmt.query_map([], shared_of)?;
        listing(rows)
    }

    /// The full id of a shared copy, from the table itself — so a copy this
    /// binary cannot read can still be named and unshared.
    pub fn resolve(&self, short: &str) -> Result<String> {
        let short = short_id(short)?;
        let mut stmt = self
            .conn
            .prepare("SELECT uid FROM shared_facts WHERE substr(uid, 1, ?2) = ?1")?;
        let mut hits = stmt
            .query_map(params![short, short.len()], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        match hits.len() {
            1 => Ok(hits.remove(0)),
            0 => bail!("nothing shared with id `{short}`"),
            n => bail!("`{short}` matches {n} shared facts; give more of the id"),
        }
    }

    /// Re-point the copies of a corrected fact at its replacement, in the
    /// owner's wording.
    fn follow_correction(&self, persona: &str, old_uid: &str, new: &Fact) -> Result<usize> {
        self.writable()?;
        let n = self
            .conn
            .execute(
                "UPDATE shared_facts SET from_uid = ?3, text = ?4, origin = ?5
                 WHERE learned_by = ?1 AND from_uid = ?2",
                params![persona, old_uid, new.uid, new.text, wire(&new.origin)],
            )
            .context(
                "corrected, but a shared copy still has the old wording — \
                 `mecha persona memory shared` lists it",
            )?;
        // The old wording was overwritten in place: scrub it from the log.
        scrub(&[&self.conn]).context("corrected, and the shared copy follows")?;
        Ok(n)
    }

    /// Stop sharing one copy. The learner's own row is untouched.
    pub fn unshare(&self, uid: &str) -> Result<()> {
        self.writable()?;
        if self
            .conn
            .execute("DELETE FROM shared_facts WHERE uid = ?1", [uid])?
            == 0
        {
            bail!("nothing shared with id `{uid}`");
        }
        scrub(&[&self.conn])
    }

    /// Drop every copy of the learned facts `from_uids` — what forgetting
    /// or withdrawing one in the learner's memory owes the shared file.
    fn unshare_from(&self, persona: &str, from_uids: &[String]) -> Result<usize> {
        self.writable()?;
        let tx = self.conn.unchecked_transaction()?;
        let mut n = 0;
        for from_uid in from_uids {
            n += tx.execute(
                "DELETE FROM shared_facts WHERE learned_by = ?1 AND from_uid = ?2",
                [persona, from_uid.as_str()],
            )?;
        }
        tx.commit()?;
        Ok(n)
    }

    fn forget_chat(&self, persona: &str, chat: &str) -> Result<usize> {
        self.writable()?;
        Ok(self.conn.execute(
            "DELETE FROM shared_facts WHERE learned_by = ?1 AND source_chat = ?2",
            [persona, chat],
        )?)
    }
}

// ── forgetting across both files ────────────────────────────────────────

/// Forget a persona chat in memory (§9.9): every row whose source is it, in
/// the persona's `memory.db` and in `shared.db`, in one pass. Chat ids are
/// per persona, so the shared side matches on who learned it too.
///
/// A store with no memory yet has nothing to forget, and says so with zeros —
/// the call never creates a database just to delete from it.
pub fn forget_chat(store_dir: &Path, persona: &str, chat: &str) -> Result<Forgotten> {
    validate_persona_name(persona)?;
    if chat.trim().is_empty() {
        bail!("which chat?");
    }
    let mut out = Forgotten::default();
    let memory = if store_dir.join(persona).join(MEMORY_DB).is_file() {
        let m = Memory::open(store_dir, persona)?;
        (out.episodes, out.facts) = m.forget_chat(chat)?;
        Some(m)
    } else {
        None
    };
    let shared = if Shared::path(store_dir).is_file() {
        let s = Shared::open(store_dir)?;
        out.shared = s.forget_chat(persona, chat)?;
        Some(s)
    } else {
        None
    };
    let conns: Vec<&Connection> = memory
        .iter()
        .map(|m| &m.conn)
        .chain(shared.iter().map(|s| &s.conn))
        .collect();
    scrub(&conns)?;
    Ok(out)
}

/// Forget one record, and every shared copy of it — including a copy made
/// before it was corrected, found through `replaces`.
pub fn forget(store_dir: &Path, persona: &str, uid: &str) -> Result<usize> {
    let memory = Memory::open_to_edit(store_dir, persona)?;
    let lineage = memory.lineage(uid)?;
    memory.delete(uid)?;
    let Some(shared) = memory.shared()? else {
        scrub(&[&memory.conn])?;
        return Ok(0);
    };
    let copies = shared.unshare_from(persona, &lineage)?;
    scrub(&[&memory.conn, &shared.conn])?;
    Ok(copies)
}

#[cfg(test)]
mod tests;
