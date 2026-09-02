//! Model providers.
//!
//! A provider translates [`CompletionRequest`] onto some wire protocol and
//! translates the reply back into [`CompletionResponse`]. Everything above this
//! layer — the agent loop, tools, sessions — is provider-agnostic.

pub mod anthropic;
pub mod openai;
pub mod preflight;
pub mod retry;
pub(crate) mod sse;

use crate::message::{CompletionRequest, CompletionResponse, Usage};
use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc::UnboundedSender;

/// What to ask a local reasoning model for, when nothing better is known.
///
/// **The number is the vendor's, and the reason it is a constant is that four
/// call sites had four different literals.** Qwen3.6-35B-A3B's card asks for
/// an output length of 32,768 tokens for most queries (81,920 for hard ones);
/// Qwen3.8-27B goes further and budgets the two halves separately — 262,144
/// for reasoning content, 131,072 for the final response. Both are far above
/// anything this repository was asking for: the reflector, the distiller and
/// the eval judge all sat at 4,096 against a server running
/// `--reasoning-budget 4096`, which leaves *nothing* for an answer once
/// thinking has run.
///
/// **Measured on this box before being adopted** (2026-08-29, qwen3.6-35b-a3b,
/// eight samples on a deliberation-shaped task):
///
/// | max_tokens | completion tokens | empty replies |
/// |---|---|---|
/// | 2048 | 2048, `finish_reason: length` | **1 of 1** |
/// | 4096 | 1453–2876 | 0 of 5 |
/// | 16384 | 2154–2855 | 0 of 2 |
/// | 32768 | 2187–2707 | 0 of 2 |
///
/// The distribution does **not** move with the cap. An early two-sample read
/// suggested reasoning expands to fill the budget; eight samples say that was
/// variance. So this is a pure safety margin — raising it costs no latency and
/// buys headroom on exactly the inputs that overflow, which are the long ones.
/// A modest task already spent 70% of 4,096 in its worst sample; the followup
/// probe in `mecha validate`, which re-asks a whole mid-task conversation,
/// overflowed on nearly every call and had its silence graded as a bad answer.
///
/// Not a floor for every provider — an Anthropic request is priced per token
/// and bounded differently. This is the local-server contract, where the cap
/// is free until it binds.
pub const LOCAL_MAX_TOKENS: u32 = 32_768;

/// Incremental output, emitted only when a sink is supplied.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    TextDelta(String),
    ThinkingDelta(String),
    /// A tool call has begun; arguments are still streaming.
    ToolUseStart {
        name: String,
    },
    /// Everything known about this turn's token usage *so far*, cumulative.
    ///
    /// Emitted as it arrives rather than only at the end, because cancelling a
    /// run drops the provider future and with it the final frame that carries
    /// the totals. Without this, a run interrupted on its first turn reports
    /// zero tokens — and the tokens were spent. Input is usually known from the
    /// very first frame, which is the expensive half when a cached prefix is in
    /// play.
    Usage(Usage),
}

pub type StreamSink = UnboundedSender<StreamEvent>;

#[async_trait]
pub trait Provider: Send + Sync {
    /// Stable identifier used in config and `--provider`.
    fn id(&self) -> &str;

    /// Model used when the caller doesn't name one.
    fn default_model(&self) -> &str;

    /// Whether this provider will actually put an image in front of a model.
    ///
    /// Asked by whatever is about to *build* an image block — the Slack
    /// connector, the TUI — so it can name a file instead of reading and
    /// base64-encoding one that would only be rendered as text anyway. The
    /// encoders degrade correctly without this; what it saves is a megabyte
    /// of base64 written into a transcript for nothing.
    ///
    /// Defaults to `false`, so a provider added later is text-only until
    /// somebody says otherwise — the direction that fails safe, on the
    /// `unrouted_domains` reasoning: forgetting costs a feature that does
    /// not fire, and the alternative costs a failed request.
    fn vision(&self) -> bool {
        false
    }

    /// Run one turn. With `sink`, stream and emit deltas as they arrive; the
    /// accumulated response is still returned.
    async fn complete(
        &self,
        req: &CompletionRequest,
        sink: Option<&StreamSink>,
    ) -> Result<CompletionResponse>;
}

/// Build a provider from a config entry.
pub fn build(cfg: &crate::config::ProviderConfig) -> Result<Box<dyn Provider>> {
    match cfg.kind.as_str() {
        "anthropic" => Ok(Box::new(anthropic::Anthropic::from_config(cfg)?)),
        "openai" | "openai-compatible" | "local" => {
            Ok(Box::new(openai::OpenAiCompatible::from_config(cfg)?))
        }
        other => {
            anyhow::bail!("unknown provider kind {other:?} (expected: anthropic, openai, local)")
        }
    }
}

/// Tries the primary, and on a *transient* exhaustion tries each fallback in
/// order — turn-local, so the next call starts from the primary again.
///
/// Two rules carry the design:
///
/// - **Only errors carrying a [`retry::ProviderError`] in their chain are
///   eligible, and only transient ones.** The providers attach that marker
///   exactly when nothing of the attempt was consumed — a mid-stream failure
///   has already shown the user deltas, and re-issuing it would replay half
///   an answer. `Invalid`/`ContextOverflow` fail identically everywhere, and
///   `Auth`/`Billing` are the primary's problem, not a routing decision.
/// - **Each fallback answers as itself.** The request's model name is
///   rewritten to the fallback's own default — sending one server's model
///   name to another server was a real recorded bug (`mecha replay -p`).
pub struct Failover {
    primary: Box<dyn Provider>,
    fallbacks: Vec<(String, Box<dyn Provider>)>,
}

/// The longest silence a provider connection may go without delivering a
/// byte before the request is failed: a stall, not a long answer.
///
/// Both clients used to put one 900 s `timeout` on the *whole* exchange,
/// streamed body included. At `LOCAL_MAX_TOKENS = 32_768` and ~60 tok/s a long
/// answer runs ~550 s before prefill and queue wait, so a legitimate answer
/// died mid-stream as a plain error and the partial text was discarded.
/// Per-read is the right shape for a stream: the server is either sending
/// tokens or it is not. Ten minutes, not five: a cold prefill at 170k tokens
/// on llama-server is ~120 s of silence before the first token *uncontended*,
/// and `docs/LLAMA-SERVER.md` measured a ~2.85× slowdown under contention on
/// this hardware, which puts the worst legitimate silence near 350 s. This is
/// the one place the change trades a bound rather than only widening one —
/// the old 900 s exchange cap also covered the pre-first-token wait — so the
/// margin errs long. A stall that lasts ten minutes is a dead connection.
pub const STALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// The whole-exchange cap that still applies to a *non-streaming* request,
/// where the body arrives at once and a per-read timeout would count the
/// entire generation as one read.
pub const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(900);

/// Time allowed to open the connection, separately from the reads.
pub const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The two HTTP clients a provider needs, chosen per request by whether the
/// body streams.
///
/// Two, because reqwest's `read_timeout` is a *client* setting and applies
/// to the wait for the response head as well as to the body — and
/// `RequestBuilder::timeout` overrides only the client's total deadline,
/// never the read timeout. So one client with `read_timeout(STALL)` bounded a
/// non-streaming request at `min(whole, stall)`, where the entire generation
/// is one silent read: the PR review of the first version found the
/// non-streaming cap had *tightened* from 900 s to 300 s in exactly the
/// "long answer, not a stall" case the change existed to fix, and the
/// resulting error classified as transient and retried a generation the
/// server had already finished. The streaming client bounds each read; the
/// whole-exchange client bounds the exchange and has no per-read bound at
/// all.
#[derive(Clone)]
pub struct HttpClients {
    /// For a request whose body arrives at once: the whole exchange is
    /// capped, and silence before the head is generation, not a stall.
    pub whole: reqwest::Client,
    /// For a streamed request: each read is capped, the exchange is not.
    pub stream: reqwest::Client,
}

impl HttpClients {
    pub fn build() -> reqwest::Result<Self> {
        Self::with(CONNECT_TIMEOUT, STALL_TIMEOUT, REQUEST_TIMEOUT)
    }

    /// The constructor the test drives with short durations.
    pub fn with(
        connect: std::time::Duration,
        stall: std::time::Duration,
        whole: std::time::Duration,
    ) -> reqwest::Result<Self> {
        Ok(HttpClients {
            whole: reqwest::Client::builder()
                .connect_timeout(connect)
                .timeout(whole)
                .build()?,
            stream: reqwest::Client::builder()
                .connect_timeout(connect)
                .read_timeout(stall)
                .build()?,
        })
    }

    /// Two default clients with no timeouts, for tests that measure counts
    /// and outcomes rather than wall clock.
    pub fn plain() -> Self {
        HttpClients {
            whole: reqwest::Client::new(),
            stream: reqwest::Client::new(),
        }
    }

    /// The client for this body: the streaming one when it says `stream:
    /// true`, the whole-exchange one otherwise.
    pub fn for_body(&self, body: &serde_json::Value) -> &reqwest::Client {
        if body.get("stream").and_then(serde_json::Value::as_bool) == Some(true) {
            &self.stream
        } else {
            &self.whole
        }
    }
}

#[cfg(test)]
mod timeout_tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A server that reads the request, stays silent for `quiet`, then
    /// answers with a tiny 200. The silence stands in for a generation that
    /// has not produced its first byte yet.
    async fn quiet_server(quiet: Duration) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let _ = sock.read(&mut buf).await;
                    tokio::time::sleep(quiet).await;
                    let _ = sock
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                        )
                        .await;
                });
            }
        });
        format!("http://{addr}/")
    }

    /// The guarantee, measured against a socket rather than a builder field:
    /// a non-streaming request survives silence longer than the stall bound
    /// (the generation is one read, and only the exchange is capped), while
    /// a streaming request on the same silent socket fails at the stall
    /// bound.
    #[tokio::test]
    async fn silence_before_the_head_is_a_stall_only_for_a_stream() {
        let url = quiet_server(Duration::from_millis(600)).await;
        let clients = HttpClients::with(
            Duration::from_secs(5),
            Duration::from_millis(200),
            Duration::from_secs(5),
        )
        .unwrap();

        let whole = clients.whole.post(&url).body("{}").send().await;
        assert!(
            whole.is_ok(),
            "a non-streaming request must not be bounded per read: {whole:?}"
        );

        let stream = clients.stream.post(&url).body("{}").send().await;
        let err = stream.expect_err("a stream that goes quiet past the stall bound must fail");
        assert!(err.is_timeout(), "{err}");
    }

    /// And the whole-exchange cap still exists: a socket quieter than the
    /// cap fails the non-streaming request.
    #[tokio::test]
    async fn a_non_streaming_request_still_has_an_exchange_cap() {
        let url = quiet_server(Duration::from_millis(800)).await;
        let clients = HttpClients::with(
            Duration::from_secs(5),
            Duration::from_secs(5),
            Duration::from_millis(200),
        )
        .unwrap();
        let err = clients
            .whole
            .post(&url)
            .body("{}")
            .send()
            .await
            .expect_err("the exchange cap must fire");
        assert!(err.is_timeout(), "{err}");
    }

    #[test]
    fn the_body_picks_the_client() {
        let c = HttpClients::plain();
        assert!(std::ptr::eq(
            c.for_body(&serde_json::json!({"stream": true})),
            &c.stream
        ));
        assert!(std::ptr::eq(c.for_body(&serde_json::json!({})), &c.whole));
        assert!(std::ptr::eq(
            c.for_body(&serde_json::json!({"stream": false})),
            &c.whole
        ));
    }
}

impl Failover {
    pub fn new(primary: Box<dyn Provider>, fallbacks: Vec<(String, Box<dyn Provider>)>) -> Self {
        Failover { primary, fallbacks }
    }
}

fn failover_worthy(e: &anyhow::Error) -> bool {
    e.downcast_ref::<retry::ProviderError>()
        .is_some_and(retry::ProviderError::transient)
}

#[async_trait]
impl Provider for Failover {
    fn id(&self) -> &str {
        self.primary.id()
    }

    fn default_model(&self) -> &str {
        self.primary.default_model()
    }

    /// The primary's answer, like `id` and `default_model`. Inheriting the
    /// trait default (`false`) meant configuring any `fallbacks` silently
    /// turned every attached image into a `[image: …]` placeholder — a
    /// feature that degraded another as a side effect, with nothing in the
    /// transcript to say why.
    fn vision(&self) -> bool {
        self.primary.vision()
    }

    async fn complete(
        &self,
        req: &CompletionRequest,
        sink: Option<&StreamSink>,
    ) -> Result<CompletionResponse> {
        let mut last = match self.primary.complete(req, sink).await {
            Ok(response) => return Ok(response),
            Err(e) if failover_worthy(&e) => e,
            Err(e) => return Err(e),
        };

        for (name, provider) in &self.fallbacks {
            tracing::warn!(
                error = %last,
                fallback = %name,
                "provider failed transiently after retries; falling back"
            );
            let fb_req = CompletionRequest {
                model: provider.default_model().to_string(),
                ..req.clone()
            };
            match provider.complete(&fb_req, sink).await {
                Ok(response) => return Ok(response),
                Err(e) if failover_worthy(&e) => last = e,
                Err(e) => return Err(e),
            }
        }
        Err(last.context(format!(
            "the primary and {} fallback(s) all failed transiently",
            self.fallbacks.len()
        )))
    }
}

#[cfg(test)]
mod failover_tests {
    use super::*;
    use crate::message::{Block, Message, StopReason};
    use retry::ProviderError;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// Fails every call with the given error; counts how often it was asked.
    struct Failing {
        error: fn() -> anyhow::Error,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Provider for Failing {
        fn id(&self) -> &str {
            "failing"
        }
        fn default_model(&self) -> &str {
            "primary-model"
        }
        async fn complete(
            &self,
            _req: &CompletionRequest,
            _sink: Option<&StreamSink>,
        ) -> Result<CompletionResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err((self.error)())
        }
    }

    /// Answers, and records the model name it was asked for.
    struct Recording {
        model_seen: Arc<Mutex<Option<String>>>,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Provider for Recording {
        fn id(&self) -> &str {
            "recording"
        }
        fn default_model(&self) -> &str {
            "fallback-model"
        }
        async fn complete(
            &self,
            req: &CompletionRequest,
            _sink: Option<&StreamSink>,
        ) -> Result<CompletionResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.model_seen.lock().unwrap() = Some(req.model.clone());
            Ok(CompletionResponse {
                message: Message::assistant(vec![Block::text("from the fallback")]),
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
                refusal: None,
                model: "fallback-model".into(),
                malformed_tool_args: 0,
            })
        }
    }

    fn req() -> CompletionRequest {
        CompletionRequest {
            model: "primary-model".into(),
            system: None,
            messages: vec![Message::user("hi")],
            tools: Vec::new(),
            max_tokens: 64,
            effort: None,
            thinking: false,
            cache_prompt: false,
        }
    }

    type Rig = (
        Failover,
        Arc<AtomicUsize>,
        Arc<Mutex<Option<String>>>,
        Arc<AtomicUsize>,
    );

    fn rig(error: fn() -> anyhow::Error) -> Rig {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let model_seen = Arc::new(Mutex::new(None));
        let failover = Failover::new(
            Box::new(Failing {
                error,
                calls: Arc::clone(&primary_calls),
            }),
            vec![(
                "small".into(),
                Box::new(Recording {
                    model_seen: Arc::clone(&model_seen),
                    calls: Arc::clone(&fallback_calls),
                }) as Box<dyn Provider>,
            )],
        );
        (failover, primary_calls, model_seen, fallback_calls)
    }

    #[tokio::test]
    async fn a_transient_exhaustion_falls_back_and_the_fallback_answers_as_itself() {
        let (failover, _, model_seen, _) =
            rig(|| anyhow::Error::new(ProviderError::Overloaded).context("anthropic 529: busy"));

        let response = failover.complete(&req(), None).await.unwrap();

        assert_eq!(response.message.text(), "from the fallback");
        // The recorded bug this guards: sending one server's model name to
        // another server. The fallback must be asked for its own model.
        assert_eq!(
            model_seen.lock().unwrap().as_deref(),
            Some("fallback-model")
        );
    }

    #[tokio::test]
    async fn terminal_classes_never_fall_back() {
        // An invalid request fails identically everywhere; auth is the
        // primary's problem, not a routing decision. The fallback must not
        // even be consulted.
        for error in [
            (|| anyhow::Error::new(ProviderError::Invalid("bad".into()))) as fn() -> anyhow::Error,
            || anyhow::Error::new(ProviderError::Auth),
            || anyhow::Error::new(ProviderError::ContextOverflow),
        ] {
            let (failover, primary_calls, _, fallback_calls) = rig(error);
            let err = failover.complete(&req(), None).await.unwrap_err();
            assert!(err.downcast_ref::<ProviderError>().is_some());
            assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                fallback_calls.load(Ordering::SeqCst),
                0,
                "the fallback was consulted"
            );
        }
    }

    /// Sees images; never called.
    struct Sighted;

    #[async_trait]
    impl Provider for Sighted {
        fn id(&self) -> &str {
            "sighted"
        }
        fn default_model(&self) -> &str {
            "sighted-model"
        }
        fn vision(&self) -> bool {
            true
        }
        async fn complete(
            &self,
            _req: &CompletionRequest,
            _sink: Option<&StreamSink>,
        ) -> Result<CompletionResponse> {
            unreachable!("vision() is a static property")
        }
    }

    /// Wrapping a vision-capable primary in a `Failover` must not blind it:
    /// `Agent::vision` asks the outermost provider, and the trait default is
    /// `false`.
    #[test]
    fn a_failover_sees_what_its_primary_sees() {
        let failover = Failover::new(Box::new(Sighted), vec![]);
        assert!(failover.vision());
        let blind = Failover::new(
            Box::new(Failing {
                error: || anyhow::anyhow!("never called"),
                calls: Arc::new(AtomicUsize::new(0)),
            }),
            vec![],
        );
        assert!(!blind.vision(), "and it invents nothing the primary lacks");
    }

    #[tokio::test]
    async fn an_unclassified_error_never_falls_back_because_it_may_be_mid_stream() {
        // Errors without a ProviderError in the chain are mid-stream failures:
        // deltas may already be on the user's screen, and a fallback would
        // replay half an answer as a whole one.
        let (failover, _, _, fallback_calls) = rig(|| anyhow::anyhow!("stream aborted mid-body"));

        let err = failover.complete(&req(), None).await.unwrap_err();
        assert!(err.to_string().contains("stream aborted"));
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 0);
    }
}
