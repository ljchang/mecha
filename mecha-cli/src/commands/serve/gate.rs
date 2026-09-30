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
//! **Loaded once, like the rest of `serve`.** The refusals are read when the
//! process starts, from the same configuration its chat's tools were built
//! from; `/api/features` marks a switch flipped since as pending.

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
            Owner::ProposalStore => capture("store")
                .is_none_or(|store| store == "entities")
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

/// The refusals this process loaded when it started: every feature whose
/// routes answer `feature_off`.
#[derive(Debug, Default)]
pub(super) struct Gate(BTreeMap<Feature, Refusal>);

impl Gate {
    /// Read from the global configuration and the disk, as `/api/features`
    /// reads its rows — never a socket.
    pub(super) fn at_start(facts: &Facts) -> Gate {
        Gate(
            Feature::ALL
                .iter()
                .filter_map(|&f| feature::refusal(facts, f).map(|r| (f, r)))
                .collect(),
        )
    }

    /// For tests: the refusals given, and nothing else.
    #[cfg(test)]
    pub(super) fn refusing(refusals: impl IntoIterator<Item = Refusal>) -> Gate {
        Gate(refusals.into_iter().map(|r| (r.feature, r)).collect())
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
    let refusal = owner
        .and_then(|o| o.feature(capture))
        .and_then(|f| gate.0.get(&f));
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
