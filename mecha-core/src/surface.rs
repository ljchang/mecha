//! The tool surface a run was sent, kept once and cited by hash.
//!
//! ## The gap this closes
//!
//! [`RunConfig`](crate::session::RunConfig) records the system prompt **in
//! full**, and says why: *"the text lets a replay rebuild the request."* It
//! records the tool surface as **names**, with a doc comment naming the risk it
//! saw — *"a tool added, removed or renamed between recording and replay
//! changes what the model could have done."*
//!
//! Add, remove and rename are the three that never happen. **Re-describe is the
//! one that happens constantly** — 49 commits touched tool definitions in three
//! weeks of this store — and it is invisible to a list of names. Render order
//! is tools → system → messages, so a replay was rebuilding the *second* half
//! of the prefix byte-exactly and the first half from whatever the registry
//! says today. Every description edit since a recording silently changes the
//! bytes the model sees before anything else.
//!
//! Measured: **12 of 13 counterfactual probes came back inconclusive**, on a
//! pinned seed and a quiet box, deterministically across repeats — median
//! divergence one tool call in. Six probes in one session, with steer points
//! from 10 to 33, all gave up at the same call: a trajectory-dependent cause
//! cannot do that, and a per-session constant can.
//!
//! ## Why a hash and a store rather than the specs inline
//!
//! Costed rather than preferred: the specs are **69 KB** against a **25 KB**
//! average session file, so inlining would quadruple the session store and put,
//! in most sessions, more bytes of tool description than conversation. A hash
//! alone is cheap and gives up the rebuild, which is the point of recording it.
//!
//! So the specs are written once per distinct surface and cited by hash — the
//! precedent is `ValidationRecord`'s `rules_hash`, *"keyed to the exact rule
//! set measured, because a tally that mixes generations measures nothing."*
//! Surfaces change tens of times over a corpus, not once per session, so the
//! store dedupes to a few megabytes where inlining would cost tens.
//!
//! ## Three states, and the one that matters is `Unknown`
//!
//! A names-only recording must never read as *matching*. Every session written
//! before this field exists — all of them, on the day it lands — is
//! [`Fidelity::Unknown`], and a probe over one is inconclusive **for a named
//! reason** instead of mysteriously. That is the whole reason this is
//! `Option<String>` and not `String`: absent is not equal, the rule
//! [`crate::homeostat`] and [`crate::backlog`] both state at length.
//!
//! [`Fidelity::Differs`] needs no blob at all — comparing today's hash against
//! the recorded one answers it — so **legibility arrives the day the field
//! ships and rebuildability accumulates afterwards**. That ordering is worth
//! more than either half alone: it turns an inconclusive probe from a mystery
//! into a labelled cause immediately.
//!
//! ## What it does not do
//!
//! **It does not recover the existing corpus.** Nothing can: those recordings
//! never held the specs, and the descriptions they were sent are only in git
//! history that cannot be matched to a session. The appraisal corpus and the
//! validation ledger start from zero the day this ships, and anyone budgeting
//! on the sessions already on disk should read that first.
//!
//! Nothing here is ever deleted, for the same reason a published bundle is not:
//! a surface blob is what makes an old session replayable, and a retention
//! policy over it would quietly cost the recordings it was keeping.

use crate::message::ToolSpec;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// How faithfully a replay can reproduce what a recording was sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fidelity {
    /// Today's surface hashes to the recorded value. The replay sends what the
    /// recording sent.
    Matches,
    /// It provably does not, and the difference is in the bytes ahead of the
    /// system prompt. A replay may still run; its divergences say nothing
    /// about the question being probed.
    Differs,
    /// The recording predates the field, so there is nothing to compare. **Not
    /// a match** — see the module note.
    Unknown,
}

impl Fidelity {
    /// Compare a recorded hash against a live surface.
    pub fn of(recorded: Option<&str>, live: &[ToolSpec]) -> Fidelity {
        match recorded {
            None => Fidelity::Unknown,
            Some(h) if h == fingerprint(live) => Fidelity::Matches,
            Some(_) => Fidelity::Differs,
        }
    }

    /// One phrase for a probe's `reason` field, or `None` when there is
    /// nothing to say.
    pub fn caveat(self) -> Option<&'static str> {
        match self {
            Fidelity::Matches => None,
            Fidelity::Differs => Some(
                "the tool surface has changed since this was recorded, so the replay sends \
                 different bytes ahead of the system prompt",
            ),
            Fidelity::Unknown => Some(
                "this was recorded before the tool surface was kept, so how faithfully it \
                 replays is unknown",
            ),
        }
    }
}

/// A stable identity for one tool surface.
///
/// Over the **rendered specs**, not the names — the whole point is that a
/// re-described tool is a different surface under the same name. Order is the
/// registry's, which is `BTreeMap` order and therefore stable, and is itself
/// part of what the model saw: the tool list is the front of the cached prefix.
///
/// **`learning::rules_hash`'s hasher, not `DefaultHasher`.** This value is
/// both a comparison key and a filename, so the caveat that function's own
/// doc comment states — *"the std hasher is deliberately unstable across
/// Rust releases, and a ledger key that drifts with the toolchain would
/// silently split every tally"* — bites harder here: a toolchain bump would
/// make [`Fidelity::of`] read every session recorded on the old one as
/// `Differs`, permanently and indistinguishably from real re-describe drift,
/// and orphan every blob already written in the store this module's own note
/// says is never pruned. A canonical rendering fed through the same hash
/// this codebase already trusts for a persisted key, rather than a second
/// hand-rolled FNV-1a, so there is one definition to keep stable.
pub fn fingerprint(specs: &[ToolSpec]) -> String {
    let mut rendered = String::new();
    for spec in specs {
        rendered.push_str(&spec.name);
        rendered.push('\0');
        rendered.push_str(&spec.description);
        rendered.push('\0');
        // `serde_json::Map` is a `BTreeMap`, so this rendering is canonical
        // whatever order a schema was built in.
        rendered.push_str(&spec.input_schema.to_string());
        rendered.push('\0');
    }
    crate::learning::rules_hash(&rendered)
}

/// Where surface blobs live. One file per distinct surface, named by its hash.
pub struct SurfaceStore {
    root: PathBuf,
}

impl SurfaceStore {
    pub fn default_root() -> Result<PathBuf> {
        Ok(crate::work::mecha_home()?.join("surfaces"))
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        // Owner-only, like every other store root under `~/.mecha` — the
        // front door's own rule for the same reason: these blobs are not
        // nothing. A `ToolSpec` carries the mail account short names baked
        // into every tool schema as an enum at startup, plus whatever an MCP
        // server put in its own descriptions.
        crate::create_private_dir(&root).with_context(|| format!("creating {}", root.display()))?;
        Ok(SurfaceStore { root })
    }

    /// Open the default store, or `None` if it cannot be reached.
    ///
    /// Best-effort by design: recording a surface is bookkeeping beside a run,
    /// and a full disk must not stop the run itself. A session that could not
    /// record its surface carries no hash and reads back as
    /// [`Fidelity::Unknown`], which is exactly true.
    pub fn open_default() -> Option<Self> {
        Self::default_root().ok().and_then(|r| Self::open(r).ok())
    }

    fn path(&self, hash: &str) -> PathBuf {
        self.root.join(format!("{hash}.json"))
    }

    /// Record a surface and return its hash. A surface already on disk costs
    /// one `exists` and no write.
    pub fn record(&self, specs: &[ToolSpec]) -> Result<String> {
        let hash = fingerprint(specs);
        let path = self.path(&hash);
        if path.exists() {
            return Ok(hash);
        }
        // Temp-and-rename on the store convention: a crash mid-write must
        // leave no half-file under a name that claims to be a whole surface.
        let tmp = self.root.join(format!("{hash}.json.tmp"));
        std::fs::write(&tmp, serde_json::to_vec_pretty(specs)?)
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path)?;
        Ok(hash)
    }

    /// The specs behind a hash, or `None` when the blob is not here.
    ///
    /// Absent is not empty: a missing blob means the surface cannot be rebuilt,
    /// never that the run had no tools.
    pub fn load(&self, hash: &str) -> Option<Vec<ToolSpec>> {
        let text = std::fs::read_to_string(self.path(hash)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(name: &str, description: &str) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            description: description.into(),
            input_schema: json!({"type": "object"}),
        }
    }

    fn scratch() -> SurfaceStore {
        let dir = std::env::temp_dir().join(format!(
            "mecha-surface-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        SurfaceStore::open(dir).unwrap()
    }

    /// Owner-only, on the front door's rule for every store root under
    /// `~/.mecha`: a `ToolSpec` carries the mail account short names baked
    /// into every tool schema, plus whatever an MCP server put in its own
    /// descriptions, so this is not nothing to protect.
    #[test]
    fn the_surface_store_directory_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("mecha-surface-perms-{}", uuid::Uuid::new_v4()));
        SurfaceStore::open(&dir).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The failure this module exists for: a name list cannot see it.
    #[test]
    fn a_re_described_tool_is_a_different_surface() {
        let before = [spec("fs_read", "Read a file.")];
        let after = [spec(
            "fs_read",
            "Read a file. Paths are workspace-relative.",
        )];
        assert_ne!(fingerprint(&before), fingerprint(&after));
        assert_eq!(
            before.iter().map(|s| &s.name).collect::<Vec<_>>(),
            after.iter().map(|s| &s.name).collect::<Vec<_>>(),
            "…and the names are identical, which is the whole problem"
        );
    }

    #[test]
    fn a_changed_schema_is_a_different_surface_too() {
        let mut other = spec("todo", "Keep a plan.");
        other.input_schema = json!({"type": "object", "properties": {"serves": {}}});
        assert_ne!(
            fingerprint(&[spec("todo", "Keep a plan.")]),
            fingerprint(&[other])
        );
    }

    #[test]
    fn the_same_surface_hashes_the_same_twice() {
        let s = [spec("a", "x"), spec("b", "y")];
        assert_eq!(fingerprint(&s), fingerprint(&s));
        // Order is part of the surface: it is the front of the cached prefix.
        let flipped = [spec("b", "y"), spec("a", "x")];
        assert_ne!(fingerprint(&s), fingerprint(&flipped));
    }

    /// The other three tests here compare two values computed in the same
    /// process, so none of them can fail if this hashed with the *wrong*
    /// hasher — the mistake this asserts against directly, on
    /// `the_rules_hash_is_stable_forever`'s precedent one module over. Pinned
    /// to `learning::rules_hash` over the exact canonical rendering rather
    /// than a second hex literal, so a change to either the rendering or the
    /// choice of hasher shows up here rather than only in a toolchain bump
    /// nobody connects back to this file.
    #[test]
    fn fingerprint_uses_the_stable_hasher_not_the_std_one() {
        let one = spec("build", "Build it.");
        let expected = crate::learning::rules_hash("build\0Build it.\0{\"type\":\"object\"}\0");
        assert_eq!(fingerprint(&[one]), expected);
    }

    /// **The rule the whole design turns on.** Every session on disk the day
    /// this ships has no hash, and none of them may read as faithful.
    #[test]
    fn a_recording_with_no_hash_is_unknown_and_never_a_match() {
        let live = [spec("fs_read", "Read a file.")];
        assert_eq!(Fidelity::of(None, &live), Fidelity::Unknown);
        assert!(Fidelity::Unknown.caveat().is_some());
        assert_ne!(Fidelity::of(None, &live), Fidelity::Matches);
    }

    /// And `Differs` needs no blob — which is why legibility lands the day the
    /// field ships, before any surface has accumulated.
    #[test]
    fn drift_is_detectable_with_nothing_but_the_hash() {
        let recorded = fingerprint(&[spec("fs_read", "Read a file.")]);
        let live = [spec(
            "fs_read",
            "Read a file. Paths are workspace-relative.",
        )];
        assert_eq!(Fidelity::of(Some(&recorded), &live), Fidelity::Differs);
        assert!(Fidelity::Differs.caveat().unwrap().contains("changed"));

        // No store was opened anywhere in this test.
        let same = [spec("fs_read", "Read a file.")];
        assert_eq!(Fidelity::of(Some(&recorded), &same), Fidelity::Matches);
        assert!(Fidelity::Matches.caveat().is_none());
    }

    #[test]
    fn a_surface_round_trips_and_a_second_record_writes_nothing_new() {
        let store = scratch();
        let specs = vec![spec("a", "one"), spec("b", "two")];
        let hash = store.record(&specs).unwrap();
        // Compared by fingerprint rather than by field, which is also the
        // assertion that matters: what round-trips is the *surface*.
        assert_eq!(fingerprint(&store.load(&hash).unwrap()), hash);

        let again = store.record(&specs).unwrap();
        assert_eq!(again, hash);
        let files = std::fs::read_dir(store.root()).unwrap().count();
        assert_eq!(files, 1, "one blob per distinct surface, not per record");
    }

    /// A missing blob is a surface that cannot be rebuilt, never a run that
    /// had no tools — the distinction every reader over these stores makes.
    #[test]
    fn a_missing_blob_is_absent_and_not_empty() {
        let store = scratch();
        assert!(store.load("0000000000000000").is_none());
    }
}
