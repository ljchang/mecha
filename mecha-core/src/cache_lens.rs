//! The cache lens: is the cached prefix actually being reused?
//!
//! Prompt caching is a prefix match, and everything that protects the prefix
//! is an invariant somewhere else: the registry's `BTreeMap` keeps the tool
//! list stable, the system prompt is fixed for a run, and the transcript is
//! append-only between turns. Each was verified by hand exactly once — a
//! two-request round-trip that paid 8 uncached input tokens — and nothing
//! has watched since. Any regression (a tool re-registered mid-run, a
//! nondeterministic schema, a mutation the append-only rule missed) presents
//! as nothing at all: requests succeed, answers arrive, and every turn
//! quietly re-pays for the whole history. The bill is the only symptom.
//!
//! The lens is that watcher: a pure observer fed each request's surface and
//! the usage the provider reported for it. It changes nothing — its verdicts
//! go to tracing, never to the model or the loop — and it names the *reason*
//! when reuse legitimately breaks (surface changed, transcript rewritten by
//! compaction), so the one remaining case, re-payment with nothing changed,
//! is an anomaly worth a warning rather than noise.
//!
//! Two honesty rules keep the warnings believable:
//!
//! - **Never accuse on a provider that reports nothing.** A backend with no
//!   cache (or no cache accounting) reports zero for both cache tiers, which
//!   is indistinguishable from a total miss. Until a nonzero cache figure
//!   has been seen, the verdict is [`Verdict::Unobservable`], not a drop.
//! - **Judge only what was sent.** The verdict compares the request that
//!   actually went out (post any overflow recovery) with the one before it,
//!   both from the caller's hand — the lens holds hashes, never content.

use crate::message::{CompletionRequest, Usage};
use std::hash::{DefaultHasher, Hash, Hasher};

/// Below this many re-paid tokens, a drop is not worth a warning: small
/// prompts and the tokenizer boundary a server keeps at the end of its
/// cached block live here (llama-server returned 2,720 of a 2,724-token
/// prefix on a measured round-trip), and a warning that fires on them
/// teaches the reader to ignore it.
const DROP_FLOOR_TOKENS: u64 = 1_024;

/// A drop is only called when the re-paid portion exceeds this fraction of
/// the previous request's whole prompt — reuse degrading, not the few
/// tokens a server trims off the end of the block it hands back.
const DROP_FRACTION: f64 = 0.25;

/// What one request's cache behaviour looked like, and why.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// The first observed request: nothing to compare against.
    Baseline,
    /// Same surface, appended-only transcript, and the numbers look like
    /// reuse: the cached prefix is doing its job.
    Stable { uncached: u64, read: u64 },
    /// The tool list or system prompt changed since the last request. Reuse
    /// breaking here is expected; the name says which knob moved.
    SurfaceChanged { system: bool, tools: bool },
    /// The messages are no longer an extension of what was last sent —
    /// compaction, eviction or thinning rewrote history, and the moving half
    /// of the cache is legitimately gone.
    TranscriptRewritten,
    /// No nonzero cache figure has ever been reported, so reuse cannot be
    /// judged — a local server without cache accounting, or caching off.
    Unobservable,
    /// Nothing changed, the transcript only grew, and the previous prompt
    /// still did not come back from the cache: the invariant this lens
    /// exists to watch has failed somewhere. `repaid` is the part of the
    /// previous prompt that had to be paid for again — never this turn's
    /// new content, however large that is.
    Drop { repaid: u64, prev_total: u64 },
}

struct Prev {
    system: u64,
    tools: u64,
    /// One hash per message *as sent*, so the append-only check is a prefix
    /// comparison rather than a diff.
    messages: Vec<u64>,
    total_input: u64,
}

#[derive(Default)]
pub struct CacheLens {
    prev: Option<Prev>,
    /// A nonzero cache tier has been reported at least once, so zeros from
    /// here on are evidence rather than silence.
    reporting_seen: bool,
}

impl CacheLens {
    pub fn new() -> Self {
        Self::default()
    }

    /// Observe one completed request and the usage the provider reported for
    /// it. Pure bookkeeping: the caller decides what, if anything, to do
    /// with the verdict.
    pub fn observe(&mut self, request: &CompletionRequest, usage: &Usage) -> Verdict {
        let current = Prev {
            system: hash_of(&request.system),
            tools: hash_of(&request.tools),
            messages: request.messages.iter().map(hash_of).collect(),
            total_input: usage.total_input(),
        };
        let cached_reported =
            usage.cache_read_input_tokens > 0 || usage.cache_creation_input_tokens > 0;

        let verdict = match &self.prev {
            None => Verdict::Baseline,
            Some(prev) => {
                let system = prev.system != current.system;
                let tools = prev.tools != current.tools;
                if system || tools {
                    Verdict::SurfaceChanged { system, tools }
                } else if !is_prefix(&prev.messages, &current.messages) {
                    Verdict::TranscriptRewritten
                } else if !self.reporting_seen && !cached_reported {
                    Verdict::Unobservable
                } else {
                    // The question is whether the *previous* prompt came
                    // back, so the only figure that answers it is what was
                    // read. `input_tokens` cannot: it is everything not
                    // read, which on this workload is overwhelmingly the
                    // turn's new content — one mail thread or search result
                    // dwarfs the prompt it was appended to, and scoring that
                    // as re-payment made the lens shout loudest exactly when
                    // tool results were biggest. It also scored the real
                    // failure — a small prompt re-paid in full because
                    // something destabilised the prefix — as stable.
                    let repaid = prev
                        .total_input
                        .saturating_sub(usage.cache_read_input_tokens);
                    let repaid_share = repaid as f64 / prev.total_input.max(1) as f64;
                    if repaid > DROP_FLOOR_TOKENS && repaid_share > DROP_FRACTION {
                        Verdict::Drop {
                            repaid,
                            prev_total: prev.total_input,
                        }
                    } else {
                        Verdict::Stable {
                            uncached: usage.input_tokens,
                            read: usage.cache_read_input_tokens,
                        }
                    }
                }
            }
        };

        self.reporting_seen |= cached_reported;
        self.prev = Some(current);
        verdict
    }
}

/// Hash anything serializable. `DefaultHasher` is unstable across processes,
/// which is fine: a lens lives inside one run and its hashes never leave it.
fn hash_of<T: serde::Serialize>(value: &T) -> u64 {
    let mut h = DefaultHasher::new();
    serde_json::to_string(value)
        .unwrap_or_default()
        .hash(&mut h);
    h.finish()
}

fn is_prefix(prev: &[u64], current: &[u64]) -> bool {
    current.len() >= prev.len() && current[..prev.len()] == *prev
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;

    fn request(messages: Vec<Message>) -> CompletionRequest {
        CompletionRequest {
            model: "m".into(),
            system: Some("system".into()),
            messages,
            tools: vec![],
            max_tokens: 512,
            effort: None,
            thinking: false,
            cache_prompt: true,
        }
    }

    fn usage(uncached: u64, creation: u64, read: u64) -> Usage {
        Usage {
            input_tokens: uncached,
            output_tokens: 10,
            cache_creation_input_tokens: creation,
            cache_read_input_tokens: read,
        }
    }

    fn convo(n: usize) -> Vec<Message> {
        (0..n).map(|i| Message::user(format!("turn {i}"))).collect()
    }

    #[test]
    fn an_appended_turn_with_reuse_is_stable() {
        let mut lens = CacheLens::new();
        assert_eq!(
            lens.observe(&request(convo(1)), &usage(8, 18_000, 0)),
            Verdict::Baseline
        );
        assert_eq!(
            lens.observe(&request(convo(2)), &usage(40, 200, 18_000)),
            Verdict::Stable {
                uncached: 40,
                read: 18_000
            }
        );
    }

    #[test]
    fn a_changed_tool_surface_is_an_expected_break_not_a_drop() {
        let mut lens = CacheLens::new();
        lens.observe(&request(convo(1)), &usage(8, 18_000, 0));
        let mut second = request(convo(2));
        second.tools = vec![crate::message::ToolSpec {
            name: "new_tool".into(),
            description: "appeared mid-run".into(),
            input_schema: serde_json::json!({}),
        }];
        assert_eq!(
            lens.observe(&second, &usage(18_000, 500, 0)),
            Verdict::SurfaceChanged {
                system: false,
                tools: true
            }
        );
    }

    #[test]
    fn a_rewritten_transcript_is_an_expected_break_not_a_drop() {
        let mut lens = CacheLens::new();
        lens.observe(&request(convo(3)), &usage(8, 18_000, 0));
        // A compaction: the head is replaced by a summary, shorter than what
        // was sent before.
        let compacted = vec![
            Message::user("[summary of turns 0-1]"),
            Message::user("turn 2"),
        ];
        assert_eq!(
            lens.observe(&request(compacted), &usage(9_000, 400, 0)),
            Verdict::TranscriptRewritten
        );
    }

    /// The verdict the lens exists for: same surface, appended-only, and the
    /// previous prompt did not come back from the cache.
    #[test]
    fn an_unexplained_repayment_is_a_drop() {
        let mut lens = CacheLens::new();
        lens.observe(&request(convo(1)), &usage(8, 18_000, 0));
        assert_eq!(
            lens.observe(&request(convo(2)), &usage(17_500, 600, 0)),
            Verdict::Drop {
                repaid: 18_008,
                prev_total: 18_008
            }
        );
    }

    /// The regression: a turn whose tool result dwarfs the prompt it was
    /// appended to, with the prefix reused in full. These are the figures a
    /// live llama-server returned — 2,720 of a 2,724-token prefix read back,
    /// 6,319 tokens of genuinely new content — and judging the new content as
    /// re-payment called it a drop at a share of 2.32. Every warning a real
    /// session produced was this shape, which is how a detector stops being
    /// read.
    #[test]
    fn a_large_appended_tool_result_is_not_a_repayment() {
        let mut lens = CacheLens::new();
        lens.observe(&request(convo(1)), &usage(2_724, 0, 0));
        assert_eq!(
            lens.observe(&request(convo(2)), &usage(6_319, 0, 2_720)),
            Verdict::Stable {
                uncached: 6_319,
                read: 2_720
            }
        );
    }

    /// And the inverse, which is the case the lens exists for and the one it
    /// could not see. On a two-tier provider a lost prefix is not re-paid as
    /// *uncached* input — it is re-**written**, so the whole history lands in
    /// `cache_creation` at a write premium while `input_tokens` stays at the
    /// same handful of tokens an ordinary turn pays. Judging on
    /// `input_tokens` therefore scored a total cache rebuild, every turn, as
    /// stable: the most expensive failure available, reported as health.
    #[test]
    fn a_silently_rewritten_cache_block_is_a_drop() {
        let mut lens = CacheLens::new();
        lens.observe(&request(convo(1)), &usage(8, 18_000, 0));
        assert_eq!(
            lens.observe(&request(convo(2)), &usage(8, 18_400, 0)),
            Verdict::Drop {
                repaid: 18_008,
                prev_total: 18_008
            }
        );
    }

    /// A provider that never reports cache figures is never accused: zeros
    /// are silence, not a miss, until a nonzero figure proves reporting.
    #[test]
    fn no_reporting_means_unobservable_never_a_drop() {
        let mut lens = CacheLens::new();
        lens.observe(&request(convo(1)), &usage(18_000, 0, 0));
        assert_eq!(
            lens.observe(&request(convo(2)), &usage(18_100, 0, 0)),
            Verdict::Unobservable
        );
        // And once reporting appears, judgment resumes.
        lens.observe(&request(convo(3)), &usage(50, 0, 18_100));
        assert!(matches!(
            lens.observe(&request(convo(4)), &usage(18_200, 0, 0)),
            Verdict::Drop { .. }
        ));
    }

    /// Small re-payments stay below the alarm: the uncached tail of an
    /// ordinary turn must not read as degradation.
    #[test]
    fn the_ordinary_uncached_tail_stays_stable() {
        let mut lens = CacheLens::new();
        lens.observe(&request(convo(1)), &usage(8, 18_000, 0));
        assert!(matches!(
            lens.observe(&request(convo(2)), &usage(900, 100, 17_000)),
            Verdict::Stable { .. }
        ));
    }
}
