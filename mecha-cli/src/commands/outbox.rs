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
use mecha_core::outbox::{DraftView, OutboxItem, OutboxKind, OutboxLock, OutboxStore};
use mecha_core::outbox_source::SourceRead;
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::{setup, GlobalOpts};

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

/// Which items a command acts on.
///
/// Shared by `send` and `reject` so "everything from the overnight triage" is
/// spelled the same way for both, and so the rules that decide what a bare
/// `--all` means live in exactly one place ([`select`]).
#[derive(clap::Args, Debug, Default, Clone)]
pub struct Selection {
    /// Ids, or unique prefixes of them. Several is fine.
    pub ids: Vec<String>,
    /// Every pending item, subject to the filters below.
    #[arg(long)]
    pub all: bool,
    /// Only `message` or only `publish`.
    #[arg(long)]
    pub kind: Option<String>,
    /// Only items staged by a tool whose name contains this.
    ///
    /// `--via`, not `--tool`: the global `--tool` narrows the tool *surface*,
    /// and on `outbox send` that is the registry the release executes through.
    /// Two flags one letter apart meaning "filter the queue" and "change what
    /// can run" is a collision clap caught and a reader would not have.
    #[arg(long)]
    pub via: Option<String>,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// List staged items, grouped by kind (default).
    List {
        /// Only `message` or only `publish`.
        #[arg(long)]
        kind: Option<String>,
        /// Only items staged by a tool whose name contains this.
        #[arg(long)]
        via: Option<String>,
    },
    /// Show one item: the draft as it would be read, its provenance, and the
    /// edit diff if there is one.
    Show {
        id: String,
        /// The exact arguments as JSON, instead of the readable draft.
        #[arg(long)]
        json: bool,
    },
    /// Open the draft's text in $EDITOR. What you save is what `send` sends;
    /// the original draft is kept for the learning capture.
    Edit {
        id: String,
        /// Edit the whole arguments as JSON instead of just the prose. What
        /// this always did — kept for the drafts that are not prose, and for
        /// changing a recipient.
        #[arg(long)]
        json: bool,
    },
    /// Walk the pending items one at a time, deciding each.
    ///
    /// The morning routine: one invocation, one decision per draft, and the
    /// draft in front of you when you make it. This is what "batch review"
    /// should mean — a queue of nine costing one command rather than nine, with
    /// the reading intact.
    Review {
        #[command(flatten)]
        selection: Selection,
    },
    /// Execute the tool call for real, and mark it sent.
    Send {
        #[command(flatten)]
        selection: Selection,
        /// Skip the confirmation. Also skips the one shown for drafts written
        /// in a tainted conversation, which is the reason it is not the
        /// default.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Refuse items. They stay on file as the record of the refusal.
    Reject {
        #[command(flatten)]
        selection: Selection,
        /// Why — recorded on each item for the next reader.
        #[arg(long)]
        reason: Option<String>,
    },
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let store = open_store()?;
    match args.cmd.unwrap_or(Cmd::List {
        kind: None,
        via: None,
    }) {
        Cmd::List { kind, via } => list(&store, kind.as_deref(), via.as_deref()),
        Cmd::Show { id, json } => show(&store, &id, json),
        Cmd::Edit { id, json } => edit(&store, &id, json),
        Cmd::Review { selection } => review(global, &store, &selection).await,
        Cmd::Send { selection, yes } => send(global, &store, &selection, yes).await,
        Cmd::Reject { selection, reason } => reject(&store, &selection, reason),
    }
}

/// The store the *agent* stages into is configured in `[outbox] dir`; the
/// review must open the same one, so this resolves through config too.
///
/// `pub(crate)` because the TUI's /outbox modal is a second review surface
/// over the same store, and two resolutions of "which store" would eventually
/// disagree.
pub(crate) fn open_store() -> Result<OutboxStore> {
    let cwd = std::env::current_dir().context("cannot determine the working directory")?;
    let cfg = mecha_core::config::Config::load(&cwd)?;
    let root = match cfg.outbox.dir {
        Some(dir) => dir,
        None => OutboxStore::default_root()?,
    };
    OutboxStore::open(root)
}

/// `--kind` as a kind, or an error naming the two that exist.
///
/// Shared by `select` and `list` so a typo cannot mean "error" on one surface
/// and "empty queue" on the other. It used to: `list` reused `select` for its
/// filter rules and then `unwrap_or_default()`, which swallowed exactly this.
fn parse_kind(kind: Option<&str>) -> Result<Option<OutboxKind>> {
    match kind {
        None => Ok(None),
        Some("message") => Ok(Some(OutboxKind::Message)),
        Some("publish") => Ok(Some(OutboxKind::Publish)),
        Some(other) => bail!("`{other}` is not a kind (message | publish)"),
    }
}

/// The items a [`Selection`] names, in store order.
///
/// Pure, over a list somebody else read, because the rules here are the ones
/// worth testing and none of them need a filesystem. Three of them are
/// decisions rather than mechanics:
///
/// - **A selection that names nothing is an error, never "everything".** The
///   most expensive mistake this surface can make is releasing drafts nobody
///   chose, so the empty case fails loudly instead of defaulting wide.
/// - **`--all` means every *pending* item**, subject to the filters. Already
///   resolved items are not candidates: re-sending something marked sent is not
///   a thing anyone means, and `resolve` would refuse it anyway.
/// - **A filter that matches nothing is an error too.** `--via mail__send`
///   with a typo silently acting on zero items reads exactly like an empty
///   queue, and the two want opposite reactions.
fn select(items: Vec<OutboxItem>, selection: &Selection) -> Result<Vec<OutboxItem>> {
    let kind = parse_kind(selection.kind.as_deref())?;
    let matches_filters = |item: &OutboxItem| {
        kind.is_none_or(|k| item.kind == k)
            && selection
                .via
                .as_deref()
                .is_none_or(|t| item.tool.contains(t))
    };

    if !selection.ids.is_empty() {
        let mut out = Vec::new();
        for id in &selection.ids {
            let matched: Vec<&OutboxItem> = items.iter().filter(|i| i.id.starts_with(id)).collect();
            match matched.len() {
                0 => bail!("no outbox item matching `{id}`"),
                1 => {
                    let item = matched[0].clone();
                    if !matches_filters(&item) {
                        bail!("`{id}` does not match the filters given alongside it");
                    }
                    // Naming the same item twice would send it twice; the
                    // second would fail on the pending check, but silently
                    // depending on that is not the same as not doing it.
                    if !out
                        .iter()
                        .any(|existing: &OutboxItem| existing.id == item.id)
                    {
                        out.push(item);
                    }
                }
                n => bail!(
                    "`{id}` matches {n} outbox items: {}",
                    matched
                        .iter()
                        .map(|i| i.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        }
        return Ok(out);
    }

    if !selection.all {
        bail!(
            "name the items, or pass --all (optionally with --kind or --via). \
             A command with no selection acts on nothing rather than on everything."
        );
    }
    let chosen: Vec<OutboxItem> = items
        .into_iter()
        .filter(|i| i.status == "pending" && matches_filters(i))
        .collect();
    if chosen.is_empty() {
        bail!("nothing pending matches that selection");
    }
    Ok(chosen)
}

fn list(store: &OutboxStore, kind: Option<&str>, via: Option<&str>) -> Result<()> {
    // Before anything is read or printed. A bad `--kind` is a typo, and the one
    // thing it must not produce is a clean empty listing — that reads as "the
    // queue is clear" while nine drafts sit in it, which is the same failure
    // `a_filter_that_matches_nothing_is_an_error` was written against.
    let kind = parse_kind(kind)?;

    let items = store.items()?;
    if items.is_empty() {
        println!("outbox empty — calls to [outbox]-routed tools are staged here");
        return Ok(());
    }
    let filter = Selection {
        all: true,
        kind: kind.map(|k| k.as_str().to_string()),
        via: via.map(String::from),
        ..Selection::default()
    };
    // Reuses `select` only for its filter rules, then falls back to the whole
    // list: `list` shows resolved items too, and an empty *pending* set is an
    // ordinary morning rather than an error.
    let pending = select(items.clone(), &filter).unwrap_or_default();
    let resolved: Vec<OutboxItem> = items
        .into_iter()
        .filter(|i| i.status != "pending")
        .filter(|i| kind.is_none_or(|k| i.kind == k) && via.is_none_or(|t| i.tool.contains(t)))
        .collect();

    // Grouped by kind, because the two are reviewed differently — a message is
    // read as prose, a publish is opened in a browser — and a morning queue
    // mixing them makes you switch modes item by item.
    for kind in [OutboxKind::Message, OutboxKind::Publish] {
        let group: Vec<&OutboxItem> = pending.iter().filter(|i| i.kind == kind).collect();
        if group.is_empty() {
            continue;
        }
        println!("{} pending {}(s):", group.len(), kind.as_str());
        for item in group {
            println!("  {}", line(item));
        }
    }
    if pending.is_empty() {
        println!("nothing pending");
    } else {
        println!(
            "\nreview them one at a time with `mecha outbox review --all`{}",
            match (kind, via) {
                (Some(k), _) => format!(" --kind {}", k.as_str()),
                (_, Some(t)) => format!(" --via {t}"),
                _ => String::new(),
            }
        );
    }
    if !resolved.is_empty() {
        println!("\n{} resolved:", resolved.len());
        for item in &resolved {
            println!("  {}", line(item));
        }
    }
    Ok(())
}

fn line(item: &OutboxItem) -> String {
    format!(
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
    )
}

/// Print one item for a person.
///
/// **A message leads with the message.** The arguments *are* the draft, which
/// is true and was taken to mean "print the JSON" — so a reviewer deciding
/// whether to send a letter in their own name read
/// `{"body_markdown": "Dear Dirk,\n\nThank…"}` and had to decode escape
/// sequences to find out what it said. The reviewable object is the letter,
/// exactly as a publish's is the rendered page; `--json` still prints the
/// exact bytes, so nothing is unavailable, only reordered by what the reader
/// is actually deciding.
fn show(store: &OutboxStore, id: &str, json: bool) -> Result<()> {
    let item = store.item(id)?;
    // Above everything, both of them: a warning that arrives after the draft
    // arrives after the decision.
    if item.taint.trifecta_armed() {
        println!(
            "⚠ drafted in a conversation holding private data AND third-party \
             content — read this as possibly an attacker's words, not the \
             assistant's."
        );
        println!();
    }
    if let Some(error) = &item.error {
        println!("last send attempt failed: {error}\n");
    }
    match item.kind {
        OutboxKind::Message if json => {
            println!("arguments a release would execute:");
            println!("{}", indent(&pretty(&item.args)));
        }
        OutboxKind::Message => {
            let view = DraftView::of(&item.args);
            for (name, value) in &view.headers {
                println!("{name:<9} {value}");
            }
            if let Some(body) = &view.body {
                println!("\n{body}");
            }
            if !view.other.is_empty() {
                println!("\nother arguments:");
                for (name, value) in &view.other {
                    println!("  {name:<9} {value}");
                }
            }
            if item.edited() {
                println!("\nedited since drafting:");
                println!(
                    "{}",
                    mecha_core::outbox::diff_args(&item.args_before, &item.args)
                );
            }
            for read in source_reads(&item) {
                println!("\n{}", source_heading(&read));
                println!("{}", indent(&read.text));
            }
        }
        // For a publish they are a path and a visibility flag. Reviewing means
        // opening the page, so lead with where it is; the arguments follow as
        // the smaller half rather than as the thing under review.
        OutboxKind::Publish => {
            for (label, path) in local_paths(&item.args, item.workspace.as_deref()) {
                println!("\n{label}: {}", path.display());
                if let Some(entry) = entry_point(&path) {
                    println!("open  {}", entry.display());
                }
                if !path.exists() {
                    println!(
                        "  ⚠ gone — this was rendered into a run's work directory, \
                         which retention may since have swept. Re-render before \
                         releasing."
                    );
                }
            }
            println!("\nwhat a release would publish:");
            println!("{}", indent(&pretty(&item.args)));
        }
    }

    // Where it came from: true, kept, and not the question being answered.
    println!(
        "\noutbox item {} · {} · {} · {}",
        item.id,
        item.kind.as_str(),
        item.tool,
        item.status
    );
    println!("created {}", item.created_at);
    if let Some(session) = &item.session_id {
        println!("drafted by session {session}");
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
    if item.kind == OutboxKind::Message && !json {
        println!(
            "`mecha outbox show {} --json` prints the exact arguments",
            item.id
        );
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

/// What the draft is answering, or nothing when it cannot be found.
///
/// Best-effort at the surface as well as inside: a review must still render
/// when the session store is unreadable, so a failure here is an absent
/// section rather than a failed command. The absence is honest — a draft
/// composing a *new* message answers nothing, and there is no way to tell
/// that apart from a swept transcript without claiming to know more than we
/// do.
pub(crate) fn source_reads(item: &OutboxItem) -> Vec<SourceRead> {
    let Ok(dir) = mecha_core::session::Session::default_dir() else {
        return Vec::new();
    };
    mecha_core::outbox_source::for_item(item, &dir)
}

/// The heading over a quoted source read.
///
/// It says three things, and each is needed: that this is *not* the draft,
/// that it came from outside this machine, and which tool fetched it. A
/// quoted block with no heading reads as more of the letter — which for text
/// an attacker may have written is the one impression this must never leave.
pub(crate) fn source_heading(read: &SourceRead) -> String {
    format!(
        "replying to — third-party content via {} ({}), not part of your draft:",
        read.tool,
        read.keys.join(", ")
    )
}

/// Arguments that name something on this machine, so `show` can point a
/// reviewer at the bytes instead of at a JSON blob.
///
/// Keyed on the argument *name* rather than on the value looking path-shaped:
/// a subject line that happens to start with `/` is not a directory, and
/// guessing would put a wrong "open this" line in front of a human whose whole
/// job here is to check what goes out.
///
/// **Relative paths resolve against the jail the item was drafted under**, for
/// the same reason a release does (see [`OutboxItem::workspace`]) — and it is
/// the display that gets this wrong most visibly. The agent said
/// `{"spec": "retro-spec.toml"}` inside its work directory; the reviewer is
/// standing somewhere else entirely. Without the jail, `show` reports a file
/// that is right there as "⚠ gone", and the symmetric case is worse: a
/// same-named file beside the reviewer gets displayed, and offered to open, as
/// though it were the draft's source. `None` — an item staged before the field
/// existed — falls back to the reviewer's directory, which is what those items
/// always did.
pub(crate) fn local_paths(
    args: &Value,
    workspace: Option<&Path>,
) -> Vec<(&'static str, std::path::PathBuf)> {
    // `bundle` is what the factory's MCP tool actually names its argument —
    // found by wiring the two together, which is the only way a mismatch like
    // this surfaces. The others are kept because a different publishing tool
    // is free to use them, and the cost of an extra key is nothing.
    // `bundle` and `spec` are what the factory's MCP tools actually name their
    // arguments — both found by wiring the two together, which is the only way
    // a mismatch like this surfaces. A poll and a request type have no rendered
    // directory to open; what a reviewer needs to read is the file that defines
    // what is about to become public, which is exactly these.
    const KEYS: [(&str, &str); 6] = [
        ("bundle", "rendered bundle"),
        ("bundle_path", "rendered bundle"),
        ("path", "rendered bundle"),
        ("source", "source"),
        ("spec", "poll spec"),
        ("manifest", "form manifest"),
    ];
    let Some(map) = args.as_object() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, label) in KEYS {
        if let Some(value) = map.get(key).and_then(|v| v.as_str()) {
            let path = std::path::PathBuf::from(value);
            let resolved = match (path.is_absolute(), workspace) {
                (false, Some(jail)) => jail.join(&path),
                _ => path,
            };
            out.push((label, resolved));
        }
    }
    out
}

/// The file a reviewer should actually open, when the argument named a
/// directory.
pub(crate) fn entry_point(path: &std::path::Path) -> Option<std::path::PathBuf> {
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    ["index.html", "index.md", "README.md"]
        .iter()
        .map(|name| path.join(name))
        .find(|candidate| candidate.is_file())
}

/// Open the draft in `$EDITOR`.
///
/// **The prose by default, the JSON on `--json`.** Editing a letter inside a
/// JSON string literal means typing `\n` for a paragraph break and escaping
/// every quote, in a file where one slip is a parse error that throws the
/// whole edit away — for the one action here whose entire purpose is changing
/// the words. So the scratch file is the words: a `.md` file holding the body
/// and nothing else, written back to the key it came from
/// ([`mecha_core::outbox::with_body`], which is where that decision lives so
/// it is made once).
///
/// The learning capture is untouched by this: `args_before` still holds the
/// draft, `args` still holds what will be sent, and `mecha reflect` still
/// mines the difference. What changed is only which bytes a human is shown.
///
/// A draft with no prose — a calendar RSVP, a reaction — falls back to the
/// arguments rather than opening an empty file, and says so.
fn edit(store: &OutboxStore, id: &str, json: bool) -> Result<()> {
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

    let body = if json {
        None
    } else {
        mecha_core::outbox::DraftView::of(&item.args).body
    };
    let args = match body {
        Some(body) => {
            // The original beneath the draft, so a reply can be written
            // against the thing it answers rather than from memory. Cut back
            // off below; see `strip_reference` for why a missing marker is a
            // refusal.
            let reads = source_reads(&item);
            let seeded = mecha_core::outbox_source::with_reference(&body, &reads);
            let text =
                crate::editor::edit_text(&seeded, &format!("mecha-outbox-edit-{}.md", item.id))
                    .context("the item is unchanged")?;
            let text = if reads.is_empty() {
                // Trailing newline: an editor adds one, and it is not an edit.
                text.strip_suffix('\n').unwrap_or(&text).to_string()
            } else {
                mecha_core::outbox_source::strip_reference(&text)
                    .context(
                        "the reference marker is gone from the edited file, so where the \
                         reply ends cannot be told from where the quoted original begins — \
                         and the two ways to guess are mailing the original back or \
                         truncating your letter. The item is unchanged; run `edit` again \
                         and leave the marker line in place, or use `--json`.",
                    )?
                    .to_string()
            };
            mecha_core::outbox::with_body(&item.args, &text)
                .context("the draft's body field went missing; the item is unchanged")?
        }
        None => {
            if !json {
                println!("no prose in this draft — opening its arguments instead");
            }
            let text = crate::editor::edit_text(
                &pretty(&item.args),
                &format!("mecha-outbox-edit-{}.json", item.id),
            )
            .context("the item is unchanged")?;
            // A parse failure keeps the original: better to make the user
            // re-edit than to stage arguments that are not what they meant.
            serde_json::from_str(&text)
                .context("the edited file is not valid JSON; the item is unchanged")?
        }
    };

    let _lock = store.lock()?;
    let updated = store.update_args(&item.id, args)?;
    if updated.edited() {
        println!(
            "edited; `send` will use the new text, and `mecha reflect` \
                  will mine the diff as a writing lesson once sent"
        );
    } else {
        println!("no change");
    }
    Ok(())
}

/// Re-read what the caller reviewed, and refuse anything not still pending.
///
/// The caller holds the store lock; this is the check-and-act that must happen
/// inside it. `send` reads its whole batch and executes straight away, so its
/// own pre-loop check is nearly enough — but `review` reads an item, shows it
/// to a human, and *waits*, and a minute at a prompt is long enough for another
/// terminal to send the same draft. Checking before the wait and executing
/// after it is a check-then-act race whose losing side is a stranger's email
/// delivered twice.
///
/// `store.resolve` refusing a non-pending item is too late to help: by the time
/// it runs, the tool has already gone out for real. Returning the freshly-read
/// item rather than the caller's copy is the other half — what executes and
/// what gets recorded are then the same object.
fn claim_for_release(store: &OutboxStore, reviewed: &OutboxItem) -> Result<OutboxItem> {
    let current = store.item(&reviewed.id)?;
    anyhow::ensure!(
        current.status == "pending",
        "outbox item {} is {}, not pending — it was resolved while you were \
         deciding, so nothing was sent",
        current.id,
        current.status
    );
    Ok(current)
}

/// The tool surface a release executes through.
///
/// Built **once per batch**, which is most of why batching is worth having at
/// all: this construction starts every configured MCP server, so nine
/// invocations of `send` were nine startups of the mail server to deliver nine
/// replies that were already written.
///
/// One per *workspace*, though (see [`Surfaces`]), because a staged call's
/// paths are relative to the jail it was drafted under and not to wherever the
/// reviewer is standing.
struct Surface {
    tools: setup::PreparedTools,
    ctx: mecha_core::tool::ToolCtx,
}

impl Surface {
    async fn build(global: &GlobalOpts, workspace: Option<&Path>) -> Result<Surface> {
        // The same construction a run uses, minus the provider: releasing a
        // draft needs no model.
        //
        // `workspace` is the item's, so the release is jailed exactly as the
        // drafting run was. `None` — an item staged before the field existed —
        // falls through to the reviewer's own workspace, which is the old
        // behaviour and the only thing there is to fall back to.
        let global = match workspace {
            Some(dir) => GlobalOpts {
                workspace: Some(dir.to_path_buf()),
                ..global.clone()
            },
            None => global.clone(),
        };
        let tools = setup::prepare_tools(&global, false).await?;
        let ctx = mecha_core::tool::ToolCtx {
            workspace: tools.workspace.clone(),
            shell_timeout: std::time::Duration::from_secs(tools.config.tools.shell_timeout_secs),
            security: tools.config.security.clone(),
            // No model reads a release's output, so the window-derived
            // budget doesn't apply; the ceiling is fine.
            output_budget_bytes: tools.config.tools.resolved_output_budget(None),
            ..mecha_core::tool::ToolCtx::default()
        };
        Ok(Surface { tools, ctx })
    }

    /// Execute one item, resolve it, and say what happened.
    ///
    /// Returns `Err` for a failure the *item* survives — it stays pending with
    /// the error recorded — so a batch reports it and carries on to the next
    /// draft rather than abandoning eight good ones over one bad address.
    ///
    /// **The caller must hold the store lock**, because the pending check is
    /// [`claim_for_release`] and it has to happen inside that lock.
    async fn release(&self, store: &OutboxStore, item: &OutboxItem) -> Result<String> {
        let item = &claim_for_release(store, item)?;
        let Some(tool) = self.tools.registry.get(&item.tool) else {
            // Recorded like every other release failure: this one dies before
            // any tool runs, and a failure that only reaches stderr is
            // invisible to `list`/`show` and to the TUI's detached watch.
            let msg = format!(
                "tool `{}` is not available in this configuration. Available: {}",
                item.tool,
                self.tools
                    .registry
                    .iter()
                    .map(|t| t.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            store.record_error(&item.id, &msg)?;
            bail!("{msg}");
        };
        let output = match tool.call(item.args.clone(), &self.ctx).await {
            Ok(out) => out,
            Err(e) => {
                let msg = format!("{e:#}");
                store.record_error(&item.id, &msg)?;
                bail!("{msg}");
            }
        };
        if output.is_error {
            store.record_error(&item.id, &output.content)?;
            bail!("the tool reported failure: {}", output.content);
        }
        store.resolve(&item.id, "sent", None)?;
        Ok(output.content.trim().to_string())
    }
}

/// The surfaces a batch needs, one per distinct workspace, built on first use.
///
/// A batch almost always shares one workspace — an overnight triage staging
/// nine replies drafted them in one run — so this is one construction and one
/// set of MCP startups, exactly as before. It stops being one only when the
/// batch genuinely spans jails, and then two surfaces is the correct cost of
/// releasing each call under the root it was written against.
#[derive(Default)]
struct Surfaces {
    by_workspace: Vec<(Option<PathBuf>, Surface)>,
}

impl Surfaces {
    async fn for_item(&mut self, global: &GlobalOpts, item: &OutboxItem) -> Result<&Surface> {
        let key = item.workspace.clone();
        if let Some(i) = self.by_workspace.iter().position(|(k, _)| *k == key) {
            return Ok(&self.by_workspace[i].1);
        }
        let surface = Surface::build(global, key.as_deref()).await?;
        self.by_workspace.push((key, surface));
        Ok(&self.by_workspace.last().unwrap().1)
    }
}

/// Put a release failure on the item before reporting it.
///
/// Everything on the release path must land in the store, the surface build
/// included: `Surface::build` runs *before* `release`, so its failures — a bad
/// workspace, an MCP server that won't start — used to reach only stderr, and
/// the TUI spawns releases detached with stderr closed. The result was an item
/// sitting pending with `error: null` while the watch said "still releasing"
/// forever. `record_error` keeps the item pending — the draft is still good
/// and the next `send` retries — with the reason where every review surface
/// can display it.
///
/// **The lock guard is a parameter on purpose.** `record_error` is a
/// read-then-write of the item with no flock of its own, and the store lock's
/// contract is "taken before reading the state acted on". A lockless call
/// here raced a concurrent `mecha outbox send`: the other terminal marks the
/// item sent inside this helper's read/write gap, the unlocked write restores
/// a stale `pending`, and a later send double-delivers. Demanding the guard
/// makes an unlocked call a compile error rather than a rule each call site
/// must remember — the guard is never read, only proven held.
fn record_release_failure(
    store: &OutboxStore,
    _lock: &OutboxLock,
    id: &str,
    err: anyhow::Error,
) -> anyhow::Error {
    match store.record_error(id, &format!("{err:#}")) {
        Ok(()) => err,
        Err(record) => {
            anyhow::anyhow!("{err:#} (and recording the failure also failed: {record:#})")
        }
    }
}

/// One y/N question, where EOF is "no".
///
/// The same rule the terminal approver follows and the reason it matters here:
/// a review surface reached by a script with no stdin must not release
/// anything. Silence is not consent.
fn confirm(question: &str) -> Result<bool> {
    use std::io::Write;
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
        println!();
        return Ok(false);
    }
    Ok(line.trim().eq_ignore_ascii_case("y"))
}

async fn send(
    global: &GlobalOpts,
    store: &OutboxStore,
    selection: &Selection,
    yes: bool,
) -> Result<()> {
    // Held across the whole batch, execution included: two concurrent `send`s
    // of the same item must not both pass the pending check and double-send.
    // Staging never takes this lock, so no agent is blocked by a review.
    let lock = store.lock()?;
    let items = select(store.items()?, selection)?;

    for item in &items {
        if item.status != "pending" {
            bail!("outbox item {} is {}, not pending", item.id, item.status);
        }
    }
    let tainted: Vec<&OutboxItem> = items.iter().filter(|i| i.taint.trifecta_armed()).collect();

    // A batch is confirmed as a batch, and a single untainted item is not —
    // which keeps `send <id>` exactly as direct as it was. What must never
    // become quiet is the tainted case: those arguments are printed in full,
    // however many there are, because "approve all" without reading is the
    // failure mode this whole surface exists to prevent.
    if !yes && (items.len() > 1 || !tainted.is_empty()) {
        if !tainted.is_empty() {
            println!(
                "⚠ {} of these drafts {} written in a conversation holding private \
                 data AND third-party content. If anything in them was not yours, \
                 an attacker may have put it there:\n",
                tainted.len(),
                if tainted.len() == 1 { "was" } else { "were" }
            );
            for item in &tainted {
                println!("{} · {}", item.id, item.summary);
                println!("{}\n", indent(&pretty(&item.args)));
            }
        }
        if items.len() > 1 {
            println!("about to send {} item(s):", items.len());
            for item in &items {
                println!("  {}", line(item));
            }
        }
        if !confirm(&format!(
            "\nsend {}?",
            if items.len() == 1 {
                "it".to_string()
            } else {
                format!("all {}", items.len())
            }
        ))? {
            println!("nothing sent; the items stay pending");
            return Ok(());
        }
    }

    let mut surfaces = Surfaces::default();
    let (mut sent, mut failed) = (0usize, 0usize);
    for item in &items {
        let release = match surfaces.for_item(global, item).await {
            Ok(surface) => surface.release(store, item).await,
            // A surface that cannot be built is a failure of this item's
            // release, so it is recorded like one — and only this item's:
            // the next draft may name a workspace that builds fine.
            Err(e) => Err(record_release_failure(store, &lock, &item.id, e)),
        };
        match release {
            Ok(output) => {
                sent += 1;
                println!("sent {} via `{}`", item.id, item.tool);
                if !output.is_empty() {
                    println!("{}", indent(&output));
                }
                if item.edited() {
                    println!(
                        "  the draft was edited before sending — `mecha reflect` will \
                         mine the diff as a writing lesson"
                    );
                }
            }
            Err(e) => {
                failed += 1;
                // Reported and survived: the item stays pending with the error
                // on it, so a batch is never all-or-nothing.
                eprintln!("failed {}: {e:#}", item.id);
            }
        }
    }
    if items.len() > 1 || failed > 0 {
        println!("\n{sent} sent, {failed} failed and still pending");
    }
    // The exit code has to say it: a batch where one send failed and eight
    // worked is not a success, and a script that fans out on this needs to
    // know without parsing prose.
    if failed > 0 {
        bail!("{failed} of {} item(s) did not send", items.len());
    }
    Ok(())
}

/// Walk the pending items, one decision at a time.
async fn review(global: &GlobalOpts, store: &OutboxStore, selection: &Selection) -> Result<()> {
    // Deliberately *not* holding the store lock across the loop: this waits on
    // a human, and `edit` shells out to `$EDITOR`. Each action takes the lock
    // for itself, the same rule `edit` already followed.
    let mut selection = selection.clone();
    if selection.ids.is_empty() {
        selection.all = true;
    }
    let items = select(store.items()?, &selection)?;
    let pending: Vec<OutboxItem> = items
        .into_iter()
        .filter(|i| i.status == "pending")
        .collect();
    if pending.is_empty() {
        println!("nothing pending");
        return Ok(());
    }

    // Built on the first release rather than up front, so a review that ends in
    // nothing but rejections never starts an MCP server.
    let mut surfaces = Surfaces::default();
    let (mut sent, mut rejected, mut kept) = (0usize, 0usize, 0usize);

    for (i, item) in pending.iter().enumerate() {
        // Re-read: the earlier decisions in this loop, or another terminal,
        // may have moved it.
        let mut current = match store.item(&item.id) {
            Ok(item) => item,
            Err(_) => continue,
        };
        if current.status != "pending" {
            continue;
        }
        loop {
            println!("\n─── {} of {} ───", i + 1, pending.len());
            show(store, &current.id, false)?;
            print!("\n[s]end  [e]dit  [r]eject  [k]eep  [q]uit > ");
            use std::io::Write;
            std::io::stdout().flush()?;
            let mut line = String::new();
            // EOF ends the review without deciding anything, for the same
            // reason it means "no" everywhere else here.
            if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
                println!("\nstopped; {} left pending", pending.len() - i);
                return Ok(());
            }
            match line.trim().to_ascii_lowercase().as_str() {
                "s" | "send" => {
                    // The surface is built before the lock — an MCP startup is
                    // not something to hold the store across. A build failure
                    // is recorded on the item and the walk continues: one bad
                    // workspace must not end the whole review. Both arms write
                    // under the lock: `record_error` reads the item before it
                    // writes, and a lockless write in that gap can restore a
                    // stale `pending` over a draft another terminal just sent.
                    let release = match surfaces.for_item(global, &current).await {
                        Ok(surface) => {
                            let _lock = store.lock()?;
                            surface.release(store, &current).await
                        }
                        Err(e) => match store.lock() {
                            Ok(lock) => Err(record_release_failure(store, &lock, &current.id, e)),
                            // No lock means no safe write; the failure is
                            // reported without touching the store.
                            Err(lock_err) => Err(e.context(format!(
                                "(and the store lock needed to record this \
                                 failure could not be taken: {lock_err:#})"
                            ))),
                        },
                    };
                    match release {
                        Ok(output) => {
                            sent += 1;
                            println!("sent via `{}`", current.tool);
                            if !output.is_empty() {
                                println!("{}", indent(&output));
                            }
                        }
                        Err(e) => eprintln!("failed: {e:#}\nit stays pending"),
                    }
                    break;
                }
                "e" | "edit" => {
                    if let Err(e) = edit(store, &current.id, false) {
                        eprintln!("{e:#}");
                    }
                    // Round again with what the edit produced, so the thing
                    // being released is the thing just read.
                    current = store.item(&current.id)?;
                }
                "r" | "reject" => {
                    let _lock = store.lock()?;
                    store.resolve(&current.id, "rejected", None)?;
                    rejected += 1;
                    println!("rejected; nothing was sent");
                    break;
                }
                "k" | "keep" | "" => {
                    kept += 1;
                    break;
                }
                "q" | "quit" => {
                    println!("stopped; {} left pending", pending.len() - i);
                    return Ok(());
                }
                other => println!("`{other}`? one of s, e, r, k, q"),
            }
        }
    }
    println!("\n{sent} sent, {rejected} rejected, {kept} left pending");
    Ok(())
}

fn reject(store: &OutboxStore, selection: &Selection, reason: Option<String>) -> Result<()> {
    let _lock = store.lock()?;
    let items = select(store.items()?, selection)?;
    for item in &items {
        // Rejecting sends nothing, so — unlike `send` — a batch of them needs
        // no confirmation. The draft stays on file either way; what is lost is
        // the queue entry, not the work.
        let resolved = store.resolve(&item.id, "rejected", reason.clone())?;
        println!("rejected {}; nothing was sent", resolved.id);
    }
    if items.len() > 1 {
        println!("{} rejected", items.len());
    }
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
    use super::*;
    use serde_json::json;

    fn item(id: &str, status: &str, kind: OutboxKind, tool: &str) -> OutboxItem {
        OutboxItem {
            id: id.into(),
            status: status.into(),
            tool: tool.into(),
            kind,
            args_before: json!({"to": "a@example.com"}),
            args: json!({"to": "a@example.com"}),
            summary: format!("{tool} to a@example.com"),
            session_id: None,
            workspace: None,
            taint: Default::default(),
            created_at: "2026-08-06T07:00:00Z".into(),
            resolved_at: None,
            reason: None,
            error: None,
        }
    }

    fn queue() -> Vec<OutboxItem> {
        vec![
            item("aaa1", "pending", OutboxKind::Message, "mail__mail_send"),
            item("aaa2", "pending", OutboxKind::Message, "mail__mail_reply"),
            item(
                "bbb1",
                "pending",
                OutboxKind::Publish,
                "factory__bundle_publish",
            ),
            item("ccc1", "sent", OutboxKind::Message, "mail__mail_send"),
        ]
    }

    fn temp_store() -> OutboxStore {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir()
            .join("mecha-outbox-test")
            .join(format!("{}-{nanos}", std::process::id()));
        OutboxStore::open(dir).unwrap()
    }

    /// `review` shows a draft and then waits for a human, so the copy it holds
    /// can go stale in the only way that matters: another terminal sends it.
    /// The check has to happen against the store under the lock, not against
    /// what was on screen a minute ago — `resolve` refusing afterwards is too
    /// late, because by then the mail has gone out for real.
    #[test]
    fn a_draft_sent_while_you_were_deciding_is_not_sent_again() {
        let store = temp_store();
        let staged = store
            .stage(
                "mail__mail_send",
                OutboxKind::Message,
                json!({"to": "a@example.com"}),
                Default::default(),
                None,
                None,
            )
            .unwrap();

        // What the reviewer read, and still believes.
        let reviewed = staged.clone();
        assert_eq!(reviewed.status, "pending");
        claim_for_release(&store, &reviewed).expect("pending is releasable");

        // The other terminal wins the race.
        store.resolve(&staged.id, "sent", None).unwrap();

        let err = claim_for_release(&store, &reviewed)
            .unwrap_err()
            .to_string();
        assert!(err.contains("not pending"), "{err}");
        assert!(err.contains("nothing was sent"), "{err}");
    }

    /// The item that executes is the one the store holds, so the tool call and
    /// the record of it cannot describe different arguments.
    #[test]
    fn the_claim_returns_the_stores_copy_rather_than_the_callers() {
        let store = temp_store();
        let staged = store
            .stage(
                "mail__mail_send",
                OutboxKind::Message,
                json!({"to": "a@example.com"}),
                Default::default(),
                None,
                None,
            )
            .unwrap();
        store
            .update_args(&staged.id, json!({"to": "corrected@example.com"}))
            .unwrap();

        let claimed = claim_for_release(&store, &staged).unwrap();
        assert_eq!(claimed.args, json!({"to": "corrected@example.com"}));
    }

    fn selection(ids: &[&str]) -> Selection {
        Selection {
            ids: ids.iter().map(|s| s.to_string()).collect(),
            ..Selection::default()
        }
    }

    /// A typo'd `--kind` must not read as a clear queue. `list` used to take
    /// `select`'s error and `unwrap_or_default()` it, so `--kind publishes`
    /// printed "nothing pending" while the drafts sat there — the same failure
    /// `a_filter_that_matches_nothing_is_an_error` was written against, on the
    /// one surface that had not been checked for it.
    #[test]
    fn a_kind_that_is_not_a_kind_is_refused_on_every_surface() {
        let err = parse_kind(Some("publishes")).unwrap_err().to_string();
        assert!(err.contains("message | publish"), "{err}");

        // `select` and `list` both go through it, so neither can disagree.
        let err = select(
            queue(),
            &Selection {
                all: true,
                kind: Some("publishes".into()),
                ..Selection::default()
            },
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("message | publish"), "{err}");

        assert_eq!(parse_kind(None).unwrap(), None);
        assert_eq!(
            parse_kind(Some("publish")).unwrap(),
            Some(OutboxKind::Publish)
        );
    }

    /// The most expensive mistake this surface can make is releasing drafts
    /// nobody chose, so naming nothing acts on nothing — loudly.
    #[test]
    fn a_selection_that_names_nothing_is_refused_rather_than_meaning_everything() {
        let err = select(queue(), &Selection::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("--all"), "{err}");
        assert!(err.contains("acts on nothing"), "{err}");
    }

    #[test]
    fn ids_may_be_prefixes_several_at_a_time_and_never_double_count() {
        let chosen = select(queue(), &selection(&["aaa1", "bbb"])).unwrap();
        assert_eq!(
            chosen.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            vec!["aaa1", "bbb1"]
        );
        // The same item named twice is one send, not two.
        let chosen = select(queue(), &selection(&["aaa1", "aaa1"])).unwrap();
        assert_eq!(chosen.len(), 1);
        // An ambiguous prefix is an error, not a guess.
        let err = select(queue(), &selection(&["aaa"]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("matches 2"), "{err}");
        assert!(select(queue(), &selection(&["zzz"])).is_err());
    }

    /// `--all` is every *pending* item. A resolved one is not a candidate:
    /// re-sending something already sent is not a thing anyone means.
    #[test]
    fn all_means_every_pending_item_and_the_filters_narrow_it() {
        let everything = Selection {
            all: true,
            ..Selection::default()
        };
        let chosen = select(queue(), &everything).unwrap();
        assert_eq!(
            chosen.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            vec!["aaa1", "aaa2", "bbb1"],
            "the sent item is not a candidate"
        );

        let messages = Selection {
            all: true,
            kind: Some("message".into()),
            ..Selection::default()
        };
        assert_eq!(select(queue(), &messages).unwrap().len(), 2);

        let one_tool = Selection {
            all: true,
            via: Some("bundle_publish".into()),
            ..Selection::default()
        };
        assert_eq!(select(queue(), &one_tool).unwrap()[0].id, "bbb1");
    }

    /// A typo'd filter acting on zero items reads exactly like an empty queue,
    /// and the two want opposite reactions.
    #[test]
    fn a_filter_that_matches_nothing_is_an_error() {
        let nothing = Selection {
            all: true,
            via: Some("mail__mail_snd".into()),
            ..Selection::default()
        };
        let err = select(queue(), &nothing).unwrap_err().to_string();
        assert!(err.contains("nothing pending matches"), "{err}");

        let bad_kind = Selection {
            all: true,
            kind: Some("publishes".into()),
            ..Selection::default()
        };
        assert!(select(queue(), &bad_kind).is_err());
    }

    /// A filter alongside an id is a claim about that id, and a claim that is
    /// false has to fail — quietly acting on it anyway would make
    /// `--kind message` mean nothing when an id is present.
    #[test]
    fn an_id_that_contradicts_its_filters_is_refused() {
        let contradictory = Selection {
            ids: vec!["bbb1".into()],
            kind: Some("message".into()),
            ..Selection::default()
        };
        let err = select(queue(), &contradictory).unwrap_err().to_string();
        assert!(err.contains("does not match the filters"), "{err}");
    }

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
    /// A publish's reviewable object is whatever file says what is about to
    /// become public. For a bundle that is a rendered directory; for a poll it
    /// is the spec, and for a request type the manifest — both of which arrived
    /// with the factory's wider tool surface and neither of which has a bundle
    /// directory at all. Without a row here `show` prints a JSON blob and the
    /// reviewer has nothing to open.
    #[test]
    fn a_publish_points_at_the_file_that_defines_it() {
        let poll = serde_json::json!({
            "instrument": "book",
            "poll_id": "lab-feb",
            "spec": "/tmp/lab-feb.toml",
        });
        assert_eq!(
            local_paths(&poll, None),
            vec![("poll spec", std::path::PathBuf::from("/tmp/lab-feb.toml"))]
        );

        let form = serde_json::json!({"manifest": "/tmp/office-hours.toml"});
        assert_eq!(
            local_paths(&form, None),
            vec![(
                "form manifest",
                std::path::PathBuf::from("/tmp/office-hours.toml")
            )]
        );

        // A subject line that happens to look like a path is still not one:
        // the key is what decides, never the value's shape.
        let message = serde_json::json!({"to": "a@b.c", "subject": "/etc/passwd"});
        assert!(local_paths(&message, None).is_empty());
    }

    /// A relative argument means nothing apart from the jail it was written
    /// in. `send` has always known that; `show` did not, and the first real
    /// staged poll reported a spec sitting right there as "⚠ gone" because the
    /// reviewer was standing in a different directory. The symmetric case is
    /// the dangerous one — a same-named file beside the reviewer would be
    /// displayed, and offered to open, as though it were the draft's source.
    #[test]
    fn a_relative_path_resolves_against_the_jail_it_was_drafted_in() {
        let args = serde_json::json!({"spec": "retro-spec.toml"});
        let jail = std::path::Path::new("/home/someone/.mecha/work/chat");
        assert_eq!(
            local_paths(&args, Some(jail)),
            vec![("poll spec", jail.join("retro-spec.toml"))]
        );

        // An absolute path is already an answer; the jail must not be prepended
        // to it.
        let absolute = serde_json::json!({"spec": "/tmp/elsewhere.toml"});
        assert_eq!(
            local_paths(&absolute, Some(jail)),
            vec![("poll spec", std::path::PathBuf::from("/tmp/elsewhere.toml"))]
        );

        // Staged before the field existed: resolve as it always did, against
        // wherever the reviewer is.
        assert_eq!(
            local_paths(&args, None),
            vec![("poll spec", std::path::PathBuf::from("retro-spec.toml"))]
        );
    }

    /// A surface with nothing in it, standing in for one whose configuration
    /// cannot serve the staged tool — buildable in a test because it needs no
    /// MCP server, no config file, and no model.
    fn empty_surface() -> Surface {
        use std::sync::Arc;
        Surface {
            tools: setup::PreparedTools {
                registry: mecha_core::tool::Registry::new(),
                sandbox: Arc::new(mecha_core::sandbox::Sandbox::new(Default::default())),
                workspace: std::env::temp_dir(),
                config: mecha_core::config::Config::default(),
                approver: Arc::new(mecha_core::tool::ModeApprover {
                    mode: mecha_core::config::PermissionMode::Allow,
                }),
                todo: None,
                mailbox: None,
                _mcp: Vec::new(),
            },
            ctx: mecha_core::tool::ToolCtx::default(),
        }
    }

    /// A release that dies before the tool executes must still say so on the
    /// item. This path used to bail without `record_error`, so the failure
    /// reached only stderr — which the TUI's detached release closes — and the
    /// item sat pending with `error: null` while the watch reported "still
    /// releasing" against a record holding nothing.
    #[tokio::test]
    async fn a_release_that_dies_before_the_tool_runs_records_the_error_on_the_item() {
        let store = temp_store();
        let staged = store
            .stage(
                "mail__mail_send",
                OutboxKind::Message,
                json!({"to": "a@example.com"}),
                Default::default(),
                None,
                None,
            )
            .unwrap();

        let err = empty_surface()
            .release(&store, &staged)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("not available"), "{err}");

        let after = store.item(&staged.id).unwrap();
        assert_eq!(after.status, "pending", "the draft survives the failure");
        assert!(
            after
                .error
                .as_deref()
                .is_some_and(|e| e.contains("not available")),
            "the failure must be on the item, not only stderr: {:?}",
            after.error
        );
    }

    /// `Surface::build` runs before `release`, so its failures never reached
    /// `record_error` at all. Both `send` and `review` route them through
    /// this, so a workspace that cannot build a surface leaves its reason on
    /// the item it failed.
    ///
    /// That the guard argument below is *required to compile* is itself the
    /// type-level test for the race this helper used to permit: `review`'s
    /// build-failure arm called it without the store lock, and a concurrent
    /// send inside `record_error`'s read/write gap had its `sent` overwritten
    /// back to `pending`. There is no way to write this call lockless now.
    #[test]
    fn a_surface_build_failure_lands_on_the_item_not_only_stderr() {
        let store = temp_store();
        let staged = store
            .stage(
                "factory__bundle_publish",
                OutboxKind::Publish,
                json!({"bundle": "site"}),
                Default::default(),
                None,
                None,
            )
            .unwrap();

        let lock = store.lock().unwrap();
        let err = record_release_failure(
            &store,
            &lock,
            &staged.id,
            anyhow::anyhow!("the MCP server would not start"),
        );
        assert!(err.to_string().contains("would not start"), "{err:#}");

        let after = store.item(&staged.id).unwrap();
        assert_eq!(after.status, "pending", "record_error never resolves");
        assert_eq!(
            after.error.as_deref(),
            Some("the MCP server would not start")
        );
    }
}
