//! The real binary against a scratch mecha home: `memory-session init` with
//! no manifest makes the home store, a second one refuses, and `features`
//! reads the row from the files it left.
#![cfg(feature = "memory-prototype")]

use std::{path::Path, process::Command};

fn mecha(home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mecha"))
        .args(args)
        .env("MECHA_HOME", home)
        .env("MECHA_SESSION_KIND", "test")
        .env_remove("MECHA_LOG")
        .current_dir(home.parent().unwrap())
        .output()
        .unwrap()
}

fn memory_row(home: &Path) -> serde_json::Value {
    let out = mecha(home, &["features", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rows: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    rows.as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "memory")
        .expect("a memory row")
        .clone()
}

#[test]
fn init_without_a_manifest_makes_the_home_store_once_and_features_reads_it() {
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let root = std::env::temp_dir().join(format!("mecha-memhome-cli-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let scratch = Scratch(root.canonicalize().unwrap());
    let root = scratch.0.clone();
    // Spelled as the real home is, so the store's own path rule is the one
    // under test.
    let home = root.join(".mecha");
    std::fs::create_dir_all(&home).unwrap();

    assert_eq!(memory_row(&home)["state"], "off");

    let out = mecha(&home, &["memory-session", "init"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["status"], "initialized");
    for file in [
        "memory/store/agent.memory-prototype.sqlite",
        "memory/key/agent.key",
        "memory/authority/identity",
    ] {
        assert!(home.join(file).is_file(), "{file}");
    }
    // Encrypted: the file does not open as plain SQLite.
    let head = std::fs::read(home.join("memory/store/agent.memory-prototype.sqlite")).unwrap();
    assert!(!head.starts_with(b"SQLite format 3"));

    let key = std::fs::read(home.join("memory/key/agent.key")).unwrap();
    let again = mecha(&home, &["memory-session", "init"]);
    assert!(!again.status.success());
    assert_eq!(
        std::fs::read(home.join("memory/key/agent.key")).unwrap(),
        key
    );

    // Off still — the store exists, the owner has not switched it on — and
    // on once they have.
    assert_eq!(memory_row(&home)["state"], "off");
    let enable = mecha(&home, &["features", "enable", "memory"]);
    assert!(
        enable.status.success(),
        "{}",
        String::from_utf8_lossy(&enable.stderr)
    );
    assert_eq!(memory_row(&home)["state"], "on");

    // The synthetic-fixture surface cannot reach it: `mecha memory` holds the
    // right key and names the right file, and is still not the admitted
    // opener (review of #862).
    let store = home.join("memory/store/agent.memory-prototype.sqlite");
    let before = std::fs::read(&store).unwrap();
    let fixture = mecha(
        &home,
        &[
            "memory",
            "--experimental",
            "--db",
            store.to_str().unwrap(),
            "--key-file",
            home.join("memory/key/agent.key").to_str().unwrap(),
            "inspect",
        ],
    );
    assert!(!fixture.status.success());
    assert_eq!(std::fs::read(&store).unwrap(), before);
}
