//! Authenticated native append boundary. The task host must separately hold its
//! lease and enforce attachment/turn phases; ordinary Session never opts in.
// This staged capability is consumed by the task host in the next increment.
use super::{authority::SOURCE_MAX, inline_notes, recording};
use crate::{
    agent::Conversation,
    memory_experience_bridge::{open_append_nofollow, open_nofollow},
    message::{Block, Message, Role},
    session::{encode_record_line, record_run_messages_and_metadata, Record, Session},
};
use anyhow::{ensure, Result};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, Metadata},
    io::Write,
    os::unix::fs::{FileExt, MetadataExt},
    path::PathBuf,
};

/// Data from the host-authenticated source, not an execution capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NativeIdentity {
    pub(crate) device: u64,
    pub(crate) inode: u64,
}
impl NativeIdentity {
    fn of(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
    fn check(self, metadata: &Metadata) -> Result<()> {
        ensure!(
            metadata.is_file() && metadata.nlink() == 1 && self == Self::of(metadata),
            "native source replaced, deleted or aliased"
        );
        Ok(())
    }
}

/// A successful same-descriptor sync and post-sync verification, never decoded
/// from disk. The endpoint/digest can be bound to a persisted host checkpoint.
pub(crate) struct SyncedNativePrefix {
    identity: NativeIdentity,
    endpoint: u64,
    digest: String,
    bytes: Vec<u8>,
}
impl SyncedNativePrefix {
    pub(crate) fn identity(&self) -> NativeIdentity {
        self.identity
    }
    pub(crate) fn endpoint(&self) -> u64 {
        self.endpoint
    }
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Bounded canonical final bytes prepared before writing. The host can persist
/// the predicted endpoint/digest first. Only the minting live writer can append
/// it, and only at its exact starting fingerprint. This has no wire decoder.
pub(crate) struct PreparedNativeAppend {
    binding: uuid::Uuid,
    identity: NativeIdentity,
    start: u64,
    start_digest: String,
    expected: Sha256,
    bytes: Vec<u8>,
}
impl PreparedNativeAppend {
    pub(crate) fn start(&self) -> u64 {
        self.start
    }
    pub(crate) fn start_digest(&self) -> &str {
        &self.start_digest
    }

    pub(crate) fn identity(&self) -> NativeIdentity {
        self.identity
    }
    pub(crate) fn endpoint(&self) -> u64 {
        self.start + self.bytes.len() as u64
    }
    pub(crate) fn digest(&self) -> String {
        hex_digest(&self.expected)
    }
    #[cfg(test)]
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// Add already constructed Outcome/Taint or other host records to the same
    /// preparation. Permissions and ordering are the task host's responsibility.
    pub(crate) fn push_record(&mut self, record: &Record) -> Result<()> {
        ordinary_append(record)?;
        self.push_checked(record)
    }
    fn push_checked(&mut self, record: &Record) -> Result<()> {
        // Count through the serializer first; do not allocate a record that
        // exceeds the remaining source budget. Legacy encoding stays unchanged.
        struct Count(usize);
        impl Write for Count {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if bytes.len() > self.0 {
                    return Err(std::io::ErrorKind::FileTooLarge.into());
                }
                self.0 -= bytes.len();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let remaining = SOURCE_MAX
            .checked_sub(self.endpoint() as usize)
            .and_then(|n| n.checked_sub(1))
            .ok_or_else(|| anyhow::anyhow!("native source exceeds 4 MiB profile"))?;
        serde_json::to_writer(Count(remaining), record)?;
        let line = encode_record_line(record)?;
        self.expected.update(&line);
        self.bytes.extend_from_slice(&line);
        Ok(())
    }
}

pub(crate) struct BoundNativeSession {
    path: PathBuf,
    file: File,
    parent: File,
    identity: NativeIdentity,
    parent_identity: NativeIdentity,
    binding: uuid::Uuid,
    endpoint: u64,
    expected: Sha256,
    poisoned: bool,
}
impl BoundNativeSession {
    /// The host supplies the exact source authenticated under its lease. Opening
    /// here cannot create a transcript, follow any path component, or adopt a
    /// different native header merely because its bytes have a matching length.
    pub(crate) fn bind(session: &Session, identity: NativeIdentity, prefix: &[u8]) -> Result<Self> {
        ensure!(
            prefix.len() <= SOURCE_MAX,
            "native source exceeds 4 MiB profile"
        );
        let file = open_append_nofollow(&session.path)?;
        identity.check(&file.metadata()?)?;
        let parent = open_nofollow(
            session
                .path
                .parent()
                .ok_or_else(|| anyhow::anyhow!("native parent unavailable"))?,
            true,
        )?;
        let parent_identity = NativeIdentity::of(&parent.metadata()?);
        let mut expected = Sha256::new();
        expected.update(prefix);
        let writer = Self {
            path: session.path.clone(),
            file,
            parent,
            identity,
            parent_identity,
            binding: uuid::Uuid::new_v4(),
            endpoint: prefix.len() as u64,
            expected,
            poisoned: false,
        };
        let bytes = writer.verified_bytes()?;
        ensure!(bytes == prefix, "native starting prefix changed");
        let records = recording::records(&bytes)?;
        ensure!(
            matches!(records.first(), Some(Record::Meta(meta))
            if serde_json::to_vec(meta)? == serde_json::to_vec(&session.meta)?),
            "native session header changed"
        );
        ensure!(
            !records.iter().skip(1).any(|r| matches!(r, Record::Meta(_))),
            "native session has multiple headers"
        );
        Ok(writer)
    }

    fn check_identity_and_endpoint(&self) -> Result<()> {
        ensure!(!self.poisoned, "native writer poisoned");
        let metadata = self.file.metadata()?;
        self.identity.check(&metadata)?;
        ensure!(metadata.len() == self.endpoint, "native endpoint changed");
        self.identity
            .check(&open_nofollow(&self.path, false)?.metadata()?)?;
        let path_parent = open_nofollow(self.path.parent().expect("bound parent"), true)?;
        ensure!(
            NativeIdentity::of(&path_parent.metadata()?) == self.parent_identity
                && NativeIdentity::of(&self.parent.metadata()?) == self.parent_identity,
            "native parent replaced"
        );
        Ok(())
    }
    fn verified_bytes(&self) -> Result<Vec<u8>> {
        self.check_identity_and_endpoint()?;
        ensure!(
            self.endpoint <= SOURCE_MAX as u64,
            "native source exceeds 4 MiB profile"
        );
        let mut bytes = vec![0; self.endpoint as usize];
        // Positional reads do not disturb the shared O_APPEND descriptor offset.
        self.file.read_exact_at(&mut bytes, 0)?;
        self.check_identity_and_endpoint()?;
        ensure!(
            Sha256::digest(&bytes) == self.expected.clone().finalize(),
            "native prefix changed"
        );
        Ok(bytes)
    }
    fn preparation(&self) -> PreparedNativeAppend {
        PreparedNativeAppend {
            binding: self.binding,
            identity: self.identity,
            start: self.endpoint,
            start_digest: hex_digest(&self.expected),
            expected: self.expected.clone(),
            bytes: Vec::new(),
        }
    }
    pub(crate) fn append(&mut self, record: &Record) -> Result<()> {
        let result = (|| {
            ensure!(!self.poisoned, "native writer poisoned");
            let mut prepared = self.preparation();
            prepared.push_record(record)?;
            self.append_prepared(prepared)
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    pub(crate) fn prepare_run(
        &mut self,
        before: &[Message],
        convo: &Conversation,
    ) -> Result<PreparedNativeAppend> {
        let result = (|| {
            ensure!(
                convo.rewritten.is_empty(),
                "native task rewrites unsupported"
            );
            ensure!(
                convo
                    .program_calls
                    .get(convo.program_calls_from..)
                    .unwrap_or_default()
                    .is_empty(),
                "native task programs unsupported"
            );
            let bytes = self.verified_bytes()?;
            recording::records(&bytes)?;
            let transcript = Session::parse(&self.path, std::str::from_utf8(&bytes)?)?;
            ensure!(
                transcript.convo.messages == before,
                "native starting history changed"
            );
            let mut prepared = self.preparation();
            record_run_messages_and_metadata(
                &mut prepared,
                before,
                convo,
                |sink, record| {
                    if let Record::Extend { index, blocks } = record {
                        ensure!(Some(*index) == before.len().checked_sub(1)
                        && before[*index].role == Role::User
                        && !before[*index].content.iter().any(|b| matches!(b, Block::ToolResult {..}))
                        && !blocks.is_empty()
                        && blocks.iter().all(|b| matches!(b, Block::Text {text} if inline_notes::empty_retry(text))),
                        "native task extension unsupported");
                        sink.push_checked(record)
                    } else {
                        sink.push_record(record)
                    }
                },
                |_, candidate| Ok(candidate == before),
            )?;
            Ok(prepared)
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    #[cfg(test)]
    pub(crate) fn record_run(&mut self, before: &[Message], convo: &Conversation) -> Result<()> {
        let prepared = self.prepare_run(before, convo)?;
        self.append_prepared(prepared)
    }
    pub(crate) fn append_prepared(&mut self, prepared: PreparedNativeAppend) -> Result<()> {
        let result = (|| {
            self.verified_bytes()?;
            ensure!(
                prepared.binding == self.binding
                    && prepared.identity == self.identity
                    && prepared.start == self.endpoint
                    && prepared.start_digest == hex_digest(&self.expected),
                "native preparation stale or foreign"
            );
            write_bytes(&mut self.file, &prepared.bytes)?;
            self.endpoint = prepared.endpoint();
            self.expected = prepared.expected;
            self.verified_bytes()?;
            Ok(())
        })();
        // A short write may have landed. Never retry it, even if an external
        // actor later restores the old bytes or length.
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    pub(crate) fn sync_verified(&mut self) -> Result<SyncedNativePrefix> {
        let result = (|| {
            let bytes = self.verified_bytes()?;
            recording::records(&bytes)?;
            sync_file(&self.file, false)?;
            // Session::create may have just introduced this directory entry.
            // Sync the held parent, then prove it still names the same file.
            sync_file(&self.parent, true)?;
            let bytes = self.verified_bytes()?;
            recording::records(&bytes)?;
            Ok(SyncedNativePrefix {
                identity: self.identity,
                endpoint: self.endpoint,
                digest: hex_digest(&self.expected),
                bytes,
            })
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
}
fn ordinary_append(record: &Record) -> Result<()> {
    ensure!(
        !matches!(
            record,
            Record::Meta(_)
                | Record::Rewrite { .. }
                | Record::Extend { .. }
                | Record::Program { .. }
        ),
        "native task record unsupported"
    );
    Ok(())
}
fn hex_digest(hash: &Sha256) -> String {
    hash.clone()
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn write_bytes(file: &mut File, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(test)]
    if let Some(length) = test_io::WRITE_AFTER.with(|v| v.take()) {
        file.write_all(&bytes[..length.min(bytes.len())])?;
        return Err(std::io::Error::other("injected partial write"));
    }
    file.write_all(bytes)
}
fn sync_file(file: &File, _parent: bool) -> std::io::Result<()> {
    #[cfg(test)]
    if test_io::SYNC_PARENT.with(|v| {
        if v.get() == Some(_parent) {
            v.set(None);
            true
        } else {
            false
        }
    }) {
        return Err(std::io::Error::other("injected sync failure"));
    }
    file.sync_all()?;
    #[cfg(test)]
    if !_parent {
        test_io::AFTER_FILE_SYNC.with(|slot| {
            if let Some(action) = slot.borrow_mut().take() {
                action();
            }
        });
    }
    Ok(())
}
#[cfg(test)]
pub(crate) mod test_io {
    use std::cell::{Cell, RefCell};
    thread_local! {
        pub(super) static AFTER_FILE_SYNC: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
        pub(super) static WRITE_AFTER: Cell<Option<usize>> = const { Cell::new(None) };
        pub(super) static SYNC_PARENT: Cell<Option<bool>> = const { Cell::new(None) };
    }
    pub(super) fn after_file_sync(action: impl FnOnce() + 'static) {
        AFTER_FILE_SYNC.with(|slot| *slot.borrow_mut() = Some(Box::new(action)));
    }
    pub(super) fn fail_write_after(length: usize) {
        WRITE_AFTER.with(|v| v.set(Some(length)));
    }
    pub(crate) fn fail_sync(parent: bool) {
        SYNC_PARENT.with(|v| v.set(Some(parent)));
    }
}
#[cfg(test)]
mod tests;
