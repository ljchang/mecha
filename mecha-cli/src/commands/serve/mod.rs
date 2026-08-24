//! `mecha serve` — the tailnet web surface (Phase 1: read-only).
//!
//! One process serves the built web app and a JSON summary of the stores.
//! Three rules carry the design (`docs/REMOTE-SURFACE-DESIGN.md`):
//!
//! - **The bind is 127.0.0.1 and there is no flag to widen it.** Reaching
//!   this from a phone is `tailscale serve`'s job; reaching it from the
//!   internet is nobody's.
//! - **Identity is the network, verified.** Every request must carry
//!   `Tailscale-User-Login` equal to `[web] owner_login` — the header
//!   `tailscale serve` injects for the authenticated tailnet user. Absent
//!   header, wrong value, or unset config fail closed: the server refuses to
//!   *start* without an owner, because a door with no owner check must not
//!   open at all.
//! - **Reads drive the CLI.** The summary shells out to `mecha review queues
//!   --json` and `mecha doctor --json` as child processes — one
//!   implementation per verb, nothing reachable here that a script cannot
//!   do, and the `depth: null` convention ("could not look" is not
//!   "nothing waiting") arrives for free because the verb already speaks it.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use axum::extract::State;
use axum::http::{HeaderValue, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};

use mecha_core::config::Config;

mod board;
mod chat;
mod files;
mod mail;
mod present;
mod review;

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Override `[web] port` for this run.
    #[arg(long)]
    pub port: Option<u16>,
    /// Override `[web] assets` (the built web app, `web/dist`) for this run.
    #[arg(long)]
    pub assets: Option<PathBuf>,
    /// Loopback port for the mounted voice facade — the OpenAI endpoint
    /// the Pipecat worker calls, sharing this process's agent and prompt
    /// cache (the unification's whole argument). 0 disables it.
    #[arg(long, default_value_t = 8990)]
    pub voice_port: u16,

    /// Voice runs act without per-call approval — the owner-present
    /// posture the standalone voice-serve had via --yes. Without it a
    /// mounted facade inherits the config's Ask, which a non-interactive
    /// run answers with Blocked: every voice tool call refused. Outbox
    /// routing is unaffected either way — sends still stage for review.
    #[arg(long)]
    pub voice_yes: bool,

    /// Override `[web] owner_login` for this run. Same trust as the config
    /// field — a flag on the owner's own process — and what lets a branch
    /// build serve while the live config stays parseable by older binaries
    /// that predate the `[web]` section.
    #[arg(long)]
    pub owner_login: Option<String>,
    /// Where the voice runner accepts WebRTC offers; `/api/offer` proxies
    /// to it. Loopback by construction of the default; empty disables.
    #[arg(long, default_value = "http://127.0.0.1:7860/api/offer")]
    pub offer_target: String,
}

#[derive(Clone)]
struct WebState {
    owner_login: Arc<String>,
    /// `None` when the agent failed to build: the dashboard still serves and
    /// the chat routes answer 503 naming the reason — fail to a lesser mode,
    /// never silently.
    chat: Option<Arc<chat::ChatState>>,
    review: Arc<review::ReviewState>,
    /// The voice runner's offer endpoint, or None when disabled.
    offer_target: Option<Arc<String>>,
}

pub async fn execute(args: Args) -> Result<()> {
    // Global config only, like a trigger run: this surface is the owner's
    // door, and a project file must have no say in it (config.rs strips
    // `[web]` from project layers as a second fence).
    let config = Config::load_global()?;

    let Some(owner) = args
        .owner_login
        .clone()
        .or_else(|| config.web.owner_login.clone())
    else {
        bail!(
            "[web] owner_login is not set, and mecha serve will not open a door with no \
             owner check.\nSet it in ~/.mecha/config.toml to the Tailscale login that may \
             drive this box, e.g.\n\n  [web]\n  owner_login = \"you@example.com\"\n\n\
             (`tailscale status --json | jq -r .Self.UserID` and the admin console list \
             logins; `tailscale serve` injects the matching Tailscale-User-Login header.)"
        );
    };

    let port = args.port.unwrap_or(config.web.port);
    let assets = args.assets.or(config.web.assets.clone());

    let chat = match chat::ChatState::build().await {
        Ok(c) => Some(Arc::new(c)),
        Err(e) => {
            tracing::warn!("chat is unavailable: {e:#}");
            eprintln!("warning: chat is unavailable — the dashboard still serves.\n  {e:#}");
            None
        }
    };
    let review = Arc::new(review::review_state(&config)?);
    let offer_target = Some(args.offer_target.trim())
        .filter(|t| !t.is_empty())
        .map(|t| Arc::new(t.to_string()));
    let state = WebState {
        owner_login: Arc::new(owner),
        chat,
        review,
        offer_target,
    };
    // Mount the voice facade on the same agent: one provider connection,
    // one cached prefix, two dialects. It rides this process's lifetime;
    // its own graceful drain runs after axum returns.
    let voice = match (&state.chat, args.voice_port) {
        (Some(chat), port) if port != 0 => {
            let (agent, provider, model, config, outbox_root) = chat.voice_parts();
            match crate::voice::Facade::new(
                agent,
                provider,
                model,
                config,
                outbox_root,
                None,
                true,
                args.voice_yes,
            ) {
                Ok(f) => {
                    let facade = Arc::new(f);
                    // Bind before announcing: a claim about a port must be
                    // the port's answer, not the plan's.
                    match facade.bind(port).await {
                        Ok(listener) => {
                            let stop = tokio_util::sync::CancellationToken::new();
                            let task = {
                                let facade = Arc::clone(&facade);
                                let stop = stop.clone();
                                tokio::spawn(async move { facade.serve(listener, stop).await })
                            };
                            println!("voice facade on http://127.0.0.1:{port} (shared agent)");
                            Some((facade, stop, task))
                        }
                        Err(e) => {
                            tracing::warn!("voice facade unavailable: {e:#}");
                            eprintln!("warning: voice facade could not bind — {e:#}");
                            None
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("voice facade unavailable: {e:#}");
                    None
                }
            }
        }
        _ => None,
    };

    let app = router(state.clone(), assets.as_deref());

    // 127.0.0.1 by construction — the address is not configurable.
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    match &assets {
        Some(dir) => tracing::info!(%addr, assets = %dir.display(), "mecha serve up"),
        None => tracing::info!(%addr, "mecha serve up (API only — no [web] assets configured)"),
    }
    println!(
        "mecha serve on http://{addr} (owner: {}) — front it with `tailscale serve {port}`",
        state.owner_login
    );

    let served = axum::serve(listener, app).await.context("serving");
    if let Some((facade, stop, task)) = voice {
        stop.cancel();
        facade.shutdown().await;
        let _ = task.await;
    }
    served?;
    Ok(())
}

/// Where every CLI child this server spawns runs. The serve unit's working
/// directory is the owner's home, which any child that builds a tool
/// surface *refuses* as a workspace — the jail must not be rooted over
/// `~/.mecha` — so children get the web producer directory instead: inside
/// the mecha home is fine (it is the workspace default), containing it is
/// what is refused. This was first fixed for mail alone, and the same
/// refusal promptly surfaced on the tasks board (`mecha tasks` reaches the
/// graph over MCP, which builds a workspace too): the fix belongs at the
/// spawn helpers, not at whichever route happened to fail first.
pub(super) fn child_cwd() -> Option<std::path::PathBuf> {
    let dir = mecha_core::work::producer_dir("web").ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// The whole surface, auth included, as a value — which is what lets the
/// guard be tested by driving the router directly instead of binding a port.
fn router(state: WebState, assets: Option<&std::path::Path>) -> Router {
    let api = Router::new()
        .route("/api/ping", get(ping))
        .route("/api/summary", get(summary))
        .route("/api/sessions", get(chat::sessions))
        .route("/api/chat/{key}", get(chat::transcript))
        .route("/api/chat/{key}/send", axum::routing::post(chat::send))
        .route("/api/chat/{key}/cancel", axum::routing::post(chat::cancel))
        .route("/api/chat/{key}/events", get(chat::events))
        .route("/api/chat/{key}/answer", axum::routing::post(chat::answer))
        .route("/api/chat/{key}/mode", axum::routing::post(chat::set_mode))
        .route(
            "/api/chat/{key}/upload",
            axum::routing::post(files::upload)
                // A phone photo is 3-10 MB; axum's 2 MB default refuses the
                // route's whole purpose. Bounded still: the jail is disk.
                .layer(axum::extract::DefaultBodyLimit::max(26_214_400)),
        )
        .route("/api/chat/{key}/file", get(files::download))
        .route("/api/outbox", get(review::list))
        .route("/api/outbox/{id}", get(review::detail))
        .route(
            "/api/outbox/{id}/approve",
            axum::routing::post(review::approve),
        )
        .route(
            "/api/outbox/{id}/reject",
            axum::routing::post(review::reject),
        )
        .route("/api/outbox/{id}/edit", axum::routing::post(review::edit))
        .route("/api/queue", get(review::queue))
        .route("/api/queue/classes", get(review::classes))
        .route("/api/queue/groups", get(review::groups))
        .route("/api/queue/sample", axum::routing::post(review::sample))
        .route("/api/queue/verdict", axum::routing::post(review::verdict))
        .route("/api/mail", get(mail::list))
        .route("/api/mail/read", get(mail::read))
        .route("/api/mail/act", axum::routing::post(mail::act))
        .route("/api/tasks", get(board::tasks))
        .route("/api/tasks/set", axum::routing::post(board::task_set))
        .route("/api/tasks/add", axum::routing::post(board::task_add))
        .route("/api/notes", axum::routing::post(board::note))
        .route("/api/find", get(board::find))
        .route("/api/offer", axum::routing::post(offer_proxy));

    let app = match assets {
        Some(dir) => api.fallback_service(tower_http::services::ServeDir::new(dir)),
        None => api,
    };

    app.layer(middleware::from_fn_with_state(state.clone(), owner_guard))
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}

/// The header `tailscale serve` injects for the authenticated tailnet user.
const TAILSCALE_LOGIN: &str = "tailscale-user-login";

/// Every request — static files included — must carry the owner's login.
///
/// There is deliberately no loopback exemption: everything that reaches this
/// process arrives over loopback (`tailscale serve` proxies to it), so the
/// header is the only thing distinguishing the owner's phone from anything
/// else that found the port. Fail closed on absence, not just mismatch.
async fn owner_guard(
    State(state): State<WebState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let presented = request
        .headers()
        .get(TAILSCALE_LOGIN)
        .and_then(|v| v.to_str().ok());
    if presented != Some(state.owner_login.as_str()) {
        return (StatusCode::FORBIDDEN, "not the owner\n").into_response();
    }
    next.run(request).await
}

/// The page renders third-party text next to buttons that will one day
/// release drafts, so the CSP is load-bearing, not hygiene: XSS here is an
/// approval clicked by script. `'unsafe-inline'` for styles only — Svelte
/// writes style attributes; scripts stay `'self'` with no exceptions, and
/// nothing may load from another origin (the page must work with no
/// internet at all — the tailnet is not the internet).
async fn security_headers(request: Request<axum::body::Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
             img-src 'self' data:; font-src 'self' data:; connect-src 'self'; \
             frame-ancestors 'none'",
        ),
    );
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    response
}

async fn ping() -> &'static str {
    "ok\n"
}

/// POST /api/offer — the page's WebRTC offer, forwarded to the loopback
/// voice runner. Same-origin for the browser (no CORS in the path at all)
/// and behind the owner guard like everything else; the runner's own
/// origin allowlist still covers its direct door. Body passed through
/// verbatim both ways — this is a pipe, not a participant.
async fn offer_proxy(State(state): State<WebState>, body: axum::body::Bytes) -> Response {
    let Some(target) = &state.offer_target else {
        return (StatusCode::NOT_FOUND, "voice offers are disabled\n").into_response();
    };
    let client = reqwest::Client::new();
    let sent = client
        .post(target.as_str())
        .header("content-type", "application/json")
        .body(body.to_vec())
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await;
    match sent {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            match resp.bytes().await {
                Ok(bytes) => (
                    status,
                    [("content-type", "application/json")],
                    bytes.to_vec(),
                )
                    .into_response(),
                Err(e) => {
                    (StatusCode::BAD_GATEWAY, format!("reading answer: {e}\n")).into_response()
                }
            }
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            format!("voice runner unreachable: {e}\n"),
        )
            .into_response(),
    }
}

/// What the Home screen renders: the five queues and doctor's findings,
/// each section independently `null` when its verb could not answer —
/// "could not look" must never render as "nothing waiting".
async fn summary(State(state): State<WebState>) -> Json<serde_json::Value> {
    let (queues, doctor) = tokio::join!(
        self_cli_json(&["review", "queues", "--json"], false),
        // Doctor exits 1 *with findings on stdout* when something is wrong —
        // that is an answer, not a failure.
        self_cli_json(&["doctor", "--json"], true),
    );
    let mut errors = Vec::new();
    let queues = queues.unwrap_or_else(|e| {
        errors.push(format!("review queues: {e}"));
        serde_json::Value::Null
    });
    let doctor = doctor.unwrap_or_else(|e| {
        errors.push(format!("doctor: {e}"));
        serde_json::Value::Null
    });
    Json(serde_json::json!({
        "owner": state.owner_login.as_str(),
        "queues": queues,
        "doctor": doctor,
        "errors": errors,
    }))
}

/// Run our own binary with `args` and parse its stdout as JSON.
///
/// `exit_one_ok` admits commands whose exit 1 means "findings" rather than
/// "failed" (doctor's contract). Ten seconds is generous for store reads and
/// short enough that a wedged child cannot hang the page.
async fn self_cli_json(args: &[&str], exit_one_ok: bool) -> Result<serde_json::Value> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::process::Command::new(crate::exe::self_exe())
            .args(args)
            .output(),
    )
    .await
    .context("timed out")?
    .context("spawning")?;

    let ok = output.status.success() || (exit_one_ok && output.status.code() == Some(1));
    if !ok {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "exit {:?}: {}",
            output.status.code(),
            stderr.lines().next().unwrap_or("no error output")
        );
    }
    serde_json::from_slice(&output.stdout).context("parsing JSON output")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::util::ServiceExt;

    fn test_router() -> Router {
        router(
            WebState {
                owner_login: Arc::new("owner@example.com".into()),
                chat: None,
                offer_target: None,
                review: Arc::new(review::ReviewState {
                    outbox_root: std::env::temp_dir().join("mecha-serve-test-outbox"),
                    sessions_dir: None,
                }),
            },
            None,
        )
    }

    fn request(header: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().uri("/api/ping");
        if let Some(v) = header {
            builder = builder.header("Tailscale-User-Login", v);
        }
        builder.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn a_request_without_the_login_header_is_refused() {
        let response = test_router().oneshot(request(None)).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn a_request_with_the_wrong_login_is_refused() {
        let response = test_router()
            .oneshot(request(Some("stranger@example.com")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn the_owner_gets_through_and_gets_the_security_headers() {
        let response = test_router()
            .oneshot(request(Some("owner@example.com")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let csp = response
            .headers()
            .get("content-security-policy")
            .expect("CSP on every response")
            .to_str()
            .unwrap();
        assert!(csp.contains("default-src 'self'"));
        assert!(
            !csp.contains("https:"),
            "no external origin may ever appear in the CSP"
        );
    }

    #[tokio::test]
    async fn the_mail_routes_sit_behind_the_owner_guard() {
        // New surface, same door: a probe without the header learns nothing,
        // not even that a mail queue exists.
        for uri in ["/api/mail", "/api/mail/read?thread=t&account=a"] {
            let response = test_router()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{uri}");
        }
    }

    #[tokio::test]
    async fn an_unknown_mail_verb_is_refused_before_an_argv_exists() {
        // The closed-verb match is the boundary: even the owner cannot make
        // this route spell a verb the match does not name.
        let response = test_router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/mail/act")
                    .header("Tailscale-User-Login", "owner@example.com")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"verb":"trash","thread":"t","account":"a"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn even_a_missing_route_is_refused_before_it_is_a_404() {
        // The guard wraps everything, static fallback included: an
        // unauthenticated probe learns nothing about what exists.
        let response = test_router()
            .oneshot(
                Request::builder()
                    .uri("/does-not-exist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
