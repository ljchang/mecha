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
//! `r` is measured between the last two observations and **clamped into the
//! band a real tokenizer can occupy** — never below the plain-text rate, never
//! above one token per byte, and not measured at all from a delta too small to
//! be a sample. The floor covers content that is genuinely cheap per byte
//! (repeated characters, whitespace-heavy tool output); the ceiling covers
//! everything that puts tokens on the numerator with no bytes on the
//! denominator, which is the larger hazard and the one that bites in both
//! directions. See `MAX_TOKENS_PER_BYTE`.
//!
//! Note what the ceiling is *not* for: an arriving image. `message_bytes`
//! excludes image payloads, so an image does not produce the cheap-per-byte
//! shape at all — it produces the opposite one, a large token delta over
//! almost no bytes, which is the ceiling's business rather than the floor's.
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
//! ## Known: the series does not cross a run boundary
//!
//! A tracker is created per `run_in`, so in `mecha chat` and the TUI — where
//! one submission is one run — it starts empty on every user turn. On the
//! first iteration of a run there is no anchor, so `over` is false whatever
//! the transcript weighs, and a conversation that grew through user turns, or
//! a resumed session already near the window, still discovers the overflow by
//! being refused.
//!
//! Left as it is deliberately, and it is not a regression: `prompt_tokens`
//! reset at exactly the same boundary before this existed. Closing it means
//! bundling the series with `Conversation`, the way taint is bundled — keep
//! the history and you keep what was learned about it — and that needs an
//! answer for the `/model` switch first, because an anchor is a measurement
//! under one tokenizer and one tool surface and means nothing under another.
//! That is a design decision about where the state lives, not a fix.
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

/// Hard ceiling on the measured rate: **no tokenizer emits more than one token
/// per byte**, because a token is at least one byte.
///
/// So an apparent rate above this is not a property of the text — it is the
/// delta measuring something that is not in the message list at all. That
/// happens: `a` is only *approximately* constant within a run. The tool specs
/// move when a skill narrows the surface or the phase changes, cache
/// accounting shifts between turns, and a failover answers with a different
/// tokenizer. Any of those puts tokens on the numerator with no bytes on the
/// denominator.
///
/// Without the ceiling that is unbounded, and it breaks in both directions.
/// Measured on a probe: `observe(49_000, 149_960)` then `observe(50_000,
/// 150_000)` is a 40-byte delta against 1,000 tokens — `r` of 25 — after which
/// an ordinary 12 KB tool result predicts 350,000 tokens on a transcript
/// really near 54k, buying a summary request and a lossy rewrite for nothing.
/// The same inflated rate then *under*-predicts once the free passes shave 2 KB
/// off: the predicted saving is 50,000 tokens, the prediction lands at zero,
/// and a transcript the provider had just priced at 50,000 skips its summary
/// and goes out oversized. The second direction is the dangerous one, and it
/// is why this is a clamp rather than a warning.
const MAX_TOKENS_PER_BYTE: f64 = 1.0;

/// Below this, an inter-turn delta is noise rather than a sample.
///
/// A turn can move very few message bytes — a `todo` call and a one-line
/// result — while the priced total moves for reasons above. Dividing by a tiny
/// denominator turns that into an arbitrarily large rate, so a short delta
/// does not get a vote and the floor stands in until a real one arrives.
const MIN_SAMPLE_BYTES: f64 = 512.0;

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

    /// Marginal tokens per byte, from the last inter-turn delta, clamped into
    /// the band any real tokenizer can occupy.
    ///
    /// Both bounds fail toward predicting *more*, which is the side that
    /// compacts early — except the ceiling, which also bounds how large a
    /// saving a rewrite may be credited with, and that is the direction a
    /// missing bound skips a summary that was needed.
    fn tokens_per_byte(&self) -> f64 {
        let floor = 1.0 / BYTES_PER_TOKEN;
        let (Some(last), Some(prev)) = (self.last, self.prev) else {
            return floor;
        };
        let d_bytes = last.bytes as f64 - prev.bytes as f64;
        let d_tokens = last.tokens as f64 - prev.tokens as f64;
        if d_bytes < MIN_SAMPLE_BYTES || d_tokens <= 0.0 {
            return floor;
        }
        (d_tokens / d_bytes).clamp(floor, MAX_TOKENS_PER_BYTE)
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

    /// How many bytes of tool output the next turn can take before the
    /// transcript crosses `limit`.
    ///
    /// The other half of §4.4's cliff-to-gradient: the compaction threshold
    /// decides *when* to summarise, and this decides how much a single turn is
    /// allowed to add in the first place. They serve one constraint —
    /// `resolved_output_budget`'s docstring already states it — that "one
    /// turn's results must not leap the gap between the threshold and the
    /// window itself". That budget sizes the gap from the *window*, once, at
    /// startup. This sizes it from where the transcript actually is.
    ///
    /// Converted at the measured rate rather than the floor, which is the
    /// conservative direction: a higher rate buys fewer bytes.
    ///
    /// `None` before the first response, where there is no anchor and so no
    /// claim worth making.
    pub fn affordable_output_bytes(&self, limit: u64, current_bytes: usize) -> Option<usize> {
        let predicted = self.predict(current_bytes)?;
        let room = limit.saturating_sub(predicted) as f64;
        Some((room / self.tokens_per_byte()) as usize)
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
    fn a_measured_rate_inside_the_band_is_used_and_one_outside_it_is_not() {
        // Token-dense content: 900 bytes cost 450 tokens, twice the prose rate.
        let mut dense = ContextTracker::new();
        dense.observe(1_000, 1_000);
        dense.observe(1_450, 1_900);
        assert_eq!(dense.predict(2_900), Some(1_950), "0.5 tok/byte carried on");

        // Genuinely cheap per byte — a result that is mostly repeated
        // characters. Carrying that forward would under-predict the next
        // thousand bytes of prose, so the floor takes over. Deliberately not
        // an image: `message_bytes` excludes image payloads, so an image
        // cannot produce this shape.
        let mut cheap = ContextTracker::new();
        cheap.observe(1_000, 1_000);
        cheap.observe(1_010, 9_000);
        assert_eq!(cheap.predict(12_000), Some(2_010), "floored at 1/3");
    }

    /// The rate is a ratio, and a ratio with a tiny denominator is not a
    /// measurement. A turn can move almost no message bytes — a `todo` call
    /// and a one-line result — while the priced total moves for reasons that
    /// are not in the message list at all.
    #[test]
    fn a_delta_too_small_to_be_a_sample_does_not_set_the_rate() {
        let mut t = ContextTracker::new();
        t.observe(49_000, 149_960);
        t.observe(50_000, 150_000); // 40 bytes, 1,000 tokens
                                    // At the unguarded rate of 25 tok/byte this predicted 350,000.
        assert_eq!(t.predict(162_000), Some(54_000), "the floor, not 25x");
    }

    /// The hole the ceiling closes, and it is the dangerous direction: an
    /// inflated rate makes a small rewrite look like an enormous saving, and
    /// the summary that was due is skipped.
    #[test]
    fn an_impossible_rate_cannot_credit_a_rewrite_with_a_saving_it_did_not_make() {
        let mut t = ContextTracker::new();
        // A sample large enough to be believed, but priced at a rate no
        // tokenizer can produce — the shape a narrowed tool surface or a
        // failover to a different tokenizer leaves behind.
        t.observe(20_000, 100_000);
        t.observe(50_000, 101_000); // 1,000 bytes, 30,000 tokens → r = 30
        assert!(t.over(40_000, 101_000), "50,000 is over the limit");

        // The free passes shave 2 KB. At r = 30 that is a 60,000-token saving
        // and the prediction floors at zero, so the summary is skipped on a
        // transcript the provider had just priced at 50,000.
        t.invalidate();
        assert_eq!(
            t.predict(99_000),
            Some(48_000),
            "a 2 KB cut may be credited with at most 2,000 tokens"
        );
        assert!(
            t.over(40_000, 99_000),
            "so the summary is still taken, which is the point"
        );
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

    /// What bounds the *other* side, where the property above does not reach.
    ///
    /// Once `invalidate` retires the reported size the prediction is the only
    /// reading, so nothing else stops it claiming a saving the rewrite did not
    /// make. The bound is the ceiling: a prediction may differ from its anchor
    /// by at most the byte change times one token per byte, in either
    /// direction. Written as a sweep including hostile observation pairs,
    /// because the version of this that only tested growth is the version that
    /// shipped the hole.
    #[test]
    fn the_predicted_change_is_bounded_by_the_byte_change() {
        let pairs = [
            (1_000u64, 1_000usize, 2_000u64, 2_000usize),
            (20_000, 100_000, 50_000, 101_000), // an impossible rate
            (49_000, 149_960, 50_000, 150_000), // a delta too small to sample
            (5_000, 50_000, 5_010, 90_000),     // very cheap per byte
            (5_000, 50_000, 4_000, 40_000),     // the transcript shrank
        ];
        for (t0, b0, t1, b1) in pairs {
            for now in [0usize, 1, 500, b1 / 2, b1, b1 + 10_000, 500_000] {
                let mut t = ContextTracker::new();
                t.observe(t0, b0);
                t.observe(t1, b1);
                for tracker in [&t, &{
                    let mut c = t.clone();
                    c.invalidate();
                    c
                }] {
                    let predicted = tracker.predict(now).unwrap() as f64;
                    let moved = (now as f64 - b1 as f64).abs() * MAX_TOKENS_PER_BYTE;
                    let anchor = t1 as f64;
                    assert!(
                        predicted <= anchor + moved + 1.0,
                        "{predicted} overshot {anchor} by more than {moved} bytes allow"
                    );
                    assert!(
                        predicted + 1.0 >= (anchor - moved).max(0.0),
                        "{predicted} undershot {anchor} by more than {moved} bytes allow"
                    );
                }
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
    fn what_a_turn_can_afford_shrinks_as_the_transcript_grows() {
        let mut t = ContextTracker::new();
        t.observe(10_000, 30_000); // 1/3 tok per byte

        // 20,000 tokens of room, at 3 bytes a token, is 60,000 bytes.
        assert_eq!(t.affordable_output_bytes(30_000, 30_000), Some(60_000));
        // Closer to the threshold, less is affordable — the gradient the flat
        // budget cannot express.
        assert_eq!(t.affordable_output_bytes(12_000, 30_000), Some(6_000));
        // Past it, nothing is: the compaction check has already fired.
        assert_eq!(t.affordable_output_bytes(9_000, 30_000), Some(0));
        // And with no anchor there is no claim.
        assert_eq!(
            ContextTracker::new().affordable_output_bytes(30_000, 30_000),
            None
        );
    }

    /// A denser measured rate buys *fewer* bytes, which is the direction that
    /// keeps the turn inside the gap rather than the one that flatters it.
    #[test]
    fn a_denser_rate_affords_less() {
        let mut dense = ContextTracker::new();
        dense.observe(10_000, 30_000);
        dense.observe(20_000, 40_000); // 10k tokens over 10k bytes → r = 1.0
        let dense_room = dense.affordable_output_bytes(30_000, 40_000).unwrap();

        let mut prose = ContextTracker::new();
        prose.observe(10_000, 30_000);
        prose.observe(20_000, 60_000); // r floors at 1/3
        let prose_room = prose.affordable_output_bytes(30_000, 60_000).unwrap();

        assert!(
            dense_room < prose_room,
            "dense {dense_room} should afford less than prose {prose_room}"
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
