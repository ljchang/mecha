//! `mecha sessions mark` (ruling 4D, 2026-09-28): the owner withdraws a
//! session — a model probe run as ordinary chat — from every reader that
//! learns from their sessions. Exercised through the real binary against a
//! fixture home; no provider, no network.
// Like `closure_event.rs`, these cases read the owner's real shell registry
// too (`work::guard_homes` names the fixture and the real `~/.mecha`): run
// the suite from a plain terminal, not from a mecha session's `shell`.

use mecha_core::closure::RunPosture;
use mecha_core::learning::{LearningStore, Reflexion, Rule};
use mecha_core::session::{Marks, Session, SessionKind, SessionMeta};
use mecha_core::shell_registry::ShellRegistry;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Root(PathBuf);

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn mecha(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mecha"))
        .args(args)
        .current_dir(home)
        .env("MECHA_HOME", home)
        .env("MECHA_SESSION_KIND", "test")
        .env_remove("MECHA_SESSION_DIR")
        .env_remove("MECHA_LEARNING_DIR")
        .env_remove(mecha_core::closure::POSTURE_ENV)
        .output()
        .unwrap()
}

fn stdout(out: &Output) -> String {
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn the_owner_marks_a_probe_and_a_runs_shell_cannot() {
    mecha_core::session::ignore_kind_env_for_tests();
    let root = Root(std::env::temp_dir().join(format!("mecha-marks-{}", Session::new_id())));
    let home = root.0.join("home");
    let sessions = home.join("sessions");
    let probe = Session::create(
        &sessions,
        SessionMeta {
            id: Session::new_id(),
            created_at: chrono::Utc::now(),
            provider: "local".into(),
            model: "m".into(),
            workspace: home.clone(),
            title: None,
            kind: Some(SessionKind::Web),
        },
    )
    .unwrap()
    .meta
    .id;

    // A home that never learned anything: the mark reports nothing reached
    // and creates no learning store (review of #382).
    let out = stdout(&mecha(&home, &["sessions", "mark", &probe, "experiment"]));
    assert!(out.contains("marked as an experiment"), "{out}");
    assert!(
        !home.join("learning").exists(),
        "a read path created the store"
    );
    stdout(&mecha(&home, &["sessions", "unmark", &probe]));

    // Then it had been distilled, and one of its reflections already
    // reached learn and minted a live rule.
    let learning = LearningStore::open(home.join("learning")).unwrap();
    learning.mark_distilled(&probe).unwrap();
    let mut learned: Reflexion = serde_json::from_value(json!({
        "id": "refl-probe",
        "domain": "behavior",
        "session_id": probe,
        "trigger": "steer",
        "context": "c",
        "intervention": "not that",
        "reflexion_text": "Keep generating.",
        "error_type": null,
        "confidence": null,
        "created_at": "2026-09-27T00:00:00Z",
        "origin": "clean"
    }))
    .unwrap();
    learned.is_processed = true;
    learning.append_reflexion(&learned).unwrap();
    let rule: Rule = serde_json::from_value(json!({
        "text": "Keep generating.",
        "id": "rule-from-probe",
        "sources": ["refl-probe"]
    }))
    .unwrap();
    learning.write_learned_rules("behavior", &[rule]).unwrap();
    // And a second, still unprocessed, is held by a pending proposal.
    let held: Reflexion = serde_json::from_value(json!({
        "id": "refl-held",
        "domain": "behavior",
        "session_id": probe,
        "trigger": "denial",
        "context": "c",
        "intervention": "no",
        "reflexion_text": "Retry the refused call.",
        "error_type": null,
        "confidence": null,
        "created_at": "2026-09-27T00:00:00Z",
        "origin": "clean"
    }))
    .unwrap();
    learning.append_reflexion(&held).unwrap();
    let proposal: mecha_core::learning::Proposal = serde_json::from_value(json!({
        "id": "p-held",
        "domain": "behavior",
        "status": "pending",
        "reflexion_ids": ["refl-held"],
        "rules_before": [],
        "rules": [{"text": "Retry the refused call."}],
        "evidence": "",
        "created_at": "2026-09-27T00:00:00Z"
    }))
    .unwrap();
    learning.write_proposal(&proposal).unwrap();

    // The owner, at their own terminal: marked, and told what it reached —
    // including the graph episode, which is a separate store.
    let out = stdout(&mecha(
        &home,
        &[
            "sessions",
            "mark",
            &probe,
            "experiment",
            "--reason",
            "a model probe",
        ],
    ));
    assert!(out.contains("marked as an experiment"), "{out}");
    assert!(out.contains(&format!("(agent:mecha, {probe})")), "{out}");
    assert!(
        out.contains("1 reflection(s) from it already reached learn")
            && out.contains("rule-from-probe"),
        "the live rule is named, never passed over in silence: {out}"
    );
    assert!(
        out.contains("pending proposal(s) p-held hold reflection(s) from it"),
        "a held reflection is named with its proposal, never called withheld: {out}"
    );
    assert!(!out.contains("withheld from learn"), "{out}");
    // And the proposal cannot mint its rule while the mark stands, --force
    // or not.
    let out = mecha(&home, &["proposals", "accept", "p-held", "--force"]);
    assert!(!out.status.success(), "accepting must be refused");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("marked as an experiment") && err.contains("refl-held"),
        "{err}"
    );
    assert_eq!(
        LearningStore::open(home.join("learning"))
            .unwrap()
            .learned_rules("behavior")
            .unwrap()
            .len(),
        1,
        "no rule written"
    );
    let marks = Marks::load(&sessions).unwrap();
    assert!(marks.withdrawn(&probe));
    assert_eq!(
        marks.latest(&probe).unwrap().reason.as_deref(),
        Some("a model probe")
    );
    let out = stdout(&mecha(&home, &["sessions", "mark", &probe, "experiment"]));
    assert!(out.contains("already marked"), "{out}");

    // A run's shell — even an interactive one, with a person in the
    // conversation — cannot hide a session from the readers.
    let registration = ShellRegistry::open(home.join("runs/shells"))
        .unwrap()
        .register(
            std::process::id(),
            Some(RunPosture::Interactive),
            Some("call-test"),
        )
        .unwrap();
    let out = mecha(&home, &["sessions", "unmark", &probe]);
    assert!(!out.status.success(), "a run's shell must be refused");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("only the owner does"), "{err}");
    assert!(
        Marks::load(&sessions).unwrap().withdrawn(&probe),
        "nothing written"
    );
    drop(registration);

    let out = stdout(&mecha(&home, &["sessions", "unmark", &probe]));
    assert!(out.contains("unmarked"), "{out}");
    assert!(!Marks::load(&sessions).unwrap().withdrawn(&probe));

    // A transcript restored under another file name is marked by the id in
    // its header, which is what every reader keys on (review of #382).
    let restored = Session::create(
        &sessions,
        SessionMeta {
            id: Session::new_id(),
            created_at: chrono::Utc::now(),
            provider: "local".into(),
            model: "m".into(),
            workspace: home.clone(),
            title: None,
            kind: Some(SessionKind::Web),
        },
    )
    .unwrap();
    std::fs::rename(&restored.path, sessions.join("restored.jsonl")).unwrap();
    let id = restored.meta.id.clone();
    stdout(&mecha(&home, &["sessions", "mark", &id, "experiment"]));
    assert!(Marks::load(&sessions).unwrap().withdrawn(&id));
    let (listed, _) = Session::list_counting(&sessions).unwrap();
    assert_eq!(
        listed.iter().find(|(m, _)| m.id == id).unwrap().0.kind,
        Some(SessionKind::Experiment)
    );
}
