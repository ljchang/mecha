use super::super::memory::{Kind, NewEpisode, NewFact, Source};
use super::super::tests_support::{fill_core, new, no_lib, scratch};
use super::*;
use crate::agent::Taint;
use crate::message::{Block, Message};
use crate::session::Record;
use std::path::Path;

fn world() -> PathBuf {
    let dir = scratch();
    crate::persona::create(&dir, &no_lib(), new("mara")).unwrap();
    fill_core(&dir, "mara");
    dir
}

fn persona(dir: &Path) -> Persona {
    Store::load(dir).get("mara").unwrap().clone()
}

fn line(r: &Record) -> String {
    serde_json::to_string(r).unwrap() + "\n"
}

fn checkpoint(untrusted: bool) -> String {
    line(&Record::Taint(Taint {
        private: false,
        untrusted,
    }))
}

/// A transcript of the persona's, as a chat writes it.
fn transcript(dir: &Path, chat: &str, body: &[String]) {
    let sessions = dir.join("mara").join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let meta = format!(
        "{{\"record\":\"meta\",\"id\":\"{chat}\",\"created_at\":\"2026-10-01T03:00:00Z\",\"provider\":\"local\",\"model\":\"m\",\"workspace\":\"/tmp\",\"title\":\"persona: Mara\"}}\n"
    );
    std::fs::write(
        sessions.join(format!("{chat}.jsonl")),
        meta + &body.concat(),
    )
    .unwrap();
}

fn owner(t: &str) -> String {
    line(&Record::Message(Message::user(t)))
}

fn says(t: &str) -> String {
    line(&Record::Message(Message::assistant(vec![Block::text(t)])))
}

fn episode(dir: &Path, chat: &str, from: u32, to: u32, summary: &str, origin: Origin) -> String {
    let m = Memory::open(dir, "mara").unwrap();
    let e = m
        .add_episode(NewEpisode {
            source: Some(Source {
                chat: chat.into(),
                from,
                to,
            }),
            summary: summary.into(),
            origin,
            model: "m".into(),
            ..NewEpisode::default()
        })
        .unwrap();
    if origin == Origin::ModelUntrusted {
        m.approve(&e.uid).unwrap();
    }
    e.uid
}

fn fact(dir: &Path, table: Table, text: &str, origin: Origin) {
    let kind = if table == Table::Inferred {
        Kind::Inferred
    } else {
        Kind::Stated
    };
    let m = Memory::open(dir, "mara").unwrap();
    let f = m
        .add_fact(
            table,
            NewFact {
                text: text.into(),
                kind,
                source: Source {
                    chat: "c1".into(),
                    from: 0,
                    to: 1,
                },
                origin,
                model: "m".into(),
                valid_from: None,
                valid_to: None,
            },
        )
        .unwrap();
    if origin == Origin::ModelUntrusted {
        m.approve(&f.uid).unwrap();
    }
}

async fn search(dir: &Path, query: &str) -> ToolOutput {
    MemorySearch::new(dir.to_path_buf(), "mara".into(), None)
        .call(json!({ "query": query }), &ToolCtx::default())
        .await
        .unwrap()
}

#[tokio::test]
async fn a_search_returns_what_it_remembers_with_ids_for_conversations() {
    let dir = world();
    let uid = episode(
        &dir,
        "c1",
        0,
        1,
        "Named the kelp project Holdfast.",
        Origin::ModelClean,
    );
    fact(
        &dir,
        Table::User,
        "Teaches the Holdfast seminar.",
        Origin::ModelClean,
    );
    let out = search(&dir, "holdfast").await;
    assert!(!out.is_error && !out.external, "{}", out.content);
    assert!(
        out.content.contains(&format!(
            "a conversation [{}]: Named the kelp project Holdfast.",
            short(&uid)
        )),
        "{}",
        out.content
    );
    assert!(out
        .content
        .contains("about the owner: Teaches the Holdfast seminar."));
    assert!(out.content.starts_with("From your memory, by words:"));
}

#[tokio::test]
async fn a_search_that_returns_a_record_from_outside_is_marked_so() {
    let dir = world();
    fact(
        &dir,
        Table::Persona,
        "A page said Holdfast sings.",
        Origin::ModelUntrusted,
    );
    let out = search(&dir, "holdfast").await;
    assert!(
        out.content.contains("Holdfast sings") && out.external,
        "{}",
        out.content
    );
}

#[tokio::test]
async fn a_search_reads_the_switches_live_and_never_creates_a_store() {
    let dir = world();
    let out = search(&dir, "holdfast").await;
    assert_eq!(out.content, "You remember nothing yet.");
    assert!(!dir.join("mara").join(MEMORY_DB).exists());

    fact(
        &dir,
        Table::User,
        "Teaches the Holdfast seminar.",
        Origin::ModelClean,
    );
    // Switched off after the chat began: the next call says so.
    let toml = dir.join("mara").join("persona.toml");
    let text = std::fs::read_to_string(&toml).unwrap();
    let off = text
        .replace("episodic    = true", "episodic    = false")
        .replace("semantic    = true", "semantic    = false")
        .replace("user_facts  = \"shared\"", "user_facts  = \"off\"");
    assert_ne!(off, text, "the template's spelling moved");
    std::fs::write(&toml, off).unwrap();
    let out = search(&dir, "holdfast").await;
    assert!(
        out.is_error && out.content.contains("switched off"),
        "{}",
        out.content
    );
}

#[test]
fn reading_a_conversation_returns_its_turns_and_carries_their_taint() {
    let dir = world();
    transcript(
        &dir,
        "c1",
        &[
            owner("Let us call the kelp project Holdfast."),
            says("Holdfast it is."),
            checkpoint(false),
            owner("Search what kelp sounds like."),
            says("A page says kelp sings at dawn."),
            checkpoint(true),
        ],
    );
    let clean = episode(&dir, "c1", 0, 1, "Named the project.", Origin::ModelClean);
    let web = episode(
        &dir,
        "c1",
        2,
        3,
        "Read about kelp singing.",
        Origin::ModelUntrusted,
    );
    let p = persona(&dir);

    let r = read_episode(&dir, &p, short(&clean)).unwrap();
    assert!(!r.untrusted);
    assert!(
        r.text
            .contains("[owner] Let us call the kelp project Holdfast."),
        "{}",
        r.text
    );
    assert!(r.text.contains("Holdfast it is."));
    assert!(!r.text.contains("sings"), "only its own turns: {}", r.text);

    let r = read_episode(&dir, &p, short(&web)).unwrap();
    assert!(r.untrusted, "the turns read from outside carry it");
    assert!(r.text.contains("kelp sings at dawn"));
}

#[test]
fn a_turn_no_checkpoint_covers_is_read_as_from_outside() {
    let dir = world();
    transcript(&dir, "c1", &[owner("hello there"), says("hi")]);
    let uid = episode(&dir, "c1", 0, 1, "Said hello.", Origin::ModelClean);
    assert!(
        read_episode(&dir, &persona(&dir), short(&uid))
            .unwrap()
            .untrusted
    );
}

#[test]
fn an_id_or_a_record_that_could_reach_elsewhere_is_refused() {
    let dir = world();
    transcript(&dir, "c1", &[owner("hello"), says("hi"), checkpoint(false)]);
    let p = persona(&dir);
    for bad in ["", "zz", "../../etc", "abc", "not-hex-at-all"] {
        assert!(read_episode(&dir, &p, bad).is_err(), "`{bad}`");
    }
    // A record naming a chat that is not a session id never becomes a path.
    let uid = episode(&dir, "../escape", 0, 1, "Odd.", Origin::ModelClean);
    let err = read_episode(&dir, &p, short(&uid)).unwrap_err();
    assert!(err.contains("cannot be opened"), "{err}");
    // A conversation gone from disk: the memory of it is what is left.
    let gone = episode(&dir, "c2", 0, 1, "Gone.", Origin::ModelClean);
    let err = read_episode(&dir, &p, short(&gone)).unwrap_err();
    assert!(err.contains("no longer kept"), "{err}");
}

#[test]
fn a_long_conversation_is_cut_and_says_so() {
    let dir = world();
    let long = "We went on about kelp. ".repeat(600);
    transcript(
        &dir,
        "c1",
        &[owner(&long), says("Indeed."), checkpoint(false)],
    );
    let uid = episode(&dir, "c1", 0, 1, "A long talk.", Origin::ModelClean);
    let r = read_episode(&dir, &persona(&dir), short(&uid)).unwrap();
    assert!(r.text.chars().count() < MAX_READ_CHARS + 400);
    assert!(r.text.contains("is cut here"));
}

#[test]
fn the_tools_are_offered_as_the_memory_switches_say() {
    let mut s = persona(&world()).settings;
    assert!(offers_search(&s) && offers_read(&s));
    s.memory.episodic = false;
    assert!(offers_search(&s) && !offers_read(&s));
    s.memory.semantic = false;
    s.memory.user_facts = UserFacts::Off;
    assert!(!offers_search(&s));
}

#[test]
fn neither_tool_can_send_anywhere() {
    let dir = world();
    let search = MemorySearch::new(dir.clone(), "mara".into(), None);
    let read = MemoryRead::new(dir, "mara".into());
    for caps in [search.capabilities(), read.capabilities()] {
        assert_eq!(caps.egress, crate::tool::Egress::None);
        assert!(caps.private_data && caps.untrusted_input);
    }
}
