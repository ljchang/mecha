//! Description rules' half of the classify sweep (MAIL-RULES-DESIGN.md §4.2).
//!
//! Which rules enter the classifier's frame and in which list, which of them
//! may answer for one message, and which force a full-body read. The sweep
//! that asks lives in the CLI; the engine that acts on the answer lives in
//! [`super::engine`]. Both judge eligibility with the engine's own
//! [`engine::header_match`], so the two cannot drift into different readings
//! of the same precondition.
//!
//! Everything here is read-only: it never persists a lapse or a brake. The
//! engine does that at action time, and is the authority for what runs.
use super::engine::{self, Candidate, HeaderMatch};
use super::*;
use crate::mail_triage::rule_frame::{FrameEntry, RuleAsk, RuleFrame, RuleReading};
use anyhow::Result;
use std::collections::BTreeMap;

/// List entries the frame admits across the active and shadow lists together
/// (§4.2's cost assumption). A rule in both lists occupies two.
pub const LIST_CAP: usize = 12;

/// The longest `match.describe` a rule may carry, in characters.
pub const DESCRIBE_MAX_CHARS: usize = crate::mail_triage::rule_frame::DESCRIBE_MAX_CHARS;

/// A description rule whose header precondition reads a field the classify
/// sweep never sees. The sweep reads the inbox listing — sender, account,
/// subject, attachment flag — and not recipients, `List-Id` or raw headers,
/// so it cannot tell whether such a rule applies before the classifier
/// answers. Admitting it anyway would let a rule that cannot apply take the
/// single `rule` slot from one that can (§4.2), so it is held, visibly,
/// rather than evaluated on a guess. An implementation choice of P4a: lifting
/// it means carrying those headers into the sweep.
pub fn precondition_unreadable(definition: &RuleDefinition) -> bool {
    let m = &definition.matcher;
    definition.is_description()
        && (!m.to.is_empty() || !m.list_id.is_empty() || !m.header.is_empty())
}

pub const PRECONDITION_HOLD: &str =
    "held: a description rule's header conditions may use only from, from_domain, account, \
     subject_contains and has_attachment (the classify sweep does not read to, list_id or raw headers)";

/// Whether a group counts for evaluation at all: not off, and not an active
/// mode whose activation cannot be read (the engine holds those).
pub(super) fn evaluates(mode: &ResolvedMode) -> bool {
    mode.effective != Mode::Off
        && !(mode.effective.rank() >= Mode::Propose.rank() && mode.activated_at.is_none())
}

/// Which lists a rule's groups put it in: (active, shadow).
pub(super) fn lists(modes: &BTreeMap<ActionGroup, ResolvedMode>) -> (bool, bool) {
    (
        modes
            .values()
            .any(|m| evaluates(m) && m.effective.rank() >= Mode::Propose.rank()),
        modes.values().any(|m| m.effective == Mode::Shadow),
    )
}

/// Modes as the rules page reads them: resolved against approvals, then
/// lowered by any unacknowledged brake. Never written.
pub(super) fn observed_modes(
    rule: &LoadedRule,
    approvals: &[ApprovalRow],
    ledger: &[LedgerRow],
) -> BTreeMap<ActionGroup, ResolvedMode> {
    let targets = engine::unacknowledged_brakes(rule, ledger, approvals);
    rule.definition
        .groups()
        .into_iter()
        .map(|group| {
            let mut mode = resolve_mode(rule, group, approvals, &owner_file_holds());
            if let Some(target) = targets.get(&group) {
                mode.effective = mode.effective.lower(*target);
            }
            (group, mode)
        })
        .collect()
}

/// How many list entries these rules occupy, and each rule's share, over
/// the description rules the frame could admit (precondition holds excluded).
pub(super) fn entry_count(
    rules: &[LoadedRule],
    approvals: &[ApprovalRow],
    ledger: &[LedgerRow],
) -> (usize, Vec<(String, usize)>) {
    let mut shares = Vec::new();
    for rule in rules
        .iter()
        .filter(|r| r.definition.is_description() && !precondition_unreadable(&r.definition))
    {
        let (active, shadow) = lists(&observed_modes(rule, approvals, ledger));
        let share = usize::from(active) + usize::from(shadow);
        if share > 0 {
            shares.push((rule.name.clone(), share));
        }
    }
    (shares.iter().map(|(_, n)| n).sum(), shares)
}

/// The refusal an admission check gives: the cap, the count, and what to lower.
pub(super) fn over_cap(after: usize, shares: &[(String, usize)], adding: &str) -> String {
    let others: Vec<_> = shares
        .iter()
        .filter(|(name, _)| name != adding)
        .map(|(name, n)| format!("{name} ({n})"))
        .collect();
    format!(
        "description rules are limited to {LIST_CAP} classifier list entries across the active \
         and shadow lists; this would make {after}. Turn off or lower another description rule \
         first{}",
        if others.is_empty() {
            String::new()
        } else {
            format!(": {}", others.join(", "))
        }
    )
}

/// The envelope facts the classify sweep holds for a message: the inbox
/// listing's, never the body.
#[derive(Debug, Clone)]
pub struct Envelope<'a> {
    pub account: &'a str,
    /// The sender's address, without a display name.
    pub from: &'a str,
    pub subject: &'a str,
    pub has_attachment: Option<bool>,
}

impl Envelope<'_> {
    /// As a candidate the engine's matcher can read. Facts the sweep does not
    /// hold are marked incomplete, so a key over them could only be unknown —
    /// and [`precondition_unreadable`] keeps such keys out of the frame.
    fn candidate(&self) -> Candidate {
        Candidate {
            message: MessageRef {
                account: self.account.into(),
                sender: self.from.into(),
                subject: self.subject.into(),
                ..Default::default()
            },
            arrival_at: None,
            headers_complete: Some(false),
            subject_complete: Some(true),
            subject_is_raw: false,
            recipients_complete: Some(false),
            attachment_presence: self.has_attachment,
            to: Vec::new(),
            list_id: None,
            headers: BTreeMap::new(),
            classified: None,
            sending: Default::default(),
        }
    }
}

#[derive(Debug, Clone)]
struct FrameRule {
    name: String,
    matcher: Matcher,
    active: bool,
    shadow: bool,
    /// A send in a group that is not off: a match forces full-body evidence.
    escalates: bool,
    /// A Board or outbound group in `propose`/`auto`: the main pass's match
    /// is a candidate the active confirmation decides (§4.2).
    confirm_active: bool,
    /// A Board or outbound group in `shadow`: measured by both passes.
    confirm_shadow: bool,
    describe: String,
}

/// The groups the confirmation pass decides on a description rule. Every
/// rule's origin gate passes until P7 installs proposals, so gate-failing
/// ceilings do not arise yet.
pub fn confirmation_decides(group: ActionGroup) -> bool {
    group.is_outbound() || group == ActionGroup::Board
}

/// The description rules one classify sweep asks about.
#[derive(Debug, Clone, Default)]
pub struct ClassifierRules {
    pub frame: RuleFrame,
    rules: Vec<FrameRule>,
    /// One line per rule left out, and why. For the sweep's stderr.
    pub held: Vec<String>,
}

impl ClassifierRules {
    /// Read the store once, at the start of a sweep. A rule that does not
    /// load is the engine's and doctor's to report; it is simply not asked.
    pub fn load(store: &RuleStore) -> Result<Self> {
        let approvals = store.approvals()?;
        let ledger = store.ledger()?;
        let mut rules = Vec::new();
        let mut modes = BTreeMap::new();
        let mut held = Vec::new();
        for name in store.rule_names()? {
            let Ok(rule) = store.get(&name) else {
                continue;
            };
            if !rule.definition.is_description() {
                continue;
            }
            if precondition_unreadable(&rule.definition) {
                held.push(format!("{name}: {PRECONDITION_HOLD}"));
                continue;
            }
            modes.insert(name.clone(), observed_modes(&rule, &approvals, &ledger));
            rules.push(rule);
        }
        let capped = engine::list_cap(&rules, &modes);
        let mut out = Self::default();
        for rule in &rules {
            if capped.contains(&rule.name) {
                held.push(format!(
                    "{}: held: list cap ({LIST_CAP} entries); lower or turn off another description rule",
                    rule.name
                ));
                continue;
            }
            let ms = &modes[&rule.name];
            let (active, shadow) = lists(ms);
            if !active && !shadow {
                continue;
            }
            let escalates = rule
                .definition
                .actions
                .iter()
                .any(|a| a.group().is_outbound() && ms.get(&a.group()).is_some_and(evaluates));
            let decided = |want: fn(&ResolvedMode) -> bool| {
                ms.iter()
                    .any(|(group, mode)| confirmation_decides(*group) && want(mode))
            };
            out.rules.push(FrameRule {
                name: rule.name.clone(),
                matcher: rule.definition.matcher.clone(),
                active,
                shadow,
                escalates,
                confirm_active: decided(|m| {
                    evaluates(m) && m.effective.rank() >= Mode::Propose.rank()
                }),
                confirm_shadow: decided(|m| m.effective == Mode::Shadow),
                describe: rule.definition.matcher.describe.clone().unwrap_or_default(),
            });
        }
        out.rules.sort_by(|a, b| {
            let pa = rules
                .iter()
                .find(|r| r.name == a.name)
                .map(|r| r.definition.priority);
            let pb = rules
                .iter()
                .find(|r| r.name == b.name)
                .map(|r| r.definition.priority);
            pa.cmp(&pb).then_with(|| a.name.cmp(&b.name))
        });
        for fr in &out.rules {
            let rule = rules
                .iter()
                .find(|r| r.name == fr.name)
                .expect("frame rule was loaded");
            let entry = FrameEntry {
                name: fr.name.clone(),
                describe: rule.definition.matcher.describe.clone().unwrap_or_default(),
                matcher: rule
                    .match_digest(ActionGroup::Mailbox, GateStatus::Passing)?
                    .matcher,
            };
            if fr.active {
                out.frame.active.push(entry.clone());
            }
            if fr.shadow {
                out.frame.shadow.push(entry);
            }
        }
        out.held = held;
        Ok(out)
    }

    pub fn is_empty(&self) -> bool {
        self.frame.is_empty()
    }

    /// Which admitted rules may answer for this message: those whose header
    /// preconditions the envelope proves. Unknown is not eligible — the
    /// engine would hold the action anyway, and an unknown rule must not take
    /// the single `rule` slot from one that is known to apply.
    pub fn ask(&self, envelope: &Envelope<'_>) -> RuleAsk<'_> {
        let candidate = envelope.candidate();
        let fits =
            |r: &&FrameRule| engine::header_match(&r.matcher, &candidate) == HeaderMatch::Match;
        RuleAsk {
            frame: &self.frame,
            active: self
                .rules
                .iter()
                .filter(|r| r.active)
                .filter(fits)
                .map(|r| r.name.clone())
                .collect(),
            shadow: self
                .rules
                .iter()
                .filter(|r| r.shadow)
                .filter(fits)
                .map(|r| r.name.clone())
                .collect(),
        }
    }

    fn entry(r: &FrameRule) -> FrameEntry {
        FrameEntry {
            name: r.name.clone(),
            describe: r.describe.clone(),
            matcher: String::new(),
        }
    }

    /// What the active confirmation is offered, when the main pass's
    /// grounded `rule` is a candidate for a task or send: every rule eligible
    /// for this message with a confirmation-eligible active group, in frame
    /// order — not only the main pass's pick, so the choice *between* two
    /// rules that both fit is never left to the pass with learned rules.
    /// Empty when no confirmation is needed.
    pub fn active_confirmation(&self, ask: &RuleAsk<'_>, reading: &RuleReading) -> Vec<FrameEntry> {
        let candidate = reading.active.as_ref().is_some_and(|hit| {
            self.rules
                .iter()
                .any(|r| r.name == hit.name && r.confirm_active)
        });
        if !candidate {
            return Vec::new();
        }
        self.rules
            .iter()
            .filter(|r| r.confirm_active && ask.active.contains(&r.name))
            .map(Self::entry)
            .collect()
    }

    /// What the shadow confirmation judges, one verdict each: every rule in
    /// the reconciled match set (named by either list) with a shadow group
    /// the pass decides.
    pub fn shadow_confirmation(&self, reading: &RuleReading) -> Vec<FrameEntry> {
        let named: Vec<&str> = reading.named().collect();
        self.rules
            .iter()
            .filter(|r| r.confirm_shadow && named.contains(&r.name.as_str()))
            .map(Self::entry)
            .collect()
    }

    /// The confirmation pass for a reading that has a task or send candidate
    /// (MAIL-RULES-DESIGN.md §4.2): a second quarantined call with no learned
    /// rules and no correction examples, over the same text the main pass's
    /// final answer read. `None` when nothing needed confirming, and then no
    /// call is made. A failed call is recorded as failed, which the engine
    /// demotes on and reports as "not confirmed".
    pub async fn confirm(
        &self,
        provider: &dyn crate::provider::Provider,
        model: &str,
        thread: &crate::mail_triage::ThreadInput,
        ask: &RuleAsk<'_>,
        reading: &RuleReading,
    ) -> Option<crate::mail_triage::confirm::Confirmation> {
        use crate::mail_triage::confirm;
        let offered = self.active_confirmation(ask, reading);
        let shadow = self.shadow_confirmation(reading);
        if offered.is_empty() && shadow.is_empty() {
            return None;
        }
        let mut out = confirm::Confirmation {
            version: confirm::CONFIRMATION_VERSION.clone(),
            ..Default::default()
        };
        if !offered.is_empty() {
            out.active = Some(confirm::confirm_active(provider, model, thread, &offered).await);
        }
        if !shadow.is_empty() {
            out.shadow = confirm::confirm_each(provider, model, thread, &shadow).await;
        }
        Some(out)
    }

    /// Whether this reading needs the full body before it is final: a match,
    /// in either list, on a rule with a send in a group that is not off.
    /// Per thread, across rules: one such match escalates every verdict.
    pub fn escalates(&self, reading: &RuleReading) -> bool {
        reading
            .named()
            .any(|name| self.rules.iter().any(|r| r.name == name && r.escalates))
    }
}
