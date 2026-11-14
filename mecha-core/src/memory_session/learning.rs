//! Host-only, one-call native concept learning over authenticated ordinary captures.
use super::*;
use crate::memory::{
    concept::{
        rumination::{ConceptRuminationRequest, RuminationOutcome},
        ConceptExcerpt, ConceptGeneration, ConceptRequest,
    },
    model_evaluation::ModelReply,
    RevisionRef,
};
use anyhow::Context;
use std::{future::Future, time::Instant};

pub const LEARNING_PROTOCOL: &str = "ordinary-session-learning-v1";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginalSelection {
    pub run_id: String,
    pub original_index: usize,
}
#[derive(Clone, Debug, Serialize)]
pub struct OriginalDescriptor {
    pub source: OriginalSelection,
    pub reference: RevisionRef,
    pub kind: PassageKind,
    pub line: u64,
    pub field: String,
    pub text: String,
    pub tool: Option<ActionInvocation>,
    pub receipt_status: Option<ReceiptStatus>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Opposition {
    pub source: OriginalSelection,
    pub quote: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LearningOperation {
    Induce {
        sources: Vec<OriginalSelection>,
    },
    Ruminate {
        expected: RevisionRef,
        counterexamples: Vec<Opposition>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearningInput {
    pub protocol: String,
    pub operation: LearningOperation,
}
#[derive(Debug, Serialize)]
pub struct GenerationAudit {
    pub prompt: String,
    pub response_schema: serde_json::Value,
}
#[derive(Debug, Serialize)]
pub struct LearningReport {
    pub protocol: &'static str,
    pub input: LearningInput,
    pub generation: Option<GenerationAudit>,
    pub callback_invoked: bool,
    pub reply: Option<ModelReply>,
    pub inference_error: Option<String>,
    pub status: String,
    pub failure_stage: Option<String>,
    pub error: Option<String>,
    pub retained: Option<RevisionRef>,
    pub rumination: Option<RuminationOutcome>,
    pub elapsed_ms: u128,
}
enum Prepared {
    Induce(ConceptRequest),
    Ruminate(ConceptRuminationRequest),
}
impl Prepared {
    fn generation(&self) -> ConceptGeneration {
        match self {
            Self::Induce(r) => r.generation(),
            Self::Ruminate(r) => r.generation(),
        }
    }
}
/// Opaque dispatch guard: transports recheck it after awaited discovery, before POST.
#[derive(Clone)]
pub struct LearningGuard {
    request: SessionMemoryRequest,
    workspace: PathBuf,
    storage: StorageAccess,
    authority: Arc<Authority>,
    prepared: Arc<Prepared>,
}
impl LearningGuard {
    pub fn check(&self) -> Result<()> {
        authority::exclude_workspace(&self.request, &self.workspace)?;
        let session = SessionMemory {
            request: self.request.clone(),
            workspace: Some(self.workspace.clone()),
            storage: self.storage.clone(),
            authority: self.authority.clone(),
        };
        let memory = session.memory()?;
        match self.prepared.as_ref() {
            Prepared::Induce(r) => memory.check_native_concept_request(r, &session.access()),
            Prepared::Ruminate(r) => memory.check_native_rumination_request(r, &session.access()),
        }
    }
}
impl SessionMemory {
    pub fn load_learning_input(path: &Path, workspace: &Path) -> Result<LearningInput> {
        load_input(path, workspace)
    }
    pub fn load_learning_selections(
        path: &Path,
        workspace: &Path,
    ) -> Result<Vec<OriginalSelection>> {
        load_input(path, workspace)
    }
    /// Bounded inventory for host selection. Includes original tool call binding.
    pub fn learning_run_sources(&self, run_id: &str) -> Result<Vec<OriginalDescriptor>> {
        self.execution_workspace()?;
        let (seal, _) = self.authority.seal(run_id)?;
        ensure!(
            seal.payload.originals.len() <= 16,
            "ordinary learning inventory exceeds sixteen originals"
        );
        self.learning_sources(
            &(0..seal.payload.originals.len())
                .map(|original_index| OriginalSelection {
                    run_id: run_id.into(),
                    original_index,
                })
                .collect::<Vec<_>>(),
        )
    }
    pub fn learning_sources(
        &self,
        sources: &[OriginalSelection],
    ) -> Result<Vec<OriginalDescriptor>> {
        let workspace = self.execution_workspace()?;
        authority::exclude_workspace(&self.request, workspace)?;
        ensure!(
            !sources.is_empty() && sources.len() <= 16,
            "ordinary learning requires one to sixteen selected originals"
        );
        let memory = self.memory()?;
        let mut descriptors = Vec::new();
        let mut sealed = Vec::new();
        for source in sources {
            let (mut seal, revision) = self.authority.seal(&source.run_id)?;
            self.authority.check_seal(&source.run_id, &seal)?;
            let frozen = self.authority.frozen(&source.run_id)?;
            authority::exclude_path(&frozen.source, workspace)?;
            seal.payload.binding.revision = revision;
            let receipt = memory.retained_ordinary_capture(&seal.payload, &self.access())?;
            let passage = seal
                .payload
                .originals
                .get(source.original_index)
                .context("ordinary original index unavailable")?;
            let reference = receipt
                .originals
                .get(source.original_index)
                .context("ordinary original reference unavailable")?
                .clone();
            let (tool, receipt_status) = tool_binding(&frozen, passage, &seal)?;
            descriptors.push(OriginalDescriptor {
                source: source.clone(),
                reference,
                kind: passage.kind.clone(),
                line: passage.line,
                field: passage.pointer.clone(),
                text: passage.text.clone(),
                tool,
                receipt_status,
            });
            self.authority.check_seal(&source.run_id, &seal)?;
            sealed.push((source.run_id.clone(), seal, receipt));
        }
        ensure!(
            serde_json::to_vec(&descriptors)?.len() <= 32_768,
            "ordinary learning descriptors exceed 32 KiB"
        );
        // Selected originals are an inventory, not a complete recall packet.
        // Reauthorize each canonical receipt and its full dependency closure
        // after assembling every descriptor without requiring dependencies in
        // the selected-original output itself.
        for (run_id, seal, expected) in sealed {
            self.authority.check_seal(&run_id, &seal)?;
            ensure!(
                memory.retained_ordinary_capture(&seal.payload, &self.access())? == expected,
                "ordinary learning receipt changed during inventory"
            );
            self.authority.check_seal(&run_id, &seal)?;
        }
        Ok(descriptors)
    }
    fn prepare_learning(&self, input: &LearningInput) -> Result<LearningGuard> {
        let workspace = self.execution_workspace()?.to_path_buf();
        ensure!(
            input.protocol == LEARNING_PROTOCOL && serde_json::to_vec(input)?.len() <= 65_536,
            "ordinary learning input unavailable"
        );
        let memory = self.memory()?;
        let snapshot = memory.snapshot()?;
        let prepared = match &input.operation {
            LearningOperation::Induce { sources } => {
                ensure!(
                    (2..=8).contains(&sources.len()),
                    "induction requires two to eight originals"
                );
                let selected = self.learning_sources(sources)?;
                Prepared::Induce(
                    memory.prepare_native_concept(
                        &selected
                            .into_iter()
                            .map(|d| d.reference)
                            .collect::<Vec<_>>(),
                        &snapshot,
                        &self.access(),
                    )?,
                )
            }
            LearningOperation::Ruminate {
                expected,
                counterexamples,
            } => {
                ensure!(
                    !counterexamples.is_empty() && counterexamples.len() <= 8,
                    "rumination requires one to eight opposing excerpts"
                );
                let selected = self.learning_sources(
                    &counterexamples
                        .iter()
                        .map(|c| c.source.clone())
                        .collect::<Vec<_>>(),
                )?;
                let excerpts = selected
                    .into_iter()
                    .zip(counterexamples)
                    .map(|(d, c)| {
                        ensure!(
                            !c.quote.is_empty() && c.quote.len() <= 2048,
                            "opposition quote unavailable"
                        );
                        let mut positions = d.text.match_indices(&c.quote);
                        let (start, _) = positions.next().context("opposition quote absent")?;
                        // Also reject overlapping duplicates; match_indices skips overlaps.
                        ensure!(
                            d.text
                                .as_bytes()
                                .windows(c.quote.len())
                                .filter(|bytes| *bytes == c.quote.as_bytes())
                                .count()
                                == 1,
                            "opposition quote ambiguous"
                        );
                        Ok(ConceptExcerpt {
                            evidence: d.reference,
                            field: d.field,
                            start,
                            end: start + c.quote.len(),
                            quote: c.quote.clone(),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                Prepared::Ruminate(memory.prepare_native_concept_rumination(
                    expected,
                    &excerpts,
                    &snapshot,
                    &self.access(),
                )?)
            }
        };
        Ok(LearningGuard {
            request: self.request.clone(),
            workspace,
            storage: self.storage.clone(),
            authority: self.authority.clone(),
            prepared: Arc::new(prepared),
        })
    }
    /// Exactly one callback, never a transaction across inference. Audit survives refusal.
    pub async fn learn<F, Fut>(&self, input: LearningInput, complete: F) -> LearningReport
    where
        F: FnOnce(ConceptGeneration, LearningGuard) -> Fut,
        Fut: Future<Output = Result<ModelReply>>,
    {
        let start = Instant::now();
        let mut report = LearningReport {
            protocol: LEARNING_PROTOCOL,
            input,
            generation: None,
            callback_invoked: false,
            reply: None,
            inference_error: None,
            status: "failed".into(),
            failure_stage: None,
            error: None,
            retained: None,
            rumination: None,
            elapsed_ms: 0,
        };
        let run = async {
            let guard = self
                .prepare_learning(&report.input)
                .map_err(|e| ("prepare", e))?;
            guard.check().map_err(|e| ("recheck", e))?;
            let generation = guard.prepared.generation();
            report.generation = Some(GenerationAudit {
                prompt: generation.prompt.clone(),
                response_schema: generation.response_schema.clone(),
            });
            report.callback_invoked = true;
            match complete(generation, guard.clone()).await {
                Ok(reply) => {
                    report.inference_error = reply.error.clone();
                    report.reply = Some(reply);
                }
                Err(error) => report.inference_error = Some(format!("{error:#}")),
            }
            // This precedes response parsing and also runs on transport/provider failure.
            guard.check().map_err(|e| ("recheck", e))?;
            if let Some(error) = &report.inference_error {
                return Err(("inference", anyhow::anyhow!(error.clone())));
            }
            let reply = report
                .reply
                .as_ref()
                .ok_or_else(|| ("inference", anyhow::anyhow!("missing model reply")))?;
            let prepared = Arc::try_unwrap(guard.prepared).map_err(|_| {
                (
                    "retention",
                    anyhow::anyhow!("learning callback retained dispatch capability"),
                )
            })?;
            let mut memory = self.memory().map_err(|e| ("recheck", e))?;
            match prepared {
                Prepared::Induce(request) => {
                    report.retained = Some(
                        memory
                            .retain_native_concept(request, &reply.text, &self.access(), None)
                            .map_err(|e| ("retention", e))?,
                    );
                    report.status = "retained".into();
                }
                Prepared::Ruminate(request) => {
                    let outcome = memory
                        .finish_native_concept_rumination(request, &reply.text, &self.access())
                        .map_err(|e| ("retention", e))?;
                    report.status = match &outcome {
                        RuminationOutcome::RevisedProvisional { reference } => {
                            report.retained = Some(reference.clone());
                            "revised_provisional"
                        }
                        RuminationOutcome::Unchanged { .. } => "unchanged",
                        RuminationOutcome::Abstained { .. } => "abstained",
                    }
                    .into();
                    report.rumination = Some(outcome);
                }
            }
            Ok::<_, (&str, anyhow::Error)>(())
        }
        .await;
        if let Err((stage, error)) = run {
            report.failure_stage = Some(stage.into());
            report.error = Some(format!("{error:#}"));
        }
        report.elapsed_ms = start.elapsed().as_millis();
        report
    }
}
fn load_input<T: serde::de::DeserializeOwned + Serialize>(
    path: &Path,
    workspace: &Path,
) -> Result<T> {
    authority::request_path(path, workspace)?;
    let value: crate::memory::plan_extraction::UniqueValue =
        serde_json::from_slice(&authority::read(path, 65_536)?)?;
    let input: T = serde_json::from_value(value.0.clone())?;
    ensure!(
        serde_json::to_value(&input)? == value.0,
        "unknown or noncanonical learning input fields"
    );
    Ok(input)
}
fn tool_binding(
    frozen: &Frozen,
    passage: &OriginalPassage,
    seal: &Seal,
) -> Result<(Option<ActionInvocation>, Option<ReceiptStatus>)> {
    if !matches!(
        passage.kind,
        PassageKind::ActionInvocation | PassageKind::ToolResult
    ) {
        return Ok((None, None));
    }
    let bytes = authority::read(&frozen.source, authority::SOURCE_MAX)?;
    let records = authenticated_tool_records(&bytes, &seal.source_digest)?;
    let payload = &seal.payload;
    let Some(crate::session::Record::Message(message)) = records.get(
        usize::try_from(passage.line)?
            .checked_sub(1)
            .context("invalid original line")?,
    ) else {
        anyhow::bail!("original message unavailable")
    };
    let index = passage
        .pointer
        .split('/')
        .nth(2)
        .context("original block pointer unavailable")?
        .parse::<usize>()?;
    let id = match message.content.get(index) {
        Some(crate::message::Block::ToolUse { id, .. }) => id,
        Some(crate::message::Block::ToolResult { tool_use_id, .. }) => tool_use_id,
        _ => anyhow::bail!("original tool block unavailable"),
    };
    let actions = payload
        .originals
        .iter()
        .filter(|p| p.kind == PassageKind::ActionInvocation)
        .map(|p| serde_json::from_str::<ActionInvocation>(&p.text))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let action = actions
        .into_iter()
        .find(|a| a.call_id == *id)
        .context("original action unavailable")?;
    let receipt = payload
        .receipts
        .iter()
        .find(|r| r.call_id == *id && r.tool_name == action.tool_name)
        .context("original receipt unavailable")?;
    Ok((Some(action), Some(receipt.status.clone())))
}

fn authenticated_tool_records(
    bytes: &[u8],
    source_digest: &str,
) -> Result<Vec<crate::session::Record>> {
    ensure!(
        authority::digest(bytes) == source_digest,
        "ordinary tool source buffer changed"
    );
    recording::records(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        message::{Block, Message},
        session::Record,
    };

    #[test]
    fn tool_binding_authenticates_the_same_typed_buffer_it_parses() {
        let mut result = Message::user("");
        result.content = vec![Block::ToolResult {
            tool_use_id: "sealed-call".into(),
            content: "fixture readout".into(),
            is_error: false,
        }];
        let original = format!(
            "{}\n",
            serde_json::to_string(&Record::Message(result)).unwrap()
        );
        let digest = authority::digest(original.as_bytes());
        assert!(authenticated_tool_records(original.as_bytes(), &digest).is_ok());
        let changed = original.replace("sealed-call", "changed-call");
        assert!(
            recording::records(changed.as_bytes()).is_ok(),
            "replacement must remain a valid typed native record"
        );
        assert!(authenticated_tool_records(changed.as_bytes(), &digest)
            .unwrap_err()
            .to_string()
            .contains("source buffer changed"));
        assert!(
            authenticated_tool_records(original.as_bytes(), &digest).is_ok(),
            "restored file does not authenticate the transient changed buffer"
        );
    }
}
