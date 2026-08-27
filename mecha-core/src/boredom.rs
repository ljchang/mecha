//! Nothing is being learned from this approach — `docs/GOAL-SYSTEM-DESIGN.md`
//! §9.1, rungs 1 and 3.
//!
//! **The loop guard is the crudest possible version of this**, and until now
//! the only one: it fires on an identical call with an identical result inside
//! a window after a compaction, and its response is rung 5 — end the run. So a
//! run that is going nowhere had exactly two states, *proceeding* and *dead*.
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

use std::collections::HashMap;

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
const STILL_STUCK: u32 = 6;

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

    /// Record one *turn's* executed calls, and say whether it just crossed a
    /// rung.
    ///
    /// Per turn rather than per call, on the loop guard's reasoning: a model
    /// that emits the same call twice in one parallel batch is being wasteful,
    /// not stuck, and the repetition this watches for is across turns. At most
    /// one notice per turn, because two at once is one thing to say.
    pub fn observe_turn<'a>(
        &mut self,
        turn: impl IntoIterator<Item = (&'a str, u64)>,
    ) -> Option<(Rung, String)> {
        if !self.enabled || self.notices >= MAX_NOTICES {
            return None;
        }
        self.turn += 1;
        let now = self.turn;
        let mut crossed: Option<(Rung, String)> = None;
        let mut this_turn = std::collections::HashSet::new();
        for (name, key) in turn {
            if !this_turn.insert(key) {
                continue;
            }
            let entry = self.seen.entry(key).or_insert((0, name.to_string(), now));
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
            let rung = match entry.0 {
                STUCK => Rung::Change,
                STILL_STUCK => Rung::Delegate,
                _ => continue,
            };
            // The higher rung wins if a turn somehow crosses both.
            if crossed.as_ref().is_none_or(|(r, _)| *r == Rung::Change) {
                crossed = Some((rung, entry.1.clone()));
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(b: &mut Boredom, key: u64) -> Option<(Rung, String)> {
        b.observe_turn([("build", key)])
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
        assert!(b
            .observe_turn([("build", 1), ("build", 1), ("build", 1)])
            .is_none());
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
