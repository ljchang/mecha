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
