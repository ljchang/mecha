//! Exact native/SQLite agreement, separate from generic episodic observation.
use super::super::native_writer::{
    BoundNativeSession, NativeIdentity, PreparedNativeAppend, SyncedNativePrefix,
};
use super::*;
use crate::{
    memory::plan::{PlanView, VerifiedPlanAudit},
    message::{Block, Role},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expected {
    protocol: String,
    attachment: String,
    turn: String,
    ordinal: u64,
    device: u64,
    inode: u64,
    start: u64,
    start_digest: String,
    end: u64,
    digest: String,
}
pub(super) fn prepare(
    memory: &SessionMemory,
    turn: &ChatMemoryTurn,
    a: &Attachment,
    prepared: &PreparedNativeAppend,
) -> Result<()> {
    let (f, _) = frozen(&memory.authority, &turn.id)?;
    ensure!(
        a.task.is_some()
            && turn.attachment == f.attachment
            && turn.session_id == a.session_id
            && prepared.identity()
                == NativeIdentity {
                    device: a.device,
                    inode: a.inode
                }
            && prepared.start() >= f.start_length,
        "task final preparation mismatch"
    );
    let expected = Expected {
        protocol: CHAT_TASK_MEMORY_PROTOCOL.into(),
        attachment: f.attachment,
        turn: turn.id.clone(),
        ordinal: f.ordinal,
        device: a.device,
        inode: a.inode,
        start: prepared.start(),
        start_digest: prepared.start_digest().into(),
        end: prepared.endpoint(),
        digest: prepared.digest(),
    };
    authority::write(
        &memory.authority.file(&turn.id, "chat-native-expected")?,
        &serde_json::to_vec(&expected)?,
    )
}
fn expected(memory: &SessionMemory, id: &str, f: &Turn, a: &Attachment) -> Result<Expected> {
    let e: Expected = read_private(&memory.authority.file(id, "chat-native-expected")?, 8192)?;
    ensure!(
        e.protocol == CHAT_TASK_MEMORY_PROTOCOL
            && a.task.is_some()
            && e.attachment == f.attachment
            && e.turn == id
            && e.ordinal == f.ordinal
            && e.device == a.device
            && e.inode == a.inode
            && e.start >= f.start_length
            && e.start < e.end
            && e.end <= authority::SOURCE_MAX as u64,
        "task final fingerprint binding mismatch"
    );
    ensure!(
        !memory.authority.file(id, "withdrawn")?.try_exists()?,
        "task turn withdrawn"
    );
    Ok(e)
}
fn verify_bytes(e: &Expected, f: &Turn, bytes: &[u8]) -> Result<()> {
    ensure!(
        bytes.len() as u64 == e.end
            && authority::digest(bytes) == e.digest
            && authority::digest(&bytes[..e.start as usize]) == e.start_digest
            && authority::digest(&bytes[..f.start_length as usize]) == f.start_digest,
        "task final fingerprint mismatch"
    );
    Ok(())
}
pub(super) fn reconcile(
    memory: &SessionMemory,
    id: &str,
    source: &SyncedNativePrefix,
) -> Result<()> {
    let (f, a) = frozen_metadata(&memory.authority, id)?;
    let e = expected(memory, id, &f, &a)?;
    ensure!(
        source.identity()
            == NativeIdentity {
                device: e.device,
                inode: e.inode
            }
            && source.endpoint() == e.end
            && source.digest() == e.digest,
        "task sync proof mismatch"
    );
    verify_bytes(&e, &f, source.bytes())?;
    let mut db = memory.memory()?;
    let task = &a.task.as_ref().unwrap().task;
    let session = db.task_session(&memory.request.account, &a.session_id, memory.access())?;
    let entries = db.plan_turn_entries(&session, task, &f.attachment, id)?;
    validate_native(&a, &f, id, source.bytes(), &entries)?;
    let proof = VerifiedPlanAudit::from_host_verified(&session, task, &f.attachment, id, entries)?;
    db.plan_reconcile_audit(&session, task, &proof)
}
pub(super) fn validate_native(
    a: &Attachment,
    f: &Turn,
    id: &str,
    bytes: &[u8],
    entries: &[PlanView],
) -> Result<()> {
    validate_source_ingress(a, bytes, &f.attachment)?;
    let transcript = Session::parse(&a.source, std::str::from_utf8(bytes)?)?;
    let prior = Session::parse(
        &a.source,
        std::str::from_utf8(&bytes[..f.start_length as usize])?,
    )?;
    let start = prior.convo.messages.len();
    let messages = &transcript.convo.messages;
    let mut ordinal = 0u32;
    let mut matched = BTreeSet::new();
    let decisions = recording::records(&bytes[f.start_length as usize..])?
        .into_iter()
        .filter_map(|r| {
            if let Record::GoalContextDecision { binding, .. } = r {
                Some(binding)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    for (index, message) in messages.iter().enumerate().skip(start) {
        crate::planning::validate_plan_results(message)?;
        if message.role != Role::User {
            ensure!(
                message.planning.is_none() && message.plan_results.is_empty(),
                "task planning on non-result message"
            );
        }
        if message.role == Role::User
            && (message.planning.is_some() || !message.plan_results.is_empty())
        {
            ensure!(
                index > start
                    && messages[index - 1].role == Role::Assistant
                    && messages[index - 1]
                        .content
                        .iter()
                        .any(|b| matches!(b, Block::ToolUse { .. })),
                "task unassociated user planning metadata"
            );
        }
        if message.role != Role::Assistant {
            continue;
        }
        let calls = message
            .content
            .iter()
            .filter_map(|b| {
                if let Block::ToolUse { id, name, input } = b {
                    Some((id, name, input))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if calls.is_empty() {
            continue;
        }
        let result = messages
            .get(index + 1)
            .ok_or_else(|| anyhow::anyhow!("task result missing"))?;
        ensure!(result.role == Role::User, "task result not adjacent");
        crate::planning::validate_plan_results(result)?;
        let bindings = decisions
            .iter()
            .filter(|b| {
                b.conversation_messages()
                    .is_some_and(|history| history == &messages[..index])
            })
            .collect::<Vec<_>>();
        ensure!(bindings.len() == 1, "task decision association ambiguous");
        let binding = bindings[0];
        let version = transcript
            .convo
            .goal_context
            .version(binding.version())
            .ok_or_else(|| anyhow::anyhow!("task goal version missing"))?;
        let decision_digest = authority::digest(&serde_json::to_vec(binding)?);
        let version_digest = authority::digest(&serde_json::to_vec(version)?);
        let mut ids = BTreeSet::new();
        let mut expected_audits = 0;
        for (call_id, name, input) in calls {
            ensure!(
                ids.insert(call_id) && ordinal < 256,
                "task duplicate call or occurrence bound"
            );
            let results = result
                .content
                .iter()
                .enumerate()
                .filter_map(|(i, b)| {
                    if let Block::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } = b
                    {
                        (tool_use_id == call_id).then_some((i, content, *is_error))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            ensure!(results.len() == 1, "task missing/duplicate result");
            let (result_index, content, is_error) = results[0];
            let audits = result
                .plan_results
                .iter()
                .filter(|r| r.result_index == result_index)
                .collect::<Vec<_>>();
            if name == "todo" && !is_error {
                expected_audits += 1;
                ensure!(
                    audits.len() == 1,
                    "task successful todo audit missing or duplicated"
                );
                let audit = audits[0];
                ensure!(
                    audit.turn_id == id && audit.ordinal == ordinal && matched.insert(ordinal),
                    "task occurrence changed"
                );
                let entry = entries
                    .iter()
                    .find(|e| e.value.request.occurrence.ordinal == ordinal)
                    .ok_or_else(|| anyhow::anyhow!("task receipt absent"))?;
                let request = &entry.value.request;
                let feedback = result
                    .planning
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("task contribution absent"))?;
                let contribution = crate::planning::Feedback {
                    steps: feedback.steps[audit.steps_start..audit.steps_end].to_vec(),
                    decisions: feedback.decisions[audit.decisions_start..audit.decisions_end]
                        .to_vec(),
                };
                crate::planning::durable_feedback(request.feedback.text(), call_id)?;
                crate::tool::todo::memory::validate_durable_request(request)?;
                ensure!(
                    entry.value.task == a.task.as_ref().unwrap().task
                        && entry.reference.revision == audit.revision_id
                        && entry.receipt_id == audit.receipt_id
                        && request.occurrence.turn_id == id
                        && request.provider_call_id == *call_id
                        && request.input_json == serde_json::to_string(input)?
                        && request.result_text == *content
                        && request.feedback.text() == serde_json::to_string(&contribution)?
                        && request.goal_version == binding.version()
                        && request.goal_decision_digest == decision_digest
                        && request.goal_version_digest == version_digest
                        && result.tool_provenance.get(call_id) == Some(&false),
                    "task native receipt evidence changed"
                );
            } else {
                ensure!(audits.is_empty(), "task audit names another result");
            }
            ordinal += 1;
        }
        ensure!(
            expected_audits == result.plan_results.len(),
            "task extra native audit"
        );
        ensure!(
            result
                .content
                .iter()
                .filter(|b| matches!(b, Block::ToolResult { .. }))
                .count()
                == ids.len(),
            "task extra result"
        );
        ensure!(
            result.planning.is_none() || !result.plan_results.is_empty(),
            "task unassociated feedback"
        );
    }
    ensure!(
        matched.len() == entries.len()
            && entries
                .iter()
                .all(|e| matched.contains(&e.value.request.occurrence.ordinal)),
        "task original receipt set incomplete"
    );
    Ok(())
}
/// Lease-bound caller only. Sync and audit existing bytes; no append/permit/tool.
pub(super) fn recover(memory: &SessionMemory, id: &str) -> Result<()> {
    let (f, a) = frozen_metadata(&memory.authority, id)?;
    let e = expected(memory, id, &f, &a)?;
    let seal_path = memory.authority.file(id, "chat-seal")?;
    let hash_path = memory.authority.file(id, "chat-seal-hash")?;
    if seal_path.try_exists()? && hash_path.try_exists()? {
        let (s, _, _) = seal(&memory.authority, id)?;
        ensure!(
            s.length == e.end && s.digest == e.digest,
            "task sealed fingerprint mismatch"
        );
        let bytes = source_prefix(&a, e.end)?;
        verify_bytes(&e, &f, &bytes)?;
        let db = memory.memory()?;
        let task = &a.task.as_ref().unwrap().task;
        let session = db.task_session(&memory.request.account, &a.session_id, memory.access())?;
        let entries = db.plan_turn_entries(&session, task, &f.attachment, id)?;
        ensure!(
            entries.iter().all(|e| e.audited),
            "task sealed before audit"
        );
        return validate_native(&a, &f, id, &bytes, &entries);
    }
    ensure!(!hash_path.try_exists()?, "task hash without seal");
    // Only latest incomplete ordinal may be repaired; never adopt a foreign suffix.
    ensure!(
        f.ordinal == MAX_TURNS
            || !ordinal_path(&memory.authority, &a.session_id, f.ordinal + 1).try_exists()?,
        "task recovery has later turn"
    );
    let bytes = authority::read(&a.source, authority::SOURCE_MAX)?;
    verify_bytes(&e, &f, &bytes)?;
    let transcript = Session::parse(&a.source, std::str::from_utf8(&bytes)?)?;
    let session = Session {
        meta: transcript.meta,
        path: a.source.clone(),
    };
    let source = BoundNativeSession::bind(
        &session,
        NativeIdentity {
            device: a.device,
            inode: a.inode,
        },
        &bytes,
    )?
    .sync_verified()?;
    reconcile(memory, id, &source)?;
    let attempts = memory.authority.attempts(id)?;
    let payload = if memory.workspace.is_none() {
        capture::capture_task_recovery(memory, &f, &a, source.bytes(), &attempts, id)?
    } else {
        capture::capture(memory, &f, &a, source.bytes(), &attempts, id)?
    };
    let s = ChatSeal {
        turn: id.into(),
        length: source.endpoint(),
        digest: source.digest().into(),
        attempts,
        payload,
    };
    let data = serde_json::to_vec(&s)?;
    authority::write(&seal_path, &data)?; // Existing partial seal must match byte for byte.
    authority::write(&hash_path, authority::digest(&data).as_bytes())
}
