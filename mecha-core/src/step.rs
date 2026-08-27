//! What a finished step actually did — the deterministic half of step
//! appraisal (`docs/GOAL-SYSTEM-DESIGN.md` §5.5).
//!
//! **A step is marked done by the agent, and nothing checked it.** The
//! symmetry with the tier above is the whole argument: a *board task* is
//! closed by the owner (`TASK-AGENT-DESIGN.md` D6), so a person is the check;
//! a *todo step* is closed by the model, so there is no person and the check
//! has to be structural. D5's rule — state is derived from the record, never
//! self-reported — reaches one tier further down than it was written for.
//!
//! **Pure, and unit-tested rather than trialled**, for `compact.rs`'s reason:
//! getting it wrong is silent. A finding that fires on honest work is a line
//! the model learns to skip, which is how a check that protects nothing
//! survives; a finding that never fires is indistinguishable from a plan that
//! always lands.
//!
//! Two readings only, and the omissions are deliberate. §5.5's table lists
//! five signals; the two here — *no calls at all* and *the last call did not
//! succeed* — are **facts about the span**. The other three (a span far longer
//! than its siblings, a verify-shaped call that passed, the same target read
//! repeatedly) are *comparisons*, and each needs either a threshold nobody has
//! measured here or a guess about what a tool call meant. A threshold that
//! cries wolf is doctor's named failure, and the escalation to a model that
//! would settle the ambiguous cases is rung 7's, not this one's. The
//! same-target reading is boredom's (§9.1) and belongs one mechanism over.
//!
//! What this module does **not** do is act. The finding is rendered onto the
//! `todo` result and the plan action — accept, revise the step, revise the
//! plan, escalate — is the model's, because the plan is the model's. The
//! harness has no way to author a decomposition and no business having one.

use crate::agent::ToolCallTrace;

/// How one executed call ended, as far as the run's own record knows.
///
/// **`Refused` is not `Failed`, and the split is load-bearing.** A denied
/// trace carries `is_error: true` *as well as* `denied: true`, so any counter
/// spelled `is_error` alone reports the approver doing its job as the step
/// going wrong — the same miscount the eval rig names on
/// `ended_on_failed_call`, which is why the fold below spells it
/// `unknown || (is_error && !denied)`. A step whose calls were refused was
/// *blocked*; a step whose calls failed did not land. Telling the model the
/// first is the second would send it to fix code that is working.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Failed,
    Refused,
}

impl Outcome {
    fn of(call: &ToolCallTrace) -> Self {
        if call.denied {
            // Includes a call withheld by policy: nothing ran, and nobody
            // claimed it did.
            Outcome::Refused
        } else if call.unknown || call.is_error {
            // A named tool that does not exist is the model's mistake rather
            // than the environment's refusal, and it counts with the failures
            // for the same reason the eval rig counts it there.
            Outcome::Failed
        } else {
            // A staged call is a success: the draft was written, and the send
            // is waiting on a person rather than having gone wrong.
            Outcome::Ok
        }
    }
}

/// The run's work so far, as of one tool call.
///
/// Cumulative, and only ever read as a **difference** between two points — the
/// turn a step went `in_progress` and the turn it was marked done. That is why
/// it is a handful of integers rather than a list: the span is arithmetic, and
/// keeping the calls themselves would make this a second copy of the trace.
///
/// Stamped on [`ToolCtx`](crate::tool::ToolCtx) by the loop, which does not
/// know which tool cares — the `taint` and `call_id` precedent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Work {
    /// Every attempt, refusals included. A step that tried three times and was
    /// refused three times is blocked, not empty, and counting only what ran
    /// would report it as having done nothing.
    pub calls: u32,
    pub failed: u32,
    pub refused: u32,
    /// How the most recent attempt ended. `None` before the run makes one.
    pub last: Option<Outcome>,
    /// Calls approved in *this* turn whose results are not back yet — the
    /// siblings of the call reading this.
    ///
    /// mecha executes a turn's calls concurrently, so a model that does the
    /// work and ticks the box in one batch has that work invisible to the fold
    /// below. Without this the commonest efficient shape in the corpus would
    /// report as the null step, which is the false positive that would teach
    /// people to ignore the reading.
    pub in_flight: u32,
    /// Which run these counters belong to.
    ///
    /// **The trace is per run and a conversation is many runs.** In chat and
    /// the TUI one submission is one run, so the counters restart at zero
    /// while the plan carries on — and a step started before the user last
    /// spoke would difference against a larger number, saturate to zero and
    /// report as the null step. That is the loudest reading this module has,
    /// firing on the commonest shape there is, which is how a check gets
    /// switched off. So a mark from another run is *unmeasurable* rather than
    /// empty, and [`Work::since`] says so by returning nothing.
    ///
    /// Only inequality is ever read, which is what makes a process-local
    /// counter enough: two runs in one process must differ, and nothing
    /// compares this across processes or across a restart.
    pub run: u64,
}

/// A fresh run identity. Monotonic within the process, meaningless outside it.
pub fn next_run() -> u64 {
    static RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl Work {
    /// Fold the run's trace so far.
    ///
    /// Called once per turn, not once per call: the numbers are the same for
    /// every call in a batch, and the walk is over every call the run has
    /// made.
    pub fn of(trace: &[ToolCallTrace]) -> Self {
        let mut work = Work::default();
        for call in trace {
            let outcome = Outcome::of(call);
            work.calls += 1;
            match outcome {
                Outcome::Failed => work.failed += 1,
                Outcome::Refused => work.refused += 1,
                Outcome::Ok => {}
            }
            work.last = Some(outcome);
        }
        work
    }

    pub fn with_in_flight(mut self, n: u32) -> Self {
        self.in_flight = n;
        self
    }

    pub fn in_run(mut self, run: u64) -> Self {
        self.run = run;
        self
    }

    /// Work done since `start`, less `bookkeeping` calls the caller knows were
    /// its own.
    ///
    /// The adjustment exists because the only caller is a tool that appears in
    /// its own count. A model that revises its plan three times mid-step would
    /// otherwise show three calls of "work" for a step where nothing happened
    /// — the null step masked by the bookkeeping that announced it. The
    /// argument for putting the subtraction here rather than in the loop is
    /// the loop's own invariant: it stamps run state without learning which
    /// tool reads it, so "the plan tool is not work" is a judgement only the
    /// plan tool can make, and this is where the arithmetic it needs lives.
    ///
    /// `None` when `start` was taken in another run — see [`Work::run`]. An
    /// unmeasurable span supports no finding, which is doctor's dash one
    /// mechanism over: could-not-look and nothing-happened are opposite
    /// answers.
    pub fn since(&self, start: Work, bookkeeping: u32) -> Option<Span> {
        if start.run != self.run {
            return None;
        }
        Some(Span {
            calls: self
                .calls
                .saturating_sub(start.calls)
                .saturating_sub(bookkeeping),
            failed: self.failed.saturating_sub(start.failed),
            refused: self.refused.saturating_sub(start.refused),
            last: self.last,
            in_flight: self.in_flight,
        })
    }
}

/// What happened between a step starting and the model calling it done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub calls: u32,
    pub failed: u32,
    pub refused: u32,
    /// The run's most recent finished attempt — which is the *span's* most
    /// recent one whenever the span holds any, since calls happen in order.
    /// Meaningless when `calls` is zero, and [`appraise`] reads it only after
    /// establishing that it is not.
    pub last: Option<Outcome>,
    /// Siblings still running. Any of them may be the work, or the recovery,
    /// so a span holding one supports no finding at all.
    pub in_flight: u32,
}

/// What the span says about the step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finding {
    /// The common path, and it says nothing. Silence is the point: this rides
    /// on a tool result in an append-only transcript, so a line per completed
    /// step is bulk carried for the rest of the run in exchange for confirming
    /// what the model already believes.
    Landed,
    /// Nothing was attempted — the step-level null run, which `WORK_FLOOR`
    /// exists to catch one tier up.
    Null,
    /// The last thing tried failed, and nothing after it succeeded.
    EndedOnFailure,
    /// The last thing tried was refused. The step was blocked, not done.
    EndedOnRefusal,
}

/// The deterministic reading. No model, no threshold, no tuned constant.
pub fn appraise(span: Span) -> Finding {
    // Unknown beats every other reading. A sibling still in flight can be the
    // work the span looks empty without, or the recovery after the failure
    // that ended it — so the honest answer is no finding rather than the
    // finding the visible half would support. This is the same direction the
    // taint snapshot takes on uncovered runs: an absence is not evidence.
    if span.in_flight > 0 {
        return Finding::Landed;
    }
    if span.calls == 0 {
        return Finding::Null;
    }
    // Only the *last* attempt counts, which is the eval rig's rule for
    // `ended_on_failed_call` one tier down: a failure among successes is
    // recovery, and recovery is the model working.
    match span.last {
        Some(Outcome::Failed) => Finding::EndedOnFailure,
        Some(Outcome::Refused) => Finding::EndedOnRefusal,
        _ => Finding::Landed,
    }
}

impl Finding {
    /// One line for the `todo` result, or nothing at all.
    ///
    /// **Wording is load-bearing**, on `EMPTY_TURN_NUDGE`'s and `ask_user`'s
    /// evidence: a vague nudge makes a model restart work it had done, and
    /// "use your best judgment" measurably makes it invent. So each line
    /// states the fact first and offers exactly one continuation — and the
    /// null line offers the model the *reading* rather than the verdict,
    /// because a step that was genuinely a decision made no calls and is not
    /// wrong. Naming that case is what keeps the line from being one the model
    /// learns to skip.
    ///
    /// No tool is named in any of them. `ask_user` is registered only by a
    /// front-end that owns a human, so an unattended run told to ask would
    /// spend a turn on a call that can only fail — `compact`'s own description
    /// declines to name `todo` for the same reason.
    pub fn line(self, step: &str, again: bool) -> Option<String> {
        let step = ellipsize(step, 60);
        let body = match self {
            Finding::Landed => return None,
            Finding::Null => format!(
                "step \"{step}\" was marked done with no tool calls behind it. \
                 If it was a decision rather than work, that is what it should say; \
                 if it was work, it has not been done yet."
            ),
            Finding::EndedOnFailure => format!(
                "step \"{step}\" was marked done with its last call still failing, \
                 and nothing after it succeeded. Check that it landed before moving on."
            ),
            Finding::EndedOnRefusal => format!(
                "step \"{step}\" was marked done with its last call refused. \
                 It was blocked rather than finished — the plan should say which."
            ),
        };
        Some(if again {
            // §5.5's bound: one revision per step. A second identical reading
            // means the revision did not work, and a third attempt is how
            // "revise the step" becomes the local minimum the drive above it
            // exists to escape — arriving through the door meant to prevent
            // it.
            format!(
                "{body} This is the second time this step has come back that way; \
                 rather than trying it again, say what you need to get past it."
            )
        } else {
            body
        })
    }
}

/// Keep a long step from being most of the line it appears in.
///
/// On a char boundary, because a step's content is whatever the model typed
/// and slicing a multi-byte character panics — in the one code path that runs
/// on every plan revision of every run.
fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max - 1).collect();
    format!("{}…", head.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(is_error: bool, denied: bool, unknown: bool) -> ToolCallTrace {
        ToolCallTrace {
            name: "shell".into(),
            input: json!({}),
            is_error,
            denied,
            unknown,
            staged: false,
        }
    }

    fn ok() -> ToolCallTrace {
        call(false, false, false)
    }
    fn failed() -> ToolCallTrace {
        call(true, false, false)
    }
    fn denied() -> ToolCallTrace {
        // As the loop writes it: a denial is an error *and* a denial.
        call(true, true, false)
    }

    #[test]
    fn a_denial_is_never_counted_as_a_failure() {
        let work = Work::of(&[ok(), denied()]);
        assert_eq!(work.calls, 2);
        assert_eq!(
            work.failed, 0,
            "the approver doing its job is not a failure"
        );
        assert_eq!(work.refused, 1);
        assert_eq!(work.last, Some(Outcome::Refused));

        // And the two produce different findings, which is the whole reason
        // the split exists: one says fix your work, the other says you were
        // blocked.
        let refused = appraise(work.since(Work::default(), 0).unwrap());
        assert_eq!(refused, Finding::EndedOnRefusal);
        let broke = appraise(
            Work::of(&[ok(), failed()])
                .since(Work::default(), 0)
                .unwrap(),
        );
        assert_eq!(broke, Finding::EndedOnFailure);
    }

    #[test]
    fn an_unknown_tool_counts_with_the_failures() {
        let work = Work::of(&[call(true, false, true)]);
        assert_eq!(work.failed, 1);
        assert_eq!(work.refused, 0);
    }

    #[test]
    fn a_step_with_nothing_behind_it_is_the_null_step() {
        let start = Work::of(&[ok(), ok()]);
        // Two turns later, and not one call in between.
        let now = start;
        assert_eq!(appraise(now.since(start, 0).unwrap()), Finding::Null);
    }

    #[test]
    fn plan_bookkeeping_does_not_count_as_work() {
        let start = Work::default();
        // Three calls since the step started — all of them the plan tool
        // rewriting the list. Nothing was done.
        let now = Work::of(&[ok(), ok(), ok()]);
        assert_eq!(
            appraise(now.since(start, 3).unwrap()),
            Finding::Null,
            "a step whose whole span is plan revision did nothing"
        );
        // One real call among them and it is no longer null.
        assert_eq!(appraise(now.since(start, 2).unwrap()), Finding::Landed);
    }

    #[test]
    fn a_failure_recovered_from_is_the_model_working() {
        let start = Work::default();
        let now = Work::of(&[failed(), ok()]);
        let span = now.since(start, 0).unwrap();
        assert_eq!(span.failed, 1, "the failure is still counted");
        assert_eq!(
            appraise(span),
            Finding::Landed,
            "only the last attempt decides; recovery is not a finding"
        );
    }

    #[test]
    fn a_sibling_still_running_supports_no_finding() {
        // The work and the tick in one batch: nothing has landed in the trace
        // yet, so the visible half says null and the honest answer is nothing.
        let empty = Work::default().with_in_flight(1);
        assert_eq!(
            appraise(empty.since(Work::default(), 0).unwrap()),
            Finding::Landed
        );

        // Same for a failure a sibling may still be recovering from.
        let failing = Work::of(&[failed()]).with_in_flight(1);
        assert_eq!(
            appraise(failing.since(Work::default(), 0).unwrap()),
            Finding::Landed
        );
    }

    #[test]
    fn a_failure_before_the_step_started_is_not_this_step_s() {
        let start = Work::of(&[failed()]);
        let now = Work::of(&[failed(), ok(), ok()]);
        let span = now.since(start, 0).unwrap();
        assert_eq!(span.failed, 0);
        assert_eq!(appraise(span), Finding::Landed);
    }

    /// The chat shape: a step started before the user last spoke. The
    /// counters restarted with the run, so the span is unmeasurable — and
    /// saying so is the whole point, because the arithmetic alone would
    /// saturate to zero and announce the null step on ordinary work.
    #[test]
    fn a_mark_from_another_run_is_unmeasurable_rather_than_empty() {
        let first = Work::of(&[ok(), ok(), ok()]).in_run(1);
        let second = Work::of(&[ok()]).in_run(2);
        assert_eq!(second.since(first, 0), None);

        // And within one run it measures as usual.
        assert!(Work::of(&[ok(), ok(), ok(), ok()])
            .in_run(1)
            .since(first, 0)
            .is_some());
    }

    #[test]
    fn the_common_path_says_nothing() {
        assert_eq!(Finding::Landed.line("read the config", false), None);
    }

    #[test]
    fn a_second_identical_reading_stops_asking_for_a_revision() {
        let first = Finding::Null.line("fix the port", false).unwrap();
        let second = Finding::Null.line("fix the port", true).unwrap();
        assert!(first.contains("fix the port") && !first.contains("second time"));
        assert!(second.contains("second time"));
        // Neither names a tool: an unattended run has no `ask_user` to reach
        // for, and pointing at an absent tool spends a turn on a call that can
        // only fail.
        for line in [&first, &second] {
            assert!(!line.contains("ask_user") && !line.contains('`'));
        }
    }

    #[test]
    fn a_long_step_is_cut_on_a_char_boundary() {
        let step = "é".repeat(200);
        let line = Finding::Null.line(&step, false).unwrap();
        assert!(line.contains('…'));
    }
}
