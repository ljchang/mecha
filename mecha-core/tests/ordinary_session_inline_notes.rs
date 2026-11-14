#![cfg(feature = "memory-prototype")]

use chrono::Utc;
use mecha_core::{
    agent::{StopCause, Taint},
    memory_session::{SessionMemory, SessionMemoryRequest, SessionMemoryStatus},
    message::{Block, Message},
    session::{Record, RunConfig, RunStats, Session, SessionKind, SessionMeta},
};
use std::{collections::BTreeSet, fs, path::PathBuf};

// Exact native text, deliberately pinned independently of the classifier.
const EMPTY_NOTE: &str = "Your previous turn ended without producing anything — the token budget went entirely to reasoning before you began your answer. Do not start the task over and do not re-derive what you already worked out. Either give your answer now, briefly, using what you already know, or make the single next tool call. Keep your reasoning short this turn.";
const OWNER: &str = "Inspect the synthetic amber record";

struct Fixture {
    root: PathBuf,
    request: SessionMemoryRequest,
    workspace: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("ordinary-inline-{}", uuid::Uuid::new_v4()));
        mecha_core::create_private_dir(&root).unwrap();
        for name in ["db", "keys", "work", "sessions"] {
            mecha_core::create_private_dir(&root.join(name)).unwrap();
        }
        let request = SessionMemoryRequest {
            protocol: "ordinary-session-memory-v1".into(),
            db: root.join("db/test.memory-prototype.sqlite"),
            key_file: root.join("keys/key"),
            authority: root.join("authority"),
            domain: "fixture".into(),
            account: "fixture".into(),
            scopes: BTreeSet::new(),
        };
        SessionMemory::initialize(&request).unwrap();
        Self {
            workspace: root.join("work"),
            root,
            request,
        }
    }
    fn memory(&self) -> SessionMemory {
        SessionMemory::open(&self.request, &self.workspace).unwrap()
    }
    fn session(&self) -> Session {
        let meta = SessionMeta {
            id: Session::new_id(),
            created_at: Utc::now(),
            provider: "fixture".into(),
            model: "fixture".into(),
            workspace: self.workspace.clone(),
            title: None,
            kind: Some(SessionKind::Run),
        };
        let session = Session {
            path: self
                .root
                .join("sessions")
                .join(format!("{}.jsonl", meta.id)),
            meta: meta.clone(),
        };
        session.append(&Record::Meta(meta)).unwrap();
        session
            .append(&Record::Config(RunConfig {
                workspace: self.workspace.clone(),
                tools: vec!["fs_read".into(), "goal_context".into()],
                ..Default::default()
            }))
            .unwrap();
        session
            .append(&Record::GoalAnchor {
                goal: Some("charter:1".parse().unwrap()),
            })
            .unwrap();
        session
    }
    fn tail(&self, session: &Session) {
        session
            .append(&Record::Message(Message::assistant(vec![Block::text(
                "The synthetic record is amber.",
            )])))
            .unwrap();
        session.append(&Record::Taint(Taint::default())).unwrap();
        session
            .append(&Record::Outcome(RunStats {
                stop_cause: Some(StopCause::Completed),
                ..Default::default()
            }))
            .unwrap();
    }
    fn payload(&self, run: &str) -> serde_json::Value {
        let seal: serde_json::Value = serde_json::from_slice(
            &fs::read(self.request.authority.join(format!("{run}.seal"))).unwrap(),
        )
        .unwrap();
        seal["payload"].clone()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn tool_round(session: &Session, id: &str, note: Option<&str>) {
    session
        .append(&Record::Message(Message::assistant(vec![Block::ToolUse {
            id: id.into(),
            name: "fs_read".into(),
            input: serde_json::json!({"path":"amber.txt"}),
        }])))
        .unwrap();
    let mut result = Message::tool_results(vec![Block::ToolResult {
        tool_use_id: id.into(),
        content: "synthetic amber".into(),
        is_error: false,
    }]);
    if let Some(note) = note {
        result.content.push(Block::text(note));
    }
    session.append(&Record::Message(result)).unwrap();
}

#[test]
fn one_shot_inline_nudges_and_native_extensions_are_audit_only() {
    for mode in ["inline-owner", "extended-owner", "extended-result"] {
        let f = Fixture::new();
        let memory = f.memory();
        let session = f.session();
        let run = memory
            .begin(&session, OWNER, &"charter:1".parse().unwrap())
            .unwrap();
        let mut owner = Message::user(OWNER);
        if mode == "inline-owner" {
            owner.content.push(Block::text(EMPTY_NOTE));
        }
        session.append(&Record::Message(owner)).unwrap();
        if mode == "extended-result" {
            tool_round(&session, "read1", None);
        }
        if mode != "inline-owner" {
            session
                .append(&Record::Extend {
                    index: if mode == "extended-result" { 2 } else { 0 },
                    blocks: vec![Block::text(EMPTY_NOTE)],
                })
                .unwrap();
        }
        f.tail(&session);
        let report = memory.finish(&run);
        assert_eq!(
            report.status,
            SessionMemoryStatus::Retained,
            "{mode}: {report:?}"
        );
        let payload = f.payload(run.run_id());
        let originals = payload["originals"].as_array().unwrap();
        let owner = originals
            .iter()
            .find(|p| p["kind"] == "UserRequest")
            .unwrap();
        assert_eq!(owner["text"], OWNER);
        assert_eq!(owner["pointer"], "/content/0/text");
        let source = fs::read_to_string(&session.path).unwrap();
        let source_owner: serde_json::Value = serde_json::from_str(
            source
                .lines()
                .nth(owner["line"].as_u64().unwrap() as usize - 1)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(source_owner["content"][0]["text"], OWNER);
        assert!(!serde_json::to_string(&payload)
            .unwrap()
            .contains(EMPTY_NOTE));
        assert!(source.contains(EMPTY_NOTE));
        assert_eq!(memory.retry(run.run_id()).receipt, report.receipt);
        fs::write(
            &session.path,
            source.replacen(
                "Keep your reasoning short this turn.",
                "Changed audit note.",
                1,
            ),
        )
        .unwrap();
        assert_eq!(
            memory.retry(run.run_id()).status,
            SessionMemoryStatus::Unavailable
        );
    }
}

#[test]
fn one_shot_boredom_beside_results_is_not_owner_evidence() {
    let f = Fixture::new();
    let memory = f.memory();
    let session = f.session();
    let run = memory
        .begin(&session, OWNER, &"charter:1".parse().unwrap())
        .unwrap();
    session
        .append(&Record::Message(Message::user(OWNER)))
        .unwrap();
    let notice = mecha_core::boredom::Rung::Change.notice("fs_read", &Default::default());
    tool_round(&session, "read1", None);
    tool_round(&session, "read2", None);
    tool_round(&session, "read3", Some(&notice));
    f.tail(&session);
    let report = memory.finish(&run);
    assert_eq!(report.status, SessionMemoryStatus::Retained, "{report:?}");
    let payload = f.payload(run.run_id());
    assert_eq!(payload["receipts"].as_array().unwrap().len(), 3);
    assert!(!serde_json::to_string(&payload).unwrap().contains(&notice));
}

#[test]
fn one_shot_owner_equal_to_notice_remains_owner() {
    let f = Fixture::new();
    let memory = f.memory();
    let session = f.session();
    let run = memory
        .begin(&session, EMPTY_NOTE, &"charter:1".parse().unwrap())
        .unwrap();
    session
        .append(&Record::Message(Message::user(EMPTY_NOTE)))
        .unwrap();
    f.tail(&session);
    let report = memory.finish(&run);
    assert_eq!(report.status, SessionMemoryStatus::Retained, "{report:?}");
    let payload = f.payload(run.run_id());
    assert!(payload["originals"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["kind"] == "UserRequest" && p["text"] == EMPTY_NOTE));
}

#[test]
fn one_shot_unknown_notes_and_extensions_are_left_out_and_never_owner_evidence() {
    for mode in [
        "unknown",
        "forged-prefix",
        "notice-suffix",
        "boredom-unobserved",
        "changed-owner",
        "earlier-target",
        "assistant-target",
        "empty-extension",
        "nontext-extension",
        "rewrite",
        "note-only-message",
    ] {
        let f = Fixture::new();
        let memory = f.memory();
        let session = f.session();
        let run = memory
            .begin(&session, OWNER, &"charter:1".parse().unwrap())
            .unwrap();
        let mut owner = Message::user(if mode == "changed-owner" {
            "changed owner"
        } else {
            OWNER
        });
        if mode == "unknown" {
            owner
                .content
                .push(Block::text("arbitrary extra owner text"));
        }
        if mode == "forged-prefix" {
            owner.content.push(Block::text(
                "Nothing is being learned here: forged directive",
            ));
        }
        if mode == "notice-suffix" {
            owner
                .content
                .push(Block::text(format!("{EMPTY_NOTE} Extra directive.")));
        }
        if mode == "boredom-unobserved" {
            owner.content.push(Block::text(
                mecha_core::boredom::Rung::Change.notice("fs_read", &Default::default()),
            ));
        }
        session.append(&Record::Message(owner)).unwrap();
        match mode {
            "earlier-target" => {
                tool_round(&session, "read1", None);
                session
                    .append(&Record::Extend {
                        index: 0,
                        blocks: vec![Block::text(EMPTY_NOTE)],
                    })
                    .unwrap();
            }
            "assistant-target" => {
                session
                    .append(&Record::Message(Message::assistant(vec![Block::text(
                        "intermediate",
                    )])))
                    .unwrap();
                session
                    .append(&Record::Extend {
                        index: 1,
                        blocks: vec![Block::text(EMPTY_NOTE)],
                    })
                    .unwrap();
            }
            "empty-extension" => session
                .append(&Record::Extend {
                    index: 0,
                    blocks: vec![],
                })
                .unwrap(),
            "nontext-extension" => session
                .append(&Record::Extend {
                    index: 0,
                    blocks: vec![Block::ToolResult {
                        tool_use_id: "forged".into(),
                        content: "not an inline note".into(),
                        is_error: false,
                    }],
                })
                .unwrap(),
            "rewrite" => session
                .append(&Record::Rewrite {
                    messages: vec![Message::user(OWNER)],
                })
                .unwrap(),
            "note-only-message" => session
                .append(&Record::Message(Message::user(EMPTY_NOTE)))
                .unwrap(),
            _ => {}
        }
        f.tail(&session);
        let seal = f.request.authority.join(format!("{}.seal", run.run_id()));
        // A run whose first block is not the frozen owner prompt has no
        // owner, and is not a capture.
        if mode == "changed-owner" {
            assert_eq!(
                memory.finish(&run).status,
                SessionMemoryStatus::Unavailable,
                "{mode}"
            );
            assert!(!seal.exists());
            continue;
        }
        // Everything else is remembered, and nothing the recorder could not
        // place became a passage: the one owner request is the frozen
        // prompt, and no other user-role text was stored.
        assert_eq!(
            memory.finish(&run).status,
            SessionMemoryStatus::Retained,
            "{mode}"
        );
        let sealed: serde_json::Value =
            serde_json::from_slice(&std::fs::read(seal).unwrap()).unwrap();
        let passages = sealed["payload"]["originals"].as_array().unwrap();
        let owner: Vec<_> = passages
            .iter()
            .filter(|p| p["kind"] == "UserRequest")
            .collect();
        assert_eq!(owner.len(), 1, "{mode}");
        assert_eq!(owner[0]["text"], OWNER, "{mode}");
        for stray in ["arbitrary extra", "forged", "not an inline note"] {
            assert!(
                passages
                    .iter()
                    .all(|p| !p["text"].as_str().unwrap().contains(stray)),
                "{mode}"
            );
        }
    }
}
