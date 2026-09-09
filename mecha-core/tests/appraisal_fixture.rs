//! Paired planning conditions and the independent artifact oracle, without a model.
mod support;

use mecha_core::config::{Config, ProviderConfig};
use mecha_core::experiment::{child_invocation, Manifest, SourceTask};
use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

#[test]
fn explicit_planning_levers_materialize_distinct_paired_conditions() {
    let manifest = Manifest::parse(include_str!("../../eval/appraisal-guidance.toml")).unwrap();
    let mut real = Config {
        default_provider: "local".into(),
        ..Default::default()
    };
    real.providers.insert(
        "local".into(),
        ProviderConfig {
            kind: "local".into(),
            ..Default::default()
        },
    );
    // Both operator settings are false: on must enable, not merely omit an off flag.
    real.agent.step_checks = false;
    real.agent.goal_guidance = false;
    let control = child_invocation(&real, &manifest.arms["control"], Some(3)).unwrap();
    let mut guided = child_invocation(&real, &manifest.arms["guided"], Some(3)).unwrap();
    assert!(control.config.agent.step_checks && guided.config.agent.step_checks);
    assert!(!control.config.agent.goal_guidance);
    assert!(guided.config.agent.goal_guidance);
    assert_eq!(control.flags, guided.flags);
    guided.config.agent.goal_guidance = false;
    assert_eq!(
        serde_json::to_value(control.config).unwrap(),
        serde_json::to_value(guided.config).unwrap(),
        "guidance is the only changed configuration field"
    );
    let inherited = child_invocation(&real, &Default::default(), None).unwrap();
    assert!(!inherited.config.agent.step_checks && !inherited.config.agent.goal_guidance);
}

#[test]
fn appraisal_source_obeys_the_driver_contract_and_rejects_false_success() {
    if support::unavailable("python3", support::python3_available()) {
        return;
    }
    let listed = Command::new("python3")
        .current_dir(root())
        .args(["-B", "eval/fixtures/appraisal_source.py", "list"])
        .output()
        .unwrap();
    assert!(
        listed.status.success(),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let tasks: Vec<SourceTask> = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(tasks.len(), 12);
    let result = Command::new("python3")
        .current_dir(root())
        .args(["-B", "eval/fixtures/test_appraisal_source.py"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn appraisal_report_preserves_pairing_and_missing_observations() {
    if support::unavailable("python3", support::python3_available()) {
        return;
    }
    let result = Command::new("python3")
        .current_dir(root())
        .args(["-B", "scripts/test_appraisal_report.py"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn synthetic_drafts_are_readable_by_the_production_outbox_schema() {
    if support::unavailable("python3", support::python3_available()) {
        return;
    }
    let temp = support::tmpdir("appraisal-fixture");
    let home = temp.join("home");
    let workspace = temp.join("workspace");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(home.join(mecha_core::experiment::HOME_MARKER), "test").unwrap();
    mecha_core::charter::Charter::parse(include_str!("../../eval/fixtures/appraisal/charter.toml"))
        .unwrap();
    for (task, expected) in [("review-pressure", 3), ("review-quiet", 0)] {
        let output = Command::new("python3")
            .current_dir(root())
            .args(["-B", "eval/fixtures/appraisal_source.py", "setup", task])
            .envs(mecha_core::experiment::source_env(
                &home,
                &workspace,
                Some(task),
            ))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let items: Vec<mecha_core::outbox::OutboxItem> = std::fs::read_dir(home.join("outbox"))
            .unwrap()
            .map(|entry| {
                serde_json::from_slice(&std::fs::read(entry.unwrap().path()).unwrap()).unwrap()
            })
            .collect();
        assert_eq!(items.len(), expected);
        assert!(items
            .iter()
            .all(|item| item.status == "pending" && !item.edited()));
    }
    std::fs::remove_dir_all(temp).unwrap();
}

#[test]
fn harder_pilot_goals_resolve_to_real_fixture_tasks() {
    let m = Manifest::parse(include_str!("../../eval/appraisal-guidance-v2.toml")).unwrap();
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("../../eval/fixtures/appraisal-v2/cases.json")).unwrap();
    let board: serde_json::Value = serde_json::from_str(include_str!(
        "../../eval/fixtures/appraisal-v2/board/board.json"
    ))
    .unwrap();
    assert_eq!(m.tasks.ids.len(), 8);
    assert_eq!(m.tasks.confirmed_goals.len(), 8);
    assert_eq!(m.trials(&m.tasks.ids, "local", "ignored").len(), 48);
    for id in &m.tasks.ids {
        assert_eq!(
            m.tasks.confirmed_goals[id].to_string(),
            format!("task:{id}")
        );
        assert!(cases.as_array().unwrap().iter().any(|c| c["id"] == *id));
        assert!(board["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == *id));
    }
    for arm in m.arms.values() {
        assert_eq!(arm.model.as_deref(), Some("qwen3.6-35b-a3b"));
        assert!(arm.overrides.contains(&"max_turns=16".to_string()));
    }
}
