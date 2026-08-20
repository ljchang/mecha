//! `mecha tasks` — the GTD board in the knowledge graph, from the terminal.
//!
//! The command line does everything first and the `/tasks` modal drives it,
//! on the front door's rule: one implementation per verb, and no way for a UI
//! to do something the terminal cannot.
//!
//! **The board is reached the same way the model reaches it — through the MCP
//! tool surface.** `mecha-cli` has no dependency on the graph and does not
//! gain one here: `kg_task_list` already answers in JSON, so this driver reads
//! the same bytes the loop would, and a deployment that renames the server or
//! turns `prefix_tools` on keeps working because the lookup matches on the
//! suffix. Reaching past the tools into the SQLite file would be a second
//! implementation of a schema that lives in another repository.
//!
//! No approver and no interlock, deliberately, exactly as `mecha mail task`
//! does it: the person at the keyboard *is* the authority a tool approval
//! exists to consult, and the board reaches nobody — `kg_task_*` carries
//! `openWorldHint: false`, so none of it is a send.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::setup::{find_tool, tool_ctx};
use crate::{setup, GlobalOpts};

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// The board: actionable statuses first, then by due date (default).
    List {
        /// Also show done and dropped tasks — the history.
        #[arg(long)]
        closed: bool,
        /// Machine output: the tool's own JSON, which is what the modal reads.
        #[arg(long)]
        json: bool,
    },
    /// Capture a task. Lands in `inbox` — captured, not yet committed to.
    Add {
        /// The task, phrased as an action. Trailing words are joined, so it
        /// needs no quoting.
        #[arg(required = true, num_args = 1..)]
        name: Vec<String>,
        /// YYYY-MM-DD, `today`, `tomorrow`, or `+Nd`.
        #[arg(long)]
        due: Option<String>,
        /// Parent project — must name a node the graph already has.
        #[arg(long)]
        project: Option<String>,
        /// GTD context tag, e.g. `@email`, `@lab`.
        #[arg(long)]
        context: Option<String>,
    },
    /// Move a task through its lifecycle, or edit its scheduling.
    ///
    /// Omitting a field leaves it untouched; passing an empty string clears
    /// it. That distinction is the tool's, and it is passed through rather
    /// than reinterpreted here — a driver that turned "unset" into "clear"
    /// would silently wipe a due date every time somebody changed a status.
    Set {
        /// The task's node id, e.g. `task-1a2b3c4d`, from `tasks list`.
        task: String,
        /// next | inbox | scheduled | waiting | done | dropped.
        #[arg(long)]
        status: Option<String>,
        /// New due date; `""` clears it.
        #[arg(long)]
        due: Option<String>,
        /// Hide until this date; `""` clears it.
        #[arg(long)]
        defer: Option<String>,
        /// New context tag; `""` clears it.
        #[arg(long)]
        context: Option<String>,
    },
}

pub async fn run(global: &GlobalOpts, args: Args) -> Result<()> {
    match args.cmd.unwrap_or(Cmd::List {
        closed: false,
        json: false,
    }) {
        Cmd::List { closed, json } => list(global, closed, json).await,
        Cmd::Add {
            name,
            due,
            project,
            context,
        } => add(global, &name.join(" "), due, project, context).await,
        Cmd::Set {
            task,
            status,
            due,
            defer,
            context,
        } => set(global, &task, status, due, defer, context).await,
    }
}

/// Call one `kg_task_*` tool and return its parsed answer.
///
/// The absence of the graph is a *named* condition rather than a panic or an
/// empty board: a machine with no `[[mcp]]` graph server has no tasks to show
/// and needs to be told which of those two it is.
async fn call(global: &GlobalOpts, tool: &str, args: Value) -> Result<Value> {
    let prepared = setup::prepare_tools(global, false).await?;
    let found = find_tool(&prepared.registry, tool).with_context(|| {
        format!("no knowledge-graph server in this configuration — `{tool}` is not on the tool surface. Is `[[mcp]]` enabled?")
    })?;
    let out = found.call(args, &tool_ctx(&prepared)).await?;
    if out.is_error {
        bail!("{}: {}", tool, out.content.trim());
    }
    serde_json::from_str(&out.content)
        .with_context(|| format!("{tool} did not answer with JSON: {}", out.content))
}

async fn list(global: &GlobalOpts, closed: bool, as_json: bool) -> Result<()> {
    let board = call(global, "kg_task_list", json!({ "include_closed": closed })).await?;
    if as_json {
        println!("{board}");
        return Ok(());
    }

    let today = board["today"].as_str().unwrap_or_default();
    let items = board["items"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if items.is_empty() {
        println!("nothing on the board — `mecha tasks add <what>` captures one");
        return Ok(());
    }

    for t in items {
        let due = match t["due_at"].as_str() {
            Some(d) if t["overdue"].as_bool().unwrap_or(false) => format!("{d} overdue"),
            Some(d) => d.to_string(),
            None => "—".into(),
        };
        println!(
            "{:<10}  {:<18}  {:<52}  {}",
            t["status"].as_str().unwrap_or("?"),
            due,
            t["name"].as_str().unwrap_or(""),
            t["id"].as_str().unwrap_or(""),
        );
        // The tail exists only when it says something. A row of empty columns
        // reads as data about a task that has none.
        let tail: Vec<String> = [
            ("project", "project"),
            ("context", "context"),
            ("waiting_on", "waiting on"),
        ]
        .iter()
        .filter_map(|(key, label)| {
            t[*key]
                .as_str()
                .filter(|v| !v.is_empty())
                .map(|v| format!("{label} {v}"))
        })
        .collect();
        if !tail.is_empty() {
            println!("{:<10}  {}", "", tail.join(" · "));
        }
    }
    println!("\n{} task(s) · today is {today}", items.len());
    Ok(())
}

async fn add(
    global: &GlobalOpts,
    name: &str,
    due: Option<String>,
    project: Option<String>,
    context: Option<String>,
) -> Result<()> {
    let mut args = json!({ "name": name });
    for (key, value) in [("due", due), ("project", project), ("context", context)] {
        if let Some(v) = value {
            args[key] = json!(v);
        }
    }
    let out = call(global, "kg_task_create", args).await?;
    println!(
        "{}  {}",
        out["id"].as_str().unwrap_or("created"),
        // The tool resolves `tomorrow` and `+3d` itself, so report what it
        // stored rather than what was typed — the two differ on purpose.
        match out["due_at"].as_str() {
            Some(d) => format!("due {d}"),
            None => "no due date".into(),
        }
    );
    println!("{name}");
    Ok(())
}

async fn set(
    global: &GlobalOpts,
    task: &str,
    status: Option<String>,
    due: Option<String>,
    defer: Option<String>,
    context: Option<String>,
) -> Result<()> {
    let mut args = json!({ "task": task });
    for (key, value) in [
        ("status", status),
        ("due", due),
        ("defer", defer),
        ("context", context),
    ] {
        if let Some(v) = value {
            args[key] = json!(v);
        }
    }
    if args.as_object().is_some_and(|o| o.len() == 1) {
        bail!("nothing to change — pass at least one of --status, --due, --defer, --context");
    }
    let out = call(global, "kg_task_update", args).await?;
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
