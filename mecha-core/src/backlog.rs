//! What is waiting on the owner, across every store that accumulates.
//!
//! Five stores collect work for a person and each grew its own verb, which is
//! how the knowledge graph's merge queue reached 6,434 items without anybody
//! deciding to let it. `mecha review` answers "what is waiting" for a human;
//! `doctor` answers "what is silently wrong"; and the goal system needs a
//! third answer — *how much does this run owe the owner, and did it just make
//! that worse* (`docs/GOAL-SYSTEM-DESIGN.md` §4).
//!
//! Three questions, one walk. This module is the walk. It computes nothing
//! about health and renders nothing for a screen: it counts what is waiting
//! and how long the oldest has waited, and every reader on top decides what
//! that means. Same division `runlog` keeps — **the module counts and never
//! judges**, because what counts as too much depends on who is asking.
//!
//! ## Two absences that are not the same, and never collapse
//!
//! - **`None` means the store could not be read.** Not "nothing is waiting".
//!   Those are opposite findings, and a reader that renders the second as the
//!   first reproduces exactly the bug the unified queue exists to catch.
//! - **A store that does not exist yet is genuinely empty** — `Some(0)`, not
//!   `None`. A machine that has never delegated a task has no question store,
//!   and reporting that as unreadable would make it indistinguishable from one
//!   whose store is broken.
//!
//! And a partial read stays partial. [`Backlog::waiting`] returns the sum of
//! what it could read *beside* the number of stores it could not, rather than
//! choosing between understating the total and discarding it. A caller that
//! needs to know whether the number is complete can see that it is not.
//!
//! **The graph's queues are deliberately absent.** Reaching them needs a
//! `mecha-graph` subprocess, which is fine once a night and far too expensive
//! in the path of every run. `mecha review` adds them on top for its own view.

use crate::frontdoor::{self, Frontdoor};
use crate::harness::HarnessStore;
use crate::learning::LearningStore;
use crate::outbox::OutboxStore;
use crate::questions::QuestionStore;
use serde::{Deserialize, Serialize};

/// One store's contribution: how much waits, and since when.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Depth {
    pub waiting: usize,
    /// RFC3339 stamp of the oldest still-waiting item. `None` when nothing is
    /// waiting — an absent age, never a zero one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest: Option<String>,
    /// Rows that stopped waiting because the owner *gave up* on them — a
    /// rejected draft, an abandoned question, a request closed with nothing
    /// sent — rather than because the commitment was kept. `waiting` falls
    /// on both, so a delta over `waiting` alone read the owner throwing a
    /// commitment away as the run having shortened the queue, and signed
    /// the same act positive in one arm and negative in another (found on
    /// review). Zero on the harness's own two queues, which nobody outside
    /// is owed. Absent on a row written before the counter, which reads as
    /// zero — an old row can only over-credit, never sign a new positive.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub given_up: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl Depth {
    fn of<'a>(waiting: usize, stamps: impl IntoIterator<Item = &'a str>) -> Depth {
        Depth {
            waiting,
            oldest: stamps.into_iter().min().map(str::to_string),
            given_up: 0,
        }
    }

    fn given_up(mut self, n: usize) -> Depth {
        self.given_up = n;
        self
    }
}

/// Everything waiting on the owner in mecha's own stores.
///
/// Each field is `None` when that store could not be read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backlog {
    pub outbox: Option<Depth>,
    pub questions: Option<Depth>,
    pub frontdoor: Option<Depth>,
    pub proposals: Option<Depth>,
    pub candidates: Option<Depth>,
}

/// A total, and how much of it is missing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Waiting {
    /// Summed over the stores that could be read.
    pub total: usize,
    /// How many stores could not be, so a caller knows the total is partial.
    pub unreadable: usize,
}

impl Backlog {
    /// Read every mecha-owned store. Best-effort per store, like doctor: one
    /// unreadable store never suppresses the other four.
    pub fn read() -> Backlog {
        Backlog {
            outbox: Self::read_outbox(),
            questions: Self::read_questions(),
            frontdoor: Self::read_frontdoor(),
            proposals: Self::read_proposals(),
            candidates: Self::read_candidates(),
        }
    }

    fn read_outbox() -> Option<Depth> {
        let store = OutboxStore::default_root()
            .and_then(OutboxStore::open)
            .ok()?;
        let items = store.items().ok()?;
        let pending: Vec<_> = items.iter().filter(|i| i.status == "pending").collect();
        let rejected = items.iter().filter(|i| i.status == "rejected").count();
        Some(
            Depth::of(pending.len(), pending.iter().map(|i| i.created_at.as_str()))
                .given_up(rejected),
        )
    }

    fn read_questions() -> Option<Depth> {
        // A store that has never existed is empty, not unreadable.
        let Some(store) = QuestionStore::open_existing_default() else {
            return Some(Depth::default());
        };
        let items = store.items().ok()?;
        let open: Vec<_> = items.iter().filter(|q| q.is_open()).collect();
        let abandoned = items
            .iter()
            .filter(|q| q.status == crate::questions::ABANDONED)
            .count();
        Some(Depth::of(open.len(), open.iter().map(|q| q.asked_at.as_str())).given_up(abandoned))
    }

    fn read_frontdoor() -> Option<Depth> {
        let records = Frontdoor::open_default().and_then(|s| s.records()).ok()?;
        let open: Vec<_> = records
            .iter()
            .filter(|r| r.state != frontdoor::CLOSED)
            .collect();
        // Closed with nothing ever staged for it — the request arm's own
        // reading of a give-up. A request closed after its draft was
        // rejected counts once, on the outbox.
        let closed_unsent = records
            .iter()
            .filter(|r| r.state == frontdoor::CLOSED && r.outbox.is_empty())
            .count();
        Some(
            Depth::of(open.len(), open.iter().map(|r| r.created_at.as_str()))
                .given_up(closed_unsent),
        )
    }

    fn read_proposals() -> Option<Depth> {
        let store = LearningStore::default_root()
            .and_then(LearningStore::open)
            .ok()?;
        let proposals = store.proposals().ok()?;
        let pending: Vec<_> = proposals.iter().filter(|p| p.status == "pending").collect();
        Some(Depth::of(
            pending.len(),
            pending.iter().map(|p| p.created_at.as_str()),
        ))
    }

    fn read_candidates() -> Option<Depth> {
        let candidates = HarnessStore::open_default().and_then(|s| s.all()).ok()?;
        let staged: Vec<_> = candidates.iter().filter(|c| c.pending()).collect();
        Some(Depth::of(
            staged.len(),
            staged.iter().map(|c| c.created_at.as_str()),
        ))
    }

    fn depths(&self) -> [&Option<Depth>; 5] {
        [
            &self.outbox,
            &self.questions,
            &self.frontdoor,
            &self.proposals,
            &self.candidates,
        ]
    }

    /// How much is waiting, and over how many stores that could not be read.
    pub fn waiting(&self) -> Waiting {
        let mut out = Waiting::default();
        for depth in self.depths() {
            match depth {
                Some(d) => out.total += d.waiting,
                None => out.unreadable += 1,
            }
        }
        out
    }

    /// The oldest still-waiting stamp anywhere, RFC3339.
    ///
    /// The signal behind *"never leave a person waiting"*: a queue of one item
    /// nine days old is a different failure from nine items an hour old, and a
    /// count alone cannot tell them apart.
    pub fn oldest(&self) -> Option<&str> {
        self.depths()
            .into_iter()
            .flatten()
            .filter_map(|d| d.oldest.as_deref())
            .min()
    }

    /// What this run changed, per store.
    ///
    /// `None` for a store unreadable at either end — a delta against an
    /// unknown is not zero.
    pub fn delta(before: &Backlog, after: &Backlog) -> BacklogDelta {
        let d = |a: &Option<Depth>, b: &Option<Depth>| match (a, b) {
            (Some(a), Some(b)) => Some(b.waiting as i64 - a.waiting as i64),
            _ => None,
        };
        // Give-ups over the three owner-facing stores, summed — `Some`
        // wherever any of the three was readable at both ends, on the same
        // rule as `owner_facing_net`.
        let g = |a: &Option<Depth>, b: &Option<Depth>| match (a, b) {
            (Some(a), Some(b)) => Some(b.given_up as i64 - a.given_up as i64),
            _ => None,
        };
        let given_up: Vec<i64> = [
            g(&before.outbox, &after.outbox),
            g(&before.questions, &after.questions),
            g(&before.frontdoor, &after.frontdoor),
        ]
        .into_iter()
        .flatten()
        .collect();
        BacklogDelta {
            outbox: d(&before.outbox, &after.outbox),
            questions: d(&before.questions, &after.questions),
            frontdoor: d(&before.frontdoor, &after.frontdoor),
            proposals: d(&before.proposals, &after.proposals),
            candidates: d(&before.candidates, &after.candidates),
            given_up: (!given_up.is_empty()).then(|| given_up.iter().sum()),
        }
    }
}

/// What one run added to, or took off, the owner's plate.
///
/// **The appraisal-relevant quantity, and the reason a level alone will not
/// do.** A run that stages nine drafts raises the outbox's depth by nine; read
/// as a level at run end, its own output is indistinguishable from a backlog
/// it inherited. The question the goal system asks — *did this run leave the
/// owner better or worse off* — is answerable only from the difference.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BacklogDelta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outbox: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub questions: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontdoor: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposals: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidates: Option<i64>,
    /// How many more rows the owner had given up on at the end than at the
    /// start, over the three owner-facing stores (`Depth::given_up`).
    /// `None` on a row written before the counter, or where none of the
    /// three could be read at both ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub given_up: Option<i64>,
}

impl BacklogDelta {
    /// Two runs' deltas, summed per store — `Some` wherever either run
    /// could read the store, on the `Option` fold `RunStats::merge` uses
    /// for every per-run counter. An episode's change to the queue is the
    /// sum of its runs' changes, which is what makes a session-scoped
    /// reading of this honest where the level beside it stays the first
    /// run's (a level is a condition, a delta is an act).
    pub fn plus(&self, other: &BacklogDelta) -> BacklogDelta {
        let f = |a: Option<i64>, b: Option<i64>| match (a, b) {
            (Some(a), Some(b)) => Some(a + b),
            (a, b) => a.or(b),
        };
        BacklogDelta {
            outbox: f(self.outbox, other.outbox),
            questions: f(self.questions, other.questions),
            frontdoor: f(self.frontdoor, other.frontdoor),
            proposals: f(self.proposals, other.proposals),
            candidates: f(self.candidates, other.candidates),
            given_up: f(self.given_up, other.given_up),
        }
    }

    /// How many owner-facing rows this run *cleared* — at least. The queue
    /// shrinks when a commitment is kept and when the owner gives one up,
    /// and only the first is the run's doing, so the give-ups are taken
    /// off the fall before it is credited: a run whose window held one
    /// abandoned question and a one-row fall cleared nothing it can claim.
    /// A lower bound, because rows the run *added* inside the window hide
    /// clearances one-for-one, and the honest direction for a positive is
    /// under. `None` where [`owner_facing_net`](Self::owner_facing_net) is;
    /// an absent give-up count (a row from before it) reads as zero.
    pub fn owner_facing_cleared(&self) -> Option<u64> {
        let net = self.owner_facing_net()?;
        let given_up = self.given_up.unwrap_or(0).max(0);
        Some((-net - given_up).max(0) as u64)
    }

    /// Net change across the three stores somebody *outside* is waiting on
    /// — the outbox, the questions, the front door — which are exactly the
    /// stores `guilt::anticipated_guilt` reads. `proposals` and `candidates`
    /// are the harness's own review queue, owed to nobody outside it: a
    /// rumination run that accepted five candidates while five drafts sat
    /// unsent read as full relief through [`net`](Self::net), and the
    /// appraisal signed it as having shortened the owner's queue (found on
    /// review). `None` when none of the three could be read at both ends.
    pub fn owner_facing_net(&self) -> Option<i64> {
        let seen: Vec<i64> = [self.outbox, self.questions, self.frontdoor]
            .into_iter()
            .flatten()
            .collect();
        (!seen.is_empty()).then(|| seen.iter().sum())
    }

    /// Net change across the stores that could be read at both ends.
    ///
    /// `None` when none could — not zero, which would read as "this run
    /// changed nothing" when the truth is that nobody looked.
    pub fn net(&self) -> Option<i64> {
        let seen: Vec<i64> = [
            self.outbox,
            self.questions,
            self.frontdoor,
            self.proposals,
            self.candidates,
        ]
        .into_iter()
        .flatten()
        .collect();
        (!seen.is_empty()).then(|| seen.iter().sum())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn depth(waiting: usize, oldest: Option<&str>) -> Option<Depth> {
        Some(Depth {
            waiting,
            oldest: oldest.map(str::to_string),
            given_up: 0,
        })
    }

    /// The bug the unified queue exists to catch, one layer down. A reader
    /// that folded an unreadable store into the total would report a broken
    /// store as a quiet one.
    #[test]
    fn an_unreadable_store_is_counted_as_unread_and_never_as_empty() {
        let b = Backlog {
            outbox: depth(3, Some("2026-08-20T09:00:00Z")),
            questions: None, // could not read
            frontdoor: depth(0, None),
            proposals: depth(1, Some("2026-08-25T09:00:00Z")),
            candidates: None, // could not read
        };
        assert_eq!(
            b.waiting(),
            Waiting {
                total: 4,
                unreadable: 2
            },
            "the total is what was readable, and says so"
        );
    }

    #[test]
    fn a_store_with_nothing_waiting_has_no_oldest_age() {
        let empty = Depth::of(0, Vec::<&str>::new());
        assert_eq!(empty.waiting, 0);
        assert_eq!(empty.oldest, None, "an absent age, never a zero one");
    }

    /// A count cannot tell one nine-day-old request from nine one-hour-old
    /// ones, and only the first is the failure the charter cares about.
    #[test]
    fn the_oldest_wait_is_the_earliest_stamp_across_every_store() {
        let b = Backlog {
            outbox: depth(2, Some("2026-08-25T09:00:00Z")),
            questions: depth(1, Some("2026-08-17T09:00:00Z")),
            frontdoor: depth(0, None),
            proposals: None,
            candidates: depth(1, Some("2026-08-26T09:00:00Z")),
        };
        assert_eq!(b.oldest(), Some("2026-08-17T09:00:00Z"));
        assert_eq!(Backlog::default().oldest(), None);
    }

    /// The appraisal-relevant quantity. A run that stages nine drafts raises
    /// the outbox by nine; a level at run end cannot separate that from a
    /// backlog it inherited.
    #[test]
    fn a_delta_reports_what_this_run_added_rather_than_what_it_found() {
        let before = Backlog {
            outbox: depth(2, None),
            questions: depth(1, None),
            frontdoor: depth(4, None),
            proposals: None,
            candidates: depth(0, None),
        };
        let after = Backlog {
            outbox: depth(11, None),   // the run staged nine
            questions: depth(0, None), // and one got answered
            frontdoor: depth(4, None),
            proposals: depth(2, None), // unreadable before, so unknown
            candidates: None,          // unreadable after, so unknown
        };
        let d = Backlog::delta(&before, &after);
        assert_eq!(d.outbox, Some(9));
        assert_eq!(d.questions, Some(-1));
        assert_eq!(d.frontdoor, Some(0), "readable and genuinely unchanged");
        assert_eq!(d.proposals, None, "a delta against an unknown is not zero");
        assert_eq!(d.candidates, None);
        assert_eq!(d.net(), Some(8));
    }

    /// "This run changed nothing" and "nobody could look" are opposite
    /// findings, and the second must not render as the first.
    #[test]
    fn a_net_over_nothing_readable_is_absent_rather_than_zero() {
        assert_eq!(BacklogDelta::default().net(), None);
        assert_eq!(
            BacklogDelta {
                outbox: Some(0),
                ..BacklogDelta::default()
            }
            .net(),
            Some(0),
            "a real zero is a different answer and stays one"
        );
    }

    #[test]
    fn the_owner_facing_net_ignores_the_harnesss_own_queues() {
        let d = BacklogDelta {
            outbox: Some(-1),
            questions: Some(0),
            frontdoor: None,
            proposals: Some(-3),
            candidates: Some(-5),
            given_up: None,
        };
        assert_eq!(d.net(), Some(-9));
        assert_eq!(d.owner_facing_net(), Some(-1));
        let only_harness = BacklogDelta {
            candidates: Some(-5),
            ..Default::default()
        };
        assert_eq!(
            only_harness.owner_facing_net(),
            None,
            "nothing owner-facing was read"
        );
    }

    /// A give-up shrinks the queue exactly as a kept commitment does, and
    /// only the second is clearance (found on review): the run gets credit
    /// for the fall *net of* the rows the owner gave up on.
    #[test]
    fn a_give_up_is_not_clearance() {
        let before = Backlog {
            outbox: depth(2, None),
            questions: depth(1, None),
            frontdoor: depth(0, None),
            ..Default::default()
        };
        // One question abandoned, one draft sent.
        let after = Backlog {
            outbox: depth(1, None),
            questions: depth(0, None).map(|d| d.given_up(1)),
            frontdoor: depth(0, None),
            ..Default::default()
        };
        let d = Backlog::delta(&before, &after);
        assert_eq!(d.owner_facing_net(), Some(-2));
        assert_eq!(d.given_up, Some(1));
        assert_eq!(
            d.owner_facing_cleared(),
            Some(1),
            "the sent draft, not the abandoned question"
        );
        // Only the give-up: nothing to claim.
        let only_given_up = Backlog {
            outbox: depth(2, None),
            questions: depth(0, None).map(|d| d.given_up(1)),
            frontdoor: depth(0, None),
            ..Default::default()
        };
        let d = Backlog::delta(&before, &only_given_up);
        assert_eq!(d.owner_facing_net(), Some(-1));
        assert_eq!(d.owner_facing_cleared(), Some(0));
        // A row from before the counter reads the fall as clearance — the
        // old behaviour, over-crediting rather than inventing.
        let old = BacklogDelta {
            questions: Some(-1),
            ..Default::default()
        };
        assert_eq!(old.owner_facing_cleared(), Some(1));
        // Unknown stays unknown.
        assert_eq!(BacklogDelta::default().owner_facing_cleared(), None);
        // The wire form omits a zero give-up count, so an old reader sees
        // the row it always did.
        let json = serde_json::to_string(&before.outbox).unwrap();
        assert!(!json.contains("given_up"), "{json}");
        let d: Depth = serde_json::from_str(r#"{"waiting":3}"#).unwrap();
        assert_eq!(d.given_up, 0);
    }
}
