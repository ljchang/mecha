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

use crate::tool::{Capabilities, Egress, Tool, ToolCtx, ToolOutput};
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
    /// `Depth::Quick` sends `"type": "auto"` — an index query, which is search
    /// terms and nothing else. `Depth::Deep` sends `"type": "deep-reasoning"`,
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

    /// [`Egress::Blind`] — the input schema above has no destination field,
    /// so the query reaches whatever `[[search]]` names and nothing else.
    ///
    /// Except with no blind backend configured, where there is no armed path
    /// at all and the honest declaration is the conservative one, so the
    /// interlock refuses with [`denial_remedy`](Tool::denial_remedy) naming
    /// the fix.
    ///
    /// **The invariant this rests on, because it is not locally obvious:** the
    /// `Blind`/`Chosen` distinction is only ever consulted while the
    /// conversation is armed, and while armed [`call`](Tool::call) reaches
    /// only the blind backends. While *clean* the full chain is reachable and
    /// Exa's deep mode is not blind — but no control keys off the distinction
    /// in that state: the interlock requires `trifecta_armed()`, and the leak
    /// guard treats both classes alike. Change either of those (the table in
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

    fn denial_remedy(&self) -> Option<String> {
        // Only one condition can set the conservative class here, and unlike
        // `shell` there is nothing to say in the other case — a blind chain is
        // never refused by the interlock at all.
        (!self.chain.has_blind_backend()).then(|| {
            "Web search is refused here only because every configured backend is one \
             the model could point somewhere. Add a backend whose destination your \
             config fixes — `[[search]]` with `kind = \"searxng\"` — and search keeps \
             working in conversations that hold private data."
                .into()
        })
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
        let armed = ctx.taint.is_none_or(|t| t.trifecta_armed());

        let response = match if armed {
            self.chain.search_blind(query, limit).await
        } else {
            self.chain.search(query, limit, depth).await
        } {
            Ok(r) => r,
            // `from_outside`, like the results: a backend's error carries what
            // the backend said, which is a third party's text.
            Err(e) => return Ok(ToolOutput::err(format!("{e:#}")).from_outside()),
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
        // Say why, or a model that asked for a deep search and got a shallow
        // one has to guess — and the guess models make is that the tool is
        // broken, which is the eight-retries failure the empty-vs-error split
        // above exists to prevent.
        if armed {
            out.push_str(
                " — this conversation holds private data and third-party content, so the \
                 search was served only by backends whose destination your config fixes, \
                 at quick depth. Nothing else about it changed.",
            );
        }

        // Everything above was written by strangers.
        Ok(ToolOutput::ok(out).from_outside())
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

    fn ctx_with(taint: Option<Taint>) -> ToolCtx {
        ToolCtx {
            taint,
            ..ToolCtx::default()
        }
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
            tool.denial_remedy().unwrap().contains("searxng"),
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

    /// Armed with no blind backend configured is an error the model can read,
    /// not a panic and not a silent empty — and the interlock has already
    /// refused the call by then anyway, because `capabilities` declared the
    /// conservative class. Both halves matter: the declaration is what stops
    /// the call, and this is what happens if anything ever reaches past it.
    #[tokio::test]
    async fn armed_with_no_blind_backend_is_a_readable_refusal() {
        let (chosen, _) = classed("paid", Egress::Chosen);
        let tool = WebSearch::new(Arc::new(SearchChain::new(vec![chosen])));
        let armed = Taint {
            private: true,
            untrusted: true,
        };
        let out = tool
            .call(json!({"query": "x"}), &ctx_with(Some(armed)))
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(out.content.contains("none is configured"));
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
            mixed.denial_remedy().is_none(),
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
