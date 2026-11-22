//! The GitHub collector's log, read back (`GITHUB-DASHBOARD-DESIGN.md` §3).
//!
//! The board's tables say what the last good answer said, and nothing on the
//! board can tell that from a collector that has failed since: an empty
//! `items` table after a revoked login reads as "nothing new". So the
//! collector logs every run, and this is the one reader. Refusals and
//! failures are separate findings, each with how long it has lasted.
use super::{Finding, Remedy, Severity};
use crate::github::{self, Kind, LastRun};
use chrono::{DateTime, Duration, Utc};
use std::path::Path;

const COMPONENT: &str = "github";

/// A collector whose last run is older than this has stopped running. The
/// shipped timer runs once a day (ruling G10), so this is a day and a half:
/// a run late by the timer's spread, or a machine off over the hour, still
/// reads as running; a missed day does not.
pub const GITHUB_STALE_AFTER: Duration = Duration::hours(36);

/// How long a repository that vanished stays a finding. A vanished one is
/// an event, logged by the run that missed it, not a state every later run
/// repeats.
pub const GITHUB_VANISHED_WITHIN: Duration = Duration::days(7);

fn finding(severity: Severity, summary: impl Into<String>, detail: impl Into<String>) -> Finding {
    Finding {
        component: COMPONENT.into(),
        severity,
        summary: summary.into(),
        detail: detail.into(),
        remedy: None,
    }
}

fn collect_now() -> Remedy {
    Remedy {
        description: "run the GitHub collector once now and show what it found".into(),
        argv: vec!["mecha".into(), "github".into(), "collect".into()],
        needs_terminal: false,
    }
}

fn when(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%d %H:%MZ").to_string()
}

/// "every run since 12:10Z (3 runs)", or "the last run (12:10Z)".
fn lasted(last: &LastRun, kind: Kind) -> String {
    match last.since.get(kind.as_str()) {
        Some((first, n)) if *n > 1 => format!("every run since {} ({n} runs)", when(*first)),
        _ => format!("the last run ({})", when(last.run)),
    }
}

/// The collector's store, read without creating or changing anything. A
/// store that is not there is a home that never ran the collector; whether
/// that is a finding depends on the switch, which `examine` cannot see
/// ([`check_github_never_ran`]).
pub fn check_github(db: &Path, now: DateTime<Utc>) -> Vec<Finding> {
    if !db.exists() {
        return Vec::new();
    }
    let report = match github::read_log(db, GITHUB_VANISHED_WITHIN, now) {
        Ok(r) => r,
        Err(e) => {
            return vec![Finding::unreadable(
                COMPONENT,
                "github/github.db",
                format!("{e:#}"),
            )]
        }
    };
    let mut out = Vec::new();
    let Some(last) = report.last.as_ref() else {
        out.push(Finding {
            remedy: Some(collect_now()),
            ..finding(
                Severity::Attention,
                "the GitHub collector has not finished a run",
                "github.db holds no run; is mecha-github-collect.timer running?",
            )
        });
        return out;
    };
    let of = |k: Kind| last.problems.iter().filter(move |p| p.kind == Some(k));
    let tables_as_of = "the board's tables still say what an earlier run found, and nothing on \
                        the board shows they are old";

    if of(Kind::GhMissing).next().is_some() {
        out.push(finding(
            Severity::Broken,
            "the GitHub collector cannot start `gh`",
            format!(
                "{} could not start `gh`; {tables_as_of}. Install GitHub CLI where the unit's \
                 PATH reaches it (scripts/mecha-github-collect.service sets PATH), or point \
                 MECHA_GH_BIN at it.",
                lasted(last, Kind::GhMissing)
            ),
        ));
    }
    if let Some(p) = of(Kind::Auth).next() {
        out.push(Finding {
            remedy: Some(Remedy {
                description: "log `gh` in to GitHub; the collector uses its login and never \
                              holds the token"
                    .into(),
                argv: vec!["gh".into(), "auth".into(), "login".into()],
                needs_terminal: true,
            }),
            ..finding(
                Severity::Broken,
                "the GitHub collector's `gh` is not logged in, or GitHub refused its login",
                format!(
                    "{}: {}; {tables_as_of}.",
                    lasted(last, Kind::Auth),
                    p.detail.as_deref().unwrap_or("refused")
                ),
            )
        });
    }
    if let Some(p) = of(Kind::BadSelection).next() {
        out.push(finding(
            Severity::Broken,
            "the GitHub board's repos.toml cannot be used, so nothing is watched",
            p.detail.clone().unwrap_or_default(),
        ));
    }
    if of(Kind::NoSelection).next().is_some() {
        out.push(finding(
            Severity::Attention,
            "no repositories are selected for the GitHub board",
            "there is no ~/.mecha/github/repos.toml; the inventory is listed, and nothing is \
             watched until the file names an owner or a repository",
        ));
    } else if of(Kind::BadSelection).next().is_none() && last.selected == Some(0) {
        // A file that matches nothing makes an empty board, which reads as
        // nothing failing: "nothing happened" is not "nothing went wrong".
        out.push(finding(
            Severity::Attention,
            "the GitHub board's repos.toml selects no repository",
            "the last run watched nothing: no owner or [[repo]] line matched a repository the \
             login can see",
        ));
    }
    let limited: Vec<&str> = of(Kind::RateLimited)
        .filter_map(|p| p.step.as_deref())
        .collect();
    if !limited.is_empty() {
        out.push(finding(
            Severity::Attention,
            "the GitHub collector was rate-limited",
            format!(
                "{} was refused for quota at {}; what it did not reach keeps its earlier answer. \
                 A selection too large for the timer's cadence does this every run.",
                lasted(last, Kind::RateLimited),
                limited.join(", ")
            ),
        ));
    }
    if of(Kind::OutOfTime).next().is_some() {
        out.push(finding(
            Severity::Attention,
            "the GitHub collector ran out of time",
            format!(
                "{} spent its {} s budget before it finished; what it did not reach keeps its \
                 earlier answer",
                lasted(last, Kind::OutOfTime),
                github::RUN_BUDGET.as_secs()
            ),
        ));
    }
    let failed: Vec<String> = of(Kind::Failed)
        .map(|p| {
            format!(
                "{}{}: {}",
                p.step.as_deref().unwrap_or("?"),
                p.repo
                    .as_deref()
                    .map(|r| format!(" {r}"))
                    .unwrap_or_default(),
                p.detail.as_deref().unwrap_or("failed")
            )
        })
        .collect();
    if !failed.is_empty() {
        out.push(finding(
            Severity::Attention,
            format!("{} GitHub collection step(s) failed", failed.len()),
            format!(
                "in {}: {}",
                lasted(last, Kind::Failed),
                failed
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        ));
    }
    // Usage (step 5). Traffic refused for push access is a refusal and not
    // a failure: the board's dash says it, and this says why there is a dash
    // where the owner may expect a number. A traffic call that failed is in
    // the failed finding above, under its step.
    let no_access: Vec<&str> = of(Kind::TrafficNoAccess)
        .filter_map(|p| p.repo.as_deref())
        .collect();
    if !no_access.is_empty() {
        out.push(finding(
            Severity::Attention,
            format!(
                "GitHub refused traffic for {} watched repositor{}: the login cannot push to {}",
                no_access.len(),
                if no_access.len() == 1 { "y" } else { "ies" },
                if no_access.len() == 1 { "it" } else { "them" }
            ),
            format!(
                "in {}: {}. GitHub shows clones and views only to accounts with push access, so \
                 the board shows a dash for {} — never a zero. If the login should be able to \
                 push, `gh auth status` says which scopes its token holds.",
                lasted(last, Kind::TrafficNoAccess),
                no_access
                    .iter()
                    .take(5)
                    .copied()
                    .collect::<Vec<_>>()
                    .join(", "),
                if no_access.len() == 1 { "it" } else { "them" }
            ),
        ));
    }
    if let Some(p) = of(Kind::BadPackages).next() {
        out.push(finding(
            Severity::Broken,
            "the GitHub board's packages.toml cannot be used, so the package mapping is kept as it was",
            p.detail.clone().unwrap_or_default(),
        ));
    }
    let registry_refused: Vec<&str> = of(Kind::RegistryRefused)
        .filter_map(|p| p.detail.as_deref())
        .collect();
    if !registry_refused.is_empty() {
        out.push(finding(
            Severity::Attention,
            "a package registry refused the GitHub collector",
            format!(
                "in {}: {}. The download counts it did not give keep their earlier days.",
                lasted(last, Kind::RegistryRefused),
                registry_refused.join("; ")
            ),
        ));
    }
    let registry_failed: Vec<&str> = of(Kind::RegistryFailed)
        .filter_map(|p| p.detail.as_deref())
        .collect();
    if !registry_failed.is_empty() {
        out.push(finding(
            Severity::Attention,
            format!(
                "{} package registry count{} failed",
                registry_failed.len(),
                if registry_failed.len() == 1 { "" } else { "s" }
            ),
            format!(
                "in {}: {}; each is asked again next run",
                lasted(last, Kind::RegistryFailed),
                registry_failed
                    .iter()
                    .take(3)
                    .copied()
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        ));
    }
    // The summary pass (§5). The collection is written before it runs, so
    // these never say the board is stale — they say its New view shows
    // titles alone, which reads the same as summaries nobody needed.
    if let Some(p) = of(Kind::SummarySkipped).next() {
        out.push(finding(
            Severity::Attention,
            "the GitHub board's summaries were not made",
            format!(
                "{}: {}",
                lasted(last, Kind::SummarySkipped),
                p.detail.as_deref().unwrap_or("skipped")
            ),
        ));
    }
    let summary_failed: Vec<String> = of(Kind::SummaryFailed)
        .map(|p| {
            format!(
                "{} {}",
                p.repo.as_deref().unwrap_or("?"),
                p.detail.as_deref().unwrap_or("failed")
            )
        })
        .collect();
    if !summary_failed.is_empty() {
        out.push(finding(
            Severity::Attention,
            format!(
                "{} GitHub summar{} failed",
                summary_failed.len(),
                if summary_failed.len() == 1 {
                    "y"
                } else {
                    "ies"
                }
            ),
            format!(
                "in {}: {}; each is asked again next run",
                lasted(last, Kind::SummaryFailed),
                summary_failed
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        ));
    }
    // Every answer refused and none made: a model that cannot do this job
    // (the wrong one loaded, a template that ignores the format) looks, on
    // the board, exactly like strangers' bodies it would not read.
    if let Some(t) = last
        .summaries
        .filter(|t| t.refused > 0 && t.summarised == 0)
    {
        out.push(finding(
            Severity::Attention,
            "every GitHub summary the last run asked for was refused",
            format!(
                "{} refused, none made ({}): the model's answers were not a summary and a kind, \
                 or it declined. Those items show their titles alone, and are asked again only \
                 when their bodies change or another model answers; is the loaded model one \
                 that can answer in JSON?",
                t.refused,
                when(last.run)
            ),
        ));
    }
    let unknown: Vec<&str> = last
        .problems
        .iter()
        .filter(|p| p.kind.is_none())
        .map(|p| p.raw_kind.as_str())
        .collect();
    if !unknown.is_empty() {
        out.push(finding(
            Severity::Attention,
            "the GitHub collector logged something this build cannot read",
            format!(
                "kind(s) {}: written by another build; read as a problem, never as clean",
                unknown.join(", ")
            ),
        ));
    }
    if !report.vanished.is_empty() {
        out.push(finding(
            Severity::Attention,
            format!(
                "{} watched GitHub repositor{} vanished",
                report.vanished.len(),
                if report.vanished.len() == 1 {
                    "y"
                } else {
                    "ies"
                }
            ),
            format!(
                "{} — deleted, renamed, transferred, or no longer visible to the login. A \
                 [[repo]] line naming one is asked for again every run; remove it from repos.toml.",
                report
                    .vanished
                    .iter()
                    .map(|(r, at)| format!("{r} (last {})", when(*at)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }
    if now - last.run > GITHUB_STALE_AFTER {
        out.push(Finding {
            remedy: Some(collect_now()),
            ..finding(
                Severity::Attention,
                "the GitHub collector has not run for a day and a half",
                format!(
                    "last run {}; is mecha-github-collect.timer running? `journalctl --user -u \
                     mecha-github-collect` says why a run stopped",
                    when(last.run)
                ),
            )
        });
    }
    out
}

/// The switch is on and the collector has never written its store: the
/// board would be empty, which reads as nothing failing. Called with the
/// global configuration's answer, which `examine` does not read.
pub fn check_github_never_ran(home: &Path, switched_on: bool) -> Vec<Finding> {
    if !switched_on || github::db_path(home).exists() {
        return Vec::new();
    }
    vec![Finding {
        remedy: Some(collect_now()),
        ..finding(
            Severity::Attention,
            "the GitHub board is switched on and its collector has never run",
            "there is no ~/.mecha/github/github.db; is mecha-github-collect.timer installed? \
             (scripts/mecha-github-collect.{service,timer})",
        )
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{record, Collected, Problem, Step};

    fn home(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mecha-doctor-github-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn now() -> DateTime<Utc> {
        "2026-11-18T12:00:00Z".parse().unwrap()
    }

    fn run(at: DateTime<Utc>, kinds: &[(Kind, Option<&str>)]) -> Collected {
        let mut c = Collected::new(at);
        for (kind, repo) in kinds {
            c.problems.push(Problem {
                kind: *kind,
                step: Step::Preflight,
                repo: repo.map(str::to_string),
                detail: "said so".into(),
            });
        }
        c.selected = Some(2);
        c
    }

    fn summaries(findings: &[Finding]) -> Vec<(Severity, String)> {
        findings
            .iter()
            .map(|f| (f.severity, f.summary.clone()))
            .collect()
    }

    /// The staleness window fits the shipped timer: past a day plus the
    /// timer's spread, so a run late by it still reads as running, and short
    /// of two days, so a missed day does not. A cadence shorter than daily
    /// passes untouched — G10's "a shorter cadence is a change to the timer
    /// alone" — and only a longer one (weekly, a day of the week, a day of
    /// the month) or a wider spread fails, until the window moves with it.
    #[test]
    fn the_staleness_window_fits_the_shipped_timer() {
        let timer = include_str!("../../../scripts/mecha-github-collect.timer");
        let field = |key: &str| {
            timer
                .lines()
                .find_map(|l| l.trim().strip_prefix(key))
                .map(|v| v.trim().to_string())
        };
        let calendar = field("OnCalendar=").expect("the timer has an OnCalendar line");
        let longer_than_daily = ["weekly", "monthly", "quarterly", "yearly", "annually"]
            .iter()
            .any(|w| calendar.eq_ignore_ascii_case(w))
            || ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]
                .iter()
                .any(|d| calendar.to_ascii_lowercase().starts_with(d))
            // Any day field but `*` — a literal day, a list, a repetition —
            // is less often than daily.
            || calendar
                .split_whitespace()
                .next()
                .filter(|date| date.contains('-'))
                .and_then(|date| date.rsplit('-').next())
                .is_some_and(|day| day != "*");
        assert!(
            !longer_than_daily,
            "{calendar:?} runs less often than daily; GITHUB_STALE_AFTER is set for a day"
        );
        // systemd's time spans: a bare number is seconds; `s`, `min`, `h`.
        let spread = field("RandomizedDelaySec=").map_or(Duration::zero(), |v| {
            let digits: String = v.chars().take_while(char::is_ascii_digit).collect();
            let n: i64 = digits.parse().expect("a time span");
            match v[digits.len()..].trim() {
                "" | "s" | "sec" => Duration::seconds(n),
                "m" | "min" => Duration::minutes(n),
                "h" | "hr" => Duration::hours(n),
                unit => panic!("a RandomizedDelaySec unit this test does not read: {unit}"),
            }
        });
        // A named zone makes the fall-back day 25 hours long.
        assert!(
            GITHUB_STALE_AFTER > Duration::hours(25) + spread,
            "a run late by the spread, on the 25-hour day, would read as stale"
        );
        assert!(
            GITHUB_STALE_AFTER < Duration::days(2),
            "a missed day would still read as running"
        );
    }

    #[test]
    fn a_home_without_the_store_has_nothing_to_say_unless_switched_on() {
        let h = home("absent");
        assert!(check_github(&github::db_path(&h), now()).is_empty());
        assert!(check_github_never_ran(&h, false).is_empty());
        assert_eq!(check_github_never_ran(&h, true).len(), 1);
    }

    /// The design's gate for step 1: a revoked login is a finding, not an
    /// empty table read as nothing failing.
    #[test]
    fn a_login_gone_is_broken_and_says_since_when() {
        let h = home("auth");
        let db = github::db_path(&h);
        record(&db, &run(now(), &[])).unwrap();
        for m in [10, 20, 30] {
            record(
                &db,
                &run(now() + Duration::minutes(m), &[(Kind::Auth, None)]),
            )
            .unwrap();
        }
        let f = check_github(&db, now() + Duration::minutes(30));
        assert_eq!(
            summaries(&f),
            [(
                Severity::Broken,
                "the GitHub collector's `gh` is not logged in, or GitHub refused its login".into()
            )]
        );
        assert!(
            f[0].detail
                .contains("every run since 2026-11-18 12:10Z (3 runs)"),
            "{}",
            f[0].detail
        );
        assert_eq!(f[0].remedy.as_ref().unwrap().argv, ["gh", "auth", "login"]);
    }

    #[test]
    fn each_kind_is_its_own_finding() {
        let h = home("kinds");
        let db = github::db_path(&h);
        record(
            &db,
            &run(
                now(),
                &[
                    (Kind::GhMissing, None),
                    (Kind::RateLimited, None),
                    (Kind::OutOfTime, None),
                    (Kind::Failed, Some("example-lab/tool-a")),
                    (Kind::Vanished, Some("example-lab/tool-b")),
                ],
            ),
        )
        .unwrap();
        let got = summaries(&check_github(&db, now()));
        assert_eq!(
            got,
            [
                (
                    Severity::Broken,
                    "the GitHub collector cannot start `gh`".into()
                ),
                (
                    Severity::Attention,
                    "the GitHub collector was rate-limited".into()
                ),
                (
                    Severity::Attention,
                    "the GitHub collector ran out of time".into()
                ),
                (
                    Severity::Attention,
                    "1 GitHub collection step(s) failed".into()
                ),
                (
                    Severity::Attention,
                    "1 watched GitHub repository vanished".into()
                ),
            ]
        );
    }

    /// Step 5's kinds: traffic refused for push access, a registry's refusal
    /// and a registry's failure, and an owner's package file that cannot be
    /// used — each its own finding, and none of them read as a failed
    /// collection step.
    #[test]
    fn usage_refusals_and_failures_are_each_their_own_finding() {
        let h = home("usage-kinds");
        let db = github::db_path(&h);
        record(
            &db,
            &run(
                now(),
                &[
                    (Kind::TrafficNoAccess, Some("example-lab/tool-a")),
                    (Kind::TrafficNoAccess, Some("example-lab/tool-b")),
                    (Kind::BadPackages, None),
                    (Kind::RegistryRefused, None),
                    (Kind::RegistryFailed, None),
                    (Kind::RegistryFailed, None),
                ],
            ),
        )
        .unwrap();
        let got = summaries(&check_github(&db, now()));
        assert_eq!(
            got,
            [
                (
                    Severity::Attention,
                    "GitHub refused traffic for 2 watched repositories: the login cannot push to them"
                        .into()
                ),
                (
                    Severity::Broken,
                    "the GitHub board's packages.toml cannot be used, so the package mapping is kept as it was"
                        .into()
                ),
                (
                    Severity::Attention,
                    "a package registry refused the GitHub collector".into()
                ),
                (
                    Severity::Attention,
                    "2 package registry counts failed".into()
                ),
            ]
        );
    }

    #[test]
    fn a_clean_recent_run_is_silent_and_a_stale_one_is_not() {
        let h = home("stale");
        let db = github::db_path(&h);
        record(&db, &run(now(), &[])).unwrap();
        // A daily run: a day later is still running, a missed day is not.
        assert!(check_github(&db, now() + Duration::hours(25)).is_empty());
        assert_eq!(
            summaries(&check_github(&db, now() + Duration::hours(37))),
            [(
                Severity::Attention,
                "the GitHub collector has not run for a day and a half".into()
            )]
        );
    }

    #[test]
    fn a_selection_that_watches_nothing_is_not_clean() {
        let h = home("empty-selection");
        let db = github::db_path(&h);
        let mut c = run(now(), &[]);
        c.selected = Some(0);
        record(&db, &c).unwrap();
        assert_eq!(
            summaries(&check_github(&db, now())),
            [(
                Severity::Attention,
                "the GitHub board's repos.toml selects no repository".into()
            )]
        );
        let mut c = run(now() + Duration::minutes(10), &[(Kind::BadSelection, None)]);
        c.selected = Some(0);
        record(&db, &c).unwrap();
        assert_eq!(
            summaries(&check_github(&db, now() + Duration::minutes(10))),
            [(
                Severity::Broken,
                "the GitHub board's repos.toml cannot be used, so nothing is watched".into()
            )]
        );
    }

    /// Summaries that have stopped being made are a finding, with how long:
    /// the collection is fine, and the board's New view shows titles alone,
    /// which reads exactly like summaries nobody needed.
    #[test]
    fn summaries_that_were_not_made_are_a_finding_and_say_since_when() {
        use crate::github::summary::{record_pass, Pass};
        let h = home("summary-skipped");
        let db = github::db_path(&h);
        for m in [0, 10] {
            let at = now() + Duration::minutes(m);
            record(&db, &run(at, &[])).unwrap();
            let skipped = Pass {
                skipped: Some("the model did not answer: the model server did not answer".into()),
                deferred: 3,
                ..Pass::default()
            };
            record_pass(&db, at, &skipped).unwrap();
        }
        let f = check_github(&db, now() + Duration::minutes(10));
        assert_eq!(
            summaries(&f),
            [(
                Severity::Attention,
                "the GitHub board's summaries were not made".into()
            )]
        );
        assert!(
            f[0].detail
                .contains("every run since 2026-11-18 12:00Z (2 runs)")
                && f[0].detail.contains("did not answer")
                && f[0].detail.contains("3 item(s) left"),
            "{}",
            f[0].detail
        );
    }

    #[test]
    fn a_failed_summary_and_an_all_refused_run_are_findings() {
        use crate::github::summary::{record_pass, Done, Outcome, Pass, Refusal};
        let h = home("summary-refused");
        let db = github::db_path(&h);
        record(&db, &run(now(), &[])).unwrap();
        let pass = Pass {
            done: vec![Done {
                repo: "example-lab/tool-a".into(),
                number: 4,
                body_hash: "h".into(),
                outcome: Outcome::Refused(Refusal::NotJson),
                model: Some("local-m".into()),
            }],
            failed: vec![(
                "example-lab/tool-a".into(),
                5,
                "the request was rejected".into(),
            )],
            ..Pass::default()
        };
        record_pass(&db, now(), &pass).unwrap();
        assert_eq!(
            summaries(&check_github(&db, now())),
            [
                (Severity::Attention, "1 GitHub summary failed".into()),
                (
                    Severity::Attention,
                    "every GitHub summary the last run asked for was refused".into()
                ),
            ]
        );
        // A run that made one is not all refused; a run with the pass off
        // (no counts) says nothing about summaries at all.
        let later = now() + Duration::minutes(10);
        record(&db, &run(later, &[])).unwrap();
        assert!(
            check_github(&db, later).is_empty(),
            "the pass off is not a finding"
        );
    }

    #[test]
    fn an_unreadable_store_is_a_finding() {
        let h = home("unreadable");
        let db = github::db_path(&h);
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        std::fs::write(&db, "not a database").unwrap();
        let f = check_github(&db, now());
        assert_eq!(f.len(), 1);
        assert!(
            f[0].summary.starts_with("store unreadable"),
            "{}",
            f[0].summary
        );
    }
}
