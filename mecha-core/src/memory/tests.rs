use super::*;
use std::{collections::BTreeSet, path::PathBuf};

struct Fixture {
    dir: PathBuf,
    db: Memory,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("mecha-memory-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let db = Memory::create(&dir.join("fixture.memory-prototype.sqlite"), "test").unwrap();
        Self { dir, db }
    }
    fn path(&self) -> PathBuf {
        self.dir.join("fixture.memory-prototype.sqlite")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn scopes(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}
fn access(names: &[&str]) -> Access {
    Access {
        domain: "test".into(),
        scopes: scopes(names),
    }
}
fn capture(text: &str) -> Capture {
    Capture {
        source: SourceKey {
            connector: "fixture".into(),
            account: "synthetic".into(),
            id: "message-1".into(),
        },
        text: text.into(),
        occurred_at: Some("2026-06-01T09:00:00-04:00".into()),
        scopes: scopes(&["mail"]),
        transcript: None,
    }
}
fn relation(db: &mut Memory) -> RelationInput {
    let ep = db
        .capture(capture("Person A works at Organization B."))
        .unwrap();
    let a = db.entity("Person A", BTreeSet::new()).unwrap();
    let b = db.entity("Organization B", BTreeSet::new()).unwrap();
    RelationInput {
        predicate: "works_at".into(),
        subject: a,
        object: Target::Entity(b),
        support: Span {
            evidence: ep,
            start: 0,
            end: 32,
        },
        consulted: vec![],
        text: "Person A works at Organization B".into(),
        polarity: 1,
        valid: Interval {
            from: Bound::Known("2026-05-01T00:00:00Z".into()),
            to: Bound::Unbounded,
        },
    }
}
fn query(db: &Memory, text: &str) -> Query {
    Query {
        text: text.into(),
        known_at: db.snapshot().unwrap(),
        valid_at: None,
        include_possible: false,
        limit: 10,
    }
}

#[test]
fn capture_retries_and_real_return_to_old_content_have_different_history() {
    let mut f = Fixture::new();
    let a = f.db.capture(capture("alpha")).unwrap();
    assert_eq!(a, f.db.capture(capture("alpha")).unwrap());
    let before = f.db.snapshot().unwrap();
    let b = f.db.capture(capture("beta")).unwrap();
    let a2 = f.db.capture(capture("alpha")).unwrap();
    assert_eq!(a.object, b.object);
    assert_eq!(a.object, a2.object);
    assert_ne!(a.revision, a2.revision);
    assert_eq!(f.db.snapshot().unwrap().sequence, before.sequence + 2);
    let db = Memory::open(&f.path()).unwrap();
    assert!(
        matches!(db.read(&a,&access(&["mail"])).unwrap(),Some(Record::Evidence(e)) if e.text=="alpha")
    );
}

#[test]
fn opener_refuses_unrelated_existing_databases_without_mutating_them() {
    let f = Fixture::new();
    let foreign = f.dir.join("foreign.memory-prototype.sqlite");
    std::fs::write(&foreign, b"do not overwrite").unwrap();
    assert!(Memory::create(&foreign, "test").is_err());
    assert!(Memory::open(&foreign).is_err());
    assert_eq!(std::fs::read(&foreign).unwrap(), b"do not overwrite");
    assert!(Memory::create(&f.dir.join("graph.db"), "test").is_err());
    let home = f.dir.join(".mecha");
    std::fs::create_dir(&home).unwrap();
    assert!(Memory::create(&home.join("bad.memory-prototype.sqlite"), "test").is_err());
}

/// The one place under a mecha home a store may live is `memory/store`; every
/// other directory beneath `.mecha` — and any path through `.mecha-graph` —
/// is still refused.
#[test]
fn a_mecha_home_holds_a_store_only_in_its_memory_store_directory() {
    let f = Fixture::new();
    let home = f.dir.join(".mecha");
    let store = home.join("memory").join("store");
    std::fs::create_dir_all(&store).unwrap();
    assert!(Memory::create(&store.join("agent.memory-prototype.sqlite"), "test").is_ok());
    for refused in [
        home.join("memory"),
        home.join("memory").join("store").join("deeper"),
        home.join("sessions").join("memory").join("store"),
        f.dir
            .join(".mecha-graph")
            .join(".mecha")
            .join("memory")
            .join("store"),
        home.join(".mecha").join("memory").join("store"),
    ] {
        std::fs::create_dir_all(&refused).unwrap();
        assert!(
            Memory::create(&refused.join("bad.memory-prototype.sqlite"), "test").is_err(),
            "{} must not hold a store",
            refused.display()
        );
    }
}

#[test]
fn ended_and_retracted_relations_have_distinct_temporal_answers() {
    let mut f = Fixture::new();
    let mut r = relation(&mut f.db);
    let first = f.db.relate(r.clone()).unwrap();
    let old = f.db.snapshot().unwrap();
    r.valid.to = Bound::Known("2026-06-30T00:00:00Z".into());
    let closed = f.db.revise_relation(&first, r).unwrap();
    let mut q = query(&f.db, "works");
    q.valid_at = Some("2026-05-15T00:00:00Z".into());
    assert_eq!(
        f.db.search(&q, &access(&["mail"])).unwrap()[0].reference,
        closed
    );
    q.valid_at = Some("2026-06-30T00:00:00Z".into());
    assert!(f.db.search(&q, &access(&["mail"])).unwrap().is_empty());
    q.known_at = old.clone();
    assert_eq!(
        f.db.search(&q, &access(&["mail"])).unwrap()[0].reference,
        first
    );
    f.db.withdraw(&closed).unwrap();
    q.known_at = f.db.snapshot().unwrap();
    q.valid_at = Some("2026-05-15T00:00:00Z".into());
    assert!(f.db.search(&q, &access(&["mail"])).unwrap().is_empty());
    q.known_at = old;
    assert_eq!(f.db.search(&q, &access(&["mail"])).unwrap().len(), 1);
}

#[test]
fn uncertain_dates_are_possible_not_definite_and_bad_dates_are_rejected() {
    let mut f = Fixture::new();
    let mut r = relation(&mut f.db);
    r.valid.from = Bound::Unknown;
    f.db.relate(r.clone()).unwrap();
    let mut q = query(&f.db, "works");
    q.valid_at = Some("2026-05-15T00:00:00Z".into());
    assert!(f.db.search(&q, &access(&["mail"])).unwrap().is_empty());
    q.include_possible = true;
    assert!(f.db.search(&q, &access(&["mail"])).unwrap()[0].possible_time);
    r.valid.from = Bound::Known("not a date".into());
    assert!(f.db.relate(r.clone()).is_err());
    r.valid.from = Bound::Known("2026-05-01T00:00:00Z".into());
    r.valid.to = r.valid.from.clone();
    assert!(f.db.relate(r).is_err());
}

#[test]
fn grounding_checks_utf8_spans_binding_types_and_expected_heads() {
    let mut f = Fixture::new();
    let r = relation(&mut f.db);
    let first = f.db.relate(r.clone()).unwrap();
    f.db.revise_relation(&first, r.clone()).unwrap();
    assert!(f.db.revise_relation(&first, r.clone()).is_err());
    let mut bad = r.clone();
    bad.support.end = usize::MAX;
    assert!(f.db.relate(bad).is_err());
    let mut bad = r.clone();
    bad.subject = r.support.evidence.clone();
    assert!(f.db.relate(bad).is_err());
    let mut unicode = capture("éclair");
    unicode.source.id = "unicode".into();
    let ep = f.db.capture(unicode).unwrap();
    let mut bad = r;
    bad.support = Span {
        evidence: ep,
        start: 1,
        end: 3,
    };
    assert!(f.db.relate(bad).is_err());
}

#[test]
fn access_filtering_precedes_limit_and_all_outputs_remain_untrusted() {
    let mut f = Fixture::new();
    let r = relation(&mut f.db);
    let id = f.db.relate(r).unwrap();
    for i in 0..20 {
        f.db.entity(&format!("works secret {i}"), scopes(&["secret"]))
            .unwrap();
    }
    let mut q = query(&f.db, "works");
    q.limit = 1;
    let hits = f.db.search(&q, &access(&["mail"])).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].reference, id);
    assert!(hits[0].untrusted_input && hits[0].private_data);
    assert!(f.db.read(&id, &access(&[])).unwrap().is_none());
    let mut wrong = access(&["mail"]);
    wrong.domain = "other".into();
    assert!(f.db.search(&q, &wrong).is_err());
}

#[test]
fn source_edits_stale_current_claims_without_erasing_old_knowledge() {
    let mut f = Fixture::new();
    let r = relation(&mut f.db);
    f.db.relate(r).unwrap();
    let old = f.db.snapshot().unwrap();
    f.db.capture(capture("The affiliation was mistaken."))
        .unwrap();
    let mut q = query(&f.db, "works");
    assert!(f.db.search(&q, &access(&["mail"])).unwrap().is_empty());
    q.known_at = old;
    assert_eq!(f.db.search(&q, &access(&["mail"])).unwrap().len(), 2);
}

#[test]
fn changed_source_scopes_restrict_even_historical_reads() {
    let mut f = Fixture::new();
    let r = relation(&mut f.db);
    let id = f.db.relate(r).unwrap();
    let q = query(&f.db, "works");
    let mut tighter = capture("Person A works at Organization B.");
    tighter.scopes = scopes(&["secret"]);
    f.db.capture(tighter).unwrap();
    assert!(f.db.read(&id, &access(&["mail"])).unwrap().is_none());
    assert!(f.db.search(&q, &access(&["mail"])).unwrap().is_empty());
}

#[test]
fn forgetting_purges_transitive_history_and_search_and_survives_restart() {
    let mut f = Fixture::new();
    let r = relation(&mut f.db);
    let root = r.support.evidence.clone();
    let claim = f.db.relate(r.clone()).unwrap();
    let mut other = capture("Independent supporting observation.");
    other.source.id = "second".into();
    let second = f.db.capture(other).unwrap();
    let mut derived = r;
    derived.support = Span {
        evidence: second,
        start: 0,
        end: 11,
    };
    derived.consulted = vec![claim.clone()];
    let dependent = f.db.relate(derived).unwrap();
    let q = query(&f.db, "works");
    let report = f.db.forget(&capture("").source).unwrap();
    assert_eq!(report.objects, 3);
    assert_eq!(report.revisions, 3);
    let mut reopened = Memory::open(&f.path()).unwrap();
    for id in [root, claim, dependent] {
        assert!(reopened.read(&id, &access(&["mail"])).unwrap().is_none());
    }
    assert!(reopened.search(&q, &access(&["mail"])).unwrap().is_empty());
    assert!(reopened
        .capture(capture("Person A works at Organization B."))
        .is_err());
    assert_eq!(reopened.forget(&capture("").source).unwrap().objects, 0);
    let bytes = std::fs::read(f.path()).unwrap();
    assert!(!bytes
        .windows(b"Person A works".len())
        .any(|w| w == b"Person A works"));
}

#[test]
fn snapshots_cannot_be_used_across_stores_or_from_the_future() {
    let mut f = Fixture::new();
    let other = Fixture::new();
    f.db.capture(capture("alpha")).unwrap();
    let mut q = query(&f.db, "alpha");
    q.known_at = other.db.snapshot().unwrap();
    assert!(f.db.search(&q, &access(&["mail"])).is_err());
    q.known_at = f.db.snapshot().unwrap();
    q.known_at.sequence += 1;
    assert!(f.db.search(&q, &access(&["mail"])).is_err());
}

#[test]
fn revision_cannot_depend_transitively_on_its_own_old_version() {
    let mut f = Fixture::new();
    let input = relation(&mut f.db);
    let a = f.db.relate(input.clone()).unwrap();
    let mut b_input = input.clone();
    b_input.consulted = vec![a.clone()];
    let b = f.db.relate(b_input).unwrap();
    let before = f.db.snapshot().unwrap();
    let mut revised = input;
    revised.consulted = vec![b.clone()];
    assert!(f.db.revise_relation(&a, revised).is_err());
    assert_eq!(f.db.snapshot().unwrap().sequence, before.sequence);
    let hits =
        f.db.search(&query(&f.db, "works"), &access(&["mail"]))
            .unwrap();
    assert!(hits.iter().any(|h| h.reference == a));
    assert!(hits.iter().any(|h| h.reference == b));
}

#[test]
fn scopes_from_entities_and_consulted_claims_cannot_be_lost() {
    let mut f = Fixture::new();
    let mut input = relation(&mut f.db);
    input.subject =
        f.db.entity("Restricted person", scopes(&["people"]))
            .unwrap();
    let private_claim = f.db.relate(input).unwrap();
    let mut derived = relation(&mut f.db);
    derived.consulted = vec![private_claim];
    let id = f.db.relate(derived).unwrap();
    assert!(f.db.read(&id, &access(&["mail"])).unwrap().is_none());
    assert!(f.db.read(&id, &access(&["people"])).unwrap().is_none());
    assert!(f
        .db
        .read(&id, &access(&["mail", "people"]))
        .unwrap()
        .is_some());
    let hits =
        f.db.search(&query(&f.db, "works"), &access(&["mail", "people"]))
            .unwrap();
    let hit = hits.iter().find(|h| h.reference == id).unwrap();
    assert_eq!(hit.required_scopes, scopes(&["mail", "people"]));
}

#[test]
fn concurrent_revision_writers_cannot_both_replace_the_same_head() {
    use std::sync::{Arc, Barrier};
    let mut f = Fixture::new();
    let input = relation(&mut f.db);
    let original = f.db.relate(input.clone()).unwrap();
    let before = f.db.snapshot().unwrap().sequence;
    let barrier = Arc::new(Barrier::new(2));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let path = f.path();
        let expected = original.clone();
        let value = input.clone();
        let ready = barrier.clone();
        threads.push(std::thread::spawn(move || {
            let mut memory = Memory::open(&path).unwrap();
            ready.wait();
            memory.revise_relation(&expected, value).is_ok()
        }));
    }
    let winners = threads
        .into_iter()
        .map(|t| usize::from(t.join().unwrap()))
        .sum::<usize>();
    assert_eq!(winners, 1);
    assert_eq!(f.db.snapshot().unwrap().sequence, before + 1);
}
