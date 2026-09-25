//! Prospective appraisal, separate from observed affect. No model chooses a label.
//! Evidence is either a harness observation or an explicitly owner-authored record.
//! Missing evidence stays unknown. Fixed guidance never includes owner evidence prose.
use crate::goal::GoalRef;
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Guilt,
    Embarrassment,
    Regret,
    Disappointment,
    Anxiety,
    Curiosity,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verification {
    #[default]
    Unknown,
    Pending,
    Passed,
    Failed,
}

/// This record is authored by the owner, not extracted from incoming messages.
///
/// **The legacy shape** (1f-2, ruling 2026-09-25: new writes only, no
/// migration). It is what an owner writes in an appraisal evidence file,
/// and what every prediction recorded before 1f-2 carries; both keep
/// working unchanged. A new write converts it to the one commitment record
/// ([`RecordedCommitment::Record`], `workflow::Commitment`) at the door
/// ([`Evidence::into_record`]), and a record already on disk in this shape
/// is read as it is and written back as it is — nothing is rewritten.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Commitment {
    pub beneficiary: String,
    pub expectation: String,
    /// The adverse consequence of failing this commitment, as judged by the owner.
    pub consequence: String,
}

/// A commitment as evidence carries it: the one record a new write makes,
/// or the legacy shape an older record — or an owner's input file — holds.
///
/// **Untagged, and the order is the rule.** The record is tried first,
/// strictly (unknown keys refused, as all owner evidence is); the legacy
/// shape second, strictly too. Each serialises back in its own shape, so a
/// prediction recorded before 1f-2 round-trips byte-identical when its
/// draft is rewritten for another reason, and its `assess` reads exactly as
/// before — it only asks whether a commitment is there. A shape neither arm
/// knows (a later binary's) fails the evidence, which the prediction
/// history keeps as `History::Unknown` rather than losing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RecordedCommitment {
    /// The one commitment record (`workflow::Commitment`): what every new
    /// prediction writes. Its `source` is the goal pointer the evidence is
    /// bound to, and its dates are only ever the owner's: none when it came
    /// from the legacy shape, exactly the ones written when the owner gave
    /// the record shape — the harness never supplies a "by when".
    Record(#[serde(deserialize_with = "strict_record")] crate::workflow::Commitment),
    /// The shape predictions recorded before 1f-2 carry, and the one an
    /// owner's evidence file may still be written in.
    Legacy(Commitment),
}

impl From<Commitment> for RecordedCommitment {
    fn from(c: Commitment) -> Self {
        RecordedCommitment::Legacy(c)
    }
}

impl From<crate::workflow::Commitment> for RecordedCommitment {
    fn from(c: crate::workflow::Commitment) -> Self {
        RecordedCommitment::Record(c)
    }
}

impl RecordedCommitment {
    /// Owner text bounds, per shape. A record on evidence must say what the
    /// party expects and what failing it costs — the two fields the legacy
    /// shape always required; the record makes them optional only for the
    /// workflow store's own commitments.
    fn validate(&self) -> Result<()> {
        match self {
            RecordedCommitment::Legacy(c) => {
                for s in [&c.beneficiary, &c.expectation, &c.consequence] {
                    bounded(s)?;
                }
            }
            RecordedCommitment::Record(c) => {
                bounded(&c.party)?;
                bounded(&c.source)?;
                let (Some(expectation), Some(consequence)) = (&c.expectation, &c.consequence)
                else {
                    anyhow::bail!("a commitment on evidence names its expectation and consequence");
                };
                bounded(expectation)?;
                bounded(consequence)?;
                if let (Some(due), Some(follow_up)) = (c.due_at, c.follow_up_at) {
                    ensure!(
                        follow_up <= due,
                        "follow-up must be no later than the commitment deadline"
                    );
                }
            }
        }
        Ok(())
    }
}

/// The record arm, read strictly: owner evidence refuses an unknown key in
/// the new shape exactly as it does in the old, so a misspelt field is an
/// error and never a silently dropped fact.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictRecord {
    party: String,
    source: String,
    #[serde(default)]
    due_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    follow_up_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    expectation: Option<String>,
    #[serde(default)]
    consequence: Option<String>,
}

fn strict_record<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<crate::workflow::Commitment, D::Error> {
    let r = StrictRecord::deserialize(d)?;
    Ok(crate::workflow::Commitment {
        party: r.party,
        source: r.source,
        due_at: r.due_at,
        follow_up_at: r.follow_up_at,
        expectation: r.expectation,
        consequence: r.consequence,
    })
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Evidence {
    #[serde(default, deserialize_with = "strict_goal")]
    pub goal: Option<GoalRef>,
    /// The owner's recorded commitment, in either shape
    /// ([`RecordedCommitment`]). A new write is always the record.
    pub commitment: Option<RecordedCommitment>,
    pub verification: Verification,
    /// Local receipt/reference or owner's account of what was checked. Never prompted.
    pub verification_evidence: Option<String>,
    pub expected_outcome: Option<String>,
    pub check_available: bool,
    pub check_cost_secs: Option<u64>,
    pub time_available_secs: Option<u64>,
    /// Evidence that outstanding work exceeds the remaining execution budget.
    pub budget_shortfall: bool,
}
impl Evidence {
    pub fn validate(&self) -> Result<()> {
        if let Some(c) = &self.commitment {
            ensure!(
                self.goal.is_some(),
                "a commitment requires a goal reference"
            );
            c.validate()?;
        }
        if let Some(s) = &self.verification_evidence {
            bounded(s)?;
        }
        if let Some(s) = &self.expected_outcome {
            bounded(s)?;
        }
        if matches!(
            self.verification,
            Verification::Passed | Verification::Failed
        ) {
            ensure!(
                self.verification_evidence.is_some(),
                "a check verdict requires evidence"
            );
        }
        Ok(())
    }

    /// The evidence as a new write records it (1f-2): validated, with its
    /// commitment as the one record. A legacy-shaped commitment — the shape
    /// an owner's evidence file is written in — becomes a
    /// `workflow::Commitment` whose party is the beneficiary, whose
    /// `source` is the goal pointer the evidence is bound to (structural,
    /// never text), and which has **no date** — that shape carries none. A
    /// record given as input must already point at that goal, so every new
    /// write's `source` is the same structural pointer, and it keeps
    /// exactly the dates the owner wrote on it. Dates are only ever the
    /// owner's: nothing here derives one (ruled 2026-09-25).
    ///
    /// The door for every owner-evidence write — `BoundEvidence::new` (a
    /// run's evidence, and so every draft it stages) and
    /// `OutboxStore::anticipate` (a draft assessed by hand). It creates no
    /// commitment: it only reshapes one the owner wrote (§7.4).
    pub fn into_record(mut self) -> Result<Evidence> {
        self.validate()?;
        let Some(commitment) = self.commitment.take() else {
            return Ok(self);
        };
        let source = self
            .goal
            .as_ref()
            .expect("validate: a commitment has a goal")
            .to_string();
        self.commitment = Some(match commitment {
            RecordedCommitment::Legacy(c) => {
                RecordedCommitment::Record(crate::workflow::Commitment {
                    party: c.beneficiary,
                    source,
                    due_at: None,
                    follow_up_at: None,
                    expectation: Some(c.expectation),
                    consequence: Some(c.consequence),
                })
            }
            RecordedCommitment::Record(c) => {
                ensure!(
                    c.source == source,
                    "a commitment's source is the goal it is bound to ({source}), not {:?}",
                    c.source
                );
                RecordedCommitment::Record(c)
            }
        });
        Ok(self)
    }
}
fn bounded(s: &str) -> Result<()> {
    ensure!(
        !s.trim().is_empty() && s.len() <= 4096,
        "evidence text must contain 1–4096 bytes"
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Response {
    Proceed,
    Verify,
    Clarify,
    Replan,
}
impl Response {
    pub fn guidance(self) -> &'static str {
        match self {
            Self::Proceed => "Review the action against the goal before proceeding; the recorded check establishes only what it tested.",
            Self::Verify => "Use the available check before committing to this action; its recorded cost fits the available time.",
            Self::Clarify => "Establish the missing evidence or clarify the commitment before proceeding; absence of a check result is not success.",
            Self::Replan => "Review the threatened commitment and choose a fallback or smaller scope; include the cost of delaying the action.",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assessment {
    pub kinds: Vec<Kind>,
    pub response: Response,
    /// No probabilistic confidence is invented from an unverified claim.
    pub uncertainty: bool,
    pub check_affordable: Option<bool>,
}

/// Deterministic, conditional concerns, not claims that an error or harm occurred.
pub fn assess(e: &Evidence, exposed: bool) -> Assessment {
    let uncertain = matches!(
        e.verification,
        Verification::Unknown | Verification::Pending
    );
    let unverified = e.verification != Verification::Passed;
    let affordable = if e.check_available {
        e.check_cost_secs
            .zip(e.time_available_secs)
            .map(|(cost, time)| cost <= time)
    } else {
        None
    };
    let threatened = e.budget_shortfall
        || affordable == Some(false)
        || (e.commitment.is_some() && e.time_available_secs == Some(0));
    let mut kinds = Vec::new();
    if e.commitment.is_some() && (unverified || threatened) {
        kinds.push(Kind::Guilt);
    }
    if exposed
        && unverified
        && (e.commitment.is_some()
            || e.expected_outcome.is_some()
            || matches!(e.verification, Verification::Pending | Verification::Failed))
    {
        kinds.push(Kind::Embarrassment);
    }
    // A named, feasible check is the supported alternative to proceeding unchecked.
    if unverified && affordable == Some(true) {
        kinds.push(Kind::Regret);
    }
    if e.expected_outcome.is_some() && (e.verification == Verification::Failed || threatened) {
        kinds.push(Kind::Disappointment);
    }
    if threatened || e.verification == Verification::Failed {
        kinds.push(Kind::Anxiety);
    }
    if uncertain && affordable == Some(true) {
        kinds.push(Kind::Curiosity);
    }
    let response = if threatened {
        Response::Replan
    } else if unverified && affordable == Some(true) {
        Response::Verify
    } else if unverified {
        Response::Clarify
    } else {
        Response::Proceed
    };
    Assessment {
        kinds,
        response,
        uncertainty: uncertain,
        check_affordable: affordable,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Harness,
    Owner,
}

/// Preserve future or malformed history records without interpreting them.
/// Strict owner input stays strict; unknown stored evidence stays reviewable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum History<T> {
    Known(T),
    Unknown(Value),
}
impl<T> History<T> {
    pub fn known(&self) -> Option<&T> {
        match self {
            Self::Known(v) => Some(v),
            Self::Unknown(_) => None,
        }
    }
    pub fn known_mut(&mut self) -> Option<&mut T> {
        match self {
            Self::Known(v) => Some(v),
            Self::Unknown(_) => None,
        }
    }
}
impl<T> From<T> for History<T> {
    fn from(value: T) -> Self {
        Self::Known(value)
    }
}

/// Immutable forecast of one exact action version. History lives beside the draft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prediction {
    pub id: String,
    pub created_at: String,
    pub args: Value,
    pub source: Source,
    pub evidence: Evidence,
    pub assessment: Assessment,
    /// Owner opt-in: unresolved evidence prevents release until reassessed.
    pub guide: bool,
}
impl Prediction {
    /// Re-evaluate remaining time without rewriting the original prediction.
    pub fn current_assessment(&self) -> Result<Assessment> {
        self.evidence.validate()?;
        let mut current = self.evidence.clone();
        if let Some(time) = &mut current.time_available_secs {
            let recorded = chrono::DateTime::parse_from_rfc3339(&self.created_at)?;
            let elapsed = chrono::Utc::now()
                .signed_duration_since(recorded)
                .num_seconds();
            ensure!(elapsed >= 0, "assessment timestamp is in the future");
            *time = time.saturating_sub(elapsed as u64);
        }
        Ok(assess(&current, true))
    }

    pub fn new(args: Value, evidence: Evidence, source: Source, guide: bool) -> Self {
        let assessment = assess(&evidence, true);
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            args,
            evidence,
            source,
            assessment,
            guide,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    ErrorExposed,
    Harm,
    ExpectationMissed,
    NoIssue,
    Withdrawn,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutcomeInput {
    pub prediction_id: String,
    pub verdict: Verdict,
    pub evidence: String,
    /// Only an explicit owner's attribution can establish mecha caused an impact.
    #[serde(default)]
    pub attributable_to_mecha: bool,
    #[serde(default)]
    pub supersedes: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub id: String,
    pub recorded_at: String,
    pub observation: OutcomeInput,
}
impl OutcomeInput {
    pub fn validate(&self) -> Result<()> {
        bounded(&self.evidence)?;
        ensure!(
            self.verdict != Verdict::Withdrawn || self.supersedes.is_some(),
            "withdrawal must name the outcome it supersedes"
        );
        Ok(())
    }
}

/// Status is per forecast. A changed/abandoned action isn't a false prediction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    Unsupported,
    Pending,
    Reassessed,
    Changed,
    Abandoned,
    DeliveryUnknown,
    AwaitingFeedback,
    Observed,
}

/// Immutable owner evidence for one invocation. An elapsed budget never restarts
/// at each plan update, and evidence for one confirmed goal cannot follow drift.
#[derive(Debug, Clone)]
pub struct BoundEvidence {
    evidence: Evidence,
    started: std::time::Instant,
}
impl BoundEvidence {
    pub fn new(evidence: Evidence) -> Result<Self> {
        // The new-write door: every draft this run stages inherits the one
        // commitment record, never the legacy shape (1f-2).
        let evidence = evidence.into_record()?;
        ensure!(
            evidence.goal.is_some(),
            "run evidence requires a confirmed goal"
        );
        Ok(Self {
            evidence,
            started: std::time::Instant::now(),
        })
    }
    pub fn snapshot(&self) -> Evidence {
        self.for_goal(self.evidence.goal.as_ref())
            .expect("bound goal matches itself")
    }
    pub fn for_goal(&self, goal: Option<&GoalRef>) -> Option<Evidence> {
        if goal != self.evidence.goal.as_ref() {
            return None;
        }
        let mut e = self.evidence.clone();
        if let Some(time) = &mut e.time_available_secs {
            *time = time.saturating_sub(self.started.elapsed().as_secs());
        }
        Some(e)
    }
    /// A check of prior context cannot certify a newly authored artifact.
    pub fn for_draft(&self, goal: Option<&GoalRef>) -> Option<Evidence> {
        self.for_goal(goal).map(|mut e| {
            e.verification = Verification::Unknown;
            e.verification_evidence = None;
            e
        })
    }
}

// ─── Calibration: every resolved prediction scored (X5, row 2b-1) ──────────

impl Response {
    /// Every variant, in the order a readout lists them.
    pub const ALL: [Response; 4] = [
        Response::Proceed,
        Response::Verify,
        Response::Clarify,
        Response::Replan,
    ];
}

impl Kind {
    /// Every variant, in the order a readout lists them.
    pub const ALL: [Kind; 6] = [
        Kind::Guilt,
        Kind::Embarrassment,
        Kind::Regret,
        Kind::Disappointment,
        Kind::Anxiety,
        Kind::Curiosity,
    ];
}

/// Why a prediction is not a calibration point — each [`Resolution`] short
/// of an observation, and one more: a clean outcome on a draft whose
/// delivery was never confirmed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Unscored {
    /// The draft has not gone out.
    pub pending: usize,
    /// Gone out, and the owner has recorded no outcome yet.
    pub awaiting_feedback: usize,
    /// A release whose delivery is still unknown.
    pub delivery_unknown: usize,
    /// The owner said nothing went wrong, but no delivery was ever
    /// confirmed — by the tool's acknowledgement or by `outbox reconcile` —
    /// so "it went out clean" is not established (X5: "a delivery positive
    /// is scored only after `outbox reconcile` has confirmed delivery").
    pub delivery_unconfirmed: usize,
    /// The draft changed after the prediction: it described another action.
    pub changed: usize,
    /// A later prediction replaced this one before release.
    pub reassessed: usize,
    /// The draft was rejected: an abandoned action is not a false forecast.
    pub abandoned: usize,
    /// The draft carries evidence this build cannot read.
    pub unsupported: usize,
}

/// Which [`Unscored`] counter a prediction that is not a point goes to.
type Why = fn(&mut Unscored);

/// One kind's calibration points and the predictions that are not yet one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Calibrated {
    /// Predictions of this kind the store holds.
    pub predictions: usize,
    /// Predictions an outcome resolved: calibration points.
    pub scored: usize,
    /// Of those, the concern materialised — the owner recorded an exposed
    /// error, a harm, or a missed expectation.
    pub materialized: usize,
    /// Of those, the owner recorded no issue, on a confirmed delivery.
    pub clean: usize,
    /// `materialized / scored`, and **`None` when nothing was scored** — a
    /// rate over nothing is not a calibration figure, and coverage is what
    /// a store with no outcomes has to report.
    pub materialized_rate: Option<f64>,
    pub unscored: Unscored,
}

impl Calibrated {
    fn add(&mut self, point: Option<bool>, why: Option<Why>) {
        self.predictions += 1;
        match point {
            Some(true) => {
                self.scored += 1;
                self.materialized += 1;
            }
            Some(false) => {
                self.scored += 1;
                self.clean += 1;
            }
            None => {
                if let Some(bump) = why {
                    bump(&mut self.unscored);
                }
            }
        }
        self.materialized_rate =
            (self.scored > 0).then(|| self.materialized as f64 / self.scored as f64);
    }
}

/// Every prediction in the outbox, scored where an outcome resolves it:
/// per response the assessment chose (did `proceed` drafts go out clean,
/// did `verify` drafts that skipped the check go badly) and per concern
/// kind it named. Coverage always; a rate only over points that exist.
///
/// **A point is the owner's recorded outcome, never a model's view.** The
/// prediction must be the one the draft was released under
/// (`OutboxItem::prediction_resolution` is `Observed`), the outcome the
/// active one for it, and a clean outcome counts only on a confirmed
/// delivery ([`crate::outbox::OutboxItem::delivery_confirmed`]). A concern
/// that materialised needs no such check: the owner saw the harm.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Calibration {
    /// Keyed by the response's wire name; every response is present, so a
    /// response nothing chose reads as zero predictions, not as missing.
    pub by_response: std::collections::BTreeMap<String, Calibrated>,
    /// Keyed by the concern kind's wire name; a prediction naming several
    /// kinds counts toward each. Every kind is present.
    pub by_kind: std::collections::BTreeMap<String, Calibrated>,
    /// Every readable prediction, once.
    pub total: Calibrated,
    /// Prediction records this build cannot read — in no count above.
    pub unreadable: usize,
}

impl Calibration {
    pub fn of<'a>(items: impl IntoIterator<Item = &'a crate::outbox::OutboxItem>) -> Calibration {
        use crate::appraisal::enum_name as name;
        let mut out = Calibration {
            by_response: Response::ALL
                .iter()
                .map(|r| (name(r), Calibrated::default()))
                .collect(),
            by_kind: Kind::ALL
                .iter()
                .map(|k| (name(k), Calibrated::default()))
                .collect(),
            ..Calibration::default()
        };
        for item in items {
            for record in &item.predictions {
                let Some(p) = record.known() else {
                    out.unreadable += 1;
                    continue;
                };
                let (point, why) = score(item, p);
                out.total.add(point, why);
                if let Some(t) = out.by_response.get_mut(&name(&p.assessment.response)) {
                    t.add(point, why);
                }
                let mut kinds = p.assessment.kinds.clone();
                kinds.sort_by_key(name);
                kinds.dedup();
                for k in kinds {
                    if let Some(t) = out.by_kind.get_mut(&name(&k)) {
                        t.add(point, why);
                    }
                }
            }
        }
        out
    }
}

/// One prediction: `Some(true)` the concern materialised, `Some(false)` it
/// went out clean, `None` not a point — with the counter that says why.
fn score(item: &crate::outbox::OutboxItem, p: &Prediction) -> (Option<bool>, Option<Why>) {
    let not = |f: Why| (None, Some(f));
    match item.prediction_resolution(p) {
        Resolution::Observed => {}
        Resolution::Pending => return not(|u| u.pending += 1),
        Resolution::AwaitingFeedback => return not(|u| u.awaiting_feedback += 1),
        Resolution::DeliveryUnknown => return not(|u| u.delivery_unknown += 1),
        Resolution::Changed => return not(|u| u.changed += 1),
        Resolution::Reassessed => return not(|u| u.reassessed += 1),
        Resolution::Abandoned => return not(|u| u.abandoned += 1),
        Resolution::Unsupported => return not(|u| u.unsupported += 1),
    }
    let Some(outcome) = item
        .active_outcomes()
        .into_iter()
        .find(|o| o.observation.prediction_id == p.id)
    else {
        return not(|u| u.awaiting_feedback += 1);
    };
    match outcome.observation.verdict {
        Verdict::ErrorExposed | Verdict::Harm | Verdict::ExpectationMissed => (Some(true), None),
        Verdict::NoIssue if item.delivery_confirmed() => (Some(false), None),
        Verdict::NoIssue => not(|u| u.delivery_unconfirmed += 1),
        // `active_outcomes` never returns a withdrawal; if it ever did, it
        // resolves nothing.
        Verdict::Withdrawn => not(|u| u.awaiting_feedback += 1),
    }
}

fn strict_goal<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<GoalRef>, D::Error> {
    Option::<String>::deserialize(d)?
        .map(|s| s.parse().map_err(serde::de::Error::custom))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn evidence() -> Evidence {
        Evidence {
            goal: Some(GoalRef::Task("meeting".into())),
            commitment: Some(
                Commitment {
                    beneficiary: "attendees".into(),
                    expectation: "send the confirmed time".into(),
                    consequence: "attendees miss the meeting".into(),
                }
                .into(),
            ),
            check_available: true,
            check_cost_secs: Some(10),
            time_available_secs: Some(60),
            expected_outcome: Some("correct invitation".into()),
            ..Evidence::default()
        }
    }
    #[test]
    fn guilt_requires_a_commitment_and_affordable_checks_support_regret_and_curiosity() {
        let mut e = evidence();
        let a = assess(&e, true);
        assert_eq!(
            a.kinds,
            vec![
                Kind::Guilt,
                Kind::Embarrassment,
                Kind::Regret,
                Kind::Curiosity
            ]
        );
        assert_eq!(a.response, Response::Verify);
        e.commitment = None;
        assert!(!assess(&e, true).kinds.contains(&Kind::Guilt));
        e.time_available_secs = None;
        let a = assess(&e, true);
        assert!(!a.kinds.contains(&Kind::Regret));
        assert!(!a.kinds.contains(&Kind::Curiosity));
        assert_eq!(a.check_affordable, None);
    }
    #[test]
    fn delay_cost_can_outweigh_checking_and_a_pass_never_waives_a_threatened_commitment() {
        let mut e = evidence();
        e.time_available_secs = Some(1);
        let a = assess(&e, true);
        assert_eq!(a.response, Response::Replan);
        assert!(a.kinds.contains(&Kind::Anxiety));
        assert!(a.kinds.contains(&Kind::Disappointment));
        e.verification = Verification::Passed;
        assert!(assess(&e, true).kinds.contains(&Kind::Guilt));
        e.time_available_secs = Some(60);
        assert_eq!(assess(&e, true).response, Response::Proceed);
    }
    #[test]
    fn verdicts_need_evidence_and_input_cannot_supply_labels() {
        let mut e = evidence();
        e.verification = Verification::Passed;
        assert!(e.validate().is_err());
        e.verification_evidence = Some("calendar receipt".into());
        e.validate().unwrap();
        assert!(serde_json::from_str::<Evidence>(r#"{"label":"guilt"}"#).is_err());
        assert!(serde_json::from_str::<Evidence>(r#"{"verification":"future"}"#).is_err());
    }

    /// The new-write door (1f-2, ruling (b)): an owner's evidence file in
    /// the legacy shape becomes the one commitment record — the beneficiary
    /// as the party, the goal pointer as the source, no date — and the
    /// assessment reads it exactly as it read the legacy shape.
    #[test]
    fn a_legacy_commitment_becomes_the_record_at_the_door_with_its_goal_and_no_date() {
        let input: Evidence = serde_json::from_str(
            r#"{"goal":"task:meeting","commitment":{"beneficiary":"attendees","expectation":"send the confirmed time","consequence":"attendees miss the meeting"},"check_available":true,"check_cost_secs":10,"time_available_secs":60}"#,
        )
        .unwrap();
        assert!(matches!(
            input.commitment,
            Some(RecordedCommitment::Legacy(_))
        ));
        let recorded = input.clone().into_record().unwrap();
        let Some(RecordedCommitment::Record(c)) = &recorded.commitment else {
            panic!("a new write is the record: {:?}", recorded.commitment);
        };
        assert_eq!(c.party, "attendees");
        assert_eq!(c.source, "task:meeting", "the goal pointer, never text");
        assert_eq!((c.due_at, c.follow_up_at), (None, None), "no derived date");
        assert_eq!(c.expectation.as_deref(), Some("send the confirmed time"));
        assert_eq!(c.consequence.as_deref(), Some("attendees miss the meeting"));
        assert_eq!(assess(&recorded, true), assess(&input, true));

        let json = serde_json::to_value(&recorded).unwrap();
        assert!(json["commitment"].get("beneficiary").is_none(), "{json}");
        assert!(json["commitment"].get("due_at").is_none(), "{json}");
        // Written in the record's shape, it reads back as the record.
        let back: Evidence = serde_json::from_value(json).unwrap();
        assert_eq!(back, recorded);
        // And the door is idempotent.
        assert_eq!(back.clone().into_record().unwrap(), back);

        // Through `BoundEvidence` too — the door every staged draft of a
        // run comes through.
        let bound = BoundEvidence::new(input).unwrap();
        assert!(matches!(
            bound.snapshot().commitment,
            Some(RecordedCommitment::Record(_))
        ));
        let goal = GoalRef::Task("meeting".into());
        assert!(matches!(
            bound.for_draft(Some(&goal)).unwrap().commitment,
            Some(RecordedCommitment::Record(_))
        ));
    }

    /// Owner evidence stays strict in the new shape: an unknown key is
    /// refused, a record must say what is expected and what failing costs,
    /// and its source must be the goal it is bound to — so no new write
    /// carries a pointer anything but the harness chose.
    #[test]
    fn a_record_given_as_input_is_strict_and_must_point_at_its_goal() {
        let with = |commitment: &str| {
            serde_json::from_str::<Evidence>(&format!(
                r#"{{"goal":"task:meeting","commitment":{commitment}}}"#
            ))
        };
        assert!(
            with(r#"{"party":"attendees","source":"task:meeting","expectation":"e","consequence":"c","deadline":"soon"}"#)
                .is_err(),
            "an unknown key in the record is refused, not dropped"
        );
        assert!(
            with(r#"{"beneficiary":"attendees","expectation":"e","consequence":"c","extra":1}"#)
                .is_err(),
            "and in the legacy shape, as it always was"
        );
        let good = with(
            r#"{"party":"attendees","source":"task:meeting","expectation":"e","consequence":"c"}"#,
        )
        .unwrap();
        good.clone().into_record().unwrap();
        let elsewhere = with(
            r#"{"party":"attendees","source":"mail thread 7","expectation":"e","consequence":"c"}"#,
        )
        .unwrap();
        assert!(
            elsewhere.into_record().is_err(),
            "source is the goal pointer"
        );
        let bare = with(r#"{"party":"attendees","source":"task:meeting"}"#).unwrap();
        assert!(
            bare.validate().is_err(),
            "expectation and consequence are named"
        );
        let empty =
            with(r#"{"party":" ","source":"task:meeting","expectation":"e","consequence":"c"}"#)
                .unwrap();
        assert!(empty.validate().is_err(), "bounded like the legacy fields");
    }

    /// Dates are only ever the owner's (ruled on review of #304): a record
    /// the owner wrote with `due_at` / `follow_up_at` keeps exactly those
    /// through the door and through a run's bound evidence, written back
    /// byte-for-byte; the legacy shape still gets none; and a follow-up
    /// after the deadline is refused rather than reordered.
    #[test]
    fn an_owner_written_date_on_a_record_passes_through_the_door_unchanged() {
        let dated = r#"{"party":"attendees","source":"task:meeting","due_at":"2026-10-02T17:00:00Z","follow_up_at":"2026-10-01T09:00:00Z","expectation":"send the confirmed time","consequence":"attendees miss the meeting"}"#;
        let input: Evidence = serde_json::from_str(&format!(
            r#"{{"goal":"task:meeting","commitment":{dated}}}"#
        ))
        .unwrap();
        let recorded = input.clone().into_record().unwrap();
        assert_eq!(recorded, input, "the door changes nothing on it");
        let Some(RecordedCommitment::Record(c)) = &recorded.commitment else {
            panic!("{:?}", recorded.commitment);
        };
        assert_eq!(
            (c.due_at, c.follow_up_at),
            (
                Some("2026-10-02T17:00:00Z".parse().unwrap()),
                Some("2026-10-01T09:00:00Z".parse().unwrap())
            )
        );
        assert_eq!(
            serde_json::to_string(recorded.commitment.as_ref().unwrap()).unwrap(),
            dated,
            "written back exactly as the owner wrote it"
        );
        let bound = BoundEvidence::new(input).unwrap();
        assert_eq!(bound.snapshot().commitment, recorded.commitment);

        // The legacy shape has no date to carry, and the door adds none.
        let legacy: Evidence = serde_json::from_str(
            r#"{"goal":"task:meeting","commitment":{"beneficiary":"attendees","expectation":"e","consequence":"c"}}"#,
        )
        .unwrap();
        let Some(RecordedCommitment::Record(c)) = legacy.into_record().unwrap().commitment else {
            panic!("a legacy commitment becomes the record");
        };
        assert_eq!((c.due_at, c.follow_up_at), (None, None));

        // A follow-up after the deadline is refused, never reordered.
        let backwards: Evidence = serde_json::from_str(
            r#"{"goal":"task:meeting","commitment":{"party":"attendees","source":"task:meeting","due_at":"2026-10-01T09:00:00Z","follow_up_at":"2026-10-02T17:00:00Z","expectation":"e","consequence":"c"}}"#,
        )
        .unwrap();
        assert!(backwards.into_record().is_err());
    }
}
