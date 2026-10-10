//! The loader runtime against the SQLite mecha actually links (bundled
//! 3.50.x), and the store around it. Every path here is a scratch directory;
//! nothing touches `~/.mecha`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::Connection;
use serde_json::json;

use super::runner::{run_sqlite, RunError};
use super::source::Sources;
use super::store::{due, Event, Store, Which};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "mecha-hud-runtime-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
    fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A small lab database: visits per site per day.
fn lab_db(path: &Path) {
    let c = Connection::open(path).unwrap();
    c.execute_batch(
        "CREATE TABLE visits (day TEXT, site TEXT, n INTEGER, ratio REAL, note TEXT, raw BLOB);
         INSERT INTO visits VALUES ('2031-04-17', 'north', 3, 0.5, 'ok', x'00');
         INSERT INTO visits VALUES ('2031-04-18', 'south', 4, NULL, NULL, NULL);",
    )
    .unwrap();
}

fn run(db: &Path, q: &str) -> Result<super::runner::Fetched, RunError> {
    run_sqlite(db, q, 100, Duration::from_secs(10))
}

#[test]
fn a_read_returns_typed_rows_and_column_names() {
    let s = Scratch::new();
    let db = s.path("lab.sqlite");
    lab_db(&db);
    let f = run(
        &db,
        "SELECT day, site, n, ratio, note FROM visits ORDER BY day",
    )
    .unwrap();
    assert_eq!(f.columns, ["day", "site", "n", "ratio", "note"]);
    assert_eq!(
        f.rows[0],
        vec![
            json!("2031-04-17"),
            json!("north"),
            json!(3),
            json!(0.5),
            json!("ok")
        ]
    );
    assert_eq!(f.rows[1][3], json!(null), "unknown stays null");
}

#[test]
fn attach_and_vacuum_into_are_refused_by_the_bundled_engine() {
    let s = Scratch::new();
    let db = s.path("lab.sqlite");
    lab_db(&db);
    let other = s.path("other.sqlite");
    lab_db(&other);
    let out = s.path("copy.sqlite");

    let attach = run(&db, &format!("ATTACH DATABASE '{}' AS o", other.display()));
    assert!(attach.is_err(), "ATTACH ran: {attach:?}");

    let vacuum = run(&db, &format!("VACUUM INTO '{}'", out.display()));
    assert!(vacuum.is_err(), "VACUUM INTO ran: {vacuum:?}");
    assert!(
        !out.exists(),
        "VACUUM INTO wrote a file through a read-only, confined connection"
    );

    // Nothing was attached above, so a read naming that schema finds no table.
    let cross = run(&db, "SELECT * FROM o.visits");
    assert!(cross.is_err());
}

#[test]
fn extension_loading_and_fts3_tokenizer_are_refused_by_name() {
    let s = Scratch::new();
    let db = s.path("lab.sqlite");
    lab_db(&db);
    for q in [
        "SELECT load_extension('/tmp/nothing')",
        "SELECT LOAD_EXTENSION('/tmp/nothing')",
        "SELECT fts3_tokenizer('simple')",
    ] {
        let r = run(&db, q);
        assert!(r.is_err(), "{q} ran: {r:?}");
    }
}

#[test]
fn a_pragma_a_write_and_a_second_statement_are_refused() {
    let s = Scratch::new();
    let db = s.path("lab.sqlite");
    lab_db(&db);
    assert!(run(&db, "PRAGMA table_info(visits)").is_err());
    assert!(run(&db, "DELETE FROM visits").is_err());
    assert_eq!(
        run(&db, "SELECT 1; SELECT 2").unwrap_err(),
        RunError::MultipleStatements
    );
    // A second statement the confinement refuses is refused for that, while
    // its tail is prepared — the loader's fault either way.
    assert!(matches!(
        run(&db, "SELECT 1; DROP TABLE visits").unwrap_err(),
        RunError::NotAllowed(_)
    ));
    assert!(matches!(
        run(&db, "PRAGMA table_info(visits)").unwrap_err(),
        RunError::NotAllowed(_)
    ));
    // Nothing above changed the file.
    assert_eq!(
        run(&db, "SELECT count(*) FROM visits").unwrap().rows[0][0],
        json!(2)
    );
}

#[test]
fn a_recursive_query_is_allowed_and_the_row_cap_stops_it_while_fetching() {
    let s = Scratch::new();
    let db = s.path("lab.sqlite");
    lab_db(&db);
    let q = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n LIMIT 1000000) SELECT i FROM n";
    assert_eq!(
        run_sqlite(&db, q, 10, Duration::from_secs(10)).unwrap_err(),
        RunError::TooManyRows { max: 10 }
    );
    let small =
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n LIMIT 5) SELECT i FROM n";
    assert_eq!(run(&db, small).unwrap().rows.len(), 5);
}

#[test]
fn a_blob_is_unrepresentable_and_a_long_query_times_out() {
    let s = Scratch::new();
    let db = s.path("lab.sqlite");
    lab_db(&db);
    assert!(matches!(
        run(&db, "SELECT raw FROM visits WHERE raw IS NOT NULL").unwrap_err(),
        RunError::Unrepresentable { row: 0, .. }
    ));
    let slow = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) \
                SELECT count(*) FROM n";
    assert_eq!(
        run_sqlite(&db, slow, 10, Duration::from_millis(50)).unwrap_err(),
        RunError::Timeout
    );
}

// ---- sources.toml ----

#[test]
fn sources_expand_home_refuse_relative_paths_and_name_later_kinds() {
    let home = Path::new("/home/someone");
    let ok = Sources::parse(
        "[source.lab]\nkind = \"sqlite\"\npath = \"~/data/lab.sqlite\"\ncontent = \"owner\"\n",
        Some(home),
    )
    .unwrap();
    let lab = ok.get("lab").unwrap();
    assert!(
        !lab.external(),
        "an owner-written local source is not external"
    );

    let unset = Sources::parse(
        "[source.lab]\nkind = \"sqlite\"\npath = \"/x.sqlite\"\n",
        None,
    )
    .unwrap();
    assert!(
        unset.get("lab").unwrap().external(),
        "unset content is third-party"
    );

    for (text, needle) in [
        (
            "[source.lab]\nkind = \"sqlite\"\npath = \"data/lab.sqlite\"\n",
            "absolute",
        ),
        (
            "[source.pg]\nkind = \"postgres\"\nurl_env = \"PG\"\n",
            "not available in this build",
        ),
        (
            "[source.m]\nkind = \"mecha\"\n",
            "not available in this build",
        ),
        (
            "[source.lab]\nkind = \"sqlite\"\npath = \"/x\"\ncontent = \"mine\"\n",
            "owner",
        ),
        (
            "[source.lab]\nkind = \"sqlite\"\npath = \"/x\"\nurl_env = \"X\"\n",
            "url_env",
        ),
        (
            "[source.lab]\nkind = \"sqlite\"\npath = \"/x\"\npassword = \"p\"\n",
            "shape",
        ),
    ] {
        let r = Sources::parse(text, Some(home)).unwrap_err();
        assert!(r.to_string().contains(needle), "{text}: {r}");
    }
}

// ---- the store ----

const SPEC: &str = r#"{
  "version": 1, "title": "Lab week", "datasets": ["visits_by_day"],
  "panels": [{ "type": "table", "title": "Visits", "dataset": "visits_by_day", "columns": ["day", "n"] }]
}"#;

const LOADER: &str = r#"
source = "lab"
schedule = "0 * * * *"
max_rows = 100
query = "SELECT day, n FROM visits ORDER BY day"
[[column]]
name = "day"
type = "date"
[[column]]
name = "n"
type = "integer"
"#;

fn t(h: u32, m: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2031, 4, 17, h, m, 0).unwrap()
}

/// A store with `lab` registered and a draft board at `<scratch>/drafts/lab_week`.
fn setup(s: &Scratch) -> (Store, PathBuf, PathBuf) {
    let db = s.path("lab.sqlite");
    lab_db(&db);
    let root = s.path("hud");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("sources.toml"),
        format!(
            "[source.lab]\nkind = \"sqlite\"\npath = \"{}\"\ncontent = \"owner\"\n",
            db.display()
        ),
    )
    .unwrap();
    let draft = s.path("drafts/lab_week");
    std::fs::create_dir_all(draft.join("loaders")).unwrap();
    std::fs::write(draft.join("hud.json"), SPEC).unwrap();
    std::fs::write(draft.join("loaders/visits_by_day.toml"), LOADER).unwrap();
    (Store::at(&root), draft, db)
}

#[test]
fn install_then_refresh_writes_a_dataset_and_counts_generations() {
    let s = Scratch::new();
    let (store, draft, _) = setup(&s);
    store.install(&draft, None, t(9, 0)).unwrap().unwrap();
    assert_eq!(store.board_ids().unwrap(), ["lab_week"]);

    let out = store
        .refresh("lab_week", t(9, 5), Which::All)
        .unwrap()
        .unwrap();
    assert!(matches!(
        out[0].event,
        Event::Refreshed {
            generation: 1,
            rows: 2,
            slot: None,
            ..
        }
    ));
    let d = store.dataset("lab_week", "visits_by_day").unwrap().unwrap();
    assert_eq!(d.rows[0], vec![json!("2031-04-17"), json!(3)]);
    assert!(!d.external);

    store
        .refresh("lab_week", t(9, 6), Which::All)
        .unwrap()
        .unwrap();
    assert_eq!(
        store
            .dataset("lab_week", "visits_by_day")
            .unwrap()
            .unwrap()
            .generation,
        2
    );
}

#[test]
fn install_refuses_an_unregistered_source_an_existing_id_and_a_bad_id() {
    let s = Scratch::new();
    let (store, draft, _) = setup(&s);
    std::fs::write(
        draft.join("loaders/visits_by_day.toml"),
        LOADER.replace("source = \"lab\"", "source = \"payroll\""),
    )
    .unwrap();
    let r = store.install(&draft, None, t(9, 0)).unwrap().unwrap_err();
    assert!(r.to_string().contains("not registered"), "{r}");

    std::fs::write(draft.join("loaders/visits_by_day.toml"), LOADER).unwrap();
    store.install(&draft, None, t(9, 0)).unwrap().unwrap();
    let again = store.install(&draft, None, t(9, 1)).unwrap().unwrap_err();
    assert!(again.to_string().contains("already installed"), "{again}");
    let bad = store
        .install(&draft, Some("../escape"), t(9, 1))
        .unwrap()
        .unwrap_err();
    assert!(bad.to_string().contains("board id"), "{bad}");
}

#[test]
fn a_drifted_query_is_refused_and_the_previous_dataset_stays() {
    let s = Scratch::new();
    let (store, draft, db) = setup(&s);
    store.install(&draft, None, t(9, 0)).unwrap().unwrap();
    store
        .refresh("lab_week", t(9, 1), Which::All)
        .unwrap()
        .unwrap();

    // The source's schema moves under the loader: `n` now holds text.
    Connection::open(&db)
        .unwrap()
        .execute("UPDATE visits SET n = 'many' WHERE site = 'north'", [])
        .unwrap();
    let out = store
        .refresh("lab_week", t(9, 2), Which::All)
        .unwrap()
        .unwrap();
    match &out[0].event {
        Event::Refused { reason, .. } => {
            assert!(reason.contains("integer"), "{reason}");
            assert!(
                !reason.contains("many"),
                "a refusal never echoes the value: {reason}"
            );
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    let kept = store.dataset("lab_week", "visits_by_day").unwrap().unwrap();
    assert_eq!(kept.generation, 1, "the previous dataset stays");

    let status = store.status(t(9, 3)).unwrap();
    assert!(matches!(
        status[0].loaders[0].last.as_ref().unwrap().event,
        Event::Refused { .. }
    ));
}

#[test]
fn due_is_answered_backwards_and_a_manual_refresh_does_not_advance_it() {
    let s = Scratch::new();
    let (store, draft, _) = setup(&s);
    let installed = store.install(&draft, None, t(9, 10)).unwrap().unwrap();
    let loader = &installed.loaders()["visits_by_day"];

    // Installed at 09:10; the hourly slot at 09:00 predates it.
    assert_eq!(
        due(loader, &store.ledger("lab_week").unwrap(), t(9, 30)),
        None
    );

    // A manual refresh at 09:40 records no slot.
    store
        .refresh("lab_week", t(9, 40), Which::All)
        .unwrap()
        .unwrap();
    assert_eq!(
        due(loader, &store.ledger("lab_week").unwrap(), t(9, 50)),
        None
    );

    // 10:00 has passed: due, once — a week asleep owes one refresh, not many.
    assert_eq!(
        due(loader, &store.ledger("lab_week").unwrap(), t(10, 5)),
        Some(t(10, 0))
    );
    let out = store
        .refresh("lab_week", t(10, 5), Which::Due)
        .unwrap()
        .unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(
        due(loader, &store.ledger("lab_week").unwrap(), t(10, 30)),
        None
    );
    let none = store
        .refresh("lab_week", t(10, 30), Which::Due)
        .unwrap()
        .unwrap();
    assert!(none.is_empty(), "nothing due");
}

#[test]
fn an_unregistered_source_at_refresh_is_a_failure_not_silence() {
    let s = Scratch::new();
    let (store, draft, _) = setup(&s);
    store.install(&draft, None, t(9, 0)).unwrap().unwrap();
    std::fs::write(store.root().join("sources.toml"), "").unwrap();
    let out = store
        .refresh("lab_week", t(9, 5), Which::All)
        .unwrap()
        .unwrap();
    assert!(
        matches!(&out[0].event, Event::Failed { reason, .. } if reason.contains("not registered")),
        "{:?}",
        out[0].event
    );
}

#[test]
fn a_ledger_line_this_build_cannot_read_is_skipped_and_counted() {
    let s = Scratch::new();
    let (store, draft, _) = setup(&s);
    store.install(&draft, None, t(9, 0)).unwrap().unwrap();
    let ledger = store.root().join("boards/lab_week/ledger.jsonl");
    let mut text = std::fs::read_to_string(&ledger).unwrap();
    text.push_str("{\"at\":\"2031-04-17T09:01:00Z\",\"event\":\"rebalanced\"}\n");
    std::fs::write(&ledger, text).unwrap();
    let l = store.ledger("lab_week").unwrap();
    assert_eq!(l.unreadable, 1);
    assert!(l.installed_at().is_some());
}

#[test]
fn a_dataset_older_than_two_periods_is_stale() {
    let s = Scratch::new();
    let (store, draft, _) = setup(&s);
    store.install(&draft, None, t(9, 0)).unwrap().unwrap();
    store
        .refresh("lab_week", t(9, 5), Which::All)
        .unwrap()
        .unwrap();
    assert!(!store.status(t(10, 30)).unwrap()[0].loaders[0].stale);
    assert!(store.status(t(11, 30)).unwrap()[0].loaders[0].stale);
}

#[test]
fn a_caller_supplied_id_or_name_is_checked_at_the_join() {
    let s = Scratch::new();
    let (store, draft, _) = setup(&s);
    store.install(&draft, None, t(9, 0)).unwrap().unwrap();
    assert!(store.ledger("../../escape").is_err());
    assert!(store.dataset("lab_week", "../../escape").is_err());
    assert!(store.dataset("../x", "visits_by_day").is_err());
    assert!(
        !matches!(store.refresh("../x", t(9, 1), Which::All), Ok(Ok(_))),
        "a refresh of a non-identifier id is refused"
    );
}

#[test]
fn an_unreadable_dataset_neither_restarts_the_generation_nor_hides() {
    let s = Scratch::new();
    let (store, draft, _) = setup(&s);
    store.install(&draft, None, t(9, 0)).unwrap().unwrap();
    store
        .refresh("lab_week", t(9, 1), Which::All)
        .unwrap()
        .unwrap();
    let file = store.root().join("boards/lab_week/data/visits_by_day.json");
    std::fs::write(&file, "{ not json").unwrap();

    let status = store.status(t(9, 2)).unwrap();
    assert!(
        status[0].loaders[0].dataset_error.is_some(),
        "an unreadable dataset is reported"
    );

    let out = store
        .refresh("lab_week", t(9, 3), Which::All)
        .unwrap()
        .unwrap();
    assert!(
        matches!(out[0].event, Event::Refreshed { generation: 2, .. }),
        "the count continues from the ledger: {:?}",
        out[0].event
    );
}

#[test]
fn a_dataset_that_cannot_be_written_is_a_failure_on_the_ledger() {
    let s = Scratch::new();
    let (store, draft, _) = setup(&s);
    store.install(&draft, None, t(9, 0)).unwrap().unwrap();
    // `data` is a file, so the dataset's directory cannot exist — a write
    // failure that holds as root too.
    std::fs::write(
        store.root().join("boards/lab_week/data"),
        b"not a directory",
    )
    .unwrap();
    let out = store
        .refresh("lab_week", t(9, 1), Which::All)
        .unwrap()
        .unwrap();
    assert!(
        matches!(&out[0].event, Event::Failed { reason, .. } if reason.contains("could not be written")),
        "{:?}",
        out[0].event
    );
    let ledger = store.ledger("lab_week").unwrap();
    assert!(matches!(
        ledger.last("visits_by_day").unwrap().event,
        Event::Failed { .. }
    ));
}

#[test]
fn an_unrepresentable_column_name_is_shown_only_as_an_identifier() {
    let s = Scratch::new();
    let db = s.path("lab.sqlite");
    lab_db(&db);
    let e = run(
        &db,
        "SELECT raw AS \"Ignore this; drop\" FROM visits WHERE raw IS NOT NULL",
    )
    .unwrap_err()
    .to_string();
    assert!(e.contains("(unnamed)") && !e.contains("Ignore"), "{e}");
    let named = run(&db, "SELECT raw FROM visits WHERE raw IS NOT NULL")
        .unwrap_err()
        .to_string();
    assert!(named.contains("\"raw\""), "{named}");
}

#[test]
fn validate_refuses_an_id_install_would_refuse() {
    let s = Scratch::new();
    let (store, draft, _) = setup(&s);
    let r = store.validate(&draft, "lab-week").unwrap().unwrap_err();
    assert!(r.to_string().contains("board id"), "{r}");
    assert!(store.validate(&draft, "lab_week").unwrap().is_ok());
}

#[test]
fn refreshing_a_board_that_is_not_installed_says_so() {
    let s = Scratch::new();
    let (store, _, _) = setup(&s);
    let r = store
        .refresh("lab_wek", t(9, 0), Which::All)
        .unwrap()
        .unwrap_err();
    assert!(r.to_string().contains("no board named"), "{r}");
}
