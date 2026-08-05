//! Re-running a recorded session and diffing what changed.
//!
//! The cheap, useful form of this is not "run it again and see": it is to
//! replay the *recorded tool results* and compare only the model's choices.
//! That turns every real session into a regression case for free, costs one
//! model call per turn and no side effects, and — the part that matters — it
//! isolates the variable. Replaying against live tools re-reads a filesystem
//! and a web that have both moved, so a difference tells you nothing about the
//! harness.
//!
//! This module is pure: it extracts a trajectory from a transcript and diffs
//! two of them. Nothing here runs an agent or touches the network, for the same
//! reason [`crate::compact`] is pure — the interesting mistakes are in deciding
//! what counts as "the same", and those should be unit-testable.
//!
//! What it cannot do, and no amount of care will fix: a local server's sampler
//! is outside this process's knowledge, and the same case measures 5/5 rather
//! than deterministically. **Replay against a non-greedy provider is
//! pass@k-shaped, not exact-match-shaped.** One divergent replay is a sample,
//! not a regression.

use crate::agent::ToolCallTrace;
use crate::message::{Block, Message, Role};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One recorded tool call, paired with what it returned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedCall {
    pub name: String,
    pub input: Value,
    /// What the tool returned at record time. Replayed verbatim.
    pub output: String,
    pub is_error: bool,
}

/// A transcript reduced to what a replay needs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Trajectory {
    /// The user's turns, in order — the input side of the replay.
    pub turns: Vec<String>,
    /// Every tool call, in order, with its recorded result.
    pub calls: Vec<RecordedCall>,
    /// The last assistant text. The weakest signal, kept for reporting only.
    pub final_text: String,
    /// True when the recording contains mid-run steering.
    ///
    /// Steering text rides in the same user message as the tool results it
    /// accompanies, because there is no legal slot between a `tool_use` and its
    /// result. That makes it indistinguishable from a turn once flattened, and
    /// re-submitting it as one would change the shape of the conversation being
    /// replayed. Flagged rather than silently dropped: a caller that replays a
    /// steered session anyway should know the comparison is approximate.
    pub steered: bool,
}

/// Reduce a recorded conversation to a replayable trajectory.
///
/// The distinction that does the work here: a user message carrying
/// `tool_result` blocks is the harness feeding results back, *not* the user
/// saying something. Treating those as turns would replay a conversation with
/// twice the turns and none of the same structure.
pub fn extract(messages: &[Message]) -> Trajectory {
    let mut t = Trajectory::default();
    // tool_use blocks awaiting their results, in the order they were issued.
    let mut pending: Vec<(String, String, Value)> = Vec::new();

    for message in messages {
        match message.role {
            Role::Assistant => {
                let text = message.text();
                if !text.trim().is_empty() {
                    t.final_text = text;
                }
                for (id, name, input) in message.tool_uses() {
                    pending.push((id.to_string(), name.to_string(), input.clone()));
                }
            }
            Role::User => {
                let mut results = Vec::new();
                let mut text = String::new();
                for block in &message.content {
                    match block {
                        Block::ToolResult {
                            tool_use_id,
                            content,
                            is_error,
                        } => results.push((tool_use_id.clone(), content.clone(), *is_error)),
                        Block::Text { text: t } => text.push_str(t),
                        _ => {}
                    }
                }

                if results.is_empty() {
                    // A genuine user turn.
                    if !text.trim().is_empty() {
                        t.turns.push(text);
                    }
                    continue;
                }

                // Results coming back. Text alongside them is steering.
                if !text.trim().is_empty() {
                    t.steered = true;
                }
                for (id, output, is_error) in results {
                    // Match by id rather than position: calls are issued in
                    // parallel and nothing promises the results come back in
                    // the order they were asked for.
                    if let Some(i) = pending.iter().position(|(p, _, _)| *p == id) {
                        let (_, name, input) = pending.remove(i);
                        t.calls.push(RecordedCall {
                            name,
                            input,
                            output,
                            is_error,
                        });
                    }
                }
            }
        }
    }

    t
}

/// How a replayed run departed from its recording.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Divergence {
    /// A different tool entirely. The strongest signal there is.
    Tool {
        index: usize,
        expected: String,
        actual: String,
    },
    /// The right tool, different arguments. Worth reporting separately: a
    /// model that reads the same file by a different path spelling has not
    /// regressed, and grading it as though it had makes replay useless inside a
    /// week.
    Arguments {
        index: usize,
        tool: String,
        expected: Value,
        actual: Value,
    },
    /// The replay kept going after the recording ran out.
    Extra { index: usize, actual: String },
    /// The replay stopped early.
    Missing { index: usize, expected: String },
}

impl Divergence {
    /// Where in the call sequence it happened.
    pub fn index(&self) -> usize {
        match self {
            Divergence::Tool { index, .. }
            | Divergence::Arguments { index, .. }
            | Divergence::Extra { index, .. }
            | Divergence::Missing { index, .. } => *index,
        }
    }

    /// Whether this changes *what the model did* rather than how it spelled it.
    ///
    /// Only argument differences are ever cosmetic, and only the caller knows
    /// whether they are — hence a predicate rather than a filter applied here.
    pub fn is_structural(&self) -> bool {
        !matches!(self, Divergence::Arguments { .. })
    }
}

/// Compare a replayed trace against its recording, call by call.
///
/// Positional rather than set-based on purpose: the order tools are called in
/// *is* the trajectory. A run that reads the same four files in a different
/// order made different decisions, and a set comparison would call them equal.
pub fn diff(recorded: &[RecordedCall], replayed: &[ToolCallTrace]) -> Vec<Divergence> {
    let mut out = Vec::new();

    for (index, (want, got)) in recorded.iter().zip(replayed.iter()).enumerate() {
        if want.name != got.name {
            out.push(Divergence::Tool {
                index,
                expected: want.name.clone(),
                actual: got.name.clone(),
            });
            // Once the tools differ, every later comparison is between two
            // sequences that already parted company. Report the first and stop
            // rather than emitting a cascade that all has one cause.
            return out;
        }
        if !same_arguments(&want.input, &got.input) {
            out.push(Divergence::Arguments {
                index,
                tool: want.name.clone(),
                expected: want.input.clone(),
                actual: got.input.clone(),
            });
        }
    }

    for (offset, extra) in replayed.iter().skip(recorded.len()).enumerate() {
        out.push(Divergence::Extra {
            index: recorded.len() + offset,
            actual: extra.name.clone(),
        });
    }
    for (offset, missing) in recorded.iter().skip(replayed.len()).enumerate() {
        out.push(Divergence::Missing {
            index: replayed.len() + offset,
            expected: missing.name.clone(),
        });
    }

    out
}

/// Arguments match when their JSON is equal after normalising whitespace in
/// strings.
///
/// Deliberately not fuzzy beyond that. Path normalisation is tempting —
/// `./a.md` and `a.md` name the same file — but it is tool-specific knowledge,
/// and the loop is not supposed to know what any particular tool means. A
/// caller that wants it can filter on [`Divergence::is_structural`].
fn same_arguments(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(x), Value::String(y)) => x.trim() == y.trim(),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| same_arguments(v, w)))
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(v, w)| same_arguments(v, w))
        }
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(id: &str, name: &str, input: Value) -> Block {
        Block::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }
    }

    fn result(id: &str, content: &str) -> Block {
        Block::ToolResult {
            tool_use_id: id.into(),
            content: content.into(),
            is_error: false,
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

    #[test]
    fn a_recorded_conversation_becomes_turns_and_calls() {
        let messages = vec![
            Message::user("what is in a.md?"),
            Message::assistant(vec![call("t1", "fs_read", json!({"path": "a.md"}))]),
            Message::tool_results(vec![result("t1", "hello")]),
            Message::assistant(vec![Block::text("it says hello")]),
        ];

        let t = extract(&messages);

        // One turn, not two: the tool results are the harness talking, not the
        // user, and counting them would replay a conversation twice as long.
        assert_eq!(t.turns, vec!["what is in a.md?"]);
        assert_eq!(t.calls.len(), 1);
        assert_eq!(t.calls[0].name, "fs_read");
        assert_eq!(t.calls[0].output, "hello");
        assert_eq!(t.final_text, "it says hello");
        assert!(!t.steered);
    }

    #[test]
    fn several_user_turns_are_all_kept_in_order() {
        let messages = vec![
            Message::user("first"),
            Message::assistant(vec![Block::text("ok")]),
            Message::user("second"),
            Message::assistant(vec![Block::text("ok again")]),
        ];

        assert_eq!(extract(&messages).turns, vec!["first", "second"]);
    }

    #[test]
    fn results_are_paired_by_id_not_by_arrival_order() {
        // Parallel calls, results back in the other order — which is allowed,
        // and which position-matching would silently mis-pair, attaching each
        // call to the other's output.
        let messages = vec![
            Message::user("read both"),
            Message::assistant(vec![
                call("t1", "fs_read", json!({"path": "a.md"})),
                call("t2", "fs_read", json!({"path": "b.md"})),
            ]),
            Message::tool_results(vec![result("t2", "B"), result("t1", "A")]),
        ];

        let t = extract(&messages);

        assert_eq!(t.calls.len(), 2);
        let by_path = |p: &str| {
            t.calls
                .iter()
                .find(|c| c.input["path"] == p)
                .unwrap_or_else(|| panic!("no call for {p}"))
        };
        assert_eq!(by_path("a.md").output, "A");
        assert_eq!(by_path("b.md").output, "B");
    }

    #[test]
    fn steering_is_flagged_rather_than_mistaken_for_a_turn() {
        // The text rides with the results because there is no legal slot
        // between a `tool_use` and its result. Replaying it as a turn would
        // change the shape of the conversation under test.
        let messages = vec![
            Message::user("start"),
            Message::assistant(vec![call("t1", "shell", json!({"command": "sleep 6"}))]),
            Message::tool_results(vec![
                result("t1", ""),
                Block::text("change of plan: just say PIVOT"),
            ]),
            Message::assistant(vec![Block::text("PIVOT")]),
        ];

        let t = extract(&messages);

        assert_eq!(t.turns, vec!["start"], "steering became a user turn");
        assert!(t.steered, "a steered recording must say so");
    }

    #[test]
    fn an_identical_replay_has_nothing_to_report() {
        let recorded = vec![RecordedCall {
            name: "fs_read".into(),
            input: json!({"path": "a.md"}),
            output: "hello".into(),
            is_error: false,
        }];
        let replayed = vec![trace("fs_read", json!({"path": "a.md"}))];

        assert!(diff(&recorded, &replayed).is_empty());
    }

    #[test]
    fn a_different_tool_stops_the_comparison_rather_than_cascading() {
        // Everything after the fork is two sequences that already parted
        // company; reporting all of it buries the one fact that matters.
        let recorded = vec![
            RecordedCall {
                name: "fs_read".into(),
                input: json!({}),
                output: String::new(),
                is_error: false,
            },
            RecordedCall {
                name: "fs_read".into(),
                input: json!({}),
                output: String::new(),
                is_error: false,
            },
            RecordedCall {
                name: "fs_read".into(),
                input: json!({}),
                output: String::new(),
                is_error: false,
            },
        ];
        let replayed = vec![
            trace("shell", json!({})),
            trace("shell", json!({})),
            trace("shell", json!({})),
        ];

        let d = diff(&recorded, &replayed);

        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0],
            Divergence::Tool {
                index: 0,
                expected: "fs_read".into(),
                actual: "shell".into()
            }
        );
        assert!(d[0].is_structural());
    }

    #[test]
    fn the_same_tool_with_different_arguments_is_reported_but_not_structural() {
        let recorded = vec![RecordedCall {
            name: "fs_read".into(),
            input: json!({"path": "a.md"}),
            output: String::new(),
            is_error: false,
        }];
        let replayed = vec![trace("fs_read", json!({"path": "./a.md"}))];

        let d = diff(&recorded, &replayed);

        assert_eq!(d.len(), 1);
        // A caller deciding what counts as a regression needs these separable:
        // the same file by another spelling is not a behaviour change.
        assert!(!d[0].is_structural());
    }

    #[test]
    fn running_long_and_stopping_early_are_different_findings() {
        let one = |name: &str| RecordedCall {
            name: name.into(),
            input: json!({}),
            output: String::new(),
            is_error: false,
        };

        let extra = diff(
            &[one("fs_read")],
            &[trace("fs_read", json!({})), trace("shell", json!({}))],
        );
        assert_eq!(
            extra,
            vec![Divergence::Extra {
                index: 1,
                actual: "shell".into()
            }]
        );

        let missing = diff(
            &[one("fs_read"), one("shell")],
            &[trace("fs_read", json!({}))],
        );
        assert_eq!(
            missing,
            vec![Divergence::Missing {
                index: 1,
                expected: "shell".into()
            }]
        );
    }

    #[test]
    fn order_is_part_of_the_trajectory_not_an_incidental_detail() {
        // A set comparison would call these equal. They are not: reading the
        // files in a different order is a different set of decisions.
        let one = |p: &str| RecordedCall {
            name: "fs_read".into(),
            input: json!({"path": p}),
            output: String::new(),
            is_error: false,
        };
        let d = diff(
            &[one("a.md"), one("b.md")],
            &[
                trace("fs_read", json!({"path": "b.md"})),
                trace("fs_read", json!({"path": "a.md"})),
            ],
        );

        assert_eq!(d.len(), 2, "a reordering went unreported");
    }

    #[test]
    fn whitespace_in_arguments_does_not_count_as_a_change() {
        let recorded = vec![RecordedCall {
            name: "shell".into(),
            input: json!({"command": "ls -la"}),
            output: String::new(),
            is_error: false,
        }];
        let replayed = vec![trace("shell", json!({"command": "  ls -la  "}))];

        assert!(diff(&recorded, &replayed).is_empty());
    }
}
