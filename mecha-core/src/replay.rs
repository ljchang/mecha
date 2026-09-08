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
    /// Which batch of concurrently-issued calls this one belonged to.
    ///
    /// One assistant turn emits its `tool_use` blocks together and the loop
    /// runs them through `join_all`, so the order they appear in is an
    /// artifact of generation, not a decision the model made. Recording the
    /// grouping lets a replay accept the same batch in a different order —
    /// worth four of the twelve episodes dropped by the 2026-09-08 harness
    /// pass, each killed at call #0 or #1 by a batch it had reproduced
    /// exactly.
    ///
    /// `None` degrades to its own batch of one — the strict positional
    /// matching this module did throughout. No production path produces it:
    /// `Trajectory` is never persisted, and every caller derives one fresh
    /// from a transcript (`harness_probe`, `probe`, `commands::replay`), so
    /// `extract` marks every call. The default is defensive, for the day this
    /// type is stored rather than a migration anyone is mid-way through.
    #[serde(default)]
    pub batch: Option<u32>,
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
    // Results come back one message per assistant turn — the API allows no
    // other shape — so counting those messages numbers the batches.
    let mut batch: u32 = 0;

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
                            batch: Some(batch),
                        });
                    }
                }
                batch += 1;
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

/// [`diff`], for a replay that joined the recording mid-stream: `recorded` is
/// the tail from `base` onward, and every reported index comes back shifted
/// into the *full* recording's coordinates. A probe point is located in those
/// coordinates, so a branched replay's divergences must land there too, or
/// "before the steer point" compares indices from two different countings.
pub fn diff_from(
    base: usize,
    recorded: &[RecordedCall],
    replayed: &[ToolCallTrace],
) -> Vec<Divergence> {
    let mut out = diff(recorded, replayed);
    for d in &mut out {
        match d {
            Divergence::Tool { index, .. }
            | Divergence::Arguments { index, .. }
            | Divergence::Extra { index, .. }
            | Divergence::Missing { index, .. } => *index += base,
        }
    }
    out
}

/// How many recorded calls, starting at `i`, were issued together.
///
/// One when the call carries no batch marker — the strict behaviour, reached
/// by the same code path rather than a branch somewhere else that has to agree
/// with it. See [`RecordedCall::batch`] for why nothing in production is
/// unmarked.
fn batch_width(recorded: &[RecordedCall], i: usize) -> usize {
    match recorded[i].batch {
        None => 1,
        Some(b) => recorded[i..]
            .iter()
            .take_while(|c| c.batch == Some(b))
            .count(),
    }
}

/// Pair a recorded batch with the replayed calls facing it, ignoring order.
///
/// `Some` maps each recorded call to the replayed one answering it, and exists
/// only when the two are the same multiset of names — a batch that gained,
/// lost or swapped a *tool* is a real divergence and comes back `None`.
///
/// Two passes, arguments first. A batch that called one tool twice has no
/// order to appeal to, so pairing it by name alone would mate each call with
/// its sibling and report two argument differences where the replay was
/// exact; matching identical arguments first pairs those calls with
/// themselves. The name-only pass then takes the calls whose arguments really
/// did change, which is the difference this is here to report.
fn pair_batch(want: &[RecordedCall], got: &[ToolCallTrace]) -> Option<Vec<(usize, usize)>> {
    if want.len() != got.len() {
        return None;
    }
    let mut used = vec![false; got.len()];
    let mut mate: Vec<Option<usize>> = vec![None; want.len()];

    for (w, call) in want.iter().enumerate() {
        if let Some(g) = (0..got.len()).find(|&g| {
            !used[g] && got[g].name == call.name && same_arguments(&call.input, &got[g].input)
        }) {
            used[g] = true;
            mate[w] = Some(g);
        }
    }
    for (w, call) in want.iter().enumerate() {
        if mate[w].is_some() {
            continue;
        }
        let g = (0..got.len()).find(|&g| !used[g] && got[g].name == call.name)?;
        used[g] = true;
        mate[w] = Some(g);
    }

    Some(
        mate.into_iter()
            .enumerate()
            .filter_map(|(w, g)| g.map(|g| (w, g)))
            .collect(),
    )
}

/// Compare a replayed trace against its recording, batch by batch.
///
/// Positional *between* batches on purpose: the order tools are called in
/// across turns *is* the trajectory. A run that reads the same four files in
/// four separate turns, in a different order, made different decisions, and a
/// set comparison would call them equal.
///
/// Within one batch it is the opposite. Those calls were issued together and
/// executed concurrently by `join_all`, so their order carries no decision and
/// grading it as one drops episodes that reproduced the recording exactly.
/// [`RecordedCall::batch`] is what tells the two apart; an unmarked call is its
/// own batch, which is the strict positional comparison this did throughout.
///
/// This must agree with what the replay's own cursor accepts
/// (`replay_run::ReplayTool::decide`) — a run that never left the recording
/// reporting a divergence, or the reverse, is the split that makes a replay
/// unreadable.
pub fn diff(recorded: &[RecordedCall], replayed: &[ToolCallTrace]) -> Vec<Divergence> {
    let mut out = Vec::new();

    let mut i = 0;
    while i < recorded.len() && i < replayed.len() {
        let end = (i + batch_width(recorded, i)).min(recorded.len());
        let want = &recorded[i..end];
        let got = &replayed[i..end.min(replayed.len())];

        match pair_batch(want, got) {
            // Same batch, whatever order it arrived in. Arguments are still
            // compared, each against the recorded call it actually answers.
            Some(pairs) => {
                for (w, g) in pairs {
                    if !same_arguments(&want[w].input, &got[g].input) {
                        out.push(Divergence::Arguments {
                            index: i + w,
                            tool: want[w].name.clone(),
                            expected: want[w].input.clone(),
                            actual: got[g].input.clone(),
                        });
                    }
                }
            }
            // A genuinely different batch, or one the replay never finished.
            None => {
                if let Some((offset, want, got)) = want
                    .iter()
                    .zip(got.iter())
                    .enumerate()
                    .find(|(_, (w, g))| w.name != g.name)
                    .map(|(offset, (w, g))| (offset, w, g))
                {
                    out.push(Divergence::Tool {
                        index: i + offset,
                        expected: want.name.clone(),
                        actual: got.name.clone(),
                    });
                    // Once the tools differ, every later comparison is between
                    // two sequences that already parted company. Report the
                    // first and stop rather than emitting a cascade that all
                    // has one cause.
                    return out;
                }
                // The names agree as far as both sequences go, so the batch was
                // cut short rather than changed. The `Missing` pass below is
                // what reports that, and it counts the whole tail.
                break;
            }
        }
        i = end;
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
pub(crate) fn same_arguments(a: &Value, b: &Value) -> bool {
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

    /// A recorded call with no batch marker; the batch tests set `batch` with
    /// struct update syntax so the grouping is visible at each call site.
    fn one(name: &str, input: Value) -> RecordedCall {
        RecordedCall {
            name: name.into(),
            input,
            output: String::new(),
            is_error: false,
            batch: None,
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

    /// The grouping the batch-tolerant matching is built on. Calls issued in
    /// one assistant turn share a batch; the next turn starts a new one.
    #[test]
    fn calls_issued_together_share_a_batch_and_the_next_turn_starts_another() {
        let messages = vec![
            Message::user("look at both, then read one"),
            Message::assistant(vec![
                call("t1", "kg_search", json!({"q": "a"})),
                call("t2", "fs_list", json!({"path": "."})),
            ]),
            Message::tool_results(vec![result("t1", "A"), result("t2", "B")]),
            Message::assistant(vec![call("t3", "fs_read", json!({"path": "a.md"}))]),
            Message::tool_results(vec![result("t3", "C")]),
        ];

        let t = extract(&messages);

        let batches: Vec<_> = t.calls.iter().map(|c| c.batch).collect();
        assert_eq!(batches, vec![Some(0), Some(0), Some(1)]);
    }

    /// The regression. The model issued the same batch, and the loop runs a
    /// batch concurrently, so the order it happened to emit them in is not a
    /// decision — grading it as one dropped four of the twelve episodes the
    /// 2026-09-08 harness pass lost, each at call #0 or #1.
    #[test]
    fn a_batch_replayed_in_another_order_is_not_a_divergence() {
        let recorded = vec![
            RecordedCall {
                batch: Some(0),
                ..one("kg_search", json!({"q": "a"}))
            },
            RecordedCall {
                batch: Some(0),
                ..one("fs_list", json!({"path": "."}))
            },
        ];
        let replayed = vec![
            trace("fs_list", json!({"path": "."})),
            trace("kg_search", json!({"q": "a"})),
        ];

        assert_eq!(diff(&recorded, &replayed), vec![]);
    }

    /// The other half of the same rule, and the reason this is not simply a
    /// set comparison: across turns the order *is* the trajectory, because
    /// each call was chosen after seeing the previous one's result.
    #[test]
    fn the_same_two_calls_in_separate_turns_still_diverge_when_swapped() {
        let recorded = vec![
            RecordedCall {
                batch: Some(0),
                ..one("kg_search", json!({"q": "a"}))
            },
            RecordedCall {
                batch: Some(1),
                ..one("fs_list", json!({"path": "."}))
            },
        ];
        let replayed = vec![
            trace("fs_list", json!({"path": "."})),
            trace("kg_search", json!({"q": "a"})),
        ];

        assert_eq!(
            diff(&recorded, &replayed),
            vec![Divergence::Tool {
                index: 0,
                expected: "kg_search".into(),
                actual: "fs_list".into(),
            }]
        );
    }

    /// Tolerating order must not tolerate a different *tool*: that is the
    /// signal the whole module exists to catch.
    #[test]
    fn a_batch_that_swapped_a_tool_still_diverges() {
        let recorded = vec![
            RecordedCall {
                batch: Some(0),
                ..one("kg_search", json!({"q": "a"}))
            },
            RecordedCall {
                batch: Some(0),
                ..one("fs_list", json!({"path": "."}))
            },
        ];
        let replayed = vec![
            trace("kg_search", json!({"q": "a"})),
            trace("kg_entity", json!({"id": "x"})),
        ];

        assert_eq!(
            diff(&recorded, &replayed),
            vec![Divergence::Tool {
                index: 1,
                expected: "fs_list".into(),
                actual: "kg_entity".into(),
            }]
        );
    }

    /// A batch calling one tool twice has no order to appeal to, so the
    /// arguments are what say which call is which. Pairing by name alone would
    /// mate each with its sibling and report two argument differences over a
    /// replay that was exact.
    #[test]
    fn a_reordered_batch_pairs_repeated_tools_by_their_arguments() {
        let recorded = vec![
            RecordedCall {
                batch: Some(0),
                ..one("fs_read", json!({"path": "a.md"}))
            },
            RecordedCall {
                batch: Some(0),
                ..one("fs_read", json!({"path": "b.md"}))
            },
        ];
        let replayed = vec![
            trace("fs_read", json!({"path": "b.md"})),
            trace("fs_read", json!({"path": "a.md"})),
        ];

        assert_eq!(diff(&recorded, &replayed), vec![]);
    }

    /// And an argument that genuinely changed is still reported, against the
    /// recorded call it answers rather than the one at its position.
    #[test]
    fn a_reordered_batch_still_reports_a_changed_argument() {
        let recorded = vec![
            RecordedCall {
                batch: Some(0),
                ..one("kg_search", json!({"q": "a"}))
            },
            RecordedCall {
                batch: Some(0),
                ..one("fs_list", json!({"path": "."}))
            },
        ];
        let replayed = vec![
            trace("fs_list", json!({"path": "src"})),
            trace("kg_search", json!({"q": "a"})),
        ];

        assert_eq!(
            diff(&recorded, &replayed),
            vec![Divergence::Arguments {
                index: 1,
                tool: "fs_list".into(),
                expected: json!({"path": "."}),
                actual: json!({"path": "src"}),
            }]
        );
    }

    /// An unmarked call gets strict positional matching. `extract` marks every
    /// call and `Trajectory` is never persisted, so this guards the struct's
    /// defensive default rather than a recording anyone has — worth keeping
    /// so the default cannot quietly become "match anything, anywhere".
    #[test]
    fn calls_with_no_batch_marker_are_matched_strictly() {
        let recorded = vec![
            one("kg_search", json!({"q": "a"})),
            one("fs_list", json!({"path": "."})),
        ];
        let replayed = vec![
            trace("fs_list", json!({"path": "."})),
            trace("kg_search", json!({"q": "a"})),
        ];

        assert_eq!(
            diff(&recorded, &replayed),
            vec![Divergence::Tool {
                index: 0,
                expected: "kg_search".into(),
                actual: "fs_list".into(),
            }]
        );
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
            batch: None,
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
                batch: None,
            },
            RecordedCall {
                name: "fs_read".into(),
                input: json!({}),
                output: String::new(),
                is_error: false,
                batch: None,
            },
            RecordedCall {
                name: "fs_read".into(),
                input: json!({}),
                output: String::new(),
                is_error: false,
                batch: None,
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
            batch: None,
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
            batch: None,
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
        // A set comparison would call these equal. They are not: read in
        // separate turns, the second was chosen after seeing the first one's
        // result, so a different order is a different set of decisions.
        // Within *one* batch the opposite holds — see
        // `a_batch_replayed_in_another_order_is_not_a_divergence`.
        let one = |b: u32, p: &str| RecordedCall {
            name: "fs_read".into(),
            input: json!({"path": p}),
            output: String::new(),
            is_error: false,
            batch: Some(b),
        };
        let d = diff(
            &[one(0, "a.md"), one(1, "b.md")],
            &[
                trace("fs_read", json!({"path": "b.md"})),
                trace("fs_read", json!({"path": "a.md"})),
            ],
        );

        assert_eq!(d.len(), 2, "a reordering went unreported");
    }

    /// A branched replay diffs the recording's tail, but a probe point lives
    /// in the full recording's coordinates — reported indices must land there.
    #[test]
    fn diff_from_reports_indices_in_the_full_recordings_coordinates() {
        let recorded_tail = vec![
            RecordedCall {
                name: "fs_read".into(),
                input: json!({}),
                output: String::new(),
                is_error: false,
                batch: None,
            },
            RecordedCall {
                name: "fs_read".into(),
                input: json!({}),
                output: String::new(),
                is_error: false,
                batch: None,
            },
        ];
        let replayed = vec![trace("fs_read", json!({})), trace("shell", json!({}))];

        let d = diff_from(10, &recorded_tail, &replayed);

        assert_eq!(
            d,
            vec![Divergence::Tool {
                index: 11,
                expected: "fs_read".into(),
                actual: "shell".into()
            }],
            "a divergence at tail position 1 sits at recording position 11"
        );
        // And a zero base is exactly `diff`.
        assert_eq!(
            diff_from(0, &recorded_tail, &replayed),
            diff(&recorded_tail, &replayed)
        );
    }

    #[test]
    fn whitespace_in_arguments_does_not_count_as_a_change() {
        let recorded = vec![RecordedCall {
            name: "shell".into(),
            input: json!({"command": "ls -la"}),
            output: String::new(),
            is_error: false,
            batch: None,
        }];
        let replayed = vec![trace("shell", json!({"command": "  ls -la  "}))];

        assert!(diff(&recorded, &replayed).is_empty());
    }
}
