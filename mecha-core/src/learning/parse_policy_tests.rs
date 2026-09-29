//! D1 (`docs/LEARNING-STORE-RESEARCH.md` §7): a machine-written learned file
//! that does not parse is skipped aloud; the owner's file still stops the run;
//! and the owner's replace checks before it writes.

use super::*;

fn store() -> (LearningStore, PathBuf) {
    let dir = std::env::temp_dir()
        .join("mecha-learning-test")
        .join(uuid::Uuid::new_v4().to_string());
    (LearningStore::open(dir.clone()).unwrap(), dir)
}

const USER: &str = "# mine\n[[rules]]\ntext = \"Ask before pushing.\"\n";
const BROKEN: &str = "[[rules]]\ntext = \"half an edit\n";

fn write(dir: &Path, name: &str, text: &str) {
    std::fs::create_dir_all(dir.join("rules")).unwrap();
    std::fs::write(dir.join("rules").join(name), text).unwrap();
}

fn run() -> Situation {
    Situation::of_run(&["shell".into()], None)
}

/// Fails on the old behaviour, where a bad `learned.toml` was an error and
/// every run start stopped on it.
#[test]
fn a_learned_file_that_does_not_parse_is_skipped_and_recorded() {
    let (store, dir) = store();
    write(&dir, "behavior.user.toml", USER);
    write(&dir, "behavior.learned.toml", BROKEN);

    let carried = store.rules_carried_for(&["behavior"], &run()).unwrap();

    let block = carried.block.expect("the owner's rules still ride");
    assert!(block.contains("Ask before pushing."), "{block}");
    assert_eq!(carried.skipped.len(), 1);
    assert_eq!(carried.skipped[0].domain, "behavior");
    assert!(carried.skipped[0].path.ends_with("behavior.learned.toml"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_owners_file_that_does_not_parse_still_stops_the_run_and_names_the_fix() {
    let (store, dir) = store();
    write(&dir, "behavior.user.toml", BROKEN);
    let err = store.rules_carried_for(&["behavior"], &run()).unwrap_err();
    assert!(
        format!("{err:#}").contains("mecha rules edit --user --domain behavior"),
        "{err:#}"
    );
    let err = store.pass_rules_block_for(&["behavior"]).unwrap_err();
    assert!(format!("{err:#}").contains("rules edit --user"), "{err:#}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_pass_skips_a_bad_learned_file_and_says_which() {
    let (store, dir) = store();
    write(&dir, "triage.user.toml", USER);
    write(&dir, "triage.learned.toml", BROKEN);
    let (block, skipped) = store.pass_rules_block_for(&["triage"]).unwrap();
    assert!(block.unwrap().contains("Ask before pushing."));
    assert_eq!(skipped.len(), 1);
    std::fs::remove_dir_all(&dir).ok();
}

/// The verbs that act on the file itself stay strict: a learn pass must not
/// read a skipped file as an empty rule set and consolidate over it.
#[test]
fn reading_learned_rules_directly_is_still_strict() {
    let (store, dir) = store();
    write(&dir, "behavior.learned.toml", BROKEN);
    assert!(store.learned_rules("behavior").is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn replacing_the_owners_rules_checks_first_and_keeps_comments() {
    let (store, dir) = store();
    write(&dir, "behavior.user.toml", USER);
    let path = dir.join("rules").join("behavior.user.toml");

    assert!(store.replace_user_rules("behavior", BROKEN).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), USER);

    let next = "# still mine\n[[rules]]\ntext = \"A.\"\n\n[[rules]]\ntext = \"B.\"\n";
    assert_eq!(store.replace_user_rules("behavior", next).unwrap(), 2);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), next);
    assert!(!dir.join("rules").join("behavior.user.toml.tmp").exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_domain_is_a_name_never_a_path() {
    let (store, dir) = store();
    for bad in ["", "../escape", "a/b", "a.b"] {
        assert!(store.replace_user_rules(bad, USER).is_err(), "{bad:?}");
    }
    assert!(!dir.join("escape.user.toml").exists());
    std::fs::remove_dir_all(&dir).ok();
}
