//! llama-server's router mode: one process on one port, one child server per
//! *loaded* model, and the request's `model` field choosing between them
//! (`REMOTE-SURFACE-DESIGN.md` §14, D12).
//!
//! **The pick is the router's loaded model, and there is no second store for
//! it.** A provider entry marked `follow_loaded` stands for "whatever that
//! router has resident": when a run takes the default provider, it takes the
//! sibling entry naming the resident model instead. The owner chooses by
//! loading (`mecha model use`, the chip, the TUI picker); every consumer
//! follows by asking. A remembered "current model" would be a second answer
//! that can disagree with the server, which is `/props`' rule applied to
//! choosing.
//!
//! **Once per process, not once per request.** [`observe`] snapshots the
//! server when a process starts, and every `Config::provider(None)` in that
//! process maps against the snapshot. A run keeps its model to the end, and a
//! `mecha eval` or `batch` is one model for its whole sweep; a process that
//! outlives one run (the trigger daemon) observes again per run.
//!
//! **`Config::provider(None)` reads that snapshot, a process-global** — the
//! hazard `brief::seats_under` was parameterised to avoid. It is accepted
//! here because the global is inert unless a config marks an entry
//! `follow_loaded`, and [`followed`] then only ever picks among the entries
//! of the config it is handed; tests that write it take turns and restore it.
//!
//! Two router answers are traps, both measured against `c841aee`:
//!
//! - `GET /props` with no `?model=` answers **200 with a placeholder** —
//!   `model_alias: "llama-server"`, `n_ctx: 0`. `role: "router"` is the
//!   envelope that says so; read it before the content.
//! - A proxied GET that names a model **loads it** unless it says
//!   `autoload=false`. A probe must never be the thing that swaps the model.

use crate::config::{Config, ProviderConfig};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::sync::RwLock;
use std::time::Duration;

/// What one router had resident when this process looked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    /// Normalised with [`base`].
    pub base_url: String,
    /// `None` is a router with nothing loaded — after a restart, before the
    /// first request. The default provider then stands, and its first request
    /// loads it.
    pub resident: Option<String>,
    /// The resident model's slot count (`-np`), from its own `/props`.
    pub slots: Option<u64>,
}

/// One entry of the router's `GET /models`.
#[derive(Debug, Clone, Deserialize)]
pub struct RouterModel {
    pub id: String,
    #[serde(default)]
    pub status: Status,
}

/// `#[serde(default)]` throughout: another program's output across versions.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct Status {
    /// `unloaded` | `loading` | `loaded` | `sleeping` | `downloading`.
    #[serde(default)]
    pub value: String,
    /// Set on an `unloaded` model whose child exited while loading.
    #[serde(default)]
    pub failed: bool,
    /// The child's command line — the preset, as flags. Where a preset's
    /// sampling is read from ([`preset_temperature`]).
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub exit_code: Option<i64>,
}

impl RouterModel {
    /// Holding (or about to hold) the router's memory. `sleeping` is loaded
    /// and idle-suspended; `loading` is resident for every purpose a caller
    /// has, because a request for it queues rather than evicting it.
    pub fn is_resident(&self) -> bool {
        matches!(
            self.status.value.as_str(),
            "loaded" | "loading" | "sleeping"
        )
    }
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    role: Option<String>,
}

#[derive(Deserialize)]
struct ModelList {
    #[serde(default)]
    data: Vec<RouterModel>,
}

/// The one spelling of a base URL two entries are compared under: the
/// server root, as `brief::read_slots` already takes it — `…:8080`,
/// `…:8080/` and `…:8080/v1` are one router (found on review).
pub fn base(url: &str) -> String {
    url.trim_end_matches('/')
        .trim_end_matches("/v1")
        .trim_end_matches('/')
        .to_string()
}

/// Whether `follow_loaded` is honoured on this entry: a llama-server
/// (`kind = "local"`) **on this machine**. `kind` names the wire dialect, not
/// the location — a `local` entry can point at a tailnet host — so the
/// address is checked too, or the "no request off-machine" promise would
/// rest on a dialect (found on review).
pub fn follows_here(p: &ProviderConfig) -> bool {
    p.follow_loaded && p.kind == "local" && p.base_url.as_deref().is_some_and(is_loopback)
}

/// Whether a base URL's host is this machine.
pub fn is_loopback(url: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(url) else {
        return false;
    };
    let Some(host) = u.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// The routers `follow_loaded` entries stand for. Local entries only: the
/// flag means nothing off-machine, and honouring it there would put a
/// request to someone else's server in front of every command — which
/// `setup::preflight_provider` refuses for the same reason (found on review).
pub fn followed_bases(cfg: &Config) -> Vec<String> {
    let mut bases: Vec<String> = cfg
        .providers
        .values()
        .filter(|p| follows_here(p))
        .filter_map(|p| p.base_url.as_deref().map(base))
        .collect();
    bases.sort();
    bases.dedup();
    bases
}

fn client(timeout: Duration) -> Option<reqwest::Client> {
    reqwest::Client::builder().timeout(timeout).build().ok()
}

/// Is this server a router? `None` when it did not answer at all.
pub async fn is_router(base_url: &str) -> Option<bool> {
    let http = client(Duration::from_secs(2))?;
    let body = http
        .get(format!("{}/props", base(base_url)))
        .send()
        .await
        .ok()?;
    if !body.status().is_success() {
        return None;
    }
    let env: Envelope = body.json().await.ok()?;
    Some(env.role.as_deref() == Some("router"))
}

/// The router's model list, or `None` when `base_url` is not a router (or is
/// not up). Silent on failure, like `preflight::fetch`: a provider that is not
/// running must not print on every start of a machine that does not use it.
pub async fn models(base_url: &str) -> Option<Vec<RouterModel>> {
    if is_router(base_url).await != Some(true) {
        return None;
    }
    list_on(&client(Duration::from_secs(2))?, &base(base_url)).await
}

/// `GET /models` on a server already known to be a router, on a client the
/// caller keeps — what the load and unload polls use, rather than paying a
/// fresh client and a `/props` check every tick (found on review).
async fn list_on(http: &reqwest::Client, b: &str) -> Option<Vec<RouterModel>> {
    let body = http.get(format!("{b}/models")).send().await.ok()?;
    if !body.status().is_success() {
        return None;
    }
    Some(body.json::<ModelList>().await.ok()?.data)
}

/// Every model status this build knows how to read (`c841aee`).
pub const KNOWN_STATUSES: [&str; 5] = ["unloaded", "loading", "loaded", "sleeping", "downloading"];

/// Whether a `/models` answer is one this can draw a conclusion from: a
/// non-empty list whose every status is one it knows. **"Nothing resident"
/// is a claim, and a list this does not fully understand cannot support it**
/// — read as nothing loaded, the default would stand and its first request
/// evict the owner's pick without a word. The same rule `model-idle.sh`
/// applies to the same answer (found on review).
pub fn readable(models: &[RouterModel]) -> bool {
    !models.is_empty()
        && models
            .iter()
            .all(|m| KNOWN_STATUSES.contains(&m.status.value.as_str()))
}

/// The resident model, if exactly one is. `--models-max 1` means at most one;
/// two resident is a server started otherwise, and naming either would be a
/// guess, so it is reported as none.
pub fn resident(models: &[RouterModel]) -> Option<&str> {
    let mut it = models.iter().filter(|m| m.is_resident());
    match (it.next(), it.next()) {
        (Some(one), None) => Some(one.id.as_str()),
        _ => None,
    }
}

/// This process's snapshot: what each router had resident, and whether a
/// default provider may follow it.
///
/// Two decisions, one read, and they are kept apart on purpose. A process
/// given `--model` or `--provider` has named what it runs and must not
/// follow — but its permit pool must still be sized to what is *loaded*, or
/// `mecha distill -p local` admits three background runs onto a one-slot
/// model (found on review).
struct Snapshot {
    seen: Vec<Seen>,
    follows: bool,
}

static SNAPSHOT: RwLock<Snapshot> = RwLock::new(Snapshot {
    seen: Vec::new(),
    follows: false,
});

/// Snapshot every router a `follow_loaded` provider points at, replacing the
/// last snapshot. `follows` is whether this process's default provider may
/// follow it (false when it was given `--model` or `--provider`); the slot
/// count is recorded either way. Returns warnings: a resident model no entry
/// names (when following), and a `follow_loaded` that is being ignored.
///
/// Called once at process start, and again per fire by anything that outlives
/// one run. Four loopback round trips per router with a model resident —
/// `/props` and `/models` here, then `preflight::fetch`'s bare `/props` and
/// the resident model's own; a refused connection costs nothing and leaves
/// the default standing.
pub async fn observe(cfg: &Config, follows: bool) -> Vec<String> {
    let mut seen = Vec::new();
    let mut unreadable = Vec::new();
    for b in followed_bases(cfg) {
        // Silence is kept for "no router here" (not a router, or nothing
        // listening). A router that *said* it is one and then gave no list
        // is a finding: unseen, the default would stand and swap the pick
        // out with nothing in the journal (found on review).
        let listed = if is_router(&b).await == Some(true) {
            let list = match client(Duration::from_secs(2)) {
                Some(http) => list_on(&http, &b).await,
                None => None,
            };
            if list.is_none() {
                unreadable.push(format!(
                    "the router at {b} did not answer /models with a model list, so which \
                     model is loaded is unknown — runs use the default provider, which may \
                     swap out the model that is loaded"
                ));
            }
            list
        } else {
            None
        };
        if let Some(list) = listed {
            if !readable(&list) {
                unreadable.push(format!(
                    "the router at {b} answered /models with a list this cannot read (empty, or \
                     a status it does not know), so which model is loaded is unknown — runs use \
                     the default provider, which may swap out the model that is loaded"
                ));
                continue;
            }
            let resident = resident(&list).map(str::to_string);
            if resident.is_none() && list.iter().filter(|m| m.is_resident()).count() > 1 {
                unreadable.push(format!(
                    "the router at {b} has more than one model resident, so which one a run \
                     follows would be a guess — runs use the default provider, which may swap \
                     one of them out"
                ));
            }
            // R4 wherever a model became resident — `load-on-startup`, a
            // `--model` run's autoload, a default run after a restart — not
            // only through `mecha model use`. A warning here: the model is
            // already loaded, and refusing it would refuse nothing (found on
            // review).
            if let Some(m) = resident
                .as_deref()
                .and_then(|r| list.iter().find(|m| m.id == r))
            {
                unreadable.extend(sampling_mismatches(cfg, &b, m));
            }
            let slots = match &resident {
                Some(m) => crate::provider::preflight::fetch(&b, Some(m))
                    .await
                    .and_then(|p| p.total_slots),
                None => None,
            };
            seen.push(Seen {
                resident,
                slots,
                base_url: b,
            });
        }
    }
    let mut warnings = if follows {
        orphans(cfg, &seen)
    } else {
        Vec::new()
    };
    warnings.extend(unreadable);
    for (name, p) in &cfg.providers {
        if p.follow_loaded && !follows_here(p) {
            warnings.push(format!(
                "[providers.{name}] sets follow_loaded, which is ignored unless the entry is \
                 kind = \"local\" at a loopback address (this one is kind = {:?} at {}): it \
                 means \"whatever a llama-server router on this machine has loaded\", and \
                 following any other server would put a request to it in front of every command.",
                p.kind,
                p.base_url.as_deref().unwrap_or("no base_url")
            ));
        }
    }
    if let Ok(mut slot) = SNAPSHOT.write() {
        *slot = Snapshot { seen, follows };
    }
    warnings
}

/// How many background runs may hold the model at once, under this process's
/// snapshot (`permit.rs`; owner's ruling 2026-09-26, §14 trap 5).
pub fn background_seats(fallback: usize) -> usize {
    SNAPSHOT
        .read()
        .map_or(fallback, |s| seats_for(&s.seen, fallback))
}

/// The pure half of [`background_seats`]: one seat short of the resident
/// model's slots, so the owner's turn does not queue — **but never fewer than
/// one**, so on a one-slot model (Gemma, Qwen3.8) background work and the
/// nightly passes still run on whatever is loaded, at most one at a time,
/// and the owner waits behind at most one decode. `fallback` (sized for
/// production's `-np 4`) when no router's slot count is known — or two
/// routers disagree, which would be a guess.
pub fn seats_for(seen: &[Seen], fallback: usize) -> usize {
    let mut known = seen.iter().filter_map(|s| s.slots);
    let Some(n) = known.next() else {
        return fallback;
    };
    if known.any(|m| m != n) {
        return fallback;
    }
    (n.saturating_sub(1) as usize).max(1)
}

/// The provider `name` stands for right now, under this process's snapshot.
/// `None` means `name` itself.
pub fn follow(cfg: &Config, name: &str) -> Option<String> {
    let s = SNAPSHOT.read().ok()?;
    if !s.follows {
        return None;
    }
    followed(cfg, name, &s.seen)
}

/// The pure half of [`follow`]: which entry `name` resolves to against `seen`.
///
/// `None` — `name` stands — when `name` does not follow, its router was not
/// seen, nothing is resident, `name` already names the resident model, or the
/// resident model is named by no entry or by more than one. The last two are
/// [`orphans`]' to report; resolving to a guess would record a model that did
/// not answer.
pub fn followed(cfg: &Config, name: &str, seen: &[Seen]) -> Option<String> {
    let p = cfg.providers.get(name)?;
    if !follows_here(p) {
        return None;
    }
    let b = base(p.base_url.as_deref()?);
    let resident = seen.iter().find(|s| s.base_url == b)?.resident.as_deref()?;
    if p.model.as_deref() == Some(resident) {
        return None;
    }
    let mut names = namers(cfg, &b, resident);
    match (names.next(), names.next()) {
        (Some(one), None) => Some(one.to_string()),
        _ => None,
    }
}

/// Every provider entry serving `model` from the router at `b`.
pub fn namers<'a>(cfg: &'a Config, b: &'a str, model: &'a str) -> impl Iterator<Item = &'a str> {
    cfg.providers
        .iter()
        .filter_map(move |(n, p): (&String, &ProviderConfig)| {
            (p.base_url.as_deref().map(base).as_deref() == Some(b)
                && p.model.as_deref() == Some(model))
            .then_some(n.as_str())
        })
}

/// A resident model that no single entry names: every default run would name
/// the default's model instead, and the router would swap it back — the owner
/// loses the pick without being told why.
fn orphans(cfg: &Config, seen: &[Seen]) -> Vec<String> {
    seen.iter()
        .filter_map(|s| unfollowable(cfg, &s.base_url, s.resident.as_deref()?))
        .collect()
}

/// Why default runs would **not** follow `model` resident on the router at
/// `b` — no entry names it, or several do — or `None` when they would. The
/// one statement of the rule, read by `observe` and by `mecha model use`'s
/// report, which had re-derived it and diverged (found on review).
///
/// The default already naming the resident model is the working case,
/// however many pinned aliases name it too: `followed` stands on it, and
/// "leave one" would be wrong advice.
pub fn unfollowable(cfg: &Config, b: &str, model: &str) -> Option<String> {
    let default_names = cfg.providers.get(&cfg.default_provider).is_some_and(|p| {
        follows_here(p)
            && p.base_url.as_deref().map(base).as_deref() == Some(b)
            && p.model.as_deref() == Some(model)
    });
    if default_names {
        return None;
    }
    match namers(cfg, b, model).count() {
        1 => None,
        0 => Some(format!(
            "the router at {b} has {model:?} loaded, but no [providers.*] entry names it with \
             that base_url — runs will use the default provider and swap it back out. Add an \
             entry with model = {model:?}."
        )),
        n => Some(format!(
            "the router at {b} has {model:?} loaded, and {n} [providers.*] entries name it — \
             which one answered would be a guess, so runs keep the default provider. Leave one."
        )),
    }
}

/// The temperature a model's preset starts its child with, off the router's
/// `/models` (`--temperature 0.6`; `--temp` is the same flag). `None` when the
/// preset leaves it to llama-server's default.
pub fn preset_temperature(m: &RouterModel) -> Option<f64> {
    let args = &m.status.args;
    args.iter()
        .position(|a| a == "--temperature" || a == "--temp")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
}

/// Entries naming `m` on the router at `b` whose `temperature` disagrees with
/// its preset (owner's ruling R4, 2026-09-26: such a model is refused).
///
/// Temperature because it is the one sampling value mecha sends on every
/// request that changes the distribution (`seed` is sent too, but only picks
/// a draw from it), so it silently overrides the preset — a model loaded under a
/// config that disagrees with its tuning is un-tuned without a word. An entry
/// with no `temperature` sends none, and the preset governs.
pub fn sampling_mismatches(cfg: &Config, b: &str, m: &RouterModel) -> Vec<String> {
    let preset = preset_temperature(m);
    cfg.providers
        .iter()
        .filter(|(_, p)| {
            p.base_url.as_deref().map(base).as_deref() == Some(b)
                && p.model.as_deref() == Some(m.id.as_str())
        })
        .filter_map(|(name, p)| {
            let t = p.temperature?;
            let agrees = preset.is_some_and(|pt| (pt - t).abs() < 1e-9);
            (!agrees).then(|| {
                format!(
                    "[providers.{name}] temperature = {t}, but the router's preset for {:?} \
                     starts it at {} — every request would override the preset's tuning",
                    m.id,
                    preset.map_or_else(|| "llama-server's default".to_string(), |p| p.to_string())
                )
            })
        })
        .collect()
}

/// Stop `model` on the router, **even mid-reply** — the request in flight
/// fails (owner's ruling R2: a switch may be made "now" rather than waiting
/// for idle). Waits until it is unloaded.
pub async fn unload(base_url: &str, model: &str, wait: Duration) -> Result<()> {
    let b = base(base_url);
    let http = client(Duration::from_secs(10)).context("building an HTTP client")?;
    let resp = http
        .post(format!("{b}/models/unload"))
        .json(&serde_json::json!({ "model": model }))
        .send()
        .await
        .with_context(|| format!("POST {b}/models/unload"))?;
    if !resp.status().is_success() {
        bail!(
            "the router refused to unload {model}: {} {}",
            resp.status(),
            resp.text().await.unwrap_or_default().trim()
        );
    }
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        // Gone is a list that answered and does not hold it resident —
        // absent counts, should a router ever drop unloaded entries (this one
        // keeps them listed). No answer is not gone.
        // Readable first: an empty or unknown-status list is not "gone"
        // either (the module's rule, found on review).
        let gone = list_on(&http, &b).await.is_some_and(|l| {
            readable(&l)
                && l.iter()
                    .find(|m| m.id == model)
                    .is_none_or(|m| !m.is_resident())
        });
        if gone {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            bail!(
                "{model} was still resident {}s after unloading it",
                wait.as_secs()
            );
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// How long a refused load is given to show up as loading anyway (a refusal
/// that raced a load already under way) before the refusal is the answer.
const REFUSAL_GRACE: Duration = Duration::from_secs(10);

/// Ask the router to load `model`, and wait until it is resident or has
/// failed. The router evicts the idle model to make room; one with a request
/// in flight finishes it first, which is the wait a caller sees.
pub async fn load(base_url: &str, model: &str, wait: Duration) -> Result<()> {
    let b = base(base_url);
    let http = client(Duration::from_secs(10)).context("building an HTTP client")?;
    let list = models(&b)
        .await
        .with_context(|| format!("{b} is not a llama-server router (or is not up)"))?;
    if !list.iter().any(|m| m.id == model) {
        bail!(
            "the router at {b} has no model {model:?}; it serves: {}",
            list.iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let resp = http
        .post(format!("{b}/models/load"))
        .json(&serde_json::json!({ "model": model }))
        .send()
        .await
        .with_context(|| format!("POST {b}/models/load"))?;
    // The router refuses a load in two cases (`post_router_models_load` at
    // `c841aee`): a model it does not know, which the check above rules out,
    // and one already running, which the poll below sees as resident. Any
    // other refusal means no load is coming — kept, so that instead of
    // waiting out `wait` and blaming a long request, the poll gives up after
    // a short grace and names it (found on review).
    //
    // A `failed` flag left by an earlier attempt is never read as this one's:
    // `server_models::load` sets the model `LOADING` before the POST handler
    // returns (`c841aee`), so by the first poll the stale status is gone.
    let status = resp.status();
    let refusal = if status.is_success() {
        None
    } else {
        Some(format!(
            "{status} from POST /models/load: {}",
            resp.text().await.unwrap_or_default().trim()
        ))
    };
    let started_at = tokio::time::Instant::now();
    let deadline = started_at + wait;
    let mut started = false;
    loop {
        if let Some(list) = list_on(&http, &b).await {
            if let Some(m) = list.iter().find(|m| m.id == model) {
                match m.status.value.as_str() {
                    "loaded" | "sleeping" => return Ok(()),
                    "loading" => started = true,
                    "unloaded" if m.status.failed => bail!(
                        "{model} failed to load (exit code {}); the router's journal has why",
                        m.status
                            .exit_code
                            .map_or_else(|| "unknown".to_string(), |c| c.to_string())
                    ),
                    // A status this build does not know — renamed, or absent
                    // (an empty value, by `serde(default)`) — says nothing about
                    // whether the load is coming: fail now rather than wait
                    // out `wait` on it (found on review).
                    other if !KNOWN_STATUSES.contains(&other) => bail!(
                        "the router reports {model} as {other:?}, a status this build does not \
                         know — `mecha model list` shows what it sees"
                    ),
                    // Unloaded and not failed: either the load has not been
                    // picked up yet (it waits for the resident model to go
                    // idle), or it came and went. Keep waiting until the
                    // deadline; the message says which it looked like.
                    _ => {}
                }
            }
        }
        if let Some(why) = &refusal {
            if !started && started_at.elapsed() >= REFUSAL_GRACE {
                bail!("the router refused to load {model}: {why}");
            }
        }
        if tokio::time::Instant::now() >= deadline {
            bail!(
                "{model} was not loaded after {}s ({}) — the model it replaces may still be \
                 answering a long request{}",
                wait.as_secs(),
                if started {
                    "it started loading"
                } else {
                    "it never started loading"
                },
                refusal
                    .as_deref()
                    .map(|r| format!("; the router said {r}"))
                    .unwrap_or_default()
            );
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(model: &str, url: &str, follow: bool) -> ProviderConfig {
        ProviderConfig {
            kind: "local".into(),
            model: Some(model.into()),
            base_url: Some(url.into()),
            follow_loaded: follow,
            ..Default::default()
        }
    }

    fn cfg() -> Config {
        let mut c = Config {
            default_provider: "local".into(),
            ..Default::default()
        };
        c.providers.insert(
            "local".into(),
            entry("qwen3.6-35b-a3b", "http://127.0.0.1:8080", true),
        );
        c.providers.insert(
            "gemma26".into(),
            entry("gemma-4-26b-a4b", "http://127.0.0.1:8080/", false),
        );
        c.providers.insert(
            "qwen38".into(),
            entry("qwen3.8-27b", "http://127.0.0.1:8080", false),
        );
        c
    }

    fn seen(resident: Option<&str>) -> Vec<Seen> {
        vec![Seen {
            base_url: "http://127.0.0.1:8080".into(),
            resident: resident.map(Into::into),
            slots: None,
        }]
    }

    #[test]
    fn the_default_follows_the_resident_model_to_the_entry_naming_it() {
        // The trailing slash on gemma26's URL is the same server.
        assert_eq!(
            followed(&cfg(), "local", &seen(Some("gemma-4-26b-a4b"))).as_deref(),
            Some("gemma26")
        );
    }

    #[test]
    fn nothing_resident_or_already_resident_leaves_the_name_standing() {
        assert_eq!(followed(&cfg(), "local", &seen(None)), None);
        assert_eq!(
            followed(&cfg(), "local", &seen(Some("qwen3.6-35b-a3b"))),
            None
        );
        assert_eq!(followed(&cfg(), "local", &[]), None, "router not seen");
    }

    #[test]
    fn an_entry_that_does_not_follow_is_pinned_even_when_another_model_is_resident() {
        // An experiment arm names its model; the loaded one must not move it.
        assert_eq!(
            followed(&cfg(), "qwen38", &seen(Some("gemma-4-26b-a4b"))),
            None
        );
    }

    #[test]
    fn a_resident_model_no_entry_names_is_reported_and_not_guessed() {
        let c = cfg();
        let s = seen(Some("qwen3.6-35b-a3b-uncensored"));
        assert_eq!(followed(&c, "local", &s), None);
        let w = orphans(&c, &s);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("no [providers.*] entry names it"), "{w:?}");
    }

    #[test]
    fn an_alias_of_the_resident_default_is_not_called_ambiguous() {
        let mut c = cfg();
        c.providers.insert(
            "prod-pinned".into(),
            entry("qwen3.6-35b-a3b", "http://127.0.0.1:8080", false),
        );
        assert!(orphans(&c, &seen(Some("qwen3.6-35b-a3b"))).is_empty());
    }

    #[test]
    fn a_models_list_it_cannot_fully_read_supports_no_conclusion() {
        let m = |v: &str| RouterModel {
            id: "m".into(),
            status: Status {
                value: v.into(),
                ..Default::default()
            },
        };
        assert!(readable(&[m("unloaded"), m("loaded")]));
        assert!(!readable(&[]), "empty");
        assert!(
            !readable(&[m("unloaded"), m("resident")]),
            "a renamed status"
        );
    }

    #[test]
    fn two_entries_naming_the_resident_model_is_ambiguous_and_not_guessed() {
        let mut c = cfg();
        c.providers.insert(
            "gemma-again".into(),
            entry("gemma-4-26b-a4b", "http://127.0.0.1:8080", false),
        );
        let s = seen(Some("gemma-4-26b-a4b"));
        assert_eq!(followed(&c, "local", &s), None);
        assert!(orphans(&c, &s)[0].contains("2 [providers.*] entries"));
    }

    #[test]
    fn an_entry_on_another_server_is_not_a_sibling() {
        let mut c = cfg();
        c.providers.remove("gemma26");
        c.providers.insert(
            "gemma-elsewhere".into(),
            entry("gemma-4-26b-a4b", "http://127.0.0.1:8082", false),
        );
        assert_eq!(followed(&c, "local", &seen(Some("gemma-4-26b-a4b"))), None);
    }

    #[test]
    fn background_seats_are_one_short_of_the_resident_models_slots_but_never_zero() {
        let at = |slots: Option<u64>| Seen {
            base_url: "http://127.0.0.1:8080".into(),
            resident: Some("m".into()),
            slots,
        };
        assert_eq!(seats_for(&[at(Some(4))], 3), 3, "production");
        assert_eq!(
            seats_for(&[at(Some(1))], 3),
            1,
            "a one-slot model keeps one seat"
        );
        assert_eq!(seats_for(&[at(Some(2))], 3), 1);
        assert_eq!(seats_for(&[at(None)], 3), 3, "unknown keeps the default");
        assert_eq!(seats_for(&[], 3), 3);
        assert_eq!(
            seats_for(&[at(Some(1)), at(Some(4))], 3),
            3,
            "two disagree: no guess"
        );
        assert_eq!(seats_for(&[at(Some(1)), at(Some(1))], 3), 1, "two agree");
    }

    #[test]
    fn a_preset_temperature_that_disagrees_with_config_is_named() {
        let m = |args: &[&str]| RouterModel {
            id: "gemma-4-26b-a4b".into(),
            status: Status {
                value: "unloaded".into(),
                args: args.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            },
        };
        let mut c = cfg();
        let b = "http://127.0.0.1:8080";
        // No temperature in config: mecha sends none, the preset governs.
        assert!(sampling_mismatches(&c, b, &m(&["--temperature", "0.6"])).is_empty());
        c.providers.get_mut("gemma26").unwrap().temperature = Some(0.6);
        assert!(sampling_mismatches(&c, b, &m(&["--temp", "0.6"])).is_empty());
        let w = sampling_mismatches(&c, b, &m(&["--temperature", "1.0"]));
        assert_eq!(w.len(), 1);
        assert!(
            w[0].contains("[providers.gemma26] temperature = 0.6"),
            "{w:?}"
        );
        // A preset that leaves it to the server's default disagrees with 0.6.
        let w = sampling_mismatches(&c, b, &m(&["--ctx-size", "32768"]));
        assert!(w[0].contains("llama-server's default"), "{w:?}");
    }

    #[test]
    fn a_local_kind_entry_off_this_machine_is_not_followed() {
        let mut c = cfg();
        c.providers.get_mut("local").unwrap().base_url = Some("http://studio.tailnet:8080".into());
        assert!(followed_bases(&c).is_empty());
        assert_eq!(followed(&c, "local", &seen(Some("gemma-4-26b-a4b"))), None);
        for url in [
            "http://127.0.0.1:8080",
            "http://localhost:8080/v1",
            "http://[::1]:8080",
        ] {
            assert!(is_loopback(url), "{url}");
        }
        assert!(!is_loopback("http://100.64.0.7:8080"));
    }

    #[test]
    fn a_v1_spelling_is_the_same_router() {
        let mut c = cfg();
        c.providers.get_mut("local").unwrap().base_url = Some("http://127.0.0.1:8080/v1".into());
        assert_eq!(base("http://127.0.0.1:8080/v1/"), "http://127.0.0.1:8080");
        assert_eq!(
            followed(&c, "local", &seen(Some("gemma-4-26b-a4b"))).as_deref(),
            Some("gemma26")
        );
    }

    #[test]
    fn follow_loaded_on_a_remote_entry_is_ignored_and_never_probed() {
        let mut c = cfg();
        let local = c.providers.get_mut("local").unwrap();
        local.kind = "openai".into();
        assert!(followed_bases(&c).is_empty(), "no request off-machine");
        assert_eq!(followed(&c, "local", &seen(Some("gemma-4-26b-a4b"))), None);
    }

    /// The snapshot is process-global; the tests that write it take turns.
    static SNAPSHOT_TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// A stub server answering one request per body, in order.
    async fn stub(bodies: Vec<&'static str>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut lines = Vec::new();
            for body in bodies {
                let (mut s, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 2048];
                let n = s.read(&mut buf).await.unwrap();
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                lines.push(head.lines().next().unwrap_or_default().to_string());
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                s.write_all(reply.as_bytes()).await.unwrap();
            }
            lines
        });
        (url, task)
    }

    /// A router with Gemma resident, as `observe` reads it: `/props`, then
    /// `/models`, then the resident model's own `/props` (the placeholder
    /// first, as `preflight::fetch` asks).
    const GEMMA_RESIDENT: [&str; 4] = [
        r#"{"role":"router","model_alias":"llama-server"}"#,
        r#"{"data":[{"id":"qwen3.6-35b-a3b","status":{"value":"unloaded"}},{"id":"gemma-4-26b-a4b","status":{"value":"loaded"}}]}"#,
        r#"{"role":"router","model_alias":"llama-server"}"#,
        r#"{"model_alias":"gemma-4-26b-a4b","total_slots":1,"default_generation_settings":{"n_ctx":32768}}"#,
    ];

    fn config_at(url: &str) -> Config {
        let mut c = Config {
            default_provider: "local".into(),
            ..Default::default()
        };
        c.providers
            .insert("local".into(), entry("qwen3.6-35b-a3b", url, true));
        c.providers
            .insert("gemma26".into(), entry("gemma-4-26b-a4b", url, false));
        c
    }

    /// The wiring, not just the pure half: `observe` reads a router, and
    /// `Config::provider(None)` — the call every default run makes — answers
    /// with the sibling. If `provider` stopped consulting the snapshot, every
    /// other test here would still pass.
    #[tokio::test]
    async fn a_default_run_takes_the_entry_naming_what_the_router_has_loaded() {
        let _turn = SNAPSHOT_TESTS.lock().await;
        let (url, server) = stub(GEMMA_RESIDENT.to_vec()).await;
        let c = config_at(&url);
        assert!(observe(&c, true).await.is_empty());
        server.await.unwrap();
        assert_eq!(c.provider(None).unwrap().0, "gemma26");
        assert_eq!(
            c.provider(Some("local")).unwrap().0,
            "local",
            "a name is a pin"
        );
        assert_eq!(
            background_seats(3),
            1,
            "one short of one slot, floored at one"
        );
        // Leave the process-global snapshot as a process with no router has it.
        observe(&Config::default(), false).await;
    }

    /// A process given `--model`/`--provider` does not follow, and its permit
    /// pool is still sized to what is loaded — the two decisions apart.
    #[tokio::test]
    async fn a_pinned_process_does_not_follow_but_still_sizes_its_seats() {
        let _turn = SNAPSHOT_TESTS.lock().await;
        let (url, server) = stub(GEMMA_RESIDENT.to_vec()).await;
        let c = config_at(&url);
        observe(&c, false).await;
        server.await.unwrap();
        assert_eq!(c.provider(None).unwrap().0, "local", "pinned: no follow");
        assert_eq!(
            background_seats(3),
            1,
            "seats still read off the resident model"
        );
        // Leave the process-global snapshot as a process with no router has it.
        observe(&Config::default(), false).await;
    }

    /// A router that says it is one and then gives no readable answer is a
    /// finding, not "no router here"; so is a router with two resident.
    #[tokio::test]
    async fn a_router_it_cannot_read_is_warned_about_not_taken_for_absent() {
        let _turn = SNAPSHOT_TESTS.lock().await;
        let router = r#"{"role":"router","model_alias":"llama-server"}"#;
        let (url, server) = stub(vec![router, "this is not a model list"]).await;
        let w = observe(&config_at(&url), true).await;
        server.await.unwrap();
        assert!(
            w.iter().any(|w| w.contains("did not answer /models")),
            "{w:?}"
        );

        let two = r#"{"data":[{"id":"a","status":{"value":"loaded"}},{"id":"b","status":{"value":"loaded"}}]}"#;
        let (url, server) = stub(vec![router, two]).await;
        let w = observe(&config_at(&url), true).await;
        server.await.unwrap();
        assert!(
            w.iter().any(|w| w.contains("more than one model resident")),
            "{w:?}"
        );
        observe(&Config::default(), false).await;
    }

    /// R4 is read wherever a model became resident, not only by `use`.
    #[tokio::test]
    async fn a_resident_model_whose_preset_disagrees_with_config_is_warned_about() {
        let _turn = SNAPSHOT_TESTS.lock().await;
        let listing = r#"{"data":[{"id":"gemma-4-26b-a4b","status":{"value":"loaded","args":["llama-server","--temperature","1.0"]}}]}"#;
        let (url, server) = stub(vec![
            GEMMA_RESIDENT[0],
            listing,
            GEMMA_RESIDENT[2],
            GEMMA_RESIDENT[3],
        ])
        .await;
        let mut c = config_at(&url);
        c.providers.get_mut("gemma26").unwrap().temperature = Some(0.6);
        let w = observe(&c, true).await;
        server.await.unwrap();
        assert!(
            w.iter()
                .any(|w| w.contains("[providers.gemma26] temperature = 0.6")),
            "{w:?}"
        );
        observe(&Config::default(), false).await;
    }

    #[tokio::test]
    async fn follow_loaded_off_machine_is_said_to_be_ignored() {
        let _turn = SNAPSHOT_TESTS.lock().await;
        let mut c = cfg();
        // Nothing listens here; the warning comes from the config alone.
        for p in c.providers.values_mut() {
            p.base_url = Some("http://127.0.0.1:9".into());
        }
        let gemma = c.providers.get_mut("gemma26").unwrap();
        gemma.kind = "openai".into();
        gemma.follow_loaded = true;
        let w = observe(&c, false).await;
        assert!(
            w.iter()
                .any(|w| w.contains("[providers.gemma26] sets follow_loaded")),
            "{w:?}"
        );
    }

    #[test]
    fn resident_is_one_model_or_none() {
        let m = |id: &str, v: &str| RouterModel {
            id: id.into(),
            status: Status {
                value: v.into(),
                ..Default::default()
            },
        };
        assert_eq!(resident(&[m("a", "unloaded"), m("b", "loaded")]), Some("b"));
        assert_eq!(
            resident(&[m("a", "loading")]),
            Some("a"),
            "a load in progress is resident"
        );
        assert_eq!(resident(&[m("a", "sleeping")]), Some("a"));
        assert_eq!(resident(&[m("a", "unloaded")]), None);
        assert_eq!(
            resident(&[m("a", "loaded"), m("b", "loaded")]),
            None,
            "two is a guess"
        );
    }

    #[test]
    fn the_router_model_list_parses_as_served() {
        // Captured from the router on 2026-09-26 (c841aee), args trimmed to
        // the flags that matter. **An unloaded model still reports its
        // preset's full argv** — which is what lets R4 check a model before
        // it is loaded — and a preset with no sampling (Gemma) has none.
        let body = r#"{"data":[
            {"id":"gemma-4-26b-a4b","status":{"value":"unloaded","args":["/home/u/.local/bin/llama-server","--host","127.0.0.1","--jinja","--port","0","--spec-type","draft-mtp","--alias","gemma-4-26b-a4b","--ctx-size","32768","--parallel","1"]}},
            {"id":"qwen3.8-27b","status":{"value":"loaded","args":["/home/u/.local/bin/llama-server","--host","127.0.0.1","--jinja","--min-p","0.0","--temperature","1.0","--top-k","20","--alias","qwen3.8-27b","--parallel","1"]}},
            {"id":"qwen3.6-35b-a3b","status":{"value":"unloaded","args":["/home/u/.local/bin/llama-server","--host","127.0.0.1","--temperature","0.6","--alias","qwen3.6-35b-a3b","--parallel","4"]}},
            {"id":"broken","status":{"value":"unloaded","failed":true,"exit_code":1}}
        ],"object":"list"}"#;
        let list: ModelList = serde_json::from_str(body).unwrap();
        assert_eq!(resident(&list.data), Some("qwen3.8-27b"));
        assert_eq!(
            preset_temperature(&list.data[0]),
            None,
            "Gemma: server default"
        );
        assert_eq!(preset_temperature(&list.data[1]), Some(1.0));
        assert_eq!(
            preset_temperature(&list.data[2]),
            Some(0.6),
            "read while unloaded"
        );
        assert!(list.data[3].status.failed);
        assert_eq!(list.data[3].status.exit_code, Some(1));
    }
}
