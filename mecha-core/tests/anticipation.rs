//! Exercise actual store records through prediction, release and retrospective appraisal.
use mecha_core::{
    agent::Taint,
    anticipation::{Commitment, Evidence, Kind, OutcomeInput, Resolution, Verdict, Verification},
    appraisal::{self, Affect, SessionRecords},
    goal::GoalRef,
    outbox::{OutboxItem, OutboxKind, OutboxStore, Provenance},
    session::RunStats,
};
use serde_json::json;

struct Fixture {
    store: OutboxStore,
    root: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("mecha-anticipation-{}", uuid::Uuid::new_v4()));
        Self {
            store: OutboxStore::open(&root).unwrap(),
            root,
        }
    }
    fn draft(&self) -> OutboxItem {
        self.store
            .stage(
                "mail_send",
                OutboxKind::Message,
                json!({"to":"test@example.invalid","body":"Meeting at 10"}),
                Taint::default(),
                Provenance {
                    session_id: Some("test-session".into()),
                    ..Provenance::default()
                },
            )
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn evidence() -> Evidence {
    Evidence {
        goal: Some(GoalRef::Task("meeting".into())),
        commitment: Some(Commitment {
            beneficiary: "attendees".into(),
            expectation: "send the confirmed time".into(),
            consequence: "attendees miss the meeting".into(),
        }),
        expected_outcome: Some("accurate invitation".into()),
        check_available: true,
        check_cost_secs: Some(10),
        time_available_secs: Some(3600),
        ..Evidence::default()
    }
}
fn feedback(p: &str, verdict: Verdict) -> OutcomeInput {
    OutcomeInput {
        prediction_id: p.into(),
        verdict,
        evidence: "Owner checked the delivered message and calendar".into(),
        attributable_to_mecha: true,
        supersedes: None,
    }
}
fn appraise(item: &OutboxItem) -> appraisal::Appraisal {
    appraisal::of_session(
        "test-session",
        &RunStats::default(),
        &[],
        &[],
        SessionRecords {
            drafts: &[item],
            ..SessionRecords::default()
        },
        Some(Taint::default()),
        "2026-09-09T00:00:00Z".into(),
    )
}

#[test]
fn guided_check_blocks_release_but_not_editing_or_rejection() {
    let f = Fixture::new();
    let _lock = f.store.lock().unwrap();
    let draft = f.draft();
    assert_eq!(draft.predictions.len(), 1);
    let item = f.store.anticipate(&draft.id, evidence(), true).unwrap();
    let original = item.predictions.last().unwrap().known().unwrap().clone();
    assert!(original.assessment.kinds.contains(&Kind::Guilt));
    assert!(f.store.begin_delivery(&item.id).is_err());
    assert!(f.store.item(&item.id).unwrap().delivery_attempts.is_empty());
    let edited = f
        .store
        .update_args(
            &item.id,
            json!({"to":"test@example.invalid","body":"Meeting at 11"}),
        )
        .unwrap();
    assert_eq!(edited.prediction_resolution(&original), Resolution::Changed);
    assert!(f
        .store
        .begin_delivery(&item.id)
        .unwrap_err()
        .to_string()
        .contains("draft changed"));
    f.store.resolve(&item.id, "rejected", None).unwrap();
}

#[test]
fn a_reassessment_is_not_a_success_or_failure_of_the_original_forecast() {
    let f = Fixture::new();
    let _lock = f.store.lock().unwrap();
    let draft = f.draft();
    let before = f.store.anticipate(&draft.id, evidence(), true).unwrap();
    let original = before.predictions.last().unwrap().known().unwrap().clone();
    let mut checked = evidence();
    checked.verification = Verification::Passed;
    checked.verification_evidence = Some("calendar entry checked".into());
    let after = f.store.anticipate(&draft.id, checked, true).unwrap();
    let current = after.predictions.last().unwrap().known().unwrap().clone();
    assert_eq!(
        after.prediction_resolution(&original),
        Resolution::Reassessed
    );
    let delivering = f.store.begin_delivery(&draft.id).unwrap();
    assert_eq!(
        delivering.prediction_resolution(&current),
        Resolution::DeliveryUnknown
    );
    assert!(f
        .store
        .record_outcome(&draft.id, feedback(&current.id, Verdict::NoIssue))
        .is_err());
    let sent = f
        .store
        .resolve_with_output(&draft.id, "sent", None, Some("receipt".into()))
        .unwrap();
    assert_eq!(
        sent.delivery_attempts[0].prediction_id.as_deref(),
        Some(current.id.as_str())
    );
    assert_eq!(
        sent.prediction_resolution(&current),
        Resolution::AwaitingFeedback
    );
    assert!(f
        .store
        .record_outcome(&draft.id, feedback(&original.id, Verdict::NoIssue))
        .is_err());
    let judged = f
        .store
        .record_outcome(&draft.id, feedback(&current.id, Verdict::NoIssue))
        .unwrap();
    assert_eq!(judged.prediction_resolution(&current), Resolution::Observed);
    assert_eq!(
        judged.predictions[1].known().unwrap(),
        &original,
        "prediction history is immutable"
    );
}

#[test]
fn delivered_error_and_attributable_impact_have_real_appraisal_producers() {
    let f = Fixture::new();
    let _lock = f.store.lock().unwrap();
    let draft = f.draft();
    let pending = f.store.anticipate(&draft.id, evidence(), false).unwrap();
    let pid = pending
        .predictions
        .last()
        .unwrap()
        .known()
        .unwrap()
        .id
        .clone();
    assert!(f
        .store
        .record_outcome(&draft.id, feedback(&pid, Verdict::ErrorExposed))
        .is_err());
    assert_eq!(appraise(&pending).label, Affect::Neutral);
    f.store.begin_delivery(&draft.id).unwrap();
    f.store.resolve(&draft.id, "sent", None).unwrap();
    let error = f
        .store
        .record_outcome(&draft.id, feedback(&pid, Verdict::ErrorExposed))
        .unwrap();
    assert_eq!(appraise(&error).label, Affect::Embarrassment);
    assert_eq!(
        appraise(&error).errors.len(),
        1,
        "one incident replaces the initial drafting verdict"
    );
    assert!(
        f.store
            .record_outcome(&draft.id, feedback(&pid, Verdict::Harm))
            .is_err(),
        "duplicate reports cannot accumulate"
    );
    let mut impact = feedback(&pid, Verdict::Harm);
    impact.supersedes = Some(error.outcomes[0].known().unwrap().id.clone());
    let harmed = f.store.record_outcome(&draft.id, impact).unwrap();
    let a = appraise(&harmed);
    assert_eq!(a.label, Affect::Guilt);
    assert_eq!(a.errors.len(), 1);
    assert_eq!(a.errors[0].goal, Some(GoalRef::Task("meeting".into())));
    let mut withdraw = feedback(&pid, Verdict::Withdrawn);
    withdraw.supersedes = Some(harmed.outcomes[1].known().unwrap().id.clone());
    let withdrawn = f.store.record_outcome(&draft.id, withdraw).unwrap();
    assert_eq!(appraise(&withdrawn).label, Affect::Neutral);
    assert_eq!(withdrawn.outcomes.len(), 3);
    assert_eq!(
        withdrawn.prediction_resolution(withdrawn.predictions.last().unwrap().known().unwrap()),
        Resolution::AwaitingFeedback
    );
    let reopened = OutboxStore::open(&f.root).unwrap().item(&draft.id).unwrap();
    assert_eq!(reopened.outcomes, withdrawn.outcomes);
}

#[test]
fn harm_needs_a_commitment_and_an_owner_rewrite_cannot_be_attributed_to_mecha() {
    let f = Fixture::new();
    let _lock = f.store.lock().unwrap();
    let draft = f.draft();
    f.store
        .update_args(&draft.id, json!({"body":"owner's different words"}))
        .unwrap();
    let pending = f
        .store
        .anticipate(&draft.id, Evidence::default(), false)
        .unwrap();
    let pid = pending
        .predictions
        .last()
        .unwrap()
        .known()
        .unwrap()
        .id
        .clone();
    f.store.begin_delivery(&draft.id).unwrap();
    let sent = f.store.resolve(&draft.id, "sent", None).unwrap();
    assert_eq!(appraise(&sent).label, Affect::Distress);
    assert!(f
        .store
        .record_outcome(&draft.id, feedback(&pid, Verdict::ErrorExposed))
        .is_err());
    assert!(f
        .store
        .record_outcome(&draft.id, feedback(&pid, Verdict::Harm))
        .is_err());
}

#[test]
fn future_guidance_stays_reviewable_and_raw_but_cannot_release() {
    let f = Fixture::new();
    let _lock = f.store.lock().unwrap();
    for (pointer, unknown) in [
        (
            "/predictions/0/assessment/response",
            json!("future_response"),
        ),
        ("/predictions/0/assessment/kinds", json!(["future_kind"])),
        (
            "/predictions/0/evidence/verification",
            json!("future_verification"),
        ),
        ("/predictions/0/source", json!("future_source")),
        ("/predictions/0/evidence", json!({"future_field":true})),
    ] {
        let draft = f.draft();
        let mut value = serde_json::to_value(&draft).unwrap();
        *value.pointer_mut(pointer).unwrap() = unknown;
        let raw = value["predictions"][0].clone();
        std::fs::write(f.root.join(format!("{}.json", draft.id)), value.to_string()).unwrap();
        let item = f.store.item(&draft.id).unwrap();
        assert!(item.predictions[0].known().is_none());
        assert!(f.store.items().unwrap().iter().any(|i| i.id == draft.id));
        assert_eq!(
            item.anticipation_readout()["predictions"][0]["resolution"],
            "unsupported"
        );
        assert!(f.store.begin_delivery(&draft.id).is_err());
        let edited = f
            .store
            .update_args(&draft.id, json!({"body":"revised"}))
            .unwrap();
        assert_eq!(
            serde_json::to_value(&edited).unwrap()["predictions"][0],
            raw
        );
        let rejected = f.store.resolve(&draft.id, "rejected", None).unwrap();
        assert_eq!(
            serde_json::to_value(&rejected).unwrap()["predictions"][0],
            raw
        );
    }
    let draft = f.draft();
    let mut value = serde_json::to_value(&draft).unwrap();
    value.as_object_mut().unwrap().remove("predictions");
    value.as_object_mut().unwrap().remove("outcomes");
    let old: OutboxItem = serde_json::from_value(value).unwrap();
    old.ensure_prediction_ready().unwrap();
    assert!(old.predictions.is_empty());
}

#[test]
fn an_unknown_outcome_is_preserved_and_never_falls_back_to_a_positive_verdict() {
    let f = Fixture::new();
    let draft = f.draft();
    f.store.begin_delivery(&draft.id).unwrap();
    let sent = f.store.resolve(&draft.id, "sent", None).unwrap();
    let mut value = serde_json::to_value(&sent).unwrap();
    value["outcomes"] = json!([{"id":"future", "observation":{"verdict":"future_verdict"}}]);
    let raw = value["outcomes"].clone();
    std::fs::write(f.root.join(format!("{}.json", draft.id)), value.to_string()).unwrap();
    let read = f.store.item(&draft.id).unwrap();
    assert_eq!(serde_json::to_value(&read).unwrap()["outcomes"], raw);
    let a = appraise(&read);
    assert!(a.partial);
    assert!(a.errors.is_empty());
    assert_eq!(read.anticipation_readout()["outcomes_supported"], false);
    assert!(read.ensure_prediction_ready().is_err());
    let pid = read.predictions[0].known().unwrap().id.clone();
    assert!(f
        .store
        .record_outcome(&draft.id, feedback(&pid, Verdict::NoIssue))
        .is_err());
}

#[test]
fn an_expired_guided_commitment_cannot_use_an_old_pass_to_release() {
    let f = Fixture::new();
    let draft = f.draft();
    let mut checked = evidence();
    checked.verification = Verification::Passed;
    checked.verification_evidence = Some("receipt".into());
    let mut item = f.store.anticipate(&draft.id, checked, true).unwrap();
    item.predictions
        .last_mut()
        .unwrap()
        .known_mut()
        .unwrap()
        .created_at = "2020-01-01T00:00:00Z".into();
    assert!(item.ensure_prediction_ready().is_err());
}

#[tokio::test]
async fn owner_commitment_changes_planning_guidance_and_stays_local_to_the_confirmed_goal() {
    use mecha_core::tool::{todo::TodoTool, GoalTrack, Tool, ToolCtx};
    use std::sync::{Arc, Mutex};
    let f = Fixture::new();
    let tool = TodoTool::new();
    let feedback = Arc::new(Mutex::new(mecha_core::planning::Feedback::default()));
    let ctx = ToolCtx {
        workspace: f.root.clone(),
        goal_track: Some(Arc::new(GoalTrack::carrying(Some(GoalRef::Task(
            "meeting".into(),
        ))))),
        appraisal_evidence: Some(mecha_core::anticipation::BoundEvidence::new(evidence()).unwrap()),
        goal_guidance: true,
        plan_feedback: Some(feedback.clone()),
        ..ToolCtx::default()
    };
    let result = tool.call(json!({"serves":"task:meeting","items":[{"content":"compose invitation","status":"in_progress"}]}), &ctx).await.unwrap();
    assert!(!result.is_error);
    assert!(
        result.content.contains("establish evidence"),
        "{}",
        result.content
    );
    assert!(
        !result.content.contains("attendees miss the meeting"),
        "owner evidence prose must not enter tool results"
    );
    {
        let recorded = feedback.lock().unwrap();
        let decision = recorded.decisions.last().unwrap();
        assert_eq!(decision.action, mecha_core::planning::Action::Verify);
        assert!(decision
            .anticipation
            .as_ref()
            .unwrap()
            .kinds
            .contains(&Kind::Guilt));
        assert!(decision
            .anticipation_evidence
            .as_ref()
            .unwrap()
            .commitment
            .is_some());
    }
    tool.call(json!({"serves":"task:another","items":[{"content":"unrelated work","status":"in_progress"}]}), &ctx).await.unwrap();
    let recorded = feedback.lock().unwrap();
    let drifted = recorded.decisions.last().unwrap();
    assert_eq!(drifted.action, mecha_core::planning::Action::ClarifyGoal);
    assert!(!drifted
        .anticipation
        .as_ref()
        .unwrap()
        .kinds
        .contains(&Kind::Guilt));
}

#[test]
fn an_owner_check_of_context_never_certifies_new_message_content() {
    let mut e = evidence();
    e.verification = Verification::Passed;
    e.verification_evidence = Some("checked the source calendar".into());
    let b = mecha_core::anticipation::BoundEvidence::new(e).unwrap();
    let goal = GoalRef::Task("meeting".into());
    assert_eq!(
        b.for_goal(Some(&goal)).unwrap().verification,
        Verification::Passed
    );
    let draft = b.for_draft(Some(&goal)).unwrap();
    assert_eq!(draft.verification, Verification::Unknown);
    assert!(draft.verification_evidence.is_none());
    assert!(draft.commitment.is_some());
    assert!(b
        .for_goal(Some(&GoalRef::Task("different".into())))
        .is_none());
}
