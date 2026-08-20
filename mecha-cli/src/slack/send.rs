//! Sending a file out of a session, into the owner's Slack DM.
//!
//! Rung 1 of `docs/REMOTE-CONTROL-DESIGN.md`, and deliberately the rung that
//! commits to nothing: no thread, no attach record, no session identity — just
//! *put this file where I can look at it*. It answers the whole of one problem
//! on its own, which is that a chart rendered on a headless box is a file
//! nobody can see: over SSH there is no viewer, and scp in the other direction
//! is a second connection nobody wants to set up to look at a PNG.
//!
//! Three decisions carry it, and the first is the one the later rungs depend
//! on:
//!
//! - **The destination is not an argument.** It is the owner's DM, read from
//!   the binding. There is no parameter that moves it and deliberately no way
//!   to add one without changing this signature. That is what will let the
//!   model-facing tool in a later rung call this function without becoming an
//!   exfiltration sink: the caller names a path, never a destination. The same
//!   shape as `frontdoor::Record::for_privileged_run` — the safety property is
//!   a function signature rather than a rule someone remembers.
//! - **A refusal costs no round trip.** Everything knowable from the
//!   filesystem is decided before the DM is opened, so "that is a directory"
//!   arrives as itself instead of arriving after — or worse, instead of — a
//!   network error that happened first.
//! - **The cap is the one already configured.** `[slack] max_upload_mb` bounds
//!   what the connector fetches *into* a workspace; the same number bounds what
//!   leaves. Two numbers for one question is how the two drift.

use std::path::Path;

use anyhow::{bail, Context, Result};
use mecha_slack::binding::SlackStore;
use mecha_slack::{files, Slack};
use serde_json::{json, Value};

/// What was sent, for the line that reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub filename: String,
    pub bytes: u64,
    pub file_id: String,
}

/// The credential and who to send to — everything answerable without a
/// network call.
///
/// Split from [`open_dm`] so a caller can do its own local work in between.
/// `notify` needs exactly that: it checks the credential first and reads its
/// message from stdin second, and an empty message must cost no round trip.
/// Composing them here rather than inlining both at each call site is what
/// keeps "where does a message to the owner go" a single answer.
pub(crate) fn owner_client(store: &SlackStore) -> Result<(Slack, String)> {
    let creds = store
        .credentials()?
        .context("no Slack tokens stored — run `mecha slack auth` first")?;
    let binding = store
        .binding()?
        .context("nothing is bound — run `mecha slack link` first")?;
    let owner = binding
        .owners
        .first()
        .context("the binding names no owners")?
        .clone();
    Ok((Slack::new(&creds.bot_token), owner))
}

/// The owner's DM channel id.
///
/// Opening one is idempotent — Slack returns the existing channel — which is
/// why nothing caches it.
pub(crate) async fn open_dm(slack: &Slack, owner: &str) -> Result<String> {
    let opened: Value = slack
        .call("conversations.open", json!({ "users": owner }))
        .await
        .context("opening a DM with the owner")?;
    Ok(opened["channel"]["id"]
        .as_str()
        .context("conversations.open returned no channel")?
        .to_string())
}

/// Both halves, for callers with nothing to do between them.
pub(crate) async fn owner_dm(store: &SlackStore) -> Result<(Slack, String)> {
    let (slack, owner) = owner_client(store)?;
    let channel = open_dm(&slack, &owner).await?;
    Ok((slack, channel))
}

/// Whether this file can be sent, and the name to send it under.
///
/// Pure, and separated from the sending for the reason every check in this
/// project is: it can be tested without a token, a network, or a workspace.
/// Each refusal names the file and says what to do instead — a remote control
/// that fails with "invalid argument" is one people stop reaching for.
pub(crate) fn vet(path: &Path, is_dir: bool, len: u64, max_bytes: u64) -> Result<String> {
    if is_dir {
        bail!(
            "{} is a directory — send one file (or tar it first)",
            path.display()
        );
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .with_context(|| format!("{} has no file name to send it under", path.display()))?;
    if len == 0 {
        // Refused here rather than at Slack, which answers a zero-length
        // upload ticket with a code that says nothing about the cause.
        bail!("{name} is empty — there is nothing to send");
    }
    if len > max_bytes {
        bail!(
            "{name} is {} — the cap is {} (`[slack] max_upload_mb`)",
            human(len),
            human(max_bytes)
        );
    }
    Ok(name.to_string())
}

/// Bytes as a person reads them. Sizes appear in refusals, and a refusal that
/// says `26214400` makes the reader do arithmetic to find out how far over
/// they are.
pub(crate) fn human(bytes: u64) -> String {
    const MB: u64 = 1024 * 1024;
    const KB: u64 = 1024;
    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} bytes")
    }
}

/// Upload one file into the owner's DM.
///
/// `path` is expected to be already resolved by the caller — the TUI puts it
/// through the run's path jail, the CLI verb takes it from the user's own
/// shell, and those are different boundaries on purpose (see the CLI verb).
pub async fn send_file(path: &Path, comment: Option<&str>) -> Result<Sent> {
    let cfg = mecha_core::config::Config::load_global()?;
    let max_bytes = cfg.slack.max_upload_mb.saturating_mul(1024 * 1024);

    let meta =
        std::fs::metadata(path).with_context(|| format!("cannot read {}", path.display()))?;
    let name = vet(path, meta.is_dir(), meta.len(), max_bytes)?;

    let store = SlackStore::open(mecha_core::work::mecha_home()?.join("slack"))?;
    let (slack, channel) = owner_dm(&store).await?;

    let bytes = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    // Re-checked against what was actually read: a file being appended to — a
    // log, the obvious thing to send — can outgrow its own metadata between
    // the two calls, and the cap should bound what is uploaded rather than
    // what was measured.
    if bytes.len() as u64 > max_bytes {
        bail!(
            "{name} grew to {} while being read — the cap is {}",
            human(bytes.len() as u64),
            human(max_bytes)
        );
    }

    let file_id = files::upload(
        &slack,
        &name,
        &bytes,
        &files::Share {
            channel_id: Some(&channel),
            thread_ts: None,
            initial_comment: comment,
            title: Some(&name),
        },
    )
    .await
    .with_context(|| format!("uploading {name}"))?;

    Ok(Sent {
        filename: name,
        bytes: bytes.len() as u64,
        file_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAP: u64 = 25 * 1024 * 1024;

    #[test]
    fn an_ordinary_file_sends_under_its_own_name() {
        let name = vet(Path::new("/w/reports/chart.png"), false, 4_096, CAP).unwrap();
        assert_eq!(name, "chart.png");
    }

    #[test]
    fn a_directory_is_refused_by_name_with_the_way_out() {
        let err = vet(Path::new("/w/reports"), true, 4_096, CAP).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("/w/reports"), "{msg}");
        assert!(msg.contains("directory"), "{msg}");
        assert!(msg.contains("tar"), "{msg}");
    }

    /// Slack answers a zero-length upload ticket with a code that says nothing
    /// about the cause, so the refusal has to happen here to be readable.
    #[test]
    fn an_empty_file_is_refused_here_rather_than_by_slack() {
        let err = vet(Path::new("/w/empty.log"), false, 0, CAP).unwrap_err();
        assert!(err.to_string().contains("nothing to send"));
    }

    #[test]
    fn an_oversized_file_is_refused_naming_both_sizes_and_the_knob() {
        let err = vet(Path::new("/w/big.bin"), false, CAP + 1, CAP).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("25.0 MB"), "{msg}");
        assert!(msg.contains("max_upload_mb"), "{msg}");
    }

    /// Exactly at the cap is allowed: a boundary that refuses the size it
    /// advertises is a boundary nobody can aim at.
    #[test]
    fn the_cap_itself_is_allowed() {
        assert!(vet(Path::new("/w/big.bin"), false, CAP, CAP).is_ok());
    }

    #[test]
    fn a_path_with_no_file_name_is_refused_rather_than_sent_as_something_else() {
        assert!(vet(Path::new("/"), false, 10, CAP).is_err());
    }

    #[test]
    fn sizes_read_the_way_a_person_reads_them() {
        assert_eq!(human(512), "512 bytes");
        assert_eq!(human(2 * 1024), "2.0 KB");
        assert_eq!(human(3 * 1024 * 1024), "3.0 MB");
    }
}
