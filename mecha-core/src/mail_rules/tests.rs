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
    store
        .change_mode(
            &rule.name,
            group,
            mode,
            Some(Actor::Owner),
            &rule.approval_digest,
            &owner_holds(),
        )
        .unwrap();
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
fn loader_parses_all_matchers_and_defaults_without_evaluating_descriptions() {
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
    approve(&store, &rule, ActionGroup::Mailbox, Mode::Auto);
    let resolved = resolve_mode(
        &rule,
        ActionGroup::Mailbox,
        &store.approvals().unwrap(),
        &owner_holds(),
    );
    assert_eq!(resolved.effective, Mode::Shadow);
    assert!(!resolved.holds.is_empty());
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
            assert!(store
                .change_mode(
                    &rule.name,
                    group,
                    Mode::Propose,
                    Some(Actor::Owner),
                    &rule.approval_digest,
                    &owner_holds()
                )
                .is_err());
        } else {
            approve(&store, &rule, group, Mode::Auto);
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
    for outcome in [
        Outcome::WouldHave,
        Outcome::Skipped,
        Outcome::Expired,
        Outcome::Failed,
    ] {
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
        ] {
            let mut action = row(&rule, outcome);
            action.group = group;
            action.closes = false;
            assert!(thread_closed(std::slice::from_ref(&action), &thread_key));
            let mut other_message = key(&action);
            other_message.provider_message_id = "new-message".into();
            assert!(!message_closed(&[action], &other_message));
        }
        for outcome in [Outcome::WouldHave, Outcome::Skipped, Outcome::Expired] {
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
    assert!(!message_closed(&rows, &key(&action)));
    assert!(store
        .expire_deferred(now + Duration::days(1))
        .unwrap()
        .is_empty());
    store.enqueue(&entry).unwrap();
    assert!(store.settle_deferred(&entry.id, &[]).is_err());
    store
        .settle_deferred(&entry.id, &[row(&rule, Outcome::Done)])
        .unwrap();
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
    assert!(store.claim_actions(&[first.clone(), task.clone()]).unwrap());
    assert!(!store
        .claim_actions(&[row(&rule, Outcome::Pending)])
        .unwrap());
    let settled = first
        .settled(Outcome::Failed, "interrupted", Utc::now())
        .unwrap();
    store.append_ledger(&settled).unwrap();
    assert!(store
        .claim_actions(&[row(&rule, Outcome::Pending)])
        .unwrap());
    let mut later_task = task;
    later_task.id = "another-attempt".into();
    later_task.message.provider_message_id = "new-message".into();
    assert!(!store.claim_actions(&[later_task]).unwrap());
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
        .filter(|won| *won)
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
            assert!(!store
                .claim_actions(&[row(&rule, Outcome::Pending)])
                .unwrap());
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
    assert!(worker
        .claim_actions(std::slice::from_ref(&pending))
        .unwrap());
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
    assert!(store.claim_actions(std::slice::from_ref(&pending)).unwrap());
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
