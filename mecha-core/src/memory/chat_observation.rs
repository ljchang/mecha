//! Bounded, reader-safe context for a host-authenticated chat turn.
//!
//! Full versions and effective requests stay in the native transcript/private
//! seal. These summaries never contain CharterSnapshot::source or sensor data.
use super::host_capture::{CaptureIdentity, OriginalPassage, PassageKind};
#[cfg(feature = "memory-prototype")]
use crate::goal_context::{ContextVersion, DecisionBinding, OwnerSource};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
#[cfg(feature = "memory-prototype")]
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const CHAT_OBSERVATION_PROTOCOL: &str = "ordinary-chat-observation-v1";
pub const CHAT_RECORDER_VERSION: &str = "ordinary-chat-recorder-v1";
pub const CHAT_WEB_OBSERVATION_PROTOCOL: &str = "ordinary-chat-observation-v2";
pub const CHAT_WEB_RECORDER_VERSION: &str = "ordinary-chat-recorder-v2";
pub const CHAT_TASK_OBSERVATION_PROTOCOL: &str = "ordinary-chat-observation-v3";
pub const CHAT_TASK_RECORDER_VERSION: &str = "ordinary-chat-recorder-v3";
/// An ordinary local chat's turn, recorded by the walk that leaves out what
/// it cannot place. Its own pair, because the version is what tells a
/// reader which rules stand behind a record: a v1 record was bound to the
/// exact history every request sent and to a closed tool list, and this one
/// is not.
pub const CHAT_OPEN_OBSERVATION_PROTOCOL: &str = "ordinary-chat-observation-v4";
pub const CHAT_OPEN_RECORDER_VERSION: &str = "ordinary-chat-recorder-v4";
/// An ordinary web chat's turn: the open recorder's rules, with the owner's
/// words authenticated by the web host.
pub const CHAT_OPEN_WEB_OBSERVATION_PROTOCOL: &str = "ordinary-chat-observation-v5";
pub const CHAT_OPEN_WEB_RECORDER_VERSION: &str = "ordinary-chat-recorder-v5";
/// The most decisions and context versions one turn's context names, and
/// the most bytes it serializes to.
pub const MAX_DECISIONS: usize = 128;
pub const MAX_VERSIONS: usize = 16;
pub const MAX_CONTEXT_BYTES: usize = 96 * 1024;

/// The currently supported authenticated chat ingress, independent of runtime types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatOwnerSource {
    CliChat,
    AuthenticatedWeb,
}

/// Reader-safe owner evidence. Field order and source spelling match the native
/// OwnerInput wire format, while the standalone memory harness owns no runtime.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatOwnerInput {
    source: ChatOwnerSource,
    text: String,
}
impl ChatOwnerInput {
    pub fn text(&self) -> &str {
        &self.text
    }
    fn validate(&self) -> Result<()> {
        ensure!(!self.text.trim().is_empty(), "owner evidence is empty");
        ensure!(
            self.text.len() <= 64 * 1024,
            "owner evidence exceeds 64 KiB"
        );
        Ok(())
    }
}

/// Sanitized historical evidence, never a confirmed evaluative criterion.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatContextVersion {
    revision: u64,
    predecessor: Option<u64>,
    context_digest: String,
    rendered_charter: Option<String>,
    owner_inputs: Vec<ChatOwnerInput>,
}
impl ChatContextVersion {
    /// Call only with the version validated from the native decision record.
    #[cfg(feature = "memory-prototype")]
    pub fn from_version(version: &ContextVersion) -> Result<Self> {
        Self::from_version_for(version, OwnerSource::CliChat, ChatOwnerSource::CliChat)
    }
    /// Sanitizes only versions attributed by the authenticated web host.
    #[cfg(feature = "memory-prototype")]
    pub fn from_authenticated_web_version(version: &ContextVersion) -> Result<Self> {
        Self::from_version_for(
            version,
            OwnerSource::AuthenticatedWeb,
            ChatOwnerSource::AuthenticatedWeb,
        )
    }
    /// For an open recorder: the owner lines from `from` (an index into
    /// the whole conversation) on, which are the turn's own. A revision
    /// carries a window of recent lines, most of them earlier turns', and
    /// a turn's capture names only what it is evidence for. Each line is
    /// held as a passage holds it, its head when it is longer than one.
    #[cfg(feature = "memory-prototype")]
    pub fn from_version_tail(version: &ContextVersion, web: bool, from: u64) -> Result<Self> {
        let (native, source) = if web {
            (
                OwnerSource::AuthenticatedWeb,
                ChatOwnerSource::AuthenticatedWeb,
            )
        } else {
            (OwnerSource::CliChat, ChatOwnerSource::CliChat)
        };
        let mut summary = Self::from_version_for(version, native, source)?;
        let skip = usize::try_from(from.saturating_sub(version.earlier_inputs()))?;
        ensure!(
            from >= version.earlier_inputs() && skip <= summary.owner_inputs.len(),
            "chat turn's owner lines are no longer carried by its context"
        );
        summary.owner_inputs.drain(..skip);
        for input in &mut summary.owner_inputs {
            input.text = super::host_capture::excerpt(std::mem::take(&mut input.text)).0;
        }
        Ok(summary)
    }
    #[cfg(feature = "memory-prototype")]
    fn from_version_for(
        version: &ContextVersion,
        native_source: OwnerSource,
        source: ChatOwnerSource,
    ) -> Result<Self> {
        version.validate()?;
        Ok(Self {
            revision: version.revision(),
            predecessor: version.predecessor(),
            context_digest: serialized_digest(version)?,
            rendered_charter: version.charter().prompt_block().map(str::to_owned),
            owner_inputs: version
                .owner_inputs()
                .iter()
                .map(|input| {
                    ensure!(
                        input.source() == native_source,
                        "unsupported chat owner source"
                    );
                    Ok(ChatOwnerInput {
                        source,
                        text: input.text().into(),
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        })
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn owner_inputs(&self) -> &[ChatOwnerInput] {
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
#[cfg(feature = "memory-prototype")]
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
/// Bounds: 16 used versions, 128 decisions, 64 owner locators, 96 KiB total
/// (`MAX_CONTEXT_BYTES`).
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
#[cfg(feature = "memory-prototype")]
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
        let source = match (
            identity.check_version.as_str(),
            identity.evaluator_version.as_str(),
        ) {
            (CHAT_OBSERVATION_PROTOCOL, CHAT_RECORDER_VERSION)
            | (CHAT_OPEN_OBSERVATION_PROTOCOL, CHAT_OPEN_RECORDER_VERSION)
            | (CHAT_TASK_OBSERVATION_PROTOCOL, CHAT_TASK_RECORDER_VERSION) => {
                ChatOwnerSource::CliChat
            }
            (CHAT_WEB_OBSERVATION_PROTOCOL, CHAT_WEB_RECORDER_VERSION)
            | (CHAT_OPEN_WEB_OBSERVATION_PROTOCOL, CHAT_OPEN_WEB_RECORDER_VERSION) => {
                ChatOwnerSource::AuthenticatedWeb
            }
            _ => anyhow::bail!("invalid chat observation protocol"),
        };
        let open = matches!(
            identity.check_version.as_str(),
            CHAT_OPEN_OBSERVATION_PROTOCOL | CHAT_OPEN_WEB_OBSERVATION_PROTOCOL
        );
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
                // A turn the recorder could not capture is passed over, so
                // the first turn retained need not be the first begun: what
                // must agree is that a turn names a predecessor exactly when
                // a sealed prefix lies before it.
                && match &self.previous_turn {
                    None => self.prefix_start == 0,
                    Some(previous) => {
                        self.turn_ordinal > 1
                            && digest(previous)
                            && previous != &identity.check_id
                            && self.prefix_start > 0
                    }
                },
            "invalid chat observation identity"
        );
        ensure!(
            (1..=MAX_VERSIONS).contains(&self.versions.len())
                && (1..=MAX_DECISIONS).contains(&self.decisions.len())
                && (1..=64).contains(&self.owner_messages.len())
                && serde_json::to_vec(self)?.len() <= MAX_CONTEXT_BYTES,
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
                    input.source == source,
                    "chat owner source does not match protocol"
                );
                input.validate()?;
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
                    && owner_pointer(&locator.pointer, open)
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
/// `/content/N/text` in a message record, or `/messages/M/content/N/text`
/// in a rewrite: an owner's line folded into a message that already held
/// tool results is recorded only there.
/// Only the open recorders read a rewrite or an extension, so only their
/// records may point into one: `/messages/M/content/N/text` in a rewrite,
/// `/blocks/N/text` in an extension (a line the owner typed before the
/// turn's first tool round, folded into the owner's own message).
fn owner_pointer(pointer: &str, listed_ok: bool) -> bool {
    let index = |s: &str| s.parse::<usize>().is_ok_and(|n| n.to_string() == s);
    if let Some(block) = pointer
        .strip_prefix("/blocks/")
        .and_then(|s| s.strip_suffix("/text"))
    {
        return listed_ok && index(block);
    }
    let rest = match pointer.strip_prefix("/messages/") {
        Some(listed) => match listed.split_once('/') {
            Some((message, rest)) if listed_ok && index(message) => rest,
            _ => return false,
        },
        None => match pointer.strip_prefix('/') {
            Some(rest) => rest,
            None => return false,
        },
    };
    rest.strip_prefix("content/")
        .and_then(|s| s.strip_suffix("/text"))
        .is_some_and(index)
}
