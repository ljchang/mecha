//! A task list the agent maintains for itself.
//!
//! Planning as a *tool* rather than a mode. The alternative — a "plan phase"
//! that produces a plan and then hands off — goes stale the moment the first
//! step surprises the model. A list it rewrites as it goes stays honest, and
//! because the current state is echoed back in every tool result, the model
//! re-reads its own plan on the next turn without anyone re-prompting it.
//!
//! It also gives the *user* something to look at during a long run, which is
//! most of why it's worth having.

use super::{CarriedState, Tool, ToolCtx, ToolOutput};
use crate::compact::CARRIED_HEADER;
use crate::goal::GoalRef;

/// The word introducing the goal line in a rendered plan.
///
/// Deliberately the *argument's* name and not better prose. The rendered block
/// is what the model re-reads after a compaction, and it is the only place the
/// plan survives; if the line said `serving` while the argument was `serves`,
/// a post-compaction rewrite would have no way to learn what to call the field
/// it must pass to keep the goal.
const SERVES: &str = "serves";
use crate::message::{Block, Message};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    #[default]
    Pending,
    InProgress,
    Completed,
}

impl Status {
    fn marker(self) -> &'static str {
        match self {
            Status::Pending => "[ ]",
            Status::InProgress => "[~]",
            Status::Completed => "[x]",
        }
    }
}

/// One step, and — optionally — the prediction the planner wrote for it.
///
/// **The plan is the prediction** (`docs/APPRAISAL-RESEARCH.md` §3.7,
/// `docs/AUDIT-RESEARCH.md` §3.11's spec). Three optional fields make a
/// step something that can be *disappointed*: what it should produce, how
/// the harness can tell, and how much work it should take. None is
/// required, because the tool's whole job is being cheap to keep updated;
/// a plan with none is today's plan.
///
/// **Two parsing policies, like `serves:`.** From the model a wrong type is
/// a `ToolOutput` error naming the item (`TodoTool::call`), because the
/// model can fix it next call and a silently dropped prediction leaves a
/// plan claiming less than it said. From a record — a transcript, a carried
/// block — a wrong type reads as absent ([`de_lenient_string`],
/// [`de_lenient_u32`]), because the record is append-only and may have
/// been written by a newer binary, where one unrecognised field must cost
/// the field and never the plan.
///
/// All three are the model's own text, trusted in context the way
/// `content` already is; none is ever rendered into the system prompt.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: Status,
    /// The step's expected outcome, one checkable sentence. Re-read with the
    /// plan, so the model meets its own prediction again after a compaction
    /// or a re-injection; scored by the appraisal when the step completes.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_lenient_string"
    )]
    pub expect: Option<String>,
    /// A command whose exit code says whether the step landed. **Frozen on
    /// completion, for the life of the plan**: while the step is open
    /// [`Tracked`] keeps the hash of the latest `check` it declared, and
    /// from the write that marks it `completed` that declaration stands — a
    /// different check on that write or on any later one, including after
    /// the step is reopened or dropped and re-added, is reported back as a
    /// tamper rather than accepted, the `expect.verify` discipline one tier
    /// down. **Not
    /// executed yet.** The loop is to run it, dispatched exactly as a model
    /// `shell` call would be (approver, sandbox, interlock, hooks), and
    /// record the result as a trace named `step::CHECK_TRACE`, which
    /// `step::Work::of` already folds into `checks_declared` /
    /// `checks_passed`; that execution is the audit lane's
    /// (`AUDIT-RESEARCH.md` §3.11) and until it lands no `check` trace is
    /// ever written.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_lenient_string"
    )]
    pub check: Option<String>,
    /// How many tool calls the model expects the step to take. The
    /// residual against the actual span is the cheapest expectation error
    /// there is. **Nothing reads it yet**: `step::escalation_candidate` is
    /// where the residual belongs, before the sibling mean, and that change
    /// is the audit lane's (`AUDIT-RESEARCH.md` §3.11).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_lenient_u32"
    )]
    pub expect_calls: Option<u32>,
}

impl TodoItem {
    pub fn new(content: impl Into<String>, status: Status) -> Self {
        TodoItem {
            content: content.into(),
            status,
            ..Default::default()
        }
    }

    /// The prediction fields, as the indented lines [`TodoTool::render`]
    /// writes under an item and [`TodoTool::parse_carried`] reads back.
    fn prediction_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(e) = &self.expect {
            out.push(format!("    {EXPECT_KEY} {e}"));
        }
        if let Some(c) = &self.check {
            out.push(format!("    {CHECK_KEY} {c}"));
        }
        if let Some(n) = self.expect_calls {
            out.push(format!("    {EXPECT_CALLS_KEY} {n}"));
        }
        out
    }
}

const EXPECT_KEY: &str = "expect:";
const CHECK_KEY: &str = "check:";
const EXPECT_CALLS_KEY: &str = "expect_calls:";

/// A record's optional string, leniently: anything that is not a string is
/// absent, never an error. See [`TodoItem`]'s two policies.
fn de_lenient_string<'de, D>(d: D) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<Value>::deserialize(d)?;
    // The record door agrees with the model door on one line: a value
    // holding a newline would forge plan lines through `render` →
    // `parse_carried`, and `TodoTool::call` refuses it; a record carrying
    // one (no reachable writer today) loses the field, never the plan.
    Ok(v.and_then(|v| v.as_str().map(str::to_string))
        .filter(|s| !s.trim().is_empty() && !s.contains(['\n', '\r'])))
}

/// A record's optional count, leniently: anything that is not a
/// non-negative integer that fits is absent.
fn de_lenient_u32<'de, D>(d: D) -> std::result::Result<Option<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<Value>::deserialize(d)?;
    Ok(v.and_then(|v| v.as_u64())
        .and_then(|n| u32::try_from(n).ok()))
}

/// One conversation's plan: the list, and what the whole of it serves.
///
/// **The goal belongs to the plan and not to each item**, which is a claim
/// about the world rather than a convenience — and it rests on the
/// conjunction of two facts, one of them very recent.
///
/// `TASK-AGENT-DESIGN.md` **D14** keys this tool by the run's workspace: one
/// workspace, one list. And since `b877e41` (2026-08-26) `tasks work` calls
/// `work::ensure(task_id)`, so each task run gets a workspace of its own —
/// before that every task run used the configured workspace and every task on
/// the board shared one key, which is the bug that commit fixed. Together
/// those give *one list, one task*, so for a delegated run a per-item goal
/// models a state that cannot arise, while costing a field the model must
/// repeat correctly on every item of every write, in the one tool whose whole
/// job is being cheap to keep updated.
///
/// **Not D11.** *One live run per task* is a one-writer rule about two runs
/// racing; it does not say a run serves only one task, and citing it here
/// would be the converse of what it states.
///
/// **The residual, stated so nobody relies on more than is true.** This tool
/// is registered once (`setup.rs`) and serves **chat** runs too, keyed by the
/// same workspace — and a long chat session in one directory legitimately
/// wanders across goals. There, a reference set early and never revised
/// becomes a line above the list that misdescribes it. Accepted: the field is
/// optional, the failure is ordinary staleness the next write corrects, and
/// per-item references would cost every run to fix the one kind that wanders.
/// So "cannot arise" is true of task runs and is not universal.
///
/// Putting it on the plan also makes the rendering fall out: the goal is one
/// line *above* the list rather than a suffix that has to be separated from
/// free-text content on the way back in.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    /// What this plan is a decomposition of. `None` is the ordinary case for
    /// a chat run nobody delegated.
    pub goal: Option<GoalRef>,
    pub items: Vec<TodoItem>,
}

/// One plan per conversation, keyed by the run's workspace.
///
/// It held a single list for the lifetime of the *agent* until 2026-08-26,
/// which was correct while every front-end holding one served a single
/// conversation. `mecha serve` is one shared agent across every session, so
/// two runs shared one list and overwrote each other — and a UI polling the
/// handle rendered the wrong conversation's plan, which is worse than
/// rendering none, because a plausible list belonging to something else is
/// indistinguishable from this one's.
///
/// The key is the run's workspace, on the precedent [`Asker::ask_in`] set for
/// exactly this shape: one agent, many conversations, and the jail as the only
/// thing in scope at call time that says which is which. Two runs sharing a
/// workspace share a list, which is right — that is the same conversation
/// resumed, not two.
///
/// [`Asker::ask_in`]: super::ask::Asker::ask_in
#[derive(Default)]
pub struct TodoTool {
    lists: Mutex<HashMap<PathBuf, Tracked>>,
}

/// One conversation's plan, and what the harness knows about how its steps
/// went.
///
/// The record beside the plan exists because **a step is closed by the model
/// and nothing checked it** (`docs/GOAL-SYSTEM-DESIGN.md` §5.5). Checking
/// needs two boundaries — where a step started and where it was called done —
/// and only this tool sees both: the loop owns the run's trace and stamps the
/// counters, but a *span* is a fact about a plan, which is the one thing the
/// loop must never learn about.
///
/// It is deliberately not a store. Nothing here survives the process, nothing
/// is written down, and a mark that goes missing costs one silent completion —
/// which is the right price, because the alternative is a second source of
/// truth about a plan whose record is already the transcript (D15).
#[derive(Default)]
struct Tracked {
    plan: Plan,
    /// Where each started step's span began, keyed by the item's content.
    ///
    /// Content is the only handle a plan write offers — items carry no id, and
    /// giving them one would cost a field the model must repeat correctly on
    /// every write of the tool whose whole job is being cheap to keep updated.
    /// So a step whose *wording* is rewritten loses its mark and is appraised
    /// as nothing, which is the safe direction: silence, never a finding about
    /// a span that is not the one measured.
    started: HashMap<String, Mark>,
    /// Steps already reported on, so a second identical reading escalates
    /// instead of asking for the same revision again (§5.5's bound).
    flagged: std::collections::HashSet<String>,
    /// How many times this tool has been called for this plan — every call,
    /// including one whose input this tool rejects. A rejected write still
    /// touches nothing but this tool's own state, so it is bookkeeping too;
    /// see [`Tracked::observe`].
    ///
    /// Subtracted from every span: rewriting the list is bookkeeping, and a
    /// model that revises its plan three times mid-step would otherwise show
    /// three calls of "work" for a step where nothing happened.
    own_calls: u32,
    /// The outcome of the most recent call that was *not* this tool touching
    /// its own state, as of the last time [`Tracked::observe`] ran.
    ///
    /// `Work::last` cannot answer this: it is the raw trace's most recent
    /// entry, which is this tool's own call whenever one lands last. Tracked
    /// incrementally because a scalar count of "how many calls were ours"
    /// cannot say *which* position in the sequence they occupied.
    last_real: Option<crate::step::Outcome>,
    /// What `work.calls` will read once *this* call's own trace entry lands —
    /// set at the end of every [`Tracked::observe`]. The next call compares
    /// its own `work.calls` against this to tell whether anything landed in
    /// between besides our own entry.
    next_own_position: Option<u32>,
    /// Steps that landed cleanly, most recent last: `(content, calls)`. The
    /// baseline `step::escalation_candidate`'s span-outlier trigger compares
    /// against, and the siblings its escalation shows the model for context.
    ///
    /// Bounded at [`COMPLETED_HISTORY_CAP`] — a long resumed conversation
    /// revises its plan many times, and this is a rolling sense of "how big
    /// are this plan's steps", not a full history.
    completed: Vec<(String, u32)>,
    /// Each item's declared `check`, hashed, keyed by content like
    /// `started`. A step may revise its check while it is still open; from
    /// the write that marks it `completed` the check is **frozen for the
    /// life of the plan** — not until the step is reopened, which the first
    /// cut allowed and which released the freeze silently: reopen, write a
    /// new check, complete again, and `tampered` stayed at zero for the
    /// exact rewrite it exists to name (found on review). Dropping the step
    /// from the plan and re-adding it is the same door, so this map is
    /// never pruned by the live sweep. A write with a different check on a
    /// frozen step, whatever its status, is a tamper; the frozen hash
    /// stays and the write is echoed back as such.
    ///
    /// **Accepted residual: a reworded step is a new step.** The key is the
    /// item's content, as it is for `started`, `flagged` and `completed`,
    /// because content is the only handle a plan write offers; so
    /// completing `wire it` against `make test` and then writing `wire it.`
    /// completed against `true` freezes the second as a fresh step, with no
    /// tamper echoed (found on review). Closing it needs a key the model
    /// cannot rewrite — a per-item id, which costs the field the plan tool
    /// refuses for the reason its docstring gives, or a fold over the whole
    /// plan's frozen hashes. Named here beside the re-add case so a reader
    /// who sees re-add closed does not assume rename is.
    checks: HashMap<String, Freeze>,
    /// Tampers seen on this plan. Read by [`TodoTool::tampered_in`].
    tampered: u32,
}

/// One step's declared check and whether it may still change.
#[derive(Clone)]
struct Freeze {
    hash: u64,
    frozen: bool,
    /// The declared command itself, so a tampered write can have the frozen
    /// check put back into the plan it tried to rewrite. A hash alone could
    /// only *announce* the change while `self.plan = next` took it — the
    /// echo, the carried block and, once the executor lands, the command
    /// actually run all followed the rewrite (found on review).
    check: String,
}

fn check_hash(check: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    check.trim().hash(&mut h);
    h.finish()
}

/// See [`Tracked::completed`].
const COMPLETED_HISTORY_CAP: usize = 20;

/// Where one step's span starts, in the two units it has to be measured in.
#[derive(Clone, Copy)]
struct Mark {
    work: crate::step::Work,
    own_calls: u32,
}

impl Tracked {
    /// Register one call to this tool, before anything about its input is
    /// known — a call this tool goes on to reject is still this tool
    /// touching its own state and nothing else, so it counts as bookkeeping
    /// exactly like a successful write. Returns `own_calls` as it stood
    /// *before* this call, which is what a mark taken during this same call
    /// must record and what a span completing during it must subtract.
    ///
    /// Also brings `last_real` current. `next_own_position` says what
    /// `work.calls` will read once this call's own entry **and every
    /// approved sibling in its batch** has landed — `work.in_flight` is
    /// exactly that sibling count, since this same `Work` snapshot is
    /// shared by every call in one turn and predates all of them landing.
    /// Missing that term made the guard assume every one of this tool's own
    /// calls was the only call in its batch: the model doing real work and
    /// ticking the box in the same turn — the shape `in_flight` exists
    /// for — advanced `work.calls` by the whole batch size at the next
    /// check, the equality failed, and `last_real` was overwritten with
    /// whatever landed last, which was this tool's own entry whenever
    /// `todo` came last in the batch (the natural order: do the work, then
    /// tick the box).
    ///
    /// The comparison side strips `work.denied`: a call denied in the
    /// *same* turn as this one lands in `trace` ahead of this call (the
    /// gate loop pushes a denial immediately, before dispatching what it
    /// approved), so it is already counted in `work.calls` the instant this
    /// call sees it — not something to predict for later. Counting it as
    /// "something new happened" would let an unrelated sibling's refusal
    /// overwrite `last_real` with `Refused`, which is exactly the
    /// misattribution `span.denied` exists to suppress a different way;
    /// this guard must not re-introduce it through `last_real` instead.
    ///
    /// If the (denial-stripped) reading handed to the *next* call doesn't
    /// match the prediction, something else landed in between (or this is
    /// the first call ever, or the run restarted) and the fresh `work.last`
    /// is real work rather than our own echo. If it does match, nothing but
    /// our own batch happened and `last_real` carries over unchanged.
    ///
    /// **Accepted residual, in the safe direction.** A match means "only
    /// this batch's siblings landed," but a sibling's *outcome* is still
    /// invisible to this tool: `Work` is deliberately a handful of integers
    /// rather than a list, so nothing here can tell whether a failing
    /// sibling landed before or after this tool's own entry within the
    /// batch — that depends on the order the model happened to list the
    /// calls in, which this tool cannot see and must not guess at. When the
    /// sibling lands after, its failure is swallowed the same way an
    /// unbatched one is, one layer removed. This is the false-negative
    /// direction the module doc names as the one to prefer: a masked
    /// failure costs a missed finding, where guessing at an unknowable order
    /// risks the manufactured-failure false positive this fix exists to
    /// close. `in_flight` already suppresses the finding for *this* span
    /// while the batch is still forming; what survives past it is the
    /// span's own last-known reading, not a reconstruction of the batch.
    fn observe(&mut self, work: Option<crate::step::Work>) -> u32 {
        let before = self.own_calls;
        if let Some(work) = work {
            let settled = work.calls.saturating_sub(work.denied);
            if self.next_own_position != Some(settled) {
                self.last_real = work.last;
            }
            self.next_own_position = Some(work.calls + work.in_flight + 1);
        }
        self.own_calls += 1;
        before
    }

    /// Fold one plan write in, and say what the steps that just finished
    /// actually did.
    ///
    /// `work` is the run's counters as of *before* this turn's batch — which
    /// is also before this call itself reaches the trace. `own_calls_before`
    /// and `last_real` are [`Tracked::observe`]'s account of this same call,
    /// taken at the same instant, so their difference against a mark is a
    /// span. Anything unknown produces no line at all: a step never seen in
    /// progress, a run whose counters restarted, a context nobody stamped.
    fn advance(
        &mut self,
        mut next: Plan,
        work: Option<crate::step::Work>,
        own_calls_before: u32,
        last_real: Option<crate::step::Outcome>,
        step_escalation: Option<
            &std::sync::Arc<std::sync::Mutex<Option<crate::step::StepEscalation>>>,
        >,
    ) -> Vec<String> {
        let before: HashMap<&str, Status> = self
            .plan
            .items
            .iter()
            .map(|i| (i.content.as_str(), i.status))
            .collect();

        let mut lines = Vec::new();
        // The check freezes on the write that claims completion, and stays
        // frozen for the life of the plan. While the step is open the latest
        // declaration is the check; on the completing write the last open
        // declaration stands (a different check on that very write is the
        // post-hoc swap the freeze exists for, so the gate reads the item's
        // status *now*, not `was`); and once frozen, a different check on
        // any later write — reopened, re-added, whatever its status — is
        // reported **and put back**: the frozen command is substituted into
        // the plan this write becomes, so the echo, the carried block and
        // the executor all see the check the step was completed against.
        // Announcing the change while taking it was the first cut's shape
        // (found on review). A step completed with a check it never declared
        // while open freezes that first declaration. Runs ahead of the
        // status loop because that loop reads `next` immutably.
        for item in next.items.iter_mut() {
            let Some(check) = item.check.clone() else {
                // The third door: a frozen step written again with the
                // field simply absent. Reopen and re-add-with-a-different-
                // check were closed and this was not (found on review), and
                // once something runs checks it is the cheapest evasion —
                // no trace written, no counter raised, nothing signed. The
                // same claim unmade after the fact is the same tamper:
                // restored, counted, echoed.
                if let Some(f) = self.checks.get(&item.content).filter(|f| f.frozen) {
                    self.tampered += 1;
                    lines.push(format!(
                        "the check for step \"{}\" was dropped after the step was marked \
                         done; the check it was completed against stands, and the change is \
                         recorded",
                        crate::step::ellipsize(&item.content, 60)
                    ));
                    item.check = Some(f.check.clone());
                }
                continue;
            };
            let hash = check_hash(&check);
            let completing = item.status == Status::Completed;
            let prior = self.checks.get(&item.content).cloned();
            match prior {
                Some(f) if (f.frozen || completing) && f.hash != hash => {
                    self.tampered += 1;
                    lines.push(format!(
                        "the check for step \"{}\" was changed on or after the write that \
                         marked it done; the check it was completed against stands, and \
                         the change is recorded",
                        crate::step::ellipsize(&item.content, 60)
                    ));
                    item.check = Some(f.check.clone());
                    // The completing write is what freezes it, even when
                    // that write is the tamper.
                    if completing {
                        self.checks.insert(
                            item.content.clone(),
                            Freeze {
                                hash: f.hash,
                                frozen: true,
                                check: f.check,
                            },
                        );
                    }
                }
                Some(f) if f.frozen => {}
                Some(_) | None => {
                    // Open, or first seen: the latest declaration is the
                    // check, and the completing write is what freezes it.
                    self.checks.insert(
                        item.content.clone(),
                        Freeze {
                            hash,
                            frozen: completing,
                            check,
                        },
                    );
                }
            }
        }
        // `live` is computed here, before the item loop, rather than only
        // after it (its other use, in the sweep below) — a write that both
        // trims finished steps out of the plan *and* lands a new one in the
        // same call needs the pruned baseline for that landing's own
        // comparison, not just for steps *after* this write. Computing it
        // once and reading it twice also means the sweep below can't drift
        // from what the snapshot used.
        let live: std::collections::HashSet<&str> =
            next.items.iter().map(|i| i.content.as_str()).collect();

        // Snapshotted once, before this batch's own completions can reach
        // it: a model that marks two steps `completed` in one write (the
        // tool's own docstring discourages this — "as soon as it is done
        // rather than in a batch at the end" — but does not prevent it)
        // would otherwise have the *second* item's comparison silently
        // contaminated by the *first* item's own call count, pushed onto
        // `self.completed` earlier in this same loop. Every candidate this
        // batch produces is judged against the plan's history as it stood
        // before the batch, never against a sibling landing beside it —
        // and filtered by `live` for the same reason the sweep below prunes
        // it: a step this same write is dropping from the plan is not "the
        // plan's other completed steps" either, even though the retain
        // below has not run yet.
        let completed_before_this_batch: Vec<(String, u32)> = self
            .completed
            .iter()
            .filter(|(k, _)| live.contains(k.as_str()))
            .cloned()
            .collect();

        for item in &next.items {
            let was = before.get(item.content.as_str()).copied();
            match item.status {
                // Started, or restarted after a revision — either way the span
                // begins now. A revised step measured from its *first* start
                // would carry the failed attempt's work into the retry's
                // verdict.
                Status::InProgress if was != Some(Status::InProgress) => {
                    if let Some(work) = work {
                        self.started.insert(
                            item.content.clone(),
                            Mark {
                                work,
                                own_calls: own_calls_before,
                            },
                        );
                    }
                }
                Status::Completed if was != Some(Status::Completed) => {
                    let Some(mark) = self.started.remove(&item.content) else {
                        continue;
                    };
                    let Some(span) = work.and_then(|w| {
                        w.since(
                            mark.work,
                            own_calls_before.saturating_sub(mark.own_calls),
                            last_real,
                        )
                    }) else {
                        continue;
                    };
                    let finding = crate::step::appraise(span);
                    match finding.line(&item.content, self.flagged.contains(&item.content)) {
                        Some(line) => {
                            self.flagged.insert(item.content.clone());
                            lines.push(line);
                        }
                        // It landed, so the next thing to go wrong here is a
                        // first time again. Not while siblings are in flight
                        // or one was denied this turn: both read as landed
                        // because nothing is known yet or nothing here is
                        // attributable, neither of which is the same as
                        // having gone well.
                        None if span.in_flight == 0 && span.denied == 0 => {
                            self.flagged.remove(&item.content);
                            // A genuinely clean landing — not an ambiguous
                            // batch `appraise` defaulted to `Landed` — is a
                            // baseline worth remembering, and a candidate
                            // worth a second opinion (§5.5's escalation).
                            if let Some(slot) = step_escalation {
                                // `completed_before_this_batch` is filtered
                                // by `live`, which this item's own content
                                // is a member of — it is in `next.items`,
                                // being completed right now — so the batch
                                // filter alone does not exclude it. A step
                                // that completed once, was reopened, and is
                                // completing again here would otherwise see
                                // its own pre-revision span as one of "the
                                // plan's other completed steps": the
                                // opposite of round 12's fix, but the same
                                // bug — a step being counted as its own
                                // sibling — reached through the batch
                                // snapshot instead of the live push.
                                let siblings_excluding_self: Vec<(String, u32)> =
                                    completed_before_this_batch
                                        .iter()
                                        .filter(|(k, _)| k != &item.content)
                                        .cloned()
                                        .collect();
                                if let Some(escalation) = crate::step::escalation_candidate(
                                    span,
                                    &item.content,
                                    &siblings_excluding_self,
                                ) {
                                    // First candidate this batch wins. The
                                    // slot is always drained once per turn
                                    // (`agent.rs`'s read-clear-call-fold), so
                                    // it is empty going into this call —
                                    // `is_none` here means "nothing else in
                                    // *this* batch has claimed it yet", not
                                    // "an older, unconsumed candidate is
                                    // stale." Two steps completing in one
                                    // write is rare and the mechanism holds
                                    // exactly one candidate at a time by
                                    // design (`compact_requested`'s own
                                    // shape); silently letting a later item
                                    // overwrite an earlier one would make
                                    // which candidate survives an accident of
                                    // iteration order rather than a choice.
                                    let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
                                    if guard.is_none() {
                                        *guard = Some(escalation);
                                    }
                                }
                            }
                            // Dedupe on content first, same as `started`
                            // (a `HashMap`, so a revision's fresh mark
                            // already replaces rather than doubles up): a
                            // step revised and completed twice would
                            // otherwise contribute two entries under the
                            // same name, inflating `completed.len()` and the
                            // mean it feeds, and letting a step be listed as
                            // its own sibling. Keeps the latest span, and
                            // moves the entry to the end so "most recent
                            // last" still holds for a step that was redone.
                            self.completed.retain(|(k, _)| k != &item.content);
                            self.completed.push((item.content.clone(), span.calls));
                            if self.completed.len() > COMPLETED_HISTORY_CAP {
                                self.completed.remove(0);
                            }
                        }
                        None => {}
                    }
                }
                _ => {}
            }
        }

        // A mark on an item the plan no longer holds describes work nobody is
        // doing, and would otherwise sit in the map for the life of the
        // conversation waiting for a step of the same wording to be re-added.
        // `completed` gets the same sweep: unlike `started`/`flagged`, it is
        // read by `escalation_candidate` as "the plan's other completed
        // steps", and `TodoTool`'s lists are keyed by workspace rather than
        // conversation — so without this, a wholesale plan rewrite (or a
        // second conversation reusing the workspace) leaves stale entries
        // from a plan that no longer exists as the mean a new plan's steps
        // are judged against. Bounding `COMPLETED_HISTORY_CAP` protects
        // against unbounded growth; it does not scope the history to the
        // plan that is live now. `live` itself is the same set the snapshot
        // above filtered by — computed once, at the top of this call.
        self.started.retain(|k, _| live.contains(k.as_str()));
        self.flagged.retain(|k| live.contains(k.as_str()));
        self.completed.retain(|(k, _)| live.contains(k.as_str()));
        drop(live);

        self.plan = next;
        lines
    }
}

impl TodoTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace one run's list wholesale — the resume path.
    ///
    /// Only [`rehydrate`](Self::rehydrate) has any business calling this: a
    /// list set by anything other than the model's own `todo` write, or a
    /// faithful restoration of one, is a second author of state the tool is
    /// supposed to own.
    ///
    /// A fresh record, not a plan swapped into the old one: the spans this
    /// tool measures are counted from a run's trace, and a plan restored from
    /// a transcript was written by a process whose counters are gone. Keeping
    /// the marks would measure the resumed run's work against the killed
    /// one's — the exact wrong-units mistake rung 4 made reading headroom off
    /// one run's outcome for a whole episode.
    pub fn set_plan_in(&self, workspace: &Path, plan: Plan) {
        self.install_in(workspace, plan, HashMap::new());
    }

    /// [`set_plan_in`](Self::set_plan_in), plus freezes the plan itself
    /// no longer shows. The marks are dropped (above) but the freezes are
    /// not: a step the restored plan shows completed against a check is
    /// frozen against that check in the new process too, or a resume was
    /// the fourth door — `Tracked::default()` zeroed `checks`, so the next
    /// write could swap the check as a first declaration with nothing
    /// counted (found on review). `extra` carries the steps the transcript
    /// froze that the last plan trimmed ([`frozen_checks_from_transcript`]
    /// (Self::frozen_checks_from_transcript)); the plan's own completed
    /// steps win where both say something. `tampered` starts at zero, as
    /// the counter is per process and the transcript's own echoes already
    /// carry the earlier lines.
    fn install_in(&self, workspace: &Path, plan: Plan, extra: HashMap<String, String>) {
        let mut checks: HashMap<String, Freeze> = extra
            .into_iter()
            .map(|(content, c)| {
                (
                    content,
                    Freeze {
                        hash: check_hash(&c),
                        frozen: true,
                        check: c,
                    },
                )
            })
            .collect();
        for i in plan.items.iter().filter(|i| i.status == Status::Completed) {
            if let Some(c) = &i.check {
                checks.insert(
                    i.content.clone(),
                    Freeze {
                        hash: check_hash(c),
                        frozen: true,
                        check: c.clone(),
                    },
                );
            }
        }
        self.lists.lock().unwrap_or_else(|e| e.into_inner()).insert(
            workspace.into(),
            Tracked {
                plan,
                checks,
                ..Tracked::default()
            },
        );
    }

    /// Restore a resumed conversation's plan from its own transcript.
    ///
    /// Returns the number of items restored, or `None` when the transcript
    /// held no plan. **D15.** The list lives in memory, which was fine while a
    /// run ended when its conversation did; a task outlives its run by
    /// construction (D13), and on resume the *model* re-reads its plan from
    /// the transcript echo while a UI polling this handle sees nothing. The
    /// model knows where it got to and the card shows no progress — D5's
    /// divergence, arriving from the side the harness controls.
    ///
    /// Deliberately not a stored copy beside the session. The transcript is
    /// already the record, and a second copy is the thing that can disagree
    /// with it — the objection that keeps a mecha-side store of task runs from
    /// existing, and the reason the TUI reads a trigger's last answer from the
    /// session file rather than caching it.
    pub fn rehydrate(&self, workspace: &Path, messages: &[Message]) -> Option<usize> {
        let plan = Self::plan_from_transcript(messages)?;
        let n = plan.items.len();
        // Freezes from every echo the transcript holds, not only the plan
        // it ends on: a completed step trimmed from the plan before the
        // resume — the tool's own docstring encourages trimming finished
        // steps — was absent from the restored plan and so re-added
        // unfrozen, the drop-then-re-add door with a process boundary in
        // the middle (found on review).
        self.install_in(
            workspace,
            plan,
            Self::frozen_checks_from_transcript(messages),
        );
        Some(n)
    }

    /// Every step a transcript's `todo` echoes show completed against a
    /// check, with the check as the *latest* echo showed it — which is the
    /// frozen one, since `advance` renders the echo after putting a
    /// tampered check back. Walked oldest-first so a later echo overrides
    /// an earlier one for the same step text; the last plan's items are
    /// covered by construction, and the trimmed ones are the point.
    pub fn frozen_checks_from_transcript(messages: &[Message]) -> HashMap<String, String> {
        let mut failed: std::collections::HashSet<&str> = Default::default();
        for b in messages.iter().flat_map(|m| m.content.iter()) {
            if let Block::ToolResult {
                tool_use_id,
                is_error: true,
                ..
            } = b
            {
                failed.insert(tool_use_id.as_str());
            }
        }
        let mut todo_ids: std::collections::HashSet<&str> = Default::default();
        for b in messages.iter().flat_map(|m| m.content.iter()) {
            if let Block::ToolUse { id, name, .. } = b {
                if name == "todo" && !failed.contains(id.as_str()) {
                    todo_ids.insert(id.as_str());
                }
            }
        }
        let mut frozen = HashMap::new();
        for b in messages.iter().flat_map(|m| m.content.iter()) {
            if let Block::ToolResult {
                tool_use_id,
                content,
                is_error: false,
            } = b
            {
                if !todo_ids.contains(tool_use_id.as_str()) {
                    continue;
                }
                for item in Self::parse_rendered(content).items {
                    if item.status == Status::Completed {
                        if let Some(c) = item.check {
                            frozen.insert(item.content, c);
                        }
                    }
                }
            }
        }
        frozen
    }

    /// The most recent plan a transcript records, from either of the two
    /// places one can survive.
    ///
    /// Walked newest-first, and the order does the arbitration for free: a
    /// `todo` call made after a compaction is found before the carried block,
    /// which sits in the head message and is therefore reached last.
    ///
    /// Two sources rather than one, because they cover disjoint cases. The
    /// **tool input** is structured and exact, and is what an uncompacted
    /// transcript holds. But a compaction *removes* those blocks — `rebuild`
    /// keeps the rendered list in the carried-state block instead — and a run
    /// long enough to compact is precisely the long-running delegation this
    /// exists for, so reading only the inputs would fail on the motivating
    /// case and succeed on the easy one.
    ///
    /// A write whose result was an error restored nothing at the time and
    /// restores nothing now: the tool rejected it, so the list it names never
    /// existed.
    pub fn from_transcript(messages: &[Message]) -> Option<Vec<TodoItem>> {
        Self::plan_from_transcript(messages).map(|p| p.items)
    }

    /// The same walk, keeping what the plan serves.
    pub fn plan_from_transcript(messages: &[Message]) -> Option<Plan> {
        // Every `todo` result, by id: the error flag, and the echo — the
        // plan as the tool *kept* it, rendered after `advance`. **The echo
        // outranks the input.** The model's raw `ToolUse.input` is what it
        // asked for, and after a tamper that carries the rewritten check
        // while the echo carries the frozen one; restoring the input on a
        // resume froze the rewrite as a first declaration, which is the
        // fifth door and the one that would have been executed (found on
        // review). The input is the fallback for an echo this binary
        // cannot parse — a result truncated by a newer renderer, say.
        let mut failed: std::collections::HashSet<&str> = Default::default();
        let mut echoes: HashMap<&str, &str> = HashMap::new();
        for b in messages.iter().flat_map(|m| m.content.iter()) {
            if let Block::ToolResult {
                tool_use_id,
                is_error,
                content,
                ..
            } = b
            {
                if *is_error {
                    failed.insert(tool_use_id.as_str());
                } else {
                    echoes.insert(tool_use_id.as_str(), content.as_str());
                }
            }
        }

        for msg in messages.iter().rev() {
            for block in msg.content.iter().rev() {
                match block {
                    Block::ToolUse { id, name, input }
                        if name == "todo" && !failed.contains(id.as_str()) =>
                    {
                        if let Some(echo) = echoes.get(id.as_str()) {
                            let plan = Self::parse_rendered(echo);
                            if !plan.items.is_empty() {
                                return Some(plan);
                            }
                        }
                        if let Some(items) = input.get("items") {
                            if let Ok(items) =
                                serde_json::from_value::<Vec<TodoItem>>(items.clone())
                            {
                                // Lenient on the way in: this is a record, and
                                // a kind this binary has not heard of must
                                // cost the reference rather than the plan.
                                let goal = input
                                    .get("serves")
                                    .and_then(Value::as_str)
                                    .and_then(GoalRef::parse_lenient);
                                return Some(Plan { goal, items });
                            }
                        }
                    }
                    Block::Text { text } if text.trim_start().starts_with(CARRIED_HEADER) => {
                        let plan = Self::parse_carried(text);
                        if !plan.items.is_empty() {
                            return Some(plan);
                        }
                    }
                    _ => {}
                }
            }
        }
        None
    }

    /// The `## todo` section of a carried-state block, back into items.
    ///
    /// The inverse of [`render`](Self::render), and a round-trip test says so.
    /// Stops at the next `## ` because the block carries every stateful tool's
    /// section, not only this one.
    fn parse_carried(text: &str) -> Plan {
        let mut lines = text.lines().skip_while(|l| l.trim() != "## todo");
        if lines.next().is_none() {
            return Plan::default();
        }
        let section: Vec<&str> = lines
            .take_while(|l| !l.trim_start().starts_with("## "))
            .collect();
        Self::parse_section(&section)
    }

    /// A `todo` result's echo — [`render`](Self::render)'s output, which is
    /// the carried section without its header, followed by the findings
    /// and the headroom line, neither of which carries a marker. The same
    /// grammar as the carried block, so the two readers cannot drift.
    fn parse_rendered(text: &str) -> Plan {
        let lines: Vec<&str> = text.lines().collect();
        Self::parse_section(&lines)
    }

    fn parse_section(section: &[&str]) -> Plan {
        // Anchored to the first non-empty line, because `render` always writes
        // it there. Scanning the whole section would let an item whose
        // *content* contains a line beginning `serves task:…` supply the
        // plan's goal — free text deciding what the run is for.
        let goal = section
            .iter()
            .find(|l| !l.trim().is_empty())
            .and_then(|l| l.trim().strip_prefix(SERVES))
            .and_then(GoalRef::parse_lenient);
        let mut items: Vec<TodoItem> = Vec::new();
        for line in section {
            let line = line.trim();
            // A prediction line belongs to the item above it. Checked before
            // the marker split so a step whose *content* begins with one of
            // these words is still an item: the marker comes first on an
            // item line and never on a prediction line.
            if let Some(last) = items.last_mut() {
                if let Some(rest) = line.strip_prefix(EXPECT_KEY) {
                    last.expect = Some(rest.trim().to_string()).filter(|s| !s.is_empty());
                    continue;
                }
                if let Some(rest) = line.strip_prefix(CHECK_KEY) {
                    last.check = Some(rest.trim().to_string()).filter(|s| !s.is_empty());
                    continue;
                }
                if let Some(rest) = line.strip_prefix(EXPECT_CALLS_KEY) {
                    last.expect_calls = rest.trim().parse().ok();
                    continue;
                }
            }
            let Some(split) = line.char_indices().nth(3).map(|(i, _)| i) else {
                continue;
            };
            let (marker, rest) = line.split_at(split);
            let status = match marker {
                "[ ]" => Status::Pending,
                "[~]" => Status::InProgress,
                "[x]" => Status::Completed,
                _ => continue,
            };
            let content = rest.trim();
            if !content.is_empty() {
                items.push(TodoItem::new(content, status));
            }
        }
        Plan { goal, items }
    }

    /// One run's list, for a UI that wants to render progress live.
    ///
    /// An absent key is an empty list rather than an error: a conversation
    /// that has not written a plan and one that never will look the same from
    /// here, and both render as no pane.
    pub fn items_in(&self, workspace: &Path) -> Vec<TodoItem> {
        self.lists
            .lock()
            .unwrap()
            .get(workspace)
            .map(|t| t.plan.items.clone())
            .unwrap_or_default()
    }

    /// What this run's plan serves, if it said.
    pub fn goal_in(&self, workspace: &Path) -> Option<GoalRef> {
        self.lists
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(workspace)?
            .plan
            .goal
            .clone()
    }

    fn render(plan: &Plan) -> String {
        if plan.items.is_empty() {
            // Still say what it was for. Carrying it across a compaction needs
            // items — `carried_state` treats an empty section as "the plan is
            // finished" — but the echo is what the model reads *this* turn,
            // and a goal it cannot see is one it cannot re-state.
            return match &plan.goal {
                Some(goal) => format!("{SERVES} {goal}\n(the list is empty)"),
                None => "(the list is empty)".to_string(),
            };
        }
        let done = plan
            .items
            .iter()
            .filter(|i| i.status == Status::Completed)
            .count();
        // Above the list, not beside an item: what the steps are *for* is the
        // half a summariser drops, and it has to be the first thing read back.
        let mut out = String::new();
        if let Some(goal) = &plan.goal {
            out.push_str(&format!("{SERVES} {goal}\n"));
        }
        out.push_str(&format!("{done}/{} done\n", plan.items.len()));
        for item in &plan.items {
            out.push_str(&format!("{} {}\n", item.status.marker(), item.content));
            // Under the item, indented, so the prediction rides the same
            // echo and the same carried block as the step it belongs to —
            // a re-read plan is a re-read prediction.
            for line in item.prediction_lines() {
                out.push_str(&line);
                out.push('\n');
            }
        }
        out
    }

    /// Tampers recorded on this workspace's plan — a completed step's check
    /// rewritten after the fact. Not yet folded into `RunStats` (the loop
    /// would have to ask this tool by name, which it never does); exposed so
    /// a caller that already holds the tool can read it.
    pub fn tampered_in(&self, workspace: &Path) -> u32 {
        self.lists
            .lock()
            .unwrap()
            .get(workspace)
            .map(|t| t.tampered)
            .unwrap_or(0)
    }
}

#[async_trait]
impl Tool for TodoTool {
    fn name(&self) -> &str {
        "todo"
    }

    fn description(&self) -> &str {
        "Record and update your task list for multi-step work. If a task will take more \
         than three tool calls, call this FIRST, before any other tool, and keep the list \
         updated as you work. Pass the COMPLETE list every time — it replaces what was \
         there, so include finished items with status `completed`. Exactly one item should \
         be `in_progress` at a time, and an item should be marked `completed` as soon as \
         it is done rather than in a batch at the end. If the work serves a task on \
         the board, pass `serves` — and pass it on every write, like `items`, \
         because both replace what was there. Skip this tool only for work of \
         one or two steps."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "description": "The complete task list, in order.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": {
                                "type": "string",
                                "description": "One concrete step, phrased as an action."
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed"]
                            },
                            "expect": {
                                "type": "string",
                                "description": "Optional. What will be true when this step is done, \
                                                as one checkable sentence."
                            },
                            "check": {
                                "type": "string",
                                "description": "Optional. A shell command whose exit code shows \
                                                whether the step landed. Fixed once the step is \
                                                completed."
                            },
                            "expect_calls": {
                                "type": "integer",
                                "minimum": 0,
                                "description": "Optional. How many tool calls you expect this \
                                                step to take."
                            }
                        },
                        "required": ["content", "status"]
                    }
                },
                "serves": {
                    "type": "string",
                    "description": "Optional. What this whole plan is working toward, as \
                                    `task:<id>` for a task on the board. Pass it on every \
                                    write, like `items` — it is replaced, not merged."
                }
            },
            "required": ["items"]
        })
    }

    fn read_only(&self) -> bool {
        // Touches nothing outside the agent's own head.
        true
    }

    /// The list survives a compaction verbatim.
    ///
    /// The model re-reads its plan every turn through the echo in the last
    /// `todo` result — which is a *message*, and therefore exactly the kind of
    /// thing a compaction summarises away. That made this tool's whole
    /// mechanism quietly conditional on the transcript never getting long,
    /// which is the one situation the list matters most in: the measured
    /// failure of summarisation is that it keeps what is true and drops how
    /// far you got, and this list is nothing but how far you got.
    ///
    /// Rendered rather than summarised, because the tool holds the exact
    /// current answer and a summariser would only be a lossy path to a worse
    /// copy of it.
    fn carried_state(&self, ctx: &ToolCtx) -> Option<CarriedState> {
        let lists = self.lists.lock().unwrap_or_else(|e| e.into_inner());
        let plan = &lists.get(&ctx.workspace)?.plan;
        // An empty list is genuinely nothing to carry, and an empty section in
        // the prompt reads as "the plan is finished" rather than "there was
        // never a plan".
        if plan.items.is_empty() {
            return None;
        }
        Some(CarriedState {
            label: "todo".into(),
            body: Self::render(plan),
        })
    }

    /// `/clear` and a finished batch item both mean "this conversation is
    /// over", and the plan is conversation state like any other.
    ///
    /// It went unimplemented while the list was agent-wide, when the same
    /// omission merely meant a stale pane. Keyed by workspace it is worse: a
    /// cleared conversation and the next one share a jail, so yesterday's plan
    /// would survive into today's run *and* be spliced into its compaction by
    /// `carried_state` — which is precisely the "plausible list belonging to
    /// something else" the keying was introduced to prevent, arriving through
    /// the one door the keying does not close.
    ///
    /// Clears every workspace rather than one, because the trait method says
    /// nothing about which conversation ended and the registry calls it on a
    /// front-end that has exactly one. That is also what bounds the map: a
    /// long-lived process minting a new session key per conversation
    /// (`serve::session_workspace`) would otherwise accumulate one entry per
    /// session for the life of the process.
    fn forget_conversation_state(&self) {
        self.lists.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        // Registered before validation, and unconditionally: a write this
        // tool goes on to reject below is still this tool touching its own
        // state and nothing else, so it must count as bookkeeping exactly
        // like a successful one — never as work that failed.
        let (own_calls_before, last_real) = {
            let mut lists = self.lists.lock().unwrap_or_else(|e| e.into_inner());
            let tracked = lists.entry(ctx.workspace.clone()).or_default();
            let own_calls_before = tracked.observe(ctx.work);
            (own_calls_before, tracked.last_real)
        };

        let Some(raw) = input.get("items").and_then(Value::as_array) else {
            return Ok(ToolOutput::err(
                "`items` must be an array of {content, status}",
            ));
        };

        let mut items = Vec::with_capacity(raw.len());
        for (i, entry) in raw.iter().enumerate() {
            let Some(content) = entry.get("content").and_then(Value::as_str) else {
                return Ok(ToolOutput::err(format!("item {i} has no `content` string")));
            };
            // Same grammar, same rule as the predictions below: `render`
            // writes an item on one line and `parse_section` reads line by
            // line, so a newline in `content` re-parses its tail as further
            // items — a forged completed step carrying a `check:` the
            // freeze never saw. The hole predates the predictions in the
            // carried block; every resume now reads through this grammar
            // (found on review).
            if content.contains(['\n', '\r']) {
                return Ok(ToolOutput::err(format!(
                    "item {i}: `content` must be a single line"
                )));
            }
            let status = match entry.get("status").and_then(Value::as_str) {
                Some("pending") => Status::Pending,
                Some("in_progress") => Status::InProgress,
                Some("completed") => Status::Completed,
                other => {
                    return Ok(ToolOutput::err(format!(
                        "item {i} has status {other:?}; expected pending, in_progress, or completed"
                    )))
                }
            };
            // Strict on the way in, like `serves`: the model can fix a wrong
            // type on the next call, and a silently dropped prediction
            // leaves a plan claiming less than the model believed it said.
            // Shape as well as type: `render` writes each prediction as one
            // indented line and `parse_carried` reads the section line by
            // line, so a value holding a newline splits across that boundary
            // and its tail re-parses as plan syntax — a `check` of
            // `make test\n[x] deploy` came back from a compaction as a
            // completed step nobody wrote, and an `expect` ending in
            // `\ncheck: …` came back carrying a check the step never
            // declared, which is the field slated for execution (found on
            // review). The model can fix a refused write on the next call.
            let string_field = |key: &str| -> std::result::Result<Option<String>, String> {
                match entry.get(key) {
                    None | Some(Value::Null) => Ok(None),
                    Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
                    Some(Value::String(s)) if s.contains(['\n', '\r']) => {
                        Err(format!("item {i}: `{key}` must be a single line"))
                    }
                    Some(Value::String(s)) => Ok(Some(s.trim().to_string())),
                    Some(_) => Err(format!("item {i}: `{key}` must be a string")),
                }
            };
            let expect = match string_field("expect") {
                Ok(v) => v,
                Err(e) => return Ok(ToolOutput::err(e)),
            };
            let check = match string_field("check") {
                Ok(v) => v,
                Err(e) => return Ok(ToolOutput::err(e)),
            };
            let expect_calls = match entry.get("expect_calls") {
                None | Some(Value::Null) => None,
                Some(v) => match v.as_u64().and_then(|n| u32::try_from(n).ok()) {
                    Some(n) => Some(n),
                    None => {
                        return Ok(ToolOutput::err(format!(
                            "item {i}: `expect_calls` must be a non-negative integer"
                        )))
                    }
                },
            };
            items.push(TodoItem {
                content: content.to_string(),
                status,
                expect,
                check,
                expect_calls,
            });
        }

        // Nudge rather than reject: two items in flight is a mild smell, not an
        // error, and refusing the write would lose the update entirely.
        let in_progress = items
            .iter()
            .filter(|i| i.status == Status::InProgress)
            .count();
        let mut note = String::new();
        if in_progress > 1 {
            note = format!(
                "\n(note: {in_progress} items are in_progress — finish one before starting another)"
            );
        }

        // Strict on the way in, unlike every reader of a record: the model can
        // fix this on the next call, and a silently dropped reference leaves a
        // plan claiming to serve something it does not.
        let goal = match input.get("serves") {
            None | Some(Value::Null) => None,
            Some(value) => {
                // Present but not a string is an error, not an absence. The
                // object spelling — `{"kind": "task", "id": …}` — is the one a
                // model reaches for, and dropping it silently would leave a
                // plan claiming to serve nothing while the model believed it
                // had said so.
                let Some(raw) = value.as_str() else {
                    return Ok(ToolOutput::err(
                        "`serves` must be a string like `task:<id>`",
                    ));
                };
                // An empty string is how a model spells an omitted optional
                // field. Refusing it would throw away an otherwise-valid plan
                // update over a field that was not being used.
                if raw.trim().is_empty() {
                    None
                } else {
                    match raw.parse::<GoalRef>() {
                        Ok(goal) => Some(goal),
                        Err(e) => return Ok(ToolOutput::err(format!("`serves`: {e}"))),
                    }
                }
            }
        };

        let plan = Plan { goal, items };
        // What the steps that just finished actually did, against the run's
        // own record of what it has done. The harness computes the fact; the
        // plan action it argues for — accept, revise the step, revise the
        // plan, escalate — is the model's next call, because the plan is the
        // model's. §5.5.
        //
        // Rendered *after* `advance`, from the plan the tracker kept, not
        // from the input: `advance` may put a frozen check back into a step
        // that tried to rewrite it, and an echo rendered from the input
        // showed the rewrite directly under the line saying the old check
        // stood (found on review).
        let (findings, rendered) = {
            let mut lists = self.lists.lock().unwrap();
            let tracked = lists.entry(ctx.workspace.clone()).or_default();
            let findings = tracked.advance(
                plan,
                ctx.work,
                own_calls_before,
                last_real,
                ctx.step_escalation.as_ref(),
            );
            (findings, Self::render(&tracked.plan))
        };
        let findings = match findings.is_empty() {
            true => String::new(),
            false => format!("\n\n{}", findings.join("\n")),
        };
        // The headroom reading, on the one result where it changes a
        // decision. Not the turn tail and not the system prompt: the tail
        // would leave one stale reading per turn in an append-only transcript
        // — the distractor shape `evict_superseded_results` exists to remove —
        // and the system prompt sits inside the cached prefix, so a per-turn
        // value there re-pays the whole thing including the tool specs. Here
        // it costs nothing on any turn that does not touch the plan, and the
        // accumulation is bounded by plan revisions rather than by turns.
        //
        // Bounded, not absent: an earlier `todo` result is *not* generally
        // superseded by this one. `compact::target_of` falls through to
        // `{name}\0{input}` for a call with no `path`, so two `todo` calls are
        // the same target only when their item lists are byte-identical — and a
        // second `todo` call exists precisely to change the list. So a run that
        // revises its plan ten times carries ten readings until a compaction
        // thins them. That is the same distractor shape as the turn tail, at a
        // far lower rate, which is the trade being made and not a case of
        // avoiding it.
        //
        // Absent when the run has no compaction threshold or has not sent a
        // request yet — a missing line is right where there is no measurement,
        // and inventing one would put a guess in the one place every other
        // number is measured.
        let context = match &ctx.context {
            Some(f) => format!("\n\n{f}"),
            None => String::new(),
        };
        Ok(ToolOutput::ok(format!(
            "{rendered}{note}{findings}{context}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writing_the_list_echoes_it_back_with_progress() {
        let tool = TodoTool::new();
        let ctx = ToolCtx::default();
        let out = tool
            .call(
                json!({"items": [
                    {"content": "read the config", "status": "completed"},
                    {"content": "fix the port", "status": "in_progress"},
                    {"content": "run the tests", "status": "pending"}
                ]}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!out.is_error);
        assert!(out.content.starts_with("1/3 done"));
        assert!(out.content.contains("[x] read the config"));
        assert!(out.content.contains("[~] fix the port"));
        assert!(out.content.contains("[ ] run the tests"));
        assert_eq!(tool.items_in(&ctx.workspace).len(), 3);
    }

    #[tokio::test]
    async fn the_list_is_replaced_not_appended() {
        let tool = TodoTool::new();
        let ctx = ToolCtx::default();
        tool.call(
            json!({"items": [{"content": "a", "status": "pending"}]}),
            &ctx,
        )
        .await
        .unwrap();
        tool.call(
            json!({"items": [{"content": "b", "status": "pending"}]}),
            &ctx,
        )
        .await
        .unwrap();

        let items = tool.items_in(&ctx.workspace);
        assert_eq!(items.len(), 1, "a write replaces the whole list");
        assert_eq!(items[0].content, "b");
    }

    #[tokio::test]
    async fn a_plan_can_name_what_it_serves_and_echoes_it_above_the_list() {
        let tool = TodoTool::new();
        let ctx = ToolCtx::default();
        let out = tool
            .call(
                json!({
                    "items": [{"content": "draft the reply", "status": "in_progress"}],
                    "serves": "task:01J8ZK",
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!out.is_error);
        // Above the list, because the echo is what the model re-reads every
        // turn and what survives a compaction.
        assert!(
            out.content.starts_with("serves task:01J8ZK\n"),
            "{}",
            out.content
        );
        assert_eq!(
            tool.goal_in(&ctx.workspace),
            Some(GoalRef::Task("01J8ZK".into()))
        );
    }

    /// The model-facing direction is strict, the opposite of every reader of a
    /// record. A dropped reference would leave a plan claiming to serve
    /// something it does not, and the model can fix this on the next call.
    #[tokio::test]
    async fn a_malformed_goal_is_reported_rather_than_silently_dropped() {
        let tool = TodoTool::new();
        let ctx = ToolCtx::default();
        let out = tool
            .call(
                json!({
                    "items": [{"content": "a", "status": "pending"}],
                    "serves": "epic:7",
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(
            out.content.contains("not a kind of goal"),
            "{}",
            out.content
        );
        assert!(
            tool.items_in(&ctx.workspace).is_empty(),
            "a rejected write changes nothing"
        );
    }

    #[tokio::test]
    async fn a_plan_that_serves_nothing_renders_no_goal_line() {
        let tool = TodoTool::new();
        let ctx = ToolCtx::default();
        let out = tool
            .call(
                json!({"items": [{"content": "a", "status": "pending"}]}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!out.is_error);
        assert!(out.content.starts_with("0/1 done"), "{}", out.content);
        assert_eq!(tool.goal_in(&ctx.workspace), None);
    }

    /// Present but not a string is an error, not an absence. The object
    /// spelling is the one a model reaches for, and dropping it silently would
    /// leave a plan serving nothing while the model believed it had said so.
    #[tokio::test]
    async fn a_non_string_goal_is_reported_rather_than_silently_dropped() {
        let tool = TodoTool::new();
        let ctx = ToolCtx::default();
        let out = tool
            .call(
                json!({
                    "items": [{"content": "a", "status": "pending"}],
                    "serves": {"kind": "task", "id": "01J8ZK"},
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.is_error, "{}", out.content);
        assert!(out.content.contains("must be a string"), "{}", out.content);
        assert!(tool.items_in(&ctx.workspace).is_empty());
    }

    /// An empty string is how a model spells an unused optional field.
    /// Refusing it would discard an otherwise-valid plan update over a field
    /// that was not being used.
    #[tokio::test]
    async fn an_empty_goal_means_omitted_and_does_not_cost_the_write() {
        let tool = TodoTool::new();
        let ctx = ToolCtx::default();
        let out = tool
            .call(
                json!({
                    "items": [{"content": "a", "status": "pending"}],
                    "serves": "",
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(tool.items_in(&ctx.workspace).len(), 1, "the plan was kept");
        assert_eq!(tool.goal_in(&ctx.workspace), None);
    }

    /// The echo is what the model reads this turn. A goal it cannot see is one
    /// it cannot re-state on the next write.
    #[tokio::test]
    async fn an_empty_list_still_says_what_it_was_for() {
        let tool = TodoTool::new();
        let ctx = ToolCtx::default();
        let out = tool
            .call(json!({"items": [], "serves": "task:01J8ZK"}), &ctx)
            .await
            .unwrap();
        assert!(!out.is_error);
        assert!(
            out.content.contains("serves task:01J8ZK"),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn a_bad_status_is_reported_rather_than_silently_dropped() {
        let tool = TodoTool::new();
        let ctx = ToolCtx::default();
        let out = tool
            .call(json!({"items": [{"content": "a", "status": "done"}]}), &ctx)
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(out.content.contains("expected pending"));
        assert!(
            tool.items_in(&ctx.workspace).is_empty(),
            "a rejected write changes nothing"
        );
    }

    fn ctx_in(dir: &str) -> ToolCtx {
        ToolCtx {
            workspace: PathBuf::from(dir),
            ..Default::default()
        }
    }

    /// The D14 property, and the reason this tool stopped holding one list.
    ///
    /// Fails on the old behaviour: a single `Mutex<Vec<TodoItem>>` returns
    /// b's plan for a's workspace, which is precisely the "plausible list
    /// belonging to something else" a UI cannot detect.
    #[tokio::test]
    async fn two_workspaces_keep_separate_lists() {
        let tool = TodoTool::new();
        let (a, b) = (ctx_in("/w/a"), ctx_in("/w/b"));

        tool.call(
            json!({"items": [{"content": "a", "status": "pending"}]}),
            &a,
        )
        .await
        .unwrap();
        tool.call(
            json!({"items": [{"content": "b", "status": "pending"}]}),
            &b,
        )
        .await
        .unwrap();

        let (ia, ib) = (tool.items_in(&a.workspace), tool.items_in(&b.workspace));
        assert_eq!(ia.len(), 1);
        assert_eq!(ib.len(), 1);
        assert_eq!(ia[0].content, "a", "b's write must not reach a's list");
        assert_eq!(ib[0].content, "b");
    }

    use crate::message::Role;

    fn todo_call(id: &str, items: &[(&str, &str)]) -> Message {
        let items: Vec<Value> = items
            .iter()
            .map(|(c, s)| json!({"content": c, "status": s}))
            .collect();
        Message {
            role: Role::Assistant,
            content: vec![Block::ToolUse {
                id: id.into(),
                name: "todo".into(),
                input: json!({ "items": items }),
            }],
        }
    }

    fn result(id: &str, is_error: bool) -> Message {
        Message {
            role: Role::User,
            content: vec![Block::ToolResult {
                tool_use_id: id.into(),
                content: "ok".into(),
                is_error,
            }],
        }
    }

    /// The ordinary resume: an uncompacted transcript still holds the
    /// structured input of the last write.
    #[tokio::test]
    async fn a_resumed_transcript_restores_the_last_plan() {
        let tool = TodoTool::new();
        let ws = PathBuf::from("/w/a");
        let msgs = vec![
            todo_call("t1", &[("first", "completed")]),
            result("t1", false),
            todo_call("t2", &[("first", "completed"), ("second", "in_progress")]),
            result("t2", false),
        ];

        assert!(tool.items_in(&ws).is_empty(), "nothing before the resume");
        assert_eq!(tool.rehydrate(&ws, &msgs), Some(2));

        let items = tool.items_in(&ws);
        assert_eq!(items[0].content, "first");
        assert_eq!(items[1].status, Status::InProgress);
    }

    /// A write the tool rejected never changed the list, so restoring it would
    /// invent a plan the conversation never had.
    #[tokio::test]
    async fn a_rejected_write_is_not_restored() {
        let tool = TodoTool::new();
        let ws = PathBuf::from("/w/a");
        let msgs = vec![
            todo_call("t1", &[("real plan", "in_progress")]),
            result("t1", false),
            todo_call("t2", &[("rejected plan", "in_progress")]),
            result("t2", true),
        ];

        tool.rehydrate(&ws, &msgs).unwrap();
        let items = tool.items_in(&ws);
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].content, "real plan",
            "the rejected write is skipped"
        );
    }

    /// The motivating case: a compaction removes the `todo` calls and keeps
    /// the rendered list in the carried block, so reading only tool inputs
    /// would fail on exactly the long-running delegation this is for.
    #[tokio::test]
    async fn a_compacted_transcript_restores_from_the_carried_block() {
        let tool = TodoTool::new();
        let ws = PathBuf::from("/w/a");
        // The real shape `compact::rebuild` produces: the original task, the
        // summary, and the carried state as three blocks on one head message —
        // not one block, which is what this test asserted until it failed and
        // sent me back to read `rebuild`.
        let head = Message {
            role: Role::User,
            content: vec![
                Block::text("the original task"),
                Block::text("\n\n[Earlier turns were compacted to fit the context window.]"),
                Block::text(format!(
                    "\n\n{CARRIED_HEADER}\n\n## todo\n1/2 done\n                     [x] read the thread\n[~] draft the reply\n"
                )),
            ],
        };

        assert_eq!(tool.rehydrate(&ws, &[head]), Some(2));
        let items = tool.items_in(&ws);
        assert_eq!(items[0].content, "read the thread");
        assert_eq!(items[0].status, Status::Completed);
        assert_eq!(items[1].content, "draft the reply");
        assert_eq!(items[1].status, Status::InProgress);
    }

    /// Newest wins, and the walk order gives it for free: a write made after
    /// the compaction supersedes the block in the head message.
    #[tokio::test]
    async fn a_write_after_the_compaction_beats_the_carried_block() {
        let tool = TodoTool::new();
        let ws = PathBuf::from("/w/a");
        let msgs = vec![
            Message {
                role: Role::User,
                content: vec![Block::text(format!(
                    "{CARRIED_HEADER}\n\n## todo\n0/1 done\n[ ] stale\n"
                ))],
            },
            todo_call("t9", &[("current", "in_progress")]),
            result("t9", false),
        ];

        tool.rehydrate(&ws, &msgs).unwrap();
        assert_eq!(tool.items_in(&ws)[0].content, "current");
    }

    /// `parse_carried` is the inverse of `render`, and drift between them
    /// would restore a plan that silently lost its statuses.
    #[test]
    fn rendering_and_parsing_round_trip() {
        let items = vec![
            TodoItem::new("read the config", Status::Completed),
            TodoItem::new("fix the port", Status::InProgress),
            TodoItem::new("run the tests", Status::Pending),
        ];
        // With a goal, because *what the steps are for* is exactly the half a
        // summariser drops — carrying the list across a compaction and losing
        // what it serves would reproduce, one field down, the failure
        // `carried_state` exists to prevent.
        let plan = Plan {
            goal: Some(GoalRef::Task("01J8ZK".into())),
            items: items.clone(),
        };
        let block = format!("{CARRIED_HEADER}\n\n## todo\n{}\n", TodoTool::render(&plan));
        assert!(
            block.contains("serves task:01J8ZK"),
            "the goal is rendered above the list: {block}"
        );

        let back = TodoTool::parse_carried(&block);
        assert_eq!(back, plan);

        // And a plan that serves nothing round-trips as one, rather than
        // acquiring a reference on the way back.
        let bare = Plan { goal: None, items };
        let block = format!("{CARRIED_HEADER}\n\n## todo\n{}\n", TodoTool::render(&bare));
        assert_eq!(TodoTool::parse_carried(&block), bare);
    }

    /// Free text must not be able to say what the run is for. `render` always
    /// writes the goal on the section's first line, so the parser anchors
    /// there — an unanchored scan let an item whose *content* held a line
    /// beginning `serves task:…` supply the plan's goal.
    #[test]
    fn an_item_whose_content_looks_like_a_goal_line_does_not_become_one() {
        let block =
            format!("{CARRIED_HEADER}\n\n## todo\n0/1 done\n[ ] paste this:\nserves task:99\n");
        assert_eq!(TodoTool::parse_carried(&block).goal, None);
    }

    /// A record written by a newer binary naming a kind this one has never
    /// heard of costs the reference and nothing else. The opposite policy from
    /// the model-facing direction, which errors — see `goal`.
    #[test]
    fn a_carried_goal_of_an_unknown_kind_does_not_cost_the_plan() {
        let block = format!("{CARRIED_HEADER}\n\n## todo\nserves epic:7\n1/1 done\n[x] mine\n");
        let back = TodoTool::parse_carried(&block);
        assert_eq!(back.goal, None);
        assert_eq!(back.items.len(), 1, "the plan survives its unreadable goal");
    }

    /// A block carries every stateful tool's section, so the walk must stop
    /// at the next heading rather than swallowing a neighbour's lines.
    #[test]
    fn a_neighbouring_carried_section_is_not_absorbed() {
        let block =
            format!("{CARRIED_HEADER}\n\n## todo\n1/1 done\n[x] mine\n\n## skill\n[x] not mine\n");
        let plan = TodoTool::parse_carried(&block);
        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].content, "mine");
    }

    /// A transcript with no plan restores nothing, rather than an empty list
    /// that would render as "the plan is finished".
    #[test]
    fn a_transcript_with_no_plan_restores_nothing() {
        assert!(TodoTool::from_transcript(&[Message::user("hello")]).is_none());
        assert!(TodoTool::from_transcript(&[]).is_none());
    }

    /// `/clear` ends a conversation, and the plan is conversation state. With
    /// the list keyed by workspace and a cleared conversation keeping the same
    /// jail, a surviving list would be spliced into the *next* conversation's
    /// compaction by `carried_state` — the exact failure the keying was for,
    /// through the one door keying does not close.
    #[tokio::test]
    async fn clearing_a_conversation_drops_its_plan() {
        let tool = TodoTool::new();
        let ctx = ToolCtx::default();
        tool.call(
            json!({"items": [{"content": "old business", "status": "in_progress"}]}),
            &ctx,
        )
        .await
        .unwrap();
        assert_eq!(tool.items_in(&ctx.workspace).len(), 1);

        tool.forget_conversation_state();
        assert!(tool.items_in(&ctx.workspace).is_empty(), "the plan is gone");
        assert!(
            tool.carried_state(&ctx).is_none(),
            "and cannot reach the next conversation's compaction"
        );
    }

    /// A compaction carries the *compacting run's* plan, not whichever list
    /// was written most recently by anyone.
    #[tokio::test]
    async fn carried_state_belongs_to_the_run_being_compacted() {
        let tool = TodoTool::new();
        let (a, b) = (ctx_in("/w/a"), ctx_in("/w/b"));

        tool.call(
            json!({"items": [{"content": "ship a", "status": "in_progress"}]}),
            &a,
        )
        .await
        .unwrap();
        tool.call(
            json!({"items": [{"content": "ship b", "status": "in_progress"}]}),
            &b,
        )
        .await
        .unwrap();

        let carried = tool.carried_state(&a).expect("a has a list to carry");
        assert!(carried.body.contains("ship a"));
        assert!(
            !carried.body.contains("ship b"),
            "a compaction must not carry another conversation's plan"
        );

        // A run that never wrote a list carries nothing, rather than
        // inheriting a neighbour's.
        assert!(tool.carried_state(&ctx_in("/w/c")).is_none());
    }

    #[tokio::test]
    async fn multiple_in_progress_items_get_a_nudge() {
        let tool = TodoTool::new();
        let out = tool
            .call(
                json!({"items": [
                    {"content": "a", "status": "in_progress"},
                    {"content": "b", "status": "in_progress"}
                ]}),
                &ToolCtx::default(),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "the write still lands");
        assert!(out.content.contains("finish one before starting another"));
    }

    // --- step appraisal (`docs/GOAL-SYSTEM-DESIGN.md` §5.5) ---
    //
    // The pure arithmetic is tested in `step.rs`; what these cover is the
    // wiring, which is where the false positives live — a reading that fires
    // on ordinary work is a line the model learns to skip.

    use crate::step::{Outcome, Work};

    /// The counters as the loop would stamp them: `calls` is everything in the
    /// run's trace, this tool's own writes included.
    fn work_ctx(run: u64, calls: u32, last: Option<Outcome>) -> ToolCtx {
        ToolCtx {
            work: Some(
                Work {
                    calls,
                    last,
                    ..Work::default()
                }
                .in_run(run),
            ),
            ..ToolCtx::default()
        }
    }

    /// Same, with `in_flight` siblings — the batched shape, where this same
    /// `Work` snapshot is handed to every approved call in the turn before
    /// any of them (this one included) has landed.
    fn batched_work_ctx(run: u64, calls: u32, last: Option<Outcome>, in_flight: u32) -> ToolCtx {
        ToolCtx {
            work: Some(
                Work {
                    calls,
                    last,
                    in_flight,
                    ..Work::default()
                }
                .in_run(run),
            ),
            ..ToolCtx::default()
        }
    }

    async fn write(tool: &TodoTool, ctx: &ToolCtx, items: Value) -> String {
        tool.call(json!({ "items": items }), ctx)
            .await
            .unwrap()
            .content
    }

    #[tokio::test]
    async fn a_step_with_work_behind_it_is_appraised_silently() {
        let tool = TodoTool::new();
        write(
            &tool,
            &work_ctx(1, 0, None),
            json!([{"content": "fix the port", "status": "in_progress"}]),
        )
        .await;
        // Two calls of real work, plus the write above, now in the trace.
        let out = write(
            &tool,
            &work_ctx(1, 3, Some(Outcome::Ok)),
            json!([{"content": "fix the port", "status": "completed"}]),
        )
        .await;
        assert!(
            !out.contains("fix the port\""),
            "the common path says nothing: {out}"
        );
    }

    #[tokio::test]
    async fn a_step_marked_done_with_nothing_behind_it_says_so() {
        let tool = TodoTool::new();
        write(
            &tool,
            &work_ctx(1, 0, None),
            json!([{"content": "fix the port", "status": "in_progress"}]),
        )
        .await;
        // The only call since is the write above.
        let out = write(
            &tool,
            &work_ctx(1, 1, Some(Outcome::Ok)),
            json!([{"content": "fix the port", "status": "completed"}]),
        )
        .await;
        assert!(out.contains("no tool calls behind it"), "{out}");
        // The list itself is still the first thing the model reads.
        assert!(out.starts_with("1/1 done"));
    }

    /// The null step masked by the bookkeeping that announced it: three plan
    /// writes are three trace entries, and counting them as work is how a step
    /// where nothing happened reads as busy.
    #[tokio::test]
    async fn revising_the_plan_is_not_work() {
        let tool = TodoTool::new();
        let started = json!([{"content": "fix the port", "status": "in_progress"}]);
        write(&tool, &work_ctx(1, 0, None), started.clone()).await;
        write(&tool, &work_ctx(1, 1, Some(Outcome::Ok)), started).await;
        let out = write(
            &tool,
            &work_ctx(1, 2, Some(Outcome::Ok)),
            json!([{"content": "fix the port", "status": "completed"}]),
        )
        .await;
        assert!(out.contains("no tool calls behind it"), "{out}");
    }

    #[tokio::test]
    async fn a_step_never_seen_in_progress_is_not_appraised() {
        let tool = TodoTool::new();
        // Straight to done in one write: there is no span, and inventing a
        // start would measure the whole run against one item.
        let out = write(
            &tool,
            &work_ctx(1, 4, Some(Outcome::Ok)),
            json!([{"content": "fix the port", "status": "completed"}]),
        )
        .await;
        assert!(!out.contains("no tool calls"), "{out}");
    }

    #[tokio::test]
    async fn an_unstamped_context_makes_no_claim() {
        let tool = TodoTool::new();
        let bare = ToolCtx::default();
        write(
            &tool,
            &bare,
            json!([{"content": "fix the port", "status": "in_progress"}]),
        )
        .await;
        let out = write(
            &tool,
            &bare,
            json!([{"content": "fix the port", "status": "completed"}]),
        )
        .await;
        assert!(
            !out.contains("no tool calls"),
            "nobody measured, so nothing is claimed: {out}"
        );
    }

    /// The chat shape. A step started before the user last spoke has a mark in
    /// the previous run's units, and differencing across that would announce
    /// the null step on ordinary work.
    #[tokio::test]
    async fn a_step_spanning_two_runs_is_unmeasurable_rather_than_empty() {
        let tool = TodoTool::new();
        write(
            &tool,
            &work_ctx(1, 6, Some(Outcome::Ok)),
            json!([{"content": "fix the port", "status": "in_progress"}]),
        )
        .await;
        let out = write(
            &tool,
            &work_ctx(2, 1, Some(Outcome::Ok)),
            json!([{"content": "fix the port", "status": "completed"}]),
        )
        .await;
        assert!(!out.contains("no tool calls"), "{out}");
    }

    #[tokio::test]
    async fn a_second_bad_reading_on_one_step_stops_asking_for_a_revision() {
        let tool = TodoTool::new();
        let started = json!([{"content": "fix the port", "status": "in_progress"}]);
        let done = json!([{"content": "fix the port", "status": "completed"}]);

        write(&tool, &work_ctx(1, 0, None), started.clone()).await;
        let first = write(&tool, &work_ctx(1, 1, Some(Outcome::Ok)), done.clone()).await;
        assert!(first.contains("no tool calls behind it") && !first.contains("second time"));

        // Put back and ticked again with nothing behind it either time.
        write(&tool, &work_ctx(1, 2, Some(Outcome::Ok)), started).await;
        let second = write(&tool, &work_ctx(1, 3, Some(Outcome::Ok)), done).await;
        assert!(second.contains("second time"), "{second}");
    }

    #[tokio::test]
    async fn a_refused_step_is_reported_as_blocked_and_not_as_broken() {
        let tool = TodoTool::new();
        write(
            &tool,
            &work_ctx(1, 0, None),
            json!([{"content": "publish the site", "status": "in_progress"}]),
        )
        .await;
        let out = write(
            &tool,
            &work_ctx(1, 3, Some(Outcome::Refused)),
            json!([{"content": "publish the site", "status": "completed"}]),
        )
        .await;
        assert!(out.contains("refused"), "{out}");
        assert!(
            !out.contains("still failing"),
            "the approver doing its job is not the step going wrong: {out}"
        );
    }

    /// A successful revision landing last must not mask an earlier failure:
    /// start, a real call fails, the plan is revised (this tool's own write,
    /// `Ok`), then completed. The raw trace's tail is the revision, not the
    /// failure — only `Tracked::observe`'s own account gets it right.
    #[tokio::test]
    async fn a_bookkeeping_revision_does_not_mask_an_earlier_failure() {
        let tool = TodoTool::new();
        let step = json!([{"content": "ship the release", "status": "in_progress"}]);
        write(&tool, &work_ctx(1, 0, None), step.clone()).await;
        // The real call fails (calls: start's own entry, plus this one).
        // The plan tool revises next — a no-op rewrite of the same status —
        // and its own write lands as calls=3, `Ok`.
        write(&tool, &work_ctx(1, 2, Some(Outcome::Failed)), step).await;
        let out = write(
            &tool,
            &work_ctx(1, 3, Some(Outcome::Ok)),
            json!([{"content": "ship the release", "status": "completed"}]),
        )
        .await;
        assert!(
            out.contains("still failing"),
            "the revision's own `Ok` must not bury the real failure: {out}"
        );
    }

    /// A rejected plan write is still this tool touching its own state, not
    /// work on the step — it must not read as the step's own failure just
    /// because it is the most recent trace entry when the step completes.
    #[tokio::test]
    async fn a_rejected_write_does_not_manufacture_a_step_failure() {
        let tool = TodoTool::new();
        let step = json!([{"content": "ship the release", "status": "in_progress"}]);
        write(&tool, &work_ctx(1, 0, None), step).await;
        // A real, non-todo call succeeds in between (start's own entry, plus
        // this one — no write of ours for it, so `calls` jumps to 2 without
        // another call through this tool).
        //
        // A malformed write this tool rejects comes next — it still lands in
        // the trace as a failed call, becoming calls=3 once it returns.
        tool.call(
            json!({"items": [{"content": "ship the release", "status": "not_a_status"}]}),
            &work_ctx(1, 2, Some(Outcome::Ok)),
        )
        .await
        .unwrap();
        let out = write(
            &tool,
            &work_ctx(1, 3, Some(Outcome::Failed)),
            json!([{"content": "ship the release", "status": "completed"}]),
        )
        .await;
        assert!(
            !out.contains("still failing"),
            "a rejected bookkeeping write is not the step's own failure: {out}"
        );
        assert!(
            !out.contains("no tool calls behind it"),
            "the real call succeeded, so the span is not empty either: {out}"
        );
    }

    /// The same manufactured-failure shape, with the rejected write batched
    /// beside a sibling instead of alone — the shape `in_flight` exists for,
    /// and the one `own_calls`'s scalar count cannot tell apart from an
    /// unrelated turn unless `next_own_position` accounts for the whole
    /// batch landing, not just this tool's own entry.
    #[tokio::test]
    async fn a_rejected_write_batched_with_a_sibling_does_not_manufacture_a_failure() {
        let tool = TodoTool::new();
        let step = json!([{"content": "ship the release", "status": "in_progress"}]);
        write(&tool, &work_ctx(1, 0, None), step).await;
        // A batch of two: a real call that succeeds, and a malformed write
        // this tool rejects. `in_flight = 1` (two approved calls this turn).
        tool.call(
            json!({"items": [{"content": "ship the release", "status": "not_a_status"}]}),
            &batched_work_ctx(1, 1, Some(Outcome::Ok), 1),
        )
        .await
        .unwrap();
        // Both landed: the start (1), the real call (1), the rejected write
        // (1) — calls = 3.
        let out = write(
            &tool,
            &work_ctx(1, 3, Some(Outcome::Failed)),
            json!([{"content": "ship the release", "status": "completed"}]),
        )
        .await;
        assert!(
            !out.contains("still failing"),
            "a rejected bookkeeping write batched with a sibling is not the \
             step's own failure: {out}"
        );
    }

    // --- the escalation slot (§5.5's model half) ---

    fn escalation_ctx(
        run: u64,
        calls: u32,
        last: Option<Outcome>,
        verify_like: u32,
    ) -> (
        ToolCtx,
        std::sync::Arc<std::sync::Mutex<Option<crate::step::StepEscalation>>>,
    ) {
        let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
        let ctx = ToolCtx {
            work: Some(
                Work {
                    calls,
                    last,
                    verify_like,
                    // Every test using this helper models a span made of
                    // `shell` calls — the ordinary case the UnverifiedClaim
                    // trigger is for — so `shell_calls` tracks `calls` here,
                    // same convention as `step.rs`'s own `span()` test
                    // helper.
                    shell_calls: calls,
                    ..Work::default()
                }
                .in_run(run),
            ),
            step_escalation: Some(slot.clone()),
            ..ToolCtx::default()
        };
        (ctx, slot)
    }

    /// A run whose escalation slot is `None` — the feature off — behaves
    /// exactly as every test above it already proves: no write, no panic.
    /// This pins the same property directly against a span large enough that
    /// it would be a candidate if the slot existed.
    #[tokio::test]
    async fn a_span_outlier_with_no_escalation_slot_writes_nothing_and_does_not_panic() {
        let tool = TodoTool::new();
        for i in 0..2 {
            let step = format!("small step {i}");
            write(
                &tool,
                &work_ctx(1, i * 3, None),
                json!([{"content": step, "status": "in_progress"}]),
            )
            .await;
            write(
                &tool,
                &work_ctx(1, i * 3 + 2, Some(Outcome::Ok)),
                json!([{"content": step, "status": "completed"}]),
            )
            .await;
        }
        write(
            &tool,
            &work_ctx(1, 6, None),
            json!([{"content": "a huge step", "status": "in_progress"}]),
        )
        .await;
        // No slot on this ctx — the feature is off for this run.
        write(
            &tool,
            &work_ctx(1, 30, Some(Outcome::Ok)),
            json!([{"content": "a huge step", "status": "completed"}]),
        )
        .await;
    }

    #[tokio::test]
    async fn a_span_outlier_writes_a_candidate_into_the_slot_when_present() {
        let tool = TodoTool::new();
        // The tool's own contract ("pass the COMPLETE list every time") means
        // a real write carries every step touched so far, finished ones
        // included — never just the one currently changing. `completed`'s
        // own sweep (`advance`, beside `started`/`flagged`) now prunes
        // against exactly that list, so a test that sent one-item plans
        // per call would prune its own history before this trigger could
        // ever see two siblings.
        let mut items: Vec<Value> = Vec::new();
        // Two small completed steps establish a baseline mean of ~2.5.
        for (i, n) in [2u32, 3u32].into_iter().enumerate() {
            let step = format!("small step {i}");
            items.push(json!({"content": step, "status": "in_progress"}));
            let (start_ctx, _) = escalation_ctx(1, 0, None, 0);
            write(&tool, &start_ctx, Value::Array(items.clone())).await;
            items.last_mut().unwrap()["status"] = json!("completed");
            let (done_ctx, _) = escalation_ctx(1, n, Some(Outcome::Ok), 0);
            write(&tool, &done_ctx, Value::Array(items.clone())).await;
        }
        items.push(json!({"content": "a huge step", "status": "in_progress"}));
        let (start_ctx, _) = escalation_ctx(1, 5, None, 0);
        write(&tool, &start_ctx, Value::Array(items.clone())).await;
        items.last_mut().unwrap()["status"] = json!("completed");
        let (done_ctx, slot) = escalation_ctx(1, 25, Some(Outcome::Ok), 0);
        write(&tool, &done_ctx, Value::Array(items.clone())).await;
        let escalation = slot
            .lock()
            .unwrap()
            .clone()
            .expect("20 calls against a mean of 2.5 should have written a candidate");
        assert_eq!(
            escalation.reason,
            crate::step::EscalationReason::SpanOutlier
        );
        assert_eq!(escalation.step, "a huge step");
    }

    /// The review finding: `completed` must not survive a step falling out
    /// of the live plan, or a wholesale plan rewrite (or a second
    /// conversation reusing the same workspace-keyed list) leaves stale
    /// history behind as the mean new steps get judged against.
    #[tokio::test]
    async fn completed_history_is_pruned_once_a_step_leaves_the_live_plan() {
        let tool = TodoTool::new();
        let mut items: Vec<Value> = Vec::new();
        for (i, n) in [2u32, 3u32].into_iter().enumerate() {
            let step = format!("small step {i}");
            items.push(json!({"content": step, "status": "in_progress"}));
            let (start_ctx, _) = escalation_ctx(1, 0, None, 0);
            write(&tool, &start_ctx, Value::Array(items.clone())).await;
            items.last_mut().unwrap()["status"] = json!("completed");
            let (done_ctx, _) = escalation_ctx(1, n, Some(Outcome::Ok), 0);
            write(&tool, &done_ctx, Value::Array(items.clone())).await;
        }
        // A wholesale rewrite: neither of the two small steps rides along.
        let (start_ctx, _) = escalation_ctx(1, 5, None, 0);
        write(
            &tool,
            &start_ctx,
            json!([{"content": "a huge step", "status": "in_progress"}]),
        )
        .await;
        let (done_ctx, slot) = escalation_ctx(1, 25, Some(Outcome::Ok), 0);
        write(
            &tool,
            &done_ctx,
            json!([{"content": "a huge step", "status": "completed"}]),
        )
        .await;
        assert!(
            slot.lock().unwrap().is_none(),
            "a rewritten plan must not escalate against a mean from steps it no longer holds"
        );
    }

    /// The review finding one step further than the test above: that one
    /// puts the rewrite (starting "a huge step" with no small steps in the
    /// array) and the *completion* in two separate writes, so the first
    /// write's own sweep already prunes `self.completed` before the second
    /// write's snapshot is even taken — the bug never gets a chance to
    /// appear. Here the rewrite and the completion land in the *same*
    /// write: "huge step" is started with the small steps still present
    /// (an ordinary write, not a rewrite), then completed in a write whose
    /// item array holds only itself. Without filtering the snapshot by
    /// `live`, that completion would still see the stale two-step mean the
    /// sweep has not pruned yet.
    #[tokio::test]
    async fn a_rewrite_and_a_completion_in_the_same_write_still_prunes_the_baseline() {
        let tool = TodoTool::new();
        let mut items: Vec<Value> = Vec::new();
        for (i, n) in [2u32, 3u32].into_iter().enumerate() {
            let step = format!("small step {i}");
            items.push(json!({"content": step, "status": "in_progress"}));
            let (start_ctx, _) = escalation_ctx(1, 0, None, 0);
            write(&tool, &start_ctx, Value::Array(items.clone())).await;
            items.last_mut().unwrap()["status"] = json!("completed");
            let (done_ctx, _) = escalation_ctx(1, n, Some(Outcome::Ok), 0);
            write(&tool, &done_ctx, Value::Array(items.clone())).await;
        }
        // An ordinary start — the small steps ride along, so this write is
        // not itself a rewrite and its own sweep prunes nothing.
        items.push(json!({"content": "huge step", "status": "in_progress"}));
        let (start_ctx, _) = escalation_ctx(1, 5, None, 0);
        write(&tool, &start_ctx, Value::Array(items.clone())).await;
        // The rewrite and the completion together: this write's item array
        // holds only "huge step".
        let (done_ctx, slot) = escalation_ctx(1, 40, Some(Outcome::Ok), 0);
        write(
            &tool,
            &done_ctx,
            json!([{"content": "huge step", "status": "completed"}]),
        )
        .await;
        assert!(
            slot.lock().unwrap().is_none(),
            "the same write that drops the small steps from the plan must not let \
             huge step's own completion see them as its baseline"
        );
    }

    /// The review finding: `completed` is a `Vec` keyed by nothing, unlike
    /// `started` (a `HashMap`, so a revision's fresh mark already replaces
    /// rather than doubles up). A step completed, reopened, and completed
    /// again used to land in `completed` twice under the same name —
    /// inflating `sibling_count`/the mean it feeds, and making the step its
    /// own sibling in the prompt's list. Here "A" completes small, gets
    /// reopened, and completes again large; a later "huge step" must see
    /// exactly one "A" entry (its latest span), not two.
    #[tokio::test]
    async fn a_step_revised_and_recompleted_contributes_one_entry_not_two() {
        let tool = TodoTool::new();
        let mut items: Vec<Value> = Vec::new();

        items.push(json!({"content": "small step 0", "status": "in_progress"}));
        write(
            &tool,
            &escalation_ctx(1, 0, None, 0).0,
            Value::Array(items.clone()),
        )
        .await;
        items.last_mut().unwrap()["status"] = json!("completed");
        write(
            &tool,
            &escalation_ctx(1, 2, Some(Outcome::Ok), 0).0,
            Value::Array(items.clone()),
        )
        .await;

        items.push(json!({"content": "A", "status": "in_progress"}));
        write(
            &tool,
            &escalation_ctx(1, 2, None, 0).0,
            Value::Array(items.clone()),
        )
        .await;
        items.last_mut().unwrap()["status"] = json!("completed");
        write(
            &tool,
            &escalation_ctx(1, 5, Some(Outcome::Ok), 0).0,
            Value::Array(items.clone()),
        )
        .await;

        // Reopen A and complete it again, at a much larger span.
        items.last_mut().unwrap()["status"] = json!("in_progress");
        write(
            &tool,
            &escalation_ctx(1, 5, None, 0).0,
            Value::Array(items.clone()),
        )
        .await;
        items.last_mut().unwrap()["status"] = json!("completed");
        write(
            &tool,
            &escalation_ctx(1, 35, Some(Outcome::Ok), 0).0,
            Value::Array(items.clone()),
        )
        .await;

        items.push(json!({"content": "huge step", "status": "in_progress"}));
        write(
            &tool,
            &escalation_ctx(1, 35, None, 0).0,
            Value::Array(items.clone()),
        )
        .await;
        items.last_mut().unwrap()["status"] = json!("completed");
        let (done_ctx, slot) = escalation_ctx(1, 100, Some(Outcome::Ok), 0);
        write(&tool, &done_ctx, Value::Array(items.clone())).await;

        let escalation = slot
            .lock()
            .unwrap()
            .clone()
            .expect("huge step's span is a clear outlier");
        assert_eq!(
            escalation.sibling_count, 2,
            "A's revision must count once, not twice, among the siblings"
        );
        assert_eq!(
            escalation.siblings.iter().filter(|s| *s == "A").count(),
            1,
            "A must not be listed as its own sibling twice"
        );
    }

    /// The review finding one step past the test above: that one checks a
    /// *later* step's escalation, once the dedupe/push has already run for
    /// "A". This checks "A"'s own re-completion — the moment where its
    /// pre-revision entry is still sitting in `completed_before_this_batch`
    /// (the dedupe/push that would remove it runs *after* the escalation
    /// check, and the batch-level `live` filter does not exclude it either,
    /// since "A" is in `next.items` — it is the very item being completed).
    /// With only "small step 0" as a genuine sibling, this must not clear
    /// `SPAN_OUTLIER_MIN_SIBLINGS`.
    #[tokio::test]
    async fn a_steps_own_pre_revision_entry_does_not_count_as_its_sibling() {
        let tool = TodoTool::new();
        let mut items: Vec<Value> = Vec::new();

        items.push(json!({"content": "small step 0", "status": "in_progress"}));
        write(
            &tool,
            &escalation_ctx(1, 0, None, 0).0,
            Value::Array(items.clone()),
        )
        .await;
        items.last_mut().unwrap()["status"] = json!("completed");
        write(
            &tool,
            &escalation_ctx(1, 2, Some(Outcome::Ok), 0).0,
            Value::Array(items.clone()),
        )
        .await;

        items.push(json!({"content": "A", "status": "in_progress"}));
        write(
            &tool,
            &escalation_ctx(1, 2, None, 0).0,
            Value::Array(items.clone()),
        )
        .await;
        items.last_mut().unwrap()["status"] = json!("completed");
        write(
            &tool,
            &escalation_ctx(1, 4, Some(Outcome::Ok), 0).0,
            Value::Array(items.clone()),
        )
        .await;

        // Reopen A and complete it again, at a span that would clear the
        // outlier floor against a mean of 2.0 (small step 0 and A's own
        // stale entry) but not against the true single-sibling baseline.
        items.last_mut().unwrap()["status"] = json!("in_progress");
        write(
            &tool,
            &escalation_ctx(1, 4, None, 0).0,
            Value::Array(items.clone()),
        )
        .await;
        items.last_mut().unwrap()["status"] = json!("completed");
        let (done_ctx, slot) = escalation_ctx(1, 19, Some(Outcome::Ok), 0);
        write(&tool, &done_ctx, Value::Array(items.clone())).await;

        assert!(
            slot.lock().unwrap().is_none(),
            "A's own pre-revision entry must not count as one of its siblings, \
             leaving only one real sibling — below SPAN_OUTLIER_MIN_SIBLINGS"
        );
    }

    /// The bug an adversarial review found after the round-2 pruning fix
    /// shipped: `advance` loops over every item in one write, and pushes
    /// each landed one onto `self.completed` as it goes — so a step earlier
    /// in the *same* write's array was, before this fix, already counted in
    /// a later step's own mean. Here "medium step" (span 4, below the
    /// outlier floor on its own) lands *before* "huge step" in the same
    /// array; without the snapshot, huge's comparison would see a 3-step,
    /// contaminated baseline instead of the real 2-step one established
    /// before this write ever started.
    #[tokio::test]
    async fn a_step_landing_earlier_in_the_same_write_does_not_contaminate_a_laters_mean() {
        let tool = TodoTool::new();
        let mut items: Vec<Value> = Vec::new();

        // Baseline: two small completed steps, span 2 and 3 (mean 2.5, n=2).
        items.push(json!({"content": "small step 0", "status": "in_progress"}));
        write(
            &tool,
            &escalation_ctx(1, 0, None, 0).0,
            Value::Array(items.clone()),
        )
        .await;
        items.last_mut().unwrap()["status"] = json!("completed");
        write(
            &tool,
            &escalation_ctx(1, 3, Some(Outcome::Ok), 0).0,
            Value::Array(items.clone()),
        )
        .await;

        items.push(json!({"content": "small step 1", "status": "in_progress"}));
        write(
            &tool,
            &escalation_ctx(1, 3, None, 0).0,
            Value::Array(items.clone()),
        )
        .await;
        items.last_mut().unwrap()["status"] = json!("completed");
        write(
            &tool,
            &escalation_ctx(1, 7, Some(Outcome::Ok), 0).0,
            Value::Array(items.clone()),
        )
        .await;

        // "huge step" starts first (span will end up large); "medium step"
        // starts later (span will end up small) — both finish in one write.
        items.push(json!({"content": "huge step", "status": "in_progress"}));
        write(
            &tool,
            &escalation_ctx(1, 7, None, 0).0,
            Value::Array(items.clone()),
        )
        .await;
        items.push(json!({"content": "medium step", "status": "in_progress"}));
        write(
            &tool,
            &escalation_ctx(1, 37, None, 0).0,
            Value::Array(items.clone()),
        )
        .await;

        // "medium step" precedes "huge step" in the array — the exact
        // ordering the bug needed to reach "huge"'s comparison at all.
        let last = items.len() - 1;
        items[last - 1]["status"] = json!("completed"); // medium step
        items[last]["status"] = json!("completed"); // huge step
        items.swap(last - 1, last); // medium now BEFORE huge in the array
        let (done_ctx, slot) = escalation_ctx(1, 42, Some(Outcome::Ok), 0);
        write(&tool, &done_ctx, Value::Array(items.clone())).await;

        let escalation = slot
            .lock()
            .unwrap()
            .clone()
            .expect("huge step's span (33) is a clear outlier against the real baseline");
        assert_eq!(escalation.step, "huge step");
        assert_eq!(
            escalation.sibling_count, 2,
            "medium step landed earlier in this same write and must not count as a third sibling"
        );
        assert_eq!(escalation.sibling_mean_calls, Some(2.5));
    }

    /// Two genuine candidates in one write — the slot holds exactly one, and
    /// it is the first one `advance` reaches, not whichever happened to be
    /// processed last. Silently overwriting an earlier candidate with a
    /// later one would make survival an accident of iteration order.
    #[tokio::test]
    async fn two_outliers_in_one_write_keep_only_the_first_found() {
        let tool = TodoTool::new();
        let mut items: Vec<Value> = Vec::new();

        items.push(json!({"content": "small step 0", "status": "in_progress"}));
        write(
            &tool,
            &escalation_ctx(1, 0, None, 0).0,
            Value::Array(items.clone()),
        )
        .await;
        items.last_mut().unwrap()["status"] = json!("completed");
        write(
            &tool,
            &escalation_ctx(1, 3, Some(Outcome::Ok), 0).0,
            Value::Array(items.clone()),
        )
        .await;

        items.push(json!({"content": "small step 1", "status": "in_progress"}));
        write(
            &tool,
            &escalation_ctx(1, 3, None, 0).0,
            Value::Array(items.clone()),
        )
        .await;
        items.last_mut().unwrap()["status"] = json!("completed");
        write(
            &tool,
            &escalation_ctx(1, 7, Some(Outcome::Ok), 0).0,
            Value::Array(items.clone()),
        )
        .await;

        items.push(json!({"content": "big step A", "status": "in_progress"}));
        write(
            &tool,
            &escalation_ctx(1, 7, None, 0).0,
            Value::Array(items.clone()),
        )
        .await;
        items.push(json!({"content": "big step B", "status": "in_progress"}));
        write(
            &tool,
            &escalation_ctx(1, 40, None, 0).0,
            Value::Array(items.clone()),
        )
        .await;

        let last = items.len() - 1;
        items[last - 1]["status"] = json!("completed"); // big step A, first in the array
        items[last]["status"] = json!("completed"); // big step B, second in the array
        let (done_ctx, slot) = escalation_ctx(1, 80, Some(Outcome::Ok), 0);
        write(&tool, &done_ctx, Value::Array(items.clone())).await;

        let escalation = slot
            .lock()
            .unwrap()
            .clone()
            .expect("both steps' spans are clear outliers");
        assert_eq!(
            escalation.step, "big step A",
            "the first candidate `advance` reaches must win, deterministically"
        );
    }

    #[tokio::test]
    async fn an_unverified_claim_writes_a_candidate_and_a_verified_one_does_not() {
        let tool = TodoTool::new();

        let (start_ctx, _) = escalation_ctx(1, 0, None, 0);
        write(
            &tool,
            &start_ctx,
            json!([{"content": "test that the API responds", "status": "in_progress"}]),
        )
        .await;
        // No verify-shaped call in the span (verify_like stays 0).
        let (done_ctx, slot) = escalation_ctx(1, 3, Some(Outcome::Ok), 0);
        write(
            &tool,
            &done_ctx,
            json!([{"content": "test that the API responds", "status": "completed"}]),
        )
        .await;
        let escalation = slot
            .lock()
            .unwrap()
            .clone()
            .expect("an unverified claim should have written a candidate");
        assert_eq!(
            escalation.reason,
            crate::step::EscalationReason::UnverifiedClaim
        );

        // Same claim, but this time the span actually contains a
        // verify-shaped call — no candidate.
        let (start_ctx, _) = escalation_ctx(2, 0, None, 0);
        write(
            &tool,
            &start_ctx,
            json!([{"content": "test that the widget renders", "status": "in_progress"}]),
        )
        .await;
        let (done_ctx, slot) = escalation_ctx(2, 3, Some(Outcome::Ok), 1);
        write(
            &tool,
            &done_ctx,
            json!([{"content": "test that the widget renders", "status": "completed"}]),
        )
        .await;
        assert!(slot.lock().unwrap().is_none());
    }

    // --- the prediction record ----------------------------------------------

    #[test]
    fn a_prediction_rides_the_carried_block_and_round_trips() {
        let mut step = TodoItem::new("run the tests", Status::InProgress);
        step.expect = Some("cargo test passes".into());
        step.check = Some("cargo test -q".into());
        step.expect_calls = Some(3);
        let plan = Plan {
            goal: None,
            items: vec![
                TodoItem::new("read the config", Status::Completed),
                step.clone(),
            ],
        };
        let rendered = TodoTool::render(&plan);
        assert!(rendered.contains("[~] run the tests\n    expect: cargo test passes\n    check: cargo test -q\n    expect_calls: 3\n"), "{rendered}");
        let block = format!("{CARRIED_HEADER}\n\n## todo\n{rendered}\n");
        let back = TodoTool::parse_carried(&block);
        assert_eq!(
            back.items, plan.items,
            "the prediction survives a compaction with the step"
        );
    }

    #[test]
    fn a_step_whose_content_starts_with_a_prediction_word_is_still_a_step() {
        let plan = Plan {
            goal: None,
            items: vec![
                TodoItem::new("check: the port is free", Status::Pending),
                TodoItem::new("expect: nothing", Status::Pending),
            ],
        };
        let block = format!("{CARRIED_HEADER}\n\n## todo\n{}\n", TodoTool::render(&plan));
        assert_eq!(TodoTool::parse_carried(&block).items, plan.items);
    }

    #[test]
    fn a_record_with_a_wrong_typed_prediction_keeps_the_plan_and_loses_the_field() {
        let items: Vec<TodoItem> = serde_json::from_value(json!([
            {"content": "a", "status": "pending", "expect": 7, "check": ["no"], "expect_calls": "three"},
            {"content": "b", "status": "completed", "expect_calls": 2}
        ]))
        .unwrap();
        assert_eq!(items[0], TodoItem::new("a", Status::Pending));
        assert_eq!(items[1].expect_calls, Some(2));
    }

    #[tokio::test]
    async fn the_model_is_told_about_a_wrong_typed_prediction_rather_than_losing_it() {
        let tool = TodoTool::default();
        let ctx = ctx_in("/tmp");
        for (bad, msg) in [
            (
                json!({"items": [{"content": "a", "status": "pending", "expect": 7}]}),
                "`expect` must be a string",
            ),
            (
                json!({"items": [{"content": "a", "status": "pending", "check": 7}]}),
                "`check` must be a string",
            ),
            (
                json!({"items": [{"content": "a", "status": "pending", "expect_calls": -1}]}),
                "`expect_calls` must be a non-negative integer",
            ),
        ] {
            let out = tool.call(bad, &ctx).await.unwrap();
            assert!(out.is_error, "{}", out.content);
            assert!(out.content.contains(msg), "{}", out.content);
        }
        // And a good one is echoed under its step.
        let out = tool
            .call(
                json!({"items": [{"content": "a", "status": "in_progress", "expect": "it works", "expect_calls": 2}]}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content
                .contains("[~] a\n    expect: it works\n    expect_calls: 2"),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn a_completed_steps_check_is_frozen_and_a_change_is_a_tamper() {
        let tool = TodoTool::default();
        let ws = std::env::temp_dir().join(format!("todo-freeze-{}", uuid::Uuid::new_v4()));
        let ctx = ctx_in(&ws.to_string_lossy());
        let write = |status: &str, check: &str| json!({"items": [{"content": "wire it", "status": status, "check": check}]});
        // Open: the check may be revised.
        tool.call(write("in_progress", "make check"), &ctx)
            .await
            .unwrap();
        let out = tool
            .call(write("in_progress", "make test"), &ctx)
            .await
            .unwrap();
        assert!(!out.content.contains("was changed"), "{}", out.content);
        // Completed against `make test`.
        tool.call(write("completed", "make test"), &ctx)
            .await
            .unwrap();
        assert_eq!(tool.tampered_in(&ws), 0);
        // Rewriting the check after the fact is reported, and counted.
        let out = tool.call(write("completed", "true"), &ctx).await.unwrap();
        assert!(
            out.content
                .contains("was changed on or after the write that marked it done"),
            "{}",
            out.content
        );
        assert_eq!(tool.tampered_in(&ws), 1);
        // Re-stating the frozen check is not a tamper.
        let out = tool
            .call(write("completed", "make test"), &ctx)
            .await
            .unwrap();
        assert!(!out.content.contains("was changed"), "{}", out.content);
        assert_eq!(tool.tampered_in(&ws), 1);
    }

    /// The one write that both completes the step and swaps its check is
    /// the post-hoc rewrite the freeze exists for, and the first cut let it
    /// through by gating on the *previous* status (found on review).
    #[tokio::test]
    async fn swapping_the_check_in_the_completing_write_is_a_tamper() {
        let tool = TodoTool::default();
        let ws = std::env::temp_dir().join(format!("todo-freeze-{}", uuid::Uuid::new_v4()));
        let ctx = ctx_in(&ws.to_string_lossy());
        let write = |status: &str, check: &str| json!({"items": [{"content": "wire it", "status": status, "check": check}]});
        tool.call(write("in_progress", "make test"), &ctx)
            .await
            .unwrap();
        let out = tool.call(write("completed", "true"), &ctx).await.unwrap();
        assert!(
            out.content.contains("was changed on or after the write"),
            "{}",
            out.content
        );
        assert_eq!(tool.tampered_in(&ws), 1);
        // A step completed with the check it declared while open is clean,
        // and one that first declares a check on the completing write
        // freezes that declaration.
        let write2 = |status: &str, check: Option<&str>| {
            let mut item = json!({"content": "ship it", "status": status});
            if let Some(c) = check {
                item["check"] = json!(c);
            }
            json!({"items": [{"content": "wire it", "status": "completed", "check": "make test"}, item]})
        };
        tool.call(write2("in_progress", None), &ctx).await.unwrap();
        let out = tool
            .call(write2("completed", Some("cargo test")), &ctx)
            .await
            .unwrap();
        assert!(
            !out.content.contains("\"ship it\" was changed"),
            "{}",
            out.content
        );
        assert_eq!(tool.tampered_in(&ws), 1);
        let out = tool
            .call(write2("completed", Some("true")), &ctx)
            .await
            .unwrap();
        assert!(
            out.content.contains("\"ship it\" was changed"),
            "{}",
            out.content
        );
        assert_eq!(tool.tampered_in(&ws), 2);
    }

    /// Reopening the step must not release the freeze — the first fix let
    /// `completed → in_progress (new check) → completed` through with
    /// `tampered` at zero, and dropping the step and re-adding it as
    /// `pending` is the same door (found on the second review pass).
    #[tokio::test]
    async fn reopening_or_re_adding_a_step_does_not_release_its_frozen_check() {
        let tool = TodoTool::default();
        let ws = std::env::temp_dir().join(format!("todo-freeze-{}", uuid::Uuid::new_v4()));
        let ctx = ctx_in(&ws.to_string_lossy());
        let write = |status: &str, check: &str| json!({"items": [{"content": "wire it", "status": status, "check": check}]});
        tool.call(write("in_progress", "make test"), &ctx)
            .await
            .unwrap();
        tool.call(write("completed", "make test"), &ctx)
            .await
            .unwrap();
        // Reopen with a new check: reported at the reopen, not only at the
        // second completion.
        let out = tool.call(write("in_progress", "true"), &ctx).await.unwrap();
        assert!(out.content.contains("was changed"), "{}", out.content);
        assert_eq!(tool.tampered_in(&ws), 1);
        let out = tool.call(write("completed", "true"), &ctx).await.unwrap();
        assert!(out.content.contains("was changed"), "{}", out.content);
        assert_eq!(tool.tampered_in(&ws), 2);
        // Reopening and re-stating the frozen check is fine.
        let out = tool
            .call(write("in_progress", "make test"), &ctx)
            .await
            .unwrap();
        assert!(!out.content.contains("was changed"), "{}", out.content);
        // Drop it from the plan, then re-add as pending with a different check.
        tool.call(json!({"items": []}), &ctx).await.unwrap();
        let out = tool.call(write("pending", "echo ok"), &ctx).await.unwrap();
        assert!(out.content.contains("was changed"), "{}", out.content);
        assert_eq!(tool.tampered_in(&ws), 3);
    }

    /// The tamper is not only announced: the plan the tool keeps — and so
    /// the echo, the carried block and whatever runs the check — carries the
    /// frozen command, not the rewrite (found on review: the first cut
    /// counted the change and then took it).
    #[tokio::test]
    async fn a_tampered_check_is_put_back_in_the_plan_the_tool_keeps() {
        let tool = TodoTool::default();
        let ws = std::env::temp_dir().join(format!("todo-freeze-{}", uuid::Uuid::new_v4()));
        let ctx = ctx_in(&ws.to_string_lossy());
        let write = |status: &str, check: &str| json!({"items": [{"content": "wire it", "status": status, "check": check}]});
        tool.call(write("in_progress", "make test"), &ctx)
            .await
            .unwrap();
        tool.call(write("completed", "make test"), &ctx)
            .await
            .unwrap();
        let out = tool.call(write("completed", "true"), &ctx).await.unwrap();
        assert!(out.content.contains("stands"), "{}", out.content);
        assert!(
            out.content.contains("    check: make test\n") && !out.content.contains("check: true"),
            "the echo shows the frozen check, not the rewrite: {}",
            out.content
        );
        let kept = tool.items_in(&ws);
        assert_eq!(kept[0].check.as_deref(), Some("make test"));
        let carried = tool.carried_state(&ctx).unwrap();
        assert!(
            carried.body.contains("check: make test"),
            "{}",
            carried.body
        );
        assert!(!carried.body.contains("check: true"), "{}", carried.body);
    }

    /// The third door (found on the fifth review pass): a frozen step
    /// written again with no `check` at all. Restored and counted like a
    /// changed one.
    #[tokio::test]
    async fn omitting_a_frozen_check_is_a_tamper_and_the_check_is_put_back() {
        let tool = TodoTool::default();
        let ws = std::env::temp_dir().join(format!("todo-freeze-{}", uuid::Uuid::new_v4()));
        let ctx = ctx_in(&ws.to_string_lossy());
        tool.call(
            json!({"items": [{"content": "wire it", "status": "in_progress", "check": "make test"}]}),
            &ctx,
        )
        .await
        .unwrap();
        tool.call(
            json!({"items": [{"content": "wire it", "status": "completed", "check": "make test"}]}),
            &ctx,
        )
        .await
        .unwrap();
        let out = tool
            .call(
                json!({"items": [{"content": "wire it", "status": "completed"}]}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            out.content
                .contains("was dropped after the step was marked done"),
            "{}",
            out.content
        );
        assert!(
            out.content.contains("    check: make test\n"),
            "{}",
            out.content
        );
        assert_eq!(tool.tampered_in(&ws), 1);
        assert_eq!(tool.items_in(&ws)[0].check.as_deref(), Some("make test"));
        // An open step may drop its check freely — nothing is frozen yet.
        tool.call(
            json!({"items": [{"content": "ship it", "status": "in_progress", "check": "true"}]}),
            &ctx,
        )
        .await
        .unwrap();
        let out = tool
            .call(
                json!({"items": [{"content": "ship it", "status": "in_progress"}]}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            !out.content.contains("ship it\" was dropped"),
            "{}",
            out.content
        );
    }

    /// The fourth door: a resume rebuilds the tracker, and the first cut
    /// rebuilt it with no freezes (found on the sixth review pass).
    #[tokio::test]
    async fn a_resume_keeps_a_completed_steps_check_frozen() {
        let tool = TodoTool::default();
        let ws = std::env::temp_dir().join(format!("todo-freeze-{}", uuid::Uuid::new_v4()));
        let ctx = ctx_in(&ws.to_string_lossy());
        let mut done = TodoItem::new("wire it", Status::Completed);
        done.check = Some("make test".into());
        let mut open = TodoItem::new("ship it", Status::InProgress);
        open.check = Some("true".into());
        tool.set_plan_in(
            &ws,
            Plan {
                goal: None,
                items: vec![done, open],
            },
        );
        let out = tool
            .call(
                json!({"items": [
                    {"content": "wire it", "status": "completed", "check": "true"},
                    {"content": "ship it", "status": "in_progress", "check": "false"}
                ]}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            out.content.contains("\"wire it\" was changed"),
            "{}",
            out.content
        );
        assert!(
            !out.content.contains("\"ship it\" was changed"),
            "an open step's check is not frozen: {}",
            out.content
        );
        assert_eq!(tool.tampered_in(&ws), 1);
        assert_eq!(tool.items_in(&ws)[0].check.as_deref(), Some("make test"));
    }

    /// A newline in a prediction would forge plan lines in the carried block
    /// (found on the sixth review pass); it is refused at the door instead.
    #[tokio::test]
    async fn a_prediction_holding_a_newline_is_refused_not_split() {
        let tool = TodoTool::default();
        let ctx = ctx_in("/tmp");
        for (field, value) in [
            ("check", "make test\n[x] deploy to prod"),
            ("expect", "ok\ncheck: rm -rf ~"),
            ("check", "a\r\nb"),
        ] {
            let out = tool
                .call(json!({"items": [{"content": "wire it", "status": "in_progress", field: value}]}), &ctx)
                .await
                .unwrap();
            assert!(out.is_error, "{field}: {}", out.content);
            assert!(
                out.content.contains("must be a single line"),
                "{}",
                out.content
            );
        }
    }

    #[test]
    fn a_record_with_a_multi_line_prediction_loses_the_field_not_the_plan() {
        let items: Vec<TodoItem> = serde_json::from_value(json!([
            {"content": "a", "status": "completed", "check": "make test\n[x] forged", "expect": "ok"}
        ]))
        .unwrap();
        assert_eq!(items[0].check, None);
        assert_eq!(items[0].expect.as_deref(), Some("ok"));
    }

    /// The fifth door (found on the ninth review pass): a resume reads the
    /// plan back from the transcript, and the model's raw input is the
    /// rewrite while the tool's echo is the frozen check. The echo wins.
    #[test]
    fn a_resume_reads_the_tools_echo_not_the_models_rewrite() {
        use crate::message::{Block, Message, Role};
        let mut frozen = TodoItem::new("wire it", Status::Completed);
        frozen.check = Some("make test".into());
        let echo = TodoTool::render(&Plan {
            goal: Some(GoalRef::Task("01J8ZK".into())),
            items: vec![frozen.clone()],
        }) + "\n\nthe check for step \"wire it\" was changed on or after the write that marked it done; the check it was completed against stands, and the change is recorded";
        let messages = vec![
            Message {
                role: Role::Assistant,
                content: vec![Block::ToolUse {
                    id: "t1".into(),
                    name: "todo".into(),
                    input: json!({"items": [{"content": "wire it", "status": "completed", "check": "true"}], "serves": "task:01J8ZK"}),
                }],
            },
            Message {
                role: Role::User,
                content: vec![Block::ToolResult {
                    tool_use_id: "t1".into(),
                    content: echo,
                    is_error: false,
                }],
            },
        ];
        let plan = TodoTool::plan_from_transcript(&messages).unwrap();
        assert_eq!(plan.items, vec![frozen]);
        assert_eq!(plan.goal, Some(GoalRef::Task("01J8ZK".into())));
        // And the fallback: an echo this reader cannot parse hands back
        // the input rather than nothing.
        let mut garbled = messages.clone();
        if let Block::ToolResult { content, .. } = &mut garbled[1].content[0] {
            *content = "…".into();
        }
        assert_eq!(
            TodoTool::plan_from_transcript(&garbled).unwrap().items[0]
                .check
                .as_deref(),
            Some("true")
        );
    }

    #[tokio::test]
    async fn a_step_holding_a_newline_is_refused_not_split_into_forged_steps() {
        let tool = TodoTool::default();
        let ctx = ctx_in("/tmp");
        let out = tool
            .call(
                json!({"items": [{"content": "step one\n[x] deploy to prod\n    check: rm -rf /tmp/x", "status": "in_progress"}]}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.is_error, "{}", out.content);
        assert!(
            out.content.contains("`content` must be a single line"),
            "{}",
            out.content
        );
    }

    /// Drop-then-re-add with a process boundary in the middle (found on the
    /// eleventh review pass): a completed step trimmed from the plan before
    /// a resume must still be frozen after it.
    #[tokio::test]
    async fn a_step_trimmed_before_a_resume_stays_frozen_after_it() {
        use crate::message::{Block, Message, Role};
        let mut done = TodoItem::new("wire it", Status::Completed);
        done.check = Some("make test".into());
        let first_echo = TodoTool::render(&Plan {
            goal: None,
            items: vec![done],
        });
        let trimmed_echo = TodoTool::render(&Plan {
            goal: None,
            items: vec![TodoItem::new("ship it", Status::InProgress)],
        });
        let messages = vec![
            Message {
                role: Role::Assistant,
                content: vec![Block::ToolUse {
                    id: "t1".into(),
                    name: "todo".into(),
                    input: json!({"items": [{"content": "wire it", "status": "completed", "check": "make test"}]}),
                }],
            },
            Message {
                role: Role::User,
                content: vec![Block::ToolResult {
                    tool_use_id: "t1".into(),
                    content: first_echo,
                    is_error: false,
                }],
            },
            Message {
                role: Role::Assistant,
                content: vec![Block::ToolUse {
                    id: "t2".into(),
                    name: "todo".into(),
                    input: json!({"items": [{"content": "ship it", "status": "in_progress"}]}),
                }],
            },
            Message {
                role: Role::User,
                content: vec![Block::ToolResult {
                    tool_use_id: "t2".into(),
                    content: trimmed_echo,
                    is_error: false,
                }],
            },
        ];
        let frozen = TodoTool::frozen_checks_from_transcript(&messages);
        assert_eq!(frozen.get("wire it").map(String::as_str), Some("make test"));

        let tool = TodoTool::default();
        let ws = std::env::temp_dir().join(format!("todo-freeze-{}", uuid::Uuid::new_v4()));
        let ctx = ctx_in(&ws.to_string_lossy());
        assert_eq!(tool.rehydrate(&ws, &messages), Some(1));
        let out = tool
            .call(
                json!({"items": [
                    {"content": "ship it", "status": "in_progress"},
                    {"content": "wire it", "status": "completed", "check": "true"}
                ]}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            out.content.contains("\"wire it\" was changed"),
            "{}",
            out.content
        );
        assert_eq!(tool.tampered_in(&ws), 1);
        assert_eq!(
            tool.items_in(&ws)
                .iter()
                .find(|i| i.content == "wire it")
                .and_then(|i| i.check.clone())
                .as_deref(),
            Some("make test")
        );
    }
}
