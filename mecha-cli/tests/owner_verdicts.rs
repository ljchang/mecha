//! The verdicts the owner already gives, read back by `mecha sessions
//! appraise` (`docs/APPRAISAL-WIRING-DESIGN.md` S3a, PR 1d): task closures
//! and reopens from 1b's record, workflow close / cancel / reopen / verify,
//! reasoned draft rejections, and the owner's curation of rules and harness
//! candidates — exercised through the real binary against a fixture home,
//! with no provider and no network.
use mecha_core::closure::{Actor, ClosureStore, Entry, Move, Surface, Transition};
use mecha_core::session::{Record, RunStats, Session, SessionMeta};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const ADA: &str = "20260925T090000-ada";
const BRAM: &str = "20260925T090000-bram";

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
            "mecha-owner-verdicts-{}",
            mecha_core::session::Session::new_id()
        ));
        std::fs::create_dir_all(root.join("work")).unwrap();
        std::fs::create_dir_all(root.join("home")).unwrap();
        Fixture { root }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn command(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(args)
            .current_dir(self.root.join("work"))
            .env("MECHA_HOME", self.home())
            .env_remove("MECHA_SESSION_DIR")
            .env_remove("MECHA_LEARNING_DIR")
            .env_remove("MECHA_OUTBOX_DIR")
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

    /// A finished session: one owner turn and an outcome record, so the
    /// appraisal has something to appraise.
    fn session(&self, id: &str) {
        let s = Session::create(
            &self.home().join("sessions"),
            SessionMeta {
                id: id.into(),
                created_at: chrono::Utc::now(),
                provider: "scripted".into(),
                model: "m".into(),
                workspace: self.root.join("work"),
                title: None,
                kind: None,
            },
        )
        .unwrap();
        s.append(&Record::Message(mecha_core::message::Message::user(
            "write the quarterly summary",
        )))
        .unwrap();
        s.append(&Record::Outcome(RunStats::default())).unwrap();
    }
}

fn close(store: &ClosureStore, task: &str, session: &str) -> Transition {
    let t = Transition::new(
        task,
        Some("next"),
        "done",
        Move::Close,
        Actor::Owner,
        Surface::Web,
        vec![session.to_string()],
        None,
    );
    store.append(&Entry::Transition(t.clone())).unwrap();
    t
}

fn workflow_id(added: &str) -> String {
    let v: Value = serde_json::from_str(added).unwrap();
    v["id"].as_str().unwrap().to_string()
}

fn seed_rule(home: &Path) {
    let store = mecha_core::learning::LearningStore::open(home.join("learning")).unwrap();
    store
        .write_learned_rules(
            "behavior",
            &[mecha_core::learning::Rule {
                text: "Ask which account before drafting.".into(),
                id: Some("rule-fixture".into()),
                ..Default::default()
            }],
        )
        .unwrap();
}

fn seed_candidates(home: &Path) {
    let store =
        mecha_core::harness::HarnessStore::open(home.join("learning").join("harness")).unwrap();
    for (id, change) in [("hc-kept", "max_turns=40"), ("hc-refused", "max_turns=12")] {
        store
            .write(&mecha_core::harness::HarnessCandidate {
                id: id.into(),
                created_at: "2026-09-25T00:00:00Z".into(),
                class: mecha_core::candidate::ChangeClass::Config,
                change: change.into(),
                metric: mecha_core::candidate::Metric::Turns,
                rationale: "fixture".into(),
                evidence: "fixture".into(),
                model: None,
                status: mecha_core::harness::STATUS_STAGED.into(),
                measurement: None,
                resolved_at: None,
                reason: None,
            })
            .unwrap();
    }
}

#[test]
fn sessions_appraise_reads_every_verdict_the_owner_already_gives() {
    let f = Fixture::new();
    f.session(ADA);
    f.session(BRAM);

    // Task closures, as 1b records them: one kept, one reopened.
    let closures = ClosureStore::open(f.home().join("closures")).unwrap();
    close(&closures, "task-kept", ADA);
    let undone = close(&closures, "task-undone", BRAM);
    let mut reopen = Transition::new(
        "task-undone",
        Some("done"),
        "next",
        Move::Reopen,
        Actor::Owner,
        Surface::Tui,
        vec![BRAM.to_string()],
        None,
    );
    reopen.undoes = Some(undone.id.clone());
    closures.append(&Entry::Transition(reopen)).unwrap();

    // Workflows, through the owner's own verbs: a verify that fails on the
    // artifact, and a cancel.
    let checked = workflow_id(&f.ok(&["workflow", "add", "Quarterly summary", "--session", ADA]));
    std::fs::write(f.root.join("work").join("summary.md"), "draft, no totals").unwrap();
    f.ok(&[
        "workflow",
        "check",
        &checked,
        "--artifact",
        "summary.md",
        "--contains",
        "Totals",
    ]);
    f.ok(&["workflow", "verify", &checked]);
    let cancelled = workflow_id(&f.ok(&["workflow", "add", "Board memo", "--session", BRAM]));
    f.ok(&[
        "workflow",
        "cancel",
        &cancelled,
        "--reason",
        "no longer needed",
    ]);

    // A draft the owner rejected, saying why.
    let outbox = mecha_core::outbox::OutboxStore::open(f.home().join("outbox")).unwrap();
    let staged = outbox
        .stage(
            "mail_send",
            mecha_core::outbox::OutboxKind::Message,
            serde_json::json!({"to": "dirk@example.invalid", "body": "Totals attached."}),
            Default::default(),
            mecha_core::outbox::Provenance {
                session_id: Some(ADA.into()),
                ..Default::default()
            },
        )
        .unwrap();
    f.ok(&[
        "outbox",
        "reject",
        &staged.id,
        "--reason",
        "totals are wrong",
    ]);

    // The owner's curation of the learners, through their verbs.
    seed_rule(&f.home());
    f.ok(&["rules", "retire", "rule-fixture", "--reason", "too broad"]);
    f.ok(&["rules", "restore", "rule-fixture"]);
    seed_candidates(&f.home());
    f.ok(&["harness", "accept", "hc-kept"]);
    f.ok(&["harness", "revert", "hc-kept"]);
    f.ok(&[
        "harness",
        "reject",
        "hc-refused",
        "--reason",
        "turns are not the problem",
    ]);

    let v: Value =
        serde_json::from_str(&f.ok(&["sessions", "appraise", "--json", "--include-tests"]))
            .unwrap();
    assert_eq!(v["appraised"], 2, "{v:#}");
    assert_eq!(v["closures_read"], true);
    assert_eq!(v["workflows_read"], true);
    let acts = &v["owner_acts"];
    assert_eq!(acts["task_closed"], 1, "{acts:#}");
    assert_eq!(acts["task_reopened"], 1, "{acts:#}");
    assert_eq!(acts["workflow_verify_failed"], 1, "{acts:#}");
    assert_eq!(acts["workflow_cancelled"], 1, "{acts:#}");
    assert!(
        acts.get("workflow_closed").is_none(),
        "nothing was closed: {acts:#}"
    );
    assert_eq!(v["reasoned_rejections"], 1);
    assert_eq!(v["curation"]["rules"]["retired"], 1);
    assert_eq!(v["curation"]["rules"]["restored"], 1);
    assert_eq!(v["curation"]["harness"]["accepted"], 1);
    assert_eq!(v["curation"]["harness"]["reverted"], 1);
    assert_eq!(v["curation"]["harness"]["rejected"], 1);
    assert_eq!(v["curation"]["reflections"]["dropped"], 0);
    assert!(
        v["graph_fact_rejections"].is_null(),
        "unread, never zero: {v:#}"
    );
    // +0.5 for the kept closure; -1.0 reopen, -1.0 failed verify, -0.5
    // cancel, -1.0 rejected draft. The reopened closure's +0.5 is gone.
    assert_eq!(v["valence"]["positive"], 0.5, "{v:#}");
    assert_eq!(v["valence"]["negative"], 3.5, "{v:#}");

    // The human readout names each channel.
    let text = f.ok(&["sessions", "appraise", "--include-tests"]);
    for needle in [
        "owner acts on runs",
        "task_reopened",
        "workflow_verify_failed",
        "rejected with a reason",
        "owner verdicts on the learner (never a run's score)",
        "1 retired · 1 restored",
        "1 accepted · 1 rejected · 1 reverted",
        "graph fact rejections",
    ] {
        assert!(text.contains(needle), "`{needle}` missing from:\n{text}");
    }
}
