//! CI builds with the Rust a developer builds with: `rust-toolchain.toml` pins
//! one exact version, and every workflow installs from that file rather than
//! floating on `stable`. Before the pin, CI's `stable` ran two versions ahead
//! of the machine the code was written on, and a lint added in between
//! (`clippy::chunks_exact_to_as_chunks`) reached a PR as a red check nobody
//! had seen locally (#551).
//!
//! And the MSRV arm — the oldest Rust a `cargo install` user may have — names
//! the version the workspace manifest promises, so the two cannot drift.

use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo().join(rel)).unwrap_or_else(|e| panic!("reading {rel}: {e}"))
}

/// The value of `key = "…"` on its first line in `text`.
fn quoted(text: &str, key: &str) -> String {
    text.lines()
        .map(str::trim)
        .find_map(|l| {
            l.strip_prefix(key)?
                .trim()
                .strip_prefix('=')?
                .trim()
                .strip_prefix('"')?
                .split('"')
                .next()
        })
        .unwrap_or_else(|| panic!("no {key} = \"…\""))
        .to_string()
}

#[test]
fn the_toolchain_is_one_exact_version() {
    let channel = quoted(&read("rust-toolchain.toml"), "channel");
    let parts: Vec<&str> = channel.split('.').collect();
    assert!(
        parts.len() == 3 && parts.iter().all(|p| p.parse::<u32>().is_ok()),
        "rust-toolchain.toml pins `{channel}`, which floats; name an exact x.y.z"
    );
}

#[test]
fn every_workflow_installs_the_pinned_toolchain() {
    for wf in std::fs::read_dir(repo().join(".github/workflows")).unwrap() {
        let path = wf.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        for floating in [
            "rust-toolchain@stable",
            "rust-toolchain@nightly",
            "rust-toolchain@beta",
        ] {
            assert!(
                !text.contains(floating),
                "{name} installs `{floating}`; install the pinned toolchain with \
                 `rustup toolchain install` (no arguments) instead"
            );
        }
        // A workflow that builds Rust says where its toolchain came from.
        if text.contains("cargo ") {
            assert!(
                text.contains("rustup toolchain install"),
                "{name} runs cargo without installing the pinned toolchain"
            );
        }
    }
}

#[test]
fn the_msrv_arm_tests_the_promised_version() {
    let promised = quoted(&read("Cargo.toml"), "rust-version");
    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains(&format!("rust: \"{promised}\"")),
        "Cargo.toml promises rust-version {promised}, and ci.yml's MSRV arm does not test it"
    );
}
