//! How big the *next* request will be, from what the last one actually cost.
//!
//! `compact_at` is checked between turns against `prompt_tokens` — the size
//! the provider reported for the previous request. By the time that check
//! runs, the loop has already appended the assistant turn and a batch of tool
//! results that nobody has priced. So the reading the decision is made from
//! describes a message list that is one turn out of date, and the gap is
//! exactly the failure the overflow-recovery arm exists to catch: *"a turn's
//! parallel tool results land all at once, so the size checked between turns
//! can sit well under the limit while the next request is well over."*
//!
//! `docs/GOAL-SYSTEM-DESIGN.md` §4.4 calls that a control problem solved with
//! a constant, and proposes predicting the next size from an observed growth
//! rate. Building it corrected that in one useful way: **there is nothing to
//! extrapolate.** The un-priced tail is sitting in `messages` and can be
//! measured; all that is missing is the conversion to tokens, and the provider
//! re-supplies that every turn by pricing a list whose size we know. So the
//! prediction is arithmetic on two measurements, with no tuned parameter and
//! no model call — which §7.4 requires, since anticipatory appraisal that
//! costs an inference is a tax on every turn.
//!
//! ## The delta form, and why not a ratio
//!
//! A request costs `a + r·bytes`, where `a` is the system prompt and the tool
//! specs — constant within a run and *not* in the message list. Predicting
//! with the cumulative ratio `tokens/bytes` would smear `a` across the bytes
//! and over-predict as the transcript grows. Anchoring instead on the last
//! real measurement and adding only the marginal cost of what changed since
//! removes `a` from the arithmetic entirely, because it is in the anchor.
//!
//! `r` is measured between the last two observations and **floored at the
//! plain-text rate**, never taken lower. A rate that measures low is a rate
//! measured across something that is cheap per byte — an image, which tiles to
//! a fixed token count however many bytes it carries — and carrying that
//! forward would under-predict the next thousand bytes of ordinary prose. The
//! floor makes the error land on the side of predicting *more*, which is the
//! side that compacts early.
//!
//! ## Monotonicity, and the one place it looks violated
//!
//! §7.3: a disposition may only narrow. *"Anxiety may compact early; relief
//! may never compact late."* So [`ContextTracker::over`] is `reported ||
//! predicted` and never `predicted` alone: no state of this type can make
//! compaction fire later than the reactive check alone would.
//!
//! The exception is [`ContextTracker::invalidate`], and it is not one. A
//! reported size is a measurement *of a particular message list*. When
//! eviction or thinning rewrites that list, the number is no longer a reading
//! of anything — the transcript it described does not exist. Continuing to
//! honour it is not caution, it is arithmetic about a deleted object. So a
//! rewrite marks it stale and the prediction becomes the only reading there
//! is, until the provider prices the new list and supplies a real one.
//!
//! The loop already assumed exactly this: after eviction freed something it
//! `continue`s, meaning to *"give it a turn to take effect before paying for a
//! summary."* That has never worked — `prompt_tokens` is assigned in one place,
//! after a response, so the re-entered check saw the same stale value, the
//! three passes returned zero the second time (they are idempotent, with tests
//! saying so), and the summary was paid for anyway one iteration later. The
//! intent needed a reading the reactive check structurally cannot produce
//! without spending a request. This is that reading.

use crate::message::{Block, Message};

/// Bytes per token for ordinary prose, and the floor on the measured rate.
///
/// The same constant `ToolsConfig::resolved_output_budget` converts with, kept
/// at that value on purpose: the two are estimating the same quantity from
/// opposite ends, and a budget that thinks results cost 3 bytes a token beside
/// a predictor that thinks they cost 4 is two answers to one question.
pub const BYTES_PER_TOKEN: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Observation {
    tokens: u64,
    bytes: usize,
}

/// The size series for one run. In memory only; nothing here is stored.
#[derive(Debug, Clone, Default)]
pub struct ContextTracker {
    last: Option<Observation>,
    prev: Option<Observation>,
    /// `last` describes a message list that has since been rewritten.
    stale: bool,
    peak_tokens: u64,
}

impl ContextTracker {
    pub fn new() -> ContextTracker {
        ContextTracker::default()
    }

    /// Record what the provider charged for a list of a known size.
    pub fn observe(&mut self, tokens: u64, bytes: usize) {
        self.prev = self.last;
        self.last = Some(Observation { tokens, bytes });
        self.stale = false;
        self.peak_tokens = self.peak_tokens.max(tokens);
    }

    /// The transcript was rewritten under the last reading, so it is no longer
    /// a reading of it. See the module docs — this is the one thing here that
    /// can move a decision *later*, and it does so by discarding a number
    /// about a message list that no longer exists rather than by overriding a
    /// live one.
    pub fn invalidate(&mut self) {
        self.stale = true;
    }

    /// The last measured prompt size, or `None` when there is not one that
    /// describes the current transcript.
    pub fn reported(&self) -> Option<u64> {
        (!self.stale).then_some(self.last?.tokens)
    }

    /// The largest prompt this run ever actually sent. A measurement
    /// throughout — never a prediction — because it is recorded, and a
    /// recorded estimate is indistinguishable from a recorded fact later.
    pub fn peak_tokens(&self) -> u64 {
        self.peak_tokens
    }

    /// Marginal tokens per byte, floored at the plain-text rate.
    fn tokens_per_byte(&self) -> f64 {
        let floor = 1.0 / BYTES_PER_TOKEN;
        let (Some(last), Some(prev)) = (self.last, self.prev) else {
            return floor;
        };
        let d_bytes = last.bytes as f64 - prev.bytes as f64;
        let d_tokens = last.tokens as f64 - prev.tokens as f64;
        if d_bytes <= 0.0 || d_tokens <= 0.0 {
            return floor;
        }
        (d_tokens / d_bytes).max(floor)
    }

    /// What a request carrying `bytes` of messages would cost.
    ///
    /// `None` before the first response: with no anchor there is no
    /// measurement to extrapolate from, and a guess made entirely of constants
    /// would be a tuned parameter wearing a prediction's clothes.
    pub fn predict(&self, bytes: usize) -> Option<u64> {
        let last = self.last?;
        let delta = (bytes as f64 - last.bytes as f64) * self.tokens_per_byte();
        Some((last.tokens as f64 + delta).max(0.0) as u64)
    }

    /// Is a transcript of this size due a compaction?
    ///
    /// `reported || predicted`, in that order and never the prediction alone.
    /// That spelling is the monotonicity guarantee in one line: whatever this
    /// type believes, it can only ever add a reason to compact.
    pub fn over(&self, limit: u64, bytes: usize) -> bool {
        self.reported().is_some_and(|t| t >= limit)
            || self.predict(bytes).is_some_and(|t| t >= limit)
    }

    /// Share of the window the largest request used, for the record.
    pub fn peak_pressure(&self, window: Option<u64>) -> Option<f32> {
        let window = window.filter(|w| *w > 0)?;
        (self.peak_tokens > 0).then(|| self.peak_tokens as f32 / window as f32)
    }
}

/// Size of a message list, for the purpose of tracking how it *changes*.
///
/// Image payloads are deliberately excluded. Base64 is enormous per token —
/// llama-server tiles an image to a fixed count regardless of its size, and a
/// 5.7 MB screenshot and its 179 KB re-encoding both priced at 294 tokens — so
/// counting those bytes would say a turn grew by megabytes when it grew by a
/// few hundred tokens. The cost is real and it is already in the anchor, which
/// is a measurement of the whole request; what this walk has to track is the
/// part that grows every turn, which is text.
pub fn message_bytes(messages: &[Message]) -> usize {
    messages
        .iter()
        .flat_map(|m| &m.content)
        .map(|b| match b {
            Block::Text { text } => text.len(),
            Block::Thinking { text, signature } => {
                text.len() + signature.as_ref().map_or(0, String::len)
            }
            // `input` is a `Value`; its rendered length is what goes on the
            // wire, and a tool call's arguments can be most of a turn.
            Block::ToolUse { id, name, input } => id.len() + name.len() + input.to_string().len(),
            Block::ToolResult {
                tool_use_id,
                content,
                ..
            } => tool_use_id.len() + content.len(),
            // `data` excluded, `source` counted: it is a file path the
            // model reads, and it is the only part of an image block whose
            // length says anything about how much text is on the wire.
            Block::Image {
                media_type, source, ..
            } => media_type.len() + source.as_ref().map_or(0, String::len),
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Role, Usage};

    fn msg(text: &str) -> Message {
        Message {
            role: Role::User,
            content: vec![Block::text(text)],
        }
    }

    #[test]
    fn with_no_measurement_there_is_no_prediction() {
        let t = ContextTracker::new();
        assert_eq!(t.predict(10_000), None);
        assert!(!t.over(1, 10_000), "and nothing to compact on");
    }

    /// The anchor carries the system prompt and the tool specs, which are not
    /// in the message list. A cumulative ratio would smear them across the
    /// bytes; the delta form must not.
    #[test]
    fn the_prediction_anchors_on_the_last_real_measurement() {
        let mut t = ContextTracker::new();
        // 1,000 bytes of messages priced at 2,000 tokens: 1,700 of that is a
        // system prompt and tool specs the message list does not contain.
        t.observe(2_000, 1_000);
        // 300 more bytes of prose is ~100 more tokens, not another 600 — which
        // is what `tokens/bytes = 2.0` scaled up would have claimed.
        assert_eq!(t.predict(1_300), Some(2_100));
    }

    #[test]
    fn a_measured_rate_above_the_floor_is_used_and_one_below_it_is_not() {
        // Token-dense content: 900 bytes cost 450 tokens, twice the prose rate.
        let mut dense = ContextTracker::new();
        dense.observe(1_000, 1_000);
        dense.observe(1_450, 1_900);
        assert_eq!(dense.predict(2_900), Some(1_950), "0.5 tok/byte carried on");

        // An image landed: bytes leapt, tokens barely moved. Carrying that
        // rate forward would under-predict every later byte of prose, so the
        // floor takes over.
        let mut cheap = ContextTracker::new();
        cheap.observe(1_000, 1_000);
        cheap.observe(1_010, 9_000);
        assert_eq!(cheap.predict(12_000), Some(2_010), "floored at 1/3");
    }

    /// The guarantee §7.3 asks for, stated as a property rather than as a
    /// comment: no state of this type makes compaction fire later than the
    /// reactive check alone.
    #[test]
    fn a_prediction_can_only_ever_add_a_reason_to_compact() {
        for (tokens, bytes, now) in [
            (100u64, 100usize, 100usize),
            (5_000, 10_000, 10_000),
            (5_000, 10_000, 1_000),
            (5_000, 10_000, 90_000),
        ] {
            let mut t = ContextTracker::new();
            t.observe(tokens, bytes);
            for limit in [1u64, 100, 4_999, 5_000, 5_001, 1_000_000] {
                let reactive = tokens >= limit;
                assert!(
                    !reactive || t.over(limit, now),
                    "reactive fired at limit {limit} and the tracker did not"
                );
            }
        }
    }

    /// The deferral the loop has always meant to make and never could.
    #[test]
    fn a_rewrite_retires_the_reading_it_invalidated() {
        let mut t = ContextTracker::new();
        // Priced at the threshold: a summary is due.
        t.observe(21_000, 60_000);
        assert!(t.over(20_000, 60_000));

        // Eviction and thinning cut the transcript in half. The reported size
        // still says 21,000, and it is now a fact about a message list that no
        // longer exists.
        t.invalidate();
        assert_eq!(t.reported(), None, "a rewritten list has no measured size");
        assert!(
            !t.over(20_000, 30_000),
            "the free passes freed enough; the summary is not paid for"
        );
        // But the anchor is not thrown away — it is still the only real
        // measurement, and the next turn is predicted from it.
        assert_eq!(t.predict(30_000), Some(11_000));
        // And a rewrite that freed too little still compacts.
        assert!(t.over(20_000, 58_000));
    }

    #[test]
    fn a_fresh_measurement_ends_the_staleness() {
        let mut t = ContextTracker::new();
        t.observe(21_000, 60_000);
        t.invalidate();
        t.observe(9_000, 30_000);
        assert_eq!(t.reported(), Some(9_000));
    }

    #[test]
    fn the_peak_is_the_largest_request_actually_sent() {
        let mut t = ContextTracker::new();
        t.observe(1_000, 1_000);
        t.observe(9_000, 9_000);
        t.observe(4_000, 4_000);
        assert_eq!(t.peak_tokens(), 9_000, "not the last, and not the current");
        assert_eq!(t.peak_pressure(Some(36_000)), Some(0.25));
        assert_eq!(t.peak_pressure(None), None, "no window, no fraction");
        assert_eq!(
            ContextTracker::new().peak_pressure(Some(100)),
            None,
            "and a run that sent nothing has no pressure, rather than zero"
        );
    }

    #[test]
    fn image_payloads_are_not_counted_as_growth() {
        let text = vec![msg("hello")];
        let with_image = vec![Message {
            role: Role::User,
            content: vec![
                Block::text("hello"),
                Block::Image {
                    media_type: "image/png".into(),
                    data: "A".repeat(200_000),
                    source: None,
                },
            ],
        }];
        assert_eq!(message_bytes(&text), 5);
        assert_eq!(
            message_bytes(&with_image),
            5 + "image/png".len(),
            "the base64 is in the anchor, not in the growth"
        );
    }

    #[test]
    fn every_other_block_kind_counts_toward_the_size() {
        let m = vec![Message {
            role: Role::Assistant,
            content: vec![
                Block::Text { text: "ab".into() },
                Block::Thinking {
                    text: "cde".into(),
                    signature: Some("fg".into()),
                },
                Block::ToolUse {
                    id: "h".into(),
                    name: "ij".into(),
                    input: serde_json::json!({}),
                },
                Block::ToolResult {
                    tool_use_id: "k".into(),
                    content: "lmno".into(),
                    is_error: false,
                },
            ],
        }];
        // 2 + (3+2) + (1+2+2) + (1+4)
        assert_eq!(message_bytes(&m), 17);
    }

    /// Guards the one thing that could silently unhook the whole module: a
    /// `Usage` whose `total_input` stopped counting the cached tiers would
    /// make every observation an underestimate, and nothing here would notice.
    #[test]
    fn the_observed_size_is_the_whole_prompt_including_cache() {
        let u = Usage {
            input_tokens: 8,
            cache_creation_input_tokens: 1_000,
            cache_read_input_tokens: 17_000,
            ..Usage::default()
        };
        assert_eq!(u.total_input(), 18_008);
    }
}
