//! `GET /api/hud…` — installed dashboards, read-only, for the web app's `#hud`
//! (`docs/LIVE-DASHBOARD-DESIGN.md` §5.1, rung 1).
//!
//! | Route | Returns |
//! |---|---|
//! | `GET /api/hud` | every board: id, title, refusals, and each loader's `generated_at` and stale state |
//! | `GET /api/hud/{id}` | the validated spec |
//! | `GET /api/hud/{id}/data/{name}` | one dataset, with an `ETag`; `304` when unchanged |
//!
//! - **Read-only.** Installing, refreshing and validating stay in the CLI.
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
        Ok(boards) => Json(serde_json::json!({ "boards": boards })).into_response(),
        Err(e) => fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the dashboards could not be read: {e:#}"),
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
}
