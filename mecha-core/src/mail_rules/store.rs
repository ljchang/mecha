use super::*;
use anyhow::{bail, ensure, Context, Result};
use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::{
    fs::{MetadataExt, OpenOptionsExt},
    io::AsRawFd,
};
use std::path::{Path, PathBuf};

/// Contention is a skipped pass; unrelated lock/I/O errors still fail.
#[derive(Debug, thiserror::Error)]
#[error("another mail rules apply is running")]
pub struct ApplyRunning;

pub struct RuleStore {
    root: PathBuf,
    #[cfg(test)]
    pub(super) fail_next_ledger_append: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    pub(super) mode_transactions: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    pub(super) ledger_rows_parsed: std::sync::atomic::AtomicUsize,
}

/// A pass-local cursor over the append-only ledger. Never a persisted cache.
#[derive(Default)]
pub(super) struct LedgerCursor {
    identity: Option<(u64, u64)>,
    offset: u64,
    modified: Option<std::time::SystemTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimStatus {
    Claimed,
    MessageClosed,
    ThreadClosed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionClaim {
    /// Correlates this result with the supplied pending row, even when skipped.
    pub pending_id: String,
    pub action: ActionIdentity,
    pub status: ClaimStatus,
}

pub(super) struct StoreLock {
    _file: File,
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        // Closing alone releases flock only when the last descriptor sharing
        // its open file description closes. Parallel fork/exec can retain a
        // copy until exec, spuriously blocking the next run of this same store.
        // Explicit unlock ends this guard's ownership regardless of such copies.
        loop {
            // SAFETY: this guard still owns an open descriptor.
            if unsafe { libc::flock(self._file.as_raw_fd(), libc::LOCK_UN) } == 0
                || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
            {
                break;
            }
        }
    }
}

/// Fixed segments of `/api/mail/rules/…`, which take precedence over a rule's
/// name there: a rule so named could be installed but never opened on the page.
pub const RESERVED_RULE_NAMES: &[&str] = &["add", "ledger", "pause-all", "undo", "wrong-match"];

/// What an admission check is asked about.
enum Admission<'a> {
    /// A new rule, installed in shadow.
    Install(&'a LoadedRule),
    /// An edited rule replacing the installed one.
    Replace(&'a LoadedRule),
    /// One group raised.
    Raise {
        rule: &'a LoadedRule,
        group: ActionGroup,
        from: Mode,
        to: Mode,
    },
}

impl RuleStore {
    pub fn root_under(home: &Path) -> PathBuf {
        home.join("mail-rules")
    }

    pub fn open_default() -> Result<Self> {
        Self::open(Self::root_under(&crate::work::mecha_home()?))
    }

    /// Explicit roots are for isolated homes; no project/layered config is read.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        crate::create_private_dir(&root)?;
        let root = root.canonicalize()?;
        let rules = root.join("rules");
        // Check existing links before chmod: opening a store must not change
        // permissions on a directory outside its root.
        match fs::symlink_metadata(&rules) {
            Ok(_) => {
                contained(&root, &rules)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        crate::create_private_dir(&rules)?;
        contained(&root, &rules)?;
        Ok(Self {
            root,
            #[cfg(test)]
            fail_next_ledger_append: Default::default(),
            #[cfg(test)]
            mode_transactions: Default::default(),
            #[cfg(test)]
            ledger_rows_parsed: Default::default(),
        })
    }

    /// Read an existing store without creating directories or changing modes.
    pub fn open_existing(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into().canonicalize()?;
        contained(&root, &root.join("rules"))?;
        Ok(Self {
            root,
            #[cfg(test)]
            fail_next_ledger_append: Default::default(),
            #[cfg(test)]
            mode_transactions: Default::default(),
            #[cfg(test)]
            ledger_rows_parsed: Default::default(),
        })
    }

    /// Separate from the short writer lock: serialize engine effects and cap
    /// reservations without holding a store transaction across network I/O.
    pub(super) fn engine_lease(&self) -> Result<StoreLock> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(self.path(".engine.lock")?)?;
        // SAFETY: the descriptor stays owned until the run completes.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::WouldBlock {
                return Err(ApplyRunning.into());
            }
            return Err(error).context("locking mail rule engine");
        }
        Ok(StoreLock { _file: file })
    }

    /// Names survive an unreadable definition so the panic button can revoke it.
    pub fn rule_names(&self) -> Result<Vec<String>> {
        let mut names: BTreeSet<_> = self.approvals()?.into_iter().map(|row| row.rule).collect();
        for entry in fs::read_dir(contained(&self.root, &self.root.join("rules"))?)? {
            let name = entry?
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF8 rule name"))?;
            if !name.starts_with('.') {
                names.insert(name);
            }
        }
        for name in &names {
            crate::persona::validate_name(name)?;
        }
        Ok(names.into_iter().collect())
    }

    /// Emergency enumeration: junk cannot prevent revocation of other rules.
    /// Historical names still reach lower_modes even if their files disappeared.
    pub(super) fn pause_names(&self) -> Result<(Vec<String>, Vec<String>)> {
        let mut names = BTreeSet::new();
        let mut skipped = BTreeSet::new();
        for row in self.approvals()? {
            if crate::persona::validate_name(&row.rule).is_ok() {
                names.insert(row.rule);
            } else {
                skipped.insert(row.rule);
            }
        }
        for entry in fs::read_dir(contained(&self.root, &self.root.join("rules"))?)? {
            let entry = entry?;
            let filename = entry.file_name();
            let name = filename.to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if names.contains(&name) {
                continue; // Revoke from history even when this entry is broken.
            }
            if filename.to_str().is_none()
                || crate::persona::validate_name(&name).is_err()
                || !entry.file_type().is_ok_and(|kind| kind.is_dir())
                || self.get(&name).is_err()
            {
                skipped.insert(name);
            } else {
                names.insert(name);
            }
        }
        Ok((
            names.into_iter().collect(),
            skipped
                .into_iter()
                .map(|name| format!("skipping invalid rule entry {name:?}"))
                .collect(),
        ))
    }

    /// Read-only preflight for the approval screen; change_mode rechecks under
    /// the writer lock. The permanent ceiling has one source in built_in_holds.
    pub fn mode_ceiling(&self, name: &str, group: ActionGroup) -> Result<Mode> {
        let rule = self.get(name)?;
        ensure!(
            rule.definition.groups().contains(&group),
            "rule has no such group"
        );
        Ok(approvals::mode_ceiling(&rule, group))
    }

    /// Freeze exactly the bytes shown before a later change_mode binds their digest.
    pub fn review_inputs(&self, name: &str) -> Result<ReviewInputs> {
        let rule = self.get(name)?;
        let inputs = directory_inputs(&rule.directory)?;
        ensure!(
            digest_inputs(&inputs)? == rule.approval_digest.directory,
            "rule changed during review"
        );
        let mut files = BTreeMap::new();
        for (name, bytes) in inputs {
            files.insert(
                format!("rule/{name}"),
                String::from_utf8(bytes).context("approval input is not UTF-8")?,
            );
        }
        for (name, expected) in &rule.approval_digest.skills {
            let directory = self
                .root
                .parent()
                .context("store has no home")?
                .join("skills")
                .join(name)
                .canonicalize()?;
            let inputs = directory_inputs(&directory)?;
            ensure!(
                &digest_inputs(&inputs)? == expected,
                "skill changed during review"
            );
            for (file, bytes) in inputs {
                files.insert(format!("skill/{name}/{file}"), String::from_utf8(bytes)?);
            }
        }
        if rule.definition.uses_limits() {
            for (name, expected) in [
                ("aliases.toml", &rule.approval_digest.aliases),
                ("limits.toml", &rule.approval_digest.limits),
            ] {
                let bytes = match fs::read(self.path(name)?) {
                    Ok(bytes) => Some(bytes),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => return Err(error.into()),
                };
                ensure!(
                    &bytes
                        .as_ref()
                        .map(|b| definition::hash(b))
                        .unwrap_or_default()
                        == expected,
                    "global rule input changed during review"
                );
                files.insert(
                    name.into(),
                    match bytes {
                        Some(bytes) => String::from_utf8(bytes)?,
                        None => "(absent; built-in defaults)".into(),
                    },
                );
            }
            files.insert(
                "effective-limits.json".into(),
                serde_json::to_string_pretty(&self.limits()?)?,
            );
        }
        ensure!(
            self.get(name)?.approval_digest == rule.approval_digest,
            "rule changed during review"
        );
        Ok(ReviewInputs {
            digest: rule.approval_digest,
            files,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, name: &str) -> Result<PathBuf> {
        let path = self.root.join(name);
        // symlink_metadata distinguishes missing from dangling links.
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                contained(&self.root, &path)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        Ok(path)
    }

    fn lock(&self) -> Result<StoreLock> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(self.path(".lock")?)?;
        // SAFETY: the owned descriptor remains open for the guard's lifetime.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error()).context("locking mail rules");
        }
        Ok(StoreLock { _file: file })
    }

    pub fn aliases(&self) -> Result<Aliases> {
        self.read_toml("aliases.toml")
    }
    pub fn limits(&self) -> Result<Limits> {
        self.read_toml("limits.toml")
    }

    fn read_toml<T: DeserializeOwned + Default>(&self, name: &str) -> Result<T> {
        match fs::read_to_string(self.path(name)?) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("reading {name}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Definition text remains editable even when its TOML cannot be loaded.
    pub fn rule_text(&self, name: &str) -> Result<String> {
        crate::persona::validate_name(name)?;
        let rules = contained(&self.root, &self.root.join("rules"))?;
        let directory = rules.join(name);
        ensure!(
            !fs::symlink_metadata(&directory)?.file_type().is_symlink(),
            "rule directory may not be a symlink"
        );
        let directory = contained(&rules, &directory)?;
        let file = contained(&directory, &directory.join("rule.toml"))?;
        Ok(fs::read_to_string(file)?)
    }

    pub fn get(&self, name: &str) -> Result<LoadedRule> {
        crate::persona::validate_name(name)?;
        let rules = contained(&self.root, &self.root.join("rules"))?;
        let path = rules.join(name);
        ensure!(
            !fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "rule directory may not be a symlink"
        );
        let directory = contained(&rules, &path)?;
        self.load_directory(name, &directory)
    }

    fn load_directory(&self, name: &str, directory: &Path) -> Result<LoadedRule> {
        let inputs = directory_inputs(directory)?;
        let rule_path = contained(directory, &directory.join("rule.toml"))?;
        let text = std::str::from_utf8(snapshot_file(&inputs, directory, &rule_path)?)?;
        let definition = RuleDefinition::parse(text)?;
        let raw: toml::Value = toml::from_str(text)?;
        let tables = raw
            .get("action")
            .and_then(toml::Value::as_array)
            .context("missing actions")?;
        let mut actions = Vec::new();
        let mut seen = BTreeSet::new();
        let mut template_refused = None;
        for (action, table) in definition.actions.iter().zip(tables) {
            let digest = definition::hash(&serde_json::to_vec(table)?);
            ensure!(
                seen.insert(digest.clone()),
                "rule declares the same action twice"
            );
            actions.push(ActionIdentity {
                verb: action.verb,
                digest,
            });
        }
        let mut files = BTreeMap::new();
        let mut skills = BTreeMap::new();
        for action in &definition.actions {
            for file in [&action.template, &action.note].into_iter().flatten() {
                let path = contained(directory, &directory.join(file))?;
                ensure!(path.is_file(), "named file is not a regular file");
                let bytes = snapshot_file(&inputs, directory, &path)?.to_vec();
                if action.group() == ActionGroup::TypedFields
                    && action.template.as_ref() == Some(file)
                {
                    ensure!(
                        std::str::from_utf8(&bytes)?
                            .lines()
                            .next()
                            .is_some_and(|line| line.starts_with("Subject:")
                                && !line["Subject:".len()..].trim().is_empty()),
                        "typed send template requires a Subject: line"
                    );
                }
                if action.group() == ActionGroup::FixedWords
                    && action.template.as_ref() == Some(file)
                {
                    // Refused at install and edit, where the owner is told
                    // which file; a rule already on disk loads with its
                    // reply held instead, so one brace in one template never
                    // stops every rule's pass (review of #856).
                    if let Err(e) = std::str::from_utf8(&bytes)
                        .map_err(anyhow::Error::from)
                        .and_then(super::send::validate_template)
                    {
                        template_refused = Some(format!("{file}: {e:#}"));
                    }
                }
                files.insert(file.clone(), bytes);
            }
            if let Some(skill_name) = &action.draft_with {
                // The normal store is under mecha_home; an explicit test root
                // uses the adjacent isolated skills directory, never the owner’s.
                let skill_root = self
                    .root
                    .parent()
                    .context("store has no home")?
                    .join("skills");
                let skill_root = skill_root
                    .canonicalize()
                    .context("missing global skill store")?;
                let skill_dir = contained(&skill_root, &skill_root.join(skill_name))?;
                contained(&skill_dir, &skill_dir.join("SKILL.md"))?;
                let (store, _) = crate::skill::SkillStore::load(&skill_root);
                let skill = store
                    .get(skill_name)
                    .context("draft_with names no loadable global skill")?;
                let dir = contained(&skill_root, &skill.dir)?;
                skills.insert(skill_name.clone(), directory_digest(&dir)?);
            }
        }
        // Parse even before hashing: unreadable configuration never looks empty.
        self.aliases()?;
        self.limits()?;
        let uses_limits = definition.uses_limits();
        let approval_digest = ApprovalDigest {
            directory: digest_inputs(&inputs)?,
            skills,
            aliases: if uses_limits {
                self.optional_digest("aliases.toml")?
            } else {
                String::new()
            },
            limits: if uses_limits {
                self.optional_digest("limits.toml")?
            } else {
                String::new()
            },
            // §6.2: a description rule with a task or send rests on the
            // compiled confirmation frame; a build that changes it lapses.
            // Asked through the same predicate as the match digest, so the
            // two cannot drift (review of #855, pass 6).
            confirmation_version: if definition.is_description()
                && definition
                    .groups()
                    .into_iter()
                    .any(super::describe::confirmation_decides)
            {
                crate::mail_triage::confirm::CONFIRMATION_VERSION.clone()
            } else {
                String::new()
            },
            extraction_version: String::new(),
        };
        Ok(LoadedRule {
            name: name.into(),
            definition,
            actions,
            directory: directory.into(),
            files,
            approval_digest,
            template_refused,
        })
    }

    fn optional_digest(&self, name: &str) -> Result<String> {
        match fs::read(self.path(name)?) {
            Ok(bytes) => Ok(definition::hash(&bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn list(&self) -> Result<Vec<LoadedRule>> {
        let mut rules = Vec::new();
        let directory = contained(&self.root, &self.root.join("rules"))?;
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF8 rule name"))?;
            if name.starts_with('.') {
                continue;
            } // unfinished install siblings
            rules.push(self.get(&name)?);
        }
        rules.sort_by(|a, b| {
            (a.definition.priority, &a.name).cmp(&(b.definition.priority, &b.name))
        });
        Ok(rules)
    }

    /// Reserve a never-used name with durable shadow rows before publishing
    /// its directory. A crash may consume a name, never reuse old authority.
    /// File names are stems under persona::validate_name plus `.md`.
    pub fn install(
        &self,
        name: &str,
        text: &str,
        files: &BTreeMap<String, String>,
    ) -> Result<LoadedRule> {
        crate::persona::validate_name(name)?;
        ensure!(
            !RESERVED_RULE_NAMES.contains(&name),
            "`{name}` is reserved by the rules page's routes; choose another name"
        );
        RuleDefinition::parse(text)?;
        for file in files.keys() {
            let stem = file
                .strip_suffix(".md")
                .context("named files must end in .md")?;
            crate::persona::validate_name(stem)?;
        }
        let _lock = self.lock()?;
        let rules = contained(&self.root, &self.root.join("rules"))?;
        let target = rules.join(name);
        ensure!(!target.try_exists()?, "rule already exists");
        ensure!(
            !self.approvals()?.iter().any(|row| row.rule == name)
                && !self.ledger()?.iter().any(|row| row.rule == name)
                && !self
                    .deferred()?
                    .iter()
                    .any(|entry| entry.actions.iter().any(|row| row.rule == name)),
            "rule name was previously used; choose a fresh name"
        );
        let temp = rules.join(format!(".{name}-{}.tmp", uuid::Uuid::new_v4()));
        crate::create_private_dir(&temp)?;
        let result = (|| {
            let temp = contained(&rules, &temp)?;
            write_new(&temp.join("rule.toml"), text.as_bytes())?;
            for (name, text) in files {
                write_new(&temp.join(name), text.as_bytes())?;
            }
            let rule = self.load_directory(name, &temp)?;
            if let Some(why) = &rule.template_refused {
                bail!("{why}");
            }
            self.admit_locked(Admission::Install(&rule), &self.approvals()?)?;
            File::open(&temp)?.sync_all()?;
            for group in rule.definition.groups() {
                self.append_approval_locked(&ApprovalRow {
                    id: uuid::Uuid::new_v4().to_string(),
                    rule: name.into(),
                    group,
                    at: Some(Utc::now()),
                    from_mode: Mode::Shadow,
                    to_mode: Mode::Shadow,
                    actor: crate::closure::Actor::Unknown,
                    actor_readable: true,
                    digests: rule.approval_digest.clone(),
                    reason: ApprovalReason::Install,
                })?;
            }
            fs::rename(&temp, &target)?;
            File::open(&rules)?.sync_all()?;
            self.get(name)
        })();
        if temp.exists() {
            let _ = fs::remove_dir_all(temp);
        }
        result
    }

    /// Replace only rule.toml. Preserve bundled files; validate the entire new
    /// snapshot and persist lapses before publishing bytes, so a failed write or
    /// a subsequent revert can never revive the old approval.
    pub fn edit(&self, name: &str, text: &str) -> Result<EditReport> {
        RuleDefinition::parse(text)?;
        crate::persona::validate_name(name)?;
        let _lock = self.lock()?;
        let rules = contained(&self.root, &self.root.join("rules"))?;
        let directory = rules.join(name);
        ensure!(directory.is_dir(), "unknown rule: {name}");
        ensure!(
            !fs::symlink_metadata(&directory)?.file_type().is_symlink(),
            "rule directory may not be a symlink"
        );
        let mut inputs = directory_inputs(&directory)?;
        let changed = inputs
            .get("rule.toml")
            .is_none_or(|bytes| bytes != text.as_bytes());
        inputs.insert("rule.toml".into(), text.as_bytes().to_vec());
        let temp = rules.join(format!(".{name}-{}.tmp", uuid::Uuid::new_v4()));
        crate::create_private_dir(&temp)?;
        let result: Result<EditReport> = (|| {
            for (file, bytes) in &inputs {
                let path = temp.join(file);
                crate::create_private_dir(path.parent().context("missing file parent")?)?;
                write_new(&path, bytes)?;
            }
            let candidate = self.load_directory(name, &temp)?;
            if let Some(why) = &candidate.template_refused {
                bail!("{why}");
            }
            let mut approvals = self.approvals()?;
            self.admit_locked(Admission::Replace(&candidate), &approvals)?;
            let ledger = self.ledger()?;
            let old = self.get(name);
            let targets = old
                .as_ref()
                .ok()
                .map(|old| engine::unacknowledged_brakes(old, &ledger, &approvals))
                .unwrap_or_default();
            struct EditHolds(BTreeMap<ActionGroup, Mode>);
            impl HoldProvider for EditHolds {
                fn holds(&self, rule: &LoadedRule, group: ActionGroup) -> Vec<ModeHold> {
                    let mut holds = owner_file_holds().holds(rule, group);
                    if let Some(ceiling) = self.0.get(&group) {
                        holds.push(ModeHold {
                            ceiling: *ceiling,
                            reason: "automatic brake".into(),
                        });
                    }
                    holds
                }
            }
            let holds = EditHolds(targets);
            let before = approvals.len();
            // A read-only observer may have seen an external edit but never
            // persisted its lapse. Restoring approved bytes must not bypass that
            // observation, nor recover auto from an unreadable definition.
            match &old {
                Ok(old) => {
                    self.lapse_locked(old, &mut approvals, &holds, Utc::now())?;
                }
                Err(_) => self.lapse_unreadable_locked(name, &mut approvals, Utc::now())?,
            }
            self.lapse_locked(&candidate, &mut approvals, &holds, Utc::now())?;
            let lapsed_groups: Vec<_> = approvals[before..]
                .iter()
                .filter(|row| row.reason == ApprovalReason::Lapse)
                .map(|row| row.group)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            if changed {
                fs::rename(temp.join("rule.toml"), directory.join("rule.toml"))?;
                File::open(&directory)?.sync_all()?;
            }
            Ok(EditReport {
                name: name.into(),
                changed_files: if changed {
                    vec!["rule.toml".into()]
                } else {
                    Vec::new()
                },
                approval_digest: candidate.approval_digest,
                notices: if lapsed_groups.is_empty() {
                    Vec::new()
                } else {
                    vec!["approval lapsed: files changed".into()]
                },
                lapsed_groups,
            })
        })();
        let cleanup = fs::remove_dir_all(&temp);
        let report = result?;
        cleanup?;
        Ok(report)
    }

    /// A correction closes its original action key even on an older reader.
    /// It never settles a pending attempt and never performs a restore.
    pub fn wrong_match(
        &self,
        row_id: &str,
        actor: crate::closure::Actor,
    ) -> Result<CorrectionReport> {
        let _lease = self.engine_lease()?;
        let (rule_name, report) = {
            let _lock = self.lock()?;
            let rows = self.ledger()?;
            let original = wrong_match_original(self, &rows, row_id)?;
            // One correction per action key: a second row of the same action
            // (an expired re-offer, a retry) is the same wrong match.
            if rows
                .iter()
                .any(|row| row.outcome == Outcome::WrongMatch && row.same_key(original))
            {
                (
                    original.rule.clone(),
                    CorrectionReport {
                        changed: false,
                        row: None,
                        message: "already marked wrong match; the matcher needs an edit".into(),
                    },
                )
            } else {
                let mut correction = original.clone();
                correction.id = uuid::Uuid::new_v4().to_string();
                correction.at = Some(Utc::now());
                correction.outcome = Outcome::WrongMatch;
                correction.closes = true;
                correction.settlement = None;
                correction.correction_of = Some(original.id.clone());
                correction.actor = Some(actor);
                correction.wrong_match = true;
                correction.reason =
                    "wrong match: the matcher is wrong; edit rule.toml (action not undone)".into();
                self.append_ledger_locked(&correction)?;
                (
                    original.rule.clone(),
                    CorrectionReport {
                        changed: true,
                        message: correction.reason.clone(),
                        row: Some(correction),
                    },
                )
            }
        };
        // Retry also applies the brake if the prior append succeeded but lowering
        // failed. Same engine lease as undo/apply; no second correction is written.
        super::undo::apply_correction_brake(self, &rule_name)?;
        Ok(report)
    }

    pub fn approvals(&self) -> Result<Vec<ApprovalRow>> {
        let rows: Vec<ApprovalRow> = self.read_rows("approvals.jsonl")?;
        for row in &rows {
            ensure!(
                !row.id.trim().is_empty() && !row.rule.trim().is_empty(),
                "approval row has no identifiable scope"
            );
        }
        Ok(rows)
    }
    pub fn ledger(&self) -> Result<Vec<LedgerRow>> {
        let rows: Vec<LedgerRow> = self.read_rows("ledger.jsonl")?;
        #[cfg(test)]
        self.ledger_rows_parsed
            .fetch_add(rows.len(), std::sync::atomic::Ordering::Relaxed);
        for row in &rows {
            row.validate_key()?;
        }
        Ok(rows)
    }
    /// Capture history and its exact byte boundary from the same descriptor.
    /// An append after that boundary belongs to the next incremental read.
    pub(super) fn ledger_snapshot(&self) -> Result<(Vec<LedgerRow>, LedgerCursor)> {
        let mut cursor = LedgerCursor::default();
        // Initial history must be readable in full, including a legacy final
        // object without a newline. Only incremental reads can await a writer's
        // unfinished append; never silently discard corrupt initial history.
        let rows = self.read_ledger(&mut cursor, false)?;
        Ok((rows, cursor))
    }

    /// Read newly appended evidence without reparsing the whole history at
    /// every dispatch. Replacement/truncation/editing is not an append and
    /// refuses this pass rather than dropping previously seen closure evidence.
    pub(super) fn ledger_updates(&self, cursor: &mut LedgerCursor) -> Result<Vec<LedgerRow>> {
        self.read_ledger(cursor, true)
    }

    fn read_ledger(
        &self,
        cursor: &mut LedgerCursor,
        complete_lines_only: bool,
    ) -> Result<Vec<LedgerRow>> {
        let mut file = match File::open(self.path("ledger.jsonl")?) {
            Ok(file) => file,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound && cursor.identity.is_none() =>
            {
                return Ok(Vec::new())
            }
            Err(error) => return Err(error.into()),
        };
        let metadata = file.metadata()?;
        let identity = (metadata.dev(), metadata.ino());
        let modified = metadata.modified()?;
        ensure!(
            cursor.identity.is_none_or(|old| old == identity) && metadata.len() >= cursor.offset,
            "ledger replaced or truncated during apply"
        );
        if cursor.identity.is_some() && metadata.len() == cursor.offset {
            ensure!(
                cursor.modified == Some(modified),
                "ledger edited during apply"
            );
            return Ok(Vec::new());
        }
        file.seek(SeekFrom::Start(cursor.offset))?;
        let mut bytes = Vec::new();
        file.take(metadata.len() - cursor.offset)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 == metadata.len() - cursor.offset,
            "ledger truncated during read"
        );
        if complete_lines_only {
            // append() writes without holding this reader's lock. A trailing
            // fragment is not committed evidence yet: retry it from the same
            // offset next time, even if it is already syntactically valid JSON.
            let complete = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
            bytes.truncate(complete);
        }
        let rows: Vec<LedgerRow> = bytes
            .split(|b| *b == b'\n')
            .filter(|line| !line.iter().all(u8::is_ascii_whitespace))
            .map(|line| serde_json::from_slice(line).context("unreadable ledger.jsonl row"))
            .collect::<Result<_>>()?;
        for row in &rows {
            row.validate_key()?;
        }
        #[cfg(test)]
        self.ledger_rows_parsed
            .fetch_add(rows.len(), std::sync::atomic::Ordering::Relaxed);
        cursor.identity = Some(identity);
        cursor.offset += bytes.len() as u64;
        cursor.modified = Some(modified);
        Ok(rows)
    }

    pub fn deferred(&self) -> Result<Vec<DeferredEntry>> {
        let entries: Vec<DeferredEntry> = self.read_rows("deferred.jsonl")?;
        for entry in &entries {
            for row in &entry.actions {
                row.validate_key()?;
            }
        }
        Ok(entries)
    }

    fn read_rows<T: DeserializeOwned>(&self, name: &str) -> Result<Vec<T>> {
        let bytes = match fs::read(self.path(name)?) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        // Unlike an audit-only reader, losing a row here could authorize a
        // duplicate action or resurrect an approval. Refuse a partial history.
        bytes
            .split(|b| *b == b'\n')
            .enumerate()
            .filter(|(_, line)| !line.iter().all(u8::is_ascii_whitespace))
            .map(|(index, line)| {
                serde_json::from_slice(line)
                    .with_context(|| format!("unreadable {name} row at line {}", index + 1))
            })
            .collect()
    }

    fn append<T: Serialize>(&self, name: &str, row: &T) -> Result<()> {
        // Every caller holds the writer lock. Accept a complete final object
        // without a newline, but validate the history before extending it.
        self.read_rows::<serde_json::Value>(name)?;
        let path = self.path(name)?;
        let existing = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        let mut bytes = serde_json::to_vec(row)?;
        bytes.push(b'\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)?;
        if !existing.is_empty() && existing.last() != Some(&b'\n') {
            file.write_all(b"\n")?;
        }
        file.write_all(&bytes)?;
        file.sync_all()?;
        File::open(&self.root)?.sync_all()?;
        Ok(())
    }

    /// The single approval append boundary. Reads preceding history under the
    /// caller's lock; no lapse, brake, install or CLI change may bypass it.
    fn append_approval_locked(&self, row: &ApprovalRow) -> Result<()> {
        crate::persona::validate_name(&row.rule)?;
        ensure!(!row.id.trim().is_empty(), "approval row needs an id");
        let rows = self.approvals()?;
        let row_ids: HashSet<_> = rows.iter().map(|r| r.id.as_str()).collect();
        ensure!(
            !row_ids.contains(row.id.as_str()),
            "duplicate approval row id"
        );
        let preceding = approvals::approval_state(&row.rule, row.group, &rows);
        ensure!(
            approvals::valid_transition(row, &preceding),
            "unreadable or unauthorized approval transition"
        );
        self.append("approvals.jsonl", row)
    }

    fn rewrite<T: Serialize>(&self, name: &str, rows: &[T]) -> Result<()> {
        let mut bytes = Vec::new();
        for row in rows {
            serde_json::to_writer(&mut bytes, row)?;
            bytes.push(b'\n');
        }
        let target = self.path(name)?;
        let temp = self
            .root
            .join(format!(".{name}-{}.tmp", uuid::Uuid::new_v4()));
        write_new(&temp, &bytes)?;
        fs::rename(&temp, target)?;
        File::open(&self.root)?.sync_all()?;
        Ok(())
    }

    /// A named-group change may raise only under the actor gate. Re-approval
    /// binds the shown digest so edits between preview and acceptance refuse.
    pub fn change_mode(
        &self,
        name: &str,
        group: ActionGroup,
        requested: Mode,
        actor: Option<crate::closure::Actor>,
        shown: &ApprovalDigest,
        holds: &dyn HoldProvider,
    ) -> Result<ModeDecision> {
        self.change_mode_checked(name, group, requested, actor, shown, holds, None, false)
    }

    /// Optional web review guards are checked under the same lock as the write.
    #[allow(clippy::too_many_arguments)]
    pub fn change_mode_checked(
        &self,
        name: &str,
        group: ActionGroup,
        requested: Mode,
        actor: Option<crate::closure::Actor>,
        shown: &ApprovalDigest,
        holds: &dyn HoldProvider,
        expected: Option<&ApprovalDigest>,
        require_digest: bool,
    ) -> Result<ModeDecision> {
        crate::persona::validate_name(name)?;
        let _lock = self.lock()?;
        let mut rows = self.approvals()?;
        let preceding_mode = approvals::approval_state(name, group, &rows).mode;
        let loaded = self.get(name);
        if let Some(expected) = expected {
            ensure!(
                loaded
                    .as_ref()
                    .is_ok_and(|rule| &rule.approval_digest == expected),
                ModeError::StaleDigest
            );
        }

        match &loaded {
            Ok(rule) => {
                self.lapse_locked(rule, &mut rows, holds, Utc::now())?;
            }
            Err(_) => {
                self.lapse_unreadable_locked(name, &mut rows, Utc::now())?;
            }
        }
        ensure!(
            Self::approval_groups(name, &rows, loaded.as_ref().ok()).contains(&group),
            "rule has no such group"
        );
        let (recorded, current, digests, mode_holds) =
            Self::mode_inputs(name, group, &rows, loaded.as_ref().ok(), holds);
        ensure!(requested != Mode::Unknown, "cannot request an unknown mode");
        // A requested decrease stays a decrease even if the failed load has
        // already forced a stricter ceiling. Re-approval is not a decrease.
        let target = if loaded.is_err() && requested.rank() < preceding_mode.rank() {
            requested.lower(current)
        } else {
            requested
        };
        let mut decision = gate_mode(current, target, actor)?;
        decision.holds = mode_holds;
        decision.requested = requested;
        decision.clamped |= target != requested;
        // Revocation never depends on loadable files or a fresh review. Only
        // raises may bind a reviewed definition, including clearing a hold.
        if decision.target.rank() > current.rank() {
            ensure!(
                !require_digest || expected.is_some(),
                ModeError::DigestRequired
            );
            let rule = loaded?;
            ensure!(
                rule.definition.groups().contains(&group),
                "cannot raise a group absent from the current rule"
            );
            ensure!(&rule.approval_digest == shown, ModeError::StaleDigest);
            // Use the same permanent ceilings as resolution. Standing holds
            // remain computed, but an unsupported permission must never be
            // recorded as if it had been granted.
            if decision.target.rank() > approvals::mode_ceiling(&rule, group).rank() {
                decision.refused = true;
                decision.write = false;
                decision.target = current;
                return Ok(decision);
            }
            self.admit_locked(
                Admission::Raise {
                    rule: &rule,
                    group,
                    from: recorded,
                    to: decision.target,
                },
                &rows,
            )?;
        }
        decision.write = decision.write || gate_mode(recorded, decision.target, actor)?.write;
        if decision.write {
            self.append_approval_locked(&ApprovalRow {
                id: uuid::Uuid::new_v4().to_string(),
                rule: name.into(),
                group,
                at: Some(Utc::now()),
                from_mode: recorded,
                to_mode: decision.target,
                actor: actor.unwrap_or(crate::closure::Actor::Unknown),
                actor_readable: true,
                digests,
                reason: if actor == Some(crate::closure::Actor::Owner) {
                    ApprovalReason::Owner
                } else {
                    ApprovalReason::StepDown
                },
            })?;
        }
        Ok(decision)
    }

    /// §4.2's admission check, under the writer lock and before any file or
    /// mode row is written: an install, an edit that adds list membership, or
    /// a raise that would put the description lists over
    /// [`describe::LIST_CAP`] is refused, naming the count and the rules
    /// whose demotion would make room. A store already over the cap (a hand
    /// edit, a safety lowering) refuses only what would add to it; lowerings
    /// are never checked, since they must always be able to stop actions.
    fn admit_locked(&self, change: Admission<'_>, approvals: &[ApprovalRow]) -> Result<()> {
        let ledger = self.ledger()?;
        let mut rules: Vec<LoadedRule> = self
            .rule_names()?
            .iter()
            .filter_map(|name| self.get(name).ok())
            .collect();
        let (before, _) = describe::entry_count(&rules, approvals, &ledger);
        let mut after_approvals = approvals.to_vec();
        let synthetic = |rule: &LoadedRule, group, from, to, actor, reason| ApprovalRow {
            id: uuid::Uuid::new_v4().to_string(),
            rule: rule.name.clone(),
            group,
            at: Some(Utc::now()),
            from_mode: from,
            to_mode: to,
            actor,
            actor_readable: true,
            digests: rule.approval_digest.clone(),
            reason,
        };
        let name = match change {
            Admission::Install(rule) => {
                for group in rule.definition.groups() {
                    after_approvals.push(synthetic(
                        rule,
                        group,
                        Mode::Shadow,
                        Mode::Shadow,
                        crate::closure::Actor::Unknown,
                        ApprovalReason::Install,
                    ));
                }
                rules.retain(|r| r.name != rule.name);
                rules.push(rule.clone());
                &rule.name
            }
            Admission::Replace(rule) => {
                rules.retain(|r| r.name != rule.name);
                rules.push(rule.clone());
                &rule.name
            }
            Admission::Raise {
                rule,
                group,
                from,
                to,
            } => {
                after_approvals.push(synthetic(
                    rule,
                    group,
                    from,
                    to,
                    crate::closure::Actor::Owner,
                    ApprovalReason::Owner,
                ));
                rules.retain(|r| r.name != rule.name);
                rules.push(rule.clone());
                &rule.name
            }
        };
        let (after, shares) = describe::entry_count(&rules, &after_approvals, &ledger);
        if after > describe::LIST_CAP && after > before {
            return Err(ModeError::ListCap(describe::over_cap(after, &shares, name)).into());
        }
        Ok(())
    }

    fn approval_groups(
        name: &str,
        rows: &[ApprovalRow],
        rule: Option<&LoadedRule>,
    ) -> BTreeSet<ActionGroup> {
        rows.iter()
            .filter(|row| row.rule == name && row.group != ActionGroup::Unknown)
            .map(|row| row.group)
            .chain(rule.into_iter().flat_map(|rule| rule.definition.groups()))
            .collect()
    }

    fn mode_inputs(
        name: &str,
        group: ActionGroup,
        rows: &[ApprovalRow],
        rule: Option<&LoadedRule>,
        holds: &dyn HoldProvider,
    ) -> (Mode, Mode, ApprovalDigest, Vec<ModeHold>) {
        if let Some(rule) = rule {
            let resolved = resolve_mode(rule, group, rows, holds);
            (
                resolved.recorded,
                resolved.effective,
                rule.approval_digest.clone(),
                resolved.holds,
            )
        } else {
            let state = approvals::approval_state(name, group, rows);
            (
                state.mode,
                state.mode,
                state.digest.cloned().unwrap_or_default(),
                Vec::new(),
            )
        }
    }

    // An unreadable definition has no digest we can bless. Persist shadow using
    // the historical groups and digests; restoration cannot revive permission.
    // Existing shadow/off rows already provide a durable ceiling.
    fn lapse_unreadable_locked(
        &self,
        name: &str,
        rows: &mut Vec<ApprovalRow>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let mut affected = Vec::new();
        for group in Self::approval_groups(name, rows, None) {
            let state = approvals::approval_state(name, group, rows);
            if state.mode.rank() <= Mode::Shadow.rank() {
                continue;
            }
            let row = ApprovalRow {
                id: uuid::Uuid::new_v4().to_string(),
                rule: name.into(),
                group,
                at: Some(now),
                from_mode: state.mode,
                to_mode: Mode::Shadow,
                actor: crate::closure::Actor::Unknown,
                actor_readable: true,
                digests: state.digest.cloned().unwrap_or_default(),
                reason: ApprovalReason::Lapse,
            };
            self.append_approval_locked(&row)?;
            rows.push(row);
            affected.push(group);
        }
        self.append_lapse_notice(name, &affected, "rule inputs unreadable", now)
    }

    /// Persist each detected lapse before returning modes, so reverting bytes
    /// cannot recover an older auto approval. The returned groups are notices.
    pub fn detect_lapses(
        &self,
        name: &str,
        holds: &dyn HoldProvider,
        now: DateTime<Utc>,
    ) -> Result<Vec<ActionGroup>> {
        #[cfg(test)]
        self.mode_transactions
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        crate::persona::validate_name(name)?;
        let _lock = self.lock()?;
        let mut rows = self.approvals()?;
        match self.get(name) {
            Ok(rule) => self.lapse_locked(&rule, &mut rows, holds, now),
            Err(error) => {
                self.lapse_unreadable_locked(name, &mut rows, now)?;
                Err(error)
            }
        }
    }

    fn lapse_locked(
        &self,
        rule: &LoadedRule,
        rows: &mut Vec<ApprovalRow>,
        holds: &dyn HoldProvider,
        now: DateTime<Utc>,
    ) -> Result<Vec<ActionGroup>> {
        let mut affected = Vec::new();
        let groups = Self::approval_groups(&rule.name, rows, Some(rule));
        for group in groups {
            let resolved = resolve_mode(rule, group, rows, holds);
            if !resolved.lapsed {
                continue;
            }
            let row = ApprovalRow {
                id: uuid::Uuid::new_v4().to_string(),
                rule: rule.name.clone(),
                group,
                at: Some(now),
                from_mode: resolved.recorded.effective(),
                to_mode: resolved.effective.lower(Mode::Propose),
                actor: crate::closure::Actor::Unknown,
                actor_readable: true,
                digests: rule.approval_digest.clone(),
                reason: ApprovalReason::Lapse,
            };
            self.append_approval_locked(&row)?;
            rows.push(row);
            affected.push(group);
        }
        self.append_lapse_notice(&rule.name, &affected, "files changed", now)?;
        Ok(affected)
    }

    fn append_lapse_notice(
        &self,
        name: &str,
        affected: &[ActionGroup],
        reason: &str,
        now: DateTime<Utc>,
    ) -> Result<()> {
        if !affected.is_empty() {
            let row = LedgerRow {
                id: uuid::Uuid::new_v4().to_string(),
                rule: name.into(),
                at: Some(now),
                message: MessageRef::default(),
                action: ActionIdentity::default(),
                group: ActionGroup::Unknown,
                mode: Mode::Shadow,
                match_digest: MatchDigest::default(),
                verdict_input: VerdictInput::NotApplicable,
                outcome: Outcome::ApprovalLapsed,
                closes: false,
                settlement: None,
                correction_of: None,
                in_inbox: None,
                received_at: None,
                headers_partial: false,
                matched_because: String::new(),
                reason: reason.into(),
                failure_kind: FailureKind::Unknown,
                wrong_match: false,
                affected_groups: affected.to_vec(),
                actor: None,
                outbox_item: None,
            };
            self.append("ledger.jsonl", &row)?;
        }
        Ok(())
    }

    /// Whole-rule pause or group-specific brakes can only lower. Off groups
    /// stay untouched. Automatic callers pass Unknown, never impersonate Owner.
    pub fn lower_modes(
        &self,
        name: &str,
        targets: &BTreeMap<ActionGroup, Mode>,
        actor: Option<crate::closure::Actor>,
        holds: &dyn HoldProvider,
        reason: ApprovalReason,
    ) -> Result<Vec<ModeDecision>> {
        #[cfg(test)]
        self.mode_transactions
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        crate::persona::validate_name(name)?;
        let _lock = self.lock()?;
        let mut rows = self.approvals()?;
        let loaded = self.get(name);
        match &loaded {
            Ok(rule) => {
                self.lapse_locked(rule, &mut rows, holds, Utc::now())?;
            }
            Err(_) => {
                self.lapse_unreadable_locked(name, &mut rows, Utc::now())?;
            }
        }
        let mut decisions = Vec::new();
        for group in Self::approval_groups(name, &rows, loaded.as_ref().ok()) {
            let Some(target) = targets.get(&group) else {
                continue;
            };
            ensure!(*target != Mode::Unknown, "unknown lowering target");
            let (recorded, current, digests, mode_holds) =
                Self::mode_inputs(name, group, &rows, loaded.as_ref().ok(), holds);
            if current == Mode::Off {
                continue;
            }
            let mut decision = gate_mode(current, current.lower(*target), actor)?;
            decision.holds = mode_holds;
            decision.write = decision.write || gate_mode(recorded, decision.target, actor)?.write;
            if decision.write {
                self.append_approval_locked(&ApprovalRow {
                    id: uuid::Uuid::new_v4().to_string(),
                    rule: name.into(),
                    group,
                    at: Some(Utc::now()),
                    from_mode: recorded,
                    to_mode: decision.target,
                    actor: actor.unwrap_or(crate::closure::Actor::Unknown),
                    actor_readable: true,
                    digests,
                    reason,
                })?;
            }
            decisions.push(decision);
        }
        Ok(decisions)
    }

    pub fn append_ledger(&self, row: &LedgerRow) -> Result<()> {
        let _lock = self.lock()?;
        self.append_ledger_locked(row)
    }

    /// Claim one occasion's actions under the same lock as the key lookup.
    /// Returns one result per supplied row, in input order. Closed message keys
    /// skip only that action; a closed thread skips all outbound/Board actions.
    /// All decisions use the same pre-claim snapshot, so newly claimed siblings
    /// cannot suppress each other. Only claimed pending rows are appended.
    pub fn claim_actions(&self, pending: &[LedgerRow]) -> Result<Vec<ActionClaim>> {
        let first = pending.first().context("no actions to claim")?;
        let mut ids = BTreeSet::new();
        let mut identities = BTreeSet::new();
        for row in pending {
            row.validate_key()?;
            ensure!(
                row.outcome == Outcome::Pending && row.settlement.is_none(),
                "claim needs pending rows"
            );
            ensure!(
                row.rule == first.rule
                    && row.message.account == first.message.account
                    && row.message.thread_id == first.message.thread_id
                    && row.message.provider_message_id == first.message.provider_message_id,
                "claim must describe one rule and message"
            );
            ensure!(
                ids.insert(&row.id) && identities.insert(&row.action.digest),
                "duplicate claimed action"
            );
        }
        let _lock = self.lock()?;
        let rows = self.ledger()?;
        let row_ids: HashSet<_> = rows.iter().map(|r| r.id.as_str()).collect();
        for row in pending {
            ensure!(
                !row_ids.contains(row.id.as_str()),
                "duplicate ledger row id"
            );
        }
        let occasion_closed = thread_closed(
            &rows,
            &ThreadKey {
                rule: first.rule.clone(),
                account: first.message.account.clone(),
                thread_id: first.message.thread_id.clone(),
            },
        );
        let claims: Vec<_> = pending
            .iter()
            .map(|row| {
                let status = if row.group.once_per_thread() && occasion_closed {
                    ClaimStatus::ThreadClosed
                } else if message_closed(
                    &rows,
                    &MessageKey {
                        rule: row.rule.clone(),
                        account: row.message.account.clone(),
                        provider_message_id: row.message.provider_message_id.clone(),
                        action: row.action.clone(),
                    },
                ) {
                    ClaimStatus::MessageClosed
                } else {
                    ClaimStatus::Claimed
                };
                ActionClaim {
                    pending_id: row.id.clone(),
                    action: row.action.clone(),
                    status,
                }
            })
            .collect();
        for (row, claim) in pending.iter().zip(&claims) {
            if claim.status == ClaimStatus::Claimed {
                self.append("ledger.jsonl", row)?;
            }
        }
        Ok(claims)
    }

    fn append_ledger_locked(&self, row: &LedgerRow) -> Result<()> {
        #[cfg(test)]
        ensure!(
            !self
                .fail_next_ledger_append
                .swap(false, std::sync::atomic::Ordering::SeqCst),
            "injected ledger append failure"
        );
        row.validate_key()?;
        let rows = self.ledger()?;
        let row_ids: HashSet<_> = rows.iter().map(|r| r.id.as_str()).collect();
        ensure!(
            !row_ids.contains(row.id.as_str()),
            "duplicate ledger row id"
        );
        if row.outcome == Outcome::WrongMatch {
            let id = row
                .correction_of
                .as_deref()
                .context("wrong-match must name its original row")?;
            let original = wrong_match_original(self, &rows, id)?;
            ensure!(
                row.same_key(original)
                    && row.match_digest == original.match_digest
                    && row.verdict_input == original.verdict_input
                    && row.mode == original.mode,
                "wrong-match must retain the original measurement and action key"
            );
            ensure!(
                row.closes && row.wrong_match && row.settlement.is_none(),
                "wrong-match must close its key without settling an attempt"
            );
            ensure!(
                !rows.iter().any(|prior| prior.outcome == Outcome::WrongMatch
                    && prior.correction_of.as_deref() == Some(id)),
                "this row already has a wrong-match correction"
            );
        }
        if row.outcome == Outcome::UndoPending {
            let original = rows
                .iter()
                .find(|original| Some(&original.id) == row.correction_of.as_ref())
                .context("undo intent names no completed row")?;
            ensure!(
                original.outcome == Outcome::Done
                    && original.group == ActionGroup::Mailbox
                    && row.same_key(original),
                "undo intent must name its completed mailbox action"
            );
            ensure!(
                row.settlement.is_none() && row.closes,
                "undo intent must close its undo key"
            );
            ensure!(
                !rows.iter().any(|prior| prior.same_key(original)
                    && prior.outcome != Outcome::WrongMatch
                    && (prior.correction_of.is_some() || prior.outcome == Outcome::Undone)),
                "this mailbox action already has an undo correction"
            );
        }
        if let Some(id) = &row.settlement {
            let pending = rows
                .iter()
                .find(|r| &r.id == id)
                .context("settlement names no pending row")?;
            ensure!(
                row.settles(pending),
                "settlement must name its own pending attempt"
            );
            ensure!(
                !rows.iter().any(|r| r.settles(pending)),
                "attempt is already settled"
            );
        }
        self.append("ledger.jsonl", row)
    }

    /// Accepts only non-closing WouldHave snapshots without settlement links.
    /// Returns the retained entry id, including when this thread was already queued.
    pub fn enqueue(&self, entry: &DeferredEntry) -> Result<String> {
        for row in &entry.actions {
            row.validate_key()?;
            ensure!(
                row.outcome == Outcome::WouldHave && !row.closes && row.settlement.is_none(),
                "deferred entries need unexecuted snapshots, not execution rows"
            );
        }
        ensure!(
            !entry.actions.is_empty(),
            "deferred entry needs action snapshots"
        );
        ensure!(
            entry
                .actions
                .iter()
                .all(|r| r.message.account == entry.account
                    && r.message.thread_id == entry.thread_id),
            "deferred actions must belong to the queued thread"
        );
        let _lock = self.lock()?;
        // Pending producers may still add snapshots while a consumer settles
        // another message. Expiry, however, must never renew an occasion.
        let rows: Vec<_> = self
            .ledger()?
            .into_iter()
            .filter(|row| row.outcome == Outcome::Expired)
            .collect();
        for action in &entry.actions {
            let closed = message_closed(
                &rows,
                &MessageKey {
                    rule: action.rule.clone(),
                    account: action.message.account.clone(),
                    provider_message_id: action.message.provider_message_id.clone(),
                    action: action.action.clone(),
                },
            ) || (action.group.once_per_thread()
                && thread_closed(
                    &rows,
                    &ThreadKey {
                        rule: action.rule.clone(),
                        account: action.message.account.clone(),
                        thread_id: action.message.thread_id.clone(),
                    },
                ));
            ensure!(!closed, "deferred occasion already expired");
        }
        let mut queue = self.deferred()?;
        let effective_id = if let Some(existing) = queue
            .iter_mut()
            .find(|q| q.account == entry.account && q.thread_id == entry.thread_id)
        {
            for action in &entry.actions {
                if !existing.actions.iter().any(|r| {
                    r.rule == action.rule
                        && r.message.provider_message_id == action.message.provider_message_id
                        && r.action == action.action
                }) {
                    existing.actions.push(action.clone());
                }
            }
            // Repeated timer sightings must not postpone expiry.
            existing.id.clone()
        } else {
            ensure!(
                !queue.iter().any(|q| q.id == entry.id),
                "duplicate deferred id"
            );
            queue.push(entry.clone());
            entry.id.clone()
        };
        self.rewrite("deferred.jsonl", &queue)?;
        Ok(effective_id)
    }

    /// Outcomes land before removal. Retrying after a crash recognizes the row
    /// ids already appended, so no settled work disappears without its trace.
    pub fn settle_deferred(&self, id: &str, outcomes: &[LedgerRow]) -> Result<()> {
        let _lock = self.lock()?;
        ensure!(!outcomes.is_empty(), "no deferred outcomes supplied");
        let mut queue = self.deferred()?;
        let entry = queue
            .iter_mut()
            .find(|q| q.id == id)
            .context("no deferred entry")?;
        let mut completed = BTreeSet::new();
        let ledger = self.ledger()?;
        for row in outcomes {
            row.validate_key()?;
            ensure!(
                !matches!(
                    row.outcome,
                    Outcome::Pending | Outcome::ApprovalLapsed | Outcome::Unknown
                ),
                "invalid deferred outcome"
            );
            let snapshot = entry
                .actions
                .iter()
                .position(|action| row.same_key(action))
                .context("outcome has no matching deferred snapshot")?;
            ensure!(completed.insert(snapshot), "duplicate deferred outcome");
            if let Some(existing) = ledger.iter().find(|old| old.id == row.id) {
                ensure!(
                    serde_json::to_value(existing)? == serde_json::to_value(row)?,
                    "outcome id already belongs to a different row"
                );
            }
        }
        for row in outcomes {
            if !ledger.iter().any(|old| old.id == row.id) {
                self.append_ledger_locked(row)?;
            }
        }
        // The entry may have grown since the worker read it. Only the supplied
        // outcomes' snapshots are settled; retain every concurrently added one.
        let mut index = 0;
        entry.actions.retain(|_| {
            let keep = !completed.contains(&index);
            index += 1;
            keep
        });
        queue.retain(|q| q.id != id || !q.actions.is_empty());
        self.rewrite("deferred.jsonl", &queue)
    }

    pub fn expire_deferred(&self, now: DateTime<Utc>) -> Result<Vec<DeferredEntry>> {
        self.expire_deferred_matching(now, None, &BTreeSet::new(), &BTreeSet::new())
    }

    /// An engine pass expires only accounts whose listing completed successfully.
    pub(super) fn expire_deferred_for_account(
        &self,
        account: &str,
        now: DateTime<Utc>,
        held_rules: &BTreeSet<String>,
        held_groups: &BTreeSet<(String, ActionGroup)>,
    ) -> Result<Vec<DeferredEntry>> {
        self.expire_deferred_matching(now, Some(account), held_rules, held_groups)
    }

    fn expire_deferred_matching(
        &self,
        now: DateTime<Utc>,
        account: Option<&str>,
        held_rules: &BTreeSet<String>,
        held_groups: &BTreeSet<(String, ActionGroup)>,
    ) -> Result<Vec<DeferredEntry>> {
        let _lock = self.lock()?;
        let held = |row: &LedgerRow| {
            held_rules.contains(&row.rule) || held_groups.contains(&(row.rule.clone(), row.group))
        };
        let mut queue = self.deferred()?;
        let expires = |entry: &DeferredEntry| {
            account.is_none_or(|account| entry.account == account) && entry.expired(now)
        };
        let expired: Vec<_> = queue
            .iter()
            .filter(|q| expires(q))
            .cloned()
            .filter_map(|mut entry| {
                entry.actions.retain(|row| !held(row));
                (!entry.actions.is_empty()).then_some(entry)
            })
            .collect();
        let mut ledger = self.ledger()?;
        for entry in &expired {
            for action in &entry.actions {
                let mut row = action.clone();
                row.id = format!("expired:{}:{}", entry.id, action.id);
                if ledger.iter().any(|r| r.id == row.id) {
                    continue;
                }
                row.at = Some(now);
                row.outcome = Outcome::Expired;
                row.closes = true;
                row.settlement = None;
                row.reason = "deferred for more than seven days".into();
                row.failure_kind = FailureKind::Unknown;
                self.append("ledger.jsonl", &row)?;
                ledger.push(row);
            }
        }
        // A standing hold is not a closing occasion. Expire only eligible
        // snapshots, retaining held siblings in the same queue entry.
        for entry in &mut queue {
            if expires(entry) {
                entry.actions.retain(held);
            }
        }
        queue.retain(|q| !expires(q) || !q.actions.is_empty());
        if !expired.is_empty() {
            self.rewrite("deferred.jsonl", &queue)?;
        }
        Ok(expired)
    }
}

fn contained(root: &Path, path: &Path) -> Result<PathBuf> {
    let resolved = path
        .canonicalize()
        .with_context(|| format!("resolving {}", path.display()))?;
    ensure!(
        resolved.starts_with(root),
        "path escapes {}",
        root.display()
    );
    Ok(resolved)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn directory_inputs(root: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    fn walk(
        root: &Path,
        path: &Path,
        names: &mut BTreeMap<String, Vec<u8>>,
        ancestors: &mut BTreeSet<PathBuf>,
    ) -> Result<()> {
        let resolved = contained(root, path)?;
        if resolved.is_dir() {
            ensure!(
                ancestors.insert(resolved.clone()),
                "directory symlink cycle"
            );
            for entry in fs::read_dir(path)? {
                walk(root, &entry?.path(), names, ancestors)?;
            }
            ancestors.remove(&resolved);
        } else if resolved.is_file() {
            let name = path
                .strip_prefix(root)?
                .to_str()
                .context("non-UTF8 rule file name")?;
            names.insert(name.into(), fs::read(resolved)?);
        } else {
            bail!("rule input is not a regular file or directory");
        }
        Ok(())
    }
    let mut names = BTreeMap::new();
    walk(root, root, &mut names, &mut BTreeSet::new())?;
    Ok(names)
}

fn snapshot_file<'a>(
    inputs: &'a BTreeMap<String, Vec<u8>>,
    root: &Path,
    path: &Path,
) -> Result<&'a [u8]> {
    let name = path
        .strip_prefix(root)?
        .to_str()
        .context("non-UTF8 file name")?;
    inputs
        .get(name)
        .map(Vec::as_slice)
        .context("rule files changed during load")
}

fn digest_inputs(inputs: &BTreeMap<String, Vec<u8>>) -> Result<String> {
    let hashes: BTreeMap<_, _> = inputs
        .iter()
        .map(|(name, bytes)| (name, definition::hash(bytes)))
        .collect();
    Ok(definition::hash(&serde_json::to_vec(&hashes)?))
}

fn directory_digest(root: &Path) -> Result<String> {
    digest_inputs(&directory_inputs(root)?)
}

#[cfg(test)]
mod lease_tests {
    use super::*;

    #[test]
    fn an_inherited_descriptor_cannot_retain_a_completed_engine_lease() {
        let root = std::env::temp_dir().join(format!("rule-lease-{}", uuid::Uuid::new_v4()));
        let store = RuleStore::open(&root).unwrap();
        let lease = store.engine_lease().unwrap();
        // dup and fork share the same open file description. No real child or
        // process-global test state is needed to reproduce that shared lifetime.
        let inherited = lease._file.try_clone().unwrap();
        assert!(store.engine_lease().is_err());
        drop(lease);
        let next = store.engine_lease().expect(
            "completed run must release the lease even while a forked child retains its descriptor",
        );
        drop(next);
        drop(inherited);
        std::fs::remove_dir_all(root).unwrap();
    }
}
