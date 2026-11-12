//! An ordinary run is remembered whatever tools it used and however it went:
//! what the recorder cannot place is left out of the capture, never a reason
//! to refuse it, and whether the model's own words can still be stored as
//! local is settled against the transcript's recorded taint.
#![cfg(feature = "memory-prototype")]
use chrono::Utc;
use mecha_core::{
    agent::{StopCause, Taint},
    memory_session::{SessionMemory, SessionMemoryRequest, SessionMemoryRun, SessionMemoryStatus},
    message::{Block, Message},
    session::{Record, RunConfig, RunStats, Session, SessionKind, SessionMeta},
};
use serde_json::Value;
use std::{collections::BTreeSet, fs, path::PathBuf};

const PROMPT: &str = "Tidy the fixture directory";

struct Fixture {
    root: PathBuf,
    request: SessionMemoryRequest,
    workspace: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("ordinary-tolerance-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        for p in ["db", "keys", "work", "sessions"] {
            mecha_core::create_private_dir(&root.join(p)).unwrap();
        }
        let request = SessionMemoryRequest {
            protocol: "ordinary-session-memory-v1".into(),
            db: root.join("db/test.memory-prototype.sqlite"),
            key_file: root.join("keys/key"),
            authority: root.join("authority"),
            domain: "fixture".into(),
            account: "fixture".into(),
            scopes: BTreeSet::new(),
        };
        let workspace = root.join("work");
        SessionMemory::initialize(&request).unwrap();
        Self {
            root,
            request,
            workspace,
        }
    }
    /// A fresh run over a tool surface no closed list names.
    fn begin(&self) -> (SessionMemory, Session, SessionMemoryRun) {
        let memory = SessionMemory::open(&self.request, &self.workspace).unwrap();
        let meta = SessionMeta {
            id: Session::new_id(),
            created_at: Utc::now(),
            provider: "fixture".into(),
            model: "fixture".into(),
            workspace: self.workspace.clone(),
            title: None,
            kind: Some(SessionKind::Run),
        };
        let session = Session {
            path: self
                .root
                .join("sessions")
                .join(format!("{}.jsonl", meta.id)),
            meta: meta.clone(),
        };
        session.append(&Record::Meta(meta)).unwrap();
        session
            .append(&Record::Config(RunConfig {
                workspace: self.workspace.clone(),
                tools: vec![
                    "goal_context".into(),
                    "notes__search".into(),
                    "shell".into(),
                    "subagent_research".into(),
                ],
                ..Default::default()
            }))
            .unwrap();
        session
            .append(&Record::GoalAnchor {
                goal: Some("charter:1".parse().unwrap()),
            })
            .unwrap();
        let run = memory
            .begin(&session, PROMPT, &"charter:1".parse().unwrap())
            .unwrap();
        session
            .append(&Record::Message(Message::user(PROMPT)))
            .unwrap();
        (memory, session, run)
    }
    /// The sealed capture, as the authority directory holds it.
    fn sealed(&self, run: &SessionMemoryRun) -> Value {
        let path = self
            .request
            .authority
            .join(format!("{}.seal", run.run_id()));
        serde_json::from_slice::<Value>(&fs::read(path).unwrap()).unwrap()["payload"].clone()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn call(session: &Session, id: &str, tool: &str, result: Option<(&str, bool)>) {
    session
        .append(&Record::Message(Message::assistant(vec![Block::ToolUse {
            id: id.into(),
            name: tool.into(),
            input: serde_json::json!({ "argument": id }),
        }])))
        .unwrap();
    if let Some((content, external)) = result {
        let mut message = Message::user("");
        message.content = vec![Block::ToolResult {
            tool_use_id: id.into(),
            content: content.into(),
            is_error: false,
        }];
        message.tool_provenance.insert(id.into(), external);
        session.append(&Record::Message(message)).unwrap();
    }
}

fn say(session: &Session, text: &str) {
    session
        .append(&Record::Message(Message::assistant(vec![Block::text(
            text,
        )])))
        .unwrap();
}

fn end(session: &Session, untrusted: bool, stop: StopCause) {
    session
        .append(&Record::Taint(Taint {
            private: false,
            untrusted,
        }))
        .unwrap();
    session
        .append(&Record::Outcome(RunStats {
            stop_cause: Some(stop),
            compactions: 1,
            ..Default::default()
        }))
        .unwrap();
}

fn passages(payload: &Value) -> &Vec<Value> {
    payload["originals"].as_array().unwrap()
}

fn of_kind<'a>(payload: &'a Value, kind: &str) -> Vec<&'a Value> {
    passages(payload)
        .iter()
        .filter(|p| p["kind"] == kind)
        .collect()
}

#[test]
fn a_run_over_any_tools_with_many_calls_and_a_long_result_is_remembered() {
    let f = Fixture::new();
    let (memory, session, run) = f.begin();
    // Forty calls is five times what one capture used to admit, and none of
    // these tools was on the recorder's list.
    let long = "é".repeat(6000);
    for index in 0..40 {
        let tool = ["shell", "notes__search", "subagent_research"][index % 3];
        let content = if index == 7 {
            long.as_str()
        } else {
            "a local result"
        };
        call(
            &session,
            &format!("call-{index}"),
            tool,
            Some((content, false)),
        );
    }
    say(&session, "The directory is tidy.");
    end(&session, false, StopCause::Completed);

    let report = memory.finish(&run);
    assert_eq!(report.status, SessionMemoryStatus::Retained, "{report:?}");
    let payload = f.sealed(&run);
    assert_eq!(passages(&payload).len(), 1 + 40 * 2 + 1);
    assert_eq!(payload["receipts"].as_array().unwrap().len(), 40);
    // Everything in a run that took in nothing from outside is stored local.
    assert!(passages(&payload).iter().all(|p| p["external"] == false));

    // The long result is kept as its head, cut between characters, and says
    // how long the whole field was.
    let excerpts: Vec<_> = passages(&payload)
        .iter()
        .filter(|p| !p["source_bytes"].is_null())
        .collect();
    assert_eq!(excerpts.len(), 1);
    let text = excerpts[0]["text"].as_str().unwrap();
    assert_eq!(excerpts[0]["source_bytes"], long.len() as u64);
    assert!(text.len() <= 4096 && long.starts_with(text) && text.len() > 4000);
}

#[test]
fn what_the_recorder_cannot_place_is_left_out_and_the_recorded_taint_decides() {
    // The same transcript three ways. `stray` puts content in the run that
    // the recorder has no passage for; `untrusted` is what the harness
    // recorded about the whole conversation.
    let record = |stray: bool, untrusted: bool| {
        let f = Fixture::new();
        let (memory, session, run) = f.begin();
        say(&session, "I will look first.");
        call(&session, "look", "shell", Some(("two files", false)));
        if stray {
            // User-role text that is neither the owner's prompt nor a notice
            // the recorder knows byte for byte, as a delivered inter-agent
            // message is.
            let mut delivered = Message::user("");
            delivered.content = vec![Block::text("a message from another session")];
            session.append(&Record::Message(delivered)).unwrap();
            // History rewritten by a compaction, and text folded into a
            // message already on file.
            session
                .append(&Record::Rewrite { messages: vec![] })
                .unwrap();
            session
                .append(&Record::Extend {
                    index: 0,
                    blocks: vec![Block::text("a line the harness folded in")],
                })
                .unwrap();
        }
        // A blank reply, and a call the run stopped before answering.
        say(&session, "   ");
        call(&session, "unanswered", "notes__search", None);
        end(&session, untrusted, StopCause::Interrupted);
        let report = memory.finish(&run);
        assert_eq!(report.status, SessionMemoryStatus::Retained, "{report:?}");
        f.sealed(&run)
    };

    for (stray, untrusted) in [(true, false), (true, true), (false, true)] {
        let payload = record(stray, untrusted);
        let texts: Vec<_> = passages(&payload)
            .iter()
            .map(|p| p["text"].as_str().unwrap())
            .collect();
        // Never a passage: the stray text, the folded line, the blank reply.
        assert!(
            texts.iter().all(|t| !t.contains("another session")
                && !t.contains("folded")
                && !t.trim().is_empty()),
            "{texts:?}"
        );
        assert_eq!(of_kind(&payload, "UserRequest").len(), 1);
        assert_eq!(of_kind(&payload, "ActionInvocation").len(), 2);
        // The unanswered call keeps its invocation, and its receipt says no
        // result is available.
        let receipts = payload["receipts"].as_array().unwrap();
        assert!(receipts
            .iter()
            .any(|r| r["call_id"] == "unanswered" && r["status"] == "Unavailable"));
        assert!(receipts
            .iter()
            .any(|r| r["call_id"] == "look" && r["status"] == "Succeeded"));

        let authored: Vec<_> = of_kind(&payload, "AssistantText")
            .into_iter()
            .chain(of_kind(&payload, "ActionInvocation"))
            .map(|p| p["external"].clone())
            .collect();
        let owner = of_kind(&payload, "UserRequest")[0]["external"].clone();
        match (stray, untrusted) {
            // A taint recorded clean says nothing outside entered by any
            // route, the stray content included.
            (true, false) => {
                assert!(authored.iter().all(|e| *e == false), "{authored:?}");
                assert_eq!(owner, false);
            }
            // Recorded untrusted, and content the walk could not place: what
            // the model wrote is outside, from its first word, and the
            // owner's own words stay the owner's.
            (true, true) => {
                assert!(authored.iter().all(|e| *e == true), "{authored:?}");
                assert_eq!(owner, false);
            }
            // Recorded untrusted with nothing in the walk to account for it:
            // no passage of the capture is sealed as local.
            (false, true) => {
                assert!(passages(&payload).iter().all(|p| p["external"].is_null()));
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn a_late_result_from_outside_marks_what_the_model_wrote_after_it() {
    let f = Fixture::new();
    let (memory, session, run) = f.begin();
    say(&session, "Starting the job.");
    call(&session, "job", "shell", Some(("started", false)));
    session
        .append(&Record::LateResult {
            index: 2,
            tool_use_id: "job".into(),
            content: "a page fetched from the network".into(),
            is_error: false,
            external: true,
        })
        .unwrap();
    say(&session, "The job found a page.");
    end(&session, true, StopCause::Completed);
    assert_eq!(memory.finish(&run).status, SessionMemoryStatus::Retained);
    let payload = f.sealed(&run);
    let texts = of_kind(&payload, "AssistantText");
    assert_eq!(texts[0]["text"], "Starting the job.");
    assert_eq!(texts[0]["external"], false);
    assert_eq!(texts[1]["text"], "The job found a page.");
    assert_eq!(texts[1]["external"], true);
}

#[test]
fn a_run_too_large_for_one_capture_keeps_its_request_its_answer_and_whole_calls() {
    let f = Fixture::new();
    let (memory, session, run) = f.begin();
    for index in 0..300 {
        call(
            &session,
            &format!("call-{index}"),
            "shell",
            Some(("a local result", false)),
        );
    }
    say(&session, "All three hundred are done.");
    end(&session, false, StopCause::Completed);
    let report = memory.finish(&run);
    assert_eq!(report.status, SessionMemoryStatus::Retained, "{report:?}");

    let payload = f.sealed(&run);
    let all = passages(&payload);
    assert_eq!(all.len(), 256);
    assert_eq!(all[0]["kind"], "UserRequest");
    assert_eq!(all[255]["text"], "All three hundred are done.");
    // Cut on whole calls: every result has its invocation before it, every
    // receipt its invocation, and the run is kept from its start.
    let invoked: Vec<String> = of_kind(&payload, "ActionInvocation")
        .iter()
        .map(|p| {
            serde_json::from_str::<Value>(p["text"].as_str().unwrap()).unwrap()["call_id"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(invoked.len(), 127);
    assert_eq!(of_kind(&payload, "ToolResult").len(), 127);
    assert!(invoked
        .iter()
        .enumerate()
        .all(|(index, id)| *id == format!("call-{index}")));
    let receipts: Vec<_> = payload["receipts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["call_id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(receipts, invoked);
}

fn grant(path: &std::path::Path) -> mecha_core::grant::Grant {
    mecha_core::grant::Grant {
        path: path.canonicalize().unwrap(),
        access: mecha_core::grant::Access::Read,
        source: mecha_core::grant::Source::Owner,
    }
}

#[test]
fn a_grant_may_not_reach_the_store_its_key_or_the_transcript() {
    let names = ["goal_context".to_string(), "shell".to_string()];
    let f = Fixture::new();
    let elsewhere = f.root.join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    let memory = SessionMemory::open(&f.request, &f.workspace).unwrap();
    // A grant somewhere else is the owner's business.
    memory
        .validate_surface(&names, &f.workspace, &[grant(&elsewhere)])
        .unwrap();
    // One over the database, the key, the authority directory, or a
    // directory holding all of them is refused before the run starts.
    for reach in [
        f.root.join("db"),
        f.root.join("keys"),
        f.request.authority.clone(),
        f.root.clone(),
    ] {
        assert!(
            memory
                .validate_surface(&names, &f.workspace, &[grant(&reach)])
                .is_err(),
            "{reach:?}"
        );
    }

    // Once the session exists, the same rule covers its transcript: a
    // session directory may sit outside the mecha home.
    let (_, session, _) = f.begin();
    memory
        .validate_session_grants(&session, &[grant(&elsewhere)])
        .unwrap();
    assert!(memory
        .validate_session_grants(&session, &[grant(&f.root.join("sessions"))])
        .is_err());

    // A grant made during the run is in the transcript. One elsewhere costs
    // nothing; one over the transcript's own directory, or over the key,
    // refuses the capture.
    for (reach, status) in [
        (elsewhere.clone(), SessionMemoryStatus::Retained),
        (f.root.join("sessions"), SessionMemoryStatus::Unavailable),
        (f.root.join("keys"), SessionMemoryStatus::Unavailable),
    ] {
        let (memory, session, run) = f.begin();
        session
            .append(&Record::Grants {
                grants: vec![grant(&reach)],
            })
            .unwrap();
        say(&session, "Done.");
        end(&session, false, StopCause::Completed);
        assert_eq!(memory.finish(&run).status, status, "{reach:?}");
    }
}

#[test]
fn a_run_is_cut_by_bytes_as_well_as_by_count_and_still_admitted() {
    let f = Fixture::new();
    let (memory, session, run) = f.begin();
    // A quote doubles when it is serialized, so each result costs about
    // 8 KiB of the capture: the byte bound is reached before the count.
    let quoted = "\"".repeat(4096);
    for index in 0..120 {
        call(
            &session,
            &format!("call-{index}"),
            "shell",
            Some((&quoted, false)),
        );
    }
    say(&session, "All of them are done.");
    end(&session, false, StopCause::Completed);
    let report = memory.finish(&run);
    assert_eq!(report.status, SessionMemoryStatus::Retained, "{report:?}");
    let payload = f.sealed(&run);
    let all = passages(&payload);
    assert!(all.len() < 2 + 120 * 2, "{}", all.len());
    assert!(serde_json::to_vec(&payload).unwrap().len() <= 1 << 20);
    assert_eq!(all[0]["kind"], "UserRequest");
    assert_eq!(all[all.len() - 1]["text"], "All of them are done.");
    // The call at the cut may keep its invocation without its result; no
    // result is kept without its invocation.
    let invoked = of_kind(&payload, "ActionInvocation").len();
    let answered = of_kind(&payload, "ToolResult").len();
    assert!(answered <= invoked && invoked <= answered + 1);
    assert_eq!(payload["receipts"].as_array().unwrap().len(), invoked);
}

#[test]
fn a_prompt_that_opens_with_blank_space_still_has_a_criterion() {
    let f = Fixture::new();
    let memory = SessionMemory::open(&f.request, &f.workspace).unwrap();
    let prompt = format!("{}Tidy the fixture directory", " ".repeat(3000));
    let meta = SessionMeta {
        id: Session::new_id(),
        created_at: Utc::now(),
        provider: "fixture".into(),
        model: "fixture".into(),
        workspace: f.workspace.clone(),
        title: None,
        kind: Some(SessionKind::Run),
    };
    let session = Session {
        path: f.root.join("sessions").join(format!("{}.jsonl", meta.id)),
        meta: meta.clone(),
    };
    session.append(&Record::Meta(meta)).unwrap();
    session
        .append(&Record::Config(RunConfig {
            workspace: f.workspace.clone(),
            tools: vec!["goal_context".into()],
            ..Default::default()
        }))
        .unwrap();
    session
        .append(&Record::GoalAnchor {
            goal: Some("charter:1".parse().unwrap()),
        })
        .unwrap();
    let run = memory
        .begin(&session, &prompt, &"charter:1".parse().unwrap())
        .unwrap();
    session
        .append(&Record::Message(Message::user(prompt.as_str())))
        .unwrap();
    say(&session, "Done.");
    end(&session, false, StopCause::Completed);
    let report = memory.finish(&run);
    assert_eq!(report.status, SessionMemoryStatus::Retained, "{report:?}");
    assert_eq!(
        f.sealed(&run)["criterion"]["criterion_text"],
        "Tidy the fixture directory"
    );
}

#[test]
fn the_learning_inventory_pages_through_a_run_larger_than_one_page() {
    let f = Fixture::new();
    let (memory, session, run) = f.begin();
    // Results near a passage's full size: sixteen of them are twice what
    // one page of descriptors may hold, so a page must also shrink to fit.
    let large = "r".repeat(4000);
    for index in 0..20 {
        call(
            &session,
            &format!("call-{index}"),
            "shell",
            Some((&large, false)),
        );
    }
    say(&session, "Done.");
    end(&session, false, StopCause::Completed);
    assert_eq!(memory.finish(&run).status, SessionMemoryStatus::Retained);
    let total = passages(&f.sealed(&run)).len();
    assert_eq!(total, 1 + 20 * 2 + 1);

    let mut seen = vec![];
    let mut pages = 0;
    loop {
        let page = memory
            .learning_run_sources(run.run_id(), seen.len())
            .unwrap();
        if page.is_empty() {
            break;
        }
        assert!(page.len() <= 16);
        pages += 1;
        for descriptor in &page {
            seen.push(
                serde_json::to_value(descriptor).unwrap()["source"]["original_index"].clone(),
            );
        }
    }
    // Every original once, in order, and more than one page to do it.
    assert_eq!(seen, (0..total).map(Value::from).collect::<Vec<_>>());
    assert!(pages > 3, "{pages}");
    assert!(memory
        .learning_run_sources(run.run_id(), total + 5)
        .unwrap()
        .is_empty());
}
