//! A learned rule's standing with the owner: **tenure by the Wilson lower
//! bound of the owner-accept rate** on the runs that carried it
//! (`APPRAISAL-WIRING-DESIGN.md` L3, row 2e-5b, ruling R41), and the
//! report of a rule whose region has gone quiet (row 2e-5c).
//!
//! Ported from mecha-graph's autonomy ladder
//! (`mecha-graph-core/src/ladder.rs`), which promotes a (proposer,
//! predicate) class staged → sampled → trusted when the Wilson lower bound
//! on its *human* accept rate clears a rung's floor. What carried over, and
//! what changed:
//!
//! - **Only the owner's acts move it.** The ladder counts `HUMAN_VERDICT_SQL`
//!   rows and never a machine's; here a verdict is an appraisal error that
//!   records an act the owner performed
//!   ([`crate::appraisal::GoalError::is_owner_verdict`], R16's channels — the
//!   same predicate 2e-6's replay priority reads): a draft sent unchanged,
//!   edited or rejected, a question answered or abandoned, a task or
//!   workflow closed or reopened, a steer, denial or stop. Positive is an
//!   accept, negative a reject. Never a counter, a sensor, or a model's
//!   account of its own work.
//! - **A rule's verdicts are the ones on runs that carried it.** The ladder's
//!   class is the proposer that produced the fact; a rule has no such
//!   producer, so its record is the owner's verdicts on runs whose
//!   `RunConfig::rule_ids` carried it (R41). A verdict cited at a turn counts
//!   toward the rules the run record covering that turn carried; one cited
//!   by a draft, question, closure or workflow counts toward the rules
//!   *every* run record of its session carried, since which run it judged
//!   is not on the cite. Each rule gets its own rate.
//! - **The bound, not a streak and not a raw rate**: z = 1.96, the ladder's
//!   (`wilson_lower_bound`). A record under [`TENURE_MIN_VERDICTS`] has no
//!   bound — `None`, "not enough verdicts" — where the ladder answered 0.0
//!   and let the floor refuse it; R41 fixes the minimum at 20, the
//!   ladder's `GATE_MIN_JUDGED`.
//! - **One rung, not two.** [`TENURE_FLOOR`] is the ladder's
//!   `PROMOTE_LB_SAMPLED` (0.65), the bound that separated the classes
//!   review had vindicated. `PROMOTE_LB_TRUSTED` (0.85) does not map: a
//!   trusted class loses its spot-check, and a rule has no spot-check to
//!   lose — it rides in the prompt either way. `GATE_ACCEPT_LB_FLOOR` (0.15)
//!   does not map either: it stops a class being *generated*, which for a
//!   rule would be taking it out of the prompt, and R41 says nothing leaves
//!   the prompt on the bound.
//! - **Promotion only, as on the ladder.** A tenured rule is released from
//!   probation (`Rule::probation`, the shorter retirement leash) and shown
//!   as tenured; a low bound demotes nothing and retires nothing.
//!   Retirement stays on measured regressions in the validation ledger
//!   (`DEFAULT_RETIRE_AT`, `PROBATION_RETIRE_AT`), beside this, unchanged.
//!
//! **Unknown is never clean.** A session that carried the rule and whose
//! verdicts could not be read in full — the transcript does not parse, an
//! appraisal store did not load, or a run staged drafts and recorded no
//! outcome — makes the rule's standing [`Tenure::Unknown`], whatever the
//! bound over the rest would say: an unread session may hold the rejects.
//!
//! **Dormancy is a report** (row 2e-5c, R41): [`Quiet`] names an active rule
//! no admitted run in 2e-6's recurrence window was matched by. Nothing is
//! evicted, no slot changes and nothing stops loading — ARCHITECTURE's
//! "Acceptance is not tenure" still stands: the rarely-fired rule must
//! never expire.
//!
//! **Numbers stay harness-side** (G4, R21): nothing here is rendered into a
//! prompt.

use crate::appraisal::{self, Cite, Stores};
use crate::learning::Rule;
use crate::replay_priority::{Recurrence, RECURRENCE_WINDOW_DAYS};
use crate::session::{Session, Transcript};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The Wilson interval's z: 95% confidence, the ladder's.
pub const TENURE_Z: f64 = 1.96;

/// Below this many owner verdicts the bound decides nothing (R41; the
/// ladder's `GATE_MIN_JUDGED`).
pub const TENURE_MIN_VERDICTS: u32 = 20;

/// The lower bound a rule's owner-accept rate must reach to be tenured
/// (R41; the ladder's `PROMOTE_LB_SAMPLED`).
pub const TENURE_FLOOR: f64 = 0.65;

/// The lower bound of the Wilson score interval at [`TENURE_Z`] — the
/// accept rate the record still supports at 95% confidence. `None` below
/// [`TENURE_MIN_VERDICTS`]: too few verdicts is no bound, never a low one
/// and never a perfect one.
pub fn wilson_lower_bound(accepted: u32, judged: u32) -> Option<f64> {
    if judged < TENURE_MIN_VERDICTS || accepted > judged {
        return None;
    }
    let n = f64::from(judged);
    let p = f64::from(accepted) / n;
    let z2 = TENURE_Z * TENURE_Z;
    let centre = p + z2 / (2.0 * n);
    let margin = TENURE_Z * ((p * (1.0 - p) / n) + z2 / (4.0 * n * n)).sqrt();
    Some(((centre - margin) / (1.0 + z2 / n)).max(0.0))
}

/// One rule's owner verdicts, from the runs that carried it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct OwnerRecord {
    pub accepted: u32,
    pub rejected: u32,
    /// Sessions that carried the rule and whose verdicts could not be read
    /// in full.
    pub unread: u32,
}

impl OwnerRecord {
    pub fn judged(&self) -> u32 {
        self.accepted + self.rejected
    }

    /// The bound over what was read; `None` below the minimum.
    pub fn lower_bound(&self) -> Option<f64> {
        wilson_lower_bound(self.accepted, self.judged())
    }

    pub fn tenure(&self) -> Tenure {
        if self.unread > 0 {
            return Tenure::Unknown;
        }
        match self.lower_bound() {
            None => Tenure::TooFewVerdicts,
            Some(lb) if lb >= TENURE_FLOOR => Tenure::Tenured { lower_bound: lb },
            Some(lb) => Tenure::Untenured { lower_bound: lb },
        }
    }
}

/// Where a rule stands with the owner.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Tenure {
    /// The bound reached [`TENURE_FLOOR`]: released from probation.
    Tenured { lower_bound: f64 },
    /// Enough verdicts, and the bound fell short. Demotes nothing.
    Untenured { lower_bound: f64 },
    /// Under [`TENURE_MIN_VERDICTS`]: no bound, no decision.
    TooFewVerdicts,
    /// A session that carried the rule could not be read in full, or the
    /// store could not be walked: the record may be missing rejects.
    Unknown,
}

impl Tenure {
    pub fn is_tenured(&self) -> bool {
        matches!(self, Tenure::Tenured { .. })
    }

    /// Words for a readout; the bound as a figure only beside its count.
    pub fn describe(&self, r: &OwnerRecord) -> String {
        let counts = format!("{} of {} owner verdict(s) accepted", r.accepted, r.judged());
        match self {
            Tenure::Tenured { lower_bound } => {
                format!("tenured: {counts}, lower bound {lower_bound:.2}")
            }
            Tenure::Untenured { lower_bound } => format!(
                "not tenured: {counts}, lower bound {lower_bound:.2} under {TENURE_FLOOR:.2}"
            ),
            Tenure::TooFewVerdicts => format!(
                "not enough owner verdicts: {counts}, the bound needs {TENURE_MIN_VERDICTS}"
            ),
            Tenure::Unknown => format!(
                "unknown: {counts}, {} session(s) that carried it unreadable",
                r.unread
            ),
        }
    }
}

/// What one session says about the rules it carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionVerdicts {
    /// Each owner verdict, with the rules the run it judged carried.
    Read(Vec<(BTreeSet<String>, bool)>),
    /// The session's verdicts could not be read in full; every rule it
    /// carried is unknown.
    Unread,
}

/// Every rule id any run record of the session carried.
pub fn carried(t: &Transcript) -> BTreeSet<String> {
    t.configs
        .iter()
        .flat_map(|c| c.rule_ids.iter().cloned())
        .collect()
}

/// The rules every run record of the session carried — where a verdict
/// whose cite names no turn is counted. Empty when any record predates
/// `rules_hash`, since what that run carried is unknown.
fn carried_throughout(t: &Transcript) -> BTreeSet<String> {
    let mut configs = t.configs.iter();
    let Some(first) = configs.next() else {
        return BTreeSet::new();
    };
    if t.configs.iter().any(|c| c.rules_hash.is_none()) {
        return BTreeSet::new();
    }
    let mut out: BTreeSet<String> = first.rule_ids.iter().cloned().collect();
    for c in configs {
        let here: BTreeSet<&String> = c.rule_ids.iter().collect();
        out.retain(|id| here.contains(id));
    }
    out
}

/// The owner verdicts on one session, each attributed to the rules its run
/// carried. `appraisal` is the session's, built over `stores`; `None` is a
/// session with no outcome recorded, which carries no verdict unless it
/// staged drafts (`staged_drafts`), whose verdicts it then cannot say.
pub fn session_verdicts(
    t: &Transcript,
    appraisal: Option<&appraisal::Appraisal>,
    staged_drafts: bool,
) -> SessionVerdicts {
    let Some(a) = appraisal else {
        return if staged_drafts {
            SessionVerdicts::Unread
        } else {
            SessionVerdicts::Read(Vec::new())
        };
    };
    if a.partial {
        return SessionVerdicts::Unread;
    }
    let throughout = carried_throughout(t);
    let verdicts = a
        .errors
        .iter()
        .filter(|e| e.sign != 0.0 && e.is_owner_verdict())
        .map(|e| {
            let rules = match &e.cite {
                Cite::Turn(at) => t
                    .config_covering(*at)
                    .filter(|c| c.rules_hash.is_some())
                    .map(|c| c.rule_ids.iter().cloned().collect())
                    .unwrap_or_default(),
                _ => throughout.clone(),
            };
            (rules, e.sign > 0.0)
        })
        .collect();
    SessionVerdicts::Read(verdicts)
}

/// Every rule's owner record, and what the walk could not read.
#[derive(Debug, Clone, Default)]
pub struct Tally {
    pub records: BTreeMap<String, OwnerRecord>,
    /// The session store could not be walked at all: every rule unknown.
    pub store_unreadable: Option<String>,
    /// What else the walk could not read, one line each.
    pub caveats: Vec<String>,
}

impl Tally {
    /// Fold one session's verdicts in, counting only the rules asked for.
    pub fn add(
        &mut self,
        carried: &BTreeSet<String>,
        v: SessionVerdicts,
        wanted: &BTreeSet<String>,
    ) {
        match v {
            SessionVerdicts::Unread => {
                for id in carried.intersection(wanted) {
                    self.records.entry(id.clone()).or_default().unread += 1;
                }
            }
            SessionVerdicts::Read(verdicts) => {
                for (rules, accepted) in verdicts {
                    for id in rules.intersection(wanted) {
                        let r = self.records.entry(id.clone()).or_default();
                        if accepted {
                            r.accepted += 1;
                        } else {
                            r.rejected += 1;
                        }
                    }
                }
            }
        }
    }

    /// One rule's record; empty for a rule no read session carried.
    pub fn record(&self, id: &str) -> OwnerRecord {
        self.records.get(id).cloned().unwrap_or_default()
    }

    /// One rule's standing. Unknown for every rule when the store could not
    /// be walked.
    pub fn tenure(&self, id: &str) -> Tenure {
        if self.store_unreadable.is_some() {
            return Tenure::Unknown;
        }
        self.record(id).tenure()
    }

    /// Walk the admitted sessions (`runlog::Scan`'s admission: a smoke test
    /// or an experiment trial is not the owner judging) for the owner
    /// verdicts on runs that carried any of `wanted`, building each
    /// session's appraisal over `stores`. Only sessions whose run records
    /// name one of the rules are read in full.
    pub fn scan(sessions_dir: &Path, stores: &Stores, wanted: &BTreeSet<String>) -> Tally {
        let mut out = Tally::default();
        if wanted.is_empty() {
            return out;
        }
        let (listed, torn) = match Session::list_counting(sessions_dir) {
            Ok(l) => l,
            Err(e) => {
                out.store_unreadable =
                    Some(format!("the session store could not be listed ({e:#})"));
                return out;
            }
        };
        for name in stores.unreadable() {
            out.caveats.push(format!(
                "the {name} could not be read: every session's owner verdicts are unknown"
            ));
        }
        let admission = crate::runlog::Scan::default();
        let mut unscanned = torn;
        for (meta, path) in listed.iter().filter(|(m, _)| admission.admits(m)) {
            let carrying = match Session::run_configs_streaming(path) {
                Ok(configs) => configs
                    .iter()
                    .any(|c| c.rule_ids.iter().any(|id| wanted.contains(id))),
                Err(_) => {
                    unscanned += 1;
                    continue;
                }
            };
            if !carrying {
                continue;
            }
            match Session::read(path) {
                Ok(t) => {
                    let drafts = stores.drafts_of(&meta.id);
                    let built = appraisal::for_transcript(
                        &t,
                        &meta.id,
                        meta.created_at.to_rfc3339(),
                        stores.records(&drafts),
                        None,
                    );
                    let v = session_verdicts(
                        &t,
                        built.as_ref().map(|b| &b.appraisal),
                        !drafts.is_empty(),
                    );
                    out.add(&carried(&t), v, wanted);
                }
                Err(_) => {
                    // Carried one of them, by its run records, and could
                    // not be read: every rule it named is unknown.
                    let ids: BTreeSet<String> = Session::run_configs_streaming(path)
                        .map(|cs| cs.into_iter().flat_map(|c| c.rule_ids).collect())
                        .unwrap_or_default();
                    out.add(&ids, SessionVerdicts::Unread, wanted);
                }
            }
        }
        if unscanned > 0 {
            out.caveats.push(format!(
                "{unscanned} transcript(s) whose header or run records could not be read: \
                 whether they carried a rule is unknown"
            ));
        }
        out
    }
}

/// Release from probation every rule the owner's verdicts have tenured —
/// beside [`crate::learning::release_probation_when_measured_clean`], never
/// instead of it. Returns how many were released. Nothing is retired,
/// narrowed or demoted here, and a rule not on probation is untouched.
pub fn release_probation_when_owner_tenures(rules: &mut [Rule], tally: &Tally) -> usize {
    let mut released = 0;
    for r in rules.iter_mut().filter(|r| r.probation) {
        if r.id
            .as_deref()
            .is_some_and(|id| tally.tenure(id).is_tenured())
        {
            r.probation = false;
            released += 1;
        }
    }
    released
}

/// Whether an active rule's region has gone quiet (row 2e-5c) — a report,
/// never an eviction.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Quiet {
    /// Admitted runs in the window matched by the rule's scope.
    Recurring { runs: usize },
    /// No admitted run in the window was matched by its scope, and every
    /// session in it was read.
    Quiet,
    /// The rule is younger than the window: too new to call quiet.
    TooNew,
    /// Not known, and why: the walk failed, part of the window could not be
    /// read, or the rule carries no birth date to measure the window from.
    Unknown { reason: String },
}

impl Quiet {
    /// One rule against the window's runs. `recurrence` is `None` when the
    /// walk failed.
    pub fn of(rule: &Rule, recurrence: Option<&Recurrence>, now: DateTime<Utc>) -> Quiet {
        let Some(rec) = recurrence else {
            return Quiet::Unknown {
                reason: "the session store could not be walked".into(),
            };
        };
        let runs = rec.matching(rule.scope.as_ref());
        if runs > 0 {
            return Quiet::Recurring { runs };
        }
        let born = rule
            .created_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc));
        match born {
            None => Quiet::Unknown {
                reason: "no birth date on the rule to measure the window from".into(),
            },
            Some(b) if now - b < chrono::Duration::days(RECURRENCE_WINDOW_DAYS) => Quiet::TooNew,
            Some(_) if rec.unreadable > 0 || rec.beyond_cap > 0 => Quiet::Unknown {
                reason: format!(
                    "no run found in its region, but {} session(s) in the window were not read",
                    rec.unreadable + rec.beyond_cap
                ),
            },
            Some(_) => Quiet::Quiet,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Quiet::Recurring { runs } => {
                format!("{runs} run(s) in its region in the last {RECURRENCE_WINDOW_DAYS} days")
            }
            Quiet::Quiet => format!(
                "QUIET: no run in its region in the last {RECURRENCE_WINDOW_DAYS} days \
                 (reported only; it still loads)"
            ),
            Quiet::TooNew => format!("younger than the {RECURRENCE_WINDOW_DAYS}-day window"),
            Quiet::Unknown { reason } => format!("quiet unknown: {reason}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appraisal::{Agency, Channel, GoalError};
    use crate::message::{Block, Message};
    use crate::session::{Record, RunConfig, RunStats, SessionKind, SessionMeta};
    use crate::situation::Situation;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "mecha-tenure-{tag}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn ids(xs: &[&str]) -> BTreeSet<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    fn carrying(xs: &[&str]) -> RunConfig {
        RunConfig {
            rules_hash: Some("h".into()),
            rule_ids: xs.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    /// A session: one run per config, each holding a denial by the owner
    /// (an owner verdict cited at the tool-result turn), with an outcome.
    fn session(dir: &Path, kind: SessionKind, runs: &[RunConfig]) -> Session {
        let s = Session::create(
            dir,
            SessionMeta {
                id: Session::new_id(),
                created_at: chrono::Utc::now(),
                provider: "p".into(),
                model: "m".into(),
                workspace: "/w".into(),
                title: None,
                kind: Some(kind),
            },
        )
        .unwrap();
        for (i, c) in runs.iter().enumerate() {
            s.append(&Record::Config(c.clone())).unwrap();
            let id = format!("c{i}");
            s.append_messages(&[
                Message::user("tidy the Northwind notes"),
                Message::assistant(vec![Block::ToolUse {
                    id: id.clone(),
                    name: "shell".into(),
                    input: serde_json::json!({"command": "rm -rf notes"}),
                }]),
                Message::tool_results(vec![Block::ToolResult {
                    tool_use_id: id,
                    content: "Denied by the user: not that".into(),
                    is_error: true,
                }]),
                Message::assistant(vec![Block::Text {
                    text: "understood".into(),
                }]),
            ])
            .unwrap();
            s.append(&Record::Outcome(RunStats::default())).unwrap();
        }
        s
    }

    fn built(t: &Transcript) -> appraisal::Appraisal {
        appraisal::for_transcript(
            t,
            &t.meta.id,
            t.meta.created_at.to_rfc3339(),
            Default::default(),
            None,
        )
        .expect("an outcome is recorded")
        .appraisal
    }

    fn owner_error(cite: Cite, sign: f32) -> GoalError {
        GoalError {
            related: Vec::new(),
            goal: None,
            channel: match cite {
                Cite::Draft(_) => Channel::Edit,
                Cite::Turn(_) => Channel::Intervention,
                _ => Channel::Commitment,
            },
            sign,
            agency: Agency::Owner,
            visible: true,
            controllable: None,
            cite,
        }
    }

    fn rec(accepted: u32, rejected: u32, unread: u32) -> OwnerRecord {
        OwnerRecord {
            accepted,
            rejected,
            unread,
        }
    }

    /// The ladder's own figures (`ladder.rs`'s `PROMOTE_LB_SAMPLED` notes):
    /// 41 of 51 lands near 0.675 and clears the floor, 31 of 46 near 0.53
    /// does not; under twenty verdicts there is no bound at all — 19 of 19
    /// is not a perfect record, and none is not a zero.
    #[test]
    fn the_bound_is_the_ladders_and_says_nothing_under_twenty() {
        let lb = wilson_lower_bound(41, 51).unwrap();
        assert!((lb - 0.675).abs() < 0.01, "{lb}");
        assert!(lb >= TENURE_FLOOR);
        let lb = wilson_lower_bound(31, 46).unwrap();
        assert!((lb - 0.53).abs() < 0.01, "{lb}");
        assert!(lb < TENURE_FLOOR);
        assert_eq!(wilson_lower_bound(19, 19), None);
        assert_eq!(wilson_lower_bound(0, 0), None);
        let all = wilson_lower_bound(20, 20).unwrap();
        assert!(all > TENURE_FLOOR && all < 1.0, "{all}");
        assert_eq!(wilson_lower_bound(0, 20), Some(0.0));
    }

    #[test]
    fn tenure_is_the_bound_at_the_floor_and_unknown_with_an_unread_session() {
        assert!(rec(41, 10, 0).tenure().is_tenured());
        assert!(matches!(rec(31, 15, 0).tenure(), Tenure::Untenured { .. }));
        assert_eq!(rec(19, 0, 0).tenure(), Tenure::TooFewVerdicts);
        assert_eq!(rec(0, 0, 0).tenure(), Tenure::TooFewVerdicts);
        assert_eq!(
            rec(41, 10, 1).tenure(),
            Tenure::Unknown,
            "an unread session may hold the rejects"
        );
    }

    /// A verdict at a turn counts toward what the run holding that turn
    /// carried; one cited by a draft (or any cite with no turn) toward what
    /// every run of the session carried; a run from before `rules_hash`
    /// carried nothing anyone can name; and only owner acts count — a
    /// counter's error is not a verdict.
    #[test]
    fn a_verdict_counts_toward_the_rules_its_run_carried() {
        let dir = scratch("attribution");
        let s = session(
            &dir,
            SessionKind::Web,
            &[carrying(&["r1", "r2"]), carrying(&["r2", "r3"])],
        );
        let t = Session::read(&s.path).unwrap();
        let mut a = built(&t);
        let denials: Vec<usize> = a
            .errors
            .iter()
            .filter(|e| e.is_owner_verdict())
            .filter_map(|e| match e.cite {
                Cite::Turn(at) => Some(at),
                _ => None,
            })
            .collect();
        assert_eq!(denials, vec![2, 6], "one denial per run");
        a.errors
            .push(owner_error(Cite::Draft("d-dana".into()), 1.0));
        a.errors.push(GoalError {
            channel: Channel::Counter,
            ..owner_error(Cite::Counter("turns".into()), -1.0)
        });
        let SessionVerdicts::Read(v) = session_verdicts(&t, Some(&a), true) else {
            panic!("read in full");
        };
        assert_eq!(
            v,
            vec![
                (ids(&["r1", "r2"]), false),
                (ids(&["r2", "r3"]), false),
                (ids(&["r2"]), true),
            ]
        );

        let mut tally = Tally::default();
        tally.add(
            &carried(&t),
            SessionVerdicts::Read(v),
            &ids(&["r1", "r2", "r3"]),
        );
        assert_eq!(tally.record("r2"), rec(1, 2, 0));
        assert_eq!(tally.record("r1"), rec(0, 1, 0));

        // A run record from before `rules_hash`: nothing is attributed.
        let old = session(&dir, SessionKind::Web, &[RunConfig::default()]);
        let t = Session::read(&old.path).unwrap();
        let mut a = built(&t);
        a.errors.push(owner_error(Cite::Draft("d-old".into()), 1.0));
        assert_eq!(
            session_verdicts(&t, Some(&a), true),
            SessionVerdicts::Read(vec![(BTreeSet::new(), false), (BTreeSet::new(), true)])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Unknown is never clean: a partial appraisal, or a session that
    /// staged drafts and recorded no outcome, makes every rule it carried
    /// unknown.
    #[test]
    fn a_partial_or_unfinished_session_is_unread() {
        let dir = scratch("unread");
        let s = session(&dir, SessionKind::Web, &[carrying(&["r1"])]);
        let t = Session::read(&s.path).unwrap();
        let mut a = built(&t);
        a.partial = true;
        assert_eq!(
            session_verdicts(&t, Some(&a), false),
            SessionVerdicts::Unread
        );
        assert_eq!(session_verdicts(&t, None, true), SessionVerdicts::Unread);
        assert_eq!(
            session_verdicts(&t, None, false),
            SessionVerdicts::Read(Vec::new()),
            "no outcome and nothing staged: no verdict to miss"
        );
        let mut tally = Tally::default();
        tally.add(&carried(&t), SessionVerdicts::Unread, &ids(&["r1"]));
        assert_eq!(tally.tenure("r1"), Tenure::Unknown);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The walk: an admitted session carrying the rule is read; a smoke
    /// test and an experiment trial are not the owner judging; a session
    /// carrying only other rules is never opened.
    #[test]
    fn the_scan_reads_admitted_sessions_that_carried_the_rule() {
        let dir = scratch("scan");
        session(&dir, SessionKind::Web, &[carrying(&["r1"])]);
        session(&dir, SessionKind::Task, &[carrying(&["r1", "r9"])]);
        session(&dir, SessionKind::Test, &[carrying(&["r1"])]);
        session(&dir, SessionKind::Experiment, &[carrying(&["r1"])]);
        session(&dir, SessionKind::Web, &[carrying(&["r9"])]);
        let tally = Tally::scan(&dir, &Stores::default(), &ids(&["r1"]));
        assert_eq!(tally.record("r1"), rec(0, 2, 0));
        assert_eq!(tally.record("r9"), OwnerRecord::default(), "not asked for");
        assert!(tally.caveats.is_empty(), "{:?}", tally.caveats);
        assert_eq!(tally.tenure("r1"), Tenure::TooFewVerdicts);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn probationary(id: &str) -> Rule {
        Rule {
            text: format!("rule {id}"),
            id: Some(id.into()),
            probation: true,
            ..Default::default()
        }
    }

    /// Row 2e-5b's acceptance: a rule's standing moves on the owner's
    /// verdicts by the bound, not a streak. Tenured releases probation; too
    /// few (however clean), a low bound, an unread session or an unwalkable
    /// store release nothing; a rule off probation is untouched, and
    /// nothing is retired.
    #[test]
    fn owner_tenure_releases_probation_and_nothing_else() {
        let mut tally = Tally::default();
        tally.records.insert("good".into(), rec(41, 10, 0));
        tally.records.insert("streak".into(), rec(19, 0, 0));
        tally.records.insert("poor".into(), rec(31, 15, 0));
        tally.records.insert("torn".into(), rec(41, 10, 1));
        let mut rules = vec![
            probationary("good"),
            probationary("streak"),
            probationary("poor"),
            probationary("torn"),
            Rule {
                probation: false,
                ..probationary("poor")
            },
        ];
        assert_eq!(release_probation_when_owner_tenures(&mut rules, &tally), 1);
        assert_eq!(
            rules.iter().map(|r| r.probation).collect::<Vec<_>>(),
            vec![false, true, true, true, false]
        );
        assert!(rules.iter().all(|r| r.active() && r.retired_at.is_none()));

        let mut rules = vec![probationary("good")];
        let unwalked = Tally {
            store_unreadable: Some("gone".into()),
            ..tally
        };
        assert_eq!(
            release_probation_when_owner_tenures(&mut rules, &unwalked),
            0
        );
        assert!(rules[0].probation);
    }

    fn run(tools: &[&str]) -> Situation {
        Situation::of_run(
            &tools.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
            None,
        )
    }

    /// Row 2e-5c: a rule whose scope no run in the window matched is named
    /// quiet — only when it is older than the window and the window was
    /// read in full; a standing rule matches every run; nothing is changed.
    #[test]
    fn a_rule_is_quiet_when_no_run_in_the_window_matched_its_scope() {
        let now = chrono::Utc::now();
        let old = (now - chrono::Duration::days(RECURRENCE_WINDOW_DAYS + 5)).to_rfc3339();
        let young = (now - chrono::Duration::days(2)).to_rfc3339();
        let scoped = |tool: &str, born: &str| Rule {
            text: "t".into(),
            id: Some("r".into()),
            created_at: Some(born.into()),
            scope: Some(run(&[tool]).scope()),
            ..Default::default()
        };
        let mut rec = Recurrence::default();
        rec.runs = vec![
            run(&["shell", "fs_read"]),
            run(&["shell"]),
            run(&["mail_send"]),
        ];
        assert_eq!(
            Quiet::of(&scoped("shell", &old), Some(&rec), now),
            Quiet::Recurring { runs: 2 }
        );
        assert_eq!(
            Quiet::of(&scoped("http_fetch", &old), Some(&rec), now),
            Quiet::Quiet
        );
        assert_eq!(
            Quiet::of(&scoped("http_fetch", &young), Some(&rec), now),
            Quiet::TooNew
        );
        let standing = Rule {
            scope: None,
            ..scoped("x", &old)
        };
        assert_eq!(
            Quiet::of(&standing, Some(&rec), now),
            Quiet::Recurring { runs: 3 }
        );
        assert!(matches!(
            Quiet::of(&scoped("http_fetch", &old), None, now),
            Quiet::Unknown { .. }
        ));
        let unborn = Rule {
            created_at: None,
            ..scoped("http_fetch", &old)
        };
        assert!(matches!(
            Quiet::of(&unborn, Some(&rec), now),
            Quiet::Unknown { .. }
        ));
        let mut torn = rec.clone();
        torn.unreadable = 1;
        assert!(matches!(
            Quiet::of(&scoped("http_fetch", &old), Some(&torn), now),
            Quiet::Unknown { .. }
        ));
        assert_eq!(
            Quiet::of(&scoped("shell", &old), Some(&torn), now),
            Quiet::Recurring { runs: 2 },
            "a match found is a match, whatever else was unread"
        );
    }
}
