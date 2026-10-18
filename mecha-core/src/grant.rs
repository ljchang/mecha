//! Access outside the workspace, granted by the owner.
//!
//! The path jail ([`crate::tool::ToolCtx::resolve`]) is one root and the
//! spill directory. A personal assistant also needs the owner's own files —
//! a folder of papers, a project checkout — and the old answer, `!cp` it in,
//! makes the owner the file clerk. A grant is the jail's other exception: a
//! directory (or file) the owner said this run may read, or read and write.
//!
//! Three rules carry the design (owner's rulings, 2026-10-18):
//!
//! - **Only a person widens the jail.** A standing grant is a line in the
//!   *global* config (`[[grant]]`, stripped from every other layer, because a
//!   cloned repository must not bring its own jail with it) or an owner's
//!   `/grant`. A model may only *ask*, through `request_access`, and the ask
//!   goes to [`crate::tool::Approver::escalate`] — never `approve`, which a
//!   permission mode, an "always" list or `--yes` may answer with nobody
//!   consulted. Headless, the ask ends `Blocked`.
//! - **A live grant lasts for the conversation**, the same unit taint lives
//!   on: resuming keeps it ([`crate::session::Record::Grants`]), a new
//!   conversation starts without it, a subagent inherits its parent's.
//!   Making one permanent is the owner editing their config.
//! - **Some places are never grantable**, whoever asks ([`admit`]): a root
//!   that contains or sits inside the mecha home (OAuth tokens, every
//!   transcript, the learning store) or a credential directory. One human
//!   "yes" is exactly what an injection asking for `~/.ssh` is engineering,
//!   so the answer is refused before a person is put in the position to give
//!   it.
//!
//! What a grant does *not* change: anything read through one arms
//! `private_data` exactly as a workspace read does, so the trifecta
//! interlock still stands between those bytes and a destination the model
//! names. A grant widens what can be read, never where it can go.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// What a grant allows. `Write` includes reading.
///
/// Strict where the owner writes it (`[[grant]] access`, a typo is a parse
/// error, not a quietly narrower grant) and lenient where the harness reads
/// it back: in the session file an unknown value is `Read`, the narrower
/// answer ([`lenient_access`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    #[default]
    Read,
    Write,
}

/// A recorded access, an unknown one read as `Read` — a closed enum in an
/// append-only store is a wire format, and the degraded value must be the
/// one that allows less.
fn lenient_access<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Access, D::Error> {
    let s = String::deserialize(d)?;
    Ok(match s.as_str() {
        "write" => Access::Write,
        _ => Access::Read,
    })
}

impl Access {
    pub fn as_str(self) -> &'static str {
        match self {
            Access::Read => "read",
            Access::Write => "write",
        }
    }

    /// The words a person approving one reads.
    pub fn describe(self) -> &'static str {
        match self {
            Access::Read => "read",
            Access::Write => "read and write",
        }
    }

    pub fn parse(s: &str) -> Option<Access> {
        match s {
            "read" | "r" | "ro" => Some(Access::Read),
            "write" | "w" | "rw" => Some(Access::Write),
            _ => None,
        }
    }
}

/// Who made a grant. Shown beside it, never consulted for a decision: every
/// source passed [`admit`], and a person stood behind each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// A `[[grant]]` line in the global config.
    Config,
    /// The owner typed `/grant`.
    Owner,
    /// The model asked (`request_access`) and a person said yes.
    Approved,
    #[serde(other)]
    Unknown,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Config => "config",
            Source::Owner => "owner",
            Source::Approved => "approved",
            Source::Unknown => "unknown",
        }
    }
}

/// One exception to the jail: a canonical path, and what it allows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    /// Canonical, as [`admit`] returned it — containment is decided against
    /// canonical paths on both sides, or a symlink is a way around it.
    pub path: PathBuf,
    #[serde(default, deserialize_with = "lenient_access")]
    pub access: Access,
    pub source: Source,
}

impl Grant {
    /// Does this grant cover `resolved` (already canonical) for `access`?
    pub fn covers(&self, resolved: &Path, access: Access) -> bool {
        self.access >= access && resolved.starts_with(&self.path)
    }
}

/// A standing grant as the global config spells it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantConfig {
    pub path: PathBuf,
    #[serde(default)]
    pub access: Access,
}

/// Directories under `$HOME` no grant may contain or enter: where
/// credentials, keys and other agents' transcripts live. Not a complete list
/// of secrets — nothing could be — but the places an injection asking for
/// one would name first. A grant of `$HOME` itself is refused by the same
/// rule, since it contains all of these.
const NEVER_GRANTED_UNDER_HOME: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".kube",
    ".docker",
    ".netrc",
    ".password-store",
    ".pki",
    ".claude",
    ".codex",
    ".config/gcloud",
    ".config/gh",
    ".config/google-chrome",
    ".config/chromium",
    ".config/BraveSoftware",
    ".mozilla",
    ".local/share/keyrings",
    "Library/Keychains",
];

/// The places [`admit`] refuses to overlap, canonical where they exist.
///
/// An error when the mecha home cannot be named at all: a set that silently
/// lost its most important member would admit a grant over it, so no grant
/// is admitted until it can be (review of #711).
fn protected() -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let canon = |p: PathBuf| p.canonicalize().unwrap_or(p);
    // Both spellings of the mecha home: `MECHA_HOME` is settable by any
    // command line, the password database's is not (`owner_mecha_home`).
    let home = crate::work::mecha_home()
        .context("cannot tell where the mecha home is, so no grant can be checked against it")?;
    out.push(canon(home));
    if let Some(h) = crate::work::owner_mecha_home() {
        out.push(canon(h));
    }
    if let Some(home) = dirs::home_dir() {
        for rel in NEVER_GRANTED_UNDER_HOME {
            out.push(canon(home.join(rel)));
        }
    }
    Ok(out)
}

/// Expand a leading `~` the way config paths are expanded.
pub fn expand(raw: &str) -> PathBuf {
    let p = Path::new(raw.trim());
    match (p.strip_prefix("~"), dirs::home_dir()) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => p.to_path_buf(),
    }
}

/// Prove a requested root may be granted, and return it canonical.
///
/// The one gate every source passes — config at load, `/grant`, an approved
/// `request_access`, and a recorded grant re-checked on resume (the list
/// below may have grown since it was written, and the path may now be a
/// symlink to somewhere else). Refuses:
///
/// - a relative path (relative to *what* is the question a grant must not
///   leave open — the workspace is the one thing it is not about);
/// - a path that does not exist (canonicalization is the proof, and a grant
///   of a future path would be decided against a name, not a place);
/// - the filesystem root, and any root that contains or sits inside the
///   mecha home or a credential directory ([`NEVER_GRANTED_UNDER_HOME`]).
pub fn admit(path: &Path) -> Result<PathBuf> {
    admit_against(path, &protected()?)
}

fn admit_against(path: &Path, protected: &[PathBuf]) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!(
            "{} is not an absolute path — name the directory in full (or from `~/`)",
            path.display()
        );
    }
    let canonical = path
        .canonicalize()
        .with_context(|| format!("{} does not exist or cannot be opened", path.display()))?;
    if canonical.parent().is_none() {
        bail!("the filesystem root cannot be granted");
    }
    for p in protected {
        if p.starts_with(&canonical) {
            bail!(
                "{} contains {}, which is never granted (credentials, or the mecha home's \
                 tokens and transcripts) — grant a directory beside it instead",
                canonical.display(),
                p.display()
            );
        }
        if canonical.starts_with(p) {
            bail!(
                "{} is inside {}, which is never granted (credentials, or the mecha home's \
                 tokens and transcripts)",
                canonical.display(),
                p.display()
            );
        }
    }
    Ok(canonical)
}

/// The grants a run holds: the standing ones from config, and the live ones
/// this conversation acquired.
///
/// `live` is a cell because a tool adds to it mid-run (`request_access`) and
/// tools see only `&ToolCtx`. [`Grants::for_run`] mints a fresh cell for
/// every run — the same discipline `run_in` applies to `compact_requested`:
/// one agent serves many concurrent conversations, and a shared cell would
/// hand one conversation's grant to another.
#[derive(Debug, Clone, Default)]
pub struct Grants {
    standing: Arc<[Grant]>,
    live: Arc<Mutex<Vec<Grant>>>,
    /// Whether `request_access` is dispatchable in the run this context
    /// belongs to, so a refusal can name it truthfully — a name in a
    /// refusal for a tool that is not there is a dead end dressed as a
    /// route. Stamped by `Agent::run_in` from the run's own registry; off
    /// everywhere else.
    pub requestable: bool,
    /// `--no-grants` (`Lever::Grants`): this run reaches no grant, standing
    /// or the conversation's, and can add none. Carried through every
    /// derivation so a resumed conversation's recorded grants cannot come
    /// back in through `for_run` while the record says the lever is off
    /// (review of #711).
    off: bool,
}

impl Grants {
    /// The standing grants, from config entries already through [`admit`].
    pub fn standing(grants: Vec<Grant>) -> Self {
        Grants {
            standing: Arc::from(grants),
            ..Grants::default()
        }
    }

    /// Admit each config line, keeping what passes and warning about what
    /// does not — a refused grant is a narrowing, so skipping it fails
    /// closed; the warning is so it does not look applied.
    pub fn from_config(lines: &[GrantConfig]) -> (Self, Vec<String>) {
        let mut ok = Vec::new();
        let mut refused = Vec::new();
        for line in lines {
            match admit(&line.path) {
                Ok(path) => ok.push(Grant {
                    path,
                    access: line.access,
                    source: Source::Config,
                }),
                Err(e) => refused.push(format!("[[grant]] {}: {e:#}", line.path.display())),
            }
        }
        (Grants::standing(ok), refused)
    }

    /// Grants switched off for every run on this context (`--no-grants`).
    pub fn off() -> Self {
        Grants {
            off: true,
            ..Grants::default()
        }
    }

    /// Whether grants are switched off here. A run that is off leaves the
    /// conversation's grants as they were rather than carrying back none:
    /// not using a grant is not ending it.
    pub fn is_off(&self) -> bool {
        self.off
    }

    /// A fresh live cell for one run, holding what this context already had
    /// live (a parent's grants, for a subagent) and what the conversation
    /// carries — nothing at all when grants are off.
    pub fn for_run(&self, conversation: &[Grant]) -> Self {
        if self.off {
            return Grants::off();
        }
        let mut live = self.live();
        for g in conversation {
            if !live.contains(g) {
                live.push(g.clone());
            }
        }
        Grants {
            standing: self.standing.clone(),
            live: Arc::new(Mutex::new(live)),
            requestable: self.requestable,
            off: false,
        }
    }

    /// The same standing grants with no live ones — a new isolation domain
    /// (`ToolCtx::with_workspace`).
    pub fn standing_only(&self) -> Self {
        Grants {
            standing: self.standing.clone(),
            live: Arc::default(),
            requestable: self.requestable,
            off: self.off,
        }
    }

    /// No grants at all — a room that must account for everything it
    /// touches (an incognito chat).
    pub fn none() -> Self {
        Grants::default()
    }

    /// The live grants, to be carried back onto the conversation.
    pub fn live(&self) -> Vec<Grant> {
        self.live.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Every grant: standing in config order, then live in the order they
    /// were granted — stable, so the mount list a sandbox builds is too.
    pub fn all(&self) -> Vec<Grant> {
        let mut out: Vec<Grant> = self.standing.to_vec();
        out.extend(self.live());
        out
    }

    pub fn is_empty(&self) -> bool {
        self.standing.is_empty() && self.live().is_empty()
    }

    /// The grant covering a canonical path for `access`, if any.
    pub fn covering(&self, resolved: &Path, access: Access) -> Option<Grant> {
        self.all().into_iter().find(|g| g.covers(resolved, access))
    }

    /// Whether some grant covers the path for reading but not for `access`
    /// — the difference between "outside" and "read-only here".
    pub fn read_only_at(&self, resolved: &Path) -> Option<Grant> {
        self.all()
            .into_iter()
            .find(|g| g.access == Access::Read && resolved.starts_with(&g.path))
    }

    /// Add a live grant. An existing grant at least as wide over the same or
    /// an enclosing root makes this a no-op: `Ok(true)` is new, `Ok(false)`
    /// already held. A list that cannot be read is an error, never `false`
    /// — the caller renders `false` as "already granted", and nothing would
    /// have been stored (review of #711).
    pub fn add(&self, grant: Grant) -> Result<bool> {
        if self.off {
            bail!("grants are switched off for this run (--no-grants); nothing was granted");
        }
        if self.covering(&grant.path, grant.access).is_some() {
            return Ok(false);
        }
        match self.live.lock() {
            Ok(mut live) => {
                live.push(grant);
                Ok(true)
            }
            Err(_) => bail!("the conversation's grant list could not be read; nothing was granted"),
        }
    }
}

/// The owner's `/grant <path> [read|write]`, applied to a conversation's
/// live list — shared by every front end that offers it, so they cannot
/// disagree about the syntax. The access is the last word when it parses as
/// one, and everything before it is the path, so a name with spaces works.
/// Returns what to say.
pub fn grant_by_owner(arg: &str, standing: &Grants, live: &mut Vec<Grant>) -> Result<String> {
    if standing.is_off() {
        bail!("grants are switched off for this session (--no-grants)");
    }
    let arg = arg.trim();
    let (raw, access) = match arg.rsplit_once(char::is_whitespace) {
        Some((path, word)) if Access::parse(word).is_some() => {
            (path.trim(), Access::parse(word).unwrap_or_default())
        }
        _ => (arg, Access::Read),
    };
    let path = admit(&expand(raw))?;
    let held = standing.for_run(live);
    if let Some(g) = held.covering(&path, access) {
        return Ok(format!(
            "already granted: {} ({}, {})",
            g.path.display(),
            g.access.describe(),
            g.source.as_str()
        ));
    }
    live.push(Grant {
        path: path.clone(),
        access,
        source: Source::Owner,
    });
    Ok(format!(
        "granted for this conversation: {} ({}) — /clear ends it; a [[grant]] line keeps it",
        path.display(),
        access.describe()
    ))
}

/// The owner's `/revoke <path>`: drop this conversation's live grants at or
/// under `path`. It only narrows, so nothing is asked; and since the session
/// file's last `Record::Grants` is the set, the next run makes it durable
/// across a resume. The path need not exist any more — a grant on a
/// directory since removed is still revocable by the name it was granted
/// under. A standing grant is config, so it is named, not touched.
pub fn revoke_by_owner(arg: &str, standing: &Grants, live: &mut Vec<Grant>) -> Result<String> {
    let raw = expand(arg);
    if !raw.is_absolute() {
        bail!("name the directory in full (or from `~/`)");
    }
    // Both spellings: the canonical one grants are stored under, and the one
    // typed, for a directory that no longer canonicalizes.
    let canonical = raw.canonicalize().ok();
    let under =
        |p: &Path| p.starts_with(&raw) || canonical.as_ref().is_some_and(|c| p.starts_with(c));
    let before = live.len();
    let mut ended = Vec::new();
    live.retain(|g| {
        let hit = under(&g.path);
        if hit {
            ended.push(format!("{} ({})", g.path.display(), g.access.describe()));
        }
        !hit
    });
    if live.len() < before {
        return Ok(format!(
            "revoked for this conversation: {}",
            ended.join(", ")
        ));
    }
    let standing_hit: Vec<String> = standing
        .standing
        .iter()
        .filter(|g| under(&g.path) || canonical.as_ref().is_some_and(|c| c.starts_with(&g.path)))
        .map(|g| g.path.display().to_string())
        .collect();
    if !standing_hit.is_empty() {
        bail!(
            "{} is granted by your config ([[grant]] {}) — remove that line to end it",
            raw.display(),
            standing_hit.join(", ")
        );
    }
    bail!(
        "this conversation holds no grant at or under {}",
        raw.display()
    )
}

/// One line per grant a conversation holds, standing first.
pub fn listing(standing: &Grants, live: &[Grant]) -> Vec<String> {
    standing
        .for_run(live)
        .all()
        .iter()
        .map(|g| {
            format!(
                "{}  {}  ({})",
                g.access.as_str(),
                g.path.display(),
                g.source.as_str()
            )
        })
        .collect()
}

/// Fold recorded live grants back through [`admit`] on resume, dropping any
/// that would no longer be admitted. What was approved yesterday is
/// re-proven today against today's list and today's filesystem.
pub fn readmit(recorded: Vec<Grant>) -> Vec<Grant> {
    let mut out: Vec<Grant> = Vec::new();
    for g in recorded {
        match admit(&g.path) {
            Ok(path) => {
                let g = Grant { path, ..g };
                if !out.contains(&g) {
                    out.push(g);
                }
            }
            Err(e) => tracing::warn!(
                path = %g.path.display(),
                "a recorded grant is no longer admitted and was dropped: {e:#}"
            ),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);
    impl Dir {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn dir() -> Dir {
        let d = std::env::temp_dir().join(format!("mecha-grant-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }

    #[test]
    fn a_relative_or_missing_path_is_never_admitted() {
        assert!(admit_against(Path::new("papers"), &[]).is_err());
        let d = dir();
        assert!(admit_against(&d.path().join("not-there"), &[]).is_err());
        assert!(admit_against(Path::new("/"), &[]).is_err());
    }

    #[test]
    fn a_root_over_or_under_a_protected_place_is_refused() {
        let d = dir();
        let home = d.path().canonicalize().unwrap();
        let secret = home.join(".mecha");
        let papers = home.join("papers");
        std::fs::create_dir_all(secret.join("work")).unwrap();
        std::fs::create_dir_all(&papers).unwrap();
        let protected = vec![secret.clone()];
        // Containing it: `$HOME` holds the mecha home.
        let over = admit_against(&home, &protected).unwrap_err().to_string();
        assert!(over.contains("contains"), "{over}");
        // Inside it.
        let under = admit_against(&secret.join("work"), &protected).unwrap_err();
        assert!(under.to_string().contains("inside"), "{under}");
        // Beside it is fine, and comes back canonical.
        assert_eq!(admit_against(&papers, &protected).unwrap(), papers);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_judged_by_where_it_lands() {
        let d = dir();
        let home = d.path().canonicalize().unwrap();
        let secret = home.join(".ssh");
        std::fs::create_dir_all(&secret).unwrap();
        let link = home.join("innocent");
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        assert!(admit_against(&link, &[secret]).is_err());
    }

    #[test]
    fn coverage_respects_access_and_containment() {
        let g = Grant {
            path: PathBuf::from("/data/papers"),
            access: Access::Read,
            source: Source::Owner,
        };
        assert!(g.covers(Path::new("/data/papers/a.pdf"), Access::Read));
        assert!(!g.covers(Path::new("/data/papers/a.pdf"), Access::Write));
        // A sibling sharing a prefix is not inside: components, not bytes.
        assert!(!g.covers(Path::new("/data/papers-old/a.pdf"), Access::Read));
        let w = Grant {
            access: Access::Write,
            ..g
        };
        assert!(w.covers(Path::new("/data/papers/a.pdf"), Access::Read));
    }

    #[test]
    fn each_run_gets_its_own_live_cell() {
        let base = Grants::standing(Vec::new());
        let a = base.for_run(&[]);
        let b = base.for_run(&[]);
        a.add(Grant {
            path: PathBuf::from("/data/a"),
            access: Access::Read,
            source: Source::Approved,
        })
        .unwrap();
        assert_eq!(a.live().len(), 1);
        assert!(b.live().is_empty(), "a grant leaked into another run");
        assert!(
            base.live().is_empty(),
            "a grant leaked into the agent's base context"
        );
        // A child minted from a run carries the run's grants.
        assert_eq!(a.for_run(&[]).live().len(), 1);
        assert!(a.standing_only().live().is_empty());
    }

    #[test]
    fn a_narrower_or_equal_grant_adds_nothing() {
        let g = Grants::default().for_run(&[]);
        let wide = Grant {
            path: PathBuf::from("/data"),
            access: Access::Write,
            source: Source::Owner,
        };
        assert!(g.add(wide).unwrap());
        assert!(!g
            .add(Grant {
                path: PathBuf::from("/data/papers"),
                access: Access::Read,
                source: Source::Approved,
            })
            .unwrap());
        assert_eq!(g.live().len(), 1);
    }

    /// A list that cannot be read is an error, never "already granted":
    /// `request_access` renders `Ok(false)` as success, and nothing would
    /// have been stored (review of #711).
    #[test]
    fn a_poisoned_list_is_an_error_not_already_granted() {
        let g = Grants::default().for_run(&[]);
        let live = Arc::clone(&g.live);
        let _ = std::thread::spawn(move || {
            let _held = live.lock().unwrap();
            panic!("poison the list");
        })
        .join();
        let added = g.add(Grant {
            path: PathBuf::from("/data"),
            access: Access::Read,
            source: Source::Approved,
        });
        assert!(added.is_err(), "{added:?}");
    }

    /// `/revoke` drops what is at or under the path and nothing beside it,
    /// works on a directory since removed, and leaves a standing grant to
    /// the config that made it.
    #[test]
    fn revoke_ends_live_grants_under_a_path_and_names_standing_ones() {
        let d = dir();
        let root = d.path().canonicalize().unwrap();
        let (papers, notes, gone) = (root.join("papers"), root.join("notes"), root.join("gone"));
        let g = |p: &Path| Grant {
            path: p.to_path_buf(),
            access: Access::Read,
            source: Source::Owner,
        };
        let mut live = vec![g(&papers.join("2024")), g(&papers), g(&notes), g(&gone)];
        let none = Grants::default();

        let said = revoke_by_owner(&papers.display().to_string(), &none, &mut live).unwrap();
        assert!(said.contains("2024"), "{said}");
        assert_eq!(live, vec![g(&notes), g(&gone)], "only at or under the path");
        // `gone` never existed on disk: revoked by the name it was given.
        revoke_by_owner(&gone.display().to_string(), &none, &mut live).unwrap();
        assert_eq!(live, vec![g(&notes)]);
        assert!(revoke_by_owner(&papers.display().to_string(), &none, &mut live).is_err());
        assert!(
            revoke_by_owner("notes", &none, &mut live).is_err(),
            "relative"
        );

        let standing = Grants::standing(vec![Grant {
            source: Source::Config,
            ..g(&papers)
        }]);
        let err = revoke_by_owner(&papers.display().to_string(), &standing, &mut live)
            .unwrap_err()
            .to_string();
        assert!(err.contains("[[grant]]"), "{err}");
        assert_eq!(live, vec![g(&notes)]);
    }

    #[test]
    fn an_unknown_access_reads_back_as_read() {
        let g: Grant =
            serde_json::from_str(r#"{"path":"/x","access":"admin","source":"later"}"#).unwrap();
        assert_eq!(g.access, Access::Read);
        assert_eq!(g.source, Source::Unknown);
    }
}
