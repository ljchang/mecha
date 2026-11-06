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
    // The root first: a rule pass can create it before the classifier ever
    // has, and verdicts written there later rely on its mode alone.
    crate::create_private_dir(root)?;
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

/// These fields are read-side projections, never part of the base verdict.
pub(super) fn clear_overlay(record: &mut super::Record) {
    for key in ["rule_tags", "digest_rules", "rules_handled_message"] {
        record.rest.remove(key);
    }
}

pub(super) fn overlay(root: &Path, record: &mut super::Record) {
    overlay_with_warning(root, record, |warning| eprintln!("{warning}"));
}

fn overlay_with_warning(root: &Path, record: &mut super::Record, warn: impl FnOnce(String)) {
    clear_overlay(record);
    if let Err(error) = apply_overlay(root, record) {
        let path = root
            .join("rule-records")
            .join(key(&record.account, &record.thread_id));
        warn(format!(
            "warning: unreadable rule annotations {}: {error}",
            path.display()
        ));
    }
}

fn apply_overlay(root: &Path, record: &mut super::Record) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreadable_overlay_warns_once_with_its_file_name() {
        let root = std::env::temp_dir().join(format!("rule-overlay-{}", uuid::Uuid::new_v4()));
        let mut record: crate::mail_triage::Record = serde_json::from_value(serde_json::json!({
            "account":"campus", "thread_id":"notice", "state":"classified"
        }))
        .unwrap();
        update(&root, "campus", "notice", |_| {}).unwrap();
        let path = root.join("rule-records").join(key("campus", "notice"));
        fs::write(&path, "corrupt").unwrap();
        let mut warnings = Vec::new();
        overlay_with_warning(&root, &mut record, |warning| warnings.push(warning));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains(&path.display().to_string()));
        assert!(record.rest.is_empty());
        fs::remove_file(&path).unwrap();
        overlay_with_warning(&root, &mut record, |_| {
            panic!("missing annotations are normal")
        });
        assert_eq!(record.rest["rule_tags"], serde_json::json!([]));
        fs::remove_dir_all(root).unwrap();
    }

    /// A rule pass can run before anything has classified mail, and the
    /// verdicts written later rely on the root's mode alone.
    #[test]
    fn the_first_annotation_creates_a_private_triage_root() {
        use std::os::unix::fs::PermissionsExt;
        let parent = std::env::temp_dir().join(format!("rule-root-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&parent).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
        let root = parent.join("mail-triage");
        update(&root, "campus", "notice", |_| {}).unwrap();
        for dir in [root.clone(), root.join("rule-records")] {
            let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{}", dir.display());
        }
        fs::remove_dir_all(parent).unwrap();
    }
}
