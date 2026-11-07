use super::*;
use crate::closure::Actor;
use chrono::{Duration, Utc};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("mecha-mail-rules-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn store(&self) -> RuleStore {
        RuleStore::open(RuleStore::root_under(&self.0)).unwrap()
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn text(actions: &str) -> String {
    format!("description = 'Fictional notice'\n[match]\nfrom = ['robot@example.com']\n{actions}")
}
fn archive() -> String {
    text("[[action]]\ndo = 'archive'\n")
}
fn owner_holds() -> P1Holds {
    P1Holds {
        origin_gate: GateStatus::Passing,
    }
}
fn install(store: &RuleStore, text: &str) -> LoadedRule {
    store.install("notices", text, &BTreeMap::new()).unwrap()
}
fn approve(store: &RuleStore, rule: &LoadedRule, group: ActionGroup, mode: Mode) {
    let decision = store
        .change_mode(
            &rule.name,
            group,
            mode,
            Some(Actor::Owner),
            &rule.approval_digest,
            &owner_holds(),
        )
        .unwrap();
    assert!(
        !decision.refused,
        "unexpected refusal: {:?}",
        decision.holds
    );
}
fn message() -> MessageRef {
    MessageRef {
        account: "campus".into(),
        provider_message_id: "provider-alpha".into(),
        thread_id: "thread-alpha".into(),
        message_id: "<display@example.edu>".into(),
        sender: "robot@example.com".into(),
        subject: "Fictional notice".into(),
    }
}
fn row(rule: &LoadedRule, outcome: Outcome) -> LedgerRow {
    LedgerRow::header(
        rule,
        0,
        message(),
        Mode::Auto,
        GateStatus::Passing,
        outcome,
        Utc::now(),
    )
    .unwrap()
}
fn key(row: &LedgerRow) -> MessageKey {
    MessageKey {
        rule: row.rule.clone(),
        account: row.message.account.clone(),
        provider_message_id: row.message.provider_message_id.clone(),
        action: row.action.clone(),
    }
}
fn thread(row: &LedgerRow) -> ThreadKey {
    ThreadKey {
        rule: row.rule.clone(),
        account: row.message.account.clone(),
        thread_id: row.message.thread_id.clone(),
    }
}

#[test]
fn loader_parses_all_matchers_and_holds_a_description_whose_preconditions_the_sweep_cannot_read() {
    let home = Home::new();
    let store = home.store();
    let rule = install(
        &store,
        "description = 'Fictional notice'\n[match]\nfrom = ['robot@example.com']\nfrom_domain = ['example.com']\nto = ['owner@example.edu']\naccount = ['campus']\nlist_id = ['notices.example.com']\nsubject_contains = ['Notice']\nhas_attachment = false\ndescribe = 'A fictional notice'\n[match.header]\nX-Notice = ['yes']\n[[action]]\ndo = 'archive'",
    );
    assert_eq!(rule.definition.priority, 100);
    assert_eq!(rule.definition.daily_cap, 20);
    assert!(rule.definition.is_description());
    assert_eq!(rule.definition.matcher.header["X-Notice"], ["yes"]);
    // P4a: a description rule's Mailbox group may rise, but this one's
    // preconditions read headers the classify sweep never sees, so it is held
    // from evaluation altogether, and says so, rather than being guessed at.
    assert_eq!(
        approvals::mode_ceiling(&rule, ActionGroup::Mailbox),
        Mode::Auto
    );
    assert!(describe::precondition_unreadable(&rule.definition));
    let views = rule_views(&store).unwrap();
    assert!(views[0]
        .holds
        .iter()
        .any(|h| h == describe::PRECONDITION_HOLD));
    let resolved = resolve_mode(
        &rule,
        ActionGroup::Mailbox,
        &store.approvals().unwrap(),
        &owner_holds(),
    );
    assert_eq!(resolved.recorded, Mode::Shadow);
    assert_eq!(resolved.effective, Mode::Shadow);
    assert!(LedgerRow::header(
        &rule,
        0,
        message(),
        Mode::Shadow,
        GateStatus::Passing,
        Outcome::WouldHave,
        Utc::now()
    )
    .is_err());
}

#[test]
fn all_action_groups_load_and_outbound_cannot_leave_shadow() {
    let home = Home::new();
    let store = home.store();
    let text = text(
        "[[action]]\ndo='archive'\n[[action]]\ndo='read'\n[[action]]\ndo='unread'\n[[action]]\ndo='trash'\n[[action]]\ndo='spam'\n[[action]]\ndo='tag'\ntag='notice'\n[[action]]\ndo='digest'\n[[action]]\ndo='task'\n[[action]]\ndo='reply'\ntemplate='reply.md'\n[[action]]\ndo='forward'\nto=['records@example.edu']\nnote='reply.md'\n[[action]]\ndo='send'\nto=['records@example.edu']\ntemplate='receipt.md'\nfields={vendor='text', amount='money', count='number', date='date', contact='email'}",
    );
    let files = BTreeMap::from([
        ("reply.md".into(), "A fictional acknowledgement.".into()),
        (
            "receipt.md".into(),
            "Subject: Fictional receipt\n{vendor}".into(),
        ),
    ]);
    let rule = store.install("notices", &text, &files).unwrap();
    for group in rule.definition.groups() {
        if group.is_outbound() {
            let before = fs::read(store.root().join("approvals.jsonl")).unwrap();
            let decision = store
                .change_mode(
                    &rule.name,
                    group,
                    Mode::Propose,
                    Some(Actor::Owner),
                    &rule.approval_digest,
                    &owner_holds(),
                )
                .unwrap();
            assert!(decision.refused);
            assert!(!decision.write);
            assert!(!decision.clamped);
            assert_eq!(decision.target, Mode::Shadow);
            assert!(decision
                .holds
                .iter()
                .any(|hold| hold.ceiling == Mode::Shadow && !hold.reason.is_empty()));
            assert_eq!(
                fs::read(store.root().join("approvals.jsonl")).unwrap(),
                before
            );
        } else {
            approve(
                &store,
                &rule,
                group,
                if group == ActionGroup::Report {
                    Mode::Propose
                } else {
                    Mode::Auto
                },
            );
        }
    }
    assert_eq!(
        resolve_mode(
            &rule,
            ActionGroup::Report,
            &store.approvals().unwrap(),
            &owner_holds()
        )
        .effective,
        Mode::Propose
    );
    assert_eq!(
        resolve_mode(
            &rule,
            ActionGroup::Mailbox,
            &store.approvals().unwrap(),
            &owner_holds()
        )
        .effective,
        Mode::Auto
    );
    assert_eq!(
        resolve_mode(
            &rule,
            ActionGroup::Board,
            &store.approvals().unwrap(),
            &P1Holds::default()
        )
        .effective,
        Mode::Propose
    );
}

#[test]
fn action_identity_ignores_position_and_key_order_but_not_own_parameters() {
    let home = Home::new();
    let store = home.store();
    let first = install(
        &store,
        &text("[[action]]\ndo='tag'\ntag='notice'\n[[action]]\ndo='archive'"),
    );
    fs::write(
        first.directory.join("rule.toml"),
        text("[[action]]\ndo='archive'\n[[action]]\ntag='notice'\ndo='tag'"),
    )
    .unwrap();
    let second = store.get("notices").unwrap();
    assert_eq!(first.actions[0], second.actions[1]);
    assert_eq!(first.actions[1], second.actions[0]);
    fs::write(
        first.directory.join("rule.toml"),
        text("[[action]]\ndo='tag'\ntag='changed'"),
    )
    .unwrap();
    assert_ne!(first.actions[0], store.get("notices").unwrap().actions[0]);
    fs::write(
        first.directory.join("rule.toml"),
        text("[[action]]\ndo='archive'\n[[action]]\ndo='archive'"),
    )
    .unwrap();
    assert!(store.get("notices").is_err());
}

#[test]
fn invalid_names_modes_unknown_keys_and_bad_actions_fail_at_install() {
    let home = Home::new();
    let store = home.store();
    for name in ["../outside", "Upper", "a/b", "-name", "", "a.md"] {
        assert!(store.install(name, &archive(), &BTreeMap::new()).is_err());
    }
    for definition in [
        text("[[action]]\ndo='future'"),
        text("[[action]]\ndo='reply'"),
        text("[[action]]\ndo='tag'"),
        format!("mode='auto'\n{}", archive()),
        archive().replace("from =", "typo ="),
    ] {
        assert!(store
            .install("notices", &definition, &BTreeMap::new())
            .is_err());
    }
    assert!(store
        .install(
            "notices",
            &archive(),
            &BTreeMap::from([("../outside.md".into(), "fiction".into())])
        )
        .is_err());
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn named_files_cannot_escape_by_parent_absolute_path_or_symlink() {
    use std::os::unix::fs::symlink;
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    fs::write(home.0.join("outside.md"), "Fictional outside text").unwrap();
    for file in [
        "../../../outside.md".to_string(),
        home.0.join("outside.md").display().to_string(),
        "escape.md".into(),
    ] {
        if file == "escape.md" {
            symlink(home.0.join("outside.md"), rule.directory.join("escape.md")).unwrap();
        }
        fs::write(
            rule.directory.join("rule.toml"),
            text(&format!("[[action]]\ndo='reply'\ntemplate='{file}'")),
        )
        .unwrap();
        assert!(store.get("notices").is_err());
    }
    fs::remove_file(rule.directory.join("escape.md")).unwrap();
    fs::write(rule.directory.join("inside.md"), "Fictional inside text").unwrap();
    symlink("inside.md", rule.directory.join("alias.md")).unwrap();
    fs::write(
        rule.directory.join("rule.toml"),
        text("[[action]]\ndo='reply'\ntemplate='alias.md'"),
    )
    .unwrap();
    assert!(store.get("notices").is_ok());
}

#[test]
fn rule_toml_rule_directory_and_unnamed_files_are_contained_too() {
    use std::os::unix::fs::symlink;
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    fs::write(home.0.join("outside.toml"), archive()).unwrap();
    fs::remove_file(rule.directory.join("rule.toml")).unwrap();
    symlink(
        home.0.join("outside.toml"),
        rule.directory.join("rule.toml"),
    )
    .unwrap();
    assert!(store.get("notices").is_err());
    fs::remove_file(rule.directory.join("rule.toml")).unwrap();
    fs::write(rule.directory.join("rule.toml"), archive()).unwrap();
    symlink(
        home.0.join("outside.toml"),
        rule.directory.join("unused.md"),
    )
    .unwrap();
    assert!(store.get("notices").is_err());
    symlink(&rule.directory, store.root().join("rules/alias")).unwrap();
    assert!(store.get("alias").is_err());
}

#[test]
fn drafted_skill_uses_global_name_lookup_and_its_whole_directory_digest() {
    let home = Home::new();
    let store = home.store();
    let definition = text("[[action]]\ndo='reply'\ndraft_with='fictional'");
    assert!(store
        .install("notices", &definition, &BTreeMap::new())
        .is_err());
    let skill = home.0.join("skills/fictional");
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "---\nname: fictional\ndescription: Fictional procedure\n---\nWrite a fictional acknowledgement.").unwrap();
    let rule = install(&store, &definition);
    assert_eq!(
        rule.definition.actions[0].group(),
        ActionGroup::DraftedProse
    );
    fs::write(skill.join("reference.md"), "Fictional reference.").unwrap();
    assert_ne!(
        rule.approval_digest,
        store.get("notices").unwrap().approval_digest
    );
}

#[test]
fn typed_template_requires_subject_and_missing_file_fails_load() {
    let home = Home::new();
    let store = home.store();
    let definition = text(
        "[[action]]\ndo='send'\nto=['records@example.edu']\ntemplate='receipt.md'\nfields={vendor='text'}",
    );
    assert!(store
        .install("notices", &definition, &BTreeMap::new())
        .is_err());
    assert!(store
        .install(
            "notices",
            &definition,
            &BTreeMap::from([("receipt.md".into(), "No subject".into())])
        )
        .is_err());
}

#[test]
fn actor_gates_refuse_raises_clamp_off_and_leave_off_untouched() {
    for actor in [None, Some(Actor::Unknown), Some(Actor::OwnerApproved)] {
        assert!(gate_mode(Mode::Shadow, Mode::Auto, actor).is_err());
        assert!(gate_mode(Mode::Off, Mode::Propose, actor).is_err());
        let decision = gate_mode(Mode::Auto, Mode::Off, actor).unwrap();
        assert!(decision.clamped);
        assert_eq!(decision.target, Mode::Shadow);
        let decision = gate_mode(Mode::Off, Mode::Off, actor).unwrap();
        assert!(decision.clamped);
        assert!(!decision.write);
        assert_eq!(decision.target, Mode::Off);
    }
    assert_eq!(
        gate_mode(Mode::Auto, Mode::Off, Some(Actor::Owner))
            .unwrap()
            .target,
        Mode::Off
    );
    assert_eq!(
        gate_mode(Mode::Shadow, Mode::Auto, Some(Actor::Owner))
            .unwrap()
            .target,
        Mode::Auto
    );
}

#[test]
fn no_rows_shadow_and_latest_per_group_never_recovers_old_auto() {
    let home = Home::new();
    let store = home.store();
    let rule = install(
        &store,
        &text("[[action]]\ndo='archive'\n[[action]]\ndo='digest'"),
    );
    assert_eq!(
        resolve_mode(&rule, ActionGroup::Mailbox, &[], &owner_holds()).effective,
        Mode::Shadow
    );
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    approve(&store, &rule, ActionGroup::PrivateRecord, Mode::Propose);
    store
        .lower_modes(
            &rule.name,
            &BTreeMap::from([(ActionGroup::Mailbox, Mode::Shadow)]),
            None,
            &owner_holds(),
            ApprovalReason::Brake,
        )
        .unwrap();
    let rows = store.approvals().unwrap();
    assert_eq!(
        resolve_mode(&rule, ActionGroup::Mailbox, &rows, &owner_holds()).effective,
        Mode::Shadow
    );
    assert_eq!(
        resolve_mode(&rule, ActionGroup::PrivateRecord, &rows, &owner_holds()).effective,
        Mode::Propose
    );
    assert!(store
        .change_mode(
            &rule.name,
            ActionGroup::Mailbox,
            Mode::Auto,
            None,
            &rule.approval_digest,
            &owner_holds()
        )
        .is_err());
}

#[test]
fn lapses_are_per_group_minimum_exclude_off_and_revert_is_inert() {
    let home = Home::new();
    let store = home.store();
    let definition = text(
        "[[action]]\ndo='archive'\n[[action]]\ndo='digest'\n[[action]]\ndo='spam'\n[[action]]\ndo='task'",
    );
    let rule = install(&store, &definition);
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    approve(&store, &rule, ActionGroup::PrivateRecord, Mode::Propose);
    approve(&store, &rule, ActionGroup::PrivateRecord, Mode::Shadow);
    approve(&store, &rule, ActionGroup::Report, Mode::Off);
    approve(&store, &rule, ActionGroup::Board, Mode::Auto);
    fs::write(
        rule.directory.join("rule.toml"),
        definition.replace("Fictional notice", "Changed label"),
    )
    .unwrap();
    let changed = store.get("notices").unwrap();
    let before = store.approvals().unwrap();
    assert!(resolve_mode(&changed, ActionGroup::Mailbox, &before, &owner_holds()).lapsed);
    assert!(!resolve_mode(&changed, ActionGroup::Report, &before, &owner_holds()).lapsed);
    let affected = store
        .detect_lapses("notices", &owner_holds(), Utc::now())
        .unwrap();
    assert_eq!(affected.len(), 3);
    assert!(!affected.contains(&ActionGroup::Report));
    assert!(store
        .detect_lapses("notices", &owner_holds(), Utc::now())
        .unwrap()
        .is_empty());
    let after = store.approvals().unwrap();
    assert_eq!(
        resolve_mode(&changed, ActionGroup::PrivateRecord, &after, &owner_holds()).effective,
        Mode::Shadow
    );
    fs::write(rule.directory.join("rule.toml"), definition).unwrap();
    store
        .detect_lapses("notices", &owner_holds(), Utc::now())
        .unwrap();
    assert_eq!(
        resolve_mode(
            &rule,
            ActionGroup::Mailbox,
            &store.approvals().unwrap(),
            &owner_holds()
        )
        .effective,
        Mode::Propose
    );
    assert!(store
        .ledger()
        .unwrap()
        .iter()
        .all(|r| r.outcome == Outcome::ApprovalLapsed
            && !r.closes
            && !r.affected_groups.contains(&ActionGroup::Report)));
}

#[test]
fn digest_inputs_include_directory_and_board_globals_but_match_inputs_are_narrow() {
    let home = Home::new();
    let store = home.store();
    let rule = install(
        &store,
        &text("[[action]]\ndo='archive'\n[[action]]\ndo='task'"),
    );
    let measurement = rule
        .match_digest(ActionGroup::Mailbox, GateStatus::Passing)
        .unwrap();
    for name in ["aliases.toml", "limits.toml"] {
        let preceding = store.get("notices").unwrap().approval_digest;
        let bytes = if name == "aliases.toml" {
            "aliases=['alias@example.edu']"
        } else {
            "daily_cap=7"
        };
        fs::write(store.root().join(name), bytes).unwrap();
        let changed = store.get("notices").unwrap();
        assert_ne!(preceding, changed.approval_digest);
        assert_eq!(
            measurement,
            changed
                .match_digest(ActionGroup::Mailbox, GateStatus::Passing)
                .unwrap()
        );
    }
    fs::write(rule.directory.join("unused.md"), "Fictional extra file").unwrap();
    assert_ne!(
        rule.approval_digest.directory,
        store.get("notices").unwrap().approval_digest.directory
    );
    let board = rule
        .match_digest(ActionGroup::Board, GateStatus::Passing)
        .unwrap();
    assert_ne!(
        board,
        rule.match_digest(ActionGroup::Board, GateStatus::Failing)
            .unwrap()
    );
    assert_eq!(
        measurement,
        rule.match_digest(ActionGroup::Mailbox, GateStatus::Failing)
            .unwrap()
    );
    fs::write(rule.directory.join("rule.toml"), archive()).unwrap();
    let mailbox = store.get("notices").unwrap();
    assert!(mailbox.approval_digest.aliases.is_empty());
    assert!(mailbox.approval_digest.limits.is_empty());
}

#[test]
fn wire_fields_are_written_and_old_unknown_rows_degrade_in_safe_directions() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let mut value = serde_json::to_value(row(&rule, Outcome::WouldHave)).unwrap();
    assert_eq!(value["verdict_input"], "not_applicable");
    assert_eq!(value["match_digest"]["confirmation_version"], "");
    assert_eq!(value["match_digest"]["origin_gate"], "");
    assert!(value.get("closes").is_some());
    assert!(value.get("settlement").is_some());
    value.as_object_mut().unwrap().remove("verdict_input");
    value.as_object_mut().unwrap().remove("closes");
    value.as_object_mut().unwrap().remove("match_digest");
    let old: LedgerRow = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(old.verdict_input, VerdictInput::Unknown);
    assert!(!message_closed(std::slice::from_ref(&old), &key(&old)));
    for unknown in [
        serde_json::json!("future"),
        serde_json::json!({"future":true}),
        serde_json::Value::Null,
    ] {
        value["verdict_input"] = unknown.clone();
        value["outcome"] = unknown;
        let future: LedgerRow = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(future.verdict_input, VerdictInput::Unknown);
        assert!(message_closed(std::slice::from_ref(&future), &key(&future)));
    }
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    let mut approval = serde_json::to_value(&store.approvals().unwrap()[0]).unwrap();
    approval["to_mode"] = serde_json::json!("future");
    approval["actor"] = serde_json::json!("future");
    let future: ApprovalRow = serde_json::from_value(approval).unwrap();
    assert_eq!(future.actor, Actor::Unknown);
    assert_eq!(
        resolve_mode(&rule, ActionGroup::Mailbox, &[future], &owner_holds()).effective,
        Mode::Shadow
    );
}

#[test]
fn closing_union_honors_every_signal_and_presence_not_latest() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    for outcome in [
        Outcome::Sent,
        Outcome::Done,
        Outcome::Staged,
        Outcome::Offered,
        Outcome::Held,
        Outcome::Rejected,
        Outcome::Undone,
        Outcome::Edited,
        Outcome::ReleasedUnchanged,
        Outcome::Expired,
        Outcome::Unknown,
    ] {
        let mut closing = row(&rule, outcome);
        closing.closes = false;
        let key = key(&closing);
        assert!(message_closed(
            &[closing, row(&rule, Outcome::Skipped)],
            &key
        ));
    }
    for outcome in [Outcome::WouldHave, Outcome::Skipped, Outcome::Failed] {
        let mut open = row(&rule, outcome);
        let key = key(&open);
        assert!(!message_closed(std::slice::from_ref(&open), &key));
        open.closes = true;
        assert!(message_closed(&[open], &key));
    }
    let mut lapse = row(&rule, Outcome::ApprovalLapsed);
    lapse.closes = true;
    assert!(!message_closed(std::slice::from_ref(&lapse), &key(&lapse)));
}

#[test]
fn exact_pending_settlement_reopens_safe_key_but_never_another_attempt() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let pending = row(&rule, Outcome::Pending);
    let key = key(&pending);
    let settled = pending
        .settled(Outcome::Failed, "interrupted", Utc::now())
        .unwrap();
    assert!(!message_closed(&[pending.clone(), settled.clone()], &key));
    let second = row(&rule, Outcome::Pending);
    assert!(message_closed(
        &[pending.clone(), settled.clone(), second],
        &key
    ));
    let mut unrelated = settled.clone();
    unrelated.message.thread_id = "other".into();
    assert!(message_closed(&[pending.clone(), unrelated], &key));
    let mut unknown = settled;
    unknown.outcome = Outcome::Unknown;
    unknown.closes = false;
    assert!(message_closed(&[pending, unknown], &key));
}

#[test]
fn thread_keys_only_read_outbound_and_board_across_messages() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let mailbox = row(&rule, Outcome::Done);
    let thread_key = thread(&mailbox);
    assert!(!thread_closed(&[mailbox], &thread_key));
    for group in [
        ActionGroup::Board,
        ActionGroup::FixedWords,
        ActionGroup::VerbatimForward,
        ActionGroup::TypedFields,
        ActionGroup::DraftedProse,
    ] {
        for outcome in [
            Outcome::Pending,
            Outcome::Done,
            Outcome::Sent,
            Outcome::Staged,
            Outcome::Held,
            Outcome::Offered,
            Outcome::Failed,
            Outcome::Expired,
        ] {
            let mut action = row(&rule, outcome);
            action.group = group;
            action.closes = false;
            assert!(thread_closed(std::slice::from_ref(&action), &thread_key));
            let mut other_message = key(&action);
            other_message.provider_message_id = "new-message".into();
            assert!(!message_closed(&[action], &other_message));
        }
        for outcome in [Outcome::WouldHave, Outcome::Skipped] {
            let mut action = row(&rule, outcome);
            action.group = group;
            assert!(!thread_closed(&[action], &thread_key));
        }
    }
}

#[test]
fn interrupted_pending_is_retryable_only_for_mailbox_and_private_record() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let now = Utc::now();
    let mut pending = row(&rule, Outcome::Pending);
    pending.at = Some(now - Duration::hours(1));
    for group in [
        ActionGroup::Mailbox,
        ActionGroup::PrivateRecord,
        ActionGroup::Board,
        ActionGroup::FixedWords,
    ] {
        pending.group = group;
        let settled = pending
            .interrupted(now, Duration::minutes(20))
            .unwrap()
            .unwrap();
        assert!(!settled.is_action_failure());
        assert_eq!(
            settled.outcome,
            if group.once_per_thread() {
                Outcome::Held
            } else {
                Outcome::Failed
            }
        );
        assert_eq!(
            message_closed(&[pending.clone(), settled], &key(&pending)),
            group.once_per_thread()
        );
    }
}

#[test]
fn ledger_append_validates_settlement_and_preserves_history() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let pending = row(&rule, Outcome::Pending);
    let settled = pending
        .settled(Outcome::Failed, "interrupted", Utc::now())
        .unwrap();
    assert!(store.append_ledger(&settled).is_err());
    store.append_ledger(&pending).unwrap();
    store.append_ledger(&settled).unwrap();
    assert!(store.append_ledger(&settled).is_err());
    let other = pending.settled(Outcome::Done, "late", Utc::now()).unwrap();
    assert!(store.append_ledger(&other).is_err());
    assert_eq!(store.ledger().unwrap().len(), 2);
    assert!(!message_closed(&store.ledger().unwrap(), &key(&pending)));
}

#[test]
fn deferred_rewrites_removes_settled_and_expires_as_expired_not_failure() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let now = Utc::now();
    let action = row(&rule, Outcome::WouldHave);
    let mut entry = DeferredEntry {
        id: "queued-alpha".into(),
        account: action.message.account.clone(),
        thread_id: action.message.thread_id.clone(),
        queued_at: now - Duration::days(7),
        actions: vec![action.clone()],
    };
    store.enqueue(&entry).unwrap();
    assert!(store.expire_deferred(now).unwrap().is_empty());
    entry.queued_at = now;
    store.enqueue(&entry).unwrap();
    let expired = store.expire_deferred(now + Duration::seconds(1)).unwrap();
    assert_eq!(expired.len(), 1);
    assert!(store.deferred().unwrap().is_empty());
    let rows = store.ledger().unwrap();
    assert_eq!(rows[0].outcome, Outcome::Expired);
    assert!(!rows[0].is_action_failure());
    assert!(message_closed(&rows, &key(&action)));
    assert!(store
        .expire_deferred(now + Duration::days(1))
        .unwrap()
        .is_empty());
    assert!(store.enqueue(&entry).is_err());
    // A different provider message is a new mailbox occasion.
    entry.actions[0].message.provider_message_id = "provider-beta".into();
    store.enqueue(&entry).unwrap();
    assert!(store.settle_deferred(&entry.id, &[]).is_err());
    let mut done = row(&rule, Outcome::Done);
    done.message.provider_message_id = "provider-beta".into();
    store.settle_deferred(&entry.id, &[done]).unwrap();
    assert!(store.deferred().unwrap().is_empty());
    assert_eq!(
        fs::read(store.root().join("deferred.jsonl")).unwrap(),
        Vec::<u8>::new()
    );
}

#[test]
fn unreadable_history_and_global_files_are_errors_not_empty_state() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    fs::write(store.root().join("ledger.jsonl"), "{torn\n").unwrap();
    assert!(store.ledger().is_err());
    assert!(store.append_ledger(&row(&rule, Outcome::Done)).is_err());
    fs::write(store.root().join("approvals.jsonl"), "{torn\n").unwrap();
    assert!(store
        .change_mode(
            "notices",
            ActionGroup::Mailbox,
            Mode::Auto,
            Some(Actor::Owner),
            &rule.approval_digest,
            &owner_holds()
        )
        .is_err());
    fs::write(store.root().join("aliases.toml"), "aliases = false").unwrap();
    assert!(store.get("notices").is_err());
}

#[test]
fn brakes_have_strict_rate_thresholds_and_failure_floor() {
    let mut evidence = BrakeEvidence {
        automatic_mailbox: 10,
        undone_mailbox: 2,
        matches: 10,
        wrong_matches: 2,
        ..Default::default()
    };
    assert!(brake_targets([ActionGroup::Mailbox, ActionGroup::Board], &evidence).is_empty());
    evidence.undone_mailbox += 1;
    assert_eq!(
        brake_targets([ActionGroup::Mailbox, ActionGroup::Board], &evidence)
            .get(&ActionGroup::Mailbox),
        Some(&Mode::Propose)
    );
    evidence.wrong_matches += 1;
    assert_eq!(
        brake_targets([ActionGroup::Mailbox, ActionGroup::Board], &evidence)
            .get(&ActionGroup::Board),
        Some(&Mode::Propose)
    );
    evidence
        .consecutive_action_failures
        .insert(ActionGroup::Board, 3);
    assert_eq!(
        brake_targets([ActionGroup::Board], &evidence)[&ActionGroup::Board],
        Mode::Shadow
    );
}

#[test]
fn activation_survives_active_lowering_and_reapproval_but_ends_at_shadow() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Propose);
    let first = resolve_mode(
        &rule,
        ActionGroup::Mailbox,
        &store.approvals().unwrap(),
        &owner_holds(),
    )
    .activated_at;
    assert!(first.is_some());
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Propose);
    assert_eq!(
        first,
        resolve_mode(
            &rule,
            ActionGroup::Mailbox,
            &store.approvals().unwrap(),
            &owner_holds()
        )
        .activated_at
    );
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Shadow);
    assert!(resolve_mode(
        &rule,
        ActionGroup::Mailbox,
        &store.approvals().unwrap(),
        &owner_holds()
    )
    .activated_at
    .is_none());
}

#[test]
fn unknown_origin_is_never_clean_and_proposal_taint_uses_existing_origin() {
    assert_eq!(GateStatus::of_origins(&[]), GateStatus::Unknown);
    assert_eq!(
        GateStatus::of_origins(&[crate::imagelib::Origin::of_proposal(None)]),
        GateStatus::Failing
    );
    assert_eq!(
        GateStatus::of_origins(&[
            crate::imagelib::Origin::Owner,
            crate::imagelib::Origin::ModelClean
        ]),
        GateStatus::Passing
    );
    let origin: crate::imagelib::Origin = serde_json::from_str("\"future\"").unwrap();
    assert_eq!(GateStatus::of_origins(&[origin]), GateStatus::Failing);
}

#[test]
fn a_step_down_persists_even_when_a_computed_hold_already_lowered_the_mode() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &text("[[action]]\ndo='task'"));
    approve(&store, &rule, ActionGroup::Board, Mode::Auto);
    let held = P1Holds::default();
    assert_eq!(
        resolve_mode(
            &rule,
            ActionGroup::Board,
            &store.approvals().unwrap(),
            &held
        )
        .effective,
        Mode::Propose
    );
    store
        .lower_modes(
            "notices",
            &BTreeMap::from([(ActionGroup::Board, Mode::Propose)]),
            None,
            &held,
            ApprovalReason::Brake,
        )
        .unwrap();
    assert_eq!(
        resolve_mode(
            &rule,
            ActionGroup::Board,
            &store.approvals().unwrap(),
            &owner_holds()
        )
        .effective,
        Mode::Propose
    );
}

#[test]
fn claim_serializes_the_key_check_and_allows_sibling_actions_in_one_occasion() {
    let home = Home::new();
    let store = home.store();
    let rule = install(
        &store,
        &text("[[action]]\ndo='archive'\n[[action]]\ndo='task'"),
    );
    let first = row(&rule, Outcome::Pending);
    let task = LedgerRow::header(
        &rule,
        1,
        message(),
        Mode::Auto,
        GateStatus::Passing,
        Outcome::Pending,
        Utc::now(),
    )
    .unwrap();
    let claims = store.claim_actions(&[first.clone(), task.clone()]).unwrap();
    assert_eq!(claims.len(), 2);
    assert!(claims
        .iter()
        .all(|claim| claim.status == ClaimStatus::Claimed));
    assert_eq!(
        store
            .claim_actions(&[row(&rule, Outcome::Pending)])
            .unwrap()[0]
            .status,
        ClaimStatus::MessageClosed
    );
    let settled = first
        .settled(Outcome::Failed, "interrupted", Utc::now())
        .unwrap();
    store.append_ledger(&settled).unwrap();
    assert_eq!(
        store
            .claim_actions(&[row(&rule, Outcome::Pending)])
            .unwrap()[0]
            .status,
        ClaimStatus::Claimed
    );
    let mut later_task = task;
    later_task.id = "another-attempt".into();
    later_task.message.provider_message_id = "new-message".into();
    assert_eq!(
        store.claim_actions(&[later_task]).unwrap()[0].status,
        ClaimStatus::ThreadClosed
    );
}

#[test]
fn concurrent_queue_rewrites_keep_both_entries_and_concurrent_claims_have_one_winner() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let mut handles = Vec::new();
    for name in ["alpha", "beta"] {
        let root = store.root().to_path_buf();
        let mut action = row(&rule, Outcome::WouldHave);
        action.message.thread_id = name.into();
        handles.push(std::thread::spawn(move || {
            let store = RuleStore::open(root).unwrap();
            store
                .enqueue(&DeferredEntry {
                    id: name.into(),
                    account: action.message.account.clone(),
                    thread_id: name.into(),
                    queued_at: Utc::now(),
                    actions: vec![action],
                })
                .unwrap();
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(store.deferred().unwrap().len(), 2);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut handles = Vec::new();
    for _ in ["alpha", "beta"] {
        let root = store.root().to_path_buf();
        let pending = row(&rule, Outcome::Pending);
        let barrier = barrier.clone();
        handles.push(std::thread::spawn(move || {
            let store = RuleStore::open(root).unwrap();
            barrier.wait();
            store.claim_actions(&[pending]).unwrap()
        }));
    }
    let winners = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .filter(|claims| claims[0].status == ClaimStatus::Claimed)
        .count();
    assert_eq!(winners, 1);
}

#[test]
fn gates_use_closure_attribution_without_treating_a_registered_run_as_owner() {
    use crate::closure::{attribute, read_posture, RunPosture, ShellReading};
    let absent = read_posture(None);
    for shell in [
        ShellReading::Unreadable("fictional read failure".into()),
        ShellReading::Registered {
            pid: std::process::id(),
            posture: Ok(RunPosture::Interactive),
        },
        ShellReading::Registered {
            pid: std::process::id(),
            posture: Ok(RunPosture::Unattended),
        },
    ] {
        let actor = attribute(&absent, &shell, None);
        assert!(gate_mode(Mode::Shadow, Mode::Auto, Some(actor)).is_err());
        assert!(
            gate_mode(Mode::Auto, Mode::Off, Some(actor))
                .unwrap()
                .clamped
        );
    }
    let actor = attribute(
        &absent,
        &ShellReading::NotRegistered,
        Some(std::process::id()),
    );
    assert!(gate_mode(Mode::Shadow, Mode::Auto, Some(actor)).is_err());
    let actor = attribute(&absent, &ShellReading::NotRegistered, None);
    assert!(gate_mode(Mode::Shadow, Mode::Auto, Some(actor)).is_ok());
}

#[test]
fn old_approvals_and_unknown_actions_keep_rows_readable_without_authority() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let old: ApprovalRow = serde_json::from_value(
        serde_json::json!({"id":"old", "rule":"notices", "group":"mailbox"}),
    )
    .unwrap();
    assert_eq!(old.actor, Actor::Unknown);
    assert_eq!(
        resolve_mode(&rule, ActionGroup::Mailbox, &[old], &owner_holds()).effective,
        Mode::Shadow
    );
    let mut value = serde_json::to_value(row(&rule, Outcome::WouldHave)).unwrap();
    value["action"]["verb"] = serde_json::json!("future_action");
    value["group"] = serde_json::json!("future_group");
    value["mode"] = serde_json::json!("future_mode");
    value["failure_kind"] = serde_json::json!("future_failure");
    let loaded: LedgerRow = serde_json::from_value(value).unwrap();
    assert_eq!(loaded.action.verb, ActionVerb::Unknown);
    assert_eq!(loaded.mode.effective(), Mode::Shadow);
    assert!(!loaded.is_action_failure());
    assert!(thread_closed(
        std::slice::from_ref(&loaded),
        &thread(&loaded)
    ));
}

#[test]
fn match_digest_changes_only_for_matcher_priority_and_applicable_gate() {
    let home = Home::new();
    let store = home.store();
    let original = text("[[action]]\ndo='tag'\ntag='notice'");
    let rule = install(&store, &original);
    let before = rule
        .match_digest(ActionGroup::PrivateRecord, GateStatus::Passing)
        .unwrap();
    fs::write(
        rule.directory.join("rule.toml"),
        original
            .replace("Fictional notice", "Changed label")
            .replace("tag='notice'", "tag='changed'"),
    )
    .unwrap();
    let edited = store.get("notices").unwrap();
    assert_ne!(rule.approval_digest, edited.approval_digest);
    assert_eq!(
        before,
        edited
            .match_digest(ActionGroup::PrivateRecord, GateStatus::Passing)
            .unwrap()
    );
    fs::write(
        rule.directory.join("rule.toml"),
        format!("priority=100\n{original}"),
    )
    .unwrap();
    assert_eq!(
        before,
        store
            .get("notices")
            .unwrap()
            .match_digest(ActionGroup::PrivateRecord, GateStatus::Passing)
            .unwrap()
    );
    fs::write(
        rule.directory.join("rule.toml"),
        format!("priority=50\n{original}"),
    )
    .unwrap();
    assert_ne!(
        before,
        store
            .get("notices")
            .unwrap()
            .match_digest(ActionGroup::PrivateRecord, GateStatus::Passing)
            .unwrap()
    );
    fs::write(
        rule.directory.join("rule.toml"),
        original.replace("robot@example.com", "other@example.com"),
    )
    .unwrap();
    assert_ne!(
        before,
        store
            .get("notices")
            .unwrap()
            .match_digest(ActionGroup::PrivateRecord, GateStatus::Passing)
            .unwrap()
    );
}

#[test]
fn stale_review_refuses_and_nonowner_pause_records_its_actual_actor() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    let decision = store
        .change_mode(
            "notices",
            ActionGroup::Mailbox,
            Mode::Off,
            None,
            &rule.approval_digest,
            &owner_holds(),
        )
        .unwrap();
    assert!(decision.clamped);
    assert_eq!(
        store.approvals().unwrap().last().unwrap().actor,
        Actor::Unknown
    );
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Off);
    let count = store.approvals().unwrap().len();
    store
        .lower_modes(
            "notices",
            &BTreeMap::from([(ActionGroup::Mailbox, Mode::Shadow)]),
            None,
            &owner_holds(),
            ApprovalReason::Brake,
        )
        .unwrap();
    assert_eq!(count, store.approvals().unwrap().len());
    fs::write(rule.directory.join("extra.md"), "Fictional extra text").unwrap();
    assert!(store
        .change_mode(
            "notices",
            ActionGroup::Mailbox,
            Mode::Auto,
            Some(Actor::Owner),
            &rule.approval_digest,
            &owner_holds()
        )
        .is_err());
}

#[test]
fn store_files_and_skill_files_refuse_outside_symlinks() {
    use std::os::unix::fs::symlink;
    let home = Home::new();
    let store = home.store();
    fs::write(home.0.join("external.jsonl"), "").unwrap();
    symlink(
        home.0.join("external.jsonl"),
        store.root().join("ledger.jsonl"),
    )
    .unwrap();
    assert!(store.ledger().is_err());
    let skill = home.0.join("skills/fictional");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        home.0.join("outside-skill.md"),
        "---\nname: fictional\ndescription: Fictional procedure\n---\nFiction.",
    )
    .unwrap();
    symlink(home.0.join("outside-skill.md"), skill.join("SKILL.md")).unwrap();
    assert!(store
        .install(
            "notices",
            &text("[[action]]\ndo='reply'\ndraft_with='fictional'"),
            &BTreeMap::new()
        )
        .is_err());
}

fn historical_row(
    rule: &LoadedRule,
    group: ActionGroup,
    from: Mode,
    to: Mode,
) -> serde_json::Value {
    serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(), "rule": rule.name, "group": group,
        "from_mode": from, "to_mode": to, "actor": "owner", "digests": rule.approval_digest,
        "at": Utc::now(), "reason": "owner"
    })
}

#[test]
fn unreadable_approval_authority_never_grants_auto_or_off() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    for target in [Mode::Auto, Mode::Off] {
        for actor in [
            None,
            Some(serde_json::json!("future_actor")),
            Some(serde_json::json!({"bad":true})),
        ] {
            let mut value = historical_row(&rule, ActionGroup::Mailbox, Mode::Shadow, target);
            if let Some(actor) = actor {
                value["actor"] = actor;
            } else {
                value.as_object_mut().unwrap().remove("actor");
            }
            let approval: ApprovalRow = serde_json::from_value(value).unwrap();
            let resolved = resolve_mode(&rule, ActionGroup::Mailbox, &[approval], &owner_holds());
            assert_eq!(
                resolved.effective,
                Mode::Shadow,
                "unreadable authority granted {target:?}"
            );
        }
    }
    let mut forged = historical_row(&rule, ActionGroup::Mailbox, Mode::Auto, Mode::Propose);
    forged["actor"] = serde_json::json!("unknown");
    let approval = serde_json::from_value(forged).unwrap();
    assert_eq!(
        resolve_mode(&rule, ActionGroup::Mailbox, &[approval], &owner_holds()).effective,
        Mode::Shadow
    );
}

#[test]
fn unreadable_approval_group_holds_every_group_instead_of_resurrecting_auto() {
    let home = Home::new();
    let store = home.store();
    let rule = install(
        &store,
        &text("[[action]]\ndo='archive'\n[[action]]\ndo='task'"),
    );
    for corrupted in [
        None,
        Some(serde_json::json!("future_group")),
        Some(serde_json::json!(false)),
    ] {
        let mut rows: Vec<ApprovalRow> = [ActionGroup::Mailbox, ActionGroup::Board]
            .into_iter()
            .map(|group| {
                serde_json::from_value(historical_row(&rule, group, Mode::Shadow, Mode::Auto))
                    .unwrap()
            })
            .collect();
        let mut brake = historical_row(&rule, ActionGroup::Mailbox, Mode::Auto, Mode::Shadow);
        if let Some(group) = corrupted {
            brake["group"] = group;
        } else {
            brake.as_object_mut().unwrap().remove("group");
        }
        rows.push(serde_json::from_value(brake).unwrap());
        for group in rule.definition.groups() {
            assert_eq!(
                resolve_mode(&rule, group, &rows, &owner_holds()).effective,
                Mode::Shadow
            );
        }
    }
}

#[test]
fn removing_a_group_then_restoring_it_cannot_restore_auto() {
    let home = Home::new();
    let store = home.store();
    let definition = text("[[action]]\ndo='archive'\n[[action]]\ndo='digest'");
    let rule = install(&store, &definition);
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    fs::write(
        rule.directory.join("rule.toml"),
        text("[[action]]\ndo='digest'"),
    )
    .unwrap();
    let affected = store
        .detect_lapses("notices", &owner_holds(), Utc::now())
        .unwrap();
    assert!(affected.contains(&ActionGroup::Mailbox));
    fs::write(rule.directory.join("rule.toml"), definition).unwrap();
    assert_eq!(
        resolve_mode(
            &rule,
            ActionGroup::Mailbox,
            &store.approvals().unwrap(),
            &owner_holds()
        )
        .effective,
        Mode::Propose
    );
}

#[test]
fn installation_reserves_a_name_in_shadow_before_publication_and_refuses_reuse() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let rows = store.approvals().unwrap();
    assert!(
        rows.iter()
            .any(|row| row.rule == rule.name && row.to_mode == Mode::Shadow),
        "installation has no durable shadow initialization"
    );
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    fs::remove_dir_all(&rule.directory).unwrap();
    for definition in [archive(), text("[[action]]\ndo='read'")] {
        assert!(store
            .install("notices", &definition, &BTreeMap::new())
            .is_err());
        assert!(!rule.directory.exists());
    }
}

#[test]
fn incomplete_ledger_keys_fail_reads_and_claims_instead_of_hiding_closures() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &text("[[action]]\ndo='task'"));
    let done = row(&rule, Outcome::Done);
    for field in ["account", "thread_id", "provider_message_id", "rule", "id"] {
        let mut value = serde_json::to_value(&done).unwrap();
        value.as_object_mut().unwrap().remove(field);
        fs::write(
            store.root().join("ledger.jsonl"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        assert!(
            store.ledger().is_err(),
            "missing {field} hid closing evidence"
        );
        let mut pending = row(&rule, Outcome::Pending);
        pending.message.provider_message_id = "later-message".into();
        assert!(store.claim_actions(&[pending]).is_err());
    }
    fs::write(store.root().join("ledger.jsonl"), "").unwrap();
    let mut pending = row(&rule, Outcome::Pending);
    pending.message.account.clear();
    assert!(store.claim_actions(&[pending]).is_err());
}

struct InvalidOffHold;
impl HoldProvider for InvalidOffHold {
    fn holds(&self, _: &LoadedRule, _: ActionGroup) -> Vec<ModeHold> {
        vec![ModeHold {
            ceiling: Mode::Off,
            reason: "invalid test hold".into(),
        }]
    }
}

#[test]
fn off_holds_cannot_write_owner_only_modes_through_lapses_or_changes() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    store
        .change_mode(
            "notices",
            ActionGroup::Mailbox,
            Mode::Off,
            None,
            &rule.approval_digest,
            &InvalidOffHold,
        )
        .unwrap();
    assert!(store
        .approvals()
        .unwrap()
        .iter()
        .all(|r| r.to_mode != Mode::Off || r.actor == Actor::Owner));
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    fs::write(rule.directory.join("extra.md"), "Fictional extra text").unwrap();
    store
        .detect_lapses("notices", &InvalidOffHold, Utc::now())
        .unwrap();
    assert!(store
        .approvals()
        .unwrap()
        .iter()
        .all(|r| r.to_mode != Mode::Off || r.actor == Actor::Owner));
    assert_eq!(
        resolve_mode(
            &store.get("notices").unwrap(),
            ActionGroup::Mailbox,
            &store.approvals().unwrap(),
            &InvalidOffHold
        )
        .effective,
        Mode::Shadow
    );
}

#[test]
fn append_terminates_complete_final_rows_and_refuses_torn_rows() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let first = row(&rule, Outcome::WouldHave);
    fs::write(
        store.root().join("ledger.jsonl"),
        serde_json::to_vec(&first).unwrap(),
    )
    .unwrap();
    store.append_ledger(&row(&rule, Outcome::Done)).unwrap();
    assert_eq!(store.ledger().unwrap().len(), 2);
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    let path = store.root().join("approvals.jsonl");
    let bytes = fs::read(&path).unwrap();
    fs::write(&path, bytes.strip_suffix(b"\n").unwrap()).unwrap();
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Shadow);
    assert_eq!(
        store.approvals().unwrap().last().unwrap().to_mode,
        Mode::Shadow
    );
    fs::write(store.root().join("ledger.jsonl"), "{\"id\":").unwrap();
    assert!(store.append_ledger(&row(&rule, Outcome::Done)).is_err());
}

#[test]
fn limits_mutation_changes_digest_independently_of_prior_alias_edits() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &text("[[action]]\ndo='task'"));
    fs::write(
        store.root().join("aliases.toml"),
        "aliases=['alias@example.edu']",
    )
    .unwrap();
    let before = store.get(&rule.name).unwrap().approval_digest;
    fs::write(store.root().join("limits.toml"), "daily_cap=7").unwrap();
    let after = store.get(&rule.name).unwrap().approval_digest;
    assert_ne!(before, after);
    assert_ne!(before.limits, after.limits);
    assert_eq!(before.aliases, after.aliases);
}

#[test]
fn a_nonowner_cannot_rebind_auto_to_new_inputs_without_a_raise_row() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let approved: ApprovalRow = serde_json::from_value(historical_row(
        &rule,
        ActionGroup::Mailbox,
        Mode::Shadow,
        Mode::Auto,
    ))
    .unwrap();
    fs::write(rule.directory.join("extra.md"), "Fictional changed input").unwrap();
    let changed = store.get("notices").unwrap();
    let mut value = historical_row(&changed, ActionGroup::Mailbox, Mode::Auto, Mode::Auto);
    value["actor"] = serde_json::json!("unknown");
    let rebound: ApprovalRow = serde_json::from_value(value).unwrap();
    assert_eq!(
        resolve_mode(
            &changed,
            ActionGroup::Mailbox,
            &[approved, rebound],
            &owner_holds()
        )
        .effective,
        Mode::Shadow
    );
}

#[test]
fn corrupt_history_stays_held_until_a_valid_owner_transition_repairs_it() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let approved = historical_row(&rule, ActionGroup::Mailbox, Mode::Shadow, Mode::Auto);
    for field in ["actor", "from_mode", "to_mode"] {
        let mut corrupt = historical_row(&rule, ActionGroup::Mailbox, Mode::Auto, Mode::Propose);
        corrupt.as_object_mut().unwrap().remove(field);
        let mut rows: Vec<ApprovalRow> = vec![
            serde_json::from_value(approved.clone()).unwrap(),
            serde_json::from_value(corrupt).unwrap(),
        ];
        let held = resolve_mode(&rule, ActionGroup::Mailbox, &rows, &owner_holds());
        assert_eq!(held.effective, Mode::Shadow);
        assert!(held.activated_at.is_none());
        let mut lowering = historical_row(&rule, ActionGroup::Mailbox, Mode::Shadow, Mode::Shadow);
        lowering["actor"] = serde_json::json!("unknown");
        rows.push(serde_json::from_value(lowering).unwrap());
        assert!(
            !resolve_mode(&rule, ActionGroup::Mailbox, &rows, &owner_holds())
                .holds
                .is_empty()
        );
        rows.push(
            serde_json::from_value(historical_row(
                &rule,
                ActionGroup::Mailbox,
                Mode::Shadow,
                Mode::Auto,
            ))
            .unwrap(),
        );
        assert_eq!(
            resolve_mode(&rule, ActionGroup::Mailbox, &rows, &owner_holds()).effective,
            Mode::Auto
        );
    }
}

#[test]
fn malformed_action_identity_blocks_even_the_pure_key_readers() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let intact = row(&rule, Outcome::Done);
    let mut corrupt = intact.clone();
    corrupt.action.digest.clear();
    assert!(message_closed(
        std::slice::from_ref(&corrupt),
        &key(&intact)
    ));
    assert!(thread_closed(
        std::slice::from_ref(&corrupt),
        &thread(&intact)
    ));
    fs::write(
        store.root().join("ledger.jsonl"),
        serde_json::to_vec(&corrupt).unwrap(),
    )
    .unwrap();
    assert!(store.ledger().is_err());
}

#[test]
fn unreadable_action_verb_closes_the_original_message_key() {
    for outcome in [Outcome::Done, Outcome::Undone] {
        for verb in [
            None,
            Some(serde_json::json!({})),
            Some(serde_json::json!("future")),
        ] {
            let home = Home::new();
            let store = home.store();
            let rule = install(&store, &archive());
            let original = row(&rule, outcome);
            let mut value = serde_json::to_value(&original).unwrap();
            value["action"].as_object_mut().unwrap().remove("verb");
            if let Some(verb) = verb {
                value["action"]["verb"] = verb;
            }
            fs::write(
                store.root().join("ledger.jsonl"),
                serde_json::to_vec(&value).unwrap(),
            )
            .unwrap();
            let rows = store.ledger().unwrap();
            assert!(message_closed(&rows, &key(&original)));
            assert_eq!(
                store
                    .claim_actions(&[row(&rule, Outcome::Pending)])
                    .unwrap()[0]
                    .status,
                ClaimStatus::MessageClosed
            );
            let mut other = key(&original);
            other.provider_message_id = "provider-beta".into();
            assert!(!message_closed(&rows, &other));
            other = key(&original);
            other.action.digest = "unrelated-action".into();
            assert!(!message_closed(&rows, &other));
        }
    }
}

#[test]
fn ambiguous_settlements_cannot_reopen_the_original_action_key() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let pending = row(&rule, Outcome::Pending);
    let failed = pending
        .settled(Outcome::Failed, "fictional failure", Utc::now())
        .unwrap();
    let mut ambiguous_pending = pending.clone();
    ambiguous_pending.action.verb = ActionVerb::Unknown;
    let mut ambiguous_failed = failed.clone();
    ambiguous_failed.action.verb = ActionVerb::Unknown;
    for rows in [
        vec![ambiguous_pending.clone(), failed],
        vec![pending.clone(), ambiguous_failed.clone()],
        vec![ambiguous_pending, ambiguous_failed],
    ] {
        assert!(message_closed(&rows, &key(&pending)));
    }
}

fn broken_definition_lowers(operation: &str) {
    for missing in [true, false] {
        let home = Home::new();
        let store = home.store();
        let rule = install(&store, &archive());
        approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
        let path = rule.directory.join("rule.toml");
        if missing {
            fs::remove_file(&path).unwrap();
        } else {
            fs::write(&path, "[invalid").unwrap();
        }
        match operation {
            "off" => {
                let decision = store
                    .change_mode(
                        "notices",
                        ActionGroup::Mailbox,
                        Mode::Off,
                        Some(Actor::Owner),
                        &ApprovalDigest::default(),
                        &owner_holds(),
                    )
                    .unwrap();
                assert_eq!(decision.target, Mode::Off);
            }
            "lower" => {
                store
                    .lower_modes(
                        "notices",
                        &BTreeMap::from([(ActionGroup::Mailbox, Mode::Shadow)]),
                        None,
                        &owner_holds(),
                        ApprovalReason::Brake,
                    )
                    .unwrap();
            }
            "detect" => {
                assert!(store
                    .detect_lapses("notices", &owner_holds(), Utc::now())
                    .is_err());
            }
            "raise" => {
                assert!(store
                    .change_mode(
                        "notices",
                        ActionGroup::Mailbox,
                        Mode::Auto,
                        Some(Actor::Owner),
                        &rule.approval_digest,
                        &owner_holds()
                    )
                    .is_err());
            }
            _ => unreachable!(),
        }
        fs::write(&path, archive()).unwrap();
        let restored = store.get("notices").unwrap();
        let mode = resolve_mode(
            &restored,
            ActionGroup::Mailbox,
            &store.approvals().unwrap(),
            &owner_holds(),
        );
        assert_eq!(
            mode.effective,
            if operation == "off" {
                Mode::Off
            } else {
                Mode::Shadow
            }
        );
        // A fresh, shown approval remains possible after the inputs recover.
        approve(&store, &restored, ActionGroup::Mailbox, Mode::Auto);
        assert_eq!(
            resolve_mode(
                &restored,
                ActionGroup::Mailbox,
                &store.approvals().unwrap(),
                &owner_holds()
            )
            .effective,
            Mode::Auto
        );
    }
}

#[test]
fn owner_off_survives_an_unreadable_definition() {
    broken_definition_lowers("off");
}

#[test]
fn brakes_survive_an_unreadable_definition() {
    broken_definition_lowers("lower");
}

#[test]
fn observed_load_failure_durably_lapses_approval() {
    broken_definition_lowers("detect");
}

#[test]
fn failed_raise_durably_lapses_unreadable_inputs() {
    broken_definition_lowers("raise");
}

#[test]
fn lowering_ignores_stale_review_but_raising_requires_it() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    fs::write(rule.directory.join("extra.md"), "Fictional edit").unwrap();
    let decision = store
        .change_mode(
            "notices",
            ActionGroup::Mailbox,
            Mode::Off,
            Some(Actor::Owner),
            &rule.approval_digest,
            &owner_holds(),
        )
        .unwrap();
    assert_eq!(decision.target, Mode::Off);
    assert!(store
        .change_mode(
            "notices",
            ActionGroup::Mailbox,
            Mode::Auto,
            Some(Actor::Owner),
            &rule.approval_digest,
            &owner_holds()
        )
        .is_err());
}

#[test]
fn deferred_settlement_preserves_concurrently_enqueued_snapshots() {
    let home = Home::new();
    let worker = home.store();
    let producer = home.store();
    let rule = install(&worker, &text("[[action]]\ndo = 'task'\n"));
    let snapshot = row(&rule, Outcome::WouldHave);
    let entry = DeferredEntry {
        id: "queued-alpha".into(),
        account: snapshot.message.account.clone(),
        thread_id: snapshot.message.thread_id.clone(),
        queued_at: Utc::now(),
        actions: vec![snapshot.clone()],
    };
    worker.enqueue(&entry).unwrap();
    let observed = worker.deferred().unwrap().remove(0);
    let pending = row(&rule, Outcome::Pending);
    assert_eq!(
        worker
            .claim_actions(std::slice::from_ref(&pending))
            .unwrap()[0]
            .status,
        ClaimStatus::Claimed
    );
    let completed = pending
        .settled(Outcome::Done, "fictional task created", Utc::now())
        .unwrap();
    let mut concurrent = entry.clone();
    concurrent.id = "queued-beta".into();
    concurrent.actions[0].id = "snapshot-beta".into();
    concurrent.actions[0].message.provider_message_id = "provider-beta".into();
    producer.enqueue(&concurrent).unwrap();
    worker
        .settle_deferred(&observed.id, std::slice::from_ref(&completed))
        .unwrap();
    let rows = worker.ledger().unwrap();
    assert!(rows.iter().any(|row| row.id == completed.id
        && row.outcome == Outcome::Done
        && row.settlement.as_deref() == Some(&pending.id)));
    let remaining = worker.deferred().unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, entry.id);
    assert_eq!(remaining[0].queued_at, entry.queued_at);
    assert_eq!(remaining[0].actions.len(), 1);
    assert_eq!(remaining[0].actions[0].id, concurrent.actions[0].id);
    let mut next = concurrent.actions[0].clone();
    next.id = "completed-beta".into();
    next.outcome = Outcome::Skipped;
    worker.settle_deferred(&entry.id, &[next]).unwrap();
    assert!(worker.deferred().unwrap().is_empty());
}

#[test]
fn load_failure_preserves_off_and_clamps_lowering_to_its_durable_ceiling() {
    let home = Home::new();
    let store = home.store();
    let definition = text("[[action]]\ndo = 'archive'\n[[action]]\ndo = 'digest'\n");
    let rule = install(&store, &definition);
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    approve(&store, &rule, ActionGroup::PrivateRecord, Mode::Off);
    let before = store.approvals().unwrap();
    fs::remove_file(rule.directory.join("rule.toml")).unwrap();
    let decision = store
        .change_mode(
            "notices",
            ActionGroup::Mailbox,
            Mode::Propose,
            None,
            &ApprovalDigest::default(),
            &owner_holds(),
        )
        .unwrap();
    assert_eq!(decision.requested, Mode::Propose);
    assert_eq!(decision.target, Mode::Shadow);
    assert!(decision.clamped);
    let decision = store
        .change_mode(
            "notices",
            ActionGroup::Mailbox,
            Mode::Off,
            Some(Actor::OwnerApproved),
            &ApprovalDigest::default(),
            &owner_holds(),
        )
        .unwrap();
    assert_eq!(decision.target, Mode::Shadow);
    assert!(decision.clamped);
    assert!(store
        .detect_lapses("notices", &owner_holds(), Utc::now())
        .is_err());
    let after = store.approvals().unwrap();
    assert_eq!(
        after
            .iter()
            .filter(|row| row.group == ActionGroup::PrivateRecord)
            .count(),
        before
            .iter()
            .filter(|row| row.group == ActionGroup::PrivateRecord)
            .count()
    );
    assert!(after
        .iter()
        .filter(|row| row.to_mode == Mode::Off)
        .all(|row| row.actor == Actor::Owner));
    let notices = store.ledger().unwrap();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].affected_groups, vec![ActionGroup::Mailbox]);
    assert_eq!(notices[0].reason, "rule inputs unreadable");
    fs::write(rule.directory.join("rule.toml"), definition).unwrap();
    assert_eq!(
        resolve_mode(
            &store.get("notices").unwrap(),
            ActionGroup::Mailbox,
            &after,
            &owner_holds()
        )
        .effective,
        Mode::Shadow
    );
    assert_eq!(
        resolve_mode(
            &store.get("notices").unwrap(),
            ActionGroup::PrivateRecord,
            &after,
            &owner_holds()
        )
        .effective,
        Mode::Off
    );
}

#[test]
fn deferred_settlement_retries_exact_rows_and_removes_only_matching_snapshots() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let pending = row(&rule, Outcome::Pending);
    let snapshot = row(&rule, Outcome::WouldHave);
    let mut concurrent = snapshot.clone();
    // Snapshot ids are caller supplied: equal ids must not erase distinct keys.
    concurrent.message.provider_message_id = "provider-beta".into();
    let entry = DeferredEntry {
        id: "queued-alpha".into(),
        account: snapshot.message.account.clone(),
        thread_id: snapshot.message.thread_id.clone(),
        queued_at: Utc::now(),
        actions: vec![snapshot, concurrent.clone()],
    };
    store.enqueue(&entry).unwrap();
    assert_eq!(
        store.claim_actions(std::slice::from_ref(&pending)).unwrap()[0].status,
        ClaimStatus::Claimed
    );
    let done = pending
        .settled(Outcome::Done, "fictional completion", Utc::now())
        .unwrap();
    let mut foreign = done.clone();
    foreign.id = "foreign-outcome".into();
    foreign.message.provider_message_id = "provider-foreign".into();
    assert!(store.settle_deferred(&entry.id, &[foreign]).is_err());
    assert!(store
        .settle_deferred(&entry.id, &[done.clone(), done.clone()])
        .is_err());
    assert_eq!(store.ledger().unwrap().len(), 1);
    // Simulate a stop after the append but before the queue rewrite.
    store.append_ledger(&done).unwrap();
    let mut mismatched = done.clone();
    mismatched.outcome = Outcome::Failed;
    assert!(store.settle_deferred(&entry.id, &[mismatched]).is_err());
    assert_eq!(store.deferred().unwrap()[0].actions.len(), 2);
    store
        .settle_deferred(&entry.id, std::slice::from_ref(&done))
        .unwrap();
    assert_eq!(
        store
            .ledger()
            .unwrap()
            .iter()
            .filter(|row| row.id == done.id)
            .count(),
        1
    );
    let remaining = store.deferred().unwrap();
    assert_eq!(remaining[0].actions.len(), 1);
    assert_eq!(remaining[0].actions[0].message, concurrent.message);
}

#[test]
fn permanent_ceilings_refuse_raises_without_recording_permission() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &text("[[action]]\ndo = 'spam'\n"));
    approve(&store, &rule, ActionGroup::Report, Mode::Propose);
    let before = fs::read(store.root().join("approvals.jsonl")).unwrap();
    let decision = store
        .change_mode(
            "notices",
            ActionGroup::Report,
            Mode::Auto,
            Some(Actor::Owner),
            &rule.approval_digest,
            &owner_holds(),
        )
        .unwrap();
    assert!(!decision.write, "a permanent ceiling must refuse the raise");
    assert!(decision.refused);
    assert!(!decision.clamped);
    assert_eq!(decision.requested, Mode::Auto);
    assert!(decision
        .holds
        .iter()
        .any(|hold| hold.ceiling == Mode::Propose && hold.reason.contains("spam")));
    assert_eq!(decision.target, Mode::Propose);
    assert_eq!(
        fs::read(store.root().join("approvals.jsonl")).unwrap(),
        before
    );
}

#[test]
fn permanent_ceiling_refusal_preserves_off_and_shadow_without_partial_raises() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &text("[[action]]\ndo = 'spam'\n"));
    for current in [Mode::Shadow, Mode::Off] {
        approve(&store, &rule, ActionGroup::Report, current);
        let before = fs::read(store.root().join("approvals.jsonl")).unwrap();
        let decision = store
            .change_mode(
                "notices",
                ActionGroup::Report,
                Mode::Auto,
                Some(Actor::Owner),
                &rule.approval_digest,
                &owner_holds(),
            )
            .unwrap();
        assert!(decision.refused);
        assert!(!decision.write);
        assert!(!decision.clamped);
        assert_eq!(decision.target, current);
        assert_eq!(
            fs::read(store.root().join("approvals.jsonl")).unwrap(),
            before
        );
    }
    approve(&store, &rule, ActionGroup::Report, Mode::Propose);
}

#[test]
fn computed_holds_are_reported_without_becoming_permanent_approval_ceilings() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &text("[[action]]\ndo = 'task'\n"));
    let decision = store
        .change_mode(
            "notices",
            ActionGroup::Board,
            Mode::Auto,
            Some(Actor::Owner),
            &rule.approval_digest,
            &P1Holds::default(),
        )
        .unwrap();
    assert!(!decision.refused);
    assert!(decision.write);
    assert!(decision
        .holds
        .iter()
        .any(|hold| hold.ceiling == Mode::Propose && !hold.reason.is_empty()));
    let rows = store.approvals().unwrap();
    assert_eq!(
        resolve_mode(&rule, ActionGroup::Board, &rows, &P1Holds::default()).effective,
        Mode::Propose
    );
    assert_eq!(
        resolve_mode(&rule, ActionGroup::Board, &rows, &owner_holds()).effective,
        Mode::Auto
    );
}

#[test]
fn unknown_failure_kinds_count_toward_the_consecutive_failure_brake() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    for kind in [
        None,
        Some(serde_json::json!("future_failure")),
        Some(serde_json::json!(false)),
        Some(serde_json::json!({})),
    ] {
        let mut value = serde_json::to_value(row(&rule, Outcome::Failed)).unwrap();
        value.as_object_mut().unwrap().remove("failure_kind");
        if let Some(kind) = kind {
            value["failure_kind"] = kind;
        }
        let loaded: LedgerRow = serde_json::from_value(value).unwrap();
        assert_eq!(loaded.failure_kind, FailureKind::Unknown);
        assert!(loaded.is_action_failure());
        let evidence = BrakeEvidence {
            consecutive_action_failures: BTreeMap::from([(
                ActionGroup::Mailbox,
                (0..3).filter(|_| loaded.is_action_failure()).count(),
            )]),
            ..Default::default()
        };
        assert_eq!(
            brake_targets([ActionGroup::Mailbox], &evidence).get(&ActionGroup::Mailbox),
            Some(&Mode::Shadow)
        );
    }
    for kind in [
        FailureKind::Action,
        FailureKind::Unknown,
        FailureKind::Environment,
        FailureKind::Interrupted,
    ] {
        let mut failed = row(&rule, Outcome::Failed);
        failed.failure_kind = kind;
        assert_eq!(
            failed.is_action_failure(),
            matches!(kind, FailureKind::Action | FailureKind::Unknown)
        );
        for outcome in [
            Outcome::Done,
            Outcome::Expired,
            Outcome::Pending,
            Outcome::Unknown,
        ] {
            failed.outcome = outcome;
            assert!(!failed.is_action_failure());
        }
    }
}

#[test]
fn a_settled_failure_without_an_explicit_kind_counts_toward_the_brake() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let failed = row(&rule, Outcome::Pending)
        .settled(Outcome::Failed, "fictional action failure", Utc::now())
        .unwrap();
    assert!(failed.is_action_failure());
}

#[test]
fn enqueue_returns_the_effective_entry_id_for_settlement() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let snapshot = row(&rule, Outcome::WouldHave);
    let entry = DeferredEntry {
        id: "queued-alpha".into(),
        account: snapshot.message.account.clone(),
        thread_id: snapshot.message.thread_id.clone(),
        queued_at: Utc::now(),
        actions: vec![snapshot.clone()],
    };
    let first = store.enqueue(&entry).unwrap();
    assert_eq!(first, entry.id);
    let mut merged = entry.clone();
    merged.id = "queued-beta".into();
    merged.actions[0].id = "snapshot-beta".into();
    merged.actions[0].message.provider_message_id = "provider-beta".into();
    let effective = store.enqueue(&merged).unwrap();
    assert_eq!(effective, first);
    let mut completed = merged.actions[0].clone();
    completed.id = "completed-beta".into();
    completed.outcome = Outcome::Done;
    store.settle_deferred(&effective, &[completed]).unwrap();
    let remaining = store.deferred().unwrap();
    assert_eq!(remaining[0].id, entry.id);
    assert_eq!(remaining[0].actions.len(), 1);
    assert_eq!(remaining[0].actions[0].id, snapshot.id);
}

#[test]
fn closed_sibling_does_not_block_an_interrupted_action_retry() {
    for retry_verb in [ActionVerb::Archive, ActionVerb::Tag] {
        for reverse in [false, true] {
            let home = Home::new();
            let store = home.store();
            let rule = install(
                &store,
                &text("[[action]]\ndo='archive'\n[[action]]\ndo='tag'\ntag='fictional'\n"),
            );
            let now = Utc::now();
            let mut pending: Vec<_> = (0..rule.actions.len())
                .map(|index| {
                    LedgerRow::header(
                        &rule,
                        index,
                        message(),
                        Mode::Auto,
                        GateStatus::Passing,
                        Outcome::Pending,
                        now,
                    )
                    .unwrap()
                })
                .collect();
            if reverse {
                pending.reverse();
            }
            store.claim_actions(&pending).unwrap();
            for action in &pending {
                let settlement = if action.action.verb == retry_verb {
                    action
                        .interrupted(now + Duration::minutes(2), Duration::minutes(1))
                        .unwrap()
                        .unwrap()
                } else {
                    action
                        .settled(Outcome::Done, "fictional action completed", now)
                        .unwrap()
                };
                store.append_ledger(&settlement).unwrap();
            }
            let retry: Vec<_> = pending
                .iter()
                .map(|action| {
                    let mut next = action.clone();
                    next.id = uuid::Uuid::new_v4().to_string();
                    next
                })
                .collect();
            let claims = store.claim_actions(&retry).unwrap();
            assert_eq!(claims.len(), retry.len());
            for (action, claim) in retry.iter().zip(&claims) {
                assert_eq!(claim.pending_id, action.id);
                assert_eq!(claim.action, action.action);
                assert_eq!(
                    claim.status,
                    if action.action.verb == retry_verb {
                        ClaimStatus::Claimed
                    } else {
                        ClaimStatus::MessageClosed
                    }
                );
            }
            let rows = store.ledger().unwrap();
            for action in &retry {
                assert_eq!(
                    rows.iter().any(|row| row.id == action.id),
                    action.action.verb == retry_verb,
                    "only the interrupted action's open key should be claimed"
                );
            }
        }
    }
}

#[test]
fn closed_thread_skips_outbound_and_board_but_not_open_mailbox_actions() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &text("[[action]]\ndo='archive'\n[[action]]\ndo='tag'\ntag='fictional'\n[[action]]\ndo='task'\n[[action]]\ndo='forward'\nto=['records@example.edu']\n"));
    let now = Utc::now();
    let pending: Vec<_> = (0..rule.actions.len())
        .map(|index| {
            LedgerRow::header(
                &rule,
                index,
                message(),
                Mode::Auto,
                GateStatus::Passing,
                Outcome::Pending,
                now,
            )
            .unwrap()
        })
        .collect();
    let claims = store.claim_actions(&pending).unwrap();
    assert_eq!(claims.len(), pending.len());
    assert!(claims
        .iter()
        .all(|claim| claim.status == ClaimStatus::Claimed));
    for action in &pending {
        let settlement = if action.action.verb == ActionVerb::Archive {
            action
                .interrupted(now + Duration::minutes(2), Duration::minutes(1))
                .unwrap()
                .unwrap()
        } else {
            action
                .settled(Outcome::Done, "fictional action completed", now)
                .unwrap()
        };
        store.append_ledger(&settlement).unwrap();
    }
    for provider_message_id in ["provider-alpha", "provider-beta"] {
        let next: Vec<_> = pending
            .iter()
            .map(|action| {
                let mut next = action.clone();
                next.id = uuid::Uuid::new_v4().to_string();
                next.message.provider_message_id = provider_message_id.into();
                next
            })
            .collect();
        let claims = store.claim_actions(&next).unwrap();
        assert_eq!(claims.len(), next.len());
        for (action, claim) in next.iter().zip(&claims) {
            assert_eq!(claim.pending_id, action.id);
            assert_eq!(claim.action, action.action);
            let expected = if action.group.once_per_thread() {
                ClaimStatus::ThreadClosed
            } else if action.action.verb == ActionVerb::Tag
                && provider_message_id == "provider-alpha"
            {
                ClaimStatus::MessageClosed
            } else {
                ClaimStatus::Claimed
            };
            assert_eq!(claim.status, expected);
        }
        let rows = store.ledger().unwrap();
        for action in &next {
            let open = action.action.verb == ActionVerb::Archive
                || (action.action.verb == ActionVerb::Tag
                    && provider_message_id == "provider-beta");
            assert_eq!(rows.iter().any(|row| row.id == action.id), open);
        }
    }
}

#[test]
fn store_and_installed_rule_directories_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new();
    let root = RuleStore::root_under(&home.0);
    // Simulate an existing store created under a permissive umask without
    // changing the process-wide umask used by concurrent tests.
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    let store = home.store();
    let rule = install(&store, &archive());
    for directory in [store.root(), &store.root().join("rules"), &rule.directory] {
        assert_eq!(
            fs::metadata(directory).unwrap().permissions().mode() & 0o777,
            0o700,
            "directory must be private: {}",
            directory.display()
        );
    }
    // Opening an existing rules directory must also repair its permissions.
    fs::set_permissions(
        store.root().join("rules"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let reopened = home.store();
    assert_eq!(
        fs::metadata(reopened.root().join("rules"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

#[test]
fn enqueue_refuses_execution_rows_without_changing_the_queue() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let snapshot = row(&rule, Outcome::WouldHave);
    let entry = DeferredEntry {
        id: "queued-alpha".into(),
        account: snapshot.message.account.clone(),
        thread_id: snapshot.message.thread_id.clone(),
        queued_at: Utc::now(),
        actions: vec![snapshot.clone()],
    };
    store.enqueue(&entry).unwrap();
    let before = fs::read(store.root().join("deferred.jsonl")).unwrap();
    for outcome in [
        Outcome::Pending,
        Outcome::Done,
        Outcome::Failed,
        Outcome::Held,
        Outcome::Staged,
        Outcome::Sent,
        Outcome::Offered,
        Outcome::Undone,
        Outcome::Edited,
        Outcome::Rejected,
        Outcome::ReleasedUnchanged,
        Outcome::Expired,
        Outcome::Skipped,
        Outcome::ApprovalLapsed,
        Outcome::Unknown,
    ] {
        let mut invalid = entry.clone();
        invalid.id = "queued-beta".into();
        let mut action = row(&rule, outcome);
        action.message.provider_message_id = "provider-beta".into();
        invalid.actions = vec![action];
        assert!(
            store.enqueue(&invalid).is_err(),
            "execution outcome {outcome:?} must be refused"
        );
        assert_eq!(
            fs::read(store.root().join("deferred.jsonl")).unwrap(),
            before
        );
    }
    for closed in [true, false] {
        let mut invalid = entry.clone();
        invalid.actions[0].closes = closed;
        if !closed {
            invalid.actions[0].settlement = Some("pending-alpha".into());
        }
        assert!(store.enqueue(&invalid).is_err());
        assert_eq!(
            fs::read(store.root().join("deferred.jsonl")).unwrap(),
            before
        );
    }
    store
        .expire_deferred(entry.queued_at + Duration::days(8))
        .unwrap();
    assert!(store.deferred().unwrap().is_empty());
    let rows = store.ledger().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, Outcome::Expired);
    assert!(message_closed(&rows, &key(&snapshot)));
}

#[test]
fn opening_a_store_does_not_chmod_an_escaping_rules_symlink() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let home = Home::new();
    let outside = Home::new();
    let store = home.store();
    fs::remove_dir(store.root().join("rules")).unwrap();
    fs::set_permissions(&outside.0, fs::Permissions::from_mode(0o755)).unwrap();
    symlink(&outside.0, store.root().join("rules")).unwrap();
    assert!(RuleStore::open(store.root()).is_err());
    assert_eq!(
        fs::metadata(&outside.0).unwrap().permissions().mode() & 0o777,
        0o755
    );
}

#[test]
fn empty_header_or_is_rejected() {
    let rule =
        "description = 'Notice'\n[match.header]\nX-Notice = []\n[[action]]\ndo = 'archive'\n";
    assert!(
        RuleDefinition::parse(rule).is_err(),
        "an empty header OR must not install an inert rule"
    );
}

#[test]
fn owner_path_install_reviews_all_bytes_and_rejects_stale_approval() {
    let home = Home::new();
    let store = home.store();
    let source = home.0.join("notice-source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("rule.toml"), archive()).unwrap();
    fs::write(source.join("extra.md"), "An owner-written explanation.").unwrap();
    let rule = install_path(&store, &source.join("rule.toml"), Some("notice")).unwrap();
    let review = store.review_inputs("notice").unwrap();
    assert_eq!(
        review.files["rule/extra.md"],
        "An owner-written explanation."
    );
    assert_eq!(review.digest, rule.approval_digest);
    assert_eq!(
        rule_views(&store).unwrap()[0].groups[0].effective_mode,
        Mode::Shadow
    );
    fs::write(rule.directory.join("extra.md"), "Changed.").unwrap();
    assert!(store
        .change_mode(
            "notice",
            ActionGroup::Mailbox,
            Mode::Auto,
            Some(Actor::Owner),
            &review.digest,
            &owner_holds()
        )
        .is_err());
}

#[test]
fn pause_all_preserves_off_and_revokes_an_unreadable_definition() {
    let home = Home::new();
    let store = home.store();
    let first = store
        .install("first", &archive(), &BTreeMap::new())
        .unwrap();
    let second = store
        .install("second", &archive(), &BTreeMap::new())
        .unwrap();
    approve(&store, &first, ActionGroup::Mailbox, Mode::Off);
    approve(&store, &second, ActionGroup::Mailbox, Mode::Auto);
    fs::write(second.directory.join("rule.toml"), "broken").unwrap();
    pause_all(&store, Actor::Unknown).unwrap();
    let views = rule_views(&store).unwrap();
    assert_eq!(views[0].groups[0].recorded_mode, Mode::Off);
    assert_eq!(views[1].groups[0].recorded_mode, Mode::Shadow);
    assert!(views[1].load_error.is_some());
    assert!(!store
        .approvals()
        .unwrap()
        .iter()
        .any(|r| r.actor != Actor::Owner && r.to_mode == Mode::Off));
}

#[test]
fn ledger_view_keeps_unknown_outcomes_and_orders_newest_first() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let older = row(&rule, Outcome::Done);
    store.append_ledger(&older).unwrap();
    let mut future = row(&rule, Outcome::Done);
    future.at = older.at.map(|t| t + Duration::hours(1));
    let mut value = serde_json::to_value(&future).unwrap();
    value["outcome"] = serde_json::json!("future-outcome");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(store.root().join("ledger.jsonl"))
        .unwrap();
    use std::io::Write;
    writeln!(file, "{value}").unwrap();
    let rows = recent_ledger(&store, Some(&rule.name), 1).unwrap();
    assert_eq!(rows[0].id, future.id);
    assert_eq!(rows[0].outcome, Outcome::Unknown);
}

#[derive(Default)]
struct RestoreFixture(std::sync::Mutex<Vec<(String, RestoreAction)>>);
#[async_trait::async_trait]
impl Restorer for RestoreFixture {
    async fn restore(&self, message: &MessageRef, action: RestoreAction) -> anyhow::Result<()> {
        self.0
            .lock()
            .unwrap()
            .push((message.provider_message_id.clone(), action));
        Ok(())
    }
}

#[tokio::test]
async fn review_undo_unreadable_definition_clamps_every_group_before_restore() {
    struct Inspect<'a> {
        store: &'a RuleStore,
        rule: &'a LoadedRule,
    }
    #[async_trait::async_trait]
    impl Restorer for Inspect<'_> {
        async fn restore(&self, _: &MessageRef, _: RestoreAction) -> anyhow::Result<()> {
            let approvals = self.store.approvals().unwrap();
            for group in [ActionGroup::Mailbox, ActionGroup::Board] {
                assert_eq!(
                    resolve_mode(self.rule, group, &approvals, &owner_holds()).recorded,
                    Mode::Shadow
                );
            }
            Ok(())
        }
    }
    let home = Home::new();
    let store = home.store();
    let definition = text("[[action]]\ndo='archive'\n[[action]]\ndo='task'\n");
    let rule = install(&store, &definition);
    for group in [ActionGroup::Mailbox, ActionGroup::Board] {
        approve(&store, &rule, group, Mode::Auto);
    }
    let original = row(&rule, Outcome::Done);
    store.append_ledger(&original).unwrap();
    fs::write(rule.directory.join("rule.toml"), "broken definition").unwrap();
    let result = undo(
        &store,
        &original.id,
        &Inspect {
            store: &store,
            rule: &rule,
        },
        Utc::now(),
    )
    .await
    .unwrap();
    assert_eq!(result.outcome, Outcome::Undone);
    fs::write(rule.directory.join("rule.toml"), definition).unwrap();
    let restored = store.get(&rule.name).unwrap();
    for group in [ActionGroup::Mailbox, ActionGroup::Board] {
        assert_eq!(
            resolve_mode(
                &restored,
                group,
                &store.approvals().unwrap(),
                &owner_holds()
            )
            .effective,
            Mode::Shadow
        );
    }
}

#[tokio::test]
async fn failed_undo_intent_append_never_reaches_the_provider() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let original = row(&rule, Outcome::Done);
    store.append_ledger(&original).unwrap();
    store
        .fail_next_ledger_append
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let restore = RestoreFixture::default();
    assert!(undo(&store, &original.id, &restore, Utc::now())
        .await
        .is_err());
    assert!(restore.0.lock().unwrap().is_empty());
    assert_eq!(store.ledger().unwrap().len(), 1);
    undo(&store, &original.id, &restore, Utc::now())
        .await
        .unwrap();
    assert_eq!(restore.0.lock().unwrap().len(), 1);
}

#[test]
fn undo_intents_have_separate_recovery_and_degrade_for_old_readers() {
    #[derive(Debug, PartialEq, serde::Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum LegacyOutcome {
        Pending,
        Done,
        Undone,
        Failed,
        #[serde(other)]
        Unknown,
    }
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let original = row(&rule, Outcome::Done);
    store.append_ledger(&original).unwrap();
    let mut intent = original.clone();
    intent.id = uuid::Uuid::new_v4().to_string();
    intent.outcome = Outcome::UndoPending;
    intent.correction_of = Some(original.id.clone());
    store.append_ledger(&intent).unwrap();
    let legacy: LegacyOutcome =
        serde_json::from_value(serde_json::to_value(intent.outcome).unwrap()).unwrap();
    assert_eq!(legacy, LegacyOutcome::Unknown);
    assert!(intent
        .interrupted(Utc::now() + Duration::days(10), Duration::minutes(1))
        .unwrap()
        .is_none());
    assert!(intent
        .settled(Outcome::Done, "not an action attempt", Utc::now())
        .is_err());
    let mut duplicate = intent.clone();
    duplicate.id = uuid::Uuid::new_v4().to_string();
    assert!(store.append_ledger(&duplicate).is_err());
    let mut wrong = intent
        .settled(Outcome::Failed, "failed restore", Utc::now())
        .unwrap();
    wrong.correction_of = Some("different-original".into());
    assert!(store.append_ledger(&wrong).is_err());
    assert_eq!(
        engine::brake_evidence(&rule.name, &store.ledger().unwrap()).undone_mailbox,
        1
    );
}

#[tokio::test]
async fn undo_recovers_provider_success_after_settlement_append_failure() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct CrashAfterRestore<'a> {
        store: &'a RuleStore,
        calls: AtomicUsize,
    }
    #[async_trait::async_trait]
    impl Restorer for CrashAfterRestore<'_> {
        async fn restore(&self, _: &MessageRef, _: RestoreAction) -> anyhow::Result<()> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                self.store
                    .fail_next_ledger_append
                    .store(true, Ordering::SeqCst);
            }
            Ok(())
        }
    }
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let original = row(&rule, Outcome::Done);
    store.append_ledger(&original).unwrap();
    let restore = CrashAfterRestore {
        store: &store,
        calls: AtomicUsize::new(0),
    };
    let error = undo(&store, &original.id, &restore, Utc::now())
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("injected ledger append failure"));
    let rows = store.ledger().unwrap();
    let intent = rows
        .iter()
        .find(|r| r.correction_of.as_deref() == Some(&original.id))
        .expect("durable undo intent must survive provider success and settlement failure");
    assert_eq!(
        serde_json::to_value(intent.outcome).unwrap(),
        "undo_pending"
    );
    assert_eq!(engine::brake_evidence(&rule.name, &rows).undone_mailbox, 1);
    assert!(message_closed(&rows, &key(&original)));
    let settled = undo(&store, &original.id, &restore, Utc::now())
        .await
        .unwrap();
    assert_eq!(settled.outcome, Outcome::Undone);
    assert_eq!(settled.settlement.as_deref(), Some(intent.id.as_str()));
    assert_eq!(restore.calls.load(Ordering::SeqCst), 2);
    let rows = store.ledger().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(engine::brake_evidence(&rule.name, &rows).undone_mailbox, 1);
    assert!(undo(&store, &original.id, &restore, Utc::now())
        .await
        .is_err());
    assert_eq!(restore.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn pending_undo_counts_and_applies_brake_before_provider_io() {
    struct Inspect<'a> {
        store: &'a RuleStore,
        rule: &'a LoadedRule,
        original: &'a LedgerRow,
    }
    #[async_trait::async_trait]
    impl Restorer for Inspect<'_> {
        async fn restore(&self, _: &MessageRef, _: RestoreAction) -> anyhow::Result<()> {
            let rows = self.store.ledger().unwrap();
            assert_eq!(
                engine::brake_evidence(&self.rule.name, &rows).undone_mailbox,
                3
            );
            assert_eq!(
                resolve_mode(
                    self.rule,
                    ActionGroup::Mailbox,
                    &self.store.approvals().unwrap(),
                    &owner_holds()
                )
                .effective,
                Mode::Propose
            );
            let other = RestoreFixture::default();
            assert!(undo(self.store, &self.original.id, &other, Utc::now())
                .await
                .is_err());
            assert!(other.0.lock().unwrap().is_empty());
            Ok(())
        }
    }
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    let mut originals = Vec::new();
    for index in 0..10 {
        let mut original = row(&rule, Outcome::Done);
        original.message.provider_message_id = format!("immutable-{index}");
        store.append_ledger(&original).unwrap();
        originals.push(original);
    }
    for original in originals.iter().take(2) {
        let mut correction = original.clone();
        correction.id = uuid::Uuid::new_v4().to_string();
        correction.outcome = Outcome::Undone;
        correction.correction_of = Some(original.id.clone());
        store.append_ledger(&correction).unwrap();
    }
    let original = &originals[2];
    undo(
        &store,
        &original.id,
        &Inspect {
            store: &store,
            rule: &rule,
            original,
        },
        Utc::now(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn undo_restores_exact_message_closes_key_and_counts_toward_brake() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    let restorer = RestoreFixture::default();
    let mut done = Vec::new();
    for index in 0..10 {
        let mut entry = row(&rule, Outcome::Done);
        entry.message.provider_message_id = format!("immutable-message-{index}");
        store.append_ledger(&entry).unwrap();
        done.push(entry);
    }
    for original in done.iter().take(3) {
        let correction = undo(&store, &original.id, &restorer, Utc::now())
            .await
            .unwrap();
        assert_eq!(
            correction.correction_of.as_deref(),
            Some(original.id.as_str())
        );
        assert_eq!(correction.outcome, Outcome::Undone);
        assert!(message_closed(&store.ledger().unwrap(), &key(original)));
        assert!(undo(&store, &original.id, &restorer, Utc::now())
            .await
            .is_err());
    }
    assert_eq!(restorer.0.lock().unwrap().len(), 3);
    assert_eq!(
        restorer.0.lock().unwrap()[0],
        (
            done[0].message.provider_message_id.clone(),
            RestoreAction::Unarchive
        )
    );
    assert_eq!(
        engine::brake_evidence(&rule.name, &store.ledger().unwrap()).undone_mailbox,
        3
    );
    assert_eq!(
        resolve_mode(
            &rule,
            ActionGroup::Mailbox,
            &store.approvals().unwrap(),
            &owner_holds()
        )
        .effective,
        Mode::Propose
    );
}

#[tokio::test]
async fn undo_refuses_noncompleted_and_nonrestorable_rows_before_provider_io() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let restorer = RestoreFixture::default();
    for outcome in [
        Outcome::Unknown,
        Outcome::Pending,
        Outcome::WouldHave,
        Outcome::Offered,
        Outcome::Failed,
    ] {
        let entry = row(&rule, outcome);
        store.append_ledger(&entry).unwrap();
        assert!(undo(&store, &entry.id, &restorer, Utc::now())
            .await
            .is_err());
    }
    for (verb, group) in [
        (ActionVerb::Spam, ActionGroup::Report),
        (ActionVerb::Task, ActionGroup::Board),
        (ActionVerb::Send, ActionGroup::TypedFields),
        (ActionVerb::Tag, ActionGroup::PrivateRecord),
    ] {
        let mut entry = row(&rule, Outcome::Done);
        entry.action.verb = verb;
        entry.group = group;
        store.append_ledger(&entry).unwrap();
        assert!(undo(&store, &entry.id, &restorer, Utc::now())
            .await
            .is_err());
    }
    assert!(restorer.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn failed_restore_settles_intent_without_counting_an_undo_or_action_failure() {
    struct Unavailable;
    #[async_trait::async_trait]
    impl Restorer for Unavailable {
        async fn restore(&self, _: &MessageRef, _: RestoreAction) -> anyhow::Result<()> {
            anyhow::bail!("provider unavailable")
        }
    }
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let original = row(&rule, Outcome::Done);
    store.append_ledger(&original).unwrap();
    assert!(undo(&store, &original.id, &Unavailable, Utc::now())
        .await
        .is_err());
    let rows = store.ledger().unwrap();
    assert_eq!(
        rows.len(),
        3,
        "intent and failed settlement must both be recorded"
    );
    assert_eq!(rows[2].outcome, Outcome::Failed);
    assert_eq!(rows[2].settlement.as_deref(), Some(rows[1].id.as_str()));
    assert_eq!(rows[2].correction_of.as_deref(), Some(original.id.as_str()));
    let evidence = engine::brake_evidence(&rule.name, &rows);
    assert_eq!(evidence.undone_mailbox, 0);
    assert_eq!(
        evidence.consecutive_action_failures[&ActionGroup::Mailbox],
        0
    );
    assert!(undo(&store, &original.id, &Unavailable, Utc::now())
        .await
        .is_err());
    assert_eq!(store.ledger().unwrap().len(), 3);
}

#[test]
fn rule_view_explains_lapse_unknown_modes_and_account_holds_without_writes() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &text("[[action]]\ndo = 'archive'\n"));
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    fs::write(rule.directory.join("extra.md"), "edited").unwrap();
    let before = fs::read(store.root().join("approvals.jsonl")).unwrap();
    let view = rule_views(&store).unwrap();
    assert_eq!(view[0].groups[0].recorded_mode, Mode::Auto);
    assert_eq!(view[0].groups[0].effective_mode, Mode::Propose);
    assert!(view[0].groups[0].holds.iter().any(|h| h.contains("lapsed")));
    assert_eq!(
        before,
        fs::read(store.root().join("approvals.jsonl")).unwrap()
    );
    let raw = String::from_utf8(before)
        .unwrap()
        .replace("\"to_mode\":\"auto\"", "\"to_mode\":\"future\"");
    fs::write(store.root().join("approvals.jsonl"), raw).unwrap();
    let view = rule_views(&store).unwrap();
    assert_eq!(view[0].groups[0].recorded_mode, Mode::Unknown);
    assert_eq!(view[0].groups[0].effective_mode, Mode::Shadow);
    store.install("headers", "description = 'Fictional list'\n[match]\nlist_id = ['notice.example.com']\n[[action]]\ndo = 'forward'\nto = ['finance@example.edu']\n", &BTreeMap::new()).unwrap();
    let mut views = rule_views(&store).unwrap();
    account_holds(
        &store,
        &mut views,
        &[RuleAccount {
            name: "campus".into(),
            proves_header_absence: false,
        }],
    )
    .unwrap();
    assert!(views
        .iter()
        .find(|v| v.name == "headers")
        .unwrap()
        .holds
        .iter()
        .any(|h| h.contains("campus") && h.contains("provider")));
}

#[test]
fn mail_rules_doctor_reports_lapses_without_writing_and_keeps_persisted_lapses_visible() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    fs::write(
        rule.directory.join("rule.toml"),
        archive().replace("Fictional notice", "Changed fictional notice"),
    )
    .unwrap();
    let before = fs::read(store.root().join("approvals.jsonl")).unwrap();
    let findings = crate::doctor::check_mail_rules(
        &home.0,
        &crate::doctor::MailStores::under(&home.0),
        Utc::now(),
    );
    assert!(findings
        .iter()
        .any(|f| f.summary.contains("approval lapsed")));
    assert_eq!(
        before,
        fs::read(store.root().join("approvals.jsonl")).unwrap()
    );
    assert!(!store.root().join("ledger.jsonl").exists());
    store
        .detect_lapses(&rule.name, &owner_holds(), Utc::now())
        .unwrap();
    let findings = crate::doctor::check_mail_rules(
        &home.0,
        &crate::doctor::MailStores::under(&home.0),
        Utc::now(),
    );
    assert!(findings
        .iter()
        .any(|f| f.summary.contains("approval lapsed")));
    assert_eq!(waiting(store.root()).unwrap().len(), 1);
}

#[test]
fn mail_rules_doctor_reports_broken_history_and_unknown_outcome_independently() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let mut unknown = row(&rule, Outcome::Done);
    unknown.outcome = Outcome::Unknown;
    store.append_ledger(&unknown).unwrap();
    fs::write(store.root().join("approvals.jsonl"), "{broken approval\n").unwrap();
    let findings = crate::doctor::check_mail_rules(
        &home.0,
        &crate::doctor::MailStores::under(&home.0),
        Utc::now(),
    );
    assert!(findings
        .iter()
        .any(|f| f.summary.contains("approvals history")
            && f.detail.contains("`off` cannot be written")));
    assert!(findings
        .iter()
        .any(|f| f.summary.contains("unknown ledger outcome")));
    assert!(waiting(store.root()).is_err());
    assert!(store
        .change_mode(
            &rule.name,
            ActionGroup::Mailbox,
            Mode::Off,
            Some(Actor::Owner),
            &rule.approval_digest,
            &owner_holds()
        )
        .is_err());
}

#[test]
fn mail_rules_doctor_distinguishes_inflight_stranded_recovered_and_completed_actions() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let now = Utc::now();
    let recent = row(&rule, Outcome::Pending);
    store.append_ledger(&recent).unwrap();
    let examine = || {
        crate::doctor::check_mail_rules(&home.0, &crate::doctor::MailStores::under(&home.0), now)
    };
    assert!(examine().is_empty());
    let completed = recent.settled(Outcome::Done, "", now).unwrap();
    store.append_ledger(&completed).unwrap();
    assert!(examine().is_empty());
    let mut old = row(&rule, Outcome::Pending);
    old.at = Some(now - Duration::hours(1));
    store.append_ledger(&old).unwrap();
    assert!(examine()
        .iter()
        .any(|f| f.summary.contains("stranded pending")));
    let recovery = old
        .interrupted(now, engine::EngineOptions::default().pending_timeout)
        .unwrap()
        .unwrap();
    store.append_ledger(&recovery).unwrap();
    let findings = examine();
    assert!(!findings
        .iter()
        .any(|f| f.summary.contains("stranded pending")));
    assert!(findings
        .iter()
        .any(|f| f.summary.contains("recovered pending")));
    assert!(findings
        .iter()
        .all(|f| f.severity == crate::doctor::Severity::Attention));
}

#[test]
fn mail_rules_doctor_reports_every_automatic_group_under_unconfined_shell() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &text("[[action]]\ndo='archive'\n[[action]]\ndo='task'\n[[action]]\ndo='forward'\nto=['records@example.edu']"));
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    approve(&store, &rule, ActionGroup::Board, Mode::Auto);
    // A future build's outbound approval: P1 itself refuses this raise.
    let mut recorded = serde_json::to_value(store.approvals().unwrap().last().unwrap()).unwrap();
    recorded["id"] = serde_json::json!("future-permission");
    recorded["group"] = serde_json::json!("verbatim_forward");
    recorded["from_mode"] = serde_json::json!("shadow");
    use std::io::Write;
    writeln!(
        fs::OpenOptions::new()
            .append(true)
            .open(store.root().join("approvals.jsonl"))
            .unwrap(),
        "{recorded}"
    )
    .unwrap();
    let mut config = crate::config::Config::default();
    config.sandbox.kind = crate::sandbox::Backend::None;
    let findings = crate::doctor::check_mail_rule_shell(&home.0, &config);
    assert_eq!(findings.len(), 3);
    for group in ["Mailbox", "Board", "VerbatimForward"] {
        assert!(findings.iter().any(|f| f.summary.contains(group)
            && f.severity == crate::doctor::Severity::Attention
            && f.detail.contains("sandbox")));
    }
    config.sandbox.kind = crate::sandbox::Backend::Bwrap;
    assert!(crate::doctor::check_mail_rule_shell(&home.0, &config).is_empty());
    config.sandbox.kind = crate::sandbox::Backend::None;
    config.tools.disabled.push("shell".into());
    assert!(crate::doctor::check_mail_rule_shell(&home.0, &config).is_empty());
}

#[test]
fn mail_rules_doctor_detects_writable_ancestor_and_aliased_store_exposure() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    let alias = home.0.join("alias");
    std::os::unix::fs::symlink(store.root(), &alias).unwrap();
    for backend in [
        crate::sandbox::Backend::Bwrap,
        crate::sandbox::Backend::Docker,
    ] {
        let mut config = crate::config::Config::default();
        config.sandbox.kind = backend;
        assert!(crate::doctor::check_mail_rule_shell(&home.0, &config).is_empty());
        for path in [&home.0, store.root(), &alias] {
            config.sandbox.writable = vec![path.to_path_buf()];
            let findings = crate::doctor::check_mail_rule_shell(&home.0, &config);
            assert_eq!(findings.len(), 1, "{backend:?} exposed {}", path.display());
            assert_eq!(findings[0].severity, crate::doctor::Severity::Attention);
        }
    }
}

#[test]
fn mail_rules_store_is_protected_by_the_general_confinement_check() {
    let home = Home::new();
    let store = home.store();
    let alias = home.0.join("alias");
    std::os::unix::fs::symlink(store.root(), &alias).unwrap();
    let mut sandbox = crate::sandbox::SandboxConfig {
        kind: crate::sandbox::Backend::Bwrap,
        ..Default::default()
    };
    for path in [store.root(), &alias, &home.0] {
        sandbox.writable = vec![path.to_path_buf()];
        assert!(
            !crate::doctor::check_shell_confinement(&sandbox, &home.0).is_empty(),
            "{}",
            path.display()
        );
    }
    // Resolve the protected directory too, not only the configured mount.
    let relocated = home.0.join("relocated");
    fs::rename(store.root(), &relocated).unwrap();
    std::os::unix::fs::symlink(&relocated, store.root()).unwrap();
    sandbox.writable = vec![relocated];
    assert!(!crate::doctor::check_shell_confinement(&sandbox, &home.0).is_empty());
}

#[test]
fn mail_rules_doctor_reads_owner_registry_addresses_and_outlook_standing_holds() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, "description='Fictional list'\n[match]\nlist_id=['news.example.com']\n[[action]]\ndo='archive'\n");
    let registry = home.0.join("separate-mail");
    fs::create_dir_all(registry.join("fictional")).unwrap();
    fs::write(
        registry.join("accounts.toml"),
        "[[account]]\nname='fictional'\nprovider='outlook'\n",
    )
    .unwrap();
    fs::write(registry.join("fictional/oauth.json"), "{}").unwrap();
    let mail = crate::doctor::MailStores {
        registry: Some(registry.clone()),
        legacy: Vec::new(),
    };
    let findings = crate::doctor::check_mail_rules(&home.0, &mail, Utc::now());
    assert!(findings
        .iter()
        .any(|f| f.summary.contains("no stored mailbox address")
            && f.detail.contains("every mail rule")));
    assert!(!findings
        .iter()
        .any(|f| f.summary.contains("provider cannot prove header absence")));
    let outbound = store.install("outbound", "description='Fictional outbound list'\n[match]\nlist_id=['news.example.com']\n[[action]]\ndo='forward'\nto=['finance@example.edu']\n", &BTreeMap::new()).unwrap();
    let findings = crate::doctor::check_mail_rules(&home.0, &mail, Utc::now());
    let provider_holds: Vec<_> = findings
        .iter()
        .filter(|f| f.summary.contains("provider cannot prove header absence"))
        .collect();
    assert_eq!(provider_holds.len(), 1);
    assert!(provider_holds[0].summary.contains("outbound"));
    let mut views = rule_views(&store).unwrap();
    account_holds(
        &store,
        &mut views,
        &[RuleAccount {
            name: "fictional".into(),
            proves_header_absence: false,
        }],
    )
    .unwrap();
    assert!(views
        .iter()
        .find(|v| v.name == rule.name)
        .unwrap()
        .holds
        .is_empty());
    assert_eq!(
        views
            .iter()
            .find(|v| v.name == "outbound")
            .unwrap()
            .holds
            .len(),
        1
    );
    fs::write(
        registry.join("fictional/oauth.json"),
        r#"{"account":"owner@example.edu"}"#,
    )
    .unwrap();
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Off);
    approve(&store, &outbound, ActionGroup::VerbatimForward, Mode::Off);
    assert!(crate::doctor::check_mail_rules(&home.0, &mail, Utc::now()).is_empty());
}

#[test]
fn mail_rules_doctor_does_not_create_stores_and_reports_an_unreadable_store() {
    let home = Home::new();
    let mail = crate::doctor::MailStores::under(&home.0);
    assert!(crate::doctor::check_mail_rules(&home.0, &mail, Utc::now()).is_empty());
    assert!(fs::read_dir(&home.0).unwrap().next().is_none());
    fs::write(RuleStore::root_under(&home.0), "not a directory").unwrap();
    assert!(crate::doctor::check_mail_rules(&home.0, &mail, Utc::now())
        .iter()
        .any(|f| f.summary.contains("store unreadable")));
}

#[test]
fn mail_rules_doctor_timer_is_checked_only_when_enabled_and_oneshot_idle_is_healthy() {
    use crate::doctor::check_mail_rules_timer;
    assert!(check_mail_rules_timer(false, Some("LoadState=not-found\n"), None).is_empty());
    assert!(check_mail_rules_timer(true, None, None).is_empty());
    let timer = "LoadState=loaded\nActiveState=active\n";
    let service = "LoadState=loaded\nActiveState=inactive\nResult=success\n";
    assert!(check_mail_rules_timer(true, Some(timer), Some(service)).is_empty());
    assert_eq!(
        check_mail_rules_timer(true, Some("LoadState=not-found\n"), Some(service)).len(),
        1
    );
    assert_eq!(
        check_mail_rules_timer(
            true,
            Some(timer),
            Some("LoadState=loaded\nActiveState=failed\nResult=exit-code\n")
        )
        .len(),
        1
    );
    assert_eq!(
        check_mail_rules_timer(
            true,
            Some("LoadState=loaded\nActiveState=inactive\n"),
            Some(service)
        )
        .len(),
        1
    );
}

#[test]
fn mail_rules_backlog_counts_proposals_and_lapsed_groups_and_clears_reapproval() {
    let home = Home::new();
    assert!(waiting(&RuleStore::root_under(&home.0)).unwrap().is_empty());
    assert!(fs::read_dir(&home.0).unwrap().next().is_none());
    let store = home.store();
    let rule = install(&store, &archive());
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    fs::write(
        rule.directory.join("rule.toml"),
        archive().replace("Fictional notice", "Fictional revision"),
    )
    .unwrap();
    let proposal = store.root().join("proposals/fictional-proposal");
    fs::create_dir_all(&proposal).unwrap();
    fs::write(proposal.join("rule.toml"), archive()).unwrap();
    let before = fs::read(store.root().join("approvals.jsonl")).unwrap();
    let items = waiting(store.root()).unwrap();
    assert_eq!(items.len(), 2);
    assert!(items
        .iter()
        .any(|i| i.id == "proposal:fictional-proposal" && !i.since.is_empty()));
    assert!(items.iter().any(|i| i.id.starts_with("approval:notices:")));
    assert_eq!(
        before,
        fs::read(store.root().join("approvals.jsonl")).unwrap()
    );
    approve(
        &store,
        &store.get(&rule.name).unwrap(),
        ActionGroup::Mailbox,
        Mode::Auto,
    );
    assert_eq!(waiting(store.root()).unwrap().len(), 1);
    fs::remove_dir_all(proposal).unwrap();
    assert!(waiting(store.root()).unwrap().is_empty());
}

#[test]
fn mail_rules_backlog_keeps_lapses_and_proposals_when_a_definition_is_unreadable() {
    let home = Home::new();
    let store = home.store();
    let broken = install(&store, &archive());
    let healthy = store
        .install("healthy", &archive(), &BTreeMap::new())
        .unwrap();
    for rule in [&broken, &healthy] {
        approve(&store, rule, ActionGroup::Mailbox, Mode::Auto);
    }
    fs::write(broken.directory.join("rule.toml"), "invalid = [").unwrap();
    assert!(store
        .detect_lapses(&broken.name, &owner_holds(), Utc::now())
        .is_err());
    fs::write(
        healthy.directory.join("rule.toml"),
        archive().replace("Fictional notice", "Fictional revision"),
    )
    .unwrap();
    let proposal = store.root().join("proposals/fictional-proposal");
    fs::create_dir_all(&proposal).unwrap();
    fs::write(proposal.join("rule.toml"), archive()).unwrap();
    let before = fs::read(store.root().join("approvals.jsonl")).unwrap();
    for deleted in [false, true] {
        if deleted {
            fs::remove_dir_all(&broken.directory).unwrap();
        }
        let items = waiting(store.root()).unwrap();
        assert_eq!(items.len(), 3);
        for id in [
            "approval:notices:Mailbox",
            "approval:healthy:Mailbox",
            "proposal:fictional-proposal",
        ] {
            assert!(
                items
                    .iter()
                    .any(|item| item.id == id && !item.since.is_empty()),
                "missing {id}"
            );
        }
        assert_eq!(
            before,
            fs::read(store.root().join("approvals.jsonl")).unwrap()
        );
    }
}

#[test]
fn mail_rules_backlog_does_not_follow_proposals_outside_its_store() {
    let home = Home::new();
    let outside = Home::new();
    let store = home.store();
    std::os::unix::fs::symlink(&outside.0, store.root().join("proposals")).unwrap();
    assert!(waiting(store.root()).is_err());
}

#[test]
fn mail_rules_engine_lease_contention_has_a_typed_skip() {
    let home = Home::new();
    let store = home.store();
    let lease = store.engine_lease().unwrap();
    let error = match store.engine_lease() {
        Err(error) => error,
        Ok(_) => panic!("overlapping lease"),
    };
    assert!(error.is::<ApplyRunning>());
    drop(lease);
    assert!(store.engine_lease().is_ok());
}

#[test]
fn mail_rules_p3_edit_is_validated_atomic_and_persists_every_lapse() {
    let home = Home::new();
    let store = home.store();
    let original =
        text("[[action]]\ndo='archive'\n[[action]]\ndo='digest'\n[[action]]\ndo='spam'\n");
    let rule = store
        .install(
            "notices",
            &original,
            &BTreeMap::from([("extra.md".into(), "Fictional template".into())]),
        )
        .unwrap();
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    approve(&store, &rule, ActionGroup::Report, Mode::Off);
    let before = fs::read(store.root().join("approvals.jsonl")).unwrap();
    assert!(store.edit("notices", "invalid = [").is_err());
    assert_eq!(
        fs::read(store.root().join("approvals.jsonl")).unwrap(),
        before
    );
    assert_eq!(
        fs::read_to_string(rule.directory.join("rule.toml")).unwrap(),
        original
    );
    assert!(store
        .edit("missing", &original)
        .unwrap_err()
        .to_string()
        .contains("unknown rule"));
    let edited = original.replace("Fictional notice", "Changed fictional label");
    let report = store.edit("notices", &edited).unwrap();
    assert_eq!(report.changed_files, ["rule.toml"]);
    assert_eq!(report.lapsed_groups.len(), 2);
    assert!(!report.lapsed_groups.contains(&ActionGroup::Report));
    assert_ne!(report.approval_digest, rule.approval_digest);
    assert_eq!(
        fs::read_to_string(rule.directory.join("extra.md")).unwrap(),
        "Fictional template"
    );
    let view = rule_views(&store).unwrap().remove(0);
    assert_eq!(
        view.groups
            .iter()
            .find(|g| g.group == ActionGroup::Mailbox)
            .unwrap()
            .recorded_mode,
        Mode::Propose
    );
    assert_eq!(
        view.groups
            .iter()
            .find(|g| g.group == ActionGroup::PrivateRecord)
            .unwrap()
            .recorded_mode,
        Mode::Shadow
    );
    assert_eq!(
        store
            .ledger()
            .unwrap()
            .iter()
            .filter(|r| r.outcome == Outcome::ApprovalLapsed)
            .count(),
        1
    );
    assert!(store
        .edit("notices", &edited)
        .unwrap()
        .lapsed_groups
        .is_empty());
    store.edit("notices", &original).unwrap();
    assert_eq!(
        rule_views(&store).unwrap()[0]
            .groups
            .iter()
            .find(|g| g.group == ActionGroup::Mailbox)
            .unwrap()
            .recorded_mode,
        Mode::Propose
    );
}

#[test]
fn mail_rules_p3_stale_and_missing_web_digest_refuse_even_without_auto() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let missing = store
        .change_mode_checked(
            &rule.name,
            ActionGroup::Mailbox,
            Mode::Propose,
            Some(Actor::Owner),
            &rule.approval_digest,
            &owner_holds(),
            None,
            true,
        )
        .unwrap_err();
    assert!(missing
        .downcast_ref::<ModeError>()
        .is_some_and(|e| matches!(e, ModeError::DigestRequired)));
    store
        .edit(
            &rule.name,
            &archive().replace("Fictional notice", "Changed label"),
        )
        .unwrap();
    for mode in [Mode::Propose, Mode::Off] {
        let error = store
            .change_mode_checked(
                &rule.name,
                ActionGroup::Mailbox,
                mode,
                Some(Actor::Owner),
                &rule.approval_digest,
                &owner_holds(),
                Some(&rule.approval_digest),
                true,
            )
            .unwrap_err();
        assert!(error
            .downcast_ref::<ModeError>()
            .is_some_and(|e| matches!(e, ModeError::StaleDigest)));
    }
}

#[test]
fn mail_rules_p3_pause_is_per_rule_preserves_off_and_records_actor() {
    let home = Home::new();
    let store = home.store();
    let rule = install(
        &store,
        &text("[[action]]\ndo='archive'\n[[action]]\ndo='digest'"),
    );
    let other = store
        .install("other", &archive(), &BTreeMap::new())
        .unwrap();
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    approve(&store, &rule, ActionGroup::PrivateRecord, Mode::Off);
    approve(&store, &other, ActionGroup::Mailbox, Mode::Auto);
    let report = pause_rule(&store, &rule.name, Actor::Unknown).unwrap();
    assert_eq!(report.groups, 1);
    let rows = store.approvals().unwrap();
    assert_eq!(rows.last().unwrap().actor, Actor::Unknown);
    assert_eq!(rows.last().unwrap().to_mode, Mode::Shadow);
    assert_eq!(
        resolve_mode(&other, ActionGroup::Mailbox, &rows, &owner_holds()).recorded,
        Mode::Auto
    );
    assert_eq!(
        resolve_mode(&rule, ActionGroup::PrivateRecord, &rows, &owner_holds()).recorded,
        Mode::Off
    );
    assert!(pause_rule(&store, "missing", Actor::Owner).is_err());
}

#[tokio::test]
async fn mail_rules_p3_wrong_match_is_idempotent_closing_and_independent_of_undo() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let mut original = row(&rule, Outcome::Done);
    original.matched_because = "from".into();
    store.append_ledger(&original).unwrap();
    let correction = store.wrong_match(&original.id, Actor::Owner).unwrap();
    let wrong = correction.row.unwrap();
    assert_eq!(wrong.correction_of.as_deref(), Some(original.id.as_str()));
    assert_eq!(wrong.outcome, Outcome::WrongMatch);
    assert!(wrong.closes && wrong.settlement.is_none());
    assert!(message_closed(&store.ledger().unwrap(), &key(&original)));
    assert!(
        !store
            .wrong_match(&original.id, Actor::Owner)
            .unwrap()
            .changed
    );
    assert_eq!(store.ledger().unwrap().len(), 2);
    // The old wire reader's unknown default tightens even without closes.
    let mut old = wrong.clone();
    old.outcome = Outcome::Unknown;
    old.closes = false;
    assert!(message_closed(&[old], &key(&original)));
    assert!(prepare_undo(&store, &original.id).is_ok());
    let restore = RestoreFixture::default();
    undo_with_actor(
        &store,
        &original.id,
        &restore,
        Utc::now(),
        Some(Actor::Owner),
    )
    .await
    .unwrap();
    assert!(
        !store
            .wrong_match(&original.id, Actor::Owner)
            .unwrap()
            .changed
    );
    let evidence = engine::brake_evidence(&rule.name, &store.ledger().unwrap());
    assert_eq!(evidence.wrong_matches, 1);
    assert_eq!(evidence.undone_mailbox, 1);
    let stats = rule_views(&store)
        .unwrap()
        .remove(0)
        .groups
        .remove(0)
        .statistics
        .remove(0);
    assert_eq!(stats.owner_corrected_actions, 1);
    assert_eq!(stats.owner_undos, 1);
    assert_eq!(stats.owner_wrong_matches, 1);
    let views = ledger_views(&store, None, 50).unwrap();
    let original_view = views.iter().find(|r| r["id"] == original.id).unwrap();
    assert_eq!(original_view["undoable"], false);
    assert_eq!(original_view["wrong_match_allowed"], false);
}

#[test]
fn mail_rules_p3_wrong_match_is_one_correction_per_action_and_never_on_an_attempt() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let mut pending = row(&rule, Outcome::Pending);
    pending.matched_because = "from".into();
    store.append_ledger(&pending).unwrap();
    // The attempt is not the action: its settlement row is what gets corrected.
    assert!(store.wrong_match(&pending.id, Actor::Owner).is_err());
    let mut done = row(&rule, Outcome::Done);
    done.matched_because = "from".into();
    store.append_ledger(&done).unwrap();
    assert!(store.wrong_match(&done.id, Actor::Owner).unwrap().changed);
    // Another row of the same action key is the same wrong match.
    let mut again = row(&rule, Outcome::Done);
    again.matched_because = "from".into();
    store.append_ledger(&again).unwrap();
    assert!(!store.wrong_match(&again.id, Actor::Owner).unwrap().changed);
    let corrections = store
        .ledger()
        .unwrap()
        .into_iter()
        .filter(|r| r.outcome == Outcome::WrongMatch)
        .count();
    assert_eq!(corrections, 1);
    for view in ledger_views(&store, None, 50).unwrap() {
        assert_eq!(view["wrong_match_allowed"], false, "{}", view["id"]);
    }
}

#[test]
fn mail_rules_p3_wrong_match_refuses_a_row_of_another_kind_and_brakes_once_per_thread() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    let mut ids = Vec::new();
    for index in 0..10 {
        let mut original = row(&rule, Outcome::Done);
        original.message.provider_message_id = format!("message-{index}");
        original.message.thread_id = format!("thread-{index}");
        original.matched_because = "from".into();
        ids.push(original.id.clone());
        store.append_ledger(&original).unwrap();
    }
    for id in ids.iter().take(3) {
        store.wrong_match(id, Actor::Owner).unwrap();
        store.wrong_match(id, Actor::Owner).unwrap();
    }
    assert_eq!(
        engine::brake_evidence(&rule.name, &store.ledger().unwrap()).wrong_matches,
        3
    );
    assert_eq!(
        rule_views(&store).unwrap()[0].groups[0].recorded_mode,
        Mode::Propose
    );
    let description = archive().replace(
        "from = ['robot@example.com']",
        "describe = 'Fictional notices'",
    );
    store.edit(&rule.name, &description).unwrap();
    assert!(store
        .wrong_match(&ids[4], Actor::Owner)
        .unwrap_err()
        .to_string()
        .contains("not the rule's current kind"));
    assert!(store.wrong_match("missing", Actor::Owner).is_err());
}

#[test]
fn mail_rules_p3_statistics_keep_inputs_digests_and_unknown_envelopes_apart() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let empty = rule_views(&store).unwrap();
    assert!(empty[0].groups[0]
        .statistics
        .iter()
        .all(|s| s.correction_rate.is_none()));
    let mut original = row(&rule, Outcome::Done);
    original.matched_because = "from".into();
    original.message.sender.clear();
    original.message.subject.clear();
    store.append_ledger(&original).unwrap();
    store.wrong_match(&original.id, Actor::Unknown).unwrap();
    let stats = rule_views(&store)
        .unwrap()
        .remove(0)
        .groups
        .remove(0)
        .statistics;
    assert_eq!(stats[0].owner_wrong_matches, 0);
    assert_eq!(stats[0].correction_rate, Some(0.0));
    assert!(stats[1].correction_rate.is_none());
    let view = ledger_views(&store, None, 50).unwrap();
    assert!(view[0]["sender"].is_null() && view[0]["subject"].is_null());
    store
        .edit(
            &rule.name,
            &archive().replace("robot@example.com", "notices@example.edu"),
        )
        .unwrap();
    assert!(rule_views(&store).unwrap()[0].groups[0].statistics[0]
        .correction_rate
        .is_none());
}

#[test]
fn mail_rules_p3_doctor_names_all_write_and_network_routes_once_per_group() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    let mut config = crate::config::Config::default();
    config.sandbox.kind = crate::sandbox::Backend::Bwrap;
    config.sandbox.network = true;
    let network = crate::doctor::check_mail_rule_shell(&home.0, &config);
    assert_eq!(network.len(), 1);
    assert!(network[0].detail.contains("serve's loopback port"));
    assert!(network[0].detail.contains("network off"));
    assert!(!network[0].detail.contains("shell can write"));
    config.sandbox.writable.push(store.root().into());
    let both = crate::doctor::check_mail_rule_shell(&home.0, &config);
    assert_eq!(both.len(), 1);
    assert!(
        both[0].detail.contains("shell can write")
            && both[0].detail.contains("serve's loopback port")
    );
    config.sandbox.network = false;
    assert!(!crate::doctor::check_mail_rule_shell(&home.0, &config)[0]
        .detail
        .contains("serve's loopback port"));
    config.sandbox.writable.clear();
    assert!(crate::doctor::check_mail_rule_shell(&home.0, &config).is_empty());
    config.sandbox.kind = crate::sandbox::Backend::Landlock;
    assert!(crate::doctor::check_mail_rule_shell(&home.0, &config)[0]
        .detail
        .contains("serve's loopback port"));
    config.tools.disabled.push("shell".into());
    assert!(crate::doctor::check_mail_rule_shell(&home.0, &config).is_empty());
}

#[test]
fn mail_rules_p3_retrospective_rows_include_envelope_and_provider_arrival_without_writes() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let now = Utc::now();
    let received = now - Duration::days(20);
    let candidate = engine::Candidate {
        message: message(),
        arrival_at: Some(received),
        headers_complete: Some(true),
        subject_complete: Some(true),
        subject_is_raw: false,
        recipients_complete: Some(true),
        attachment_presence: Some(false),
        to: Vec::new(),
        list_id: None,
        headers: BTreeMap::new(),
        classified: None,
    };
    let rows = test_match(&rule, &candidate, Some(false), now).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, Outcome::WouldHave);
    assert_eq!(rows[0].received_at, Some(received));
    assert_eq!(rows[0].in_inbox, Some(false));
    assert_eq!(rows[0].message.sender, "robot@example.com");
    assert!(!rows[0].matched_because.is_empty());
    assert!(rows[0].reason.contains("no longer in the inbox"));
    assert!(store.ledger().unwrap().is_empty());
    let json = serde_json::to_value(&rows[0]).unwrap();
    assert!(json.get("body").is_none());
}

#[test]
fn mail_rules_p3_wrong_match_append_guard_retains_key_and_cannot_settle() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let original = row(&rule, Outcome::WouldHave);
    store.append_ledger(&original).unwrap();
    let mut correction = original.clone();
    correction.id = "wrong-fixture".into();
    correction.correction_of = Some(original.id.clone());
    correction.outcome = Outcome::WrongMatch;
    correction.wrong_match = true;
    assert!(
        store.append_ledger(&correction).is_err(),
        "closes:false must refuse"
    );
    correction.closes = true;
    correction.match_digest.matcher = "different".into();
    assert!(
        store.append_ledger(&correction).is_err(),
        "changed measurement must refuse"
    );
    correction.match_digest = original.match_digest.clone();
    store.append_ledger(&correction).unwrap();
    assert!(message_closed(&store.ledger().unwrap(), &key(&original)));
    correction.id = "duplicate-fixture".into();
    assert!(store.append_ledger(&correction).is_err());
    let pending = row(&rule, Outcome::Pending);
    assert!(pending
        .settled(Outcome::WrongMatch, "not a settlement", Utc::now())
        .is_err());
}

#[test]
fn mail_rules_p3_edit_cannot_restore_auto_after_an_unpersisted_external_lapse() {
    for broken in [false, true] {
        let home = Home::new();
        let store = home.store();
        let original = archive();
        let rule = install(&store, &original);
        approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
        let external = if broken {
            "[broken".to_owned()
        } else {
            original.replace("Fictional notice", "External edit")
        };
        fs::write(rule.directory.join("rule.toml"), external).unwrap();
        let before = rule_views(&store).unwrap()[0].groups[0].effective_mode;
        assert!(before.rank() < Mode::Auto.rank());
        let report = store.edit(&rule.name, &original).unwrap();
        assert_eq!(report.approval_digest, rule.approval_digest);
        assert_eq!(report.lapsed_groups, [ActionGroup::Mailbox]);
        let after = &rule_views(&store).unwrap()[0].groups[0];
        assert!(after.effective_mode.rank() <= before.rank());
        assert!(after.recorded_mode.rank() < Mode::Auto.rank());
    }
}

#[test]
fn mail_rules_partial_header_marker_defaults_false_for_historical_and_unknown_wire_values() {
    let home = Home::new();
    let store = home.store();
    let rule = install(&store, &archive());
    let mut value = serde_json::to_value(row(&rule, Outcome::Done)).unwrap();
    value.as_object_mut().unwrap().remove("headers_partial");
    assert!(
        !serde_json::from_value::<LedgerRow>(value.clone())
            .unwrap()
            .headers_partial
    );
    for unknown in [
        serde_json::Value::Null,
        serde_json::json!("true"),
        serde_json::json!({"future":true}),
    ] {
        value["headers_partial"] = unknown;
        assert!(
            !serde_json::from_value::<LedgerRow>(value.clone())
                .unwrap()
                .headers_partial
        );
    }
    value["headers_partial"] = serde_json::json!(true);
    let row: LedgerRow = serde_json::from_value(value).unwrap();
    assert!(row.headers_partial);
    store.append_ledger(&row).unwrap();
    assert_eq!(
        ledger_views(&store, None, 10).unwrap()[0]["headers_partial"],
        true
    );
}

#[test]
fn route_segment_names_are_refused_at_install() {
    let home = Home::new();
    let store = home.store();
    for name in RESERVED_RULE_NAMES {
        let error = store
            .install(name, &archive(), &BTreeMap::new())
            .unwrap_err();
        assert!(error.to_string().contains("reserved"), "{name}: {error:#}");
        assert!(store.get(name).is_err(), "{name} must not be installed");
    }
}
