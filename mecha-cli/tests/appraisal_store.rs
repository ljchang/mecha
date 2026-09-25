//! Row 2a-1's acceptance through the real binary and a fresh handle: a text
//! appraisal of a clean session and one of a tainted session are both
//! stored; `mecha sessions appraise --json` counts them apart, with the
//! claims grounding dropped; and the clean door, read from a new handle on
//! the store the binary read, returns the clean one and never the other
//! (`docs/APPRAISAL-WIRING-DESIGN.md` I1, R18, R19). No producer exists yet
//! (row 2a-2), so the appraisals are written through the store's own door.

use mecha_core::agent::Taint;
use mecha_core::appraisal_store::{
    AppraisalStore, Bearing, Claim, Draft, Judgment, Pointer, Recorded, SessionEvidence,
};
use mecha_core::goal::GoalRef;
use mecha_core::message::{Block, Message};
use mecha_core::session::{Record, RunStats, Session, SessionKind, SessionMeta};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

struct Root(PathBuf);

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A delegated session that searched the owner's mail, as a front-end
/// records one. Returns its transcript path.
fn session(home: &Path, untrusted: bool) -> PathBuf {
    let session = Session::create(
        &home.join("sessions"),
        SessionMeta {
            id: Session::new_id(),
            created_at: chrono::Utc::now(),
            provider: "fixture".into(),
            model: "fixture".into(),
            workspace: home.to_path_buf(),
            title: None,
            kind: Some(SessionKind::Task),
        },
    )
    .unwrap();
    for m in [
        Message::user("when is the budget review?"),
        Message::assistant(vec![Block::ToolUse {
            id: "t1".into(),
            name: "mail_search".into(),
            input: json!({"query": "budget review"}),
        }]),
        Message::tool_results(vec![Block::ToolResult {
            tool_use_id: "t1".into(),
            content: "From Dana Rowe: the budget review moved to Thursday at 10.".into(),
            is_error: false,
        }]),
        Message::assistant(vec![Block::text("Thursday at 10.")]),
    ] {
        session.append(&Record::Message(m)).unwrap();
    }
    session
        .append(&Record::Outcome(RunStats {
            turns: 2,
            ..Default::default()
        }))
        .unwrap();
    session
        .append(&Record::Taint(Taint {
            untrusted,
            private: true,
        }))
        .unwrap();
    session.path
}

/// One claim that dereferences, one whose call the run never issued.
fn draft() -> Draft {
    Draft {
        interpretation: "The run answered from the owner's own mail; the date is what the \
                         budget task needed."
            .into(),
        judgments: vec![Judgment {
            goal: Some(GoalRef::Task("t-budget".into())),
            bearing: Bearing::Good,
            because: vec![0, 1],
        }],
        claims: vec![
            Claim {
                statement: "The review moved to Thursday".into(),
                pointer: Pointer::parse("result:t1"),
                quote: "the budget review moved to Thursday".into(),
            },
            Claim {
                statement: "Idris confirmed the room".into(),
                pointer: Pointer::parse("result:t7"),
                quote: "the room is confirmed for Thursday".into(),
            },
        ],
        prediction: Some("The owner will ask for the agenda next.".into()),
        ..Draft::default()
    }
}

async fn appraise(home: &Path, work: &Path) -> Value {
    let out = tokio::time::timeout(
        Duration::from_secs(60),
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            // `--include-tests`: CI exports `MECHA_SESSION_KIND=test`.
            .args(["sessions", "appraise", "--json", "--include-tests"])
            .env("MECHA_HOME", home)
            .env_remove("MECHA_SESSION_DIR")
            .env_remove("MECHA_LEARNING_DIR")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .current_dir(work)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("appraise finished")
    .unwrap();
    assert!(
        out.status.success(),
        "appraise failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "not JSON ({e}): {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

#[tokio::test]
async fn a_clean_appraisal_is_served_clean_and_a_tainted_one_only_to_the_owner() {
    let root =
        Root(std::env::temp_dir().join(format!("mecha-appraisal-store-{}", Session::new_id())));
    let home = root.0.join("home");
    let work = root.0.join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    let before = appraise(&home, &work).await;
    assert_eq!(before["text_appraisals"]["records"], 0, "{before:#}");
    assert_eq!(before["text_appraisals"]["read"], true);

    let clean = SessionEvidence::read(&session(&home, false)).unwrap();
    let tainted = SessionEvidence::read(&session(&home, true)).unwrap();
    let store = AppraisalStore::open(home.join("appraisals")).unwrap();
    for (evidence, expect_clean) in [(&clean, true), (&tainted, false)] {
        match store.record(evidence, draft(), "fixture").unwrap() {
            Recorded::Written {
                clean, grounding, ..
            } => {
                assert_eq!(clean, expect_clean);
                assert_eq!((grounding.offered, grounding.dropped), (2, 1));
            }
            other => panic!("{other:?}"),
        }
    }

    let after = appraise(&home, &work).await;
    let t = &after["text_appraisals"];
    assert_eq!(t["records"], 2, "{after:#}");
    assert_eq!(t["sessions"], 2);
    assert_eq!(t["clean"], 1);
    assert_eq!(t["not_clean"], 1);
    assert_eq!(t["claims_kept"], 2);
    assert_eq!(t["claims_dropped"], 2);
    assert_eq!(t["dropped_by"]["no_such_referent"], 2);
    assert_eq!(t["read"], true);

    // The second read: a fresh handle on the store the binary read.
    let read = AppraisalStore::open(home.join("appraisals"))
        .unwrap()
        .clean()
        .unwrap();
    assert_eq!(read.appraisals.len(), 1);
    assert_eq!(read.withheld, 1);
    let got = &read.appraisals[0];
    assert_eq!(
        got.session_id,
        clean.session_id(),
        "never {}",
        tainted.session_id()
    );
    assert_eq!(got.claims.len(), 1);
    assert_eq!(got.judgments[0].because, vec![0]);
    let (owner, skipped) = AppraisalStore::open(home.join("appraisals"))
        .unwrap()
        .for_owner()
        .unwrap();
    assert_eq!(skipped, 0);
    assert!(
        owner.iter().any(|r| r.session_id == tainted.session_id()),
        "the tainted appraisal is stored, for the owner"
    );
}
