use super::*;
use anyhow::{bail, ensure, Context, Result};
use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::{fs::OpenOptionsExt, io::AsRawFd};
use std::path::{Path, PathBuf};

pub struct RuleStore {
    root: PathBuf,
}

struct StoreLock {
    _file: File,
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
        fs::create_dir_all(&root)?;
        let root = root.canonicalize()?;
        let rules = root.join("rules");
        if !rules.try_exists()? {
            fs::create_dir(&rules)?;
        }
        contained(&root, &rules)?;
        Ok(Self { root })
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
            confirmation_version: String::new(),
            extraction_version: String::new(),
        };
        Ok(LoadedRule {
            name: name.into(),
            definition,
            actions,
            directory: directory.into(),
            files,
            approval_digest,
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
        fs::create_dir(&temp)?;
        let result = (|| {
            let temp = contained(&rules, &temp)?;
            write_new(&temp.join("rule.toml"), text.as_bytes())?;
            for (name, text) in files {
                write_new(&temp.join(name), text.as_bytes())?;
            }
            let rule = self.load_directory(name, &temp)?;
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
        for row in &rows {
            row.validate_key()?;
        }
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
            .filter(|line| !line.iter().all(u8::is_ascii_whitespace))
            .map(|line| {
                serde_json::from_slice(line).with_context(|| format!("unreadable {name} row"))
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
        ensure!(
            !rows.iter().any(|old| old.id == row.id),
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
        crate::persona::validate_name(name)?;
        let _lock = self.lock()?;
        let mut rows = self.approvals()?;
        let preceding_mode = approvals::approval_state(name, group, &rows).mode;
        let loaded = self.get(name);
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
            let rule = loaded?;
            ensure!(
                rule.definition.groups().contains(&group),
                "cannot raise a group absent from the current rule"
            );
            ensure!(
                &rule.approval_digest == shown,
                "rule inputs changed since review"
            );
            // Use the same permanent ceilings as resolution. Standing holds
            // remain computed, but an unsupported permission must never be
            // recorded as if it had been granted.
            if approvals::built_in_holds(&rule, group)
                .iter()
                .any(|hold| decision.target.rank() > hold.ceiling.rank())
            {
                decision.refused = true;
                decision.write = false;
                decision.target = current;
                return Ok(decision);
            }
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
                matched_because: String::new(),
                reason: reason.into(),
                failure_kind: FailureKind::Unknown,
                wrong_match: false,
                affected_groups: affected.to_vec(),
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
    /// Sibling actions share the pre-claim thread check, so one task does not
    /// suppress another action in the same occasion. False means already handled.
    pub fn claim_actions(&self, pending: &[LedgerRow]) -> Result<bool> {
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
        for row in pending {
            ensure!(
                !rows.iter().any(|old| old.id == row.id),
                "duplicate ledger row id"
            );
            if message_closed(
                &rows,
                &MessageKey {
                    rule: row.rule.clone(),
                    account: row.message.account.clone(),
                    provider_message_id: row.message.provider_message_id.clone(),
                    action: row.action.clone(),
                },
            ) || (row.group.once_per_thread()
                && thread_closed(
                    &rows,
                    &ThreadKey {
                        rule: row.rule.clone(),
                        account: row.message.account.clone(),
                        thread_id: row.message.thread_id.clone(),
                    },
                ))
            {
                return Ok(false);
            }
        }
        for row in pending {
            self.append("ledger.jsonl", row)?;
        }
        Ok(true)
    }

    fn append_ledger_locked(&self, row: &LedgerRow) -> Result<()> {
        row.validate_key()?;
        let rows = self.ledger()?;
        ensure!(
            !rows.iter().any(|r| r.id == row.id),
            "duplicate ledger row id"
        );
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

    /// Returns the retained entry id, including when this thread was already queued.
    pub fn enqueue(&self, entry: &DeferredEntry) -> Result<String> {
        for row in &entry.actions {
            row.validate_key()?;
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
                    && r.message.thread_id == entry.thread_id
                    && r.outcome != Outcome::ApprovalLapsed
                    && r.settlement.is_none()),
            "deferred actions must belong to the queued thread"
        );
        let _lock = self.lock()?;
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
        let _lock = self.lock()?;
        let mut queue = self.deferred()?;
        let expired: Vec<_> = queue.iter().filter(|q| q.expired(now)).cloned().collect();
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
                row.closes = false;
                row.settlement = None;
                row.reason = "deferred for more than seven days".into();
                row.failure_kind = FailureKind::Unknown;
                self.append("ledger.jsonl", &row)?;
                ledger.push(row);
            }
        }
        queue.retain(|q| !q.expired(now));
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
