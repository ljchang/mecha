use super::*;
use crate::message::Message;
use crate::session::{Record, SessionKind};
use std::cell::RefCell;

const GONE: &str = "20260928T120000-deadbeef";
const KEPT: &str = "20260928T130000-cafef00d";
/// A phrase only the forgotten conversation ever said. Every copy of it — a
/// lesson, a log line, an appraisal's quote — is a trace.
const CANARY: &str = "the lighthouse keeper's violet umbrella";

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(name: &str) -> Scratch {
    let dir = std::env::temp_dir().join(format!("mecha-forget-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Scratch(dir)
}

struct Graph {
    answer: RefCell<Vec<Result<GraphOutcome>>>,
    asked: RefCell<Vec<String>>,
}

impl Graph {
    fn answering(answers: Vec<Result<GraphOutcome>>) -> Self {
        Graph {
            answer: RefCell::new(answers),
            asked: RefCell::new(Vec::new()),
        }
    }
}

impl GraphRedactor for Graph {
    fn redact_session(&self, id: &str) -> Result<GraphOutcome> {
        self.asked.borrow_mut().push(id.to_string());
        self.answer.borrow_mut().remove(0)
    }
}

fn session(roots: &Roots, id: &str, workspace: &Path, said: &str) -> Session {
    std::fs::create_dir_all(workspace).unwrap();
    let s = Session::create(
        &roots.sessions,
        SessionMeta {
            id: id.into(),
            created_at: chrono::Utc::now(),
            provider: "scripted".into(),
            model: "m".into(),
            workspace: workspace.to_path_buf(),
            title: Some(format!("web: {said}")),
            kind: Some(SessionKind::Web),
        },
    )
    .unwrap();
    s.append(&Record::Message(Message::user(said))).unwrap();
    s
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Every file under `dir` whose bytes contain `needle`.
fn holding(dir: &Path, needle: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p
                .file_name()
                .is_some_and(|n| n.to_string_lossy().contains(needle))
                || std::fs::read(&p).is_ok_and(|b| String::from_utf8_lossy(&b).contains(needle))
            {
                // A name is a copy too: `.agents/<id>.json`, `.archived/<id>`.
                out.push(p);
            }
        }
    }
    out
}

/// A home with every store holding one row from the conversation being
/// forgotten and one from a conversation that is not.
fn seeded(home: &Path) -> Roots {
    let roots = Roots::under(home);
    let gone_ws = home.join("work/web/chat-gone");
    let kept_ws = home.join("work/web/chat-kept");
    session(&roots, GONE, &gone_ws, CANARY);
    session(&roots, KEPT, &kept_ws, "an ordinary afternoon");
    write(&gone_ws.join("inbox/photo.txt"), CANARY);
    let spill = crate::tool::session_spill_dir_under(home, &gone_ws);
    write(&spill.join("out-1.txt"), CANARY);

    let o = &roots.outbox;
    write(
        &o.join("item-gone.json"),
        &format!(r#"{{"id":"item-gone","session_id":"{GONE}","args":{{"body":"{CANARY}"}}}}"#),
    );
    write(
        &o.join("item-kept.json"),
        &format!(r#"{{"id":"item-kept","session_id":"{KEPT}","args":{{}}}}"#),
    );
    write(
        &roots.questions.join("q1.json"),
        &format!(r#"{{"id":"q1","session_id":"{GONE}","question":"{CANARY}?"}}"#),
    );
    write(
        &roots.questions.join("q2.json"),
        &format!(r#"{{"id":"q2","session_id":"{KEPT}","question":"ok?"}}"#),
    );
    write(
        &roots.messages.join("hermes/m1.json"),
        &format!(r#"{{"id":"m1","from_session":"{GONE}","body":"{CANARY}"}}"#),
    );
    write(
        &roots.messages.join("hermes/m2.json"),
        &format!(r#"{{"id":"m2","from_session":"{KEPT}","body":"hi"}}"#),
    );
    // Received, not sent: `claim_pending` stamps the claiming session.
    write(
        &roots.messages.join("hermes/m3.json"),
        &format!(r#"{{"id":"m3","from_session":"{KEPT}","delivered_to":"{GONE}","body":"hello"}}"#),
    );
    write(&roots.messages.join(format!(".agents/{GONE}.json")), "{}");

    let l = &roots.learning;
    write(&l.join("reflections.jsonl"), &format!(
        "{{\"id\":\"refl-gone\",\"session_id\":\"{GONE}\",\"reflexion_text\":\"Always remember {CANARY}.\"}}\n\
         {{\"id\":\"refl-kept\",\"session_id\":\"{KEPT}\",\"reflexion_text\":\"Be brief.\",\"a_field_from_later\":1}}\n"
    ));
    write(&l.join("mined.jsonl"), &format!("{GONE}\n{KEPT}\n"));
    write(&l.join("distilled.jsonl"), &format!("{GONE}\n{KEPT}\n"));
    write(&l.join("mined_outbox.jsonl"), "item-gone\nitem-kept\n");
    write(
        &l.join("validations.jsonl"),
        "{\"reflexion_id\":\"refl-gone\"}\n{\"reflexion_id\":\"refl-kept\"}\n",
    );
    write(&l.join("validation-attempts.jsonl"), &format!(
        "{{\"reflexion_id\":\"refl-gone\",\"arms\":[{{\"text\":\"{CANARY}\"}}]}}\n{{\"reflexion_id\":\"refl-kept\",\"arms\":[]}}\n"
    ));
    write(
        &l.join("proposals/p-only.json"),
        &format!(
            r#"{{"id":"p-only","reflexion_ids":["refl-gone"],"rules":[{{"text":"Mind {CANARY}."}}]}}"#
        ),
    );
    // The realistic shape: a proposal *about* the learned set carries whole
    // rule values, so a removed rule's text sits in `rules` and
    // `rules_before` — and in a proposal that never argued from the
    // forgotten reflections at all.
    write(
        &l.join("proposals/p-both.json"),
        &format!(
            r#"{{"id":"p-both","reflexion_ids":["refl-gone","refl-kept"],"rules":[{{"text":"Mind {CANARY}.","sources":["refl-gone"]}},{{"text":"Keep answers short.","sources":["refl-gone","refl-kept"]}}],"rules_before":[{{"text":"Mind {CANARY}.","sources":["refl-gone"]}}]}}"#
        ),
    );
    write(
        &l.join("proposals/p-other.json"),
        &format!(
            r#"{{"id":"p-other","reflexion_ids":["refl-kept"],"rules":[],"rules_before":[{{"text":"Mind {CANARY}.","sources":["refl-gone"]}}]}}"#
        ),
    );
    write(&l.join("rules/behavior.learned.toml"), &format!(
        "[[rules]]\ntext = \"Mind {CANARY}.\"\nid = \"r-only\"\nsources = [\"refl-gone\"]\n\n\
         [[rules]]\ntext = \"Keep answers short.\"\nid = \"r-both\"\nsources = [\"refl-gone\", \"refl-kept\"]\n\n\
         [[rules]]\ntext = \"Predates lineage.\"\nid = \"r-old\"\n"
    ));
    write(
        &l.join("rules/behavior.user.toml"),
        "[[rules]]\ntext = \"The owner's own.\"\n",
    );
    write(
        &l.join("logs/nightly.log"),
        &format!(
        "distill: · {GONE} → ep-1\n  · [steer] Always remember {CANARY}.\nreflect: 2 session(s)\n"
    ),
    );
    write(
        &roots.harness.join("candidates/c1.json"),
        &format!(
            r#"{{"id":"c1","measurement":{{"episodes":["{GONE}","{KEPT}"],"holdout_episodes":["{GONE}"]}}}}"#
        ),
    );

    let a = &roots.appraisals;
    write(&a.join("appraisals.jsonl"), &format!(
        "{{\"session_id\":\"{GONE}\",\"interpretation\":\"{CANARY}\"}}\n{{\"session_id\":\"{KEPT}\"}}\n"
    ));
    write(
        &a.join("scores.jsonl"),
        &format!("{{\"session_id\":\"{GONE}\"}}\n{{\"session_id\":\"{KEPT}\"}}\n"),
    );
    write(
        &a.join("counterfactuals.jsonl"),
        &format!("{{\"session_id\":\"{GONE}\",\"quote\":\"{CANARY}\"}}\n"),
    );
    write(&roots.comparisons.join("comparisons.jsonl"), &format!(
        "{{\"id\":\"c-a\",\"pointers\":{{\"session_id\":\"{GONE}\"}}}}\n\
         {{\"id\":\"c-b\",\"pointers\":{{\"session_id\":\"{KEPT}\",\"reflection_id\":\"refl-gone\"}}}}\n\
         {{\"id\":\"c-c\",\"pointers\":{{\"session_id\":\"{KEPT}\"}}}}\n"
    ));
    write(
        &roots.closures.join("closures.jsonl"),
        &format!("{{\"task\":\"t1\",\"sessions\":[\"{GONE}\",\"{KEPT}\"],\"reason\":\"done\"}}\n"),
    );
    write(&roots.triggers.join("runs.jsonl"), &format!(
        "{{\"trigger\":\"briefing\",\"slot\":\"2026-09-28T07:00:00Z\",\"session_id\":\"{GONE}\",\"summary\":\"{CANARY}\"}}\n{{\"trigger\":\"briefing\",\"session_id\":\"{KEPT}\"}}\n"
    ));
    write(
        &roots.workflows.join("w1.json"),
        &format!(
            r#"{{"id":"w1","title":"Weekly","session_id":"{GONE}","outbox":["item-gone","item-kept"],"questions":["q1","q2"],"events":[{{"at":"2026-09-28T12:00:00Z","kind":"started","detail":"{GONE}"}},{{"at":"2026-09-28T12:05:00Z","kind":"note","detail":"kept"}}],"verify_history":[{{"session":"{GONE}"}}]}}"#
        ),
    );
    write(
        &home.join("slack/threads/C1-1.json"),
        &format!(r#"{{"session_id":"{GONE}"}}"#),
    );
    write(
        &home.join("regression-sessions.txt"),
        &format!("{GONE}\n{KEPT}\n"),
    );
    // A stranger's request the forgotten conversation triaged: the request
    // stays, un-pointed, and loses its link to the draft that was removed.
    write(
        &roots.requests.join("0000000009-meeting.json"),
        &format!(
            r#"{{"id":9,"kind":"meeting","triage_session":"{GONE}","outbox":["item-gone","item-kept"]}}"#
        ),
    );
    // A mail thread whose reply the forgotten conversation drafted: the
    // thread is the owner's and stays; the drafting pointer goes.
    write(
        &roots.triage.join("acct-thread1.json"),
        &format!(r#"{{"thread":"thread1","verdict":"reply","draft_session":"{GONE}"}}"#),
    );
    crate::archive::archive(&roots.sessions, GONE, chrono::Utc::now()).unwrap();
    roots
}

#[test]
fn forgetting_leaves_no_trace_in_any_store_and_touches_nothing_else() {
    let home = scratch("everywhere");
    let roots = seeded(&home.0);
    let kept_before =
        std::fs::read_to_string(roots.sessions.join(format!("{KEPT}.jsonl"))).unwrap();
    let graph = Graph::answering(vec![Ok(GraphOutcome::Redacted(1))]);
    // Not vacuous: the fixture really does spread the session everywhere.
    let seeded_ids = holding(&home.0, GONE).len();
    let seeded_text = holding(&home.0, CANARY).len();
    assert!(
        seeded_ids >= 18 && seeded_text >= 12,
        "{seeded_ids} / {seeded_text}"
    );

    let report = forget(&roots, GONE, &graph).unwrap();

    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.complete);
    assert_eq!(*graph.asked.borrow(), vec![GONE.to_string()]);
    // The whole claim, asked of the bytes: nothing under the home names the
    // session or says what it said.
    assert_eq!(holding(&home.0, GONE), Vec::<PathBuf>::new());
    assert_eq!(holding(&home.0, CANARY), Vec::<PathBuf>::new());
    assert_eq!(holding(&home.0, "refl-gone"), Vec::<PathBuf>::new());
    assert!(!home.0.join("work/web/chat-gone").exists());

    // And everything that was not the session's is still there.
    assert_eq!(
        std::fs::read_to_string(roots.sessions.join(format!("{KEPT}.jsonl"))).unwrap(),
        kept_before
    );
    let reflections = std::fs::read_to_string(roots.learning.join("reflections.jsonl")).unwrap();
    assert!(
        reflections.contains("\"a_field_from_later\":1"),
        "a kept row lost a field this binary cannot name: {reflections}"
    );
    for (path, needle) in [
        ("outbox/item-kept.json", KEPT),
        ("questions/q2.json", KEPT),
        ("messages/hermes/m2.json", KEPT),
        ("learning/mined.jsonl", KEPT),
        ("learning/distilled.jsonl", KEPT),
        ("learning/mined_outbox.jsonl", "item-kept"),
        ("learning/validations.jsonl", "refl-kept"),
        ("learning/validation-attempts.jsonl", "refl-kept"),
        ("learning/proposals/p-both.json", "Keep answers short."),
        ("learning/proposals/p-other.json", "refl-kept"),
        ("learning/rules/behavior.user.toml", "The owner's own."),
        ("learning/logs/nightly.log", "reflect: 2 session(s)"),
        ("learning/harness/candidates/c1.json", KEPT),
        ("appraisals/appraisals.jsonl", KEPT),
        ("appraisals/scores.jsonl", KEPT),
        ("comparisons/comparisons.jsonl", "c-c"),
        ("closures/closures.jsonl", KEPT),
        ("triggers/runs.jsonl", KEPT),
        // The run row is the schedule marker; losing it re-fires the slot.
        ("triggers/runs.jsonl", "2026-09-28T07:00:00Z"),
        ("workflows/w1.json", "item-kept"),
        ("workflows/w1.json", "q2"),
        ("workflows/w1.json", "Weekly"),
        ("regression-sessions.txt", KEPT),
        ("requests/0000000009-meeting.json", "item-kept"),
        ("mail-triage/acct-thread1.json", "reply"),
    ] {
        let text = std::fs::read_to_string(home.0.join(path)).unwrap();
        assert!(
            text.contains(needle),
            "{path} lost what was not the session's: {text}"
        );
    }
    assert!(!roots.learning.join("proposals/p-only.json").exists());
    assert!(home.0.join("work/web/chat-kept").exists());

    // A rule the forgotten reflections alone argued for is gone — not
    // retired, which would quote it to the learner forever; one with other
    // support keeps it; one from before lineage is untouched.
    let rules = crate::learning::LearningStore::open(&roots.learning)
        .unwrap()
        .learned_rules("behavior")
        .unwrap();
    let ids: Vec<_> = rules.iter().map(|r| r.id.clone().unwrap()).collect();
    assert_eq!(ids, ["r-both", "r-old"]);
    assert_eq!(rules[0].sources, ["refl-kept"]);
    assert!(rules.iter().all(|r| r.retired_at.is_none()));
}

#[test]
fn a_store_that_fails_keeps_the_transcript_to_finish_next_time() {
    let home = scratch("retry");
    let roots = seeded(&home.0);
    let graph = Graph::answering(vec![
        Err(anyhow::anyhow!("mecha-graph: not found")),
        Ok(GraphOutcome::Redacted(1)),
    ]);

    let first = forget(&roots, GONE, &graph).unwrap();
    assert!(!first.complete);
    assert!(first.errors.iter().any(|e| e.starts_with("graph:")));
    assert!(
        !first.residue.iter().any(|r| r.contains("distilled.jsonl")),
        "the ledger line kept for the retry is not a field delete does not know"
    );
    // Set aside, not deleted: no listing sees it, and the retry still has it.
    let parked = roots.sessions.join(format!("{GONE}.jsonl.forgetting"));
    assert!(parked.exists());
    assert!(Session::list(&roots.sessions)
        .unwrap()
        .iter()
        .all(|(m, _)| m.id != GONE));
    // The draft is kept for the retry, so its "already mined" mark must be
    // too — or tonight's reflect mines the forgotten draft into a new lesson.
    assert!(roots.outbox.join("item-gone.json").exists());
    let mined = std::fs::read_to_string(roots.learning.join("mined_outbox.jsonl")).unwrap();
    assert!(
        mined.contains("item-gone"),
        "a kept draft lost its mined mark"
    );
    // The graph still owes the episode, so the ledger that says so stays.
    let distilled = std::fs::read_to_string(roots.learning.join("distilled.jsonl")).unwrap();
    assert!(distilled.contains(GONE));

    // Nothing lists it, so the doctor's reader must.
    assert_eq!(unfinished(&roots.sessions).unwrap(), [GONE]);

    let second = forget(&roots, GONE, &graph).unwrap();
    assert!(unfinished(&roots.sessions).unwrap().is_empty());
    assert!(second.complete, "{:?}", second.errors);
    assert!(!parked.exists());
    assert_eq!(holding(&home.0, GONE), Vec::<PathBuf>::new());
}

#[test]
fn a_distilled_session_with_no_graph_to_answer_is_not_forgotten() {
    // "There is no graph" is only an answer when nothing was ever sent to one.
    let home = scratch("absent");
    let roots = seeded(&home.0);
    let report = forget(
        &roots,
        GONE,
        &Graph::answering(vec![Ok(GraphOutcome::Absent)]),
    )
    .unwrap();
    assert!(!report.complete);
    assert!(report.errors.iter().any(|e| e.contains("distilled")));
}

#[test]
fn a_shared_workspace_is_kept_and_said_to_be() {
    let home = scratch("shared");
    let roots = Roots::under(&home.0);
    let main = home.0.join("work/web/main");
    session(&roots, GONE, &main, CANARY);
    session(&roots, KEPT, &main, "later, same key");
    write(&main.join("notes.txt"), "the other conversation's file");

    let report = forget(
        &roots,
        GONE,
        &Graph::answering(vec![Ok(GraphOutcome::Absent)]),
    )
    .unwrap();

    assert!(report.complete, "{:?}", report.errors);
    assert!(main.join("notes.txt").exists());
    assert!(report
        .residue
        .iter()
        .any(|r| r.contains("shared with 1 other")));
}

#[test]
fn a_workspace_outside_mecha_is_never_touched() {
    let home = scratch("project");
    let project = scratch("project-dir");
    let roots = Roots::under(&home.0);
    session(&roots, GONE, &project.0, CANARY);
    write(&project.0.join("src/main.rs"), "fn main() {}");

    let report = forget(
        &roots,
        GONE,
        &Graph::answering(vec![Ok(GraphOutcome::Absent)]),
    )
    .unwrap();

    assert!(report.complete);
    assert!(project.0.join("src/main.rs").exists());
    assert!(report.residue.iter().any(|r| r.contains("are yours")));
}

#[test]
fn a_writer_still_holding_a_forgotten_session_cannot_bring_it_back() {
    let home = scratch("writer");
    let roots = Roots::under(&home.0);
    let live = session(&roots, GONE, &home.0.join("work/web/chat-x"), CANARY);
    forget(
        &roots,
        GONE,
        &Graph::answering(vec![Ok(GraphOutcome::Absent)]),
    )
    .unwrap();

    assert!(live
        .append(&Record::Message(Message::user(CANARY)))
        .is_err());
    assert!(
        !live.path.exists(),
        "an append recreated a deleted transcript"
    );
}

#[test]
fn an_unknown_session_is_an_error_and_a_prefix_is_not_an_id() {
    let home = scratch("unknown");
    let roots = Roots::under(&home.0);
    session(&roots, GONE, &home.0.join("work/web/a"), CANARY);
    let graph = Graph::answering(vec![]);
    assert!(forget(&roots, "20260928T120000", &graph).is_err());
    assert!(forget(&roots, "../sessions", &graph).is_err());
    assert!(roots.sessions.join(format!("{GONE}.jsonl")).exists());
}

#[test]
fn a_store_config_relocates_is_the_one_purged() {
    // `[outbox] dir` / `[messages] dir`: asking only the environment would
    // purge a default that does not exist and call the store clean.
    let mut cfg = crate::config::Config::default();
    cfg.outbox.dir = Some(PathBuf::from("/data/outbox"));
    cfg.messages.dir = Some(PathBuf::from("/data/messages"));
    let roots = Roots::from_config(&cfg).unwrap();
    assert_eq!(roots.outbox, PathBuf::from("/data/outbox"));
    assert_eq!(roots.messages, PathBuf::from("/data/messages"));
    let plain = Roots::from_config(&crate::config::Config::default()).unwrap();
    assert_eq!(plain.outbox, Roots::from_env().unwrap().outbox);
}

#[test]
fn a_trace_in_a_field_delete_does_not_know_is_reported_not_hidden() {
    let home = scratch("unknown-field");
    let roots = Roots::under(&home.0);
    session(&roots, GONE, &home.0.join("work/web/a"), CANARY);
    // A store row that names the session somewhere no purge looks.
    write(
        &roots.questions.join("q9.json"),
        &format!(r#"{{"id":"q9","session_id":"{KEPT}","note":"see {GONE}"}}"#),
    );
    let report = forget(
        &roots,
        GONE,
        &Graph::answering(vec![Ok(GraphOutcome::Absent)]),
    )
    .unwrap();
    assert!(report.complete);
    assert!(
        report
            .residue
            .iter()
            .any(|r| r.contains("q9.json") && r.contains("does not know")),
        "{:?}",
        report.residue
    );
}

#[test]
fn a_learning_failure_leaves_the_keys_so_the_retry_still_finds_their_rows() {
    // A rule file this binary cannot parse stops the learning purge part-way.
    // The reflections are the key every rule and ledger is found by; had they
    // gone first, the retry would find nothing by them and report clean.
    let home = scratch("learning-retry");
    let roots = seeded(&home.0);
    let rules = roots.learning.join("rules/behavior.learned.toml");
    let good = std::fs::read_to_string(&rules).unwrap();
    std::fs::write(&rules, "[[rules]\nthis is not toml").unwrap();
    let graph = Graph::answering(vec![
        Ok(GraphOutcome::Redacted(0)),
        Ok(GraphOutcome::Redacted(0)),
    ]);

    let first = forget(&roots, GONE, &graph).unwrap();
    assert!(!first.complete);
    assert!(first.errors.iter().any(|e| e.starts_with("learning store")));
    let reflections = std::fs::read_to_string(roots.learning.join("reflections.jsonl")).unwrap();
    assert!(
        reflections.contains("refl-gone"),
        "the key went before its rows"
    );
    assert!(
        roots.outbox.join("item-gone.json").exists(),
        "an outbox key went early"
    );

    std::fs::write(&rules, good).unwrap();
    let second = forget(&roots, GONE, &graph).unwrap();
    assert!(second.complete, "{:?}", second.errors);
    assert_eq!(holding(&home.0, "refl-gone"), Vec::<PathBuf>::new());
    assert_eq!(holding(&home.0, "item-gone"), Vec::<PathBuf>::new());
}

#[test]
fn a_peer_conversation_that_received_a_message_is_named_not_edited() {
    // A message this conversation sent was delivered into the recipient's
    // transcript, id and text both. That transcript is the peer's, so the
    // delete names it rather than rewriting someone else's conversation.
    let home = scratch("peer");
    let roots = Roots::under(&home.0);
    session(&roots, GONE, &home.0.join("work/web/a"), CANARY);
    session(
        &roots,
        KEPT,
        &home.0.join("work/web/b"),
        &format!("message from hermes (session {GONE}): {CANARY}"),
    );
    let report = forget(
        &roots,
        GONE,
        &Graph::answering(vec![Ok(GraphOutcome::Absent)]),
    )
    .unwrap();
    assert!(report.complete);
    let peer = roots.sessions.join(format!("{KEPT}.jsonl"));
    assert!(
        std::fs::read_to_string(&peer).unwrap().contains(CANARY),
        "a peer's words were edited"
    );
    assert!(
        report
            .residue
            .iter()
            .any(|r| r.contains(&format!("{KEPT}.jsonl"))),
        "{:?}",
        report.residue
    );
}

#[test]
fn an_unreadable_header_says_its_workspace_was_not_found() {
    let home = scratch("headless");
    let roots = Roots::under(&home.0);
    std::fs::create_dir_all(&roots.sessions).unwrap();
    std::fs::write(
        roots.sessions.join(format!("{GONE}.jsonl")),
        "not a header\n",
    )
    .unwrap();
    let report = forget(
        &roots,
        GONE,
        &Graph::answering(vec![Ok(GraphOutcome::Absent)]),
    )
    .unwrap();
    assert!(
        report
            .residue
            .iter()
            .any(|r| r.contains("header could not be read")),
        "{:?}",
        report.residue
    );
}

#[test]
fn comparisons_are_purged_in_a_home_that_never_reflected() {
    // `mecha sessions appraise` writes comparisons without ever creating the
    // learning store; the purge must depend only on the store it purges.
    let home = scratch("no-learning");
    let roots = Roots::under(&home.0);
    session(&roots, GONE, &home.0.join("work/web/a"), CANARY);
    write(
        &roots.comparisons.join("comparisons.jsonl"),
        &format!("{{\"id\":\"c1\",\"pointers\":{{\"session_id\":\"{GONE}\"}}}}\n"),
    );
    assert!(!roots.learning.exists());
    let report = forget(
        &roots,
        GONE,
        &Graph::answering(vec![Ok(GraphOutcome::Absent)]),
    )
    .unwrap();
    assert!(report.complete, "{:?}", report.errors);
    assert!(report
        .removed
        .iter()
        .any(|(s, n)| s == "comparisons" && *n == 1));
    assert_eq!(holding(&home.0, GONE), Vec::<PathBuf>::new());
}
