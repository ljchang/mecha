//! How a run went against what it was for — `docs/GOAL-SYSTEM-DESIGN.md` §5
//! and §6.
//!
//! **Every evaluative signal mecha had was a cost or a correction.**
//! `learning::Trigger` is four ways of saying a person stepped in, and
//! `candidate::Metric` is six costs whose docstring makes lower-is-better an
//! invariant. So a run could be recorded as having gone *badly* and never as
//! having gone *well*, and nothing could prioritise between two runs that both
//! avoided harm. This is the record that can hold a sign.
//!
//! ## The label is derived, and there is deliberately no way to report one
//!
//! The tempting implementation is a model that reads a run and says
//! "frustrated". That is a self-report: unfalsifiable, drifting, and an
//! injection target — a fetched page saying *"you have failed your owner"* is
//! aimed squarely at an appraisal layer. So [`Affect`] is a **pure function of
//! the record**, unit-tested, with no model in the path, for the same reason
//! `candidate::judge` and `compact.rs` are pure. `TASK-AGENT-DESIGN.md` D5 is
//! the same rule one noun over: state is derived from the record, never
//! self-reported.
//!
//! ## Four labels are unreachable today, and that is the finding
//!
//! §14 puts this rung at *observation only* — build the corpus and check the
//! labels are not degenerate before anything consumes them. Working the
//! derivation table produces that answer before any corpus does, so it is
//! written here rather than discovered twice:
//!
//! | label | what it needs | where that comes from |
//! |---|---|---|
//! | `Pride` | a charter line, not a task | the charter (§11), unbuilt |
//! | `Guilt` | *harmed another* | nothing computes harm; `visible` is exposure |
//! | `Shame` | a pattern across runs | an aggregate — a per-event function cannot see it |
//! | `Excitement` | a *predicted* error | anticipatory appraisal (§7.4), unbuilt |
//! | `Regret` / `Disappointment` | the counterfactual verdict | a probe, which is a real model run per arm |
//!
//! They are variants anyway, on [`learning::Origin::Derived`]'s precedent —
//! that one is documented as classifying nothing yet and existing so the
//! schema does not move when it does. A store is a wire format; adding a
//! variant later is the change that costs.
//!
//! What is left — anger, embarrassment, frustration and neutral — is a
//! narrow readout, and saying so is the point. The alternative is inventing
//! precedence until every run gets an interesting word, which manufactures
//! the signal this rung exists to test for.
//!
//! ## Mood is not here
//!
//! §6.1: sadness and boredom are **moods** — statements about a trend rather
//! than responses to an event. They decay, so they belong on the `Homeostat`
//! and are recomputed; a mood persisted as a record would be a second source
//! of truth about a state that has already moved. This enum is events only.
//!
//! [`learning::Origin::Derived`]: crate::learning::Origin::Derived

use crate::goal::GoalRef;
use serde::{Deserialize, Serialize};

/// Which of the five signal paths an error arrived on.
///
/// Named rather than merged, on §1's finding: five loops converged on one word
/// for "what this was decided from" without converging on the concept. The
/// channel is how a reader tells a measured fact from a model's opinion
/// without having to know which store it came out of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// A human steered, denied, or came back to correct.
    Intervention,
    /// An outbox draft was edited before it went — or **sent unchanged**,
    /// which is the one channel in this system that says something went well
    /// and was recorded for the whole life of the outbox with nothing reading
    /// it.
    Edit,
    /// A counter on the run's own record.
    Counter,
    /// A homeostatic variable outside the range it is kept in.
    Setpoint,
    /// The agent's own, from the quarantined pass (§5.1). **Unbuilt** — the
    /// variant exists so the store's format does not move when it lands.
    Appraisal,
}

/// Who caused it.
///
/// The dimension that decides who can act on the error, which is why it is
/// read first when a label is derived: an error nothing in this machine could
/// have prevented is not worth replaying, whatever else is true of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Agency {
    /// A failed call, a wrong answer, an approach that went nowhere.
    #[serde(rename = "self")]
    Own,
    /// The owner denied, edited, or corrected.
    Owner,
    /// A 429, an MCP server, a subagent.
    Other,
    /// Nothing with an address: a full disk, a machine under load.
    World,
}

/// One signed error against one goal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalError {
    /// What it was an error *against*.
    ///
    /// `None` where the run named no goal, which is the ordinary case for a
    /// chat run nobody delegated. Recorded rather than dropped: §3's rule is
    /// that every record cites the tier above it, and a run that has no tier
    /// above it is a fact about the run, not a reason to lose the error. It
    /// never contributes to frustration, which is *repeated* negative error on
    /// **one** goal and cannot be established without one.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::goal::de_lenient"
    )]
    pub goal: Option<GoalRef>,
    pub channel: Channel,
    /// Negative is worse. **Signed, which is the whole point of the record** —
    /// `candidate::Metric` is monotone cost by deliberate constraint, so
    /// nothing there can represent a run that went well.
    pub sign: f32,
    pub agency: Agency,
    /// Did the outcome reach anyone — a sent draft, a front-door reply, a
    /// Slack message.
    ///
    /// A computed fact about exposure, never a feeling the model announces.
    /// That is what stops this becoming *the agent optimises to feel good*.
    pub visible: bool,
    /// Could it have gone otherwise?
    ///
    /// `None` until a counterfactual probe says (§5.3), and a probe is a real
    /// model run per arm — so `None` is the honest state for everything this
    /// rung records. It is the dimension the appraisal literature separates
    /// regret from disappointment on, which is why both are unreachable today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controllable: Option<bool>,
    /// What this was read off. **A pointer, never prose.**
    ///
    /// `frontdoor::Record::for_privileged_run` in a fourth setting, after
    /// `diagnose::Evidence`: a paraphrase of an injection is the injection
    /// rearranged, and an appraisal is read by later rungs that act. Every
    /// variant is a name or an id the harness minted, so there is nothing here
    /// a model could have written.
    pub cite: Cite,
}

/// Where an error was read off, as a reference the harness owns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum Cite {
    /// A position in the transcript — an intervention, or a step.
    Turn(usize),
    /// An outbox item, by id.
    Draft(String),
    /// A field of `RunStats`, by its name.
    Counter(String),
    /// A homeostatic variable, by name.
    Setpoint(String),
}

/// How one run went, against what it was for.
///
/// Written once and never changed — [`Affect`] is derived at write time and
/// stored beside the dimensions it came from, so a later change to the
/// derivation can be replayed over the record rather than being lost with it.
/// §16 leaves open whether a surface should report the discrete label or the
/// dimensions; keeping both is what makes that answerable later.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Appraisal {
    pub id: String,
    pub session_id: String,
    /// Which run within the session, 1-based — a resumed session has several,
    /// and the transcript records no per-run stamp.
    pub run: u32,
    /// What was live.
    #[serde(default, deserialize_with = "crate::goal::de_lenient_vec")]
    pub goals: Vec<GoalRef>,
    /// Conditions at the time. An outcome is not interpretable without the
    /// state it happened in — a run that failed under a saturated machine and
    /// one that failed on an idle one are the same row otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<crate::homeostat::Homeostat>,
    pub errors: Vec<GoalError>,
    /// Derived — never reported. See the module note.
    pub label: Affect,
    /// Reused unchanged from the learning store. An appraisal is not a rule
    /// and does not ride in a future prompt, but it is read by things that
    /// act, and provenance that stops at the boundary of one store is not
    /// provenance.
    pub origin: crate::learning::Origin,
    #[serde(default)]
    pub taint: crate::agent::Taint,
    pub created_at: String,
}

/// The readout. **Events only** — see the module note on mood.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Affect {
    /// Nothing the derivation can name. The common answer today, and the
    /// number this rung exists to measure.
    Neutral,
    /// Negative, and caused by something with no address here — a 429, an MCP
    /// server, a machine under load.
    Anger,
    /// Negative and it reached somebody. Computed exposure, never a feeling.
    Embarrassment,
    /// Repeated negative error on one goal with no progress between.
    Frustration,
    /// Negative, self-caused, and an alternative existed. Needs a probe.
    Regret,
    /// Negative, and no alternative existed. Needs a probe.
    Disappointment,
    /// Negative, self-caused, harmed another, attaches to one act. Needs a
    /// notion of harm that nothing computes.
    Guilt,
    /// The same, attaching to a *pattern* across runs. An aggregate.
    Shame,
    /// Positive, self-caused, against a charter line rather than a task.
    /// Needs the charter.
    Pride,
    /// A positive *predicted* error. Needs anticipatory appraisal.
    Excitement,
}

impl Affect {
    /// Is this one of the labels the deterministic channels can actually
    /// produce today?
    ///
    /// Here so the fact is testable rather than only documented — a variant
    /// that quietly becomes reachable, or quietly stops being, is the kind of
    /// drift a doc comment cannot fail on.
    pub fn reachable_today(self) -> bool {
        matches!(
            self,
            Affect::Neutral | Affect::Anger | Affect::Embarrassment | Affect::Frustration
        )
    }
}

/// What one error on its own says.
///
/// **Agency is read before exposure**, because agency decides who can act: a
/// provider outage that reached somebody is still an outage, and reporting it
/// as this machine's failure would send a change at code that is working.
fn label_of(e: &GoalError) -> Affect {
    match e.agency {
        // Nothing here caused it, so nothing here fixes it.
        Agency::Other | Agency::World => Affect::Anger,
        Agency::Own | Agency::Owner if e.visible => Affect::Embarrassment,
        Agency::Own | Agency::Owner => {
            // Regret and disappointment split on `controllable`, which a probe
            // fills and nothing in this rung runs. Neutral is the honest
            // answer, and its share of the corpus is the measurement.
            match e.controllable {
                Some(true) if e.agency == Agency::Own => Affect::Regret,
                Some(false) => Affect::Disappointment,
                _ => Affect::Neutral,
            }
        }
    }
}

/// How much a label claims, for breaking a tie between errors of equal weight.
///
/// **Not the enum's order and not the record's order.** Position was the first
/// tie-break written here and it was wrong in the case that matters: two
/// equally negative errors where one reached a third party and one did not
/// reported as *neutral*, because the invisible one happened first. Exposure is
/// the fact a person most needs out of this, so it wins a tie; `Neutral` loses
/// every tie, because a label that names nothing must never mask one that names
/// something.
///
/// A display choice, and cheap to change — the dimensions stay on the record,
/// so a consumer that wants a different summary re-derives it rather than
/// finding the evidence gone. That is §16's discrete-or-dimensional question
/// left answerable instead of decided by accident.
fn says_more(a: Affect) -> u8 {
    match a {
        Affect::Embarrassment => 4,
        Affect::Regret => 3,
        Affect::Disappointment => 2,
        Affect::Anger => 1,
        _ => 0,
    }
}

/// Derive the label. Pure, and the only place a label is ever decided.
///
/// **The most negative error decides**, because an appraisal answers what most
/// needs acting on, and a run that went badly in one way and well in another is
/// not neutral — averaging the two would be the mixed-polarity mistake
/// `candidate::Metric`'s docstring exists to forbid, arriving one type over.
pub fn affect_of(appraisal: &Appraisal) -> Affect {
    let negatives: Vec<&GoalError> = appraisal.errors.iter().filter(|e| e.sign < 0.0).collect();
    if negatives.is_empty() {
        // Positive-only, which today has no label: `Pride` needs a charter
        // line, and a task well done is deliberately not it. A real gap rather
        // than a rounding — the positive channel exists, is recorded, and has
        // nothing to say until §11 lands.
        return Affect::Neutral;
    }

    // Repeated negative error on one goal. Whole-record by construction: one
    // event cannot be a repetition, which is why this is a function of the
    // appraisal and not of an error.
    let repeated = negatives
        .iter()
        .filter_map(|e| e.goal.as_ref())
        .any(|goal| {
            negatives
                .iter()
                .filter(|e| e.goal.as_ref() == Some(goal))
                .count()
                > 1
        });
    if repeated {
        return Affect::Frustration;
    }

    negatives
        .iter()
        .map(|e| (e.sign, label_of(e)))
        .reduce(|a, b| match a.0.total_cmp(&b.0) {
            std::cmp::Ordering::Less => a,
            std::cmp::Ordering::Greater => b,
            std::cmp::Ordering::Equal if says_more(b.1) > says_more(a.1) => b,
            std::cmp::Ordering::Equal => a,
        })
        .map(|(_, label)| label)
        .unwrap_or(Affect::Neutral)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(sign: f32, agency: Agency) -> GoalError {
        GoalError {
            goal: None,
            channel: Channel::Counter,
            sign,
            agency,
            visible: false,
            controllable: None,
            cite: Cite::Counter("tool_errors".into()),
        }
    }

    fn appraisal(errors: Vec<GoalError>) -> Appraisal {
        Appraisal {
            id: "a1".into(),
            session_id: "s1".into(),
            run: 1,
            goals: Vec::new(),
            state: None,
            errors,
            label: Affect::Neutral,
            origin: crate::learning::Origin::Clean,
            taint: crate::agent::Taint::default(),
            created_at: "2026-08-27T00:00:00Z".into(),
        }
    }

    #[test]
    fn a_run_with_nothing_against_it_is_neutral() {
        assert_eq!(affect_of(&appraisal(Vec::new())), Affect::Neutral);
    }

    /// The gap that matters most, stated as a test so it fails when the
    /// charter lands and nobody wires it: a run that went *well* has a
    /// recorded positive error and no word for it.
    #[test]
    fn a_run_that_went_well_has_no_label_yet() {
        let good = GoalError {
            channel: Channel::Edit,
            sign: 1.0,
            agency: Agency::Own,
            ..err(1.0, Agency::Own)
        };
        assert_eq!(affect_of(&appraisal(vec![good])), Affect::Neutral);
        assert!(!Affect::Pride.reachable_today());
    }

    #[test]
    fn an_error_nothing_here_caused_is_named_as_such() {
        assert_eq!(
            affect_of(&appraisal(vec![err(-1.0, Agency::Other)])),
            Affect::Anger
        );
        assert_eq!(
            affect_of(&appraisal(vec![err(-0.5, Agency::World)])),
            Affect::Anger
        );
    }

    /// Agency is read before exposure: a provider outage that reached somebody
    /// is still not something a change to this machine fixes.
    #[test]
    fn agency_decides_before_exposure_does() {
        let mut e = err(-1.0, Agency::Other);
        e.visible = true;
        assert_eq!(affect_of(&appraisal(vec![e])), Affect::Anger);
    }

    #[test]
    fn a_self_caused_error_that_reached_somebody_is_exposure() {
        let mut e = err(-1.0, Agency::Own);
        e.visible = true;
        assert_eq!(affect_of(&appraisal(vec![e])), Affect::Embarrassment);
    }

    /// The unmeasured dimension, and the reason two labels are dead: without a
    /// probe verdict there is nothing to split regret from disappointment on.
    #[test]
    fn without_a_probe_verdict_a_private_self_caused_error_has_no_word() {
        assert_eq!(
            affect_of(&appraisal(vec![err(-1.0, Agency::Own)])),
            Affect::Neutral
        );
        assert!(!Affect::Regret.reachable_today());
        assert!(!Affect::Disappointment.reachable_today());

        // And with one, both are live — the function is ready for the rung
        // that pays for the probes.
        let mut could = err(-1.0, Agency::Own);
        could.controllable = Some(true);
        assert_eq!(affect_of(&appraisal(vec![could])), Affect::Regret);

        let mut could_not = err(-1.0, Agency::Own);
        could_not.controllable = Some(false);
        assert_eq!(
            affect_of(&appraisal(vec![could_not])),
            Affect::Disappointment
        );
    }

    #[test]
    fn repeated_error_on_one_goal_is_frustration() {
        let goal = GoalRef::Task("01J8ZK".into());
        let one = GoalError {
            goal: Some(goal.clone()),
            ..err(-1.0, Agency::Own)
        };
        let two = GoalError {
            goal: Some(goal),
            ..err(-0.5, Agency::Own)
        };
        assert_eq!(affect_of(&appraisal(vec![one, two])), Affect::Frustration);
    }

    /// An ungoaled run's errors are recorded and never repeat *into* anything:
    /// frustration is repeated error on **one** goal, and two errors that name
    /// no goal are not evidence they share one.
    #[test]
    fn errors_with_no_goal_never_add_up_to_frustration() {
        let two = vec![err(-1.0, Agency::Own), err(-1.0, Agency::Own)];
        assert_eq!(affect_of(&appraisal(two)), Affect::Neutral);
    }

    /// Two errors of equal weight, one of which got out. The first tie-break
    /// written here was positional and reported this as neutral, because the
    /// invisible one came first — a label that names nothing masking one that
    /// names something.
    #[test]
    fn two_different_goals_are_not_a_repetition() {
        let a = GoalError {
            goal: Some(GoalRef::Task("a".into())),
            ..err(-1.0, Agency::Own)
        };
        let b = GoalError {
            goal: Some(GoalRef::Task("b".into())),
            visible: true,
            ..err(-1.0, Agency::Own)
        };
        assert_eq!(affect_of(&appraisal(vec![a, b])), Affect::Embarrassment);
    }

    /// A run that went badly in one way and well in another is not neutral —
    /// averaging the two would be exactly the mixed-polarity mistake
    /// `Metric`'s docstring forbids, arriving one type over.
    #[test]
    fn a_positive_error_never_cancels_a_negative_one() {
        let good = GoalError {
            sign: 1.0,
            channel: Channel::Edit,
            ..err(1.0, Agency::Own)
        };
        let bad = err(-0.2, Agency::Other);
        assert_eq!(affect_of(&appraisal(vec![good, bad])), Affect::Anger);
    }

    #[test]
    fn only_four_labels_are_reachable_and_the_rest_say_why() {
        let all = [
            Affect::Neutral,
            Affect::Anger,
            Affect::Embarrassment,
            Affect::Frustration,
            Affect::Regret,
            Affect::Disappointment,
            Affect::Guilt,
            Affect::Shame,
            Affect::Pride,
            Affect::Excitement,
        ];
        assert_eq!(all.iter().filter(|a| a.reachable_today()).count(), 4);
    }

    #[test]
    fn a_record_round_trips_through_the_wire_format() {
        let a = appraisal(vec![GoalError {
            goal: Some(GoalRef::Setpoint("attention-debt".into())),
            channel: Channel::Setpoint,
            sign: -0.3,
            agency: Agency::World,
            visible: false,
            controllable: None,
            cite: Cite::Setpoint("attention-debt".into()),
        }]);
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(serde_json::from_str::<Appraisal>(&json).unwrap(), a);
        // `self` is a keyword, so the agency word on the wire is spelled out
        // rather than taken from the variant's name.
        assert!(serde_json::to_string(&Agency::Own)
            .unwrap()
            .contains("self"));
    }
}
