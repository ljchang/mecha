//! Closing or reopening a board task is one recorded event (S8,
//! `docs/APPRAISAL-WIRING-DESIGN.md`), exercised through the real binary
//! against the fixture board server — no provider, no network.
use mecha_core::closure::{Actor, ClosureStore, Entry, Move, RunPosture, Surface};
use mecha_core::shell_registry::{Registration, ShellRegistry};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Fixture {
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn board_server() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("eval")
        .join("fixtures")
        .join("board_server.py")
}

impl Fixture {
    /// A home whose config names the fixture board as the graph server, a
    /// board holding one open task, and optional extra config.
    fn new(extra_config: &str) -> Option<Self> {
        if Command::new("python3").arg("--version").output().is_err() {
            eprintln!("skipping: no python3 for the fixture board server");
            return None;
        }
        let root = std::env::temp_dir().join(format!(
            "mecha-closure-event-{}",
            mecha_core::session::Session::new_id()
        ));
        let store = root.join("board");
        std::fs::create_dir_all(root.join("work")).unwrap();
        std::fs::create_dir_all(root.join("home")).unwrap();
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(
            store.join("board.json"),
            json!({"v": 1, "next": 2, "tasks": [
                {"id": "task-1", "name": "Write the quarterly summary", "status": "next"}
            ]})
            .to_string(),
        )
        .unwrap();
        let config = format!(
            "[[mcp]]\nname = \"graph\"\ncommand = \"python3\"\nargs = [{:?}]\nprefix_tools = false\n\
             [mcp.env]\nMECHA_FIXTURE_DIR = {:?}\n{extra_config}",
            board_server().display().to_string(),
            store.display().to_string(),
        );
        std::fs::write(root.join("home/config.toml"), config).unwrap();
        Some(Fixture { root })
    }

    fn command(&self, args: &[&str], posture: Option<&str>) -> Output {
        self.command_with(args, posture, &[])
    }

    /// As [`Self::command`], with extra environment the command text could
    /// have exported — what a model's `bash -lc` hands the binary.
    fn command_with(&self, args: &[&str], posture: Option<&str>, env: &[(&str, &str)]) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_mecha"));
        c.args(args)
            .current_dir(self.root.join("work"))
            .env("MECHA_HOME", self.root.join("home"))
            .env("MECHA_SESSION_KIND", "test")
            .env_remove(mecha_core::closure::POSTURE_ENV);
        if let Some(p) = posture {
            c.env(mecha_core::closure::POSTURE_ENV, p);
        }
        for (k, v) in env {
            c.env(k, v);
        }
        c.output().unwrap()
    }

    fn status(&self) -> String {
        let board: Value = serde_json::from_str(
            &std::fs::read_to_string(self.root.join("board/board.json")).unwrap(),
        )
        .unwrap();
        board["tasks"][0]["status"].as_str().unwrap().to_string()
    }

    /// The fixture home's shell registry (`MECHA_HOME/runs/shells`).
    fn shells(&self) -> ShellRegistry {
        ShellRegistry::open(self.root.join("home/runs/shells")).unwrap()
    }

    /// Register *this test process* as a `shell` the harness spawned, with
    /// `posture` — so every `mecha` this test then spawns has a registered
    /// shell as its parent, exactly as a command a run's `shell` tool ran.
    fn under_shell(&self, posture: Option<RunPosture>) -> Registration {
        self.shells()
            .register(std::process::id(), posture, Some("call-test"))
            .unwrap()
    }

    fn store(&self) -> ClosureStore {
        ClosureStore::open(self.root.join("home/closures")).unwrap()
    }
}

fn ok(out: &Output) {
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn refused(out: &Output, needle: &str) {
    assert!(!out.status.success(), "must be refused");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains(needle), "{err}");
}

fn records(root: &Path) -> Vec<Entry> {
    ClosureStore::open(root.join("home/closures"))
        .unwrap()
        .entries()
        .unwrap()
}

#[test]
fn a_closure_and_its_reopen_are_recorded_and_joined() {
    let Some(f) = Fixture::new("") else { return };
    ok(&f.command(
        &[
            "tasks",
            "set",
            "task-1",
            "--status",
            "done",
            "--surface",
            "web",
        ],
        None,
    ));
    assert_eq!(f.status(), "done");
    let closed = f
        .store()
        .latest_closure("task-1")
        .unwrap()
        .expect("recorded");
    assert_eq!(closed.kind, Move::Close);
    assert_eq!(closed.from.as_deref(), Some("next"));
    assert_eq!((closed.actor, closed.surface), (Actor::Owner, Surface::Web));
    // The readout line is written even when there is nothing to appraise —
    // a task nobody delegated — so a surface can tell "said nothing" from
    // "was not read".
    assert!(records(&f.root)
        .iter()
        .any(|e| matches!(e, Entry::Readout { of, readout: None, .. } if *of == closed.id)));

    ok(&f.command(
        &[
            "tasks",
            "set",
            "task-1",
            "--status",
            "next",
            "--reason",
            "not finished",
        ],
        None,
    ));
    assert_eq!(f.status(), "next");
    let (reopen, _) = f.store().latest_with_readout("task-1").unwrap().unwrap();
    assert_eq!(reopen.kind, Move::Reopen);
    assert_eq!(reopen.surface, Surface::Cli, "no flag is a terminal");
    assert_eq!(reopen.undoes.as_deref(), Some(closed.id.as_str()));
    assert_eq!(reopen.reason.as_deref(), Some("not finished"));

    // A status change that crosses no line is not recorded.
    let before = records(&f.root).len();
    ok(&f.command(&["tasks", "set", "task-1", "--status", "waiting"], None));
    assert_eq!(records(&f.root).len(), before);
}

/// The residue `closure_guard.rs` named: a run with nobody present used to be
/// able to close its own task through `shell: mecha tasks set`. It is refused
/// now, before anything is recorded or moved — the posture read from the
/// harness's shell registry, not from the command's environment (1b-2).
#[test]
fn a_run_with_nobody_present_cannot_close_through_the_shell() {
    let Some(f) = Fixture::new("") else { return };
    for (posture, stamp) in [
        (Some(RunPosture::Delegated), "delegated"),
        (Some(RunPosture::Unattended), "unattended"),
        (None, "unknown"),
    ] {
        let _shell = f.under_shell(posture);
        refused(
            &f.command(&["tasks", "set", "task-1", "--status", "done"], Some(stamp)),
            "owner",
        );
        assert_eq!(f.status(), "next", "{stamp}: the board must not move");
    }
    assert!(records(&f.root).is_empty(), "a refusal records nothing");

    // A chat the owner is in may, behind its approver — recorded as such,
    // and the surface cannot be claimed.
    let _shell = f.under_shell(Some(RunPosture::Interactive));
    ok(&f.command(
        &[
            "tasks",
            "set",
            "task-1",
            "--status",
            "done",
            "--surface",
            "web",
        ],
        Some("interactive"),
    ));
    let closed = f.store().latest_closure("task-1").unwrap().unwrap();
    assert_eq!(
        (closed.actor, closed.surface),
        (Actor::OwnerApproved, Surface::Chat)
    );
}

/// #293's review: a delegated run's command could set the posture variable
/// itself — `MECHA_RUN_POSTURE=interactive mecha tasks set …` — and be
/// recorded `owner-approved`. The registered shell's posture wins over the
/// variable, so it is refused. Fails on #293's code, which read the variable.
#[test]
fn a_delegated_shell_that_sets_the_variable_itself_is_still_refused() {
    let Some(f) = Fixture::new("") else { return };
    let _shell = f.under_shell(Some(RunPosture::Delegated));
    refused(
        &f.command(
            &["tasks", "set", "task-1", "--status", "done"],
            Some("interactive"),
        ),
        "delegated run's shell",
    );
    assert_eq!(f.status(), "next");
    assert!(records(&f.root).is_empty());
}

/// Review of #294: the registry's reader used to honour `MECHA_SHELLS_DIR`,
/// so a delegated run's command could point it at an empty directory, drop
/// the posture variable, and read as the owner's terminal (rule 4). The
/// location has no override now: the command is still refused under its
/// registered delegated shell. Fails on the head that honoured the variable.
#[test]
fn a_command_cannot_point_the_registry_somewhere_else() {
    let Some(f) = Fixture::new("") else { return };
    let _shell = f.under_shell(Some(RunPosture::Delegated));
    let decoy = f.root.join("decoy-shells");
    std::fs::create_dir_all(&decoy).unwrap();
    refused(
        &f.command_with(
            &["tasks", "set", "task-1", "--status", "done"],
            None,
            &[("MECHA_SHELLS_DIR", decoy.to_str().unwrap())],
        ),
        "delegated run's shell",
    );
    assert_eq!(f.status(), "next");
    assert!(records(&f.root).is_empty());
}

/// The variable with no registered shell behind it is a claim no marker
/// confirms, and refuses in every value — `interactive` first.
#[test]
fn a_claimed_posture_with_no_registered_shell_is_refused() {
    let Some(f) = Fixture::new("") else { return };
    for stamp in ["interactive", "delegated"] {
        refused(
            &f.command(&["tasks", "set", "task-1", "--status", "done"], Some(stamp)),
            "no registered mecha shell",
        );
    }
    assert_eq!(f.status(), "next");
    assert!(records(&f.root).is_empty());
}

/// Registering the *shell child*, not the hosting process: the owner's own
/// close — a `serve` board tap, whose parent is no registered shell — goes
/// through as the owner even while another run's delegated shell is live
/// and registered elsewhere.
#[test]
fn an_owner_close_is_unaffected_by_another_runs_registered_shell() {
    let Some(f) = Fixture::new("") else { return };
    let mut other = Command::new("sleep").arg("30").spawn().unwrap();
    let _elsewhere = f
        .shells()
        .register(other.id(), Some(RunPosture::Delegated), None)
        .unwrap();
    ok(&f.command(
        &[
            "tasks",
            "set",
            "task-1",
            "--status",
            "done",
            "--surface",
            "web",
        ],
        None,
    ));
    let closed = f.store().latest_closure("task-1").unwrap().unwrap();
    assert_eq!((closed.actor, closed.surface), (Actor::Owner, Surface::Web));
    let _ = other.kill();
    let _ = other.wait();
}

#[test]
fn a_denying_pre_task_close_hook_stops_the_move_with_nothing_changed() {
    let Some(f) = Fixture::new(
        "[[hook]]\nevent = \"pre_task_close\"\ncommand = \"echo 'not this week'; exit 2\"\n",
    ) else {
        return;
    };
    refused(
        &f.command(&["tasks", "set", "task-1", "--status", "done"], None),
        "not this week",
    );
    assert_eq!(f.status(), "next");
    assert!(records(&f.root).is_empty());
}

#[test]
fn an_observer_hook_receives_the_record() {
    let Some(f) = Fixture::new("") else { return };
    let seen = f.root.join("seen.json");
    std::fs::write(
        f.root.join("home/config.toml"),
        format!(
            "{}[[hook]]\nevent = \"task_closed\"\ncommand = \"cat > {}\"\n",
            std::fs::read_to_string(f.root.join("home/config.toml")).unwrap(),
            seen.display()
        ),
    )
    .unwrap();
    ok(&f.command(&["tasks", "set", "task-1", "--status", "dropped"], None));
    let payload: Value = serde_json::from_str(&std::fs::read_to_string(&seen).unwrap()).unwrap();
    assert_eq!(payload["event"], "task_closed");
    assert_eq!(payload["record"]["task"], "task-1");
    assert_eq!(payload["record"]["to"], "dropped");
    assert_eq!(payload["record"]["move"], "close");
}

#[test]
fn a_surface_the_build_cannot_name_is_refused_before_anything_moves() {
    let Some(f) = Fixture::new("") else { return };
    refused(
        &f.command(
            &[
                "tasks",
                "set",
                "task-1",
                "--status",
                "done",
                "--surface",
                "chat",
            ],
            None,
        ),
        "--surface",
    );
    assert_eq!(f.status(), "next");
}

/// A delegated run that strips the posture variable from its command is
/// still that run's child: the ancestry check refuses it. The test process
/// stands in for the run — its own pid in a live task-run marker makes it an
/// ancestor of the `mecha` it spawns.
// The ancestry check reads `/proc`, so it is Linux-only by construction
// (documented on `closure::parent_of` and in ARCHITECTURE); off Linux this
// test would assert a protection the platform cannot give (macOS CI).
#[cfg(target_os = "linux")]
#[test]
fn a_command_descended_from_a_live_task_run_is_refused_without_the_variable() {
    let Some(f) = Fixture::new("") else { return };
    let markers = mecha_core::runmarker::RunMarkers::new(f.root.join("home/taskruns"));
    std::fs::create_dir_all(markers.dir()).unwrap();
    std::fs::write(
        markers.dir().join("task-9.running"),
        json!({"pid": std::process::id(), "started_at": "2026-09-24T00:00:00Z"}).to_string(),
    )
    .unwrap();
    refused(
        &f.command(&["tasks", "set", "task-1", "--status", "done"], None),
        "delegated or scheduled run",
    );
    assert_eq!(f.status(), "next");
    assert!(records(&f.root).is_empty());
}

/// The silently-degrading guard: a closure whose record cannot be written
/// does not happen. The ledger's path is made a directory, so the append
/// fails whatever the process's privileges.
#[test]
fn a_closure_that_cannot_be_recorded_does_not_happen() {
    let Some(f) = Fixture::new("") else { return };
    std::fs::create_dir_all(f.root.join("home/closures/closures.jsonl")).unwrap();
    refused(
        &f.command(&["tasks", "set", "task-1", "--status", "done"], None),
        "nothing was changed",
    );
    assert_eq!(f.status(), "next");
}

/// A board the server truncated cannot say whether a missing row exists, so
/// a status change to a row past the cut is refused as *unknown* — naming
/// the truncation — never as "no such task" (found on review: the pre-read
/// is now load-bearing, and the old scan ignored `truncated`). With a cap of
/// one over two tasks, exactly one of the two moves falls past the cut.
#[test]
fn a_row_past_a_truncated_board_is_refused_as_unknown_not_missing() {
    let Some(f) = Fixture::new("MECHA_FIXTURE_BOARD_CAP = \"1\"\n") else {
        return;
    };
    std::fs::write(
        f.root.join("board/board.json"),
        json!({"v": 1, "next": 3, "tasks": [
            {"id": "task-1", "name": "Write the quarterly summary", "status": "next"},
            {"id": "task-2", "name": "Book the room", "status": "next"}
        ]})
        .to_string(),
    )
    .unwrap();
    // Which row the cap cut depends on the server's order, so ask it:
    // closing the visible one first would re-sort the board and bring the
    // other inside the cap.
    let listed: Value = serde_json::from_slice(
        &f.command(&["tasks", "list", "--json", "--closed"], None)
            .stdout,
    )
    .unwrap();
    assert_eq!(listed["truncated"], json!(true), "{listed}");
    let visible = listed["items"][0]["id"].as_str().unwrap().to_string();
    let hidden = if visible == "task-1" {
        "task-2"
    } else {
        "task-1"
    };
    let out = f.command(&["tasks", "set", hidden, "--status", "done"], None);
    refused(&out, "truncated");
    assert!(!String::from_utf8_lossy(&out.stderr).contains("no such task"));
    let board: Value =
        serde_json::from_str(&std::fs::read_to_string(f.root.join("board/board.json")).unwrap())
            .unwrap();
    let row = board["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == json!(hidden))
        .unwrap();
    assert_eq!(row["status"], json!("next"), "nothing moved");
}

/// Seed the fixture home's closure store with a transition whose board write
/// ended with an unknown outcome, as `mecha tasks set` records one when the
/// graph server's reply is lost.
fn seed_uncertain(f: &Fixture, from: &str, to: &str) -> String {
    use mecha_core::closure::{Actor, Move, Surface, Transition};
    let store = ClosureStore::open(f.root.join("home/closures")).unwrap();
    let t = Transition::new(
        "task-1",
        Some(from),
        to,
        Move::Close,
        Actor::Owner,
        Surface::Cli,
        vec![],
        None,
    );
    store.append(&Entry::Transition(t.clone())).unwrap();
    store
        .append(&Entry::Uncertain {
            of: t.id.clone(),
            at: chrono::Utc::now(),
            error: "connection reset".into(),
        })
        .unwrap();
    t.id
}

fn set_board_status(f: &Fixture, status: &str) {
    let path = f.root.join("board/board.json");
    let mut board: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    board["tasks"][0]["status"] = json!(status);
    std::fs::write(&path, board.to_string()).unwrap();
}

/// A closure whose reply was lost but which did land is confirmed by the
/// next status change, not left unconfirmed forever — the retry finds the
/// board already `done`, classifies no move, and used to record nothing
/// (review of #293).
#[test]
fn an_uncertain_closure_that_landed_is_confirmed_by_the_next_status_change() {
    let Some(f) = Fixture::new("") else {
        return;
    };
    let id = seed_uncertain(&f, "next", "done");
    set_board_status(&f, "done");
    let out = f.command(&["tasks", "set", "task-1", "--status", "done"], None);
    ok(&out);
    assert!(String::from_utf8_lossy(&out.stderr).contains("did land"));
    assert!(records(&f.root)
        .iter()
        .any(|e| matches!(e, Entry::Confirmed { of, .. } if *of == id)));
    assert_eq!(f.store().unresolved_uncertain("task-1").unwrap(), None);
    assert_eq!(
        f.store().transitions().unwrap().len(),
        1,
        "the closure stands"
    );
}

/// And one that did not land is withdrawn, so a reader never counts it.
#[test]
fn an_uncertain_closure_that_did_not_land_is_withdrawn_by_the_next_status_change() {
    let Some(f) = Fixture::new("") else {
        return;
    };
    let id = seed_uncertain(&f, "next", "done");
    let out = f.command(&["tasks", "set", "task-1", "--status", "waiting"], None);
    ok(&out);
    assert!(String::from_utf8_lossy(&out.stderr).contains("did not land"));
    assert!(records(&f.root)
        .iter()
        .any(|e| matches!(e, Entry::Aborted { of, .. } if *of == id)));
    assert!(f.store().transitions().unwrap().is_empty());
}

/// A board at neither end of the uncertain move is evidence of nothing —
/// something else moved the row — so the record stays uncertain rather than
/// being withdrawn as a move that did not happen (review of #293: the
/// earlier two-way read withdrew it).
#[test]
fn an_uncertain_closure_with_the_board_at_neither_end_stays_uncertain() {
    let Some(f) = Fixture::new("") else {
        return;
    };
    let id = seed_uncertain(&f, "next", "done");
    set_board_status(&f, "waiting");
    let out = f.command(&["tasks", "set", "task-1", "--status", "next"], None);
    ok(&out);
    assert!(String::from_utf8_lossy(&out.stderr).contains("still unsettled"));
    assert!(!records(&f.root)
        .iter()
        .any(|e| matches!(e, Entry::Aborted { of, .. } if *of == id)));
    assert_eq!(
        f.store()
            .unresolved_uncertain("task-1")
            .unwrap()
            .map(|t| t.id),
        Some(id)
    );
}
