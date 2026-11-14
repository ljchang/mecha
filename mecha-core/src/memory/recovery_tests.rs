use super::*;
use std::{fs, path::PathBuf};
fn fixture() -> Result<(
    super::super::storage_tests::KeyScratch,
    Memory,
    PathBuf,
    StorageAccess,
)> {
    let scratch = super::super::storage_tests::KeyScratch::new()?;
    let path = scratch
        .keys
        .parent()
        .unwrap()
        .join("store/source.memory-prototype.sqlite");
    let key = StorageAccess::generate_key_file(&scratch.keys.join("source.key"), &scratch.policy)?;
    let mut db = Memory::create_supported_situated_with_storage(&path, "snapshot-fixture", key)?;
    db.capture(Capture {
        source: SourceKey {
            connector: "fixture".into(),
            account: "synthetic".into(),
            id: "one".into(),
        },
        text: "Synthetic amber snapshot evidence".into(),
        occurred_at: None,
        scopes: ["private".into()].into(),
        transcript: None,
    })?;
    let forgotten = SourceKey {
        connector: "fixture".into(),
        account: "synthetic".into(),
        id: "forgotten".into(),
    };
    db.capture(Capture {
        source: forgotten.clone(),
        text: "Synthetic forgotten content".into(),
        occurred_at: None,
        scopes: ["private".into()].into(),
        transcript: None,
    })?;
    db.forget(&forgotten)?;
    db.conn.execute_batch("INSERT INTO change_set(seq, committed_at) VALUES(100,'synthetic'); DELETE FROM change_set WHERE seq=100; INSERT INTO host_capture_receipt(identity_hash,state,lineage_version) VALUES('forgotten-capture','forgotten',2); INSERT INTO interpretation_receipt(request_id,state) VALUES('forgotten-interpretation','forgotten'); CREATE TABLE typed_probe(a,b,c,d,e); INSERT INTO typed_probe VALUES(NULL,'',X'',7,7.0);")?;
    let target = scratch
        .keys
        .parent()
        .unwrap()
        .join("store/backup.memory-prototype.sqlite");
    let key = StorageAccess::generate_key_file(&scratch.keys.join("target.key"), &scratch.policy)?;
    Ok((scratch, db, target, key))
}
#[test]
fn snapshot_reopens_with_new_key_and_preserves_sequence_and_source() -> Result<()> {
    let (_scratch, db, destination, key) = fixture()?;
    let source = Path::new(db.conn.path().unwrap());
    let before = fs::read(source)?;
    let snapshot = db.snapshot()?;
    let report = db.backup_encrypted(&destination, key.clone())?;
    assert_eq!(report.schema_version, 8);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(&destination)?.permissions().mode() & 0o777,
        0o600
    );
    let ciphertext = fs::read(&destination)?;
    assert!(!ciphertext.starts_with(b"SQLite format 3\0"));
    assert!(!ciphertext
        .windows(b"Synthetic amber".len())
        .any(|w| w == b"Synthetic amber"));
    let copy = Memory::open_read_only_with_storage(&destination, key)?;
    assert_eq!(copy.snapshot()?.instance, snapshot.instance);
    assert_eq!(copy.snapshot()?.sequence, snapshot.sequence);
    assert_eq!(
        copy.conn.query_row(
            "SELECT seq FROM sqlite_sequence WHERE name='change_set'",
            [],
            |r| r.get::<_, i64>(0)
        )?,
        100
    );
    assert_eq!(
        copy.conn.query_row(
            "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'amber'",
            [],
            |r| r.get::<_, i64>(0)
        )?,
        1
    );
    assert_eq!(fs::read(source)?, before);
    assert!(Memory::open_read_only(&destination).is_err());
    assert!(Memory::open_read_only_with_storage(&destination, db.storage_access()).is_err());
    Ok(())
}

fn stages(destination: &Path) -> Vec<PathBuf> {
    fs::read_dir(destination.parent().unwrap())
        .unwrap()
        .map(|x| x.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".mecha-snapshot-")
        })
        .collect()
}
#[test]
fn refuses_plaintext_old_schema_and_existing_output_without_mutation() -> Result<()> {
    let (scratch, db, destination, key) = fixture()?;
    assert!(db
        .backup_encrypted(&destination, StorageAccess::plaintext_fixture())
        .is_err());
    assert!(!destination.exists());
    let plain = Memory::create_supported_situated(&destination, "snapshot-fixture")?;
    let output = destination.with_file_name("other.memory-prototype.sqlite");
    assert!(plain.backup_encrypted(&output, key.clone()).is_err());
    let bytes = fs::read(&destination)?;
    assert!(db.backup_encrypted(&destination, key.clone()).is_err());
    assert_eq!(fs::read(&destination)?, bytes);
    db.conn.execute_batch("PRAGMA user_version=7;")?;
    assert!(db.backup_encrypted(&output, key).is_err());
    assert!(!output.exists());
    assert!(stages(&destination).is_empty());
    drop(scratch);
    Ok(())
}
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    Copy,
    Corrupt,
    OmitTable,
    OmitShadow,
    OmitShadowTable,
    Sync,
    Publish,
    Directory,
}
struct FailingFiles {
    fault: Fault,
    key: StorageAccess,
}
impl FileOps for FailingFiles {
    fn before_copy(&self, _: &Path) -> Result<()> {
        anyhow::ensure!(self.fault != Fault::Copy, "authored copy fault");
        Ok(())
    }
    fn after_copy(&self, path: &Path) -> Result<()> {
        if self.fault == Fault::Corrupt {
            use std::io::{Seek, SeekFrom, Write};
            let mut file = fs::OpenOptions::new().write(true).open(path)?;
            file.seek(SeekFrom::Start(200))?;
            file.write_all(&[0; 32])?;
        } else if matches!(
            self.fault,
            Fault::OmitTable | Fault::OmitShadow | Fault::OmitShadowTable
        ) {
            let conn = super::super::storage::connect_existing(
                path,
                &self.key,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
            )?;
            conn.execute_batch(if self.fault == Fault::OmitTable {
                "DROP TABLE typed_probe;"
            } else if self.fault == Fault::OmitShadowTable {
                "DROP TABLE memory_fts_docsize;"
            } else {
                "DELETE FROM memory_fts_config WHERE k='secure-delete';"
            })?;
        }
        Ok(())
    }
    fn sync_file(&self, file: &fs::File) -> Result<()> {
        anyhow::ensure!(self.fault != Fault::Sync, "authored file sync fault");
        Ok(file.sync_all()?)
    }
    fn publish(&self, stage: &Path, destination: &Path) -> Result<()> {
        anyhow::ensure!(self.fault != Fault::Publish, "authored publish fault");
        Ok(fs::hard_link(stage, destination)?)
    }
    fn sync_directory(&self, file: &fs::File) -> Result<()> {
        anyhow::ensure!(
            self.fault != Fault::Directory,
            "authored directory sync fault"
        );
        Ok(file.sync_all()?)
    }
}
#[test]
fn copy_verification_and_file_failures_do_not_publish_or_damage_source() -> Result<()> {
    let (_scratch, db, destination, key) = fixture()?;
    let before = fs::read(db.conn.path().unwrap())?;
    for fault in [
        Fault::Copy,
        Fault::Corrupt,
        Fault::OmitTable,
        Fault::OmitShadow,
        Fault::OmitShadowTable,
        Fault::Sync,
        Fault::Publish,
    ] {
        assert!(db
            .backup_with_files(
                &destination,
                key.clone(),
                &FailingFiles {
                    fault,
                    key: key.clone()
                }
            )
            .is_err());
        assert!(!destination.exists());
        assert!(stages(&destination).is_empty());
        assert_eq!(fs::read(db.conn.path().unwrap())?, before);
    }
    Ok(())
}
#[test]
fn post_publication_sync_failure_reports_uncertainty_and_preserves_valid_output() -> Result<()> {
    let (_scratch, db, destination, key) = fixture()?;
    let err = db
        .backup_with_files(
            &destination,
            key.clone(),
            &FailingFiles {
                fault: Fault::Directory,
                key: key.clone(),
            },
        )
        .unwrap_err();
    assert!(format!("{err:#}").contains("published; directory durability is uncertain"));
    assert!(Memory::open_read_only_with_storage(&destination, key).is_ok());
    assert!(stages(&destination).is_empty());
    Ok(())
}
#[test]
fn concurrent_publishers_never_replace_the_winner() -> Result<()> {
    use std::sync::{Arc, Barrier};
    struct Together(Arc<Barrier>);
    impl FileOps for Together {
        fn publish(&self, stage: &Path, destination: &Path) -> Result<()> {
            self.0.wait();
            Ok(fs::hard_link(stage, destination)?)
        }
    }
    let (_scratch, db, destination, key) = fixture()?;
    let path = PathBuf::from(db.conn.path().unwrap());
    let storage = db.storage_access();
    let barrier = Arc::new(Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let (path, storage, destination, key, barrier) = (
                path.clone(),
                storage.clone(),
                destination.clone(),
                key.clone(),
                barrier.clone(),
            );
            std::thread::spawn(move || {
                Memory::open_read_only_with_storage(&path, storage)?.backup_with_files(
                    &destination,
                    key,
                    &Together(barrier),
                )
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        Memory::open_read_only_with_storage(&destination, key)?
            .snapshot()?
            .instance,
        db.snapshot()?.instance
    );
    assert!(stages(&destination).is_empty());
    Ok(())
}
#[test]
fn readonly_source_pin_blocks_mid_export_commits() -> Result<()> {
    struct Writer {
        path: PathBuf,
        storage: StorageAccess,
    }
    impl FileOps for Writer {
        fn after_copy(&self, _: &Path) -> Result<()> {
            let conn = super::super::storage::connect_existing(
                &self.path,
                &self.storage,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
            )?;
            conn.busy_timeout(std::time::Duration::from_millis(20))?;
            conn.execute_batch(
                "BEGIN IMMEDIATE; INSERT INTO change_set(committed_at) VALUES('concurrent');",
            )?;
            assert!(conn.execute_batch("COMMIT").is_err());
            conn.execute_batch("ROLLBACK")?;
            Ok(())
        }
    }
    let (_scratch, db, destination, key) = fixture()?;
    let before = db.snapshot()?.sequence;
    db.backup_with_files(
        &destination,
        key.clone(),
        &Writer {
            path: db.conn.path().unwrap().into(),
            storage: db.storage_access(),
        },
    )?;
    assert_eq!(
        Memory::open_read_only_with_storage(&destination, key)?
            .snapshot()?
            .sequence,
        before
    );
    assert_eq!(db.snapshot()?.sequence, before);
    Ok(())
}
#[test]
fn snapshot_uri_quotes_reserved_path_bytes() -> Result<()> {
    let (_scratch, db, destination, key) = fixture()?;
    let original = PathBuf::from(db.conn.path().unwrap());
    let storage = db.storage_access();
    drop(db);
    let special = original.with_file_name("source ?#% \" ü.memory-prototype.sqlite");
    fs::rename(&original, &special)?;
    let db = Memory::open_read_only_with_storage(&special, storage)?;
    db.backup_encrypted(&destination, key.clone())?;
    assert_eq!(
        Memory::open_read_only_with_storage(&destination, key)?
            .snapshot()?
            .instance,
        db.snapshot()?.instance
    );
    Ok(())
}
#[test]
fn inventory_distinguishes_types_and_ordinary_shadow_omissions() -> Result<()> {
    let (_scratch, db, _destination, _key) = fixture()?;
    let before = inventory::inventory(&db.conn)?;
    db.conn
        .execute_batch("UPDATE typed_probe SET a='', b=NULL;")?;
    assert_ne!(inventory::inventory(&db.conn)?, before);
    db.conn
        .execute_batch("UPDATE typed_probe SET a=NULL, b='';")?;
    assert_eq!(inventory::inventory(&db.conn)?, before);
    db.conn.execute_batch("UPDATE typed_probe SET d=7.0,e=7;")?;
    assert_ne!(inventory::inventory(&db.conn)?, before);
    Ok(())
}

#[test]
fn staging_cleanup_does_not_remove_a_replaced_path() -> Result<()> {
    use std::sync::Mutex;
    struct Replace(Mutex<Option<PathBuf>>);
    impl FileOps for Replace {
        fn after_copy(&self, path: &Path) -> Result<()> {
            let moved = path.with_extension("moved");
            fs::rename(path, &moved)?;
            fs::write(path, b"another actor's file")?;
            *self.0.lock().unwrap() = Some(moved);
            Ok(())
        }
    }
    let (_scratch, db, destination, key) = fixture()?;
    let ops = Replace(Mutex::new(None));
    assert!(db
        .backup_with_files(&destination, key.clone(), &ops)
        .is_err());
    assert!(!destination.exists());
    let moved = ops.0.lock().unwrap().take().unwrap();
    let replacement = moved.with_extension("pending");
    assert_eq!(fs::read(&replacement)?, b"another actor's file");
    assert!(super::super::storage::connect_existing(
        &moved,
        &key,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
    )
    .is_ok());
    fs::remove_file(replacement)?;
    fs::remove_file(moved)?;
    Ok(())
}

#[test]
fn changed_source_path_is_refused_before_publication() -> Result<()> {
    struct ReplaceSource(PathBuf);
    impl FileOps for ReplaceSource {
        fn after_copy(&self, _: &Path) -> Result<()> {
            fs::rename(&self.0, self.0.with_extension("moved"))?;
            fs::write(&self.0, b"replacement source")?;
            Ok(())
        }
    }
    let (_scratch, db, destination, key) = fixture()?;
    let source = PathBuf::from(db.conn.path().unwrap());
    assert!(db
        .backup_with_files(&destination, key, &ReplaceSource(source.clone()))
        .is_err());
    assert!(!destination.exists());
    assert!(stages(&destination).is_empty());
    assert_eq!(fs::read(source)?, b"replacement source");
    Ok(())
}
