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
pub(super) struct Jobs {
    pub(super) queue: std::sync::Arc<mecha_core::jobs::JobQueue>,
    delivered: std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<Delivered>>>,
    started: std::sync::Once,
}

impl Default for Jobs {
    fn default() -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Jobs {
            queue: mecha_core::jobs::JobQueue::new(move |d| {
                let _ = tx.send(d);
            }),
            delivered: std::sync::Mutex::new(Some(rx)),
            started: std::sync::Once::new(),
        }
    }
}

impl Jobs {
    /// Hand the finished jobs to `deliver`, once: the first call takes the
    /// receiver, every later one does nothing.
    pub(super) fn start(
        &self,
        deliver: impl FnOnce(tokio::sync::mpsc::UnboundedReceiver<Delivered>),
    ) {
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
    /// The turns this process has started on the conversation, numbered: a
    /// run's jobs carry its number (`JobQueue::sink`), and `outcome_of` maps
    /// it to the ordinal of the outcome the run wrote when it handed back —
    /// which a late failure books against (`Record::LateFailure`). A run
    /// that ended in error wrote none, and its call was rolled back with it,
    /// so it has no entry, and its job's late result books and notes nothing
    /// (review of #573, pass 9).
    turns: usize,
    outcome_of: HashMap<usize, usize>,
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

    /// Number the run about to start.
    pub(super) fn next_turn(&mut self) -> usize {
        let turn = self.turns;
        self.turns += 1;
        turn
    }

    /// Run `turn` handed back having written its outcome.
    pub(super) fn recorded(&mut self, turn: usize) {
        self.outcome_of.insert(turn, self.runs_recorded);
        self.runs_recorded += 1;
    }

    /// The notes owed to the run about to start.
    pub(super) fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }

    /// Forget which outcome each run wrote, as if each had failed.
    #[cfg(test)]
    pub(super) fn forget_outcomes(&mut self) {
        self.outcome_of.clear();
    }
}

/// What the next run is told when a late result has arrived since the last
/// reply: the call and what its result now reads, facts only (§5.2) — what
/// to do about it is the tool description's. Capped, since it rides every
/// request of that run.
fn arrived_note(tool: &str, content: &str) -> String {
    const MAX: usize = 800;
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
    let run = state.outcome_of.get(&late.run).copied();
    // What came back entered whatever happened to the call, so its taint is
    // armed and recorded on every path below.
    let out = late.settle(&mut convo.taint);
    // The run that made the call ended in error: it was rolled back, call
    // and all, so there is no result to rewrite, no run to book a failure
    // against, and no picture the model knows it asked for.
    let Some(run) = run else {
        let _ = session.append(&Record::Taint(convo.taint));
        return;
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
    if out.is_error {
        let _ = session.append(&Record::LateFailure {
            run,
            tool_use_id: late.call_id.clone(),
        });
    }
    let note = arrived_note(&late.tool, &out.content);
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
    let orphans: Vec<(String, String)> = convo
        .messages
        .iter()
        .filter(|m| m.role == mecha_core::message::Role::User)
        .flat_map(|m| &m.content)
        .filter_map(|b| match b {
            mecha_core::message::Block::ToolResult {
                tool_use_id,
                content,
                ..
            } if content.starts_with(mecha_core::imagegen::BEING_MADE) => {
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
