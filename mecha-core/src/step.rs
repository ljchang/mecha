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
use anyhow::{Context, Result};
use serde::Deserialize;

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
    /// Successful calls that look like they verified something — a `shell`
    /// call whose command matches a small test-runner-shaped keyword list.
    /// See [`looks_like_verification`]. Folded here rather than kept as a
    /// raw trace for the same reason `calls`/`failed`/`refused` are: the span
    /// is arithmetic, and a tool asking "was there a check in this span"
    /// needs a count, not the calls themselves.
    pub verify_like: u32,
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
    /// Calls settled *this turn* without becoming approved work, whose
    /// target step is unknown to the harness. Named for the commonest case
    /// (the approver, a hook, the interlock) but not only that: an unknown
    /// or withheld tool name and a failed staging attempt settle the same
    /// way, without ever being denied by anyone.
    ///
    /// Any of these is settled the instant it happens — unlike `in_flight`
    /// it is already in the trace — but the batch it happened in is exactly
    /// the shape `in_flight` exists for: a model ticking a step and
    /// reaching for the next one's tool in the same turn. `trace.push` for
    /// one of these runs ahead of the calls it approved, so `Work::of` folds
    /// it in as the raw trace's last entry regardless of which call it sat
    /// beside — blaming *this* step for an outcome that belongs to the
    /// next one. Carried alongside `in_flight` for the same reason: a batch
    /// holding either supports no finding at all.
    pub denied: u32,
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
                Outcome::Ok => {
                    if looks_like_verification(call) {
                        work.verify_like += 1;
                    }
                }
            }
            work.last = Some(outcome);
        }
        work
    }

    pub fn with_in_flight(mut self, n: u32) -> Self {
        self.in_flight = n;
        self
    }

    pub fn with_denied(mut self, n: u32) -> Self {
        self.denied = n;
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
    /// **`last` is the caller's to supply, and `self.last` is the wrong
    /// answer.** `self.last` is the raw trace's most recent entry, which is
    /// this same bookkeeping tool's own call whenever one lands last — a
    /// successful revision masks an earlier failure (`EndedOnFailure` never
    /// fires), and a *rejected* one, which never reaches this method's caller
    /// at all, reads as the step's own failure (`EndedOnFailure` fires on
    /// work that landed). `bookkeeping` is a count and cannot say which
    /// position it occupied, so only a caller tracking its own calls as they
    /// happen — [`crate::tool::todo::Tracked`] does, incrementally — can name
    /// the outcome that actually belongs to the span.
    ///
    /// `None` when `start` was taken in another run — see [`Work::run`]. An
    /// unmeasurable span supports no finding, which is doctor's dash one
    /// mechanism over: could-not-look and nothing-happened are opposite
    /// answers.
    pub fn since(&self, start: Work, bookkeeping: u32, last: Option<Outcome>) -> Option<Span> {
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
            verify_like: self.verify_like.saturating_sub(start.verify_like),
            last,
            in_flight: self.in_flight,
            denied: self.denied,
        })
    }
}

/// What happened between a step starting and the model calling it done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub calls: u32,
    pub failed: u32,
    pub refused: u32,
    /// See [`Work::verify_like`]. Read by [`escalation_candidate`], never by
    /// [`appraise`] — a verify-shaped call is evidence for the escalation to
    /// weigh, not a fact the deterministic reading changes on.
    pub verify_like: u32,
    /// The run's most recent finished attempt — which is the *span's* most
    /// recent one whenever the span holds any, since calls happen in order.
    /// Meaningless when `calls` is zero, and [`appraise`] reads it only after
    /// establishing that it is not.
    pub last: Option<Outcome>,
    /// Siblings still running. Any of them may be the work, or the recovery,
    /// so a span holding one supports no finding at all.
    pub in_flight: u32,
    /// Siblings denied this turn. See [`Work::denied`] — the denial cannot be
    /// attributed to this step over any other in the same batch, so a span
    /// holding one supports no finding either.
    pub denied: u32,
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
    // A sibling denied this turn gets the same treatment: the refusal is
    // settled, but which step it belongs to is not.
    if span.in_flight > 0 || span.denied > 0 {
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

// ─── The model half: escalation (§5.5, rung 7) ──────────────────────────────
//
// Two of §5.5's five signals are *comparisons* rather than facts about one
// span, and each needs either a threshold nobody has measured or a guess
// about what a call meant — which is why the deterministic reading above
// declines both. What follows is the escalation itself: a cheap deterministic
// pre-filter decides *whether* to ask (never the answer), and one quarantined
// model call settles the ambiguous case.
//
// **Live, not offline.** Unlike the appraiser (§5.1), which reviews a
// finished session from outside it, a step's plan action has to reach the
// *same* run before it wastes more turns on a bad decomposition — so this
// has no CLI surface of its own. `agent.rs`'s loop calls `escalate` directly,
// the same way it already calls the compaction summariser, and folds the
// verdict into the turn the way `boredom.rs`'s notices already do.
//
// **What this may see, and what it may never say back.** The step's own
// text (and its siblings') is this same model's own prior plan output —
// already fully trusted in-context every turn, not a new place for
// third-party text to reach a decision, which is what made the appraiser's
// evidence numbers-only. But the model's free-text `reasoning` here never
// re-enters the conversation: a model's paraphrase of step text it just read
// is `frontdoor`'s "a paraphrase of an injection is the injection rearranged"
// risk, arriving through the one channel that *does* reach context.
// `templated_nudge` is fully templated by which trigger fired; the model
// only ever decides the binary accept/revise_plan.

/// Does this successful call look like it checked something, rather than
/// merely done something? A coarse keyword match on a `shell` command —
/// argued, not measured, on this module's own convention for its constants.
/// Only `shell` is matched: a project's own test runner, wired up as an MCP
/// tool, has no name this module could know in advance, and guessing at one
/// would be the same mistake as guessing at a threshold.
fn looks_like_verification(call: &ToolCallTrace) -> bool {
    const KEYWORDS: &[&str] = &[
        "cargo test",
        "pytest",
        "npm test",
        "npm run test",
        "yarn test",
        "pnpm test",
        "make test",
        "go test",
        "rspec",
        "jest",
    ];
    if call.name != "shell" {
        return false;
    }
    let Some(command) = call.input.get("command").and_then(|v| v.as_str()) else {
        return false;
    };
    let command = command.to_ascii_lowercase();
    KEYWORDS.iter().any(|k| command.contains(k))
}

/// A step's own words that read as a checkable claim. Argued, not measured,
/// same convention as [`looks_like_verification`]'s keyword list.
fn reads_as_a_verification_claim(step: &str) -> bool {
    // Single words, matched on a word boundary — `"test"` as a plain
    // substring also matches `"latest"`, `"attest"`, `"contest"`, of which
    // `"latest"` is the one that actually turns up in plans ("pull the
    // latest changes"). Multi-word phrases below stay substring matches:
    // they cannot collide with an unrelated word the same way.
    const WORDS: &[&str] = &["test", "verify", "confirm", "ensure"];
    const PHRASES: &[&str] = &["check that", "make sure"];
    let step = step.to_ascii_lowercase();
    let tokens: std::collections::HashSet<&str> = step
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    WORDS.iter().any(|k| tokens.contains(k)) || PHRASES.iter().any(|p| step.contains(p))
}

/// Which comparison flagged a landed step for a second opinion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscalationReason {
    /// This step's span took far more calls than the plan's other completed
    /// steps — maybe the decomposition was wrong, maybe the work was just
    /// harder. The model, not a threshold, tells the two apart.
    SpanOutlier,
    /// The step's own words read as a checkable claim, but nothing in its
    /// span looks like a check. The eval rig's "grade the artifact, never
    /// the claim", one tier down.
    UnverifiedClaim,
}

/// What the quarantined escalation call is handed. See the module note above
/// on why the step's own text is safe to include here in a way an
/// appraiser's evidence (§5.1) could not be.
#[derive(Debug, Clone, PartialEq)]
pub struct StepEscalation {
    pub reason: EscalationReason,
    pub step: String,
    /// A sample of other completed steps in the same plan, most recent
    /// first — never the whole history, which `Tracked` bounds but does not
    /// make small. Empty for [`EscalationReason::UnverifiedClaim`], which
    /// needs no comparison.
    pub siblings: Vec<String>,
    pub calls: u32,
    /// The mean call count of the completed steps this was compared
    /// against. Only set for [`EscalationReason::SpanOutlier`].
    pub sibling_mean_calls: Option<f32>,
    /// How many completed steps `sibling_mean_calls` is a mean *over* —
    /// `completed.len()` at the time of the comparison, not `siblings.len()`.
    /// The two diverge once the plan has more completed steps than
    /// `ESCALATION_SIBLING_SAMPLE`: the mean is still over all of them, but
    /// `siblings` is a truncated sample for the model to read, and stating
    /// the sample's length beside the full mean would describe a mean over
    /// 5 steps that was actually taken over 20.
    pub sibling_count: usize,
}

/// A span is a clear enough outlier to be worth a second opinion at this
/// ratio against the mean of the plan's other completed steps...
const SPAN_OUTLIER_RATIO: f32 = 3.0;
/// ...and at least this many calls outright, so a plan of tiny steps does
/// not escalate on a difference of one or two calls that means nothing.
const SPAN_OUTLIER_FLOOR: u32 = 6;
/// Fewer completed steps than this and there is no "the plan's other steps"
/// to compare against yet.
const SPAN_OUTLIER_MIN_SIBLINGS: usize = 2;
/// How many prior steps' text ride along as context — enough to judge "does
/// this decomposition look right", not the whole plan's history.
const ESCALATION_SIBLING_SAMPLE: usize = 5;

/// The escalation's own pre-filter: cheap, deterministic, and it only ever
/// decides *whether to ask*, never the answer.
///
/// **The caller must already know `appraise(span) == Finding::Landed`.** A
/// step with its own deterministic finding needs no second opinion, and this
/// function takes that as given rather than re-deriving it, because deriving
/// it needs the same `span` this function already has — asking the caller to
/// check first is one comparison, not two.
pub fn escalation_candidate(
    span: Span,
    step: &str,
    completed: &[(String, u32)],
) -> Option<StepEscalation> {
    if completed.len() >= SPAN_OUTLIER_MIN_SIBLINGS {
        let mean = completed.iter().map(|(_, n)| *n as f32).sum::<f32>() / completed.len() as f32;
        if span.calls as f32 >= mean * SPAN_OUTLIER_RATIO && span.calls >= SPAN_OUTLIER_FLOOR {
            return Some(StepEscalation {
                reason: EscalationReason::SpanOutlier,
                step: step.to_string(),
                siblings: completed
                    .iter()
                    .rev()
                    .take(ESCALATION_SIBLING_SAMPLE)
                    .map(|(s, _)| s.clone())
                    .collect(),
                calls: span.calls,
                sibling_mean_calls: Some(mean),
                sibling_count: completed.len(),
            });
        }
    }
    if reads_as_a_verification_claim(step) && span.verify_like == 0 {
        return Some(StepEscalation {
            reason: EscalationReason::UnverifiedClaim,
            step: step.to_string(),
            siblings: Vec::new(),
            calls: span.calls,
            sibling_mean_calls: None,
            sibling_count: 0,
        });
    }
    None
}

/// What the escalation decided: nothing further, or a plan-revision nudge is
/// worth surfacing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepVerdict {
    Accept,
    RevisePlan,
}

/// The prompt the quarantined pass runs. Reasoning first, the typed field
/// last — the front door's and the appraiser's own finding: constrained
/// output degrades reasoning when the answer precedes the thinking.
pub fn escalation_prompt(escalation: &StepEscalation) -> String {
    let question = match escalation.reason {
        EscalationReason::SpanOutlier => format!(
            "This step just finished after {} tool calls. The plan's other completed \
             steps averaged {:.1} calls each ({} of them). Does the size of this step \
             suggest the plan's decomposition should be revised for the steps still \
             ahead, or was this step just harder than the others with nothing wrong \
             in how the plan divided the work?",
            escalation.calls,
            escalation.sibling_mean_calls.unwrap_or(0.0),
            escalation.sibling_count,
        ),
        EscalationReason::UnverifiedClaim => format!(
            "This step was marked done. Its own wording reads as claiming something \
             was tested, verified, or confirmed, but none of its {} tool call(s) \
             looked like a check — grade the calls, not the claim. Does this look \
             like the step actually verified what it says, or like an unverified \
             claim the plan should revisit?",
            escalation.calls,
        ),
    };
    let siblings = if escalation.siblings.is_empty() {
        String::new()
    } else {
        format!(
            "\n\nThe {} most recent of them, for context:\n{}",
            escalation.siblings.len(),
            escalation
                .siblings
                .iter()
                .map(|s| format!("- {s}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    format!(
        "You are reviewing one step of your own plan from outside the run that made \
         it — you have no tools and cannot act, only judge.\n\n\
         The step: \"{}\"\n\
         {question}{siblings}\n\n\
         Return exactly this JSON and nothing else:\n\
         {{\n  \"reasoning\": \"one or two sentences\",\n  \
         \"verdict\": \"accept | revise_plan\"\n}}\n\n\
         `accept` is the common, correct answer when the work looks sound; \
         `revise_plan` only when there is a real reason to reconsider the \
         decomposition.",
        escalation.step,
    )
}

/// Parse what the escalation returned.
///
/// The bracket-matching leniency is `frontdoor::parse_extraction`'s: models
/// wrap JSON in prose and code fences however firmly they are asked not to.
/// `reasoning`, if present, is logged at `debug` and never returned — see the
/// module note on why it must not reach [`templated_nudge`].
pub fn parse_step_verdict(text: &str) -> Result<StepVerdict> {
    let start = text
        .find('{')
        .context("the escalation returned no JSON object")?;
    let end = text
        .rfind('}')
        .context("the escalation returned no JSON object")?;
    if end <= start {
        anyhow::bail!("the escalation returned no JSON object");
    }

    #[derive(Deserialize)]
    struct Wire {
        #[serde(default)]
        reasoning: Option<String>,
        verdict: String,
    }
    let wire: Wire = serde_json::from_str(&text[start..=end]).with_context(|| {
        let cut = crate::text::char_boundary_at_or_before(text, end.min(start + 400) + 1);
        format!("parsing the escalation's verdict: {}", &text[start..cut])
    })?;

    if let Some(reasoning) = &wire.reasoning {
        tracing::debug!(%reasoning, "step escalation reasoning (never shown to the model)");
    }
    match wire.verdict.as_str() {
        "accept" => Ok(StepVerdict::Accept),
        "revise_plan" => Ok(StepVerdict::RevisePlan),
        other => anyhow::bail!("the escalation returned an unrecognised verdict `{other}`"),
    }
}

// **The quarantined call itself is `agent.rs`'s to make, not this module's.**
// This is the one place rung 7's two halves genuinely differ: the appraiser
// (§5.1) is offline, so a bare `&dyn Provider` and a plain retry loop are the
// whole story. This escalation runs *inside* a live, cancellable run, so it
// has to go through `Agent::complete` the same way `compact`'s summariser and
// `compact_validate`'s check already do — that is what wires it into the
// run's own cancellation token and folds its spend into `RunStats`, neither
// of which a bare provider call can reach. `escalation_prompt` and
// `parse_step_verdict` above are what stay pure and testable here; the retry
// loop that drives them lives beside `compact` in `agent.rs`.

/// The nudge folded into the run when the escalation says `revise_plan`.
///
/// Fully templated — the model's own free-text reasoning never reaches this
/// output, on `frontdoor`'s rule one door over: a paraphrase of text the
/// model just read is the same risk as the text itself, arriving through
/// the one channel that re-enters context. Wording follows `Finding::line`'s
/// own discipline: state the fact, offer one continuation.
/// Marks a folded nudge as the harness's own words, on `boredom::NOTICE_STEM`'s
/// exact precedent: `agent::is_harness_voice` is a closed list the learning
/// miner filters every tool-result message's text through before deciding
/// whether it is a user's `Steer`/`Followup` intervention. Without an entry
/// here, `templated_nudge`'s output — folded into the very message
/// `escalation_candidate` also carries tool results in — would be mined as if
/// a person had typed it, `escalation.step` and all, and could ride into a
/// future prompt as a `Clean`-origin learned rule derived from nobody's words.
pub const STEP_ESCALATION_STEM: &str = "A second opinion on your plan:";

pub fn templated_nudge(escalation: &StepEscalation) -> String {
    let step = ellipsize(&escalation.step, 60);
    let body = match escalation.reason {
        EscalationReason::SpanOutlier => format!(
            "step \"{step}\" took {} tool call(s) against the plan's other completed \
             steps' average of {:.1} — worth checking whether the remaining steps in \
             the plan need to be broken down differently, or re-scoped.",
            escalation.calls,
            escalation.sibling_mean_calls.unwrap_or(0.0),
        ),
        EscalationReason::UnverifiedClaim => format!(
            "step \"{step}\" reads as claiming something was tested or verified, but \
             nothing in its tool calls looked like a check — worth confirming it \
             actually landed before moving on."
        ),
    };
    format!("{STEP_ESCALATION_STEM} {body}")
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
        let refused = appraise(work.since(Work::default(), 0, work.last).unwrap());
        assert_eq!(refused, Finding::EndedOnRefusal);
        let broke = appraise(
            Work::of(&[ok(), failed()])
                .since(Work::default(), 0, Some(Outcome::Failed))
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
        assert_eq!(
            appraise(now.since(start, 0, now.last).unwrap()),
            Finding::Null
        );
    }

    #[test]
    fn plan_bookkeeping_does_not_count_as_work() {
        let start = Work::default();
        // Three calls since the step started — all of them the plan tool
        // rewriting the list. Nothing was done.
        let now = Work::of(&[ok(), ok(), ok()]);
        assert_eq!(
            appraise(now.since(start, 3, now.last).unwrap()),
            Finding::Null,
            "a step whose whole span is plan revision did nothing"
        );
        // One real call among them and it is no longer null.
        assert_eq!(
            appraise(now.since(start, 2, now.last).unwrap()),
            Finding::Landed
        );
    }

    #[test]
    fn a_bookkeeping_call_landing_last_does_not_mask_a_real_failure() {
        // build fails, then the plan tool revises the list (successfully) —
        // `now.last` is the revision's `Ok`, but the caller (`Tracked`) knows
        // the failure is the outcome that actually belongs to the span.
        let start = Work::default();
        let now = Work::of(&[failed(), ok()]);
        let span = now.since(start, 1, Some(Outcome::Failed)).unwrap();
        assert_eq!(span.calls, 1, "the bookkeeping call is excluded from work");
        assert_eq!(
            appraise(span),
            Finding::EndedOnFailure,
            "the caller's `last` overrides the raw trace tail"
        );
    }

    #[test]
    fn a_bookkeeping_call_landing_last_does_not_manufacture_a_failure() {
        // A rejected plan write is itself a failed call, but it is still the
        // plan tool touching its own state rather than work on the step —
        // the caller excludes it from both the count and `last`.
        let start = Work::default();
        let now = Work::of(&[ok(), failed()]);
        let span = now.since(start, 1, Some(Outcome::Ok)).unwrap();
        assert_eq!(span.calls, 1);
        assert_eq!(
            appraise(span),
            Finding::Landed,
            "a rejected bookkeeping call must not read as the step's own failure"
        );
    }

    #[test]
    fn a_denied_sibling_supports_no_finding_either() {
        // The batched shape `in_flight` exists for, except the sibling is
        // denied rather than still running: settled, but not attributable to
        // this step.
        let span = Work::of(&[ok()])
            .with_denied(1)
            .since(Work::default(), 0, Some(Outcome::Ok))
            .unwrap();
        assert_eq!(appraise(span), Finding::Landed);

        // Same when the visible half would otherwise report a refusal.
        let span = Work::of(&[ok(), denied()])
            .with_denied(1)
            .since(Work::default(), 0, Some(Outcome::Refused))
            .unwrap();
        assert_eq!(
            appraise(span),
            Finding::Landed,
            "the denial in the same batch is not necessarily this step's"
        );
    }

    #[test]
    fn a_failure_recovered_from_is_the_model_working() {
        let start = Work::default();
        let now = Work::of(&[failed(), ok()]);
        let span = now.since(start, 0, now.last).unwrap();
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
            appraise(empty.since(Work::default(), 0, empty.last).unwrap()),
            Finding::Landed
        );

        // Same for a failure a sibling may still be recovering from.
        let failing = Work::of(&[failed()]).with_in_flight(1);
        assert_eq!(
            appraise(failing.since(Work::default(), 0, failing.last).unwrap()),
            Finding::Landed
        );
    }

    #[test]
    fn a_failure_before_the_step_started_is_not_this_step_s() {
        let start = Work::of(&[failed()]);
        let now = Work::of(&[failed(), ok(), ok()]);
        let span = now.since(start, 0, now.last).unwrap();
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
        assert_eq!(second.since(first, 0, second.last), None);

        // And within one run it measures as usual.
        assert!(Work::of(&[ok(), ok(), ok(), ok()])
            .in_run(1)
            .since(first, 0, Some(Outcome::Ok))
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

    // --- the model half: escalation ---

    fn shell(command: &str) -> ToolCallTrace {
        ToolCallTrace {
            name: "shell".into(),
            input: json!({"command": command}),
            is_error: false,
            denied: false,
            unknown: false,
            staged: false,
        }
    }

    fn span(calls: u32, verify_like: u32) -> Span {
        Span {
            calls,
            failed: 0,
            refused: 0,
            verify_like,
            last: Some(Outcome::Ok),
            in_flight: 0,
            denied: 0,
        }
    }

    #[test]
    fn a_shell_call_matching_a_test_runner_looks_like_verification() {
        for command in [
            "cargo test --workspace",
            "pytest tests/",
            "npm test",
            "make test",
            "CARGO TEST -p mecha-core",
        ] {
            assert!(
                looks_like_verification(&shell(command)),
                "{command:?} should have matched"
            );
        }
    }

    #[test]
    fn an_ordinary_shell_call_does_not_look_like_verification() {
        assert!(!looks_like_verification(&shell("cargo build --release")));
        assert!(!looks_like_verification(&call(false, false, false)));
    }

    #[test]
    fn work_folds_verify_like_only_for_successful_calls() {
        // A failed test invocation did not confirm anything.
        let mut failing_test = shell("cargo test");
        failing_test.is_error = true;
        let work = Work::of(&[shell("cargo test"), failing_test]);
        assert_eq!(work.verify_like, 1);
    }

    #[test]
    fn a_span_far_longer_than_its_siblings_is_a_span_outlier_candidate() {
        let completed = vec![
            ("read the config".to_string(), 2),
            ("write the file".to_string(), 3),
        ];
        let escalation = escalation_candidate(span(20, 0), "do the big thing", &completed)
            .expect("20 calls against a mean of 2.5 should escalate");
        assert_eq!(escalation.reason, EscalationReason::SpanOutlier);
        assert_eq!(escalation.calls, 20);
        assert_eq!(escalation.sibling_mean_calls, Some(2.5));
        assert_eq!(escalation.siblings.len(), 2);
    }

    #[test]
    fn a_tiny_plan_never_fires_the_span_outlier_trigger() {
        // Only one prior completed step: nothing to compare against yet.
        let completed = vec![("read the config".to_string(), 2)];
        assert!(escalation_candidate(span(20, 0), "do the big thing", &completed).is_none());
    }

    #[test]
    fn a_step_within_the_floor_never_fires_even_against_a_tiny_mean() {
        // 3x a mean of 1 is 3, which is under the absolute floor.
        let completed = vec![("a".to_string(), 1), ("b".to_string(), 1)];
        assert!(escalation_candidate(span(3, 0), "a small step", &completed).is_none());
    }

    #[test]
    fn a_step_only_moderately_bigger_than_its_siblings_does_not_escalate() {
        let completed = vec![("a".to_string(), 5), ("b".to_string(), 5)];
        // 2x the mean, not 3x.
        assert!(escalation_candidate(span(10, 0), "a somewhat bigger step", &completed).is_none());
    }

    #[test]
    fn a_step_that_claims_verification_with_none_in_its_span_escalates() {
        let escalation = escalation_candidate(span(3, 0), "test that the API responds", &[])
            .expect("a verification claim with no verify-shaped call should escalate");
        assert_eq!(escalation.reason, EscalationReason::UnverifiedClaim);
        assert!(escalation.siblings.is_empty());
        assert_eq!(escalation.sibling_mean_calls, None);
    }

    #[test]
    fn a_step_that_claims_verification_and_has_it_does_not_escalate() {
        assert!(escalation_candidate(span(3, 1), "test that the API responds", &[]).is_none());
    }

    #[test]
    fn an_ordinary_step_with_no_claim_and_no_outlier_never_escalates() {
        let completed = vec![("a".to_string(), 4), ("b".to_string(), 5)];
        assert!(escalation_candidate(span(4, 0), "write the docs", &completed).is_none());
    }

    /// The bug the review found: `"test"` as a plain substring also matches
    /// `"latest"`, so an ordinary step about pulling the latest changes read
    /// as a verification claim with nothing to back it.
    #[test]
    fn a_word_containing_test_as_a_substring_is_not_a_verification_claim() {
        for step in [
            "pull the latest changes",
            "read the latest config",
            "copy the latest bundle",
        ] {
            assert!(
                escalation_candidate(span(3, 0), step, &[]).is_none(),
                "{step:?} must not read as a verification claim"
            );
        }
        // The word-boundary match must still catch the real thing.
        assert!(escalation_candidate(span(3, 0), "test that the API responds", &[]).is_some());
    }

    #[test]
    fn only_the_most_recent_siblings_ride_along() {
        let completed: Vec<(String, u32)> = (0..20).map(|i| (format!("step {i}"), 2)).collect();
        let escalation = escalation_candidate(span(30, 0), "a big step", &completed).unwrap();
        assert_eq!(escalation.siblings.len(), ESCALATION_SIBLING_SAMPLE);
        // Most recent first.
        assert_eq!(escalation.siblings[0], "step 19");
        // The review finding: the mean is over all 20, and the prompt must
        // say so — not the length of the truncated sample listed below it.
        assert_eq!(escalation.sibling_count, 20);
        let prompt = escalation_prompt(&escalation);
        assert!(prompt.contains("(20 of them)"));
        assert!(prompt.contains(&format!(
            "The {ESCALATION_SIBLING_SAMPLE} most recent of them"
        )));
    }

    fn span_outlier_escalation() -> StepEscalation {
        StepEscalation {
            reason: EscalationReason::SpanOutlier,
            step: "do the big thing".into(),
            siblings: vec!["read the config".into()],
            calls: 20,
            sibling_mean_calls: Some(2.5),
            sibling_count: 1,
        }
    }

    #[test]
    fn the_prompt_asks_for_reasoning_before_the_typed_field() {
        let prompt = escalation_prompt(&span_outlier_escalation());
        assert!(prompt.find("\"reasoning\"").unwrap() < prompt.find("\"verdict\"").unwrap());
        assert!(prompt.contains("do the big thing"));
        assert!(prompt.contains("read the config"));
    }

    #[test]
    fn parsing_an_accept_verdict() {
        let v = parse_step_verdict(r#"{"reasoning": "looks fine", "verdict": "accept"}"#).unwrap();
        assert_eq!(v, StepVerdict::Accept);
    }

    #[test]
    fn parsing_a_revise_plan_verdict_wrapped_in_prose() {
        let text = "Here you go:\n```json\n{\"reasoning\": \"too broad\", \"verdict\": \"revise_plan\"}\n```\n";
        assert_eq!(parse_step_verdict(text).unwrap(), StepVerdict::RevisePlan);
    }

    #[test]
    fn an_unrecognised_verdict_is_refused() {
        assert!(parse_step_verdict(r#"{"reasoning": "x", "verdict": "maybe"}"#).is_err());
    }

    #[test]
    fn an_unparseable_reply_is_an_error() {
        assert!(parse_step_verdict("I could not do that.").is_err());
    }

    /// The property the whole design turns on: whatever the model wrote as
    /// `reasoning` must never appear in the nudge shown back to it.
    #[test]
    fn the_nudge_never_contains_the_models_own_reasoning() {
        let escalation = span_outlier_escalation();
        let nudge = templated_nudge(&escalation);
        assert!(nudge.contains("do the big thing"));
        assert!(!nudge.contains("looks fine"));
        assert!(!nudge.contains("too broad"));
        // Fully templated: two calls with the same escalation produce the
        // same nudge regardless of what any model said.
        assert_eq!(nudge, templated_nudge(&escalation));
    }

    #[test]
    fn the_unverified_claim_nudge_names_no_siblings() {
        let escalation = StepEscalation {
            reason: EscalationReason::UnverifiedClaim,
            step: "test that the API responds".into(),
            siblings: Vec::new(),
            calls: 3,
            sibling_mean_calls: None,
            sibling_count: 0,
        };
        let nudge = templated_nudge(&escalation);
        assert!(nudge.contains("test that the API responds"));
    }

    /// The review finding: a nudge not registered in `agent::is_harness_voice`
    /// gets mined by the learning miner as if a person had typed it —
    /// `escalation.step` included — exactly the bug `boredom::NOTICE_STEM`
    /// and `mailbox::DELIVERY_STEM` were each added to fix for their own
    /// voice. Both `EscalationReason` variants must be recognised.
    #[test]
    fn the_nudge_is_recognised_as_the_harness_own_voice() {
        assert!(crate::agent::is_harness_voice(&templated_nudge(
            &span_outlier_escalation()
        )));
        let unverified = StepEscalation {
            reason: EscalationReason::UnverifiedClaim,
            step: "test that the API responds".into(),
            siblings: Vec::new(),
            calls: 3,
            sibling_mean_calls: None,
            sibling_count: 0,
        };
        assert!(crate::agent::is_harness_voice(&templated_nudge(
            &unverified
        )));
    }

    // The retry loop that used to live here moved to `agent.rs`'s
    // `Agent::escalate_step`, which needs `self.complete` for cancellation
    // and usage accounting — see the module note above `parse_step_verdict`.
    // Its tests live beside `agent.rs`'s own `ScriptedProvider`.
}
