//! Provider boundary for the core rule engine. OAuth never crosses it; every
//! engine Graph client opts into immutable IDs, including mutation clients.
use crate::{
    accounts::Provider,
    google::gmail::GmailProvider,
    microsoft::graph_mail::OutlookProvider,
    types::{Email, MailDirection, MailError},
    unified::{self, MailTools, TriageResult},
};
use anyhow::{bail, ensure, Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use mecha_core::{
    mail_rules::{
        engine::{
            self, ActionResult, Candidate, Classified, Mailbox, Pass, Task, TaskSink, ThreadRead,
        },
        Action, ActionVerb, FailureKind, GateStatus, LoadedRule, RuleStore,
    },
    mail_triage::{self, rule_records},
    mcp::McpClient,
    tool::{Tool, ToolCtx},
};
use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

pub struct RuleMailbox {
    tools: MailTools,
    triage_root: PathBuf,
    rule_root: PathBuf,
    dry_run: bool,
}
impl RuleMailbox {
    pub fn load(dry_run: bool, store: &RuleStore) -> Result<Self> {
        Ok(Self {
            tools: MailTools::load()?,
            rule_root: store.root().to_owned(),
            dry_run,
            triage_root: mail_triage::TriageStore::default_root()?,
        })
    }
    pub fn accounts(&self, selected: Option<&str>) -> Result<Vec<String>> {
        let names: Vec<_> = self
            .tools
            .accounts()
            .iter()
            .filter(|a| selected.is_none_or(|s| a.name == s))
            .map(|a| a.name.clone())
            .collect();
        ensure!(
            !names.is_empty(),
            "no configured account matches the selection"
        );
        Ok(names)
    }
    async fn with_token<T, F, Fut>(&self, account: &unified::Account, f: F) -> Result<T, MailError>
    where
        F: Fn(String) -> Fut,
        Fut: std::future::Future<Output = Result<T, MailError>>,
    {
        if self.dry_run {
            let token = account
                .manager
                .cached_access_token()
                .await
                .map_err(|e| MailError::AuthError(e.to_string()))?;
            f(token).await
        } else {
            unified::with_token(&account.manager, f).await
        }
    }

    fn account(&self, name: &str) -> Result<&unified::Account> {
        self.tools
            .accounts()
            .iter()
            .find(|a| a.name == name)
            .context("unknown account")
    }
    fn record(&self, account: &str, thread: &str) -> Result<Option<mail_triage::Record>> {
        classified_record(&self.triage_root, account, thread)
    }
}

fn classified_record(
    root: &std::path::Path,
    account: &str,
    thread: &str,
) -> Result<Option<mail_triage::Record>> {
    let record =
        mail_triage::TriageStore::open_existing(root).and_then(|store| store.get(account, thread));
    // Inbox views tolerate a broken overlay; authorization cannot. Check even
    // without a classifier record, since rules may already have annotations.
    rule_records::read(root, account, thread).context("rule annotations unreadable")?;
    Ok(record)
}

fn owner_addresses(
    accounts: &[Option<String>],
    selected: Option<&str>,
    store: &RuleStore,
) -> Result<Vec<String>> {
    ensure!(
        selected.is_some_and(|a| !a.trim().is_empty()),
        "account address unknown"
    );
    ensure!(
        accounts
            .iter()
            .all(|a| a.as_deref().is_some_and(|a| !a.trim().is_empty())),
        "configured account address unknown"
    );
    let mut owners = store.aliases()?.aliases;
    owners.extend(accounts.iter().flatten().cloned());
    Ok(owners
        .into_iter()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

fn finish_read(
    account: &str,
    thread: &str,
    emails: Result<Vec<Email>, MailError>,
    owners: Result<Vec<String>>,
    record: Result<Option<mail_triage::Record>>,
) -> Result<ThreadRead> {
    let emails = match emails {
        Ok(emails) => emails,
        Err(MailError::ParseError(_)) => {
            return Ok(ThreadRead::Held {
                reason: "provider message facts unknown".into(),
            });
        }
        Err(error) => return Err(error.into()),
    };
    let Ok(record) = record else {
        return Ok(ThreadRead::Held {
            reason: "rule annotations unreadable".into(),
        });
    };
    candidate_with_owners(account, thread, &emails, owners, record.as_ref())
}

fn candidate_with_owners(
    account: &str,
    thread: &str,
    emails: &[Email],
    owners: Result<Vec<String>>,
    record: Option<&mail_triage::Record>,
) -> Result<ThreadRead> {
    let Ok(owners) = owners else {
        return Ok(ThreadRead::Held {
            reason: "owner addresses unknown".into(),
        });
    };
    Ok(match candidate(account, thread, emails, &owners, record) {
        Ok(candidate) => candidate.into(),
        Err(reason) => ThreadRead::Held {
            reason: reason.into(),
        },
    })
}

fn candidate(
    account: &str,
    thread: &str,
    emails: &[Email],
    owners: &[String],
    record: Option<&mail_triage::Record>,
) -> Result<Candidate, &'static str> {
    // This function only inspects facts, never performs provider I/O. Its
    // uncertainty must become a held read, not a closing failed occasion.
    if owners.is_empty() {
        return Err("account address unknown");
    }
    if !emails.iter().all(|e| {
        e.sender_known == Some(true)
            && !e.from_address.is_empty()
            && e.direction(owners).is_some()
            && !e.provider_id.is_empty()
    }) {
        return Err("message direction unknown");
    }
    let inbound: Vec<_> = emails
        .iter()
        .filter(|e| e.direction(owners) == Some(MailDirection::Inbound))
        .collect();
    if inbound.is_empty() {
        return Err("no inbound message");
    }
    // With several inbound messages an unknown arrival makes newest selection
    // unknowable. For one message preserve the unknown for an explicit hold.
    if inbound.len() > 1 && inbound.iter().any(|e| e.provider_arrival_at.is_none()) {
        return Err("newest inbound message is unknown");
    }
    let email = inbound
        .into_iter()
        .max_by_key(|e| e.provider_arrival_at)
        .ok_or("newest inbound missing")?;
    let arrival_at = email.provider_arrival_at;
    let classified = record
        .filter(|r| r.state != mail_triage::FAILED && r.verdict.is_some())
        // Deployed classifiers stored envelope stamps at whole-second precision.
        // Compare at that precision for both providers, while retaining precise
        // arrival for activation. A later classified_at cannot certify stale input.
        .filter(|r| {
            arrival_at.is_some_and(|arrival| {
                DateTime::parse_from_rfc3339(&r.date)
                    .is_ok_and(|d| d.timestamp() >= arrival.timestamp())
            })
        })
        .and_then(|r| {
            Some(Classified {
                at: DateTime::parse_from_rfc3339(&r.classified_at)
                    .ok()?
                    .with_timezone(&Utc),
                deadline: r
                    .verdict
                    .as_ref()?
                    .deadline
                    .as_deref()
                    .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()),
            })
        });
    Ok(Candidate {
        message: mecha_core::mail_rules::MessageRef {
            account: account.into(),
            thread_id: thread.into(),
            provider_message_id: email.provider_id.clone(),
            message_id: email.message_id.clone().unwrap_or_default(),
            sender: email.from_address.clone(),
            subject: email.subject.clone(),
        },
        arrival_at,
        headers_complete: email.headers_complete,
        subject_complete: email.subject_complete,
        subject_is_raw: email.subject_is_raw,
        recipients_complete: email.recipients_complete,
        attachment_presence: email.attachment_presence,
        to: email
            .to_addresses
            .iter()
            .chain(&email.cc_addresses)
            .chain(&email.bcc_addresses)
            .filter(|_| email.recipients_complete != Some(false))
            .cloned()
            .collect(),
        list_id: email.list_id.clone(),
        headers: email.headers.clone(),
        classified,
    })
}

pub(crate) async fn gmail_action(
    provider: &GmailProvider,
    message: &mecha_core::mail_rules::MessageRef,
    action: ActionVerb,
) -> Result<TriageResult, MailError> {
    let id = &message.provider_message_id;
    match action {
        ActionVerb::Archive => provider.archive_message(id).await?,
        ActionVerb::Trash => provider.trash_message(id).await?,
        ActionVerb::Read => provider.set_message_read(id, true).await?,
        ActionVerb::Unread => provider.set_message_read(id, false).await?,
        _ => return Err(MailError::InvalidInput("not a mailbox verb".into())),
    }
    Ok(TriageResult::Uncounted)
}
pub(crate) async fn outlook_action(
    provider: &OutlookProvider,
    message: &mecha_core::mail_rules::MessageRef,
    action: ActionVerb,
) -> Result<TriageResult, MailError> {
    let id = &message.provider_message_id;
    match action {
        ActionVerb::Archive => provider.move_message(id, "archive").await?,
        ActionVerb::Trash => provider.move_message(id, "deleteditems").await?,
        ActionVerb::Read => provider.set_message_read(id, true).await?,
        ActionVerb::Unread => provider.set_message_read(id, false).await?,
        _ => return Err(MailError::InvalidInput("not a mailbox verb".into())),
    };
    Ok(TriageResult::Uncounted)
}

pub(crate) fn failure(error: MailError) -> ActionResult {
    let kind = mutation_failure_kind(crate::microsoft::graph_mail::MutationFailureKind::of(
        &error,
    ));
    ActionResult::Failed {
        kind,
        reason: error.to_string(),
    }
}
fn mutation_failure_kind(kind: crate::microsoft::graph_mail::MutationFailureKind) -> FailureKind {
    use crate::microsoft::graph_mail::MutationFailureKind;
    match kind {
        MutationFailureKind::Environment => FailureKind::Environment,
        MutationFailureKind::Action => FailureKind::Action,
        MutationFailureKind::Unknown => FailureKind::Unknown,
    }
}
pub(crate) fn triage_result(result: TriageResult) -> ActionResult {
    match result {
        TriageResult::Uncounted => ActionResult::Done,
        TriageResult::Count(count) if count.failed == 0 => ActionResult::Done,
        TriageResult::Count(count) => ActionResult::Failed {
            kind: count
                .failure_kind
                .map_or(FailureKind::Unknown, mutation_failure_kind),
            reason: format!(
                "partial completion: {} succeeded; {} failed",
                count.moved, count.failed
            ),
        },
        TriageResult::Restore(_) => ActionResult::Held {
            kind: FailureKind::Unknown,
            reason: "unexpected restore result".into(),
        },
    }
}

const RECENT_WINDOW: chrono::Duration = chrono::Duration::days(7);

fn classified_candidates(
    records: &[mail_triage::Record],
    account: &str,
    now: DateTime<Utc>,
) -> Vec<String> {
    records
        .iter()
        .filter(|r| {
            r.account == account
                && r.verdict.is_some()
                && r.state != mail_triage::FAILED
                && DateTime::parse_from_rfc3339(&r.classified_at)
                    .is_ok_and(|at| at >= now - RECENT_WINDOW && at <= now)
        })
        .map(|r| r.thread_id.clone())
        .collect()
}

fn after_classify_candidates(
    triage_root: &std::path::Path,
    store: &RuleStore,
    account: &str,
    now: DateTime<Utc>,
) -> Result<Vec<String>> {
    let records = mail_triage::TriageStore::open_existing(triage_root)
        .map(|triage| triage.list())
        .transpose()?
        .unwrap_or_default();
    let mut ids: BTreeSet<_> = classified_candidates(&records, account, now)
        .into_iter()
        .collect();
    ids.extend(
        store
            .deferred()?
            .into_iter()
            .filter(|entry| entry.account == account && !entry.expired(now))
            .map(|entry| entry.thread_id),
    );
    Ok(ids.into_iter().collect())
}

#[async_trait]
impl Mailbox for RuleMailbox {
    async fn candidates(&self, account: &str, pass: Pass) -> Result<Vec<String>> {
        let now = Utc::now();
        if pass == Pass::AfterClassify {
            return after_classify_candidates(
                &self.triage_root,
                &RuleStore::open_existing(&self.rule_root)?,
                account,
                now,
            );
        }
        let a = self.account(account)?;
        let ids = self
            .with_token(a, |t| async move {
                match a.provider {
                    Provider::Google => GmailProvider::new(t).rule_thread_ids(500).await,
                    Provider::Outlook => {
                        OutlookProvider::new(t)
                            .with_immutable_ids()
                            .rule_thread_ids(500, now - RECENT_WINDOW)
                            .await
                    }
                }
            })
            .await?;
        Ok(ids
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect())
    }
    async fn read_thread(&self, account: &str, thread: &str) -> Result<ThreadRead> {
        let Ok(a) = self.account(account) else {
            return Ok(ThreadRead::Held {
                reason: "account unknown".into(),
            });
        };
        let owners = RuleStore::open_existing(&self.rule_root).and_then(|store| {
            owner_addresses(
                &self
                    .tools
                    .accounts()
                    .iter()
                    .map(|a| a.address.clone())
                    .collect::<Vec<_>>(),
                a.address.as_deref(),
                &store,
            )
        });
        if owners.is_err() {
            // Ownership uncertainty is an observation, before credential or
            // provider I/O and before reading any classifier record.
            return candidate_with_owners(account, thread, &[], owners, None);
        }
        let emails = self
            .with_token(a, |t| async move {
                match a.provider {
                    Provider::Google => GmailProvider::new(t).get_thread(thread).await,
                    Provider::Outlook => {
                        OutlookProvider::new(t)
                            .with_immutable_ids()
                            .get_thread(thread)
                            .await
                    }
                }
            })
            .await;
        finish_read(
            account,
            thread,
            emails,
            owners,
            self.record(account, thread),
        )
    }
    async fn mailbox_action(&self, c: &Candidate, action: ActionVerb) -> ActionResult {
        let Ok(account) = self.account(&c.message.account) else {
            return ActionResult::Held {
                kind: FailureKind::Unknown,
                reason: "unknown account".into(),
            };
        };
        match self
            .with_token(account, |token| async move {
                match account.provider {
                    Provider::Google => {
                        gmail_action(&GmailProvider::new(token), &c.message, action).await
                    }
                    Provider::Outlook => {
                        outlook_action(
                            &OutlookProvider::new(token).with_immutable_ids(),
                            &c.message,
                            action,
                        )
                        .await
                    }
                }
            })
            .await
        {
            Ok(result) => triage_result(result),
            Err(error) => failure(error),
        }
    }
    async fn private_record(&self, c: &Candidate, rule: &str, action: &Action) -> ActionResult {
        if !matches!(action.verb, ActionVerb::Tag | ActionVerb::Digest) {
            return ActionResult::Held {
                kind: FailureKind::Unknown,
                reason: "not a private record action".into(),
            };
        }
        match rule_records::update(
            &self.triage_root,
            &c.message.account,
            &c.message.thread_id,
            |r| {
                if action.verb == ActionVerb::Tag {
                    if let Some(tag) = &action.tag {
                        r.tags.insert(tag.clone());
                    }
                } else {
                    r.digest_rules.insert(rule.into());
                }
            },
        ) {
            Ok(()) => ActionResult::Done,
            Err(e) => ActionResult::Failed {
                kind: FailureKind::Environment,
                reason: e.to_string(),
            },
        }
    }
    async fn handled(&self, c: &Candidate) -> Result<()> {
        rule_records::update(
            &self.triage_root,
            &c.message.account,
            &c.message.thread_id,
            |r| r.handled_message = Some(c.message.provider_message_id.clone()),
        )
    }
    fn origin_gate(&self, _rule: &LoadedRule) -> GateStatus {
        // P1's only live installation is owner-authored global files. P7 must
        // replace this with the proposal provenance reader before installing.
        GateStatus::Passing
    }
}

struct PlannedSink;
#[async_trait]
impl TaskSink for PlannedSink {
    async fn create(&self, _: &Task) -> ActionResult {
        ActionResult::Held {
            kind: FailureKind::Environment,
            reason: "dry run: sink not called".into(),
        }
    }
}

fn graph_entry(
    config: &mecha_core::config::Config,
) -> Option<&mecha_core::config::McpServerConfig> {
    config.mcp.iter().find(|m| m.name == "graph" && !m.disabled)
}

struct GraphSink {
    client: Arc<McpClient>,
    create: Arc<dyn Tool>,
    ctx: ToolCtx,
}
impl GraphSink {
    async fn connect(config: &mecha_core::config::Config) -> Result<Self> {
        let entry = graph_entry(config).context("graph MCP is not configured")?;
        let workspace = mecha_core::work::ensure("mail-rules")?;
        Self::connect_at(config, entry, workspace).await
    }

    async fn connect_at(
        config: &mecha_core::config::Config,
        entry: &mecha_core::config::McpServerConfig,
        workspace: PathBuf,
    ) -> Result<Self> {
        let sandbox = mecha_core::sandbox::Sandbox::new(config.sandbox.clone());
        if entry.sandbox {
            sandbox.preflight(&workspace).await?;
        }
        let client = McpClient::connect(entry, &sandbox, &workspace).await?;
        let create = client
            .list_tools()
            .await?
            .into_iter()
            .find(|t| {
                let name = t.name();
                name == "kg_task_create" || name.ends_with("__kg_task_create")
            })
            .context("graph MCP has no kg_task_create")?;
        if create.defers() {
            let _ = client.close().await;
            bail!("graph kg_task_create requires background execution; mail rules require an inline task sink");
        }
        let ctx = ToolCtx {
            workspace,
            security: config.security.clone(),
            ..ToolCtx::default()
        };
        Ok(Self {
            client,
            create,
            ctx,
        })
    }
}
#[async_trait]
impl TaskSink for GraphSink {
    async fn create(&self, task: &Task) -> ActionResult {
        let mut args = serde_json::json!({
            "name": task.name,
            "context": format!("mail: {}/{}", task.account, task.thread_id),
            "captured_from": {"kind": "mail", "account": task.account, "id": task.thread_id, "at": task.received_at.to_rfc3339()}
        });
        if let Some(due) = task.due {
            args["due"] = serde_json::json!(due.to_string());
        }
        match self.create.call(args, &self.ctx).await {
            Ok(output) if output.deferred.is_some() => {
                // The MCP job is lazy: it has not been submitted to a job sink
                // or dispatched. Cancel and drop it; a later pass can retry.
                if let Some(job) = output.deferred {
                    job.cancel_token().cancel();
                }
                ActionResult::Held {
                    kind: FailureKind::Environment,
                    reason: mecha_core::mail_rules::HeldReason::NotDispatched(
                        "graph task tool requires background execution".into(),
                    ),
                }
            }
            Ok(output) if output.not_dispatched => ActionResult::Held {
                // MCP validates this claim only for an operator-trusted server.
                // Nothing ran: release the unused claim and allow a later retry.
                kind: FailureKind::Environment,
                reason: mecha_core::mail_rules::HeldReason::NotDispatched(output.content),
            },
            Ok(output)
                if !output.is_error
                    && output.deferred.is_none()
                    && !output.content.trim().is_empty()
                    && output.content.trim() != "(no content)" =>
            {
                ActionResult::Done
            }
            Ok(output)
                if output.failure_kind == Some(mecha_core::tool::ToolFailureKind::Transport) =>
            {
                // No acknowledgement: preserve the once-per-thread claim and
                // cap reservation. An outage says nothing about task fate.
                ActionResult::Held {
                    kind: FailureKind::Environment,
                    reason: mecha_core::mail_rules::HeldReason::FateUnknown,
                }
            }
            Ok(output) => ActionResult::Failed {
                kind: FailureKind::Unknown,
                reason: if output.content.trim().is_empty()
                    || output.content.trim() == "(no content)"
                {
                    "graph returned no task acknowledgement".into()
                } else {
                    output.content
                },
            },
            Err(error) => ActionResult::Failed {
                kind: FailureKind::Unknown,
                reason: error.to_string(),
            },
        }
    }
}

/// Operator entrypoint. Global config only, and graph absence/failure holds
/// Board actions while leaving mailbox and private annotations available.
pub async fn apply(account: Option<&str>, after_classify: bool, dry_run: bool) -> Result<()> {
    let config = mecha_core::config::Config::load_global()?;
    let root = RuleStore::root_under(&mecha_core::work::mecha_home()?);
    if !root.exists() {
        println!(
            "{}",
            serde_json::to_string_pretty(&engine::ApplyReport::default())?
        );
        return Ok(());
    }
    let store = RuleStore::open_existing(root)?;
    let mailbox = RuleMailbox::load(dry_run, &store)?;
    let accounts = mailbox.accounts(account)?;
    let timezone = config
        .agent
        .timezone
        .as_deref()
        .unwrap_or("UTC")
        .parse()
        .context("invalid global agent timezone")?;
    let approvals = store.approvals()?;
    // This is only a connection optimization. The engine performs the
    // authoritative load and persists lapses for unreadable definitions.
    let connection_rules = store.list().unwrap_or_default();
    let needs_graph = after_classify
        && connection_rules.iter().any(|rule| {
            rule.definition
                .groups()
                .contains(&mecha_core::mail_rules::ActionGroup::Board)
                && mecha_core::mail_rules::resolve_mode(
                    rule,
                    mecha_core::mail_rules::ActionGroup::Board,
                    &approvals,
                    &mecha_core::mail_rules::P1Holds {
                        origin_gate: mailbox.origin_gate(rule),
                    },
                )
                .effective
                    == mecha_core::mail_rules::Mode::Auto
        });
    let mut sink_notice = None;
    // Dry runs never spawn a server (startup itself may write). Report planned
    // tasks only if the configured sink exists, without claiming reachability.
    let graph = if needs_graph && !dry_run {
        match GraphSink::connect(&config).await {
            Ok(sink) => Some(sink),
            Err(e) => {
                sink_notice = Some(format!("task sink unavailable: {e:#}"));
                None
            }
        }
    } else {
        None
    };
    let planned = PlannedSink;
    let sink: Option<&dyn TaskSink> = if dry_run && graph_entry(&config).is_some() {
        Some(&planned)
    } else {
        graph.as_ref().map(|g| g as &dyn TaskSink)
    };
    let result = engine::apply(
        &store,
        &mailbox,
        sink,
        &accounts,
        &engine::EngineOptions {
            pass: if after_classify {
                Pass::AfterClassify
            } else {
                Pass::Headers
            },
            dry_run,
            timezone,
            ..Default::default()
        },
    )
    .await;
    let report = finish_apply(result, sink_notice, async {
        if let Some(graph) = &graph {
            graph.client.close().await?;
        }
        Ok(())
    })
    .await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    report_status(&report)
}

async fn finish_apply(
    result: Result<engine::ApplyReport>,
    sink_notice: Option<String>,
    close: impl std::future::Future<Output = Result<()>>,
) -> Result<engine::ApplyReport> {
    let close_result = close.await;
    let mut report = result?;
    if let Err(error) = close_result {
        report
            .notices
            .push(format!("graph close failed: {error:#}"));
    }
    if let Some(notice) = sink_notice {
        report.notices.push(notice);
    }
    Ok(report)
}

fn report_status(report: &engine::ApplyReport) -> Result<()> {
    if report.read_failures > 0 {
        bail!("some rule threads could not be read; see report");
    }
    if report.handled_failures > 0 {
        bail!("some rule threads could not be marked handled; see report");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
