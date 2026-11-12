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
        names.iter().any(|n| n == "goal_context")
            && names.iter().collect::<BTreeSet<_>>().len() == names.len(),
        "ordinary memory requires goal_context among uniquely named tools"
    );
    Ok(())
}
/// The chat recorder's surface, still the closed set: it binds every turn
/// to the exact history each request sent, and has not been opened to
/// other tools yet.
pub(super) fn chat_surface(names: &[String]) -> Result<()> {
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
) -> Result<(Vec<OriginalPassage>, Vec<ToolReceipt>)> {
    // Room left for the identity, the criterion and the consulted list.
    const ROOM: usize = MAX_CAPTURE_BYTES - 96 * 1024;
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
    let receipt_cost = |call: &str| {
        receipts
            .iter()
            .find(|r| r.call_id == call)
            .map_or(0, |r| r.call_id.len() + r.tool_name.len() + 64)
    };
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
        let invocation = passage.kind == PassageKind::ActionInvocation;
        let cost = costs[index]
            + if invocation {
                receipt_cost(call.as_deref().unwrap_or_default())
            } else {
                0
            };
        let answered = passage.kind != PassageKind::ToolResult
            || call.as_ref().is_some_and(|id| invoked.contains(id));
        full |= !answered
            || count + 1 > MAX_ORIGINALS
            || bytes + cost > ROOM
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
pub(super) fn capture(
    f: &Frozen,
    bytes: &[u8],
    attempts: &[Attempt],
    memory: &Memory,
    access: &Access,
    id: &str,
) -> Result<HostCapture> {
    // Each passage with the call it belongs to, if any, so a run too large
    // for one capture can be cut on whole calls (`fit`).
    let mut units: Vec<(OriginalPassage, Option<String>)> = vec![];
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
    let mut recorded_untrusted = None;
    // A one-shot run is a whole conversation, so it starts having taken
    // in nothing.
    let mut intake = Intake::starting(false);
    let mut ended = false;
    let mut configured_tools = BTreeSet::new();
    // The rule of this walk: a check on *whose run this is* refuses the
    // capture (the header, the workspace, the goal, a recall that does not
    // reconcile with what the tool returned). A record or block whose
    // *shape* the walk does not place is left out of the capture and
    // reported to `intake`, which settles against the recorded taint
    // whether what the model wrote can still be stored as local.
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
            Record::Notes { notes } => {
                let mut carried = crate::agent::Taint::default();
                carried.arm_for_notes(&notes);
                if carried.untrusted {
                    intake.ambient();
                }
            }
            Record::PendingNote { note } => {
                let mut carried = crate::agent::Taint::default();
                carried.arm_for_notes(std::slice::from_ref(&note));
                if carried.untrusted {
                    intake.ambient();
                }
                intake.unplaced();
            }
            // A deferred tool's result, delivered after its turn. The record
            // says where its bytes came from, so it taints what follows as
            // any result does; the bytes themselves are not a passage yet.
            Record::LateResult { external, .. } => {
                intake.result(Some(external));
            }
            // Nothing here is content the model read that a message or a
            // result does not also carry: what a run cost, a title, a
            // rewrite of history the walk has already read line by line,
            // the audit of a recall decision.
            // An owner-made exception to the jail is a fact about whose run
            // this is when it reaches the store or this transcript: a tool
            // could then have read the key or rewritten what is sealed.
            Record::Grants { grants } => {
                authority::exclude_grants(&f.request, Some(&f.source), &grants)?;
            }
            Record::Summary { .. }
            | Record::Title { .. }
            | Record::Rewrite { .. }
            | Record::ChatMemory { .. }
            | Record::GoalContextDecision { .. } => {}
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
                recorded_untrusted = Some(recorded.untrusted);
            }
            // However the run stopped. One that was cancelled, ran out of
            // turns or compacted on the way is still what happened.
            Record::Outcome(_) => {
                ensure!(taint, "ordinary run incomplete");
                ended = true;
            }
            Record::Extend { blocks, .. } => {
                // Harness text folded into a message already on file: a
                // nudge, a calendar line, a steer. Never a passage, and
                // outside content only if the harness's own rule or the
                // recorded taint says so.
                ensure!(!taint, "ordinary inline extension unavailable");
                let folded = crate::message::Message {
                    role: Role::User,
                    content: blocks,
                    ..crate::message::Message::user("")
                };
                let mut content = crate::agent::Taint::default();
                content.arm_for_content(std::slice::from_ref(&folded));
                if content.untrusted {
                    intake.ambient();
                }
                for block in &folded.content {
                    if !matches!(block, Block::Text { text }
                        if supported_note(text, &configured_tools, &calls))
                    {
                        intake.unplaced();
                    }
                }
            }
            Record::Message(mut m) => {
                ensure!(
                    !taint && config == 1 && anchors > 0,
                    "ordinary message unsupported"
                );
                messages += 1;
                // The harness's own content rule, asked of this message: a
                // user-role block it reads as untrusted is outside content no
                // tool result accounts for.
                let mut content = crate::agent::Taint::default();
                content.arm_for_content(std::slice::from_ref(&m));
                if content.untrusted {
                    intake.ambient();
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
                            // The original frozen owner block wins even if
                            // its bytes happen to equal a harness notice.
                            if owner == 0
                                && messages == 1
                                && index == 0
                                && !m.harness
                                && text == f.prompt
                            {
                                owner += 1;
                                external = intake.owner();
                                call = None;
                                (PassageKind::UserRequest, "text", text)
                            } else {
                                // Any other user-role text is the harness's
                                // or another agent's, and never owner
                                // evidence. A notice known byte for byte is
                                // local; anything else is for the recorded
                                // taint to classify.
                                if !supported_note(&text, &configured_tools, &calls) {
                                    intake.unplaced();
                                }
                                continue;
                            }
                        }
                        Block::Text { text } => {
                            external = intake.authored();
                            call = None;
                            (PassageKind::AssistantText, "text", text)
                        }
                        Block::ToolUse {
                            id: call_id,
                            name,
                            input,
                        } => {
                            if m.role != Role::Assistant
                                || calls.contains_key(&call_id)
                                || !nameable(&call_id)
                                || !nameable(&name)
                            {
                                intake.unplaced();
                                continue;
                            }
                            external = intake.authored();
                            let action = action_invocation(&call_id, &name, &input)?;
                            call = Some(call_id.clone());
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
                            // Counted before anything can skip it: a result
                            // the walk cannot pair still entered the
                            // conversation.
                            external = intake.result(provenance.get(&tool_use_id).copied());
                            let Some((name, input, complete)) = calls
                                .get_mut(&tool_use_id)
                                .filter(|call| !call.2 && m.role == Role::User)
                            else {
                                continue;
                            };
                            *complete = true;
                            receipts.push(ToolReceipt {
                                call_id: tool_use_id.clone(),
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
                            call = Some(tool_use_id);
                            (PassageKind::ToolResult, "content", content)
                        }
                        // An image, or a block in a role the walk has no
                        // passage for.
                        _ => {
                            intake.unplaced();
                            continue;
                        }
                    };
                    // Nothing to remember in an empty or blank field, and
                    // admission would refuse the whole capture for it.
                    if text.trim().is_empty() {
                        continue;
                    }
                    let (text, source_bytes) = excerpt(text);
                    let pointer = format!("/content/{index}/{field}");
                    units.push((
                        OriginalPassage {
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
                            source_bytes,
                        },
                        call,
                    ));
                }
            }
            // A record this walk has no reading for: a program's calls, a
            // late failure, a spoken direction, a cleared context.
            _ => intake.unplaced(),
        }
    }
    ensure!(
        meta == 1 && config == 1 && owner == 1 && ended && matched.len() == attempts.len(),
        "ordinary recording incomplete"
    );
    // Settled once the whole transcript is read, not at the taint record: a
    // record the walk cannot place may follow it, and must still count.
    let mut originals: Vec<_> = units.iter().map(|(p, _)| p.clone()).collect();
    intake.settle(recorded_untrusted, &mut originals);
    for ((passage, _), settled) in units.iter_mut().zip(originals) {
        *passage = settled;
    }
    // A call the run stopped before answering is still a call the model
    // made; its receipt says no result is available.
    for (call_id, (name, _, complete)) in &calls {
        if !complete {
            receipts.push(ToolReceipt {
                call_id: call_id.clone(),
                tool_name: name.clone(),
                status: ReceiptStatus::Unavailable,
            });
        }
    }
    let (originals, receipts) = fit(units, receipts)?;
    let criterion = criterion_text(&f.prompt);
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
            goal_text: criterion.clone(),
            criterion_text: criterion,
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
