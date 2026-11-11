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

/// The database. Its directory is the one place under a mecha home the store
/// accepts (`memory::sql::validate_path`).
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

/// Whether `home` holds an initialized store — all three of the database, its
/// key and the authority's identity, as files. No open, no key read: a page
/// load asks this. A partial set is not initialized, and `init` refuses it by
/// name rather than building over it.
pub fn initialized(home: &Path) -> bool {
    db(home).is_file() && key_file(home).is_file() && authority(home).join("identity").is_file()
}

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
        for path in [&request.db, &request.key_file, &request.authority] {
            ensure!(
                !path.try_exists()?,
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
        SessionMemory::initialize(&request)?;
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
        assert!(!initialized(home));
        for file in [db(home), key_file(home)] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, b"x").unwrap();
            assert!(!initialized(home));
        }
        std::fs::create_dir_all(authority(home)).unwrap();
        std::fs::write(authority(home).join("identity"), b"x").unwrap();
        assert!(initialized(home));
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

        // Not this process's home yet: the spelling alone admits nothing.
        let refused = initialize(&home).unwrap_err();
        assert!(
            format!("{refused:#}").contains("live Mecha directories"),
            "{refused:#}"
        );
        std::fs::remove_dir_all(dir(&home)).unwrap();

        std::env::set_var("MECHA_HOME", &home);
        let request = initialize(&home).unwrap();
        assert!(initialized(&home));
        assert_eq!(request, super::request(&home).unwrap());
        SessionMemory::open(&request, &workspace).unwrap();
        SessionMemory::open_for_recovery(&request).unwrap();

        let key = std::fs::read(key_file(&home)).unwrap();
        let again = initialize(&home).unwrap_err().to_string();
        assert!(again.contains("already exists"), "{again}");
        assert_eq!(std::fs::read(key_file(&home)).unwrap(), key);
    }
}
