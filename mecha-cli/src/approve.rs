//! Terminal approval gate.
//!
//! Anything that isn't read-only stops here and asks. "Always" is remembered
//! per tool for the life of the process — never persisted, because a decision
//! made in one session shouldn't silently apply to the next.

use async_trait::async_trait;
use mecha_core::tool::{Approver, Decision, Tool};
use serde_json::Value;
use std::collections::HashSet;
use std::io::Write;
use std::sync::Mutex;

#[derive(Default)]
pub struct TerminalApprover {
    always: Mutex<HashSet<String>>,
}

#[async_trait]
impl Approver for TerminalApprover {
    async fn approve(&self, tool: &dyn Tool, input: &Value) -> Decision {
        if self.always.lock().unwrap().contains(tool.name()) {
            return Decision::Allow;
        }
        self.ask(tool, input, None).await
    }

    /// Past the `always` list on purpose: an escalation is the interlock
    /// asking a person about *this* call, and a standing yes for the tool is
    /// not that.
    async fn escalate(&self, tool: &dyn Tool, input: &Value, why: &str) -> Decision {
        self.ask(tool, input, Some(why)).await
    }
}

impl TerminalApprover {
    async fn ask(&self, tool: &dyn Tool, input: &Value, why: Option<&str>) -> Decision {
        let name = tool.name().to_string();
        let summary = summarize(&name, input);
        let preface = why.map(|w| format!("  {w}\n")).unwrap_or_default();
        let prompt =
            format!("\n{preface}  {name}  {summary}\n  allow? [y]es / [a]lways / [n]o / [q]uit > ");

        // Reading stdin blocks; keep it off the async runtime's worker threads.
        let answer = tokio::task::spawn_blocking(move || {
            print!("{prompt}");
            let _ = std::io::stdout().flush();
            let mut line = String::new();
            match std::io::stdin().read_line(&mut line) {
                // EOF (piped stdin, closed terminal) is not consent.
                Ok(0) | Err(_) => "n".to_string(),
                Ok(_) => line.trim().to_lowercase(),
            }
        })
        .await
        .unwrap_or_else(|_| "n".to_string());

        match answer.chars().next() {
            Some('y') | None => Decision::Allow,
            Some('a') => {
                self.always.lock().unwrap().insert(tool.name().to_string());
                Decision::Allow
            }
            Some('q') => Decision::Deny("the user stopped the run".into()),
            _ => Decision::Deny("the user declined this call".into()),
        }
    }
}

/// A one-line gist of what the call will do. The full arguments are available
/// with `--verbose`; this is what someone reads before deciding.
pub fn summarize(tool: &str, input: &Value) -> String {
    let field = |key: &str| input.get(key).and_then(Value::as_str);

    let text = match tool {
        "shell" => field("command").map(str::to_string),
        "fs_write" | "fs_edit" | "fs_read" | "fs_list" => field("path").map(str::to_string),
        "http_fetch" => field("url").map(str::to_string),
        _ => None,
    }
    .unwrap_or_else(|| {
        // Unknown tool (usually MCP): fall back to compact JSON.
        serde_json::to_string(input).unwrap_or_default()
    });

    let flat = text.replace('\n', " ");
    if flat.chars().count() > 100 {
        format!("{}…", flat.chars().take(100).collect::<String>())
    } else {
        flat
    }
}
