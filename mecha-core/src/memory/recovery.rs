//! Encrypted full-store snapshots; no live replacement or source-file backup.
use super::*;
use anyhow::{ensure, Context, Result};
use rusqlite::OpenFlags;
use serde::Serialize;
use std::{fs, path::Path};
#[path = "recovery_files.rs"]
mod files;
#[path = "recovery_inventory.rs"]
pub(super) mod inventory;
use files::{FileIdentity, FileOps, Stage, SystemFiles};

#[derive(Debug, Serialize)]
pub struct BackupReport {
    pub tables: usize,
    pub rows: u64,
    pub schema_version: i64,
    pub sequence: i64,
    pub security_epoch: i64,
    pub scope: &'static str,
}
impl Memory {
    /// Writes a new encrypted schema-preserving snapshot, never replaces a live file.
    /// Keys, raw JSONL and external authority/artifact files are not backed up.
    pub fn backup_encrypted(
        &self,
        destination: &Path,
        destination_storage: StorageAccess,
    ) -> Result<BackupReport> {
        self.backup_with_files(destination, destination_storage, &SystemFiles)
    }
    fn backup_with_files(
        &self,
        destination: &Path,
        destination_storage: StorageAccess,
        ops: &impl FileOps,
    ) -> Result<BackupReport> {
        ensure!(
            self.storage_access().is_encrypted() && destination_storage.is_encrypted(),
            "snapshot source and destination must be encrypted"
        );
        ensure!(
            self.conn.is_autocommit(),
            "cannot snapshot an active caller transaction"
        );
        let destination = sql::validate_path(destination)?;
        ensure!(
            !destination.try_exists()? && fs::symlink_metadata(&destination).is_err(),
            "snapshot destination already exists"
        );
        let source_path = sql::validate_existing_path(Path::new(
            self.conn.path().context("missing memory path")?,
        ))?;
        let file_identity = FileIdentity::at(&source_path)?;
        let source = self.reopen_read_only()?;
        let pinned = source.conn.unchecked_transaction()?;
        inventory::integrity(&pinned)?;
        let snapshot = source.snapshot()?;
        let epoch = pinned.query_row("SELECT security_epoch FROM meta", [], |r| {
            r.get::<_, i64>(0)
        })?;
        let schema_version = task::version(&pinned)?;
        let expected = inventory::inventory(&pinned)?;
        file_identity.check(&source_path)?;
        let stage = Stage::reserve(&destination)?;
        ops.before_copy(&stage.path)?;
        export_snapshot(
            &stage.path,
            &destination_storage,
            &source_path,
            &self.storage_access(),
            &pinned,
        )?;
        ops.after_copy(&stage.path)?;
        verify_snapshot(&stage.path, &destination_storage, &expected)?;
        file_identity.check(&source_path)?;
        let (tables, rows) = expected.counts();
        stage.publish(&destination, ops)?;
        // The pinned reader remains alive through publication, so a concurrent
        // rollback-journal writer cannot commit a different source halfway through.
        drop(pinned);
        Ok(BackupReport {
            tables,
            rows,
            schema_version,
            sequence: snapshot.sequence,
            security_epoch: epoch,
            scope: concat!(
                "encrypted local memory database only; ",
                "excludes keys, JSONL, source authority and task artifacts"
            ),
        })
    }
}
#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;

fn export_snapshot(
    stage: &Path,
    destination_storage: &StorageAccess,
    source_path: &Path,
    source_storage: &StorageAccess,
    pinned: &rusqlite::Connection,
) -> Result<()> {
    let epoch: i64 = pinned.query_row("SELECT security_epoch FROM meta", [], |r| r.get(0))?;
    let conn = storage::connect_new_reserved(stage, destination_storage)?;
    sql::configure(&conn)?;
    storage::attach_snapshot_source(&conn, source_path, source_storage)?;
    conn.execute_batch("BEGIN;")?;
    let source_identity = conn.query_row(
        "SELECT instance,domain,security_epoch FROM snapshot_source.meta",
        [],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        },
    )?;
    ensure!(
        source_identity
            == sql::identity(pinned).map(|(instance, domain)| (instance, domain, epoch))?,
        "snapshot source changed before export"
    );
    conn.query_row(
        "SELECT sqlcipher_export('main','snapshot_source')",
        [],
        |_| Ok(()),
    )
    .map_err(|_| anyhow::anyhow!("encrypted snapshot export failed"))?;
    let application: i64 = pinned.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    let version = task::version(pinned)?;
    conn.pragma_update(Some("main"), "application_id", application)?;
    conn.pragma_update(Some("main"), "user_version", version)?;
    conn.execute_batch("COMMIT; DETACH DATABASE snapshot_source;")?;
    Ok(())
}
fn verify_snapshot(
    path: &Path,
    storage: &StorageAccess,
    expected: &inventory::Inventory,
) -> Result<()> {
    let verify = storage::connect_existing(path, storage, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    sql::configure_read_only(&verify)?;
    inventory::integrity(&verify)?;
    ensure!(
        &inventory::inventory(&verify)? == expected,
        "snapshot inventory differs from the pinned source"
    );
    Ok(())
}
/// Owner-selected current deletion/access authority. It must remain accessible;
/// an old snapshot alone cannot establish whether newer forgetting occurred.
/// This capability carries storage access, never a reconstructed execution permit.
pub struct RestoreAuthority {
    path: std::path::PathBuf,
    storage: StorageAccess,
}
impl RestoreAuthority {
    pub fn current(path: &Path, storage: StorageAccess) -> Self {
        Self {
            path: path.to_path_buf(),
            storage,
        }
    }
}
impl std::fmt::Debug for RestoreAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RestoreAuthority([host-only])")
    }
}
#[derive(Debug, Serialize)]
pub struct RestoreReport {
    pub schema_version: i64,
    pub snapshot_sequence: i64,
    pub authority_sequence: i64,
    pub security_epoch: i64,
    pub purged_objects: usize,
    pub purged_revisions: usize,
    pub scope: &'static str,
}
/// Restore a NEW encrypted offline historical copy using CURRENT authority.
/// Newer factual/concept revisions and withdrawals are not merged: old heads
/// are locally current within the copy. External authority still governs reads.
/// This does not install a live store or propagate future cross-copy deletion.
pub fn restore_encrypted(
    backup: &Path,
    backup_storage: StorageAccess,
    destination: &Path,
    destination_storage: StorageAccess,
    current: &RestoreAuthority,
) -> Result<RestoreReport> {
    restore_with_files(
        backup,
        backup_storage,
        destination,
        destination_storage,
        current,
        &SystemFiles,
    )
}
fn restore_with_files(
    backup: &Path,
    backup_storage: StorageAccess,
    destination: &Path,
    destination_storage: StorageAccess,
    current: &RestoreAuthority,
    ops: &impl FileOps,
) -> Result<RestoreReport> {
    use rusqlite::{OpenFlags, TransactionBehavior};
    ensure!(
        backup_storage.is_encrypted()
            && destination_storage.is_encrypted()
            && current.storage.is_encrypted(),
        "restore requires encrypted backup, destination and current authority"
    );
    let destination = sql::validate_path(destination)?;
    ensure!(
        !destination.try_exists()? && fs::symlink_metadata(&destination).is_err(),
        "restore destination already exists"
    );
    let current_path = sql::validate_existing_path(&current.path)?;
    let current_file = FileIdentity::at(&current_path)?;
    // Refuse unsupported authority before writable configuration can touch it.
    let probe = Memory::open_read_only_with_storage(&current_path, current.storage.clone())?;
    let authority_version = task::version(&probe.conn)?;
    ensure!(
        matches!(authority_version, 8..=10),
        "restore current authority requires schema v8, v9 or v10"
    );
    if matches!(authority_version, 9 | 10) {
        task::require_live(&probe.conn)?;
    }
    drop(probe);
    let mut authority = Memory::open_with_storage(&current_path, current.storage.clone())?;
    // A reserved writer blocks concurrent forgetting AND scope changes. A read
    // transaction alone would not reserve authority through publication.
    let held = authority
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    inventory::integrity(&held)?;
    ensure!(
        task::version(&held)? == authority_version,
        "restore authority schema changed"
    );
    if matches!(authority_version, 9 | 10) {
        task::require_live(&held)?;
    }
    let state = SuppressionState::read(&held)?;
    current_file.check(&current_path)?;
    let backup_path = sql::validate_existing_path(backup)?;
    let backup_file = FileIdentity::at(&backup_path)?;
    let source = Memory::open_read_only_with_storage(&backup_path, backup_storage.clone())?;
    let pinned = source.conn.unchecked_transaction()?;
    inventory::integrity(&pinned)?;
    ensure!(
        task::version(&pinned)? == authority_version,
        "mixed schema restore is unsupported"
    );
    let before = SuppressionState::read(&pinned)?;
    before.require_older_than(&state)?;
    let original = inventory::inventory(&pinned)?;
    backup_file.check(&backup_path)?;
    let stage = Stage::reserve(&destination)?;
    ops.before_copy(&stage.path)?;
    export_snapshot(
        &stage.path,
        &destination_storage,
        &backup_path,
        &backup_storage,
        &pinned,
    )?;
    ops.after_copy(&stage.path)?;
    verify_snapshot(&stage.path, &destination_storage, &original)?;
    let (counts, transformed) = {
        let mut conn = storage::connect_existing(
            &stage.path,
            &destination_storage,
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )?;
        sql::configure(&conn)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        super::forget::initialize_doomed(&tx)?;
        state.seed(&tx)?;
        let counts = super::forget::purge_seeded(&tx)?;
        tx.execute(
            "UPDATE meta SET security_epoch=?1 WHERE singleton=1",
            [state.epoch],
        )?;
        if matches!(authority_version, 9 | 10) {
            tx.execute(
                "UPDATE task_store_state SET disposition='historical' WHERE singleton=1",
                [],
            )?;
        }
        state.verify_restored(&tx, &held)?;
        tx.commit()?;
        conn.execute_batch("VACUUM;")?;
        (counts, inventory::inventory(&conn)?)
    };
    ops.after_transform(&stage.path)?;
    verify_snapshot(&stage.path, &destination_storage, &transformed)?;
    {
        let verify = storage::connect_existing(
            &stage.path,
            &destination_storage,
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        sql::configure_read_only(&verify)?;
        state.verify_restored(&verify, &held)?;
    }
    current_file.check(&current_path)?;
    backup_file.check(&backup_path)?;
    // Independently reopen the selected path, not only the original descriptor.
    // The held authority write reservation remains alive until publication ends.
    let fresh = Memory::open_read_only_with_storage(&current_path, current.storage.clone())?;
    ensure!(
        SuppressionState::read(&fresh.conn)? == state,
        "current restore authority changed"
    );
    if matches!(authority_version, 9 | 10) {
        task::require_live(&fresh.conn)?;
    }
    stage.publish(&destination, ops)?;
    drop(pinned);
    drop(held);
    Ok(RestoreReport {
        schema_version: authority_version,
        snapshot_sequence: before.sequence,
        authority_sequence: state.sequence,
        security_epoch: state.epoch,
        purged_objects: counts.objects,
        purged_revisions: counts.revisions,
        scope: concat!(
            "fresh offline historical memory database only; current authority required; ",
            "newer facts and withdrawals are not merged; old heads are locally current; ",
            "excludes keys, JSONL, external source authority and task artifacts; ",
            "no live installation or future cross-copy deletion propagation"
        ),
    })
}
#[derive(PartialEq, Eq)]
struct SuppressionState {
    instance: String,
    domain: String,
    sequence: i64,
    epoch: i64,
    digests: std::collections::BTreeSet<String>,
    restricted: std::collections::BTreeMap<String, String>,
}
impl SuppressionState {
    fn read(conn: &rusqlite::Connection) -> Result<Self> {
        let (instance, domain) = sql::identity(conn)?;
        let sequence = sql::sequence(conn)?;
        let epoch = conn.query_row("SELECT security_epoch FROM meta", [], |r| r.get(0))?;
        let digests = conn
            .prepare("SELECT digest FROM suppression")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let restricted = conn
            .prepare(
                "SELECT r.object_id,o.kind FROM restriction r JOIN object o ON o.id=r.object_id",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Self {
            instance,
            domain,
            sequence,
            epoch,
            digests,
            restricted,
        })
    }
    fn require_older_than(&self, current: &Self) -> Result<()> {
        ensure!(
            self.instance == current.instance && self.domain == current.domain,
            "restore authority instance or domain differs"
        );
        ensure!(
            self.sequence <= current.sequence && self.epoch <= current.epoch,
            "restore authority is older than backup"
        );
        ensure!(
            self.digests.is_subset(&current.digests)
                && self
                    .restricted
                    .iter()
                    .all(|(id, kind)| current.restricted.get(id) == Some(kind)),
            "restore authority is missing backup suppression or restrictions"
        );
        Ok(())
    }
    fn seed(&self, conn: &rusqlite::Connection) -> Result<()> {
        use rusqlite::OptionalExtension;
        for digest in &self.digests {
            conn.execute("INSERT OR IGNORE INTO suppression VALUES(?1)", [digest])?;
        }
        for (id, kind) in &self.restricted {
            let old: Option<String> = conn
                .query_row("SELECT kind FROM object WHERE id=?1", [id], |r| r.get(0))
                .optional()?;
            ensure!(
                old.as_ref().is_none_or(|k| k == kind),
                "restricted object kind conflicts with backup"
            );
            conn.execute(
                "INSERT OR IGNORE INTO object VALUES(?1,?2)",
                rusqlite::params![id, kind],
            )?;
            conn.execute("INSERT OR IGNORE INTO doomed VALUES(?1)", [id])?;
        }
        conn.execute("INSERT OR IGNORE INTO doomed SELECT object_id FROM source_item WHERE digest IN (SELECT digest FROM suppression)", [])?;
        Ok(())
    }
    fn verify_restored(
        &self,
        conn: &rusqlite::Connection,
        current: &rusqlite::Connection,
    ) -> Result<()> {
        use rusqlite::OptionalExtension;
        let restored = Self::read(conn)?;
        ensure!(
            restored.instance == self.instance
                && restored.domain == self.domain
                && restored.epoch == self.epoch,
            "restored authority metadata differs"
        );
        ensure!(
            self.digests.is_subset(&restored.digests)
                && self
                    .restricted
                    .iter()
                    .all(|(id, kind)| restored.restricted.get(id) == Some(kind)),
            "restore lost current suppression or restrictions"
        );
        let forbidden: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM revision WHERE object_id IN (SELECT object_id FROM restriction)) OR EXISTS(SELECT 1 FROM source_item WHERE object_id IN (SELECT object_id FROM restriction) OR digest IN (SELECT digest FROM suppression))", [], |r|r.get(0))?;
        ensure!(
            !forbidden,
            "restore retained restricted or suppressed content"
        );
        let objects = conn
            .prepare(
                "SELECT id,kind FROM object WHERE id NOT IN (SELECT object_id FROM restriction)",
            )?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, kind) in objects {
            let current_kind: Option<String> = current
                .query_row("SELECT kind FROM object WHERE id=?1", [&id], |r| r.get(0))
                .optional()?;
            ensure!(
                current_kind.as_ref() == Some(&kind),
                "surviving object is unknown to current authority"
            );
            let (head, _, _) = sql::head(current, &id, i64::MAX)?
                .context("current authority head is unavailable")?;
            let (old, _, _) =
                sql::head(conn, &id, i64::MAX)?.context("restored object head is unavailable")?;
            ensure!(
                sql::required_scopes(current, &head.revision)?
                    .is_subset(&sql::required_scopes(conn, &old.revision)?),
                "restore would weaken current head scopes"
            );
        }
        Ok(())
    }
}
#[cfg(test)]
#[path = "restore_tests.rs"]
mod restore_tests;
