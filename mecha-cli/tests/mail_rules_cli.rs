//! Real command routing and actor gates; provider children are executable fixtures.
use mecha_core::{
    closure::{Actor, RunPosture},
    mail_rules::*,
    shell_registry::ShellRegistry,
};
use std::{
    collections::BTreeMap,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output},
};

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "mail-rule-cli-{}",
            mecha_core::session::Session::new_id()
        ));
        std::fs::create_dir_all(root.join("home")).unwrap();
        std::fs::write(
            root.join("home/config.toml"),
            "[features]\nmail = true\nmail_rules = true\n",
        )
        .unwrap();
        Self { root }
    }
    fn command(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_mecha"));
        c.current_dir(&self.root)
            .env("MECHA_HOME", self.root.join("home"))
            .env("MECHA_MAIL_DIR", self.root.join("home/mail"))
            .env("MECHA_SESSION_KIND", "test")
            .env_remove(mecha_core::closure::POSTURE_ENV);
        c
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command()
            .args(["mail", "rule"])
            .args(args)
            .output()
            .unwrap()
    }
    fn store(&self) -> RuleStore {
        RuleStore::open(self.root.join("home/mail-rules")).unwrap()
    }
    fn rule(&self) -> LoadedRule {
        self.store().install("notice", "description = 'Fictional notice'\n[match]\nfrom_domain = ['example.com']\n[[action]]\ndo = 'archive'\n", &BTreeMap::new()).unwrap()
    }
    fn isolated_binary(&self) -> PathBuf {
        let executable = self.root.join("bin/mecha");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        if std::fs::hard_link(env!("CARGO_BIN_EXE_mecha"), &executable).is_err() {
            std::fs::copy(env!("CARGO_BIN_EXE_mecha"), &executable).unwrap();
        }
        executable
    }
    fn fake(&self, directory: &str, marker: &str, code: u8) -> PathBuf {
        let directory = self.root.join(directory);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("mecha-mail");
        std::fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' '{marker}' \"$@\"\nprintf '%s\\n' 'fixture stderr' >&2\nexit {code}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        directory
    }
    fn isolated_command(&self, executable: &std::path::Path) -> Command {
        let mut c = Command::new(executable);
        c.current_dir(&self.root)
            .env("MECHA_HOME", self.root.join("home"))
            .env("MECHA_MAIL_DIR", self.root.join("home/mail"))
            .env("MECHA_SESSION_KIND", "test")
            .env_remove(mecha_core::closure::POSTURE_ENV);
        c
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn success(out: &Output) {
    assert!(
        out.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn mail_rule_review_registry_uses_the_shared_non_utf8_override_fallback() {
    use std::os::unix::ffi::OsStringExt;
    let f = Fixture::new();
    f.store().install("notice", "description='Fictional notice'\n[match]\nlist_id=['notices.example.com']\n[[action]]\ndo='archive'\n", &BTreeMap::new()).unwrap();
    let real_home = f.root.join("real-home");
    let fallback = real_home.join(".mecha/mail");
    let invalid = f
        .root
        .join(std::ffi::OsString::from_vec(b"mail-\xff".to_vec()));
    for (directory, provider) in [(&fallback, "google"), (&invalid, "outlook")] {
        std::fs::create_dir_all(directory).unwrap();
        std::fs::write(
            directory.join("accounts.toml"),
            format!("[[account]]\nname='fictional'\nprovider='{provider}'\n"),
        )
        .unwrap();
    }
    let read = |override_path: Option<&std::path::Path>| {
        let mut command = f.command();
        command.env("HOME", &real_home).env_remove("MECHA_MAIL_DIR");
        if let Some(path) = override_path {
            command.env("MECHA_MAIL_DIR", path);
        }
        command
            .args(["mail", "rule", "list", "--json"])
            .output()
            .unwrap()
    };
    let fallback_read = read(None);
    success(&fallback_read);
    let ignored_override = read(Some(&invalid));
    success(&ignored_override);
    assert_eq!(ignored_override.stdout, fallback_read.stdout);
    let missing_registry = read(Some(invalid.parent().unwrap()));
    assert_ne!(
        missing_registry.stdout, fallback_read.stdout,
        "the fallback must supply provider evidence"
    );
}

#[test]
fn mail_rule_review_pause_skips_junk_but_keeps_strict_store_commands() {
    let f = Fixture::new();
    let store = f.store();
    let first = f.rule();
    let second = store
        .install(
            "second",
            &std::fs::read_to_string(first.directory.join("rule.toml")).unwrap(),
            &BTreeMap::new(),
        )
        .unwrap();
    for rule in [&first, &second] {
        store
            .change_mode(
                &rule.name,
                ActionGroup::Mailbox,
                Mode::Auto,
                Some(Actor::Owner),
                &rule.approval_digest,
                &owner_file_holds(),
            )
            .unwrap();
    }
    let stray = store.root().join("rules/notice.old");
    std::fs::create_dir(&stray).unwrap();
    std::fs::copy(first.directory.join("rule.toml"), stray.join("rule.toml")).unwrap();
    std::fs::write(store.root().join("rules/stray"), "not a directory").unwrap();
    let paused = f.run(&["pause-all"]);
    success(&paused);
    let notices = String::from_utf8_lossy(&paused.stderr);
    for junk in ["notice.old", "stray"] {
        assert_eq!(notices.matches(junk).count(), 1, "{notices}");
    }
    for rule in [&first, &second] {
        assert_eq!(
            resolve_mode(
                rule,
                ActionGroup::Mailbox,
                &store.approvals().unwrap(),
                &owner_file_holds()
            )
            .recorded,
            Mode::Shadow
        );
    }
    assert!(!f.run(&["list"]).status.success());
    assert!(!f.run(&["add", stray.to_str().unwrap()]).status.success());
}

fn refused_before_review(action: &str, group: &str) {
    let f = Fixture::new();
    let store = f.store();
    store
        .install(
            "notice",
            &format!(
                "description='Fictional notice'\n[match]\nfrom_domain=['example.com']\n{action}"
            ),
            &BTreeMap::new(),
        )
        .unwrap();
    let before = store.approvals().unwrap().len();
    for confirmation in [vec![], vec!["--yes"]] {
        let mut args = vec!["mode", "notice", group, "auto"];
        args.extend(confirmation);
        let out = f.run(&args);
        assert!(!out.status.success());
        assert!(
            out.stdout.is_empty(),
            "approval screen shown for a forbidden mode: {}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert!(String::from_utf8_lossy(&out.stderr).contains("permanent ceiling"));
        assert_eq!(store.approvals().unwrap().len(), before);
    }
}

#[test]
fn mail_rule_review_send_ceiling_precedes_the_approval_screen() {
    refused_before_review(
        "[[action]]\ndo='forward'\nto=['records@example.edu']\n",
        "verbatim-forward",
    );
}

#[test]
fn mail_rule_review_report_ceiling_precedes_the_approval_screen() {
    refused_before_review("[[action]]\ndo='spam'\n", "report");
}

#[test]
fn mail_rule_review_empty_list_is_explicit_without_changing_json() {
    let f = Fixture::new();
    for create_store in [false, true] {
        if create_store {
            f.store();
        }
        let out = f.run(&["list"]);
        success(&out);
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            "no mail rules installed"
        );
        let json = f.run(&["list", "--json"]);
        success(&json);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&json.stdout).unwrap(),
            serde_json::json!([])
        );
        // show already refuses a missing name explicitly; it is not an empty success.
        let show = f.run(&["show", "missing"]);
        assert!(!show.status.success());
        assert!(!show.stderr.is_empty());
    }
}

#[test]
fn mail_rule_review_empty_ledger_is_explicit_without_changing_json() {
    let f = Fixture::new();
    for create_store in [false, true] {
        if create_store {
            f.store();
        }
        let out = f.run(&["ledger"]);
        success(&out);
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            "no mail rule ledger entries"
        );
        let json = f.run(&["ledger", "--json"]);
        success(&json);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&json.stdout).unwrap(),
            serde_json::json!([])
        );
    }
}

#[test]
fn mail_rule_store_verbs_show_json_and_require_explicit_auto_confirmation() {
    let f = Fixture::new();
    let source = f.root.join("notice");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("rule.toml"), "description = 'Fictional notice'\n[match]\nfrom_domain = ['example.com']\n[[action]]\ndo = 'archive'\n").unwrap();
    let out = f.run(&["add", source.to_str().unwrap()]);
    success(&out);
    assert!(String::from_utf8_lossy(&out.stdout).contains("Approval digest"));
    let show = f.run(&["show", "notice", "--json"]);
    success(&show);
    let view: serde_json::Value = serde_json::from_slice(&show.stdout).unwrap();
    assert_eq!(view["groups"][0]["recorded_mode"], "shadow");
    let refused = f.run(&["mode", "notice", "mailbox", "auto"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("--yes"));
    let approved = f.run(&["mode", "notice", "mailbox", "auto", "--yes"]);
    success(&approved);
    let displayed = String::from_utf8_lossy(&approved.stdout);
    assert!(displayed.contains("from_domain = ['example.com']"));
    assert!(displayed.contains("Approval digest"));
    let before = std::fs::read(f.store().root().join("approvals.jsonl")).unwrap();
    success(&f.run(&["list", "--json"]));
    success(&f.run(&["show", "notice", "--json"]));
    success(&f.run(&["ledger", "notice", "--json"]));
    assert_eq!(
        before,
        std::fs::read(f.store().root().join("approvals.jsonl")).unwrap()
    );
    success(&f.run(&["pause-all"]));
    assert_eq!(
        f.store().approvals().unwrap().last().unwrap().to_mode,
        Mode::Shadow
    );
}

#[test]
fn mail_rule_unknown_actor_cannot_raise_and_off_clamps_honestly() {
    let f = Fixture::new();
    f.rule();
    let registry = ShellRegistry::open(f.root.join("home/runs/shells")).unwrap();
    let _registered = registry
        .register(
            std::process::id(),
            Some(RunPosture::Delegated),
            Some("fixture-call"),
        )
        .unwrap();
    let out = f.run(&["mode", "notice", "mailbox", "auto", "--yes"]);
    assert!(!out.status.success());
    let reason = String::from_utf8_lossy(&out.stderr);
    assert!(reason.contains("Owner"));
    let out = f.run(&["mode", "notice", "mailbox", "off"]);
    success(&out);
    assert!(String::from_utf8_lossy(&out.stdout).contains("clamped to shadow"));
    assert!(!f
        .store()
        .approvals()
        .unwrap()
        .iter()
        .any(|r| r.to_mode == Mode::Auto || r.to_mode == Mode::Off));
    let store = f.store();
    let rule = store.get("notice").unwrap();
    store
        .change_mode(
            "notice",
            ActionGroup::Mailbox,
            Mode::Auto,
            Some(Actor::Owner),
            &rule.approval_digest,
            &owner_file_holds(),
        )
        .unwrap();
    success(&f.run(&["pause-all"]));
    assert_eq!(
        store.approvals().unwrap().last().unwrap().actor,
        Actor::Unknown
    );
}

#[test]
fn mail_rule_child_prefers_sibling_and_preserves_argv_streams_and_exit_status() {
    let f = Fixture::new();
    let executable = f.isolated_binary();
    f.fake("bin", "sibling", 17);
    let path = f.fake("path", "path", 0);
    let out = f
        .isolated_command(&executable)
        .env("PATH", path)
        .args([
            "mail",
            "rule",
            "test",
            "notice;touch forbidden",
            "--account",
            "campus with spaces",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(17));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "sibling\nrules\ntest\nnotice;touch forbidden\n--account\ncampus with spaces\n"
    );
    assert_eq!(String::from_utf8(out.stderr).unwrap(), "fixture stderr\n");
    assert!(!f.root.join("forbidden").exists());
}

#[test]
fn mail_rule_child_refuses_path_binary_when_sibling_is_missing() {
    let f = Fixture::new();
    let executable = f.isolated_binary();
    let path = f.fake("path", "path", 0);
    std::fs::write(
        path.join("mecha-mail"),
        "#!/bin/sh\nprintf planted > path-executed\n",
    )
    .unwrap();
    let out = f
        .isolated_command(&executable)
        .env("PATH", path)
        .args(["mail", "rule", "undo", "ledger-row"])
        .output()
        .unwrap();
    assert!(!out.status.success(), "PATH executable must not run");
    assert!(!f.root.join("path-executed").exists());
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cargo install --path mecha-mail"));
    let planted = f.root.join("mecha-mail");
    std::fs::write(&planted, "#!/bin/sh\nprintf planted > executed\n").unwrap();
    std::fs::set_permissions(&planted, std::fs::Permissions::from_mode(0o700)).unwrap();
    let out = f
        .isolated_command(&executable)
        .env("PATH", ".")
        .args(["mail", "rule", "undo", "ledger-row"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(!f.root.join("executed").exists());
    let out = f
        .isolated_command(&executable)
        .env("PATH", f.root.join("empty"))
        .args(["mail", "rule", "test", "notice"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cargo install --path mecha-mail"));
}

#[test]
fn mail_rules_off_preserves_store_reads_and_revocation_but_blocks_active_verbs() {
    let f = Fixture::new();
    let rule = f.rule();
    std::fs::write(
        f.root.join("home/config.toml"),
        "[features]\nmail=false\nmail_rules=false\n",
    )
    .unwrap();
    success(&f.run(&["list", "--json"]));
    success(&f.run(&["show", "notice", "--json"]));
    success(&f.run(&["ledger", "--json"]));
    success(&f.run(&["pause-all"]));
    success(&f.run(&["mode", "notice", "mailbox", "off"]));
    for args in [
        vec!["mode", "notice", "mailbox", "auto", "--yes"],
        vec!["undo", "row"],
        vec!["test", "notice"],
        vec!["add", rule.directory.to_str().unwrap()],
    ] {
        let result = f.run(&args);
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("mail_rules"));
    }
}

#[test]
fn mail_rules_classify_hook_failure_keeps_classify_stdout_exit_and_dry_run_unchanged() {
    let f = Fixture::new();
    let executable = f.isolated_binary();
    f.fake("bin", "hook-child", 23);
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/classify_empty.py");
    let workspace = f.root.join("work");
    std::fs::create_dir(&workspace).unwrap();
    let configure = |enabled: bool| {
        std::fs::write(f.root.join("home/config.toml"), format!(
        "[features]\nmail=true\nmail_rules={enabled}\n[[mcp]]\nname='fixture'\ncommand='python3'\nargs=[{}]\n",
        serde_json::to_string(&fixture).unwrap())).unwrap()
    };
    let run = |extra: &[&str]| {
        f.isolated_command(&executable)
            .args([
                "--workspace",
                workspace.to_str().unwrap(),
                "mail",
                "classify",
            ])
            .args(extra)
            .output()
            .unwrap()
    };
    configure(false);
    let disabled = run(&["--account", "fictional"]);
    success(&disabled);
    assert!(!String::from_utf8_lossy(&disabled.stderr).contains("hook-child"));
    assert_eq!(
        String::from_utf8_lossy(&disabled.stderr)
            .matches("skipping after-classify hook")
            .count(),
        1
    );
    configure(true);
    let failed = run(&["--account", "fictional"]);
    success(&failed);
    assert_eq!(failed.stdout, disabled.stdout);
    let stderr = String::from_utf8_lossy(&failed.stderr);
    assert!(
        stderr.contains("hook-child\nrules\napply\n--after-classify\n--account\nfictional\n"),
        "{stderr}"
    );
    assert!(stderr.contains("1 attempted, 1 failed"));
    let dry = run(&["--dry-run"]);
    success(&dry);
    assert!(!String::from_utf8_lossy(&dry.stderr).contains("hook-child"));
    assert!(!String::from_utf8_lossy(&dry.stderr).contains("after classify:"));
}

#[test]
fn mail_rules_classify_hook_observes_durable_verdicts_and_preserves_summary_on_failure() {
    let f = Fixture::new();
    let executable = f.isolated_binary();
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/classify_empty.py");
    let workspace = f.root.join("work");
    std::fs::create_dir(&workspace).unwrap();
    let configure = |enabled: bool| {
        std::fs::write(f.root.join("home/config.toml"), format!(
        "default_provider='fixture'\n[providers.fixture]\nkind='openai-compatible'\nmodel='fictional'\nbase_url='http://localhost:1'\n[features]\nmail=true\nmail_rules={enabled}\n[[mcp]]\nname='fixture'\ncommand='python3'\nargs=[{}, '--bulk']\n",
        serde_json::to_string(&fixture).unwrap())).unwrap()
    };
    let run = || {
        f.isolated_command(&executable)
            .args([
                "--workspace",
                workspace.to_str().unwrap(),
                "mail",
                "classify",
                "--account",
                "fictional",
                "--force",
            ])
            .output()
            .unwrap()
    };
    configure(false);
    let before = run();
    success(&before);
    let fake = f.root.join("bin/mecha-mail");
    std::fs::write(&fake, r#"#!/usr/bin/env python3
import json, os, pathlib, sys
records = [json.loads(p.read_text()) for p in (pathlib.Path(os.environ['MECHA_HOME']) / 'mail-triage').glob('*.json')]
assert any(r['thread_id'] == 'fictional-thread' and r['state'] == 'classified' and r['verdict'] for r in records)
print('verdict durable before child', file=sys.stderr)
sys.exit(29)
"#).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
    // Remove the earlier verdict so this assertion cannot pass on old data.
    std::fs::remove_dir_all(f.root.join("home/mail-triage")).unwrap();
    configure(true);
    let after = run();
    success(&after);
    assert_eq!(after.stdout, before.stdout);
    let stderr = String::from_utf8_lossy(&after.stderr);
    assert!(stderr.contains("verdict durable before child"), "{stderr}");
    assert!(stderr.contains("1 attempted, 1 failed"));
    let store =
        mecha_core::mail_triage::TriageStore::open_existing(f.root.join("home/mail-triage"))
            .unwrap();
    assert_eq!(
        store.get("fictional", "fictional-thread").unwrap().state,
        "classified"
    );
}
