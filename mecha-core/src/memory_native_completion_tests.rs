use super::*;

fn shape() -> NativeCompletionConfig {
    NativeCompletionRequest {
        rows: 1,
        keys: vec!["route".into()],
    }
    .into_config()
    .unwrap()
}

#[test]
fn completion_shape_rejects_invalid_and_duplicate_identifier_keys() {
    for rows in [0, 33, usize::MAX] {
        assert!(NativeCompletionRequest {
            rows,
            keys: vec!["route".into()]
        }
        .into_config()
        .is_err());
    }
    for keys in [
        vec![],
        vec!["a".into(); 17],
        vec!["a".into(), "a".into()],
        vec!["".into()],
        vec!["1key".into()],
        vec!["a-b".into()],
        vec!["non ascii é".into()],
        vec!["a".repeat(65)],
    ] {
        assert!(NativeCompletionRequest { rows: 1, keys }
            .into_config()
            .is_err());
    }
    assert!(serde_json::from_str::<NativeCompletionRequest>(
        r#"{"rows":1,"keys":["route"],"expected_rows":[]}"#
    )
    .is_err());
    assert!(serde_json::from_str::<NativeCompletionRequest>(
        r#"{"rows":1,"rows":2,"keys":["route"]}"#
    )
    .is_err());
    let first = NativeCompletionRequest {
        rows: 32,
        keys: vec!["_first".into(), "second2".into()],
    }
    .into_config()
    .unwrap();
    let second = NativeCompletionRequest {
        rows: 32,
        keys: vec!["second2".into(), "_first".into()],
    }
    .into_config()
    .unwrap();
    assert_eq!(first.shape_digest, second.shape_digest);
}

#[tokio::test]
async fn completion_artifact_checks_only_public_shape_and_refuses_unsafe_reads() -> Result<()> {
    let root = std::env::temp_dir().join(format!("completion-shape-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root)?;
    let root = root.canonicalize()?;
    let path = root.join("output.json");
    let completion = NativeCompletion::new(shape());
    let observer = completion.observer(path.clone(), None);
    let missing = observer.observe().await;
    assert_eq!(missing.readiness, CompletionReadiness::Incomplete);
    assert_eq!(
        missing.feedback.unwrap().artifact,
        Some(CompletionArtifactIssue::Missing)
    );
    for text in [
        "not json",
        "[]",
        r#"[{"route":"a"},{"route":"b"}]"#,
        r#"[{"route":"a","route":"b"}]"#,
        r#"[{"route":1}]"#,
        r#"[{"other":"a"}]"#,
        r#"[{"route":"a","extra":"b"}]"#,
    ] {
        std::fs::write(&path, text)?;
        let observation = observer.observe().await;
        assert_eq!(
            observation.readiness,
            CompletionReadiness::Incomplete,
            "{text}"
        );
        assert_eq!(
            observation.feedback.unwrap().artifact,
            Some(CompletionArtifactIssue::Malformed)
        );
    }
    for text in [r#"[{"route":"wrong"}]"#, r#"[{"route":"unknown"}]"#] {
        std::fs::write(&path, text)?;
        assert_eq!(
            observer.observe().await.readiness,
            CompletionReadiness::Ready
        );
    }
    std::fs::write(&path, vec![b'x'; 1_048_577])?;
    assert_eq!(
        observer.observe().await.readiness,
        CompletionReadiness::Unavailable
    );
    std::fs::remove_file(&path)?;
    std::fs::create_dir(&path)?;
    assert_eq!(
        observer.observe().await.readiness,
        CompletionReadiness::Unavailable
    );
    std::fs::remove_dir(&path)?;
    let target = root.join("other.json");
    std::fs::write(&target, r#"[{"route":"unknown"}]"#)?;
    std::os::unix::fs::symlink(&target, &path)?;
    assert_eq!(
        observer.observe().await.readiness,
        CompletionReadiness::Unavailable
    );
    assert!(
        completion.snapshot().report.first.is_none(),
        "observe must never publish reports"
    );
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn completion_consumed_recall_states_allow_only_artifact_repair_and_never_ready() -> Result<()> {
    use crate::tool::goal_memory::{
        NativeRecallConfig, NativeRecallEmbeddingCost, NativeRecallMode,
    };
    for status in [
        NativeRecallStatus::Running,
        NativeRecallStatus::Rejected,
        NativeRecallStatus::Fallback,
        NativeRecallStatus::Failed,
        NativeRecallStatus::Cancelled,
        NativeRecallStatus::TimedOut,
        NativeRecallStatus::Interrupted,
    ] {
        let report = NativeRecallReport {
            identity: NativeRecallConfig::lexical("authored".into())?.identity(),
            entered_calls: 1,
            admitted_calls: 1,
            rejected_calls: 0,
            status,
            effective_mode: Some(NativeRecallMode::Lexical),
            fallback: None,
            delivered: vec![],
            elapsed_micros: 0,
            embedding: NativeRecallEmbeddingCost::default(),
        };
        let ready_artifact = readiness(Some(None), Some(&report));
        assert_eq!(ready_artifact.readiness, CompletionReadiness::Incomplete);
        assert!(
            ready_artifact.feedback.is_none(),
            "consumed recall alone cannot invite another attempt"
        );
        let repair = readiness(
            Some(Some(CompletionArtifactIssue::Malformed)),
            Some(&report),
        );
        let feedback = repair.feedback.unwrap();
        assert!(!feedback.required_read_missing && feedback.do_not_repeat_read);
        assert_eq!(feedback.artifact, Some(CompletionArtifactIssue::Malformed));
        assert_eq!(
            readiness(None, Some(&report)).readiness,
            CompletionReadiness::Unavailable
        );
    }
    Ok(())
}
