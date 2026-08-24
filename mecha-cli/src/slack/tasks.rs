//! The `tasks` and `task` command words: the GTD board from the screen in
//! hand, and a capture onto it.
//!
//! Two established shapes, taken whole. `tasks` is the `triggers` pattern —
//! an exact-word listing with per-row controls, every button composed from a
//! typed [`Action`] so the payload is a fixed verb and a task id, re-resolved
//! against the graph at tap time. `task <text>` is the `note` pattern: a
//! deterministic capture matched before the text can become a prompt, because
//! a capture that depends on a model's mood is not a capture. Both boundaries
//! from `doctor` hold: the connector's gate runs before either word is looked
//! at (the board is the owner's private surface), and everything spawns off
//! the ack path — each child starts an MCP server to reach the graph, and the
//! three-second ack budget is Slack's.
//!
//! **The board is driven through `mecha tasks`, never re-implemented.** The
//! listing reads `mecha tasks list --json` — the same bytes the TUI's
//! `/tasks` modal reads — and a capture runs `mecha tasks add`: one
//! implementation per verb, and nothing this surface can do is missing from
//! a script. `mecha-cli` still knows nothing of the graph's schema; the JSON
//! here is `kg_task_list`'s own answer, passed through the CLI.
//!
//! **Only the singular word captures.** `note` and `notes` both capture, but
//! `tasks` is taken by the board, so `tasks buy milk` would be one keystroke
//! away from a listing and a guess away from a capture — it falls through to
//! the model instead. A capture surface that guesses is worse than a prompt.

use mecha_slack::blocks;
use serde_json::Value;

use super::actions::Action;

/// A message that is the word `tasks` and nothing else, however cased or
/// padded — "show my tasks please" is a prompt for the model, not a command.
pub fn is_tasks_command(text: &str) -> bool {
    text.trim().eq_ignore_ascii_case("tasks")
}

/// `task <text>` (or `task: <text>`), first word exactly: the remainder is
/// captured onto the board. The bare word falls through to the model — it
/// carries nothing to capture — and the plural never captures (see the
/// module doc: `tasks` is the board).
pub fn task_command(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let (first, rest) = trimmed.split_once(char::is_whitespace)?;
    if !first.trim_end_matches(':').eq_ignore_ascii_case("task") {
        return None;
    }
    let body = rest.trim();
    (!body.is_empty()).then(|| body.to_string())
}

/// One task, as the listing shows it. A plain struct so the rendering is a
/// pure function of it and testable without a graph.
pub struct Row {
    pub id: String,
    pub name: String,
    pub status: String,
    pub due: Option<String>,
    pub overdue: bool,
    pub project: Option<String>,
    pub context: Option<String>,
    pub waiting_on: Option<String>,
}

/// Read the board and shape the answer for a thread: the notification
/// fallback line, and the blocks when there is a board to show. The read is
/// a child `mecha tasks list --json` — it starts an MCP server, which is why
/// this is only ever called from spawned work.
pub async fn board() -> (String, Option<Vec<Value>>) {
    let exe = crate::exe::self_exe();
    let out = tokio::process::Command::new(exe)
        .args(["tasks", "list", "--json"])
        .stdin(std::process::Stdio::null())
        .output()
        .await;
    let raw = match out {
        Ok(out) if out.status.success() => out.stdout,
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr);
            return (
                format!(
                    "the board could not be read: {}",
                    err.trim()
                        .lines()
                        .next_back()
                        .unwrap_or("mecha tasks failed")
                ),
                None,
            );
        }
        Err(e) => return (format!("the board could not be read: {e}"), None),
    };
    let Ok(parsed) = serde_json::from_slice::<Value>(&raw) else {
        return (
            "the board answered with something unreadable — try `mecha tasks list`".to_string(),
            None,
        );
    };
    let rows = rows_from(&parsed);
    if rows.is_empty() {
        return (
            "nothing on the board — `task <what>` captures one".to_string(),
            None,
        );
    }
    let today = parsed["today"].as_str().unwrap_or("?");
    let heading = format!("Tasks: {} open · today is {today}", rows.len());
    let mut out = vec![blocks::section(&format!(
        "*Tasks:* {} open · today is {today}.",
        rows.len()
    ))];
    out.extend(listing_blocks(&rows));
    (heading, Some(blocks::cap_blocks(out)))
}

/// The rows out of `kg_task_list`'s answer, in the tool's own order — the
/// board sorts actionable statuses first, then by due date, and re-sorting
/// here would be a second opinion about a store this crate does not own.
fn rows_from(board: &Value) -> Vec<Row> {
    let field = |t: &Value, key: &str| {
        t[key]
            .as_str()
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    board["items"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .map(|t| Row {
            id: t["id"].as_str().unwrap_or("").to_string(),
            name: t["name"].as_str().unwrap_or("").to_string(),
            status: t["status"].as_str().unwrap_or("?").to_string(),
            due: field(t, "due_at"),
            overdue: t["overdue"].as_bool().unwrap_or(false),
            project: field(t, "project"),
            context: field(t, "context"),
            waiting_on: field(t, "waiting_on"),
        })
        .collect()
}

/// The rows as Block Kit: one section and one actions block per task, the
/// buttons composed from typed actions so verb and value cannot drift from
/// what `from_payload` parses. What a row offers follows its status: an
/// inbox capture offers **Next** (commit to it) beside **Done**; every other
/// open task offers **Done** alone — drop, defer and dates stay on the
/// terminal, where the board's full keyboard lives. Capped visibly by the
/// caller like every other report.
pub fn listing_blocks(rows: &[Row]) -> Vec<Value> {
    let button = |action: &Action, label: &str, style: Option<&str>| {
        blocks::button(action.action_id(), label, &action.value(), style)
    };
    let mut out = Vec::new();
    for row in rows {
        let mut meta = vec![format!("`{}`", row.status)];
        if let Some(due) = &row.due {
            meta.push(if row.overdue {
                format!("due {due} — *overdue*")
            } else {
                format!("due {due}")
            });
        }
        for (label, value) in [
            ("project", &row.project),
            ("", &row.context),
            ("waiting on", &row.waiting_on),
        ] {
            if let Some(v) = value {
                meta.push(if label.is_empty() {
                    v.clone()
                } else {
                    format!("{label} {v}")
                });
            }
        }
        out.push(blocks::section(&format!(
            "{}\n{}",
            row.name,
            meta.join(" · ")
        )));
        let mut controls = Vec::new();
        if row.status == "inbox" {
            controls.push(button(
                &Action::TaskNext { id: row.id.clone() },
                "Next",
                None,
            ));
        }
        controls.push(button(
            &Action::TaskDone { id: row.id.clone() },
            "Done",
            Some("primary"),
        ));
        out.push(blocks::actions(controls));
    }
    out
}

/// Capture one task through `mecha tasks add` and hand back what the child
/// reported — the id-and-due line plus the name, so the reply confirms what
/// landed rather than what was typed (the tool resolves `tomorrow` itself).
/// One argv element, never a shell — the `note` capture's rule.
pub async fn capture(body: &str) -> String {
    let exe = crate::exe::self_exe();
    let out = tokio::process::Command::new(exe)
        .args(["tasks", "add", body])
        .stdin(std::process::Stdio::null())
        .output()
        .await;
    match out {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let mut lines = stdout.lines().map(str::trim).filter(|l| !l.is_empty());
            match (lines.next(), lines.next()) {
                (Some(landing), Some(name)) => format!("captured: {name} — {landing}"),
                (Some(landing), None) => format!("captured — {landing}"),
                _ => "captured".to_string(),
            }
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr);
            format!(
                "the task did not land: {}",
                err.trim()
                    .lines()
                    .next_back()
                    .unwrap_or("mecha tasks add failed")
            )
        }
        Err(e) => format!("the task could not be captured: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slack::actions::ids;
    use serde_json::json;

    fn text(blocks: &[Value]) -> String {
        serde_json::to_string(blocks).unwrap()
    }

    fn row(id: &str, name: &str, status: &str) -> Row {
        Row {
            id: id.into(),
            name: name.into(),
            status: status.into(),
            due: None,
            overdue: false,
            project: None,
            context: None,
            waiting_on: None,
        }
    }

    #[test]
    fn the_board_word_matches_exactly_and_nothing_longer() {
        for word in ["tasks", "Tasks", "  TASKS  "] {
            assert!(is_tasks_command(word), "{word:?} should fire");
        }
        for prose in ["task", "tasks?", "show my tasks", "tasks buy milk", ""] {
            assert!(!is_tasks_command(prose), "{prose:?} should not fire");
        }
    }

    /// The capture matcher takes exactly the sentences that are captures.
    /// The plural is the deliberate asymmetry with `note`/`notes`: `tasks`
    /// is the board, so a plural with a body is ambiguous and falls through
    /// — a capture surface that guesses is worse than a prompt.
    #[test]
    fn the_task_word_captures_and_the_plural_never_does() {
        assert_eq!(
            task_command("task email Dirk about the fMRI slot").as_deref(),
            Some("email Dirk about the fMRI slot"),
            "the plain capture"
        );
        assert_eq!(
            task_command("  Task: book the scanner  ").as_deref(),
            Some("book the scanner"),
            "cased, colon, padded"
        );
        assert_eq!(task_command("task"), None, "the bare word is a prompt");
        assert_eq!(
            task_command("tasks buy milk"),
            None,
            "the plural is the board, never a capture"
        );
        assert_eq!(
            task_command("taskmaster is on tonight"),
            None,
            "a word that merely starts with it is a prompt"
        );
        assert_eq!(
            task_command("add a task to call the vet"),
            None,
            "the word mid-sentence is a prompt"
        );
    }

    /// The listing's contract: an inbox capture offers the commit beside the
    /// finish, everything else offers the finish alone, and every button's
    /// value is the task id and nothing more.
    #[test]
    fn an_inbox_row_offers_next_and_done_and_an_open_row_offers_done_alone() {
        let inbox = row("task-1a2b3c4d", "email Dirk", "inbox");
        let mut next = row("task-9f8e7d6c", "book the scanner", "next");
        next.due = Some("2026-08-25".into());
        next.overdue = true;
        next.context = Some("@lab".into());

        let t = text(&listing_blocks(&[inbox]));
        assert!(t.contains(ids::TASK_NEXT), "{t}");
        assert!(t.contains(ids::TASK_DONE), "{t}");
        assert!(t.contains("task-1a2b3c4d"), "the value is the id: {t}");

        let t = text(&listing_blocks(&[next]));
        assert!(t.contains(ids::TASK_DONE), "{t}");
        assert!(
            !t.contains(ids::TASK_NEXT),
            "a committed task has nothing to commit to: {t}"
        );
        assert!(t.contains("overdue"), "an overdue due date says so: {t}");
        assert!(t.contains("@lab"), "{t}");
    }

    /// The rows come out of the tool's answer in the tool's own order, and
    /// empty fields stay absent rather than rendering as empty columns.
    #[test]
    fn rows_parse_the_tools_answer_and_keep_its_order() {
        let board = json!({
            "today": "2026-08-23",
            "items": [
                { "id": "task-aa", "name": "second", "status": "next",
                  "due_at": "2026-08-24", "overdue": false, "context": "@email" },
                { "id": "task-bb", "name": "first", "status": "inbox",
                  "due_at": null, "project": "" },
            ]
        });
        let rows = rows_from(&board);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "task-aa", "the tool's order is kept");
        assert_eq!(rows[0].context.as_deref(), Some("@email"));
        assert_eq!(rows[1].due, None);
        assert_eq!(rows[1].project, None, "an empty string is an absence");
    }
}
