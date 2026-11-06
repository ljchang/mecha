use super::{sql::*, *};
use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use std::{collections::BTreeSet, fs::OpenOptions, path::Path};
use uuid::Uuid;

/// A fixture-only store. No handles to live Mecha or graph stores are discovered.
pub struct Memory {
    pub(super) conn: Connection,
    pub(super) instance: String,
    domain: String,
}

impl Memory {
    /// Creates a new file, never replaces one. Parent directory must already exist.
    pub fn create(path: &Path, domain: &str) -> Result<Self> {
        validate_path(path)?;
        ensure!(
            !domain.trim().is_empty() && domain.len() <= 256,
            "invalid domain"
        );
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(path)
            .context("prototype requires a new file")?;
        drop(file);
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        configure(&conn)?;
        conn.execute_batch(include_str!("schema.sql"))?;
        let instance = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO meta VALUES (1,?1,?2,0)",
            params![instance, domain],
        )?;
        Ok(Self {
            conn,
            instance,
            domain: domain.into(),
        })
    }

    /// Checks identity read-only before enabling writes; never upgrades another DB.
    pub fn open(path: &Path) -> Result<Self> {
        validate_path(path)?;
        let meta = std::fs::symlink_metadata(path)?;
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "expected a regular prototype file"
        );
        let probe = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        check_identity(&probe)?;
        drop(probe);
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        check_identity(&conn)?;
        configure(&conn)?;
        let (instance, domain) = conn.query_row(
            "SELECT instance,domain FROM meta WHERE singleton=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(Self {
            conn,
            instance,
            domain,
        })
    }

    /// A genuinely read-only handle: no journal/schema changes or write fallback.
    pub fn open_read_only(path: &Path) -> Result<Self> {
        validate_path(path)?;
        let meta = std::fs::symlink_metadata(path)?;
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "expected a regular prototype file"
        );
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.busy_timeout(std::time::Duration::from_secs(3))?;
        conn.execute_batch("PRAGMA query_only=ON; PRAGMA temp_store=MEMORY;")?;
        check_identity(&conn)?;
        let (instance, domain) = conn.query_row(
            "SELECT instance,domain FROM meta WHERE singleton=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(Self {
            conn,
            instance,
            domain,
        })
    }

    pub fn capture(&mut self, input: Capture) -> Result<RevisionRef> {
        anyhow::ensure!(
            input.transcript.is_none(),
            "transcript metadata is owned by the session adapter"
        );
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let reference = capture_in(&tx, &self.instance, input)?;
        tx.commit()?;
        Ok(reference)
    }

    pub fn entity(&mut self, label: &str, scopes: BTreeSet<String>) -> Result<RevisionRef> {
        validate_text(label)?;
        validate_scopes(&scopes)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let reference = append(
            &tx,
            None,
            &Record::Entity {
                label: label.into(),
            },
            &scopes,
            &[],
            "active",
        )?;
        tx.commit()?;
        Ok(reference)
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        Ok(Snapshot {
            instance: self.instance.clone(),
            sequence: sequence(&self.conn)?,
        })
    }

    /// Explicit revision dereference is historical evidence, not a current belief query.
    pub fn read(&self, reference: &RevisionRef, access: &Access) -> Result<Option<Record>> {
        self.check_access(access)?;
        let tx = self.conn.unchecked_transaction()?;
        let record = lookup(&tx, reference)?;
        let result = if record.is_some() && authorized(&tx, reference, access)? {
            record
        } else {
            None
        };
        tx.commit()?;
        Ok(result)
    }

    pub fn relate(&mut self, input: RelationInput) -> Result<RevisionRef> {
        self.write_relation(None, input)
    }
    pub fn revise_relation(
        &mut self,
        expected: &RevisionRef,
        input: RelationInput,
    ) -> Result<RevisionRef> {
        self.write_relation(Some(expected), input)
    }

    fn write_relation(
        &mut self,
        expected: Option<&RevisionRef>,
        mut input: RelationInput,
    ) -> Result<RevisionRef> {
        validate_text(&input.text)?;
        ensure!(
            !input.predicate.is_empty()
                && input.predicate.len() <= 128
                && input
                    .predicate
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "invalid predicate"
        );
        ensure!(matches!(input.polarity, -1 | 1), "polarity must be -1 or 1");
        normalize_interval(&mut input.valid)?;
        ensure!(input.consulted.len() <= 128, "too many consulted inputs");
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(prev) = expected {
            current(&tx, prev)?;
            ensure!(
                matches!(lookup(&tx, prev)?, Some(Record::Relation(_))),
                "expected a relation"
            );
        }
        current(&tx, &input.subject)?;
        ensure!(
            matches!(lookup(&tx, &input.subject)?, Some(Record::Entity { .. })),
            "subject must bind an entity revision"
        );
        let mut dependencies = vec![input.subject.clone(), input.support.evidence.clone()];
        if let Target::Entity(ref target) = input.object {
            current(&tx, target)?;
            ensure!(
                matches!(lookup(&tx, target)?, Some(Record::Entity { .. })),
                "object must bind an entity revision"
            );
            dependencies.push(target.clone());
        } else if let Target::Literal(ref value) = input.object {
            validate_text(value)?;
        }
        current(&tx, &input.support.evidence)?;
        let Some(Record::Evidence(evidence)) = lookup(&tx, &input.support.evidence)? else {
            bail!("support must be captured evidence")
        };
        ensure!(
            input.support.start < input.support.end
                && evidence
                    .text
                    .get(input.support.start..input.support.end)
                    .is_some(),
            "invalid UTF-8 evidence span"
        );
        dependencies.extend(input.consulted.iter().cloned());
        let mut scopes = BTreeSet::new();
        for dependency in &dependencies {
            current(&tx, dependency)?;
            if let Some(expected) = expected {
                ensure!(
                    closure(&tx, dependency)?
                        .iter()
                        .all(|r| r.object != expected.object),
                    "transitive self-dependency is not permitted"
                );
            }
            ensure!(
                fresh(&tx, dependency, i64::MAX)?,
                "consulted input is stale"
            );
            scopes.extend(required_scopes(&tx, &dependency.revision)?);
        }
        let reference = append(
            &tx,
            expected,
            &Record::Relation(input),
            &scopes,
            &dependencies,
            "active",
        )?;
        tx.commit()?;
        Ok(reference)
    }

    pub fn withdraw(&mut self, expected: &RevisionRef) -> Result<RevisionRef> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        current(&tx, expected)?;
        let record = lookup(&tx, expected)?.context("missing record")?;
        ensure!(
            matches!(record, Record::Relation(_) | Record::Concept(_)),
            "only relation/concept withdrawal is exposed"
        );
        let deps = dependencies(&tx, &expected.revision)?;
        let scopes = required_scopes(&tx, &expected.revision)?;
        let reference = append(&tx, Some(expected), &record, &scopes, &deps, "withdrawn")?;
        tx.commit()?;
        Ok(reference)
    }

    pub fn search(&self, query: &Query, access: &Access) -> Result<Vec<Hit>> {
        self.check_access(access)?;
        ensure!(
            query.limit > 0 && query.limit <= 1000,
            "limit must be 1..=1000"
        );
        ensure!(query.text.len() <= 4096, "query too long");
        let at = query.valid_at.as_deref().map(normalize_time).transpose()?;
        let expression = fts_expression(&query.text)?;
        let tx = self.conn.unchecked_transaction()?;
        ensure!(
            query.known_at.instance == self.instance
                && query.known_at.sequence >= 0
                && query.known_at.sequence <= sequence(&tx)?,
            "invalid or foreign snapshot"
        );
        let mut statement = tx.prepare(
            "WITH ranked AS (
            SELECT r.*, row_number() OVER(PARTITION BY object_id ORDER BY seq DESC) n
            FROM revision r WHERE seq<=?1
          ) SELECT id,object_id,seq,payload FROM ranked
          WHERE n=1 AND lifecycle='active'
          AND (?2='' OR id IN (SELECT revision FROM memory_fts WHERE memory_fts MATCH ?2))
          ORDER BY seq DESC,id",
        )?;
        let rows = statement.query_map(params![query.known_at.sequence, expression], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut hits = Vec::new();
        for row in rows {
            let (revision, object, seq, payload) = row?;
            let reference = RevisionRef { object, revision };
            if !authorized(&tx, &reference, access)?
                || !fresh(&tx, &reference, query.known_at.sequence)?
            {
                continue;
            }
            let record: Record = serde_json::from_str(&payload)?;
            let mut possible = false;
            if let Some(ref point) = at {
                let Record::Relation(ref relation) = record else {
                    continue;
                };
                let Some(uncertain) = covers(&relation.valid, point) else {
                    continue;
                };
                possible = uncertain;
                if possible && !query.include_possible {
                    continue;
                }
            }
            hits.push(Hit {
                required_scopes: required_scopes(&tx, &reference.revision)?,
                reference,
                sequence: seq,
                record,
                possible_time: possible,
                private_data: true,
                untrusted_input: true,
            });
            if hits.len() == query.limit {
                break;
            }
        }
        drop(statement);
        tx.commit()?;
        Ok(hits)
    }

    pub(crate) fn check_access(&self, access: &Access) -> Result<()> {
        ensure!(
            access.domain == self.domain,
            "wrong physical security domain"
        );
        Ok(())
    }
}

pub(super) fn capture_in(
    conn: &Connection,
    instance: &str,
    mut input: Capture,
) -> Result<RevisionRef> {
    validate_text(&input.text)?;
    for value in [
        &input.source.connector,
        &input.source.account,
        &input.source.id,
    ] {
        ensure!(
            !value.trim().is_empty() && value.len() <= 8192,
            "invalid source key"
        );
    }
    validate_scopes(&input.scopes)?;
    input.occurred_at = input
        .occurred_at
        .as_deref()
        .map(normalize_time)
        .transpose()?;
    let digest = source_digest(instance, &input.source)?;
    let suppressed: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM suppression WHERE digest=?1)",
        [&digest],
        |r| r.get(0),
    )?;
    ensure!(!suppressed, "source was forgotten");
    let object: Option<String> = conn
        .query_row(
            "SELECT object_id FROM source_item WHERE digest=?1",
            [&digest],
            |r| r.get(0),
        )
        .optional()?;
    let scopes = input.scopes.clone();
    let record = Record::Evidence(input);
    let payload = serde_json::to_string(&record)?;
    let mut previous = None;
    if let Some(ref id) = object {
        let (reference, old, state) = head(conn, id, i64::MAX)?.context("missing source head")?;
        if serde_json::to_string(&old)? == payload && state == "active" {
            return Ok(reference);
        }
        previous = Some(reference);
    }
    let reference = append(conn, previous.as_ref(), &record, &scopes, &[], "active")?;
    conn.execute(
        "INSERT OR IGNORE INTO source_item VALUES (?1,?2)",
        params![digest, reference.object],
    )?;
    Ok(reference)
}
