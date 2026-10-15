//! Records where this build came from, as `MECHA_BUILD_SOURCE`, so mecha can
//! install its own servers from the same source (`own_build.rs`) and say
//! which build it is (`mecha --version`):
//!
//! - `git:<sha>` — built from a git checkout whose commit its remote has: a
//!   `cargo install --git`, or a clone whose HEAD a remote branch contains.
//! - `local:<sha>` — a clone's commit no remote branch holds yet: the
//!   matching servers can only come from this checkout.
//! - `registry:<version>` — built from the published crate, which carries no
//!   `.git` but `.cargo_vcs_info.json` beside its manifest; the version is
//!   this package's.
//!
//! Neither: nothing is set, and the source reads unknown.

use std::path::Path;
use std::process::Command;

include!("src/build_source.rs");

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
        // A commit cargo fetched (`cargo install --git`, under its own
        // checkouts) is one its remote has; a local clone's HEAD may not be
        // pushed, and a `cargo install --git --rev` of it would fail minutes
        // in. So a clone's commit is `git:` only when a remote branch holds
        // it, else `local:` (review of #699).
        let fetched = dir.components().any(|c| c.as_os_str() == "checkouts")
            && dir.components().any(|c| c.as_os_str() == "git");
        let repo = std::env::var("CARGO_PKG_REPOSITORY").unwrap_or_default();
        // A branch of mecha's own repository holds it: a remote-tracking
        // branch (not a pull request's ref) whose remote is that repository.
        let pushed = fetched
            || git(&[
                "for-each-ref",
                "--contains",
                "HEAD",
                "--format=%(refname)",
                "refs/remotes",
            ])
            .is_some_and(|refs| {
                refs.lines().filter_map(remote_of).any(|(remote, _)| {
                    git(&["remote", "get-url", remote]).is_some_and(|url| same_repo(&url, &repo))
                })
            });
        let kind = if pushed { "git" } else { "local" };
        println!("cargo:rustc-env=MECHA_BUILD_SOURCE={kind}:{sha}");
        // Rebuilt when the checkout moves, not only when a file of this
        // crate changes. Only paths that exist: cargo reads a missing one
        // as changed, and would rebuild every time (review of #699).
        let mut watch = vec![git(&["rev-parse", "--git-path", "HEAD"])];
        watch.push(git(&["rev-parse", "--git-path", "packed-refs"]));
        // And when a push lands: a commit built before it read `local:`.
        watch.push(git(&["rev-parse", "--git-path", "refs/remotes"]));
        if let Some(head) = git(&["symbolic-ref", "-q", "HEAD"]) {
            watch.push(git(&["rev-parse", "--git-path", &head]));
        }
        for p in watch.into_iter().flatten().map(|p| dir.join(p)) {
            if p.exists() {
                println!("cargo:rerun-if-changed={}", p.display());
            }
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}
