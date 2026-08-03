//! Model providers.
//!
//! A provider translates [`CompletionRequest`] onto some wire protocol and
//! translates the reply back into [`CompletionResponse`]. Everything above this
//! layer — the agent loop, tools, sessions — is provider-agnostic.

pub mod anthropic;
pub mod openai;

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
    ToolUseStart { name: String },
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
        other => anyhow::bail!(
            "unknown provider kind {other:?} (expected: anthropic, openai, local)"
        ),
    }
}
