//! The path to run *this* program again.
//!
//! Every surface that drives `mecha <verb>` as a child process — the TUI
//! modals, detached releases, the trigger daemon, the Slack doctor — needs
//! its own binary's path. `std::env::current_exe()` looks like the answer
//! and carries a trap: on Linux it resolves `/proc/self/exe` to its target,
//! and after `cargo install` has replaced the file on disk, the target of a
//! *running* process reads `…/mecha (deleted)` — a path that does not exist.
//! Every child spawn then fails with `No such file or directory (os error
//! 2)`, in exactly the long-lived session an install is invisible to.
//!
//! Measured 2026-08-23, minutes after it shipped: a TUI nineteen minutes
//! older than the install had `/queues` fail by name, and an outbox release
//! fail *quietly* — the confirmation ran, the release child could never
//! start, and the item sat `pending` looking like the release surface was
//! broken. The update skill's `/proc/<pid>/exe … (deleted)` sweep already
//! documents the *diagnosis*; this module removes the failure.
//!
//! The fix is the same file that exposed the problem: `/proc/self/exe` is a
//! magic link, and **executing the link itself runs the deleted inode** —
//! the kernel resolves it to the file this process was launched from, on
//! disk or not. That is also the more coherent behaviour: a session drives
//! the version it *is*, rather than picking up a newer binary mid-session
//! whose flags may have moved (which is precisely what `/queues` against a
//! younger `mecha review` would have been, inverted).

use std::path::{Path, PathBuf};

/// The path to spawn this same program from.
///
/// On Linux, the `/proc/self/exe` link itself, so a replaced-on-disk binary
/// keeps re-execing the version it is. Anywhere else — or if `/proc` is not
/// mounted — whatever `current_exe` reports, which is the best available.
pub fn self_exe() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        let link = Path::new("/proc/self/exe");
        // symlink_metadata, never `exists()`: `exists()` follows the link,
        // and a deleted target is exactly the case this path is for.
        if link.symlink_metadata().is_ok() {
            return link.to_path_buf();
        }
    }
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("mecha"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever `self_exe` names must actually be spawnable — that is its
    /// one job, and the property `current_exe` loses after an install.
    ///
    /// (The deleted-inode half cannot be arranged in a unit test without
    /// overwriting the running test binary; what is asserted here is that
    /// the magic-link path is exec-able at all, which is the mechanism the
    /// fix rests on.)
    #[test]
    fn the_path_it_names_can_be_spawned() {
        let out = std::process::Command::new(self_exe())
            .arg("--list")
            .arg("--format=terse")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        assert!(out.is_ok(), "self_exe must name something exec-able");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn on_linux_it_is_the_magic_link_not_its_target() {
        assert_eq!(self_exe(), Path::new("/proc/self/exe"));
    }
}
