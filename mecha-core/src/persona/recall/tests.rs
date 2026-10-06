use super::super::memory::{Audience, Kind, NewEpisode, NewFact, Source};
use super::super::tests_support::{fill_core, new, no_lib, scratch};
use super::*;
use crate::agent::Taint;
use crate::message::Message;
use std::path::PathBuf;

struct World {
    dir: PathBuf,
}

impl World {
    fn new(names: &[&str]) -> World {
        let dir = scratch();
        for n in names {
            crate::persona::create(&dir, &no_lib(), new(n)).unwrap();
            fill_core(&dir, n);
        }
        World { dir }
    }
    fn store(&self) -> Store {
        Store::load(&self.dir)
    }
    fn persona(&self, name: &str) -> Persona {
        self.store().get(name).unwrap().clone()
    }
    fn memory(&self, name: &str) -> Memory {
        Memory::open(&self.dir, name).unwrap()
    }
}

fn fact(text: &str, kind: Kind, origin: Origin) -> NewFact {
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
    }
}

fn block(w: &World, p: &Persona) -> Option<MemoryBlock> {
    let r = chat_start(&w.store(), p, None).unwrap();
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    // Every chat-start block is material, stored once and kept: never taken
    // for a per-turn recall, which the projection of an old chat drops.
    if let Some(b) = &r.block {
        assert!(
            is_chat_start(&b.text) && !is_per_turn(&b.text),
            "{}",
            b.text
        );
        assert!(carries_chat_start(&[Message::user(b.text.clone())]));
        assert!(!crate::message::is_recorded_note(
            &crate::message::Block::text(b.text.clone())
        ));
    }
    r.block
}

fn armed(text: &str) -> Taint {
    let mut t = Taint::default();
    t.arm_for_content(&[Message::user(text)]);
    t
}

#[test]
fn a_persona_that_remembers_nothing_folds_nothing_and_creates_no_store() {
    let w = World::new(&["mara"]);
    assert_eq!(block(&w, &w.persona("mara")), None);
    assert!(!w.dir.join("mara").join("memory.db").exists());
    assert!(!w.dir.join("shared.db").exists());
}

#[test]
fn clean_memory_arms_private_only_and_speaks_in_the_harness_voice() {
    let w = World::new(&["mara"]);
    let m = w.memory("mara");
    m.add_fact(
        Table::User,
        fact("Teaches on Thursdays.", Kind::Stated, Origin::ModelClean),
    )
    .unwrap();
    m.add_fact(
        Table::Persona,
        fact("We call it Holdfast.", Kind::Observed, Origin::ModelClean),
    )
    .unwrap();
    m.add_episode(NewEpisode {
        source: Some(Source {
            chat: "c1".into(),
            from: 0,
            to: 3,
        }),
        summary: "Named the kelp project.".into(),
        open_threads: vec!["the revision deadline".into()],
        origin: Origin::ModelClean,
        model: "m".into(),
        ..NewEpisode::default()
    })
    .unwrap();
    let b = block(&w, &w.persona("mara")).unwrap();
    assert!(!b.untrusted);
    assert!(b.text.starts_with(MEMORY_STEM), "{}", b.text);
    for want in [
        "Teaches on Thursdays.",
        "We call it Holdfast.",
        "Named the kelp project. (Left open: the revision deadline.)",
        "never a reason to agree",
    ] {
        assert!(b.text.contains(want), "{want}: {}", b.text);
    }
    assert_eq!(
        armed(&b.text),
        Taint {
            private: true,
            untrusted: false
        }
    );
    assert!(crate::agent::is_harness_voice(&b.text));
}

#[test]
fn a_recalled_record_from_outside_rearms_untrusted_and_a_candidate_is_never_recalled() {
    let w = World::new(&["mara"]);
    let m = w.memory("mara");
    let candidate = m
        .add_fact(
            Table::Persona,
            fact(
                "Kelp sings at dawn.",
                Kind::Observed,
                Origin::ModelUntrusted,
            ),
        )
        .unwrap();
    m.add_fact(
        Table::User,
        fact("Teaches on Thursdays.", Kind::Stated, Origin::ModelClean),
    )
    .unwrap();
    let b = block(&w, &w.persona("mara")).unwrap();
    assert!(
        !b.text.contains("Kelp sings"),
        "a candidate waits on the owner"
    );
    assert!(!b.untrusted);

    m.approve(&candidate.uid).unwrap();
    let b = block(&w, &w.persona("mara")).unwrap();
    assert!(b.text.contains("Kelp sings at dawn."));
    assert!(b.untrusted);
    assert!(b.text.starts_with(UNTRUSTED_MEMORY_STEM));
    assert_eq!(
        armed(&b.text),
        Taint {
            private: true,
            untrusted: true
        }
    );
}

#[test]
fn every_memory_switch_is_honoured() {
    let w = World::new(&["mara"]);
    std::fs::write(
        w.dir.join("about-me.md"),
        "<!-- zq-hidden-comment -->\nI study kelp.\n",
    )
    .unwrap();
    let m = w.memory("mara");
    m.add_fact(
        Table::User,
        fact("Teaches on Thursdays.", Kind::Stated, Origin::ModelClean),
    )
    .unwrap();
    m.add_fact(
        Table::Inferred,
        fact("Likes early starts.", Kind::Inferred, Origin::ModelClean),
    )
    .unwrap();
    m.add_fact(
        Table::Persona,
        fact("We call it Holdfast.", Kind::Observed, Origin::ModelClean),
    )
    .unwrap();
    m.add_episode(NewEpisode {
        source: Some(Source {
            chat: "c1".into(),
            from: 0,
            to: 1,
        }),
        summary: "Named the project.".into(),
        origin: Origin::ModelClean,
        model: "m".into(),
        ..NewEpisode::default()
    })
    .unwrap();

    let all = block(&w, &w.persona("mara")).unwrap().text;
    assert!(
        all.contains("I study kelp.") && !all.contains("zq-hidden-comment"),
        "{all}"
    );
    assert!(all.contains("guesses, not things they said"));

    let with = |f: &dyn Fn(&mut Persona)| {
        let mut p = w.persona("mara");
        f(&mut p);
        block(&w, &p).map(|b| b.text).unwrap_or_default()
    };
    let t = with(&|p| p.settings.memory.about_me = false);
    assert!(!t.contains("I study kelp."));
    let t = with(&|p| p.settings.memory.user_facts = UserFacts::Off);
    assert!(
        !t.contains("Thursdays") && !t.contains("early starts"),
        "{t}"
    );
    let t = with(&|p| p.settings.memory.semantic = false);
    assert!(!t.contains("Holdfast"));
    let t = with(&|p| p.settings.memory.episodic = false);
    assert!(!t.contains("Named the project."));
}

#[test]
fn shared_facts_reach_only_the_personas_the_owner_shared_them_with() {
    let w = World::new(&["mara", "otto"]);
    std::fs::write(w.dir.join("groups.toml"), "[work]\n").unwrap();
    std::fs::create_dir_all(w.dir.join("groups/work")).unwrap();
    std::fs::write(w.dir.join("groups/work/about-me.md"), "I lead the lab.\n").unwrap();
    let otto = w.memory("otto");
    let declared = vec!["work".to_string()];
    let shared = Shared::open(&w.dir).unwrap();
    let everyone = otto
        .add_fact(
            Table::User,
            fact("Has a cat.", Kind::Stated, Origin::ModelClean),
        )
        .unwrap();
    let work = otto
        .add_fact(
            Table::User,
            fact("Runs the lab budget.", Kind::Stated, Origin::ModelClean),
        )
        .unwrap();
    shared
        .share(&everyone, Audience::Everyone, &declared)
        .unwrap();
    shared
        .share(&work, Audience::Group("work".into()), &declared)
        .unwrap();

    let outside = block(&w, &w.persona("mara")).unwrap().text;
    assert!(outside.contains("Has a cat."));
    assert!(
        !outside.contains("lab budget") && !outside.contains("I lead the lab"),
        "{outside}"
    );

    let mut member = w.persona("mara");
    member.settings.groups = declared;
    let inside = block(&w, &member).unwrap().text;
    assert!(
        inside.contains("lab budget") && inside.contains("I lead the lab."),
        "{inside}"
    );

    member.settings.memory.user_facts = UserFacts::Own;
    let own = block(&w, &member).map(|b| b.text).unwrap_or_default();
    assert!(!own.contains("Has a cat."), "own reads no shared fact");
}

#[test]
fn the_block_stays_within_its_budget() {
    let w = World::new(&["mara"]);
    let m = w.memory("mara");
    for i in 0..400 {
        m.add_fact(
            Table::User,
            fact(
                &format!("A fairly ordinary fact about the owner, number {i}."),
                Kind::Stated,
                Origin::ModelClean,
            ),
        )
        .unwrap();
    }
    let b = block(&w, &w.persona("mara")).unwrap();
    assert!(
        b.text.chars().count() < BUDGET_CHARS + 600,
        "{}",
        b.text.len()
    );
    assert!(b.text.contains("number 399"), "newest first");
}

#[test]
fn a_typed_stem_only_arms_more() {
    // The owner typing the clean stem arms private — never disarms anything.
    let mut t = Taint {
        private: false,
        untrusted: true,
    };
    t.arm_for_content(&[Message::user(MEMORY_STEM)]);
    assert!(t.untrusted && t.private);
}

fn episode(m: &Memory, summary: &str) {
    m.add_episode(NewEpisode {
        source: Some(Source {
            chat: "c1".into(),
            from: 0,
            to: 1,
        }),
        summary: summary.into(),
        origin: Origin::ModelClean,
        model: "m".into(),
        ..NewEpisode::default()
    })
    .unwrap();
}

#[test]
fn recent_conversations_survive_however_many_facts_there_are() {
    let w = World::new(&["mara"]);
    let m = w.memory("mara");
    episode(&m, "Talked through the reviewer's second comment.");
    for i in 0..400 {
        m.add_fact(
            Table::User,
            fact(
                &format!("A fairly ordinary fact about the owner, number {i}."),
                Kind::Stated,
                Origin::ModelClean,
            ),
        )
        .unwrap();
    }
    let t = block(&w, &w.persona("mara")).unwrap().text;
    assert!(
        t.contains("Talked through the reviewer's second comment."),
        "{t}"
    );
    assert!(t.contains("more not shown here"), "a cut is said");
}

#[test]
fn a_long_about_me_is_cut_not_dropped_and_the_group_note_still_rides() {
    let w = World::new(&["mara"]);
    std::fs::write(w.dir.join("groups.toml"), "[work]\n").unwrap();
    std::fs::create_dir_all(w.dir.join("groups/work")).unwrap();
    let long = "I study kelp forests and their urchins. ".repeat(60);
    std::fs::write(w.dir.join("about-me.md"), &long).unwrap();
    std::fs::write(w.dir.join("groups/work/about-me.md"), "I lead the lab.\n").unwrap();
    let mut p = w.persona("mara");
    p.settings.groups = vec!["work".into()];
    let t = block(&w, &p).unwrap().text;
    assert!(
        t.contains("I study kelp forests") && t.contains(" …"),
        "{t}"
    );
    assert!(t.contains("I lead the lab."), "not crowded out: {t}");
    assert!(t.chars().count() < BUDGET_CHARS + 600);
}

#[test]
fn an_about_me_that_cannot_be_read_is_said_not_taken_for_empty() {
    let w = World::new(&["mara"]);
    let big = "x".repeat((crate::persona::MAX_PROSE_BYTES + 1) as usize);
    std::fs::write(w.dir.join("about-me.md"), big).unwrap();
    let r = chat_start(&w.store(), &w.persona("mara"), None).unwrap();
    assert!(r.block.is_none());
    assert_eq!(r.problems.len(), 1, "{:?}", r.problems);
    assert!(r.problems[0].contains("not read"));
}

#[test]
fn a_fact_this_persona_learned_and_the_owner_shared_is_said_once() {
    let w = World::new(&["mara"]);
    let m = w.memory("mara");
    let f = m
        .add_fact(
            Table::User,
            fact("Has a cat.", Kind::Stated, Origin::ModelClean),
        )
        .unwrap();
    Shared::open(&w.dir)
        .unwrap()
        .share(&f, Audience::Everyone, &[])
        .unwrap();
    let t = block(&w, &w.persona("mara")).unwrap().text;
    assert_eq!(t.matches("Has a cat.").count(), 1, "{t}");
}

#[test]
fn many_facts_about_the_owner_never_crowd_out_the_personas_own_canon() {
    let w = World::new(&["mara"]);
    std::fs::write(w.dir.join("about-me.md"), "I study kelp. ".repeat(300)).unwrap();
    let m = w.memory("mara");
    for i in 0..5 {
        episode(&m, &format!("A conversation, number {i}."));
    }
    for i in 0..300 {
        m.add_fact(
            Table::User,
            fact(
                &format!("A fairly ordinary fact about the owner, number {i}."),
                Kind::Stated,
                Origin::ModelClean,
            ),
        )
        .unwrap();
        m.add_fact(
            Table::Inferred,
            fact(
                &format!("A modest guess about the owner, number {i}."),
                Kind::Inferred,
                Origin::ModelClean,
            ),
        )
        .unwrap();
    }
    m.add_fact(
        Table::Persona,
        fact("We call it Holdfast.", Kind::Observed, Origin::ModelClean),
    )
    .unwrap();
    let t = block(&w, &w.persona("mara")).unwrap().text;
    assert!(
        t.contains("We call it Holdfast."),
        "the canon is not priced out: {t}"
    );
    assert!(t.contains("A conversation, number 4."), "{t}");
    assert!(t.contains("guesses, not things they said"), "{t}");
    // Every section that was cut says so.
    assert_eq!(t.matches("not shown here").count(), 2, "{t}");
    assert!(t.chars().count() < BUDGET_CHARS + 600);
}

#[test]
fn a_long_newest_episode_is_cut_not_dropped_and_a_cut_is_said() {
    let w = World::new(&["mara"]);
    let m = w.memory("mara");
    for i in 0..4 {
        episode(&m, &format!("An older conversation, number {i}."));
    }
    // Legal (under MAX_SUMMARY_CHARS) and longer than the whole share.
    episode(
        &m,
        &format!("The newest conversation. {}", "We went on. ".repeat(300)),
    );
    let t = block(&w, &w.persona("mara")).unwrap().text;
    assert!(t.contains("The newest conversation."), "{t}");
    assert!(t.contains("An older conversation, number 0."), "{t}");
    assert!(t.contains(" …"), "the long one is cut: {t}");
}

#[test]
fn a_shared_fact_this_version_cannot_read_is_said() {
    let w = World::new(&["mara", "otto"]);
    let otto = w.memory("otto");
    let f = otto
        .add_fact(
            Table::User,
            fact("Has a cat.", Kind::Stated, Origin::ModelClean),
        )
        .unwrap();
    let shared = Shared::open(&w.dir).unwrap();
    shared.share(&f, Audience::Everyone, &[]).unwrap();
    drop(shared);
    let conn = rusqlite::Connection::open(w.dir.join("shared.db")).unwrap();
    conn.execute(
        "INSERT INTO shared_facts (uid, text, kind, source_chat, source_from, source_to,
         learned_by, origin, model, ingested_at, audience, from_uid, from_table, shared_at)
         VALUES ('feed0001', 'From the future.', 'stated', 'c9', 0, 0, 'otto', 'owner', 'm',
         'now', 'everyone', 'x', 'a_fourth_table', 'now')",
        [],
    )
    .unwrap();
    let r = chat_start(&w.store(), &w.persona("mara"), None).unwrap();
    assert!(r.block.unwrap().text.contains("Has a cat."));
    assert_eq!(r.problems.len(), 1, "{:?}", r.problems);
    assert!(r.problems[0].contains("cannot read"), "{:?}", r.problems);
}

#[test]
fn a_memory_that_will_not_open_still_lets_the_about_me_notes_ride() {
    let w = World::new(&["mara"]);
    std::fs::write(w.dir.join("about-me.md"), "I study kelp.\n").unwrap();
    std::fs::write(w.dir.join("mara").join("memory.db"), "not a database").unwrap();
    let r = chat_start(&w.store(), &w.persona("mara"), None).unwrap();
    assert!(r.block.unwrap().text.contains("I study kelp."));
    assert_eq!(r.problems.len(), 1, "{:?}", r.problems);
    assert!(r.problems[0].starts_with("its memory"), "{:?}", r.problems);
}

#[test]
fn a_message_recalls_what_it_names_and_not_what_the_chat_already_holds() {
    let w = World::new(&["mara"]);
    let m = w.memory("mara");
    m.add_fact(
        Table::Persona,
        fact(
            "We call the kelp project Holdfast.",
            Kind::Observed,
            Origin::ModelClean,
        ),
    )
    .unwrap();
    m.add_fact(
        Table::User,
        fact("Has a cat.", Kind::Stated, Origin::ModelClean),
    )
    .unwrap();
    let p = w.persona("mara");
    let b = per_turn(
        &w.dir,
        &p,
        "How is the Holdfast work going?",
        None,
        "",
        None,
    )
    .unwrap()
    .unwrap();
    assert!(b.text.starts_with(MEMORY_STEM), "{}", b.text);
    // A run's note, never material: the projection of an old chat leaves it
    // off the wire, and a chat never counts it as its chat-start block.
    assert!(
        is_per_turn(&b.text) && !is_chat_start(&b.text),
        "{}",
        b.text
    );
    assert!(!carries_chat_start(&[Message::user(b.text.clone())]));
    assert!(b
        .text
        .contains("between you: We call the kelp project Holdfast."));
    assert!(!b.text.contains("cat"), "{}", b.text);
    assert!(crate::agent::is_harness_voice(&b.text));
    // Already in the conversation: not folded again.
    assert!(per_turn(
        &w.dir,
        &p,
        "How is the Holdfast work going?",
        None,
        "We call the kelp project Holdfast.",
        None
    )
    .unwrap()
    .is_none());
    // "ok" calls nothing up, and neither does a word too short to be a
    // message, even one that names something remembered.
    assert!(per_turn(&w.dir, &p, "ok", None, "", None)
        .unwrap()
        .is_none());
    assert!(per_turn(&w.dir, &p, "Holdfast?", None, "", None)
        .unwrap()
        .is_none());
}

#[test]
fn a_recalled_record_from_outside_arms_its_turn_and_off_is_off() {
    let w = World::new(&["mara"]);
    let m = w.memory("mara");
    let f = m
        .add_fact(
            Table::Persona,
            fact(
                "A page said Holdfast sings.",
                Kind::Observed,
                Origin::ModelUntrusted,
            ),
        )
        .unwrap();
    let p = w.persona("mara");
    assert!(
        per_turn(&w.dir, &p, "Tell me about Holdfast", None, "", None)
            .unwrap()
            .is_none()
    );
    m.approve(&f.uid).unwrap();
    let b = per_turn(&w.dir, &p, "Tell me about Holdfast", None, "", None)
        .unwrap()
        .unwrap();
    assert!(b.untrusted && b.text.starts_with(UNTRUSTED_MEMORY_STEM));

    let mut off = p.clone();
    off.settings.memory.semantic = false;
    assert!(
        per_turn(&w.dir, &off, "Tell me about Holdfast", None, "", None)
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_persona_with_no_memory_recalls_nothing_per_turn_and_creates_nothing() {
    let w = World::new(&["mara"]);
    assert!(per_turn(
        &w.dir,
        &w.persona("mara"),
        "Tell me about Holdfast",
        None,
        "",
        None
    )
    .unwrap()
    .is_none());
    assert!(!w.dir.join("mara").join("memory.db").exists());
}

#[test]
fn a_shared_fact_is_ordered_by_its_date_not_cut_first_for_being_shared() {
    let w = World::new(&["mara", "otto"]);
    let m = w.memory("mara");
    m.add_fact(
        Table::User,
        fact("Has a cat.", Kind::Stated, Origin::ModelClean),
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let otto = w.memory("otto");
    let f = otto
        .add_fact(
            Table::User,
            fact("Runs at dawn.", Kind::Stated, Origin::ModelClean),
        )
        .unwrap();
    // Shared a day later than mara learned hers, as dates go.
    let shared = Shared::open(&w.dir).unwrap();
    shared.share(&f, Audience::Everyone, &[]).unwrap();
    drop(shared);
    let conn = rusqlite::Connection::open(w.dir.join("shared.db")).unwrap();
    conn.execute(
        "UPDATE shared_facts SET shared_at = '2999-01-01T00:00:00Z'",
        [],
    )
    .unwrap();
    let t = block(&w, &w.persona("mara")).unwrap().text;
    let (dawn, cat) = (
        t.find("Runs at dawn.").unwrap(),
        t.find("Has a cat.").unwrap(),
    );
    assert!(dawn < cat, "newer first, shared or not: {t}");
}

#[test]
fn a_shared_store_that_will_not_read_costs_only_its_own_facts() {
    let w = World::new(&["mara"]);
    std::fs::write(w.dir.join("about-me.md"), "I study kelp.\n").unwrap();
    w.memory("mara")
        .add_fact(
            Table::User,
            fact("Has a cat.", Kind::Stated, Origin::ModelClean),
        )
        .unwrap();
    std::fs::write(w.dir.join("shared.db"), "not a database").unwrap();
    let r = chat_start(&w.store(), &w.persona("mara"), None).unwrap();
    let t = r.block.unwrap().text;
    assert!(
        t.contains("I study kelp.") && t.contains("Has a cat."),
        "{t}"
    );
    assert_eq!(r.problems.len(), 1, "{:?}", r.problems);
    assert!(r.problems[0].starts_with("what the owner shared"));
}

#[test]
fn a_common_word_never_arms_a_turn_untrusted() {
    let w = World::new(&["mara"]);
    let m = w.memory("mara");
    let f = m
        .add_fact(
            Table::Persona,
            fact(
                "A page said the kelp sings.",
                Kind::Observed,
                Origin::ModelUntrusted,
            ),
        )
        .unwrap();
    m.approve(&f.uid).unwrap();
    let p = w.persona("mara");
    assert!(per_turn(
        &w.dir,
        &p,
        "Thanks, that is all for the day",
        None,
        "",
        None
    )
    .unwrap()
    .is_none());
}

#[test]
fn an_episode_shown_cut_short_at_chat_start_is_not_folded_again() {
    let w = World::new(&["mara"]);
    let m = w.memory("mara");
    let summary = format!(
        "Talked about the Holdfast grant at length. {}",
        "More detail. ".repeat(40)
    );
    m.add_episode(NewEpisode {
        source: Some(Source {
            chat: "c1".into(),
            from: 0,
            to: 1,
        }),
        summary: summary.clone(),
        origin: Origin::ModelClean,
        model: "m".into(),
        ..NewEpisode::default()
    })
    .unwrap();
    // What a clipped chat-start line leaves in the conversation.
    let already: String = summary.chars().take(80).collect();
    assert!(per_turn(
        &w.dir,
        &w.persona("mara"),
        "How is the Holdfast grant?",
        None,
        &already,
        None
    )
    .unwrap()
    .is_none());
}

#[test]
fn a_turn_that_will_not_search_is_known_before_anything_is_embedded() {
    let w = World::new(&["mara"]);
    let p = w.persona("mara");
    // No store yet.
    assert!(!would_search(&w.dir, &p, "How is the Holdfast work going?"));
    w.memory("mara")
        .add_fact(
            Table::User,
            fact("Has a cat.", Kind::Stated, Origin::ModelClean),
        )
        .unwrap();
    assert!(would_search(&w.dir, &p, "How is the Holdfast work going?"));
    assert!(!would_search(&w.dir, &p, "ok thanks"), "too short");
    let mut off = p.clone();
    off.settings.memory.episodic = false;
    off.settings.memory.semantic = false;
    off.settings.memory.user_facts = UserFacts::Off;
    assert!(
        !would_search(&w.dir, &off, "How is the Holdfast work going?"),
        "all off"
    );
}

/// A fact from a late-evening chat, written by tonight's run.
fn said_late(chat: &str, text: &str) -> NewFact {
    NewFact {
        source: Source {
            chat: chat.into(),
            from: 0,
            to: 1,
        },
        ..fact(text, Kind::Stated, Origin::ModelClean)
    }
}

#[test]
fn a_record_is_dated_by_the_owners_day_of_its_chat_not_the_night_it_was_written() {
    let w = World::new(&["mara"]);
    let m = w.memory("mara");
    // 02:49 UTC on the 30th is 22:49 on the 29th in New York.
    m.add_fact(
        Table::User,
        said_late("20260930T024959-1a2b3c4d", "Is repainting the porch."),
    )
    .unwrap();
    m.add_episode(NewEpisode {
        // A stretch past the first turn has no start of its own.
        source: Some(Source {
            chat: "20260930T024959-1a2b3c4d".into(),
            from: 12,
            to: 20,
        }),
        summary: "Talked about the porch.".into(),
        origin: Origin::ModelClean,
        model: "m".into(),
        ..NewEpisode::default()
    })
    .unwrap();
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let ny: chrono_tz::Tz = "America/New_York".parse().unwrap();
    let p = w.persona("mara");

    let text = chat_start(&w.store(), &p, Some(ny))
        .unwrap()
        .block
        .unwrap()
        .text;
    assert!(
        text.contains("- 2026-09-29 · Is repainting the porch."),
        "{text}"
    );
    assert!(text.contains("- 2026-09-29 · ["), "the episode too: {text}");
    assert!(!text.contains(&today), "never the write night: {text}");

    let utc = chat_start(&w.store(), &p, Some(chrono_tz::UTC))
        .unwrap()
        .block
        .unwrap()
        .text;
    assert!(
        utc.contains("- 2026-09-30 · Is repainting the porch."),
        "{utc}"
    );

    let turn = per_turn(
        &w.dir,
        &p,
        "how is the porch coming along",
        None,
        "",
        Some(ny),
    )
    .unwrap()
    .unwrap()
    .text;
    assert!(turn.contains("2026-09-29"), "{turn}");
}

#[test]
fn a_chat_id_that_is_not_a_session_id_falls_back_to_the_write() {
    let s = |chat: &str| Source {
        chat: chat.into(),
        from: 0,
        to: 0,
    };
    assert_eq!(
        s("20260930T024959-1a2b3c4d").chat_began().as_deref(),
        Some("2026-09-30T02:49:59Z")
    );
    for odd in ["c1", "2026-09-30", "20261399T000000-x", ""] {
        assert_eq!(s(odd).chat_began(), None, "`{odd}`");
    }
    assert_eq!(
        local_day("2026-10-02T03:34:12.123Z", Some(chrono_tz::UTC)),
        "2026-10-02"
    );
    assert_eq!(local_day("2026-10-02 sometime", None), "2026-10-02");
}

#[test]
fn recent_means_recently_said_not_recently_written() {
    let w = World::new(&["mara"]);
    let m = w.memory("mara");
    // Written in the opposite order to when they were said.
    for (chat, summary) in [
        ("20261001T120000-aaaaaaaa", "The newer talk."),
        ("20260901T120000-bbbbbbbb", "The older talk."),
    ] {
        std::thread::sleep(std::time::Duration::from_millis(5));
        m.add_episode(NewEpisode {
            source: Some(Source {
                chat: chat.into(),
                from: 3,
                to: 4,
            }),
            summary: summary.into(),
            origin: Origin::ModelClean,
            model: "m".into(),
            ..NewEpisode::default()
        })
        .unwrap();
    }
    let eps = m.episodes(Filter::Recallable).unwrap();
    assert_eq!(eps[0].summary, "The newer talk.", "{eps:?}");
}

#[test]
fn a_fact_said_long_ago_but_written_tonight_does_not_crowd_out_a_newer_one() {
    let w = World::new(&["mara"]);
    let m = w.memory("mara");
    // Written newer-said first, so the write order is the wrong order.
    for (chat, text) in [
        ("20260920T150000-aaaaaaaa", "Mara keeps a tide clock."),
        (
            "20260910T150000-bbbbbbbb",
            "Mara once lived by a lighthouse.",
        ),
    ] {
        std::thread::sleep(std::time::Duration::from_millis(5));
        m.add_fact(
            Table::Persona,
            NewFact {
                source: Source {
                    chat: chat.into(),
                    from: 0,
                    to: 1,
                },
                ..fact(text, Kind::Stated, Origin::ModelClean)
            },
        )
        .unwrap();
    }
    let facts = m.facts(Table::Persona, Filter::Recallable).unwrap();
    assert_eq!(facts[0].text, "Mara keeps a tide clock.", "{facts:?}");
    let text = chat_start(&w.store(), &w.persona("mara"), Some(chrono_tz::UTC))
        .unwrap()
        .block
        .unwrap()
        .text;
    let (newer, older) = (
        text.find("tide clock").unwrap(),
        text.find("lighthouse").unwrap(),
    );
    assert!(newer < older, "the newer-said first: {text}");
}
