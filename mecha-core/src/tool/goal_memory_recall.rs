//! Host-owned, single-call retrieval control for explicit native experiments.
use super::*;
use crate::{
    embed::{Embedder, EmbeddingInput, EncodingProfile, Task},
    memory::{
        hybrid::{
            HybridFallback, HybridMode, HybridOptions, HybridReadiness, HybridResult, HybridVectors,
        },
        RevisionRef,
    },
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

const RETRIEVAL_DEADLINE: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeRecallMode {
    Lexical,
    Dense,
}

#[derive(Clone, Debug, Serialize)]
pub struct NativeRecallIdentity {
    pub protocol: String,
    pub mode: NativeRecallMode,
    pub query_digest: String,
    pub profile_id: Option<String>,
}

/// Validated host inputs. Neither endpoint nor profile comes from tool arguments.
#[derive(Clone)]
pub struct NativeRecallConfig {
    query: String,
    dense: Option<(Embedder, EncodingProfile)>,
}
impl NativeRecallConfig {
    pub fn lexical(query: String) -> Result<Self> {
        ensure!(
            !query.trim().is_empty() && query.len() <= 4096,
            "native recall query must be nonempty and at most 4096 bytes"
        );
        crate::memory::validate_recall_query(&query)?;
        Ok(Self { query, dense: None })
    }
    pub fn dense(query: String, endpoint: String, profile: EncodingProfile) -> Result<Self> {
        let mut config = Self::lexical(query)?;
        ensure!(
            crate::provider::router::is_literal_loopback(&endpoint),
            "native recall requires literal loopback HTTP without credentials, query or fragment"
        );
        config.dense = Some((Embedder::new(&endpoint)?, profile));
        Ok(config)
    }
    pub fn identity(&self) -> NativeRecallIdentity {
        NativeRecallIdentity {
            protocol: "native-recall-v1".into(),
            mode: if self.dense.is_some() {
                NativeRecallMode::Dense
            } else {
                NativeRecallMode::Lexical
            },
            query_digest: Sha256::digest(self.query.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            profile_id: self.dense.as_ref().map(|(_, p)| p.id().into()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeRecallStatus {
    NotCalled,
    Running,
    Rejected,
    Completed,
    Fallback,
    Failed,
    Cancelled,
    TimedOut,
    Interrupted,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct NativeRecallEmbeddingCost {
    /// None means no complete transport accounting was available.
    pub inputs: Option<usize>,
    pub attempts: Option<usize>,
    pub request_bytes: Option<usize>,
    pub response_bytes: Option<usize>,
    pub server_reported_alias: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct NativeRecallAnchor {
    pub reference: RevisionRef,
    pub rank: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct NativeRecallReport {
    pub identity: NativeRecallIdentity,
    /// Calls entering GoalMemory, not earlier Agent dispatch refusals.
    pub entered_calls: usize,
    pub admitted_calls: usize,
    pub rejected_calls: usize,
    pub status: NativeRecallStatus,
    pub effective_mode: Option<NativeRecallMode>,
    pub fallback: Option<HybridFallback>,
    pub delivered: Vec<NativeRecallAnchor>,
    pub elapsed_micros: u128,
    pub embedding: NativeRecallEmbeddingCost,
}
impl NativeRecallReport {
    pub fn protocol_satisfied(&self) -> bool {
        self.entered_calls == 1
            && self.admitted_calls == 1
            && self.rejected_calls == 0
            && self.status == NativeRecallStatus::Completed
            && self.effective_mode == Some(self.identity.mode)
            && self.fallback.is_none()
    }
}
#[derive(Clone)]
pub struct NativeRecallHandle(Arc<Mutex<NativeRecallReport>>);
impl NativeRecallHandle {
    pub fn report(&self) -> NativeRecallReport {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

pub(super) struct NativeRecallControl {
    config: NativeRecallConfig,
    handle: NativeRecallHandle,
}
impl NativeRecallControl {
    pub(super) fn query(&self) -> &str {
        &self.config.query
    }
    fn new(config: NativeRecallConfig) -> Self {
        let report = NativeRecallReport {
            identity: config.identity(),
            entered_calls: 0,
            admitted_calls: 0,
            rejected_calls: 0,
            status: NativeRecallStatus::NotCalled,
            effective_mode: None,
            fallback: None,
            delivered: vec![],
            elapsed_micros: 0,
            embedding: NativeRecallEmbeddingCost::default(),
        };
        Self {
            config,
            handle: NativeRecallHandle(Arc::new(Mutex::new(report))),
        }
    }
    fn claim(&self) -> Option<Attempt> {
        let mut report = self.handle.0.lock().unwrap_or_else(|e| e.into_inner());
        report.entered_calls = report.entered_calls.saturating_add(1);
        if report.entered_calls != 1 {
            report.rejected_calls = report.rejected_calls.saturating_add(1);
            return None;
        }
        report.status = NativeRecallStatus::Running;
        Some(Attempt {
            handle: self.handle.clone(),
            started: Instant::now(),
            finished: false,
        })
    }
}
struct Attempt {
    handle: NativeRecallHandle,
    started: Instant,
    finished: bool,
}
impl Attempt {
    fn finish(&mut self, status: NativeRecallStatus) {
        let mut report = self.handle.0.lock().unwrap_or_else(|e| e.into_inner());
        report.status = status;
        report.elapsed_micros = self.started.elapsed().as_micros();
        self.finished = true;
    }
    fn reject(&mut self) {
        self.handle
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .rejected_calls += 1;
        self.finish(NativeRecallStatus::Rejected);
    }
}
impl Attempt {
    async fn complete<F>(
        &mut self,
        retrieval: F,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<ToolOutput>
    where
        F: std::future::Future<Output = Result<(ToolOutput, HybridResult)>>,
    {
        let deadline = self.started + RETRIEVAL_DEADLINE;
        let outcome = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(NativeRecallStatus::Cancelled),
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => Err(NativeRecallStatus::TimedOut),
            value = retrieval => value.map_err(|_| NativeRecallStatus::Failed),
        };
        // Assemble host metadata before the final publication check. A future
        // can return Ready in the same poll that expires/cancels this attempt.
        let outcome = outcome.map(|(output, result)| {
            let mode = match result.effective_mode {
                HybridMode::Lexical => NativeRecallMode::Lexical,
                HybridMode::Dense => NativeRecallMode::Dense,
                HybridMode::Hybrid => unreachable!(),
            };
            let anchors = result
                .ranked_anchors
                .into_iter()
                .map(|a| NativeRecallAnchor {
                    reference: a.reference,
                    rank: a.rank,
                })
                .collect();
            (output, mode, result.fallback, anchors)
        });
        let mut report = self.handle.0.lock().unwrap_or_else(|e| e.into_inner());
        // This is the publication linearization point, under the monitor lock,
        // with no await or fallible work between the check and publication.
        let outcome = if cancel.is_cancelled() {
            Err(NativeRecallStatus::Cancelled)
        } else if Instant::now() >= deadline {
            Err(NativeRecallStatus::TimedOut)
        } else {
            outcome
        };
        let output = match outcome {
            Err(status) => {
                report.status = status;
                ToolOutput::err(match status {
                    NativeRecallStatus::Cancelled => "native recall cancelled",
                    NativeRecallStatus::TimedOut => "native recall deadline exceeded",
                    _ => "native recall unavailable",
                })
            }
            Ok((output, mode, fallback, anchors)) => {
                report.effective_mode = Some(mode);
                report.fallback = fallback;
                report.delivered = anchors;
                report.status = if fallback.is_some() {
                    NativeRecallStatus::Fallback
                } else {
                    NativeRecallStatus::Completed
                };
                output
            }
        };
        report.elapsed_micros = self.started.elapsed().as_micros();
        self.finished = true;
        Ok(output)
    }
}
impl Drop for Attempt {
    fn drop(&mut self) {
        if !self.finished {
            self.finish(NativeRecallStatus::Interrupted);
        }
    }
}

impl GoalMemory {
    pub fn with_native_recall(
        mut self,
        config: NativeRecallConfig,
    ) -> Result<(Self, NativeRecallHandle)> {
        ensure!(
            self.interpretations.is_none(),
            "native controlled recall cannot combine interpretation reads"
        );
        ensure!(
            self.native_recall.is_none(),
            "native recall is already configured"
        );
        let control = NativeRecallControl::new(config);
        let handle = control.handle.clone();
        self.native_recall = Some(control);
        Ok((self, handle))
    }
    pub(super) async fn controlled_call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let control = self.native_recall.as_ref().expect("controlled route");
        let Some(mut attempt) = control.claim() else {
            return Ok(ToolOutput::err(
                "native recall permits only one attempted goal_context call",
            ));
        };
        let valid = input
            .as_object()
            .is_some_and(|o| o.keys().all(|k| k == "memory" || k == "serves"))
            && input
                .get("serves")
                .is_none_or(|g| g.as_str() == Some(self.situation.goal.as_str()))
            && ctx
                .goal_track
                .as_ref()
                .and_then(|g| g.anchor())
                .is_none_or(|g| g.to_string() == self.situation.goal)
            && input
                .get("memory")
                .and_then(Value::as_object)
                .is_some_and(|m| {
                    m.len() == 1
                        && m.get("query").and_then(Value::as_str)
                            == Some(control.config.query.as_str())
                });
        if !valid {
            attempt.reject();
            return Ok(ToolOutput::err("native recall requires the exact host query, confirmed goal and no bindings; this attempt consumed the call"));
        }
        control
            .handle
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .admitted_calls = 1;
        let cancel = ctx.cancel.clone().unwrap_or_default();
        let retrieval = async {
            let result = self.controlled_read(control).await?;
            let mut base_input = input;
            base_input["serves"] = json!(self.situation.goal);
            let base = GoalContext.call(base_input, ctx).await?;
            ensure!(!base.is_error, "goal context unavailable");
            let mut answer: Value = serde_json::from_str(&base.content)?;
            // Derived, as on the lexical route: outside when any delivered
            // record is untrusted, never because the packet is non-empty.
            let external = result.packet.records.iter().any(|hit| hit.untrusted_input);
            answer["memory"] = serde_json::to_value(&result.packet)?;
            let output = ToolOutput::ok(answer.to_string());
            Ok::<_, anyhow::Error>((
                if external {
                    output.from_outside()
                } else {
                    output
                },
                result,
            ))
        };
        attempt.complete(retrieval, &cancel).await
    }
    async fn controlled_read(&self, control: &NativeRecallControl) -> Result<HybridResult> {
        let request = ContextRequest {
            query: control.config.query.clone(),
            bindings: vec![],
        };
        let mut policy = self.policy.clone();
        policy.valid_at = Some(self.clock.now().to_rfc3339());
        let situation = self.situation.clone();
        let memory = self.memory.clone();
        let Some((embedder, profile)) = &control.config.dense else {
            let clock = self.clock.clone();
            return tokio::task::spawn_blocking(move || {
                let memory = memory
                    .lock()
                    .map_err(|_| anyhow::anyhow!("memory reader unavailable"))?;
                policy.valid_at = Some(clock.now().to_rfc3339());
                memory.lexical_hybrid_context(&request, &policy, &situation, None)
            })
            .await
            .context("memory read task failed")?;
        };
        let options = HybridOptions {
            profile_id: profile.id().into(),
            dimensions: profile.dimensions(),
            required_mention: None,
        };
        let prepared = tokio::task::spawn_blocking(move || {
            memory
                .lock()
                .map_err(|_| anyhow::anyhow!("memory reader unavailable"))?
                .prepare_hybrid_context(&request, &policy, &situation, &options)
        })
        .await
        .context("memory preparation task failed")??;
        let vectors = if prepared.readiness() == HybridReadiness::Ready {
            let inputs: Vec<_> = std::iter::once(EmbeddingInput {
                task: Task::Query,
                text: prepared.query().into(),
            })
            .chain(prepared.passages().iter().map(|p| EmbeddingInput {
                task: Task::Passage,
                text: p.text.clone(),
            }))
            .collect();
            control
                .handle
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .embedding
                .inputs = Some(inputs.len());
            match embedder.embed_checked(&inputs, profile).await {
                Ok(batch) => {
                    {
                        let mut report = control.handle.0.lock().unwrap_or_else(|e| e.into_inner());
                        report.embedding.attempts = Some(batch.attempts);
                        report.embedding.request_bytes = Some(batch.request_bytes);
                        report.embedding.response_bytes = Some(batch.response_bytes);
                        report.embedding.server_reported_alias = batch.server_model;
                    }
                    let mut vectors = batch.vectors.into_iter();
                    Some(HybridVectors {
                        profile_id: batch.profile_id,
                        query: vectors.next().unwrap_or_default(),
                        passages: vectors.collect(),
                    })
                }
                Err(_) => None,
            }
        } else {
            control
                .handle
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .embedding = NativeRecallEmbeddingCost {
                inputs: Some(0),
                attempts: Some(0),
                request_bytes: Some(0),
                response_bytes: Some(0),
                server_reported_alias: None,
            };
            None
        };
        let memory = self.memory.clone();
        let clock = self.clock.clone();
        tokio::task::spawn_blocking(move || {
            let memory = memory
                .lock()
                .map_err(|_| anyhow::anyhow!("memory reader unavailable"))?;
            let valid_at = clock.now().to_rfc3339();
            memory.finish_hybrid_context_at(
                &prepared,
                vectors.as_ref(),
                HybridMode::Dense,
                &valid_at,
            )
        })
        .await
        .context("memory finalization task failed")?
    }
}

#[cfg(test)]
#[path = "goal_memory_recall_tests.rs"]
mod tests;
