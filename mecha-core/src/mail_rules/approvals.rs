use super::{ActionGroup, ApprovalDigest, LoadedRule};
use crate::{closure::Actor, imagelib::Origin};
use anyhow::{ensure, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Off,
    Shadow,
    Propose,
    Auto,
    #[default]
    #[serde(other)]
    Unknown,
}

impl Mode {
    pub fn effective(self) -> Self {
        if self == Self::Unknown {
            Self::Shadow
        } else {
            self
        }
    }

    pub fn rank(self) -> u8 {
        match self.effective() {
            Self::Off => 0,
            Self::Shadow | Self::Unknown => 1,
            Self::Propose => 2,
            Self::Auto => 3,
        }
    }

    pub fn lower(self, ceiling: Self) -> Self {
        if self.rank() <= ceiling.rank() {
            self.effective()
        } else {
            ceiling.effective()
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalReason {
    Install,
    Owner,
    StepDown,
    Lapse,
    Brake,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalRow {
    pub id: String,
    pub rule: String,
    pub group: ActionGroup,
    pub at: Option<DateTime<Utc>>,
    pub from_mode: Mode,
    pub to_mode: Mode,
    pub actor: Actor,
    pub digests: ApprovalDigest,
    pub reason: ApprovalReason,
    // Explicit Unknown may lower permissions; an unreadable actor cannot
    // authenticate even the claimed transition. Never trust this bit from disk.
    #[serde(skip)]
    pub(super) actor_readable: bool,
}

impl<'de> Deserialize<'de> for ApprovalRow {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct WireRow {
            id: String,
            rule: String,
            #[serde(default, deserialize_with = "super::ledger::wire")]
            group: ActionGroup,
            #[serde(default)]
            at: Option<DateTime<Utc>>,
            #[serde(default, deserialize_with = "super::ledger::wire")]
            from_mode: Mode,
            #[serde(default, deserialize_with = "super::ledger::wire")]
            to_mode: Mode,
            #[serde(default)]
            actor: serde_json::Value,
            #[serde(default)]
            digests: ApprovalDigest,
            #[serde(default, deserialize_with = "super::ledger::wire")]
            reason: ApprovalReason,
        }
        let row = WireRow::deserialize(d)?;
        let actor_readable = matches!(
            row.actor.as_str(),
            Some("owner" | "owner-approved" | "unknown")
        );
        Ok(Self {
            id: row.id,
            rule: row.rule,
            group: row.group,
            at: row.at,
            from_mode: row.from_mode,
            to_mode: row.to_mode,
            actor: serde_json::from_value(row.actor).unwrap_or(Actor::Unknown),
            digests: row.digests,
            reason: row.reason,
            actor_readable,
        })
    }
}

pub(super) struct ApprovalState<'a> {
    pub mode: Mode,
    pub digest: Option<&'a ApprovalDigest>,
    pub activated_at: Option<DateTime<Utc>>,
    pub held: bool,
}

/// Validate against the preceding *validated* state, never the row's claimed
/// from_mode. A bad row must not reveal an older permission underneath it.
pub(super) fn valid_transition(row: &ApprovalRow, preceding: &ApprovalState<'_>) -> bool {
    // Rebinding an Auto row to changed inputs is a raise too: the preceding
    // permission only covers its own digest, even if no lapse row was written.
    let authority = if preceding
        .digest
        .is_some_and(|digest| digest != &row.digests)
    {
        preceding.mode.lower(Mode::Propose)
    } else {
        preceding.mode
    };
    !row.id.trim().is_empty()
        && !row.rule.trim().is_empty()
        && row.actor_readable
        && row.group != ActionGroup::Unknown
        && row.from_mode != Mode::Unknown
        && row.to_mode != Mode::Unknown
        && row.from_mode == preceding.mode
        && (row.actor == Actor::Owner
            || (row.to_mode != Mode::Off && row.to_mode.rank() <= authority.rank()))
}

pub(super) fn approval_state<'a>(
    rule: &str,
    group: ActionGroup,
    rows: &'a [ApprovalRow],
) -> ApprovalState<'a> {
    let mut state = ApprovalState {
        mode: Mode::Shadow,
        digest: None,
        activated_at: None,
        held: false,
    };
    for row in rows {
        let unknown_rule = row.rule.trim().is_empty();
        if !unknown_rule && row.rule != rule {
            continue;
        }
        if !unknown_rule && row.group != group && row.group != ActionGroup::Unknown {
            continue;
        }
        let valid = !unknown_rule && valid_transition(row, &state);
        state.digest = Some(&row.digests);
        if !valid {
            state.mode = state.mode.lower(Mode::Shadow);
            state.activated_at = None;
            state.held = true;
            continue;
        }
        let preceding = state.mode;
        state.mode = row.to_mode;
        // A later, positively identified owner may repair this group's
        // permission. Automatic lowerings never clear corrupt-history holds.
        if row.actor == Actor::Owner {
            state.held = false;
        }
        if state.mode.rank() < Mode::Propose.rank() {
            state.activated_at = None;
        } else if preceding.rank() < Mode::Propose.rank() {
            state.activated_at = row.at;
        }
    }
    state
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeDecision {
    pub requested: Mode,
    /// The resulting effective mode when refused; otherwise the requested or
    /// clamped target to record.
    pub target: Mode,
    pub clamped: bool,
    pub write: bool,
    /// A permanent built-in ceiling refused this request; no raise was written.
    pub refused: bool,
    /// Current ceilings and their reasons, including the reason for a refusal.
    pub holds: Vec<ModeHold>,
}

/// Uses an already-read closure actor. None is an unreadable reading, never
/// ownership; callers use closure::attribute and its existing shell/run readers.
pub fn gate_mode(current: Mode, requested: Mode, actor: Option<Actor>) -> Result<ModeDecision> {
    ensure!(requested != Mode::Unknown, "cannot request an unknown mode");
    let owner = actor == Some(Actor::Owner);
    let clamped = requested == Mode::Off && !owner;
    let target = if clamped { Mode::Shadow } else { requested };
    if current == Mode::Off && !owner && requested.rank() <= Mode::Shadow.rank() {
        return Ok(ModeDecision {
            requested,
            target: Mode::Off,
            clamped,
            write: false,
            refused: false,
            holds: Vec::new(),
        });
    }
    ensure!(
        target.rank() <= current.rank() || owner,
        "raising a mail rule requires Actor::Owner"
    );
    Ok(ModeDecision {
        requested,
        target,
        clamped,
        write: target != current,
        refused: false,
        holds: Vec::new(),
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GateStatus {
    Passing,
    Failing,
    #[default]
    Unknown,
}

impl GateStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passing => "passing",
            Self::Failing => "failing",
            Self::Unknown => "unknown",
        }
    }

    /// The caller supplies every rule key and named file. Missing provenance
    /// must be supplied as Origin::default(), never omitted as if clean.
    pub fn of_origins(origins: &[Origin]) -> Self {
        if origins.is_empty() {
            Self::Unknown
        } else if origins.contains(&Origin::ModelUntrusted) {
            Self::Failing
        } else {
            Self::Passing
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeHold {
    pub ceiling: Mode,
    pub reason: String,
}

/// Additional standing holds (account, provider, provenance) are computed,
/// never approvals rows. The resolver always intersects them with P1's holds.
pub trait HoldProvider {
    fn holds(&self, rule: &LoadedRule, group: ActionGroup) -> Vec<ModeHold>;
}

/// P1 has no proposal installer: owner-authored rules can use Passing; a
/// caller without a complete provenance reading must use the default Unknown.
#[derive(Debug, Default)]
pub struct P1Holds {
    pub origin_gate: GateStatus,
}

impl HoldProvider for P1Holds {
    fn holds(&self, _rule: &LoadedRule, group: ActionGroup) -> Vec<ModeHold> {
        if group == ActionGroup::Board && self.origin_gate != GateStatus::Passing {
            vec![ModeHold {
                ceiling: Mode::Propose,
                reason: "rule provenance is not known clean".into(),
            }]
        } else {
            Vec::new()
        }
    }
}

pub(super) fn built_in_holds(rule: &LoadedRule, group: ActionGroup) -> Vec<ModeHold> {
    let mut holds = Vec::new();
    if !group.supported() || rule.definition.is_description() {
        holds.push(ModeHold {
            ceiling: Mode::Shadow,
            reason: "not yet supported in P1".into(),
        });
    }
    if group == ActionGroup::Report {
        holds.push(ModeHold {
            ceiling: Mode::Propose,
            reason: "spam requires owner confirmation".into(),
        });
    }
    holds
}

pub(super) fn mode_ceiling(rule: &LoadedRule, group: ActionGroup) -> Mode {
    built_in_holds(rule, group)
        .iter()
        .fold(Mode::Auto, |mode, hold| mode.lower(hold.ceiling))
}

#[derive(Debug, Clone)]
pub struct ResolvedMode {
    pub recorded: Mode,
    pub effective: Mode,
    pub lapsed: bool,
    pub holds: Vec<ModeHold>,
    pub activated_at: Option<DateTime<Utc>>,
}

pub fn resolve_mode(
    rule: &LoadedRule,
    group: ActionGroup,
    rows: &[ApprovalRow],
    provider: &dyn HoldProvider,
) -> ResolvedMode {
    let state = approval_state(&rule.name, group, rows);
    let recorded = state.mode;
    let lapsed = recorded != Mode::Off
        && state
            .digest
            .is_some_and(|digest| digest != &rule.approval_digest);
    let mut effective = recorded;
    if lapsed {
        effective = effective.lower(Mode::Propose);
    }
    let mut holds = built_in_holds(rule, group);
    holds.extend(provider.holds(rule, group));
    if state.held {
        holds.push(ModeHold {
            ceiling: Mode::Shadow,
            reason: "unreadable or unauthorized approval history".into(),
        });
    }
    for hold in &mut holds {
        // A hold may stop actions, never the would-have record. Preserve an
        // existing owner Off through lower(), but never manufacture one.
        if hold.ceiling == Mode::Off || hold.ceiling == Mode::Unknown {
            hold.ceiling = Mode::Shadow;
            hold.reason = format!("invalid hold ceiling (clamped to shadow): {}", hold.reason);
        }
        effective = effective.lower(hold.ceiling);
    }
    ResolvedMode {
        recorded,
        effective,
        lapsed,
        holds,
        activated_at: state.activated_at,
    }
}

/// Statistics are supplied by the execution layer, over distinct actions and
/// matches. Environment failures and interrupted attempts never enter the streak.
#[derive(Debug, Clone, Default)]
pub struct BrakeEvidence {
    pub automatic_mailbox: usize,
    pub undone_mailbox: usize,
    pub matches: usize,
    pub wrong_matches: usize,
    pub consecutive_action_failures: std::collections::BTreeMap<ActionGroup, usize>,
    pub wrong_automatic_sends: std::collections::BTreeSet<ActionGroup>,
}

pub fn brake_targets(
    groups: impl IntoIterator<Item = ActionGroup>,
    evidence: &BrakeEvidence,
) -> std::collections::BTreeMap<ActionGroup, Mode> {
    let mut targets = std::collections::BTreeMap::new();
    let wrong_match =
        evidence.matches >= 10 && evidence.wrong_matches as f64 / evidence.matches as f64 > 0.2;
    for group in groups {
        if wrong_match
            || (group == ActionGroup::Mailbox
                && evidence.automatic_mailbox >= 10
                && evidence.undone_mailbox as f64 / evidence.automatic_mailbox as f64 > 0.2)
            || (group.is_outbound() && evidence.wrong_automatic_sends.contains(&group))
        {
            targets.insert(group, Mode::Propose);
        }
        if evidence
            .consecutive_action_failures
            .get(&group)
            .copied()
            .unwrap_or_default()
            >= 3
        {
            targets.insert(group, Mode::Shadow);
        }
    }
    targets
}
