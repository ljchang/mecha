use super::*;
use anyhow::{ensure, Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path, time::Duration};
use uuid::Uuid;

pub(super) fn configure(conn: &Connection) -> Result<()> {
    conn.busy_timeout(Duration::from_secs(3))?;
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;")?;
    Ok(())
}
pub(super) fn check_identity(conn: &Connection) -> Result<()> {
    let app: i64 = conn.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(
        app == 1296912466 && matches!(version, 1..=3),
        "not a supported Mecha memory prototype"
    );
    Ok(())
}
pub(super) fn validate_path(path: &Path) -> Result<()> {
    ensure!(
        path.file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.ends_with(".memory-prototype.sqlite")),
        "expected .memory-prototype.sqlite suffix"
    );
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = parent.canonicalize()?;
    ensure!(
        !parent
            .components()
            .any(|c| c.as_os_str() == ".mecha" || c.as_os_str() == ".mecha-graph"),
        "live Mecha directories are not prototype roots"
    );
    Ok(())
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
pub(super) fn authorized(
    conn: &Connection,
    reference: &RevisionRef,
    access: &Access,
) -> Result<bool> {
    for input in closure(conn, reference)? {
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
    let kind = match record {
        Record::Evidence(_) => "evidence",
        Record::Entity { .. } => "entity",
        Record::Relation(_) => "relation",
        Record::Concept(_) => "concept",
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
