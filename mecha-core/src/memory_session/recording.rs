use super::authority::{digest, Attempt, Frozen};
use super::*;
use crate::{
    memory::{SourceKey, ORDINARY_OBSERVATION_PROTOCOL, ORDINARY_RECORDER_VERSION},
    message::{Block, Role},
    session::{Record, SessionKind},
};
use std::collections::BTreeMap;
/// What a memory session needs of its tools: `goal_context` among them, and
/// no name twice. Which other tools run is the session's own business; the
/// recorder stores what each returned beside where it came from.
pub(super) fn surface(names: &[String]) -> Result<()> {
    ensure!(
        names
            .iter()
            .any(|n| n == "goal_context" || n == recall::SEARCH_NAME)
            && names.iter().collect::<BTreeSet<_>>().len() == names.len(),
        "ordinary memory requires its recall tool among uniquely named tools"
    );
    Ok(())
}
/// The closed set a web memory chat still records: its strict walk admits
/// no other call, and its host pins the same four names.
pub(super) fn closed_surface(names: &[String]) -> Result<()> {
    surface(names)?;
    ensure!(
        names.iter().all(|n| matches!(
            n.as_str(),
            "fs_read" | "fs_write" | "fs_list" | "goal_context"
        )),
        "chat memory requires an explicit supported tool surface"
    );
    Ok(())
}
/// A one-shot run's header: an ordinary run, or one an experiment started
/// in a trial home. The experiment's mark hides the session from every
/// corpus reader; the store it is remembered in is the trial home's own,
/// which no real session shares.
///
/// The mark is not proof of a trial: it is environment, and can be set by
/// hand. An experiment's header is admitted only where the store is a
/// trial's own, which is a fact about the home this process runs in: it
/// carries the runner's marker (`experiment::in_experiment_home`).
fn run_kind(kind: Option<SessionKind>) -> bool {
    run_kind_in(kind, crate::experiment::in_experiment_home())
}
fn run_kind_in(kind: Option<SessionKind>, experiment_home: bool) -> bool {
    match kind {
        Some(SessionKind::Run) => true,
        Some(SessionKind::Experiment) => experiment_home,
        _ => false,
    }
}
/// The head of the owner's prompt that a criterion's text fields admit.
pub(super) fn criterion_text(prompt: &str) -> String {
    // Of the trimmed prompt: admission refuses a blank criterion, and a
    // prompt may open with more whitespace than the head holds.
    let prompt = prompt.trim();
    let mut end = prompt.len().min(2048);
    while !prompt.is_char_boundary(end) {
        end -= 1;
    }
    prompt[..end].into()
}
/// What a call id or a tool name must be to appear in a receipt.
fn nameable(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.trim() == value
        && !value.chars().any(char::is_control)
}
/// One capture's worth of a run. A run that is larger keeps the owner's
/// request, the model's last text and as many passages from the start as fit
/// between them; the transcript still holds every line. A result is kept
/// only with the invocation it answers, and a receipt only with its
/// invocation, which is what admission requires of them.
fn fit(
    units: Vec<(OriginalPassage, Option<String>)>,
    receipts: Vec<ToolReceipt>,
    rest: usize,
) -> Result<(Vec<OriginalPassage>, Vec<ToolReceipt>)> {
    // Room left once everything else in the capture is counted, measured
    // rather than estimated: the identity, the criterion or the chat's
    // context, the scopes, and every record the span consulted. A
    // dependency is never dropped to fit a passage.
    let room = MAX_CAPTURE_BYTES.saturating_sub(rest).saturating_sub(1024);
    let last_text = units
        .iter()
        .rposition(|(p, _)| p.kind == PassageKind::AssistantText);
    let must = |index: usize, p: &OriginalPassage| {
        p.kind == PassageKind::UserRequest || Some(index) == last_text
    };
    let mut costs = Vec::with_capacity(units.len());
    for (p, _) in &units {
        costs.push(serde_json::to_vec(p)?.len() + 1);
    }
    // Indexed once: a run's call count is bounded only by its transcript.
    let receipt_costs: BTreeMap<&str, usize> = receipts
        .iter()
        .map(|r| (r.call_id.as_str(), r.call_id.len() + r.tool_name.len() + 64))
        .collect();
    let mut bytes: usize = units
        .iter()
        .enumerate()
        .filter(|(i, (p, _))| must(*i, p))
        .map(|(i, _)| costs[i])
        .sum();
    let mut count = units
        .iter()
        .enumerate()
        .filter(|(i, (p, _))| must(*i, p))
        .count();
    let mut invoked = BTreeSet::new();
    let mut full = false;
    let mut kept = Vec::new();
    for (index, (passage, call)) in units.into_iter().enumerate() {
        if must(index, &passage) {
            kept.push(passage);
            continue;
        }
        if full {
            continue;
        }
        let invocation = passage.kind == PassageKind::ActionInvocation;
        let cost = costs[index]
            + call
                .as_deref()
                .filter(|_| invocation)
                .and_then(|id| receipt_costs.get(id))
                .copied()
                .unwrap_or(0);
        let answered = passage.kind != PassageKind::ToolResult
            || call.as_ref().is_some_and(|id| invoked.contains(id));
        full |= !answered
            || count + 1 > MAX_ORIGINALS
            || bytes + cost > room
            || (invocation && invoked.len() + 1 > MAX_RECEIPTS);
        if full {
            continue;
        }
        if let (true, Some(id)) = (invocation, call) {
            invoked.insert(id);
        }
        bytes += cost;
        count += 1;
        kept.push(passage);
    }
    let receipts = receipts
        .into_iter()
        .filter(|r| invoked.contains(&r.call_id))
        .collect();
    Ok((kept, receipts))
}
/// Bound the reader payload without changing native input identity or recall matching.
pub(super) fn action_invocation(
    call_id: &str,
    tool_name: &str,
    input: &serde_json::Value,
) -> Result<ActionInvocation> {
    let bytes = serde_json::to_vec(input)?;
    let complete = bytes.len() <= 1024;
    Ok(ActionInvocation {
        call_id: call_id.into(),
        tool_name: tool_name.into(),
        input_status: if complete {
            InputStatus::Complete
        } else {
            InputStatus::Oversized
        },
        input: if complete {
            input.clone()
        } else {
            serde_json::Value::Null
        },
        input_digest: digest(&bytes),
    })
}
pub(super) fn records(bytes: &[u8]) -> Result<Vec<Record>> {
    ensure!(
        bytes.len() <= authority::SOURCE_MAX && bytes.ends_with(b"\n"),
        "ordinary transcript incomplete"
    );
    let text = std::str::from_utf8(bytes)?;
    text.lines()
        .map(|line| {
            ensure!(!line.is_empty(), "ordinary transcript malformed");
            let v: crate::memory::plan_extraction::UniqueValue = serde_json::from_str(line)?;
            let record: Record = serde_json::from_value(v.0.clone())?;
            // Compare with the encoding Session::append writes. Serializing
            // directly to Value widens f32 fields such as the load average.
            let encoded: serde_json::Value =
                serde_json::from_str(&serde_json::to_string(&record)?)?;
            ensure!(
                encoded == v.0,
                "ordinary transcript unknown or lossy fields"
            );
            Ok(record)
        })
        .collect()
}
pub(super) fn initial(
    bytes: &[u8],
    session: &Session,
    prompt: &str,
    goal: &GoalRef,
    workspace: &Path,
) -> Result<()> {
    let rs = records(bytes)?;
    let mut meta = 0;
    let mut config = 0;
    let mut anchor = 0;
    let mut owner = 0;
    for (i, r) in rs.iter().enumerate() {
        match r {
            Record::Meta(m) => {
                ensure!(
                    i == 0
                        && run_kind(m.kind)
                        && serde_json::to_value(m)? == serde_json::to_value(&session.meta)?
                        && m.workspace.canonicalize()? == workspace,
                    "ordinary run header unavailable"
                );
                meta += 1;
            }
            Record::Config(c) => {
                ensure!(
                    owner == 0 && c.workspace.canonicalize()? == workspace,
                    "ordinary configuration unavailable"
                );
                surface(&c.tools)?;
                config += 1;
            }
            Record::GoalAnchor { goal: g } => {
                ensure!(g.as_ref() == Some(goal), "ordinary goal changed");
                anchor += 1;
            }
            Record::Message(m) => {
                ensure!(
                    config == 1
                        && anchor > 0
                        && m.role == Role::User
                        && !m.harness
                        && matches!(m.content.first(), Some(Block::Text { text }) if text == prompt),
                    "ordinary begin is not fresh"
                );
                owner += 1;
            }
            // Owner-made exceptions and carried notes are facts about the
            // conversation, written before its first message as readily as
            // after; neither makes the run less fresh.
            Record::Grants { .. } | Record::Notes { .. } => {}
            _ => anyhow::bail!("ordinary begin is not fresh"),
        }
    }
    ensure!(
        meta == 1 && config <= 1 && anchor >= 1 && owner <= 1,
        "ordinary begin is not fresh"
    );
    Ok(())
}
/// The part of a recorder's walk that reads messages, shared by the one-shot
/// and the chat recorder so the two cannot drift apart on what a passage is.
///
/// The rule: a check on *whose run this is* refuses the capture (a recall
/// that does not reconcile with what the tool returned, a grant that reaches
/// the store). A record or block whose *shape* the walk does not place is
/// left out of the capture and reported to `intake`, which settles against
/// the recorded taint whether what the model wrote can still be stored as
/// local.
pub(super) struct Walk<'a> {
    account: &'a str,
    session_id: &'a str,
    owner_text: &'a str,
    attempts: &'a [Attempt],
    memory: &'a Memory,
    access: &'a Access,
    pub(super) intake: Intake,
    /// Each passage with the call it belongs to, if any, so a span too
    /// large for one capture can be cut on whole calls (`fit`).
    units: Vec<(OriginalPassage, Option<String>)>,
    receipts: Vec<ToolReceipt>,
    calls: BTreeMap<String, (String, serde_json::Value, bool)>,
    matched: BTreeSet<usize>,
    /// Of those, the ones the harness delivered.
    delivered: BTreeSet<usize>,
    pub(super) consulted: Vec<crate::memory::RevisionRef>,
    pub(super) configured: BTreeSet<String>,
    /// Where the owner's request was found: its line and pointer.
    pub(super) owner: Option<(u64, String)>,
    /// Lines the owner typed while the turn ran, which the host has
    /// authenticated as owner inputs and the walk has yet to find; and
    /// where each one found was: line, pointer, text.
    steers: Vec<String>,
    pub(super) steered: Vec<(u64, String, String)>,
    /// Records of this span that carried messages.
    pub(super) messages: usize,
    /// The conversation's messages before this span and the messages this
    /// walk has read, each by digest and in order, and the earlier turns'
    /// call ids. A rewrite lists the conversation again; these say which
    /// of its entries are not new.
    earlier: Vec<String>,
    walked: Vec<String>,
    earlier_calls: BTreeSet<String>,
}
/// What a finished walk hands the recorder.
pub(super) struct Walked {
    pub(super) originals: Vec<OriginalPassage>,
    pub(super) receipts: Vec<ToolReceipt>,
    pub(super) consulted: Vec<crate::memory::RevisionRef>,
}
impl<'a> Walk<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        account: &'a str,
        session_id: &'a str,
        owner_text: &'a str,
        attempts: &'a [Attempt],
        memory: &'a Memory,
        access: &'a Access,
        intake: Intake,
        consulted: Vec<crate::memory::RevisionRef>,
    ) -> Self {
        Self {
            account,
            session_id,
            owner_text,
            attempts,
            memory,
            access,
            intake,
            units: vec![],
            receipts: vec![],
            calls: BTreeMap::new(),
            matched: BTreeSet::new(),
            delivered: BTreeSet::new(),
            consulted,
            configured: BTreeSet::new(),
            owner: None,
            steers: vec![],
            steered: vec![],
            messages: 0,
            earlier: vec![],
            walked: vec![],
            earlier_calls: BTreeSet::new(),
        }
    }
    /// The conversation as it stood before this span: an earlier turn's
    /// messages are never this turn's passages, however a rewrite lists them.
    pub(super) fn hold(&mut self, earlier: &[crate::message::Message]) -> Result<()> {
        for m in earlier {
            self.earlier.push(digest(&serde_json::to_vec(m)?));
            for b in &m.content {
                if let Block::ToolUse { id, .. } = b {
                    self.earlier_calls.insert(id.clone());
                }
            }
        }
        Ok(())
    }
    /// Owner inputs the host authenticated after the turn began: a line
    /// typed while the model was working is folded into the message that
    /// carries the tool results, and is the owner's as much as the first.
    pub(super) fn expect_steers(&mut self, texts: Vec<String>) {
        self.steers = texts;
    }
    /// Take back a steer passage the capture has no owner evidence for.
    pub(super) fn forget_steer(&mut self, line: u64, pointer: &str) {
        self.units
            .retain(|(p, _)| !(p.line == line && p.pointer == pointer));
        self.steered
            .retain(|(l, p, _)| !(*l == line && p == pointer));
        self.intake.unplaced();
    }
    /// A note carried beside the messages.
    pub(super) fn notes(&mut self, notes: &[String]) {
        let mut carried = crate::agent::Taint::default();
        carried.arm_for_notes(notes);
        if carried.untrusted {
            self.intake.ambient();
        }
    }
    /// Harness text folded into a message already on file: a nudge, a
    /// calendar line, a steer. Never a passage, and outside content only if
    /// the harness's own rule or the recorded taint says so.
    ///
    /// Except a line the owner typed while the turn ran, which the host has
    /// authenticated as an owner input (`expect_steers`): typed before the
    /// turn's first tool round it is folded into the owner's own message,
    /// and the transcript records that fold as an extension.
    pub(super) fn extension(&mut self, line: usize, blocks: Vec<Block>) {
        let folded = crate::message::Message {
            role: Role::User,
            content: blocks,
            ..crate::message::Message::user("")
        };
        let mut content = crate::agent::Taint::default();
        content.arm_for_content(std::slice::from_ref(&folded));
        if content.untrusted {
            self.intake.ambient();
        }
        for (index, block) in folded.content.into_iter().enumerate() {
            let Block::Text { text } = block else {
                self.intake.unplaced();
                continue;
            };
            let Some(at) = self.steers.iter().position(|steer| *steer == text) else {
                if !supported_note(&text, &self.configured, &self.calls) {
                    self.intake.unplaced();
                }
                continue;
            };
            self.steers.remove(at);
            let pointer = format!("/blocks/{index}/text");
            self.steered
                .push(((line + 1) as u64, pointer.clone(), text.clone()));
            self.units.push((
                OriginalPassage {
                    source: SourceKey {
                        connector: "host-session-field-v1".into(),
                        account: self.account.into(),
                        id: format!("{}:{}:{pointer}", self.session_id, line + 1),
                    },
                    kind: PassageKind::UserRequest,
                    line: (line + 1) as u64,
                    pointer,
                    text,
                    occurred_at: None,
                    external: self.intake.owner(),
                    source_bytes: None,
                },
                None,
            ));
        }
    }
    /// One message record. The first of a span may open with the owner's
    /// request.
    pub(super) fn message(&mut self, line: usize, m: crate::message::Message) -> Result<()> {
        let owner_at = (self.messages == 0).then_some(0);
        self.messages += 1;
        self.blocks(line, String::new(), m, false, owner_at)
    }
    /// A rewrite carries the whole current list in place of the tail: after
    /// a compaction, an eviction, or an owner's line folded into a message
    /// that already held tool results. What it lists that is new exists
    /// nowhere else in the transcript, so it is read here.
    ///
    /// Which entries are new is decided by position, never by content
    /// alone: a reply byte-identical to an earlier one ("Done.") is still
    /// a reply. Where the list holds this span's owner message, everything
    /// before it is an earlier turn's and everything after is this span's,
    /// less what the walk already read, matched in order. Where it does
    /// not (a compaction summarised it away), the entries are matched in
    /// order against the earlier conversation and then the walk, so a new
    /// message identical to one the compaction dropped can still be taken
    /// for the old one; that costs the passage, never its label.
    pub(super) fn rewrite(
        &mut self,
        line: usize,
        messages: Vec<crate::message::Message>,
    ) -> Result<()> {
        // An owner's line folded into the tail is that message's last block.
        let folded = self.messages == 0 && self.owner.is_none();
        self.messages += 1;
        let digests = messages
            .iter()
            .map(|m| Ok(digest(&serde_json::to_vec(m)?)))
            .collect::<Result<Vec<_>>>()?;
        let owner_text = self.owner_text;
        let anchor = if folded {
            messages
                .len()
                .checked_sub(1)
                .filter(|last| messages[*last].role == Role::User)
        } else {
            messages.iter().rposition(|m| {
                m.role == Role::User
                    && !m.harness
                    && m.content
                        .iter()
                        .any(|b| matches!(b, Block::Text { text } if text == owner_text))
            })
        };
        let known: Vec<String> = match anchor {
            Some(_) => self.walked.clone(),
            None => self.earlier.iter().chain(&self.walked).cloned().collect(),
        };
        let mut cursor = 0;
        for (index, m) in messages.into_iter().enumerate() {
            if anchor.is_some_and(|anchor| index < anchor) {
                continue;
            }
            if let Some(at) = known[cursor..].iter().position(|d| *d == digests[index]) {
                cursor += at + 1;
                continue;
            }
            let owner_at =
                (folded && Some(index) == anchor).then(|| m.content.len().saturating_sub(1));
            self.blocks(line, format!("/messages/{index}"), m, true, owner_at)?;
        }
        Ok(())
    }
    fn blocks(
        &mut self,
        line: usize,
        base: String,
        mut m: crate::message::Message,
        listed: bool,
        owner_at: Option<usize>,
    ) -> Result<()> {
        self.walked.push(digest(&serde_json::to_vec(&m)?));
        // The harness's own content rule, asked of this message: a
        // user-role block it reads as untrusted is outside content no
        // tool result accounts for.
        let mut content = crate::agent::Taint::default();
        content.arm_for_content(std::slice::from_ref(&m));
        if content.untrusted {
            self.intake.ambient();
        }
        // What the loop recorded beside each result: whether its bytes
        // came from outside. Read before the blocks are consumed.
        let provenance = std::mem::take(&mut m.tool_provenance);
        for (index, b) in m.content.into_iter().enumerate() {
            let external: Option<bool>;
            let call: Option<String>;
            let (kind, field, text) = match b {
                // The full native transcript stays bound by the seal.
                // Reasoning and its opaque signature are audit data.
                Block::Thinking { .. } if m.role == Role::Assistant => continue,
                Block::Text { text } if m.role == Role::User => {
                    // The frozen owner block wins even if its bytes happen
                    // to equal a harness notice.
                    if self.owner.is_none()
                        && owner_at == Some(index)
                        && !m.harness
                        && text == self.owner_text
                    {
                        self.owner =
                            Some(((line + 1) as u64, format!("{base}/content/{index}/text")));
                        external = self.intake.owner();
                        call = None;
                        (PassageKind::UserRequest, "text", text)
                    } else if let Some(at) = self
                        .steers
                        .iter()
                        .position(|steer| !m.harness && *steer == text)
                    {
                        self.steers.remove(at);
                        let pointer = format!("{base}/content/{index}/text");
                        self.steered
                            .push(((line + 1) as u64, pointer, text.clone()));
                        external = self.intake.owner();
                        call = None;
                        (PassageKind::UserRequest, "text", text)
                    } else if self.delivery(&text)? {
                        continue;
                    } else {
                        // Any other user-role text is the harness's or
                        // another agent's, and never owner evidence. A
                        // notice known byte for byte is local; anything
                        // else is for the recorded taint to classify.
                        // (The owner's line listed again beside something
                        // new is the line the walk already has.)
                        let again = listed && text == self.owner_text && !m.harness;
                        if !again && !supported_note(&text, &self.configured, &self.calls) {
                            self.intake.unplaced();
                        }
                        continue;
                    }
                }
                Block::Text { text } => {
                    external = self.intake.authored();
                    call = None;
                    (PassageKind::AssistantText, "text", text)
                }
                Block::ToolUse {
                    id: call_id,
                    name,
                    input,
                } => {
                    if listed
                        && (self.earlier_calls.contains(&call_id)
                            || self.calls.contains_key(&call_id))
                    {
                        continue;
                    }
                    if m.role != Role::Assistant
                        || self.calls.contains_key(&call_id)
                        || !nameable(&call_id)
                        || !nameable(&name)
                    {
                        self.intake.unplaced();
                        continue;
                    }
                    external = self.intake.authored();
                    let action = action_invocation(&call_id, &name, &input)?;
                    call = Some(call_id.clone());
                    // Recall reconciliation needs the complete native input, even
                    // when the reader-facing invocation omits its contents.
                    self.calls.insert(call_id, (name, input, false));
                    (
                        PassageKind::ActionInvocation,
                        "input",
                        serde_json::to_string(&action)?,
                    )
                }
                Block::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => {
                    // Listed again: an earlier turn's result, or one this
                    // walk has already read.
                    if listed
                        && (self.earlier_calls.contains(&tool_use_id)
                            || self.calls.get(&tool_use_id).is_some_and(|call| call.2))
                    {
                        continue;
                    }
                    // Counted before anything can skip it: a result the
                    // walk cannot pair still entered the conversation.
                    external = self.intake.result(provenance.get(&tool_use_id).copied());
                    let Some((name, input, complete)) = self
                        .calls
                        .get_mut(&tool_use_id)
                        .filter(|call| !call.2 && m.role == Role::User)
                    else {
                        continue;
                    };
                    *complete = true;
                    self.receipts.push(ToolReceipt {
                        call_id: tool_use_id.clone(),
                        tool_name: name.clone(),
                        status: if is_error {
                            ReceiptStatus::Failed
                        } else {
                            ReceiptStatus::Succeeded
                        },
                    });
                    // The same tool under its plain name: the tracked
                    // attempt holds what it forwarded.
                    let searched =
                        (name == recall::SEARCH_NAME).then(|| recall::search_as_recall(input));
                    let input: &serde_json::Value = searched.as_ref().unwrap_or(&*input);
                    if name == "goal_context" || searched.is_some() {
                        // An outside result is wrapped under the name of the
                        // tool that was called; the packet's readers know
                        // the tracked tool's.
                        let content = match content
                            .strip_prefix("<untrusted-content source=\"memory_search\">")
                            .filter(|_| searched.is_some())
                        {
                            Some(rest) => {
                                format!("<untrusted-content source=\"goal_context\">{rest}")
                            }
                            None => content.clone(),
                        };
                        let candidates: Vec<_> = self
                            .attempts
                            .iter()
                            .enumerate()
                            .filter(|(at, a)| {
                                // A recall the harness delivered is spent;
                                // a model's identical call is its own.
                                !self.delivered.contains(at)
                                    && a.input == *input
                                    && a.is_error == is_error
                                    && recall::exact_output(&a.output, &content, a.external)
                            })
                            .collect();
                        ensure!(
                            candidates.len() == 1 && self.matched.insert(candidates[0].0),
                            "ordinary recall delivery ambiguous"
                        );
                        let a = candidates[0].1;
                        let refs = if !a.is_error && input.get("memory").is_some() {
                            crate::memory_experience_bridge::memory_packet_references_checked(
                                &content,
                                Some((self.memory, self.access)),
                            )?
                        } else {
                            ensure!(!a.external, "ordinary failed recall delivery unknown");
                            vec![]
                        };
                        ensure!(refs == a.references, "ordinary recall dependencies changed");
                        self.memory
                            .validate_delivered_context(&refs, &[], self.access)?;
                        for r in refs {
                            if !self.consulted.contains(&r) {
                                self.consulted.push(r);
                            }
                        }
                        continue;
                    }
                    call = Some(tool_use_id);
                    (PassageKind::ToolResult, "content", content)
                }
                // An image, or a block in a role the walk has no passage
                // for.
                _ => {
                    self.intake.unplaced();
                    continue;
                }
            };
            // Nothing to remember in an empty or blank field, and
            // admission would refuse the whole capture for it.
            if text.trim().is_empty() {
                continue;
            }
            // An invocation is never cut here: `action_invocation` omits an
            // input over 1,024 bytes, so its JSON is well under a passage.
            // A cut one would not parse at admission and would refuse the
            // capture.
            let (text, source_bytes) = excerpt(text);
            let pointer = format!("{base}/content/{index}/{field}");
            self.units.push((
                OriginalPassage {
                    source: SourceKey {
                        connector: if kind == PassageKind::ActionInvocation {
                            "host-action-invocation-v1"
                        } else {
                            "host-session-field-v1"
                        }
                        .into(),
                        account: self.account.into(),
                        id: format!("{}:{}:{pointer}", self.session_id, line + 1),
                    },
                    kind,
                    line: (line + 1) as u64,
                    pointer,
                    text,
                    occurred_at: None,
                    external,
                    source_bytes,
                },
                call,
            ));
        }
        Ok(())
    }
    /// A recall the harness made before the run, delivered as text beside
    /// the owner's words (`recall::DELIVERY_STEM`). It is reconciled with its
    /// tracked attempt as a model's call is with its result, counted as a
    /// result that entered the conversation, and never read as a passage.
    /// Text under the stem that matches no attempt is not a delivery.
    fn delivery(&mut self, text: &str) -> Result<bool> {
        let Some(content) = text
            .strip_prefix(recall::DELIVERY_STEM)
            .or_else(|| text.strip_prefix(recall::ERROR_STEM))
        else {
            return Ok(false);
        };
        let Some((at, a)) = self.attempts.iter().enumerate().find(|(at, a)| {
            !self.matched.contains(at)
                && recall::delivery_shaped(&a.input)
                && recall::exact_output(&a.output, content, a.external)
        }) else {
            return Ok(false);
        };
        self.matched.insert(at);
        self.delivered.insert(at);
        self.intake.result(Some(a.external));
        let refs = if a.is_error {
            ensure!(!a.external, "ordinary failed recall delivery unknown");
            vec![]
        } else {
            crate::memory_experience_bridge::memory_packet_references_checked(
                content,
                Some((self.memory, self.access)),
            )?
        };
        ensure!(refs == a.references, "ordinary recall dependencies changed");
        self.memory
            .validate_delivered_context(&refs, &[], self.access)?;
        for r in refs {
            if !self.consulted.contains(&r) {
                self.consulted.push(r);
            }
        }
        Ok(true)
    }
    /// Every recall the tool served must have been found in the transcript.
    pub(super) fn reconciled(&self) -> bool {
        self.matched.len() == self.attempts.len()
    }
    /// Settle provenance against the recorded taint, then cut to one
    /// capture. `rest` is what the capture serializes to without a passage
    /// or a receipt in it.
    pub(super) fn finish(
        mut self,
        recorded_untrusted: Option<bool>,
        rest: usize,
    ) -> Result<Walked> {
        // Settled once the whole span is read, not at the taint record: a
        // record the walk cannot place may follow it, and must still count.
        let mut originals: Vec<_> = self.units.iter().map(|(p, _)| p.clone()).collect();
        self.intake.settle(recorded_untrusted, &mut originals);
        for ((passage, _), settled) in self.units.iter_mut().zip(originals) {
            *passage = settled;
        }
        // A call the run stopped before answering is still a call the model
        // made; its receipt says no result is available.
        for (call_id, (name, _, complete)) in &self.calls {
            if !complete {
                self.receipts.push(ToolReceipt {
                    call_id: call_id.clone(),
                    tool_name: name.clone(),
                    status: ReceiptStatus::Unavailable,
                });
            }
        }
        let (originals, receipts) = fit(self.units, self.receipts, rest)?;
        Ok(Walked {
            originals,
            receipts,
            consulted: self.consulted,
        })
    }
}
/// What `payload` serializes to before a passage or a receipt is in it: the
/// room `fit` must leave.
pub(super) fn rest_of(payload: &HostCapture) -> Result<usize> {
    Ok(serde_json::to_vec(payload)?.len())
}
pub(super) fn capture(
    f: &Frozen,
    bytes: &[u8],
    attempts: &[Attempt],
    memory: &Memory,
    access: &Access,
    id: &str,
) -> Result<HostCapture> {
    let mut meta = 0;
    let mut config = 0;
    let mut anchors = 0;
    let mut taint = false;
    let mut recorded_untrusted = None;
    let mut ended = false;
    // A one-shot run is a whole conversation, so it starts having taken
    // in nothing.
    let mut walk = Walk::new(
        &f.request.account,
        &f.session_id,
        &f.prompt,
        attempts,
        memory,
        access,
        Intake::starting(false),
        vec![],
    );
    for (line, r) in records(bytes)?.into_iter().enumerate() {
        ensure!(!ended, "ordinary transcript appended after outcome");
        match r {
            Record::Meta(m) => {
                ensure!(
                    line == 0
                        && run_kind(m.kind)
                        && m.id == f.session_id
                        && m.workspace.canonicalize()? == f.workspace,
                    "ordinary run header unavailable"
                );
                meta += 1;
            }
            Record::Config(c) => {
                ensure!(
                    walk.messages == 0 && c.workspace.canonicalize()? == f.workspace,
                    "ordinary configuration unavailable"
                );
                surface(&c.tools)?;
                walk.configured.extend(c.tools);
                config += 1;
            }
            Record::GoalAnchor { goal } => {
                ensure!(
                    goal.is_some_and(|g| g.to_string() == f.goal),
                    "ordinary goal changed"
                );
                anchors += 1;
            }
            Record::Notes { notes } => walk.notes(&notes),
            Record::PendingNote { note } => {
                walk.notes(std::slice::from_ref(&note));
                walk.intake.unplaced();
            }
            // A deferred tool's result, delivered after its turn. The record
            // says where its bytes came from, so it taints what follows as
            // any result does; the bytes themselves are not a passage yet.
            // Nothing writes this into a one-shot's transcript today: both
            // writers are the server's, and each follows it with a second
            // taint record this walk still refuses.
            Record::LateResult { external, .. } => {
                walk.intake.result(Some(external));
            }
            // An owner-made exception to the jail is a fact about whose run
            // this is when it reaches the store or this transcript: a tool
            // could then have read the key or rewritten what is sealed.
            Record::Grants { grants } => {
                authority::exclude_grants(&f.request, Some(&f.source), &grants)?;
            }
            // Nothing here is content the model read that a message or a
            // result does not also carry: what a run cost, a title, the
            // audit of a recall decision.
            Record::Summary { .. }
            | Record::Title { .. }
            | Record::ChatMemory { .. }
            | Record::GoalContextDecision { .. } => {}
            // A rewrite that lists nothing carries nothing to read.
            Record::Rewrite { messages } if messages.is_empty() => {}
            Record::Rewrite { messages } => {
                ensure!(
                    !taint && config == 1 && anchors > 0,
                    "ordinary message unsupported"
                );
                walk.rewrite(line, messages)?;
            }
            Record::Taint(recorded) => {
                ensure!(
                    attempts.is_empty() || recorded.private,
                    "ordinary private recall taint missing"
                );
                ensure!(
                    !attempts.iter().any(|a| a.external) || recorded.untrusted,
                    "ordinary external recall taint missing"
                );
                ensure!(!taint && walk.messages > 0, "ordinary taint unavailable");
                taint = true;
                recorded_untrusted = Some(recorded.untrusted);
            }
            // However the run stopped. One that was cancelled, ran out of
            // turns or compacted on the way is still what happened.
            Record::Outcome(_) => {
                ensure!(taint, "ordinary run incomplete");
                ended = true;
            }
            Record::Extend { blocks, .. } => {
                ensure!(!taint, "ordinary inline extension unavailable");
                walk.extension(line, blocks);
            }
            Record::Message(m) => {
                ensure!(
                    !taint && config == 1 && anchors > 0,
                    "ordinary message unsupported"
                );
                walk.message(line, m)?;
            }
            // A record this walk has no reading for: a program's calls, a
            // late failure, a spoken direction, a cleared context.
            _ => walk.intake.unplaced(),
        }
    }
    ensure!(
        meta == 1 && config == 1 && walk.owner.is_some() && ended && walk.reconciled(),
        "ordinary recording incomplete"
    );
    let criterion = criterion_text(&f.prompt);
    let mut payload = HostCapture {
        identity: CaptureIdentity {
            domain: f.request.domain.clone(),
            account: f.request.account.clone(),
            session_id: f.session_id.clone(),
            prefix_digest: digest(bytes),
            check_id: id.into(),
            check_version: ORDINARY_OBSERVATION_PROTOCOL.into(),
            evaluator_version: ORDINARY_RECORDER_VERSION.into(),
        },
        binding: ExternalBinding {
            authority: f.authority.clone(),
            locator: id.into(),
            revision: "0".repeat(64),
        },
        criterion: Some(ConfirmedCriterion {
            goal: f.goal.clone(),
            goal_text: criterion.clone(),
            criterion_text: criterion,
            criterion_revision: digest(f.prompt.as_bytes()),
            confirmation_digest: id.into(),
        }),
        chat_context: None,
        originals: vec![],
        receipts: vec![],
        artifact: ArtifactCheck {
            artifact_digest: None,
            verdict: CheckVerdict::Unavailable,
        },
        consulted: walk.consulted.clone(),
        scopes: f.request.scopes.clone(),
        occurred_at: None,
    };
    let walked = walk.finish(recorded_untrusted, rest_of(&payload)?)?;
    payload.originals = walked.originals;
    payload.receipts = walked.receipts;
    payload.consulted = walked.consulted;
    OrdinaryObservationCapture::try_from_host(payload.clone())?;
    Ok(payload)
}

fn supported_note(
    text: &str,
    configured: &BTreeSet<String>,
    calls: &BTreeMap<String, (String, serde_json::Value, bool)>,
) -> bool {
    let observed: BTreeSet<_> = calls
        .values()
        .filter(|(name, _, complete)| *complete && configured.contains(name))
        .map(|(name, _, _)| name.clone())
        .collect();
    let completed = calls.values().filter(|(_, _, complete)| *complete).count();
    super::inline_notes::supported(text, &observed, completed)
}
#[cfg(test)]
mod run_kind_tests {
    use super::*;
    #[test]
    fn an_experiment_s_run_is_remembered_only_in_a_trial_home() {
        assert!(run_kind_in(Some(SessionKind::Run), false));
        assert!(run_kind_in(Some(SessionKind::Experiment), true));
        assert!(!run_kind_in(Some(SessionKind::Experiment), false));
        for kind in [Some(SessionKind::Test), Some(SessionKind::Chat), None] {
            assert!(!run_kind_in(kind, true), "{kind:?}");
        }
    }
}
