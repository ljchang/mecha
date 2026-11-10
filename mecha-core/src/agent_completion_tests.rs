mod completion_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    fn incomplete() -> CompletionObservation {
        CompletionObservation {
            readiness: CompletionReadiness::Incomplete,
            feedback: Some(CompletionFeedback {
                required_read_missing: false,
                artifact: Some(CompletionArtifactIssue::Missing),
                do_not_repeat_read: false,
            }),
        }
    }
    fn ready() -> CompletionObservation {
        CompletionObservation {
            readiness: CompletionReadiness::Ready,
            feedback: None,
        }
    }
    enum Action {
        Return(CompletionObservation),
        Steer(Arc<Mutex<VecDeque<String>>>, CompletionObservation),
        Wait(Duration, CompletionObservation),
        Block(Duration, CompletionObservation),
        Cancel(CancellationToken, CompletionObservation),
    }
    struct Observer {
        actions: Mutex<VecDeque<Action>>,
        reports: Mutex<Vec<CompletionReport>>,
        calls: AtomicUsize,
        dropped: Arc<AtomicBool>,
    }
    impl Observer {
        fn new(actions: Vec<Action>) -> Arc<Self> {
            Arc::new(Self {
                actions: Mutex::new(actions.into()),
                reports: Mutex::new(vec![]),
                calls: AtomicUsize::new(0),
                dropped: Arc::new(AtomicBool::new(false)),
            })
        }
        fn report(&self) -> CompletionReport {
            self.reports.lock().unwrap().last().unwrap().clone()
        }
    }
    struct DropFlag(Arc<AtomicBool>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    #[async_trait]
    impl CompletionObserver for Observer {
        async fn observe(&self) -> CompletionObservation {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let action = self
                .actions
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Action::Return(incomplete()));
            let _guard = DropFlag(self.dropped.clone());
            match action {
                Action::Return(observation) => observation,
                Action::Steer(queue, observation) => {
                    queue
                        .lock()
                        .unwrap()
                        .push_back("Use the alternate format.".into());
                    observation
                }
                Action::Wait(delay, observation) => {
                    tokio::time::sleep(delay).await;
                    observation
                }
                Action::Block(delay, observation) => {
                    std::thread::sleep(delay);
                    observation
                }
                Action::Cancel(token, observation) => {
                    token.cancel();
                    observation
                }
            }
        }
        fn record(&self, report: &CompletionReport) {
            self.reports.lock().unwrap().push(report.clone());
        }
    }
    fn answer() -> CompletionResponse {
        assistant(vec![Block::text("An answer.")], StopReason::EndTurn)
    }
    fn conversation() -> Conversation {
        Conversation::from(vec![Message::user("Save the result.")])
    }
    fn context(agent: &Agent, observer: &Arc<Observer>) -> RunContext {
        agent
            .context()
            .as_ref()
            .clone()
            .with_completion_observer(observer.clone())
    }
    fn notices(convo: &Conversation) -> usize {
        convo
            .messages
            .iter()
            .filter(|m| m.harness && m.role == Role::User)
            .count()
    }

    #[tokio::test]
    async fn completion_repair_continues_original_loop() {
        let (agent, provider) = agent_with(
            vec![
                answer(),
                assistant(
                    vec![Block::ToolUse {
                        id: "repair".into(),
                        name: "echo".into(),
                        input: json!({"text":"saved"}),
                    }],
                    StopReason::ToolUse,
                ),
                assistant(vec![Block::text("Saved.")], StopReason::EndTurn),
            ],
            PermissionMode::Allow,
        );
        let observer = Observer::new(vec![Action::Return(incomplete()), Action::Return(ready())]);
        let cx = context(&agent, &observer);
        let mut convo = conversation();
        convo.taint = Taint {
            private: true,
            untrusted: true,
        };
        let outcome = agent.run_in(&cx, &mut convo, None).await.unwrap();
        assert_eq!(
            outcome.turns, 3,
            "normal answer must allow the missing operation within the same run"
        );
        assert_eq!(outcome.tool_calls.len(), 1);
        assert_eq!(outcome.usage.output_tokens, 15);
        assert!(outcome.taint.private && outcome.taint.untrusted);
        assert!(convo.taint.private && convo.taint.untrusted);
        let sent = provider.seen.lock().unwrap();
        assert_eq!(sent.len(), 3);
        assert!(sent[1].messages.last().unwrap().harness);
        assert!(is_harness_voice(&sent[1].messages.last().unwrap().text()));
        assert_eq!(notices(&convo), 1);
        assert_eq!(observer.report().first, Some(incomplete()));
        assert_eq!(observer.report().latest, Some(ready()));
        assert!(observer.report().feedback_delivered);
    }

    #[tokio::test]
    async fn completion_never_delivers_a_second_notice() {
        let (agent, _) = agent_with(vec![answer(), answer()], PermissionMode::Allow);
        let observer = Observer::new(vec![]);
        let mut convo = conversation();
        let result = agent
            .run_in(&context(&agent, &observer), &mut convo, None)
            .await
            .unwrap();
        assert_eq!(result.turns, 2);
        assert_eq!(notices(&convo), 1);
        assert_eq!(observer.calls.load(Ordering::SeqCst), 2);
        assert_eq!(observer.report().latest, Some(incomplete()));
    }

    #[tokio::test]
    async fn completion_default_path_still_stops_at_first_answer() {
        let (agent, provider) = agent_with(vec![answer(), answer()], PermissionMode::Allow);
        let mut convo = conversation();
        assert_eq!(agent.run(&mut convo, None).await.unwrap().turns, 1);
        assert_eq!(provider.seen.lock().unwrap().len(), 1);
        assert_eq!(notices(&convo), 0);
    }

    #[tokio::test]
    async fn completion_normal_answer_at_original_caps_can_observe_but_never_resume() {
        for token_cap in [false, true] {
            for observation in [ready(), incomplete()] {
                let (agent, provider) = agent_with(vec![answer()], PermissionMode::Allow);
                let observer = Observer::new(vec![Action::Return(observation.clone())]);
                let mut cx = context(&agent, &observer);
                cx.budget = if token_cap {
                    Budget {
                        max_output_tokens: Some(5),
                        ..Budget::default()
                    }
                } else {
                    Budget::turns(1)
                };
                let mut convo = conversation();
                let result = agent.run_in(&cx, &mut convo, None).await.unwrap();
                assert_eq!(result.turns, 1);
                assert_eq!(provider.seen.lock().unwrap().len(), 1);
                assert_eq!(observer.report().latest, Some(observation));
                assert!(!observer.report().feedback_delivered);
                assert_eq!(notices(&convo), 0);
            }
        }
    }

    #[tokio::test]
    async fn completion_repair_does_not_reset_original_turn_or_token_budget() {
        for token_cap in [false, true] {
            let (agent, provider) = agent_with(vec![answer(), answer()], PermissionMode::Allow);
            let observer = Observer::new(vec![]);
            let mut cx = context(&agent, &observer);
            cx.budget = if token_cap {
                Budget {
                    max_output_tokens: Some(10),
                    ..Budget::default()
                }
            } else {
                Budget::turns(2)
            };
            let result = agent.run_in(&cx, &mut conversation(), None).await.unwrap();
            assert_eq!(result.turns, 2);
            assert_eq!(result.usage.output_tokens, 10);
            assert_eq!(provider.seen.lock().unwrap().len(), 2);
        }
    }

    #[tokio::test]
    async fn completion_terminal_exclusions_never_observe() {
        for kind in 0..5 {
            let mut response = answer();
            if kind == 0 {
                response.stop_reason = StopReason::Refusal;
            }
            if kind == 1 {
                response.stop_reason = StopReason::MaxTokens;
            }
            let (agent, _) = agent_with(vec![response], PermissionMode::Allow);
            let observer = Observer::new(vec![]);
            let mut cx = context(&agent, &observer);
            if kind == 2 {
                cx.close_with = Some(ON_ITS_WAY);
            }
            if kind == 3 {
                cx.budget = Budget::turns(0);
            }
            if kind == 4 {
                let token = CancellationToken::new();
                token.cancel();
                cx = cx.with_cancel(token);
            }
            let mut convo = conversation();
            agent.run_in(&cx, &mut convo, None).await.unwrap();
            assert_eq!(
                observer.calls.load(Ordering::SeqCst),
                0,
                "excluded kind {kind}"
            );
            assert_eq!(notices(&convo), 0);
        }
    }

    #[tokio::test]
    async fn completion_empty_answers_never_observe() {
        let turns = (0..=EMPTY_TURN_RETRIES)
            .map(|_| assistant(vec![], StopReason::EndTurn))
            .collect();
        let (agent, _) = agent_with(turns, PermissionMode::Allow);
        let observer = Observer::new(vec![]);
        let result = agent
            .run_in(&context(&agent, &observer), &mut conversation(), None)
            .await
            .unwrap();
        assert_eq!(result.stop_cause, StopCause::NoOutput);
        assert_eq!(observer.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn completion_reused_context_has_fresh_loop_allowance() {
        let (agent, _) = agent_with(
            vec![answer(), answer(), answer(), answer()],
            PermissionMode::Allow,
        );
        let observer = Observer::new(vec![]);
        let cx = context(&agent, &observer);
        let mut convo = conversation();
        for _ in 0..2 {
            assert_eq!(agent.run_in(&cx, &mut convo, None).await.unwrap().turns, 2);
            convo.messages.push(Message::user("Save another result."));
        }
        assert_eq!(notices(&convo), 2);
    }

    #[tokio::test]
    async fn completion_provider_error_retains_accepted_report() {
        let (agent, _) = agent_with(vec![answer()], PermissionMode::Allow);
        let observer = Observer::new(vec![]);
        let error = agent
            .run_in(&context(&agent, &observer), &mut conversation(), None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("scripted turns"));
        let report = observer.report();
        assert_eq!(report.first, Some(incomplete()));
        assert!(report.feedback_delivered);
    }

    #[tokio::test]
    async fn completion_invalid_feedback_never_resumes() {
        let feedbacks = [
            CompletionFeedback {
                required_read_missing: false,
                artifact: None,
                do_not_repeat_read: false,
            },
            CompletionFeedback {
                required_read_missing: false,
                artifact: None,
                do_not_repeat_read: true,
            },
            CompletionFeedback {
                required_read_missing: true,
                artifact: Some(CompletionArtifactIssue::Missing),
                do_not_repeat_read: true,
            },
        ];
        for feedback in feedbacks {
            let (agent, _) = agent_with(vec![answer()], PermissionMode::Allow);
            let observer = Observer::new(vec![Action::Return(CompletionObservation {
                readiness: CompletionReadiness::Incomplete,
                feedback: Some(feedback),
            })]);
            agent
                .run_in(&context(&agent, &observer), &mut conversation(), None)
                .await
                .unwrap();
            assert_eq!(
                observer.report().latest.unwrap().readiness,
                CompletionReadiness::Unavailable
            );
            assert!(!observer.report().feedback_delivered);
        }
        for readiness in [CompletionReadiness::Ready, CompletionReadiness::Unavailable] {
            let (agent, _) = agent_with(vec![answer()], PermissionMode::Allow);
            let observer = Observer::new(vec![Action::Return(CompletionObservation {
                readiness,
                feedback: incomplete().feedback,
            })]);
            agent
                .run_in(&context(&agent, &observer), &mut conversation(), None)
                .await
                .unwrap();
            assert_eq!(
                observer.report().latest.unwrap().readiness,
                CompletionReadiness::Unavailable
            );
            assert!(!observer.report().feedback_delivered);
        }
    }

    #[tokio::test]
    async fn completion_deadline_rejects_late_ready_even_from_blocking_poll() {
        for blocking in [false, true] {
            let (agent, _) = agent_with(vec![answer()], PermissionMode::Allow);
            let action = if blocking {
                Action::Block(Duration::from_millis(30), ready())
            } else {
                Action::Wait(Duration::from_secs(1), ready())
            };
            let observer = Observer::new(vec![action]);
            let cx = context(&agent, &observer)
                .with_completion_deadline(tokio::time::Instant::now() + Duration::from_millis(15));
            let mut convo = conversation();
            agent.run_in(&cx, &mut convo, None).await.unwrap();
            let report = observer.report();
            assert_eq!(report.first, None);
            assert_eq!(report.latest, None);
            assert_eq!(report.timed_out, 1);
            assert!(!report.feedback_delivered);
            assert_eq!(notices(&convo), 0);
            assert!(observer.dropped.load(Ordering::SeqCst));
        }
    }

    #[tokio::test]
    async fn completion_cancel_wins_over_ready_and_drops_pending_observer() {
        for immediate in [false, true] {
            let token = CancellationToken::new();
            let (agent, _) = agent_with(vec![answer()], PermissionMode::Allow);
            let action = if immediate {
                Action::Cancel(token.clone(), ready())
            } else {
                Action::Wait(Duration::from_secs(1), ready())
            };
            let observer = Observer::new(vec![action]);
            let cx = context(&agent, &observer).with_cancel(token.clone());
            let cancel_task = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                token.cancel();
            });
            let mut convo = conversation();
            let result = agent.run_in(&cx, &mut convo, None).await.unwrap();
            cancel_task.await.unwrap();
            assert_eq!(result.stop_cause, StopCause::Interrupted);
            assert_eq!(observer.report().first, None);
            assert_eq!(observer.report().interrupted, 1);
            assert_eq!(notices(&convo), 0);
            assert!(observer.dropped.load(Ordering::SeqCst));
        }
    }

    #[tokio::test]
    async fn completion_second_timeout_keeps_first_accepted_observation() {
        let (agent, _) = agent_with(vec![answer(), answer()], PermissionMode::Allow);
        let observer = Observer::new(vec![
            Action::Return(incomplete()),
            Action::Wait(Duration::from_secs(5), ready()),
        ]);
        let result = agent
            .run_in(&context(&agent, &observer), &mut conversation(), None)
            .await
            .unwrap();
        assert_eq!(result.turns, 2);
        assert_eq!(observer.report().first, Some(incomplete()));
        assert_eq!(observer.report().latest, Some(incomplete()));
        assert_eq!(observer.report().timed_out, 1);
        assert!(observer.report().feedback_delivered);
    }

    #[tokio::test]
    async fn completion_expired_host_deadline_never_polls_observer() {
        let (agent, _) = agent_with(vec![answer()], PermissionMode::Allow);
        let observer = Observer::new(vec![Action::Return(ready())]);
        let cx = context(&agent, &observer).with_completion_deadline(tokio::time::Instant::now());
        agent.run_in(&cx, &mut conversation(), None).await.unwrap();
        assert_eq!(observer.calls.load(Ordering::SeqCst), 0);
        assert_eq!(observer.report().timed_out, 1);
        assert_eq!(observer.report().latest, None);
    }
    #[tokio::test]
    async fn completion_all_feedback_variants_are_harness_voice_not_owner_interventions() {
        use CompletionArtifactIssue::{Malformed, Missing};
        for (required_read_missing, artifact, do_not_repeat_read) in [
            (true, None, false),
            (false, Some(Missing), false),
            (false, Some(Malformed), false),
            (true, Some(Missing), false),
            (true, Some(Malformed), false),
            (false, Some(Missing), true),
            (false, Some(Malformed), true),
        ] {
            let (agent, _) = agent_with(vec![answer(), answer()], PermissionMode::Allow);
            let observer = Observer::new(vec![
                Action::Return(CompletionObservation {
                    readiness: CompletionReadiness::Incomplete,
                    feedback: Some(CompletionFeedback {
                        required_read_missing,
                        artifact,
                        do_not_repeat_read,
                    }),
                }),
                Action::Return(ready()),
            ]);
            let mut convo = conversation();
            agent
                .run_in(&context(&agent, &observer), &mut convo, None)
                .await
                .unwrap();
            assert_eq!(notices(&convo), 1);
            let note = convo.messages.iter().find(|m| m.harness).unwrap();
            assert!(is_harness_voice(&note.text()));
            assert!(!is_harness_voice(&format!(
                "{} The owner's separate correction.",
                note.text()
            )));
            assert!(crate::learning::extract_interventions(&convo.messages).is_empty());
            assert_eq!(
                note.text().contains("Do not repeat the required read."),
                do_not_repeat_read
            );
            assert_eq!(
                note.text()
                    .contains("Perform the required read before finishing."),
                required_read_missing
            );
        }
    }

    #[tokio::test]
    async fn completion_cost_cap_observes_without_resuming() {
        let (agent, provider) = agent_with(vec![answer()], PermissionMode::Allow);
        let agent = agent.with_pricing(Some(Pricing {
            output_per_mtok: 1_000_000.0,
            ..Pricing::default()
        }));
        let observer = Observer::new(vec![]);
        let mut cx = context(&agent, &observer);
        cx.budget.max_cost_usd = Some(5.0);
        let result = agent.run_in(&cx, &mut conversation(), None).await.unwrap();
        assert_eq!(result.cost_usd, Some(5.0));
        assert_eq!(provider.seen.lock().unwrap().len(), 1);
        assert_eq!(observer.report().latest, Some(incomplete()));
        assert!(!observer.report().feedback_delivered);
    }

    #[tokio::test]
    async fn completion_unavailable_or_incomplete_without_advice_never_resumes() {
        for readiness in [
            CompletionReadiness::Ready,
            CompletionReadiness::Unavailable,
            CompletionReadiness::Incomplete,
        ] {
            let (agent, provider) = agent_with(vec![answer()], PermissionMode::Allow);
            let observation = CompletionObservation {
                readiness,
                feedback: None,
            };
            let observer = Observer::new(vec![Action::Return(observation.clone())]);
            agent
                .run_in(&context(&agent, &observer), &mut conversation(), None)
                .await
                .unwrap();
            assert_eq!(provider.seen.lock().unwrap().len(), 1);
            assert_eq!(observer.report().latest, Some(observation));
            assert!(!observer.report().feedback_delivered);
        }
    }

    #[tokio::test]
    async fn completion_notice_does_not_relabel_a_later_owner_steer() {
        let (agent, _) = agent_with(vec![answer(), answer()], PermissionMode::Allow);
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let observer = Observer::new(vec![
            Action::Steer(queue.clone(), incomplete()),
            Action::Return(ready()),
        ]);
        let mut cx = context(&agent, &observer);
        cx.queued_input = Some(queue);
        let mut convo = conversation();
        agent.run_in(&cx, &mut convo, None).await.unwrap();
        let steer = convo
            .messages
            .iter()
            .find(|m| m.text().contains("Use the alternate format."))
            .unwrap();
        assert!(
            !steer.harness,
            "owner text merged beside a notice is still owner evidence"
        );
        assert_eq!(convo.messages[2].content.len(), 2);
        assert_eq!(owner_text(steer), "Use the alternate format.");
        assert!(
            matches!(&convo.messages[2].content[1], Block::Text { text } if text == "Use the alternate format.")
        );
        assert!(convo
            .messages
            .windows(2)
            .all(|pair| !(pair[0].role == Role::User && pair[1].role == Role::User)));
        let interventions = crate::learning::extract_interventions(&convo.messages);
        assert_eq!(interventions.len(), 1);
    }
    #[tokio::test]
    async fn completion_empty_reply_nudge_does_not_hide_a_later_owner_steer() {
        struct EmptyReplySteers {
            queue: Arc<Mutex<VecDeque<String>>>,
            calls: AtomicUsize,
        }
        #[async_trait]
        impl Provider for EmptyReplySteers {
            fn local(&self) -> bool {
                true
            }
            fn id(&self) -> &str {
                "scripted"
            }
            fn default_model(&self) -> &str {
                "scripted-1"
            }
            async fn complete(
                &self,
                req: &CompletionRequest,
                _sink: Option<&StreamSink>,
            ) -> Result<CompletionResponse> {
                match self.calls.fetch_add(1, Ordering::SeqCst) {
                    0 => Ok(answer()),
                    1 => {
                        self.queue
                            .lock()
                            .unwrap()
                            .push_back("Use the alternate format.".into());
                        Ok(assistant(vec![], StopReason::EndTurn))
                    }
                    2 => {
                        let tail = req.messages.last().unwrap();
                        assert_eq!(tail.content.len(), 3);
                        assert!(
                            matches!(&tail.content[2], Block::Text { text } if text == "Use the alternate format.")
                        );
                        Ok(answer())
                    }
                    _ => anyhow::bail!("unexpected extra request"),
                }
            }
        }
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let (mut agent, _) = agent_with(vec![], PermissionMode::Allow);
        agent.provider = Box::new(EmptyReplySteers {
            queue: queue.clone(),
            calls: AtomicUsize::new(0),
        });
        let observer = Observer::new(vec![Action::Return(incomplete()), Action::Return(ready())]);
        let mut cx = context(&agent, &observer);
        cx.queued_input = Some(queue);
        let mut convo = conversation();
        let result = agent.run_in(&cx, &mut convo, None).await.unwrap();
        assert_eq!(result.turns, 3);
        assert_eq!(
            convo.messages.len(),
            4,
            "empty response must not add a transcript slot"
        );
        let steer = &convo.messages[2];
        assert_eq!(
            owner_text(steer),
            "Use the alternate format.",
            "the empty-turn nudge must not hide the later owner steer"
        );
        assert!(!steer.harness);
        assert_eq!(steer.content.len(), 3);
        assert!(matches!(&steer.content[0], Block::Text { text } if completion::is_notice(text)));
        assert!(matches!(&steer.content[1], Block::Text { text } if text == EMPTY_TURN_NUDGE));
        assert!(
            matches!(&steer.content[2], Block::Text { text } if text == "Use the alternate format.")
        );
        assert!(convo
            .messages
            .windows(2)
            .all(|pair| !(pair[0].role == Role::User && pair[1].role == Role::User)));
        assert_eq!(
            crate::learning::extract_interventions(&convo.messages).len(),
            1
        );
        assert!(observer.report().feedback_delivered);
        assert_eq!(observer.report().latest, Some(ready()));
    }
}
