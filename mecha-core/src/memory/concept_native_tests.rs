use super::*;
use crate::memory::host_capture::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
struct Guard(Arc<AtomicBool>);
impl ExternalSourceGuard for Guard {
    fn check(&self, _: &ExternalBinding) -> SourceStatus {
        if self.0.load(Ordering::SeqCst) {
            SourceStatus::Checked
        } else {
            SourceStatus::Withheld
        }
    }
}
struct Fixture {
    _dir: std::path::PathBuf,
    path: std::path::PathBuf,
    db: Memory,
    live: Arc<AtomicBool>,
    access: Access,
}
impl Fixture {
    fn new() -> Self {
        let dir =
            std::env::temp_dir().join(format!("native-concept-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("native.memory-prototype.sqlite");
        let live = Arc::new(AtomicBool::new(true));
        let db = Memory::create_situated(&path, "test")
            .unwrap()
            .with_source_guard(Arc::new(Guard(live.clone())));
        Self {
            _dir: dir,
            path,
            db,
            live,
            access: Access {
                domain: "test".into(),
                scopes: BTreeSet::new(),
            },
        }
    }
    fn capture(
        &mut self,
        session: &str,
        id: &str,
        text: &str,
        consulted: Vec<RevisionRef>,
    ) -> CaptureReceipt {
        self.db
            .admit_host_capture(input(session, id, text, consulted), &self.access)
            .unwrap()
    }
}
fn input(session: &str, id: &str, text: &str, consulted: Vec<RevisionRef>) -> HostCapture {
    HostCapture {
        identity: CaptureIdentity {
            domain: "test".into(),
            account: "test".into(),
            session_id: session.into(),
            prefix_digest: "a".repeat(64),
            check_id: "b".repeat(64),
            check_version: id.into(),
            evaluator_version: "v1".into(),
        },
        binding: ExternalBinding {
            authority: "host-v1".into(),
            locator: "c".repeat(64),
            revision: "d".repeat(64),
        },
        criterion: ConfirmedCriterion {
            goal: "charter:1".into(),
            goal_text: "Choose a route".into(),
            criterion_text: "Reach the destination".into(),
            criterion_revision: "e".repeat(64),
            confirmation_digest: "f".repeat(64),
        },
        originals: vec![OriginalPassage {
            source: SourceKey {
                connector: "bridge".into(),
                account: "test".into(),
                id: id.into(),
            },
            kind: PassageKind::UserRequest,
            line: 1,
            pointer: "/content/0/text".into(),
            text: text.into(),
            occurred_at: None,
        }],
        receipts: vec![ToolReceipt {
            call_id: "call1".into(),
            tool_name: "fs_write".into(),
            status: ReceiptStatus::Succeeded,
        }],
        artifact: ArtifactCheck {
            artifact_digest: None,
            verdict: CheckVerdict::Failed,
        },
        consulted,
        scopes: BTreeSet::new(),
        occurred_at: None,
    }
}
fn selection(r: &RevisionRef, text: &str) -> Value {
    json!({"source_id":r.revision,"field":"/content/0/text","quote":text})
}
fn draft(refs: &[RevisionRef]) -> Value {
    json!({"name":"Pending approval", "principle":"Keep an alternative while approval is pending.","applicability":"Ordinary permit cases.","support":[selection(&refs[0],"permit pending"),selection(&refs[1],"inspection pending")],"counterexamples":[]})
}
fn sources(f: &mut Fixture) -> Vec<RevisionRef> {
    vec![
        f.capture("one", "one", "permit pending", vec![]).originals[0].clone(),
        f.capture("two", "two", "inspection pending", vec![])
            .originals[0]
            .clone(),
    ]
}
#[test]
fn native_concepts_retain_independent_host_episodes() {
    let mut f = Fixture::new();
    let refs = sources(&mut f);
    let request =
        f.db.prepare_native_concept(&refs, &f.db.snapshot().unwrap(), &f.access)
            .unwrap();
    let concept =
        f.db.retain_native_concept(request, &draft(&refs).to_string(), &f.access, None)
            .unwrap();
    assert!(matches!(
        f.db.read(&concept, &f.access).unwrap(),
        Some(Record::Concept(_))
    ));
}

fn retain(f: &mut Fixture, refs: &[RevisionRef]) -> RevisionRef {
    let request =
        f.db.prepare_native_concept(refs, &f.db.snapshot().unwrap(), &f.access)
            .unwrap();
    f.db.retain_native_concept(request, &draft(refs).to_string(), &f.access, None)
        .unwrap()
}
fn opposition(f: &mut Fixture) -> ConceptExcerpt {
    ConceptExcerpt {
        evidence: f
            .capture("exception", "exception", "emergency waiver", vec![])
            .originals[0]
            .clone(),
        field: "/content/0/text".into(),
        start: 0,
        end: 16,
        quote: "emergency waiver".into(),
    }
}
fn revised(refs: &[RevisionRef], e: &ConceptExcerpt) -> String {
    let mut d = draft(refs);
    d["counterexamples"] = json!([selection(&e.evidence, &e.quote)]);
    json!({"outcome":"revised_provisional","draft":d}).to_string()
}
#[test]
fn native_concepts_preserve_legacy_refusal_and_reject_same_session_and_projection() {
    let mut f = Fixture::new();
    let refs = sources(&mut f);
    assert!(f
        .db
        .prepare_concept(&refs, &f.db.snapshot().unwrap(), &f.access)
        .is_err());
    let again = f.capture("one", "other-prefix", "inspection pending", vec![]);
    assert!(f
        .db
        .prepare_native_concept(
            &[refs[0].clone(), again.originals[0].clone()],
            &f.db.snapshot().unwrap(),
            &f.access
        )
        .is_err());
    assert!(f
        .db
        .prepare_native_concept(
            &[refs[0].clone(), again.projection],
            &f.db.snapshot().unwrap(),
            &f.access
        )
        .is_err());
    let request =
        f.db.prepare_native_concept(&refs, &f.db.snapshot().unwrap(), &f.access)
            .unwrap();
    assert!(request.generation().prompt.contains("UserRequest"));
    assert!(f
        .db
        .retain_concept(request, &draft(&refs).to_string(), &f.access, None)
        .is_err());
    let unguarded = Memory::open(&f.path).unwrap();
    assert!(unguarded
        .prepare_native_concept(&refs, &unguarded.snapshot().unwrap(), &f.access)
        .is_err());
}
#[test]
fn native_concepts_recheck_uncited_sources_and_transitive_scope_changes() {
    for mutation in ["guard", "forget", "scope", "withdraw", "metadata"] {
        let mut f = Fixture::new();
        let mut refs = sources(&mut f);
        let prior = f.capture("ancestor", "ancestor", "earlier context", vec![]);
        let uncited = f.capture(
            "uncited",
            "uncited",
            "clear weather",
            vec![prior.projection.clone()],
        );
        refs.push(uncited.originals[0].clone());
        f.access.scopes.insert("extra".into());
        let request =
            f.db.prepare_native_concept(&refs, &f.db.snapshot().unwrap(), &f.access)
                .unwrap();
        match mutation {
            "guard" => f.live.store(false, Ordering::SeqCst),
            "forget" => {
                f.db.forget(&SourceKey {
                    connector: "bridge".into(),
                    account: "test".into(),
                    id: "ancestor".into(),
                })
                .unwrap();
            }
            "scope" => {
                f.db.conn
                    .execute(
                        "INSERT INTO scope VALUES (?1,'extra')",
                        [&prior.projection.revision],
                    )
                    .unwrap();
            }
            "withdraw" => {
                f.db.conn
                    .execute(
                        "UPDATE revision SET lifecycle='withdrawn' WHERE id=?1",
                        [&prior.projection.revision],
                    )
                    .unwrap();
            }
            "metadata" => {
                f.db.conn.execute("UPDATE host_capture_receipt SET payload_hash=?1 WHERE projection_revision=?2",rusqlite::params!["a".repeat(64),uncited.projection.revision]).unwrap();
            }
            _ => unreachable!(),
        }
        let before = sequence(&f.db.conn).unwrap();
        assert!(
            f.db.retain_native_concept(request, &draft(&refs).to_string(), &f.access, None)
                .is_err(),
            "{mutation}"
        );
        assert_eq!(sequence(&f.db.conn).unwrap(), before);
    }
}
#[test]
fn native_concepts_retain_metadata_dependencies_and_forget_after_reopen() {
    let mut f = Fixture::new();
    let mut refs = sources(&mut f);
    let extra = f.capture("uncited", "uncited", "clear weather", vec![]);
    refs.push(extra.originals[0].clone());
    let concept = retain(&mut f, &refs);
    assert!(closure(&f.db.conn, &concept)
        .unwrap()
        .contains(&extra.projection));
    let mut reopened = Memory::open(&f.path)
        .unwrap()
        .with_source_guard(Arc::new(Guard(f.live.clone())));
    assert!(reopened.read(&concept, &f.access).unwrap().is_some());
    reopened
        .forget(&SourceKey {
            connector: "bridge".into(),
            account: "test".into(),
            id: "uncited".into(),
        })
        .unwrap();
    assert!(reopened.read(&concept, &f.access).unwrap().is_none());
    assert!(Memory::open(&f.path)
        .unwrap()
        .read(&concept, &f.access)
        .unwrap()
        .is_none());
}
#[test]
fn native_rumination_preserves_opposition_and_concept_only_scopes() {
    let mut f = Fixture::new();
    let refs = sources(&mut f);
    let prior = retain(&mut f, &refs);
    let counter = opposition(&mut f);
    f.db.conn
        .execute("INSERT INTO scope VALUES (?1,'extra')", [&prior.revision])
        .unwrap();
    f.access.scopes.insert("extra".into());
    let request =
        f.db.prepare_native_concept_rumination(
            &prior,
            std::slice::from_ref(&counter),
            &f.db.snapshot().unwrap(),
            &f.access,
        )
        .unwrap();
    assert!(f
        .db
        .finish_native_concept_rumination(
            request,
            &json!({"outcome":"revised_provisional","draft":draft(&refs)}).to_string(),
            &f.access
        )
        .is_err());
    let request =
        f.db.prepare_native_concept_rumination(
            &prior,
            std::slice::from_ref(&counter),
            &f.db.snapshot().unwrap(),
            &f.access,
        )
        .unwrap();
    let rumination::RuminationOutcome::RevisedProvisional { reference } =
        f.db.finish_native_concept_rumination(request, &revised(&refs, &counter), &f.access)
            .unwrap()
    else {
        panic!()
    };
    assert!(required_scopes(&f.db.conn, &reference.revision)
        .unwrap()
        .contains("extra"));
    let Some(Record::Concept(c)) = f.db.read(&reference, &f.access).unwrap() else {
        panic!()
    };
    assert_eq!(c.counterexamples[0].quote, "emergency waiver");
    assert_eq!(c.consulted.len(), 3);
    assert!(f
        .db
        .prepare_concept_rumination(&reference, &[], &f.db.snapshot().unwrap(), &f.access)
        .is_err());
}
#[test]
fn every_native_rumination_outcome_rechecks_source_authority_and_closure_scopes() {
    for outcome in ["unchanged", "abstained", "revised_provisional"] {
        for mutation in ["guard", "scope", "forget", "previous_scope", "new_source"] {
            let mut f = Fixture::new();
            let refs = sources(&mut f);
            let prior = retain(&mut f, &refs);
            let counter = opposition(&mut f);
            f.access.scopes.insert("extra".into());
            let request =
                f.db.prepare_native_concept_rumination(
                    &prior,
                    std::slice::from_ref(&counter),
                    &f.db.snapshot().unwrap(),
                    &f.access,
                )
                .unwrap();
            match mutation {
                "guard" => f.live.store(false, Ordering::SeqCst),
                "scope" => {
                    f.db.conn
                        .execute("INSERT INTO scope VALUES (?1,'extra')", [&refs[0].revision])
                        .unwrap();
                }
                "forget" => {
                    f.db.forget(&SourceKey {
                        connector: "bridge".into(),
                        account: "test".into(),
                        id: "one".into(),
                    })
                    .unwrap();
                }
                "previous_scope" => {
                    f.db.conn
                        .execute("INSERT INTO scope VALUES (?1,'extra')", [&prior.revision])
                        .unwrap();
                }
                "new_source" => {
                    f.db.conn
                        .execute(
                            "UPDATE revision SET lifecycle='withdrawn' WHERE id=?1",
                            [&counter.evidence.revision],
                        )
                        .unwrap();
                }
                _ => unreachable!(),
            }
            let before = sequence(&f.db.conn).unwrap();
            let reply = if outcome == "revised_provisional" {
                revised(&refs, &counter)
            } else {
                json!({"outcome":outcome,"reason":"No change"}).to_string()
            };
            assert!(
                f.db.finish_native_concept_rumination(request, &reply, &f.access)
                    .is_err(),
                "{outcome}/{mutation}"
            );
            assert_eq!(sequence(&f.db.conn).unwrap(), before);
        }
    }
}
#[test]
fn canonical_native_concepts_can_be_recaptured_but_never_become_independent_support() {
    let mut f = Fixture::new();
    let refs = sources(&mut f);
    let concept = retain(&mut f, &refs);
    let capture = f.capture(
        "followup",
        "followup",
        "permit pending",
        vec![concept.clone()],
    );
    assert!(closure(&f.db.conn, &capture.projection)
        .unwrap()
        .contains(&concept));
    assert!(f.db.read(&capture.projection, &f.access).unwrap().is_some());
    assert!(interpretation::contains_hypothesis(&f.db.conn, &capture.originals[0]).unwrap());
    assert!(f
        .db
        .prepare_native_concept(
            &[capture.originals[0].clone(), refs[1].clone()],
            &f.db.snapshot().unwrap(),
            &f.access
        )
        .is_err());
    // Altering quotes cannot retain canonical host admission authority.
    let Some(Record::Concept(mut c)) = lookup(&f.db.conn, &concept).unwrap() else {
        panic!()
    };
    c.support[0].quote = "fabricated".into();
    f.db.conn
        .execute(
            "UPDATE revision SET payload=?1 WHERE id=?2",
            rusqlite::params![
                serde_json::to_string(&Record::Concept(c)).unwrap(),
                concept.revision
            ],
        )
        .unwrap();
    assert!(f
        .db
        .admit_host_capture(input("bad", "bad", "observed", vec![concept]), &f.access)
        .is_err());
}

struct Until {
    count: std::sync::atomic::AtomicUsize,
    permitted: usize,
}
impl ExternalSourceGuard for Until {
    fn check(&self, _: &ExternalBinding) -> SourceStatus {
        if self.count.fetch_add(1, Ordering::SeqCst) < self.permitted {
            SourceStatus::Checked
        } else {
            SourceStatus::Withheld
        }
    }
}
#[test]
fn native_preparation_and_every_finish_have_fresh_final_authorization() {
    let mut f = Fixture::new();
    let refs = sources(&mut f);
    f.db.source_guard = Some(Arc::new(Until {
        count: std::sync::atomic::AtomicUsize::new(0),
        permitted: 1,
    }));
    assert!(f
        .db
        .prepare_native_concept(&refs, &f.db.snapshot().unwrap(), &f.access)
        .is_err());
    for outcome in ["retain", "unchanged", "abstained"] {
        let mut f = Fixture::new();
        let refs = sources(&mut f);
        if outcome == "retain" {
            let request =
                f.db.prepare_native_concept(&refs, &f.db.snapshot().unwrap(), &f.access)
                    .unwrap();
            f.db.source_guard = Some(Arc::new(Until {
                count: std::sync::atomic::AtomicUsize::new(0),
                permitted: 1,
            }));
            let before = sequence(&f.db.conn).unwrap();
            assert!(f
                .db
                .retain_native_concept(request, &draft(&refs).to_string(), &f.access, None)
                .is_err());
            assert_eq!(sequence(&f.db.conn).unwrap(), before);
        } else {
            let prior = retain(&mut f, &refs);
            let counter = opposition(&mut f);
            let request =
                f.db.prepare_native_concept_rumination(
                    &prior,
                    &[counter],
                    &f.db.snapshot().unwrap(),
                    &f.access,
                )
                .unwrap();
            f.db.source_guard = Some(Arc::new(Until {
                count: std::sync::atomic::AtomicUsize::new(0),
                permitted: 2,
            }));
            let before = sequence(&f.db.conn).unwrap();
            assert!(f
                .db
                .finish_native_concept_rumination(
                    request,
                    &json!({"outcome":outcome,"reason":"No revision"}).to_string(),
                    &f.access
                )
                .is_err());
            assert_eq!(sequence(&f.db.conn).unwrap(), before);
        }
    }
}
#[test]
fn native_no_change_outcomes_do_not_retain_new_opposition() {
    for outcome in ["unchanged", "abstained"] {
        let mut f = Fixture::new();
        let refs = sources(&mut f);
        let prior = retain(&mut f, &refs);
        let counter = opposition(&mut f);
        let request =
            f.db.prepare_native_concept_rumination(
                &prior,
                std::slice::from_ref(&counter),
                &f.db.snapshot().unwrap(),
                &f.access,
            )
            .unwrap();
        let before = sequence(&f.db.conn).unwrap();
        f.db.finish_native_concept_rumination(
            request,
            &json!({"outcome":outcome,"reason":"Insufficient evidence"}).to_string(),
            &f.access,
        )
        .unwrap();
        assert_eq!(sequence(&f.db.conn).unwrap(), before);
        assert!(!closure(&f.db.conn, &prior)
            .unwrap()
            .contains(&counter.evidence));
        let request = f
            .db
            .prepare_native_concept_rumination(&prior, &[], &f.db.snapshot().unwrap(), &f.access)
            .unwrap();
        assert!(f
            .db
            .finish_concept_rumination(
                request,
                &json!({"outcome":outcome,"reason":"No change"}).to_string(),
                &f.access
            )
            .is_err());
    }
}
#[test]
fn native_capture_refuses_legacy_concepts_and_appraisal_refuses_concept_closure() {
    let mut f = Fixture::new();
    let refs = sources(&mut f);
    let concept = retain(&mut f, &refs);
    let receipt = f.capture("recaptured", "recaptured", "observed", vec![concept]);
    let context = interpretation::InterpretationContext::new(
        input("x", "x", "x", vec![]).criterion,
        "host-context".into(),
    )
    .unwrap();
    assert!(f
        .db
        .prepare_interpretation(
            &receipt,
            &[],
            &context,
            &f.db.snapshot().unwrap(),
            &f.access
        )
        .is_err());
    let synthetic: Vec<_> = [("s1", "permit pending"), ("s2", "inspection pending")]
        .iter()
        .map(|(id, text)| {
            f.db.capture(Capture {
                source: SourceKey {
                    connector: "synthetic".into(),
                    account: "test".into(),
                    id: (*id).into(),
                },
                text: (*text).into(),
                occurred_at: None,
                scopes: BTreeSet::new(),
                transcript: None,
            })
            .unwrap()
        })
        .collect();
    let request =
        f.db.prepare_concept(&synthetic, &f.db.snapshot().unwrap(), &f.access)
            .unwrap();
    let mut d = draft(&synthetic);
    for s in d["support"].as_array_mut().unwrap() {
        s["field"] = json!("");
    }
    let legacy =
        f.db.retain_concept(request, &d.to_string(), &f.access, None)
            .unwrap();
    assert!(f
        .db
        .admit_host_capture(
            input("legacy", "legacy", "observation", vec![legacy]),
            &f.access
        )
        .is_err());
}
