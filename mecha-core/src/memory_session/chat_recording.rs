//! Strict capture of one new span, retaining native absolute source pointers.
use super::*;
use crate::memory::SourceKey;
use crate::message::{Block, Role};
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
    ensure!(
        bytes.len() > f.start_length as usize
            && digest(&bytes[..f.start_length as usize]) == f.start_digest,
        "chat begin prefix changed"
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
    expected_owners.push(f.owner.clone());
    let mut external_results = a.graph_surface.is_some() && prior.convo.taint.untrusted;
    let mut expected_history = prior.convo.messages;
    expected_history.push(crate::message::Message::user(f.owner.text()));

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
    let start_line = bytes[..f.start_length as usize]
        .iter()
        .filter(|b| **b == b'\n')
        .count();
    let db = memory.memory()?;
    db.validate_dependency_roots(&f.dependencies, &memory.access())?;
    let mut originals = vec![];
    let mut receipts = vec![];
    let mut calls = BTreeMap::<String, (String, serde_json::Value, bool)>::new();
    let mut matched = BTreeSet::new();
    let mut consulted = f.dependencies.clone();
    let mut decisions = vec![];
    let mut used_versions = BTreeSet::new();
    let mut owner_line = None;
    let mut owner = 0;
    let mut messages = 0;
    let mut taint = false;
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
                // Native decisions are written during execution; the CLI appends
                // assistant/results at run end. Reconstruct the exact request
                // boundaries from that final append-only native history.
                if !decisions.is_empty() {
                    let index = expected_history.len();
                    let response = transcript
                        .convo
                        .messages
                        .get(index)
                        .ok_or_else(|| anyhow::anyhow!("chat decision skips response"))?;
                    let result = transcript
                        .convo
                        .messages
                        .get(index + 1)
                        .ok_or_else(|| anyhow::anyhow!("chat decision skips tool delivery"))?;
                    let calls: BTreeSet<_> = response
                        .content
                        .iter()
                        .filter_map(|b| {
                            if let Block::ToolUse { id, .. } = b {
                                Some(id.as_str())
                            } else {
                                None
                            }
                        })
                        .collect();
                    let returned: BTreeSet<_> = result
                        .content
                        .iter()
                        .filter_map(|b| {
                            if let Block::ToolResult { tool_use_id, .. } = b {
                                Some(tool_use_id.as_str())
                            } else {
                                None
                            }
                        })
                        .collect();
                    ensure!(
                        response.role == Role::Assistant
                            && result.role == Role::User
                            && !calls.is_empty()
                            && calls == returned
                            && result
                                .content
                                .iter()
                                .all(|b| matches!(b, Block::ToolResult { .. })),
                        "chat decision boundary lacks complete native tool delivery"
                    );
                    expected_history.push(response.clone());
                    expected_history.push(result.clone());
                }
                ensure!(
                    sent == expected_history && transcript.convo.messages.starts_with(sent),
                    "chat decision history differs from exact native boundary"
                );
                used_versions.insert(binding.version());
                decisions.push(ChatDecisionLocator::from_binding(
                    (line + 1) as u64,
                    &binding,
                )?);
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
            Record::Message(m) => {
                ensure!(
                    !ended && messages < 128 && !m.harness && m.planning.is_none(),
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
                for (index, b) in m.content.into_iter().enumerate() {
                    let (kind, field, text) = match b {
                        // The full native transcript stays bound by the seal.
                        // Reasoning and its opaque signature are audit data.
                        Block::Thinking { .. } if m.role == Role::Assistant => continue,
                        Block::Text { text } => {
                            if m.role == Role::User {
                                ensure!(
                                    owner == 0
                                        && messages == 1
                                        && index == 0
                                        && text == f.owner.text(),
                                    "ordinary owner prompt changed"
                                );
                                owner += 1;
                                owner_line = Some((line + 1) as u64);
                                (PassageKind::UserRequest, "text", text)
                            } else {
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
                                    && !calls.contains_key(&call_id)
                                    && calls.len() < 32
                                    && a.graph_surface.as_ref().map_or_else(
                                        || matches!(
                                            name.as_str(),
                                            "fs_read" | "fs_write" | "fs_list" | "goal_context"
                                        ),
                                        |graph| graph.contains_tool(&name)
                                    ),
                                "ordinary invocation unsupported"
                            );
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
                            ensure!(m.role == Role::User, "ordinary result unsupported");
                            let (name, input, complete) = calls
                                .get_mut(&tool_use_id)
                                .ok_or_else(|| anyhow::anyhow!("ordinary orphan result"))?;
                            ensure!(!*complete, "ordinary duplicate result");
                            *complete = true;
                            let external = if a.graph_surface.is_some() {
                                m.tool_provenance.get(&tool_use_id).copied()
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
            ChatContextVersion::from_version(
                transcript
                    .convo
                    .goal_context
                    .version(*revision)
                    .expect("validated version"),
            )
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
    let payload = HostCapture {
        identity: CaptureIdentity {
            domain: memory.request.domain.clone(),
            account: memory.request.account.clone(),
            session_id: f.session_id.clone(),
            prefix_digest: digest(bytes),
            check_id: id.into(),
            check_version: CHAT_OBSERVATION_PROTOCOL.into(),
            evaluator_version: CHAT_RECORDER_VERSION.into(),
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
