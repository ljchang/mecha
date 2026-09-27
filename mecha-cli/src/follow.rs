//! A long-lived surface following the router's loaded model, turn by turn.
//!
//! `mecha serve` (chat and the voice facade), `voice-serve` and the Slack
//! connector build one agent and serve every conversation from it. Under the
//! llama-server router the request's `model` field *selects* (§14, D12), so an
//! agent resolved at startup names the model that was loaded then — and once
//! the owner switches, that surface's next turn loads the old model back. The
//! owner's ruling (2026-09-27): **one model serves every surface, and a switch
//! from any of them is a switch for all of them.** So these surfaces follow.
//!
//! [`Follower::follow`] is called before each turn. It re-observes the router
//! and, when the loaded model is no longer the one the current binding was
//! built on, rebuilds through [`setup::prepare`] — the same code that built it
//! at startup, so every value derived from the provider is derived again:
//! the context window and with it the compaction threshold and the tool-output
//! budget, sampling, vision, pricing, the subagents' own providers. A switch
//! already costs a model load of ten to forty seconds; one rebuild beside it,
//! MCP servers included, is noise, and it happens only when the model changed.
//!
//! **A run keeps the binding it started with** — the binding is an `Arc`,
//! taken once at the top of a turn — **but that does not keep the model.** The
//! router evicts only an idle model, which protects one *request*, not a run:
//! between a run's requests the model is idle, a switch made then completes,
//! and the run's next request names its own model and loads it back — after
//! which every following surface follows that. The same holds for a run in
//! any per-run process (a trigger, `mecha run`), and for the title a web chat
//! names after its run. Whether a switch should wait for runs, or runs should
//! re-follow between requests, is an open question for the owner
//! (REMOTE-SURFACE-DESIGN §14); nothing here claims either.
//!
//! **The config is read from disk on every turn**, because a rebuild does
//! (`setup::prepare` loads it), and the two must agree: resolving against the
//! startup file while building from the current one would miss a provider
//! added since, and gate on a sandbox the rebuilt agent no longer has. A file
//! that does not load keeps the current binding and says so once — it never
//! wedges every surface on the next switch.
//!
//! **Not following is sometimes right, and never silent:**
//! - A surface started with `--provider` or `--model` is pinned and does not
//!   move — but it still observes, because the background permit pool is
//!   sized to whatever is loaded (`router::background_seats`).
//! - A router that was not seen (restarting, down, unreadable), or seen with a
//!   model resident that no entry names or several do, or with more than one
//!   model resident (a swap mid-flight), is no evidence of *which* entry to
//!   move to, so the binding stays. Moving to the default on that would have
//!   the next request load production over the owner's choice.
//! - A rebuild that fails fails the turn, loudly. Falling back to the old
//!   binding would have its request load the old model — silently undoing the
//!   owner's switch, which is the degrading-guard shape.

use crate::setup::{self, Prepared};
use crate::GlobalOpts;
use anyhow::{Context, Result};
use mecha_core::agent::Agent;
use mecha_core::config::Config;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

/// What a surface takes from one [`setup::prepare`]: the agent and everything
/// read off the build beside it. Replaced whole on a switch, never edited.
pub struct Bound {
    pub agent: Arc<Agent>,
    pub provider_name: String,
    pub model: String,
    /// The resolved provider's window, for the context gauge.
    pub context_window: Option<u64>,
    /// For `RunConfig::levers_off`: the switches are not readable off an agent.
    pub levers_off: Vec<mecha_core::harness::Lever>,
    /// For `RunConfig::rules_hash`, carried the same way.
    pub rules: mecha_core::learning::RulesCarried,
    /// The resolved config this binding was built from, for `RunConfig::of`.
    pub config: Config,
    pub workspace: PathBuf,
    /// The todo tool in this agent's registry. Carried across a rebuild (the
    /// same handle is put into the new registry), because its lists are keyed
    /// by each run's jail and live in memory: a switch must not wipe the plan
    /// of every open conversation.
    pub todo: Option<Arc<mecha_core::tool::todo::TodoTool>>,
    /// Which build this is, counting from 1. A surface records a fresh
    /// `RunConfig` into a conversation when the generation it last recorded
    /// is not the one its next turn runs on — so a transcript that crossed a
    /// switch says which model answered each run, not only the first.
    pub generation: u64,
    /// Dropping an MCP client kills its server; held as long as any run holds
    /// this binding.
    _mcp: Vec<Arc<mecha_core::mcp::McpClient>>,
}

impl Bound {
    fn from_prepared(p: Prepared, generation: u64) -> Self {
        let context_window = p
            .config
            .providers
            .get(&p.provider_name)
            .and_then(|c| c.context_window);
        Bound {
            agent: Arc::new(p.agent),
            provider_name: p.provider_name,
            model: p.model,
            context_window,
            levers_off: p.levers_off,
            rules: p.rules,
            config: p.config,
            workspace: p.workspace,
            todo: p.todo,
            generation,
            _mcp: p._mcp,
        }
    }

    /// The MCP clients, for a surface that probes their health.
    pub fn mcp(&self) -> &[Arc<mecha_core::mcp::McpClient>] {
        &self._mcp
    }
}

/// What a surface adds to a fresh build before it is shared — `serve`'s
/// `ask_user`, routed to the session that asked. Run on every rebuild, so a
/// switch never produces an agent missing a tool the surface registered.
pub type Finish = Box<dyn Fn(&mut Prepared) + Send + Sync>;

pub struct Follower {
    opts: GlobalOpts,
    /// Named by `--provider` or `--model`: observed, never moved.
    pinned: bool,
    /// Asks the router at all. Off only for a test's fixed binding.
    observes: bool,
    finish: Finish,
    current: RwLock<Arc<Bound>>,
    /// One rebuild at a time: concurrent turns that all see the switch wait
    /// for the first to finish and take its result.
    rebuilding: tokio::sync::Mutex<()>,
    generations: AtomicU64,
    /// `observe` returns the same warning every turn while its cause stands;
    /// each is logged once per occurrence — forgotten when its condition
    /// clears (`observe`) — not once per message.
    warned: Mutex<HashSet<String>>,
}

impl Follower {
    /// Build the first binding. `opts` are the surface's own, exactly as it
    /// would have handed them to `setup::prepare`.
    pub async fn start(opts: GlobalOpts, finish: Finish) -> Result<Self> {
        let cfg = load_config(&opts)?;
        let pinned = opts.provider.is_some() || opts.model.is_some();
        let warned = Mutex::new(HashSet::new());
        observe(&cfg, pinned, &warned).await;
        let generations = AtomicU64::new(0);
        let first = build(&opts, &finish, &generations, resolve(&cfg, pinned), None).await?;
        Ok(Follower {
            opts,
            pinned,
            observes: true,
            finish,
            current: RwLock::new(first),
            rebuilding: tokio::sync::Mutex::new(()),
            generations,
            warned,
        })
    }

    fn warn_once(&self, w: String) {
        if self
            .warned
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(w.clone())
        {
            tracing::warn!("{w}");
        }
    }

    /// The router this surface's model is served from, when it is a local
    /// server on this machine — what a run's hold is keyed by (D13).
    pub fn router_base(&self) -> Option<String> {
        let bound = self.current();
        let url = bound
            .config
            .providers
            .get(&bound.provider_name)
            .filter(|p| p.kind == "local")?
            .base_url
            .as_deref()?;
        mecha_core::provider::router::is_loopback(url)
            .then(|| mecha_core::provider::router::base(url))
    }

    /// A turn's start under D13: hold the router — waiting out any pending
    /// switch, and telling `on_wait` which — *then* follow it. In that order,
    /// so a turn can never resolve the old model and then wait out the switch
    /// that replaces it. The caller keeps the hold until the run, and the
    /// title named after it, have ended; dropping it is what lets a switch go.
    pub async fn enter(
        &self,
        what: &str,
        on_wait: impl FnMut(&mecha_core::hold::Switch),
    ) -> Result<(Option<mecha_core::hold::Held>, Arc<Bound>)> {
        let held = match self.router_base() {
            Some(base) => Some(
                mecha_core::hold::Holds::open_default()?
                    .hold_when_clear(&base, what, on_wait)
                    .await?,
            ),
            None => None,
        };
        Ok((held, self.follow().await?))
    }

    /// [`enter`](Self::enter)'s hold without the wait, for a surface that must
    /// not wait in place (the Slack connector's single event loop): `Err` is
    /// the pending switch, and the caller defers the turn until it clears.
    pub fn try_hold(
        &self,
        what: &str,
    ) -> Result<std::result::Result<Option<mecha_core::hold::Held>, mecha_core::hold::Switch>> {
        let Some(base) = self.router_base() else {
            return Ok(Ok(None));
        };
        Ok(mecha_core::hold::Holds::open_default()?
            .try_hold(&base, what)?
            .map(Some))
    }

    /// The binding as it stands, without asking the router. For what is not a
    /// turn: the page's session listing, a health probe.
    pub fn current(&self) -> Arc<Bound> {
        Arc::clone(&self.current.read().unwrap_or_else(|e| e.into_inner()))
    }

    /// The binding the next turn should run on: re-observe the router, and
    /// rebuild if the loaded model moved. Call once per turn, before taking
    /// any lock the turn holds, and use the result for the whole turn.
    pub async fn follow(&self) -> Result<Arc<Bound>> {
        if !self.observes {
            return Ok(self.current());
        }
        let cfg = match load_config(&self.opts) {
            Ok(cfg) => cfg,
            Err(e) => {
                self.warn_once(format!(
                    "the config did not load, so this surface stays on [providers.{}] until \
                     it does: {e:#}",
                    self.current().provider_name
                ));
                return Ok(self.current());
            }
        };
        observe(&cfg, self.pinned, &self.warned).await;
        let Some(want) = resolve(&cfg, self.pinned) else {
            return Ok(self.current());
        };
        if self.current().provider_name == want {
            return Ok(self.current());
        }
        let _one = self.rebuilding.lock().await;
        // Another turn may have rebuilt while this one waited.
        let cur = self.current();
        if cur.provider_name == want {
            return Ok(cur);
        }
        let built = build(
            &self.opts,
            &self.finish,
            &self.generations,
            Some(want.clone()),
            Some(&cur),
        );
        let bound = built.await.with_context(|| {
            format!(
                "the router's loaded model is now served by [providers.{want}], and the agent \
                 could not be rebuilt for it — this turn stops rather than run on \
                 [providers.{}], whose request would load that model back",
                cur.provider_name
            )
        })?;
        tracing::info!(
            "following the router: [providers.{}] ({}) → [providers.{}] ({})",
            cur.provider_name,
            cur.model,
            bound.provider_name,
            bound.model
        );
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = Arc::clone(&bound);
        Ok(bound)
    }
}

#[cfg(test)]
impl Follower {
    /// A binding that never moves and never asks a router, around an agent a
    /// test built itself — the surfaces' tests drive a scripted provider.
    pub fn fixed(agent: Agent, provider_name: &str, model: &str, config: Config) -> Self {
        let bound = Bound {
            agent: Arc::new(agent),
            provider_name: provider_name.into(),
            model: model.into(),
            context_window: None,
            levers_off: Vec::new(),
            rules: Default::default(),
            config: config.clone(),
            workspace: PathBuf::new(),
            todo: None,
            generation: 1,
            _mcp: Vec::new(),
        };
        Follower {
            opts: GlobalOpts::default(),
            pinned: true,
            observes: false,
            finish: Box::new(|_| {}),
            current: RwLock::new(Arc::new(bound)),
            rebuilding: tokio::sync::Mutex::new(()),
            generations: AtomicU64::new(1),
            warned: Mutex::new(HashSet::new()),
        }
    }
}

/// A hold on the router `provider` (the default when `None`) is served from,
/// for a process that is not a long-lived surface: a command that is one run
/// (`main`), and each trigger fire. `None` when that provider is not a local
/// server on this machine — nothing to switch there. Waits out a pending
/// switch first, saying so on stderr, which is where these processes speak.
pub async fn hold_router(
    cfg: &Config,
    provider: Option<&str>,
    what: &str,
) -> Result<Option<mecha_core::hold::Held>> {
    let name = provider.unwrap_or(&cfg.default_provider);
    let Some(url) = cfg
        .providers
        .get(name)
        .filter(|p| p.kind == "local")
        .and_then(|p| p.base_url.as_deref())
        .filter(|u| mecha_core::provider::router::is_loopback(u))
    else {
        return Ok(None);
    };
    let held = mecha_core::hold::Holds::open_default()?
        .hold_when_clear(&mecha_core::provider::router::base(url), what, |switch| {
            eprintln!(
                "mecha: the model is switching to {} (since {}) — waiting for it to load; \
                 `mecha model cancel-switch` withdraws a switch that is stuck",
                switch.to,
                switch.started_at.format("%H:%M:%SZ")
            )
        })
        .await?;
    Ok(Some(held))
}

#[cfg(test)]
impl Follower {
    /// Install a new binding under the next generation — what a turn sees
    /// after `follow` rebuilt for a switch, without a router or a rebuild.
    pub fn switch_to(&self, agent: Agent, provider_name: &str, model: &str, config: Config) {
        let generation = self.generations.fetch_add(1, Ordering::Relaxed) + 1;
        let bound = Bound {
            agent: Arc::new(agent),
            provider_name: provider_name.into(),
            model: model.into(),
            context_window: None,
            levers_off: Vec::new(),
            rules: Default::default(),
            config,
            workspace: PathBuf::new(),
            todo: None,
            generation,
            _mcp: Vec::new(),
        };
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(bound);
    }
}

/// Re-observe the routers `cfg` follows, logging each warning once.
///
/// Once per *occurrence*, not per process: a warning whose condition has
/// cleared is forgotten, so the same condition coming back weeks later into
/// a `serve` is logged again rather than never.
async fn observe(cfg: &Config, pinned: bool, warned: &Mutex<HashSet<String>>) {
    let warnings = mecha_core::provider::router::observe(cfg, !pinned).await;
    let mut warned = warned.lock().unwrap_or_else(|e| e.into_inner());
    warned.retain(|w| warnings.contains(w));
    for w in warnings {
        if warned.insert(w.clone()) {
            tracing::warn!("{w}");
        }
    }
}

/// The provider the default resolves to now, or `None` to stay where the
/// surface is: pinned, or the router the default follows was not seen.
fn resolve(cfg: &Config, pinned: bool) -> Option<String> {
    let base = cfg
        .providers
        .get(&cfg.default_provider)
        .and_then(|p| p.base_url.as_deref());
    let seen = base.and_then(mecha_core::provider::router::observed);
    resolve_seen(cfg, pinned, seen)
}

/// The pure half of [`resolve`], against what the router showed (`seen`, or
/// `None` when it was not seen). Separate so each way of *not* moving can be
/// tested without a router.
///
/// Moves only on evidence of where to: nothing resident resolves to the
/// default (every process does that after a router restart), a resident
/// model resolves to the one entry that names it, and anything else — a
/// resident model no entry names or several do — stays.
fn resolve_seen(
    cfg: &Config,
    pinned: bool,
    seen: Option<mecha_core::provider::router::Seen>,
) -> Option<String> {
    if pinned {
        return None;
    }
    let name = &cfg.default_provider;
    let default = cfg.providers.get(name)?;
    if !mecha_core::provider::router::follows_here(default) {
        return Some(name.clone());
    }
    let seen = seen?;
    let Some(resident) = seen.resident.as_deref() else {
        return Some(name.clone());
    };
    if default.model.as_deref() == Some(resident) {
        return Some(name.clone());
    }
    mecha_core::provider::router::followed(cfg, name, std::slice::from_ref(&seen))
}

/// One `setup::prepare`, finished by the surface. `provider` names the
/// resolved entry, so `build` takes exactly it rather than resolving the
/// default a second time against a snapshot another turn may have replaced
/// in between; `None` keeps the surface's own options (pinned, or the router
/// unseen at startup).
async fn build(
    opts: &GlobalOpts,
    finish: &Finish,
    generations: &AtomicU64,
    provider: Option<String>,
    carry: Option<&Bound>,
) -> Result<Arc<Bound>> {
    let mut opts = opts.clone();
    if provider.is_some() {
        opts.provider = provider;
    }
    let mut prepared = setup::prepare(&opts, false).await?;
    if let (Some(old), Some(_)) = (carry.and_then(|b| b.todo.clone()), &prepared.todo) {
        prepared
            .agent
            .registry_mut()
            .insert(Arc::clone(&old) as Arc<dyn mecha_core::tool::Tool>);
        prepared.todo = Some(old);
    }
    finish(&mut prepared);
    let generation = generations.fetch_add(1, Ordering::Relaxed) + 1;
    Ok(Arc::new(Bound::from_prepared(prepared, generation)))
}

fn load_config(opts: &GlobalOpts) -> Result<Config> {
    if opts.global_config_only {
        Config::load_global()
    } else {
        Config::load(&std::env::current_dir().context("cannot determine the working directory")?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::config::ProviderConfig;
    use mecha_core::provider::router::Seen;

    const ROUTER: &str = "http://127.0.0.1:8080";

    /// Production following the router, the uncensored arm and Gemma as its
    /// siblings on the same port — the installed shape (§14).
    fn cfg(follow: bool) -> Config {
        let mut cfg = Config {
            default_provider: "local".into(),
            ..Default::default()
        };
        for (name, model, follows) in [
            ("local", "qwen3.6-35b-a3b", follow),
            ("local-uncensored", "qwen3.6-35b-a3b-uncensored", false),
            ("gemma26", "gemma-4-26b-a4b", false),
        ] {
            cfg.providers.insert(
                name.into(),
                ProviderConfig {
                    kind: "local".into(),
                    base_url: Some(ROUTER.into()),
                    model: Some(model.into()),
                    follow_loaded: follows,
                    ..Default::default()
                },
            );
        }
        cfg
    }

    fn seen(resident: Option<&str>) -> Option<Seen> {
        Some(Seen {
            base_url: ROUTER.into(),
            resident: resident.map(str::to_string),
            slots: Some(4),
        })
    }

    /// The case the module exists for: the owner loaded another model from
    /// somewhere else, and this surface's next turn moves onto its entry
    /// instead of naming production and loading it back.
    #[test]
    fn a_switch_made_elsewhere_is_followed() {
        assert_eq!(
            resolve_seen(&cfg(true), false, seen(Some("qwen3.6-35b-a3b-uncensored"))).as_deref(),
            Some("local-uncensored")
        );
        assert_eq!(
            resolve_seen(&cfg(true), false, seen(Some("gemma-4-26b-a4b"))).as_deref(),
            Some("gemma26")
        );
    }

    /// A router that was not seen — restarting, down, unreadable — is no
    /// evidence the pick moved. Resolving to the default here would make the
    /// next request load production over the owner's choice.
    #[test]
    fn an_unseen_router_moves_nothing() {
        assert_eq!(resolve_seen(&cfg(true), false, None), None);
    }

    /// Seen with nothing loaded (after a restart or a crash): the default
    /// stands, as it does for every other process (§14).
    #[test]
    fn a_router_with_nothing_loaded_resolves_to_the_default() {
        assert_eq!(
            resolve_seen(&cfg(true), false, seen(None)).as_deref(),
            Some("local")
        );
    }

    /// A model no entry names — a provider added to the file since, a preset
    /// loaded by hand — is no evidence of where to go. Resolving it to the
    /// default was the review's finding: the next turn named production and
    /// loaded it over the owner's choice.
    #[test]
    fn a_model_no_entry_names_moves_nothing() {
        assert_eq!(
            resolve_seen(&cfg(true), false, seen(Some("something-added-since"))),
            None
        );
    }

    /// Two entries naming the resident model is a guess either way; stay.
    #[test]
    fn a_model_two_entries_name_moves_nothing() {
        let mut c = cfg(true);
        let twin = c.providers["local-uncensored"].clone();
        c.providers.insert("local-uncensored-2".into(), twin);
        assert_eq!(
            resolve_seen(&c, false, seen(Some("qwen3.6-35b-a3b-uncensored"))),
            None
        );
    }

    /// The default's own model resident resolves to the default — the switch
    /// back to production.
    #[test]
    fn the_defaults_own_model_resolves_to_the_default() {
        assert_eq!(
            resolve_seen(&cfg(true), false, seen(Some("qwen3.6-35b-a3b"))).as_deref(),
            Some("local")
        );
    }

    /// `--provider` / `--model` name what the surface runs; it never moves,
    /// whatever is loaded.
    #[test]
    fn a_pinned_surface_does_not_move() {
        assert_eq!(
            resolve_seen(&cfg(true), true, seen(Some("gemma-4-26b-a4b"))),
            None
        );
    }

    /// A default that does not follow is itself, and needs no router at all —
    /// so a machine without one behaves exactly as before.
    #[test]
    fn a_default_that_does_not_follow_is_itself() {
        assert_eq!(
            resolve_seen(&cfg(false), false, seen(Some("gemma-4-26b-a4b"))).as_deref(),
            Some("local")
        );
        assert_eq!(
            resolve_seen(&cfg(false), false, None).as_deref(),
            Some("local")
        );
    }
}
