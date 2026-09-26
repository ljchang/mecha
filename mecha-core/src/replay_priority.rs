//! What the nightly replays, learns from and validates first: **gain ×
//! need** (`APPRAISAL-WIRING-DESIGN.md` L1, row 2e-6).
//!
//! Prioritised experience replay beat uniform replay on 41 of 49 Atari
//! games by sampling on |TD error|, and Mattar & Daw (2018) sharpen the
//! priority to *gain* — how much a backup would change the policy — times
//! *need* — how often the state recurs. Here, per recorded session:
//!
//! ```text
//! priority = gain × need × decay
//!   gain  = Σ |sign| over the owner's verdicts on the run, each × its charter weight
//!         + SURPRISE_WEIGHT per clean miss of the run's appraisal (2b-2)
//!   need  = ln(1 + n), n = recent runs in the same `Situation` region
//!   decay = 2^(−age / AGE_HALF_LIFE_DAYS)
//! ```
//!
//! - **Gain is the owner's, never a model's.** Only errors that record an
//!   act the owner performed count ([`crate::appraisal::GoalError::is_owner_verdict`],
//!   R16's channels): a counter, a sensor or a model's judgment orders
//!   nothing here. The charter weight is [`crate::appraisal::charter_weight`]
//!   on clean-origin records only, and a surprise counts only when the
//!   appraisal it scored is clean — both are read off text a tainted run's
//!   model could have steered, and this priority decides membership of the
//!   slice the gate reads.
//! - **Need is mecha-graph's Selector demand term, ported.** The graph's
//!   Selector (`mecha-graph-core/src/probe.rs`, `probe_targets`) scores a
//!   probe target `ln(1 + touches) · gaps` — multiplicative, so an untouched
//!   node scores nothing, "accuracy is non-uniform by design, high where the
//!   graph is used" — with `touches` read from `retrieval_touch`, what
//!   retrieval actually served. Ported: `touches` becomes how many recent
//!   admitted runs were matched in the episode's region
//!   ([`crate::situation::Situation::region_key`], the key I2's "same
//!   situation and goal" compares on), and the graph's rule that *a probe's
//!   own reads are not demand* (`ledger.rs`, `record_query`) becomes the
//!   corpus admission: test and experiment sessions are not recurrence. The
//!   episode is itself one occurrence, so need is never zero — a one-off
//!   scores `ln 2`, a region seen five times `ln 6`.
//! - **Decay** halves a session's priority every [`AGE_HALF_LIFE_DAYS`]:
//!   the harness being graded is the one running now.
//!
//! ## Unknown is never zero, and never a free pass
//!
//! Each factor that cannot be read is named in [`Priority::unknown`] —
//! the owner's verdicts (no appraisal could be built, or a store it reads
//! did not load — the charter among them, which it weighs by, so an
//! unreadable charter makes the whole gain unknown rather than only its
//! weighting), the surprises (the score ledger did not load, or
//! has lines that do not parse), the recurrence (the session has no run
//! record, or its region names a surface or goal this build cannot read, or
//! the corpus walk failed), and the hopeless mark (the harness or
//! comparison store did not load). The order ([`order`]) is by tier:
//!
//! 1. every factor known and the priority positive, by value;
//! 2. **any factor unknown** — after every fully known positive priority
//!    (never a free pass), and before every known zero (never zero); among
//!    these, fewer unknown factors first, then the value computed with each
//!    unknown factor at its floor (none of the gain's terms; the episode's
//!    own occurrence, `ln 2`, for need — never more than a known one-off);
//! 3. a known zero — no owner verdict and no surprise;
//! 4. the hopeless ([`History::hopeless`]), last.
//!
//! The id breaks every remaining tie, so the order is total and a seed is
//! not a lie.
//!
//! **Numbers stay harness-side** (G4, R21): nothing here is rendered into a
//! prompt. A priority orders what is examined; the gate decides whether a
//! change helped (`GOAL-SYSTEM-DESIGN.md` §8.3), and the holdout that
//! confirms it is drawn uniformly before any priority is read (§8.1).

use crate::appraisal::{self, Stores};
use crate::comparison::{Comparison, Role, Verdict};
use crate::harness::{HarnessCandidate, STATUS_ACCEPTED, STATUS_REVERTED};
use crate::session::{Session, Transcript};
use crate::situation::Situation;
use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// What one clean miss of a run's appraisal adds to its gain: the
/// magnitude of one owner rejection. A miss is the appraisal's own
/// prediction error (X5) — the |TD error| PER samples on — so it raises the
/// priority by as much as the owner's clearest verdict does, and no more.
pub const SURPRISE_WEIGHT: f64 = 1.0;

/// The age at which a session's priority has halved. Two weeks: the
/// nightly measures the harness running now, and a session recorded under
/// a harness several accepted changes old is weaker evidence about it.
pub const AGE_HALF_LIFE_DAYS: f64 = 14.0;

/// How far back a region's recurrence is counted — "recent runs" (L1).
/// Thirty days: long enough that a weekly situation recurs four times,
/// short enough that a situation the owner stopped being in stops counting.
pub const RECURRENCE_WINDOW_DAYS: i64 = 30;

/// At most this many sessions are read for recurrence, newest first — a
/// bound on the walk, which reads each run record's line and nothing else.
pub const RECURRENCE_SCAN_CAP: usize = 500;

/// An episode is **hopeless** once it has sat in a measured selection slice
/// on this many distinct nights since anything last won on it (§9.2's
/// "skip the hopeless"). Three: one losing night is the ordinary case,
/// two can be one bad candidate proposed twice, and a third is the
/// selection spending real model runs on an episode nothing has learned
/// from. Being in the selection *is* "high": the draw put it there.
pub const HOPELESS_NIGHTS: usize = 3;

/// A factor of the priority that could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Factor {
    OwnerVerdicts,
    Surprises,
    Recurrence,
    Hopeless,
}

impl Factor {
    pub fn as_str(self) -> &'static str {
        match self {
            Factor::OwnerVerdicts => "owner verdicts",
            Factor::Surprises => "surprises",
            Factor::Recurrence => "recurrence",
            Factor::Hopeless => "hopeless mark",
        }
    }
}

/// What one session's priority is computed from, each input `None` where
/// it could not be read.
#[derive(Debug, Clone, PartialEq)]
pub struct Inputs {
    /// [`appraisal::owner_verdict_gain`] of the session's appraisal.
    pub owner_gain: Option<f64>,
    /// Clean misses of the session's appraisal's prediction (2b-2).
    pub surprises: Option<usize>,
    /// Recent admitted runs in the session's region.
    pub recurrence: Option<usize>,
    /// Days since the session was created.
    pub age_days: f64,
    /// [`History::hopeless`] for the session.
    pub hopeless: Option<bool>,
}

/// One session's replay priority. See the module note for the formula and
/// the order.
#[derive(Debug, Clone, PartialEq)]
pub struct Priority {
    /// The known part of the gain — a floor where a part is unknown.
    pub gain: f64,
    /// `ln(1 + n)`; `None` where the recurrence is unknown.
    pub need: Option<f64>,
    pub decay: f64,
    /// The factors that could not be read.
    pub unknown: BTreeSet<Factor>,
    /// No owner verdict and no surprise, both known: nothing to learn from.
    pub known_zero: bool,
    pub hopeless: Option<bool>,
}

impl Priority {
    pub fn of(i: &Inputs) -> Priority {
        let mut unknown = BTreeSet::new();
        if i.owner_gain.is_none() {
            unknown.insert(Factor::OwnerVerdicts);
        }
        if i.surprises.is_none() {
            unknown.insert(Factor::Surprises);
        }
        if i.recurrence.is_none() {
            unknown.insert(Factor::Recurrence);
        }
        if i.hopeless.is_none() {
            unknown.insert(Factor::Hopeless);
        }
        let gain = i.owner_gain.unwrap_or(0.0) + SURPRISE_WEIGHT * i.surprises.unwrap_or(0) as f64;
        Priority {
            gain,
            // The episode is one occurrence of its own region, so need is
            // never zero (the Selector's untouched node has no analogue: a
            // recorded run was, by definition, touched once).
            need: i.recurrence.map(need_of),
            decay: 0.5f64.powf(i.age_days.max(0.0) / AGE_HALF_LIFE_DAYS),
            unknown,
            known_zero: i.owner_gain == Some(0.0) && i.surprises == Some(0),
            hopeless: i.hopeless,
        }
    }

    /// A session that could not be read at all: every factor unknown.
    pub fn unread() -> Priority {
        Priority::of(&Inputs {
            owner_gain: None,
            surprises: None,
            recurrence: None,
            age_days: 0.0,
            hopeless: None,
        })
    }

    /// gain × need × decay, an unknown need at its floor: the episode's
    /// own occurrence. `1.0` here ranked an unreadable recurrence above a
    /// known one-off (`ln 2`), a promotion for not being read (found on
    /// review).
    pub fn value(&self) -> f64 {
        self.gain * self.need.unwrap_or(NEED_FLOOR) * self.decay
    }

    /// The tier [`order`] ranks by first — see the module note.
    pub fn tier(&self) -> u8 {
        if self.hopeless == Some(true) {
            3
        } else if self.known_zero {
            2
        } else if !self.unknown.is_empty() {
            1
        } else {
            0
        }
    }
}

/// The one priority order — the selection's, `learn`'s batches' and the
/// validation budget's (L1: "the same priority orders `learn`'s batches
/// and the validation budget"): tier, then fewer unknown factors, then
/// value descending. Not total: [`order`] adds the id, and the harness
/// selection puts headroom ahead of it and the charter tiebreak after.
pub fn order_by_priority(a: &Priority, b: &Priority) -> Ordering {
    a.tier()
        .cmp(&b.tier())
        .then_with(|| a.unknown.len().cmp(&b.unknown.len()))
        .then_with(|| b.value().partial_cmp(&a.value()).unwrap_or(Ordering::Equal))
}

/// [`order_by_priority`], then the id — total, so a seed is not a lie.
pub fn order(a: (&Priority, &str), b: (&Priority, &str)) -> Ordering {
    order_by_priority(a.0, b.0).then_with(|| a.1.cmp(b.1))
}

/// Sort `items` by [`order`] over the priority and id `key` gives each.
pub fn sort_by_priority<T>(items: &mut [T], key: impl Fn(&T) -> (&Priority, &str)) {
    items.sort_by(|a, b| order(key(a), key(b)));
}

// ─── The hopeless ──────────────────────────────────────────────────────

/// Which episodes the harness has spent selection slots on without
/// anything winning — read from the records that already exist.
///
/// - **A night** is a distinct UTC date of a measurement
///   (`Measurement::measured_at`): `ruminate` measures once a night.
/// - **High** is being in a measurement's *selection* slice
///   (`Measurement::episodes` less `holdout_episodes`) — the prioritised
///   draw put it there. The holdout is uniform and says nothing about
///   priority, so it never counts.
/// - **Winning** is a candidate that selected the episode and was accepted
///   — by the gate (`disposition: accept`) or by the owner (`accepted`,
///   and `reverted`, which was accepted first) — or a point-wise comparison
///   on the episode's session that preferred a candidate arm (2d-2's
///   `Role::Candidate`, `Verdict::Separated`).
///
/// Only nights after the latest win count, so an episode something learned
/// from starts over.
///
/// **A night is losing unless something won, not only when something
/// lost**: a `staged` proposal waiting on the owner counts, as a rejection
/// does. That is the design's rule (APPRAISAL-WIRING-DESIGN L1, "since
/// anything last won"), and it is deliberate: an episode whose change is
/// already in the owner's queue is not re-spent a slot a night while it
/// waits, and accepting the proposal is a win that starts it over — the
/// demotion lasts only while the queue is unread.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct History {
    losing: BTreeMap<String, BTreeSet<NaiveDate>>,
    last_win: BTreeMap<String, NaiveDate>,
}

impl History {
    /// Built from the harness store and the comparison store. The count is
    /// of measurements whose date could not be read: a record that cannot
    /// be placed makes every answer a floor, which the caller treats as
    /// unknown.
    pub fn from_records(
        candidates: &[HarnessCandidate],
        comparisons: &[Comparison],
    ) -> (History, usize) {
        let mut h = History::default();
        let mut unreadable = 0usize;
        for c in candidates {
            let Some(m) = &c.measurement else {
                continue;
            };
            let Some(night) = DateTime::parse_from_rfc3339(&m.measured_at)
                .ok()
                .map(|t| t.with_timezone(&Utc).date_naive())
            else {
                unreadable += 1;
                continue;
            };
            let won = m.disposition == "accept"
                || c.status == STATUS_ACCEPTED
                || c.status == STATUS_REVERTED;
            let held: BTreeSet<&str> = m.holdout_episodes.iter().map(String::as_str).collect();
            for id in m.episodes.iter().filter(|e| !held.contains(e.as_str())) {
                if won {
                    h.win(id, night);
                } else {
                    h.losing.entry(id.clone()).or_default().insert(night);
                }
            }
        }
        for c in comparisons {
            let candidate_won = c.verdict == Verdict::Separated
                && c.preferred
                    .iter()
                    .any(|&i| c.arms.get(i).is_some_and(|a| a.role == Role::Candidate));
            if candidate_won && !c.pointers.session_id.is_empty() {
                h.win(&c.pointers.session_id, c.at.date_naive());
            }
        }
        (h, unreadable)
    }

    fn win(&mut self, id: &str, night: NaiveDate) {
        let at = self.last_win.entry(id.to_string()).or_insert(night);
        *at = (*at).max(night);
    }

    /// Losing nights since the latest win.
    pub fn losing_nights(&self, id: &str) -> usize {
        let since = self.last_win.get(id);
        self.losing.get(id).map_or(0, |d| {
            d.iter().filter(|d| since.is_none_or(|w| *d > w)).count()
        })
    }

    pub fn hopeless(&self, id: &str) -> bool {
        self.losing_nights(id) >= HOPELESS_NIGHTS
    }

    /// The default stores. `Err` where either store, or any record of it,
    /// could not be read — the hopeless mark is then unknown for every
    /// episode, never "not hopeless".
    pub fn load() -> Result<History> {
        let candidates = match crate::harness::HarnessStore::open_existing_default() {
            None => Vec::new(),
            Some(store) => {
                let (all, skipped) = store.all_counting()?;
                anyhow::ensure!(skipped == 0, "{skipped} harness candidate(s) unreadable");
                all
            }
        };
        let comparisons = match crate::comparison::ComparisonStore::open_existing_default() {
            None => Vec::new(),
            Some(store) => {
                let (rows, skipped) = store.comparisons_counting()?;
                anyhow::ensure!(skipped == 0, "{skipped} comparison line(s) unreadable");
                rows
            }
        };
        let (h, unreadable) = History::from_records(&candidates, &comparisons);
        anyhow::ensure!(
            unreadable == 0,
            "{unreadable} measurement(s) with an unreadable date"
        );
        Ok(h)
    }
}

// ─── Need: the Selector's demand term ──────────────────────────────────

/// How many recent runs were matched in each region — the demand ledger's
/// `touches`, from the session store.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Recurrence {
    counts: BTreeMap<String, usize>,
    /// Sessions in the window whose run record could not be read — counted,
    /// so a caller can say the counts are floors.
    pub unreadable: usize,
    /// Admitted sessions in the window past [`RECURRENCE_SCAN_CAP`], the
    /// oldest: not walked, so their regions' counts are floors too.
    pub beyond_cap: usize,
}

impl Recurrence {
    /// From region keys directly — one per run.
    pub fn from_keys(keys: impl IntoIterator<Item = String>) -> Recurrence {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for k in keys {
            *counts.entry(k).or_default() += 1;
        }
        Recurrence {
            counts,
            unreadable: 0,
            beyond_cap: 0,
        }
    }

    /// Walk the admitted sessions of the last [`RECURRENCE_WINDOW_DAYS`],
    /// newest first and at most [`RECURRENCE_SCAN_CAP`], reading each one's
    /// last run record (`Session::run_configs_streaming`, a substring scan)
    /// for the region it was matched in. Admission is the corpus's
    /// (`runlog::Scan`): a smoke test or an experiment trial is the harness
    /// reading itself, which the Selector's ledger refuses to count as
    /// demand for the same reason.
    pub fn scan(sessions_dir: &Path, now: DateTime<Utc>) -> Result<Recurrence> {
        let admission = crate::runlog::Scan {
            since: Some(now - chrono::Duration::days(RECURRENCE_WINDOW_DAYS)),
            ..Default::default()
        };
        // Counting: a session whose header does not read is dropped by the
        // listing, and one in the window would lower its region's count
        // with nothing saying the count is a floor.
        let (mut listed, torn) = Session::list_counting(sessions_dir)?;
        listed.retain(|(meta, _)| admission.admits(meta));
        listed.sort_by_key(|(meta, _)| std::cmp::Reverse(meta.created_at));
        let mut out = Recurrence {
            unreadable: torn,
            ..Recurrence::default()
        };
        out.beyond_cap = listed.len().saturating_sub(RECURRENCE_SCAN_CAP);
        for (_, path) in listed.into_iter().take(RECURRENCE_SCAN_CAP) {
            match Session::run_configs_streaming(&path) {
                Ok(configs) => {
                    if let Some(key) = configs
                        .last()
                        .map(Situation::of_record)
                        .and_then(|s| s.region_key())
                    {
                        *out.counts.entry(key).or_default() += 1;
                    }
                }
                Err(_) => out.unreadable += 1,
            }
        }
        Ok(out)
    }

    /// Runs in `key`'s region, at least the episode itself; `None` for a
    /// region that cannot be keyed.
    pub fn of(&self, key: Option<&str>) -> Option<usize> {
        key.map(|k| self.counts.get(k).copied().unwrap_or(0).max(1))
    }
}

/// Need for `n` recent runs in the region: `ln(1 + n)`, `n` at least the
/// episode itself.
fn need_of(n: usize) -> f64 {
    (1.0 + n.max(1) as f64).ln()
}

/// An unknown need's stand-in — [`need_of`] the episode alone.
const NEED_FLOOR: f64 = std::f64::consts::LN_2;

/// One line for a pass's log: how many of the priorities it ordered by had
/// a factor it could not read, and which — the per-episode half of
/// [`Ranker::caveats`], which names only the stores a whole pass could not
/// load, so one session whose appraisal did not build was otherwise said
/// nowhere (found on review). `None` when every priority was fully known.
pub fn unknown_summary<'a>(priorities: impl IntoIterator<Item = &'a Priority>) -> Option<String> {
    let mut total = 0usize;
    let mut with_unknown = 0usize;
    let mut by_factor: BTreeMap<Factor, usize> = BTreeMap::new();
    for p in priorities {
        total += 1;
        if !p.unknown.is_empty() {
            with_unknown += 1;
        }
        for f in &p.unknown {
            *by_factor.entry(*f).or_default() += 1;
        }
    }
    (with_unknown > 0).then(|| {
        let named: Vec<String> = by_factor
            .iter()
            .map(|(f, n)| format!("{} ×{n}", f.as_str()))
            .collect();
        format!(
            "{with_unknown} of {total} ranked with an unknown factor ({})",
            named.join(", ")
        )
    })
}

// ─── The ranker ────────────────────────────────────────────────────────

/// One caveat per appraisal store that did not load. Any one of them makes
/// every appraisal partial and so every session's owner verdicts unknown —
/// the charter was the only one said, and a torn outbox line took the
/// verdicts out of every priority with nothing printed (found on review).
fn store_caveats(stores: &Stores) -> Vec<String> {
    stores
        .unreadable()
        .into_iter()
        .map(|name| {
            format!(
                "the {name} could not be read: every appraisal is partial, so every \
                 session's owner verdicts are unknown"
            )
        })
        .collect()
}

/// Everything a priority is read from, loaded once per pass.
pub struct Ranker {
    now: DateTime<Utc>,
    stores: Stores,
    /// Clean surprises per session; `None` where the ledger did not load.
    surprises: Option<BTreeMap<String, usize>>,
    recurrence: Option<Recurrence>,
    history: Option<History>,
    /// What could not be read, for the pass's log.
    caveats: Vec<String>,
}

impl Ranker {
    /// Read the default stores under the mecha home and walk
    /// `sessions_dir` for recurrence. Nothing fails the caller: what cannot
    /// be read is an unknown factor on every priority, and a caveat.
    pub fn load(sessions_dir: &Path, now: DateTime<Utc>) -> Ranker {
        let stores = Stores::load();
        let mut caveats = Vec::new();
        let surprises = match crate::appraisal_store::AppraisalStore::open_existing_default() {
            None => Some(BTreeMap::new()),
            Some(store) => match store.scores() {
                Ok((rows, 0)) => Some(clean_surprises(&rows)),
                Ok((_, skipped)) => {
                    caveats.push(format!(
                        "{skipped} line(s) of the appraisal score ledger unreadable: surprises unknown"
                    ));
                    None
                }
                Err(e) => {
                    caveats.push(format!(
                        "the appraisal score ledger could not be read ({e:#}): surprises unknown"
                    ));
                    None
                }
            },
        };
        let recurrence = match Recurrence::scan(sessions_dir, now) {
            Ok(r) => {
                if r.unreadable > 0 {
                    caveats.push(format!(
                        "{} session(s) whose header or run record could not be read: recurrence counts are floors",
                        r.unreadable
                    ));
                }
                if r.beyond_cap > 0 {
                    caveats.push(format!(
                        "{} older session(s) in the window past the {RECURRENCE_SCAN_CAP}-session scan: recurrence counts are floors",
                        r.beyond_cap
                    ));
                }
                Some(r)
            }
            Err(e) => {
                caveats.push(format!(
                    "the session store could not be walked ({e:#}): recurrence unknown"
                ));
                None
            }
        };
        let history = match History::load() {
            Ok(h) => Some(h),
            Err(e) => {
                caveats.push(format!(
                    "the harness or comparison store could not be read ({e:#}): hopeless mark unknown"
                ));
                None
            }
        };
        caveats.extend(store_caveats(&stores));
        Ranker::from_parts(now, stores, surprises, recurrence, history, caveats)
    }

    /// From parts a caller already holds — a test's fixture.
    pub fn from_parts(
        now: DateTime<Utc>,
        stores: Stores,
        surprises: Option<BTreeMap<String, usize>>,
        recurrence: Option<Recurrence>,
        history: Option<History>,
        caveats: Vec<String>,
    ) -> Ranker {
        Ranker {
            now,
            stores,
            surprises,
            recurrence,
            history,
            caveats,
        }
    }

    /// What this pass could not read, one line each.
    pub fn caveats(&self) -> &[String] {
        &self.caveats
    }

    /// The stores the priority's appraisal was built from — the draw
    /// reads the charter rank off the same appraisal.
    pub fn stores(&self) -> &Stores {
        &self.stores
    }

    /// One session's priority, off a transcript already read, and the
    /// appraisal it was built from (`None` where none could be built).
    pub fn of_transcript(&self, t: &Transcript) -> (Priority, Option<appraisal::Appraisal>) {
        let id = t.meta.id.as_str();
        let drafts = self.stores.drafts_of(id);
        let built = appraisal::for_transcript(
            t,
            id,
            t.meta.created_at.to_rfc3339(),
            self.stores.records(&drafts),
            None,
        )
        .map(|b| b.appraisal);
        // A partial appraisal was built with a store it reads missing, so
        // its owner verdicts are a floor — unknown, never "these and no more".
        let owner_gain = built
            .as_ref()
            .filter(|a| !a.partial)
            .map(|a| appraisal::owner_verdict_gain(a, self.stores.charter.as_ref()));
        let key = t
            .configs
            .last()
            .map(Situation::of_record)
            .and_then(|s| s.region_key());
        let inputs = Inputs {
            owner_gain,
            surprises: self
                .surprises
                .as_ref()
                .map(|m| m.get(id).copied().unwrap_or(0)),
            recurrence: self.recurrence.as_ref().and_then(|r| r.of(key.as_deref())),
            age_days: (self.now - t.meta.created_at).num_seconds() as f64 / 86_400.0,
            hopeless: self.history.as_ref().map(|h| h.hopeless(id)),
        };
        (Priority::of(&inputs), built)
    }

    /// Many sessions' priorities by exact id, each transcript read once.
    /// The store is listed once for all of them — `Session::find` lists it
    /// per call, which over a pass's reflections is a header walk per id.
    /// A session that cannot be found or read has every factor unknown, as
    /// does every id when the store cannot be listed.
    pub fn of_sessions<'a>(
        &self,
        sessions_dir: &Path,
        ids: impl IntoIterator<Item = &'a str>,
    ) -> BTreeMap<String, Priority> {
        let paths: BTreeMap<String, std::path::PathBuf> = Session::list(sessions_dir)
            .map(|listed| listed.into_iter().map(|(m, p)| (m.id, p)).collect())
            .unwrap_or_default();
        let mut out = BTreeMap::new();
        for id in ids {
            if out.contains_key(id) {
                continue;
            }
            let priority = match paths.get(id).map(|p| Session::read(p)) {
                Some(Ok(t)) => self.of_transcript(&t).0,
                _ => Priority::unread(),
            };
            out.insert(id.to_string(), priority);
        }
        out
    }
}

/// Clean misses per session. A surprise on a tainted appraisal is the
/// model's own expected act under whatever it read, and could be steered
/// into a miss; it never raises a priority.
pub fn clean_surprises(scores: &[crate::appraisal_store::Score]) -> BTreeMap<String, usize> {
    let mut out: BTreeMap<String, usize> = BTreeMap::new();
    for s in scores.iter().filter(|s| s.surprise && s.clean) {
        *out.entry(s.session_id.clone()).or_default() += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> Inputs {
        Inputs {
            owner_gain: Some(1.0),
            surprises: Some(0),
            recurrence: Some(1),
            age_days: 0.0,
            hopeless: Some(false),
        }
    }

    fn rank(ps: &[(&str, Priority)]) -> Vec<String> {
        let mut v: Vec<(String, Priority)> =
            ps.iter().map(|(i, p)| (i.to_string(), p.clone())).collect();
        sort_by_priority(&mut v, |(id, p)| (p, id.as_str()));
        v.into_iter().map(|(id, _)| id).collect()
    }

    /// The row's acceptance line: equal error, and the region seen five
    /// times this month outranks the one seen once. The ids sort the other
    /// way, so an order that ignored recurrence would put the one-off first.
    #[test]
    fn a_recurring_region_outranks_a_one_off_of_equal_error() {
        let one_off = Priority::of(&inputs());
        let recurring = Priority::of(&Inputs {
            recurrence: Some(5),
            ..inputs()
        });
        assert_eq!(one_off.gain, recurring.gain, "equal error");
        assert_eq!(
            rank(&[("a-one-off", one_off), ("b-recurring", recurring)]),
            ["b-recurring", "a-one-off"]
        );
    }

    #[test]
    fn a_surprise_raises_priority() {
        let plain = Priority::of(&inputs());
        let surprised = Priority::of(&Inputs {
            surprises: Some(1),
            ..inputs()
        });
        assert!(surprised.value() > plain.value());
        assert_eq!(
            rank(&[("a-plain", plain), ("b-surprised", surprised)]),
            ["b-surprised", "a-plain"]
        );
        // A miss alone — no owner verdict — is still something to learn
        // from, not a known zero.
        let only_a_miss = Priority::of(&Inputs {
            owner_gain: Some(0.0),
            surprises: Some(1),
            ..inputs()
        });
        assert_eq!(only_a_miss.tier(), 0);
        assert!(only_a_miss.value() > 0.0);
    }

    #[test]
    fn a_hopeless_episode_is_demoted_below_everything() {
        let hopeless = Priority::of(&Inputs {
            owner_gain: Some(5.0),
            recurrence: Some(9),
            hopeless: Some(true),
            ..inputs()
        });
        let zero = Priority::of(&Inputs {
            owner_gain: Some(0.0),
            ..inputs()
        });
        let small = Priority::of(&inputs());
        assert!(hopeless.value() > small.value());
        assert_eq!(
            rank(&[("a", hopeless), ("b", zero), ("c", small)]),
            ["c", "b", "a"]
        );
    }

    /// An unreadable factor is unknown: not zero (it ranks above a known
    /// zero), not a free pass (it ranks below every fully known positive
    /// priority, however small), and named.
    #[test]
    fn an_unreadable_factor_is_unknown_not_zero() {
        let unread_scores = Priority::of(&Inputs {
            owner_gain: Some(3.0),
            surprises: None,
            ..inputs()
        });
        assert_eq!(
            unread_scores.unknown,
            BTreeSet::from([Factor::Surprises]),
            "named"
        );
        assert!(!unread_scores.known_zero);
        let known_small = Priority::of(&Inputs {
            owner_gain: Some(0.5),
            ..inputs()
        });
        let known_zero = Priority::of(&Inputs {
            owner_gain: Some(0.0),
            ..inputs()
        });
        assert!(unread_scores.value() > known_small.value());
        assert_eq!(
            rank(&[
                ("a-zero", known_zero),
                ("b-unknown", unread_scores),
                ("c-small", known_small)
            ]),
            ["c-small", "b-unknown", "a-zero"]
        );
        // Every unknown factor is unknown, and an unreadable session has
        // all of them — which still ranks above a known zero.
        for p in [
            Priority::of(&Inputs {
                owner_gain: None,
                ..inputs()
            }),
            Priority::of(&Inputs {
                recurrence: None,
                ..inputs()
            }),
            Priority::of(&Inputs {
                hopeless: None,
                ..inputs()
            }),
            Priority::unread(),
        ] {
            assert_eq!(p.tier(), 1, "{p:?}");
            assert!(!p.known_zero);
        }
    }

    #[test]
    fn age_halves_the_priority_every_half_life() {
        let fresh = Priority::of(&inputs());
        let old = Priority::of(&Inputs {
            age_days: AGE_HALF_LIFE_DAYS,
            ..inputs()
        });
        assert!((old.value() * 2.0 - fresh.value()).abs() < 1e-9);
    }

    fn candidate(
        id: &str,
        night: &str,
        status: &str,
        disposition: &str,
        selection: &[&str],
        holdout: &[&str],
    ) -> HarnessCandidate {
        let mut episodes: Vec<String> = selection.iter().map(|s| s.to_string()).collect();
        episodes.extend(holdout.iter().map(|s| s.to_string()));
        serde_json::from_value(serde_json::json!({
            "id": id, "created_at": night, "class": "config", "change": "max_turns=9",
            "metric": "turns", "rationale": "", "evidence": "", "status": status,
            "measurement": {
                "measured_at": night, "model": "m", "disposition": disposition,
                "reason": "", "selection": {"wins": 0, "losses": 0, "ties": 0},
                "holdout": {"wins": 0, "losses": 0, "ties": 0},
                "work_baseline": 0, "work_candidate": 0,
                "episodes": episodes, "holdout_episodes": holdout,
                "diverged": [], "skipped": 0,
            }
        }))
        .unwrap()
    }

    /// Three nights in the selection with nothing winning is hopeless; two
    /// is not; a night in the holdout is not "high"; two candidates on one
    /// night are one night; and a win resets the count.
    #[test]
    fn the_hopeless_rule_reads_nights_high_and_winning_from_the_records() {
        let n1 = "2026-09-01T02:00:00Z";
        let n1b = "2026-09-01T03:00:00Z";
        let n2 = "2026-09-02T02:00:00Z";
        let n3 = "2026-09-03T02:00:00Z";
        let rows = vec![
            candidate("h1", n1, "rejected", "reject", &["ep-a", "ep-b"], &["ep-c"]),
            candidate("h2", n1b, "rejected", "reject", &["ep-a"], &[]),
            candidate("h3", n2, "rejected", "reject", &["ep-a", "ep-b"], &["ep-c"]),
            candidate("h4", n3, "staged", "propose", &["ep-a"], &["ep-c"]),
        ];
        let (h, unreadable) = History::from_records(&rows, &[]);
        assert_eq!(unreadable, 0);
        assert_eq!(h.losing_nights("ep-a"), 3, "two candidates on one night");
        assert!(h.hopeless("ep-a"));
        assert!(!h.hopeless("ep-b"), "two nights");
        assert_eq!(h.losing_nights("ep-c"), 0, "the holdout is never high");

        // An accepted candidate on the second night: only the third is since.
        let mut won = rows.clone();
        won.push(candidate("h5", n2, "accepted", "accept", &["ep-a"], &[]));
        let (h, _) = History::from_records(&won, &[]);
        assert_eq!(h.losing_nights("ep-a"), 1);
        assert!(!h.hopeless("ep-a"));

        // A point-wise comparison that preferred a candidate arm is a win.
        use crate::comparison::{Arm, Kind, Outcome, Pointers, Validator};
        let mut cmp = Comparison::new(
            Kind::PointSteer,
            None,
            None,
            None,
            vec![
                Arm::new(Role::Candidate, None, Outcome::Pass),
                Arm::new(Role::Rules, None, Outcome::Fail),
            ],
            Validator::StructuralSteer,
            Pointers {
                session_id: "ep-a".into(),
                ..Default::default()
            },
            "m",
        );
        cmp.at = DateTime::parse_from_rfc3339(n3)
            .unwrap()
            .with_timezone(&Utc);
        let (h, _) = History::from_records(&rows, &[cmp]);
        assert_eq!(h.losing_nights("ep-a"), 0, "the win on night three resets");
        assert!(!h.hopeless("ep-a"));

        // An unreadable measurement date is counted, never skipped quietly.
        let (_, unreadable) = History::from_records(
            &[candidate(
                "h9",
                "yesterday",
                "rejected",
                "reject",
                &["x"],
                &[],
            )],
            &[],
        );
        assert_eq!(unreadable, 1);
    }

    #[test]
    fn only_clean_misses_raise_priority() {
        let score = |session: &str, surprise: bool, clean: bool| {
            serde_json::from_value::<crate::appraisal_store::Score>(serde_json::json!({
                "id": "s", "scored_at": "2026-09-01T00:00:00Z", "appraisal_id": "a",
                "session_id": session, "expected": "released_unchanged",
                "actual": "rejected", "hit": !surprise, "surprise": surprise, "clean": clean,
            }))
            .unwrap()
        };
        let m = clean_surprises(&[
            score("ep-a", true, true),
            score("ep-a", true, true),
            score("ep-b", true, false),
            score("ep-c", false, true),
        ]);
        assert_eq!(m.get("ep-a"), Some(&2));
        assert_eq!(m.get("ep-b"), None, "a tainted appraisal's miss");
        assert_eq!(m.get("ep-c"), None, "a hit");
    }

    #[test]
    fn recurrence_counts_the_region_and_never_less_than_the_episode() {
        let r = Recurrence::from_keys([
            "shell on tui".to_string(),
            "shell on tui".into(),
            "web".into(),
        ]);
        assert_eq!(r.of(Some("shell on tui")), Some(2));
        assert_eq!(r.of(Some("never seen")), Some(1));
        assert_eq!(r.of(None), None);
    }

    #[test]
    fn a_torn_header_makes_the_recurrence_counts_floors() {
        let dir = std::env::temp_dir()
            .join("mecha-replay-priority-test")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("torn.jsonl"), "{\"type\":\"meta\",\"id\":").unwrap();
        let r = Recurrence::scan(&dir, Utc::now()).unwrap();
        assert_eq!(r.unreadable, 1, "the listing dropped it without a word");
        assert!(r.counts.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn every_store_that_makes_an_appraisal_partial_is_named() {
        assert!(store_caveats(&Stores::default()).is_empty());
        let torn_outbox = Stores {
            outbox_unreadable: true,
            ..Stores::default()
        };
        let said = store_caveats(&torn_outbox);
        assert_eq!(said.len(), 1);
        assert!(
            said[0].starts_with("the outbox could not be read"),
            "{said:?}"
        );
        let all = Stores {
            outbox_unreadable: true,
            questions_unreadable: true,
            frontdoor_unreadable: true,
            learning_unreadable: true,
            charter_unreadable: true,
            closures_unreadable: true,
            workflows_unreadable: true,
            ..Stores::default()
        };
        assert_eq!(store_caveats(&all).len(), 7);
    }

    /// The need half, end to end: run records on disk, keyed by their
    /// region through `Situation::of_record`, counted per region, a test
    /// session never demand. Every draw fixture writes one default record,
    /// so need was constant across every pool the suite ranked; a
    /// `region_key` that stopped naming ordinary records would have turned
    /// need unknown everywhere with the suite green (found on review).
    #[test]
    fn recurrence_is_counted_per_region_from_the_run_records() {
        use crate::session::{Record, RunConfig, SessionKind, SessionMeta};
        let dir = std::env::temp_dir()
            .join("mecha-replay-priority-test")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&dir).unwrap();
        let record = |surface: SessionKind| RunConfig {
            rules_surface: Some(surface),
            ..Default::default()
        };
        let write = |kind: SessionKind, surface: SessionKind| {
            let s = Session::create(
                &dir,
                SessionMeta {
                    id: Session::new_id(),
                    created_at: Utc::now(),
                    provider: "p".into(),
                    model: "m".into(),
                    workspace: "/w".into(),
                    title: None,
                    kind: Some(kind),
                },
            )
            .unwrap();
            s.append(&Record::Config(record(surface))).unwrap();
        };
        for _ in 0..3 {
            write(SessionKind::Web, SessionKind::Web);
        }
        write(SessionKind::Tui, SessionKind::Tui);
        // A smoke test is the harness reading itself, never demand.
        write(SessionKind::Test, SessionKind::Tui);

        let key = |surface| Situation::of_record(&record(surface)).region_key();
        let (web, tui) = (key(SessionKind::Web), key(SessionKind::Tui));
        assert!(web.is_some() && tui.is_some() && web != tui);
        let r = Recurrence::scan(&dir, Utc::now()).unwrap();
        assert_eq!(r.unreadable, 0);
        assert_eq!(r.of(web.as_deref()), Some(3));
        assert_eq!(r.of(tui.as_deref()), Some(1));
        let (common, rare) = (
            Priority::of(&Inputs {
                recurrence: r.of(web.as_deref()),
                ..inputs()
            }),
            Priority::of(&Inputs {
                recurrence: r.of(tui.as_deref()),
                ..inputs()
            }),
        );
        assert!(common.value() > rare.value());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// An unreadable need stands in at the floor — the episode alone — so
    /// not being read never outranks a known one-off of equal gain.
    #[test]
    fn an_unknown_need_is_never_a_promotion() {
        let unread = Priority::of(&Inputs {
            recurrence: None,
            ..inputs()
        });
        let one_off = Priority::of(&Inputs {
            recurrence: Some(1),
            ..inputs()
        });
        assert!(unread.need.is_none());
        assert_eq!(unread.value(), one_off.value());
    }

    #[test]
    fn a_pass_says_how_many_priorities_it_ranked_blind() {
        let known = Priority::of(&inputs());
        assert_eq!(unknown_summary([&known, &known]), None);
        let no_need = Priority::of(&Inputs {
            recurrence: None,
            ..inputs()
        });
        let unread = Priority::unread();
        assert_eq!(
            unknown_summary([&known, &no_need, &unread]).unwrap(),
            "2 of 3 ranked with an unknown factor (owner verdicts ×1, surprises ×1, \
             recurrence ×2, hopeless mark ×1)"
        );
    }
}
