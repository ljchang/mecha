//! Capture of one new span, retaining native absolute source pointers. An
//! ordinary chat's turn is read by the walk the one-shot recorder uses
//! (`capture_open`); the graph and task profiles keep the strict walk below.
use super::*;
use crate::memory::SourceKey;
use crate::memory_session::inline_notes;
use crate::message::{Block, Message, Role};
use authority::{digest, Attempt};
use std::collections::BTreeMap;
pub(super) fn capture(
    memory: &SessionMemory,
    f: &Turn,
    a: &Attachment,
    bytes: &[u8],
    attempts: &[Attempt],
    id: &str,
) -> Result<HostCapture> {
    capture_impl(memory, f, a, bytes, attempts, id, false)
}
/// Only authenticated V4 storage recovery may use its frozen canonical workspace
/// spelling without reopening a directory that may legitimately have been removed.
pub(super) fn capture_task_recovery(
    memory: &SessionMemory,
    f: &Turn,
    a: &Attachment,
    bytes: &[u8],
    attempts: &[Attempt],
    id: &str,
) -> Result<HostCapture> {
    ensure!(
        memory.workspace.is_none() && a.task.is_some() && a.protocol == CHAT_TASK_MEMORY_PROTOCOL,
        "historical workspace comparison requires task recovery authority"
    );
    capture_impl(memory, f, a, bytes, attempts, id, true)
}
fn capture_impl(
    memory: &SessionMemory,
    f: &Turn,
    a: &Attachment,
    bytes: &[u8],
    attempts: &[Attempt],
    id: &str,
    task_recovery: bool,
) -> Result<HostCapture> {
    if a.open() {
        return capture_open(memory, f, a, bytes, attempts, id);
    }
    ensure!(
        bytes.len() > f.start_length as usize
            && digest(&bytes[..f.start_length as usize]) == f.start_digest,
        "chat begin prefix changed"
    );
    validate_source_ingress(a, bytes, &f.attachment)?;
    ensure!(
        f.owner.source() == a.owner_source(),
        "chat owner ingress changed"
    );
    let transcript = Session::parse(&a.source, std::str::from_utf8(bytes)?)?;
    let prior = Session::parse(
        &a.source,
        std::str::from_utf8(&bytes[..f.start_length as usize])?,
    )?;
    let mut expected_owners = prior
        .convo
        .goal_context
        .current()
        .map(|v| v.owner_inputs().to_vec())
        .unwrap_or_default();
    ensure!(
        expected_owners
            .iter()
            .all(|input| input.source() == a.owner_source()),
        "chat prior owner ingress changed"
    );
    expected_owners.push(f.owner.clone());
    let prior_untrusted = prior.convo.taint.untrusted;
    let mut external_results = a.graph_surface.is_some() && prior_untrusted;
    let prior_messages = prior.convo.messages.len();
    let mut expected_history = prior.convo.messages;
    expected_history.push(crate::message::Message::user(f.owner.text()));

    let config = transcript
        .configs
        .last()
        .ok_or_else(|| anyhow::anyhow!("chat configuration missing"))?;
    if task_recovery {
        ensure!(
            config.workspace == a.workspace && a.workspace.is_absolute(),
            "task historical workspace changed"
        );
    } else {
        ensure!(
            config.workspace.canonicalize()? == a.workspace,
            "chat workspace changed"
        );
    }
    a.validate_tools(&config.tools)?;
    ensure!(
        !transcript.convo.goal_context.has_cleared(),
        "chat clear unsupported"
    );
    let start_line = bytes[..f.start_length as usize]
        .iter()
        .filter(|b| **b == b'\n')
        .count();
    let db = memory.memory()?;
    db.validate_dependency_roots(&f.dependencies, &memory.access())?;
    let mut originals = vec![];
    let mut receipts = vec![];
    let mut all_calls = 0usize;
    let mut observed_names = BTreeSet::new();
    let mut calls = BTreeMap::<String, (String, serde_json::Value, bool)>::new();
    let mut matched = BTreeSet::new();
    let mut consulted = f.dependencies.clone();
    let mut decisions = vec![];
    let mut decision_calls = BTreeMap::new();
    let mut used_versions = BTreeSet::new();
    let mut owner_line = None;
    let mut owner = 0;
    let mut messages = 0;
    let mut taint = false;
    // This turn continues a conversation: it begins with whatever the
    // earlier turns' recorded taint says they took in.
    let mut intake = Intake::starting(prior_untrusted);
    let mut ended = false;
    let mut final_assistant = false;
    for (line, r) in recording::records(bytes)?
        .into_iter()
        .enumerate()
        .skip(start_line)
    {
        ensure!(!taint, "chat transcript appended after taint");
        match r {
            Record::GoalContextDecision { binding, .. } => {
                ensure!(!ended, "chat decision after outcome");
                ensure!(
                    binding.run_start() == decisions.is_empty(),
                    "chat decision run boundary changed"
                );
                let version = transcript
                    .convo
                    .goal_context
                    .version(binding.version())
                    .ok_or_else(|| anyhow::anyhow!("chat version unavailable"))?;
                ensure!(
                    version.owner_inputs() == expected_owners,
                    "chat decision owner changed"
                );
                let sent = binding
                    .conversation_messages()
                    .ok_or_else(|| anyhow::anyhow!("chat decision lacks native history"))?;
                // Bindings preserve the exact history at each request. A later
                // empty retry can extend the final user message, so comparing
                // every earlier binding to the final native prefix is wrong.
                // Advance only through the loop's two supported transitions;
                // the final check still requires exact native history coverage.
                if decisions.is_empty() {
                    ensure!(
                        sent == expected_history,
                        "chat initial decision history changed"
                    );
                } else {
                    validate_decision_transition(
                        &expected_history,
                        sent,
                        &config.tools,
                        &mut decision_calls,
                        a.task.is_some(),
                    )?;
                    expected_history = sent.to_vec();
                }
                used_versions.insert(binding.version());
                decisions.push(ChatDecisionLocator::from_binding(
                    (line + 1) as u64,
                    &binding,
                )?);
            }
            Record::Extend { index, blocks } => {
                // record_run persists an initial owner's folded retry as an
                // Extend. Never accept edits to a prior retained conversation,
                // another message, or arbitrary known-looking user content.
                ensure!(
                    !ended
                        && messages == 1
                        && index == prior_messages
                        && !blocks.is_empty()
                        && blocks.iter().all(|block| matches!(block,
                            Block::Text { text } if inline_notes::empty_retry(text))),
                    "chat owner extension unsupported"
                );
            }
            Record::GoalAnchor { goal } => {
                ensure!(goal.is_none(), "chat profile has no confirmed anchor")
            }
            Record::Grants { grants } => ensure!(grants.is_empty(), "ordinary grants unsupported"),
            Record::Notes { notes } => {
                ensure!(
                    notes.len() <= 32 && notes.iter().all(|n| n.len() <= 8192),
                    "ordinary notes oversized"
                );
                let mut carried = crate::agent::Taint::default();
                carried.arm_for_notes(&notes);
                if carried.untrusted {
                    intake.ambient();
                }
            }
            Record::Summary { .. } => {}
            Record::Taint(recorded) => {
                ensure!(recorded.private, "ordinary private recall taint missing");
                ensure!(
                    (!attempts.iter().any(|a| a.external) && !external_results)
                        || recorded.untrusted,
                    "ordinary external recall taint missing"
                );
                ensure!(
                    !taint && ended && messages > 0,
                    "ordinary taint unavailable"
                );
                taint = true;
                // No message follows this record, so every passage is in.
                intake.settle(Some(recorded.untrusted), &mut originals);
            }
            Record::Outcome(s) => {
                ensure!(
                    !ended
                        && s.stop_cause == Some(crate::agent::StopCause::Completed)
                        && s.compactions == 0
                        && s.context_overflows == Some(0)
                        && s.turns as usize == decisions.len(),
                    "ordinary run incomplete"
                );
                ended = true;
            }
            Record::Message(mut m) => {
                ensure!(
                    !ended
                        && messages < 128
                        && !m.harness
                        && (a.task.is_some()
                            || (m.planning.is_none() && m.plan_results.is_empty())),
                    "ordinary message unsupported"
                );
                if a.graph_surface.is_some() {
                    let results: Vec<_> = m
                        .content
                        .iter()
                        .filter_map(|b| {
                            if let Block::ToolResult { tool_use_id, .. } = b {
                                Some(tool_use_id)
                            } else {
                                None
                            }
                        })
                        .collect();
                    let ids: BTreeSet<_> = results.iter().copied().collect();
                    ensure!(
                        ids.len() == results.len()
                            && ids == m.tool_provenance.keys().collect::<BTreeSet<_>>(),
                        "chat result provenance missing, duplicate or unrelated"
                    );
                    external_results |= m.tool_provenance.values().any(|external| *external);
                }
                messages += 1;
                final_assistant = m.role == Role::Assistant
                    && m.content.iter().any(|b| matches!(b, Block::Text { .. }));
                let result_prefix = m
                    .content
                    .iter()
                    .take_while(|block| matches!(block, Block::ToolResult { .. }))
                    .count();
                // What the loop recorded beside each result: whether its bytes
                // came from outside. Read before the blocks are consumed.
                // The harness's own content rule, asked of this message: a
                // user-role block it reads as untrusted is outside content no
                // tool result accounts for.
                let mut content = crate::agent::Taint::default();
                content.arm_for_content(std::slice::from_ref(&m));
                if content.untrusted {
                    intake.ambient();
                }
                let provenance = std::mem::take(&mut m.tool_provenance);
                for (index, b) in m.content.into_iter().enumerate() {
                    let external: Option<bool>;
                    let (kind, field, text) = match b {
                        // The full native transcript stays bound by the seal.
                        // Reasoning and its opaque signature are audit data.
                        Block::Thinking { .. } if m.role == Role::Assistant => continue,
                        Block::Text { text } => {
                            if m.role == Role::User {
                                if owner == 0 && messages == 1 && index == 0 {
                                    ensure!(
                                        text == f.owner.text(),
                                        "ordinary owner prompt changed"
                                    );
                                    owner += 1;
                                    owner_line = Some((line + 1) as u64);
                                    external = intake.owner();
                                    (PassageKind::UserRequest, "text", text)
                                } else {
                                    let observed_tools = observed_names.clone();
                                    ensure!(
                                        result_prefix > 0
                                            && index >= result_prefix
                                            && inline_notes::supported(
                                                &text,
                                                &observed_tools,
                                                all_calls
                                            ),
                                        "ordinary user text is not a supported inline notice"
                                    );
                                    // Retain the raw native block and its decision
                                    // history, but never mint owner evidence for it.
                                    continue;
                                }
                            } else {
                                external = intake.authored();
                                (PassageKind::AssistantText, "text", text)
                            }
                        }
                        Block::ToolUse {
                            id: call_id,
                            name,
                            input,
                        } => {
                            ensure!(
                                m.role == Role::Assistant
                                    && (!calls.contains_key(&call_id)
                                        || (a.task.is_some()
                                            && name == "todo"
                                            && calls
                                                .get(&call_id)
                                                .is_some_and(|(n, _, done)| n == "todo" && *done)))
                                    && all_calls < 32
                                    && a.graph_surface.as_ref().map_or_else(
                                        || matches!(
                                            name.as_str(),
                                            "fs_read" | "fs_write" | "fs_list" | "goal_context"
                                        ) || (a.task.is_some() && name == "todo"),
                                        |graph| graph.contains_tool(&name)
                                    ),
                                "ordinary invocation unsupported"
                            );
                            all_calls += 1;
                            if a.task.is_some() && name == "todo" {
                                calls.insert(call_id, (name, input, false));
                                continue;
                            }
                            external = intake.authored();
                            let action = recording::action_invocation(&call_id, &name, &input)?;
                            // Recall reconciliation needs the complete native input, even
                            // when the reader-facing invocation omits its contents.
                            calls.insert(call_id, (name, input, false));
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
                            external = intake.result(provenance.get(&tool_use_id).copied());
                            ensure!(m.role == Role::User, "ordinary result unsupported");
                            let (name, input, complete) = calls
                                .get_mut(&tool_use_id)
                                .ok_or_else(|| anyhow::anyhow!("ordinary orphan result"))?;
                            ensure!(!*complete, "ordinary duplicate result");
                            *complete = true;
                            observed_names.insert(name.clone());
                            if a.task.is_some() && name == "todo" {
                                continue;
                            }
                            let external = if a.graph_surface.is_some() {
                                provenance.get(&tool_use_id).copied()
                            } else {
                                None
                            };
                            if let Some(graph) = &a.graph_surface {
                                ensure!(
                                    !graph.graph_tools.iter().any(|t| t.local_name == *name)
                                        || external == Some(true)
                                        || is_error,
                                    "chat graph success lacks external provenance"
                                );
                            }
                            receipts.push(ToolReceipt {
                                call_id: tool_use_id,
                                tool_name: name.clone(),
                                status: if is_error {
                                    ReceiptStatus::Failed
                                } else {
                                    ReceiptStatus::Succeeded
                                },
                            });
                            if name == "goal_context" {
                                let candidates: Vec<_> = attempts
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, a)| {
                                        a.input == *input
                                            && external
                                                .is_none_or(|external| a.external == external)
                                            && a.is_error == is_error
                                            && super::super::recall::exact_output(
                                                &a.output, &content, a.external,
                                            )
                                    })
                                    .collect();
                                ensure!(
                                    candidates.len() == 1 && matched.insert(candidates[0].0),
                                    "ordinary recall delivery ambiguous"
                                );
                                let a = candidates[0].1;
                                let refs = if !a.is_error && input.get("memory").is_some() {
                                    crate::memory_experience_bridge::memory_packet_references_checked(&content,Some((&db,&memory.access())))?
                                } else {
                                    ensure!(!a.external, "ordinary failed recall delivery unknown");
                                    vec![]
                                };
                                ensure!(
                                    refs == a.references,
                                    "ordinary recall dependencies changed"
                                );
                                db.validate_delivered_context(&refs, &[], &memory.access())?;
                                for r in refs {
                                    if !consulted.contains(&r) {
                                        consulted.push(r);
                                    }
                                }
                                continue;
                            }
                            (PassageKind::ToolResult, "content", content)
                        }
                        _ => anyhow::bail!("ordinary block unsupported"),
                    };
                    ensure!(
                        !text.is_empty() && text.len() <= 4096,
                        "ordinary passage unavailable"
                    );
                    let pointer = format!("/content/{index}/{field}");
                    originals.push(OriginalPassage {
                        source: SourceKey {
                            connector: if kind == PassageKind::ActionInvocation {
                                "host-action-invocation-v1"
                            } else {
                                "host-session-field-v1"
                            }
                            .into(),
                            account: memory.request.account.clone(),
                            id: format!("{}:{}:{pointer}", f.session_id, line + 1),
                        },
                        kind,
                        line: (line + 1) as u64,
                        pointer,
                        text,
                        occurred_at: None,
                        external,
                        source_bytes: None,
                    });
                }
            }
            _ => anyhow::bail!("ordinary recording unsupported"),
        }
    }
    ensure!(
        owner == 1
            && taint
            && !decisions.is_empty()
            && ended
            && final_assistant
            && calls.values().all(|c| c.2)
            && matched.len() == attempts.len(),
        "ordinary recording incomplete"
    );
    ensure!(
        transcript.convo.messages.len() == expected_history.len() + 1
            && transcript.convo.messages.starts_with(&expected_history),
        "chat final decision does not cover native history"
    );
    let versions: Vec<_> = used_versions
        .iter()
        .map(|revision| {
            let version = transcript
                .convo
                .goal_context
                .version(*revision)
                .expect("validated version");
            if a.web_ingress.is_some() {
                ChatContextVersion::from_authenticated_web_version(version)
            } else {
                ChatContextVersion::from_version(version)
            }
        })
        .collect::<Result<_>>()?;
    let first_version = transcript
        .convo
        .goal_context
        .version(decisions[0].context_revision)
        .expect("validated version");
    let context = ChatObservationContext {
        conversation_id: f.session_id.clone(),
        turn_ordinal: f.ordinal,
        previous_turn: f.previous.clone(),
        prefix_start: f.prefix_start,
        prefix_length: bytes.len() as u64,
        prefix_digest: digest(bytes),
        owner_messages: vec![ChatOwnerMessageLocator {
            line: owner_line.ok_or_else(|| anyhow::anyhow!("chat owner missing"))?,
            pointer: "/content/0/text".into(),
            context_revision: first_version.revision(),
            owner_input_index: first_version.owner_inputs().len() - 1,
        }],
        versions,
        decisions,
    };
    let (observation, recorder) = a.observation_protocol();
    let payload = HostCapture {
        identity: CaptureIdentity {
            domain: memory.request.domain.clone(),
            account: memory.request.account.clone(),
            session_id: f.session_id.clone(),
            prefix_digest: digest(bytes),
            check_id: id.into(),
            check_version: observation.into(),
            evaluator_version: recorder.into(),
        },
        binding: ExternalBinding {
            authority: memory.authority.identity.clone(),
            locator: id.into(),
            revision: "0".repeat(64),
        },
        criterion: None,
        chat_context: Some(context),
        originals,
        receipts,
        artifact: ArtifactCheck {
            artifact_digest: None,
            verdict: CheckVerdict::Unavailable,
        },
        consulted,
        scopes: memory.request.scopes.clone(),
        occurred_at: None,
    };
    OrdinaryObservationCapture::try_from_host(payload.clone())?;
    Ok(payload)
}

/// A retry grows only the current user tail; a normal tool turn appends exactly
/// one assistant/result pair. Neither path can edit or omit earlier content.
fn validate_decision_transition(
    previous: &[Message],
    next: &[Message],
    configured: &[String],
    calls: &mut BTreeMap<String, String>,
    task: bool,
) -> Result<()> {
    if next.len() == previous.len() {
        let index = previous
            .len()
            .checked_sub(1)
            .ok_or_else(|| anyhow::anyhow!("chat retry history missing"))?;
        let old = &previous[index];
        let new = &next[index];
        let mut extended = old.clone();
        extended
            .content
            .push(Block::text(crate::agent::EMPTY_TURN_NUDGE));
        ensure!(
            old.role == Role::User && next[..index] == previous[..index] && *new == extended,
            "chat retry changed native history"
        );
        return Ok(());
    }
    ensure!(
        next.len() == previous.len() + 2 && next.starts_with(previous),
        "chat decision skips or changes native history"
    );
    let response = &next[previous.len()];
    let result = &next[previous.len() + 1];
    let requested: Vec<_> = response
        .content
        .iter()
        .filter_map(|block| {
            if let Block::ToolUse { id, name, .. } = block {
                Some((id, name))
            } else {
                None
            }
        })
        .collect();
    let returned: Vec<_> = result
        .content
        .iter()
        .filter_map(|block| {
            if let Block::ToolResult { tool_use_id, .. } = block {
                Some(tool_use_id)
            } else {
                None
            }
        })
        .collect();
    let requested_ids: BTreeSet<_> = requested.iter().map(|(id, _)| *id).collect();
    let returned_ids: BTreeSet<_> = returned.iter().copied().collect();
    ensure!(
        response.role == Role::Assistant
            && result.role == Role::User
            && !requested.is_empty()
            && requested_ids.len() == requested.len()
            && returned_ids.len() == returned.len()
            && requested_ids == returned_ids,
        "chat decision boundary lacks complete native tool delivery"
    );
    for (id, name) in requested {
        let key = if task && name == "todo" {
            format!("task-occurrence-{}:{id}", calls.len())
        } else {
            id.clone()
        };
        ensure!(
            configured.contains(name)
                && calls.insert(key, name.clone()).is_none()
                && calls.len() <= 32,
            "chat decision tool surface changed"
        );
    }
    let observed = calls.values().cloned().collect();
    let mut notes = false;
    for block in &result.content {
        match block {
            Block::ToolResult { .. } if !notes => {}
            Block::Text { text }
                if !inline_notes::empty_retry(text)
                    && inline_notes::supported(text, &observed, calls.len()) =>
            {
                notes = true
            }
            _ => anyhow::bail!("chat tool delivery contains unsupported inline content"),
        }
    }
    Ok(())
}

/// The most decisions and context versions one turn's sidecar admits.
const MAX_DECISIONS: usize = 128;
const MAX_VERSIONS: usize = 16;
const MAX_CONTEXT_BYTES: usize = 24 * 1024;

/// One turn of an ordinary chat, however it went.
///
/// What still refuses is whose turn this is: the prefix the turn began
/// from, the attachment and its ingress, the workspace, the owner's line
/// as frozen at the start, the owner inputs behind every goal-context
/// decision, a recall that does not reconcile, a grant that reaches the
/// store or this transcript. Everything about the turn's shape is left to
/// the shared walk: which tools ran, how it stopped, whether it compacted,
/// what else was delivered into it.
fn capture_open(
    memory: &SessionMemory,
    f: &Turn,
    a: &Attachment,
    bytes: &[u8],
    attempts: &[Attempt],
    id: &str,
) -> Result<HostCapture> {
    let start = f.start_length as usize;
    ensure!(
        bytes.len() > start && digest(&bytes[..start]) == f.start_digest,
        "chat begin prefix changed"
    );
    validate_source_ingress(a, bytes, &f.attachment)?;
    ensure!(
        f.owner.source() == a.owner_source(),
        "chat owner ingress changed"
    );
    let transcript = Session::parse(&a.source, std::str::from_utf8(bytes)?)?;
    let prior = Session::parse(&a.source, std::str::from_utf8(&bytes[..start])?)?;
    let mut expected_owners = prior
        .convo
        .goal_context
        .current()
        .map(|v| v.owner_inputs().to_vec())
        .unwrap_or_default();
    ensure!(
        expected_owners
            .iter()
            .all(|input| input.source() == a.owner_source()),
        "chat prior owner ingress changed"
    );
    expected_owners.push(f.owner.clone());
    let config = transcript
        .configs
        .last()
        .ok_or_else(|| anyhow::anyhow!("chat configuration missing"))?;
    ensure!(
        config.workspace.canonicalize()? == a.workspace,
        "chat workspace changed"
    );
    a.validate_tools(&config.tools)?;
    ensure!(
        !transcript.convo.goal_context.has_cleared(),
        "chat clear unsupported"
    );
    // Every grant the conversation holds, whichever turn made it.
    authority::exclude_grants(&memory.request, Some(&a.source), &transcript.convo.grants)?;
    let start_line = bytes[..start].iter().filter(|b| **b == b'\n').count();
    let db = memory.memory()?;
    let access = memory.access();
    db.validate_dependency_roots(&f.dependencies, &access)?;
    // This turn continues a conversation: it begins with whatever the
    // earlier turns' recorded taint says they took in, and none of their
    // messages is its own.
    let mut walk = recording::Walk::new(
        &memory.request.account,
        &f.session_id,
        f.owner.text(),
        attempts,
        &db,
        &access,
        Intake::starting(prior.convo.taint.untrusted),
        f.dependencies.clone(),
    );
    walk.hold(&prior.convo.messages)?;
    walk.configured.extend(config.tools.iter().cloned());
    let mut history = prior.convo.messages;
    history.push(Message::user(f.owner.text()));
    let mut decisions = vec![];
    let mut versions = BTreeSet::new();
    let mut cut = false;
    let mut taint = None;
    let mut untrusted = false;
    for (line, r) in recording::records(bytes)?
        .into_iter()
        .enumerate()
        .skip(start_line)
    {
        match r {
            Record::Meta(_) | Record::ChatMemory { .. } => {
                anyhow::bail!("chat header repeated inside a turn")
            }
            Record::GoalContextDecision { binding, .. } => {
                let version = transcript
                    .convo
                    .goal_context
                    .version(binding.version())
                    .ok_or_else(|| anyhow::anyhow!("chat version unavailable"))?;
                ensure!(
                    version.owner_inputs() == expected_owners,
                    "chat decision owner changed"
                );
                // The history a request sent says what was in front of the
                // model. One that does not follow from the last by growing
                // (a compaction, an eviction, a line folded into the tail)
                // is a shape the walk does not derive, so the recorded
                // taint classifies the turn.
                let follows = binding.conversation_messages().is_some_and(|sent| {
                    let grew = if decisions.is_empty() {
                        sent == history
                    } else {
                        // Everything before the tail unchanged, and the
                        // tail only added to: a notice or a steer folded in.
                        let kept = history.len().saturating_sub(1);
                        sent.len() >= history.len()
                            && sent[..kept] == history[..kept]
                            && history.last().is_none_or(|tail| {
                                sent[kept].role == tail.role
                                    && sent[kept].content.starts_with(&tail.content)
                            })
                    };
                    history = sent.to_vec();
                    grew
                });
                if !follows || binding.run_start() != decisions.is_empty() {
                    walk.intake.unplaced();
                }
                // The sidecar is bounded; the transcript keeps every
                // decision, and the capture names the first of them.
                cut |= decisions.len() == MAX_DECISIONS
                    || (versions.len() == MAX_VERSIONS && !versions.contains(&binding.version()));
                if !cut {
                    versions.insert(binding.version());
                    decisions.push(ChatDecisionLocator::from_binding(
                        (line + 1) as u64,
                        &binding,
                    )?);
                }
            }
            Record::Config(c) => {
                ensure!(
                    c.workspace.canonicalize()? == a.workspace,
                    "chat workspace changed"
                );
                a.validate_tools(&c.tools)?;
                walk.configured.extend(c.tools);
            }
            // An owner-made exception to the jail is a fact about whose
            // turn this is when it reaches the store or this transcript.
            Record::Grants { grants } => {
                authority::exclude_grants(&memory.request, Some(&a.source), &grants)?;
            }
            Record::Notes { notes } => walk.notes(&notes),
            Record::PendingNote { note } => {
                walk.notes(std::slice::from_ref(&note));
                walk.intake.unplaced();
            }
            // A deferred tool's result, delivered after its turn. The
            // record says where its bytes came from, so it taints what
            // follows as any result does; the bytes are not a passage yet.
            Record::LateResult { external, .. } => {
                walk.intake.result(Some(external));
            }
            Record::Summary { .. } | Record::Title { .. } | Record::GoalAnchor { .. } => {}
            Record::Extend { blocks, .. } => walk.extension(blocks),
            Record::Rewrite { messages } => walk.rewrite(line, messages)?,
            Record::Message(m) => walk.message(line, m)?,
            // The conversation's taint only grows, and a turn may record
            // it more than once (a late result writes its own).
            Record::Taint(recorded) => {
                untrusted |= recorded.untrusted;
                taint = Some(recorded);
            }
            // However the turn stopped, and whether it recorded that at
            // all: a turn the provider failed has no outcome and is still
            // what happened.
            Record::Outcome(_) => {}
            _ => walk.intake.unplaced(),
        }
    }
    let taint = taint.ok_or_else(|| anyhow::anyhow!("chat taint unavailable"))?;
    ensure!(
        attempts.is_empty() || taint.private,
        "ordinary private recall taint missing"
    );
    ensure!(
        !attempts.iter().any(|a| a.external) || untrusted,
        "ordinary external recall taint missing"
    );
    ensure!(
        !decisions.is_empty() && walk.reconciled(),
        "ordinary recording incomplete"
    );
    let (owner_line, owner_pointer) = walk
        .owner
        .clone()
        .ok_or_else(|| anyhow::anyhow!("chat owner missing"))?;
    let context = loop {
        let used: BTreeSet<_> = decisions.iter().map(|d| d.context_revision).collect();
        let first = transcript
            .convo
            .goal_context
            .version(decisions[0].context_revision)
            .expect("validated version");
        let context = ChatObservationContext {
            conversation_id: f.session_id.clone(),
            turn_ordinal: f.ordinal,
            previous_turn: f.previous.clone(),
            prefix_start: f.prefix_start,
            prefix_length: bytes.len() as u64,
            prefix_digest: digest(bytes),
            owner_messages: vec![ChatOwnerMessageLocator {
                line: owner_line,
                pointer: owner_pointer.clone(),
                context_revision: first.revision(),
                owner_input_index: first.owner_inputs().len() - 1,
            }],
            versions: used
                .iter()
                .map(|revision| {
                    let version = transcript
                        .convo
                        .goal_context
                        .version(*revision)
                        .expect("validated version");
                    if a.web_ingress.is_some() {
                        ChatContextVersion::from_authenticated_web_version(version)
                    } else {
                        ChatContextVersion::from_version(version)
                    }
                })
                .collect::<Result<_>>()?,
            decisions: decisions.clone(),
        };
        // A long turn's later decisions are dropped from the sidecar
        // before the turn is refused for its size.
        if decisions.len() == 1 || serde_json::to_vec(&context)?.len() <= MAX_CONTEXT_BYTES {
            break context;
        }
        decisions.truncate(decisions.len() / 2);
    };
    let (observation, recorder) = a.observation_protocol();
    let mut payload = HostCapture {
        identity: CaptureIdentity {
            domain: memory.request.domain.clone(),
            account: memory.request.account.clone(),
            session_id: f.session_id.clone(),
            prefix_digest: digest(bytes),
            check_id: id.into(),
            check_version: observation.into(),
            evaluator_version: recorder.into(),
        },
        binding: ExternalBinding {
            authority: memory.authority.identity.clone(),
            locator: id.into(),
            revision: "0".repeat(64),
        },
        criterion: None,
        chat_context: Some(context),
        originals: vec![],
        receipts: vec![],
        artifact: ArtifactCheck {
            artifact_digest: None,
            verdict: CheckVerdict::Unavailable,
        },
        consulted: walk.consulted.clone(),
        scopes: memory.request.scopes.clone(),
        occurred_at: None,
    };
    let walked = walk.finish(Some(untrusted), recording::rest_of(&payload)?)?;
    payload.originals = walked.originals;
    payload.receipts = walked.receipts;
    payload.consulted = walked.consulted;
    OrdinaryObservationCapture::try_from_host(payload.clone())?;
    Ok(payload)
}
