//! `mecha workflow commit --expectation/--consequence` through the real
//! binary (review of #302, picked up in 1f-2): the two fields the one
//! commitment record absorbed reach the stored workflow, and the 1–4096-byte
//! bound the owner-evidence commitment always had refuses an empty or an
//! oversized value — writing nothing — rather than recording it.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Root(PathBuf);

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn mecha(home: &Path, work: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mecha"))
        .args(args)
        .env("MECHA_HOME", home)
        .env("MECHA_SESSION_KIND", "test")
        .env_remove("MECHA_SESSION_DIR")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .current_dir(work)
        .output()
        .unwrap()
}

fn json(out: &Output, what: &str) -> Value {
    assert!(
        out.status.success(),
        "{what} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "{what}: not JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// The commitment as the store holds it, read from the file the store
/// wrote rather than from the command's own echo.
fn stored(home: &Path, id: &str) -> Value {
    let raw = std::fs::read_to_string(home.join("workflows").join(format!("{id}.json")))
        .unwrap_or_else(|e| panic!("no stored workflow `{id}`: {e}"));
    serde_json::from_str::<Value>(&raw).unwrap()["commitment"].clone()
}

#[test]
fn commit_flags_reach_the_record_and_the_byte_bound_refuses_empty_and_oversized() {
    let root = Root(std::env::temp_dir().join(format!(
        "mecha-workflow-flags-{}",
        mecha_core::session::Session::new_id()
    )));
    let home = root.0.join("home");
    let work = root.0.join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();

    let added = json(
        &mecha(
            &home,
            &work,
            &["workflow", "add", "Prepare the reading-group agenda"],
        ),
        "workflow add",
    );
    let id = added["id"].as_str().expect("an id").to_string();
    let dates = [
        "--party",
        "the reading group",
        "--source",
        "owner instruction",
        "--due",
        "2026-10-15T17:00:00Z",
        "--follow-up",
        "2026-10-14T09:00:00Z",
    ];
    let commit = |extra: &[&str]| {
        let mut args = vec!["workflow", "commit", id.as_str()];
        args.extend_from_slice(&dates);
        args.extend_from_slice(extra);
        mecha(&home, &work, &args)
    };

    // Both flags reach the record the store wrote.
    json(
        &commit(&[
            "--expectation",
            "the agenda before the Friday meeting",
            "--consequence",
            "the group meets unprepared",
        ]),
        "workflow commit",
    );
    let c = stored(&home, &id);
    assert_eq!(c["expectation"], "the agenda before the Friday meeting");
    assert_eq!(c["consequence"], "the group meets unprepared");
    assert_eq!(c["party"], "the reading group");
    assert_eq!(c["due_at"], "2026-10-15T17:00:00Z");

    // Refused, and nothing written: an empty (all-space) value, an
    // oversized one on either flag, and the bound's edge accepted.
    let before = stored(&home, &id);
    let oversized = "x".repeat(4097);
    for (flag, value) in [
        ("--expectation", "   "),
        ("--consequence", ""),
        ("--expectation", oversized.as_str()),
        ("--consequence", oversized.as_str()),
    ] {
        let out = commit(&[flag, value]);
        assert!(
            !out.status.success(),
            "{flag} of {} bytes should be refused",
            value.len()
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("1–4096 bytes"),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(stored(&home, &id), before, "a refusal writes nothing");
    }
    let edge = "y".repeat(4096);
    json(&commit(&["--expectation", edge.as_str()]), "4096 bytes");
    assert_eq!(stored(&home, &id)["expectation"], edge.as_str());
}
