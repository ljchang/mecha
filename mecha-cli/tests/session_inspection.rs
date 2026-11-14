//! Owner inspection reads memory-marked transcripts without granting resume.
//! These actual CLI tests also run without the memory-prototype feature.

use mecha_core::message::{Block, Message};
use mecha_core::session::{ChatMemoryMarker, Record, Session, SessionKind, SessionMeta};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output};

struct Fixture {
    home: PathBuf,
    session: Session,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

impl Fixture {
    fn new(protocol: &str) -> Self {
        let home = std::env::temp_dir().join(format!("mecha-inspection-{}", Session::new_id()));
        std::fs::create_dir_all(&home).unwrap();
        let home = home.canonicalize().unwrap();
        let session = Session::create(
            &home.join("sessions"),
            SessionMeta {
                id: Session::new_id(),
                created_at: chrono::Utc::now(),
                provider: "fixture-provider".into(),
                model: "fixture-model".into(),
                workspace: home.join("workspace"),
                title: None,
                kind: Some(SessionKind::Chat),
            },
        )
        .unwrap();
        session
            .append(&Record::ChatMemory {
                marker: ChatMemoryMarker {
                    protocol: protocol.into(),
                    conversation_id: session.meta.id.clone(),
                    authority: "fixture-authority".into(),
                    attachment: "a".repeat(64),
                },
            })
            .unwrap();
        session
            .append_messages(&[
                Message::user("Inspect the garden."),
                Message::assistant(vec![Block::text("The north bed needs water.")]),
            ])
            .unwrap();
        Self { home, session }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(args)
            .current_dir(&self.home)
            .env_clear()
            .env("HOME", self.home.join("user"))
            .env("MECHA_HOME", &self.home)
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap()
    }
}

#[test]
fn pretty_inspection_displays_memory_marked_transcripts_without_changing_them() {
    for protocol in [
        "ordinary-chat-memory-v1",
        "ordinary-chat-memory-v2",
        "ordinary-chat-memory-v3",
    ] {
        let f = Fixture::new(protocol);
        let before = std::fs::read(&f.session.path).unwrap();
        let pretty = f.run(&["sessions", "show", &f.session.meta.id]);
        assert!(
            pretty.status.success(),
            "{protocol}: {}",
            String::from_utf8_lossy(&pretty.stderr)
        );
        let text = String::from_utf8(pretty.stdout).unwrap();
        assert!(text.contains("fixture-model (fixture-provider)"), "{text}");
        assert!(text.contains("› Inspect the garden."), "{text}");
        assert!(text.contains("The north bed needs water."), "{text}");
        let raw = f.run(&["sessions", "show", &f.session.meta.id, "--json"]);
        assert!(raw.status.success());
        assert_eq!(raw.stdout, before);
        assert_eq!(std::fs::read(&f.session.path).unwrap(), before);
    }
}

#[test]
fn ordinary_chat_still_refuses_a_memory_marked_resume() {
    let f = Fixture::new("ordinary-chat-memory-v1");
    let before = std::fs::read(&f.session.path).unwrap();
    let output = f.run(&["chat", "--resume", &f.session.meta.id]);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("memory-attached chat requires"), "{error}");
    assert_eq!(std::fs::read(&f.session.path).unwrap(), before);
}

#[test]
fn pretty_inspection_refuses_malformed_memory_marked_history() {
    let f = Fixture::new("ordinary-chat-memory-v1");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&f.session.path)
        .unwrap()
        .write_all(b"{truncated\n")
        .unwrap();
    let before = std::fs::read(&f.session.path).unwrap();
    let output = f.run(&["sessions", "show", &f.session.meta.id]);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("malformed dynamic goal context record"),
        "{error}"
    );
    assert!(
        output.stdout.is_empty(),
        "must not display a partial transcript"
    );
    assert_eq!(std::fs::read(&f.session.path).unwrap(), before);
}
