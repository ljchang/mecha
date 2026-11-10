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
    listing_failure: Option<String>,
    partial_header_accounts: BTreeSet<String>,
    hold_read: bool,
    handled_root: Option<std::path::PathBuf>,
    in_window: bool,
    gate: GateStatus,
    on_effect: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    /// Staged replies' bodies, and what the owner did with each item.
    staged: Mutex<Vec<String>>,
    answering: Mutex<Vec<crate::outbox::Answering>>,
    fates: Mutex<BTreeMap<String, StagedFate>>,
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
                    rules: None,
                }),
                sending: SendFacts {
                    from_name: "Sender Example".into(),
                    owner_participated: Some(false),
                    owners: vec!["owner@example.edu".into()],
                },
            }]),
            calls: Mutex::new(Vec::new()),
            results: Mutex::new(Vec::new()),
            unreadable: false,
            listing_failure: None,
            partial_header_accounts: BTreeSet::new(),
            hold_read: false,
            handled_root: None,
            in_window: true,
            gate: GateStatus::Passing,
            on_effect: Mutex::new(None),
            staged: Mutex::new(Vec::new()),
            answering: Mutex::new(Vec::new()),
            fates: Mutex::new(BTreeMap::new()),
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
    fn proves_header_absence(&self, account: &str) -> bool {
        !self.partial_header_accounts.contains(account)
    }
    async fn candidates(&self, account: &str, _: Pass, _: DateTime<Utc>) -> Result<Vec<String>> {
        ensure!(
            self.listing_failure.as_deref() != Some(account),
            "provider listing HTTP 503 unavailable"
        );
        if !self.in_window {
            return Ok(Vec::new());
        }
        Ok(self
            .candidates
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.message.account == account)
            .map(|c| c.message.thread_id.clone())
            .collect())
    }
    async fn read_thread(&self, _: &str, id: &str) -> Result<ThreadRead> {
        if self.hold_read {
            return Ok(ThreadRead::Held {
                reason: "owner addresses unknown".into(),
            });
        }
        ensure!(!self.unreadable, "provider HTTP 503 unavailable");
        self.candidates
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.message.thread_id == id)
            .cloned()
            .map(ThreadRead::from)
            .ok_or_else(|| anyhow::anyhow!("missing thread"))
    }
    async fn mailbox_action(&self, _: &Candidate, action: ActionVerb) -> ActionResult {
        self.effect(format!("{action:?}"))
    }
    async fn private_record(&self, _: &Candidate, _: &str, action: &Action) -> ActionResult {
        self.effect(format!("{:?}", action.verb))
    }
    async fn handled(&self, c: &Candidate) -> Result<()> {
        self.calls.lock().unwrap().push("Handled".into());
        if let Some(root) = &self.handled_root {
            crate::mail_triage::rule_records::update(
                &root.join(&c.message.account),
                &c.message.account,
                &c.message.thread_id,
                |flags| flags.handled_message = Some(c.message.provider_message_id.clone()),
            )?;
        }
        Ok(())
    }
    fn origin_gate(&self, _: &LoadedRule) -> GateStatus {
        self.gate
    }
    async fn stage_reply(
        &self,
        _: &Candidate,
        body: &str,
        answering: &crate::outbox::Answering,
    ) -> Result<String, HeldReason> {
        self.answering.lock().unwrap().push(answering.clone());
        let mut staged = self.staged.lock().unwrap();
        staged.push(body.into());
        Ok(format!("item-{}", staged.len()))
    }
    fn staged_fate(&self, item: &str) -> StagedFate {
        self.fates
            .lock()
            .unwrap()
            .get(item)
            .copied()
            .unwrap_or(StagedFate::Waiting)
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
async fn missing_sink_retries_without_consuming_keys_caps_or_brakes() {
    let mut f = Fixture::new();
    let r = f.rule("task", TASK);
    f.mode(&r, ActionGroup::Board, Mode::Auto);
    f.options.pass = Pass::AfterClassify;
    for _ in 0..4 {
        let report = f.run(None).await;
        assert_eq!(report.rows.len(), 1);
        assert_eq!(report.rows[0].outcome, Outcome::Held);
        assert_eq!(report.rows[0].reason, "task sink unavailable");
        assert!(!report.rows[0].closes);
        assert!(f.store.ledger().unwrap().is_empty());
        assert_eq!(f.store.deferred().unwrap().len(), 1);
    }
    let sink = Sink::default();
    assert_eq!(f.run(Some(&sink)).await.rows[0].outcome, Outcome::Done);
    assert_eq!(sink.0.lock().unwrap().len(), 1);
    f.mail.candidates.lock().unwrap()[0]
        .message
        .provider_message_id = "new-message".into();
    assert_eq!(
        f.run(Some(&sink)).await.rows[0].reason,
        "thread already handled"
    );
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
    assert!(report.rows.iter().any(|r| r.reason == FATE_UNKNOWN_REASON));
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
            let report = f.run(None).await;
            assert_eq!(report.action_failures, 1);
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
    assert!(report.rows.iter().any(|r| r.outcome == Outcome::Held
        && !r.closes
        && r.reason == "thread unreadable"
        && r.failure_kind == FailureKind::Environment));
    assert_eq!(f.store.deferred().unwrap().len(), 1);
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
    for index in 0..12 {
        f.store
            .install(
                &format!("description-{index}"),
                "description = 'Notice'\n[match]\ndescribe = 'A notice'\n[[action]]\ndo = 'task'\n",
                &BTreeMap::new(),
            )
            .unwrap();
    }
    // Admission refuses a thirteenth; a hand-made one is discovered by the
    // loader and held rather than admitted because its files exist (§4.2).
    let hand = f.store.root().join("rules").join("description-12");
    crate::create_private_dir(&hand).unwrap();
    std::fs::write(
        hand.join("rule.toml"),
        "description = 'Notice'\n[match]\ndescribe = 'A notice'\n[[action]]\ndo = 'task'\n",
    )
    .unwrap();
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
            row_ids: HashSet::new(),
            report_row_ids: HashSet::new(),
            ledger_cursor: Default::default(),
            mode_cache: BTreeMap::new(),
            checked_inputs: BTreeMap::from([(r.name.clone(), r.approval_digest.clone())]),
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
            rules: None,
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

#[tokio::test]
async fn unknown_owner_read_holds_without_closing_deferred_board_occasions() {
    let mut f = Fixture::new();
    let rule = f.rule("task", TASK);
    f.mode(&rule, ActionGroup::Board, Mode::Auto);
    f.run(None).await;
    assert_eq!(f.store.deferred().unwrap().len(), 1);
    let before = f.store.ledger().unwrap().len();
    f.options.pass = Pass::AfterClassify;
    f.mail.hold_read = true;
    let sink = Sink::default();
    let report = f.run(Some(&sink)).await;
    assert_eq!(report.rows.len(), 1);
    assert_eq!(report.rows[0].outcome, Outcome::Held);
    assert!(!report.rows[0].closes);
    assert_eq!(report.rows[0].reason, "owner addresses unknown");
    assert!(!report
        .notices
        .iter()
        .any(|n| n.contains("thread unreadable")));
    assert_eq!(f.store.ledger().unwrap().len(), before);
    assert_eq!(f.store.deferred().unwrap().len(), 1);
    assert!(sink.0.lock().unwrap().is_empty());
    f.mail.hold_read = false;
    assert_eq!(f.run(Some(&sink)).await.rows[0].outcome, Outcome::Done);
    assert_eq!(sink.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn missing_sink_stays_nonclosing_when_caps_reset_at_local_midnight() {
    let mut f = Fixture::new();
    f.options.timezone = chrono_tz::Asia::Tokyo;
    let before = (Utc::now().date_naive() + Duration::days(2))
        .and_hms_opt(14, 59, 59)
        .unwrap()
        .and_utc();
    f.clock.set(before);
    let rule = f.store.install("task", &format!("description = 'Notice'\ndaily_cap = 1\n[match]\nfrom_domain = ['example.com']\n[[action]]\ndo = 'tag'\ntag = 'notice'\n{TASK}"), &BTreeMap::new()).unwrap();
    f.mode(&rule, ActionGroup::Board, Mode::Auto);
    f.mode(&rule, ActionGroup::PrivateRecord, Mode::Auto);
    let mut previous = f.mail.candidates.lock().unwrap()[0].message.clone();
    previous.provider_message_id = "previous-message".into();
    previous.thread_id = "previous-thread".into();
    f.store
        .append_ledger(
            &LedgerRow::header(
                &rule,
                1,
                previous,
                Mode::Auto,
                GateStatus::Passing,
                Outcome::Done,
                before - Duration::minutes(1),
            )
            .unwrap(),
        )
        .unwrap();
    {
        let mut cs = f.mail.candidates.lock().unwrap();
        cs[0].arrival_at = Some(before - Duration::minutes(1));
        cs[0].classified.as_mut().unwrap().at = before;
    }
    f.options.pass = Pass::AfterClassify;
    let clock = f.clock.clone();
    *f.mail.on_effect.lock().unwrap() = Some(Box::new(move || clock.advance(Duration::seconds(2))));
    let report = f.run(None).await;
    let held = report
        .rows
        .iter()
        .find(|r| r.action.verb == ActionVerb::Task)
        .unwrap();
    assert_eq!(held.outcome, Outcome::Held);
    assert_eq!(held.reason, "task sink unavailable");
    assert!(!held.closes);
    assert_eq!(held.failure_kind, FailureKind::Environment);
    assert!(!f
        .store
        .ledger()
        .unwrap()
        .iter()
        .any(|r| r.message.thread_id == "thread-alpha" && r.action.verb == ActionVerb::Task));
    let sink = Sink::default();
    f.run(Some(&sink)).await;
    assert_eq!(
        sink.0.lock().unwrap().len(),
        1,
        "new-day capacity and occasion remain available"
    );
}

#[tokio::test]
async fn provider_read_failures_preserve_board_retry_and_terminal_expiry() {
    for recover in [true, false] {
        let mut f = Fixture::new();
        std::fs::write(f.store.root().join("limits.toml"), "daily_cap = 1").unwrap();
        let rule = f.store.install("task", &format!("description = 'Review notice'\ndaily_cap = 1\n[match]\nfrom_domain = ['example.com']\n{TASK}"), &BTreeMap::new()).unwrap();
        f.mode(&rule, ActionGroup::Board, Mode::Auto);
        f.run(None).await;
        let queued = f.store.deferred().unwrap().remove(0);
        let ledger_before = f.store.ledger().unwrap().len();
        f.options.pass = Pass::AfterClassify;
        f.mail.unreadable = true;
        let sink = Sink::default();
        for _ in 0..3 {
            let report = f.run(Some(&sink)).await;
            assert_eq!(report.rows.len(), 1);
            assert_eq!(report.rows[0].outcome, Outcome::Held);
            assert!(!report.rows[0].closes);
            assert_eq!(report.rows[0].failure_kind, FailureKind::Environment);
            assert!(report
                .notices
                .iter()
                .any(|n| n.contains("thread unreadable")));
            assert_eq!(serde_json::to_value(&report).unwrap()["read_failures"], 1);
            assert_eq!(f.store.ledger().unwrap().len(), ledger_before);
            assert_eq!(f.store.deferred().unwrap()[0].id, queued.id);
            assert!(sink.0.lock().unwrap().is_empty());
        }
        if recover {
            f.mail.unreadable = false;
            let report = f.run(Some(&sink)).await;
            assert_eq!(report.rows[0].outcome, Outcome::Done);
            assert!(f.store.deferred().unwrap().is_empty());
            f.run(Some(&sink)).await;
            assert_eq!(
                sink.0.lock().unwrap().len(),
                1,
                "recovery creates exactly once"
            );
        } else {
            f.clock.set(queued.queued_at + Duration::days(7));
            f.run(Some(&sink)).await;
            assert_eq!(f.store.deferred().unwrap().len(), 1);
            f.clock.advance(Duration::seconds(1));
            f.run(Some(&sink)).await;
            assert!(f.store.deferred().unwrap().is_empty());
            assert!(f
                .store
                .ledger()
                .unwrap()
                .iter()
                .any(|r| r.outcome == Outcome::Expired && r.closes));
            f.mail.unreadable = false;
            f.run(Some(&sink)).await;
            assert!(
                sink.0.lock().unwrap().is_empty(),
                "expiry cannot be revived by recovery"
            );
            assert!(f.store.enqueue(&queued).is_err());
        }
    }
}

#[tokio::test]
async fn read_failure_count_distinguishes_errors_from_observation_holds() {
    let mut f = Fixture::new();
    f.rule("task", TASK);
    f.mail.unreadable = true;
    let failed = f.run(None).await;
    assert_eq!(serde_json::to_value(&failed).unwrap()["read_failures"], 1);
    assert!(
        f.store.deferred().unwrap().is_empty(),
        "header read failure cannot authorize a snapshot"
    );
    f.mail.unreadable = false;
    f.mail.hold_read = true;
    let held = f.run(None).await;
    assert_eq!(serde_json::to_value(&held).unwrap()["read_failures"], 0);
}

#[tokio::test]
async fn mode_transactions_are_bounded_by_rules_not_threads_or_actions() {
    for thread_count in 1..=8 {
        let f = Fixture::new();
        let r = f.rule(
            "notice",
            "[[action]]\ndo = 'read'\n[[action]]\ndo = 'tag'\ntag = 'notice'\n",
        );
        f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
        f.mode(&r, ActionGroup::PrivateRecord, Mode::Auto);
        let original = f.mail.candidates.lock().unwrap()[0].clone();
        *f.mail.candidates.lock().unwrap() = (0..thread_count)
            .map(|index| {
                let mut c = original.clone();
                c.message.thread_id = format!("thread-{index}");
                c.message.provider_message_id = format!("message-{index}");
                c
            })
            .collect();
        f.store
            .mode_transactions
            .store(0, std::sync::atomic::Ordering::Relaxed);
        let report = f.run(None).await;
        assert_eq!(
            report
                .rows
                .iter()
                .filter(|r| r.outcome == Outcome::Done)
                .count(),
            thread_count * 2
        );
        let transactions = f
            .store
            .mode_transactions
            .load(std::sync::atomic::Ordering::Relaxed);
        assert!(
            transactions <= 2,
            "one rule used {transactions} mode transactions for {thread_count} threads"
        );
    }
}

#[tokio::test]
async fn concurrent_ledger_failures_still_brake_the_next_sibling() {
    let f = Fixture::new();
    let rule = f.rule(
        "notice",
        "[[action]]\ndo = 'tag'\ntag = 'notice'\n[[action]]\ndo = 'read'\n",
    );
    f.mode(&rule, ActionGroup::PrivateRecord, Mode::Auto);
    f.mode(&rule, ActionGroup::Mailbox, Mode::Auto);
    let store = RuleStore::open_existing(f.store.root()).unwrap();
    let r = rule.clone();
    let c = f.mail.candidates.lock().unwrap()[0].clone();
    let now = f.options.clock.now();
    *f.mail.on_effect.lock().unwrap() = Some(Box::new(move || {
        for index in 0..3 {
            let mut message = c.message.clone();
            message.thread_id = format!("other-thread-{index}");
            message.provider_message_id = format!("other-message-{index}");
            let mut row = LedgerRow::header(
                &r,
                1,
                message,
                Mode::Auto,
                GateStatus::Passing,
                Outcome::Failed,
                now,
            )
            .unwrap();
            row.failure_kind = FailureKind::Action;
            store.append_ledger(&row).unwrap();
        }
    }));
    let report = f.run(None).await;
    assert_eq!(f.mail.calls.lock().unwrap().as_slice(), ["Tag"]);
    assert!(report
        .rows
        .iter()
        .any(|r| r.action.verb == ActionVerb::Read && r.outcome == Outcome::WouldHave));
}

#[tokio::test]
async fn input_edit_during_io_lapses_before_dispatch_and_cannot_be_reverted_to_auto() {
    let f = Fixture::new();
    let r = f.rule(
        "notice",
        "[[action]]\ndo = 'tag'\ntag = 'notice'\n[[action]]\ndo = 'read'\n",
    );
    f.mode(&r, ActionGroup::PrivateRecord, Mode::Auto);
    f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
    let path = r.directory.join("extra.md");
    let edit = path.clone();
    *f.mail.on_effect.lock().unwrap() = Some(Box::new(move || {
        std::fs::write(edit, "Owner edit").unwrap()
    }));
    let report = f.run(None).await;
    assert!(report
        .rows
        .iter()
        .any(|r| r.reason == "rule inputs changed during apply"));
    assert_eq!(f.mail.calls.lock().unwrap().as_slice(), ["Tag"]);
    std::fs::remove_file(path).unwrap();
    f.mail.next("after-revert");
    let report = f.run(None).await;
    assert!(report.rows.iter().all(|r| r.outcome != Outcome::Done));
    assert_eq!(f.mail.calls.lock().unwrap().as_slice(), ["Tag"]);
}

#[tokio::test]
async fn concurrent_success_keeps_ledger_order_for_failure_brakes() {
    let f = Fixture::new();
    let rule = f.rule("notice", "[[action]]\ndo = 'read'\n");
    f.mode(&rule, ActionGroup::Mailbox, Mode::Auto);
    let original = f.mail.candidates.lock().unwrap()[0].clone();
    let now = f.options.clock.now();
    for label in ["prior-alpha", "prior-beta"] {
        let mut message = original.message.clone();
        message.provider_message_id = label.into();
        message.thread_id = label.into();
        let mut row = LedgerRow::header(
            &rule,
            0,
            message,
            Mode::Auto,
            GateStatus::Passing,
            Outcome::Failed,
            now,
        )
        .unwrap();
        row.failure_kind = FailureKind::Action;
        f.store.append_ledger(&row).unwrap();
    }
    *f.mail.candidates.lock().unwrap() = ["alpha", "beta", "gamma"]
        .into_iter()
        .map(|label| {
            let mut c = original.clone();
            c.message.provider_message_id = label.into();
            c.message.thread_id = label.into();
            c
        })
        .collect();
    *f.mail.results.lock().unwrap() = (0..3)
        .map(|_| ActionResult::Failed {
            kind: FailureKind::Action,
            reason: "fixture refusal".into(),
        })
        .collect();
    let store = RuleStore::open_existing(f.store.root()).unwrap();
    let r = rule.clone();
    *f.mail.on_effect.lock().unwrap() = Some(Box::new(move || {
        let mut message = original.message;
        message.provider_message_id = "concurrent-success".into();
        message.thread_id = "concurrent-success".into();
        store
            .append_ledger(
                &LedgerRow::header(
                    &r,
                    0,
                    message,
                    Mode::Auto,
                    GateStatus::Passing,
                    Outcome::Done,
                    now,
                )
                .unwrap(),
            )
            .unwrap();
    }));
    let report = f.run(None).await;
    assert_eq!(
        report
            .rows
            .iter()
            .filter(|r| r.outcome == Outcome::Failed)
            .count(),
        3
    );
    assert_eq!(
        resolve_mode(
            &rule,
            ActionGroup::Mailbox,
            &f.store.approvals().unwrap(),
            &P1Holds {
                origin_gate: GateStatus::Passing
            }
        )
        .effective,
        Mode::Shadow,
        "the concurrent success precedes all three new failures in ledger order"
    );
}

fn cursor_fixture_row(f: &Fixture, rule: &LoadedRule) -> LedgerRow {
    LedgerRow::header(
        rule,
        0,
        f.mail.candidates.lock().unwrap()[0].message.clone(),
        Mode::Shadow,
        GateStatus::Passing,
        Outcome::WouldHave,
        f.options.clock.now(),
    )
    .unwrap()
}

#[test]
fn ledger_snapshot_initial_sync_never_reparses_history() {
    use std::io::Write;
    use std::sync::atomic::Ordering;
    let f = Fixture::new();
    let rule = f.rule("notice", "[[action]]\ndo = 'read'\n");
    let row_count = 2048;
    let rows: Vec<_> = (0..row_count)
        .map(|_| cursor_fixture_row(&f, &rule))
        .collect();
    let path = f.store.root().join("ledger.jsonl");
    let mut file = std::fs::File::create(&path).unwrap();
    for row in &rows {
        serde_json::to_writer(&mut file, row).unwrap();
        file.write_all(b"\n").unwrap();
    }
    let (snapshot, mut cursor) = f.store.ledger_snapshot().unwrap();
    assert_eq!(snapshot.len(), row_count);
    assert_eq!(
        f.store.ledger_rows_parsed.load(Ordering::Relaxed),
        row_count
    );
    let updates = f.store.ledger_updates(&mut cursor).unwrap();
    assert_eq!(
        f.store.ledger_rows_parsed.load(Ordering::Relaxed),
        row_count,
        "initial sync reparsed the existing history"
    );
    assert!(updates.is_empty());
    // An append after the snapshot must still be observed exactly once.
    let appended = cursor_fixture_row(&f, &rule);
    serde_json::to_writer(&mut file, &appended).unwrap();
    file.write_all(b"\n").unwrap();
    let updates = f.store.ledger_updates(&mut cursor).unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].id, appended.id);
    assert!(f.store.ledger_updates(&mut cursor).unwrap().is_empty());
    assert_eq!(
        f.store.ledger_rows_parsed.load(Ordering::Relaxed),
        row_count + 1
    );
}

#[test]
fn ledger_cursor_waits_for_partial_line_then_consumes_it_once() {
    use std::io::Write;
    let f = Fixture::new();
    let rule = f.rule("notice", "[[action]]\ndo = 'read'\n");
    let first = cursor_fixture_row(&f, &rule);
    let second = cursor_fixture_row(&f, &rule);
    let path = f.store.root().join("ledger.jsonl");
    let mut file = std::fs::File::create(&path).unwrap();
    serde_json::to_writer(&mut file, &first).unwrap();
    file.write_all(b"\n").unwrap();
    let bytes = serde_json::to_vec(&second).unwrap();
    let split = bytes.len() / 2;
    file.write_all(&bytes[..split]).unwrap();
    let mut cursor = store::LedgerCursor::default();
    let updates = f
        .store
        .ledger_updates(&mut cursor)
        .expect("partial append is not corruption");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].id, first.id);
    assert!(f.store.ledger_updates(&mut cursor).unwrap().is_empty());
    file.write_all(&bytes[split..]).unwrap();
    // A complete object without its newline is still uncommitted to this reader.
    assert!(f.store.ledger_updates(&mut cursor).unwrap().is_empty());
    file.write_all(b"\n").unwrap();
    let updates = f.store.ledger_updates(&mut cursor).unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].id, second.id);
    assert!(f.store.ledger_updates(&mut cursor).unwrap().is_empty());
    file.write_all(b"{invalid}\n").unwrap();
    assert!(
        f.store.ledger_updates(&mut cursor).is_err(),
        "complete invalid rows must refuse"
    );
}

#[tokio::test]
async fn changed_off_rule_emits_no_lapse_notice() {
    let f = Fixture::new();
    let active = f.rule("active", "[[action]]\ndo = 'tag'\ntag = 'notice'\n");
    let inactive = f.rule("inactive", "[[action]]\ndo = 'read'\n");
    f.mode(&active, ActionGroup::PrivateRecord, Mode::Auto);
    f.mode(&inactive, ActionGroup::Mailbox, Mode::Off);
    let path = inactive.directory.join("extra.md");
    *f.mail.on_effect.lock().unwrap() = Some(Box::new(move || {
        std::fs::write(path, "Owner edit to an inactive rule").unwrap();
    }));
    let mut next = f.mail.candidates.lock().unwrap()[0].clone();
    next.message.thread_id = "thread-next".into();
    next.message.provider_message_id = "message-next".into();
    f.mail.candidates.lock().unwrap().push(next);
    let report = f.run(None).await;
    assert_eq!(f.mail.calls.lock().unwrap().len(), 2);
    assert!(f
        .store
        .ledger()
        .unwrap()
        .iter()
        .all(|row| row.outcome != Outcome::ApprovalLapsed));
    assert!(
        report
            .notices
            .iter()
            .all(|notice| !notice.contains("approval lapsed")),
        "an Off rule cannot lapse: {:?}",
        report.notices
    );
}

#[tokio::test]
async fn handled_write_failure_preserves_effects_and_continues_other_accounts() {
    use crate::mail_triage::rule_records;
    let mut f = Fixture::new();
    let rule = f.rule("archive", ARCHIVE);
    f.mode(&rule, ActionGroup::Mailbox, Mode::Auto);
    let annotations = f.root.join("annotations");
    let broken = annotations.join("campus");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("rule-records"), "not a directory").unwrap();
    f.mail.handled_root = Some(annotations.clone());
    let mut next = f.mail.candidates.lock().unwrap()[0].clone();
    next.message.account = "project".into();
    next.message.thread_id = "thread-beta".into();
    next.message.provider_message_id = "message-beta".into();
    f.mail.candidates.lock().unwrap().push(next);
    let accounts = ["campus".into(), "project".into()];
    let report = apply(&f.store, &f.mail, None, &accounts, &f.options)
        .await
        .expect("marker failure must not discard effects or abort the other account");
    assert_eq!(
        report
            .rows
            .iter()
            .filter(|r| r.outcome == Outcome::Done)
            .count(),
        2
    );
    assert_eq!(report.handled_failures, 1);
    assert_eq!(report.handled, ["thread-beta"]);
    assert!(report
        .notices
        .iter()
        .any(|notice| notice.contains("campus") && notice.contains("thread-alpha")));
    assert!(f
        .store
        .ledger()
        .unwrap()
        .iter()
        .all(|row| !row.is_action_failure()));
    assert!(rule_records::read(&broken, "campus", "thread-alpha").is_err());
    // Restoring the annotation store exposes no skip marker for the failed write.
    std::fs::remove_file(broken.join("rule-records")).unwrap();
    assert!(rule_records::read(&broken, "campus", "thread-alpha")
        .unwrap()
        .handled_message
        .is_none());
    let report = apply(&f.store, &f.mail, None, &accounts, &f.options)
        .await
        .unwrap();
    assert_eq!(report.handled_failures, 0);
    assert!(report.handled.contains(&"thread-alpha".into()));
    assert_eq!(
        rule_records::read(&broken, "campus", "thread-alpha")
            .unwrap()
            .handled_message
            .as_deref(),
        Some("message-alpha")
    );
    assert_eq!(
        f.mail
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| *call == "Archive")
            .count(),
        2,
        "recovering the marker must not repeat mailbox effects"
    );
}

async fn assert_listing_failure_isolated(reverse: bool, expired: bool, dry_run: bool, pass: Pass) {
    let mut f = Fixture::new();
    let task_rule = f.store.install("task", "description = 'Review notice'\n[match]\naccount = ['campus']\nfrom_domain = ['example.com']\n[[action]]\ndo = 'task'", &BTreeMap::new()).unwrap();
    f.mode(&task_rule, ActionGroup::Board, Mode::Auto);
    f.run(None).await;
    let deferred_path = f.store.root().join("deferred.jsonl");
    let before = std::fs::read(&deferred_path).unwrap();
    assert_eq!(f.store.deferred().unwrap().len(), 1);
    if expired {
        f.clock.advance(Duration::days(8));
    }
    let read_rule = f.store.install("notice", "description = 'Review notice'\n[match]\naccount = ['project']\nfrom_domain = ['example.com']\n[[action]]\ndo = 'read'", &BTreeMap::new()).unwrap();
    f.mode(&read_rule, ActionGroup::Mailbox, Mode::Auto);
    let mut next = f.mail.candidates.lock().unwrap()[0].clone();
    next.message.account = "project".into();
    next.message.thread_id = "thread-beta".into();
    next.message.provider_message_id = "message-beta".into();
    next.arrival_at = Some(f.options.clock.now() - Duration::minutes(1));
    next.classified.as_mut().unwrap().at = f.options.clock.now();
    f.mail.candidates.lock().unwrap().push(next);
    f.mail.listing_failure = Some("campus".into());
    f.options.pass = pass;
    f.options.dry_run = dry_run;
    let mut accounts = ["project".into(), "campus".into()];
    if reverse {
        accounts.reverse();
    }
    let result = apply(&f.store, &f.mail, None, &accounts, &f.options).await;
    assert_eq!(
        std::fs::read(&deferred_path).unwrap(),
        before,
        "a failed account listing must not expire or settle its deferred entries"
    );
    let report =
        result.expect("a listing failure must preserve effects and continue other accounts");
    assert_eq!(report.candidate_failures, 1);
    assert_eq!(report.read_failures, 0);
    assert_eq!(report.handled_failures, 0);
    assert!(report
        .notices
        .iter()
        .any(|notice| notice.contains("campus") && notice.contains("listing")));
    assert_eq!(report.rows.len(), 1);
    assert_eq!(report.rows[0].outcome, Outcome::Done);
    assert_eq!(report.rows[0].message.account, "project");
    assert_eq!(
        f.mail
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| *call == "Read")
            .count(),
        usize::from(!dry_run)
    );
    assert!(f
        .store
        .ledger()
        .unwrap()
        .iter()
        .all(|r| r.message.account != "campus" || r.outcome != Outcome::Expired));
    if expired {
        // A later successful listing, even empty, allows terminal expiry again.
        f.options.dry_run = false;
        f.mail.listing_failure = None;
        f.mail.candidates.lock().unwrap().clear();
        apply(&f.store, &f.mail, None, &accounts, &f.options)
            .await
            .unwrap();
        assert!(f.store.deferred().unwrap().is_empty());
        assert!(f
            .store
            .ledger()
            .unwrap()
            .iter()
            .any(|r| r.message.account == "campus" && r.outcome == Outcome::Expired && r.closes));
    }
}

#[tokio::test]
async fn candidate_listing_failure_preserves_effects_in_both_account_orders() {
    for reverse in [false, true] {
        for pass in [Pass::Headers, Pass::AfterClassify] {
            assert_listing_failure_isolated(reverse, false, false, pass).await;
        }
    }
}

#[tokio::test]
async fn candidate_listing_failure_preserves_expired_queue_in_live_and_dry_runs() {
    for reverse in [false, true] {
        for dry_run in [false, true] {
            for pass in [Pass::Headers, Pass::AfterClassify] {
                assert_listing_failure_isolated(reverse, true, dry_run, pass).await;
            }
        }
    }
}

#[tokio::test]
async fn unsupported_dispatch_reports_a_hold_without_closing_the_action() {
    let f = Fixture::new();
    let rule = f.rule("archive", ARCHIVE);
    f.mode(&rule, ActionGroup::Mailbox, Mode::Auto);
    f.mail.results.lock().unwrap().push(unsupported_action());
    let report = f.run(None).await;
    assert_eq!(report.rows.len(), 1);
    assert_eq!(report.rows[0].outcome, Outcome::Held);
    assert!(!report.rows[0].closes);
    assert!(report.handled.is_empty());
    assert!(f
        .store
        .ledger()
        .unwrap()
        .iter()
        .all(|row| row.outcome != Outcome::Held && !row.is_action_failure()));
    let report = f.run(None).await;
    assert_eq!(
        report.rows[0].outcome,
        Outcome::Done,
        "the unused key stays available"
    );
}

fn header_capability_fixture(header_key: &str, positive: bool, proves_absence: bool) -> Fixture {
    header_capability_fixture_actions(header_key, positive, proves_absence, DIGEST)
}

fn header_capability_fixture_actions(
    header_key: &str,
    positive: bool,
    proves_absence: bool,
    actions: &str,
) -> Fixture {
    let mut f = Fixture::new();
    let key = if header_key == "list_id" {
        "list_id = ['news.example.com']"
    } else {
        "header.X-Notice = ['invoice']"
    };
    let raw = f.store.install("raw", &format!("description = 'Review notice'\n[match]\nfrom = ['sender@example.com']\n{key}\n{actions}"), &BTreeMap::new()).unwrap();
    if raw
        .definition
        .groups()
        .contains(&ActionGroup::PrivateRecord)
    {
        f.mode(&raw, ActionGroup::PrivateRecord, Mode::Auto);
    }
    let archive = f.rule("archive", ARCHIVE);
    f.mode(&archive, ActionGroup::Mailbox, Mode::Auto);
    if !proves_absence {
        f.mail.partial_header_accounts.insert("campus".into());
    }
    let mut message = f.mail.candidates.lock().unwrap()[0].clone();
    message.headers_complete = proves_absence.then_some(true);
    message.list_id = positive.then(|| "News <news.example.com>".into());
    message.headers.clear();
    if positive {
        message
            .headers
            .insert("x-notice".into(), vec!["invoice".into()]);
    }
    *f.mail.candidates.lock().unwrap() = ["alpha", "beta"]
        .into_iter()
        .map(|suffix| {
            let mut c = message.clone();
            c.message.thread_id = format!("thread-{suffix}");
            c.message.provider_message_id = format!("message-{suffix}");
            c
        })
        .collect();
    f
}

#[tokio::test]
async fn truncated_header_provider_holds_outbound_rules_without_blocking_other_rules() {
    for key in ["list_id", "header"] {
        for positive in [false, true] {
            let f = header_capability_fixture_actions(
                key,
                positive,
                false,
                "[[action]]\ndo = 'forward'\nto = ['finance@example.edu']\n",
            );
            for _ in 0..2 {
                let report = f.run(None).await;
                assert_eq!(report.rows.len(), 2);
                assert!(report.rows.iter().all(|row| row.rule == "archive"
                    && matches!(row.outcome, Outcome::Done | Outcome::Skipped)));
                assert_eq!(report.handled.len(), 2);
                assert_eq!(report.notices.len(), 1);
                assert!(
                    report.notices[0].contains("campus")
                        && report.notices[0].contains("raw")
                        && report.notices[0].contains("header absence")
                );
                assert!(f.store.deferred().unwrap().is_empty());
                assert!(f
                    .store
                    .ledger()
                    .unwrap()
                    .iter()
                    .all(|row| row.rule != "raw"));
            }
            assert_eq!(
                f.mail
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|call| *call == "Archive")
                    .count(),
                2
            );
        }
    }
}

#[tokio::test]
async fn complete_header_provider_retains_matching_deferral_and_known_negatives() {
    for key in ["list_id", "header"] {
        for positive in [false, true] {
            let mut f = header_capability_fixture(key, positive, true);
            let report = f.run(None).await;
            assert!(report.notices.is_empty());
            if positive {
                assert!(report.rows.is_empty());
                assert!(report.handled.is_empty());
                assert_eq!(f.store.deferred().unwrap().len(), 2);
                f.options.pass = Pass::AfterClassify;
                let report = f.run(None).await;
                assert_eq!(report.rows.len(), 4);
                assert!(report.rows.iter().all(|row| row.outcome == Outcome::Done));
            } else {
                assert_eq!(report.handled.len(), 2);
                assert_eq!(report.rows.len(), 2);
                assert!(report
                    .rows
                    .iter()
                    .all(|row| row.rule == "archive" && row.outcome == Outcome::Done));
            }
            assert!(f.store.deferred().unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn header_capability_hold_is_account_scoped_and_preserves_old_deferred_work() {
    let mut f = header_capability_fixture_actions(
        "list_id",
        true,
        true,
        "[[action]]\ndo = 'forward'\nto = ['finance@example.edu']\n",
    );
    f.run(None).await; // Older pass queued both the outbound action and removal.
    f.mail.partial_header_accounts.insert("campus".into());
    f.options.pass = Pass::AfterClassify;
    let report = f.run(None).await;
    assert_eq!(report.rows.len(), 2);
    assert!(report
        .rows
        .iter()
        .all(|row| row.rule == "archive" && row.outcome == Outcome::Done));
    let queued = f.store.deferred().unwrap();
    assert_eq!(queued.len(), 2);
    assert!(queued
        .iter()
        .all(|entry| entry.actions.len() == 1 && entry.actions[0].rule == "raw"));
    let before = std::fs::read(f.store.root().join("deferred.jsonl")).unwrap();
    f.clock.advance(Duration::days(8));
    f.mail.candidates.lock().unwrap().clear();
    let report = f.run(None).await;
    assert!(report.rows.is_empty());
    assert_eq!(report.notices.len(), 1);
    assert_eq!(
        std::fs::read(f.store.root().join("deferred.jsonl")).unwrap(),
        before
    );

    let f = header_capability_fixture_actions(
        "list_id",
        true,
        false,
        "[[action]]\ndo = 'forward'\nto = ['finance@example.edu']\n",
    );
    let mut other = f.mail.candidates.lock().unwrap()[0].clone();
    other.message.account = "project".into();
    other.message.thread_id = "other-thread".into();
    other.headers_complete = Some(true);
    f.mail.candidates.lock().unwrap().push(other);
    let report = apply(
        &f.store,
        &f.mail,
        None,
        &["campus".into(), "project".into()],
        &f.options,
    )
    .await
    .unwrap();
    assert_eq!(report.handled.len(), 2);
    assert_eq!(report.notices.len(), 1);
    let queued = f.store.deferred().unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].account, "project");
}

#[tokio::test]
async fn incomplete_message_on_capable_provider_still_holds_per_message() {
    let f = header_capability_fixture("list_id", false, true);
    for c in f.mail.candidates.lock().unwrap().iter_mut() {
        c.headers_complete = None;
    }
    let report = f.run(None).await;
    assert!(report.handled.is_empty());
    assert!(!report.rows.is_empty());
    assert!(report
        .rows
        .iter()
        .all(|row| row.outcome == Outcome::Held && !row.closes));
    assert_eq!(f.store.deferred().unwrap().len(), 2);
    assert!(report.notices.is_empty());
}

#[tokio::test]
async fn live_expiry_rows_equal_the_dry_run_plan() {
    let mut f = Fixture::new();
    f.rule("digest", DIGEST);
    f.run(None).await;
    f.clock.advance(Duration::days(8));
    f.mail.candidates.lock().unwrap().clear();
    f.options.dry_run = true;
    let plan = f.run(None).await;
    assert_eq!(plan.rows.len(), 1);
    f.options.dry_run = false;
    let live = f.run(None).await;
    assert_eq!(
        serde_json::to_value(&live.rows).unwrap(),
        serde_json::to_value(&plan.rows).unwrap()
    );
}

#[tokio::test]
async fn unknown_activation_neither_enqueues_nor_dispatches_and_notices_once() {
    let f = Fixture::new();
    let r = f.rule("digest", DIGEST);
    f.mode(&r, ActionGroup::PrivateRecord, Mode::Auto);
    let path = f.store.root().join("approvals.jsonl");
    let rows = f
        .store
        .approvals()
        .unwrap()
        .into_iter()
        .map(|mut row| {
            row.at = None;
            serde_json::to_string(&row).unwrap()
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(path, rows).unwrap();
    for _ in 0..2 {
        let report = f.run(None).await;
        assert!(
            f.store.deferred().unwrap().is_empty(),
            "unknown activation enqueued work"
        );
        assert!(
            report.rows.is_empty(),
            "standing holds must not repeat per message"
        );
        assert_eq!(
            report
                .notices
                .iter()
                .filter(|s| s.contains("activation unknown"))
                .count(),
            1
        );
        assert!(f.mail.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn live_rule_filter_is_refused_before_sibling_safeguards_can_be_lost() {
    let mut f = Fixture::new();
    let archive = f.rule("archive", ARCHIVE);
    f.rule("digest", DIGEST);
    f.mode(&archive, ActionGroup::Mailbox, Mode::Auto);
    f.options.rules = vec![archive.name.clone()];
    let before = std::fs::read(f.store.root().join("approvals.jsonl")).unwrap();
    let result = apply(&f.store, &f.mail, None, &["campus".into()], &f.options).await;
    assert!(
        result.is_err(),
        "live rule filtering must refuse before effects"
    );
    assert!(result.unwrap_err().to_string().contains("dry-run"));
    assert!(f.mail.calls.lock().unwrap().is_empty());
    assert!(f.store.ledger().unwrap().is_empty());
    assert!(f.store.deferred().unwrap().is_empty());
    assert_eq!(
        before,
        std::fs::read(f.store.root().join("approvals.jsonl")).unwrap()
    );
    f.options.dry_run = true;
    let plan = f.run(None).await;
    assert!(!plan.rows.is_empty());
    assert!(plan.rows.iter().all(|row| row.rule == archive.name));
    assert!(f.mail.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn explicit_rule_selection_preserves_priority_order() {
    let mut f = Fixture::new();
    let early = f.store.install("z-early", "description = 'Notice'\npriority = 5\n[match]\nfrom_domain = ['example.com']\n[[action]]\ndo = 'read'\n", &BTreeMap::new()).unwrap();
    let later = f.store.install("a-later", "description = 'Notice'\npriority = 10\n[match]\nfrom_domain = ['example.com']\n[[action]]\ndo = 'archive'\n", &BTreeMap::new()).unwrap();
    f.mode(&early, ActionGroup::Mailbox, Mode::Auto);
    f.mode(&later, ActionGroup::Mailbox, Mode::Auto);
    f.options.rules = vec![later.name.clone(), early.name.clone(), later.name.clone()];
    f.options.dry_run = true;
    let report = f.run(None).await;
    let names: Vec<_> = report
        .rows
        .iter()
        .filter(|row| row.outcome == Outcome::Done)
        .map(|row| row.rule.as_str())
        .collect();
    assert_eq!(names, vec!["z-early", "a-later"]);
    assert!(f.mail.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn explicit_rule_selection_never_evaluates_or_sweeps_siblings() {
    let mut f = Fixture::new();
    f.rule("first", DIGEST);
    f.rule("second", DIGEST);
    f.run(None).await;
    assert_eq!(f.store.deferred().unwrap()[0].actions.len(), 2);
    f.clock.advance(Duration::days(8));
    f.mail.candidates.lock().unwrap().clear();
    f.options.rules = vec!["first".into()];
    f.options.dry_run = true;
    let plan = f.run(None).await;
    assert_eq!(plan.rows.len(), 1);
    assert_eq!(plan.rows[0].rule, "first");
    assert_eq!(f.store.deferred().unwrap()[0].actions.len(), 2);
    assert!(f.store.ledger().unwrap().is_empty());
}

#[test]
fn retrospective_match_includes_archived_mail_without_activation_or_claims() {
    let f = Fixture::new();
    let rule = f.rule("notice", ARCHIVE);
    let candidate = f.mail.candidates.lock().unwrap()[0].clone();
    let rows = test_match(&rule, &candidate, Some(false), f.options.clock.now()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, Outcome::WouldHave);
    assert_eq!(rows[0].in_inbox, Some(false));
    assert!(rows[0].reason.contains("no longer in the inbox"));
    assert!(!rows[0].closes);
    assert!(f.store.ledger().unwrap().is_empty());
    assert!(f.mail.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn unknown_activation_preserves_old_queue_and_does_not_defer_other_rules() {
    let mut f = Fixture::new();
    let digest = f.rule("digest", DIGEST);
    f.mode(&digest, ActionGroup::PrivateRecord, Mode::Auto);
    f.run(None).await;
    let queued = std::fs::read(f.store.root().join("deferred.jsonl")).unwrap();
    let archive = f.rule("archive", ARCHIVE);
    f.mode(&archive, ActionGroup::Mailbox, Mode::Auto);
    let approvals = f
        .store
        .approvals()
        .unwrap()
        .into_iter()
        .map(|mut row| {
            if row.rule == "digest" {
                row.at = None;
            }
            serde_json::to_string(&row).unwrap()
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(f.store.root().join("approvals.jsonl"), approvals).unwrap();
    f.clock.advance(Duration::days(8));
    for dry_run in [true, false] {
        f.options.dry_run = dry_run;
        let report = f.run(None).await;
        assert!(report.rows.iter().all(|row| row.rule != "digest"));
        assert!(report
            .rows
            .iter()
            .any(|row| row.rule == "archive" && row.outcome == Outcome::Done));
        assert_eq!(
            queued,
            std::fs::read(f.store.root().join("deferred.jsonl")).unwrap()
        );
    }
}

#[test]
fn retrospective_match_refuses_description_rules() {
    let f = Fixture::new();
    let rule = f.store.install("described", "description = 'Fictional notice'\n[match]\ndescribe = 'A fictional notice'\n[[action]]\ndo = 'archive'\n", &BTreeMap::new()).unwrap();
    let candidate = f.mail.candidates.lock().unwrap()[0].clone();
    assert!(
        test_match(&rule, &candidate, Some(true), f.options.clock.now())
            .unwrap_err()
            .to_string()
            .contains("not built yet")
    );
}

#[tokio::test]
async fn expiry_report_reuses_a_row_persisted_before_queue_rewrite() {
    let mut f = Fixture::new();
    f.rule("digest", DIGEST);
    f.run(None).await;
    let queued = std::fs::read(f.store.root().join("deferred.jsonl")).unwrap();
    f.clock.advance(Duration::days(8));
    f.mail.candidates.lock().unwrap().clear();
    f.store.expire_deferred(f.options.clock.now()).unwrap();
    // Reproduce a crash after appending the row but before rewriting the queue.
    std::fs::write(f.store.root().join("deferred.jsonl"), queued).unwrap();
    f.clock.advance(Duration::hours(1));
    f.options.dry_run = true;
    let plan = f.run(None).await;
    f.options.dry_run = false;
    let live = f.run(None).await;
    assert_eq!(
        serde_json::to_value(&plan.rows).unwrap(),
        serde_json::to_value(&live.rows).unwrap()
    );
}

#[tokio::test]
async fn p3_partial_headers_positive_archive_acts_and_unknown_writes_nothing() {
    for raw_key in [
        "list_id = ['news.example.com']",
        "header.X-Notice = ['update']",
    ] {
        let mut f = Fixture::new();
        f.mail.partial_header_accounts.insert("campus".into());
        let rule = f
            .store
            .install(
                "raw",
                &format!("description = 'Visible header notice'\n[match]\n{raw_key}\n{ARCHIVE}"),
                &BTreeMap::new(),
            )
            .unwrap();
        f.mode(&rule, ActionGroup::Mailbox, Mode::Auto);
        f.mail.candidates.lock().unwrap()[0].headers_complete = None;
        let report = f.run(None).await;
        assert_eq!(report.rows.len(), 1, "a visible positive header must act");
        assert_eq!(report.rows[0].outcome, Outcome::Done);
        assert_eq!(
            serde_json::to_value(&report.rows[0]).unwrap()["headers_partial"],
            true
        );
        assert_eq!(report.handled.len(), 1);
        assert!(report.notices.is_empty());
        let before = f.store.ledger().unwrap().len();
        f.mail.next("unknown");
        {
            let mut candidates = f.mail.candidates.lock().unwrap();
            candidates[0].list_id = None;
            candidates[0].headers.clear();
        }
        let report = f.run(None).await;
        assert!(report.rows.is_empty());
        assert!(report.notices.is_empty());
        assert!(report.handled.is_empty());
        assert!(report.deferred.is_empty());
        assert_eq!(f.store.ledger().unwrap().len(), before);
        assert!(f.store.deferred().unwrap().is_empty());
        assert_eq!(*f.mail.calls.lock().unwrap(), ["Archive", "Handled"]);
    }
}

#[tokio::test]
async fn p3_partial_headers_unknown_is_absent_before_deferral_and_handled_decisions() {
    for key in ["list_id", "header"] {
        let f = header_capability_fixture(key, false, false);
        let report = f.run(None).await;
        assert!(
            report.notices.is_empty(),
            "non-outbound unknown is not a standing hold"
        );
        assert_eq!(report.rows.len(), 2);
        assert!(report
            .rows
            .iter()
            .all(|row| row.rule == "archive" && row.outcome == Outcome::Done));
        assert_eq!(report.handled.len(), 2);
        assert!(report.deferred.is_empty());
        assert!(f.store.deferred().unwrap().is_empty());
        assert!(f
            .store
            .ledger()
            .unwrap()
            .iter()
            .all(|row| row.rule != "raw"));
    }
}

#[tokio::test]
async fn p3_partial_headers_unknown_does_not_admit_descriptions_or_settle_queued_positives() {
    let mut f = header_capability_fixture("list_id", true, false);
    f.run(None).await;
    assert_eq!(f.store.deferred().unwrap().len(), 2);
    f.options.pass = Pass::AfterClassify;
    for c in f.mail.candidates.lock().unwrap().iter_mut() {
        c.list_id = None;
        c.headers.clear();
    }
    let report = f.run(None).await;
    assert!(report.rows.iter().all(|r| r.rule == "archive"));
    assert_eq!(
        report
            .rows
            .iter()
            .filter(|r| r.outcome == Outcome::Done)
            .count(),
        2,
        "the deferred archive runs once the unknown raw rule is absent"
    );
    // Handled markers are a header-pass write; classification already ran.
    assert!(report.handled.is_empty());
    let queue = f.store.deferred().unwrap();
    assert_eq!(queue.len(), 2);
    assert!(queue.iter().all(|e| e.actions.len() == 1
        && e.actions[0].rule == "raw"
        && e.actions[0].headers_partial));
    assert!(f.store.ledger().unwrap().iter().all(|r| r.rule != "raw"));

    let mut f = Fixture::new();
    f.mail.partial_header_accounts.insert("campus".into());
    {
        let mut cs = f.mail.candidates.lock().unwrap();
        cs[0].headers_complete = None;
        cs[0].list_id = None;
    }
    for n in 0..13 {
        f.store.install(&format!("described-{n}"), "description='Fictional receipt'\n[match]\nlist_id=['news.example.com']\ndescribe='A receipt'\n[[action]]\ndo='digest'\n", &BTreeMap::new()).unwrap();
    }
    let archive = f.rule("archive", ARCHIVE);
    f.mode(&archive, ActionGroup::Mailbox, Mode::Auto);
    let report = f.run(None).await;
    // Description rules keep the account hold: list admission and the entry
    // cap are decided before a message is read, so a per-message exclusion
    // would let thirteen of them bypass the cap.
    assert!(report.list_cap.is_empty(), "held rules are not admitted");
    assert_eq!(report.notices.len(), 13);
    assert!(report
        .notices
        .iter()
        .all(|n| n.contains("held: provider cannot prove header absence")));
    assert!(report.deferred.is_empty());
    assert_eq!(report.handled.len(), 1);
    assert_eq!(report.rows.len(), 1);
    assert_eq!(report.rows[0].rule, "archive");
}

#[test]
fn partial_header_relief_is_for_non_outbound_header_rules_only() {
    let f = Fixture::new();
    let install = |name: &str, body: &str| f.store.install(name, body, &BTreeMap::new()).unwrap();
    let header = install(
        "h",
        &format!("description='Notice'\n[match]\nlist_id=['news.example.com']\n{ARCHIVE}"),
    );
    let described = install("d", "description='Notice'\n[match]\nlist_id=['news.example.com']\ndescribe='A notice'\n[[action]]\ndo='archive'\n");
    let outbound = install("o", "description='Notice'\n[match]\nlist_id=['news.example.com']\n[[action]]\ndo='forward'\nto=['finance@example.edu']\n");
    let plain = install(
        "p",
        &format!("description='Notice'\n[match]\nfrom_domain=['example.com']\n{ARCHIVE}"),
    );
    assert!(!super::header_provider_hold(&header, false));
    assert!(super::partial_header_rule(&header, false));
    assert!(super::header_provider_hold(&described, false));
    assert!(super::header_provider_hold(&outbound, false));
    for rule in [&header, &described, &outbound, &plain] {
        assert!(
            !super::header_provider_hold(rule, true),
            "full-header readers never hold"
        );
        assert!(!super::partial_header_rule(rule, true));
    }
    assert!(!super::header_provider_hold(&plain, false));
    assert!(!super::partial_header_rule(&plain, false));
}

#[tokio::test]
async fn p3_partial_headers_positive_non_outbound_defers_and_executes_like_full_headers() {
    for key in ["list_id", "header"] {
        let mut f = header_capability_fixture(key, true, false);
        let report = f.run(None).await;
        assert!(report.notices.is_empty());
        assert!(report.rows.is_empty());
        assert_eq!(f.store.deferred().unwrap().len(), 2);
        f.options.pass = Pass::AfterClassify;
        let report = f.run(None).await;
        assert_eq!(report.rows.len(), 4);
        assert!(report.rows.iter().all(|row| row.outcome == Outcome::Done));
        assert!(report
            .rows
            .iter()
            .all(|row| row.headers_partial == (row.rule == "raw")));
        assert!(f.store.deferred().unwrap().is_empty());
    }
}

#[tokio::test]
async fn an_observation_is_recorded_once_however_many_passes_see_it() {
    // The header timer runs every five minutes over a seven-day window, so a
    // shadow match or a pre-activation skip is seen ~2000 times. Recording
    // each sighting buried the page's ledger window under one message.
    let f = Fixture::new();
    let r = f.rule("archive", &format!("{ARCHIVE}[[action]]\ndo = 'read'\n"));
    for _ in 0..3 {
        f.run(None).await;
    }
    let rows = f.store.ledger().unwrap();
    assert_eq!(rows.len(), 2, "one would-have row per action: {rows:#?}");
    assert!(rows.iter().all(|r| r.outcome == Outcome::WouldHave));

    f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
    f.mail.candidates.lock().unwrap()[0].arrival_at = Some(Utc::now() - Duration::days(1));
    for _ in 0..3 {
        f.run(None).await;
    }
    let skipped: Vec<_> = f
        .store
        .ledger()
        .unwrap()
        .into_iter()
        .filter(|r| r.outcome == Outcome::Skipped)
        .collect();
    assert_eq!(skipped.len(), 2, "{skipped:#?}");
    assert!(skipped
        .iter()
        .all(|r| r.reason == "before the rule was active"));

    // A different observation of the same key is still a new fact: after an
    // edit the match digest changes, and the would-have row is recorded anew.
    f.mode(&r, ActionGroup::Mailbox, Mode::Shadow);
    std::fs::write(
        r.directory.join("rule.toml"),
        "description = 'Review notice'\n[match]\nfrom_domain = ['example.com']\nsubject_contains = ['Sender']\n[[action]]\ndo = 'archive'\n[[action]]\ndo = 'read'\n",
    )
    .unwrap();
    f.run(None).await;
    f.run(None).await;
    let would_have = f
        .store
        .ledger()
        .unwrap()
        .into_iter()
        .filter(|r| r.outcome == Outcome::WouldHave)
        .count();
    assert_eq!(would_have, 4);
}

#[tokio::test]
async fn an_already_handled_action_is_noted_once() {
    let f = Fixture::new();
    let r = f.rule(
        "read",
        "[[action]]\ndo = 'read'\n[[action]]\ndo = 'tag'\ntag = 'notices'\n",
    );
    f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
    f.mode(&r, ActionGroup::PrivateRecord, Mode::Auto);
    f.run(None).await;
    for _ in 0..3 {
        f.run(None).await;
    }
    let rows = f.store.ledger().unwrap();
    let skipped = rows
        .iter()
        .filter(|r| r.outcome == Outcome::Skipped && r.reason == "action already handled")
        .count();
    assert_eq!(skipped, 2, "{rows:#?}");
}

#[tokio::test]
async fn a_repeated_deferred_observation_settles_as_itself_and_adds_no_row() {
    // Review of #853: a shadow task re-defers on every headers pass, and its
    // settlement after classify reads the run's report. A repeat must still
    // be found there, as the row the ledger already holds.
    let mut f = Fixture::new();
    f.rule("task", TASK);
    for _ in 0..3 {
        f.options.pass = Pass::Headers;
        f.run(None).await;
        f.options.pass = Pass::AfterClassify;
        let report = f.run(None).await;
        assert!(
            report.rows.iter().all(|r| r.outcome == Outcome::WouldHave),
            "{:#?}",
            report.rows
        );
    }
    let rows = f.store.ledger().unwrap();
    assert_eq!(rows.len(), 1, "{rows:#?}");
    assert!(f.store.deferred().unwrap().is_empty());
    // A dry run reports what a live pass would, already recorded or not.
    f.options.dry_run = true;
    assert_eq!(f.run(None).await.rows.len(), 1);
}

// ---- description rules (P4a) ----

use crate::mail_triage::rule_frame::{Considered, RuleHit, RuleInput, RuleList, RuleRecord};

fn described_rule(f: &Fixture, name: &str, extra_match: &str, actions: &str) -> LoadedRule {
    f.store
        .install(
            name,
            &format!(
                "description = 'Bookings'\n[match]\ndescribe = 'Room booking confirmations'\n{extra_match}{actions}"
            ),
            &BTreeMap::new(),
        )
        .unwrap()
}

/// What the classifier stored for the fixture's message about `rule`.
fn told(f: &Fixture, rule: &LoadedRule, list: RuleList, named: bool, input: RuleInput) {
    let matcher = rule
        .match_digest(ActionGroup::Mailbox, GateStatus::Passing)
        .unwrap()
        .matcher;
    let hit = RuleHit {
        name: rule.name.clone(),
        quote: "booking for Friday".into(),
    };
    let mut cs = f.mail.candidates.lock().unwrap();
    cs[0].classified.as_mut().unwrap().rules = Some(RuleRecord {
        input,
        considered: vec![Considered {
            name: rule.name.clone(),
            list,
            matcher,
        }],
        active: (named && list == RuleList::Active).then(|| hit.clone()),
        shadow: if named && list == RuleList::Shadow {
            vec![hit]
        } else {
            Vec::new()
        },
        refused: Vec::new(),
        confirmation: None,
    });
}

#[tokio::test]
async fn a_description_match_writes_one_would_have_row_after_classify_with_its_quote() {
    let mut f = Fixture::new();
    let r = described_rule(&f, "bookings", "", ARCHIVE);
    // The header timer never evaluates a description rule.
    assert!(f.run(None).await.rows.is_empty());
    f.options.pass = Pass::AfterClassify;
    // Classified, but no description verdict: nothing is known, so nothing.
    assert!(f.run(None).await.rows.is_empty());
    told(&f, &r, RuleList::Shadow, false, RuleInput::Snippet);
    assert!(
        f.run(None).await.rows.is_empty(),
        "asked and not named is no match, and no row"
    );
    told(&f, &r, RuleList::Shadow, true, RuleInput::Snippet);
    for _ in 0..3 {
        f.run(None).await;
    }
    let rows = f.store.ledger().unwrap();
    assert_eq!(rows.len(), 1, "{rows:#?}");
    assert_eq!(rows[0].outcome, Outcome::WouldHave);
    assert_eq!(rows[0].verdict_input, VerdictInput::Snippet);
    assert!(rows[0].matched_because.contains("booking for Friday"));
    assert!(f.mail.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn only_the_active_list_can_authorise_an_active_action() {
    let mut f = Fixture::new();
    let r = described_rule(&f, "bookings", "", ARCHIVE);
    f.mode(&r, ActionGroup::Mailbox, Mode::Auto);
    f.options.pass = Pass::AfterClassify;
    // Named only from the shadow list (asked while the group was shadow).
    told(&f, &r, RuleList::Shadow, true, RuleInput::Snippet);
    assert!(f.run(None).await.rows.is_empty());
    assert!(f.mail.calls.lock().unwrap().is_empty());
    told(&f, &r, RuleList::Active, true, RuleInput::FullBody);
    let report = f.run(None).await;
    assert_eq!(report.rows[0].outcome, Outcome::Done, "{:#?}", report.rows);
    assert_eq!(report.rows[0].verdict_input, VerdictInput::FullBody);
    assert_eq!(*f.mail.calls.lock().unwrap(), vec!["Archive"]);
}

#[tokio::test]
async fn a_verdict_about_an_edited_description_is_not_evidence_for_the_new_one() {
    let mut f = Fixture::new();
    let r = described_rule(&f, "bookings", "", ARCHIVE);
    f.options.pass = Pass::AfterClassify;
    told(&f, &r, RuleList::Shadow, true, RuleInput::Snippet);
    std::fs::write(
        r.directory.join("rule.toml"),
        "description = 'Bookings'\n[match]\ndescribe = 'Invoices for room bookings'\n[[action]]\ndo = 'archive'\n",
    )
    .unwrap();
    assert!(f.run(None).await.rows.is_empty());
}

#[tokio::test]
async fn header_preconditions_still_bind_a_description_rule() {
    let mut f = Fixture::new();
    let r = described_rule(
        &f,
        "bookings",
        "from_domain = ['elsewhere.example']\n",
        ARCHIVE,
    );
    f.options.pass = Pass::AfterClassify;
    told(&f, &r, RuleList::Shadow, true, RuleInput::Snippet);
    assert!(f.run(None).await.rows.is_empty());
}

#[tokio::test]
async fn a_precondition_the_sweep_cannot_read_holds_the_rule_visibly() {
    let mut f = Fixture::new();
    let r = described_rule(&f, "bookings", "to = ['owner@example.edu']\n", ARCHIVE);
    f.options.pass = Pass::AfterClassify;
    told(&f, &r, RuleList::Shadow, true, RuleInput::Snippet);
    let report = f.run(None).await;
    assert!(report.rows.is_empty());
    assert!(report
        .notices
        .iter()
        .any(|n| n.starts_with("bookings: held:")));
    let views = rule_views(&f.store).unwrap();
    assert!(views[0].holds.iter().any(|h| h.contains("classify sweep")));
}

#[tokio::test]
async fn description_ceilings_follow_the_group() {
    let f = Fixture::new();
    let archive = described_rule(&f, "archive", "", ARCHIVE);
    let trash = described_rule(&f, "trash", "", "[[action]]\ndo = 'trash'\n");
    let task = described_rule(&f, "task", "", TASK);
    let forward = described_rule(
        &f,
        "forward",
        "from = ['sender@example.com']\n",
        "[[action]]\ndo = 'forward'\nto = ['finance@example.edu']\n",
    );
    assert_eq!(
        approvals::mode_ceiling(&archive, ActionGroup::Mailbox),
        Mode::Auto
    );
    assert_eq!(
        approvals::mode_ceiling(&trash, ActionGroup::Mailbox),
        Mode::Propose
    );
    // P4b-1: a description task is confirmed, so it may rise.
    assert_eq!(
        approvals::mode_ceiling(&task, ActionGroup::Board),
        Mode::Auto
    );
    assert_eq!(
        approvals::mode_ceiling(&forward, ActionGroup::VerbatimForward),
        Mode::Shadow
    );
}

#[test]
fn the_frame_lists_by_mode_and_priority_and_asks_only_the_eligible() {
    use super::super::describe::{ClassifierRules, Envelope};
    let f = Fixture::new();
    let late = f
        .store
        .install(
            "a-late",
            "description = 'Late'\npriority = 200\n[match]\ndescribe = 'Seminar notices'\n[[action]]\ndo = 'archive'\n",
            &BTreeMap::new(),
        )
        .unwrap();
    f.mode(&late, ActionGroup::Mailbox, Mode::Auto);
    f.store
        .install(
            "z-early",
            "description = 'Early'\npriority = 5\n[match]\ndescribe = 'Booking notices'\nfrom_domain = ['example.com']\n[[action]]\ndo = 'archive'\n[[action]]\ndo = 'forward'\nto = ['office@example.edu']\n",
            &BTreeMap::new(),
        )
        .unwrap();
    described_rule(&f, "held", "list_id = ['news.example.com']\n", ARCHIVE);
    f.rule("header-only", ARCHIVE);
    let rules = ClassifierRules::load(&f.store).unwrap();
    assert_eq!(
        rules
            .frame
            .active
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
        vec!["a-late"]
    );
    assert_eq!(
        rules
            .frame
            .shadow
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
        vec!["z-early"]
    );
    assert!(
        rules.held.iter().any(|h| h.starts_with("held: held:")),
        "{:?}",
        rules.held
    );
    let here = rules.ask(&Envelope {
        account: "campus",
        from: "someone@example.com",
        subject: "Friday",
        has_attachment: None,
    });
    assert_eq!(here.active, vec!["a-late".to_string()]);
    assert_eq!(here.shadow, vec!["z-early".to_string()]);
    let elsewhere = rules.ask(&Envelope {
        account: "campus",
        from: "someone@other.example",
        subject: "Friday",
        has_attachment: None,
    });
    assert!(elsewhere.shadow.is_empty(), "its precondition disproves it");
    // A shadow forward still needs full-body evidence; an archive does not.
    let named = |name: &str| crate::mail_triage::rule_frame::RuleReading {
        shadow: vec![RuleHit {
            name: name.into(),
            quote: "Friday notice".into(),
        }],
        ..Default::default()
    };
    assert!(rules.escalates(&named("z-early")));
    assert!(!rules.escalates(&named("a-late")));
}

#[test]
fn admission_refuses_what_would_overfill_the_lists_and_never_a_lowering() {
    let f = Fixture::new();
    for i in 0..12 {
        described_rule(&f, &format!("d{i:02}"), "", ARCHIVE);
    }
    let refused = f
        .store
        .install(
            "d12",
            "description = 'One more'\n[match]\ndescribe = 'More notices'\n[[action]]\ndo = 'archive'\n",
            &BTreeMap::new(),
        )
        .unwrap_err();
    assert!(
        format!("{refused:#}").contains("limited to 12"),
        "{refused:#}"
    );
    assert!(f.store.get("d12").is_err(), "nothing was written");
    // A header rule takes no list entry.
    f.rule("header-only", ARCHIVE);
    // Moving a one-group rule from shadow to auto keeps its one entry.
    let one = f.store.get("d00").unwrap();
    f.mode(&one, ActionGroup::Mailbox, Mode::Auto);
    // A rule with two groups gains an active entry if one is raised alone.
    f.mode(&one, ActionGroup::Mailbox, Mode::Shadow);
    let two_text = "description = 'Bookings'\n[match]\ndescribe = 'Room booking confirmations'\n[[action]]\ndo = 'archive'\n[[action]]\ndo = 'tag'\ntag = 'rooms'\n";
    let report = f.store.edit("d01", two_text);
    assert!(report.is_ok(), "an edit that adds no entry is admitted");
    let two = f.store.get("d01").unwrap();
    let raised = f.store.change_mode(
        "d01",
        ActionGroup::Mailbox,
        Mode::Auto,
        Some(Actor::Owner),
        &two.approval_digest,
        &P1Holds {
            origin_gate: GateStatus::Passing,
        },
    );
    let error = raised.unwrap_err();
    assert!(
        matches!(
            error.downcast_ref::<ModeError>(),
            Some(ModeError::ListCap(_))
        ),
        "{error:#}"
    );
}

#[tokio::test]
async fn a_description_row_can_be_marked_a_wrong_match_and_counts_toward_the_brake() {
    let mut f = Fixture::new();
    let r = described_rule(&f, "bookings", "", ARCHIVE);
    f.options.pass = Pass::AfterClassify;
    told(&f, &r, RuleList::Shadow, true, RuleInput::Snippet);
    f.run(None).await;
    let row = f.store.ledger().unwrap()[0].clone();
    let views = ledger_views(&f.store, Some("bookings"), 10).unwrap();
    assert_eq!(views[0]["wrong_match_allowed"], true);
    let report = f.store.wrong_match(&row.id, Actor::Owner).unwrap();
    assert!(report.changed);
    assert_eq!(
        brake_evidence("bookings", &f.store.ledger().unwrap()).wrong_matches,
        1
    );
}

// ---- the confirmation pass (P4b-1) ----

fn confirmed(f: &Fixture, choice: Option<&str>, shadow: &[(&str, bool)]) {
    use crate::mail_triage::confirm::{
        ActiveConfirmation, Confirmation, ShadowConfirmation, CONFIRMATION_VERSION,
    };
    let mut cs = f.mail.candidates.lock().unwrap();
    let rules = cs[0].classified.as_mut().unwrap().rules.as_mut().unwrap();
    rules.confirmation = Some(Confirmation {
        version: CONFIRMATION_VERSION.clone(),
        active: choice.map(|name| ActiveConfirmation {
            offered: vec![name.into()],
            choice: (name != "none").then(|| RuleHit {
                name: name.into(),
                quote: "booking for Friday".into(),
            }),
            ..Default::default()
        }),
        shadow: shadow
            .iter()
            .map(|(name, agrees)| ShadowConfirmation {
                name: (*name).into(),
                agrees: *agrees,
                quote: None,
                refused: None,
                answered: true,
                failed: None,
            })
            .collect(),
    });
}

#[tokio::test]
async fn an_automatic_description_task_needs_the_confirmation_pass_to_agree() {
    let mut f = Fixture::new();
    let r = described_rule(&f, "bookings", "", TASK);
    f.mode(&r, ActionGroup::Board, Mode::Auto);
    f.options.pass = Pass::AfterClassify;
    let sink = Sink::default();
    // The main pass named it, the confirmation pass chose none: proposed.
    told(&f, &r, RuleList::Active, true, RuleInput::Snippet);
    confirmed(&f, Some("none"), &[]);
    let report = f.run(Some(&sink)).await;
    assert_eq!(
        report.rows[0].outcome,
        Outcome::Offered,
        "{:#?}",
        report.rows
    );
    assert_eq!(report.rows[0].mode, Mode::Propose);
    assert_eq!(
        report.rows[0].reason,
        "unconfirmed: the confirmation pass did not choose this rule"
    );
    assert!(report.rows[0]
        .matched_because
        .contains(r#""confirmation":"none""#));
    assert!(sink.0.lock().unwrap().is_empty());

    // Both passes chose it: the task is created.
    f.mail.next("beta");
    told(&f, &r, RuleList::Active, true, RuleInput::Snippet);
    confirmed(&f, Some("bookings"), &[]);
    let report = f.run(Some(&sink)).await;
    assert_eq!(report.rows[0].outcome, Outcome::Done, "{:#?}", report.rows);
    assert_eq!(sink.0.lock().unwrap().len(), 1);

    // No confirmation at all (another build, or none recorded): proposed.
    f.mail.next("gamma");
    told(&f, &r, RuleList::Active, true, RuleInput::Snippet);
    let report = f.run(Some(&sink)).await;
    assert_eq!(report.rows[0].outcome, Outcome::Offered);
    assert_eq!(
        report.rows[0].reason, "unconfirmed: the confirmation pass gave no answer",
        "no answer is not a disagreement (review of #855, pass 2)"
    );
    assert_eq!(sink.0.lock().unwrap().len(), 1);

    // The call failed: proposed, and the row says so rather than "none".
    f.mail.next("delta");
    told(&f, &r, RuleList::Active, true, RuleInput::Snippet);
    confirmed(&f, Some("none"), &[]);
    {
        let mut cs = f.mail.candidates.lock().unwrap();
        let rules = cs[0].classified.as_mut().unwrap().rules.as_mut().unwrap();
        let active = rules
            .confirmation
            .as_mut()
            .unwrap()
            .active
            .as_mut()
            .unwrap();
        active.failed = Some("the model server did not answer".into());
    }
    let report = f.run(Some(&sink)).await;
    assert_eq!(report.rows[0].outcome, Outcome::Offered);
    assert_eq!(
        report.rows[0].reason,
        "unconfirmed: the confirmation pass gave no answer"
    );
    assert!(
        report.rows[0]
            .matched_because
            .contains(r#""confirmation":"not confirmed""#),
        "{}",
        report.rows[0].matched_because
    );
    assert_eq!(sink.0.lock().unwrap().len(), 1);
}

/// Review of #855, pass 3: the demotion happens at planning, not only at
/// dispatch, so an unconfirmed task is never an automatic one anywhere —
/// not in the claimed row, not against the sink hold, not against a cap.
#[tokio::test]
async fn an_unconfirmed_task_is_demoted_before_it_is_planned() {
    // No sink: an automatic task would be held for it; an offer is not.
    let mut f = Fixture::new();
    let r = described_rule(&f, "bookings", "", TASK);
    f.mode(&r, ActionGroup::Board, Mode::Auto);
    f.options.pass = Pass::AfterClassify;
    told(&f, &r, RuleList::Active, true, RuleInput::Snippet);
    confirmed(&f, Some("none"), &[]);
    let report = f.run(None).await;
    assert_eq!(
        report.rows[0].outcome,
        Outcome::Offered,
        "{:#?}",
        report.rows
    );
    assert!(
        f.store
            .ledger()
            .unwrap()
            .iter()
            .all(|row| row.mode != Mode::Auto),
        "no row of an unconfirmed task is written as automatic: {:#?}",
        f.store.ledger().unwrap()
    );

    // Two threads, one global cap: the unconfirmed first leaves the cap to
    // the confirmed second.
    let mut f = Fixture::new();
    std::fs::write(f.store.root().join("limits.toml"), "daily_cap = 1\n").unwrap();
    let r = described_rule(&f, "bookings", "", TASK);
    f.mode(&r, ActionGroup::Board, Mode::Auto);
    f.options.pass = Pass::AfterClassify;
    told(&f, &r, RuleList::Active, true, RuleInput::Snippet);
    confirmed(&f, Some("none"), &[]);
    {
        let mut cs = f.mail.candidates.lock().unwrap();
        let mut second = cs[0].clone();
        second.message.provider_message_id = "message-beta".into();
        second.message.thread_id = "thread-beta".into();
        let rules = second.classified.as_mut().unwrap().rules.as_mut().unwrap();
        let active = rules
            .confirmation
            .as_mut()
            .unwrap()
            .active
            .as_mut()
            .unwrap();
        active.choice = Some(RuleHit {
            name: "bookings".into(),
            quote: "booking for Friday".into(),
        });
        cs.push(second);
    }
    let sink = Sink::default();
    let report = f.run(Some(&sink)).await;
    let by_thread = |t: &str| {
        report
            .rows
            .iter()
            .find(|row| row.message.thread_id == t)
            .unwrap_or_else(|| panic!("no row for {t}: {:#?}", report.rows))
            .outcome
    };
    assert_eq!(
        by_thread("thread-beta"),
        Outcome::Done,
        "{:#?}",
        report.rows
    );
    assert_ne!(by_thread("thread-alpha"), Outcome::Done);
    assert_eq!(sink.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_shadow_task_records_what_the_confirmation_pass_said() {
    let mut f = Fixture::new();
    let r = described_rule(&f, "bookings", "", TASK);
    f.options.pass = Pass::AfterClassify;
    told(&f, &r, RuleList::Shadow, true, RuleInput::Snippet);
    confirmed(&f, None, &[("bookings", false)]);
    let row = f.run(None).await.rows[0].clone();
    assert_eq!(row.outcome, Outcome::WouldHave);
    assert_eq!(
        row.reason,
        "would have proposed: the confirmation pass disagreed"
    );
    f.mail.next("beta");
    told(&f, &r, RuleList::Shadow, true, RuleInput::Snippet);
    confirmed(&f, None, &[("bookings", true)]);
    assert_eq!(f.run(None).await.rows[0].reason, "");
}

#[test]
fn only_the_groups_the_confirmation_pass_decides_carry_its_version() {
    use crate::mail_triage::confirm::CONFIRMATION_VERSION;
    let f = Fixture::new();
    let r = described_rule(&f, "bookings", "", &format!("{ARCHIVE}{TASK}"));
    let board = r
        .match_digest(ActionGroup::Board, GateStatus::Passing)
        .unwrap();
    let mailbox = r
        .match_digest(ActionGroup::Mailbox, GateStatus::Passing)
        .unwrap();
    assert_eq!(board.confirmation_version, *CONFIRMATION_VERSION);
    assert!(mailbox.confirmation_version.is_empty());
    assert_eq!(
        r.approval_digest.confirmation_version,
        *CONFIRMATION_VERSION
    );
    let header = f.rule("header-task", TASK);
    assert!(header
        .match_digest(ActionGroup::Board, GateStatus::Passing)
        .unwrap()
        .confirmation_version
        .is_empty());
    assert!(header.approval_digest.confirmation_version.is_empty());
    // A description rule the pass decides nothing for carries no version in
    // either digest (review of #855, pass 6).
    let archive_only = described_rule(&f, "archive-only", "", ARCHIVE);
    assert!(archive_only.approval_digest.confirmation_version.is_empty());
    assert!(archive_only
        .match_digest(ActionGroup::Mailbox, GateStatus::Passing)
        .unwrap()
        .confirmation_version
        .is_empty());
    // A send counts, as a task does.
    let replying = reply_rule(&f, "replying", "describe = 'Questions about office hours'");
    assert_eq!(
        replying.approval_digest.confirmation_version,
        *CONFIRMATION_VERSION
    );
}

/// Answers in order, and counts the calls.
struct Answers(
    std::sync::Mutex<Vec<&'static str>>,
    std::sync::atomic::AtomicUsize,
);
#[async_trait]
impl crate::provider::Provider for Answers {
    fn local(&self) -> bool {
        true
    }
    fn id(&self) -> &str {
        "answers"
    }
    fn default_model(&self) -> &str {
        "answers-1"
    }
    async fn complete(
        &self,
        _: &crate::message::CompletionRequest,
        _: Option<&crate::provider::StreamSink>,
    ) -> Result<crate::message::CompletionResponse> {
        use crate::message::{Block, CompletionResponse, Message, StopReason};
        self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let text = self.0.lock().unwrap().remove(0).to_string();
        Ok(CompletionResponse {
            message: Message::assistant(vec![Block::Text { text }]),
            stop_reason: StopReason::EndTurn,
            usage: crate::message::Usage::default(),
            refusal: None,
            model: "answers-1".into(),
            malformed_tool_args: 0,
        })
    }
}

/// Review of #855, pass 4: the producer of the record the engine demotes on
/// is measured, not only the record — an inert one would turn every
/// automatic description task into an offer with the suite green.
#[tokio::test]
async fn the_confirmation_pass_runs_only_when_a_candidate_is_owed_one() {
    use super::super::describe::{ClassifierRules, Envelope};
    use crate::mail_triage::confirm::CONFIRMATION_VERSION;
    use crate::mail_triage::rule_frame::RuleReading;
    use std::sync::atomic::Ordering;
    let f = Fixture::new();
    let a = described_rule(&f, "a-tasks", "", TASK);
    f.mode(&a, ActionGroup::Board, Mode::Auto);
    let archive = described_rule(&f, "c-archive", "", ARCHIVE);
    f.mode(&archive, ActionGroup::Mailbox, Mode::Auto);
    described_rule(&f, "d-shadow-task", "", TASK);
    let rules = ClassifierRules::load(&f.store).unwrap();
    let ask = rules.ask(&Envelope {
        account: "campus",
        from: "someone@example.com",
        subject: "Friday",
        has_attachment: None,
    });
    let thread = crate::mail_triage::ThreadInput {
        thread_id: "t1".into(),
        account: "campus".into(),
        from: "someone@example.com".into(),
        from_name: "Someone".into(),
        subject: "Friday".into(),
        date: "2026-11-07T10:00:00Z".into(),
        body: "Your booking for Friday is confirmed.".into(),
    };
    let hit = |name: &str| RuleHit {
        name: name.into(),
        quote: "booking for Friday".into(),
    };
    let provider = Answers(
        std::sync::Mutex::new(vec![
            r#"{"reasoning":"x","rule":"a-tasks","quote":"booking for Friday"}"#,
            r#"{"reasoning":"x","verdicts":[{"name":"d-shadow-task","matches":true,"quote":"booking for Friday"}]}"#,
        ]),
        Default::default(),
    );

    // A mailbox-only match owes nothing: no call, no record.
    let only_archive = RuleReading {
        active: Some(hit("c-archive")),
        ..Default::default()
    };
    assert!(rules
        .confirm(&provider, "m", &thread, &ask, &only_archive)
        .await
        .is_none());
    assert_eq!(provider.1.load(Ordering::SeqCst), 0);

    // An automatic task candidate: the active shape runs, under this build.
    let task = RuleReading {
        active: Some(hit("a-tasks")),
        ..Default::default()
    };
    let c = rules
        .confirm(&provider, "m", &thread, &ask, &task)
        .await
        .expect("a task candidate is confirmed");
    assert_eq!(c.version, *CONFIRMATION_VERSION);
    assert!(c.confirms_active("a-tasks"), "{c:#?}");
    assert!(c.shadow.is_empty());

    // A shadow task named: the each shape runs, and nothing active.
    let shadow = RuleReading {
        shadow: vec![hit("d-shadow-task")],
        ..Default::default()
    };
    let c = rules
        .confirm(&provider, "m", &thread, &ask, &shadow)
        .await
        .expect("a shadow task is confirmed");
    assert!(c.active.is_none());
    assert_eq!(c.shadow_verdict("d-shadow-task"), Some(true), "{c:#?}");
    assert_eq!(provider.1.load(Ordering::SeqCst), 2);
}

#[test]
fn the_sweep_confirms_a_task_candidate_against_every_eligible_task_rule_and_nothing_else() {
    use super::super::describe::{ClassifierRules, Envelope};
    let f = Fixture::new();
    let a = described_rule(&f, "a-tasks", "", TASK);
    f.mode(&a, ActionGroup::Board, Mode::Auto);
    let b = described_rule(&f, "b-tasks", "", TASK);
    f.mode(&b, ActionGroup::Board, Mode::Propose);
    let archive = described_rule(&f, "c-archive", "", ARCHIVE);
    f.mode(&archive, ActionGroup::Mailbox, Mode::Auto);
    described_rule(&f, "d-shadow-task", "", TASK);
    let rules = ClassifierRules::load(&f.store).unwrap();
    let ask = rules.ask(&Envelope {
        account: "campus",
        from: "someone@example.com",
        subject: "Friday",
        has_attachment: None,
    });
    let hit = |name: &str| RuleHit {
        name: name.into(),
        quote: "Friday notice".into(),
    };
    let reading =
        |active: Option<&str>, shadow: &[&str]| crate::mail_triage::rule_frame::RuleReading {
            active: active.map(hit),
            shadow: shadow.iter().map(|n| hit(n)).collect(),
            ..Default::default()
        };
    let names = |v: Vec<crate::mail_triage::rule_frame::FrameEntry>| {
        v.into_iter().map(|e| e.name).collect::<Vec<_>>()
    };
    // A task candidate: both task rules are offered, the archive rule is not.
    assert_eq!(
        names(rules.active_confirmation(&ask, &reading(Some("a-tasks"), &[]))),
        vec!["a-tasks", "b-tasks"]
    );
    // A mailbox-only match needs no confirmation at all.
    assert!(rules
        .active_confirmation(&ask, &reading(Some("c-archive"), &[]))
        .is_empty());
    // Shadow: only the named rules with a shadow task, one verdict each.
    assert_eq!(
        names(rules.shadow_confirmation(&reading(None, &["d-shadow-task", "c-archive"]))),
        vec!["d-shadow-task"]
    );
}

// ---- template replies, staged (P4b-2a) ----

const REPLY: &str = "[[action]]\ndo = 'reply'\ntemplate = 'reply.md'\n";

fn reply_rule(f: &Fixture, name: &str, matcher: &str) -> LoadedRule {
    f.store
        .install(
            name,
            &format!("description = 'Office hours'\n[match]\n{matcher}\n{REPLY}"),
            &BTreeMap::from([(
                "reply.md".to_string(),
                "Hi {sender_first_name}, thanks for {subject}. Office hours are Fridays."
                    .to_string(),
            )]),
        )
        .unwrap()
}

/// The fixture's message, as one a person wrote: no list headers.
fn personal(f: &Fixture) {
    let mut cs = f.mail.candidates.lock().unwrap();
    cs[0].list_id = None;
    cs[0].headers.clear();
}

#[tokio::test]
async fn a_proposed_template_reply_is_staged_with_its_slots_filled() {
    let mut f = Fixture::new();
    personal(&f);
    let r = reply_rule(&f, "office-hours", "from_domain = ['example.com']");
    f.mode(&r, ActionGroup::FixedWords, Mode::Propose);
    f.options.pass = Pass::AfterClassify;
    let report = f.run(None).await;
    assert_eq!(
        report.rows[0].outcome,
        Outcome::Staged,
        "{:#?}",
        report.rows
    );
    assert_eq!(report.rows[0].outbox_item.as_deref(), Some("item-1"));
    assert!(report.rows[0].closes);
    assert_eq!(
        f.mail.staged.lock().unwrap()[0],
        "Hi Sender, thanks for Re: Sender's untrusted task instructions. Office hours are Fridays."
    );
    // Staged once: a later message in the thread is refused (here by the
    // per-sender week, checked before the thread's closed occasion).
    f.mail.candidates.lock().unwrap()[0]
        .message
        .provider_message_id = "later".into();
    let again = f.run(None).await;
    assert_eq!(again.rows[0].outcome, Outcome::Skipped);
    assert_eq!(f.mail.staged.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn the_guards_refuse_a_reply_in_every_mode_and_say_which() {
    for (setup, why) in [
        ("auto", "auto-submitted"),
        ("list", "a mailing list"),
        ("noreply", "an automated sender"),
        ("own", "one of your own addresses"),
        ("joined", "a conversation you have joined"),
    ] {
        let mut f = Fixture::new();
        personal(&f);
        {
            let mut cs = f.mail.candidates.lock().unwrap();
            match setup {
                "auto" => {
                    cs[0]
                        .headers
                        .insert("Auto-Submitted".into(), vec!["auto-replied".into()]);
                }
                "list" => {
                    cs[0].headers.insert(
                        "List-Unsubscribe".into(),
                        vec!["<mailto:x@example.com>".into()],
                    );
                }
                "noreply" => cs[0].message.sender = "no-reply@example.com".into(),
                "own" => cs[0].sending.owners.push("sender@example.com".into()),
                _ => cs[0].sending.owner_participated = Some(true),
            }
        }
        let r = reply_rule(&f, "office-hours", "from_domain = ['example.com']");
        f.options.pass = Pass::AfterClassify;
        // Shadow too: a would-have rate never counts a refused reply.
        let shadow = f.run(None).await;
        assert_eq!(shadow.rows[0].outcome, Outcome::Skipped, "{setup}");
        assert_eq!(shadow.rows[0].reason, why, "{setup}");
        // Nor does the retrospective preview the owner reads before raising
        // the mode (review of #856, pass 8).
        let candidate = f.mail.candidates.lock().unwrap()[0].clone();
        let tested = test_match(&r, &candidate, Some(true), f.options.clock.now()).unwrap();
        assert_eq!(tested[0].outcome, Outcome::Skipped, "{setup}");
        assert!(
            tested[0]
                .reason
                .starts_with(&format!("test: skipped: {why};")),
            "{setup}: {}",
            tested[0].reason
        );
        f.mode(&r, ActionGroup::FixedWords, Mode::Propose);
        let proposed = f.run(None).await;
        assert_eq!(proposed.rows[0].reason, why, "{setup}");
        assert!(f.mail.staged.lock().unwrap().is_empty(), "{setup}");
    }
}

/// The negative above is not vacuous: a message no guard refuses previews a
/// reply as would-have.
#[test]
fn a_reply_no_guard_refuses_previews_as_would_have() {
    let f = Fixture::new();
    personal(&f);
    let r = reply_rule(&f, "office-hours", "from_domain = ['example.com']");
    let candidate = f.mail.candidates.lock().unwrap()[0].clone();
    let tested = test_match(&r, &candidate, Some(true), f.options.clock.now()).unwrap();
    assert_eq!(tested[0].outcome, Outcome::WouldHave, "{:#?}", tested);
}

#[tokio::test]
async fn one_reply_per_sender_per_rule_per_week() {
    let mut f = Fixture::new();
    personal(&f);
    let r = reply_rule(&f, "office-hours", "from_domain = ['example.com']");
    f.mode(&r, ActionGroup::FixedWords, Mode::Propose);
    f.options.pass = Pass::AfterClassify;
    assert_eq!(f.run(None).await.rows[0].outcome, Outcome::Staged);
    f.mail.next("beta");
    let second = f.run(None).await;
    assert_eq!(second.rows[0].outcome, Outcome::Skipped);
    assert!(second.rows[0].reason.contains("last 7 days"));
    assert_eq!(f.mail.staged.lock().unwrap().len(), 1);
}

/// Review of #856: the shadow readout is the rate the rule would reply at,
/// so the week binds there too — and a pass over the same message never
/// skips it for its own earlier row.
#[tokio::test]
async fn the_per_sender_week_binds_in_shadow_too() {
    let mut f = Fixture::new();
    personal(&f);
    reply_rule(&f, "office-hours", "from_domain = ['example.com']");
    f.options.pass = Pass::AfterClassify;
    assert_eq!(f.run(None).await.rows[0].outcome, Outcome::WouldHave);
    let again = f.run(None).await;
    assert!(
        again.rows.iter().all(|r| r.outcome != Outcome::Skipped),
        "the same message is not its own earlier reply: {:#?}",
        again.rows
    );
    f.mail.next("beta");
    let second = f.run(None).await;
    assert_eq!(
        second.rows[0].outcome,
        Outcome::Skipped,
        "{:#?}",
        second.rows
    );
    assert!(second.rows[0].reason.contains("last 7 days"));
}

/// Review of #856: `{subject}` is the subject as read, never Gmail's raw
/// encoded-words, and a subject that cannot be read whole refuses the reply
/// rather than render a hole.
#[tokio::test]
async fn the_subject_slot_is_decoded_or_the_reply_is_refused() {
    let mut f = Fixture::new();
    personal(&f);
    {
        let mut cs = f.mail.candidates.lock().unwrap();
        cs[0].subject_is_raw = true;
        cs[0].message.subject = "=?UTF-8?B?UsOpc2VydmF0aW9uIGNvbmZpcm3DqWU=?=".into();
    }
    let r = reply_rule(&f, "office-hours", "from_domain = ['example.com']");
    f.mode(&r, ActionGroup::FixedWords, Mode::Propose);
    f.options.pass = Pass::AfterClassify;
    let report = f.run(None).await;
    assert_eq!(
        report.rows[0].outcome,
        Outcome::Staged,
        "{:#?}",
        report.rows
    );
    let body = f.mail.staged.lock().unwrap()[0].clone();
    assert!(
        body.contains("thanks for Re: Réservation confirmée."),
        "{body}"
    );
    // The review is told who the reply goes to and what it answers.
    let answering = f.mail.answering.lock().unwrap()[0].clone();
    assert_eq!(answering.subject, "Re: Réservation confirmée");
    assert!(answering.to.contains('@'), "{answering:?}");

    for unreadable in ["truncated", "undecodable", "empty", "blank", "nameless"] {
        let mut f = Fixture::new();
        personal(&f);
        {
            let mut cs = f.mail.candidates.lock().unwrap();
            match unreadable {
                "truncated" => cs[0].subject_complete = Some(false),
                "undecodable" => {
                    cs[0].subject_is_raw = true;
                    cs[0].message.subject = "=?X-UNKNOWN?Q?broken?=".into();
                }
                // Reads fine, but would render a bare "Re: " (review of #856,
                // pass 5): no Subject header at all, and one of only spaces.
                "empty" => {
                    cs[0].message.subject = String::new();
                    cs[0]
                        .headers
                        .retain(|name, _| !name.eq_ignore_ascii_case("subject"));
                }
                "blank" => cs[0].message.subject = "   ".into(),
                // A bare address in From: `{sender_first_name}` has no value.
                _ => cs[0].sending.from_name = String::new(),
            }
        }
        let r = reply_rule(&f, "office-hours", "from_domain = ['example.com']");
        f.mode(&r, ActionGroup::FixedWords, Mode::Propose);
        f.options.pass = Pass::AfterClassify;
        let report = f.run(None).await;
        assert_eq!(
            report.rows[0].outcome,
            Outcome::Skipped,
            "{unreadable}: {:#?}",
            report.rows
        );
        assert_eq!(
            report.rows[0].reason,
            if unreadable == "nameless" {
                "the sender gave no name to address"
            } else {
                "the subject could not be read whole"
            }
        );
        assert!(f.mail.staged.lock().unwrap().is_empty());
    }
}

/// Review of #856: a staged reply is the rule firing, once — its settlement
/// is what the owner did with that fire, never a second one.
#[tokio::test]
async fn a_released_reply_is_one_fire_on_the_rules_page() {
    let mut f = Fixture::new();
    personal(&f);
    let r = reply_rule(&f, "office-hours", "from_domain = ['example.com']");
    f.mode(&r, ActionGroup::FixedWords, Mode::Propose);
    f.options.pass = Pass::AfterClassify;
    f.run(None).await;
    f.mail
        .fates
        .lock()
        .unwrap()
        .insert("item-1".into(), StagedFate::ReleasedUnchanged);
    f.run(None).await;
    let ledger = f.store.ledger().unwrap();
    assert!(ledger
        .iter()
        .any(|row| row.outcome == Outcome::ReleasedUnchanged));
    let (fires, last, _) = super::super::presentation::activity(
        "office-hours",
        &ledger,
        Utc::now() + Duration::days(1),
    );
    assert_eq!(fires, 1, "{ledger:#?}");
    assert!(last.is_some());
}

#[tokio::test]
async fn what_the_owner_did_with_a_staged_reply_is_written_back_once() {
    let mut f = Fixture::new();
    personal(&f);
    let r = reply_rule(&f, "office-hours", "from_domain = ['example.com']");
    f.mode(&r, ActionGroup::FixedWords, Mode::Propose);
    f.options.pass = Pass::AfterClassify;
    let staged = f.run(None).await.rows[0].clone();
    f.mail
        .fates
        .lock()
        .unwrap()
        .insert("item-1".into(), StagedFate::Rejected);
    for _ in 0..3 {
        f.run(None).await;
    }
    let rejected: Vec<_> = f
        .store
        .ledger()
        .unwrap()
        .into_iter()
        .filter(|row| row.outcome == Outcome::Rejected)
        .collect();
    assert_eq!(rejected.len(), 1, "{rejected:#?}");
    assert_eq!(
        rejected[0].correction_of.as_deref(),
        Some(staged.id.as_str())
    );
    assert!(rejected[0].closes);
    assert_eq!(rejected[0].outbox_item.as_deref(), Some("item-1"));
}

/// Review of #856: one outbound slot per message, so a rule with two
/// replies would stage two drafts for one message; it does not load.
#[test]
fn a_rule_with_two_replies_does_not_install() {
    let f = Fixture::new();
    let error = f
        .store
        .install(
            "twice",
            &format!(
                "description = 'Twice'\n[match]\nfrom_domain = ['example.com']\n{REPLY}[[action]]\ndo = 'reply'\ntemplate = 'other.md'\n"
            ),
            &BTreeMap::from([
                ("reply.md".to_string(), "Hello.".to_string()),
                ("other.md".to_string(), "Hello again.".to_string()),
            ]),
        )
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("replies at most once"),
        "{error:#}"
    );
}

/// Review of #856: a template refused on disk holds that rule's reply and
/// stops nothing else — the pass runs every other rule, and the bad rule's
/// would-have rate never counts a reply it could not render.
#[tokio::test]
async fn a_bad_template_on_disk_holds_its_reply_and_stops_nothing_else() {
    let mut f = Fixture::new();
    personal(&f);
    let good = f.rule("archiver", ARCHIVE);
    f.mode(&good, ActionGroup::Mailbox, Mode::Auto);
    let bad = reply_rule(&f, "office-hours", "from_domain = ['example.com']");
    // Written past install, as a rule from before validation would be.
    std::fs::write(bad.directory.join("reply.md"), "Dear {name}").unwrap();
    let loaded = f.store.get("office-hours").unwrap();
    assert!(loaded.template_refused.is_some());
    f.options.pass = Pass::AfterClassify;
    let report = f.run(None).await;
    assert!(
        report
            .rows
            .iter()
            .any(|r| r.rule == "archiver" && r.outcome == Outcome::Done),
        "{:#?}",
        report.rows
    );
    let reply = report
        .rows
        .iter()
        .find(|r| r.rule == "office-hours")
        .expect("the bad rule is still considered");
    assert_eq!(reply.outcome, Outcome::Skipped);
    assert!(
        reply.reason.starts_with("template refused"),
        "{}",
        reply.reason
    );
    // And it cannot be raised past shadow while the template stands.
    let decision = f
        .store
        .change_mode(
            "office-hours",
            ActionGroup::FixedWords,
            Mode::Propose,
            Some(Actor::Owner),
            &loaded.approval_digest,
            &P1Holds {
                origin_gate: GateStatus::Passing,
            },
        )
        .unwrap();
    assert!(decision.refused);
}

#[test]
fn a_template_naming_a_slot_outside_the_closed_set_does_not_install() {
    let f = Fixture::new();
    let error = f
        .store
        .install(
            "bad",
            &format!("description = 'Bad'\n[match]\nfrom_domain = ['example.com']\n{REPLY}"),
            &BTreeMap::from([("reply.md".to_string(), "Dear {name}".to_string())]),
        )
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("not a template slot"),
        "{error:#}"
    );
}

#[tokio::test]
async fn a_description_rules_reply_takes_the_outbound_slot_through_its_active_name() {
    let mut f = Fixture::new();
    personal(&f);
    let r = f
        .store
        .install(
            "office-hours",
            &format!(
                "description = 'Office hours'\n[match]\ndescribe = 'Questions about office hours'\n{REPLY}"
            ),
            &BTreeMap::from([(
                "reply.md".to_string(),
                "Office hours are Fridays.".to_string(),
            )]),
        )
        .unwrap();
    f.mode(&r, ActionGroup::FixedWords, Mode::Propose);
    f.options.pass = Pass::AfterClassify;
    told(&f, &r, RuleList::Active, true, RuleInput::FullBody);
    let report = f.run(None).await;
    assert_eq!(
        report.rows[0].outcome,
        Outcome::Staged,
        "{:#?}",
        report.rows
    );
    assert_eq!(report.rows[0].verdict_input, VerdictInput::FullBody);
}
