//! Revision-backed, synthetic-only model-attempt ledger. This module grants no
//! inference or action authority; only a live reservation may settle a result.
use super::{durable, sql::*, *};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub(super) const SCHEMA: &str = "
CREATE TABLE durable_model (
 id TEXT PRIMARY KEY, generation INTEGER NOT NULL DEFAULT 0,
 read_id TEXT NOT NULL UNIQUE, read_generation INTEGER NOT NULL,
 access_digest TEXT NOT NULL,
 phase TEXT NOT NULL CHECK(phase IN ('active','finalized','cancelled','forgotten')),
 contract_object TEXT, contract_revision TEXT, decision_object TEXT, decision_revision TEXT
);
CREATE TABLE durable_model_attempt (
 model_id TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('initial','retry')),
 phase TEXT NOT NULL CHECK(phase IN ('unused','indeterminate','settled','assessed')),
 retry_requested INTEGER NOT NULL DEFAULT 0 CHECK(retry_requested IN (0,1)),
 request_object TEXT, request_revision TEXT, result_object TEXT, result_revision TEXT,
 assessment_object TEXT, assessment_revision TEXT,
 PRIMARY KEY(model_id,kind)
);
PRAGMA user_version=4;";

#[derive(Debug)]
pub struct ModelReceipt {
    pub(super) instance: String,
    pub(super) id: String,
    pub(super) generation: i64,
}
impl ModelReceipt {
    pub fn id(&self) -> &str {
        &self.id
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptKind {
    Initial,
    Retry,
}
impl AttemptKind {
    fn key(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::Retry => "retry",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelPhase {
    Active,
    Finalized,
    Cancelled,
    Forgotten,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptPhase {
    Unused,
    Indeterminate,
    Settled,
    Assessed,
}
#[derive(Clone, Debug, Serialize)]
pub struct AttemptStatus {
    pub phase: AttemptPhase,
    pub request: Option<RevisionRef>,
    pub result: Option<RevisionRef>,
    pub assessment: Option<RevisionRef>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ModelStatus {
    pub id: String,
    pub phase: ModelPhase,
    pub read_id: String,
    pub contract: Option<RevisionRef>,
    pub decision: Option<RevisionRef>,
    pub initial: AttemptStatus,
    pub retry: AttemptStatus,
    pub cancelled: bool,
}
#[derive(Debug, Serialize)]
pub struct AttemptData {
    pub request: Value,
    pub result: Option<Value>,
    pub assessment: Option<Value>,
}
pub(crate) struct ModelRead {
    pub task: RevisionRef,
    pub observation: RevisionRef,
    pub content: String,
    pub sha256: String,
}
pub(crate) struct HostModelResult {
    payload: Value,
}
impl HostModelResult {
    pub(crate) fn new(payload: Value) -> Self {
        Self { payload }
    }
}
pub(crate) struct ModelPermit {
    receipt: ModelReceipt,
    kind: AttemptKind,
}
/// A fresh cooperative safe point. Construction requires a live, one-use permit.
/// No database transaction or connection survives a check or spans inference.
pub struct ModelGuard {
    path: PathBuf,
    receipt: ModelReceipt,
    kind: AttemptKind,
    access: Access,
}
impl ModelGuard {
    pub fn check(&self) -> Result<()> {
        let db = Memory::open_read_only(&self.path)?;
        let tx = db.conn.unchecked_transaction()?;
        let row = checked(&tx, &db.instance, &self.receipt, &self.access)?;
        check_content(&tx, &row, &self.access)?;
        ensure!(
            row.status.phase == ModelPhase::Active,
            "model attempt cancelled or terminal"
        );
        ensure!(
            slot(&row.status, self.kind).phase == AttemptPhase::Indeterminate,
            "model attempt no longer pending"
        );
        tx.commit()?;
        Ok(())
    }
}
struct Row {
    generation: i64,
    read_generation: i64,
    digest: String,
    status: ModelStatus,
    initial_retry: bool,
}
fn reference(object: Option<String>, revision: Option<String>) -> Option<RevisionRef> {
    object
        .zip(revision)
        .map(|(object, revision)| RevisionRef { object, revision })
}
fn require_v4(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(
        matches!(version, 4 | 5),
        "model lifecycle requires an explicitly created version 4 or 5 store"
    );
    Ok(())
}
fn bounded(value: &Value, limit: usize) -> Result<()> {
    ensure!(
        serde_json::to_vec(value)?.len() <= limit,
        "oversized model lifecycle payload"
    );
    Ok(())
}
fn copy(r: &ModelReceipt) -> ModelReceipt {
    ModelReceipt {
        instance: r.instance.clone(),
        id: r.id.clone(),
        generation: r.generation,
    }
}
fn slot(s: &ModelStatus, k: AttemptKind) -> &AttemptStatus {
    match k {
        AttemptKind::Initial => &s.initial,
        AttemptKind::Retry => &s.retry,
    }
}
fn attempt(conn: &Connection, id: &str, k: AttemptKind) -> Result<(AttemptStatus, bool)> {
    conn.query_row("SELECT phase,request_object,request_revision,result_object,result_revision,assessment_object,assessment_revision,retry_requested FROM durable_model_attempt WHERE model_id=?1 AND kind=?2",params![id,k.key()],|r| {
        let phase:String=r.get(0)?;
        Ok((phase,reference(r.get(1)?,r.get(2)?),reference(r.get(3)?,r.get(4)?),reference(r.get(5)?,r.get(6)?),r.get::<_,bool>(7)?))
    }).map_err(Into::into).and_then(|(phase,request,result,assessment,retry)|Ok((AttemptStatus{phase:match phase.as_str(){"unused"=>AttemptPhase::Unused,"indeterminate"=>AttemptPhase::Indeterminate,"settled"=>AttemptPhase::Settled,"assessed"=>AttemptPhase::Assessed,_=>anyhow::bail!("unknown model attempt phase")},request,result,assessment},retry)))
}
fn load(conn: &Connection, id: &str) -> Result<Row> {
    let (generation,read_id,read_generation,digest,phase,contract,decision)=conn.query_row("SELECT generation,read_id,read_generation,access_digest,phase,contract_object,contract_revision,decision_object,decision_revision FROM durable_model WHERE id=?1",[id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,reference(r.get(5)?,r.get(6)?),reference(r.get(7)?,r.get(8)?)))).optional()?.context("unknown durable model binding")?;
    let mut phase = match phase.as_str() {
        "active" => ModelPhase::Active,
        "finalized" => ModelPhase::Finalized,
        "cancelled" => ModelPhase::Cancelled,
        "forgotten" => ModelPhase::Forgotten,
        _ => anyhow::bail!("unknown model phase"),
    };
    let cancelled: bool = conn.query_row(
        "SELECT phase='cancelled' FROM durable_task WHERE id=?1",
        [&read_id],
        |r| r.get(0),
    )?;
    if cancelled && phase == ModelPhase::Active {
        phase = ModelPhase::Cancelled;
    }
    let (initial, initial_retry) = attempt(conn, id, AttemptKind::Initial)?;
    let (retry, _) = attempt(conn, id, AttemptKind::Retry)?;
    Ok(Row {
        generation,
        read_generation,
        digest,
        initial_retry,
        status: ModelStatus {
            id: id.into(),
            phase,
            read_id,
            contract,
            decision,
            initial,
            retry,
            cancelled: cancelled || phase == ModelPhase::Cancelled,
        },
    })
}
fn identity(row: &Row, instance: &str, r: &ModelReceipt, a: &Access) -> Result<()> {
    ensure!(r.instance == instance, "foreign model receipt");
    ensure!(
        row.digest == durable::access_digest(a)?,
        "model access context mismatch"
    );
    Ok(())
}
fn checked(conn: &Connection, instance: &str, r: &ModelReceipt, a: &Access) -> Result<Row> {
    require_v4(conn)?;
    let row = load(conn, &r.id)?;
    identity(&row, instance, r, a)?;
    ensure!(
        row.generation == r.generation && row.status.phase != ModelPhase::Forgotten,
        "model binding forgotten or generation invalidated"
    );
    Ok(row)
}
fn read_receipt(row: &Row, instance: &str) -> durable::Receipt {
    durable::Receipt {
        instance: instance.into(),
        id: row.status.read_id.clone(),
        generation: row.read_generation,
    }
}
fn successful_read(conn: &Connection, row: &durable::Row, a: &Access) -> Result<ModelRead> {
    durable::checked_task(conn, row, a)?;
    durable::check_observation(conn, row, a)?;
    let task = row.status.task.clone().context("missing task")?;
    let observation = row
        .status
        .observation
        .clone()
        .context("missing observation")?;
    let value = durable::content(conn, &observation)?;
    let value = value
        .get("observation")
        .context("missing host read result")?;
    ensure!(
        value.get("status").and_then(Value::as_str) == Some("successful"),
        "model binding requires successful host read"
    );
    let content = value
        .get("content")
        .and_then(Value::as_str)
        .context("missing host read content")?
        .to_owned();
    let sha256 = Sha256::digest(content.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(ModelRead {
        task,
        observation,
        content,
        sha256,
    })
}
fn check_content(conn: &Connection, row: &Row, a: &Access) -> Result<ModelRead> {
    let instance: String =
        conn.query_row("SELECT instance FROM meta WHERE singleton=1", [], |r| {
            r.get(0)
        })?;
    let read = durable::checked_row(conn, &read_receipt(row, &instance), a)?;
    let result = successful_read(conn, &read, a)?;
    let all = [
        row.status.contract.as_ref(),
        row.status.decision.as_ref(),
        row.status.initial.request.as_ref(),
        row.status.initial.result.as_ref(),
        row.status.initial.assessment.as_ref(),
        row.status.retry.request.as_ref(),
        row.status.retry.result.as_ref(),
        row.status.retry.assessment.as_ref(),
    ];
    for r in all.into_iter().flatten() {
        current(conn, r)?;
        ensure!(
            authorized(conn, r, a)?
                && fresh(conn, r, i64::MAX)?
                && required_scopes(conn, &r.revision)? == a.scopes,
            "model content stale or unauthorized"
        );
    }
    Ok(result)
}
fn save(
    conn: &Connection,
    instance: &str,
    id: &str,
    kind: &str,
    payload: Value,
    deps: &[RevisionRef],
    access: &Access,
) -> Result<RevisionRef> {
    durable::linked(
        conn,
        instance,
        id,
        &format!("model-{kind}"),
        payload,
        deps,
        access,
    )
}

impl Memory {
    /// Creates a new v4 file. No existing store is migrated or replaced.
    pub fn create_durable_model(path: &Path, domain: &str) -> Result<Self> {
        let mut db = Self::create(path, domain)?;
        let tx = db
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(durable::SCHEMA)?;
        tx.execute_batch(SCHEMA)?;
        tx.commit()?;
        Ok(db)
    }
    pub(crate) fn durable_bind_model(
        &mut self,
        read_receipt: &durable::Receipt,
        spec: Value,
        access: &Access,
    ) -> Result<ModelReceipt> {
        self.check_access(access)?;
        require_v4(&self.conn)?;
        ensure!(
            read_receipt.instance == self.instance,
            "foreign read receipt"
        );
        bounded(&spec, 24_000)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = durable::checked_row(&tx, read_receipt, access)?;
        ensure!(
            row.status.phase == durable::Phase::Observed && !row.status.reconsideration_spent,
            "read is cancelled, terminal or already proposed"
        );
        let read = successful_read(&tx, &row, access)?;
        let id = uuid::Uuid::new_v4().to_string();
        let contract = save(
            &tx,
            &self.instance,
            &id,
            "contract",
            spec,
            &[read.task, read.observation],
            access,
        )?;
        tx.execute("INSERT INTO durable_model(id,read_id,read_generation,access_digest,phase,contract_object,contract_revision) VALUES (?1,?2,?3,?4,'active',?5,?6)",params![id,read_receipt.id,read_receipt.generation,durable::access_digest(access)?,contract.object,contract.revision])?;
        for k in [AttemptKind::Initial, AttemptKind::Retry] {
            tx.execute(
                "INSERT INTO durable_model_attempt(model_id,kind,phase) VALUES (?1,?2,'unused')",
                params![id, k.key()],
            )?;
        }
        tx.commit()?;
        Ok(ModelReceipt {
            instance: self.instance.clone(),
            id,
            generation: 0,
        })
    }
    pub fn durable_model_reopen(&self, id: &str, access: &Access) -> Result<ModelReceipt> {
        self.check_access(access)?;
        require_v4(&self.conn)?;
        let row = load(&self.conn, id)?;
        let r = ModelReceipt {
            instance: self.instance.clone(),
            id: id.into(),
            generation: row.generation,
        };
        identity(&row, &self.instance, &r, access)?;
        Ok(r)
    }
    /// Content-free metadata remains readable after erasure or source changes.
    pub fn durable_model_status(
        &self,
        receipt: &ModelReceipt,
        access: &Access,
    ) -> Result<ModelStatus> {
        self.check_access(access)?;
        require_v4(&self.conn)?;
        let tx = self.conn.unchecked_transaction()?;
        let row = load(&tx, &receipt.id)?;
        identity(&row, &self.instance, receipt, access)?;
        tx.commit()?;
        Ok(row.status)
    }
    pub fn durable_model_spec(&self, receipt: &ModelReceipt, access: &Access) -> Result<Value> {
        self.check_access(access)?;
        let tx = self.conn.unchecked_transaction()?;
        let row = checked(&tx, &self.instance, receipt, access)?;
        check_content(&tx, &row, access)?;
        let v = durable::content(
            &tx,
            row.status.contract.as_ref().context("missing model spec")?,
        )?;
        tx.commit()?;
        Ok(v)
    }
    pub(crate) fn durable_model_read(
        &self,
        receipt: &ModelReceipt,
        access: &Access,
    ) -> Result<ModelRead> {
        self.check_access(access)?;
        let tx = self.conn.unchecked_transaction()?;
        let row = checked(&tx, &self.instance, receipt, access)?;
        let read = check_content(&tx, &row, access)?;
        tx.commit()?;
        Ok(read)
    }
    pub fn durable_model_attempt(
        &self,
        receipt: &ModelReceipt,
        kind: AttemptKind,
        access: &Access,
    ) -> Result<Option<AttemptData>> {
        self.check_access(access)?;
        let tx = self.conn.unchecked_transaction()?;
        let row = checked(&tx, &self.instance, receipt, access)?;
        check_content(&tx, &row, access)?;
        let s = slot(&row.status, kind);
        let value = s
            .request
            .as_ref()
            .map(|r| {
                Ok::<_, anyhow::Error>(AttemptData {
                    request: durable::content(&tx, r)?,
                    result: s
                        .result
                        .as_ref()
                        .map(|r| durable::content(&tx, r))
                        .transpose()?,
                    assessment: s
                        .assessment
                        .as_ref()
                        .map(|r| durable::content(&tx, r))
                        .transpose()?,
                })
            })
            .transpose()?;
        tx.commit()?;
        Ok(value)
    }
    pub fn durable_model_decision(
        &self,
        receipt: &ModelReceipt,
        access: &Access,
    ) -> Result<Option<Value>> {
        self.check_access(access)?;
        let tx = self.conn.unchecked_transaction()?;
        let row = checked(&tx, &self.instance, receipt, access)?;
        check_content(&tx, &row, access)?;
        let v = row
            .status
            .decision
            .as_ref()
            .map(|r| durable::content(&tx, r))
            .transpose()?;
        tx.commit()?;
        Ok(v)
    }
    pub(crate) fn reserve_model_attempt(
        &mut self,
        receipt: &ModelReceipt,
        kind: AttemptKind,
        request: Value,
        access: &Access,
    ) -> Result<ModelPermit> {
        self.check_access(access)?;
        bounded(&request, 48_000)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = checked(&tx, &self.instance, receipt, access)?;
        check_content(&tx, &row, access)?;
        ensure!(
            row.status.phase == ModelPhase::Active,
            "model binding cancelled or terminal"
        );
        ensure!(
            slot(&row.status, kind).phase == AttemptPhase::Unused,
            "model allowance spent; indeterminate calls cannot be replayed"
        );
        if kind == AttemptKind::Retry {
            ensure!(
                row.status.initial.phase == AttemptPhase::Assessed && row.initial_retry,
                "retry requires initial host finding"
            );
        }
        let mut deps = vec![row.status.contract.clone().context("missing contract")?];
        if kind == AttemptKind::Retry {
            deps.push(
                row.status
                    .initial
                    .assessment
                    .clone()
                    .context("missing initial assessment")?,
            );
        }
        let r = save(
            &tx,
            &self.instance,
            &receipt.id,
            &format!("{}-request", kind.key()),
            request,
            &deps,
            access,
        )?;
        tx.execute("UPDATE durable_model_attempt SET phase='indeterminate',request_object=?1,request_revision=?2 WHERE model_id=?3 AND kind=?4",params![r.object,r.revision,receipt.id,kind.key()])?;
        tx.commit()?;
        Ok(ModelPermit {
            receipt: copy(receipt),
            kind,
        })
    }
    pub(crate) fn model_attempt_guard(
        &self,
        permit: &ModelPermit,
        access: &Access,
    ) -> Result<ModelGuard> {
        self.check_access(access)?;
        let path =
            Path::new(self.conn.path().context("model store has no file path")?).canonicalize()?;
        let guard = ModelGuard {
            path,
            receipt: copy(&permit.receipt),
            kind: permit.kind,
            access: access.clone(),
        };
        guard.check()?;
        Ok(guard)
    }
    pub(crate) fn settle_model_attempt(
        &mut self,
        permit: ModelPermit,
        result: HostModelResult,
        access: &Access,
    ) -> Result<ModelStatus> {
        self.check_access(access)?;
        bounded(&result.payload, 64_000)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = checked(&tx, &self.instance, &permit.receipt, access)?;
        check_content(&tx, &row, access)?;
        ensure!(
            matches!(row.status.phase, ModelPhase::Active | ModelPhase::Cancelled),
            "model binding terminal"
        );
        let s = slot(&row.status, permit.kind);
        ensure!(
            s.phase == AttemptPhase::Indeterminate && s.result.is_none(),
            "attempt already settled or invalidated"
        );
        let deps = [s.request.clone().context("missing request")?];
        let r = save(
            &tx,
            &self.instance,
            &permit.receipt.id,
            &format!("{}-result", permit.kind.key()),
            result.payload,
            &deps,
            access,
        )?;
        tx.execute("UPDATE durable_model_attempt SET phase='settled',result_object=?1,result_revision=?2 WHERE model_id=?3 AND kind=?4",params![r.object,r.revision,permit.receipt.id,permit.kind.key()])?;
        let status = load(&tx, &permit.receipt.id)?.status;
        tx.commit()?;
        Ok(status)
    }
    pub(crate) fn assess_model_attempt(
        &mut self,
        receipt: &ModelReceipt,
        kind: AttemptKind,
        assessment: Value,
        retry_requested: bool,
        access: &Access,
    ) -> Result<ModelStatus> {
        self.check_access(access)?;
        bounded(&assessment, 24_000)?;
        ensure!(
            kind == AttemptKind::Initial || !retry_requested,
            "third call is forbidden"
        );
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = checked(&tx, &self.instance, receipt, access)?;
        check_content(&tx, &row, access)?;
        ensure!(
            row.status.phase == ModelPhase::Active,
            "assessment cancelled or terminal"
        );
        let s = slot(&row.status, kind);
        if s.phase == AttemptPhase::Assessed {
            let (_, saved_retry) = attempt(&tx, &receipt.id, kind)?;
            ensure!(
                saved_retry == retry_requested
                    && durable::content(&tx, s.assessment.as_ref().context("missing assessment")?)?
                        == assessment,
                "assessment is immutable"
            );
            tx.commit()?;
            return Ok(row.status);
        }
        ensure!(
            s.phase == AttemptPhase::Settled,
            "assessment requires settled result"
        );
        let deps = [s.result.clone().context("missing result")?];
        let r = save(
            &tx,
            &self.instance,
            &receipt.id,
            &format!("{}-assessment", kind.key()),
            assessment,
            &deps,
            access,
        )?;
        tx.execute("UPDATE durable_model_attempt SET phase='assessed',assessment_object=?1,assessment_revision=?2,retry_requested=?3 WHERE model_id=?4 AND kind=?5",params![r.object,r.revision,retry_requested,receipt.id,kind.key()])?;
        let status = load(&tx, &receipt.id)?.status;
        tx.commit()?;
        Ok(status)
    }
    pub(crate) fn finalize_model(
        &mut self,
        receipt: &ModelReceipt,
        decision: Value,
        access: &Access,
    ) -> Result<ModelStatus> {
        self.check_access(access)?;
        bounded(&decision, 24_000)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = checked(&tx, &self.instance, receipt, access)?;
        check_content(&tx, &row, access)?;
        ensure!(
            row.status.phase == ModelPhase::Active,
            "finalization cancelled or terminal"
        );
        ensure!(
            row.status.initial.phase == AttemptPhase::Assessed,
            "initial assessment unresolved"
        );
        let last = if row.initial_retry {
            ensure!(
                row.status.retry.phase == AttemptPhase::Assessed,
                "requested retry unresolved"
            );
            &row.status.retry
        } else {
            ensure!(
                row.status.retry.phase == AttemptPhase::Unused,
                "unexpected retry"
            );
            &row.status.initial
        };
        let deps = [
            row.status.contract.clone().context("missing contract")?,
            last.assessment.clone().context("missing assessment")?,
        ];
        let r = save(
            &tx,
            &self.instance,
            &receipt.id,
            "decision",
            decision,
            &deps,
            access,
        )?;
        tx.execute("UPDATE durable_model SET phase='finalized',decision_object=?1,decision_revision=?2 WHERE id=?3",params![r.object,r.revision,receipt.id])?;
        let status = load(&tx, &receipt.id)?.status;
        tx.commit()?;
        Ok(status)
    }
    pub fn durable_model_cancel(
        &mut self,
        receipt: &ModelReceipt,
        access: &Access,
    ) -> Result<ModelStatus> {
        self.check_access(access)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = checked(&tx, &self.instance, receipt, access)?;
        ensure!(
            row.status.phase != ModelPhase::Finalized,
            "finalized model binding is terminal"
        );
        tx.execute(
            "UPDATE durable_model SET phase='cancelled' WHERE id=?1",
            [&receipt.id],
        )?;
        tx.execute(
            "UPDATE durable_task SET phase='cancelled' WHERE id=?1 AND phase!='forgotten'",
            [&row.status.read_id],
        )?;
        let status = load(&tx, &receipt.id)?.status;
        tx.commit()?;
        Ok(status)
    }
}

pub(super) fn extend_forgotten(conn: &Connection) -> Result<usize> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if !matches!(version, 4 | 5) {
        return Ok(0);
    }
    Ok(conn.execute("INSERT OR IGNORE INTO doomed SELECT d.task_object FROM durable_task d JOIN durable_model m ON m.read_id=d.id WHERE m.contract_object IN (SELECT id FROM doomed) OR m.decision_object IN (SELECT id FROM doomed) OR m.id IN (SELECT model_id FROM durable_model_attempt WHERE request_object IN (SELECT id FROM doomed) OR result_object IN (SELECT id FROM doomed) OR assessment_object IN (SELECT id FROM doomed))",[])?)
}
pub(super) fn invalidate_forgotten(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if !matches!(version, 4 | 5) {
        return Ok(());
    }
    conn.execute("INSERT OR IGNORE INTO suppression SELECT digest FROM source_item WHERE object_id IN (SELECT id FROM doomed) AND object_id IN (SELECT contract_object FROM durable_model UNION SELECT decision_object FROM durable_model UNION SELECT request_object FROM durable_model_attempt UNION SELECT result_object FROM durable_model_attempt UNION SELECT assessment_object FROM durable_model_attempt)",[])?;
    conn.execute("UPDATE durable_model_attempt SET request_object=NULL,request_revision=NULL,result_object=NULL,result_revision=NULL,assessment_object=NULL,assessment_revision=NULL WHERE model_id IN (SELECT m.id FROM durable_model m JOIN durable_task d ON d.id=m.read_id WHERE d.task_object IN (SELECT id FROM doomed))",[])?;
    conn.execute("UPDATE durable_model SET phase='forgotten',generation=generation+1,contract_object=NULL,contract_revision=NULL,decision_object=NULL,decision_revision=NULL WHERE read_id IN (SELECT id FROM durable_task WHERE task_object IN (SELECT id FROM doomed))",[])?;
    Ok(())
}

/// Read and model cancellation are one atomic lifecycle operation. The model's
/// finalized decision is terminal through either public cancellation surface.
pub(super) fn cancel_for_read(conn: &Connection, read_id: &str) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if !matches!(version, 4 | 5) {
        return Ok(());
    }
    let finalized: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM durable_model WHERE read_id=?1 AND phase='finalized')",
        [read_id],
        |r| r.get(0),
    )?;
    ensure!(!finalized, "finalized model binding is terminal");
    conn.execute(
        "UPDATE durable_model SET phase='cancelled' WHERE read_id=?1 AND phase='active'",
        [read_id],
    )?;
    Ok(())
}

/// Validate the entire finalized model lifecycle in the caller's transaction.
/// Action bindings retain every sibling, including unused-by-the-final-answer inputs.
pub(super) fn action_dependencies(
    conn: &Connection,
    instance: &str,
    receipt: &ModelReceipt,
    access: &Access,
) -> Result<Vec<RevisionRef>> {
    let row = checked(conn, instance, receipt, access)?;
    check_content(conn, &row, access)?;
    ensure!(
        row.status.phase == ModelPhase::Finalized && !row.status.cancelled,
        "action requires a current finalized model decision"
    );
    Ok([
        row.status.contract,
        row.status.decision,
        row.status.initial.request,
        row.status.initial.result,
        row.status.initial.assessment,
        row.status.retry.request,
        row.status.retry.result,
        row.status.retry.assessment,
    ]
    .into_iter()
    .flatten()
    .collect())
}
