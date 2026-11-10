//! Versioned owner evidence, distinct from a confirmed GoalRef or model interpretation.
//!
//! Only trusted host ingress constructs OwnerInput. Nothing here writes a charter,
//! infers authority from message roles, or interprets an owner's sentence as a goal.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::message::{Message, ToolChoice, ToolSpec};

const MAX_SOURCE_BYTES: usize = 64 * 1024;
const MAX_VERSIONS: usize = 256;
const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
const MAX_HISTORY_BYTES: usize = 64 * 1024 * 1024;

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Authentication is the caller's responsibility; these are host entry points,
/// never values accepted by a model tool or inferred from Role::User.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerSource {
    CliChat,
    Tui,
    AuthenticatedWeb,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerInput {
    source: OwnerSource,
    text: String,
}

impl OwnerInput {
    pub fn new(source: OwnerSource, text: String) -> Result<Self> {
        ensure!(!text.trim().is_empty(), "owner evidence is empty");
        ensure!(
            text.len() <= MAX_SOURCE_BYTES,
            "owner evidence exceeds 64 KiB"
        );
        Ok(Self { source, text })
    }
    pub fn source(&self) -> OwnerSource {
        self.source
    }
    pub fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CharterStatus {
    Disabled,
    Missing,
    Present,
}

/// Keeps original owner bytes and the exact rendering separately. Sensor fields
/// remain in the private source; prompt consumers receive only `prompt_block`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharterSnapshot {
    status: CharterStatus,
    source: Option<String>,
    source_digest: String,
    rendered: Option<String>,
    todo_available: bool,
}

impl CharterSnapshot {
    pub fn disabled() -> Self {
        Self {
            status: CharterStatus::Disabled,
            source: None,
            source_digest: digest(b"disabled"),
            rendered: None,
            todo_available: false,
        }
    }
    pub fn from_source(source: Option<String>, todo_available: bool) -> Result<Self> {
        let (status, rendered, source_digest) = match source.as_deref() {
            None => (CharterStatus::Missing, None, digest(b"missing")),
            Some(text) => {
                ensure!(
                    text.len() <= MAX_SOURCE_BYTES,
                    "charter source exceeds 64 KiB"
                );
                let charter = crate::charter::Charter::parse(text)?;
                (
                    CharterStatus::Present,
                    crate::charter::prompt_block_for(&charter, todo_available),
                    digest(text.as_bytes()),
                )
            }
        };
        Ok(Self {
            status,
            source,
            source_digest,
            rendered,
            todo_available,
        })
    }
    /// Typed historical source for harness appraisal. Never reads a live file
    /// and never exposes sensor fields to a model-facing prompt.
    pub fn parsed(&self) -> Result<Option<crate::charter::Charter>> {
        self.validate()?;
        self.source
            .as_deref()
            .map(crate::charter::Charter::parse)
            .transpose()
    }
    pub fn prompt_block(&self) -> Option<&str> {
        self.rendered.as_deref()
    }
    pub fn source_digest(&self) -> &str {
        &self.source_digest
    }
    fn validate(&self) -> Result<()> {
        let expected = match self.status {
            CharterStatus::Disabled => Self::disabled(),
            CharterStatus::Missing => {
                ensure!(self.source.is_none(), "missing charter has source bytes");
                Self::from_source(None, self.todo_available)?
            }
            CharterStatus::Present => {
                ensure!(self.source.is_some(), "present charter has no source bytes");
                Self::from_source(self.source.clone(), self.todo_available)?
            }
        };
        // Historical rendered bytes are authoritative. Never regenerate them
        // with today's wording when replaying an older binary's record.
        ensure!(
            expected.status == self.status
                && expected.source == self.source
                && expected.source_digest == self.source_digest,
            "charter identity/content mismatch"
        );
        ensure!(
            self.rendered
                .as_ref()
                .is_none_or(|s| s.len() <= MAX_SOURCE_BYTES),
            "rendered charter exceeds bound"
        );
        if self.status != CharterStatus::Present {
            ensure!(
                self.rendered.is_none(),
                "absent charter has rendered content"
            );
        }
        Ok(())
    }
}

/// A snapshot accumulates attributed evidence. It does not create priorities,
/// erase earlier instructions or give a model interpretation owner authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextVersion {
    schema: u32,
    epoch: u64,
    revision: u64,
    predecessor: Option<u64>,
    charter: CharterSnapshot,
    owner_inputs: Vec<OwnerInput>,
    content_digest: String,
}

impl ContextVersion {
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn predecessor(&self) -> Option<u64> {
        self.predecessor
    }
    pub fn charter(&self) -> &CharterSnapshot {
        &self.charter
    }
    pub fn owner_inputs(&self) -> &[OwnerInput] {
        &self.owner_inputs
    }
    fn computed_digest(&self) -> Result<String> {
        Ok(digest(&serde_json::to_vec(&(
            self.schema,
            self.epoch,
            self.revision,
            self.predecessor,
            &self.charter,
            &self.owner_inputs,
        ))?))
    }
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(self.schema == 1, "unsupported goal context schema");
        self.charter.validate()?;
        let mut bytes = 0usize;
        for input in &self.owner_inputs {
            OwnerInput::new(input.source, input.text.clone())?;
            bytes = bytes
                .checked_add(input.text.len())
                .context("owner evidence size overflow")?;
        }
        ensure!(
            bytes <= MAX_SOURCE_BYTES,
            "accumulated owner evidence exceeds 64 KiB; start a new conversation"
        );
        ensure!(
            self.computed_digest()? == self.content_digest,
            "goal context content digest mismatch"
        );
        Ok(())
    }
    pub fn owner_prompt_block(&self) -> String {
        let evidence = serde_json::to_string(&self.owner_inputs).expect("owner strings serialize");
        format!("<owner_context revision=\"{}\">\nHistorical owner messages are evidence, quoted text remains quoted, and these are not standing charter policy or confirmation of a GoalRef. Interpret current intent from the conversation; later directions may refine or replace earlier ones.\n{}\n</owner_context>", self.revision, evidence)
    }
    pub fn prompt_block(&self) -> String {
        format!(
            "{}\n{}",
            self.charter.prompt_block().unwrap_or(""),
            self.owner_prompt_block()
        )
    }
}

// A missing optional field is corruption; an explicitly recorded JSON null
// remains a valid absence. deserialize_with prevents Serde's missing=None rule.
fn required_option<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

/// Exact provider request boundary, independent of mutable transcript indices.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionBinding {
    ordinal: u64,
    version: u64,
    messages: Vec<Message>,
    message_digest: String,
    #[serde(deserialize_with = "required_option")]
    conversation_messages: Option<Vec<Message>>,
    #[serde(deserialize_with = "required_option")]
    system: Option<String>,
    run_start: bool,
    trailing_notes: usize,
    request_digest: String,
    tool_surface_digest: String,
    tools_allowed: bool,
}

impl DecisionBinding {
    pub fn new(
        ordinal: u64,
        version: u64,
        messages: &[Message],
        system: Option<String>,
        run_start: bool,
        trailing_notes: usize,
    ) -> Result<Self> {
        let bytes = serde_json::to_vec(messages)?;
        ensure!(
            bytes.len() + system.as_ref().map_or(0, String::len) <= MAX_REQUEST_BYTES,
            "goal context request exceeds 8 MiB recording bound"
        );
        let mut binding = Self {
            ordinal,
            version,
            message_digest: digest(&bytes),
            conversation_messages: None,
            messages: messages.to_vec(),
            system,
            run_start,
            trailing_notes,
            request_digest: String::new(),
            tool_surface_digest: crate::surface::fingerprint(&[]),
            tools_allowed: true,
        };
        binding.request_digest = binding.computed_request_digest()?;
        Ok(binding)
    }
    pub fn with_tool_surface(mut self, specs: &[ToolSpec], choice: ToolChoice) -> Result<Self> {
        self.tool_surface_digest = crate::surface::fingerprint(specs);
        self.tools_allowed = matches!(choice, ToolChoice::Auto);
        self.request_digest = self.computed_request_digest()?;
        Ok(self)
    }
    pub fn verify_tool_surface(&self, specs: &[ToolSpec], choice: ToolChoice) -> Result<()> {
        self.validate()?;
        ensure!(
            self.tool_surface_digest == crate::surface::fingerprint(specs)
                && self.tools_allowed == matches!(choice, ToolChoice::Auto),
            "dynamic goal replay tool surface or tool-choice mode differs from recording"
        );
        Ok(())
    }
    pub fn with_conversation_messages(mut self, messages: &[Message]) -> Result<Self> {
        self.conversation_messages = Some(messages.to_vec());
        self.request_digest = self.computed_request_digest()?;
        ensure!(
            serde_json::to_vec(&self)?.len() <= MAX_REQUEST_BYTES,
            "goal context request and history exceed 8 MiB recording bound"
        );
        Ok(self)
    }
    pub fn conversation_messages(&self) -> Option<&[Message]> {
        self.conversation_messages.as_deref()
    }
    pub fn ordinal(&self) -> u64 {
        self.ordinal
    }
    pub fn version(&self) -> u64 {
        self.version
    }
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }
    pub fn message_digest(&self) -> &str {
        &self.message_digest
    }
    pub fn system(&self) -> Option<&str> {
        self.system.as_deref()
    }
    pub fn trailing_notes(&self) -> usize {
        self.trailing_notes
    }
    pub fn run_start(&self) -> bool {
        self.run_start
    }
    pub fn verify_messages(&self, messages: &[Message]) -> Result<()> {
        ensure!(
            digest(&serde_json::to_vec(messages)?) == self.message_digest,
            "dynamic goal replay request boundary diverged from recording"
        );
        Ok(())
    }
    fn computed_request_digest(&self) -> Result<String> {
        Ok(digest(&serde_json::to_vec(&(
            self.ordinal,
            self.version,
            &self.messages,
            &self.message_digest,
            &self.conversation_messages,
            &self.system,
            self.run_start,
            self.trailing_notes,
            &self.tool_surface_digest,
            self.tools_allowed,
        ))?))
    }
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.request_digest == self.computed_request_digest()?,
            "goal context effective request digest mismatch"
        );
        self.verify_messages(&self.messages)?;
        ensure!(
            serde_json::to_vec(self)?.len() <= MAX_REQUEST_BYTES,
            "goal context recorded boundary exceeds 8 MiB"
        );
        Self::new(
            self.ordinal,
            self.version,
            &self.messages,
            self.system.clone(),
            self.run_start,
            self.trailing_notes,
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct GoalContextState {
    versions: Vec<ContextVersion>,
    decisions: Vec<DecisionBinding>,
    history_bytes: usize,
    epoch: u64,
    active: Option<usize>,
}

impl GoalContextState {
    pub fn current(&self) -> Option<&ContextVersion> {
        self.active.and_then(|i| self.versions.get(i))
    }
    pub fn has_cleared(&self) -> bool {
        self.epoch != 0
    }
    pub fn clear(&mut self) {
        self.epoch += 1;
        self.active = None;
    }
    pub fn versions(&self) -> &[ContextVersion] {
        &self.versions
    }
    pub fn decisions(&self) -> &[DecisionBinding] {
        &self.decisions
    }
    pub fn next_decision(&self) -> u64 {
        self.decisions.last().map_or(1, |d| d.ordinal + 1)
    }
    pub fn version(&self, revision: u64) -> Option<&ContextVersion> {
        self.versions.iter().find(|v| v.revision == revision)
    }
    pub fn transition(
        &mut self,
        charter: CharterSnapshot,
        input: Option<OwnerInput>,
    ) -> Result<Option<ContextVersion>> {
        if self.current().is_some_and(|v| v.charter == charter) && input.is_none() {
            return Ok(None);
        }
        let mut inputs = self
            .current()
            .map_or_else(Vec::new, |v| v.owner_inputs.clone());
        inputs.extend(input);
        let predecessor = self.versions.last().map(|v| v.revision);
        let mut version = ContextVersion {
            schema: 1,
            epoch: self.epoch,
            revision: predecessor.map_or(1, |v| v + 1),
            predecessor,
            charter,
            owner_inputs: inputs,
            content_digest: String::new(),
        };
        version.content_digest = version.computed_digest()?;
        self.accept_version(version.clone())?;
        Ok(Some(version))
    }
    pub fn accept_version(&mut self, version: ContextVersion) -> Result<()> {
        version.validate()?;
        if let Some(old) = self.version(version.revision) {
            ensure!(old == &version, "goal context revision rewritten");
            return Ok(());
        }
        ensure!(
            self.versions.len() < MAX_VERSIONS,
            "goal context exceeds 256 revisions; exit and start a fresh session without resuming"
        );
        let previous = self.versions.last();
        ensure!(
            version.epoch == self.epoch,
            "goal context epoch has no recorded clear"
        );
        ensure!(
            version.predecessor == previous.map(|v| v.revision)
                && version.revision == previous.map_or(1, |v| v.revision + 1),
            "broken goal context revision chain"
        );
        if let Some(previous) = self.current() {
            ensure!(
                version.owner_inputs.starts_with(&previous.owner_inputs),
                "goal context discarded or rewrote owner evidence"
            );
            ensure!(
                version.owner_inputs.len() <= previous.owner_inputs.len() + 1,
                "goal context skips owner evidence revisions"
            );
        } else {
            ensure!(
                version.owner_inputs.len() <= 1,
                "initial goal context has unexplained evidence"
            );
        }
        let bytes = serde_json::to_vec(&version)?.len();
        ensure!(
            self.history_bytes + bytes <= MAX_HISTORY_BYTES,
            "goal context history exceeds 64 MiB; exit and start a fresh session without resuming"
        );
        self.history_bytes += bytes;
        self.active = Some(self.versions.len());
        self.versions.push(version);
        Ok(())
    }
    pub fn bind(&mut self, binding: DecisionBinding) -> Result<()> {
        binding.validate()?;
        ensure!(
            binding.ordinal == self.next_decision(),
            "broken goal context decision sequence"
        );
        ensure!(
            self.current()
                .is_some_and(|v| v.revision == binding.version),
            "decision does not bind current recorded goal context"
        );
        let bytes = serde_json::to_vec(&binding)?.len();
        ensure!(
            self.history_bytes + bytes <= MAX_HISTORY_BYTES,
            "goal context history exceeds 64 MiB; exit and start a fresh session without resuming"
        );
        self.history_bytes += bytes;
        self.decisions.push(binding);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(text: &str) -> OwnerInput {
        OwnerInput::new(OwnerSource::CliChat, text.into()).unwrap()
    }
    fn charter(text: &str) -> CharterSnapshot {
        CharterSnapshot::from_source(
            Some(format!("[[line]]\nid = 'owner'\ntext = '{text}'\n")),
            false,
        )
        .unwrap()
    }
    #[test]
    fn clear_retains_session_limits_and_errors_direct_the_owner_to_a_fresh_session() {
        let mut revisions = GoalContextState::default();
        for _ in 0..MAX_VERSIONS {
            revisions
                .transition(CharterSnapshot::disabled(), Some(input("review the draft")))
                .unwrap();
            revisions.clear();
        }
        assert!(revisions.current().is_none());
        assert_eq!(revisions.versions().len(), MAX_VERSIONS);
        let error = revisions
            .transition(CharterSnapshot::disabled(), Some(input("check the labels")))
            .unwrap_err()
            .to_string();
        assert!(error.contains("256 revisions"));
        assert!(error.contains("exit and start a fresh session without resuming"));

        let mut history = GoalContextState::default();
        history
            .transition(CharterSnapshot::disabled(), Some(input("check the labels")))
            .unwrap();
        // Set the counter to its boundary without allocating 64 MiB of records.
        history.history_bytes = MAX_HISTORY_BYTES;
        let error = history.bind(binding_fixture()).unwrap_err().to_string();
        assert!(error.contains("64 MiB"));
        assert!(error.contains("exit and start a fresh session without resuming"));
        history.clear();
        assert!(history.current().is_none());
        let error = history
            .transition(CharterSnapshot::disabled(), Some(input("check the lids")))
            .unwrap_err()
            .to_string();
        assert!(error.contains("64 MiB"));
        assert!(error.contains("exit and start a fresh session without resuming"));
    }

    fn binding_fixture() -> DecisionBinding {
        let messages = vec![Message::user("authored task")];
        DecisionBinding::new(
            1,
            1,
            &messages,
            Some("historical charter system".into()),
            true,
            0,
        )
        .unwrap()
        .with_conversation_messages(&messages)
        .unwrap()
    }
    #[test]
    fn effective_request_binds_tool_surface_and_forced_final_mode() {
        let tool = ToolSpec {
            name: "echo".into(),
            description: "Authored echo tool".into(),
            input_schema: serde_json::json!({"type":"object"}),
        };
        let binding = binding_fixture()
            .with_tool_surface(std::slice::from_ref(&tool), ToolChoice::None)
            .unwrap();
        binding
            .verify_tool_surface(std::slice::from_ref(&tool), ToolChoice::None)
            .unwrap();
        assert!(binding
            .verify_tool_surface(std::slice::from_ref(&tool), ToolChoice::Auto)
            .is_err());
        assert!(binding.verify_tool_surface(&[], ToolChoice::None).is_err());
        let changed = ToolSpec {
            description: "Different tool semantics".into(),
            ..tool
        };
        assert!(binding
            .verify_tool_surface(&[changed], ToolChoice::None)
            .is_err());
        let original = serde_json::to_value(&binding).unwrap();
        for field in ["tool_surface_digest", "tools_allowed"] {
            let mut missing = original.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<DecisionBinding>(missing).is_err());
            let mut corrupt = original.clone();
            corrupt[field] = if field == "tools_allowed" {
                serde_json::json!(true)
            } else {
                serde_json::json!("substituted")
            };
            assert!(serde_json::from_value::<DecisionBinding>(corrupt)
                .unwrap()
                .validate()
                .is_err());
        }
        binding_fixture()
            .verify_tool_surface(&[], ToolChoice::Auto)
            .unwrap();
    }
    #[test]
    fn effective_request_requires_present_nullable_fields() {
        let mut value = serde_json::to_value(binding_fixture()).unwrap();
        value.as_object_mut().unwrap().remove("system");
        assert!(serde_json::from_value::<DecisionBinding>(value).is_err());
        let mut value = serde_json::to_value(binding_fixture()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("conversation_messages");
        assert!(serde_json::from_value::<DecisionBinding>(value).is_err());
        let absent = DecisionBinding::new(1, 1, &[Message::user("task")], None, true, 0).unwrap();
        let value = serde_json::to_value(&absent).unwrap();
        assert!(value.get("system").unwrap().is_null());
        assert!(value.get("conversation_messages").unwrap().is_null());
        serde_json::from_value::<DecisionBinding>(value)
            .unwrap()
            .validate()
            .unwrap();
    }
    #[test]
    fn effective_request_integrity_covers_every_replay_field() {
        let original = serde_json::to_value(binding_fixture()).unwrap();
        for (key, replacement) in [
            ("system", serde_json::json!("substituted system")),
            ("system", serde_json::Value::Null),
            ("trailing_notes", serde_json::json!(1)),
            ("run_start", serde_json::json!(false)),
            ("ordinal", serde_json::json!(2)),
            ("version", serde_json::json!(2)),
            (
                "conversation_messages",
                serde_json::json!([Message::user("substituted history")]),
            ),
            ("conversation_messages", serde_json::Value::Null),
            (
                "messages",
                serde_json::json!([Message::user("substituted wire")]),
            ),
        ] {
            let mut changed = original.clone();
            changed[key] = replacement;
            let binding: DecisionBinding = serde_json::from_value(changed).unwrap();
            assert!(binding.validate().is_err(), "accepted corrupt {key}");
        }
    }
    #[test]
    fn owner_revision_retains_sources_and_historical_charter() {
        let mut state = GoalContextState::default();
        let first = state
            .transition(charter("first priority"), Some(input("prepare a report")))
            .unwrap()
            .unwrap();
        let second = state
            .transition(
                charter("revised priority"),
                Some(input("focus on the figure first")),
            )
            .unwrap()
            .unwrap();
        assert_eq!(second.predecessor(), Some(first.revision()));
        assert_eq!(second.owner_inputs().len(), 2);
        assert!(first.prompt_block().contains("first priority"));
        assert!(!first.prompt_block().contains("revised priority"));
        assert!(second.prompt_block().contains("prepare a report"));
    }
    #[test]
    fn malformed_charter_is_an_error_not_a_deletion() {
        assert!(CharterSnapshot::from_source(Some("broken = [".into()), true).is_err());
        assert_ne!(
            CharterSnapshot::disabled(),
            CharterSnapshot::from_source(None, false).unwrap()
        );
    }
    #[test]
    fn corrupt_or_discontinuous_revisions_refuse() {
        let mut state = GoalContextState::default();
        let first = state
            .transition(charter("priority"), Some(input("task")))
            .unwrap()
            .unwrap();
        let mut bad = first.clone();
        bad.revision = 4;
        assert!(GoalContextState::default().accept_version(bad).is_err());
        let mut bad = first;
        bad.owner_inputs[0].text = "substituted".into();
        assert!(GoalContextState::default().accept_version(bad).is_err());
    }
    #[test]
    fn decision_binds_exact_request_and_monotonic_ordinal() {
        let mut state = GoalContextState::default();
        let version = state
            .transition(charter("priority"), Some(input("task")))
            .unwrap()
            .unwrap();
        let messages = vec![Message::user("task")];
        let binding = DecisionBinding::new(
            1,
            version.revision(),
            &messages,
            Some("exact system".into()),
            true,
            0,
        )
        .unwrap();
        assert!(binding
            .verify_messages(&[Message::user("different")])
            .is_err());
        state.bind(binding.clone()).unwrap();
        assert!(state.bind(binding).is_err());
        assert_eq!(state.decisions()[0].system(), Some("exact system"));
    }
    #[test]
    fn bounds_refuse_without_evicting_earlier_owner_evidence() {
        let mut state = GoalContextState::default();
        state
            .transition(
                CharterSnapshot::disabled(),
                Some(input(&"a".repeat(MAX_SOURCE_BYTES))),
            )
            .unwrap();
        assert!(state
            .transition(CharterSnapshot::disabled(), Some(input("b")))
            .is_err());
        assert_eq!(state.current().unwrap().owner_inputs().len(), 1);
    }
    #[test]
    fn sensors_never_enter_model_facing_context() {
        let source = "[[line]]\nid='owner'\ntext='priority'\n[line.sensor]\nkind='outbox_waiting'\nsetpoint=3\n";
        let mut state = GoalContextState::default();
        state
            .transition(
                CharterSnapshot::from_source(Some(source.into()), true).unwrap(),
                None,
            )
            .unwrap();
        assert!(!state
            .current()
            .unwrap()
            .prompt_block()
            .contains("outbox_waiting"));
    }
}
