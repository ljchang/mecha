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
//! payload fits in `?q=`. So the tool declares `untrusted_input`, marks its
//! output `from_outside`, and declares egress.
//!
//! But **which** egress, and that is the whole of `docs/EGRESS-DESIGN.md`.
//! [`WebSearch::input_schema`] has three properties — `query`, `limit`,
//! `depth` — and no destination field. The model fills the channel; it cannot
//! choose who reads it, because the recipients are whatever `[[search]]` names.
//! That is [`Egress::Blind`], and the trifecta interlock (which exists to stop
//! an injection *directing* a send) leaves it alone, while the leak guard
//! `block_sends_after_private` (which exists to stop private data reaching a
//! third party at all) still refuses it.
//!
//! Blind is earned per backend and per depth, never assumed: see
//! [`SearchBackend::egress`], whose default is the conservative class. An
//! armed conversation is served only by the blind backends, at quick depth —
//! [`SearchChain::search_blind`].

use crate::tool::{Capabilities, DenialCause, Egress, Tool, ToolCtx, ToolOutput};
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

    /// Who receives this backend's query at this depth — see [`Egress`].
    ///
    /// [`Egress::Blind`] is the claim that the backend treats the query as
    /// *search terms* and nothing else: it will not dereference a URL written
    /// into the query, and it will not fetch a page the query names. Then an
    /// injection can fill the channel and still has no way to read it back,
    /// because the recipients are the ones `[[search]]` names.
    ///
    /// **The default is [`Egress::Chosen`], and it is load-bearing.** A
    /// backend added later is not blind until somebody reads its API and says
    /// so in a diff — the same fail-closed shape as unknown taint. The three
    /// classifications this repo ships were made on 2026-09-17 against vendor
    /// documentation, which is weaker evidence than a test and is recorded as
    /// such in the design document; what protects against a vendor adding the
    /// behaviour later is this default plus the fact that mecha never sends
    /// `livecrawl`, `maxAgeHours` or `include_raw_content`.
    fn egress(&self, depth: Depth) -> Egress {
        let _ = depth;
        Egress::Chosen
    }
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

    /// Blind at quick depth, not at deep.
    ///
    /// `Depth::Quick` sends `"type": "auto"` with `livecrawl: "never"` — an
    /// index query, and an explicit refusal to fetch anything not already
    /// indexed. Both halves matter: the type keeps it a lookup, and the
    /// `livecrawl` says so in a parameter rather than by omission. `Depth::Deep` sends `"type": "deep-reasoning"`,
    /// documented as multi-step agentic research that *fetches pages chosen
    /// during the research*; the query steers that choice, so a query naming a
    /// host is a query that may cause a request to it. That is a destination
    /// the model picked, which is exactly what `Blind` promises it cannot.
    fn egress(&self, depth: Depth) -> Egress {
        match depth {
            Depth::Quick => Egress::Blind,
            Depth::Deep => Egress::Chosen,
        }
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
            //
            // `livecrawl: "never"` is what makes `egress(Quick) == Blind` a
            // property of this file rather than of Exa's defaults. Asking for
            // page contents is what raises the question at all — omitting the
            // parameter would hand "does a query cause a fetch" to a default
            // the vendor can change with nothing here changing and no diff to
            // review, which is the absence-as-mitigation this PR's own
            // `Chosen` trait default exists to refuse. Named in review,
            // 2026-09-17.
            "contents": {"text": {"maxCharacters": 1200}, "livecrawl": "never"},
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

    /// Blind at both depths. `search_depth` selects how hard Tavily ranks and
    /// how much of each source it extracts, not whether it follows the query
    /// somewhere: crawling lives in a separate Crawl API, which takes a base
    /// URL and which mecha does not call. The documented `/search` parameter
    /// surface has no query-driven fetch.
    fn egress(&self, _depth: Depth) -> Egress {
        Egress::Blind
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

/// Talks to a SearXNG instance's JSON API. No key and no quota — and the
/// property that matters for an agent that also reads private data: **you
/// choose the recipients, and no text the model writes can change them.**
///
/// An earlier comment here claimed the query "never leaves your network".
/// That is over-generous and was corrected on 2026-09-17: a self-hosted
/// instance forwards `q` to whichever upstream engines its `settings.yml`
/// enables, so the query does reach them. The weaker claim is the true one and
/// is all [`Egress::Blind`] ever needed. Anyone who wants the stronger
/// property wants `block_sends_after_private = true`.
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

    /// Blind at both depths: `q` goes to the engines named in the instance's
    /// own `settings.yml` and nowhere else, and there is no path by which a
    /// URL written into the query becomes a request to it.
    fn egress(&self, _depth: Depth) -> Egress {
        Egress::Blind
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

    /// The backends whose destination the model cannot choose, in config
    /// order. Quick depth, because that is the only depth
    /// [`search_blind`](Self::search_blind) runs at — a backend blind at quick
    /// and chosen at deep (Exa) belongs here, and is served quick.
    fn blind_entries(&self) -> Vec<&ChainEntry> {
        self.entries
            .iter()
            .filter(|e| e.backend.egress(Depth::Quick) == Egress::Blind)
            .collect()
    }

    /// Is there any backend an armed conversation could still be served by?
    ///
    /// What [`WebSearch::capabilities`] declares turns on this: with none, the
    /// tool has no blind path at all and must declare the conservative class
    /// so the interlock refuses it with a remedy, rather than promising a
    /// route that does not exist.
    pub fn has_blind_backend(&self) -> bool {
        !self.blind_entries().is_empty()
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
        self.run(self.order_for(depth), query, limit, depth).await
    }

    /// Search using only the backends whose destination the *operator* fixed.
    ///
    /// What an armed conversation gets. Forced to [`Depth::Quick`] because
    /// that is the depth [`blind_entries`](Self::blind_entries) classified —
    /// Exa's deep mode is agentic research that fetches pages the query can
    /// steer it towards, which is not blind whatever the backend is blind at
    /// otherwise.
    ///
    /// Degrading beats refusing, and the reason is measured: on 2026-08-21
    /// every general engine behind the local SearXNG refused this box's IP,
    /// which is why paid backends are in the chain at all. "Delete your paid
    /// backends to keep searching while armed" would trade one refusal for
    /// another.
    pub async fn search_blind(&self, query: &str, limit: usize) -> Result<SearchResponse> {
        let order = self.blind_entries();
        if order.is_empty() {
            bail!(
                "this conversation holds both private data and third-party content, so \
                 search is served only by backends whose destination is fixed by your \
                 config — and none is configured"
            );
        }
        self.run(order, query, limit, Depth::Quick).await
    }

    async fn run(
        &self,
        order: Vec<&ChainEntry>,
        query: &str,
        limit: usize,
        depth: Depth,
    ) -> Result<SearchResponse> {
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

        for entry in order {
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

/// Every URL a search handed this process, under a handle the model can name.
///
/// This is what makes [`WebOpen`] blind (`docs/PROVENANCE-DESIGN.md` §4). The
/// model never writes a URL for it. It names a handle `web_search` printed
/// beside a result, and the URL is the one the search backend returned,
/// looked up here. An argument that is not a handle this ledger issued opens
/// nothing, so there is no field a secret can be written into. What leaks is
/// *which* result was chosen: `log2(N)` bits per call, to whoever serves the
/// page.
///
/// Process-wide rather than per conversation, on purpose. A handle issued to
/// one conversation opens only a URL some search already returned, and each
/// search's token is random, so a handle has to be *held* to be used: one
/// run cannot derive another's. The random epoch in every handle means a
/// handle from before a restart is refused rather than silently naming a
/// different result.
pub struct ResultLedger {
    state: std::sync::Mutex<LedgerState>,
}

struct LedgerState {
    epoch: String,
    /// Live search tokens, oldest first, so a new one never reuses a token
    /// whose handles are still held. Capped like the handles: a token more
    /// than `LEDGER_CAP` searches old has had every handle evicted.
    tokens: std::collections::VecDeque<String>,
    order: std::collections::VecDeque<String>,
    urls: std::collections::HashMap<String, String>,
}

/// How many handles the ledger keeps before forgetting the oldest. A
/// forgotten handle is refused with the same words as an invented one, and
/// the remedy (search again) is the same.
const LEDGER_CAP: usize = 4096;

impl ResultLedger {
    pub fn new() -> Self {
        let epoch: String = uuid::Uuid::new_v4().simple().to_string()[..3].to_string();
        ResultLedger {
            state: std::sync::Mutex::new(LedgerState {
                epoch,
                tokens: std::collections::VecDeque::new(),
                order: std::collections::VecDeque::new(),
                urls: std::collections::HashMap::new(),
            }),
        }
    }

    /// Issue one handle per URL of one search, in order. An empty URL gets
    /// no handle, since there is nothing to open.
    fn record(&self, urls: &[&str]) -> Vec<Option<String>> {
        let Ok(mut st) = self.state.lock() else {
            return urls.iter().map(|_| None).collect();
        };
        // A random token per search, not a counter: a handle has to be *held*
        // to be used. With a counter, one search taught a conversation the
        // epoch and every other run's handles followed by arithmetic —
        // batch items, eval cases, a `/clear`ed chat, a front-door run all
        // share this ledger (found in review of #276). Retried on the
        // vanishing chance of a collision with a token still held.
        let search = loop {
            let t = uuid::Uuid::new_v4().simple().to_string()[..6].to_string();
            if !st.tokens.contains(&t) {
                break t;
            }
        };
        st.tokens.push_back(search.clone());
        while st.tokens.len() > LEDGER_CAP {
            st.tokens.pop_front();
        }
        let mut handles = Vec::with_capacity(urls.len());
        for (i, url) in urls.iter().enumerate() {
            if url.is_empty() {
                handles.push(None);
                continue;
            }
            let handle = format!("{}-{search}.{}", st.epoch, i + 1);
            st.urls.insert(handle.clone(), url.to_string());
            st.order.push_back(handle.clone());
            while st.order.len() > LEDGER_CAP {
                if let Some(old) = st.order.pop_front() {
                    st.urls.remove(&old);
                }
            }
            handles.push(Some(handle));
        }
        handles
    }

    /// Record one URL as a search would, for tests outside this module that
    /// need a real handle — the interlock test in `agent.rs`.
    #[cfg(test)]
    pub(crate) fn record_for_test(&self, url: &str) -> String {
        self.record(&[url])[0]
            .clone()
            .expect("a non-empty url gets a handle")
    }

    /// The URL a handle names, if this ledger issued it and still holds it.
    pub fn resolve(&self, handle: &str) -> Option<String> {
        self.state.lock().ok()?.urls.get(handle.trim()).cloned()
    }
}

impl Default for ResultLedger {
    fn default() -> Self {
        Self::new()
    }
}

pub struct WebSearch {
    chain: Arc<SearchChain>,
    /// Where result handles are recorded for [`WebOpen`]. `None` prints no
    /// handles, which is right exactly when no `web_open` is registered.
    ledger: Option<Arc<ResultLedger>>,
}

impl WebSearch {
    pub fn new(chain: Arc<SearchChain>) -> Self {
        WebSearch {
            chain,
            ledger: None,
        }
    }

    /// Print a handle beside every result, recorded in `ledger`, so that a
    /// [`WebOpen`] sharing the ledger can open it.
    pub fn with_ledger(mut self, ledger: Arc<ResultLedger>) -> Self {
        self.ledger = Some(ledger);
        self
    }
}

#[async_trait]
impl Tool for WebSearch {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        if self.ledger.is_some() {
            "Search the web. Returns titles, URLs, and extracts, each result with a handle \
             in brackets. To read a full page, pass that handle to web_open if you have it, \
             or the URL to http_fetch otherwise. Set depth to \"deep\" only for genuine \
             research questions that need several hops; it is much slower and costs more, \
             and a plain lookup does not need it."
        } else {
            "Search the web. Returns titles, URLs, and extracts — use http_fetch afterwards if \
             you need a full page. Set depth to \"deep\" only for genuine research questions \
             that need several hops; it is much slower and costs more, and a plain lookup does \
             not need it."
        }
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

    /// [`Egress::Blind`] — the input schema above has no destination field,
    /// so the query reaches whatever `[[search]]` names and nothing else.
    ///
    /// Except with no blind backend configured, where there is no armed path
    /// at all and the honest declaration is the conservative one, so the
    /// interlock refuses with [`denial_remedy`](Tool::denial_remedy) naming
    /// the fix.
    ///
    /// **The invariant this rests on, because it is not locally obvious:** no
    /// *control* consults the `Blind`/`Chosen` distinction in any state where
    /// [`call`](Tool::call) could reach a non-blind backend. There are two
    /// such states and both are covered — while the conversation is clean the
    /// full chain is reachable, and under `trifecta = "allow"` it is reachable
    /// while armed — because the interlock requires `trifecta_armed()` *and*
    /// is waived by `allow`, while the leak guard treats both classes alike.
    ///
    /// What does read the class in those states is *reader-facing*:
    /// `mecha tools --json` prints `"egress": "blind"` and the TUI tool list
    /// says `sends(fixed)`. Under `allow` that describes the declaration
    /// rather than the run, which is why the TUI gloss says what the class
    /// means and not what the tool will unconditionally do, and why
    /// `EGRESS-DESIGN.md` §6 lists it as a residual. Flagged in review,
    /// 2026-09-17. Change either of those (the table in
    /// `docs/EGRESS-DESIGN.md` §D3) and this declaration must be revisited.
    /// Guarded from both sides:
    /// `the_declared_class_tracks_whether_a_blind_route_exists` below, and
    /// `agent::tests::a_clean_conversation_does_not_consult_the_egress_class`.
    fn capabilities(&self) -> Capabilities {
        let caps = Capabilities::default().untrusted();
        if self.chain.has_blind_backend() {
            caps.sends_blind()
        } else {
            caps.sends()
        }
    }

    fn denial_remedy(&self, cause: DenialCause) -> Option<String> {
        match cause {
            // Adding a blind backend is the answer to the interlock and only
            // to the interlock. Under the leak guard it is worse than silence:
            // the operator adds SearXNG, `capabilities()` flips to `Blind`,
            // and the leak guard refuses `Blind` too — a second refusal
            // reached by following the exit. The honest answer there is the
            // refusal's own text, which names the setting that fired.
            DenialCause::Leak => None,
            DenialCause::Injection => (!self.chain.has_blind_backend()).then(|| {
                "Web search is refused here only because every configured backend is one \
                 the model could point somewhere. Add a backend whose destination your \
                 config fixes — `[[search]]` with `kind = \"searxng\"` — and search keeps \
                 working in conversations that hold private data."
                    .into()
            }),
        }
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let Some(query) = input.get("query").and_then(Value::as_str) else {
            return Ok(ToolOutput::err("missing required string argument `query`"));
        };
        let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(8) as usize;
        let depth = match input.get("depth").and_then(Value::as_str) {
            Some("deep") => Depth::Deep,
            _ => Depth::Quick,
        };

        // Fail closed on `None`, per `ToolCtx::taint`'s own contract: a
        // subagent's context, or any run wired outside the loop, must not pass
        // as a clean sender by omission. The cost of being wrong here is a
        // quick searxng search instead of a deep exa one.
        //
        // `trifecta = "allow"` is the operator's written waiver of the
        // injection interlock, and the switch table calls it "waives the
        // injection interlock entirely". Narrowing anyway would make that
        // false and would silently remove deep search from an operator who
        // had explicitly opted out — a capability lost with no configuration
        // able to restore it. So the waiver reaches here too.
        //
        // `"ask"` deliberately does not: a blind call raises no escalation
        // (`injection_risk` is false for it), so there is no human yes to
        // widen on, and quietly running the full chain would be a widening
        // nobody asked for. Degrading is the answer `ask` gets — search keeps
        // working with no modal at all. It is not free: an `ask` operator
        // used to get a prompt and, on approval, a full-depth search, and now
        // gets a quiet quick one. `EGRESS-DESIGN.md` §6 item 5 records that
        // rather than counting it purely as a gain.
        // `has_blind_backend()` is the third term, and it is what keeps this
        // in agreement with `capabilities()` *by construction*: the tool
        // declares `Blind` exactly when a blind route exists, so degrading
        // exactly when one exists makes `search_blind`'s `bail!` unreachable
        // from here rather than latently reachable.
        //
        // Without it, the `"ask"` reasoning above quietly stopped holding in
        // the one branch `capabilities()` added for the other case. With no
        // blind backend the tool declares `Chosen`, so `injection_risk` *is*
        // true, an escalation *is* raised, and a human *does* say yes — and
        // that yes landed on `bail!("… none is configured")` while the full
        // chain would have answered. `ask` was strictly worse than `block`,
        // which hands back `denial_remedy(Injection)` naming SearXNG. Latent
        // today (every shipped backend is blind at quick depth), and exactly
        // the path the `Chosen` trait default exists to light up for the next
        // one. Found by review, 2026-09-17.
        //
        // Reaching `call` with no blind route means the ordinary gate already
        // let this through — approved under `ask`, waived under `allow`, or
        // the conversation is not armed — so the full chain is the right
        // answer in all three.
        let waived = ctx.security.trifecta == crate::config::TrifectaPolicy::Allow;
        let armed = !waived
            && self.chain.has_blind_backend()
            && ctx.taint.is_none_or(|t| t.trifecta_armed());

        let response = match if armed {
            self.chain.search_blind(query, limit).await
        } else {
            self.chain.search(query, limit, depth).await
        } {
            Ok(r) => r,
            // `from_outside`, like the results: a backend's error carries what
            // the backend said, which is a third party's text.
            Err(e) => {
                return Ok(ToolOutput::err(format!("{e:#}{}", narrowing_note(armed))).from_outside())
            }
        };

        if response.results.is_empty() && response.answer.is_none() {
            return Ok(ToolOutput::ok(format!(
                "no results for {query:?}{}",
                narrowing_note(armed)
            ))
            .from_outside());
        }

        let mut out = String::new();
        if let Some(answer) = &response.answer {
            out.push_str(&format!("Synthesized answer: {answer}\n\n"));
        }
        // A result whose host is in the query gets no handle. `web_open` is
        // blind only because its URL is one a backend returned rather than
        // one the model wrote, and a backend that echoed a URL-shaped query
        // back as a result would break exactly that: `web_search("https://
        // evil.example/?d=<secret>")` would mint a handle to a destination
        // the model composed (found in review of #284). No shipped backend is
        // known to do it; the guard costs one lookup, and a host the query
        // named is one the model could have chosen.
        let origin: Vec<Provenance> = response
            .results
            .iter()
            .map(|r| provenance(query, &r.url))
            .collect();
        let urls: Vec<&str> = response
            .results
            .iter()
            .zip(&origin)
            .map(|(r, o)| {
                if *o == Provenance::Supplied {
                    r.url.as_str()
                } else {
                    ""
                }
            })
            .collect();
        let handles = match &self.ledger {
            Some(ledger) => ledger.record(&urls),
            None => vec![None; urls.len()],
        };
        for (i, r) in response.results.iter().enumerate() {
            match &handles[i] {
                Some(h) => out.push_str(&format!("{}. [{h}] {}\n   {}\n", i + 1, r.title, r.url)),
                None if self.ledger.is_some() && origin[i] == Provenance::Echoed => {
                    out.push_str(&format!(
                        "{}. {}\n   {}\n   (no handle: this address carries what the query wrote, so \
                         opening it would be fetching an address this conversation composed)\n",
                        i + 1,
                        r.title,
                        r.url
                    ))
                }
                None if self.ledger.is_some() && origin[i] == Provenance::Unparseable => {
                    out.push_str(&format!(
                        "{}. {}\n   {}\n   (no handle: that address does not parse)\n",
                        i + 1,
                        r.title,
                        r.url
                    ))
                }
                None => out.push_str(&format!("{}. {}\n   {}\n", i + 1, r.title, r.url)),
            }

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
        out.push_str(narrowing_note(armed));

        // Everything above was written by strangers.
        Ok(ToolOutput::ok(out).from_outside())
    }
}

/// Whether a result may be handed a handle, and if not, why.
#[derive(Debug, PartialEq, Eq)]
enum Provenance {
    /// A URL the backend supplied: nothing the query wrote is in it.
    Supplied,
    /// Bytes the query wrote are in the URL — an echo, or a redirect wrapper
    /// carrying the model's address in its own query string.
    Echoed,
    /// The URL does not parse, or has no host, so nothing can vouch for it.
    Unparseable,
}

/// Did the model write this result's URL?
///
/// `web_open` is blind only because a backend, not the model, supplied the
/// URL (`docs/PROVENANCE-DESIGN.md` §4). The question is whether the result
/// URL *carries bytes the query wrote*. That covers a backend that echoes a
/// URL-shaped query verbatim, and the more common shape, a backend that wraps
/// results in its own redirect (`search.example/r?u=https%3A%2F%2Fevil…`).
/// Both sides are normalised the same way before comparing. The result is
/// percent-decoded twice, and the query's URL-shaped tokens go through the
/// same parser. So a percent-encoded or Unicode spelling of the model's
/// address matches the form the backend returned (found in review of #286).
///
/// A query that merely *mentions* a domain (`docs.rust-lang.org tracing`)
/// keeps the handles on that domain's real pages. Only a result that is
/// nothing but the host the query named is refused, because a subdomain can
/// carry data too.
fn provenance(query: &str, url: &str) -> Provenance {
    let Some(parsed) = reqwest::Url::parse(url)
        .ok()
        .filter(|u| u.host_str().is_some())
    else {
        return Provenance::Unparseable;
    };
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    let decoded = percent_decode(&percent_decode(url)).to_lowercase();
    let bare = parsed.path().trim_end_matches('/').is_empty() && parsed.query().is_none();

    for raw in query.split_whitespace() {
        let token = raw
            .trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | '<' | '>' | ',' | ';'))
            .to_lowercase();
        if !token.contains('.') {
            continue;
        }
        let with_scheme = if token.contains("://") {
            token.clone()
        } else {
            format!("http://{token}")
        };
        let Some(t) = reqwest::Url::parse(&with_scheme).ok() else {
            continue;
        };
        let Some(t_host) = t.host_str().map(str::to_ascii_lowercase) else {
            continue;
        };
        // Everything the token carries past its scheme, in the parser's form
        // and as the model wrote it, decoded like the result.
        let t_norm = t.as_str().split_once("://").map(|(_, r)| r).unwrap_or("");
        let t_norm = t_norm.trim_end_matches('/').to_string();
        let t_raw = percent_decode(&percent_decode(
            token.split_once("://").map(|(_, r)| r).unwrap_or(&token),
        ))
        .trim_end_matches('/')
        .to_string();
        let payload = !t.path().trim_end_matches('/').is_empty() || t.query().is_some();

        // The token, with a path or query, appears inside the result URL:
        // an echo or a wrapped redirect.
        if payload && (decoded.contains(&t_norm) || decoded.contains(&t_raw)) {
            return Provenance::Echoed;
        }
        // The result is nothing but the host the query named.
        if bare && host == t_host {
            return Provenance::Echoed;
        }
        // The token's host appears in the result's path or query — a
        // wrapper pointing at an address the model wrote.
        let beyond_host = decoded
            .split_once(&host)
            .map(|(_, rest)| rest)
            .unwrap_or("");
        if t_host != host && (beyond_host.contains(&t_host) || beyond_host.contains(&t_raw)) {
            return Provenance::Echoed;
        }
    }
    Provenance::Supplied
}

/// `%XX` → byte, leaving anything malformed as it was. One pass; callers
/// apply it twice for double encoding.
fn percent_decode(s: &str) -> String {
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// How many redirects `web_open` follows before giving up. Each hop is
/// vetted and pinned like the first (`fetch_vetted`).
const MAX_REDIRECTS: usize = 5;

/// Open a search result by the handle `web_search` printed beside it.
///
/// [`Egress::Blind`] by schema: the only argument is a handle, and the URL
/// fetched is the one the search backend returned, looked up in the
/// [`ResultLedger`]. The model chooses among results and never composes a
/// destination, so a payload has no field to ride in. That is the rule
/// `docs/PROVENANCE-DESIGN.md` §4 states — the destination's provenance
/// decides the class — and it is why this tool works in a conversation where
/// `http_fetch` is refused. `ARMED-READING-RESEARCH.md` §3.2 measured that
/// refusal as the complaint left after blind search shipped: you could
/// search, and could not open what you found.
///
/// What it does not close, stated so nobody mistakes it for closed:
/// selection leaks `log2(N)` bits per call to whoever serves the chosen page,
/// and a query is still model-composed, so an attacker with pages indexed for
/// chosen tokens could read which one was opened. Bandwidth, not reach. The
/// leak guard (`block_sends_after_private`) still refuses it, as it refuses
/// every blind send.
///
/// Redirects are followed, unlike `http_fetch`: a redirect target is chosen
/// by the page's server, not by the model, so following it adds no
/// model-authored bytes, and each hop goes through the same vetting.
pub struct WebOpen {
    ledger: Arc<ResultLedger>,
}

impl WebOpen {
    pub fn new(ledger: Arc<ResultLedger>) -> Self {
        WebOpen { ledger }
    }
}

#[async_trait]
impl Tool for WebOpen {
    fn name(&self) -> &str {
        "web_open"
    }

    fn description(&self) -> &str {
        "Read the full page behind a web_search result. Pass the handle printed in brackets \
         before the result's title, not its URL. Works in conversations where http_fetch is \
         refused, because you are choosing among results rather than writing an address."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "result": {
                    "type": "string",
                    "description": "The handle from a web_search result, e.g. \"k3f-2.4\"."
                }
            },
            "required": ["result"]
        })
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default().untrusted().sends_blind()
    }

    /// The URL the handle opens, beside the handle, so an approval card shows
    /// the page and not a pointer to it.
    fn review_input(&self, input: &Value) -> Value {
        let mut shown = input.clone();
        let url = input
            .get("result")
            .and_then(Value::as_str)
            .and_then(|h| self.ledger.resolve(h))
            .unwrap_or_else(|| {
                "(no search result has this handle; nothing would be fetched)".into()
            });
        if let Some(obj) = shown.as_object_mut() {
            obj.insert("url".into(), Value::String(url));
        }
        shown
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let Some(handle) = input.get("result").and_then(Value::as_str) else {
            return Ok(ToolOutput::err("missing required string argument `result`"));
        };
        // Our own words, so not `from_outside`: nothing left the machine.
        let Some(first) = self.ledger.resolve(handle) else {
            return Ok(ToolOutput::err(format!(
                "no search result has the handle {handle:?}. Pass the bracketed handle from a \
                 web_search result exactly; handles do not survive a restart, so search again \
                 if this one is old."
            )));
        };

        let mut url = first;
        for _ in 0..=MAX_REDIRECTS {
            match crate::tool::builtin::fetch_vetted(&url, ctx).await? {
                crate::tool::builtin::Fetched::Done(mut out) => {
                    out.content = format!("{url}\n{}", out.content);
                    // Every URL here is third-party text: the first came from
                    // the search backend, every later one from a `location`
                    // header the far end chose. `fetch_vetted` leaves a
                    // vetting refusal unmarked, which is right for
                    // `http_fetch`, whose URL the model wrote — but a refusal
                    // quoting `https://<payload>.invalid/` back into the
                    // conversation unmarked would put a stranger's bytes in
                    // without arming `untrusted` (found in review of #276).
                    return Ok(out.from_outside());
                }
                crate::tool::builtin::Fetched::Redirect {
                    status,
                    target: None,
                } => {
                    return Ok(ToolOutput::err(format!(
                        "{status} redirect from {url} with no usable location — not followed"
                    ))
                    .from_outside());
                }
                crate::tool::builtin::Fetched::Redirect {
                    status,
                    target: Some(target),
                } => {
                    let base = match reqwest::Url::parse(&url) {
                        Ok(b) => b,
                        // Marked like every other exit past the ledger: the
                        // URL came from a backend or a location header.
                        // `ParseError`'s text does not echo it, but a stated
                        // absolute with one quiet exception reads as a bug.
                        Err(e) => {
                            return Ok(ToolOutput::err(format!("invalid url: {e}")).from_outside())
                        }
                    };
                    match base.join(&target) {
                        Ok(next) => url = next.to_string(),
                        Err(_) => {
                            return Ok(ToolOutput::err(format!(
                                "{status} redirect to {target}, which is not a URL"
                            ))
                            .from_outside())
                        }
                    }
                }
            }
        }
        Ok(ToolOutput::err(format!(
            "more than {MAX_REDIRECTS} redirects, last to {url} — not followed further"
        ))
        .from_outside())
    }
}

/// What an armed run is told about its own narrowing — on **every** exit,
/// which is the whole point of hoisting it out of one of them.
///
/// It first rode only on the success path, and the two outcomes a cut chain is
/// *most likely* to produce said nothing: an empty answer read as "the web has
/// nothing" when the full chain might have answered, and a failing sole
/// backend read as "the tool is broken" rather than "your chain was cut to one
/// entry". Those are the opposite findings the empty-vs-error split in
/// [`SearchChain::run`] exists to keep apart, collapsed again one layer up —
/// and "a model told its tools are broken rewords and retries, eight times in
/// one recorded run" is the measured cost. Found by review, 2026-09-17.
fn narrowing_note(armed: bool) -> &'static str {
    if armed {
        " — this conversation holds private data and third-party content, so the search \
         was served only by backends whose destination your config fixes, at quick depth. \
         Nothing else about it changed, and a fuller chain might answer differently."
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Taint;
    use std::sync::atomic::{AtomicUsize, Ordering};
    // ----------------------------------------------------------------
    // Egress classes — `docs/EGRESS-DESIGN.md`
    // ----------------------------------------------------------------

    /// A backend that answers, remembers the depth it was asked for, and
    /// declares whatever class the test needs.
    struct Classed {
        id: &'static str,
        egress: Egress,
        seen: Arc<std::sync::Mutex<Vec<Depth>>>,
    }

    #[async_trait]
    impl SearchBackend for Classed {
        fn id(&self) -> &str {
            self.id
        }
        fn egress(&self, _depth: Depth) -> Egress {
            self.egress
        }
        async fn search(&self, _q: &str, _l: usize, depth: Depth) -> Result<SearchResponse> {
            self.seen.lock().unwrap().push(depth);
            Ok(SearchResponse {
                results: vec![SearchResult {
                    title: "A page".into(),
                    url: "https://example.com".into(),
                    snippet: "words".into(),
                    published: None,
                    score: None,
                }],
                answer: None,
                backend: self.id.into(),
            })
        }
    }

    fn classed(
        id: &'static str,
        egress: Egress,
    ) -> (Box<dyn SearchBackend>, Arc<std::sync::Mutex<Vec<Depth>>>) {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        (
            Box::new(Classed {
                id,
                egress,
                seen: Arc::clone(&seen),
            }),
            seen,
        )
    }

    // ----------------------------------------------------------------
    // Opening a result — `docs/PROVENANCE-DESIGN.md` §4
    // ----------------------------------------------------------------

    /// A local server that answers each connection with the next canned
    /// response, and records every request line it saw — because "nothing
    /// was fetched" is an assertion about the wire, not about a return value.
    async fn serve(
        responses: Vec<String>,
    ) -> (std::net::SocketAddr, Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        tokio::spawn(async move {
            for response in responses {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let mut req = Vec::new();
                let mut tmp = [0u8; 4096];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => req.extend_from_slice(&tmp[..n]),
                    }
                }
                let line = String::from_utf8_lossy(&req)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_string();
                log.lock().unwrap().push(line);
                let _ = sock.write_all(response.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        (addr, seen)
    }

    fn ok_body(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    /// Loopback is the test server, so the private-IP guard steps aside.
    fn loopback_ctx() -> ToolCtx {
        ToolCtx {
            security: crate::config::SecurityConfig {
                block_private_ips: false,
                ..Default::default()
            },
            ..ToolCtx::default()
        }
    }

    /// Blind is earned by the schema, so the schema is what is asserted: one
    /// property, and it is a handle. A `url` field added later would make
    /// this tool `http_fetch` with a blind label on it.
    #[test]
    fn web_open_is_blind_because_its_schema_has_no_destination() {
        let tool = WebOpen::new(Arc::new(ResultLedger::new()));
        let caps = tool.capabilities();
        assert_eq!(caps.egress, Egress::Blind);
        assert!(caps.untrusted_input, "a page is a stranger's words");
        assert!(!caps.private_data);
        let props: Vec<String> = tool.input_schema()["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(props, vec!["result".to_string()]);
    }

    /// The happy path, measured on the wire: the handle opens exactly the URL
    /// the search returned.
    #[tokio::test]
    async fn a_handle_opens_exactly_the_url_the_search_returned() {
        let (addr, seen) = serve(vec![ok_body("the page")]).await;
        let ledger = Arc::new(ResultLedger::new());
        let handles = ledger.record(&[&format!("http://{addr}/article?id=7")]);
        let handle = handles[0].clone().unwrap();

        let out = WebOpen::new(ledger)
            .call(json!({"result": handle}), &loopback_ctx())
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert!(out.external, "a page is third-party content");
        assert!(out.content.contains("the page"));
        assert_eq!(*seen.lock().unwrap(), vec!["GET /article?id=7 HTTP/1.1"]);
    }

    /// The attack this tool must not become: an injection that wants a
    /// secret on the wire writes a URL, or a guessed handle, into `result`.
    /// Neither opens anything — asserted on the wire, where the leak would be.
    #[tokio::test]
    async fn a_composed_url_or_a_forged_handle_opens_nothing() {
        let (addr, seen) = serve(vec![ok_body("never")]).await;
        let ledger = Arc::new(ResultLedger::new());
        let real = ledger.record(&[&format!("http://{addr}/ok")])[0]
            .clone()
            .unwrap();
        let other_epoch = format!("zz{}", &real[2..]);
        let tool = WebOpen::new(ledger);
        for forged in [
            format!("http://{addr}/?d=SECRET"),
            other_epoch,
            format!("{real}9"),
            "1".to_string(),
        ] {
            let out = tool
                .call(json!({"result": forged}), &loopback_ctx())
                .await
                .unwrap();
            assert!(out.is_error, "{forged} opened: {}", out.content);
            assert!(!out.external, "our own refusal is not third-party text");
            assert!(out.content.contains("no search result has the handle"));
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            seen.lock().unwrap().is_empty(),
            "a request reached the wire"
        );
    }

    /// A redirect is the page's server choosing, not the model, so it is
    /// followed — and every hop is vetted like the first. The second hop here
    /// names a blocked host, and must be refused rather than fetched.
    #[tokio::test]
    async fn redirects_are_followed_and_every_hop_is_vetted() {
        let (addr, seen) = serve(vec![
            "HTTP/1.1 302 Found\r\nlocation: /final\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".into(),
            ok_body("landed"),
        ])
        .await;
        let ledger = Arc::new(ResultLedger::new());
        let h = ledger.record(&[&format!("http://{addr}/start")])[0]
            .clone()
            .unwrap();
        let out = WebOpen::new(Arc::clone(&ledger))
            .call(json!({"result": h}), &loopback_ctx())
            .await
            .unwrap();
        assert!(out.content.contains("landed"), "{}", out.content);
        assert_eq!(
            *seen.lock().unwrap(),
            vec!["GET /start HTTP/1.1", "GET /final HTTP/1.1"]
        );

        let (addr2, _) = serve(vec![
            "HTTP/1.1 302 Found\r\nlocation: http://blocked.example/x\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".into(),
        ])
        .await;
        let h2 = ledger.record(&[&format!("http://{addr2}/start")])[0]
            .clone()
            .unwrap();
        let mut ctx = loopback_ctx();
        ctx.security.blocked_domains = vec!["blocked.example".into()];
        let out = WebOpen::new(Arc::clone(&ledger))
            .call(json!({"result": h2}), &ctx)
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(out.content.contains("blocked-domain"), "{}", out.content);
        // The refusal quotes a URL the far end chose, so it arms `untrusted`
        // like any other third-party text. Fails on the first cut, which
        // returned it unmarked.
        assert!(out.external, "a refused hop quotes the far end's location");

        // A 3xx with no location is an error, not a second request to a
        // path spelled like a placeholder.
        let (addr3, seen3) = serve(vec![
            "HTTP/1.1 302 Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".into(),
            ok_body("should not be fetched"),
        ])
        .await;
        let h3 = ledger.record_for_test(&format!("http://{addr3}/start"));
        let out = WebOpen::new(ledger)
            .call(json!({"result": h3}), &loopback_ctx())
            .await
            .unwrap();
        assert!(
            out.is_error && out.content.contains("no usable location"),
            "{}",
            out.content
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(*seen3.lock().unwrap(), vec!["GET /start HTTP/1.1"]);
    }

    /// An approval card for `web_open` shows where the handle leads: a
    /// handle alone is the whole argument and none of the decision.
    #[test]
    fn a_review_shows_the_url_a_handle_opens() {
        let ledger = Arc::new(ResultLedger::new());
        let h = ledger.record_for_test("https://example.org/article");
        let tool = WebOpen::new(ledger);
        let shown = tool.review_input(&json!({"result": h}));
        assert_eq!(shown["url"], "https://example.org/article");
        assert_eq!(shown["result"], h.as_str());
        let unknown = tool.review_input(&json!({"result": "zzz-000000.1"}));
        assert!(unknown["url"]
            .as_str()
            .unwrap()
            .contains("nothing would be fetched"));
    }

    /// `web_search` prints the handle only when a `web_open` shares its
    /// ledger, and the handle it prints resolves to that result's URL.
    #[tokio::test]
    async fn search_prints_a_handle_that_resolves_to_its_result() {
        let (blind, _) = classed("searxng", Egress::Blind);
        let ledger = Arc::new(ResultLedger::new());
        let tool = WebSearch::new(Arc::new(SearchChain::new(vec![blind])))
            .with_ledger(Arc::clone(&ledger));
        let out = tool
            .call(json!({"query": "x"}), &ctx_with(None))
            .await
            .unwrap();
        let handle = out
            .content
            .split('[')
            .nth(1)
            .and_then(|r| r.split(']').next())
            .expect("a bracketed handle");
        assert_eq!(
            ledger.resolve(handle).as_deref(),
            Some("https://example.com")
        );
        assert!(tool.description().contains("web_open"));

        let (blind, _) = classed("searxng", Egress::Blind);
        let bare = WebSearch::new(Arc::new(SearchChain::new(vec![blind])));
        let out = bare
            .call(json!({"query": "x"}), &ctx_with(None))
            .await
            .unwrap();
        assert!(!out.content.contains('['), "{}", out.content);
        assert!(!bare.description().contains("web_open"));
    }

    /// A backend that echoes a URL-shaped query back as a result must not
    /// mint a handle for it: that would be a destination the model wrote.
    #[tokio::test]
    async fn a_result_whose_host_the_query_named_gets_no_handle() {
        struct Echo;
        #[async_trait]
        impl SearchBackend for Echo {
            fn id(&self) -> &str {
                "echo"
            }
            fn egress(&self, _depth: Depth) -> Egress {
                Egress::Blind
            }
            async fn search(&self, q: &str, _l: usize, _d: Depth) -> Result<SearchResponse> {
                Ok(SearchResponse {
                    results: vec![
                        SearchResult {
                            title: "echoed".into(),
                            url: q.to_string(),
                            snippet: String::new(),
                            published: None,
                            score: None,
                        },
                        SearchResult {
                            title: "honest".into(),
                            url: "https://example.org/about".into(),
                            snippet: String::new(),
                            published: None,
                            score: None,
                        },
                    ],
                    answer: None,
                    backend: "echo".into(),
                })
            }
        }
        let ledger = Arc::new(ResultLedger::new());
        let tool = WebSearch::new(Arc::new(SearchChain::new(vec![Box::new(Echo)])))
            .with_ledger(Arc::clone(&ledger));
        let out = tool
            .call(
                json!({"query": "https://Evil.example/?d=SECRET"}),
                &ctx_with(None),
            )
            .await
            .unwrap();
        let handles: Vec<&str> = out
            .content
            .split('[')
            .skip(1)
            .filter_map(|r| r.split(']').next())
            .collect();
        assert_eq!(handles.len(), 1, "{}", out.content);
        assert_eq!(
            ledger.resolve(handles[0]).as_deref(),
            Some("https://example.org/about")
        );
        assert!(
            out.content.contains("carries what the query wrote"),
            "{}",
            out.content
        );
    }

    /// The spellings a model controls all normalise to the same answer, and
    /// a query that only mentions a domain keeps that domain's pages.
    #[test]
    fn provenance_catches_echoes_in_any_spelling_and_spares_mentions() {
        use Provenance::*;
        let q = "https://evil.example/?d=SECRET";
        // Verbatim echo, redirect wrapper, double-encoded wrapper.
        assert_eq!(provenance(q, "https://evil.example/?d=SECRET"), Echoed);
        assert_eq!(
            provenance(
                q,
                "https://search.example/r?u=https%3A%2F%2Fevil.example%2F%3Fd%3DSECRET"
            ),
            Echoed
        );
        assert_eq!(
            provenance(
                q,
                "https://search.example/r?u=https%253A%252F%252Fevil.example%252F%253Fd%253DSECRET"
            ),
            Echoed
        );
        // A wrapper that also encodes the host: only decoding sees it.
        assert_eq!(
            provenance(
                q,
                "https://search.example/r?u=https%3A%2F%2F%65%76%69%6C.example%2F%3Fd%3DSECRET"
            ),
            Echoed
        );
        // A percent-encoded host in the query, echoed back normalised.
        assert_eq!(
            provenance("https://%65vil.example/?d=S", "https://evil.example/?d=S"),
            Echoed
        );
        // A Unicode host, echoed back as punycode by the parser.
        let uni = "https://evıl.example/?d=S";
        let puny = reqwest::Url::parse(uni).unwrap().to_string();
        assert_eq!(provenance(uni, &puny), Echoed);
        // A bare host whose subdomain carries data, echoed as a bare result.
        assert_eq!(
            provenance("s3cr3t.evil.example", "https://s3cr3t.evil.example/"),
            Echoed
        );
        // A wrapper pointing at a bare host the query named.
        assert_eq!(
            provenance(
                "s3cr3t.evil.example",
                "https://search.example/r?u=s3cr3t.evil.example"
            ),
            Echoed
        );
        // Honest: a mentioned domain's real page, and an unrelated result.
        assert_eq!(
            provenance(
                "docs.rust-lang.org tracing",
                "https://docs.rust-lang.org/std/index.html"
            ),
            Supplied
        );
        assert_eq!(
            provenance("rust tracing crate", "https://docs.rs/tracing"),
            Supplied
        );
        assert_eq!(provenance("anything", "//no-scheme.example/x"), Unparseable);
        // A `%` before a multi-byte character decodes as itself, never panics.
        assert_eq!(percent_decode("a%ı%4"), "a%ı%4");
        assert_eq!(percent_decode("%41%2f"), "A/");
    }

    /// Forgetting is bounded and exact: past the cap the oldest handle is
    /// refused, the newest still resolves.
    #[test]
    fn the_ledger_forgets_the_oldest_past_its_cap() {
        let ledger = ResultLedger::new();
        let first = ledger.record(&["https://a.example/0"])[0].clone().unwrap();
        for i in 0..LEDGER_CAP {
            ledger.record(&[&format!("https://a.example/{}", i + 1)]);
        }
        assert!(ledger.resolve(&first).is_none());
        let last = ledger.record(&["https://a.example/last"])[0]
            .clone()
            .unwrap();
        assert_eq!(
            ledger.resolve(&last).as_deref(),
            Some("https://a.example/last")
        );
    }

    fn ctx_with(taint: Option<Taint>) -> ToolCtx {
        ToolCtx {
            taint,
            ..ToolCtx::default()
        }
    }

    fn ctx_waived(taint: Option<Taint>) -> ToolCtx {
        let mut ctx = ctx_with(taint);
        ctx.security.trifecta = crate::config::TrifectaPolicy::Allow;
        ctx
    }

    /// The three shipped backends, classified by what each does with the
    /// query text — the only property `Egress::Blind` ever claimed.
    #[test]
    fn the_shipped_backends_are_classified_by_what_they_do_with_the_query() {
        let exa = Exa::new("k".into(), None).unwrap();
        assert_eq!(
            exa.egress(Depth::Quick),
            Egress::Blind,
            "`type: auto` is an index query"
        );
        assert_eq!(
            exa.egress(Depth::Deep),
            Egress::Chosen,
            "`deep-reasoning` is agentic research that fetches pages the query can steer it to"
        );

        let tavily = Tavily::new("k".into(), None).unwrap();
        assert_eq!(tavily.egress(Depth::Quick), Egress::Blind);
        assert_eq!(
            tavily.egress(Depth::Deep),
            Egress::Blind,
            "`advanced` ranks and extracts harder; crawling is a different API mecha never calls"
        );

        let searxng = Searxng::new("http://127.0.0.1:8888".into()).unwrap();
        assert_eq!(searxng.egress(Depth::Quick), Egress::Blind);
        assert_eq!(searxng.egress(Depth::Deep), Egress::Blind);
    }

    /// Fails on a trait default of `Blind`: a backend added later must not
    /// inherit a promise nobody made for it. Unknown is never clean.
    #[test]
    fn an_unclassified_backend_is_not_blind_and_never_serves_an_armed_run() {
        struct Newcomer;
        #[async_trait]
        impl SearchBackend for Newcomer {
            fn id(&self) -> &str {
                "newcomer"
            }
            async fn search(&self, _q: &str, _l: usize, _d: Depth) -> Result<SearchResponse> {
                unreachable!("an armed run must never reach an unclassified backend")
            }
        }
        assert_eq!(Newcomer.egress(Depth::Quick), Egress::Chosen);

        let chain = SearchChain::new(vec![Box::new(Newcomer)]);
        assert!(!chain.has_blind_backend());
        // And the tool over it declares the conservative class, so the
        // interlock refuses it rather than the tool promising a route it
        // does not have.
        let tool = WebSearch::new(Arc::new(chain));
        assert_eq!(tool.capabilities().egress, Egress::Chosen);
        assert!(
            tool.denial_remedy(DenialCause::Injection)
                .unwrap()
                .contains("searxng"),
            "a refusal that names no exit teaches the operator to weaken policy"
        );
    }

    /// The whole point. An armed conversation still searches — on the
    /// backends whose destination the operator fixed, and only those.
    #[tokio::test]
    async fn an_armed_conversation_is_served_only_by_blind_backends() {
        let (chosen, chosen_seen) = classed("paid", Egress::Chosen);
        let (blind, blind_seen) = classed("searxng", Egress::Blind);
        // Chosen first in config order, so reaching the blind one cannot be
        // an accident of ordering.
        let tool = WebSearch::new(Arc::new(SearchChain::new(vec![chosen, blind])));

        let armed = Taint {
            private: true,
            untrusted: true,
        };
        let out = tool
            .call(
                json!({"query": "tide times", "depth": "deep"}),
                &ctx_with(Some(armed)),
            )
            .await
            .unwrap();

        assert!(!out.is_error);
        assert!(out.content.contains("via searxng"));
        assert!(
            chosen_seen.lock().unwrap().is_empty(),
            "a backend the model could point somewhere must not see an armed query"
        );
        assert_eq!(
            *blind_seen.lock().unwrap(),
            vec![Depth::Quick],
            "deep is forced down to the depth the classification was made at"
        );
        assert!(
            out.content.contains("destination your config fixes"),
            "a silently shallower search reads as a broken tool, and models retry broken tools"
        );
    }

    /// The two outcomes a cut chain is most likely to produce, and the ones
    /// that said nothing about the cut until review caught it on 2026-09-17.
    ///
    /// An empty blind result must not read as "the web has nothing" — those
    /// are the opposite findings `SearchChain::run`'s empty-vs-error split
    /// exists to keep apart — and a failing sole backend must not read as
    /// "the tool is broken", which is the eight-retries failure. Fails on the
    /// success-path-only notice.
    #[tokio::test]
    async fn an_empty_or_failing_blind_search_still_says_it_was_narrowed() {
        struct Quiet {
            fail: bool,
        }
        #[async_trait]
        impl SearchBackend for Quiet {
            fn id(&self) -> &str {
                "searxng"
            }
            fn egress(&self, _d: Depth) -> Egress {
                Egress::Blind
            }
            async fn search(&self, _q: &str, _l: usize, _d: Depth) -> Result<SearchResponse> {
                if self.fail {
                    bail!("quota exhausted");
                }
                Ok(SearchResponse {
                    backend: "searxng".into(),
                    ..Default::default()
                })
            }
        }

        let armed = Taint {
            private: true,
            untrusted: true,
        };

        // Answered, with nothing. Not the same as the web being empty.
        let empty = WebSearch::new(Arc::new(SearchChain::new(vec![Box::new(Quiet {
            fail: false,
        })])));
        let out = empty
            .call(json!({"query": "tide times"}), &ctx_with(Some(armed)))
            .await
            .unwrap();
        assert!(out.content.contains("no results"), "{}", out.content);
        assert!(
            out.content.contains("destination your config fixes"),
            "an empty narrowed search must not read as an empty web: {}",
            out.content
        );

        // Did not answer at all. Not the same as the tool being broken.
        let broken = WebSearch::new(Arc::new(SearchChain::new(vec![Box::new(Quiet {
            fail: true,
        })])));
        let out = broken
            .call(json!({"query": "tide times"}), &ctx_with(Some(armed)))
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(
            out.content.contains("destination your config fixes"),
            "a failing sole backend must not read as a broken tool: {}",
            out.content
        );

        // And a clean conversation says none of it on any path.
        let clean = WebSearch::new(Arc::new(SearchChain::new(vec![Box::new(Quiet {
            fail: false,
        })])));
        let out = clean
            .call(
                json!({"query": "tide times"}),
                &ctx_with(Some(Taint::default())),
            )
            .await
            .unwrap();
        assert!(!out.content.contains("destination your config fixes"));
    }

    /// Clean, nothing is narrowed: the full chain at the depth asked for.
    #[tokio::test]
    async fn a_clean_conversation_keeps_the_whole_chain_and_the_depth_it_asked_for() {
        let (chosen, chosen_seen) = classed("paid", Egress::Chosen);
        let (blind, blind_seen) = classed("searxng", Egress::Blind);
        let tool = WebSearch::new(Arc::new(SearchChain::new(vec![chosen, blind])));

        let out = tool
            .call(
                json!({"query": "tide times", "depth": "deep"}),
                &ctx_with(Some(Taint::default())),
            )
            .await
            .unwrap();

        assert_eq!(*chosen_seen.lock().unwrap(), vec![Depth::Deep]);
        assert!(
            blind_seen.lock().unwrap().is_empty(),
            "the first backend answered"
        );
        assert!(!out.content.contains("destination your config fixes"));
    }

    /// `ToolCtx::taint`'s own contract, honoured here: nobody stamped it, so
    /// it is fully tainted. A subagent's context and any run wired outside
    /// the loop must not pass as clean by omission — the cost of being wrong
    /// this way is a quick search instead of a deep one.
    #[tokio::test]
    async fn an_unstamped_taint_is_treated_as_armed() {
        let (chosen, chosen_seen) = classed("paid", Egress::Chosen);
        let (blind, _) = classed("searxng", Egress::Blind);
        let tool = WebSearch::new(Arc::new(SearchChain::new(vec![chosen, blind])));

        let out = tool
            .call(json!({"query": "tide times"}), &ctx_with(None))
            .await
            .unwrap();

        assert!(out.content.contains("via searxng"));
        assert!(chosen_seen.lock().unwrap().is_empty());
    }

    /// With no blind backend the *declaration* is what stops an armed call —
    /// `capabilities()` is `Chosen`, so the interlock refuses it under
    /// `block` with a remedy naming SearXNG. Reaching `call` at all therefore
    /// means the gate let it through, and the honest answer then is the full
    /// chain rather than a refusal the caller has already overridden.
    ///
    /// This test used to assert the opposite, and was wrong for a reason
    /// worth keeping: it read `search_blind`'s `bail!` as a safety net when
    /// it was really an unreachable-by-construction precondition that the
    /// two-term `armed` had quietly made reachable.
    #[tokio::test]
    async fn no_blind_backend_means_the_declaration_gates_it_and_call_answers() {
        let (chosen, chosen_seen) = classed("paid", Egress::Chosen);
        let tool = WebSearch::new(Arc::new(SearchChain::new(vec![chosen])));

        assert_eq!(
            tool.capabilities().egress,
            Egress::Chosen,
            "the gate is the declaration, and it is the conservative one"
        );

        let out = tool
            .call(
                json!({"query": "x"}),
                &ctx_with(Some(Taint {
                    private: true,
                    untrusted: true,
                })),
            )
            .await
            .unwrap();
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(*chosen_seen.lock().unwrap(), vec![Depth::Quick]);
    }

    /// `search_blind` is `pub`, so its precondition is still defended for a
    /// caller that is not `WebSearch::call` — it errors readably rather than
    /// returning a silent empty, which would read as "the web has nothing".
    #[tokio::test]
    async fn search_blind_still_refuses_readably_when_called_with_no_blind_entry() {
        let (chosen, _) = classed("paid", Egress::Chosen);
        let chain = SearchChain::new(vec![chosen]);
        let err = chain.search_blind("x", 5).await.unwrap_err().to_string();
        assert!(err.contains("none is configured"), "{err}");
    }

    /// `trifecta = "allow"` is the operator's written waiver of the injection
    /// interlock, and the switch table calls it exactly that. Narrowing anyway
    /// would remove deep search from the one operator who had opted out, with
    /// no setting able to put it back. Found by review on 2026-09-17; fails on
    /// the first cut of this feature, which read `ctx.taint` alone.
    #[tokio::test]
    async fn an_explicit_waiver_reaches_the_whole_chain_while_armed() {
        let (chosen, chosen_seen) = classed("paid", Egress::Chosen);
        let (blind, blind_seen) = classed("searxng", Egress::Blind);
        let tool = WebSearch::new(Arc::new(SearchChain::new(vec![chosen, blind])));

        let armed = Taint {
            private: true,
            untrusted: true,
        };
        let out = tool
            .call(
                json!({"query": "tide times", "depth": "deep"}),
                &ctx_waived(Some(armed)),
            )
            .await
            .unwrap();

        assert_eq!(
            *chosen_seen.lock().unwrap(),
            vec![Depth::Deep],
            "the waiver reaches the tool, not only the interlock"
        );
        assert!(blind_seen.lock().unwrap().is_empty());
        assert!(!out.content.contains("destination your config fixes"));
    }

    /// And `"ask"` is not a waiver: a blind call raises no escalation, so
    /// there is no human yes to widen on. It keeps the degradation, which is
    /// the better answer anyway — search works and nobody sees a modal.
    #[tokio::test]
    async fn ask_is_not_a_waiver_and_still_degrades() {
        let (chosen, chosen_seen) = classed("paid", Egress::Chosen);
        let (blind, _) = classed("searxng", Egress::Blind);
        let tool = WebSearch::new(Arc::new(SearchChain::new(vec![chosen, blind])));

        let mut ctx = ctx_with(Some(Taint {
            private: true,
            untrusted: true,
        }));
        ctx.security.trifecta = crate::config::TrifectaPolicy::Ask;

        let out = tool.call(json!({"query": "x"}), &ctx).await.unwrap();
        assert!(chosen_seen.lock().unwrap().is_empty());
        assert!(out.content.contains("via searxng"));
    }

    /// The remedy answers the interlock and only the interlock. Printed on a
    /// leak-guard refusal it is false in every clause — the guard refuses
    /// `Blind` too, so an operator who adds SearXNG reaches a *second*
    /// refusal, this time with no remedy at all, having followed the exit.
    /// Found by review on 2026-09-17; fails on the argument-less signature.
    #[test]
    fn the_leak_guard_gets_no_remedy_from_this_tool() {
        let (only_chosen, _) = classed("paid", Egress::Chosen);
        let bare = WebSearch::new(Arc::new(SearchChain::new(vec![only_chosen])));

        assert!(
            bare.denial_remedy(DenialCause::Injection)
                .is_some_and(|r| r.contains("searxng")),
            "the interlock's refusal still names its exit"
        );
        assert_eq!(
            bare.denial_remedy(DenialCause::Leak),
            None,
            "adding a blind backend does not lift the leak guard, so do not say it does"
        );
    }

    /// A `Chosen`-only chain declares `Chosen`, so under `ask` the interlock
    /// escalates and a human can say yes — and that yes must answer, not
    /// error. Before this, `call` degraded anyway and hit `search_blind`'s
    /// `bail!`, making `ask` strictly worse than `block`, which at least
    /// hands back a remedy naming SearXNG. Fails on the two-term `armed`.
    #[tokio::test]
    async fn an_approved_call_on_a_chosen_only_chain_answers_rather_than_erroring() {
        let (chosen, chosen_seen) = classed("paid", Egress::Chosen);
        let tool = WebSearch::new(Arc::new(SearchChain::new(vec![chosen])));

        let mut ctx = ctx_with(Some(Taint {
            private: true,
            untrusted: true,
        }));
        ctx.security.trifecta = crate::config::TrifectaPolicy::Ask;

        let out = tool
            .call(json!({"query": "tide times", "depth": "deep"}), &ctx)
            .await
            .unwrap();

        assert!(
            !out.is_error,
            "the human's yes must not become an error: {}",
            out.content
        );
        assert_eq!(*chosen_seen.lock().unwrap(), vec![Depth::Deep]);
        assert!(!out.content.contains("destination your config fixes"));
    }

    /// The invariant `WebSearch::capabilities` rests on, in the half that
    /// lives here: while armed, `call` reaches only blind backends, so the
    /// `Blind` declaration is true in the one state any control reads it. The
    /// other half — that nothing consults the class while clean — is
    /// `agent::tests::a_clean_conversation_does_not_consult_the_egress_class`.
    #[test]
    fn the_declared_class_tracks_whether_a_blind_route_exists() {
        let (chosen, _) = classed("paid", Egress::Chosen);
        let (blind, _) = classed("searxng", Egress::Blind);

        let mixed = WebSearch::new(Arc::new(SearchChain::new(vec![chosen, blind])));
        assert_eq!(mixed.capabilities().egress, Egress::Blind);
        assert!(
            mixed.denial_remedy(DenialCause::Injection).is_none(),
            "nothing to remedy: it is not refused"
        );

        let (only_chosen, _) = classed("paid", Egress::Chosen);
        let bare = WebSearch::new(Arc::new(SearchChain::new(vec![only_chosen])));
        assert_eq!(bare.capabilities().egress, Egress::Chosen);
    }

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

        // A clean conversation, so the whole chain is in play. An unstamped
        // `ToolCtx::default()` would be read as armed — see
        // `an_unstamped_taint_is_treated_as_armed` — and this test is about
        // what the *results* are marked with, not about the chain.
        let out = tool
            .call(json!({"query": "rust"}), &ctx_with(Some(Taint::default())))
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
        let (blind, _) = classed("searxng", Egress::Blind);
        let tool = WebSearch::new(Arc::new(SearchChain::new(vec![blind])));
        let caps = tool.capabilities();
        assert!(caps.untrusted_input, "results are attacker-influenced");
        assert_eq!(
            caps.egress,
            Egress::Blind,
            "the query leaves the machine, but only to the backends config names"
        );
        // The empty chain is the degenerate case and takes the conservative
        // class with everything else that has no blind route.
        let bare = WebSearch::new(Arc::new(SearchChain::new(Vec::new())));
        assert_eq!(bare.capabilities().egress, Egress::Chosen);
    }
}
