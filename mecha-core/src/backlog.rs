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
use std::collections::HashSet;

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
        // The outbox is read once and its sent ids handed to the front
        // door's reader: whether a closed request was a give-up is a join
        // (below), and the join must not see a different outbox than the
        // outbox depth was counted from.
        let outbox_items = Self::read_outbox_items();
        let sent: Option<HashSet<&str>> = outbox_items.as_ref().map(|items| {
            items
                .iter()
                .filter(|i| i.status == "sent")
                .map(|i| i.id.as_str())
                .collect()
        });
        Backlog {
            outbox: outbox_items.as_deref().map(Self::outbox_depth),
            questions: Self::read_questions(),
            frontdoor: Self::read_frontdoor(sent.as_ref()),
            proposals: Self::read_proposals(),
            candidates: Self::read_candidates(),
        }
    }

    fn read_outbox_items() -> Option<Vec<crate::outbox::OutboxItem>> {
        // An outbox that has never existed is empty, not unreadable, and a
        // reader must not create it — the same rule as the two readers
        // below (found on review).
        let Some(store) = OutboxStore::open_existing_default() else {
            return Some(Vec::new());
        };
        store.items().ok()
    }

    fn outbox_depth(items: &[crate::outbox::OutboxItem]) -> Depth {
        let pending: Vec<_> = items.iter().filter(|i| i.status == "pending").collect();
        let rejected = items.iter().filter(|i| i.status == "rejected").count();
        Depth::of(pending.len(), pending.iter().map(|i| i.created_at.as_str())).given_up(rejected)
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

    fn read_frontdoor(sent: Option<&HashSet<&str>>) -> Option<Depth> {
        // Like `read_questions`: a front door that has never existed is
        // empty, not unreadable — and `open_default` would *create* it,
        // twice per run at both ends of `Homeostat::finish`, and read a
        // failed creation as an unknown depth (found on review).
        let Some(store) = Frontdoor::open_existing_default() else {
            return Some(Depth::default());
        };
        let records = store.records().ok()?;
        let open: Vec<_> = records
            .iter()
            .filter(|r| r.state != frontdoor::CLOSED)
            .collect();
        Some(
            Depth::of(open.len(), open.iter().map(|r| r.created_at.as_str()))
                .given_up(Self::frontdoor_given_up(&records, sent)),
        )
    }

    /// Closed requests the owner gave up on: closed, and no draft ever
    /// staged for the request was sent. `waiting` leaves on exactly one
    /// transition, `→ closed`, so the give-up predicate has to match every
    /// close that was not an answer, not only the closes with nothing
    /// staged — a request whose draft was rejected goes back to
    /// `extracted`, still waiting, with the rejected id in `outbox`, and
    /// the owner closing it by hand later fell out of `waiting` with no
    /// give-up to match, crediting whatever run spanned the close (found
    /// on review). The give-up then counts on *both* stores, as the fall
    /// does, so the subtraction reaches. `sent` is `None` when the outbox
    /// could not be read; a closed request that has staged anything then
    /// counts as a give-up, the direction that under-credits.
    fn frontdoor_given_up(records: &[frontdoor::Record], sent: Option<&HashSet<&str>>) -> usize {
        records
            .iter()
            .filter(|r| r.state == frontdoor::CLOSED)
            .filter(|r| {
                !r.outbox
                    .iter()
                    .any(|id| sent.is_some_and(|sent| sent.contains(id.as_str())))
            })
            .count()
    }

    fn read_proposals() -> Option<Depth> {
        // The same rule as the three owner-facing readers: a store that
        // has never existed is empty, and a read creates nothing —
        // `LearningStore::open` runs `git init` (found on review, after
        // the test below asserted the rule for three of five readers).
        let Some(store) = LearningStore::open_existing_default() else {
            return Some(Depth::default());
        };
        let proposals = store.proposals().ok()?;
        let pending: Vec<_> = proposals.iter().filter(|p| p.status == "pending").collect();
        Some(Depth::of(
            pending.len(),
            pending.iter().map(|p| p.created_at.as_str()),
        ))
    }

    fn read_candidates() -> Option<Depth> {
        let Some(store) = HarnessStore::open_existing_default() else {
            return Some(Depth::default());
        };
        let candidates = store.all().ok()?;
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

    /// A hand-closed request whose draft was rejected is a give-up, not a
    /// clearance (found on review): the predicate matches every close that
    /// was not an answer, joined against the outbox's sent ids.
    #[test]
    fn a_closed_request_is_a_give_up_unless_a_draft_it_staged_was_sent() {
        let record = |seq: i64, state: &str, outbox: &[&str]| -> frontdoor::Record {
            serde_json::from_value(serde_json::json!({
                "seq": seq,
                "type_id": "contact",
                "state": state,
                "created_at": "2026-09-01T00:00:00Z",
                "drained_at": "2026-09-01T00:00:00Z",
                "values": {},
                "outbox": outbox,
            }))
            .unwrap()
        };
        let records = vec![
            record(1, frontdoor::CLOSED, &[]),         // nothing staged
            record(2, frontdoor::CLOSED, &["o-rej"]),  // draft rejected, closed by hand
            record(3, frontdoor::CLOSED, &["o-sent"]), // answered, then closed
            record(4, "extracted", &["o-rej"]),        // still waiting
        ];
        let sent: HashSet<&str> = ["o-sent"].into_iter().collect();
        assert_eq!(Backlog::frontdoor_given_up(&records, Some(&sent)), 2);
        // Outbox unreadable: a close with anything staged reads as a
        // give-up — under-crediting, never inventing a clearance.
        assert_eq!(Backlog::frontdoor_given_up(&records, None), 3);
        // Through the delta: the rejection and the hand-close in one
        // window fall on both stores and are given up on both, so nothing
        // is credited.
        let before = Backlog {
            outbox: depth(1, None),
            questions: depth(0, None),
            frontdoor: depth(1, None),
            ..Default::default()
        };
        let after = Backlog {
            outbox: depth(0, None).map(|d| d.given_up(1)),
            questions: depth(0, None),
            frontdoor: depth(0, None).map(|d| d.given_up(1)),
            ..Default::default()
        };
        let d = Backlog::delta(&before, &after);
        assert_eq!(d.owner_facing_net(), Some(-2));
        assert_eq!(d.given_up, Some(2));
        assert_eq!(d.owner_facing_cleared(), Some(0));
    }

    /// A read creates nothing (found on review, which noted nothing
    /// asserted it): every owner-facing reader opens only what exists, and
    /// a store that has never existed is an empty depth, never an unknown
    /// one — `anticipated_guilt` is `None` unless all three were read, so
    /// a machine that had simply never used the front door read as
    /// unmeasurable. Fails on `open_default` twice over: the directory
    /// appears, and the depth comes back `None`.
    #[test]
    fn reading_the_backlog_creates_no_store_and_reads_a_missing_one_as_empty() {
        let home = crate::work::tests::HomeGuard::new();
        let b = Backlog::read();
        assert_eq!(b.outbox, Some(Depth::default()));
        assert_eq!(b.questions, Some(Depth::default()));
        assert_eq!(b.frontdoor, Some(Depth::default()));
        assert_eq!(b.proposals, Some(Depth::default()));
        assert_eq!(b.candidates, Some(Depth::default()));
        // All five, not the three owner-facing ones: the first cut of this
        // test asserted three while `LearningStore::open` (which also runs
        // `git init`) and `HarnessStore::open` still created `learning/`
        // and `learning/harness/candidates` twice per run (found on
        // review). The empty-home assertion below is the strong form, and
        // it is what makes this test sensitive to any test in this binary
        // that writes under `MECHA_HOME` without holding `work::tests::ENV`
        // — none does today; if this goes flaky, that is the first place
        // to look.
        for store in ["outbox", "questions", "requests", "learning"] {
            assert!(
                !home.dir().join(store).exists(),
                "a read created {store}/ under the mecha home"
            );
        }
        assert!(
            std::fs::read_dir(home.dir()).unwrap().next().is_none(),
            "a read created something under the mecha home"
        );
    }
}
