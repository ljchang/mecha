//! Deterministic rule execution. Provider clients and credentials belong to the
//! caller. One run lock serializes caps, sweeps and effects across accounts;
//! the store's per-action claims remain the authority for idempotence.
use super::*;
use anyhow::{ensure, Result};
use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// A description rule's grounded match on one message, as the engine reads
/// it from the stored verdict.
#[derive(Debug, Clone)]
struct Described {
    /// The quote, when the active list named the rule.
    active: Option<String>,
    /// The quote, when the shadow list named it.
    shadow: Option<String>,
    input: VerdictInput,
    /// The active confirmation chose this rule under this build.
    confirmed: bool,
    /// The shadow confirmation's verdict on this rule, when it gave one.
    shadow_confirmed: Option<bool>,
    /// What the active confirmation chose, for the row: a name, "none", or
    /// "not confirmed" when it decided nothing.
    confirmation_choice: Option<String>,
}

impl Described {
    /// Shadow rows use the shadow quote when there is one (§4.2); active rows
    /// only ever reach here with an active quote.
    fn quote(&self, mode: Mode) -> &str {
        if mode == Mode::Shadow {
            self.shadow.as_deref().or(self.active.as_deref())
        } else {
            self.active.as_deref()
        }
        .unwrap_or_default()
    }
}

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
    /// What a reply's guards read (§5.4). Empty by default: a host that
    /// cannot supply them gets no reply staged on their absence alone, since
    /// each guard keys on a fact being present.
    pub sending: SendFacts,
}

/// The facts a send's guards read that header matching does not.
#[derive(Debug, Clone, Default)]
pub struct SendFacts {
    /// The `From` display name: prose, the sender's, for `{sender_first_name}`.
    pub from_name: String,
    /// Whether the owner has sent a message in the thread; None is unknown.
    pub owner_participated: Option<bool>,
    /// Every address the owner sends from: each account's own, plus aliases.
    pub owners: Vec<String>,
}

/// Where a staged send stands in the outbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StagedFate {
    Waiting,
    ReleasedUnchanged,
    Edited,
    Rejected,
    /// The item is gone, or unreadable: nothing is settled from it.
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Classified {
    pub at: DateTime<Utc>,
    pub deadline: Option<NaiveDate>,
    /// The description-rule verdict stored with this classification. Absent
    /// means no description rule was asked about this message.
    pub rules: Option<crate::mail_triage::rule_frame::RuleRecord>,
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

fn activation_unknown(mode: &ResolvedMode) -> bool {
    mode.effective.rank() >= Mode::Propose.rank() && mode.activated_at.is_none()
}
fn evaluates(mode: &ResolvedMode) -> bool {
    mode.effective != Mode::Off && !activation_unknown(mode)
}

fn needs_raw_header_absence(rule: &LoadedRule) -> bool {
    !rule.definition.matcher.list_id.is_empty() || !rule.definition.matcher.header.is_empty()
}

/// Outbound guards still require absence evidence; visible positive headers
/// suffice only for a header rule whose every action is non-outbound (owner
/// ruling 2026-11-06). A description rule keeps the account hold: its list
/// admission and entry cap are decided before any message is read, so a
/// per-message exclusion would let it bypass the cap.
pub fn header_provider_hold(rule: &LoadedRule, proves_absence: bool) -> bool {
    !proves_absence
        && needs_raw_header_absence(rule)
        && (rule.definition.is_description()
            || rule
                .definition
                .actions
                .iter()
                .any(|a| a.group().is_outbound()))
}

pub(super) fn partial_header_rule(rule: &LoadedRule, proves_absence: bool) -> bool {
    !proves_absence && needs_raw_header_absence(rule) && !header_provider_hold(rule, proves_absence)
}

/// The owner ruling's Unknown: this rule is absent for this message because
/// a reader that cannot prove absence did not surface the header.
pub fn absent_on_partial_headers(
    rule: &LoadedRule,
    candidate: &Candidate,
    proves_absence: bool,
) -> bool {
    partial_header_rule(rule, proves_absence)
        && header_match(&rule.definition.matcher, candidate) == HeaderMatch::Unknown
}

fn unsupported_action() -> ActionResult {
    ActionResult::Held {
        kind: FailureKind::Environment,
        reason: HeldReason::NotDispatched("unsupported action".into()),
    }
}

#[async_trait]
pub trait Mailbox: Send + Sync {
    /// Whether this account's reader can prove raw-header absence. Unknown
    /// capabilities fail closed; individual messages still need complete data.
    fn proves_header_absence(&self, _account: &str) -> bool {
        false
    }
    async fn candidates(
        &self,
        account: &str,
        pass: Pass,
        now: DateTime<Utc>,
    ) -> Result<Vec<String>>;
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
    /// Stage a template reply in the owner's outbox, answering exactly the
    /// candidate's authorised message, to its authenticated `From` only.
    /// Returns the outbox item id. A host with no outbox holds, dispatching
    /// nothing.
    ///
    /// `answering` is what the review shows the reply is to and about.
    async fn stage_reply(
        &self,
        _candidate: &Candidate,
        _body: &str,
        _answering: &crate::outbox::Answering,
    ) -> Result<String, HeldReason> {
        Err(HeldReason::NotDispatched("no outbox".into()))
    }
    /// Where a staged item stands now.
    fn staged_fate(&self, _item: &str) -> StagedFate {
        StagedFate::Unknown
    }
}

#[async_trait]
pub trait TaskSink: Send + Sync {
    async fn create(&self, task: &Task) -> ActionResult;
}

pub struct EngineOptions {
    pub pass: Pass,
    /// Dry-run selection only. Live passes need every sibling's safeguards.
    pub rules: Vec<String>,
    pub dry_run: bool,
    pub clock: std::sync::Arc<dyn crate::clock::Clock>,
    pub timezone: Tz,
    pub pending_timeout: Duration,
}
impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            pass: Pass::Headers,
            rules: Vec::new(),
            dry_run: false,
            clock: std::sync::Arc::new(crate::clock::SystemClock),
            timezone: chrono_tz::UTC,
            pending_timeout: Duration::minutes(15),
        }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct ApplyReport {
    /// Thread read failures, independent of diagnostic prose. Hosts use this
    /// for a non-zero exit even though no action was dispatched or settled.
    pub read_failures: usize,
    /// Dispatched effects that failed in this pass (holds are not failures).
    pub action_failures: usize,
    /// Account candidate-list failures; other accounts still run.
    pub candidate_failures: usize,
    /// Failed classification-skip marker writes; effects remain in the report.
    pub handled_failures: usize,
    pub rows: Vec<LedgerRow>,
    pub notices: Vec<String>,
    pub deferred: Vec<String>,
    pub handled: Vec<String>,
    /// Evaluation holds do not close action keys.
    pub list_cap: Vec<String>,
    /// Messages a non-outbound header rule was absent for because the reader
    /// cannot prove absence and did not surface the header (owner ruling
    /// 2026-11-06). Counted by the retrospective test, so "matched nothing"
    /// and "could not see the header" stay different readings.
    pub unknown_header_messages: usize,
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
    let subject = reply_subject_text(c);
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

/// The subject as `{subject}` fills it: decoded from Gmail's raw RFC 2047
/// form, unfolded otherwise, and `None` when it cannot be read whole — not
/// decodable, truncated, or more than one Subject header — the same reading
/// the matcher compares against.
pub(super) fn reply_subject_text(c: &Candidate) -> Option<String> {
    let unique = c
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("subject"))
        .map(|(_, values)| values.len())
        .sum::<usize>()
        <= 1;
    if c.subject_is_raw {
        headers::decode(&c.message.subject)
    } else {
        headers::unfold(&c.message.subject)
    }
    .filter(|_| unique && c.subject_complete != Some(false))
}

// Deliberately accepts a conservative addr-spec subset, never a display name,
// whitespace or syntax that could turn into instructions in a task's name.
pub fn parsed_address(s: &str) -> Option<String> {
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
        if row.counts_as_undo(rows) {
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
        } else if matches!(
            row.outcome,
            // A staged reply, and what the owner did with it, are this
            // group's successes (review of #856).
            Outcome::Done
                | Outcome::Sent
                | Outcome::Staged
                | Outcome::ReleasedUnchanged
                | Outcome::Edited
        ) {
            *streak = 0;
        }
    }
    evidence.automatic_mailbox = mailbox.len();
    evidence.undone_mailbox = mailbox.intersection(&undone).count();
    evidence.matches = matches.len();
    evidence.wrong_matches = matches.intersection(&wrong).count();
    evidence
}

pub fn unacknowledged_brakes(
    rule: &LoadedRule,
    rows: &[LedgerRow],
    approvals: &[ApprovalRow],
) -> BTreeMap<ActionGroup, Mode> {
    let mut targets = brake_targets(rule.definition.groups(), &brake_evidence(&rule.name, rows));
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
        rows.iter().any(|r| {
            r.rule == rule.name
                && (r.wrong_match
                    || (r.group == *group && (r.is_action_failure() || r.counts_as_undo(rows))))
                && acknowledged.is_none_or(|at| r.at.is_none_or(|event| event > at))
        })
    });
    targets
}

struct CachedModes {
    approvals: Vec<ApprovalRow>,
    gate: GateStatus,
    modes: BTreeMap<ActionGroup, ResolvedMode>,
}

struct Run<'a> {
    store: &'a RuleStore,
    mailbox: &'a dyn Mailbox,
    sink: Option<&'a dyn TaskSink>,
    options: &'a EngineOptions,
    rows: Vec<LedgerRow>,
    row_ids: HashSet<String>,
    report_row_ids: HashSet<String>,
    ledger_cursor: store::LedgerCursor,
    mode_cache: BTreeMap<String, CachedModes>,
    checked_inputs: BTreeMap<String, ApprovalDigest>,
    report: ApplyReport,
}
impl Run<'_> {
    fn remember_row(&mut self, row: LedgerRow) {
        if self.row_ids.insert(row.id.clone()) {
            self.mode_cache.remove(&row.rule);
            self.rows.push(row);
        }
    }

    fn report_row(&mut self, row: LedgerRow) {
        if self.report_row_ids.insert(row.id.clone()) {
            self.report.rows.push(row);
        }
    }

    /// An observation the ledger already holds, seen again by a later pass.
    ///
    /// The header timer runs every five minutes over a seven-day window, so
    /// the same shadow match or pre-activation skip is seen about two
    /// thousand times. Each is one fact about one key, recorded once: neither
    /// closes the key, and the page's rates already count distinct keys, so a
    /// repeat changes no measurement and only buries the ledger. A change in
    /// anything that makes it a different fact (mode, match digest, evidence
    /// kind, reason, partial headers) is recorded anew. Settlements and
    /// corrections are never observations.
    fn already_observed(&self, row: &LedgerRow) -> Option<&LedgerRow> {
        if !matches!(row.outcome, Outcome::WouldHave | Outcome::Skipped)
            || row.closes
            || row.settlement.is_some()
            || row.correction_of.is_some()
        {
            return None;
        }
        self.rows.iter().find(|seen| {
            seen.same_key(row)
                && seen.outcome == row.outcome
                && seen.settlement.is_none()
                && seen.correction_of.is_none()
                && seen.mode == row.mode
                && seen.match_digest == row.match_digest
                && seen.verdict_input == row.verdict_input
                && seen.reason == row.reason
                && seen.headers_partial == row.headers_partial
        })
    }

    fn write(&mut self, row: LedgerRow) -> Result<()> {
        if let Some(seen) = self.already_observed(&row).cloned() {
            // Reported as the row the ledger already holds, never as a new
            // one: the deferred settlement reads this report and appends any
            // outcome whose id the ledger lacks, and a dry run shows what a
            // live pass would. Still sync, so a concurrent owner correction
            // reaches this run as it would through a write.
            if !self.options.dry_run {
                self.sync_ledger()?;
            }
            self.report_row(seen);
            return Ok(());
        }
        if !self.options.dry_run {
            self.store.append_ledger(&row)?;
            self.sync_ledger()?;
        } else {
            self.remember_row(row.clone());
        }
        self.report_row(row);
        Ok(())
    }
    fn hold_unread_thread(
        &mut self,
        entries: &[&DeferredEntry],
        reason: &str,
        failure_kind: FailureKind,
    ) {
        // No message was authorized. These rows are observations only: never
        // persist them or settle a deferred snapshot, since Held closes keys.
        for snapshot in entries.iter().flat_map(|entry| &entry.actions) {
            let mut row = snapshot.clone();
            row.id = uuid::Uuid::new_v4().to_string();
            row.at = Some(self.options.clock.now());
            row.outcome = Outcome::Held;
            row.closes = false;
            row.reason = reason.into();
            row.failure_kind = failure_kind;
            self.report_row(row);
        }
    }
    fn sync_ledger(&mut self) -> Result<()> {
        // Concurrent owner corrections matter even while provider I/O awaits.
        // Own writes also enter through this cursor, preserving append order.
        for row in self.store.ledger_updates(&mut self.ledger_cursor)? {
            self.remember_row(row);
        }
        Ok(())
    }

    fn check_inputs(&mut self, rule: &LoadedRule) -> Result<bool> {
        let loaded = self.store.get(&rule.name);
        let changed = loaded
            .as_ref()
            .map_or(true, |fresh| fresh.approval_digest != rule.approval_digest);
        let unseen = loaded.as_ref().map_or(true, |fresh| {
            self.checked_inputs.get(&rule.name) != Some(&fresh.approval_digest)
        });
        if unseen {
            // Keep the edit/revert protection fresh. Only actual input changes
            // need another lapse transaction, never every ordinary dispatch.
            if !self.options.dry_run {
                let lapses = self.store.detect_lapses(
                    &rule.name,
                    &P1Holds {
                        origin_gate: self.mailbox.origin_gate(rule),
                    },
                    self.options.clock.now(),
                )?;
                if !lapses.is_empty() {
                    self.report
                        .notices
                        .push(format!("{}: approval lapsed: files changed", rule.name));
                }
            }
            if let Ok(fresh) = loaded {
                self.checked_inputs
                    .insert(rule.name.clone(), fresh.approval_digest);
            }
            self.mode_cache.remove(&rule.name);
        }
        Ok(changed)
    }

    fn refresh_mode_cache(&mut self, rule: &LoadedRule) -> Result<()> {
        self.sync_ledger()?;
        let holds = P1Holds {
            origin_gate: self.mailbox.origin_gate(rule),
        };
        // Always fresh: an owner may pause/reactivate or acknowledge a brake
        // during the preceding provider call. This read takes no writer lock.
        let mut approvals = self.store.approvals()?;
        if self
            .mode_cache
            .get(&rule.name)
            .is_some_and(|cached| cached.approvals == approvals && cached.gate == holds.origin_gate)
        {
            return Ok(());
        }
        let targets = unacknowledged_brakes(rule, &self.rows, &approvals);
        if !self.options.dry_run
            && targets.iter().any(|(group, target)| {
                let mode = resolve_mode(rule, *group, &approvals, &holds);
                mode.recorded.rank() > mode.effective.lower(*target).rank()
            })
        {
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
            approvals = self.store.approvals()?;
            self.sync_ledger()?;
        }
        let modes = rule
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
            .collect();
        self.mode_cache.insert(
            rule.name.clone(),
            CachedModes {
                approvals,
                gate: holds.origin_gate,
                modes,
            },
        );
        Ok(())
    }

    fn modes(&mut self, rule: &LoadedRule) -> Result<BTreeMap<ActionGroup, ResolvedMode>> {
        self.refresh_mode_cache(rule)?;
        let modes = self.mode_cache[&rule.name].modes.clone();
        for (group, mode) in &modes {
            if activation_unknown(mode) {
                let notice = format!("{} / {group:?}: held: activation unknown", rule.name);
                if !self.report.notices.contains(&notice) {
                    self.report.notices.push(notice);
                }
            }
        }
        Ok(modes)
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
    /// §5.4's per-sender limit: this rule already answered this sender —
    /// a reply staged, sent or still pending — within the last seven days,
    /// on another message. In shadow a would-have reply counts too, so the
    /// would-have rate is the rate the rule would actually reply at (review
    /// of #856); a shadow row never holds back a real reply after a raise.
    fn answered_this_week(&self, rule: &LoadedRule, c: &Candidate, mode: Mode) -> bool {
        let Some(sender) = parsed_address(&c.message.sender) else {
            return false;
        };
        let since = self.options.clock.now() - Duration::days(super::send::PER_SENDER_DAYS);
        self.rows.iter().any(|r| {
            r.rule == rule.name
                && r.group == ActionGroup::FixedWords
                && (matches!(
                    r.outcome,
                    Outcome::Pending
                        | Outcome::Staged
                        | Outcome::Sent
                        | Outcome::ReleasedUnchanged
                        | Outcome::Edited
                        | Outcome::Held
                ) || (mode == Mode::Shadow
                    && r.outcome == Outcome::WouldHave
                    && r.mode == Mode::Shadow))
                && !(r.message.account == c.message.account
                    && r.message.provider_message_id == c.message.provider_message_id)
                && r.at.is_some_and(|at| at >= since)
                && parsed_address(&r.message.sender).is_some_and(|s| s == sender)
        })
    }

    /// The rule's template text for a reply action.
    fn template<'r>(rule: &'r LoadedRule, action: &Action) -> Option<&'r str> {
        action
            .template
            .as_ref()
            .and_then(|name| rule.files.get(name))
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
    }

    /// Render the rule's template for this message and stage it.
    async fn stage_reply(
        &self,
        rule: &LoadedRule,
        action: &Action,
        c: &Candidate,
    ) -> Result<String, HeldReason> {
        let template = Self::template(rule, action)
            .ok_or_else(|| HeldReason::Other("template unreadable".into()))?;
        let today = self
            .options
            .clock
            .now()
            .with_timezone(&self.options.timezone)
            .format("%Y-%m-%d")
            .to_string();
        // The guard refused a template naming `{subject}` when the subject
        // has no honest value, so an empty one here is a template without it.
        let subject = reply_subject_text(c).unwrap_or_default();
        let body = super::send::render(template, &c.sending.from_name, &subject, &today);
        // What the owner's review shows: the address `from_only` answers and
        // the reply's subject, since no drafting session holds the thread.
        let answering = crate::outbox::Answering {
            to: parsed_address(&c.message.sender).unwrap_or_default(),
            subject: super::send::reply_subject(&subject),
        };
        self.mailbox.stage_reply(c, &body, &answering).await
    }

    /// Settle every staged send whose outbox item has been released or
    /// rejected since: one closing row each, linked to the staged row and
    /// its item, so the ledger shows what the owner did with it (§8).
    fn settle_staged(&mut self) -> Result<()> {
        let waiting: Vec<LedgerRow> = self
            .rows
            .iter()
            .filter(|r| r.outcome == Outcome::Staged && r.outbox_item.is_some())
            .filter(|staged| {
                !self.rows.iter().any(|later| {
                    later.correction_of.as_deref() == Some(staged.id.as_str())
                        && matches!(
                            later.outcome,
                            Outcome::ReleasedUnchanged
                                | Outcome::Edited
                                | Outcome::Rejected
                                | Outcome::Undone
                        )
                })
            })
            .cloned()
            .collect();
        for staged in waiting {
            let item = staged.outbox_item.clone().unwrap_or_default();
            let outcome = match self.mailbox.staged_fate(&item) {
                StagedFate::ReleasedUnchanged => Outcome::ReleasedUnchanged,
                StagedFate::Edited => Outcome::Edited,
                StagedFate::Rejected => Outcome::Rejected,
                StagedFate::Waiting | StagedFate::Unknown => continue,
            };
            let mut row = staged.clone();
            row.id = uuid::Uuid::new_v4().to_string();
            row.at = Some(self.options.clock.now());
            row.outcome = outcome;
            row.closes = true;
            row.settlement = None;
            row.correction_of = Some(staged.id.clone());
            row.reason = String::new();
            if !self.options.dry_run {
                self.store.append_ledger(&row)?;
                self.sync_ledger()?;
            } else {
                self.remember_row(row.clone());
            }
            self.report_row(row);
        }
        Ok(())
    }

    fn snapshot(
        &self,
        rule: &LoadedRule,
        index: usize,
        c: &Candidate,
        mode: Mode,
    ) -> Result<LedgerRow> {
        self.snapshot_described(rule, index, c, mode, None)
    }
    fn snapshot_described(
        &self,
        rule: &LoadedRule,
        index: usize,
        c: &Candidate,
        mode: Mode,
        described: Option<&Described>,
    ) -> Result<LedgerRow> {
        let gate = self.mailbox.origin_gate(rule);
        let at = self.options.clock.now();
        let mut row = match described {
            None => LedgerRow::header(
                rule,
                index,
                c.message.clone(),
                mode,
                gate,
                Outcome::WouldHave,
                at,
            )?,
            Some(d) => LedgerRow::described(
                rule,
                index,
                c.message.clone(),
                mode,
                gate,
                Outcome::WouldHave,
                at,
                d.input,
            )?,
        };
        let mut because = serde_json::to_value(&rule.definition.matcher)?;
        if let Some(d) = described {
            // The sender's words the classifier cited, grounded against what
            // it read: a human's "matched because", never a privileged run's.
            because["quote"] = serde_json::json!(d.quote(mode));
            if let Some(choice) = &d.confirmation_choice {
                // Both answers on the row (§4.2): the main pass's quote above,
                // and what the pass without learned rules chose.
                because["confirmation"] = serde_json::json!(choice);
            }
        }
        row.matched_because = serde_json::to_string(&because)?;
        row.headers_partial =
            partial_header_rule(rule, self.mailbox.proves_header_absence(&c.message.account));
        Ok(row)
    }

    /// What the stored verdict says about one description rule for this
    /// message, or `None` when it says nothing: the rule was not asked about,
    /// was asked under a matcher since edited, or was not named. Only a name
    /// from the active list can authorise an active action (§4.2).
    fn described(rule: &LoadedRule, c: &Candidate) -> Result<Option<Described>> {
        use crate::mail_triage::rule_frame::{RuleInput, RuleList};
        let Some(record) = c.classified.as_ref().and_then(|v| v.rules.as_ref()) else {
            return Ok(None);
        };
        let matcher = rule
            .match_digest(ActionGroup::Mailbox, GateStatus::Passing)?
            .matcher;
        let asked = |list| {
            record
                .considered
                .iter()
                .any(|k| k.name == rule.name && k.list == list && k.matcher == matcher)
        };
        let active = record
            .active
            .as_ref()
            .filter(|hit| hit.name == rule.name && asked(RuleList::Active))
            .map(|hit| hit.quote.clone());
        let shadow = record
            .shadow
            .iter()
            .find(|hit| hit.name == rule.name && asked(RuleList::Shadow))
            .map(|hit| hit.quote.clone());
        if active.is_none() && shadow.is_none() {
            return Ok(None);
        }
        let confirmation = record.confirmation.as_ref();
        Ok(Some(Described {
            // Only a rule the main pass named on its active list can be
            // confirmed active, structurally rather than by the guard that
            // drops it later (review of #855, pass 5).
            confirmed: active.is_some()
                && confirmation.is_some_and(|c| c.confirms_active(&rule.name)),
            shadow_confirmed: confirmation.and_then(|c| c.shadow_verdict(&rule.name)),
            // Under this build only, as `confirmed` is, so a row never says
            // both "the pass chose X" and "unconfirmed" (review of #855).
            confirmation_choice: confirmation
                .filter(|_| active.is_some())
                .and_then(|c| c.active_choice()),
            active,
            shadow,
            input: match record.input {
                RuleInput::Snippet => VerdictInput::Snippet,
                RuleInput::FullBody => VerdictInput::FullBody,
                RuleInput::Unknown => VerdictInput::Unknown,
            },
        }))
    }
    async fn apply_thread(&mut self, rules: &[LoadedRule], c: &Candidate) -> Result<()> {
        // Exclude before list admission, description obligations, matched-set
        // construction and every deferral/precedence/handled decision. Unknown
        // remains unknown; it is not converted into a proven negative.
        let applicable: Vec<_> = rules
            .iter()
            .filter(|rule| {
                !absent_on_partial_headers(
                    rule,
                    c,
                    self.mailbox.proves_header_absence(&c.message.account),
                )
            })
            .cloned()
            .collect();
        let rules = applicable.as_slice();
        let mut modes = BTreeMap::new();
        for rule in rules {
            self.check_inputs(rule)?;
            modes.insert(rule.name.clone(), self.modes(rule)?);
        }
        let unreadable: BTreeSet<String> = rules
            .iter()
            .filter(|r| super::describe::precondition_unreadable(&r.definition))
            .map(|r| r.name.clone())
            .collect();
        for name in &unreadable {
            let notice = format!("{name}: {}", super::describe::PRECONDITION_HOLD);
            if !self.report.notices.contains(&notice) {
                self.report.notices.push(notice);
            }
        }
        let admissible: Vec<_> = rules
            .iter()
            .filter(|r| !unreadable.contains(&r.name))
            .cloned()
            .collect();
        let mut held = list_cap(&admissible, &modes);
        self.report.list_cap.extend(held.iter().cloned());
        held.extend(unreadable);
        // A held description rule is not evaluated, so it owes classify
        // nothing and defers no other rule's removal.
        let descriptions = rules.iter().any(|r| {
            r.definition.is_description()
                && !held.contains(&r.name)
                && modes[&r.name].values().any(evaluates)
        });
        let matched: Vec<_> = rules
            .iter()
            .filter(|r| {
                !r.definition.is_description()
                    && !held.contains(&r.name)
                    && modes[&r.name].values().any(evaluates)
                    && header_match(&r.definition.matcher, c) != HeaderMatch::NoMatch
            })
            .collect();
        let owes_verdict = matched.iter().any(|r| {
            r.definition.actions.iter().any(|a| {
                evaluates(&modes[&r.name][&a.group()])
                    && (a.group().is_outbound()
                        || matches!(a.verb, ActionVerb::Digest | ActionVerb::Task))
            })
        });
        let unknown_deferral = matched.iter().any(|r| {
            header_match(&r.definition.matcher, c) == HeaderMatch::Unknown
                && r.definition.actions.iter().any(|a| {
                    evaluates(&modes[&r.name][&a.group()])
                        && (a.group().is_outbound()
                            || matches!(a.verb, ActionVerb::Digest | ActionVerb::Task))
                })
        });
        let defer_removal = descriptions || owes_verdict;
        let outbound = outbound_owner(&matched, &modes);
        let classified = classified(c);
        // Description rules join after classification, from the verdict
        // stored for this very message (`classified` holds only when the
        // record is at least as new as its newest inbound message), and only
        // when their header preconditions do not disprove it.
        let mut described = BTreeMap::new();
        if self.options.pass == Pass::AfterClassify && classified {
            for rule in rules.iter().filter(|r| {
                r.definition.is_description()
                    && !held.contains(&r.name)
                    && modes[&r.name].values().any(evaluates)
                    && header_match(&r.definition.matcher, c) != HeaderMatch::NoMatch
            }) {
                if let Some(d) = Self::described(rule, c)? {
                    described.insert(rule.name.clone(), d);
                }
            }
        }
        let acting: Vec<&LoadedRule> = rules
            .iter()
            .filter(|r| matched.iter().any(|m| m.name == r.name) || described.contains_key(&r.name))
            .collect();
        // §4.3: the outbound slot goes to the first matched rule by priority
        // with an active outbound group, description rules included — but a
        // description rule only through its active-list name, since a shadow
        // match never authorises an active action.
        let slot_candidates: Vec<&LoadedRule> = acting
            .iter()
            .copied()
            .filter(|r| described.get(&r.name).is_none_or(|d| d.active.is_some()))
            .collect();
        let outbound = if self.options.pass == Pass::AfterClassify {
            outbound_owner(&slot_candidates, &modes)
        } else {
            outbound
        };
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
                    || (mode.effective.rank() >= Mode::Propose.rank()
                        && mode.activated_at.is_none())
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
        for rule in acting {
            let current = self.modes(rule)?;
            let described = described.get(&rule.name);
            let mut pending = Vec::new();
            let mut planned = Vec::new();
            let mut reservations = Vec::new();
            let mut rule_handled = current
                .get(&ActionGroup::Mailbox)
                .is_some_and(|m| m.effective == Mode::Auto && m.activated_at.is_some())
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
                let mut mode = resolved.effective;
                if mode == Mode::Off
                    || (mode.rank() >= Mode::Propose.rank() && resolved.activated_at.is_none())
                {
                    continue;
                }
                if described.is_some_and(|d| d.active.is_none())
                    && mode.rank() >= Mode::Propose.rank()
                {
                    // Named only in the shadow list: measurement, never an
                    // active action, and no row for this group.
                    continue;
                }
                // §4.2: for a task or send, the main pass's match is a
                // candidate; an automatic one runs only when the confirmation
                // pass, with no learned rules in it, chose the same rule.
                // Keyed on the rule, not on the record, so a description rule
                // that ever arrived here without one fails closed (review of
                // #855, pass 2). `Some` carries the row's reason.
                let unconfirmed = (rule.definition.is_description()
                    && super::describe::confirmation_decides(action.group())
                    && !described.is_some_and(|d| d.confirmed))
                .then(|| {
                    let choice = described.and_then(|d| d.confirmation_choice.as_deref());
                    match choice {
                        Some(c) if c != crate::mail_triage::confirm::NOT_CONFIRMED => {
                            "unconfirmed: the confirmation pass did not choose this rule"
                        }
                        _ => "unconfirmed: the confirmation pass gave no answer",
                    }
                });
                if unconfirmed.is_some() && mode == Mode::Auto {
                    mode = Mode::Propose;
                }
                let mut row = self.snapshot_described(rule, index, c, mode, described)?;
                if mode == Mode::Shadow {
                    if let Some(d) =
                        described.filter(|_| super::describe::confirmation_decides(action.group()))
                    {
                        row.reason = match d.shadow_confirmed {
                            Some(true) => String::new(),
                            Some(false) => {
                                "would have proposed: the confirmation pass disagreed".into()
                            }
                            None => "not confirmed".into(),
                        };
                    }
                }
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
                    self.report_row(row);
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
                // §5.4's refusals bind a reply in every non-off mode, shadow
                // included, so a would-have rate never counts a reply the
                // guards would refuse.
                // A rule that does not hold the outbound slot is recorded as
                // superseded below, never refused for a guard it would not
                // have reached (review of #856).
                if action.group() == ActionGroup::FixedWords
                    && (mode == Mode::Shadow || outbound.as_deref() == Some(&rule.name))
                {
                    if let Some(guard) = rule
                        .template_refused
                        .as_ref()
                        .map(|why| format!("template refused: {why}"))
                        .or_else(|| super::send::reply_refusal(c).map(str::to_string))
                        .or_else(|| {
                            // A slot with no honest value refuses rather
                            // than render raw bytes (review of #856).
                            (Self::template(rule, action).is_some_and(|t| t.contains("{subject}"))
                                && reply_subject_text(c).is_none())
                            .then(|| "the subject could not be read whole".to_string())
                        })
                        .or_else(|| {
                            // Nor a name: "Hi , thanks" is a hole too.
                            (Self::template(rule, action)
                                .is_some_and(|t| t.contains("{sender_first_name}"))
                                && super::send::sender_first_name(&c.sending.from_name).is_empty())
                            .then(|| "the sender gave no name to address".to_string())
                        })
                        .or_else(|| {
                            self.answered_this_week(rule, c, mode).then(|| {
                                "this sender was answered by this rule in the last 7 days"
                                    .to_string()
                            })
                        })
                    {
                        row.outcome = Outcome::Skipped;
                        row.closes = false;
                        row.reason = guard;
                        self.write(row)?;
                        rule_handled = false;
                        continue;
                    }
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
                    self.report_row(row);
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
                planned.push((index, cap, unconfirmed));
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
                if !self.options.dry_run {
                    self.sync_ledger()?;
                }
                let mut provisional: BTreeSet<_> = pending.iter().map(|r| r.id.clone()).collect();
                for ((row, claim), (index, reserved_cap, unconfirmed)) in
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
                    self.remember_row(row.clone());
                    let action = &rule.definition.actions[index];
                    // A preceding sibling can await I/O while the owner pauses
                    // and reactivates a group, or can trip its failure brake.
                    let resolved = self.modes(rule)?[&action.group()].clone();
                    let mut mode = resolved.effective;
                    if unconfirmed.is_some() && mode == Mode::Auto {
                        mode = Mode::Propose;
                    }
                    let inputs_changed = self.check_inputs(rule)?;
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
                    let mut not_dispatched = false;
                    let mut staged_item = None;
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
                    } else if mode.rank() >= Mode::Propose.rank() && resolved.activated_at.is_none()
                    {
                        not_dispatched = true;
                        (
                            Outcome::Skipped,
                            "activation unknown".into(),
                            FailureKind::Environment,
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
                    } else if mode == Mode::Propose && action.group() == ActionGroup::FixedWords {
                        if self.options.dry_run {
                            (
                                Outcome::Staged,
                                "dry run: planned".into(),
                                FailureKind::Unknown,
                            )
                        } else {
                            match self.stage_reply(rule, action, c).await {
                                Ok(item) => {
                                    staged_item = Some(item);
                                    (Outcome::Staged, String::new(), FailureKind::Unknown)
                                }
                                Err(HeldReason::NotDispatched(reason)) => {
                                    not_dispatched = true;
                                    (Outcome::Skipped, reason, FailureKind::Environment)
                                }
                                Err(other) => {
                                    (Outcome::Failed, other.to_string(), FailureKind::Action)
                                }
                            }
                        }
                    } else if mode == Mode::Propose {
                        (
                            Outcome::Offered,
                            unconfirmed.unwrap_or_default().into(),
                            FailureKind::Unknown,
                        )
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
                                        reason: HeldReason::NotDispatched(
                                            "task sink unavailable".into(),
                                        ),
                                    },
                                },
                                // Report (spam) cannot reach dispatch:
                                // built_in_holds caps it at Propose. If a new
                                // unsupported group reaches this guard, nothing
                                // was dispatched: report a hold, release its key.
                                _ => unsupported_action(),
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
                            ActionResult::Held {
                                kind,
                                reason: HeldReason::NotDispatched(reason),
                            } => {
                                not_dispatched = true;
                                (Outcome::Skipped, reason, kind)
                            }
                            ActionResult::Held { kind, reason } => {
                                (Outcome::Held, reason.to_string(), kind)
                            }
                        }
                    };
                    self.report.action_failures += usize::from(outcome == Outcome::Failed);
                    rule_handled &= outcome == Outcome::Done;
                    let mut settled = row.settled(outcome, reason, attempted_at)?;
                    provisional.remove(&row.id);
                    settled.mode = mode;
                    settled.failure_kind = failure;
                    settled.outbox_item = staged_item;
                    let evidence_before_write = self.rows.len();
                    if not_dispatched {
                        // A persisted Held closes by outcome even with closes=false.
                        // Settle the unused claim as Skipped and report the hold
                        // separately, so caps and deferred occasions remain open.
                        if !self.options.dry_run {
                            self.store.append_ledger(&settled)?;
                            self.sync_ledger()?;
                        } else {
                            self.remember_row(settled.clone());
                        }
                        settled.id = uuid::Uuid::new_v4().to_string();
                        settled.settlement = None;
                        settled.outcome = Outcome::Held;
                        self.report_row(settled);
                    } else {
                        self.write(settled)?;
                    }
                    if self.rows[evidence_before_write..].iter().any(|r| {
                        r.rule == rule.name
                            && (r.is_action_failure()
                                || r.wrong_match
                                || r.outcome == Outcome::Undone)
                    }) {
                        self.refresh_mode_cache(rule)?;
                    }
                }
            }
            handled |= rule_handled;
        }
        if handled && self.options.pass == Pass::Headers {
            if !self.options.dry_run {
                if let Err(error) = self.mailbox.handled(c).await {
                    // Effects have already landed. Preserve their report and
                    // continue other threads/accounts, but do not advertise a
                    // classification skip without a successful marker write.
                    self.report.handled_failures += 1;
                    self.report.notices.push(format!(
                        "{}/{}: handled marker failed; leave for classification: {error:#}",
                        c.message.account, c.message.thread_id,
                    ));
                    return Ok(());
                }
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

/// §4.2's over-cap hold: shadow-only rules first, then `propose`, then
/// `auto`, each in priority then name order, until the entries fit
/// [`super::describe::LIST_CAP`]. A held rule is not evaluated at all.
pub fn list_cap(
    rules: &[LoadedRule],
    modes: &BTreeMap<String, BTreeMap<ActionGroup, ResolvedMode>>,
) -> BTreeSet<String> {
    let mut entries: Vec<_> = rules
        .iter()
        .filter(|r| r.definition.is_description())
        .map(|r| {
            let ms = &modes[&r.name];
            // The frame's own reading of which lists a rule is in, so the cap
            // never charges an entry the frame would not render (an active
            // mode with unknown activation is neither asked nor counted).
            let (active, shadow) = super::describe::lists(ms);
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
        if total <= super::describe::LIST_CAP {
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
        options.rules.is_empty() || options.dry_run,
        "a rule filter requires a dry-run; live apply must consider all rules"
    );
    ensure!(
        options.pending_timeout > Duration::zero(),
        "pending timeout must be positive"
    );
    let _lease = if options.dry_run {
        None
    } else {
        Some(store.engine_lease()?)
    };
    let selected = |name: &str| options.rules.is_empty() || options.rules.iter().any(|r| r == name);
    for name in &options.rules {
        store.get(name)?;
    }
    if !options.dry_run {
        // Detect broken/deleted definitions before list() can abort. Restoring
        // their bytes must not restore an old auto approval.
        for name in store
            .approvals()?
            .into_iter()
            .map(|r| r.rule)
            .filter(|name| selected(name))
            .collect::<BTreeSet<_>>()
        {
            let gate = store
                .get(&name)
                .map(|r| mailbox.origin_gate(&r))
                .unwrap_or_default();
            store.detect_lapses(&name, &P1Holds { origin_gate: gate }, options.clock.now())?;
        }
    }
    let mut rules = if options.rules.is_empty() {
        store.list()?
    } else {
        options
            .rules
            .iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|name| store.get(name))
            .collect::<Result<Vec<_>>>()?
    };
    rules.sort_by(|a, b| (a.definition.priority, &a.name).cmp(&(b.definition.priority, &b.name)));
    let (rows, ledger_cursor) = store.ledger_snapshot()?;
    let mut run = Run {
        store,
        mailbox,
        sink,
        options,
        row_ids: rows.iter().map(|row| row.id.clone()).collect(),
        rows,
        report_row_ids: HashSet::new(),
        ledger_cursor,
        mode_cache: BTreeMap::new(),
        checked_inputs: rules
            .iter()
            .map(|rule| (rule.name.clone(), rule.approval_digest.clone()))
            .collect(),
        report: ApplyReport::default(),
    };
    // What the owner did with staged sends since the last pass, before any
    // new decision: a rejection must be on the ledger before the same rule
    // considers the same thread again.
    run.settle_staged()?;
    let recovery_approvals = store.approvals()?;
    for row in run.rows.clone() {
        if row.outcome != Outcome::Pending || !selected(&row.rule) {
            continue;
        }
        if let Some(rule) = rules.iter().find(|rule| rule.name == row.rule) {
            let mode = resolve_mode(
                rule,
                row.group,
                &recovery_approvals,
                &P1Holds {
                    origin_gate: mailbox.origin_gate(rule),
                },
            );
            if activation_unknown(&mode) {
                continue;
            }
        }
        if row.outcome == Outcome::Pending
            && rules.iter().any(|rule| {
                rule.name == row.rule
                    && header_provider_hold(
                        rule,
                        mailbox.proves_header_absence(&row.message.account),
                    )
            })
        {
            continue; // A standing hold generates no per-message settlements.
        }
        if !run.rows.iter().any(|s| s.settles(&row)) {
            if let Some(settled) = row.interrupted(options.clock.now(), options.pending_timeout)? {
                run.report
                    .notices
                    .push(format!("{}: {}", row.rule, settled.reason));
                run.write(settled)?;
            }
        }
    }
    // Also resolve brakes and capacity when no mail is returned.
    let mut modes = BTreeMap::new();
    for rule in &rules {
        let resolved = run.modes(rule)?;
        modes.insert(rule.name.clone(), resolved);
    }
    let held_groups: BTreeSet<_> = modes
        .iter()
        .flat_map(|(name, groups)| {
            groups
                .iter()
                .filter(|(_, mode)| activation_unknown(mode))
                .map(|(group, _)| (name.clone(), *group))
        })
        .collect();
    let mut queue = store.deferred()?;
    let excluded: BTreeSet<_> = queue
        .iter()
        .flat_map(|entry| &entry.actions)
        .filter(|row| !selected(&row.rule))
        .map(|row| row.rule.clone())
        .collect();
    for entry in &mut queue {
        entry.actions.retain(|row| selected(&row.rule));
    }
    queue.retain(|entry| !entry.actions.is_empty());
    for account in accounts {
        let mut held_rules: BTreeSet<_> = rules
            .iter()
            .filter(|rule| header_provider_hold(rule, mailbox.proves_header_absence(account)))
            .map(|rule| rule.name.clone())
            .collect();
        for name in &held_rules {
            run.report.notices.push(format!(
                "{account}/{name}: held: provider cannot prove header absence for list_id/header matchers"
            ));
        }
        held_rules.extend(excluded.iter().cloned());
        // Standing conditions remove the whole rule before descriptions,
        // deferral, outbound priority, list caps or handled-marker decisions.
        // Even a positive raw-header match cannot override an outbound hold.
        let account_rules: Vec<_> = rules
            .iter()
            .filter(|rule| !held_rules.contains(&rule.name))
            .cloned()
            .collect();
        // Partial-header rules are header rules, which the list cap never counts.
        run.report.list_cap.extend(list_cap(&account_rules, &modes));
        let account_queue: Vec<_> = queue
            .iter()
            .filter(|entry| entry.account == *account)
            .cloned()
            .filter_map(|mut entry| {
                entry.actions.retain(|row| {
                    !held_rules.contains(&row.rule)
                        && modes
                            .get(&row.rule)
                            .and_then(|groups| groups.get(&row.group))
                            .is_none_or(|mode| {
                                mode.effective.rank() < Mode::Propose.rank()
                                    || mode.activated_at.is_some()
                            })
                });
                (!entry.actions.is_empty()).then_some(entry)
            })
            .collect();
        let mut ids: BTreeSet<_> = match mailbox
            .candidates(account, options.pass, options.clock.now())
            .await
        {
            Ok(ids) => ids.into_iter().collect(),
            Err(error) => {
                run.report.candidate_failures += 1;
                run.report
                    .notices
                    .push(format!("{account}: candidate listing failed: {error:#}"));
                // A failed listing is not an empty successful enumeration. Do
                // not expire or settle this account's deferred work, even in
                // the dry-run plan; preserve effects from the other accounts.
                continue;
            }
        };
        if options.dry_run {
            for entry in account_queue
                .iter()
                .filter(|entry| entry.account == *account && entry.expired(options.clock.now()))
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
                    row.settlement = None;
                    row.failure_kind = FailureKind::Unknown;
                    row.reason = "deferred for more than seven days".into();
                    // A crash may have persisted expiry before rewriting its
                    // queue. Preview the existing row, including its timestamp.
                    let row = run
                        .rows
                        .iter()
                        .find(|stored| stored.id == row.id)
                        .cloned()
                        .unwrap_or(row);
                    run.write(row)?;
                }
            }
        } else {
            let mut expired_ids = HashSet::new();
            for expired in store.expire_deferred_for_account(
                account,
                options.clock.now(),
                &held_rules,
                &held_groups,
            )? {
                for snapshot in &expired.actions {
                    expired_ids.insert(format!("expired:{}:{}", expired.id, snapshot.id));
                }
                run.report
                    .notices
                    .push(format!("{}: deferred work expired", expired.thread_id));
            }
            run.sync_ledger()?;
            for row in run
                .rows
                .clone()
                .into_iter()
                .filter(|row| expired_ids.contains(&row.id))
            {
                run.report_row(row);
            }
        }
        if options.pass == Pass::AfterClassify {
            ids.extend(
                account_queue
                    .iter()
                    .filter(|e| e.account == *account && !e.expired(options.clock.now()))
                    .map(|e| e.thread_id.clone()),
            );
        }
        for id in ids {
            let entries: Vec<_> = account_queue
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
                    run.hold_unread_thread(&entries, &reason, FailureKind::Unknown);
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
            let c = match read {
                Ok(c) => c,
                Err(_) => {
                    run.report.read_failures += 1;
                    run.report
                        .notices
                        .push(format!("{account}/{id}: thread unreadable"));
                    run.hold_unread_thread(&entries, "thread unreadable", FailureKind::Environment);
                    continue;
                }
            };
            let classified = classified(&c);
            run.apply_thread(&account_rules, &c).await?;
            if options.pass != Pass::AfterClassify || !classified {
                continue;
            }
            for entry in entries {
                let mut outcomes = Vec::new();
                for snapshot in &entry.actions {
                    if account_rules.iter().any(|rule| {
                        rule.name == snapshot.rule
                            && absent_on_partial_headers(
                                rule,
                                &c,
                                mailbox.proves_header_absence(account),
                            )
                    }) {
                        // A previously queued positive match is still unknown,
                        // not a negative that can settle its occasion.
                        continue;
                    }
                    if let Some(row) = run.report.rows.iter().rev().find(|r| {
                        r.same_key(snapshot)
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
                        row.outcome = Outcome::Skipped;
                        row.closes = row.outcome.closes(row.group);
                        row.reason =
                            "deferred action no longer matches the current rule/message".into();
                        row.failure_kind = FailureKind::Environment;
                        // No unknown fate: this snapshot was never dispatched.
                        outcomes.push(row);
                    }
                }
                if !options.dry_run && !outcomes.is_empty() {
                    store.settle_deferred(&entry.id, &outcomes)?;
                    run.sync_ledger()?;
                }
                for row in outcomes {
                    run.report_row(row.clone());
                    run.remember_row(row);
                }
            }
        }
    }
    run.report.list_cap.sort();
    run.report.list_cap.dedup();
    for name in &run.report.list_cap {
        run.report.notices.push(format!(
            "{name}: held: list cap ({} entries); lower or turn off another description rule",
            super::describe::LIST_CAP
        ));
    }
    run.report.notices.sort();
    run.report.notices.dedup();
    Ok(run.report)
}

#[cfg(test)]
mod tests;
