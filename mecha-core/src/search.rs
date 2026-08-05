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
        let results = v
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

        Ok(SearchResponse {
            results,
            answer: None,
            backend: "searxng".into(),
        })
    }
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
    backends: Vec<Box<dyn SearchBackend>>,
}

impl SearchChain {
    pub fn new(backends: Vec<Box<dyn SearchBackend>>) -> Self {
        SearchChain { backends }
    }

    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }

    pub fn ids(&self) -> Vec<&str> {
        self.backends.iter().map(|b| b.id()).collect()
    }

    pub async fn search(&self, query: &str, limit: usize, depth: Depth) -> Result<SearchResponse> {
        let mut failures = Vec::new();

        for backend in &self.backends {
            match backend.search(query, limit, depth).await {
                // A backend that answers with nothing is not an error, but it
                // is worth trying the next one before giving up.
                Ok(r) if r.results.is_empty() && r.answer.is_none() => {
                    failures.push(format!("{}: no results", backend.id()));
                }
                Ok(r) => return Ok(r),
                Err(e) => {
                    tracing::warn!(backend = backend.id(), error = %e, "search backend failed");
                    failures.push(format!("{}: {e}", backend.id()));
                }
            }
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
