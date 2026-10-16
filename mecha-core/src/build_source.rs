// How `build.rs` decides whether a checkout's commit is one mecha's
// repository holds — `include!`d there, and a module here so it is tested.
// Plain comments only: an included file cannot open with inner docs.

/// A remote-tracking ref split into its remote and branch:
/// `refs/remotes/origin/main` is `("origin", "main")`. A pull request's ref
/// (`refs/remotes/pull/699/merge`, as CI checks one out) is no branch of the
/// repository and is `None`: a `cargo install --git --rev` of a commit only it
/// holds fails (review of #699).
pub fn remote_of(refname: &str) -> Option<(&str, &str)> {
    let rest = refname.strip_prefix("refs/remotes/")?;
    let (remote, branch) = rest.split_once('/')?;
    (remote != "pull" && !branch.starts_with("pull/") && branch != "HEAD")
        .then_some((remote, branch))
}

/// Whether a remote's URL is the repository mecha is published from, over
/// https or ssh, with or without `.git`.
pub fn same_repo(url: &str, repo: &str) -> bool {
    fn norm(u: &str) -> String {
        let u = u.trim().trim_end_matches('/');
        let u = u.strip_suffix(".git").unwrap_or(u);
        let u = u
            .strip_prefix("git@github.com:")
            .map(|r| format!("https://github.com/{r}"))
            .or_else(|| {
                u.strip_prefix("ssh://git@github.com/")
                    .map(|r| format!("https://github.com/{r}"))
            })
            .unwrap_or_else(|| u.to_string());
        u.to_ascii_lowercase()
    }
    !repo.is_empty() && norm(url) == norm(repo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_branch_of_mecha_s_repository_is_pushed_and_a_pull_ref_is_not() {
        assert_eq!(
            remote_of("refs/remotes/origin/main"),
            Some(("origin", "main"))
        );
        assert_eq!(
            remote_of("refs/remotes/origin/features/x"),
            Some(("origin", "features/x"))
        );
        assert_eq!(remote_of("refs/remotes/pull/699/merge"), None);
        assert_eq!(remote_of("refs/remotes/origin/pull/699/merge"), None);
        assert_eq!(remote_of("refs/remotes/origin/HEAD"), None);
        assert_eq!(remote_of("refs/heads/main"), None);
        let repo = "https://github.com/ljchang/mecha";
        for url in [
            "https://github.com/ljchang/mecha",
            "https://github.com/ljchang/mecha.git",
            "git@github.com:ljchang/mecha.git",
            "ssh://git@github.com/ljchang/mecha",
            "https://github.com/LJChang/mecha/",
        ] {
            assert!(same_repo(url, repo), "{url}");
        }
        assert!(!same_repo("https://github.com/someone/mecha", repo));
        assert!(!same_repo("https://github.com/ljchang/mecha-graph", repo));
        assert!(!same_repo("https://github.com/ljchang/mecha", ""));
    }
}
