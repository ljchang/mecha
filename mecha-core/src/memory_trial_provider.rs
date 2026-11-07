//! Strict resident-only provider for bounded synthetic bridge trials.
//!
//! Every inference checks the resident under the shared router hold and sends
//! `autoload=false`, so even an uncoordinated unload cannot reload the model.
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{ensure, Context, Result};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::config::ProviderConfig;
use crate::hold::{Held, Holds};
use crate::message::{CompletionRequest, CompletionResponse};
use crate::provider::{openai::OpenAiCompatible, router, Provider, StreamSink};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// Explicit host inputs; never loaded from global or project configuration.
pub struct LocalTrialConfig {
    pub endpoint: String,
    pub model: String,
    pub holds_dir: PathBuf,
    pub seed: u64,
}

/// A rejected response remains available to the host audit recorder.
#[derive(Debug)]
pub struct ReturnedModelMismatch {
    pub expected: String,
    pub response: CompletionResponse,
}
impl std::fmt::Display for ReturnedModelMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "local trial returned model {:?}, expected {:?}",
            self.response.model, self.expected
        )
    }
}
impl std::error::Error for ReturnedModelMismatch {}

pub struct LocalTrialProvider {
    inner: OpenAiCompatible,
    endpoint: String,
    model: String,
    cancel: CancellationToken,
    // Held until the provider (and therefore the trial) ends, including errors.
    _held: Held,
}
impl LocalTrialProvider {
    pub async fn new(config: LocalTrialConfig, cancel: CancellationToken) -> Result<Self> {
        ensure!(
            router::is_literal_loopback(&config.endpoint),
            "explicit numeric loopback HTTP endpoint required"
        );
        let url = reqwest::Url::parse(&config.endpoint)?;
        ensure!(
            matches!(url.path(), "" | "/"),
            "local trial endpoint must be an origin without a path"
        );
        ensure!(!config.model.trim().is_empty(), "explicit model required");
        ensure!(
            config.holds_dir.is_absolute(),
            "explicit absolute shared holds directory required"
        );
        let endpoint = router::base(&config.endpoint);
        let holds = Holds::new(config.holds_dir);
        let held = tokio::select! {
            biased;
            () = cancel.cancelled() => anyhow::bail!("local trial cancelled before taking model hold"),
            held = tokio::time::timeout(REQUEST_TIMEOUT, holds.hold_when_clear(&endpoint, "synthetic memory bridge trial", |_| {})) => held.context("local trial model hold timed out after 180s")??,
        };
        let hold_cancel = cancel.clone();
        held.on_cancel(move || hold_cancel.cancel());
        let inner = OpenAiCompatible::from_local_config(&ProviderConfig {
            kind: "local".into(),
            base_url: Some(endpoint.clone()),
            model: Some(config.model.clone()),
            temperature: Some(0.6),
            seed: Some(config.seed),
            max_retries: Some(0),
            ..Default::default()
        })?
        .without_autoload();
        let provider = Self {
            inner,
            endpoint,
            model: config.model,
            cancel,
            _held: held,
        };
        tokio::select! {
            biased;
            () = provider.cancel.cancelled() => anyhow::bail!("local trial cancelled during resident check"),
            result = provider.check_resident() => result?,
        }
        Ok(provider)
    }

    async fn check_resident(&self) -> Result<()> {
        let models = router::local_models(&self.endpoint)
            .await
            .context("could not verify local router resident model")?;
        ensure!(
            router::resident(&models) == Some(self.model.as_str()),
            "local trial resident model {:?} does not match expected {:?}",
            router::resident(&models),
            self.model
        );
        Ok(())
    }
}

#[async_trait]
impl Provider for LocalTrialProvider {
    fn id(&self) -> &str {
        "local-trial"
    }
    fn default_model(&self) -> &str {
        &self.model
    }
    fn local(&self) -> bool {
        true
    }
    fn wire_body(&self, req: &CompletionRequest, stream: bool) -> Option<serde_json::Value> {
        self.inner.wire_body(req, stream)
    }
    async fn complete(
        &self,
        req: &CompletionRequest,
        sink: Option<&StreamSink>,
    ) -> Result<CompletionResponse> {
        ensure!(
            req.model == self.model,
            "local trial request cannot switch models"
        );
        tokio::select! {
            biased;
            () = self.cancel.cancelled() => anyhow::bail!("local trial cancelled"),
            result = tokio::time::timeout(REQUEST_TIMEOUT, async {
                self.check_resident().await?;
                // The recorder applied fast/local-only settings before auditing.
                // Never rewrite the request after its effective body was recorded.
                let response = self.inner.complete(req, sink).await?;
                if response.model != self.model {
                    return Err(ReturnedModelMismatch { expected:self.model.clone(),response }.into());
                }
                Ok(response)
            }) => result.context("local trial request timed out after 180s")?,
        }
    }
}

#[cfg(test)]
#[path = "memory_trial_provider_tests.rs"]
mod tests;
