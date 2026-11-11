#![cfg(feature = "memory-prototype")]
use chrono::Utc;
use mecha_core::{
    agent::{StopCause, Taint},
    goal_context::{CharterSnapshot, DecisionBinding, GoalContextState, OwnerInput, OwnerSource},
    memory_session::{
        chat::{ChatAttachMode, ChatMemory},
        SessionMemory, SessionMemoryRequest, SessionMemoryStatus,
    },
    message::{Block, Message},
    session::{Record, RunConfig, RunStats, Session, SessionKind, SessionMeta},
};
use std::{collections::BTreeSet, fs, path::PathBuf};
fn recorded_context(session: &Session) -> GoalContextState {
    let mut state = GoalContextState::default();
    for line in fs::read_to_string(&session.path).unwrap().lines() {
        if let Record::GoalContextDecision { binding, versions } =
            serde_json::from_str::<Record>(line).unwrap()
        {
            for version in versions {
                state.accept_version(version).unwrap();
            }
            state.bind(binding).unwrap();
        }
    }
    state
}
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
            .join(format!("chat-lifecycle-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        for p in ["db", "keys", "work", "sessions"] {
            mecha_core::create_private_dir(&root.join(p)).unwrap();
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
            kind: Some(SessionKind::Chat),
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
    }
    fn config(&self, s: &Session) {
        s.append(&Record::Config(RunConfig {
            workspace: self.workspace.clone(),
            tools: vec!["fs_read".into(), "goal_context".into()],
            ..Default::default()
        }))
        .unwrap();
    }
    fn turn(
        &self,
        chat: &mut ChatMemory,
        s: &Session,
        prompt: &str,
        charter: &str,
    ) -> mecha_core::memory_session::chat::ChatMemoryReport {
        self.turn_reported(chat, s, prompt, charter, 1)
    }
    fn turn_reported(
        &self,
        chat: &mut ChatMemory,
        s: &Session,
        prompt: &str,
        charter: &str,
        requests: u32,
    ) -> mecha_core::memory_session::chat::ChatMemoryReport {
        self.turn_outcome(
            chat,
            s,
            prompt,
            charter,
            RunStats {
                stop_cause: Some(StopCause::Completed),
                turns: requests,
                context_overflows: Some(0),
                ..Default::default()
            },
        )
    }
    fn turn_outcome(
        &self,
        chat: &mut ChatMemory,
        s: &Session,
        prompt: &str,
        charter: &str,
        outcome: RunStats,
    ) -> mecha_core::memory_session::chat::ChatMemoryReport {
        let turn = self.record_turn(chat, s, prompt, charter, outcome);
        chat.finish_turn(&turn)
    }
    fn record_turn(
        &self,
        chat: &mut ChatMemory,
        s: &Session,
        prompt: &str,
        charter: &str,
        outcome: RunStats,
    ) -> mecha_core::memory_session::chat::ChatMemoryTurn {
        let owner = OwnerInput::new(OwnerSource::CliChat, prompt.into()).unwrap();
        let turn = chat.begin_turn(s, &owner).unwrap();
        let mut transcript = Session::read(&s.path).unwrap();
        transcript.convo.messages.push(Message::user(prompt));
        s.append(&Record::Message(Message::user(prompt))).unwrap();
        let mut state = recorded_context(s);
        let version = state
            .transition(
                CharterSnapshot::from_source(
                    Some(format!("[[line]]\nid='priority'\ntext='{charter}'\n")),
                    false,
                )
                .unwrap(),
                Some(owner),
            )
            .unwrap()
            .unwrap();
        let binding = DecisionBinding::new(
            state.next_decision(),
            version.revision(),
            &transcript.convo.messages,
            Some(version.prompt_block()),
            true,
            0,
        )
        .unwrap()
        .with_conversation_messages(&transcript.convo.messages)
        .unwrap();
        s.append(&Record::GoalContextDecision {
            binding,
            versions: vec![version],
        })
        .unwrap();
        s.append(&Record::Message(Message::assistant(vec![Block::text(
            "Authored response.",
        )])))
        .unwrap();
        s.append(&Record::Outcome(outcome)).unwrap();
        s.append(&Record::Taint(Taint {
            private: true,
            untrusted: false,
        }))
        .unwrap();
        turn
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn changing_context_retains_distinct_turns_and_resumes_without_reimporting_history() {
    let f = Fixture::new();
    let s = f.session();
    let mut chat = f.memory().attach_chat(&s, ChatAttachMode::Create).unwrap();
    f.config(&s);
    let first = f.turn(
        &mut chat,
        &s,
        "Prefer blue folders",
        "Keep folders readable",
    );
    assert_eq!(first.status, SessionMemoryStatus::Retained, "{first:?}");
    let second = f.turn(
        &mut chat,
        &s,
        "Now prefer green folders",
        "Keep folders distinct",
    );
    assert_eq!(second.status, SessionMemoryStatus::Retained, "{second:?}");
    assert_ne!(first.receipt, second.receipt);
    assert_eq!(first.receipt.as_ref().unwrap().originals.len(), 2);
    assert_eq!(second.receipt.as_ref().unwrap().originals.len(), 2);
    assert!(Session::load(&s.path).is_err());
    drop(chat);
    let recovery = SessionMemory::open_for_recovery(&f.request).unwrap();
    assert_eq!(recovery.retry_chat(&first.run_id).receipt, first.receipt);
    let mut resumed = f.memory().attach_chat(&s, ChatAttachMode::Resume).unwrap();
    f.config(&s);
    assert_eq!(
        f.turn(&mut resumed, &s, "Use both colors", "Keep folders distinct")
            .status,
        SessionMemoryStatus::Retained
    );
}
#[test]
fn exclusive_attachment_and_unfinished_tail_refuse_continuation_but_not_old_seal() {
    let f = Fixture::new();
    let s = f.session();
    let mut chat = f.memory().attach_chat(&s, ChatAttachMode::Create).unwrap();
    f.config(&s);
    assert!(f.memory().attach_chat(&s, ChatAttachMode::Resume).is_err());
    let first = f.turn(&mut chat, &s, "Remember the authored label", "Keep records");
    assert_eq!(first.status, SessionMemoryStatus::Retained, "{first:?}");
    chat.begin_turn(
        &s,
        &OwnerInput::new(OwnerSource::CliChat, "Interrupted".into()).unwrap(),
    )
    .unwrap();
    s.append(&Record::Message(Message::user("Interrupted")))
        .unwrap();
    drop(chat);
    assert!(f.memory().attach_chat(&s, ChatAttachMode::Resume).is_err());
    assert_eq!(
        SessionMemory::open_for_recovery(&f.request)
            .unwrap()
            .retry_chat(&first.run_id)
            .receipt,
        first.receipt
    );
}
#[test]
fn prefix_mutation_and_withdrawal_block_resume_and_descendant_retry() {
    let f = Fixture::new();
    let s = f.session();
    let mut chat = f.memory().attach_chat(&s, ChatAttachMode::Create).unwrap();
    f.config(&s);
    let first = f.turn(&mut chat, &s, "Remember the authored label", "Keep records");
    let second = f.turn(&mut chat, &s, "Repeat the prior label", "Keep records");
    drop(chat);
    let recovery = SessionMemory::open_for_recovery(&f.request).unwrap();
    recovery.forget_chat(&first.run_id).unwrap();
    assert!(f.memory().attach_chat(&s, ChatAttachMode::Resume).is_err());
    assert_eq!(
        recovery.retry_chat(&second.run_id).status,
        SessionMemoryStatus::Unavailable
    );
}

#[test]
fn older_prefix_survives_oversized_later_tail_but_mutation_revokes_it() {
    let f = Fixture::new();
    let s = f.session();
    let mut chat = f.memory().attach_chat(&s, ChatAttachMode::Create).unwrap();
    f.config(&s);
    let first = f.turn(&mut chat, &s, "Remember the authored label", "Keep records");
    drop(chat);
    assert_eq!(first.status, SessionMemoryStatus::Retained, "{first:?}");
    let original = fs::read(&s.path).unwrap();
    use std::io::Write;
    let mut file = fs::OpenOptions::new().append(true).open(&s.path).unwrap();
    file.write_all(&vec![b'x'; 4 * 1024 * 1024 + 1]).unwrap();
    drop(file);
    let recovery = SessionMemory::open_for_recovery(&f.request).unwrap();
    assert_eq!(recovery.retry_chat(&first.run_id).receipt, first.receipt);
    assert!(f.memory().attach_chat(&s, ChatAttachMode::Resume).is_err());
    let mut changed = original.clone();
    let index = changed.iter().position(|b| *b == b'R').unwrap();
    changed[index] = b'S';
    fs::write(&s.path, changed).unwrap();
    assert_eq!(
        recovery.retry_chat(&first.run_id).status,
        SessionMemoryStatus::Unavailable
    );
    fs::remove_file(&s.path).unwrap();
    assert!(
        recovery.forget_chat(&first.run_id).is_ok(),
        "withdrawal remains possible after source deletion"
    );
}

#[test]
fn sealed_admission_retry_is_idempotent_and_pending_receipt_blocks_execution() {
    let f = Fixture::new();
    let s = f.session();
    let mut chat = f.memory().attach_chat(&s, ChatAttachMode::Create).unwrap();
    f.config(&s);
    let first = f.turn(&mut chat, &s, "Remember the authored label", "Keep records");
    drop(chat);
    let path = f
        .request
        .authority
        .join(format!("{}.chat-retained", first.run_id));
    fs::remove_file(path).unwrap();
    assert!(f.memory().attach_chat(&s, ChatAttachMode::Resume).is_err());
    let recovery = SessionMemory::open_for_recovery(&f.request).unwrap();
    assert_eq!(recovery.retry_chat(&first.run_id).receipt, first.receipt);
    assert_eq!(recovery.retry_chat(&first.run_id).receipt, first.receipt);
    assert!(f.memory().attach_chat(&s, ChatAttachMode::Resume).is_ok());
}

#[test]
fn missing_marker_wrong_authority_and_aliases_refuse_attachment() {
    let f = Fixture::new();
    let s = f.session();
    let mut chat = f.memory().attach_chat(&s, ChatAttachMode::Create).unwrap();
    f.config(&s);
    let first = f.turn(&mut chat, &s, "Remember the authored label", "Keep records");
    drop(chat);
    let other = Fixture::new();
    assert!(other
        .memory()
        .attach_chat(&s, ChatAttachMode::Resume)
        .is_err());
    let bytes = fs::read_to_string(&s.path).unwrap();
    let stripped = bytes
        .lines()
        .filter(|l| !l.contains("\"record\":\"chat_memory\""))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&s.path, stripped).unwrap();
    assert!(f.memory().attach_chat(&s, ChatAttachMode::Resume).is_err());
    fs::write(&s.path, bytes).unwrap();
    let alias = f.root.join("alias");
    fs::hard_link(&s.path, &alias).unwrap();
    assert!(f.memory().attach_chat(&s, ChatAttachMode::Resume).is_err());
    assert_eq!(
        SessionMemory::open_for_recovery(&f.request)
            .unwrap()
            .retry_chat(&first.run_id)
            .status,
        SessionMemoryStatus::Unavailable
    );
}

#[tokio::test]
async fn per_turn_recall_uses_current_owner_context_and_retains_exact_delivery() {
    let f = Fixture::new();
    let s = f.session();
    let mut chat = f.memory().attach_chat(&s, ChatAttachMode::Create).unwrap();
    f.config(&s);
    let first = f.turn(&mut chat, &s, "The authored label is coral", "Keep records");
    assert_eq!(first.status, SessionMemoryStatus::Retained, "{first:?}");
    let owner = OwnerInput::new(OwnerSource::CliChat, "Find the prior coral label".into()).unwrap();
    let turn = chat.begin_turn(&s, &owner).unwrap();
    let tool = chat.goal_memory(&turn).unwrap();
    let mut transcript = Session::read(&s.path).unwrap();
    transcript.convo.messages.push(Message::user(owner.text()));
    s.append(&Record::Message(Message::user(owner.text())))
        .unwrap();
    let mut state = recorded_context(&s);
    let version = state
        .transition(CharterSnapshot::disabled(), Some(owner))
        .unwrap()
        .unwrap();
    let binding = DecisionBinding::new(
        state.next_decision(),
        version.revision(),
        &transcript.convo.messages,
        Some(version.prompt_block()),
        true,
        0,
    )
    .unwrap()
    .with_conversation_messages(&transcript.convo.messages)
    .unwrap();
    s.append(&Record::GoalContextDecision {
        binding,
        versions: vec![version.clone()],
    })
    .unwrap();
    let context = mecha_core::tool::ToolCtx {
        workspace: f.workspace.clone(),
        ..Default::default()
    };
    *context.goal_context_version.lock().unwrap() = Some(version);
    let input = serde_json::json!({"memory":{"query":"coral"}});
    let output = tool.call(input.clone(), &context).await.unwrap();
    assert!(!output.is_error, "{}", output.content);
    // The first turn kept the owner's words and a reply written before
    // anything from outside arrived, so the recall is private and local.
    assert!(!output.external, "{}", output.content);
    let packet: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert!(packet["confirmed_goal"].is_null());
    assert_eq!(packet["goal_context_revision"], 2);
    s.append(&Record::Message(Message::assistant(vec![Block::ToolUse {
        id: "recall".into(),
        name: "goal_context".into(),
        input,
    }])))
    .unwrap();
    let mut result = Message::user("");
    result.content = vec![Block::ToolResult {
        tool_use_id: "recall".into(),
        content: output.content,
        is_error: false,
    }];
    s.append(&Record::Message(result)).unwrap();
    // A second native request uses exactly the accumulated conversation.
    let t = Session::read(&s.path).unwrap();
    let state = recorded_context(&s);
    let version = state.current().unwrap();
    let binding = DecisionBinding::new(
        state.next_decision(),
        version.revision(),
        &t.convo.messages,
        Some(version.prompt_block()),
        false,
        0,
    )
    .unwrap()
    .with_conversation_messages(&t.convo.messages)
    .unwrap();
    s.append(&Record::GoalContextDecision {
        binding,
        versions: vec![],
    })
    .unwrap();
    s.append(&Record::Message(Message::assistant(vec![Block::text(
        "The earlier authored label was coral.",
    )])))
    .unwrap();
    s.append(&Record::Outcome(RunStats {
        stop_cause: Some(StopCause::Completed),
        turns: 2,
        context_overflows: Some(0),
        ..Default::default()
    }))
    .unwrap();
    s.append(&Record::Taint(Taint {
        private: true,
        untrusted: false,
    }))
    .unwrap();
    let second = chat.finish_turn(&turn);
    assert_eq!(second.status, SessionMemoryStatus::Retained, "{second:?}");
    let seal: serde_json::Value = serde_json::from_slice(
        &fs::read(
            f.request
                .authority
                .join(format!("{}.chat-seal", second.run_id)),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(seal["payload"]["originals"].as_array().unwrap().len(), 3);
    assert!(seal["payload"]["consulted"]
        .as_array()
        .unwrap()
        .contains(&serde_json::to_value(first.receipt.unwrap().projection).unwrap()));
}

#[test]
fn missing_decision_coverage_withholds_even_a_completed_answer() {
    let f = Fixture::new();
    let s = f.session();
    let mut chat = f.memory().attach_chat(&s, ChatAttachMode::Create).unwrap();
    f.config(&s);
    let report = f.turn_reported(&mut chat, &s, "Authored input", "Keep records", 2);
    assert_eq!(report.status, SessionMemoryStatus::Unavailable);
}

#[test]
fn unsupported_compaction_overflow_and_forced_final_outcomes_stop_continuation() {
    for outcome in [
        RunStats {
            stop_cause: Some(StopCause::Completed),
            turns: 1,
            compactions: 1,
            context_overflows: Some(0),
            ..Default::default()
        },
        RunStats {
            stop_cause: Some(StopCause::Completed),
            turns: 1,
            context_overflows: Some(1),
            ..Default::default()
        },
        RunStats {
            stop_cause: Some(StopCause::MaxTurns),
            turns: 1,
            context_overflows: Some(0),
            ..Default::default()
        },
    ] {
        let f = Fixture::new();
        let s = f.session();
        let mut chat = f.memory().attach_chat(&s, ChatAttachMode::Create).unwrap();
        f.config(&s);
        let report = f.turn_outcome(&mut chat, &s, "Authored input", "Keep records", outcome);
        assert_eq!(report.status, SessionMemoryStatus::Unavailable);
        assert!(chat
            .begin_turn(
                &s,
                &OwnerInput::new(OwnerSource::CliChat, "Continue".into()).unwrap()
            )
            .is_err());
    }
}

#[test]
fn repeated_owner_text_requires_a_new_authenticated_owner_occurrence() {
    let f = Fixture::new();
    let s = f.session();
    let mut chat = f.memory().attach_chat(&s, ChatAttachMode::Create).unwrap();
    f.config(&s);
    let first = f.turn(&mut chat, &s, "Repeat the authored request", "Keep records");
    assert_eq!(first.status, SessionMemoryStatus::Retained, "{first:?}");
    let owner =
        OwnerInput::new(OwnerSource::CliChat, "Repeat the authored request".into()).unwrap();
    let turn = chat.begin_turn(&s, &owner).unwrap();
    let mut prior = Session::read(&s.path).unwrap();
    prior.convo.messages.push(Message::user(owner.text()));
    s.append(&Record::Message(Message::user(owner.text())))
        .unwrap();
    // A malformed recorder reuses the preceding owner occurrence because its
    // text happens to match. No new ContextVersion acknowledges this submit.
    let state = recorded_context(&s);
    let version = state.current().unwrap();
    let binding = DecisionBinding::new(
        state.next_decision(),
        version.revision(),
        &prior.convo.messages,
        Some(version.prompt_block()),
        true,
        0,
    )
    .unwrap()
    .with_conversation_messages(&prior.convo.messages)
    .unwrap();
    s.append(&Record::GoalContextDecision {
        binding,
        versions: vec![],
    })
    .unwrap();
    s.append(&Record::Message(Message::assistant(vec![Block::text(
        "Authored second response.",
    )])))
    .unwrap();
    s.append(&Record::Outcome(RunStats {
        stop_cause: Some(StopCause::Completed),
        turns: 1,
        context_overflows: Some(0),
        ..Default::default()
    }))
    .unwrap();
    s.append(&Record::Taint(Taint {
        private: true,
        untrusted: false,
    }))
    .unwrap();
    assert_eq!(
        chat.finish_turn(&turn).status,
        SessionMemoryStatus::Unavailable
    );
}

#[test]
fn recomputed_native_bindings_cannot_omit_or_reuse_response_boundaries() {
    let mut accepted = vec![];
    for mode in ["empty", "old_only", "stale", "skipped"] {
        let f = Fixture::new();
        let s = f.session();
        let mut chat = f.memory().attach_chat(&s, ChatAttachMode::Create).unwrap();
        f.config(&s);
        assert_eq!(
            f.turn(&mut chat, &s, "Prior authored task", "Keep records")
                .status,
            SessionMemoryStatus::Retained
        );
        let owner = OwnerInput::new(OwnerSource::CliChat, "New authored task".into()).unwrap();
        let turn = chat.begin_turn(&s, &owner).unwrap();
        let prior = Session::read(&s.path).unwrap();
        let mut history = prior.convo.messages.clone();
        history.push(Message::user(owner.text()));
        s.append(&Record::Message(Message::user(owner.text())))
            .unwrap();
        let mut state = recorded_context(&s);
        let version = state
            .transition(CharterSnapshot::disabled(), Some(owner))
            .unwrap()
            .unwrap();
        let sent = match mode {
            "empty" => vec![],
            "old_only" => prior.convo.messages,
            _ => history.clone(),
        };
        let binding = DecisionBinding::new(
            state.next_decision(),
            version.revision(),
            &sent,
            Some(version.prompt_block()),
            true,
            0,
        )
        .unwrap()
        .with_conversation_messages(&sent)
        .unwrap();
        state.bind(binding.clone()).unwrap();
        s.append(&Record::GoalContextDecision {
            binding,
            versions: vec![version.clone()],
        })
        .unwrap();
        let mut turns = 1;
        if matches!(mode, "stale" | "skipped") {
            for n in 0..if mode == "skipped" { 2 } else { 1 } {
                let call = Message::assistant(vec![Block::ToolUse {
                    id: format!("read-{n}"),
                    name: "fs_read".into(),
                    input: serde_json::json!({"path":"authored.txt"}),
                }]);
                let mut result = Message::user("");
                result.content = vec![Block::ToolResult {
                    tool_use_id: format!("read-{n}"),
                    content: "Authored file content".into(),
                    is_error: false,
                }];
                s.append(&Record::Message(call.clone())).unwrap();
                s.append(&Record::Message(result.clone())).unwrap();
                history.push(call);
                history.push(result);
            }
            let next = if mode == "stale" { &sent } else { &history };
            let binding = DecisionBinding::new(
                state.next_decision(),
                version.revision(),
                next,
                Some(version.prompt_block()),
                false,
                0,
            )
            .unwrap()
            .with_conversation_messages(next)
            .unwrap();
            s.append(&Record::GoalContextDecision {
                binding,
                versions: vec![],
            })
            .unwrap();
            turns = 2;
        }
        s.append(&Record::Message(Message::assistant(vec![Block::text(
            "Authored final reply.",
        )])))
        .unwrap();
        s.append(&Record::Outcome(RunStats {
            stop_cause: Some(StopCause::Completed),
            turns,
            context_overflows: Some(0),
            ..Default::default()
        }))
        .unwrap();
        s.append(&Record::Taint(Taint {
            private: true,
            untrusted: false,
        }))
        .unwrap();
        if chat.finish_turn(&turn).status != SessionMemoryStatus::Unavailable {
            accepted.push(mode);
        }
    }
    assert!(
        accepted.is_empty(),
        "accepted malformed boundaries: {accepted:?}"
    );
}

#[test]
fn turn_binding_and_finish_require_the_original_attachment_lease() {
    for finish in [false, true] {
        let f = Fixture::new();
        let a = f.session();
        let mut chat_a = f.memory().attach_chat(&a, ChatAttachMode::Create).unwrap();
        f.config(&a);
        let turn = chat_a
            .begin_turn(
                &a,
                &OwnerInput::new(OwnerSource::CliChat, "Authored task".into()).unwrap(),
            )
            .unwrap();
        // Binding itself must refuse even if no response has been recorded yet.
        drop(chat_a);
        let b = f.session();
        let mut chat_b = f.memory().attach_chat(&b, ChatAttachMode::Create).unwrap();
        f.config(&b);
        if !finish {
            assert!(chat_b.goal_memory(&turn).is_err());
        } else {
            assert_eq!(
                chat_b.finish_turn(&turn).status,
                SessionMemoryStatus::Unavailable
            );
        }
    }
}
#[test]
fn turn_finish_requires_the_original_attachment_lease() {
    let f = Fixture::new();
    let a = f.session();
    let mut chat_a = f.memory().attach_chat(&a, ChatAttachMode::Create).unwrap();
    f.config(&a);
    let turn = f.record_turn(
        &mut chat_a,
        &a,
        "Authored task",
        "Keep records",
        RunStats {
            stop_cause: Some(StopCause::Completed),
            turns: 1,
            context_overflows: Some(0),
            ..Default::default()
        },
    );
    drop(chat_a);
    let b = f.session();
    let mut chat_b = f.memory().attach_chat(&b, ChatAttachMode::Create).unwrap();
    f.config(&b);
    assert_eq!(
        chat_b.finish_turn(&turn).status,
        SessionMemoryStatus::Unavailable,
        "B must not seal A under B's lease"
    );
}

#[test]
fn tracked_reader_keeps_the_conversation_lease_until_it_is_dropped() {
    let f = Fixture::new();
    let s = f.session();
    let mut chat = f.memory().attach_chat(&s, ChatAttachMode::Create).unwrap();
    f.config(&s);
    let turn = f.record_turn(
        &mut chat,
        &s,
        "Authored task",
        "Keep records",
        RunStats {
            stop_cause: Some(StopCause::Completed),
            turns: 1,
            context_overflows: Some(0),
            ..Default::default()
        },
    );
    let tool = chat.goal_memory(&turn).unwrap();
    assert_eq!(
        chat.finish_turn(&turn).status,
        SessionMemoryStatus::Retained
    );
    drop(chat);
    assert!(
        f.memory().attach_chat(&s, ChatAttachMode::Resume).is_err(),
        "a retained tool still owns its conversation lease"
    );
    drop(tool);
    assert!(f.memory().attach_chat(&s, ChatAttachMode::Resume).is_ok());
}

#[tokio::test]
async fn schema_placeholder_refuses_dispatch_without_a_tracked_turn() {
    let f = Fixture::new();
    let s = f.session();
    let mut chat = f.memory().attach_chat(&s, ChatAttachMode::Create).unwrap();
    f.config(&s);
    assert_eq!(
        f.turn(&mut chat, &s, "The authored label is coral", "Keep records")
            .status,
        SessionMemoryStatus::Retained
    );
    let placeholder = chat.context_tool().unwrap();
    let state = recorded_context(&s);
    let context = mecha_core::tool::ToolCtx {
        workspace: f.workspace.clone(),
        ..Default::default()
    };
    *context.goal_context_version.lock().unwrap() = state.current().cloned();
    let turn = chat
        .begin_turn(
            &s,
            &OwnerInput::new(OwnerSource::CliChat, "Find the coral label".into()).unwrap(),
        )
        .unwrap();
    let tracked = chat.goal_memory(&turn).unwrap();
    assert_eq!(placeholder.name(), tracked.name());
    assert_eq!(placeholder.description(), tracked.description());
    assert_eq!(placeholder.input_schema(), tracked.input_schema());
    assert_eq!(placeholder.capabilities(), tracked.capabilities());
    assert_eq!(placeholder.effect(), tracked.effect());
    assert_eq!(placeholder.read_only(), tracked.read_only());
    let result = placeholder
        .call(serde_json::json!({"memory":{"query":"coral"}}), &context)
        .await
        .unwrap();
    assert!(
        result.is_error,
        "schema placeholder delivered: {}",
        result.content
    );
    assert!(!result.external);
}

#[test]
fn dangling_alternate_protocol_claims_refuse_chat_and_ordinary_routes() {
    let mut accepted = vec![];
    for chat_mode in [false, true] {
        let f = Fixture::new();
        let mut session = f.session();
        let id = if chat_mode {
            let mut chat = f
                .memory()
                .attach_chat(&session, ChatAttachMode::Create)
                .unwrap();
            f.config(&session);
            let report = f.turn(&mut chat, &session, "Authored input", "Keep records");
            assert_eq!(report.status, SessionMemoryStatus::Retained);
            report.run_id
        } else {
            session.meta.kind = Some(SessionKind::Run);
            fs::write(
                &session.path,
                serde_json::to_string(&Record::Meta(session.meta.clone())).unwrap() + "\n",
            )
            .unwrap();
            f.config(&session);
            let goal: mecha_core::goal::GoalRef = "charter:authored".parse().unwrap();
            session
                .append(&Record::GoalAnchor {
                    goal: Some(goal.clone()),
                })
                .unwrap();
            let memory = f.memory();
            let run = memory.begin(&session, "Authored input", &goal).unwrap();
            session
                .append(&Record::Message(Message::user("Authored input")))
                .unwrap();
            session
                .append(&Record::Message(Message::assistant(vec![Block::text(
                    "Authored response",
                )])))
                .unwrap();
            session.append(&Record::Taint(Taint::default())).unwrap();
            session
                .append(&Record::Outcome(RunStats {
                    stop_cause: Some(StopCause::Completed),
                    turns: 1,
                    context_overflows: Some(0),
                    ..Default::default()
                }))
                .unwrap();
            assert_eq!(memory.finish(&run).status, SessionMemoryStatus::Retained);
            run.run_id().to_string()
        };
        let alternate = if chat_mode { "begin" } else { "chat-begin" };
        std::os::unix::fs::symlink(
            f.request.authority.join("missing-claim"),
            f.request.authority.join(format!("{id}.{alternate}")),
        )
        .unwrap();
        let recovery = SessionMemory::open_for_recovery(&f.request).unwrap();
        if recovery.is_chat_capture(&id).is_ok() {
            accepted.push(alternate);
        }
        if chat_mode && recovery.retry_chat(&id).status == SessionMemoryStatus::Retained {
            accepted.push("chat-source-guard");
        }
    }
    assert!(
        accepted.is_empty(),
        "accepted dangling alternate claims: {accepted:?}"
    );
}

// The limit applies to serialized JSON bytes, including quotes and UTF-8.
fn invocation_boundary_inputs() -> Vec<serde_json::Value> {
    let mut inputs = vec![];
    for seed in ["", "é🦀", "\"\\\n\t"] {
        for size in [1024, 1025] {
            let mut text = seed.to_owned();
            let overhead =
                serde_json::to_vec(&serde_json::json!({"path":"result.txt","content":text}))
                    .unwrap()
                    .len();
            text.push_str(&"x".repeat(size - overhead));
            let input = serde_json::json!({"path":"result.txt","content":text});
            assert_eq!(serde_json::to_vec(&input).unwrap().len(), size);
            inputs.push(input);
        }
    }
    inputs
}
fn assert_invocation_capture(
    payload: &serde_json::Value,
    source: &str,
    session_id: &str,
    input: &serde_json::Value,
) {
    use sha2::{Digest, Sha256};
    let encoded = serde_json::to_vec(input).unwrap();
    let digest: String = Sha256::digest(&encoded)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let passage = payload["originals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["kind"] == "ActionInvocation")
        .unwrap();
    let line = source
        .lines()
        .position(|line| line.contains("\"name\":\"fs_write\""))
        .unwrap()
        + 1;
    assert_eq!(passage["line"], line);
    assert_eq!(passage["pointer"], "/content/1/input");
    assert_eq!(
        passage["source"]["id"],
        format!("{session_id}:{line}:/content/1/input")
    );
    let action: serde_json::Value =
        serde_json::from_str(passage["text"].as_str().unwrap()).unwrap();
    assert_eq!(action["call_id"], "write1");
    assert_eq!(action["tool_name"], "fs_write");
    assert_eq!(action["input_digest"], digest);
    if encoded.len() <= 1024 {
        let historical = mecha_core::memory::host_capture::ActionInvocation {
            call_id: "write1".into(),
            tool_name: "fs_write".into(),
            input_status: mecha_core::memory::host_capture::InputStatus::Complete,
            input: input.clone(),
            input_digest: digest,
        };
        assert_eq!(passage["text"], serde_json::to_string(&historical).unwrap());
    } else {
        assert_eq!(action["input_status"], "Oversized");
        assert!(action["input"].is_null());
    }
    assert_eq!(
        payload["receipts"],
        serde_json::json!([{
            "call_id":"write1", "tool_name":"fs_write", "status":"Succeeded"
        }])
    );
    let native: serde_json::Value =
        serde_json::from_str(source.lines().nth(line - 1).unwrap()).unwrap();
    assert_eq!(native["content"][1]["input"], *input);
}
fn authored_write(input: serde_json::Value, result_text: &str) -> (Message, Message) {
    let call = Message::assistant(vec![
        Block::Thinking {
            text: "Private authored reasoning".into(),
            signature: None,
        },
        Block::ToolUse {
            id: "write1".into(),
            name: "fs_write".into(),
            input,
        },
    ]);
    let mut result = Message::user("");
    result.content = vec![Block::ToolResult {
        tool_use_id: "write1".into(),
        content: result_text.into(),
        is_error: false,
    }];
    (call, result)
}

fn record_write_turn(
    chat: &mut ChatMemory,
    s: &Session,
    input: serde_json::Value,
    result_text: &str,
) -> mecha_core::memory_session::chat::ChatMemoryTurn {
    let owner = OwnerInput::new(OwnerSource::CliChat, "Write fixture".into()).unwrap();
    let turn = chat.begin_turn(s, &owner).unwrap();
    let mut history = Session::read(&s.path).unwrap().convo.messages;
    history.push(Message::user(owner.text()));
    s.append(&Record::Message(history.last().unwrap().clone()))
        .unwrap();
    let mut state = recorded_context(s);
    let version = state
        .transition(CharterSnapshot::disabled(), Some(owner))
        .unwrap()
        .unwrap();
    let binding = DecisionBinding::new(
        state.next_decision(),
        version.revision(),
        &history,
        Some(version.prompt_block()),
        true,
        0,
    )
    .unwrap()
    .with_conversation_messages(&history)
    .unwrap();
    state.bind(binding.clone()).unwrap();
    s.append(&Record::GoalContextDecision {
        binding,
        versions: vec![version.clone()],
    })
    .unwrap();
    let (call, result) = authored_write(input, result_text);
    for msg in [call, result] {
        s.append(&Record::Message(msg.clone())).unwrap();
        history.push(msg);
    }
    let binding = DecisionBinding::new(
        state.next_decision(),
        version.revision(),
        &history,
        Some(version.prompt_block()),
        false,
        0,
    )
    .unwrap()
    .with_conversation_messages(&history)
    .unwrap();
    s.append(&Record::GoalContextDecision {
        binding,
        versions: vec![],
    })
    .unwrap();
    s.append(&Record::Message(Message::assistant(vec![Block::text(
        "Authored final response",
    )])))
    .unwrap();
    s.append(&Record::Outcome(RunStats {
        stop_cause: Some(StopCause::Completed),
        turns: 2,
        context_overflows: Some(0),
        ..Default::default()
    }))
    .unwrap();
    s.append(&Record::Taint(Taint {
        private: true,
        untrusted: false,
    }))
    .unwrap();
    turn
}
#[test]
fn oversized_invocation_retains_bounded_metadata_and_exact_sealed_source() {
    for input in invocation_boundary_inputs() {
        let f = Fixture::new();
        let s = f.session();
        let mut chat = f.memory().attach_chat(&s, ChatAttachMode::Create).unwrap();
        s.append(&Record::Config(RunConfig {
            workspace: f.workspace.clone(),
            tools: vec!["fs_write".into(), "goal_context".into()],
            ..Default::default()
        }))
        .unwrap();
        let turn = record_write_turn(&mut chat, &s, input.clone(), "wrote fixture");
        let report = chat.finish_turn(&turn);
        assert_eq!(report.status, SessionMemoryStatus::Retained, "{report:?}");
        assert_eq!(
            serde_json::to_value(&report).unwrap()["semantic_assessment"],
            "not_evaluated"
        );
        let seal: serde_json::Value = serde_json::from_slice(
            &fs::read(
                f.request
                    .authority
                    .join(format!("{}.chat-seal", report.run_id)),
            )
            .unwrap(),
        )
        .unwrap();
        let source = fs::read_to_string(&s.path).unwrap();
        assert_invocation_capture(&seal["payload"], &source, &s.meta.id, &input);
        drop(chat);
        let recovery = SessionMemory::open_for_recovery(&f.request).unwrap();
        assert_eq!(recovery.retry_chat(&report.run_id).receipt, report.receipt);
        fs::write(&s.path, source.replacen("result.txt", "mutant.txt", 1)).unwrap();
        assert_eq!(
            recovery.retry_chat(&report.run_id).status,
            SessionMemoryStatus::Unavailable
        );
        fs::write(&s.path, source).unwrap();
        assert_eq!(recovery.retry_chat(&report.run_id).receipt, report.receipt);
        fs::remove_file(&s.path).unwrap();
        assert_eq!(
            recovery.retry_chat(&report.run_id).status,
            SessionMemoryStatus::Unavailable
        );
    }
}

#[test]
fn oversized_invocation_does_not_relax_result_passage_bound() {
    let f = Fixture::new();
    let s = f.session();
    let mut chat = f.memory().attach_chat(&s, ChatAttachMode::Create).unwrap();
    s.append(&Record::Config(RunConfig {
        workspace: f.workspace.clone(),
        tools: vec!["fs_write".into(), "goal_context".into()],
        ..Default::default()
    }))
    .unwrap();
    let turn = record_write_turn(
        &mut chat,
        &s,
        invocation_boundary_inputs()[1].clone(),
        &"x".repeat(4097),
    );
    let report = chat.finish_turn(&turn);
    assert_eq!(report.status, SessionMemoryStatus::Unavailable);
    assert!(!f
        .request
        .authority
        .join(format!("{}.chat-seal", report.run_id))
        .exists());
}

fn authored_graph_surface(f: &Fixture) -> mecha_core::memory_session::surface::ChatGraphSurface {
    use mecha_core::memory_session::surface::{ChatGraphSurface, GraphToolBinding};
    ChatGraphSurface {
        version: "ordinary-chat-graph-surface-v1".into(),
        server_name: "authored".into(),
        launch_digest: "a".repeat(64),
        local_tools: vec!["fs_read".into(), "goal_context".into()],
        graph_tools: vec![GraphToolBinding {
            local_name: "authored__kg_search".into(),
            remote_name: "kg_search".into(),
            schema_digest: "b".repeat(64),
            description_digest: "c".repeat(64),
            read_only: true,
            private_data: true,
            untrusted_input: true,
            egress: "none".into(),
            destructive: false,
            effect: "read_that_records".into(),
            deferred: false,
            fixed_workspace: f.workspace.clone(),
        }],
    }
}

#[test]
fn graph_attachment_retains_and_recovers_without_live_runtime() {
    let f = Fixture::new();
    let s = f.session();
    let graph = authored_graph_surface(&f);
    let mut chat = f
        .memory()
        .attach_chat_with_graph(&s, ChatAttachMode::Create, Some(graph))
        .unwrap();
    s.append(&Record::Config(RunConfig {
        workspace: f.workspace.clone(),
        tools: vec![
            "authored__kg_search".into(),
            "fs_read".into(),
            "goal_context".into(),
        ],
        ..Default::default()
    }))
    .unwrap();
    let report = f.turn(
        &mut chat,
        &s,
        "Keep authored graph snapshots",
        "Keep records",
    );
    assert_eq!(report.status, SessionMemoryStatus::Retained, "{report:?}");
    assert_eq!(report.protocol, "ordinary-chat-memory-v2");
    drop(chat);
    assert!(f.memory().attach_chat(&s, ChatAttachMode::Resume).is_err());
    let recovery = SessionMemory::open_for_recovery(&f.request).unwrap();
    let retried = recovery.retry_chat(&report.run_id);
    assert_eq!(retried.protocol, "ordinary-chat-memory-v2");
    assert_eq!(retried.receipt, report.receipt);
    assert!(f
        .memory()
        .attach_chat_with_graph(&s, ChatAttachMode::Resume, None)
        .is_ok());
}

fn graph_config(f: &Fixture, s: &Session) {
    s.append(&Record::Config(RunConfig {
        workspace: f.workspace.clone(),
        tools: vec![
            "authored__kg_search".into(),
            "fs_read".into(),
            "goal_context".into(),
        ],
        ..Default::default()
    }))
    .unwrap();
}

fn graph_result_turn(
    chat: &mut ChatMemory,
    s: &Session,
    name: &str,
    content: &str,
    provenance: (Option<bool>, bool),
    is_error: bool,
    untrusted: bool,
) -> mecha_core::memory_session::chat::ChatMemoryTurn {
    let (external, extra) = provenance;
    let owner =
        OwnerInput::new(OwnerSource::CliChat, "Find the authored graph label".into()).unwrap();
    let turn = chat.begin_turn(s, &owner).unwrap();
    s.append(&Record::Message(Message::user(owner.text())))
        .unwrap();
    let mut state = recorded_context(s);
    let version = state
        .transition(CharterSnapshot::disabled(), Some(owner))
        .unwrap()
        .unwrap();
    let t = Session::read(&s.path).unwrap();
    let binding = DecisionBinding::new(
        state.next_decision(),
        version.revision(),
        &t.convo.messages,
        Some(version.prompt_block()),
        true,
        0,
    )
    .unwrap()
    .with_conversation_messages(&t.convo.messages)
    .unwrap();
    s.append(&Record::GoalContextDecision {
        binding,
        versions: vec![version.clone()],
    })
    .unwrap();
    s.append(&Record::Message(Message::assistant(vec![Block::ToolUse {
        id: "graph-call".into(),
        name: name.into(),
        input: serde_json::json!({"query":"authored label"}),
    }])))
    .unwrap();
    let mut result = Message::user("");
    result.content = vec![Block::ToolResult {
        tool_use_id: "graph-call".into(),
        content: content.into(),
        is_error,
    }];
    if let Some(external) = external {
        result.tool_provenance.insert("graph-call".into(), external);
    }
    if extra {
        result
            .tool_provenance
            .insert("unrelated-call".into(), false);
    }
    s.append(&Record::Message(result)).unwrap();
    let t = Session::read(&s.path).unwrap();
    let state = recorded_context(s);
    let binding = DecisionBinding::new(
        state.next_decision(),
        version.revision(),
        &t.convo.messages,
        Some(version.prompt_block()),
        false,
        0,
    )
    .unwrap()
    .with_conversation_messages(&t.convo.messages)
    .unwrap();
    s.append(&Record::GoalContextDecision {
        binding,
        versions: vec![],
    })
    .unwrap();
    s.append(&Record::Message(Message::assistant(vec![Block::text(
        "The observed graph label was coral.",
    )])))
    .unwrap();
    s.append(&Record::Outcome(RunStats {
        stop_cause: Some(StopCause::Completed),
        turns: 2,
        context_overflows: Some(0),
        ..Default::default()
    }))
    .unwrap();
    s.append(&Record::Taint(Taint {
        private: true,
        untrusted,
    }))
    .unwrap();
    turn
}

#[test]
fn graph_results_retain_exact_native_snapshot_pointer_input_digest_and_failed_receipt() {
    for (external, is_error, content) in [
        (true, false, "{\"authored_label\":\"coral\"}"),
        (true, true, "MCP call failed: authored transport failure"),
        (false, true, "Blocked by an authored harness guard"),
    ] {
        let f = Fixture::new();
        let s = f.session();
        let mut chat = f
            .memory()
            .attach_chat_with_graph(&s, ChatAttachMode::Create, Some(authored_graph_surface(&f)))
            .unwrap();
        graph_config(&f, &s);
        let turn = graph_result_turn(
            &mut chat,
            &s,
            "authored__kg_search",
            content,
            (Some(external), false),
            is_error,
            external,
        );
        let report = chat.finish_turn(&turn);
        assert_eq!(report.status, SessionMemoryStatus::Retained, "{report:?}");
        assert_eq!(
            serde_json::to_value(report.semantic_assessment).unwrap(),
            "not_evaluated"
        );
        let seal: serde_json::Value = serde_json::from_slice(
            &fs::read(
                f.request
                    .authority
                    .join(format!("{}.chat-seal", report.run_id)),
            )
            .unwrap(),
        )
        .unwrap();
        let originals = seal["payload"]["originals"].as_array().unwrap();
        let result = originals
            .iter()
            .find(|p| p["kind"] == "ToolResult")
            .unwrap();
        assert_eq!(result["text"], content);
        assert_eq!(result["pointer"], "/content/0/content");
        let line = result["line"].as_u64().unwrap() as usize;
        let native: serde_json::Value = serde_json::from_str(
            fs::read_to_string(&s.path)
                .unwrap()
                .lines()
                .nth(line - 1)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(native["content"][0]["content"], content);
        assert_eq!(
            result["source"]["id"],
            format!("{}:{line}:/content/0/content", s.meta.id)
        );
        let action = originals
            .iter()
            .find(|p| p["kind"] == "ActionInvocation")
            .unwrap();
        let action: serde_json::Value =
            serde_json::from_str(action["text"].as_str().unwrap()).unwrap();
        assert_eq!(action["call_id"], "graph-call");
        assert_eq!(action["tool_name"], "authored__kg_search");
        assert_eq!(
            action["input"],
            serde_json::json!({"query":"authored label"})
        );
        use sha2::{Digest, Sha256};
        assert_eq!(
            action["input_digest"],
            Sha256::digest(br#"{"query":"authored label"}"#)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        assert_eq!(
            seal["payload"]["receipts"][0]["status"],
            if is_error { "Failed" } else { "Succeeded" }
        );
    }
}

#[test]
fn graph_capture_refuses_missing_extra_or_false_success_provenance_and_unadmitted_calls() {
    for (name, external, is_error, untrusted, extra) in [
        ("authored__kg_search", None, false, true, false),
        ("authored__kg_search", Some(true), false, true, true),
        ("authored__kg_search", Some(true), false, false, false),
        ("authored__kg_search", Some(false), false, true, false),
        ("other__kg_search", Some(true), false, true, false),
        ("fs_write", Some(false), false, true, false),
    ] {
        let f = Fixture::new();
        let s = f.session();
        let mut chat = f
            .memory()
            .attach_chat_with_graph(&s, ChatAttachMode::Create, Some(authored_graph_surface(&f)))
            .unwrap();
        graph_config(&f, &s);
        let turn = graph_result_turn(
            &mut chat,
            &s,
            name,
            "authored result",
            (external, extra),
            is_error,
            untrusted,
        );
        assert_eq!(
            chat.finish_turn(&turn).status,
            SessionMemoryStatus::Unavailable,
            "{name} {external:?} {extra}"
        );
        assert!(!f
            .request
            .authority
            .join(format!("{}.chat-seal", turn.run_id()))
            .exists());
    }
}

#[test]
fn graph_empty_and_oversized_results_withhold_whole_turn_and_prevent_continuation() {
    for content in [String::new(), "x".repeat(4097)] {
        let f = Fixture::new();
        let s = f.session();
        let mut chat = f
            .memory()
            .attach_chat_with_graph(&s, ChatAttachMode::Create, Some(authored_graph_surface(&f)))
            .unwrap();
        graph_config(&f, &s);
        let turn = graph_result_turn(
            &mut chat,
            &s,
            "authored__kg_search",
            &content,
            (Some(true), false),
            false,
            true,
        );
        assert_eq!(
            chat.finish_turn(&turn).status,
            SessionMemoryStatus::Unavailable
        );
        assert!(chat
            .begin_turn(
                &s,
                &OwnerInput::new(OwnerSource::CliChat, "Next".into()).unwrap()
            )
            .is_err());
    }
}

#[test]
fn graph_attachment_rejects_unsafe_ambiguous_and_unknown_descriptors_before_writing() {
    for mutation in 0..15 {
        let f = Fixture::new();
        let s = f.session();
        let mut surface = authored_graph_surface(&f);
        match mutation {
            0 => surface.version = "future".into(),
            1 => surface.graph_tools.clear(),
            2 => surface.graph_tools.push(surface.graph_tools[0].clone()),
            3 => surface.local_tools.push("goal_context".into()),
            4 => surface.graph_tools[0].effect = "read".into(),
            5 => surface.graph_tools[0].read_only = false,
            6 => surface.graph_tools[0].private_data = false,
            7 => surface.graph_tools[0].untrusted_input = false,
            8 => surface.graph_tools[0].egress = "chosen".into(),
            9 => surface.graph_tools[0].destructive = true,
            10 => surface.graph_tools[0].deferred = true,
            11 => surface.graph_tools[0].fixed_workspace = f.root.clone(),
            12 => surface.graph_tools[0].remote_name = "kg_write".into(),
            13 => surface.graph_tools[0].schema_digest = "unknown".into(),
            14 => surface.launch_digest = "A".repeat(64),
            _ => unreachable!(),
        }
        let before = fs::read(&s.path).unwrap();
        assert!(
            f.memory()
                .attach_chat_with_graph(&s, ChatAttachMode::Create, Some(surface))
                .is_err(),
            "mutation {mutation}"
        );
        assert_eq!(fs::read(&s.path).unwrap(), before);
        assert!(!fs::read_dir(&f.request.authority).unwrap().any(|p| p
            .unwrap()
            .path()
            .extension()
            .is_some_and(|e| e == "chat")));
    }
}

#[test]
fn graph_source_and_surface_mutation_invalidate_sealed_retry_without_runtime() {
    for surface_change in [false, true] {
        let f = Fixture::new();
        let s = f.session();
        let mut chat = f
            .memory()
            .attach_chat_with_graph(&s, ChatAttachMode::Create, Some(authored_graph_surface(&f)))
            .unwrap();
        graph_config(&f, &s);
        let turn = graph_result_turn(
            &mut chat,
            &s,
            "authored__kg_search",
            "Authored coral snapshot",
            (Some(true), false),
            false,
            true,
        );
        let report = chat.finish_turn(&turn);
        assert_eq!(report.status, SessionMemoryStatus::Retained);
        drop(chat);
        if surface_change {
            let path = fs::read_dir(&f.request.authority)
                .unwrap()
                .map(|p| p.unwrap().path())
                .find(|p| p.extension().is_some_and(|e| e == "chat"))
                .unwrap();
            let mut a: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            a["graph_surface"]["launch_digest"] = serde_json::json!("d".repeat(64));
            fs::write(path, serde_json::to_vec(&a).unwrap()).unwrap();
        } else {
            let text = fs::read_to_string(&s.path).unwrap();
            fs::write(
                &s.path,
                text.replace("Authored coral snapshot", "Authored black snapshot"),
            )
            .unwrap();
        }
        let recovery = SessionMemory::open_for_recovery(&f.request).unwrap();
        assert_eq!(
            recovery.retry_chat(&report.run_id).status,
            SessionMemoryStatus::Unavailable
        );
        assert!(f
            .memory()
            .attach_chat_with_graph(&s, ChatAttachMode::Resume, None)
            .is_err());
        if !surface_change {
            assert!(recovery.forget_chat(&report.run_id).is_ok());
        }
    }
}

#[test]
fn graph_retry_repairs_missing_admission_receipt_and_forget_withdraws_descendants() {
    let f = Fixture::new();
    let s = f.session();
    let mut chat = f
        .memory()
        .attach_chat_with_graph(&s, ChatAttachMode::Create, Some(authored_graph_surface(&f)))
        .unwrap();
    graph_config(&f, &s);
    let first = f.turn(
        &mut chat,
        &s,
        "Remember the authored graph scope",
        "Keep records",
    );
    let second = f.turn(
        &mut chat,
        &s,
        "Continue the authored graph scope",
        "Keep records",
    );
    assert_eq!(first.status, SessionMemoryStatus::Retained);
    assert_eq!(second.status, SessionMemoryStatus::Retained);
    assert!(f
        .memory()
        .attach_chat_with_graph(&s, ChatAttachMode::Resume, None)
        .is_err());
    drop(chat);
    fs::remove_file(
        f.request
            .authority
            .join(format!("{}.chat-retained", second.run_id)),
    )
    .unwrap();
    assert!(f
        .memory()
        .attach_chat_with_graph(&s, ChatAttachMode::Resume, None)
        .is_err());
    fs::remove_dir(&f.workspace).unwrap();
    let recovery = SessionMemory::open_for_recovery(&f.request).unwrap();
    let retry = recovery.retry_chat(&second.run_id);
    assert_eq!(retry.status, SessionMemoryStatus::Retained);
    assert_eq!(retry.protocol, "ordinary-chat-memory-v2");
    assert_eq!(retry.receipt, second.receipt);
    recovery.forget_chat(&first.run_id).unwrap();
    assert_eq!(
        recovery.retry_chat(&second.run_id).status,
        SessionMemoryStatus::Unavailable
    );
}

#[test]
fn graph_surface_authentication_detects_descriptor_drift_and_exact_config_membership() {
    let f = Fixture::new();
    let s = f.session();
    let graph = authored_graph_surface(&f);
    let chat = f
        .memory()
        .attach_chat_with_graph(&s, ChatAttachMode::Create, Some(graph.clone()))
        .unwrap();
    assert_eq!(chat.graph_surface().unwrap(), Some(graph.clone()));
    chat.validate_graph_surface(&graph).unwrap();
    assert!(chat
        .validate_surface(
            &["goal_context".into(), "authored__kg_search".into()],
            &f.workspace,
            &[]
        )
        .is_err());
    chat.validate_surface(
        &[
            "goal_context".into(),
            "authored__kg_search".into(),
            "fs_read".into(),
        ],
        &f.workspace,
        &[],
    )
    .unwrap();
    for which in 0..4 {
        let mut changed = graph.clone();
        match which {
            0 => changed.launch_digest = "d".repeat(64),
            1 => changed.graph_tools[0].schema_digest = "d".repeat(64),
            2 => changed.graph_tools[0].description_digest = "d".repeat(64),
            3 => changed.server_name = "another-server".into(),
            _ => unreachable!(),
        }
        assert!(chat.validate_graph_surface(&changed).is_err());
    }
    assert!(!graph.contains_tool("wrong__kg_search"));
    assert!(graph.contains_tool("authored__kg_search"));
    assert_eq!(
        graph.tool_names(),
        vec!["authored__kg_search", "fs_read", "goal_context"]
    );
    assert_ne!(graph.digest().unwrap(), {
        let mut g = graph.clone();
        g.launch_digest = "d".repeat(64);
        g.digest().unwrap()
    });
}

#[test]
fn local_attachment_keeps_v1_bytes_and_refuses_graph_resume_door() {
    let f = Fixture::new();
    let s = f.session();
    let chat = f.memory().attach_chat(&s, ChatAttachMode::Create).unwrap();
    assert!(chat.graph_surface().unwrap().is_none());
    drop(chat);
    let path = fs::read_dir(&f.request.authority)
        .unwrap()
        .map(|p| p.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "chat"))
        .unwrap();
    let bytes = fs::read(&path).unwrap();
    let a: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(a["protocol"], "ordinary-chat-memory-v1");
    assert!(a.get("graph_surface").is_none());
    assert_eq!(a.as_object().unwrap().len(), 11);
    assert!(f
        .memory()
        .attach_chat_with_graph(&s, ChatAttachMode::Resume, None)
        .is_err());
    assert!(f.memory().attach_chat(&s, ChatAttachMode::Resume).is_ok());
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn graph_attachment_unknown_missing_and_duplicate_wire_authority_refuse() {
    for mutation in 0..5 {
        let f = Fixture::new();
        let s = f.session();
        let chat = f
            .memory()
            .attach_chat_with_graph(&s, ChatAttachMode::Create, Some(authored_graph_surface(&f)))
            .unwrap();
        drop(chat);
        let path = fs::read_dir(&f.request.authority)
            .unwrap()
            .map(|p| p.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "chat"))
            .unwrap();
        let mut a: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        match mutation {
            0 => {
                a.as_object_mut().unwrap().remove("graph_surface");
            }
            1 => a["protocol"] = serde_json::json!("ordinary-chat-memory-v3"),
            2 => a["graph_surface"]["unknown_authority"] = serde_json::json!(true),
            3 => {
                a["graph_surface"]["graph_tools"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("private_data");
            }
            4 => {}
            _ => unreachable!(),
        }
        let bytes = if mutation == 4 {
            let mut text = serde_json::to_string(&a).unwrap();
            text.insert_str(1, "\"protocol\":\"ordinary-chat-memory-v2\",");
            text.into_bytes()
        } else {
            serde_json::to_vec(&a).unwrap()
        };
        fs::write(path, bytes).unwrap();
        assert!(
            f.memory()
                .attach_chat_with_graph(&s, ChatAttachMode::Resume, None)
                .is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn graph_pending_sealed_capture_retries_with_no_server_or_workspace() {
    let f = Fixture::new();
    let s = f.session();
    let mut chat = f
        .memory()
        .attach_chat_with_graph(&s, ChatAttachMode::Create, Some(authored_graph_surface(&f)))
        .unwrap();
    graph_config(&f, &s);
    let turn = graph_result_turn(
        &mut chat,
        &s,
        "authored__kg_search",
        "Authored coral snapshot",
        (Some(true), false),
        false,
        true,
    );
    // A receipt write cannot complete. Recovery must use the existing sealed
    // native result, not invoke the graph or regenerate any transcript text.
    let receipt_path = f
        .request
        .authority
        .join(format!("{}.chat-retained", turn.run_id()));
    mecha_core::create_private_dir(&receipt_path).unwrap();
    let report = chat.finish_turn(&turn);
    assert_eq!(report.status, SessionMemoryStatus::Pending, "{report:?}");
    assert_eq!(report.protocol, "ordinary-chat-memory-v2");
    assert!(f
        .request
        .authority
        .join(format!("{}.chat-seal", turn.run_id()))
        .is_file());
    let before = fs::read(&s.path).unwrap();
    drop(chat);
    fs::remove_dir(receipt_path).unwrap();
    fs::remove_dir(&f.workspace).unwrap();
    let recovery = SessionMemory::open_for_recovery(&f.request).unwrap();
    let retry = recovery.retry_chat(turn.run_id());
    assert_eq!(retry.status, SessionMemoryStatus::Retained, "{retry:?}");
    assert_eq!(retry.protocol, "ordinary-chat-memory-v2");
    assert_eq!(recovery.retry_chat(turn.run_id()).receipt, retry.receipt);
    assert_eq!(fs::read(&s.path).unwrap(), before);
}

#[test]
fn graph_cumulative_taint_and_provenance_on_nonresults_cannot_be_laundered() {
    for smuggled in [false, true] {
        let f = Fixture::new();
        let s = f.session();
        let mut chat = f
            .memory()
            .attach_chat_with_graph(&s, ChatAttachMode::Create, Some(authored_graph_surface(&f)))
            .unwrap();
        graph_config(&f, &s);
        if !smuggled {
            let turn = graph_result_turn(
                &mut chat,
                &s,
                "authored__kg_search",
                "Authored result",
                (Some(true), false),
                false,
                true,
            );
            assert_eq!(
                chat.finish_turn(&turn).status,
                SessionMemoryStatus::Retained
            );
        }
        // The fixture records a clean checkpoint. After graph delivery that
        // regresses native cumulative taint, even though the parser unions it.
        let turn = f.record_turn(
            &mut chat,
            &s,
            "Continue",
            "Keep records",
            RunStats {
                stop_cause: Some(StopCause::Completed),
                turns: 1,
                context_overflows: Some(0),
                ..Default::default()
            },
        );
        if smuggled {
            let mut rows: Vec<String> = fs::read_to_string(&s.path)
                .unwrap()
                .lines()
                .map(str::to_owned)
                .collect();
            let index = rows
                .iter()
                .rposition(|r| {
                    serde_json::from_str::<serde_json::Value>(r).unwrap()["record"] == "message"
                })
                .unwrap();
            let mut message: serde_json::Value = serde_json::from_str(&rows[index]).unwrap();
            message["tool_provenance"] = serde_json::json!({"not-a-result":false});
            rows[index] = serde_json::to_string(&message).unwrap();
            fs::write(&s.path, rows.join("\n") + "\n").unwrap();
        }
        assert_eq!(
            chat.finish_turn(&turn).status,
            SessionMemoryStatus::Unavailable
        );
    }
}

#[test]
fn unauthenticated_report_protocol_never_becomes_attachment_authority() {
    use mecha_core::memory_session::chat::CHAT_MEMORY_UNAVAILABLE_PROTOCOL;
    let f = Fixture::new();
    let s = f.session();
    let mut chat = f.memory().attach_chat(&s, ChatAttachMode::Create).unwrap();
    f.config(&s);
    let report = f.turn(&mut chat, &s, "Authored legacy report", "Keep records");
    drop(chat);
    let path = fs::read_dir(&f.request.authority)
        .unwrap()
        .map(|p| p.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "chat"))
        .unwrap();
    let mut a: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    a["protocol"] = serde_json::json!(CHAT_MEMORY_UNAVAILABLE_PROTOCOL);
    fs::write(path, serde_json::to_vec(&a).unwrap()).unwrap();
    let retry = SessionMemory::open_for_recovery(&f.request)
        .unwrap()
        .retry_chat(&report.run_id);
    assert_eq!(retry.protocol, CHAT_MEMORY_UNAVAILABLE_PROTOCOL);
    assert_eq!(retry.status, SessionMemoryStatus::Unavailable);
    assert!(f.memory().attach_chat(&s, ChatAttachMode::Resume).is_err());
    assert!(f
        .memory()
        .attach_chat_with_graph(&s, ChatAttachMode::Resume, None)
        .is_err());
    let text = fs::read_to_string(&s.path)
        .unwrap()
        .replace("ordinary-chat-memory-v1", CHAT_MEMORY_UNAVAILABLE_PROTOCOL);
    assert!(Session::parse(&s.path, &text).is_err());
}

#[tokio::test]
async fn graph_goal_context_native_provenance_must_match_the_tracked_attempt() {
    for provenance in [true, false] {
        let f = Fixture::new();
        let s = f.session();
        let mut chat = f
            .memory()
            .attach_chat_with_graph(&s, ChatAttachMode::Create, Some(authored_graph_surface(&f)))
            .unwrap();
        graph_config(&f, &s);
        let first = f.turn(&mut chat, &s, "The authored label is coral", "Keep records");
        assert_eq!(first.status, SessionMemoryStatus::Retained, "{first:?}");
        let owner =
            OwnerInput::new(OwnerSource::CliChat, "Find the prior coral label".into()).unwrap();
        let turn = chat.begin_turn(&s, &owner).unwrap();
        let tool = chat.goal_memory(&turn).unwrap();
        let mut transcript = Session::read(&s.path).unwrap();
        transcript.convo.messages.push(Message::user(owner.text()));
        s.append(&Record::Message(Message::user(owner.text())))
            .unwrap();
        let mut state = recorded_context(&s);
        let version = state
            .transition(CharterSnapshot::disabled(), Some(owner))
            .unwrap()
            .unwrap();
        let binding = DecisionBinding::new(
            state.next_decision(),
            version.revision(),
            &transcript.convo.messages,
            Some(version.prompt_block()),
            true,
            0,
        )
        .unwrap()
        .with_conversation_messages(&transcript.convo.messages)
        .unwrap();
        s.append(&Record::GoalContextDecision {
            binding,
            versions: vec![version.clone()],
        })
        .unwrap();
        let context = mecha_core::tool::ToolCtx {
            workspace: f.workspace.clone(),
            ..Default::default()
        };
        *context.goal_context_version.lock().unwrap() = Some(version);
        let input = serde_json::json!({"memory":{"query":"coral"}});
        let output = tool.call(input.clone(), &context).await.unwrap();
        assert!(!output.is_error, "{}", output.content);
        // A recall of the owner's words and a clean reply is local, so the
        // tracked attempt is local and the native record must say the same.
        assert!(!output.external, "{}", output.content);
        let packet: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert!(packet["confirmed_goal"].is_null());
        assert_eq!(packet["goal_context_revision"], 2);
        s.append(&Record::Message(Message::assistant(vec![Block::ToolUse {
            id: "recall".into(),
            name: "goal_context".into(),
            input,
        }])))
        .unwrap();
        let mut result = Message::user("");
        result.content = vec![Block::ToolResult {
            tool_use_id: "recall".into(),
            content: output.content,
            is_error: false,
        }];
        result.tool_provenance.insert("recall".into(), provenance);
        s.append(&Record::Message(result)).unwrap();
        // A second native request uses exactly the accumulated conversation.
        let t = Session::read(&s.path).unwrap();
        let state = recorded_context(&s);
        let version = state.current().unwrap();
        let binding = DecisionBinding::new(
            state.next_decision(),
            version.revision(),
            &t.convo.messages,
            Some(version.prompt_block()),
            false,
            0,
        )
        .unwrap()
        .with_conversation_messages(&t.convo.messages)
        .unwrap();
        s.append(&Record::GoalContextDecision {
            binding,
            versions: vec![],
        })
        .unwrap();
        s.append(&Record::Message(Message::assistant(vec![Block::text(
            "The earlier authored label was coral.",
        )])))
        .unwrap();
        s.append(&Record::Outcome(RunStats {
            stop_cause: Some(StopCause::Completed),
            turns: 2,
            context_overflows: Some(0),
            ..Default::default()
        }))
        .unwrap();
        s.append(&Record::Taint(Taint {
            private: true,
            untrusted: provenance,
        }))
        .unwrap();
        let second = chat.finish_turn(&turn);
        if provenance {
            assert_eq!(second.status, SessionMemoryStatus::Unavailable);
            continue;
        }
        assert_eq!(second.status, SessionMemoryStatus::Retained, "{second:?}");
        let seal: serde_json::Value = serde_json::from_slice(
            &fs::read(
                f.request
                    .authority
                    .join(format!("{}.chat-seal", second.run_id)),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(seal["payload"]["originals"].as_array().unwrap().len(), 3);
        assert!(seal["payload"]["consulted"]
            .as_array()
            .unwrap()
            .contains(&serde_json::to_value(first.receipt.unwrap().projection).unwrap()));
    }
}

fn web_session(f: &Fixture) -> Session {
    let mut session = f.session();
    session.meta.kind = Some(SessionKind::Web);
    fs::write(
        &session.path,
        format!(
            "{}\n",
            serde_json::to_string(&Record::Meta(session.meta.clone())).unwrap()
        ),
    )
    .unwrap();
    session
}

fn web_record_turn(
    chat: &mut ChatMemory,
    session: &Session,
    text: &str,
    ordinal: u64,
    recorded_source: OwnerSource,
) -> mecha_core::memory_session::chat::ChatMemoryTurn {
    let owner = OwnerInput::new(OwnerSource::AuthenticatedWeb, text.into()).unwrap();
    let turn = chat
        .begin_authenticated_web_turn(session, &owner, ordinal)
        .unwrap();
    let mut history = Session::read(&session.path).unwrap().convo.messages;
    history.push(Message::user(text));
    session
        .append(&Record::Message(Message::user(text)))
        .unwrap();
    let mut state = recorded_context(session);
    let version = state
        .transition(
            CharterSnapshot::disabled(),
            Some(OwnerInput::new(recorded_source, text.into()).unwrap()),
        )
        .unwrap()
        .unwrap();
    let binding = DecisionBinding::new(
        state.next_decision(),
        version.revision(),
        &history,
        Some(version.prompt_block()),
        true,
        0,
    )
    .unwrap()
    .with_conversation_messages(&history)
    .unwrap();
    session
        .append(&Record::GoalContextDecision {
            binding,
            versions: vec![version],
        })
        .unwrap();
    session
        .append(&Record::Message(Message::assistant(vec![Block::text(
            "Authored web response.",
        )])))
        .unwrap();
    session
        .append(&Record::Outcome(RunStats {
            stop_cause: Some(StopCause::Completed),
            turns: 1,
            context_overflows: Some(0),
            ..Default::default()
        }))
        .unwrap();
    session
        .append(&Record::Taint(Taint {
            private: true,
            untrusted: false,
        }))
        .unwrap();
    turn
}

#[test]
fn web_ingress_retains_two_turns_and_resumes_only_through_its_host_door() {
    let f = Fixture::new();
    let s = web_session(&f);
    let mut chat = f
        .memory()
        .attach_authenticated_web_chat(&s, ChatAttachMode::Create)
        .unwrap();
    assert_eq!(chat.next_turn_ordinal().unwrap(), 1);
    f.config(&s);
    let first = web_record_turn(
        &mut chat,
        &s,
        "Keep synthetic folders blue",
        1,
        OwnerSource::AuthenticatedWeb,
    );
    let report = chat.finish_turn(&first);
    assert_eq!(report.status, SessionMemoryStatus::Retained, "{report:?}");
    assert_eq!(report.protocol, "ordinary-chat-memory-v3");
    let before = fs::read(&s.path).unwrap();
    let owner = OwnerInput::new(
        OwnerSource::AuthenticatedWeb,
        "Keep synthetic folders blue".into(),
    )
    .unwrap();
    assert!(
        chat.begin_authenticated_web_turn(&s, &owner, 1).is_err(),
        "completed duplicate ordinal ran twice"
    );
    assert_eq!(fs::read(&s.path).unwrap(), before);
    assert_eq!(chat.next_turn_ordinal().unwrap(), 2);
    let second = web_record_turn(
        &mut chat,
        &s,
        "Use synthetic green folders instead",
        2,
        OwnerSource::AuthenticatedWeb,
    );
    let second_report = chat.finish_turn(&second);
    assert_eq!(
        second_report.status,
        SessionMemoryStatus::Retained,
        "{second_report:?}"
    );
    let seal: serde_json::Value = serde_json::from_slice(
        &fs::read(
            f.request
                .authority
                .join(format!("{}.chat-seal", second.run_id())),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        seal["payload"]["identity"]["check_version"],
        "ordinary-chat-observation-v2"
    );
    assert_eq!(
        seal["payload"]["identity"]["evaluator_version"],
        "ordinary-chat-recorder-v2"
    );
    assert_eq!(
        seal["payload"]["chat_context"]["previous_turn"],
        first.run_id()
    );
    assert_eq!(
        seal["payload"]["chat_context"]["versions"][0]["owner_inputs"][0]["source"],
        "authenticated_web"
    );
    assert!(seal["payload"]["criterion"].is_null());
    drop(chat);
    assert!(f.memory().attach_chat(&s, ChatAttachMode::Resume).is_err());
    assert!(f
        .memory()
        .attach_chat_with_graph(&s, ChatAttachMode::Resume, None)
        .is_err());
    let resumed = f
        .memory()
        .attach_authenticated_web_chat(&s, ChatAttachMode::Resume)
        .unwrap();
    assert_eq!(resumed.next_turn_ordinal().unwrap(), 3);
    assert!(Session::load(&s.path).is_err());
}

#[test]
fn web_ingress_rejects_wrong_kind_source_and_ordinal_before_reserving_a_turn() {
    let f = Fixture::new();
    let cli = f.session();
    assert!(f
        .memory()
        .attach_authenticated_web_chat(&cli, ChatAttachMode::Create)
        .is_err());
    let s = web_session(&f);
    assert!(f.memory().attach_chat(&s, ChatAttachMode::Create).is_err());
    let mut chat = f
        .memory()
        .attach_authenticated_web_chat(&s, ChatAttachMode::Create)
        .unwrap();
    f.config(&s);
    let before = fs::read(&s.path).unwrap();
    let files_before = fs::read_dir(&f.request.authority).unwrap().count();
    let cli_owner = OwnerInput::new(OwnerSource::CliChat, "Authored input".into()).unwrap();
    let web_owner =
        OwnerInput::new(OwnerSource::AuthenticatedWeb, "Authored input".into()).unwrap();
    assert!(chat
        .begin_authenticated_web_turn(&s, &cli_owner, 1)
        .is_err());
    assert!(chat.begin_turn(&s, &cli_owner).is_err());
    assert!(chat.begin_turn(&s, &web_owner).is_err());
    for ordinal in [0, 2, u64::MAX] {
        assert!(chat
            .begin_authenticated_web_turn(&s, &web_owner, ordinal)
            .is_err());
    }
    assert_eq!(fs::read(&s.path).unwrap(), before);
    assert_eq!(
        fs::read_dir(&f.request.authority).unwrap().count(),
        files_before
    );
    assert_eq!(chat.next_turn_ordinal().unwrap(), 1);
    let _turn = chat
        .begin_authenticated_web_turn(&s, &web_owner, 1)
        .unwrap();
    assert!(
        chat.next_turn_ordinal().is_err(),
        "unfinished turn cannot advertise another ordinal"
    );
}

#[test]
fn web_ingress_rejects_wrong_native_owner_even_with_matching_text() {
    let f = Fixture::new();
    let s = web_session(&f);
    let mut chat = f
        .memory()
        .attach_authenticated_web_chat(&s, ChatAttachMode::Create)
        .unwrap();
    f.config(&s);
    let turn = web_record_turn(
        &mut chat,
        &s,
        "Authored same words",
        1,
        OwnerSource::CliChat,
    );
    assert_eq!(
        chat.finish_turn(&turn).status,
        SessionMemoryStatus::Unavailable
    );
    assert!(chat.next_turn_ordinal().is_err());
}

#[test]
fn web_ingress_requires_closed_private_ingress_and_excludes_graph_authority() {
    for mutation in 0..7 {
        let f = Fixture::new();
        let s = web_session(&f);
        let chat = f
            .memory()
            .attach_authenticated_web_chat(&s, ChatAttachMode::Create)
            .unwrap();
        drop(chat);
        let path = fs::read_dir(&f.request.authority)
            .unwrap()
            .map(|p| p.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "chat"))
            .unwrap();
        let mut wire: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            wire["ingress"],
            serde_json::json!({"session_kind":"web","owner_source":"authenticated_web"})
        );
        match mutation {
            0 => {
                wire.as_object_mut().unwrap().remove("ingress");
            }
            1 => wire["ingress"] = serde_json::Value::Null,
            2 => wire["ingress"]["session_kind"] = serde_json::json!("chat"),
            3 => wire["ingress"]["owner_source"] = serde_json::json!("cli_chat"),
            4 => wire["ingress"]["extra"] = serde_json::json!(true),
            5 => wire["graph_surface"] = serde_json::Value::Null,
            6 => {}
            _ => unreachable!(),
        }
        let mut changed = serde_json::to_string(&wire).unwrap();
        if mutation == 6 {
            changed.insert_str(1, "\"ingress\":null,");
        }
        fs::write(&path, changed).unwrap();
        assert!(
            f.memory()
                .attach_authenticated_web_chat(&s, ChatAttachMode::Resume)
                .is_err(),
            "accepted {mutation}"
        );
    }
}

#[test]
fn web_ingress_sealed_retry_is_storage_only_and_forget_survives_source_loss() {
    let f = Fixture::new();
    let s = web_session(&f);
    let mut chat = f
        .memory()
        .attach_authenticated_web_chat(&s, ChatAttachMode::Create)
        .unwrap();
    f.config(&s);
    let turn = web_record_turn(
        &mut chat,
        &s,
        "Authored retained evidence",
        1,
        OwnerSource::AuthenticatedWeb,
    );
    let receipt_path = f
        .request
        .authority
        .join(format!("{}.chat-retained", turn.run_id()));
    mecha_core::create_private_dir(&receipt_path).unwrap();
    assert_eq!(chat.finish_turn(&turn).status, SessionMemoryStatus::Pending);
    assert!(chat.next_turn_ordinal().is_err());
    drop(chat);
    fs::remove_dir(receipt_path).unwrap();
    fs::remove_dir(&f.workspace).unwrap();
    let recovery = SessionMemory::open_for_recovery(&f.request).unwrap();
    let before = fs::read(&s.path).unwrap();
    let report = recovery.retry_chat(turn.run_id());
    assert_eq!(report.status, SessionMemoryStatus::Retained, "{report:?}");
    assert_eq!(report.protocol, "ordinary-chat-memory-v3");
    assert_eq!(recovery.retry_chat(turn.run_id()).receipt, report.receipt);
    assert_eq!(fs::read(&s.path).unwrap(), before);
    fs::remove_file(&s.path).unwrap();
    assert_eq!(
        recovery.retry_chat(turn.run_id()).status,
        SessionMemoryStatus::Unavailable
    );
    recovery.forget_chat(turn.run_id()).unwrap();
}

#[test]
fn web_ingress_does_not_reencode_cli_attachments_or_admit_web_metadata_into_them() {
    #[derive(serde::Serialize, serde::Deserialize)]
    struct OriginalAttachment {
        protocol: String,
        request: SessionMemoryRequest,
        authority: String,
        session_id: String,
        source: PathBuf,
        workspace: PathBuf,
        device: u64,
        inode: u64,
        initial_length: u64,
        initial_digest: String,
        nonce: String,
    }
    for graph in [false, true] {
        let f = Fixture::new();
        let s = f.session();
        let mut chat = if graph {
            f.memory()
                .attach_chat_with_graph(
                    &s,
                    ChatAttachMode::Create,
                    Some(authored_graph_surface(&f)),
                )
                .unwrap()
        } else {
            f.memory().attach_chat(&s, ChatAttachMode::Create).unwrap()
        };
        let web_owner =
            OwnerInput::new(OwnerSource::AuthenticatedWeb, "Authored web input".into()).unwrap();
        assert!(chat
            .begin_authenticated_web_turn(&s, &web_owner, 1)
            .is_err());
        assert!(chat.begin_turn(&s, &web_owner).is_err());
        drop(chat);
        let path = fs::read_dir(&f.request.authority)
            .unwrap()
            .map(|p| p.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "chat"))
            .unwrap();
        let original = fs::read(&path).unwrap();
        let mut wire: serde_json::Value = serde_json::from_slice(&original).unwrap();
        assert!(wire.get("ingress").is_none());
        let old: OriginalAttachment = serde_json::from_value(wire.clone()).unwrap();
        let old_bytes = if graph {
            let mut old_wire = serde_json::to_value(old).unwrap();
            old_wire["graph_surface"] = wire["graph_surface"].clone();
            serde_json::to_vec(&old_wire).unwrap()
        } else {
            serde_json::to_vec(&old).unwrap()
        };
        assert_eq!(original, old_bytes, "CLI attachment byte encoding changed");
        assert!(f
            .memory()
            .attach_authenticated_web_chat(&s, ChatAttachMode::Resume)
            .is_err());
        wire["ingress"] = serde_json::Value::Null;
        fs::write(&path, serde_json::to_vec(&wire).unwrap()).unwrap();
        let refused = if graph {
            f.memory()
                .attach_chat_with_graph(&s, ChatAttachMode::Resume, None)
                .is_err()
        } else {
            f.memory().attach_chat(&s, ChatAttachMode::Resume).is_err()
        };
        assert!(refused);
    }
}

struct HarnessNoticeProvider {
    replies: std::sync::Mutex<std::collections::VecDeque<Message>>,
}
#[async_trait::async_trait]
impl mecha_core::provider::Provider for HarnessNoticeProvider {
    fn id(&self) -> &str {
        "fixture"
    }
    fn default_model(&self) -> &str {
        "fixture"
    }
    fn local(&self) -> bool {
        true
    }
    async fn complete(
        &self,
        _: &mecha_core::message::CompletionRequest,
        _: Option<&mecha_core::provider::StreamSink>,
    ) -> anyhow::Result<mecha_core::message::CompletionResponse> {
        let message = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected request");
        let stop_reason = if message.tool_uses().is_empty() {
            mecha_core::message::StopReason::EndTurn
        } else {
            mecha_core::message::StopReason::ToolUse
        };
        Ok(mecha_core::message::CompletionResponse {
            message,
            stop_reason,
            usage: Default::default(),
            refusal: None,
            model: "fixture".into(),
            malformed_tool_args: 0,
        })
    }
}

async fn retained_agent_harness_notice(web: bool, empty_retry: bool) {
    use mecha_core::agent::Agent;
    use mecha_core::config::PermissionMode;
    use mecha_core::tool::{ModeApprover, Registry, ToolCtx};
    use std::sync::Arc;
    let f = Fixture::new();
    fs::write(
        f.workspace.join("note.txt"),
        "authored unchanged observation",
    )
    .unwrap();
    let s = if web { web_session(&f) } else { f.session() };
    let mut chat = if web {
        f.memory()
            .attach_authenticated_web_chat(&s, ChatAttachMode::Create)
    } else {
        f.memory().attach_chat(&s, ChatAttachMode::Create)
    }
    .unwrap();
    f.config(&s);
    let first = if web {
        let turn = web_record_turn(
            &mut chat,
            &s,
            "Earlier exact owner input",
            1,
            OwnerSource::AuthenticatedWeb,
        );
        chat.finish_turn(&turn)
    } else {
        f.turn(
            &mut chat,
            &s,
            "Earlier exact owner input",
            "Keep authored provenance",
        )
    };
    assert_eq!(first.status, SessionMemoryStatus::Retained, "{first:?}");
    let prior = Session::read(&s.path).unwrap().convo.messages;
    let prior_decisions = recorded_context(&s).decisions().len();
    // An owner is allowed to type harness-looking text. Its first block must
    // remain owner evidence even though later blocks use the same voice.
    let prompt = if empty_retry {
        "Your previous turn ended without producing anything — the token budget went entirely to reasoning before you began your answer. Do not start the task over and do not re-derive what you already worked out. Either give your answer now, briefly, using what you already know, or make the single next tool call. Keep your reasoning short this turn.".to_string()
    } else {
        mecha_core::boredom::Rung::Change.notice("fs_read", &Default::default())
    };
    let prompt = prompt.as_str();
    let owner = OwnerInput::new(
        if web {
            OwnerSource::AuthenticatedWeb
        } else {
            OwnerSource::CliChat
        },
        prompt.into(),
    )
    .unwrap();
    let turn = if web {
        chat.begin_authenticated_web_turn(&s, &owner, 2)
    } else {
        chat.begin_turn(&s, &owner)
    }
    .unwrap();
    let mut convo = Session::read(&s.path).unwrap().convo;
    convo.push(Message::user(prompt));
    s.append(&Record::Message(Message::user(prompt))).unwrap();
    let before = convo.messages.clone();
    let mut registry = Registry::new();
    registry.insert(Arc::new(mecha_core::tool::builtin::FsRead));
    registry.insert(chat.goal_memory(&turn).unwrap());
    let mut replies = Vec::new();
    if empty_retry {
        replies.push(Message::assistant(Vec::new()));
    } else {
        for index in 0..3 {
            replies.push(Message::assistant(vec![Block::ToolUse {
                id: format!("notice-read-{index}"),
                name: "fs_read".into(),
                input: serde_json::json!({"path":"note.txt"}),
            }]));
        }
    }
    replies.push(Message::assistant(vec![Block::text(
        "Authored final answer.",
    )]));
    let agent = Agent::new(
        Box::new(HarnessNoticeProvider {
            replies: std::sync::Mutex::new(replies.into()),
        }),
        registry,
        Arc::new(ModeApprover {
            mode: PermissionMode::ReadOnly,
        }),
        ToolCtx {
            workspace: f.workspace.clone(),
            ..Default::default()
        },
        mecha_core::config::AgentConfig {
            max_turns: 12,
            boredom: true,
            ..Default::default()
        },
        None,
    )
    .unwrap()
    .with_dynamic_goals();
    let cx = agent
        .context()
        .as_ref()
        .clone()
        .with_goal_ingress(vec![owner], Some(Arc::new(s.clone())));
    let outcome = agent.run_in(&cx, &mut convo, None).await.unwrap();
    assert_eq!(outcome.stop_cause, StopCause::Completed);
    assert_eq!(&convo.messages[..prior.len()], prior.as_slice());
    assert_eq!(convo.messages[prior.len()].content[0], Block::text(prompt));
    if empty_retry {
        assert_eq!(convo.messages[prior.len()].content.len(), 2);
        assert_eq!(
            convo.messages.len(),
            before.len() + 1,
            "empty reply is not persisted"
        );
    } else {
        assert_eq!(outcome.boredom_notices, 1);
        assert!(convo.messages.iter().any(|m| m.content.iter().any(|b| matches!(b, Block::Text { text } if text.starts_with("Nothing is being learned here: `fs_read`")))));
    }
    let bindings: Vec<_> = recorded_context(&s)
        .decisions()
        .iter()
        .skip(prior_decisions)
        .cloned()
        .collect();
    assert_eq!(bindings[0].conversation_messages().unwrap(), before);
    assert_eq!(bindings.len(), if empty_retry { 2 } else { 4 });
    if empty_retry {
        let next = bindings[1].conversation_messages().unwrap();
        assert_eq!(next.len(), before.len());
        assert_eq!(next.last().unwrap().content.len(), 2);
    }
    s.record_run(&before, &convo).unwrap();
    s.record_outcome(&outcome).unwrap();
    s.append(&Record::Taint(convo.taint)).unwrap();
    let source = fs::read_to_string(&s.path).unwrap();
    assert_eq!(
        Session::read(&s.path).unwrap().convo.messages,
        convo.messages
    );
    if empty_retry {
        assert!(source.lines().any(|line| matches!(
            serde_json::from_str::<Record>(line),
            Ok(Record::Extend { .. })
        )));
    }
    let report = chat.finish_turn(&turn);
    assert_eq!(
        report.status,
        SessionMemoryStatus::Retained,
        "web={web} empty={empty_retry}: {report:?}"
    );
    let seal: serde_json::Value = serde_json::from_slice(
        &fs::read(
            f.request
                .authority
                .join(format!("{}.chat-seal", report.run_id)),
        )
        .unwrap(),
    )
    .unwrap();
    let originals = seal["payload"]["originals"].as_array().unwrap();
    let owners: Vec<_> = originals
        .iter()
        .filter(|p| p["kind"] == "UserRequest")
        .collect();
    assert_eq!(owners.len(), 1);
    assert_eq!(owners[0]["text"], prompt);
    assert_eq!(owners[0]["pointer"], "/content/0/text");
    assert!(originals
        .iter()
        .all(|p| p["text"] != "Earlier exact owner input"));
    assert_eq!(
        originals
            .iter()
            .filter(|p| p["kind"] == "ToolResult")
            .count(),
        if empty_retry { 0 } else { 3 }
    );
    assert_eq!(
        recorded_context(&s)
            .decisions()
            .iter()
            .skip(prior_decisions)
            .cloned()
            .collect::<Vec<_>>(),
        bindings,
        "retention must not rewrite replay bindings"
    );
}

#[tokio::test]
async fn native_empty_retry_notice_retains_exact_owner_and_decision_histories() {
    for web in [false, true] {
        retained_agent_harness_notice(web, true).await;
    }
}

#[tokio::test]
async fn native_boredom_notice_retains_exact_owner_and_decision_histories() {
    for web in [false, true] {
        retained_agent_harness_notice(web, false).await;
    }
}

#[test]
fn native_inline_notices_cannot_rewrite_history_or_admit_arbitrary_user_text() {
    let retry = "Your previous turn ended without producing anything — the token budget went entirely to reasoning before you began your answer. Do not start the task over and do not re-derive what you already worked out. Either give your answer now, briefly, using what you already know, or make the single next tool call. Keep your reasoning short this turn.";
    for mode in [
        "unknown",
        "canonical-prefix",
        "wrong-tail",
        "rewrite",
        "edited-owner",
    ] {
        let f = Fixture::new();
        let s = f.session();
        let mut chat = f.memory().attach_chat(&s, ChatAttachMode::Create).unwrap();
        f.config(&s);
        assert_eq!(
            f.turn(&mut chat, &s, "Prior owner", "Keep provenance")
                .status,
            SessionMemoryStatus::Retained
        );
        let owner = OwnerInput::new(OwnerSource::CliChat, "Current owner".into()).unwrap();
        let turn = chat.begin_turn(&s, &owner).unwrap();
        let mut history = Session::read(&s.path).unwrap().convo.messages;
        history.push(Message::user(owner.text()));
        s.append(&Record::Message(Message::user(owner.text())))
            .unwrap();
        let mut context = recorded_context(&s);
        let version = context
            .transition(CharterSnapshot::disabled(), Some(owner))
            .unwrap()
            .unwrap();
        let first = DecisionBinding::new(
            context.next_decision(),
            version.revision(),
            &history,
            Some(version.prompt_block()),
            true,
            0,
        )
        .unwrap()
        .with_conversation_messages(&history)
        .unwrap();
        context.bind(first.clone()).unwrap();
        s.append(&Record::GoalContextDecision {
            binding: first,
            versions: vec![version.clone()],
        })
        .unwrap();
        let text = match mode {
            "unknown" => "An unrecognized owner-like instruction".to_string(),
            "canonical-prefix" => format!(
                "{} Extra untrusted instruction.",
                mecha_core::boredom::Rung::Change.notice("fs_read", &Default::default())
            ),
            _ => retry.to_string(),
        };
        let index = if mode == "wrong-tail" {
            0
        } else {
            history.len() - 1
        };
        history[index].content.push(Block::text(text.clone()));
        if mode == "edited-owner" {
            history[index].content[0] = Block::text("Substituted owner");
        }
        if mode == "rewrite" {
            s.append(&Record::Rewrite {
                messages: history.clone(),
            })
            .unwrap();
        } else {
            s.append(&Record::Extend {
                index,
                blocks: vec![Block::text(text)],
            })
            .unwrap();
        }
        // Recompute a structurally valid binding: rejection must concern the
        // actual history transition, not merely a mismatched message digest.
        let second = DecisionBinding::new(
            context.next_decision(),
            version.revision(),
            &history,
            Some(version.prompt_block()),
            false,
            0,
        )
        .unwrap()
        .with_conversation_messages(&history)
        .unwrap();
        s.append(&Record::GoalContextDecision {
            binding: second,
            versions: vec![],
        })
        .unwrap();
        s.append(&Record::Message(Message::assistant(vec![Block::text(
            "Final answer",
        )])))
        .unwrap();
        s.append(&Record::Outcome(RunStats {
            stop_cause: Some(StopCause::Completed),
            turns: 2,
            context_overflows: Some(0),
            ..Default::default()
        }))
        .unwrap();
        s.append(&Record::Taint(Taint {
            private: true,
            untrusted: false,
        }))
        .unwrap();
        assert_eq!(
            chat.finish_turn(&turn).status,
            SessionMemoryStatus::Unavailable,
            "{mode}"
        );
        assert!(
            !f.request
                .authority
                .join(format!("{}.chat-seal", turn.run_id()))
                .exists(),
            "{mode} sealed unsupported content"
        );
    }
}
