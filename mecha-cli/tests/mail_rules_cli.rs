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
        std::fs::write(root.join("home/config.toml"), "[features]\nmail = true\n").unwrap();
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
