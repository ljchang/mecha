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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: Status,
}

/// One list per conversation, keyed by the run's workspace.
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
    lists: Mutex<HashMap<PathBuf, Vec<TodoItem>>>,
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
    pub fn set_items_in(&self, workspace: &Path, items: Vec<TodoItem>) {
        self.lists.lock().unwrap().insert(workspace.into(), items);
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
        let items = Self::from_transcript(messages)?;
        let n = items.len();
        self.set_items_in(workspace, items);
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
                                return Some(items);
                            }
                        }
                    }
                    Block::Text { text } if text.trim_start().starts_with(CARRIED_HEADER) => {
                        let items = Self::parse_carried(text);
                        if !items.is_empty() {
                            return Some(items);
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
    fn parse_carried(text: &str) -> Vec<TodoItem> {
        let mut lines = text.lines().skip_while(|l| l.trim() != "## todo");
        if lines.next().is_none() {
            return Vec::new();
        }
        lines
            .take_while(|l| !l.trim_start().starts_with("## "))
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
            .collect()
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
            .cloned()
            .unwrap_or_default()
    }

    fn render(items: &[TodoItem]) -> String {
        if items.is_empty() {
            return "(the list is empty)".to_string();
        }
        let done = items
            .iter()
            .filter(|i| i.status == Status::Completed)
            .count();
        let mut out = format!("{done}/{} done\n", items.len());
        for item in items {
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
         it is done rather than in a batch at the end. Skip this tool only for work of \
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
        let items = lists.get(&ctx.workspace)?;
        // An empty list is genuinely nothing to carry, and an empty section in
        // the prompt reads as "the plan is finished" rather than "there was
        // never a plan".
        if items.is_empty() {
            return None;
        }
        Some(CarriedState {
            label: "todo".into(),
            body: Self::render(items),
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

        let rendered = Self::render(&items);
        self.lists
            .lock()
            .unwrap()
            .insert(ctx.workspace.clone(), items);
        Ok(ToolOutput::ok(format!("{rendered}{note}")))
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
        let block = format!(
            "{CARRIED_HEADER}\n\n## todo\n{}\n",
            TodoTool::render(&items)
        );
        let back = TodoTool::parse_carried(&block);

        assert_eq!(back.len(), items.len());
        for (a, b) in back.iter().zip(&items) {
            assert_eq!(a.content, b.content);
            assert_eq!(a.status, b.status);
        }
    }

    /// A block carries every stateful tool's section, so the walk must stop
    /// at the next heading rather than swallowing a neighbour's lines.
    #[test]
    fn a_neighbouring_carried_section_is_not_absorbed() {
        let block =
            format!("{CARRIED_HEADER}\n\n## todo\n1/1 done\n[x] mine\n\n## skill\n[x] not mine\n");
        let items = TodoTool::parse_carried(&block);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].content, "mine");
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
}
