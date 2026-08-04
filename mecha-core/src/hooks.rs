//! Lifecycle hooks: user commands that attach to the loop without touching it.
//!
//! Three events. `pre_tool` runs before the approver and can deny a call;
//! `post_tool` observes a completed call; `session_end` fires when a front-end
//! closes a recorded session. Each hook is a shell command run as the user in
//! the workspace, with the event payload as one JSON object on stdin.
//!
//! Two policy decisions worth stating out loud:
//!
//! - **`pre_tool` fails closed.** Exit 0 allows; exit 2 denies with the hook's
//!   output as the reason; any other exit, a spawn failure, or a timeout also
//!   **denies**. A policy hook that cannot run and silently allows is the
//!   silently-degrading-sandbox mistake with a different spelling. Observers
//!   (`post_tool`, `session_end`) are best-effort: their failures are logged
//!   and swallowed, because they cannot be load-bearing.
//! - **Hooks run before the human.** A `pre_tool` denial never reaches the
//!   approver — mechanical policy is cheaper than an interruption, and a hook
//!   cannot be talked into clicking yes. The trifecta interlock still sits in
//!   front of everything; hooks do not replace it and cannot loosen it.
//!
//! The order in config is the order they run. For `pre_tool`, the first denial
//! wins and later hooks do not fire.

use crate::config::HookConfig;
use anyhow::{bail, Result};
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

const DEFAULT_TIMEOUT_SECS: u64 = 10;

/// What a `pre_tool` hook decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookVerdict {
    Allow,
    Deny(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event {
    PreTool,
    PostTool,
    SessionEnd,
}

#[derive(Debug)]
struct Hook {
    event: Event,
    command: String,
    tools: Vec<String>,
    timeout: Duration,
}

impl Hook {
    fn matches_tool(&self, tool: &str) -> bool {
        self.tools.is_empty() || self.tools.iter().any(|t| t == tool)
    }
}

/// The validated hook set for one process. Empty is the common case and free.
#[derive(Debug, Default)]
pub struct HookSet {
    hooks: Vec<Hook>,
}

impl HookSet {
    /// Validate config into a runnable set. Unknown events and empty commands
    /// are startup errors: a hook that can never fire is a typo, and finding
    /// out at startup beats finding out during an incident.
    pub fn from_config(configs: &[HookConfig]) -> Result<Self> {
        let mut hooks = Vec::new();
        for c in configs {
            let event = match c.event.as_str() {
                "pre_tool" => Event::PreTool,
                "post_tool" => Event::PostTool,
                "session_end" => Event::SessionEnd,
                other => bail!(
                    "hook event {other:?} is not one of pre_tool, post_tool, session_end"
                ),
            };
            if c.command.trim().is_empty() {
                bail!("a hook for {:?} has an empty command", c.event);
            }
            hooks.push(Hook {
                event,
                command: c.command.clone(),
                tools: c.tools.clone(),
                timeout: Duration::from_secs(c.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS)),
            });
        }
        Ok(HookSet { hooks })
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    /// True when any `pre_tool` or `post_tool` hook exists — lets the dispatch
    /// path skip payload construction entirely in the common empty case.
    pub fn watches_tools(&self) -> bool {
        self.hooks.iter().any(|h| matches!(h.event, Event::PreTool | Event::PostTool))
    }

    async fn run_one(hook: &Hook, payload: &Value, workdir: &std::path::Path) -> Result<(i32, String)> {
        let mut child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&hook.command)
            .current_dir(workdir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let bytes = serde_json::to_vec(payload)?;

        // The timeout covers the stdin write as well as the wait. A hook that
        // never reads stdin blocks the write once the payload outgrows the
        // pipe buffer — a pre_tool hook fed a large fs_write input would hang
        // the run forever with the timeout never starting. Dropping the timed
        // future drops the child, and kill_on_drop reaps it.
        let fut = async move {
            if let Some(mut stdin) = child.stdin.take() {
                // Best-effort: a hook that decides without reading is fine.
                let _ = stdin.write_all(&bytes).await;
                drop(stdin);
            }
            child.wait_with_output().await
        };
        let out = tokio::time::timeout(hook.timeout, fut).await??;
        let mut text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if text.is_empty() {
            text = String::from_utf8_lossy(&out.stderr).trim().to_string();
        }
        Ok((out.status.code().unwrap_or(-1), text))
    }

    /// Run the matching `pre_tool` hooks in order. First denial wins.
    pub async fn pre_tool(&self, tool: &str, input: &Value, workdir: &std::path::Path) -> HookVerdict {
        for hook in self.hooks.iter().filter(|h| h.event == Event::PreTool) {
            if !hook.matches_tool(tool) {
                continue;
            }
            let payload = serde_json::json!({
                "event": "pre_tool",
                "tool": tool,
                "input": input,
            });
            match Self::run_one(hook, &payload, workdir).await {
                Ok((0, _)) => {}
                Ok((2, reason)) => {
                    return HookVerdict::Deny(if reason.is_empty() {
                        format!("blocked by hook `{}`", hook.command)
                    } else {
                        reason
                    });
                }
                // Fail closed: an exit code the contract does not define, a
                // crash, or a timeout is not permission.
                Ok((code, reason)) => {
                    return HookVerdict::Deny(format!(
                        "hook `{}` exited {code} (exit 0 allows, 2 denies){}",
                        hook.command,
                        if reason.is_empty() { String::new() } else { format!(": {reason}") }
                    ));
                }
                Err(e) => {
                    return HookVerdict::Deny(format!("hook `{}` failed to run: {e}", hook.command));
                }
            }
        }
        HookVerdict::Allow
    }

    /// Notify `post_tool` observers. Best-effort by design.
    pub async fn post_tool(
        &self,
        tool: &str,
        input: &Value,
        is_error: bool,
        content: &str,
        workdir: &std::path::Path,
    ) {
        for hook in self.hooks.iter().filter(|h| h.event == Event::PostTool) {
            if !hook.matches_tool(tool) {
                continue;
            }
            let payload = serde_json::json!({
                "event": "post_tool",
                "tool": tool,
                "input": input,
                "is_error": is_error,
                // Bounded: a hook that wants the whole output can read the
                // session file; stdin is for deciding, not archiving.
                "content": content.chars().take(4000).collect::<String>(),
            });
            if let Err(e) = Self::run_one(hook, &payload, workdir).await {
                tracing::warn!("post_tool hook `{}` failed: {e}", hook.command);
            }
        }
    }

    /// Notify `session_end` observers. Best-effort by design.
    pub async fn session_end(&self, session_id: &str, path: &std::path::Path, workdir: &std::path::Path) {
        for hook in self.hooks.iter().filter(|h| h.event == Event::SessionEnd) {
            let payload = serde_json::json!({
                "event": "session_end",
                "session_id": session_id,
                "path": path,
            });
            if let Err(e) = Self::run_one(hook, &payload, workdir).await {
                tracing::warn!("session_end hook `{}` failed: {e}", hook.command);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cfg(event: &str, command: &str) -> HookConfig {
        HookConfig {
            event: event.into(),
            command: command.into(),
            tools: Vec::new(),
            timeout_secs: Some(5),
        }
    }

    #[test]
    fn an_unknown_event_is_a_startup_error() {
        let err = HookSet::from_config(&[cfg("pre-tool", "true")]).unwrap_err().to_string();
        assert!(err.contains("pre-tool"), "{err}");
        assert!(HookSet::from_config(&[cfg("pre_tool", "  ")]).is_err());
    }

    #[tokio::test]
    async fn exit_zero_allows_and_exit_two_denies_with_the_reason() {
        let set = HookSet::from_config(&[cfg("pre_tool", "true")]).unwrap();
        let v = set.pre_tool("echo", &json!({}), std::path::Path::new(".")).await;
        assert_eq!(v, HookVerdict::Allow);

        let set = HookSet::from_config(&[cfg("pre_tool", "echo not today; exit 2")]).unwrap();
        let v = set.pre_tool("echo", &json!({}), std::path::Path::new(".")).await;
        assert_eq!(v, HookVerdict::Deny("not today".into()));
    }

    #[tokio::test]
    async fn an_undefined_exit_code_fails_closed() {
        let set = HookSet::from_config(&[cfg("pre_tool", "exit 1")]).unwrap();
        match set.pre_tool("echo", &json!({}), std::path::Path::new(".")).await {
            HookVerdict::Deny(reason) => assert!(reason.contains("exited 1"), "{reason}"),
            HookVerdict::Allow => panic!("an undefined exit code must not be permission"),
        }
    }

    #[tokio::test]
    async fn a_hook_that_hangs_fails_closed_at_its_timeout() {
        let mut c = cfg("pre_tool", "sleep 30");
        c.timeout_secs = Some(1);
        let set = HookSet::from_config(&[c]).unwrap();
        match set.pre_tool("echo", &json!({}), std::path::Path::new(".")).await {
            HookVerdict::Deny(reason) => assert!(reason.contains("failed to run"), "{reason}"),
            HookVerdict::Allow => panic!("a timeout must not be permission"),
        }
    }

    #[tokio::test]
    async fn a_hook_that_never_reads_a_large_payload_still_times_out() {
        // The bug this pins: the stdin write used to sit outside the timeout,
        // so a hook that never reads blocked write_all forever once the
        // payload outgrew the pipe buffer — the timeout never started.
        let mut c = cfg("pre_tool", "sleep 30");
        c.timeout_secs = Some(1);
        let set = HookSet::from_config(&[c]).unwrap();
        let big = json!({"content": "x".repeat(256 * 1024)});
        let verdict = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            set.pre_tool("fs_write", &big, std::path::Path::new(".")),
        )
        .await
        .expect("the hook's own timeout must fire; the write must not wedge it");
        assert!(matches!(verdict, HookVerdict::Deny(_)));
    }

    #[tokio::test]
    async fn the_tool_filter_scopes_a_hook_and_the_payload_reaches_stdin() {
        let marker = std::env::temp_dir().join(format!("mecha-hook-{}", uuid::Uuid::new_v4()));
        let mut c = cfg("pre_tool", &format!("cat > {}; exit 2", marker.display()));
        c.tools = vec!["shell".into()];
        let set = HookSet::from_config(&[c]).unwrap();

        // A tool outside the filter never fires the hook.
        let v = set.pre_tool("echo", &json!({}), std::path::Path::new(".")).await;
        assert_eq!(v, HookVerdict::Allow);
        assert!(!marker.exists());

        // A matching tool does, and the payload arrives on stdin.
        let v = set
            .pre_tool("shell", &json!({"command": "rm -rf /"}), std::path::Path::new("."))
            .await;
        assert!(matches!(v, HookVerdict::Deny(_)));
        let written = std::fs::read_to_string(&marker).unwrap();
        assert!(written.contains("\"event\":\"pre_tool\""));
        assert!(written.contains("rm -rf /"));
        std::fs::remove_file(&marker).ok();
    }

    #[tokio::test]
    async fn post_tool_failures_are_swallowed_because_observers_cannot_be_load_bearing() {
        let set = HookSet::from_config(&[cfg("post_tool", "exit 7")]).unwrap();
        // Nothing to assert beyond "does not panic or error": the call has no
        // way to fail the caller.
        set.post_tool("echo", &json!({}), false, "out", std::path::Path::new(".")).await;
    }
}
