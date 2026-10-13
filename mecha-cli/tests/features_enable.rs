//! Step 7a-3 of `docs/FEATURES-DESIGN.md` (§10.2 item 3, F7): `mecha features
//! enable` offers what the feature needs and this machine lacks, and without
//! a terminal it installs nothing and writes nothing.
//!
//! Driven through the real binary with stdin closed — the non-terminal case
//! — against an isolated home, where layout's environment is missing, so the
//! refusal fires with no network and no model. The negative is not vacuous:
//! `--no-install` and a feature with nothing to install both still write the
//! switch, so a command that refused everything would fail here.

use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Home(PathBuf);

impl Home {
    fn new(tag: &str) -> Home {
        let dir = std::env::temp_dir().join(format!("mecha-enable-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("home")).unwrap();
        Home(dir)
    }

    fn config(&self) -> String {
        std::fs::read_to_string(self.0.join("home/config.toml")).unwrap_or_default()
    }

    fn enable(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(["features", "enable"])
            .args(args)
            .env("MECHA_HOME", self.0.join("home"))
            .env("HOME", self.0.join("home"))
            .env("HF_HUB", self.0.join("hub"))
            .env("MECHA_SESSION_KIND", "test")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn without_a_terminal_an_enable_that_would_install_refuses_and_writes_nothing() {
    let home = Home::new("refuse");
    let out = home.enable(&["documents"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "it must refuse: {err}");
    assert!(
        err.contains("--no-install"),
        "the refusal names the way through: {err}"
    );
    assert!(
        err.contains("layout"),
        "and what it would have installed: {err}"
    );
    assert!(
        !home.config().contains("documents = true"),
        "nothing written:\n{}",
        home.config()
    );
    assert!(!home.0.join("home/sidecars").exists(), "nothing installed");
}

#[test]
fn no_install_writes_the_switch_alone() {
    let home = Home::new("no-install");
    let out = home.enable(&["documents", "--no-install"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        home.config().contains("documents = true"),
        "{}",
        home.config()
    );
    assert!(!home.0.join("home/sidecars").exists(), "nothing installed");
}

/// A feature with nothing mecha can install writes its switch without a
/// terminal, as `enable` always did: the refusal bites only when there is
/// something to install.
#[test]
fn with_nothing_to_install_the_switch_is_written_as_before() {
    let home = Home::new("nothing");
    let out = home.enable(&["messages"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        home.config().contains("messages = true"),
        "{}",
        home.config()
    );
}

/// An install whose model is gone from the hub — a cleared cache leaves its
/// link dangling — is offered again, not passed over as installed: so the
/// message extraction gives (`mecha features enable documents`) is true.
#[cfg(unix)]
#[test]
fn an_installed_layout_whose_model_is_gone_is_offered_again() {
    let home = Home::new("dangling");
    // MECHA_HOME is home/, so mecha's tree is home/sidecars/layout.
    let mecha_home = home.0.join("home");
    let tree = mecha_home.join("sidecars/layout");
    std::fs::create_dir_all(tree.join("venv/bin")).unwrap();
    std::fs::write(tree.join("venv/bin/python"), "").unwrap();
    std::os::unix::fs::symlink(home.0.join("hub/gone"), tree.join("PP-DocLayoutV3.onnx")).unwrap();
    let manifest = format!(
        r#"{{"entries":[{{"sidecar":"layout","incomplete":false,"wrote":["{}","{}","{}"]}}]}}"#,
        tree.display(),
        tree.join("venv").display(),
        tree.join("PP-DocLayoutV3.onnx").display()
    );
    std::fs::write(mecha_home.join("sidecars/manifest.json"), manifest).unwrap();
    let out = home.enable(&["documents"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "the dangling model is offered, so a non-tty enable refuses: {err}"
    );
    assert!(err.contains("layout"), "{err}");
}

/// `features enable mail docs` wires both servers through the real binary:
/// an `[[mcp]]` entry each, running the program by its absolute path, with
/// `untrusted_input` forced — and one `[outbox]` staging both features' calls
/// (the owner, 2026-10-12; review of #669). A program found only through a
/// relative PATH entry is never recorded. The negative is not vacuous: the
/// same programs on an absolute PATH entry are wired.
#[test]
fn enable_wires_mail_and_docs_with_one_outbox() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new("wire");
    let bin = home.0.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for name in ["mecha-mail", "mecha-docs"] {
        let p = bin.join(name);
        std::fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let run = |path: &str, cwd: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(["features", "enable", "mail", "docs", "--no-install"])
            .env("MECHA_HOME", home.0.join("home"))
            .env("HOME", home.0.join("home"))
            .env("PATH", path)
            .env("MECHA_SESSION_KIND", "test")
            .current_dir(cwd)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };

    // Relative only: found nowhere absolute, so nothing is written.
    let out = run("bin:/usr/bin:/bin", &home.0);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!home.config().contains("[[mcp]]"), "{}", home.config());

    // Absolute: both wired, one outbox.
    let path = format!("{}:/usr/bin:/bin", bin.display());
    let out = run(&path, &home.0);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let cfg: toml::Value = toml::from_str(&home.config()).unwrap();
    let servers = cfg["mcp"].as_array().expect("[[mcp]] entries");
    for (name, program) in [("mail", "mecha-mail"), ("docs", "mecha-docs")] {
        let e = servers
            .iter()
            .find(|e| e["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("no {name} entry: {cfg}"));
        assert_eq!(
            e["command"].as_str(),
            Some(bin.join(program).to_str().unwrap())
        );
        assert_eq!(e["capabilities"]["untrusted_input"].as_bool(), Some(true));
    }
    let tools: Vec<&str> = cfg["outbox"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t.as_str())
        .collect();
    for t in [
        "mail__mail_send",
        "mail__mail_reply",
        "docs__docs_append",
        "docs__sheets_write",
    ] {
        assert!(tools.contains(&t), "{t} not staged: {tools:?}");
    }
}

/// An operator rule on a send `enable mail` would stage: a `prompt` beside
/// the route refuses every start (`setup::live_rules`), and a `forbid` would
/// be unreached — the send a draft a person can release — so no `[outbox]` is
/// written for either and the conflict is named; the owner's rule is never
/// overridden (review of #669, passes 3 and 4). The entry is still wired, and
/// nothing is said about an `[outbox]` "left as it is", because there is none.
#[test]
fn enable_does_not_stage_a_send_an_operator_rule_judges() {
    use std::os::unix::fs::PermissionsExt;
    for decision in ["prompt", "forbid"] {
        let home = Home::new(&format!("ruled-{decision}"));
        let bin = home.0.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let p = bin.join("mecha-mail");
        std::fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            home.0.join("home/config.toml"),
            format!("[[rule]]\ntool = \"mail__mail_send\"\ndecision = \"{decision}\"\n"),
        )
        .unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(["features", "enable", "mail", "--no-install"])
            .env("MECHA_HOME", home.0.join("home"))
            .env("HOME", home.0.join("home"))
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("MECHA_SESSION_KIND", "test")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "{decision}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !home.config().contains("[outbox]"),
            "{decision}: {}",
            home.config()
        );
        assert!(
            home.config().contains("[[mcp]]"),
            "{decision}: {}",
            home.config()
        );
        assert!(
            said.contains("mail__mail_send") && said.contains("no [outbox]"),
            "{decision}: {said}"
        );
        assert!(!said.contains("left as it is"), "{decision}: {said}");
    }
}
