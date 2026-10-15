//! Records where this build came from, as `MECHA_BUILD_SOURCE`, so mecha can
//! install its own servers from the same source (`own_build.rs`) and say
//! which build it is (`mecha --version`):
//!
//! - `git:<sha>` — built from a git checkout: a `cargo install --git`, or a
//!   build in a clone. The commit is `git rev-parse HEAD`.
//! - `registry:<version>` — built from the published crate, which carries no
//!   `.git` but `.cargo_vcs_info.json` beside its manifest; the version is
//!   this package's.
//!
//! Neither: nothing is set, and the source reads unknown.

use std::path::Path;
use std::process::Command;

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let dir = Path::new(&dir);
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    // A published crate first: its vcs file is the published commit, and a
    // registry source is what its sibling crates must come from too. Read as
    // text, so the build script needs no dependency.
    if dir.join(".cargo_vcs_info.json").is_file() {
        println!("cargo:rustc-env=MECHA_BUILD_SOURCE=registry:{version}");
        return;
    }
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    if let Some(sha) = git(&["rev-parse", "HEAD"]).filter(|s| s.len() == 40) {
        println!("cargo:rustc-env=MECHA_BUILD_SOURCE=git:{sha}");
        // Rebuilt when the checkout moves, not only when a file of this
        // crate changes: a commit elsewhere in the workspace is a new source.
        for path in ["HEAD", "packed-refs"] {
            if let Some(p) = git(&["rev-parse", "--git-path", path]) {
                println!("cargo:rerun-if-changed={}", dir.join(p).display());
            }
        }
        if let Some(head) = git(&["symbolic-ref", "-q", "HEAD"]) {
            if let Some(p) = git(&["rev-parse", "--git-path", &head]) {
                println!("cargo:rerun-if-changed={}", dir.join(p).display());
            }
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}
