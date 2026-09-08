//! Owner-facing workflow commands; all state and scheduling rules live in core.
use crate::GlobalOpts;
use anyhow::{ensure, Result};
use chrono::{DateTime, Utc};
use mecha_core::{
    outbox::OutboxStore,
    questions::QuestionStore,
    workflow::{AttentionPolicy, Check, Commitment, ObservationCache, Workflow, WorkflowStore},
};

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Cmd,
}
#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Today's urgent items, decisions, verified work and waiting work.
    Today,
    List,
    /// Remove a completion check by its one-based position.
    Uncheck {
        id: String,
        number: usize,
    },
    /// Cancel tracking and block further task/chat/trigger runs until explicitly reopened.
    /// Does not claim completion or stop an active runner.
    Cancel {
        id: String,
        #[arg(long)]
        reason: String,
    },
    Show {
        id: String,
    },
    /// Track an outcome. Does not run an agent or close a board task.
    Add {
        title: String,
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        session: Option<String>,
    },
    /// Record an explicit commitment and when to follow up (RFC3339 timestamps).
    Commit {
        id: String,
        #[arg(long)]
        party: String,
        #[arg(long)]
        source: String,
        #[arg(long)]
        due: DateTime<Utc>,
        #[arg(long)]
        follow_up: DateTime<Utc>,
    },
    /// Add an artifact-content or confirmed-delivery completion check.
    Check {
        id: String,
        #[arg(long, requires = "contains", conflicts_with = "delivered")]
        artifact: Option<String>,
        #[arg(long, requires = "artifact")]
        contains: Option<String>,
        #[arg(long, conflicts_with = "artifact")]
        delivered: Option<String>,
    },
    /// Read the actual artifacts and delivery records, retaining failures as evidence.
    Verify {
        id: String,
    },
    /// Defer reminders. Urgent items remain visible on Today.
    Snooze {
        id: String,
        until: DateTime<Utc>,
    },
    /// Acknowledge the current in-app notice.
    Ack {
        id: String,
    },
    /// Close a verified workflow. Board task closure remains `mecha tasks set`.
    Close {
        id: String,
    },
    Reopen {
        id: String,
    },
    /// Clear stale ownership only after confirming the previous runner stopped.
    Recover {
        id: String,
        #[arg(long)]
        reason: String,
    },
    /// Wait for another workflow to be completed before this one can run.
    Depend {
        id: String,
        dependency: String,
    },
    /// Continue the recorded task conversation under its existing approval and taint rules.
    Resume {
        id: String,
    },
    /// Refresh linked state and collect a coalesced in-app digest. No model or sends.
    Tick {
        #[arg(long)]
        dry_run: bool,
    },
    /// Set owner-only notification timing; absent configuration defaults to UTC.
    Attention {
        #[arg(long)]
        timezone: chrono_tz::Tz,
        #[arg(long, default_value_t = 22)]
        quiet_start: u32,
        #[arg(long, default_value_t = 8)]
        quiet_end: u32,
        #[arg(long, default_value_t = 8)]
        digest_hour: u32,
    },
}
fn default_outbox_root() -> Result<std::path::PathBuf> {
    match mecha_core::config::Config::load_global()?.outbox.dir {
        Some(root) => Ok(root),
        None => OutboxStore::default_root(),
    }
}
fn open_outbox(root: std::path::PathBuf) -> Result<Option<OutboxStore>> {
    if root.try_exists()? {
        Ok(Some(OutboxStore::open(root)?))
    } else {
        Ok(None)
    }
}
fn outbox(w: Option<&Workflow>) -> Result<Option<OutboxStore>> {
    open_outbox(match w.and_then(|w| w.outbox_root.clone()) {
        Some(root) => root,
        None => default_outbox_root()?,
    })
}
/// One config read and one store handle per root, scoped to a Today/tick request.
struct OutboxPool {
    default_root: std::path::PathBuf,
    stores: std::collections::BTreeMap<std::path::PathBuf, Option<OutboxStore>>,
}
impl OutboxPool {
    fn new() -> Result<Self> {
        Ok(Self {
            default_root: default_outbox_root()?,
            stores: Default::default(),
        })
    }
    fn get(&mut self, w: Option<&Workflow>) -> Result<Option<&OutboxStore>> {
        let root = w
            .and_then(|w| w.outbox_root.as_ref())
            .unwrap_or(&self.default_root)
            .clone();
        if let std::collections::btree_map::Entry::Vacant(entry) = self.stores.entry(root.clone()) {
            entry.insert(open_outbox(root.clone())?);
        }
        Ok(self.stores[&root].as_ref())
    }
}
fn questions() -> Result<Option<QuestionStore>> {
    let root = QuestionStore::default_root()?;
    if root.try_exists()? {
        Ok(Some(QuestionStore::open(root)?))
    } else {
        Ok(None)
    }
}
pub fn tick(dry_run: bool) -> Result<serde_json::Value> {
    let store = WorkflowStore::default_store()?;
    let policy = store.policy()?;
    let questions = questions()?;
    let mut outboxes = OutboxPool::new()?;
    let mut cache = ObservationCache::default();
    let now = Utc::now();
    let mut notices = vec![];
    for mut w in store.list()? {
        if w.closed_at.is_some() {
            continue;
        }
        let out = outboxes.get(Some(&w))?;
        if dry_run {
            w.observe_cached(&mut cache, out, questions.as_ref(), now);
            store.observe_dependencies(&mut w, now);
            if w.tick(&policy, now) {
                notices.push(serde_json::json!({"id":w.id, "notice":w.notice}));
            }
        } else {
            let refreshed =
                store.refresh_cached(&w.id, &mut cache, out, questions.as_ref(), now)?;
            let mut emitted = false;
            let row = store.update(&refreshed.id, |w| {
                emitted = w.tick(&policy, now);
                Ok(())
            })?;
            if emitted {
                notices.push(serde_json::json!({"id":row.id, "notice":row.notice}));
            }
        }
    }
    Ok(serde_json::json!({"notices":notices, "dry_run":dry_run}))
}
pub fn today() -> Result<serde_json::Value> {
    let store = WorkflowStore::default_store()?;
    let questions = questions()?;
    let mut outboxes = OutboxPool::new()?;
    let mut cache = ObservationCache::default();
    let now = Utc::now();
    let mut items = vec![];
    let mut closed = vec![];
    let mut linked_outbox = std::collections::HashSet::new();
    let mut linked_questions = std::collections::HashSet::new();
    for mut w in store.list()? {
        if w.closed_at.is_some() {
            closed.push(serde_json::json!({"id":w.id, "title":w.title, "state":w.state, "closed_at":w.closed_at}));
            continue;
        }
        let out = outboxes.get(Some(&w))?;
        w.observe_cached(&mut cache, out, questions.as_ref(), now);
        store.observe_dependencies(&mut w, now);
        if w.state != "running" && !w.checks.is_empty() {
            w.check_evidence(out, now)?;
        }
        linked_outbox.extend(w.outbox.iter().cloned());
        linked_questions.extend(w.questions.iter().cloned());
        let waiting_for: std::collections::BTreeSet<_> = w
            .observed
            .iter()
            .filter_map(|(key, state)| match state.as_str() {
                "open" => Some("A question needs your answer"),
                "pending" => Some("A draft needs review"),
                "delivery_unknown" => Some("Delivery needs reconciliation"),
                "unreadable" => Some("A linked record could not be read"),
                "waiting" if key.starts_with("dependency:") => {
                    Some("An earlier workflow has not finished")
                }
                "rejected" | "abandoned" => {
                    Some("A draft or question was declined; review the next step")
                }
                _ => None,
            })
            .collect();
        items.push(serde_json::json!({"id":w.id, "title":w.title, "section":w.section(now), "state":w.state,
            "task_id":w.task_id, "session_id":w.session_id, "outbox":w.outbox, "questions":w.questions,
            "waiting_for":waiting_for, "depends_on":w.depends_on, "commitment":w.commitment, "notice":w.notice, "verified_at":w.verified_at,
            "verification":w.verification, "snoozed_until":w.snoozed_until, "workflow":true}));
    }
    if let Some(out) = outboxes.get(None)? {
        for d in cache
            .outbox_items(out)?
            .iter()
            .filter(|d| d.status == "pending" && !linked_outbox.contains(&d.id))
        {
            items.push(serde_json::json!({"id":d.id,"title":d.summary,"section":if d.delivery_uncertain() {"urgent"} else {"decisions"}, "state":if d.delivery_uncertain() {"delivery uncertain"} else {"draft ready for review"},"outbox":[d.id]}));
        }
    }
    if let Some(qs) = questions {
        let (rows, skipped) = cache.question_items(&qs)?;
        ensure!(skipped == 0, "{skipped} question records could not be read");
        for q in rows
            .iter()
            .filter(|q| q.is_open() && !linked_questions.contains(&q.id))
        {
            items.push(serde_json::json!({"id":q.id,"title":q.asked(),"section":"decisions","state":"answer needed","task_id":q.task_id,"session_id":q.session_id,"questions":[q.id]}));
        }
    }
    closed.sort_by(|a, b| b["closed_at"].as_str().cmp(&a["closed_at"].as_str()));
    Ok(serde_json::json!({"as_of":now,"items":items,"closed":closed}))
}
pub async fn run(global: &GlobalOpts, args: Args) -> Result<()> {
    let store = WorkflowStore::default_store()?;
    let now = Utc::now();
    let closing = matches!(&args.cmd, Cmd::Close { .. });
    let value = match args.cmd {
        Cmd::Today => today()?,
        Cmd::Tick { dry_run } => tick(dry_run)?,
        Cmd::Uncheck { id, number } => serde_json::to_value(store.update(&id, |w| {
            ensure!(
                number > 0 && number <= w.checks.len(),
                "check number is out of range"
            );
            let removed = w.checks.remove(number - 1);
            w.record("check_removed", serde_json::to_string(&removed)?, now);
            Ok(())
        })?)?,
        Cmd::Cancel { id, reason } => serde_json::to_value(store.update(&id, |w| {
            ensure!(w.state != "running", "stop the running task first");
            ensure!(!reason.trim().is_empty(), "record a reason");
            w.closed_at = Some(now);
            w.state = "cancelled".into();
            w.notice = None;
            w.record("cancelled", reason, now);
            Ok(())
        })?)?,
        Cmd::List => serde_json::to_value(store.list()?)?,
        Cmd::Show { id } => serde_json::to_value(store.get(&id)?)?,
        Cmd::Add {
            title,
            task,
            session,
        } => {
            ensure!(!title.trim().is_empty(), "title is required");
            let id = task
                .clone()
                .unwrap_or_else(|| format!("flow-{}", mecha_core::session::Session::new_id()));
            let workspace = global
                .workspace
                .clone()
                .unwrap_or(std::env::current_dir()?)
                .canonicalize()?;
            let mut w = Workflow::new(id, title, workspace, now);
            w.task_id = task;
            w.session_id = session;
            w.outbox_root = mecha_core::config::Config::load_global()?.outbox.dir;
            w.record("created", "Owner created workflow", now);
            serde_json::to_value(store.create(w)?)?
        }
        Cmd::Commit {
            id,
            party,
            source,
            due,
            follow_up,
        } => serde_json::to_value(store.update(&id, |w| {
            ensure!(
                !party.trim().is_empty() && !source.trim().is_empty(),
                "party and source are required"
            );
            ensure!(
                follow_up <= due,
                "follow-up must be no later than the commitment deadline"
            );
            w.commitment = Some(Commitment {
                party,
                source,
                due_at: due,
                follow_up_at: follow_up,
            });
            w.record("commitment", serde_json::to_string(&w.commitment)?, now);
            Ok(())
        })?)?,
        Cmd::Check {
            id,
            artifact,
            contains,
            delivered,
        } => {
            let check = match (artifact, contains, delivered) {
                (Some(path), Some(text), None) => {
                    ensure!(!text.is_empty(), "required content must be nonempty");
                    Check::ArtifactContains { path, text }
                }
                (None, None, Some(outbox_id)) => Check::Delivered { outbox_id },
                _ => anyhow::bail!("choose --artifact PATH --contains TEXT or --delivered ID"),
            };
            serde_json::to_value(store.update(&id, |w| {
                if let Check::Delivered { outbox_id } = &check {
                    if !w.outbox.contains(outbox_id) {
                        w.outbox.push(outbox_id.clone());
                    }
                }
                if !w.checks.contains(&check) {
                    w.checks.push(check);
                    w.record("check_added", "Owner specified completion evidence", now);
                }
                Ok(())
            })?)?
        }
        Cmd::Verify { id } | Cmd::Close { id } => {
            let close = closing;
            let w = store.get(&id)?;
            let out = outbox(Some(&w))?;
            let qs = questions()?;
            store.refresh(&id, out.as_ref(), qs.as_ref(), now)?;
            let verified = store.verify(&id, out.as_ref(), now)?;
            if close {
                ensure!(verified.verified(), "completion checks or linked actions are unresolved; inspect `mecha workflow verify {id}`");
                serde_json::to_value(store.update(&id, |w| w.close(now))?)?
            } else {
                serde_json::to_value(verified)?
            }
        }
        Cmd::Snooze { id, until } => serde_json::to_value(store.update(&id, |w| {
            ensure!(until > now, "snooze time must be in the future");
            w.snoozed_until = Some(until);
            w.notice = None;
            Ok(())
        })?)?,
        Cmd::Ack { id } => serde_json::to_value(store.update(&id, |w| {
            w.notice = None;
            Ok(())
        })?)?,
        Cmd::Recover { id, reason } => serde_json::to_value(store.recover(&id, &reason, now)?)?,
        Cmd::Reopen { id } => serde_json::to_value(store.update(&id, |w| {
            ensure!(w.state != "running", "stop the runner first; if it already stopped, use `mecha workflow recover {id} --reason ...`");
            w.runner_pid = None;
            w.run_id = None;
            w.closed_at = None;
            w.state = "idle".into();
            w.record("reopened", "Owner reopened workflow", now);
            Ok(())
        })?)?,
        Cmd::Depend { id, dependency } => {
            serde_json::to_value(store.depend(&id, &dependency, now)?)?
        }
        Cmd::Resume { id } => {
            let w = store.get(&id)?;
            w.ensure_open()?;
            let out = outbox(Some(&w))?;
            let qs = questions()?;
            let w = store.refresh(&id, out.as_ref(), qs.as_ref(), now)?;
            ensure!(w.actions_resolved(), "resolve outstanding questions, drafts, deliveries, or dependencies before resuming");
            let task = w
                .task_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("workflow has no delegated task"))?;
            let session = w
                .session_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("workflow has no recorded conversation"))?;
            let status = tokio::process::Command::new(crate::exe::self_exe())
                .args(["tasks", "work", task, "--resume", session, "--workspace"])
                .arg(&w.workspace)
                .status()
                .await?;
            ensure!(status.success(), "task resume failed: {status}");
            serde_json::json!({"resumed":id})
        }
        Cmd::Attention {
            timezone,
            quiet_start,
            quiet_end,
            digest_hour,
        } => {
            let policy = AttentionPolicy {
                timezone,
                quiet_start,
                quiet_end,
                digest_hour,
            };
            store.set_policy(&policy)?;
            serde_json::to_value(policy)?
        }
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
