//! Anticipated guilt, per commitment (`docs/GOAL-SYSTEM-DESIGN.md` §7.4;
//! `docs/APPRAISAL-WIRING-DESIGN.md` S7, ruling R12, built as 1f): predicted
//! error against *another party's* expectation, as distinct from anxiety's
//! predicted error against the run's own setpoints (`pressure.rs`'s
//! predictive compaction is that half).
//!
//! ## What it may be computed from
//!
//! > **An expectation is a recorded commitment, never a claimed one.**
//!
//! A commitment is something mecha's own stores hold — a staged draft that
//! says a reply is coming, a parked question, a front-door request waiting on
//! the owner — never a third party's assertion that mecha owes them
//! something. That distinction is the whole safety argument: an attacker who
//! wants to manufacture guilt would need to fabricate a row in a store this
//! reads, not just write a sentence saying *"your colleague is counting on
//! you."* §7.2 names the attack this closes: a charter line like "don't let a
//! colleague down" is a lever an injection can pull only if guilt can be
//! talked into existing, and a claim in fetched text or an inbound message
//! cannot write to `OutboxStore`, `QuestionStore` or `Frontdoor`.
//!
//! So the three stores here are commitments **by construction** — a row in
//! one *is* the recorded commitment, owed to the draft's recipient, the
//! delegated task's asker, or the requester — and this module reads their
//! items out of the survey [`crate::backlog::Backlog::survey`] already takes
//! at every run's start (`Inventory`), never a store of its own and never a
//! row anything else could write. 1f changed which code computes guilt, not
//! what may create a row. The front door is read as the requests **waiting on
//! the owner** (`frontdoor::WAITING_ON_OWNER`, aged from `arrived_at`), the
//! set the doctor's stale-request finding and the `request_closure` sensor
//! read — a `needs_info` parked on the stranger is owed by nobody here, and
//! the scalar this replaced counted it anyway. The graph board's `due_at` and
//! the workflow store's commitments are not read: the first needs a
//! `mecha-graph` subprocess in the path of every run, and the second has no
//! charter kind and no doctor constant to supply a patience (deferred; see
//! the design doc's S7 entry).
//!
//! ## Guilt per item, and why it replaced one scalar
//!
//! Per commitment, `guilt = excess(age, patience) × weight(rank)`:
//!
//! - **Patience** is how long is too long, and it is the doctor's own
//!   ([`crate::doctor::Patience::for_store`]): the setpoint of the charter
//!   line whose age kind watches the store, else the harness constant (48h a
//!   draft, 24h a question, 72h a request). One definition, so a draft the
//!   doctor calls stuck is exactly a draft that carries guilt.
//! - **Excess** is [`crate::reading::excess`]: zero within the patience,
//!   half of maximal at twice it, asymptotic toward one — never one, because
//!   a term that reaches `1.0` stops varying and a corpus of a constant
//!   carries nothing. The retired scalar learned that twice: its age term
//!   first clamped at one day, then at one week, and each time the live
//!   outbox's normal state pinned it (its history is in `docs/HISTORY.md`
//!   and `APPRAISAL-RESEARCH.md` §1.5).
//! - **Rank** is how much it matters: the charter rank of that line (file
//!   order, zero the top), weighted `1 / (1 + rank)`. A store no line
//!   watches ranks **below every line the owner wrote** — at the number of
//!   lines — because the owner ranked it nowhere: an unwatched store must not
//!   outweigh one the owner put third. With no charter at all that is rank
//!   zero, a weight of one, and guilt is the excess alone.
//!
//! The charter supplies *how much it matters and how long is too long*; the
//! commitment supplies *to whom and since when*. Neither alone is guilt: a
//! charter line is the owner's priority, not a promise to anyone.
//!
//! **The scalar this replaced** was one number folded from the three stores'
//! count, their oldest age and the run's context pressure, as a logical OR.
//! It read 0.95–1.0 on every live run (inventory §1): the oldest draft pins
//! an age, and the age pinned the OR. It is kept as a **readout** —
//! [`readout`], the maximum per-commitment value, written into
//! `Homeostat::anticipated_guilt` for the diagnostician's brief — and no
//! consumer decides on it (here §1, decision 3: a level is read per item,
//! never as a level). A row recorded before 1f still loads with the old
//! fold's number in that field; the corpus mean reads only rows that carry
//! the per-commitment record, so the two formulas are never averaged
//! together. Context pressure left the number with the fold: it is a fact
//! about the run's own room, not about anyone waiting.
//!
//! ## Unknown is never zero
//!
//! A store that could not be read has `waiting: None`; an item whose stamp
//! will not parse has an unknown age and so an unknown guilt, counted apart
//! (`unknown`) — it could be the oldest of all. A store with either reads
//! [`StoreGuilt::max`] `None`, and the readout is `None` when any store's
//! is. A store that holds nothing is a real zero. Undated items sort
//! *last* in the record, so the cap never drops a known overdue commitment
//! for them; `unknown` counts them whether or not they are listed.
//!
//! ## It never reaches a prompt
//!
//! No number here is rendered into any request (G4, R21 — per-commitment
//! guilt is named there as a score a model would move); the fixture scan in
//! `provider/anthropic.rs` holds every value this produces. The one in-run
//! consumer, `planning::Decision::assess`, reaches the model only as fixed
//! advice behind `goal_guidance`.
//!
//! ## The formula is argued, not measured
//!
//! There is no corpus yet linking any weighting here to a real missed
//! expectation — the discipline every sensor in this arc shipped under.

use crate::backlog::{Inventory, Waiter};
use crate::charter::{Charter, SensorKind};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// How many of a store's commitments one record names, highest guilt first.
/// Every one of them has its own value while it is in memory; the record
/// keeps this many per store, with `waiting` saying how many there were, so
/// a queue of hundreds does not grow every run record with it. Argued, not
/// measured: the live install holds about a dozen pending drafts.
pub const COMMITMENTS_RECORDED: usize = 32;

/// The store a commitment is recorded in — which is what makes it one.
///
/// **A closed enum written to an append-only store is a wire format**: a
/// store a later binary adds loads through [`lenient`] as unknown, costing
/// the per-commitment record and never the run record it rides on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Store {
    /// A staged draft, owed to its recipient.
    Outbox,
    /// A parked question, owed to the run and the task that asked it.
    Questions,
    /// A front-door request waiting on the owner, owed to the requester.
    Requests,
}

impl Store {
    pub const ALL: [Store; 3] = [Store::Outbox, Store::Questions, Store::Requests];

    /// The age kind whose line is this store's patience and rank.
    pub fn kind(self) -> SensorKind {
        match self {
            Store::Outbox => SensorKind::OutboxAge,
            Store::Questions => SensorKind::QuestionLatency,
            Store::Requests => SensorKind::RequestClosure,
        }
    }

    /// This store's items out of one survey — `None` where it could not be
    /// read.
    fn waiters(self, inv: &Inventory) -> Option<&[Waiter]> {
        match self {
            Store::Outbox => inv.outbox.as_deref(),
            Store::Questions => inv.questions.as_deref(),
            Store::Requests => inv.requests_on_owner.as_deref(),
        }
    }
}

/// One pending commitment and its own guilt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemGuilt {
    /// The id its store knows it by: the draft's, the question's, the
    /// request's `seq`.
    pub id: String,
    /// How long it has waited, in seconds. `None` when its stamp would not
    /// parse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_secs: Option<u64>,
    /// `excess(age, patience) × weight`, in `[0, 1)`. `None` exactly when
    /// the age is unknown — never a guess dressed as zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guilt: Option<f32>,
}

/// One store's pending commitments, each with its own guilt, and what they
/// were read against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreGuilt {
    pub store: Store,
    /// The charter line whose setpoint is the patience and whose rank is
    /// the weight. `None`: no line's age kind watches this store, so the
    /// patience is the doctor's constant and the store ranks below every
    /// line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    /// The patience as spelled — the owner's setpoint, or the constant.
    pub patience: String,
    /// `1 / (1 + rank)`, the rank of `line` (or one past the last line).
    pub weight: f32,
    /// How many commitments the store holds. `None` when it could not be
    /// read — unknown, never an empty queue.
    pub waiting: Option<u64>,
    /// Of those, how many have an unknown age.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unknown: u64,
    /// Each commitment, by guilt and age descending, then id, with the
    /// undated ones last — so the cap keeps every known overdue commitment
    /// before any whose standing is unknown (those are counted in
    /// `unknown` whether or not they are listed); at most
    /// [`COMMITMENTS_RECORDED`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ItemGuilt>,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl StoreGuilt {
    /// The largest guilt among this store's commitments: `Some(0.0)` for a
    /// store that holds nothing, `None` for one that could not be read or
    /// holds an item of unknown age. Sorting puts the largest known first
    /// and the cap keeps it, and `unknown` counts the undated ones whether
    /// or not the cap kept them, so the record alone answers this.
    pub fn max(&self) -> Option<f32> {
        if self.waiting.is_none() || self.unknown > 0 {
            return None;
        }
        Some(
            self.items
                .iter()
                .filter_map(|i| i.guilt)
                .fold(0.0, f32::max),
        )
    }

    /// Whether any commitment here is past its patience — a known guilt
    /// above zero. Answers `true` whatever else is unknown: one known
    /// overdue commitment is a fact an unknown sibling cannot undo — and
    /// the undated ones sort after it, so the cap cannot drop it for them.
    pub fn any_owed(&self) -> bool {
        self.items.iter().any(|i| i.guilt.is_some_and(|g| g > 0.0))
    }
}

/// The weight a charter rank gives: `1 / (1 + rank)`, one for the top line,
/// a half for the second — rank orders, and no amount of lower-ranked guilt
/// is made to equal a higher line's by a threshold.
pub fn weight(rank: usize) -> f32 {
    1.0 / (1.0 + rank as f32)
}

/// One commitment's guilt: how far past its patience it has waited
/// ([`crate::reading::excess`]) times the weight of the line that set the
/// patience. `None` when the age is unknown.
pub fn item_guilt(age_secs: Option<u64>, patience_secs: f64, weight: f32) -> Option<f32> {
    age_secs.map(|age| crate::reading::excess(age as f64, patience_secs) * weight)
}

/// Every pending commitment in one survey, with its own guilt, per store in
/// [`Store::ALL`] order. A pure function of the items, the charter and
/// `now`, so it replays over whatever a record holds.
pub fn read_commitments(
    items: &Inventory,
    charter: &Charter,
    now: DateTime<Utc>,
) -> Vec<StoreGuilt> {
    Store::ALL
        .into_iter()
        .map(|store| {
            let patience = crate::doctor::Patience::for_store(Some(charter), store.kind())
                .expect("every store's kind is an age");
            // The line's rank, or one past the last line: an unwatched store
            // ranks below everything the owner ranked.
            let rank = patience
                .line
                .as_deref()
                .and_then(|l| charter.rank_of(l))
                .unwrap_or(charter.lines().len());
            let weight = weight(rank);
            let patience_secs = patience.after.num_seconds().max(1) as f64;
            let Some(waiters) = store.waiters(items) else {
                return StoreGuilt {
                    store,
                    line: patience.line,
                    patience: patience.text,
                    weight,
                    waiting: None,
                    unknown: 0,
                    items: Vec::new(),
                };
            };
            let mut owed: Vec<ItemGuilt> = waiters
                .iter()
                .map(|w| {
                    let age_secs = crate::reading::age_secs(&w.since, now);
                    ItemGuilt {
                        id: w.id.clone(),
                        age_secs,
                        guilt: item_guilt(age_secs, patience_secs, weight),
                    }
                })
                .collect();
            // Undated **last**: the cap must never drop a known overdue
            // commitment for ones whose standing is unknown — those are
            // counted in `unknown` whatever the cap keeps, and that count is
            // what makes the maximum unknown. With undated first, 32 torn
            // stamps filled the record and hid every overdue draft behind
            // them (found on review of #302).
            owed.sort_by(|a, b| match (a.age_secs, b.age_secs) {
                (None, None) => a.id.cmp(&b.id),
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                // Within one store the weight and patience are shared, so
                // guilt orders exactly as age does, and age still orders
                // the ones inside their patience.
                (Some(x), Some(y)) => y.cmp(&x).then_with(|| a.id.cmp(&b.id)),
            });
            let unknown = owed.iter().filter(|i| i.age_secs.is_none()).count() as u64;
            let waiting = owed.len() as u64;
            owed.truncate(COMMITMENTS_RECORDED);
            StoreGuilt {
                store,
                line: patience.line,
                patience: patience.text,
                weight,
                waiting: Some(waiting),
                unknown,
                items: owed,
            }
        })
        .collect()
}

/// The readout: the largest per-commitment guilt across the stores — what
/// `Homeostat::anticipated_guilt` records and the diagnostician's brief
/// averages. `None` when any store's maximum is unknown, or when there are
/// no stores to read: a dash is never zero.
pub fn readout(stores: &[StoreGuilt]) -> Option<f32> {
    if stores.is_empty() {
        return None;
    }
    stores
        .iter()
        .map(StoreGuilt::max)
        .try_fold(0.0f32, |acc, m| m.map(|m| acc.max(m)))
}

/// The commitments a run's in-run consumers may read: every store but one
/// whose line was withdrawn as saturated (S5, `reading::in_run`) — a line
/// withheld from the run takes its commitments with it, or the consumer
/// fires on every run through the side door. `None` stays `None`.
pub fn in_run(
    stores: Option<&[StoreGuilt]>,
    readings: Option<&[crate::reading::LineReading]>,
) -> Option<Vec<StoreGuilt>> {
    let withdrawn = |line: &str| {
        readings
            .unwrap_or_default()
            .iter()
            .any(|r| r.withdrawn && r.line == line)
    };
    stores.map(|ss| {
        ss.iter()
            .filter(|s| !s.line.as_deref().is_some_and(withdrawn))
            .cloned()
            .collect()
    })
}

/// The homeostat's `commitments` field, loaded leniently: a record this
/// binary cannot parse — a store a later one added — reads as `None`,
/// unknown, rather than failing the run record it sits on.
pub fn lenient<'de, D>(d: D) -> Result<Option<Vec<StoreGuilt>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: Option<serde_json::Value> = Option::deserialize(d)?;
    Ok(raw.and_then(|v| serde_json::from_value(v).ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::charter::{RawLine, RawSensor, RawSetpoint};

    fn now() -> DateTime<Utc> {
        "2026-09-20T12:00:00Z".parse().unwrap()
    }

    fn ago(hours: i64) -> String {
        (now() - chrono::Duration::hours(hours)).to_rfc3339()
    }

    fn line(id: &str, sensor: Option<(SensorKind, &str)>) -> RawLine {
        RawLine {
            id: id.into(),
            text: format!("{id} text"),
            sensor: sensor.map(|(kind, sp)| RawSensor {
                kind,
                setpoint: RawSetpoint::Text(sp.into()),
            }),
        }
    }

    fn inventory(
        outbox: Option<Vec<Waiter>>,
        questions: Option<Vec<Waiter>>,
        requests: Option<Vec<Waiter>>,
    ) -> Inventory {
        Inventory {
            outbox,
            questions,
            requests_on_owner: requests,
            ..Default::default()
        }
    }

    fn of(stores: &[StoreGuilt], store: Store) -> &StoreGuilt {
        stores.iter().find(|s| s.store == store).unwrap()
    }

    fn guilt_of(stores: &[StoreGuilt], id: &str) -> Option<f32> {
        stores
            .iter()
            .flat_map(|s| &s.items)
            .find(|i| i.id == id)
            .unwrap_or_else(|| panic!("no commitment `{id}`"))
            .guilt
    }

    /// The per-item function: excess over the patience times the line's
    /// weight — zero within the patience, a half of the weight at twice
    /// it, and never the weight itself.
    #[test]
    fn guilt_per_item_is_excess_over_patience_times_the_rank_weight() {
        let day = 86_400.0;
        assert_eq!(item_guilt(Some(3_600), day, 1.0), Some(0.0));
        assert_eq!(item_guilt(Some(86_400), day, 1.0), Some(0.0));
        assert_eq!(item_guilt(Some(2 * 86_400), day, 1.0), Some(0.5));
        assert_eq!(item_guilt(Some(2 * 86_400), day, weight(1)), Some(0.25));
        assert_eq!(item_guilt(None, day, 1.0), None, "unknown, not zero");
        let far = item_guilt(Some(86_400 * 10_000), day, 1.0).unwrap();
        assert!(far < 1.0 && far > 0.99, "{far}");
        assert_eq!(weight(0), 1.0);
        assert_eq!(weight(3), 0.25);
    }

    /// The acceptance, at the function: every pending commitment of every
    /// kind has its own value, patience comes from the line watching its
    /// store or the doctor's constant, rank weighs it, and the readout is
    /// the largest. The old scalar was one number over all of them — here
    /// two drafts of different ages under one line read differently, and a
    /// question and a request each read under their own patience.
    #[test]
    fn each_pending_commitment_has_its_own_value_and_the_readout_is_the_maximum() {
        let charter = Charter::from_raw_lines(vec![
            line("replies", Some((SensorKind::OutboxAge, "24h"))),
            line("craft", None),
            line("questions", Some((SensorKind::QuestionLatency, "12h"))),
        ])
        .unwrap();
        let inv = inventory(
            Some(vec![
                Waiter::new("d-fresh", ago(2)),
                Waiter::new("d-old", ago(72)),
                Waiter::new("d-older", ago(96)),
            ]),
            Some(vec![Waiter::new("q-1", ago(36))]),
            Some(vec![Waiter::new("7", ago(144))]),
        );
        let stores = read_commitments(&inv, &charter, now());

        // The outbox: the top line, 24h. Three days is 48h past a day:
        // 2/3; four days 3/4; two hours nothing.
        let outbox = of(&stores, Store::Outbox);
        assert_eq!(outbox.line.as_deref(), Some("replies"));
        assert_eq!(outbox.patience, "24h");
        assert_eq!(outbox.weight, 1.0);
        assert_eq!(outbox.waiting, Some(3));
        let ids: Vec<&str> = outbox.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["d-older", "d-old", "d-fresh"], "oldest first");
        assert!((guilt_of(&stores, "d-older").unwrap() - 0.75).abs() < 1e-6);
        assert!((guilt_of(&stores, "d-old").unwrap() - 2.0 / 3.0).abs() < 1e-6);
        assert_eq!(guilt_of(&stores, "d-fresh"), Some(0.0));

        // The question: the third line (rank 2, weight 1/3), 12h. 36h is
        // 24h past: excess 2/3, times a third.
        let q = of(&stores, Store::Questions);
        assert_eq!(q.line.as_deref(), Some("questions"));
        assert!((q.weight - 1.0 / 3.0).abs() < 1e-6);
        assert!((guilt_of(&stores, "q-1").unwrap() - 2.0 / 9.0).abs() < 1e-6);

        // The request: no line watches the front door, so the doctor's 72h
        // and a rank below all three lines (weight 1/4). Six days is 72h
        // past 72h: a half, times a quarter.
        let r = of(&stores, Store::Requests);
        assert_eq!(r.line, None);
        assert_eq!(r.patience, "72h");
        assert_eq!(r.weight, 0.25);
        assert!((guilt_of(&stores, "7").unwrap() - 0.125).abs() < 1e-6);

        // Each has its own value, and the readout is the largest.
        let values: Vec<f32> = stores
            .iter()
            .flat_map(|s| &s.items)
            .filter_map(|i| i.guilt)
            .collect();
        assert_eq!(values.len(), 5);
        assert_eq!(readout(&stores), Some(0.75));
        assert_eq!(outbox.max(), Some(0.75));
        assert!(outbox.any_owed() && q.any_owed() && r.any_owed());
    }

    /// Rank orders: the same overdue draft reads less under a line the
    /// owner put lower, and an unwatched store ranks below every line.
    #[test]
    fn a_lower_line_weighs_less_and_an_unwatched_store_ranks_below_every_line() {
        let inv = inventory(
            Some(vec![Waiter::new("d", ago(48))]),
            Some(Vec::new()),
            Some(Vec::new()),
        );
        let top =
            Charter::from_raw_lines(vec![line("replies", Some((SensorKind::OutboxAge, "24h")))])
                .unwrap();
        let second = Charter::from_raw_lines(vec![
            line("first", None),
            line("replies", Some((SensorKind::OutboxAge, "24h"))),
        ])
        .unwrap();
        let g_top = guilt_of(&read_commitments(&inv, &top, now()), "d").unwrap();
        let g_second = guilt_of(&read_commitments(&inv, &second, now()), "d").unwrap();
        assert!((g_top - 0.5).abs() < 1e-6 && (g_second - 0.25).abs() < 1e-6);

        // No line on the outbox: 48h is within the doctor's 48h — nothing.
        // At four days, 48h past: a half, times the weight of a rank one
        // past the two lines (a third).
        let unwatched = Charter::from_raw_lines(vec![line("a", None), line("b", None)]).unwrap();
        assert_eq!(
            guilt_of(&read_commitments(&inv, &unwatched, now()), "d"),
            Some(0.0)
        );
        let late = inventory(
            Some(vec![Waiter::new("d", ago(96))]),
            Some(Vec::new()),
            Some(Vec::new()),
        );
        let g = guilt_of(&read_commitments(&late, &unwatched, now()), "d").unwrap();
        assert!((g - 0.5 / 3.0).abs() < 1e-6, "{g}");
        // No charter at all: rank zero, the excess alone.
        let none = guilt_of(&read_commitments(&late, &Charter::default(), now()), "d").unwrap();
        assert!((none - 0.5).abs() < 1e-6, "{none}");
    }

    /// Unknown is never zero: an unreadable store and an undated item each
    /// make the readout unknown, while an empty store is a real zero and a
    /// known overdue sibling is still owed.
    #[test]
    fn an_unreadable_store_or_an_undated_item_is_unknown_and_an_empty_store_is_zero() {
        let charter = Charter::default();
        let empty = inventory(Some(Vec::new()), Some(Vec::new()), Some(Vec::new()));
        let stores = read_commitments(&empty, &charter, now());
        assert_eq!(readout(&stores), Some(0.0), "nothing owed is a real zero");

        let unread = inventory(Some(Vec::new()), None, Some(Vec::new()));
        let stores = read_commitments(&unread, &charter, now());
        assert_eq!(of(&stores, Store::Questions).waiting, None);
        assert_eq!(readout(&stores), None);

        let undated = inventory(
            Some(vec![
                Waiter::new("late", ago(200)),
                Waiter::new("torn", "not-a-stamp"),
            ]),
            Some(Vec::new()),
            Some(Vec::new()),
        );
        let stores = read_commitments(&undated, &charter, now());
        let outbox = of(&stores, Store::Outbox);
        assert_eq!(outbox.unknown, 1);
        assert_eq!(outbox.items[0].id, "late", "the known overdue sorts first");
        assert_eq!(outbox.items[1].id, "torn", "the unknown sorts last");
        assert_eq!(outbox.items[1].guilt, None);
        assert_eq!(outbox.max(), None);
        assert_eq!(readout(&stores), None);
        assert!(outbox.any_owed(), "the known overdue draft is still owed");
        assert_eq!(readout(&[]), None, "no stores read is not zero");
    }

    /// A queue of hundreds is recorded up to the cap, the largest kept.
    #[test]
    fn the_record_keeps_the_largest_up_to_the_cap_and_counts_the_rest() {
        let many: Vec<Waiter> = (0..100)
            .map(|i| Waiter::new(format!("d{i:03}"), ago(i)))
            .collect();
        let inv = inventory(Some(many), Some(Vec::new()), Some(Vec::new()));
        let stores = read_commitments(&inv, &Charter::default(), now());
        let outbox = of(&stores, Store::Outbox);
        assert_eq!(outbox.waiting, Some(100));
        assert_eq!(outbox.items.len(), COMMITMENTS_RECORDED);
        assert_eq!(outbox.items[0].id, "d099");
        let full = item_guilt(Some(99 * 3_600), 48.0 * 3_600.0, 1.0).unwrap();
        assert_eq!(outbox.max(), Some(full));
    }

    /// The cap must not hide what is owed (review of #302). With more
    /// undated items than the record keeps, one dated draft past its
    /// patience is still recorded and still owed: the known overdue
    /// commitment is the fact a consumer acts on, and undated ones are
    /// already counted apart in `unknown`. When undated items sorted first,
    /// 32 torn stamps filled the record and `any_owed` read `false`.
    #[test]
    fn a_dated_overdue_commitment_survives_the_cap_behind_many_undated_ones() {
        let mut waiters: Vec<Waiter> = (0..COMMITMENTS_RECORDED + 8)
            .map(|i| Waiter::new(format!("torn{i:03}"), "not-a-stamp"))
            .collect();
        waiters.push(Waiter::new("late", ago(200)));
        let inv = inventory(Some(waiters), Some(Vec::new()), Some(Vec::new()));
        let stores = read_commitments(&inv, &Charter::default(), now());
        let outbox = of(&stores, Store::Outbox);
        assert_eq!(outbox.waiting, Some(COMMITMENTS_RECORDED as u64 + 9));
        assert_eq!(outbox.unknown, COMMITMENTS_RECORDED as u64 + 8);
        assert_eq!(outbox.items.len(), COMMITMENTS_RECORDED);
        assert!(outbox.items.iter().any(|i| i.id == "late"), "{outbox:?}");
        assert!(outbox.any_owed(), "the dated overdue draft is still owed");
        assert_eq!(outbox.max(), None, "and the maximum is still unknown");
        assert_eq!(readout(&stores), None);
    }

    /// A withdrawn line takes its store's commitments out of the run with
    /// it; an unwatched store and a line still in front of the run stay.
    #[test]
    fn a_withdrawn_lines_commitments_leave_the_run_with_it() {
        let charter =
            Charter::from_raw_lines(vec![line("replies", Some((SensorKind::OutboxAge, "24h")))])
                .unwrap();
        let inv = inventory(Some(Vec::new()), Some(Vec::new()), Some(Vec::new()));
        let stores = read_commitments(&inv, &charter, now());
        let reading = |withdrawn| crate::reading::LineReading {
            line: "replies".into(),
            kind: SensorKind::OutboxAge,
            setpoint: "24h".into(),
            reading: crate::reading::Reading::Nothing,
            items: None,
            delta: None,
            withdrawn,
        };
        let kept = in_run(Some(&stores), Some(&[reading(false)])).unwrap();
        assert_eq!(kept.len(), 3);
        let left = in_run(Some(&stores), Some(&[reading(true)])).unwrap();
        assert_eq!(
            left.iter().map(|s| s.store).collect::<Vec<_>>(),
            [Store::Questions, Store::Requests]
        );
        assert_eq!(in_run(None, Some(&[reading(true)])), None);
    }

    /// The record round-trips, and a store a later binary adds costs the
    /// per-commitment record — `None`, unknown — never the row.
    #[test]
    fn the_record_round_trips_and_an_unknown_store_loads_as_unknown() {
        #[derive(Deserialize)]
        struct Row {
            #[serde(default, deserialize_with = "lenient")]
            commitments: Option<Vec<StoreGuilt>>,
        }
        let inv = inventory(
            Some(vec![Waiter::new("d", ago(30))]),
            Some(Vec::new()),
            Some(Vec::new()),
        );
        let stores = read_commitments(&inv, &Charter::default(), now());
        let json = serde_json::json!({ "commitments": stores }).to_string();
        let row: Row = serde_json::from_str(&json).unwrap();
        assert_eq!(row.commitments.as_deref(), Some(stores.as_slice()));
        let later = json.replace("\"requests\"", "\"board\"");
        assert_ne!(later, json);
        let row: Row = serde_json::from_str(&later).unwrap();
        assert_eq!(row.commitments, None);
        let absent: Row = serde_json::from_str("{}").unwrap();
        assert_eq!(absent.commitments, None);
    }
}
