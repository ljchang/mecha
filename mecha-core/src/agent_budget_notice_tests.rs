// A missing delivery, incorrect effective ceiling, or provenance misclassification
// must fail these tests; the provider only replaces external inference.
const TEST_BUDGET_NOTICE_TWO: &str = "Run budget: two model turns remain, including this one. Preserve time for the requested deliverable and an honest account of unresolved requirements. Continue gathering evidence when a concrete available source or new result justifies it. Otherwise finish from supported evidence and state what is missing, using any fallback the user specified. Do not invent missing facts.";
const TEST_BUDGET_NOTICE_ONE: &str = "Run budget: one model turn remains, including this one. Preserve time for the requested deliverable and an honest account of unresolved requirements. Continue gathering evidence when a concrete available source or new result justifies it. Otherwise finish from supported evidence and state what is missing, using any fallback the user specified. Do not invent missing facts.";

fn budget_notice_count(messages: &[Message]) -> usize {
    messages
        .iter()
        .flat_map(|m| &m.content)
        .filter(|b| matches!(b, Block::Text { text } if text.starts_with("Run budget:")))
        .count()
}

fn budget_notice_echo(i: u32) -> CompletionResponse {
    assistant(
        vec![Block::ToolUse {
            id: format!("echo-{i}"),
            name: "echo".into(),
            input: json!({"value":i}),
        }],
        StopReason::ToolUse,
    )
}

#[tokio::test]
async fn budget_notice_reaches_seventh_wire_request_and_preserves_late_read_write() {
    let root = std::env::temp_dir().join(format!("mecha-budget-notice-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("input.txt"), "first\nuseful evidence\nthird\n").unwrap();
    let mut script: Vec<_> = (0..6).map(budget_notice_echo).collect();
    script.push(assistant(
        vec![Block::ToolUse {
            id: "late-read".into(),
            name: "fs_read".into(),
            input: json!({"path":"input.txt","offset":2,"limit":1}),
        }],
        StopReason::ToolUse,
    ));
    script.push(assistant(
        vec![Block::ToolUse {
            id: "late-write".into(),
            name: "fs_write".into(),
            input: json!({"path":"output.txt","content":"useful evidence"}),
        }],
        StopReason::ToolUse,
    ));
    let (mut agent, provider) = agent_with_tools(
        script,
        vec![
            Arc::new(EchoTool),
            Arc::new(crate::tool::builtin::FsRead),
            Arc::new(crate::tool::builtin::FsWrite),
        ],
        PermissionMode::Allow,
    );
    agent.cfg.max_turns = 8;
    agent.cfg.force_final_answer = false;
    agent.ctx_mut().workspace = root.clone();
    let cx = (*agent.cx).clone().with_budget_notice(true);
    let mut convo = Conversation::user("Read the useful line and save the requested output.");
    let result = agent.run_in(&cx, &mut convo, None).await.unwrap();
    assert_eq!(result.stop_cause, StopCause::MaxTurns);
    assert_eq!(result.turns, 8);
    assert_eq!(
        std::fs::read_to_string(root.join("output.txt")).unwrap(),
        "useful evidence"
    );
    assert!(result.taint.private && !result.taint.untrusted);
    let seen = provider.seen.lock().unwrap();
    assert_eq!(seen.len(), 8, "advice cannot purchase a ninth request");
    let encoder = crate::provider::openai::OpenAiCompatible::from_local_config(
        &crate::config::ProviderConfig {
            kind: "local".into(),
            base_url: Some("http://127.0.0.1:1".into()),
            ..Default::default()
        },
    )
    .unwrap();
    for (i, request) in seen.iter().enumerate() {
        let wire = encoder.body_for_test(request);
        assert_eq!(
            wire.to_string().contains(TEST_BUDGET_NOTICE_TWO),
            i >= 6,
            "request {}",
            i + 1
        );
        assert_eq!(budget_notice_count(&request.messages), usize::from(i >= 6));
        assert_eq!(
            serde_json::to_value(&request.tools).unwrap(),
            serde_json::to_value(&seen[0].tools).unwrap()
        );
        assert!(matches!(
            request.tool_choice,
            crate::message::ToolChoice::Auto
        ));
        assert!(request.messages.windows(2).all(|m| m[0].role != m[1].role));
    }
    assert_eq!(budget_notice_count(&convo.messages), 1);
    let carrier = convo
        .messages
        .iter()
        .find(|m| budget_notice_count(std::slice::from_ref(m)) > 0)
        .unwrap();
    assert!(carrier
        .content
        .iter()
        .any(|b| matches!(b, Block::ToolResult { .. })));
    assert!(owner_text(carrier).is_empty());
    assert!(crate::learning::extract_interventions(&convo.messages).is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn budget_notice_uses_effective_override_short_caps_and_defaults_off() {
    for (enabled, cap, override_cap, first) in [
        (true, 8, None, Some(7)),
        (true, 8, Some(3), Some(2)),
        (true, 1, Some(3), Some(2)),
        (true, 8, Some(0), None),
        (true, 8, Some(1), Some(1)),
        (true, 8, Some(2), Some(1)),
        (false, 8, Some(2), None),
    ] {
        let (mut agent, provider) = agent_with(
            (0..8).map(budget_notice_echo).collect(),
            PermissionMode::Allow,
        );
        agent.cfg.max_turns = cap;
        agent.cfg.force_final_answer = false;
        let mut cx = (*agent.cx).clone();
        assert!(!cx.budget_notice, "ordinary runs remain unchanged");
        if enabled {
            cx = cx.with_budget_notice(true);
        }
        cx.budget.max_turns = override_cap;
        let mut convo = Conversation::user("Continue reading until stopped.");
        let outcome = agent.run_in(&cx, &mut convo, None).await.unwrap();
        assert_eq!(outcome.stop_cause, StopCause::MaxTurns);
        let seen = provider.seen.lock().unwrap();
        assert_eq!(seen.len(), override_cap.unwrap_or(cap) as usize);
        assert_eq!(
            seen.iter()
                .position(|r| budget_notice_count(&r.messages) > 0)
                .map(|i| i + 1),
            first
        );
        assert_eq!(
            budget_notice_count(&convo.messages),
            usize::from(first.is_some())
        );
        if override_cap == Some(1) {
            assert!(seen[0].messages.iter().any(|m| m
                .content
                .iter()
                .any(|b| matches!(b,Block::Text{text} if text==TEST_BUDGET_NOTICE_ONE))));
        }
    }
}

#[tokio::test]
async fn budget_notice_cannot_override_cancellation_tokens_or_cost() {
    for reason in ["cancel", "tokens", "cost"] {
        let (mut agent, provider) = agent_with(vec![budget_notice_echo(0)], PermissionMode::Allow);
        agent.cfg.max_turns = 1;
        agent.cfg.force_final_answer = false;
        agent.pricing = Some(Pricing {
            input_per_mtok: 1.0,
            output_per_mtok: 1.0,
            ..Default::default()
        });
        let mut cx = (*agent.cx).clone().with_budget_notice(true);
        let want = match reason {
            "cancel" => {
                let cancel = CancellationToken::new();
                cancel.cancel();
                cx = cx.with_cancel(cancel);
                StopCause::Interrupted
            }
            "tokens" => {
                cx.budget.max_output_tokens = Some(0);
                StopCause::OutputTokenBudget
            }
            _ => {
                cx.budget.max_cost_usd = Some(0.0);
                StopCause::CostBudget
            }
        };
        let mut convo = Conversation::user("Do the task.");
        let outcome = agent.run_in(&cx, &mut convo, None).await.unwrap();
        assert_eq!(outcome.stop_cause, want);
        assert!(provider.seen.lock().unwrap().is_empty());
        assert_eq!(budget_notice_count(&convo.messages), 0);
    }
}

#[test]
fn budget_notice_is_exact_harness_voice_preserving_owner_corrections() {
    for notice in [TEST_BUDGET_NOTICE_ONE, TEST_BUDGET_NOTICE_TWO] {
        assert!(is_harness_voice(notice));
        let mut messages = vec![Message::user("Use the newly supplied evidence.")];
        append_user_text(&mut messages, notice.into());
        assert_eq!(owner_text(&messages[0]), "Use the newly supplied evidence.");
        let rendered = crate::compact::render_for_summary(&messages, 4096);
        assert!(rendered.contains(&format!("[harness] {notice}")));
        assert!(!rendered.contains(&format!("[user] {notice}")));
        let quoted = format!("{notice} The owner adds a correction.");
        assert!(
            !is_harness_voice(&quoted),
            "a shared prefix must not swallow owner text"
        );
        assert_eq!(owner_text(&Message::user(&quoted)), quoted);
    }
}

#[tokio::test]
async fn budget_notice_is_not_reinserted_when_compaction_removes_it() {
    let mut script: Vec<_> = (0..7).map(budget_notice_echo).collect();
    // Only the penultimate response crosses the threshold. A summary then
    // removes the delivered block before the final task request.
    script[6].usage.input_tokens = 1000;
    script.push(assistant(
        vec![Block::text(
            "Earlier work read the fixture; one step remains.",
        )],
        StopReason::EndTurn,
    ));
    script.push(budget_notice_echo(7));
    let (mut agent, provider) = agent_with(script, PermissionMode::Allow);
    agent.cfg.max_turns = 8;
    agent.cfg.force_final_answer = false;
    agent.cfg.compact_at_tokens = Some(100);
    agent.cfg.compact_keep_recent = 2;
    agent.cfg.compact_validate = false;
    agent.cfg.predictive_compaction = false;
    let cx = (*agent.cx).clone().with_budget_notice(true);
    let mut convo = Conversation::user("Read the synthetic fixture in sequence.");
    let outcome = agent.run_in(&cx, &mut convo, None).await.unwrap();
    assert_eq!(outcome.stop_cause, StopCause::MaxTurns);
    assert_eq!(outcome.turns, 8);
    assert_eq!(outcome.compactions, 1);
    let seen = provider.seen.lock().unwrap();
    assert_eq!(
        seen.len(),
        9,
        "eight task calls plus the existing compaction call"
    );
    assert_eq!(budget_notice_count(&seen[6].messages), 1);
    assert!(
        seen[7].tools.is_empty(),
        "the intervening call is the summary"
    );
    assert!(seen[7]
        .messages
        .iter()
        .any(|m| m.text().contains("[harness] Run budget:")));
    assert_eq!(
        budget_notice_count(&seen[8].messages),
        0,
        "the run-local once flag survives a cut"
    );
    assert!(convo
        .rewritten
        .iter()
        .any(|old| budget_notice_count(old) == 1));
}

#[tokio::test]
async fn budget_notice_late_write_keeps_phase_permission_and_path_guards() {
    let root = std::env::temp_dir().join(format!("mecha-budget-guard-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("workspace")).unwrap();
    for guard in ["phase", "permission", "path"] {
        let destination = if guard == "path" {
            "../escaped.txt"
        } else {
            "output.txt"
        };
        let write = assistant(
            vec![Block::ToolUse {
                id: "write".into(),
                name: "fs_write".into(),
                input: json!({"path":destination,"content":"unsupported output"}),
            }],
            StopReason::ToolUse,
        );
        let (mut agent, provider) = agent_with_tools(
            vec![budget_notice_echo(0), write],
            vec![Arc::new(EchoTool), Arc::new(crate::tool::builtin::FsWrite)],
            if guard == "permission" {
                PermissionMode::ReadOnly
            } else {
                PermissionMode::Allow
            },
        );
        agent.cfg.max_turns = 2;
        agent.cfg.force_final_answer = false;
        agent.ctx_mut().workspace = root.join("workspace");
        let mut cx = (*agent.cx).clone().with_budget_notice(true);
        if guard == "phase" {
            cx = cx.with_phase(Phase::Plan);
        }
        let mut convo = Conversation::user("Write the requested output.");
        let outcome = agent.run_in(&cx, &mut convo, None).await.unwrap();
        assert_eq!(outcome.stop_cause, StopCause::MaxTurns);
        assert_eq!(provider.seen.lock().unwrap().len(), 2);
        assert_eq!(budget_notice_count(&convo.messages), 1);
        let write = outcome
            .tool_calls
            .iter()
            .find(|c| c.name == "fs_write")
            .unwrap();
        assert!(
            write.denied || write.is_error,
            "late advice must not bypass {guard}"
        );
        assert!(!root.join("workspace/output.txt").exists());
        assert!(!root.join("escaped.txt").exists());
    }
    std::fs::remove_dir_all(root).unwrap();
}
