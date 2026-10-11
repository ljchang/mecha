//! The outbox's dashboard previews, on their own origin
//! (`docs/LIVE-DASHBOARD-DESIGN.md` §6.5, R17).
//!
//! Releasing a dashboard is reviewed as **the rendered page with real numbers
//! in it**, and that page is a model-drafted spec drawn by a renderer that
//! runs script. It must not run on the origin that holds the outbox's send
//! button — the hazard `PUBLIC-SURFACE-RESEARCH.md` names — so a second
//! listener serves it, on a separate port that `tailscale serve` exposes as a
//! separate origin, framed by the app with `frame-src` naming exactly it.
//!
//! - **Owner only.** The same `Tailscale-User-Login` check as the app: the
//!   page shows private data.
//! - **Only a pending dashboard publish, and only its own bytes.** `/p/<item>/`
//!   serves an outbox item that is pending, a publish, and a `hud-…` bundle
//!   assembled under the work directory's `hud` producer — never a path an
//!   item names elsewhere — and every file is proved inside that bundle.
//! - **The board's rows now**, at `/p/<item>/data/<name>.json`: what release
//!   would put in front of a reader on its first refresh.
//! - **The factory's policy.** The interactive class's CSP, so a preview that
//!   renders here renders there; `frame-ancestors` names this machine's own
//!   doors and nothing else.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use axum::extract::{Path as UrlPath, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use mecha_core::outbox::{OutboxKind, OutboxStore};

/// The preview origin `[web] preview_origin` names, set once when serve
/// starts a preview listener. Read by the outbox detail to offer a preview;
/// unset (tests, a serve with no preview port), no preview is offered.
static ORIGIN: OnceLock<String> = OnceLock::new();

pub(super) fn set_origin(origin: &str) {
    let _ = ORIGIN.set(origin.trim_end_matches('/').to_string());
}

pub(super) fn origin() -> Option<&'static str> {
    ORIGIN.get().map(String::as_str)
}

/// The preview URL for an outbox item, or `None` when it is not a pending
/// dashboard publish or there is no preview origin.
pub(super) fn url_for(
    origin: Option<&str>,
    item: &mecha_core::outbox::OutboxItem,
) -> Option<String> {
    let origin = origin?;
    is_dashboard_publish(item).then(|| format!("{origin}/p/{}/", item.id))
}

fn is_dashboard_publish(item: &mecha_core::outbox::OutboxItem) -> bool {
    item.status == "pending"
        && item.kind == OutboxKind::Publish
        && item.tool.ends_with("__bundle_publish")
        && item.args["id"]
            .as_str()
            .is_some_and(|id| id.strip_prefix("hud-").is_some_and(|b| !b.is_empty()))
}

/// The app's `frame-src`: exactly the preview origin, or nothing.
pub(super) fn frame_src(origin: Option<&str>) -> String {
    match origin {
        Some(o) => format!("; frame-src {}", o.trim_end_matches('/')),
        None => String::new(),
    }
}

#[derive(Clone)]
struct Preview {
    owner_login: Arc<String>,
    outbox_root: PathBuf,
    /// Where `mecha hud publish` assembles bundles: only these are served.
    bundles_root: PathBuf,
    hud: mecha_core::hud::store::Store,
    csp: HeaderValue,
}

/// The interactive class's policy (the factory's `ContentClass::csp`), framed
/// only by `origin`'s own host: this machine's doors.
fn csp_for(origin: &str) -> String {
    let host = origin
        .split("://")
        .nth(1)
        .unwrap_or(origin)
        .split(['/', ':'])
        .next()
        .unwrap_or_default();
    format!(
        "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; \
         font-src 'self'; connect-src 'self'; frame-ancestors https://{host}:*; \
         base-uri 'none'; form-action 'none'"
    )
}

pub(super) fn router(
    owner_login: Arc<String>,
    outbox_root: PathBuf,
    origin: &str,
) -> anyhow::Result<Router> {
    let csp = HeaderValue::from_str(&csp_for(origin))?;
    let state = Preview {
        owner_login,
        outbox_root,
        bundles_root: mecha_core::work::producer_dir("hud")?,
        hud: mecha_core::hud::store::Store::open()?,
        csp,
    };
    Ok(Router::new()
        .route("/p/{item}", get(redirect))
        .route("/p/{item}/", get(root))
        .route("/p/{item}/{*path}", get(file))
        .with_state(state))
}

async fn redirect(UrlPath(item): UrlPath<String>) -> Response {
    (
        StatusCode::FOUND,
        [(header::LOCATION, format!("/p/{item}/"))],
    )
        .into_response()
}

async fn root(state: State<Preview>, item: UrlPath<String>, headers: HeaderMap) -> Response {
    serve(state, item.0, String::new(), headers).await
}

async fn file(
    state: State<Preview>,
    UrlPath((item, path)): UrlPath<(String, String)>,
    headers: HeaderMap,
) -> Response {
    serve(state, item, path, headers).await
}

fn missing() -> Response {
    (StatusCode::NOT_FOUND, "not found\n").into_response()
}

async fn serve(
    State(p): State<Preview>,
    item: String,
    path: String,
    headers: HeaderMap,
) -> Response {
    let presented = headers
        .get(super::TAILSCALE_LOGIN)
        .and_then(|v| v.to_str().ok());
    if presented != Some(p.owner_login.as_str()) {
        return (StatusCode::FORBIDDEN, "not the owner\n").into_response();
    }
    let answer = tokio::task::spawn_blocking(move || answer(&p, &item, &path)).await;
    match answer {
        Ok(Some((bytes, content_type, csp))) => {
            let mut response = (
                StatusCode::OK,
                [(header::CONTENT_TYPE, content_type)],
                bytes,
            )
                .into_response();
            let h = response.headers_mut();
            h.insert("content-security-policy", csp);
            h.insert(
                "x-content-type-options",
                HeaderValue::from_static("nosniff"),
            );
            h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
            h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Ok(None) => missing(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}\n")).into_response(),
    }
}

/// The bytes for `path` of outbox item `item`'s preview, or `None`.
fn answer(p: &Preview, item: &str, path: &str) -> Option<(Vec<u8>, &'static str, HeaderValue)> {
    let store = OutboxStore::open(&p.outbox_root).ok()?;
    let item = store.item(item).ok()?;
    if !is_dashboard_publish(&item) {
        return None;
    }
    let bundle = PathBuf::from(item.args["bundle"].as_str()?);
    let bundle = within(&p.bundles_root, &bundle)?;
    if let Some(file) = path.strip_prefix("data/") {
        let board = item.args["id"].as_str()?.strip_prefix("hud-")?;
        let name = file.strip_suffix(".json")?;
        let dataset = p.hud.dataset(board, name).ok()??;
        return Some((
            serde_json::to_vec(&dataset).ok()?,
            "application/json",
            p.csp.clone(),
        ));
    }
    let rel = if path.is_empty() { "index.html" } else { path };
    let file = within(&bundle, &bundle.join(rel))?;
    let bytes = std::fs::read(&file).ok()?;
    Some((bytes, content_type(&file), p.csp.clone()))
}

/// `path`, canonical, when it is inside `root` (both canonicalized) — never a
/// path an outbox item or a URL names outside it.
fn within(root: &Path, path: &Path) -> Option<PathBuf> {
    let root = root.canonicalize().ok()?;
    let path = path.canonicalize().ok()?;
    path.starts_with(&root).then_some(path)
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preview_is_framed_only_by_this_machines_doors() {
        let csp = csp_for("https://box.tail.ts.net:8444");
        assert!(
            csp.contains("frame-ancestors https://box.tail.ts.net:*"),
            "{csp}"
        );
        assert!(csp.contains("script-src 'self';"), "{csp}");
        assert!(!csp.contains("unsafe"), "{csp}");
        assert_eq!(frame_src(None), "");
        assert_eq!(
            frame_src(Some("https://box.tail.ts.net:8444/")),
            "; frame-src https://box.tail.ts.net:8444"
        );
    }

    #[test]
    fn a_file_is_served_only_from_inside_its_bundle() {
        let root = std::env::temp_dir().join(format!("mecha-preview-{}", uuid::Uuid::new_v4()));
        let bundle = root.join("lab-1");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("index.html"), "x").unwrap();
        std::fs::write(root.join("secret.txt"), "y").unwrap();
        assert!(within(&bundle, &bundle.join("index.html")).is_some());
        assert!(within(&bundle, &bundle.join("../secret.txt")).is_none());
        assert!(within(&root, &PathBuf::from("/etc/hostname")).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A real staged item, through the store, then set to the case at hand.
    fn item(
        status: &str,
        kind: OutboxKind,
        tool: &str,
        id: &str,
    ) -> mecha_core::outbox::OutboxItem {
        let root =
            std::env::temp_dir().join(format!("mecha-preview-outbox-{}", uuid::Uuid::new_v4()));
        let store = OutboxStore::open(&root).unwrap();
        let mut item = store
            .stage_publish_by_harness(tool, serde_json::json!({"id": id, "bundle": "/x"}))
            .unwrap();
        let _ = std::fs::remove_dir_all(&root);
        item.status = status.to_string();
        item.kind = kind;
        item
    }

    #[test]
    fn only_a_pending_dashboard_publish_is_previewed() {
        let o = Some("https://box.tail.ts.net:8444");
        let ok = item(
            "pending",
            OutboxKind::Publish,
            "factory__bundle_publish",
            "hud-lab",
        );
        assert_eq!(
            url_for(o, &ok).as_deref(),
            Some(format!("https://box.tail.ts.net:8444/p/{}/", ok.id).as_str())
        );
        assert_eq!(url_for(None, &ok), None, "no preview origin, no preview");
        for not in [
            item(
                "sent",
                OutboxKind::Publish,
                "factory__bundle_publish",
                "hud-lab",
            ),
            item(
                "pending",
                OutboxKind::Message,
                "factory__bundle_publish",
                "hud-lab",
            ),
            item("pending", OutboxKind::Publish, "mail__mail_send", "hud-lab"),
            item(
                "pending",
                OutboxKind::Publish,
                "factory__bundle_publish",
                "morning-brief",
            ),
            item(
                "pending",
                OutboxKind::Publish,
                "factory__bundle_publish",
                "hud-",
            ),
        ] {
            assert_eq!(
                url_for(o, &not),
                None,
                "{} {:?} {}",
                not.status,
                not.kind,
                not.tool
            );
        }
    }
}
