//! Deterministic rule execution. Provider clients and credentials belong to the
//! caller. One run lock serializes caps, sweeps and effects across accounts;
//! the store's per-action claims remain the authority for idempotence.
use super::*;
use anyhow::{ensure, Result};
use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    Headers,
    AfterClassify,
}

/// Only envelope facts enter the matcher. No body or classifier prose exists
/// in this interface. Unknown arrival or envelope facts cannot authorize effects.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub message: MessageRef,
    /// Provider arrival only; a sender-written Date is never eligibility evidence.
    pub arrival_at: Option<DateTime<Utc>>,
    pub headers_complete: Option<bool>,
    /// Completeness of the subject itself, independent of raw headers.
    pub subject_complete: Option<bool>,
    /// Decode RFC 2047 only when this subject came from a raw header.
    pub subject_is_raw: bool,
    /// Completeness of the union of To, Cc and Bcc.
    pub recipients_complete: Option<bool>,
    pub attachment_presence: Option<bool>,
    pub to: Vec<String>,
    pub list_id: Option<String>,
    pub headers: BTreeMap<String, Vec<String>>,
    pub classified: Option<Classified>,
}

#[derive(Debug, Clone)]
pub struct Classified {
    pub at: DateTime<Utc>,
    pub deadline: Option<NaiveDate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub name: String,
    pub account: String,
    pub thread_id: String,
    pub received_at: DateTime<Utc>,
    pub due: Option<NaiveDate>,
}

/// An observation hold has no authorized message and must consume no occasion.
#[derive(Debug, Clone)]
pub enum ThreadRead {
    Ready(Box<Candidate>),
    Held { reason: String },
}
impl From<Candidate> for ThreadRead {
    fn from(candidate: Candidate) -> Self {
        Self::Ready(Box::new(candidate))
    }
}

/// An acknowledgement is distinct from partial completion or an unknown fate.
#[derive(Debug, Clone)]
pub enum ActionResult {
    Done,
    Failed {
        kind: FailureKind,
        reason: String,
    },
    Held {
        kind: FailureKind,
        reason: HeldReason,
    },
}

#[async_trait]
pub trait Mailbox: Send + Sync {
    async fn candidates(&self, account: &str, pass: Pass) -> Result<Vec<String>>;
    async fn read_thread(&self, account: &str, thread: &str) -> Result<ThreadRead>;
    /// Mutate only `candidate.message.provider_message_id`. Thread-only hosts
    /// must hold unless every affected message independently passed all guards.
    async fn mailbox_action(&self, candidate: &Candidate, action: ActionVerb) -> ActionResult;
    async fn private_record(
        &self,
        candidate: &Candidate,
        rule: &str,
        action: &Action,
    ) -> ActionResult;
    /// Persist a separate, message-specific skip marker, never a fake verdict.
    async fn handled(&self, candidate: &Candidate) -> Result<()>;
    /// No proposal installer exists in P1. Hosts that cannot prove owner files
    /// must leave this Unknown, which holds automatic Board actions.
    fn origin_gate(&self, _rule: &LoadedRule) -> GateStatus {
        GateStatus::Unknown
    }
}

#[async_trait]
pub trait TaskSink: Send + Sync {
    async fn create(&self, task: &Task) -> ActionResult;
}

pub struct EngineOptions {
    pub pass: Pass,
    pub dry_run: bool,
    pub clock: std::sync::Arc<dyn crate::clock::Clock>,
    pub timezone: Tz,
    pub pending_timeout: Duration,
}
impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            pass: Pass::Headers,
            dry_run: false,
            clock: std::sync::Arc::new(crate::clock::SystemClock),
            timezone: chrono_tz::UTC,
            pending_timeout: Duration::minutes(15),
        }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct ApplyReport {
    pub rows: Vec<LedgerRow>,
    pub notices: Vec<String>,
    pub deferred: Vec<String>,
    pub handled: Vec<String>,
    /// Evaluation holds do not close action keys.
    pub list_cap: Vec<String>,
}

/// Keys AND; values OR. Addresses/domains/accounts/List-Id compare exactly
/// ignoring ASCII case; subject/header values use case-insensitive contains.
/// List-Id's optional display phrase and angle brackets are not its identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderMatch {
    Match,
    NoMatch,
    Unknown,
}

/// A boolean convenience for callers that only accept a proven match.
pub fn matches_headers(m: &Matcher, c: &Candidate) -> bool {
    header_match(m, c) == HeaderMatch::Match
}

/// Missing or truncated facts cannot prove a negative. A known negative key
/// still disproves the conjunction even if another key is unknown.
pub fn header_match(m: &Matcher, c: &Candidate) -> HeaderMatch {
    fn compare(wanted: &[String], actual: &[&str], complete: bool, substring: bool) -> HeaderMatch {
        if wanted.is_empty()
            || wanted.iter().any(|w| {
                actual.iter().any(|a| {
                    if substring {
                        a.to_lowercase().contains(&w.to_lowercase())
                    } else {
                        w.eq_ignore_ascii_case(a)
                    }
                })
            })
        {
            HeaderMatch::Match
        } else if complete {
            HeaderMatch::NoMatch
        } else {
            HeaderMatch::Unknown
        }
    }
    let complete = c.headers_complete == Some(true);
    let sender = parsed_address(&c.message.sender);
    let domain = sender
        .as_deref()
        .and_then(|s| s.rsplit_once('@').map(|(_, d)| d));
    let list_unique = c
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("list-id"))
        .map(|(_, values)| values.len())
        .sum::<usize>()
        <= 1;
    let list = c
        .list_id
        .as_deref()
        .filter(|_| list_unique)
        .and_then(headers::list_id);
    let list_known = list_unique && (c.list_id.is_none() || list.is_some());
    let subject_unique = c
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("subject"))
        .map(|(_, values)| values.len())
        .sum::<usize>()
        <= 1;
    let subject = if c.subject_is_raw {
        headers::decode(&c.message.subject)
    } else {
        headers::unfold(&c.message.subject)
    }
    .filter(|_| subject_unique && c.subject_complete != Some(false));
    let mut keys = vec![
        compare(
            &m.from,
            &sender.as_deref().into_iter().collect::<Vec<_>>(),
            sender.is_some(),
            false,
        ),
        compare(
            &m.from_domain,
            &domain.into_iter().collect::<Vec<_>>(),
            domain.is_some(),
            false,
        ),
        compare(
            &m.to,
            &c.to.iter().map(String::as_str).collect::<Vec<_>>(),
            c.recipients_complete == Some(true),
            false,
        ),
        compare(&m.account, &[&c.message.account], true, false),
        compare(
            &m.list_id,
            &list.as_deref().into_iter().collect::<Vec<_>>(),
            complete && list_known,
            false,
        ),
        compare(
            &m.subject_contains,
            &subject.as_deref().into_iter().collect::<Vec<_>>(),
            c.subject_complete == Some(true) && subject.is_some(),
            true,
        ),
        match (m.has_attachment, c.attachment_presence) {
            (None, _) => HeaderMatch::Match,
            (Some(_), None) => HeaderMatch::Unknown,
            (Some(wanted), Some(actual)) if wanted == actual => HeaderMatch::Match,
            _ => HeaderMatch::NoMatch,
        },
    ];
    for (key, wanted) in &m.header {
        let values: Option<Vec<_>> = c
            .headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(key))
            .flat_map(|(_, values)| values.iter().map(|raw| headers::decode(raw)))
            .collect();
        keys.push(if wanted.is_empty() {
            HeaderMatch::NoMatch
        } else if key.eq_ignore_ascii_case("subject")
            && (!subject_unique || c.subject_complete == Some(false))
        {
            HeaderMatch::Unknown
        } else if let Some(values) = values {
            compare(
                wanted,
                &values.iter().map(String::as_str).collect::<Vec<_>>(),
                complete,
                true,
            )
        } else {
            HeaderMatch::Unknown
        });
    }
    if keys.contains(&HeaderMatch::NoMatch) {
        HeaderMatch::NoMatch
    } else if keys.contains(&HeaderMatch::Unknown) {
        HeaderMatch::Unknown
    } else {
        HeaderMatch::Match
    }
}

fn classified(c: &Candidate) -> bool {
    c.arrival_at
        .is_some_and(|at| c.classified.as_ref().is_some_and(|v| v.at >= at))
}

// Deliberately accepts a conservative addr-spec subset, never a display name,
// whitespace or syntax that could turn into instructions in a task's name.
fn parsed_address(s: &str) -> Option<String> {
    let (local, domain) = s.split_once('@')?;
    if local.is_empty()
        || domain.is_empty()
        || s.len() > 254
        || !local
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".!#$%&'*+-/=?^_`{|}~".contains(&b))
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
        || !domain.split('.').all(|part| {
            !part.is_empty()
                && !part.starts_with('-')
                && !part.ends_with('-')
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return None;
    }
    Some(s.to_ascii_lowercase())
}

fn task(rule: &LoadedRule, c: &Candidate) -> Task {
    let due = c.classified.as_ref().and_then(|v| v.deadline);
    let mut name = rule.definition.description.clone();
    if let Some(sender) = parsed_address(&c.message.sender) {
        name.push_str(&format!(" — {sender}"));
    }
    name.push_str(&format!(
        " — {}",
        c.arrival_at.expect("eligible arrival").date_naive()
    ));
    if let Some(due) = due {
        name.push_str(&format!(" — due {due}"));
    }
    Task {
        name,
        account: c.message.account.clone(),
        thread_id: c.message.thread_id.clone(),
        received_at: c.arrival_at.expect("eligible arrival"),
        due,
    }
}

/// Count distinct executed actions/matches, not pending+settlement+correction
/// rows. Outages are ignored, not successes that reset an action streak.
pub fn brake_evidence(rule: &str, rows: &[LedgerRow]) -> BrakeEvidence {
    let mut evidence = BrakeEvidence::default();
    let mut mailbox = BTreeSet::new();
    let mut undone = BTreeSet::new();
    let mut matches = BTreeSet::new();
    let mut wrong = BTreeSet::new();
    for row in rows
        .iter()
        .filter(|r| r.rule == rule && r.outcome != Outcome::ApprovalLapsed)
    {
        let message = (
            row.message.account.clone(),
            row.message.thread_id.clone(),
            row.message.provider_message_id.clone(),
        );
        let action = (message.clone(), row.action.digest.clone());
        let thread = (row.message.account.clone(), row.message.thread_id.clone());
        if !row.matched_because.is_empty() {
            matches.insert(thread.clone());
        }
        if row.wrong_match {
            wrong.insert(thread);
        }
        if row.group == ActionGroup::Mailbox
            && row.mode == Mode::Auto
            && row.outcome == Outcome::Done
        {
            mailbox.insert(action.clone());
        }
        if row.outcome == Outcome::Undone {
            undone.insert(action);
        }
        if row.group.is_outbound() && row.mode == Mode::Auto && row.wrong_match {
            evidence.wrong_automatic_sends.insert(row.group);
        }
        let streak = evidence
            .consecutive_action_failures
            .entry(row.group)
            .or_default();
        if row.is_action_failure() {
            *streak += 1;
        } else if matches!(row.outcome, Outcome::Done | Outcome::Sent) {
            *streak = 0;
        }
    }
    evidence.automatic_mailbox = mailbox.len();
    evidence.undone_mailbox = mailbox.intersection(&undone).count();
    evidence.matches = matches.len();
    evidence.wrong_matches = matches.intersection(&wrong).count();
    evidence
}

struct Run<'a> {
    store: &'a RuleStore,
    mailbox: &'a dyn Mailbox,
    sink: Option<&'a dyn TaskSink>,
    options: &'a EngineOptions,
    rows: Vec<LedgerRow>,
    report: ApplyReport,
}
impl Run<'_> {
    fn write(&mut self, row: LedgerRow) -> Result<()> {
        if !self.options.dry_run {
            self.store.append_ledger(&row)?;
        }
        self.rows.push(row.clone());
        self.report.rows.push(row);
        Ok(())
    }
    fn modes(&mut self, rule: &LoadedRule) -> Result<BTreeMap<ActionGroup, ResolvedMode>> {
        if !self.options.dry_run {
            self.rows = self.store.ledger()?;
        }
        let holds = P1Holds {
            origin_gate: self.mailbox.origin_gate(rule),
        };
        let approvals = self.store.approvals()?;
        let mut targets = brake_targets(
            rule.definition.groups(),
            &brake_evidence(&rule.name, &self.rows),
        );
        // An owner raise acknowledges prior evidence. A brake needs a new
        // adverse event after that approval, or recovery would be impossible:
        // shadow cannot produce the success needed to reset its own streak.
        targets.retain(|group, _| {
            let acknowledged = approvals
                .iter()
                .rev()
                .find(|r| {
                    r.rule == rule.name
                        && r.group == *group
                        && (r.reason == ApprovalReason::Brake
                            || r.actor == crate::closure::Actor::Owner)
                })
                .and_then(|r| r.at);
            self.rows.iter().any(|r| {
                r.rule == rule.name
                    && (r.wrong_match
                        || (r.group == *group
                            && (r.is_action_failure() || r.outcome == Outcome::Undone)))
                    && acknowledged.is_none_or(|at| r.at.is_none_or(|event| event > at))
            })
        });
        if !self.options.dry_run {
            let lapses = self
                .store
                .detect_lapses(&rule.name, &holds, self.options.clock.now())?;
            if !lapses.is_empty() {
                self.report
                    .notices
                    .push(format!("{}: approval lapsed: files changed", rule.name));
            }
            if self
                .store
                .lower_modes(&rule.name, &targets, None, &holds, ApprovalReason::Brake)?
                .iter()
                .any(|d| d.write)
            {
                self.report
                    .notices
                    .push(format!("{}: automatic brake", rule.name));
            }
        }
        let approvals = self.store.approvals()?;
        Ok(rule
            .definition
            .groups()
            .into_iter()
            .map(|g| {
                let mut mode = resolve_mode(rule, g, &approvals, &holds);
                if let Some(target) = targets.get(&g) {
                    mode.effective = mode.effective.lower(*target);
                }
                (g, mode)
            })
            .collect())
    }
    fn cap(
        &self,
        rule: &LoadedRule,
        reservations: &[LedgerRow],
        provisional: &BTreeSet<String>,
        at: DateTime<Utc>,
    ) -> Result<Option<&'static str>> {
        let day = at.with_timezone(&self.options.timezone).date_naive();
        let mut all = BTreeSet::new();
        let mut own = BTreeSet::new();
        for r in self.rows.iter().chain(reservations) {
            if r.mode != Mode::Auto || !r.group.once_per_thread() || provisional.contains(&r.id) {
                continue;
            }
            let in_flight =
                r.outcome == Outcome::Pending && !self.rows.iter().any(|s| s.settles(r));
            // An unsettled attempt has unknown fate even across midnight. Keep
            // reserving capacity until a settlement supplies its attempt date.
            let attempted = matches!(
                r.outcome,
                Outcome::Done | Outcome::Sent | Outcome::Failed | Outcome::Unknown
            ) || (r.outcome == Outcome::Held
                && HeldReason::from_ledger(&r.reason) == HeldReason::FateUnknown);
            if !in_flight
                && !(attempted
                    && r.at.is_some_and(|at| {
                        at.with_timezone(&self.options.timezone).date_naive() == day
                    }))
            {
                continue;
            }
            let key = (
                &r.rule,
                &r.message.account,
                &r.message.provider_message_id,
                &r.action.digest,
            );
            all.insert(key);
            if r.rule == rule.name {
                own.insert(key);
            }
        }
        if own.len() >= rule.definition.daily_cap as usize {
            Ok(Some("rule cap"))
        } else if all.len() >= self.store.limits()?.daily_cap as usize {
            Ok(Some("global cap"))
        } else {
            Ok(None)
        }
    }
    fn closed(&self, row: &LedgerRow) -> ClaimStatus {
        if row.group.once_per_thread()
            && thread_closed(
                &self.rows,
                &ThreadKey {
                    rule: row.rule.clone(),
                    account: row.message.account.clone(),
                    thread_id: row.message.thread_id.clone(),
                },
            )
        {
            ClaimStatus::ThreadClosed
        } else if message_closed(
            &self.rows,
            &MessageKey {
                rule: row.rule.clone(),
                account: row.message.account.clone(),
                provider_message_id: row.message.provider_message_id.clone(),
                action: row.action.clone(),
            },
        ) {
            ClaimStatus::MessageClosed
        } else {
            ClaimStatus::Claimed
        }
    }
    fn snapshot(
        &self,
        rule: &LoadedRule,
        index: usize,
        c: &Candidate,
        mode: Mode,
    ) -> Result<LedgerRow> {
        let mut row = LedgerRow::header(
            rule,
            index,
            c.message.clone(),
            mode,
            self.mailbox.origin_gate(rule),
            Outcome::WouldHave,
            self.options.clock.now(),
        )?;
        row.matched_because = serde_json::to_string(&rule.definition.matcher)?;
        Ok(row)
    }
    async fn apply_thread(&mut self, rules: &[LoadedRule], c: &Candidate) -> Result<()> {
        let mut modes = BTreeMap::new();
        for rule in rules {
            modes.insert(rule.name.clone(), self.modes(rule)?);
        }
        let held = list_cap(rules, &modes);
        self.report.list_cap.extend(held.iter().cloned());
        let descriptions = rules.iter().any(|r| {
            r.definition.is_description()
                && modes[&r.name].values().any(|m| m.effective != Mode::Off)
        });
        let matched: Vec<_> = rules
            .iter()
            .filter(|r| {
                !r.definition.is_description()
                    && !held.contains(&r.name)
                    && header_match(&r.definition.matcher, c) != HeaderMatch::NoMatch
            })
            .collect();
        let owes_verdict = matched.iter().any(|r| {
            r.definition.actions.iter().any(|a| {
                modes[&r.name][&a.group()].effective != Mode::Off
                    && (a.group().is_outbound()
                        || matches!(a.verb, ActionVerb::Digest | ActionVerb::Task))
            })
        });
        let unknown_deferral = matched.iter().any(|r| {
            header_match(&r.definition.matcher, c) == HeaderMatch::Unknown
                && r.definition.actions.iter().any(|a| {
                    modes[&r.name][&a.group()].effective != Mode::Off
                        && (a.group().is_outbound()
                            || matches!(a.verb, ActionVerb::Digest | ActionVerb::Task))
                })
        });
        let defer_removal = descriptions || owes_verdict;
        let outbound = outbound_owner(&matched, &modes);
        let classified = classified(c);
        let mut deferred = Vec::new();
        for rule in &matched {
            for (index, action) in rule.definition.actions.iter().enumerate() {
                let mode = &modes[&rule.name][&action.group()];
                let waits = action.group().is_outbound()
                    || matches!(action.verb, ActionVerb::Digest | ActionVerb::Task)
                    || (defer_removal
                        && matches!(
                            action.verb,
                            ActionVerb::Archive | ActionVerb::Trash | ActionVerb::Spam
                        ));
                if mode.effective == Mode::Off
                    || !waits
                    || (self.options.pass == Pass::AfterClassify
                        && classified
                        && header_match(&rule.definition.matcher, c) == HeaderMatch::Match
                        && !unknown_deferral
                        && !(mode.effective == Mode::Auto
                            && action.verb == ActionVerb::Task
                            && self.sink.is_none()))
                    || (mode.effective.rank() >= Mode::Propose.rank()
                        && mode
                            .activated_at
                            .is_some_and(|at| c.arrival_at.is_some_and(|arrival| arrival <= at)))
                {
                    continue;
                }
                let row = self.snapshot(rule, index, c, mode.effective)?;
                if self.closed(&row) == ClaimStatus::Claimed {
                    deferred.push(row);
                }
            }
        }
        if !deferred.is_empty() {
            if !self.options.dry_run {
                self.store.enqueue(&DeferredEntry {
                    id: uuid::Uuid::new_v4().to_string(),
                    account: c.message.account.clone(),
                    thread_id: c.message.thread_id.clone(),
                    queued_at: self.options.clock.now(),
                    actions: deferred,
                })?;
            }
            self.report.deferred.push(c.message.thread_id.clone());
        }
        let mut handled = false;
        for rule in matched {
            let current = self.modes(rule)?;
            let mut pending = Vec::new();
            let mut planned = Vec::new();
            let mut reservations = Vec::new();
            let mut rule_handled = current
                .get(&ActionGroup::Mailbox)
                .is_some_and(|m| m.effective == Mode::Auto)
                && c.arrival_at.is_some()
                && header_match(&rule.definition.matcher, c) == HeaderMatch::Match
                && !defer_removal
                && rule.definition.actions.iter().all(|a| {
                    current[&a.group()].effective == Mode::Off
                        || matches!(
                            a.verb,
                            ActionVerb::Archive
                                | ActionVerb::Trash
                                | ActionVerb::Read
                                | ActionVerb::Unread
                                | ActionVerb::Tag
                        )
                });
            for (index, action) in rule.definition.actions.iter().enumerate() {
                let resolved = &current[&action.group()];
                let mode = resolved.effective;
                if mode == Mode::Off {
                    continue;
                }
                let mut row = self.snapshot(rule, index, c, mode)?;
                let unknown = header_match(&rule.definition.matcher, c) == HeaderMatch::Unknown;
                let removal = matches!(
                    action.verb,
                    ActionVerb::Archive | ActionVerb::Trash | ActionVerb::Spam
                );
                if unknown
                    || parsed_address(&c.message.sender).is_none()
                    || c.arrival_at.is_none()
                    || (removal && unknown_deferral)
                {
                    // An observation hold reserves no action key. A later read
                    // may supply the missing fact; deferred work stays queued.
                    row.outcome = Outcome::Held;
                    row.closes = false;
                    row.reason = if parsed_address(&c.message.sender).is_none() {
                        "sender unknown"
                    } else if c.arrival_at.is_none() {
                        "provider arrival unknown"
                    } else {
                        "matching or deferral facts unknown"
                    }
                    .into();
                    self.report.rows.push(row);
                    rule_handled = false;
                    continue;
                }
                if mode.rank() >= Mode::Propose.rank()
                    && resolved
                        .activated_at
                        .is_none_or(|at| c.arrival_at.is_some_and(|arrival| arrival <= at))
                {
                    row.outcome = Outcome::Skipped;
                    row.reason = "before the rule was active".into();
                    self.write(row)?;
                    rule_handled = false;
                    continue;
                }
                let waits = action.group().is_outbound()
                    || matches!(action.verb, ActionVerb::Digest | ActionVerb::Task)
                    || (defer_removal
                        && matches!(
                            action.verb,
                            ActionVerb::Archive | ActionVerb::Trash | ActionVerb::Spam
                        ));
                if waits && (self.options.pass == Pass::Headers || !classified) {
                    rule_handled = false;
                    continue;
                }
                if self.options.pass == Pass::AfterClassify && !classified {
                    continue;
                }
                if mode == Mode::Shadow {
                    self.write(row)?;
                    rule_handled = false;
                    continue;
                }
                if action.group().is_outbound() && outbound.as_deref() != Some(&rule.name) {
                    row.outcome = Outcome::Skipped;
                    row.reason = format!("superseded by {}", outbound.as_deref().unwrap_or("none"));
                    self.write(row)?;
                    continue;
                }
                if mode == Mode::Auto && action.verb == ActionVerb::Task && self.sink.is_none() {
                    // No dispatch was possible: this environmental observation
                    // consumes neither a claim nor capacity. Keep the queue.
                    row.outcome = Outcome::Held;
                    row.closes = false;
                    row.reason = "task sink unavailable".into();
                    row.failure_kind = FailureKind::Environment;
                    self.report.rows.push(row);
                    rule_handled = false;
                    continue;
                }
                row.outcome = Outcome::Pending;
                row.closes = true;
                // Reserve cumulatively within this batch, including siblings.
                let cap = if mode == Mode::Auto && action.group().once_per_thread() {
                    self.cap(
                        rule,
                        &reservations,
                        &BTreeSet::new(),
                        row.at.expect("reservation timestamp"),
                    )?
                } else {
                    None
                };
                if cap.is_none() && mode == Mode::Auto && action.group().once_per_thread() {
                    reservations.push(row.clone());
                }
                pending.push(row);
                planned.push((index, cap));
            }
            if !pending.is_empty() {
                let claims = if self.options.dry_run {
                    pending
                        .iter()
                        .map(|r| ActionClaim {
                            pending_id: r.id.clone(),
                            action: r.action.clone(),
                            status: self.closed(r),
                        })
                        .collect()
                } else {
                    self.store.claim_actions(&pending)?
                };
                let mut provisional: BTreeSet<_> = pending.iter().map(|r| r.id.clone()).collect();
                for ((row, claim), (index, reserved_cap)) in
                    pending.into_iter().zip(claims).zip(planned)
                {
                    if claim.status != ClaimStatus::Claimed {
                        // A previous failure/pending/offer cannot certify a skip.
                        rule_handled &= self
                            .rows
                            .iter()
                            .rev()
                            .find(|r| {
                                r.same_key(&row)
                                    && !matches!(
                                        r.outcome,
                                        Outcome::Skipped
                                            | Outcome::WouldHave
                                            | Outcome::Expired
                                            | Outcome::Pending
                                    )
                            })
                            .is_some_and(|r| r.outcome == Outcome::Done);
                        let mut skipped = row;
                        skipped.outcome = Outcome::Skipped;
                        skipped.closes = false;
                        skipped.reason = if claim.status == ClaimStatus::ThreadClosed {
                            "thread already handled"
                        } else {
                            "action already handled"
                        }
                        .into();
                        self.write(skipped)?;
                        continue;
                    }
                    self.rows.push(row.clone());
                    let action = &rule.definition.actions[index];
                    // A preceding sibling can await I/O while the owner pauses
                    // and reactivates a group, or can trip its failure brake.
                    let resolved = self.modes(rule)?[&action.group()].clone();
                    let mode = resolved.effective;
                    let inputs_changed = self
                        .store
                        .get(&rule.name)
                        .map_or(true, |fresh| fresh.approval_digest != rule.approval_digest);
                    // Recheck at dispatch: provider reads and earlier actions
                    // can cross the local day boundary after reservation.
                    let attempted_at = self.options.clock.now();
                    let cap = if mode == Mode::Auto && action.group().once_per_thread() {
                        let same_day = row.at.is_some_and(|at| {
                            at.with_timezone(&self.options.timezone).date_naive()
                                == attempted_at
                                    .with_timezone(&self.options.timezone)
                                    .date_naive()
                        });
                        let reserved = if same_day { reserved_cap } else { None };
                        reserved.or(self.cap(rule, &[], &provisional, attempted_at)?)
                    } else {
                        None
                    };
                    let (outcome, reason, failure) = if inputs_changed {
                        (
                            Outcome::Skipped,
                            "rule inputs changed during apply".into(),
                            FailureKind::Unknown,
                        )
                    } else if mode == Mode::Off {
                        (
                            Outcome::Skipped,
                            "group disabled during apply".into(),
                            FailureKind::Unknown,
                        )
                    } else if mode.rank() >= Mode::Propose.rank()
                        && resolved
                            .activated_at
                            .is_none_or(|at| c.arrival_at.is_none_or(|arrival| arrival <= at))
                    {
                        (
                            Outcome::Skipped,
                            "before the rule was active".into(),
                            FailureKind::Unknown,
                        )
                    } else if mode == Mode::Shadow {
                        (
                            Outcome::WouldHave,
                            "automatic brake".into(),
                            FailureKind::Unknown,
                        )
                    } else if mode == Mode::Propose {
                        (Outcome::Offered, String::new(), FailureKind::Unknown)
                    } else if let Some(cap) = cap {
                        self.report.notices.push(cap.into());
                        (Outcome::Held, cap.into(), FailureKind::Unknown)
                    } else if self.options.dry_run {
                        (
                            Outcome::Done,
                            "dry run: planned".into(),
                            FailureKind::Unknown,
                        )
                    } else {
                        let effect = async {
                            match action.group() {
                                ActionGroup::Mailbox => {
                                    self.mailbox.mailbox_action(c, action.verb).await
                                }
                                ActionGroup::PrivateRecord => {
                                    self.mailbox.private_record(c, &rule.name, action).await
                                }
                                // The sink is checked before the cap at
                                // planning; hold here, never panic, if that
                                // order ever moves.
                                ActionGroup::Board => match self.sink {
                                    Some(sink) => sink.create(&task(rule, c)).await,
                                    None => ActionResult::Held {
                                        kind: FailureKind::Environment,
                                        reason: "task sink unavailable".into(),
                                    },
                                },
                                _ => ActionResult::Held {
                                    kind: FailureKind::Unknown,
                                    reason: "unsupported action".into(),
                                },
                            }
                        };
                        let result =
                            tokio::time::timeout(std::time::Duration::from_secs(120), effect)
                                .await
                                .unwrap_or_else(|_| {
                                    if action.group().once_per_thread() {
                                        ActionResult::Held {
                                            kind: FailureKind::Interrupted,
                                            reason: HeldReason::FateUnknown,
                                        }
                                    } else {
                                        ActionResult::Failed {
                                            kind: FailureKind::Interrupted,
                                            reason: "interrupted: action timeout".into(),
                                        }
                                    }
                                });
                        match result {
                            ActionResult::Done => {
                                (Outcome::Done, String::new(), FailureKind::Unknown)
                            }
                            ActionResult::Failed { kind, reason } => {
                                (Outcome::Failed, reason, kind)
                            }
                            ActionResult::Held { kind, reason } => {
                                (Outcome::Held, reason.to_string(), kind)
                            }
                        }
                    };
                    rule_handled &= outcome == Outcome::Done;
                    let mut settled = row.settled(outcome, reason, attempted_at)?;
                    provisional.remove(&row.id);
                    settled.mode = mode;
                    settled.failure_kind = failure;
                    self.write(settled)?;
                }
                self.modes(rule)?;
            }
            handled |= rule_handled;
        }
        if handled && self.options.pass == Pass::Headers {
            if !self.options.dry_run {
                self.mailbox.handled(c).await?;
            }
            self.report.handled.push(c.message.thread_id.clone());
        }
        Ok(())
    }
}

// Matching input is already in priority/name order. Shadow and off groups
// do not reserve an outbound occasion, nor do mailbox or Board siblings.
fn outbound_owner(
    matched: &[&LoadedRule],
    modes: &BTreeMap<String, BTreeMap<ActionGroup, ResolvedMode>>,
) -> Option<String> {
    matched
        .iter()
        .find(|rule| {
            rule.definition.actions.iter().any(|action| {
                action.group().is_outbound()
                    && modes[&rule.name][&action.group()].effective.rank() >= Mode::Propose.rank()
            })
        })
        .map(|rule| rule.name.clone())
}

// The loader currently admits no description execution. Still expose the
// specified capacity hold so P4 cannot silently inherit an unbounded frame.
fn list_cap(
    rules: &[LoadedRule],
    modes: &BTreeMap<String, BTreeMap<ActionGroup, ResolvedMode>>,
) -> BTreeSet<String> {
    let mut entries: Vec<_> = rules
        .iter()
        .filter(|r| r.definition.is_description())
        .map(|r| {
            let ms = &modes[&r.name];
            let shadow = ms.values().any(|m| m.effective == Mode::Shadow);
            let active = ms
                .values()
                .any(|m| m.effective.rank() >= Mode::Propose.rank());
            (
                ms.values()
                    .map(|m| m.effective.rank())
                    .max()
                    .unwrap_or_default(),
                r.definition.priority,
                &r.name,
                usize::from(shadow) + usize::from(active),
            )
        })
        .collect();
    entries.sort();
    let mut total: usize = entries.iter().map(|e| e.3).sum();
    let mut held = BTreeSet::new();
    for (_, _, name, cost) in entries {
        if total <= 12 {
            break;
        }
        if cost > 0 {
            held.insert(name.clone());
            total -= cost;
        }
    }
    held
}

/// Apply all accounts under one engine lease. Dry runs read only and return
/// the same plan without claiming, sweeping, lowering, enqueuing or effects.
pub async fn apply(
    store: &RuleStore,
    mailbox: &dyn Mailbox,
    sink: Option<&dyn TaskSink>,
    accounts: &[String],
    options: &EngineOptions,
) -> Result<ApplyReport> {
    ensure!(
        options.pending_timeout > Duration::zero(),
        "pending timeout must be positive"
    );
    let _lease = if options.dry_run {
        None
    } else {
        Some(store.engine_lease()?)
    };
    if !options.dry_run {
        // Detect broken/deleted definitions before list() can abort. Restoring
        // their bytes must not restore an old auto approval.
        for name in store
            .approvals()?
            .into_iter()
            .map(|r| r.rule)
            .collect::<BTreeSet<_>>()
        {
            let gate = store
                .get(&name)
                .map(|r| mailbox.origin_gate(&r))
                .unwrap_or_default();
            store.detect_lapses(&name, &P1Holds { origin_gate: gate }, options.clock.now())?;
        }
    }
    let rules = store.list()?;
    let mut run = Run {
        store,
        mailbox,
        sink,
        options,
        rows: store.ledger()?,
        report: ApplyReport::default(),
    };
    for row in run.rows.clone() {
        if !run.rows.iter().any(|s| s.settles(&row)) {
            if let Some(settled) = row.interrupted(options.clock.now(), options.pending_timeout)? {
                run.report
                    .notices
                    .push(format!("{}: {}", row.rule, settled.reason));
                run.write(settled)?;
            }
        }
    }
    if !options.dry_run {
        for expired in store.expire_deferred(options.clock.now())? {
            run.report
                .notices
                .push(format!("{}: deferred work expired", expired.thread_id));
        }
        run.rows = store.ledger()?;
    }
    // Also resolve brakes and capacity when no mail is returned.
    let mut modes = BTreeMap::new();
    for rule in &rules {
        modes.insert(rule.name.clone(), run.modes(rule)?);
    }
    run.report.list_cap.extend(list_cap(&rules, &modes));
    let queue = store.deferred()?;
    if options.dry_run {
        for entry in queue
            .iter()
            .filter(|entry| entry.expired(options.clock.now()))
        {
            run.report
                .notices
                .push(format!("{}: deferred work expired", entry.thread_id));
            for snapshot in &entry.actions {
                let mut row = snapshot.clone();
                row.id = format!("expired:{}:{}", entry.id, snapshot.id);
                row.at = Some(options.clock.now());
                row.outcome = Outcome::Expired;
                row.closes = true;
                row.reason = "deferred for more than seven days".into();
                run.write(row)?;
            }
        }
    }
    for account in accounts {
        let mut ids: BTreeSet<_> = mailbox
            .candidates(account, options.pass)
            .await?
            .into_iter()
            .collect();
        if options.pass == Pass::AfterClassify {
            ids.extend(
                queue
                    .iter()
                    .filter(|e| e.account == *account && !e.expired(options.clock.now()))
                    .map(|e| e.thread_id.clone()),
            );
        }
        for id in ids {
            let entries: Vec<_> = queue
                .iter()
                .filter(|e| {
                    e.account == *account && e.thread_id == id && !e.expired(options.clock.now())
                })
                .collect();
            let read = match mailbox.read_thread(account, &id).await {
                Ok(ThreadRead::Ready(c)) => Ok(*c),
                Ok(ThreadRead::Held { reason }) => {
                    run.report
                        .notices
                        .push(format!("{account}/{id}: held: {reason}"));
                    // No message was authorized. Keep every deferred occasion
                    // and cap slot open; the report is an observation only.
                    for snapshot in entries.iter().flat_map(|entry| &entry.actions) {
                        let mut row = snapshot.clone();
                        row.id = uuid::Uuid::new_v4().to_string();
                        row.at = Some(options.clock.now());
                        row.outcome = Outcome::Held;
                        row.closes = false;
                        row.reason = reason.clone();
                        run.report.rows.push(row);
                    }
                    continue;
                }
                Err(error) => Err(error),
            }
            .and_then(|c| {
                ensure!(
                    c.message.account == *account
                        && c.message.thread_id == id
                        && !c.message.provider_message_id.is_empty(),
                    "invalid candidate identity"
                );
                Ok(c)
            });
            let classified = read.as_ref().is_ok_and(classified);
            if let Ok(c) = &read {
                run.apply_thread(&rules, c).await?;
            } else {
                run.report
                    .notices
                    .push(format!("{account}/{id}: thread unreadable"));
            }
            if options.pass != Pass::AfterClassify || (read.is_ok() && !classified) {
                continue;
            }
            for entry in entries {
                let mut outcomes = Vec::new();
                for snapshot in &entry.actions {
                    if let Some(row) = run.report.rows.iter().rev().find(|r| {
                        read.is_ok()
                            && r.same_key(snapshot)
                            && !matches!(r.outcome, Outcome::Pending | Outcome::ApprovalLapsed)
                    }) {
                        if row.outcome == Outcome::Held && !row.closes {
                            continue; // unknown facts are an observation, not a settled action
                        }
                        outcomes.push(row.clone());
                    } else {
                        let mut row = snapshot.clone();
                        row.id = uuid::Uuid::new_v4().to_string();
                        row.at = Some(options.clock.now());
                        row.outcome = if read.is_err() {
                            Outcome::Failed
                        } else {
                            Outcome::Skipped
                        };
                        row.closes = row.outcome.closes(row.group);
                        row.reason = if read.is_err() {
                            "thread unreadable"
                        } else {
                            "deferred action no longer matches the current rule/message"
                        }
                        .into();
                        row.failure_kind = FailureKind::Environment;
                        // No unknown fate: this snapshot was never dispatched.
                        outcomes.push(row);
                    }
                }
                if !options.dry_run && !outcomes.is_empty() {
                    store.settle_deferred(&entry.id, &outcomes)?;
                }
                for row in outcomes {
                    if !run.rows.iter().any(|r| r.id == row.id) {
                        run.report.rows.push(row.clone());
                        run.rows.push(row);
                    }
                }
            }
        }
    }
    run.report.list_cap.sort();
    run.report.list_cap.dedup();
    for name in &run.report.list_cap {
        run.report.notices.push(format!("{name}: held: list cap (12 entries); reduce description list membership through an owner mode change"));
    }
    run.report.notices.sort();
    run.report.notices.dedup();
    Ok(run.report)
}

#[cfg(test)]
mod tests;
