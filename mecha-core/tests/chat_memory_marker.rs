use mecha_core::session::{Record, Session, SessionKind, SessionMeta};
#[test]
fn recognized_memory_marker_cannot_silently_resume_as_ordinary_chat() {
    let path = std::env::temp_dir().join(format!("memory-marker-{}.jsonl", uuid::Uuid::new_v4()));
    let meta = SessionMeta {
        id: "authored".into(),
        created_at: chrono::Utc::now(),
        provider: "test".into(),
        model: "test".into(),
        workspace: std::env::temp_dir(),
        title: None,
        kind: Some(SessionKind::Chat),
    };
    let mut text = serde_json::to_string(&Record::Meta(meta)).unwrap();
    text.push_str("\n{\"record\":\"chat_memory\",\"marker\":{\"protocol\":\"ordinary-chat-memory-v1\",\"conversation_id\":\"authored\",\"authority\":\"authored-authority\",\"attachment\":\"");
    text.push_str(&"a".repeat(64));
    text.push_str("\"}}\n");
    std::fs::write(&path, text).unwrap();
    assert!(
        Session::load(&path).is_err(),
        "a memory session requires authenticated attachment"
    );
    std::fs::remove_file(path).unwrap();
}
#[test]
fn malformed_memory_marker_never_enters_lenient_recovery() {
    let text = "{\"record\":\"meta\",\"id\":\"authored\",\"created_at\":\"2026-01-01T00:00:00Z\",\"provider\":\"test\",\"model\":\"test\",\"workspace\":\"/tmp\"}\n{\"record\":\"chat_memory\",\"marker\":null}\n";
    assert!(Session::parse(std::path::Path::new("authored"), text).is_err());
}

#[test]
fn marked_transcript_cannot_leniently_drop_an_unknown_message_block() {
    let marker = serde_json::json!({"record":"chat_memory","marker":{"protocol":"ordinary-chat-memory-v1","conversation_id":"authored","authority":"authority","attachment":"a".repeat(64)}});
    let header = serde_json::json!({"record":"meta","id":"authored","created_at":"2026-01-01T00:00:00Z","provider":"test","model":"test","workspace":"/tmp"});
    let message = serde_json::json!({"record":"message","role":"user","content":[{"type":"text","text":"hello"},{"type":"future_block"}]});
    let text = format!("{header}\n{marker}\n{message}\n");
    assert!(Session::parse(std::path::Path::new("authored"), &text).is_err());
}

#[test]
fn duplicate_marker_cannot_rebind_a_conversation() {
    let marker = serde_json::json!({"record":"chat_memory","marker":{"protocol":"ordinary-chat-memory-v1","conversation_id":"authored","authority":"authority","attachment":"a".repeat(64)}});
    let header = serde_json::json!({"record":"meta","id":"authored","created_at":"2026-01-01T00:00:00Z","provider":"test","model":"test","workspace":"/tmp"});
    let text = format!("{header}\n{marker}\n{marker}\n");
    assert!(Session::parse(std::path::Path::new("authored"), &text).is_err());
}

#[test]
fn graph_memory_marker_is_recognized_but_ordinary_loader_refuses() {
    let marker = serde_json::json!({"record":"chat_memory","marker":{"protocol":"ordinary-chat-memory-v2","conversation_id":"authored","authority":"authority","attachment":"a".repeat(64)}});
    let header = serde_json::json!({"record":"meta","id":"authored","created_at":"2026-01-01T00:00:00Z","provider":"test","model":"test","workspace":"/tmp"});
    let text = format!("{header}\n{marker}\n");
    let path = std::env::temp_dir().join(format!("graph-marker-{}.jsonl", uuid::Uuid::new_v4()));
    std::fs::write(&path, &text).unwrap();
    let transcript = Session::parse(&path, &text)
        .expect("recognized V2 needs its authenticated attachment, not a parser failure");
    assert_eq!(
        transcript.chat_memory.unwrap().protocol,
        "ordinary-chat-memory-v2"
    );
    assert!(Session::load(&path).is_err());
    std::fs::remove_file(path).unwrap();
}

fn authored_web_memory_transcript() -> String {
    let header = serde_json::json!({"record":"meta","id":"authored-web","created_at":"2026-01-01T00:00:00Z","provider":"test","model":"test","workspace":"/tmp","kind":"web"});
    let marker = serde_json::json!({"record":"chat_memory","marker":{"protocol":"ordinary-chat-memory-v3","conversation_id":"authored-web","authority":"authored-authority","attachment":"a".repeat(64)}});
    format!("{header}\n{marker}\n")
}

#[test]
fn web_memory_v3_marker_is_readable_without_authorizing_ordinary_resume() {
    let text = authored_web_memory_transcript();
    let path = std::env::temp_dir().join(format!("web-marker-{}.jsonl", uuid::Uuid::new_v4()));
    let transcript = Session::parse(&path, &text)
        .expect("V3 must remain readable even when executable memory attachment is unavailable");
    assert_eq!(transcript.meta.kind, Some(SessionKind::Web));
    assert_eq!(
        transcript.chat_memory.unwrap().protocol,
        "ordinary-chat-memory-v3"
    );
    std::fs::write(&path, text).unwrap();
    let ordinary_resume = Session::load(&path);
    std::fs::remove_file(path).unwrap();
    assert!(
        ordinary_resume.is_err(),
        "V3 requires authenticated web attachment"
    );
}

#[test]
fn web_memory_v3_marker_never_recovers_duplicate_or_unknown_tail_records() {
    let text = authored_web_memory_transcript();
    let marker = text.lines().nth(1).unwrap();
    for suffix in [
        format!("{marker}\n"),
        "{\"record\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"unknown_web_block\"}]}\n".into(),
    ] {
        assert!(Session::parse(std::path::Path::new("authored-web"), &format!("{text}{suffix}")).is_err());
    }
    for invalid in [
        text.replace("ordinary-chat-memory-v3", "ordinary-chat-memory-v999"),
        text.replace(
            "\"protocol\":",
            "\"protocol\":\"ordinary-chat-memory-v1\",\"protocol\":",
        ),
        text.replace(
            "\"attachment\":",
            "\"unexpected_ingress\":true,\"attachment\":",
        ),
    ] {
        assert!(Session::parse(std::path::Path::new("authored-web"), &invalid).is_err());
    }
}

#[test]
fn task_start_reservation_is_readable_and_only_finalizes_to_matching_v4() {
    let header = serde_json::json!({"record":"meta","id":"authored-task","created_at":"2026-01-01T00:00:00Z","provider":"test","model":"test","workspace":"/tmp","kind":"chat"});
    let start = serde_json::json!({"record":"chat_memory","marker":{"protocol":"ordinary-chat-task-start-v1","conversation_id":"authored-task","authority":"authored-authority","attachment":"a".repeat(64)}});
    let mut final_marker = start.clone();
    final_marker["marker"]["protocol"] = "ordinary-chat-memory-v4".into();
    final_marker["marker"]["attachment"] = "b".repeat(64).into();
    let path = std::env::temp_dir().join(format!("task-marker-{}.jsonl", uuid::Uuid::new_v4()));
    let reserved = format!("{header}\n{start}\n");
    assert!(
        Session::parse(&path, &reserved).is_ok(),
        "reservation must be inspectable"
    );
    std::fs::write(&path, &reserved).unwrap();
    assert!(Session::load(&path).is_err());
    let complete = format!("{reserved}{final_marker}\n");
    assert_eq!(
        Session::parse(&path, &complete)
            .unwrap()
            .chat_memory
            .unwrap()
            .protocol,
        "ordinary-chat-memory-v4"
    );
    assert!(Session::parse(&path, &format!("{complete}{final_marker}\n")).is_err());
    final_marker["marker"]["authority"] = "other".into();
    assert!(Session::parse(&path, &format!("{reserved}{final_marker}\n")).is_err());
    std::fs::remove_file(path).unwrap();
}
