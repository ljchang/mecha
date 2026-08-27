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
//! ## Six labels are unreachable today, and that is the finding
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
///
/// **Exhaustive on purpose, with no catch-all.** A `_ => 0` arm would put any
/// future label — `Guilt`, `Shame`, `Pride`, `Excitement`, and `Frustration`
/// itself — at the same rank as `Neutral`, silently contradicting the rule
/// above that `Neutral` loses every tie: a variant added to [`label_of`] would
/// compile fine and mask nothing, when the whole point of this function is
/// that everything *but* `Neutral` should be able to win one. Listing every
/// variant means the compiler catches that instead. `Guilt`/`Shame` join
/// `Embarrassment`'s rank: all three are exposure-flavoured harm that a reader
/// most needs surfaced. `Anger`/`Pride`/`Excitement` join the lowest non-zero
/// rank — `label_of` never actually produces the latter two, so this is only
/// ever exercised through `Anger`. `Frustration` never reaches this function
/// today either (`affect_of` decides it separately, see below), but its rank
/// still has to sit *below* the exposure tier so a repeated self-inflicted
/// error can never be preferred over — or mistaken for beating — a visible
/// mistake in the same record.
fn says_more(a: Affect) -> u8 {
    match a {
        Affect::Embarrassment | Affect::Guilt | Affect::Shame => 4,
        Affect::Frustration | Affect::Regret => 3,
        Affect::Disappointment => 2,
        Affect::Anger | Affect::Pride | Affect::Excitement => 1,
        Affect::Neutral => 0,
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

    // The most negative error decides, exactly as when there is no
    // repetition below — computed first so the repetition check can only
    // ever upgrade this result, never bury it. See that check for why the
    // order matters.
    let reduced = negatives
        .iter()
        .map(|e| (e.sign, label_of(e)))
        .reduce(|a, b| match a.0.total_cmp(&b.0) {
            std::cmp::Ordering::Less => a,
            std::cmp::Ordering::Greater => b,
            std::cmp::Ordering::Equal if says_more(b.1) > says_more(a.1) => b,
            std::cmp::Ordering::Equal => a,
        })
        .map(|(_, label)| label)
        .unwrap_or(Affect::Neutral);

    // Repeated negative error on one goal, self-agency, of the *same kind*
    // (§6.1: "repeated, one goal, self-agency"). Whole-record by
    // construction: one event cannot be a repetition, which is why this is a
    // function of the appraisal and not of an error.
    //
    // **Self-agency, not any negative, is load-bearing.** `of_session` clones
    // one `goals.first()` onto every error it builds, so "shares a goal" is
    // not yet a discriminator at all — without the agency filter, `repeated`
    // degenerates to "two or more negative errors" the moment a session
    // names a goal. Filtering to `Agency::Own` keeps this reachable only by
    // errors the agent itself caused.
    //
    // **And agency alone still is not enough.** `of_session` can emit up to
    // three distinct `Agency::Own` counter errors from one run —
    // `stop_cause: Loop|NoOutput`, `ended_on_failed_call`, and
    // `boredom_notices > 0` — all sharing the goal, and none of those is a
    // repetition of another: three different symptoms, not one mistake made
    // twice. `error_kind` groups by `Channel` plus, for a `Cite::Counter`,
    // the counter's own name — the only thing this record carries that names
    // *which* signal fired — so two errors count as "the same kind" only
    // when they really are one: two probed interventions on the same goal
    // (`Channel::Intervention`, no counter name to divide further) are a
    // repetition; `ended_on_failed_call` and `boredom_notices` are not.
    //
    // **And this may only ever upgrade `reduced`, never bury it.** The first
    // cut returned `Frustration` the moment `repeated` was true, before
    // `label_of`'s reduce ran at all — which outranked agency and exposure
    // both, the one ordering this module argues hardest for: a ceiling
    // nobody here caused (`Agency::World`) plus a draft the owner rewrote
    // (`Agency::Owner`, visible) would report `Frustration` and discard the
    // fact that something went out wrong, exactly what `says_more`'s
    // tie-break exists to prevent from being masked. Comparing ranks instead
    // means a repetition can promote a `Neutral`/`Anger`/`Disappointment`
    // result to `Frustration`, but can never step in front of a
    // higher-or-equal-ranked exposed error — `says_more(Frustration)` sits
    // below the exposure tier for exactly that reason.
    fn error_kind(e: &GoalError) -> (Channel, Option<&str>) {
        match &e.cite {
            Cite::Counter(name) => (e.channel, Some(name.as_str())),
            _ => (e.channel, None),
        }
    }
    let repeated = negatives
        .iter()
        .filter(|e| e.agency == Agency::Own && e.goal.is_some())
        .any(|e| {
            let kind = error_kind(e);
            negatives
                .iter()
                .filter(|o| o.agency == Agency::Own && o.goal == e.goal && error_kind(o) == kind)
                .count()
                > 1
        });
    if repeated && says_more(Affect::Frustration) >= says_more(reduced) {
        return Affect::Frustration;
    }

    reduced
}

/// Build one **session's** appraisal from records that already exist.
///
/// **Derived, not stored, and that is a correction to §10.** The design gives
/// this an appraisal store under the learning root. Every channel here is a
/// pure function of records the machine already keeps — the transcript, the
/// run's own `RunStats`, the outbox — so a store would be `runlog`'s rejected
/// ledger: faster, and a second source of truth that can disagree with the
/// first. Until then there is nothing to keep.
///
/// **What earns a store is the first thing here that costs a model run**, and
/// which one arrives first is not settled: the quarantined appraiser (§5.1) is
/// the design's answer, and the counterfactual probe behind [`apply_probe`] is
/// a real model run per intervention and may well land sooner. Either way it
/// is a *verdict* that needs keeping and not an appraisal — the assembled
/// record stays derivable from the transcript, the outbox and `RunStats`, and
/// only the paid-for part is irrecoverable. So the thing to reach for first is
/// the ledger that already exists for exactly this: `validations.jsonl` keeps
/// probe outcomes today, keyed to what was measured, and a second store beside
/// it needs an argument that these verdicts are keyed differently — which they
/// are, to an intervention rather than to a rule set. Worth deciding
/// deliberately rather than by whichever lands first.
///
/// `interventions` and `drafts` are passed in rather than read here, on
/// doctor's rule: this is a function, and the walking belongs to the caller
/// that decided how much reading it could afford.
/// **A session, not a run, and the unit is the whole correctness of it.** The
/// design's own record carries a session id and no run index; adding one looked
/// harmless and was not, because the two channels that make this record worth
/// having are session-scoped and cannot be split. Interventions come out of the
/// transcript with a message index and nothing marks which run was in flight,
/// and an outbox item records the session that drafted it and never a run — so
/// a per-run appraisal has to attribute every one of them to every run, which
/// multiplies both channels by the number of times the session was resumed.
/// Rung 4 paid for this exact mistake in the other direction, reading headroom
/// off one run's outcome for a whole episode; `RunStats::fold` exists so the
/// fold is written once, and `Session::episode_stats` is the caller's way to it.
pub fn of_session(
    session_id: &str,
    stats: &crate::session::RunStats,
    goals: &[GoalRef],
    interventions: &[crate::learning::Intervention],
    drafts: &[&crate::outbox::OutboxItem],
    // Coverage at the end of the session, from `Session::taint_timeline` —
    // `None` when the caller could not establish it, which includes a
    // transcript recorded before checkpoints existed. Deliberately not read
    // off `stats.taint`: that field is `#[serde(default)]` over a
    // both-false `Taint`, so a row written before the field existed
    // deserialises as *clean* rather than as *unknown*, and passing it
    // through `Some(..)` would make `classify_origin`'s fail-closed `None`
    // arm unreachable from here — the same inversion the taint snapshot and
    // `distill::corrections_for` both refuse elsewhere in this codebase.
    end_taint: Option<crate::agent::Taint>,
    created_at: String,
) -> Appraisal {
    let goal = goals.first().cloned();
    let mut errors = Vec::new();

    // --- Counter: the run's own record ---
    //
    // Only counters whose **agency is determined**. A bare `tool_errors` is
    // not one of those: a failed call may be a wrong argument (mine), an MCP
    // server (another's) or a full disk (the world's), and guessing would put
    // a fabricated attribution in the field the label is derived from. The
    // ones below each say who.
    //
    // Three counters are deliberately absent because they are the harness
    // *working*: `tool_denied` and `blocked_sends` are the approver and the
    // interlock doing their jobs — the same rule that keeps a denial out of
    // the failure count — and `context_overflows` is a recovery that
    // succeeded. Counting any of them would make a well-defended run look like
    // a bad one.
    match stats.stop_cause {
        Some(crate::agent::StopCause::Loop) => errors.push(GoalError {
            goal: goal.clone(),
            channel: Channel::Counter,
            sign: -1.0,
            agency: Agency::Own,
            visible: false,
            controllable: None,
            cite: Cite::Counter("stop_cause".into()),
        }),
        Some(crate::agent::StopCause::NoOutput) => errors.push(GoalError {
            goal: goal.clone(),
            channel: Channel::Counter,
            sign: -1.0,
            agency: Agency::Own,
            visible: false,
            controllable: None,
            cite: Cite::Counter("stop_cause".into()),
        }),
        // A ceiling is a number somebody set, and hitting one is not a thing
        // this run could have done differently — `World`, the agency for what
        // has no address here.
        Some(
            crate::agent::StopCause::MaxTurns
            | crate::agent::StopCause::OutputTokenBudget
            | crate::agent::StopCause::CostBudget,
        ) => errors.push(GoalError {
            goal: goal.clone(),
            channel: Channel::Counter,
            sign: -0.5,
            agency: Agency::World,
            visible: false,
            controllable: None,
            cite: Cite::Counter("stop_cause".into()),
        }),
        // `Interrupted` is **not** an error, on doctor's rule for the same
        // field: a person pressing Ctrl-C is the system working, and counting
        // it would make an attentive owner look like a problem.
        _ => {}
    }

    // The model stopped of its own accord with its last call failed and
    // answered as though it had not — the silent failure the eval rig grades.
    if stats.ended_on_failed_call {
        errors.push(GoalError {
            goal: goal.clone(),
            channel: Channel::Counter,
            sign: -1.0,
            agency: Agency::Own,
            visible: false,
            controllable: None,
            cite: Cite::Counter("ended_on_failed_call".into()),
        });
    }

    // An approach that stopped teaching the run anything (§9.1). Absent is not
    // zero: a row from before the sensor says nothing, and reading it as a run
    // that was never stuck is the dilution the field is `Option` to prevent.
    if stats.boredom_notices.is_some_and(|n| n > 0) {
        errors.push(GoalError {
            goal: goal.clone(),
            channel: Channel::Counter,
            sign: -0.5,
            agency: Agency::Own,
            visible: false,
            controllable: None,
            cite: Cite::Counter("boredom_notices".into()),
        });
    }

    // --- Intervention: a person stepped in ---
    //
    // `Agency::Owner` on the design's own example — *the owner denied/edited*.
    // Not `Own`, because whether the work was wrong or the owner simply wanted
    // something else is a judgement, and **nothing here can make it**. That is
    // a statement about this function, not about the world: it is a pure
    // function of on-disk records, and the question needs a replay. See
    // [`apply_probe`], which is what licenses moving it.
    //
    // `Followup` is excluded, on `counterfactual.rs`'s own precedent: it
    // declines to grade a followup at all, because there is no counterfactual
    // in a later turn — a second question in a chat is not a correction, and
    // `extract_interventions` mines one for *any* later user turn following a
    // non-empty answer. Measured at 86% of this corpus's interventions. A
    // `-1.0` here has no such gate, so an ordinary multi-turn conversation
    // read a run that went well as a run that went badly, once per turn — the
    // channel this rung exists to measure would have dominated on a signal
    // with no ground truth behind it. `Steer` and `Denial` keep their sign:
    // both are the owner unambiguously stepping in mid-run, which is what
    // `Agency::Owner` states.
    for i in interventions {
        if i.trigger == crate::learning::Trigger::Followup {
            continue;
        }
        errors.push(GoalError {
            goal: goal.clone(),
            channel: Channel::Intervention,
            sign: -1.0,
            agency: Agency::Owner,
            visible: false,
            controllable: None,
            cite: Cite::Turn(i.at),
        });
    }

    // --- Edit: what the owner did with a draft written in their name ---
    for item in drafts {
        // `writing_outcome` already decides `sent` vs `sent-and-edited` vs
        // "says nothing about drafting" — including a publish, on
        // `mineable_as_writing`'s reasoning that its arguments are a path
        // and reading bookkeeping as a judgement of the work is the mistake
        // that rule exists to name. Reusing it rather than re-deriving the
        // same split from `status`/`edited()` is what keeps a third status
        // or a third `OutboxKind` from teaching only one of the two places
        // that reason about it.
        let (sign, agency) = match (item.writing_outcome(), item.status.as_str()) {
            // **The one signal in this system that says something went well.**
            // Recorded since the outbox existed; positive, and it is the reason
            // this record is signed at all.
            (Some(crate::outbox::WritingOutcome::SentUnchanged), _) => (1.0, Agency::Own),
            (Some(crate::outbox::WritingOutcome::SentEdited), _) => (-1.0, Agency::Owner),
            // `writing_outcome` returns `None` for a rejected item too (it
            // never went out), so the message-only guard is this arm's to
            // keep — a rejected publish is still bookkeeping, not a
            // judgement of prose.
            (None, "rejected") if item.kind == crate::outbox::OutboxKind::Message => {
                (-1.0, Agency::Owner)
            }
            // Still pending: the owner has not said anything yet, and reading
            // silence as either answer is what a queue nobody has reached
            // would turn into a verdict.
            _ => continue,
        };
        errors.push(GoalError {
            goal: goal.clone(),
            channel: Channel::Edit,
            sign,
            agency,
            // Exposure means *mecha's* mistake reached somebody, not merely
            // that a message went out. `item.status == "sent"` is true for
            // `SentEdited` too, which reported the owner's own catch as an
            // exposure error — a draft they rewrote in `$EDITOR` sends their
            // words, not mecha's, and the review that caught the difference
            // is the mechanism working, not something that should itself read
            // as `Embarrassment`. Only `SentUnchanged` is mecha's text
            // actually reaching a third party.
            visible: item.writing_outcome() == Some(crate::outbox::WritingOutcome::SentUnchanged),
            controllable: None,
            cite: Cite::Draft(item.id.clone()),
        });
    }

    let mut a = Appraisal {
        id: session_id.to_string(),
        session_id: session_id.to_string(),
        goals: goals.to_vec(),
        state: stats.homeostat.clone(),
        errors,
        label: Affect::Neutral,
        origin: crate::learning::classify_origin(end_taint),
        taint: stats.taint,
        created_at,
    };
    a.label = affect_of(&a);
    a
}

/// What a counterfactual probe found about one intervention.
///
/// The verdict is [`counterfactual::ProbeVerdict`]'s, restated in this
/// module's terms so the label's semantics stay where the label is. Producing
/// one costs a model run per intervention; deciding what it *means* costs
/// nothing and belongs beside [`label_of`].
///
/// [`counterfactual::ProbeVerdict`]: crate::counterfactual::ProbeVerdict
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// Replayed without the intervention, the run went somewhere else. The
    /// steer was load-bearing.
    Mattered,
    /// Replayed without it, the run tracked the recording anyway. The steer
    /// changed nothing.
    Redundant,
    /// The replay departed before the probe point, so the question was never
    /// posed. Not evidence in either direction.
    Inconclusive,
}

/// Fold a probe's finding into the intervention error it was run for.
///
/// **This is the one thing allowed to move `agency`, and that is the whole
/// point of paying for a probe.** [`of_session`] assembles an intervention as
/// `Agency::Owner` because it cannot tell a correction of a wrong trajectory
/// from a change of the owner's mind — and the split between them is exactly
/// what a replay answers:
///
/// - **Mattered** — without the steer the run went elsewhere, so the
///   trajectory *was* wrong and the steer names the alternative that existed.
///   The agent could have done otherwise: `Own` + `controllable`, which is
///   **regret**, and which is the case §8's prioritised replay wants — a run
///   worth re-running because something in this machine could have gone
///   differently.
/// - **Redundant** — the run tracked the recording without the steer, so it
///   was already going the right way. The owner still had to step in, which is
///   a real cost and stays a negative error, but nothing the agent did caused
///   it and nothing it could have done would have avoided it: `Owner` +
///   `controllable: false`, which is **disappointment**, read literally as the
///   literature defines it — a bad outcome with no alternative.
/// - **Inconclusive** — nothing changes. `ProbeVerdict`'s own inconclusive arm
///   exists because a replay that diverged early never posed the question, and
///   an answer invented from a question nobody asked is worse than no answer.
///
/// **The magnitude is deliberately untouched.** A redundant steer is weaker
/// evidence of a goal error than a load-bearing one, and there is an argument
/// for shrinking its `sign` — but the multiplier would be a tuned constant
/// nobody has measured, in the field the label is derived from. `Metric`'s
/// docstring is the precedent for refusing that.
///
/// Applied to a built `Appraisal` rather than inside `of_session`, so that
/// function stays pure over on-disk records and a run with no probe budget
/// produces exactly what it produces today.
pub fn apply_probe(e: &mut GoalError, probe: Probe) {
    match probe {
        Probe::Mattered => {
            e.agency = Agency::Own;
            e.controllable = Some(true);
        }
        Probe::Redundant => {
            e.controllable = Some(false);
        }
        Probe::Inconclusive => {}
    }
}

/// Re-derive the label after probes have spoken.
///
/// Separate from `apply_probe` because a label is a fact about the *whole*
/// record — frustration is repeated error on one goal — so it cannot be
/// recomputed one error at a time.
pub fn relabel(a: &mut Appraisal) {
    a.label = affect_of(a);
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

    /// Two *different* failures on one goal must not read as one repeated
    /// one — a ceiling nobody here caused, plus a draft the owner rewrote,
    /// share a goal only because `of_session` stamps the same reference on
    /// every error it builds. Frustration's own definition is "repeated,
    /// one goal, self-agency" (§6.1); a ceiling is `Agency::World`, so it
    /// cannot be the repetition, and exposure — the fact `says_more` says a
    /// person most needs out of this — must win instead.
    #[test]
    fn two_different_failures_sharing_a_goal_are_not_frustration() {
        let goal = GoalRef::Task("01J8ZK".into());
        let ceiling = GoalError {
            goal: Some(goal.clone()),
            ..err(-0.5, Agency::World)
        };
        let rewritten_draft = GoalError {
            goal: Some(goal),
            visible: true,
            ..err(-1.0, Agency::Owner)
        };
        assert_eq!(
            affect_of(&appraisal(vec![ceiling, rewritten_draft])),
            Affect::Embarrassment,
            "exposure must not be masked by a repetition that never happened"
        );
    }

    /// Restricting `repeated` to `Agency::Own` is not enough on its own:
    /// `of_session` can emit up to three distinct Own-agency counter errors
    /// from one run (`stop_cause`, `ended_on_failed_call`, `boredom_notices`),
    /// all sharing the goal it stamps on everything. Two different symptoms
    /// are not one mistake made twice.
    #[test]
    fn two_different_kinds_of_own_agency_error_are_not_frustration() {
        let goal = GoalRef::Task("01J8ZK".into());
        let ended_on_failed_call = GoalError {
            goal: Some(goal.clone()),
            cite: Cite::Counter("ended_on_failed_call".into()),
            ..err(-1.0, Agency::Own)
        };
        let boredom = GoalError {
            goal: Some(goal),
            cite: Cite::Counter("boredom_notices".into()),
            ..err(-0.5, Agency::Own)
        };
        assert_ne!(
            affect_of(&appraisal(vec![ended_on_failed_call, boredom])),
            Affect::Frustration,
            "two different self-caused symptoms are not one mistake repeated"
        );
    }

    /// A genuine repetition — the *same* kind of self-caused error twice on
    /// one goal — still reports `Frustration` when nothing outranks it, but
    /// must still yield to a higher-ranked exposed error in the same record,
    /// which is what `says_more(Frustration)` sitting below the exposure tier
    /// is for.
    #[test]
    fn a_repeated_own_agency_error_does_not_mask_a_higher_ranked_exposure() {
        let goal = GoalRef::Task("01J8ZK".into());
        let first = GoalError {
            goal: Some(goal.clone()),
            cite: Cite::Counter("ended_on_failed_call".into()),
            ..err(-1.0, Agency::Own)
        };
        let second = GoalError {
            goal: Some(goal.clone()),
            cite: Cite::Counter("ended_on_failed_call".into()),
            ..err(-1.0, Agency::Own)
        };
        let exposed = GoalError {
            goal: Some(goal),
            visible: true,
            ..err(-1.0, Agency::Owner)
        };
        assert_eq!(
            affect_of(&appraisal(vec![first, second, exposed])),
            Affect::Embarrassment,
            "a genuine repetition must still yield to a visible mistake in the same record"
        );
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

    // --- the assembler ---

    fn stats() -> crate::session::RunStats {
        crate::session::RunStats {
            boredom_notices: Some(0),
            ..Default::default()
        }
    }

    fn draft(id: &str, status: &str, edited: bool) -> crate::outbox::OutboxItem {
        let before = serde_json::json!({"body_markdown": "Dear Dirk,"});
        crate::outbox::OutboxItem {
            id: id.into(),
            status: status.into(),
            tool: "mail_send".into(),
            kind: crate::outbox::OutboxKind::Message,
            args: if edited {
                serde_json::json!({"body_markdown": "Dear Dr Vermeulen,"})
            } else {
                before.clone()
            },
            args_before: before,
            summary: "a reply".into(),
            session_id: Some("s1".into()),
            workspace: None,
            taint: crate::agent::Taint::default(),
            created_at: "2026-08-27T00:00:00Z".into(),
            resolved_at: None,
            reason: None,
            error: None,
        }
    }

    fn built(
        stats: &crate::session::RunStats,
        drafts: &[&crate::outbox::OutboxItem],
        interventions: &[crate::learning::Intervention],
    ) -> Appraisal {
        of_session(
            "s1",
            stats,
            &[],
            interventions,
            drafts,
            Some(stats.taint),
            "2026-08-27T00:00:00Z".into(),
        )
    }

    /// The one channel that says something went well, and the reason the
    /// record is signed at all.
    #[test]
    fn a_draft_sent_unchanged_is_a_positive_error() {
        let d = draft("o1", "sent", false);
        let a = built(&stats(), &[&d], &[]);
        assert_eq!(a.errors.len(), 1);
        assert!(a.errors[0].sign > 0.0);
        assert_eq!(a.errors[0].channel, Channel::Edit);
        assert!(a.errors[0].visible, "it went out");
        // …and still has no word for it, which is the finding above.
        assert_eq!(a.label, Affect::Neutral);
    }

    /// The owner's rewrite is what reached the recipient, not mecha's
    /// mistake — the catch is the mechanism working, and must not itself
    /// read as an exposure error. `status == "sent"` is true for this item
    /// exactly as it is for a `SentUnchanged` one, which is why `visible`
    /// has to come from `writing_outcome()` rather than from `status` alone.
    #[test]
    fn a_draft_the_owner_rewrote_is_negative_but_not_exposed() {
        let d = draft("o1", "sent", true);
        let a = built(&stats(), &[&d], &[]);
        assert_eq!(a.errors[0].sign, -1.0);
        assert!(
            !a.errors[0].visible,
            "the owner's words went out, not mecha's mistake"
        );
        assert_eq!(a.label, Affect::Neutral);
    }

    /// A queue nobody has reached is not a verdict in either direction.
    #[test]
    fn a_pending_draft_says_nothing() {
        let d = draft("o1", "pending", false);
        assert!(built(&stats(), &[&d], &[]).errors.is_empty());
    }

    /// The three counters that mean the harness worked, and the one that means
    /// the person did.
    #[test]
    fn a_run_that_was_defended_is_not_a_run_that_went_badly() {
        let mut s = stats();
        s.tool_denied = 4;
        s.blocked_sends = 2;
        s.context_overflows = Some(3);
        s.stop_cause = Some(crate::agent::StopCause::Interrupted);
        let a = built(&s, &[], &[]);
        assert!(
            a.errors.is_empty(),
            "the approver, the interlock, a recovered overflow and a person \
             pressing Ctrl-C are all the system working: {:?}",
            a.errors
        );
        assert_eq!(a.label, Affect::Neutral);
    }

    #[test]
    fn a_ceiling_is_nobody_here_s_fault_and_a_loop_is() {
        let mut ceiling = stats();
        ceiling.stop_cause = Some(crate::agent::StopCause::MaxTurns);
        assert_eq!(built(&ceiling, &[], &[]).label, Affect::Anger);

        let mut stuck = stats();
        stuck.stop_cause = Some(crate::agent::StopCause::Loop);
        let a = built(&stuck, &[], &[]);
        assert_eq!(a.errors[0].agency, Agency::Own);
    }

    /// The assembler's own version of the pure-function test above: a session
    /// with a goal that ended on a failed call *and* went nowhere is two
    /// different Own-agency counter errors on the same goal (`of_session`
    /// stamps the same goal on both), and neither should read as the other
    /// repeated.
    #[test]
    fn ended_on_failed_call_and_boredom_share_a_goal_but_are_not_frustration() {
        let mut s = stats();
        s.ended_on_failed_call = true;
        s.boredom_notices = Some(2);
        let goal = GoalRef::Task("01J8ZK".into());
        let a = of_session(
            "s1",
            &s,
            &[goal],
            &[],
            &[],
            Some(s.taint),
            "2026-08-27T00:00:00Z".into(),
        );
        assert_eq!(a.errors.len(), 2);
        assert_ne!(
            a.label,
            Affect::Frustration,
            "a failed call and a stuck approach are two different symptoms, not one repeated"
        );
    }

    /// Absent is not zero: a row from before the sensor is not a run that was
    /// never stuck.
    #[test]
    fn an_unrecorded_boredom_counter_contributes_nothing() {
        let mut none = stats();
        none.boredom_notices = None;
        assert!(built(&none, &[], &[]).errors.is_empty());

        let mut some = stats();
        some.boredom_notices = Some(2);
        assert_eq!(built(&some, &[], &[]).errors.len(), 1);
    }

    #[test]
    fn a_taint_carried_by_the_run_decides_the_appraisal_s_provenance() {
        let mut s = stats();
        s.taint = crate::agent::Taint {
            private: true,
            untrusted: true,
        };
        assert_eq!(
            built(&s, &[], &[]).origin,
            crate::learning::Origin::Untrusted
        );
    }

    /// `classify_origin`'s fail-closed `None` arm has to stay reachable from
    /// here: a caller that could not establish end-of-session coverage (a
    /// torn transcript, one recorded before checkpoints existed) must not
    /// read as provably clean just because nothing was passed.
    #[test]
    fn no_established_coverage_classifies_untrusted_rather_than_clean() {
        let s = stats();
        assert_eq!(
            of_session("s1", &s, &[], &[], &[], None, "t".into()).origin,
            crate::learning::Origin::Untrusted
        );
    }

    /// A followup is a later user turn the miner cannot tell from an ordinary
    /// question, and `counterfactual.rs` already declines to grade it for
    /// exactly that reason — this channel must decline the same way, or an
    /// unremarkable multi-turn chat reads as a run that went badly once per
    /// turn. `Steer` and `Denial` are unambiguous and keep their sign.
    #[test]
    fn a_followup_contributes_no_signed_error() {
        let followup = crate::learning::Intervention {
            trigger: crate::learning::Trigger::Followup,
            context: String::new(),
            text: "and another thing".into(),
            aftermath: String::new(),
            at: 4,
            tools_before: vec![],
            tools_after: vec![],
        };
        assert!(built(&stats(), &[], std::slice::from_ref(&followup))
            .errors
            .is_empty());

        let steer = crate::learning::Intervention {
            trigger: crate::learning::Trigger::Steer,
            ..followup
        };
        assert_eq!(built(&stats(), &[], &[steer]).errors.len(), 1);
    }

    // --- what a probe buys ---

    fn intervention() -> GoalError {
        GoalError {
            goal: None,
            channel: Channel::Intervention,
            sign: -1.0,
            agency: Agency::Owner,
            visible: false,
            controllable: None,
            cite: Cite::Turn(4),
        }
    }

    /// The case §8 wants and the one the corpus cannot currently label: the
    /// owner had to steer, and without them the run would have gone elsewhere.
    /// Something in this machine could have gone differently.
    #[test]
    fn a_steer_that_mattered_makes_the_error_the_agents_own() {
        let mut e = intervention();
        apply_probe(&mut e, Probe::Mattered);
        assert_eq!(e.agency, Agency::Own);
        assert_eq!(e.controllable, Some(true));

        let mut a = appraisal(vec![e]);
        relabel(&mut a);
        assert_eq!(a.label, Affect::Regret);
    }

    /// The owner stepped in and the run was already going the right way. A
    /// real cost, and nothing the agent could have done about it.
    #[test]
    fn a_steer_that_changed_nothing_stays_the_owners() {
        let mut e = intervention();
        apply_probe(&mut e, Probe::Redundant);
        assert_eq!(e.agency, Agency::Owner, "the agent did not cause this");
        assert_eq!(e.controllable, Some(false));

        let mut a = appraisal(vec![e]);
        relabel(&mut a);
        assert_eq!(a.label, Affect::Disappointment);
    }

    /// A replay that departed before the probe point never posed the question,
    /// and an answer to a question nobody asked is worse than none.
    #[test]
    fn an_inconclusive_probe_changes_nothing() {
        let mut e = intervention();
        apply_probe(&mut e, Probe::Inconclusive);
        assert_eq!(e, intervention());

        let mut a = appraisal(vec![e]);
        relabel(&mut a);
        assert_eq!(a.label, Affect::Neutral);
    }

    /// The magnitude is evidence the probe does not speak to, so it is left
    /// alone in both directions.
    #[test]
    fn a_probe_never_moves_the_sign() {
        for probe in [Probe::Mattered, Probe::Redundant, Probe::Inconclusive] {
            let mut e = intervention();
            apply_probe(&mut e, probe);
            assert_eq!(e.sign, -1.0);
        }
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

    /// `goal::de_lenient_vec`'s own claim — one unrecognised entry costs the
    /// reference and nothing around it — exercised through `Appraisal`
    /// itself rather than only through `parse_lenient` directly. There is no
    /// store for this record yet, so today `serde` on `goals` is reachable
    /// only from a test; without one, a future binary that discards a whole
    /// appraisal over one bad reference would have nothing to catch it.
    #[test]
    fn an_unrecognised_goal_kind_costs_only_itself() {
        let json = r#"{
            "id": "s1",
            "session_id": "s1",
            "goals": ["task:a", "banana:b"],
            "errors": [],
            "label": "neutral",
            "origin": "clean",
            "created_at": "t"
        }"#;
        let a: Appraisal = serde_json::from_str(json).unwrap();
        assert_eq!(a.goals, vec![GoalRef::Task("a".into())]);
    }
}
