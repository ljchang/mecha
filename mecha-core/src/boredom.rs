//! Nothing is being learned from this approach — `docs/GOAL-SYSTEM-DESIGN.md`
//! §9.1, rungs 1 and 3.
//!
//! **The loop guard is the crudest possible version of this**, and until now
//! the only one: it fires on an identical call with an identical result inside
//! a window after a compaction, and its response is rung 5 — end the run. So a
//! run that is going nowhere had exactly two states, *proceeding* and *dead*.
//! (Its second trigger, one call failing identically for `STILL_STUCK + 1`
//! turns, waits for this ladder to finish on purpose — `agent::LoopGuard`.)
//! This is the graded version, and it fires earlier for the reason §4.4 gives
//! for predicting context pressure rather than reacting to it: acting before a
//! deviation beats reacting to one.
//!
//! **It spends nothing, which is what makes it ungated.** The run was going to
//! happen; boredom only changes *how*. That is the whole distinction from
//! curiosity (§9.2), which starts work nobody asked for and is therefore
//! preempted by everything with a person attached.
//!
//! Three properties, each of which is a bug if undone:
//!
//! - **Keyed on the call *and* its result**, on the loop guard's rule.
//!   Identical arguments with a changing result is polling, and a poll must
//!   never grade as stuck. The key is `compact::target_of` rather than the raw
//!   arguments, so two different tools that read the same file and get the same
//!   bytes count as the same thing learned twice — which is exactly what this
//!   is looking for.
//! - **Once per rung, never per turn.** The count is compared with `==` rather
//!   than `>=`, so crossing a rung fires exactly once. A notice repeated every
//!   turn would be the distractor shape `evict_superseded_results` exists to
//!   remove, and worse than that: a model is measurably likelier to fail a step
//!   when its context holds its own earlier errors, so nagging about being
//!   stuck is a way of making it stick.
//! - **The response is the model's.** The harness names the condition and what
//!   is actually reachable; it does not change the approach, because the
//!   approach is the model's. Rungs 4 and 5 — ask, and stop — are not here:
//!   `questions.rs` and the loop guard already own them.
//!
//! **Rung 2 — consult — is deliberately missing, and the reason is not
//! sequencing.** §9.1 offers two things to consult: a marker for this
//! situation, which is §7.4's and does not exist, and a skill, which does —
//! but nothing in the `Tool` trait identifies the tool that loads one.
//! `narrows_surface_to` is the closest and answers `None` until a skill is
//! already loaded, so it recognises the state this notice exists to escape
//! only after the escape has been taken. Naming `skill` by name from the loop
//! is the alternative and is the thing the trait family exists to avoid: the
//! loop learns that *some* tool has a property, never which tool has it. So
//! the rung waits for a property worth adding rather than being approximated
//! by a string.
//!
//! **A second way to reach rung 1: the same failure on changing arguments.**
//! The key above cannot see a model *guessing* — `fs_list` on one path
//! outside the workspace, then another, then a third; a `shell` command a
//! policy forbids, reworded each turn; `image_view` on three invented file
//! names. Every call has a new target, so every outcome is new, and the
//! one thing that is not changing — the error — is the thing the run should
//! be learning from. A corpus pass on 2026-10-30 found these were most of the
//! model-side failures: path-jail errors, policy blocks and invented paths,
//! each concentrated in a handful of sessions that repeated one refusal
//! across many arguments. So failures are also counted by their **shape**
//! ([`error_shape`]: the error with every quoted span, path and number
//! masked) per tool, and the count is of turns that brought the shape back
//! on a target the streak had *not* seen. The two keys partition the
//! repeats: an unchanged call is the first key's (and the loop guard's), and
//! an unchanged call whose error moves is a poll, which this second key
//! cannot count because its target is not new. A call the *machine* refused
//! (`ToolCallTrace::blocked`: a policy, a hook, the interlock) is told so in
//! different words — a refusal is about what the call does, and rewording it
//! is the one response guaranteed to be refused again. **A person's
//! `Decision::Deny` never reaches this key**: it is a correction, every
//! interactive approver words it alike, and "stop asking" is exactly the
//! wrong thing to say about one (review of #762). The notice is added beside
//! the results; the refusal itself is never touched, so `"Blocked by
//! policy:"` stays `Blocked` for every reader downstream.

use std::collections::{HashMap, HashSet};

/// Identical outcomes before an approach counts as going nowhere.
///
/// Two is ordinary work — a retry is how things get done, and the eval rig's
/// own rule is that one failure among successes is recovery. Three identical
/// outcomes is the model not learning anything from the last two. Deliberately
/// *not* the loop guard's threshold of two: that one fires only after a
/// compaction, where the failure is specific and expensive, and it kills the
/// run. This one watches all of ordinary work and only speaks, so it has to be
/// slower to accuse.
const STUCK: u32 = 3;

/// …and after which the cheap escapes have demonstrably not worked.
pub(crate) const STILL_STUCK: u32 = 6;

/// What one run may say about being stuck.
///
/// A notice stays true — unlike a headroom reading, it does not go stale — so
/// the bound is about bulk and about self-conditioning rather than about
/// accuracy. Three is enough for a run that is stuck on genuinely different
/// things and short of the point where the transcript is mostly the harness
/// talking about the harness.
const MAX_NOTICES: u32 = 3;

/// How many turns may pass between two occurrences of the same target before
/// they stop counting as one streak.
///
/// Without this, `seen` accumulates for the life of the run, so "three
/// identical outcomes" meant three *anywhere*, not three in a row — the same
/// `shell: git status` at three natural checkpoints an hour apart would trip
/// it exactly as a genuinely stuck run would, on a detector whose only job is
/// telling those two apart. The loop guard this is modelled on is explicitly
/// windowed for the same reason; this one was not. `STUCK` itself is the
/// natural size: the window has to admit at least the ordinary work between
/// two repeats of a stuck call, and cannot be wider than the count that
/// defines "stuck" without the two numbers arguing with each other.
const RECENCY_WINDOW: u32 = STUCK;

/// How every notice opens.
///
/// **A constant, because the transcript is the only thing that can tell a
/// reader who spoke.** A notice is folded into the message carrying the tool
/// results — steering's slot — and `learning::extract_interventions` reads text
/// riding beside tool results as *the user steering*, which is right for
/// everything else that lands there. Without a stem the harness's own words
/// would be mined as a correction from a person who never spoke, and rules
/// learned from them would ride in every future prompt: the
/// `"Blocked by a hook:"` mistake in a third costume. `agent::is_harness_voice`
/// is the one place that recognition lives, and it matches on this.
pub const NOTICE_STEM: &str = "Nothing is being learned here:";

/// Which rung of §9.1's ladder the run has reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rung {
    /// Rung 1: change approach.
    Change,
    /// Rung 3: a fresh `Conversation` — the strongest available escape from a
    /// context that has talked itself into a corner.
    Delegate,
    /// Rung 1 again, reached by the shape key: one tool failing the same way
    /// on `STUCK` different targets.
    Guessing,
    /// Rung 1 for a call the machine refused (policy, hook, interlock, an
    /// approver's `Blocked`) on `STUCK` different targets — never a person's
    /// `Deny`, which the loop keeps out of the shape key. Its own words because the
    /// response differs: a refusal is not a puzzle to reword.
    Rewording,
}

impl Rung {
    /// Which notice wins when one turn crosses several: the higher rung, and
    /// within rung 1 the exact repeat, which names the more specific thing.
    fn weight(self) -> u8 {
        match self {
            Rung::Delegate => 3,
            Rung::Change => 2,
            Rung::Rewording | Rung::Guessing => 1,
        }
    }
}

/// One executed call, as this counts it.
#[derive(Debug, Clone, Copy)]
pub struct Outcome<'a> {
    pub tool: &'a str,
    /// [`Boredom::key`]: the target and the result.
    pub key: u64,
    /// Set when the call errored or was refused ([`Boredom::failure`]).
    pub failure: Option<Failure>,
}

/// What an errored call failed *as*, and what it was aimed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Failure {
    /// The tool and the error's [`error_shape`].
    class: u64,
    /// `compact::target_of`, so a reworded argument order is not a new guess.
    target: u64,
    /// The machine refused the call (`ToolCallTrace::blocked`) rather than
    /// the call failing.
    refused: bool,
}

/// An error with what varies between attempts masked out: every span in
/// double quotes or backticks, and every word holding a digit or a path
/// separator. Two jail errors for two different paths, or two policy blocks
/// for two different commands, come out identical; "no such file" and
/// "resolves outside the workspace" do not.
///
/// The whole text rather than its first line, so a failure carrying a long
/// and varying explanation (a compiler's stderr) does not collapse into its
/// wrapper — the cost of matching too little is a missed notice, and of
/// matching too much a false accusation.
pub fn error_shape(error: &str) -> String {
    fn flush(word: &mut String, out: &mut String) {
        if word
            .chars()
            .any(|c| c.is_ascii_digit() || c == '/' || c == '\\')
        {
            out.push('_');
        } else {
            out.push_str(word);
        }
        word.clear();
    }
    let mut out = String::with_capacity(error.len());
    let mut word = String::new();
    let mut chars = error.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' | '`' => {
                flush(&mut word, &mut out);
                out.push(c);
                out.push('_');
                // An unclosed quote masks to the end: fewer matches, never
                // more.
                if chars.by_ref().any(|d| d == c) {
                    out.push(c);
                }
            }
            c if c.is_whitespace() => {
                flush(&mut word, &mut out);
                out.push(' ');
            }
            _ => word.push(c),
        }
    }
    flush(&mut word, &mut out);
    out
}

/// Per shape: the targets this streak has failed on, how many turns brought
/// a new one, the turn it was last seen, the tool, and whether it was a
/// refusal.
#[derive(Debug)]
struct Shape {
    targets: HashSet<u64>,
    turns: u32,
    last: u32,
    tool: String,
    refused: bool,
}

/// What a bored run can actually reach, read off the registry by the loop.
///
/// Named rather than assumed, on `compact`'s rule about not naming `todo`:
/// pointing the model at a tool that is not registered spends a turn on a call
/// that can only fail. A run with nothing here still gets the notice — *stop
/// repeating this* is the part that does the work, and the rest is where to
/// go instead.
#[derive(Debug, Clone, Default)]
pub struct Escapes {
    /// A tool that runs its work in a conversation of its own, found by
    /// [`Tool::runs_a_fresh_conversation`](crate::tool::Tool::runs_a_fresh_conversation).
    pub delegate: Option<String>,
}

#[derive(Debug, Default)]
pub struct Boredom {
    enabled: bool,
    /// Per key: how many turns have produced this outcome, the tool that
    /// produced it, and the turn number of the most recent one — the third
    /// is what lets a gap past `RECENCY_WINDOW` reset the streak instead of
    /// letting it accumulate for the life of the run. The name is kept and
    /// the arguments are not — the notice needs something concrete to point
    /// at, and a rendered argument list can be most of a turn and can hold
    /// the user's data.
    seen: HashMap<u64, (u32, String, u32)>,
    /// The shape key's streaks, by [`Failure::class`].
    shapes: HashMap<u64, Shape>,
    /// Turns observed so far. Monotonic within one `Boredom`, meaningless
    /// outside it — the same shape as `step::next_run`.
    turn: u32,
    notices: u32,
}

impl Boredom {
    pub fn new(enabled: bool) -> Self {
        Boredom {
            enabled,
            ..Boredom::default()
        }
    }

    /// How many times this run has been told it is going nowhere — the
    /// counter that makes the thresholds above falsifiable.
    pub fn notices(&self) -> u32 {
        self.notices
    }

    /// One outcome, as this counts them.
    ///
    /// The **target** rather than the raw arguments, so two different tools
    /// that reach the same file and get the same bytes are one thing learned
    /// twice — which is the signal, not an approximation of it. And the result
    /// as well as the call, on the loop guard's rule: identical arguments with
    /// a changing result is polling, and a poll must never grade as stuck.
    ///
    /// A 64-bit hash rather than the strings: nothing adversarial is being
    /// resisted, a collision needs two different outcomes to repeat three
    /// times each, and keeping the text would make this a second copy of the
    /// transcript — including of the user's data.
    pub fn key(name: &str, input: &serde_json::Value, result: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        crate::compact::target_of(name, input).hash(&mut hasher);
        result.hash(&mut hasher);
        hasher.finish()
    }

    /// An errored call, for the shape key: the tool and [`error_shape`] of
    /// its error as the class, `compact::target_of` as what it was aimed at.
    /// `refused` is the trace's `blocked` — read off the loop's own record,
    /// never off the result's wording. A person's `Deny` is not passed here
    /// at all.
    pub fn failure(name: &str, input: &serde_json::Value, error: &str, refused: bool) -> Failure {
        use std::hash::{Hash, Hasher};
        let mut class = std::collections::hash_map::DefaultHasher::new();
        name.hash(&mut class);
        error_shape(error).hash(&mut class);
        refused.hash(&mut class);
        let mut target = std::collections::hash_map::DefaultHasher::new();
        crate::compact::target_of(name, input).hash(&mut target);
        Failure {
            class: class.finish(),
            target: target.finish(),
            refused,
        }
    }

    /// Record one *turn's* executed calls, and say whether it just crossed a
    /// rung.
    ///
    /// Per turn rather than per call, on the loop guard's reasoning: a model
    /// that emits the same call twice in one parallel batch is being wasteful,
    /// not stuck, and the repetition this watches for is across turns. At most
    /// one notice per turn, because two at once is one thing to say.
    pub fn observe_turn<'a>(
        &mut self,
        turn: impl IntoIterator<Item = Outcome<'a>>,
    ) -> Option<(Rung, String)> {
        if !self.enabled || self.notices >= MAX_NOTICES {
            return None;
        }
        self.turn += 1;
        let now = self.turn;
        // Deduplicated by outcome, in the order the calls were made.
        let mut once = HashSet::new();
        let outcomes: Vec<Outcome<'a>> = turn.into_iter().filter(|o| once.insert(o.key)).collect();
        let mut crossed: Option<(Rung, String)> = None;
        let mut cross = |rung: Rung, tool: &str| {
            if crossed
                .as_ref()
                .is_none_or(|(r, _)| rung.weight() > r.weight())
            {
                crossed = Some((rung, tool.to_string()));
            }
        };
        for &Outcome { tool, key, .. } in &outcomes {
            let entry = self.seen.entry(key).or_insert((0, tool.to_string(), now));
            // A gap past the window is a fresh streak, not a continuation —
            // the same target read again after enough ordinary work in
            // between is not the same finding as three in a row.
            if now.saturating_sub(entry.2) > RECENCY_WINDOW {
                entry.0 = 0;
            }
            entry.0 += 1;
            entry.2 = now;
            // `==`, not `>=`: a rung is crossed once. A run that keeps
            // repeating past the last rung is left to the loop guard and the
            // turn ceiling, which is the honest end of this ladder — rungs 4
            // and 5 belong to mechanisms that already exist.
            match entry.0 {
                STUCK => cross(Rung::Change, &entry.1),
                STILL_STUCK => cross(Rung::Delegate, &entry.1),
                _ => {}
            }
        }
        // The shape key. Per shape, whether this turn brought it back on a
        // target the streak had not seen; a parallel batch of three guesses
        // is one turn of guessing, on the per-turn rule above.
        let mut fresh: Vec<u64> = Vec::new();
        for &Outcome { tool, failure, .. } in &outcomes {
            let Some(f) = failure else { continue };
            let shape = self.shapes.entry(f.class).or_insert_with(|| Shape {
                targets: HashSet::new(),
                turns: 0,
                last: now,
                tool: tool.to_string(),
                refused: f.refused,
            });
            if now.saturating_sub(shape.last) > RECENCY_WINDOW {
                shape.targets.clear();
                shape.turns = 0;
            }
            shape.last = now;
            if shape.targets.insert(f.target) && !fresh.contains(&f.class) {
                fresh.push(f.class);
            }
        }
        for class in fresh {
            let Some(shape) = self.shapes.get_mut(&class) else {
                continue;
            };
            shape.turns += 1;
            // `==` again: crossed once per streak.
            if shape.turns == STUCK {
                let rung = if shape.refused {
                    Rung::Rewording
                } else {
                    Rung::Guessing
                };
                cross(rung, &shape.tool);
            }
        }
        if crossed.is_some() {
            self.notices += 1;
        }
        crossed
    }
}

impl Rung {
    /// What the model is told, folded into the message carrying the tool
    /// results.
    ///
    /// **Wording is load-bearing**, on `EMPTY_TURN_NUDGE`'s evidence: a vague
    /// nudge invites a model to start the task over from the top, which burns
    /// the budget that was already the problem. So each line names the cause,
    /// forbids the repeat rather than the task, and offers concrete
    /// continuations — and never more than the run can actually reach.
    pub fn notice(self, tool: &str, escapes: &Escapes) -> String {
        match self {
            Rung::Change => format!(
                "{NOTICE_STEM} `{tool}` has now returned exactly the same thing \
                 {STUCK} times. Do not start the task over — keep what you have \
                 worked out, and either take a different route to this one piece or \
                 revise the plan if the step itself is the wrong shape."
            ),
            Rung::Delegate => {
                let mut s = format!(
                    "{NOTICE_STEM} `{tool}` has returned the same thing {STILL_STUCK} \
                     times now, and changing the approach inside this conversation has \
                     not moved it."
                );
                match &escapes.delegate {
                    Some(delegate) => s.push_str(&format!(
                        " Hand this piece to `{delegate}`, which starts from a clean \
                         conversation — write the task for someone with no memory of \
                         this one."
                    )),
                    None => s.push_str(
                        " Say what is blocking it and what you would need, rather than \
                         trying it again.",
                    ),
                }
                s
            }
            // No delegate offered for either: a fresh conversation inherits
            // the same jail and the same policy, so handing a refusal to a
            // child is the reword in a third costume.
            Rung::Guessing => format!(
                "{NOTICE_STEM} `{tool}` has now failed the same way on {STUCK} \
                 different arguments. The arguments are changing and the error is \
                 not, so another variant will fail the same way too. Do not start \
                 the task over — read the error for what it asks for instead (a \
                 location it names, an argument it wants, a way to ask for access), \
                 look at what actually exists before naming something again, or say \
                 what is blocking this piece."
            ),
            Rung::Rewording => format!(
                "{NOTICE_STEM} `{tool}` has now been refused for {STUCK} different \
                 calls. A refusal is about what the call does, not how it is \
                 written, so another variant will be refused the same way. Do not \
                 try to route around it — finish what does not need it, and tell \
                 the user what was refused and why."
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(b: &mut Boredom, key: u64) -> Option<(Rung, String)> {
        b.observe_turn([Outcome {
            tool: "build",
            key,
            failure: None,
        }])
    }

    /// One errored call, as the loop hands it over.
    fn failed<'a>(tool: &'a str, path: &str, error: &str, refused: bool) -> Outcome<'a> {
        let input = serde_json::json!({ "path": path });
        Outcome {
            tool,
            key: Boredom::key(tool, &input, error),
            failure: Some(Boredom::failure(tool, &input, error, refused)),
        }
    }

    fn jail(path: &str) -> String {
        format!("tool `fs_list` failed: path {path:?} resolves outside the workspace (/srv/work).")
    }

    #[test]
    fn ordinary_repetition_is_the_model_s_business() {
        let mut b = Boredom::new(true);
        assert!(turn(&mut b, 1).is_none());
        assert!(turn(&mut b, 1).is_none(), "a retry is how work gets done");
    }

    #[test]
    fn a_third_identical_outcome_crosses_the_first_rung_once() {
        let mut b = Boredom::new(true);
        turn(&mut b, 1);
        turn(&mut b, 1);
        assert_eq!(turn(&mut b, 1).unwrap().0, Rung::Change);
        assert!(
            turn(&mut b, 1).is_none(),
            "a rung is crossed once; a notice every turn is the distractor shape"
        );
        assert!(turn(&mut b, 1).is_none());
        // Six.
        assert_eq!(turn(&mut b, 1).unwrap().0, Rung::Delegate);
        assert!(
            turn(&mut b, 1).is_none(),
            "and then the loop guard's problem"
        );
    }

    /// The bug this guards: `seen` used to accumulate for the life of the
    /// run with no recency window, so three occurrences of the same target
    /// *anywhere* — a `shell: git status` at three natural checkpoints an
    /// hour apart — read as three in a row.
    #[test]
    fn a_repeat_far_apart_does_not_accumulate_toward_the_rung() {
        let mut b = Boredom::new(true);
        assert!(turn(&mut b, 1).is_none());
        // More turns of unrelated work than the recency window allows.
        for k in 100..100 + RECENCY_WINDOW + 1 {
            assert!(turn(&mut b, k as u64).is_none());
        }
        // Two more repeats, close together — a streak of two since the gap
        // reset it, not three, so still no rung.
        assert!(
            turn(&mut b, 1).is_none(),
            "the gap past the window should have reset the streak"
        );
        assert!(
            turn(&mut b, 1).is_none(),
            "three occurrences spread across a long run are not three in a row"
        );
    }

    /// The window has to be wide enough to admit ordinary interleaved work —
    /// a gap *inside* it must not reset the streak, or the detector would
    /// never fire on the commonest stuck shape (a failing call retried with
    /// something else attempted in between).
    #[test]
    fn a_gap_inside_the_window_still_counts_toward_the_rung() {
        let mut b = Boredom::new(true);
        assert!(turn(&mut b, 1).is_none());
        // Turns-since-last-occurrence must stay at or under the window,
        // counting the repeat's own turn: `RECENCY_WINDOW - 1` calls of
        // unrelated work leaves exactly `RECENCY_WINDOW` turns of gap.
        for k in 100..100 + RECENCY_WINDOW - 1 {
            assert!(turn(&mut b, k as u64).is_none());
        }
        assert!(turn(&mut b, 1).is_none());
        assert_eq!(
            turn(&mut b, 1).unwrap().0,
            Rung::Change,
            "a gap within the window is still one streak"
        );
    }

    #[test]
    fn a_changing_result_is_polling_and_never_stuck() {
        let mut b = Boredom::new(true);
        for key in 0..10 {
            assert!(turn(&mut b, key).is_none());
        }
    }

    #[test]
    fn the_same_call_twice_in_one_batch_is_waste_and_not_a_loop() {
        let mut b = Boredom::new(true);
        // Three turns' worth of repetition, all inside one turn.
        let call = Outcome {
            tool: "build",
            key: 1,
            failure: None,
        };
        assert!(b.observe_turn([call, call, call]).is_none());
    }

    // --- the shape key ---

    #[test]
    fn a_shape_masks_what_varies_and_keeps_what_does_not() {
        assert_eq!(
            error_shape(&jail("/etc")),
            error_shape(&jail("/home/x/2026"))
        );
        assert_eq!(
            error_shape("No such file or directory (os error 2)"),
            error_shape("No such file or directory (os error 13)"),
        );
        assert_eq!(
            error_shape("Blocked by policy: `shell` call forbidden: `rm -rf a`"),
            error_shape("Blocked by policy: `shell` call forbidden: `rm -rf b/c`"),
        );
        assert_ne!(
            error_shape(&jail("/etc")),
            error_shape("No such file or directory (os error 2)"),
            "two different failures are two different facts"
        );
        // An unclosed quote masks to the end rather than leaking the rest.
        assert_eq!(
            error_shape("bad `x and more"),
            error_shape("bad `y and other")
        );
    }

    /// The corpus's commonest model-side failure: one tool, one refusal,
    /// a new path every turn. The first key saw three new outcomes and said
    /// nothing; this is the test that failed before the shape key existed.
    #[test]
    fn the_same_failure_on_three_new_targets_crosses_rung_one_once() {
        let mut b = Boredom::new(true);
        let mut go = |p: &str| b.observe_turn([failed("fs_list", p, &jail(p), false)]);
        assert!(go("/etc").is_none());
        assert!(go("/usr").is_none(), "two guesses is a retry");
        assert_eq!(go("/var").unwrap(), (Rung::Guessing, "fs_list".to_string()));
        assert!(go("/opt").is_none(), "crossed once per streak");
    }

    #[test]
    fn a_refusal_reworded_is_named_as_a_refusal() {
        let mut b = Boredom::new(true);
        let blocked = "Blocked by policy: `fs_write` modifies state and this run is read-only";
        let mut go = |p: &str| b.observe_turn([failed("fs_write", p, blocked, true)]);
        go("a.md");
        go("b.md");
        assert_eq!(go("c.md").unwrap().0, Rung::Rewording);
    }

    #[test]
    fn different_failures_on_different_targets_are_ordinary_work() {
        let mut b = Boredom::new(true);
        assert!(b
            .observe_turn([failed("fs_read", "a", "No such file or directory", false)])
            .is_none());
        assert!(b
            .observe_turn([failed("fs_read", "b", "permission denied", false)])
            .is_none());
        assert!(b
            .observe_turn([failed("fs_read", "c", "is a directory", false)])
            .is_none());
    }

    /// Three guesses in one parallel batch are one turn of guessing — the
    /// per-turn rule the first key already keeps.
    #[test]
    fn three_guesses_in_one_batch_count_as_one_turn() {
        let mut b = Boredom::new(true);
        let batch = |b: &mut Boredom, ps: [&str; 3]| {
            b.observe_turn(ps.map(|p| failed("fs_list", p, &jail(p), false)))
        };
        assert!(batch(&mut b, ["/a", "/b", "/c"]).is_none());
        assert!(batch(&mut b, ["/d", "/e", "/f"]).is_none());
        assert_eq!(batch(&mut b, ["/g", "/h", "/i"]).unwrap().0, Rung::Guessing);
    }

    /// An unchanged call is the first key's, never this one's — or a call
    /// polled with a moving error would grade as guessing.
    #[test]
    fn the_same_target_again_is_not_a_new_guess() {
        let mut b = Boredom::new(true);
        for n in 0..5 {
            let err = format!("not ready: `{n}` of 9");
            assert!(
                b.observe_turn([failed("status", "job", &err, false)])
                    .is_none(),
                "a poll with a moving error is not guessing"
            );
        }
    }

    #[test]
    fn a_guess_far_apart_starts_a_new_streak() {
        let mut b = Boredom::new(true);
        b.observe_turn([failed("fs_list", "/a", &jail("/a"), false)]);
        b.observe_turn([failed("fs_list", "/b", &jail("/b"), false)]);
        for k in 100..100 + RECENCY_WINDOW as u64 + 1 {
            turn(&mut b, k);
        }
        assert!(b
            .observe_turn([failed("fs_list", "/c", &jail("/c"), false)])
            .is_none());
    }

    #[test]
    fn the_exact_repeat_outranks_the_shape_in_one_turn() {
        let mut b = Boredom::new(true);
        let same = failed("fs_list", "/same", &jail("/same"), false);
        for p in ["/x", "/y"] {
            b.observe_turn([same, failed("fs_list", p, &jail(p), false)]);
        }
        // Third turn: `/same` crosses the first key, and the shape key
        // crosses too (three turns that each brought a new target).
        let crossed = b
            .observe_turn([same, failed("fs_list", "/z", &jail("/z"), false)])
            .unwrap();
        assert_eq!(crossed.0, Rung::Change);
        assert_eq!(b.notices(), 1, "one notice per turn");
    }

    #[test]
    fn the_new_notices_say_what_is_true() {
        let guess = Rung::Guessing.notice("image_view", &Escapes::default());
        assert!(
            guess.starts_with(NOTICE_STEM),
            "recognisable as the harness"
        );
        assert!(guess.contains("`image_view`") && guess.contains("different arguments"));
        assert!(guess.contains("Do not start the task over"));
        let reword = Rung::Rewording.notice(
            "shell",
            &Escapes {
                delegate: Some("researcher".into()),
            },
        );
        assert!(reword.starts_with(NOTICE_STEM));
        assert!(reword.contains("refused") && reword.contains("tell the user"));
        assert!(
            !reword.contains("researcher"),
            "a child inherits the policy: delegating a refusal is rewording it"
        );
    }

    #[test]
    fn a_run_stops_talking_about_itself_eventually() {
        let mut b = Boredom::new(true);
        for key in 0..5 {
            for _ in 0..STUCK {
                turn(&mut b, key);
            }
        }
        assert_eq!(b.notices, MAX_NOTICES);
    }

    #[test]
    fn switched_off_it_says_nothing() {
        let mut b = Boredom::new(false);
        for _ in 0..20 {
            assert!(turn(&mut b, 1).is_none());
        }
    }

    #[test]
    fn a_notice_names_only_what_the_run_can_reach() {
        let bare = Escapes::default();
        let change = Rung::Change.notice("build", &bare);
        assert!(change.contains("`build`") && change.contains("different route"));
        assert!(
            change.starts_with(NOTICE_STEM),
            "every notice is recognisable"
        );
        // Forbids the repeat rather than the task — the nudge that sends a
        // model back to the top burns the budget that was already the problem.
        assert!(change.contains("Do not start the task over"));

        let full = Escapes {
            delegate: Some("researcher".into()),
        };
        let delegate = Rung::Delegate.notice("build", &full);
        assert!(delegate.starts_with(NOTICE_STEM));
        assert!(delegate.contains("`researcher`") && delegate.contains("no memory"));

        // With nothing to delegate to, the fallback says what is true rather
        // than pointing at a tool that is not there.
        let alone = Rung::Delegate.notice("build", &bare);
        assert!(alone.contains("blocking") && !alone.contains("researcher"));
    }
}
