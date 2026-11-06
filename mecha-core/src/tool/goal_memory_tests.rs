use super::{goal_context::GoalContext, goal_memory::GoalMemory, Tool, ToolCtx};
use crate::memory::{context::Situation, Access, Memory};
use serde_json::{json, Value};

#[tokio::test]
async fn exact_selection_routes_through_existing_planning_tool_with_taint() {
    let dir = std::env::temp_dir().join(format!("goal-memory-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("fixture.memory-prototype.sqlite");
    let mut db = Memory::create(&path, "fixture").unwrap();
    let mut log = json!({"record":"meta","id":"fixture","created_at":"2026-01-01T00:00:00Z"})
        .to_string()
        + "\n";
    for text in [
        "launch launch choose Ilex or Quill pipeline",
        "launch review",
        "launch review again",
        "Ilex cost is seven credits",
    ] {
        log += &(json!({"record":"message","role":"user","content":[{"type":"text","text":text}]})
            .to_string()
            + "\n");
    }
    let refs = db
        .import_session_jsonl(log.as_bytes(), "synthetic", ["chat".into()].into())
        .unwrap()
        .records;
    let tool = GoalMemory::open(
        &path,
        Access {
            domain: "fixture".into(),
            scopes: ["chat".into()].into(),
        },
        "synthetic".into(),
        Situation {
            event: "launch".into(),
            goal: "task:launch".into(),
            needs: None,
        },
    )
    .unwrap();
    assert_eq!(
        tool.input_schema()["properties"]["serves"]["enum"],
        json!(["task:launch"])
    );
    let ctx = ToolCtx::default();
    let original = GoalContext.call(json!({}), &ctx).await.unwrap();
    assert_eq!(
        tool.call(json!({}), &ctx).await.unwrap().content,
        original.content
    );
    assert!(tool.capabilities().private_data && tool.capabilities().untrusted_input);
    let first = tool
        .call(json!({"memory":{"query":"launch"}}), &ctx)
        .await
        .unwrap();
    assert!(first.external && !first.is_error);
    let first: Value = serde_json::from_str(&first.content).unwrap();
    assert!(!first.to_string().contains("seven credits"));
    let input = json!({"memory":{"query":"launch","bindings":[{"source": refs[1],"field":"/content/0/text","quote":"Ilex","query":"cost"}]}});
    let found = tool.call(input.clone(), &ctx).await.unwrap();
    assert!(found.external && !found.is_error);
    assert!(found.content.contains("seven credits"));
    let mut bad = input.clone();
    bad["memory"]["bindings"][0]["quote"] = json!("Ilex pipeline");
    let refused = tool.call(bad, &ctx).await.unwrap();
    assert!(refused.is_error && !refused.external);
    let refused = tool
        .call(
            json!({"serves":"task:other","memory":{"query":"launch"}}),
            &ctx,
        )
        .await
        .unwrap();
    assert!(refused.is_error && !refused.external);
    assert!(refused.content.contains("task:launch") && refused.content.contains("omit serves"));
    db.forget(&crate::memory::transcript_source("synthetic", "fixture", 2))
        .unwrap();
    assert!(tool.call(input, &ctx).await.unwrap().is_error);
    drop(tool);
    drop(db);
    std::fs::remove_dir_all(dir).unwrap();
}
