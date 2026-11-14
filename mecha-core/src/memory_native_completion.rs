//! Public-shape-only completion for explicit synthetic native sessions.
//! No semantic checker or expected artifact values are available to this module.

use crate::{
    agent::{
        CompletionArtifactIssue, CompletionFeedback, CompletionObservation, CompletionObserver,
        CompletionReadiness, CompletionReport,
    },
    memory_experience_bridge::read_completion_artifact,
    tool::goal_memory::{NativeRecallHandle, NativeRecallReport, NativeRecallStatus},
};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

const PROTOCOL: &str = "native-completion-feedback-v1";

/// Host-authored public output shape. Hidden semantic check rows are not inputs.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCompletionRequest {
    pub rows: usize,
    pub keys: Vec<String>,
}

/// Validated shape; construction is only through NativeCompletionRequest.
#[derive(Clone, Debug)]
pub struct NativeCompletionConfig {
    rows: usize,
    keys: Vec<String>,
    shape_digest: String,
}
impl NativeCompletionRequest {
    pub fn into_config(mut self) -> Result<NativeCompletionConfig> {
        ensure!(
            (1..=32).contains(&self.rows),
            "invalid native completion row count"
        );
        ensure!(
            (1..=16).contains(&self.keys.len()),
            "invalid native completion key count"
        );
        ensure!(
            self.keys.iter().all(|key| {
                (1..=64).contains(&key.len())
                    && key
                        .bytes()
                        .next()
                        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                    && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            }),
            "invalid native completion identifier"
        );
        self.keys.sort();
        ensure!(
            !self.keys.windows(2).any(|pair| pair[0] == pair[1]),
            "duplicate native completion key"
        );
        let bytes = serde_json::to_vec(&(PROTOCOL, self.rows, &self.keys))?;
        let shape_digest = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(NativeCompletionConfig {
            rows: self.rows,
            keys: self.keys,
            shape_digest,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct NativeCompletionReport {
    pub protocol: &'static str,
    pub shape_digest: String,
    pub report: CompletionReport,
}
impl NativeCompletionReport {
    pub fn protocol_satisfied(&self) -> bool {
        self.report
            .latest
            .as_ref()
            .is_some_and(|observation| observation.readiness == CompletionReadiness::Ready)
    }
}

/// Fresh per invocation, retained outside execution so later errors cannot erase
/// the last report the loop actually accepted. Async observations never write it.
#[derive(Clone)]
pub(crate) struct NativeCompletion {
    config: NativeCompletionConfig,
    report: Arc<Mutex<CompletionReport>>,
}
impl NativeCompletion {
    pub(crate) fn new(config: NativeCompletionConfig) -> Self {
        Self {
            config,
            report: Arc::new(Mutex::new(CompletionReport::default())),
        }
    }
    pub(crate) fn snapshot(&self) -> NativeCompletionReport {
        NativeCompletionReport {
            protocol: PROTOCOL,
            shape_digest: self.config.shape_digest.clone(),
            report: self
                .report
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        }
    }
    pub(crate) fn observer(
        &self,
        artifact: PathBuf,
        recall: Option<NativeRecallHandle>,
    ) -> Arc<dyn CompletionObserver> {
        Arc::new(NativeObserver {
            completion: self.clone(),
            artifact,
            recall,
        })
    }
}
struct NativeObserver {
    completion: NativeCompletion,
    artifact: PathBuf,
    recall: Option<NativeRecallHandle>,
}

#[async_trait::async_trait]
impl CompletionObserver for NativeObserver {
    async fn observe(&self) -> CompletionObservation {
        let path = self.artifact.clone();
        let config = self.completion.config.clone();
        // A dropped observer future cannot publish a late read. Blocking I/O is
        // off the async executor and only returns data; the loop owns acceptance.
        let artifact = tokio::task::spawn_blocking(move || artifact_shape(&path, &config)).await;
        let Ok(artifact) = artifact else {
            return unavailable();
        };
        readiness(
            artifact,
            self.recall
                .as_ref()
                .map(NativeRecallHandle::report)
                .as_ref(),
        )
    }
    fn record(&self, report: &CompletionReport) {
        *self
            .completion
            .report
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = report.clone();
    }
}
fn unavailable() -> CompletionObservation {
    CompletionObservation {
        readiness: CompletionReadiness::Unavailable,
        feedback: None,
    }
}
fn artifact_shape(
    path: &std::path::Path,
    config: &NativeCompletionConfig,
) -> Option<Option<CompletionArtifactIssue>> {
    let bytes = match read_completion_artifact(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Some(Some(CompletionArtifactIssue::Missing))
        }
        Err(_) => return None,
    };
    let valid = serde_json::from_slice::<crate::memory::plan_extraction::UniqueValue>(&bytes)
        .ok()
        .and_then(|value| serde_json::from_value::<Vec<BTreeMap<String, String>>>(value.0).ok())
        .is_some_and(|rows| {
            rows.len() == config.rows && rows.iter().all(|row| row.keys().eq(config.keys.iter()))
        });
    Some((!valid).then_some(CompletionArtifactIssue::Malformed))
}
fn readiness(
    artifact: Option<Option<CompletionArtifactIssue>>,
    recall: Option<&NativeRecallReport>,
) -> CompletionObservation {
    let Some(artifact) = artifact else {
        return unavailable();
    };
    let read_satisfied = recall.is_none_or(NativeRecallReport::protocol_satisfied);
    let required_read_missing = recall.is_some_and(|report| {
        report.status == NativeRecallStatus::NotCalled
            && report.entered_calls == 0
            && report.admitted_calls == 0
            && report.rejected_calls == 0
    });
    if artifact.is_none() && read_satisfied {
        return CompletionObservation {
            readiness: CompletionReadiness::Ready,
            feedback: None,
        };
    }
    CompletionObservation {
        readiness: CompletionReadiness::Incomplete,
        feedback: (required_read_missing || artifact.is_some()).then_some(CompletionFeedback {
            required_read_missing,
            artifact,
            do_not_repeat_read: recall.is_some() && !required_read_missing,
        }),
    }
}

#[cfg(test)]
#[path = "memory_native_completion_tests.rs"]
mod tests;
