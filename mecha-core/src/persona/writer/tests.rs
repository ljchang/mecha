use super::super::tests_support::{fill_core, new, no_lib, scratch};
use super::*;
use crate::message::{Block, CompletionRequest, StopReason};
use crate::session::Record;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// ── transcripts, as a chat writes them ───────────────────────────────────

fn line(r: &Record) -> String {
    serde_json::to_string(r).unwrap() + "\n"
}

fn owner(text: &str) -> String {
    line(&Record::Message(Message::user(text)))
}

fn persona_says(text: &str) -> String {
    line(&Record::Message(Message::assistant(vec![Block::text(
        text,
    )])))
}

fn checkpoint(untrusted: bool) -> String {
    line(&Record::Taint(Taint {
        private: false,
        untrusted,
    }))
}

const META: &str = "{\"record\":\"meta\",\"id\":\"c1\",\"created_at\":\"2026-10-01T03:00:00Z\",\"provider\":\"local\",\"model\":\"local-model\",\"title\":\"persona: Mara\"}\n";

/// Two clean runs, then a run that read the web.
fn two_clean_then_untrusted() -> String {
    [
        META.to_string(),
        owner("We should call the kelp project Holdfast."),
        persona_says("Holdfast it is."),
        checkpoint(false),
        owner("I teach my methods seminar on Thursdays."),
        persona_says("Noted, Thursdays."),
        checkpoint(false),
        owner("Search what kelp sounds like."),
        persona_says("A page says kelp sings at dawn."),
        checkpoint(true),
    ]
    .concat()
}

// ── reading ──────────────────────────────────────────────────────────────

#[test]
fn a_turn_is_its_message_record_and_a_rewrite_moves_no_address() {
    let text = [
        META.to_string(),
        owner("one"),
        persona_says("two"),
        checkpoint(false),
        // A compaction: the list is replaced, but nothing is appended.
        line(&Record::Rewrite {
            messages: vec![Message::user("summary of one and two")],
        }),
        // A line from a newer build: counted, unread.
        "{\"record\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"hologram\"}]}\n"
            .to_string(),
        persona_says("four"),
        checkpoint(false),
    ]
    .concat();
    let chat = read_chat(&text);
    assert_eq!(chat.turns.len(), 4);
    assert!(chat.turns[2].message.is_none(), "counted, not read");
    assert_eq!(chat.turns[3].message.as_ref().unwrap().text(), "four");
    assert_eq!(chat.model.as_deref(), Some("local-model"));
    // The unreadable turn cuts the clean stretch: unknown is never clean.
    let s = stretches(&chat, 0);
    assert_eq!(s.len(), 2);
    assert_eq!(
        (s[0].from, s[0].to, s[0].origin),
        (0, 2, Origin::ModelClean)
    );
    assert_eq!((s[1].from, s[1].to), (2, 4));
    assert_eq!(s[1].origin, Origin::ModelUntrusted);
}

#[test]
fn provenance_is_split_where_taint_first_rose_and_unknown_is_untrusted() {
    let chat = read_chat(&two_clean_then_untrusted());
    let s = stretches(&chat, 0);
    assert_eq!(
        s,
        [
            Stretch {
                from: 0,
                to: 4,
                origin: Origin::ModelClean
            },
            Stretch {
                from: 4,
                to: 6,
                origin: Origin::ModelUntrusted
            },
        ]
    );
    // From the ledger's point on: the clean part is behind it.
    assert_eq!(stretches(&chat, 4).len(), 1);
    assert!(stretches(&chat, 6).is_empty());

    // A run with no checkpoint after it (torn, or still going) is unknown.
    let torn = [META.to_string(), owner("hi"), persona_says("hello")].concat();
    let s = stretches(&read_chat(&torn), 0);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].origin, Origin::ModelUntrusted);
}

#[test]
fn blocks_folded_in_after_a_checkpoint_are_covered_by_the_next_one() {
    let text = [
        META.to_string(),
        owner("hello"),
        checkpoint(false),
        line(&Record::Extend {
            index: 0,
            blocks: vec![Block::text("a stranger's page")],
        }),
        persona_says("hi"),
        checkpoint(true),
    ]
    .concat();
    let chat = read_chat(&text);
    assert!(chat.turns[0]
        .message
        .as_ref()
        .unwrap()
        .text()
        .contains("stranger"));
    assert!(
        chat.turns[0].taint.unwrap().untrusted,
        "re-covered, not kept clean"
    );
}

#[test]
fn the_model_reads_the_owner_and_the_character_by_name() {
    let chat = read_chat(&two_clean_then_untrusted());
    let s = stretches(&chat, 0)[0];
    let r = render(&chat, s, "Mara");
    assert!(
        r.contains("[owner] We should call the kelp project Holdfast."),
        "{r}"
    );
    assert!(r.contains("[Mara] Holdfast it is."), "{r}");
    assert!(
        !r.contains("kelp sings"),
        "the untrusted stretch is not in the clean one"
    );
}

// ── the reply ────────────────────────────────────────────────────────────

#[test]
fn a_reply_is_salvaged_entry_by_entry() {
    let reply = r#"Here you go: {"episode": {"summary": "Named the project."},
      "facts": [
        {"op": "add", "about": "character", "how": "observed", "text": "The project is Holdfast."},
        {"op": "add", "about": "owner", "how": "guessed", "text": "Likes kelp."},
        {"op": "add", "about": "the moon", "text": "Dropped."},
        {"op": "add", "about": "owner", "how": "stated", "text": "  "},
        {"op": "update", "id": "ab12cd34", "text": "New wording."},
        {"op": "invalidate"},
        {"op": "explode", "id": "x"}
      ]}"#;
    let p = parse_reply(reply).unwrap();
    assert_eq!(p.episode.unwrap().summary, "Named the project.");
    assert_eq!(
        p.ops,
        [
            Op::Add {
                about_owner: false,
                kind: Kind::Observed,
                text: "The project is Holdfast.".into()
            },
            Op::Add {
                about_owner: true,
                kind: Kind::Inferred,
                text: "Likes kelp.".into()
            },
            Op::Update {
                id: "ab12cd34".into(),
                text: "New wording.".into()
            },
        ]
    );
    assert!(parse_reply("no json here").is_none());
    assert_eq!(
        parse_reply(r#"{"episode": null}"#).unwrap(),
        Proposal::default()
    );
}

// ── applying ─────────────────────────────────────────────────────────────

struct World {
    dir: PathBuf,
    persona: Persona,
}

fn world() -> World {
    let dir = scratch();
    crate::persona::create(&dir, &no_lib(), new("mara")).unwrap();
    fill_core(&dir, "mara");
    let persona = crate::persona::Store::load(&dir)
        .get("mara")
        .unwrap()
        .clone();
    World { dir, persona }
}

fn add(about_owner: bool, kind: Kind, text: &str) -> Op {
    Op::Add {
        about_owner,
        kind,
        text: text.into(),
    }
}

fn stretch(origin: Origin) -> Stretch {
    Stretch {
        from: 0,
        to: 2,
        origin,
    }
}

fn run(w: &World, s: Stretch, ops: Vec<Op>) -> Applied {
    let m = Memory::open(&w.dir, "mara").unwrap();
    let chat = read_chat(&two_clean_then_untrusted());
    let known = known(&m).unwrap();
    let upto = m.written_upto("c1").unwrap();
    m.write_stretch("c1", upto, upto + 2, |m| {
        apply(
            m,
            &w.persona,
            "c1",
            &chat,
            s,
            &known,
            Proposal { episode: None, ops },
        )
    })
    .unwrap()
    .unwrap()
}

#[test]
fn facts_land_in_their_tables_and_an_untrusted_stretchs_wait_on_the_owner() {
    let w = world();
    let out = run(
        &w,
        stretch(Origin::ModelClean),
        vec![
            add(false, Kind::Observed, "The project is Holdfast."),
            add(true, Kind::Stated, "Teaches on Thursdays."),
            add(true, Kind::Inferred, "Seems to enjoy fieldwork."),
            add(true, Kind::Stated, "teaches on thursdays."),
        ],
    );
    assert_eq!(out.added, 3);
    assert_eq!(out.refused, 1, "the duplicate");
    let m = Memory::open(&w.dir, "mara").unwrap();
    assert_eq!(
        m.facts(Table::Persona, Filter::Recallable).unwrap().len(),
        1
    );
    assert_eq!(m.facts(Table::User, Filter::Recallable).unwrap().len(), 1);
    assert_eq!(
        m.facts(Table::Inferred, Filter::Recallable).unwrap().len(),
        1
    );
    let f = &m.facts(Table::User, Filter::All).unwrap()[0];
    assert_eq!(f.source.chat, "c1");
    assert_eq!((f.source.from, f.source.to), (0, 1));
    assert_eq!(f.model, "local-model");

    run(
        &w,
        stretch(Origin::ModelUntrusted),
        vec![add(true, Kind::Stated, "Lives by the sea.")],
    );
    let all = m.facts(Table::User, Filter::All).unwrap();
    let sea = all.iter().find(|f| f.text == "Lives by the sea.").unwrap();
    assert_eq!(sea.status, Status::Candidate);
    assert_eq!(sea.origin, Origin::ModelUntrusted);
}

#[test]
fn an_untrusted_stretch_cannot_reach_what_the_persona_already_knows() {
    let w = world();
    run(
        &w,
        stretch(Origin::ModelClean),
        vec![add(true, Kind::Stated, "Teaches on Thursdays.")],
    );
    let m = Memory::open(&w.dir, "mara").unwrap();
    let id = m.facts(Table::User, Filter::All).unwrap()[0].uid[..8].to_owned();
    let out = run(
        &w,
        stretch(Origin::ModelUntrusted),
        vec![
            Op::Invalidate { id: id.clone() },
            Op::Update {
                id,
                text: "Teaches on Fridays.".into(),
            },
        ],
    );
    assert_eq!((out.invalidated, out.updated, out.refused), (0, 0, 2));
    assert_eq!(
        m.facts(Table::User, Filter::Recallable).unwrap()[0].text,
        "Teaches on Thursdays."
    );
}

#[test]
fn a_clean_stretch_updates_a_models_fact_but_never_the_owners_or_a_pinned_one() {
    let w = world();
    run(
        &w,
        stretch(Origin::ModelClean),
        vec![
            add(true, Kind::Stated, "Teaches on Thursdays."),
            add(true, Kind::Stated, "Has a cat."),
            add(true, Kind::Stated, "Runs at dawn."),
        ],
    );
    let m = Memory::open(&w.dir, "mara").unwrap();
    let uid = |t: &str| {
        m.facts(Table::User, Filter::All)
            .unwrap()
            .into_iter()
            .find(|f| f.text == t)
            .unwrap()
            .uid
    };
    let owners = m.correct(&uid("Has a cat."), "Has two cats.").unwrap();
    m.pin(&uid("Runs at dawn."), true).unwrap();

    let out = run(
        &w,
        stretch(Origin::ModelClean),
        vec![
            Op::Update {
                id: uid("Teaches on Thursdays.")[..8].into(),
                text: "Teaches on Fridays now.".into(),
            },
            Op::Invalidate {
                id: owners.uid[..8].into(),
            },
            Op::Invalidate {
                id: uid("Runs at dawn.")[..8].into(),
            },
        ],
    );
    assert_eq!((out.updated, out.invalidated, out.refused), (1, 0, 2));
    let live: Vec<String> = m
        .facts(Table::User, Filter::Recallable)
        .unwrap()
        .into_iter()
        .map(|f| f.text)
        .collect();
    assert!(live.contains(&"Teaches on Fridays now.".to_string()));
    assert!(live.contains(&"Has two cats.".to_string()));
    assert!(live.contains(&"Runs at dawn.".to_string()));
    assert!(!live.contains(&"Teaches on Thursdays.".to_string()));
    let fridays = m
        .facts(Table::User, Filter::All)
        .unwrap()
        .into_iter()
        .find(|f| f.text == "Teaches on Fridays now.")
        .unwrap();
    assert!(fridays.replaces.is_some(), "an update points back");
}

#[test]
fn switched_off_memory_writes_nothing_of_that_kind() {
    let mut w = world();
    w.persona.settings.memory.semantic = false;
    w.persona.settings.memory.user_facts = UserFacts::Off;
    let out = run(
        &w,
        stretch(Origin::ModelClean),
        vec![
            add(false, Kind::Observed, "The project is Holdfast."),
            add(true, Kind::Stated, "Teaches on Thursdays."),
        ],
    );
    assert_eq!((out.added, out.refused), (0, 2));
}

// ── one chat, end to end ─────────────────────────────────────────────────

struct Recording {
    replies: Mutex<std::collections::VecDeque<String>>,
    seen: Arc<Mutex<Vec<CompletionRequest>>>,
}

#[async_trait::async_trait]
impl crate::provider::Provider for Recording {
    fn id(&self) -> &str {
        "recording"
    }
    fn default_model(&self) -> &str {
        "local-1"
    }
    async fn complete(
        &self,
        req: &CompletionRequest,
        _sink: Option<&crate::provider::StreamSink>,
    ) -> Result<crate::message::CompletionResponse> {
        self.seen.lock().unwrap().push(req.clone());
        let text = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("a scripted reply");
        Ok(crate::message::CompletionResponse {
            message: Message::assistant(vec![Block::text(text)]),
            stop_reason: StopReason::EndTurn,
            usage: crate::message::Usage::default(),
            refusal: None,
            model: "local-1".into(),
            malformed_tool_args: 0,
        })
    }
}

fn scripted(replies: &[&str]) -> (Writer, Arc<Mutex<Vec<CompletionRequest>>>) {
    let seen = Arc::default();
    let provider = Recording {
        replies: Mutex::new(replies.iter().map(|r| r.to_string()).collect()),
        seen: Arc::clone(&seen),
    };
    (Writer::new(Box::new(provider), None), seen)
}

fn asked(req: &CompletionRequest) -> String {
    assert!(req.tools.is_empty(), "a quarantined pass has no tools");
    assert_eq!(req.messages.len(), 1, "and no history");
    req.messages[0].text()
}

#[tokio::test]
async fn a_chat_is_written_once_in_two_stretches_and_the_clean_one_never_sees_the_web() {
    let w = world();
    let m = Memory::open(&w.dir, "mara").unwrap();
    let (writer, seen) = scripted(&[
        r#"{"episode": {"summary": "Named the kelp project Holdfast."},
            "facts": [{"op": "add", "about": "owner", "how": "stated", "text": "Teaches on Thursdays."}]}"#,
        r#"{"episode": {"summary": "Read that kelp sings at dawn."},
            "facts": [{"op": "add", "about": "character", "how": "observed", "text": "Kelp sings at dawn."}]}"#,
    ]);
    let text = two_clean_then_untrusted();
    let report = write_chat(&writer, &m, &w.persona, "c1", &read_chat(&text))
        .await
        .unwrap();
    assert_eq!(report.stretches.len(), 2);
    assert_eq!(m.written_upto("c1").unwrap(), 6);

    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    let clean = asked(&seen[0]);
    assert!(
        clean.contains("Holdfast") && !clean.contains("sings"),
        "{clean}"
    );
    assert!(
        clean.contains("<core>\nA marine ecologist"),
        "the Core is shown"
    );
    // The second call is shown what the first one wrote, with its id.
    assert!(asked(&seen[1]).contains("about the owner: Teaches on Thursdays."));

    let eps = m.episodes(Filter::All).unwrap();
    assert_eq!(eps.len(), 2);
    let web = eps.iter().find(|e| e.summary.contains("sings")).unwrap();
    assert_eq!(web.status, Status::Candidate);
    assert_eq!((web.source.from, web.source.to), (4, 5));
    assert_eq!(
        m.facts(Table::Persona, Filter::All).unwrap()[0].status,
        Status::Candidate
    );

    // Run again: nothing new, so no model call and nothing written.
    let (again, seen) = scripted(&[]);
    let report = write_chat(&again, &m, &w.persona, "c1", &read_chat(&text))
        .await
        .unwrap();
    assert!(report.stretches.is_empty());
    assert!(seen.lock().unwrap().is_empty());

    // The chat grows; only the new turns are read.
    let more = text + &owner("Goodnight.") + &persona_says("Goodnight.") + &checkpoint(true);
    let (third, seen) = scripted(&[r#"{"episode": null, "facts": []}"#]);
    write_chat(&third, &m, &w.persona, "c1", &read_chat(&more))
        .await
        .unwrap();
    let only = asked(&seen.lock().unwrap()[0]);
    assert!(
        only.contains("Goodnight") && !only.contains("kelp sings"),
        "{only}"
    );
    assert_eq!(m.written_upto("c1").unwrap(), 8);
}

#[tokio::test]
async fn a_second_writer_on_the_same_stretch_writes_nothing() {
    let w = world();
    let m = Memory::open(&w.dir, "mara").unwrap();
    let other = Memory::open(&w.dir, "mara").unwrap();
    // The other writer finishes first.
    other
        .write_stretch("c1", 0, 4, |_| Ok(()))
        .unwrap()
        .unwrap();
    let wrote = m
        .write_stretch("c1", 0, 4, |m| {
            m.add_fact(
                Table::User,
                NewFact {
                    text: "Twice?".into(),
                    kind: Kind::Stated,
                    source: Source {
                        chat: "c1".into(),
                        from: 0,
                        to: 3,
                    },
                    origin: Origin::ModelClean,
                    model: "m".into(),
                    valid_from: None,
                    valid_to: None,
                },
            )
        })
        .unwrap();
    assert!(wrote.is_none());
    assert!(m.facts(Table::User, Filter::All).unwrap().is_empty());
}

#[tokio::test]
async fn memory_switched_off_asks_nothing_and_still_moves_on() {
    let mut w = world();
    w.persona.settings.memory.episodic = false;
    w.persona.settings.memory.semantic = false;
    w.persona.settings.memory.user_facts = UserFacts::Off;
    let m = Memory::open(&w.dir, "mara").unwrap();
    let (writer, seen) = scripted(&[]);
    write_chat(
        &writer,
        &m,
        &w.persona,
        "c1",
        &read_chat(&two_clean_then_untrusted()),
    )
    .await
    .unwrap();
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(m.written_upto("c1").unwrap(), 6);
}

#[test]
fn a_chat_is_written_by_its_own_model_and_never_swaps_the_resident_one() {
    let use_ = |m: &str| ModelPick::Use(m.into());
    let pick = |chat, resident| pick_model(chat, Server::Router(resident), "default");
    assert_eq!(pick(Some("story"), Some("story")), use_("story"));
    assert_eq!(
        pick(Some("story"), Some("work")),
        ModelPick::Wait("story".into())
    );
    // An empty router: loading the chat's model evicts nothing.
    assert_eq!(pick(Some("story"), None), use_("story"));
    assert_eq!(pick(None, Some("work")), use_("work"));
    assert_eq!(pick(None, None), use_("default"));
    // One model served, whatever a request names.
    assert_eq!(
        pick_model(Some("story"), Server::One, "default"),
        use_("default")
    );
    // A router that would not say: naming any model could be the swap.
    assert_eq!(
        pick_model(Some("story"), Server::Unanswered, "default"),
        ModelPick::Wait("story".into())
    );
    assert_eq!(
        pick_model(None, Server::Unanswered, "default"),
        ModelPick::Wait("default".into())
    );
}

#[test]
fn one_proposal_updates_a_fact_once_and_never_adds_what_it_just_wrote() {
    let w = world();
    run(
        &w,
        stretch(Origin::ModelClean),
        vec![add(true, Kind::Stated, "Teaches on Thursdays.")],
    );
    let m = Memory::open(&w.dir, "mara").unwrap();
    let id = m.facts(Table::User, Filter::All).unwrap()[0].uid[..8].to_owned();
    let out = run(
        &w,
        stretch(Origin::ModelClean),
        vec![
            Op::Update {
                id: id.clone(),
                text: "Teaches on Fridays.".into(),
            },
            Op::Update {
                id: id.clone(),
                text: "Teaches on Mondays.".into(),
            },
            Op::Invalidate { id },
            add(true, Kind::Stated, "Teaches on Fridays."),
        ],
    );
    assert_eq!(
        (out.updated, out.invalidated, out.added, out.refused),
        (1, 0, 0, 3)
    );
    assert_eq!(
        m.facts(Table::User, Filter::Recallable)
            .unwrap()
            .into_iter()
            .map(|f| f.text)
            .collect::<Vec<_>>(),
        ["Teaches on Fridays."],
        "one live replacement"
    );
}

#[test]
fn an_over_long_summary_is_turned_away_and_the_facts_beside_it_are_kept() {
    let w = world();
    let m = Memory::open(&w.dir, "mara").unwrap();
    let chat = read_chat(&two_clean_then_untrusted());
    let out = m
        .write_stretch("c1", 0, 2, |m| {
            apply(
                m,
                &w.persona,
                "c1",
                &chat,
                stretch(Origin::ModelClean),
                &[],
                Proposal {
                    episode: Some(EpisodeDraft {
                        summary: "x".repeat(MAX_SUMMARY_CHARS + 1),
                        ..EpisodeDraft::default()
                    }),
                    ops: vec![add(true, Kind::Stated, "Teaches on Thursdays.")],
                },
            )
        })
        .unwrap()
        .unwrap();
    assert_eq!((out.episodes, out.added, out.refused), (0, 1, 1));
    assert_eq!(m.written_upto("c1").unwrap(), 2);
}

#[test]
fn an_id_the_model_invents_is_turned_away_never_sliced() {
    let w = world();
    run(
        &w,
        stretch(Origin::ModelClean),
        vec![add(true, Kind::Stated, "Teaches on Thursdays.")],
    );
    // Multi-byte at the eighth byte: sliced by bytes, this panicked.
    let out = run(
        &w,
        stretch(Origin::ModelUntrusted),
        vec![
            Op::Invalidate {
                id: "abcdefg\u{1F600}".into(),
            },
            Op::Update {
                id: "zzzzzzzz".into(),
                text: "x".into(),
            },
        ],
    );
    assert_eq!(out.refused, 2);
}

#[tokio::test]
async fn an_unanswered_owner_turn_waits_for_its_reply() {
    let w = world();
    let m = Memory::open(&w.dir, "mara").unwrap();
    // The run failed after the owner spoke: a checkpoint, no reply.
    let first = [
        META.to_string(),
        owner("The seminar moved to Fridays."),
        checkpoint(false),
    ]
    .concat();
    let (none, seen) = scripted(&[]);
    write_chat(&none, &m, &w.persona, "c1", &read_chat(&first))
        .await
        .unwrap();
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(m.written_upto("c1").unwrap(), 0, "not skipped for good");

    // The persona answers the next day; the owner's words reach the model.
    let later = first + &persona_says("Fridays, then.") + &checkpoint(false);
    let (writer, seen) = scripted(&[r#"{"episode": null, "facts": []}"#]);
    write_chat(&writer, &m, &w.persona, "c1", &read_chat(&later))
        .await
        .unwrap();
    assert!(asked(&seen.lock().unwrap()[0]).contains("The seminar moved to Fridays."));
    assert_eq!(m.written_upto("c1").unwrap(), 2);
}

#[test]
fn a_message_with_a_block_this_build_does_not_know_keeps_the_rest() {
    let text = [
        META.to_string(),
        "{\"record\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"hello\"},{\"type\":\"hologram\"}]}\n".to_string(),
        persona_says("hi"),
        checkpoint(false),
    ]
    .concat();
    let chat = read_chat(&text);
    assert_eq!(chat.turns[0].message.as_ref().unwrap().text(), "hello");
    assert_eq!(stretches(&chat, 0).len(), 1, "the chat stays clean");
}

#[test]
fn a_test_chat_never_becomes_a_memory_and_an_unreadable_one_is_said() {
    let dir = scratch().join("sessions");
    std::fs::create_dir_all(&dir).unwrap();
    let meta = |id: &str, kind: &str| {
        format!(
            "{{\"record\":\"meta\",\"id\":\"{id}\",\"created_at\":\"2026-10-01T03:00:00Z\",\"provider\":\"local\",\"model\":\"m\",\"workspace\":\"/tmp\",\"title\":\"persona: Mara\"{kind}}}\n"
        )
    };
    let body = owner("hi") + &persona_says("hello") + &checkpoint(false);
    std::fs::write(dir.join("real.jsonl"), meta("real", "") + &body).unwrap();
    std::fs::write(
        dir.join("smoke.jsonl"),
        meta("smoke", ",\"kind\":\"test\"") + &body,
    )
    .unwrap();
    std::fs::write(
        dir.join("trial.jsonl"),
        meta("trial", ",\"kind\":\"experiment\"") + &body,
    )
    .unwrap();
    std::fs::write(dir.join("torn.jsonl"), "{not json\n").unwrap();
    std::fs::write(dir.join("real.persona.json"), "{}").unwrap();

    let p = pending_chats(&dir, None, std::time::Duration::ZERO);
    assert_eq!(
        p.due.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
        ["real"]
    );
    assert_eq!(p.marked, 2);
    assert_eq!(p.problems.len(), 1, "{:?}", p.problems);
    assert!(p.problems[0].contains("torn"));

    // Recent chats wait unless named.
    let p = pending_chats(&dir, None, std::time::Duration::from_secs(3600));
    assert!(p.due.is_empty());
    let p = pending_chats(&dir, Some("real"), std::time::Duration::from_secs(3600));
    assert_eq!(p.due.len(), 1);
    // A missing folder is a persona that has not chatted.
    assert_eq!(
        pending_chats(&dir.join("nope"), None, std::time::Duration::ZERO),
        Pending::default()
    );
}

#[test]
fn an_owner_correction_during_the_model_call_is_not_undone() {
    let w = world();
    run(
        &w,
        stretch(Origin::ModelClean),
        vec![add(true, Kind::Stated, "Teaches on Thursdays.")],
    );
    let m = Memory::open(&w.dir, "mara").unwrap();
    // What the writer read before asking the model...
    let snapshot = known(&m).unwrap();
    let old = snapshot[0].uid.clone();
    // ...and what the owner did while it was asking.
    m.correct(&old, "Teaches on Fridays.").unwrap();
    let chat = read_chat(&two_clean_then_untrusted());
    let upto = m.written_upto("c1").unwrap();
    let out = m
        .write_stretch("c1", upto, upto + 2, |m| {
            apply(
                m,
                &w.persona,
                "c1",
                &chat,
                stretch(Origin::ModelClean),
                &snapshot,
                Proposal {
                    episode: None,
                    ops: vec![Op::Update {
                        id: old[..8].into(),
                        text: "Teaches on Mondays.".into(),
                    }],
                },
            )
        })
        .unwrap()
        .unwrap();
    assert_eq!((out.updated, out.refused), (0, 1));
    assert_eq!(
        m.facts(Table::User, Filter::Recallable)
            .unwrap()
            .into_iter()
            .map(|f| f.text)
            .collect::<Vec<_>>(),
        ["Teaches on Fridays."]
    );
}
