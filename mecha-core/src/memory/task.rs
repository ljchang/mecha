//! Host-only session task state. No tool/native audit adapter or graph authority.
use super::{sql::*, *};
use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRef {
    pub reference: RevisionRef,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Open,
    Active,
    Closed,
    Cancelled,
}
impl TaskStatus {
    fn terminal(self) -> bool {
        matches!(self, Self::Closed | Self::Cancelled)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDraft {
    pub title: String,
    pub intent: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRevision {
    pub account: String,
    pub session_id: String,
    pub title: String,
    pub intent: String,
    pub status: TaskStatus,
}
#[derive(Clone, Debug, Serialize)]
pub struct TaskView {
    pub task: TaskRef,
    pub value: TaskRevision,
}
/// Trusted host construction only; not deserializable and never a model DTO.
pub struct TaskSession {
    instance: String,
    account: String,
    session_id: String,
    access: Access,
}
pub struct TaskOperationId(String);
impl TaskOperationId {
    pub fn new(value: &str) -> Result<Self> {
        identifier(value, 128)?;
        Ok(Self(value.into()))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOperation {
    Create,
    Revise,
    Status,
}
impl TaskOperation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Revise => "revise",
            Self::Status => "status",
        }
    }
    fn parse(value: &str) -> Result<Self> {
        match value {
            "create" => Ok(Self::Create),
            "revise" => Ok(Self::Revise),
            "status" => Ok(Self::Status),
            _ => bail!("invalid task operation"),
        }
    }
}
/// This receipt proves a SQLite commit only, never native append or task success.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskCommit {
    DatabaseOnly,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TaskReceipt {
    pub id: String,
    pub task: TaskRef,
    pub previous: Option<TaskRef>,
    pub operation: TaskOperation,
    pub sequence: i64,
    pub commit: TaskCommit,
}
#[derive(Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum Request<'a> {
    Create {
        draft: &'a TaskDraft,
    },
    Revise {
        expected: &'a TaskRef,
        draft: &'a TaskDraft,
    },
    Status {
        expected: &'a TaskRef,
        status: TaskStatus,
    },
}
impl Request<'_> {
    fn operation(&self) -> TaskOperation {
        match self {
            Self::Create { .. } => TaskOperation::Create,
            Self::Revise { .. } => TaskOperation::Revise,
            Self::Status { .. } => TaskOperation::Status,
        }
    }
    fn expected(&self) -> Option<&TaskRef> {
        match self {
            Self::Create { .. } => None,
            Self::Revise { expected, .. } | Self::Status { expected, .. } => Some(expected),
        }
    }
    fn validate(&self) -> Result<()> {
        match self {
            Self::Create { draft } | Self::Revise { draft, .. } => validate_draft(draft)?,
            Self::Status { .. } => {}
        }
        if let Some(r) = self.expected() {
            validate_ref(&r.reference)?;
        }
        Ok(())
    }
}
fn identifier(s: &str, max: usize) -> Result<()> {
    ensure!(
        !s.is_empty() && s.len() <= max && s.trim() == s && !s.chars().any(char::is_control),
        "invalid task identifier"
    );
    Ok(())
}
fn native_id(id: &str) -> Result<()> {
    // Native IDs allow uppercase timestamp T; browser-key validation does not.
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')),
        "invalid native session id"
    );
    Ok(())
}
fn validate_ref(r: &RevisionRef) -> Result<()> {
    Uuid::parse_str(&r.object).context("invalid task object id")?;
    Uuid::parse_str(&r.revision).context("invalid task revision id")?;
    Ok(())
}
fn validate_draft(draft: &TaskDraft) -> Result<()> {
    ensure!(
        !draft.title.trim().is_empty()
            && draft.title.len() <= 512
            && !draft.intent.trim().is_empty()
            && draft.intent.len() <= 8192,
        "empty or oversized task content"
    );
    Ok(())
}
fn validate_value(value: &TaskRevision) -> Result<()> {
    identifier(&value.account, 256)?;
    native_id(&value.session_id)?;
    validate_draft(&TaskDraft {
        title: value.title.clone(),
        intent: value.intent.clone(),
    })?;
    ensure!(
        serde_json::to_vec(&Record::Task(value.clone()))?.len() <= 16384,
        "oversized task record"
    );
    Ok(())
}
fn hash(value: &impl Serialize) -> Result<String> {
    Ok(Sha256::digest(serde_json::to_vec(value)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn payload_hash(request: &Request<'_>) -> Result<String> {
    hash(&("session-task-request-v1", request))
}
fn is_digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(super) fn version(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}
pub(super) fn require_live(conn: &Connection) -> Result<()> {
    ensure!(
        version(conn)? == 9,
        "session tasks require explicit schema v9 migration"
    );
    check_identity(conn)?;
    let live: String = conn.query_row(
        "SELECT disposition FROM task_store_state WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    ensure!(
        live == "live",
        "historical task copy has no live task authority"
    );
    Ok(())
}
/// Generic Access is insufficient even for an indirect dependency on a Task.
pub(super) fn contains_task(conn: &Connection, r: &RevisionRef) -> Result<bool> {
    Ok(conn.query_row("WITH RECURSIVE inputs(id) AS (SELECT ?1 UNION SELECT d.input FROM dependency d JOIN inputs i ON d.output=i.id) SELECT EXISTS(SELECT 1 FROM inputs JOIN revision r ON r.id=inputs.id JOIN object o ON o.id=r.object_id WHERE o.kind='task' OR json_extract(r.payload,'$.kind')='task')", [&r.revision], |r| r.get(0))?)
}
fn canonical_value(conn: &Connection, reference: &RevisionRef) -> Result<TaskRevision> {
    validate_ref(reference)?;
    let (kind, lifecycle, payload): (String, String, String) = conn.query_row("SELECT o.kind,r.lifecycle,r.payload FROM revision r JOIN object o ON o.id=r.object_id WHERE r.id=?1 AND r.object_id=?2", params![reference.revision,reference.object], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    let Record::Task(value) = serde_json::from_str::<Record>(&payload)? else {
        bail!("task unavailable")
    };
    ensure!(
        kind == "task"
            && lifecycle == "active"
            && payload == serde_json::to_string(&Record::Task(value.clone()))?,
        "invalid canonical task revision"
    );
    validate_value(&value)?;
    Ok(value)
}
fn authorize(
    conn: &Connection,
    session: &TaskSession,
    reference: &RevisionRef,
    guard: Option<&dyn host_capture::ExternalSourceGuard>,
) -> Result<TaskRevision> {
    let value = canonical_value(conn, reference)?;
    ensure!(
        value.account == session.account
            && value.session_id == session.session_id
            && authorized_with_guard(conn, reference, &session.access, guard)?,
        "task unavailable to this session"
    );
    // A historical receipt is not current-head authority; nevertheless today's
    // canonical head must still belong to the exact same account/native session.
    let (head, _, _) = head(conn, &reference.object, i64::MAX)?.context("task unavailable")?;
    let latest = canonical_value(conn, &head)?;
    ensure!(
        latest.account == session.account && latest.session_id == session.session_id,
        "task binding changed"
    );
    Ok(value)
}
fn session_check(conn: &Connection, instance: &str, session: &TaskSession) -> Result<()> {
    require_live(conn)?;
    let (actual, domain): (String, String) = conn.query_row(
        "SELECT instance,domain FROM meta WHERE singleton=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    ensure!(
        actual == instance && actual == session.instance && domain == session.access.domain,
        "wrong task store or security domain"
    );
    Ok(())
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FailurePoint {
    None,
    AfterAppend,
    AfterReceipt,
    Migration,
}

impl Memory {
    /// Explicit keyed v8 -> v9 migration. Opening a file never calls this.
    pub fn enable_session_tasks(&mut self) -> Result<()> {
        self.migrate_tasks(FailurePoint::None)
    }
    fn migrate_tasks(&mut self, failure: FailurePoint) -> Result<()> {
        ensure!(
            self.storage_access().is_encrypted(),
            "session task migration requires keyed storage"
        );
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure!(version(&tx)? == 8, "task migration requires schema v8");
        super::recovery::inventory::integrity(&tx)?;
        // Validate the required v8 base shape as well as its extension lineage.
        for sql in [
            "SELECT singleton,instance,domain,security_epoch FROM meta",
            "SELECT seq,committed_at FROM change_set",
            "SELECT id,kind FROM object",
            "SELECT id,object_id,seq,previous,lifecycle,payload FROM revision",
            "SELECT output,input FROM dependency",
            "SELECT revision,name FROM scope",
            "SELECT digest,object_id FROM source_item",
            "SELECT digest FROM suppression",
            "SELECT object_id FROM restriction",
            "SELECT revision,text FROM memory_fts",
        ] {
            tx.prepare(sql)?;
        }
        ensure!(
            identity(&tx)? == (self.instance.clone(), self.domain.clone()),
            "task migration store identity changed"
        );
        tx.execute_batch(include_str!("task_schema.sql"))?;
        validate_schema(&tx)?;
        ensure!(
            failure != FailurePoint::Migration,
            "injected task migration failure"
        );
        tx.commit()?;
        Ok(())
    }
    /// Construct only from trusted host identity/authentication, never tool JSON.
    /// The returned binding belongs to this instance and a live task-capable DB.
    pub fn task_session(
        &self,
        account: &str,
        native_session_id: &str,
        access: Access,
    ) -> Result<TaskSession> {
        self.check_access(&access)?;
        identifier(account, 256)?;
        native_id(native_session_id)?;
        validate_scopes(&access.scopes)?;
        let tx = self.conn.unchecked_transaction()?;
        let session = TaskSession {
            instance: self.instance.clone(),
            account: account.into(),
            session_id: native_session_id.into(),
            access,
        };
        session_check(&tx, &self.instance, &session)?;
        tx.commit()?;
        Ok(session)
    }
    pub fn task_create(
        &mut self,
        session: &TaskSession,
        operation: &TaskOperationId,
        draft: TaskDraft,
    ) -> Result<TaskReceipt> {
        self.write_task(
            session,
            operation,
            Request::Create { draft: &draft },
            FailurePoint::None,
        )
    }
    pub fn task_revise(
        &mut self,
        session: &TaskSession,
        operation: &TaskOperationId,
        expected: &TaskRef,
        draft: TaskDraft,
    ) -> Result<TaskReceipt> {
        self.write_task(
            session,
            operation,
            Request::Revise {
                expected,
                draft: &draft,
            },
            FailurePoint::None,
        )
    }
    pub fn task_set_status(
        &mut self,
        session: &TaskSession,
        operation: &TaskOperationId,
        expected: &TaskRef,
        status: TaskStatus,
    ) -> Result<TaskReceipt> {
        self.write_task(
            session,
            operation,
            Request::Status { expected, status },
            FailurePoint::None,
        )
    }
    pub fn task_read(&self, session: &TaskSession, task: &TaskRef) -> Result<TaskView> {
        let tx = self.conn.unchecked_transaction()?;
        session_check(&tx, &self.instance, session)?;
        current(&tx, &task.reference)?;
        let value = authorize(&tx, session, &task.reference, self.source_guard.as_deref())?;
        ensure!(fresh(&tx, &task.reference, i64::MAX)?, "stale task inputs");
        tx.commit()?;
        Ok(TaskView {
            task: task.clone(),
            value,
        })
    }
    fn write_task(
        &mut self,
        session: &TaskSession,
        operation_id: &TaskOperationId,
        request: Request<'_>,
        failure: FailurePoint,
    ) -> Result<TaskReceipt> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        session_check(&tx, &self.instance, session)?;
        request.validate()?;
        let key = hash(&(
            "session-task-operation-v1",
            &self.instance,
            &session.account,
            &session.session_id,
            &operation_id.0,
        ))?;
        let payload = payload_hash(&request)?;
        let existing: Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT state,payload_hash FROM task_operation WHERE operation_key=?1",
                [&key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((state, saved)) = existing {
            ensure!(
                state == "active" && saved.as_deref() == Some(&payload),
                "task operation forgotten or conflicting"
            );
            let receipt = receipt(&tx, &key)?;
            authorize(
                &tx,
                session,
                &receipt.task.reference,
                self.source_guard.as_deref(),
            )?;
            tx.commit()?;
            return Ok(receipt);
        }
        let (mut value, scopes) = if let Some(expected) = request.expected() {
            current(&tx, &expected.reference)?;
            let value = authorize(
                &tx,
                session,
                &expected.reference,
                self.source_guard.as_deref(),
            )?;
            ensure!(!value.status.terminal(), "terminal task cannot change");
            ensure!(
                fresh(&tx, &expected.reference, i64::MAX)?,
                "stale task inputs"
            );
            (value, required_scopes(&tx, &expected.reference.revision)?)
        } else {
            (
                TaskRevision {
                    account: session.account.clone(),
                    session_id: session.session_id.clone(),
                    title: String::new(),
                    intent: String::new(),
                    status: TaskStatus::Open,
                },
                session.access.scopes.clone(),
            )
        };
        match &request {
            Request::Create { draft } | Request::Revise { draft, .. } => {
                value.title.clone_from(&draft.title);
                value.intent.clone_from(&draft.intent);
            }
            Request::Status { status, .. } => {
                ensure!(
                    value.status != *status,
                    "fresh status operation must change task status"
                );
                value.status = *status;
            }
        }
        validate_value(&value)?;
        let reference = append(
            &tx,
            request.expected().map(|r| &r.reference),
            &Record::Task(value),
            &scopes,
            &[],
            "active",
        )?;
        ensure!(
            failure != FailurePoint::AfterAppend,
            "injected task failure after append"
        );
        let id = Uuid::new_v4().to_string();
        tx.execute("INSERT INTO task_operation(operation_key,payload_hash,receipt_id,task_object,before_revision,after_revision,operation,state) VALUES(?1,?2,?3,?4,?5,?6,?7,'active')", params![key,payload,id,reference.object,request.expected().map(|r| &r.reference.revision),reference.revision,request.operation().as_str()])?;
        ensure!(
            failure != FailurePoint::AfterReceipt,
            "injected task failure after receipt"
        );
        let result = receipt(&tx, &key)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn task_forget(
        &mut self,
        session: &TaskSession,
        expected: &TaskRef,
    ) -> Result<ForgetReport> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        session_check(&tx, &self.instance, session)?;
        current(&tx, &expected.reference)?;
        authorize(
            &tx,
            session,
            &expected.reference,
            self.source_guard.as_deref(),
        )?;
        ensure!(
            fresh(&tx, &expected.reference, i64::MAX)?,
            "stale task inputs"
        );
        forget::initialize_doomed(&tx)?;
        tx.execute(
            "INSERT INTO doomed VALUES(?1)",
            [&expected.reference.object],
        )?;
        let counts = forget::purge_seeded(&tx)?;
        tx.execute(
            "UPDATE meta SET security_epoch=security_epoch+1 WHERE singleton=1",
            [],
        )?;
        let epoch = tx.query_row(
            "SELECT security_epoch FROM meta WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        tx.commit()?;
        self.conn
            .execute_batch("VACUUM;")
            .context("task erasure committed; post-commit cleanup failed")?;
        Ok(ForgetReport {
            objects: counts.objects,
            revisions: counts.revisions,
            security_epoch: epoch,
            scope: "local prototype database only",
        })
    }
    #[cfg(test)]
    pub(super) fn task_create_failing(
        &mut self,
        session: &TaskSession,
        operation: &TaskOperationId,
        draft: TaskDraft,
        point: FailurePoint,
    ) -> Result<TaskReceipt> {
        self.write_task(session, operation, Request::Create { draft: &draft }, point)
    }
    #[cfg(test)]
    pub(super) fn enable_session_tasks_failing(&mut self) -> Result<()> {
        self.migrate_tasks(FailurePoint::Migration)
    }
}
fn receipt(conn: &Connection, key: &str) -> Result<TaskReceipt> {
    let (id, object, before, after, op): (String, String, Option<String>, String, String) = conn.query_row("SELECT receipt_id,task_object,before_revision,after_revision,operation FROM task_operation WHERE operation_key=?1 AND state='active'", [key], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
    let sequence = conn.query_row(
        "SELECT seq FROM revision WHERE id=?1 AND object_id=?2",
        params![after, object],
        |r| r.get(0),
    )?;
    Ok(TaskReceipt {
        id,
        previous: before.map(|revision| TaskRef {
            reference: RevisionRef {
                object: object.clone(),
                revision,
            },
        }),
        task: TaskRef {
            reference: RevisionRef {
                object,
                revision: after,
            },
        },
        operation: TaskOperation::parse(&op)?,
        sequence,
        commit: TaskCommit::DatabaseOnly,
    })
}

/// Checks complete canonical receipt coverage and exact operation transitions.
/// It deliberately does not treat old task statuses as execution permission.
pub(super) fn validate_schema(conn: &Connection) -> Result<()> {
    conn.prepare("SELECT operation_key,payload_hash,receipt_id,task_object,before_revision,after_revision,operation,state FROM task_operation LIMIT 0")?;
    let states = conn
        .prepare("SELECT singleton,disposition FROM task_store_state")?
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(
        states.len() == 1
            && states[0].0 == 1
            && matches!(states[0].1.as_str(), "live" | "historical"),
        "invalid task store disposition"
    );
    let bad: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM task_operation WHERE operation_key IS NULL OR state NOT IN ('active','forgotten') OR (state='forgotten' AND (payload_hash IS NOT NULL OR receipt_id IS NOT NULL OR task_object IS NOT NULL OR before_revision IS NOT NULL OR after_revision IS NOT NULL OR operation IS NOT NULL))) OR EXISTS(SELECT 1 FROM revision r JOIN object o ON o.id=r.object_id WHERE (o.kind='task' OR json_extract(r.payload,'$.kind')='task') AND (SELECT count(*) FROM task_operation t WHERE t.state='active' AND t.task_object=r.object_id AND t.after_revision=r.id)<>1) OR EXISTS(SELECT after_revision FROM task_operation WHERE state='active' GROUP BY after_revision HAVING count(*)<>1) OR EXISTS(SELECT receipt_id FROM task_operation WHERE state='active' GROUP BY receipt_id HAVING count(*)<>1)", [], |r| r.get(0))?;
    ensure!(!bad, "invalid task operation coverage");
    let rows = conn
        .prepare("SELECT operation_key,payload_hash,state FROM task_operation")?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (key, payload, state) in rows {
        ensure!(is_digest(&key), "invalid task operation key");
        if state == "forgotten" {
            continue;
        }
        let receipt = receipt(conn, &key)?;
        Uuid::parse_str(&receipt.id).context("invalid task receipt id")?;
        let value = canonical_value(conn, &receipt.task.reference)?;
        let stored_previous: Option<String> = conn.query_row(
            "SELECT previous FROM revision WHERE id=?1",
            [&receipt.task.reference.revision],
            |r| r.get(0),
        )?;
        ensure!(
            stored_previous.as_ref() == receipt.previous.as_ref().map(|r| &r.reference.revision),
            "task receipt previous mismatch"
        );
        ensure!(
            dependencies(conn, &receipt.task.reference.revision)?.is_empty(),
            "tasks cannot depend on evidence or outcomes"
        );
        let restricted: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM restriction WHERE object_id=?1)",
            [&receipt.task.reference.object],
            |r| r.get(0),
        )?;
        ensure!(!restricted, "restricted task retained content");
        let draft = TaskDraft {
            title: value.title.clone(),
            intent: value.intent.clone(),
        };
        let request = match (receipt.operation, &receipt.previous) {
            (TaskOperation::Create, None) => {
                ensure!(
                    value.status == TaskStatus::Open,
                    "task creation status mismatch"
                );
                let count: i64 = conn.query_row(
                    "SELECT count(*) FROM revision WHERE object_id=?1 AND previous IS NULL",
                    [&receipt.task.reference.object],
                    |r| r.get(0),
                )?;
                ensure!(count == 1, "task has multiple roots");
                Request::Create { draft: &draft }
            }
            (TaskOperation::Revise | TaskOperation::Status, Some(previous)) => {
                let old = canonical_value(conn, &previous.reference)?;
                let seq: i64 = conn.query_row(
                    "SELECT seq FROM revision WHERE id=?1",
                    [&previous.reference.revision],
                    |r| r.get(0),
                )?;
                ensure!(
                    seq < receipt.sequence
                        && !old.status.terminal()
                        && old.account == value.account
                        && old.session_id == value.session_id,
                    "invalid task revision transition"
                );
                let immediate: String = conn.query_row("SELECT id FROM revision WHERE object_id=?1 AND seq<?2 ORDER BY seq DESC LIMIT 1", params![receipt.task.reference.object,receipt.sequence], |r| r.get(0))?;
                ensure!(
                    immediate == previous.reference.revision,
                    "task revision fork"
                );
                if receipt.operation == TaskOperation::Revise {
                    ensure!(
                        old.status == value.status,
                        "content edit changed task status"
                    );
                    Request::Revise {
                        expected: previous,
                        draft: &draft,
                    }
                } else {
                    ensure!(
                        old.title == value.title
                            && old.intent == value.intent
                            && old.status != value.status,
                        "invalid task status transition or content"
                    );
                    Request::Status {
                        expected: previous,
                        status: value.status,
                    }
                }
            }
            _ => bail!("invalid task operation transition"),
        };
        ensure!(
            payload.as_deref() == Some(payload_hash(&request)?.as_str()),
            "task operation payload mismatch"
        );
    }
    Ok(())
}
pub(super) fn invalidate_forgotten(conn: &Connection) -> Result<()> {
    if version(conn)? != 9 {
        return Ok(());
    }
    conn.execute("UPDATE task_operation SET payload_hash=NULL,receipt_id=NULL,task_object=NULL,before_revision=NULL,after_revision=NULL,operation=NULL,state='forgotten' WHERE task_object IN (SELECT id FROM doomed)", [])?;
    Ok(())
}
