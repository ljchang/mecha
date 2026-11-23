//! `mecha exp` through the real binary refuses every global option it would
//! ignore, and the one environment override it would drop. Its trials run on
//! what each manifest arm names, and on 2026-11-22 `exp run <name> -p
//! <provider>` parsed — the options are global — and ran every trial on the
//! config's default provider instead.
//! The refusal comes before anything else: before the store is opened, so
//! an experiment that does not exist still answers with the refusal, and
//! before the router is held or observed on the named provider's account.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Root(PathBuf);

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn mecha(home: &Path, work: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mecha"))
        .args(args)
        .env_remove("MECHA_EFFORT")
        .env_remove("MECHA_PROVIDER")
        .env_remove("MECHA_MODEL")
        .envs(env.iter().copied())
        .env("MECHA_HOME", home)
        .env("MECHA_SESSION_KIND", "test")
        .env_remove("MECHA_SESSION_DIR")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .current_dir(work)
        .output()
        .unwrap()
}

const REFUSAL: &str = "`mecha exp` takes no global option";
const DROPPED: &str = "`MECHA_EFFORT` is set";

#[test]
fn exp_refuses_a_global_option_it_would_ignore_and_only_then() {
    let root = Root(std::env::temp_dir().join(format!(
        "mecha-exp-model-flags-{}",
        mecha_core::session::Session::new_id()
    )));
    let home = root.0.join("home");
    let work = root.0.join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    // A provider nothing listens on: the control line below observes the
    // default provider as any `exp` command does, and must reach no server.
    std::fs::write(
        home.join("config.toml"),
        "default_provider = \"fixture\"\n\
         [providers.fixture]\n\
         kind = \"openai-compatible\"\n\
         base_url = \"http://127.0.0.1:1\"\n\
         model = \"fictional\"\n\
         max_retries = 0\n",
    )
    .unwrap();

    for (args, flag) in [
        (
            &["exp", "run", "no-such-exp", "--dry-run", "-p", "other"][..],
            "--provider/-p",
        ),
        (
            &["-m", "other", "exp", "status", "no-such-exp"],
            "--model/-m",
        ),
        (
            &["exp", "report", "no-such-exp", "--effort", "high"],
            "--effort/-e",
        ),
        (
            &["exp", "export", "no-such-exp", "-s", "be brief"],
            "--system/-s",
        ),
        (
            &["exp", "run", "no-such-exp", "--max-turns", "3"],
            "--max-turns",
        ),
        (&["--no-mcp", "exp", "judge", "no-such-exp"], "--no-mcp"),
    ] {
        let out = mecha(&home, &work, args, &[]);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{args:?} must fail: {err}");
        assert!(
            err.contains(REFUSAL) && err.contains(flag),
            "{args:?} must be refused naming {flag}, not fail elsewhere: {err}"
        );
        assert!(err.contains("[arms.<arm>]"), "{args:?}: {err}");
    }

    // The effort variable a trial would never see is refused too.
    let out = mecha(
        &home,
        &work,
        &["exp", "status", "no-such-exp"],
        &[("MECHA_EFFORT", "high")],
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "MECHA_EFFORT must fail: {err}");
    assert!(err.contains(DROPPED), "refused by name: {err}");

    // The controls: no option, or the model variables `exp` honours and
    // hashes, is not refused — it fails on the missing experiment, which is
    // the next thing it reads.
    for env in [&[][..], &[("MECHA_MODEL", "other-fictional")]] {
        let out = mecha(&home, &work, &["exp", "status", "no-such-exp"], env);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            !err.contains(REFUSAL) && !err.contains(DROPPED),
            "{env:?}: no refusal: {err}"
        );
        assert!(
            err.contains("no-such-exp"),
            "{env:?}: the control must fail on the experiment it named, not earlier: {err}"
        );
    }
}

/// `exp new` writes the options a manifest can say into every arm of the
/// stored design, under a new name, and refuses the rest; with no option it
/// stores the file as written.
#[test]
fn exp_new_writes_the_model_flags_into_a_new_experiment() {
    let root = Root(std::env::temp_dir().join(format!(
        "mecha-exp-new-flags-{}",
        mecha_core::session::Session::new_id()
    )));
    let home = root.0.join("home");
    let work = root.0.join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    let file = "# why this design exists\n\
        name = \"levers\"\n\
        control = \"full\"\n\
        split_seed = 7\n\
        seeds = [1]\n\
        repetitions = 1\n\
        [tasks]\n\
        cases = \"eval/cases.jsonl\"\n\
        fixture = \"eval/workspace\"\n\
        [arms.full]\n\
        preset = \"full\"\n\
        [arms.quiet]\n\
        overrides = [\"max_turns=20\"]\n\
        [arms.quiet.prediction]\n\
        metric = \"turns\"\n\
        rationale = \"fewer turns\"\n";
    std::fs::write(work.join("m.toml"), file).unwrap();
    std::fs::write(
        home.join("config.toml"),
        "default_provider = \"fixture\"\n\
         [providers.fixture]\n\
         kind = \"openai-compatible\"\n\
         base_url = \"http://127.0.0.1:1\"\n\
         model = \"fictional\"\n\
         [providers.other]\n\
         kind = \"openai-compatible\"\n\
         base_url = \"http://127.0.0.1:1\"\n\
         model = \"other-fictional\"\n",
    )
    .unwrap();
    let ok = |args: &[&str]| {
        let out = mecha(&home, &work, args, &[]);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let refused = |args: &[&str]| {
        let out = mecha(&home, &work, args, &[]);
        assert!(!out.status.success(), "{args:?} must fail");
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    let stored = |name: &str| {
        std::fs::read_to_string(home.join("experiments").join(name).join("manifest.toml")).unwrap()
    };

    let err = refused(&["exp", "new", "m.toml", "-p", "other"]);
    assert!(err.contains("--name <new-name>"), "{err}");
    let err = refused(&[
        "exp", "new", "m.toml", "--name", "b", "-p", "other", "-w", "/tmp",
    ]);
    assert!(
        err.contains(REFUSAL) && err.contains("--workspace/-w"),
        "{err}"
    );
    assert!(
        !home.join("experiments").join("b").exists(),
        "nothing stored"
    );

    let said = ok(&[
        "exp", "new", "m.toml", "--name", "levers-b", "-p", "other", "-e", "low", "--no-mcp",
    ]);
    assert!(said.contains("every arm: provider = \"other\""), "{said}");
    let text = stored("levers-b");
    assert!(
        text.starts_with("# Written by `mecha exp new --name levers-b`"),
        "{text}"
    );
    assert!(text.contains("# why this design exists"), "{text}");
    let m = mecha_core::experiment::Manifest::parse(&text).unwrap();
    assert_eq!(m.name, "levers-b");
    for (arm, a) in &m.arms {
        assert_eq!(a.provider.as_deref(), Some("other"), "{arm}");
        assert!(a.overrides.contains(&"effort=low".to_string()), "{arm}");
        assert_eq!(a.levers_off, ["mcp"], "{arm}");
    }
    assert_eq!(m.arms["quiet"].overrides, ["max_turns=20", "effort=low"]);

    // A flag that contradicts an arm is refused, not written over it.
    let err = refused(&["exp", "new", "m.toml", "--name", "c", "--max-turns", "30"]);
    assert!(err.contains("already overrides max_turns to `20`"), "{err}");
    // One that repeats the treatment's only difference into the control
    // would run the design as A/A: refused.
    let err = refused(&["exp", "new", "m.toml", "--name", "c", "--max-turns", "20"]);
    assert!(err.contains("would run under one condition"), "{err}");
    // A provider the config does not name is refused when written, not
    // when the first trial fails on it.
    let err = refused(&["exp", "new", "m.toml", "--name", "c", "-p", "othr"]);
    assert!(
        err.contains("`-p othr` names no provider") && err.contains("fixture, local, other"),
        "{err}"
    );
    // `MECHA_EFFORT` on `new` points at the flag that writes it.
    let out = mecha(
        &home,
        &work,
        &["exp", "new", "m.toml", "--name", "c"],
        &[("MECHA_EFFORT", "low")],
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && err.contains("pass `-e <level>`"),
        "{err}"
    );
    assert!(
        !home.join("experiments").join("c").exists(),
        "nothing stored"
    );

    // `exp run` still takes none of them, and points at `exp new`.
    let err = refused(&["exp", "run", "levers-b", "-p", "other"]);
    assert!(
        err.contains(REFUSAL) && err.contains("mecha exp new <file>"),
        "{err}"
    );

    // No option: the file as written.
    ok(&["exp", "new", "m.toml"]);
    assert_eq!(stored("levers"), file);
}
