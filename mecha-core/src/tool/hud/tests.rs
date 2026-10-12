use super::*;
use crate::agent::Taint;
use std::path::PathBuf;

/// A store with one SQLite source, `system`, whose `content` is `content`.
struct Fixture {
    root: PathBuf,
    store: Store,
}

impl Fixture {
    fn new(content: &str) -> Fixture {
        let root = std::env::temp_dir().join(format!("mecha-hud-tools-{}", uuid::Uuid::new_v4()));
        let hud = root.join("hud");
        std::fs::create_dir_all(&hud).unwrap();
        let db = root.join("series.sqlite");
        let c = rusqlite::Connection::open(&db).unwrap();
        c.execute_batch(
            "CREATE TABLE model_minute (at TEXT NOT NULL, kind INTEGER NOT NULL, gen_tps REAL);
             INSERT INTO model_minute VALUES ('2026-01-01 00:00:00', 1, 50.0);
             INSERT INTO model_minute VALUES (datetime('now', '-10 minutes'), 1, 88.25);",
        )
        .unwrap();
        std::fs::write(
            hud.join("sources.toml"),
            format!(
                "[source.system]\nkind = \"sqlite\"\npath = \"{}\"\ncontent = \"{content}\"\n",
                db.display()
            ),
        )
        .unwrap();
        Fixture {
            store: Store::at(&hud),
            root,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn ctx(taint: Option<Taint>) -> ToolCtx {
    ToolCtx {
        taint,
        ..Default::default()
    }
}

const LOADER: &str = "source = \"system\"\nschedule = \"* * * * *\"\nmax_rows = 100\n\
    query = \"SELECT at, gen_tps AS tps FROM model_minute ORDER BY at\"\n\n\
    [[column]]\nname = \"at\"\ntype = \"timestamp\"\n\n[[column]]\nname = \"tps\"\ntype = \"float\"\n";

fn draft_input(id: &str) -> Value {
    json!({
        "id": id,
        "spec": {
            "version": 1, "title": "Speed", "datasets": ["speed"],
            "panels": [{"type": "kpi", "title": "Latest", "dataset": "speed",
                        "value": {"op": "last", "field": "tps"}}]
        },
        "loaders": {"speed": LOADER}
    })
}

/// The overview is everything a model needs before drafting: each table's
/// schema with its newest rows (the format a guess would get wrong), the
/// boards, and the guide.
#[tokio::test]
async fn the_overview_shows_each_tables_schema_and_newest_rows() {
    let f = Fixture::new("owner");
    let out = HudSources::new(f.store.clone())
        .call(json!({}), &ctx(None))
        .await
        .unwrap();
    assert!(!out.is_error, "{}", out.content);
    assert!(
        out.content.contains("CREATE TABLE model_minute"),
        "{}",
        out.content
    );
    let newest = out.content.find("88.25").expect("the newest row");
    let oldest = out.content.find("2026-01-01").expect("the older row");
    assert!(newest < oldest, "newest first: {}", out.content);
    assert!(out.content.contains("None installed."));
    assert!(
        out.content.contains("# Drafting a dashboard"),
        "the guide rides along"
    );
    assert!(
        !out.external,
        "an owner source's rows are not someone else's text"
    );
}

/// A third-party source's sample rows taint the conversation, as a refresh
/// classifies them — the overview, a query and a preview alike.
#[tokio::test]
async fn a_third_party_source_marks_every_result_from_outside() {
    let f = Fixture::new("third-party");
    let overview = HudSources::new(f.store.clone())
        .call(json!({}), &ctx(None))
        .await
        .unwrap();
    assert!(overview.external);
    let query = HudSources::new(f.store.clone())
        .call(
            json!({"source": "system", "query": "SELECT count(*) FROM model_minute"}),
            &ctx(None),
        )
        .await
        .unwrap();
    assert!(query.external);
    let preview = HudPreview::new(f.store.clone())
        .call(draft_input("speed"), &ctx(None))
        .await
        .unwrap();
    assert!(!preview.is_error, "{}", preview.content);
    assert!(preview.external);
}

/// A query runs through the confined runner: rows back, a write refused, an
/// unknown source named with the ones that exist.
#[tokio::test]
async fn a_query_is_read_only_and_names_the_sources() {
    let f = Fixture::new("owner");
    let tool = HudSources::new(f.store.clone());
    let rows = tool
        .call(
            json!({"source": "system", "query": "SELECT kind, count(*) AS n FROM model_minute GROUP BY kind"}),
            &ctx(None),
        )
        .await
        .unwrap();
    assert!(!rows.is_error, "{}", rows.content);
    assert!(
        rows.content.contains("1 row(s). Columns: kind | n"),
        "{}",
        rows.content
    );
    let write = tool
        .call(
            json!({"source": "system", "query": "DELETE FROM model_minute"}),
            &ctx(None),
        )
        .await
        .unwrap();
    assert!(write.is_error, "{}", write.content);
    let unknown = tool
        .call(json!({"source": "nope", "query": "SELECT 1"}), &ctx(None))
        .await
        .unwrap();
    assert!(unknown.is_error);
    assert!(
        unknown.content.contains("Registered: system"),
        "{}",
        unknown.content
    );
}

/// A loader may come as an object as well as TOML text; and `id` alone
/// returns the dashboard's files to revise — the waiting proposal first.
#[tokio::test]
async fn a_board_can_be_read_back_and_a_loader_sent_as_an_object() {
    let f = Fixture::new("owner");
    let mut input = draft_input("speed");
    input["loaders"]["speed"] = json!({
        "source": "system", "schedule": "* * * * *", "max_rows": 100,
        "query": "SELECT at, gen_tps AS tps FROM model_minute ORDER BY at",
        "column": [{"name": "at", "type": "timestamp"}, {"name": "tps", "type": "float"}]
    });
    let preview = HudPreview::new(f.store.clone())
        .call(input.clone(), &ctx(None))
        .await
        .unwrap();
    assert!(!preview.is_error, "{}", preview.content);
    assert!(
        preview.content.contains("## speed — 2 row(s)"),
        "{}",
        preview.content
    );

    let none = HudPreview::new(f.store.clone())
        .call(json!({"id": "speed"}), &ctx(None))
        .await
        .unwrap();
    assert!(none.is_error, "nothing by that id yet");
    HudPropose::new(f.store.clone())
        .call(input, &ctx(None))
        .await
        .unwrap();
    let back = HudPreview::new(f.store.clone())
        .call(json!({"id": "speed"}), &ctx(None))
        .await
        .unwrap();
    assert!(!back.is_error, "{}", back.content);
    assert!(
        back.content.contains("proposed, waiting for the owner"),
        "{}",
        back.content
    );
    assert!(
        back.content.contains("\"title\": \"Speed\""),
        "{}",
        back.content
    );
    assert!(back.content.contains("loaders.speed:"), "{}", back.content);
}

/// Who proposed is the run's record, never the model's claim: a tainted
/// conversation's proposal — or one from a run with no record — is marked
/// untrusted.
#[tokio::test]
async fn a_proposals_origin_is_the_conversations_taint() {
    for (taint, origin) in [
        (Some(Taint::default()), Origin::ModelClean),
        (
            Some(Taint {
                untrusted: true,
                ..Taint::default()
            }),
            Origin::ModelUntrusted,
        ),
        (None, Origin::ModelUntrusted),
    ] {
        let f = Fixture::new("owner");
        let out = HudPropose::new(f.store.clone())
            .call(draft_input("speed"), &ctx(taint))
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("waits for the owner"),
            "{}",
            out.content
        );
        let p = f.store.proposal("speed").unwrap().unwrap().unwrap();
        assert_eq!(p.meta.origin, origin);
    }
}

/// The three are read-only — a web chat starts read-only and must be able
/// to draft (R18) — and the two that return rows say they carry private and
/// possibly third-party data.
#[test]
fn the_tools_declare_what_they_return() {
    let store = Store::at("/nonexistent");
    let sources = HudSources::new(store.clone());
    let preview = HudPreview::new(store.clone());
    let propose = HudPropose::new(store);
    assert!(sources.read_only() && preview.read_only() && propose.read_only());
    // `hud_propose` too: a refused draft is described with the rows its
    // other loaders returned (review of #659).
    for caps in [
        sources.capabilities(),
        preview.capabilities(),
        propose.capabilities(),
    ] {
        assert!(caps.private_data && caps.untrusted_input);
        assert_eq!(caps.egress, crate::tool::Egress::None);
    }
}

/// A waiting proposal read back carries the proposing conversation's taint:
/// text staged from a tainted chat must not reach a clean one unmarked
/// (review of #659). An installed board passed the owner's review.
#[tokio::test]
async fn a_proposal_read_back_carries_its_origin() {
    for (taint, outside) in [
        (
            Some(Taint {
                untrusted: true,
                ..Taint::default()
            }),
            true,
        ),
        (Some(Taint::default()), false),
    ] {
        let f = Fixture::new("owner");
        HudPropose::new(f.store.clone())
            .call(draft_input("speed"), &ctx(taint))
            .await
            .unwrap();
        let back = HudPreview::new(f.store.clone())
            .call(json!({"id": "speed"}), &ctx(None))
            .await
            .unwrap();
        assert!(!back.is_error, "{}", back.content);
        assert_eq!(back.external, outside, "{taint:?}");

        let p = f.store.proposal("speed").unwrap().unwrap().unwrap();
        f.store
            .accept("speed", &p.draft.digest(), chrono::Utc::now())
            .unwrap()
            .unwrap();
        let installed = HudPreview::new(f.store.clone())
            .call(json!({"id": "speed"}), &ctx(None))
            .await
            .unwrap();
        assert!(
            installed.content.contains("(installed)"),
            "{}",
            installed.content
        );
        assert!(!installed.external, "the owner accepted it");
    }
}
