use super::*;
use std::{collections::BTreeSet, path::PathBuf};
struct Fixture {
    _scratch: super::super::storage_tests::KeyScratch,
    db: Memory,
    backup: PathBuf,
    backup_key: StorageAccess,
    destination: PathBuf,
    output_key: StorageAccess,
}
impl Fixture {
    fn new() -> Result<Self> {
        let scratch = super::super::storage_tests::KeyScratch::new()?;
        let data = scratch.keys.parent().unwrap().join("store");
        let key = |name: &str| {
            StorageAccess::generate_key_file(&scratch.keys.join(name), &scratch.policy)
        };
        let db = Memory::create_supported_situated_with_storage(
            &data.join("current.memory-prototype.sqlite"),
            "restore-fixture",
            key("current.key")?,
        )?;
        Ok(Self {
            db,
            backup: data.join("backup.memory-prototype.sqlite"),
            destination: data.join("restored.memory-prototype.sqlite"),
            backup_key: key("backup.key")?,
            output_key: key("output.key")?,
            _scratch: scratch,
        })
    }
    fn authority(&self) -> RestoreAuthority {
        RestoreAuthority::current(
            Path::new(self.db.conn.path().unwrap()),
            self.db.storage_access(),
        )
    }
    fn backup(&self) -> Result<()> {
        self.db
            .backup_encrypted(&self.backup, self.backup_key.clone())?;
        Ok(())
    }
    fn restore(&self) -> Result<RestoreReport> {
        restore_encrypted(
            &self.backup,
            self.backup_key.clone(),
            &self.destination,
            self.output_key.clone(),
            &self.authority(),
        )
    }
    fn open(&self) -> Result<Memory> {
        Memory::open_with_storage(&self.destination, self.output_key.clone())
    }
}
fn capture(id: &str, scopes: &[&str]) -> Capture {
    Capture {
        source: SourceKey {
            connector: "fixture".into(),
            account: "synthetic".into(),
            id: id.into(),
        },
        text: format!("Synthetic zircon observation {id}"),
        occurred_at: None,
        scopes: scopes.iter().map(|s| (*s).into()).collect(),
        transcript: None,
    }
}
fn access() -> Access {
    Access {
        domain: "restore-fixture".into(),
        scopes: BTreeSet::from(["chat".into(), "restricted".into()]),
    }
}
#[test]
fn restore_purges_forgotten_history_and_keeps_postbackup_restricted_stubs() -> Result<()> {
    let mut f = Fixture::new()?;
    let original = capture("original", &["chat"]);
    let old = f.db.capture(original.clone())?;
    let survivor = f.db.capture(capture("independent", &["chat"]))?;
    f.backup()?;
    let later = capture("postbackup", &["chat"]);
    let later_ref = f.db.capture(later.clone())?;
    f.db.forget(&original.source)?;
    f.db.forget(&later.source)?;
    let backup_bytes = fs::read(&f.backup)?;
    let current_bytes = fs::read(f.db.conn.path().unwrap())?;
    let prior = Memory::open_read_only_with_storage(&f.backup, f.backup_key.clone())?;
    assert!(
        prior.read(&old, &access())?.is_some(),
        "negative control: old backup must retain forgotten payload"
    );
    let report = f.restore()?;
    assert!(report.authority_sequence > report.snapshot_sequence);
    assert_eq!(report.security_epoch, 2);
    let mut restored = f.open()?;
    assert!(restored.read(&old, &access())?.is_none());
    assert!(restored.read(&survivor, &access())?.is_some());
    assert!(restored.capture(original).is_err());
    assert!(restored.capture(later).is_err());
    assert_eq!(
        restored.conn.query_row(
            "SELECT count(*) FROM restriction WHERE object_id=?1",
            [&later_ref.object],
            |r| r.get::<_, i64>(0)
        )?,
        1
    );
    assert_eq!(
        restored.conn.query_row(
            "SELECT count(*) FROM revision WHERE object_id IN (SELECT object_id FROM restriction)",
            [],
            |r| r.get::<_, i64>(0)
        )?,
        0
    );
    inventory::integrity(&restored.conn)?;
    assert_eq!(fs::read(&f.backup)?, backup_bytes);
    assert_eq!(fs::read(f.db.conn.path().unwrap())?, current_bytes);
    Ok(())
}
#[test]
fn restore_refuses_scope_tightening_without_an_epoch_change() -> Result<()> {
    let mut f = Fixture::new()?;
    let original = capture("scope", &["chat"]);
    let old = f.db.capture(original)?;
    f.backup()?;
    f.db.capture(capture("scope", &["chat", "restricted"]))?;
    assert_eq!(
        f.db.conn
            .query_row("SELECT security_epoch FROM meta", [], |r| r
                .get::<_, i64>(0))?,
        0
    );
    let broad = Access {
        domain: "restore-fixture".into(),
        scopes: ["chat".into()].into(),
    };
    assert!(f.db.read(&old, &broad)?.is_none());
    let err = f.restore().unwrap_err();
    assert!(format!("{err:#}").contains("scope"));
    assert!(!f.destination.exists());
    Ok(())
}
#[test]
fn restore_retains_historical_payloads_with_equal_or_stricter_scopes() -> Result<()> {
    for backup_scopes in [vec!["chat"], vec!["chat", "restricted"]] {
        let mut f = Fixture::new()?;
        let old = f.db.capture(capture("historical", &backup_scopes))?;
        f.backup()?;
        let mut new = capture("historical", &["chat"]);
        new.text = "Synthetic updated fact".into();
        f.db.capture(new)?;
        f.restore()?;
        let restored = f.open()?;
        assert!(
            matches!(restored.read(&old, &access())?, Some(Record::Evidence(e)) if e.text == "Synthetic zircon observation historical")
        );
        assert_eq!(restored.snapshot()?.sequence, 1);
    }
    Ok(())
}
#[test]
fn restore_checks_entity_scopes_and_missing_heads() -> Result<()> {
    for missing in [false, true] {
        let mut f = Fixture::new()?;
        let entity = f.db.entity("Synthetic entity", BTreeSet::new())?;
        f.backup()?;
        if missing {
            f.db.conn
                .execute("DELETE FROM revision WHERE object_id=?1", [&entity.object])?;
        } else {
            f.db.conn.execute(
                "INSERT INTO scope VALUES(?1,'new-required-scope')",
                [&entity.revision],
            )?;
        }
        assert!(f.restore().is_err());
        assert!(!f.destination.exists());
    }
    Ok(())
}
#[test]
fn restore_refuses_stale_foreign_or_damaged_current_authority() -> Result<()> {
    for mutation in [
        "UPDATE meta SET instance='foreign'",
        "UPDATE meta SET domain='foreign'",
        "UPDATE meta SET security_epoch=-1",
        "DELETE FROM suppression",
        "DELETE FROM restriction",
        "UPDATE object SET kind='conflicting' WHERE id IN (SELECT object_id FROM restriction)",
        "PRAGMA user_version=7",
    ] {
        let mut f = Fixture::new()?;
        let c = capture("forgotten", &["chat"]);
        f.db.capture(c.clone())?;
        f.db.forget(&c.source)?;
        f.backup()?;
        f.db.conn.execute_batch(mutation)?;
        assert!(f.restore().is_err(), "accepted {mutation}");
        assert!(!f.destination.exists());
    }
    Ok(())
}
#[test]
fn restore_refuses_missing_keys_plaintext_and_existing_destination() -> Result<()> {
    let f = Fixture::new()?;
    f.backup()?;
    for (backup_key, output_key, authority) in [
        (f.output_key.clone(), f.output_key.clone(), f.authority()),
        (
            f.backup_key.clone(),
            StorageAccess::plaintext_fixture(),
            f.authority(),
        ),
        (
            f.backup_key.clone(),
            f.output_key.clone(),
            RestoreAuthority::current(Path::new(f.db.conn.path().unwrap()), f.backup_key.clone()),
        ),
        (
            f.backup_key.clone(),
            f.output_key.clone(),
            RestoreAuthority::current(&f.destination, f.db.storage_access()),
        ),
    ] {
        assert!(restore_encrypted(
            &f.backup,
            backup_key,
            &f.destination,
            output_key,
            &authority
        )
        .is_err());
        assert!(!f.destination.exists());
    }
    fs::write(&f.destination, b"owned by someone else")?;
    assert!(f.restore().is_err());
    assert_eq!(fs::read(&f.destination)?, b"owned by someone else");
    Ok(())
}
#[test]
fn restore_seeded_purge_fails_atomically_and_removes_owned_stage() -> Result<()> {
    let mut f = Fixture::new()?;
    let c = capture("purge-fault", &["chat"]);
    f.db.capture(c.clone())?;
    f.db.conn.execute_batch("CREATE TRIGGER authored_purge_fault BEFORE DELETE ON revision BEGIN SELECT RAISE(ABORT,'authored purge failure'); END;")?;
    f.backup()?;
    f.db.conn
        .execute_batch("DROP TRIGGER authored_purge_fault;")?;
    f.db.forget(&c.source)?;
    let before = fs::read(&f.backup)?;
    assert!(format!("{:#}", f.restore().unwrap_err()).contains("authored purge failure"));
    assert!(!f.destination.exists());
    assert_eq!(fs::read(&f.backup)?, before);
    assert!(
        fs::read_dir(f.destination.parent().unwrap())?.all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".pending"))
    );
    Ok(())
}
#[derive(Clone, Copy, PartialEq)]
enum RestoreFault {
    Verify,
    Sync,
    Publish,
    Directory,
    ReplaceCurrent,
    CompetingOutput,
}
struct RestoreFiles {
    fault: RestoreFault,
    current: PathBuf,
    key: StorageAccess,
}
impl FileOps for RestoreFiles {
    fn after_transform(&self, path: &Path) -> Result<()> {
        if self.fault == RestoreFault::Verify {
            let conn =
                storage::connect_existing(path, &self.key, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
            conn.execute("DELETE FROM memory_fts_config WHERE k='secure-delete'", [])?;
        } else if self.fault == RestoreFault::ReplaceCurrent {
            fs::rename(&self.current, self.current.with_extension("held"))?;
            fs::write(&self.current, b"authored replacement")?;
        }
        Ok(())
    }
    fn sync_file(&self, file: &fs::File) -> Result<()> {
        ensure!(self.fault != RestoreFault::Sync, "authored sync refusal");
        Ok(file.sync_all()?)
    }
    fn publish(&self, stage: &Path, destination: &Path) -> Result<()> {
        ensure!(
            self.fault != RestoreFault::Publish,
            "authored publication refusal"
        );
        if self.fault == RestoreFault::CompetingOutput {
            fs::write(destination, b"competing publisher")?;
        }
        Ok(fs::hard_link(stage, destination)?)
    }
    fn sync_directory(&self, file: &fs::File) -> Result<()> {
        ensure!(
            self.fault != RestoreFault::Directory,
            "authored directory refusal"
        );
        Ok(file.sync_all()?)
    }
}
#[test]
fn restore_verifies_transformed_state_and_handles_publication_failures() -> Result<()> {
    for fault in [
        RestoreFault::Verify,
        RestoreFault::Sync,
        RestoreFault::Publish,
        RestoreFault::Directory,
        RestoreFault::ReplaceCurrent,
        RestoreFault::CompetingOutput,
    ] {
        let mut f = Fixture::new()?;
        f.db.capture(capture("survivor", &["chat"]))?;
        f.backup()?;
        let before = fs::read(&f.backup)?;
        let current = PathBuf::from(f.db.conn.path().unwrap());
        let files = RestoreFiles {
            fault,
            current: current.clone(),
            key: f.output_key.clone(),
        };
        let err = restore_with_files(
            &f.backup,
            f.backup_key.clone(),
            &f.destination,
            f.output_key.clone(),
            &f.authority(),
            &files,
        )
        .unwrap_err();
        match fault {
            RestoreFault::Directory => {
                assert!(format!("{err:#}").contains("durability is uncertain"));
                inventory::integrity(&f.open()?.conn)?;
            }
            RestoreFault::CompetingOutput => {
                assert_eq!(fs::read(&f.destination)?, b"competing publisher")
            }
            _ => assert!(!f.destination.exists()),
        }
        assert_eq!(fs::read(&f.backup)?, before);
        if fault == RestoreFault::ReplaceCurrent {
            assert_eq!(fs::read(current)?, b"authored replacement");
        }
        assert!(
            fs::read_dir(f.destination.parent().unwrap())?.all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".pending"))
        );
    }
    Ok(())
}
struct TighteningWriter {
    current: PathBuf,
    key: StorageAccess,
    revision: String,
    attempted: std::cell::Cell<bool>,
}
impl FileOps for TighteningWriter {
    fn after_copy(&self, _: &Path) -> Result<()> {
        let writer =
            storage::connect_existing(&self.current, &self.key, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        writer.busy_timeout(std::time::Duration::ZERO)?;
        let err = writer
            .execute("INSERT INTO scope VALUES(?1,'new-scope')", [&self.revision])
            .unwrap_err();
        assert_eq!(
            err.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseBusy)
        );
        self.attempted.set(true);
        Ok(())
    }
}
#[test]
fn restore_holds_current_write_reservation_through_export_and_refuses_busy_authority() -> Result<()>
{
    let mut f = Fixture::new()?;
    let r = f.db.capture(capture("locked", &["chat"]))?;
    f.backup()?;
    f.db.conn.execute_batch("BEGIN IMMEDIATE;")?;
    let start = std::time::Instant::now();
    assert!(f.restore().is_err());
    assert!(start.elapsed() < std::time::Duration::from_secs(8));
    assert!(!f.destination.exists());
    f.db.conn.execute_batch("ROLLBACK;")?;
    let files = TighteningWriter {
        current: PathBuf::from(f.db.conn.path().unwrap()),
        key: f.db.storage_access(),
        revision: r.revision.clone(),
        attempted: std::cell::Cell::new(false),
    };
    restore_with_files(
        &f.backup,
        f.backup_key.clone(),
        &f.destination,
        f.output_key.clone(),
        &f.authority(),
        &files,
    )?;
    assert!(files.attempted.get());
    f.db.conn
        .execute("INSERT INTO scope VALUES(?1,'new-scope')", [&r.revision])?;
    Ok(())
}
#[test]
fn restore_purges_relation_history_and_dependency_only_descendants() -> Result<()> {
    let mut f = Fixture::new()?;
    let source = capture("root", &["chat"]);
    let root = f.db.capture(source.clone())?;
    let subject = f.db.entity("Synthetic subject", BTreeSet::new())?;
    let input = RelationInput {
        predicate: "observed".into(),
        subject,
        object: Target::Literal("synthetic".into()),
        support: Span {
            evidence: root.clone(),
            start: 0,
            end: 9,
        },
        consulted: vec![],
        text: "Synthetic linked zircon claim".into(),
        polarity: 1,
        valid: Interval {
            from: Bound::Unknown,
            to: Bound::Unknown,
        },
    };
    let a = f.db.relate(input.clone())?;
    let mut changed = input.clone();
    changed.text = "Synthetic revised zircon claim".into();
    let revised = f.db.revise_relation(&a, changed)?;
    let other = f.db.capture(capture("independent", &["chat"]))?;
    let mut descendant = input;
    descendant.support.evidence = other.clone();
    descendant.consulted = vec![revised.clone()];
    let child = f.db.relate(descendant)?;
    f.backup()?;
    f.db.forget(&source.source)?;
    f.restore()?;
    let restored = f.open()?;
    for r in [root, a, revised, child] {
        assert!(restored.read(&r, &access())?.is_none());
    }
    assert!(restored.read(&other, &access())?.is_some());
    assert_eq!(
        restored.conn.query_row(
            "SELECT count(*) FROM memory_fts WHERE text LIKE '%claim%'",
            [],
            |r| r.get::<_, i64>(0)
        )?,
        0
    );
    Ok(())
}
#[test]
fn restore_independently_seeds_current_digests_and_restricted_ids() -> Result<()> {
    for omitted in ["suppression", "restriction"] {
        let mut f = Fixture::new()?;
        let c = capture("seed", &["chat"]);
        let r = f.db.capture(c.clone())?;
        f.backup()?;
        f.db.forget(&c.source)?;
        // Synthetic authority isolates each seed source: both are necessary.
        f.db.conn.execute(&format!("DELETE FROM {omitted}"), [])?;
        f.restore()?;
        let mut restored = f.open()?;
        assert!(restored.read(&r, &access())?.is_none());
        assert!(restored.capture(c).is_err());
    }
    Ok(())
}
#[test]
fn restore_rejects_future_sequence_and_conflicting_new_restriction_kind() -> Result<()> {
    for change_kind in [false, true] {
        let mut f = Fixture::new()?;
        let c = capture("sequence", &["chat"]);
        f.db.capture(c.clone())?;
        if !change_kind {
            f.db.conn.execute(
                "INSERT INTO change_set(seq,committed_at) VALUES(42,'synthetic')",
                [],
            )?;
        }
        f.backup()?;
        if change_kind {
            f.db.forget(&c.source)?;
            f.db.conn
                .execute("UPDATE object SET kind='conflicting'", [])?;
        } else {
            f.db.conn
                .execute("DELETE FROM change_set WHERE seq=42", [])?;
        }
        assert!(f.restore().is_err());
        assert!(!f.destination.exists());
    }
    Ok(())
}
