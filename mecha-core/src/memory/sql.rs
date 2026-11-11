use super::*;
use anyhow::{ensure, Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

pub(super) fn configure(conn: &Connection) -> Result<()> {
    conn.busy_timeout(Duration::from_secs(3))?;
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA temp_store=MEMORY;")?;
    let pragma = |name| conn.pragma_query_value(None, name, |row| row.get::<_, i64>(0));
    let journal: String = conn.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
    ensure!(
        pragma("foreign_keys")? == 1
            && pragma("secure_delete")? == 1
            && journal.eq_ignore_ascii_case("delete")
            && pragma("synchronous")? == 2
            && pragma("temp_store")? == 2,
        "could not establish memory store durability and temporary storage settings"
    );
    Ok(())
}

pub(super) fn configure_read_only(conn: &Connection) -> Result<()> {
    conn.busy_timeout(Duration::from_secs(3))?;
    conn.execute_batch("PRAGMA query_only=ON; PRAGMA temp_store=MEMORY;")?;
    let pragma = |name| conn.pragma_query_value(None, name, |row| row.get::<_, i64>(0));
    ensure!(
        pragma("query_only")? == 1 && pragma("temp_store")? == 2,
        "could not establish read-only memory settings"
    );
    Ok(())
}

pub(super) fn identity(conn: &Connection) -> Result<(String, String)> {
    check_identity(conn)?;
    Ok(conn.query_row(
        "SELECT instance,domain FROM meta WHERE singleton=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?)
}

pub(super) fn validate_existing_path(path: &Path) -> Result<PathBuf> {
    let path = validate_path(path)?;
    let meta = std::fs::symlink_metadata(&path)?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink(),
        "expected a regular prototype file"
    );
    Ok(path)
}

pub(super) fn check_identity(conn: &Connection) -> Result<()> {
    let app: i64 = conn.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(
        app == 1296912466 && matches!(version, 1..=10),
        "not a supported Mecha memory prototype"
    );
    if version >= 6 {
        host_capture::validate_schema(conn)?;
        if matches!(version, 7..=10) {
            interpretation::validate_schema(conn)?;
        }
    }
    if matches!(version, 9 | 10) {
        task::validate_schema(conn)?;
    }
    if version == 10 {
        plan::validate_schema(conn)?;
    }
    Ok(())
}
pub(super) fn validate_path(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .filter(|name| {
            name.to_str()
                .is_some_and(|name| name.ends_with(".memory-prototype.sqlite"))
        })
        .context("expected .memory-prototype.sqlite suffix")?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = parent.canonicalize()?;
    ensure!(
        home_store_parent(&parent)
            || !parent
                .components()
                .any(|c| c.as_os_str() == ".mecha" || c.as_os_str() == ".mecha-graph"),
        "live Mecha directories are not memory roots, except the home's memory/store"
    );
    // Retain legitimate ancestor aliases (such as /var on macOS) while
    // leaving the final name unresolved for exclusive/no-follow opens.
    Ok(parent.join(name))
}
/// Whether `parent` is the one directory under a mecha home that may hold a
/// memory database: `<home>/.mecha/memory/store`, and nothing else beneath
/// `.mecha`. Decided by spelling, as the refusal around it is, so this module
/// still names no path of the host's and compiles alone.
fn home_store_parent(parent: &Path) -> bool {
    let names: Vec<_> = parent.components().map(|c| c.as_os_str()).collect();
    !names.iter().any(|c| *c == ".mecha-graph")
        && names.iter().filter(|c| **c == ".mecha").count() == 1
        && names.len() >= 3
        && names[names.len() - 3] == ".mecha"
        && names[names.len() - 2] == "memory"
        && names[names.len() - 1] == "store"
}
pub(super) fn validate_text(text: &str) -> Result<()> {
    ensure!(
        !text.trim().is_empty() && text.len() <= 1_000_000,
        "empty or oversized text"
    );
    Ok(())
}
pub(super) fn validate_scopes(scopes: &BTreeSet<String>) -> Result<()> {
    ensure!(
        scopes.len() <= 128
            && scopes
                .iter()
                .all(|s| !s.trim().is_empty() && s.len() <= 256),
        "invalid scopes"
    );
    Ok(())
}
pub(super) fn source_digest(instance: &str, source: &SourceKey) -> Result<String> {
    // Synthetic-only pseudonymization, not the production HMAC/secret-key design.
    Ok(Sha256::digest(serde_json::to_vec(&(instance, source))?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
pub(super) fn normalize_time(value: &str) -> Result<String> {
    let parsed = DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc);
    let canonical = parsed.to_rfc3339_opts(SecondsFormat::Nanos, true);
    ensure!(
        canonical.len() == 30,
        "timestamp year out of supported range"
    );
    Ok(canonical)
}
pub(super) fn normalize_interval(interval: &mut Interval) -> Result<()> {
    for bound in [&mut interval.from, &mut interval.to] {
        if let Bound::Known(value) = bound {
            *value = normalize_time(value)?;
        }
    }
    if let (Bound::Known(start), Bound::Known(end)) = (&interval.from, &interval.to) {
        ensure!(start < end, "valid interval must be nonempty");
    }
    Ok(())
}
pub(super) fn covers(interval: &Interval, at: &str) -> Option<bool> {
    if let Bound::Known(ref start) = interval.from {
        if at < start.as_str() {
            return None;
        }
    }
    if let Bound::Known(ref end) = interval.to {
        if at >= end.as_str() {
            return None;
        }
    }
    Some(matches!(interval.from, Bound::Unknown) || matches!(interval.to, Bound::Unknown))
}
pub(super) fn fts_expression(text: &str) -> Result<String> {
    let tokens: Vec<_> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    ensure!(tokens.len() <= 32, "too many search terms");
    ensure!(
        text.trim().is_empty() || !tokens.is_empty(),
        "query has no searchable terms"
    );
    Ok(tokens
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(" AND "))
}
pub(super) fn sequence(conn: &Connection) -> Result<i64> {
    Ok(
        conn.query_row("SELECT coalesce(max(seq),0) FROM change_set", [], |r| {
            r.get(0)
        })?,
    )
}
pub(super) fn lookup(conn: &Connection, reference: &RevisionRef) -> Result<Option<Record>> {
    let value: Option<String> = conn
        .query_row(
            "SELECT payload FROM revision WHERE id=?1 AND object_id=?2",
            params![reference.revision, reference.object],
            |r| r.get(0),
        )
        .optional()?;
    value
        .map(|s| serde_json::from_str(&s).map_err(Into::into))
        .transpose()
}
pub(super) fn head(
    conn: &Connection,
    object: &str,
    seq: i64,
) -> Result<Option<(RevisionRef, Record, String)>> {
    let row=conn.query_row("SELECT id,payload,lifecycle FROM revision WHERE object_id=?1 AND seq<=?2 ORDER BY seq DESC LIMIT 1",params![object,seq],|r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?;
    row.map(|(revision, payload, state)| {
        Ok((
            RevisionRef {
                object: object.into(),
                revision,
            },
            serde_json::from_str(&payload)?,
            state,
        ))
    })
    .transpose()
}
pub(super) fn current(conn: &Connection, reference: &RevisionRef) -> Result<()> {
    ensure!(!restricted(conn, &reference.object)?, "restricted input");
    let (now, _, state) = head(conn, &reference.object, i64::MAX)?.context("missing input")?;
    ensure!(
        now == *reference && state == "active",
        "stale expected revision or inactive input"
    );
    Ok(())
}
pub(super) fn dependencies(conn: &Connection, revision: &str) -> Result<Vec<RevisionRef>> {
    let mut s=conn.prepare("SELECT r.object_id,r.id FROM dependency d JOIN revision r ON r.id=d.input WHERE d.output=?1")?;
    let rows = s.query_map([revision], |r| {
        Ok(RevisionRef {
            object: r.get(0)?,
            revision: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}
pub(super) fn closure(conn: &Connection, reference: &RevisionRef) -> Result<Vec<RevisionRef>> {
    let mut s=conn.prepare("WITH RECURSIVE inputs(id) AS (SELECT ?1 UNION SELECT d.input FROM dependency d JOIN inputs i ON d.output=i.id) SELECT r.object_id,r.id FROM inputs JOIN revision r ON r.id=inputs.id")?;
    let rows = s.query_map([&reference.revision], |r| {
        Ok(RevisionRef {
            object: r.get(0)?,
            revision: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}
pub(super) fn required_scopes(conn: &Connection, revision: &str) -> Result<BTreeSet<String>> {
    let mut s = conn.prepare("SELECT name FROM scope WHERE revision=?1")?;
    let rows = s.query_map([revision], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
/// Reuse exact source verdicts only within one read phase. Selection and final
/// return validation must construct separate instances over the original guard;
/// the SQLite transaction does not snapshot external files. Never cache DB scope,
/// restriction or head checks here, nor retain this object across operations.
pub(super) struct SourceChecks<'a> {
    guard: Option<&'a dyn host_capture::ExternalSourceGuard>,
    statuses: std::sync::Mutex<
        std::collections::BTreeMap<(String, String, String), host_capture::SourceStatus>,
    >,
}
impl<'a> SourceChecks<'a> {
    pub(super) fn new(guard: Option<&'a dyn host_capture::ExternalSourceGuard>) -> Self {
        Self {
            guard,
            statuses: std::sync::Mutex::new(std::collections::BTreeMap::new()),
        }
    }
}
impl host_capture::ExternalSourceGuard for SourceChecks<'_> {
    fn check(&self, binding: &host_capture::ExternalBinding) -> host_capture::SourceStatus {
        let Ok(mut statuses) = self.statuses.lock() else {
            return host_capture::SourceStatus::Unavailable;
        };
        let key = (
            binding.authority.clone(),
            binding.locator.clone(),
            binding.revision.clone(),
        );
        *statuses.entry(key).or_insert_with(|| {
            self.guard
                .map_or(host_capture::SourceStatus::Unavailable, |guard| {
                    guard.check(binding)
                })
        })
    }
}
pub(super) fn authorized(
    conn: &Connection,
    reference: &RevisionRef,
    access: &Access,
) -> Result<bool> {
    authorized_with_guard(conn, reference, access, None)
}
pub(super) fn authorized_with_guard(
    conn: &Connection,
    reference: &RevisionRef,
    access: &Access,
    guard: Option<&dyn host_capture::ExternalSourceGuard>,
) -> Result<bool> {
    let bindings = host_capture::has_bindings(conn)?;
    let checks = SourceChecks::new(guard);
    let guard: Option<&dyn host_capture::ExternalSourceGuard> = Some(&checks);
    for input in closure(conn, reference)? {
        if bindings {
            let binding = conn.query_row("SELECT authority,locator,binding_revision FROM external_binding WHERE revision=?1", [&input.revision], |r| Ok(host_capture::ExternalBinding { authority:r.get(0)?,locator:r.get(1)?,revision:r.get(2)? })).optional();
            match binding {
                Ok(Some(b))
                    if host_capture::valid_binding(&b)
                        && guard.is_some_and(|g| {
                            g.check(&b) == host_capture::SourceStatus::Checked
                        }) => {}
                Ok(None) => {}
                _ => return Ok(false),
            }
        }
        if restricted(conn, &input.object)? {
            return Ok(false);
        }
        if !required_scopes(conn, &input.revision)?.is_subset(&access.scopes) {
            return Ok(false);
        }
        let Some((now, _, _)) = head(conn, &input.object, i64::MAX)? else {
            return Ok(false);
        };
        if !required_scopes(conn, &now.revision)?.is_subset(&access.scopes) {
            return Ok(false);
        }
    }
    Ok(true)
}
/// Unsupported producers never acquire bridge authority from a guarded input.
pub(super) fn unguarded(conn: &Connection, reference: &RevisionRef) -> Result<bool> {
    if !host_capture::has_bindings(conn)? {
        return Ok(true);
    }
    for input in closure(conn, reference)? {
        let bound: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM external_binding WHERE revision=?1)",
            [&input.revision],
            |r| r.get(0),
        )?;
        if bound {
            return Ok(false);
        }
    }
    Ok(true)
}
pub(super) fn fresh(conn: &Connection, reference: &RevisionRef, seq: i64) -> Result<bool> {
    for input in closure(conn, reference)? {
        let Some((at, _, state)) = head(conn, &input.object, seq)? else {
            return Ok(false);
        };
        if at != input || state != "active" {
            return Ok(false);
        }
    }
    Ok(true)
}
fn restricted(conn: &Connection, object: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM restriction WHERE object_id=?1)",
        [object],
        |r| r.get(0),
    )?)
}
pub(super) fn append(
    conn: &Connection,
    previous: Option<&RevisionRef>,
    record: &Record,
    scopes: &BTreeSet<String>,
    dependencies: &[RevisionRef],
    state: &str,
) -> Result<RevisionRef> {
    if let Record::Task(_) = record {
        ensure!(
            matches!(task::version(conn)?, 9 | 10) && state == "active",
            "invalid task append schema or lifecycle"
        );
        if let Some(previous) = previous {
            for dependency in dependencies {
                ensure!(
                    closure(conn, dependency)?
                        .iter()
                        .all(|r| r.object != previous.object),
                    "transitive task self-dependency is not permitted"
                );
            }
        }
        ensure!(dependencies.is_empty(), "task dependencies are not exposed");
    }
    if let Record::Plan(plan) = record {
        ensure!(
            task::version(conn)? == 10
                && state == "active"
                && dependencies == [plan.task.reference.clone()],
            "invalid plan append"
        );
    }
    let kind = match record {
        Record::Evidence(_) => "evidence",
        Record::Entity { .. } => "entity",
        Record::Relation(_) => "relation",
        Record::Concept(_) => "concept",
        Record::Interpretation(_) => "interpretation",
        Record::Task(_) => "task",
        Record::Plan(_) => "plan",
    };
    let reference = RevisionRef {
        object: previous
            .map(|r| r.object.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        revision: Uuid::new_v4().to_string(),
    };
    conn.execute(
        "INSERT OR IGNORE INTO object VALUES (?1,?2)",
        params![reference.object, kind],
    )?;
    conn.execute(
        "INSERT INTO change_set(committed_at) VALUES (?1)",
        [Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true)],
    )?;
    let seq = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO revision VALUES (?1,?2,?3,?4,?5,?6)",
        params![
            reference.revision,
            reference.object,
            seq,
            previous.map(|r| &r.revision),
            state,
            serde_json::to_string(record)?
        ],
    )?;
    for scope in scopes {
        conn.execute(
            "INSERT INTO scope VALUES (?1,?2)",
            params![reference.revision, scope],
        )?;
    }
    for input in dependencies {
        conn.execute(
            "INSERT OR IGNORE INTO dependency VALUES (?1,?2)",
            params![reference.revision, input.revision],
        )?;
    }
    let text = match record {
        Record::Evidence(e) => &e.text,
        Record::Entity { label } => label,
        Record::Relation(r) => &r.text,
        Record::Concept(c) => &c.principle,
        Record::Interpretation(_) | Record::Task(_) | Record::Plan(_) => "",
    };
    let transcript_text = match record {
        Record::Evidence(Capture {
            transcript: Some(meta),
            ..
        }) => Some(
            meta.fields
                .iter()
                .map(|f| f.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    };
    let concept_text = match record {
        Record::Concept(c) => Some(format!("{}\n{}\n{}", c.name, c.principle, c.applicability)),
        _ => None,
    };
    let text = concept_text
        .as_deref()
        .or(transcript_text.as_deref())
        .unwrap_or(text);
    if state == "active" && !text.is_empty() {
        conn.execute(
            "INSERT INTO memory_fts(revision,text) VALUES (?1,?2)",
            params![reference.revision, text],
        )?;
    }
    Ok(reference)
}
