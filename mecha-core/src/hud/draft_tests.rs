use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::Utc;
use rusqlite::Connection;

use super::{Draft, Fetch, Proposed, MAX_PENDING};
use crate::hud::{Store, AUTHORING_GUIDE};
use crate::imagelib::Origin;

/// A store with one source, `system`, over a small `model_minute` table
/// whose rows are recent enough for the guide's example query.
struct Fixture {
    root: PathBuf,
    store: Store,
}

impl Fixture {
    fn new() -> Fixture {
        let root = std::env::temp_dir().join(format!("mecha-hud-draft-{}", uuid::Uuid::new_v4()));
        let hud = root.join("hud");
        std::fs::create_dir_all(&hud).unwrap();
        let db = root.join("series.sqlite");
        let c = Connection::open(&db).unwrap();
        c.execute_batch(
            "CREATE TABLE model_minute (at TEXT NOT NULL, kind INTEGER NOT NULL, gen_tps REAL);
             INSERT INTO model_minute VALUES (datetime('now', '-30 minutes'), 1, 91.5);
             INSERT INTO model_minute VALUES (datetime('now', '-20 minutes'), 1, NULL);
             INSERT INTO model_minute VALUES (datetime('now', '-10 minutes'), 1, 88.25);",
        )
        .unwrap();
        std::fs::write(
            hud.join("sources.toml"),
            format!(
                "[source.system]\nkind = \"sqlite\"\npath = \"{}\"\ncontent = \"owner\"\n",
                db.display()
            ),
        )
        .unwrap();
        Fixture {
            store: Store::at(&hud),
            root,
        }
    }

    fn hud(&self) -> PathBuf {
        self.root.join("hud")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The fenced block of `lang` that follows `after` in the guide.
fn fenced<'a>(lang: &str, after: &str) -> &'a str {
    let guide: &'static str = AUTHORING_GUIDE;
    let from = guide.find(after).expect("the guide has the section");
    let open = format!("```{lang}\n");
    let start = from + guide[from..].find(&open).expect("a fenced block") + open.len();
    let end = start + guide[start..].find("```").expect("closed");
    &guide[start..end]
}

fn example() -> Draft {
    let mut loaders = BTreeMap::new();
    loaders.insert(
        "speed_24h".to_string(),
        fenced("toml", "## Loaders").to_string(),
    );
    Draft::new(
        "example",
        fenced("json", "## The spec").to_string(),
        loaders,
    )
    .unwrap()
}

fn loader(query: &str, columns: &[(&str, &str)]) -> String {
    let mut s = format!(
        "source = \"system\"\nschedule = \"* * * * *\"\nmax_rows = 100\nquery = \"\"\"\n{query}\n\"\"\"\n"
    );
    for (name, kind) in columns {
        s.push_str(&format!(
            "\n[[column]]\nname = \"{name}\"\ntype = \"{kind}\"\n"
        ));
    }
    s
}

fn draft(id: &str, title: &str, gen_query: &str) -> Draft {
    let spec = format!(
        r#"{{"version": 1, "title": "{title}", "datasets": ["speed"],
            "panels": [{{"type": "kpi", "title": "Latest", "dataset": "speed",
                         "value": {{"op": "last", "field": "tps"}}}}]}}"#
    );
    let mut loaders = BTreeMap::new();
    loaders.insert(
        "speed".to_string(),
        loader(gen_query, &[("at", "timestamp"), ("tps", "float")]),
    );
    Draft::new(id, spec, loaders).unwrap()
}

const GOOD: &str =
    "SELECT at, gen_tps AS tps FROM model_minute WHERE gen_tps IS NOT NULL ORDER BY at";

/// The guide is what a model drafts from: its worked example must be a board
/// that checks and runs, or the guide teaches a refusal.
#[test]
fn the_guides_example_previews_clean() {
    let f = Fixture::new();
    let p = f.store.preview(&example()).unwrap();
    assert!(p.refusals.is_empty(), "{:?}", p.refusals);
    assert!(p.is_clean(), "{p:?}");
    let Fetch::Rows { rows, .. } = &p.datasets[0].fetch else {
        unreachable!()
    };
    assert_eq!(rows.len(), 2, "NULL speeds are left out, as the guide says");
}

/// Every refusal comes back named, and the scratch copy is gone afterwards:
/// a preview writes nothing a board or a refresh reads.
#[test]
fn a_preview_names_every_refusal_and_leaves_nothing_behind() {
    let f = Fixture::new();
    let spec = r#"{"version": 1, "title": "Bad", "datasets": ["speed"],
        "panels": [{"type": "chart", "title": "c", "dataset": "speed",
                    "vegalite": {"data": {"name": "speed"}, "mark": "line"}}]}"#;
    let mut loaders = BTreeMap::new();
    loaders.insert(
        "speed".to_string(),
        loader(GOOD, &[("at", "timestamp"), ("Tps", "float")]),
    );
    let d = Draft::new("bad", spec.to_string(), loaders).unwrap();
    let p = f.store.preview(&d).unwrap();
    let all: Vec<String> = p.refusals.iter().map(|r| r.to_string()).collect();
    assert!(all.iter().any(|r| r.contains("dataset")), "{all:?}");
    assert!(all.iter().any(|r| r.contains("Tps")), "{all:?}");
    assert!(!p.is_clean());
    assert!(p.datasets.is_empty(), "nothing runs until the checks pass");
    let scratch = f.hud().join(".scratch");
    let left = std::fs::read_dir(&scratch)
        .map(|d| d.count())
        .unwrap_or_default();
    assert_eq!(left, 0, "the scratch copy is removed");
    assert!(!f.hud().join("boards").exists(), "and no board was made");
}

/// A query the source fails is a failed dataset, not a clean preview, and
/// such a draft is not put in front of the owner.
#[test]
fn a_draft_whose_loader_fails_is_not_proposed() {
    let f = Fixture::new();
    let d = draft(
        "broken",
        "Broken",
        "SELECT at, gen_tps AS tps FROM no_such_table",
    );
    let p = f.store.preview(&d).unwrap();
    assert!(p.refusals.is_empty());
    assert!(matches!(p.datasets[0].fetch, Fetch::Failed(_)), "{p:?}");
    match f.store.propose(&d, Origin::ModelClean, Utc::now()).unwrap() {
        Proposed::NotClean(_) => {}
        other => panic!("{other:?}"),
    }
    assert!(f.store.proposal_ids().unwrap().is_empty());
}

/// A name becomes a path: one that is not an identifier never reaches the
/// filesystem.
#[test]
fn a_draft_names_nothing_outside_its_directory() {
    let mut loaders = BTreeMap::new();
    loaders.insert("../escape".to_string(), "x".to_string());
    let r = Draft::new("../boards/x", "{}".to_string(), loaders).unwrap_err();
    let all = r.to_string();
    assert!(all.contains("board id"), "{all}");
    assert!(all.contains("dataset name"), "{all}");
    assert!(
        Draft::new("ok", "{}".into(), BTreeMap::new()).is_err(),
        "no loaders"
    );
}

/// Propose stages; only an accept naming what was shown installs; the board
/// is refreshed straight away and the proposal is gone.
#[test]
fn a_proposal_installs_only_as_shown() {
    let f = Fixture::new();
    let d = draft("speed", "Speed", GOOD);
    match f
        .store
        .propose(&d, Origin::ModelUntrusted, Utc::now())
        .unwrap()
    {
        Proposed::Staged { revised: false, .. } => {}
        other => panic!("{other:?}"),
    }
    assert!(
        !f.hud().join("boards/speed").exists(),
        "proposing installs nothing"
    );
    let p = f.store.proposal("speed").unwrap().unwrap().unwrap();
    assert_eq!(p.meta.origin, Origin::ModelUntrusted);
    assert_eq!(p.meta.digest, d.digest());
    assert!(!p.replaces);
    let data = f.store.proposal_dataset("speed", "speed").unwrap().unwrap();
    assert_eq!(data.rows.len(), 2, "the owner sees the rows it has now");

    // Revised after the owner looked: the old digest no longer installs.
    let shown = d.digest();
    let revised = draft("speed", "Speed, revised", GOOD);
    match f
        .store
        .propose(&revised, Origin::ModelUntrusted, Utc::now())
        .unwrap()
    {
        Proposed::Staged { revised: true, .. } => {}
        other => panic!("{other:?}"),
    }
    let refused = f.store.accept("speed", &shown, Utc::now()).unwrap();
    assert!(refused.is_err(), "a revision is never installed unseen");
    assert!(f.store.accept("speed", "", Utc::now()).unwrap().is_err());

    let accepted = f
        .store
        .accept("speed", &revised.digest(), Utc::now())
        .unwrap()
        .unwrap();
    assert!(!accepted.replaced);
    assert_eq!(accepted.installed.spec().title(), "Speed, revised");
    assert_eq!(accepted.refreshed.len(), 1);
    assert!(
        f.store.dataset("speed", "speed").unwrap().is_some(),
        "refreshed"
    );
    assert!(
        f.store.proposal_ids().unwrap().is_empty(),
        "the proposal is gone"
    );
}

/// Replacing an installed board keeps what it accumulated — the ledger, the
/// publish markers — and drops datasets the old queries made.
#[test]
fn accepting_a_replacement_keeps_the_boards_ledger_and_publish_state() {
    let f = Fixture::new();
    let first = draft("speed", "Speed", GOOD);
    f.store
        .propose(&first, Origin::ModelClean, Utc::now())
        .unwrap();
    f.store
        .accept("speed", &first.digest(), Utc::now())
        .unwrap()
        .unwrap();
    let board = f.hud().join("boards/speed");
    std::fs::write(board.join("published.json"), r#"{"bundle":"hud-speed"}"#).unwrap();
    let generation = f.store.ledger("speed").unwrap().last_generation("speed");
    assert_eq!(generation, Some(1));

    let second = draft(
        "speed",
        "Speed, again",
        "SELECT at, gen_tps AS tps FROM model_minute WHERE gen_tps > 90",
    );
    match f
        .store
        .propose(&second, Origin::ModelClean, Utc::now())
        .unwrap()
    {
        Proposed::Staged { preview, .. } => assert!(preview.replaces),
        other => panic!("{other:?}"),
    }
    let accepted = f
        .store
        .accept("speed", &second.digest(), Utc::now())
        .unwrap()
        .unwrap();
    assert!(accepted.replaced);
    assert_eq!(
        f.store.load("speed").unwrap().unwrap().spec().title(),
        "Speed, again"
    );
    assert!(board.join("published.json").exists(), "publish state kept");
    assert_eq!(
        f.store.ledger("speed").unwrap().last_generation("speed"),
        Some(2),
        "generations keep counting up across the replacement"
    );
    let data = f.store.dataset("speed", "speed").unwrap().unwrap();
    assert_eq!(data.rows.len(), 1, "the new query's rows, not the old ones");
    let strays: Vec<_> = std::fs::read_dir(f.hud().join("boards"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name())
        .filter(|n| n.to_string_lossy().starts_with('.'))
        .collect();
    assert!(strays.is_empty(), "no staging left behind: {strays:?}");
}

/// A conversation proposing in a loop fills the queue to the cap and is then
/// refused; revising one already waiting is still allowed.
#[test]
fn proposals_are_capped_but_revisable() {
    let f = Fixture::new();
    for i in 0..MAX_PENDING {
        let d = draft(&format!("b{i}"), "B", GOOD);
        assert!(matches!(
            f.store.propose(&d, Origin::ModelClean, Utc::now()).unwrap(),
            Proposed::Staged { .. }
        ));
    }
    let one_more = draft("extra", "B", GOOD);
    assert!(matches!(
        f.store
            .propose(&one_more, Origin::ModelClean, Utc::now())
            .unwrap(),
        Proposed::TooMany
    ));
    let again = draft("b0", "B again", GOOD);
    assert!(matches!(
        f.store
            .propose(&again, Origin::ModelClean, Utc::now())
            .unwrap(),
        Proposed::Staged { revised: true, .. }
    ));
    assert!(f.store.reject("b1").unwrap());
    assert!(!f.store.reject("b1").unwrap(), "already gone");
    assert!(
        f.store.reject("../boards").is_err(),
        "an id is checked at the join"
    );
}

/// An installed board reads back as the draft it was installed from, so a
/// conversation can revise it.
#[test]
fn an_installed_board_reads_back_as_its_draft() {
    let f = Fixture::new();
    let d = draft("speed", "Speed", GOOD);
    f.store.propose(&d, Origin::ModelClean, Utc::now()).unwrap();
    f.store
        .accept("speed", &d.digest(), Utc::now())
        .unwrap()
        .unwrap();
    let back = f.store.board_draft("speed").unwrap().unwrap();
    assert_eq!(back, d);
    assert!(f.store.board_draft("nothing").unwrap().is_none());
}
