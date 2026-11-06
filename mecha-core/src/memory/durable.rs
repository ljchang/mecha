//! Restart-safe, synthetic-only acquisition bookkeeping. Content is revision-backed;
//! the operational row carries only opaque identities, phases and spent allowances.
use super::{sql::*, store::capture_in, *};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

pub(super) const SCHEMA: &str = "
CREATE TABLE durable_task (
 id TEXT PRIMARY KEY, generation INTEGER NOT NULL DEFAULT 0,
 access_digest TEXT NOT NULL,
 phase TEXT NOT NULL CHECK(phase IN ('ready','indeterminate','observed','reconsidering','committed','cancelled','forgotten')),
 acquisition_spent INTEGER NOT NULL DEFAULT 0 CHECK(acquisition_spent IN (0,1)),
 reconsideration_spent INTEGER NOT NULL DEFAULT 0 CHECK(reconsideration_spent IN (0,1)),
 task_object TEXT, task_revision TEXT,
 observation_object TEXT, observation_revision TEXT,
 attempt_object TEXT, attempt_revision TEXT,
 decision_object TEXT, decision_revision TEXT
);
PRAGMA user_version=3;";

/// An explicit host selection; never inferred from model text. Runtime additionally
/// resolves this relative path through the current ToolCtx jail before dispatch.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskSpec {
    pub workspace: PathBuf,
    pub path: PathBuf,
    pub max_bytes: usize,
    pub max_lines: usize,
}
impl TaskSpec {
    fn validate(&self) -> Result<()> {
        ensure!(self.workspace.is_absolute(), "workspace must be absolute");
        ensure!(self.path.to_str().is_some(), "read path must be UTF-8");
        ensure!(
            !self.path.as_os_str().is_empty()
                && self
                    .path
                    .components()
                    .all(|c| matches!(c, Component::Normal(_))),
            "read path must be a nonempty relative path without traversal"
        );
        ensure!(
            (1024..=24_000).contains(&self.max_bytes) && (1..=256).contains(&self.max_lines),
            "invalid bounded read limits"
        );
        Ok(())
    }
}

/// No Deserialize or public constructor: reopening always checks the store.
#[derive(Debug)]
pub struct Receipt {
    pub(super) instance: String,
    pub(super) id: String,
    pub(super) generation: i64,
}
impl Receipt {
    pub fn id(&self) -> &str {
        &self.id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Ready,
    Indeterminate,
    Observed,
    Reconsidering,
    Committed,
    Cancelled,
    Forgotten,
}
impl Phase {
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "ready" => Self::Ready,
            "indeterminate" => Self::Indeterminate,
            "observed" => Self::Observed,
            "reconsidering" => Self::Reconsidering,
            "committed" => Self::Committed,
            "cancelled" => Self::Cancelled,
            "forgotten" => Self::Forgotten,
            _ => anyhow::bail!("unknown durable phase"),
        })
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub id: String,
    pub phase: Phase,
    pub task: Option<RevisionRef>,
    pub observation: Option<RevisionRef>,
    pub decision: Option<RevisionRef>,
    pub acquisition_spent: bool,
    pub reconsideration_spent: bool,
    pub cancelled: bool,
}

/// Internal host adapter DTO. Model JSON and public Capture cannot construct or
/// settle this value. It deliberately has no Deserialize implementation.
pub(crate) struct HostObservation {
    payload: Value,
}
impl HostObservation {
    pub(crate) fn from_dispatch(payload: Value) -> Self {
        Self { payload }
    }
}
/// A live, one-use capability. Only the reserving process gets it; no reopen API
/// reconstructs it from the spent operational row.
pub(crate) struct DispatchPermit {
    receipt: Receipt,
    spec: TaskSpec,
}
impl DispatchPermit {
    pub(crate) fn spec(&self) -> &TaskSpec {
        &self.spec
    }
}
/// A live permit's last pre-read check. It holds no connection or transaction
/// across hooks/approval; each check reopens the exact bound store read-only.
pub(crate) struct DispatchGuard {
    path: PathBuf,
    receipt: Receipt,
    access: Access,
}
impl DispatchGuard {
    pub(crate) fn check(&self) -> Result<()> {
        let db = Memory::open_read_only(&self.path)?;
        db.check_access(&self.access)?;
        db.check_receipt_instance(&self.receipt)?;
        require_read_schema(&db.conn)?;
        let tx = db.conn.unchecked_transaction()?;
        let row = checked_row(&tx, &self.receipt, &self.access)?;
        ensure!(
            row.status.phase == Phase::Indeterminate
                && row.status.acquisition_spent
                && row.status.observation.is_none(),
            "reserved dispatch cancelled, settled or invalidated"
        );
        checked_task(&tx, &row, &self.access)?;
        tx.commit()?;
        Ok(())
    }
}

/// Model reconsideration is separately spent before inference; never an action permit.
pub struct ReconsiderationPermit {
    receipt: Receipt,
}

#[derive(Serialize, Deserialize)]
struct Sealed {
    reference: RevisionRef,
    scopes: BTreeSet<String>,
}
#[derive(Serialize, Deserialize)]
pub(super) struct TaskData {
    spec: TaskSpec,
    sealed: Vec<Sealed>,
}
pub(super) struct Row {
    generation: i64,
    digest: String,
    pub(super) status: Status,
}

impl Memory {
    /// Explicitly creates a new lifecycle-capable file. Existing v1/v2 stores are
    /// never silently upgraded; create a separate synthetic v3 store instead.
    pub fn create_durable(path: &Path, domain: &str) -> Result<Self> {
        let mut db = Self::create(path, domain)?;
        let tx = db
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(SCHEMA)?;
        tx.commit()?;
        Ok(db)
    }
    pub fn durable_create(
        &mut self,
        mut spec: TaskSpec,
        inputs: &[RevisionRef],
        access: &Access,
    ) -> Result<Receipt> {
        self.check_access(access)?;
        require_read_schema(&self.conn)?;
        spec.validate()?;
        spec.workspace = spec
            .workspace
            .canonicalize()
            .context("resolve bound workspace")?;
        ensure!(spec.workspace.is_dir(), "workspace must be a directory");
        ensure!(inputs.len() <= 128, "too many task inputs");
        validate_scopes(&access.scopes)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut sealed = Vec::new();
        let mut seen = BTreeSet::new();
        for input in inputs {
            current(&tx, input)?;
            ensure!(
                authorized(&tx, input, access)? && fresh(&tx, input, i64::MAX)?,
                "task input unavailable"
            );
            for reference in closure(&tx, input)? {
                current(&tx, &reference)?;
                if seen.insert(reference.revision.clone()) {
                    sealed.push(Sealed {
                        scopes: required_scopes(&tx, &reference.revision)?,
                        reference,
                    });
                }
            }
        }
        ensure!(sealed.len() <= 512, "task input closure too large");
        let id = uuid::Uuid::new_v4().to_string();
        let data = TaskData { spec, sealed };
        let deps = data
            .sealed
            .iter()
            .map(|s| s.reference.clone())
            .collect::<Vec<_>>();
        let task = linked(
            &tx,
            &self.instance,
            &id,
            "task",
            serde_json::to_value(data)?,
            &deps,
            access,
        )?;
        tx.execute("INSERT INTO durable_task(id,access_digest,phase,task_object,task_revision) VALUES (?1,?2,'ready',?3,?4)",params![id,access_digest(access)?,task.object,task.revision])?;
        tx.commit()?;
        Ok(Receipt {
            instance: self.instance.clone(),
            id,
            generation: 0,
        })
    }
    pub fn durable_reopen(&self, id: &str, access: &Access) -> Result<Receipt> {
        self.check_access(access)?;
        require_read_schema(&self.conn)?;
        let row = load(&self.conn, id)?;
        check_context(&row, access)?;
        Ok(Receipt {
            instance: self.instance.clone(),
            id: id.into(),
            generation: row.generation,
        })
    }
    /// Status deliberately remains readable when sources become stale or erased.
    /// It exposes no source/model content and still checks exact host access identity.
    pub fn durable_status(&self, receipt: &Receipt, access: &Access) -> Result<Status> {
        self.check_access(access)?;
        self.check_receipt_instance(receipt)?;
        require_read_schema(&self.conn)?;
        let row = load(&self.conn, &receipt.id)?;
        check_context(&row, access)?;
        Ok(row.status)
    }
    pub fn durable_spec(&self, receipt: &Receipt, access: &Access) -> Result<TaskSpec> {
        self.check_access(access)?;
        self.check_receipt_instance(receipt)?;
        require_read_schema(&self.conn)?;
        let tx = self.conn.unchecked_transaction()?;
        let row = checked_row(&tx, receipt, access)?;
        let data = checked_task(&tx, &row, access)?;
        tx.commit()?;
        Ok(data.spec)
    }
    /// The returned wrapper labels cancellation using the current lifecycle phase,
    /// without rewriting the immutable observation bytes.
    pub fn durable_observation(&self, receipt: &Receipt, access: &Access) -> Result<Option<Value>> {
        self.check_access(access)?;
        self.check_receipt_instance(receipt)?;
        require_read_schema(&self.conn)?;
        let tx = self.conn.unchecked_transaction()?;
        let row = checked_row(&tx, receipt, access)?;
        checked_task(&tx, &row, access)?;
        let value = row
            .status
            .observation
            .as_ref()
            .map(|r| {
                current(&tx, r)?;
                ensure!(
                    authorized(&tx, r, access)? && fresh(&tx, r, i64::MAX)?,
                    "observation unavailable"
                );
                let mut value = content(&tx, r)?;
                value["cancelled_task"] = json!(row.status.cancelled);
                Ok(value)
            })
            .transpose()?;
        tx.commit()?;
        Ok(value)
    }
    pub(crate) fn durable_reserve_dispatch(
        &mut self,
        receipt: &Receipt,
        access: &Access,
    ) -> Result<DispatchPermit> {
        self.check_access(access)?;
        self.check_receipt_instance(receipt)?;
        require_read_schema(&self.conn)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = checked_row(&tx, receipt, access)?;
        let data = checked_task(&tx, &row, access)?;
        ensure!(
            row.status.phase == Phase::Ready && !row.status.acquisition_spent,
            "acquisition unavailable or already spent; indeterminate work cannot be replayed"
        );
        tx.execute(
            "UPDATE durable_task SET phase='indeterminate',acquisition_spent=1 WHERE id=?1",
            [&receipt.id],
        )?;
        tx.commit()?;
        Ok(DispatchPermit {
            receipt: copy_receipt(receipt),
            spec: data.spec,
        })
    }
    /// Gives the adapter a fresh check to run after hooks and human approval,
    /// immediately before the bound read. This cannot recreate an execution permit.
    pub(crate) fn durable_dispatch_guard(
        &self,
        permit: &DispatchPermit,
        access: &Access,
    ) -> Result<DispatchGuard> {
        self.check_access(access)?;
        self.check_receipt_instance(&permit.receipt)?;
        require_read_schema(&self.conn)?;
        let path = Path::new(self.conn.path().context("durable store has no file path")?)
            .canonicalize()
            .context("resolve durable store path")?;
        let guard = DispatchGuard {
            path,
            receipt: copy_receipt(&permit.receipt),
            access: access.clone(),
        };
        guard.check()?;
        Ok(guard)
    }

    pub(crate) fn durable_settle(
        &mut self,
        permit: DispatchPermit,
        observation: HostObservation,
        access: &Access,
    ) -> Result<Status> {
        self.check_access(access)?;
        self.check_receipt_instance(&permit.receipt)?;
        require_read_schema(&self.conn)?;
        ensure!(
            observation.payload.is_object()
                && serde_json::to_vec(&observation.payload)?.len() <= 100_000,
            "invalid host observation"
        );
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = checked_row(&tx, &permit.receipt, access)?;
        checked_task(&tx, &row, access)?;
        ensure!(
            row.status.acquisition_spent
                && row.status.observation.is_none()
                && matches!(row.status.phase, Phase::Indeterminate | Phase::Cancelled),
            "dispatch receipt already settled or invalidated"
        );
        let cancelled = row.status.cancelled
            || observation
                .payload
                .get("cancelled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        let task = row.status.task.as_ref().context("missing task")?;
        let result = linked(
            &tx,
            &self.instance,
            &permit.receipt.id,
            "observation",
            json!({"kind":"host_dispatch_observation","task_id":permit.receipt.id,"cancelled_task":cancelled,"observation":observation.payload}),
            std::slice::from_ref(task),
            access,
        )?;
        tx.execute("UPDATE durable_task SET phase=?1,observation_object=?2,observation_revision=?3 WHERE id=?4",params![if cancelled {"cancelled"}else{"observed"},result.object,result.revision,permit.receipt.id])?;
        let status = load(&tx, &permit.receipt.id)?.status;
        tx.commit()?;
        Ok(status)
    }
    pub fn durable_cancel(&mut self, receipt: &Receipt, access: &Access) -> Result<Status> {
        self.check_access(access)?;
        self.check_receipt_instance(receipt)?;
        require_read_schema(&self.conn)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = checked_row(&tx, receipt, access)?;
        ensure!(
            row.status.phase != Phase::Committed,
            "committed task is terminal"
        );
        super::durable_decision::cancel_for_read(&tx, &receipt.id)?;
        if row.status.phase != Phase::Cancelled {
            // Cancellation must still work after an input becomes stale/restricted;
            // it only narrows authority and stores no source content.
            tx.execute(
                "UPDATE durable_task SET phase='cancelled' WHERE id=?1",
                [&receipt.id],
            )?;
        }
        let status = load(&tx, &receipt.id)?.status;
        tx.commit()?;
        Ok(status)
    }
    pub fn durable_reserve_reconsideration(
        &mut self,
        receipt: &Receipt,
        access: &Access,
    ) -> Result<ReconsiderationPermit> {
        self.check_access(access)?;
        self.check_receipt_instance(receipt)?;
        require_legacy_v3(&self.conn)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = checked_row(&tx, receipt, access)?;
        checked_task(&tx, &row, access)?;
        ensure!(
            row.status.phase == Phase::Observed && !row.status.reconsideration_spent,
            "reconsideration unavailable or already spent"
        );
        check_observation(&tx, &row, access)?;
        tx.execute(
            "UPDATE durable_task SET phase='reconsidering',reconsideration_spent=1 WHERE id=?1",
            [&receipt.id],
        )?;
        tx.commit()?;
        Ok(ReconsiderationPermit {
            receipt: copy_receipt(receipt),
        })
    }
    /// Commits a proposal as untrusted content, not semantic success or action
    /// authority. Checks all original inputs, including those the proposal omits.
    pub fn durable_commit(
        &mut self,
        permit: ReconsiderationPermit,
        attempt: Value,
        access: &Access,
    ) -> Result<Status> {
        self.check_access(access)?;
        self.check_receipt_instance(&permit.receipt)?;
        require_legacy_v3(&self.conn)?;
        ensure!(
            serde_json::to_vec(&attempt)?.len() <= 24_000,
            "oversized reconsideration"
        );
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = checked_row(&tx, &permit.receipt, access)?;
        checked_task(&tx, &row, access)?;
        ensure!(
            row.status.phase == Phase::Reconsidering && row.status.reconsideration_spent,
            "reconsideration cancelled, committed or invalidated"
        );
        check_observation(&tx, &row, access)?;
        let mut deps = vec![
            row.status.task.clone().context("missing task")?,
            row.status
                .observation
                .clone()
                .context("missing observation")?,
        ];
        let attempt = linked(
            &tx,
            &self.instance,
            &permit.receipt.id,
            "attempt",
            json!({"kind":"untrusted_reconsideration","proposal":attempt}),
            &deps,
            access,
        )?;
        deps.push(attempt.clone());
        let decision = linked(
            &tx,
            &self.instance,
            &permit.receipt.id,
            "decision",
            json!({"kind":"host_commit","task_id":permit.receipt.id,"attempt":attempt,"meaning":"proposal retained with original inputs current; no semantic success or action authority"}),
            &deps,
            access,
        )?;
        tx.execute("UPDATE durable_task SET phase='committed',attempt_object=?1,attempt_revision=?2,decision_object=?3,decision_revision=?4 WHERE id=?5",params![attempt.object,attempt.revision,decision.object,decision.revision,permit.receipt.id])?;
        let status = load(&tx, &permit.receipt.id)?.status;
        tx.commit()?;
        Ok(status)
    }
    fn check_receipt_instance(&self, receipt: &Receipt) -> Result<()> {
        ensure!(receipt.instance == self.instance, "foreign durable receipt");
        Ok(())
    }
}
fn copy_receipt(r: &Receipt) -> Receipt {
    Receipt {
        instance: r.instance.clone(),
        id: r.id.clone(),
        generation: r.generation,
    }
}
fn require_legacy_v3(conn: &Connection) -> Result<()> {
    let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(
        v == 3,
        "supplied reconsideration requires version 3; v4 requires model validation"
    );
    Ok(())
}
fn require_read_schema(conn: &Connection) -> Result<()> {
    let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(
        matches!(v, 3..=5),
        "durable lifecycle requires an explicitly created version 3, 4 or 5 store"
    );
    Ok(())
}
pub(super) fn access_digest(a: &Access) -> Result<String> {
    Ok(Sha256::digest(serde_json::to_vec(&(&a.domain, &a.scopes))?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn check_context(row: &Row, a: &Access) -> Result<()> {
    ensure!(
        row.digest == access_digest(a)?,
        "durable task access context mismatch"
    );
    Ok(())
}
pub(super) fn checked_row(conn: &Connection, r: &Receipt, a: &Access) -> Result<Row> {
    let row = load(conn, &r.id)?;
    check_context(&row, a)?;
    ensure!(
        row.generation == r.generation && row.status.phase != Phase::Forgotten,
        "task forgotten or receipt generation invalidated"
    );
    Ok(row)
}
fn reference(object: Option<String>, revision: Option<String>) -> Option<RevisionRef> {
    object
        .zip(revision)
        .map(|(object, revision)| RevisionRef { object, revision })
}
fn load(conn: &Connection, id: &str) -> Result<Row> {
    let (generation,digest,phase,acquisition,reconsideration,task,observation,decision)=conn.query_row("SELECT generation,access_digest,phase,acquisition_spent,reconsideration_spent,task_object,task_revision,observation_object,observation_revision,decision_object,decision_revision FROM durable_task WHERE id=?1",[id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,bool>(3)?,r.get::<_,bool>(4)?,reference(r.get(5)?,r.get(6)?),reference(r.get(7)?,r.get(8)?),reference(r.get(9)?,r.get(10)?)))).optional()?.context("unknown durable task")?;
    let phase = Phase::parse(&phase)?;
    Ok(Row {
        generation,
        digest,
        status: Status {
            id: id.into(),
            phase,
            task,
            observation,
            decision,
            acquisition_spent: acquisition,
            reconsideration_spent: reconsideration,
            cancelled: phase == Phase::Cancelled,
        },
    })
}
pub(super) fn content(conn: &Connection, r: &RevisionRef) -> Result<Value> {
    let Some(Record::Evidence(e)) = lookup(conn, r)? else {
        anyhow::bail!("missing durable content")
    };
    Ok(serde_json::from_str(&e.text)?)
}
pub(super) fn checked_task(conn: &Connection, row: &Row, access: &Access) -> Result<TaskData> {
    let task = row.status.task.as_ref().context("erased task")?;
    current(conn, task)?;
    ensure!(
        authorized(conn, task, access)? && fresh(conn, task, i64::MAX)?,
        "task sources stale or unauthorized"
    );
    let data: TaskData = serde_json::from_value(content(conn, task)?)?;
    for source in &data.sealed {
        // current() explicitly detects missing revisions; closure-based helpers
        // alone are vacuously true after erasure and cannot authorize a write.
        current(conn, &source.reference)?;
        ensure!(
            authorized(conn, &source.reference, access)?
                && fresh(conn, &source.reference, i64::MAX)?
                && required_scopes(conn, &source.reference.revision)? == source.scopes,
            "original input changed or became unavailable"
        );
    }
    Ok(data)
}
pub(super) fn check_observation(conn: &Connection, row: &Row, access: &Access) -> Result<()> {
    let r = row
        .status
        .observation
        .as_ref()
        .context("missing observation")?;
    current(conn, r)?;
    ensure!(
        authorized(conn, r, access)? && fresh(conn, r, i64::MAX)?,
        "observation unavailable"
    );
    Ok(())
}
pub(super) fn linked(
    conn: &Connection,
    instance: &str,
    id: &str,
    kind: &str,
    payload: Value,
    deps: &[RevisionRef],
    access: &Access,
) -> Result<RevisionRef> {
    let reference = capture_in(
        conn,
        instance,
        Capture {
            source: SourceKey {
                connector: "durable-read".into(),
                account: kind.into(),
                id: id.into(),
            },
            text: serde_json::to_string(&payload)?,
            occurred_at: None,
            scopes: access.scopes.clone(),
            transcript: None,
        },
    )?;
    for input in deps {
        conn.execute(
            "INSERT INTO dependency(output,input) VALUES (?1,?2)",
            params![reference.revision, input.revision],
        )?;
    }
    Ok(reference)
}

/// Called inside forget's write transaction before deleting any revision. If any
/// lifecycle content is doomed, doom its root and all sibling records as well.
pub(super) fn extend_forgotten(conn: &Connection) -> Result<usize> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if !matches!(version, 3..=5) {
        return Ok(0);
    }
    Ok(conn.execute("INSERT OR IGNORE INTO doomed SELECT task_object FROM durable_task WHERE task_object IN (SELECT id FROM doomed) OR observation_object IN (SELECT id FROM doomed) OR attempt_object IN (SELECT id FROM doomed) OR decision_object IN (SELECT id FROM doomed)",[])?)
}
pub(super) fn invalidate_forgotten(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if matches!(version, 3..=5) {
        // Suppress only lifecycle-owned Capture keys. A caller holding old content
        // cannot re-ingest an erased task/result under its former source identity;
        // unrelated generic capture/reimport semantics stay unchanged.
        conn.execute("INSERT OR IGNORE INTO suppression SELECT digest FROM source_item WHERE object_id IN (SELECT id FROM doomed) AND object_id IN (SELECT task_object FROM durable_task UNION SELECT observation_object FROM durable_task UNION SELECT attempt_object FROM durable_task UNION SELECT decision_object FROM durable_task)", [])?;
        conn.execute("UPDATE durable_task SET phase='forgotten',generation=generation+1,task_object=NULL,task_revision=NULL,observation_object=NULL,observation_revision=NULL,attempt_object=NULL,attempt_revision=NULL,decision_object=NULL,decision_revision=NULL WHERE task_object IN (SELECT id FROM doomed)",[])?;
    }
    Ok(())
}
