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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Commitment {
    pub beneficiary: String,
    pub expectation: String,
    /// The adverse consequence of failing this commitment, as judged by the owner.
    pub consequence: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Evidence {
    #[serde(default, deserialize_with = "strict_goal")]
    pub goal: Option<GoalRef>,
    pub commitment: Option<Commitment>,
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
            for s in [&c.beneficiary, &c.expectation, &c.consequence] {
                bounded(s)?;
            }
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

/// Immutable forecast of one exact action version. History lives beside the draft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
        evidence.validate()?;
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
            commitment: Some(Commitment {
                beneficiary: "attendees".into(),
                expectation: "send the confirmed time".into(),
                consequence: "attendees miss the meeting".into(),
            }),
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
}
