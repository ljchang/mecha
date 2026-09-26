//! Row 2d-3's acceptance through the real binary (`docs/APPRAISAL-WIRING-
//! DESIGN.md` O3): a `mecha distill` pass writes each decided point-wise
//! comparison's losing arm into its session's appraisal, pointing at the
//! comparison; an undecided one writes nothing; a tainted session's
//! reflection stays tainted; a second pass writes nothing twice; and the
//! owner's readout shows the reflection under its appraisal. No model, no
//! network — a pass with nothing to distill still teaches. Fictional cast
//! only.

use mecha_core::comparison::{
    Arm, Comparison, ComparisonStore, Kind, Outcome, Pointers, Provenance, Recorded, Role,
    Validator,
};
use mecha_core::learning::Origin;
use mecha_core::surface::Fidelity;
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

async fn mecha(home: &Path, work: &Path, args: &[&str]) -> String {
    let out = tokio::time::timeout(
        Duration::from_secs(90),
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(args)
            .env("MECHA_HOME", home)
            .env("MECHA_SESSION_KIND", "test")
            .env_remove("MECHA_SESSION_DIR")
            .env_remove("MECHA_LEARNING_DIR")
            .env_remove("MECHA_TRIGGERS_DIR")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .current_dir(work)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap_or_else(|_| panic!("mecha {args:?} hung"))
    .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success(),
        "mecha {args:?} failed: {stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

/// An appraisal row as the store writes one, clean or tainted.
fn appraisal(session: &str, clean: bool) -> Value {
    json!({
        "id": format!("apr-{session}"),
        "at": "2026-09-24T00:00:00Z",
        "session_id": session,
        "origin": if clean { "clean" } else { "untrusted" },
        "taint": {"private": true, "untrusted": !clean},
        "model": "local-model",
        "interpretation": format!("In {session} the owner wanted the draft to Cleo Park held."),
    })
}

fn point(session: &str, kind: Kind, validator: Validator, arms: Vec<Arm>) -> Comparison {
    Comparison::new(
        kind,
        None,
        None,
        None,
        arms,
        validator,
        Pointers {
            session_id: session.into(),
            message_index: Some(6),
            call_index: Some(2),
            ..Pointers::default()
        },
        "local-model",
    )
}

/// Today's rules drafted the rejected mail again; no rules held it.
fn decided(session: &str) -> Comparison {
    point(
        session,
        Kind::PointRejectedDraft,
        Validator::RejectedDraft,
        vec![
            Arm::new(Role::Rules, Some("9f8e7d6c5b4a3210".into()), Outcome::Fail),
            Arm::new(Role::RulesFree, Arm::no_block(), Outcome::Pass),
        ],
    )
}

#[tokio::test]
async fn a_distill_pass_teaches_each_decided_losing_arm_once_into_its_appraisal() {
    let root = Root(std::env::temp_dir().join(format!(
        "mecha-losing-arm-{}",
        mecha_core::session::Session::new_id()
    )));
    let home = root.0.join("home");
    let work = root.0.join("work");
    let empty = root.0.join("no-sessions");
    for d in [&home, &work, &empty] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::create_dir_all(home.join("appraisals")).unwrap();
    std::fs::write(
        home.join("appraisals/appraisals.jsonl"),
        format!(
            "{}\n{}\n",
            appraisal("s-dana", true),
            appraisal("s-mara", false)
        ),
    )
    .unwrap();

    let store = ComparisonStore::open(home.join("comparisons")).unwrap();
    let clean = Provenance {
        origin: Origin::Clean,
        surface: Fidelity::Matches,
    };
    let dana = decided("s-dana");
    let mara = decided("s-mara");
    let undecided = point(
        "s-dana",
        Kind::PointEditedDraft,
        Validator::ReleasedDraft,
        vec![
            Arm::new(Role::Rules, Some("9f8e7d6c5b4a3210".into()), Outcome::Fail),
            Arm::new(Role::RulesFree, Arm::no_block(), Outcome::Inconclusive),
        ],
    );
    let unposed = point("s-dana", Kind::PointSurprise, Validator::Unposed, vec![]);
    for c in [&dana, &mara, &undecided, &unposed] {
        assert_eq!(store.record(clean, c).unwrap(), Recorded::Written);
    }
    let sessions = empty.display().to_string();
    let distill = ["distill", "--sessions-dir", sessions.as_str()];

    let first = mecha(&home, &work, &distill).await;
    assert!(
        first.contains(
            "losing arms: 2 taught into their session's appraisal (1 not clean — the owner's \
             alone) · 0 already taught · 0 whose session has no appraisal yet · 2 undecided"
        ),
        "{first}"
    );
    let second = mecha(&home, &work, &distill).await;
    assert!(
        second.contains("losing arms: 0 taught into their session's appraisal (0 not clean"),
        "{second}"
    );
    assert!(second.contains("2 already taught"), "{second}");
    let ledger = std::fs::read_to_string(home.join("appraisals/counterfactuals.jsonl")).unwrap();
    assert_eq!(ledger.lines().count(), 2, "nothing written twice: {ledger}");
    // The appraisal ledger itself was never rewritten.
    let appraisals = std::fs::read_to_string(home.join("appraisals/appraisals.jsonl")).unwrap();
    assert_eq!(appraisals.lines().count(), 2);

    // The owner's readout: the reflection under its appraisal, pointing at
    // its comparison, dereferencing, clean as the appraisal is.
    let read: Value = serde_json::from_str(
        &mecha(&home, &work, &["sessions", "appraise", "s-dana", "--json"]).await,
    )
    .unwrap();
    let reflections = read[0]["counterfactuals"].as_array().unwrap();
    assert_eq!(reflections.len(), 1, "{read:#}");
    let r = &reflections[0];
    assert_eq!(r["comparison"], format!("comparison:{}", dana.id));
    assert_eq!(r["appraisal_id"], "apr-s-dana");
    assert_eq!(r["dereferences"], true, "{r:#}");
    assert_eq!(r["clean"], true);
    assert!(r["reflection"]
        .as_str()
        .unwrap()
        .contains("under today's deployed rules (rules 9f8e7d6c5b4a), the arm drafted the text the owner rejected"));

    let tainted: Value = serde_json::from_str(
        &mecha(&home, &work, &["sessions", "appraise", "s-mara", "--json"]).await,
    )
    .unwrap();
    let r = &tainted[0]["counterfactuals"][0];
    assert_eq!(r["comparison"], format!("comparison:{}", mara.id));
    assert_eq!(
        r["clean"], false,
        "a tainted session's reflection stays tainted"
    );
    assert_eq!(tainted[0]["clean"], false);

    let text = mecha(&home, &work, &["sessions", "appraise", "s-dana"]).await;
    assert!(
        text.contains("counterfactual reflection cfr-")
            && text.contains(&format!("comparison:{} · dereferences · clean", dana.id)),
        "{text}"
    );

    // The store's counts carry them, beside the appraisals.
    let counts: Value = serde_json::from_str(
        &mecha(
            &home,
            &work,
            &["sessions", "appraise", "--json", "--include-tests"],
        )
        .await,
    )
    .unwrap();
    let t = &counts["text_appraisals"];
    assert_eq!(
        (&t["counterfactuals"], &t["counterfactuals_not_clean"]),
        (&json!(2), &json!(1)),
        "{t:#}"
    );
}
