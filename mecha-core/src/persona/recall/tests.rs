use super::super::memory::{Audience, Kind, NewEpisode, NewFact, Source};
use super::super::tests_support::{fill_core, new, no_lib, scratch};
use super::*;
use crate::agent::Taint;
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
    chat_start(&w.store(), p).unwrap()
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
fn the_block_rides_once_before_the_first_reply_and_a_typed_stem_only_arms_more() {
    let block = Message::user(format!("{MEMORY_STEM} — notes.)"));
    assert!(carries_now(&[Message::user("hi")]));
    assert!(!carries_now(std::slice::from_ref(&block)));
    assert!(!carries_now(&[
        Message::user("hi"),
        Message::assistant(vec![Block::text("hello")])
    ]));
    // In the persona's own words it is not the harness's block.
    assert!(!carries(&[Message::assistant(vec![Block::text(
        MEMORY_STEM
    )])]));
    // The owner typing the clean stem arms private — never disarms anything.
    let mut t = Taint {
        private: false,
        untrusted: true,
    };
    t.arm_for_content(&[Message::user(MEMORY_STEM)]);
    assert!(t.untrusted && t.private);
}
