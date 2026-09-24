//! What an experiment's trials say, read off its rows: the readout `judge`
//! is not.
//!
//! `judge` answers one question — does each treatment beat the control on
//! its predicted metric, through the gate — and says nothing else. This
//! module is everything a person reads *before* trusting that answer: which
//! tasks moved, what each arm cost in turns, tokens and time, whether a
//! task passes every time it runs or only sometimes, and, for a lifetime,
//! whether the pass rate moves along the sequence.
//!
//! Pure: rows in, a [`Report`] out, no store and no clock. Every rate is an
//! `Option` over its own denominator, and `None` when there is nothing to
//! divide by — "nothing ran" and "nothing passed" are opposite findings,
//! and a zero would print one as the other.

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

use crate::experiment::{Manifest, Trial, TrialKind, TrialStatus};

/// The whole readout.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub name: String,
    pub kind: TrialKind,
    pub control: Option<String>,
    pub arms: Vec<ArmReport>,
    /// One row per task, in first-seen order, with a cell per arm.
    pub tasks: Vec<TaskRow>,
    /// Lifetimes only: pass rate by sequence position, per arm.
    pub curves: Vec<Curve>,
}

/// One arm, over its rows.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ArmReport {
    pub arm: String,
    pub rows: usize,
    pub done: usize,
    pub failed: usize,
    /// Planned and not yet started.
    pub pending: usize,
    /// Started and not finished: in flight, or a crashed runner's row.
    pub running: usize,
    /// A status this build cannot read: neither rerun nor judged, and a
    /// lifetime stops at one, so it is a finding, never "pending".
    pub unknown: usize,
    /// An arm the manifest no longer names, whose rows are still stored.
    pub off_manifest: bool,
    /// Done rows with a grade, and how many of them passed.
    pub graded: usize,
    pub passed: usize,
    pub pass_rate: Option<f64>,
    /// Done rows that carry run stats, the denominator of `mean_turns` and
    /// of every total below it (a row whose session could not be read has
    /// none).
    pub with_stats: usize,
    /// Over `with_stats` rows.
    pub mean_turns: Option<f64>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub tool_calls: u64,
    pub tool_errors: u64,
    pub compactions: u64,
    /// Done rows with both timestamps, the denominator of `mean_wall_secs`.
    pub with_wall: usize,
    /// Seconds from start to finish, over `with_wall` rows.
    pub mean_wall_secs: Option<f64>,
    /// Tasks with at least one graded run, and of those: passed on every run
    /// (pass^k) and on at least one (pass@k). Across seeds and repetitions.
    pub tasks_graded: usize,
    pub tasks_always: usize,
    pub tasks_ever: usize,
    /// The `--jobs` limits this arm's rows ran under; a row from before the
    /// field counts as 1.
    pub jobs_seen: Vec<u32>,
}

/// One task across the arms.
#[derive(Debug, Clone, Serialize)]
pub struct TaskRow {
    pub task: String,
    pub cells: BTreeMap<String, Cell>,
}

/// Graded runs of one task in one arm.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
pub struct Cell {
    pub runs: usize,
    pub passed: usize,
}

/// A lifetime arm's pass rate along its sequence.
#[derive(Debug, Clone, Serialize)]
pub struct Curve {
    pub arm: String,
    /// (position, graded runs there, passed) in position order.
    pub points: Vec<(u32, usize, usize)>,
    /// Failure rate over the first and the second half of the positions —
    /// the crude slope: a loop that learns fails less later.
    pub early_failure: Option<f64>,
    pub late_failure: Option<f64>,
}

fn rate(num: usize, den: usize) -> Option<f64> {
    (den > 0).then(|| num as f64 / den as f64)
}

fn wall_secs(t: &Trial) -> Option<f64> {
    let start = chrono::DateTime::parse_from_rfc3339(t.started_at.as_deref()?).ok()?;
    let end = chrono::DateTime::parse_from_rfc3339(t.finished_at.as_deref()?).ok()?;
    let ms = (end - start).num_milliseconds();
    (ms >= 0).then(|| ms as f64 / 1000.0)
}

/// Build the readout from a manifest and its rows (the store's, as `judge`
/// reads them).
pub fn build(manifest: &Manifest, trials: &[Trial]) -> Report {
    // One pass for the task × arm cells, which both the per-task table and
    // each arm's pass^k / pass@k read, so the two cannot disagree.
    let mut cells: BTreeMap<(&str, &str), Cell> = BTreeMap::new();
    for t in trials.iter().filter(|t| t.status == TrialStatus::Done) {
        if let Some(p) = t.passed {
            let c = cells.entry((t.arm.as_str(), t.task.as_str())).or_default();
            c.runs += 1;
            c.passed += usize::from(p);
        }
    }
    // The manifest's arms, then any arm a stored row names that the
    // manifest does not: rows are never dropped from the count.
    let mut names: Vec<(String, bool)> = manifest.arms.keys().map(|a| (a.clone(), false)).collect();
    let stray: BTreeSet<&str> = trials
        .iter()
        .map(|t| t.arm.as_str())
        .filter(|a| !manifest.arms.contains_key(*a))
        .collect();
    names.extend(stray.into_iter().map(|a| (a.to_string(), true)));

    let mut arms = Vec::new();
    for (name, off_manifest) in &names {
        let rows: Vec<&Trial> = trials.iter().filter(|t| &t.arm == name).collect();
        let mut a = ArmReport {
            arm: name.clone(),
            rows: rows.len(),
            off_manifest: *off_manifest,
            ..ArmReport::default()
        };
        let (mut turns, mut with_stats) = (0u64, 0usize);
        let (mut wall, mut with_wall) = (0f64, 0usize);
        let mut jobs = BTreeSet::new();
        for t in &rows {
            match t.status {
                TrialStatus::Done => a.done += 1,
                TrialStatus::Failed => a.failed += 1,
                TrialStatus::Pending => a.pending += 1,
                TrialStatus::Running => a.running += 1,
                TrialStatus::Unknown => a.unknown += 1,
            }
            if t.status != TrialStatus::Done {
                continue;
            }
            jobs.insert(t.jobs.unwrap_or(1));
            if let Some(p) = t.passed {
                a.graded += 1;
                a.passed += usize::from(p);
            }
            if let Some(s) = &t.stats {
                with_stats += 1;
                turns += u64::from(s.turns);
                a.input_tokens += s.usage.input_tokens;
                a.output_tokens += s.usage.output_tokens;
                a.tool_calls += u64::from(s.tool_calls);
                a.tool_errors += u64::from(s.tool_errors);
                a.compactions += u64::from(s.compactions);
            }
            if let Some(w) = wall_secs(t) {
                wall += w;
                with_wall += 1;
            }
        }
        a.pass_rate = rate(a.passed, a.graded);
        a.with_stats = with_stats;
        a.with_wall = with_wall;
        a.mean_turns = (with_stats > 0).then(|| turns as f64 / with_stats as f64);
        a.mean_wall_secs = (with_wall > 0).then(|| wall / with_wall as f64);
        let mine: Vec<&Cell> = cells
            .iter()
            .filter(|((arm, _), _)| arm == name)
            .map(|(_, c)| c)
            .collect();
        a.tasks_graded = mine.len();
        a.tasks_always = mine.iter().filter(|c| c.passed == c.runs).count();
        a.tasks_ever = mine.iter().filter(|c| c.passed > 0).count();
        a.jobs_seen = jobs.into_iter().collect();
        arms.push(a);
    }

    let mut order: Vec<&str> = Vec::new();
    for t in trials {
        if !order.contains(&t.task.as_str()) {
            order.push(&t.task);
        }
    }
    let tasks = order
        .into_iter()
        .map(|task| TaskRow {
            task: task.to_string(),
            cells: names
                .iter()
                .map(|(arm, _)| {
                    (
                        arm.clone(),
                        cells
                            .get(&(arm.as_str(), task))
                            .copied()
                            .unwrap_or_default(),
                    )
                })
                .collect(),
        })
        .collect();

    let mut curves = Vec::new();
    if manifest.kind == TrialKind::Lifetime {
        for name in manifest.arms.keys() {
            let mut at: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
            for t in trials
                .iter()
                .filter(|t| &t.arm == name && t.status == TrialStatus::Done && t.passed.is_some())
            {
                if let Some(p) = t.position {
                    let e = at.entry(p).or_default();
                    e.0 += 1;
                    e.1 += usize::from(t.passed == Some(true));
                }
            }
            let points: Vec<(u32, usize, usize)> =
                at.into_iter().map(|(p, (n, k))| (p, n, k)).collect();
            let half = points.len() / 2;
            let fail = |slice: &[(u32, usize, usize)]| {
                let (n, k) = slice
                    .iter()
                    .fold((0, 0), |(n, k), (_, pn, pk)| (n + pn, k + pk));
                rate(n - k, n)
            };
            let (early, late) = if points.len() >= 2 {
                (fail(&points[..half]), fail(&points[half..]))
            } else {
                (None, None)
            };
            curves.push(Curve {
                arm: name.clone(),
                points,
                early_failure: early,
                late_failure: late,
            });
        }
    }

    Report {
        name: manifest.name.clone(),
        kind: manifest.kind,
        control: manifest.control.clone(),
        arms,
        tasks,
        curves,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::RunStats;

    fn row(arm: &str, task: &str, seed: u64, passed: Option<bool>, turns: u32) -> Trial {
        let mut t: Trial = serde_json::from_value(serde_json::json!({
            "id": format!("{arm}__{task}__s{seed}"),
            "arm": arm,
            "task": task,
            "seed": seed,
            "repetition": 1,
            "condition_hash": "h",
            "status": "done",
            "started_at": "2026-09-24T00:00:00Z",
            "finished_at": "2026-09-24T00:00:10Z",
        }))
        .unwrap();
        t.passed = passed;
        t.stats = Some(RunStats {
            turns,
            tool_calls: 2,
            ..RunStats::default()
        });
        t
    }

    fn manifest(kind: &str) -> Manifest {
        Manifest::parse(&format!(
            r#"
name = "r"
kind = "{kind}"
control = "a"
split_seed = 1
[tasks]
cases = "c.jsonl"
fixture = "w"
ids = ["t1", "t2"]
[arms.a]
[arms.b]
[arms.b.prediction]
metric = "failure"
rationale = "r"
"#
        ))
        .unwrap()
    }

    /// Pass rates, pass^k against pass@k, cost sums and the per-task table
    /// come out of the rows, and a pending row is counted, not graded.
    #[test]
    fn the_readout_is_the_rows_counted() {
        let m = manifest("single");
        let mut rows = vec![
            row("a", "t1", 1, Some(true), 2),
            row("a", "t1", 2, Some(true), 4),
            row("a", "t2", 1, Some(true), 3),
            row("a", "t2", 2, Some(false), 3),
            row("b", "t1", 1, Some(false), 5),
            row("b", "t1", 2, Some(false), 5),
        ];
        let mut pending = row("b", "t2", 1, None, 0);
        pending.status = TrialStatus::Pending;
        pending.stats = None;
        rows.push(pending);
        let mut running = row("b", "t2", 2, None, 0);
        running.status = TrialStatus::Running;
        rows.push(running);
        let mut unknown = row("a", "t3", 1, None, 0);
        unknown.status = TrialStatus::Unknown;
        rows.push(unknown);
        // A row from an arm the manifest no longer names.
        rows.push(row("old", "t1", 1, Some(true), 1));
        let r = build(&m, &rows);
        let a = &r.arms[0];
        assert_eq!((a.done, a.graded, a.passed), (4, 4, 3));
        assert_eq!(a.pass_rate, Some(0.75));
        assert_eq!(a.mean_turns, Some(3.0));
        assert_eq!((a.with_stats, a.with_wall), (4, 4));
        assert_eq!(a.tool_calls, 8);
        assert_eq!(a.mean_wall_secs, Some(10.0));
        assert_eq!((a.tasks_graded, a.tasks_always, a.tasks_ever), (2, 1, 2));
        assert_eq!(a.jobs_seen, vec![1]);
        assert_eq!(
            (a.unknown, a.pending, a.running),
            (1, 0, 0),
            "unknown is never pending"
        );
        let b = &r.arms[1];
        assert_eq!((b.done, b.pending, b.running, b.passed), (2, 1, 1, 0));
        let old = &r.arms[2];
        assert!(old.off_manifest && !r.arms[0].off_manifest);
        assert_eq!(
            (old.done, old.passed),
            (1, 1),
            "a stray arm's rows are counted"
        );
        assert_eq!(r.tasks[0].cells["old"], Cell { runs: 1, passed: 1 });
        assert_eq!(b.pass_rate, Some(0.0), "nothing passed is a zero");
        assert_eq!((b.tasks_graded, b.tasks_always, b.tasks_ever), (1, 0, 0));
        assert_eq!(r.tasks[0].task, "t1");
        assert_eq!(r.tasks[0].cells["b"], Cell { runs: 2, passed: 0 });
        assert_eq!(
            r.tasks[1].cells["b"],
            Cell::default(),
            "pending is not graded"
        );
        assert!(r.curves.is_empty());
    }

    /// An arm with nothing done has no rates: nothing ran, which is not the
    /// same finding as nothing passed.
    #[test]
    fn nothing_ran_is_none_and_never_zero() {
        let m = manifest("single");
        let r = build(&m, &[row("a", "t1", 1, Some(true), 2)]);
        let b = &r.arms[1];
        assert_eq!(b.pass_rate, None);
        assert_eq!(b.mean_turns, None);
        assert_eq!(
            b.with_stats, 0,
            "totals over no stats are nothing, not zero"
        );
        assert_eq!(b.mean_wall_secs, None);
        assert!(b.jobs_seen.is_empty());
    }

    /// A lifetime's curve is pass rate by position, with the first half's
    /// failure rate beside the second's.
    #[test]
    fn a_lifetime_has_a_curve_by_position() {
        let m = manifest("lifetime");
        let mut rows = Vec::new();
        for (pos, passed) in [(1, false), (2, false), (3, true), (4, true)] {
            let mut t = row("b", &format!("t{pos}"), 1, Some(passed), 2);
            t.position = Some(pos);
            rows.push(t);
        }
        let r = build(&m, &rows);
        let b = r.curves.iter().find(|c| c.arm == "b").unwrap();
        assert_eq!(b.points, vec![(1, 1, 0), (2, 1, 0), (3, 1, 1), (4, 1, 1)]);
        assert_eq!(b.early_failure, Some(1.0));
        assert_eq!(b.late_failure, Some(0.0));
        let a = r.curves.iter().find(|c| c.arm == "a").unwrap();
        assert_eq!((a.early_failure, a.late_failure), (None, None));
    }
}
