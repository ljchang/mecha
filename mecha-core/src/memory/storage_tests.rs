use super::*;
use std::{fs, path::PathBuf};

#[test]
fn linked_sqlite_reports_a_cipher_runtime() {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    let version = connection.query_row("PRAGMA cipher_version", [], |row| row.get::<_, String>(0));
    assert!(
        version.as_ref().is_ok_and(|value| !value.trim().is_empty()),
        "memory storage requires an active SQLCipher runtime, got {version:?}"
    );
}

#[cfg(unix)]
pub(super) mod unix {
    use super::*;
    use anyhow::Result;
    use std::os::unix::fs::{symlink, DirBuilderExt, PermissionsExt};

    pub(in super::super) struct Fixture {
        root: PathBuf,
        db: PathBuf,
        pub(in super::super) keys: PathBuf,
        pub(in super::super) policy: KeyFilePolicy,
    }

    impl Fixture {
        pub(in super::super) fn new() -> Result<Self> {
            let root = std::env::temp_dir()
                .canonicalize()?
                .join(format!("memory-storage-{}", uuid::Uuid::new_v4()));
            fs::DirBuilder::new().mode(0o700).create(&root)?;
            let store = root.join("store");
            let keys = root.join("owner");
            fs::DirBuilder::new().mode(0o700).create(&store)?;
            fs::DirBuilder::new().mode(0o700).create(&keys)?;
            let policy = KeyFilePolicy::new(std::slice::from_ref(&store))?;
            Ok(Self {
                root,
                db: store.join("test.memory-prototype.sqlite"),
                keys,
                policy,
            })
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    const CANARY: &str = "AUTHORED_STORAGE_EVIDENCE_75A31";

    fn capture() -> Capture {
        Capture {
            source: SourceKey {
                connector: "fixture".into(),
                account: "synthetic".into(),
                id: "storage-evidence".into(),
            },
            text: CANARY.into(),
            occurred_at: None,
            scopes: ["fixture".into()].into(),
            transcript: None,
        }
    }

    #[test]
    fn encrypted_v8_reads_and_forgets_after_fresh_key_load() -> Result<()> {
        let fixture = Fixture::new()?;
        let key = fixture.keys.join("memory.key");
        let storage = StorageAccess::generate_key_file(&key, &fixture.policy)?;
        let mut writer =
            Memory::create_supported_situated_with_storage(&fixture.db, "synthetic", storage)?;
        let original = capture();
        let reference = writer.capture(original.clone())?;
        let snapshot = writer.snapshot()?;
        assert_eq!(
            writer
                .conn
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
            8
        );
        for (setting, expected) in [("secure_delete", 1), ("synchronous", 2), ("temp_store", 2)] {
            assert_eq!(
                writer
                    .conn
                    .pragma_query_value(None, setting, |r| r.get::<_, i64>(0))?,
                expected
            );
        }
        assert_eq!(
            writer
                .conn
                .pragma_query_value(None, "journal_mode", |r| r.get::<_, String>(0))?,
            "delete"
        );
        drop(writer);

        let bytes = fs::read(&fixture.db)?;
        assert!(!bytes.starts_with(b"SQLite format 3\0"));
        assert!(!bytes.windows(CANARY.len()).any(|w| w == CANARY.as_bytes()));
        let storage = StorageAccess::load_key_file(&key, &fixture.policy)?;
        let reader = Memory::open_read_only_with_storage(&fixture.db, storage.clone())?;
        let access = Access {
            domain: "synthetic".into(),
            scopes: original.scopes.clone(),
        };
        assert!(
            matches!(reader.read(&reference, &access)?, Some(Record::Evidence(c)) if c.text == CANARY)
        );
        assert_eq!(reader.snapshot()?.instance, snapshot.instance);
        assert_eq!(
            reader
                .conn
                .pragma_query_value(None, "query_only", |r| r.get::<_, i64>(0))?,
            1
        );
        drop(reader);
        assert_eq!(fs::read(&fixture.db)?, bytes);
        let mut writer = Memory::open_with_storage(&fixture.db, storage)?;
        let report = writer.forget(&original.source)?;
        assert_eq!(report.revisions, 1);
        assert!(writer.read(&reference, &access)?.is_none());
        assert!(writer.capture(original).is_err());
        drop(writer);
        let storage = StorageAccess::load_key_file(&key, &fixture.policy)?;
        assert!(Memory::open_read_only_with_storage(&fixture.db, storage)?
            .read(&reference, &access)?
            .is_none());
        Ok(())
    }

    #[test]
    fn rollback_journal_keeps_prior_page_bytes_encrypted() -> Result<()> {
        for encrypted in [false, true] {
            let fixture = Fixture::new()?;
            let storage = if encrypted {
                StorageAccess::generate_key_file(
                    &fixture.keys.join("journal.key"),
                    &fixture.policy,
                )?
            } else {
                StorageAccess::plaintext_fixture()
            };
            let mut writer =
                Memory::create_supported_situated_with_storage(&fixture.db, "synthetic", storage)?;
            let original = writer.capture(capture())?;
            let journal = fixture
                .db
                .with_file_name("test.memory-prototype.sqlite-journal");
            let tx = writer.conn.unchecked_transaction()?;
            tx.execute(
                "UPDATE revision SET payload=payload || ' ' WHERE id=?1",
                [&original.revision],
            )?;
            let journal_bytes = fs::read(&journal)?;
            let has_canary = journal_bytes
                .windows(CANARY.len())
                .any(|w| w == CANARY.as_bytes());
            assert_eq!(
                has_canary, !encrypted,
                "rollback journal canary for encrypted={encrypted}"
            );
            tx.rollback()?;
            assert!(!journal.exists());
        }
        Ok(())
    }

    #[test]
    fn wrong_and_missing_keys_refuse_without_writing() -> Result<()> {
        let fixture = Fixture::new()?;
        let key = fixture.keys.join("right.key");
        let right = StorageAccess::generate_key_file(&key, &fixture.policy)?;
        let wrong =
            StorageAccess::generate_key_file(&fixture.keys.join("wrong.key"), &fixture.policy)?;
        let mut writer =
            Memory::create_supported_situated_with_storage(&fixture.db, "synthetic", right)?;
        writer.capture(capture())?;
        drop(writer);
        let bytes = fs::read(&fixture.db)?;
        assert!(Memory::open(&fixture.db).is_err());
        assert!(Memory::open_read_only(&fixture.db).is_err());
        assert!(Memory::open_with_storage(&fixture.db, wrong.clone()).is_err());
        assert!(Memory::open_read_only_with_storage(&fixture.db, wrong).is_err());
        assert!(
            StorageAccess::load_key_file(&fixture.keys.join("missing.key"), &fixture.policy)
                .is_err()
        );
        assert_eq!(fs::read(&fixture.db)?, bytes);
        assert_eq!(fs::read_dir(fixture.db.parent().unwrap())?.count(), 1);
        Ok(())
    }

    #[test]
    fn key_loader_bounds_input_and_rejects_unsafe_files() -> Result<()> {
        let fixture = Fixture::new()?;
        let key = fixture.keys.join("input.key");
        let alternate = fixture.keys.join("alias.key");
        let expected = [b'e'; 64];
        for invalid in [
            vec![b'a'; 63],
            vec![b' '; 64],
            vec![0xff; 64],
            vec![b'a'; 4096],
        ] {
            fs::write(&key, invalid)?;
            fs::set_permissions(&key, fs::Permissions::from_mode(0o600))?;
            assert!(StorageAccess::load_key_file(&key, &fixture.policy).is_err());
        }
        fs::write(&key, expected)?;
        assert!(StorageAccess::load_key_file(&key, &fixture.policy)?.is_encrypted());
        fs::write(&key, [expected.as_slice(), b"\n"].concat())?;
        assert!(StorageAccess::load_key_file(&key, &fixture.policy)?.is_encrypted());
        fs::write(&key, [expected.as_slice(), b"\n\n"].concat())?;
        assert!(StorageAccess::load_key_file(&key, &fixture.policy).is_err());
        fs::write(&key, expected)?;
        symlink(&key, &alternate)?;
        assert!(StorageAccess::load_key_file(&alternate, &fixture.policy).is_err());
        assert!(StorageAccess::generate_key_file(&alternate, &fixture.policy).is_err());
        fs::remove_file(&alternate)?;
        fs::hard_link(&key, &alternate)?;
        assert!(StorageAccess::load_key_file(&key, &fixture.policy).is_err());
        fs::remove_file(&alternate)?;
        fs::set_permissions(&key, fs::Permissions::from_mode(0o640))?;
        assert!(StorageAccess::load_key_file(&key, &fixture.policy).is_err());
        fs::set_permissions(&key, fs::Permissions::from_mode(0o600))?;
        fs::set_permissions(&fixture.keys, fs::Permissions::from_mode(0o750))?;
        assert!(StorageAccess::load_key_file(&key, &fixture.policy).is_err());
        assert!(StorageAccess::generate_key_file(&alternate, &fixture.policy).is_err());
        assert_eq!(
            fs::metadata(&fixture.keys)?.permissions().mode() & 0o777,
            0o750
        );
        Ok(())
    }

    #[test]
    fn database_accepts_canonicalized_ancestor_alias_but_refuses_final_symlink() -> Result<()> {
        let fixture = Fixture::new()?;
        let storage = StorageAccess::generate_key_file(&fixture.keys.join("key"), &fixture.policy)?;
        let alias = fixture.root.join("data-alias");
        symlink(fixture.db.parent().unwrap(), &alias)?;
        let through_alias = alias.join(fixture.db.file_name().unwrap());
        let writer = Memory::create_supported_situated_with_storage(
            &through_alias,
            "synthetic",
            storage.clone(),
        )?;
        drop(writer);
        assert!(Memory::open_with_storage(&through_alias, storage.clone()).is_ok());
        let link = fixture.db.with_file_name("link.memory-prototype.sqlite");
        symlink(&fixture.db, &link)?;
        assert!(Memory::open_with_storage(&link, storage).is_err());
        Ok(())
    }

    #[test]
    fn key_policy_refuses_case_aliases_of_existing_excluded_directories() -> Result<()> {
        let fixture = Fixture::new()?;
        let alias = fixture.root.join("OWNER");
        // The alternate spelling must denote the same existing inode for
        // the probe to exercise containment on this filesystem.
        if !alias.exists() {
            return Ok(());
        }
        let policy = KeyFilePolicy::new(std::slice::from_ref(&fixture.keys))?;
        let key = alias.join("inside.key");
        let result = StorageAccess::generate_key_file(&key, &policy);
        let created = key.exists();
        assert!(
            result.is_err() && !created,
            "case alias crossed the exclusion"
        );
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_key_policy_refuses_extended_acls_even_with_private_mode_bits() -> Result<()> {
        use std::{fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt, process::Command};

        let fixture = Fixture::new()?;
        let key = fixture.keys.join("authored.key");
        let mut empty = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&key)?;
        let deny_xattrs = |path: &std::path::Path| -> Result<()> {
            // Restrict an empty fixture further. This ACE grants no access.
            assert!(Command::new("/bin/chmod")
                .args(["+a", "everyone deny writeextattr"])
                .arg(path)
                .status()?
                .success());
            Ok(())
        };
        deny_xattrs(&key)?;
        // Public fixture bytes. This file is never used to encrypt a store.
        empty.write_all(&[b'a'; 64])?;
        drop(empty);
        assert_eq!(fs::metadata(&key)?.permissions().mode() & 0o777, 0o600);
        let load = StorageAccess::load_key_file(&key, &fixture.policy);

        deny_xattrs(&fixture.keys)?;
        assert_eq!(
            fs::metadata(&fixture.keys)?.permissions().mode() & 0o777,
            0o700
        );
        let generated = fixture.keys.join("must-remain-absent.key");
        let create = StorageAccess::generate_key_file(&generated, &fixture.policy);
        assert!(load.is_err(), "extended ACL on key file was ignored");
        assert!(create.is_err(), "extended ACL on key directory was ignored");
        assert!(!generated.exists());
        Ok(())
    }

    #[test]
    fn key_generation_refuses_existing_or_excluded_paths() -> Result<()> {
        let fixture = Fixture::new()?;
        let key = fixture.keys.join("memory.key");
        let loaded = StorageAccess::generate_key_file(&key, &fixture.policy)?;
        let bytes = fs::read(&key)?;
        assert_eq!(fs::metadata(&key)?.permissions().mode() & 0o777, 0o600);
        assert!(StorageAccess::generate_key_file(&key, &fixture.policy).is_err());
        assert_eq!(fs::read(&key)?, bytes);
        let forbidden = fixture.db.with_extension("key");
        assert!(StorageAccess::generate_key_file(&forbidden, &fixture.policy).is_err());
        assert!(!forbidden.exists());
        let root_alias = fixture.root.join("owner-alias");
        symlink(&fixture.keys, &root_alias)?;
        assert!(
            StorageAccess::load_key_file(&root_alias.join("memory.key"), &fixture.policy).is_err()
        );
        let secret = std::str::from_utf8(&bytes)?.trim_end();
        assert!(!format!("{loaded:?}").contains(secret));
        Ok(())
    }
}

/// Creates a host-only capability outside the fixture's task root.
pub(crate) fn fixture_key(workspace: &std::path::Path) -> StorageAccess {
    use std::os::unix::fs::DirBuilderExt;
    let owner = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("storage-fixture-key-{}", uuid::Uuid::new_v4()));
    fs::DirBuilder::new().mode(0o700).create(&owner).unwrap();
    let policy = KeyFilePolicy::new(&[workspace.canonicalize().unwrap()]).unwrap();
    let access = StorageAccess::generate_key_file(&owner.join("memory.key"), &policy).unwrap();
    fs::remove_dir_all(owner).unwrap();
    access
}

#[cfg(unix)]
pub(super) use unix::Fixture as KeyScratch;
