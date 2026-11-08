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
    /// A durable correction intent. Ordinary action recovery must not settle it.
    UndoPending,
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
    WrongMatch,
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
            | Self::UndoPending
            | Self::Done
            | Self::Offered
            | Self::Staged
            | Self::Held
            | Self::Sent
            | Self::ReleasedUnchanged
            | Self::Edited
            | Self::Rejected
            | Self::WrongMatch
            | Self::Undone
            | Self::Expired
            | Self::Unknown => true,
            Self::Failed => !matches!(group, ActionGroup::Mailbox | ActionGroup::PrivateRecord),
            Self::WouldHave | Self::Skipped | Self::ApprovalLapsed => false,
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
    /// RFC Message-ID: never a ledger key. A staged template reply does use
    /// it as `mail_reply`'s `message_id`, to name the answered message within
    /// its thread on both providers — sender-written, so to be re-examined
    /// before replies are sent unattended (MAIL-RULES-DESIGN §11 P4b-2a).
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
    /// Original completed row for an undo intent or its settlement.
    #[serde(default)]
    pub correction_of: Option<String>,
    /// Fresh provider membership for retrospective tests; absent on historical rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_inbox: Option<bool>,
    /// Provider arrival for retrospective rows, not sender Date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub received_at: Option<DateTime<Utc>>,
    /// Positive raw-header evidence on a reader that cannot prove absence.
    /// Historical or unrecognized values must never claim partial-header proof.
    #[serde(default, deserialize_with = "wire")]
    pub headers_partial: bool,
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
    /// Missing on historical rows; unknown attribution is never the owner.
    #[serde(default, deserialize_with = "wire")]
    pub actor: Option<crate::closure::Actor>,
    /// The outbox item a `staged` send waits in, and that its settlement
    /// (`released unchanged`, `edited`, `rejected`) was read from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outbox_item: Option<String>,
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
            "a description rule's row records what its verdict read"
        );
        Self::action(
            rule,
            action,
            message,
            mode,
            gate,
            outcome,
            at,
            VerdictInput::NotApplicable,
        )
    }

    /// A description rule's row: `input` is what the main pass's matching
    /// verdict read, never inferred from the rule's actions (§4.2).
    #[allow(clippy::too_many_arguments)]
    pub fn described(
        rule: &LoadedRule,
        action: usize,
        message: MessageRef,
        mode: Mode,
        gate: GateStatus,
        outcome: Outcome,
        at: DateTime<Utc>,
        input: VerdictInput,
    ) -> Result<Self> {
        ensure!(
            rule.definition.is_description(),
            "a header rule's row reads no verdict"
        );
        ensure!(
            input != VerdictInput::NotApplicable,
            "a description verdict read a snippet or the full body"
        );
        Self::action(rule, action, message, mode, gate, outcome, at, input)
    }

    #[allow(clippy::too_many_arguments)]
    fn action(
        rule: &LoadedRule,
        action: usize,
        message: MessageRef,
        mode: Mode,
        gate: GateStatus,
        outcome: Outcome,
        at: DateTime<Utc>,
        verdict_input: VerdictInput,
    ) -> Result<Self> {
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
            verdict_input,
            outcome,
            closes: outcome.closes(group),
            settlement: None,
            correction_of: None,
            in_inbox: None,
            received_at: None,
            headers_partial: false,
            matched_because: String::new(),
            reason: String::new(),
            failure_kind: FailureKind::Unknown,
            wrong_match: false,
            affected_groups: Vec::new(),
            actor: None,
            outbox_item: None,
        })
    }

    pub fn settled(
        &self,
        outcome: Outcome,
        reason: impl Into<String>,
        at: DateTime<Utc>,
    ) -> Result<Self> {
        ensure!(
            matches!(self.outcome, Outcome::Pending | Outcome::UndoPending),
            "only a pending row can be settled"
        );
        ensure!(
            !matches!(
                outcome,
                Outcome::Pending
                    | Outcome::UndoPending
                    | Outcome::ApprovalLapsed
                    | Outcome::WrongMatch
                    | Outcome::Unknown
            ),
            "invalid settlement outcome"
        );
        ensure!(
            self.outcome != Outcome::UndoPending
                || matches!(outcome, Outcome::Undone | Outcome::Failed),
            "undo intent must settle as undone or failed"
        );
        let mut row = self.clone();
        row.id = uuid::Uuid::new_v4().to_string();
        row.at = Some(at);
        row.settlement = Some(self.id.clone());
        row.outcome = outcome;
        row.closes = self.outcome == Outcome::UndoPending || outcome.closes(row.group);
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
            if safe {
                "interrupted"
            } else {
                super::FATE_UNKNOWN_REASON
            },
            now,
        )?;
        row.failure_kind = FailureKind::Interrupted;
        Ok(Some(row))
    }

    pub fn is_action_failure(&self) -> bool {
        // Only positively identified environment/interruption failures are
        // exempt from the brake. Missing or future kinds must count.
        self.outcome == Outcome::Failed
            && self.correction_of.is_none()
            && !matches!(
                self.failure_kind,
                FailureKind::Environment | FailureKind::Interrupted
            )
    }

    /// Uncertain undo fate counts toward stopping until a definitive failure
    /// settles this exact intent. A successful settlement still counts once.
    pub(super) fn counts_as_undo(&self, rows: &[Self]) -> bool {
        self.outcome == Outcome::Undone
            || (self.outcome == Outcome::UndoPending
                && !rows.iter().any(|settlement| settlement.settles(self)))
    }

    pub(super) fn same_key(&self, other: &Self) -> bool {
        self.rule == other.rule
            && self.message.account == other.message.account
            && self.message.provider_message_id == other.message.provider_message_id
            && self.message.thread_id == other.message.thread_id
            && self.action == other.action
            && self.group == other.group
    }

    pub(crate) fn settles(&self, pending: &Self) -> bool {
        // An ambiguous action cannot identify an attempt precisely enough to
        // discard its pending closure evidence.
        self.action.verb != ActionVerb::Unknown
            && self.group != ActionGroup::Unknown
            && match pending.outcome {
                Outcome::Pending => {
                    !matches!(
                        self.outcome,
                        Outcome::Pending
                            | Outcome::UndoPending
                            | Outcome::ApprovalLapsed
                            | Outcome::WrongMatch
                    ) && self.correction_of == pending.correction_of
                }
                Outcome::UndoPending => {
                    matches!(self.outcome, Outcome::Undone | Outcome::Failed)
                        && pending.correction_of.is_some()
                        && self.correction_of == pending.correction_of
                }
                _ => false,
            }
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
        !matches!(row.outcome, Outcome::Pending | Outcome::UndoPending)
            || !rows.iter().any(|settlement| settlement.settles(row))
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
