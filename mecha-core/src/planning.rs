//! Structured planning evidence. Sensor values stay in local metadata, never prompts.
use crate::{goal::GoalRef, reading::LineReading};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Feedback {
    #[serde(default)]
    pub steps: Vec<StepFeedback>,
    #[serde(default)]
    pub decisions: Vec<Decision>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verification {
    #[default]
    NotDeclared,
    Pending,
    Passed,
    Failed,
    Refused,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepFeedback {
    /// Owner-bound task criterion; never a model-authored grading command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criterion: Option<crate::mismatch::CriterionFeedback>,
    /// More than one newly completed step makes the work boundary ambiguous.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_batch: Option<u32>,
    #[serde(default)]
    pub call_id: Option<String>,
    pub step: String,
    #[serde(default, deserialize_with = "crate::goal::de_lenient")]
    pub goal: Option<GoalRef>,
    pub expected: Option<String>,
    pub expected_calls: Option<u32>,
    pub actual_calls: Option<u32>,
    pub verification: Verification,
    #[serde(default)]
    pub check_tampered: bool,
}
impl StepFeedback {
    /// An estimate is evidence only when declared before completion and its
    /// work span is unambiguous. The threshold avoids learning from small noise.
    pub fn forecast_miss(&self) -> bool {
        matches!((self.expected_calls, self.actual_calls), (Some(e), Some(a)) if a >= e.saturating_mul(3).max(6))
    }
    pub fn goals(&self) -> Vec<GoalRef> {
        self.goal
            .iter()
            .cloned()
            .chain(
                self.criterion
                    .as_ref()
                    .and_then(|c| c.context.as_ref())
                    .and_then(|c| c.constraint.charter_goal.as_ref())
                    .cloned(),
            )
            .collect()
    }
    pub fn learnable_failure(&self) -> bool {
        self.verification == Verification::Failed || self.check_tampered
    }
    /// Observations identify an error class, not its causal explanation.
    pub fn attribution(&self) -> &'static str {
        if self.criterion.is_some() && self.verification == Verification::Failed {
            "task_criterion_failed"
        } else if self.check_tampered {
            "check_tampered"
        } else if self.verification == Verification::Failed {
            "declared_check_failed"
        } else if self.forecast_miss() && self.completion_batch.is_some_and(|n| n > 1) {
            "forecast_overrun_with_batched_completion"
        } else if self.forecast_miss() {
            "forecast_overrun_cause_unknown"
        } else {
            "no_attributed_failure"
        }
    }
    pub fn mismatch(&self) -> bool {
        self.verification == Verification::Failed || self.forecast_miss() || self.check_tampered
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Gap {
    #[serde(default, deserialize_with = "crate::goal::de_lenient")]
    pub goal: Option<GoalRef>,
    /// Remaining sensor discrepancy; unknown is not zero. No causal credit.
    pub remaining: Option<f32>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    ClarifyGoal,
    ReviewCommitment,
    GatherContext,
    Verify,
    Replan,
    Continue,
    Complete,
}
impl Action {
    /// Fixed vocabulary is the only part of a sensor decision allowed into a prompt.
    pub fn guidance(self) -> &'static str {
        match self {
            Self::ClarifyGoal => "Planning guidance: reconcile this plan with the owner's confirmed goal before expanding its scope.",
            Self::ReviewCommitment => "Planning guidance: review the relevant charter commitment before choosing further work.",
            Self::GatherContext => "Planning guidance: obtain missing evidence before assuming the relevant commitment is satisfied.",
            Self::Verify => "Planning guidance: establish evidence for completed steps before claiming the goal is achieved.",
            Self::Replan => "Planning guidance: reduce or split the remaining work to fit the available context.",
            Self::Continue => "Planning guidance: continue the next step toward the named goal.",
            Self::Complete => "Planning guidance: review the completed plan against the goal's acceptance criteria.",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    #[serde(default, deserialize_with = "crate::goal::de_lenient")]
    pub goal: Option<GoalRef>,
    #[serde(default, deserialize_with = "crate::goal::de_lenient")]
    pub anchor: Option<GoalRef>,
    pub open_steps: usize,
    pub unverified_steps: usize,
    /// Charter file order is priority order. Never reduced to an affect score.
    #[serde(default)]
    pub charter_observed: bool,
    pub charter: Vec<Gap>,
    pub action: Action,
    pub applied: bool,
}
impl Decision {
    pub fn assess(
        plan: &crate::tool::todo::Plan,
        anchor: Option<GoalRef>,
        readings: Option<&[LineReading]>,
        turns_left: Option<u64>,
        verified: &std::collections::HashSet<String>,
        applied: bool,
    ) -> Self {
        use crate::tool::todo::Status;
        let open_steps = plan
            .items
            .iter()
            .filter(|i| i.status != Status::Completed)
            .count();
        // A status claim is not evidence. The check executor updates verification
        // separately; a newly completed plan must still inspect that evidence.
        let unverified_steps = plan
            .items
            .iter()
            .filter(|i| {
                i.status == Status::Completed
                    && !verified.contains(&verification_key(plan.goal.as_ref(), &i.content))
            })
            .count();
        let charter: Vec<Gap> = readings
            .unwrap_or_default()
            .iter()
            .map(|r| Gap {
                goal: Some(GoalRef::Charter(r.line.clone())),
                remaining: r.reading.excess(),
            })
            .collect();
        let charter_action = charter.iter().find_map(|g| match g.remaining {
            None => Some(Action::GatherContext),
            Some(e) if e > 0.0 => Some(Action::ReviewCommitment),
            _ => None,
        });
        let action = if anchor.is_some() && anchor != plan.goal {
            Action::ClarifyGoal
        } else if let Some(a) = charter_action {
            a
        } else if unverified_steps > 0 {
            Action::Verify
        } else if turns_left.is_some_and(|n| open_steps as u64 > n) {
            Action::Replan
        } else if open_steps > 0 {
            Action::Continue
        } else {
            Action::Complete
        };
        Self {
            goal: plan.goal.clone(),
            anchor,
            open_steps,
            unverified_steps,
            charter_observed: readings.is_some(),
            charter,
            action,
            applied,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Lesson {
    pub goal: GoalRef,
    pub text: String,
    pub source: String,
}

/// Forward records drop only the unread metadata, never the conversation.
pub fn de_lenient<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Feedback>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(v).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forecasts_and_failed_checks_are_distinct_from_missing_evidence() {
        let mut s = StepFeedback {
            criterion: None,
            completion_batch: None,
            call_id: None,
            step: "work".into(),
            goal: None,
            expected: None,
            expected_calls: Some(2),
            actual_calls: None,
            verification: Verification::Skipped,
            check_tampered: false,
        };
        assert!(!s.mismatch());
        s.actual_calls = Some(6);
        assert!(s.forecast_miss());
        s.actual_calls = Some(2);
        s.verification = Verification::Failed;
        assert!(s.mismatch());
        s.verification = Verification::Refused;
        assert!(!s.mismatch());
    }
    #[test]
    fn unknown_higher_ranked_commitments_are_not_outvoted_by_lower_sensors() {
        use crate::{
            charter::SensorKind,
            reading::{Observed, Reading},
        };
        let mut readings = vec![
            LineReading {
                line: "first".into(),
                kind: SensorKind::OutboxAge,
                setpoint: "1h".into(),
                reading: Reading::Unread,
            },
            LineReading {
                line: "second".into(),
                kind: SensorKind::OutboxAge,
                setpoint: "1h".into(),
                reading: Reading::Observed {
                    value: Observed::Seconds(7200),
                    over: true,
                    excess: 0.5,
                },
            },
        ];
        let plan = crate::tool::todo::Plan::default();
        let d = Decision::assess(
            &plan,
            None,
            Some(&readings),
            None,
            &Default::default(),
            false,
        );
        assert_eq!(d.action, Action::GatherContext);
        assert_eq!(d.charter[0].remaining, None);
        assert!(!d.applied);
        readings[0].reading = Reading::Nothing;
        assert_eq!(
            Decision::assess(
                &plan,
                None,
                Some(&readings),
                None,
                &Default::default(),
                false
            )
            .action,
            Action::ReviewCommitment
        );
        assert_eq!(
            Decision::assess(
                &plan,
                Some(GoalRef::Task("t1".into())),
                Some(&readings),
                None,
                &Default::default(),
                true
            )
            .action,
            Action::ClarifyGoal
        );
        assert!(!Action::ReviewCommitment.guidance().contains("7200"));
    }
}

pub fn verification_key(goal: Option<&GoalRef>, step: &str) -> String {
    format!(
        "{}\0{}",
        goal.map(ToString::to_string).unwrap_or_default(),
        step
    )
}

#[derive(Debug, Clone)]
pub struct Example {
    pub goal: GoalRef,
    pub step: String,
    pub expected: Option<String>,
    pub source: String,
}

/// A bounded startup snapshot, reused on demand. Unread, large, unscoped, and
/// tainted episodes contribute no examples. A passing check proves only the
/// declared check at that moment, never the whole task or current artifacts.
pub fn examples(
    dir: &std::path::Path,
    situation: &crate::situation::Situation,
) -> anyhow::Result<Vec<Example>> {
    let mut out = Vec::new();
    for (meta, path) in crate::session::Session::list(dir)?.into_iter().take(32) {
        if !std::fs::metadata(&path).is_ok_and(|m| m.len() <= 2_000_000) {
            continue;
        }
        let Ok(t) = crate::session::Session::read(&path) else {
            continue;
        };
        let mut seen = std::collections::HashSet::new();
        for (at, message) in t.convo.messages.iter().enumerate().rev() {
            let Some(feedback) = &message.planning else {
                continue;
            };
            let current: Vec<_> = feedback
                .steps
                .iter()
                .rev()
                .filter(|s| seen.insert(verification_key(s.goal.as_ref(), &s.step)))
                .collect();
            if crate::learning::classify_origin(t.taint_timeline.covering(at))
                != crate::learning::Origin::Clean
            {
                continue;
            }
            let Some(config) = t.config_covering(at) else {
                continue;
            };
            let Some(workspace) = &config.rules_workspace else {
                continue;
            };
            let Some(surface) = config.rules_surface else {
                continue;
            };
            let scope = crate::situation::Situation::of_run(&config.tools, Some(workspace))
                .on(Some(surface));
            if !scope.matches(situation) {
                continue;
            }
            for step in current
                .into_iter()
                .filter(|s| s.verification == Verification::Passed && !s.check_tampered)
            {
                let Some(goal) = &step.goal else { continue };
                out.push(Example {
                    goal: goal.clone(),
                    step: step.step.clone(),
                    expected: step.expected.clone(),
                    source: meta.id.clone(),
                });
                if out.len() >= 64 {
                    return Ok(out);
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod attribution_tests {
    use super::*;
    #[test]
    fn batch_boundaries_are_distinct_from_a_known_failure() {
        let mut s:StepFeedback=serde_json::from_value(serde_json::json!({"step":"read head","expected_calls":1,"actual_calls":13,"verification":"not_declared"})).unwrap();
        assert_eq!(s.attribution(), "forecast_overrun_cause_unknown");
        assert!(!s.learnable_failure());
        s.completion_batch = Some(3);
        assert_eq!(s.attribution(), "forecast_overrun_with_batched_completion");
        s.verification = Verification::Failed;
        assert_eq!(s.attribution(), "declared_check_failed");
        assert!(s.learnable_failure());
    }
}
