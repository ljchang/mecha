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

use super::{Tool, ToolCtx, ToolOutput};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
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

/// Holds the list for the lifetime of the agent.
#[derive(Default)]
pub struct TodoTool {
    items: Mutex<Vec<TodoItem>>,
}

impl TodoTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Current list, for a UI that wants to render progress live.
    pub fn items(&self) -> Vec<TodoItem> {
        self.items.lock().unwrap().clone()
    }

    fn render(items: &[TodoItem]) -> String {
        if items.is_empty() {
            return "(the list is empty)".to_string();
        }
        let done = items.iter().filter(|i| i.status == Status::Completed).count();
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
        "Record and update your task list for multi-step work. Pass the COMPLETE list \
         every time — it replaces what was there, so include finished items with status \
         `completed`. Write the list once before you start, then update it as you go: \
         exactly one item should be `in_progress` at a time, and an item should be marked \
         `completed` as soon as it is done rather than in a batch at the end. Skip this \
         tool entirely for anything that takes one or two steps."
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

    async fn call(&self, input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
        let Some(raw) = input.get("items").and_then(Value::as_array) else {
            return Ok(ToolOutput::err("`items` must be an array of {content, status}"));
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
            items.push(TodoItem { content: content.to_string(), status });
        }

        // Nudge rather than reject: two items in flight is a mild smell, not an
        // error, and refusing the write would lose the update entirely.
        let in_progress = items.iter().filter(|i| i.status == Status::InProgress).count();
        let mut note = String::new();
        if in_progress > 1 {
            note = format!(
                "\n(note: {in_progress} items are in_progress — finish one before starting another)"
            );
        }

        let rendered = Self::render(&items);
        *self.items.lock().unwrap() = items;
        Ok(ToolOutput::ok(format!("{rendered}{note}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writing_the_list_echoes_it_back_with_progress() {
        let tool = TodoTool::new();
        let out = tool
            .call(
                json!({"items": [
                    {"content": "read the config", "status": "completed"},
                    {"content": "fix the port", "status": "in_progress"},
                    {"content": "run the tests", "status": "pending"}
                ]}),
                &ToolCtx::default(),
            )
            .await
            .unwrap();

        assert!(!out.is_error);
        assert!(out.content.starts_with("1/3 done"));
        assert!(out.content.contains("[x] read the config"));
        assert!(out.content.contains("[~] fix the port"));
        assert!(out.content.contains("[ ] run the tests"));
        assert_eq!(tool.items().len(), 3);
    }

    #[tokio::test]
    async fn the_list_is_replaced_not_appended() {
        let tool = TodoTool::new();
        let ctx = ToolCtx::default();
        tool.call(json!({"items": [{"content": "a", "status": "pending"}]}), &ctx)
            .await
            .unwrap();
        tool.call(json!({"items": [{"content": "b", "status": "pending"}]}), &ctx)
            .await
            .unwrap();

        let items = tool.items();
        assert_eq!(items.len(), 1, "a write replaces the whole list");
        assert_eq!(items[0].content, "b");
    }

    #[tokio::test]
    async fn a_bad_status_is_reported_rather_than_silently_dropped() {
        let tool = TodoTool::new();
        let out = tool
            .call(
                json!({"items": [{"content": "a", "status": "done"}]}),
                &ToolCtx::default(),
            )
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(out.content.contains("expected pending"));
        assert!(tool.items().is_empty(), "a rejected write changes nothing");
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
