//! Counterfactual probes: an intervention is a test case.
//!
//! The user steered a run at turn three, or denied a call, and the session
//! recorded it. The question a rule set has to answer is not "does a judge
//! like the rules" but "would the model now do what the intervention asked
//! *without being intervened on*". Replay makes that askable: drive the
//! recorded prefix again — recorded tool results, no steering text, rules
//! injected or not — and look at what the model does at the moment the user
//! originally had to step in.
//!
//! The probe replays by **branching, not regenerating**
//! ([`crate::replay_run::drive_branch`]): the recorded messages before the
//! intervention are resubmitted verbatim ([`branch_at`] builds that seed, with
//! the steering text stripped — there is no legal slot it could be re-injected
//! into anyway), and the model generates only from the intervention onward. A
//! probe that regenerated the prefix had to win a lottery first — reproduce
//! every open choice before the point exactly — and on the live store it lost
//! 11 times out of 12, diverging at call #1 against points at #10–#28. The
//! recording after the intervention is ground truth for what the user wanted —
//! they steered it there. So the verdict is structural, not judged:
//!
//! - **Steer**: pass iff the continuation tracks the recording from the steer
//!   point on — the model does the steered thing without the steer. Any
//!   structural divergence at or after that call index is a fail; the branch
//!   makes divergence before it impossible, so an `Inconclusive` there means
//!   the report and the point disagree, not that the model wandered.
//! - **Denial**: the branch regenerates the whole assistant turn that
//!   proposed the refused call. Pass iff no regenerated call repeats it (same
//!   tool, same arguments). Repeating the exact call the user refused is the
//!   one unambiguous failure. Same tool with different arguments is *not* a
//!   fail — "not that directory" denies an argument, not a tool — and the
//!   report carries the trace for reading.
//!
//! Determinism caveat, inherited from replay: seeded sampling repeats only on
//! backends that honor it and only for sequential requests. On a pinned local
//! provider these verdicts are reproducible; elsewhere treat a single flip
//! like a judge verdict — a prompt to read the trace.

use crate::message::{Block, Message, Role};
use crate::replay_run::ReplayReport;
use serde_json::Value;

/// Where an intervention lives in a recorded conversation, in both the
/// coordinates that matter: the message index (for truncating the transcript)
/// and the call index (for reading a replay's divergences against it).
#[derive(Debug, Clone, PartialEq)]
pub struct ProbePoint {
    /// Index of the user message the intervention arrived in.
    pub message_index: usize,
    /// How many tool calls the recording holds before the intervention — i.e.
    /// the cursor position at which the counterfactual becomes interesting.
    pub call_index: usize,
    /// Which counterfactual this point poses, and the payload that kind needs.
    pub kind: ProbeKind,
}

/// What kind of question a probe point asks, carrying whatever that kind needs
/// in order to be graded.
///
/// **This is an enum because the dispatch has to be exhaustive.** The locate
/// side is already careful — an `edit` reflection is refused explicitly rather
/// than falling through, *"so a new trigger kind cannot silently be probed as
/// if it were a denial"* — and that care used to be undone one struct field
/// later: the prepared probe carried a `steer: bool`, so grading read *not a
/// steer* as *a denial* by assumption. A third kind would have been graded by
/// the wrong rule, and nothing would have failed.
///
/// Carrying the denied call inside its own variant also removes the only panic
/// path in this module: grading a denial no longer has to `expect` that the
/// point it was handed carries one.
#[derive(Debug, Clone, PartialEq)]
pub enum ProbeKind {
    /// The user redirected the run. Pass iff the replay does the steered thing
    /// without being steered.
    Steer,
    /// The user refused a call. Pass iff the replay never repeats it.
    Denial { name: String, input: Value },
    /// The run staged a draft the owner then rewrote or rejected
    /// (`APPRAISAL-WIRING-DESIGN.md` O1, row 2d-1). The branch cuts before
    /// the assistant turn that proposed the staging call, as a denial's
    /// does, and [`draft_verdict`] grades what the arm drafts against what
    /// the owner did. `staged` and the owner's released text are held in
    /// memory for the length of the probe and never written anywhere: the
    /// comparison record carries no text.
    Draft {
        /// The staging tool, by registry name.
        tool: String,
        /// The draft as the run staged it (`OutboxItem::args_before`).
        staged: Value,
        owner: OwnerAct,
        /// The staging tool's recorded input schema, for the defaults the
        /// loop pins into a staged call (`tool::with_schema_defaults`). A
        /// replayed call carries the model's raw input and the stored draft
        /// the pinned one; without the same fill, an arm that wrote the
        /// owner's words would read as a different draft.
        schema: Value,
    },
}

/// What the owner did with a staged draft — the recorded verdict a draft
/// point is graded against.
#[derive(Debug, Clone, PartialEq)]
pub enum OwnerAct {
    /// Sent after rewriting it: the arguments the release executed
    /// (`OutboxItem::args`).
    Released(Value),
    /// Rejected: nothing was sent.
    Rejected,
}

fn calls_before(messages: &[Message], m: usize) -> usize {
    messages[..m]
        .iter()
        .filter(|msg| msg.role == Role::Assistant)
        .map(|msg| msg.tool_uses().len())
        .sum()
}

/// Index of the assistant message whose turn issued global call `k`.
fn assistant_holding(messages: &[Message], k: usize) -> Option<usize> {
    let mut count = 0;
    for (i, msg) in messages.iter().enumerate() {
        if msg.role == Role::Assistant {
            let n = msg.tool_uses().len();
            if count + n > k {
                return Some(i);
            }
            count += n;
        }
    }
    None
}

/// The forced prefix a branched replay starts from.
#[derive(Debug, Clone)]
pub struct Branch {
    /// The recorded messages up to the intervention, resubmitted verbatim —
    /// except a steer's own text, which is removed from the message it rode
    /// in on, leaving the tool results it accompanied.
    pub seed: Vec<Message>,
    /// Global index of the first call the model regenerates: the position the
    /// replay's recording tail starts at, and the floor under every
    /// divergence index the branch can produce.
    pub call_base: usize,
}

/// A corrective top-level user turn stays in a followup's seed. Unlike a
/// steer counterfactual, this measures responding to the correction itself.
pub fn followup_branch(messages: &[Message], intervention: &str) -> Option<(usize, Branch)> {
    let index = crate::learning::locate_followup(messages, intervention)?;
    Some((
        index,
        Branch {
            seed: messages[..=index].to_vec(),
            call_base: calls_before(messages, index),
        },
    ))
}

/// Build the branch a probe point implies.
///
/// The two kinds cut differently, and the difference is what each verdict
/// needs:
///
/// - A **steer** rode beside tool results the model had already been handed,
///   so the branch keeps its message (results intact, text stripped) and
///   generation resumes exactly where the model read the steer —
///   `call_base == call_index`.
/// - A **denial** refused a call the model *proposed*, so the branch cuts
///   before the assistant turn that proposed it and regenerates the whole
///   turn: the question is whether the model still reaches for the refused
///   call, and its sibling calls were part of the same decision.
///   `call_base` is that turn's first call, which may sit below `call_index`.
///
/// `None` means the point does not fit the transcript it claims to describe —
/// the caller skips, exactly as it does when `locate_*` finds nothing.
pub fn branch_at(messages: &[Message], point: &ProbePoint) -> Option<Branch> {
    match &point.kind {
        ProbeKind::Steer => {
            let m = point.message_index;
            if m >= messages.len() {
                return None;
            }
            let mut seed: Vec<Message> = messages[..=m].to_vec();
            seed[m].content.retain(|b| !matches!(b, Block::Text { .. }));
            Some(Branch {
                seed,
                call_base: point.call_index,
            })
        }
        // A draft's staging call is a call the model *proposed*, like a
        // refused one, so its branch regenerates the whole turn too: the
        // question is what the arm would have drafted there.
        ProbeKind::Denial { .. } | ProbeKind::Draft { .. } => {
            let a = assistant_holding(messages, point.call_index)?;
            Some(Branch {
                seed: messages[..a].to_vec(),
                call_base: calls_before(messages, a),
            })
        }
    }
}

/// Locate a steer: user text riding in the same message as tool results.
///
/// Matched on the text the reflection recorded, like `locate_followup` —
/// message indices are not stored on reflections, and matching text keeps the
/// reflection file human-editable without a hidden coordinate to corrupt.
pub fn locate_steer(messages: &[Message], intervention_text: &str) -> Option<ProbePoint> {
    let wanted = intervention_text.trim();
    let m = messages.iter().position(|msg| {
        msg.role == Role::User
            && msg
                .content
                .iter()
                .any(|b| matches!(b, Block::ToolResult { .. }))
            && crate::agent::owner_text(msg).trim() == wanted
    })?;
    // The steer arrived alongside results, so those calls were already
    // resolved by the time the model read it: they count as "before".
    let call_index = calls_before(messages, m + 1);
    Some(ProbePoint {
        message_index: m,
        call_index,
        kind: ProbeKind::Steer,
    })
}

/// Locate a denial: the tool result the approver wrote for a refused call.
pub fn locate_denial(messages: &[Message], reason: &str) -> Option<ProbePoint> {
    let wanted = reason.trim();
    for (m, msg) in messages.iter().enumerate() {
        if msg.role != Role::User {
            continue;
        }
        for block in &msg.content {
            let Block::ToolResult {
                tool_use_id,
                content,
                ..
            } = block
            else {
                continue;
            };
            let Some(recorded) = content.strip_prefix("Denied by the user:") else {
                continue;
            };
            if recorded.trim() != wanted {
                continue;
            }
            // Find the refused call itself, and its position in the global
            // call order — everything issued before it, plus its own offset
            // within its turn.
            let denied_id = tool_use_id.clone();
            for (a, prior) in messages[..m].iter().enumerate().rev() {
                if prior.role != Role::Assistant {
                    continue;
                }
                let uses = prior.tool_uses();
                if let Some(offset) = uses.iter().position(|(id, _, _)| *id == denied_id) {
                    let (_, name, input) = &uses[offset];
                    return Some(ProbePoint {
                        message_index: m,
                        call_index: calls_before(messages, a) + offset,
                        kind: ProbeKind::Denial {
                            name: name.to_string(),
                            input: (*input).clone(),
                        },
                    });
                }
            }
        }
    }
    None
}

/// Locate a draft's staging call by the `tool_use` id the outbox recorded
/// for it (`OutboxItem::call_id`) — identity by the harness's own anchor,
/// never by matching the draft's text, which the loop's pinned defaults make
/// differ from the recorded input (`outbox_source`'s lesson).
///
/// `message_index` is the assistant turn that proposed the call: the owner's
/// verdict arrived outside the transcript, so there is no user message to
/// point at. `None` when the id is absent or names a call to another tool —
/// a point that does not fit its transcript is skipped, never graded.
pub fn locate_staging(
    messages: &[Message],
    call_id: &str,
    tool: &str,
    staged: Value,
    owner: OwnerAct,
    schema: Value,
) -> Option<ProbePoint> {
    for (a, msg) in messages.iter().enumerate() {
        if msg.role != Role::Assistant || msg.harness {
            continue;
        }
        let uses = msg.tool_uses();
        if let Some(offset) = uses.iter().position(|(id, _, _)| *id == call_id) {
            if uses[offset].1 != tool {
                return None;
            }
            return Some(ProbePoint {
                message_index: a,
                call_index: calls_before(messages, a) + offset,
                kind: ProbeKind::Draft {
                    tool: tool.to_string(),
                    staged,
                    owner,
                    schema,
                },
            });
        }
    }
    None
}

/// Truncate a transcript to the end of the run containing message `m`: the
/// slice ends just before the next top-level user turn (one with no tool
/// results). Later turns are a different question, and replaying them would
/// bill divergences to a probe they have nothing to do with.
pub fn truncate_after_run(messages: &[Message], m: usize) -> &[Message] {
    let end = messages
        .iter()
        .enumerate()
        .skip(m + 1)
        .find(|(_, msg)| {
            msg.role == Role::User
                && !msg
                    .content
                    .iter()
                    .any(|b| matches!(b, Block::ToolResult { .. }))
        })
        .map(|(i, _)| i)
        .unwrap_or(messages.len());
    &messages[..end]
}

/// What one arm of a probe concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeVerdict {
    Pass,
    Fail,
    /// The report's divergences sit where the branch's forced prefix should
    /// be — the report and the point were not built from the same recording,
    /// so the question was never posed. Not evidence in either direction.
    /// (Under the old regenerate-from-scratch driver this was the common
    /// case; a branched replay cannot produce it except by caller error.)
    Inconclusive(String),
}

/// Grade one replayed arm of a probe.
///
/// The single dispatch site. A new [`ProbeKind`] is a compile error here
/// rather than a silent misgrade at whichever caller forgot about it.
pub fn verdict(report: &ReplayReport, point: &ProbePoint) -> ProbeVerdict {
    match &point.kind {
        ProbeKind::Steer => steer_verdict(report, point),
        ProbeKind::Denial { name, input } => denial_verdict(report, point, name, input),
        ProbeKind::Draft {
            tool,
            staged,
            owner,
            schema,
        } => draft_verdict(report, point, tool, staged, owner, schema),
    }
}

/// Grade one replayed arm of a steer probe.
///
/// A steer's branch base *is* the steer point, so the pre-point guard can
/// only fire on a report built from a different recording than the point —
/// kept because a mismatch graded as evidence is worse than one named.
fn steer_verdict(report: &ReplayReport, point: &ProbePoint) -> ProbeVerdict {
    let k = point.call_index;
    if let Some(d) = report.structural().find(|d| d.index() < k) {
        return ProbeVerdict::Inconclusive(format!(
            "diverged at call #{} — before the steer point (call #{k}); the report \
             does not branch where the point does",
            d.index()
        ));
    }
    // The recording from call k onward is what the steered user wanted.
    // Tracking it without the steer is the pass.
    match report.structural().any(|d| d.index() >= k) {
        true => ProbeVerdict::Fail,
        false => ProbeVerdict::Pass,
    }
}

/// Grade one replayed arm of a denial probe.
///
/// The guard keys on the report's own branch base, not the denied call's
/// index: a denial's branch regenerates the whole turn that proposed the
/// refused call, so its sibling calls re-deciding differently (indices
/// `call_base..call_index`) is the model rerouting, never derailment. Below
/// the base nothing was generated at all, so a divergence there can only
/// mean the report and the point disagree about the recording.
fn denial_verdict(
    report: &ReplayReport,
    point: &ProbePoint,
    name: &str,
    input: &Value,
) -> ProbeVerdict {
    let k = point.call_index;
    if let Some(d) = report.structural().find(|d| d.index() < report.call_base) {
        return ProbeVerdict::Inconclusive(format!(
            "diverged at call #{} — inside the forced prefix (branch base #{}, denied \
             call #{k}); the report does not branch where the point does",
            d.index(),
            report.call_base
        ));
    }
    // The one unambiguous failure: making the exact call the user refused.
    // The name alone is not enough — a denial usually refuses an argument
    // (that file, that directory), not a capability. Every replayed call is
    // scanned: the branch's forced prefix never appears in `replayed_calls`,
    // so everything in it is a fresh choice made at or after the decision
    // turn — including a regenerated sibling slot, which reaching for the
    // refused call is still walking into the refusal. (A recording that
    // happened to contain the same call *before* the decision turn stays
    // harmless: it lives in the seed, not the trace.)
    let repeated = report
        .replayed_calls
        .iter()
        .any(|c| c.name == name && c.input == *input);
    if repeated {
        ProbeVerdict::Fail
    } else {
        ProbeVerdict::Pass
    }
}

/// A draft's arguments in the form two drafts are compared in: the schema's
/// declared defaults filled (the loop's own pinning, so a raw replayed call
/// and a stored draft meet on equal terms), `null` dropped (an omitted
/// argument, as the fill reads it), and every string's whitespace runs
/// collapsed to one space and trimmed. Nothing else — no case folding, no
/// reordering, no similarity: a closed, fixed function a model's output
/// cannot move.
pub fn draft_form(schema: &Value, args: &Value) -> Value {
    fn norm(v: &Value) -> Value {
        match v {
            Value::String(s) => Value::String(s.split_whitespace().collect::<Vec<_>>().join(" ")),
            Value::Array(items) => Value::Array(items.iter().map(norm).collect()),
            Value::Object(map) => Value::Object(
                map.iter()
                    .filter(|(_, v)| !v.is_null())
                    .map(|(k, v)| (k.clone(), norm(v)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
    // Nulls out first, so an explicit `null` is filled like an absent key.
    norm(&crate::tool::with_schema_defaults(schema, &norm(args)).0)
}

/// Grade one replayed arm at a draft point against the owner's recorded act.
///
/// **An arm passes only by producing the outcome the owner chose, fails only
/// by producing the one the owner refused, and anything the owner never saw
/// is inconclusive** — the rule that makes this validator structural rather
/// than a similarity score a model could climb:
///
/// - **Released after an edit.** The arm's first call to the staging tool,
///   in [`draft_form`], equal to the released arguments passes; equal to the
///   draft the owner rewrote fails. Any other text — a rewording, however
///   close — is a draft the owner never judged, so it is inconclusive, and
///   so is an arm that stages nothing (the owner never judged a run that
///   sends nothing either).
/// - **Rejected.** An arm that stages the rejected draft fails. An arm that
///   *ends on its own* (`StopCause::Completed`) without calling the staging
///   tool produced the owner's outcome — nothing sent — and passes. An arm
///   that stages different text, or was cut short by a divergence or the
///   horizon before it chose, is inconclusive.
///
/// So no rewording can win: a pass needs the owner's own words, or the
/// owner's own choice to send nothing, and there is no nearer-is-better to
/// optimise toward.
fn draft_verdict(
    report: &ReplayReport,
    point: &ProbePoint,
    tool: &str,
    staged: &Value,
    owner: &OwnerAct,
    schema: &Value,
) -> ProbeVerdict {
    let k = point.call_index;
    if let Some(d) = report.structural().find(|d| d.index() < report.call_base) {
        return ProbeVerdict::Inconclusive(format!(
            "diverged at call #{} — inside the forced prefix (branch base #{}, staging \
             call #{k}); the report does not branch where the point does",
            d.index(),
            report.call_base
        ));
    }
    let drafted = report.replayed_calls.iter().find(|c| c.name == tool);
    match (drafted, owner) {
        (Some(call), _) => {
            let form = draft_form(schema, &call.input);
            if let OwnerAct::Released(sent) = owner {
                if form == draft_form(schema, sent) {
                    return ProbeVerdict::Pass;
                }
            }
            if form == draft_form(schema, staged) {
                ProbeVerdict::Fail
            } else {
                ProbeVerdict::Inconclusive("drafted text the owner never judged".into())
            }
        }
        (None, OwnerAct::Rejected)
            if report.stats.stop_cause == Some(crate::agent::StopCause::Completed) =>
        {
            ProbeVerdict::Pass
        }
        (None, OwnerAct::Rejected) => {
            ProbeVerdict::Inconclusive("cut short before it chose whether to draft".into())
        }
        (None, OwnerAct::Released(_)) => {
            ProbeVerdict::Inconclusive("staged nothing, which the owner never judged".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::ToolCallTrace;
    use crate::replay::Divergence;
    use serde_json::json;

    fn tool_use(id: &str, name: &str, input: Value) -> Block {
        Block::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }
    }
    fn result(id: &str, content: &str, is_error: bool) -> Block {
        Block::ToolResult {
            tool_use_id: id.into(),
            content: content.into(),
            is_error,
        }
    }
    fn trace(name: &str, input: Value) -> ToolCallTrace {
        ToolCallTrace {
            name: name.into(),
            input,
            is_error: false,
            denied: false,
            unknown: false,
            staged: false,
        }
    }
    fn report(divergences: Vec<Divergence>, replayed: Vec<ToolCallTrace>) -> ReplayReport {
        branch_report(0, divergences, replayed)
    }
    fn branch_report(
        call_base: usize,
        divergences: Vec<Divergence>,
        replayed: Vec<ToolCallTrace>,
    ) -> ReplayReport {
        ReplayReport {
            divergences,
            replayed_calls: replayed,
            recorded_calls: 0,
            turns: 1,
            stopped_early: false,
            final_text: String::new(),
            stats: Default::default(),
            call_base,
        }
    }

    /// user → assistant(2 calls) → results+steer → assistant(1 call) → results → done
    fn steered_transcript() -> Vec<Message> {
        vec![
            Message::user("audit the reports"),
            Message::assistant(vec![
                tool_use("t1", "fs_list", json!({})),
                tool_use("t2", "fs_read", json!({"path": "a.md"})),
            ]),
            Message {
                harness: false,
                planning: None,
                tool_provenance: Default::default(),
                role: Role::User,
                content: vec![
                    result("t1", "a.md b.md", false),
                    result("t2", "contents", false),
                    Block::text("change of plan: only summarize b.md"),
                ],
            },
            Message::assistant(vec![tool_use("t3", "fs_read", json!({"path": "b.md"}))]),
            Message::user("next task entirely"),
        ]
    }

    #[test]
    fn followup_keeps_the_correction_and_rebases_after_prior_calls() {
        let messages = vec![
            Message::user("old task"),
            Message::assistant(vec![Block::ToolUse {
                id: "a".into(),
                name: "fs_read".into(),
                input: serde_json::json!({"path":"a"}),
            }]),
            Message::user("Use the updated schedule instead"),
            Message::assistant(vec![Block::text("new answer")]),
            Message::user("unrelated next task"),
        ];
        let (index, branch) =
            followup_branch(&messages, "Use the updated schedule instead").unwrap();
        assert_eq!(index, 2);
        assert_eq!(branch.call_base, 1);
        assert_eq!(
            branch.seed.last().unwrap().text(),
            "Use the updated schedule instead"
        );
        assert_eq!(truncate_after_run(&messages, index).len(), 4);
    }

    #[test]
    fn a_steer_is_located_with_the_calls_already_resolved_counted_before_it() {
        let messages = steered_transcript();
        let p = locate_steer(&messages, "change of plan: only summarize b.md").unwrap();
        assert_eq!(p.message_index, 2);
        // t1 and t2 were answered in the same message the steer rode in on, so
        // the counterfactual question starts at call #2.
        assert_eq!(p.call_index, 2);
        assert_eq!(p.kind, ProbeKind::Steer);
        assert!(locate_steer(&messages, "never said").is_none());
        // A followup turn is not a steer, even with matching text.
        assert!(locate_steer(&messages, "next task entirely").is_none());
    }

    /// The steer twin of `learning`'s followup case: a steer that arrives on
    /// the turn a run crosses midnight rides beside the folded calendar
    /// reference, and the locator has to see past it or the counterfactual
    /// probe reports "could not locate the intervention" for exactly those
    /// turns.
    #[test]
    fn a_steer_is_located_through_a_calendar_reference_folded_beside_it() {
        let reference = crate::date_context::render(
            "2026-09-16T13:21:33Z".parse().unwrap(),
            Some(chrono_tz::America::New_York),
        );
        let messages = vec![
            Message::user("summarize a.md and b.md"),
            Message::assistant(vec![tool_use("t1", "fs_read", json!({}))]),
            Message {
                harness: false,
                planning: None,
                tool_provenance: Default::default(),
                role: Role::User,
                content: vec![
                    result("t1", "ok", false),
                    Block::text("only summarize b.md"),
                    Block::text(reference),
                ],
            },
        ];
        let p = locate_steer(&messages, "only summarize b.md")
            .expect("located past the calendar reference");
        assert_eq!(p.message_index, 2);
        assert_eq!(p.kind, ProbeKind::Steer);
    }

    #[test]
    fn a_denial_is_located_with_the_refused_call_attached() {
        let messages = vec![
            Message::user("clean up"),
            Message::assistant(vec![
                tool_use("t1", "fs_list", json!({})),
                tool_use("t2", "fs_write", json!({"path": "notes.md"})),
            ]),
            Message {
                harness: false,
                planning: None,
                tool_provenance: Default::default(),
                role: Role::User,
                content: vec![
                    result("t1", "ok", false),
                    result("t2", "Denied by the user: not that file", true),
                ],
            },
        ];
        let p = locate_denial(&messages, "not that file").unwrap();
        assert_eq!(p.message_index, 2);
        assert_eq!(p.call_index, 1, "the denied call is the second issued");
        assert_eq!(
            p.kind,
            ProbeKind::Denial {
                name: "fs_write".to_string(),
                input: json!({"path": "notes.md"}),
            }
        );
        assert!(locate_denial(&messages, "some other reason").is_none());
    }

    /// A steer's branch keeps the message the steer rode in on — the tool
    /// results the model had already been handed — and removes only the
    /// steering text, so generation resumes exactly where the model read it.
    #[test]
    fn a_steer_branch_keeps_the_results_and_strips_the_steering_text() {
        let messages = steered_transcript();
        let point = locate_steer(&messages, "change of plan: only summarize b.md").unwrap();
        let branch = branch_at(&messages, &point).unwrap();

        assert_eq!(branch.call_base, 2);
        assert_eq!(branch.seed.len(), 3, "seed ends at the steer message");
        let last = branch.seed.last().unwrap();
        assert_eq!(
            last.content
                .iter()
                .filter(|b| matches!(b, Block::ToolResult { .. }))
                .count(),
            2,
            "the results the steer rode beside stay"
        );
        assert!(
            !last.content.iter().any(|b| matches!(b, Block::Text { .. })),
            "the steering text must not reach the counterfactual arm"
        );
    }

    /// A denial's branch cuts before the assistant turn that proposed the
    /// refused call — the whole turn is the decision being re-asked — so the
    /// base is the turn's first call even when the denied call was not.
    #[test]
    fn a_denial_branch_regenerates_the_whole_proposing_turn() {
        let messages = vec![
            Message::user("clean up"),
            Message::assistant(vec![
                tool_use("t1", "fs_list", json!({})),
                tool_use("t2", "fs_write", json!({"path": "notes.md"})),
            ]),
            Message {
                harness: false,
                planning: None,
                tool_provenance: Default::default(),
                role: Role::User,
                content: vec![
                    result("t1", "ok", false),
                    result("t2", "Denied by the user: not that file", true),
                ],
            },
        ];
        let point = locate_denial(&messages, "not that file").unwrap();
        assert_eq!(point.call_index, 1, "the denied call is the second issued");

        let branch = branch_at(&messages, &point).unwrap();
        assert_eq!(
            branch.seed.len(),
            1,
            "the seed ends before the turn that proposed the refused call"
        );
        assert_eq!(
            branch.call_base, 0,
            "the sibling issued beside the denied call regenerates too"
        );
    }

    #[test]
    fn truncation_ends_the_slice_before_the_next_top_level_turn() {
        let messages = steered_transcript();
        let slice = truncate_after_run(&messages, 2);
        assert_eq!(slice.len(), 4, "the follow-on turn is a different question");
        // And an intervention in the last run keeps everything.
        assert_eq!(truncate_after_run(&messages, 4).len(), 5);
    }

    #[test]
    fn a_steer_passes_when_the_replay_tracks_the_recording_through_the_steer() {
        let point = ProbePoint {
            message_index: 2,
            call_index: 2,
            kind: ProbeKind::Steer,
        };
        assert_eq!(verdict(&report(vec![], vec![]), &point), ProbeVerdict::Pass);
        // Argument spellings at the steer point do not fail it.
        let cosmetic = report(
            vec![Divergence::Arguments {
                index: 2,
                tool: "fs_read".into(),
                expected: json!({"path": "b.md"}),
                actual: json!({"path": "./b.md"}),
            }],
            vec![],
        );
        assert_eq!(verdict(&cosmetic, &point), ProbeVerdict::Pass);
    }

    #[test]
    fn a_steer_fails_on_structural_divergence_at_or_after_the_steer_point() {
        let point = ProbePoint {
            message_index: 2,
            call_index: 2,
            kind: ProbeKind::Steer,
        };
        let diverged = report(
            vec![Divergence::Tool {
                index: 2,
                expected: "fs_read".into(),
                actual: "fs_list".into(),
            }],
            vec![],
        );
        assert_eq!(verdict(&diverged, &point), ProbeVerdict::Fail);
        // Stopping short of the steered work is also not doing it.
        let stopped = report(
            vec![Divergence::Missing {
                index: 2,
                expected: "fs_read".into(),
            }],
            vec![],
        );
        assert_eq!(verdict(&stopped, &point), ProbeVerdict::Fail);
    }

    /// The guard the branch design keeps: a divergence where the forced
    /// prefix should be cannot come from the model (nothing was generated
    /// there), so it means the report and the point were built from
    /// different recordings — a caller error, refused as evidence.
    #[test]
    fn a_report_that_diverges_where_the_prefix_was_forced_is_inconclusive() {
        let point = ProbePoint {
            message_index: 2,
            call_index: 2,
            kind: ProbeKind::Steer,
        };
        let early = report(
            vec![Divergence::Tool {
                index: 0,
                expected: "fs_list".into(),
                actual: "shell".into(),
            }],
            vec![],
        );
        match verdict(&early, &point) {
            ProbeVerdict::Inconclusive(why) => assert!(why.contains("before the steer"), "{why}"),
            other => panic!("expected inconclusive, got {other:?}"),
        }
        // A denial's guard keys on the branch base, not the denied call's
        // index — sibling slots regenerating differently is rerouting. Only
        // a divergence *below the base* is impossible for a real branch.
        let denial_point = ProbePoint {
            message_index: 2,
            call_index: 2,
            kind: ProbeKind::Denial {
                name: "fs_write".into(),
                input: json!({}),
            },
        };
        let mismatched = branch_report(
            2,
            vec![Divergence::Tool {
                index: 0,
                expected: "fs_list".into(),
                actual: "shell".into(),
            }],
            vec![],
        );
        assert!(matches!(
            verdict(&mismatched, &denial_point),
            ProbeVerdict::Inconclusive(_)
        ));
        // The same early divergence on a report whose base really is zero is
        // the model's own choice at the decision turn — gradeable, and with
        // no repeat of the refused call in the trace, a pass.
        assert_eq!(verdict(&early, &denial_point), ProbeVerdict::Pass);
    }

    /// The property the [`ProbeKind`] enum exists for: which rule grades a
    /// point is carried by the point, not chosen by the caller.
    ///
    /// One report, two points at the same index, differing only in kind, and
    /// they must disagree. Under the old shape the caller passed a `bool` and
    /// this distinction lived at the call site — where "not a steer" meant
    /// "a denial" by assumption, and a third kind would have been graded by
    /// whichever branch it fell into.
    #[test]
    fn the_kind_decides_the_rule_and_the_caller_does_not() {
        // A replay that tracks the recording exactly and calls `fs_write` on
        // `notes.md` at the decision point.
        let tracked = report(
            vec![],
            vec![
                trace("fs_list", json!({})),
                trace("fs_write", json!({"path": "notes.md"})),
            ],
        );

        // As a steer: no structural divergence means the replay did the
        // steered thing unprompted.
        let as_steer = ProbePoint {
            message_index: 2,
            call_index: 1,
            kind: ProbeKind::Steer,
        };
        assert_eq!(verdict(&tracked, &as_steer), ProbeVerdict::Pass);

        // The same report as a denial of that very call: repeating what the
        // user refused is the one unambiguous failure.
        let as_denial = ProbePoint {
            kind: ProbeKind::Denial {
                name: "fs_write".into(),
                input: json!({"path": "notes.md"}),
            },
            ..as_steer.clone()
        };
        assert_eq!(verdict(&tracked, &as_denial), ProbeVerdict::Fail);
    }

    #[test]
    fn a_denial_fails_only_on_the_exact_refused_call() {
        let point = ProbePoint {
            message_index: 2,
            call_index: 1,
            kind: ProbeKind::Denial {
                name: "fs_write".into(),
                input: json!({"path": "notes.md"}),
            },
        };
        // Repeating the refused call verbatim at the decision point is the
        // failure. (The call before it is the faithful prefix — the denied
        // call sits at index 1.)
        let repeated = report(
            vec![],
            vec![
                trace("fs_list", json!({})),
                trace("fs_write", json!({"path": "notes.md"})),
            ],
        );
        assert_eq!(verdict(&repeated, &point), ProbeVerdict::Fail);
        // Same tool, different target: the user denied an argument, not a
        // capability. Divergence there is the model routing around the denial.
        let rerouted = report(
            vec![Divergence::Tool {
                index: 1,
                expected: "fs_write".into(),
                actual: "fs_read".into(),
            }],
            vec![trace("fs_write", json!({"path": "drafts/notes.md"}))],
        );
        assert_eq!(verdict(&rerouted, &point), ProbeVerdict::Pass);
        // Avoiding the tool entirely passes too.
        let avoided = report(vec![], vec![trace("fs_list", json!({}))]);
        assert_eq!(verdict(&avoided, &point), ProbeVerdict::Pass);
    }

    /// The recording held the same exact call at index 0 — executed, then
    /// later denied at index 1. Under the branch that earlier call lives in
    /// the forced *seed*, never in `replayed_calls`, so it cannot fail the
    /// arm; only the regenerated continuation reaching for the refused call
    /// again is walking into the refusal.
    #[test]
    fn a_denied_call_that_also_appears_before_the_denial_is_not_a_repeat() {
        let point = ProbePoint {
            message_index: 4,
            call_index: 1,
            kind: ProbeKind::Denial {
                name: "fs_write".into(),
                input: json!({"path": "notes.md"}),
            },
        };
        // Branched at the decision turn (base 1): the trace holds only what
        // the model chose fresh, and going elsewhere is compliance.
        let rerouted = branch_report(1, vec![], vec![trace("fs_list", json!({}))]);
        assert_eq!(verdict(&rerouted, &point), ProbeVerdict::Pass);
        // ...whereas the refused call anywhere in the regenerated trace fails
        // — including later than the slot it was originally proposed in.
        let repeated_later = branch_report(
            1,
            vec![],
            vec![
                trace("fs_list", json!({})),
                trace("fs_write", json!({"path": "notes.md"})),
            ],
        );
        assert_eq!(verdict(&repeated_later, &point), ProbeVerdict::Fail);
    }

    /// A denial's branch regenerates the whole turn that proposed the
    /// refused call, so the refused call re-proposed in a regenerated
    /// *sibling* slot — an index below `call_index` but at or above the
    /// branch base — is a fresh choice and fails, not prefix-faithfulness.
    #[test]
    fn a_regenerated_sibling_slot_repeating_the_refused_call_still_fails() {
        let point = ProbePoint {
            message_index: 2,
            call_index: 2,
            kind: ProbeKind::Denial {
                name: "fs_write".into(),
                input: json!({"path": "notes.md"}),
            },
        };
        let sibling_repeat = branch_report(
            1,
            vec![],
            vec![trace("fs_write", json!({"path": "notes.md"}))],
        );
        assert_eq!(verdict(&sibling_repeat, &point), ProbeVerdict::Fail);
    }

    fn draft_point(owner: OwnerAct) -> ProbePoint {
        ProbePoint {
            message_index: 1,
            call_index: 1,
            kind: ProbeKind::Draft {
                tool: "mail_send".into(),
                staged: json!({"to": "dirk@example.invalid", "body": "Totals attached.", "urgent": false}),
                owner,
                schema: json!({"type": "object", "properties": {
                    "to": {"type": "string"},
                    "body": {"type": "string"},
                    "urgent": {"type": "boolean", "default": false}
                }}),
            },
        }
    }

    fn completed(calls: Vec<ToolCallTrace>) -> ReplayReport {
        let mut r = branch_report(1, vec![], calls);
        r.stats.stop_cause = Some(crate::agent::StopCause::Completed);
        r
    }

    /// Row 2d-1's draft validator: the owner's released words pass, the
    /// words they rewrote fail, and a rewording — however close — is a draft
    /// the owner never judged. Whitespace and a pinned default are not a
    /// different draft; case is.
    #[test]
    fn a_released_draft_passes_only_the_owners_words() {
        let point = draft_point(OwnerAct::Released(json!({
            "to": "dirk@example.invalid",
            "body": "Totals attached; the Q3 sheet follows.",
            "urgent": false
        })));
        let arm = |body: &str| {
            completed(vec![trace(
                "mail_send",
                json!({"to": "dirk@example.invalid", "body": body}),
            )])
        };
        assert_eq!(
            verdict(&arm("Totals  attached;\nthe Q3 sheet follows. "), &point),
            ProbeVerdict::Pass,
            "whitespace and the schema's default are not a different draft"
        );
        assert_eq!(
            verdict(&arm("Totals attached."), &point),
            ProbeVerdict::Fail
        );
        for rewording in [
            "Totals attached; the Q3 sheet follows soon.",
            "totals attached; the Q3 sheet follows.",
            "Here are the totals; the Q3 sheet follows.",
        ] {
            assert!(
                matches!(
                    verdict(&arm(rewording), &point),
                    ProbeVerdict::Inconclusive(_)
                ),
                "a rewording is never a pass: {rewording}"
            );
        }
        assert!(
            matches!(
                verdict(&completed(vec![]), &point),
                ProbeVerdict::Inconclusive(_)
            ),
            "sending nothing is not what the owner judged"
        );
    }

    /// A rejection passes an arm that ends without drafting, fails one that
    /// drafts the rejected text, and leaves every other arm inconclusive —
    /// a reworded draft, and an arm a divergence cut short.
    #[test]
    fn a_rejected_draft_passes_only_an_arm_that_chose_to_send_nothing() {
        let point = draft_point(OwnerAct::Rejected);
        assert_eq!(verdict(&completed(vec![]), &point), ProbeVerdict::Pass);
        assert_eq!(
            verdict(
                &completed(vec![trace(
                    "mail_send",
                    json!({"to": "dirk@example.invalid", "body": "Totals attached.", "urgent": null})
                )]),
                &point
            ),
            ProbeVerdict::Fail,
            "a null is an omitted argument, and the default fills it"
        );
        assert!(matches!(
            verdict(
                &completed(vec![trace(
                    "mail_send",
                    json!({"to": "dirk@example.invalid", "body": "Totals attached!"})
                )]),
                &point
            ),
            ProbeVerdict::Inconclusive(_)
        ));
        let cut_short = branch_report(1, vec![], vec![trace("fs_read", json!({"path": "x"}))]);
        assert!(
            matches!(verdict(&cut_short, &point), ProbeVerdict::Inconclusive(_)),
            "an arm stopped before it chose is not a choice to send nothing"
        );
        let mismatched = branch_report(
            1,
            vec![Divergence::Tool {
                index: 0,
                expected: "fs_list".into(),
                actual: "fs_read".into(),
            }],
            vec![],
        );
        assert!(matches!(
            verdict(&mismatched, &point),
            ProbeVerdict::Inconclusive(_)
        ));
    }

    /// The staging call is found by the id the outbox recorded, and a
    /// draft's branch regenerates the whole turn that proposed it.
    #[test]
    fn a_staging_call_is_located_by_its_recorded_id() {
        let messages = vec![
            Message::user("send the totals"),
            Message::assistant(vec![tool_use("t1", "fs_read", json!({"path": "q3.csv"}))]),
            Message::tool_results(vec![result("t1", "1,2,3", false)]),
            Message::assistant(vec![
                tool_use("t2", "fs_list", json!({})),
                tool_use("t3", "mail_send", json!({"body": "Totals attached."})),
            ]),
            Message::tool_results(vec![
                result("t2", "q3.csv", false),
                result("t3", "staged for review", false),
            ]),
        ];
        let locate = |id: &str, tool: &str| {
            locate_staging(
                &messages,
                id,
                tool,
                json!({}),
                OwnerAct::Rejected,
                json!({}),
            )
        };
        let p = locate("t3", "mail_send").unwrap();
        assert_eq!((p.message_index, p.call_index), (3, 2));
        let b = branch_at(&messages, &p).unwrap();
        assert_eq!(b.seed.len(), 3, "cut before the staging turn");
        assert_eq!(b.call_base, 1);
        assert!(
            locate("t3", "fs_list").is_none(),
            "the id names another tool"
        );
        assert!(locate("t9", "mail_send").is_none());
    }
}
