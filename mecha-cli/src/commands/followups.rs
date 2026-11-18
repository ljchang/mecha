//! `mecha followups` — the follow-ups a run proposed, and the owner's answer.
//!
//! A proposal is the model's (`propose_follow_up`); everything here is the
//! owner's. `accept` is the one way a proposal becomes a workflow, whose
//! reminder then comes due in Tasks → workflows and on Home
//! (`mecha_core::followup`, `ASSISTANT-FOLLOW-THROUGH-DESIGN.md` §Model-proposed
//! follow-ups). Shaped for `commands::review::review_source`: `list --json`,
//! `show`, `accept`, `reject --reason`, so the web's proposals pane and the
//! TUI's `/queues` drive it as they drive the other proposal stores.

use anyhow::{ensure, Context, Result};
use chrono::Utc;
use mecha_core::followup::{self, FollowUp, FollowUpStore};
use mecha_core::workflow::WorkflowStore;

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Follow-ups waiting on your answer (`--all` for decided ones too).
    List {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        all: bool,
    },
    /// One follow-up whole: what, when, and the conversation it came from.
    Show { id: String },
    /// Schedule it: creates the workflow whose reminder comes due at its time.
    Accept { id: String },
    /// Decline it, saying why.
    Reject {
        id: String,
        #[arg(long)]
        reason: String,
    },
}

fn store() -> Result<FollowUpStore> {
    FollowUpStore::open_default()
}

/// The list line's detail: when, in the owner's zone, and whether the
/// proposing conversation had read outside content.
fn detail(f: &FollowUp, tz: chrono_tz::Tz) -> String {
    let when = f.due_at.with_timezone(&tz).format("%a %b %-d, %H:%M %Z");
    let mut d = format!("due {when}");
    if f.armed() {
        d.push_str(" · proposed after reading outside content");
    }
    if f.is_proposed() && f.due_at <= Utc::now() {
        d.push_str(" · its time has passed; accepting it makes it due now");
    }
    if !f.is_proposed() {
        d.push_str(&format!(" · {}", f.status));
    }
    d
}

/// Where the owner picks the conversation back up: the web app's link, and
/// the TUI's resume for a terminal, which has no Open-chat button.
fn way_back(f: &FollowUp) -> String {
    format!(
        "Open chat on its workflow in Tasks → Workflows, or `mecha tui --resume {}`",
        f.session_id
    )
}

fn owner_tz() -> chrono_tz::Tz {
    mecha_core::config::Config::load_global()
        .ok()
        .and_then(|c| c.agent.timezone.as_deref().and_then(|t| t.parse().ok()))
        .unwrap_or(chrono_tz::UTC)
}

fn show(f: &FollowUp, tz: chrono_tz::Tz) -> String {
    let mut out = String::new();
    out.push_str(&format!("follow-up   {}\n", f.id));
    out.push_str(&format!("status      {}\n", f.status));
    out.push_str(&format!(
        "due         {}\n",
        f.due_at.with_timezone(&tz).format("%Y-%m-%d %H:%M %Z")
    ));
    out.push_str(&format!("from chat   {}\n", f.session_id));
    out.push_str(&format!("back to it  {}\n", way_back(f)));
    if let Some(w) = &f.workflow_id {
        out.push_str(&format!("workflow    {w}\n"));
    }
    if let Some(r) = &f.reason {
        out.push_str(&format!("reason      {r}\n"));
    }
    if f.armed() {
        out.push_str(
            "\n⚠ The conversation that proposed this had read content from outside (mail, \
             the web, a document). Accept it only if you would have asked for it yourself.\n",
        );
    }
    out.push_str(&format!("\n{}\n", f.what));
    out
}

/// Accept, under the follow-up store's lock: the workflow is written first
/// and the proposal marked only if that succeeded, so a failure leaves it
/// proposed. A workflow already under the id is the one an interrupted
/// accept wrote, and is taken as done rather than refused — the retry
/// finishes the first attempt, never duplicates it.
pub fn accept(
    store: &FollowUpStore,
    workflows: &WorkflowStore,
    id: &str,
    outbox_root: Option<std::path::PathBuf>,
) -> Result<FollowUp> {
    let now = Utc::now();
    let found = store.find(id)?;
    store.decide(&found.id, followup::ACCEPTED, now, |f| {
        let workspace = f
            .workspace
            .clone()
            .context("this follow-up recorded no workspace, so its workflow has no jail")?;
        let w = followup::workflow_for(f, workspace, outbox_root, now);
        match workflows.get(&w.id) {
            Ok(existing) => ensure!(
                existing.session_id.as_deref() == Some(f.session_id.as_str()),
                "workflow {} exists and belongs to another conversation",
                existing.id
            ),
            Err(_) => {
                workflows.create(w.clone())?;
            }
        }
        f.workflow_id = Some(w.id);
        Ok(())
    })
}

pub fn reject(store: &FollowUpStore, id: &str, reason: &str) -> Result<FollowUp> {
    let reason = reason.trim();
    ensure!(!reason.is_empty(), "say why (`--reason`)");
    let found = store.find(id)?;
    store.decide(&found.id, followup::REJECTED, Utc::now(), |f| {
        f.reason = Some(reason.to_string());
        Ok(())
    })
}

pub fn run(args: Args) -> Result<()> {
    let tz = owner_tz();
    match args.cmd.unwrap_or(Cmd::List {
        json: false,
        all: false,
    }) {
        Cmd::List { json, all } => {
            // A store that does not exist yet is empty; one that exists and
            // cannot be read whole says so rather than listing a part.
            let (items, skipped) = match FollowUpStore::open_existing_default() {
                Some(s) => s.items_counting()?,
                None => (vec![], 0),
            };
            ensure!(
                skipped == 0,
                "{skipped} follow-up record(s) could not be read; fix or remove them under {}",
                FollowUpStore::default_root()?.display()
            );
            let shown: Vec<&FollowUp> = items.iter().filter(|f| all || f.is_proposed()).collect();
            if json {
                let rows: Vec<serde_json::Value> = shown
                    .iter()
                    .map(|f| {
                        serde_json::json!({
                            "id": f.id,
                            "kind": "follow-up",
                            "title": f.what,
                            "detail": detail(f, tz),
                            "status": f.status,
                            "due_at": f.due_at,
                            "session_id": f.session_id,
                            "taint": f.taint,
                            "proposed_at": f.proposed_at,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else if shown.is_empty() {
                println!("No follow-ups are waiting on you.");
            } else {
                for f in shown {
                    println!(
                        "{}  {}  ({})",
                        followup::short(&f.id),
                        f.what,
                        detail(f, tz)
                    );
                }
            }
        }
        // A read: it must not create the store it reads (`List`'s rule).
        Cmd::Show { id } => {
            let store = FollowUpStore::open_existing_default()
                .with_context(|| format!("no such follow-up: {id}"))?;
            print!("{}", show(&store.find(&id)?, tz));
        }
        Cmd::Accept { id } => {
            let outbox_root = mecha_core::config::Config::load_global()?.outbox.dir;
            let f = accept(
                &store()?,
                &WorkflowStore::default_store()?,
                &id,
                outbox_root,
            )?;
            let when = if f.due_at <= Utc::now() {
                "now — its time had already passed".to_string()
            } else {
                f.due_at
                    .with_timezone(&tz)
                    .format("at %Y-%m-%d %H:%M %Z")
                    .to_string()
            };
            println!(
                "Accepted {}: workflow {} reminds you {when}.\n{}.",
                followup::short(&f.id),
                f.workflow_id.as_deref().unwrap_or("?"),
                way_back(&f)
            );
        }
        Cmd::Reject { id, reason } => {
            let f = reject(&store()?, &id, &reason)?;
            println!("Rejected {}.", followup::short(&f.id));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::agent::Taint;

    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn stores() -> (Scratch, FollowUpStore, WorkflowStore) {
        let dir = std::env::temp_dir().join(format!("followups-cli-{}", uuid::Uuid::new_v4()));
        let f = FollowUpStore::open(dir.join("followups")).unwrap();
        let w = WorkflowStore::at(dir.join("workflows"));
        (Scratch(dir), f, w)
    }

    fn proposed(store: &FollowUpStore, workspace: Option<&str>) -> FollowUp {
        let now = Utc::now();
        store
            .propose(
                "check the build".into(),
                now + chrono::Duration::hours(2),
                "sess-a",
                workspace.map(Into::into),
                Taint::default(),
                now,
            )
            .unwrap()
    }

    #[test]
    fn accepting_writes_the_workflow_that_reminds_and_marks_the_proposal() {
        let (_d, store, workflows) = stores();
        let f = proposed(&store, Some("/tmp/w"));
        let done = accept(&store, &workflows, followup::short(&f.id), None).unwrap();
        assert_eq!(done.status, followup::ACCEPTED);
        let w = workflows.get(done.workflow_id.as_deref().unwrap()).unwrap();
        assert_eq!(w.session_id.as_deref(), Some("sess-a"));
        assert_eq!(
            w.commitment.as_ref().unwrap().follow_up_at,
            Some(f.due_at),
            "the reminder comes due at the proposal's time"
        );
        // Decided once.
        assert!(accept(&store, &workflows, &f.id, None).is_err());
        assert!(reject(&store, &f.id, "changed my mind").is_err());
    }

    #[test]
    fn an_interrupted_accept_is_finished_by_the_retry_not_duplicated() {
        let (_d, store, workflows) = stores();
        let f = proposed(&store, Some("/tmp/w"));
        // The workflow landed and the proposal's own write did not.
        workflows
            .create(followup::workflow_for(
                &f,
                "/tmp/w".into(),
                None,
                Utc::now(),
            ))
            .unwrap();
        let done = accept(&store, &workflows, &f.id, None).unwrap();
        assert_eq!(done.status, followup::ACCEPTED);
        assert_eq!(workflows.list().unwrap().len(), 1);
    }

    #[test]
    fn a_failed_accept_leaves_it_proposed() {
        let (_d, store, workflows) = stores();
        let f = proposed(&store, None);
        assert!(accept(&store, &workflows, &f.id, None).is_err());
        assert!(store.get(&f.id).unwrap().is_proposed());
        assert!(workflows.list().unwrap().is_empty());
    }

    #[test]
    fn a_reject_needs_a_reason_and_keeps_it() {
        let (_d, store, _w) = stores();
        let f = proposed(&store, Some("/tmp/w"));
        assert!(reject(&store, &f.id, "  ").is_err());
        let done = reject(&store, &f.id, "not needed").unwrap();
        assert_eq!(done.status, followup::REJECTED);
        assert_eq!(done.reason.as_deref(), Some("not needed"));
    }

    #[test]
    fn show_warns_when_the_proposing_conversation_had_read_outside_content() {
        let (_d, store, _w) = stores();
        let mut f = proposed(&store, Some("/tmp/w"));
        assert!(!show(&f, chrono_tz::UTC).contains('⚠'));
        f.taint = Some(Taint {
            private: false,
            untrusted: true,
        });
        assert!(show(&f, chrono_tz::UTC).contains('⚠'));
        assert!(detail(&f, chrono_tz::UTC).contains("outside content"));
    }
}
