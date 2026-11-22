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
