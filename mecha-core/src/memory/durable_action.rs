//! Synthetic action bookkeeping. No public API can fabricate an action outcome;
//! settlement requires the live, one-use capability issued before execution.
use super::{
    durable,
    durable_decision::{self, ModelReceipt},
    sql::*,
    *,
};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const SCHEMA: &str = "
CREATE TABLE durable_action (
 id TEXT PRIMARY KEY, generation INTEGER NOT NULL DEFAULT 0,
 model_id TEXT NOT NULL UNIQUE, model_generation INTEGER NOT NULL,
 access_digest TEXT NOT NULL,
 phase TEXT NOT NULL CHECK(phase IN ('ready','indeterminate','observed','finalized','cancelled','forgotten')),
 spec_object TEXT, spec_revision TEXT,
 observation_object TEXT, observation_revision TEXT,
 feedback_object TEXT, feedback_revision TEXT
);
PRAGMA user_version=5;";

#[derive(Debug)]
pub struct ActionReceipt {
    instance: String,
    id: String,
    generation: i64,
}
impl ActionReceipt {
    pub fn id(&self) -> &str {
        &self.id
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionPhase {
    Ready,
    Indeterminate,
    Observed,
    Finalized,
    Cancelled,
    Forgotten,
}
#[derive(Clone, Debug, Serialize)]
pub struct ActionStatus {
    pub id: String,
    pub model_id: String,
    pub phase: ActionPhase,
    pub cancelled: bool,
    pub spec: Option<RevisionRef>,
    pub observation: Option<RevisionRef>,
    pub feedback: Option<RevisionRef>,
}
/// Never reconstructed by reopening an operational row, and never cloneable.
pub(crate) struct ActionPermit {
    receipt: ActionReceipt,
}
/// A cooperative execution boundary; it keeps no transaction across an effect.
pub struct ActionGuard {
    path: PathBuf,
    receipt: ActionReceipt,
    access: Access,
}
impl ActionGuard {
    pub fn check(&self) -> Result<()> {
        let db = Memory::open_read_only(&self.path)?;
        db.check_access(&self.access)?;
        let tx = db.conn.unchecked_transaction()?;
        let row = checked(&tx, &db.instance, &self.receipt, &self.access)?;
        check_content(&tx, &db.instance, &row, &self.access)?;
        ensure!(
            row.status.phase == ActionPhase::Indeterminate,
            "action cancelled or no longer pending"
        );
        tx.commit()?;
        Ok(())
    }
}
struct Row {
    generation: i64,
    model_generation: i64,
    digest: String,
    status: ActionStatus,
}
fn require_v5(conn: &Connection) -> Result<()> {
    let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(
        v == 5,
        "action lifecycle requires an explicitly created version 5 store"
    );
    Ok(())
}
fn bounded(v: &Value) -> Result<()> {
    ensure!(
        v.is_object() && serde_json::to_vec(v)?.len() <= 24_000,
        "invalid or oversized action lifecycle payload"
    );
    Ok(())
}
fn copy(r: &ActionReceipt) -> ActionReceipt {
    ActionReceipt {
        instance: r.instance.clone(),
        id: r.id.clone(),
        generation: r.generation,
    }
}
fn reference(object: Option<String>, revision: Option<String>) -> Option<RevisionRef> {
    object
        .zip(revision)
        .map(|(object, revision)| RevisionRef { object, revision })
}
fn load(conn: &Connection, id: &str) -> Result<Row> {
    let (generation,model_id,model_generation,digest,phase,spec,observation,feedback) = conn.query_row(
        "SELECT generation,model_id,model_generation,access_digest,phase,spec_object,spec_revision,observation_object,observation_revision,feedback_object,feedback_revision FROM durable_action WHERE id=?1",[id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,reference(r.get(5)?,r.get(6)?),reference(r.get(7)?,r.get(8)?),reference(r.get(9)?,r.get(10)?))))
        .optional()?.context("unknown durable action binding")?;
    let phase = match phase.as_str() {
        "ready" => ActionPhase::Ready,
        "indeterminate" => ActionPhase::Indeterminate,
        "observed" => ActionPhase::Observed,
        "finalized" => ActionPhase::Finalized,
        "cancelled" => ActionPhase::Cancelled,
        "forgotten" => ActionPhase::Forgotten,
        _ => anyhow::bail!("unknown action phase"),
    };
    Ok(Row {
        generation,
        model_generation,
        digest,
        status: ActionStatus {
            id: id.into(),
            model_id,
            phase,
            cancelled: phase == ActionPhase::Cancelled,
            spec,
            observation,
            feedback,
        },
    })
}
fn identity(row: &Row, instance: &str, r: &ActionReceipt, access: &Access) -> Result<()> {
    ensure!(r.instance == instance, "foreign action receipt");
    ensure!(
        row.digest == durable::access_digest(access)?,
        "action access context mismatch"
    );
    Ok(())
}
fn checked(conn: &Connection, instance: &str, r: &ActionReceipt, access: &Access) -> Result<Row> {
    require_v5(conn)?;
    let row = load(conn, &r.id)?;
    identity(&row, instance, r, access)?;
    ensure!(
        row.generation == r.generation && row.status.phase != ActionPhase::Forgotten,
        "action forgotten or generation invalidated"
    );
    Ok(row)
}
fn check_content(conn: &Connection, instance: &str, row: &Row, access: &Access) -> Result<()> {
    durable_decision::action_dependencies(
        conn,
        instance,
        &ModelReceipt {
            instance: instance.into(),
            id: row.status.model_id.clone(),
            generation: row.model_generation,
        },
        access,
    )?;
    for r in [
        row.status.spec.as_ref(),
        row.status.observation.as_ref(),
        row.status.feedback.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        current(conn, r)?;
        ensure!(
            authorized(conn, r, access)?
                && fresh(conn, r, i64::MAX)?
                && required_scopes(conn, &r.revision)? == access.scopes,
            "action content stale or unauthorized"
        );
    }
    ensure!(row.status.spec.is_some(), "missing action specification");
    Ok(())
}

impl Memory {
    /// Creates v5 explicitly. Existing stores are never migrated or replaced.
    pub fn create_durable_action(path: &Path, domain: &str) -> Result<Self> {
        let mut db = Self::create(path, domain)?;
        let tx = db
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(durable::SCHEMA)?;
        tx.execute_batch(durable_decision::SCHEMA)?;
        tx.execute_batch(SCHEMA)?;
        tx.commit()?;
        Ok(db)
    }
    pub(crate) fn bind_action(
        &mut self,
        model: &ModelReceipt,
        spec: Value,
        access: &Access,
    ) -> Result<ActionReceipt> {
        self.check_access(access)?;
        require_v5(&self.conn)?;
        bounded(&spec)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let deps = durable_decision::action_dependencies(&tx, &self.instance, model, access)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM durable_action WHERE model_id=?1)",
            [model.id()],
            |r| r.get(0),
        )?;
        ensure!(!exists, "model decision already bound to an action");
        let id = uuid::Uuid::new_v4().to_string();
        let r = durable::linked(&tx, &self.instance, &id, "action-spec", spec, &deps, access)?;
        tx.execute("INSERT INTO durable_action(id,model_id,model_generation,access_digest,phase,spec_object,spec_revision) VALUES (?1,?2,?3,?4,'ready',?5,?6)",params![id,model.id(),model.generation,durable::access_digest(access)?,r.object,r.revision])?;
        tx.commit()?;
        Ok(ActionReceipt {
            instance: self.instance.clone(),
            id,
            generation: 0,
        })
    }
    pub fn durable_action_reopen(&self, id: &str, access: &Access) -> Result<ActionReceipt> {
        self.check_access(access)?;
        require_v5(&self.conn)?;
        let row = load(&self.conn, id)?;
        let r = ActionReceipt {
            instance: self.instance.clone(),
            id: id.into(),
            generation: row.generation,
        };
        identity(&row, &self.instance, &r, access)?;
        Ok(r)
    }
    /// Content-free metadata is still available after source revocation/erasure.
    pub fn durable_action_status(
        &self,
        receipt: &ActionReceipt,
        access: &Access,
    ) -> Result<ActionStatus> {
        self.check_access(access)?;
        require_v5(&self.conn)?;
        let tx = self.conn.unchecked_transaction()?;
        let row = load(&tx, &receipt.id)?;
        identity(&row, &self.instance, receipt, access)?;
        tx.commit()?;
        Ok(row.status)
    }
    fn action_content(
        &self,
        receipt: &ActionReceipt,
        access: &Access,
        field: fn(&ActionStatus) -> Option<&RevisionRef>,
    ) -> Result<Option<Value>> {
        self.check_access(access)?;
        let tx = self.conn.unchecked_transaction()?;
        let row = checked(&tx, &self.instance, receipt, access)?;
        check_content(&tx, &self.instance, &row, access)?;
        let v = field(&row.status)
            .map(|r| durable::content(&tx, r))
            .transpose()?;
        tx.commit()?;
        Ok(v)
    }
    pub fn durable_action_spec(&self, receipt: &ActionReceipt, access: &Access) -> Result<Value> {
        self.action_content(receipt, access, |s| s.spec.as_ref())?
            .context("missing action specification")
    }
    pub fn durable_action_observation(
        &self,
        receipt: &ActionReceipt,
        access: &Access,
    ) -> Result<Option<Value>> {
        self.action_content(receipt, access, |s| s.observation.as_ref())
    }
    pub fn durable_action_feedback(
        &self,
        receipt: &ActionReceipt,
        access: &Access,
    ) -> Result<Option<Value>> {
        self.action_content(receipt, access, |s| s.feedback.as_ref())
    }
    pub(crate) fn reserve_action(
        &mut self,
        receipt: &ActionReceipt,
        access: &Access,
    ) -> Result<ActionPermit> {
        self.check_access(access)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = checked(&tx, &self.instance, receipt, access)?;
        check_content(&tx, &self.instance, &row, access)?;
        ensure!(
            row.status.phase == ActionPhase::Ready,
            "action allowance spent; indeterminate effects cannot be replayed"
        );
        tx.execute(
            "UPDATE durable_action SET phase='indeterminate' WHERE id=?1",
            [&receipt.id],
        )?;
        tx.commit()?;
        Ok(ActionPermit {
            receipt: copy(receipt),
        })
    }
    pub(crate) fn action_guard(
        &self,
        permit: &ActionPermit,
        access: &Access,
    ) -> Result<ActionGuard> {
        self.check_access(access)?;
        let path =
            Path::new(self.conn.path().context("action store has no file path")?).canonicalize()?;
        let guard = ActionGuard {
            path,
            receipt: copy(&permit.receipt),
            access: access.clone(),
        };
        guard.check()?;
        Ok(guard)
    }
    pub(crate) fn settle_action(
        &mut self,
        permit: ActionPermit,
        outcome: Value,
        access: &Access,
    ) -> Result<ActionStatus> {
        self.check_access(access)?;
        bounded(&outcome)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = checked(&tx, &self.instance, &permit.receipt, access)?;
        check_content(&tx, &self.instance, &row, access)?;
        ensure!(
            matches!(
                row.status.phase,
                ActionPhase::Indeterminate | ActionPhase::Cancelled
            ) && row.status.observation.is_none(),
            "action already settled or invalidated"
        );
        let spec = row
            .status
            .spec
            .as_ref()
            .context("missing action specification")?;
        let r = durable::linked(
            &tx,
            &self.instance,
            &permit.receipt.id,
            "action-observation",
            json!({"kind":"host_action_observation","action_id":permit.receipt.id,"cancelled_action":row.status.cancelled,"observation":outcome}),
            std::slice::from_ref(spec),
            access,
        )?;
        tx.execute("UPDATE durable_action SET phase=?1,observation_object=?2,observation_revision=?3 WHERE id=?4",params![if row.status.cancelled {"cancelled"}else{"observed"},r.object,r.revision,permit.receipt.id])?;
        let status = load(&tx, &permit.receipt.id)?.status;
        tx.commit()?;
        Ok(status)
    }
    pub(crate) fn finalize_action(
        &mut self,
        receipt: &ActionReceipt,
        feedback: Value,
        access: &Access,
    ) -> Result<ActionStatus> {
        self.check_access(access)?;
        bounded(&feedback)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = checked(&tx, &self.instance, receipt, access)?;
        check_content(&tx, &self.instance, &row, access)?;
        ensure!(
            row.status.phase == ActionPhase::Observed,
            "feedback requires an uncancelled observed action"
        );
        let deps = [
            row.status.spec.context("missing action specification")?,
            row.status
                .observation
                .context("missing action observation")?,
        ];
        let r = durable::linked(
            &tx,
            &self.instance,
            &receipt.id,
            "action-feedback",
            feedback,
            &deps,
            access,
        )?;
        tx.execute("UPDATE durable_action SET phase='finalized',feedback_object=?1,feedback_revision=?2 WHERE id=?3",params![r.object,r.revision,receipt.id])?;
        let status = load(&tx, &receipt.id)?.status;
        tx.commit()?;
        Ok(status)
    }
    pub fn durable_action_cancel(
        &mut self,
        receipt: &ActionReceipt,
        access: &Access,
    ) -> Result<ActionStatus> {
        self.check_access(access)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = checked(&tx, &self.instance, receipt, access)?;
        ensure!(
            row.status.phase != ActionPhase::Finalized,
            "finalized action is terminal"
        );
        tx.execute(
            "UPDATE durable_action SET phase='cancelled' WHERE id=?1",
            [&receipt.id],
        )?;
        let status = load(&tx, &receipt.id)?.status;
        tx.commit()?;
        Ok(status)
    }
}

/// Forgetting any action sibling dooms the read root, hence every model/action
/// sibling. The outer fixed-point walk also follows ordinary dependencies.
pub(super) fn extend_forgotten(conn: &Connection) -> Result<usize> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version != 5 {
        return Ok(0);
    }
    Ok(conn.execute("INSERT OR IGNORE INTO doomed SELECT d.task_object FROM durable_task d JOIN durable_model m ON m.read_id=d.id JOIN durable_action a ON a.model_id=m.id WHERE a.spec_object IN (SELECT id FROM doomed) OR a.observation_object IN (SELECT id FROM doomed) OR a.feedback_object IN (SELECT id FROM doomed)",[])?)
}
pub(super) fn invalidate_forgotten(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version != 5 {
        return Ok(());
    }
    conn.execute("INSERT OR IGNORE INTO suppression SELECT digest FROM source_item WHERE object_id IN (SELECT id FROM doomed) AND object_id IN (SELECT spec_object FROM durable_action UNION SELECT observation_object FROM durable_action UNION SELECT feedback_object FROM durable_action)",[])?;
    conn.execute("UPDATE durable_action SET phase='forgotten',generation=generation+1,spec_object=NULL,spec_revision=NULL,observation_object=NULL,observation_revision=NULL,feedback_object=NULL,feedback_revision=NULL WHERE model_id IN (SELECT m.id FROM durable_model m JOIN durable_task d ON d.id=m.read_id WHERE d.task_object IN (SELECT id FROM doomed))",[])?;
    Ok(())
}
