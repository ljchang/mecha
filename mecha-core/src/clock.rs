//! What time it is, as a capability the harness supplies per request rather
//! than a value it freezes.
//!
//! **A model has no clock, so the only time it knows is the time it was
//! handed.** That made the date a fact somebody had to render, and rendering
//! it once was the bug: `prepare_tools` called `Utc::now()` at process start,
//! `Agent::new` froze the result into its immutable `system` field, and
//! `mecha serve` then held that agent for as long as the daemon ran. On
//! 2026-09-14 a voice call at 09:21 local was told it was Sunday the 13th by a
//! process that had started at 22:37 the night before, queried the calendar
//! for the wrong day, and read yesterday's schedule back as today's — and when
//! the owner corrected it twice, its own reasoning was "the user is insisting
//! today is Monday, September 14th, which contradicts my system prompt. I
//! should trust the system." A session on 2026-09-11 carried a date two days
//! stale the same way. The trigger runner never did, because it builds an
//! agent per run.
//!
//! So the clock is a trait object, like [`Provider`] and [`Tool`] and
//! [`Approver`]: the loop asks for the time where it needs it and cannot hold
//! a stale answer, because it never holds an answer at all. The refresh
//! cadence stops being a thing anybody has to get right.
//!
//! [`Provider`]: crate::provider::Provider
//! [`Tool`]: crate::tool::Tool
//! [`Approver`]: crate::tool::Approver
use chrono::{DateTime, Utc};
use std::sync::Arc;

/// The one question: what is it now?
pub trait Clock: Send + Sync + std::fmt::Debug {
    fn now(&self) -> DateTime<Utc>;
}

/// The machine's clock — what every real run uses.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// One instant, forever.
///
/// An experiment's fixture clock ([`crate::experiment::fixture_now`]): a
/// designed comparison replaces the operator's world, and "what day is it"
/// is part of that world. Also the honest spelling of the old behaviour —
/// a run whose date never moves — which is why the experiment path is
/// unchanged by moving the render per-turn.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock(pub DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

/// A clock a test can move.
///
/// The verification this file exists for is "an agent built on one day and
/// still running on the next states the next day", and there is no way to
/// assert that against a clock you cannot advance — the failure was invisible
/// for at least a week precisely because every test finished inside a second.
#[derive(Debug)]
pub struct TestClock(std::sync::Mutex<DateTime<Utc>>);

impl TestClock {
    pub fn new(at: DateTime<Utc>) -> Self {
        TestClock(std::sync::Mutex::new(at))
    }

    /// Parse-or-panic, for a test that wants a literal.
    pub fn at(rfc3339: &str) -> Self {
        TestClock::new(rfc3339.parse().expect("a valid RFC 3339 instant"))
    }

    pub fn set(&self, at: DateTime<Utc>) {
        *self.0.lock().unwrap() = at;
    }

    pub fn advance(&self, by: chrono::Duration) {
        let mut now = self.0.lock().unwrap();
        *now += by;
    }
}

impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

/// The clock a replay of a recorded run should stand in.
///
/// One definition, because three call sites replay a transcript — `mecha
/// replay`, the validation probe, the harness probe — and a replay that
/// stands in a different day than the run it is reproducing is measuring the
/// difference between two Tuesdays rather than the change under test. The
/// recorded reading is [`crate::session::RunConfig::clock`]; a record written
/// before that field existed has none and replays at today's date, which is
/// the only thing left to do and is said out loud rather than hidden.
pub fn for_replay(recorded: Option<DateTime<Utc>>) -> Arc<dyn Clock> {
    match recorded {
        Some(at) => Arc::new(FixedClock(at)),
        None => Arc::new(SystemClock),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fixed_clock_does_not_move_and_a_test_clock_does() {
        let fixed = FixedClock("2026-09-13T22:37:00Z".parse().unwrap());
        let first = fixed.now();
        assert_eq!(first, fixed.now());

        let test = TestClock::at("2026-09-13T22:37:00Z");
        let before = test.now();
        test.advance(chrono::Duration::hours(12));
        assert_eq!(test.now() - before, chrono::Duration::hours(12));
    }
}
