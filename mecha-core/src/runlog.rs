//! The run-quality corpus: every recorded outcome, across every session.
//!
//! [`crate::session::RunStats`] is written one row per finished run. This is
//! the reader that puts them side by side, which is the whole point of having
//! recorded them: one run's counters say almost nothing, and a thousand runs'
//! counters say what normal looks like and when it stopped.
//!
//! ## Why this reads the transcripts instead of keeping a ledger
//!
//! A second file would be faster and would be a second source of truth. The
//! transcript already holds the rows, written by the process that produced
//! them; a ledger beside it can disagree with it, and then someone has to
//! decide which is right. Same reasoning as the TUI reading a trigger's last
//! answer back from the session record rather than caching it.
//!
//! The cost is that a scan reads files, so every scan is **bounded** —
//! newest-first, with a session cap and an optional cutoff. A corpus reader
//! that must read everything before it answers is one nobody runs
//! interactively, and doctor runs in one pass with no network and no model.
//!
//! ## What it deliberately does not do
//!
//! No judgement. Nothing here decides that a rate is bad, because "bad"
//! depends on what a run was for, and the thresholds belong with the reader
//! that acts on them. This module counts.

use crate::agent::StopCause;
use crate::session::{Record, RunStats, Session};
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;
use std::path::Path;

/// One finished run, with enough of its session to be identifiable.
#[derive(Debug, Clone)]
pub struct RunRow {
    pub session_id: String,
    /// When the *session* started. A resumed session's later runs share it;
    /// the transcript records no per-run stamp, and inventing one from file
    /// mtime would be a guess dressed as a measurement.
    pub started_at: DateTime<Utc>,
    pub provider: String,
    pub model: String,
    pub title: Option<String>,
    /// Which run within the session, 1-based. A resumed session has several.
    pub run: u32,
    pub stats: RunStats,
}

/// Every run the scan looked at, newest session first.
#[derive(Debug, Clone, Default)]
pub struct Corpus {
    pub rows: Vec<RunRow>,
    /// Sessions read, including those that contributed no rows — the
    /// denominator for "how much of the store did this answer come from".
    pub sessions_read: usize,
    /// Transcripts the scan could not read at all — a headerless file the
    /// listing skipped, or one whose body failed to parse. Never folded into
    /// `sessions_read`: an unreadable store is a finding, not an empty
    /// queue, and a store rotting one file at a time was invisible from
    /// every reader before this was counted.
    pub unreadable: usize,
}

/// How to bound a scan. Both limits are honest about cost rather than about
/// relevance: the caller decides how much reading it can afford.
#[derive(Debug, Clone, Default)]
pub struct Scan {
    /// Stop after this many sessions, newest first.
    pub max_sessions: Option<usize>,
    /// Skip sessions started before this.
    pub since: Option<DateTime<Utc>>,
}

impl Corpus {
    /// Read outcomes out of the session store.
    ///
    /// Best-effort per session, like every other reader over this store: an
    /// unreadable or torn transcript contributes nothing and does not stop
    /// the ones after it.
    pub fn scan(dir: &Path, scan: &Scan) -> Result<Corpus> {
        let mut out = Corpus::default();
        let (listed, skipped) = Session::list_counting(dir)?;
        out.unreadable = skipped;
        for (meta, path) in listed {
            if scan.since.is_some_and(|t| meta.created_at < t) {
                continue;
            }
            if scan.max_sessions.is_some_and(|n| out.sessions_read >= n) {
                break;
            }
            // Attributed rather than taken from the header: a mid-session
            // model switch writes a `Config`, and crediting those runs to the
            // header's model would defeat `by_model` in the one case where a
            // corpus genuinely blends two.
            //
            // Counted as read only *after* the read succeeds — found on
            // review: incrementing first put a torn-body transcript in both
            // counters, which is exactly the "never folded into
            // `sessions_read`" the field's own doc promises, broken two
            // lines down. The two numbers are disjoint by construction now.
            let Ok(rows) = Session::outcomes_attributed(&path) else {
                out.unreadable += 1;
                continue;
            };
            out.sessions_read += 1;
            for (i, (provider, model, s)) in rows.into_iter().enumerate() {
                out.rows.push(RunRow {
                    session_id: meta.id.clone(),
                    started_at: meta.created_at,
                    provider,
                    model,
                    title: meta.title.clone(),
                    run: i as u32 + 1,
                    stats: s,
                });
            }
        }
        Ok(out)
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Keep only the rows a predicate accepts. `sessions_read` and
    /// `unreadable` are preserved, because they describe the scan and not
    /// the selection.
    pub fn filter(&self, keep: impl Fn(&RunRow) -> bool) -> Corpus {
        Corpus {
            rows: self.rows.iter().filter(|r| keep(r)).cloned().collect(),
            sessions_read: self.sessions_read,
            unreadable: self.unreadable,
        }
    }

    pub fn tool_calls(&self) -> u64 {
        self.rows
            .iter()
            .map(|r| u64::from(r.stats.tool_calls))
            .sum()
    }

    pub fn tool_errors(&self) -> u64 {
        self.rows
            .iter()
            .map(|r| u64::from(r.stats.tool_errors))
            .sum()
    }

    /// Share of attempted calls the environment refused.
    ///
    /// `None` when nothing was attempted — the denominator is zero, and a
    /// rate over no calls is undefined rather than perfect. That distinction
    /// is the one a threshold silent on zero gets wrong, which is how a
    /// trigger that stopped working entirely read as healthy.
    pub fn tool_error_rate(&self) -> Option<f64> {
        let calls = self.tool_calls();
        (calls > 0).then(|| self.tool_errors() as f64 / calls as f64)
    }

    /// Runs that decided they were done with their last call failed.
    pub fn ended_on_failed_call(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| r.stats.ended_on_failed_call)
            .count()
    }

    /// Share of runs that did. `None` on an empty corpus, for the reason
    /// above.
    pub fn rate_of(&self, of: impl Fn(&RunRow) -> bool) -> Option<f64> {
        (!self.rows.is_empty())
            .then(|| self.rows.iter().filter(|r| of(r)).count() as f64 / self.rows.len() as f64)
    }

    /// How runs ended. Runs recorded before `stop_cause` existed, or written
    /// by a path that did not set it, count under `None` rather than being
    /// assumed complete.
    pub fn stop_causes(&self) -> BTreeMap<Option<StopCause>, usize> {
        let mut out = BTreeMap::new();
        for row in &self.rows {
            *out.entry(row.stats.stop_cause).or_insert(0) += 1;
        }
        out
    }

    pub fn compactions(&self) -> u64 {
        self.rows
            .iter()
            .map(|r| u64::from(r.stats.compactions))
            .sum()
    }

    /// Overflow recoveries, and how many rows had the sensor.
    ///
    /// A pair for `cost_usd`'s reason one field over: a total drawn from part
    /// of the corpus is a lower bound, and one that does not say so is a wrong
    /// number. Here the stakes are sharper than for cost, because the corpus
    /// this is read from deliberately spans the introduction of the field —
    /// so a caller that ignores the second element is comparing runs that
    /// could report an overflow against runs that could not.
    pub fn context_overflows(&self) -> (u64, usize) {
        let sensed: Vec<u32> = self
            .rows
            .iter()
            .filter_map(|r| r.stats.context_overflows)
            .collect();
        (sensed.iter().map(|n| u64::from(*n)).sum(), sensed.len())
    }

    /// Share of runs that hit at least one overflow, over the rows that could
    /// have reported one.
    ///
    /// `None` when no row carried the sensor — not zero, which would make a
    /// corpus written before the field indistinguishable from one where the
    /// threshold never failed.
    pub fn overflow_rate(&self) -> Option<f64> {
        let sensed: Vec<u32> = self
            .rows
            .iter()
            .filter_map(|r| r.stats.context_overflows)
            .collect();
        (!sensed.is_empty())
            .then(|| sensed.iter().filter(|n| **n > 0).count() as f64 / sensed.len() as f64)
    }

    /// Share of runs the harness told at least once that an approach had
    /// stopped teaching them anything, over the rows that could have reported
    /// it (`GOAL-SYSTEM-DESIGN.md` §9.1).
    ///
    /// The number every threshold in `boredom.rs` is answerable against, and
    /// it is a rate rather than a total on purpose: what the constants get
    /// wrong is *how often* a run is spoken to, and a total over a corpus of
    /// unknown size answers that only if you already know the size.
    ///
    /// `None` over no sensed rows, like every rate here — a corpus written
    /// before the detector existed and one where nothing ever got stuck are
    /// opposite findings.
    pub fn boredom_rate(&self) -> Option<f64> {
        let sensed: Vec<u32> = self
            .rows
            .iter()
            .filter_map(|r| r.stats.boredom_notices)
            .collect();
        (!sensed.is_empty())
            .then(|| sensed.iter().filter(|n| **n > 0).count() as f64 / sensed.len() as f64)
    }

    /// Average of `Homeostat::peak_context_pressure` over the rows that
    /// sensed it (`GOAL-SYSTEM-DESIGN.md` §4, feeding `diagnose::Evidence`).
    ///
    /// A mean rather than a rate against a threshold, on purpose: this module
    /// counts and never judges, and a fixed "high pressure" cutoff would be
    /// exactly the judgement the reader — `diagnose::Evidence`'s consumer —
    /// is supposed to make. `None` over no sensed rows, like every reading
    /// here: a corpus written before the homeostat existed and one where
    /// every run had headroom to spare are opposite findings.
    pub fn mean_peak_context_pressure(&self) -> Option<f64> {
        let sensed: Vec<f64> = self
            .rows
            .iter()
            .filter_map(|r| r.stats.homeostat.as_ref())
            .filter_map(|h| h.peak_context_pressure)
            .map(f64::from)
            .collect();
        (!sensed.is_empty()).then(|| sensed.iter().sum::<f64>() / sensed.len() as f64)
    }

    /// Average of `Homeostat::anticipated_guilt` over the rows that sensed
    /// it. See [`crate::guilt`] — the sensor has no consumer yet, and this is
    /// the corpus existing before anything is built on it, same as every
    /// other reading here.
    ///
    /// `None` over no sensed rows, not zero — a corpus predating the sensor
    /// must not read as one where nothing was ever owed.
    pub fn mean_anticipated_guilt(&self) -> Option<f64> {
        let sensed: Vec<f64> = self
            .rows
            .iter()
            .filter_map(|r| r.stats.homeostat.as_ref())
            .filter_map(|h| h.anticipated_guilt)
            .map(f64::from)
            .collect();
        (!sensed.is_empty()).then(|| sensed.iter().sum::<f64>() / sensed.len() as f64)
    }

    /// Total cost, and how many rows knew theirs. Reported as a pair because
    /// a total over partial data is a lower bound, and one that does not say
    /// so is a wrong number.
    pub fn cost_usd(&self) -> (f64, usize) {
        let priced: Vec<f64> = self.rows.iter().filter_map(|r| r.stats.cost_usd).collect();
        // `+ 0.0` normalizes the sign: Rust's `Sum for f64` folds from -0.0 to
        // preserve the sign of a negative-zero summand, so an empty corpus
        // otherwise reports a cost of `-0.00`, which reads as a bug in the
        // price table rather than as an absence of priced runs.
        let total: f64 = priced.iter().sum::<f64>() + 0.0;
        (total, priced.len())
    }

    /// Split by model, so a rate can be read against the thing that produced
    /// it. A corpus spanning two models has no single error rate worth
    /// quoting.
    pub fn by_model(&self) -> BTreeMap<String, Corpus> {
        let mut out: BTreeMap<String, Corpus> = BTreeMap::new();
        for row in &self.rows {
            let bucket = out.entry(row.model.clone()).or_default();
            bucket.rows.push(row.clone());
            // The scan's denominator, not the slice's: `sessions_read`
            // describes how much of the store was looked at, which is the
            // same for every bucket. Left at zero it reads as "from 0
            // sessions", which is a lie in the one direction that matters —
            // it makes a well-sampled rate look like it came from nowhere.
            bucket.sessions_read = self.sessions_read;
        }
        out
    }
}

/// Every `Record` variant a corpus scan ignores, named so the compiler
/// complains when a new one appears and nobody decided what it means here.
#[allow(dead_code)]
fn exhaustive(record: &Record) {
    match record {
        Record::Meta(_)
        | Record::Message(_)
        | Record::Summary { .. }
        | Record::Config(_)
        | Record::Taint(_)
        | Record::Rewrite { .. }
        | Record::Outcome(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Taint;
    use crate::message::Usage;
    use crate::session::SessionMeta;
    use std::path::PathBuf;

    fn tmpdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mecha-runlog-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn session_with(dir: &Path, id: &str, model: &str, runs: Vec<RunStats>) -> Session {
        let s = Session::create(
            dir,
            SessionMeta {
                id: id.to_string(),
                created_at: DateTime::parse_from_rfc3339("2026-08-01T00:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                provider: "local".into(),
                model: model.to_string(),
                workspace: PathBuf::from("/tmp"),
                title: None,
            },
        )
        .unwrap();
        for stats in runs {
            s.append(&Record::Outcome(stats)).unwrap();
        }
        s
    }

    fn stats(calls: u32, errors: u32, ended_failed: bool, cause: StopCause) -> RunStats {
        RunStats {
            homeostat: None,
            context_overflows: None,
            boredom_notices: None,
            step_escalations_attempted: None,
            step_escalations_revised: None,
            turns: 3,
            usage: Usage::default(),
            cost_usd: Some(0.25),
            usage_complete: true,
            stop_cause: Some(cause),
            exhausted: false,
            ended_on_failed_call: ended_failed,
            tool_calls: calls,
            tool_errors: errors,
            tool_denied: 0,
            tool_staged: 0,
            malformed_tool_args: 0,
            blocked_sends: 0,
            compactions: 1,
            taint: Taint::default(),
        }
    }

    /// The whole reason `context_overflows` is an `Option` where every other
    /// counter here is a plain `u32`.
    ///
    /// This corpus is the shape the field will actually be read in: rows from
    /// before the sensor existed sitting beside rows from after it. Under a
    /// plain `u32` the old rows arrive as *zero overflows* and land in the
    /// denominator, so the rate they dilute is the one the field was added to
    /// establish — a change measured against it would look better the more
    /// stale corpus it was averaged over.
    #[test]
    fn a_row_without_the_sensor_is_unknown_and_never_a_zero() {
        let dir = tmpdir();
        let sensed = |n: u32| {
            let mut st = stats(4, 0, false, StopCause::Completed);
            st.context_overflows = Some(n);
            st
        };
        session_with(
            &dir,
            "20260801T000000-mixed",
            "opus",
            vec![
                // Written before the field existed: knows nothing.
                stats(4, 0, false, StopCause::Completed),
                sensed(0),
                sensed(3),
            ],
        );

        let corpus = Corpus::scan(&dir, &Scan::default()).unwrap();
        assert_eq!(corpus.len(), 3, "all three rows are in the corpus");
        // Three recoveries, but only two rows could have reported any — and
        // the pair says so rather than implying a rate over three.
        assert_eq!(corpus.context_overflows(), (3, 2));
        // One of the two sensed rows hit an overflow. Reading the unsensed row
        // as a clean run would give 1/3 here, which is the quiet dilution.
        assert_eq!(corpus.overflow_rate(), Some(0.5));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// "Nobody has the sensor" and "nobody overflowed" are opposite findings,
    /// and a corpus predating the field must not report the reassuring one.
    #[test]
    fn a_corpus_with_no_sensor_at_all_has_no_rate() {
        let dir = tmpdir();
        session_with(
            &dir,
            "20260801T000000-old",
            "opus",
            vec![stats(4, 0, false, StopCause::Completed)],
        );

        let corpus = Corpus::scan(&dir, &Scan::default()).unwrap();
        assert_eq!(corpus.context_overflows(), (0, 0));
        assert_eq!(corpus.overflow_rate(), None, "not Some(0.0)");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `boredom_notices` is the same shape as `context_overflows` — an
    /// `Option` because a run written before the sensor existed must not
    /// read as a run it definitely fired zero times in.
    #[test]
    fn a_row_without_the_boredom_sensor_is_unknown_and_never_a_zero() {
        let dir = tmpdir();
        let sensed = |n: u32| {
            let mut st = stats(4, 0, false, StopCause::Completed);
            st.boredom_notices = Some(n);
            st
        };
        session_with(
            &dir,
            "20260801T000000-mixed",
            "opus",
            vec![
                // Written before the field existed: knows nothing.
                stats(4, 0, false, StopCause::Completed),
                sensed(0),
                sensed(1),
            ],
        );

        let corpus = Corpus::scan(&dir, &Scan::default()).unwrap();
        assert_eq!(corpus.len(), 3, "all three rows are in the corpus");
        // One of the two sensed rows was told at least once. Reading the
        // unsensed row as a quiet one would give 1/3 here, which is the
        // same dilution `overflow_rate` exists to avoid.
        assert_eq!(corpus.boredom_rate(), Some(0.5));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// "Nobody has the sensor" and "nobody ever got stuck" are opposite
    /// findings, and a corpus predating the field must not report the
    /// reassuring one.
    #[test]
    fn a_corpus_with_no_boredom_sensor_at_all_has_no_rate() {
        let dir = tmpdir();
        session_with(
            &dir,
            "20260801T000000-old",
            "opus",
            vec![stats(4, 0, false, StopCause::Completed)],
        );

        let corpus = Corpus::scan(&dir, &Scan::default()).unwrap();
        assert_eq!(corpus.boredom_rate(), None, "not Some(0.0)");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_row_without_a_homeostat_snapshot_is_unknown_and_never_a_zero_for_either_mean() {
        let dir = tmpdir();
        let sensed = |pressure: f32, guilt: f32| {
            let mut st = stats(4, 0, false, StopCause::Completed);
            st.homeostat = Some(crate::homeostat::Homeostat {
                peak_context_pressure: Some(pressure),
                anticipated_guilt: Some(guilt),
                ..Default::default()
            });
            st
        };
        session_with(
            &dir,
            "20260801T000000-mixed",
            "opus",
            vec![
                // Written before Homeostat was recorded: knows nothing.
                stats(4, 0, false, StopCause::Completed),
                sensed(0.25, 0.0),
                sensed(0.75, 0.5),
            ],
        );

        let corpus = Corpus::scan(&dir, &Scan::default()).unwrap();
        assert_eq!(corpus.len(), 3, "all three rows are in the corpus");
        // Averaged over the two sensed rows only — the unsensed row must not
        // dilute it toward zero, the same dilution `boredom_rate` guards
        // against. Chosen as exact binary fractions so the f32→f64 widening
        // this method does cannot introduce rounding noise into the assertion.
        assert_eq!(corpus.mean_peak_context_pressure(), Some(0.5));
        assert_eq!(corpus.mean_anticipated_guilt(), Some(0.25));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corpus_predating_the_homeostat_has_neither_mean() {
        let dir = tmpdir();
        session_with(
            &dir,
            "20260801T000000-old",
            "opus",
            vec![stats(4, 0, false, StopCause::Completed)],
        );

        let corpus = Corpus::scan(&dir, &Scan::default()).unwrap();
        assert_eq!(corpus.mean_peak_context_pressure(), None, "not Some(0.0)");
        assert_eq!(corpus.mean_anticipated_guilt(), None, "not Some(0.0)");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_scan_collects_every_run_across_every_session() {
        let dir = tmpdir();
        session_with(
            &dir,
            "20260801T000000-a",
            "opus",
            vec![
                stats(4, 1, false, StopCause::Completed),
                // A resumed session: several runs, one row each, numbered.
                stats(6, 0, true, StopCause::MaxTurns),
            ],
        );
        session_with(
            &dir,
            "20260801T000001-b",
            "opus",
            vec![stats(10, 4, false, StopCause::Completed)],
        );

        let corpus = Corpus::scan(&dir, &Scan::default()).unwrap();
        assert_eq!(corpus.len(), 3);
        assert_eq!(corpus.sessions_read, 2);
        assert_eq!(corpus.tool_calls(), 20);
        assert_eq!(corpus.tool_errors(), 5);
        assert_eq!(corpus.compactions(), 3);
        assert_eq!(corpus.ended_on_failed_call(), 1);
        assert_eq!(corpus.cost_usd(), (0.75, 3));

        let runs: Vec<u32> = corpus
            .rows
            .iter()
            .filter(|r| r.session_id.ends_with('a'))
            .map(|r| r.run)
            .collect();
        assert_eq!(
            runs,
            vec![1, 2],
            "runs within a session are numbered in order"
        );

        let causes = corpus.stop_causes();
        assert_eq!(causes[&Some(StopCause::Completed)], 2);
        assert_eq!(causes[&Some(StopCause::MaxTurns)], 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_mid_session_model_switch_attributes_each_run_to_the_model_that_ran_it() {
        // The TUI can change model mid-session and records a `Config` when it
        // does. Reading the header instead would credit the second model's
        // runs to the first — defeating `by_model` in the one case where a
        // corpus genuinely blends two, and pointing a threshold at the wrong
        // model.
        let dir = tmpdir();
        let s = session_with(
            &dir,
            "20260801T000000-switch",
            "first-model",
            vec![stats(4, 0, false, StopCause::Completed)],
        );
        s.append(&Record::Config(crate::session::RunConfig {
            provider: "local".into(),
            model: "second-model".into(),
            ..Default::default()
        }))
        .unwrap();
        s.append(&Record::Outcome(stats(6, 3, false, StopCause::Completed)))
            .unwrap();

        let corpus = Corpus::scan(&dir, &Scan::default()).unwrap();
        assert_eq!(corpus.len(), 2);
        let by_model = corpus.by_model();
        assert_eq!(by_model["first-model"].tool_error_rate(), Some(0.0));
        assert_eq!(by_model["second-model"].tool_error_rate(), Some(0.5));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rate_over_nothing_is_unknown_rather_than_perfect() {
        // The distinction a threshold silent on zero gets wrong: no calls is
        // not a clean record, it is no evidence. An empty corpus and a corpus
        // of tool-less runs both answer `None`, and a reader that wants to
        // treat that as healthy has to say so itself.
        let dir = tmpdir();
        let empty = Corpus::scan(&dir, &Scan::default()).unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.tool_error_rate(), None);
        assert_eq!(empty.rate_of(|r| r.stats.ended_on_failed_call), None);

        session_with(
            &dir,
            "20260801T000000-c",
            "opus",
            vec![stats(0, 0, false, StopCause::Completed)],
        );
        let tool_less = Corpus::scan(&dir, &Scan::default()).unwrap();
        assert_eq!(tool_less.len(), 1);
        assert_eq!(
            tool_less.tool_error_rate(),
            None,
            "no calls is not a clean record"
        );
        // A run-level rate is still defined: there was a run, it just made no
        // calls. The two denominators are different questions.
        assert_eq!(
            tool_less.rate_of(|r| r.stats.ended_on_failed_call),
            Some(0.0)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_scan_is_bounded_and_says_how_much_it_read() {
        let dir = tmpdir();
        for i in 0..5 {
            session_with(
                &dir,
                &format!("20260801T00000{i}-s"),
                "opus",
                vec![stats(2, 0, false, StopCause::Completed)],
            );
        }
        let bounded = Corpus::scan(
            &dir,
            &Scan {
                max_sessions: Some(2),
                since: None,
            },
        )
        .unwrap();
        assert_eq!(bounded.sessions_read, 2);
        assert_eq!(bounded.len(), 2);

        // The cutoff is on the session's own stamp, and every fixture here
        // predates this one, so nothing survives it.
        let cut = Corpus::scan(
            &dir,
            &Scan {
                max_sessions: None,
                since: Some(
                    DateTime::parse_from_rfc3339("2026-08-02T00:00:00Z")
                        .unwrap()
                        .with_timezone(&Utc),
                ),
            },
        )
        .unwrap();
        assert!(cut.is_empty());
        assert_eq!(cut.sessions_read, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_session_with_no_outcomes_is_read_and_contributes_nothing() {
        // Transcripts written before the record existed, and runs that died
        // before producing one. They must not read as a run with zero of
        // everything, which would drag every rate toward a fiction.
        let dir = tmpdir();
        let s = session_with(&dir, "20260801T000000-d", "opus", vec![]);
        s.append_messages(&[crate::message::Message::user("go")])
            .unwrap();
        session_with(
            &dir,
            "20260801T000001-e",
            "opus",
            vec![stats(4, 2, false, StopCause::Completed)],
        );

        let corpus = Corpus::scan(&dir, &Scan::default()).unwrap();
        assert_eq!(corpus.sessions_read, 2, "both sessions were read");
        assert_eq!(corpus.len(), 1, "only one contributed a run");
        assert_eq!(corpus.tool_error_rate(), Some(0.5));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rates_split_by_model_because_a_mixed_corpus_has_no_single_one() {
        let dir = tmpdir();
        session_with(
            &dir,
            "20260801T000000-f",
            "opus",
            vec![stats(10, 1, false, StopCause::Completed)],
        );
        session_with(
            &dir,
            "20260801T000001-g",
            "tiny-local",
            vec![stats(10, 9, false, StopCause::Completed)],
        );

        let corpus = Corpus::scan(&dir, &Scan::default()).unwrap();
        // The blended number is true and useless: neither model behaves this
        // way, and a threshold on it fires for the wrong one.
        assert_eq!(corpus.tool_error_rate(), Some(0.5));
        let by_model = corpus.by_model();
        assert_eq!(by_model["opus"].tool_error_rate(), Some(0.1));
        assert_eq!(by_model["tiny-local"].tool_error_rate(), Some(0.9));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
