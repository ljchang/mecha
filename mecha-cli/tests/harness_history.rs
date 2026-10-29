//! The owner's typed verdict on a harness rejection, and the null arm's
//! readout (HARNESS-EVOLUTION-DESIGN.md P1, P2) — through the real binary
//! against a fixture home, with no provider and no network.
use mecha_core::harness::{HarnessCandidate, HarnessStore, STATUS_REJECTED, STATUS_STAGED};
use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Output};

struct Fixture {
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "mecha-harness-history-{}",
            mecha_core::session::Session::new_id()
        ));
        std::fs::create_dir_all(root.join("work")).unwrap();
        std::fs::create_dir_all(root.join("home")).unwrap();
        Fixture { root }
    }

    fn store(&self) -> HarnessStore {
        HarnessStore::open(self.root.join("home").join("learning").join("harness")).unwrap()
    }

    fn command(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(args)
            .current_dir(self.root.join("work"))
            .env("MECHA_HOME", self.root.join("home"))
            .env_remove("MECHA_SESSION_DIR")
            .env_remove("MECHA_LEARNING_DIR")
            .env("MECHA_SESSION_KIND", "test")
            .output()
            .unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.command(args);
        assert!(
            out.status.success(),
            "`mecha {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn staged(&self, id: &str, change: &str) {
        self.store()
            .write(&HarnessCandidate {
                id: id.into(),
                created_at: "2026-10-19T00:00:00Z".into(),
                class: mecha_core::candidate::ChangeClass::Config,
                change: change.into(),
                metric: mecha_core::candidate::Metric::Turns,
                rationale: "fixture".into(),
                evidence: "fixture".into(),
                model: None,
                status: STATUS_STAGED.into(),
                measurement: None,
                resolved_at: None,
                reason: None,
                rejection: None,
                cites: None,
                shown: None,
            })
            .unwrap();
    }
}

/// The incident P1 answers: a rejection that said "the measurement could not
/// tell" was banned with the rest. A plain reject is a plain no; the owner
/// can then type it, and only the type changes — the status, the time and
/// the curation ledger's single entry stay as they were.
#[test]
fn a_rejection_is_a_plain_no_until_the_owner_types_it() {
    let f = Fixture::new();
    f.staged("hc-tied", "max_turns=40");

    f.ok(&["harness", "reject", "hc-tied", "--reason", "all ties"]);
    let first = f.store().find("hc-tied").unwrap();
    assert_eq!(first.status, STATUS_REJECTED);
    assert!(f
        .ok(&["harness", "show", "hc-tied"])
        .contains("history:    RejectedUntyped\n"));

    // Already rejected, and no verdict to type: refused, record unchanged.
    let out = f.command(&["harness", "reject", "hc-tied"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("pass --verdict"));

    f.ok(&["harness", "reject", "hc-tied", "--verdict", "unmeasured"]);
    let retyped = f.store().find("hc-tied").unwrap();
    assert_eq!(retyped.status, STATUS_REJECTED);
    assert_eq!(
        retyped.resolved_at, first.resolved_at,
        "the rejection's time stands"
    );
    assert_eq!(
        retyped.reason.as_deref(),
        Some("all ties"),
        "no reason given, none replaced"
    );
    assert!(f
        .ok(&["harness", "show", "hc-tied"])
        .contains("history:    OwnerUnmeasured (may be proposed again)"));

    let (ledger, _) = mecha_core::curation::read_counting(f.store().root()).unwrap();
    assert_eq!(ledger.len(), 1, "a retype is not a second rejection");
}

/// Retyping a *gate* rejection is the owner overruling the gate — it can
/// re-open the change — so it is recorded as the owner's act: the gate wrote
/// no ledger line, and the retype writes the first.
#[test]
fn retyping_a_gate_rejection_is_recorded_as_the_owners_act() {
    let f = Fixture::new();
    f.staged("hc-gate", "max_turns=40");
    let mut c = f.store().find("hc-gate").unwrap();
    c.status = STATUS_REJECTED.into();
    c.rejection = Some(mecha_core::harness::Rejection::Gate);
    f.store().write(&c).unwrap();
    let ledger = |f: &Fixture| {
        mecha_core::curation::read_counting(f.store().root())
            .unwrap()
            .0
    };
    assert!(ledger(&f).is_empty(), "the gate's rejection wrote nothing");

    f.ok(&["harness", "reject", "hc-gate", "--verdict", "unmeasured"]);
    let after = ledger(&f);
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].act, mecha_core::curation::Act::Rejected);
    assert!(f.store().find("hc-gate").unwrap().rejected_by_owner());

    // Retyped again, it is already the owner's: still one line.
    f.ok(&["harness", "reject", "hc-gate", "--verdict", "harmful"]);
    assert_eq!(ledger(&f).len(), 1);
}

#[test]
fn a_verdict_given_at_rejection_is_kept_and_a_bad_one_is_refused() {
    let f = Fixture::new();
    f.staged("hc-short", "max_turns=3");
    let out = f.command(&["harness", "reject", "hc-short", "--verdict", "meh"]);
    assert!(
        !out.status.success(),
        "clap refuses a verdict outside the closed set"
    );
    assert!(f.store().find("hc-short").unwrap().pending());

    f.ok(&["harness", "reject", "hc-short", "--verdict", "harmful"]);
    assert!(f
        .ok(&["harness", "show", "hc-short"])
        .contains("history:    OwnerHarmful\n"));
    f.staged("hc-flat", "max_turns=10");
    f.ok(&["harness", "reject", "hc-flat", "--verdict", "no-effect"]);
    assert!(f
        .ok(&["harness", "show", "hc-flat"])
        .contains("history:    OwnerNoEffect\n"));
}

/// No null run yet means the floors are asserted, and the readout says so
/// rather than printing a rate over nothing. One recorded null run that
/// "accepted" under a metric is counted as a verdict on nothing.
#[test]
fn floors_say_asserted_until_a_null_run_measures_them() {
    let f = Fixture::new();
    assert!(f
        .ok(&["harness", "floors"])
        .contains("the floors are asserted, not measured"));
    let empty: Value = serde_json::from_str(&f.ok(&["harness", "floors", "--json"])).unwrap();
    assert_eq!(empty["null_runs"], 0);
    assert_eq!(empty["floors"]["min_selection_pairs"], 8);

    use mecha_core::candidate::{judge_drawn, ChangeClass, Metric, Prediction};
    let nothing = judge_drawn(
        ChangeClass::Config,
        &Prediction {
            metric: Metric::Turns,
            rationale: String::new(),
        },
        &[],
        &[],
    );
    let mut accepted = mecha_core::harness::NullJudgement::of(Metric::Turns, &nothing);
    accepted.disposition = "accept".into();
    let proposed = mecha_core::harness::NullJudgement::of(Metric::ToolErrorRate, &nothing);
    assert!(accepted.false_verdict() && !proposed.false_verdict());
    f.store()
        .write_null(&mecha_core::harness::NullRun {
            id: HarnessStore::mint_null_id(),
            drawn_by: Metric::Turns,
            measurement: mecha_core::harness::Measurement::record(
                &nothing,
                "m",
                "2026-10-19T00:00:00Z".into(),
                mecha_core::harness::Drawn {
                    episodes: Vec::new(),
                    holdout_episodes: Vec::new(),
                    seed: 1,
                    ranked: None,
                    diverged: vec!["s-1".into()],
                    replay_caveats: Vec::new(),
                    divergence_detail: vec![
                        mecha_core::harness::Divergence {
                            episode: "s-1".into(),
                            arm: mecha_core::harness::Arm::Baseline,
                            reason: "left the recording".into(),
                        },
                        mecha_core::harness::Divergence {
                            episode: "s-1".into(),
                            arm: mecha_core::harness::Arm::Candidate,
                            reason: "left the recording".into(),
                        },
                    ],
                    // Sessions the pool scan passed over before anything was
                    // drawn: never part of the denominator.
                    skipped: 40,
                },
            ),
            by_metric: vec![accepted, proposed],
            drawn: 3,
            arms_identical: false,
            no_headroom: Vec::new(),
        })
        .unwrap();
    let one: Value = serde_json::from_str(&f.ok(&["harness", "floors", "--json"])).unwrap();
    assert_eq!(one["null_runs"], 1);
    assert_eq!(one["accept"], 1);
    assert_eq!(one["propose"], 1);
    assert_eq!(one["runs_filling_both_floors"], 0);
    assert_eq!(
        one["episodes_drawn"], 3,
        "what was drawn, not what the scan skipped"
    );
    assert_eq!(
        one["replays"], 6,
        "both replays of a null run are the baseline"
    );
    assert_eq!(
        one["replays_diverged"], 2,
        "a divergence on either replay is the identity failing to reproduce itself"
    );
    assert!(f
        .ok(&["harness", "floors"])
        .contains("1 accepted, 0 rejected, 1 proposed"));
    // A null run is never a candidate: nothing waits on the owner.
    assert!(f.store().all().unwrap().is_empty());
}

/// A null run in which nothing paired judged nothing: it counts toward how
/// often the arms left the recording, and never as a run in which the gate
/// declined to react. Fails on a readout that judges the empty draw, which
/// gives a clean-looking `propose` under every metric.
#[test]
fn a_null_run_that_paired_nothing_is_not_a_clean_sheet() {
    use mecha_core::candidate::{judge_drawn, ChangeClass, Metric, Prediction};
    let f = Fixture::new();
    let nothing = judge_drawn(
        ChangeClass::Config,
        &Prediction {
            metric: Metric::Turns,
            rationale: String::new(),
        },
        &[],
        &[],
    );
    f.store()
        .write_null(&mecha_core::harness::NullRun {
            id: HarnessStore::mint_null_id(),
            drawn_by: Metric::Turns,
            measurement: mecha_core::harness::Measurement::record(
                &nothing,
                "m",
                "2026-10-19T00:00:00Z".into(),
                mecha_core::harness::Drawn {
                    episodes: Vec::new(),
                    holdout_episodes: Vec::new(),
                    seed: 1,
                    ranked: None,
                    diverged: vec!["s-1".into(), "s-2".into()],
                    replay_caveats: Vec::new(),
                    divergence_detail: Vec::new(),
                    skipped: 0,
                },
            ),
            by_metric: Vec::new(),
            drawn: 2,
            arms_identical: false,
            no_headroom: Vec::new(),
        })
        .unwrap();
    // And one whose arms replayed identically: it paired, but there was no
    // noise to react to, so it judged nothing either.
    let mut same = f.store().nulls_counting().unwrap().0.remove(0);
    same.id = HarnessStore::mint_null_id() + "-same";
    same.measurement.episodes = vec!["s-3".into()];
    // Full tallies, all ties: both floors look filled, and none of it is
    // power — the arms were the same run.
    same.measurement.selection = mecha_core::harness::TallyRecord {
        wins: 0,
        losses: 0,
        ties: 8,
    };
    same.measurement.holdout = mecha_core::harness::TallyRecord {
        wins: 0,
        losses: 0,
        ties: 4,
    };
    same.arms_identical = true;
    f.store().write_null(&same).unwrap();
    // And one that paired, differed, and had no metric with headroom to
    // judge: it judged nothing, and must land in a bucket of its own.
    let mut dry = same.clone();
    dry.id = HarnessStore::mint_null_id() + "-dry";
    dry.arms_identical = false;
    dry.no_headroom = mecha_core::candidate::Metric::ALL.to_vec();
    f.store().write_null(&dry).unwrap();

    let read: Value = serde_json::from_str(&f.ok(&["harness", "floors", "--json"])).unwrap();
    assert_eq!(read["null_runs"], 3);
    assert_eq!(read["runs_with_no_metric_to_judge"], 1);
    assert_eq!(read["runs_with_identical_arms"], 1);
    assert_eq!(
        read["runs_filling_both_floors"], 0,
        "ties from two replays of the same run fill no floor"
    );
    assert_eq!(read["runs_pairing_nothing"], 1);
    assert_eq!(read["judgements"], 0);
    assert_eq!(
        read["propose"], 0,
        "nothing judged is not a proposal per metric"
    );
    let text = f.ok(&["harness", "floors"]);
    assert!(
        text.contains(
            "1 paired nothing, 1 replayed identically on both arms, 1 had no metric with \
             headroom to judge"
        ),
        "{text}"
    );
    assert!(text.contains("no null run judged anything"), "{text}");
}
