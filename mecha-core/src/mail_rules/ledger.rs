use super::{ActionGroup, ActionIdentity, ActionVerb, GateStatus, LoadedRule, MatchDigest, Mode};
use anyhow::{ensure, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

pub(super) fn wire<'de, D: serde::Deserializer<'de>, T: DeserializeOwned + Default>(
    d: D,
) -> std::result::Result<T, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictInput {
    NotApplicable,
    Snippet,
    FullBody,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pending,
    Done,
    WouldHave,
    Offered,
    Staged,
    Held,
    Sent,
    ReleasedUnchanged,
    Edited,
    Rejected,
    Undone,
    Expired,
    Failed,
    Skipped,
    ApprovalLapsed,
    #[default]
    #[serde(other)]
    Unknown,
}

impl Outcome {
    pub fn closes(self, group: ActionGroup) -> bool {
        match self {
            Self::Pending
            | Self::Done
            | Self::Offered
            | Self::Staged
            | Self::Held
            | Self::Sent
            | Self::ReleasedUnchanged
            | Self::Edited
            | Self::Rejected
            | Self::Undone
            | Self::Unknown => true,
            Self::Failed => !matches!(group, ActionGroup::Mailbox | ActionGroup::PrivateRecord),
            Self::WouldHave | Self::Expired | Self::Skipped | Self::ApprovalLapsed => false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Action,
    Environment,
    Interrupted,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MessageRef {
    pub account: String,
    pub provider_message_id: String,
    pub thread_id: String,
    /// RFC Message-ID is display only, never a key.
    pub message_id: String,
    pub sender: String,
    pub subject: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerRow {
    pub id: String,
    pub rule: String,
    #[serde(default)]
    pub at: Option<DateTime<Utc>>,
    #[serde(flatten)]
    pub message: MessageRef,
    #[serde(default, deserialize_with = "wire")]
    pub action: ActionIdentity,
    #[serde(default, deserialize_with = "wire")]
    pub group: ActionGroup,
    #[serde(default, deserialize_with = "wire")]
    pub mode: Mode,
    #[serde(default)]
    pub match_digest: MatchDigest,
    #[serde(default, deserialize_with = "wire")]
    pub verdict_input: VerdictInput,
    #[serde(default, deserialize_with = "wire")]
    pub outcome: Outcome,
    #[serde(default)]
    pub closes: bool,
    /// Exact pending-row id; never merely another row under the same key.
    #[serde(default)]
    pub settlement: Option<String>,
    #[serde(default)]
    pub matched_because: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default, deserialize_with = "wire")]
    pub failure_kind: FailureKind,
    #[serde(default)]
    pub wrong_match: bool,
    #[serde(default)]
    pub affected_groups: Vec<ActionGroup>,
}

impl LedgerRow {
    pub(super) fn validate_key(&self) -> Result<()> {
        ensure!(
            !self.id.trim().is_empty() && !self.rule.trim().is_empty(),
            "ledger row has no identifiable rule or row id"
        );
        if self.outcome == Outcome::ApprovalLapsed {
            return Ok(());
        }
        ensure!(
            !self.message.account.trim().is_empty()
                && !self.message.provider_message_id.trim().is_empty()
                && !self.message.thread_id.trim().is_empty()
                && !self.action.digest.trim().is_empty(),
            "action ledger row has an incomplete key"
        );
        Ok(())
    }

    pub fn header(
        rule: &LoadedRule,
        action: usize,
        message: MessageRef,
        mode: Mode,
        gate: GateStatus,
        outcome: Outcome,
        at: DateTime<Utc>,
    ) -> Result<Self> {
        ensure!(
            !rule.definition.is_description(),
            "P1 does not evaluate description rules"
        );
        let definition = rule
            .definition
            .actions
            .get(action)
            .ok_or_else(|| anyhow::anyhow!("no such action"))?;
        let group = definition.group();
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            rule: rule.name.clone(),
            at: Some(at),
            message,
            action: rule.actions[action].clone(),
            group,
            mode,
            match_digest: rule.match_digest(group, gate)?,
            verdict_input: VerdictInput::NotApplicable,
            outcome,
            closes: outcome.closes(group),
            settlement: None,
            matched_because: String::new(),
            reason: String::new(),
            failure_kind: FailureKind::Unknown,
            wrong_match: false,
            affected_groups: Vec::new(),
        })
    }

    pub fn settled(
        &self,
        outcome: Outcome,
        reason: impl Into<String>,
        at: DateTime<Utc>,
    ) -> Result<Self> {
        ensure!(
            self.outcome == Outcome::Pending,
            "only a pending row can be settled"
        );
        ensure!(
            !matches!(
                outcome,
                Outcome::Pending | Outcome::ApprovalLapsed | Outcome::Unknown
            ),
            "invalid settlement outcome"
        );
        let mut row = self.clone();
        row.id = uuid::Uuid::new_v4().to_string();
        row.at = Some(at);
        row.settlement = Some(self.id.clone());
        row.outcome = outcome;
        row.closes = outcome.closes(row.group);
        row.reason = reason.into();
        Ok(row)
    }

    pub fn interrupted(&self, now: DateTime<Utc>, timeout: Duration) -> Result<Option<Self>> {
        if self.outcome != Outcome::Pending || self.at.is_some_and(|at| now - at <= timeout) {
            return Ok(None);
        }
        let safe = matches!(
            self.group,
            ActionGroup::Mailbox | ActionGroup::PrivateRecord
        );
        let mut row = self.settled(
            if safe { Outcome::Failed } else { Outcome::Held },
            if safe { "interrupted" } else { "fate unknown" },
            now,
        )?;
        row.failure_kind = FailureKind::Interrupted;
        Ok(Some(row))
    }

    pub fn is_action_failure(&self) -> bool {
        self.outcome == Outcome::Failed && self.failure_kind == FailureKind::Action
    }

    pub(super) fn same_key(&self, other: &Self) -> bool {
        self.rule == other.rule
            && self.message.account == other.message.account
            && self.message.provider_message_id == other.message.provider_message_id
            && self.message.thread_id == other.message.thread_id
            && self.action == other.action
            && self.group == other.group
    }

    pub(super) fn settles(&self, pending: &Self) -> bool {
        // An ambiguous action cannot identify an attempt precisely enough to
        // discard its pending closure evidence.
        self.action.verb != ActionVerb::Unknown
            && self.group != ActionGroup::Unknown
            && pending.outcome == Outcome::Pending
            && self.outcome != Outcome::Pending
            && self.outcome != Outcome::ApprovalLapsed
            && self.settlement.as_deref() == Some(&pending.id)
            && self.same_key(pending)
    }

    fn closing(&self) -> bool {
        self.outcome != Outcome::ApprovalLapsed
            && (self.closes
                || self.outcome.closes(self.group)
                || self.action.verb == ActionVerb::Unknown)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageKey {
    pub rule: String,
    pub account: String,
    pub provider_message_id: String,
    pub action: ActionIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadKey {
    pub rule: String,
    pub account: String,
    pub thread_id: String,
}

/// Pending suppression precedes *every* closure signal, including closes=true.
fn surviving(rows: &[LedgerRow]) -> impl Iterator<Item = &LedgerRow> {
    rows.iter().filter(|row| {
        row.outcome != Outcome::Pending || !rows.iter().any(|settlement| settlement.settles(row))
    })
}

pub fn message_closed(rows: &[LedgerRow], key: &MessageKey) -> bool {
    if rows.iter().any(|row| row.validate_key().is_err()) {
        return true;
    }
    surviving(rows).any(|row| {
        row.rule == key.rule
            && row.message.account == key.account
            && row.message.provider_message_id == key.provider_message_id
            && row.action.digest == key.action.digest
            && (row.action.verb == key.action.verb
                || row.action.verb == ActionVerb::Unknown
                || key.action.verb == ActionVerb::Unknown)
            && row.closing()
    })
}

pub fn thread_closed(rows: &[LedgerRow], key: &ThreadKey) -> bool {
    if rows.iter().any(|row| row.validate_key().is_err()) {
        return true;
    }
    surviving(rows).any(|row| {
        row.rule == key.rule
            && row.message.account == key.account
            && row.message.thread_id == key.thread_id
            && row.group.once_per_thread()
            && row.closing()
    })
}

/// Snapshots of the owed actions let expiry leave a row per action even if the
/// rule has since been edited. These snapshots are not execution/pending rows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeferredEntry {
    pub id: String,
    pub account: String,
    pub thread_id: String,
    pub queued_at: DateTime<Utc>,
    pub actions: Vec<LedgerRow>,
}

impl DeferredEntry {
    pub fn expired(&self, now: DateTime<Utc>) -> bool {
        now - self.queued_at > Duration::days(7)
    }
}
