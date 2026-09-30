//! Which feature every route belongs to, and the one guard that refuses a
//! route whose feature is off (FEATURES-DESIGN.md §4.2 item 4, §9 step 3).
//!
//! **Declared where registered.** `router` adds every route through
//! [`Owned::at`], which records the route's [`Owner`] beside it, so a route
//! cannot be added without saying whose it is — and a test holds `router` to
//! that (`every_route_is_registered_with_its_owner`). Before this, an off
//! feature's routes failed in whichever of five ways the code underneath
//! happened to: 503, 502, 409, 501, or a 200 that created a store.
//!
//! **One answer, behind the owner.** The guard sits inside `owner_guard`, so
//! a probe without the owner's header gets the owner guard's 403 and never
//! learns which features exist; the owner gets F4's 404 with the feature and
//! the command that turns it on. It runs before the handler, so a read with a
//! side effect (`/api/frontdoor` creating its store) no longer happens for a
//! feature that is off.
//!
//! **Read on every request, as the nav is.** A request to a feature's route
//! asks the global file then (config plus disk, never a socket — the read
//! `/api/features` makes per request). A start-time snapshot broke both
//! directions: a feature enabled while `serve` ran showed its tab and 404'd
//! every request on it, naming the command just run, and one disabled kept
//! writing until a restart (review of #451). Only the chat's tools still
//! load once, which is what `/api/features`' `pending` marks.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{FromRequestParts, MatchedPath, RawPathParams, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::MethodRouter;
use axum::{Json, Router};
use mecha_core::feature::{self, Facts, Feature, Refusal};

/// Whose a route is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Owner {
    /// Core: works whatever is switched on (the outbox, questions, the chat).
    Core,
    Of(Feature),
    /// `/api/proposals/{store}/…`: the `entities` store is the graph's, the
    /// `harness` and `rules` stores are core — the `store` capture decides.
    ProposalStore,
    /// `/api/chat/{key}/…`: an incognito room's key is incognito's, every
    /// other key the chat's — the `key` capture decides.
    ChatKey,
}

impl Owner {
    /// The feature a request to this route belongs to, if any, given the
    /// route's captures **as the handler will see them** — percent-decoded,
    /// from the router's own match (`RawPathParams`). Reading the raw URI
    /// instead let `/api/proposals/%65ntities/…` past a graph that is off,
    /// into a handler that decodes it to `entities` and writes the graph
    /// (review of #451). A capture that cannot be read fails closed: the
    /// route is taken to be the feature's.
    fn feature(self, capture: impl Fn(&str) -> Option<String>) -> Option<Feature> {
        match self {
            Owner::Core => None,
            Owner::Of(f) => Some(f),
            // Whose store it is comes from the table the handler dispatches
            // on (`proposals::queue_of` → `review_source`), not a second
            // hand-kept name: a graph-backed store added there is the
            // graph's here, and an unknown one fails closed (review of #451).
            Owner::ProposalStore => capture("store")
                .and_then(|store| super::proposals::queue_of(&store))
                .and_then(crate::commands::review::review_source)
                .is_none_or(|source| source.graph)
                .then_some(Feature::Graph),
            Owner::ChatKey => capture("key")
                .is_none_or(|key| key.starts_with(super::incognito::KEY_PREFIX))
                .then_some(Feature::Incognito),
        }
    }
}

/// Every route and its owner, built together.
pub(super) struct Owned {
    router: Router<super::WebState>,
    owners: BTreeMap<&'static str, Owner>,
}

impl Owned {
    pub(super) fn new() -> Self {
        Owned {
            router: Router::new(),
            owners: BTreeMap::new(),
        }
    }

    /// `Router::route`, with the route's owner beside it.
    pub(super) fn at(
        mut self,
        path: &'static str,
        owner: Owner,
        method_router: MethodRouter<super::WebState>,
    ) -> Self {
        let previous = self.owners.insert(path, owner);
        assert!(previous.is_none(), "{path} is registered twice");
        self.router = self.router.route(path, method_router);
        self
    }

    pub(super) fn finish(self) -> (Router<super::WebState>, Arc<Owners>) {
        (self.router, Arc::new(Owners(self.owners)))
    }
}

/// The owner of every registered route, by its template (`MatchedPath`).
#[derive(Debug, Default)]
pub(super) struct Owners(pub(super) BTreeMap<&'static str, Owner>);

/// Where the guard's refusals come from.
#[derive(Debug)]
pub(super) enum Gate {
    /// The global configuration and the disk, read per request.
    Live,
    /// A fixed set, for tests: the refusals given and nothing else.
    #[cfg_attr(not(test), allow(dead_code))]
    Fixed(BTreeMap<Feature, Refusal>),
}

impl Default for Gate {
    /// Refuse nothing — the tests' router, which switches nothing on.
    fn default() -> Self {
        Gate::Fixed(BTreeMap::new())
    }
}

impl Gate {
    #[cfg(test)]
    pub(super) fn refusing(refusals: impl IntoIterator<Item = Refusal>) -> Gate {
        Gate::Fixed(refusals.into_iter().map(|r| (r.feature, r)).collect())
    }

    /// Why `f`'s route refuses now, if it does. A feature whose guard has
    /// not landed never costs a read. A configuration that does not load
    /// refuses — the guard cannot run, so the route does not either, as
    /// `features::require` refuses a verb.
    async fn refusal(&self, f: Feature) -> Option<Refusal> {
        match self {
            Gate::Fixed(refusals) => refusals.get(&f).cloned(),
            Gate::Live if !f.gated() => None,
            Gate::Live => {
                let unknown = move |why: String| Refusal {
                    feature: f,
                    why: format!("cannot tell whether it is switched on — {why}"),
                    fix: "fix ~/.mecha/config.toml".into(),
                };
                let read = tokio::task::spawn_blocking(move || {
                    let home = mecha_core::work::mecha_home()?;
                    let cfg = mecha_core::config::Config::load_global()?;
                    anyhow::Ok(feature::refusal(&Facts::read(&home, &cfg), f))
                })
                .await;
                match read {
                    Ok(Ok(refusal)) => refusal,
                    Ok(Err(e)) => Some(unknown(format!(
                        "the global configuration did not load: {e:#}"
                    ))),
                    // Never open on a read that did not finish.
                    Err(e) => Some(unknown(format!("the read did not finish: {e}"))),
                }
            }
        }
    }
}

/// The guard: a route whose feature this process loaded as off answers F4's
/// 404 and never reaches its handler.
pub(super) async fn guard(
    State((gate, owners)): State<(Arc<Gate>, Arc<Owners>)>,
    request: Request,
    next: Next,
) -> Response {
    // No matched route is the static fallback (the web app itself, which
    // `serve` refuses to start without) or a 404 already.
    let owner = request
        .extensions()
        .get::<MatchedPath>()
        .and_then(|m| owners.0.get(m.as_str()).copied());
    let (mut parts, body) = request.into_parts();
    let captures = RawPathParams::from_request_parts(&mut parts, &())
        .await
        .ok();
    let request = Request::from_parts(parts, body);
    let capture = |name: &str| {
        captures
            .as_ref()?
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.to_string())
    };
    let refusal = match owner.and_then(|o| o.feature(capture)) {
        Some(f) => gate.refusal(f).await,
        None => None,
    };
    match refusal {
        None => next.run(request).await,
        Some(r) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "feature_off",
                "feature": r.feature,
                "why": r.why,
                "fix": r.fix,
            })),
        )
            .into_response(),
    }
}
