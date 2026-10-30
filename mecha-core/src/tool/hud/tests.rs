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

/// A context as the loop stamps it: `taint` is the turn's conservative
/// union, `recorded_taint` the conversation before the batch. Tests here set
/// both alike; `agent.rs`'s loop test pins how the loop sets them apart.
fn ctx(taint: Option<Taint>) -> ToolCtx {
    ToolCtx {
        taint,
        recorded_taint: taint,
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
    // A proposal returns nothing read from the source — not even a count —
    // so it carries nothing from outside (review of #659, pass 3).
    let propose = HudPropose::new(f.store.clone())
        .call(draft_input("speed"), &ctx(None))
        .await
        .unwrap();
    assert!(!propose.is_error, "{}", propose.content);
    assert!(!propose.external);
    assert!(!propose.content.contains("88.25"), "{}", propose.content);
}

/// A proposal that is waiting but no longer passes its checks is said, not
/// read as "none waiting" with the installed board answered instead.
#[tokio::test]
async fn a_broken_waiting_proposal_is_a_finding_not_absent() {
    let f = Fixture::new("owner");
    HudPropose::new(f.store.clone())
        .call(draft_input("speed"), &ctx(None))
        .await
        .unwrap();
    std::fs::write(f.store.root().join("proposals/speed/hud.json"), "{").unwrap();
    let back = HudPreview::new(f.store.clone())
        .call(json!({"id": "speed"}), &ctx(None))
        .await
        .unwrap();
    assert!(back.is_error, "{}", back.content);
    assert!(
        back.content.contains("no longer passes"),
        "{}",
        back.content
    );
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
    for caps in [sources.capabilities(), preview.capabilities()] {
        assert!(caps.private_data && caps.untrusted_input);
        assert_eq!(caps.egress, crate::tool::Egress::None);
    }
    // `hud_propose` returns no source values, and must not declare reach:
    // its own declaration would fold into the taint it records (review of
    // #659, pass 3).
    let caps = propose.capabilities();
    assert!(!caps.private_data && !caps.untrusted_input);
    assert_eq!(caps.egress, crate::tool::Egress::None);
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

/// A refused draft names each loader's outcome and reason, never a value:
/// the working loader's rows stay out of the result.
#[tokio::test]
async fn a_refused_proposal_carries_no_rows() {
    let f = Fixture::new("third-party");
    let mut input = draft_input("speed");
    input["spec"]["datasets"] = json!(["speed", "broken"]);
    input["loaders"]["broken"] = json!(LOADER.replace("FROM model_minute", "FROM no_such_table"));
    let out = HudPropose::new(f.store.clone())
        .call(input, &ctx(None))
        .await
        .unwrap();
    assert!(out.is_error, "{}", out.content);
    assert!(out.content.contains("broken: failed"), "{}", out.content);
    assert!(
        !out.content.contains("no_such_table"),
        "a third-party source's schema names stay out: {}",
        out.content
    );
    assert!(out.content.contains("speed: runs"), "{}", out.content);
    assert!(!out.content.contains("88.25"), "{}", out.content);
    assert!(!out.external);
}

/// `look` is offered only where it was given — a model that can see — and
/// asked for where it was not, or on a draft that is not clean, it says why
/// there is no picture rather than failing the preview.
#[tokio::test]
async fn look_is_offered_only_where_given_and_says_why_not() {
    let f = Fixture::new("owner");
    let blind = HudPreview::new(f.store.clone());
    assert!(blind.input_schema()["properties"].get("look").is_none());
    let mut input = draft_input("speed");
    input["look"] = json!(true);
    let out = blind.call(input.clone(), &ctx(None)).await.unwrap();
    assert!(!out.is_error, "{}", out.content);
    assert!(
        out.content.contains("No picture: drawing is not offered"),
        "{}",
        out.content
    );

    let seeing = HudPreview::new(f.store.clone()).with_look(Look {
        renderer: std::path::PathBuf::from("/nonexistent/hud-renderer"),
        timezone: None,
    });
    assert!(seeing.input_schema()["properties"].get("look").is_some());
    let mut broken = input.clone();
    broken["spec"]["title"] = json!("");
    broken["spec"]["datasets"] = json!(["nope"]);
    let out = seeing.call(broken, &ctx(None)).await.unwrap();
    assert!(out.is_error);
    assert!(
        out.content.contains("No picture until the draft passes"),
        "{}",
        out.content
    );
    // A clean draft with no renderer built: said, and still a clean preview.
    let out = seeing.call(input, &ctx(None)).await.unwrap();
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("(No picture: "), "{}", out.content);

    // `id` and `look` alone draw the board under that id; with nothing
    // there, it says so rather than returning a file listing.
    let out = seeing
        .call(json!({"id": "nothing_here", "look": true}), &ctx(None))
        .await
        .unwrap();
    assert!(out.is_error);
    assert!(out.content.contains("to draw"), "{}", out.content);
    HudPropose::new(f.store.clone())
        .call(draft_input("speed"), &ctx(None))
        .await
        .unwrap();
    let out = seeing
        .call(json!({"id": "speed", "look": true}), &ctx(None))
        .await
        .unwrap();
    assert!(
        out.content.contains("passes its checks"),
        "a preview of the waiting proposal: {}",
        out.content
    );
    // The proposal above ran with no taint record, which loads untrusted, so
    // its drawing carries that origin as `id` alone does (review of #662).
    assert!(
        out.external,
        "a proposal not from a clean conversation is from outside"
    );
    assert!(out.content.contains("(No picture: "), "{}", out.content);
}

/// Where this machine can draw (a browser, `bwrap`, the built renderer and
/// `mecha` beside the test binary), a clean draft comes back with its
/// picture attached. Skipped otherwise, unless `MECHA_TEST_REQUIRE_BACKENDS=1`.
#[tokio::test]
async fn a_clean_draft_is_drawn_and_attached() {
    let require = std::env::var("MECHA_TEST_REQUIRE_BACKENDS").as_deref() == Ok("1");
    let renderer = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../web/dist")
        .join(crate::hud::publish::RENDERER_DIR);
    let built_mecha = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent()?.parent().map(|d| d.join("mecha")))
        .filter(|p| p.is_file());
    if crate::hud::render::Renderer::find().is_err()
        || !renderer.join("hud.js").is_file()
        || built_mecha.is_none()
    {
        assert!(!require, "this machine cannot draw a preview");
        eprintln!("skipped: no browser, bwrap, built renderer or built mecha");
        return;
    }
    // `Renderer::find` serves through `current_exe`, which in a test is the
    // test binary; the tool path is exercised through the store directly.
    let f = Fixture::new("owner");
    let draft = draft_of(&draft_input("speed")).unwrap();
    let preview = f.store.preview(&draft).unwrap();
    let with = crate::hud::render::Renderer {
        mecha: built_mecha.unwrap(),
        ..crate::hud::render::Renderer::find().unwrap()
    };
    let shot = f
        .store
        .picture(&draft, &preview, &renderer, &with, chrono::Utc::now())
        .unwrap();
    assert!(!shot.cut, "a one-panel board fits");
    let img = image::load_from_memory(&shot.png).unwrap();
    assert_eq!(img.width(), crate::hud::render::WIDTH);
    assert!(img.height() > 100, "the board was drawn, not a blank page");
}

/// Proposing a board is a store write, previewing one a read, and both are
/// `read_only()`: the flag cannot tell them apart, the effect class does.
#[test]
fn a_proposal_is_a_store_write_and_a_preview_a_read() {
    use crate::tool::effect::{class_of, EffectClass};
    let f = Fixture::new("owner");
    let propose = HudPropose::new(f.store.clone());
    let preview = HudPreview::new(f.store.clone());
    assert!(propose.read_only() && preview.read_only());
    assert_eq!(class_of(&propose, false), EffectClass::StoreWrite);
    assert_eq!(class_of(&preview, false), EffectClass::Read);
    assert_eq!(
        class_of(&HudSources::new(f.store.clone()), false),
        EffectClass::Read
    );
}
