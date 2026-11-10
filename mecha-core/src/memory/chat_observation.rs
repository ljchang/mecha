//! Bounded, reader-safe context for a host-authenticated chat turn.
//!
//! Full versions and effective requests stay in the native transcript/private
//! seal. These summaries never contain CharterSnapshot::source or sensor data.
use super::host_capture::{CaptureIdentity, OriginalPassage, PassageKind};
use crate::goal_context::{ContextVersion, DecisionBinding, OwnerInput, OwnerSource};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const CHAT_OBSERVATION_PROTOCOL: &str = "ordinary-chat-observation-v1";
pub const CHAT_RECORDER_VERSION: &str = "ordinary-chat-recorder-v1";

/// Sanitized historical evidence, never a confirmed evaluative criterion.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatContextVersion {
    revision: u64,
    predecessor: Option<u64>,
    context_digest: String,
    rendered_charter: Option<String>,
    owner_inputs: Vec<OwnerInput>,
}
impl ChatContextVersion {
    /// Call only with the version validated from the native decision record.
    pub fn from_version(version: &ContextVersion) -> Result<Self> {
        version.validate()?;
        Ok(Self {
            revision: version.revision(),
            predecessor: version.predecessor(),
            context_digest: serialized_digest(version)?,
            rendered_charter: version.charter().prompt_block().map(str::to_owned),
            owner_inputs: version.owner_inputs().to_vec(),
        })
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn owner_inputs(&self) -> &[OwnerInput] {
        &self.owner_inputs
    }
}

/// One-based native JSONL line and the exact original message field.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatOwnerMessageLocator {
    pub line: u64,
    pub pointer: String,
    pub context_revision: u64,
    pub owner_input_index: usize,
}

/// A locator, not a second copy of the effective provider request.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatDecisionLocator {
    pub line: u64,
    pub ordinal: u64,
    pub context_revision: u64,
    pub binding_digest: String,
}
impl ChatDecisionLocator {
    pub fn from_binding(line: u64, binding: &DecisionBinding) -> Result<Self> {
        binding.validate()?;
        Ok(Self {
            line,
            ordinal: binding.ordinal(),
            context_revision: binding.version(),
            binding_digest: serialized_digest(binding)?,
        })
    }
}

/// Host-produced sidecar. Native source validation must additionally prove these
/// locators/digests match the sealed prefix; a deserialized DTO is not authority.
/// Bounds: 16 used versions, 128 decisions, 64 owner locators, 24 KiB total.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatObservationContext {
    pub conversation_id: String,
    /// One-based completed-turn ordinal within the authenticated conversation.
    pub turn_ordinal: u64,
    /// Previous turn's check_id, absent only on the first turn.
    pub previous_turn: Option<String>,
    /// End of the preceding sealed prefix (zero on the first turn).
    pub prefix_start: u64,
    pub prefix_length: u64,
    pub prefix_digest: String,
    pub owner_messages: Vec<ChatOwnerMessageLocator>,
    pub versions: Vec<ChatContextVersion>,
    pub decisions: Vec<ChatDecisionLocator>,
}
fn serialized_digest(value: &impl Serialize) -> Result<String> {
    Ok(Sha256::digest(serde_json::to_vec(value)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl ChatObservationContext {
    pub(crate) fn validate(&self, identity: &CaptureIdentity) -> Result<()> {
        ensure!(
            self.conversation_id == identity.session_id
                && !self.conversation_id.is_empty()
                && self.conversation_id.len() <= 256
                && self.conversation_id.trim() == self.conversation_id
                && !self.conversation_id.chars().any(char::is_control)
                && self.turn_ordinal > 0
                && self.prefix_start < self.prefix_length
                && self.prefix_length <= 64 * 1024 * 1024
                && digest(&self.prefix_digest)
                && self.prefix_digest == identity.prefix_digest
                && if self.turn_ordinal == 1 {
                    self.previous_turn.is_none() && self.prefix_start == 0
                } else {
                    self.previous_turn
                        .as_ref()
                        .is_some_and(|s| digest(s) && s != &identity.check_id)
                        && self.prefix_start > 0
                },
            "invalid chat observation identity"
        );
        ensure!(
            (1..=16).contains(&self.versions.len())
                && (1..=128).contains(&self.decisions.len())
                && (1..=64).contains(&self.owner_messages.len())
                && serde_json::to_vec(self)?.len() <= 24 * 1024,
            "invalid chat observation bounds"
        );
        let mut revisions = BTreeSet::new();
        let mut last_revision = 0;
        for version in &self.versions {
            ensure!(
                version.revision > last_revision
                    && version.predecessor == version.revision.checked_sub(1).filter(|v| *v > 0)
                    && digest(&version.context_digest)
                    && version
                        .rendered_charter
                        .as_ref()
                        .is_none_or(|s| !s.trim().is_empty() && s.len() <= 64 * 1024)
                    && version.owner_inputs.len() <= 64,
                "invalid chat context version"
            );
            for input in &version.owner_inputs {
                ensure!(
                    input.source() == OwnerSource::CliChat,
                    "unsupported chat owner source"
                );
                OwnerInput::new(input.source(), input.text().into())?;
            }
            revisions.insert(version.revision);
            last_revision = version.revision;
        }
        let mut used = BTreeSet::new();
        let mut previous: Option<&ChatDecisionLocator> = None;
        for decision in &self.decisions {
            ensure!(
                decision.line > 0
                    && decision.line <= self.prefix_length
                    && digest(&decision.binding_digest)
                    && revisions.contains(&decision.context_revision)
                    && previous.is_none_or(|p| p.ordinal.checked_add(1) == Some(decision.ordinal)
                        && p.line < decision.line
                        && p.context_revision <= decision.context_revision),
                "invalid chat decision locator"
            );
            used.insert(decision.context_revision);
            previous = Some(decision);
        }
        ensure!(used == revisions, "unused chat context version");
        let mut locations = BTreeSet::new();
        for locator in &self.owner_messages {
            ensure!(
                locator.line > 0
                    && locator.line <= self.prefix_length
                    && owner_pointer(&locator.pointer)
                    && locations.insert((locator.line, &locator.pointer))
                    && self
                        .versions
                        .iter()
                        .any(|v| v.revision == locator.context_revision
                            && locator.owner_input_index < v.owner_inputs.len()),
                "invalid chat owner locator"
            );
        }
        Ok(())
    }
    pub(crate) fn validate_originals(&self, originals: &[OriginalPassage]) -> Result<()> {
        for passage in originals {
            ensure!(
                passage.line <= self.prefix_length,
                "chat passage outside prefix"
            );
            if passage.kind != PassageKind::UserRequest {
                continue;
            }
            let locator = self
                .owner_messages
                .iter()
                .find(|l| l.line == passage.line && l.pointer == passage.pointer);
            ensure!(
                locator.is_some_and(|l| self.versions.iter().any(|v| v.revision
                    == l.context_revision
                    && v.owner_inputs
                        .get(l.owner_input_index)
                        .is_some_and(|input| input.text() == passage.text))),
                "chat original lacks matching owner evidence"
            );
        }
        Ok(())
    }
}
fn owner_pointer(pointer: &str) -> bool {
    let Some(index) = pointer
        .strip_prefix("/content/")
        .and_then(|s| s.strip_suffix("/text"))
    else {
        return false;
    };
    index.parse::<usize>().is_ok_and(|n| n.to_string() == index)
}
