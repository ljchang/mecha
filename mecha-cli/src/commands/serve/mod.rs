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

use std::future::IntoFuture;
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

mod archive;
mod board;
mod chat;
mod files;
mod frontdoor;
pub(crate) mod incognito;
mod library;
mod mail;
mod model;
mod persona_chat;
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
    /// The image library's directory and the unlocks granted to it.
    library: Arc<library::LibraryState>,
}

pub async fn execute(args: Args) -> Result<()> {
    // Global config only, like a trigger run: this surface is the owner's
    // door, and a project file must have no say in it (config.rs strips
    // `[web]` from project layers as a second fence).
    let config = Config::load_global()?;
    // Latched before anything else can happen to this process's ancestry:
    // a `serve` a run's shell started stays not-a-person for its life, even
    // if it is reparented later (`chat::started_by_a_run`).
    if chat::started_by_a_run() {
        eprintln!(
            "note: this serve was started beneath a run's shell (or one the registry \
             cannot vouch for); its web chats are unattended and cannot close a task"
        );
    }

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
        Ok(c) => {
            let c = Arc::new(c);
            c.spawn_incognito_reaper();
            Some(c)
        }
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
        library: Arc::new(library::LibraryState::new(
            mecha_core::imagelib::Library::default_dir()?,
        )),
    };
    // 127.0.0.1 by construction — the address is not configurable.
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    let mut signals = crate::interrupt::ShutdownSignals::new()?;
    // Mount the voice facade on the same agent: one provider connection,
    // one cached prefix, two dialects. It rides this process's lifetime;
    // its graceful drain runs alongside the chat drain when the host stops.
    let voice = match (&state.chat, args.voice_port) {
        (Some(chat), port) if port != 0 => {
            let (follower, outbox_root) = chat.voice_parts();
            match crate::voice::Facade::new(
                follower,
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

    match &assets {
        Some(dir) => tracing::info!(%addr, assets = %dir.display(), "mecha serve up"),
        None => tracing::info!(%addr, "mecha serve up (API only — no [web] assets configured)"),
    }
    println!(
        "mecha serve on http://{addr} (owner: {}) — front it with `tailscale serve {port}`",
        state.owner_login
    );

    let stop = tokio_util::sync::CancellationToken::new();
    let server = axum::serve(listener, app)
        .with_graceful_shutdown(stop.clone().cancelled_owned())
        .into_future();
    tokio::pin!(server);
    let served = tokio::select! {
        result = &mut server => Some(result),
        _ = signals.recv() => None,
    };
    tracing::info!("mecha serve: stopping admission and recording active turns");
    let drained = signals
        .drain_or_force(async {
            if let Some(chat) = &state.chat {
                chat.stop().await;
            }
            if let Some((_, voice_stop, _)) = &voice {
                voice_stop.cancel();
            }
            stop.cancel();
            let drain_chat = async {
                if let Some(chat) = &state.chat {
                    chat.drain().await;
                }
            };
            let drain_voice = async {
                if let Some((facade, _, task)) = voice {
                    facade.shutdown().await;
                    let _ = task.await;
                }
            };
            tokio::join!(drain_chat, drain_voice);
            if let Some(chat) = &state.chat {
                chat.close_mcp().await;
            }
        })
        .await;
    if !drained {
        return Ok(());
    }
    // Model work and recording have finished. An incomplete HTTP request or
    // a client that stopped reading must not keep the daemon alive forever.
    match served {
        Some(result) => result.context("serving")?,
        None => match tokio::time::timeout(std::time::Duration::from_secs(5), &mut server).await {
            Ok(result) => result.context("serving")?,
            Err(_) => tracing::warn!("closing HTTP connections after shutdown drain"),
        },
    }
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
        .route("/api/today", get(today))
        .route(
            "/api/workflows/{id}/{action}",
            axum::routing::post(workflow_action),
        )
        .route(
            "/api/outbox/{id}/reconcile",
            axum::routing::post(review::reconcile),
        )
        .route("/api/sessions", get(chat::sessions))
        .route("/api/history", get(chat::history))
        .route("/api/sessions/{id}", axum::routing::delete(archive::delete))
        .route(
            "/api/sessions/{id}/archive",
            axum::routing::post(archive::archive),
        )
        .route(
            "/api/sessions/{id}/unarchive",
            axum::routing::post(archive::unarchive),
        )
        .route("/api/resume", axum::routing::post(chat::resume))
        .route("/api/incognito", axum::routing::post(chat::open_incognito))
        .route(
            "/api/incognito/{key}/end",
            axum::routing::post(chat::end_incognito),
        )
        .route(
            "/api/incognito/{key}/alive",
            axum::routing::post(chat::incognito_alive),
        )
        .route("/api/chat/{key}", get(chat::transcript).post(chat::open))
        .route("/api/chat/{key}/todo", get(chat::todo))
        .route("/api/chat/{key}/send", axum::routing::post(chat::send))
        .route("/api/chat/{key}/cancel", axum::routing::post(chat::cancel))
        // Persona chats: a door of their own, never the routes above
        // (`persona_chat`, `PERSONA-DESIGN.md` §3.2).
        .route(
            "/api/personas",
            get(persona_chat::list).post(persona_chat::create),
        )
        // Authoring: the owner's door, verbatim writes (`persona_chat`).
        .route("/api/personas/authoring", get(persona_chat::authoring))
        .route(
            "/api/personas/relationships",
            axum::routing::post(persona_chat::add_relationship),
        )
        .route(
            "/api/personas/groups",
            axum::routing::post(persona_chat::add_group),
        )
        .route(
            "/api/personas/{name}/files",
            get(persona_chat::files).post(persona_chat::save),
        )
        .route(
            "/api/personas/{name}/lock",
            axum::routing::post(persona_chat::lock),
        )
        .route(
            "/api/personas/{name}/chats",
            get(persona_chat::history).post(persona_chat::open),
        )
        .route(
            "/api/personas/{name}/resume",
            axum::routing::post(persona_chat::resume),
        )
        .route("/api/persona-chat/{key}", get(persona_chat::transcript))
        .route("/api/persona-chat/{key}/events", get(persona_chat::events))
        .route(
            "/api/persona-chat/{key}/send",
            axum::routing::post(persona_chat::send),
        )
        .route(
            "/api/persona-chat/{key}/cancel",
            axum::routing::post(persona_chat::cancel),
        )
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
        // The chip (§14 step 5). The owner's only: every route here is behind
        // `owner_guard`, and no tool reaches these.
        .route("/api/model", get(model::state))
        .route("/api/model/use", axum::routing::post(model::switch))
        .route("/api/model/cancel", axum::routing::post(model::cancel))
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
        .route("/api/mail/calendars", get(mail::calendars))
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
        .route("/api/library", get(library::list))
        .route("/api/library/portrait/{blob}", get(library::portrait))
        .route("/api/library/unlock", axum::routing::post(library::unlock))
        .route("/api/library/relock", axum::routing::post(library::relock))
        .route("/api/library/source", get(library::source))
        .route(
            "/api/library/save",
            axum::routing::post(library::save), // A portrait's bytes never cross this body — the handler
                                                // reads them from the chat's jail — so the default is ample.
        )
        .route(
            "/api/library/add",
            axum::routing::post(library::add)
                // A portrait rides this body, base64: the store's cap, not
                // axum's 2 MB default, is the ceiling that should answer.
                .layer(axum::extract::DefaultBodyLimit::max(
                    library::MAX_WRITE_BODY,
                )),
        )
        .route(
            "/api/library/edit",
            axum::routing::post(library::edit).layer(axum::extract::DefaultBodyLimit::max(
                library::MAX_WRITE_BODY,
            )),
        )
        .route(
            "/api/library/{kind}/{name}/{action}",
            axum::routing::post(library::act),
        )
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
    // Authentication is ambient through the tailnet proxy. A form on an
    // unrelated site can carry it too. Require a non-simple header on every
    // mutation, including bodyless approval and raw-byte upload routes.
    // There is deliberately no CORS middleware granting that preflight.
    if !matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    ) && (request
        .headers()
        .get("x-mecha-request")
        .and_then(|v| v.to_str().ok())
        != Some("1")
        || request
            .headers()
            .get("sec-fetch-site")
            .is_some_and(|v| v != "same-origin"))
    {
        return (
            StatusCode::FORBIDDEN,
            "request verification failed — reload the page and retry\n",
        )
            .into_response();
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

/// A path segment that starts an incognito key: `/` + `incognito::KEY_PREFIX`
/// (a test holds them together).
const INCOGNITO_KEY_SEGMENT: &str = "/incognito-";

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
    // An incognito chat's every response — its transcript, its events, the
    // pictures in it — is kept out of the browser's cache (R6, design §4.4).
    // `/api/` is otherwise left to its handlers, deliberately (below); this
    // is the one exception, keyed on the prefix every incognito key and door
    // carries.
    let is_incognito = {
        let path = request.uri().path();
        // `/api/sessions` too: the list carries an open incognito chat's
        // key, its flag and its live taint (found on review of #321).
        path.starts_with("/api/incognito")
            || path == "/api/sessions"
            || path.contains(INCOGNITO_KEY_SEGMENT)
            // A persona chat's too: a locked persona's words must not sit in
            // the browser's cache after the library relocks (§8.3).
            || path.starts_with("/api/persona")
    };
    let mut response = next.run(request).await;
    if is_api {
        if is_incognito {
            response.headers_mut().insert(
                axum::http::header::CACHE_CONTROL,
                HeaderValue::from_static("no-store"),
            );
        }
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

/// A `reqwest::Error` with its causes, innermost last. `{e:#}` is anyhow's
/// idiom and reqwest ignores the flag: the top line says "error sending
/// request" and the one fact worth logging — `Connection refused` — is two
/// sources down.
fn error_chain(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut cur = e.source();
    while let Some(next) = cur {
        out.push_str(": ");
        out.push_str(&next.to_string());
        cur = next.source();
    }
    out
}

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
        Ok(resp) => {
            // The refusal's own words travel: a 400 "empty audio" is the
            // page's fault and says so, where "stt answered 400" says only
            // that something happened somewhere behind the page.
            let status = resp.status();
            // One bounded, printable line: the STT is a loopback service of
            // our own, but its body is trusted by convention only, and this
            // reaches the journal — the same rule the worker applies to the
            // page's `link` strings.
            let detail: String = resp
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .filter(|c| c.is_ascii_graphic() || *c == ' ')
                .take(200)
                .collect();
            tracing::warn!("dictate: stt answered {status}: {}", detail.trim());
            (
                StatusCode::BAD_GATEWAY,
                format!("stt answered {status}: {}\n", detail.trim()),
            )
                .into_response()
        }
        Err(e) => {
            let why = error_chain(&e);
            tracing::warn!("dictate: stt unreachable: {why}");
            (
                StatusCode::BAD_GATEWAY,
                format!("stt unreachable — is mecha-parakeet up? {why}\n"),
            )
                .into_response()
        }
    }
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
    forward_offer(target, body).await
}

/// The pipe behind `offer_proxy`, apart from the state that switches it off.
///
/// An offer naming an incognito chat is the one exception to "a pipe, not a
/// participant" (`INCOGNITO-DESIGN.md` §6.4). The worker logs a call from the
/// moment it holds the offer unless it keeps no text of it, and a worker
/// that predates the silence would write the chat's key — and then its
/// words — before the facade's gate ever ran. So the runner is asked first
/// (`runner_keeps_no_text`), and without a yes the offer never reaches it;
/// with one, the answer says so (`unlogged`), which the page requires before
/// it lets a word through. Refused without a log line: the refusal would say
/// what kind of chat was called.
async fn forward_offer(target: &str, body: axum::body::Bytes) -> Response {
    let client = reqwest::Client::new();
    // An offer serve cannot read is not one it relays: the worker's parser
    // accepts shapes `serde_json` refuses (`NaN`, deeper nesting), so an
    // unreadable body could name an incognito chat to the worker while
    // naming nothing here (review of #376). The page only ever sends an
    // object.
    let Some(offer) = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .filter(|v| v.is_object())
    else {
        return (StatusCode::BAD_REQUEST, "malformed offer\n").into_response();
    };
    let incognito = offer_names_incognito(&offer);
    // The prefix decides it, and a name carrying it must also be one the
    // worker will accept: it validates to `valid_key`'s rule and would drop a
    // malformed one — and with it the chat binding and the silence — while
    // this side had vouched for the call (review of #376).
    if incognito && !offer_session(&offer).is_some_and(|s| chat::valid_key(&s)) {
        return (StatusCode::BAD_REQUEST, "malformed chat session\n").into_response();
    }
    if incognito && !runner_keeps_no_text(&client, target).await {
        return (StatusCode::CONFLICT, format!("{UNLOGGED_WORKER_WANTED}\n")).into_response();
    }
    let sent = client
        .post(target)
        .header("content-type", "application/json")
        .body(body.to_vec())
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await;
    match sent {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            // Serve logs at `warn` by default, so until this line a call
            // that never connected left no record anywhere: the worker's
            // journal shows only offers that reached it. 2026-09-12 had two
            // calls in the journal and an afternoon of failed attempts.
            if !status.is_success() {
                tracing::warn!("voice offer: runner answered {status}");
            }
            match resp.bytes().await {
                Ok(bytes) => (
                    status,
                    [("content-type", "application/json")],
                    if incognito && status.is_success() {
                        vouched_answer(&bytes)
                    } else {
                        bytes.to_vec()
                    },
                )
                    .into_response(),
                Err(e) => {
                    (StatusCode::BAD_GATEWAY, format!("reading answer: {e}\n")).into_response()
                }
            }
        }
        Err(e) => {
            let why = error_chain(&e);
            tracing::warn!("voice offer: runner unreachable: {why}");
            (
                StatusCode::BAD_GATEWAY,
                format!("voice runner unreachable: {why}\n"),
            )
                .into_response()
        }
    }
}

/// What the page shows when a call into an incognito chat is refused at the
/// door because the voice worker cannot keep it unlogged.
const UNLOGGED_WORKER_WANTED: &str = "this voice worker keeps call logs — restart \
     mecha-voice-worker to talk in an incognito chat";

/// Whether an offer names an incognito chat (`request_data.session`, the
/// passthrough the worker reads), on the prefix as the worker decides it.
fn offer_names_incognito(offer: &serde_json::Value) -> bool {
    offer_session(offer).is_some_and(|s| incognito::is_incognito_key(&s))
}

/// The session an offer names, trimmed as the worker trims it.
fn offer_session(offer: &serde_json::Value) -> Option<String> {
    Some(
        offer
            .get("request_data")?
            .get("session")?
            .as_str()?
            .trim()
            .to_string(),
    )
}

/// Ask the runner beside `target` whether it holds its log silence for an
/// incognito call: `GET /mecha/unlogged`, answered `{"unlogged": true}` by a
/// worker that does. Anything else — a 404 from one that predates it, a
/// timeout, a body that says otherwise — is no.
async fn runner_keeps_no_text(client: &reqwest::Client, target: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(target).and_then(|t| t.join("/mecha/unlogged")) else {
        return false;
    };
    let Ok(resp) = client
        .get(url)
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
    else {
        return false;
    };
    if !resp.status().is_success() {
        return false;
    }
    resp.json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|v| v.get("unlogged")?.as_bool())
        == Some(true)
}

/// The runner's answer with `"unlogged": true` added — the page's condition
/// for letting a word into an incognito call. `setRemoteDescription` ignores
/// the extra member. An answer that is not a JSON object passes unchanged,
/// and without the flag the page refuses the call.
fn vouched_answer(bytes: &[u8]) -> Vec<u8> {
    match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(serde_json::Value::Object(mut answer)) => {
            answer.insert("unlogged".into(), serde_json::Value::Bool(true));
            serde_json::to_vec(&answer).unwrap_or_else(|_| bytes.to_vec())
        }
        _ => bytes.to_vec(),
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

async fn today() -> axum::response::Response {
    use axum::response::IntoResponse;
    match self_cli_json(&["workflow", "today"], false).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            format!("Today could not be read: {e:#}"),
        )
            .into_response(),
    }
}

async fn workflow_action(
    State(state): State<WebState>,
    axum::extract::Path((id, action)): axum::extract::Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    match action.as_str() {
        "verify" | "ack" | "close" | "reopen" => {
            review::verb(&state, &["workflow", &action, &id]).await
        }
        "snooze" => match body["until"].as_str() {
            Some(until) => review::verb(&state, &["workflow", "snooze", &id, until]).await,
            None => (axum::http::StatusCode::BAD_REQUEST, "snooze needs until").into_response(),
        },
        _ => (axum::http::StatusCode::NOT_FOUND, "unknown workflow action").into_response(),
    }
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
                library: library::state_for_tests(
                    std::env::temp_dir()
                        .join(format!("mecha-serve-test-lib-{}", uuid::Uuid::new_v4())),
                ),
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
        for uri in [
            "/api/mail",
            "/api/mail/read?thread=t&account=a",
            "/api/mail/calendars",
        ] {
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
            ("GET", "/api/today"),
            ("POST", "/api/workflows/test/close"),
            ("POST", "/api/workflows/test/snooze"),
            ("POST", "/api/outbox/test/reconcile"),
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
            // The proposal stores, and the higher-stakes half: an accept on
            // `harness` writes an entry into the override layer every future
            // run reads its config through, and one on `entities` applies a
            // merge with no undo. The guard is a whole-router layer, so these
            // are covered today and this pins that they stay so — which is
            // this test's own argument, since a route added later is exactly
            // the one an earlier guard test cannot be covering.
            ("GET", "/api/proposals"),
            ("GET", "/api/proposals/harness"),
            ("GET", "/api/proposals/harness/1"),
            ("POST", "/api/proposals/harness/1/accept"),
            ("POST", "/api/proposals/harness/1/reject"),
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
    async fn the_model_routes_sit_behind_the_owner_guard() {
        // The chip's routes move the model every surface on the machine
        // answers with (§14, D12), so they are the owner's alone: no header
        // is a 403 on all three, and the owner's header without the
        // request-verification header is a 403 on the two that act — the
        // cross-site form the guard exists for. Covered by the whole-router
        // layer today; pinned because a route added later is exactly the
        // one an earlier guard test cannot be covering.
        for (method, uri, owner) in [
            ("GET", "/api/model", false),
            ("POST", "/api/model/use", false),
            ("POST", "/api/model/cancel", false),
            ("POST", "/api/model/use", true),
            ("POST", "/api/model/cancel", true),
        ] {
            let mut req = Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json");
            if owner {
                req = req.header("Tailscale-User-Login", "owner@example.com");
            }
            let response = test_router()
                .oneshot(req.body(Body::from(r#"{"name":"x"}"#)).unwrap())
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "{method} {uri} owner={owner}"
            );
        }
        // …and the guard is guarding *these* routes: a 403 alone would pass
        // on a misspelled URI too (review, pass 3). The owner, verified,
        // reaches the handler — which refuses an empty name before it spawns
        // anything, so no switch is made against the developer's machine.
        let response = test_router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/model/use")
                    .header("Tailscale-User-Login", "owner@example.com")
                    .header("x-mecha-request", "1")
                    .header("sec-fetch-site", "same-origin")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"  "}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "the owner did not reach the use handler"
        );
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
                    .header("x-mecha-request", "1")
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
                        .header("x-mecha-request", "1")
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
                    .header("x-mecha-request", "1")
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
                library: library::state_for_tests(
                    std::env::temp_dir()
                        .join(format!("mecha-serve-test-lib-{}", uuid::Uuid::new_v4())),
                ),
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
                library: library::state_for_tests(
                    std::env::temp_dir()
                        .join(format!("mecha-serve-test-lib-{}", uuid::Uuid::new_v4())),
                ),
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
                    .header("x-mecha-request", "1")
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
                    .header("x-mecha-request", "1")
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
                    .header("x-mecha-request", "1")
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
        // Under tower-http 0.7 a `304` names what it confirmed whichever
        // precondition header asked — by date here, by tag in the test below
        // — so the browser can refresh both validators on the entry it holds
        // (RFC 9110 §15.4.5). Under 0.6 this `304` was bare.
        assert!(
            again.headers().get("etag").is_some(),
            "a 304 to If-Modified-Since carries the ETag validator"
        );
        assert!(
            again.headers().get("last-modified").is_some(),
            "a 304 to If-Modified-Since carries the Last-Modified validator"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The entry document revalidates by `ETag` as well as by date, and the
    /// `304` names what it confirmed.
    ///
    /// This is what tower-http 0.7 added to `ServeDir` (strong `ETag`s from
    /// size and mtime, `If-None-Match` per RFC 9110 §13.1.2) and what the
    /// `no-cache` half of `cache_headers` now rides on: a browser holding the
    /// document sends the tag back, and a `304` that carries no validators
    /// would leave it unable to update the entry it just confirmed (RFC 9110
    /// §15.4.5). Under 0.6 the response had no `ETag` at all and the `304`
    /// was bare — this test fails there at the first `expect`.
    #[tokio::test]
    async fn a_revalidation_by_etag_answers_304_with_both_validators() {
        let dir = std::env::temp_dir().join(format!("mecha-cache-etag-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let router = test_router_with_assets(&dir);

        let first = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("Tailscale-User-Login", "owner@example.com")
                    .header("x-mecha-request", "1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        let etag = first
            .headers()
            .get("etag")
            .expect("ServeDir tags what it serves")
            .clone();
        assert!(
            !etag.as_bytes().starts_with(b"W/"),
            "a weak tag would not satisfy If-Match or a byte-range resume"
        );
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
                    .header("x-mecha-request", "1")
                    .header("If-None-Match", etag.clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            again.status(),
            StatusCode::NOT_MODIFIED,
            "a matching tag must cost no bytes"
        );
        assert_eq!(
            again.headers().get("etag"),
            Some(&etag),
            "the 304 must name the tag it confirmed"
        );
        assert_eq!(
            again.headers().get("last-modified"),
            Some(&last_modified),
            "the 304 must carry the date it confirmed"
        );
        assert_eq!(
            again.headers().get("cache-control").unwrap(),
            "no-cache",
            "our own header still rides on the tag-validated 304"
        );

        // The other half of the guarantee, and the one that protects against
        // the 2026-08-29 incident: a *stale* tag must get the new document,
        // not a 304 with false confidence. The tag is `"<secs>.<nanos>-<size>"`,
        // so a rewrite of a different length changes it whatever the clock
        // granularity — no sleep, no flake. (`test_router_with_assets` writes
        // the fixture, so the rewrite comes after it.)
        let router = test_router_with_assets(&dir);
        std::fs::write(
            dir.join("index.html"),
            "<!doctype html><script src=/assets/index-def456.js></script><!-- changed -->",
        )
        .unwrap();
        let changed = router
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("Tailscale-User-Login", "owner@example.com")
                    .header("x-mecha-request", "1")
                    .header("If-None-Match", etag.clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            changed.status(),
            StatusCode::OK,
            "a stale tag must fetch the changed document, not confirm the old one"
        );
        let new_tag = changed
            .headers()
            .get("etag")
            .expect("the changed document is tagged too")
            .clone();
        assert_ne!(new_tag, etag, "a changed document carries a different tag");
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
                    .header("x-mecha-request", "1")
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

    /// The list editor's save: rows set in place in the file on disk, so a
    /// comment among the lines survives a re-rank and an edit, which the JS
    /// serialiser it replaced could not do — and a page that read a charter
    /// changed since is refused, not written over.
    #[tokio::test]
    async fn a_list_save_edits_the_charter_in_place_and_refuses_a_stale_page() {
        let home = crate::testenv::HomeGuard::new("serve-charter-rows");
        let on_disk = home.dir.join("charter.toml");
        let start = "# Mine.\n\n[[line]]\n# the one that matters\nid = \"first\"\ntext = \"tell the truth early\"\n\n[[line]]\nid = \"second\"\ntext = \"rest\"\n";
        std::fs::write(&on_disk, start).unwrap();
        let post = |body: serde_json::Value| {
            test_router().oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/settings/charter")
                    .header("Tailscale-User-Login", "owner@example.com")
                    .header("x-mecha-request", "1")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
        };
        let base = mecha_core::charter::digest(start);
        let rows = serde_json::json!([
            {"id": "second", "text": "rest well", "sensor": null},
            {"id": "first", "text": "tell the truth early", "sensor": null},
        ]);
        let ok = post(serde_json::json!({ "changes": { "line": rows }, "base": base }))
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        let now = std::fs::read_to_string(&on_disk).unwrap();
        assert!(now.starts_with("# Mine."), "{now}");
        assert!(now.contains("# the one that matters"), "{now}");
        let parsed = mecha_core::charter::Charter::parse(&now).unwrap();
        let ids: Vec<&str> = parsed.lines().iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["second", "first"], "{now}");

        // The same base again is stale now: refused, the file as it was.
        let stale = post(serde_json::json!({ "changes": { "line": [] }, "base": base }))
            .await
            .unwrap();
        assert_eq!(stale.status(), StatusCode::CONFLICT);
        assert_eq!(std::fs::read_to_string(&on_disk).unwrap(), now);

        // A row the form does not offer, and a result the charter refuses,
        // are both 422 and write nothing.
        let fresh = mecha_core::charter::digest(&now);
        for bad in [
            serde_json::json!([{"id": "x", "text": "y", "rank": 1}]),
            serde_json::json!([{"id": "has space", "text": "y", "sensor": null}]),
        ] {
            let refused = post(serde_json::json!({ "changes": { "line": bad }, "base": fresh }))
                .await
                .unwrap();
            assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
            assert_eq!(std::fs::read_to_string(&on_disk).unwrap(), now);
        }
        // Every line deleted: the save lands, the header and the comments
        // stay, and what is on disk is a charter with no lines (review of
        // #439: this path reached disk untested end to end).
        let emptied = post(serde_json::json!({ "changes": { "line": [] }, "base": fresh }))
            .await
            .unwrap();
        assert_eq!(emptied.status(), StatusCode::OK);
        let bare = std::fs::read_to_string(&on_disk).unwrap();
        assert!(bare.starts_with("# Mine."), "{bare}");
        assert!(bare.contains("# the one that matters"), "{bare}");
        assert!(!bare.contains("[[line]]"), "{bare}");
        assert!(mecha_core::charter::Charter::parse(&bare)
            .unwrap()
            .lines()
            .is_empty());

        // Neither, or both, is a malformed request.
        let neither = post(serde_json::json!({})).await.unwrap();
        assert_eq!(neither.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_clone_without_the_wav_content_type_is_refused_before_the_write() {
        // The owner guard already checks request intent. The handler also
        // requires the declared audio format: text/plain must fail with 415
        // and write nothing, even when the bytes happen to be a valid WAV.
        let dir = std::env::temp_dir().join(format!("mecha-clone-ct-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let response = test_router_with_voices(&dir)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/settings/voice/clone?name=x")
                    .header("Tailscale-User-Login", "owner@example.com")
                    .header("x-mecha-request", "1")
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
                    .header("x-mecha-request", "1")
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
                    .header("x-mecha-request", "1")
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
                    .header("x-mecha-request", "1")
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

#[cfg(test)]
mod boundary_tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use tower::util::ServiceExt;

    fn app(chat: Arc<chat::ChatState>) -> Router {
        router(
            WebState {
                owner_login: Arc::new("owner@example.com".into()),
                chat: Some(chat),
                review: Arc::new(review::ReviewState {
                    outbox_root: PathBuf::new(),
                    sessions_dir: None,
                }),
                offer_target: None,
                voices_dir: None,
                library: library::state_for_tests(
                    std::env::temp_dir()
                        .join(format!("mecha-serve-test-lib-{}", uuid::Uuid::new_v4())),
                ),
            },
            None,
        )
    }
    fn post(uri: &str, body: &'static str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(TAILSCALE_LOGIN, "owner@example.com")
            .header("x-mecha-request", "1")
            .body(Body::from(body))
            .unwrap()
    }
    async fn body(response: Response) -> serde_json::Value {
        serde_json::from_slice(&to_bytes(response.into_body(), 100_000).await.unwrap()).unwrap()
    }

    /// Every file under `dir` whose bytes contain `needle`.
    fn files_containing(dir: &std::path::Path, needle: &str) -> Vec<PathBuf> {
        let mut hits = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            let Ok(read) = std::fs::read_dir(&d) else {
                continue;
            };
            for entry in read.flatten() {
                let path = entry.path();
                let Ok(meta) = std::fs::symlink_metadata(&path) else {
                    continue;
                };
                if meta.is_dir() {
                    stack.push(path);
                } else if meta.is_file()
                    && (path
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().contains(needle))
                        || std::fs::read(&path)
                            .is_ok_and(|b| b.windows(needle.len()).any(|w| w == needle.as_bytes())))
                {
                    // A name is a copy too.
                    hits.push(path);
                }
            }
        }
        hits
    }

    /// Point `XDG_RUNTIME_DIR` at a fresh directory on `/dev/shm` for the
    /// guard's lifetime; `None` where `/dev/shm` is not tmpfs, and the caller
    /// skips. Under the `HomeGuard` lock, which serialises every test that
    /// moves process environment.
    struct RuntimeDir(PathBuf, Option<std::ffi::OsString>);
    impl RuntimeDir {
        fn new() -> Option<Self> {
            let shm = PathBuf::from("/dev/shm");
            let dir = shm.join(format!("mecha-incognito-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&dir).ok()?;
            let previous = std::env::var_os("XDG_RUNTIME_DIR");
            std::env::set_var("XDG_RUNTIME_DIR", &dir);
            if let Err(e) = super::incognito::rooms_root() {
                match &previous {
                    Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
                    None => std::env::remove_var("XDG_RUNTIME_DIR"),
                }
                let _ = std::fs::remove_dir_all(&dir);
                // In CI a skipped test reads exactly like a passing one.
                assert!(
                    std::env::var_os("MECHA_TEST_REQUIRE_BACKENDS").is_none(),
                    "MECHA_TEST_REQUIRE_BACKENDS is set and no RAM-backed room can be made: {e:#}"
                );
                eprintln!("skipped: no RAM-backed room here ({e:#})");
                return None;
            }
            Some(RuntimeDir(dir, previous))
        }
    }
    impl Drop for RuntimeDir {
        fn drop(&mut self) {
            match &self.1 {
                Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
                None => std::env::remove_var("XDG_RUNTIME_DIR"),
            }
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn json_post(uri: &str, json: String) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(TAILSCALE_LOGIN, "owner@example.com")
            .header("x-mecha-request", "1")
            .header("content-type", "application/json")
            .body(Body::from(json))
            .unwrap()
    }

    fn get(uri: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .header(TAILSCALE_LOGIN, "owner@example.com")
            .body(Body::empty())
            .unwrap()
    }

    /// Send `text` into `key` and wait for the answer to land.
    async fn converse(app: &Router, key: &str, text: &str) {
        let sent = app
            .clone()
            .oneshot(json_post(
                &format!("/api/chat/{key}/send"),
                serde_json::json!({ "text": text }).to_string(),
            ))
            .await
            .unwrap();
        assert!(sent.status().is_success(), "{}", sent.status());
        for _ in 0..200 {
            let t = app
                .clone()
                .oneshot(get(&format!("/api/chat/{key}")))
                .await
                .unwrap();
            let v = body(t).await;
            if v["running"] == false && v["held_by_run"] == false {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("the turn in {key} never finished");
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbImage::from_pixel(w, h, image::Rgb([10, 200, 90]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    /// Upload each of `files`, send `text` naming them the way the page
    /// does, and return the last user message the model was sent.
    async fn send_with_attachments(
        vision: bool,
        files: &[(&str, Vec<u8>)],
        extra: &[&str],
    ) -> (mecha_core::message::Message, serde_json::Value) {
        let (chat, seen) = chat::test_chat_seeing(vision);
        let app = app(chat);
        let mut paths = Vec::new();
        for (name, bytes) in files {
            let up = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!("/api/chat/pix/upload?name={name}"))
                        .header(TAILSCALE_LOGIN, "owner@example.com")
                        .header("x-mecha-request", "1")
                        .body(Body::from(bytes.clone()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(up.status(), StatusCode::OK);
            paths.push(body(up).await["path"].as_str().unwrap().to_string());
        }
        paths.extend(extra.iter().map(|p| p.to_string()));
        let text = paths
            .iter()
            .map(|p| format!("Attached file at {p}"))
            .collect::<Vec<_>>()
            .join("\n");
        let sent = app
            .clone()
            .oneshot(json_post(
                "/api/chat/pix/send",
                serde_json::json!({ "text": text, "attachments": paths }).to_string(),
            ))
            .await
            .unwrap();
        assert!(sent.status().is_success(), "{}", sent.status());
        let reply = body(sent).await;
        for _ in 0..200 {
            if let Some(req) = seen.lock().unwrap().first() {
                return (req.messages.last().unwrap().clone(), reply);
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("the model was never asked");
    }

    fn images(m: &mecha_core::message::Message) -> Vec<Option<String>> {
        m.content
            .iter()
            .filter_map(|b| match b {
                mecha_core::message::Block::Image { source, .. } => Some(source.clone()),
                _ => None,
            })
            .collect()
    }

    /// D6's second half: a picture uploaded from the page rides on the turn
    /// as pixels beside the path the text names; a non-image rides as its
    /// path alone. Fails on the old door, which put only the text on the turn.
    #[tokio::test]
    async fn an_uploaded_picture_rides_on_the_turn_for_a_model_that_can_see() {
        let _home = crate::testenv::HomeGuard::new("web-attach-pixels");
        let (user, reply) = send_with_attachments(
            true,
            &[("shot.png", png(40, 20)), ("notes.txt", b"hello".to_vec())],
            &[],
        )
        .await;
        assert_eq!(images(&user), vec![Some("inbox/shot.png".to_string())]);
        assert_eq!(reply["pictures_not_shown"], 0, "{reply}");
        assert!(user.text().contains("Attached file at inbox/shot.png"));
        assert!(user.text().contains("Attached file at inbox/notes.txt"));
    }

    /// A blind model gets the paths and nothing else, and a path the page
    /// names outside the jail attaches nothing and fails nothing.
    #[tokio::test]
    async fn a_blind_model_or_an_escaping_path_gets_no_pixels() {
        let home = crate::testenv::HomeGuard::new("web-attach-blind");
        let (user, reply) = send_with_attachments(false, &[("shot.png", png(8, 8))], &[]).await;
        assert!(images(&user).is_empty(), "{:?}", user.content);
        // Said to the page, which cleared the chip on send (found on review
        // of #366): one picture named, none shown, and why.
        assert_eq!(reply["pictures_not_shown"], 1, "{reply}");
        assert_eq!(reply["model_sees"], false, "{reply}");

        // One real upload beside the escaping paths, so the session and its
        // workspace exist: without it `send` has no workspace to read and
        // the escapes are never resolved at all — passing for the wrong
        // reason (found on review of #366).
        let outside = home.dir.join("outside.png");
        std::fs::write(&outside, png(8, 8)).unwrap();
        let (user, reply) = send_with_attachments(
            true,
            &[("inside.png", png(8, 8))],
            &["../../../outside.png", outside.to_str().unwrap()],
        )
        .await;
        assert_eq!(images(&user), vec![Some("inbox/inside.png".to_string())]);
        assert_eq!(reply["pictures_not_shown"], 2, "{reply}");
    }

    /// An upload survives the chat leaving the map before the send — a
    /// `serve` restart, a handover — and still rides as pixels, read from the
    /// directory the send re-creates the session in. Fails on the first-look
    /// lookup alone, which found no session and dropped them silently.
    #[tokio::test]
    async fn an_upload_still_rides_after_its_chat_left_the_map() {
        let _home = crate::testenv::HomeGuard::new("web-attach-restart");
        let (chat, seen) = chat::test_chat_seeing(true);
        let app = app(Arc::clone(&chat));
        let up = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/chat/pix/upload?name=shot.png")
                    .header(TAILSCALE_LOGIN, "owner@example.com")
                    .header("x-mecha-request", "1")
                    .body(Body::from(png(8, 8)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(up.status(), StatusCode::OK);
        chat::test_forget_session(&chat, "pix").await;
        let sent = app
            .clone()
            .oneshot(json_post(
                "/api/chat/pix/send",
                serde_json::json!({
                    "text": "Attached file at inbox/shot.png",
                    "attachments": ["inbox/shot.png"],
                })
                .to_string(),
            ))
            .await
            .unwrap();
        assert!(sent.status().is_success(), "{}", sent.status());
        for _ in 0..200 {
            if let Some(req) = seen.lock().unwrap().first() {
                let user = req.messages.last().unwrap();
                assert_eq!(images(user), vec![Some("inbox/shot.png".to_string())]);
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("the model was never asked");
    }

    #[tokio::test]
    async fn an_incognito_chat_leaves_no_trace_and_an_ordinary_one_does() {
        let home = crate::testenv::HomeGuard::new("incognito-trace");
        let Some(runtime) = RuntimeDir::new() else {
            return;
        };
        const CANARY: &str = "KUMQUAT-7731";
        let app = app(chat::test_chat_answering("noted", true));

        // Open through its own door.
        let opened = app
            .clone()
            .oneshot(post("/api/incognito", ""))
            .await
            .unwrap();
        assert_eq!(opened.status(), StatusCode::OK);
        assert_eq!(
            opened
                .headers()
                .get(axum::http::header::CACHE_CONTROL)
                .map(|v| v.to_str().unwrap()),
            Some("no-store")
        );
        let key = body(opened).await["key"].as_str().unwrap().to_string();
        assert!(key.starts_with(incognito::KEY_PREFIX), "{key}");

        // A turn and an upload, both carrying the canary.
        converse(&app, &key, &format!("remember {CANARY} for me")).await;
        let uploaded = app
            .clone()
            .oneshot(json_post(
                &format!("/api/chat/{key}/upload?name={CANARY}.txt"),
                CANARY.to_string(),
            ))
            .await
            .unwrap();
        assert!(uploaded.status().is_success(), "{}", uploaded.status());
        let read = app
            .clone()
            .oneshot(get(&format!("/api/chat/{key}")))
            .await
            .unwrap();
        assert_eq!(
            read.headers()
                .get(axum::http::header::CACHE_CONTROL)
                .map(|v| v.to_str().unwrap()),
            Some("no-store")
        );
        let v = body(read).await;
        assert_eq!(v["incognito"], true);
        assert!(
            v.to_string().contains(CANARY),
            "the conversation is live in memory"
        );

        // While open: nothing in the mecha home, everything in the room.
        assert!(
            files_containing(&home.dir, CANARY).is_empty(),
            "{:?}",
            files_containing(&home.dir, CANARY)
        );
        assert!(
            !files_containing(&runtime.0, CANARY).is_empty(),
            "the upload is in the room"
        );

        // End: the room goes, and the key is dead.
        let ended = app
            .clone()
            .oneshot(post(&format!("/api/incognito/{key}/end"), ""))
            .await
            .unwrap();
        assert_eq!(ended.status(), StatusCode::NO_CONTENT);
        assert!(
            files_containing(&runtime.0, CANARY).is_empty(),
            "the room is gone"
        );
        assert!(files_containing(&home.dir, CANARY).is_empty());
        let reopened = app
            .clone()
            .oneshot(json_post(
                &format!("/api/chat/{key}/send"),
                serde_json::json!({ "text": "still there?" }).to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(
            reopened.status(),
            StatusCode::GONE,
            "a closed key does not reopen"
        );
        let revived = app
            .clone()
            .oneshot(post(&format!("/api/chat/{key}"), ""))
            .await
            .unwrap();
        assert_eq!(
            revived.status(),
            StatusCode::GONE,
            "nor through the ordinary door"
        );

        // The vacuity check: the same turn in an ordinary chat is recorded,
        // so the scan above was looking where a trace would be.
        converse(&app, "main", &format!("remember {CANARY} for me")).await;
        assert!(
            !files_containing(&home.dir, CANARY).is_empty(),
            "an ordinary chat's transcript carries the canary"
        );
    }

    #[tokio::test]
    async fn an_idle_incognito_chat_closes_itself_and_a_recent_one_does_not() {
        let _home = crate::testenv::HomeGuard::new("incognito-idle");
        let Some(runtime) = RuntimeDir::new() else {
            return;
        };
        let chat = chat::test_chat_answering("noted", true);
        let stale = chat.open_incognito().await.unwrap();
        let fresh = chat.open_incognito().await.unwrap();
        let stale_room = chat.room_of(&stale).await.unwrap();
        stale_room.backdate(incognito::IDLE + std::time::Duration::from_secs(1));
        assert_eq!(chat.reap_idle_incognito().await, 1);
        assert!(!stale_room.root.exists(), "the idle room is gone");
        assert!(chat.room_of(&stale).await.is_none());
        assert!(
            chat.room_of(&fresh).await.is_some(),
            "a recent chat stays open"
        );
        assert!(chat.close_incognito(&fresh).await);
        drop(runtime);
    }

    /// A stand-in runner: `/api/offer` counts what reaches it and answers an
    /// SDP-shaped object; `/mecha/unlogged` exists only when `vouches`.
    async fn stub_runner(vouches: bool) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let offers = Arc::new(AtomicUsize::new(0));
        let seen = offers.clone();
        let mut app = Router::new().route(
            "/api/offer",
            axum::routing::post(move || {
                let seen = seen.clone();
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    Json(serde_json::json!({"sdp": "v=0", "type": "answer", "pc_id": "pc-1"}))
                }
            }),
        );
        if vouches {
            app = app.route(
                "/mecha/unlogged",
                axum::routing::get(|| async { Json(serde_json::json!({"unlogged": true})) }),
            );
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        (format!("http://{addr}/api/offer"), offers)
    }

    async fn answer_of(resp: Response) -> (StatusCode, Vec<u8>) {
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, body.to_vec())
    }

    /// An incognito offer reaches the runner only if it keeps no text of the
    /// call, and the page learns so from the answer; an ordinary one is the
    /// pipe it always was (review of #376: the facade's gate came after the
    /// worker had logged).
    #[tokio::test]
    async fn an_incognito_offer_reaches_only_a_runner_that_keeps_no_text() {
        use std::sync::atomic::Ordering;
        let incognito = axum::body::Bytes::from(
            r#"{"sdp":"x","type":"offer","request_data":{"session":"incognito-0123456789abcdef012345"}}"#,
        );
        let ordinary = axum::body::Bytes::from(
            r#"{"sdp":"x","type":"offer","request_data":{"session":"main"}}"#,
        );

        let (old, offers) = stub_runner(false).await;
        let (status, body) = answer_of(forward_offer(&old, incognito.clone()).await).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(String::from_utf8_lossy(&body).contains("restart mecha-voice-worker"));
        assert_eq!(
            offers.load(Ordering::SeqCst),
            0,
            "an old worker was handed the offer"
        );
        let (status, body) = answer_of(forward_offer(&old, ordinary.clone()).await).await;
        assert_eq!(status, StatusCode::OK, "an ordinary call needs no vouch");
        let answer: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            answer.get("unlogged").is_none(),
            "an ordinary answer was vouched for"
        );

        let (new, offers) = stub_runner(true).await;
        // A name with the prefix that the worker would refuse is refused
        // here, vouch or no: the worker would drop it, and the silence with it.
        for bad in [
            r#"{"request_data":{"session":"incognito-0123456789abcdef0123456789"}}"#,
            r#"{"request_data":{"session":"incognito-ABC"}}"#,
        ] {
            let (status, _) = answer_of(forward_offer(&new, bad.into()).await).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        }
        assert_eq!(
            offers.load(Ordering::SeqCst),
            0,
            "a malformed name was forwarded"
        );
        let (status, body) = answer_of(forward_offer(&new, incognito).await).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(offers.load(Ordering::SeqCst), 1);
        let answer: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(answer["unlogged"], serde_json::Value::Bool(true));
        assert_eq!(answer["sdp"], "v=0", "the answer itself passes through");
    }

    #[test]
    fn only_an_offer_naming_an_incognito_chat_is_one() {
        let named = |body: &str| offer_names_incognito(&serde_json::from_str(body).unwrap());
        assert!(named(
            r#"{"request_data":{"session":"incognito-0123456789abcdef012345"}}"#
        ));
        for body in [
            r#"{"request_data":{"session":"main"}}"#,
            r#"{"request_data":{"uplink":"channel"}}"#,
            r#"{"sdp":"x"}"#,
        ] {
            assert!(!named(body));
        }
    }

    /// An offer serve cannot read never reaches the runner: the worker's
    /// parser takes shapes this one refuses, and could find a chat in it.
    #[tokio::test]
    async fn an_offer_serve_cannot_read_is_not_relayed() {
        use std::sync::atomic::Ordering;
        let (target, offers) = stub_runner(false).await;
        for body in [
            "not json",
            r#"{"request_data":{"session":"incognito-0123456789abcdef012345"},"x":NaN}"#,
            "[1, 2]",
        ] {
            let (status, _) = answer_of(forward_offer(&target, body.into()).await).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        }
        assert_eq!(offers.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_no_store_needle_is_the_key_prefix() {
        assert_eq!(INCOGNITO_KEY_SEGMENT, format!("/{}", incognito::KEY_PREFIX));
    }

    /// The prefix has two copies outside Rust, and each decides something:
    /// the page's whether a switch hangs up a call, the worker's whether a
    /// call holds the log silence (review of #376). Pinned to the one here.
    #[test]
    fn every_copy_of_the_incognito_prefix_is_the_servers() {
        let page = include_str!("../../../../web/src/lib/Chat.svelte");
        let worker = include_str!("../../../../scripts/voice/worker.py");
        let want = incognito::KEY_PREFIX;
        assert!(
            page.contains(&format!("const INCOGNITO_PREFIX = '{want}';")),
            "Chat.svelte's INCOGNITO_PREFIX is not {want:?}"
        );
        assert!(
            worker.contains(&format!("\nINCOGNITO_PREFIX = \"{want}\"\n")),
            "worker.py's INCOGNITO_PREFIX is not {want:?}"
        );
    }

    #[tokio::test]
    async fn the_session_list_is_not_cached() {
        let _home = crate::testenv::HomeGuard::new("incognito-list");
        let app = app(chat::test_chat());
        let list = app.clone().oneshot(get("/api/sessions")).await.unwrap();
        assert_eq!(
            list.headers()
                .get(axum::http::header::CACHE_CONTROL)
                .map(|v| v.to_str().unwrap()),
            Some("no-store")
        );
    }

    #[tokio::test]
    async fn stopping_the_server_closes_every_incognito_chat() {
        let _home = crate::testenv::HomeGuard::new("incognito-stop");
        let Some(_runtime) = RuntimeDir::new() else {
            return;
        };
        let chat = chat::test_chat_answering("noted", true);
        let key = chat.open_incognito().await.unwrap();
        let room = chat.room_of(&key).await.unwrap();
        chat.stop().await;
        assert!(!room.root.exists(), "the room is gone at shutdown");
        // Out of the map as well, so a run still finishing sees it closed
        // and removes whatever it spilled on the way out.
        assert!(chat.room_of(&key).await.is_none());
    }

    #[tokio::test]
    async fn a_run_in_flight_at_shutdown_keeps_its_room_until_it_is_done() {
        let _home = crate::testenv::HomeGuard::new("incognito-stop-live");
        let Some(_runtime) = RuntimeDir::new() else {
            return;
        };
        let go = Arc::new(tokio::sync::Notify::new());
        let chat = chat::test_chat_waiting(go.clone());
        let app = app(chat.clone());
        let key = chat.open_incognito().await.unwrap();
        let room = chat.room_of(&key).await.unwrap();
        let sent = app
            .clone()
            .oneshot(json_post(
                &format!("/api/chat/{key}/send"),
                serde_json::json!({ "text": "hello" }).to_string(),
            ))
            .await
            .unwrap();
        assert!(sent.status().is_success(), "{}", sent.status());
        chat.stop().await;
        // Still there: a forced drain from here on must leave the room — and
        // its image trail — for the next start's sweep.
        assert!(
            room.root.exists(),
            "the room of a run in flight went at stop"
        );
        // The run finishes; on its way out it finds its chat closed and
        // removes the room itself.
        go.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(5), chat.drain())
            .await
            .expect("the run never finished");
        assert!(!room.root.exists(), "the run left its room behind");
    }

    /// A page that loads while a run holds the conversation — or a phone
    /// whose stream reconnected mid-run — replaces its transcript with this
    /// read. It used to be empty, and the whole history vanished from the
    /// page for as long as the run lasted (an image edit, on a phone: until
    /// a manual reload after it finished).
    #[tokio::test]
    async fn a_transcript_read_mid_run_returns_the_history_the_run_started_from() {
        let _home = crate::testenv::HomeGuard::new("mid-run-history");
        let go = Arc::new(tokio::sync::Notify::new());
        let chat = chat::test_chat_waiting(go.clone());
        let app = app(chat.clone());
        let key = "midrun";

        // One finished turn: the provider answers once it is let go.
        go.notify_one();
        converse(&app, key, "first question").await;

        // A second turn, held in flight.
        let sent = app
            .clone()
            .oneshot(json_post(
                &format!("/api/chat/{key}/send"),
                serde_json::json!({ "text": "second question" }).to_string(),
            ))
            .await
            .unwrap();
        assert!(sent.status().is_success(), "{}", sent.status());
        let v = body(
            app.clone()
                .oneshot(get(&format!("/api/chat/{key}")))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(
            v["held_by_run"], true,
            "the second run is not in flight: {v}"
        );
        let texts: Vec<&str> = v["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["text"].as_str().unwrap_or(""))
            .collect();
        assert_eq!(
            texts,
            ["first question", "late", "second question"],
            "a mid-run read must carry the history up to this run's own input"
        );
        // With its taint: history without the chip reads as clean.
        assert!(
            v["taint"].is_object(),
            "a mid-run read dropped the taint: {v}"
        );

        // Released until drained: the first turn's title generation waits on
        // the same provider, and either may take a single permit.
        let release = tokio::spawn(async move {
            loop {
                go.notify_one();
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        });
        let mut settled = None;
        for _ in 0..200 {
            let v = body(
                app.clone()
                    .oneshot(get(&format!("/api/chat/{key}")))
                    .await
                    .unwrap(),
            )
            .await;
            if v["running"] == false && v["held_by_run"] == false {
                settled = Some(v);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        release.abort();
        // And once it is over, the read is the whole conversation again —
        // the one the page's re-read at `done` relies on.
        let v = settled.expect("the second run never finished");
        assert_eq!(v["entries"].as_array().unwrap().len(), 4, "{v}");
    }

    /// The chip beside a mid-run history answers for that history: an image
    /// in it arms `private` even though the loop has not yet armed the
    /// conversation for it (it does that inside the run, after `Live` was
    /// built), or the page would draw `[image]` under a clean chip.
    #[tokio::test]
    async fn a_mid_run_read_is_tainted_by_the_history_it_carries() {
        let _home = crate::testenv::HomeGuard::new("mid-run-taint");
        let go = Arc::new(tokio::sync::Notify::new());
        let chat = chat::test_chat_waiting(go.clone());
        let app = app(chat.clone());
        let key = "midrun-taint";

        go.notify_one();
        converse(&app, key, "first question").await;
        chat::test_plant_image(&chat, key).await;

        let sent = app
            .clone()
            .oneshot(json_post(
                &format!("/api/chat/{key}/send"),
                serde_json::json!({ "text": "what is in it?" }).to_string(),
            ))
            .await
            .unwrap();
        assert!(sent.status().is_success(), "{}", sent.status());
        let v = body(
            app.clone()
                .oneshot(get(&format!("/api/chat/{key}")))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(v["held_by_run"], true, "the run is not in flight: {v}");
        assert!(
            v["entries"].to_string().contains("[image]"),
            "the history does not carry the image: {v}"
        );
        assert_eq!(
            v["taint"]["private"], true,
            "an image under a clean chip: {v}"
        );

        let release = tokio::spawn(async move {
            loop {
                go.notify_one();
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        });
        for _ in 0..200 {
            let v = body(
                app.clone()
                    .oneshot(get(&format!("/api/chat/{key}")))
                    .await
                    .unwrap(),
            )
            .await;
            if v["running"] == false && v["held_by_run"] == false {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        release.abort();
    }

    /// The page re-reads the plan on every `todo` result a run streams; this
    /// read is the plan and nothing else, so a mid-run revision does not pay
    /// for the history the transcript read now carries.
    #[tokio::test]
    async fn the_plan_read_carries_the_plan_and_not_the_history() {
        let _home = crate::testenv::HomeGuard::new("plan-read");
        let todo = Arc::new(mecha_core::tool::todo::TodoTool::new());
        let chat = chat::test_chat_planned("noted", todo.clone());
        let app = app(chat.clone());
        converse(&app, "planned", "first question").await;
        // One step, in this session's jail and nowhere else: a read that
        // looked the plan up by any other key would come back empty.
        todo.set_plan_in(
            &chat::test_workspace(&chat, "planned").await,
            mecha_core::tool::todo::Plan {
                goal: None,
                items: vec![mecha_core::tool::todo::TodoItem::new(
                    "draft the reply",
                    mecha_core::tool::todo::Status::InProgress,
                )],
            },
        );

        let plan = body(
            app.clone()
                .oneshot(get("/api/chat/planned/todo"))
                .await
                .unwrap(),
        )
        .await;
        let whole = body(app.clone().oneshot(get("/api/chat/planned")).await.unwrap()).await;
        assert_eq!(
            plan["todo"], whole["todo"],
            "the two reads disagree on the plan"
        );
        assert_eq!(
            plan["todo"][0]["content"], "draft the reply",
            "the plan read did not find the session's plan: {plan}"
        );
        assert_eq!(
            plan.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["todo"],
            "the plan read carries more than the plan: {plan}"
        );

        let missing = app
            .clone()
            .oneshot(get("/api/chat/nobody-here/todo"))
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        // An incognito key not open has closed: gone, like the transcript
        // read, and never a 404 that reads as a chat that never existed.
        let closed = app
            .clone()
            .oneshot(get(&format!(
                "/api/chat/{}0123456789abcdef012345/todo",
                super::incognito::KEY_PREFIX
            )))
            .await
            .unwrap();
        assert_eq!(closed.status(), StatusCode::GONE);
    }

    #[tokio::test]
    async fn incognito_refuses_a_model_that_is_not_on_this_machine() {
        let _home = crate::testenv::HomeGuard::new("incognito-cloud");
        let Some(_runtime) = RuntimeDir::new() else {
            return;
        };
        let app = app(chat::test_chat_answering("noted", false));
        let opened = app
            .clone()
            .oneshot(post("/api/incognito", ""))
            .await
            .unwrap();
        assert_eq!(opened.status(), StatusCode::CONFLICT);
        let why = String::from_utf8(to_bytes(opened.into_body(), 10_000).await.unwrap().to_vec())
            .unwrap();
        assert!(why.contains("local model"), "{why}");
    }

    /// A conversation that crosses a switch says which model answered each
    /// run (`follow::Bound::generation`): the corpus pairs an outcome with the
    /// config record before it, so without the second record every turn after
    /// a switch would be credited to the model the chat opened on.
    #[tokio::test]
    async fn a_conversation_that_crosses_a_switch_records_the_model_of_each_turn() {
        let _home = crate::testenv::HomeGuard::new("follow-crossing");
        let chat = chat::test_chat_answering("noted", true);
        let app = app(Arc::clone(&chat));
        converse(&app, "crossing", "first").await;
        chat::test_switch(&chat, "noted", "test-b", true);
        converse(&app, "crossing", "second").await;
        converse(&app, "crossing", "third").await;

        let dir = mecha_core::session::Session::default_dir().unwrap();
        let recorded: Vec<Vec<String>> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
            .map(|e| {
                mecha_core::session::Session::run_configs(&e.path())
                    .unwrap()
                    .into_iter()
                    .map(|c| c.model)
                    .collect()
            })
            .collect();
        assert_eq!(
            recorded,
            vec![vec!["test".to_string(), "test-b".to_string()]],
            "one record on open, one at the switch — and none for a turn on the same binding"
        );
    }

    /// A chat opened after an outside switch is headed with the model that
    /// will answer it: the page opens a chat (`POST /api/chat/{key}`) before
    /// its first turn follows, and that path headed the session — and wrote
    /// its first record — from `current()`, the model from before the switch
    /// (review of #347).
    #[tokio::test]
    async fn a_chat_opened_after_an_outside_switch_is_headed_with_the_new_model() {
        let _home = crate::testenv::HomeGuard::new("follow-open-header");
        let chat = chat::test_chat_answering("noted", true);
        chat::test_switch_unseen(&chat, "noted", "test-b");
        let app = app(Arc::clone(&chat));
        let opened = app
            .clone()
            .oneshot(post("/api/chat/opened-late", ""))
            .await
            .unwrap();
        assert!(opened.status().is_success(), "{}", opened.status());

        let dir = mecha_core::session::Session::default_dir().unwrap();
        let path = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .expect("opening created a session");
        let header = mecha_core::session::Session::load(&path).unwrap().0;
        assert_eq!(
            header.model, "test-b",
            "headed with the model from before the switch"
        );
        let configs: Vec<String> = mecha_core::session::Session::run_configs(&path)
            .unwrap()
            .into_iter()
            .map(|c| c.model)
            .collect();
        assert_eq!(configs, vec!["test-b".to_string()]);
    }

    /// Incognito's gates are re-derived from the binding each turn runs on,
    /// not kept from the one it opened on: a switch onto a model behind a
    /// cloud URL stops the next turn, as `open_incognito` would have refused it.
    #[tokio::test]
    async fn an_incognito_turn_is_regated_against_the_binding_it_runs_on() {
        let _home = crate::testenv::HomeGuard::new("follow-incognito-regate");
        let Some(_runtime) = RuntimeDir::new() else {
            return;
        };
        let chat = chat::test_chat_answering("noted", true);
        let app = app(Arc::clone(&chat));
        let opened = app
            .clone()
            .oneshot(post("/api/incognito", ""))
            .await
            .unwrap();
        assert_eq!(opened.status(), StatusCode::OK);
        let key = body(opened).await["key"].as_str().unwrap().to_string();
        converse(&app, &key, "first").await;

        chat::test_switch(&chat, "noted", "cloud-model", false);
        let sent = app
            .clone()
            .oneshot(json_post(
                &format!("/api/chat/{key}/send"),
                serde_json::json!({ "text": "second" }).to_string(),
            ))
            .await
            .unwrap();
        assert!(!sent.status().is_success(), "{}", sent.status());
        let why =
            String::from_utf8(to_bytes(sent.into_body(), 10_000).await.unwrap().to_vec()).unwrap();
        assert!(why.contains("local model"), "{why}");
    }

    /// A ComfyUI stand-in for the image leg: answers what `image_generate`
    /// asks, writes the preview into `temp` when a job is submitted (as the
    /// real server does), and records every request — and every file it
    /// wrote, so the test knows the deletion it checks was not vacuous.
    async fn fake_image_server(temp: PathBuf) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
        use axum::extract::Path as UrlPath;
        use axum::routing::{get, post};
        const PREVIEW: &str = "c_temp_00001_.png";
        let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let log = |seen: &Arc<std::sync::Mutex<Vec<String>>>, line: String| {
            seen.lock().unwrap().push(line);
        };
        let record = move |id: &str| {
            serde_json::json!({ id: {
                "status": {"status_str": "success", "completed": true},
                "outputs": {"out": {"images": [
                    {"filename": PREVIEW, "subfolder": "", "type": "temp"}
                ]}}
            }})
        };
        let (s1, s2, s3, s4) = (seen.clone(), seen.clone(), seen.clone(), seen.clone());
        let app = Router::new()
            .route(
                "/object_info/{node}",
                get(|UrlPath(node): UrlPath<String>| async move {
                    let files = |input: &str, name: &str| {
                        serde_json::json!({ node.clone(): {"input": {"required": {input: [[name], {}]}}}})
                    };
                    axum::Json(match node.as_str() {
                        "UnetLoaderGGUF" => files("unet_name", "Qwen-Image-2.1-Q4.gguf"),
                        "CLIPLoader" => files("clip_name", "qwen3vl_8b_w4a8.safetensors"),
                        "VAELoader" => files("vae_name", "qwen_image_2.1_vae_bf16.safetensors"),
                        _ => serde_json::json!({ node.clone(): {"input": {}} }),
                    })
                }),
            )
            .route(
                "/prompt",
                post(move |body: String| async move {
                    log(&s1, format!("POST /prompt {body}"));
                    std::fs::write(temp.join(PREVIEW), b"\x89PNG\r\n\x1a\nfake").unwrap();
                    log(&s1, format!("wrote {PREVIEW}"));
                    let id = serde_json::from_str::<serde_json::Value>(&body)
                        .ok()
                        .and_then(|v| v["prompt_id"].as_str().map(str::to_string))
                        .unwrap_or_else(|| "job-1".into());
                    axum::Json(serde_json::json!({ "prompt_id": id }))
                }),
            )
            .route(
                "/history/{id}",
                get(move |UrlPath(id): UrlPath<String>| async move { axum::Json(record(&id)) }),
            )
            .route(
                "/history",
                post(move |body: String| async move {
                    log(&s2, format!("POST /history {body}"));
                    axum::Json(serde_json::json!({}))
                }),
            )
            .route(
                "/queue",
                get(|| async {
                    axum::Json(serde_json::json!({"queue_running": [], "queue_pending": []}))
                })
                .post(move |body: String| async move {
                    log(&s3, format!("POST /queue {body}"));
                    axum::Json(serde_json::json!({}))
                }),
            )
            .route(
                "/view",
                get(|| async {
                    (
                        [(axum::http::header::CONTENT_TYPE, "image/png")],
                        b"\x89PNG\r\n\x1a\nfake".to_vec(),
                    )
                }),
            )
            .route(
                "/free",
                post(move |body: String| async move {
                    log(&s4, format!("POST /free {body}"));
                    axum::Json(serde_json::json!({}))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        (url, seen)
    }

    /// What the process logs at the default level (`warn`), captured.
    #[derive(Clone, Default)]
    struct Captured(Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// `$TMPDIR` pointed at a fresh directory for the guard's life (under
    /// the `HomeGuard` lock, which serialises every test that moves process
    /// environment), so what this process stages there can be scanned.
    struct TmpDir(PathBuf, Option<std::ffi::OsString>);
    impl TmpDir {
        fn new(at: PathBuf) -> Self {
            std::fs::create_dir_all(&at).unwrap();
            let previous = std::env::var_os("TMPDIR");
            std::env::set_var("TMPDIR", &at);
            TmpDir(at, previous)
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            match &self.1 {
                Some(v) => std::env::set_var("TMPDIR", v),
                None => std::env::remove_var("TMPDIR"),
            }
        }
    }

    #[tokio::test]
    async fn an_incognito_picture_leaves_nothing_here_or_on_the_image_server() {
        let home = crate::testenv::HomeGuard::new("incognito-image");
        let Some(runtime) = RuntimeDir::new() else {
            return;
        };
        const CANARY: &str = "KUMQUAT-9911";
        let tmp = TmpDir::new(runtime.0.join("tmpdir"));
        // The image server's temp directory, in RAM as comfyui.service's is.
        let server_temp = runtime.0.join("comfy-temp");
        std::fs::create_dir_all(&server_temp).unwrap();
        let (url, seen) = fake_image_server(server_temp.clone()).await;
        // The default level, as journald gets it.
        let logs = Captured::default();
        let writer = logs.clone();
        let _logging = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::WARN)
                .with_ansi(false)
                .with_writer(move || writer.clone())
                .finish(),
        );

        let chat = chat::test_chat_drawing(
            &format!("a shop sign that reads {CANARY}"),
            mecha_core::imagegen::ImageConfig {
                url,
                min_available_mb: 0,
                unload_after_secs: 0,
                server_temp_dir: Some(server_temp.clone()),
                ..Default::default()
            },
        );
        let app = app(chat.clone());
        let opened = app
            .clone()
            .oneshot(post("/api/incognito", ""))
            .await
            .unwrap();
        assert_eq!(opened.status(), StatusCode::OK);
        let key = body(opened).await["key"].as_str().unwrap().to_string();
        converse(&app, &key, "draw the sign").await;

        // While open: the picture is in the room, and the server has already
        // been asked to forget the job and has lost its preview.
        let room = chat.room_of(&key).await.unwrap();
        let files: Vec<String> = std::fs::read_dir(room.workspace.join("images"))
            .map(|d| {
                d.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        let pictures: Vec<&String> = files.iter().filter(|n| n.ends_with(".png")).collect();
        assert_eq!(
            pictures.len(),
            1,
            "the picture landed in the room: {files:?}"
        );
        // Its manifest is in the room beside it — and goes with the room,
        // which the close below checks for every file, not just the picture.
        assert_eq!(files.len(), 2, "the picture and its manifest: {files:?}");
        let seen = seen.lock().unwrap().clone();
        let submitted = seen.iter().find(|l| l.starts_with("POST /prompt")).unwrap();
        assert!(
            submitted.contains(CANARY),
            "the drawing reached the server: {submitted}"
        );
        let id = serde_json::from_str::<serde_json::Value>(
            submitted.trim_start_matches("POST /prompt "),
        )
        .unwrap()["prompt_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            seen.iter()
                .any(|l| l.starts_with("POST /history") && l.contains(&id)),
            "the job's record, prompt and all, stayed on the server: {seen:?}"
        );
        assert!(
            seen.iter().any(|l| l == "wrote c_temp_00001_.png"),
            "the server never wrote a preview, so its deletion proves nothing"
        );
        assert!(
            !server_temp.join("c_temp_00001_.png").exists(),
            "the server's preview stayed in its temp directory"
        );
        assert!(
            mecha_core::imagegen::read_trail(&room.image_trail)
                .contains(&mecha_core::imagegen::TrailEntry::Job(id.clone())),
            "the job was not recorded where a sweep would find it"
        );

        // End: nothing anywhere this process writes, at any level journald sees.
        let ended = app
            .clone()
            .oneshot(post(&format!("/api/incognito/{key}/end"), ""))
            .await
            .unwrap();
        assert_eq!(ended.status(), StatusCode::NO_CONTENT);
        assert!(!room.root.exists(), "the room, picture and trail, is gone");
        for (what, dir) in [
            ("the mecha home", &home.dir),
            ("$TMPDIR", &tmp.0),
            ("the runtime directory", &runtime.0),
        ] {
            assert!(
                files_containing(dir, CANARY).is_empty(),
                "{what} holds the canary: {:?}",
                files_containing(dir, CANARY)
            );
        }
        // The capture is live — a warning from this thread lands in it — so
        // its silence about the canary means something.
        tracing::warn!("capture-probe");
        let logged = String::from_utf8_lossy(&logs.0.lock().unwrap()).to_string();
        assert!(
            logged.contains("capture-probe"),
            "the log capture saw nothing"
        );
        assert!(
            !logged.contains(CANARY),
            "the default log level carries it: {logged}"
        );
    }

    #[tokio::test]
    async fn an_open_page_keeps_the_chat_and_a_closed_chat_answers_gone() {
        let _home = crate::testenv::HomeGuard::new("incognito-alive");
        let Some(_runtime) = RuntimeDir::new() else {
            return;
        };
        let chat = chat::test_chat_answering("noted", true);
        let app = app(chat.clone());
        let opened = app
            .clone()
            .oneshot(post("/api/incognito", ""))
            .await
            .unwrap();
        let key = body(opened).await["key"].as_str().unwrap().to_string();
        // Aged past the idle limit, then pinged by the open page: kept.
        let room = chat.room_of(&key).await.unwrap();
        room.backdate(incognito::IDLE + std::time::Duration::from_secs(60));
        let alive = app
            .clone()
            .oneshot(post(&format!("/api/incognito/{key}/alive"), ""))
            .await
            .unwrap();
        assert_eq!(alive.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            chat.reap_idle_incognito().await,
            0,
            "a pinged chat is in use"
        );
        // Ended: the ping says so, and so does every ordinary door.
        app.clone()
            .oneshot(post(&format!("/api/incognito/{key}/end"), ""))
            .await
            .unwrap();
        for uri in [
            format!("/api/incognito/{key}/alive"),
            format!("/api/chat/{key}"),
        ] {
            let r = app.clone().oneshot(post(&uri, "")).await.unwrap();
            assert_eq!(r.status(), StatusCode::GONE, "{uri}");
        }
        // The read too, so the page shows the gone screen rather than an error.
        let read = app
            .clone()
            .oneshot(get(&format!("/api/chat/{key}")))
            .await
            .unwrap();
        assert_eq!(read.status(), StatusCode::GONE);
        let ordinary = app
            .clone()
            .oneshot(post("/api/incognito/main/alive", ""))
            .await
            .unwrap();
        assert_eq!(ordinary.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn end_is_only_for_an_open_incognito_chat() {
        let _home = crate::testenv::HomeGuard::new("incognito-end");
        let app = app(chat::test_chat());
        let ordinary = app
            .clone()
            .oneshot(post("/api/incognito/main/end", ""))
            .await
            .unwrap();
        assert_eq!(ordinary.status(), StatusCode::BAD_REQUEST);
        let unknown = app
            .clone()
            .oneshot(post(
                &format!("/api/incognito/{}/end", incognito::new_key()),
                "",
            ))
            .await
            .unwrap();
        assert_eq!(unknown.status(), StatusCode::GONE);
    }

    #[tokio::test]
    async fn mutations_require_a_non_simple_header_before_any_handler_runs() {
        let _home = crate::testenv::HomeGuard::new("web-csrf");
        let app = app(chat::test_chat());
        for uri in [
            "/api/chat/main/upload?name=x.txt",
            "/api/outbox/known-id/approve",
            "/api/chat/main/cancel",
            "/api/settings/charter",
        ] {
            let mut req = post(uri, "forged");
            req.headers_mut().remove("x-mecha-request");
            req.headers_mut()
                .insert("content-type", HeaderValue::from_static("text/plain"));
            req.headers_mut().insert(
                "origin",
                HeaderValue::from_static("https://untrusted.example"),
            );
            req.headers_mut()
                .insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
            assert_eq!(
                app.clone().oneshot(req).await.unwrap().status(),
                StatusCode::FORBIDDEN,
                "{uri}"
            );
        }
        assert!(!mecha_core::work::producer_dir("web")
            .unwrap()
            .join("main")
            .exists());
        // Non-browser clients may omit Fetch Metadata, but still need the
        // custom header. A foreign browser context is refused even with it.
        for (intent, site, expected) in [
            (None, None, StatusCode::FORBIDDEN),
            (Some("wrong"), Some("same-origin"), StatusCode::FORBIDDEN),
            (Some("1"), Some("same-site"), StatusCode::FORBIDDEN),
            (Some("1"), Some("cross-site"), StatusCode::FORBIDDEN),
            (Some("1"), Some("same-origin"), StatusCode::NOT_FOUND),
            (Some("1"), None, StatusCode::NOT_FOUND),
        ] {
            // Disabled voice offers return 404 once the guard admits them;
            // this route exercises raw bytes without a JSON extractor.
            let mut req = post("/api/offer", "{}");
            req.headers_mut().remove("x-mecha-request");
            if let Some(value) = intent {
                req.headers_mut()
                    .insert("x-mecha-request", HeaderValue::from_static(value));
            }
            if let Some(value) = site {
                req.headers_mut()
                    .insert("sec-fetch-site", HeaderValue::from_static(value));
            }
            assert_eq!(app.clone().oneshot(req).await.unwrap().status(), expected);
        }
        let req = Request::builder()
            .method("OPTIONS")
            .uri("/api/chat/main/upload?name=x")
            .header(TAILSCALE_LOGIN, "owner@example.com")
            .header("origin", "https://untrusted.example")
            .header("access-control-request-method", "POST")
            .header("access-control-request-headers", "x-mecha-request")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert!(!response
            .headers()
            .contains_key("access-control-allow-origin"));
    }

    #[tokio::test]
    async fn uploads_cannot_follow_symlinks_or_overwrite_an_existing_file() {
        let home = crate::testenv::HomeGuard::new("web-file-jail");
        let app = app(chat::test_chat());
        let ws = chat::session_workspace("main").unwrap();
        let outside = home.dir.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, ws.join("inbox")).unwrap();
        let response = app
            .clone()
            .oneshot(post("/api/chat/main/upload?name=x.txt", "probe"))
            .await
            .unwrap();
        assert!(!response.status().is_success());
        assert!(!outside.join("x.txt").exists());
        std::fs::remove_file(ws.join("inbox")).unwrap();
        std::fs::create_dir(ws.join("inbox")).unwrap();
        std::os::unix::fs::symlink(outside.join("x.txt"), ws.join("inbox/x.txt")).unwrap();
        let response = app
            .clone()
            .oneshot(post("/api/chat/main/upload?name=x.txt", "first"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let first = body(response).await;
        assert!(!outside.join("x.txt").exists());
        let response = app
            .oneshot(post("/api/chat/main/upload?name=x.txt", "second"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let second = body(response).await;
        assert_ne!(first["path"], second["path"]);
        assert_eq!(
            std::fs::read_to_string(ws.join(first["path"].as_str().unwrap())).unwrap(),
            "first"
        );
    }

    #[tokio::test]
    async fn downloading_from_an_unknown_chat_creates_no_session_or_workspace() {
        let home = crate::testenv::HomeGuard::new("web-read-creates-nothing");
        let app = app(chat::test_chat());
        let request = Request::builder()
            .uri("/api/chat/unknown/file?path=x.txt")
            .header(TAILSCALE_LOGIN, "owner@example.com")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.oneshot(request).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        assert!(!home.dir.join("work/web/unknown").exists());
        assert!(!home.dir.join("sessions").exists());
    }

    #[tokio::test]
    async fn chat_mode_rejects_a_decoded_path_escape_before_creating_state() {
        let home = crate::testenv::HomeGuard::new("web-mode-path");
        let mut request = post(
            "/api/chat/%2E%2E%2F%2E%2E%2Fescape/mode",
            r#"{"mode":"ask"}"#,
        );
        request
            .headers_mut()
            .insert("content-type", HeaderValue::from_static("application/json"));
        let response = app(chat::test_chat()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(!home.dir.join("sessions").exists());
        assert!(!home.dir.join("escape").exists());
    }

    #[tokio::test]
    async fn shutdown_refuses_typed_and_hosted_voice_admission() {
        use crate::voice::SessionHost;
        let home = crate::testenv::HomeGuard::new("web-shutdown-admission");
        let chat = chat::test_chat();
        chat.stop().await;
        let mut request = post("/api/chat/new/send", r#"{"text":"too late"}"#);
        request
            .headers_mut()
            .insert("content-type", HeaderValue::from_static("application/json"));
        assert_eq!(
            app(chat.clone()).oneshot(request).await.unwrap().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert!(matches!(
            chat::VoiceHost(chat.clone())
                .speak("new", "too late", false, false)
                .await,
            crate::voice::Hosted::Failed(_)
        ));
        chat.drain().await;
        assert!(!home.dir.join("sessions").exists());
    }

    #[tokio::test]
    async fn chat_reads_never_create_and_explicit_open_is_guarded_and_idempotent() {
        let home = crate::testenv::HomeGuard::new("web-explicit-open");
        let app = app(chat::test_chat());
        for method in ["GET", "HEAD"] {
            for suffix in ["", "/events", "/todo"] {
                let req = Request::builder()
                    .method(method)
                    .uri(format!("/api/chat/new{suffix}"))
                    .header(TAILSCALE_LOGIN, "owner@example.com")
                    .body(Body::empty())
                    .unwrap();
                assert_eq!(
                    app.clone().oneshot(req).await.unwrap().status(),
                    StatusCode::NOT_FOUND
                );
                assert!(!home.dir.join("sessions").exists());
                assert!(!home.dir.join("work/web/new").exists());
            }
        }
        let mut forged = post("/api/chat/new", "");
        forged.headers_mut().remove("x-mecha-request");
        assert_eq!(
            app.clone().oneshot(forged).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        assert!(!home.dir.join("sessions").exists());
        for _ in 0..2 {
            assert_eq!(
                app.clone()
                    .oneshot(post("/api/chat/new", ""))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::NO_CONTENT
            );
        }
        assert_eq!(
            std::fs::read_dir(home.dir.join("sessions"))
                .unwrap()
                .count(),
            1
        );
        for suffix in ["", "/events", "/todo"] {
            let req = Request::builder()
                .uri(format!("/api/chat/new{suffix}"))
                .header(TAILSCALE_LOGIN, "owner@example.com")
                .body(Body::empty())
                .unwrap();
            assert_eq!(
                app.clone().oneshot(req).await.unwrap().status(),
                StatusCode::OK
            );
        }
        assert_eq!(
            std::fs::read_dir(home.dir.join("sessions"))
                .unwrap()
                .count(),
            1
        );
    }

    /// Point the graph redactor at nothing for the guard's lifetime: a test
    /// that deletes a conversation must never reach the owner's live graph,
    /// which `mecha-graph` on `PATH` and `~/.mecha-graph` would. Under the
    /// `HomeGuard` lock, which serialises every test that moves environment.
    struct NoGraph(Vec<(&'static str, Option<std::ffi::OsString>)>);
    impl NoGraph {
        fn under(home: &std::path::Path) -> Self {
            let saved = ["MECHA_GRAPH_BIN", "MECHA_GRAPH_DB"]
                .into_iter()
                .map(|k| (k, std::env::var_os(k)))
                .collect();
            std::env::set_var("MECHA_GRAPH_BIN", home.join("no-such-mecha-graph"));
            std::env::set_var("MECHA_GRAPH_DB", home.join("no-such-graph.db"));
            NoGraph(saved)
        }
    }
    impl Drop for NoGraph {
        fn drop(&mut self) {
            for (k, v) in &self.0 {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }

    #[tokio::test]
    async fn an_archived_chat_leaves_the_lists_and_a_deleted_one_leaves_no_trace() {
        const CANARY: &str = "the cartographer's lemon-yellow kayak";
        let home = crate::testenv::HomeGuard::new("web-archive-delete");
        let _graph = NoGraph::under(&home.dir);
        let app = app(chat::test_chat_answering("noted", false));
        converse(&app, "chat-arch", CANARY).await;
        let rail = body(app.clone().oneshot(get("/api/sessions")).await.unwrap()).await;
        let id = rail["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["key"] == "chat-arch")
            .and_then(|r| r["id"].as_str())
            .unwrap()
            .to_string();
        let listed = |v: &serde_json::Value, id: &str| {
            v["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["id"] == id)
        };

        // Archive: out of the rail and the list, into the archive, and the
        // record untouched.
        let transcript = home.dir.join("sessions").join(format!("{id}.jsonl"));
        let before = std::fs::read(&transcript).unwrap();
        let r = app
            .clone()
            .oneshot(post(&format!("/api/sessions/{id}/archive"), ""))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert!(!listed(
            &body(app.clone().oneshot(get("/api/sessions")).await.unwrap()).await,
            &id
        ));
        assert!(!listed(
            &body(app.clone().oneshot(get("/api/history")).await.unwrap()).await,
            &id
        ));
        let archived = body(
            app.clone()
                .oneshot(get("/api/history?archived=true"))
                .await
                .unwrap(),
        )
        .await;
        assert!(listed(&archived, &id), "{archived}");
        assert_eq!(std::fs::read(&transcript).unwrap(), before);

        let r = app
            .clone()
            .oneshot(post(&format!("/api/sessions/{id}/unarchive"), ""))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        assert!(listed(
            &body(app.clone().oneshot(get("/api/history")).await.unwrap()).await,
            &id
        ));

        // Opening an archived conversation un-archives it (owner's ruling).
        let r = app
            .clone()
            .oneshot(post(&format!("/api/sessions/{id}/archive"), ""))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        let r = app
            .clone()
            .oneshot(json_post(
                "/api/resume",
                serde_json::json!({ "id": id }).to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert!(
            !mecha_core::archive::is_archived(&home.dir.join("sessions"), &id),
            "opening an archived conversation left it archived"
        );
        // Resumed under a new key; let it go again so the delete below can run.
        let r = app
            .clone()
            .oneshot(post(&format!("/api/sessions/{id}/archive"), ""))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::NO_CONTENT);

        // A detached task run writing this conversation from another process:
        // delete refuses rather than remove its workspace mid-call.
        let markers = crate::commands::tasks::markers().unwrap();
        markers
            .mark_running_for("task-live", None, Some(&id))
            .unwrap();
        let busy = Request::builder()
            .method("DELETE")
            .uri(format!("/api/sessions/{id}"))
            .header(TAILSCALE_LOGIN, "owner@example.com")
            .header("x-mecha-request", "1")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(busy).await.unwrap().status(),
            StatusCode::CONFLICT
        );
        assert!(
            transcript.exists(),
            "a refused delete touched the transcript"
        );
        markers.clear("task-live");

        // A marker store that cannot be read is not "no run": the delete
        // refuses rather than remove a workspace a run may be using.
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = markers.dir().to_path_buf();
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o000)).unwrap();
            let blind = Request::builder()
                .method("DELETE")
                .uri(format!("/api/sessions/{id}"))
                .header(TAILSCALE_LOGIN, "owner@example.com")
                .header("x-mecha-request", "1")
                .body(Body::empty())
                .unwrap();
            let status = app.clone().oneshot(blind).await.unwrap().status();
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
            // Root reads a 000 directory anyway; only then is this vacuous.
            if std::fs::read_dir("/root").is_err() {
                assert_eq!(status, StatusCode::CONFLICT);
                assert!(transcript.exists());
            }
        }

        // Delete: a mutation like any other, so the intent header is required.
        let bare = Request::builder()
            .method("DELETE")
            .uri(format!("/api/sessions/{id}"))
            .header(TAILSCALE_LOGIN, "owner@example.com")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(bare).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        assert!(transcript.exists());

        assert!(
            !files_containing(&home.dir, CANARY).is_empty(),
            "the canary was never recorded"
        );
        let del = Request::builder()
            .method("DELETE")
            .uri(format!("/api/sessions/{id}"))
            .header(TAILSCALE_LOGIN, "owner@example.com")
            .header("x-mecha-request", "1")
            .body(Body::empty())
            .unwrap();
        let r = app.clone().oneshot(del).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let report = body(r).await;
        assert_eq!(report["complete"], true, "{report}");
        assert_eq!(files_containing(&home.dir, CANARY), Vec::<PathBuf>::new());
        assert_eq!(files_containing(&home.dir, &id), Vec::<PathBuf>::new());
        assert!(!listed(
            &body(app.clone().oneshot(get("/api/history")).await.unwrap()).await,
            &id
        ));
        assert!(!home.dir.join("work/web/chat-arch").exists());
    }

    #[tokio::test]
    async fn resumed_attachments_use_the_recorded_workspace_and_stream_downloads() {
        let home = crate::testenv::HomeGuard::new("web-resume-files");
        let app = app(chat::test_chat());
        let ws = home.dir.join("task-workspace");
        std::fs::create_dir_all(&ws).unwrap();
        let session = mecha_core::session::Session::create(
            &mecha_core::session::Session::default_dir().unwrap(),
            mecha_core::session::SessionMeta {
                id: mecha_core::session::Session::new_id(),
                created_at: chrono::Utc::now(),
                provider: "test".into(),
                model: "test".into(),
                workspace: ws.clone(),
                title: Some("test attachment resume".into()),
                kind: Some(mecha_core::session::SessionKind::Test),
            },
        )
        .unwrap();
        let mut req = post("/api/resume", "");
        req.headers_mut()
            .insert("content-type", HeaderValue::from_static("application/json"));
        *req.body_mut() = Body::from(serde_json::json!({"id": session.meta.id}).to_string());
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let key = body(response).await["key"].as_str().unwrap().to_string();
        let response = app
            .clone()
            .oneshot(post(
                &format!("/api/chat/{key}/upload?name=x.txt"),
                "in the original jail",
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let path = body(response).await["path"].as_str().unwrap().to_string();
        assert_eq!(
            std::fs::read_to_string(ws.join(&path)).unwrap(),
            "in the original jail"
        );
        let req = Request::builder()
            .uri(format!("/api/chat/{key}/file?path={path}"))
            .header(TAILSCALE_LOGIN, "owner@example.com")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()["content-type"],
            "application/octet-stream"
        );
        assert_eq!(
            to_bytes(response.into_body(), 100).await.unwrap().as_ref(),
            b"in the original jail"
        );
    }
}
