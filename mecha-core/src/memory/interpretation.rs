//! Host-bound provisional interpretations. No provider, sensor or action authority.
use super::{context::*, host_capture::*, sql::*, *};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[path = "support.rs"]
mod support;
pub use support::*;

#[path = "evidence_metadata.rs"]
mod evidence_metadata;
use evidence_metadata::evidence_metadata;
pub use evidence_metadata::*;

#[path = "native_evidence.rs"]
mod native_evidence;
pub use native_evidence::*;

const PRODUCER: &str = "situated-interpretation-v1";
const SCHEMA: &str = "CREATE TABLE external_binding(revision TEXT PRIMARY KEY REFERENCES revision(id),authority TEXT NOT NULL,locator TEXT NOT NULL,binding_revision TEXT NOT NULL);
CREATE TABLE host_capture_receipt(identity_hash TEXT PRIMARY KEY,payload_hash TEXT,receipt_id TEXT,projection_object TEXT,projection_revision TEXT,originals TEXT,state TEXT NOT NULL CHECK(state IN ('active','forgotten')),lineage_version INTEGER NOT NULL CHECK(lineage_version=2));
CREATE TABLE interpretation_receipt(request_id TEXT PRIMARY KEY,payload_hash TEXT,receipt_id TEXT,object_id TEXT,revision_id TEXT,context_key TEXT,state TEXT NOT NULL CHECK(state IN ('active','withdrawn','forgotten')));
CREATE INDEX interpretation_context ON interpretation_receipt(context_key,state);
PRAGMA user_version=7;";

#[derive(Clone, Debug)]
pub struct InterpretationContext {
    criterion: ConfirmedCriterion,
    host_context_id: String,
    key: String,
}
impl InterpretationContext {
    pub fn new(criterion: ConfirmedCriterion, host_context_id: String) -> Result<Self> {
        ensure!(
            identifier(&criterion.goal, 256)
                && identifier(&host_context_id, 256)
                && [&criterion.goal_text, &criterion.criterion_text]
                    .iter()
                    .all(|s| !s.trim().is_empty() && s.len() <= 2048)
                && [
                    &criterion.criterion_revision,
                    &criterion.confirmation_digest
                ]
                .iter()
                .all(|s| digest(s)),
            "invalid interpretation context"
        );
        let key = hash(&(&criterion, &host_context_id))?;
        Ok(Self {
            criterion,
            host_context_id,
            key,
        })
    }
    pub fn criterion(&self) -> &ConfirmedCriterion {
        &self.criterion
    }
    pub fn key(&self) -> &str {
        &self.key
    }
    pub fn read_policy(&self) -> InterpretationRead {
        InterpretationRead {
            context: self.clone(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct InterpretationRead {
    context: InterpretationContext,
}
impl InterpretationRead {
    pub fn criterion(&self) -> &ConfirmedCriterion {
        self.context.criterion()
    }
    pub fn key(&self) -> &str {
        self.context.key()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    Observation,
    AssistantStatement,
    ActionInvocation,
    ArtifactObservation,
    PriorInterpretation,
    /// Passage may repeat prior hypotheses; not independent evidence of their truth.
    InterpretationInfluenced,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceField {
    pub id: String,
    pub source: RevisionRef,
    pub field: String,
    pub text: String,
    pub kind: SourceKind,
}
#[derive(Clone, Debug)]
pub struct PreparedInterpretation {
    instance: String,
    access: Access,
    receipt: CaptureReceipt,
    context: InterpretationContext,
    snapshot: Snapshot,
    roots: Vec<RevisionRef>,
    sources: Vec<SourceField>,
    evidence_metadata: Vec<EvidenceMetadata>,
    generation_input: String,
    request_id: String,
}
impl PreparedInterpretation {
    pub(crate) fn request_identity(&self) -> &str {
        &self.request_id
    }
    pub fn context(&self) -> &InterpretationContext {
        &self.context
    }
    pub fn sources(&self) -> &[SourceField] {
        &self.sources
    }
    pub fn evidence_metadata(&self) -> &[EvidenceMetadata] {
        &self.evidence_metadata
    }
    pub fn generation_input(&self) -> &str {
        &self.generation_input
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterpretationClaim {
    pub source_id: String,
    pub statement: String,
    pub quote: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterpretationJudgment {
    pub goal: String,
    pub bearing: InterpretationBearing,
    pub because: Vec<usize>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InterpretationBearing {
    Good,
    Bad,
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundingReport {
    pub offered: usize,
    pub dropped: usize,
    pub dropped_by: BTreeMap<String, usize>,
    pub unresolved_goals: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterpretationDraft {
    pub interpretation: String,
    pub claims: Vec<InterpretationClaim>,
    pub judgments: Vec<InterpretationJudgment>,
    pub prediction: Option<String>,
    pub grounding: GroundingReport,
    pub model: String,
    pub producer_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub support: Option<Box<SupportReport>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimSource {
    pub id: String,
    pub source: RevisionRef,
    pub field: String,
    pub kind: SourceKind,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Interpretation {
    pub context_key: String,
    pub criterion: ConfirmedCriterion,
    pub host_context_id: String,
    pub draft: InterpretationDraft,
    pub claim_sources: Vec<ClaimSource>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterpretationReceipt {
    pub id: String,
    pub reference: RevisionRef,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InterpretationRole {
    SelectedCurrent,
    HistoricalDependency,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InterpretationRoleEntry {
    pub reference: RevisionRef,
    pub role: InterpretationRole,
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
fn require_v7(conn: &Connection) -> Result<()> {
    ensure!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? == 7,
        "interpretation requires version 7"
    );
    validate_schema(conn)
}
fn require_situated(conn: &Connection) -> Result<()> {
    ensure!(
        matches!(
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
            7..=9
        ),
        "interpretation preparation requires version 7 or 8"
    );
    validate_schema(conn)
}
pub(super) fn validate_schema(conn: &Connection) -> Result<()> {
    conn.prepare("SELECT request_id,payload_hash,receipt_id,object_id,revision_id,context_key,state FROM interpretation_receipt LIMIT 0").context("invalid interpretation schema")?;
    let invalid:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM interpretation_receipt i WHERE i.state='active' AND (i.payload_hash IS NULL OR i.receipt_id IS NULL OR i.context_key IS NULL OR NOT EXISTS(SELECT 1 FROM revision r WHERE r.id=i.revision_id AND r.object_id=i.object_id AND json_extract(r.payload,'$.kind')='interpretation' AND json_extract(r.payload,'$.data.context_key')=i.context_key)))",[],|r|r.get(0))?;
    ensure!(!invalid, "invalid interpretation schema");
    let mut stmt = conn
        .prepare("SELECT object_id,revision_id FROM interpretation_receipt WHERE state='active'")?;
    let refs = stmt
        .query_map([], |r| {
            Ok(RevisionRef {
                object: r.get(0)?,
                revision: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for reference in refs {
        ensure!(
            canonical(conn, &reference)?,
            "invalid interpretation provenance"
        );
    }
    Ok(())
}
pub(super) fn canonical(conn: &Connection, r: &RevisionRef) -> Result<bool> {
    if !matches!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
        7..=9
    ) {
        return Ok(false);
    }
    let saved: Option<(String,String)> = conn.query_row("SELECT payload_hash,context_key FROM interpretation_receipt WHERE revision_id=?1 AND object_id=?2 AND state='active'",params![r.revision,r.object],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
    let Some((payload_hash, key)) = saved else {
        return Ok(false);
    };
    let Some(record @ Record::Interpretation(_)) = lookup(conn, r)? else {
        return Ok(false);
    };
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if (matches!(version, 8 | 9) && serde_json::to_vec(&record)?.len() > 16384)
        || hash(&record)? != payload_hash
    {
        return Ok(false);
    }
    let Record::Interpretation(i) = record else {
        unreachable!()
    };
    let context = InterpretationContext::new(i.criterion.clone(), i.host_context_id.clone())?;
    if context.key() != i.context_key || i.context_key != key {
        return Ok(false);
    }
    let refs: Vec<_> = closure(conn, r)?
        .into_iter()
        .filter(|input| input != r)
        .collect();
    if refs.len() > 32 {
        return Ok(false);
    }
    // Validate this record against its immutable dependencies without recursively
    // revalidating every older interpretation. Schema/read walks validate those
    // records separately, keeping long histories bounded.
    let sources = source_fields_impl(conn, &refs, false)?;
    let valid = if matches!(version, 8 | 9) {
        support::validate_evaluated_draft(&context, &sources, &i.draft)
    } else {
        validate_draft(&context, &sources, &i.draft)
    };
    if valid.is_err() {
        return Ok(false);
    }
    Ok(serde_json::to_vec(&claim_sources_for(&sources, &i.draft))?
        == serde_json::to_vec(&i.claim_sources)?)
}
/// A default read must not leak a copied hypothesis through a later observation.
pub(super) fn contains_interpretation(conn: &Connection, r: &RevisionRef) -> Result<bool> {
    Ok(conn.query_row("WITH RECURSIVE inputs(id) AS (SELECT ?1 UNION SELECT d.input FROM dependency d JOIN inputs i ON d.output=i.id) SELECT EXISTS(SELECT 1 FROM inputs JOIN revision r ON r.id=inputs.id JOIN object o ON o.id=r.object_id WHERE o.kind='interpretation')",[&r.revision],|r|r.get(0))?)
}
/// A recaptured provisional concept is derived meaning too. Keep this distinct
/// from default interpretation withholding: provisional concepts remain readable.
pub(super) fn contains_hypothesis(conn: &Connection, r: &RevisionRef) -> Result<bool> {
    Ok(conn.query_row("WITH RECURSIVE inputs(id) AS (SELECT ?1 UNION SELECT d.input FROM dependency d JOIN inputs i ON d.output=i.id) SELECT EXISTS(SELECT 1 FROM inputs JOIN revision r ON r.id=inputs.id JOIN object o ON o.id=r.object_id WHERE o.kind IN ('interpretation','concept'))",[&r.revision],|r|r.get(0))?)
}
fn authenticate_receipt(conn: &Connection, r: &CaptureReceipt) -> Result<()> {
    let row=conn.query_row("SELECT projection_object,projection_revision,originals FROM host_capture_receipt WHERE receipt_id=?1 AND state='active'",[&r.id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?))).optional()?.context("capture receipt unavailable")?;
    let originals: Vec<RevisionRef> = serde_json::from_str(&row.2)?;
    ensure!(
        r.projection.object == row.0 && r.projection.revision == row.1 && r.originals == originals,
        "capture receipt is not canonical"
    );
    Ok(())
}
fn check_roots(
    conn: &Connection,
    roots: &[RevisionRef],
    snapshot: &Snapshot,
    access: &Access,
    guard: Option<&dyn ExternalSourceGuard>,
) -> Result<Vec<RevisionRef>> {
    check_roots_bounded(conn, roots, snapshot, access, guard, 32)
}
fn check_roots_bounded(
    conn: &Connection,
    roots: &[RevisionRef],
    snapshot: &Snapshot,
    access: &Access,
    guard: Option<&dyn ExternalSourceGuard>,
    limit: usize,
) -> Result<Vec<RevisionRef>> {
    let mut all = BTreeMap::new();
    let checks = SourceChecks::new(guard);
    for root in roots {
        current(conn, root)?;
        ensure!(
            fresh(conn, root, snapshot.sequence)?
                && fresh(conn, root, i64::MAX)?
                && authorized_with_guard(conn, root, access, Some(&checks))?,
            "interpretation source unavailable"
        );
        for member in closure(conn, root)? {
            ensure!(
                lookup(conn, &member)?.is_some(),
                "interpretation source missing"
            );
            all.insert((member.object.clone(), member.revision.clone()), member);
        }
    }
    ensure!(
        all.len() <= limit,
        "interpretation lineage exceeds {limit} records"
    );
    Ok(all.into_values().collect())
}
fn source_fields(conn: &Connection, refs: &[RevisionRef]) -> Result<Vec<SourceField>> {
    source_fields_impl(conn, refs, true)
}
fn source_fields_impl(
    conn: &Connection,
    refs: &[RevisionRef],
    check_interpretations: bool,
) -> Result<Vec<SourceField>> {
    let mut out = vec![];
    for r in refs {
        let record = lookup(conn, r)?.context("missing source")?;
        let mut fields: Vec<(String, String, SourceKind)> = vec![];
        match record {
            Record::Interpretation(i) => {
                ensure!(
                    !check_interpretations || canonical(conn, r)?,
                    "noncanonical interpretation"
                );
                fields.push((
                    "/criterion".into(),
                    serde_json::to_string(&i.criterion)?,
                    SourceKind::PriorInterpretation,
                ));
                fields.push((
                    "/interpretation".into(),
                    i.draft.interpretation,
                    SourceKind::PriorInterpretation,
                ));
                for (n, c) in i.draft.claims.into_iter().enumerate() {
                    fields.push((
                        format!("/claims/{n}/statement"),
                        c.statement,
                        SourceKind::PriorInterpretation,
                    ));
                }
            }
            Record::Evidence(e) => {
                let bound: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM external_binding WHERE revision=?1)",
                    [&r.revision],
                    |r| r.get(0),
                )?;
                ensure!(bound, "interpretation requires sealed bridge evidence");
                if matches!(
                    e.source.connector.as_str(),
                    "host-capture-projection-v1" | "host-capture-projection-v2"
                ) {
                    let h = projection_input(conn, &e)?;
                    if let Some(criterion) = &h.criterion {
                        fields.push((
                            "/criterion".into(),
                            serde_json::to_string(criterion)?,
                            SourceKind::Observation,
                        ));
                    }
                    if let Some(context) = &h.chat_context {
                        fields.push((
                            "/chat_context".into(),
                            serde_json::to_string(context)?,
                            SourceKind::Observation,
                        ));
                    }
                    fields.push((
                        "/receipts".into(),
                        serde_json::to_string(&h.receipts)?,
                        SourceKind::Observation,
                    ));
                    fields.push((
                        "/artifact".into(),
                        serde_json::to_string(&h.artifact)?,
                        SourceKind::Observation,
                    ));
                } else {
                    // The bridge stores original text separately. Recover its original
                    // pointer and speaker from the authenticated projection, never text.
                    let projection:Option<String>=conn.query_row("SELECT r.payload FROM host_capture_receipt h JOIN revision r ON r.id=h.projection_revision WHERE h.state='active' AND EXISTS(SELECT 1 FROM json_each(h.originals) j WHERE json_extract(j.value,'$.revision')=?1) ORDER BY r.seq LIMIT 1",[&r.revision],|row|row.get(0)).optional()?;
                    let payload = projection.context("original has no canonical capture")?;
                    let Record::Evidence(projection) = serde_json::from_str(&payload)? else {
                        anyhow::bail!("invalid capture projection")
                    };
                    let h = projection_input(conn, &projection)?;
                    let p = h
                        .originals
                        .into_iter()
                        .find(|p| {
                            serde_json::to_string(&p.source).ok()
                                == serde_json::to_string(&e.source).ok()
                                && p.text == e.text
                        })
                        .context("original does not match projection")?;
                    let kind = if contains_hypothesis(conn, r)? {
                        SourceKind::InterpretationInfluenced
                    } else {
                        match p.kind {
                            PassageKind::AssistantText => SourceKind::AssistantStatement,
                            PassageKind::ActionInvocation => SourceKind::ActionInvocation,
                            PassageKind::ArtifactObservation => SourceKind::ArtifactObservation,
                            _ => SourceKind::Observation,
                        }
                    };
                    fields.push((p.pointer, e.text, kind));
                }
            }
            _ => anyhow::bail!("unsupported interpretation input"),
        }
        for (field, text, kind) in fields {
            let id = format!("source:{}", hash(&(r, &field))?);
            out.push(SourceField {
                id,
                source: r.clone(),
                field,
                text,
                kind,
            });
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    ensure!(out.len() <= 64, "too many interpretation source fields");
    Ok(out)
}
fn claim_sources_for(sources: &[SourceField], draft: &InterpretationDraft) -> Vec<ClaimSource> {
    let claimed: BTreeSet<_> = draft.claims.iter().map(|c| c.source_id.as_str()).collect();
    sources
        .iter()
        .filter(|s| claimed.contains(s.id.as_str()))
        .map(|s| ClaimSource {
            id: s.id.clone(),
            source: s.source.clone(),
            field: s.field.clone(),
            kind: s.kind.clone(),
        })
        .collect()
}

fn validate_draft(
    context: &InterpretationContext,
    sources: &[SourceField],
    d: &InterpretationDraft,
) -> Result<()> {
    ensure!(d.support.is_none(), "legacy support metadata refused");
    ensure!(
        !d.interpretation.trim().is_empty()
            && d.interpretation.len() <= 8192
            && d.claims.len() <= 16
            && d.judgments.len() <= 16
            && d.prediction.as_ref().is_none_or(|s| s.len() <= 2000)
            && identifier(&d.model, 256)
            && identifier(&d.producer_version, 256)
            && serde_json::to_vec(d)?.len() <= 16384,
        "invalid interpretation bounds"
    );
    ensure!(
        d.grounding.offered <= 64
            && Some(d.grounding.offered) == d.claims.len().checked_add(d.grounding.dropped)
            && d.grounding
                .dropped_by
                .values()
                .try_fold(0usize, |a, b| a.checked_add(*b))
                == Some(d.grounding.dropped)
            && d.grounding.dropped_by.keys().all(|k| identifier(k, 64))
            && d.grounding.unresolved_goals <= 16,
        "invalid grounding counts"
    );
    for c in &d.claims {
        ensure!(
            !c.statement.trim().is_empty()
                && c.statement.chars().count() <= 2000
                && (12..=300).contains(&c.quote.chars().count()),
            "invalid grounded claim"
        );
        let s = sources
            .iter()
            .find(|s| s.id == c.source_id)
            .context("unknown qualified source")?;
        ensure!(
            s.text.contains(&c.quote),
            "quote is not an exact source substring"
        );
    }
    let mut goals = BTreeSet::new();
    for j in &d.judgments {
        ensure!(
            j.goal == context.criterion.goal && goals.insert(&j.goal),
            "unknown or duplicate interpretation goal"
        );
        ensure!(
            !j.because.is_empty()
                && j.because.len() <= 16
                && j.because.iter().all(|n| *n < d.claims.len())
                && j.because.iter().collect::<BTreeSet<_>>().len() == j.because.len(),
            "unsupported interpretation judgment"
        );
    }
    Ok(())
}
impl Memory {
    pub fn create_situated(path: &Path, domain: &str) -> Result<Self> {
        Self::create_schema(path, domain, SCHEMA)
    }
    pub fn create_supported_situated(path: &Path, domain: &str) -> Result<Self> {
        Self::create_supported_situated_with_storage(
            path,
            domain,
            StorageAccess::plaintext_fixture(),
        )
    }

    pub fn create_supported_situated_with_storage(
        path: &Path,
        domain: &str,
        storage: StorageAccess,
    ) -> Result<Self> {
        Self::create_schema_with_storage(
            path,
            domain,
            &SCHEMA.replace("user_version=7", "user_version=8"),
            storage,
        )
    }

    pub(super) fn live_identity(&self) -> Result<()> {
        self.reopen_read_only()?;
        Ok(())
    }
    pub fn prepare_interpretation(
        &self,
        receipt: &CaptureReceipt,
        previous: &[RevisionRef],
        context: &InterpretationContext,
        snapshot: &Snapshot,
        access: &Access,
    ) -> Result<PreparedInterpretation> {
        self.check_access(access)?;
        self.live_identity()?;
        let tx = self.conn.unchecked_transaction()?;
        require_situated(&tx)?;
        ensure!(
            snapshot.instance == self.instance
                && snapshot.sequence >= 0
                && snapshot.sequence <= sequence(&tx)?,
            "invalid interpretation snapshot"
        );
        ensure!(previous.len() < 16, "at most 16 interpretation roots");
        authenticate_receipt(&tx, receipt)?;
        let mut roots = vec![receipt.projection.clone()];
        roots.extend(previous.iter().cloned());
        roots.sort_by(|a, b| (&a.object, &a.revision).cmp(&(&b.object, &b.revision)));
        roots.dedup();
        let refs = check_roots(&tx, &roots, snapshot, access, self.source_guard.as_deref())?;
        let sources = source_fields(&tx, &refs)?;
        let evidence_metadata = evidence_metadata(&tx, &refs, &receipt.projection, &sources)?;
        let generation_input = serde_json::to_string(
            &serde_json::json!({"criterion":context.criterion,"sources":sources}),
        )?;
        ensure!(
            generation_input.len() <= 32768,
            "oversized interpretation packet"
        );
        let request_id = hash(&(
            &self.instance,
            &access.domain,
            &access.scopes,
            receipt,
            context.key(),
            snapshot,
            &roots,
            &generation_input,
            PRODUCER,
        ))?;
        check_roots(&tx, &roots, snapshot, access, self.source_guard.as_deref())?;
        tx.commit()?;
        Ok(PreparedInterpretation {
            instance: self.instance.clone(),
            access: access.clone(),
            receipt: receipt.clone(),
            context: context.clone(),
            snapshot: snapshot.clone(),
            roots,
            sources,
            evidence_metadata,
            generation_input,
            request_id,
        })
    }
    pub fn retain_interpretation(
        &mut self,
        p: PreparedInterpretation,
        draft: InterpretationDraft,
        access: &Access,
    ) -> Result<InterpretationReceipt> {
        self.check_access(access)?;
        self.live_identity()?;
        ensure!(
            p.instance == self.instance
                && p.access.domain == access.domain
                && p.access.scopes == access.scopes,
            "foreign interpretation preparation"
        );
        validate_draft(&p.context, &p.sources, &draft)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_v7(&tx)?;
        authenticate_receipt(&tx, &p.receipt)?;
        let refs = check_roots(
            &tx,
            &p.roots,
            &p.snapshot,
            access,
            self.source_guard.as_deref(),
        )?;
        ensure!(
            serde_json::to_vec(&source_fields(&tx, &refs)?)? == serde_json::to_vec(&p.sources)?,
            "prepared source fields changed"
        );
        ensure!(
            evidence_metadata(&tx, &refs, &p.receipt.projection, &p.sources)?
                == p.evidence_metadata,
            "prepared evidence metadata changed"
        );
        let claim_sources = claim_sources_for(&p.sources, &draft);
        let record = Record::Interpretation(Interpretation {
            context_key: p.context.key.clone(),
            criterion: p.context.criterion.clone(),
            host_context_id: p.context.host_context_id.clone(),
            draft,
            claim_sources,
        });
        let payload = hash(&record)?;
        let existing=tx.query_row("SELECT payload_hash,receipt_id,object_id,revision_id,state FROM interpretation_receipt WHERE request_id=?1",[&p.request_id],|r|Ok((r.get::<_,Option<String>>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,String>(4)?))).optional()?;
        let receipt = if let Some((old, id, object, revision, state)) = existing {
            ensure!(
                state == "active" && old.as_deref() == Some(&payload),
                "interpretation retry conflict or unavailable"
            );
            let reference = RevisionRef {
                object: object.context("missing interpretation object")?,
                revision: revision.context("missing interpretation revision")?,
            };
            current(&tx, &reference)?;
            InterpretationReceipt {
                id: id.context("missing interpretation receipt")?,
                reference,
            }
        } else {
            let mut scopes = BTreeSet::new();
            for r in &refs {
                scopes.extend(required_scopes(&tx, &r.revision)?);
            }
            let reference = append(&tx, None, &record, &scopes, &p.roots, "active")?;
            let receipt = InterpretationReceipt {
                id: uuid::Uuid::new_v4().to_string(),
                reference,
            };
            tx.execute(
                "INSERT INTO interpretation_receipt VALUES (?1,?2,?3,?4,?5,?6,'active')",
                params![
                    p.request_id,
                    payload,
                    receipt.id,
                    receipt.reference.object,
                    receipt.reference.revision,
                    p.context.key
                ],
            )?;
            receipt
        };
        check_roots(
            &tx,
            &p.roots,
            &p.snapshot,
            access,
            self.source_guard.as_deref(),
        )?;
        tx.commit()?;
        Ok(receipt)
    }
    pub fn withdraw_interpretation(
        &mut self,
        expected: &RevisionRef,
        access: &Access,
    ) -> Result<RevisionRef> {
        self.check_access(access)?;
        self.live_identity()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_situated(&tx)?;
        ensure!(
            canonical(&tx, expected)?
                && authorized_with_guard(&tx, expected, access, self.source_guard.as_deref())?,
            "interpretation unavailable"
        );
        current(&tx, expected)?;
        let record = lookup(&tx, expected)?.context("missing interpretation")?;
        let reference = append(
            &tx,
            Some(expected),
            &record,
            &required_scopes(&tx, &expected.revision)?,
            &dependencies(&tx, &expected.revision)?,
            "withdrawn",
        )?;
        tx.execute(
            "UPDATE interpretation_receipt SET state='withdrawn' WHERE revision_id=?1",
            [&expected.revision],
        )?;
        tx.commit()?;
        Ok(reference)
    }
    /// Host-only canonical audit read, independent of the model packet budget.
    /// A serialized receipt is a lookup key, never source authority.
    pub(crate) fn stored_interpretation(
        &self,
        receipt: &InterpretationReceipt,
        access: &Access,
        context: &InterpretationContext,
    ) -> Result<Interpretation> {
        self.check_access(access)?;
        self.live_identity()?;
        let read = |conn: &Connection| -> Result<Interpretation> {
            require_situated(conn)?;
            let matches: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM interpretation_receipt WHERE receipt_id=?1 AND object_id=?2 AND revision_id=?3 AND context_key=?4 AND state='active')",
                params![receipt.id, receipt.reference.object, receipt.reference.revision, context.key()],
                |row| row.get(0),
            )?;
            ensure!(matches, "interpretation receipt unavailable");
            let checks = SourceChecks::new(self.source_guard.as_deref());
            let refs = closure(conn, &receipt.reference)?;
            for reference in refs {
                ensure!(
                    authorized_with_guard(conn, &reference, access, Some(&checks))?
                        && fresh(conn, &reference, i64::MAX)?,
                    "interpretation source unavailable"
                );
                if matches!(lookup(conn, &reference)?, Some(Record::Interpretation(_))) {
                    ensure!(canonical(conn, &reference)?, "noncanonical interpretation");
                }
            }
            let Some(Record::Interpretation(value)) = lookup(conn, &receipt.reference)? else {
                anyhow::bail!("interpretation unavailable")
            };
            ensure!(
                value.context_key == context.key(),
                "interpretation context mismatch"
            );
            Ok(value)
        };
        let tx = self.conn.unchecked_transaction()?;
        let selected = read(&tx)?;
        tx.commit()?;
        let live = self.reopen_read_only()?;
        let tx = live.conn.unchecked_transaction()?;
        let current = read(&tx)?;
        ensure!(
            serde_json::to_vec(&selected)? == serde_json::to_vec(&current)?,
            "interpretation changed"
        );
        tx.commit()?;
        Ok(current)
    }

    pub fn context_with_interpretations(
        &self,
        request: &ContextRequest,
        policy: &ContextPolicy,
        situation: &Situation,
        read: &InterpretationRead,
    ) -> Result<ContextPacket> {
        ensure!(
            situation.goal == read.criterion().goal,
            "interpretation goal mismatch"
        );
        self.live_identity()?;
        let mut packet = self.context(request, policy, situation)?;
        let tx = self.conn.unchecked_transaction()?;
        require_situated(&tx)?;
        let mut statement=tx.prepare("SELECT i.object_id,i.revision_id FROM interpretation_receipt i JOIN revision r ON r.id=i.revision_id WHERE i.state='active' AND i.context_key=?1 AND r.seq<=?2 ORDER BY r.seq DESC,r.id")?;
        let roots = statement
            .query_map(params![read.key(), policy.recall.known_at.sequence], |r| {
                Ok(RevisionRef {
                    object: r.get(0)?,
                    revision: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        let checks = SourceChecks::new(self.source_guard.as_deref());
        let budget = policy.recall.max_bytes.min(20_000);
        let mut selected = vec![];
        'roots: for root in roots {
            if task::contains_task(&tx, &root)?
                || !fresh(&tx, &root, i64::MAX)?
                || !fresh(&tx, &root, policy.recall.known_at.sequence)?
                || !authorized_with_guard(&tx, &root, &policy.access, Some(&checks))?
            {
                continue;
            }
            let Some(Record::Interpretation(i)) = lookup(&tx, &root)? else {
                continue;
            };
            if i.context_key != read.key()
                || hash(&(&i.criterion, &i.host_context_id))? != read.key()
            {
                continue;
            }
            let refs = closure(&tx, &root)?;
            if refs.len() > 32 {
                packet.budget_omitted = true;
                continue;
            }
            // Host source filters constrain every dependency, just as in the
            // default knowledge reader. A context match cannot broaden them.
            for r in &refs {
                if let Some(Record::Evidence(e)) = lookup(&tx, r)? {
                    if !matches_filters(&e, &policy.recall)? {
                        continue 'roots;
                    }
                }
            }
            let mut candidate = packet.clone();
            for r in refs {
                if candidate.records.iter().any(|h| h.reference == r) {
                    continue;
                }
                let record = lookup(&tx, &r)?.context("missing interpretation dependency")?;
                if matches!(&record, Record::Interpretation(_)) {
                    ensure!(
                        canonical(&tx, &r)?,
                        "noncanonical historical interpretation"
                    );
                    candidate
                        .interpretation_roles
                        .push(InterpretationRoleEntry {
                            reference: r.clone(),
                            role: InterpretationRole::HistoricalDependency,
                        });
                }
                candidate.records.push(Hit {
                    sequence: tx.query_row(
                        "SELECT seq FROM revision WHERE id=?1",
                        [&r.revision],
                        |row| row.get(0),
                    )?,
                    required_scopes: required_scopes(&tx, &r.revision)?,
                    reference: r,
                    record,
                    possible_time: false,
                    private_data: true,
                    untrusted_input: true,
                });
            }
            let role = candidate
                .interpretation_roles
                .iter_mut()
                .find(|r| r.reference == root)
                .context("missing selected interpretation role")?;
            role.role = InterpretationRole::SelectedCurrent;
            candidate.prepare(&tx)?;
            if candidate.records.len() > 64 || serde_json::to_vec(&candidate)?.len() > budget {
                packet.budget_omitted = true;
                continue;
            }
            packet = candidate;
            selected.push(root);
            if selected.len() == 2 {
                break;
            }
        }
        // Final authority is independent of selection, including uncited ancestors.
        let checks = SourceChecks::new(self.source_guard.as_deref());
        let mut valid = true;
        for hit in &packet.records {
            valid &= authorized_with_guard(&tx, &hit.reference, &policy.access, Some(&checks))?
                && fresh(&tx, &hit.reference, i64::MAX)?;
        }
        if !valid {
            packet.clear_records();
            packet.bindings.clear();
            packet.interpretation_roles.clear();
        }
        validate_roles(&packet, read, &selected)?;
        if packet.records.len() > 64 || serde_json::to_vec(&packet)?.len() > budget {
            packet.clear_records();
            packet.bindings.clear();
            packet.interpretation_roles.clear();
            packet.budget_omitted = true;
        }
        ensure!(
            serde_json::to_vec(&packet)?.len() <= budget,
            "context envelope exceeds packet budget"
        );
        tx.commit()?;
        // Reopen after ending selection's snapshot: a concurrent local source
        // update is as disqualifying as an external guard withdrawal.
        let live = self.reopen_read_only()?;
        let final_tx = live.conn.unchecked_transaction()?;
        let final_checks = SourceChecks::new(self.source_guard.as_deref());
        for hit in &packet.records {
            if !authorized_with_guard(
                &final_tx,
                &hit.reference,
                &policy.access,
                Some(&final_checks),
            )? || !fresh(&final_tx, &hit.reference, i64::MAX)?
            {
                packet.clear_records();
                packet.bindings.clear();
                packet.interpretation_roles.clear();
                break;
            }
        }
        final_tx.commit()?;
        Ok(packet)
    }
}
fn matches_filters(e: &Capture, query: &RecallQuery) -> Result<bool> {
    if query
        .account
        .as_ref()
        .is_some_and(|a| *a != e.source.account)
        || query
            .session_id
            .as_ref()
            .is_some_and(|id| e.transcript.as_ref().is_none_or(|t| t.session_id != *id))
    {
        return Ok(false);
    }
    if query.occurred_after.is_some() || query.occurred_before.is_some() {
        let Some(at) = e.occurred_at.as_deref().map(normalize_time).transpose()? else {
            return Ok(false);
        };
        let after = query
            .occurred_after
            .as_deref()
            .map(normalize_time)
            .transpose()?;
        let before = query
            .occurred_before
            .as_deref()
            .map(normalize_time)
            .transpose()?;
        if after.as_ref().is_some_and(|a| &at < a) || before.as_ref().is_some_and(|b| &at >= b) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn validate_roles(
    packet: &ContextPacket,
    read: &InterpretationRead,
    selected: &[RevisionRef],
) -> Result<()> {
    let mut seen = BTreeSet::new();
    for role in &packet.interpretation_roles {
        ensure!(
            seen.insert((&role.reference.object, &role.reference.revision)),
            "duplicate interpretation role"
        );
        let hit = packet
            .records
            .iter()
            .find(|h| h.reference == role.reference)
            .context("foreign interpretation role")?;
        let Record::Interpretation(i) = &hit.record else {
            anyhow::bail!("role on observation")
        };
        if role.role == InterpretationRole::SelectedCurrent {
            ensure!(
                i.context_key == read.key() && selected.contains(&role.reference),
                "inconsistent interpretation role"
            );
        }
    }
    ensure!(
        packet
            .records
            .iter()
            .filter(|h| matches!(h.record, Record::Interpretation(_)))
            .count()
            == seen.len(),
        "missing interpretation role"
    );
    Ok(())
}
pub(super) fn invalidate_forgotten(conn: &Connection) -> Result<()> {
    if matches!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
        7..=9
    ) {
        conn.execute("UPDATE interpretation_receipt SET state='forgotten',payload_hash=NULL,receipt_id=NULL,object_id=NULL,revision_id=NULL,context_key=NULL WHERE object_id IN (SELECT id FROM doomed)",[])?;
    }
    Ok(())
}
