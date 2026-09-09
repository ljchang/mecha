//! Exercise the owner's CLI, including mode preservation, with no provider or network.
use mecha_core::{
    agent::Taint,
    outbox::{OutboxKind, OutboxStore, Provenance},
};
use serde_json::{json, Value};
use std::process::{Command, Output};
struct Fixture {
    root: std::path::PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    fn command(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(args)
            .current_dir(self.root.join("work"))
            .env("MECHA_HOME", self.root.join("home"))
            .env_remove("MECHA_OUTBOX_DIR")
            .env("MECHA_SESSION_KIND", "test")
            .output()
            .unwrap()
    }
}
fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn owner_commands_preserve_guidance_and_link_post_delivery_feedback() {
    let f = Fixture {
        root: std::env::temp_dir().join(format!(
            "mecha-anticipation-cli-{}",
            mecha_core::session::Session::new_id()
        )),
    };
    std::fs::create_dir_all(f.root.join("work")).unwrap();
    let store = OutboxStore::open(f.root.join("home/outbox")).unwrap();
    let draft = store
        .stage(
            "mail_send",
            OutboxKind::Message,
            json!({"to":"test@example.invalid","body":"Meeting at 10"}),
            Taint::default(),
            Provenance::default(),
        )
        .unwrap();
    let path = f.root.join("evidence.json");
    let mut evidence = json!({"goal":"task:meeting", "commitment": {
        "beneficiary":"attendees", "expectation":"send the correct time", "consequence":"missed meeting"
    }, "check_available":true, "check_cost_secs":10, "time_available_secs":3600});
    std::fs::write(&path, evidence.to_string()).unwrap();
    let wrong_goal = f.command(&[
        "run",
        "--goal",
        "task:other",
        "--appraisal-evidence",
        path.to_str().unwrap(),
        "draft an invitation",
    ]);
    assert!(!wrong_goal.status.success());
    assert!(String::from_utf8_lossy(&wrong_goal.stderr)
        .contains("appraisal evidence must match --goal"));
    let first = success(f.command(&[
        "outbox",
        "anticipate",
        &draft.id,
        "--file",
        path.to_str().unwrap(),
        "--guide",
    ]));
    assert_eq!(first["predictions"][1]["prediction"]["guide"], true);
    assert!(store.begin_delivery(&draft.id).is_err());
    evidence["verification"] = json!("passed");
    evidence["verification_evidence"] = json!("owner checked calendar entry");
    std::fs::write(&path, evidence.to_string()).unwrap();
    let checked = success(f.command(&[
        "outbox",
        "anticipate",
        &draft.id,
        "--file",
        path.to_str().unwrap(),
    ]));
    assert_eq!(
        checked["predictions"][2]["prediction"]["guide"], true,
        "omitting the flag preserves the gate"
    );
    let pid = checked["predictions"][2]["prediction"]["id"]
        .as_str()
        .unwrap();
    // Simulated acknowledgement, never an actual send.
    store.begin_delivery(&draft.id).unwrap();
    store
        .resolve_with_output(&draft.id, "sent", None, Some("fixture receipt".into()))
        .unwrap();
    let input = json!({"prediction_id":pid,"verdict":"harm", "attributable_to_mecha":true,
        "evidence":"owner confirmed the incorrect time caused a missed meeting"});
    std::fs::write(&path, input.to_string()).unwrap();
    let result = success(f.command(&[
        "outbox",
        "outcome",
        &draft.id,
        "--file",
        path.to_str().unwrap(),
    ]));
    assert_eq!(result["predictions"][2]["resolution"], "observed");
    assert_eq!(result["outcomes"][0]["observation"]["verdict"], "harm");
    assert!(
        !f.command(&[
            "outbox",
            "outcome",
            &draft.id,
            "--file",
            path.to_str().unwrap()
        ])
        .status
        .success(),
        "a repeated report must explicitly supersede prior evidence"
    );
}
