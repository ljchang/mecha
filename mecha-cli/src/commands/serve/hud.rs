//! `GET /api/hud…` — installed dashboards, read-only, for the web app's `#hud`
//! (`docs/LIVE-DASHBOARD-DESIGN.md` §5.1, rung 1).
//!
//! | Route | Returns |
//! |---|---|
//! | `GET /api/hud` | every board: id, title, refusals, and each loader's `generated_at` and stale state |
//! | `GET /api/hud/{id}` | the validated spec |
//! | `GET /api/hud/{id}/data/{name}` | one dataset, with an `ETag`; `304` when unchanged |
//!
//! | `GET /api/hud-proposals/{id}` | a proposed board: its checked spec, its loaders' queries, who proposed it, and the digest an accept names |
//! | `GET /api/hud-proposals/{id}/data/{name}` | one dataset of it, as it was when proposed |
//! | `POST /api/hud-proposals/{id}/accept` | `{"shown": <digest>}` — install it, or replace the board under its id, exactly as shown |
//! | `POST /api/hud-proposals/{id}/reject` | set it aside |
//!
//! - **Installed boards are read-only here.** Refreshing and validating stay
//!   in the CLI; the one write is the owner's accept or reject of a board a
//!   conversation proposed (`hud_propose`, design §5.3, R18).
//! - **An accept names what was shown.** The page sends back the digest it
//!   was given with the proposal, and the store recomputes it from the files:
//!   a proposal revised in between is refused, never installed unseen.
//! - **A URL segment never reaches the filesystem on its own say-so.** `{id}`
//!   must be one of the boards the store lists, and `{name}` one of the
//!   datasets the loaded spec declares, before either is joined to a path.
//! - **The spec served is the checked one.** `Installed::load` parses it
//!   through `Spec::parse`, so a file edited by hand after install that no
//!   longer passes is a refusal here, never drawn.

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use mecha_core::hud::store::{Dataset, Store};
use serde::Deserialize;

type St = State<super::WebState>;

/// GET /api/hud.
pub(super) async fn list(State(_): St) -> Response {
    blocking(|| Ok(list_at(&Store::open()?))).await
}

/// GET /api/hud/{id}.
pub(super) async fn board(State(_): St, Path(id): Path<String>) -> Response {
    blocking(move || Ok(board_at(&Store::open()?, &id))).await
}

/// GET /api/hud/{id}/data/{name}.
pub(super) async fn data(
    State(_): St,
    Path((id, name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let seen = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    blocking(move || Ok(data_at(&Store::open()?, &id, &name, seen.as_deref()))).await
}

/// GET /api/hud-proposals/{id}.
pub(super) async fn proposal(State(_): St, Path(id): Path<String>) -> Response {
    blocking(move || Ok(proposal_at(&Store::open()?, &id))).await
}

/// GET /api/hud-proposals/{id}/data/{name}.
pub(super) async fn proposal_data(
    State(_): St,
    Path((id, name)): Path<(String, String)>,
) -> Response {
    blocking(move || Ok(proposal_data_at(&Store::open()?, &id, &name))).await
}

#[derive(Deserialize)]
pub(super) struct Shown {
    shown: String,
}

/// POST /api/hud-proposals/{id}/accept.
pub(super) async fn accept(
    State(_): St,
    Path(id): Path<String>,
    Json(body): Json<Shown>,
) -> Response {
    blocking(move || Ok(accept_at(&Store::open()?, &id, &body.shown))).await
}

/// POST /api/hud-proposals/{id}/reject.
pub(super) async fn reject(State(_): St, Path(id): Path<String>) -> Response {
    blocking(move || Ok(reject_at(&Store::open()?, &id))).await
}

/// The store is files: read it off the async threads.
async fn blocking(f: impl FnOnce() -> anyhow::Result<Response> + Send + 'static) -> Response {
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => fail(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

fn fail(status: StatusCode, why: String) -> Response {
    (status, format!("{why}\n")).into_response()
}

pub(super) fn list_at(store: &Store) -> Response {
    match store.status(Utc::now()) {
        Ok(boards) => Json(serde_json::json!({
            "boards": boards,
            "proposals": proposals(store),
        }))
        .into_response(),
        Err(e) => fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the dashboards could not be read: {e:#}"),
        ),
    }
}

/// Each waiting proposal, for the list: a store that cannot be read says
/// so in the list rather than reading as "nothing proposed".
fn proposals(store: &Store) -> serde_json::Value {
    let ids = match store.proposal_ids() {
        Ok(ids) => ids,
        Err(e) => return serde_json::json!([{ "id": "", "refusals": [format!("{e:#}")] }]),
    };
    let rows: Vec<_> = ids
        .iter()
        .filter_map(|id| match store.proposal(id) {
            Ok(None) => None,
            Ok(Some(Ok(p))) => Some(serde_json::json!({
                "id": id,
                "title": p.installed.spec().title(),
                "replaces": p.replaces,
                "proposed_at": p.meta.proposed_at,
                "origin": p.meta.origin,
                "refusals": [],
            })),
            Ok(Some(Err(r))) => Some(serde_json::json!({
                "id": id,
                "refusals": r.0.iter().map(|r| r.to_string()).collect::<Vec<_>>(),
            })),
            Err(e) => Some(serde_json::json!({ "id": id, "refusals": [format!("{e:#}")] })),
        })
        .collect();
    serde_json::Value::Array(rows)
}

/// The refusal for an `id` that is not a waiting proposal — checked before
/// anything joins it to a path.
fn unproposed(store: &Store, id: &str) -> Option<Response> {
    match store.proposal_ids() {
        Ok(ids) if ids.iter().any(|i| i == id) => None,
        Ok(_) => Some(fail(StatusCode::NOT_FOUND, format!("no proposal {id:?}"))),
        Err(e) => Some(fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the proposals could not be listed: {e:#}"),
        )),
    }
}

pub(super) fn proposal_at(store: &Store, id: &str) -> Response {
    if let Some(r) = unproposed(store, id) {
        return r;
    }
    match store.proposal(id) {
        Ok(Some(Ok(p))) => {
            let loaders: Vec<_> = p
                .installed
                .loaders()
                .iter()
                .map(|(name, l)| {
                    serde_json::json!({
                        "name": name,
                        "source": l.source(),
                        "query": l.query(),
                        "max_rows": l.max_rows(),
                    })
                })
                .collect();
            Json(serde_json::json!({
                "id": id,
                "spec": p.installed.spec(),
                "loaders": loaders,
                "replaces": p.replaces,
                "proposed_at": p.meta.proposed_at,
                "origin": p.meta.origin,
                // Recomputed from the files: what an accept will compare.
                "digest": p.draft.digest(),
            }))
            .into_response()
        }
        Ok(None) => fail(StatusCode::NOT_FOUND, format!("no proposal {id:?}")),
        Ok(Some(Err(refusals))) => fail(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("the proposal {id:?} does not pass its checks: {refusals}"),
        ),
        Err(e) => fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the proposal {id:?} could not be read: {e:#}"),
        ),
    }
}

pub(super) fn proposal_data_at(store: &Store, id: &str, name: &str) -> Response {
    if let Some(r) = unproposed(store, id) {
        return r;
    }
    let declared = match store.proposal(id) {
        Ok(Some(Ok(p))) => p.installed.spec().datasets().iter().any(|d| d == name),
        _ => false,
    };
    if !declared {
        return fail(
            StatusCode::NOT_FOUND,
            format!("the proposal {id:?} declares no dataset {name:?}"),
        );
    }
    match store.proposal_dataset(id, name) {
        Ok(Some(d)) => (
            [
                (header::ETAG, etag(&d)),
                (header::CACHE_CONTROL, "no-cache".into()),
            ],
            Json(d),
        )
            .into_response(),
        Ok(None) => fail(StatusCode::NOT_FOUND, format!("{name:?} has no rows")),
        Err(e) => fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{name:?} could not be read: {e:#}"),
        ),
    }
}

pub(super) fn accept_at(store: &Store, id: &str, shown: &str) -> Response {
    if let Some(r) = unproposed(store, id) {
        return r;
    }
    // Revised since it was shown is a conflict; any other refusal (a source
    // unregistered since, files that no longer pass) is not, and says so
    // under its own status (review of #659).
    let revised = match store.proposal(id) {
        Ok(Some(Ok(p))) => p.draft.digest() != shown,
        _ => false,
    };
    match store.accept(id, shown, Utc::now()) {
        Ok(Ok(a)) => {
            let failed = a
                .refreshed
                .iter()
                .filter(|o| !matches!(o.event, mecha_core::hud::store::Event::Refreshed { .. }))
                .count();
            let verb = if a.replaced { "Replaced" } else { "Installed" };
            let mut output = format!("{verb} “{}”.", a.installed.spec().title());
            if failed > 0 {
                output.push_str(&format!(
                    " {failed} of its datasets did not refresh; `mecha hud list` says why."
                ));
            }
            Json(serde_json::json!({ "ok": true, "id": id, "output": output })).into_response()
        }
        Ok(Err(r)) if revised => fail(StatusCode::CONFLICT, r.to_string()),
        Ok(Err(r)) => fail(StatusCode::UNPROCESSABLE_ENTITY, r.to_string()),
        Err(e) => fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the proposal {id:?} could not be installed: {e:#}"),
        ),
    }
}

pub(super) fn reject_at(store: &Store, id: &str) -> Response {
    if let Some(r) = unproposed(store, id) {
        return r;
    }
    match store.reject(id) {
        Ok(_) => Json(serde_json::json!({ "ok": true, "output": "Rejected." })).into_response(),
        Err(e) => fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the proposal {id:?} could not be removed: {e:#}"),
        ),
    }
}

/// The refusal for an `id` that is not a board the store lists — checked
/// before anything joins it to a path. `None` is a board.
fn unknown(store: &Store, id: &str) -> Option<Response> {
    match store.board_ids() {
        Ok(ids) if ids.iter().any(|i| i == id) => None,
        Ok(_) => Some(fail(StatusCode::NOT_FOUND, format!("no dashboard {id:?}"))),
        Err(e) => Some(fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the dashboards could not be listed: {e:#}"),
        )),
    }
}

pub(super) fn board_at(store: &Store, id: &str) -> Response {
    if let Some(r) = unknown(store, id) {
        return r;
    }
    match store.load(id) {
        Ok(Ok(installed)) => Json(serde_json::json!({
            "id": installed.id(),
            "spec": installed.spec(),
        }))
        .into_response(),
        // Installed once, edited since into something that no longer passes:
        // never drawn, and the reason is the page's to show.
        Ok(Err(refusals)) => fail(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("dashboard {id:?} does not pass its checks: {refusals}"),
        ),
        Err(e) => fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("dashboard {id:?} could not be read: {e:#}"),
        ),
    }
}

/// A dataset's validator: its generation and when it was made. A refresh
/// always moves both, so equal tags are the same rows.
pub(super) fn etag(d: &Dataset) -> String {
    format!(
        "\"{}-{}\"",
        d.generation,
        d.generated_at.timestamp_nanos_opt().unwrap_or_default()
    )
}

pub(super) fn data_at(store: &Store, id: &str, name: &str, seen: Option<&str>) -> Response {
    if let Some(r) = unknown(store, id) {
        return r;
    }
    let installed = match store.load(id) {
        Ok(Ok(installed)) => installed,
        Ok(Err(refusals)) => {
            return fail(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("dashboard {id:?} does not pass its checks: {refusals}"),
            )
        }
        Err(e) => {
            return fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("dashboard {id:?} could not be read: {e:#}"),
            )
        }
    };
    if !installed.spec().datasets().iter().any(|d| d == name) {
        return fail(
            StatusCode::NOT_FOUND,
            format!("dashboard {id:?} declares no dataset {name:?}"),
        );
    }
    match store.dataset(id, name) {
        Ok(Some(d)) => {
            let tag = etag(&d);
            let same = seen.is_some_and(|s| s.split(',').any(|t| t.trim() == tag));
            if same {
                return (
                    StatusCode::NOT_MODIFIED,
                    [
                        (header::ETAG, tag),
                        (header::CACHE_CONTROL, "no-cache".into()),
                    ],
                )
                    .into_response();
            }
            (
                [
                    (header::ETAG, tag),
                    (header::CACHE_CONTROL, "no-cache".into()),
                ],
                Json(d),
            )
                .into_response()
        }
        Ok(None) => fail(
            StatusCode::NOT_FOUND,
            format!("{name:?} has not been produced yet"),
        ),
        Err(e) => fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{name:?} could not be read: {e:#}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::hud::loader::{Column, ColumnType};

    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A store with one board, `lab`, declaring one dataset, `visits`, whose
    /// data file is not written yet.
    fn lab() -> (Scratch, Store) {
        let root = std::env::temp_dir().join(format!("mecha-serve-hud-{}", uuid::Uuid::new_v4()));
        let draft = root.join("draft").join("lab");
        std::fs::create_dir_all(draft.join("loaders")).unwrap();
        std::fs::write(
            root.join("sources.toml"),
            format!(
                "[source.lab]\nkind = \"sqlite\"\npath = \"{}\"\ncontent = \"owner\"\n",
                root.join("lab.sqlite").display()
            ),
        )
        .unwrap();
        std::fs::write(
            draft.join("hud.json"),
            r#"{"version":1,"title":"Lab week","datasets":["visits"],
                "filters":[{"id":"site","label":"Site","dataset":"visits","field":"site"}],
                "panels":[{"type":"kpi","title":"Visits","dataset":"visits",
                           "value":{"op":"sum","field":"visits"}}]}"#,
        )
        .unwrap();
        std::fs::write(
            draft.join("loaders").join("visits.toml"),
            "source = \"lab\"\nschedule = \"0 * * * *\"\nmax_rows = 100\n\
             query = \"SELECT site, visits FROM visits\"\n\
             [[column]]\nname = \"site\"\ntype = \"string\"\n\
             [[column]]\nname = \"visits\"\ntype = \"integer\"\n",
        )
        .unwrap();
        let store = Store::at(&root);
        store
            .install(&draft, Some("lab"), Utc::now())
            .unwrap()
            .unwrap_or_else(|r| panic!("the fixture does not install: {r}"));
        (Scratch(root), store)
    }

    fn produce(store: &Store, generation: u64) -> Dataset {
        let d = Dataset {
            name: "visits".into(),
            generated_at: Utc::now(),
            generation,
            loader_digest: "fixture".into(),
            columns: vec![
                Column {
                    name: "site".into(),
                    kind: ColumnType::String,
                },
                Column {
                    name: "visits".into(),
                    kind: ColumnType::Integer,
                },
            ],
            rows: vec![vec!["North".into(), 4.into()]],
            external: false,
        };
        let dir = store.root().join("boards").join("lab").join("data");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("visits.json"), serde_json::to_vec(&d).unwrap()).unwrap();
        d
    }

    async fn body(r: Response) -> (StatusCode, HeaderMap, String) {
        let status = r.status();
        let headers = r.headers().clone();
        let bytes = axum::body::to_bytes(r.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            headers,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }

    #[tokio::test]
    async fn the_list_names_each_board_and_its_datasets_freshness() {
        let (_s, store) = lab();
        let (status, _, text) = body(list_at(&store)).await;
        assert_eq!(status, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["boards"][0]["id"], "lab");
        assert_eq!(v["boards"][0]["title"], "Lab week");
        assert_eq!(v["boards"][0]["loaders"][0]["name"], "visits");
    }

    /// The spec the page draws is the checked one, with the fields the
    /// renderer reads by name.
    #[tokio::test]
    async fn a_board_is_served_as_its_checked_spec() {
        let (_s, store) = lab();
        let (status, _, text) = body(board_at(&store, "lab")).await;
        assert_eq!(status, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["spec"]["datasets"][0], "visits");
        assert_eq!(v["spec"]["filters"][0]["field"], "site");
        assert_eq!(v["spec"]["panels"][0]["type"], "kpi");
        assert_eq!(v["spec"]["panels"][0]["value"]["op"], "sum");
        assert!(v["spec"].get("checked").is_none());
    }

    /// A URL segment reaches no path unless the store or the spec names it.
    #[tokio::test]
    async fn an_unknown_board_or_dataset_is_not_found_before_any_path() {
        let (_s, store) = lab();
        for id in ["nope", "../lab", "lab/..", ""] {
            assert_eq!(
                board_at(&store, id).status(),
                StatusCode::NOT_FOUND,
                "{id:?}"
            );
            assert_eq!(
                data_at(&store, id, "visits", None).status(),
                StatusCode::NOT_FOUND,
                "{id:?}"
            );
        }
        for name in ["other", "../hud", "visits.json"] {
            assert_eq!(
                data_at(&store, "lab", name, None).status(),
                StatusCode::NOT_FOUND,
                "{name:?}"
            );
        }
    }

    /// A dataset carries its validator; the same one back is a 304, a new
    /// refresh is the new rows.
    #[tokio::test]
    async fn a_dataset_revalidates_by_its_generation() {
        let (_s, store) = lab();
        let (status, _, text) = body(data_at(&store, "lab", "visits", None)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(text.contains("not been produced"), "{text}");

        let first = produce(&store, 1);
        let (status, headers, text) = body(data_at(&store, "lab", "visits", None)).await;
        assert_eq!(status, StatusCode::OK);
        let tag = headers[header::ETAG].to_str().unwrap().to_string();
        assert_eq!(tag, etag(&first));
        let got: Dataset = serde_json::from_str(&text).unwrap();
        assert_eq!(got.rows, first.rows);

        let again = data_at(&store, "lab", "visits", Some(&tag));
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);
        let listed = format!("\"0-0\", {tag}");
        assert_eq!(
            data_at(&store, "lab", "visits", Some(&listed)).status(),
            StatusCode::NOT_MODIFIED,
            "one of several tags"
        );

        produce(&store, 2);
        assert_eq!(
            data_at(&store, "lab", "visits", Some(&tag)).status(),
            StatusCode::OK,
            "a refresh is new rows"
        );
    }

    /// A spec edited by hand after install into one that fails its checks is
    /// refused, never drawn.
    #[tokio::test]
    async fn a_board_edited_into_a_refusal_is_not_served() {
        let (_s, store) = lab();
        let spec = store.root().join("boards").join("lab").join("hud.json");
        let text = std::fs::read_to_string(&spec).unwrap();
        std::fs::write(
            &spec,
            text.replace(
                r#""panels":["#,
                r#""panels":[{"type":"text","markdown":"[x](https://e.example)"},"#,
            ),
        )
        .unwrap();
        assert_eq!(
            board_at(&store, "lab").status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            data_at(&store, "lab", "visits", None).status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    /// A store whose `lab` source has rows, with a board `trend` proposed
    /// over it and nothing installed.
    fn proposed() -> (Scratch, Store, mecha_core::hud::Draft) {
        let root = std::env::temp_dir().join(format!("mecha-serve-hud-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = root.join("lab.sqlite");
        rusqlite::Connection::open(&db)
            .unwrap()
            .execute_batch(
                "CREATE TABLE visits (site TEXT, visits INTEGER);
                 INSERT INTO visits VALUES ('North', 4), ('South', 2);",
            )
            .unwrap();
        std::fs::write(
            root.join("sources.toml"),
            format!(
                "[source.lab]\nkind = \"sqlite\"\npath = \"{}\"\ncontent = \"owner\"\n",
                db.display()
            ),
        )
        .unwrap();
        let mut loaders = std::collections::BTreeMap::new();
        loaders.insert(
            "visits".to_string(),
            "source = \"lab\"\nschedule = \"0 * * * *\"\nmax_rows = 100\n\
             query = \"SELECT site, visits FROM visits\"\n\
             [[column]]\nname = \"site\"\ntype = \"string\"\n\
             [[column]]\nname = \"visits\"\ntype = \"integer\"\n"
                .to_string(),
        );
        let draft = mecha_core::hud::Draft::new(
            "trend",
            r#"{"version":1,"title":"Trend","datasets":["visits"],
                "panels":[{"type":"kpi","title":"Visits","dataset":"visits",
                           "value":{"op":"sum","field":"visits"}}]}"#
                .to_string(),
            loaders,
        )
        .unwrap();
        let store = Store::at(&root);
        let staged = store
            .propose(
                &draft,
                mecha_core::imagelib::Origin::ModelUntrusted,
                Utc::now(),
            )
            .unwrap();
        assert!(
            matches!(staged, mecha_core::hud::Proposed::Staged { .. }),
            "{staged:?}"
        );
        (Scratch(root), store, draft)
    }

    /// The list carries proposals beside boards, and a proposal is served
    /// with what the owner reviews: the spec, every query, who proposed it,
    /// and the rows it had.
    #[tokio::test]
    async fn a_proposal_is_served_with_its_queries_origin_and_rows() {
        let (_s, store, draft) = proposed();
        let (_, _, text) = body(list_at(&store)).await;
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["boards"], serde_json::json!([]));
        assert_eq!(v["proposals"][0]["id"], "trend");
        assert_eq!(v["proposals"][0]["origin"], "model_untrusted");

        let (status, _, text) = body(proposal_at(&store, "trend")).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["spec"]["title"], "Trend");
        assert_eq!(v["loaders"][0]["query"], "SELECT site, visits FROM visits");
        assert_eq!(v["digest"], draft.digest());
        assert_eq!(v["replaces"], false);

        let (status, _, text) = body(proposal_data_at(&store, "trend", "visits")).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let d: Dataset = serde_json::from_str(&text).unwrap();
        assert_eq!(d.rows.len(), 2);
        for (id, name) in [
            ("trend", "nope"),
            ("../trend", "visits"),
            ("nope", "visits"),
        ] {
            assert_eq!(
                proposal_data_at(&store, id, name).status(),
                StatusCode::NOT_FOUND,
                "{id}/{name}"
            );
        }
    }

    /// Accept installs only what was shown: a stale or forged digest is a
    /// conflict and installs nothing; the shown one installs and refreshes.
    #[tokio::test]
    async fn an_accept_installs_only_the_digest_that_was_shown() {
        let (_s, store, draft) = proposed();
        assert_eq!(
            accept_at(&store, "trend", "0123").status(),
            StatusCode::CONFLICT
        );
        // Shown as it is, but its source unregistered since: not a conflict.
        let sources = store.root().join("sources.toml");
        let kept = std::fs::read_to_string(&sources).unwrap();
        std::fs::write(&sources, "").unwrap();
        assert_eq!(
            accept_at(&store, "trend", &draft.digest()).status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        std::fs::write(&sources, kept).unwrap();
        assert_eq!(
            accept_at(&store, "trend", "").status(),
            StatusCode::CONFLICT
        );
        assert!(store.board_ids().unwrap().is_empty());
        assert_eq!(
            accept_at(&store, "../trend", &draft.digest()).status(),
            StatusCode::NOT_FOUND
        );

        let (status, _, text) = body(accept_at(&store, "trend", &draft.digest())).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        assert!(text.contains("Installed"), "{text}");
        assert_eq!(store.board_ids().unwrap(), vec!["trend".to_string()]);
        assert!(
            store.dataset("trend", "visits").unwrap().is_some(),
            "refreshed"
        );
        assert_eq!(proposal_at(&store, "trend").status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_reject_sets_the_proposal_aside_and_installs_nothing() {
        let (_s, store, _) = proposed();
        assert_eq!(reject_at(&store, "trend").status(), StatusCode::OK);
        assert_eq!(reject_at(&store, "trend").status(), StatusCode::NOT_FOUND);
        assert!(store.board_ids().unwrap().is_empty());
    }
}
