//! Model providers.
//!
//! A provider translates [`CompletionRequest`] onto some wire protocol and
//! translates the reply back into [`CompletionResponse`]. Everything above this
//! layer — the agent loop, tools, sessions — is provider-agnostic.

pub mod anthropic;
pub mod openai;
pub mod retry;
pub(crate) mod sse;

use crate::message::{CompletionRequest, CompletionResponse, Usage};
use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc::UnboundedSender;

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
