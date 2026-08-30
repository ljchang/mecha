//! `mecha serve` — the tailnet web surface.
//!
//! One process serves the built web app and a JSON summary of the stores.
//! It shipped read-only ("Phase 1"), which the header claimed long after it
//! stopped being true — chat, the outbox, the board, the charter and the
//! learning store all write from here now. What did not change is *how*: a
//! write is a `mecha …` child process, on the third rule below.
//!
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
//! - **The CLI drives both directions.** The summary shells out to `mecha
//!   review queues --json` and `mecha doctor --json`, and a write shells out
//!   the same way (`mecha outbox approve`, `mecha rules retire`) — one
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
mod frontdoor;
mod mail;
mod present;
mod proposals;
mod questions;
mod review;
mod settings;

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
    /// Host directory of TTS cloning references (`[web] voices_dir`), or
    /// None when cloning is not configured on this box.
    voices_dir: Option<Arc<PathBuf>>,
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
        voices_dir: config.web.voices_dir.clone().map(Arc::new),
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
                crate::voice::Mount {
                    inject_voice_block: true,
                    approve_all: args.voice_yes,
                    // D3: a call that names one of this process's chat
                    // sessions speaks into it, so talking and typing are one
                    // conversation rather than two transcripts.
                    host: Some(Arc::new(chat::VoiceHost(Arc::clone(chat)))),
                },
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
        .route("/api/history", get(chat::history))
        .route("/api/resume", axum::routing::post(chat::resume))
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
        .route("/api/queue/items", get(review::items))
        .route("/api/queue/sample", axum::routing::post(review::sample))
        .route("/api/entity", get(board::entity))
        .route("/api/queue/shadow", get(review::shadow))
        .route(
            "/api/queue/shadow/verdict",
            axum::routing::post(review::shadow_verdict),
        )
        .route("/api/queue/verdict", axum::routing::post(review::verdict))
        .route("/api/queue/bind", axum::routing::post(review::bind))
        // The proposal stores: harness candidates, rule proposals, the
        // graph's entity proposals. One generic surface over
        // `commands::review::review_source`, so a store added to that table
        // reaches the phone without another handler.
        .route("/api/proposals", get(proposals::stores))
        .route("/api/proposals/{store}", get(proposals::list))
        .route("/api/proposals/{store}/{id}", get(proposals::detail))
        .route(
            "/api/proposals/{store}/{id}/accept",
            axum::routing::post(proposals::accept),
        )
        .route(
            "/api/proposals/{store}/{id}/reject",
            axum::routing::post(proposals::reject),
        )
        .route("/api/mail", get(mail::list))
        .route("/api/mail/inbox", get(mail::inbox))
        .route("/api/mail/compose", axum::routing::post(mail::compose))
        .route("/api/mail/read", get(mail::read))
        .route("/api/mail/act", axum::routing::post(mail::act))
        .route("/api/tasks", get(board::tasks))
        .route("/api/tasks/set", axum::routing::post(board::task_set))
        .route("/api/tasks/work", axum::routing::post(board::task_work))
        .route("/api/tasks/stop", axum::routing::post(board::task_stop))
        .route("/api/tasks/steer", axum::routing::post(board::task_steer))
        .route("/api/tasks/chat", axum::routing::post(board::task_chat))
        .route(
            "/api/tasks/handover",
            axum::routing::post(board::task_handover),
        )
        .route("/api/tasks/plan", axum::routing::post(board::task_plan))
        .route("/api/tasks/source", axum::routing::post(board::task_source))
        .route("/api/tasks/add", axum::routing::post(board::task_add))
        .route("/api/tasks/parse", axum::routing::post(board::task_parse))
        .route("/api/questions", get(questions::list))
        .route(
            "/api/questions/answer",
            axum::routing::post(questions::answer),
        )
        .route(
            "/api/questions/abandon",
            axum::routing::post(questions::abandon),
        )
        .route(
            "/api/settings/charter",
            get(settings::charter).post(settings::charter_save),
        )
        .route("/api/settings/rules", get(settings::rules))
        .route(
            "/api/settings/rules/retire",
            axum::routing::post(settings::rule_retire),
        )
        .route(
            "/api/settings/rules/restore",
            axum::routing::post(settings::rule_restore),
        )
        .route("/api/settings/reflections", get(settings::reflections))
        .route(
            "/api/settings/learning-report",
            get(settings::learning_report),
        )
        .route(
            "/api/settings/reflections/show",
            get(settings::reflection_show),
        )
        .route(
            "/api/settings/reflections/edit",
            axum::routing::post(settings::reflection_edit),
        )
        .route(
            "/api/settings/reflections/drop",
            axum::routing::post(settings::reflection_drop),
        )
        .route(
            "/api/settings/reflections/restore",
            axum::routing::post(settings::reflection_restore),
        )
        .route(
            "/api/settings/voice/clone",
            axum::routing::post(settings::voice_clone)
                // A cloning reference is a multi-megabyte WAV by design —
                // ~50s of 48 kHz mono s16 is ~5 MB — and axum's 2 MB
                // default would cut the recording the page itself asks for
                // at ~22s, with a bare 413 instead of any of the handler's
                // own refusals. Same reasoning as upload and dictate above;
                // the handler's MAX_CLONE_BYTES is the real ceiling.
                .layer(axum::extract::DefaultBodyLimit::max(
                    settings::MAX_CLONE_BYTES + 4096,
                )),
        )
        .route(
            "/api/settings/voice/clone/delete",
            axum::routing::post(settings::voice_clone_delete),
        )
        .route("/api/settings/voice", get(settings::voice))
        .route("/api/notes", get(board::notes).post(board::note))
        .route("/api/notes/edit", axum::routing::post(board::note_edit))
        .route("/api/frontdoor", get(frontdoor::list))
        .route("/api/frontdoor/read", get(frontdoor::read))
        .route("/api/frontdoor/act", axum::routing::post(frontdoor::act))
        .route("/api/find", get(board::find))
        .route("/api/related", get(board::related))
        .route("/api/timeline", get(board::timeline))
        .route(
            "/api/entity/alias",
            axum::routing::post(board::entity_alias),
        )
        .route(
            "/api/entity/unalias",
            axum::routing::post(board::entity_unalias),
        )
        .route(
            "/api/entity/merge",
            axum::routing::post(board::entity_merge),
        )
        .route(
            "/api/entity/create",
            axum::routing::post(board::entity_create),
        )
        .route("/api/facts", axum::routing::post(board::fact))
        .route(
            "/api/facts/retract",
            axum::routing::post(board::fact_retract),
        )
        .route(
            "/api/dictate",
            axum::routing::post(dictate)
                // A minute of 16 kHz mono 16-bit is ~2 MB; axum's default
                // refuses at 2 MB exactly, which is the wrong place to cut
                // off a long thought.
                .layer(axum::extract::DefaultBodyLimit::max(8_388_608)),
        )
        .route("/api/offer", axum::routing::post(offer_proxy));

    let app = match assets {
        Some(dir) => api.fallback_service(tower_http::services::ServeDir::new(dir)),
        None => api,
    };

    app.layer(middleware::from_fn_with_state(state.clone(), owner_guard))
        .layer(middleware::from_fn(security_headers))
        .layer(middleware::from_fn(cache_headers))
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
             media-src 'self' blob:; frame-ancestors 'none'",
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

/// `Cache-Control` for the built app — and the two halves take opposite
/// rules, because they are opposite kinds of file.
///
/// **The entry document is a pointer, and a cached pointer is a whole old
/// build.** `index.html` names content-hashed bundles that the next deploy's
/// `rsync --delete` removes, so a browser reusing it does not show a broken
/// page: it shows the *previous* app, rendering perfectly, missing whatever
/// shipped since. Reported 2026-08-29 as the settings page's learning
/// section being "gone" on the owner's phone, minutes after that section
/// deployed — nothing had regressed, and nothing looked wrong, which is the
/// silently-degrading shape rather than a failure anyone could act on.
///
/// It was reachable because the response carried **no `Cache-Control` at
/// all**, only `Last-Modified`, and a cache with no explicit freshness is
/// permitted to invent one (RFC 9111 §4.2.2 — conventionally a fraction of
/// the age since that date). `no-cache` is not "do not store": it is
/// "revalidate before reuse", and `ServeDir` already answers `304` to a
/// conditional request, so the cost is one round trip and no bytes.
///
/// **The hashed assets take the opposite rule for the same reason.** Their
/// names change whenever their bytes do, so they cannot go stale, and
/// leaving them unlabelled was costing a full re-download of the bundle on
/// every page load — ~200 kB of JS and ~88 kB of CSS, on a phone, over a
/// tailnet. `private` rather than `public` because every response here is
/// behind the owner guard; the browser caches either way, and a shared cache
/// has no business holding this box's bytes.
///
/// `/api/` is deliberately untouched. Those responses carry no
/// `Last-Modified` for a heuristic to work from, their freshness is the
/// store's business rather than this layer's, and a blanket header here
/// would be this middleware quietly deciding policy for every handler.
async fn cache_headers(request: Request<axum::body::Body>, next: Next) -> Response {
    // Taken before the request is consumed. Matching on "not an asset"
    // rather than on `/` alone is what catches the other unhashed entry
    // paths that do reach the server — `/index.html` itself, and
    // `/favicon.svg` out of `web/public/`. (A hash route like `/#graph`
    // arrives as plain `/`; the fragment is never sent, so it needs no help
    // from the broader match.)
    let is_asset = request.uri().path().starts_with("/assets/");
    let is_api = request.uri().path().starts_with("/api/");
    let mut response = next.run(request).await;
    if is_api {
        return response;
    }
    // **Only a response that is actually the file gets a freshness policy.**
    // This layer is the outermost one, so it sees `owner_guard`'s 403 and
    // `ServeDir`'s 404 as well — and an explicit `max-age` makes any status
    // storable (RFC 9111 §3), with `immutable` telling the browser not to
    // revalidate even on a manual reload. Labelling a 404 that way pins a
    // permanently broken app for a year, past every later deploy.
    //
    // The window is opened by the other half of this very fix: once the
    // document always revalidates, a load landing mid-`rsync` gets the *new*
    // document, asks for a bundle that has not been written yet, and would
    // cache that 404 immutably — the same "looks like it is working" failure
    // this middleware exists to remove, made unrecoverable instead of
    // self-healing on the next deploy.
    if !(response.status().is_success() || response.status() == StatusCode::NOT_MODIFIED) {
        return response;
    }
    // Set on whatever came back, `304 Not Modified` included: a revalidation
    // that answered without the header would leave the next cache lookup
    // right back where it started. (`is_success()` alone would drop it —
    // 304 is not in the 2xx range.)
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static(match is_asset {
            true => "private, max-age=31536000, immutable",
            false => "no-cache",
        }),
    );
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
/// POST /api/dictate — a WAV clip in, its words out, via the local Parakeet
/// STT (the transducer that CANNOT obey speech — see the voice research).
/// The page encodes 16 kHz mono WAV itself, so no transcoder runs here; the
/// audio never leaves the box, which is the whole argument against the
/// browser speech APIs that ship the clip to a third party.
async fn dictate(State(_state): State<WebState>, body: axum::body::Bytes) -> Response {
    if body.is_empty() {
        return (StatusCode::BAD_REQUEST, "empty audio\n").into_response();
    }
    // Multipart by hand: one part, one fixed server, and the workspace's
    // reqwest deliberately carries few features. The boundary needs no
    // randomness — nothing in a WAV clip can contain it.
    let boundary = "mecha-dictate-7f3a9c51e2b8";
    let mut form: Vec<u8> = Vec::with_capacity(body.len() + 256);
    form.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; \
             filename=\"clip.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        )
        .as_bytes(),
    );
    form.extend_from_slice(&body);
    form.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let sent = reqwest::Client::new()
        .post("http://127.0.0.1:8992/v1/audio/transcriptions")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(form)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await;
    match sent {
        Ok(resp) if resp.status().is_success() => match resp.bytes().await {
            Ok(bytes) => (
                StatusCode::OK,
                [("content-type", "application/json")],
                bytes.to_vec(),
            )
                .into_response(),
            Err(e) => (StatusCode::BAD_GATEWAY, format!("reading answer: {e}\n")).into_response(),
        },
        Ok(resp) => (
            StatusCode::BAD_GATEWAY,
            format!("stt answered {}\n", resp.status()),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            format!("stt unreachable — is mecha-parakeet up? {e}\n"),
        )
            .into_response(),
    }
}

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
                voices_dir: None,
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
    async fn the_settings_routes_sit_behind_the_owner_guard() {
        // The charter save is the only write on the web surface that lands
        // in a file every future run's prompt is built from, so it is the
        // route most worth pinning behind the door — with the reads and the
        // clone verbs beside it. A probe without the header learns nothing,
        // not even that these routes exist.
        for (method, uri) in [
            ("GET", "/api/settings/charter"),
            ("POST", "/api/settings/charter"),
            ("GET", "/api/settings/rules"),
            ("POST", "/api/settings/rules/retire"),
            ("POST", "/api/settings/rules/restore"),
            ("GET", "/api/settings/reflections"),
            ("GET", "/api/settings/learning-report"),
            ("GET", "/api/settings/reflections/show?id=x"),
            ("POST", "/api/settings/reflections/edit"),
            ("POST", "/api/settings/reflections/drop"),
            ("POST", "/api/settings/reflections/restore"),
            ("GET", "/api/settings/voice"),
            ("POST", "/api/settings/voice/clone?name=x"),
            ("POST", "/api/settings/voice/clone/delete"),
        ] {
            let response = test_router()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {uri}");
        }
    }

    #[tokio::test]
    async fn the_graph_routes_sit_behind_the_owner_guard() {
        // The graph tab reads the owner's whole private store — every
        // entity, fact and note — so a probe without the header learns
        // nothing, not even that a graph exists. The two new reads
        // (`related`, `timeline`) are pinned beside the ones that predate
        // them, because a route added later is exactly the one a guard
        // test written earlier cannot be covering — and the two fact
        // writes are the routes most worth pinning: they land live in the
        // store, by the owner's authority, which is exactly what a probe
        // must never borrow.
        for (method, uri) in [
            ("GET", "/api/entity?name=x"),
            ("GET", "/api/find?q=x"),
            ("GET", "/api/notes"),
            ("GET", "/api/related?name=x"),
            ("GET", "/api/timeline?name=x"),
            ("POST", "/api/facts"),
            ("POST", "/api/facts/retract"),
            ("POST", "/api/entity/alias"),
            ("POST", "/api/entity/unalias"),
            ("POST", "/api/entity/merge"),
            ("POST", "/api/entity/create"),
        ] {
            let response = test_router()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {uri}");
        }
    }

    #[tokio::test]
    async fn an_invalid_charter_save_is_refused_at_the_handler() {
        // The module doc's claim measured where it is made: a document the
        // runs' own reader refuses comes back 422 from the handler — the
        // validation happens before any path is even resolved, so a refusal
        // here is structurally a refusal to write. (A *valid* save is
        // deliberately not driven from a test: the handler writes to
        // `Charter::default_path()`, which is the developer's real home.)
        let dup =
            r#"{"raw":"[[line]]\nid = \"a\"\ntext = \"x\"\n[[line]]\nid = \"a\"\ntext = \"y\"\n"}"#;
        let response = test_router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/settings/charter")
                    .header("Tailscale-User-Login", "owner@example.com")
                    .header("content-type", "application/json")
                    .body(Body::from(dup))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn a_learning_verb_without_a_real_id_is_refused_before_any_child_runs() {
        // Both learning stores resolve by *prefix*, and `starts_with("")`
        // matches every record — `LearningStore::reflexion` and
        // `rules::find_rule` each carry that guard, and this is the third.
        // The point of measuring it here rather than only in the store is
        // that it is the one that runs *before* a child process is spawned:
        // a browser cannot reach the case at all, and a 422 rather than a
        // 404 is also what pins the route as wired.
        for (uri, body) in [
            ("/api/settings/reflections/drop", r#"{"id":""}"#),
            ("/api/settings/reflections/restore", r#"{"id":""}"#),
            ("/api/settings/rules/retire", r#"{"id":""}"#),
            ("/api/settings/rules/restore", r#"{"id":""}"#),
            (
                "/api/settings/reflections/edit",
                r#"{"id":"","text":"a lesson"}"#,
            ),
            // And never a leading dash: the id is a positional argument to
            // a clap command, where one is a flag rather than a value.
            ("/api/settings/rules/retire", r#"{"id":"--help"}"#),
        ] {
            let response = test_router()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(uri)
                        .header("Tailscale-User-Login", "owner@example.com")
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNPROCESSABLE_ENTITY,
                "{uri} {body}"
            );
        }
    }

    #[tokio::test]
    async fn an_empty_lesson_is_refused_at_the_handler() {
        // `edit_reflexion` refuses it too; refusing here as well is what
        // keeps the page from spawning a child to be told so.
        let response = test_router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/settings/reflections/edit")
                    .header("Tailscale-User-Login", "owner@example.com")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"id":"20260829T014200-ab12cd34","text":"   \n "}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    /// A router whose clone endpoints have somewhere to write, so the
    /// content-type boundary and the write path itself are testable — the
    /// default fixture's `voices_dir: None` refuses at the front door
    /// (correctly, and revealing nothing), which also means it cannot pin
    /// anything behind it.
    fn test_router_with_voices(dir: &std::path::Path) -> Router {
        router(
            WebState {
                owner_login: Arc::new("owner@example.com".into()),
                chat: None,
                offer_target: None,
                voices_dir: Some(Arc::new(dir.to_path_buf())),
                review: Arc::new(review::ReviewState {
                    outbox_root: std::env::temp_dir().join("mecha-serve-test-outbox"),
                    sessions_dir: None,
                }),
            },
            None,
        )
    }

    /// A router with a built app behind it: an entry document and one
    /// content-hashed asset, which is the whole shape the cache rule is
    /// about.
    fn test_router_with_assets(dir: &std::path::Path) -> Router {
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(
            dir.join("index.html"),
            "<!doctype html><script src=/assets/index-abc123.js></script>",
        )
        .unwrap();
        std::fs::write(dir.join("assets/index-abc123.js"), "console.log(1)").unwrap();
        router(
            WebState {
                owner_login: Arc::new("owner@example.com".into()),
                chat: None,
                offer_target: None,
                voices_dir: None,
                review: Arc::new(review::ReviewState {
                    outbox_root: std::env::temp_dir().join("mecha-serve-test-outbox"),
                    sessions_dir: None,
                }),
            },
            Some(dir),
        )
    }

    async fn cache_control_of(router: Router, uri: &str) -> Option<String> {
        let response = router
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header("Tailscale-User-Login", "owner@example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        response
            .headers()
            .get("cache-control")
            .map(|v| v.to_str().unwrap().to_string())
    }

    /// The entry document must revalidate and the hashed assets must not.
    ///
    /// The bug this pins: served with no `Cache-Control` at all, `index.html`
    /// is heuristically cacheable, and a browser reusing it renders the
    /// *previous* build — correctly, and missing whatever shipped since.
    /// Found on the owner's phone on 2026-08-29, minutes after a deploy, as
    /// a feature that had "gone".
    #[tokio::test]
    async fn the_entry_document_revalidates_and_the_hashed_assets_do_not() {
        let dir = std::env::temp_dir().join(format!("mecha-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let router = test_router_with_assets(&dir);

        assert_eq!(
            cache_control_of(router.clone(), "/").await.as_deref(),
            Some("no-cache"),
            "the entry document names hashed files the next deploy deletes"
        );
        // The same document by its own name. A hash route (`/#graph`) never
        // reaches the server as anything but `/`, which is the case that
        // matters here; there is no SPA fallback on this router, so an
        // unknown path is a 404 from `ServeDir` rather than the document.
        assert_eq!(
            cache_control_of(router.clone(), "/index.html")
                .await
                .as_deref(),
            Some("no-cache")
        );
        assert_eq!(
            cache_control_of(router.clone(), "/assets/index-abc123.js")
                .await
                .as_deref(),
            Some("private, max-age=31536000, immutable"),
            "a content-hashed name cannot go stale"
        );
        // The API is not a static file and this layer must not invent a
        // freshness policy for it.
        assert_eq!(cache_control_of(router, "/api/ping").await, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A response that is not the file must carry no freshness policy at
    /// all — and the 404 is the one that matters.
    ///
    /// This layer is the outermost, so it sees the owner guard's 403 and
    /// `ServeDir`'s 404 too. An explicit `max-age` makes any status storable
    /// (RFC 9111 §3) and `immutable` suppresses revalidation even on a manual
    /// reload, so a 404 labelled that way pins a broken app for a year that
    /// no later deploy can clear. The other half of this change is what opens
    /// the window: once the document always revalidates, a load landing
    /// mid-`rsync` asks for a bundle that has not been written yet.
    #[tokio::test]
    async fn a_refusal_or_a_missing_bundle_is_never_labelled_immutable() {
        let dir = std::env::temp_dir().join(format!("mecha-cache-miss-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let router = test_router_with_assets(&dir);

        // Mid-rsync: the document is new, this bundle has not landed yet.
        assert_eq!(
            cache_control_of(router.clone(), "/assets/index-not-yet.js").await,
            None,
            "a 404 cached for a year is this bug with a longer fuse"
        );

        // And the owner guard's refusal, which is not a file either.
        let refused = router
            .oneshot(
                Request::builder()
                    .uri("/assets/index-abc123.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            refused.headers().get("cache-control"),
            None,
            "a 403 must not be cached as though it were the bundle"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The header has to survive the revalidation it asks for. A `304` that
    /// answered without it would leave the next cache lookup exactly where
    /// this fix started.
    #[tokio::test]
    async fn a_revalidated_entry_document_still_carries_the_header() {
        let dir = std::env::temp_dir().join(format!("mecha-cache-304-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let router = test_router_with_assets(&dir);

        let first = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("Tailscale-User-Login", "owner@example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let last_modified = first
            .headers()
            .get("last-modified")
            .expect("ServeDir dates what it serves")
            .clone();

        let again = router
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("Tailscale-User-Login", "owner@example.com")
                    .header("If-Modified-Since", last_modified)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            again.status(),
            StatusCode::NOT_MODIFIED,
            "the no-cache round trip must cost no bytes"
        );
        assert_eq!(
            again.headers().get("cache-control").unwrap(),
            "no-cache",
            "a 304 that drops the header undoes the fix on the next lookup"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn tiny_wav(rate: u32, seconds: f64) -> Vec<u8> {
        let byte_rate = rate * 2;
        let data_len = (f64::from(byte_rate) * seconds) as u32;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data_len).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&byte_rate.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data_len.to_le_bytes());
        b.resize(b.len() + data_len as usize, 0);
        b
    }

    #[tokio::test]
    async fn a_valid_charter_save_lands_and_a_refused_one_leaves_the_old_bytes() {
        // The accepting half, previously untested "because the handler
        // writes to the developer's real home" — which `$MECHA_HOME` (the
        // env-locked guard) makes a non-reason. The property most worth
        // pinning is the second half: a refused save must leave the charter
        // that was already on disk byte-for-byte intact, because the module
        // doc's whole claim is that a refusal is a refusal to write.
        let home = crate::testenv::HomeGuard::new("serve-charter");
        let good = "[[line]]\nid = \"first\"\ntext = \"tell the truth early\"\n";
        let save = |raw: &str| {
            let body = serde_json::json!({ "raw": raw }).to_string();
            test_router().oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/settings/charter")
                    .header("Tailscale-User-Login", "owner@example.com")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
        };
        let ok = save(good).await.unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        let on_disk = home.dir.join("charter.toml");
        assert_eq!(std::fs::read_to_string(&on_disk).unwrap(), good);

        let refused = save(
            "[[line]]\nid = \"first\"\ntext = \"a\"\n[[line]]\nid = \"first\"\ntext = \"b\"\n",
        )
        .await
        .unwrap();
        assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            std::fs::read_to_string(&on_disk).unwrap(),
            good,
            "a refused save must leave the old charter byte-for-byte intact"
        );
    }

    #[tokio::test]
    async fn a_clone_without_the_wav_content_type_is_refused_before_the_write() {
        // The content-type check is the CSRF boundary for the one raw-Bytes
        // write: `audio/wav` is not a CORS-simple type, so requiring it
        // forces a cross-origin caller through a preflight nothing answers.
        // text/plain — the simple type a form post carries — must die at
        // 415 with nothing written, even carrying a perfectly valid WAV.
        let dir = std::env::temp_dir().join(format!("mecha-clone-ct-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let response = test_router_with_voices(&dir)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/settings/voice/clone?name=x")
                    .header("Tailscale-User-Login", "owner@example.com")
                    .header("content-type", "text/plain")
                    .body(Body::from(tiny_wav(16_000, 6.0)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert!(
            std::fs::read_dir(&dir).unwrap().next().is_none(),
            "a refused clone left something in the store"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_valid_clone_lands_and_an_invalid_one_leaves_no_trace() {
        let dir = std::env::temp_dir().join(format!("mecha-clone-ok-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Too short: refused by the duration check, nothing written.
        let short = test_router_with_voices(&dir)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/settings/voice/clone?name=guest")
                    .header("Tailscale-User-Login", "owner@example.com")
                    .header("content-type", "audio/wav")
                    .body(Body::from(tiny_wav(16_000, 2.0)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(short.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(std::fs::read_dir(&dir).unwrap().next().is_none());
        // Long enough: lands as <name>.wav, byte for byte.
        let ok = test_router_with_voices(&dir)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/settings/voice/clone?name=guest")
                    .header("Tailscale-User-Login", "owner@example.com")
                    .header("content-type", "audio/wav")
                    .body(Body::from(tiny_wav(16_000, 6.0)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(
            std::fs::read(dir.join("guest.wav")).unwrap(),
            tiny_wav(16_000, 6.0)
        );
        let _ = std::fs::remove_dir_all(&dir);
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
