//! A draft released unchanged is the owner's verdict — +1.0, an
//! owner-verified success, a writing exemplar — only when the owner
//! released it (`docs/APPRAISAL-WIRING-DESIGN.md`, R16a's ruling D3 carried
//! to releases, 2026-09-27). Exercised through the real binary: `mecha
//! outbox approve -y` releases a staged `fs_list` call (a builtin, so no MCP
//! server and no network), and `mecha sessions successes` reads the set.
// **These cases read the owner's real registry too**, as
// `outbox_resolve_actor.rs` and `closure_event.rs` do: the fixture sets
// `MECHA_HOME`, so `work::guard_homes` names the fixture *and* the real
// `~/.mecha`. Run the suite from a plain terminal, not a mecha run's shell.
// The shell case needs the `/proc` walk; off Linux its helper is unused.
#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

use mecha_core::agent::Taint;
use mecha_core::closure::{Actor, RunPosture};
use mecha_core::outbox::{OutboxKind, OutboxStore, Provenance};
use mecha_core::session::{Record, RunStats, SessionKind, SessionMeta};
use mecha_core::shell_registry::{Registration, ShellRegistry};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Command, Output};

const SESSION: &str = "20260927T090000-northwind";
const BODY: &str = "Hi Sam,\n\nThursday at 2pm works for the Northwind Labs visit.\n\nDana";

struct Fixture {
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Fixture {
    /// A home holding one completed session (recorded as a test, so every
    /// readout here passes `--include-tests`) and one pending draft it
    /// staged: a builtin `fs_list` call carrying a message body, so a
    /// release executes for real with nothing outside the process.
    fn new() -> (Self, String) {
        let root = std::env::temp_dir().join(format!(
            "mecha-release-actor-{}",
            mecha_core::session::Session::new_id()
        ));
        let home = root.join("home");
        std::fs::create_dir_all(home.join("sessions")).unwrap();
        std::fs::create_dir_all(root.join("work")).unwrap();
        let records = [
            Record::Meta(SessionMeta {
                id: SESSION.into(),
                created_at: chrono::Utc::now(),
                provider: "scripted".into(),
                model: "scripted".into(),
                workspace: root.join("work"),
                title: None,
                kind: Some(SessionKind::Test),
            }),
            Record::Outcome(RunStats {
                stop_cause: Some(mecha_core::agent::StopCause::Completed),
                ..Default::default()
            }),
        ];
        std::fs::write(
            home.join("sessions").join(format!("{SESSION}.jsonl")),
            records
                .iter()
                .map(|r| serde_json::to_string(r).unwrap())
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let staged = OutboxStore::open(home.join("outbox"))
            .unwrap()
            .stage(
                "fs_list",
                OutboxKind::Message,
                json!({"to": "sam@example.edu", "subject": "Northwind visit", "body_markdown": BODY}),
                Taint::default(),
                Provenance {
                    session_id: Some(SESSION.into()),
                    workspace: Some(root.join("work")),
                    ..Default::default()
                },
            )
            .unwrap();
        (Fixture { root }, staged.id)
    }

    fn mecha(&self, args: &[&str]) -> Output {
        let out = Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(args)
            .current_dir(self.root.join("work"))
            .env("MECHA_HOME", self.root.join("home"))
            .env("MECHA_SESSION_KIND", "test")
            .env_remove("MECHA_SESSION_DIR")
            .env_remove("MECHA_OUTBOX_DIR")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .env_remove(mecha_core::closure::POSTURE_ENV)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "mecha {args:?} failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }

    fn successes(&self) -> Value {
        let out = self.mecha(&["sessions", "successes", "--json", "--include-tests"]);
        serde_json::from_slice(&out.stdout).unwrap()
    }

    fn resolved_by(&self, id: &str) -> Option<Actor> {
        OutboxStore::open(self.root.join("home/outbox"))
            .unwrap()
            .item(id)
            .unwrap()
            .resolved_by
    }

    /// Register *this test process* as a `shell` the harness spawned, so the
    /// `mecha` it then runs is a command a run's `shell` tool ran.
    fn under_shell(&self, posture: RunPosture) -> Registration {
        ShellRegistry::open(self.root.join("home/runs/shells"))
            .unwrap()
            .register(std::process::id(), Some(posture), Some("call-test"))
            .unwrap()
    }
}

/// The owner releases the draft unchanged at their own terminal: one
/// standing success, the draft its exemplar.
#[test]
fn an_owners_unchanged_release_is_a_success() {
    let (f, id) = Fixture::new();
    f.mecha(&["outbox", "approve", &id, "-y"]);
    assert_eq!(f.resolved_by(&id), Some(Actor::Owner));
    let set = f.successes();
    assert_eq!(set["by_kind"]["sent_unchanged"], 1, "{set:#}");
    assert_eq!(set["exemplars"]["count"], 1, "{set:#}");
}

/// The forge: a run's `shell` that runs `mecha outbox approve -y` on its own
/// draft, unchanged, earns no success and no exemplar — behind an
/// interactive run's approver (`owner-approved`) or with nobody present
/// (`unknown`). Fails on the tree before this ruling, which counted every
/// unchanged release as the owner's verdict whoever made it.
#[cfg(target_os = "linux")]
#[test]
fn a_runs_own_unchanged_release_is_no_success() {
    for (posture, expected) in [
        (RunPosture::Interactive, Actor::OwnerApproved),
        (RunPosture::Delegated, Actor::Unknown),
        (RunPosture::Unattended, Actor::Unknown),
    ] {
        let (f, id) = Fixture::new();
        let shell = f.under_shell(posture);
        f.mecha(&["outbox", "approve", &id, "-y"]);
        drop(shell);
        assert_eq!(f.resolved_by(&id), Some(expected), "{posture:?}");
        let set = f.successes();
        assert!(
            set["by_kind"]["sent_unchanged"].is_null() || set["by_kind"]["sent_unchanged"] == 0,
            "{posture:?}: {set:#}"
        );
        assert_eq!(set["exemplars"]["count"], 0, "{posture:?}: {set:#}");
        assert_eq!(set["standing"], 0, "{posture:?}: {set:#}");
    }
}
