use super::super::tests_support::{new, no_lib, scratch};
use super::*;

/// A store holding personas `names`, nothing remembered yet.
fn store(names: &[&str]) -> PathBuf {
    let dir = scratch();
    for n in names {
        crate::persona::create(&dir, &no_lib(), new(n)).unwrap();
    }
    dir
}

fn src(chat: &str) -> Source {
    Source {
        chat: chat.into(),
        from: 2,
        to: 5,
    }
}

fn fact(text: &str, kind: Kind, chat: &str, origin: Origin) -> NewFact {
    NewFact {
        text: text.into(),
        kind,
        source: src(chat),
        origin,
        model: "local".into(),
        valid_from: None,
        valid_to: None,
    }
}

fn texts(facts: &[Fact]) -> Vec<&str> {
    facts.iter().map(|f| f.text.as_str()).collect()
}

#[test]
fn an_untrusted_record_waits_on_the_owner_and_keeps_its_origin_once_approved() {
    let dir = store(&["mara"]);
    let m = Memory::open(&dir, "mara").unwrap();
    let clean = m
        .add_fact(
            Table::Persona,
            fact(
                "We call it Holdfast.",
                Kind::Observed,
                "c1",
                Origin::ModelClean,
            ),
        )
        .unwrap();
    let dirty = m
        .add_fact(
            Table::Persona,
            fact(
                "A page said kelp sings.",
                Kind::Observed,
                "c1",
                Origin::ModelUntrusted,
            ),
        )
        .unwrap();
    assert_eq!(clean.status, Status::Active);
    assert_eq!(dirty.status, Status::Candidate);
    assert_eq!(
        texts(&m.facts(Table::Persona, Filter::Recallable).unwrap()),
        ["We call it Holdfast."]
    );
    assert_eq!(m.facts(Table::Persona, Filter::All).unwrap().len(), 2);

    m.approve(&dirty.uid).unwrap();
    let approved = m.fact(&dirty.uid).unwrap().unwrap();
    assert_eq!(approved.status, Status::Active);
    // Recalling it must still re-arm what it was written under.
    assert_eq!(approved.origin, Origin::ModelUntrusted);

    let ep = m
        .add_episode(NewEpisode {
            source: Some(src("c2")),
            summary: "Read a stranger's page about kelp.".into(),
            origin: Origin::ModelUntrusted,
            model: "local".into(),
            ..NewEpisode::default()
        })
        .unwrap();
    assert_eq!(ep.status, Status::Candidate);
    assert!(m.episodes(Filter::Recallable).unwrap().is_empty());
}

#[test]
fn inferred_facts_about_the_owner_live_in_their_own_table_and_nowhere_else() {
    let dir = store(&["mara"]);
    let m = Memory::open(&dir, "mara").unwrap();
    let err = m
        .add_fact(
            Table::User,
            fact("Seems stressed.", Kind::Inferred, "c1", Origin::ModelClean),
        )
        .unwrap_err();
    assert!(err.to_string().contains("inferred table"), "{err}");
    let err = m
        .add_fact(
            Table::Inferred,
            fact(
                "Teaches on Thursdays.",
                Kind::Stated,
                "c1",
                Origin::ModelClean,
            ),
        )
        .unwrap_err();
    assert!(err.to_string().contains("only inferred"), "{err}");

    let inferred = m
        .add_fact(
            Table::Inferred,
            fact("Seems stressed.", Kind::Inferred, "c1", Origin::ModelClean),
        )
        .unwrap();
    // D18: accepted automatically.
    assert_eq!(inferred.status, Status::Active);
    assert!(m.facts(Table::User, Filter::All).unwrap().is_empty());
    // A persona's own facts may be inferences; only owner facts are split.
    m.add_fact(
        Table::Persona,
        fact(
            "She likes the sea.",
            Kind::Inferred,
            "c1",
            Origin::ModelClean,
        ),
    )
    .unwrap();
}

#[test]
fn a_record_with_no_source_is_refused_by_the_api_and_by_the_schema() {
    let dir = store(&["mara"]);
    let m = Memory::open(&dir, "mara").unwrap();
    let err = m
        .add_fact(
            Table::Persona,
            fact("No home.", Kind::Observed, "  ", Origin::ModelClean),
        )
        .unwrap_err();
    assert!(err.to_string().contains("chat it came from"), "{err}");
    let mut backwards = fact("Backwards.", Kind::Observed, "c1", Origin::ModelClean);
    backwards.source.from = 9;
    assert!(m.add_fact(Table::Persona, backwards).is_err());
    assert!(m
        .add_episode(NewEpisode {
            summary: "Somewhere.".into(),
            ..NewEpisode::default()
        })
        .is_err());

    // The schema holds the line too, for any writer that skips the API.
    for t in Table::ALL {
        let raw = m.conn.execute(
            &format!(
                "INSERT INTO {} (uid, text, kind, source_chat, source_from, source_to,
                 learned_by, origin, model, ingested_at, status)
                 VALUES ('x', 't', 'stated', '', 0, 0, 'mara', 'owner', 'm', 'now', 'active')",
                t.sql()
            ),
            [],
        );
        assert!(raw.is_err(), "{} took an empty source", t.sql());
    }
}

#[test]
fn a_correction_invalidates_the_old_row_and_keeps_the_chat_it_came_from() {
    let dir = store(&["mara"]);
    let m = Memory::open(&dir, "mara").unwrap();
    let old = m
        .add_fact(
            Table::User,
            fact(
                "Teaches on Tuesdays.",
                Kind::Stated,
                "c1",
                Origin::ModelClean,
            ),
        )
        .unwrap();
    let new = m.correct(&old.uid, "Teaches on Thursdays.").unwrap();
    assert_eq!(new.origin, Origin::Owner);
    assert_eq!(new.replaces.as_deref(), Some(old.uid.as_str()));
    assert_eq!(new.source, old.source);
    assert_eq!(new.table, Table::User);

    let was = m.fact(&old.uid).unwrap().unwrap();
    assert_eq!(was.status, Status::Invalidated);
    assert!(was.invalidated_at.is_some());
    assert_eq!(was.text, "Teaches on Tuesdays.", "text is append-only");
    assert_eq!(
        texts(&m.facts(Table::User, Filter::Recallable).unwrap()),
        ["Teaches on Thursdays."]
    );

    // Forgetting the chat forgets the correction with it.
    let gone = forget_chat(&dir, "mara", "c1").unwrap();
    assert_eq!(gone.facts, 2);
    assert!(m.facts(Table::User, Filter::All).unwrap().is_empty());
}

#[test]
fn forgetting_a_chat_clears_both_files_and_nothing_else() {
    let dir = store(&["mara", "otto"]);
    std::fs::write(dir.join("groups.toml"), "[work]\n").unwrap();
    let mara = Memory::open(&dir, "mara").unwrap();
    let otto = Memory::open(&dir, "otto").unwrap();
    let shared = Shared::open(&dir).unwrap();
    let groups = vec!["work".to_string()];

    let gone = mara
        .add_fact(
            Table::User,
            fact("Runs at dawn.", Kind::Stated, "c1", Origin::ModelClean),
        )
        .unwrap();
    let kept = mara
        .add_fact(
            Table::User,
            fact("Has a cat.", Kind::Stated, "c2", Origin::ModelClean),
        )
        .unwrap();
    mara.add_episode(NewEpisode {
        source: Some(src("c1")),
        summary: "Talked about running.".into(),
        origin: Origin::ModelClean,
        model: "local".into(),
        ..NewEpisode::default()
    })
    .unwrap();
    // Otto has a chat with the same id — ids are per persona.
    let ottos = otto
        .add_fact(
            Table::User,
            fact("Likes chess.", Kind::Stated, "c1", Origin::ModelClean),
        )
        .unwrap();
    shared.share(&gone, Audience::Everyone, &groups).unwrap();
    shared
        .share(&kept, Audience::Group("work".into()), &groups)
        .unwrap();
    shared.share(&ottos, Audience::Everyone, &groups).unwrap();

    let out = forget_chat(&dir, "mara", "c1").unwrap();
    assert_eq!(
        out,
        Forgotten {
            episodes: 1,
            facts: 1,
            shared: 1
        }
    );
    assert_eq!(
        texts(&mara.facts(Table::User, Filter::All).unwrap()),
        ["Has a cat."]
    );
    assert!(mara.episodes(Filter::All).unwrap().is_empty());
    assert_eq!(otto.facts(Table::User, Filter::All).unwrap().len(), 1);
    let left: Vec<_> = shared.all().unwrap().into_iter().map(|s| s.text).collect();
    assert_eq!(left.len(), 2);
    assert!(left.contains(&"Has a cat.".to_string()));
    assert!(left.contains(&"Likes chess.".to_string()));
}

#[test]
fn forgotten_text_survives_neither_in_the_file_nor_in_the_log() {
    let dir = store(&["mara"]);
    let secret = "zq-unmistakable-secret-7731";
    let m = Memory::open(&dir, "mara").unwrap();
    let f = m
        .add_fact(
            Table::User,
            fact(secret, Kind::Stated, "c1", Origin::ModelClean),
        )
        .unwrap();
    // Padding, so the page the secret sits on is not simply dropped.
    for i in 0..20 {
        m.add_fact(
            Table::User,
            fact(
                &format!("filler {i}"),
                Kind::Stated,
                "c2",
                Origin::ModelClean,
            ),
        )
        .unwrap();
    }
    forget(&dir, "mara", &f.uid).unwrap();

    let pdir = dir.join("mara");
    let mut seen = Vec::new();
    for name in [MEMORY_DB, "memory.db-wal"] {
        if let Ok(bytes) = std::fs::read(pdir.join(name)) {
            let hit = bytes.windows(secret.len()).any(|w| w == secret.as_bytes());
            seen.push((name, hit));
        }
    }
    assert!(!seen.is_empty(), "the database was never read");
    assert!(
        seen.iter().all(|(_, hit)| !hit),
        "forgotten text still on disk: {seen:?}"
    );
}

#[test]
fn a_persona_outside_a_group_sees_none_of_its_shared_facts() {
    let dir = store(&["mara"]);
    std::fs::write(dir.join("groups.toml"), "[work]\n[everyone]\n").unwrap();
    let mara = Memory::open(&dir, "mara").unwrap();
    let shared = Shared::open(&dir).unwrap();
    let declared = vec!["work".to_string(), "everyone".to_string()];
    let mk = |t: &str| {
        mara.add_fact(Table::User, fact(t, Kind::Stated, "c1", Origin::ModelClean))
            .unwrap()
    };
    shared
        .share(&mk("For all."), Audience::Everyone, &declared)
        .unwrap();
    shared
        .share(&mk("For work."), Audience::Group("work".into()), &declared)
        .unwrap();
    // A group *named* everyone is still a group, not everybody.
    shared
        .share(
            &mk("For the group called everyone."),
            Audience::Group("everyone".into()),
            &declared,
        )
        .unwrap();

    let seen = |groups: &[&str]| {
        let groups: Vec<String> = groups.iter().map(|g| g.to_string()).collect();
        let mut t: Vec<String> = shared
            .visible_to(&groups)
            .unwrap()
            .into_iter()
            .map(|s| s.text)
            .collect();
        t.sort();
        t
    };
    assert_eq!(seen(&[]), ["For all."]);
    assert_eq!(seen(&["work"]), ["For all.", "For work."]);
    assert_eq!(
        seen(&["everyone"]),
        ["For all.", "For the group called everyone."]
    );
    assert_eq!(seen(&["friends"]), ["For all."]);
}

#[test]
fn only_an_approved_fact_about_the_owner_can_be_shared_and_only_into_a_real_group() {
    let dir = store(&["mara"]);
    let m = Memory::open(&dir, "mara").unwrap();
    let shared = Shared::open(&dir).unwrap();
    let declared = vec!["work".to_string()];

    let own = m
        .add_fact(
            Table::Persona,
            fact(
                "She grew up on the coast.",
                Kind::Observed,
                "c1",
                Origin::ModelClean,
            ),
        )
        .unwrap();
    assert!(shared.share(&own, Audience::Everyone, &declared).is_err());

    let candidate = m
        .add_fact(
            Table::User,
            fact("Lives in Ohio.", Kind::Stated, "c1", Origin::ModelUntrusted),
        )
        .unwrap();
    let err = shared
        .share(&candidate, Audience::Everyone, &declared)
        .unwrap_err();
    assert!(err.to_string().contains("approve"), "{err}");

    let ok = m
        .add_fact(
            Table::Inferred,
            fact(
                "Likes early starts.",
                Kind::Inferred,
                "c1",
                Origin::ModelClean,
            ),
        )
        .unwrap();
    let err = shared
        .share(&ok, Audience::Group("friends".into()), &declared)
        .unwrap_err();
    assert!(
        err.to_string().contains("no group named `friends`"),
        "{err}"
    );
    let copy = shared.share(&ok, Audience::Everyone, &declared).unwrap();
    assert_eq!(copy.source, ok.source);
    assert_eq!(copy.learned_by, "mara");
    assert_eq!(copy.from_table, Table::Inferred);
    assert!(shared.share(&ok, Audience::Everyone, &declared).is_err());

    // Forgetting the original takes its copies with it.
    assert_eq!(forget(&dir, "mara", &ok.uid).unwrap(), 1);
    assert!(shared.all().unwrap().is_empty());
}

#[test]
fn reading_memory_never_creates_it_and_never_writes() {
    let dir = store(&["mara"]);
    assert!(Memory::open_existing(&dir, "mara").unwrap().is_none());
    assert!(Shared::open_existing(&dir).unwrap().is_none());
    assert!(!dir.join("mara").join(MEMORY_DB).exists());
    assert!(!dir.join(SHARED_DB).exists());
    assert_eq!(
        forget_chat(&dir, "mara", "c1").unwrap(),
        Forgotten::default()
    );
    assert!(!dir.join("mara").join(MEMORY_DB).exists());

    {
        let m = Memory::open(&dir, "mara").unwrap();
        m.add_fact(
            Table::User,
            fact("Has a cat.", Kind::Stated, "c1", Origin::ModelClean),
        )
        .unwrap();
    }
    let ro = Memory::open_existing(&dir, "mara").unwrap().unwrap();
    assert_eq!(ro.facts(Table::User, Filter::Recallable).unwrap().len(), 1);
    let err = ro
        .add_fact(
            Table::User,
            fact("Also a dog.", Kind::Stated, "c1", Origin::ModelClean),
        )
        .unwrap_err();
    assert!(err.to_string().contains("read-only"), "{err}");
}

#[test]
fn memory_is_owner_only_and_refuses_a_persona_that_is_not_there() {
    let dir = store(&["mara"]);
    assert!(Memory::open(&dir, "ghost").is_err());
    assert!(Memory::open(&dir, "../mara").is_err());
    Memory::open(&dir, "mara").unwrap();
    Shared::open(&dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for p in [dir.join("mara").join(MEMORY_DB), dir.join(SHARED_DB)] {
            let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{}", p.display());
        }
    }
}

#[test]
fn a_value_this_binary_cannot_read_narrows_rather_than_failing_the_row() {
    let dir = store(&["mara"]);
    let m = Memory::open(&dir, "mara").unwrap();
    let f = m
        .add_fact(
            Table::User,
            fact("Has a cat.", Kind::Stated, "c1", Origin::Owner),
        )
        .unwrap();
    m.conn
        .execute(
            "UPDATE user_facts SET origin = 'from_the_future', status = 'glowing',
             kind = 'dreamt' WHERE uid = ?1",
            [&f.uid],
        )
        .unwrap();
    let read = m.fact(&f.uid).unwrap().unwrap();
    assert_eq!(
        read.origin,
        Origin::ModelUntrusted,
        "unknown is never clean"
    );
    assert_eq!(read.status, Status::Candidate, "unknown is never recalled");
    assert_eq!(read.kind, Kind::Inferred);
    assert!(m.facts(Table::User, Filter::Recallable).unwrap().is_empty());
}

#[test]
fn pins_come_first_and_the_export_is_stable() {
    let dir = store(&["mara"]);
    let m = Memory::open(&dir, "mara").unwrap();
    let a = m
        .add_fact(
            Table::Persona,
            fact("First.", Kind::Observed, "c1", Origin::ModelClean),
        )
        .unwrap();
    m.add_fact(
        Table::Persona,
        fact("Second.", Kind::Observed, "c1", Origin::ModelClean),
    )
    .unwrap();
    m.pin(&a.uid, true).unwrap();
    assert_eq!(
        texts(&m.facts(Table::Persona, Filter::All).unwrap())[0],
        "First."
    );
    let one = m.export().unwrap();
    assert_eq!(one, m.export().unwrap());
    assert_eq!(one.lines().count(), 2);
    assert!(m.invalidate("no-such-uid").is_err());
}

#[test]
fn a_short_id_resolves_only_when_it_names_one_record() {
    let dir = store(&["mara"]);
    let m = Memory::open(&dir, "mara").unwrap();
    let f = m
        .add_fact(
            Table::User,
            fact("Has a cat.", Kind::Stated, "c1", Origin::ModelClean),
        )
        .unwrap();
    assert_eq!(m.resolve(&f.uid[..8].to_uppercase()).unwrap(), f.uid);
    assert!(m.resolve("abc").is_err(), "too short");
    assert!(m.resolve("zzzz").is_err(), "not hex");
    assert!(m.resolve("%%%%").is_err(), "no pattern reaches SQL");
    // Two records sharing a prefix: forced, since real ids are random.
    m.conn
        .execute(
            "UPDATE user_facts SET uid = 'abcd0001' WHERE uid = ?1",
            [&f.uid],
        )
        .unwrap();
    m.add_fact(
        Table::Persona,
        fact("Other.", Kind::Observed, "c1", Origin::ModelClean),
    )
    .unwrap();
    m.conn
        .execute("UPDATE facts SET uid = 'abcd0002'", [])
        .unwrap();
    let err = m.resolve("abcd").unwrap_err();
    assert!(err.to_string().contains("matches 2"), "{err}");
    assert_eq!(m.resolve("abcd0002").unwrap(), "abcd0002");
}
