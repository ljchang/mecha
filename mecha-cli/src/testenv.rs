//! Test-only guard for moving `MECHA_HOME`, shared across every test module
//! in this binary that needs one.
//!
//! The variable is process-global and `cargo test` runs tests in parallel
//! threads of one process, so two modules each holding their *own* mutex
//! around it would serialise against themselves and still race each other —
//! the lock only means something if there is exactly one. `slack/show.rs`
//! carried the first copy of this guard; the serve settings tests needed a
//! second, which is the moment a private copy stops being private and
//! starts being a race.

use std::sync::{Mutex, MutexGuard};

static ENV: Mutex<()> = Mutex::new(());

pub(crate) struct HomeGuard {
    _lock: MutexGuard<'static, ()>,
    previous: Option<String>,
    pub(crate) dir: std::path::PathBuf,
}

impl HomeGuard {
    pub(crate) fn new(tag: &str) -> Self {
        let lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var("MECHA_HOME").ok();
        let dir = std::env::temp_dir().join(format!("mecha-{tag}-{}", std::process::id()));
        // A fresh home every acquisition: a leftover from a killed run must
        // not leak state into this test.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("MECHA_HOME", &dir);
        HomeGuard {
            _lock: lock,
            previous,
            dir,
        }
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(v) => std::env::set_var("MECHA_HOME", v),
            None => std::env::remove_var("MECHA_HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
