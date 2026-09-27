//! Structured planning evidence. Live sensor values stay in local metadata.
//! Owner-bound criterion snapshots may reach the quarantined post-run reflector.

/// Frozen harness voice, also recognized in historical learning records.
pub const CRITERION_OBSERVATION: &str = "Harness observations of owner-bound task criteria.";
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
    /// Remaining sensor discrepancy — the level's `excess`, recorded as
    /// evidence; unknown is not zero. No causal credit. Not what the action
    /// is decided on where the line has a per-item reading (`items_over`)
    /// or per-commitment guilt (`guilt`).
    pub remaining: Option<f32>,
    /// How many of the line's items were past the setpoint as the run
    /// began (`reading::Items::over`) — the per-item form the action is
    /// decided on (S5) for a count kind. `None` for a kind with no items
    /// and on a record from before the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items_over: Option<u64>,
    /// The largest guilt among the pending commitments whose patience this
    /// line sets (`guilt::StoreGuilt::max`, S7) — recorded beside the
    /// decision, which is made per commitment: an age kind's line is a
    /// commitment to review when any commitment it weighs is past its
    /// patience. `None` for a line that sets no store's patience, where
    /// the maximum is unknown, and on a record from before the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guilt: Option<f32>,
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
    /// Local evidence behind the assessment; provider adapters omit planning metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anticipation_evidence: Option<crate::anticipation::Evidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anticipation: Option<crate::anticipation::Assessment>,
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
    pub fn with_owner_evidence(&mut self, bound: &crate::anticipation::BoundEvidence) {
        // An unnamed or drifted plan is handled by goal-alignment guidance first.
        if self.goal != self.anchor {
            return;
        }
        if let Some(mut evidence) = bound.for_goal(self.anchor.as_ref()) {
            if let Some(harness) = &self.anticipation_evidence {
                evidence.budget_shortfall |= harness.budget_shortfall;
                if evidence.expected_outcome.is_none() {
                    evidence.expected_outcome = harness.expected_outcome.clone();
                }
                // The owner's external check cannot certify unfinished plan verification.
                if harness.verification == crate::anticipation::Verification::Pending
                    && matches!(
                        evidence.verification,
                        crate::anticipation::Verification::Unknown
                            | crate::anticipation::Verification::Passed
                    )
                {
                    evidence.verification = crate::anticipation::Verification::Pending;
                    evidence.verification_evidence = None;
                }
            }
            let assessment = crate::anticipation::assess(&evidence, false);
            if !matches!(
                self.action,
                Action::ClarifyGoal | Action::ReviewCommitment | Action::GatherContext
            ) {
                use crate::anticipation::Response;
                self.action = match assessment.response {
                    Response::Verify => Action::Verify,
                    Response::Clarify if self.action == Action::Verify => Action::Verify,
                    Response::Clarify => Action::GatherContext,
                    Response::Replan => Action::Replan,
                    Response::Proceed => self.action,
                };
            }
            self.anticipation = Some(assessment);
            self.anticipation_evidence = Some(evidence);
        }
    }

    pub fn assess(
        plan: &crate::tool::todo::Plan,
        anchor: Option<GoalRef>,
        readings: Option<&[LineReading]>,
        turns_left: Option<u64>,
        verified: &std::collections::HashSet<String>,
        applied: bool,
        commitments: Option<&[crate::guilt::StoreGuilt]>,
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
        // The commitments whose patience a line sets — exactly one store per
        // age kind's line (`doctor::Patience::for_store`), none for a count
        // or a rate.
        let weighed = |line: &str| {
            commitments
                .unwrap_or_default()
                .iter()
                .find(|s| s.line.as_deref() == Some(line))
        };
        let charter: Vec<Gap> = readings
            .unwrap_or_default()
            .iter()
            .map(|r| Gap {
                goal: Some(GoalRef::Charter(r.line.clone())),
                remaining: r.reading.excess(),
                items_over: r.items.as_ref().map(|i| i.over),
                guilt: weighed(&r.line).and_then(|s| s.max()),
            })
            .collect();
        // Per commitment, never the level (S5, S7): a line is a commitment
        // to review when a pending commitment it weighs is past its
        // patience — the same per-item guilt every guilt consumer reads,
        // and never the `anticipated_guilt` readout. Unknown is still
        // unknown — a reading that says nothing asks for context first. A
        // count kind, whose line sets no commitment's patience, is judged
        // on its items over the count; the corpus rate, which has no items
        // at all, on its level, because for it the level is the only form
        // there is. A caller with no per-commitment record (a snapshot
        // from before it) reads the per-item count, which the guilt agrees
        // with for every age kind: both are "an item older than the
        // setpoint".
        let charter_action =
            readings
                .unwrap_or_default()
                .iter()
                .zip(&charter)
                .find_map(
                    |(r, g)| match (g.remaining, weighed(&r.line), g.items_over) {
                        (None, _, _) => Some(Action::GatherContext),
                        (Some(_), Some(store), _) => {
                            store.any_owed().then_some(Action::ReviewCommitment)
                        }
                        (Some(_), None, Some(n)) if n > 0 => Some(Action::ReviewCommitment),
                        (Some(_), None, Some(_)) => None,
                        (Some(e), None, None) if e > 0.0 => Some(Action::ReviewCommitment),
                        _ => None,
                    },
                );
        // Only against an anchor a plan could have named (`GoalRef::
        // a_plan_can_name`): under a trigger or request anchor every plan
        // goal differs by construction, and asking to reconcile it would
        // fire on every plan write.
        let action = if anchor.as_ref().is_some_and(|a| a.a_plan_can_name()) && anchor != plan.goal
        {
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
        let prediction_evidence = crate::anticipation::Evidence {
            goal: anchor.clone(),
            verification: if unverified_steps > 0 {
                crate::anticipation::Verification::Pending
            } else {
                crate::anticipation::Verification::Unknown
            },
            expected_outcome: plan.items.iter().find_map(|i| i.expect.clone()),
            // A declaration alone does not establish availability or cost of a check.
            budget_shortfall: turns_left.is_some_and(|n| open_steps as u64 > n),
            ..crate::anticipation::Evidence::default()
        };
        Self {
            anticipation: Some(crate::anticipation::assess(&prediction_evidence, false)),
            anticipation_evidence: Some(prediction_evidence),
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
                items: None,
                delta: None,
                withdrawn: false,
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
                items: None,
                delta: None,
                withdrawn: false,
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
            None,
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
                false,
                None,
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
                true,
                None,
            )
            .action,
            Action::ClarifyGoal
        );
        // A trigger anchor is one no plan can name, so a plan under it is
        // never asked to reconcile with it (review of #292): the decision
        // falls through to the charter reading, as with no anchor at all.
        assert_eq!(
            Decision::assess(
                &plan,
                Some(GoalRef::Trigger("morning".into())),
                Some(&readings),
                None,
                &Default::default(),
                true,
                None,
            )
            .action,
            Action::ReviewCommitment
        );
        assert!(!Action::ReviewCommitment.guidance().contains("7200"));
    }

    /// `ReviewCommitment` reads per-commitment guilt (S7): the line whose
    /// patience weighs a store is a commitment to review when a pending
    /// commitment there is past its patience — and not when the level says
    /// "over" but the per-commitment record says nothing is owed, nor on
    /// the `anticipated_guilt` readout, which it is never handed. A count
    /// line, which weighs no commitment, keeps its per-item count.
    #[test]
    fn review_commitment_reads_each_commitments_guilt_not_the_level() {
        use crate::{
            charter::SensorKind,
            guilt::{ItemGuilt, Store, StoreGuilt},
            reading::{Items, Observed, Reading},
        };
        let over = |line: &str, kind| LineReading {
            line: line.into(),
            kind,
            setpoint: "24h".into(),
            reading: Reading::Observed {
                value: Observed::Seconds(200_000),
                over: true,
                excess: 0.6,
            },
            items: Some(Items {
                waiting: 2,
                over: 1,
                ..Default::default()
            }),
            delta: None,
            withdrawn: false,
        };
        let store = |guilts: &[Option<f32>]| StoreGuilt {
            store: Store::Outbox,
            line: Some("replies".into()),
            patience: "24h".into(),
            weight: 1.0,
            waiting: Some(guilts.len() as u64),
            unknown: guilts.iter().filter(|g| g.is_none()).count() as u64,
            items: guilts
                .iter()
                .enumerate()
                .map(|(i, g)| ItemGuilt {
                    id: format!("d{i}"),
                    age_secs: g.map(|_| 1),
                    guilt: *g,
                })
                .collect(),
        };
        let plan = crate::tool::todo::Plan::default();
        let assess = |readings: &[LineReading], stores: &[StoreGuilt]| {
            Decision::assess(
                &plan,
                None,
                Some(readings),
                None,
                &Default::default(),
                false,
                Some(stores),
            )
        };
        let replies = [over("replies", SensorKind::OutboxAge)];

        // One draft past its patience: review, and the line records the
        // largest guilt it weighs.
        let d = assess(&replies, &[store(&[Some(0.4), Some(0.0)])]);
        assert_eq!(d.action, Action::ReviewCommitment);
        assert_eq!(d.charter[0].guilt, Some(0.4));

        // Nothing owed per commitment: no review, whatever the level and
        // the per-item count say. Under the per-item reading this was a
        // review — the count said one item was over.
        let d = assess(&replies, &[store(&[Some(0.0), Some(0.0)])]);
        assert_eq!(d.action, Action::Complete);
        assert_eq!(d.charter[0].guilt, Some(0.0));

        // A known overdue draft beside an undated one: still owed, and the
        // line's maximum is unknown rather than the known one.
        let d = assess(&replies, &[store(&[None, Some(0.3)])]);
        assert_eq!(d.action, Action::ReviewCommitment);
        assert_eq!(d.charter[0].guilt, None);

        // A count line weighs no commitment: its items over the count.
        let queue = [over("queue", SensorKind::OutboxWaiting)];
        let d = assess(&queue, &[store(&[Some(0.0)])]);
        assert_eq!(d.action, Action::ReviewCommitment);
        assert_eq!(d.charter[0].guilt, None);
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
    /// A passed plan step's text, or — for an owner-verified success — the
    /// session's tool sequence ([`tool_sequence`], R40).
    pub step: String,
    pub expected: Option<String>,
    pub source: String,
    /// The owner's act that verified the session, for a success example
    /// ([`success_examples`]); `None` for a passed declared check, whose
    /// rendering is the bytes it was before success examples existed.
    pub owner_act: Option<crate::success::Act>,
}

/// A bounded startup snapshot, reused on demand. Unread, large, unscoped, and
/// tainted episodes contribute no examples. A passing check proves only the
/// declared check at that moment, never the whole task or current artifacts.
pub fn examples(
    dir: &std::path::Path,
    situation: &crate::situation::Situation,
) -> anyhow::Result<Vec<Example>> {
    let mut out = Vec::new();
    let population = crate::runlog::Scan::default();
    for (meta, path) in crate::session::Session::list(dir)?
        .into_iter()
        .filter(|(meta, _)| population.admits(meta))
        .take(32)
    {
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
            // The goal the example's run was matched toward joins its
            // scope like the other keys: a past run toward one goal is an
            // example only for a run toward the same goal, and one toward
            // none — or from before the field — for every run, as before.
            let scope = crate::situation::Situation::of_run(&config.tools, Some(workspace))
                .on(Some(surface))
                .toward(config.rules_goal.clone());
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
                    owner_act: None,
                });
                if out.len() >= 64 {
                    return Ok(out);
                }
            }
        }
    }
    Ok(out)
}

// ─── Success examples (row 2e-4b-1, R40) ────────────────────────────────

/// At most this many transcripts are read for success examples, newest
/// success first — [`examples`]' own window.
const SUCCESS_SESSIONS_READ: usize = 32;
/// At most this many success examples are kept — [`examples`]' own cap.
const SUCCESS_EXAMPLES_KEPT: usize = 64;
/// A transcript larger than this lends no example, as in [`examples`].
const SUCCESS_TRANSCRIPT_BYTES: u64 = 2_000_000;
/// How many steps of a tool sequence are spelled out.
const SEQUENCE_STEPS: usize = 24;

/// The tools a session called, in order: registry names only — never an
/// argument, never prose — with a consecutive repeat folded into `name ×n`
/// and the harness's own calls left out. `None` when the session called no
/// tool, which leaves nothing to plan from. The step of a planning success
/// example (R40): 4 of 79 long runs wrote a plan, so a plan step would
/// supply almost nothing, and the call trace is what every run has.
pub fn tool_sequence(messages: &[crate::message::Message]) -> Option<String> {
    let mut runs: Vec<(String, usize)> = Vec::new();
    for m in messages
        .iter()
        .filter(|m| m.role == crate::message::Role::Assistant && !m.harness)
    {
        for (_, name, _) in m.tool_uses() {
            match runs.last_mut() {
                Some((last, n)) if last == name => *n += 1,
                _ => runs.push((name.to_string(), 1)),
            }
        }
    }
    if runs.is_empty() {
        return None;
    }
    let more = runs.len().saturating_sub(SEQUENCE_STEPS);
    let mut parts: Vec<String> = runs
        .into_iter()
        .take(SEQUENCE_STEPS)
        .map(|(name, n)| if n > 1 { format!("{name} ×{n}") } else { name })
        .collect();
    if more > 0 {
        parts.push(format!("… {more} more"));
    }
    Some(parts.join(" → "))
}

/// Why a standing success, or one session it names, lends no planning
/// example — said by the owner's readout, never a quiet empty list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Withheld {
    /// The success names no goal (a draft sent unchanged, a workflow bound
    /// to no task, a question asked toward none), and an example is served
    /// only toward its goal.
    NoGoal,
    /// The success names no session.
    NoSession,
    /// The session is not one the corpus admits (a smoke test named beside
    /// a real one), or is not in the store.
    NotAdmitted,
    /// Larger than [`examples`]' bound.
    TooLarge,
    /// The transcript did not read.
    Unread,
    /// Its recorded taint is not provably clean — third-party content, or
    /// no checkpoint covering its end. Unknown is never clean.
    NotClean,
    /// A run in it recorded no workspace or surface its rules were matched
    /// on (or it has no run record), so no run can be said to be in its
    /// situation — [`examples`]' "unscoped".
    Unscoped,
    /// It called no tool.
    NoToolCalls,
    /// Past the read window or the cap: not read this time.
    BeyondWindow,
}

impl Withheld {
    pub fn as_str(self) -> &'static str {
        match self {
            Withheld::NoGoal => "no goal",
            Withheld::NoSession => "no session",
            Withheld::NotAdmitted => "session not admitted",
            Withheld::TooLarge => "transcript too large",
            Withheld::Unread => "transcript unread",
            Withheld::NotClean => "not clean",
            Withheld::Unscoped => "unscoped",
            Withheld::NoToolCalls => "no tool calls",
            Withheld::BeyondWindow => "beyond the read window",
        }
    }
}

/// One session's example, with the situations its runs were matched in.
#[derive(Debug, Clone)]
pub struct SuccessExample {
    pub example: Example,
    /// Every run record's scope: the registry, workspace, surface and goal
    /// its rules block was matched against. A run is in the example's
    /// situation only when every one of them matches it.
    pub scopes: Vec<crate::situation::Situation>,
    pub at: Option<chrono::DateTime<chrono::Utc>>,
}

/// The planning examples a success set lends, before any run's situation
/// is asked — what the owner's readout lists, and what
/// [`SuccessExamples::for_run`] narrows.
#[derive(Debug, Clone, Default)]
pub struct SuccessExamples {
    pub examples: Vec<SuccessExample>,
    /// `(the act's pointer, the session if one, why)` for each standing
    /// success, or session of one, that lent nothing.
    pub withheld: Vec<(String, Option<String>, Withheld)>,
}

impl SuccessExamples {
    /// The examples a run in `run`'s situation may be served: those whose
    /// every run scope matches it (`Situation::matches`, the rules block's
    /// own match), newest success first.
    pub fn for_run(&self, run: &crate::situation::Situation) -> Vec<Example> {
        self.examples
            .iter()
            .filter(|e| !e.scopes.is_empty() && e.scopes.iter().all(|s| s.matches(run)))
            .map(|e| e.example.clone())
            .collect()
    }
}

/// The success examples one run holds: the whole pool, the run's situation
/// as rendered, and the examples served in it. Kept whole rather than
/// resolved at build, because the registry the run record names is the one
/// the run *starts* with — `tasks work` and `questions answer` withhold
/// `kg_task_update` and insert `ask_user` after `setup::build` — and
/// `Situation::matches` is a subset test on tools, so keying on the build's
/// registry both withholds an example the run is in the situation of and
/// serves one it is not (found on review of #342). The loop re-keys it
/// with [`Self::for_registry`], beside `PastAppraisals::for_registry`.
#[derive(Debug, Clone, Default)]
pub struct ServedSuccesses {
    pool: SuccessExamples,
    run: crate::situation::Situation,
    served: Vec<Example>,
    /// Re-read the owning stores at each run start ([`Self::select`]).
    live: bool,
}

impl ServedSuccesses {
    /// A pool read from the default stores: every run start re-reads the
    /// closure and workflow stores and drops what has been taken back
    /// ([`Self::at_run_start`]).
    pub fn select(pool: SuccessExamples, run: &crate::situation::Situation) -> ServedSuccesses {
        ServedSuccesses {
            live: true,
            ..Self::fixed(pool, run)
        }
    }

    /// A pool that is not re-read at run start — a test's, which must not
    /// read the operator's stores. Still re-keyed on the registry.
    pub fn fixed(pool: SuccessExamples, run: &crate::situation::Situation) -> ServedSuccesses {
        let served = pool.for_run(run);
        ServedSuccesses {
            pool,
            run: run.clone(),
            served,
            live: false,
        }
    }

    /// Re-key on the registry the run starts with: the same workspace,
    /// surface and goal, these tools.
    pub fn for_registry(&mut self, tools: &[String]) {
        self.run.tools = tools.to_vec();
        self.served = self.pool.for_run(&self.run);
    }

    /// What the loop calls at every run start: drop what the owner has
    /// taken back since the pool was read, from the closure and workflow
    /// stores as they stand now, then re-key on the registry. The pool is
    /// read once per `setup::build`, and `chat`, the TUI, `serve` and Slack
    /// drive many runs off one build — a reopen on Tuesday must withdraw an
    /// example a process read on Monday, or the answer's "has not reopened
    /// it" is false (found on review of #342). A success verified since the
    /// read is missed until the next build — a miss, never a retraction
    /// ignored.
    pub fn at_run_start(&mut self, tools: &[String]) {
        if self.live {
            let (closures, closures_unreadable) = crate::appraisal::load_closures();
            let (workflows, workflows_unreadable) = crate::appraisal::load_workflows();
            self.restand(&crate::success::Sources {
                closures: &closures,
                closures_unreadable,
                workflows: &workflows,
                workflows_unreadable,
                ..Default::default()
            });
        }
        self.for_registry(tools);
    }

    /// Keep only the examples whose act still stands in `now` — derived by
    /// `success::derive` itself, so withdrawal means here what it means
    /// everywhere. Only a closure or a workflow close can be taken back; a
    /// draft sent and a question answered cannot, and stay. A store that
    /// cannot be read now withdraws every example of its kind: whether the
    /// act still stands is unknown, and unknown is never served.
    pub fn restand(&mut self, now: &crate::success::Sources<'_>) {
        use crate::success::{Act, Seen, SessionFacts};
        // Placement was settled when the pool was read; this read asks only
        // whether the act stands.
        struct Placed;
        impl SessionFacts for Placed {
            fn seen(&self, _: &str) -> Seen {
                Seen::Admitted
            }
            fn completed(&self, _: &str) -> Option<bool> {
                None
            }
        }
        let set = crate::success::derive(now, &Placed);
        let standing: std::collections::HashSet<String> =
            set.standing.iter().map(|s| s.act.pointer()).collect();
        self.pool.examples.retain(|e| match &e.example.owner_act {
            Some(act @ Act::TaskDone { .. }) => {
                !now.closures_unreadable && standing.contains(&act.pointer())
            }
            Some(act @ Act::WorkflowClosed { .. }) => {
                !now.workflows_unreadable && standing.contains(&act.pointer())
            }
            Some(Act::SentUnchanged { .. } | Act::QuestionAnswered { .. }) => true,
            None => false,
        });
        self.served = self.pool.for_run(&self.run);
    }

    pub fn served(&self) -> &[Example] {
        &self.served
    }
}

/// A success example toward `goal` whose one run was matched on `tools`
/// and nothing else — for tests of what serves and re-keys it.
#[cfg(test)]
pub(crate) fn test_success(
    goal: &str,
    step: &str,
    session: &str,
    tools: &[&str],
) -> SuccessExample {
    SuccessExample {
        example: Example {
            goal: goal.parse().unwrap(),
            step: step.into(),
            expected: None,
            source: session.into(),
            owner_act: Some(crate::success::Act::TaskDone {
                task: goal.trim_start_matches("task:").into(),
                closure: "c1".into(),
            }),
        },
        scopes: vec![crate::situation::Situation::of_run(
            &tools.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
            None,
        )],
        at: None,
    }
}

/// The planning examples a success set lends (row 2e-4b-1, R40). For each
/// **standing** success toward a goal — a withdrawn one is not in
/// `standing`, so a reopened task lends nothing, by construction — each
/// session it names that the corpus admits, whose recorded taint is clean
/// to its end, whose every run was matched on a workspace and a surface,
/// and that called a tool, lends one example: its tool sequence as the
/// step, toward the success's goal, with the owner's act beside it. Newest
/// success first; a session lends once per goal.
///
/// Read from the transcripts on every call and never stored: the success
/// set is derived at read time (R40), and so is what it lends. A caller that
/// holds the result across runs re-checks it ([`ServedSuccesses::at_run_start`]).
pub fn success_examples(
    set: &crate::success::Successes,
    sessions: &crate::success::SessionIndex,
) -> SuccessExamples {
    let lent = success_traces(set, sessions, true);
    SuccessExamples {
        examples: lent
            .traces
            .into_iter()
            .filter_map(|t| {
                Some(SuccessExample {
                    example: Example {
                        goal: t.goal?,
                        step: t.sequence,
                        expected: None,
                        source: t.session,
                        owner_act: Some(t.act),
                    },
                    scopes: t.scopes,
                    at: t.at,
                })
            })
            .collect(),
        withheld: lent.withheld,
    }
}

/// One clean, scoped, verified session's trace: what a success lends,
/// before it is shaped as a planning example (2e-4b-1) or as contrast
/// evidence beside a correction (2e-4b-2).
#[derive(Debug, Clone)]
pub struct SuccessTrace {
    pub act: crate::success::Act,
    /// The success's goal, where its record names one.
    pub goal: Option<GoalRef>,
    pub session: String,
    /// Every session the success names, this one among them: a success
    /// verified work across sessions, and a correction in any of them is
    /// part of what it verified (found on review of #345).
    pub named: Vec<String>,
    /// [`tool_sequence`] over every message the session ever held.
    pub sequence: String,
    /// Every run record's situation, as [`SuccessExample::scopes`].
    pub scopes: Vec<crate::situation::Situation>,
    pub at: Option<chrono::DateTime<chrono::Utc>>,
}

/// What a success set lends as traces, and what lent nothing and why.
#[derive(Debug, Clone, Default)]
pub struct SuccessTraces {
    pub traces: Vec<SuccessTrace>,
    pub withheld: Vec<(String, Option<String>, Withheld)>,
}

/// The lending [`success_examples`] describes, with the goal optional:
/// `need_goal` withholds a success toward no goal before any transcript is
/// read (a planning example is served only toward its goal); without it, a
/// draft sent unchanged or a goal-less workflow lends too (contrast
/// evidence, which is matched by region, not goal).
pub fn success_traces(
    set: &crate::success::Successes,
    sessions: &crate::success::SessionIndex,
    need_goal: bool,
) -> SuccessTraces {
    let mut out = SuccessTraces::default();
    let mut standing: Vec<&crate::success::Success> = set.standing.iter().collect();
    // Undated last, as every listing of the set orders them.
    standing.sort_by_key(|s| (s.at.is_none(), std::cmp::Reverse(s.at)));
    let mut read = 0usize;
    let mut lent = std::collections::HashSet::new();
    for success in standing {
        let pointer = success.act.pointer();
        let goal = success.goal.as_ref();
        if need_goal && goal.is_none() {
            out.withheld.push((pointer, None, Withheld::NoGoal));
            continue;
        }
        if success.sessions.is_empty() {
            out.withheld.push((pointer, None, Withheld::NoSession));
            continue;
        }
        for id in &success.sessions {
            let mut withhold = |why| out.withheld.push((pointer.clone(), Some(id.clone()), why));
            // The window before the dedup, so a pair past the window is
            // said for every success naming it, never consumed quietly.
            if read >= SUCCESS_SESSIONS_READ || out.traces.len() >= SUCCESS_EXAMPLES_KEPT {
                withhold(Withheld::BeyondWindow);
                continue;
            }
            if !lent.insert((id.clone(), goal.map(ToString::to_string))) {
                continue;
            }
            let Some(path) = sessions.admitted_path(id) else {
                withhold(Withheld::NotAdmitted);
                continue;
            };
            match std::fs::metadata(path) {
                Ok(m) if m.len() <= SUCCESS_TRANSCRIPT_BYTES => {}
                Ok(_) => {
                    withhold(Withheld::TooLarge);
                    continue;
                }
                // Gone since the index walked it: unread, not large (found
                // on review of #342).
                Err(_) => {
                    withhold(Withheld::Unread);
                    continue;
                }
            }
            read += 1;
            // One read of the file for both views: the loaded list for the
            // taint and the run records, and every message the session ever
            // held for its sequence — the loaded list is what survived a
            // compaction, and a long verified session's trace would be its
            // tail served as the whole (found on review of #342).
            let parsed = std::fs::read_to_string(path)
                .map_err(anyhow::Error::from)
                .and_then(|text| crate::session::Session::parse(path, &text).map(|t| (text, t)));
            let Ok((text, t)) = parsed else {
                withhold(Withheld::Unread);
                continue;
            };
            // The taint covering the last message is the session's: the
            // timeline is cumulative, so clean there is clean throughout,
            // and no checkpoint after it is unknown — never clean.
            let covering = t
                .convo
                .messages
                .len()
                .checked_sub(1)
                .and_then(|last| t.taint_timeline.covering(last));
            if crate::learning::classify_origin(covering) != crate::learning::Origin::Clean {
                withhold(Withheld::NotClean);
                continue;
            }
            // Every run's scope, as `examples` reads one: a run with no
            // matched workspace or surface is unscoped, and so is a
            // transcript with no run record.
            let scopes: Option<Vec<crate::situation::Situation>> = t
                .configs
                .iter()
                .map(|c| {
                    let workspace = c.rules_workspace.as_deref()?;
                    let surface = c.rules_surface?;
                    Some(
                        crate::situation::Situation::of_run(&c.tools, Some(workspace))
                            .on(Some(surface))
                            .toward(c.rules_goal.clone()),
                    )
                })
                .collect();
            let Some(scopes) = scopes.filter(|s| !s.is_empty()) else {
                withhold(Withheld::Unscoped);
                continue;
            };
            let Some(step) = tool_sequence(&crate::session::Session::messages_ever(&text)) else {
                withhold(Withheld::NoToolCalls);
                continue;
            };
            out.traces.push(SuccessTrace {
                act: success.act.clone(),
                goal: goal.cloned(),
                session: id.clone(),
                named: success.sessions.clone(),
                sequence: step,
                scopes,
                at: success.at,
            });
        }
    }
    out
}

/// What the success set lends from the default stores: the owning stores
/// read once (`success::Owned`), the session headers at `dir` under the
/// corpus's admission, and the set derived from them. A session store that
/// cannot be listed lends nothing.
pub fn success_examples_at(dir: &std::path::Path) -> SuccessExamples {
    let owned = crate::success::Owned::load();
    match crate::success::SessionIndex::load(dir, false) {
        Ok(index) => success_examples(&crate::success::derive(&owned.sources(), &index), &index),
        Err(_) => SuccessExamples::default(),
    }
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

#[cfg(test)]
mod example_admission_tests {
    use super::*;
    use crate::{
        agent::Taint,
        message::Message,
        session::{Record, RunConfig, SessionKind, SessionMeta},
        situation::Situation,
    };

    #[test]
    fn smoke_tests_cannot_supply_examples_or_crowd_real_work_out_of_the_window() {
        let root = crate::mismatch::Workspace::new().unwrap();
        let tools = vec!["todo".into()];
        let situation = Situation::of_run(&tools, Some(root.path())).on(Some(SessionKind::Run));
        for i in 0..34 {
            let mut message = Message::user("harness check observation");
            message.planning = Some(Feedback {
                steps: vec![serde_json::from_value(serde_json::json!({
                    "step":"write answer", "goal":"task:example", "verification":"passed"
                }))
                .unwrap()],
                ..Feedback::default()
            });
            let records = [
                Record::Meta(SessionMeta {
                    id: format!("example-{i}"),
                    created_at: chrono::Utc::now() + chrono::Duration::seconds(i),
                    provider: "scripted".into(),
                    model: "scripted".into(),
                    workspace: root.path().into(),
                    title: None,
                    kind: Some(if i == 0 {
                        SessionKind::Run
                    } else {
                        SessionKind::Test
                    }),
                }),
                Record::Config(RunConfig {
                    tools: tools.clone(),
                    rules_workspace: Some(root.path().into()),
                    rules_surface: Some(SessionKind::Run),
                    ..Default::default()
                }),
                Record::Message(message),
                Record::Taint(Taint::default()),
            ];
            let text = records
                .iter()
                .map(|r| serde_json::to_string(r).unwrap())
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(root.path().join(format!("example-{i}.jsonl")), text).unwrap();
        }
        let found = examples(root.path(), &situation).unwrap();
        assert_eq!(
            found.len(),
            1,
            "development runs are not evidence of successful real work"
        );
        assert_eq!(
            found[0].source, "example-0",
            "admission precedes the recent-session limit"
        );
    }
}

#[cfg(test)]
mod success_example_tests {
    use super::*;
    use crate::{
        agent::Taint,
        closure::Transition,
        message::{Block, Message},
        session::{Record, RunConfig, SessionKind, SessionMeta},
        situation::{GoalKey, Situation},
        success::{derive, SessionIndex, Sources},
    };
    use serde_json::json;

    const TASK: &str = "task-northwind-report";

    /// How a fixture session differs from a clean, scoped one that called
    /// tools.
    #[derive(Clone, Copy, PartialEq)]
    enum Shape {
        Clean,
        Tainted,
        NoTaintRecord,
        Unscoped,
        NoTools,
        Test,
        Compacted,
    }

    fn call(id: &str, name: &str) -> Message {
        Message::assistant(vec![Block::ToolUse {
            id: id.into(),
            name: name.into(),
            input: json!({"path": "Northwind Labs/report.md"}),
        }])
    }

    fn result(id: &str) -> Message {
        Message::tool_results(vec![Block::ToolResult {
            tool_use_id: id.into(),
            content: "ok".into(),
            is_error: false,
        }])
    }

    /// A session a delegated run for Dana Whitfield recorded: read the
    /// draft, run the build twice, write the report.
    fn session(dir: &std::path::Path, id: &str, shape: Shape) {
        let tools: Vec<String> = ["fs_read", "fs_write", "goal_context", "shell"]
            .map(String::from)
            .to_vec();
        let mut messages = vec![Message::user(
            "Finish the Northwind Labs report for Dana Whitfield",
        )];
        if shape != Shape::NoTools {
            for (i, name) in ["fs_read", "shell", "shell", "fs_write"].iter().enumerate() {
                let id = format!("c{i}");
                messages.push(call(&id, name));
                messages.push(result(&id));
            }
        }
        messages.push(Message::assistant(vec![Block::text("Done.")]));
        let mut records = vec![
            Record::Meta(SessionMeta {
                id: id.into(),
                created_at: chrono::Utc::now(),
                provider: "scripted".into(),
                model: "scripted".into(),
                workspace: dir.into(),
                title: None,
                kind: Some(if shape == Shape::Test {
                    SessionKind::Test
                } else {
                    SessionKind::Task
                }),
            }),
            Record::Config(RunConfig {
                tools,
                rules_workspace: (shape != Shape::Unscoped).then(|| dir.into()),
                rules_surface: Some(SessionKind::Task),
                rules_goal: Some(GoalKey::Named(GoalRef::Task(TASK.into()))),
                ..Default::default()
            }),
        ];
        records.extend(messages.into_iter().map(Record::Message));
        if shape == Shape::Compacted {
            // A compaction that kept a summary and the last answer: the
            // loaded list holds no tool call at all.
            records.push(Record::Rewrite {
                messages: vec![
                    Message::user("Summary: the report was drafted and built."),
                    Message::assistant(vec![Block::text("Done.")]),
                ],
            });
        }
        if shape != Shape::NoTaintRecord {
            records.push(Record::Taint(Taint {
                private: true,
                untrusted: shape == Shape::Tainted,
            }));
        }
        let text = records
            .iter()
            .map(|r| serde_json::to_string(r).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(dir.join(format!("{id}.jsonl")), text).unwrap();
    }

    fn close(id: &str, session: &str) -> Transition {
        serde_json::from_value(json!({
            "id": id, "task": TASK, "from": "review", "to": "done", "move": "close",
            "actor": "owner", "surface": "cli", "sessions": [session],
            "at": "2026-09-21T12:00:00Z",
        }))
        .unwrap()
    }

    fn reopen(id: &str, undoes: &str) -> Transition {
        serde_json::from_value(json!({
            "id": id, "task": TASK, "from": "done", "to": "next", "move": "reopen",
            "actor": "owner", "surface": "cli", "sessions": [],
            "at": "2026-09-22T12:00:00Z", "undoes": undoes,
        }))
        .unwrap()
    }

    fn lent(dir: &std::path::Path, closures: &[Transition]) -> SuccessExamples {
        let index = SessionIndex::load(dir, false).unwrap();
        let set = derive(
            &Sources {
                closures,
                ..Default::default()
            },
            &index,
        );
        success_examples(&set, &index)
    }

    /// The run a later delegation of the same task is in: its registry, the
    /// workspace and surface its rules were matched on, and the task.
    fn run_in(dir: &std::path::Path) -> Situation {
        let tools: Vec<String> = ["fs_read", "fs_write", "goal_context", "shell", "todo"]
            .map(String::from)
            .to_vec();
        Situation::of_run(&tools, Some(dir))
            .on(Some(SessionKind::Task))
            .toward(Some(GoalKey::Named(GoalRef::Task(TASK.into()))))
    }

    /// Row 2e-4b's first acceptance: a task closed `done` lends its
    /// session's tool sequence as a planning example toward the task — and
    /// once the owner reopens it, the same session lends nothing, with no
    /// store to forget: the success is withdrawn where it is derived. Fails
    /// on the tree before 2e-4b-1, where a success lent no example at all.
    #[test]
    fn a_reopened_tasks_session_supplies_no_example() {
        let root = crate::mismatch::Workspace::new().unwrap();
        let dir = root.path();
        session(dir, "s-dana", Shape::Clean);

        let standing = lent(dir, &[close("c1", "s-dana")]);
        let served = standing.for_run(&run_in(dir));
        assert_eq!(served.len(), 1, "{standing:?}");
        let e = &served[0];
        assert_eq!(e.step, "fs_read → shell ×2 → fs_write");
        assert_eq!(e.goal, GoalRef::Task(TASK.into()));
        assert_eq!(e.source, "s-dana");
        assert_eq!(
            e.owner_act.as_ref().map(|a| a.pointer()),
            Some("closure:c1".to_string())
        );

        let withdrawn = lent(dir, &[close("c1", "s-dana"), reopen("r1", "c1")]);
        assert!(withdrawn.examples.is_empty(), "{withdrawn:?}");
        assert!(withdrawn.for_run(&run_in(dir)).is_empty());
    }

    /// One build drives many runs in `chat`, the TUI, `serve` and Slack, so
    /// a reopen after the pool was read must withdraw its example at the
    /// next run start — and a closure store that cannot be read then
    /// withdraws every closure's example, since whether it stands is
    /// unknown; an answered question cannot be taken back and stays (found
    /// on review of #342: the pool was read once per process).
    #[test]
    fn a_reopen_after_the_pool_was_read_withdraws_its_example_at_the_next_run() {
        let root = crate::mismatch::Workspace::new().unwrap();
        let dir = root.path();
        session(dir, "s-dana", Shape::Clean);
        let mut pool = lent(dir, &[close("c1", "s-dana")]);
        let mut answered = pool.examples[0].clone();
        answered.example.source = "s-asked".into();
        answered.example.owner_act = Some(crate::success::Act::QuestionAnswered {
            question: "q1".into(),
        });
        pool.examples.push(answered);
        let run = run_in(dir);
        let sources = |closures: &[Transition], unreadable| {
            let mut s = ServedSuccesses::fixed(pool.clone(), &run);
            s.restand(&Sources {
                closures,
                closures_unreadable: unreadable,
                ..Default::default()
            });
            s.served()
                .iter()
                .map(|e| e.source.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            sources(&[close("c1", "s-dana")], false),
            vec!["s-dana", "s-asked"],
            "not vacuous: still standing, both served"
        );
        assert_eq!(
            sources(&[close("c1", "s-dana"), reopen("r1", "c1")], false),
            vec!["s-asked"]
        );
        assert_eq!(sources(&[close("c1", "s-dana")], true), vec!["s-asked"]);
    }

    /// A model that records the one request it is sent and has no lesson.
    #[derive(Clone, Default)]
    struct Heard(std::sync::Arc<std::sync::Mutex<Option<crate::message::CompletionRequest>>>);
    #[async_trait::async_trait]
    impl crate::provider::Provider for Heard {
        fn id(&self) -> &str {
            "heard"
        }
        fn default_model(&self) -> &str {
            "heard-1"
        }
        async fn complete(
            &self,
            req: &crate::message::CompletionRequest,
            _sink: Option<&crate::provider::StreamSink>,
        ) -> anyhow::Result<crate::message::CompletionResponse> {
            *self.0.lock().unwrap() = Some(req.clone());
            Ok(crate::message::CompletionResponse {
                message: Message::assistant(vec![Block::text(r#"{"skip": true}"#)]),
                stop_reason: crate::message::StopReason::EndTurn,
                usage: crate::message::Usage::default(),
                refusal: None,
                model: "heard-1".into(),
                malformed_tool_args: 0,
            })
        }
    }

    /// What the reflector is sent for a steer recorded in `situation`
    /// within `session`, with contrast evidence on over `pool`.
    async fn sent(
        pool: &crate::success::ContrastPool,
        situation: &Situation,
        session: &str,
    ) -> String {
        let heard = Heard::default();
        let steer = crate::learning::Intervention {
            trigger: crate::learning::Trigger::Steer,
            context: "shell cargo build".into(),
            text: "Run the tests before the build.".into(),
            aftermath: "Running the tests first.".into(),
            at: 3,
            tools_before: vec!["shell".into()],
            tools_after: Vec::new(),
        };
        let beside = pool.beside(Some(situation), session);
        crate::learning::Reflector::new(Box::new(heard.clone()), None)
            .with_contrast(true)
            .reflect_beside(&steer, beside.as_ref())
            .await
            .unwrap();
        let req = heard.0.lock().unwrap().clone().unwrap();
        serde_json::to_string(&req.messages).unwrap()
    }

    /// Row 2e-4b-2's acceptance (R43): a correction beside a verified
    /// success in its region reaches the reflector with it, one outside the
    /// region without. The region is the loader's match — the correction's
    /// scope against the success session's run records. Never the
    /// correction's own session, and never a tainted session's success.
    /// Fails on the tree before 2e-4b-2, where no success reached the
    /// reflector.
    #[tokio::test]
    async fn a_correction_in_a_verified_successs_region_is_reflected_beside_it() {
        let root = crate::mismatch::Workspace::new().unwrap();
        let dir = root.path();
        session(dir, "s-dana", Shape::Clean);
        session(dir, "s-tainted", Shape::Tainted);
        let index = SessionIndex::load(dir, false).unwrap();
        let pool = crate::success::ContrastPool::of(success_traces(
            &derive(
                &Sources {
                    closures: &[close("c1", "s-dana"), close("c2", "s-tainted")],
                    ..Default::default()
                },
                &index,
            ),
            &index,
            false,
        ));
        let goal = || Some(GoalKey::Named(GoalRef::Task(TASK.into())));
        let here = Situation::recorded(
            &["shell".into()],
            "steer",
            Some(SessionKind::Task),
            Some(dir),
        )
        .toward(goal());

        let with = sent(&pool, &here, "s-steered").await;
        assert!(with.contains("a-verified-success-in-this-region"), "{with}");
        assert!(with.contains("fs_read → shell ×2 → fs_write"));

        // Outside the region: another workspace, another goal, a tool the
        // success's runs never registered.
        let elsewhere = crate::mismatch::Workspace::new().unwrap();
        for outside in [
            Situation::recorded(
                &["shell".into()],
                "steer",
                Some(SessionKind::Task),
                Some(elsewhere.path()),
            )
            .toward(goal()),
            here.clone().toward(Some(GoalKey::Named(GoalRef::Task(
                "task-lakeside-visit".into(),
            )))),
            Situation::recorded(
                &["mail_send".into()],
                "steer",
                Some(SessionKind::Task),
                Some(dir),
            )
            .toward(goal()),
        ] {
            let without = sent(&pool, &outside, "s-steered").await;
            assert!(!without.contains("a-verified-success"), "{outside:?}");
        }
        // A correction in the success's own session is not contrasted
        // with its own outcome; the tainted session lent nothing at all.
        let own = sent(&pool, &here, "s-dana").await;
        assert!(!own.contains("a-verified-success"));
        assert!(pool.beside(Some(&here), "s-dana").is_none());
        // Unknown situation: nothing beside it.
        assert!(pool.beside(None, "s-steered").is_none());

        // One closure over work done in two sessions: a correction in
        // either is part of what it verified, so neither session's trace is
        // set beside it — only a correction from elsewhere gets one (found
        // on review of #345: the exclusion was per trace).
        session(dir, "s-pair", Shape::Clean);
        let index = SessionIndex::load(dir, false).unwrap();
        let mut both = close("c3", "s-dana");
        both.sessions.push("s-pair".into());
        let paired = crate::success::ContrastPool::of(success_traces(
            &derive(
                &Sources {
                    closures: &[both],
                    ..Default::default()
                },
                &index,
            ),
            &index,
            false,
        ));
        assert!(
            paired.beside(Some(&here), "s-steered").is_some(),
            "not vacuous"
        );
        assert!(paired.beside(Some(&here), "s-pair").is_none());
        assert!(paired.beside(Some(&here), "s-dana").is_none());
    }

    /// A compacted session lends its whole trace, not the tail the
    /// compaction left in the loaded list — here a tail with no call at
    /// all, which read as "no tool calls" (found on review of #342).
    #[test]
    fn a_compacted_session_lends_every_call_it_made() {
        let root = crate::mismatch::Workspace::new().unwrap();
        let dir = root.path();
        session(dir, "s-long", Shape::Compacted);
        let out = lent(dir, &[close("c1", "s-long")]);
        let served = out.for_run(&run_in(dir));
        assert_eq!(served.len(), 1, "{:?}", out.withheld);
        assert_eq!(served[0].step, "fs_read → shell ×2 → fs_write");
    }

    /// Only a clean, admitted, scoped session that called a tool lends one;
    /// each refusal is named, never a quiet empty list. Unknown taint is
    /// not clean.
    #[test]
    fn only_a_clean_admitted_scoped_session_with_calls_lends_an_example() {
        let root = crate::mismatch::Workspace::new().unwrap();
        let dir = root.path();
        let cases = [
            ("s-tainted", Shape::Tainted, Withheld::NotClean),
            ("s-taint-unknown", Shape::NoTaintRecord, Withheld::NotClean),
            ("s-unscoped", Shape::Unscoped, Withheld::Unscoped),
            ("s-idle", Shape::NoTools, Withheld::NoToolCalls),
        ];
        for (id, shape, _) in cases {
            session(dir, id, shape);
        }
        // A smoke test named beside a real session lends nothing, though
        // the success stands on the real one.
        session(dir, "s-real", Shape::Clean);
        session(dir, "s-smoke", Shape::Test);
        let mut both = close("c-both", "s-real");
        both.sessions.push("s-smoke".into());

        let mut closures: Vec<Transition> = cases
            .iter()
            .enumerate()
            .map(|(i, (id, _, _))| close(&format!("c{i}"), id))
            .collect();
        closures.push(both);
        let out = lent(dir, &closures);
        for (id, _, why) in cases {
            assert!(
                out.withheld
                    .iter()
                    .any(|(_, s, w)| s.as_deref() == Some(id) && *w == why),
                "{id}: {:?}",
                out.withheld
            );
        }
        assert!(out
            .withheld
            .iter()
            .any(|(_, s, w)| s.as_deref() == Some("s-smoke") && *w == Withheld::NotAdmitted));
        let served: Vec<String> = out
            .for_run(&run_in(dir))
            .into_iter()
            .map(|e| e.source)
            .collect();
        assert_eq!(served, vec!["s-real".to_string()]);

        // A success toward no goal — a draft sent unchanged — lends none:
        // an example is served only toward its goal.
        let set = crate::success::Successes {
            standing: vec![crate::success::Success {
                act: crate::success::Act::SentUnchanged { item: "o1".into() },
                sessions: vec!["s-real".into()],
                goal: None,
                at: None,
            }],
            ..Default::default()
        };
        let index = SessionIndex::load(dir, false).unwrap();
        let out = success_examples(&set, &index);
        assert!(out.examples.is_empty());
        assert_eq!(out.withheld[0].2, Withheld::NoGoal);
    }

    /// A run is in an example's situation only when every run of the
    /// session was matched where this one is: another workspace, another
    /// surface or another goal is served nothing, and so is a run whose
    /// registry lacks a tool the session carried.
    #[test]
    fn an_example_is_served_only_in_its_sessions_situation() {
        let root = crate::mismatch::Workspace::new().unwrap();
        let dir = root.path();
        session(dir, "s-dana", Shape::Clean);
        let out = lent(dir, &[close("c1", "s-dana")]);
        assert_eq!(out.for_run(&run_in(dir)).len(), 1);

        let elsewhere = crate::mismatch::Workspace::new().unwrap();
        let other_ws = Situation {
            workspace: Some(elsewhere.path().into()),
            ..run_in(dir)
        };
        assert!(out.for_run(&other_ws).is_empty(), "another workspace");
        assert!(
            out.for_run(&run_in(dir).on(Some(SessionKind::Chat)))
                .is_empty(),
            "another surface"
        );
        assert!(
            out.for_run(&run_in(dir).toward(Some(GoalKey::Named(GoalRef::Task(
                "task-lakeside-visit".into()
            )))))
            .is_empty(),
            "another goal"
        );
        assert!(out.for_run(&run_in(dir).toward(None)).is_empty(), "no goal");
        let narrower = Situation {
            tools: vec!["fs_read".into()],
            ..run_in(dir)
        };
        assert!(out.for_run(&narrower).is_empty(), "a narrower registry");
    }

    /// Names only, in order, the harness's own calls left out, a repeat
    /// folded, and a long trace cut with a count rather than silently.
    #[test]
    fn the_tool_sequence_is_names_in_order_without_the_harness() {
        let mut harness = call("h1", "todo");
        harness.harness = true;
        let messages = vec![
            Message::user("go"),
            call("a", "fs_read"),
            harness,
            call("b", "fs_read"),
            call("c", "mail_search"),
        ];
        assert_eq!(
            tool_sequence(&messages).as_deref(),
            Some("fs_read ×2 → mail_search")
        );
        assert_eq!(tool_sequence(&[Message::user("hi")]), None);
        let long: Vec<Message> = (0..30)
            .map(|i| call(&i.to_string(), if i % 2 == 0 { "fs_read" } else { "shell" }))
            .collect();
        let seq = tool_sequence(&long).unwrap();
        assert!(seq.ends_with("→ … 6 more"), "{seq}");
        assert!(!seq.contains("path"), "never an argument: {seq}");
    }
}
