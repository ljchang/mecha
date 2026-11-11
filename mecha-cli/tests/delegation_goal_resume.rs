//! Unsupported delegated resumes stop before setup or activation. The broken
//! fixture config is a setup sentinel, so no model, MCP server, or network is used.

use mecha_core::goal_context::{
    CharterSnapshot, DecisionBinding, GoalContextState, OwnerInput, OwnerSource,
};
use mecha_core::message::{Block, Message};
use mecha_core::questions::QuestionStore;
use mecha_core::session::{Record, Session, SessionKind, SessionMeta};
use std::path::PathBuf;
use std::process::Command;

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    workspace: PathBuf,
    session: Session,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Fixture {
    fn new(dynamic: bool, cleared: bool) -> Self {
        let root =
            std::env::temp_dir().join(format!("mecha-delegation-resume-{}", Session::new_id()));
        std::fs::create_dir_all(&root).unwrap();
        // Canonical paths keep the fixture valid on macOS's /var -> /private/var.
        let root = root.canonicalize().unwrap();
        let home = root.join("home");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        // Task feature gating reads only the global layer; the malformed
        // project layer is reached by setup. Dispatch's best-effort router
        // observers skip config errors, so the sentinel makes no network call.
        std::fs::write(home.join("config.toml"), "[features]\ngraph = true\n").unwrap();
        std::fs::write(workspace.join("mecha.toml"), "[resume_setup_sentinel\n").unwrap();
        let session = Session::create(
            &home.join("sessions"),
            SessionMeta {
                id: Session::new_id(),
                created_at: chrono::Utc::now(),
                provider: "fixture".into(),
                model: "fixture-model".into(),
                workspace: workspace.clone(),
                title: None,
                kind: Some(SessionKind::Chat),
            },
        )
        .unwrap();
        let messages = vec![Message::user("Inspect the garden.")];
        session
            .append(&Record::Message(messages[0].clone()))
            .unwrap();
        if dynamic {
            let mut state = GoalContextState::default();
            let version = state
                .transition(
                    CharterSnapshot::disabled(),
                    Some(
                        OwnerInput::new(OwnerSource::CliChat, "Inspect the garden.".into())
                            .unwrap(),
                    ),
                )
                .unwrap()
                .unwrap();
            let binding = DecisionBinding::new(1, version.revision(), &messages, None, true, 0)
                .unwrap()
                .with_conversation_messages(&messages)
                .unwrap();
            session
                .append(&Record::GoalContextDecision {
                    binding,
                    versions: vec![version],
                })
                .unwrap();
        }
        session
            .append(&Record::Message(Message::assistant(vec![Block::text(
                "Awaiting the owner.",
            )])))
            .unwrap();
        if cleared {
            session.append(&Record::GoalContextClear).unwrap();
        }
        assert_eq!(
            Session::load(&session.path)
                .unwrap()
                .1
                .has_dynamic_goal_context(),
            dynamic
        );
        mecha_core::archive::archive(&home.join("sessions"), &session.meta.id, chrono::Utc::now())
            .unwrap();
        Self {
            root,
            home,
            workspace,
            session,
        }
    }

    fn run(&self, args: &[&str]) -> String {
        let output = Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(args)
            .current_dir(&self.workspace)
            .env_clear()
            .env("HOME", self.root.join("user"))
            .env("MECHA_HOME", &self.home)
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap();
        assert!(!output.status.success(), "unexpected successful resume");
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    fn assert_session_untouched(&self, bytes: &[u8]) {
        assert_eq!(std::fs::read(&self.session.path).unwrap(), bytes);
        assert!(mecha_core::archive::is_archived(
            &self.home.join("sessions"),
            &self.session.meta.id
        ));
        assert!(!self.home.join("workflows").exists());
    }
}

#[test]
fn questions_answer_refuses_versioned_goal_resume_and_keeps_question_pending() {
    for cleared in [false, true] {
        let f = Fixture::new(true, cleared);
        let store = QuestionStore::open(f.home.join("questions")).unwrap();
        let q = store
            .park(
                "Which bed should be inspected?",
                vec![],
                &f.session.meta.id,
                Some("task-resume-fixture".into()),
                Some(f.workspace.clone()),
                Default::default(),
                None,
            )
            .unwrap();
        let question_path = store.root().join(format!("{}.json", q.id));
        let before_question = std::fs::read(&question_path).unwrap();
        let before_session = std::fs::read(&f.session.path).unwrap();
        let error = f.run(&["questions", "answer", &q.id, "The north bed."]);
        assert!(
            error.contains("questions answer cannot resume versioned goal context"),
            "{error}"
        );
        assert!(error.contains("question remains pending"), "{error}");
        assert!(
            !error.contains("resume_setup_sentinel"),
            "setup ran before refusal: {error}"
        );
        assert_eq!(std::fs::read(question_path).unwrap(), before_question);
        assert!(store.find(&q.id).unwrap().is_open());
        f.assert_session_untouched(&before_session);
    }
}

#[test]
fn tasks_work_refuses_versioned_goal_resume_before_workspace_or_setup() {
    for cleared in [false, true] {
        let f = Fixture::new(true, cleared);
        let before = std::fs::read(&f.session.path).unwrap();
        let error = f.run(&[
            "tasks",
            "work",
            "task-resume-fixture",
            "--resume",
            &f.session.meta.id,
        ]);
        assert!(
            error.contains("tasks work --resume cannot resume versioned goal context"),
            "{error}"
        );
        assert!(
            !error.contains("resume_setup_sentinel"),
            "setup ran before refusal: {error}"
        );
        assert!(
            !f.home.join("work").exists(),
            "unsupported resume created a task workspace"
        );
        f.assert_session_untouched(&before);
    }
}

#[test]
fn ordinary_question_resume_reaches_setup_sentinel() {
    let f = Fixture::new(false, false);
    let store = QuestionStore::open(f.home.join("questions")).unwrap();
    let q = store
        .park(
            "Which bed should be inspected?",
            vec![],
            &f.session.meta.id,
            None,
            Some(f.workspace.clone()),
            Default::default(),
            None,
        )
        .unwrap();
    let error = f.run(&["questions", "answer", &q.id, "The north bed."]);
    assert!(error.contains("resume_setup_sentinel"), "{error}");
    assert!(
        !error.contains("cannot resume versioned goal context"),
        "{error}"
    );
    assert!(store.find(&q.id).unwrap().is_open());
}

#[test]
fn ordinary_task_resume_reaches_setup_sentinel() {
    let f = Fixture::new(false, false);
    let error = f.run(&[
        "tasks",
        "work",
        "task-resume-fixture",
        "--resume",
        &f.session.meta.id,
    ]);
    assert!(error.contains("resume_setup_sentinel"), "{error}");
    assert!(
        !error.contains("cannot resume versioned goal context"),
        "{error}"
    );
    assert!(
        f.home.join("work").exists(),
        "ordinary task did not reach its existing setup path"
    );
}
