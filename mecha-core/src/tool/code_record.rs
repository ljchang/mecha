//! What a program read, recorded (`CODE-MODE-DESIGN.md` §7, "Recording a
//! program's reads, so grounding can find them"; slice 3e).
//!
//! A host call is not a `tool_use`/`tool_result` pair, and a handle keeps its
//! bytes out of the transcript, so without a record every reader of
//! [`crate::grounding::calls`] — gossip's citation check, the outbox's "what
//! is this draft answering" join, the attribution pack, appraisal's evidence,
//! persona citations — loses its referent for what a program read. The
//! record is this module's [`ProgramCall`], one per host call that ran:
//!
//! - **Carried on the conversation, never in its messages.** A `code` call's
//!   output carries its calls ([`crate::tool::ToolOutput::reads`]); the loop
//!   moves them onto `Conversation::program_calls`, beside the messages as
//!   taint is, and the session writes them as a `program` record. The
//!   model's context never holds them, and a compaction, which rewrites only
//!   the messages, carries them across its cut.
//! - **The bytes are not in the JSONL.** Each result is stored once,
//!   content-addressed, in [`Store`]: `~/.mecha/programs/<session-id>/`, a
//!   root no jail reaches (`work::ensure_outside_mecha_home` refuses a
//!   workspace at or under it, and no grant may name the mecha home), and
//!   never the spill directory, which the jail admits. The record holds the
//!   blob's sha256 and length. A missing or mismatched blob makes that call
//!   ungrounded *by name* ([`Bytes::Missing`], [`Bytes::Mismatched`]), never
//!   silently empty.
//! - **Through a subagent, nested.** A child writes no session of its own, so
//!   the subagent's output carries its child's calls up, each under the
//!   subagent call that made it ([`Under`]). A `trusted_output` child's calls
//!   are carried for the owner's review and yielded by the walk to no reader
//!   by default: for those readers the subagent call's result is the shaped
//!   answer and nothing under it (§7, "The walk stops at a `trusted_output`
//!   child's shape"; R9 opens them for the outbox's owner-facing view only).
//!
//! **A closed shape on an append-only store is a wire format.** Every field a
//! later build might add defaults on load; an older build, which knows no
//! `program` record, skips the line as one it cannot parse
//! (`Session::messages_ever`'s loop), never failing the file. And an
//! [`Under`] whose `trusted` a reader cannot find reads `true`: an unknown
//! boundary is kept closed.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The directory under the mecha home that holds every session's blobs.
pub const ROOT: &str = "programs";

/// One host call a program made that ran (`CODE-MODE-DESIGN.md` §7): which
/// tool, with what input, whether its result came from outside, and a
/// pointer to the bytes. Never the bytes themselves on the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramCall {
    /// The host call's id, as the dispatch minted it
    /// (`<code call id>.host-<n>`): unique within the run that made it.
    pub id: String,
    /// The `tool_use` id of the `code` call whose program made it, in the
    /// run that ran the program — for a child's call, the child's id.
    pub code: String,
    /// The tool, by its registered name.
    pub tool: String,
    /// The arguments the program passed.
    #[serde(default)]
    pub input: Value,
    /// The result came from outside the machine (`ToolOutput::external`).
    #[serde(default)]
    pub external: bool,
    /// The tool reported failure: its bytes are an error body, which the walk
    /// never offers as a referent (a failed call describes nothing).
    #[serde(default)]
    pub is_error: bool,
    /// Where the bytes are: set when the session stored them. `None` in
    /// memory before that, and on a record that lost it, which reads as a
    /// missing blob.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<BlobRef>,
    /// The subagent call this was carried up through, when a child's program
    /// made it. `None` for a call of this conversation's own `code`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub under: Option<Under>,
    /// The bytes, as far as this process has them. Never serialised.
    #[serde(skip)]
    pub bytes: Bytes,
}

/// A blob's address in [`Store`]: its sha256, as lowercase hex, and its
/// length in bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobRef {
    pub sha256: String,
    pub len: u64,
}

/// The subagent call a child's program call was carried up through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Under {
    /// The parent's `tool_use` id of the subagent call. Stamped by the
    /// parent's dispatch, which is the only place that knows it; empty until
    /// then.
    #[serde(default)]
    pub call: String,
    /// The subagent, by its tool name.
    #[serde(default)]
    pub tool: String,
    /// The child's profile is `trusted_output`: its calls are yielded to no
    /// reader of the walk by default (§7). Unknown reads `true`, closed.
    #[serde(default = "closed")]
    pub trusted: bool,
}

fn closed() -> bool {
    true
}

/// What this process holds of a call's bytes.
#[derive(Debug, Clone, Default)]
pub enum Bytes {
    /// In hand: from the program that read them, or loaded from the store
    /// and checked against the record.
    Held(Arc<str>),
    /// Not in hand: a record read without its store, or a call of an earlier
    /// run of a live conversation, whose bytes the loop let go at the next
    /// run's start: the store has them if that run was recorded, and an
    /// unrecorded run's (`--no-session`) are gone for good, which reads as
    /// ungrounded, never as empty.
    #[default]
    Unloaded,
    /// The record names no blob, or the store has none at its address — a
    /// session reaped or forgotten, or a store that cannot be read.
    Missing,
    /// The store has a blob there whose bytes are not the record's: another
    /// length, another hash, or not UTF-8.
    Mismatched,
}

impl ProgramCall {
    /// A call that ran, with its bytes in hand.
    pub fn ran(
        id: impl Into<String>,
        code: impl Into<String>,
        tool: impl Into<String>,
        input: Value,
        external: bool,
        is_error: bool,
        bytes: Arc<str>,
    ) -> Self {
        ProgramCall {
            id: id.into(),
            code: code.into(),
            tool: tool.into(),
            input,
            external,
            is_error,
            blob: None,
            under: None,
            bytes: Bytes::Held(bytes),
        }
    }

    /// The bytes, when this process holds them.
    pub fn text(&self) -> Option<&str> {
        match &self.bytes {
            Bytes::Held(t) => Some(t),
            _ => None,
        }
    }

    /// The `tool_use` id this call sits under in the conversation that
    /// carries it: the subagent call it came up through, or its own `code`
    /// call.
    pub fn anchor(&self) -> &str {
        match &self.under {
            Some(u) => &u.call,
            None => &self.code,
        }
    }

    /// Carried up through a `trusted_output` child (§7): yielded to no reader
    /// of the walk by default.
    pub fn shaped(&self) -> bool {
        self.under.as_ref().is_some_and(|u| u.trusted)
    }

    /// Carry this call up through the subagent `tool`, whose profile is
    /// `trusted_output` or not. A call already carried up through another
    /// subagent keeps that one: it is anchored inside the child, so no walk
    /// of the parent yields it, which is closed, and nothing here could widen
    /// a grandchild's boundary by restamping it.
    pub fn carried_up(mut self, tool: &str, trusted: bool) -> Self {
        if self.under.is_none() {
            self.under = Some(Under {
                call: String::new(),
                tool: tool.to_string(),
                trusted,
            });
        }
        self
    }

    /// The parent's dispatch names the subagent call `call` it came up
    /// through, where nothing has yet.
    pub fn anchored_at(mut self, call: &str) -> Self {
        if let Some(u) = &mut self.under {
            if u.call.is_empty() {
                u.call = call.to_string();
            }
        }
        self
    }
}

/// The most program bytes one conversation holds in hand, newest first: the
/// process-wide host-byte pool's own number, so a run's retained reads stay
/// under the ceiling that bounded its programs while they ran (review of
/// #784). The pool bounds programs in flight and is handed back when each
/// ends, while the record's bytes outlive it.
pub const RETAINED_BYTES: usize = super::defaults::PROCESS_HOST_BYTES;

/// Hold at most [`RETAINED_BYTES`] of `calls`' bytes, keeping the newest:
/// an older call past the ceiling lets its bytes go ([`Bytes::Unloaded`]),
/// and its call stays for the walk to place. Fail-closed for every reader:
/// a call without bytes offers no referent, never an empty one, and is
/// recorded without an address.
pub fn bound_held(calls: &mut [ProgramCall]) {
    bound_at(calls, RETAINED_BYTES)
}

fn bound_at(calls: &mut [ProgramCall], ceiling: usize) {
    let mut held = 0usize;
    for call in calls.iter_mut().rev() {
        let Bytes::Held(text) = &call.bytes else {
            continue;
        };
        held = held.saturating_add(text.len());
        if held > ceiling {
            call.bytes = Bytes::Unloaded;
        }
    }
}

/// Add a batch's reads to a conversation's, then [`bound_held`]: the one
/// way the loop puts program calls on a conversation, so no site can add
/// them unbounded.
pub fn extend_bounded(calls: &mut Vec<ProgramCall>, reads: Vec<ProgramCall>) {
    extend_at(calls, reads, RETAINED_BYTES)
}

fn extend_at(calls: &mut Vec<ProgramCall>, reads: Vec<ProgramCall>, ceiling: usize) {
    if reads.is_empty() {
        return;
    }
    calls.extend(reads);
    bound_at(calls, ceiling);
}

/// The blob store: `<mecha home>/programs/<session-id>/<sha256>`, each file
/// mode 0600 in a directory mode 0700. Content-addressed within one session
/// only, so there is nothing shared across sessions to count, and forgetting
/// a session removes its directory whole.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

/// The store's root under `home`.
pub fn root_under(home: &Path) -> PathBuf {
    home.join(ROOT)
}

/// A session id names a directory here, so it is held to the archive's rule
/// (`forget::forget`'s): letters, digits, `-` and `_`, at most 128.
fn plain_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl Store {
    /// The store under the mecha home (`$MECHA_HOME`, or `~/.mecha`).
    pub fn default_root() -> Result<Self> {
        Ok(Store::at(root_under(&crate::work::mecha_home()?)))
    }

    pub fn at(root: impl Into<PathBuf>) -> Self {
        Store { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The directory holding session `id`'s blobs.
    pub fn session_dir(&self, id: &str) -> Result<PathBuf> {
        anyhow::ensure!(plain_id(id), "invalid session id {id:?}");
        Ok(self.root.join(id))
    }

    /// Store `text` for session `id`, once: a blob already at its address
    /// with its length is kept, since the address is its hash.
    pub fn put(&self, id: &str, text: &str) -> Result<BlobRef> {
        let dir = self.session_dir(id)?;
        crate::create_private_dir(&self.root)
            .and_then(|()| crate::create_private_dir(&dir))
            .with_context(|| format!("creating {}", dir.display()))?;
        let blob = BlobRef {
            sha256: sha256_hex(text.as_bytes()),
            len: text.len() as u64,
        };
        let path = dir.join(&blob.sha256);
        let present =
            std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file() && m.len() == blob.len);
        if !present {
            crate::imagelib::write_atomic_mode(&path, text.as_bytes(), Some(0o600))
                .with_context(|| format!("storing a program's read at {}", path.display()))?;
        }
        Ok(blob)
    }

    /// Fill each call's bytes from session `id`'s blobs, checked against the
    /// record: held when the hash and length match, otherwise named missing
    /// or mismatched — never an empty string standing in for either.
    pub fn load(&self, id: &str, calls: &mut [ProgramCall]) {
        let dir = self.session_dir(id).ok();
        for call in calls.iter_mut() {
            call.bytes = match (&dir, &call.blob) {
                (Some(dir), Some(blob)) if blob.sha256.len() == 64 && plain_id(&blob.sha256) => {
                    match std::fs::read(dir.join(&blob.sha256)) {
                        Err(_) => Bytes::Missing,
                        Ok(raw)
                            if raw.len() as u64 != blob.len || sha256_hex(&raw) != blob.sha256 =>
                        {
                            Bytes::Mismatched
                        }
                        Ok(raw) => match String::from_utf8(raw) {
                            Ok(text) => Bytes::Held(Arc::from(text)),
                            Err(_) => Bytes::Mismatched,
                        },
                    }
                }
                _ => Bytes::Missing,
            };
        }
    }

    /// Remove session `id`'s blobs, whole: how many files went. Nothing to
    /// remove is zero, never an error.
    pub fn forget(&self, id: &str) -> Result<usize> {
        let dir = self.session_dir(id)?;
        let n = match std::fs::read_dir(&dir) {
            Ok(entries) => entries.count(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e).with_context(|| format!("reading {}", dir.display())),
        };
        std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
        Ok(n)
    }
}

/// Every program call a transcript recorded, in record order, once each,
/// with its bytes loaded from the default store — the sibling of
/// [`crate::session::Session::messages_ever`] for the readers that walk a
/// session file. Empty when the session recorded none; a store that cannot
/// be found leaves every call [`Bytes::Missing`].
pub fn ever(transcript: &str, session_id: &str) -> Vec<ProgramCall> {
    let mut calls = crate::session::Session::programs_ever(transcript);
    if calls.is_empty() {
        return calls;
    }
    match Store::default_root() {
        Ok(store) => store.load(session_id, &mut calls),
        Err(_) => calls.iter_mut().for_each(|c| c.bytes = Bytes::Missing),
    }
    calls
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: &str, text: &str) -> ProgramCall {
        ProgramCall::ran(
            id,
            "t1",
            "mail_get_thread",
            serde_json::json!({"thread_id": "x"}),
            true,
            false,
            Arc::from(text),
        )
    }

    fn scratch() -> PathBuf {
        let d = std::env::temp_dir().join(format!("mecha-programs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The record carries a pointer, never the bytes: the serialised call
    /// holds the blob's hash and length and no byte of the result.
    #[test]
    fn a_recorded_call_carries_its_address_and_never_its_bytes() {
        let root = scratch();
        let store = Store::at(&root);
        let mut c = call("t1.host-1", "MARKER-IN-THE-RESULT");
        c.blob = Some(
            store
                .put("20260101T000000-abcd1234", c.text().unwrap())
                .unwrap(),
        );
        let line = serde_json::to_string(&c).unwrap();
        assert!(!line.contains("MARKER-IN-THE-RESULT"), "{line}");
        assert!(line.contains(&c.blob.as_ref().unwrap().sha256), "{line}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A blob that is gone, or whose bytes changed, is named as such, never
    /// read as an empty result.
    #[test]
    fn a_missing_or_altered_blob_is_named_and_never_empty() {
        let root = scratch();
        let store = Store::at(&root);
        let id = "20260101T000000-abcd1234";
        let mut a = call("t1.host-1", "first thread");
        a.blob = Some(store.put(id, a.text().unwrap()).unwrap());
        let mut b = call("t1.host-2", "second thread");
        b.blob = Some(store.put(id, b.text().unwrap()).unwrap());
        let mut c = call("t1.host-3", "third");
        c.blob = None;

        let path = root.join(id).join(&b.blob.as_ref().unwrap().sha256);
        std::fs::write(&path, "second threaX").unwrap();
        let mut loaded = vec![a.clone(), b.clone(), c];
        for l in &mut loaded {
            l.bytes = Bytes::Unloaded;
        }
        store.load(id, &mut loaded);
        assert_eq!(loaded[0].text(), Some("first thread"));
        assert!(
            matches!(loaded[1].bytes, Bytes::Mismatched),
            "{:?}",
            loaded[1].bytes
        );
        assert!(matches!(loaded[2].bytes, Bytes::Missing));

        assert_eq!(store.forget(id).unwrap(), 2);
        store.load(id, &mut loaded[..1]);
        assert!(matches!(loaded[0].bytes, Bytes::Missing));
        assert_eq!(store.forget(id).unwrap(), 0, "nothing left is not an error");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A session id is a directory name here, so one that could climb out of
    /// the store is refused.
    #[test]
    fn a_session_id_cannot_climb_out_of_the_store() {
        let store = Store::at(scratch());
        for bad in ["", "..", "../x", "a/b", "a.b"] {
            assert!(store.put(bad, "x").is_err(), "{bad:?} was accepted");
        }
        let _ = std::fs::remove_dir_all(store.root());
    }

    /// A conversation's retained reads are bounded within a run, not only
    /// across runs (review of #784, pass 4): the host-byte pool is handed
    /// back when a program ends while the record's bytes live on, so the
    /// loop holds the newest up to the pool's own number and lets older ones
    /// go. The calls stay. Without the bound every read stays resident.
    #[test]
    fn retained_reads_are_bounded_newest_first() {
        assert_eq!(
            RETAINED_BYTES,
            crate::tool::code::defaults::PROCESS_HOST_BYTES
        );
        let mut held = vec![call("t1.host-1", "aaaaaaaaaa")];
        extend_at(
            &mut held,
            vec![
                call("t1.host-2", "bbbbbbbbbb"),
                call("t1.host-3", "cccccccccc"),
            ],
            25,
        );
        assert_eq!(held.len(), 3, "every call is kept");
        assert!(matches!(held[0].bytes, Bytes::Unloaded), "the oldest goes");
        assert_eq!(held[1].text(), Some("bbbbbbbbbb"));
        assert_eq!(held[2].text(), Some("cccccccccc"));
    }

    /// An `under` a reader cannot read the trust of is closed.
    #[test]
    fn an_under_with_no_trust_on_file_reads_closed() {
        let c: ProgramCall = serde_json::from_str(
            r#"{"id":"a.host-1","code":"a","tool":"mail_search","under":{"call":"p1"}}"#,
        )
        .unwrap();
        assert!(c.shaped());
        assert_eq!(c.anchor(), "p1");
        assert!(matches!(c.bytes, Bytes::Unloaded));
    }
}
