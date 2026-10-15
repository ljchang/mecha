//! mecha's mail and documents servers, built from mecha's own source
//! (`docs/FEATURES-DESIGN.md` §15): what `mecha features enable mail` and
//! `enable docs` install when no current `mecha-mail` is here.
//!
//! **One crate, both servers.** `mecha-mail` builds `mecha-mail`,
//! `mecha-docs`, `mecha-google` and `mecha-outlook`; one install brings the
//! two a feature wires.
//!
//! **mecha's build, not the registry's latest.** The source is the one this
//! mecha was built from (`own_build::this`): its git commit, or its
//! published version — so the servers speak the protocol and take the
//! arguments this mecha expects. A copy on the PATH from another build reads
//! stale (`own_build::copy_of`); the owner's own checkout build is theirs.
//!
//! **Its own tree, by absolute path**, as the graph's server
//! (`graph_mcp.rs`): the `[[mcp]]` entries name the binaries absolutely,
//! and nothing is put on the PATH.
//!
//! **The check never runs it**: cargo's record in the tree says what was
//! built, from what.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use crate::install::Say;
use crate::own_build::{Copy, Source};
use crate::sidecar::{Machinery, Manifest};

/// The sidecar this installs (`sidecar::SIDECARS`).
pub const ID: &str = "mail-mcp";
/// The package cargo builds.
pub const PACKAGE: &str = "mecha-mail";
/// The programs a feature wires from it.
pub const PROGRAMS: &[&str] = &["mecha-mail", "mecha-docs"];
/// The repository mecha is published from: a git build's source.
pub const REPO: &str = env!("CARGO_PKG_REPOSITORY");

/// The tree cargo installs into.
pub fn root(mecha_home: &Path) -> PathBuf {
    mecha_home.join("sidecars").join(ID)
}

/// One of [`PROGRAMS`], installed.
pub fn binary(mecha_home: &Path, program: &str) -> PathBuf {
    root(mecha_home).join("bin").join(program)
}

/// Whether mecha's own build of both servers is here and is this mecha's.
pub fn installed(mecha_home: &Path) -> bool {
    let Some(want) = crate::own_build::this() else {
        return false;
    };
    PROGRAMS.iter().all(|p| {
        let bin = binary(mecha_home, p);
        is_executable(&bin) && crate::own_build::copy_of(&bin, PACKAGE, &want) == Copy::Current
    })
}

/// `cargo` and this mecha's source, or why the build cannot run.
pub fn fits() -> std::result::Result<(PathBuf, Source), String> {
    fits_with(crate::onboarding::find_program, crate::own_build::this())
}

fn fits_with(
    find: impl Fn(&str) -> Option<PathBuf>,
    source: Option<Source>,
) -> std::result::Result<(PathBuf, Source), String> {
    let source = source.ok_or_else(|| {
        "this mecha cannot tell which source it was built from, so the matching mail server \
         cannot be chosen — `cargo install mecha-mail --locked` installs one by hand"
            .to_string()
    })?;
    // A commit no remote holds cannot be fetched: the matching servers are
    // this checkout's to build, and a build of one's own is the owner's
    // (review of #699).
    if let Source::Local(sha) = &source {
        return Err(format!(
            "this mecha was built from commit {}, which no remote branch holds yet — build the \
             matching servers from the same checkout: `cargo install --path mecha-mail --locked`",
            &sha[..9]
        ));
    }
    let cargo = find("cargo").ok_or_else(|| {
        "it is built from mecha's own source with cargo, which is not installed (rustup.rs \
         installs it)"
            .to_string()
    })?;
    Ok((cargo, source))
}

/// The arguments `cargo` is run with: mecha's commit from its repository,
/// or its published version.
fn cargo_args(source: &Source, root: &Path) -> Vec<String> {
    let mut args: Vec<String> = match source {
        Source::Git(sha) | Source::Local(sha) => vec![
            "install".into(),
            "--git".into(),
            REPO.into(),
            "--rev".into(),
            sha.clone(),
        ],
        Source::Registry(version) => {
            vec!["install".into(), "--version".into(), format!("={version}")]
        }
    };
    args.extend(["--locked".into(), "--force".into(), "--root".into()]);
    args.push(root.display().to_string());
    args.push(PACKAGE.into());
    args
}

/// Build mecha's source of the servers into [`root`] and check it.
/// Idempotent: a build already here from this source is kept.
pub async fn install(m: &Machinery, say: Say<'_>) -> Result<()> {
    let (cargo, source) = fits().map_err(anyhow::Error::msg)?;
    install_with(m, &cargo, &source, say).await
}

async fn install_with(m: &Machinery, cargo: &Path, source: &Source, say: Say<'_>) -> Result<()> {
    let home = &m.mecha_home;
    let root = root(home);
    Manifest::begin(home, ID)?;
    Manifest::record(home, ID, &root)?;
    if built_from(&root, source) {
        say(&format!(
            "{PACKAGE} is built from {} already",
            source.short()
        ));
        return Manifest::finish(home, ID);
    }
    say(&format!(
        "building {PACKAGE} (the mail and documents servers) from {} — a few minutes; cargo \
         prints its progress",
        source.short()
    ));
    let status = tokio::process::Command::new(cargo)
        .args(cargo_args(source, &root))
        // As the graph's build: no stdin, no credential prompt, and the
        // build goes with this process.
        .stdin(std::process::Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .kill_on_drop(true)
        .status()
        .await
        .with_context(|| format!("running {}", cargo.display()))?;
    if !status.success() {
        bail!("building {PACKAGE} failed ({status}) — cargo's output above says why");
    }
    if !built_from(&root, source) {
        bail!(
            "the build finished, but {} does not record {PACKAGE} from {} with both servers",
            root.join(".crates2.json").display(),
            source.short()
        );
    }
    say(&format!("{PACKAGE} is at {}", root.join("bin").display()));
    Manifest::finish(home, ID)
}

/// Both servers are in `root`'s `bin/` and cargo recorded them from `source`.
fn built_from(root: &Path, source: &Source) -> bool {
    PROGRAMS.iter().all(|p| {
        let bin = root.join("bin").join(p);
        is_executable(&bin) && crate::own_build::copy_of(&bin, PACKAGE, source) == Copy::Current
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

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "7d7f0fa05aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn the_build_is_this_mecha_s_source() {
        let root = Path::new("/h/.mecha/sidecars/mail-mcp");
        assert_eq!(
            cargo_args(&Source::Git(SHA.into()), root),
            [
                "install",
                "--git",
                REPO,
                "--rev",
                SHA,
                "--locked",
                "--force",
                "--root",
                "/h/.mecha/sidecars/mail-mcp",
                "mecha-mail"
            ]
        );
        assert_eq!(
            cargo_args(&Source::Registry("0.1.25".into()), root),
            [
                "install",
                "--version",
                "=0.1.25",
                "--locked",
                "--force",
                "--root",
                "/h/.mecha/sidecars/mail-mcp",
                "mecha-mail"
            ]
        );
        assert!(REPO.starts_with("https://github.com/"), "{REPO}");
    }

    #[test]
    fn it_fits_only_with_cargo_and_a_known_source() {
        let cargo = |p: &str| (p == "cargo").then(|| PathBuf::from("/c/cargo"));
        let none = |_: &str| None;
        assert!(fits_with(cargo, Some(Source::Git(SHA.into()))).is_ok());
        assert!(fits_with(none, Some(Source::Git(SHA.into())))
            .unwrap_err()
            .contains("cargo"));
        assert!(fits_with(cargo, None)
            .unwrap_err()
            .contains("cannot tell which source"));
        assert!(fits_with(cargo, Some(Source::Local(SHA.into())))
            .unwrap_err()
            .contains("cargo install --path mecha-mail"));
    }

    /// The tree counts as built only when cargo recorded both servers from
    /// this source.
    #[test]
    fn built_from_reads_cargo_s_record() {
        let root = std::env::temp_dir().join(format!("mecha-mailmcp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let src = Source::Git(SHA.into());
        let key = format!("mecha-mail 0.1.24 (git+{REPO}?rev={SHA}#{SHA})");
        let write = |bins: &[&str]| {
            std::fs::write(
                root.join(".crates2.json"),
                serde_json::json!({"installs": {key.clone(): {"bins": bins}}}).to_string(),
            )
            .unwrap();
        };
        for p in PROGRAMS {
            let b = root.join("bin").join(p);
            std::fs::write(&b, "#!/bin/sh\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&b, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        write(&["mecha-mail", "mecha-docs"]);
        assert!(built_from(&root, &src));
        assert!(!built_from(&root, &Source::Git("1".repeat(40))));
        write(&["mecha-mail"]);
        assert!(
            !built_from(&root, &src),
            "docs' server missing from the record"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
