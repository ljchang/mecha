//! Web search.
//!
//! Search is a swappable backend for the same reason models are: the landscape
//! moves, free tiers appear and vanish, and no single provider is right for
//! every query. Exa ranks by meaning, Tavily returns agent-ready extracts, a
//! self-hosted SearXNG keeps queries off other people's servers. All three sit
//! behind [`SearchBackend`].
//!
//! Backends are tried in order and the chain falls through on failure, which is
//! what makes stacking two free tiers a working strategy rather than a hack:
//! run out on the first, the second answers.
//!
//! ## Security
//!
//! Search results are the single largest indirect prompt-injection surface an
//! agent has, and the search *query itself* is an exfiltration channel — the
//! payload fits in `?q=`. So the tool declares both `untrusted_input` and
//! `external_send`, and its output is marked `from_outside`. The trifecta
//! interlock does the rest.

use crate::tool::{Capabilities, Tool, ToolCtx, ToolOutput};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    /// Extract or snippet. Backends differ wildly in how much they return.
    pub snippet: String,
    pub published: Option<String>,
    pub score: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    /// Some backends synthesize an answer. Treated as just another untrusted
    /// string — it was written from the same pages.
    pub answer: Option<String>,
    /// Which backend actually served this, for the trace.
    pub backend: String,
}

/// How much to spend on one query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    /// One cheap round trip. The default, and right for nearly everything.
    Quick,
    /// Multi-hop retrieval. Slower and several times the price — worth it for
    /// a genuine research question, wasted on a lookup.
    Deep,
}

#[async_trait]
pub trait SearchBackend: Send + Sync {
    fn id(&self) -> &str;
    async fn search(&self, query: &str, limit: usize, depth: Depth) -> Result<SearchResponse>;
}

// --------------------------------------------------------------------------
// Exa — https://api.exa.ai/search
// --------------------------------------------------------------------------

pub struct Exa {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl Exa {
    pub fn new(api_key: String, base_url: Option<String>) -> Result<Self> {
        Ok(Exa {
            http: reqwest::Client::builder()
                // `deep-reasoning` is documented at 12-50s, so the timeout has
                // to clear the slow end of that.
                .timeout(std::time::Duration::from_secs(90))
                .build()?,
            api_key,
            base_url: base_url.unwrap_or_else(|| "https://api.exa.ai".into()),
        })
    }
}

#[async_trait]
impl SearchBackend for Exa {
    fn id(&self) -> &str {
        "exa"
    }

    async fn search(&self, query: &str, limit: usize, depth: Depth) -> Result<SearchResponse> {
        // Exa's base price covers 10 results and bills extra beyond that, so
        // don't quietly exceed it.
        let num_results = limit.clamp(1, 10);

        let body = json!({
            "query": query,
            "numResults": num_results,
            "type": match depth {
                Depth::Quick => "auto",
                Depth::Deep => "deep-reasoning",
            },
            // Text extracts, capped: enough to judge relevance without pulling
            // whole pages into context.
            "contents": {"text": {"maxCharacters": 1200}},
        });

        let resp = self
            .http
            .post(format!("{}/search", self.base_url.trim_end_matches('/')))
            .header("x-api-key", &self.api_key)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("exa request failed")?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!(
                "exa {}: {}",
                status,
                text.chars().take(300).collect::<String>()
            );
        }

        let v: Value = serde_json::from_str(&text).context("exa returned malformed JSON")?;
        let results = v
            .get("results")
            .and_then(Value::as_array)
            .map(|rs| {
                rs.iter()
                    .map(|r| SearchResult {
                        title: str_field(r, "title").unwrap_or_else(|| "(untitled)".into()),
                        url: str_field(r, "url").unwrap_or_default(),
                        snippet: str_field(r, "text").unwrap_or_default(),
                        published: str_field(r, "publishedDate"),
                        score: r.get("score").and_then(Value::as_f64),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(SearchResponse {
            results,
            answer: None,
            backend: "exa".into(),
        })
    }
}

// --------------------------------------------------------------------------
// Tavily — https://api.tavily.com/search
// --------------------------------------------------------------------------

pub struct Tavily {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl Tavily {
    pub fn new(api_key: String, base_url: Option<String>) -> Result<Self> {
        Ok(Tavily {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(90))
                .build()?,
            api_key,
            base_url: base_url.unwrap_or_else(|| "https://api.tavily.com".into()),
        })
    }
}

#[async_trait]
impl SearchBackend for Tavily {
    fn id(&self) -> &str {
        "tavily"
    }

    async fn search(&self, query: &str, limit: usize, depth: Depth) -> Result<SearchResponse> {
        let body = json!({
            "query": query,
            "max_results": limit.clamp(1, 20),
            // basic costs 1 credit, advanced 2.
            "search_depth": match depth {
                Depth::Quick => "basic",
                Depth::Deep => "advanced",
            },
            "include_answer": matches!(depth, Depth::Deep),
        });

        let resp = self
            .http
            .post(format!("{}/search", self.base_url.trim_end_matches('/')))
            .bearer_auth(&self.api_key)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .context("tavily request failed")?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!(
                "tavily {}: {}",
                status,
                text.chars().take(300).collect::<String>()
            );
        }

        let v: Value = serde_json::from_str(&text).context("tavily returned malformed JSON")?;
        let results = v
            .get("results")
            .and_then(Value::as_array)
            .map(|rs| {
                rs.iter()
                    .map(|r| SearchResult {
                        title: str_field(r, "title").unwrap_or_else(|| "(untitled)".into()),
                        url: str_field(r, "url").unwrap_or_default(),
                        snippet: str_field(r, "content").unwrap_or_default(),
                        published: None,
                        score: r.get("score").and_then(Value::as_f64),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(SearchResponse {
            results,
            answer: str_field(&v, "answer"),
            backend: "tavily".into(),
        })
    }
}

// --------------------------------------------------------------------------
// SearXNG — a self-hosted metasearch instance
// --------------------------------------------------------------------------

/// Talks to a SearXNG instance's JSON API. No key, no quota, and the query
/// never leaves your network — which for an agent that also reads private data
/// is the only way to stop the *query* being the leak.
pub struct Searxng {
    http: reqwest::Client,
    base_url: String,
}

impl Searxng {
    pub fn new(base_url: String) -> Result<Self> {
        Ok(Searxng {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()?,
            base_url,
        })
    }
}

#[async_trait]
impl SearchBackend for Searxng {
    fn id(&self) -> &str {
        "searxng"
    }

    async fn search(&self, query: &str, limit: usize, _depth: Depth) -> Result<SearchResponse> {
        // SearXNG has no depth control and returns a fixed page size; we
        // truncate client-side rather than pretend otherwise.
        let resp = self
            .http
            .get(format!("{}/search", self.base_url.trim_end_matches('/')))
            .query(&[("q", query), ("format", "json")])
            .send()
            .await
            .context("searxng request failed")?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!(
                "searxng {}: {} (a fresh instance must enable the `json` format in settings.yml)",
                status,
                text.chars().take(200).collect::<String>()
            );
        }

        let v: Value = serde_json::from_str(&text).context("searxng returned malformed JSON")?;
        let results: Vec<SearchResult> = v
            .get("results")
            .and_then(Value::as_array)
            .map(|rs| {
                rs.iter()
                    .take(limit)
                    .map(|r| SearchResult {
                        title: str_field(r, "title").unwrap_or_else(|| "(untitled)".into()),
                        url: str_field(r, "url").unwrap_or_default(),
                        snippet: str_field(r, "content").unwrap_or_default(),
                        published: str_field(r, "publishedDate"),
                        score: r.get("score").and_then(Value::as_f64),
                    })
                    .collect()
            })
            .unwrap_or_default();

        // A SearXNG instance answers 200 with an empty `results` list whether
        // the web has nothing or every engine behind it is rate-limited, and
        // the difference is only in `unresponsive_engines`. Ignoring it made
        // an outage indistinguishable from an answer — measured live, with
        // all four engines reporting `Suspended: too many requests` and
        // `CAPTCHA` while the tool reported no results. So an empty page with
        // an unresponsive engine behind it is a backend *failure*: it falls
        // through to the next backend and, if there is none, says the search
        // broke rather than that the web is silent.
        let unresponsive = unresponsive_engines(&v);
        if results.is_empty() && !unresponsive.is_empty() {
            bail!(
                "searxng asked no working engine — {}",
                unresponsive.join("; ")
            );
        }
        // Partial degradation still answers, but the operator should see it:
        // results thinned to one surviving engine look like a quiet web.
        if !unresponsive.is_empty() {
            tracing::warn!(
                unresponsive = unresponsive.join("; "),
                returned = results.len(),
                "searxng answered with engines missing"
            );
        }

        Ok(SearchResponse {
            results,
            answer: None,
            backend: "searxng".into(),
        })
    }
}

/// `[["brave", "Suspended: too many requests"], ["duckduckgo", "CAPTCHA"]]` —
/// read defensively, because this is a third-party instance's shape and an
/// unexpected one must read as "nothing to report", never panic a search.
fn unresponsive_engines(v: &Value) -> Vec<String> {
    v.get("unresponsive_engines")
        .and_then(Value::as_array)
        .map(|es| {
            es.iter()
                .map(|e| match e.as_array() {
                    Some(pair) => {
                        let name = pair.first().and_then(Value::as_str).unwrap_or("?");
                        match pair.get(1).and_then(Value::as_str) {
                            Some(why) => format!("{name}: {why}"),
                            None => name.to_string(),
                        }
                    }
                    None => e.as_str().unwrap_or("?").to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

// --------------------------------------------------------------------------
// The chain
// --------------------------------------------------------------------------

/// Backends in preference order, with fall-through on failure.
///
/// This is what makes stacking free tiers work: when the first backend returns
/// 429 or 402 because the month's allowance is gone, the next one answers, and
/// the agent never sees a failure.
pub struct SearchChain {
    entries: Vec<ChainEntry>,
}

/// One backend and the one thing the chain needs to know about it beyond how
/// to call it.
pub struct ChainEntry {
    pub backend: Box<dyn SearchBackend>,
    /// Move this backend to the front when the caller asked for [`Depth::Deep`].
    ///
    /// `Depth` used to change only *how* a backend searched, never *which* one
    /// ran, so a research question went to whatever was cheapest and first —
    /// and a paid backend chosen precisely for hard questions was reached only
    /// when the free one came up empty. This is the other half: config says
    /// which backends are worth their price on a hard question, and the chain
    /// puts them first for exactly those.
    ///
    /// It reorders rather than filters, deliberately. A preferred backend that
    /// is rate-limited must still fall through to the free one, and a quick
    /// query must still be able to reach the paid backend as a *fallback* when
    /// the free one is down — which is the arrangement that kept working
    /// through a total searxng outage.
    pub prefer_deep: bool,
}

impl SearchChain {
    pub fn new(backends: Vec<Box<dyn SearchBackend>>) -> Self {
        SearchChain {
            entries: backends
                .into_iter()
                .map(|backend| ChainEntry {
                    backend,
                    prefer_deep: false,
                })
                .collect(),
        }
    }

    pub fn with_entries(entries: Vec<ChainEntry>) -> Self {
        SearchChain { entries }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn ids(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.backend.id()).collect()
    }

    /// The order this depth should try backends in. A stable partition, so
    /// config order still decides everything within each group — the only
    /// thing depth moves is which group goes first.
    fn order_for(&self, depth: Depth) -> Vec<&ChainEntry> {
        match depth {
            Depth::Quick => self.entries.iter().collect(),
            Depth::Deep => self
                .entries
                .iter()
                .filter(|e| e.prefer_deep)
                .chain(self.entries.iter().filter(|e| !e.prefer_deep))
                .collect(),
        }
    }

    pub async fn search(&self, query: &str, limit: usize, depth: Depth) -> Result<SearchResponse> {
        let mut failures = Vec::new();
        // A backend that answered, even with nothing, is the difference
        // between "the web does not have this" and "the search is broken",
        // and only the second is an error. Exhausting the chain on empties
        // used to report the first as the second, which is worse than
        // useless: a model told its tools are broken rewords and retries —
        // eight times in one recorded run — where a model told there are no
        // results moves on. `bail!` is reserved for the case where nothing
        // answered at all, which is the one the model genuinely cannot route
        // around.
        let mut empty_from: Option<String> = None;

        for entry in self.order_for(depth) {
            let backend = &entry.backend;
            match backend.search(query, limit, depth).await {
                // A backend that answers with nothing is not an error, but it
                // is worth trying the next one before giving up.
                Ok(r) if r.results.is_empty() && r.answer.is_none() => {
                    failures.push(format!("{}: no results", backend.id()));
                    empty_from.get_or_insert_with(|| backend.id().to_string());
                }
                Ok(r) => return Ok(r),
                Err(e) => {
                    tracing::warn!(backend = backend.id(), error = %e, "search backend failed");
                    failures.push(format!("{}: {e}", backend.id()));
                }
            }
        }

        if let Some(backend) = empty_from {
            // Whichever backends did break are in the operator's log above;
            // the model gets the answer the working ones gave.
            return Ok(SearchResponse {
                backend,
                ..Default::default()
            });
        }

        bail!("every search backend failed — {}", failures.join("; "))
    }
}

// --------------------------------------------------------------------------
// The tool
// --------------------------------------------------------------------------

pub struct WebSearch {
    chain: Arc<SearchChain>,
}

impl WebSearch {
    pub fn new(chain: Arc<SearchChain>) -> Self {
        WebSearch { chain }
    }
}

#[async_trait]
impl Tool for WebSearch {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web. Returns titles, URLs, and extracts — use http_fetch afterwards if \
         you need a full page. Set depth to \"deep\" only for genuine research questions \
         that need several hops; it is much slower and costs more, and a plain lookup does \
         not need it."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "What to search for. Write it as a search query, not a question."
                },
                "limit": {
                    "type": "integer",
                    "description": "How many results to return. Default 8."
                },
                "depth": {
                    "type": "string",
                    "enum": ["quick", "deep"],
                    "description": "Default \"quick\"."
                }
            },
            "required": ["query"]
        })
    }

    fn read_only(&self) -> bool {
        // Changes nothing of yours — but see `capabilities`: the query itself
        // leaves the machine.
        true
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default().untrusted().sends()
    }

    async fn call(&self, input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
        let Some(query) = input.get("query").and_then(Value::as_str) else {
            return Ok(ToolOutput::err("missing required string argument `query`"));
        };
        let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(8) as usize;
        let depth = match input.get("depth").and_then(Value::as_str) {
            Some("deep") => Depth::Deep,
            _ => Depth::Quick,
        };

        let response = match self.chain.search(query, limit, depth).await {
            Ok(r) => r,
            Err(e) => return Ok(ToolOutput::err(format!("{e:#}"))),
        };

        if response.results.is_empty() && response.answer.is_none() {
            return Ok(ToolOutput::ok(format!("no results for {query:?}")).from_outside());
        }

        let mut out = String::new();
        if let Some(answer) = &response.answer {
            out.push_str(&format!("Synthesized answer: {answer}\n\n"));
        }
        for (i, r) in response.results.iter().enumerate() {
            out.push_str(&format!("{}. {}\n   {}\n", i + 1, r.title, r.url));
            if let Some(date) = &r.published {
                out.push_str(&format!("   published: {date}\n"));
            }
            if !r.snippet.is_empty() {
                let snippet: String = r.snippet.chars().take(700).collect();
                out.push_str(&format!("   {}\n", snippet.replace('\n', " ")));
            }
            out.push('\n');
        }
        out.push_str(&format!("(via {})", response.backend));

        // Everything above was written by strangers.
        Ok(ToolOutput::ok(out).from_outside())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Stub {
        id: &'static str,
        calls: Arc<AtomicUsize>,
        behaviour: Behaviour,
    }

    enum Behaviour {
        Fail,
        Empty,
        One,
    }

    #[async_trait]
    impl SearchBackend for Stub {
        fn id(&self) -> &str {
            self.id
        }
        async fn search(&self, _q: &str, _l: usize, _d: Depth) -> Result<SearchResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.behaviour {
                Behaviour::Fail => bail!("quota exhausted"),
                Behaviour::Empty => Ok(SearchResponse {
                    backend: self.id.into(),
                    ..Default::default()
                }),
                Behaviour::One => Ok(SearchResponse {
                    results: vec![SearchResult {
                        title: "A page".into(),
                        url: "https://example.com".into(),
                        snippet: "words".into(),
                        published: None,
                        score: None,
                    }],
                    answer: None,
                    backend: self.id.into(),
                }),
            }
        }
    }

    fn stub(id: &'static str, behaviour: Behaviour) -> (Box<dyn SearchBackend>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Box::new(Stub {
                id,
                calls: Arc::clone(&calls),
                behaviour,
            }),
            calls,
        )
    }

    /// The measured case: every engine behind the instance suspended or
    /// CAPTCHA'd, `results: []`, HTTP 200. Without reading
    /// `unresponsive_engines` this is byte-identical to a genuine no-match,
    /// so a total search outage would report as "the web has nothing" — the
    /// silently-degrading shape, arriving through a third party's JSON.
    #[test]
    fn an_instance_with_every_engine_suspended_is_a_failure_not_an_empty_web() {
        let v: Value = serde_json::json!({
            "results": [],
            "unresponsive_engines": [
                ["brave", "Suspended: too many requests"],
                ["duckduckgo", "CAPTCHA"],
            ],
        });
        let reasons = unresponsive_engines(&v);
        assert_eq!(
            reasons,
            vec![
                "brave: Suspended: too many requests".to_string(),
                "duckduckgo: CAPTCHA".to_string()
            ]
        );
    }

    /// And the honest empty: engines answered, the web had nothing. Nothing
    /// to report, so the chain is free to call it an answer.
    #[test]
    fn an_empty_page_with_every_engine_healthy_reports_nothing_unresponsive() {
        let v: Value = serde_json::json!({ "results": [], "unresponsive_engines": [] });
        assert!(unresponsive_engines(&v).is_empty());
    }

    /// A third-party instance is free to change this shape; an unexpected one
    /// must read as "nothing to report" rather than panicking a search.
    #[test]
    fn an_unexpected_unresponsive_shape_is_read_defensively() {
        assert!(unresponsive_engines(&serde_json::json!({})).is_empty());
        assert!(
            unresponsive_engines(&serde_json::json!({"unresponsive_engines": "brave"})).is_empty()
        );
        assert_eq!(
            unresponsive_engines(&serde_json::json!({"unresponsive_engines": ["brave", ["ddg"]]})),
            vec!["brave".to_string(), "ddg".to_string()]
        );
    }

    /// The recorded failure: one configured backend, a query the web has no
    /// answer for, and the model told `every search backend failed` — which
    /// it read as broken infrastructure and answered by rewording the query
    /// eight times. "Nothing found" is an answer and must arrive as one.
    #[tokio::test]
    async fn an_exhausted_chain_of_empties_is_an_answer_not_a_failure() {
        let (only, calls) = stub("searxng", Behaviour::Empty);
        let chain = SearchChain::new(vec![only]);

        let r = chain.search("q", 5, Depth::Quick).await.unwrap();
        assert!(r.results.is_empty() && r.answer.is_none());
        assert_eq!(r.backend, "searxng");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// A broken backend beside an empty one still yields the empty one's
    /// answer: the breakage is the operator's to see in the log, and hiding
    /// a real "nothing found" behind it tells the model to retry.
    #[tokio::test]
    async fn one_broken_backend_does_not_hide_anothers_empty_answer() {
        let (first, _) = stub("exa", Behaviour::Fail);
        let (second, _) = stub("searxng", Behaviour::Empty);
        let chain = SearchChain::new(vec![first, second]);

        let r = chain.search("q", 5, Depth::Quick).await.unwrap();
        assert_eq!(r.backend, "searxng");
        assert!(r.results.is_empty());
    }

    /// And the case `bail!` is reserved for: nothing answered at all.
    #[tokio::test]
    async fn a_chain_where_nothing_answered_is_still_an_error() {
        let (first, _) = stub("exa", Behaviour::Fail);
        let (second, _) = stub("tavily", Behaviour::Fail);
        let chain = SearchChain::new(vec![first, second]);

        let e = chain.search("q", 5, Depth::Quick).await.unwrap_err();
        assert!(format!("{e:#}").contains("every search backend failed"));
    }

    fn entry(id: &'static str, behaviour: Behaviour, prefer_deep: bool) -> ChainEntry {
        ChainEntry {
            backend: stub(id, behaviour).0,
            prefer_deep,
        }
    }

    /// An ordinary lookup takes config order, so the free backend stays the
    /// head and the paid one is never reached while it is answering.
    #[tokio::test]
    async fn a_quick_search_keeps_config_order() {
        let chain = SearchChain::with_entries(vec![
            entry("searxng", Behaviour::One, false),
            entry("exa", Behaviour::One, true),
        ]);
        let r = chain.search("q", 5, Depth::Quick).await.unwrap();
        assert_eq!(r.backend, "searxng");
    }

    /// A research question goes to the backend that was configured for one,
    /// even though it sits second. This is the half `Depth` was missing: it
    /// chose how a backend searched and never which one ran.
    #[tokio::test]
    async fn a_deep_search_promotes_the_preferred_backend() {
        let chain = SearchChain::with_entries(vec![
            entry("searxng", Behaviour::One, false),
            entry("exa", Behaviour::One, true),
        ]);
        let r = chain.search("q", 5, Depth::Deep).await.unwrap();
        assert_eq!(r.backend, "exa");
    }

    /// Promotion reorders and never filters, in both directions — otherwise a
    /// rate-limited preferred backend would take a deep query down with it,
    /// and a quick query could not reach the paid backend during the free
    /// one's outage, which is the arrangement that survived a real searxng
    /// blackout.
    #[tokio::test]
    async fn every_backend_stays_reachable_at_either_depth() {
        let deep = SearchChain::with_entries(vec![
            entry("searxng", Behaviour::One, false),
            entry("exa", Behaviour::Fail, true),
        ]);
        assert_eq!(
            deep.search("q", 5, Depth::Deep).await.unwrap().backend,
            "searxng",
            "a broken preferred backend must fall through, not fail the query"
        );

        let quick = SearchChain::with_entries(vec![
            entry("searxng", Behaviour::Fail, false),
            entry("exa", Behaviour::One, true),
        ]);
        assert_eq!(
            quick.search("q", 5, Depth::Quick).await.unwrap().backend,
            "exa",
            "a quick query must still reach the paid backend when the free one is down"
        );
    }

    /// Config order still decides within each group: promotion moves a group,
    /// not an individual backend past its peers.
    #[tokio::test]
    async fn promotion_is_a_stable_partition() {
        let chain = SearchChain::with_entries(vec![
            entry("free-a", Behaviour::Empty, false),
            entry("paid-a", Behaviour::Empty, true),
            entry("paid-b", Behaviour::One, true),
        ]);
        // paid-a and paid-b both promote, and paid-a still precedes paid-b.
        let r = chain.search("q", 5, Depth::Deep).await.unwrap();
        assert_eq!(r.backend, "paid-b");
    }

    #[tokio::test]
    async fn a_failed_backend_falls_through_to_the_next() {
        let (first, first_calls) = stub("exa", Behaviour::Fail);
        let (second, second_calls) = stub("tavily", Behaviour::One);
        let chain = SearchChain::new(vec![first, second]);

        let r = chain.search("q", 5, Depth::Quick).await.unwrap();
        assert_eq!(r.backend, "tavily");
        assert_eq!(first_calls.load(Ordering::SeqCst), 1);
        assert_eq!(second_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_empty_result_set_also_falls_through() {
        let (first, _) = stub("exa", Behaviour::Empty);
        let (second, _) = stub("tavily", Behaviour::One);
        let chain = SearchChain::new(vec![first, second]);
        assert_eq!(
            chain.search("q", 5, Depth::Quick).await.unwrap().backend,
            "tavily"
        );
    }

    #[tokio::test]
    async fn the_first_working_backend_wins_and_the_rest_are_not_called() {
        let (first, first_calls) = stub("exa", Behaviour::One);
        let (second, second_calls) = stub("tavily", Behaviour::One);
        let chain = SearchChain::new(vec![first, second]);

        assert_eq!(
            chain.search("q", 5, Depth::Quick).await.unwrap().backend,
            "exa"
        );
        assert_eq!(first_calls.load(Ordering::SeqCst), 1);
        assert_eq!(second_calls.load(Ordering::SeqCst), 0, "no wasted quota");
    }

    #[tokio::test]
    async fn all_backends_failing_reports_every_reason() {
        let (first, _) = stub("exa", Behaviour::Fail);
        let (second, _) = stub("tavily", Behaviour::Fail);
        let chain = SearchChain::new(vec![first, second]);

        let err = chain
            .search("q", 5, Depth::Quick)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("exa"), "{err}");
        assert!(err.contains("tavily"), "{err}");
    }

    #[tokio::test]
    async fn results_are_marked_as_coming_from_outside() {
        let (backend, _) = stub("exa", Behaviour::One);
        let tool = WebSearch::new(Arc::new(SearchChain::new(vec![backend])));

        let out = tool
            .call(json!({"query": "rust"}), &ToolCtx::default())
            .await
            .unwrap();
        assert!(
            out.external,
            "search output must taint the conversation as untrusted"
        );
        assert!(out.content.contains("https://example.com"));
        assert!(out.content.contains("(via exa)"));
    }

    #[test]
    fn the_search_tool_declares_both_trifecta_legs_it_touches() {
        let tool = WebSearch::new(Arc::new(SearchChain::new(Vec::new())));
        let caps = tool.capabilities();
        assert!(caps.untrusted_input, "results are attacker-influenced");
        assert!(caps.external_send, "the query itself leaves the machine");
    }
}
