//! A rule change as one unit, and each way recovery can find one a crash
//! interrupted (`LearningStore::commit_rules`, `resume_interrupted`).

use super::*;

fn store() -> (LearningStore, PathBuf) {
    let dir = std::env::temp_dir()
        .join("mecha-learning-test")
        .join(uuid::Uuid::new_v4().to_string());
    (LearningStore::open(dir.clone()).unwrap(), dir)
}

fn with_reflections(dir: &Path, ids: &[&str]) {
    let lines: String = ids
        .iter()
        .map(|id| {
            format!(
                r#"{{"id":"{id}","domain":"behavior","session_id":"s1","trigger":"steer","context":"c","intervention":"i","reflexion_text":"t","created_at":"2026-09-01T00:00:00Z"}}"#
            ) + "\n"
        })
        .collect();
    std::fs::write(dir.join("reflections.jsonl"), lines).unwrap();
}

fn rule(text: &str, id: &str) -> Rule {
    Rule {
        text: text.into(),
        id: Some(id.into()),
        ..Default::default()
    }
}

fn change(proposal: bool) -> RuleCommit {
    let run = LeapRun {
        id: "run-1".into(),
        domain: "behavior".into(),
        reflexions_processed: 2,
        rules_before: 1,
        rules_after: 2,
        created_at: "2026-09-29T06:00:00Z".into(),
    };
    let rules = vec![rule("Old.", "r-a"), rule("New.", "r-b")];
    RuleCommit {
        proposal: proposal.then(|| Proposal {
            id: run.id.clone(),
            domain: "behavior".into(),
            status: "accepted".into(),
            reflexion_ids: vec!["x1".into(), "x2".into()],
            rules_before: vec![rule("Old.", "r-a")],
            rules: rules.clone(),
            evidence: "e".into(),
            created_at: "2026-09-29T06:00:00Z".into(),
            resolved_at: Some("2026-09-29T06:00:01Z".into()),
            reason: None,
            scope: None,
        }),
        run,
        reflexion_ids: vec!["x1".into(), "x2".into()],
        rules,
    }
}

fn runs(dir: &Path) -> usize {
    std::fs::read_to_string(dir.join("runs.jsonl"))
        .map(|t| t.lines().count())
        .unwrap_or(0)
}

fn marked(store: &LearningStore) -> usize {
    store
        .reflexions()
        .unwrap()
        .iter()
        .filter(|r| r.is_processed && r.leap_run_id.as_deref() == Some("run-1"))
        .count()
}

/// The crash, staged: the record on disk, and whichever steps "ran".
fn interrupted(dir: &Path, c: &RuleCommit, before: Vec<Rule>) {
    let intent = CommitIntent {
        change: c.clone(),
        rules_before: before,
    };
    std::fs::write(
        dir.join(COMMIT_FILE),
        serde_json::to_string_pretty(&intent).unwrap(),
    )
    .unwrap();
}

#[test]
fn a_commit_lands_every_step_and_leaves_no_record() {
    let (store, dir) = store();
    with_reflections(&dir, &["x1", "x2", "x3"]);
    store
        .write_learned_rules("behavior", &[rule("Old.", "r-a")])
        .unwrap();
    store.commit_rules(&change(true)).unwrap();

    assert_eq!(store.learned_rules("behavior").unwrap().len(), 2);
    assert_eq!(marked(&store), 2);
    assert_eq!(runs(&dir), 1);
    assert_eq!(store.proposal("run-1").unwrap().status, "accepted");
    assert!(!dir.join(COMMIT_FILE).exists());
    assert_eq!(store.resume_interrupted().unwrap(), None);
    std::fs::remove_dir_all(&dir).ok();
}

/// The crash between the rules and the marks — the one that used to make
/// the next pass re-argue a batch against its own result.
#[test]
fn rules_that_landed_get_their_marks_run_and_proposal() {
    let (store, dir) = store();
    with_reflections(&dir, &["x1", "x2"]);
    let c = change(true);
    store.write_learned_rules("behavior", &c.rules).unwrap();
    interrupted(&dir, &c, vec![rule("Old.", "r-a")]);

    let said = store.resume_interrupted().unwrap().unwrap();
    assert!(
        said.starts_with("finished an interrupted rule change"),
        "{said}"
    );
    assert_eq!(marked(&store), 2);
    assert_eq!(runs(&dir), 1);
    assert_eq!(store.proposal("run-1").unwrap().status, "accepted");
    assert!(!dir.join(COMMIT_FILE).exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn rules_that_never_landed_are_written_by_recovery() {
    let (store, dir) = store();
    with_reflections(&dir, &["x1", "x2"]);
    store
        .write_learned_rules("behavior", &[rule("Old.", "r-a")])
        .unwrap();
    let c = change(false);
    interrupted(&dir, &c, vec![rule("Old.", "r-a")]);

    store.resume_interrupted().unwrap().unwrap();
    let live = store.learned_rules("behavior").unwrap();
    assert_eq!(live.len(), 2);
    assert_eq!(live[1].text, "New.");
    assert_eq!(marked(&store), 2);
    std::fs::remove_dir_all(&dir).ok();
}

/// Every step ran but the record's removal: finishing again duplicates
/// nothing.
#[test]
fn finishing_a_finished_change_duplicates_no_run() {
    let (store, dir) = store();
    with_reflections(&dir, &["x1", "x2"]);
    store
        .write_learned_rules("behavior", &[rule("Old.", "r-a")])
        .unwrap();
    let c = change(true);
    store.commit_rules(&c).unwrap();
    interrupted(&dir, &c, vec![rule("Old.", "r-a")]);

    store.resume_interrupted().unwrap().unwrap();
    assert_eq!(runs(&dir), 1);
    assert_eq!(marked(&store), 2);
    std::fs::remove_dir_all(&dir).ok();
}

/// An owner verb moved the rules after the crash: recovery must not write
/// over that decision. The record is set aside and the batch left unmarked.
#[test]
fn rules_moved_since_set_the_change_aside() {
    let (store, dir) = store();
    with_reflections(&dir, &["x1", "x2"]);
    let owners = vec![rule("Old.", "r-a"), rule("The owner's edit.", "r-c")];
    store.write_learned_rules("behavior", &owners).unwrap();
    interrupted(&dir, &change(true), vec![rule("Old.", "r-a")]);

    let said = store.resume_interrupted().unwrap().unwrap();
    assert!(said.contains("set aside"), "{said}");
    let live = store.learned_rules("behavior").unwrap();
    assert_eq!(live[1].text, "The owner's edit.");
    assert_eq!(marked(&store), 0);
    assert_eq!(runs(&dir), 0);
    assert!(!dir.join(COMMIT_FILE).exists());
    let aside = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(UNFINISHED_COMMIT_PREFIX)
        })
        .count();
    assert_eq!(aside, 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_unreadable_record_is_set_aside_not_fatal() {
    let (store, dir) = store();
    std::fs::write(dir.join(COMMIT_FILE), "{\"half").unwrap();
    let said = store.resume_interrupted().unwrap().unwrap();
    assert!(said.contains("cannot read"), "{said}");
    assert!(!dir.join(COMMIT_FILE).exists());
    std::fs::remove_dir_all(&dir).ok();
}
