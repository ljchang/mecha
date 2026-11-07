//! Typed, sensor-free host observations. Authority is injected, never inferred.
use super::{sql::*, store::capture_in, *};
use anyhow::{ensure, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalBinding {
    pub authority: String,
    pub locator: String,
    pub revision: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceStatus {
    Checked,
    Withheld,
    Unavailable,
}
pub trait ExternalSourceGuard: Send + Sync {
    fn check(&self, binding: &ExternalBinding) -> SourceStatus;
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureIdentity {
    pub domain: String,
    pub account: String,
    pub session_id: String,
    pub prefix_digest: String,
    pub check_id: String,
    pub check_version: String,
    pub evaluator_version: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureReceipt {
    pub id: String,
    #[serde(deserialize_with = "strict_reference")]
    pub projection: RevisionRef,
    #[serde(deserialize_with = "strict_references")]
    pub originals: Vec<RevisionRef>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PassageKind {
    UserRequest,
    AssistantText,
    ToolResult,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginalPassage {
    #[serde(deserialize_with = "strict_source")]
    pub source: SourceKey,
    pub kind: PassageKind,
    pub line: u64,
    pub pointer: String,
    pub text: String,
    pub occurred_at: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReceiptStatus {
    Succeeded,
    Failed,
    Refused,
    Pending,
    Unavailable,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolReceipt {
    pub call_id: String,
    pub tool_name: String,
    pub status: ReceiptStatus,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckVerdict {
    Passed,
    Failed,
    Refused,
    Pending,
    Unavailable,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactCheck {
    pub artifact_digest: Option<String>,
    pub verdict: CheckVerdict,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmedCriterion {
    pub goal: String,
    pub goal_text: String,
    pub criterion_text: String,
    pub criterion_revision: String,
    pub confirmation_digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostCapture {
    pub identity: CaptureIdentity,
    pub binding: ExternalBinding,
    pub criterion: ConfirmedCriterion,
    pub originals: Vec<OriginalPassage>,
    pub receipts: Vec<ToolReceipt>,
    pub artifact: ArtifactCheck,
    #[serde(deserialize_with = "strict_references")]
    pub consulted: Vec<RevisionRef>,
    pub scopes: BTreeSet<String>,
    pub occurred_at: Option<String>,
}

// Historical Record/RevisionRef decoders intentionally allow future fields.
// Strict trusted-host DTO decoding is local to this admission boundary.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictReference {
    object: String,
    revision: String,
}
impl From<StrictReference> for RevisionRef {
    fn from(r: StrictReference) -> Self {
        Self {
            object: r.object,
            revision: r.revision,
        }
    }
}
fn strict_reference<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<RevisionRef, D::Error> {
    Ok(StrictReference::deserialize(d)?.into())
}
fn strict_references<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Vec<RevisionRef>, D::Error> {
    Ok(Vec::<StrictReference>::deserialize(d)?
        .into_iter()
        .map(Into::into)
        .collect())
}
fn strict_source<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<SourceKey, D::Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Source {
        connector: String,
        account: String,
        id: String,
    }
    let source = Source::deserialize(d)?;
    Ok(SourceKey {
        connector: source.connector,
        account: source.account,
        id: source.id,
    })
}

pub(super) fn has_bindings(conn: &Connection) -> Result<bool> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version == 6 {
        validate_schema(conn)?;
        return Ok(true);
    }
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='external_binding')",
        [],
        |r| r.get(0),
    )?)
}
pub(super) fn validate_schema(conn: &Connection) -> Result<()> {
    conn.prepare(
        "SELECT revision,authority,locator,binding_revision FROM external_binding LIMIT 0",
    )
    .map_err(|_| anyhow::anyhow!("invalid host capture schema"))?;
    // Earlier unpublished v6 fixtures could copy memory without complete
    // lineage. Refuse that schema; never backfill immutable provenance.
    conn.prepare("SELECT identity_hash,payload_hash,receipt_id,projection_object,projection_revision,originals,state,lineage_version FROM host_capture_receipt LIMIT 0")
        .map_err(|_|anyhow::anyhow!("invalid host capture schema"))?;
    let incomplete: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM host_capture_receipt h WHERE h.lineage_version IS NOT 2 OR h.state='active' AND
           (h.projection_revision IS NULL OR h.originals IS NULL OR
            NOT EXISTS(SELECT 1 FROM external_binding b WHERE b.revision=h.projection_revision) OR
            EXISTS(SELECT 1 FROM json_each(h.originals) j WHERE
              NOT EXISTS(SELECT 1 FROM external_binding b WHERE b.revision=json_extract(j.value,'$.revision')))))",
        [], |r|r.get(0)).map_err(|_|anyhow::anyhow!("invalid host capture schema"))?;
    ensure!(!incomplete, "invalid host capture schema");
    Ok(())
}
pub(super) fn valid_binding(b: &ExternalBinding) -> bool {
    identifier(&b.authority, 64)
        && b.authority.is_ascii()
        && digest(&b.locator)
        && digest(&b.revision)
}
fn identifier(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && s.trim() == s && !s.chars().any(char::is_control)
}
fn digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn hash<T: Serialize>(value: &T) -> Result<String> {
    Ok(Sha256::digest(serde_json::to_vec(value)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

impl Memory {
    pub fn create_host_capture(path: &Path, domain: &str) -> Result<Self> {
        Self::create_schema(path, domain, "CREATE TABLE external_binding(revision TEXT PRIMARY KEY REFERENCES revision(id),authority TEXT NOT NULL,locator TEXT NOT NULL,binding_revision TEXT NOT NULL);
          CREATE TABLE host_capture_receipt(identity_hash TEXT PRIMARY KEY,payload_hash TEXT,receipt_id TEXT,projection_object TEXT,projection_revision TEXT,originals TEXT,state TEXT NOT NULL CHECK(state IN ('active','forgotten')),lineage_version INTEGER NOT NULL CHECK(lineage_version=2));
          PRAGMA user_version=6;")
    }
    pub fn with_source_guard(mut self, guard: Arc<dyn ExternalSourceGuard>) -> Self {
        self.source_guard = Some(guard);
        self
    }
    pub(crate) fn admit_host_capture(
        &mut self,
        input: HostCapture,
        access: &Access,
    ) -> Result<CaptureReceipt> {
        admit(self, input, access)
    }
}

fn admit(db: &mut Memory, input: HostCapture, access: &Access) -> Result<CaptureReceipt> {
    db.check_access(access)?;
    ensure!(
        input.identity.domain == access.domain,
        "invalid host capture domain"
    );
    let version: i64 = db.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(version == 6, "host capture requires version 6");
    validate(&input)?;
    let identity = hash(&input.identity)?;
    let payload = hash(&input)?;
    let guard = db.source_guard.as_deref();
    ensure!(
        guard.is_some_and(|g| g.check(&input.binding) == SourceStatus::Checked),
        "host source unavailable"
    );
    let tx = db
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut scopes = input.scopes.clone();
    ensure!(scopes.is_subset(&access.scopes), "host source unavailable");
    for original in &input.originals {
        let key = source_digest(&db.instance, &original.source)?;
        let suppressed: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM suppression WHERE digest=?1)",
            [&key],
            |r| r.get(0),
        )?;
        ensure!(!suppressed, "host source unavailable");
    }
    let mut consulted_objects = BTreeSet::new();
    for dependency in &input.consulted {
        current(&tx, dependency)?;
        ensure!(
            fresh(&tx, dependency, i64::MAX)?
                && authorized_with_guard(&tx, dependency, access, guard)?,
            "host source unavailable"
        );
        // Every closure member must be a sealed bridge record. Legacy Evidence
        // may contain raw host lines and cannot become a projection dependency.
        for member in closure(&tx, dependency)? {
            let safe: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM external_binding WHERE revision=?1)",
                [&member.revision],
                |r| r.get(0),
            )?;
            ensure!(
                safe && matches!(lookup(&tx, &member)?, Some(Record::Evidence(_))),
                "unsupported host dependency"
            );
            scopes.extend(required_scopes(&tx, &member.revision)?);
            consulted_objects.insert(member.object);
        }
    }
    let existing = tx.query_row("SELECT payload_hash,receipt_id,projection_object,projection_revision,originals,state FROM host_capture_receipt WHERE identity_hash=?1", [&identity], |r| Ok((r.get::<_,Option<String>>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,String>(5)?))).optional()?;
    if let Some((old, id, object, revision, originals, state)) = existing {
        ensure!(
            state == "active" && old.as_deref() == Some(&payload),
            "host capture conflict or unavailable"
        );
        let receipt = CaptureReceipt {
            id: id.ok_or_else(|| anyhow::anyhow!("host capture unavailable"))?,
            projection: RevisionRef {
                object: object.ok_or_else(|| anyhow::anyhow!("host capture unavailable"))?,
                revision: revision.ok_or_else(|| anyhow::anyhow!("host capture unavailable"))?,
            },
            originals: serde_json::from_str(
                &originals.ok_or_else(|| anyhow::anyhow!("host capture unavailable"))?,
            )?,
        };
        for reference in receipt
            .originals
            .iter()
            .chain(std::iter::once(&receipt.projection))
        {
            current(&tx, reference)?;
            ensure!(
                fresh(&tx, reference, i64::MAX)?
                    && authorized_with_guard(&tx, reference, access, guard)?,
                "host source unavailable"
            );
        }
        tx.commit()?;
        return Ok(receipt);
    }
    let mut originals = Vec::new();
    for passage in &input.originals {
        let key = source_digest(&db.instance, &passage.source)?;
        let old_object: Option<String> = tx
            .query_row(
                "SELECT object_id FROM source_item WHERE digest=?1",
                [&key],
                |r| r.get(0),
            )
            .optional()?;
        let old_reference = old_object
            .as_deref()
            .map(|object| head(&tx, object, i64::MAX))
            .transpose()?
            .flatten()
            .map(|(reference, _, _)| reference);
        let reference = capture_in(
            &tx,
            &db.instance,
            Capture {
                source: passage.source.clone(),
                text: passage.text.clone(),
                occurred_at: passage.occurred_at.clone(),
                scopes: scopes.clone(),
                transcript: None,
            },
        )?;
        // Every separately readable passage can repeat consulted evidence.
        // Seal the same conservative lineage onto originals in this transaction.
        // Reused revisions are immutable: a later prefix cannot add/drop edges.
        ensure!(
            !consulted_objects.contains(&reference.object),
            "unsupported host dependency cycle"
        );
        if old_reference.as_ref() == Some(&reference) {
            let pairs = |refs: &[RevisionRef]| {
                refs.iter()
                    .map(|r| (r.object.clone(), r.revision.clone()))
                    .collect::<BTreeSet<_>>()
            };
            ensure!(
                pairs(&dependencies(&tx, &reference.revision)?) == pairs(&input.consulted),
                "unsupported host source lineage change"
            );
        } else {
            for dependency in &input.consulted {
                tx.execute(
                    "INSERT OR IGNORE INTO dependency VALUES (?1,?2)",
                    params![reference.revision, dependency.revision],
                )?;
            }
        }
        bind(&tx, &reference, &input.binding)?;
        originals.push(reference);
    }
    let projection_source = SourceKey {
        connector: "host-capture-projection-v1".into(),
        account: input.identity.account.clone(),
        id: identity.clone(),
    };
    let key = source_digest(&db.instance, &projection_source)?;
    let suppressed: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM suppression WHERE digest=?1)",
        [&key],
        |r| r.get(0),
    )?;
    ensure!(!suppressed, "host source unavailable");
    let mut dependencies = originals.clone();
    dependencies.extend(input.consulted.iter().cloned());
    let projection = append(
        &tx,
        None,
        &Record::Evidence(Capture {
            source: projection_source,
            text: serde_json::to_string(&input)?,
            occurred_at: input.occurred_at.clone(),
            scopes: scopes.clone(),
            transcript: None,
        }),
        &scopes,
        &dependencies,
        "active",
    )?;
    tx.execute(
        "INSERT INTO source_item VALUES (?1,?2)",
        params![key, projection.object],
    )?;
    bind(&tx, &projection, &input.binding)?;
    let receipt = CaptureReceipt {
        id: uuid::Uuid::new_v4().to_string(),
        projection,
        originals,
    };
    tx.execute(
        "INSERT INTO host_capture_receipt VALUES (?1,?2,?3,?4,?5,?6,'active',2)",
        params![
            identity,
            payload,
            receipt.id,
            receipt.projection.object,
            receipt.projection.revision,
            serde_json::to_string(&receipt.originals)?
        ],
    )?;
    ensure!(
        authorized_with_guard(&tx, &receipt.projection, access, guard)?,
        "host source unavailable"
    );
    tx.commit()?;
    Ok(receipt)
}
fn bind(conn: &Connection, reference: &RevisionRef, binding: &ExternalBinding) -> Result<()> {
    // An unchanged original keeps its first immutable historical binding. A
    // later-prefix projection adds its own binding and must pass both checks;
    // replacing the original binding could silently weaken older receipts.
    conn.execute(
        "INSERT OR IGNORE INTO external_binding VALUES (?1,?2,?3,?4)",
        params![
            reference.revision,
            binding.authority,
            binding.locator,
            binding.revision
        ],
    )?;
    Ok(())
}
fn validate(input: &HostCapture) -> Result<()> {
    let i = &input.identity;
    ensure!(
        [
            &i.domain,
            &i.account,
            &i.session_id,
            &i.check_version,
            &i.evaluator_version,
            &input.criterion.goal
        ]
        .iter()
        .all(|s| identifier(s, 256))
            && [
                &i.prefix_digest,
                &i.check_id,
                &input.criterion.criterion_revision,
                &input.criterion.confirmation_digest
            ]
            .iter()
            .all(|s| digest(s))
            && valid_binding(&input.binding),
        "invalid host capture identifiers"
    );
    ensure!(
        (1..=16).contains(&input.originals.len())
            && (1..=32).contains(&input.receipts.len())
            && input.consulted.len() <= 32,
        "invalid host capture bounds"
    );
    ensure!(
        [&input.criterion.goal_text, &input.criterion.criterion_text]
            .iter()
            .all(|s| !s.trim().is_empty() && s.len() <= 2048),
        "invalid host criterion"
    );
    let mut sources = BTreeSet::new();
    for p in &input.originals {
        ensure!(
            p.source.account == i.account
                && [&p.source.connector, &p.source.account, &p.source.id]
                    .iter()
                    .all(|s| identifier(s, 256))
                && p.line > 0
                && p.pointer.starts_with('/')
                && p.pointer.len() <= 256
                && !p.text.is_empty()
                && p.text.len() <= 4096,
            "invalid host passage"
        );
        ensure!(
            sources.insert(serde_json::to_string(&p.source)?),
            "duplicate host source"
        );
        if let Some(t) = &p.occurred_at {
            normalize_time(t).map_err(|_| anyhow::anyhow!("invalid host time"))?;
        }
    }
    for r in &input.receipts {
        ensure!(
            identifier(&r.call_id, 256) && identifier(&r.tool_name, 256),
            "invalid host receipt"
        );
    }
    for r in &input.consulted {
        ensure!(
            identifier(&r.object, 256) && identifier(&r.revision, 256),
            "invalid host dependency"
        );
    }
    ensure!(
        input
            .artifact
            .artifact_digest
            .as_ref()
            .is_none_or(|s| digest(s)),
        "invalid artifact digest"
    );
    if let Some(t) = &input.occurred_at {
        normalize_time(t).map_err(|_| anyhow::anyhow!("invalid host time"))?;
    }
    validate_scopes(&input.scopes)?;
    ensure!(
        serde_json::to_vec(input)?.len() <= 32768,
        "oversized host projection"
    );
    Ok(())
}

/// Receipt siblings share the same suppression boundary, whichever member the
/// caller forgets. The fixed-point walk also purges dependent captures.
pub(super) fn extend_forgotten(conn: &Connection) -> Result<usize> {
    if !has_bindings(conn)? {
        return Ok(0);
    }
    Ok(conn.execute("INSERT OR IGNORE INTO doomed SELECT r.object_id FROM host_capture_receipt h JOIN revision r ON r.id=h.projection_revision OR r.id IN (SELECT json_extract(value,'$.revision') FROM json_each(h.originals)) WHERE h.state='active' AND (h.projection_object IN (SELECT id FROM doomed) OR EXISTS(SELECT 1 FROM json_each(h.originals) WHERE json_extract(value,'$.object') IN (SELECT id FROM doomed)))",[])? )
}
pub(super) fn invalidate_forgotten(conn: &Connection) -> Result<()> {
    if !has_bindings(conn)? {
        return Ok(());
    }
    conn.execute("INSERT OR IGNORE INTO suppression SELECT digest FROM source_item WHERE object_id IN (SELECT id FROM doomed)",[])?;
    conn.execute("UPDATE host_capture_receipt SET state='forgotten',payload_hash=NULL,receipt_id=NULL,projection_object=NULL,projection_revision=NULL,originals=NULL WHERE projection_object IN (SELECT id FROM doomed)",[])?;
    conn.execute("DELETE FROM external_binding WHERE revision IN (SELECT id FROM revision WHERE object_id IN (SELECT id FROM doomed))",[])?;
    Ok(())
}
