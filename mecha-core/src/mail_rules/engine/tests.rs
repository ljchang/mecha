use super::*;
use crate::closure::Actor;
use std::sync::Mutex;

struct Fixture {
    root: std::path::PathBuf,
    store: RuleStore,
    mail: Fake,
    options: EngineOptions,
    clock: std::sync::Arc<crate::clock::TestClock>,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("mail-rules-engine-{}", uuid::Uuid::new_v4()));
        let store = RuleStore::open(root.join("mail-rules")).unwrap();
        let now = Utc::now() + Duration::hours(1);
        let clock = std::sync::Arc::new(crate::clock::TestClock::new(now));
        Self {
            clock: clock.clone(),
            root,
            store,
            mail: Fake::new(now),
            options: EngineOptions {
                clock,
                ..Default::default()
            },
        }
    }
    fn rule(&self, name: &str, actions: &str) -> LoadedRule {
        self.store.install(name, &format!("description = 'Review notice'\n[match]\nfrom_domain = ['example.com']\n{actions}"), &BTreeMap::new()).unwrap()
    }
    fn mode(&self, r: &LoadedRule, group: ActionGroup, mode: Mode) {
        let d = self
            .store
            .change_mode(
                &r.name,
                group,
                mode,
                Some(Actor::Owner),
                &r.approval_digest,
                &P1Holds {
                    origin_gate: GateStatus::Passing,
                },
            )
            .unwrap();
        assert!(!d.refused);
    }
    async fn run(&self, sink: Option<&dyn TaskSink>) -> ApplyReport {
        apply(
            &self.store,
            &self.mail,
            sink,
            &["campus".into()],
            &self.options,
        )
        .await
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
struct Fake {
    candidates: Mutex<Vec<Candidate>>,
    calls: Mutex<Vec<String>>,
    results: Mutex<Vec<ActionResult>>,
    unreadable: bool,
    in_window: bool,
    gate: GateStatus,
    on_effect: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}
impl Fake {
    fn new(now: DateTime<Utc>) -> Self {
        Self {
            candidates: Mutex::new(vec![Candidate {
                message: MessageRef {
                    account: "campus".into(),
                    provider_message_id: "message-alpha".into(),
                    thread_id: "thread-alpha".into(),
                    message_id: "<display@example.com>".into(),
                    sender: "sender@example.com".into(),
                    subject: "Sender's untrusted task instructions".into(),
                },
                arrival_at: Some(now - Duration::minutes(1)),
                headers_complete: Some(true),
                subject_complete: Some(true),
                subject_is_raw: false,
                recipients_complete: Some(true),
                attachment_presence: Some(true),
                to: vec!["owner@example.edu".into()],
                list_id: Some("News <news.example.com>".into()),
                headers: BTreeMap::from([(
                    "x-notice".into(),
                    vec!["Quarterly update".into(), "Urgent".into()],
                )]),
                classified: Some(Classified {
                    at: now,
                    deadline: NaiveDate::from_ymd_opt(2026, 12, 1),
                }),
            }]),
            calls: Mutex::new(Vec::new()),
            results: Mutex::new(Vec::new()),
            unreadable: false,
            in_window: true,
            gate: GateStatus::Passing,
            on_effect: Mutex::new(None),
        }
    }
    fn effect(&self, name: String) -> ActionResult {
        self.calls.lock().unwrap().push(name);
        if let Some(callback) = self.on_effect.lock().unwrap().take() {
            callback();
        }
        let mut results = self.results.lock().unwrap();
        if results.is_empty() {
            ActionResult::Done
        } else {
            results.remove(0)
        }
    }
    fn next(&self, suffix: &str) {
        let mut cs = self.candidates.lock().unwrap();
        cs[0].message.provider_message_id = format!("message-{suffix}");
        cs[0].message.thread_id = format!("thread-{suffix}");
    }
}
#[async_trait]
impl Mailbox for Fake {
    async fn candidates(&self, _: &str, _: Pass) -> Result<Vec<String>> {
        if !self.in_window {
            return Ok(Vec::new());
        }
        Ok(self
            .candidates
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.message.thread_id.clone())
            .collect())
    }
    async fn read_thread(&self, _: &str, id: &str) -> Result<Candidate> {
        ensure!(!self.unreadable, "unreadable");
        self.candidates
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.message.thread_id == id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing thread"))
    }
    async fn mailbox_action(&self, _: &Candidate, action: ActionVerb) -> ActionResult {
        self.effect(format!("{action:?}"))
    }
    async fn private_record(&self, _: &Candidate, _: &str, action: &Action) -> ActionResult {
        self.effect(format!("{:?}", action.verb))
    }
    async fn handled(&self, _: &Candidate) -> Result<()> {
        self.calls.lock().unwrap().push("Handled".into());
        Ok(())
    }
    fn origin_gate(&self, _: &LoadedRule) -> GateStatus {
        self.gate
    }
}
#[derive(Default)]
struct Sink(Mutex<Vec<Task>>);
#[async_trait]
impl TaskSink for Sink {
    async fn create(&self, task: &Task) -> ActionResult {
        self.0.lock().unwrap().push(task.clone());
        ActionResult::Done
    }
}
const ARCHIVE: &str = "[[action]]\ndo = 'archive'\n";
const TASK: &str = "[[action]]\ndo = 'task'\n";
const DIGEST: &str = "[[action]]\ndo = 'digest'\n";

#[test]
fn header_keys_and_values_have_exact_and_contains_semantics() {
    let f = Fixture::new();
    let c = f.mail.candidates.lock().unwrap()[0].clone();
    let m = Matcher {
        from: vec!["other@example.com".into(), "SENDER@EXAMPLE.COM".into()],
        from_domain: vec!["EXAMPLE.COM".into()],
        to: vec!["OWNER@EXAMPLE.EDU".into()],
        account: vec!["CAMPUS".into()],
        list_id: vec!["NEWS.EXAMPLE.COM".into()],
        subject_contains: vec!["UNTRUSTED".into()],
        has_attachment: Some(true),
        header: BTreeMap::from([("X-Notice".into(), vec!["missing".into(), "UPDATE".into()])]),
        describe: None,
    };
    assert!(matches_headers(&m, &c));
    for bad in [
        Matcher {
            from_domain: vec!["ample.com".into()],
            ..m.clone()
        },
        Matcher {
            from: vec!["sender@else.example.com".into()],
            ..m.clone()
        },
        Matcher {
            has_attachment: Some(false),
            ..m.clone()
        },
        Matcher {
            header: BTreeMap::from([("absent".into(), vec!["".into()])]),
            ..m.clone()
        },
        Matcher {
            to: vec!["stranger@example.edu".into()],
            ..m.clone()
        },
    ] {
        assert!(!matches_headers(&bad, &c));
    }
}

#[tokio::test]
async fn shadow_off_propose_auto_and_wire_fields() {
    let f = Fixture::new();
    let r = f.rule("archive", ARCHIVE);
    let shadow = f.run(None).await;
    assert_eq!(shadow.rows[0].outcome, Outcome::WouldHave);
    assert!(f.mail.calls.lock().unwrap().is_empty());
    let row = &shadow.rows[0];
    assert_eq!(row.verdict_input, VerdictInput::NotApplicable);
    assert!(!row.match_digest.matcher.is_empty());
    assert!(row.match_digest.confirmation_version.is_empty());
    assert!(!row.closes);
    f.mode(&r, ActionGroup::Mailbox, Mode::Off);
    assert!(f.run(None).await.rows.is_empty());
    f.mode(&r, ActionGroup::Mailbox, Mode::Propose);
    assert_eq!(f.run(None).await.rows[0].outcome, Outcome::Offered);
    assert!(f.mail.calls.lock().unwrap().is_empty());
    f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
    f.mail.next("beta");
    let report = f.run(None).await;
    assert_eq!(report.rows[0].outcome, Outcome::Done);
    assert_eq!(report.handled.len(), 1);
    assert_eq!(*f.mail.calls.lock().unwrap(), vec!["Archive", "Handled"]);
}

#[tokio::test]
async fn partial_claim_does_not_suppress_unfinished_sibling_and_reordering_is_stable() {
    let f = Fixture::new();
    let r = f.rule("archive", &format!("{ARCHIVE}[[action]]\ndo = 'read'\n"));
    f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
    *f.mail.results.lock().unwrap() = vec![
        ActionResult::Done,
        ActionResult::Failed {
            kind: FailureKind::Environment,
            reason: "offline".into(),
        },
    ];
    f.run(None).await;
    f.mail.calls.lock().unwrap().clear();
    f.run(None).await;
    assert_eq!(*f.mail.calls.lock().unwrap(), vec!["Read", "Handled"]);
    let new_text = "description = 'Review notice'\n[match]\nfrom_domain = ['example.com']\n[[action]]\ndo = 'read'\n[[action]]\ndo = 'archive'\n";
    std::fs::write(r.directory.join("rule.toml"), new_text).unwrap();
    f.run(None).await;
    let edited = f.store.get("archive").unwrap();
    f.mode(&edited, ActionGroup::Mailbox, Mode::Auto);
    f.mail.calls.lock().unwrap().clear();
    f.run(None).await;
    assert_eq!(*f.mail.calls.lock().unwrap(), vec!["Handled"]);
}

#[tokio::test]
async fn digest_and_task_defer_all_matching_removals_regardless_of_rank() {
    let mut f = Fixture::new();
    let a = f.rule("a-archive", ARCHIVE);
    let d = f.rule("z-digest", &format!("{DIGEST}{TASK}"));
    f.mode(&a, ActionGroup::Mailbox, Mode::Auto);
    f.mode(&d, ActionGroup::PrivateRecord, Mode::Auto);
    f.mode(&d, ActionGroup::Board, Mode::Auto);
    let headers = f.run(None).await;
    assert!(headers.rows.is_empty());
    assert_eq!(f.store.deferred().unwrap()[0].actions.len(), 3);
    assert!(headers.handled.is_empty());
    f.options.pass = Pass::AfterClassify;
    let sink = Sink::default();
    let report = f.run(Some(&sink)).await;
    assert_eq!(
        report
            .rows
            .iter()
            .filter(|r| r.outcome == Outcome::Done)
            .count(),
        3
    );
    assert!(f.store.deferred().unwrap().is_empty());
    let tasks = sink.0.lock().unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0]
        .name
        .starts_with("Review notice — sender@example.com"));
    assert!(!tasks[0].name.contains("untrusted"));
    assert_eq!(tasks[0].due, NaiveDate::from_ymd_opt(2026, 12, 1));
}

#[tokio::test]
async fn shadow_and_description_actions_need_classify_but_off_actions_do_not() {
    let f = Fixture::new();
    let a = f.rule(
        "archive",
        &format!(
            "{ARCHIVE}{DIGEST}[[action]]\ndo = 'read'\n[[action]]\ndo = 'tag'\ntag = 'personal'\n"
        ),
    );
    f.mode(&a, ActionGroup::Mailbox, Mode::Auto);
    let report = f.run(None).await;
    assert!(report.handled.is_empty());
    assert_eq!(*f.mail.calls.lock().unwrap(), vec!["Read"]);
    assert_eq!(f.store.deferred().unwrap()[0].actions.len(), 2);
    f.mode(&a, ActionGroup::PrivateRecord, Mode::Off);
    f.mail.next("beta");
    assert_eq!(f.run(None).await.handled.len(), 1);
    f.store.install("description", "description = 'Description'\n[match]\ndescribe = 'A receipt'\n[[action]]\ndo = 'digest'\n", &BTreeMap::new()).unwrap();
    f.mail.next("gamma");
    f.mail.calls.lock().unwrap().clear();
    assert!(f.run(None).await.handled.is_empty());
    assert_eq!(*f.mail.calls.lock().unwrap(), vec!["Read"]);
}

#[tokio::test]
async fn outbound_shadow_defers_without_ever_sending_or_stealing_a_slot() {
    let f = Fixture::new();
    let r = f.rule("archive", ARCHIVE);
    f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
    f.rule(
        "forward",
        "[[action]]\ndo = 'forward'\nto = ['finance@example.edu']\n",
    );
    let report = f.run(None).await;
    assert!(report.rows.is_empty());
    assert!(f.mail.calls.lock().unwrap().is_empty());
    assert_eq!(f.store.deferred().unwrap()[0].actions.len(), 2);
}

#[tokio::test]
async fn activation_cutoff_and_lapse_do_not_replay_a_backlog() {
    let f = Fixture::new();
    let r = f.rule("archive", ARCHIVE);
    f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
    f.mail.candidates.lock().unwrap()[0].arrival_at = Some(Utc::now() - Duration::days(1));
    let report = f.run(None).await;
    assert_eq!(report.rows[0].reason, "before the rule was active");
    assert!(!report.rows[0].closes);
    assert!(report.handled.is_empty());
    f.mail.candidates.lock().unwrap()[0].arrival_at =
        Some(f.options.clock.now() - Duration::minutes(1));
    std::fs::write(r.directory.join("extra.md"), "Owner edit").unwrap();
    let lapsed = f.run(None).await;
    assert_eq!(lapsed.rows[0].outcome, Outcome::Offered);
    std::fs::remove_file(r.directory.join("extra.md")).unwrap();
    f.mail.next("beta");
    assert_eq!(f.run(None).await.rows[0].outcome, Outcome::Offered);
}

#[tokio::test]
async fn tasks_are_once_per_thread_and_missing_sink_is_held() {
    let mut f = Fixture::new();
    let r = f.rule("task", TASK);
    f.mode(&r, ActionGroup::Board, Mode::Auto);
    f.options.pass = Pass::AfterClassify;
    let report = f.run(None).await;
    assert_eq!(report.rows[0].outcome, Outcome::Held);
    assert_eq!(report.rows[0].reason, "task sink unavailable");
    let sink = Sink::default();
    f.mail.candidates.lock().unwrap()[0]
        .message
        .provider_message_id = "new-message".into();
    assert_eq!(
        f.run(Some(&sink)).await.rows[0].reason,
        "thread already handled"
    );
    assert!(sink.0.lock().unwrap().is_empty());
    f.mail.next("beta");
    f.run(Some(&sink)).await;
    assert_eq!(sink.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn board_origin_gate_and_report_ceiling_fail_closed() {
    let mut f = Fixture::new();
    let r = f.rule("rule", &format!("{TASK}[[action]]\ndo = 'spam'\n"));
    f.mode(&r, ActionGroup::Board, Mode::Auto);
    f.mode(&r, ActionGroup::Report, Mode::Propose);
    f.mail.gate = GateStatus::Unknown;
    f.options.pass = Pass::AfterClassify;
    let sink = Sink::default();
    let report = f.run(Some(&sink)).await;
    assert_ceiling_rows(&report);
    assert!(sink.0.lock().unwrap().is_empty());
    assert!(f.mail.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn pending_sweep_reopens_mailbox_and_holds_task_fate() {
    let mut f = Fixture::new();
    let r = f.rule("rule", &format!("{ARCHIVE}{TASK}"));
    f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
    f.mode(&r, ActionGroup::Board, Mode::Auto);
    let message = f.mail.candidates.lock().unwrap()[0].message.clone();
    let pending: Vec<_> = r
        .definition
        .actions
        .iter()
        .enumerate()
        .map(|(i, _)| {
            LedgerRow::header(
                &r,
                i,
                message.clone(),
                Mode::Auto,
                GateStatus::Passing,
                Outcome::Pending,
                f.options.clock.now() - Duration::hours(1),
            )
            .unwrap()
        })
        .collect();
    f.store.claim_actions(&pending).unwrap();
    f.options.pass = Pass::AfterClassify;
    let sink = Sink::default();
    let report = f.run(Some(&sink)).await;
    assert!(report
        .rows
        .iter()
        .any(|r| r.reason == "interrupted" && r.failure_kind == FailureKind::Interrupted));
    assert!(report.rows.iter().any(|r| r.reason == "fate unknown"));
    assert_eq!(*f.mail.calls.lock().unwrap(), vec!["Archive"]);
    assert!(sink.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn only_action_failures_trip_the_sticky_group_brake() {
    for kind in [
        FailureKind::Action,
        FailureKind::Unknown,
        FailureKind::Environment,
        FailureKind::Interrupted,
    ] {
        let f = Fixture::new();
        let r = f.rule("archive", ARCHIVE);
        f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
        for _ in 0..3 {
            f.mail.results.lock().unwrap().push(ActionResult::Failed {
                kind,
                reason: "failed".into(),
            });
            f.run(None).await;
        }
        f.mail.next("beta");
        let report = f.run(None).await;
        let braked = matches!(kind, FailureKind::Action | FailureKind::Unknown);
        assert_eq!(
            report.rows[0].outcome,
            if braked {
                Outcome::WouldHave
            } else {
                Outcome::Done
            }
        );
        let mode = resolve_mode(
            &r,
            ActionGroup::Mailbox,
            &f.store.approvals().unwrap(),
            &P1Holds {
                origin_gate: GateStatus::Passing,
            },
        );
        assert_eq!(
            mode.effective,
            if braked { Mode::Shadow } else { Mode::Auto }
        );
    }
}

#[tokio::test]
async fn dry_run_does_not_write_or_execute_even_sweeps_lapses_and_brakes() {
    let mut f = Fixture::new();
    let r = f.rule("archive", ARCHIVE);
    f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
    let before = std::fs::read(f.store.root().join("approvals.jsonl")).unwrap();
    f.options.dry_run = true;
    let report = f.run(None).await;
    assert_eq!(report.rows[0].outcome, Outcome::Done);
    assert!(f.mail.calls.lock().unwrap().is_empty());
    assert!(f.store.ledger().unwrap().is_empty());
    assert!(f.store.deferred().unwrap().is_empty());
    assert!(!f.store.root().join(".engine.lock").exists());
    std::fs::write(r.directory.join("extra.md"), "changed").unwrap();
    assert_eq!(f.run(None).await.rows[0].outcome, Outcome::Offered);
    assert_eq!(
        before,
        std::fs::read(f.store.root().join("approvals.jsonl")).unwrap()
    );
}

#[tokio::test]
async fn unreadable_deferred_threads_fail_visibly_and_expiry_does_not_brake() {
    let mut f = Fixture::new();
    f.rule("digest", DIGEST);
    f.run(None).await;
    f.mail.unreadable = true;
    f.options.pass = Pass::AfterClassify;
    let report = f.run(None).await;
    assert!(report.rows.iter().any(|r| r.outcome == Outcome::Failed
        && r.reason == "thread unreadable"
        && r.failure_kind == FailureKind::Environment));
    assert!(f.store.deferred().unwrap().is_empty());
    f.mail.unreadable = false;
    f.options.pass = Pass::Headers;
    f.run(None).await;
    f.clock.advance(Duration::days(8));
    f.mail.candidates.lock().unwrap().clear();
    let report = f.run(None).await;
    assert!(report.notices.iter().any(|s| s.contains("expired")));
    assert!(f
        .store
        .ledger()
        .unwrap()
        .iter()
        .any(|r| r.outcome == Outcome::Expired && r.closes));
}

#[tokio::test]
async fn after_classify_requires_a_fresh_verdict_and_keeps_owed_work() {
    let mut f = Fixture::new();
    let r = f.rule("digest", DIGEST);
    f.mode(&r, ActionGroup::PrivateRecord, Mode::Auto);
    f.run(None).await;
    f.options.pass = Pass::AfterClassify;
    f.mail.candidates.lock().unwrap()[0].classified = None;
    assert!(f.run(None).await.rows.is_empty());
    assert!(!f.store.deferred().unwrap().is_empty());
    assert!(f.mail.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn per_rule_and_global_caps_hold_tasks_but_not_mailbox() {
    for global in [false, true] {
        let mut f = Fixture::new();
        if global {
            std::fs::write(f.store.root().join("limits.toml"), "daily_cap = 1\n").unwrap();
        }
        let text = format!("description = 'Review notice'\ndaily_cap = {}\n[match]\nfrom_domain = ['example.com']\n{TASK}{ARCHIVE}{DIGEST}", if global { 20 } else { 1 });
        let r = f.store.install("task", &text, &BTreeMap::new()).unwrap();
        f.mode(&r, ActionGroup::Board, Mode::Auto);
        f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
        f.mode(&r, ActionGroup::PrivateRecord, Mode::Auto);
        f.options.pass = Pass::AfterClassify;
        let sink = Sink::default();
        f.run(Some(&sink)).await;
        f.mail.next("beta");
        let report = f.run(Some(&sink)).await;
        assert_eq!(sink.0.lock().unwrap().len(), 1);
        assert!(report
            .rows
            .iter()
            .any(|r| r.action.verb == ActionVerb::Digest && r.outcome == Outcome::Done));
        assert!(report.rows.iter().any(|r| r.outcome == Outcome::Held
            && r.reason == if global { "global cap" } else { "rule cap" }));
        assert!(report
            .rows
            .iter()
            .any(|r| r.action.verb == ActionVerb::Archive && r.outcome == Outcome::Done));
    }
}

#[tokio::test]
async fn list_cap_is_visible_without_any_action_or_permission_change() {
    let f = Fixture::new();
    for index in 0..13 {
        f.store
            .install(
                &format!("description-{index}"),
                "description = 'Notice'\n[match]\ndescribe = 'A notice'\n[[action]]\ndo = 'task'\n",
                &BTreeMap::new(),
            )
            .unwrap();
    }
    let before = f.store.approvals().unwrap().len();
    let report = f.run(None).await;
    assert_eq!(report.list_cap, vec!["description-0"]);
    assert!(report.rows.is_empty());
    assert_eq!(f.store.approvals().unwrap().len(), before);
}

#[tokio::test]
async fn stable_priority_and_name_order_applies_every_mailbox_match() {
    let f = Fixture::new();
    for (name, priority, verb) in [
        ("z-first", 5, "read"),
        ("a-second", 10, "archive"),
        ("b-third", 10, "unread"),
    ] {
        let r = f.store.install(name, &format!("description = 'Notice'\npriority = {priority}\n[match]\nfrom_domain = ['example.com']\n[[action]]\ndo = '{verb}'\n"), &BTreeMap::new()).unwrap();
        f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
    }
    f.run(None).await;
    assert_eq!(
        *f.mail.calls.lock().unwrap(),
        vec!["Read", "Archive", "Unread", "Handled"]
    );
}

#[tokio::test]
async fn undo_and_wrong_match_brakes_count_distinct_actions_and_matches() {
    for wrong_match in [false, true] {
        let f = Fixture::new();
        let r = f.rule(
            "archive",
            &format!("{ARCHIVE}[[action]]\ndo = 'tag'\ntag = 'notice'\n"),
        );
        f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
        f.mode(&r, ActionGroup::PrivateRecord, Mode::Auto);
        let mut done = Vec::new();
        for index in 0..10 {
            f.mail.next(&format!("item-{index}"));
            let report = f.run(None).await;
            done.push(
                report
                    .rows
                    .into_iter()
                    .find(|r| r.action.verb == ActionVerb::Archive && r.outcome == Outcome::Done)
                    .unwrap(),
            );
        }
        for row in done.iter().take(3) {
            let mut correction = row.clone();
            correction.id = uuid::Uuid::new_v4().to_string();
            correction.settlement = None;
            correction.outcome = Outcome::Undone;
            correction.wrong_match = wrong_match;
            f.store.append_ledger(&correction).unwrap();
        }
        f.mail.next("last");
        let report = f.run(None).await;
        assert!(report
            .rows
            .iter()
            .any(|r| r.action.verb == ActionVerb::Archive && r.outcome == Outcome::Offered));
        assert_eq!(
            report
                .rows
                .iter()
                .find(|r| r.action.verb == ActionVerb::Tag)
                .unwrap()
                .outcome,
            if wrong_match {
                Outcome::Offered
            } else {
                Outcome::Done
            }
        );
    }
}

#[tokio::test]
async fn unreadable_history_and_concurrent_run_refuse_effects() {
    let f = Fixture::new();
    let r = f.rule("archive", ARCHIVE);
    f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
    let lease = f.store.engine_lease().unwrap();
    assert!(
        apply(&f.store, &f.mail, None, &["campus".into()], &f.options)
            .await
            .is_err()
    );
    drop(lease);
    std::fs::write(f.store.root().join("ledger.jsonl"), "{broken").unwrap();
    assert!(
        apply(&f.store, &f.mail, None, &["campus".into()], &f.options)
            .await
            .is_err()
    );
    assert!(f.mail.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_undo_never_reexecutes_or_reasserts_the_classification_skip() {
    let f = Fixture::new();
    let r = f.rule("archive", ARCHIVE);
    f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
    let mut undone = f.run(None).await.rows.remove(0);
    undone.id = uuid::Uuid::new_v4().to_string();
    undone.settlement = None;
    undone.outcome = Outcome::Undone;
    f.store.append_ledger(&undone).unwrap();
    f.mail.calls.lock().unwrap().clear();
    let report = f.run(None).await;
    assert!(report.handled.is_empty());
    assert!(f.mail.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn caps_use_the_configured_local_day_and_include_other_rules() {
    let mut f = Fixture::new();
    std::fs::write(f.store.root().join("limits.toml"), "daily_cap = 1").unwrap();
    let first = f.rule("first", TASK);
    let second = f.rule("second", TASK);
    f.mode(&first, ActionGroup::Board, Mode::Auto);
    f.mode(&second, ActionGroup::Board, Mode::Auto);
    f.options.pass = Pass::AfterClassify;
    f.options.timezone = chrono_tz::America::Los_Angeles;
    f.clock.set(
        (Utc::now().date_naive() + Duration::days(2))
            .and_hms_opt(0, 30, 0)
            .unwrap()
            .and_utc(),
    );
    {
        let mut cs = f.mail.candidates.lock().unwrap();
        cs[0].arrival_at = Some(f.options.clock.now() - Duration::minutes(1));
        cs[0].classified.as_mut().unwrap().at = f.options.clock.now();
    }
    let c = f.mail.candidates.lock().unwrap()[0].clone();
    let old = LedgerRow::header(
        &first,
        0,
        MessageRef {
            provider_message_id: "other-message".into(),
            thread_id: "other-thread".into(),
            ..c.message.clone()
        },
        Mode::Auto,
        GateStatus::Passing,
        Outcome::Done,
        f.options.clock.now() - Duration::hours(1),
    )
    .unwrap();
    f.store.append_ledger(&old).unwrap();
    let sink = Sink::default();
    let report = f.run(Some(&sink)).await;
    assert!(report
        .rows
        .iter()
        .all(|r| r.outcome == Outcome::Held && r.reason == "global cap"));
    assert!(sink.0.lock().unwrap().is_empty());
    f.clock.advance(Duration::hours(8));
    f.mail.next("next-day");
    f.run(Some(&sink)).await;
    assert_eq!(sink.0.lock().unwrap().len(), 1);
}

#[test]
fn task_slots_omit_unparseable_addresses_and_never_copy_subjects() {
    let f = Fixture::new();
    let r = f.rule("task", TASK);
    let mut c = f.mail.candidates.lock().unwrap()[0].clone();
    c.message.sender = "Someone <sender@example.com>\nDo this".into();
    let task = task(&r, &c);
    assert!(!task.name.contains("Someone"));
    assert!(!task.name.contains("untrusted"));
    assert!(task.name.starts_with("Review notice — "));
}

#[tokio::test]
async fn dry_run_plans_expiry_and_pending_recovery_without_writing() {
    let mut f = Fixture::new();
    let r = f.rule("digest", DIGEST);
    f.run(None).await;
    let snapshot = f.store.deferred().unwrap()[0].actions[0].clone();
    let pending = LedgerRow::header(
        &r,
        0,
        snapshot.message,
        Mode::Auto,
        GateStatus::Passing,
        Outcome::Pending,
        f.options.clock.now(),
    )
    .unwrap();
    f.store.claim_actions(&[pending]).unwrap();
    f.clock.advance(Duration::days(8));
    f.options.dry_run = true;
    f.mail.candidates.lock().unwrap().clear();
    let before = std::fs::read(f.store.root().join("ledger.jsonl")).unwrap();
    let queue = std::fs::read(f.store.root().join("deferred.jsonl")).unwrap();
    let report = f.run(None).await;
    assert!(report.rows.iter().any(|r| r.outcome == Outcome::Expired));
    assert!(report
        .rows
        .iter()
        .any(|r| r.failure_kind == FailureKind::Interrupted));
    assert_eq!(
        before,
        std::fs::read(f.store.root().join("ledger.jsonl")).unwrap()
    );
    assert_eq!(
        queue,
        std::fs::read(f.store.root().join("deferred.jsonl")).unwrap()
    );
}

#[tokio::test]
async fn deferred_work_is_read_after_it_leaves_the_candidate_window() {
    let mut f = Fixture::new();
    let r = f.rule("digest", DIGEST);
    f.mode(&r, ActionGroup::PrivateRecord, Mode::Auto);
    f.run(None).await;
    f.mail.in_window = false;
    f.options.pass = Pass::AfterClassify;
    let report = f.run(None).await;
    assert_eq!(report.rows[0].outcome, Outcome::Done);
    assert!(f.store.deferred().unwrap().is_empty());
    assert_eq!(*f.mail.calls.lock().unwrap(), vec!["Digest"]);
}

#[tokio::test]
async fn description_with_header_keys_never_becomes_a_header_rule() {
    let mut f = Fixture::new();
    f.store.install("described", "description = 'Description'\n[match]\nfrom_domain = ['example.com']\ndescribe = 'A receipt'\n[[action]]\ndo = 'trash'\n", &BTreeMap::new()).unwrap();
    assert!(f.run(None).await.rows.is_empty());
    f.options.pass = Pass::AfterClassify;
    assert!(f.run(None).await.rows.is_empty());
    assert!(f.mail.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn owner_reapproval_acknowledges_the_prior_failure_brake() {
    let f = Fixture::new();
    let r = f.rule("archive", ARCHIVE);
    f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
    for _ in 0..3 {
        f.mail.results.lock().unwrap().push(ActionResult::Failed {
            kind: FailureKind::Action,
            reason: "refused".into(),
        });
        f.run(None).await;
    }
    f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
    // The store's owner command uses the wall clock; move the last approval
    // past this fixture's injected future action clock, preserving its shape.
    let mut approvals = f.store.approvals().unwrap();
    approvals.last_mut().unwrap().at = Some(f.options.clock.now() + Duration::seconds(1));
    let bytes = approvals
        .iter()
        .map(|r| format!("{}\n", serde_json::to_string(r).unwrap()))
        .collect::<String>();
    std::fs::write(f.store.root().join("approvals.jsonl"), bytes).unwrap();
    f.clock.advance(Duration::minutes(1));
    f.mail.candidates.lock().unwrap()[0].arrival_at =
        Some(f.options.clock.now() - Duration::seconds(1));
    f.mail.next("recovered");
    assert_eq!(f.run(None).await.rows[0].outcome, Outcome::Done);
}

#[test]
fn outbound_slot_ignores_mailbox_board_shadow_and_off_groups() {
    let f = Fixture::new();
    f.rule("a-mailbox", ARCHIVE);
    f.rule("b-board", TASK);
    let forward = "[[action]]\ndo = 'forward'\nto = ['finance@example.edu']\n";
    f.rule("c-shadow", forward);
    f.rule("d-off", forward);
    f.rule("e-active", forward);
    f.rule("f-active", forward);
    let rules = f.store.list().unwrap();
    // P1's resolver permanently holds outbound at shadow. Exercise the slot's
    // active-input contract separately so P4 cannot inherit an untested slot.
    let mut modes = BTreeMap::new();
    for rule in &rules {
        let mode = match rule.name.as_str() {
            "c-shadow" => Mode::Shadow,
            "d-off" => Mode::Off,
            "e-active" => Mode::Propose,
            _ => Mode::Auto,
        };
        modes.insert(
            rule.name.clone(),
            rule.definition
                .groups()
                .into_iter()
                .map(|g| {
                    (
                        g,
                        ResolvedMode {
                            recorded: mode,
                            effective: mode,
                            lapsed: false,
                            holds: Vec::new(),
                            activated_at: Some(f.options.clock.now()),
                        },
                    )
                })
                .collect(),
        );
    }
    let matched: Vec<_> = rules.iter().collect();
    assert_eq!(
        outbound_owner(&matched, &modes).as_deref(),
        Some("e-active")
    );
    modes
        .get_mut("e-active")
        .unwrap()
        .get_mut(&ActionGroup::VerbatimForward)
        .unwrap()
        .effective = Mode::Shadow;
    assert_eq!(
        outbound_owner(&matched, &modes).as_deref(),
        Some("f-active")
    );
    modes
        .get_mut("f-active")
        .unwrap()
        .get_mut(&ActionGroup::VerbatimForward)
        .unwrap()
        .effective = Mode::Off;
    assert_eq!(outbound_owner(&matched, &modes), None);
}

// Protect the fail-closed test itself against vacuous success.
fn assert_ceiling_rows(report: &ApplyReport) {
    assert_eq!(report.rows.len(), 2);
    assert_eq!(
        report
            .rows
            .iter()
            .filter(|r| r.action.verb == ActionVerb::Task && r.outcome == Outcome::Offered)
            .count(),
        1
    );
    assert_eq!(
        report
            .rows
            .iter()
            .filter(|r| r.action.verb == ActionVerb::Spam && r.outcome == Outcome::Offered)
            .count(),
        1
    );
}
#[test]
#[should_panic]
fn ceiling_assertion_rejects_silent_omission() {
    assert_ceiling_rows(&ApplyReport::default());
}

#[test]
fn semantic_duplicate_tasks_are_rejected() {
    let f = Fixture::new();
    assert!(f.store.install("duplicates", &format!("description = 'Notice'\n[match]\nfrom_domain = ['example.com']\n{TASK}{TASK}to = []\n"), &BTreeMap::new()).is_err());
}

#[tokio::test]
async fn sibling_reservations_share_rule_and_global_capacity() {
    for global in [false, true] {
        let mut f = Fixture::new();
        if global {
            std::fs::write(f.store.root().join("limits.toml"), "daily_cap = 1\n").unwrap();
        }
        let text = format!("description = 'Notice'\ndaily_cap = {}\n[match]\nfrom_domain = ['example.com']\n{TASK}", if global {20} else {1});
        let mut r = f
            .store
            .install("siblings", &text, &BTreeMap::new())
            .unwrap();
        f.mode(&r, ActionGroup::Board, Mode::Auto);
        // Exercise the reservation defense independently of loader validation:
        // a future second Board action must share the same batch capacity.
        r.definition.actions.push(r.definition.actions[0].clone());
        let mut second = r.actions[0].clone();
        second.digest = "future-board-action".into();
        r.actions.push(second);
        f.options.pass = Pass::AfterClassify;
        let sink = Sink::default();
        let c = f.mail.candidates.lock().unwrap()[0].clone();
        let mut run = Run {
            store: &f.store,
            mailbox: &f.mail,
            sink: Some(&sink),
            options: &f.options,
            rows: Vec::new(),
            report: ApplyReport::default(),
        };
        run.apply_thread(&[r], &c).await.unwrap();
        assert_eq!(sink.0.lock().unwrap().len(), 1);
        assert_eq!(
            run.report
                .rows
                .iter()
                .filter(|r| r.outcome == Outcome::Held && r.reason.ends_with("cap"))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn unknown_envelope_facts_cannot_remove_or_skip_classification() {
    for attachment in [false, true] {
        let f = Fixture::new();
        let text = format!(
            "description = 'Notice'\n[match]\nfrom_domain = ['example.com']\n{}{}",
            if attachment {
                "has_attachment = false\n"
            } else {
                ""
            },
            ARCHIVE
        );
        let r = f.store.install("archive", &text, &BTreeMap::new()).unwrap();
        f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
        if !attachment {
            f.store
                .install(
                    "digest",
                    &format!(
                        "description = 'Notice'\n[match.header]\nx-missing = ['notice']\n{DIGEST}"
                    ),
                    &BTreeMap::new(),
                )
                .unwrap();
        }
        {
            let mut cs = f.mail.candidates.lock().unwrap();
            cs[0].headers_complete = None;
            cs[0].attachment_presence = None;
        }
        let report = f.run(None).await;
        assert!(f.mail.calls.lock().unwrap().is_empty());
        assert!(report.handled.is_empty());
        assert!(
            report.rows.iter().any(|r| r.outcome == Outcome::Held) || !report.deferred.is_empty()
        );
    }
}

#[tokio::test]
async fn unknown_arrival_cannot_pass_activation() {
    let f = Fixture::new();
    let r = f.rule("archive", ARCHIVE);
    f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
    f.mail.candidates.lock().unwrap()[0].arrival_at = None;
    let report = f.run(None).await;
    assert!(f.mail.calls.lock().unwrap().is_empty());
    assert_eq!(report.rows[0].outcome, Outcome::Held);
}

#[tokio::test]
async fn reservations_and_attempts_read_the_live_clock() {
    let mut f = Fixture::new();
    let r = f.rule("task", TASK);
    f.mode(&r, ActionGroup::Board, Mode::Auto);
    f.options.pass = Pass::AfterClassify;
    let next_day = f.options.clock.now() + Duration::days(1);
    let clock = std::sync::Arc::new(crate::clock::TestClock::new(next_day));
    f.options.clock = clock.clone();
    let sink = Sink::default();
    f.run(Some(&sink)).await;
    let ledger = f.store.ledger().unwrap();
    let attempts: Vec<_> = ledger
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Pending | Outcome::Done))
        .collect();
    assert_eq!(attempts.len(), 2);
    assert_eq!(sink.0.lock().unwrap().len(), 1);
    assert!(attempts.iter().all(|r| r.at == Some(next_day)));
}

#[tokio::test]
async fn midnight_rechecks_caps_and_charges_the_day_of_each_attempt() {
    let mut f = Fixture::new();
    let before = (Utc::now().date_naive() + Duration::days(2))
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_utc();
    f.clock.set(before);
    std::fs::write(f.store.root().join("limits.toml"), "daily_cap = 1").unwrap();

    let r = f.store.install("task", &format!("description = 'Notice'\ndaily_cap = 1\n[match]\nfrom_domain = ['example.com']\n{TASK}"), &BTreeMap::new()).unwrap();
    f.mode(&r, ActionGroup::Board, Mode::Auto);
    f.options.pass = Pass::AfterClassify;
    {
        let mut cs = f.mail.candidates.lock().unwrap();
        cs[0].arrival_at = Some(before - Duration::minutes(1));
        cs[0].classified.as_mut().unwrap().at = before;
        let mut sibling = cs[0].clone();
        sibling.message.provider_message_id = "message-beta".into();
        sibling.message.thread_id = "thread-beta".into();
        cs.push(sibling);
    }
    struct MidnightSink {
        clock: std::sync::Arc<crate::clock::TestClock>,
        calls: Mutex<usize>,
    }
    #[async_trait]
    impl TaskSink for MidnightSink {
        async fn create(&self, _: &Task) -> ActionResult {
            *self.calls.lock().unwrap() += 1;
            self.clock.advance(Duration::seconds(2));
            ActionResult::Done
        }
    }
    let sink = MidnightSink {
        clock: f.clock.clone(),
        calls: Mutex::new(0),
    };
    let report = f.run(Some(&sink)).await;
    assert_eq!(*sink.calls.lock().unwrap(), 2);
    let done: Vec<_> = report
        .rows
        .iter()
        .filter(|r| r.outcome == Outcome::Done)
        .collect();
    assert_eq!(done.len(), 2);
    assert_eq!(done[0].at, Some(before));
    assert_eq!(done[1].at, Some(before + Duration::seconds(2)));
    f.mail.next("gamma");
    let report = f.run(Some(&sink)).await;
    assert!(report
        .rows
        .iter()
        .any(|r| r.outcome == Outcome::Held && r.reason.ends_with("cap")));
    assert_eq!(*sink.calls.lock().unwrap(), 2);
}

#[tokio::test]
async fn unknown_deferral_survives_classification_until_headers_are_complete() {
    let mut f = Fixture::new();
    let archive = f.rule("archive", ARCHIVE);
    f.mode(&archive, ActionGroup::Mailbox, Mode::Auto);
    f.store
        .install(
            "digest",
            &format!("description = 'Notice'\n[match.header]\nx-missing = ['notice']\n{DIGEST}"),
            &BTreeMap::new(),
        )
        .unwrap();
    f.mail.candidates.lock().unwrap()[0].headers_complete = Some(false);
    f.run(None).await;
    assert_eq!(f.store.deferred().unwrap()[0].actions.len(), 2);
    f.options.pass = Pass::AfterClassify;
    let report = f.run(None).await;
    assert_eq!(report.rows.len(), 2);
    assert!(report.rows.iter().all(|r| r.outcome == Outcome::Held));
    assert!(f.mail.calls.lock().unwrap().is_empty());
    assert_eq!(f.store.deferred().unwrap()[0].actions.len(), 2);
    f.mail.candidates.lock().unwrap()[0].headers_complete = Some(true);
    f.run(None).await;
    assert_eq!(*f.mail.calls.lock().unwrap(), vec!["Archive"]);
    assert!(f.store.deferred().unwrap().is_empty());
}

#[tokio::test]
async fn reactivation_during_a_sibling_invalidates_pending_eligibility() {
    for group in [ActionGroup::Mailbox, ActionGroup::Board] {
        let mut f = Fixture::new();
        let r = f.rule(
            "reactivated",
            &format!("[[action]]\ndo = 'read'\n{ARCHIVE}{TASK}"),
        );
        f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
        f.mode(&r, ActionGroup::Board, Mode::Auto);
        f.options.pass = Pass::AfterClassify;
        let root = f.store.root().to_owned();
        let rule = r.clone();
        let reactivated_at = f.options.clock.now() + Duration::minutes(1);
        *f.mail.on_effect.lock().unwrap() = Some(Box::new(move || {
            let store = RuleStore::open_existing(root).unwrap();
            for mode in [Mode::Off, Mode::Auto] {
                let decision = store
                    .change_mode(
                        &rule.name,
                        group,
                        mode,
                        Some(Actor::Owner),
                        &rule.approval_digest,
                        &P1Holds {
                            origin_gate: GateStatus::Passing,
                        },
                    )
                    .unwrap();
                assert!(!decision.refused);
            }
            // Store approvals use wall time. Give the new activation a precise
            // fixture timestamp after the candidate without sleeping a test.
            let mut approvals = store.approvals().unwrap();
            approvals.last_mut().unwrap().at = Some(reactivated_at);
            let text = approvals
                .iter()
                .map(|r| serde_json::to_string(r).unwrap() + "\n")
                .collect::<String>();
            std::fs::write(store.root().join("approvals.jsonl"), text).unwrap();
        }));
        let sink = Sink::default();
        let report = f.run(Some(&sink)).await;
        let verb = if group == ActionGroup::Mailbox {
            ActionVerb::Archive
        } else {
            ActionVerb::Task
        };
        let row = report.rows.iter().find(|r| r.action.verb == verb).unwrap();
        assert_eq!(row.outcome, Outcome::Skipped);
        assert_eq!(row.reason, "before the rule was active");
        assert!(row.settlement.is_some());
        if group == ActionGroup::Mailbox {
            assert!(!f.mail.calls.lock().unwrap().contains(&"Archive".into()));
        } else {
            assert!(sink.0.lock().unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn known_envelope_negatives_do_not_defer_archive_for_raw_header_uncertainty() {
    for condition in [
        "subject_contains = ['invoice']",
        "to = ['other@example.edu']",
    ] {
        let f = Fixture::new();
        let archive = f.rule("archive", ARCHIVE);
        f.mode(&archive, ActionGroup::Mailbox, Mode::Auto);
        f.store
            .install(
                "task",
                &format!("description = 'Notice'\n[match]\n{condition}\n{TASK}"),
                &BTreeMap::new(),
            )
            .unwrap();
        {
            let mut cs = f.mail.candidates.lock().unwrap();
            cs[0].headers_complete = None;
            cs[0].message.subject = "Meeting agenda".into();
        }
        let report = f.run(None).await;
        assert_eq!(*f.mail.calls.lock().unwrap(), vec!["Archive", "Handled"]);
        assert!(report.deferred.is_empty());
    }
}

#[test]
fn stored_triage_threads_order_mixed_precision_arrivals() {
    let f = Fixture::new();
    let triage = crate::mail_triage::TriageStore::open(f.root.join("triage")).unwrap();
    for (id, date) in [
        ("older", "2026-11-03T12:00:00Z"),
        ("newer", "2026-11-03T12:00:00.100Z"),
    ] {
        let record: crate::mail_triage::Record = serde_json::from_value(
            serde_json::json!({"account":"campus", "thread_id":id, "date":date, "state":"classified"}),
        )
        .unwrap();
        triage.put(&record).unwrap();
    }
    let records = triage.list().unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].thread_id, "newer");
}

#[tokio::test]
async fn unknown_sender_holds_even_a_subject_only_rule() {
    let f = Fixture::new();
    let rule = f.store.install("subject", "description = 'Review notice'\n[match]\nsubject_contains = ['untrusted']\n[[action]]\ndo = 'trash'", &BTreeMap::new()).unwrap();
    f.mode(&rule, ActionGroup::Mailbox, Mode::Auto);
    f.mail.candidates.lock().unwrap()[0].message.sender.clear();
    let report = f.run(None).await;
    assert_eq!(report.rows.len(), 1);
    assert_eq!(report.rows[0].outcome, Outcome::Held);
    assert!(f.mail.calls.lock().unwrap().is_empty());
}

#[test]
fn list_id_parses_surrounding_phrase_whitespace_and_comments() {
    let f = Fixture::new();
    let mut c = f.mail.candidates.lock().unwrap()[0].clone();
    let m = Matcher {
        list_id: vec!["news.example.com".into()],
        ..Default::default()
    };
    for raw in [
        "News <news.example.com> ",
        "News < news. example.com >\r\n",
        "News <news.example.com> (weekly)",
        "\"News <display>\" <news.example.com> (outer (inner))\r\n\t",
    ] {
        c.list_id = Some(raw.into());
        assert_eq!(header_match(&m, &c), HeaderMatch::Match, "{raw}");
    }
}

#[test]
fn malformed_list_id_is_unknown_even_with_complete_headers() {
    let f = Fixture::new();
    let mut c = f.mail.candidates.lock().unwrap()[0].clone();
    let m = Matcher {
        list_id: vec!["news.example.com".into()],
        ..Default::default()
    };
    for raw in [
        "News <news.example.com",
        "news.example.com",
        "<news.example.com> junk",
        "<news.example.com> (unterminated",
        "<news.example.com><other.example.com>",
    ] {
        c.list_id = Some(raw.into());
        assert_eq!(header_match(&m, &c), HeaderMatch::Unknown, "{raw}");
    }
}

#[tokio::test]
async fn expired_occasions_cannot_be_reenqueued_or_dispatched() {
    for fresh_verdict in [false, true] {
        let mut f = Fixture::new();
        let r = f.rule("task", "[[action]]\ndo = 'task'\n");
        f.mode(&r, ActionGroup::Board, Mode::Auto);
        f.run(None).await;
        let queued = f.store.deferred().unwrap()[0].clone();
        f.clock.advance(Duration::days(8));
        f.options.pass = Pass::AfterClassify;
        let now = f.options.clock.now();
        f.mail.candidates.lock().unwrap()[0].classified = fresh_verdict.then_some(Classified {
            at: now,
            deadline: None,
        });
        let sink = Sink::default();
        let report = f.run(Some(&sink)).await;
        assert!(report.notices.iter().any(|n| n.contains("expired")));
        assert!(
            f.store.deferred().unwrap().is_empty(),
            "expired occasion was re-enqueued"
        );
        assert!(
            sink.0.lock().unwrap().is_empty(),
            "expired occasion was dispatched"
        );
        assert!(
            f.store.enqueue(&queued).is_err(),
            "store admitted an expired snapshot"
        );
    }
}

#[tokio::test]
async fn undecodable_subject_and_malformed_list_id_hold_actions_and_skips() {
    for bad_subject in [true, false] {
        let f = Fixture::new();
        let matcher = if bad_subject {
            "subject_contains = ['invoice']"
        } else {
            "list_id = ['news.example.com']"
        };
        let rule = f
            .store
            .install(
                "notice",
                &format!(
                    "description = 'Review notice'\n[match]\n{matcher}\n[[action]]\ndo = 'trash'"
                ),
                &BTreeMap::new(),
            )
            .unwrap();
        f.mode(&rule, ActionGroup::Mailbox, Mode::Auto);
        {
            let mut candidates = f.mail.candidates.lock().unwrap();
            candidates[0].subject_is_raw = true;
            candidates[0].message.subject = "=?utf-8?Q?=ZZ?=".into();
            candidates[0].list_id = Some("<news.example.com> unexpected".into());
        }
        let report = f.run(None).await;
        assert_eq!(report.rows.len(), 1);
        assert_eq!(report.rows[0].outcome, Outcome::Held);
        assert!(!report.rows[0].closes);
        assert!(f.mail.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn dry_run_expiry_closes_the_planned_occasion_without_writing() {
    let mut f = Fixture::new();
    let r = f.rule("task", "[[action]]\ndo = 'task'\n");
    f.mode(&r, ActionGroup::Board, Mode::Auto);
    f.run(None).await;
    let queue = std::fs::read(f.store.root().join("deferred.jsonl")).unwrap();
    let rows = f.store.ledger().unwrap();
    f.clock.advance(Duration::days(8));
    f.options.pass = Pass::AfterClassify;
    f.options.dry_run = true;
    let sink = Sink::default();
    let report = f.run(Some(&sink)).await;
    assert!(report
        .rows
        .iter()
        .any(|r| r.outcome == Outcome::Expired && r.closes));
    assert!(report.rows.iter().all(|r| r.outcome != Outcome::Done));
    assert!(report.deferred.is_empty());
    assert!(sink.0.lock().unwrap().is_empty());
    assert_eq!(
        std::fs::read(f.store.root().join("deferred.jsonl")).unwrap(),
        queue
    );
    assert_eq!(f.store.ledger().unwrap().len(), rows.len());
}
