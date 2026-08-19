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
//! The trick that makes steers cheap is that [`crate::replay::extract`]
//! already drops steering text (it rides beside tool results, and there is no
//! legal slot to re-inject it), so a replay of a steered session *is* the
//! no-steer counterfactual. And the recording after the steer is ground truth
//! for what the user wanted — they steered it there. So the verdict is
//! structural, not judged:
//!
//! - **Steer**: pass iff the replay tracks the recording *through* the steer
//!   point — the model does the steered thing without the steer. Divergence
//!   at or after that call index is a fail; divergence before it means the
//!   replay went off the rails before the question was even posed, which is
//!   inconclusive, not evidence.
//! - **Denial**: pass iff the replay reaches the decision point and never
//!   makes the denied call (same tool, same arguments) again. Repeating the
//!   exact call the user refused is the one unambiguous failure. Same tool
//!   with different arguments is *not* a fail — "not that directory" denies
//!   an argument, not a tool — and the report carries the trace for reading.
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
    /// For a denial: the call the user refused. `None` for a steer.
    pub denied: Option<(String, Value)>,
}

fn calls_before(messages: &[Message], m: usize) -> usize {
    messages[..m]
        .iter()
        .filter(|msg| msg.role == Role::Assistant)
        .map(|msg| msg.tool_uses().len())
        .sum()
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
            && msg.text().trim() == wanted
    })?;
    // The steer arrived alongside results, so those calls were already
    // resolved by the time the model read it: they count as "before".
    let call_index = calls_before(messages, m + 1);
    Some(ProbePoint {
        message_index: m,
        call_index,
        denied: None,
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
                        denied: Some((name.to_string(), (*input).clone())),
                    });
                }
            }
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
    /// The replay departed from the recording before the intervention point,
    /// so the question was never posed. Not evidence in either direction.
    Inconclusive(String),
}

/// Grade one replayed arm of a steer probe.
pub fn steer_verdict(report: &ReplayReport, point: &ProbePoint) -> ProbeVerdict {
    let k = point.call_index;
    if let Some(d) = report.structural().find(|d| d.index() < k) {
        return ProbeVerdict::Inconclusive(format!(
            "diverged at call #{} — before the steer point (call #{k})",
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
pub fn denial_verdict(report: &ReplayReport, point: &ProbePoint) -> ProbeVerdict {
    let k = point.call_index;
    let (name, input) = point
        .denied
        .as_ref()
        .expect("a denial point carries the call");
    if let Some(d) = report.structural().find(|d| d.index() < k) {
        return ProbeVerdict::Inconclusive(format!(
            "diverged at call #{} — before the denied call (call #{k})",
            d.index()
        ));
    }
    // The one unambiguous failure: making the exact call the user refused,
    // at or after the point where they refused it. The name alone is not
    // enough — a denial usually refuses an argument (that file, that
    // directory), not a capability. And the scan starts at k, not zero:
    // calls before k are the replay faithfully following the recording, and
    // a recording that happened to contain the same call earlier must not
    // fail both arms for it. Positions align with recording indices here
    // because a structural divergence before k already returned above.
    let repeated = report
        .replayed_calls
        .iter()
        .skip(k)
        .any(|c| c.name == *name && c.input == *input);
    if repeated {
        ProbeVerdict::Fail
    } else {
        ProbeVerdict::Pass
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
        ReplayReport {
            divergences,
            replayed_calls: replayed,
            recorded_calls: 0,
            turns: 1,
            stopped_early: false,
            final_text: String::new(),
            stats: Default::default(),
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
    fn a_steer_is_located_with_the_calls_already_resolved_counted_before_it() {
        let messages = steered_transcript();
        let p = locate_steer(&messages, "change of plan: only summarize b.md").unwrap();
        assert_eq!(p.message_index, 2);
        // t1 and t2 were answered in the same message the steer rode in on, so
        // the counterfactual question starts at call #2.
        assert_eq!(p.call_index, 2);
        assert!(p.denied.is_none());
        assert!(locate_steer(&messages, "never said").is_none());
        // A followup turn is not a steer, even with matching text.
        assert!(locate_steer(&messages, "next task entirely").is_none());
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
            p.denied,
            Some(("fs_write".to_string(), json!({"path": "notes.md"})))
        );
        assert!(locate_denial(&messages, "some other reason").is_none());
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
            denied: None,
        };
        assert_eq!(
            steer_verdict(&report(vec![], vec![]), &point),
            ProbeVerdict::Pass
        );
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
        assert_eq!(steer_verdict(&cosmetic, &point), ProbeVerdict::Pass);
    }

    #[test]
    fn a_steer_fails_on_structural_divergence_at_or_after_the_steer_point() {
        let point = ProbePoint {
            message_index: 2,
            call_index: 2,
            denied: None,
        };
        let diverged = report(
            vec![Divergence::Tool {
                index: 2,
                expected: "fs_read".into(),
                actual: "fs_list".into(),
            }],
            vec![],
        );
        assert_eq!(steer_verdict(&diverged, &point), ProbeVerdict::Fail);
        // Stopping short of the steered work is also not doing it.
        let stopped = report(
            vec![Divergence::Missing {
                index: 2,
                expected: "fs_read".into(),
            }],
            vec![],
        );
        assert_eq!(steer_verdict(&stopped, &point), ProbeVerdict::Fail);
    }

    #[test]
    fn a_probe_that_derails_before_the_point_is_inconclusive_not_evidence() {
        let point = ProbePoint {
            message_index: 2,
            call_index: 2,
            denied: None,
        };
        let early = report(
            vec![Divergence::Tool {
                index: 0,
                expected: "fs_list".into(),
                actual: "shell".into(),
            }],
            vec![],
        );
        match steer_verdict(&early, &point) {
            ProbeVerdict::Inconclusive(why) => assert!(why.contains("before the steer"), "{why}"),
            other => panic!("expected inconclusive, got {other:?}"),
        }
        let denial_point = ProbePoint {
            message_index: 2,
            call_index: 2,
            denied: Some(("fs_write".into(), json!({}))),
        };
        assert!(matches!(
            denial_verdict(&early, &denial_point),
            ProbeVerdict::Inconclusive(_)
        ));
    }

    #[test]
    fn a_denial_fails_only_on_the_exact_refused_call() {
        let point = ProbePoint {
            message_index: 2,
            call_index: 1,
            denied: Some(("fs_write".into(), json!({"path": "notes.md"}))),
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
        assert_eq!(denial_verdict(&repeated, &point), ProbeVerdict::Fail);
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
        assert_eq!(denial_verdict(&rerouted, &point), ProbeVerdict::Pass);
        // Avoiding the tool entirely passes too.
        let avoided = report(vec![], vec![trace("fs_list", json!({}))]);
        assert_eq!(denial_verdict(&avoided, &point), ProbeVerdict::Pass);
    }

    #[test]
    fn a_denied_call_that_also_appears_before_the_denial_is_not_a_repeat() {
        // The recording held the same exact call at index 0 — executed, then
        // later denied at index 1. A replay faithfully walking the prefix
        // makes that first call; only making it again at or after the denial
        // point is walking into the refusal.
        let point = ProbePoint {
            message_index: 4,
            call_index: 1,
            denied: Some(("fs_write".into(), json!({"path": "notes.md"}))),
        };
        let rerouted_after_prefix = report(
            vec![],
            vec![
                trace("fs_write", json!({"path": "notes.md"})),
                trace("fs_list", json!({})),
            ],
        );
        // Call #0 matches the denied call textually, but call #1 — the
        // decision point — went elsewhere: that is compliance.
        assert_eq!(
            denial_verdict(&rerouted_after_prefix, &point),
            ProbeVerdict::Pass
        );
        // ...whereas repeating it anywhere from the decision point on fails.
        let repeated_later = report(
            vec![],
            vec![
                trace("fs_write", json!({"path": "notes.md"})),
                trace("fs_list", json!({})),
                trace("fs_write", json!({"path": "notes.md"})),
            ],
        );
        assert_eq!(denial_verdict(&repeated_later, &point), ProbeVerdict::Fail);
    }
}
