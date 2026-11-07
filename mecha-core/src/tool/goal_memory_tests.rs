use super::{goal_context::GoalContext, goal_memory::GoalMemory, Tool, ToolCtx};
use crate::{
    clock::TestClock,
    memory::{
        context::Situation, Access, Bound, Capture, Interval, Memory, RelationInput, SourceKey,
        Span, Target,
    },
};
use serde_json::{json, Value};
use std::sync::Arc;

#[tokio::test]
async fn reused_memory_tool_refreshes_validity_without_advancing_knowledge() {
    let dir = std::env::temp_dir().join(format!("goal-memory-clock-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("fixture.memory-prototype.sqlite");
    let mut db = Memory::create(&path, "fixture").unwrap();
    let capture = |id: &str| Capture {
        source: SourceKey {
            connector: "fixture".into(),
            account: "synthetic".into(),
            id: id.into(),
        },
        text: "launch choice is Ilex".into(),
        occurred_at: None,
        scopes: ["chat".into()].into(),
        transcript: None,
    };
    let source = db.capture(capture("original")).unwrap();
    let entity = db.entity("Ilex", Default::default()).unwrap();
    let relation = db
        .relate(RelationInput {
            predicate: "choice".into(),
            subject: entity,
            object: Target::Literal("Ilex".into()),
            support: Span {
                evidence: source.clone(),
                start: 0,
                end: 20,
            },
            consulted: vec![],
            text: "choice is Ilex".into(),
            valid: Interval {
                from: Bound::Known("2026-06-01T00:00:00Z".into()),
                to: Bound::Known("2026-07-01T00:00:00Z".into()),
            },
            polarity: 1,
        })
        .unwrap();
    let snapshot = db.snapshot().unwrap();
    let clock = Arc::new(TestClock::at("2026-06-30T23:59:59Z"));
    let tool = GoalMemory::open_with_clock(
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
        clock.clone(),
    )
    .unwrap();
    let ctx = ToolCtx::default();
    let input = json!({"memory":{"query":"launch"}});
    let first = tool.call(input.clone(), &ctx).await.unwrap();
    assert!(!first.is_error && first.external);
    let first: Value = serde_json::from_str(&first.content).unwrap();
    assert_eq!(first["memory"]["known_at"], json!(snapshot));
    assert!(first["memory"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .any(|hit| hit["reference"] == json!(relation)));

    let later = db.capture(capture("after-open")).unwrap();
    assert!(db.snapshot().unwrap().sequence > snapshot.sequence);
    clock.advance(chrono::Duration::seconds(1));
    let second = tool.call(input.clone(), &ctx).await.unwrap();
    assert!(!second.is_error && second.external);
    let second: Value = serde_json::from_str(&second.content).unwrap();
    let records = second["memory"]["records"].as_array().unwrap();
    assert!(
        !records
            .iter()
            .any(|hit| hit["reference"] == json!(relation)),
        "the same tool must exclude a relation at its exclusive validity end"
    );
    let at: chrono::DateTime<chrono::Utc> = second["memory"]["valid_at"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        at,
        "2026-07-01T00:00:00Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap()
    );
    assert_eq!(second["memory"]["known_at"], first["memory"]["known_at"]);
    assert!(records.iter().any(|hit| hit["reference"] == json!(source)));
    assert!(!records.iter().any(|hit| hit["reference"] == json!(later)));

    let mut restricted = capture("original");
    restricted.scopes = ["restricted".into()].into();
    db.capture(restricted).unwrap();
    let empty = tool.call(input, &ctx).await.unwrap();
    assert!(!empty.is_error && !empty.external);
    let empty: Value = serde_json::from_str(&empty.content).unwrap();
    assert_eq!(empty["memory"]["records"], json!([]));
    assert_eq!(empty["memory"]["known_at"], first["memory"]["known_at"]);

    let refused = tool
        .call(
            json!({"memory":{"query":"launch","valid_at":"2026-06-01T00:00:00Z"}}),
            &ctx,
        )
        .await
        .unwrap();
    assert!(refused.is_error && !refused.external);
    drop(tool);
    drop(db);
    std::fs::remove_dir_all(dir).unwrap();
}

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
