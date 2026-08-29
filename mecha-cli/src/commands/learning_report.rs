//! `mecha learning report` — is the learning loop actually improving anything?
//!
//! Everything else in this subsystem measures **one thing**: `rule_tallies`
//! scores a rule, `candidate::Judgement` scores a candidate. Neither answers
//! "is mecha getting better", so nothing would notice the loop going wrong —
//! which matters most exactly when learning applies its own output.
//!
//! Four series, folded from records that already exist. **No model call and
//! no network**, for the same reason `doctor` has none: a health check that
//! needs the thing it is checking is not a health check.
//!
//! **These are observational, and the report says so rather than implying
//! otherwise.** The corpus is one owner's real work, so the mix of tasks
//! moves under the metric — a falling correction rate can mean better rules
//! or an easier week. It is a monitor for noticing regression between
//! controlled runs, and `mecha eval --ab-rules` (rules-off vs rules-on over a
//! fixed case set) remains the thing to cite as evidence that rules help.
//! Reporting it as proof of improvement would be the stronger claim the data
//! cannot carry.

use anyhow::{Context, Result};
use mecha_core::learning::{rule_tallies, LearningStore};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Machine-readable, for the web settings page.
    #[arg(long)]
    pub json: bool,

    /// Bucket width in days. Weekly by default: the corpus is a few hundred
    /// sessions over a month, and daily buckets are mostly noise around a
    /// denominator of one or two.
    #[arg(long, default_value_t = 7)]
    pub bucket_days: i64,
}

/// One time bucket, with everything measured over it.
#[derive(Debug, Serialize, Default, Clone)]
pub struct Bucket {
    /// ISO date of the bucket's first day.
    pub period: String,
    /// Sessions that started in this bucket — the denominator.
    pub sessions: u32,
    /// Reflections mined from them — the numerator.
    pub reflections: u32,
    /// Reflections per session. **`None` when no session ran**, never 0.0:
    /// a rate over an empty denominator is not zero, and a chart that draws
    /// it as zero shows a perfect week where there was no week at all.
    pub rate: Option<f64>,
    /// Reflections by `error_type`, for the composition chart.
    pub error_types: BTreeMap<String, u32>,
}

/// A consolidation or retirement pass, for the rule-count step chart.
#[derive(Debug, Serialize)]
pub struct RuleStep {
    pub at: String,
    pub domain: String,
    pub rules_before: u32,
    pub rules_after: u32,
    pub reflections: u32,
}

/// The standing state of the rule set — the part that is a fact rather than
/// a trend.
#[derive(Debug, Serialize, Default)]
pub struct RuleHealth {
    pub active: u32,
    pub retired: u32,
    /// Active rules no probe has ever covered. Not a defect on its own, but
    /// a set that is *entirely* unvalidated means the ledger is not running,
    /// and every claim downstream of it is empty.
    pub never_validated: u32,
    pub attributed_regressions: u32,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub buckets: Vec<Bucket>,
    pub steps: Vec<RuleStep>,
    pub health: BTreeMap<String, RuleHealth>,
    /// Stated in the payload, not just in this module's docs — the surface
    /// rendering a trend line is where the caveat has to be readable.
    pub caveat: String,
}

/// The bucket start an ISO-ish timestamp falls in, as `YYYY-MM-DD`.
///
/// Buckets are anchored to the epoch rather than to the first record, so the
/// same day lands in the same bucket across runs. An anchor that moves with
/// the data would re-bucket the whole history whenever the earliest session
/// aged out, and a chart whose x-axis silently re-cuts is a chart nobody can
/// compare against last week's.
fn bucket_of(ts: &str, days: i64) -> Option<String> {
    let d = ts.get(..10)?;
    let date = chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()?;
    let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1)?;
    let n = (date - epoch).num_days();
    let start = epoch + chrono::Duration::days(n - n.rem_euclid(days));
    Some(start.format("%Y-%m-%d").to_string())
}

/// A session id's date, from its own name: `20260804T165213-09b757a1`.
fn session_date(name: &str) -> Option<String> {
    let d = name.get(..8)?;
    if !d.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..8]))
}

pub fn build(store: &LearningStore, sessions_dir: &std::path::Path, days: i64) -> Result<Report> {
    let mut buckets: BTreeMap<String, Bucket> = BTreeMap::new();

    // Denominator first. A bucket with sessions and no reflections is a real
    // and good measurement — nothing needed correcting — so the session walk
    // must create buckets the reflection walk would never reach.
    if let Ok(rd) = std::fs::read_dir(sessions_dir) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(date) = session_date(&name) else {
                continue;
            };
            let Some(b) = bucket_of(&date, days) else {
                continue;
            };
            buckets.entry(b.clone()).or_default().sessions += 1;
        }
    }

    for r in store.reflexions()? {
        let Some(b) = bucket_of(&r.created_at, days) else {
            continue;
        };
        let e = buckets.entry(b).or_default();
        e.reflections += 1;
        *e.error_types
            .entry(r.error_type.clone().unwrap_or_else(|| "unknown".into()))
            .or_default() += 1;
    }

    let mut out: Vec<Bucket> = buckets
        .into_iter()
        .map(|(period, mut b)| {
            b.period = period;
            b.rate = (b.sessions > 0).then(|| b.reflections as f64 / b.sessions as f64);
            b
        })
        .collect();
    out.sort_by(|a, b| a.period.cmp(&b.period));

    // Rule-count steps, from the store's own pass log.
    let runs_path = store.root().join("runs.jsonl");
    let mut steps = Vec::new();
    if let Ok(text) = std::fs::read_to_string(&runs_path) {
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            steps.push(RuleStep {
                at: v["created_at"].as_str().unwrap_or_default().to_string(),
                domain: v["domain"].as_str().unwrap_or_default().to_string(),
                rules_before: v["rules_before"].as_u64().unwrap_or(0) as u32,
                rules_after: v["rules_after"].as_u64().unwrap_or(0) as u32,
                reflections: v["reflexions_processed"].as_u64().unwrap_or(0) as u32,
            });
        }
    }

    let tallies = rule_tallies(&store.validations()?);
    let mut health: BTreeMap<String, RuleHealth> = BTreeMap::new();
    for domain in store.domains() {
        let h = health.entry(domain.clone()).or_default();
        for r in store.learned_rules(&domain)? {
            if r.retired_at.is_some() {
                h.retired += 1;
                continue;
            }
            h.active += 1;
            match r.id.as_deref().and_then(|id| tallies.get(id)) {
                Some(t) if t.observations > 0 => {
                    h.attributed_regressions += t.attributed_regressions;
                }
                // Unmeasured, which is not the same as measured-clean.
                _ => h.never_validated += 1,
            }
        }
    }

    Ok(Report {
        buckets: out,
        steps,
        health,
        caveat: "Observational, over one owner's real work: the task mix moves under \
                 the metric, so a falling correction rate may mean better rules or an \
                 easier week. Use `mecha eval --ab-rules` for a controlled comparison."
            .into(),
    })
}

pub async fn execute(args: Args) -> Result<()> {
    let store = LearningStore::open(LearningStore::default_root()?)?;
    let sessions = mecha_core::session::Session::default_dir()
        .context("cannot locate the session directory")?;
    let report = build(&store, &sessions, args.bucket_days.max(1))?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!(
        "correction rate (reflections per session, {}-day buckets)",
        args.bucket_days
    );
    if report.buckets.is_empty() {
        println!("  no sessions recorded yet");
    }
    for b in &report.buckets {
        // A dash, never a zero: no session means no measurement.
        let rate = b
            .rate
            .map(|r| format!("{r:.2}"))
            .unwrap_or_else(|| "   —".into());
        println!(
            "  {}  {:>5}  {:>3} reflection(s) / {:>3} session(s)",
            b.period, rate, b.reflections, b.sessions
        );
    }

    println!("\nrule set");
    if report.health.is_empty() {
        println!("  no learned rules yet — `mecha learn` creates them");
    }
    for (domain, h) in &report.health {
        println!(
            "  {domain}: {} active, {} retired, {} never validated, {} attributed regression(s)",
            h.active, h.retired, h.never_validated, h.attributed_regressions
        );
    }

    println!("\nconsolidation passes");
    if report.steps.is_empty() {
        println!("  none recorded");
    }
    for s in report.steps.iter().rev().take(10) {
        println!(
            "  {}  {:<9} {} → {} rule(s) from {} reflection(s)",
            &s.at[..s.at.len().min(10)],
            s.domain,
            s.rules_before,
            s.rules_after,
            s.reflections
        );
    }

    println!("\n{}", report.caveat);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_are_anchored_to_the_epoch_not_to_the_data() {
        // Same day, same bucket, regardless of what else is in the corpus —
        // a chart whose x-axis re-cuts when old data ages out cannot be
        // compared against last week's.
        // A date and that date with a time land together.
        assert_eq!(
            bucket_of("2026-08-29", 7),
            bucket_of("2026-08-29T17:00:00Z", 7)
        );

        // Within one epoch-aligned window, the same bucket.
        assert_eq!(
            bucket_of("2026-08-27", 7).unwrap(),
            bucket_of("2026-08-29", 7).unwrap()
        );

        // Across the boundary, a different one — even though these are only
        // three days apart. Fixed boundaries are the whole point: buckets
        // that floated with the data would re-cut the x-axis whenever the
        // oldest session aged out.
        assert_ne!(
            bucket_of("2026-08-26", 7).unwrap(),
            bucket_of("2026-08-27", 7).unwrap()
        );
        assert_eq!(bucket_of("2026-08-26", 7).unwrap(), "2026-08-20");
    }

    #[test]
    fn a_session_id_yields_its_date_and_a_stray_file_is_ignored() {
        assert_eq!(
            session_date("20260804T165213-09b757a1.jsonl").as_deref(),
            Some("2026-08-04")
        );
        assert!(session_date("README.md").is_none());
        assert!(session_date("short").is_none());
    }
}
