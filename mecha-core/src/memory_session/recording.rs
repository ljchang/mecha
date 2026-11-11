use super::authority::{digest, Attempt, Frozen};
use super::*;
use crate::{
    memory::{SourceKey, ORDINARY_OBSERVATION_PROTOCOL, ORDINARY_RECORDER_VERSION},
    message::{Block, Role},
    session::{Record, SessionKind},
};
use std::collections::BTreeMap;
pub(super) fn surface(names: &[String]) -> Result<()> {
    ensure!(
        names.iter().any(|n| n == "goal_context")
            && names.iter().all(|n| matches!(
                n.as_str(),
                "fs_read" | "fs_write" | "fs_list" | "goal_context"
            ))
            && names.iter().collect::<BTreeSet<_>>().len() == names.len(),
        "ordinary memory requires an explicit supported tool surface"
    );
    Ok(())
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
                        && m.kind == Some(SessionKind::Run)
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
                    config == 1 && anchor > 0 && m == &crate::message::Message::user(prompt),
                    "ordinary begin is not fresh"
                );
                owner += 1;
            }
            _ => anyhow::bail!("ordinary begin is not fresh"),
        }
    }
    ensure!(
        meta == 1 && config <= 1 && anchor >= 1 && owner <= 1,
        "ordinary begin is not fresh"
    );
    Ok(())
}
pub(super) fn capture(
    f: &Frozen,
    bytes: &[u8],
    attempts: &[Attempt],
    memory: &Memory,
    access: &Access,
    id: &str,
) -> Result<HostCapture> {
    let mut originals = vec![];
    let mut receipts = vec![];
    let mut calls = BTreeMap::<String, (String, serde_json::Value, bool)>::new();
    let mut matched = BTreeSet::new();
    let mut consulted = vec![];
    let mut meta = 0;
    let mut config = 0;
    let mut anchors = 0;
    let mut owner = 0;
    let mut messages = 0;
    let mut taint = false;
    // A one-shot run is a whole conversation, so it starts having taken
    // in nothing.
    let mut intake = Intake::starting(false);
    let mut ended = false;
    let mut final_assistant = false;
    let mut configured_tools = BTreeSet::new();
    let mut last_accepts_notes = false;
    for (line, r) in records(bytes)?.into_iter().enumerate() {
        ensure!(!ended, "ordinary transcript appended after outcome");
        match r {
            Record::Meta(m) => {
                ensure!(
                    line == 0
                        && m.kind == Some(SessionKind::Run)
                        && m.id == f.session_id
                        && m.workspace.canonicalize()? == f.workspace,
                    "ordinary run header unavailable"
                );
                meta += 1;
            }
            Record::Config(c) => {
                ensure!(
                    messages == 0 && c.workspace.canonicalize()? == f.workspace,
                    "ordinary configuration unavailable"
                );
                surface(&c.tools)?;
                configured_tools.extend(c.tools);
                config += 1;
            }
            Record::GoalAnchor { goal } => {
                ensure!(
                    goal.is_some_and(|g| g.to_string() == f.goal),
                    "ordinary goal changed"
                );
                anchors += 1;
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
                ensure!(
                    attempts.is_empty() || recorded.private,
                    "ordinary private recall taint missing"
                );
                ensure!(
                    !attempts.iter().any(|a| a.external) || recorded.untrusted,
                    "ordinary external recall taint missing"
                );
                ensure!(!taint && messages > 0, "ordinary taint unavailable");
                taint = true;
                // No message follows this record, so every passage is in.
                intake.settle(Some(recorded.untrusted), &mut originals);
            }
            Record::Outcome(s) => {
                ensure!(
                    taint
                        && s.stop_cause == Some(crate::agent::StopCause::Completed)
                        && s.compactions == 0,
                    "ordinary run incomplete"
                );
                ended = true;
            }
            Record::Extend { index, blocks } => {
                // Native record_run uses Extend for a nudge folded into the
                // already-recorded owner/result tail. It is audit data, not a
                // new owner input or permission to rewrite earlier messages.
                ensure!(
                    !taint
                        && last_accepts_notes
                        && index.checked_add(1) == Some(messages)
                        && !blocks.is_empty(),
                    "ordinary inline extension unavailable"
                );
                for block in blocks {
                    let Block::Text { text } = block else {
                        anyhow::bail!("ordinary inline extension is not text")
                    };
                    ensure!(
                        supported_note(&text, &configured_tools, &calls),
                        "ordinary inline extension unsupported"
                    );
                }
            }
            Record::Message(mut m) => {
                ensure!(
                    !taint
                        && config == 1
                        && anchors > 0
                        && messages < 128
                        && !m.harness
                        && m.planning.is_none(),
                    "ordinary message unsupported"
                );
                messages += 1;
                let note_container = m.role == Role::User
                    && (messages == 1
                        || m.content
                            .iter()
                            .any(|b| matches!(b, Block::ToolResult { .. })));
                last_accepts_notes = note_container;
                final_assistant = m.role == Role::Assistant
                    && m.content.iter().any(|b| matches!(b, Block::Text { .. }));
                let mut saw_note = false;
                // What the loop recorded beside each result: whether its bytes
                // came from outside. Read before the blocks are consumed.
                let provenance = std::mem::take(&mut m.tool_provenance);
                for (index, b) in m.content.into_iter().enumerate() {
                    let external: Option<bool>;
                    let (kind, field, text) = match b {
                        // The full native transcript stays bound by the seal.
                        // Reasoning and its opaque signature are audit data.
                        Block::Thinking { .. } if m.role == Role::Assistant => continue,
                        Block::Text { text } => {
                            if m.role == Role::User {
                                // The original frozen owner block wins even if
                                // its bytes happen to equal a harness notice.
                                if owner == 0 && messages == 1 && index == 0 && text == f.prompt {
                                    owner += 1;
                                    external = intake.owner();
                                    (PassageKind::UserRequest, "text", text)
                                } else {
                                    ensure!(
                                        owner == 1
                                            && note_container
                                            && supported_note(&text, &configured_tools, &calls),
                                        "ordinary owner prompt or inline note changed"
                                    );
                                    saw_note = true;
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
                                    && !calls.contains_key(&call_id)
                                    && calls.len() < 32
                                    && matches!(
                                        name.as_str(),
                                        "fs_read" | "fs_write" | "fs_list" | "goal_context"
                                    ),
                                "ordinary invocation unsupported"
                            );
                            external = intake.authored();
                            let action = action_invocation(&call_id, &name, &input)?;
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
                            ensure!(
                                m.role == Role::User && !saw_note,
                                "ordinary result unsupported"
                            );
                            let (name, input, complete) = calls
                                .get_mut(&tool_use_id)
                                .ok_or_else(|| anyhow::anyhow!("ordinary orphan result"))?;
                            ensure!(!*complete, "ordinary duplicate result");
                            *complete = true;
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
                                            && a.is_error == is_error
                                            && recall::exact_output(&a.output, &content, a.external)
                                    })
                                    .collect();
                                ensure!(
                                    candidates.len() == 1 && matched.insert(candidates[0].0),
                                    "ordinary recall delivery ambiguous"
                                );
                                let a = candidates[0].1;
                                let refs = if !a.is_error && input.get("memory").is_some() {
                                    crate::memory_experience_bridge::memory_packet_references_checked(&content,Some((memory,access)))?
                                } else {
                                    ensure!(!a.external, "ordinary failed recall delivery unknown");
                                    vec![]
                                };
                                ensure!(
                                    refs == a.references,
                                    "ordinary recall dependencies changed"
                                );
                                memory.validate_delivered_context(&refs, &[], access)?;
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
                            account: f.request.account.clone(),
                            id: format!("{}:{}:{pointer}", f.session_id, line + 1),
                        },
                        kind,
                        line: (line + 1) as u64,
                        pointer,
                        text,
                        occurred_at: None,
                        external,
                    });
                }
            }
            _ => anyhow::bail!("ordinary recording unsupported"),
        }
    }
    ensure!(
        meta == 1
            && config == 1
            && owner == 1
            && ended
            && final_assistant
            && calls.values().all(|c| c.2)
            && matched.len() == attempts.len(),
        "ordinary recording incomplete"
    );
    let payload = HostCapture {
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
            goal_text: f.prompt.clone(),
            criterion_text: f.prompt.clone(),
            criterion_revision: digest(f.prompt.as_bytes()),
            confirmation_digest: id.into(),
        }),
        chat_context: None,
        originals,
        receipts,
        artifact: ArtifactCheck {
            artifact_digest: None,
            verdict: CheckVerdict::Unavailable,
        },
        consulted,
        scopes: f.request.scopes.clone(),
        occurred_at: None,
    };
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
