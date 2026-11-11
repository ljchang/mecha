//! Typed, sensor-free host observations. Authority is injected, never inferred.
use super::{
    ordinary_observation::{
        claims_ordinary_protocol, validate_ordinary, OrdinaryObservationCapture,
    },
    sql::*,
    store::capture_in,
    *,
};
use anyhow::{ensure, Context, Result};
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
    /// Native requested input, not evidence of execution or factual truth.
    ActionInvocation,
    /// Independently read bytes, not evidence of their author's factual claims.
    ArtifactObservation,
}

pub const EVIDENCE_CHECK_VERSION: &str = "exact-json-string-rows-evidence-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputStatus {
    Complete,
    Oversized,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionInvocation {
    pub call_id: String,
    pub tool_name: String,
    pub input_status: InputStatus,
    pub input: serde_json::Value,
    pub input_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContentStatus {
    Complete,
    Missing,
    Refused,
    NonUtf8,
    Oversized,
    Unavailable,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactObservation {
    pub content_status: ContentStatus,
    pub artifact_text: Option<String>,
    pub artifact_digest: Option<String>,
    pub verdict: CheckVerdict,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criterion: Option<ConfirmedCriterion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_context: Option<super::chat_observation::ChatObservationContext>,
    pub originals: Vec<OriginalPassage>,
    pub receipts: Vec<ToolReceipt>,
    pub artifact: ArtifactCheck,
    #[serde(deserialize_with = "strict_references")]
    pub consulted: Vec<RevisionRef>,
    pub scopes: BTreeSet<String>,
    pub occurred_at: Option<String>,
}

// V7 stores original passage bytes once. Descriptors retain every other
// admitted attribute and bind the exact independent Evidence revision.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OriginalDescriptor {
    #[serde(deserialize_with = "strict_reference")]
    pub(crate) reference: RevisionRef,
    pub(crate) kind: PassageKind,
    pub(crate) line: u64,
    pub(crate) pointer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    occurred_at: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionV2 {
    identity: CaptureIdentity,
    binding: ExternalBinding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    criterion: Option<ConfirmedCriterion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chat_context: Option<super::chat_observation::ChatObservationContext>,
    originals: Vec<OriginalDescriptor>,
    receipts: Vec<ToolReceipt>,
    artifact: ArtifactCheck,
    consulted: Vec<RevisionRef>,
    scopes: BTreeSet<String>,
    occurred_at: Option<String>,
}
fn projection_text(input: &HostCapture, originals: &[RevisionRef], version: i64) -> Result<String> {
    if version == 6 {
        return Ok(serde_json::to_string(input)?);
    }
    ensure!(
        matches!(version, 7..=9) && originals.len() == input.originals.len(),
        "invalid host projection version"
    );
    Ok(serde_json::to_string(&ProjectionV2 {
        identity: input.identity.clone(),
        binding: input.binding.clone(),
        criterion: input.criterion.clone(),
        chat_context: input.chat_context.clone(),
        originals: input
            .originals
            .iter()
            .zip(originals)
            .map(|(p, r)| OriginalDescriptor {
                reference: r.clone(),
                kind: p.kind.clone(),
                line: p.line,
                pointer: p.pointer.clone(),
                occurred_at: p.occurred_at.clone(),
            })
            .collect(),
        receipts: input.receipts.clone(),
        artifact: input.artifact.clone(),
        consulted: input.consulted.clone(),
        scopes: input.scopes.clone(),
        occurred_at: input.occurred_at.clone(),
    })?)
}
/// Reconstruct only from the canonical receipt and exact immutable originals.
/// The receipt's original HostCapture hash authenticates descriptor metadata too.
pub(super) fn projection_input(conn: &Connection, projection: &Capture) -> Result<HostCapture> {
    projection_input_for_receipt(conn, projection, None)
}

fn projection_input_for_receipt(
    conn: &Connection,
    projection: &Capture,
    receipt: Option<&CaptureReceipt>,
) -> Result<HostCapture> {
    let (object, revision, originals, payload_hash, receipt_id): (String,String,String,String,String) = conn.query_row(
        "SELECT projection_object,projection_revision,originals,payload_hash,receipt_id FROM host_capture_receipt WHERE identity_hash=?1 AND state='active'",
        [&projection.source.id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).optional()?.context("host projection has no active receipt")?;
    let reference = RevisionRef { object, revision };
    let Some(Record::Evidence(saved)) = lookup(conn, &reference)? else {
        anyhow::bail!("host projection missing")
    };
    ensure!(
        serde_json::to_vec(&saved)? == serde_json::to_vec(projection)?,
        "noncanonical host projection"
    );
    let originals: Vec<RevisionRef> = serde_json::from_str(&originals)?;
    ensure!(
        receipt.is_none_or(|r| {
            r.id == receipt_id && r.projection == reference && r.originals == originals
        }),
        "capture receipt is not canonical"
    );
    let closure = closure(conn, &reference)?;
    ensure!(
        originals.iter().all(|r| closure.contains(r)),
        "missing original dependency"
    );
    let input = match projection.source.connector.as_str() {
        "host-capture-projection-v1" => serde_json::from_str::<HostCapture>(&projection.text)?,
        "host-capture-projection-v2" => {
            ensure!(
                matches!(
                    conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
                    7..=9
                ),
                "unsupported host projection version"
            );
            let p: ProjectionV2 = serde_json::from_str(&projection.text)?;
            ensure!(
                p.originals
                    .iter()
                    .map(|d| &d.reference)
                    .eq(originals.iter()),
                "noncanonical original descriptors"
            );
            let mut passages = Vec::new();
            for d in p.originals {
                let Some(Record::Evidence(original)) = lookup(conn, &d.reference)? else {
                    anyhow::bail!("missing original evidence")
                };
                ensure!(
                    original
                        .occurred_at
                        .as_deref()
                        .map(normalize_time)
                        .transpose()?
                        == d.occurred_at.as_deref().map(normalize_time).transpose()?,
                    "original descriptor mismatch"
                );
                passages.push(OriginalPassage {
                    source: original.source,
                    kind: d.kind,
                    line: d.line,
                    pointer: d.pointer,
                    text: original.text,
                    occurred_at: d.occurred_at,
                });
            }
            HostCapture {
                identity: p.identity,
                binding: p.binding,
                criterion: p.criterion,
                chat_context: p.chat_context,
                originals: passages,
                receipts: p.receipts,
                artifact: p.artifact,
                consulted: p.consulted,
                scopes: p.scopes,
                occurred_at: p.occurred_at,
            }
        }
        _ => anyhow::bail!("unsupported host projection version"),
    };
    ensure!(
        hash(&input)? == payload_hash
            && hash(&input.identity)? == projection.source.id
            && input.identity.account == projection.source.account,
        "host projection payload mismatch"
    );
    super::ordinary_observation::validate_context(
        &input.identity,
        input.criterion.as_ref(),
        input.chat_context.as_ref(),
    )?;
    if claims_ordinary_protocol(&input.identity) {
        ensure!(
            projection.source.connector == "host-capture-projection-v2"
                && matches!(
                    conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
                    8 | 9
                ),
            "ordinary observations require version 8"
        );
        validate_ordinary(&input)?;
        ensure!(
            input.consulted.iter().all(|r| closure.contains(r)),
            "missing ordinary consulted dependency"
        );
    }
    Ok(input)
}

/// Only active, exact v7/v8 projection receipts may acquire this presentation.
/// A source that merely uses the connector name remains ordinary evidence.
pub(super) fn native_reader(
    conn: &Connection,
    reference: &RevisionRef,
    projection: &Capture,
) -> Result<Option<super::interpretation_reader::NativeCaptureReaderV1>> {
    if projection.source.connector != "host-capture-projection-v2"
        || !matches!(
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
            7..=9
        )
    {
        return Ok(None);
    }
    let authentic: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM host_capture_receipt WHERE state='active' AND projection_object=?1 AND projection_revision=?2 AND identity_hash=?3)",
        params![reference.object, reference.revision, projection.source.id], |r| r.get(0))?;
    if !authentic {
        return Ok(None);
    }
    let input = projection_input(conn, projection)?;
    let descriptors: ProjectionV2 = serde_json::from_str(&projection.text)?;
    Ok(Some(super::interpretation_reader::NativeCaptureReaderV1 {
        identity: input.identity,
        criterion: input.criterion,
        chat_context: input.chat_context,
        originals: descriptors.originals,
        receipts: input.receipts,
        artifact: input.artifact,
        consulted: input.consulted,
        occurred_at: input.occurred_at,
    }))
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
pub(crate) fn strict_references<'de, D: serde::Deserializer<'de>>(
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
    if version >= 6 {
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
    pub(crate) fn require_host_capture_version(&self, rich: bool) -> Result<()> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(
            if rich {
                matches!(version, 8 | 9)
            } else {
                matches!(version, 6 | 7)
            },
            "host capture producer does not match store version"
        );
        Ok(())
    }
    pub fn create_host_capture(path: &Path, domain: &str) -> Result<Self> {
        Self::create_schema(path, domain, "CREATE TABLE external_binding(revision TEXT PRIMARY KEY REFERENCES revision(id),authority TEXT NOT NULL,locator TEXT NOT NULL,binding_revision TEXT NOT NULL);
          CREATE TABLE host_capture_receipt(identity_hash TEXT PRIMARY KEY,payload_hash TEXT,receipt_id TEXT,projection_object TEXT,projection_revision TEXT,originals TEXT,state TEXT NOT NULL CHECK(state IN ('active','forgotten')),lineage_version INTEGER NOT NULL CHECK(lineage_version=2));
          PRAGMA user_version=6;")
    }
    pub fn with_source_guard(mut self, guard: Arc<dyn ExternalSourceGuard>) -> Self {
        self.source_guard = Some(guard);
        self
    }
    /// Inspect an exact receipt using current store identity and source authority.
    /// Resolve an already retained ordinary seal; this never admits or retries it.
    pub(crate) fn retained_ordinary_capture(
        &self,
        input: &HostCapture,
        access: &Access,
    ) -> Result<CaptureReceipt> {
        self.check_access(access)?;
        validate_ordinary(input)?;
        let tx = self.conn.unchecked_transaction()?;
        let (id, object, revision, originals): (String, String, String, String) = tx.query_row(
            "SELECT receipt_id,projection_object,projection_revision,originals FROM host_capture_receipt WHERE identity_hash=?1 AND payload_hash=?2 AND state='active'",
            params![hash(&input.identity)?, hash(input)?],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).optional()?.context("ordinary capture has not been retained")?;
        let receipt = CaptureReceipt {
            id,
            projection: RevisionRef { object, revision },
            originals: serde_json::from_str(&originals)?,
        };
        let Some(Record::Evidence(projection)) = lookup(&tx, &receipt.projection)? else {
            anyhow::bail!("ordinary projection unavailable")
        };
        let canonical = projection_input_for_receipt(&tx, &projection, Some(&receipt))?;
        ensure!(
            hash(&canonical)? == hash(input)?,
            "ordinary seal differs from retained capture"
        );
        let checks = SourceChecks::new(self.source_guard.as_deref());
        for reference in receipt
            .originals
            .iter()
            .chain(std::iter::once(&receipt.projection))
        {
            current(&tx, reference)?;
            ensure!(
                fresh(&tx, reference, i64::MAX)?
                    && authorized_with_guard(&tx, reference, access, Some(&checks))?,
                "ordinary learning source unavailable"
            );
        }
        tx.commit()?;
        Ok(receipt)
    }

    pub fn capture_semantic_assessment(
        &self,
        receipt: &CaptureReceipt,
        access: &Access,
    ) -> Result<SemanticAssessment> {
        self.check_access(access)?;
        let live = self.reopen_read_only()?;
        let tx = live.conn.unchecked_transaction()?;
        let Some(Record::Evidence(projection)) = lookup(&tx, &receipt.projection)? else {
            anyhow::bail!("host projection missing")
        };
        let input = projection_input_for_receipt(&tx, &projection, Some(receipt))?;
        let bound: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM external_binding WHERE revision=?1 AND authority=?2 AND locator=?3 AND binding_revision=?4)",
            params![
                receipt.projection.revision,
                input.binding.authority,
                input.binding.locator,
                input.binding.revision,
            ],
            |r| r.get(0),
        )?;
        ensure!(
            bound
                && fresh(&tx, &receipt.projection, i64::MAX)?
                && authorized_with_guard(
                    &tx,
                    &receipt.projection,
                    access,
                    live.source_guard.as_deref()
                )?,
            "host source unavailable"
        );
        tx.commit()?;
        Ok(if claims_ordinary_protocol(&input.identity) {
            SemanticAssessment::NotEvaluated
        } else {
            SemanticAssessment::ArtifactCheck(input.artifact)
        })
    }

    pub(crate) fn admit_ordinary_observation(
        &mut self,
        input: OrdinaryObservationCapture,
        access: &Access,
    ) -> Result<CaptureReceipt> {
        admit(self, input.into_host(), access, Admission::Ordinary)
    }

    pub(crate) fn admit_host_capture(
        &mut self,
        input: HostCapture,
        access: &Access,
    ) -> Result<CaptureReceipt> {
        admit(self, input, access, Admission::Legacy)
    }
    pub(crate) fn admit_host_capture_with_evidence(
        &mut self,
        input: HostCapture,
        access: &Access,
    ) -> Result<CaptureReceipt> {
        admit(self, input, access, Admission::Rich)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Admission {
    Legacy,
    Rich,
    Ordinary,
}

fn admit(
    db: &mut Memory,
    input: HostCapture,
    access: &Access,
    policy: Admission,
) -> Result<CaptureReceipt> {
    db.check_access(access)?;
    ensure!(
        input.identity.domain == access.domain,
        "invalid host capture domain"
    );
    let version: i64 = db.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(
        match policy {
            Admission::Legacy => matches!(version, 6 | 7),
            Admission::Rich | Admission::Ordinary => matches!(version, 8 | 9),
        },
        "host capture producer does not match store version"
    );
    if policy != Admission::Ordinary {
        ensure!(
            !claims_ordinary_protocol(&input.identity),
            "ordinary observation requires ordinary admission"
        );
        validate(&input, if matches!(version, 8 | 9) { 64 } else { 32 }, 1)?;
        if policy == Admission::Rich {
            validate_rich(&input)?;
        } else {
            ensure!(
                input.originals.iter().all(|p| !matches!(
                    p.kind,
                    PassageKind::ActionInvocation | PassageKind::ArtifactObservation
                )),
                "rich evidence requires version 8"
            );
        }
    }
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
        // Every closure member must be sealed bridge evidence or a canonical
        // provisional derivative. Legacy Evidence may contain raw host lines.
        // Concept status stays provisional; recapture does not promote it.
        for member in closure(&tx, dependency)? {
            let safe: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM external_binding WHERE revision=?1)",
                [&member.revision],
                |r| r.get(0),
            )?;
            ensure!(
                (safe && matches!(lookup(&tx, &member)?, Some(Record::Evidence(_))))
                    || (matches!(version, 7..=9)
                        && interpretation::canonical(&tx, &member)?
                        && matches!(lookup(&tx, &member)?, Some(Record::Interpretation(_))))
                    || (matches!(version, 7..=9) && concept::canonical_native(&tx, &member)?),
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
        connector: if matches!(version, 7..=9) {
            "host-capture-projection-v2"
        } else {
            "host-capture-projection-v1"
        }
        .into(),
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
            text: projection_text(&input, &originals, version)?,
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
pub(super) fn validate(
    input: &HostCapture,
    max_consulted: usize,
    min_receipts: usize,
) -> Result<()> {
    let i = &input.identity;
    super::ordinary_observation::validate_context(
        i,
        input.criterion.as_ref(),
        input.chat_context.as_ref(),
    )?;
    ensure!(
        [
            &i.domain,
            &i.account,
            &i.session_id,
            &i.check_version,
            &i.evaluator_version
        ]
        .iter()
        .all(|s| identifier(s, 256))
            && [&i.prefix_digest, &i.check_id].iter().all(|s| digest(s))
            && valid_binding(&input.binding),
        "invalid host capture identifiers"
    );
    ensure!(
        (1..=16).contains(&input.originals.len())
            && (min_receipts..=32).contains(&input.receipts.len())
            && input.consulted.len() <= max_consulted,
        "invalid host capture bounds"
    );
    if let Some(criterion) = &input.criterion {
        ensure!(
            identifier(&criterion.goal, 256)
                && digest(&criterion.criterion_revision)
                && digest(&criterion.confirmation_digest)
                && [&criterion.goal_text, &criterion.criterion_text]
                    .iter()
                    .all(|s| !s.trim().is_empty() && s.len() <= 2048),
            "invalid host criterion"
        );
    }
    if let Some(context) = &input.chat_context {
        context.validate_originals(&input.originals)?;
    }
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
                && p.text.len()
                    <= if p.kind == PassageKind::ArtifactObservation {
                        16_384
                    } else {
                        4096
                    },
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

pub(super) fn validate_action(passage: &OriginalPassage) -> Result<ActionInvocation> {
    let value: super::plan_extraction::UniqueValue = serde_json::from_str(&passage.text)?;
    let action: ActionInvocation = serde_json::from_value(value.0)?;
    ensure!(
        passage.source.connector == "host-action-invocation-v1"
            && passage.pointer.ends_with("/input")
            && digest(&action.input_digest),
        "invalid action evidence"
    );
    let bytes = serde_json::to_vec(&action.input)?;
    match action.input_status {
        InputStatus::Complete => ensure!(
            bytes.len() <= 1024 && hash(&action.input)? == action.input_digest,
            "invalid complete action input"
        ),
        InputStatus::Oversized => {
            ensure!(action.input.is_null(), "invalid omitted action input")
        }
    }
    Ok(action)
}

fn validate_rich(input: &HostCapture) -> Result<()> {
    ensure!(
        input.identity.check_version == EVIDENCE_CHECK_VERSION,
        "invalid rich check version"
    );
    let mut actions = BTreeSet::new();
    let mut artifacts = 0;
    for passage in &input.originals {
        match passage.kind {
            PassageKind::ActionInvocation => {
                let action = validate_action(passage)?;
                ensure!(
                    actions.insert(action.call_id.clone())
                        && input.receipts.iter().any(|r| {
                            r.call_id == action.call_id && r.tool_name == action.tool_name
                        }),
                    "invalid action evidence"
                );
            }
            PassageKind::ArtifactObservation => {
                artifacts += 1;
                let value: super::plan_extraction::UniqueValue =
                    serde_json::from_str(&passage.text)?;
                let artifact: ArtifactObservation = serde_json::from_value(value.0)?;
                ensure!(
                    passage.source.connector == "host-artifact-observation-v1"
                        && passage.pointer == "/artifact_observation"
                        && artifact.artifact_digest == input.artifact.artifact_digest
                        && artifact.verdict == input.artifact.verdict,
                    "invalid artifact evidence"
                );
                if artifact.content_status == ContentStatus::Complete {
                    let text = artifact
                        .artifact_text
                        .as_deref()
                        .context("missing complete artifact")?;
                    let actual = Sha256::digest(text.as_bytes())
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>();
                    ensure!(
                        text.len() <= 2048
                            && artifact.artifact_digest.as_deref() == Some(actual.as_str()),
                        "invalid complete artifact"
                    );
                } else {
                    ensure!(
                        artifact.artifact_text.is_none(),
                        "noncomplete artifact has text"
                    );
                    match artifact.content_status {
                        ContentStatus::Missing | ContentStatus::Unavailable => ensure!(
                            artifact.artifact_digest.is_none()
                                && artifact.verdict == CheckVerdict::Unavailable,
                            "invalid unavailable artifact"
                        ),
                        ContentStatus::Refused => ensure!(
                            artifact.artifact_digest.is_none()
                                && artifact.verdict == CheckVerdict::Refused,
                            "invalid refused artifact"
                        ),
                        ContentStatus::NonUtf8 => ensure!(
                            artifact.artifact_digest.is_some()
                                && artifact.verdict == CheckVerdict::Failed,
                            "invalid non-UTF8 artifact"
                        ),
                        ContentStatus::Oversized => ensure!(
                            if artifact.artifact_digest.is_some() {
                                matches!(
                                    artifact.verdict,
                                    CheckVerdict::Passed | CheckVerdict::Failed
                                )
                            } else {
                                artifact.verdict == CheckVerdict::Refused
                            },
                            "invalid oversized artifact"
                        ),
                        ContentStatus::Complete => unreachable!(),
                    }
                }
            }
            _ => {}
        }
    }
    ensure!(
        artifacts == 1 && actions.len() == input.receipts.len(),
        "incomplete rich evidence"
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
