//! Real unattended entrypoint: a feature-off pass and lease contention need no network.
use mecha_core::mail_rules::RuleStore;
use std::{fs, process::Command};

#[test]
fn direct_rule_reads_and_undo_refuse_disabled_switches_before_opening_stores() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir(&home).unwrap();
    for config in [
        "",
        "[features]\nmail=true\nmail_rules=false\n",
        "[features]\nmail=false\nmail_rules=true\n",
    ] {
        fs::write(home.join("config.toml"), config).unwrap();
        for verb in ["undo", "test"] {
            let out = Command::new(env!("CARGO_BIN_EXE_mecha-mail"))
                .current_dir(root.path())
                .env("MECHA_HOME", &home)
                .env("MECHA_MAIL_DIR", root.path().join("missing-mail"))
                .args(["rules", verb, "fictional"])
                .output()
                .unwrap();
            assert!(!out.status.success());
            let diagnostic = String::from_utf8_lossy(&out.stderr);
            assert!(
                diagnostic.contains("mail_rules is off or requires mail"),
                "{verb}: {diagnostic}"
            );
            assert!(!home.join("mail-rules").exists());
        }
    }
}

#[test]
fn every_direct_rule_subcommand_refuses_unreadable_global_configuration_first() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("config.toml")).unwrap();
    for args in [
        vec!["rules", "undo", "fictional"],
        vec!["rules", "test", "fictional"],
        vec!["rules", "apply", "--headers"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_mecha-mail"))
            .current_dir(root.path())
            .env("MECHA_HOME", &home)
            .env("MECHA_MAIL_DIR", root.path().join("missing-mail"))
            .args(&args)
            .output()
            .unwrap();
        assert!(!out.status.success());
        let diagnostic = String::from_utf8_lossy(&out.stderr);
        assert!(diagnostic.contains("config.toml"), "{args:?}: {diagnostic}");
        assert!(
            !diagnostic.contains("mail-rules"),
            "store opened before config: {diagnostic}"
        );
        assert!(!home.join("mail-rules").exists());
    }
}

#[test]
fn mail_rules_off_does_not_open_a_store_or_accounts_and_ignores_project_switches() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("mecha.toml"),
        "[features]\nmail=true\nmail_rules=true\n",
    )
    .unwrap();
    let home = root.path().join("home");
    fs::create_dir(&home).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_mecha-mail"))
        .current_dir(root.path())
        .env("MECHA_HOME", &home)
        .env("MECHA_MAIL_DIR", root.path().join("missing-accounts"))
        .args(["rules", "apply", "--headers"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr)
            .matches("skipping mail rules apply")
            .count(),
        1
    );
    assert!(fs::read_dir(home).unwrap().next().is_none());
}

#[test]
fn mail_rules_apply_skips_only_typed_engine_contention() {
    use std::os::unix::io::AsRawFd;
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let mail = root.path().join("mail");
    let store = RuleStore::open(RuleStore::root_under(&home)).unwrap();
    fs::create_dir_all(mail.join("fictional")).unwrap();
    fs::write(
        home.join("config.toml"),
        "[features]\nmail=true\nmail_rules=true\n",
    )
    .unwrap();
    fs::write(
        mail.join("accounts.toml"),
        "[[account]]\nname='fictional'\nprovider='google'\n",
    )
    .unwrap();
    fs::write(mail.join("fictional/oauth.json"), r#"{"client_id":"fixture","access_token":"fictional","refresh_token":"fictional","expires_at":0,"account":"owner@example.edu"}"#).unwrap();
    let lock = fs::File::create(store.root().join(".engine.lock")).unwrap();
    // SAFETY: this test owns the descriptor until the subprocess has exited.
    assert_eq!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_mecha-mail"))
            .current_dir(root.path())
            .env("MECHA_HOME", &home)
            .env("MECHA_MAIL_DIR", &mail)
            .args(["rules", "apply", "--headers"])
            .output()
            .unwrap()
    };
    let skipped = run();
    assert!(
        skipped.status.success(),
        "{}",
        String::from_utf8_lossy(&skipped.stderr)
    );
    assert!(String::from_utf8_lossy(&skipped.stderr)
        .contains("another mail rules apply is running; skipping"));
    drop(lock);
    fs::remove_file(store.root().join(".engine.lock")).unwrap();
    fs::create_dir(store.root().join(".engine.lock")).unwrap();
    let failed = run();
    assert!(!failed.status.success());
    assert!(!String::from_utf8_lossy(&failed.stderr).contains("skipping"));
}
