//! Phase 4's file half: uploads into the session jail's `inbox/`,
//! authenticated downloads out of the jail. The phone's camera roll is the
//! motivating case.
//!
//! Uploads follow the Slack door's shape: the original lands in
//! `<workspace>/inbox/` and the *path* is what reaches the conversation —
//! announced by the page in the message text, never injected as content, so
//! the taint arms through `fs_read` (which already declares `private_data`)
//! rather than a parallel route someone has to label by hand.
//!
//! Downloads prove containment the way every model-supplied path does:
//! canonicalize, require containment, then open canonical components through
//! held directory descriptors without following replacement symlinks.
//! Outside — or missing, or a symlink pointing out — is the same
//! 404, deliberately: an authenticated probe should not learn which failure
//! it hit.
//!
//! **Nothing but images is served with a renderable content type.** A file
//! in the jail is model-written, possibly from third-party content; HTML
//! served same-origin would run script with the owner's cookie-less auth
//! (the tailnet header) against this very API. So images get their real
//! type and everything else is `application/octet-stream` with an
//! attachment disposition — inert bytes the phone can save.

use axum::body::{Body, Bytes};
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use mecha_core::workspace_files::WorkspaceFiles;
use serde::Deserialize;
use tokio::io::AsyncReadExt;

type St = State<super::WebState>;

/// A filename, tamed to one path component: the final segment of whatever
/// the phone said, characters outside `[A-Za-z0-9._-]` replaced, length
/// capped, and never dot-leading (a name like `.bashrc` — or `..` — must
/// not survive as itself). Sanitized rather than refused because the common
/// case is a camera roll's `IMG_0231 (1).jpg`, and a door that bounces the
/// normal case teaches people to rename files for their own harness.
fn tame_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let mut tamed: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(120)
        .collect();
    while tamed.starts_with('.') {
        tamed.remove(0);
    }
    if tamed.is_empty() {
        tamed = "upload".into();
    }
    tamed
}

#[derive(Deserialize)]
pub struct UploadQuery {
    pub name: String,
}

/// POST /api/chat/{key}/upload?name= — raw bytes into the session jail's
/// `inbox/`. Returns the workspace-relative path the page announces in the
/// message. A name collision gets a numbered sibling rather than an
/// overwrite: two photos taken seconds apart share a camera name, and the
/// second must not silently replace the first under a prompt that already
/// names it.
pub async fn upload(
    State(state): St,
    UrlPath(key): UrlPath<String>,
    Query(q): Query<UploadQuery>,
    body: Bytes,
) -> Response {
    if !super::chat::valid_key(&key) {
        return (StatusCode::BAD_REQUEST, "bad session key\n").into_response();
    }
    if body.is_empty() {
        return (StatusCode::BAD_REQUEST, "empty upload\n").into_response();
    }
    let ws = match super::chat::attachment_workspace(&state, &key, true).await {
        Ok(ws) => ws,
        Err(response) => return *response,
    };
    let name = tame_filename(&q.name);
    let bytes = body.len();
    let written =
        tokio::task::spawn_blocking(move || WorkspaceFiles::open(&ws)?.upload(&name, &body)).await;
    match written {
        Ok(Ok(path)) => Json(serde_json::json!({ "path": path, "bytes": bytes })).into_response(),
        Ok(Err(e)) => (StatusCode::CONFLICT, format!("cannot save upload: {e}\n")).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("upload failed: {e}\n"),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct DownloadQuery {
    pub path: String,
}

/// The types a browser may render inline. Images only: everything else in
/// the jail downloads as inert bytes (see the module header for why).
fn inline_image_type(path: &std::path::Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

/// GET /api/chat/{key}/file?path= — one file out of the session jail.
pub async fn download(
    State(state): St,
    UrlPath(key): UrlPath<String>,
    Query(q): Query<DownloadQuery>,
) -> Response {
    if !super::chat::valid_key(&key) {
        return (StatusCode::BAD_REQUEST, "bad session key\n").into_response();
    }
    let ws = match super::chat::attachment_workspace(&state, &key, false).await {
        Ok(ws) => ws,
        Err(response) => return *response,
    };
    let opened = tokio::task::spawn_blocking(move || {
        let (file, target) = WorkspaceFiles::open(&ws)?.read(&q.path)?;
        let len = file.metadata()?.len();
        Ok::<_, std::io::Error>((file, target, len))
    })
    .await;
    let (file, target, len) = match opened {
        Ok(Ok(opened)) => opened,
        _ => return (StatusCode::NOT_FOUND, "no such file\n").into_response(),
    };
    // Bound reads to the opened file's size and let HTTP backpressure pull
    // each chunk. No whole-file buffer and no disk I/O on the runtime worker.
    let file = tokio::fs::File::from_std(file).take(len);
    let body = Body::from_stream(tokio_util::io::ReaderStream::new(file));

    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let name = tame_filename(name);
    let mut response = match inline_image_type(&target) {
        Some(mime) => ([(header::CONTENT_TYPE, mime.to_string())], body).into_response(),
        None => (
            [
                (header::CONTENT_TYPE, "application/octet-stream".to_string()),
                (
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{name}\""),
                ),
            ],
            body,
        )
            .into_response(),
    };
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, len.into());
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filename_is_tamed_to_one_harmless_component() {
        assert_eq!(tame_filename("IMG_0231 (1).jpg"), "IMG_0231__1_.jpg");
        assert_eq!(tame_filename("../../etc/passwd"), "passwd");
        assert_eq!(tame_filename("..\\..\\evil.exe"), "evil.exe");
        assert_eq!(tame_filename(".bashrc"), "bashrc");
        assert_eq!(tame_filename("..."), "upload");
        assert_eq!(tame_filename(""), "upload");
        // A name that is nothing but separators cannot become an escape.
        assert_eq!(tame_filename("///"), "upload");
    }

    #[test]
    fn only_images_render_inline_everything_else_is_inert() {
        use std::path::Path;
        assert_eq!(
            inline_image_type(Path::new("a/shot.PNG")),
            Some("image/png")
        );
        // The load-bearing negatives: markup served same-origin would run
        // script with the owner's auth, so it must never carry its own type.
        for name in ["page.html", "img.svg", "doc.xml", "a.js", "x.pdf"] {
            assert_eq!(inline_image_type(Path::new(name)), None, "{name}");
        }
    }
}
