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
}

impl Depth {
    fn of<'a>(waiting: usize, stamps: impl IntoIterator<Item = &'a str>) -> Depth {
        Depth {
            waiting,
            oldest: stamps.into_iter().min().map(str::to_string),
        }
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
        Some(Depth::of(
            pending.len(),
            pending.iter().map(|i| i.created_at.as_str()),
        ))
    }

    fn read_questions() -> Option<Depth> {
        // A store that has never existed is empty, not unreadable.
        let Some(store) = QuestionStore::open_existing_default() else {
            return Some(Depth::default());
        };
        let items = store.items().ok()?;
        let open: Vec<_> = items.iter().filter(|q| q.is_open()).collect();
        Some(Depth::of(
            open.len(),
            open.iter().map(|q| q.asked_at.as_str()),
        ))
    }

    fn read_frontdoor() -> Option<Depth> {
        let records = Frontdoor::open_default().and_then(|s| s.records()).ok()?;
        let open: Vec<_> = records
            .iter()
            .filter(|r| r.state != frontdoor::CLOSED)
            .collect();
        Some(Depth::of(
            open.len(),
            open.iter().map(|r| r.created_at.as_str()),
        ))
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
        BacklogDelta {
            outbox: d(&before.outbox, &after.outbox),
            questions: d(&before.questions, &after.questions),
            frontdoor: d(&before.frontdoor, &after.frontdoor),
            proposals: d(&before.proposals, &after.proposals),
            candidates: d(&before.candidates, &after.candidates),
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
}

impl BacklogDelta {
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
}
