//! Where the owner's memory lives when no manifest names another place:
//! `<mecha home>/memory/`, three siblings that never contain one another.
//!
//! ```text
//! memory/store/agent.memory-prototype.sqlite   the encrypted database
//! memory/key/agent.key                         its key, in a directory of its own
//! memory/authority/                            the store's identity and per-run seals
//! ```
//!
//! The key is beside the store rather than inside it because the key policy
//! refuses a key under the database's directory or the authority's
//! (`memory::KeyFilePolicy`), and the three are fixed in code rather than read
//! from configuration: a cloned repository's `mecha.toml` must not be able to
//! say where the owner's memory or its key is.
//!
//! The home is `work::mecha_home()`, so the store follows `MECHA_HOME`, and
//! that is deliberate: a redirected home gets a store of its own beside its
//! other state, and can never read or replace another home's — the session
//! layer admits only the directory of the home it was started in.
//!
//! The paths and [`initialized`] are in every build, so `mecha features` can
//! say what a build without the store would need; opening one is the cargo
//! feature `memory-prototype`'s ([`BUILT`]).

use std::path::{Path, PathBuf};

/// Whether this binary can open a memory store at all: the cargo feature
/// `memory-prototype`. A build without it still knows the feature, and says
/// by name that it cannot run it (`feature::unbuilt`).
pub const BUILT: bool = cfg!(feature = "memory-prototype");

/// The command that creates the home store, as every surface prints it.
pub const INIT_COMMAND: &str = "mecha memory-session init";

/// `<home>/memory`.
pub fn dir(home: &Path) -> PathBuf {
    home.join("memory")
}

/// The database. Its directory is the one place under a mecha home the
/// session layer admits a store to (`StorageAccess::admitting_home_store`).
pub fn db(home: &Path) -> PathBuf {
    dir(home)
        .join("store")
        .join("agent.memory-prototype.sqlite")
}

/// The store's key file.
pub fn key_file(home: &Path) -> PathBuf {
    dir(home).join("key").join("agent.key")
}

/// The authority directory: the store's identity, and what each run sealed.
pub fn authority(home: &Path) -> PathBuf {
    dir(home).join("authority")
}

/// What `home` holds of a memory store, from one read of its three paths.
///
/// Four answers kept apart, because each has a different next step and
/// `init` treats them differently: nothing (run `init`), all of it, some of
/// it (`init` refuses, so a row must not offer it), and could not be read —
/// which is a finding of its own, never an empty home.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Store {
    /// None of the three paths exists.
    #[default]
    Absent,
    /// The database, its key and the authority's identity, each a regular
    /// file. A symlink is not one: every opener of the three refuses a link,
    /// and a row that says on for a store nothing can open is the wrong
    /// answer.
    Whole,
    /// Something is at one of the paths, in any form, and it is not `Whole`.
    Partial,
    /// A path could not be read for a reason other than its absence.
    Unreadable,
}

/// One path, by the link and never its target: absent, a regular file,
/// something else, or unreadable.
fn entry(path: &Path) -> std::io::Result<Option<bool>> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => Ok(Some(meta.is_file())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Read what `home` holds. No open, no key read: a page load asks this.
pub fn store(home: &Path) -> Store {
    let read = [
        entry(&db(home)),
        entry(&key_file(home)),
        entry(&authority(home).join("identity")),
        entry(&authority(home)),
    ];
    if read.iter().any(|r| r.is_err()) {
        return Store::Unreadable;
    }
    let [db, key, identity, authority] = read.map(|r| r.unwrap_or(None));
    if [db, key, identity] == [Some(true); 3] {
        Store::Whole
    } else if [db, key, identity, authority] == [None; 4] {
        Store::Absent
    } else {
        Store::Partial
    }
}

/// Whether `home` holds a whole store ([`Store::Whole`]).
pub fn initialized(home: &Path) -> bool {
    store(home) == Store::Whole
}

/// What a row says to do about a [`Store::Partial`] store.
pub const REPAIR: &str =
    "restore the missing file under the mecha home's memory/, or move memory/ aside and run \
     `mecha memory-session init`";

/// What a row says to do about a [`Store::Unreadable`] store.
pub const READABLE: &str =
    "make the mecha home's memory/ and what is under it readable by you again";

#[cfg(feature = "memory-prototype")]
mod open {
    use super::*;
    use crate::memory_session::{SessionMemory, SessionMemoryRequest, SESSION_MEMORY_PROTOCOL};
    use anyhow::{ensure, Context, Result};
    use std::os::unix::fs::DirBuilderExt;

    /// The home store's identity. Written into the store at `init` and
    /// compared on every open, so these are a wire format: changing one
    /// orphans every store made before the change.
    const DOMAIN: &str = "home";
    const ACCOUNT: &str = "assistant";
    const SCOPE: &str = "agent-experience";

    /// The request that names the home store, as a manifest would have.
    pub fn request(home: &Path) -> Result<SessionMemoryRequest> {
        let home = std::path::absolute(home).context("resolve the mecha home")?;
        Ok(SessionMemoryRequest {
            protocol: SESSION_MEMORY_PROTOCOL.into(),
            db: db(&home),
            key_file: key_file(&home),
            authority: authority(&home),
            domain: DOMAIN.into(),
            account: ACCOUNT.into(),
            scopes: [SCOPE.to_string()].into(),
        })
    }

    /// Create the home store: its private directories, a fresh key, the
    /// encrypted database and the authority. Refuses a home that already
    /// holds any of the three, whole or partial — a second `init` must never
    /// replace a key.
    pub fn initialize(home: &Path) -> Result<SessionMemoryRequest> {
        let request = request(home)?;
        // By the link, as `store` reads: a dangling symlink is something
        // here, and `init` refuses it rather than removing what it did not
        // make. An error other than absence is the caller's to see.
        for path in [&request.db, &request.key_file, &request.authority] {
            ensure!(
                super::entry(path)
                    .with_context(|| format!("reading {}", path.display()))?
                    .is_none(),
                "{} already exists — the home store is initialized once, and a \
                 partial one is repaired by hand, never built over",
                path.display()
            );
        }
        for directory in [
            request
                .authority
                .parent()
                .expect("the memory directory")
                .to_path_buf(),
            request
                .db
                .parent()
                .expect("a store directory")
                .to_path_buf(),
            request
                .key_file
                .parent()
                .expect("a key directory")
                .to_path_buf(),
        ] {
            if !directory.is_dir() {
                std::fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(&directory)
                    .with_context(|| format!("creating {}", directory.display()))?;
            }
        }
        // Every path was just proved absent, so a failure past this point
        // can only have left this call's own leftovers — a key written before
        // the database could be made, say. Take them back: a half-made store
        // is one `init` itself would then refuse.
        if let Err(error) = SessionMemory::initialize(&request) {
            let _ = std::fs::remove_file(&request.db);
            let _ = std::fs::remove_file(&request.key_file);
            let _ = std::fs::remove_dir_all(&request.authority);
            return Err(error);
        }
        Ok(request)
    }
}

#[cfg(feature = "memory-prototype")]
pub use open::{initialize, request};

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory removed when the test ends.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Scratch {
            let dir = std::env::temp_dir().join(format!("mecha-memhome-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_three_paths_never_contain_one_another() {
        let home = Path::new("/h/.mecha");
        let (db, key, authority) = (db(home), key_file(home), authority(home));
        let store = db.parent().unwrap();
        assert!(!key.starts_with(store) && !key.starts_with(&authority));
        assert!(!db.starts_with(&authority) && !authority.starts_with(store));
        assert!(db.ends_with(".mecha/memory/store/agent.memory-prototype.sqlite"));
    }

    #[test]
    fn a_partial_store_is_not_initialized() {
        let scratch = Scratch::new();
        let home = scratch.0.as_path();
        assert_eq!(store(home), Store::Absent);
        for file in [db(home), key_file(home)] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, b"x").unwrap();
            assert!(!initialized(home));
        }
        std::fs::create_dir_all(authority(home)).unwrap();
        std::fs::write(authority(home).join("identity"), b"x").unwrap();
        assert!(initialized(home));
        // Relocated and left behind as a link: every opener refuses it, so
        // the row must not call it a store.
        let moved = home.join("moved.key");
        std::fs::rename(key_file(home), &moved).unwrap();
        std::os::unix::fs::symlink(&moved, key_file(home)).unwrap();
        assert_eq!(store(home), Store::Partial);
    }

    /// Unreadable is its own answer, never an empty home: a directory the
    /// owner cannot search hides whether anything is in it.
    #[test]
    fn a_store_that_cannot_be_read_is_not_an_absent_one() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::new();
        let home = scratch.0.as_path();
        std::fs::create_dir_all(dir(home).join("store")).unwrap();
        std::fs::write(db(home), b"x").unwrap();
        let locked = dir(home).join("store");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Root reads through any mode; the assertion means something only
        // where the lock holds.
        let holds = std::fs::symlink_metadata(db(home)).is_err();
        let read = store(home);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        if holds {
            assert_eq!(read, Store::Unreadable);
        }
    }

    /// The whole path, on a home spelled as the real one is: `init` makes a
    /// store the session API opens, and a second `init` refuses. The home is
    /// this process's (`MECHA_HOME`, under the guard's lock), because that is
    /// the only home whose store the session layer admits — and the same
    /// request against a home that is not this process's is refused.
    #[cfg(feature = "memory-prototype")]
    #[test]
    fn init_makes_a_store_under_this_process_s_home_that_opens_and_is_made_once() {
        use crate::memory_session::SessionMemory;
        let guard = crate::work::tests::HomeGuard::new();
        let root = guard.dir().canonicalize().unwrap();
        let home = root.join(".mecha");
        std::fs::create_dir(&home).unwrap();
        let workspace = root.join("work");
        std::fs::create_dir(&workspace).unwrap();

        // Not this process's home yet: the spelling alone admits nothing,
        // and the refused attempt leaves no half-made store behind.
        let refused = initialize(&home).unwrap_err();
        assert!(
            format!("{refused:#}").contains("live Mecha directories"),
            "{refused:#}"
        );
        assert_eq!(store(&home), Store::Absent);

        std::env::set_var("MECHA_HOME", &home);
        let request = initialize(&home).unwrap();
        assert!(initialized(&home));
        assert_eq!(request, super::request(&home).unwrap());
        SessionMemory::open(&request, &workspace).unwrap();
        SessionMemory::open_for_recovery(&request).unwrap();

        // A manifest cannot claim the home store's database with a key and
        // authority of its own: the admission is for the whole home request.
        let foreign = root.join("foreign");
        std::fs::create_dir(&foreign).unwrap();
        std::fs::set_permissions(
            &foreign,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        let mut claim = request.clone();
        claim.key_file = foreign.join("k.key");
        claim.authority = foreign.join("authority");
        std::fs::rename(db(&home), root.join("aside.sqlite")).unwrap();
        let refused = SessionMemory::initialize(&claim).unwrap_err();
        assert!(
            format!("{refused:#}").contains("live Mecha directories"),
            "{refused:#}"
        );
        assert!(!db(&home).exists());
        std::fs::rename(root.join("aside.sqlite"), db(&home)).unwrap();

        // A dangling link where the key would go is something there: `init`
        // refuses it and leaves it, where following the link read it as
        // absent and the failed attempt then removed it.
        let other = root.join("other-home").join(".mecha");
        std::fs::create_dir_all(dir(&other).join("key")).unwrap();
        std::os::unix::fs::symlink(root.join("nowhere"), key_file(&other)).unwrap();
        std::env::set_var("MECHA_HOME", &other);
        assert_eq!(store(&other), Store::Partial);
        let refused = initialize(&other).unwrap_err().to_string();
        assert!(refused.contains("already exists"), "{refused}");
        assert!(std::fs::symlink_metadata(key_file(&other)).is_ok());
        std::env::set_var("MECHA_HOME", &home);

        let key = std::fs::read(key_file(&home)).unwrap();
        let again = initialize(&home).unwrap_err().to_string();
        assert!(again.contains("already exists"), "{again}");
        assert_eq!(std::fs::read(key_file(&home)).unwrap(), key);
    }
}
