//! Rule-owned annotations survive classification rewriting a verdict. Kept
//! beside triage, keyed by account/thread, and merged into Record::rest on read.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::{fs::OpenOptionsExt, io::AsRawFd},
    path::Path,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuleRecord {
    pub tags: BTreeSet<String>,
    pub digest_rules: BTreeSet<String>,
    pub handled_message: Option<String>,
}

fn key(account: &str, thread: &str) -> String {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(&(account, thread)).expect("strings serialize");
    format!(
        "{}.json",
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

pub fn read(root: &Path, account: &str, thread: &str) -> Result<RuleRecord> {
    match fs::read(root.join("rule-records").join(key(account, thread))) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(RuleRecord::default()),
        Err(e) => Err(e.into()),
    }
}

/// Atomic, idempotent rule annotations; never manufactures a classifier verdict.
pub fn update(
    root: &Path,
    account: &str,
    thread: &str,
    edit: impl FnOnce(&mut RuleRecord),
) -> Result<()> {
    let directory = root.join("rule-records");
    crate::create_private_dir(&directory)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(directory.join(".lock"))?;
    // SAFETY: the owned file stays live through the atomic replacement.
    ensure!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } == 0,
        "locking rule annotations"
    );
    let mut record = read(root, account, thread)?;
    edit(&mut record);
    let path = directory.join(key(account, thread));
    let tmp = directory.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(&serde_json::to_vec(&record)?)?;
    file.sync_all()?;
    fs::rename(tmp, path)?;
    File::open(directory)?.sync_all()?;
    Ok(())
}

pub(super) fn overlay(root: &Path, record: &mut super::Record) -> Result<()> {
    let flags = read(root, &record.account, &record.thread_id)?;
    record
        .rest
        .insert("rule_tags".into(), serde_json::to_value(flags.tags)?);
    record.rest.insert(
        "digest_rules".into(),
        serde_json::to_value(flags.digest_rules)?,
    );
    record.rest.insert(
        "rules_handled_message".into(),
        serde_json::to_value(flags.handled_message)?,
    );
    Ok(())
}
