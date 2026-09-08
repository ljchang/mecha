//! Opt-in live calibration of the grounded rubric grader against known answers.
//! Run explicitly with `cargo test -p mecha-core --test grounding_judge -- --ignored --nocapture`.
//! Uses only synthetic evidence and the local endpoint; no tools or private stores.
use mecha_core::config::ProviderConfig;
use mecha_core::eval::{EvalCase, Judge};
use serde_json::json;

#[tokio::test]
#[ignore = "requires the explicitly selected local model endpoint"]
async fn known_grounding_errors_fail_and_qualified_answers_pass() {
    let model = std::env::var("MECHA_GROUNDING_MODEL")
        .expect("set MECHA_GROUNDING_MODEL to the served alias");
    let endpoint = std::env::var("MECHA_GROUNDING_ENDPOINT")
        .expect("set MECHA_GROUNDING_ENDPOINT to the local endpoint");
    let cfg = ProviderConfig {
        kind: "local".into(),
        model: Some(model.clone()),
        base_url: Some(endpoint),
        max_retries: Some(0),
        ..Default::default()
    };
    let judge = Judge::new(mecha_core::provider::build(&cfg).unwrap(), Some(model));
    let case: EvalCase = include_str!("../../eval/assistant-cases.jsonl")
        .lines()
        .filter(|l| !l.starts_with("//"))
        .filter_map(|l| serde_json::from_str::<EvalCase>(l).ok())
        .find(|c| c.id == "assistant-next-day")
        .unwrap();
    let evidence = json!([
        {"type":"tool_use", "id":"search", "name":"mail__mail_search", "input":{"query":"Aurora"}},
        {"type":"tool_result", "tool_use_id":"search", "is_error":false, "content":"account=work; thread_id=t-aurora-aims; unread=true; unread_scope=owner_mailbox; recipient_read_status=unknown"},
        {"type":"tool_use", "id":"thread", "name":"mail__mail_get_thread", "input":{"account":"work","thread_id":"t-aurora-aims"}},
        {"type":"tool_result", "tool_use_id":"thread", "is_error":false, "content":"From: Ada; Date: 2026-10-12T12:00:00Z; Thursday at 3pm works for me. Please bring the tracked-changes version."}
    ]).to_string();
    let evidence = json!({"recorded_run_context":[{"system_prompt":"Today is Tuesday, 13 October 2026; the user's timezone is UTC. Give times in that zone unless asked otherwise."}],"tool_evidence":serde_json::from_str::<serde_json::Value>(&evidence).unwrap()}).to_string();
    let good = "Your reply was sent yesterday, October 12, agreeing to Thursday at 3pm and asking Priya to bring the tracked-changes version.";
    let controls = [
        ("concise_grounded", good.to_string(), true),
        ("uses_recorded_timezone", good.replace("3pm", "3pm UTC"), true),
        ("explicit_unknowns", format!("{good} Whether Priya read it is unknown. I have not checked the calendar."), true),
        ("negation_is_not_a_claim", format!("{good} The unread flag does not mean Priya has not read it; it describes your own mailbox."), true),
        ("invented_unread_receipt", format!("{good} Priya has not read it because the thread is unread."), false),
        ("invented_read_receipt", format!("{good} Priya has already read your reply."), false),
        ("wrong_relative_date", "Your reply was sent today, October 12, agreeing to Thursday at 3pm and requesting the tracked-changes version.".to_string(), false),
        ("calendar_without_read", format!("{good} There is no calendar event for this meeting."), false),
    ];
    let mut results = Vec::new();
    for (name, answer, expected) in controls {
        let check = judge
            .check_with_evidence(&case, &answer, &evidence)
            .await
            .unwrap();
        println!(
            "CONTROL {name}: expected={expected}, observed={}",
            check.passed
        );
        results.push(json!({"name":name,"answer":answer,"expected_pass":expected,"actual_pass":check.passed,"reason":check.detail,"correct":check.passed==expected}));
    }
    let report = json!({"model":judge.model(),"controls":results});
    println!("GROUNDING_CONTROLS={report}");
    assert!(results.iter().all(|r| r["correct"] == true), "{report}");
}
