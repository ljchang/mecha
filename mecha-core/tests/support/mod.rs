//! Shared helpers for the tests that need a real backend.
//!
//! These tests execute things — a container, an interpreter — so they can only
//! run where those exist. Skipping is the right default for a developer laptop
//! and the wrong one for CI, where a silently skipped test reads exactly like a
//! passing one. `MECHA_TEST_REQUIRE_BACKENDS=1` turns every skip into a
//! failure, so a machine that is supposed to have docker says so out loud.

#![allow(dead_code)]

use std::path::PathBuf;
use std::process::Command;

/// True when the caller should return early. Prints why, and fails instead of
/// skipping when the environment says these backends are mandatory.
pub fn unavailable(what: &str, present: bool) -> bool {
    if present {
        return false;
    }
    assert!(
        std::env::var("MECHA_TEST_REQUIRE_BACKENDS").is_err(),
        "{what} is unavailable, and MECHA_TEST_REQUIRE_BACKENDS is set"
    );
    eprintln!("SKIPPED: {what} is unavailable");
    true
}

pub fn docker_available() -> bool {
    Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Whether an image is already local. Pulling inside a test would make it slow
/// and network-dependent, so a missing image is a skip rather than a fetch.
pub fn docker_image_present(image: &str) -> bool {
    Command::new("docker")
        .args(["image", "inspect", image])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn bwrap_present() -> bool {
    Command::new("bwrap")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn python3_available() -> bool {
    Command::new("python3")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// The nosy MCP server fixture, in this crate's test tree.
pub fn fixture_server() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("nosy_mcp_server.py")
}

pub fn tmpdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mecha-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    // Canonicalised because the sandbox binds absolute paths, and on macOS
    // /tmp is a symlink — a mismatch there shows up as a confusing mount error.
    dir.canonicalize().unwrap()
}
