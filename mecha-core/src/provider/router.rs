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

/// The one spelling of a base URL two entries are compared under.
pub fn base(url: &str) -> String {
    url.trim_end_matches('/').to_string()
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
    let http = client(Duration::from_secs(2))?;
    let body = http
        .get(format!("{}/models", base(base_url)))
        .send()
        .await
        .ok()?;
    if !body.status().is_success() {
        return None;
    }
    Some(body.json::<ModelList>().await.ok()?.data)
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

static SEEN: RwLock<Vec<Seen>> = RwLock::new(Vec::new());

/// Snapshot every router a `follow_loaded` provider points at, replacing the
/// last snapshot. Returns what `Config::provider(None)` will now follow and a
/// warning for each resident model no provider entry names.
///
/// Called once at process start, and again per fire by anything that outlives
/// one run. Costs one loopback round trip per router; a refused connection
/// costs nothing and leaves the default standing.
pub async fn observe(cfg: &Config) -> Vec<String> {
    let mut bases: Vec<String> = cfg
        .providers
        .values()
        .filter(|p| p.follow_loaded)
        .filter_map(|p| p.base_url.as_deref().map(base))
        .collect();
    bases.sort();
    bases.dedup();
    let mut seen = Vec::new();
    for b in bases {
        if let Some(list) = models(&b).await {
            seen.push(Seen {
                resident: resident(&list).map(str::to_string),
                base_url: b,
            });
        }
    }
    let warnings = orphans(cfg, &seen);
    if let Ok(mut slot) = SEEN.write() {
        *slot = seen;
    }
    warnings
}

/// The provider `name` stands for right now, under this process's snapshot.
/// `None` means `name` itself.
pub fn follow(cfg: &Config, name: &str) -> Option<String> {
    let seen = SEEN.read().ok()?;
    followed(cfg, name, &seen)
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
    if !p.follow_loaded {
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
    let mut out = Vec::new();
    for s in seen {
        let Some(model) = s.resident.as_deref() else {
            continue;
        };
        match namers(cfg, &s.base_url, model).count() {
            1 => {}
            0 => out.push(format!(
                "the router at {} has {model:?} loaded, but no [providers.*] entry names it \
                 with that base_url — runs will use the default provider and swap it back out. \
                 Add an entry with model = {model:?}.",
                s.base_url
            )),
            n => out.push(format!(
                "the router at {} has {model:?} loaded, and {n} [providers.*] entries name it \
                 — which one answered would be a guess, so runs keep the default provider. \
                 Leave one.",
                s.base_url
            )),
        }
    }
    out
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
        if let Some(list) = models(&b).await {
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
        // Trimmed from the router on 2026-09-26 (c841aee).
        let body = r#"{"data":[
            {"id":"gemma-4-26b-a4b","status":{"value":"unloaded","args":["llama-server"]}},
            {"id":"qwen3.8-27b","status":{"value":"loaded","args":["llama-server"]}},
            {"id":"broken","status":{"value":"unloaded","failed":true,"exit_code":1}}
        ],"object":"list"}"#;
        let list: ModelList = serde_json::from_str(body).unwrap();
        assert_eq!(resident(&list.data), Some("qwen3.8-27b"));
        assert!(list.data[2].status.failed);
        assert_eq!(list.data[2].status.exit_code, Some(1));
    }
}
