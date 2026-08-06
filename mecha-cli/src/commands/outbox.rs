//! `mecha outbox` — review, edit, release, or reject staged outbound actions.
//!
//! The human half of the outbox. The agent loop stages outbox-routed tool
//! calls as drafts (see `mecha_core::outbox`); nothing leaves the machine
//! until `send` here executes the real tool — after the human has read
//! exactly what would go out, in the exact arguments that will be used.
//!
//! `edit` before `send` is not just a correction, it is a *measurement*: the
//! item keeps the drafted arguments beside the edited ones, and
//! `mecha reflect` mines `diff(staged, sent)` as a writing-domain lesson.
//! That is why `edit` rewrites `args` and never `args_before`.
//!
//! **All of which is true of a message and false of a publish.** Staging is
//! sink-agnostic; reviewing is not. So `show` and `edit` branch on
//! [`OutboxKind`]: a publish's reviewable object is the rendered page rather
//! than a path and a visibility flag, and editing its arguments would be
//! editing neither the draft nor anything a reader sees.

use anyhow::{bail, Context, Result};
use mecha_core::outbox::{OutboxKind, OutboxStore};
use serde_json::Value;

use crate::{setup, GlobalOpts};

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// List staged items (default).
    List,
    /// Show one item: the exact arguments a release would execute, its
    /// provenance, and the edit diff if there is one.
    Show { id: String },
    /// Open the item's arguments in $EDITOR. What you save is what `send`
    /// executes; the original draft is kept for the learning capture.
    Edit { id: String },
    /// Execute the item's tool call, for real, and mark it sent.
    Send {
        id: String,
        /// Skip the confirmation shown for items drafted in a tainted
        /// conversation.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Refuse an item. It stays on file as the record of the refusal.
    Reject {
        id: String,
        /// Why — recorded on the item for the next reader.
        #[arg(long)]
        reason: Option<String>,
    },
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let store = open_store()?;
    match args.cmd.unwrap_or(Cmd::List) {
        Cmd::List => list(&store),
        Cmd::Show { id } => show(&store, &id),
        Cmd::Edit { id } => edit(&store, &id),
        Cmd::Send { id, yes } => send(global, &store, &id, yes).await,
        Cmd::Reject { id, reason } => reject(&store, &id, reason),
    }
}

/// The store the *agent* stages into is configured in `[outbox] dir`; the
/// review must open the same one, so this resolves through config too.
fn open_store() -> Result<OutboxStore> {
    let cwd = std::env::current_dir().context("cannot determine the working directory")?;
    let cfg = mecha_core::config::Config::load(&cwd)?;
    let root = match cfg.outbox.dir {
        Some(dir) => dir,
        None => OutboxStore::default_root()?,
    };
    OutboxStore::open(root)
}

fn list(store: &OutboxStore) -> Result<()> {
    let items = store.items()?;
    if items.is_empty() {
        println!("outbox empty — calls to [outbox]-routed tools are staged here");
        return Ok(());
    }
    // Pending first — they are the ones waiting on a decision.
    let (pending, resolved): (Vec<_>, Vec<_>) =
        items.into_iter().partition(|i| i.status == "pending");
    for item in pending.iter().chain(resolved.iter()) {
        println!(
            "{}  {:<8} {:<8} {}{}{}",
            item.id,
            item.status,
            item.kind.as_str(),
            item.summary,
            if item.taint.trifecta_armed() {
                "  ⚠ tainted"
            } else {
                ""
            },
            if item.edited() { "  (edited)" } else { "" },
        );
    }
    Ok(())
}

fn show(store: &OutboxStore, id: &str) -> Result<()> {
    let item = store.item(id)?;
    println!(
        "outbox item {} · {} · {} · {}",
        item.id,
        item.kind.as_str(),
        item.tool,
        item.status
    );
    println!("created {}", item.created_at);
    if let Some(session) = &item.session_id {
        println!("drafted by session {session}");
    }
    if item.taint.trifecta_armed() {
        println!(
            "⚠ drafted in a conversation holding private data AND third-party \
             content — review these arguments as possibly an attacker's words, \
             not the assistant's."
        );
    }
    if let Some(resolved) = &item.resolved_at {
        println!(
            "resolved {resolved}{}",
            item.reason
                .as_deref()
                .map(|r| format!(" — {r}"))
                .unwrap_or_default()
        );
    }
    if let Some(error) = &item.error {
        println!("last send attempt failed: {error}");
    }
    match item.kind {
        // For a message the arguments *are* the draft, so print them and the
        // diff that a release would carry.
        OutboxKind::Message => {
            println!("\narguments a release would execute:");
            println!("{}", indent(&pretty(&item.args)));
            if item.edited() {
                println!("\nedited since drafting:");
                println!(
                    "{}",
                    mecha_core::outbox::diff_args(&item.args_before, &item.args)
                );
            }
        }
        // For a publish they are a path and a visibility flag. Reviewing means
        // opening the page, so lead with where it is; the arguments follow as
        // the smaller half rather than as the thing under review.
        OutboxKind::Publish => {
            for (label, path) in local_paths(&item.args) {
                println!("\n{label}: {}", path.display());
                if let Some(entry) = entry_point(&path) {
                    println!("open  {}", entry.display());
                }
                if !path.exists() {
                    println!(
                        "  ⚠ gone — this was rendered into a run's work directory,                          which retention may since have swept. Re-render before                          releasing."
                    );
                }
            }
            println!("\nwhat a release would publish:");
            println!("{}", indent(&pretty(&item.args)));
        }
    }
    if item.status == "pending" {
        match item.kind {
            OutboxKind::Message => println!(
                "\nrelease with `mecha outbox send {}`, or `edit` / `reject` it",
                item.id
            ),
            OutboxKind::Publish => println!(
                "\nrelease with `mecha outbox send {}`, or `reject` it. To change \
                 the content, edit the source and re-render — that stages a new item.",
                item.id
            ),
        }
    }
    Ok(())
}

/// Arguments that name something on this machine, so `show` can point a
/// reviewer at the bytes instead of at a JSON blob.
///
/// Keyed on the argument *name* rather than on the value looking path-shaped:
/// a subject line that happens to start with `/` is not a directory, and
/// guessing would put a wrong "open this" line in front of a human whose whole
/// job here is to check what goes out.
fn local_paths(args: &Value) -> Vec<(&'static str, std::path::PathBuf)> {
    // `bundle` is what the factory's MCP tool actually names its argument —
    // found by wiring the two together, which is the only way a mismatch like
    // this surfaces. The others are kept because a different publishing tool
    // is free to use them, and the cost of an extra key is nothing.
    const KEYS: [(&str, &str); 4] = [
        ("bundle", "rendered bundle"),
        ("bundle_path", "rendered bundle"),
        ("path", "rendered bundle"),
        ("source", "source"),
    ];
    let Some(map) = args.as_object() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, label) in KEYS {
        if let Some(value) = map.get(key).and_then(|v| v.as_str()) {
            out.push((label, std::path::PathBuf::from(value)));
        }
    }
    out
}

/// The file a reviewer should actually open, when the argument named a
/// directory.
fn entry_point(path: &std::path::Path) -> Option<std::path::PathBuf> {
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    ["index.html", "index.md", "README.md"]
        .iter()
        .map(|name| path.join(name))
        .find(|candidate| candidate.is_file())
}

fn edit(store: &OutboxStore, id: &str) -> Result<()> {
    // Resolve before the editor so a bad id fails in milliseconds, but take
    // the lock only *after* the editor exits — holding it across a human's
    // editing session would wedge every concurrent pass that wants it.
    let item = store.item(id)?;
    if item.status != "pending" {
        bail!("outbox item {} is {}, not pending", item.id, item.status);
    }
    // Refused rather than allowed-but-pointless. Editing a publish's arguments
    // rewrites a filesystem path or a visibility flag — it does not edit the
    // draft, and nothing a reader sees changes. Naming the real action is the
    // whole value of the refusal.
    if item.kind == OutboxKind::Publish {
        bail!(
            "outbox item {} is a publish, and its arguments are a path and a \
             visibility flag rather than a draft — editing them would change \
             neither the page nor what a reader sees.\n\
             To change the content: edit the source, re-render, and publish \
             again, which stages a new item. Then `reject {}`.",
            item.id,
            item.id
        );
    }

    let text = crate::editor::edit_text(
        &pretty(&item.args),
        &format!("mecha-outbox-edit-{}.json", item.id),
    )
    .context("the item is unchanged")?;
    // A parse failure keeps the original: better to make the user re-edit
    // than to stage arguments that are not what they meant.
    let args: Value = serde_json::from_str(&text)
        .context("the edited file is not valid JSON; the item is unchanged")?;

    let _lock = store.lock()?;
    let updated = store.update_args(&item.id, args)?;
    if updated.edited() {
        println!(
            "edited; `send` will use the new arguments, and `mecha reflect` \
                  will mine the diff as a writing lesson once sent"
        );
    } else {
        println!("no change");
    }
    Ok(())
}

async fn send(global: &GlobalOpts, store: &OutboxStore, id: &str, yes: bool) -> Result<()> {
    // Held across the whole release, execution included: two concurrent
    // `send`s of the same item must not both pass the pending check and
    // double-send. Staging never takes this lock, so no agent is blocked.
    let _lock = store.lock()?;
    let item = store.item(id)?;
    if item.status != "pending" {
        bail!("outbox item {} is {}, not pending", item.id, item.status);
    }

    if item.taint.trifecta_armed() && !yes {
        println!(
            "⚠ this draft was written in a conversation that held private data \
             AND third-party content. If anything in these arguments was not \
             yours, an attacker may have put it there:\n"
        );
        println!("{}", indent(&pretty(&item.args)));
        print!("\nsend it? [y/N] ");
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut line = String::new();
        // EOF is "no", same as the terminal approver: silence must not send.
        if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0
            || !line.trim().eq_ignore_ascii_case("y")
        {
            println!("not sent; the item stays pending");
            return Ok(());
        }
    }

    // The real tool surface, MCP servers included — the same construction a
    // run uses, minus the provider: releasing a draft needs no model.
    let tools = setup::prepare_tools(global, false).await?;
    let Some(tool) = tools.registry.get(&item.tool) else {
        bail!(
            "tool `{}` is not available in this configuration. Available: {}",
            item.tool,
            tools
                .registry
                .iter()
                .map(|t| t.name())
                .collect::<Vec<_>>()
                .join(", ")
        );
    };

    let ctx = mecha_core::tool::ToolCtx {
        workspace: tools.workspace.clone(),
        shell_timeout: std::time::Duration::from_secs(tools.config.tools.shell_timeout_secs),
        security: tools.config.security.clone(),
        output_budget_bytes: tools.config.tools.output_budget_bytes,
        ..mecha_core::tool::ToolCtx::default()
    };
    let output = match tool.call(item.args.clone(), &ctx).await {
        Ok(out) => out,
        Err(e) => {
            let msg = format!("{e:#}");
            store.record_error(&item.id, &msg)?;
            bail!("send failed: {msg}\nthe item stays pending; retry with `send`");
        }
    };
    if output.is_error {
        store.record_error(&item.id, &output.content)?;
        bail!(
            "the tool reported failure: {}\nthe item stays pending; retry with `send`",
            output.content
        );
    }

    store.resolve(&item.id, "sent", None)?;
    println!("sent via `{}`", item.tool);
    if !output.content.trim().is_empty() {
        println!("{}", indent(output.content.trim()));
    }
    if item.edited() {
        println!(
            "the draft was edited before sending — `mecha reflect` will mine the \
             diff as a writing lesson"
        );
    }
    Ok(())
}

fn reject(store: &OutboxStore, id: &str, reason: Option<String>) -> Result<()> {
    let _lock = store.lock()?;
    let item = store.resolve(id, "rejected", reason)?;
    println!("rejected {}; nothing was sent", item.id);
    Ok(())
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

fn indent(s: &str) -> String {
    s.lines()
        .map(|l| format!("  {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn the_diff_names_changed_lines_only() {
        let before = json!({"to": "a@example.com", "body": "hi"});
        let after = json!({"to": "a@example.com", "body": "hello"});
        let d = mecha_core::outbox::diff_args(&before, &after);
        let removed = d.lines().find(|l| l.trim_start().starts_with('-')).unwrap();
        let added = d.lines().find(|l| l.trim_start().starts_with('+')).unwrap();
        assert!(removed.contains(r#""hi""#), "{d}");
        assert!(added.contains(r#""hello""#), "{d}");
        assert!(!d.contains("example.com"), "unchanged lines stay out: {d}");

        let same = mecha_core::outbox::diff_args(&before, &before);
        assert!(same.contains("no textual change"), "{same}");
    }
}
