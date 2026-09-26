//! What went right, as the owner said so — derived where it is recorded
//! (`docs/APPRAISAL-WIRING-DESIGN.md` L2, row 2e-4a, ruled R40).
//!
//! The learning store is corrections only: every trigger the reflector reads
//! is a person stepping in. An **owner-verified success** is the other half,
//! and it needs no new recording, because each kind is an act the owner
//! already performs, in the store that already owns it:
//!
//! - **a draft sent unchanged** — a model-written message the owner released
//!   as drafted (`OutboxItem::writing_outcome` is `SentUnchanged`);
//! - **a task closed `done`** that no recorded reopen undoes (1b's closure
//!   record, by `undoes`);
//! - **a workflow the owner closed** — `workflow close` runs after a passing
//!   verification — that no `workflow reopen` took back;
//! - **a question the owner answered**, whose session then finished of its
//!   own accord (the appraisal's question arm, the same reading).
//!
//! **Derived at read time, never stored** (R40, the owner's ruling). There is
//! no success store and no ledger of successes, so a reopen *withdraws* a
//! success by construction: the next read finds the reopen beside the
//! closure it undoes and reports the pair as withdrawn, never as standing.
//! That is 1d's rule for every owner verdict — read from the store that owns
//! it, never copied into a new one — and it is what keeps a copied success
//! from outliving the owner taking it back.
//!
//! **Self-judged success is never one of these.** Every kind is an act of the
//! owner's (a release, a closure, a close, an answer); nothing a model says
//! about its own work, and no appraisal's `good`, enters the set. The one
//! harness fact read is the question arm's recorded stop cause, which says
//! the resumed run finished — the same fact the appraisal signs on.
//!
//! **Writing exemplars ride on it, in shadow.** A draft sent unchanged is the
//! positive half of the comparison `reflect` mines from edits
//! (`mined_outbox`), and its exemplar is the draft itself, verbatim — its
//! tool, the arguments that went out, the situation keyed the way an edit's
//! lesson is (the drafting tool), and an [`Origin`] from the staging taint.
//! No model call makes one. **Nothing serves an exemplar to a run in this
//! row**: the only reader is the owner's readout (`mecha sessions
//! successes`). Serving them to drafting runs is a later lever, and it must
//! arm `private_data` as the brief does (R35) — an exemplar is sent mail.
//!
//! What this leaves for later rows: planning success examples and contrast
//! evidence for the reflector (2e-4b), and staged skill drafts (2e-4c,
//! deferred by R40 until this set has been read on real data).

use crate::closure::{Actor, Move, Transition};
use crate::goal::GoalRef;
use crate::learning::Origin;
use crate::outbox::{OutboxItem, WritingOutcome};
use crate::questions::Question;
use crate::situation::Situation;
use crate::workflow::{Disposition, Workflow};
use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// The owner's act that verified a success, with the record it is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    /// An outbox item released as drafted.
    SentUnchanged { item: String },
    /// A board task closed `done`, by closure record `closure`.
    TaskDone { task: String, closure: String },
    /// A workflow the owner closed after its verification passed.
    WorkflowClosed { workflow: String },
    /// A question the owner answered, whose session then completed.
    QuestionAnswered { question: String },
}

impl Act {
    /// The kind, as the readout counts it.
    pub fn kind(&self) -> &'static str {
        match self {
            Act::SentUnchanged { .. } => "sent_unchanged",
            Act::TaskDone { .. } => "task_done",
            Act::WorkflowClosed { .. } => "workflow_closed",
            Act::QuestionAnswered { .. } => "question_answered",
        }
    }

    /// Where the act is recorded: `<store>:<id>`.
    pub fn pointer(&self) -> String {
        match self {
            Act::SentUnchanged { item } => format!("outbox:{item}"),
            Act::TaskDone { closure, .. } => format!("closure:{closure}"),
            Act::WorkflowClosed { workflow } => format!("workflow:{workflow}"),
            Act::QuestionAnswered { question } => format!("question:{question}"),
        }
    }
}

/// One owner-verified success.
#[derive(Debug, Clone, PartialEq)]
pub struct Success {
    pub act: Act,
    /// The sessions whose work it verified, as the owning record names them.
    /// Empty when the record names none (a draft staged where no session
    /// was recorded, a closure of a task the board linked to no session).
    pub sessions: Vec<String>,
    /// The goal the work was toward, where the record carries one: the task
    /// a closure or a task-bound workflow names. Never inferred.
    pub goal: Option<GoalRef>,
    /// When the owner acted; `None` where the record's time does not parse.
    pub at: Option<DateTime<Utc>>,
}

/// A success the owner later took back.
#[derive(Debug, Clone, PartialEq)]
pub struct Withdrawn {
    pub success: Success,
    /// The act that took it back, `<store>:<id>` — the reopen's closure
    /// record, or the workflow whose `reopened` event did it.
    pub by: String,
    pub at: Option<DateTime<Utc>>,
}

/// A candidate success this read cannot settle. Never counted as one:
/// unknown is never clean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unknown {
    pub pointer: String,
    pub why: &'static str,
}

/// A writing exemplar: a draft the owner sent as written, verbatim.
#[derive(Debug, Clone, PartialEq)]
pub struct Exemplar {
    pub item: String,
    /// The drafting tool, by registry name.
    pub tool: String,
    /// The arguments that went out — equal to the drafted ones, which is
    /// what "unchanged" means.
    pub args: serde_json::Value,
    pub session: Option<String>,
    /// Keyed as an edit's lesson is: the drafting tool, nothing else. The
    /// item records no surface, and its `workspace` is the drafting jail,
    /// which is not a rules-block workspace (`reflect` never stamps a jail
    /// as a key). The positive half of the same comparison, in the same
    /// region.
    pub situation: Situation,
    /// From the conversation's taint when the draft was staged, fail-closed
    /// (`learning::classify_origin`): third-party text in context makes it
    /// `Untrusted`, and only a `Clean` exemplar could ever be served.
    pub origin: Origin,
    pub sent_at: Option<DateTime<Utc>>,
}

/// The success set, as one read of the owning stores found it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Successes {
    pub standing: Vec<Success>,
    pub withdrawn: Vec<Withdrawn>,
    pub unknown: Vec<Unknown>,
    /// One writing exemplar per standing `SentUnchanged` success.
    pub exemplars: Vec<Exemplar>,
    /// Successes whose every named session is a smoke-test or experiment
    /// session: the harness measuring itself, in no count above.
    pub hidden: usize,
    /// The stores that could not be fully read — the set is short of them,
    /// not empty of them.
    pub unreadable: Vec<&'static str>,
}

impl Successes {
    /// A store was short, so the set is a lower bound.
    pub fn partial(&self) -> bool {
        !self.unreadable.is_empty()
    }

    /// Standing successes by kind.
    pub fn by_kind(&self) -> BTreeMap<&'static str, usize> {
        let mut out = BTreeMap::new();
        for s in &self.standing {
            *out.entry(s.act.kind()).or_insert(0) += 1;
        }
        out
    }
}

/// What the derivation reads: each owning store, and whether it was read in
/// full. A store never created is empty and read; one that could not be
/// read is empty *and* marked.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sources<'a> {
    pub drafts: &'a [OutboxItem],
    pub outbox_unreadable: bool,
    pub closures: &'a [Transition],
    pub closures_unreadable: bool,
    pub workflows: &'a [Workflow],
    pub workflows_unreadable: bool,
    pub questions: &'a [Question],
    pub questions_unreadable: bool,
}

impl<'a> Sources<'a> {
    /// The stores `appraisal::Stores::load` already read, on its terms.
    pub fn of(stores: &'a crate::appraisal::Stores) -> Sources<'a> {
        Sources {
            drafts: &stores.drafts,
            outbox_unreadable: stores.outbox_unreadable,
            closures: &stores.closures,
            closures_unreadable: stores.closures_unreadable,
            workflows: &stores.workflows,
            workflows_unreadable: stores.workflows_unreadable,
            questions: &stores.questions,
            questions_unreadable: stores.questions_unreadable,
        }
    }
}

/// Where a session named by an owning record stands in the session store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seen {
    /// In the store, and in the population every corpus reader admits.
    Admitted,
    /// A smoke-test or experiment session (`runlog::Scan::admits` refuses it).
    Hidden,
    /// Not in the store: deleted, never recorded, or a header that did not
    /// read. The act still stands in its own store.
    Missing,
}

/// What the derivation asks of the session store.
pub trait SessionFacts {
    fn seen(&self, id: &str) -> Seen;
    /// Did the session's last run complete (`StopCause::Completed`)?
    /// `None` when that cannot be read. Asked for questions only, so a
    /// reader pays a transcript read per answered question and nothing
    /// for the other kinds.
    fn completed(&self, id: &str) -> Option<bool>;
}

/// The session store's headers, listed once, with the admission every
/// corpus reader applies.
pub struct SessionIndex {
    by_id: HashMap<String, (bool, PathBuf)>,
    /// Transcripts whose header could not be read (`list_counting`'s
    /// count). Each is `Missing` to [`SessionFacts::seen`] — a torn
    /// smoke-test session reads as not a test — so a non-zero count makes
    /// the set partial: the store is short, not whole (found on review).
    pub skipped: usize,
}

impl SessionIndex {
    /// `include_tests` lifts the smoke-test admission as `--include-tests`
    /// does on every corpus reader, so a readout that counts test sessions
    /// counts their successes too (found on review).
    pub fn load(dir: &Path, include_tests: bool) -> anyhow::Result<SessionIndex> {
        let scan = crate::runlog::Scan {
            include_tests,
            ..Default::default()
        };
        let (listed, skipped) = crate::session::Session::list_counting(dir)?;
        Ok(SessionIndex {
            skipped,
            by_id: listed
                .into_iter()
                .map(|(meta, path)| {
                    let admitted = scan.admits(&meta);
                    (meta.id, (admitted, path))
                })
                .collect(),
        })
    }
}

impl SessionFacts for SessionIndex {
    fn seen(&self, id: &str) -> Seen {
        match self.by_id.get(id) {
            None => Seen::Missing,
            Some((true, _)) => Seen::Admitted,
            Some((false, _)) => Seen::Hidden,
        }
    }

    fn completed(&self, id: &str) -> Option<bool> {
        let (_, path) = self.by_id.get(id)?;
        // The folded episode's stop cause, which `RunStats::merge` takes
        // from the last run — what the appraisal's question arm reads. An
        // absent cause is unknown — `lenient_stop_cause` degrades a variant
        // this build cannot name to it, and an outcome from before the
        // field has none — never "did not finish" (found on review).
        let stats = crate::session::Session::episode_stats(path)
            .ok()
            .flatten()?;
        Some(stats.stop_cause? == crate::agent::StopCause::Completed)
    }
}

fn parse_at(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Where a success's named sessions put it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// Kept: a named session is admitted, or none is named — a success with
    /// no session is not a test's for want of one.
    Kept,
    /// Every named session is a smoke-test or experiment session.
    Hidden,
    /// No named session is admitted and at least one is not in the store:
    /// whether it was a test cannot be read, so it is unknown — never kept.
    /// A torn header and a pruned transcript land here alike (found on
    /// review: a missing session failed open for three of the four kinds).
    Unplaced,
}

/// Why an unplaced success is unknown.
const UNPLACED: &str = "its session is not in the store, so whether it was a smoke test is unknown";

fn place(sessions: &[String], facts: &dyn SessionFacts) -> Place {
    let seen: Vec<Seen> = sessions.iter().map(|s| facts.seen(s)).collect();
    if seen.is_empty() || seen.contains(&Seen::Admitted) {
        Place::Kept
    } else if seen.iter().all(|s| *s == Seen::Hidden) {
        Place::Hidden
    } else {
        Place::Unplaced
    }
}

/// Count a success that is not kept; `true` when it was kept.
fn placed(out: &mut Successes, success: &Success, facts: &dyn SessionFacts) -> bool {
    match place(&success.sessions, facts) {
        Place::Kept => true,
        Place::Hidden => {
            out.hidden += 1;
            false
        }
        Place::Unplaced => {
            out.unknown.push(Unknown {
                pointer: success.act.pointer(),
                why: UNPLACED,
            });
            false
        }
    }
}

/// A model's message draft the owner sent as written.
fn sent_unchanged(item: &OutboxItem) -> bool {
    item.writing_outcome() == Some(WritingOutcome::SentUnchanged)
}

/// Derive the success set from the stores that own each act.
pub fn derive(src: &Sources<'_>, facts: &dyn SessionFacts) -> Successes {
    let mut out = Successes::default();
    for (name, short) in [
        ("outbox", src.outbox_unreadable),
        ("closure store", src.closures_unreadable),
        ("workflow store", src.workflows_unreadable),
        ("question store", src.questions_unreadable),
    ] {
        if short {
            out.unreadable.push(name);
        }
    }

    // --- Drafts sent unchanged, each with its exemplar ---
    for item in src.drafts.iter().filter(|i| sent_unchanged(i)) {
        let sent_at = item.resolved_at.as_deref().and_then(parse_at);
        let success = Success {
            act: Act::SentUnchanged {
                item: item.id.clone(),
            },
            sessions: item.session_id.iter().cloned().collect(),
            goal: None,
            at: sent_at,
        };
        if !placed(&mut out, &success, facts) {
            continue;
        }
        out.exemplars.push(Exemplar {
            item: item.id.clone(),
            tool: item.tool.clone(),
            args: item.args.clone(),
            session: item.session_id.clone(),
            // Through the door, keyed on the tool alone: the item's
            // `workspace` is the drafting jail, which is not the workspace
            // a run's rules block is matched on (`reflect`'s reason for
            // never stamping a jail), so it is not a key here.
            situation: Situation::of_run(std::slice::from_ref(&item.tool), None),
            origin: crate::learning::classify_origin(Some(item.taint)),
            sent_at,
        });
        out.standing.push(success);
    }

    // --- Tasks closed `done`, unless a recorded reopen undoes the closure ---
    //
    // Only a `done` closure is a success (`dropped` is the zero-signed
    // twin). A reopen names what it undoes; a reopen that names nothing
    // (a closure older than the record) has no success here to withdraw.
    for c in src.closures {
        if c.kind != Move::Close || c.to != "done" {
            continue;
        }
        let success = Success {
            act: Act::TaskDone {
                task: c.task.clone(),
                closure: c.id.clone(),
            },
            sessions: c.sessions.clone(),
            goal: Some(GoalRef::Task(c.task.clone())),
            at: Some(c.at),
        };
        if !placed(&mut out, &success, facts) {
            continue;
        }
        let reopen = src
            .closures
            .iter()
            .find(|r| r.kind == Move::Reopen && r.undoes.as_deref() == Some(c.id.as_str()));
        if let Some(r) = reopen {
            out.withdrawn.push(Withdrawn {
                success,
                by: format!("closure:{}", r.id),
                at: Some(r.at),
            });
            continue;
        }
        // Who closed it must be readable: the owner's hand or the owner's
        // approval. An actor this build cannot name is not an owner act it
        // can vouch for.
        if c.actor == Actor::Unknown {
            out.unknown.push(Unknown {
                pointer: success.act.pointer(),
                why: "closed by an actor this build cannot read",
            });
            continue;
        }
        out.standing.push(success);
    }

    // --- Workflows the owner closed, unless reopened ---
    for w in src.workflows {
        for d in w.owner_dispositions() {
            if d.kind != Disposition::Closed {
                continue;
            }
            let success = Success {
                act: Act::WorkflowClosed {
                    workflow: w.id.clone(),
                },
                sessions: d.session.iter().cloned().collect(),
                goal: w.task_id.clone().map(GoalRef::Task),
                at: Some(d.at),
            };
            if !placed(&mut out, &success, facts) {
                continue;
            }
            match d.reopened_at {
                // A `reopened` event carries no id of its own: named as the
                // workflow's reopen, never as the workflow itself, which
                // read as a workflow taking itself back (found on review).
                Some(at) => out.withdrawn.push(Withdrawn {
                    success,
                    by: format!("workflow:{}#reopened", w.id),
                    at: Some(at),
                }),
                None => out.standing.push(success),
            }
        }
    }

    // --- Questions answered, whose session then completed ---
    for q in src
        .questions
        .iter()
        .filter(|q| q.status == crate::questions::ANSWERED)
    {
        let act = Act::QuestionAnswered {
            question: q.id.clone(),
        };
        match facts.seen(&q.session_id) {
            Seen::Hidden => {
                out.hidden += 1;
                continue;
            }
            Seen::Missing => {
                out.unknown.push(Unknown {
                    pointer: act.pointer(),
                    why: "its session is not in the store, so whether it finished is unknown",
                });
                continue;
            }
            Seen::Admitted => {}
        }
        match facts.completed(&q.session_id) {
            Some(true) => out.standing.push(Success {
                act,
                sessions: vec![q.session_id.clone()],
                goal: q.task_id.clone().map(GoalRef::Task),
                at: q.answered_at.as_deref().and_then(parse_at),
            }),
            // Answered, and the work did not finish (yet): asking was not
            // shown to be the right call. Nothing, as in the appraisal.
            Some(false) => {}
            None => out.unknown.push(Unknown {
                pointer: act.pointer(),
                why: "its session's outcome could not be read",
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DANA: &str = "20260920T090000-dana";

    /// A session store answered from a table: sessions not named are
    /// missing; `completed` answers from the table's second column.
    #[derive(Default)]
    struct Facts(HashMap<String, (Seen, Option<bool>)>);

    impl Facts {
        fn with(mut self, id: &str, seen: Seen, completed: Option<bool>) -> Self {
            self.0.insert(id.into(), (seen, completed));
            self
        }
    }

    impl SessionFacts for Facts {
        fn seen(&self, id: &str) -> Seen {
            self.0.get(id).map(|f| f.0).unwrap_or(Seen::Missing)
        }
        fn completed(&self, id: &str) -> Option<bool> {
            self.0.get(id).and_then(|f| f.1)
        }
    }

    fn draft(id: &str, status: &str, edited: bool) -> OutboxItem {
        let before = json!({
            "to": "sam@example.edu",
            "subject": "Lakeside Institute visit",
            "body_markdown": "Hi Sam,\n\nThursday at 2pm works for the Lakeside visit.\n\nDana",
        });
        let mut item: OutboxItem = serde_json::from_value(json!({
            "id": id, "status": status, "tool": "mail_send", "kind": "message",
            "args_before": before, "args": before, "summary": "a reply",
            "session_id": DANA, "created_at": "2026-09-20T09:00:00Z",
            "resolved_at": "2026-09-20T10:00:00Z",
        }))
        .unwrap();
        if edited {
            item.args = json!({
                "to": "sam@example.edu",
                "subject": "Lakeside Institute visit",
                "body_markdown": "Sam — Thursday 2pm is good. Dana",
            });
        }
        item
    }

    fn closure(id: &str, task: &str, to: &str, kind: &str, undoes: Option<&str>) -> Transition {
        serde_json::from_value(json!({
            "id": id, "task": task, "from": if kind == "close" { "next" } else { to },
            "to": if kind == "close" { to } else { "next" }, "move": kind,
            "actor": "owner", "surface": "cli", "sessions": [DANA],
            "at": "2026-09-21T12:00:00Z", "undoes": undoes,
        }))
        .unwrap()
    }

    fn workflow(id: &str, reopened: bool) -> Workflow {
        let now = Utc::now();
        let mut w = Workflow::new(id.into(), "fixture".into(), PathBuf::from("/tmp"), now);
        w.task_id = Some("task-northwind-report".into());
        w.record("started", DANA, now);
        w.record("owner_closed", "Owner closed after verification", now);
        if reopened {
            w.record("reopened", "Owner reopened workflow", now);
        }
        w
    }

    fn question(id: &str, status: &str) -> Question {
        serde_json::from_value(json!({
            "id": id, "status": status, "question": "Which Northwind Labs contact?",
            "session_id": DANA, "asked_at": "2026-09-20T09:00:00Z",
            "answered_at": "2026-09-20T09:30:00Z", "answer": "Dana Whitfield",
        }))
        .unwrap()
    }

    fn admitted() -> Facts {
        Facts::default().with(DANA, Seen::Admitted, Some(true))
    }

    /// Row 2e-4's first acceptance: a draft sent unchanged is mined as an
    /// exemplar — verbatim, no model — and only that. An edited draft is
    /// the correction `reflect` already mines; a rejected one never went
    /// out; one the harness authored says nothing about a model's writing.
    #[test]
    fn a_draft_sent_unchanged_is_an_exemplar_verbatim_and_nothing_else_is() {
        let unchanged = draft("ob-kept", "sent", false);
        let edited = draft("ob-edited", "sent", true);
        let rejected = draft("ob-rejected", "rejected", false);
        let pending = draft("ob-pending", "pending", false);
        let mut harness = draft("ob-poll", "sent", false);
        harness.author = "harness".into();
        let drafts = vec![unchanged.clone(), edited, rejected, pending, harness];
        let set = derive(
            &Sources {
                drafts: &drafts,
                ..Default::default()
            },
            &admitted(),
        );
        assert_eq!(set.exemplars.len(), 1, "{:?}", set.exemplars);
        let e = &set.exemplars[0];
        assert_eq!(e.item, "ob-kept");
        assert_eq!(e.tool, "mail_send");
        assert_eq!(e.args, unchanged.args_before, "the draft itself, verbatim");
        assert_eq!(e.situation.tools, vec!["mail_send".to_string()]);
        assert_eq!(e.situation.trigger, None);
        assert_eq!(e.origin, Origin::Clean);
        assert_eq!(e.session.as_deref(), Some(DANA));
        assert_eq!(
            set.standing
                .iter()
                .map(|s| s.act.pointer())
                .collect::<Vec<_>>(),
            vec!["outbox:ob-kept"]
        );
    }

    /// Provenance is the staging taint's, fail-closed: a draft written with
    /// third-party text in context is kept as an exemplar and labelled
    /// untrusted, never clean.
    #[test]
    fn an_exemplar_staged_under_taint_is_untrusted() {
        let mut d = draft("ob-armed", "sent", false);
        d.taint.untrusted = true;
        let drafts = vec![d];
        let set = derive(
            &Sources {
                drafts: &drafts,
                ..Default::default()
            },
            &admitted(),
        );
        assert_eq!(set.exemplars[0].origin, Origin::Untrusted);
    }

    /// Row 2e-4's second acceptance: a success the owner later reopens is
    /// withdrawn. A `done` closure stands; the same closure beside a reopen
    /// that undoes it is withdrawn, named by the reopen, and in no standing
    /// count. A second `done` closure after the reopen stands on its own.
    #[test]
    fn a_task_the_owner_reopens_withdraws_its_success() {
        let closed = vec![closure("cl-1", "task-a", "done", "close", None)];
        let set = derive(
            &Sources {
                closures: &closed,
                ..Default::default()
            },
            &admitted(),
        );
        assert_eq!(set.standing.len(), 1);
        assert_eq!(set.standing[0].goal, Some(GoalRef::Task("task-a".into())));
        assert!(set.withdrawn.is_empty());

        let reopened = vec![
            closure("cl-1", "task-a", "done", "close", None),
            closure("cl-2", "task-a", "done", "reopen", Some("cl-1")),
        ];
        let set = derive(
            &Sources {
                closures: &reopened,
                ..Default::default()
            },
            &admitted(),
        );
        assert!(set.standing.is_empty(), "{:?}", set.standing);
        assert_eq!(set.withdrawn.len(), 1);
        assert_eq!(set.withdrawn[0].by, "closure:cl-2");
        assert_eq!(set.withdrawn[0].success.act.pointer(), "closure:cl-1");

        let reclosed = vec![
            closure("cl-1", "task-a", "done", "close", None),
            closure("cl-2", "task-a", "done", "reopen", Some("cl-1")),
            closure("cl-3", "task-a", "done", "close", None),
        ];
        let set = derive(
            &Sources {
                closures: &reclosed,
                ..Default::default()
            },
            &admitted(),
        );
        assert_eq!(
            set.standing
                .iter()
                .map(|s| s.act.pointer())
                .collect::<Vec<_>>(),
            vec!["closure:cl-3"]
        );
        assert_eq!(set.withdrawn.len(), 1);
    }

    /// A `dropped` closure is no success; a closure whose actor this build
    /// cannot read is unknown, never counted.
    #[test]
    fn a_drop_is_no_success_and_an_unread_actor_is_unknown() {
        let mut unread = closure("cl-9", "task-b", "done", "close", None);
        unread.actor = Actor::Unknown;
        let closures = vec![closure("cl-8", "task-a", "dropped", "close", None), unread];
        let set = derive(
            &Sources {
                closures: &closures,
                ..Default::default()
            },
            &admitted(),
        );
        assert!(set.standing.is_empty());
        assert_eq!(set.unknown.len(), 1);
        assert_eq!(set.unknown[0].pointer, "closure:cl-9");
    }

    /// The workflow twin of the closure: a close stands, a reopened close
    /// is withdrawn.
    #[test]
    fn a_workflow_the_owner_reopens_withdraws_its_success() {
        let workflows = vec![workflow("wf-kept", false), workflow("wf-back", true)];
        let set = derive(
            &Sources {
                workflows: &workflows,
                ..Default::default()
            },
            &admitted(),
        );
        assert_eq!(
            set.standing
                .iter()
                .map(|s| s.act.pointer())
                .collect::<Vec<_>>(),
            vec!["workflow:wf-kept"]
        );
        assert_eq!(
            set.standing[0].goal,
            Some(GoalRef::Task("task-northwind-report".into()))
        );
        assert_eq!(set.withdrawn.len(), 1);
        assert_eq!(set.withdrawn[0].by, "workflow:wf-back#reopened");
    }

    /// A question answered is a success only once its session completed;
    /// an outcome that cannot be read, or a session not in the store, is
    /// unknown — never a success, never nothing.
    #[test]
    fn an_answered_question_counts_only_when_its_session_completed() {
        let questions = vec![question("q-1", "answered"), question("q-2", "abandoned")];
        let src = Sources {
            questions: &questions,
            ..Default::default()
        };
        let set = derive(&src, &admitted());
        assert_eq!(
            set.standing
                .iter()
                .map(|s| s.act.pointer())
                .collect::<Vec<_>>(),
            vec!["question:q-1"]
        );

        let unfinished = Facts::default().with(DANA, Seen::Admitted, Some(false));
        let set = derive(&src, &unfinished);
        assert!(set.standing.is_empty() && set.unknown.is_empty());

        let unreadable = Facts::default().with(DANA, Seen::Admitted, None);
        let set = derive(&src, &unreadable);
        assert!(set.standing.is_empty());
        assert_eq!(set.unknown.len(), 1);

        let set = derive(&src, &Facts::default());
        assert!(set.standing.is_empty());
        assert_eq!(set.unknown.len(), 1, "a missing session is unknown");
    }

    /// Smoke-test and experiment sessions are the harness measuring itself:
    /// counted as hidden and in no standing count, withdrawn count or
    /// exemplar.
    #[test]
    fn a_success_in_a_test_session_is_hidden() {
        let drafts = vec![draft("ob-test", "sent", false)];
        let closures = vec![closure("cl-t", "task-t", "done", "close", None)];
        let questions = vec![question("q-t", "answered")];
        let hidden = Facts::default().with(DANA, Seen::Hidden, Some(true));
        let set = derive(
            &Sources {
                drafts: &drafts,
                closures: &closures,
                questions: &questions,
                ..Default::default()
            },
            &hidden,
        );
        assert!(set.standing.is_empty() && set.exemplars.is_empty());
        assert_eq!(set.hidden, 3);
    }

    /// The session store's own answers: a smoke-test session is hidden, an
    /// ordinary one admitted, and `completed` is the recorded stop cause of
    /// the last run — `None` for a session that recorded no outcome.
    #[test]
    fn the_session_index_reads_admission_and_the_last_runs_stop_cause() {
        use crate::agent::StopCause;
        use crate::session::{Record, RunStats, SessionKind, SessionMeta};
        let root = crate::mismatch::Workspace::new().unwrap();
        let write = |id: &str, kind: SessionKind, stops: &[StopCause]| {
            let mut records = vec![Record::Meta(SessionMeta {
                id: id.into(),
                created_at: Utc::now(),
                provider: "scripted".into(),
                model: "scripted".into(),
                workspace: root.path().into(),
                title: None,
                kind: Some(kind),
            })];
            for stop in stops {
                records.push(Record::Outcome(RunStats {
                    stop_cause: Some(*stop),
                    ..Default::default()
                }));
            }
            let text = records
                .iter()
                .map(|r| serde_json::to_string(r).unwrap())
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(root.path().join(format!("{id}.jsonl")), text).unwrap();
        };
        write(
            "s-resumed",
            SessionKind::Chat,
            &[StopCause::Interrupted, StopCause::Completed],
        );
        write("s-parked", SessionKind::Chat, &[StopCause::Interrupted]);
        write("s-silent", SessionKind::Chat, &[]);
        // An outcome whose stop cause this build cannot name: the lenient
        // read makes it absent, which is unknown, not "did not finish".
        write("s-newer", SessionKind::Chat, &[]);
        let newer = root.path().join("s-newer.jsonl");
        let mut text = std::fs::read_to_string(&newer).unwrap();
        text.push('\n');
        // `Record` is internally tagged, so the cause sits beside the tag.
        let mut outcome = serde_json::to_value(Record::Outcome(RunStats::default())).unwrap();
        outcome["stop_cause"] = json!("a_cause_from_a_newer_build");
        text.push_str(&outcome.to_string());
        std::fs::write(&newer, text).unwrap();
        write("s-smoke", SessionKind::Test, &[StopCause::Completed]);
        let index = SessionIndex::load(root.path(), false).unwrap();
        assert_eq!(index.seen("s-resumed"), Seen::Admitted);
        assert_eq!(index.seen("s-smoke"), Seen::Hidden);
        assert_eq!(index.seen("s-gone"), Seen::Missing);
        let with_tests = SessionIndex::load(root.path(), true).unwrap();
        assert_eq!(with_tests.seen("s-smoke"), Seen::Admitted);
        assert_eq!(index.completed("s-resumed"), Some(true));
        assert_eq!(index.completed("s-parked"), Some(false));
        assert_eq!(index.completed("s-silent"), None);
        assert_eq!(index.completed("s-newer"), None);
        assert_eq!(index.skipped, 0);
        // A transcript whose header does not read is counted, not forgotten.
        std::fs::write(root.path().join("s-torn.jsonl"), "{\"kind\":\"me").unwrap();
        assert_eq!(SessionIndex::load(root.path(), false).unwrap().skipped, 1);
        assert_eq!(index.completed("s-gone"), None);
    }

    /// A named session the store does not hold — pruned, or a header that
    /// did not read — cannot be told from a smoke test, so its success is
    /// unknown, never kept; one naming no session at all is kept, and one
    /// naming an admitted session beside a missing one stands.
    #[test]
    fn a_success_whose_session_is_missing_is_unknown() {
        let drafts = vec![draft("ob-gone", "sent", false)];
        let closures = vec![closure("cl-gone", "task-g", "done", "close", None)];
        let workflows = vec![workflow("wf-gone", false)];
        let src = Sources {
            drafts: &drafts,
            closures: &closures,
            workflows: &workflows,
            ..Default::default()
        };
        let set = derive(&src, &Facts::default());
        assert!(
            set.standing.is_empty() && set.exemplars.is_empty(),
            "{set:?}"
        );
        assert_eq!(set.unknown.len(), 3);
        assert!(set.unknown.iter().all(|u| u.why == UNPLACED));

        let mut unnamed = draft("ob-unnamed", "sent", false);
        unnamed.session_id = None;
        let mut shared = closure("cl-shared", "task-s", "done", "close", None);
        shared.sessions = vec!["s-pruned".into(), DANA.into()];
        let drafts = vec![unnamed];
        let closures = vec![shared];
        let set = derive(
            &Sources {
                drafts: &drafts,
                closures: &closures,
                ..Default::default()
            },
            &admitted(),
        );
        assert_eq!(set.standing.len(), 2, "{set:?}");
        assert!(set.unknown.is_empty());
    }

    /// A store that could not be read makes the set partial, by name —
    /// short, not empty.
    #[test]
    fn an_unreadable_store_makes_the_set_partial() {
        let set = derive(
            &Sources {
                closures_unreadable: true,
                ..Default::default()
            },
            &admitted(),
        );
        assert!(set.partial());
        assert_eq!(set.unreadable, vec!["closure store"]);
        assert!(!derive(&Sources::default(), &admitted()).partial());
    }
}
