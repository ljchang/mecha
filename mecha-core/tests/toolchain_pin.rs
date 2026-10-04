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
        // The setup actions at all, not only a floating tag of one:
        // `dtolnay/rust-toolchain@master` with `toolchain: stable` is the
        // spelling this repository had, and it floats without ever matching
        // `@stable`. And no cargo line picks its own toolchain (`cargo +x`).
        // Between them, a job added later cannot set Rust up some other way
        // while the file's one `rustup toolchain install` satisfies the
        // check below.
        for floating in ["dtolnay/rust-toolchain", "actions-rs/toolchain", "cargo +"] {
            assert!(
                !text.contains(floating),
                "{name} sets Rust up with `{floating}`, which can float; install the \
                 pinned toolchain with `rustup toolchain install` (no arguments) instead"
            );
        }
        // A workflow that builds Rust says where its toolchain came from.
        // The two Claude workflows are held back for one PR:
        // `claude-code-action` refuses to run on a PR that edits its own
        // workflow, so the pin lands where it can be reviewed and their
        // install steps follow in a PR of their own, which removes this.
        let pending = ["claude.yml", "claude-code-review.yml"];
        if text.contains("cargo ") && !pending.contains(&name.as_str()) {
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
    // And the arm must *select* it. With rust-toolchain.toml in the tree, an
    // arm that names no toolchain does not fail — it builds on the pin while
    // still labelled with the MSRV, the "testing one version twice" collapse
    // CONTRIBUTING.md records as the objection to a toolchain file.
    assert!(
        // The export into the job's environment itself — the version line
        // beside it names the variable too, and would satisfy a looser match.
        ci.contains(r#"echo "RUSTUP_TOOLCHAIN=${{ matrix.rust }}" >> "$GITHUB_ENV""#),
        "ci.yml's MSRV arm does not export RUSTUP_TOOLCHAIN, so rust-toolchain.toml \
         wins and the arm tests the pin rather than rust-version {promised}"
    );
}
