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
    let left: Vec<_> = shared
        .all()
        .unwrap()
        .facts
        .into_iter()
        .map(|s| s.text)
        .collect();
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
            .facts
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
    assert!(shared.all().unwrap().facts.is_empty());
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

/// Mara, a user fact she learned, and that fact shared with everyone.
fn shared_fact(dir: &Path, text: &str) -> (Memory, Shared, Fact) {
    let m = Memory::open(dir, "mara").unwrap();
    let f = m
        .add_fact(
            Table::User,
            fact(text, Kind::Stated, "c1", Origin::ModelClean),
        )
        .unwrap();
    let shared = Shared::open(dir).unwrap();
    shared.share(&f, Audience::Everyone, &[]).unwrap();
    (m, shared, f)
}

fn shared_texts(shared: &Shared) -> Vec<String> {
    let l = shared.visible_to(&[]).unwrap();
    assert_eq!(l.unreadable, 0);
    l.facts.into_iter().map(|s| s.text).collect()
}

#[test]
fn a_correction_reaches_the_shared_copy_and_forgetting_it_takes_the_copy() {
    let dir = store(&["mara"]);
    let (m, shared, old) = shared_fact(&dir, "Teaches on Tuesdays.");
    let new = m.correct(&old.uid, "Teaches on Thursdays.").unwrap();

    // Other personas read the owner's wording, never the one replaced.
    assert_eq!(shared_texts(&shared), ["Teaches on Thursdays."]);
    let copy = &shared.all().unwrap().facts[0];
    assert_eq!(copy.from_uid, new.uid);
    assert_eq!(copy.origin, Origin::Owner);

    // Forgetting the row the owner can see forgets the copy with it.
    assert_eq!(forget(&dir, "mara", &new.uid).unwrap(), 1);
    assert!(shared_texts(&shared).is_empty());
}

#[test]
fn a_copy_left_behind_by_a_correction_still_goes_when_the_fact_is_forgotten() {
    let dir = store(&["mara"]);
    let (m, shared, old) = shared_fact(&dir, "Teaches on Tuesdays.");
    let new = m.correct(&old.uid, "Teaches on Thursdays.").unwrap();
    // As if the second file's write had failed after the first committed.
    shared
        .conn
        .execute("UPDATE shared_facts SET from_uid = ?1", [&old.uid])
        .unwrap();
    let newer = m
        .correct(&new.uid, "Teaches on Thursday afternoons.")
        .unwrap();
    assert_eq!(
        forget(&dir, "mara", &newer.uid).unwrap(),
        1,
        "found through `replaces`, two corrections back"
    );
    assert!(shared.all().unwrap().facts.is_empty());
}

#[test]
fn a_withdrawn_fact_is_no_longer_shared_and_cannot_be_approved_back() {
    let dir = store(&["mara"]);
    let (m, shared, f) = shared_fact(&dir, "Runs at dawn.");
    assert_eq!(m.invalidate(&f.uid).unwrap(), 1);
    assert!(shared_texts(&shared).is_empty());
    let first = m.fact(&f.uid).unwrap().unwrap().invalidated_at.unwrap();
    // A second withdrawal keeps the first's time.
    m.invalidate(&f.uid).unwrap();
    assert_eq!(
        m.fact(&f.uid).unwrap().unwrap().invalidated_at.unwrap(),
        first
    );

    let err = m.approve(&f.uid).unwrap_err();
    assert!(err.to_string().contains("withdrawn"), "{err}");
    let after = m.fact(&f.uid).unwrap().unwrap();
    assert_eq!(after.status, Status::Invalidated);
    assert_eq!(after.invalidated_at.as_deref(), Some(first.as_str()));

    // A corrected fact's old row is withdrawn too: never both recallable.
    let old = m
        .add_fact(
            Table::User,
            fact("Has a cat.", Kind::Stated, "c1", Origin::ModelClean),
        )
        .unwrap();
    let fixed = m.correct(&old.uid, "Has two cats.").unwrap();
    assert!(m.approve(&old.uid).is_err());
    assert_eq!(
        texts(&m.facts(Table::User, Filter::Recallable).unwrap()),
        ["Has two cats."]
    );
    // And an active record has nothing to approve.
    let err = m.approve(&fixed.uid).unwrap_err();
    assert!(err.to_string().contains("already"), "{err}");
}

#[test]
fn an_unreadable_shared_row_is_counted_never_shown_and_can_still_be_unshared() {
    let dir = store(&["mara"]);
    let (_m, shared, _f) = shared_fact(&dir, "Runs at dawn.");
    for (uid, audience, table) in [
        ("feed0001", "persona:otto", "user"),
        ("feed0002", "everyone", "a_fourth_table"),
    ] {
        shared
            .conn
            .execute(
                "INSERT INTO shared_facts (uid, text, kind, source_chat, source_from, source_to,
                 learned_by, origin, model, ingested_at, audience, from_uid, from_table, shared_at)
                 VALUES (?1, 'From the future.', 'stated', 'c9', 0, 0, 'mara', 'owner', 'm',
                 'now', ?2, ?1, ?3, 'now')",
                params![uid, audience, table],
            )
            .unwrap();
    }
    let all = shared.all().unwrap();
    assert_eq!(all.facts.len(), 1);
    assert_eq!(all.unreadable, 2);
    let seen = shared.visible_to(&["otto".to_string()]).unwrap();
    assert_eq!(
        seen.facts.len(),
        1,
        "an audience it cannot read reaches no one"
    );

    // The owner can still name and remove what this binary cannot read.
    assert!(shared
        .resolve("feed")
        .unwrap_err()
        .to_string()
        .contains("matches"));
    shared
        .unshare(&shared.resolve("feed0001").unwrap())
        .unwrap();
    assert_eq!(shared.all().unwrap().unreadable, 1);
    for bad in ["", "0", "fee", "zzzz"] {
        assert!(shared.resolve(bad).is_err(), "`{bad}` resolved");
    }
}

#[test]
fn an_edit_never_creates_a_memory_just_to_find_the_id_missing() {
    let dir = store(&["mara"]);
    let err = Memory::open_to_edit(&dir, "mara").err().unwrap();
    assert!(err.to_string().contains("remembers nothing yet"), "{err}");
    assert!(forget(&dir, "mara", "abcd1234").is_err());
    assert!(!dir.join("mara").join(MEMORY_DB).exists());
}

#[test]
fn a_forget_whose_log_cannot_be_truncated_says_so_and_the_next_one_clears_it() {
    let dir = store(&["mara"]);
    let secret = "zq-held-by-a-reader-4410";
    let (m, shared, f) = shared_fact(&dir, secret);
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
    // A chat recalling on another connection, mid-read.
    let reader = Connection::open_with_flags(
        dir.join("mara").join(MEMORY_DB),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    reader.execute_batch("BEGIN").unwrap();
    let _: i64 = reader
        .query_row("SELECT count(*) FROM user_facts", [], |r| r.get(0))
        .unwrap();

    let err = forget(&dir, "mara", &f.uid).unwrap_err();
    assert!(err.to_string().contains("write-ahead log"), "{err}");
    // The deletes still all happened: the row, and its shared copy.
    assert!(m.fact(&f.uid).unwrap().is_none());
    assert!(shared.all().unwrap().facts.is_empty());

    drop(reader);
    let filler = m.facts(Table::User, Filter::All).unwrap()[0].uid.clone();
    forget(&dir, "mara", &filler).unwrap();
    for name in [MEMORY_DB, "memory.db-wal"] {
        if let Ok(bytes) = std::fs::read(dir.join("mara").join(name)) {
            assert!(
                !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
                "still in {name}"
            );
        }
    }
}

#[test]
fn a_withdrawn_fact_cannot_be_corrected_into_a_second_live_branch() {
    let dir = store(&["mara"]);
    let m = Memory::open(&dir, "mara").unwrap();
    let f = m
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
    m.correct(&f.uid, "Teaches on Thursdays.").unwrap();
    let stamp = m.fact(&f.uid).unwrap().unwrap().invalidated_at;
    let err = m.correct(&f.uid, "Teaches on Fridays.").unwrap_err();
    assert!(err.to_string().contains("withdrawn"), "{err}");
    assert_eq!(m.fact(&f.uid).unwrap().unwrap().invalidated_at, stamp);
    assert_eq!(
        texts(&m.facts(Table::User, Filter::Recallable).unwrap()),
        ["Teaches on Thursdays."]
    );
}

#[test]
fn a_store_from_before_the_ledger_upgrades_and_reads_as_never_written() {
    let dir = store(&["mara"]);
    {
        let m = Memory::open(&dir, "mara").unwrap();
        m.add_fact(
            Table::User,
            fact("Has a cat.", Kind::Stated, "c1", Origin::ModelClean),
        )
        .unwrap();
        // As every install from before the writer has it.
        m.conn
            .execute_batch("DROP TABLE written; PRAGMA user_version = 1;")
            .unwrap();
    }
    // A read-only handle does not upgrade, and the ledger reads as empty.
    let ro = Memory::open_existing(&dir, "mara").unwrap().unwrap();
    assert_eq!(ro.written_upto("c1").unwrap(), 0);
    drop(ro);
    let m = Memory::open(&dir, "mara").unwrap();
    let v: i64 = m
        .conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v, SCHEMA);
    assert_eq!(m.facts(Table::User, Filter::All).unwrap().len(), 1, "kept");
    m.write_stretch("c1", 0, 3, |_| Ok(())).unwrap().unwrap();
    assert_eq!(m.written_upto("c1").unwrap(), 3);
}

fn recall_words(m: &Memory, q: &str) -> Vec<String> {
    let kinds = [
        Recallable::Episodes,
        Recallable::Facts(Table::Persona),
        Recallable::Facts(Table::User),
        Recallable::Facts(Table::Inferred),
    ];
    m.recall_search(q, None, &kinds, &|_| false, 5)
        .unwrap()
        .0
        .into_iter()
        .map(|r| r.text)
        .collect()
}

#[test]
fn recall_finds_by_words_and_never_a_candidate_or_a_withdrawn_record() {
    let dir = store(&["mara"]);
    let m = Memory::open(&dir, "mara").unwrap();
    m.add_fact(
        Table::Persona,
        fact(
            "We call the kelp project Holdfast.",
            Kind::Observed,
            "c1",
            Origin::ModelClean,
        ),
    )
    .unwrap();
    m.add_fact(
        Table::Persona,
        fact(
            "A page said Holdfast sings.",
            Kind::Observed,
            "c1",
            Origin::ModelUntrusted,
        ),
    )
    .unwrap();
    let old = m
        .add_fact(
            Table::User,
            fact(
                "Holdfast is due in May.",
                Kind::Stated,
                "c1",
                Origin::ModelClean,
            ),
        )
        .unwrap();
    m.correct(&old.uid, "Holdfast is due in June.").unwrap();
    let found = recall_words(&m, "what about holdfast?");
    assert!(found.contains(&"We call the kelp project Holdfast.".to_string()));
    assert!(found.contains(&"Holdfast is due in June.".to_string()));
    assert!(
        !found.iter().any(|t| t.contains("sings")),
        "a candidate: {found:?}"
    );
    assert!(
        !found.iter().any(|t| t.contains("May")),
        "withdrawn: {found:?}"
    );
    assert!(recall_words(&m, "something else entirely").is_empty());
}

#[test]
fn a_store_from_before_the_index_is_indexed_when_it_upgrades() {
    let dir = store(&["mara"]);
    {
        let m = Memory::open(&dir, "mara").unwrap();
        m.add_fact(
            Table::User,
            fact(
                "Teaches a seminar on kelp.",
                Kind::Stated,
                "c1",
                Origin::ModelClean,
            ),
        )
        .unwrap();
        m.conn
            .execute_batch("DROP TABLE recall_fts; DROP TABLE vectors; PRAGMA user_version = 2;")
            .unwrap();
    }
    let ro = Memory::open_existing(&dir, "mara").unwrap().unwrap();
    assert!(
        recall_words(&ro, "kelp seminar").is_empty(),
        "no index, no recall, no error"
    );
    drop(ro);
    let m = Memory::open(&dir, "mara").unwrap();
    assert_eq!(
        recall_words(&m, "kelp seminar"),
        ["Teaches a seminar on kelp."]
    );
}

#[test]
fn meaning_needs_a_floor_and_recency_breaks_ties() {
    let dir = store(&["mara"]);
    let m = Memory::open(&dir, "mara").unwrap();
    let a = m
        .add_fact(
            Table::User,
            fact("Likes the sea.", Kind::Stated, "c1", Origin::ModelClean),
        )
        .unwrap();
    let b = m
        .add_fact(
            Table::User,
            fact("Owns a boat.", Kind::Stated, "c1", Origin::ModelClean),
        )
        .unwrap();
    m.store_vectors(&[
        (a.uid.clone(), vec![1.0, 0.0]),
        (b.uid.clone(), vec![0.0, 1.0]),
    ])
    .unwrap();
    let kinds = [Recallable::Facts(Table::User)];
    // Near `a` only: `b` is orthogonal, under the floor.
    let (found, meaning) = m
        .recall_search("zzz", Some(&[1.0, 0.1]), &kinds, &|_| false, 5)
        .unwrap();
    assert!(meaning);
    assert_eq!(
        found.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
        ["Likes the sea."]
    );
    // Equal by meaning: the newer comes first.
    let (found, _) = m
        .recall_search("zzz", Some(&[1.0, 1.0]), &kinds, &|_| false, 5)
        .unwrap();
    assert_eq!(found[0].text, "Owns a boat.", "{found:?}");
    // A vector of another model's length is never compared.
    let (found, meaning) = m
        .recall_search("zzz", Some(&[1.0, 0.0, 0.0]), &kinds, &|_| false, 5)
        .unwrap();
    assert!(!meaning && found.is_empty());
}

#[test]
fn forgetting_takes_a_record_out_of_the_index_and_its_vector_with_it() {
    let dir = store(&["mara"]);
    let m = Memory::open(&dir, "mara").unwrap();
    let f = m
        .add_fact(
            Table::User,
            fact(
                "Grows zqxjkvbw orchids.",
                Kind::Stated,
                "c1",
                Origin::ModelClean,
            ),
        )
        .unwrap();
    m.store_vectors(&[(f.uid.clone(), vec![1.0, 0.0])]).unwrap();
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
    assert!(recall_words(&m, "zqxjkvbw orchids").is_empty());
    let left: i64 = m
        .conn
        .query_row("SELECT count(*) FROM vectors", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, 0);
    for name in [MEMORY_DB, "memory.db-wal"] {
        if let Ok(bytes) = std::fs::read(dir.join("mara").join(name)) {
            assert!(
                !bytes.windows(8).any(|w| w == b"zqxjkvbw"),
                "the term survives in {name}"
            );
        }
    }
    // And forgetting a chat does the same.
    m.add_fact(
        Table::User,
        fact("Keeps bees.", Kind::Stated, "c9", Origin::ModelClean),
    )
    .unwrap();
    forget_chat(&dir, "mara", "c9").unwrap();
    assert!(recall_words(&m, "bees").is_empty());
}
