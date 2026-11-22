//! Settings → GitHub: the picker (`docs/GITHUB-DASHBOARD-DESIGN.md` step 6,
//! ruling G3), where the owner chooses which repositories the collector
//! watches and corrects the package mapping.
//!
//! **Two owner files, the owner's act.** The writes reach
//! `~/.mecha/github/repos.toml` and `packages.toml` and nothing else, through
//! `owner_guard` (the owner's login, and the page's `x-mecha-request` header
//! on every mutation) and the feature's gate, like every settings write. No
//! tool and no agent path writes either file; the collector only reads them.
//! Everything the write decides is `mecha_core::github::picker`'s, so this is
//! the HTTP and nothing else: a save carries the digest of the file its page
//! read, is refused when the file has changed since (409) or as it stands
//! cannot be used (409), and when a change, or the file it would make, is one
//! the collector's reader refuses (422) — and on success answers the page's
//! whole read again, so the page draws what the collector will now do.
//!
//! **The read never calls GitHub** (§2): it lists the inventory the collector
//! last wrote, so a slow API is a stale list with its age shown.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mecha_core::github::picker::{self, PackageChange, Refusal, SelectionChange};
use serde::Deserialize;

type St = State<super::WebState>;

#[allow(clippy::result_large_err)]
fn home() -> Result<std::path::PathBuf, Response> {
    mecha_core::work::mecha_home()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response())
}

/// The read, off the executor: the store is a SQLite file of up to a few
/// hundred rows, and its reader must not hold an async worker.
async fn view() -> Response {
    let home = match home() {
        Ok(h) => h,
        Err(r) => return r,
    };
    match tokio::task::spawn_blocking(move || picker::view(&home)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("reading the picker did not finish: {e}\n"),
        )
            .into_response(),
    }
}

/// GET /api/github/picker
pub async fn read(State(_state): St) -> Response {
    view().await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionSave {
    /// The digest of `repos.toml` as the page read it; `null` when it read
    /// no file.
    base: Option<String>,
    changes: Vec<SelectionChange>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackagesSave {
    base: Option<String>,
    changes: Vec<PackageChange>,
}

fn refused(r: Refusal) -> Response {
    let status = match r {
        Refusal::Stale | Refusal::Unusable(_) => StatusCode::CONFLICT,
        Refusal::Invalid(_) => StatusCode::UNPROCESSABLE_ENTITY,
        Refusal::BadRequest(_) => StatusCode::BAD_REQUEST,
        Refusal::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, format!("{r}\n")).into_response()
}

async fn saved(
    write: impl FnOnce(&std::path::Path) -> Result<Option<String>, Refusal> + Send + 'static,
) -> Response {
    let home = match home() {
        Ok(h) => h,
        Err(r) => return r,
    };
    match tokio::task::spawn_blocking(move || write(&home)).await {
        Ok(Ok(_)) => view().await,
        Ok(Err(r)) => refused(r),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("the save did not finish: {e}\n"),
        )
            .into_response(),
    }
}

/// POST /api/github/picker/selection
pub async fn save_selection(State(_state): St, Json(body): Json<SelectionSave>) -> Response {
    saved(move |home| picker::save_selection(home, body.base.as_deref(), &body.changes)).await
}

/// POST /api/github/picker/packages
pub async fn save_packages(State(_state): St, Json(body): Json<PackagesSave>) -> Response {
    saved(move |home| picker::save_packages(home, body.base.as_deref(), &body.changes)).await
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use mecha_core::github::{self, Collected, Repo, SelectionRead, Watched};
    use tower::util::ServiceExt;

    const LAB_LISTED: &str = "[[owner]]\nlogin = \"example-lab\"\ninclude = \"listed\"\n";

    fn repo(full: &str, fork: bool, archived: bool) -> Repo {
        let (owner, name) = full.split_once('/').unwrap();
        Repo {
            full_name: full.into(),
            owner: owner.into(),
            name: name.into(),
            fork,
            archived,
            private: false,
            default_branch: Some("main".into()),
            pushed_at: Some("2026-11-01T00:00:00Z".into()),
            listed: true,
            push: Some(true),
        }
    }

    /// A store as a collection under `LAB_LISTED` leaves it: five
    /// repositories listed, none watched.
    fn store(home: &std::path::Path) {
        std::fs::create_dir_all(github::dir(home)).unwrap();
        std::fs::write(github::selection_path(home), LAB_LISTED).unwrap();
        let mut c = Collected::new("2026-11-20T11:40:00Z".parse().unwrap());
        c.inventory = Some(
            [
                repo("example-lab/tool-a", false, false),
                repo("example-lab/tool-b", false, false),
                repo("example-lab/fork-of-upstream", true, false),
                repo("example-lab/old-site", false, true),
                repo("example-owner/notes", false, false),
            ]
            .into_iter()
            .map(|r| (r, false))
            .collect(),
        );
        c.selected = Some(0);
        github::record(&github::db_path(home), &c).unwrap();
    }

    fn request(method: &str, uri: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("Tailscale-User-Login", "owner@example.com")
            .header("x-mecha-request", "1")
            .header("content-type", "application/json")
            .body(if method == "GET" {
                Body::empty()
            } else {
                Body::from(body.to_string())
            })
            .unwrap()
    }

    async fn call(req: Request<Body>) -> (StatusCode, String) {
        let response = super::super::tests::test_router()
            .oneshot(req)
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn read() -> serde_json::Value {
        let (status, text) = call(request(
            "GET",
            "/api/github/picker",
            serde_json::Value::Null,
        ))
        .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        serde_json::from_str(&text).unwrap()
    }

    fn included(v: &serde_json::Value) -> Vec<String> {
        v["owners"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|o| o["repos"].as_array().unwrap())
            .filter(|r| r["included"] == true)
            .map(|r| r["full_name"].as_str().unwrap().to_string())
            .collect()
    }

    /// §7 step 6's acceptance, through the endpoint: select-all on an owner
    /// writes the file the collector's own reader then obeys, and the
    /// answer is the page's read of what the collector will now do.
    #[tokio::test]
    async fn select_all_through_the_endpoint_writes_the_file_the_collector_obeys() {
        let guard = crate::testenv::HomeGuard::new("serve-github-picker");
        let home = guard.dir.clone();
        store(&home);
        let before = read().await;
        assert_eq!(before["selection"]["state"], "read");
        assert_eq!(included(&before), Vec::<String>::new());
        assert!(before["store"]["last_collect"]["finished"].is_string());

        let (status, text) = call(request(
            "POST",
            "/api/github/picker/selection",
            serde_json::json!({
                "base": before["selection"]["digest"],
                "changes": [{ "op": "select_all", "login": "example-lab" }],
            }),
        ))
        .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let after: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            included(&after),
            ["example-lab/tool-a", "example-lab/tool-b"]
        );

        let SelectionRead::Read(s) = github::read_selection(&github::selection_path(&home)) else {
            panic!("the collector cannot read what the page wrote");
        };
        for (full, want) in [
            ("example-lab/tool-a", true),
            ("example-lab/tool-b", true),
            ("example-lab/fork-of-upstream", false),
            ("example-lab/old-site", false),
            ("example-owner/notes", false),
        ] {
            let r = repo(full, full.contains("fork-of"), full.contains("old-site"));
            assert_eq!(s.selects(&r), want, "{full}");
        }
        let w = Watched::read(&home);
        assert!(w.contains("example-lab", "tool-a") && w.contains("example-lab", "tool-b"));
        assert!(!w.contains("example-lab", "fork-of-upstream"));
        assert!(!w.contains("example-owner", "notes"));
    }

    /// Owner-only, and the page's own request only: without the login, or
    /// without the page's header, or from another site, nothing is read and
    /// nothing written.
    #[tokio::test]
    async fn the_picker_is_the_owners_and_the_pages_alone() {
        let guard = crate::testenv::HomeGuard::new("serve-github-picker-gate");
        let home = guard.dir.clone();
        store(&home);
        let body = serde_json::json!({
            "base": mecha_core::github::picker::digest(LAB_LISTED),
            "changes": [{ "op": "select_all", "login": "example-lab" }],
        });
        for (login, verified, site) in [
            (false, true, None),
            (true, false, None),
            (true, true, Some("cross-site")),
            (true, true, Some("same-site")),
        ] {
            for uri in [
                "/api/github/picker/selection",
                "/api/github/picker/packages",
            ] {
                let mut req = Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json");
                if login {
                    req = req.header("Tailscale-User-Login", "owner@example.com");
                }
                if verified {
                    req = req.header("x-mecha-request", "1");
                }
                if let Some(site) = site {
                    req = req.header("sec-fetch-site", site);
                }
                let (status, text) = call(req.body(Body::from(body.to_string())).unwrap()).await;
                assert_eq!(
                    status,
                    StatusCode::FORBIDDEN,
                    "{uri} {login} {verified} {site:?}: {text}"
                );
            }
        }
        // Someone else's login.
        let mut req = request("POST", "/api/github/picker/selection", body.clone());
        req.headers_mut().insert(
            "Tailscale-User-Login",
            "someone@example.com".parse().unwrap(),
        );
        assert_eq!(call(req).await.0, StatusCode::FORBIDDEN);
        let anonymous = Request::builder()
            .uri("/api/github/picker")
            .body(Body::empty())
            .unwrap();
        assert_eq!(call(anonymous).await.0, StatusCode::FORBIDDEN);
        assert_eq!(
            std::fs::read_to_string(github::selection_path(&home)).unwrap(),
            LAB_LISTED
        );
        assert!(!github::usage::packages_path(&home).exists());
    }

    /// A stale page is 409, a change the collector's reader refuses 422, a
    /// change outside the closed set a client error — each writing nothing.
    #[tokio::test]
    async fn refused_saves_write_nothing_and_say_why() {
        let guard = crate::testenv::HomeGuard::new("serve-github-picker-refused");
        let home = guard.dir.clone();
        store(&home);
        let path = github::selection_path(&home);
        let stale = serde_json::json!({
            "base": mecha_core::github::picker::digest("something older"),
            "changes": [{ "op": "select_all", "login": "example-lab" }],
        });
        let (status, text) = call(request("POST", "/api/github/picker/selection", stale)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{text}");
        assert!(text.contains("changed since this page read it"), "{text}");

        let digest = mecha_core::github::picker::digest(LAB_LISTED);
        let bad = serde_json::json!({
            "base": digest,
            "changes": [{ "op": "repo", "name": "example-lab/..", "include": true }],
        });
        let (status, _) = call(request("POST", "/api/github/picker/selection", bad)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        let unknown = serde_json::json!({
            "base": digest,
            "changes": [{ "op": "replace_file", "text": "[[owner]]" }],
        });
        let (status, _) = call(request("POST", "/api/github/picker/selection", unknown)).await;
        assert!(status.is_client_error(), "{status}");
        let extra = serde_json::json!({ "base": digest, "changes": [], "raw": "x" });
        let (status, _) = call(request("POST", "/api/github/picker/selection", extra)).await;
        assert!(status.is_client_error(), "{status}");
        let none = serde_json::json!({ "base": digest, "changes": [] });
        let (status, _) = call(request("POST", "/api/github/picker/selection", none)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), LAB_LISTED);

        let name = serde_json::json!({
            "base": null,
            "changes": [{ "repo": "example-lab/tool-a", "registry": "pypi", "to": { "name": "a b" } }],
        });
        let (status, _) = call(request("POST", "/api/github/picker/packages", name)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!github::usage::packages_path(&home).exists());
    }

    /// A package correction through the endpoint lands in the file step 5's
    /// reader reads, and the page's answer draws it as the owner's.
    #[tokio::test]
    async fn a_package_correction_through_the_endpoint_is_the_owners_line() {
        let guard = crate::testenv::HomeGuard::new("serve-github-picker-packages");
        let home = guard.dir.clone();
        store(&home);
        let (status, text) = call(request(
            "POST",
            "/api/github/picker/packages",
            serde_json::json!({
                "base": null,
                "changes": [{ "repo": "example-lab/tool-a", "registry": "npm", "to": "none" }],
            }),
        ))
        .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let npm = v["owners"][0]["repos"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["full_name"] == "example-lab/tool-a")
            .unwrap()["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["registry"] == "npm")
            .cloned()
            .unwrap();
        assert_eq!(npm["state"], "owner_none");
        let github::usage::PackagesRead::Read(rules) =
            github::usage::read_packages(&github::usage::packages_path(&home))
        else {
            panic!("step 5's reader cannot read what the page wrote");
        };
        assert!(rules[0].none);
    }

    /// A store that cannot be read is said, not drawn as an empty list.
    #[tokio::test]
    async fn an_unreadable_store_is_said() {
        let guard = crate::testenv::HomeGuard::new("serve-github-picker-unreadable");
        let home = guard.dir.clone();
        std::fs::create_dir_all(github::dir(&home)).unwrap();
        std::fs::write(github::db_path(&home), b"not sqlite at all, not even close").unwrap();
        let v = read().await;
        assert_eq!(v["store"]["state"], "unreadable", "{v}");
        assert!(v["store"]["error"].as_str().is_some_and(|e| !e.is_empty()));
        assert_eq!(v["selection"]["state"], "absent");
    }
}
