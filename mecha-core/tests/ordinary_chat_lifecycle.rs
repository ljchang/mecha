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
    assert!(output.external, "{}", output.content);
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
        untrusted: true,
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
