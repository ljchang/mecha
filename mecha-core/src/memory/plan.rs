//! Internal task-chat storage primitives. Runtime feedback is opaque, never trusted on load.
// These host doors are consumed by the task-chat adapter; the standalone harness has no host.
#![cfg_attr(not(feature = "memory-prototype"), allow(dead_code))]
use super::{sql::*, *};
use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanItemStatus {
    Pending,
    InProgress,
    Completed,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanItem {
    pub content: String,
    pub status: PlanItemStatus,
    pub expect: Option<String>,
    pub check: Option<String>,
    pub expect_calls: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanCheckDeclaration {
    pub check: String,
    pub frozen: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSnapshot {
    pub items: Vec<PlanItem>,
    pub serves: Option<String>,
    #[serde(deserialize_with = "unique_declarations")]
    pub declarations: BTreeMap<String, PlanCheckDeclaration>,
}
fn unique_declarations<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<BTreeMap<String, PlanCheckDeclaration>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = BTreeMap<String, PlanCheckDeclaration>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("unique plan check declarations")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut declarations = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, PlanCheckDeclaration>()? {
                if declarations.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate plan declaration"));
                }
                if declarations.len() > 256 {
                    return Err(serde::de::Error::custom("too many plan declarations"));
                }
            }
            Ok(declarations)
        }
    }
    deserializer.deserialize_map(Visitor)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanOccurrence {
    pub turn_id: String,
    pub ordinal: u32,
}
/// Exact bytes only. Deserialization conveys no runtime planner validation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredPlanFeedback {
    text: String,
    digest: String,
}
impl StoredPlanFeedback {
    /// Caller must separately validate the concrete runtime Feedback round-trip/profile.
    pub(crate) fn from_host_text(text: String) -> Result<Self> {
        let value = Self {
            digest: digest(text.as_bytes()),
            text,
        };
        value.validate()?;
        Ok(value)
    }
    pub(crate) fn text(&self) -> &str {
        &self.text
    }
    pub(crate) fn validate(&self) -> Result<()> {
        json_object(&self.text, 65536)?;
        ensure!(
            self.digest == digest(self.text.as_bytes()),
            "plan feedback digest mismatch"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanCommitRequest {
    pub occurrence: PlanOccurrence,
    pub snapshot: PlanSnapshot,
    pub goal_decision_digest: String,
    pub goal_version_digest: String,
    pub goal_version: u64,
    pub provider_call_id: String,
    pub input_json: String,
    pub result_text: String,
    pub feedback: StoredPlanFeedback,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRevision {
    pub task: TaskRef,
    pub request: PlanCommitRequest,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlanView {
    pub reference: RevisionRef,
    pub value: PlanRevision,
    pub receipt_id: String,
    pub audited: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlanCommitResult {
    pub view: PlanView,
    pub replayed: bool,
}

/// Non-serde, unique to one live host turn. The host checks its held attachment
/// lease before invoking storage. Dropping/revoking waits for an in-flight commit.
pub(crate) struct PlanTurnPermit {
    instance: String,
    account: String,
    session_id: String,
    access: Access,
    task: TaskRef,
    attachment_digest: String,
    turn_id: String,
    permit_digest: String,
    live: Mutex<bool>,
}
impl PlanTurnPermit {
    /// Trusted authenticated task-chat host only; never construct from tool/native JSON.
    pub(crate) fn from_host(
        session: &TaskSession,
        task: &TaskRef,
        attachment_digest: &str,
        turn_id: &str,
    ) -> Result<Self> {
        valid_digest(attachment_digest)?;
        identifier(turn_id, 128)?;
        task::validate_ref(&task.reference)?;
        Ok(Self {
            instance: session.instance.clone(),
            account: session.account.clone(),
            session_id: session.session_id.clone(),
            access: session.access.clone(),
            task: task.clone(),
            attachment_digest: attachment_digest.into(),
            turn_id: turn_id.into(),
            permit_digest: digest(Uuid::new_v4().as_bytes()),
            live: Mutex::new(true),
        })
    }
    pub(crate) fn revoke(&self) -> Result<()> {
        *self
            .live
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned plan turn"))? = false;
        Ok(())
    }
    fn matches(&self, session: &TaskSession, task: &TaskRef) -> bool {
        self.instance == session.instance
            && self.account == session.account
            && self.session_id == session.session_id
            && self.access.domain == session.access.domain
            && self.access.scopes == session.access.scopes
            && self.task == *task
    }
}
impl Drop for PlanTurnPermit {
    fn drop(&mut self) {
        if let Ok(live) = self.live.get_mut() {
            *live = false;
        }
    }
}
/// Only the host's authenticated synced-native verifier constructs this value.
/// It has audit authority, never execution authority. Entries preserve each exact
/// occurrence's original revision, not a substituted current Plan head.
pub(crate) struct VerifiedPlanAudit {
    instance: String,
    account: String,
    session_id: String,
    task: TaskRef,
    attachment_digest: String,
    turn_id: String,
    entries: Vec<PlanView>,
}
impl VerifiedPlanAudit {
    pub(crate) fn from_host_verified(
        session: &TaskSession,
        task: &TaskRef,
        attachment_digest: &str,
        turn_id: &str,
        entries: Vec<PlanView>,
    ) -> Result<Self> {
        valid_digest(attachment_digest)?;
        identifier(turn_id, 128)?;
        ensure!(entries.len() <= 256, "too many plan audit entries");
        Ok(Self {
            instance: session.instance.clone(),
            account: session.account.clone(),
            session_id: session.session_id.clone(),
            task: task.clone(),
            attachment_digest: attachment_digest.into(),
            turn_id: turn_id.into(),
            entries,
        })
    }
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn hash(value: &impl Serialize) -> Result<String> {
    Ok(digest(&serde_json::to_vec(value)?))
}
fn valid_digest(value: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid plan digest"
    );
    Ok(())
}
fn identifier(value: &str, max: usize) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= max
            && value.trim() == value
            && !value.chars().any(char::is_control),
        "invalid plan identifier"
    );
    Ok(())
}
fn json_object(text: &str, max: usize) -> Result<()> {
    ensure!(text.len() <= max, "oversized plan JSON");
    let parsed: super::plan_extraction::UniqueValue = serde_json::from_str(text)?;
    ensure!(parsed.0.is_object(), "plan JSON must be an object");
    Ok(())
}
fn field(value: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.len() <= 512,
        "empty or oversized plan field"
    );
    Ok(())
}
impl PlanSnapshot {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.items.len() <= 32
                && self.declarations.len() <= 256
                && serde_json::to_vec(self)?.len() <= 32768,
            "plan snapshot exceeds bounds"
        );
        let mut seen = BTreeSet::new();
        if let Some(serves) = &self.serves {
            identifier(serves, 512)?;
        }
        for item in &self.items {
            field(&item.content)?;
            ensure!(
                item.content.trim() == item.content && seen.insert(&item.content),
                "duplicate or noncanonical plan content"
            );
            for text in [&item.expect, &item.check].into_iter().flatten() {
                field(text)?;
            }
            if let Some(declaration) = self.declarations.get(&item.content) {
                ensure!(
                    Some(&declaration.check) == item.check.as_ref(),
                    "plan declaration mismatch"
                );
            } else {
                ensure!(item.check.is_none(), "missing plan declaration");
            }
            if item.status == PlanItemStatus::Completed && item.check.is_some() {
                ensure!(
                    self.declarations
                        .get(&item.content)
                        .is_some_and(|d| d.frozen),
                    "completed step must freeze its declaration"
                );
            }
        }
        for (content, declaration) in &self.declarations {
            field(content)?;
            ensure!(content.trim() == content, "noncanonical declaration key");
            field(&declaration.check)?;
        }
        Ok(())
    }
}
impl PlanCommitRequest {
    fn validate(&self) -> Result<()> {
        identifier(&self.occurrence.turn_id, 128)?;
        ensure!(
            self.occurrence.ordinal < 256,
            "plan occurrence exceeds bounds"
        );
        self.snapshot.validate()?;
        valid_digest(&self.goal_decision_digest)?;
        valid_digest(&self.goal_version_digest)?;
        identifier(&self.provider_call_id, 256)?;
        json_object(&self.input_json, 65536)?;
        ensure!(self.result_text.len() <= 65536, "oversized plan result");
        self.feedback.validate()
    }
}
fn require(
    conn: &Connection,
    instance: &str,
    session: &TaskSession,
    task: &TaskRef,
    writing: bool,
    guard: Option<&dyn host_capture::ExternalSourceGuard>,
) -> Result<()> {
    ensure!(
        task::version(conn)? == 10,
        "plans require explicit schema v10 migration"
    );
    task::session_check(conn, instance, session)?;
    current(conn, &task.reference)?;
    let value = task::authorize(conn, session, &task.reference, guard)?;
    ensure!(fresh(conn, &task.reference, i64::MAX)?, "stale task inputs");
    ensure!(
        !writing || !value.status.terminal(),
        "terminal task cannot change plan"
    );
    Ok(())
}
fn key(
    instance: &str,
    session: &TaskSession,
    task: &TaskRef,
    attachment: &str,
    occurrence: &PlanOccurrence,
) -> Result<String> {
    hash(&(
        "session-plan-operation-v1",
        instance,
        &session.account,
        &session.session_id,
        task,
        attachment,
        occurrence,
    ))
}
fn payload(
    task: &TaskRef,
    expected: Option<&RevisionRef>,
    request: &PlanCommitRequest,
) -> Result<String> {
    hash(&("session-plan-request-v1", task, expected, request))
}
fn canonical(conn: &Connection, reference: &RevisionRef) -> Result<PlanRevision> {
    task::validate_ref(reference)?;
    let (kind,state,text):(String,String,String)=conn.query_row("SELECT o.kind,r.lifecycle,r.payload FROM revision r JOIN object o ON o.id=r.object_id WHERE r.id=?1 AND r.object_id=?2",params![reference.revision,reference.object],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    // Reject duplicate map keys before typed decoding, including declarations.
    let _: super::plan_extraction::UniqueValue = serde_json::from_str(&text)?;
    let Record::Plan(value) = serde_json::from_str::<Record>(&text)? else {
        bail!("invalid plan payload")
    };
    ensure!(
        kind == "plan"
            && state == "active"
            && text == serde_json::to_string(&Record::Plan(value.clone()))?,
        "invalid canonical plan revision"
    );
    task::validate_ref(&value.task.reference)?;
    value.request.validate()?;
    Ok(value)
}
fn view(conn: &Connection, operation: &str) -> Result<PlanView> {
    let (object,revision,receipt_id,state):(String,String,String,String)=conn.query_row("SELECT plan_object,after_revision,receipt_id,state FROM plan_operation WHERE operation_key=?1 AND state IN ('pending','audited')",[operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    let reference = RevisionRef { object, revision };
    Ok(PlanView {
        value: canonical(conn, &reference)?,
        reference,
        receipt_id,
        audited: state == "audited",
    })
}
fn plan_head(conn: &Connection, task: &TaskRef) -> Result<Option<RevisionRef>> {
    let objects=conn.prepare("SELECT DISTINCT plan_object FROM plan_operation WHERE task_object=?1 AND state<>'forgotten'")?.query_map([&task.reference.object],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(objects.len() <= 1, "multiple plans for task");
    objects
        .first()
        .map(|o| {
            head(conn, o, i64::MAX)?
                .map(|h| h.0)
                .context("missing plan head")
        })
        .transpose()
}
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))] // Fault variants are constructed only by storage qualification.
pub(super) enum FailurePoint {
    None,
    Migration,
    AfterAppend,
    BeforeReceipt,
    BeforeCommit,
}
impl Memory {
    /// Validated schema metadata for explicit host migration orchestration.
    /// Reading a version neither migrates nor grants live task/plan authority.
    pub fn session_schema_version(&self) -> Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        ensure!(
            identity(&tx)? == (self.instance.clone(), self.domain.clone()),
            "store identity changed"
        );
        let version = task::version(&tx)?;
        tx.commit()?;
        Ok(version)
    }
    pub fn enable_session_plans(&mut self) -> Result<()> {
        self.migrate_plans(FailurePoint::None)
    }
    pub(super) fn migrate_plans(&mut self, failure: FailurePoint) -> Result<()> {
        ensure!(
            self.storage_access().is_encrypted(),
            "plan migration requires keyed storage"
        );
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure!(
            task::version(&tx)? == 9,
            "plan migration requires schema v9"
        );
        task::require_live(&tx)?;
        super::recovery::inventory::integrity(&tx)?;
        ensure!(
            identity(&tx)? == (self.instance.clone(), self.domain.clone()),
            "plan migration identity changed"
        );
        tx.execute_batch(include_str!("plan_schema.sql"))?;
        validate_schema(&tx)?;
        ensure!(
            failure != FailurePoint::Migration,
            "injected plan migration failure"
        );
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn plan_read(
        &self,
        session: &TaskSession,
        task: &TaskRef,
    ) -> Result<Option<PlanView>> {
        let tx = self.conn.unchecked_transaction()?;
        require(
            &tx,
            &self.instance,
            session,
            task,
            false,
            self.source_guard.as_deref(),
        )?;
        let result = if let Some(reference) = plan_head(&tx, task)? {
            let operation: String = tx.query_row(
                "SELECT operation_key FROM plan_operation WHERE after_revision=?1",
                [&reference.revision],
                |r| r.get(0),
            )?;
            let result = view(&tx, &operation)?;
            ensure!(
                result.value.task == *task
                    && authorized_with_guard(
                        &tx,
                        &reference,
                        &session.access,
                        self.source_guard.as_deref()
                    )?
                    && fresh(&tx, &reference, i64::MAX)?,
                "plan unavailable"
            );
            Some(result)
        } else {
            None
        };
        tx.commit()?;
        Ok(result)
    }
    pub(crate) fn plan_commit(
        &mut self,
        session: &TaskSession,
        permit: &PlanTurnPermit,
        task: &TaskRef,
        expected: Option<&RevisionRef>,
        request: PlanCommitRequest,
    ) -> Result<PlanCommitResult> {
        self.write_plan(session, permit, task, expected, request, FailurePoint::None)
    }
    pub(super) fn write_plan(
        &mut self,
        session: &TaskSession,
        permit: &PlanTurnPermit,
        task: &TaskRef,
        expected: Option<&RevisionRef>,
        request: PlanCommitRequest,
        failure: FailurePoint,
    ) -> Result<PlanCommitResult> {
        let live = permit
            .live
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned plan turn"))?;
        ensure!(
            *live && permit.matches(session, task) && permit.turn_id == request.occurrence.turn_id,
            "invalid live plan permit"
        );
        request.validate()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require(
            &tx,
            &self.instance,
            session,
            task,
            true,
            self.source_guard.as_deref(),
        )?;
        let operation = key(
            &self.instance,
            session,
            task,
            &permit.attachment_digest,
            &request.occurrence,
        )?;
        let payload = payload(task, expected, &request)?;
        let existing: Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT state,payload_hash FROM plan_operation WHERE operation_key=?1",
                [&operation],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((state, saved)) = existing {
            ensure!(
                state != "forgotten" && saved.as_deref() == Some(&payload),
                "conflicting or forgotten plan retry"
            );
            let stored_permit: String = tx.query_row(
                "SELECT permit_digest FROM plan_operation WHERE operation_key=?1",
                [&operation],
                |r| r.get(0),
            )?;
            ensure!(
                state == "audited" || stored_permit == permit.permit_digest,
                "pending retry requires original live permit"
            );
            let result = view(&tx, &operation)?;
            ensure!(
                authorized_with_guard(
                    &tx,
                    &result.reference,
                    &session.access,
                    self.source_guard.as_deref()
                )?,
                "plan retry unauthorized"
            );
            tx.commit()?;
            return Ok(PlanCommitResult {
                view: result,
                replayed: true,
            });
        }
        let foreign:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM plan_operation WHERE task_object=?1 AND state='pending' AND (permit_digest<>?2 OR turn_id<>?3 OR attachment_digest<>?4))",params![task.reference.object,permit.permit_digest,permit.turn_id,permit.attachment_digest],|r|r.get(0))?;
        ensure!(
            !foreign,
            "plan has pending native audit from another live turn"
        );
        let last_ordinal: Option<u32> = tx.query_row(
            "SELECT max(ordinal) FROM plan_operation WHERE task_object=?1 AND attachment_digest=?2 AND turn_id=?3 AND state<>'forgotten'",
            params![task.reference.object, permit.attachment_digest, permit.turn_id],
            |r| r.get(0),
        )?;
        ensure!(
            last_ordinal.is_none_or(|last| last < request.occurrence.ordinal),
            "plan occurrence is out of authored order"
        );
        let previous = plan_head(&tx, task)?;
        ensure!(previous.as_ref() == expected, "stale plan head");
        if let Some(previous) = &previous {
            let old = canonical(&tx, previous)?;
            ensure!(
                old.task == *task
                    && authorized_with_guard(
                        &tx,
                        previous,
                        &session.access,
                        self.source_guard.as_deref()
                    )?,
                "plan unavailable"
            );
            validate_transition(Some(&old.request.snapshot), &request.snapshot)?;
        } else {
            validate_transition(None, &request.snapshot)?;
        }
        let reference = append(
            &tx,
            previous.as_ref(),
            &Record::Plan(PlanRevision {
                task: task.clone(),
                request: request.clone(),
            }),
            &required_scopes(&tx, &task.reference.revision)?,
            std::slice::from_ref(&task.reference),
            "active",
        )?;
        ensure!(
            !matches!(
                failure,
                FailurePoint::AfterAppend | FailurePoint::BeforeReceipt
            ),
            "injected plan failure before receipt"
        );
        tx.execute("INSERT INTO plan_operation(operation_key,payload_hash,receipt_id,task_object,plan_object,before_revision,after_revision,attachment_digest,turn_id,permit_digest,ordinal,state) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'pending')",params![operation,payload,Uuid::new_v4().to_string(),task.reference.object,reference.object,previous.as_ref().map(|r|&r.revision),reference.revision,permit.attachment_digest,permit.turn_id,permit.permit_digest,request.occurrence.ordinal])?;
        ensure!(
            failure != FailurePoint::BeforeCommit,
            "injected plan failure before commit"
        );
        let result = view(&tx, &operation)?;
        tx.commit()?;
        Ok(PlanCommitResult {
            view: result,
            replayed: false,
        })
    }
    pub(crate) fn plan_pending(
        &self,
        session: &TaskSession,
        task: &TaskRef,
    ) -> Result<Vec<PlanView>> {
        let tx = self.conn.unchecked_transaction()?;
        require(
            &tx,
            &self.instance,
            session,
            task,
            false,
            self.source_guard.as_deref(),
        )?;
        let keys=tx.prepare("SELECT operation_key FROM plan_operation WHERE task_object=?1 AND state='pending' ORDER BY ordinal LIMIT 257")?.query_map([&task.reference.object],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(keys.len() <= 256, "too many pending plan operations");
        let result = keys
            .iter()
            .map(|k| view(&tx, k))
            .collect::<Result<Vec<_>>>()?;
        tx.commit()?;
        Ok(result)
    }
    /// Authorized original receipts, including audited siblings; never execution authority.
    pub(crate) fn plan_turn_entries(
        &self,
        session: &TaskSession,
        task: &TaskRef,
        attachment: &str,
        turn: &str,
    ) -> Result<Vec<PlanView>> {
        valid_digest(attachment)?;
        ensure!(
            !turn.is_empty() && turn.len() <= 128 && !turn.chars().any(char::is_control),
            "invalid plan turn"
        );
        let tx = self.conn.unchecked_transaction()?;
        require(
            &tx,
            &self.instance,
            session,
            task,
            false,
            self.source_guard.as_deref(),
        )?;
        let foreign: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM plan_operation WHERE task_object=?1 AND turn_id=?2 AND state IN ('pending','audited') AND attachment_digest<>?3)", rusqlite::params![task.reference.object, turn, attachment], |r| r.get(0))?;
        ensure!(!foreign, "plan turn attachment mismatch");
        let keys = tx.prepare("SELECT operation_key FROM plan_operation WHERE task_object=?1 AND attachment_digest=?2 AND turn_id=?3 AND state IN ('pending','audited') ORDER BY ordinal LIMIT 257")?.query_map(rusqlite::params![task.reference.object, attachment, turn], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(keys.len() <= 256, "too many plan turn entries");
        let entries = keys
            .iter()
            .map(|k| view(&tx, k))
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            entries
                .iter()
                .all(|e| e.value.task == *task && e.value.request.occurrence.turn_id == turn),
            "plan turn binding mismatch"
        );
        tx.commit()?;
        Ok(entries)
    }
    pub(crate) fn plan_reconcile_audit(
        &mut self,
        session: &TaskSession,
        task: &TaskRef,
        proof: &VerifiedPlanAudit,
    ) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require(
            &tx,
            &self.instance,
            session,
            task,
            false,
            self.source_guard.as_deref(),
        )?;
        ensure!(
            proof.instance == self.instance
                && proof.account == session.account
                && proof.session_id == session.session_id
                && proof.task == *task,
            "foreign plan audit proof"
        );
        let mut seen = BTreeSet::new();
        for entry in &proof.entries {
            ensure!(
                entry.value.task == *task
                    && entry.value.request.occurrence.turn_id == proof.turn_id
                    && seen.insert(entry.value.request.occurrence.ordinal),
                "invalid plan audit occurrence"
            );
            let operation = key(
                &self.instance,
                session,
                task,
                &proof.attachment_digest,
                &entry.value.request.occurrence,
            )?;
            let stored = view(&tx, &operation)?;
            ensure!(
                stored.reference == entry.reference
                    && stored.receipt_id == entry.receipt_id
                    && stored.value == entry.value,
                "plan audit evidence mismatch"
            );
            tx.execute(
                "UPDATE plan_operation SET state='audited' WHERE operation_key=?1",
                [operation],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
/// Validate every receipt against canonical immutable revision payloads. SQL
/// constraints alone do not detect externally corrupted or rewritten ledgers.
pub(super) fn validate_schema(conn: &Connection) -> Result<()> {
    conn.prepare("SELECT operation_key,payload_hash,receipt_id,task_object,plan_object,before_revision,after_revision,attachment_digest,turn_id,permit_digest,ordinal,state FROM plan_operation LIMIT 0")?;
    let bad:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM plan_operation WHERE state NOT IN ('pending','audited','forgotten') OR operation_key IS NULL OR (state='forgotten' AND (payload_hash IS NOT NULL OR receipt_id IS NOT NULL OR task_object IS NOT NULL OR plan_object IS NOT NULL OR before_revision IS NOT NULL OR after_revision IS NOT NULL OR attachment_digest IS NOT NULL OR turn_id IS NOT NULL OR permit_digest IS NOT NULL OR ordinal IS NOT NULL))) OR EXISTS(SELECT 1 FROM revision r JOIN object o ON o.id=r.object_id WHERE (o.kind='plan' OR json_extract(r.payload,'$.kind')='plan') AND (SELECT count(*) FROM plan_operation p WHERE p.state IN ('pending','audited') AND p.plan_object=r.object_id AND p.after_revision=r.id)<>1) OR EXISTS(SELECT task_object FROM plan_operation WHERE state<>'forgotten' GROUP BY task_object HAVING count(DISTINCT plan_object)<>1) OR EXISTS(SELECT receipt_id FROM plan_operation WHERE state<>'forgotten' GROUP BY receipt_id HAVING count(*)<>1)",[],|r|r.get(0))?;
    ensure!(!bad, "invalid plan operation coverage");
    let rows = conn
        .prepare("SELECT operation_key,state FROM plan_operation")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (operation, state) in rows {
        valid_digest(&operation)?;
        if state == "forgotten" {
            continue;
        }
        let entry = view(conn, &operation)?;
        Uuid::parse_str(&entry.receipt_id)?;
        let (saved,task_object,before,attachment,turn,permit,ordinal):(String,String,Option<String>,String,String,String,u32)=conn.query_row("SELECT payload_hash,task_object,before_revision,attachment_digest,turn_id,permit_digest,ordinal FROM plan_operation WHERE operation_key=?1",[&operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)))?;
        valid_digest(&saved)?;
        valid_digest(&attachment)?;
        valid_digest(&permit)?;
        ensure!(
            task_object == entry.value.task.reference.object
                && turn == entry.value.request.occurrence.turn_id
                && ordinal == entry.value.request.occurrence.ordinal,
            "plan receipt occurrence mismatch"
        );
        let task_value = task::canonical_value(conn, &entry.value.task.reference)?;
        let (instance, domain): (String, String) = conn.query_row(
            "SELECT instance,domain FROM meta WHERE singleton=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let session = TaskSession {
            instance,
            account: task_value.account,
            session_id: task_value.session_id,
            access: Access {
                domain,
                scopes: required_scopes(conn, &entry.value.task.reference.revision)?,
            },
        };
        ensure!(
            operation
                == key(
                    &session.instance,
                    &session,
                    &entry.value.task,
                    &attachment,
                    &entry.value.request.occurrence
                )?,
            "plan receipt key mismatch"
        );
        let (previous, seq): (Option<String>, i64) = conn.query_row(
            "SELECT previous,seq FROM revision WHERE id=?1",
            [&entry.reference.revision],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(previous == before, "plan receipt predecessor mismatch");
        let expected = before.map(|revision| RevisionRef {
            object: entry.reference.object.clone(),
            revision,
        });
        ensure!(
            saved == payload(&entry.value.task, expected.as_ref(), &entry.value.request)?,
            "plan receipt payload mismatch"
        );
        let immediate: Option<String> = conn
            .query_row(
                "SELECT id FROM revision WHERE object_id=?1 AND seq<?2 ORDER BY seq DESC LIMIT 1",
                params![entry.reference.object, seq],
                |r| r.get(0),
            )
            .optional()?;
        ensure!(immediate == previous, "plan revision fork");
        ensure!(
            dependencies(conn, &entry.reference.revision)? == [entry.value.task.reference.clone()],
            "invalid plan dependency closure"
        );
        ensure!(
            required_scopes(conn, &entry.reference.revision)? == session.access.scopes,
            "plan scopes differ from task"
        );
        let restricted: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM restriction WHERE object_id IN (?1,?2))",
            params![entry.reference.object, task_object],
            |r| r.get(0),
        )?;
        ensure!(!restricted, "restricted plan retained content");
        if let Some(expected) = expected {
            let prior = canonical(conn, &expected)?;
            ensure!(prior.task == entry.value.task, "plan task binding changed");
            validate_transition(Some(&prior.request.snapshot), &entry.value.request.snapshot)?;
        } else {
            validate_transition(None, &entry.value.request.snapshot)?;
        }
        let earlier: Option<u32> = conn.query_row(
            "SELECT max(p.ordinal) FROM plan_operation p JOIN revision r ON r.id=p.after_revision WHERE p.task_object=?1 AND p.attachment_digest=?2 AND p.turn_id=?3 AND r.seq<?4",
            params![task_object, attachment, turn, seq], |r| r.get(0),
        )?;
        ensure!(
            earlier.is_none_or(|prior| prior < ordinal),
            "plan ledger occurrence order mismatch"
        );
    }
    Ok(())
}
fn validate_transition(old: Option<&PlanSnapshot>, new: &PlanSnapshot) -> Result<()> {
    // A declaration originates only from a present item. Once its item is
    // absent, carry it verbatim: dropping cannot rewrite the last open check
    // or manufacture a completion. Initial revisions use the same rules.
    for (content, next) in &new.declarations {
        let prior = old.and_then(|snapshot| snapshot.declarations.get(content));
        let item = new.items.iter().find(|item| item.content == *content);
        match (prior, item) {
            (None, Some(item)) => ensure!(
                item.check.as_ref() == Some(&next.check)
                    && next.frozen == (item.status == PlanItemStatus::Completed),
                "new check declaration lacks matching item state"
            ),
            (None, None) => bail!("new check declaration has no present item"),
            (Some(prior), None) => {
                ensure!(next == prior, "absent item's check declaration changed")
            }
            (Some(prior), Some(item)) => {
                if prior.frozen {
                    ensure!(next == prior, "frozen declaration changed");
                } else if item.status == PlanItemStatus::Completed {
                    ensure!(
                        next.frozen && next.check == prior.check,
                        "completing step changed its last open check"
                    );
                } else {
                    ensure!(
                        !next.frozen,
                        "open step cannot freeze its check declaration"
                    );
                }
            }
        }
    }
    for (content, prior) in old.into_iter().flat_map(|snapshot| &snapshot.declarations) {
        if !new.declarations.contains_key(content) {
            ensure!(
                !prior.frozen
                    && new.items.iter().any(|item| item.content == *content
                        && item.status != PlanItemStatus::Completed
                        && item.check.is_none()),
                "lost lifetime check declaration"
            );
        }
    }
    Ok(())
}

pub(super) fn invalidate_forgotten(conn: &Connection) -> Result<()> {
    if task::version(conn)? != 10 {
        return Ok(());
    }
    conn.execute("UPDATE plan_operation SET payload_hash=NULL,receipt_id=NULL,task_object=NULL,plan_object=NULL,before_revision=NULL,after_revision=NULL,attachment_digest=NULL,turn_id=NULL,permit_digest=NULL,ordinal=NULL,state='forgotten' WHERE task_object IN (SELECT id FROM doomed) OR plan_object IN (SELECT id FROM doomed)",[])?;
    Ok(())
}
