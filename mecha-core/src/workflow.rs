//! Durable follow-through, by reference to the stores that own the work.
//!
//! No model tool mutates this store. Owner commands establish commitments and
//! completion checks; task runners record their lifecycle. Refresh observes
//! questions and deliveries without resuming a conversation or granting authority.
use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

/// Recent recovery context; transcripts retain the full conversation history.
const EVENT_HISTORY_LIMIT: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Check {
    /// Required bytes in an artifact, read within the recorded workspace.
    ArtifactContains { path: String, text: String },
    /// A recorded successful delivery, not merely a staged draft.
    Delivered { outbox_id: String },
    #[serde(other)]
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub check: Check,
    pub passed: bool,
    pub evidence: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub at: DateTime<Utc>,
    pub kind: String,
    pub detail: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commitment {
    pub party: String,
    /// A pointer to the owner's instruction, mail thread, or other originating record.
    pub source: String,
    pub due_at: DateTime<Utc>,
    pub follow_up_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub title: String,
    pub task_id: Option<String>,
    pub session_id: Option<String>,
    pub workspace: PathBuf,
    #[serde(default)]
    pub outbox_root: Option<PathBuf>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Lifecycle only; successful completion is established by checks separately.
    pub state: String,
    pub runner_pid: Option<u32>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub questions: Vec<String>,
    #[serde(default)]
    pub outbox: Vec<String>,
    #[serde(default)]
    pub observed: BTreeMap<String, String>,
    #[serde(default)]
    pub checks: Vec<Check>,
    #[serde(default)]
    pub verification: Vec<CheckResult>,
    #[serde(default)]
    pub verified_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub commitment: Option<Commitment>,
    #[serde(default)]
    pub snoozed_until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub events: Vec<Event>,
    /// Persists beyond the bounded event list so pruning cannot reuse notice keys.
    #[serde(default)]
    pub event_sequence: u64,
    #[serde(default)]
    pub last_notice_key: Option<String>,
    #[serde(default)]
    pub notice: Option<Notice>,
    #[serde(default)]
    pub closed_at: Option<DateTime<Utc>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notice {
    pub at: DateTime<Utc>,
    pub reason: String,
    pub urgent: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AttentionPolicy {
    #[serde(with = "timezone_serde")]
    pub timezone: chrono_tz::Tz,
    /// Local hours [start,end); equal hours disable quiet time.
    pub quiet_start: u32,
    pub quiet_end: u32,
    pub digest_hour: u32,
}
impl Default for AttentionPolicy {
    fn default() -> Self {
        Self {
            timezone: chrono_tz::UTC,
            quiet_start: 22,
            quiet_end: 8,
            digest_hour: 8,
        }
    }
}
impl AttentionPolicy {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.quiet_start < 24 && self.quiet_end < 24 && self.digest_hour < 24,
            "attention hours must be 0..23"
        );
        Ok(())
    }
    pub fn quiet(&self, now: DateTime<Utc>) -> bool {
        let h = now.with_timezone(&self.timezone).hour();
        if self.quiet_start <= self.quiet_end {
            h >= self.quiet_start && h < self.quiet_end
        } else {
            h >= self.quiet_start || h < self.quiet_end
        }
    }
}
impl Workflow {
    pub fn new(id: String, title: String, workspace: PathBuf, now: DateTime<Utc>) -> Self {
        Self {
            id,
            title,
            workspace,
            outbox_root: None,
            task_id: None,
            session_id: None,
            created_at: now,
            updated_at: now,
            state: "idle".into(),
            runner_pid: None,
            run_id: None,
            depends_on: vec![],
            questions: vec![],
            outbox: vec![],
            observed: BTreeMap::new(),
            checks: vec![],
            verification: vec![],
            verified_at: None,
            commitment: None,
            snoozed_until: None,
            events: vec![],
            event_sequence: 0,
            last_notice_key: None,
            notice: None,
            closed_at: None,
        }
    }
    pub fn record(&mut self, kind: &str, detail: impl Into<String>, now: DateTime<Utc>) {
        self.updated_at = now;
        // Legacy records lack a sequence; seed it from their retained history.
        self.event_sequence = self
            .event_sequence
            .max(self.events.len() as u64)
            .saturating_add(1);
        self.events.push(Event {
            at: now,
            kind: kind.into(),
            detail: detail.into(),
        });
        if self.events.len() > EVENT_HISTORY_LIMIT {
            self.events.drain(..self.events.len() - EVENT_HISTORY_LIMIT);
        }
        // Every material event invalidates old verification; refresh never manufactures success.
        self.verified_at = None;
        self.verification.clear();
    }
    pub fn verified(&self) -> bool {
        self.verified_at.is_some()
            && !self
                .observed
                .values()
                .any(|s| !matches!(s.as_str(), "sent" | "answered" | "closed"))
            && !self.checks.is_empty()
            && self.verification.len() == self.checks.len()
            && self.verification.iter().all(|c| c.passed)
    }
    pub fn section(&self, now: DateTime<Utc>) -> &'static str {
        if self.closed_at.is_some() {
            return "closed";
        }
        if self
            .observed
            .values()
            .any(|s| s == "delivery_unknown" || s == "unreadable")
            || self.state == "interrupted"
            || self.state == "failed"
            || !matches!(
                self.state.as_str(),
                "idle" | "running" | "awaiting_owner" | "failed" | "interrupted"
            )
            || self.commitment.as_ref().is_some_and(|c| c.due_at <= now)
        {
            return "urgent";
        }
        if self
            .observed
            .values()
            .any(|s| s == "open" || s == "pending")
        {
            return "decisions";
        }
        if self.verified() {
            "ready"
        } else {
            "waiting"
        }
    }
    /// Read a fresh snapshot, including drafts/questions left by a crashed runner.
    pub fn observe(
        &mut self,
        outbox: Option<&crate::outbox::OutboxStore>,
        questions: Option<&crate::questions::QuestionStore>,
        now: DateTime<Utc>,
    ) {
        let mut scan_errors = BTreeMap::new();
        if let Some(session) = &self.session_id {
            if let Some(store) = outbox {
                match store.items_strict() {
                    Ok(items) => {
                        for d in items
                            .into_iter()
                            .filter(|d| d.session_id.as_deref() == Some(session))
                        {
                            if !self.outbox.contains(&d.id) {
                                self.outbox.push(d.id);
                            }
                        }
                    }
                    Err(_) => {
                        scan_errors.insert("outbox_scan".into(), "unreadable".into());
                    }
                }
            }
            if let Some(store) = questions {
                match store.items_counting() {
                    Ok((items, skipped)) => {
                        for q in items.into_iter().filter(|q| &q.session_id == session) {
                            if !self.questions.contains(&q.id) {
                                self.questions.push(q.id);
                            }
                        }
                        if skipped > 0 {
                            scan_errors.insert("question_scan".into(), "unreadable".into());
                        }
                    }
                    Err(_) => {
                        scan_errors.insert("question_scan".into(), "unreadable".into());
                    }
                }
            }
        }
        if self.state == "running" && !self.runner_pid.is_some_and(crate::process_alive) {
            self.state = "interrupted".into();
            self.runner_pid = None;
            self.record(
                "interrupted",
                "Runner ended without a final record; inspect partial work before resuming",
                now,
            );
        }
        let mut observed = scan_errors;
        observed.extend(
            self.observed
                .iter()
                .filter(|(k, _)| k.starts_with("dependency:"))
                .map(|(k, v)| (k.clone(), v.clone())),
        );
        for id in &self.outbox {
            let state = outbox
                .and_then(|s| s.item(id).ok())
                .map(|d| {
                    if d.delivery_uncertain() {
                        "delivery_unknown".into()
                    } else {
                        d.status
                    }
                })
                .unwrap_or("unreadable".into());
            observed.insert(format!("outbox:{id}"), state);
        }
        for id in &self.questions {
            let state = questions
                .and_then(|s| {
                    if id.is_empty()
                        || !id
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                    {
                        None
                    } else {
                        s.get(id).ok()
                    }
                })
                .map(|q| q.status)
                .unwrap_or("unreadable".into());
            observed.insert(format!("question:{id}"), state);
        }
        if observed != self.observed {
            self.observed = observed;
            self.record(
                "source_changed",
                "Linked question or delivery state changed",
                now,
            );
        }
    }
    /// Re-read evidence at display or closure time; cached success cannot prove current state.
    pub fn check_evidence(
        &mut self,
        outbox: Option<&crate::outbox::OutboxStore>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        ensure!(
            self.state != "running",
            "cannot verify while a runner may change the artifacts"
        );
        let ctx = crate::tool::ToolCtx {
            workspace: self.workspace.clone(),
            ..Default::default()
        };
        self.verification = self
            .checks
            .iter()
            .map(|check| {
                let result: Result<String> = (|| match check {
                    Check::ArtifactContains { path, text } => {
                        ensure!(
                            !text.is_empty(),
                            "empty artifact checks do not establish completion"
                        );
                        let resolved = ctx.resolve(path)?;
                        use std::os::unix::fs::OpenOptionsExt;
                        let file = OpenOptions::new()
                            .read(true)
                            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
                            .open(&resolved)?;
                        ensure!(file.metadata()?.is_file(), "artifact is not a regular file");
                        let mut bytes = Vec::new();
                        file.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
                        ensure!(
                            bytes.len() <= 4 * 1024 * 1024,
                            "artifact exceeds verification limit"
                        );
                        ensure!(
                            String::from_utf8(bytes)?.contains(text),
                            "required content missing"
                        );
                        Ok(format!("Required content observed in {path}"))
                    }
                    Check::Delivered { outbox_id } => {
                        let d = outbox.context("outbox unavailable")?.item(outbox_id)?;
                        ensure!(
                            d.status == "sent" && !d.delivery_uncertain(),
                            "delivery not confirmed"
                        );
                        Ok(format!(
                            "Outbox {outbox_id} recorded sent at {}",
                            d.resolved_at.unwrap_or_default()
                        ))
                    }
                    Check::Unknown => anyhow::bail!("unknown completion check"),
                })();
                CheckResult {
                    check: check.clone(),
                    passed: result.is_ok(),
                    evidence: result.unwrap_or_else(|e| format!("{e:#}")),
                }
            })
            .collect();
        self.verified_at = Some(now);
        self.updated_at = now;
        Ok(())
    }
    /// Coalesced, durable in-app reminders. Never dispatches a send or wakes a model.
    pub fn tick(&mut self, policy: &AttentionPolicy, now: DateTime<Utc>) -> bool {
        if self.closed_at.is_some()
            || policy.quiet(now)
            || self.snoozed_until.is_some_and(|t| t > now)
        {
            return false;
        }
        let urgent = self.section(now) == "urgent";
        let due = self
            .commitment
            .as_ref()
            .is_some_and(|c| c.follow_up_at <= now);
        let changed = self
            .events
            .last()
            .is_some_and(|e| e.kind == "source_changed" || e.kind == "interrupted");
        if !due && !changed && !urgent {
            return false;
        }
        if !urgent && now.with_timezone(&policy.timezone).hour() < policy.digest_hour {
            return false;
        }
        // A changed observation can notify once more, while a static commitment gets one digest/day.
        let key = format!(
            "{}:{}:{}:{:?}",
            now.with_timezone(&policy.timezone).date_naive(),
            self.event_sequence.max(self.events.len() as u64),
            urgent,
            self.snoozed_until
        );
        if self.last_notice_key.as_deref() == Some(&key) {
            return false;
        }
        let reason = if urgent {
            "Needs attention: overdue, interrupted, or delivery uncertain"
        } else if due {
            "Follow-up due"
        } else {
            "A linked question or draft changed"
        };
        self.last_notice_key = Some(key);
        self.notice = Some(Notice {
            at: now,
            reason: reason.into(),
            urgent,
        });
        true
    }
}

/// All mutations take one process-wide file lock and atomically replace a record.
/// A reader never creates a store. Corruption is an error, never an empty queue.
pub struct WorkflowStore {
    root: PathBuf,
}
/// Cleans up early returns and unwinding while the host process stays alive.
pub struct WorkflowRun {
    root: PathBuf,
    id: String,
    run_id: String,
}
impl Drop for WorkflowRun {
    fn drop(&mut self) {
        let store = WorkflowStore::at(self.root.clone());
        if let Err(e) = store.update(&self.id, |w| {
            if w.state == "running" && w.run_id.as_deref() == Some(&self.run_id) {
                w.state = "interrupted".into();
                w.runner_pid = None;
                w.record(
                    "interrupted",
                    "Run exited before recording its outcome; inspect partial work",
                    Utc::now(),
                );
            }
            Ok(())
        }) {
            tracing::error!("could not clear workflow runner: {e:#}");
        }
    }
}
impl WorkflowStore {
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn default_store() -> Result<Self> {
        Ok(Self::at(crate::work::mecha_home()?.join("workflows")))
    }
    fn path(&self, id: &str) -> Result<PathBuf> {
        ensure!(
            !id.is_empty()
                && id.len() <= 128
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "invalid workflow id"
        );
        Ok(self.root.join(format!("{id}.json")))
    }
    fn lock(&self) -> Result<File> {
        use std::os::unix::{fs::OpenOptionsExt, io::AsRawFd};
        crate::create_private_dir(&self.root)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(self.root.join(".lock"))?;
        // SAFETY: the owned descriptor lives as long as this writer guard.
        ensure!(
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0,
            "locking workflow store: {}",
            std::io::Error::last_os_error()
        );
        Ok(file)
    }
    fn write(&self, w: &Workflow) -> Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        let path = self.path(&w.id)?;
        let tmp = path.with_extension("tmp");
        let mut f = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(&serde_json::to_vec_pretty(w)?)?;
        f.sync_all()?;
        fs::rename(tmp, path)?;
        File::open(&self.root)?.sync_all()?;
        Ok(())
    }
    pub fn get(&self, id: &str) -> Result<Workflow> {
        let w: Workflow =
            serde_json::from_slice(&fs::read(self.path(id)?)?).context("reading workflow")?;
        ensure!(w.id == id, "workflow id does not match filename");
        Ok(w)
    }
    pub fn list(&self) -> Result<Vec<Workflow>> {
        let dir = match fs::read_dir(&self.root) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e.into()),
        };
        let mut rows = vec![];
        for entry in dir {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                rows.push(
                    self.get(
                        path.file_stem()
                            .and_then(|s| s.to_str())
                            .context("invalid workflow filename")?,
                    )?,
                );
            }
        }
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(rows)
    }
    pub fn create(&self, w: Workflow) -> Result<Workflow> {
        let _lock = self.lock()?;
        ensure!(!self.path(&w.id)?.try_exists()?, "workflow already exists");
        self.write(&w)?;
        Ok(w)
    }
    pub fn update(
        &self,
        id: &str,
        f: impl FnOnce(&mut Workflow) -> Result<()>,
    ) -> Result<Workflow> {
        let _lock = self.lock()?;
        let mut w = self.get(id)?;
        f(&mut w)?;
        ensure!(w.id == id, "cannot rename workflow");
        self.write(&w)?;
        Ok(w)
    }
    pub fn start_task(
        &self,
        task: &str,
        title: &str,
        session: &str,
        workspace: &Path,
        now: DateTime<Utc>,
    ) -> Result<WorkflowRun> {
        let _lock = self.lock()?;
        let mut w = if self.path(task)?.try_exists()? {
            self.get(task)?
        } else {
            Workflow::new(task.into(), title.into(), workspace.into(), now)
        };
        ensure!(
            !w.runner_pid.is_some_and(crate::process_alive),
            "workflow already has a live runner"
        );
        ensure!(
            w.closed_at.is_none(),
            "workflow is closed; reopen it before starting another run"
        );
        for dependency in &w.depends_on {
            let predecessor = self.get(dependency)?;
            ensure!(
                predecessor.state == "closed" && predecessor.closed_at.is_some(),
                "dependency {dependency} must be completed first"
            );
        }
        w.task_id = Some(task.into());
        w.session_id = Some(session.into());
        w.workspace = workspace.into();
        w.state = "running".into();
        w.runner_pid = Some(std::process::id());
        let run_id = uuid::Uuid::new_v4().to_string();
        w.run_id = Some(run_id.clone());
        w.record("started", session, now);
        self.write(&w)?;
        Ok(WorkflowRun {
            root: self.root.clone(),
            id: task.into(),
            run_id,
        })
    }
    pub fn finish_task(
        &self,
        id: &str,
        failed: bool,
        drafts: Vec<String>,
        questions: Vec<String>,
        now: DateTime<Utc>,
    ) -> Result<Workflow> {
        self.update(id, |w| {
            w.state = if failed { "failed" } else { "awaiting_owner" }.into();
            w.runner_pid = None;
            for id in drafts {
                if !w.outbox.contains(&id) {
                    w.outbox.push(id);
                }
            }
            for id in questions {
                if !w.questions.contains(&id) {
                    w.questions.push(id);
                }
            }
            w.record(
                if failed { "failed" } else { "run_finished" },
                "Review recorded artifacts and linked actions",
                now,
            );
            Ok(())
        })
    }
    /// Observe the owning stores. References missing or unreadable stay explicitly unknown.
    pub fn refresh(
        &self,
        id: &str,
        outbox: Option<&crate::outbox::OutboxStore>,
        questions: Option<&crate::questions::QuestionStore>,
        now: DateTime<Utc>,
    ) -> Result<Workflow> {
        self.update(id, |w| {
            w.observe(outbox, questions, now);
            self.observe_dependencies(w, now);
            Ok(())
        })
    }
    pub fn verify(
        &self,
        id: &str,
        outbox: Option<&crate::outbox::OutboxStore>,
        now: DateTime<Utc>,
    ) -> Result<Workflow> {
        self.update(id, |w| w.check_evidence(outbox, now))
    }
    /// Link existing workflows without duplicating the task board. Cycles are rejected.
    pub fn depend(&self, id: &str, dependency: &str, now: DateTime<Utc>) -> Result<Workflow> {
        self.update(id, |w| {
            let mut todo = vec![dependency.to_string()];
            let mut seen = std::collections::BTreeSet::new();
            while let Some(next) = todo.pop() {
                ensure!(next != id, "workflow dependencies would form a cycle");
                if seen.insert(next.clone()) {
                    todo.extend(self.get(&next)?.depends_on);
                }
            }
            if !w.depends_on.iter().any(|d| d == dependency) {
                w.depends_on.push(dependency.into());
                w.record("dependency_added", dependency, now);
            }
            Ok(())
        })
    }
    pub fn observe_dependencies(&self, w: &mut Workflow, now: DateTime<Utc>) {
        let old = w.observed.clone();
        w.observed.retain(|k, _| !k.starts_with("dependency:"));
        for id in &w.depends_on {
            let state = match self.get(id) {
                Ok(d) if d.state == "closed" && d.closed_at.is_some() => "closed",
                Ok(_) => "waiting",
                Err(_) => "unreadable",
            };
            w.observed.insert(format!("dependency:{id}"), state.into());
        }
        if old != w.observed {
            w.record("source_changed", "Workflow dependency state changed", now);
        }
    }
    pub fn policy(&self) -> Result<AttentionPolicy> {
        let path = self.root.join("attention.toml");
        let policy = match fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => AttentionPolicy::default(),
            Err(e) => return Err(e.into()),
        };
        policy.validate()?;
        Ok(policy)
    }
    pub fn set_policy(&self, policy: &AttentionPolicy) -> Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        policy.validate()?;
        let _lock = self.lock()?;
        let tmp = self.root.join("attention.tmp");
        let mut f = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(toml::to_string_pretty(policy)?.as_bytes())?;
        f.sync_all()?;
        fs::rename(tmp, self.root.join("attention.toml"))?;
        File::open(&self.root)?.sync_all()?;
        Ok(())
    }
}

mod timezone_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(tz: &chrono_tz::Tz, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(tz.name())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<chrono_tz::Tz, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outbox::{OutboxKind, OutboxStore, Provenance};
    struct Home(PathBuf);
    impl Home {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("mecha-workflow-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn store(&self) -> WorkflowStore {
            WorkflowStore::at(self.0.join("workflows"))
        }
        fn workflow(&self) -> Workflow {
            Workflow::new(
                "flow-one".into(),
                "Prepare the grant reply".into(),
                self.0.clone(),
                at("2026-09-08T12:00:00Z"),
            )
        }
    }
    impl Drop for Home {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }
    #[test]
    fn event_history_is_bounded_without_suppressing_new_notices_after_restart() {
        let h = Home::new();
        let mut w = h.workflow();
        let now = at("2026-09-08T13:00:00Z");
        for i in 0..256 {
            w.record("source_changed", i.to_string(), now);
        }
        assert_eq!(w.events.len(), 128);
        assert_eq!(w.events.first().unwrap().detail, "128");
        assert_eq!(w.events.last().unwrap().detail, "255");
        assert!(w.tick(&AttentionPolicy::default(), now));
        let first_key = w.last_notice_key.clone();
        h.store().create(w).unwrap();
        let mut restored = h.store().get("flow-one").unwrap();
        assert!(!restored.tick(&AttentionPolicy::default(), now));
        restored.record("source_changed", "another change", now);
        assert_eq!(restored.events.len(), 128);
        assert!(restored.tick(&AttentionPolicy::default(), now));
        assert_ne!(restored.last_notice_key, first_key);
    }

    #[test]
    fn a_multi_day_workflow_requires_real_artifacts_and_delivery_after_restart() {
        let h = Home::new();
        let out = OutboxStore::open(h.0.join("outbox")).unwrap();
        let draft = out
            .stage(
                "mail_reply",
                OutboxKind::Message,
                serde_json::json!({"thread_id":"t-aurora", "body_markdown":"Thursday at 3pm"}),
                Default::default(),
                Provenance::default(),
            )
            .unwrap();
        let mut w = h.workflow();
        w.outbox.push(draft.id.clone());
        w.checks = vec![
            Check::ArtifactContains {
                path: "reply.md".into(),
                text: "tracked changes".into(),
            },
            Check::Delivered {
                outbox_id: draft.id.clone(),
            },
        ];
        h.store().create(w).unwrap();
        let now = at("2026-09-08T13:00:00Z");
        assert!(!h
            .store()
            .verify("flow-one", Some(&out), now)
            .unwrap()
            .verified());
        fs::write(h.0.join("reply.md"), "Please bring the tracked changes.").unwrap();
        assert!(
            !h.store()
                .verify("flow-one", Some(&out), now)
                .unwrap()
                .verified(),
            "staging isn't delivery"
        );
        {
            let _lock = out.lock().unwrap();
            out.begin_delivery(&draft.id).unwrap();
        }
        let later = at("2026-09-09T13:00:00Z");
        let restarted = h.store();
        assert!(
            !restarted
                .verify("flow-one", Some(&out), later)
                .unwrap()
                .verified(),
            "unknown delivery isn't success"
        );
        assert_eq!(
            restarted
                .refresh("flow-one", Some(&out), None, later)
                .unwrap()
                .section(later),
            "urgent"
        );
        {
            let _lock = out.lock().unwrap();
            out.reconcile_delivery(
                &draft.id,
                crate::outbox::DeliveryOutcome::Delivered,
                "destination message m-123",
            )
            .unwrap();
        }
        restarted
            .refresh("flow-one", Some(&out), None, later)
            .unwrap();
        assert!(restarted
            .verify("flow-one", Some(&out), later)
            .unwrap()
            .verified());
        fs::write(h.0.join("reply.md"), "wrong artifact").unwrap();
        assert!(
            !restarted
                .verify("flow-one", Some(&out), later)
                .unwrap()
                .verified(),
            "a previous pass cannot stand in for current evidence"
        );
    }
    #[test]
    fn a_crash_recovers_partial_drafts_and_never_looks_complete() {
        let h = Home::new();
        let now = at("2026-09-08T12:00:00Z");
        let mut w = h.workflow();
        w.session_id = Some("session-one".into());
        w.state = "running".into();
        w.runner_pid = Some(u32::MAX);
        h.store().create(w).unwrap();
        let out = OutboxStore::open(h.0.join("outbox")).unwrap();
        let draft = out
            .stage(
                "mail_reply",
                OutboxKind::Message,
                serde_json::json!({}),
                Default::default(),
                Provenance {
                    session_id: Some("session-one".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let recovered = h
            .store()
            .refresh("flow-one", Some(&out), None, now)
            .unwrap();
        assert_eq!(recovered.state, "interrupted");
        assert_eq!(recovered.outbox, vec![draft.id]);
        assert_eq!(recovered.section(now), "urgent");
        assert!(!recovered.verified());
        let count = recovered.events.len();
        assert_eq!(
            h.store()
                .refresh("flow-one", Some(&out), None, now)
                .unwrap()
                .events
                .len(),
            count,
            "unchanged polls must not invent events"
        );
    }
    #[test]
    fn quiet_hours_snooze_dedup_and_overdue_escalation_survive_restart() {
        let h = Home::new();
        let mut w = h.workflow();
        w.commitment = Some(Commitment {
            party: "Priya".into(),
            source: "owner instruction".into(),
            follow_up_at: at("2026-09-08T20:00:00Z"),
            due_at: at("2026-09-10T12:00:00Z"),
        });
        let policy = AttentionPolicy {
            timezone: chrono_tz::America::New_York,
            ..Default::default()
        };
        assert!(
            !w.tick(&policy, at("2026-09-09T02:00:00Z")),
            "22:00 local is quiet"
        );
        assert!(w.tick(&policy, at("2026-09-09T12:00:00Z")), "08:00 digest");
        h.store().create(w).unwrap();
        let mut w = h.store().get("flow-one").unwrap();
        assert!(
            !w.tick(&policy, at("2026-09-09T15:00:00Z")),
            "same daily digest is deduplicated after restart"
        );
        w.snoozed_until = Some(at("2026-09-10T14:00:00Z"));
        assert!(!w.tick(&policy, at("2026-09-10T13:00:00Z")));
        assert_eq!(
            w.section(at("2026-09-10T13:00:00Z")),
            "urgent",
            "snoozing reminders does not hide an overdue commitment"
        );
        assert!(w.tick(&policy, at("2026-09-10T14:00:00Z")));
        assert!(w.notice.unwrap().urgent);
        assert!(
            policy.quiet(at("2026-11-01T05:30:00Z")) && policy.quiet(at("2026-11-01T06:30:00Z")),
            "both repeated DST hours stay quiet"
        );
    }
    #[test]
    fn checks_fail_closed_for_missing_sources_unknown_variants_and_path_escape() {
        let h = Home::new();
        let mut w = h.workflow();
        let now = at("2026-09-08T12:00:00Z");
        assert!(!w.verified());
        w.outbox.push("missing".into());
        w.checks
            .push(serde_json::from_value(serde_json::json!({"kind":"future_check"})).unwrap());
        w.checks.push(Check::ArtifactContains {
            path: "../secret".into(),
            text: "secret".into(),
        });
        h.store().create(w).unwrap();
        h.store().refresh("flow-one", None, None, now).unwrap();
        let result = h.store().verify("flow-one", None, now).unwrap();
        assert!(!result.verified());
        assert!(result.verification.iter().all(|r| !r.passed));
        assert_eq!(result.section(now), "urgent");
        assert!(h.store().get("../outside").is_err());
        fs::write(h.0.join("workflows/corrupt.json"), "{").unwrap();
        assert!(h.store().list().is_err());
    }
    #[test]
    fn concurrent_writers_preserve_every_event() {
        let h = Home::new();
        h.store().create(h.workflow()).unwrap();
        std::thread::scope(|scope| {
            for i in 0..8 {
                let store = h.store();
                scope.spawn(move || {
                    store
                        .update("flow-one", |w| {
                            w.record("owner_note", i.to_string(), Utc::now());
                            Ok(())
                        })
                        .unwrap();
                });
            }
        });
        assert_eq!(h.store().get("flow-one").unwrap().events.len(), 8);
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    #[test]
    fn dependencies_reject_cycles_and_a_dropped_runner_records_interruption() {
        let root =
            std::env::temp_dir().join(format!("mecha-workflow-deps-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let store = WorkflowStore::at(root.join("workflows"));
        let now = Utc::now();
        for id in ["prepare", "send"] {
            store
                .create(Workflow::new(id.into(), id.into(), root.clone(), now))
                .unwrap();
        }
        store.depend("send", "prepare", now).unwrap();
        assert!(store.depend("prepare", "send", now).is_err());
        assert!(store.start_task("send", "send", "s", &root, now).is_err());
        let guard = store
            .start_task("prepare", "prepare", "s", &root, now)
            .unwrap();
        assert!(
            store
                .start_task("prepare", "prepare", "s", &root, now)
                .is_err(),
            "one live runner"
        );
        drop(guard);
        assert_eq!(
            store.get("prepare").unwrap().state,
            "interrupted",
            "process is still alive, but the run isn't"
        );
        store
            .update("prepare", |w| {
                w.closed_at = Some(now);
                w.state = "closed".into();
                Ok(())
            })
            .unwrap();
        let guard = store.start_task("send", "send", "s", &root, now).unwrap();
        store
            .finish_task("send", false, vec![], vec![], now)
            .unwrap();
        drop(guard);
        assert_eq!(
            store.get("send").unwrap().state,
            "awaiting_owner",
            "guard cannot overwrite a recorded outcome"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
