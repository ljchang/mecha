//! Versioned owner evidence, distinct from a confirmed GoalRef or model interpretation.
//!
//! Only trusted host ingress constructs OwnerInput. Nothing here writes a charter,
//! infers authority from message roles, or interprets an owner's sentence as a goal.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::message::{Message, ToolChoice, ToolSpec};

const MAX_SOURCE_BYTES: usize = 64 * 1024;
/// The owner lines a revision carries in full: the most recent ones. The
/// conversation itself holds every line; what a revision adds is which of
/// them the host authenticated as the owner's, and a model deciding what
/// the owner wants now reads the recent ones. Earlier lines are kept as a
/// count and a digest ([`Earlier`]), so a revision's size does not grow
/// with the conversation.
const WINDOW_INPUTS: usize = 16;
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

/// The owner lines that came before a revision's window: how many, and a
/// digest chained over each in order, so a revision cannot drop or reorder
/// one without the chain saying so. Absent from the wire while nothing has
/// left the window, so a revision recorded before windows existed keeps its
/// bytes and its digest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Earlier {
    count: u64,
    digest: String,
}
impl Earlier {
    fn is_none(&self) -> bool {
        self.count == 0
    }
    /// With `input` the next line to leave the window.
    fn with(&self, input: &OwnerInput) -> Result<Self> {
        let mut bytes = self.digest.clone().into_bytes();
        bytes.extend(serde_json::to_vec(input)?);
        Ok(Self {
            count: self.count + 1,
            digest: digest(&bytes),
        })
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
    #[serde(default, skip_serializing_if = "Earlier::is_none")]
    earlier: Earlier,
    content_digest: String,
}

impl ContextVersion {
    /// How many owner lines came before the ones this revision carries.
    pub fn earlier_inputs(&self) -> u64 {
        self.earlier.count
    }
    /// Every owner line of the conversation up to this revision, carried
    /// or not.
    pub fn total_inputs(&self) -> u64 {
        self.earlier.count + self.owner_inputs.len() as u64
    }
    /// The owner line at `index` of the whole conversation, when this
    /// revision still carries it.
    pub fn input_at(&self, index: u64) -> Option<&OwnerInput> {
        index
            .checked_sub(self.earlier.count)
            .and_then(|at| self.owner_inputs.get(usize::try_from(at).ok()?))
    }
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
        // The tuple a revision recorded before windows existed was hashed
        // over, unchanged while nothing has left the window.
        if self.earlier.is_none() {
            return Ok(digest(&serde_json::to_vec(&(
                self.schema,
                self.epoch,
                self.revision,
                self.predecessor,
                &self.charter,
                &self.owner_inputs,
            ))?));
        }
        Ok(digest(&serde_json::to_vec(&(
            self.schema,
            self.epoch,
            self.revision,
            self.predecessor,
            &self.charter,
            &self.owner_inputs,
            &self.earlier,
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
        // Of the lines carried. A window slides to stay under this; a
        // revision recorded before windows existed carried every line and
        // was refused past it.
        ensure!(
            bytes <= MAX_SOURCE_BYTES,
            "accumulated owner evidence exceeds 64 KiB; start a new conversation"
        );
        ensure!(
            if self.earlier.is_none() {
                self.earlier.digest.is_empty()
            } else {
                self.earlier.digest.len() == 64
            },
            "goal context earlier evidence digest malformed"
        );
        ensure!(
            self.computed_digest()? == self.content_digest,
            "goal context content digest mismatch"
        );
        Ok(())
    }
    pub fn owner_prompt_block(&self) -> String {
        let evidence = serde_json::to_string(&self.owner_inputs).expect("owner strings serialize");
        // Said only once lines have left the window, so a short
        // conversation's block is byte for byte what it was. It does not
        // say the earlier lines are above: a compaction may have cut the
        // messages that held them, and then a summary is all that is.
        let earlier = if self.earlier.is_none() {
            String::new()
        } else {
            format!(
                " These are the {} most recent of {}; the earlier ones are earlier in the conversation, or in a summary of it if it was compacted.",
                self.owner_inputs.len(),
                self.total_inputs()
            )
        };
        format!("<owner_context revision=\"{}\">\nHistorical owner messages are evidence, quoted text remains quoted, and these are not standing charter policy or confirmation of a GoalRef. Interpret current intent from the conversation; later directions may refine or replace earlier ones.{}\n{}\n</owner_context>", self.revision, earlier, evidence)
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

/// What a compact binding keeps of a request in place of its copy: enough
/// to tell whether a later request is the same one, and whether the
/// history it sent is a prefix of the conversation as recorded.
///
/// A full binding copies the request's messages and the conversation's on
/// every model request, so a transcript grows with the square of the
/// conversation. This is a few hundred bytes whatever the length.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Compact {
    /// How many messages the conversation held when the request went out.
    conversation: u64,
    /// The chain over all of them but the last ([`history_heads`]).
    conversation_head: String,
    /// The last one's digest. A notice or a steer folded into the tail
    /// changes this and leaves the head alone.
    conversation_tail: Option<String>,
    system_digest: Option<String>,
}

/// The chain over each prefix of `messages`: `heads[k]` covers the first
/// `k`. A compact binding records the head over all but its last message,
/// and a reader with the conversation in hand finds whether that history
/// is a prefix of it by one comparison.
pub fn history_heads(messages: &[Message]) -> Result<Vec<String>> {
    let mut heads = Vec::with_capacity(messages.len() + 1);
    let mut head = digest(b"");
    heads.push(head.clone());
    for message in messages {
        let mut bytes = head.into_bytes();
        bytes.extend(digest(&serde_json::to_vec(message)?).into_bytes());
        head = digest(&bytes);
        heads.push(head.clone());
    }
    Ok(heads)
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
    /// Present when the copies above were left out. Absent from the wire
    /// otherwise, so a full binding keeps its bytes and its digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compact: Option<Compact>,
}

impl DecisionBinding {
    /// As [`DecisionBinding::new`] and `with_conversation_messages`, keeping
    /// digests in place of the copies.
    ///
    /// The system prompt is the one copy still kept, and only when it is
    /// not `earlier`'s (the previous decision's): it is the same for most
    /// of a session, so it is recorded where it changes and a replay
    /// carries it forward ([`DecisionBinding::with_system_from`]).
    #[allow(clippy::too_many_arguments)]
    pub fn compact(
        ordinal: u64,
        version: u64,
        messages: &[Message],
        system: Option<&str>,
        run_start: bool,
        trailing_notes: usize,
        conversation_messages: &[Message],
        earlier: Option<&Self>,
    ) -> Result<Self> {
        let bytes = serde_json::to_vec(messages)?;
        ensure!(
            bytes.len() + system.map_or(0, str::len) <= MAX_REQUEST_BYTES,
            "goal context request exceeds 8 MiB recording bound"
        );
        let heads = history_heads(conversation_messages)?;
        let last = conversation_messages.len().saturating_sub(1);
        let mut binding = Self {
            ordinal,
            version,
            message_digest: digest(&bytes),
            conversation_messages: None,
            messages: Vec::new(),
            system: system
                .filter(|system| {
                    earlier.and_then(Self::system_digest) != Some(digest(system.as_bytes()))
                })
                .map(str::to_string),
            run_start,
            trailing_notes,
            request_digest: String::new(),
            tool_surface_digest: crate::surface::fingerprint(&[]),
            tools_allowed: true,
            compact: Some(Compact {
                conversation: conversation_messages.len() as u64,
                conversation_head: heads[last].clone(),
                conversation_tail: match conversation_messages.last() {
                    Some(tail) => Some(digest(&serde_json::to_vec(tail)?)),
                    None => None,
                },
                system_digest: system.map(|s| digest(s.as_bytes())),
            }),
        };
        binding.request_digest = binding.computed_request_digest()?;
        Ok(binding)
    }
    pub fn is_compact(&self) -> bool {
        self.compact.is_some()
    }
    /// The digest of the system prompt this request carried.
    pub fn system_digest(&self) -> Option<String> {
        match &self.compact {
            Some(compact) => compact.system_digest.clone(),
            None => self.system.as_deref().map(|s| digest(s.as_bytes())),
        }
    }
    /// A compact binding that left its system prompt out because it was the
    /// previous decision's, given that one back.
    pub fn with_system_from(mut self, earlier: Option<&str>) -> Self {
        if self.compact.is_some() && self.system.is_none() {
            self.system = earlier
                .filter(|system| self.system_digest() == Some(digest(system.as_bytes())))
                .map(str::to_string);
        }
        self
    }
    /// A compact binding's history: how many messages it sent, and the
    /// chain head over all but the last.
    pub fn conversation_shape(&self) -> Option<(usize, &str)> {
        self.compact.as_ref().and_then(|c| {
            Some((
                usize::try_from(c.conversation).ok()?,
                c.conversation_head.as_str(),
            ))
        })
    }
    /// Whether `message` is the last message a compact binding's request
    /// sent, by the digest it recorded. A full binding answers no: it has a
    /// copy to compare.
    pub fn sent_tail_is(&self, message: &Message) -> bool {
        self.compact.as_ref().is_some_and(|c| {
            serde_json::to_vec(message)
                .is_ok_and(|bytes| c.conversation_tail.as_deref() == Some(digest(&bytes).as_str()))
        })
    }
    /// The conversation as it stood when this request went out, for a
    /// replay to start from. A full binding copied it. A compact one
    /// recovers it from `conversation`, the session's messages as they
    /// ended: its first messages, when the chain over all but the last and
    /// the last one's digest are the ones recorded. They are not after a
    /// compaction or an eviction rewrote that history, and then there is
    /// nothing to recover it from.
    pub fn sent_conversation(&self, conversation: &[Message]) -> Result<Vec<Message>> {
        let Some(compact) = &self.compact else {
            return self
                .conversation_messages
                .clone()
                .context("dynamic replay lacks the recorded pre-request conversation");
        };
        let count = usize::try_from(compact.conversation)?;
        let sent = conversation
            .get(..count)
            .context("compactly recorded history is longer than the conversation")?;
        let tail = match sent.last() {
            Some(tail) => Some(digest(&serde_json::to_vec(tail)?)),
            None => None,
        };
        ensure!(
            history_heads(sent)?[count.saturating_sub(1)] == compact.conversation_head
                && tail == compact.conversation_tail,
            "this request's history was later rewritten (a compaction or an eviction), and a \
             compact record keeps no copy to replay it from"
        );
        Ok(sent.to_vec())
    }
    /// Whether `system` is what the request carried. A full binding holds
    /// the text; a compact one its digest.
    pub fn verify_system(&self, system: Option<&str>) -> Result<()> {
        let same = match &self.compact {
            Some(c) => c.system_digest == system.map(|s| digest(s.as_bytes())),
            None => self.system.as_deref() == system,
        };
        ensure!(
            same,
            "dynamic goal replay system prompt diverged from recording"
        );
        Ok(())
    }
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
            compact: None,
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
        if let Some(compact) = &self.compact {
            return Ok(digest(&serde_json::to_vec(&(
                self.ordinal,
                self.version,
                &self.message_digest,
                compact,
                self.run_start,
                self.trailing_notes,
                &self.tool_surface_digest,
                self.tools_allowed,
            ))?));
        }
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
        if let Some(compact) = &self.compact {
            let hex = |s: &str| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit());
            ensure!(
                self.messages.is_empty()
                    && self.conversation_messages.is_none()
                    && self.system.as_deref().is_none_or(|system| {
                        compact.system_digest == Some(digest(system.as_bytes()))
                    })
                    && hex(&self.message_digest)
                    && hex(&compact.conversation_head)
                    && compact.conversation_tail.as_deref().is_none_or(hex)
                    && compact.system_digest.as_deref().is_none_or(hex)
                    && compact.conversation_tail.is_some() == (compact.conversation > 0),
                "compact goal context boundary malformed"
            );
            return Ok(());
        }
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
        let (mut inputs, mut earlier) = self.current().map_or_else(Default::default, |v| {
            (v.owner_inputs.clone(), v.earlier.clone())
        });
        inputs.extend(input);
        // Slide the window: the oldest lines leave it, into the chain,
        // until what is carried fits. The newest line always stays.
        while inputs.len() > 1
            && (inputs.len() > WINDOW_INPUTS
                || inputs.iter().map(|i| i.text.len()).sum::<usize>() > MAX_SOURCE_BYTES)
        {
            earlier = earlier.with(&inputs.remove(0))?;
        }
        let predecessor = self.versions.last().map(|v| v.revision);
        let mut version = ContextVersion {
            schema: 1,
            epoch: self.epoch,
            revision: predecessor.map_or(1, |v| v + 1),
            predecessor,
            charter,
            owner_inputs: inputs,
            earlier,
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
        // No count of revisions bounds a conversation: each is bounded by
        // its window, and what they add up to by `MAX_HISTORY_BYTES` below.
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
            // At most one line more than before, and the window slid over
            // exactly the lines it dropped: what the previous revision
            // carried, plus the new line, less a prefix whose chain is the
            // one this revision records.
            let added = version
                .total_inputs()
                .checked_sub(previous.total_inputs())
                .filter(|added| *added <= 1)
                .context("goal context skips owner evidence revisions")?;
            let mut carried = previous.owner_inputs.clone();
            if added == 1 {
                carried.extend(version.owner_inputs.last().cloned());
            }
            let dropped = version
                .earlier
                .count
                .checked_sub(previous.earlier.count)
                .and_then(|dropped| usize::try_from(dropped).ok())
                .filter(|dropped| *dropped <= carried.len())
                .context("goal context discarded or rewrote owner evidence")?;
            let mut chain = previous.earlier.clone();
            for input in &carried[..dropped] {
                chain = chain.with(input)?;
            }
            ensure!(
                carried[dropped..] == version.owner_inputs[..] && chain == version.earlier,
                "goal context discarded or rewrote owner evidence"
            );
        } else {
            ensure!(
                version.owner_inputs.len() <= 1 && version.earlier.is_none(),
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
        // A count of revisions no longer ends a conversation (there were
        // 256): well past it, across clears, the next line is still taken.
        let mut revisions = GoalContextState::default();
        for _ in 0..600 {
            revisions
                .transition(CharterSnapshot::disabled(), Some(input("review the draft")))
                .unwrap();
            revisions.clear();
        }
        assert!(revisions.current().is_none());
        assert_eq!(revisions.versions().len(), 600);
        revisions
            .transition(CharterSnapshot::disabled(), Some(input("check the labels")))
            .unwrap();

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
    fn a_full_window_slides_and_the_earlier_line_is_still_counted() {
        let mut state = GoalContextState::default();
        state
            .transition(
                CharterSnapshot::disabled(),
                Some(input(&"a".repeat(MAX_SOURCE_BYTES))),
            )
            .unwrap();
        // The next line used to be refused and the conversation ended. It
        // is taken, and the line that no longer fits leaves the window for
        // the chain: carried no longer, counted still.
        state
            .transition(CharterSnapshot::disabled(), Some(input("b")))
            .unwrap();
        let current = state.current().unwrap();
        assert_eq!(current.owner_inputs().len(), 1);
        assert_eq!(current.owner_inputs()[0].text(), "b");
        assert_eq!((current.earlier_inputs(), current.total_inputs()), (1, 2));
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

    /// A long conversation: every revision carries the most recent lines
    /// and a chain over the rest, so a revision's size stays bounded while
    /// the count keeps going, and the chain refuses a revision that drops,
    /// reorders or rewrites a line.
    #[test]
    fn a_long_conversation_slides_a_window_over_its_owner_lines() {
        let mut state = GoalContextState::default();
        let mut sizes = vec![];
        for n in 0..300 {
            let version = state
                .transition(
                    CharterSnapshot::disabled(),
                    Some(input(&format!("line {n}"))),
                )
                .unwrap()
                .unwrap();
            sizes.push(serde_json::to_vec(&version).unwrap().len());
            assert_eq!(version.total_inputs(), n + 1);
            assert_eq!(
                version.input_at(n).map(OwnerInput::text),
                Some(format!("line {n}").as_str())
            );
        }
        let last = state.current().unwrap().clone();
        assert_eq!(last.owner_inputs().len(), WINDOW_INPUTS);
        assert_eq!(last.earlier_inputs(), 300 - WINDOW_INPUTS as u64);
        assert!(last.input_at(0).is_none() && last.input_at(300).is_none());
        // Bounded: the last revision is no larger than the first full window.
        assert!(sizes[299] <= sizes[WINDOW_INPUTS] + 64, "{sizes:?}");
        assert!(last.owner_prompt_block().contains("most recent of 300"));

        // A replayed history is accepted revision by revision.
        let mut replayed = GoalContextState::default();
        for version in state.versions() {
            replayed.accept_version(version.clone()).unwrap();
        }
        // One that drops a carried line, or claims a different past, is not.
        for tamper in 0..3 {
            let mut replayed = GoalContextState::default();
            for version in &state.versions()[..299] {
                replayed.accept_version(version.clone()).unwrap();
            }
            let mut forged = last.clone();
            match tamper {
                0 => {
                    forged.owner_inputs.remove(3);
                }
                1 => forged.earlier.count += 1,
                _ => forged.earlier.digest = "0".repeat(64),
            }
            forged.content_digest = forged.computed_digest().unwrap();
            assert!(replayed.accept_version(forged).is_err(), "tamper {tamper}");
        }
        // A revision whose window never slid records no chain. One that
        // names a digest over no lines is a state its own digest does not
        // cover and a round trip would drop, so it does not validate.
        let mut forged = state.versions()[0].clone();
        assert!(forged.earlier.is_none());
        forged.earlier.digest = "0".repeat(64);
        assert_eq!(forged.computed_digest().unwrap(), forged.content_digest);
        assert!(forged.validate().is_err());
    }

    /// One line as large as the window's byte bound stays, alone: the
    /// newest line is never the one dropped.
    #[test]
    fn a_window_keeps_its_newest_line_whatever_its_size() {
        let mut state = GoalContextState::default();
        state
            .transition(CharterSnapshot::disabled(), Some(input("short")))
            .unwrap();
        let large = "x".repeat(MAX_SOURCE_BYTES);
        let version = state
            .transition(CharterSnapshot::disabled(), Some(input(&large)))
            .unwrap()
            .unwrap();
        assert_eq!(version.owner_inputs().len(), 1);
        assert_eq!(version.earlier_inputs(), 1);
        version.validate().unwrap();
    }

    /// A short conversation records exactly what it did before windows
    /// existed: no new field on the wire, the same digest.
    #[test]
    fn a_short_conversation_s_revisions_are_unchanged_on_the_wire() {
        let mut state = GoalContextState::default();
        let version = state
            .transition(CharterSnapshot::disabled(), Some(input("only line")))
            .unwrap()
            .unwrap();
        let wire = serde_json::to_value(&version).unwrap();
        assert!(wire.get("earlier").is_none(), "{wire}");
        assert!(!version.owner_prompt_block().contains("most recent"));
    }

    /// A compact binding is the same size whatever the conversation's
    /// length, still tells one request from another, and says whether the
    /// history it sent is a prefix of the conversation in hand.
    #[test]
    fn a_compact_binding_keeps_digests_in_place_of_copies() {
        let conversation: Vec<Message> = (0..400)
            .map(|n| Message::user(format!("message {n} {}", "x".repeat(200))))
            .collect();
        let size = |upto: usize| {
            let sent = &conversation[..upto];
            let binding =
                DecisionBinding::compact(1, 1, sent, Some("system"), true, 0, sent, None).unwrap();
            binding.validate().unwrap();
            serde_json::to_vec(&binding).unwrap().len()
        };
        assert!(size(400) <= size(2) + 16, "{} {}", size(400), size(2));
        let full = DecisionBinding::new(1, 1, &conversation, Some("system".into()), true, 0)
            .unwrap()
            .with_conversation_messages(&conversation)
            .unwrap();
        assert!(serde_json::to_vec(&full).unwrap().len() > 100 * size(400));
        // A full binding's wire form has no trace of the compact one.
        assert!(serde_json::to_value(&full)
            .unwrap()
            .get("compact")
            .is_none());
        assert!(!full.is_compact() && full.conversation_shape().is_none());

        let sent = &conversation[..10];
        let binding =
            DecisionBinding::compact(1, 1, sent, Some("system"), true, 0, sent, None).unwrap();
        binding.verify_messages(sent).unwrap();
        assert!(binding.verify_messages(&conversation[..9]).is_err());
        binding.verify_system(Some("system")).unwrap();
        assert!(binding.verify_system(Some("another")).is_err());
        assert!(binding.verify_system(None).is_err());
        assert!(binding.conversation_messages().is_none() && binding.messages().is_empty());

        // The head is over all but the last message: the history is a
        // prefix of the conversation, and of one whose tenth message
        // differs (a line folded into the tail), and not of one that
        // differs before it.
        let (count, head) = binding.conversation_shape().unwrap();
        assert_eq!(count, 10);
        assert_eq!(head, history_heads(&conversation).unwrap()[9]);
        let mut folded = conversation.clone();
        folded[9] = Message::user("the tail with a line folded in");
        assert_eq!(head, history_heads(&folded).unwrap()[9]);
        let mut rewritten = conversation.clone();
        rewritten[3] = Message::user("a different past");
        assert_ne!(head, history_heads(&rewritten).unwrap()[9]);

        // A compact binding that carries a copy after all, or whose digests
        // are not digests, does not validate.
        // The system prompt is kept where it changes and left out where it
        // is the previous decision's; either way the digest is the same,
        // and a replay gives it back.
        assert_eq!(binding.system(), Some("system"));
        let next =
            DecisionBinding::compact(2, 1, sent, Some("system"), false, 0, sent, Some(&binding))
                .unwrap();
        next.validate().unwrap();
        assert!(next.system().is_none());
        assert_eq!(next.system_digest(), binding.system_digest());
        assert_eq!(
            next.clone().with_system_from(binding.system()).system(),
            Some("system")
        );
        assert!(next
            .clone()
            .with_system_from(Some("another"))
            .system()
            .is_none());
        let changed =
            DecisionBinding::compact(3, 1, sent, Some("revised"), false, 0, sent, Some(&next))
                .unwrap();
        assert_eq!(changed.system(), Some("revised"));

        let mut forged = binding.clone();
        forged.messages = sent.to_vec();
        forged.request_digest = forged.computed_request_digest().unwrap();
        assert!(forged.validate().is_err());
        let mut forged = binding.clone();
        forged.compact.as_mut().unwrap().conversation_head = "short".into();
        forged.request_digest = forged.computed_request_digest().unwrap();
        assert!(forged.validate().is_err());
        let mut forged = binding;
        forged.compact.as_mut().unwrap().conversation += 1;
        assert!(forged.validate().is_err(), "the digest covers the shape");
    }
}
