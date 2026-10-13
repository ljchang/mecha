//! The knowledge graph's MCP server, built from its pinned commit
//! (`docs/FEATURES-DESIGN.md` §13): what `mecha features enable graph`
//! installs when no `mecha-graph-mcp` is on the PATH.
//!
//! **Only the server.** The owner's ruling (2026-10-13): `enable graph`
//! installs `mecha-graph-mcp` and not the `mecha-graph` CLI, "to be cleaner";
//! the CLI stays a `cargo install` of its own for whoever wants it.
//!
//! **A pinned commit, never the registry's latest.** The source is the public
//! `ljchang/mecha-graph` at [`REV`], built with its own lockfile
//! (`cargo install --git … --rev … --locked`) into `~/.mecha/sidecars/graph-mcp/`
//! — code that will hold the owner's graph is the code that was reviewed, as
//! the engine's and the speech engine's builds are. A bump is that one line.
//!
//! **Its own tree, by absolute path.** The binary is not put on the PATH:
//! the `[[mcp]]` entry `enable` writes names it absolutely, so a service unit
//! or a launchd agent with a short PATH finds it, and a `mecha-graph-mcp` the
//! owner installs later is theirs and never shadowed by this one.
//!
//! **The check never runs it.** Started, the server opens (and creates) the
//! owner's graph database at its default path — a side effect no install
//! check may have. What is checked instead is that cargo recorded this
//! package from this commit in the tree's own `.crates2.json`, and that the
//! binary is there and executable.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use crate::install::Say;
use crate::sidecar::{Machinery, Manifest};

/// The sidecar this installs (`sidecar::SIDECARS`).
pub const ID: &str = "graph-mcp";
/// The program, as `feature::mcp_wiring` names it.
pub const PROGRAM: &str = "mecha-graph-mcp";
/// The public repository it is built from.
pub const REPO: &str = "https://github.com/ljchang/mecha-graph";
/// The commit: mecha-graph's main on 2026-10-13.
pub const REV: &str = "1368fba42ebe46be5947e7e5152fb8b2033bb6d8";

/// The tree cargo installs into: `bin/` under it holds the binary.
pub fn root(mecha_home: &Path) -> PathBuf {
    mecha_home.join("sidecars").join(ID)
}

/// The installed binary.
pub fn binary(mecha_home: &Path) -> PathBuf {
    root(mecha_home).join("bin").join(PROGRAM)
}

/// Whether mecha's own build is here and is this commit's.
pub fn installed(mecha_home: &Path) -> bool {
    is_executable(&binary(mecha_home)) && built_from_pin(&root(mecha_home))
}

/// `cargo`, or why the build cannot run: it is the whole toolchain asked for,
/// since `cargo install` brings its own compile. A C compiler is needed too
/// (the graph's SQLCipher is built from source), and its absence is said by
/// cargo, at the build.
pub fn fits() -> std::result::Result<PathBuf, String> {
    fits_with(crate::onboarding::find_program)
}

/// [`fits`] over a lookup, so a test chooses whether cargo is here.
fn fits_with(find: impl Fn(&str) -> Option<PathBuf>) -> std::result::Result<PathBuf, String> {
    find("cargo").ok_or_else(|| {
        "it is built from its pinned source with cargo, which is not installed (rustup.rs \
         installs it)"
            .to_string()
    })
}

/// Build the pinned commit into [`root`] and check it. Idempotent: a build
/// already here from this commit is kept.
pub async fn install(m: &Machinery, say: Say<'_>) -> Result<()> {
    let cargo = fits().map_err(anyhow::Error::msg)?;
    install_with(m, &cargo, say).await
}

/// [`install`] with the cargo to run, so a test can stand one in.
async fn install_with(m: &Machinery, cargo: &Path, say: Say<'_>) -> Result<()> {
    let home = &m.mecha_home;
    let root = root(home);
    Manifest::begin(home, ID)?;
    Manifest::record(home, ID, &root)?;
    if installed(home) {
        say(&format!("{PROGRAM} is built from {} already", &REV[..7]));
        return Manifest::finish(home, ID);
    }
    say(&format!(
        "building {PROGRAM} from {REPO} at {} — a few minutes; cargo prints its progress",
        &REV[..7]
    ));
    let status = tokio::process::Command::new(cargo)
        .args([
            "install", "--git", REPO, "--rev", REV, "--locked", "--force", "--root",
        ])
        .arg(&root)
        .arg(PROGRAM)
        .status()
        .await
        .with_context(|| format!("running {}", cargo.display()))?;
    if !status.success() {
        bail!(
            "building {PROGRAM} failed ({status}) — cargo's output above says why; a C compiler \
             is needed beside cargo"
        );
    }
    check(&root)?;
    say(&format!("{PROGRAM} is at {}", binary(home).display()));
    Manifest::finish(home, ID)
}

/// The build is this commit's and its binary runs: asked of cargo's own
/// record of what it installed, never by starting the server.
fn check(root: &Path) -> Result<()> {
    let bin = root.join("bin").join(PROGRAM);
    if !is_executable(&bin) {
        bail!("the build finished, but {} is not there", bin.display());
    }
    if !built_from_pin(root) {
        bail!(
            "{} does not record {PROGRAM} from {REPO} at {REV} — not the pinned build",
            root.join(".crates2.json").display()
        );
    }
    Ok(())
}

/// Whether cargo's record in `root` names this package from the pinned
/// commit: its key reads `mecha-graph-mcp <version> (git+<repo>?rev=<rev>#<rev>)`.
fn built_from_pin(root: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(root.join(".crates2.json")) else {
        return false;
    };
    records_pin(&text)
}

fn records_pin(crates2: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(crates2) else {
        return false;
    };
    v["installs"].as_object().is_some_and(|installs| {
        installs.keys().any(|k| {
            k.starts_with(&format!("{PROGRAM} "))
                && k.contains(&format!("git+{REPO}"))
                && k.contains(REV)
        })
    })
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// Remove mecha's build, record and all.
pub fn remove(m: &Machinery) -> Result<()> {
    let home = &m.mecha_home;
    let root = root(home);
    match std::fs::remove_dir_all(&root) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("removing {}", root.display())),
    }
    Manifest::unrecord(home, ID, &root)?;
    Manifest::forget_if_empty(home, ID)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only cargo's record of this package from this repository at this
    /// commit is the pinned build — another rev, another source, another
    /// package, or no record, is not.
    #[test]
    fn only_the_pinned_commit_s_record_is_the_build() {
        let rec = |key: &str| format!(r#"{{"installs":{{"{key}":{{"bins":["{PROGRAM}"]}}}}}}"#);
        assert!(records_pin(&rec(&format!(
            "{PROGRAM} 0.1.5 (git+{REPO}?rev={REV}#{REV})"
        ))));
        assert!(!records_pin(&rec(&format!(
            "{PROGRAM} 0.1.5 (git+{REPO}?rev=0000000#0000000)"
        ))));
        assert!(!records_pin(&rec(&format!(
            "{PROGRAM} 0.1.5 (registry+https://github.com/rust-lang/crates.io-index)"
        ))));
        assert!(!records_pin(&rec(&format!(
            "mecha-graph 0.1.5 (git+{REPO}?rev={REV}#{REV})"
        ))));
        assert!(!records_pin("not json"));
        assert!(!records_pin("{}"));
    }

    #[test]
    fn without_cargo_the_build_is_not_offered_and_says_why() {
        let why = fits_with(|_| None).unwrap_err();
        assert!(why.contains("cargo") && why.contains("rustup"), "{why}");
        assert_eq!(
            fits_with(|n| (n == "cargo").then(|| PathBuf::from("/c/cargo"))).unwrap(),
            PathBuf::from("/c/cargo")
        );
    }

    /// A stand-in cargo, as a script: `body` runs with the arguments cargo
    /// would get, `--root <dir>` among them.
    fn fake_cargo(dir: &Path, body: &str) -> PathBuf {
        let p = dir.join("cargo");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    fn scratch_machinery(root: &Path) -> Machinery {
        Machinery {
            home: root.join("home"),
            mecha_home: root.join("home/.mecha"),
            unit_dirs: vec![],
            path: vec![],
            docker: Box::new(|_| crate::sidecar::Lookup::Absent),
        }
    }

    /// A failed build is an error and leaves nothing installed — the install
    /// unfinished in the record, so `enable` resumes it rather than reading
    /// mecha's half-tree as someone's; a build that exits 0 and wrote no
    /// binary is a failure too; one that wrote the pinned build is installed.
    #[tokio::test]
    async fn a_failed_or_empty_build_installs_nothing_and_a_good_one_is_checked() {
        let root = std::env::temp_dir().join(format!("mecha-gmcp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let m = scratch_machinery(&root);
        let home = m.mecha_home.clone();

        let failing = fake_cargo(&root, "exit 101");
        let e = install_with(&m, &failing, &mut |_| {}).await.unwrap_err();
        assert!(format!("{e:#}").contains("failed"), "{e:#}");
        assert!(!installed(&home));
        let man = Manifest::read(&home).unwrap();
        assert!(man.entries.iter().any(|e| e.sidecar == ID && e.incomplete));

        let empty = fake_cargo(&root, "exit 0");
        let e = install_with(&m, &empty, &mut |_| {}).await.unwrap_err();
        assert!(format!("{e:#}").contains("is not there"), "{e:#}");
        assert!(!installed(&home));

        // Writes what `cargo install --root <r> --git … --rev …` writes.
        let good = fake_cargo(
            &root,
            &format!(
                "while [ \"$1\" != --root ]; do shift; done; r=\"$2\"\n\
                 mkdir -p \"$r/bin\" && printf '#!/bin/sh\\n' > \"$r/bin/{PROGRAM}\" && \
                 chmod 755 \"$r/bin/{PROGRAM}\"\n\
                 printf '%s' '{{\"installs\":{{\"{PROGRAM} 0.1.5 (git+{REPO}?rev={REV}#{REV})\":{{}}}}}}' \
                 > \"$r/.crates2.json\""
            ),
        );
        install_with(&m, &good, &mut |_| {}).await.unwrap();
        assert!(installed(&home));
        let man = Manifest::read(&home).unwrap();
        assert!(man.entries.iter().any(|e| e.sidecar == ID && !e.incomplete));
        remove(&m).unwrap();
        assert!(!root.join("home/.mecha/sidecars").join(ID).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A tree with the binary but no record, or the record but no binary, is
    /// not installed.
    #[test]
    fn installed_needs_the_binary_and_the_record() {
        let home = std::env::temp_dir().join(format!("mecha-graph-mcp-{}", uuid::Uuid::new_v4()));
        let root = root(&home);
        assert!(!installed(&home));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::write(
            root.join(".crates2.json"),
            format!(r#"{{"installs":{{"{PROGRAM} 0.1.5 (git+{REPO}?rev={REV}#{REV})":{{}}}}}}"#),
        )
        .unwrap();
        assert!(!installed(&home), "no binary yet");
        std::fs::write(binary(&home), b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(binary(&home), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        assert!(installed(&home));
        std::fs::remove_file(root.join(".crates2.json")).unwrap();
        assert!(!installed(&home), "no record");
        let _ = std::fs::remove_dir_all(&home);
    }
}
