use super::*;
use crate::microsoft::graph_mail::MutationOutcome;
use serde_json::json;

fn email(id: &str, sender: &str, date: &str) -> Email {
    serde_json::from_value(json!({"id":id,"provider":"fixture","provider_id":id,"thread_id":"thread-alpha","message_id":null,"subject":"Untrusted subject","from_address":sender,"sender_known":true,"from_name":"Untrusted display name","to_addresses":["owner@example.edu"],"cc_addresses":[],"bcc_addresses":[],"date_received":date,"date_known":true,"provider_arrival_at":date,"attachment_presence":false,"body_text":"","body_html":"","snippet":"","labels":[],"is_read":false,"is_starred":false,"has_attachments":false,"list_unsubscribe":null,"is_draft":false,"direction_evidence":"not_draft"})).unwrap()
}

#[test]
fn candidate_uses_newest_inbound_and_refuses_missing_direction_or_dates() {
    let inbound = email("inbound", "sender@example.com", "2026-11-03T12:00:00Z");
    let sent = email("outbound", "owner@example.edu", "2026-11-03T13:00:00Z");
    let emails = vec![inbound.clone(), sent];
    let c = candidate(
        "campus",
        "thread-alpha",
        &emails,
        &["owner@example.edu".into()],
        None,
    )
    .unwrap();
    assert_eq!(c.message.provider_message_id, "inbound");
    assert!(c.classified.is_none());
    assert!(candidate("campus", "thread-alpha", &emails, &[], None).is_err());
    let mut unknown = inbound;
    unknown.is_draft = None;
    unknown.direction_evidence = None;
    assert!(candidate(
        "campus",
        "thread-alpha",
        &[unknown],
        &["owner@example.edu".into()],
        None
    )
    .is_err());
}

#[test]
fn partial_mutations_are_failures_and_outages_do_not_indict_a_rule() {
    assert!(matches!(
        triage_result(TriageResult::Count(MutationOutcome {
            moved: 1,
            failed: 1,
            ..Default::default()
        })),
        ActionResult::Failed {
            kind: FailureKind::Unknown,
            ..
        }
    ));
    assert!(matches!(
        triage_result(TriageResult::Count(MutationOutcome {
            moved: 0,
            failed: 0,
            ..Default::default()
        })),
        ActionResult::Done
    ));
    assert!(matches!(
        triage_result(TriageResult::Uncounted),
        ActionResult::Done
    ));
    for error in [
        MailError::AuthError("reauth".into()),
        MailError::ApiError {
            status: 503,
            message: "offline".into(),
        },
        MailError::ApiError {
            status: 429,
            message: "wait".into(),
        },
    ] {
        assert!(matches!(
            failure(error),
            ActionResult::Failed {
                kind: FailureKind::Environment,
                ..
            }
        ));
    }
    assert!(matches!(
        failure(MailError::ApiError {
            status: 400,
            message: "refused action".into()
        }),
        ActionResult::Failed {
            kind: FailureKind::Action,
            ..
        }
    ));
}

#[test]
fn header_instances_are_preserved_by_both_provider_parsers() {
    let headers = json!([{"name":"X-Notice","value":"first"},{"name":"x-notice","value":"second"}]);
    let gmail = crate::google::gmail::parse_gmail_message(
        crate::google::gmail::HeaderRead::Full,
        &json!({"payload":{"headers":headers}}),
    );
    let outlook = crate::microsoft::graph_mail::parse_outlook_message(
        &json!({"internetMessageHeaders":headers}),
    );
    assert_eq!(gmail.headers["x-notice"], vec!["first", "second"]);
    assert_eq!(outlook.headers, gmail.headers);
}

#[test]
fn private_annotations_survive_classification_and_never_invent_a_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    rule_records::update(root, "campus", "thread-alpha", |r| {
        r.tags.insert("notice".into());
        r.digest_rules.insert("archive-notice".into());
        r.handled_message = Some("message-alpha".into());
    })
    .unwrap();
    let store = mail_triage::TriageStore::open(root).unwrap();
    assert!(store.get("campus", "thread-alpha").is_none());
    let r: mail_triage::Record = serde_json::from_value(json!({"account":"campus","thread_id":"thread-alpha","state":"classified","classified_at":"2026-11-03T12:00:00Z"})).unwrap();
    store.put(&r).unwrap();
    assert_eq!(
        store.get("campus", "thread-alpha").unwrap().rest["rule_tags"],
        json!(["notice"])
    );
    store.put(&r).unwrap();
    assert_eq!(
        store.list().unwrap()[0].rest["digest_rules"],
        json!(["archive-notice"])
    );
}

#[tokio::test]
async fn absent_graph_is_refused_before_any_server_or_workspace_is_created() {
    assert!(GraphSink::connect(&mecha_core::config::Config::default())
        .await
        .is_err());
}

#[tokio::test]
async fn graph_sink_uses_mcp_with_typed_name_and_pointer_and_checks_errors() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("graph.py");
    std::fs::write(&script, r#"
import json, sys
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    method = request['method']
    if method == 'initialize':
        result = {'protocolVersion':'2024-11-05','capabilities':{},'serverInfo':{'name':'fixture','version':'test'}}
    elif method == 'tools/list':
        result = {'tools':[{'name':'kg_task_create','description':'Create a fictional task','inputSchema':{'type':'object'}}]}
    elif method == 'tools/call':
        args = request['params']['arguments']
        with open('captured.json', 'w') as stream:
            json.dump(args, stream)
        result = {'content':[] if args['name'] == 'empty' else [{'type':'text','text':'fixture response'}], 'isError':args['name'].startswith('refused')}
    else:
        result = {}
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}), flush=True)
"#).unwrap();
    for prefix in [false, true] {
        let entry = mecha_core::config::McpServerConfig {
            name: "graph".into(),
            command: "python3".into(),
            args: vec![script.display().to_string()],
            prefix_tools: Some(prefix),
            ..Default::default()
        };
        let graph = GraphSink::connect_at(
            &mecha_core::config::Config::default(),
            &entry,
            dir.path().into(),
        )
        .await
        .unwrap();
        let mut task = Task {
            name: "Review notice — sender@example.com".into(),
            account: "campus".into(),
            thread_id: "thread-alpha".into(),
            received_at: Utc::now(),
            due: NaiveDate::from_ymd_opt(2026, 12, 1),
        };
        assert!(matches!(graph.create(&task).await, ActionResult::Done));
        let captured: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("captured.json")).unwrap())
                .unwrap();
        assert_eq!(captured["name"], task.name);
        assert_eq!(captured["captured_from"]["id"], "thread-alpha");
        assert_eq!(captured["due"], "2026-12-01");
        assert!(captured["captured_from"].get("label").is_none());
        task.name = "refused".into();
        assert!(matches!(
            graph.create(&task).await,
            ActionResult::Failed { .. }
        ));
        task.name = "empty".into();
        assert!(matches!(
            graph.create(&task).await,
            ActionResult::Failed { .. }
        ));
        check_graph_brake(&graph, "refused", false).await;
        graph.client.close().await.unwrap();
    }
}

#[test]
fn gmail_sender_date_is_display_only_for_rule_eligibility() {
    for internal in [serde_json::Value::Null, json!("invalid")] {
        let email = crate::google::gmail::parse_gmail_message(
            crate::google::gmail::HeaderRead::Full,
            &json!({"id":"message-alpha", "threadId":"thread-alpha", "internalDate":internal, "labelIds":["INBOX"], "payload":{"headers":[{"name":"From","value":"sender@example.com"},{"name":"Date","value":"Tue, 03 Nov 2026 12:00:00 +0000"}]}}),
        );
        let candidate = candidate(
            "campus",
            "thread-alpha",
            &[email],
            &["owner@example.edu".into()],
            None,
        )
        .unwrap();
        assert!(candidate.arrival_at.is_none());
    }
}

#[test]
fn missing_outlook_attachment_metadata_stays_unknown() {
    let email = crate::microsoft::graph_mail::parse_outlook_message(
        &json!({"id":"message-alpha", "conversationId":"thread-alpha", "from":{"emailAddress":{"address":"sender@example.com"}}, "receivedDateTime":"2026-11-03T12:00:00Z", "isDraft":false}),
    );
    let c = candidate(
        "campus",
        "thread-alpha",
        &[email],
        &["owner@example.edu".into()],
        None,
    )
    .unwrap();
    assert_eq!(c.attachment_presence, None);
    assert_eq!(c.headers_complete, None);
}

#[test]
fn provider_arrival_precision_survives_the_classification_envelope() {
    let arrival: DateTime<Utc> = "2026-11-03T12:00:00.123Z".parse().unwrap();
    let gmail = crate::google::gmail::parse_gmail_message(
        crate::google::gmail::HeaderRead::Full,
        &json!({"id":"message-alpha", "threadId":"thread-alpha", "internalDate":arrival.timestamp_millis().to_string(), "labelIds":["INBOX"], "payload":{"headers":[{"name":"From","value":"sender@example.com"}]}}),
    );
    let outlook = crate::microsoft::graph_mail::parse_outlook_message(
        &json!({"id":"message-alpha", "conversationId":"thread-alpha", "receivedDateTime":arrival.to_rfc3339(), "isDraft":false, "from":{"emailAddress":{"address":"sender@example.com"}}}),
    );
    for email in [gmail, outlook] {
        assert_eq!(email.provider_arrival_at, Some(arrival));
        assert_eq!(
            email.date_received.parse::<DateTime<Utc>>().unwrap(),
            arrival
        );
        let c = candidate(
            "campus",
            "thread-alpha",
            &[email],
            &["owner@example.edu".into()],
            None,
        )
        .unwrap();
        assert_eq!(c.arrival_at, Some(arrival));
    }
}

#[test]
fn graph_dedicated_envelope_fields_prove_negative_matches() {
    let email = crate::microsoft::graph_mail::parse_outlook_message(
        &json!({"id":"message-alpha", "isDraft":false, "subject":"Meeting agenda", "from":{"emailAddress":{"address":"sender@example.com"}}, "toRecipients":[{"emailAddress":{"address":"owner@example.edu"}}], "ccRecipients":[], "bccRecipients":[], "receivedDateTime":"2026-11-03T12:00:00Z"}),
    );
    let c = candidate(
        "campus",
        "thread-alpha",
        &[email],
        &["owner@example.edu".into()],
        None,
    )
    .unwrap();
    assert_eq!(c.headers_complete, None);
    for matcher in [
        mecha_core::mail_rules::Matcher {
            subject_contains: vec!["invoice".into()],
            ..Default::default()
        },
        mecha_core::mail_rules::Matcher {
            to: vec!["other@example.edu".into()],
            ..Default::default()
        },
    ] {
        assert_eq!(
            engine::header_match(&matcher, &c),
            engine::HeaderMatch::NoMatch
        );
    }
}

#[test]
fn graph_missing_envelope_fields_and_raw_headers_remain_unknown() {
    use mecha_core::mail_rules::Matcher;
    let base = json!({"id":"message-alpha", "isDraft":false, "subject":"Meeting agenda", "from":{"emailAddress":{"address":"sender@example.com"}}, "toRecipients":[], "ccRecipients":[], "bccRecipients":[], "receivedDateTime":"2026-11-03T12:00:00Z"});
    for field in ["subject", "toRecipients", "ccRecipients", "bccRecipients"] {
        let mut raw = base.clone();
        raw.as_object_mut().unwrap().remove(field);
        let email = crate::microsoft::graph_mail::parse_outlook_message(&raw);
        let c = candidate(
            "campus",
            "thread-alpha",
            &[email],
            &["owner@example.edu".into()],
            None,
        )
        .unwrap();
        let matcher = if field == "subject" {
            Matcher {
                subject_contains: vec!["invoice".into()],
                ..Default::default()
            }
        } else {
            Matcher {
                to: vec!["other@example.edu".into()],
                ..Default::default()
            }
        };
        assert_eq!(
            engine::header_match(&matcher, &c),
            engine::HeaderMatch::Unknown
        );
    }
    let email = crate::microsoft::graph_mail::parse_outlook_message(&base);
    let c = candidate(
        "campus",
        "thread-alpha",
        &[email],
        &["owner@example.edu".into()],
        None,
    )
    .unwrap();
    for matcher in [
        Matcher {
            list_id: vec!["notice.example.com".into()],
            ..Default::default()
        },
        Matcher {
            header: std::collections::BTreeMap::from([("x-notice".into(), vec!["invoice".into()])]),
            ..Default::default()
        },
    ] {
        assert_eq!(
            engine::header_match(&matcher, &c),
            engine::HeaderMatch::Unknown
        );
    }
}

#[test]
fn quoted_display_mailbox_never_authorizes_a_sender_rule() {
    let email = crate::google::gmail::parse_gmail_message(
        crate::google::gmail::HeaderRead::Full,
        &json!({"id":"inbound", "labelIds":["INBOX"], "payload":{"headers":[
            {"name":"From","value":"\"Accounts <billing@vendor.example>\" <person@other.example>"}
        ]}}),
    );
    assert_eq!(email.from_address, "person@other.example");
    let c = candidate(
        "campus",
        "thread-alpha",
        &[email],
        &["owner@example.edu".into()],
        None,
    )
    .unwrap();
    let rule = mecha_core::mail_rules::RuleDefinition::parse("description = 'Review notice'\n[match]\nfrom = ['billing@vendor.example']\n[[action]]\ndo = 'trash'").unwrap();
    assert_eq!(
        engine::header_match(&rule.matcher, &c),
        engine::HeaderMatch::NoMatch
    );
}

#[test]
fn ambiguous_or_malformed_from_is_unknown_to_rules() {
    for from in [
        "first@example.com, second@example.com",
        "Broken <person@example.com",
        "person@example.com (unterminated",
        "Group: person@example.com;",
        "not a mailbox",
        "person@example.com;",
        "person@example.com,",
        "person@example.com second@example.com",
    ] {
        let email = crate::google::gmail::parse_gmail_message(
            crate::google::gmail::HeaderRead::Full,
            &json!({"id":"inbound", "labelIds":["INBOX"], "payload":{"headers":[{"name":"From","value":from}]}}),
        );
        assert!(
            email.sender_known == Some(false),
            "guessed sender for {from}: {}",
            email.from_address
        );
        assert!(candidate(
            "campus",
            "thread-alpha",
            &[email],
            &["owner@example.edu".into()],
            None
        )
        .is_err());
    }
}

#[tokio::test]
async fn sent_alias_reply_cannot_replace_pre_activation_inbound() {
    use mecha_core::mail_rules::{ActionGroup, Mode, Outcome, P1Holds};
    let dir = tempfile::tempdir().unwrap();
    let store = RuleStore::open(dir.path().join("rules")).unwrap();
    let rule = store.install("subject", "description = 'Review notice'\n[match]\nsubject_contains = ['untrusted']\n[[action]]\ndo = 'trash'", &Default::default()).unwrap();
    let holds = P1Holds {
        origin_gate: GateStatus::Passing,
    };
    store
        .change_mode(
            &rule.name,
            ActionGroup::Mailbox,
            Mode::Auto,
            Some(mecha_core::closure::Actor::Owner),
            &rule.approval_digest,
            &holds,
        )
        .unwrap();
    let activation = mecha_core::mail_rules::resolve_mode(
        &rule,
        ActionGroup::Mailbox,
        &store.approvals().unwrap(),
        &holds,
    )
    .activated_at
    .unwrap();
    let old = email(
        "inbound",
        "sender@example.com",
        &(activation - chrono::Duration::hours(1)).to_rfc3339(),
    );
    let sent_at = activation + chrono::Duration::hours(1);
    let reply = crate::google::gmail::parse_gmail_message(
        crate::google::gmail::HeaderRead::Full,
        &json!({"id":"reply", "labelIds":["SENT"], "internalDate":sent_at.timestamp_millis().to_string(), "payload":{"headers":[{"name":"From","value":"alias@example.edu"}, {"name":"Subject","value":"Untrusted subject"}]}}),
    );
    let c = candidate(
        "campus",
        "thread-alpha",
        &[old, reply],
        &["owner@example.edu".into()],
        None,
    )
    .unwrap();
    assert_eq!(c.message.provider_message_id, "inbound");
    assert!(c.arrival_at.unwrap() < activation);
    let report = engine::apply(
        &store,
        &FixtureMailbox(vec![c]),
        None,
        &["campus".into()],
        &engine::EngineOptions {
            clock: Arc::new(mecha_core::clock::FixedClock(sent_at)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(report.rows.len(), 1);
    assert_eq!(report.rows[0].outcome, Outcome::Skipped);
    assert_eq!(report.rows[0].reason, "before the rule was active");
}

#[tokio::test]
async fn graph_exit_after_discovery_holds_task_fate() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("exiting_graph.py");
    std::fs::write(&script, r#"
import json, sys
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    method = request['method']
    if method == 'initialize':
        result = {'protocolVersion':'2024-11-05','capabilities':{},'serverInfo':{'name':'fixture','version':'test'}}
    elif method == 'tools/list':
        result = {'tools':[{'name':'kg_task_create','inputSchema':{'type':'object'}}]}
    else:
        sys.exit()
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}), flush=True)
    if method == 'tools/list':
        sys.exit()
"#).unwrap();
    let entry = mecha_core::config::McpServerConfig {
        name: "graph".into(),
        command: "python3".into(),
        args: vec![script.display().to_string()],
        ..Default::default()
    };
    let graph = GraphSink::connect_at(
        &mecha_core::config::Config::default(),
        &entry,
        dir.path().into(),
    )
    .await
    .unwrap();
    for thread in ["alpha", "beta", "gamma"] {
        let result = graph
            .create(&Task {
                name: "Review notice".into(),
                account: "campus".into(),
                thread_id: thread.into(),
                received_at: Utc::now(),
                due: None,
            })
            .await;
        assert!(
            matches!(
                result,
                ActionResult::Held {
                    kind: FailureKind::Environment,
                    reason: mecha_core::mail_rules::HeldReason::FateUnknown
                }
            ),
            "{result:?}"
        );
    }
    check_graph_brake(&graph, "Review notice", true).await;
    graph_fate_unknown_reserves_both_caps(&graph).await;
    graph.client.close().await.unwrap();
}

#[test]
fn mailbox_parser_handles_comments_lists_and_duplicate_from_fields() {
    use crate::google::gmail::{parse_gmail_message, HeaderRead};
    let parsed = parse_gmail_message(
        HeaderRead::Full,
        &json!({"payload":{"headers":[
            {"name":"From","value":"(outer (nested)) \"Accounts, Europe\" <person@example.com> (office)"},
            {"name":"To","value":"\"Surname, Given\" <first@example.edu>, Team: second@example.edu, third@example.edu;"},
            {"name":"Cc","value":"fourth@example.edu (office)"}
        ]}}),
    );
    assert_eq!(parsed.from_address, "person@example.com");
    assert_eq!(parsed.from_name, "Accounts, Europe");
    assert_eq!(
        parsed.to_addresses,
        [
            "first@example.edu",
            "second@example.edu",
            "third@example.edu"
        ]
    );
    assert_eq!(parsed.cc_addresses, ["fourth@example.edu"]);
    assert_eq!(parsed.recipients_complete, Some(true));
    let duplicate = parse_gmail_message(
        HeaderRead::Full,
        &json!({"payload":{"headers":[
            {"name":"From","value":"first@example.com"}, {"name":"From","value":"second@example.com"}
        ]}}),
    );
    assert_eq!(duplicate.sender_known, Some(false));
    let broken = parse_gmail_message(
        HeaderRead::Full,
        &json!({"payload":{"headers":[
            {"name":"To","value":"Broken <person@example.edu"}
        ]}}),
    );
    assert_ne!(broken.recipients_complete, Some(true));
}

#[test]
fn provider_direction_and_all_owner_addresses_bound_candidate_selection() {
    use crate::google::gmail::{parse_gmail_message, HeaderRead};
    use crate::microsoft::graph_mail::parse_outlook_message;
    let dir = tempfile::tempdir().unwrap();
    let store = RuleStore::open(dir.path().join("rules")).unwrap();
    std::fs::write(
        store.root().join("aliases.toml"),
        "aliases = ['Alias@Example.edu']",
    )
    .unwrap();
    let accounts = [
        Some("owner@example.edu".into()),
        Some("other@example.com".into()),
    ];
    let owners = owner_addresses(&accounts, Some("owner@example.edu"), &store).unwrap();
    assert!(owners.contains(&"alias@example.edu".into()));
    assert!(owners.contains(&"other@example.com".into()));
    assert!(owner_addresses(&accounts, None, &store).is_err());
    let inbound = email("inbound", "sender@example.com", "2026-11-03T10:00:00Z");
    for sender in ["alias@example.edu", "other@example.com"] {
        let gmail = parse_gmail_message(
            HeaderRead::Full,
            &json!({"id":"reply", "labelIds":["INBOX"], "payload":{"headers":[{"name":"From","value":sender}]}}),
        );
        let outlook = parse_outlook_message(
            &json!({"id":"reply", "isDraft":false, "from":{"emailAddress":{"address":sender}}}),
        );
        for reply in [gmail, outlook] {
            let c = candidate(
                "campus",
                "thread-alpha",
                &[inbound.clone(), reply],
                &owners,
                None,
            )
            .unwrap();
            assert_eq!(c.message.provider_message_id, "inbound");
        }
    }
    for folder in ["sentitems", "drafts"] {
        let reply = parse_outlook_message(
            &json!({"id":"reply", "parentFolderId":folder, "from":{"emailAddress":{"address":"unlisted-alias@example.edu"}}}),
        );
        let c = candidate(
            "campus",
            "thread-alpha",
            &[inbound.clone(), reply],
            &owners,
            None,
        )
        .unwrap();
        assert_eq!(c.message.provider_message_id, "inbound");
    }
    let unknown_gmail = parse_gmail_message(
        HeaderRead::Full,
        &json!({"id":"unknown", "payload":{"headers":[{"name":"From","value":"sender@example.com"}]}}),
    );
    let unknown_outlook = parse_outlook_message(
        &json!({"id":"unknown", "parentFolderId":"opaque-folder", "from":{"emailAddress":{"address":"sender@example.com"}}}),
    );
    for unknown in [unknown_gmail, unknown_outlook] {
        assert!(candidate(
            "campus",
            "thread-alpha",
            &[inbound.clone(), unknown],
            &owners,
            None
        )
        .is_err());
    }
    std::fs::write(store.root().join("aliases.toml"), "aliases = false").unwrap();
    assert!(owner_addresses(&accounts, Some("owner@example.edu"), &store).is_err());
}

struct FixtureMailbox(Vec<Candidate>);
#[async_trait]
impl Mailbox for FixtureMailbox {
    async fn candidates(&self, _: &str, _: Pass) -> Result<Vec<String>> {
        Ok(self.0.iter().map(|c| c.message.thread_id.clone()).collect())
    }
    async fn read_thread(&self, _: &str, thread: &str) -> Result<ThreadRead> {
        Ok(self
            .0
            .iter()
            .find(|c| c.message.thread_id == thread)
            .unwrap()
            .clone()
            .into())
    }
    async fn mailbox_action(&self, _: &Candidate, _: ActionVerb) -> ActionResult {
        panic!("fixture must never mutate mail")
    }
    async fn private_record(&self, _: &Candidate, _: &str, _: &Action) -> ActionResult {
        panic!("fixture must never write annotations")
    }
    async fn handled(&self, _: &Candidate) -> Result<()> {
        panic!("fixture must never skip classification")
    }
    fn origin_gate(&self, _: &LoadedRule) -> GateStatus {
        GateStatus::Passing
    }
}

async fn check_graph_brake(sink: &GraphSink, description: &str, outage: bool) {
    use mecha_core::mail_rules::{ActionGroup, Mode, Outcome, P1Holds};
    let dir = tempfile::tempdir().unwrap();
    let store = RuleStore::open(dir.path().join("rules")).unwrap();
    let rule = store.install("task", &format!("description = '{description}'\n[match]\nfrom_domain = ['example.com']\n[[action]]\ndo = 'task'"), &Default::default()).unwrap();
    let holds = P1Holds {
        origin_gate: GateStatus::Passing,
    };
    store
        .change_mode(
            &rule.name,
            ActionGroup::Board,
            Mode::Auto,
            Some(mecha_core::closure::Actor::Owner),
            &rule.approval_digest,
            &holds,
        )
        .unwrap();
    let now = Utc::now() + chrono::Duration::hours(1);
    let mail = FixtureMailbox(
        ["alpha", "beta", "gamma"]
            .into_iter()
            .map(|thread| {
                let mut c = candidate(
                    "campus",
                    thread,
                    &[email(thread, "sender@example.com", &now.to_rfc3339())],
                    &["owner@example.edu".into()],
                    None,
                )
                .unwrap();
                c.classified = Some(Classified {
                    at: now,
                    deadline: None,
                });
                c
            })
            .collect(),
    );
    let report = engine::apply(
        &store,
        &mail,
        Some(sink),
        &["campus".into()],
        &engine::EngineOptions {
            pass: Pass::AfterClassify,
            clock: Arc::new(mecha_core::clock::FixedClock(now)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(report.rows.len(), 3);
    assert!(report.rows.iter().all(|row| row.outcome
        == if outage {
            Outcome::Held
        } else {
            Outcome::Failed
        }));
    assert!(report
        .rows
        .iter()
        .all(|row| row.is_action_failure() != outage));
    if outage {
        assert!(report.rows.iter().all(|row| row.reason
            == mecha_core::mail_rules::FATE_UNKNOWN_REASON
            && row.closes
            && row.failure_kind == FailureKind::Environment));
    }
    let mode = mecha_core::mail_rules::resolve_mode(
        &rule,
        ActionGroup::Board,
        &store.approvals().unwrap(),
        &holds,
    );
    assert_eq!(
        mode.effective,
        if outage { Mode::Auto } else { Mode::Shadow }
    );
}

#[test]
fn raw_encoded_and_folded_headers_match_semantic_text() {
    use crate::google::gmail::{parse_gmail_message, HeaderRead};
    use mecha_core::mail_rules::Matcher;
    for raw in [
        "=?UTF-8?B?aW52b2ljZQ==?=",
        "=?us-ascii?Q?invoice?=",
        "=?ISO-8859-1?Q?invoice?=",
    ] {
        let email = parse_gmail_message(
            HeaderRead::Full,
            &json!({"id":"notice", "labelIds":["INBOX"], "payload":{"headers":[{"name":"From","value":"sender@example.com"},{"name":"Subject","value":raw},{"name":"X-Notice","value":raw}]}}),
        );
        let c = candidate(
            "campus",
            "thread",
            &[email],
            &["owner@example.edu".into()],
            None,
        )
        .unwrap();
        assert_eq!(c.message.subject, raw, "stored display text stays raw");
        assert_eq!(
            engine::header_match(
                &Matcher {
                    subject_contains: vec!["invoice".into()],
                    ..Default::default()
                },
                &c
            ),
            engine::HeaderMatch::Match
        );
        assert_eq!(
            engine::header_match(
                &Matcher {
                    subject_contains: vec!["utf".into()],
                    ..Default::default()
                },
                &c
            ),
            engine::HeaderMatch::NoMatch
        );
        assert_eq!(
            engine::header_match(
                &Matcher {
                    header: [("X-Notice".into(), vec!["invoice".into()])].into(),
                    ..Default::default()
                },
                &c
            ),
            engine::HeaderMatch::Match
        );
    }
    for provider in ["gmail", "outlook"] {
        let headers = json!([{"name":"From","value":"sender@example.com"},{"name":"Subject","value":"invoice\r\n\t ready"},{"name":"X-Notice","value":"invoice\r\n\t ready"}]);
        let email = if provider == "gmail" {
            parse_gmail_message(
                HeaderRead::Full,
                &json!({"id":"notice", "labelIds":["INBOX"], "payload":{"headers":headers}}),
            )
        } else {
            crate::microsoft::graph_mail::parse_outlook_message(
                &json!({"id":"notice", "isDraft":false,"from":{"emailAddress":{"address":"sender@example.com"}},"subject":"invoice ready", "internetMessageHeaders":headers}),
            )
        };
        let c = candidate(
            "campus",
            "thread",
            &[email],
            &["owner@example.edu".into()],
            None,
        )
        .unwrap();
        assert_eq!(
            engine::header_match(
                &Matcher {
                    subject_contains: vec!["invoice ready".into()],
                    header: [("X-Notice".into(), vec!["invoice ready".into()])].into(),
                    ..Default::default()
                },
                &c
            ),
            engine::HeaderMatch::Match
        );
    }
}

#[test]
fn undecodable_header_words_are_unknown_not_negative() {
    use crate::google::gmail::{parse_gmail_message, HeaderRead};
    use mecha_core::mail_rules::Matcher;
    for raw in [
        "=?UTF-8?B?invalid!?=",
        "=?utf-8?Q?=ZZ?=",
        "=?unknown-charset?Q?invoice?=",
        "=?utf-8?Q?=FF?=",
    ] {
        let email = parse_gmail_message(
            HeaderRead::Full,
            &json!({"id":"notice", "labelIds":["INBOX"], "payload":{"headers":[{"name":"From","value":"sender@example.com"},{"name":"Subject","value":raw},{"name":"X-Notice","value":raw}]}}),
        );
        let c = candidate(
            "campus",
            "thread",
            &[email],
            &["owner@example.edu".into()],
            None,
        )
        .unwrap();
        for matcher in [
            Matcher {
                subject_contains: vec!["invoice".into()],
                ..Default::default()
            },
            Matcher {
                header: [("X-Notice".into(), vec!["invoice".into()])].into(),
                ..Default::default()
            },
        ] {
            assert_eq!(
                engine::header_match(&matcher, &c),
                engine::HeaderMatch::Unknown,
                "{raw}"
            );
        }
    }
}

#[test]
fn historical_verdicts_are_not_after_classify_candidates() {
    let now = Utc::now();
    let record = |thread: &str, at: DateTime<Utc>| -> mail_triage::Record {
        serde_json::from_value(json!({"thread_id":thread,"account":"campus","state":"classified", "classified_at":at.to_rfc3339(), "verdict":{"bucket":"notify","urgency":"none","proposed":"none","one_line":"Fictional notice"}})).unwrap()
    };
    let records = [
        record("old", now - chrono::Duration::days(8)),
        record("recent", now - chrono::Duration::minutes(1)),
    ];
    assert_eq!(
        classified_candidates(&records, "campus", now),
        vec!["recent"]
    );
}

#[test]
fn graph_decoded_subject_is_not_decoded_twice_but_raw_headers_are() {
    let raw = "=?UTF-8?B?aW52b2ljZQ==?=";
    let email = crate::microsoft::graph_mail::parse_outlook_message(
        &json!({"id":"notice","isDraft":false,"from":{"emailAddress":{"address":"sender@example.com"}}, "subject":raw, "internetMessageHeaders":[{"name":"X-Notice","value":raw}]}),
    );
    let c = candidate(
        "campus",
        "thread",
        &[email],
        &["owner@example.edu".into()],
        None,
    )
    .unwrap();
    use mecha_core::mail_rules::Matcher;
    assert_eq!(
        engine::header_match(
            &Matcher {
                subject_contains: vec!["invoice".into()],
                ..Default::default()
            },
            &c
        ),
        engine::HeaderMatch::NoMatch
    );
    assert_eq!(
        engine::header_match(
            &Matcher {
                subject_contains: vec!["utf".into()],
                header: [("X-Notice".into(), vec!["invoice".into()])].into(),
                ..Default::default()
            },
            &c
        ),
        engine::HeaderMatch::Match
    );
    assert_eq!(c.headers["x-notice"], [raw]);
}

#[test]
fn after_classify_enumerates_recent_verdicts_and_outstanding_queue_without_writes() {
    use mecha_core::mail_rules::{DeferredEntry, LedgerRow, Mode, Outcome};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("triage");
    let store = RuleStore::open(dir.path().join("rules")).unwrap();
    let now = Utc::now();
    assert!(after_classify_candidates(&root, &store, "campus", now)
        .unwrap()
        .is_empty());
    assert!(!root.exists());
    let triage = mail_triage::TriageStore::open(&root).unwrap();
    let record = |id: &str, at: &str| -> mail_triage::Record {
        serde_json::from_value(json!({"thread_id":id,"account":"campus","state":"classified", "classified_at":at, "verdict":{"bucket":"notify","urgency":"none","proposed":"none"}})).unwrap()
    };
    for r in [
        record("recent", &(now - chrono::Duration::minutes(1)).to_rfc3339()),
        record("boundary", &(now - RECENT_WINDOW).to_rfc3339()),
        record(
            "old",
            &(now - RECENT_WINDOW - chrono::Duration::seconds(1)).to_rfc3339(),
        ),
        record("unknown", "invalid"),
        record("future", &(now + chrono::Duration::days(1)).to_rfc3339()),
    ] {
        triage.put(&r).unwrap();
    }
    let rule = store.install("digest", "description = 'Review notice'\n[match]\nfrom_domain = ['example.com']\n[[action]]\ndo = 'digest'", &Default::default()).unwrap();
    for account in ["campus", "personal"] {
        let c = candidate(
            account,
            "queued",
            &[email(
                "queued",
                "sender@example.com",
                &(now - chrono::Duration::days(30)).to_rfc3339(),
            )],
            &["owner@example.edu".into()],
            None,
        )
        .unwrap();
        let row = LedgerRow::header(
            &rule,
            0,
            c.message,
            Mode::Shadow,
            GateStatus::Passing,
            Outcome::WouldHave,
            now,
        )
        .unwrap();
        store
            .enqueue(&DeferredEntry {
                id: account.into(),
                account: account.into(),
                thread_id: "queued".into(),
                queued_at: now - chrono::Duration::days(1),
                actions: vec![row],
            })
            .unwrap();
    }
    let queue_before = std::fs::read(store.root().join("deferred.jsonl")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o750)).unwrap();
    assert_eq!(
        after_classify_candidates(&root, &store, "campus", now).unwrap(),
        ["boundary", "queued", "recent"]
    );
    assert_eq!(
        after_classify_candidates(&root, &store, "personal", now).unwrap(),
        ["queued"]
    );
    assert_eq!(
        std::fs::read(store.root().join("deferred.jsonl")).unwrap(),
        queue_before
    );
    assert_eq!(
        std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o750
    );
}

#[test]
fn duplicate_subjects_are_unknown_for_both_providers() {
    use mecha_core::mail_rules::{
        engine::{header_match, HeaderMatch},
        Matcher,
    };
    let headers = json!([
        {"name":"From", "value":"sender@example.com"},
        {"name":"Subject", "value":"Meeting agenda"},
        {"name":"sUbJeCt", "value":"invoice"}
    ]);
    let gmail = crate::google::gmail::parse_gmail_message(
        crate::google::gmail::HeaderRead::Full,
        &json!({"id":"message", "threadId":"thread", "labelIds":["INBOX"],
            "internalDate":"1793707200000", "payload":{"headers":headers}}),
    );
    let outlook = crate::microsoft::graph_mail::parse_outlook_message(&json!({
        "id":"message", "conversationId":"thread", "subject":"invoice", "isDraft":false,
        "receivedDateTime":"2026-11-03T12:00:00Z",
        "from":{"emailAddress":{"address":"sender@example.com"}},
        "internetMessageHeaders":headers}));
    for email in [gmail, outlook] {
        let c = candidate(
            "campus",
            "thread",
            std::slice::from_ref(&email),
            &["owner@example.edu".into()],
            None,
        )
        .unwrap();
        for needle in ["invoice", "absent"] {
            let matcher = Matcher {
                subject_contains: vec![needle.into()],
                ..Default::default()
            };
            assert_eq!(
                header_match(&matcher, &c),
                HeaderMatch::Unknown,
                "{}: {needle}",
                email.provider
            );
        }
        assert_eq!(
            header_match(
                &Matcher {
                    header: std::collections::BTreeMap::from([(
                        "Subject".into(),
                        vec!["invoice".into()]
                    )]),
                    ..Default::default()
                },
                &c
            ),
            HeaderMatch::Unknown
        );
        assert_eq!(email.subject_complete, Some(false));
        assert_eq!(email.subject, "invoice", "display value is retained");
        assert_eq!(email.headers["subject"].len(), 2);
    }
}

#[test]
fn duplicate_subject_with_a_missing_value_is_still_unknown() {
    use mecha_core::mail_rules::{
        engine::{header_match, HeaderMatch},
        Matcher,
    };
    let headers = json!([
        {"name":"From", "value":"sender@example.com"},
        {"name":"Subject"}, {"name":"Subject", "value":"invoice"}
    ]);
    let gmail = crate::google::gmail::parse_gmail_message(
        crate::google::gmail::HeaderRead::Full,
        &json!({"id":"message", "threadId":"thread", "labelIds":["INBOX"],
            "payload":{"headers":headers}}),
    );
    let outlook = crate::microsoft::graph_mail::parse_outlook_message(&json!({
        "id":"message", "conversationId":"thread", "subject":"invoice", "isDraft":false,
        "from":{"emailAddress":{"address":"sender@example.com"}},
        "internetMessageHeaders":headers}));
    for email in [gmail, outlook] {
        assert_eq!(email.subject_complete, Some(false));
        let c = candidate(
            "campus",
            "thread",
            &[email],
            &["owner@example.edu".into()],
            None,
        )
        .unwrap();
        assert_eq!(
            header_match(
                &Matcher {
                    subject_contains: vec!["invoice".into()],
                    ..Default::default()
                },
                &c
            ),
            HeaderMatch::Unknown
        );
    }
}

#[test]
fn unreadable_overlay_holds_engine_even_without_a_classifier_record() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_owned();
    let triage = mail_triage::TriageStore::open(&root).unwrap();
    rule_records::update(&root, "campus", "thread-alpha", |_| {}).unwrap();
    let path = std::fs::read_dir(root.join("rule-records"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "json"))
        .unwrap();
    std::fs::write(path, "corrupt").unwrap();
    for classified in [false, true] {
        if classified {
            let record =
                serde_json::from_value(json!({"account":"campus", "thread_id":"thread-alpha",
                "state":"classified", "classified_at":"2026-11-03T12:00:00Z",
                "verdict":{"bucket":"notify", "urgency":"none", "proposed":"none"}}))
                .unwrap();
            triage.put(&record).unwrap();
        }
        assert!(
            classified_record(&root, "campus", "thread-alpha").is_err(),
            "unknown annotations must stop engine authorization, even without a verdict"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn quoted_local_sender_keeps_display_and_reply_but_cannot_authorize_rules() {
    for address in [
        r#""quoted.local"@example.com"#,
        "sender@[192.0.2.1]",
        "élise@example.com",
    ] {
        let email = crate::google::gmail::parse_gmail_message(
            crate::google::gmail::HeaderRead::Full,
            &json!({"id":"message", "labelIds":["INBOX"], "payload":{"headers":[
                {"name":"From", "value":format!("Fixture <{address}>")},
                {"name":"To", "value":format!("Recipient <{address}>")}
            ]}}),
        );
        assert_eq!(email.from_name, "Fixture");
        assert_eq!(email.from_address, address);
        assert_eq!(email.to_addresses, [address]);
        let (to, _, _) =
            crate::unified::gmail_reply_fields(&email, Some("owner@example.edu"), false);
        assert_eq!(to, address);
        assert!(candidate(
            "campus",
            "thread",
            &[email],
            &["owner@example.edu".into()],
            None
        )
        .is_err());
    }
}

async fn graph_fate_unknown_reserves_both_caps(graph: &GraphSink) {
    use mecha_core::mail_rules::{ActionGroup, HeldReason, Mode, Outcome, P1Holds};
    for global in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = RuleStore::open(dir.path().join("rules")).unwrap();
        if global {
            std::fs::write(store.root().join("limits.toml"), "daily_cap = 1").unwrap();
        }
        let rule = store.install("task", &format!("description = 'Review notice'\ndaily_cap = {}\n[match]\nfrom_domain = ['example.com']\n[[action]]\ndo = 'task'", if global { 20 } else { 1 }), &Default::default()).unwrap();
        store
            .change_mode(
                &rule.name,
                ActionGroup::Board,
                Mode::Auto,
                Some(mecha_core::closure::Actor::Owner),
                &rule.approval_digest,
                &P1Holds {
                    origin_gate: GateStatus::Passing,
                },
            )
            .unwrap();
        let now = Utc::now() + chrono::Duration::hours(1);
        let make = |thread: &str| {
            let mut c = candidate(
                "campus",
                thread,
                &[email(thread, "sender@example.com", &now.to_rfc3339())],
                &["owner@example.edu".into()],
                None,
            )
            .unwrap();
            c.classified = Some(Classified {
                at: now,
                deadline: None,
            });
            FixtureMailbox(vec![c])
        };
        let options = engine::EngineOptions {
            pass: Pass::AfterClassify,
            clock: Arc::new(mecha_core::clock::FixedClock(now)),
            ..Default::default()
        };
        let first = engine::apply(
            &store,
            &make("alpha"),
            Some(graph),
            &["campus".into()],
            &options,
        )
        .await
        .unwrap();
        assert_eq!(first.rows.len(), 1);
        assert_eq!(first.rows[0].outcome, Outcome::Held);
        assert_eq!(
            HeldReason::from_ledger(&first.rows[0].reason),
            HeldReason::FateUnknown
        );
        assert_eq!(
            first.rows[0].reason, "fate unknown",
            "ledger wire text stays compatible"
        );
        let second = engine::apply(
            &store,
            &make("beta"),
            Some(graph),
            &["campus".into()],
            &options,
        )
        .await
        .unwrap();
        assert_eq!(second.rows.len(), 1);
        assert_eq!(second.rows[0].outcome, Outcome::Held);
        assert_eq!(
            second.rows[0].reason,
            if global { "global cap" } else { "rule cap" }
        );
    }
}

#[test]
fn display_recipient_fallback_cannot_supply_rule_matching_facts() {
    let email = crate::google::gmail::parse_gmail_message(
        crate::google::gmail::HeaderRead::Full,
        &json!({"id":"message", "labelIds":["INBOX"], "payload":{"headers":[
            {"name":"From", "value":"sender@example.com"},
            {"name":"To", "value":", person@example.edu"}
        ]}}),
    );
    assert_eq!(email.to_addresses, ["person@example.edu"]);
    assert_eq!(email.recipients_complete, Some(false));
    let c = candidate(
        "campus",
        "thread",
        &[email],
        &["owner@example.edu".into()],
        None,
    )
    .unwrap();
    assert_eq!(
        engine::header_match(
            &mecha_core::mail_rules::Matcher {
                to: vec!["person@example.edu".into()],
                ..Default::default()
            },
            &c
        ),
        engine::HeaderMatch::Unknown
    );
}

#[tokio::test]
async fn disabled_graph_is_not_configured_for_live_or_dry_runs() {
    let mut config = mecha_core::config::Config::default();
    config.mcp.push(mecha_core::config::McpServerConfig {
        name: "graph".into(),
        disabled: true,
        command: "must-not-run-disabled-graph".into(),
        ..Default::default()
    });
    assert!(
        graph_entry(&config).is_none(),
        "dry run must not offer a planned sink"
    );
    let error = GraphSink::connect(&config)
        .await
        .err()
        .expect("disabled graph cannot connect");
    assert_eq!(error.to_string(), "graph MCP is not configured");
    let dir = tempfile::tempdir().unwrap();
    let store = RuleStore::open(dir.path().join("rules")).unwrap();
    let rule = store.install("task", "description = 'Review notice'\n[match]\nfrom_domain = ['example.com']\n[[action]]\ndo = 'task'", &Default::default()).unwrap();
    store
        .change_mode(
            &rule.name,
            mecha_core::mail_rules::ActionGroup::Board,
            mecha_core::mail_rules::Mode::Auto,
            Some(mecha_core::closure::Actor::Owner),
            &rule.approval_digest,
            &mecha_core::mail_rules::P1Holds {
                origin_gate: GateStatus::Passing,
            },
        )
        .unwrap();
    let now = Utc::now() + chrono::Duration::hours(1);
    let mut c = candidate(
        "campus",
        "thread",
        &[email("message", "sender@example.com", &now.to_rfc3339())],
        &["owner@example.edu".into()],
        None,
    )
    .unwrap();
    c.classified = Some(Classified {
        at: now,
        deadline: None,
    });
    let mail = FixtureMailbox(vec![c]);
    let planned = PlannedSink;
    for disabled in [true, false] {
        config.mcp[0].disabled = disabled;
        let sink = graph_entry(&config).map(|_| &planned as &dyn TaskSink);
        let report = engine::apply(
            &store,
            &mail,
            sink,
            &["campus".into()],
            &engine::EngineOptions {
                pass: Pass::AfterClassify,
                dry_run: true,
                clock: Arc::new(mecha_core::clock::FixedClock(now)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(report.rows.len(), 1);
        assert_eq!(
            report.rows[0].outcome,
            if disabled {
                mecha_core::mail_rules::Outcome::Held
            } else {
                mecha_core::mail_rules::Outcome::Done
            }
        );
        assert_eq!(
            report.rows[0].reason,
            if disabled {
                "task sink unavailable"
            } else {
                "dry run: planned"
            }
        );
        assert!(store.ledger().unwrap().is_empty());
        assert!(store.deferred().unwrap().is_empty());
    }
}

#[test]
fn every_configured_account_address_is_required_for_owner_direction() {
    let dir = tempfile::tempdir().unwrap();
    let store = RuleStore::open(dir.path().join("rules")).unwrap();
    for missing in [None, Some(String::new()), Some("  ".into())] {
        assert!(
            owner_addresses(
                &[Some("primary@example.edu".into()), missing],
                Some("primary@example.edu"),
                &store
            )
            .is_err(),
            "a configured account cannot disappear from owner evidence"
        );
    }
}

#[tokio::test]
async fn disabled_graph_live_connection_is_refused_before_startup() {
    let mut config = mecha_core::config::Config::default();
    config.mcp.push(mecha_core::config::McpServerConfig {
        name: "graph".into(),
        disabled: true,
        command: "must-not-run-disabled-graph".into(),
        ..Default::default()
    });
    let error = GraphSink::connect(&config)
        .await
        .err()
        .expect("disabled graph cannot connect");
    assert_eq!(error.to_string(), "graph MCP is not configured");
}

#[test]
fn missing_second_account_address_holds_instead_of_selecting_its_outlook_reply() {
    let dir = tempfile::tempdir().unwrap();
    let store = RuleStore::open(dir.path().join("rules")).unwrap();
    let incoming = crate::microsoft::graph_mail::parse_outlook_message(&json!({
        "id":"inbound", "isDraft":false, "receivedDateTime":"2026-11-03T10:00:00Z",
        "from":{"emailAddress":{"address":"sender@example.com"}}
    }));
    let reply = crate::microsoft::graph_mail::parse_outlook_message(&json!({
        "id":"owner-reply", "isDraft":false, "receivedDateTime":"2026-11-03T12:00:00Z",
        "from":{"emailAddress":{"address":"secondary@example.edu"}}
    }));
    let emails = [incoming, reply];
    for second in [None, Some(String::new()), Some(" ".into())] {
        let owners = owner_addresses(
            &[Some("primary@example.edu".into()), second],
            Some("primary@example.edu"),
            &store,
        );
        let read = candidate_with_owners("campus", "thread", &emails, owners, None).unwrap();
        assert!(
            matches!(read, ThreadRead::Held { .. }),
            "unknown ownership authorized {read:?}"
        );
    }
    let owners = owner_addresses(
        &[
            Some("primary@example.edu".into()),
            Some("secondary@example.edu".into()),
        ],
        Some("primary@example.edu"),
        &store,
    );
    let ThreadRead::Ready(c) =
        candidate_with_owners("campus", "thread", &emails, owners, None).unwrap()
    else {
        panic!("repaired owner set must allow a read")
    };
    assert_eq!(c.message.provider_message_id, "inbound");
}

#[test]
fn uncertain_candidate_facts_are_held_reads() {
    let valid = email("inbound", "sender@example.com", "2026-11-03T12:00:00Z");
    let mut missing_direction = valid.clone();
    missing_direction.direction_evidence = None;
    let mut missing_identity = valid.clone();
    missing_identity.provider_id.clear();
    let mut missing_arrival = valid.clone();
    missing_arrival.provider_arrival_at = None;
    for emails in [
        Vec::new(),
        vec![missing_direction],
        vec![missing_identity],
        vec![valid, missing_arrival],
        vec![email("sent", "owner@example.edu", "2026-11-03T13:00:00Z")],
    ] {
        let read = candidate_with_owners(
            "campus",
            "thread",
            &emails,
            Ok(vec!["owner@example.edu".into()]),
            None,
        );
        assert!(
            matches!(read, Ok(ThreadRead::Held { .. })),
            "fact uncertainty became {read:?}"
        );
    }
    assert!(matches!(
        candidate_with_owners("campus", "thread", &[], Ok(Vec::new()), None),
        Ok(ThreadRead::Held { .. })
    ));
}

#[tokio::test]
async fn deferred_board_survives_unknown_from_until_expiry() {
    use mecha_core::mail_rules::{ActionGroup, Mode, Outcome, P1Holds};
    struct ChangingMailbox {
        messages: std::sync::Mutex<Vec<Email>>,
        record: mail_triage::Record,
    }
    #[async_trait]
    impl Mailbox for ChangingMailbox {
        async fn candidates(&self, _: &str, _: Pass) -> Result<Vec<String>> {
            Ok(vec!["thread".into()])
        }
        async fn read_thread(&self, account: &str, thread: &str) -> Result<ThreadRead> {
            candidate_with_owners(
                account,
                thread,
                &self.messages.lock().unwrap(),
                Ok(vec!["owner@example.edu".into()]),
                Some(&self.record),
            )
        }
        async fn mailbox_action(&self, _: &Candidate, _: ActionVerb) -> ActionResult {
            panic!("no mailbox effects")
        }
        async fn private_record(&self, _: &Candidate, _: &str, _: &Action) -> ActionResult {
            panic!("no record effects")
        }
        async fn handled(&self, _: &Candidate) -> Result<()> {
            panic!("no classification skips")
        }
        fn origin_gate(&self, _: &LoadedRule) -> GateStatus {
            GateStatus::Passing
        }
    }
    for bad_from in [
        json!([{"name":"From", "value":"Broken <sender@example.com"}]),
        json!([{"name":"From", "value":"first@example.com"}, {"name":"From", "value":"second@example.com"}]),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let store = RuleStore::open(dir.path().join("rules")).unwrap();
        let rule = store.install("task", "description = 'Review notice'\ndaily_cap = 1\n[match]\nfrom_domain = ['example.com']\n[[action]]\ndo = 'task'", &Default::default()).unwrap();
        store
            .change_mode(
                &rule.name,
                ActionGroup::Board,
                Mode::Auto,
                Some(mecha_core::closure::Actor::Owner),
                &rule.approval_digest,
                &P1Holds {
                    origin_gate: GateStatus::Passing,
                },
            )
            .unwrap();
        let now = Utc::now() + chrono::Duration::hours(1);
        let clock = Arc::new(mecha_core::clock::TestClock::new(now));
        let original = email("original", "sender@example.com", &now.to_rfc3339());
        let mailbox = ChangingMailbox {
            messages: std::sync::Mutex::new(vec![original]),
            record: serde_json::from_value(
                json!({"account":"campus", "thread_id":"thread", "state":"classified",
                "date":now.to_rfc3339(), "classified_at":now.to_rfc3339(),
                "verdict":{"bucket":"notify", "urgency":"none", "proposed":"none"}}),
            )
            .unwrap(),
        };
        let mut options = engine::EngineOptions {
            clock: clock.clone(),
            ..Default::default()
        };
        engine::apply(&store, &mailbox, None, &["campus".into()], &options)
            .await
            .unwrap();
        let queued = store.deferred().unwrap().remove(0);
        let before = store.ledger().unwrap().len();
        mailbox.messages.lock().unwrap().push(crate::google::gmail::parse_gmail_message(
            crate::google::gmail::HeaderRead::Full,
            &json!({"id":"later", "labelIds":["INBOX"], "internalDate":(now + chrono::Duration::seconds(1)).timestamp_millis().to_string(),
                "payload":{"headers":bad_from}})));
        options.pass = Pass::AfterClassify;
        for at in [now, queued.queued_at + chrono::Duration::days(7)] {
            clock.set(at);
            let report = engine::apply(
                &store,
                &mailbox,
                Some(&PlannedSink),
                &["campus".into()],
                &options,
            )
            .await
            .unwrap();
            assert_eq!(report.rows.len(), 1);
            assert_eq!(report.rows[0].outcome, Outcome::Held);
            assert!(!report.rows[0].closes);
            assert_eq!(store.ledger().unwrap().len(), before);
            assert_eq!(store.deferred().unwrap().len(), 1);
        }
        clock.advance(chrono::Duration::seconds(1));
        engine::apply(
            &store,
            &mailbox,
            Some(&PlannedSink),
            &["campus".into()],
            &options,
        )
        .await
        .unwrap();
        assert!(store.deferred().unwrap().is_empty());
        assert!(store
            .ledger()
            .unwrap()
            .iter()
            .any(|r| r.outcome == Outcome::Expired && r.closes));
        assert!(
            store.enqueue(&queued).is_err(),
            "expiry is terminal even if facts recover"
        );
    }
}

#[test]
fn uncertain_read_inputs_hold_but_provider_failures_remain_errors() {
    let owners = || Ok(vec!["owner@example.edu".into()]);
    let message = || {
        Ok(vec![email(
            "inbound",
            "sender@example.com",
            "2026-11-03T12:00:00Z",
        )])
    };
    let malformed = finish_read(
        "campus",
        "thread",
        Err(MailError::ParseError("missing message array".into())),
        owners(),
        Ok(None),
    );
    assert!(
        matches!(malformed, Ok(ThreadRead::Held { .. })),
        "unknown payload became {malformed:?}"
    );
    let annotations = finish_read(
        "campus",
        "thread",
        message(),
        owners(),
        Err(anyhow::anyhow!("unreadable annotation sidecar")),
    );
    assert!(
        matches!(annotations, Ok(ThreadRead::Held { .. })),
        "unknown annotations became {annotations:?}"
    );
    assert!(finish_read(
        "campus",
        "thread",
        Err(MailError::ApiError {
            status: 503,
            message: "unavailable".into()
        }),
        owners(),
        Ok(None)
    )
    .is_err());
}

#[test]
fn legacy_whole_second_verdicts_cover_fractional_provider_arrivals() {
    let arrival: DateTime<Utc> = "2026-11-03T12:00:00.123Z".parse().unwrap();
    let gmail = crate::google::gmail::parse_gmail_message(
        crate::google::gmail::HeaderRead::Full,
        &json!({"id":"message-alpha", "internalDate":arrival.timestamp_millis().to_string(),
            "labelIds":["INBOX"], "payload":{"headers":[{"name":"From","value":"sender@example.com"}]}}),
    );
    let outlook = crate::microsoft::graph_mail::parse_outlook_message(
        &json!({"id":"message-alpha", "receivedDateTime":arrival.to_rfc3339(),
            "isDraft":false, "from":{"emailAddress":{"address":"sender@example.com"}}}),
    );
    for email in [gmail, outlook] {
        let mut record: mail_triage::Record = serde_json::from_value(json!({
            "account":"campus", "thread_id":"thread-alpha", "state":"classified",
            "date":"2026-11-03T12:00:00Z", "classified_at":"2026-11-03T12:00:05Z",
            "verdict":{"bucket":"notify", "urgency":"none", "proposed":"none"}
        }))
        .unwrap();
        let read = |record: &mail_triage::Record| {
            candidate(
                "campus",
                "thread-alpha",
                std::slice::from_ref(&email),
                &["owner@example.edu".into()],
                Some(record),
            )
            .unwrap()
        };
        let fresh = read(&record);
        assert_eq!(
            fresh.arrival_at,
            Some(arrival),
            "eligibility must retain precision"
        );
        assert!(
            fresh.classified.is_some(),
            "legacy {} verdict was rejected",
            email.provider
        );
        record.date = "2026-11-03T11:59:59Z".into();
        assert!(
            read(&record).classified.is_none(),
            "a later classified_at must not certify stale input"
        );
    }
}

fn graph_test_task() -> Task {
    Task {
        name: "Review notice".into(),
        account: "campus".into(),
        thread_id: "thread-alpha".into(),
        received_at: Utc::now(),
        due: None,
    }
}

#[tokio::test]
async fn planned_sink_returns_environment_hold() {
    let result = PlannedSink.create(&graph_test_task()).await;
    assert!(
        matches!(&result, ActionResult::Held {kind: FailureKind::Environment, reason}
        if reason.to_string() == "dry run: sink not called"),
        "{result:?}"
    );
}

fn background_graph_fixture(dir: &std::path::Path) -> mecha_core::config::McpServerConfig {
    let script = dir.join("background_graph.py");
    std::fs::write(&script, r#"
import json, sys
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    method = request['method']
    if method == 'initialize':
        result = {'protocolVersion':'2024-11-05','capabilities':{},'serverInfo':{'name':'fixture','version':'test'}}
    elif method == 'tools/list':
        result = {'tools':[{'name':'kg_task_create','inputSchema':{'type':'object'},
            '_meta':{'mecha-factory.ai/background':sys.argv[1] == 'background'}}]}
    elif method == 'tools/call':
        with open('dispatched', 'a') as stream:
            stream.write('task\n')
        result = {'content':[{'type':'text','text':'created fictional task'}]}
    else:
        result = {}
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}), flush=True)
"#).unwrap();
    mecha_core::config::McpServerConfig {
        name: "graph".into(),
        command: "python3".into(),
        args: vec![script.display().to_string(), "background".into()],
        ..Default::default()
    }
}

#[tokio::test]
async fn background_graph_is_refused_before_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let entry = background_graph_fixture(dir.path());
    let result = GraphSink::connect_at(&Default::default(), &entry, dir.path().into()).await;
    if let Ok(graph) = &result {
        graph.client.close().await.unwrap();
    }
    assert!(result.is_err(), "background task tool must not be admitted");
    let notice = result.err().unwrap().to_string();
    assert!(
        notice.contains("kg_task_create") && notice.contains("background"),
        "{notice}"
    );
    assert!(!dir.path().join("dispatched").exists());
}

#[tokio::test]
async fn unexpected_deferred_graph_output_keeps_occasion_and_capacity_open() {
    use mecha_core::mail_rules::{ActionGroup, Mode, Outcome, P1Holds};
    let dir = tempfile::tempdir().unwrap();
    let mut entry = background_graph_fixture(dir.path());
    // Bypass connection validation to exercise the defensive output boundary.
    let client = McpClient::connect(
        &entry,
        &mecha_core::sandbox::Sandbox::new(Default::default()),
        dir.path(),
    )
    .await
    .unwrap();
    let create = client.list_tools().await.unwrap().remove(0);
    assert!(create.defers());
    let graph = GraphSink {
        client,
        create,
        ctx: ToolCtx {
            workspace: dir.path().into(),
            ..Default::default()
        },
    };
    let result = graph.create(&graph_test_task()).await;
    assert!(
        matches!(
            result,
            ActionResult::Held {
                kind: FailureKind::Environment,
                ..
            }
        ),
        "{result:?}"
    );
    assert!(!dir.path().join("dispatched").exists());

    let store = RuleStore::open(dir.path().join("rules")).unwrap();
    std::fs::write(store.root().join("limits.toml"), "daily_cap = 1").unwrap();
    let rule = store.install("task", "description = 'Review notice'\ndaily_cap = 1\n[match]\nfrom_domain = ['example.com']\n[[action]]\ndo = 'task'", &Default::default()).unwrap();
    store
        .change_mode(
            &rule.name,
            ActionGroup::Board,
            Mode::Auto,
            Some(mecha_core::closure::Actor::Owner),
            &rule.approval_digest,
            &P1Holds {
                origin_gate: GateStatus::Passing,
            },
        )
        .unwrap();
    let now = Utc::now() + chrono::Duration::hours(1);
    let mut c = candidate(
        "campus",
        "thread-alpha",
        &[email(
            "message-alpha",
            "sender@example.com",
            &now.to_rfc3339(),
        )],
        &["owner@example.edu".into()],
        None,
    )
    .unwrap();
    c.classified = Some(Classified {
        at: now,
        deadline: None,
    });
    let mail = FixtureMailbox(vec![c]);
    let mut options = engine::EngineOptions {
        clock: Arc::new(mecha_core::clock::TestClock::new(now)),
        ..Default::default()
    };
    engine::apply(&store, &mail, None, &["campus".into()], &options)
        .await
        .unwrap();
    options.pass = Pass::AfterClassify;
    for _ in 0..3 {
        let report = engine::apply(&store, &mail, Some(&graph), &["campus".into()], &options)
            .await
            .unwrap();
        assert_eq!(report.rows.len(), 1);
        assert_eq!(report.rows[0].outcome, Outcome::Held);
        assert!(
            !report.rows[0].closes,
            "deferred is undispatched, not unknown fate"
        );
        assert_eq!(report.rows[0].failure_kind, FailureKind::Environment);
        assert_eq!(store.deferred().unwrap().len(), 1);
        assert!(!store
            .ledger()
            .unwrap()
            .iter()
            .any(|r| r.is_action_failure()));
    }
    assert!(!dir.path().join("dispatched").exists());
    graph.client.close().await.unwrap();
    entry.args[1] = "inline".into();
    let repaired = GraphSink::connect_at(&Default::default(), &entry, dir.path().into())
        .await
        .unwrap();
    let report = engine::apply(&store, &mail, Some(&repaired), &["campus".into()], &options)
        .await
        .unwrap();
    assert_eq!(report.rows.len(), 1);
    assert_eq!(
        report.rows[0].outcome,
        Outcome::Done,
        "claim, both caps, and auto mode remain available"
    );
    assert!(store.deferred().unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("dispatched")).unwrap(),
        "task\n"
    );
    repaired.client.close().await.unwrap();
}

#[test]
fn rule_exit_status_ignores_downstream_notice_text() {
    let report = engine::ApplyReport {
        notices: vec!["task sink unavailable: downstream says thread unreadable".into()],
        ..Default::default()
    };
    assert!(
        report_status(&report).is_ok(),
        "downstream prose must not become a read failure"
    );
}

#[tokio::test]
async fn rule_exit_status_uses_read_failures_without_notices() {
    struct UnavailableMailbox;
    #[async_trait]
    impl Mailbox for UnavailableMailbox {
        async fn candidates(&self, _: &str, _: Pass) -> Result<Vec<String>> {
            Ok(vec!["thread-alpha".into()])
        }
        async fn read_thread(&self, _: &str, _: &str) -> Result<ThreadRead> {
            Err(MailError::ApiError {
                status: 503,
                message: "temporarily unavailable".into(),
            }
            .into())
        }
        async fn mailbox_action(&self, _: &Candidate, _: ActionVerb) -> ActionResult {
            panic!("no authorized message")
        }
        async fn private_record(&self, _: &Candidate, _: &str, _: &Action) -> ActionResult {
            panic!("no authorized record")
        }
        async fn handled(&self, _: &Candidate) -> Result<()> {
            panic!("no authorized skip")
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let store = RuleStore::open(dir.path()).unwrap();
    let mut report = engine::apply(
        &store,
        &UnavailableMailbox,
        None,
        &["campus".into()],
        &Default::default(),
    )
    .await
    .unwrap();
    assert!(!report.notices.is_empty());
    report.notices.clear();
    assert!(
        report_status(&report).is_err(),
        "read failure must survive reworded or missing notices"
    );
}

#[tokio::test]
async fn graph_close_failure_preserves_report_and_typed_exit_status() {
    for read_failures in 0..=1 {
        let report = engine::ApplyReport {
            read_failures,
            handled: vec!["thread-alpha".into()],
            ..Default::default()
        };
        let result = finish_apply(Ok(report), None, async {
            anyhow::bail!("fixture close failure")
        })
        .await;
        assert!(
            result.is_ok(),
            "cleanup discarded completed effects: {result:?}"
        );
        let report = result.unwrap();
        assert_eq!(report.handled, ["thread-alpha"]);
        assert_eq!(report.read_failures, read_failures);
        assert!(report
            .notices
            .iter()
            .any(|n| n.contains("close") && n.contains("fixture close failure")));
        assert_eq!(report_status(&report).is_err(), read_failures != 0);
    }
}
