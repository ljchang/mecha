//! Which build mecha is, and whether a copy of one of its own servers is the
//! same build (`docs/FEATURES-DESIGN.md` §15, the owner's ruling of
//! 2026-10-15).
//!
//! **mecha's own servers follow mecha.** `mecha-mail` (with `mecha-docs`) and
//! `mecha-graph-mcp` speak a protocol and read settings that move with
//! mecha, so a copy from another build is wrong in ways no error names: on
//! the owner's fresh Mac, an older `mecha-mail` failed its handshake, took
//! other `auth` arguments, and an older graph server assumed another
//! embedding width. So `enable` installs the matching build into
//! `~/.mecha/sidecars/`, and a copy on the PATH from another build reads
//! stale rather than provided.
//!
//! **Asked of cargo's own record, never by running the copy.** `cargo install`
//! writes `.crates2.json` in its root (the parent of `bin/`), keyed by
//! `"<package> <version> (<source>)"`. The source decides:
//!
//! - `path+file://…` — the owner built it from a checkout of their own. It is
//!   theirs, and never stale: a development build is not to be replaced.
//! - `git+…#<sha>` — current when the commit is the one wanted.
//! - `registry+…` — current when a registry build of this version is wanted.
//!
//! No record is a copy cargo did not install — the owner's, left alone.

use std::path::Path;

/// Where a build came from (`build.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A git checkout at this commit.
    Git(String),
    /// The published crates at this version.
    Registry(String),
}

impl Source {
    /// Read from `build.rs`'s `MECHA_BUILD_SOURCE`.
    pub fn parse(text: &str) -> Option<Source> {
        match text.split_once(':')? {
            ("git", sha) if sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()) => {
                Some(Source::Git(sha.to_string()))
            }
            ("registry", v) if !v.is_empty() => Some(Source::Registry(v.to_string())),
            _ => None,
        }
    }

    /// As `mecha --version` says it: the short commit, or the release.
    pub fn short(&self) -> String {
        match self {
            Source::Git(sha) => format!("git {}", &sha[..9]),
            Source::Registry(v) => format!("crates.io {v}"),
        }
    }
}

/// This build's source, when `build.rs` could tell.
pub fn this() -> Option<Source> {
    option_env!("MECHA_BUILD_SOURCE").and_then(Source::parse)
}

/// `mecha --version`'s line: the version, and the source when known —
/// `0.1.24 (git 7d7f0fa05)` — since a git build and the release it precedes
/// carry one number.
pub fn version_line() -> &'static str {
    static LINE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    LINE.get_or_init(|| match this() {
        Some(source) => format!("{} ({})", env!("CARGO_PKG_VERSION"), source.short()),
        None => env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// What a copy of a server is, against the build wanted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Copy {
    /// The wanted build.
    Current,
    /// Built by the owner from a checkout, or by something other than cargo:
    /// theirs, left alone.
    Owners,
    /// Another build: why, for the plan to say.
    Stale(String),
}

/// What cargo recorded for the package that installed `bin` (a program under
/// a cargo root's `bin/`), against `want`.
pub fn copy_of(bin: &Path, package: &str, want: &Source) -> Copy {
    let Some(root) = bin.parent().and_then(Path::parent) else {
        return Copy::Owners;
    };
    let Ok(text) = std::fs::read_to_string(root.join(".crates2.json")) else {
        return Copy::Owners;
    };
    let program = bin.file_name().and_then(|n| n.to_str()).unwrap_or("");
    judge(&text, package, program, want)
}

/// [`copy_of`] over the record's text, so every case is a test.
fn judge(crates2: &str, package: &str, program: &str, want: &Source) -> Copy {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(crates2) else {
        return Copy::Owners;
    };
    let Some(installs) = v.get("installs").and_then(|i| i.as_object()) else {
        return Copy::Owners;
    };
    // The record that owns this program: the package's key whose bins name
    // it (cargo keeps one per package and source).
    let key = installs.iter().find_map(|(key, entry)| {
        let bins = entry.get("bins")?.as_array()?;
        (key.split(' ').next() == Some(package) && bins.iter().any(|b| b.as_str() == Some(program)))
            .then_some(key.as_str())
    });
    let Some(key) = key else {
        return Copy::Owners;
    };
    let (version, source) = match key
        .split_once(' ')
        .and_then(|(_, rest)| rest.split_once(' '))
    {
        Some((version, source)) => (version, source.trim_matches(['(', ')'])),
        None => return Copy::Owners,
    };
    if source.starts_with("path+") {
        return Copy::Owners;
    }
    match (source.split_once('#'), want) {
        (Some((_, sha)), Source::Git(wanted)) if source.starts_with("git+") => {
            if sha == wanted {
                Copy::Current
            } else {
                Copy::Stale(format!(
                    "built from commit {}, not {}",
                    short(sha),
                    short(wanted)
                ))
            }
        }
        (None, Source::Registry(wanted)) if source.starts_with("registry+") => {
            if version == wanted {
                Copy::Current
            } else {
                Copy::Stale(format!("version {version}, not {wanted}"))
            }
        }
        _ if source.starts_with("registry+") => {
            Copy::Stale(format!("the published {version}, not {}", describe(want)))
        }
        _ if source.starts_with("git+") => {
            Copy::Stale(format!("built from a git commit, not {}", describe(want)))
        }
        // A source this build does not know: not ours to judge.
        _ => Copy::Owners,
    }
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(9)]
}

fn describe(want: &Source) -> String {
    match want {
        Source::Git(sha) => format!("commit {}", short(sha)),
        Source::Registry(v) => format!("the published {v}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "7d7f0fa05aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OTHER: &str = "1111111111111111111111111111111111111111";

    fn record(key: &str, bins: &[&str]) -> String {
        serde_json::json!({"installs": {key: {"bins": bins}}}).to_string()
    }

    #[test]
    fn a_source_reads_back_and_refuses_the_malformed() {
        assert_eq!(
            Source::parse(&format!("git:{SHA}")),
            Some(Source::Git(SHA.into()))
        );
        assert_eq!(
            Source::parse("registry:0.1.24"),
            Some(Source::Registry("0.1.24".into()))
        );
        assert_eq!(Source::parse("git:7d7f0fa05"), None, "not a full commit");
        assert_eq!(Source::parse("registry:"), None);
        assert_eq!(Source::parse("path:/x"), None);
        assert_eq!(Source::Git(SHA.into()).short(), "git 7d7f0fa05");
    }

    /// The owner's fresh Mac: mecha from a git commit, `mecha-mail` from
    /// crates.io — stale. The same commit is current; the owner's own
    /// checkout build is theirs; no record is theirs.
    #[test]
    fn a_copy_is_judged_by_cargo_s_record() {
        let git = Source::Git(SHA.into());
        let registry = |v: &str| Source::Registry(v.into());
        let crates_io = "mecha-mail 0.1.24 (registry+https://github.com/rust-lang/crates.io-index)";
        let bins = ["mecha-mail", "mecha-docs"];
        assert!(matches!(
            judge(&record(crates_io, &bins), "mecha-mail", "mecha-docs", &git),
            Copy::Stale(_)
        ));
        assert_eq!(
            judge(
                &record(crates_io, &bins),
                "mecha-mail",
                "mecha-mail",
                &registry("0.1.24")
            ),
            Copy::Current
        );
        assert!(matches!(
            judge(
                &record(crates_io, &bins),
                "mecha-mail",
                "mecha-mail",
                &registry("0.1.25")
            ),
            Copy::Stale(_)
        ));
        let at = |sha: &str| {
            format!("mecha-mail 0.1.24 (git+https://github.com/ljchang/mecha?rev={sha}#{sha})")
        };
        assert_eq!(
            judge(&record(&at(SHA), &bins), "mecha-mail", "mecha-mail", &git),
            Copy::Current
        );
        let Copy::Stale(why) = judge(&record(&at(OTHER), &bins), "mecha-mail", "mecha-mail", &git)
        else {
            panic!("another commit is stale")
        };
        assert!(
            why.contains("111111111") && why.contains("7d7f0fa05"),
            "{why}"
        );
        // The owner's checkout build, under any wanted source.
        let path = "mecha-mail 0.1.23 (path+file:///home/u/mecha/mecha-mail)";
        assert_eq!(
            judge(&record(path, &bins), "mecha-mail", "mecha-mail", &git),
            Copy::Owners
        );
        // Nothing recorded for this program, or no readable record.
        assert_eq!(
            judge(
                &record(crates_io, &["mecha-google"]),
                "mecha-mail",
                "mecha-mail",
                &git
            ),
            Copy::Owners
        );
        assert_eq!(
            judge("not json", "mecha-mail", "mecha-mail", &git),
            Copy::Owners
        );
    }

    #[test]
    fn a_copy_s_record_is_found_beside_its_bin_directory() {
        let root = std::env::temp_dir().join(format!("mecha-own-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::write(
            root.join(".crates2.json"),
            record(
                "mecha-graph-mcp 0.1.5 (registry+https://github.com/rust-lang/crates.io-index)",
                &["mecha-graph-mcp"],
            ),
        )
        .unwrap();
        let bin = root.join("bin/mecha-graph-mcp");
        assert!(matches!(
            copy_of(&bin, "mecha-graph-mcp", &Source::Git(SHA.into())),
            Copy::Stale(_)
        ));
        assert_eq!(
            copy_of(&root.join("nowhere/bin/x"), "x", &Source::Git(SHA.into())),
            Copy::Owners
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
