//! A deferred call's result, arriving after the run that made it: what both
//! chat hosts do with it (`docs/BACKGROUND-JOBS-DESIGN.md` §2.3; ARCHITECTURE
//! §Background jobs). The queue and the finishing rule are core's
//! (`mecha_core::jobs`); this is the host's half — where the result lands,
//! what is recorded, and what the next run is owed.

use std::collections::HashMap;
use std::path::Path;

use mecha_core::agent::Conversation;
use mecha_core::jobs::Delivered;
use mecha_core::session::{Record, Session, Transcript};

/// A host's job queue and the hand-off from it: every conversation's jobs,
/// one in flight per key (`JobQueue`), and the finished ones waiting for the
/// host's delivery task, which the first turn starts (`start`).
/// What the queue tells its host, in the order it happened: a finished job,
/// or a conversation whose line changed. One channel, so a page sees a
/// picture land before the line that no longer holds it.
pub(super) enum Late {
    Delivered(Box<Delivered>),
    Changed(String),
}

pub(super) struct Jobs {
    pub(super) queue: std::sync::Arc<mecha_core::jobs::JobQueue>,
    delivered: std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<Late>>>,
    started: std::sync::Once,
    /// Run numbers, unique across the process: a chat removed from the map
    /// and opened again gets a fresh `LateState`, and its numbers must never
    /// meet an earlier incarnation's job (review of #583).
    next_run: std::sync::atomic::AtomicUsize,
}

impl Default for Jobs {
    fn default() -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let changed = tx.clone();
        let queue = mecha_core::jobs::JobQueue::new(move |d| {
            let _ = tx.send(Late::Delivered(Box::new(d)));
        });
        queue.set_watch(move |key| {
            let _ = changed.send(Late::Changed(key.to_string()));
        });
        Jobs {
            queue,
            delivered: std::sync::Mutex::new(Some(rx)),
            started: std::sync::Once::new(),
            next_run: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl Jobs {
    /// Number the run about to start, for its sink and its `LateState`.
    pub(super) fn number(&self) -> usize {
        self.next_run
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    /// Hand the finished jobs to `deliver`, once: the first call takes the
    /// receiver, every later one does nothing.
    pub(super) fn start(&self, deliver: impl FnOnce(tokio::sync::mpsc::UnboundedReceiver<Late>)) {
        self.started.call_once(|| {
            if let Some(rx) = self.delivered.lock().ok().and_then(|mut r| r.take()) {
                deliver(rx);
            }
        });
    }
}

/// One conversation's late results, held beside it in the host's map.
#[derive(Default)]
pub(super) struct LateState {
    /// Results that arrived while a run held the conversation, landed when
    /// it hands the conversation back, before its end event sends the page
    /// to re-read.
    pub(super) held: Vec<Delivered>,
    /// Notes owed to the next run — a late result's "it has arrived" —
    /// recorded as `Record::PendingNote` and taken by the next turn.
    notes: Vec<String>,
    /// How many runs the transcript holds an outcome for: the ordinal the
    /// next run's outcome takes.
    runs_recorded: usize,
    /// How each of this conversation's runs ended, by its number
    /// (`Jobs::number`, which its jobs carry): what a late result may do.
    ended: HashMap<usize, RunEnd>,
}

impl LateState {
    /// As a resumed transcript leaves it: the notes no run has taken, and
    /// the outcomes already written.
    pub(super) fn resumed(t: &Transcript) -> Self {
        LateState {
            notes: t.pending_notes.clone(),
            runs_recorded: t.outcomes.len(),
            ..Default::default()
        }
    }

    /// As the transcript at `path` leaves it; a file that will not read
    /// owes nothing, which only costs a note.
    pub(super) fn of_file(path: &Path) -> Self {
        Session::read(path)
            .map(|t| Self::resumed(&t))
            .unwrap_or_default()
    }

    /// Run `run` handed back: `ok` or rolled back, and whether it `wrote`
    /// its outcome.
    pub(super) fn ended(&mut self, run: usize, ok: bool, wrote: bool) {
        let end = match (ok, wrote) {
            (false, _) => RunEnd::RolledBack,
            (true, false) => RunEnd::Unwritten,
            (true, true) => {
                self.runs_recorded += 1;
                RunEnd::Wrote(self.runs_recorded - 1)
            }
        };
        self.ended.insert(run, end);
    }

    /// The notes owed to the run about to start.
    pub(super) fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }

    /// Put back notes a run took and did not deliver: it ended in error and
    /// was rolled back, and the file still owes them (`Record::PendingNote`
    /// retires only at an outcome), so memory owes them too.
    pub(super) fn owe(&mut self, notes: Vec<String>) {
        let mut notes = notes;
        notes.append(&mut self.notes);
        self.notes = notes;
    }

    /// Forget how every run ended, as a reopened chat would.
    #[cfg(test)]
    pub(super) fn forget_outcomes(&mut self) {
        self.ended.clear();
    }
}

/// How a run handed its conversation back, for its jobs' late results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunEnd {
    /// It wrote its outcome, this ordinal among the file's: a late failure
    /// books against it.
    Wrote(usize),
    /// It ended `Ok` but its outcome did not write: its call is still in the
    /// conversation, so its picture still lands, and only the booking has
    /// nowhere to go (review of #583).
    Unwritten,
    /// It ended in error and was rolled back, call and all: its job's late
    /// result has no message to land in, no run and no note (review of #573,
    /// pass 9).
    RolledBack,
}

/// What the next run is told when a late result has arrived since the last
/// reply: the call and what its result now reads, facts only (§5.2) — what
/// to do about it is the tool description's. Capped, since it rides every
/// request of that run. A result from outside is never quoted here: notes
/// arm taint by stem only, and a cut would land inside its envelope, so the
/// note points at the result instead (review of #583).
fn arrived_note(tool: &str, content: &str, external: bool) -> String {
    const MAX: usize = 800;
    if external {
        return format!(
            "The {tool} call from an earlier turn has finished since your last reply. Its result \
             came from outside this machine; it is in the conversation, as data."
        );
    }
    let mut said = content.trim();
    if said.len() > MAX {
        let mut at = MAX;
        while !said.is_char_boundary(at) {
            at -= 1;
        }
        said = &said[..at];
    }
    format!(
        "The {tool} call from an earlier turn has finished since your last reply. Its result now \
         reads:\n{said}"
    )
}

/// A late result into a conversation in hand (§2.3): settled by the loop's
/// own rule (cap, envelope, taint armed from what came back), put where its
/// "being made" result is and recorded there, the taint recorded whether or
/// not a result was left to rewrite — a compaction may have cut it, and
/// what came back still entered — a failure booked against the run that
/// made the call, and a note owed to the next run.
pub(super) fn land(
    state: &mut LateState,
    session: &Session,
    convo: &mut Conversation,
    late: &Delivered,
) {
    let end = state.ended.get(&late.run).copied();
    // What came back entered whatever happened to the call, so its taint is
    // armed and recorded on every path below.
    let out = late.settle(&mut convo.taint);
    let run = match end {
        Some(RunEnd::Wrote(ordinal)) => Some(ordinal),
        Some(RunEnd::Unwritten) => None,
        // Rolled back, call and all: no result to rewrite, no run to book a
        // failure against, no picture the model knows it asked for. And a
        // run this conversation never saw — an earlier incarnation's, which
        // its removal should have stopped (`release_*` refuse while a job is
        // out) — is no answer to anything here either: whatever settled that
        // call on reopening stands.
        Some(RunEnd::RolledBack) | None => {
            let _ = session.append(&Record::Taint(convo.taint));
            return;
        }
    };
    if let Some(index) = convo.apply_late(&late.call_id, &out.content, out.is_error, out.external) {
        if let Err(e) = session.append(&Record::LateResult {
            index,
            tool_use_id: late.call_id.clone(),
            content: out.content.clone(),
            is_error: out.is_error,
            external: out.external,
        }) {
            tracing::warn!("a late result was not recorded: {e:#}");
        }
    }
    if let Err(e) = session.append(&Record::Taint(convo.taint)) {
        tracing::warn!("a late result's taint was not recorded: {e:#}");
    }
    if let (true, Some(run)) = (out.is_error, run) {
        let _ = session.append(&Record::LateFailure {
            run,
            tool_use_id: late.call_id.clone(),
        });
    }
    let note = arrived_note(&late.tool, &out.content, out.external);
    if let Err(e) = session.append(&Record::PendingNote { note: note.clone() }) {
        tracing::warn!("a late result's note was not recorded: {e:#}");
    }
    state.notes.push(note);
}

/// Settle every "being made" result a resumed conversation holds, when no
/// job can be behind it (§2.3, the restart repair): by what the workspace
/// holds, never by the absent job (`imagegen::repair_orphan`). Recorded as
/// late results, then one taint checkpoint covering them. Only
/// `image_generate` writes a "being made" result, and it declares no outside
/// reach, so its repair is recorded as not external; a deferring tool that
/// could return untrusted content would take its declared capability here
/// instead. No failure is booked: the run that made the call is not known
/// from here.
pub(super) fn repair_orphans(session: &Session, workspace: &Path, convo: &mut Conversation) {
    use mecha_core::message::Block;
    // The calls that could have deferred: an orphan is identified by its
    // call, never by its words, which any tool's result can carry — a shell
    // echo, a file read (review of #583, pass 6).
    let renders: std::collections::HashSet<&str> = convo
        .messages
        .iter()
        .flat_map(|m| &m.content)
        .filter_map(|b| match b {
            Block::ToolUse { id, name, .. } if name == "image_generate" => Some(id.as_str()),
            _ => None,
        })
        .collect();
    let orphans: Vec<(String, String)> = convo
        .messages
        .iter()
        .filter(|m| m.role == mecha_core::message::Role::User)
        .flat_map(|m| &m.content)
        .filter_map(|b| match b {
            Block::ToolResult {
                tool_use_id,
                content,
                ..
            } if renders.contains(tool_use_id.as_str())
                && content.starts_with(mecha_core::imagegen::BEING_MADE) =>
            {
                Some((tool_use_id.clone(), content.clone()))
            }
            _ => None,
        })
        .collect();
    if orphans.is_empty() {
        return;
    }
    for (id, content) in orphans {
        let Some((settled, is_error)) =
            mecha_core::imagegen::repair_orphan(workspace, &id, &content)
        else {
            continue;
        };
        if let Some(index) = convo.apply_late(&id, &settled, is_error, false) {
            if let Err(e) = session.append(&Record::LateResult {
                index,
                tool_use_id: id,
                content: settled,
                is_error,
                external: false,
            }) {
                tracing::warn!("a repaired picture result was not recorded: {e:#}");
            }
        }
    }
    let _ = session.append(&Record::Taint(convo.taint));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failed run's notes are owed again, ahead of any that arrived while
    /// it ran.
    #[test]
    fn a_failed_runs_notes_are_owed_again_first() {
        let mut state = LateState::default();
        state.notes.push("older".into());
        let taken = state.take_notes();
        state.notes.push("newer".into());
        state.owe(taken);
        assert_eq!(
            state.take_notes(),
            vec!["older".to_string(), "newer".to_string()]
        );
    }
}
