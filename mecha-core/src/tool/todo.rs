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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: Status,
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
    /// How many times this tool has been called for this plan.
    ///
    /// Subtracted from every span: rewriting the list is bookkeeping, and a
    /// model that revises its plan three times mid-step would otherwise show
    /// three calls of "work" for a step where nothing happened.
    own_calls: u32,
}

/// Where one step's span starts, in the two units it has to be measured in.
#[derive(Clone, Copy)]
struct Mark {
    work: crate::step::Work,
    own_calls: u32,
}

impl Tracked {
    /// Fold one plan write in, and say what the steps that just finished
    /// actually did.
    ///
    /// `work` is the run's counters as of *before* this turn's batch — which
    /// is also before this call itself reaches the trace, so it and
    /// `own_calls` are measured at the same instant and their difference is a
    /// span. Anything unknown produces no line at all: a step never seen in
    /// progress, a run whose counters restarted, a context nobody stamped.
    fn advance(&mut self, next: Plan, work: Option<crate::step::Work>) -> Vec<String> {
        let before: HashMap<&str, Status> = self
            .plan
            .items
            .iter()
            .map(|i| (i.content.as_str(), i.status))
            .collect();

        let mut lines = Vec::new();
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
                                own_calls: self.own_calls,
                            },
                        );
                    }
                }
                Status::Completed if was != Some(Status::Completed) => {
                    let Some(mark) = self.started.remove(&item.content) else {
                        continue;
                    };
                    let Some(span) = work.and_then(|w| {
                        w.since(mark.work, self.own_calls.saturating_sub(mark.own_calls))
                    }) else {
                        continue;
                    };
                    match crate::step::appraise(span)
                        .line(&item.content, self.flagged.contains(&item.content))
                    {
                        Some(line) => {
                            self.flagged.insert(item.content.clone());
                            lines.push(line);
                        }
                        // It landed, so the next thing to go wrong here is a
                        // first time again. Not while siblings are in flight:
                        // that reads as landed because nothing is known yet,
                        // which is not the same as having gone well.
                        None if span.in_flight == 0 => {
                            self.flagged.remove(&item.content);
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
        let live: std::collections::HashSet<&str> =
            next.items.iter().map(|i| i.content.as_str()).collect();
        self.started.retain(|k, _| live.contains(k.as_str()));
        self.flagged.retain(|k| live.contains(k.as_str()));
        drop(live);

        self.plan = next;
        self.own_calls += 1;
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
    /// A fresh record, not a plan swapped into the old one: the spans this
    /// tool measures are counted from a run's trace, and a plan restored from
    /// a transcript was written by a process whose counters are gone. Keeping
    /// the marks would measure the resumed run's work against the killed
    /// one's — the exact wrong-units mistake rung 4 made reading headroom off
    /// one run's outcome for a whole episode.
    pub fn set_plan_in(&self, workspace: &Path, plan: Plan) {
        self.lists.lock().unwrap().insert(
            workspace.into(),
            Tracked {
                plan,
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
        self.set_plan_in(workspace, plan);
        Some(n)
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
        let failed: std::collections::HashSet<&str> = messages
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|b| match b {
                Block::ToolResult {
                    tool_use_id,
                    is_error: true,
                    ..
                } => Some(tool_use_id.as_str()),
                _ => None,
            })
            .collect();

        for msg in messages.iter().rev() {
            for block in msg.content.iter().rev() {
                match block {
                    Block::ToolUse { id, name, input }
                        if name == "todo" && !failed.contains(id.as_str()) =>
                    {
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
        // Anchored to the first non-empty line, because `render` always writes
        // it there. Scanning the whole section would let an item whose
        // *content* contains a line beginning `serves task:…` supply the
        // plan's goal — free text deciding what the run is for.
        let goal = section
            .iter()
            .find(|l| !l.trim().is_empty())
            .and_then(|l| l.trim().strip_prefix(SERVES))
            .and_then(GoalRef::parse_lenient);
        let items = section
            .iter()
            .filter_map(|line| {
                let line = line.trim();
                let (marker, rest) = line.split_at(line.char_indices().nth(3)?.0);
                let status = match marker {
                    "[ ]" => Status::Pending,
                    "[~]" => Status::InProgress,
                    "[x]" => Status::Completed,
                    _ => return None,
                };
                let content = rest.trim();
                (!content.is_empty()).then(|| TodoItem {
                    content: content.to_string(),
                    status,
                })
            })
            .collect();
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
        self.lists.lock().unwrap().get(workspace)?.plan.goal.clone()
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
        }
        out
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
        let lists = self.lists.lock().unwrap();
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
        self.lists.lock().unwrap().clear();
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
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
            items.push(TodoItem {
                content: content.to_string(),
                status,
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
        let rendered = Self::render(&plan);
        // What the steps that just finished actually did, against the run's
        // own record of what it has done. The harness computes the fact; the
        // plan action it argues for — accept, revise the step, revise the
        // plan, escalate — is the model's next call, because the plan is the
        // model's. §5.5.
        let findings = self
            .lists
            .lock()
            .unwrap()
            .entry(ctx.workspace.clone())
            .or_default()
            .advance(plan, ctx.work);
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
            TodoItem {
                content: "read the config".into(),
                status: Status::Completed,
            },
            TodoItem {
                content: "fix the port".into(),
                status: Status::InProgress,
            },
            TodoItem {
                content: "run the tests".into(),
                status: Status::Pending,
            },
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
}
